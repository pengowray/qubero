/**
 * A folder opened as one file: the files in it written into a ZIP that stores
 * them as they are, built in the browser.
 *
 * Qubero reads one file at a time, and some formats are folders. An ADIOS2 BP5
 * dataset is `md.idx`, `md.0`, `mmd.0` and `data.0`, and `md.0` cannot be read
 * without `mmd.0` beside it; a Zarr store is a tree of chunks. Stored in one
 * archive, every file is a run of bytes at a known place in one address space,
 * so a pointer from one file into another is an offset like any other and the
 * core reads the dataset as one tree. See DESIGN.md, "A folder opened as
 * one file".
 *
 * Nothing is copied. The archive is a `Blob` of the headers this writes and the
 * picked files themselves, so its bytes are read from the files when the
 * editor asks for them, the same as for a file opened on its own. The one pass
 * over every byte is the CRC-32 each entry's header carries.
 */

/** A file of the folder, by its path inside the folder's parent: the folder's
 *  own name first, then the path below it, separated by `/`. */
export type FolderFile = {
  readonly path: string;
  readonly file: Blob;
  /** When the file last changed, in milliseconds since 1970, for the entry's
   *  date. */
  readonly modified?: number;
};

/** Names a format is recognised by, which go first so they are inside the
 *  window recognition reads: a BP5 index and format list, and Zarr's metadata. */
const FIRST = ["md.idx", "mmd.0", "md.0", ".zgroup", ".zattrs", ".zarray", "zarr.json"];

/** The last part of a path. */
export function leafOf(path: string): string {
  const cut = path.lastIndexOf("/");
  return cut < 0 ? path : path.slice(cut + 1);
}

/**
 * The order the files are written in: the ones a format names itself by
 * first, in the order of `FIRST`, then the rest in path order with data files
 * (`data.0` and the like) last, since those can be most of the folder and
 * nothing is recognised by them.
 */
export function orderForArchive(files: readonly FolderFile[]): FolderFile[] {
  const rank = (f: FolderFile): number => {
    const leaf = leafOf(f.path);
    const first = FIRST.indexOf(leaf);
    if (first >= 0) return first;
    return /^data\.\d+$/.test(leaf) ? FIRST.length + 1 : FIRST.length;
  };
  return [...files].sort((a, b) => rank(a) - rank(b) || (a.path < b.path ? -1 : a.path > b.path ? 1 : 0));
}

/** CRC-32 tables for slicing by eight: table `k` is the CRC of a byte followed
 *  by `k` zero bytes. */
const TABLES: Uint32Array[] = (() => {
  const t0 = new Uint32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    t0[n] = c >>> 0;
  }
  const tables = [t0];
  for (let k = 1; k < 8; k++) {
    const prev = tables[k - 1] as Uint32Array;
    const t = new Uint32Array(256);
    for (let n = 0; n < 256; n++) t[n] = ((prev[n] as number) >>> 8) ^ (t0[(prev[n] as number) & 0xff] as number);
    tables.push(t);
  }
  return tables;
})();

/** Carry a CRC-32 on over more bytes. Start from 0; the result is the CRC of
 *  everything handed in so far. */
export function crc32Update(crc: number, bytes: Uint8Array): number {
  const [t0, t1, t2, t3, t4, t5, t6, t7] = TABLES as [Uint32Array, Uint32Array, Uint32Array, Uint32Array, Uint32Array, Uint32Array, Uint32Array, Uint32Array];
  let c = ~crc >>> 0;
  let i = 0;
  const n = bytes.length;
  for (; i + 8 <= n; i += 8) {
    c ^= (bytes[i] as number) | ((bytes[i + 1] as number) << 8) | ((bytes[i + 2] as number) << 16) | ((bytes[i + 3] as number) << 24);
    c =
      (t7[c & 0xff] as number) ^
      (t6[(c >>> 8) & 0xff] as number) ^
      (t5[(c >>> 16) & 0xff] as number) ^
      (t4[c >>> 24] as number) ^
      (t3[bytes[i + 4] as number] as number) ^
      (t2[bytes[i + 5] as number] as number) ^
      (t1[bytes[i + 6] as number] as number) ^
      (t0[bytes[i + 7] as number] as number);
  }
  for (; i < n; i++) c = (t0[(c ^ (bytes[i] as number)) & 0xff] as number) ^ (c >>> 8);
  return ~c >>> 0;
}

/** The archive was not finished: the signal said stop. */
export class Stopped extends Error {
  constructor() {
    super("stopped");
  }
}

/** The CRC-32 of a file's bytes, read a piece at a time. `read` is told how
 *  many more bytes have been read after each piece. */
