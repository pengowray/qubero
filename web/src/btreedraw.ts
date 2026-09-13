/**
 * What one HDF5 B-tree looks like, worked out from the walked tree alone.
 *
 * Split out of `btreepanel.ts`, which had grown to hold two tree grammars, the
 * panel's state, its events and its drawing all at once. This half is the
 * picture only: a tree goes in, and geometry, words or elements come out.
 * Nothing here knows which tree is on screen, which box the reader pressed, or
 * where in the document any of it lands.
 *
 * There are two tiers inside this file, and the seam between them earns its
 * keep more than the one above it:
 *
 * - Geometry and words, which touch no DOM at all: `place`, `boxesOf`,
 *   `leafCount`, `rowLines`, `readoutLines` and the helpers under them. These
 *   are the rules the picture is a drawing of, and they are the half with
 *   tests. What a box weighs, set against the count `leafCount` prints in the
 *   band under it, is arithmetic about a tree and nothing else; while it lived
 *   in the panel it could only be checked by looking at the screen, and it was
 *   wrong for a version 2 tree for as long as it lived there.
 * - Elements: `drawTree`, `drawStrip` and `keyRow`, which build SVG out of
 *   what the first tier settled. They put nothing in the document and read
 *   nothing out of it; the panel does that.
 *
 * `formatBytes` and `formatOffset` are imported from `format.ts` rather than
 * from `doc.ts`, which re-exports them and is where the rest of the panel gets
 * them. The same two functions either way, but `doc.ts` loads the wasm core at
 * the top of the module, so taking them from there would make this file
 * unloadable outside a browser and would take its tests down with it.
 */

import type { Tree, TreeNode } from "./doc.ts";
import { formatBytes, formatOffset } from "./format.ts";
import { BTREES } from "./strings.ts";

/** How tall one band of the tree is, in the rail and over the window. */
export const ROW_H = 16;
export const ROW_H_BIG = 30;
/** How tall one band of the address strip is. Shorter than a tree band: it
 *  carries marks rather than labelled boxes. */
export const STRIP_H = 7;
export const STRIP_H_BIG = 11;

/** The gap between bands. Small enough that a child still reads as sitting
 *  under its parent, wide enough that the two rows are two rows. */
const ROW_GAP = 3;

/** A box narrower than this cannot be pressed, so neighbouring siblings are
 *  drawn as one until they reach it. */
const MIN_W = 3;

const SVG = "http://www.w3.org/2000/svg";

/** One drawn box: the nodes it stands for and where it sits. A box stands for
 *  more than one node only when they are siblings too narrow to draw apart. */
export type Box = {
  readonly key: string;
  readonly row: number;
  readonly nodes: readonly number[];
  readonly x: number;
  readonly w: number;
  /** What the width stands for: the weights of the nodes drawn as this box,
   *  added up. The pixels are what the reader sees and this is the number
   *  behind them. */
  readonly weight: number;
  /** The same, from `Placed.count`, which is the honest number and the one the
   *  readout prints. Zero where there is none. */
  readonly count: number;
  /** True where any node in the box has children the walk never reached, so
   *  `count` is a lower bound. */
  readonly floor: boolean;
};

/** Where each node sits in the tree picture, before neighbours are merged.
 *  `weight` is the count the width stands for, kept beside the pixels because
 *  it is the number a test can check and a pixel is not. */
export type Placed = {
  x: number;
  w: number;
  weight: number;
  /** The same quantity with no floor under it and no fallback in it: what the
   *  tree really indexes in and below this node. `weight` is what the drawing
   *  needs, so it is at least 1 and an index node whose children were never
   *  walked falls back to its own entry count; both of those are facts about
   *  the picture rather than about the file, and neither may be printed as a
   *  number of links, chunks or records. Zero where there is no honest one. */
  count: number;
  /** Whether this node or anything under it had children the walk never
   *  reached, so `count` is a lower bound. Worked out here rather than read
   *  off `TreeNode.truncated`, which the core carries upward only in `ranges`
   *  and so never in a version 2 group tree. */
  floor: boolean;
};

/** One line of the summary under the picture. `short` is the muted class the
 *  warnings and caveats take; `title` is the one row that explains itself on
 *  hover. */
export type RowLine = { readonly text: string; readonly short: boolean; readonly title?: string; readonly muted?: boolean };

/**
 * How many of the things this tree indexes a node holds itself, as against
 * holding a pointer at a node that holds them.
 *
 * This is the whole of the difference between the two versions, and getting it
 * wrong is how a root box came to disagree with the band under it.
 *
 * A version 1 index node's entries are child pointers, and every child is
 * drawn as a box on the next row down, so counting them here would count that
 * row twice. Such a node holds nothing of its own and weighs what its children
 * weigh. The bottom row is the exception and falls out of having no children:
 * a link table's entries are links and a bottom-row chunk index node's are
 * chunks, and neither links nor chunks are nodes of the tree.
 *
 * A version 2 tree is a B-tree and not a B+ tree, so a record sits between
 * every two children and records live at every level. Every node's entries are
 * records, none of which any child repeats. Summing the leaves alone left the
 * root of a 2,000 record tree weighing 1,952, under a band printing the
 * header's own 2,000: the 48 records held above the leaves are in the tree,
 * and a width that leaves them out is a width that disagrees with the number
 * written directly under it.
 *
 * `childless` rather than the node's kind or level, because that is the test
 * the weighting can actually act on: a version 1 index node whose children the
 * walk never reached has none to take a weight from, and its own count is the
 * only floor there is. `omitted` says that happened.
 */
