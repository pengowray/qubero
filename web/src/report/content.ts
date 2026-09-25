// Section 5: the content, what a reader opened the file for, before any of the
// structure that encodes it. Three kinds today, the first that fits:
//
// - a picture the browser can draw, through the same card the listing opens
//   with (`contentcard.ts`);
// - a table, for a template that says a list reads as one (`TableShape`): its
//   facts and first rows, through the plan the table view uses;
// - text, for the longest run of text the template reads.
//
// The design's other roles (samples as a waveform, events in time, outlines)
// need a declared content role on the template first. See "Content" in
// docs/DESIGN-report-view.md.

import { cardKind, cardState, watchCard } from "../contentcard.ts";
import { formatBytes } from "../format.ts";
import { tablePlan } from "../tableplan.ts";
import { archiveContent } from "./archive.ts";
import { factRows, planTable } from "./bodies.ts";
import { ok } from "./model.ts";
import { WAIT, type ReportCtx, type Rendered, type Section } from "./section.ts";
import { RV } from "./text.ts";
import { walkTemplate } from "./walk.ts";

/** Rows of a table shown here; the table view has the rest. */
const ROWS = 10;
/** Characters of text shown. */
const TEXT_CHARS = 2000;
/** A picture whose longer side is under this is magnified to reach it, the
 *  rule the listing's card follows. */
const TINY_PX = 128;

export const contentSection: Section = {
  id: "content",
  render(ctx: ReportCtx): Rendered {
    if (cardKind(ctx.doc.template) !== null) return picture(ctx);
    if (ctx.doc.template === null) return plainText(ctx);
    const walk = ctx.data.memo("walk", () => walkTemplate(ctx.doc));
    if (walk === WAIT) return WAIT;
    for (const node of walk.tables) {
      const plan = tablePlan(ctx.doc, node);
      if (plan === null) continue;
      const t = planTable(plan, ROWS);
      if (!t.complete) return WAIT;
      const sec = section(node.offset_bits, node.offset_bits + node.size_bits);
      const h = document.createElement("h2");
      h.textContent = RV.tableHeading(plan.count, plan.rowWord, plan.rate);
      sec.append(h);
      if (plan.facts.length > 0) {
        const dl = document.createElement("dl");
        dl.className = "rv-inline-facts";
        for (const f of factRows(plan.facts)) {
          const dt = document.createElement("dt");
          const name = document.createElement("code");
          name.textContent = f.name;
          dt.append(name);
          const dd = document.createElement("dd");
          dd.textContent = f.value;
          dl.append(dt, dd);
        }
        sec.append(dl);
      }
      sec.append(t.el);
      const foot = document.createElement("p");
      foot.className = "rv-note";
      foot.append(`${RV.firstRows(Math.min(ROWS, plan.count), plan.count, plan.rowWord)} `);
      const open = document.createElement("button");
      open.type = "button";
      open.className = "rv-button";
      open.textContent = RV.openTable;
      open.addEventListener("click", () => ctx.host.openTable(node.path));
      foot.append(open);
      sec.append(foot);
      return sec;
    }
    // A container's content is what it holds: an archive's files, an HDF5
    // file's objects.
    const archive = archiveContent(ctx.doc);
    if (archive !== null) return archive;
    const text = walk.texts[0];
    if (text !== undefined) {
      const r = ok(ctx.doc.fieldText(text.path));
      if (r === WAIT) return WAIT;
      if (r === null || r.text.trim() === "") return null;
      const sec = section(text.offset_bits, text.offset_bits + text.size_bits);
      const h = document.createElement("h2");
      h.append(...headingWithName(text.name, text.value_bytes));
      sec.append(h);
      const pre = document.createElement("pre");
      pre.className = "rv-text";
      pre.textContent = r.text.slice(0, TEXT_CHARS);
      sec.append(pre);
      if (r.text.length > TEXT_CHARS || r.truncated) {
        const p = document.createElement("p");
        p.className = "rv-note";
        p.textContent = RV.textCut(TEXT_CHARS, r.text.length);
        sec.append(p);
      }
      return sec;
    }
    return null;
  },
};