async function crcOf(file: Blob, read: (bytes: number) => void, signal?: AbortSignal): Promise<number> {
  const reader = file.stream().getReader();
  let crc = 0;
  for (;;) {
    if (signal?.aborted === true) {
      await reader.cancel();
      throw new Stopped();
    }
    const { done, value } = await reader.read();
    if (done) return crc;
    crc = crc32Update(crc, value);
    read(value.length);
  }
}

/** Bytes written little-endian, a field at a time. */
class Writer {
  private readonly parts: number[] = [];
  u16(v: number): this {
    this.parts.push(v & 0xff, (v >>> 8) & 0xff);
    return this;
  }
  u32(v: number): this {
    return this.u16(v & 0xffff).u16(Math.floor(v / 0x10000) & 0xffff);
  }
  u64(v: number): this {
    return this.u32(v % 0x1_0000_0000).u32(Math.floor(v / 0x1_0000_0000));
  }
  bytes(b: Uint8Array): this {
    for (const x of b) this.parts.push(x);
    return this;
  }
  get length(): number {
    return this.parts.length;
  }
  done(): Uint8Array {
    return Uint8Array.from(this.parts);
  }
}

/** The largest number a 32-bit field holds before the ZIP64 extra has to. */
const MAX32 = 0xffff_ffff;

/** An MS-DOS date and time, as a ZIP entry writes when a file was changed. */
function dosTime(ms: number | undefined): { time: number; date: number } {
  const d = new Date(ms ?? Date.UTC(1980, 0, 1));
  if (d.getFullYear() < 1980) return { time: 0, date: 0x21 };
  return {
    time: (d.getHours() << 11) | (d.getMinutes() << 5) | Math.floor(d.getSeconds() / 2),
    date: ((d.getFullYear() - 1980) << 9) | ((d.getMonth() + 1) << 5) | d.getDate(),
  };
}

/** What is known of the archive as it is written: bytes checked so far of
 *  how many. */
export type ArchiveProgress = { readonly done: number; readonly total: number };

/**
 * The files written as a ZIP that stores each one as it is, in the order
 * given, with a CRC-32 read from each and ZIP64 records where a size or an
 * offset is past what 32 bits hold.
 */
export async function storedZip(files: readonly FolderFile[], progress?: (p: ArchiveProgress) => void, signal?: AbortSignal): Promise<Blob> {
  const total = files.reduce((n, f) => n + f.file.size, 0);
  let done = 0;
  const parts: BlobPart[] = [];
  const central = new Writer();
  let at = 0;
  const utf8 = new TextEncoder();
  for (const f of files) {
    const crc = await crcOf(
      f.file,
      (n) => {
        done += n;
        progress?.({ done, total });
      },
      signal,
    );
    const name = utf8.encode(f.path);
    const size = f.file.size;
    const { time, date } = dosTime(f.modified);
    const big = size >= MAX32;
    const far = at >= MAX32;
    // The local header's ZIP64 extra holds both sizes, always both.
    const localExtra = big ? new Writer().u16(1).u16(16).u64(size).u64(size).done() : new Uint8Array(0);
    const needed = big || far ? 45 : 20;
    const local = new Writer()
      .u32(0x04034b50)
      .u16(needed)
      .u16(0x0800)
      .u16(0)
      .u16(time)
      .u16(date)
      .u32(crc)
      .u32(big ? MAX32 : size)
      .u32(big ? MAX32 : size)
      .u16(name.length)
      .u16(localExtra.length)
      .bytes(name)
      .bytes(localExtra)
      .done();
    // The central record's extra holds only what did not fit, in this order.
    const centralExtra = new Writer();
    if (big || far) {
      centralExtra.u16(1).u16((big ? 16 : 0) + (far ? 8 : 0));
      if (big) centralExtra.u64(size).u64(size);
      if (far) centralExtra.u64(at);
    }
    const extra = centralExtra.done();
    central
      .u32(0x02014b50)
      .u16(needed)
      .u16(needed)
      .u16(0x0800)
      .u16(0)
      .u16(time)
      .u16(date)
      .u32(crc)
      .u32(big ? MAX32 : size)
      .u32(big ? MAX32 : size)
      .u16(name.length)
      .u16(extra.length)
      .u16(0)
      .u16(0)
      .u16(0)
      .u32(0)
      .u32(far ? MAX32 : at)
      .bytes(name)
      .bytes(extra);
    parts.push(local as BlobPart, f.file);
    at += local.length + size;
  }
  const directory = central.done();
  const entries = files.length;
  const zip64 = entries > 0xffff || at >= MAX32 || directory.length >= MAX32;
  const end = new Writer();
  if (zip64) {
    const endAt = at + directory.length;
    end
      .u32(0x06064b50)
      .u64(44)
      .u16(45)
      .u16(45)
      .u32(0)
      .u32(0)
      .u64(entries)
      .u64(entries)
      .u64(directory.length)
      .u64(at)
      .u32(0x07064b50)
      .u32(0)
      .u64(endAt)
      .u32(1);
  }
  end
    .u32(0x06054b50)
    .u16(0)
    .u16(0)
    .u16(Math.min(entries, 0xffff))
    .u16(Math.min(entries, 0xffff))
    .u32(zip64 ? MAX32 : directory.length)
    .u32(zip64 ? MAX32 : at)
    .u16(0);
  parts.push(directory as BlobPart, end.done() as BlobPart);
  return new Blob(parts, { type: "application/zip" });
}

