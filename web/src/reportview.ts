// The report: one reading of the whole file from start to end, a level above
// the listing. What the file is, what it holds, where its bytes go, what is
// wrong with it, and its parts in the order a reader needs them, with every
// claim linked to the bytes it rests on. See docs/DESIGN-report-view.md.
//
// This file is the view: the scrolling page, the slots the sections draw into,
// and the handful of interactions every section shares. Each section and each
// figure is a module under `report/`, so one can be changed or added without
// touching the others. The order of the sections is `SECTIONS` below.
//
// Nothing here walks the file. Each section asks the core what it needs,
// bounded, and answers `WAIT` while the bytes are on their way; the view draws
// again when the document says something arrived. A gigabyte file shows its
// title and facts at once, and its map fills in as the byte scan gets on.

import "./report/report.css";
import type { Doc } from "./doc.ts";
import type { OutlineHeading, Viewport } from "./outline.ts";
import { ReportData } from "./report/data.ts";
import { refTip, targetOf } from "./report/refs.ts";
import { hideTip, tipBy } from "./report/tip.ts";
import { WAIT, type ReportCtx, type ReportHost, type Section } from "./report/section.ts";
import { RV } from "./report/text.ts";
import { titleSection } from "./report/title.ts";
import { aboutSection } from "./report/about.ts";
import { briefSection } from "./report/brief.ts";
import { findingsSection } from "./report/findings.ts";
import { contentSection } from "./report/content.ts";
import { bytesSection } from "./report/bytes.ts";
import { directoriesSection } from "./report/directories.ts";
import { partsSection } from "./report/parts.ts";
import { streamsSection } from "./report/streams.ts";
import { pngScanlinesSection } from "./report/pngscanlines.ts";
import { jpegSection } from "./report/jpeg.ts";
import { profileSection } from "./report/profile.ts";
import { termsSection } from "./report/terms.ts";
import { everyByteSection } from "./report/everybyte.ts";

/** The sections, in the order the report reads. Numbered as in the design. */
const SECTIONS: readonly Section[] = [
  titleSection, // 1
  aboutSection, // 2
  briefSection, // 3
  findingsSection, // 4
  contentSection, // 5
  bytesSection, // 6
  directoriesSection, // 7, a stub until the core says where directories point
  partsSection, // 8
  streamsSection, // 9
  pngScanlinesSection, // after 9: a PNG's passes and filters, drawn only for a PNG
  jpegSection, // 9, for a JPEG: where its scan's bits go, and one block traced
  profileSection, // 10, a stub until the core has the format profile
  termsSection, // 11
  everyByteSection, // 12
];

/** How long one pass at drawing sections may take before the page gets the
 *  turn back. A section that is not reached is drawn on the next pass. */
const PASS_MS = 40;
/** How often a figure still filling in (the map while the byte scan runs) is
 *  given another go when nothing else wakes it. */
const LIVE_MS = 60;
/** How long the pointer rests on a reference before its bytes are shown. */
const TIP_DELAY_MS = 150;

type Slot = {
  readonly section: Section;
  readonly el: HTMLElement;
  state: "new" | "wait" | "done";
};

export class ReportView {
  readonly el: HTMLElement;
  private readonly page: HTMLElement;
  private slots: Slot[] = [];
  private data: ReportData;
  private lives: (() => boolean)[] = [];
  private frame = 0;
  private liveTimer = 0;
  private tipTimer = 0;
  private tipFor: Element | null = null;
  /** What the file was when the report was drawn: an edit or another template
   *  makes it a report on something else. */
  private drawnFor: { template: string | null; pieces: number } | null = null;
  private lastViewport = "";

  /** The part the section on screen is about, for the rail to mark. */
  onViewport: (v: Viewport) => void = () => {};

  constructor(
    private readonly doc: Doc,
    private readonly host: ReportHost,
  ) {
    this.el = document.createElement("div");
    this.el.className = "reportview";
    this.el.tabIndex = -1;
    this.el.setAttribute("role", "region");
    this.el.setAttribute("aria-label", RV.regionLabel);
    this.page = document.createElement("div");
    this.page.className = "rv-page";
    this.el.append(this.page);
    this.data = new ReportData(doc, () => this.schedule());
    this.wire();
    doc.onChange(() => this.schedule());
  }

  /** The view has come on screen. */
  show(): void {
    this.checkFile();
    this.schedule();
  }

  /** The listing walked the file and named its parts. The rail draws them;
   *  the report reads parts from the template itself (see `report/model.ts`),
   *  and only needs to know that the file has been walked further. */
  setOutline(_headings: readonly OutlineHeading[]): void {
    this.schedule();
  }

  dispose(): void {
    hideTip();
    cancelAnimationFrame(this.frame);
    clearTimeout(this.liveTimer);
  }

  private get shown(): boolean {
    return !this.el.hidden && this.el.isConnected;
  }

