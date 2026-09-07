/**
 * The four trees the treemap can draw, built from what the document knows.
 *
 * Each mode is one answer to "divide the file by what": by the structure the
 * template found, by the kind of field, by the value of each byte, or by the
 * value of each bit. The widget itself knows none of this; it takes a tree of
 * values with colours already chosen. What is here is the four ways of getting
 * one, and the honesty that has to go with them.
 *
 * The honesty is the awkward part. Two of these modes read the whole file and
 * a file can be a gigabyte, so a tree is asked for while the reading is part
 * way through. Leaving out what has not been read yet would be the easy thing
 * and the wrong one: the areas would still add up to the whole box, so a map
 * of the first five per cent of a file would fill the panel and look like the
 * file. So what has not been read is a box of its own, drawn empty, and it
 * shrinks as the reading gets on.
 */

import { byteText, formatBytes, formatOffset, percentText, type Doc, type KindTotal, type KindTotals, type TemplateNode } from "./doc.js";
import { fieldClass, sectionColor, UNMAPPED_COLOR } from "./fieldstyle.js";
import { childWord, countText, GAP_LABEL, KIND_LABEL, NO_TEMPLATE_HINT, REPORT, TREEMAP } from "./strings.js";
import type { TreeNode } from "./treemap.js";

/** Which key the file's bytes are divided by. */
export type TreemapMode = "structure" | "kinds" | "bytes" | "bits";

export const TREEMAP_MODES: readonly TreemapMode[] = ["structure", "kinds", "bytes", "bits"];

/** How the caller says what a box is worth: bits everywhere except the byte
 *  histogram, whose numbers are counts of a value rather than sizes of a run.
 *  The distinction is only in the wording, since a treemap divides by share. */
export type Unit = "bits" | "count";

/** A box the caller can point at, with what it took to say so. */
export type TreemapTree = {
  readonly root: TreeNode;
  readonly unit: Unit;
  /** Null when everything the mode covers has been read. Otherwise the line to
   *  put under the map while it is still being read. */
  readonly progress: string | null;
  /** Shown instead of the map: no template, or a reading that failed. */
  readonly none: string | null;
};

/** The smallest box worth drawing on its own, in square pixels. Under about
 *  four pixels each way there is nothing to point at, nothing to label and
 *  nothing to tell apart from its neighbour, so the rest go in one box that
 *  says how many they were. */
export const POOL_UNDER = 20;

// ---- structure ----

/**
 * The template's parts and what is inside them.
 *
 * Built from the template rather than from the outline the rail already
 * holds, which was the first thing tried and was wrong: the outline lists
 * *some* of a part's sub-parts, so a frame's children summed to a fraction of
 * the frame. A treemap lays children out to fill their parent whatever they
 * add up to, so the boxes came out several times too big and nothing said so.
 * Every set of boxes here tiles its parent, and where the children are more
 * than are worth reading, the rest is a box of its own.
 */
export function structureTree(doc: Doc, at: readonly number[] | null): TreemapTree {
  if (doc.template === null) return nothing(NO_TEMPLATE_HINT);
  const root = doc.templateNode(at ?? []);
  if (root.status === "error") return nothing(root.message);
  // No parts yet is not no parts: the listing walks the template as the bytes
  // arrive, and the picture fills in behind it.
  if (root.status !== "ok") return nothing(REPORT.reading);
  return { root: nodeTree(doc, root.node, at ?? [], 0), unit: "bits", progress: null, none: null };
}

/**
 * One node and the levels under it, as deep as is worth reading.
 *
 * The widget prunes by area, so reading past the point where a box would be a
 * sliver is work nobody sees. Three levels covers what fits in the rail and
 * most of what fits in the view; past that a reader opens a box and gets three
 * more.
 */
function nodeTree(doc: Doc, n: TemplateNode, path: readonly number[], depth: number): TreeNode {
  const section = path[0] ?? 0;
  const self: TreeNode = {
    key: path.length === 0 ? "file" : String(path[path.length - 1]),
    name: path.length === 0 ? TREEMAP.root : n.name,
    value: n.size_bits,
    color: sectionColor(section),
    detail: n.type,
    range: { offsetBits: n.offset_bits, sizeBits: n.size_bits },
    path,
  };
  if (!n.composite || n.child_count === 0 || depth >= READ_DEPTH) return self;
  const want = Math.min(n.child_count, CHILDREN_MAX);
  const kids = doc.templateChildren(path, 0, want);
  if (kids.status !== "ok") return self;
  const drawn = kids.node.map((k) => nodeTree(doc, k, k.path, depth + 1));
  // What the children do not account for, so that the boxes inside a frame add
  // up to the frame: the elements past the cap, and the slack a structure
  // leaves between or after its fields.
  const rest = n.size_bits - drawn.reduce((sum, k) => sum + k.value, 0);
  if (rest > 0) {
    const over = n.child_count - kids.node.length;
    // Two different leftovers: elements there was no room to draw, which are
    // the part's own children faded, and bytes no field covers, which are
    // hatched because they are not a kind of anything.
    drawn.push(
      over > 0
        ? { key: "rest", name: TREEMAP.pooled(over), value: rest, color: sectionColor(section), colorClass: "tm-pooled", detail: TREEMAP.pooledTitle(over, childWord(n), "", "") }
        : { key: "rest", name: GAP_LABEL, value: rest, color: UNMAPPED_COLOR, colorClass: "tm-unmapped", detail: UNMAPPED_DETAIL },
    );
  }
  return { ...self, children: drawn };
}

