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
 * over every byte is the CRC-32 each entry's header carries, which is taken
 * before the archive opens for a folder of up to `CRC_AT_OPEN_MAX_BYTES`, and
 * after it for a larger one: see `sumjob.ts`.
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
 * first, shallowest first and then in the order of `FIRST`, so a Zarr store's
 * root metadata leads the metadata of the arrays under it; then the rest in
 * path order with data files (`data.0` and the like) last, since those can be
 * most of the folder and nothing is recognised by them.
 */
export function orderForArchive(files: readonly FolderFile[]): FolderFile[] {
  const rank = (f: FolderFile): readonly [number, number, number] => {
    const leaf = leafOf(f.path);
    const first = FIRST.indexOf(leaf);
    if (first >= 0) return [0, f.path.split("/").length, first];
    return [/^data\.\d+$/.test(leaf) ? 2 : 1, 0, 0];
  };
  const byPath = (a: FolderFile, b: FolderFile): number => (a.path < b.path ? -1 : a.path > b.path ? 1 : 0);
  return [...files].sort((a, b) => {
    const [x, y] = [rank(a), rank(b)];
    return x[0] - y[0] || x[1] - y[1] || x[2] - y[2] || byPath(a, b);
  });
}

import { crcOf } from "./crc32.ts";

export { crc32Update, Stopped } from "./crc32.ts";

/**
 * The most a folder may hold for its CRC-32s to be read before it opens. Past
 * this the archive opens at once with nought in every CRC-32 field, the sums
 * are taken in the background, and Save as writes them: a gigabyte read before
 * the first byte shows is a wait for a number most readers never look at.
 */
export const CRC_AT_OPEN_MAX_BYTES = 50 * 1024 * 1024;

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

/** An archive built from a folder: what the editor reads, and where its sums
 *  go. */
export type BuiltZip = {
  /** The archive. Its CRC-32 fields hold each file's sum when the sums were
   *  read as it was written, and nought when they were not. */
  readonly blob: Blob;
  readonly files: readonly FolderFile[];
  /** Whether the CRC-32 fields hold the sums. */
  readonly summed: boolean;
  /** Where each entry's two CRC-32 fields are, in bytes of the archive: the
   *  local header's and the central directory's. */
  readonly sumAt: readonly { readonly local: number; readonly central: number }[];
  /** The same archive with these sums, one per file, written into both of
   *  each entry's fields. Nothing is copied but the headers. */
  withSums(crcs: readonly number[]): Blob;
};

/** Where a local header keeps its CRC-32, and where a central directory
 *  record does. */
const LOCAL_CRC = 14;
const CENTRAL_CRC = 16;

/** `bytes` with a copy of the four at `at` set to `crc`, little-endian. */
function withCrc(bytes: Uint8Array, at: number, crc: number): void {
  new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength).setUint32(at, crc >>> 0, true);
}

/**
 * The files written as a ZIP that stores each one as it is, in the order
 * given, with ZIP64 records where a size or an offset is past what 32 bits
 * hold. With `sums`, a CRC-32 read from each file first; without, nought in
 * every CRC-32 field, for `withSums` to fill in later.
 */
export async function storedZip(
  files: readonly FolderFile[],
  options: { readonly sums?: boolean; readonly progress?: (p: ArchiveProgress) => void; readonly signal?: AbortSignal } = {},
): Promise<BuiltZip> {
  const { progress, signal } = options;
  const summed = options.sums ?? true;
  const total = files.reduce((n, f) => n + f.file.size, 0);
  let done = 0;
  const locals: Uint8Array[] = [];
  const centralAt: number[] = [];
  const localAt: number[] = [];
  const central = new Writer();
  let at = 0;
  const utf8 = new TextEncoder();
  for (const f of files) {
    const crc = !summed
      ? 0
      : await crcOf(
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
    centralAt.push(central.length + CENTRAL_CRC);
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
    locals.push(local);
    localAt.push(at + LOCAL_CRC);
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
  const tail = end.done();
  // The headers and the directory are the only bytes written here; the files
  // are the files.
  const blobOf = (headers: readonly Uint8Array[], dir: Uint8Array): Blob => {
    const parts: BlobPart[] = [];
    files.forEach((f, i) => parts.push(headers[i] as BlobPart, f.file));
    parts.push(dir as BlobPart, tail as BlobPart);
    return new Blob(parts, { type: "application/zip" });
  };
  const directoryAt = at;
  return {
    blob: blobOf(locals, directory),
    files,
    summed,
    sumAt: localAt.map((local, i) => ({ local, central: directoryAt + (centralAt[i] as number) })),
    withSums(crcs: readonly number[]): Blob {
      const headers = locals.map((local, i) => {
        const copy = local.slice();
        withCrc(copy, LOCAL_CRC, crcs[i] ?? 0);
        return copy;
      });
      const dir = directory.slice();
      centralAt.forEach((crcAt, i) => withCrc(dir, crcAt, crcs[i] ?? 0));
      return blobOf(headers, dir);
    },
  };
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

/** The kind of dataset a folder holds when it has to be read as one space: an
 *  ADIOS2 BP5 dataset, or a Zarr store (OME-Zarr included). Told by names
 *  alone, before anything is read. Null for a folder of files that each read
 *  on their own. */
export function datasetIn(files: readonly FolderFile[]): "bp5" | "zarr" | null {
  let zarr = false;
  for (const f of files) {
    const leaf = leafOf(f.path);
    if (leaf === "md.idx" || leaf === "md.0" || leaf === "mmd.0") return "bp5";
    if (leaf === ".zgroup" || leaf === ".zarray" || leaf === "zarr.json") zarr = true;
  }
  return zarr ? "zarr" : null;
}
