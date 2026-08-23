// Document facade: owns the wasm Editor and streams chunks in from a File/Blob.
// Nothing here ever reads the whole file; only the chunks the view asks for.

import init, { Editor } from "./pkg/qubero_wasm.js";

const CHUNK_SIZE = 64 * 1024;
const CHUNK_CAPACITY = 512; // 32 MiB resident at most

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
  readonly kind: "uint" | "int" | "float" | "bytes" | "str" | "magic" | "enum" | "composite";
  readonly ok: boolean;
  readonly child_count: number;
  readonly composite: boolean;
  /** True when `writeNode` accepts typed text for this field. */
  readonly editable: boolean;
};

/** The bit range a successful `writeNode` replaced. */
export type WrittenRange = { readonly offset_bits: number; readonly size_bits: number };

export type TemplateReply<T> =
  | { readonly status: "ok"; readonly node: T }
  | { readonly status: "pending" }
  | { readonly status: "error"; readonly message: string };

type RawReply<T> =
  | { status: "ok"; node: T }
  | { status: "pending"; chunks: number[] }
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

/** An offset as `0x1F`, or `0x1F+3b` when it falls inside a byte. */
export function formatOffset(bits: number): string {
  const byte = Math.floor(bits / 8);
  const rem = bits % 8;
  return `0x${byte.toString(16).toUpperCase()}${rem === 0 ? "" : `+${rem}b`}`;
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

  /** Built-in template name matching the file's first bytes, or null. */
  async sniffTemplate(): Promise<string | null> {
    const n = Math.min(16, this.lengthBytes);
    if (n === 0) return null;
    await this.ensureRange(0, n);
    const name = this.editor.sniff_template(this.read(0, n).bytes);
    return name === "" ? null : name;
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
      return { status: "pending" };
    }
    return r;
  }

  templateNode(path: readonly number[]): TemplateReply<TemplateNode> {
    return this.handleReply(this.editor.template_node(Uint32Array.from(path)));
  }

  templateChildren(path: readonly number[], from: number, to: number): TemplateReply<TemplateNode[]> {
    return this.handleReply(this.editor.template_children(Uint32Array.from(path), from, to));
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

  private fetchChunk(chunk: number): void {
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