/** How many levels are read before a reader has to open a box. */
const READ_DEPTH = 3;

/** How many children of one node are read for a picture of it. Past this the
 *  boxes are under a pixel anyway, and what is left over is one box saying how
 *  many were not drawn. */
const CHILDREN_MAX = 400;

// ---- field kinds ----

/**
 * Every field the walk has reached, grouped by kind and then by type.
 *
 * The core totals by (kind, type) and never counts a structure's bits twice:
 * a composite is in the totals only for what its own syntax accounts for over
 * and above its children, so the braces of a JSON object are there and the
 * object is not.
 *
 * Two boxes here are not kinds of field at all. `unmapped` is bytes inside the
 * walked region that no field covers, which is a real answer and often a large
 * one. `not read yet` is the rest of the file, and it is drawn so that the map
 * keeps meaning the whole file: leave it out and a walk that has reached five
 * per cent fills the panel and looks like the finished picture.
 */
export function kindsTree(totals: KindTotals, fileBits: number): TreemapTree {
  const byKind = new Map<string, KindTotal[]>();
  for (const t of totals.totals) {
    // A computed field is the template working something out in the open. It
    // occupies no bytes, so it would be a box of no width.
    if (t.bits <= 0) continue;
    const group = KIND_LABEL[t.kind] ?? t.kind;
    const have = byKind.get(group);
    if (have === undefined) byKind.set(group, [t]);
    else have.push(t);
  }
  const kids: TreeNode[] = [];
  for (const [label, entries] of byKind) {
    const bits = entries.reduce((n, t) => n + t.bits, 0);
    const kind = entries[0]?.kind ?? "bytes";
    kids.push({
      key: label,
      name: label,
      value: bits,
      color: UNMAPPED_COLOR,
      colorClass: fieldClass(kind),
      detail: countText(entries.reduce((n, t) => n + t.count, 0), "field"),
      children: entries.map((t) => ({
        key: t.type,
        name: t.type,
        value: t.bits,
        color: UNMAPPED_COLOR,
        colorClass: fieldClass(t.kind),
        detail: countText(t.count, "field"),
      })),
    });
  }
  // Three boxes could all be read as "not shown as itself", so each is drawn
  // its own way: a kind is solid, bytes nothing claims are hatched, and bytes
  // nobody has looked at yet are an empty outline.
  if (totals.unmapped_bits > 0) {
    kids.push({ key: "unmapped", name: GAP_LABEL, value: totals.unmapped_bits, color: UNMAPPED_COLOR, colorClass: "tm-unmapped", detail: UNMAPPED_DETAIL });
  }
  const left = Math.max(0, fileBits - totals.reached_bits);
  if (left > 0) kids.push({ key: "unwalked", name: TREEMAP.unwalked, value: left, color: UNMAPPED_COLOR, colorClass: "tm-unwalked", detail: TREEMAP.unwalked });
  return {
    root: { key: "file", name: TREEMAP.root, value: fileBits, color: UNMAPPED_COLOR, children: kids },
    unit: "bits",
    progress: totals.done ? null : TREEMAP.reading(shareOf(totals.reached_bits, fileBits)),
    none: null,
  };
}

const UNMAPPED_DETAIL = "no field covers these bytes";

// ---- byte values ----

/**
 * The 256 byte values, in four groups.
 *
 * Zero gets a group to itself because it is the value a reader most wants
 * separated: padding, holes and unwritten space are all zero, and folding it
 * in with the printable range would hide the one thing the picture is best at
 * showing. The other three splits are the ones the values themselves make.
 */
export function bytesTree(histogram: readonly number[], scanned: number, total: number): TreemapTree {
  const read = histogram.reduce((n, c) => n + c, 0);
  const groups = TREEMAP.byteGroups.map((g) => {
    let sum = 0;
    const values: TreeNode[] = [];
    for (let v = g.from; v <= g.to; v++) {
      const count = histogram[v] ?? 0;
      if (count === 0) continue;
      sum += count;
      values.push({
        key: String(v),
        name: byteText(v),
        value: count,
        color: BYTE_GROUP_COLOR[g.label] ?? UNMAPPED_COLOR,
        detail: `${count.toLocaleString()} bytes`,
      });
    }
    const node: TreeNode = {
      key: g.label,
      name: g.label,
      value: sum,
      color: BYTE_GROUP_COLOR[g.label] ?? UNMAPPED_COLOR,
      detail: g.title,
    };
    return values.length === 0 ? node : { ...node, children: values };
  });
  return {
    root: { key: "file", name: TREEMAP.root, value: read, color: UNMAPPED_COLOR, children: groups },
    unit: "count",
    progress: scanned >= total ? null : SCANNING(share(scanned, total)),
    none: null,
  };
}