function held(tree: Tree, node: TreeNode, childless: boolean): number {
  if (tree.version === 2) return node.entries;
  return childless ? node.entries : 0;
}

/**
 * The same, for the number that is printed rather than the number that is
 * drawn.
 *
 * Two differences from `held`, and both are the difference between a width and
 * a count. A version 1 node's entries are links or chunks only on the bottom
 * row; above it they are pointers at nodes, and `held`'s fallback counts them
 * anyway because a box with no width is worse than a box whose width is a
 * guess. The test for the bottom row is `leafCount`'s, so the root's `count`
 * and the band's number are the same quantity from the same rule.
 */
function indexed(tree: Tree, node: TreeNode): number {
  if (tree.version === 2) return node.entries;
  return node.kind === "links" || (tree.job === "chunk" && node.level === 0) ? node.entries : 0;
}

/**
 * Where every node sits across the width, by how much of what the tree indexes
 * is at or below it.
 *
 * A node's weight is what it holds itself, by `held` above, plus the weights of
 * its children. So the root weighs the whole tree, which is the number the band
 * under the last row prints, and a box spans exactly its children, which is
 * what makes the picture a tree without a line drawn between any two boxes.
 *
 * A node with no weight at all still gets a sliver: an empty node is a fact
 * about the file, and a box of no width says nothing.
 */
export function place(tree: Tree, width: number): Placed[] {
  const kids: number[][] = tree.nodes.map(() => []);
  for (let i = 1; i < tree.nodes.length; i++) {
    const parent = tree.nodes[i]?.parent ?? -1;
    if (parent >= 0) kids[parent]?.push(i);
  }
  const weight = new Array<number>(tree.nodes.length).fill(0);
  const count = new Array<number>(tree.nodes.length).fill(0);
  const floor = new Array<boolean>(tree.nodes.length).fill(false);
  // Backwards over a breadth-first list: every child is settled before its
  // parent is asked about.
  for (let i = tree.nodes.length - 1; i >= 0; i--) {
    const own = kids[i] ?? [];
    const node = tree.nodes[i];
    const below = own.reduce((sum, k) => sum + (weight[k] ?? 0), 0);
    weight[i] = Math.max(1, (node === undefined ? 0 : held(tree, node, own.length === 0)) + below);
    count[i] = (node === undefined ? 0 : indexed(tree, node)) + own.reduce((sum, k) => sum + (count[k] ?? 0), 0);
    floor[i] = (node?.truncated ?? false) || own.some((k) => floor[k] === true);
  }
  const placed: Placed[] = tree.nodes.map((_, i) => ({ x: 0, w: 0, weight: 0, count: count[i] ?? 0, floor: floor[i] ?? false }));
  const root = placed[0];
  if (root !== undefined) {
    root.x = 0;
    root.w = width;
    root.weight = weight[0] ?? 1;
  }
  for (let i = 0; i < tree.nodes.length; i++) {
    const here = placed[i];
    const own = kids[i] ?? [];
    if (here === undefined || own.length === 0) continue;
    // Divided by what the children weigh, not by what the parent weighs, so
    // the children fill the parent's span exactly. The two differ under a
    // version 2 internal node, by the records it holds itself; giving those a
    // sliver of their own would open a gap between two children, and in a
    // picture that draws no lines between boxes a gap reads as a child the
    // walk did not reach. What it costs is that the pixels-per-record scale
    // inside such a subtree is that subtree's own, a couple of percent off the
    // root's. What the caption promises, that the root's width is the count the
    // band prints, is exact either way.
    const total = own.reduce((sum, k) => sum + (weight[k] ?? 0), 0) || 1;
    let at = here.x;
    for (const k of own) {
      const child = placed[k];
      if (child === undefined) continue;
      child.x = at;
      child.w = (here.w * (weight[k] ?? 0)) / total;
      child.weight = weight[k] ?? 0;
      at += child.w;
    }
    // Rounding a thousand widths down leaves the last child short of its
    // parent's right edge, which reads as a parent wider than its children.
    const last = placed[own[own.length - 1] ?? -1];
    if (last !== undefined) last.w = here.x + here.w - last.x;
  }
  return placed;
}

/**
 * The boxes actually drawn: one a node, except where neighbouring siblings are
 * too narrow to press and are drawn as one.
 *
 * Only siblings are merged, and only ones next to each other, so a merged box
 * still sits inside its parent's span and the picture stays a true tree. What
 * a merged box says is how many nodes it stands for, which the readout and the
 * tooltip both carry.
 */
