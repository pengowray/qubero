// The arithmetic behind the lineage breadcrumb (`lineage.ts`): which streams a
// path passes through on its way down from the file, and how many of their
// names fit in a tab.
//
// A stream is opened inside a reading as one node with one child. The node is
// the compressed run (`decoded`) or the node that joins several runs (type
// `joined → X`); its one child is the stream's own root, marked `space_root`,
// and every field under that child counts its bits from the front of the
// stream rather than from the front of whatever held the run. So the streams a
// path is inside are exactly the prefixes of the path that end on a
// `space_root` node, and the node one step above each is what made it. PNG
// image data is three of them: the IDAT runs joined, the zlib payload
// inflated, the scanlines unfiltered. `spaceSteps` finds them.
//
// Nothing here goes by `TemplateNode.space` beyond whether it is 0, the bytes
// of the reading itself. Any other number is the core's for one
// reading, renumbered from 1 whenever a reading is thrown away, and is not the
// number a tab is opened under; the scanlines of a PNG are space 4 in the
// file's reading and space 1 as a tab. The shape of the path is the same in
// both, so the shape is what is read.
//
// No DOM and no Doc: a node is asked for through a callback, as `trailItems`
// asks, so the tests feed in nodes by hand. The same goes for text widths in
// `cutMiddle`, which are asked of a `Measure`.

import type { TemplateNode } from "../doc.ts";

/** What a step needs to know of a node. */
export type StepNode = Pick<TemplateNode, "name" | "type" | "offset_bits" | "size_bits" | "space_root" | "decoded" | "list" | "space" | "child_count">;

/** One stream a path is inside. Paths are of the reading that was walked. */
export type SpaceStep = {
  /** The node that made the stream: the compressed run, or the joining node. */
  readonly opener: readonly number[];
  /** The stream's one node, under the opener. */
  readonly root: readonly number[];
  /** What to call the stream: see `stepLabel`. */
  readonly label: string;
  /** True for a stream joined from several runs, false for one unpacked from
   *  one run. */
  readonly joined: boolean;
  /** The opener's type: the codec for an unpacked run (`deflate`, `png
   *  scanlines`, `stored`), `joined → X` for a joined one. */
  readonly codec: string;
  /** The type of the stream's root: what the stream was read as. */
  readonly holds: string;
  /** Bits the run takes up where it is, which is 0 for a joining node: its
   *  runs are scattered over the reading and it has no extent of its own. */
  readonly inBits: number;
  /** Bits the stream holds. */
  readonly outBits: number;
  /** Where the run starts, in bits of the stream (or file) it is in. */
  readonly atBits: number;
  /** True when the run is bits of the reading's own bytes rather than of a
   *  stream inside them, so the hex view of that reading's tab can mark it.
   *  The one thing `TemplateNode.space` is read for: 0 is 0 in every
   *  reading. */
  readonly topLevel: boolean;
};

/**
 * The streams `path` passes through, outermost first, in the reading `node`
 * answers for. The path's own first node is not one, even when it is a
 * stream's root: a tab over a stream has that root as its `[]`, and the
 * stream is the tab itself, which the caller already knows about.
 *
 * Null when a node on the way is not there to read yet.
 */
export function spaceSteps(path: readonly number[], node: (path: readonly number[]) => StepNode | null): SpaceStep[] | null {
  const nodes: StepNode[] = [];
  for (let i = 0; i <= path.length; i++) {
    const n = node(path.slice(0, i));
    if (n === null) return null;
    nodes.push(n);
  }
  const steps: SpaceStep[] = [];
  /** Where the stream the walk is in starts, as an index into `nodes`. */
  let from = 0;
  for (let i = 2; i < nodes.length; i++) {
    const root = nodes[i] as StepNode;
    if (!root.space_root) continue;
    const opener = nodes[i - 1] as StepNode;
    steps.push({
      opener: path.slice(0, i - 1),
      root: path.slice(0, i),
      label: stepLabel(nodes, path, from, i - 1, node),
      joined: !opener.decoded,
      codec: opener.type,
      holds: root.type,
      inBits: opener.size_bits,
      outBits: root.size_bits,
      atBits: opener.offset_bits,
      topLevel: opener.space === 0,
    });
    from = i;
  }
  return steps;
}

/**
 * What to call the stream made at `nodes[at]`, looking no higher than
 * `nodes[from]`, the root of the stream the opener is in.
 *
 * The opener's own name is usually a word the format uses for every run
 * (`compressed`, `data`), so the record holding it names it better when the
 * file gave that record a name: a zip entry's `[0] word/document.xml` names
 * its `data`, and the stream is called `word/document.xml`, as the report
 * calls it. A record known only by its place gives its place instead, so the
 * eight members of a gzip file are `members[0].compressed` to
 * `members[7].compressed` rather than eight streams called `compressed`.
 *
 * A record's name counts as a name only when no record near it in the same
 * list has it too and a stream made the same way. PNG chunks are labelled by
 * their type, `[4] zTXt`, so a file with four zTXt chunks would otherwise
 * have four streams called `zTXt`; those are called by their place,
 * `chunks[4].compressed`. A zip names each entry twice, in its local header
 * and in the central directory, but only the local header holds the data, so
 * its name still names the stream. `node` is asked for the neighbours.
 */
export function stepLabel(
  nodes: readonly StepNode[],
  path: readonly number[],
  from: number,
  at: number,
  node: (path: readonly number[]) => StepNode | null = () => null,
): string {
  const opener = nodes[at] as StepNode;
  for (let i = at; i > from; i--) {
    const named = recordName((nodes[i] as StepNode).name);
    if (named === undefined) continue;
    const list = nodes[i - 1] as StepNode;
    if (!nameShared(list, path.slice(0, i - 1), path[i - 1] as number, named, path.slice(i, at), opener, node)) return named;
  }
  for (let i = at; i > from; i--) {
    const up = nodes[i - 1] as StepNode;
    if (!up.list) continue;
    // The element and the opener, with whatever is between them left out:
    // `members[3].compressed`, not `members[3].body.compressed`.
    const element = `${up.name}[${path[i - 1]}]`;
    return i === at ? element : `${element}.${opener.name}`;
  }
  return opener.name;
}

