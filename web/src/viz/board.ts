// Pan and zoom for any picture, SVG or canvas: the wheel zooms about the
// pointer, a drag moves the picture, and a click is still a click.
//
// Every piece above word size answers the wheel (rule 1 of
// docs/DESIGN-diagrams.md), and before this each one that did wrote its own:
// the Diagram view's stage, the zoom map's wheel handler, the treemap's full
// window. This is d3-zoom with the choices made once:
// - The wheel zooms about the pointer, and only when the page is still
//   (`report/wheel.ts`): a reader scrolling past a figure is not stopped by
//   it, and a figure zoomed all the way out lets a zoom-out through to the
//   page.
// - A press that moves less than four pixels before it lifts is a click, so a
//   piece's bars and cells can be clicked without the board eating the click.
//   A longer drag pans, and d3-zoom swallows the click that ends it.
// - Double-click does nothing here. A piece's click goes to the bytes, and a
//   double-click is two of those; zooming in on it as well would move the
//   picture out from under the field just picked.
// - `xOnly` zooms and pans across only. Anything drawn over an address axis
//   keeps file order running left to right at one height, so only x zooms.
//
// The board never measures its host. The caller says how big the view is and
// says again with `resize` when that changes, which is what lets the same
// piece be drawn in a card, in the rail and over the whole window.
//
// The board does not draw. It tells the piece the transform on every change
// (`onZoom`), and the piece either applies it to a group (`<g transform>`) or
// redraws at the new scale, which is how a piece changes what it shows as it
// zooms (`lod.ts`). The arithmetic for that is `boardmath.ts`.
//
// d3-zoom and the d3 modules under it are loaded the first time a board is
// made, the way `graphview.ts` loads cytoscape, so a file that never shows a
// zoomable piece never fetches them. The board is usable at once: it starts
// at no zoom, and a Fit or Home asked for before the module arrives is done
// when it does.

import type { ZoomBehavior, ZoomTransform } from "d3-zoom";
import type { Selection } from "d3-selection";
import { figureTakesWheel } from "../report/wheel.ts";
import { fitTransform, IDENTITY, type Box, type View } from "./boardmath.ts";

export type BoardOptions = {
  readonly width: number;
  readonly height: number;
  /** Zoom and pan across only. */
  readonly xOnly?: boolean;
  /** The smallest and largest zoom. The smallest defaults to 1, the picture
   *  as drawn; the largest to 64. A piece over bytes sets the largest from
   *  how many bytes it holds, so the deepest zoom is a few bytes wide. */
  readonly minScale?: number;
  readonly maxScale?: number;
  /** The drawn picture's box in layout pixels: what Fit fits, and how far a
   *  drag may take it. Defaults to the view. */
  readonly content?: Box;
  /** Called with the transform after every change, and once with the
   *  starting one when the board is made. */
  readonly onZoom: (view: View) => void;
};

export type Board = {
  /** The transform now. */
  view(): View;
  /** Zoom out to show the whole picture, as large as it fits. */
  fit(animate?: boolean): void;
  /** Back to the picture as drawn, with no zoom. */
  home(animate?: boolean): void;
  /** Show this box of the layout, as large as it fits. */
  zoomTo(box: Box, animate?: boolean): void;
  /** Zoom by a factor about the middle of the view, for buttons and keys. */
  zoomBy(factor: number): void;
  /** The view is a new size. `content` changes with it when given. */
  resize(width: number, height: number, content?: Box): void;
  /** Whether the picture is zoomed all the way out. */
  fullyOut(): boolean;
  /** Take the listeners off the host. */
  destroy(): void;
  /** Settles when d3-zoom has loaded and the board answers the pointer. */
  readonly ready: Promise<void>;
};

type D3 = {
  readonly zoom: typeof import("d3-zoom");
  readonly selection: typeof import("d3-selection");
};

let loading: Promise<D3> | null = null;

/** d3-zoom and d3-selection, once. */
function loadD3(): Promise<D3> {
  loading ??= Promise.all([import("d3-zoom"), import("d3-selection")]).then(([zoom, selection]) => ({ zoom, selection }));
  return loading;
}

/** How far a press may move before it is a drag and not a click. */
const CLICK_SLOP_PX = 4;
/** How long an animated Fit or Home takes. Short: it says where the picture
 *  went, and nobody wants to wait for it. */
const ANIMATE_MS = 200;

/** Give `host` pan and zoom. `host` is the element the pointer is over: the
 *  SVG, the canvas, or a div around either. */
