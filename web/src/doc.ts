// Document facade: owns the wasm Editor and streams chunks in from a File/Blob.
// Nothing here ever reads the whole file; only the chunks the view asks for.

import init, { Editor } from "./pkg/qubero_wasm.js";

const CHUNK_SIZE = 64 * 1024;
/** Chunks to fetch past one the template asked for and did not have. Placing
 * fields runs forward through a file, so the chunk after the missing one is
 * nearly always the next one wanted. They are read in one go: asking the file
 * for 64 KiB at a time costs a round trip each time, and reading a file's
 * worth of metadata that way is hundreds of them. */
const READ_AHEAD = 48;
const CHUNK_CAPACITY = 512; // 32 MiB resident at most

// What the file(1) rules get to look at. Their offsets count from the start of
// the file, so this is always the head and never a window from elsewhere. Rules
// that search rather than test a fixed offset stop where it stops: the widest
// in the database reaches 16 KiB, and a handful measure from the end of the
// file, which nothing here can answer.
const IDENTIFY_WINDOW = 64 * 1024;

/** The OME-NGFF keys that distinguish an OME-Zarr metadata JSON document from
 * an ordinary Zarr store record. */
function isOmeZarrMetadata(name: string, bytes: Uint8Array): boolean {
  const leaf = name.replace(/\\/g, "/").split("/").pop()?.toLowerCase();
  if (leaf !== ".zattrs" && leaf !== "zarr.json") return false;
  const text = new TextDecoder().decode(bytes);
  return /"(?:multiscales|ome)"\s*:/.test(text);
}

type MagicModule = typeof import("./pkg-magic/qubero_magic.js");
let magic: Promise<MagicModule> | null = null;

/** The rule database, fetched the first time a file needs it and kept after. */
function loadMagic(): Promise<MagicModule> {
  magic ??= import("./pkg-magic/qubero_magic.js")
    .then(async (m) => {
      await m.default();
      return m;
    })
    .catch((e: unknown) => {
      // A failed fetch is forgotten rather than remembered, so opening the next
      // file tries again instead of inheriting one bad moment offline.
      magic = null;
      throw e;
    });
  return magic;
}

/**
 * Fetch the rule database before a file asks for it, so the wait lands on an
 * idle browser rather than on someone watching. Called once a file is open and
 * the page has settled.
 *
 * It costs a megabyte to someone who only ever opens formats the editor has a
 * template for, which is why it waits for idle and stops on a connection that
 * would rather not: Save Data on, or anything the browser rates below 4G.
 * Those cases still identify on demand, and say they are doing it.
 */
export function prefetchMagic(): void {
  if (magic !== null) return;
  const link = (navigator as Navigator & { connection?: NetworkInformation }).connection;
  if (link?.saveData === true) return;
  if (link?.effectiveType !== undefined && link.effectiveType !== "4g") return;
  const start = (): void => {
    // Nothing waits on this. A failure clears itself in `loadMagic`, so the
    // first file that needs the rules asks for them again.
    void loadMagic().catch(() => {});
  };
  // Safari only got requestIdleCallback in 16.4, so this checks rather than
  // assumes. The timeout stops a busy page putting it off forever.
  const idle: unknown = window.requestIdleCallback;
  if (typeof idle === "function") window.requestIdleCallback(start, { timeout: 5000 });
  else window.setTimeout(start, 2000);
}

/** Rule bundles already fetched, so a second file of the same kind is free. */
const ruleCache = new Map<string, Promise<string | null>>();

/** One bundle of signature rules, or null when it cannot be had. */
function fetchRules(name: string): Promise<string | null> {
  let p = ruleCache.get(name);
  if (p === undefined) {
    p = fetch(`diesig/${name}`)
      .then((r) => (r.ok ? r.text() : null))
      .catch(() => null);
    ruleCache.set(name, p);
  }
  return p;
}

/** The parts of the Network Information API we use; Safari has none of it. */
type NetworkInformation = { saveData?: boolean; effectiveType?: string };

/** The subset of Blob we need; lets tests and dev tooling supply synthetic files. */
export type ByteSource = {
  readonly size: number;
  readonly name: string;
  slice(start: number, end: number): { arrayBuffer(): Promise<ArrayBuffer> } | Blob;
};

export type TemplateNode = {
  readonly path: readonly number[];
  readonly name: string;
  readonly type: string;
  readonly offset_bits: number;
  readonly size_bits: number;
  readonly value: string;
  /** What the in-place editor starts with; differs from `value` for enums. */
  readonly edit_text: string;
  readonly kind: "uint" | "int" | "float" | "bytes" | "unread" | "str" | "magic" | "enum" | "composite";
  readonly ok: boolean;
  readonly child_count: number;
  /** What one child is called, for counting them. Absent when they are items. */
  readonly unit?: string;
  readonly composite: boolean;
  /** True when `writeNode` accepts typed text for this field. */
  readonly editable: boolean;
  /** Bytes the value occupies: short of the field's size when text is padded
   * or terminated, since neither belongs to the value. */
  readonly value_bytes: number;
  /** Where the value starts, past a byte-order mark if the field has one. */
  readonly value_offset_bits: number;
  /** How the encoding was settled, or that the bytes do not fit it. */
  readonly read_as: string | null;
};

/** The bit range a successful `writeNode` replaced. */
/** One entry of the annotation column: a field, a run of them, or a stretch
 *  the template does not describe. */