/** The name in a list element's label, `[3] word/document.xml`. Undefined
 *  for an element labelled by its index alone. */
function recordName(label: string): string | undefined {
  return /^\[\d+\]\s+(.+)$/.exec(label)?.[1];
}

/** How many elements either side of a record are looked at for one with the
 *  same name. A list of thousands, a zip's entries say, is not read through
 *  on every move of the cursor; the records whose names repeat, a PNG's
 *  chunks of one type, sit next to each other. */
export const NAME_NEIGHBOURS = 32;

/** Whether an element near the `index`th of the list `list` (at `listPath`)
 *  has the name `name` too, and a node like `opener` at `below` under it:
 *  whether another stream would be called the same. */
function nameShared(
  list: StepNode,
  listPath: readonly number[],
  index: number,
  name: string,
  below: readonly number[],
  opener: StepNode,
  node: (path: readonly number[]) => StepNode | null,
): boolean {
  const lo = Math.max(0, index - NAME_NEIGHBOURS);
  const hi = Math.min(list.child_count - 1, index + NAME_NEIGHBOURS);
  for (let k = lo; k <= hi; k++) {
    if (k === index) continue;
    const other = node([...listPath, k]);
    if (other === null || recordName(other.name) !== name) continue;
    const made = node([...listPath, k, ...below]);
    if (made !== null && made.type === opener.type && made.decoded === opener.decoded) return true;
  }
  return false;
}

/**
 * `text` cut down to `room` pixels by taking characters out of its middle,
 * with an ellipsis where they were: whole when it fits, null when fewer than
 * `minChars` characters would be left.
 *
 * The middle, because the end of a name is as often what tells it from its
 * neighbours as the start is: every entry of a PyTorch zip starts
 * `state-dict-zip/`, and what differs is the file name after it. A third of
 * what is kept is the start and two thirds the end.
 */
export function cutMiddle(text: string, room: number, measure: (text: string) => number, minChars: number): string | null {
  if (measure(text) <= room) return text;
  if (text.length <= minChars) return null;
  const cut = (kept: number): string => {
    const head = Math.ceil(kept / 3);
    return `${text.slice(0, head)}…${text.slice(text.length - (kept - head))}`;
  };
  // The most characters that fit, by halving. Keeping one more character
  // never makes the text narrower, so the search is sound.
  let lo = minChars;
  let hi = text.length - 1;
  if (measure(cut(lo)) > room) return null;
  while (lo < hi) {
    const mid = Math.ceil((lo + hi) / 2);
    if (measure(cut(mid)) <= room) lo = mid;
    else hi = mid - 1;
  }
  return cut(lo);
}

/** What `fitCrumbs` decided for one place in the row. */
export type CrumbFit =
  | { readonly kind: "crumb"; readonly index: number; readonly room: number | null }
  | { readonly kind: "more"; readonly from: number; readonly to: number };

/**
 * Which crumbs of a breadcrumb to draw in `room` pixels.
 *
 * The last crumb is what the row is about (the tab's own stream), so it is
 * always drawn, and cut with an ellipsis when it does not fit on its own
 * (`room` then says how wide it may be; null means whole). The first is where
 * it all came from and goes in next. After those, crumbs are put back from
 * the end, nearest the last first, while they fit. Whatever is left out is one
 * `more` mark standing for crumbs `from` to `to` (end exclusive), whose names
 * the hover card lists.
 *
 * `widths` are the crumbs' drawn widths, `sep` the width of a separator and
 * `more(n)` the width of the mark for `n` left-out crumbs.
 */
export function fitCrumbs(widths: readonly number[], room: number, sep: number, more: (hidden: number) => number): CrumbFit[] {
  const n = widths.length;
  if (n === 0) return [];
  const w = (i: number): number => widths[i] as number;
  const total = widths.reduce((a, b) => a + b, 0) + sep * (n - 1);
  if (total <= room) return widths.map((_, index) => ({ kind: "crumb", index, room: null }));
  const last = n - 1;
  if (n === 1) return [{ kind: "crumb", index: 0, room }];
  // The first and the last, with every crumb between them behind the mark.
  // The mark is left out when it stands for nothing.
  const cost = (kept: readonly number[]): number => {
    const hidden = n - kept.length;
    const parts = kept.length + (hidden > 0 ? 1 : 0);
    return kept.reduce((a, i) => a + w(i), 0) + (hidden > 0 ? more(hidden) : 0) + sep * (parts - 1);
  };
  const kept = [0, last];
  if (cost(kept) > room) {
    // Not even the two ends fit: the first goes behind the mark as well, and
    // the last is cut to what is left.
    const rest = room - more(n - 1) - sep;
    return [
      { kind: "more", from: 0, to: last },
      { kind: "crumb", index: last, room: Math.max(0, Math.min(w(last), rest)) },
    ];
  }
  for (let i = last - 1; i > 0; i--) {
    const tryKept = [0, i, ...kept.slice(1)];
    if (cost(tryKept) > room) break;
    kept.splice(1, 0, i);
  }
  const out: CrumbFit[] = [{ kind: "crumb", index: 0, room: null }];
  const second = kept[1] as number;
  if (second > 1) out.push({ kind: "more", from: 1, to: second });
  for (const index of kept.slice(1)) out.push({ kind: "crumb", index, room: null });
  return out;
}
