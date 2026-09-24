// The one hover box the report shows exact values in: the bytes behind a
// reference, a part of the map, a band of a ribbon. One element for the whole
// page, since only one thing is under the pointer at a time.

let box: HTMLElement | null = null;

function element(): HTMLElement {
  if (box === null || !box.isConnected) {
    box = document.createElement("div");
    box.className = "rv-tip";
    box.setAttribute("role", "tooltip");
    box.hidden = true;
    document.body.append(box);
  }
  return box;
}

/** Show the box beside a point, kept on screen. */
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

/** Show the box under an element, or over it where there is no room below. */
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
