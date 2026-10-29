// The run a value plot draws: which runs can be plotted, what their channels
// and dimensions are, and their numbers, read from the run's own bytes.
//
// Part of candidate `value-plot` (`valueplot.ts` has the whole piece's
// story). Kept apart from the drawing so the line, the grid and the list
// pane's figure all read a run the same way, and so the listing's question
// "can this heading have a Plot button?" (`plottable`) does not load the
// drawing code's imports to answer.
//
// The numbers come from `Doc.read` and are decoded with a DataView by the
// element type the core names (`i16 le[]`), since `runCells` writes each
// element as text and a plot wants millions of them. A run whose bytes are
// elsewhere (inside a decoded stream) is read with `fieldBytes`.

import type { Doc, TemplateNode } from "../doc.ts";
import { childWord } from "../strings.ts";
import { decode, elementType, numType, type NumType } from "./decimate.ts";

/** The most bytes of one run the plot reads. A run past this is plotted from
 *  its start, and the plot says how much of it that is. Sixteen MiB is eight
 *  million 16-bit samples, three minutes of CD audio, decoded in well under a
 *  second. */
const MAX_PLOT_BYTES = 16 << 20;
/** Most channels drawn as lanes. A table of more numeric columns than this
 *  has outgrown lanes, and is the Table view's to show. */
const MAX_LANES = 8;

/** Whether `node` is a run the plot can draw, from the node alone, since the
 *  listing asks this of every heading on every paint: a list of fixed-width
 *  numbers, packed one after another from a whole byte. */
export function plottable(node: TemplateNode): boolean {
  if (!node.composite || node.list !== true || node.child_count < 2) return false;
  const t = numType(elementType(node.type));
  if (t === null) return false;
  return node.offset_bits % 8 === 0 && !node.lsb_first && node.child_count * t.bytes * 8 === node.size_bits;
}

/** A run, and how to read it. */
export type RunSource = {
  readonly path: readonly number[];
  readonly node: TemplateNode;
  readonly type: NumType;
  /** Elements to one frame: the channels of interleaved audio. One for a run
   *  that is not a table. */
  readonly stride: number;
  /** A name per channel, or one empty name when there is one channel. */
  readonly names: readonly string[];
  /** What the template calls one frame of a table: a sample. Null for a run
   *  that is not a table, whose elements are numbered as the listing numbers
   *  them. */
  readonly rowWord: string | null;
  /** What one element is called, for counting them: values, weights. */
  readonly unit: string;
  /** Frames a second, when the run is spaced in time. */
  readonly rate: number | null;
  /** The run's two dimensions, for a tensor that says it has two: `columns`
   *  values to a row, stored row after row. */
  readonly dims: { readonly rows: number; readonly columns: number } | null;
};

/**
 * Everything the plot needs to know about the run at `path` beyond its bytes:
 * the channels the template's table shape gives it (a WAV's `channels`), and
 * the two dimensions a tensor's element count is the product of (a GGUF
 * tensor's `dims`). Null for a run the plot cannot draw.
 */
export function runSource(doc: Doc, path: readonly number[]): RunSource | null {
  const reply = doc.templateNode(path);
  if (reply.status !== "ok" || !plottable(reply.node)) return null;
  const node = reply.node;
  const type = numType(elementType(node.type));
  if (type === null) return null;
  let stride = 1;
  let names: string[] = [""];
  let rowWord: string | null = null;
  let rate: number | null = null;
  const shape = node.table === true ? doc.tableShape(path) : null;
  if (shape !== null && shape.status === "ok" && shape.node !== null) {
    const s = shape.node;
    const columns = s.columns ?? 1;
    if (columns > 1 && columns <= MAX_LANES && node.child_count % columns === 0) {
      stride = columns;
      names = Array.from({ length: columns }, (_, i) => (s.names.length === columns ? (s.names[i] ?? "") : `${s.column_word ?? "column"} ${i + 1}`));
    }
    if (s.row_word !== null) rowWord = s.row_word;
    if (s.rate !== null && s.rate > 0) rate = s.rate;
  }
  return { path, node, type, stride, names, rowWord, unit: childWord(node), rate, dims: stride === 1 ? dimsOf(doc, path, node.child_count) : null };
}

/**
 * The two dimensions of a run whose element count the template works out as
 * the product of a list of sizes: a GGUF tensor's `dims`, which are written
 * innermost first, so the first is the length of a row. Null for a run of one
 * dimension, or of more than two, which have no one grid to draw.
 */
function dimsOf(doc: Doc, path: readonly number[], count: number): { rows: number; columns: number } | null {
  const origins = doc.origins(path);
  if (origins.status !== "ok") return null;
  for (const o of origins.node) {
    if (o.role !== "count") continue;
    const list = doc.templateNode(o.path);
    if (list.status !== "ok" || !list.node.composite || list.node.child_count < 2) continue;
    const kids = doc.templateChildren(o.path, 0, Math.min(8, list.node.child_count));
    if (kids.status !== "ok") continue;
    const sizes = kids.node.map((k) => Number(k.value));
    if (sizes.some((s) => !Number.isInteger(s) || s < 1)) continue;
    const big = sizes.filter((s) => s > 1);
    if (big.length !== 2 || sizes.reduce((a, b) => a * b, 1) !== count) continue;
    const columns = sizes[0] === 1 ? (big[0] as number) : (sizes[0] as number);
    return { rows: count / columns, columns };
  }
  return null;
}

/** The decoded run, or null while its bytes are still arriving. */
export type RunValues = {
  readonly values: Float64Array;
  /** How many elements of the run the values are: all of them, unless the run
   *  is longer than `MAX_PLOT_BYTES`. */
  readonly shown: number;
};

export function readRun(doc: Doc, src: RunSource): RunValues | null {
  const n = src.node;
  const frameBytes = src.type.bytes * src.stride;
  const total = n.child_count * src.type.bytes;
  const want = Math.min(total, Math.floor(MAX_PLOT_BYTES / frameBytes) * frameBytes);
  let bytes: Uint8Array;
  if (n.space === doc.space && !n.joined) {
    const r = doc.read(n.offset_bits / 8, want);
    if (!r.complete) return null;
    bytes = r.bytes;
  } else {
    const r = doc.fieldBytes(src.path, want);
    if (r.status !== "ok") return null;
    bytes = Uint8Array.from(r.node.bytes);
  }
  const shown = Math.floor(bytes.length / src.type.bytes);
  return { values: decode(bytes, src.type, shown), shown };
}