export type Span = {
  readonly path: number[];
  readonly name: string;
  /** What it sits inside, outermost first. */
  readonly trail: string[];
  readonly type: string;
  readonly offset_bits: number;
  readonly size_bits: number;
  readonly value: string;
  readonly kind: string;
  readonly gap: boolean;
  /** Fields this entry stands for, when a run of numbers is shown as one. */
  readonly count: number;
  /** A structure that reads on one row, already joined: an instruction rather
   *  than its opcode and its immediate. Null for a field that reads as its own
   *  value. */
  readonly line: string | null;
  /** The first few values of a run shown as one entry. */
  readonly sample: string[];
};

/** What the byte-class scan behind the overview has found so far. */
export type OverviewState = {
  /** True once every bucket is classified; until then ask again. */
  readonly done: boolean;
  /** Bytes one bucket stands for. A power of two, and 1 for a small file. */
  readonly bucket_bytes: number;
  readonly total_buckets: number;
  /** One digit per finished bucket, in file order: 0 zeros, 1 one repeated
   *  byte, 2 text, 3 data, 4 high entropy. */
  readonly classes: string;
  readonly zero_bytes: number;
  readonly text_bytes: number;
  /** How far the scan has read, in bytes. */
  readonly read_bytes: number;
};

/** The same scan over one block, with what the block's bytes turned out to be
 *  as a whole rather than what each of its buckets did. */
export type FocusState = OverviewState & {
  /** The block, in bytes. */
  readonly start: number;
  readonly end: number;
  /** Entropy over the block, and the most a block this long could reach. */
  readonly entropy: number;
  readonly entropy_max: number;
  readonly distinct: number;
  /** The values that appear most, commonest first. */
  readonly common: readonly { readonly value: number; readonly count: number }[];
};

/** How the text in the search bar is read. */
export type NeedleKind = "hex" | "text" | "regex";

/** Everything a search needs to know, which is everything the bar holds. */
export type Query = {
  readonly kind: NeedleKind;
  readonly text: string;
  /** Text only: match letters in either case. */
  readonly fold: boolean;
  readonly backward: boolean;
};

/** What one window of a search found. Offsets are bytes. */
export type SearchStep =
  | { readonly step: "found"; readonly at: number; readonly len: number }
  | { readonly step: "more"; readonly resume: number }
  | { readonly step: "end" };

/** What one other field decided about this one. `points` is the other way
 *  round: this field holds an offset, and that is where it points. */
export type Origin = {
  readonly role: "length" | "count" | "type" | "position" | "points";
  /** The field as the reader would name it: `len`, or `tensors[3].offset`. */
  readonly label: string;
  /** Where it is, so the reader can go there. Empty for a `points` entry. */
  readonly path: number[];
  /** What it says, in brief. Empty when it could not be read. */
  readonly value: string;
  /** For `points`: the bit this field's value points at. */
  readonly target_bits: number | null;
};

/** What a type permits, beyond what this file's bytes happen to say. */
/** One row of a cross-reference stream, already decoded. `offset` is a real
 *  place in the file for an in-use row and -1 for every other kind. */
/** One object inside an object stream. Its offset is inside the decompressed
 *  bytes, so there is nowhere in the file to go to. */
export type ObjStmObject = {
  readonly number: number;
  /** How long the object is in the decompressed bytes. */
  readonly len: number;
  /** The object as written, cut at the limit the core keeps. */
  readonly text: string;
  readonly cut: boolean;
};

/** One filter undone on the way back to a chunk's elements. */
export type ChunkStep = {
  readonly filter: string;
  readonly in_bytes: number;
  readonly out_bytes: number;
  /** True when this chunk's own mask said the filter was not applied to it. */
  readonly skipped: boolean;
};

/** One object of an HDF5 file, as the contents list reads it. */
export type ContentObject = {
  readonly path: readonly number[];
  /** The path it goes by inside the file: `/obs/n_genes`. */
  readonly name: string;
  readonly group: boolean;
  /** What the file calls it, where it says: `dataframe`, `csr_matrix`. */
  readonly encoding: string;
  readonly shape: readonly number[];
  /** What one element is. */
  readonly element: string;
  /** Which of the three ways its bytes are kept: "contiguous", "compact",
   *  "chunked", or nothing at all for a group. */
  readonly storage: string;
  /** How many bytes, where they are in one run. */
  readonly bytes: number;
  /** The chunk it is kept in, where it is kept in chunks. */
  readonly chunk_dims: readonly number[];
  /** The filters its chunks were written through, in that order. */
  readonly filters: readonly string[];
  /** Where its object header is. */
  readonly address: number;
};

/** What an HDF5 file holds, and what kind of file it is. */
export type Contents = {
  readonly objects: readonly ContentObject[];
  readonly total: number;
  /** Whether it is an AnnData object, and what the root group calls itself. */
  readonly anndata: boolean;
  readonly encoding: string;
  readonly rows: number;
  readonly columns: number;
};

export type XrefRow = {
  readonly object: number;
  readonly kind: string;
  /** The type number the row held: 0, 1, 2, or whatever an undefined type
   *  wrote. `kind` is the same word for every undefined one. */
  readonly type_raw: number;
  readonly offset: number;
  readonly second: number;
  readonly third: number;
};