/** How much of a file with no template is read to decide whether it is text,
 *  and the share of those bytes that must be. */
const TEXT_SNIFF = 4096;
const TEXT_SHARE = 0.95;

/** A file no template reads that is text: its first lines, as UTF-8. The
 *  Text view reads it in full, in whatever encoding it turns out to be. */
function plainText(ctx: ReportCtx): Rendered {
  const n = Math.min(ctx.doc.lengthBytes, TEXT_SNIFF);
  if (n === 0) return null;
  const r = ctx.doc.read(0, n);
  if (!r.complete) return WAIT;
  let text = 0;
  for (const b of r.bytes) if ((b >= 0x20 && b !== 0x7f) || b === 9 || b === 10 || b === 13) text++;
  if (text / n < TEXT_SHARE) return null;
  const sec = section(0, ctx.doc.lengthBits);
  const h = document.createElement("h2");
  h.textContent = RV.plainTextHeading(ctx.doc.lengthBytes);
  const pre = document.createElement("pre");
  pre.className = "rv-text";
  pre.textContent = new TextDecoder("utf-8").decode(r.bytes).slice(0, TEXT_CHARS);
  sec.append(h, pre);
  if (ctx.doc.lengthBytes > n || pre.textContent.length >= TEXT_CHARS) {
    const p = document.createElement("p");
    p.className = "rv-note";
    p.textContent = RV.plainTextCut(pre.textContent.length);
    sec.append(p);
  }
  return sec;
}

/** `The text of <code>name</code>: 4,886 bytes`, with the name in code font. */
function headingWithName(name: string, bytes: number): (Node | string)[] {
  const code = document.createElement("code");
  code.textContent = name;
  const [before, after] = RV.textHeading("\u0000", bytes).split("\u0000");
  return [before ?? "", code, after ?? ""];
}

function section(startBit: number, endBit: number): HTMLElement {
  const sec = document.createElement("section");
  sec.className = "rv-section rv-content";
  sec.dataset.rvPartStart = String(startBit);
  sec.dataset.rvPartEnd = String(endBit);
  return sec;
}

/** The picture, decoded by the listing's content card (`contentcard.ts`, one
 *  decode per document for both views), and its size in the heading once it is
 *  known. A picture a few pixels across is drawn a whole number of screen
 *  pixels to each of its own, square, as the card draws it. */
function picture(ctx: ReportCtx): HTMLElement {
  const sec = section(0, ctx.doc.lengthBits);
  const h = document.createElement("h2");
  const body = document.createElement("div");
  body.className = "rv-picture";
  sec.append(h, body);
  const draw = (): boolean => {
    const s = cardState(ctx.doc);
    if (s === null) return true;
    switch (s.status) {
      case "ready": {
        h.textContent = RV.pictureHeading(s.width, s.height);
        const img = document.createElement("img");
        img.src = s.url;
        img.width = s.width;
        img.height = s.height;
        img.alt = RV.pictureSize(s.width, s.height);
        const zoom = Math.max(1, Math.floor(TINY_PX / Math.max(1, s.width, s.height)));
        if (zoom > 1) {
          img.style.width = `${s.width * zoom}px`;
          img.style.height = "auto";
          img.style.imageRendering = "pixelated";
        }
        body.replaceChildren(img);
        return true;
      }
      case "failed":
        h.textContent = RV.pictureFailedHeading;
        body.replaceChildren(RV.pictureFailed);
        return true;
      case "too-large":
        h.textContent = RV.pictureFailedHeading;
        body.replaceChildren(RV.pictureTooLarge(formatBytes(s.bytes)));
        return true;
      default:
        h.textContent = RV.pictureHeadingWait;
        return false;
    }
  };
  if (!draw()) {
    const stop = watchCard(ctx.doc, (what) => {
      if (what === "state" && draw()) stop();
    });
  }
  return sec;
}
