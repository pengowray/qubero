// The content of a container: the files of an archive, the objects of an HDF5
// file, the tensors of a GGUF model, read through the same adapters as the
// rail's Logical tab (`logicaloutline.ts`). An archive's content is its files,
// so a ZIP gets a table of its entries with their sizes, packed sizes, method
// and ratio; any other container gets its outline's entries and what each is.
// Every name links to its bytes, and a long list stops at `ROWS` with a count.

import type { Doc } from "../doc.ts";
import { formatOffset } from "../format.ts";
import { hasLogicalOutline, logicalLength, logicalOutline, zipEntries } from "../logicaloutline.ts";
import { byteRef } from "./refs.ts";
import { WAIT, type Rendered } from "./section.ts";
import { bytesText, RV } from "./text.ts";

/** Entries listed before the rest are counted. */
const ROWS = 100;

/** The container's entries as a section, null for a file that is not one, or
 *  `WAIT` while they are read. */
export function archiveContent(doc: Doc): Rendered {
  if (!hasLogicalOutline(doc)) return null;
  if (doc.template === "zip") return zipContent(doc);
  const r = logicalOutline(doc, new Set(), new Map());
  if (r === null) return null;
  if (r.status === "pending" || r.status === "working") return WAIT;
  if (r.status !== "ok") return null;
  const o = r.node;
  const leaves = o.nodes.filter((n) => !n.group && n.depth > 0);
  if (leaves.length === 0) return null;
  const sec = section();
  const h = document.createElement("h2");
  h.textContent = RV.outlineHeading(o.title, o.summary);
  sec.append(h);
  const t = table([RV.entryName, ""], [o.sizeLabel ?? RV.entrySize, "rv-num"], [RV.entryWhat, ""]);
  for (const n of leaves.slice(0, ROWS)) {
    const tr = document.createElement("tr");
    const name = document.createElement("td");
    name.className = "rv-cell-name";
    const code = document.createElement("code");
    code.textContent = n.fullName.replace(/^\//, "");
    name.append(code);
    if (n.sourceBits !== null) name.append(" ", byteRef({ ...(n.sourcePath.length > 0 ? { path: n.sourcePath } : {}), startBit: n.sourceBits, endBit: n.sourceBits + 8 }, n.label, formatOffset(n.sourceBits)));
    tr.append(name, cell(logicalLength(n), "rv-num", o.sizeLabel ?? RV.entrySize), cell(n.value, "rv-muted", RV.entryWhat));
    t.body.append(tr);
  }
  sec.append(t.el);
  const more = Math.max(0, o.total - Math.min(leaves.length, ROWS), leaves.length - ROWS);
  if (more > 0) sec.append(note(RV.moreEntries(more)));
  return sec;
}

/** A ZIP's entries: name, unpacked size, size in the archive, method, and
 *  how much the method saved. */
function zipContent(doc: Doc): Rendered {
  const r = zipEntries(doc, ROWS, true);
  if (r.status === "pending" || r.status === "working") return WAIT;
  if (r.status !== "ok") return null;
  const { entries, partial } = r.node;
  if (entries.length === 0) return null;
  const sec = section();
  const unpacked = entries.reduce((n, e) => n + e.unpackedBytes, 0);
  const packed = entries.reduce((n, e) => n + e.compressedBytes, 0);
  const h = document.createElement("h2");
  h.textContent = partial ? RV.zipHeadingFirst(entries.length) : RV.zipHeading(entries.length, unpacked, packed);
  sec.append(h);
  const t = table([RV.entryName, ""], [RV.entrySize, "rv-num"], [RV.entryPacked, "rv-num"], [RV.entryMethod, ""], [RV.entryRatio, "rv-num"]);
  for (const e of entries) {
    const tr = document.createElement("tr");
    const name = document.createElement("td");
    name.className = "rv-cell-name";
    const code = document.createElement("code");
    code.textContent = e.name;
    name.append(code, " ", byteRef({ path: e.path, startBit: e.offsetBits, endBit: e.offsetBits + 32 }, e.name, formatOffset(e.offsetBits)));
    const ratio = e.compressedBytes > 0 ? e.unpackedBytes / e.compressedBytes : null;
    tr.append(
      name,
      cell(bytesText(e.unpackedBytes), "rv-num", RV.entrySize),
      cell(bytesText(e.compressedBytes), "rv-num", RV.entryPacked),
      cell(e.compression, "", RV.entryMethod),
      cell(ratio === null || e.stored ? "" : RV.ratio(ratio), "rv-num", RV.entryRatio),
    );
    t.body.append(tr);
  }
  sec.append(t.el);
  if (partial) sec.append(note(RV.zipMore));
  return sec;
}

function section(): HTMLElement {
  const sec = document.createElement("section");
  sec.className = "rv-section rv-content rv-archive";
  return sec;
}

function note(text: string): HTMLElement {
  const p = document.createElement("p");
  p.className = "rv-note";
  p.textContent = text;
  return p;
}

function cell(text: string, cls: string, label: string): HTMLTableCellElement {
  const td = document.createElement("td");
  if (cls !== "") td.className = cls;
  td.dataset.label = label;
  td.textContent = text;
  return td;
}

function table(...cols: readonly (readonly [string, string])[]): { el: HTMLElement; body: HTMLTableSectionElement } {
  const wrap = document.createElement("div");
  wrap.className = "rv-tablewrap";
  const t = document.createElement("table");
  t.className = "rv-table rv-stack";
  const thead = document.createElement("thead");
  const hr = document.createElement("tr");
  for (const [text, cls] of cols) {
    const th = document.createElement("th");
    th.textContent = text;
    if (cls !== "") th.className = cls;
    hr.append(th);
  }
  thead.append(hr);
  const body = document.createElement("tbody");
  t.append(thead, body);
  wrap.append(t);
  return { el: wrap, body };
}