/**
 * Byte values are names, not amounts, so a ramp across them would say 0xF0 is
 * more of something than 0x10. The groups get the colours the class map
 * already gives the same ideas: its zero and its text, and its data blue at
 * two steps for the rest.
 */
const BYTE_GROUP_COLOR: Readonly<Record<string, string>> = {
  "00": "#3b4048",
  "01-1F": "#6f8fbf",
  "20-7F": "#147a5b",
  "80-FF": "#1769aa",
};

// ---- bits ----

/**
 * Set bits against clear ones, over everything scanned.
 *
 * Two boxes are one number, and a treemap of one number is a bar. It is drawn
 * with the same widget so nothing here is a special case, and the number is
 * written out under it, where a number belongs.
 */
export function bitsTree(histogram: readonly number[], scanned: number, total: number): TreemapTree {
  let set = 0;
  let bytes = 0;
  for (let v = 0; v < 256; v++) {
    const count = histogram[v] ?? 0;
    bytes += count;
    set += count * popcount(v);
  }
  const clear = bytes * 8 - set;
  return {
    root: {
      key: "file",
      name: TREEMAP.root,
      value: set + clear,
      color: UNMAPPED_COLOR,
      children: [
        { key: "1", name: TREEMAP.bits.set, value: set, color: SET_COLOR, colorClass: "tm-bit-set", detail: TREEMAP.bitsTitle("1", `${set.toLocaleString()} bits`, percentText(set, set + clear)) },
        { key: "0", name: TREEMAP.bits.clear, value: clear, color: CLEAR_COLOR, colorClass: "tm-bit-clear", detail: TREEMAP.bitsTitle("0", `${clear.toLocaleString()} bits`, percentText(clear, set + clear)) },
      ],
    },
    unit: "count",
    progress: scanned >= total ? null : SCANNING(share(scanned, total)),
    none: null,
  };
}

/** Ink and paper, so the two boxes survive greyscale and colour blindness:
 *  the difference between them is lightness, which is the only channel two
 *  boxes of one number need. Kept here as well as in the stylesheet for a
 *  caller drawing without it. */
const SET_COLOR = "var(--fg)";
const CLEAR_COLOR = "var(--line)";

function popcount(v: number): number {
  let n = 0;
  for (let b = v; b !== 0; b >>= 1) n += b & 1;
  return n;
}

/** The line under a two-box map, since two boxes cannot be read as numbers. */
export function bitsLine(histogram: readonly number[]): string {
  let set = 0;
  let bytes = 0;
  for (let v = 0; v < 256; v++) {
    const count = histogram[v] ?? 0;
    bytes += count;
    set += count * popcount(v);
  }
  const whole = bytes * 8;
  return TREEMAP.bitsLine(percentText(set, whole), percentText(whole - set, whole));
}

// ---- shared ----

/**
 * The title on a box: what it is, how big, how much of the picture, and where.
 *
 * The share is of the whole box rather than of the file, which is the same
 * number until a reader opens a box and a different one after. What the eye
 * reads off the picture is the share of what is drawn; the address beside it
 * is what says which bytes those are.
 *
 * While a scan or a walk is part way through, the whole is what has been read
 * rather than the file, and the wording says so. A share of a number nobody
 * has yet is not a share.
 */
export function boxTitle(node: TreeNode, share: number, unit: Unit, read: number | null): string {
  const size = unit === "bits" ? formatBytes(Math.ceil(node.value / 8)) : `${node.value.toLocaleString()} bytes`;
  const pct = percentText(share, 1);
  const of = read === null ? TREEMAP.ofFile(pct) : TREEMAP.ofRead(pct, formatBytes(read), unit === "bits");
  const parts = [node.name, size, of];
  if (node.range !== undefined) parts.push(formatOffset(node.range.offsetBits));
  return node.detail === undefined ? parts.join(" · ") : `${parts.join(" · ")}\n${node.detail}`;
}

/** What the pooled box is called in a given mode: the format's own word for
 *  what the parent holds, where it has one. */
export function poolNoun(mode: TreemapMode, parent: TemplateNode | null): string {
  if (mode === "bytes") return TREEMAP.byteNoun;
  if (mode === "kinds") return "type";
  return parent === null ? "part" : childWord(parent);
}

export { GAP_LABEL, KIND_LABEL, fieldClass };

/** The whole-file byte scan's own line, so the treemap and the map above it
 *  never report the same scan two different ways. */
const SCANNING = (percent: number): string => `Scanning the file… ${percent}%`;

function share(part: number, whole: number): number {
  return whole === 0 ? 0 : Math.round((part / whole) * 100);
}

const shareOf = share;

/** A sentence instead of a picture. A treemap of nothing is a blank box, and a
 *  blank box says the file is empty rather than that nothing is known yet. */
function nothing(none: string): TreemapTree {
  return { root: { key: "file", name: TREEMAP.root, value: 0, color: UNMAPPED_COLOR }, unit: "bits", progress: null, none };
}