export function boxesOf(tree: Tree, placed: readonly Placed[], width: number): Box[] {
  const out: Box[] = [];
  const depth = Math.max(...tree.nodes.map((n) => n.depth)) + 1;
  for (let d = 0; d < depth; d++) {
    let pool: number[] = [];
    let from = 0;
    let parent = -2;
    const flush = (end: number): void => {
      if (pool.length === 0) return;
      const weight = pool.reduce((sum, i) => sum + (placed[i]?.weight ?? 0), 0);
      const count = pool.reduce((sum, i) => sum + (placed[i]?.count ?? 0), 0);
      const floor = pool.some((i) => placed[i]?.floor === true);
      out.push({ key: keyOf(pool), row: d, nodes: pool, x: from, w: Math.max(MIN_W, end - from), weight, count, floor });
      pool = [];
    };
    for (let i = 0; i < tree.nodes.length; i++) {
      const node = tree.nodes[i];
      const at = placed[i];
      if (node === undefined || at === undefined || node.depth !== d) continue;
      if (pool.length > 0 && node.parent !== parent) flush(at.x);
      if (pool.length === 0) {
        from = at.x;
        parent = node.parent;
      }
      pool.push(i);
      if (at.x + at.w - from >= MIN_W) flush(at.x + at.w);
    }
    flush(width);
  }
  return out;
}

export function keyOf(nodes: readonly number[]): string {
  return nodes.join(",");
}

/**
 * One entry of one node, drawn as a slice of that node's box.
 *
 * Until these were drawn, a node holding nine records showed the nine nowhere:
 * the number was printed on the box, the band underneath counted the same nine
 * things, and neither could be pointed at. An entry is a run of bytes at a
 * fixed stride inside the node, so a slice is a place in the file and goes to
 * its own bytes like any other mark here.
 *
 * Only a node with nothing drawn below it is divided. An index node's entries
 * are pointers at the children on the next row, which are drawn at their own
 * weights, so equal slices above unequal boxes would draw a correspondence the
 * file does not have. A childless node's entries point at things that are not
 * boxes in this picture at all: the links in a table, the chunks a bottom row
 * indexes, the records of a version 2 leaf.
 */
export type Entry = {
  readonly key: string;
  /** Index into `Tree.nodes` of the node this is an entry of, and which entry
   *  of it, counted from zero. */
  readonly node: number;
  readonly index: number;
  readonly row: number;
  readonly x: number;
  readonly w: number;
  readonly startBit: number;
  readonly endBit: number;
};

/** Keys for the two kinds of mark that share one `data-key` attribute. A box
 *  is keyed by the node numbers it stands for, joined with commas; the letter
 *  in front of an entry's key is what keeps `e0:3` out of the list `0,3`,
 *  which is a box standing for two pooled siblings. */
export function entryKey(node: number, index: number): string {
  return `e${node}:${index}`;
}

export function readEntryKey(key: string): { node: number; index: number } | null {
  const m = /^e(\d+):(\d+)$/.exec(key);
  if (m === null) return null;
  return { node: Number(m[1]), index: Number(m[2]) };
}

/**
 * The slices to draw, from the boxes already placed.
 *
 * Four things stop a node being divided, and each of them is a case where the
 * slices would say something untrue or useless:
 *
 * - Children drawn below it, for the reason in [`Entry`].
 * - A stride the walk could not settle, which it reports as zero. A picture
 *   that divided such a node would be dividing it by a guess.
 * - Entries that would not reach `MIN_W`, the same floor a box has to clear to
 *   be pressable. The count printed on the box is what carries the number
 *   there.
 * - A last entry that would end past the node's own bytes. That cannot happen
 *   from a correct walk; if it does, the arithmetic is wrong somewhere and the
 *   picture must not be the thing that asserts it.
 *
 * One entry is not drawn either: a slice the width of the box is the box, and
 * a reader who pressed it would land on bytes a shade different from the ones
 * the box goes to for no visible reason.
 */
export function entriesOf(tree: Tree, boxes: readonly Box[]): Entry[] {
  const parents = new Array<boolean>(tree.nodes.length).fill(false);
  for (const node of tree.nodes) {
    if (node.parent >= 0) parents[node.parent] = true;
  }
  const out: Entry[] = [];
  for (const box of boxes) {
    if (box.nodes.length !== 1) continue;
    const i = box.nodes[0] ?? -1;
    const node = tree.nodes[i];
    if (node === undefined || parents[i] === true) continue;
    const n = node.entries;
    // `> 0` rather than `not <= 0`, so a core built before it sent the stride,
    // whose nodes have no such field at all, divides nothing instead of
    // drawing slices at NaN.
    if (n < 2 || !(node.entry_bits > 0) || !(node.first_entry_bits >= 0)) continue;
    if (box.w < n * MIN_W) continue;
    if (node.first_entry_bits + n * node.entry_bits > node.size_bits) continue;
    for (let k = 0; k < n; k++) {
      const startBit = node.address * 8 + node.first_entry_bits + k * node.entry_bits;
      out.push({
        key: entryKey(i, k),
        node: i,
        index: k,
        row: box.row,
        x: box.x + (box.w * k) / n,
        w: box.w / n,
        startBit,
        endBit: startBit + node.entry_bits,
      });
    }
  }
  return out;
}

/** What one entry of a node is called. A version 2 node of either kind holds
 *  records, which is the word every line on such a screen uses; a version 1
 *  node's entry is a symbol table entry or a key and a child pointer, and
 *  "entry" is what the count on the box and the key beside it already call
 *  those. */