export type TypeInfo = {
  readonly kind: "magic" | "enum" | "flags" | "float" | "quant" | "xref" | "objstm" | "chunk" | "plain";
  /** The type's own name, for an enum or a flags field. */
  readonly name: string;
  /** Magic: what the format requires, and what is there. */
  readonly expected: number[];
  readonly actual: number[];
  /** Enum: every value it names, and the one in the file. */
  readonly cases: readonly { readonly value: number; readonly name: string }[];
  readonly current: number;
  /** Enum: what the value in the file is called, where the name comes from a
   *  counted run of values rather than from `cases`. Empty when it has none. */
  readonly named: string;
  readonly hex: boolean;
  /** Flags: one entry per bit of the field, from bit 0 up. */
  readonly bits: readonly { readonly bit: number; readonly name: string | null; readonly set: boolean }[];
  /** Float: which layout it is, how wide, and its bits in value order in hex. */
  readonly format: string;
  /** Float: how many bits wide. Quant: how many bits one weight is worth. */
  readonly width: number;
  readonly pattern: string;
  /** Quant: the block's shared scale, and what it pairs with it, named as the
   *  file names it. `second_name` is empty where the layout has no second. */
  readonly scale: number;
  readonly second_name: string;
  readonly second: number;
  /** Quant: whether that second number is taken away rather than added, and
   *  whether it is multiplied by the group's own minimum first. */
  readonly second_subtract: boolean;
  readonly second_per_group: boolean;
  /** Quant: where the block starts, so a weight's bits can be found from the
   *  offset it carries. */
  readonly block_bits: number;
  /** Quant: the scale the block keeps for each run of weights, where it keeps
   *  them, and how many weights one run covers. Empty for a block with one
   *  scale for all of them. */
  readonly groups: readonly QuantGroup[];
  readonly group_weights: number;
  /** Quant: taken off the packed value to get the stored one, and whether that
   *  value is read signed instead of biased. */
  readonly bias: number;
  readonly signed: boolean;
  /** Quant: every weight the block stands for, in the order the tensor reads
   *  them, and which one the cursor is inside (-1 for none). */
  readonly weights: readonly QuantWeight[];
  readonly at: number;
  /** Xref: the widths from `/W`, and the PNG predictor where there was one
   *  (-1 where there was not). */
  readonly xref_widths: readonly number[];
  readonly xref_predictor: number;
  /** Xref: how many bytes the rows are in the file, and how many once
   *  decompressed. */
  readonly xref_packed: number;
  readonly xref_decoded: number;
  /** Xref: how many rows of each kind, over the whole table rather than over
   *  the ones listed. */
  readonly xref_free: number;
  readonly xref_in_file: number;
  readonly xref_in_stream: number;
  readonly xref_unknown: number;
  /** Xref: the rows, and how many there are altogether. */
  readonly xref_rows: readonly XrefRow[];
  readonly xref_total: number;
  /** Xref: why there are no rows, where there are none. An object stream that
   *  would not open says why here too. Empty otherwise. */
  readonly problem: string;
  /** ObjStm: how many bytes the objects are in the file, and how many once
   *  decompressed. */
  readonly objstm_packed: number;
  readonly objstm_decoded: number;
  /** ObjStm: the object stream this one continues, or -1 where it continues
   *  none. */
  readonly objstm_extends: number;
  /** ObjStm: the objects, and how many there are altogether. */
  readonly objstm_objects: readonly ObjStmObject[];
  readonly objstm_total: number;
  /** Chunk: how many bytes it is in the file, and how many its elements came
   *  to once the filters were undone. */
  readonly chunk_packed: number;
  readonly chunk_decoded: number;
  /** Chunk: each filter, in the order it was undone. */
  readonly chunk_steps: readonly ChunkStep[];
  /** Chunk: what one element is called, the first few of them, and how many
   *  there are altogether. */
  readonly chunk_element_type: string;
  readonly chunk_values: readonly string[];
  readonly chunk_total: number;
};

/** One run of weights inside a block that share a scale of their own. */
export type QuantGroup = {
  /** The scale as stored, after whatever bias the type takes off it. */
  readonly scale: number;
  /** The minimum taken off every weight in the run, or null where the type has
   *  none. */
  readonly min: number | null;
};

/** One weight of a block of packed weights. */
export type QuantWeight = {
  /** The stored integer, after whatever bias the layout takes off it. */
  readonly q: number;
  /** That integer through the block's scale: the number the model reads. */
  readonly value: number;
  /** The run holding its low bits, and the rest of the packed value where the
   *  layout keeps that somewhere else in the block. */
  readonly bits: QuantPart;
  readonly high: QuantPart | null;
};

/** One run of bits that makes up part of a packed weight. */
export type QuantPart = {
  /** The block field these bits are in, as the file names it: `qs`, `qh`. */
  readonly field: string;
  /** Where they are, counted in bits from the start of the block. */
  readonly bit: number;
  readonly width: number;
  /** Where they sit in the packed value: 0 for the low part. */
  readonly shift: number;
};

/** What one rule, or the editor itself, concluded about what produced a file. */
export type ToolMatch = {
  /** The database's own word: `packer`, `compiler`, `protector`. */
  readonly category: string;
  readonly name: string;
  readonly version: string | null;
  /** Free text written by the rule's author, shown as written. */
  readonly options: string | null;
  /** The signature file that answered, or `OWN_SOURCE` for the editor's own. */
  readonly source: string;
};

