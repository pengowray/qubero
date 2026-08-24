// Finding things, and putting other things in their place.
//
// A search runs a window at a time so the page never freezes on a file too
// large to hold. This drives that loop on a time budget: it keeps stepping
// while there are milliseconds left in the frame, then yields and carries on,
// so a scan over gigabytes still repaints and still takes a click to stop.

import type { Doc, NeedleKind, Query } from "./doc.js";
import {
  BAD_REPLACEMENT,
  COUNTED,
  COUNTING,
  COUNT_STOPPED,
  NO_MATCH,
  REPLACED,
  SEARCH_LABELS,
  SEARCH_PLACEHOLDER,
  WRAPPED_BACK,
  WRAPPED_ON,
} from "./strings.js";

/** Milliseconds of one frame a search may take before yielding. */
const SLICE = 8;

export type Match = { readonly at: number; readonly len: number };

function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  props: Partial<HTMLElementTagNameMap[K]> = {},
  ...children: (Node | string)[]
): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  Object.assign(e, props);
  e.append(...children);
  return e;
}

export class SearchBar {
  readonly el: HTMLElement;
  private readonly find: HTMLInputElement;
  private readonly repl: HTMLInputElement;
  private readonly kind: HTMLSelectElement;
  private readonly fold: HTMLInputElement;
  private readonly status: HTMLElement;
  private readonly replaceRow: HTMLElement;
  private readonly countBtn: HTMLButtonElement;
  private readonly expand: HTMLButtonElement;
  private readonly foldLabel: HTMLLabelElement;

  /** The match the cursor is on, which is what "the next one" is next to.
   *  Direction decides which side of it a search starts from, so turning
   *  round does not find the same one again. */
  private last: Match | null = null;
  private origin = 0;
  private wrapped = false;
  /** Set while a loop is running, so a second Enter does not start a race. */
  private running = false;
  /** Raised to stop whatever loop is running. */
  private cancel = false;

  /** Where to look from, and what to do with what is found. */
  onCursor: () => number = () => 0;
  onFound: (m: Match) => void = () => {};