export function entryNoun(tree: Tree): string {
  return tree.version === 2 ? "record" : "entry";
}

/**
 * One slice written out: which entry of how many, where it is and how long,
 * and then what it holds where that is known.
 *
 * The index is the Listing's own, counted from zero and in brackets, because
 * pressing a slice puts the cursor in those bytes and the row it lands on is
 * labelled that way. "Entry 3" over a row reading `[2]` would be a number with
 * the wrong origin.
 *
 * What a slice holds is thin on purpose for now: the walk sends the stride and
 * nothing per entry, so a link's name, a chunk's offset and a child's address
 * are not here to print. The two lines that can be said without them are said:
 * what a version 2 group's record holds instead of a name, and that a record of
 * an undecoded type was not read.
 */
export function entryLines(tree: Tree, entry: Entry): string[] {
  const node = tree.nodes[entry.node];
  if (node === undefined) return [];
  const bytes = formatBytes(Math.ceil((entry.endBit - entry.startBit) / 8));
  const lines = [BTREES.entryAt(entryNoun(tree), entry.index, node.entries, formatOffset(entry.startBit), bytes)];
  if (tree.version === 2 && tree.records !== "read") lines.push(BTREES.recordNotDecoded);
  else if (tree.version === 2 && tree.job === "group") lines.push(BTREES.recordLinkHash);
  return lines;
}

/** The same slice for the readout under the picture, which unlike the tooltip
 *  does not sit beside the box it came out of and has to say which node that
 *  was. */
export function entryReadout(tree: Tree, entry: Entry): string[] {
  const node = tree.nodes[entry.node];
  if (node === undefined) return [];
  return [...entryLines(tree, entry), BTREES.entryIn(kindWord(node), signWord(node), formatOffset(node.address * 8))];
}

/**
 * How many things the band under the last row of boxes stands for, or null
 * where there is no honest number to print.
 *
 * A number of its own, apart from the words `leafBand` wraps around it, because
 * it is the one number in this file that has to match `place`: the root box's
 * weight and the band's count are the same quantity counted two ways, once
 * down the tree and once out of the file, and a test can only hold them
 * against each other if both are numbers.
 *
 * A version 2 tree's count is the header's own `record_count`, so it stands
 * when the walk stopped above the leaves and never says "or more". It could
 * not be a sum of the bottom row anyway: internal nodes hold records too, and
 * the leaves alone come to less than the whole.
 *
 * Null where the last row drawn is index nodes that should have had children:
 * the walk ran out before it reached the bottom, so what is below that row was
 * never counted and a band would be a number from nowhere. `omitted` says that
 * happened.
 */
export function leafCount(tree: Tree): number | null {
  if (tree.version === 2) return tree.records_total;
  const depth = Math.max(...tree.nodes.map((n) => n.depth));
  const row = tree.nodes.filter((n) => n.depth === depth);
  const first = row[0];
  if (first === undefined) return null;
  if (first.kind !== "links" && !(tree.job === "chunk" && first.level === 0)) return null;
  return row.reduce((sum, n) => sum + n.entries, 0);
}

/**
 * The band under the last row of boxes, in words, or null where `leafCount`
 * found no number.
 *
 * For a version 1 tree the count is the same sum `rowLines` puts in words on
 * the last row, so the picture and the lines under it never disagree, and both
 * are a floor when the walk stopped early.
 */
export function leafBand(tree: Tree): { label: string; title: string } | null {
  const n = leafCount(tree);
  if (n === null) return null;
  const capped = tree.omitted > 0;
  if (tree.version === 2) {
    if (tree.records === "read" && tree.record_type === 5) {
      return { label: BTREES.leafLinksV2(n), title: BTREES.leafLinksV2Title(n, capped) };
    }
    if (tree.records === "read" && tree.record_type === 10) {
      return { label: BTREES.leafChunksV2(n), title: BTREES.leafChunksV2Title(n, capped) };
    }
    // A type whose records were not read. The count is still the header's, so
    // the band is drawn; what a record refers to is not claimed.
    const name = tree.records === "unknown" ? null : tree.record_type_name;
    return { label: BTREES.leafRecords(n), title: BTREES.leafRecordsTitle(n, name, tree.record_type, capped) };
  }
  const depth = Math.max(...tree.nodes.map((node) => node.depth));
  const row = tree.nodes.filter((node) => node.depth === depth);
  if (row[0]?.kind === "links") {
    return { label: BTREES.leafLinks(n, capped), title: BTREES.leafLinksTitle(row.length, n, capped) };
  }
  return { label: BTREES.leafChunks(n, capped), title: BTREES.leafChunksTitle(row.length, n, capped) };
}

/** What kind of node this is, in words, and the signature written at it.
 *
 *  Three words for three structures. "Internal node" rather than "index node"
 *  for a `BTIN`, because that is the specification's own word for one and the
 *  summary row uses it; "index node" was coined here for a `TREE`, whose
 *  contents needed explaining. */
export function kindWord(node: TreeNode): string {
  if (node.kind === "links") return BTREES.kindLinks;
  if (node.kind === "leaf") return BTREES.kindLeaf;
  return node.sign === "BTIN" ? BTREES.kindInternal : BTREES.kindIndex;
}

