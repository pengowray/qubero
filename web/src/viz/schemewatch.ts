// Repaint a canvas figure when the page turns light or dark, without the
// listener keeping the figure alive.
//
// A canvas takes its colours from the stylesheet when it paints, so a change
// of theme has to be a repaint. The obvious way, a `change` listener on the
// media query per figure, holds the figure for as long as the page lives
// unless the figure is destroyed, and the At cursor panel replaces its
// figures without destroying them (its section registry has no hook for it):
// every redraw of a miniSEED record's samples left a uPlot behind.
//
// So there is one listener for the page, and it holds each figure's element
// weakly. The repaint is kept in a WeakMap under that element, which keeps it
// exactly as long as the element is reachable from anything else: a figure
// nobody shows any more is collected with its element, and its entry goes the
// next time the theme changes. Written for `valueplot.ts` and `valuegrid.ts`.

const repaints = new WeakMap<Element, () => void>();
const figures = new Set<WeakRef<Element>>();
let listening = false;

function repaintAll(): void {
  for (const ref of figures) {
    const el = ref.deref();
    const repaint = el === undefined ? undefined : repaints.get(el);
    if (repaint === undefined) figures.delete(ref);
    else repaint();
  }
}

/** Call `repaint` whenever the page turns light or dark, for as long as `el`
 *  exists. Returns the way to stop earlier, for a figure that is destroyed. */
export function onSchemeChange(el: Element, repaint: () => void): () => void {
  if (!listening && typeof matchMedia === "function") {
    listening = true;
    matchMedia("(prefers-color-scheme: dark)").addEventListener("change", repaintAll);
  }
  // Entries whose element has been collected go now as well as on a change
  // of theme, so a page that never changes theme does not gather them.
  for (const old of figures) if (old.deref() === undefined) figures.delete(old);
  const ref = new WeakRef(el);
  repaints.set(el, repaint);
  figures.add(ref);
  return () => {
    repaints.delete(el);
    figures.delete(ref);
  };
}
