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
 *
 * The colours are the other shared thing. Structure and Field type both paint
 * by `fieldClass`, so a box is the same colour in both and the same colour the
 * hex view gives those bytes; Byte value paints by the classes the rail's map
 * above it already has a legend for. Four pictures of one file that agreed
 * about nothing would be four pictures nobody could hold together.
 */

import { byteText, formatBytes, formatOffset, percentText, type Doc, type KindTotal, type KindTotals, type TemplateNode } from "./doc.js";
import { byteClassColor, fieldClass, UNMAPPED_COLOR } from "./fieldstyle.js";
import { childWord, countText, GAP_LABEL, KIND_LABEL, NO_TEMPLATE_HINT, REPORT, TREEMAP } from "./strings.js";
import type { TreeNode } from "./treemap.js";

/** Which key the file's bytes are divided by. */
export type TreemapMode = "structure" | "classes" | "kinds" | "bytes" | "bits";

export const TREEMAP_MODES: readonly TreemapMode[] = ["structure", "classes", "kinds", "bytes", "bits"];

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
  /** Lay the children out in the order given rather than largest first. */
  readonly ordered?: boolean;
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
 *
 * How deep it reads is decided by how big the boxes come out, not by a fixed
 * number of levels. Three levels everywhere was both too few and too many: a
 * file whose whole content hangs off one 99% child spent all three levels
 * getting to it and showed nothing, while a flat file read two levels that
 * were never going to be drawn. Reading follows the area, so the big side of
 * a lopsided file is read until the boxes are too small to see and the small
 * side is not read at all.
 */
export function structureTree(doc: Doc, at: StructureAt, pixels: number): TreemapTree {
  if (doc.template === null) return nothing(NO_TEMPLATE_HINT);
  const path = at?.path ?? [];
  const root = doc.templateNode(path);
  if (root.status === "error") return nothing(root.message);
  // No parts yet is not no parts: the listing walks the template as the bytes
  // arrive, and the picture fills in behind it.
  if (root.status !== "ok") return nothing(REPORT.reading);
  const area = Math.max(pixels, 1);
  const span = at?.span ?? null;
  const node = span === null ? nodeTree(doc, root.node, path, 0, area) : runTree(doc, root.node, path, span, area);
  // In file order, not largest first. A structure's children are a sequence,
  // and laying them out in it is what makes this picture the same picture as
  // the map above it and the rows in the hex view: the header is at the top
  // left because it is at the front of the file, and a reader who opens a box
  // can see where in its parent it came from. The one box that is not in file
  // order is the pooled remainder, which is drawn faded and last because it
  // is not one place in the file at all.
  return { root: node, unit: "bits", progress: null, none: null, ordered: true };
}

/** Where the picture is rooted: a node of the template, and, where the reader
 *  has opened one run of a long array, which elements of it. */
export type StructureAt = { readonly path: readonly number[]; readonly span?: Span | null } | null;

/** A stretch of one node's children, by index. `to` is past the end. */
export type Span = { readonly from: number; readonly to: number; readonly offsetBits: number; readonly sizeBits: number };

/**
 * One node and the levels under it, as deep as is worth reading.
 *
 * `area` is how many pixels this node is expected to get. The widget prunes by
 * the same estimate, so reading a child that will come out under a few pixels
 * is work nobody sees, and stopping at it is what lets the levels that will be
 * seen be read instead.
 */