/** The four bytes written at the node's address.
 *
 *  The core sends them, because it is the core that checked for them: it reads
 *  a version 2 node by its signature and refuses the node when the signature
 *  is not the one the level called for. Worked out here instead, from the kind
 *  and the version, this would be a claim about bytes nothing in this file has
 *  seen. The two constants are the fallback for a walk that sent none. */
export function signWord(node: TreeNode): string {
  if (node.sign !== "") return node.sign;
  return node.kind === "links" ? BTREES.signLinks : BTREES.signIndex;
}

/**
 * What this node's own count counts.
 *
 * Four different nouns on purpose, because the file's one number means four
 * different things: a link table's entries are names, and an index node's are
 * pointers at whatever the next row down holds, which is index nodes above the
 * bottom row, link tables under a group tree's bottom row and chunks under a
 * chunk tree's. A single word for all four would be a label whose value is not
 * the thing the label names.
 */
function holdWord(tree: Tree, node: TreeNode): string {
  if (node.kind === "links") return "link";
  if (node.level > 0) return BTREES.kindIndex;
  return tree.job === "chunk" ? "chunk" : BTREES.kindLinks;
}

/**
 * One node written out: what it is and where, what it points at or holds, and
 * the keys at its ends.
 *
 * One function for the tooltip and the readout, so the two never say the same
 * node two different ways. A node whose children were not all read says so
 * instead of a range, because that is the same fact: the core clears a range
 * it could only settle at one end.
 */
function nodeLines(tree: Tree, node: TreeNode, box: Box): string[] {
  const lines = [BTREES.selectedAt(kindWord(node), signWord(node), formatOffset(node.address * 8))];
  if (tree.version === 2) {
    // Both kinds of version 2 node hold records, so the count is a count of
    // records and the verb is "holds" for both. An internal node points at one
    // more child than that, which is what a B-tree is, and the second line is
    // where a reader who noticed the mismatch finds out why.
    lines.push(BTREES.holds(node.entries, "record"));
    if (node.kind === "index") lines.push(BTREES.pointsAtChildren(node.entries + 1));
  } else {
    const noun = holdWord(tree, node);
    lines.push(node.kind === "links" ? BTREES.holds(node.entries, noun) : BTREES.pointsAt(node.entries, noun));
  }
  // What the box's own width stands for, which is the other of the two marks a
  // box carries and the only one nothing said. Above the truncated branch, so
  // a node whose children were not all reached still gets it: the count is a
  // lower bound there and says so. Nothing is printed where there is no honest
  // number; `BTREES.widthStands` says when that is.
  const rows = Math.max(...tree.nodes.map((n) => n.depth)) + 1;
  if (box.count > 0 && rows > 1) {
    lines.push(
      tree.version === 2
        ? BTREES.widthStandsV2(box.count, node.kind === "leaf", box.floor)
        : BTREES.widthStands(box.count, tree.job === "chunk" ? "chunk" : "link", box.floor),
    );
  }
  if (node.truncated) {
    // The clause about a missing range only belongs where a range was going to
    // be shown. A version 2 group tree has none to begin with, and a tree whose
    // records were not read has none either; there the clause would read as a
    // second thing gone wrong.
    const ranged = tree.job === "chunk" && tree.records === "read";
    lines.push(ranged ? BTREES.truncated("chunk") : tree.version === 1 ? BTREES.truncated("link") : BTREES.truncatedNoRange);
    return lines;
  }
  // Why a version 2 group node shows no range: its records hold a hash of a
  // name and an id into the fractal heap. On the node rather than under the
  // picture, because it is a fact about what is inside this node and a reader
  // meets it when they press one. Only where the records were read; where they
  // were not, the summary already says so and two notes about one absence read
  // as two problems.
  if (tree.version === 2 && tree.job === "group" && tree.records === "read") lines.push(BTREES.nodeLinkHashes);
  if (node.first_key === "") return lines;
  lines.push(
    tree.job === "chunk"
      ? BTREES.selectedChunkRange(node.first_key, node.last_key)
      : BTREES.selectedRange(node.first_key, node.last_key),
  );
  return lines;
}

/** One box's tooltip: what it is, how much it holds, and where it is. */
export function boxTitle(tree: Tree, box: Box): string {
  if (box.nodes.length > 1) {
    const first = tree.nodes[box.nodes[0] ?? -1];
    const noun = first === undefined ? BTREES.kindIndex : kindWord(first);
    return BTREES.pooled(box.nodes.length, noun);
  }
  const node = tree.nodes[box.nodes[0] ?? -1];
  if (node === undefined) return "";
  return nodeLines(tree, node, box).join("\n");
}

/**
 * What the pressed box is, written out for under the picture.
 *
 * Strings and not elements, as `rowLines` gives data and not elements: what
 * belongs to the picture is which words a tree gets, which turns on its
 * version and its job and is the part worth a test, and what belongs to the
 * panel is the `div` around each one. Nothing is lost by the split, and what
 * is gained is that these two can be read without a document.
 *
 * A box naming a node the tree does not have gives an empty list rather than
 * leaving the last readout up. It cannot happen, because every box was built
 * from this tree, but "cannot happen" is not a reason to leave stale words on
 * screen if it does.
 */
