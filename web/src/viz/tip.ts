// The one hover card every picture shows its values in: the field under the
// pointer, its bytes, a band of a ribbon, a cell of a raster. One element for
// the whole page, since only one thing is under the pointer at a time, shown
// beside the pointer and kept on screen.
//
// First written for the report (`report/tip.ts`, which now hands its callers
// on to this) and promoted when the diagram pieces needed the same thing.
// Every piece puts its values here rather than writing them on the drawing,
// which is what lets the drawings stay unlabelled where a label would not
// fit: the rule in `fit.ts` drops a name that does not fit, and this is where
// a reader finds it. It also replaces the native `title` attribute, which
// waits a second before showing, cannot be styled, and never shows at all on
// a canvas.
//
// The styles are `.viz-tip` in `style.css`, loaded with the page, so a tip
// from the rail looks the same as one from the report.

let box: HTMLElement | null = null;

function element(): HTMLElement {
  if (box === null || !box.isConnected) {
    box = document.createElement("div");
    // Both names: `rv-tip` is the report's, whose own rules style what the
    // report puts inside it (`.rv-tip .rv-dump`).
    box.className = "viz-tip rv-tip";
    box.setAttribute("role", "tooltip");
    box.hidden = true;
    document.body.append(box);
  }
  return box;
}

/** Show the card beside a point, kept on screen. */
export function tipAt(content: Node | string, clientX: number, clientY: number): void {
  const t = element();
  t.replaceChildren(content);
  t.hidden = false;
  const w = t.offsetWidth;
  const h = t.offsetHeight;
  let x = clientX + 14;
  let y = clientY + 16;
  if (x + w > window.innerWidth - 8) x = clientX - w - 14;
  if (y + h > window.innerHeight - 8) y = clientY - h - 14;
  t.style.left = `${Math.max(8, x)}px`;
  t.style.top = `${Math.max(8, y)}px`;
}

/** Show the card under an element, or over it where there is no room below. */
export function tipBy(content: Node | string, r: DOMRect): void {
  const t = element();
  t.replaceChildren(content);
  t.hidden = false;
  const w = t.offsetWidth;
  const h = t.offsetHeight;
  const x = Math.max(8, Math.min(r.left, window.innerWidth - w - 8));
  let y = r.bottom + 6;
  if (y + h > window.innerHeight - 8 && r.top - h - 6 > 0) y = r.top - h - 6;
  t.style.left = `${x}px`;
  t.style.top = `${y}px`;
}

export function hideTip(): void {
  if (box !== null) box.hidden = true;
}

/**
 * The usual content of a card: a name in bold, lines of values under it, and
 * where in the file in the muted line at the foot. A piece with more to say
 * builds its own node; this is for the common case, so that most cards look
 * alike and a reader learns where on a card to look.
 */
export function tipCard(title: string, lines: readonly string[] = [], where = ""): HTMLElement {
  const card = document.createElement("div");
  const head = document.createElement("b");
  head.className = "viz-tip-title";
  head.textContent = title;
  card.append(head);
  for (const line of lines) {
    const row = document.createElement("div");
    row.textContent = line;
    card.append(row);
  }
  if (where !== "") {
    const foot = document.createElement("div");
    foot.className = "viz-tip-where";
    foot.textContent = where;
    card.append(foot);
  }
  return card;
}

/**
 * Show a card while the pointer moves over `el`, and hide it when the pointer
 * leaves or presses a button (a drag is not a hover). `content` is asked on
 * every move with the event, so a canvas can hit-test the pointer; it answers
 * null where there is nothing under the pointer, which hides the card.
 * Returns the way to stop.
 */
export function hoverTip(el: Element, content: (e: PointerEvent) => Node | string | null): () => void {
  const move = (ev: Event): void => {
    const e = ev as PointerEvent;
    if (e.buttons !== 0) {
      hideTip();
      return;
    }
    const c = content(e);
    if (c === null) hideTip();
    else tipAt(c, e.clientX, e.clientY);
  };
  const leave = (): void => hideTip();
  el.addEventListener("pointermove", move);
  el.addEventListener("pointerleave", leave);
  el.addEventListener("pointerdown", leave);
  return () => {
    el.removeEventListener("pointermove", move);
    el.removeEventListener("pointerleave", leave);
    el.removeEventListener("pointerdown", leave);
  };
}