  /** Start again from nothing when the file is no longer the one reported on. */
  private checkFile(): void {
    const now = { template: this.doc.template, pieces: this.doc.pieceCount };
    if (this.drawnFor !== null && this.drawnFor.template === now.template && this.drawnFor.pieces === now.pieces) return;
    this.drawnFor = now;
    this.data = new ReportData(this.doc, () => this.schedule());
    this.lives = [];
    this.page.replaceChildren();
    this.slots = SECTIONS.map((section) => {
      const el = document.createElement("div");
      el.className = "rv-slot";
      el.dataset.section = section.id;
      this.page.append(el);
      return { section, el, state: "new" as const };
    });
  }

  private schedule(): void {
    if (!this.shown || this.frame !== 0) return;
    this.frame = requestAnimationFrame(() => {
      this.frame = 0;
      this.pass();
    });
  }

  private ctx(): ReportCtx {
    return {
      doc: this.doc,
      host: this.host,
      data: this.data,
      live: (update) => this.lives.push(update),
    };
  }

  /** Draw every section still owed, then give the figures still filling in
   *  their turn. */
  private pass(): void {
    if (!this.shown) return;
    this.checkFile();
    const until = performance.now() + PASS_MS;
    const ctx = this.ctx();
    let owed = false;
    for (const slot of this.slots) {
      if (slot.state === "done") continue;
      if (performance.now() > until) {
        owed = true;
        break;
      }
      const out = slot.section.render(ctx);
      if (out === WAIT) {
        slot.state = "wait";
        continue;
      }
      slot.state = "done";
      slot.el.replaceChildren(...(out === null ? [] : [out]));
      this.data.markDrawn(slot.section.id);
    }
    this.lives = this.lives.filter((update) => !update());
    if (owed) this.schedule();
    clearTimeout(this.liveTimer);
    if (this.lives.length > 0 || this.slots.some((s) => s.state === "wait")) {
      this.liveTimer = window.setTimeout(() => this.schedule(), LIVE_MS * (this.lives.length > 0 ? 1 : 10));
    }
    this.markPlace();
  }

  /** The events every reference shares: the bytes on hover, the cursor on a
   *  click, the hex view on a double click. */
  private wire(): void {
    this.el.addEventListener("mouseover", (e) => {
      const hit = targetOf(e.target as Element);
      if (hit === null || hit.el.closest(".rv-map") !== null) return;
      if (hit.el === this.tipFor) return;
      clearTimeout(this.tipTimer);
      this.tipTimer = window.setTimeout(() => {
        this.tipFor = hit.el;
        tipBy(refTip(this.doc, hit.target, hit.label), hit.el.getBoundingClientRect());
      }, TIP_DELAY_MS);
    });
    this.el.addEventListener("mouseout", (e) => {
      const hit = targetOf(e.target as Element);
      const to = targetOf(e.relatedTarget as Element | null);
      if (hit !== null && to !== null && to.el === hit.el) return;
      clearTimeout(this.tipTimer);
      this.hideTip();
    });
    this.el.addEventListener("click", (e) => {
      const hit = targetOf(e.target as Element);
      if (hit === null || hit.el.closest(".rv-map") !== null) return;
      if ((e.target as Element).closest("a[href]:not(.rv-at), button") !== null) return;
      e.preventDefault();
      this.host.pick(hit.target);
    });
    this.el.addEventListener("dblclick", (e) => {
      const hit = targetOf(e.target as Element);
      if (hit === null || hit.el.closest(".rv-map") !== null) return;
      e.preventDefault();
      this.hideTip();
      this.host.go(hit.target);
    });
    this.el.addEventListener("keydown", (e) => {
      if (e.key !== "Enter") return;
      const hit = targetOf(e.target as Element);
      if (hit === null) return;
      e.preventDefault();
      if (e.shiftKey) this.host.go(hit.target);
      else this.host.pick(hit.target);
    });
    this.el.addEventListener("scroll", () => {
      this.hideTip();
      this.markPlace();
    }, { passive: true });
  }

  private hideTip(): void {
    hideTip();
    this.tipFor = null;
  }

  /** Tell the rail which part the section at the top of the screen is about.
   *  Only a section about one part of the file says; the title and the
   *  findings are about the whole of it and leave the mark where it was. */
  private markPlace(): void {
    if (!this.shown) return;
    const top = this.el.getBoundingClientRect().top;
    const line = top + this.el.clientHeight / 3;
    let best: HTMLElement | null = null;
    for (const s of this.page.querySelectorAll<HTMLElement>("[data-rv-part-start]")) {
      const r = s.getBoundingClientRect();
      if (r.top <= line && r.bottom > top) best = s;
      if (r.top > line) break;
    }
    if (best === null) return;
    const v = { startBit: Number(best.dataset.rvPartStart), endBit: Number(best.dataset.rvPartEnd) };
    const key = `${v.startBit}:${v.endBit}`;
    if (key === this.lastViewport) return;
    this.lastViewport = key;
    this.onViewport(v);
  }
}
