// The value plot in the list pane: a heading row saying what is drawn, with
// its choices, and the line or the grid under it.
//
// Part of candidate `value-plot` (`valueplot.ts` has the whole piece's
// story). The list pane (`listpane.ts`) asks for this through
// `setPanePlotter` and owns its size. The figure reads the run itself and
// says it is reading until the bytes have all arrived, then draws the line
// (`valueline.ts`) or, for a run of two dimensions, the grid
// (`valuegrid.ts`), and draws again when the reader picks the other one or
// the other colour range.

import type { Doc } from "../doc.ts";
import type { PaneFigure } from "../listpane.ts";
import { childWord, VIZ } from "../strings.ts";
import type { Board } from "./board.ts";
import { finiteRange, valueText } from "./decimate.ts";
import { drawGridPlot } from "./valuegrid.ts";
import { drawLinePlot, lineFromRun } from "./valueline.ts";
import { readRun, runSource, type RunValues } from "./valuerun.ts";

/**
 * The plot of the run at `path`, for the list pane: a heading row with what
 * is drawn and its controls, and the line or the grid under it. Reads the run
 * itself, and says it is reading until the bytes have all arrived.
 */
export function paneFigure(doc: Doc, path: readonly number[], width: number, height: number): PaneFigure {
  const root = document.createElement("figure");
  root.className = "vp";
  const head = document.createElement("figcaption");
  head.className = "vp-head";
  const note = document.createElement("span");
  note.className = "vp-note vp-title";
  // The choices and Reset zoom stay together on one line, which goes under
  // the note when the pane is too narrow for both.
  const tools = document.createElement("span");
  tools.className = "vp-tools";
  const controls = document.createElement("span");
  controls.className = "vp-controls";
  const reset = document.createElement("button");
  reset.type = "button";
  reset.className = "viz-reset";
  reset.textContent = VIZ.resetZoom;
  reset.disabled = true;
  tools.append(controls, reset);
  head.append(note, tools);
  const body = document.createElement("div");
  const reading = document.createElement("div");
  reading.className = "vp-reading";
  reading.textContent = VIZ.valuePlot.reading;
  body.append(reading);
  root.append(head, body);

  let size = { width, height };
  let src = runSource(doc, path);
  let run: RunValues | null = null;
  /** The run's lowest and highest value, worked out once when it is read:
   *  the line's y scale, and the note over it. */
  let range: { readonly min: number; readonly max: number } | null = null;
  let mode: "line" | "grid" = src?.dims !== null && src?.dims !== undefined ? "grid" : "line";
  let clip = false;
  let drawn: { resize(w: number, h: number): void; destroy(): void; board: Board } | null = null;
  let stopWatch: (() => void) | null = null;
  let frame = 0;
  /** The head row's height before it is on the page, so the plot under it
   *  gets the rest. Once it is on the page its own height is used, since its
   *  note wraps in a narrow pane. This is the figure's own row, not the host. */
  const HEAD_PX = 28;

  const bodySize = (): { width: number; height: number } => ({
    width: size.width,
    height: Math.max(80, size.height - (head.offsetHeight > 0 ? head.offsetHeight + 4 : HEAD_PX)),
  });

  const seg = (choices: { label: string; title: string; on: boolean; press: () => void }[]): HTMLElement => {
    const s = document.createElement("span");
    s.className = "toggle-seg";
    for (const c of choices) {
      const b = document.createElement("button");
      b.type = "button";
      b.textContent = c.label;
      b.title = c.title;
      b.setAttribute("aria-pressed", String(c.on));
      b.addEventListener("click", c.press);
      s.append(b);
    }
    return s;
  };

  const draw = (): void => {
    if (src === null || run === null) return;
    drawn?.destroy();
    drawn = null;
    const s = src;
    const r = run;
    const notes: string[] = [];
    if (r.shown < s.node.child_count) notes.push(VIZ.valuePlot.firstOf(r.shown, s.node.child_count, childWord(s.node)));
    const parts: HTMLElement[] = [];
    if (s.dims !== null && r.shown === s.node.child_count) {
      parts.push(
        seg([
          { label: VIZ.valuePlot.grid, title: VIZ.valuePlot.gridTitle, on: mode === "grid", press: () => setMode("grid") },
          { label: VIZ.valuePlot.line, title: VIZ.valuePlot.lineTitle, on: mode === "line", press: () => setMode("line") },
        ]),
      );
    }
    if (mode === "grid" && s.dims !== null && r.shown === s.node.child_count) {
      notes.unshift(VIZ.valuePlot.shape(s.dims.rows, s.dims.columns));
      parts.push(
        seg([
          { label: VIZ.valuePlot.rangeFull, title: VIZ.valuePlot.rangeFullTitle, on: !clip, press: () => setClip(false) },
          { label: VIZ.valuePlot.rangeClip, title: VIZ.valuePlot.rangeClipTitle, on: clip, press: () => setClip(true) },
        ]),
      );
      const g = drawGridPlot(doc, s, r, { ...bodySize(), clip, onNote: (text) => (note.textContent = [...notes, text].filter((t) => t !== "").join(" · ")) });
      drawn = g;
      body.replaceChildren(g.el);
    } else {
      const data = lineFromRun(s, r);
      // The sentence that goes with the picture: how low and how high the
      // values go, which the y axis otherwise leaves to be read off.
      range ??= finiteRange(data.channels);
      if (range !== null) notes.push(VIZ.valuePlot.oneRange(valueText(range.min, s.type.float), valueText(range.max, s.type.float)));
      const line = drawLinePlot(doc, data, { ...bodySize(), range });
      drawn = line;
      body.replaceChildren(line.el);
    }
    controls.replaceChildren(...parts);
    note.textContent = notes.join(" · ");
    note.title = note.textContent;
    const b = drawn.board;
    const onReset = (): void => b.home(true);
    reset.onclick = onReset;
    reset.disabled = false;
  };

  const setMode = (m: "line" | "grid"): void => {
    mode = m;
    draw();
  };
  const setClip = (c: boolean): void => {
    clip = c;
    draw();
  };

  const attempt = (): void => {
    src ??= runSource(doc, path);
    if (src === null) return;
    run = readRun(doc, src);
    if (run === null) return;
    range = null;
    stopWatch?.();
    stopWatch = null;
    draw();
  };
  attempt();
  // Bytes still arriving: try again once a frame has passed after each chunk,
  // however many land in it.
  if (run === null) {
    stopWatch = doc.onChange(() => {
      if (frame !== 0) return;
      frame = requestAnimationFrame(() => {
        frame = 0;
        attempt();
      });
    });
  }

  return {
    el: root,
    resize: (w, h) => {
      size = { width: w, height: h };
      drawn?.resize(bodySize().width, bodySize().height);
    },
    destroy: () => {
      cancelAnimationFrame(frame);
      stopWatch?.();
      drawn?.destroy();
    },
  };
}
