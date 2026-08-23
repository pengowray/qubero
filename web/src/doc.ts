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
  readonly kind: "uint" | "int" | "float" | "bytes" | "str" | "magic" | "composite";
  readonly ok: boolean;
  readonly child_count: number;
  readonly composite: boolean;
};

export type TemplateReply<T> =
  | { readonly status: "ok"; readonly node: T }
  | { readonly status: "pending" }
  | { readonly status: "error"; readonly message: string };

type RawReply<T> =
  | { status: "ok"; node: T }
  | { status: "pending"; chunks: number[] }
  | { status: "error"; message: string };

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

  /** Synchronous read. Missing chunks are zero and fetched in the background. */
  read(at: number, len: number): ReadResult {
    const bytes = new Uint8Array(len);
    const missing = this.editor.read_bytes(at, bytes);
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
      if (!this.editor.has_chunk(c)) waits.push(this.loadChunk(c));
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
          const { bytes, complete } = this.read(docOff + o, n);
          if (!complete) throw new Error("Some of the file could not be read.");
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