function nodeTree(doc: Doc, n: TemplateNode, path: readonly number[], depth: number, area: number): TreeNode {
  const inside = n.composite && n.child_count > 0;
  const self: TreeNode = {
    key: path.length === 0 ? "file" : String(path[path.length - 1]),
    name: path.length === 0 ? TREEMAP.root : nodeLabel(n),
    value: n.size_bits,
    color: UNMAPPED_COLOR,
    colorClass: fieldClass(n.kind),
    detail: n.type,
    range: { offsetBits: n.offset_bits, sizeBits: n.size_bits },
    path,
    // Worth opening whenever there is anything under it, whether or not this
    // reading drew it: opening is how a reader gets past what the pixels can
    // hold, and a box that refused to open would be the picture saying there
    // is nothing more when there is.
    openable: inside,
  };
  if (!inside || depth >= MAX_READ_DEPTH || area < READ_AREA) return self;
  // More elements than there are pixels to draw them in. Reading four hundred
  // of a quarter of a million and pooling the rest would hand back one box the
  // size of the parent, which is the parent with a second name on it. The run
  // is divided by index instead, and each division can be opened for the next
  // division down, so the way to one instruction of a program is nine presses
  // rather than a scroll nobody can make.
  const runs = runsOf(doc, n, path, area);
  if (runs !== null) return { ...self, children: runs };
  const want = Math.min(n.child_count, CHILDREN_MAX);
  const kids = doc.templateChildren(path, 0, want);
  if (kids.status !== "ok") return self;
  const total = kids.node.reduce((sum, k) => sum + Math.max(0, k.size_bits), 0);
  const whole = Math.max(total, n.size_bits);
  const drawn = kids.node.map((k) =>
    nodeTree(doc, k, k.path, depth + 1, whole === 0 ? 0 : (area * Math.max(0, k.size_bits)) / whole),
  );
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
        ? { key: "rest", name: TREEMAP.pooled(over), value: rest, color: UNMAPPED_COLOR, colorClass: "tm-pooled", detail: TREEMAP.pooledTitle(over, childWord(n), "", "") }
        : { key: "rest", name: GAP_LABEL, value: rest, color: UNMAPPED_COLOR, colorClass: "tm-unmapped", detail: UNMAPPED_DETAIL },
    );
  }
  if (drawn.length === 0) return self;
  return { ...self, children: drawn };
}

/**
 * One node's children as a handful of index ranges, or null when they are few
 * enough to draw as themselves.
 *
 * Every range's size is the bytes between where its first element starts and
 * where the next range's first element starts, so the ranges tile the parent
 * exactly and no arithmetic over a quarter of a million elements is needed:
 * one read per boundary, a dozen or so reads in all.
 *
 * That only holds while the elements are laid out in order and end to end,
 * which an array is and a table of offsets into the file is not. The sampled
 * boundaries are checked for it, and a set of children that fails the check is
 * left to the ordinary path, where the ones that do not fit are pooled and
 * said to be pooled.
 */
function runsOf(doc: Doc, n: TemplateNode, path: readonly number[], area: number): TreeNode[] | null {
  const count = n.child_count;
  if (count <= RUNS_MIN || area / count >= POOL_UNDER) return null;
  const runs = Math.max(2, Math.min(RUNS_MAX, Math.floor(area / RUN_AREA)));
  if (runs >= count) return null;
  const edge = (i: number): number | null => {
    if (i >= count) return n.offset_bits + n.size_bits;
    const k = doc.templateChildren(path, i, i + 1);
    return k.status === "ok" ? (k.node[0]?.offset_bits ?? null) : null;
  };
  const bounds: number[] = [];
  for (let r = 0; r <= runs; r++) {
    const at = edge(Math.floor((r * count) / runs));
    // A boundary that could not be read, or one that steps backwards, means
    // these children are not a run laid out in order.
    if (at === null) return null;
    const last = bounds[bounds.length - 1];
    if (last !== undefined && at < last) return null;
    bounds.push(at);
  }
  const first = bounds[0] ?? n.offset_bits;
  const out: TreeNode[] = [];
  // Whatever sits between the front of the parent and its first element. A
  // table's header, usually, and always someone's bytes.
  if (first > n.offset_bits) {
    out.push({ key: "head", name: GAP_LABEL, value: first - n.offset_bits, color: UNMAPPED_COLOR, colorClass: "tm-unmapped", detail: UNMAPPED_DETAIL });
  }
  for (let r = 0; r < runs; r++) {
    const from = Math.floor((r * count) / runs);
    const to = Math.floor(((r + 1) * count) / runs);
    const start = bounds[r] ?? 0;
    const end = bounds[r + 1] ?? start;
    if (to <= from || end <= start) continue;
    out.push({
      key: `${from}-${to}`,
      name: TREEMAP.runName(from, to - 1),
      value: end - start,
      color: UNMAPPED_COLOR,
      colorClass: fieldClass(n.kind),
      detail: TREEMAP.runDetail(to - from, childWord(n)),
      range: { offsetBits: start, sizeBits: end - start },
      path,
      span: { from, to, offsetBits: start, sizeBits: end - start },
      openable: true,
    });
  }
  return out.length === 0 ? null : out;
}