/** A folder entry the browser hands a drop, read to its files. `under` is the
 *  path of the entry's parent. */
async function readEntry(entry: FileSystemEntry, into: FolderFile[], seen: (count: number) => void): Promise<void> {
  const path = entry.fullPath.replace(/^\/+/, "");
  if (entry.isFile) {
    const file = await new Promise<File>((resolve, reject) => (entry as FileSystemFileEntry).file(resolve, reject));
    into.push({ path, file, modified: file.lastModified });
    seen(into.length);
    return;
  }
  if (!entry.isDirectory) return;
  const reader = (entry as FileSystemDirectoryEntry).createReader();
  // A directory reader hands its entries over in batches and says it is done
  // with an empty one.
  for (;;) {
    const batch = await new Promise<FileSystemEntry[]>((resolve, reject) => reader.readEntries(resolve, reject));
    if (batch.length === 0) break;
    for (const child of batch) await readEntry(child, into, seen);
  }
}

/** What was dropped, when it was more than one plain file: every file under
 *  every item, and what to call the whole. */
export type Dropped = { readonly files: FolderFile[]; readonly name: string; readonly folder: string | null };

/** Whether a drop holds a folder or several items, which open as one archive
 *  rather than as the first file. */
export function dropIsFolder(items: DataTransferItemList): boolean {
  let n = 0;
  for (const item of items) {
    if (item.kind !== "file") continue;
    n++;
    if (item.webkitGetAsEntry()?.isDirectory === true) return true;
  }
  return n > 1;
}

/**
 * Every file under the dropped items. The entries have to be taken from the
 * drop before anything is awaited, since the browser empties the list once the
 * event is over. Null when the browser gives no entries to read.
 */
export async function readDrop(items: DataTransferItemList, seen: (count: number) => void): Promise<Dropped | null> {
  const entries: FileSystemEntry[] = [];
  // An item with no entry behind it is still a file, when it is one: a drop
  // made by a script, or by a browser that does not hand out entries.
  const loose: FolderFile[] = [];
  for (const item of items) {
    if (item.kind !== "file") continue;
    const entry = item.webkitGetAsEntry();
    if (entry !== null) {
      entries.push(entry);
      continue;
    }
    const file = item.getAsFile();
    if (file === null) return null;
    loose.push({ path: file.name, file, modified: file.lastModified });
  }
  const files: FolderFile[] = [...loose];
  for (const entry of entries) await readEntry(entry, files, seen);
  const only = entries.length === 1 && loose.length === 0 ? entries[0] : undefined;
  const folder = only !== undefined && only.isDirectory ? only.name : null;
  return { files, name: folder === null ? "files.zip" : `${folder}.zip`, folder };
}

/** The files a folder picker gave, each under the folder's name as the picker
 *  says it. */
export function readPicked(list: FileList): Dropped {
  const files: FolderFile[] = Array.from(list).map((file) => ({
    path: (file.webkitRelativePath === "" ? file.name : file.webkitRelativePath).replace(/\\/g, "/"),
    file,
    modified: file.lastModified,
  }));
  const top = files[0]?.path.split("/")[0] ?? "";
  const folder = top !== "" && files.every((f) => f.path.startsWith(`${top}/`)) ? top : null;
  return { files, name: folder === null ? "files.zip" : `${folder}.zip`, folder };
}

/** What a folder is missing of a BP5 dataset, when it holds one: the dataset's
 *  directory holds `md.0` or `md.idx`, and lacks `mmd.0` or `data.0`. Empty for
 *  a folder that is not one, or has everything. */
export function missingFromDataset(files: readonly FolderFile[]): ("mmd.0" | "data.0")[] {
  const dirOf = (p: string): string => p.slice(0, Math.max(0, p.lastIndexOf("/")));
  const anchor = files.find((f) => leafOf(f.path) === "md.idx" || leafOf(f.path) === "md.0");
  if (anchor === undefined) return [];
  const dir = dirOf(anchor.path);
  const has = (leaf: string): boolean => files.some((f) => dirOf(f.path) === dir && leafOf(f.path) === leaf);
  const out: ("mmd.0" | "data.0")[] = [];
  if (!has("mmd.0")) out.push("mmd.0");
  if (!has("data.0")) out.push("data.0");
  return out;
}
