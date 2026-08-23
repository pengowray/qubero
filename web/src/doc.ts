// Document facade: owns the wasm Editor and streams chunks in from a File/Blob.
// Nothing here ever reads the whole file; only the chunks the view asks for.

import init, { Editor } from "./pkg/qubero_wasm.js";

const CHUNK_SIZE = 64 * 1024;
const CHUNK_CAPACITY = 512; // 32 MiB resident at most

/** The subset of Blob we need; lets tests and dev tooling supply synthetic files. */
export type ByteSource = {
  readonly size: number;
  readonly name: string;
  slice(start: number, end: number): { arrayBuffer(): Promise<ArrayBuffer> };
};

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