/**
 * One opened index range, with its own elements inside it.
 *
 * The same node the range came out of, cut down to the elements the reader
 * pressed on. Opening again divides that range in turn, so the way from an
 * array of a quarter of a million elements to any one of them is a handful of
 * presses and every step of it is a picture of what is there.
 */
function runTree(doc: Doc, n: TemplateNode, path: readonly number[], span: Span, area: number): TreeNode {
  const self: TreeNode = {
    key: `${span.from}-${span.to}`,
    name: TREEMAP.runName(span.from, span.to - 1),
    value: span.sizeBits,
    color: UNMAPPED_COLOR,
    colorClass: fieldClass(n.kind),
    detail: TREEMAP.runDetail(span.to - span.from, childWord(n)),
    range: { offsetBits: span.offsetBits, sizeBits: span.sizeBits },
    path,
    openable: true,
  };
  const count = span.to - span.from;
  if (count <= 0) return self;
  // Still too many to draw one each, so it divides again.
  if (count > RUNS_MIN && area / count < POOL_UNDER) {
    const runs = Math.max(2, Math.min(RUNS_MAX, Math.floor(area / RUN_AREA)));
    const edge = (i: number): number | null => {
      if (i >= span.to) return span.offsetBits + span.sizeBits;
      const k = doc.templateChildren(path, i, i + 1);
      return k.status === "ok" ? (k.node[0]?.offset_bits ?? null) : null;
    };
    const kids: TreeNode[] = [];
    let ok = true;
    let at = edge(span.from) ?? span.offsetBits;
    for (let r = 0; r < runs && ok; r++) {
      const from = span.from + Math.floor((r * count) / runs);
      const to = span.from + Math.floor(((r + 1) * count) / runs);
      const end = edge(to);
      if (end === null || end < at || to <= from) {
        ok = false;
        break;
      }
      if (end > at) {
        kids.push({
          key: `${from}-${to}`,
          name: TREEMAP.runName(from, to - 1),
          value: end - at,
          color: UNMAPPED_COLOR,
          colorClass: fieldClass(n.kind),
          detail: TREEMAP.runDetail(to - from, childWord(n)),
          range: { offsetBits: at, sizeBits: end - at },
          path,
          span: { from, to, offsetBits: at, sizeBits: end - at },
          openable: true,
        });
      }
      at = end;
    }
    if (ok && kids.length > 0) return { ...self, children: kids };
  }
  const kids = doc.templateChildren(path, span.from, Math.min(span.to, span.from + CHILDREN_MAX));
  if (kids.status !== "ok") return self;
  const drawn = kids.node.map((k) => nodeTree(doc, k, k.path, 1, count === 0 ? 0 : area / count));
  const rest = span.sizeBits - drawn.reduce((sum, k) => sum + k.value, 0);
  if (rest > 0) {
    const over = count - kids.node.length;
    drawn.push(
      over > 0
        ? { key: "rest", name: TREEMAP.pooled(over), value: rest, color: UNMAPPED_COLOR, colorClass: "tm-pooled", detail: TREEMAP.pooledTitle(over, childWord(n), "", "") }
        : { key: "rest", name: GAP_LABEL, value: rest, color: UNMAPPED_COLOR, colorClass: "tm-unmapped", detail: UNMAPPED_DETAIL },
    );
  }
  return drawn.length === 0 ? self : { ...self, children: drawn };
}

/** Below this many children there is nothing to gain by dividing them into
 *  ranges: a dozen boxes named after their contents beat a dozen named after
 *  their indexes. */
