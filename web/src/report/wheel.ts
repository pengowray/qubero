// Wheel zoom that does not trap the page's scrolling.
//
// A reader scrolling down the report passes the pointer over every figure on
// the way, and a figure that took every wheel event would stop the page dead
// and start zooming. So a figure takes the wheel only when the page is still:
// a wheel event that comes within `SETTLE_MS` of one the page scrolled with
// goes to the page as well, which keeps a scroll that started elsewhere
// scrolling. A figure zoomed all the way out also lets a zoom-out through,
// since there is nothing left for it to do. Every wheel-zoom figure in the
// report goes through here, so they all behave the same.

/** How long after the page last scrolled a wheel event still counts as part
 *  of that scroll. */
export const SETTLE_MS = 400;

/** When the page last took a wheel event itself, by the event clock. */
let pageScrolled = Number.NEGATIVE_INFINITY;
let listening = false;

function listen(): void {
  if (listening) return;
  listening = true;
  // After every figure has had its say: an event no figure took is one the
  // page scrolled with.
  window.addEventListener(
    "wheel",
    (e) => {
      if (!e.defaultPrevented) pageScrolled = e.timeStamp;
    },
    { passive: true },
  );
}

/** Whether a figure should take this wheel event, by the rules above. Pure,
 *  for the tests: `sinceScroll` is how long ago the page last scrolled. */
export function takesWheel(sinceScroll: number, zoomingOut: boolean, fullyOut: boolean): boolean {
  if (sinceScroll < SETTLE_MS) return false;
  if (zoomingOut && fullyOut) return false;
  return true;
}

/**
 * Give `el` wheel zoom. `zoom` is called with each event the figure takes;
 * `fullyOut` says whether the figure is zoomed all the way out.
 */
export function wheelZoom(el: Element, zoom: (e: WheelEvent) => void, fullyOut: () => boolean): void {
  listen();
  el.addEventListener(
    "wheel",
    (ev) => {
      const e = ev as WheelEvent;
      const out = (e.deltaY || e.deltaX) > 0;
      if (!takesWheel(e.timeStamp - pageScrolled, out, fullyOut())) return;
      e.preventDefault();
      zoom(e);
    },
    { passive: false },
  );
}
