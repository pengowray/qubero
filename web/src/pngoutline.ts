// A PNG's logical outline: what the picture is, then its chunks grouped by
// what they are for, rather than in the order the file keeps them.
//
// The chunk order is the encoder's: a header, then whatever ancillary chunks
// it wrote, then a run of IDAT that can be thousands long, then the end. The
// reader's questions are what size and kind of picture it is, whether it has a
// palette, what colour space it claims, what text it carries, whether it moves,
// and how much compressed data the pixels take. Each is a group here, and each
// row is the chunk that answers it, so a click goes to that chunk.
//
// Reading is `pngOutline`, which asks the document for each chunk's fields by
// name. Grouping is `pngOutlineOf`, which has no document in it and is tested
// on its own. The IDAT and fdAT runs are one row each however many chunks they
// are, since a row per chunk of a large picture would be a list of identical
// rows saying nothing the one row does not.

import { formatOffset } from "./format.ts";
import type { Doc, TemplateNode, TemplateReply } from "./doc.ts";
import type { LogicalNode, LogicalOutline } from "./logicaloutline.ts";
import { countText } from "./strings.ts";

/** Chunks read for the outline. A PNG with more is a picture cut into many
 *  IDAT chunks, and what the outline says of the run is a count and a total
 *  that are already plain at this many; the summary says how many were left. */
const CHUNK_LIMIT = 4096;
/** Characters of a text chunk shown as its row's value. */
const TEXT_CHARS = 120;

/** What the header says, as far as the outline needs it. */
export type PngHeader = {
  readonly width: number;
  readonly height: number;
  readonly depth: number;
  /** The colour type's name as the template gives it: `rgba`, `indexed`. */
  readonly colourType: string;
  readonly interlaced: boolean;
};

/** One chunk, as the reader of the file found it. */
export type PngChunk = {
  readonly index: number;
  /** The four letters. */
  readonly type: string;
  readonly path: readonly number[];
  readonly offsetBits: number;
  /** Bytes of data, the length field's number. */
  readonly dataBytes: number;
  /** The row's name, where the chunk names itself: a text chunk's keyword.
   *  Empty for the chunk's own name. */
  readonly label: string;
  /** What the chunk says, in a few words. */
  readonly said: string;
};

/** The groups, in the order the outline shows them, and which chunks go in
 *  each. A chunk in none of them is in "Other chunks". */
const GROUPS: readonly { readonly id: string; readonly label: string; readonly types: ReadonlySet<string> }[] = [
  { id: "/palette", label: "Palette", types: new Set(["PLTE", "tRNS", "sPLT", "hIST"]) },
  { id: "/colour", label: "Colour space", types: new Set(["gAMA", "cHRM", "sRGB", "iCCP", "sBIT", "bKGD", "cICP", "mDCV", "cLLI"]) },
  { id: "/text", label: "Text", types: new Set(["tEXt", "zTXt", "iTXt", "eXIf"]) },
  { id: "/animation", label: "Animation", types: new Set(["acTL", "fcTL", "fdAT"]) },
  { id: "/data", label: "Image data", types: new Set(["IDAT"]) },
];

/** What each chunk is called on its row, where the four letters alone are not
 *  a name anybody reading the outline knows. */
const CHUNK_NAMES: ReadonlyMap<string, string> = new Map([
  ["IHDR", "Header"],
  ["PLTE", "Colours"],
  ["tRNS", "Transparency"],
  ["sPLT", "Suggested palette"],
  ["hIST", "Colour use"],
  ["gAMA", "Gamma"],
  ["cHRM", "Chromaticities"],
  ["sRGB", "sRGB"],
  ["iCCP", "ICC profile"],
  ["sBIT", "Significant bits"],
  ["bKGD", "Background"],
  ["pHYs", "Pixel size"],
  ["tIME", "Last changed"],
  ["acTL", "Animation"],
  ["fcTL", "Frame"],
  ["fdAT", "Frame data"],
  ["IDAT", "Pixels, compressed"],
  ["eXIf", "Exif"],
]);