  constructor(private readonly doc: Doc) {
    this.find = el("input", { type: "text", className: "sb-find" });
    this.find.setAttribute("aria-label", SEARCH_LABELS.find);
    this.repl = el("input", { type: "text", className: "sb-repl" });
    this.repl.setAttribute("aria-label", SEARCH_LABELS.replace);

    this.kind = el("select", { className: "sb-kind" });
    this.kind.setAttribute("aria-label", SEARCH_LABELS.kind);
    for (const [value, label] of Object.entries(SEARCH_LABELS.kinds)) {
      this.kind.append(el("option", { value, textContent: label }));
    }

    this.fold = el("input", { type: "checkbox", className: "sb-fold" });
    const foldLabel = el("label", { className: "sb-foldbox", title: SEARCH_LABELS.foldNote }, this.fold, SEARCH_LABELS.fold);
    this.foldLabel = foldLabel;

    const next = this.button(SEARCH_LABELS.next, () => void this.run(false));
    const prev = this.button(SEARCH_LABELS.previous, () => void this.run(true));
    // While a count is running this is the only way to stop it. Closing the
    // bar also stops it, but the message then lands on a bar that is going.
    this.countBtn = this.button(SEARCH_LABELS.count, () => {
      if (this.running) this.cancel = true;
      else void this.count();
    });
    const count = this.countBtn;
    const one = this.button(SEARCH_LABELS.replaceOne, () => void this.replaceOne());
    const all = this.button(SEARCH_LABELS.replaceAll, () => void this.replaceAll());
    const close = this.button(SEARCH_LABELS.close, () => this.hide());
    close.classList.add("sb-close");

    this.status = el("span", { className: "sb-status" });
    this.status.setAttribute("role", "status");

    this.expand = this.button(SEARCH_LABELS.showReplace, () => this.setReplace(this.replaceRow.hidden));
    this.expand.className = "sb-expand";
    this.replaceRow = el("div", { className: "sb-row sb-replace" }, this.repl, one, all);
    this.el = el(
      "div",
      { className: "searchbar" },
      el("div", { className: "sb-row" }, this.expand, this.kind, this.find, foldLabel, prev, next, count, close),
      this.status,
      this.replaceRow,
    );
    this.el.hidden = true;
    this.setReplace(false);
    this.setKind("text");

    this.find.addEventListener("keydown", (e) => this.onKey(e));
    this.repl.addEventListener("keydown", (e) => this.onKey(e));
    this.el.addEventListener("keydown", (e) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        this.hide();
      }
    });
    this.find.addEventListener("input", () => this.recheck(true));
    this.kind.addEventListener("change", () => this.setKind(this.kind.value as NeedleKind));
    // Half a hex byte is not a mistake until you stop typing.
    this.find.addEventListener("blur", () => this.recheck(false));
  }

  /** Folding is a property of text: a hex needle has no case, and a pattern
   *  says so itself with `(?i)`. The box is disabled rather than hidden, so
   *  the buttons beside it do not move when the kind changes. */
  private setKind(kind: NeedleKind): void {
    this.fold.disabled = kind !== "text";
    this.foldLabel.classList.toggle("is-off", kind !== "text");
    this.find.placeholder = SEARCH_PLACEHOLDER[kind].find;
    this.repl.placeholder = SEARCH_PLACEHOLDER[kind].replace;
    this.recheck(true);
  }

  private setReplace(show: boolean): void {
    this.replaceRow.hidden = !show;
    this.expand.textContent = show ? SEARCH_LABELS.hideReplace : SEARCH_LABELS.showReplace;
    this.expand.setAttribute("aria-expanded", String(show));
  }

  private button(text: string, run: () => void): HTMLButtonElement {
    const b = el("button", { type: "button", textContent: text });
    b.addEventListener("click", run);
    return b;
  }

  private onKey(e: KeyboardEvent): void {
    if (e.key !== "Enter") return;
    e.preventDefault();
    void this.run(e.shiftKey);
  }

  // ----- showing -----

  show(replacing = false): void {
    this.el.hidden = false;
    if (replacing) this.setReplace(true);
    this.find.focus();
    this.find.select();
  }

  hide(): void {
    this.cancel = true;
    this.el.hidden = true;
    this.status.textContent = "";
  }

  get open(): boolean {
    return !this.el.hidden;
  }

  private query(backward: boolean): Query {
    return {
      kind: this.kind.value as NeedleKind,
      text: this.find.value,
      fold: this.fold.checked && this.kind.value === "text",
      backward,
    };
  }

  /** Say what is wrong with the needle as it is typed, and nothing when it is
   *  fine: a search box that only complains when you press Enter makes you
   *  press Enter to find out. */
  private recheck(typing: boolean): void {
    const why = this.doc.checkNeedle(this.kind.value as NeedleKind, this.find.value, typing);
    this.find.classList.toggle("invalid", why !== "" && this.find.value !== "");
    this.status.textContent = this.find.value === "" ? "" : why;
    this.origin = -1;
  }

  // ----- the loop -----

  /**
   * Step until something is found, the file runs out, or the budget for this
   * frame does. Pending chunks are fetched by the document itself, so a
   * pending step is a yield and not a failure.
   */
  private async scan(q: Query, from: number): Promise<Match | null> {
    let at = from;
    let since = performance.now();
    for (;;) {
      if (this.cancel) return null;
      const r = this.doc.searchStep(q, at);
      if (r.status === "error") {
        this.status.textContent = r.message;
        return null;
      }
      if (r.status === "pending") {
        await frame();
        since = performance.now();
        continue;
      }
      const step = r.node;
      if (step.step === "found") return { at: step.at, len: step.len };
      if (step.step === "end") return null;
      at = step.resume;
      if (performance.now() - since > SLICE) {
        await frame();
        since = performance.now();
      }
    }
  }

  /** Where a search starts: past the match the cursor is on going forwards,
   *  at its first byte going backwards, and the cursor itself when there is no
   *  match behind us. */
  private from(backward: boolean): number {
    if (this.last === null) return this.origin;
    return backward ? this.last.at : this.last.at + Math.max(1, this.last.len);
  }

  /** Find the next match, wrapping once. */
  private async run(backward: boolean): Promise<void> {
    if (this.running || this.find.value === "") return;
    const q = this.query(backward);
    if (this.doc.checkNeedle(q.kind, q.text, false) !== "") {
      this.recheck(false);
      return;
    }
    this.running = true;
    this.cancel = false;
    try {
      // A fresh search starts at the cursor; carrying on continues from the
      // last match, so the same one is not found twice.
      if (this.origin < 0) {
        this.origin = this.onCursor();
        this.last = null;
      }
      this.wrapped = false;
      let found = await this.scan(q, this.from(backward));
      if (found === null && !this.cancel) {
        // Round once, and say so. Going round is what a reader expects of a
        // Next button, but landing at the other end of the file without a word
        // is a jump they did not ask for. A second pass that finds nothing
        // means there is nothing, since it covered the whole file.
        found = await this.scan(q, backward ? this.doc.lengthBytes : 0);
        this.wrapped = found !== null;
      }
      if (found === null) {
        if (!this.cancel) this.status.textContent = NO_MATCH;
        return;
      }
      this.status.textContent = this.wrapped ? (backward ? WRAPPED_BACK : WRAPPED_ON) : "";
      this.last = found;
      this.onFound(found);
    } finally {
      this.running = false;
    }
  }

  /** Count every match. This is the one thing here that reads the whole file,
   *  so it says it is still counting rather than showing a number that is not
   *  the answer yet. */
  private async count(): Promise<void> {
    if (this.running || this.find.value === "") return;
    const q = this.query(false);
    if (this.doc.checkNeedle(q.kind, q.text, false) !== "") return;
    this.running = true;
    this.cancel = false;
    this.countBtn.textContent = SEARCH_LABELS.stop;
    try {
      let n = 0;
      let at = 0;
      for (;;) {
        const found = await this.scan(q, at);
        if (found === null) break;
        n += 1;
        at = found.at + Math.max(1, found.len);
        if (n % 64 === 0) {
          this.status.textContent = COUNTING(n);
          await frame();
        }
      }
      this.status.textContent = this.cancel ? COUNT_STOPPED(n) : COUNTED(n);
    } finally {
      this.countBtn.textContent = SEARCH_LABELS.count;
      this.running = false;
    }
  }

  // ----- replacing -----

  /** The replacement as bytes, read the way the needle is. A pattern has no
   *  bytes of its own, so its replacement is taken as text. */
  private replacement(): Uint8Array | null {
    if (this.kind.value === "hex") {
      const text = this.repl.value.trim();
      if (text === "") return new Uint8Array();
      if (!/^([0-9a-f]{2}\s*)+$/i.test(text)) return null;
      const pairs = text.replace(/\s+/g, "").match(/../g) ?? [];
      return Uint8Array.from(pairs, (p) => parseInt(p, 16));
    }
    return new TextEncoder().encode(this.repl.value);
  }

  private async replaceOne(): Promise<void> {
    const bytes = this.replacement();
    if (bytes === null) {
      this.status.textContent = BAD_REPLACEMENT;
      return;
    }
    const q = this.query(false);
    if (this.find.value === "" || this.doc.checkNeedle(q.kind, q.text, false) !== "") return;
    if (this.origin < 0) {
      this.origin = this.onCursor();
      this.last = null;
    }
    const found = await this.scan(q, this.from(false));
    if (found === null) {
      this.status.textContent = NO_MATCH;
      return;
    }
    this.doc.replaceAt(found.at, found.len, bytes);
    this.last = { at: found.at, len: bytes.length };
    this.status.textContent = REPLACED(1);
    this.onFound({ at: found.at, len: bytes.length });
  }

  private async replaceAll(): Promise<void> {
    const bytes = this.replacement();
    if (bytes === null) {
      this.status.textContent = BAD_REPLACEMENT;
      return;
    }
    const q = this.query(false);
    if (this.running || this.find.value === "" || this.doc.checkNeedle(q.kind, q.text, false) !== "") return;
    this.running = true;
    this.cancel = false;
    // One thing the user did, so one thing to undo.
    this.doc.beginBatch();
    try {
      let n = 0;
      let at = 0;
      for (;;) {
        const found = await this.scan(q, at);
        if (found === null) break;
        this.doc.replaceAt(found.at, found.len, bytes);
        n += 1;
        // Carry on from the end of what was written: a replacement of a
        // different length has moved every byte behind it.
        at = found.at + Math.max(1, bytes.length);
        if (n % 64 === 0) await frame();
      }
      this.status.textContent = REPLACED(n);
      this.origin = -1;
    } finally {
      this.doc.endBatch();
      this.running = false;
    }
  }

  /** Start again from the cursor, after it has been moved by something else. */
  reset(): void {
    this.origin = -1;
  }
}

/** Yield to the browser so the page repaints. */
function frame(): Promise<void> {
  return new Promise((r) => requestAnimationFrame(() => r()));
}