const RUNS_MIN = 24;
/** At most this many ranges at one level. More than this and the names stop
 *  fitting, and a reader counting past two dozen boxes is not reading a
 *  picture. */
const RUNS_MAX = 24;
/** Pixels one range should get, so a reader can point at it and read what it
 *  says. */
const RUN_AREA = 900;

/**
 * What a box is called.
 *
 * A template that names its parts is left alone. What needs help is the part
 * whose name is its index: an ELF's sections are `[0]` to `[9]`, and ten boxes
 * numbered nought to nine tell a reader nothing they could not have guessed.
 * The type says what the bytes are, so it goes on the end, without the empty
 * brackets that only mean "there are several of these".
 */
function nodeLabel(n: TemplateNode): string {
  if (!/^\[\d+\]$/.test(n.name) || n.type === "") return n.name;
  const type = n.type.replace(/\[\]$/, "");
  return type === "" ? n.name : `${n.name} ${type}`;
}

/** How many levels are read before a reader has to open a box. Reading stops
 *  at the area long before this on any real file; the cap is for a template
 *  that nests without getting smaller. */
const MAX_READ_DEPTH = 8;

/** How many pixels a node has to be expected to get before its children are
 *  read. A little under the widget's own nesting threshold, so reading never
 *  stops one level short of what would have been drawn. */
const READ_AREA = 600;

/** How many children of one node are read for a picture of it. Past this the
 *  boxes are under a pixel anyway, and what is left over is one box saying how
 *  many were not drawn. */
const CHILDREN_MAX = 400;

// ---- byte classes ----

/**
 * What the map above this one is coloured by, as amounts instead of places.
 *
 * The rail's map says where the zeros and the compressed middle are; it cannot
 * say how much of the file they are, because a cell is a cell whether it holds
 * a per cent or a thousandth. This is the other half of that map, in the same
 * five colours with the same legend over it, and it is the answer for a file
 * no template covers: a tail of padding and a compressed body show up whether
 * or not anything describes them.
 *
 * Inside each class are its runs, in file order and biggest first, so the
 * question after "a third of this file is high entropy" has an answer in the
 * picture: which third, and in how many pieces.
 */
