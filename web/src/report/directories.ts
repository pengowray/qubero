// Section 7: what each directory points to. For every list whose elements
// place something elsewhere in the file (a ZIP's central directory, the TTF
// table directory, ELF's section headers, TIFF's IFD entries), the elements in
// stored order along the top, joined by the ribbon to what each places, in
// file order along the bottom. The core finds them with nothing
// format-specific: see "What each directory points to" in
// docs/DESIGN-report-view.md.

import { formatOffset } from "../format.ts";
import type { Directory, DirectoryTarget } from "./coredata.ts";
import { groupAt, type PartsModel } from "./model.ts";
import { stripIndex } from "./partrules.ts";
import { ribbon, type RibbonData } from "./ribbon.ts";
import { WAIT, type ReportCtx, type Rendered, type Section } from "./section.ts";
import { hideTip, tipAt } from "./tip.ts";
import { bitsText, RV } from "./text.ts";

/** Directories drawn, largest first, and entries drawn in one ribbon. */
const LISTS_SHOWN = 6;
const ENTRIES_SHOWN = 128;

export const directoriesSection: Section = {
  id: "directories",
  render(ctx: ReportCtx): Rendered {
    const core = ctx.data.core();
    if (core === null || core.failed !== null) return null;
    const dirs = core.dirs;
    if (dirs === null || !dirs.done) return WAIT;
    const lists = dirs.lists.filter((l) => l.placing > 0 && l.entries.length > 0).sort((a, b) => b.placing - a.placing);
    if (lists.length === 0) return null;
    const model = ctx.data.parts();
    const parts = model === WAIT ? null : model;
    const sec = document.createElement("section");
    sec.className = "rv-section rv-directories";
    const h = document.createElement("h2");
    h.textContent = RV.directoriesHeading(lists.length);
    sec.append(h);
    for (const l of lists.slice(0, LISTS_SHOWN)) sec.append(directoryBlock(ctx, l, parts));
    const notes: string[] = [];
    if (lists.length > LISTS_SHOWN) notes.push(RV.moreDirectories(lists.length - LISTS_SHOWN));
    if (dirs.unexamined > 0) notes.push(RV.dirUnexamined(dirs.unexamined));
    if (notes.length > 0) {
      const p = document.createElement("p");
      p.className = "rv-note";
      p.textContent = notes.join(" ");
      sec.append(p);
    }
    return sec;
  },
};

/** True when the targets come in the order of the entries that place them. */
export function sameOrder(firsts: readonly number[]): boolean {
  for (let i = 1; i < firsts.length; i++) if ((firsts[i] ?? 0) < (firsts[i - 1] ?? 0)) return false;
  return true;
}

function directoryBlock(ctx: ReportCtx, l: Directory, model: PartsModel | null): HTMLElement {
  const box = document.createElement("div");
  box.className = "rv-directory";
  const first = l.entries[0];
  const last = l.entries[l.entries.length - 1];
  if (first !== undefined && last !== undefined) {
    box.dataset.rvPartStart = String(first.offset_bits);
    box.dataset.rvPartEnd = String(last.offset_bits + last.size_bits);
  }
  const entries = l.entries.slice(0, ENTRIES_SHOWN);
  const targets: { entry: number; t: DirectoryTarget }[] = [];
  entries.forEach((e, i) => {
    for (const t of e.targets) targets.push({ entry: i, t });
  });
  const h = document.createElement("h3");
  const code = document.createElement("code");
  code.textContent = l.name;
  h.append(code, " ", RV.directorySummary(l.placing, l.elements, targets.length));
  box.append(h);
  const fileBits = ctx.doc.lengthBits;
  const colour = (bit: number): string => (model === null ? "var(--accent)" : (groupAt(model, bit)?.color ?? "var(--muted)"));
  const byOffset = [...targets].sort((a, b) => a.t.offset_bits - b.t.offset_bits);
  const data: RibbonData = {
    top: {
      from: first?.offset_bits ?? 0,
      to: (last?.offset_bits ?? 0) + (last?.size_bits ?? 0),
      caption: RV.dirTop(l.name),
      boxes: entries.map((e) => ({
        from: e.offset_bits,
        to: e.offset_bits + e.size_bits,
        label: stripIndex(e.name),
        color: colour(e.targets[e.targets.length - 1]?.offset_bits ?? e.offset_bits),
      })),
    },
    bottom: {
      from: 0,
      to: fileBits,
      caption: RV.dirBottom(bitsText(fileBits)),
      boxes: byOffset.map(({ t }) => ({ from: t.offset_bits, to: t.offset_bits + Math.max(8, t.size_bits), label: stripIndex(t.name), color: colour(t.offset_bits) })),
    },
    bands: byOffset.map(({ entry, t }, i) => ({ top: entry, bottom: i, ...(t.aside ? { color: "var(--muted)" } : {}) })),
  };
  const fig = document.createElement("figure");
  fig.className = "rv-figure rv-ribbonfig";
  fig.append(
    ribbon(data, {
      width: 960,
      onHover: (i, e) => {
        const pair = i === null ? undefined : byOffset[i];
        const entry = pair === undefined ? undefined : entries[pair.entry];
        if (pair === undefined || entry === undefined) {
          hideTip();
          return;
        }
        const tip = document.createElement("div");
        const b = document.createElement("b");
        b.textContent = stripIndex(entry.name) || entry.name;
        const at = document.createElement("div");
        at.className = "rv-tip-where";
        at.textContent = RV.dirEntryAt(formatOffset(entry.offset_bits), bitsText(entry.size_bits));
        const to = document.createElement("div");
        to.textContent = RV.dirPointsTo(pair.t.name, formatOffset(pair.t.offset_bits), bitsText(pair.t.size_bits));
        const via = document.createElement("div");
        via.className = "rv-tip-where";
        via.textContent = RV.dirVia(pair.t.via);
        tip.append(b, at, to, via);
        tipAt(tip, e.clientX, e.clientY);
      },
      onPick: (i) => {
        const pair = byOffset[i];
        if (pair === undefined) return;
        ctx.host.pick({ path: pair.t.path, startBit: pair.t.offset_bits, endBit: pair.t.offset_bits + pair.t.size_bits });
      },
    }),
  );
  const cap = document.createElement("figcaption");
  const lead = document.createElement("b");
  const firsts = entries.map((e) => e.targets[0]?.offset_bits ?? 0);
  lead.textContent = sameOrder(firsts) ? RV.dirInOrder : RV.dirOutOfOrder;
  cap.append(lead, ` ${RV.dirCaption}`);
  if (l.entries.length > ENTRIES_SHOWN) cap.append(` ${RV.dirFirstEntries(ENTRIES_SHOWN, l.entries.length)}`);
  fig.append(cap);
  box.append(fig);
  return box;
}