/**
 * What `source` says for an answer the editor worked out itself rather than
 * took from the signature database. Kept in step with `dosbasic::SOURCE` in
 * the core, so the dialog credits each answer to whatever actually found it.
 */
export const OWN_SOURCE = "qubero";

/**
 * The largest a .COM can be. It is loaded whole into one 64 KiB segment below
 * the stack, so anything bigger is not one, whatever its name says.
 */
const COM_LIMIT = 65280;

/**
 * How much of a DOS executable the signature rules get to look at.
 *
 * More than the 64 KiB an unknown format is identified from, because what says
 * which BASIC runtime one of these needs is at the end of the program rather
 * than the start, and so is the entry point of anything but a small one. A DOS
 * program cannot be much larger than this and still be one, and 1 MiB is 16
 * chunks against a cache of 512.
 */
const DOS_WINDOW = 1024 * 1024;

export type WrittenRange = { readonly offset_bits: number; readonly size_bits: number };

/** What the file(1) rules made of a file the editor has no template for. */
export type Identification = {
  /** The rule's own sentence, values and all: `PNG image data, 1280 x 720`. */
  readonly message: string;
  /** Media type, or "" where the rule carries none. */
  readonly mime: string;
  /** Extensions the rule lists, alphabetical. */
  readonly ext: readonly string[];
  /** The matching rule's strength; higher beat it to the answer. */
  readonly strength: number;
  /** The rule file it came from. */
  readonly source: string;
};

/**
 * What to call a template built from a rule.
 *
 * One extension is the best name there is: `gif` says more than the `images`
 * rule file it came from. Several are no use, because the rules hand them over
 * as an unordered set, so "the first" is whichever way they fell: a Windows
 * executable would be called `com` as readily as `exe`. In that case the rule
 * file's own name is at least stable and is what the dialog says beside it.
 */
function signatureName(id: Identification): string {
  if (id.ext.length === 1) return id.ext[0] ?? id.source;
  if (id.source !== "") return id.source;
  return (id.mime.split("/").pop() ?? "").replace(/^x-/, "");
}


export type TemplateReply<T> =
  | { readonly status: "ok"; readonly node: T }
  | { readonly status: "pending"; readonly reachedBytes: number }
  /** Still being worked out. `reachedBytes` is how far into the file the
   * reading has got. Asking again carries on from there. */
  | { readonly status: "working"; readonly reachedBytes: number }
  | { readonly status: "error"; readonly message: string };

type RawReply<T> =
  | { status: "ok"; node: T; wanted?: number[] }
  | { status: "pending"; chunks: number[]; reached_bytes: number }
  | { status: "working"; reached_bytes: number }
  | { status: "error"; message: string };

export class ReadFailure extends Error {
  constructor(
    readonly offset: number,
    readonly length: number,
    cause: unknown,
  ) {
    super(describeReadFailure(offset, length, cause), { cause });
    this.name = "ReadFailure";
  }
}

function describeReadFailure(offset: number, length: number, cause: unknown): string {
  const where = `${formatBytes(length)} at offset 0x${offset.toString(16).toUpperCase()}`;
  const reason =
    cause instanceof DOMException && cause.name === "NotReadableError"
      ? "The file has changed or moved since it was opened."
      : cause instanceof Error
        ? cause.message
        : "The file may have changed or moved since it was opened.";
  return `Could not read ${where} from the original file. ${reason}`;
}

/** An offset as `0x1f`, or `0x1f+3b` when it falls inside a byte. Lowercase
 * to match the hex gutter, so every address in the app reads the same way. */
export function formatOffset(bits: number): string {
  const byte = Math.floor(bits / 8);
  const rem = bits % 8;
  return `0x${byte.toString(16)}${rem === 0 ? "" : `+${rem}b`}`;
}

/**
 * The shift-and-mask that reads `width` bits starting at `bitOffset` and leaves
 * them right-aligned in a plain number.
 *
 * Bits are counted the way the rest of the editor counts them: bit 0 of a byte
 * is its top bit, so a field three bits into a byte starts under the mask
 * `0x1f`. The first term is masked because the bits above it belong to whatever
 * came before; the last is shifted down by whatever the field leaves behind in
 * its final byte. `b[n]` is the byte at address `n`.
 */
export function bitFormula(bitOffset: number, width: number): string {
  const first = Math.floor(bitOffset / 8);
  const skip = bitOffset % 8;
  const bytes = Math.ceil((skip + width) / 8);
  const trailing = bytes * 8 - (skip + width);
  const terms: string[] = [];
  for (let i = 0; i < bytes; i++) {
    const shift = 8 * (bytes - 1 - i) - trailing;
    let term = `b[0x${(first + i).toString(16)}]`;
    if (i === 0 && skip !== 0) {
      const mask = ` & 0x${(0xff >> skip).toString(16).padStart(2, "0")}`;
      // Brackets only where something else would bind tighter than the mask.
      term = bytes === 1 && shift === 0 ? term + mask : `(${term}${mask})`;
    }
    if (shift > 0) term = `${term} << ${shift}`;
    else if (shift < 0) term = `${term} >> ${-shift}`;
    terms.push(term);
  }
  return terms.join(" | ");
}

/**
 * How much of the file something covers, written the way an address is: whole
 * bytes, then whatever bits are left over. `12`, or `0+4b` for a nibble.
 */