export function board(host: Element, opts: BoardOptions): Board {
  let width = opts.width;
  let height = opts.height;
  let content: Box = opts.content ?? { x0: 0, y0: 0, x1: width, y1: height };
  const xOnly = opts.xOnly === true;
  const minScale = opts.minScale ?? 1;
  const maxScale = opts.maxScale ?? 64;
  let current: View = IDENTITY;
  let destroyed = false;
  /** What to do once d3 has arrived, for a call made before it did. */
  let pending: View | null = null;
  let behaviour: ZoomBehavior<Element, unknown> | null = null;
  let sel: Selection<Element, unknown, null, undefined> | null = null;
  let d3: D3 | null = null;
  /** The animation frame an animated move is waiting on. */
  let frame = 0;

  const toD3 = (v: View): ZoomTransform => {
    const z = (d3 as D3).zoom;
    return new z.ZoomTransform(v.k, v.x, v.y);
  };

  const apply = (v: View, animate: boolean): void => {
    if (behaviour === null || sel === null || d3 === null) {
      pending = v;
      current = v;
      opts.onZoom(current);
      return;
    }
    const b = behaviour;
    const s = sel;
    cancelAnimationFrame(frame);
    if (!animate || typeof requestAnimationFrame !== "function") {
      s.call(b.transform, toD3(v));
      return;
    }
    // Eased by hand rather than through d3-transition, which has no types in
    // the repo and would be a second animation system for one movement. The
    // scale is eased in log space, so a zoom from 1 to 100 does not spend
    // its whole first half above 50.
    const from = current;
    const t0 = performance.now();
    const step = (now: number): void => {
      const u = Math.min(1, (now - t0) / ANIMATE_MS);
      const e = u * (2 - u);
      const k = Math.exp(Math.log(from.k) + (Math.log(v.k) - Math.log(from.k)) * e);
      s.call(b.transform, toD3({ k, x: from.x + (v.x - from.x) * e, y: from.y + (v.y - from.y) * e }));
      if (u < 1) frame = requestAnimationFrame(step);
    };
    frame = requestAnimationFrame(step);
  };

  const configure = (): void => {
    if (behaviour === null) return;
    behaviour.extent([
      [0, 0],
      [width, height],
    ]);
    // A little room past the edges, so the first and last bar can be pulled
    // into the middle of the view to be looked at, and no further.
    const slackX = width * 0.25;
    const slackY = xOnly ? 0 : height * 0.25;
    behaviour.translateExtent([
      [content.x0 - slackX, content.y0 - slackY],
      [content.x1 + slackX, content.y1 + slackY],
    ]);
  };

  const ready = loadD3().then((loaded) => {
    if (destroyed) return;
    d3 = loaded;
    const z = loaded.zoom.zoom<Element, unknown>();
    behaviour = z;
    z.scaleExtent([minScale, maxScale]);
    z.clickDistance(CLICK_SLOP_PX);
    // d3's own rule (no ctrl-drag, no right button) for everything but the
    // wheel, which goes by the page's rule instead.
    z.filter((event: Event) => {
      if (event.type === "wheel") return figureTakesWheel(event as WheelEvent, current.k <= minScale + 1e-9);
      const e = event as MouseEvent;
      return !e.ctrlKey && !e.button;
    });
    if (xOnly) {
      const base = z.constrain();
      z.constrain((t, extent, translateExtent) => {
        const c = base(t, extent, translateExtent);
        return new loaded.zoom.ZoomTransform(c.k, c.x, 0);
      });
    }
    z.on("zoom", (e: { transform: ZoomTransform }) => {
      current = { k: e.transform.k, x: e.transform.x, y: e.transform.y };
      opts.onZoom(current);
    });
    configure();
    sel = loaded.selection.select<Element, unknown>(host);
    sel.call(z);
    sel.on("dblclick.zoom", null);
    const start = pending ?? current;
    pending = null;
    sel.call(z.transform, toD3(start));
  });

  opts.onZoom(current);

  const b: Board = {
    view: () => current,
    fit: (animate = false) => apply(fitTransform(content, width, height, xOnly, 0), animate),
    home: (animate = false) => apply(IDENTITY, animate),
    zoomTo: (box, animate = false) => apply(fitTransform(box, width, height, xOnly, 0), animate),
    zoomBy: (factor) => {
      if (behaviour === null || sel === null) return;
      behaviour.scaleBy(sel, factor, [width / 2, height / 2]);
    },
    resize: (w, h, next) => {
      width = w;
      height = h;
      content = next ?? content;
      configure();
      // The same transform under the new extent, put through the constraint
      // again so a picture made narrower is not left panned off its edge.
      if (behaviour !== null && sel !== null) sel.call(behaviour.transform, toD3(current));
    },
    fullyOut: () => current.k <= minScale + 1e-9,
    destroy: () => {
      destroyed = true;
      if (sel !== null) sel.on(".zoom", null);
    },
    ready: ready.then(() => undefined),
  };
  return b;
}