export function readoutLines(tree: Tree, box: Box): string[] {
  if (box.nodes.length > 1) {
    const first = tree.nodes[box.nodes[0] ?? -1];
    return [BTREES.pooled(box.nodes.length, first === undefined ? BTREES.kindIndex : kindWord(first))];
  }
  const node = tree.nodes[box.nodes[0] ?? -1];
  if (node === undefined) return [];
  const lines = nodeLines(tree, node, box);
  // How many bytes the node takes, which only the readout has room for. On the
  // line with the address, not the line with the count: "points at 28 link
  // tables · 480 B" reads as 480 bytes of link tables at least as readily as
  // 480 bytes of this node, and the address line has nothing else on it a size
  // could belong to.
  const first = lines[0];
  if (first !== undefined) lines[0] = `${first} · ${formatBytes(Math.ceil(node.size_bits / 8))}`;
  // What the numbers in a chunk key are. The visible line says "element
  // offset"; this says how many of the numbers are dimensions, so that nobody
  // counts them and gets one too many.
  // A version 1 key ends with an offset within an element that is always zero
  // and a version 2 record does not, so the two notes count the numbers
  // differently. `coords_pad` is which, from the walk, because reading it off
  // the version would be a rule this file made up.
  if (tree.job === "chunk" && node.first_key !== "" && tree.coords > 0) {
    lines.push(tree.coords_pad ? BTREES.chunkRangeNote(tree.coords) : BTREES.chunkRangeNoteV2(tree.coords));
  }
  // A node the template does not place where the walk read it has nowhere in
  // the Listing to open, and the reader has just read a hint that says
  // double-click opens one.
  if (node.path.length === 0) lines.push(BTREES.notInListing);
  return lines;
}

/**
 * The summary under the picture: one line a row, so how wide the tree is at
 * each level is a number as well as a picture, then what the drawing does not
 * cover.
 */
export function rowLines(tree: Tree): RowLine[] {
  const depth = Math.max(...tree.nodes.map((n) => n.depth)) + 1;
  const out: RowLine[] = [];
  for (let d = 0; d < depth; d++) {
    const row = tree.nodes.filter((n) => n.depth === d);
    const first = row[0];
    if (first === undefined) continue;
    const held = row.reduce((sum, n) => sum + n.entries, 0);
    // A version 2 node holds records, both kinds of it, and writes no level of
    // its own. So neither of the version 1 rows fits and neither does
    // `levelTitle`, which names a field those nodes do not have.
    if (first.kind === "leaf") out.push({ text: BTREES.rowLeaves(row.length, held), short: false });
    else if (tree.version === 2) out.push({ text: BTREES.rowInternal(row.length, first.level), short: false });
    else if (first.kind === "links") out.push({ text: BTREES.rowLinks(row.length, held), short: false });
    else if (tree.job === "chunk" && first.level === 0) out.push({ text: BTREES.rowChunks(row.length, held), short: false });
    else {
      // HDF5 counts levels up from the leaves, the opposite of the way the
      // bands are stacked. Said once, where a reader who wondered will look,
      // rather than on every row.
      out.push({ text: BTREES.rowIndex(row.length, first.level), short: false, title: BTREES.levelTitle });
    }
  }
  if (tree.omitted > 0) out.push({ text: BTREES.omitted(tree.omitted), short: true });
  // What the drawing does not cover, in the order a reader meets it: what the
  // records are, and then why there are no keys.
  //
  // A version 2 B-tree is typed and the core reads two of the twelve types. The
  // shape of one does not depend on the type: it comes out of the header's node
  // size, record size and depth and out of the counts in the child pointers, so
  // a tree of a type nobody here decoded still has a true picture. What it does
  // not have is anything to say about the contents, and a picture that stayed
  // silent about that would be a picture a reader took for a full account.
  // Muted rather than warned. The warn colour on these rows put a red line
  // under a healthy tree of any type but the two this decodes, which is most
  // of the twelve; what is red on this panel is `omitted` and `stripPooled`,
  // the lines that say the picture is short of the file.
  if (tree.records === "unread") out.push({ text: BTREES.recordsUnread(tree.record_type_name), short: false, muted: true });
  if (tree.records === "unknown") out.push({ text: BTREES.recordsUnknown(tree.record_type), short: false, muted: true });
  return out;
}

/** The line under the heading: what this tree indexes, and which of the two
 *  structures it is.
 *
 *  The version is on the line for every tree and not only for a version 2 one.
 *  The two have different silhouettes, and a reader comparing a file to
 *  another file cannot tell from a picture of three rows of boxes which kind
 *  they are looking at unless it says. */
/**
 * What this tree holds, and what its keys are in order of.
 *
 * A version 2 tree says which of the twelve record types it is, and the four
 * types that are a group's links or a dataset's chunks are two pairs: by name
 * or by creation order, filtered or not. Those are different orders over the
 * same things, and a line that said only "links" would leave a reader holding
 * two trees of a group unable to tell which was which. Every other named type
 * is the specification's own phrase, which is the word the Listing prints at
 * the header's `type` field.
 *
 * A version 1 tree is untyped and its job is what it hangs under, so it takes
 * the unqualified pair. Its group keys are names and its chunk keys are
 * offsets, which is what the two lines already say.
 */
