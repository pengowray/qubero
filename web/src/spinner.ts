// A small turning Qubero crystal, for text that says something is under way:
// `Calculating…` with the mark beside it. It sits in a line of text at the
// text's own height and colour, and says nothing itself; the words next to it
// carry the meaning, so it is hidden from assistive technology.
//
// A custom element, so it turns only while it is on the page: taken out, its
// animation frame is cancelled with no one having to remember to.

import { CRYSTAL_FACES, drawCrystal } from "./crystal.ts";

const NS = "http://www.w3.org/2000/svg";

/** One turn, in milliseconds. */
const TURN = 1600;

const TAG = "qubero-spinner";

/** A spinner for a line of text. Put it before the words that say what is
 *  being waited for. */
export function spinner(): HTMLElement {
  if (customElements.get(TAG) === undefined) customElements.define(TAG, Spinner);
  return document.createElement(TAG);
}

class Spinner extends HTMLElement {
  private readonly paths = Array.from({ length: CRYSTAL_FACES }, () => document.createElementNS(NS, "path"));
  private readonly outline = document.createElementNS(NS, "path");
  private readonly reduced = matchMedia("(prefers-reduced-motion: reduce)");
  private frame = 0;
  private readonly still = (): void => this.restart();

  constructor() {
    super();
    const svg = document.createElementNS(NS, "svg");
    svg.setAttribute("viewBox", "-40 -36 560 894");
    svg.setAttribute("fill", "var(--bg)");
    svg.setAttribute("stroke", "currentColor");
    // Thicker than the welcome screen's mark: at the height of a line of text
    // its hairlines would vanish.
    svg.setAttribute("stroke-width", "34");
    svg.setAttribute("stroke-linejoin", "round");
    svg.setAttribute("stroke-linecap", "round");
    this.outline.setAttribute("fill", "none");
    this.outline.setAttribute("stroke-width", "64");
    svg.append(...this.paths, this.outline);
    this.append(svg);
    this.setAttribute("aria-hidden", "true");
    drawCrystal(this.paths, this.outline, 0);
  }

  connectedCallback(): void {
    this.reduced.addEventListener("change", this.still);
    this.restart();
  }

  disconnectedCallback(): void {
    this.reduced.removeEventListener("change", this.still);
    cancelAnimationFrame(this.frame);
    this.frame = 0;
  }

  /** Turning, or held still at rest where the reader has asked for less
   *  motion. The words beside it still say the wait is on. */
  private restart(): void {
    cancelAnimationFrame(this.frame);
    this.frame = 0;
    if (this.reduced.matches) {
      drawCrystal(this.paths, this.outline, 0);
      return;
    }
    const tick = (now: number): void => {
      drawCrystal(this.paths, this.outline, ((now % TURN) / TURN) * Math.PI * 2);
      this.frame = requestAnimationFrame(tick);
    };
    this.frame = requestAnimationFrame(tick);
  }
}