/** The colour types in words a reader uses. */
const COLOUR_WORDS: ReadonlyMap<string, string> = new Map([
  ["greyscale", "grey"],
  ["rgb", "RGB"],
  ["indexed", "palette"],
  ["greyscale alpha", "grey and alpha"],
  ["rgba", "RGBA"],
]);

/** Samples a pixel of each colour type has. */
const SAMPLES: ReadonlyMap<string, number> = new Map([
  ["greyscale", 1],
  ["rgb", 3],
  ["indexed", 1],
  ["greyscale alpha", 2],
  ["rgba", 4],
]);

/** The picture in a line: `64 × 48 · 8-bit RGBA · interlaced`. */
export function pictureText(h: PngHeader): string {
  const kind = COLOUR_WORDS.get(h.colourType) ?? h.colourType;
  return [`${h.width.toLocaleString()} × ${h.height.toLocaleString()}`, `${h.depth}-bit ${kind}`, h.interlaced ? "interlaced" : ""]
    .filter(Boolean)
    .join(" · ");
}

/** How many bytes the pixels come to unpacked, every row starting on a byte
 *  as PNG keeps them, filter bytes left out. Null for a colour type the
 *  specification does not define. */
export function pixelBytes(h: PngHeader): number | null {
  const samples = SAMPLES.get(h.colourType);
  if (samples === undefined) return null;
  return h.height * Math.ceil((h.width * h.depth * samples) / 8);
}

/** The outline, from the header and the chunks already read. `more` is how
 *  many chunks past those were not read. */
export function pngOutlineOf(header: PngHeader | null, chunks: readonly PngChunk[], more: number): LogicalOutline {
  const picture = header === null ? "" : pictureText(header);
  const unpacked = header === null ? null : pixelBytes(header);
  const nodes: LogicalNode[] = [
    {
      id: "/", parentId: null, label: "Picture", fullName: "/", depth: 0, group: true, hasChildren: true,
      sourcePath: [], sourceBits: 0, sourceText: formatOffset(0), value: picture, type: "PNG",
      logicalBytes: unpacked, logicalApproximate: false, title: "The picture, and its pixels' size unpacked",
    },
  ];
  const ihdr = chunks.find((c) => c.type === "IHDR");
  if (ihdr !== undefined) nodes.push(leaf(ihdr, "/", 1));
  const placed = new Set<PngChunk>(ihdr === undefined ? [] : [ihdr]);
  const section = (id: string, label: string, rows: readonly LogicalNode[]): void => {
    if (rows.length === 0) return;
    const first = rows[0] as LogicalNode;
    nodes.push({
      id, parentId: "/", label, fullName: id, depth: 1, group: true, hasChildren: true,
      sourcePath: first.sourcePath, sourceBits: first.sourceBits, sourceText: rows.length === 1 ? first.sourceText : "multiple",
      value: countText(rows.length, "chunk"), type: "chunk group",
      logicalBytes: rows.reduce((n, r) => n + (r.logicalBytes ?? 0), 0), logicalApproximate: false, title: label,
    });
    nodes.push(...rows);
  };
  for (const g of GROUPS) {
    const mine = chunks.filter((c) => g.types.has(c.type));
    for (const c of mine) placed.add(c);
    section(g.id, g.label, rowsOf(mine, g.id));
  }
  const other = chunks.filter((c) => !placed.has(c) && c.type !== "IEND");
  section("/other", "Other chunks", rowsOf(other, "/other"));
  const summary = [picture, countText(chunks.length + more, "chunk")].filter(Boolean).join(" · ");
  return { format: "png", title: "PNG image", summary, nodes, total: nodes.length, sizeLabel: "Data size" };
}

/** A group's rows: one per chunk, except that a run of IDAT or fdAT chunks
 *  one after another in the file is one row standing for all of them. Only
 *  one after another: an animation's frames each have their own fdAT run
 *  after their own fcTL, and one row for all of them would stand between two
 *  frames and belong to neither. */