function jobWords(tree: Tree, other: string): string {
  if (tree.version === 1) return tree.job === "group" ? BTREES.jobLinksByName : tree.job === "chunk" ? BTREES.jobChunks : other;
  switch (tree.record_type) {
    case 5:
      return BTREES.jobLinksByName;
    case 6:
      return BTREES.jobLinksByOrder;
    case 10:
      return BTREES.jobChunksV2(false);
    case 11:
      return BTREES.jobChunksV2(true);
    default:
      return other;
  }
}

export function jobCaption(tree: Tree): string {
  // On `records` and not on the name being empty, because the two cases are
  // not the same fact: a type the specification names is a thing this tree
  // indexes and can be said outright, and a type byte nothing names is a
  // number and a caveat.
  const other = tree.records === "unknown" ? BTREES.jobUnknown(tree.record_type) : BTREES.jobOther(tree.record_type_name);
  return jobWords(tree, other) + BTREES.versionTag(tree.version);
}

/** The line under the picture: what a box's width is and what the number on it
 *  is, which are two different facts about the same box.
 *
 *  A version 2 tree gets its own, because both of those facts are records
 *  rather than links or chunks: a version 2 tree is a B-tree, both kinds of
 *  node hold records, and what a record refers to is not in the tree at all. */
export function widthCaption(tree: Tree): string {
  if (tree.version === 2) return BTREES.widthRecords;
  return tree.job === "group" ? BTREES.widthGroup : BTREES.widthChunk;
}

/** The tree itself: one band per row, each box under its parent's span, and
 *  under the last of them the band of links, chunks or records the tree
 *  indexes. */
export function drawTree(tree: Tree, boxes: readonly Box[], entries: readonly Entry[], width: number, rowH: number): SVGElement {
  const depth = Math.max(...tree.nodes.map((n) => n.depth)) + 1;
  const leaves = leafBand(tree);
  const bands = depth + (leaves === null ? 0 : 1);
  const height = bands * rowH + (bands - 1) * ROW_GAP;
  const svg = document.createElementNS(SVG, "svg");
  svg.setAttribute("class", "btp-svg");
  svg.setAttribute("width", String(width));
  svg.setAttribute("height", String(height));
  svg.setAttribute("viewBox", `0 0 ${width} ${height}`);
  // Three passes in paint order, rather than a box and its number together:
  // the slices of a node sit over the box they divide, and the number a box
  // carries has to stay legible over a slice the mouse is on.
  const labels = document.createDocumentFragment();
  for (const box of boxes) {
    const rect = document.createElementNS(SVG, "rect");
    rect.setAttribute("x", String(box.x));
    rect.setAttribute("y", String(box.row * (rowH + ROW_GAP)));
    rect.setAttribute("width", String(Math.max(1, box.w - 1)));
    rect.setAttribute("height", String(rowH));
    rect.setAttribute("class", "btp-box");
    rect.dataset["key"] = box.key;
    const title = document.createElementNS(SVG, "title");
    title.textContent = boxTitle(tree, box);
    rect.append(title);
    svg.append(rect);
    // The count on the box, where the box is wide enough to hold it. A box too
    // narrow for its own number is not given a clipped one: the readout under
    // the picture says it for whichever box is pressed.
    const one = box.nodes.length === 1 ? tree.nodes[box.nodes[0] ?? -1] : undefined;
    if (one !== undefined && box.w > 26 && rowH >= ROW_H) {
      const text = document.createElementNS(SVG, "text");
      text.setAttribute("x", String(box.x + box.w / 2));
      text.setAttribute("y", String(box.row * (rowH + ROW_GAP) + rowH / 2));
      text.setAttribute("class", "btp-count");
      text.textContent = one.entries.toLocaleString();
      labels.append(text);
    }
  }
  // What is inside the nodes that have nothing under them: one slice an entry,
  // each a run of bytes to go to. `entriesOf` says which nodes get them.
  for (const entry of entries) {
    const rect = document.createElementNS(SVG, "rect");
    rect.setAttribute("x", String(entry.x));
    rect.setAttribute("y", String(entry.row * (rowH + ROW_GAP)));
    rect.setAttribute("width", String(Math.max(1, entry.w - 1)));
    rect.setAttribute("height", String(rowH));
    rect.setAttribute("class", "btp-entry");
    rect.dataset["key"] = entry.key;
    const title = document.createElementNS(SVG, "title");
    title.textContent = entryLines(tree, entry).join("\n");
    rect.append(title);
    svg.append(rect);
  }
  svg.append(labels);
  // The links, chunks or records themselves, as one dashed band across the
  // full width. Undivided because they are not nodes and have no place of
  // their own here: a group's links are inside the link tables in the row
  // above, and a dataset's chunks are elsewhere in the file. Drawn all the
  // same, because a picture that stops at the last row of boxes leaves a chunk
  // tree looking like a group tree with its bottom row missing, and leaves the
  // number printed on a box with nothing to count against.
  if (leaves !== null) {
    const y = depth * (rowH + ROW_GAP);
    const band = document.createElementNS(SVG, "rect");
    band.setAttribute("x", "0.5");
    band.setAttribute("y", String(y + 0.5));
    band.setAttribute("width", String(Math.max(1, width - 2)));
    band.setAttribute("height", String(rowH - 1));
    band.setAttribute("class", "btp-leaf");
    // No `data-key`, so a press on it falls out of `hit` before it looks for a
    // node. There is nowhere to go: the walk never gave these an address.
    const title = document.createElementNS(SVG, "title");
    title.textContent = leaves.title;
    band.append(title);
    svg.append(band);
    const text = document.createElementNS(SVG, "text");
    text.setAttribute("x", String(width / 2));
    text.setAttribute("y", String(y + rowH / 2));
    text.setAttribute("class", "btp-leafcount");
    text.textContent = leaves.label;
    svg.append(text);
  }
  return svg;
}