export function formatLength(bits: number): string {
  const bytes = Math.floor(bits / 8);
  const rem = bits % 8;
  return rem === 0 ? String(bytes) : `${bytes}+${rem}b`;
}

export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  const units = ["KiB", "MiB", "GiB", "TiB"];
  let x = n / 1024;
  let i = 0;
  while (x >= 1024 && i < units.length - 1) {
    x /= 1024;
    i++;
  }
  return `${x < 10 ? x.toFixed(2) : x < 100 ? x.toFixed(1) : Math.round(x)} ${units[i]}`;
}

/** In-memory bytes as a `ByteSource`, for documents that come out of another
 *  one rather than off a disk: a decompressed zip entry, a run of bytes opened
 *  on its own. */
export function bytesSource(bytes: Uint8Array, name: string): ByteSource {
  return {
    size: bytes.length,
    name,
    slice(start, end) {
      return { arrayBuffer: () => Promise.resolve(bytes.slice(start, end).buffer as ArrayBuffer) };
    },
  };
}

export type ReadResult = {
  readonly bytes: Uint8Array;
  /** True when every byte came from loaded data. False means a reload will follow. */
  readonly complete: boolean;
};

let wasmReady: Promise<unknown> | null = null;
function ensureWasm(): Promise<unknown> {
  wasmReady ??= init();
  return wasmReady;
}

export class Doc {
  private readonly inflight = new Set<number>();
  private readonly listeners = new Set<() => void>();
  /** A go at unfinished work is already queued. */
  private workScheduled = false;

  private constructor(
    private readonly editor: Editor,
    private readonly blob: ByteSource,
    readonly name: string,
  ) {}

  static async open(file: ByteSource): Promise<Doc> {
    await ensureWasm();
    const editor = new Editor(file.size, CHUNK_SIZE, CHUNK_CAPACITY);
    return new Doc(editor, file, file.name);
  }