function rowsOf(chunks: readonly PngChunk[], parentId: string): LogicalNode[] {
  const runs: PngChunk[][] = [];
  for (const c of chunks) {
    const last = runs[runs.length - 1];
    const prev = last?.[last.length - 1];
    if (last !== undefined && prev !== undefined && RUNS.has(c.type) && c.type === prev.type && c.index === prev.index + 1) last.push(c);
    else runs.push([c]);
  }
  return runs.map((run) => {
    const first = leaf(run[0] as PngChunk, parentId, 2);
    if (!RUNS.has(first.type)) return first;
    // The run's row stands where its first chunk is, and says how many and
    // how much for all of them.
    return {
      ...first,
      value: countText(run.length, `${first.type} chunk`),
      logicalBytes: run.reduce((n, c) => n + c.dataBytes, 0),
      title: `${CHUNK_NAMES.get(first.type) ?? first.type} · ${countText(run.length, "chunk")}`,
    };
  });
}

/** The chunks a stream is cut into, which run on one after another. */
const RUNS: ReadonlySet<string> = new Set(["IDAT", "fdAT"]);

function idOf(c: PngChunk, parentId: string): string {
  return `${parentId === "/" ? "" : parentId}/${c.index}`;
}

function leaf(c: PngChunk, parentId: string, depth: number): LogicalNode {
  const name = CHUNK_NAMES.get(c.type) ?? c.type;
  return {
    id: idOf(c, parentId), parentId, label: c.label === "" ? name : c.label, fullName: `${parentId}/${c.type}`,
    depth, group: false, hasChildren: false, sourcePath: c.path, sourceBits: c.offsetBits, sourceText: formatOffset(c.offsetBits),
    value: c.said, type: c.type, logicalBytes: c.dataBytes, logicalApproximate: false,
    title: c.label === "" ? `${c.type} chunk` : `${c.type} chunk · ${name}`,
  };
}

// ----- reading -----

/** A structure's fields by name. Empty where the structure has none, or its
 *  bytes have not arrived: the outline is drawn again when they do. */
type Fields = ReadonlyMap<string, TemplateNode>;

function fieldsOf(doc: Doc, node: TemplateNode | undefined): Fields {
  if (node === undefined || node.child_count === 0) return new Map();
  const kids = doc.templateChildren(node.path, 0, node.child_count);
  return kids.status === "ok" ? new Map(kids.node.map((k) => [k.name, k])) : new Map();
}

/** The name of an enumerated value, without the number after it. */
const named = (n: TemplateNode | undefined): string => n?.edit_text || (n?.value ?? "").replace(/ \([^)]*\)$/, "");
const num = (n: TemplateNode | undefined): number => Number(n?.value ?? Number.NaN);

/** A number as a reader would write it, not as a float prints. */
const short = (n: number, places = 2): string => (Number.isFinite(n) ? Number(n.toFixed(places)).toLocaleString() : "");

/** The outline of the PNG `doc` is. */
export function pngOutline(doc: Doc): TemplateReply<LogicalOutline> {
  const list = doc.templateNode([1]);
  if (list.status !== "ok") return list;
  const count = Math.min(list.node.child_count, CHUNK_LIMIT);
  const reply = doc.templateChildren([1], 0, count);
  if (reply.status !== "ok") return reply;
  let header: PngHeader | null = null;
  const chunks: PngChunk[] = [];
  for (const [index, chunk] of reply.node.entries()) {
    const f = fieldsOf(doc, chunk);
    const type = f.get("type")?.value ?? "";
    const data = f.get("data");
    const d = fieldsOf(doc, data);
    if (type === "IHDR") {
      header = {
        width: num(d.get("width")),
        height: num(d.get("height")),
        depth: num(d.get("bit_depth")),
        colourType: named(d.get("color_type")),
        interlaced: num(d.get("interlace")) === 1 || named(d.get("interlace")) === "adam7",
      };
    }
    const { label, said } = describe(doc, type, data, d, header);
    chunks.push({ index, type, path: chunk.path, offsetBits: chunk.offset_bits, dataBytes: num(f.get("length")) || 0, label, said });
  }
  return { status: "ok", node: pngOutlineOf(header, chunks, list.node.child_count - count) };
}