export function classesTree(classes: string, bucketBytes: number, fileBytes: number, scanned: number): TreemapTree {
  const runs = new Map<number, { from: number; len: number }[]>();
  for (let i = 0; i < classes.length; i++) {
    const cls = Number(classes[i]);
    const list = runs.get(cls) ?? [];
    const last = list[list.length - 1];
    if (last !== undefined && last.from + last.len === i) last.len += 1;
    else list.push({ from: i, len: 1 });
    runs.set(cls, list);
  }
  const kids: TreeNode[] = [];
  for (const [cls, list] of [...runs].sort((a, b) => a[0] - b[0])) {
    const cells = list.reduce((n, r) => n + r.len, 0);
    const label = TREEMAP.classLabel[cls] ?? TREEMAP.classLabel[3] ?? "Data";
    const color = byteClassColor(cls);
    const bytesOf = (from: number, len: number): number => Math.min(fileBytes - from * bucketBytes, len * bucketBytes);
    kids.push({
      key: String(cls),
      name: label,
      value: cells * bucketBytes,
      color,
      detail: TREEMAP.classDetail(list.length),
      children: list.map((r) => ({
        key: String(r.from),
        name: formatOffset(r.from * bucketBytes * 8),
        value: bytesOf(r.from, r.len),
        color,
        detail: label,
        range: { offsetBits: r.from * bucketBytes * 8, sizeBits: bytesOf(r.from, r.len) * 8 },
      })),
    });
  }
  const left = Math.max(0, fileBytes - classes.length * bucketBytes);
  if (left > 0 && scanned < fileBytes) {
    kids.push({ key: "unscanned", name: TREEMAP.unscanned, value: left, color: UNMAPPED_COLOR, colorClass: "tm-unwalked", detail: TREEMAP.unscanned });
  }
  return {
    root: { key: "file", name: TREEMAP.root, value: Math.max(fileBytes, 1), color: UNMAPPED_COLOR, children: kids },
    unit: "count",
    progress: scanned >= fileBytes ? null : SCANNING(share(scanned, fileBytes)),
    none: null,
  };
}

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
    const fields = entries.reduce((n, t) => n + t.count, 0);
    kids.push({
      key: label,
      name: label,
      value: bits,
      color: UNMAPPED_COLOR,
      colorClass: fieldClass(kind),
      detail: countText(fields, "field"),
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
 * The 256 byte values, in order, 0x00 at the top left.
 *
 * In order, because a byte value is a position on a scale a reader already
 * knows, and a picture sorted by count would be sorted by the one thing the
 * areas already say. The layout that keeps the order is the binary one; the
 * squarified tiling used everywhere else would scatter the sequence.
 *
 * The four groups are drawn only when they are worth drawing. Each has a share
 * of the 256 values it would get if every value were equally common, and while
 * every group is near that share the grouping says nothing at all: the file is
 * uniform, which is what compressed and encrypted data looks like, and four
 * labelled frames would dress that up as structure. When one group is well off
 * its expected share the split is the finding, and then it is drawn.
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
        detail: TREEMAP.byteDetail(count, g.title),
      });
    }
    return { g, sum, values };
  });
  const grouped = worthGrouping(groups.map((x) => ({ share: read === 0 ? 0 : x.sum / read, expected: (x.g.to - x.g.from + 1) / 256 })));
  const kids: TreeNode[] = [];
  for (const { g, sum, values } of groups) {
    const color = BYTE_GROUP_COLOR[g.label] ?? UNMAPPED_COLOR;
    // A group of one value is that value. `00` is the whole of its group, and
    // a frame round it would put the group's name over the value's own and
    // spend a border saying nothing.
    if (!grouped || values.length === 1) {
      kids.push(...values);
      continue;
    }
    const node: TreeNode = { key: g.label, name: g.label, value: sum, color, detail: g.title };
    kids.push(values.length === 0 ? node : { ...node, children: values });
  }
  return {
    root: { key: "file", name: TREEMAP.root, value: read, color: UNMAPPED_COLOR, children: kids },
    unit: "count",
    progress: scanned >= total ? null : SCANNING(share(scanned, total)),
    none: null,
    ordered: true,
  };
}

/**
 * Whether the four byte groups are telling the reader anything.
 *
 * A group's expected share is how much of the file it would be if every value
 * turned up as often as every other: 0x80-0xFF is half the values, so half the
 * file. A file where all four land near their expected share has no interesting
 * split in it, and the honest picture of it is 256 boxes of much the same size.
 */
function worthGrouping(groups: readonly { share: number; expected: number }[]): boolean {
  return groups.some((g) => Math.abs(g.share - g.expected) >= GROUP_DEVIATION);
}

/** How far off its expected share one group has to be before the four groups
 *  are drawn. Eight points of the file is a difference a reader can see in the
 *  picture without being told; less than that is the noise every real file has
 *  and would put four frames round nothing. */
const GROUP_DEVIATION = 0.08;

/**
 * Byte values are names, not amounts, so a ramp across them would say 0xF0 is
 * more of something than 0x10. The groups take the colours the rail's own map
 * gives the same ideas, so the legend above the treemap reads for the treemap:
 * zeros, text, and the two shades of data.
 */
const BYTE_GROUP_COLOR: Readonly<Record<string, string>> = {
  "00": "#4a4f58",
  "01-1F": "#6f93e8",
  "20-7F": "#4f9e63",
  "80-FF": "#3f6fbf",
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
  const head = node.detail === undefined ? parts.join(" · ") : `${parts.join(" · ")}\n${node.detail}`;
  return node.openable === true ? `${head}\n${TREEMAP.openHint}` : head;
}

/** What the pooled box is called in a given mode: the format's own word for
 *  what the parent holds, where it has one. */
export function poolNoun(mode: TreemapMode, parent: TemplateNode | null): string {
  if (mode === "bytes") return TREEMAP.byteNoun;
  if (mode === "kinds") return "type";
  if (mode === "classes") return "run";
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
