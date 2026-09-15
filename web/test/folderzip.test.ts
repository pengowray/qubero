// A folder written as one stored ZIP: the order its files go in, their CRCs,
// and records any ZIP reader walks.

import { test } from "node:test";
import assert from "node:assert/strict";

import { crc32Update, datasetIn, missingFromDataset, orderForArchive, storedZip, type FolderFile } from "../src/folderzip.ts";

const file = (path: string, text: string): FolderFile => ({ path, file: new Blob([text]) });

test("a CRC-32 matches the check value, however the bytes are split", () => {
  const bytes = new TextEncoder().encode("123456789");
  assert.equal(crc32Update(0, bytes), 0xcbf43926);
  assert.equal(crc32Update(crc32Update(0, bytes.slice(0, 5)), bytes.slice(5)), 0xcbf43926);
  const long = new Uint8Array(1000).map((_, i) => (i * 7) & 0xff);
  let split = 0;
  for (let i = 0; i < long.length; i += 13) split = crc32Update(split, long.slice(i, i + 13));
  assert.equal(split, crc32Update(0, long));
});

test("the files a format is recognised by go first and data files last", () => {
  const order = orderForArchive([
    file("steps.bp5/data.0", "d"),
    file("steps.bp5/profiling.json", "p"),
    file("steps.bp5/md.0", "m"),
    file("steps.bp5/mmd.0", "f"),
    file("steps.bp5/md.idx", "i"),
  ]).map((f) => f.path);
  assert.deepEqual(order, ["steps.bp5/md.idx", "steps.bp5/mmd.0", "steps.bp5/md.0", "steps.bp5/profiling.json", "steps.bp5/data.0"]);
});

test("a Zarr store's root metadata leads the metadata under it", () => {
  const order = orderForArchive([
    file("image.zarr/0/0.0", "chunk"),
    file("image.zarr/0/.zarray", "a"),
    file("image.zarr/labels/.zgroup", "g"),
    file("image.zarr/.zattrs", "r"),
    file("image.zarr/.zgroup", "g"),
  ]).map((f) => f.path);
  assert.deepEqual(order, ["image.zarr/.zgroup", "image.zarr/.zattrs", "image.zarr/labels/.zgroup", "image.zarr/0/.zarray", "image.zarr/0/0.0"]);
});

test("a large folder's archive holds nought for each sum until the sums are written in", async () => {
  const files = [file("steps.bp5/md.idx", "index"), file("steps.bp5/data.0", "0123456789")];
  const built = await storedZip(files, { sums: false });
  assert.equal(built.summed, false);
  const bare = new DataView(await built.blob.arrayBuffer());
  const crcs = await Promise.all(files.map(async (f) => crc32Update(0, new Uint8Array(await f.file.arrayBuffer()))));
  for (const at of built.sumAt) {
    assert.equal(bare.getUint32(at.local, true), 0);
    assert.equal(bare.getUint32(at.central, true), 0);
  }
  const summed = await built.withSums(crcs).arrayBuffer();
  const view = new DataView(summed);
  built.sumAt.forEach((at, i) => {
    assert.equal(view.getUint32(at.local, true), crcs[i]);
    assert.equal(view.getUint32(at.central, true), crcs[i]);
  });
  // Nothing else moved: the archive summed as it was written is the same bytes.
  const whole = new Uint8Array(await (await storedZip(files)).blob.arrayBuffer());
  assert.deepEqual(new Uint8Array(summed), whole);
});

test("a stored ZIP holds each file whole behind its header, and a directory that finds them", async () => {
  const files = [file("steps.bp5/md.idx", "index"), file("steps.bp5/data.0", "0123456789")];
  const zip = new Uint8Array(await (await storedZip(files)).blob.arrayBuffer());
  const view = new DataView(zip.buffer);
  let at = 0;
  for (const f of files) {
    assert.equal(view.getUint32(at, true), 0x04034b50);
    assert.equal(view.getUint16(at + 8, true), 0, "stored");
    const text = await f.file.text();
    const bytes = new TextEncoder().encode(text);
    assert.equal(view.getUint32(at + 14, true), crc32Update(0, bytes));
    assert.equal(view.getUint32(at + 18, true), bytes.length);
    const nameLength = view.getUint16(at + 26, true);
    const extraLength = view.getUint16(at + 28, true);
    assert.equal(new TextDecoder().decode(zip.slice(at + 30, at + 30 + nameLength)), f.path);
    const dataAt = at + 30 + nameLength + extraLength;
    assert.equal(new TextDecoder().decode(zip.slice(dataAt, dataAt + bytes.length)), text);
    at = dataAt + bytes.length;
  }
  // The central directory, and the end record pointing at it.
  const end = zip.length - 22;
  assert.equal(view.getUint32(end, true), 0x06054b50);
  assert.equal(view.getUint16(end + 10, true), 2);
  assert.equal(view.getUint32(end + 16, true), at);
  assert.equal(view.getUint32(at, true), 0x02014b50);
  assert.equal(view.getUint32(at + 42, true), 0, "the first entry's local header is at the start");
});

test("a BP5 folder says which of its companions it lacks", () => {
  assert.deepEqual(missingFromDataset([file("s/md.idx", ""), file("s/md.0", "")]), ["mmd.0", "data.0"]);
  assert.deepEqual(missingFromDataset([file("s/md.idx", ""), file("s/md.0", ""), file("s/mmd.0", ""), file("s/data.0", "")]), []);
  assert.deepEqual(missingFromDataset([file("store/.zgroup", "")]), []);
});

test("a folder is a dataset by the names of its files, and otherwise only files", () => {
  assert.equal(datasetIn([file("s/data.0", ""), file("s/md.idx", "")]), "bp5");
  assert.equal(datasetIn([file("image.zarr/.zattrs", ""), file("image.zarr/0/.zarray", "")]), "zarr");
  assert.equal(datasetIn([file("v3/zarr.json", "")]), "zarr");
  assert.equal(datasetIn([file("photos/a.bmp", ""), file("photos/sub/b.cab", "")]), null);
});