/**
 * The same nodes by address, in the same rows, and the line naming the stretch
 * they cover.
 *
 * Zoomed to the tree's own span rather than to the file: a tree confined to one
 * small region of a file of gigabytes would otherwise put every mark on the
 * same pixel. Marks closer together than a pixel are pooled, which is the
 * treemap's answer to the same problem and keeps the count honest.
 *
 * The caption comes back beside the picture rather than being left for the
 * panel to work out, because the two ends it names are the two ends this
 * function zoomed to, and a second reading of the node list could only get them
 * right by doing the same arithmetic again.
 */
export function drawStrip(tree: Tree, width: number, rowH: number): { svg: SVGElement; span: string } {
  const from = Math.min(...tree.nodes.map((n) => n.address));
  const to = Math.max(...tree.nodes.map((n) => n.address + Math.ceil(n.size_bits / 8)));
  const span = Math.max(1, to - from);
  const depth = Math.max(...tree.nodes.map((n) => n.depth)) + 1;
  const height = depth * rowH + (depth - 1) * 2;
  const svg = document.createElementNS(SVG, "svg");
  svg.setAttribute("class", "btp-svg");
  svg.setAttribute("width", String(width));
  svg.setAttribute("height", String(height));
  svg.setAttribute("viewBox", `0 0 ${width} ${height}`);
  for (let d = 0; d < depth; d++) {
    // One bucket a pixel: two nodes that would land on the same column are one
    // mark saying how many, rather than one mark drawn twice.
    const buckets = new Map<number, number[]>();
    for (let i = 0; i < tree.nodes.length; i++) {
      const node = tree.nodes[i];
      if (node === undefined || node.depth !== d) continue;
      const at = Math.min(width - 1, Math.floor(((node.address - from) / span) * width));
      const pool = buckets.get(at);
      if (pool === undefined) buckets.set(at, [i]);
      else pool.push(i);
    }
    for (const [at, pool] of buckets) {
      const mark = document.createElementNS(SVG, "rect");
      mark.setAttribute("x", String(at));
      mark.setAttribute("y", String(d * (rowH + 2)));
      mark.setAttribute("width", "1");
      mark.setAttribute("height", String(rowH));
      mark.setAttribute("class", "btp-mark");
      mark.dataset["key"] = keyOf(pool);
      const title = document.createElementNS(SVG, "title");
      const one = pool.length === 1 ? tree.nodes[pool[0] ?? -1] : undefined;
      title.textContent =
        one === undefined
          ? BTREES.stripPooled(pool.length)
          : BTREES.selectedAt(kindWord(one), signWord(one), formatOffset(one.address * 8));
      mark.append(title);
      svg.append(mark);
    }
  }
  return { svg, span: BTREES.stripSpan(formatOffset(from * 8), formatOffset(to * 8)) };
}

/**
 * The key to the number printed on a box: one box drawn the way the picture
 * draws them, with `N` where the count goes, and four words saying what the
 * count counts.
 *
 * Built once and never rebuilt: a legend rebuilt on every paint is work done
 * during a scroll for a picture that did not change. One word of it does
 * depend on the tree, because a version 1 node's number is entries and a
 * version 2 node's is records, so the row is handed back with that word's
 * element and a repaint sets its text. Its own class rather than `.btp-box`,
 * which is pressable and answers the mouse; a key that lights up under the
 * cursor is a control that does nothing.
 */
export function keyRow(): { row: HTMLElement; label: HTMLElement } {
  const row = document.createElement("div");
  row.className = "btp-key";
  const svg = document.createElementNS(SVG, "svg");
  svg.setAttribute("class", "btp-svg btp-keysvg");
  svg.setAttribute("width", "26");
  svg.setAttribute("height", "14");
  svg.setAttribute("viewBox", "0 0 26 14");
  svg.setAttribute("aria-hidden", "true");
  const box = document.createElementNS(SVG, "rect");
  box.setAttribute("x", "0.5");
  box.setAttribute("y", "0.5");
  box.setAttribute("width", "25");
  box.setAttribute("height", "13");
  box.setAttribute("class", "btp-chip");
  const text = document.createElementNS(SVG, "text");
  text.setAttribute("x", "13");
  text.setAttribute("y", "7");
  text.setAttribute("class", "btp-count");
  text.textContent = BTREES.entriesChip;
  svg.append(box, text);
  const label = document.createElement("span");
  label.textContent = BTREES.entriesKey;
  row.append(svg, label);
  return { row, label };
}
