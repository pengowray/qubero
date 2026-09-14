/**
 * The CRC-32s of a large folder opened as one ZIP, taken after it opens.
 *
 * A folder of more than `CRC_AT_OPEN_MAX_BYTES` opens with nought in every
 * CRC-32 field of the archive built from it, since reading every byte first is
 * a wait for a number the reader rarely looks at. The sums are still owed to
 * the file Save as writes, so they are taken in the background: in a worker
 * where the browser has one, and a slice at a time in idle moments where it has
 * not. Save as waits for whatever is left and writes the archive with the sums
 * in; the open document keeps its noughts, and the inspector says what they
 * are (`ArchiveSums.slotAt`).
 */

import { crc32Update } from "./crc32.ts";
import type { SumMessage, SumRequest } from "./crcworker.ts";
import type { BuiltZip } from "./folderzip.ts";

/** How much of a file one idle moment reads, where there is no worker. */
const IDLE_SLICE = 4 * 1024 * 1024;

/** The sums of `files`, taken once, in the background. */
export class SumJob {
  readonly total: number;
  private readBytes = 0;
  private readonly crcs: (number | null)[];
  private started = false;
  private stopped = false;
  private failure: Error | null = null;
  private worker: Worker | null = null;
  private readonly listeners = new Set<() => void>();
  private readonly waiting: { resolve: (crcs: number[]) => void; reject: (e: Error) => void }[] = [];
  private readonly files: readonly Blob[];

  constructor(files: readonly Blob[]) {
    this.files = files;
    this.total = files.reduce((n, f) => n + f.size, 0);
    this.crcs = files.map(() => null);
  }

  /** How many bytes have been summed so far. */
  get read(): number {
    return this.readBytes;
  }

  /** The sum of file `i`, once it is known. */
  sum(i: number): number | null {
    return this.crcs[i] ?? null;
  }

  get finished(): boolean {
    return this.crcs.every((c) => c !== null);
  }

  /** Called whenever more is known. Returns the way to stop being told. */
  onChange(listener: () => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  /** Start taking the sums, if they are not already being taken. */
  start(): void {
    if (this.started || this.stopped) return;
    this.started = true;
    if (this.finished) return this.settle();
    try {
      this.worker = new Worker(new URL("./crcworker.ts", import.meta.url), { type: "module" });
    } catch {
      this.worker = null;
    }
    if (this.worker === null) {
      void this.sumWhenIdle();
      return;
    }
    this.worker.onmessage = (event: MessageEvent<SumMessage>) => this.take(event.data);
    this.worker.onerror = (event) => {
      event.preventDefault();
      // A worker that will not load is not a reason to go without the sums.
      this.worker?.terminate();
      this.worker = null;
      this.readBytes = this.crcs.reduce<number>((n, c, i) => n + (c === null ? 0 : (this.files[i] as Blob).size), 0);
      void this.sumWhenIdle();
    };
    this.worker.postMessage({ files: this.files } satisfies SumRequest);
  }

  /** Every sum, taking them now if nothing has started them. */
  whenDone(): Promise<number[]> {
    if (this.failure !== null) return Promise.reject(this.failure);
    if (this.finished) return Promise.resolve(this.crcs as number[]);
    const done = new Promise<number[]>((resolve, reject) => this.waiting.push({ resolve, reject }));
    this.start();
    return done;
  }

  /** Give up on the sums: the document they were for has closed. */
  stop(): void {
    this.stopped = true;
    this.worker?.terminate();
    this.worker = null;
    this.listeners.clear();
  }

  private take(message: SumMessage): void {
    if (message.kind === "read") this.readBytes += message.bytes;
    else if (message.kind === "sum") this.crcs[message.index] = message.crc;
    else this.fail(new Error(message.message));
    this.changed();
    if (this.finished) this.settle();
  }

  private changed(): void {
    for (const listener of this.listeners) listener();
  }

  private settle(): void {
    this.worker?.terminate();
    this.worker = null;
    for (const w of this.waiting.splice(0)) w.resolve(this.crcs as number[]);
  }

  private fail(error: Error): void {
    this.failure = error;
    for (const w of this.waiting.splice(0)) w.reject(error);
  }

  /** The same on the main thread, a slice of a file per idle moment. */
  private async sumWhenIdle(): Promise<void> {
    const idle = (): Promise<void> =>
      new Promise((resolve) => {
        const request = (globalThis as { requestIdleCallback?: (cb: () => void) => void }).requestIdleCallback;
        if (request !== undefined) request(() => resolve());
        else setTimeout(resolve, 0);
      });
    try {
      for (const [index, file] of this.files.entries()) {
        if (this.crcs[index] !== null) continue;
        let crc = 0;
        for (let at = 0; at < file.size; at += IDLE_SLICE) {
          if (this.stopped) return;
          await idle();
          const piece = new Uint8Array(await file.slice(at, Math.min(file.size, at + IDLE_SLICE)).arrayBuffer());
          crc = crc32Update(crc, piece);
          this.readBytes += piece.length;
          this.changed();
        }
        this.crcs[index] = crc;
        this.changed();
      }
      this.settle();
    } catch (error) {
      this.fail(error instanceof Error ? error : new Error(String(error)));
    }
  }
}

/** A built archive whose sums are being taken: where its CRC-32 fields are,
 *  and the archive with them filled in. */
export class ArchiveSums {
  /** Which entry's field starts at each byte of the archive. */
  private readonly fields = new Map<number, number>();
  readonly built: BuiltZip;
  readonly job: SumJob;

  constructor(built: BuiltZip, job: SumJob) {
    this.built = built;
    this.job = job;
    built.sumAt.forEach((at, i) => {
      this.fields.set(at.local, i);
      this.fields.set(at.central, i);
    });
  }

  /** The CRC-32 field starting at byte `at` of the archive as built, and its
   *  sum once that is known. Null for any other byte. */
  slotAt(at: number): { readonly crc: number | null } | null {
    const i = this.fields.get(at);
    return i === undefined ? null : { crc: this.job.sum(i) };
  }

  /** The archive with every sum written in, once they are all known. */
  async summed(): Promise<Blob> {
    return this.built.withSums(await this.job.whenDone());
  }
}