/** What one chunk says, from its fields. A chunk the template reads as bytes
 *  says how many. */
function describe(doc: Doc, type: string, data: TemplateNode | undefined, d: Fields, header: PngHeader | null): { label: string; said: string } {
  const plain = (said: string): { label: string; said: string } => ({ label: "", said });
  switch (type) {
    case "IHDR":
      return plain(header === null ? "" : pictureText(header));
    case "PLTE":
      return plain(countText(d.get("entries")?.child_count ?? 0, "colour"));
    case "tRNS":
      return plain(data?.colour !== undefined ? `${data.colour} is see-through` : countText(d.get("alpha")?.child_count ?? 0, "alpha"));
    case "bKGD":
      return plain(data?.colour ?? (d.has("palette_index") ? `palette entry ${d.get("palette_index")?.value}` : ""));
    case "gAMA": {
      const worth = num(fieldsOf(doc, d.get("gamma")).get("worth"));
      return plain(worth > 0 ? `${short(worth, 5)}, display gamma ${short(1 / worth, 2)}` : "");
    }
    case "cHRM": {
      const x = num(fieldsOf(doc, d.get("white_x")).get("worth"));
      const y = num(fieldsOf(doc, d.get("white_y")).get("worth"));
      return plain(Number.isFinite(x) && Number.isFinite(y) ? `white point ${short(x, 4)}, ${short(y, 4)}` : "");
    }
    case "sRGB":
      return plain(`${named(d.get("rendering_intent"))} rendering`);
    case "iCCP":
      return { label: d.get("profile_name")?.value ?? "", said: "compressed ICC profile" };
    case "pHYs": {
      const dpi = num(d.get("dpi_x"));
      const dpiY = num(d.get("dpi_y"));
      if (Number.isFinite(dpi) && dpi > 0) return plain(Math.round(dpi) === Math.round(dpiY) ? `${short(dpi, 0)} dpi` : `${short(dpi, 0)} × ${short(dpiY, 0)} dpi`);
      return plain(`aspect ${num(d.get("pixels_per_unit_x"))}:${num(d.get("pixels_per_unit_y"))}`);
    }
    case "tIME": {
      const two = (k: string): string => String(num(d.get(k))).padStart(2, "0");
      return plain(`${num(d.get("year"))}-${two("month")}-${two("day")} ${two("hour")}:${two("minute")}:${two("second")} UTC`);
    }
    case "tEXt":
      return { label: d.get("keyword")?.value ?? "", said: clip(d.get("text")?.value ?? "") };
    case "zTXt":
      return { label: d.get("keyword")?.value ?? "", said: "compressed text" };
    case "iTXt": {
      const compressed = num(d.get("compression_flag")) === 1;
      const text = d.get("text");
      return { label: d.get("keyword")?.value ?? "", said: compressed || text === undefined || text.composite ? "compressed text" : clip(text.value) };
    }
    case "acTL": {
      const plays = num(d.get("num_plays"));
      return plain(`${countText(num(d.get("num_frames")), "frame")} · ${plays === 0 ? "loops for ever" : `plays ${countText(plays, "time")}`}`);
    }
    case "fcTL": {
      const seconds = num(d.get("delay_seconds"));
      const size = `${num(d.get("width"))} × ${num(d.get("height"))} at ${num(d.get("x_offset"))}, ${num(d.get("y_offset"))}`;
      return plain(Number.isFinite(seconds) ? `${size} · ${short(seconds, 3)} s` : size);
    }
    default:
      return plain(data === undefined ? "" : data.composite ? "" : `${countText(data.size_bits / 8, "byte")}`);
  }
}

function clip(text: string): string {
  const line = text.replace(/\s+/g, " ").trim();
  return line.length > TEXT_CHARS ? `${line.slice(0, TEXT_CHARS)}…` : line;
}
