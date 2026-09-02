// The open documents, and which one is showing.
//
// A tab is a document: the file the reader opened, a compressed stream unpacked
// out of it, or a run of bytes lifted out of one of those. They share nothing
// but the strip above the toolbar, so switching is a matter of showing one page
// and hiding the rest.
//
// The pages stay in the document while they are hidden rather than being torn
// down and built again. Rebuilding is simpler to write and worse to use: a tab
// switched away from and back would come back scrolled to the top, with every
// folded part of the rail unfolded again and the cursor at offset zero. Since
// the pages are all live at once, anything a page puts on `document` rather
// than inside itself has to check whether it is the one showing, which is what
// `showing` is for.

import { el } from "./dom.js";
import type { Doc } from "./doc.js";

export type Tab = {
  readonly doc: Doc;
  /** What the strip calls it. */
  readonly title: string;
  /** Where the bytes came from, shown on hover. Null for a file from disk. */
  readonly origin: string | null;
  /** The file the reader opened cannot be closed: closing it would leave the
   *  page showing tabs of bytes with nothing they came out of. Everything
   *  opened out of it can. */
  readonly closable: boolean;
  /** The page built for this tab, once it has been shown at least once. */
  page: HTMLElement | null;
  /** Run every time the page becomes the one showing, and once just after it
   *  is first put in the document. A view measures itself to lay out, and an
   *  element that is not in the document measures nothing. */
  shown: () => void;
  /** Undo for everything the page put outside itself. Run when it closes. */
  release: (() => void)[];
};

/** A tab's page, and how to wake it when it comes to the front. */
export type Page = {
  readonly el: HTMLElement;
  readonly shown: () => void;
};

/** What a tab needs before it has a page. */
export type NewTab = {
  doc: Doc;
  title: string;
  origin?: string | null;
  closable?: boolean;
};

export class Tabs {
  private list: Tab[] = [];
  private at = 0;
  /** Where the strip and the pages go. */
  private readonly host: HTMLElement;
  /** Builds the page for a tab the first time it is shown. `shown` is called
   *  once it is in the document, and again whenever it comes back. */
  private readonly build: (tab: Tab) => Page;

  /** Asked before a tab with unsaved edits closes. True to go ahead. */
  onConfirmClose: (tab: Tab) => boolean = () => true;
  /** The last tab closed, so the page can go back to the start screen. */
  onEmpty: () => void = () => {};
  /** A different tab is showing now. */
  onSwitch: (tab: Tab) => void = () => {};

  constructor(host: HTMLElement, build: (tab: Tab) => Page) {
    this.host = host;
    this.build = build;
  }

  get all(): readonly Tab[] {
    return this.list;
  }

  get active(): number {
    return this.at;
  }

  get current(): Tab | undefined {
    return this.list[this.at];
  }

  get doc(): Doc | null {
    return this.list[this.at]?.doc ?? null;
  }

  /** True while `tab` is the one the reader is looking at. A page's document
   *  level listeners ask this before acting: every open tab's page is live, and
   *  only one of them is being read. */
  showing(tab: Tab): boolean {
    return this.list[this.at] === tab;
  }

  /** The first tab with unsaved edits, which is what closing the page or
   *  opening another file would throw away. */
  modified(): Tab | null {
    return this.list.find((t) => t.doc.modified) ?? null;
  }

  /** The tab showing a given space of the file, if it is already open. */
  forSpace(space: number): number {
    return this.list.findIndex((t) => t.doc.space === space);
  }

  /** Show this file on its own, closing whatever was open. */
  only(t: NewTab): void {
    for (const tab of this.list) this.discard(tab);
    this.list = [];
    this.at = 0;
    // Whatever else was in the host goes too: the start screen, or the strip
    // left over from the documents just closed.
    this.host.replaceChildren();
    this.add({ ...t, closable: false });
  }

  /** Open another document beside the ones already open, and show it. */
  add(t: NewTab): void {
    this.list.push({
      doc: t.doc,
      title: t.title,
      origin: t.origin ?? null,
      closable: t.closable ?? true,
      page: null,
      shown: () => {},
      release: [],
    });
    this.at = this.list.length - 1;
    this.render();
  }

  /** Show the tab at `i`. */
  focus(i: number): void {
    if (i < 0 || i >= this.list.length || i === this.at) return;
    this.at = i;
    this.render();
  }

  /** Close one tab. Its document is gone for good, so unsaved edits ask first. */
  close(i: number): void {
    const tab = this.list[i];
    if (tab === undefined || !tab.closable) return;
    if (tab.doc.modified && !this.onConfirmClose(tab)) return;
    this.discard(tab);
    this.list.splice(i, 1);
    if (this.at >= this.list.length) this.at = this.list.length - 1;
    else if (i < this.at) this.at -= 1;
    if (this.list.length === 0) {
      this.onEmpty();
      return;
    }
    this.render();
  }

  private discard(tab: Tab): void {
    for (const off of tab.release) off();
    tab.release = [];
    tab.page?.remove();
    tab.page = null;
  }

  /** Put the strip and every page in the host, with one page showing. */
  render(): void {
    const current = this.list[this.at];
    if (current === undefined) return;
    if (current.page === null) {
      const built = this.build(current);
      current.page = built.el;
      current.shown = built.shown;
    }
    for (const t of this.list) if (t.page !== null) t.page.hidden = t !== current;
    // Only what is not already here is put here. Taking a page out of the
    // document and putting it back resets every scroll container inside it, and
    // the whole point of keeping the pages is that a tab comes back where it
    // was left.
    for (const t of this.list) {
      if (t.page !== null && !t.page.isConnected) this.host.append(t.page);
    }
    this.refresh();
    current.shown();
    this.onSwitch(current);
  }

  /** Redraw the strip alone, for a change that is only about the tabs: a
   *  document picking up unsaved edits, say. */
  refresh(): void {
    const old = this.host.querySelector(".tabstrip");
    if (this.list.length < 2) {
      old?.remove();
      return;
    }
    const strip = this.strip();
    if (old === null) this.host.prepend(strip);
    else old.replaceWith(strip);
  }

  private strip(): HTMLElement {
    const strip = el("nav", { className: "tabstrip" });
    strip.setAttribute("aria-label", "Open documents");
    const list = el("div", { className: "tabstrip-tabs" });
    list.setAttribute("role", "tablist");
    this.list.forEach((tab, i) => {
      const here = i === this.at;
      const pick = el("button", { type: "button", className: "tab-pick", textContent: tab.title });
      pick.setAttribute("role", "tab");
      pick.setAttribute("aria-selected", String(here));
      if (tab.origin !== null) pick.title = tab.origin;
      if (!here) pick.addEventListener("click", () => this.focus(i));
      const item = el("div", { className: here ? "tab is-active" : "tab" }, pick);
      if (tab.closable) {
        const close = el("button", { type: "button", className: "tab-close", textContent: "×" });
        close.title = `Close ${tab.title}`;
        close.setAttribute("aria-label", `Close ${tab.title}`);
        close.addEventListener("click", () => this.close(i));
        item.append(close);
      }
      if (tab.doc.modified) item.classList.add("is-edited");
      list.append(item);
    });
    strip.append(list);
    return strip;
  }
}