  /** Called whenever the document content changes or a pending chunk arrives. */
  onChange(fn: () => void): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }

  private notify(): void {
    for (const fn of this.listeners) fn();
  }

  get lengthBytes(): number {
    return this.editor.len_bytes();
  }
  get lengthBits(): number {
    return this.editor.len_bits();
  }
  get modified(): boolean {
    return this.editor.is_modified();
  }
  get canUndo(): boolean {
    return this.editor.can_undo();
  }
  get canRedo(): boolean {
    return this.editor.can_redo();
  }
  get pieceCount(): number {
    return this.editor.piece_count();
  }

  // ----- templates -----

  template: string | null = null;

  get templateNames(): string[] {
    return this.editor.template_names();
  }

  /**
   * Install a template built from the rule that identified this file, for a
   * format no built-in covers. It describes the format's signature and nothing
   * else, so most of the file stays unannotated; that is the whole of what the
   * rule proves.
   *
   * Returns the name to show for it, or null when the rule pins no fixed bytes
   * to a fixed place, which is the case for a format found by searching.
   */
  async signatureTemplate(id: Identification): Promise<string | null> {
    if (id.source === "") return null;
    // The rule files are static assets, one per format family, a few KiB each.
    // Only the one the identification named is fetched.
    let rules: string;
    try {
      const res = await fetch(`magdir/${encodeURIComponent(id.source)}`);
      if (!res.ok) return null;
      rules = await res.text();
    } catch {
      return null;
    }
    const n = Math.min(IDENTIFY_WINDOW, this.lengthBytes);
    await this.ensureRange(0, n);
    const { bytes, complete } = this.read(0, n);
    if (!complete) return null;
    const name = signatureName(id);
    if (!this.editor.set_magic_template(name, rules, bytes)) return null;
    this.template = name;
    this.notify();
    return name;
  }

  /** Built-in template name matching the file's first bytes, or null. */
  async sniffTemplate(): Promise<string | null> {
    // Enough for a magic number, for the format tag inside a WAVE's first
    // chunk (the only thing that tells a W4V from a WAV), for the PE header
    // a Windows executable puts at an offset of its own choosing, usually
    // 0x80 to 0x100 but fixed nowhere, and for an Anvil region's two
    // tables, which are 8 KiB and whose entries only say anything all
    // together.
    const n = Math.min(8192, this.lengthBytes);
    if (n === 0) return null;
    await this.ensureRange(0, n);
    const head = this.read(0, n).bytes;
    // An OME-Zarr store is a directory, so its identifying record is JSON
    // metadata rather than a byte signature.
    if (isOmeZarrMetadata(this.name, head)) return "omezarr";
    const name = this.editor.sniff_template(head, this.lengthBytes);
    return name === "" ? null : name;
  }

  /**
   * Ask the file(1) rule database what this file is, for the files no template
   * covers. The rules and the engine that runs them outweigh the rest of the
   * editor, so they live in their own wasm module that is fetched on the first
   * call and never at all for a file `sniffTemplate` already answered.
   *
   * The answer is a label, not a layout: it names the format without
   * describing a single field.
   */
  async identify(): Promise<Identification | null> {
    const n = Math.min(IDENTIFY_WINDOW, this.lengthBytes);
    if (n === 0) return null;
    await this.ensureRange(0, n);
    const { bytes, complete } = this.read(0, n);
    // Rules read what they are given, so a short window would answer for a
    // different file. Nothing is better than a wrong name.
    if (!complete) throw new Error("identify: the head of the file did not arrive");
    const json = (await loadMagic()).identify(bytes);
    return json === "" ? null : (JSON.parse(json) as Identification);
  }

  setTemplate(name: string | null): boolean {
    const ok = this.editor.set_template(name ?? "");
    this.template = ok ? name : null;
    this.notify();
    return ok;
  }

  private handleReply<T>(json: string): TemplateReply<T> {
    const r: RawReply<T> = JSON.parse(json);
    if (r.status === "pending") {
      for (const c of r.chunks) this.fetchChunk(c);
      // A contiguous run of missing chunks means something is being read
      // through from front to back, so the chunks after it are what comes
      // next: worth reading in one go. Scattered ones mean fields across the
      // file wanting a byte each, and reading around those would evict what
      // they asked for.
      const first = r.chunks[0];
      if (first !== undefined && r.chunks.every((c, i) => c === first + i)) {
        this.fetchRun(first + r.chunks.length, READ_AHEAD);
      }
      return { status: "pending", reachedBytes: r.reached_bytes };
    }
    if (r.status === "ok" && r.wanted !== undefined && r.wanted.length > 0) {
      // Answered without some previews, so the rows are here and their first
      // bytes are on their way. Asking again once they land fills them in.
      for (const c of r.wanted) this.fetchChunk(c);
      return { status: "ok", node: r.node };
    }
    if (r.status === "working") {
      // Nothing is going to arrive to wake this up, the way a chunk does, so
      // it has to ask itself again. Yielding first lets the page draw what it
      // has and stay usable while the rest is worked out.
      this.scheduleMoreWork();
      return { status: "working", reachedBytes: r.reached_bytes };
    }
    return r;
  }

  /** Carry on with unfinished work after the page has had a chance to draw. */
  private scheduleMoreWork(): void {
    if (this.workScheduled) return;
    this.workScheduled = true;
    setTimeout(() => {
      this.workScheduled = false;
      this.notify();
    }, 0);
  }

  /**
   * What the file holds, in its own terms rather than the template's: the
   * objects of an HDF5 file, each with the path it goes by, what the file
   * calls it, its shape and where its bytes are. Empty for every other format.
   */
  contents(): TemplateReply<Contents> {
    return this.handleReply(this.editor.contents());
  }

  templateNode(path: readonly number[]): TemplateReply<TemplateNode> {
    return this.handleReply(this.editor.template_node(Uint32Array.from(path)));
  }

  templateChildren(path: readonly number[], from: number, to: number): TemplateReply<TemplateNode[]> {
    return this.handleReply(this.editor.template_children(Uint32Array.from(path), from, to));
  }

  /** The whole text of a text field, decoded in the field's own encoding. */
  fieldText(path: readonly number[]): TemplateReply<{ text: string; truncated: boolean }> {
    return this.handleReply(this.editor.field_text(Uint32Array.from(path)));
  }

  /**
   * Every field between two bit offsets, in order. One call covers what is on
   * screen, so the annotation column costs one round trip per view rather than
   * one per field.
   */
  spans(fromBit: number, toBit: number, max: number): TemplateReply<Span[]> {
    return this.handleReply<Span[]>(this.editor.spans(fromBit, toBit, max));
  }

  /**
   * One step of the byte-class scan behind the overview. The node carries
   * everything found so far, so a partial map can be drawn while the rest is
   * read; `done` says when to stop asking. An edit throws the scan away, and
   * the next step starts it over.
   */
  overviewStep(buckets: number): TemplateReply<OverviewState> {
    return this.handleReply<OverviewState>(this.editor.overview_step(buckets));
  }

  /**
   * One step of the scan over a single block of the file, divided into up to
   * `buckets` of its own. Asking about a different block starts a new scan.
   */
  overviewFocusStep(from: number, to: number, buckets: number): TemplateReply<FocusState> {
    return this.handleReply<FocusState>(this.editor.overview_focus_step(from, to, buckets));
  }

  /** What is wrong with what the search bar holds, or "" when nothing is.
   *  `typing` suppresses the one complaint that is not a mistake yet: half a
   *  hex byte, which every valid needle passes through on the way in. */
  checkNeedle(kind: NeedleKind, text: string, typing: boolean): string {
    return this.editor.check_needle(kind, text, typing);
  }

  /**
   * One window of a search. The reply is the usual tri-state: a step, or
   * pending while the bytes it needs are fetched. The caller loops.
   */
  searchStep(needle: Query, from: number): TemplateReply<SearchStep> {
    return this.handleReply<SearchStep>(
      this.editor.search_step(needle.kind, needle.text, needle.fold, needle.backward, from),
    );
  }

  /** Put bytes where a match was found. */
  replaceAt(at: number, len: number, bytes: Uint8Array): void {
    this.editor.replace_at(at, len, bytes);
    this.notify();
  }

  /** Fold the edits that follow into one undo step. */
  beginBatch(): void {
    this.editor.begin_batch();
  }

  endBatch(): void {
    this.editor.end_batch();
  }

  /**
   * Which fields settled the shape of the one at `path`, and where this one
   * points if it holds an offset. Usually empty: most fields are placed and
   * sized by the template outright.
   */
  origins(path: readonly number[]): TemplateReply<Origin[]> {
    return this.handleReply<Origin[]>(this.editor.origins(Uint32Array.from(path)));
  }

  /** What the type at `path` permits: enum values, magic bytes, flag bits. */
  /** `atBits` is where the cursor is; only a block of packed weights uses it,
   *  to say which weight the reader is standing on. */
  typeInfo(path: readonly number[], atBits = -1): TemplateReply<TypeInfo> {
    return this.handleReply<TypeInfo>(this.editor.type_info(Uint32Array.from(path), atBits));
  }

  /**
   * What tool produced this file, from the Detect It Easy signature rules.
   *
   * Which rules are worth fetching depends on what the file is, and both
   * bundles are only worth fetching for the files they describe. A DOS
   * executable is asked about at its entry point; a .COM has no header to say
   * it is one, so the format's own limit stands in: it is loaded whole into a
   * single 64 KiB segment, and anything larger is not one.
   */
  async detectTools(identified: boolean): Promise<ToolMatch[]> {
    const n = Math.min(IDENTIFY_WINDOW, this.lengthBytes);
    if (n === 0) return [];
    await this.ensureRange(0, n);
    const first = this.read(0, n);
    if (!first.complete) return [];
    let bytes = first.bytes;
    const bundles: string[] = [];
    const mz = bytes[0] === 0x4d && bytes[1] === 0x5a;
    if (mz) {
      // Both start MZ. Which rules apply is decided by whether there is a PE
      // header where the DOS header points, the same question the built-in
      // sniffer asks.
      const byteAt = (i: number): number => bytes[i] ?? 0;
      const at = byteAt(0x3c) + (byteAt(0x3d) << 8) + (byteAt(0x3e) << 16) + byteAt(0x3f) * 0x1000000;
      const pe = at + 4 <= bytes.length && byteAt(at) === 0x50 && byteAt(at + 1) === 0x45 && byteAt(at + 2) === 0 && byteAt(at + 3) === 0;
      bundles.push(pe ? "pe.sig" : "msdos.sig");
      // A DOS program is asked about further in than the 64 KiB an unknown
      // format is identified from. A Windows one is not: its rules read the
      // section table, which is in the header either way.
      const want = Math.min(DOS_WINDOW, this.lengthBytes);
      if (!pe && want > n) {
        await this.ensureRange(0, want);
        const wider = this.read(0, want);
        if (wider.complete) bytes = wider.bytes;
      }
    }
    // A .COM is bytes with no header at all, so nothing but its size and its
    // name suggest one. Asking for every unknown small file would be worse.
    if (!mz && this.lengthBytes <= COM_LIMIT && (!identified || this.name.toLowerCase().endsWith(".com"))) {
      bundles.push("com.sig");
    }
    const out: ToolMatch[] = [];
    for (const bundle of bundles) {
      const rules = await fetchRules(bundle);
      if (rules === null) continue;
      out.push(...(JSON.parse(this.editor.detect_tools(rules, bytes)) as ToolMatch[]));
    }
    return out;
  }

  /** Path of the deepest template field covering `bitOffset`. */
  locate(bitOffset: number): TemplateReply<number[]> {
    return this.handleReply<number[]>(this.editor.locate(bitOffset));
  }

  /**
   * Write `text` into the field at `path`, encoded as that field's type. The
   * core writes exactly the field's own bits, so nothing after it shifts.
   * A "pending" reply means the field's position could not be worked out yet;
   * the chunks are on their way and the caller should ask again.
   */
  writeNode(path: readonly number[], text: string): TemplateReply<WrittenRange> {
    const r = this.handleReply<WrittenRange>(this.editor.write_node(Uint32Array.from(path), text));
    if (r.status === "ok") this.notify();
    return r;
  }

  /** Synchronous read. Missing chunks are zero and fetched in the background. */
  read(at: number, len: number): ReadResult {
    const bytes = new Uint8Array(len);
    const missing = this.editor.read_bytes(at, bytes);
    for (const chunk of missing) this.fetchChunk(chunk);
    return { bytes, complete: missing.length === 0 };
  }

  /** Synchronous read of `nBits` starting at any bit, packed MSB first. */
  readBits(atBit: number, nBits: number): ReadResult {
    const bytes = new Uint8Array(Math.ceil(nBits / 8));
    const missing = this.editor.read_bits(atBit, nBits, bytes);
    for (const chunk of missing) this.fetchChunk(chunk);
    return { bytes, complete: missing.length === 0 };
  }

  /**
   * Read `count` chunks from `from` onwards in a single go, skipping any that
   * are already here or already on their way. One read of three megabytes
   * costs about what one read of sixty-four kilobytes costs, and the file is
   * being walked forwards, so this is most of what makes a large file open in
   * seconds rather than minutes.
   */
  private fetchRun(from: number, count: number): void {
    const total = Math.ceil(this.blob.size / CHUNK_SIZE);
    let start = from;
    while (start < from + count && start < total && (this.inflight.has(start) || this.editor.has_chunk(start))) {
      start += 1;
    }
    let end = start;
    while (end < from + count && end < total && !this.inflight.has(end) && !this.editor.has_chunk(end)) {
      end += 1;
    }
    if (end <= start) return;
    for (let c = start; c < end; c++) this.inflight.add(c);
    const at = start * CHUNK_SIZE;
    void this.blob
      .slice(at, Math.min(end * CHUNK_SIZE, this.blob.size))
      .arrayBuffer()
      .then((buf) => {
        const bytes = new Uint8Array(buf);
        for (let c = start; c < end; c++) {
          const off = (c - start) * CHUNK_SIZE;
          if (off >= bytes.length) break;
          this.editor.feed_chunk(c, bytes.subarray(off, Math.min(off + CHUNK_SIZE, bytes.length)));
        }
      })
      .finally(() => {
        for (let c = start; c < end; c++) this.inflight.delete(c);
        this.notify();
      });
  }

  private fetchChunk(chunk: number): void {
    // Reading ahead can run off the end, and a chunk past the end is not a
    // chunk: feeding an empty one would look like bytes that are all zero.
    if (chunk * CHUNK_SIZE >= this.blob.size) return;
    if (this.inflight.has(chunk)) return;
    this.inflight.add(chunk);
    const start = chunk * CHUNK_SIZE;
    void this.blob
      .slice(start, Math.min(start + CHUNK_SIZE, this.blob.size))
      .arrayBuffer()
      .then((buf) => {
        this.editor.feed_chunk(chunk, new Uint8Array(buf));
      })
      .finally(() => {
        this.inflight.delete(chunk);
        this.notify();
      });
  }

  /** Resolve once every chunk covering [at, at+len) is loaded. */
  async ensureRange(at: number, len: number): Promise<void> {
    const first = Math.floor(at / CHUNK_SIZE);
    const last = Math.floor((at + len - 1) / CHUNK_SIZE);
    const waits: Promise<void>[] = [];
    for (let c = first; c <= last; c++) {
      if (!this.editor.has_chunk(c)) {
        waits.push(this.loadChunk(c).catch((e: unknown) => {
          throw new ReadFailure(c * CHUNK_SIZE, Math.min(CHUNK_SIZE, this.blob.size - c * CHUNK_SIZE), e);
        }));
      }
    }
    await Promise.all(waits);
  }

  private loadChunk(chunk: number): Promise<void> {
    const start = chunk * CHUNK_SIZE;
    return Promise.resolve(this.blob.slice(start, Math.min(start + CHUNK_SIZE, this.blob.size)).arrayBuffer()).then(
      (buf) => {
        this.editor.feed_chunk(chunk, new Uint8Array(buf));
      },
    );
  }

  /**
   * Build the saved file as a Blob of lazy parts. Unchanged stretches of the
   * original are referenced, not copied, so this works for any file size.
   */
  async buildOutput(): Promise<Blob> {
    const plan = this.editor.save_plan();
    const add = this.editor.add_bytes();
    const parts: BlobPart[] = [];
    for (let i = 0; i < plan.length; i += 4) {
      const kind = plan[i] ?? 0;
      const docOff = plan[i + 1] ?? 0;
      const srcOff = plan[i + 2] ?? 0;
      const len = plan[i + 3] ?? 0;
      if (kind === 0) {
        const part = this.blob.slice(srcOff, srcOff + len);
        if (part instanceof Blob) {
          parts.push(part); // lazy reference, nothing copied
        } else {
          // Non-Blob sources (dev synthetic files) must be read; keep pieces bounded.
          const STEP = 16 * 1024 * 1024;
          for (let o = 0; o < len; o += STEP) {
            parts.push(await this.blob.slice(srcOff + o, srcOff + Math.min(len, o + STEP)).arrayBuffer());
          }
        }
      } else if (kind === 1) {
        parts.push(add.slice(srcOff, srcOff + len));
      } else {
        // Bit-unaligned stretch: read it through the piece table in chunks.
        const STEP = 4 * 1024 * 1024;
        for (let o = 0; o < len; o += STEP) {
          const n = Math.min(STEP, len - o);
          await this.ensureRange(docOff + o, n);
          let { bytes, complete } = this.read(docOff + o, n);
          if (!complete) {
            // Chunks were evicted between load and read; one retry covers it.
            await this.ensureRange(docOff + o, n);
            ({ bytes, complete } = this.read(docOff + o, n));
          }
          if (!complete) throw new ReadFailure(docOff + o, n, "Chunks were evicted before they could be used.");
          parts.push(new Uint8Array(bytes) as Uint8Array<ArrayBuffer>);
        }
      }
    }
    return new Blob(parts, { type: "application/octet-stream" });
  }

  overwrite(at: number, data: Uint8Array): void {
    this.editor.overwrite_bytes(at, data);
    this.notify();
  }
  /** Overwrite that joins the previous edit's undo step. */
  amendOverwrite(at: number, data: Uint8Array): void {
    this.editor.amend_overwrite_bytes(at, data);
    this.notify();
  }
  insert(at: number, data: Uint8Array): void {
    this.editor.insert_bytes(at, data);
    this.notify();
  }
  delete(at: number, n: number): void {
    this.editor.delete_bytes(at, n);
    this.notify();
  }
  overwriteBits(atBit: number, data: Uint8Array, n: number): void {
    this.editor.overwrite_bits(atBit, data, n);
    this.notify();
  }
  insertBits(atBit: number, data: Uint8Array, n: number): void {
    this.editor.insert_bits(atBit, data, n);
    this.notify();
  }
  deleteBits(atBit: number, n: number): void {
    this.editor.delete_bits(atBit, n);
    this.notify();
  }
  undo(): void {
    if (this.editor.undo()) this.notify();
  }
  redo(): void {
    if (this.editor.redo()) this.notify();
  }
}
