// The CRC-32s of a large folder, taken after its archive opens.

import { test } from "node:test";
import assert from "node:assert/strict";

import { crc32Update } from "../src/crc32.ts";
import { storedZip, type FolderFile } from "../src/folderzip.ts";
import { ArchiveSums, SumJob } from "../src/sumjob.ts";

const file = (path: string, bytes: Uint8Array): FolderFile => ({ path, file: new Blob([bytes as Uint8Array<ArrayBuffer>]) });

test("the sums come out the same taken in the background as taken before the archive opened", async () => {
  // Past one idle slice, so a file is summed in more than one piece.
  const big = new Uint8Array(9 * 1024 * 1024).map((_, i) => (i * 31) & 0xff);
  const files = [file("s/md.idx", new TextEncoder().encode("index")), file("s/data.0", big), file("s/empty", new Uint8Array(0))];
  const job = new SumJob(files.map((f) => f.file));
  let told = 0;
  job.onChange(() => told++);
  assert.equal(job.sum(0), null);
  const crcs = await job.whenDone();
  assert.deepEqual(crcs, [crc32Update(0, new TextEncoder().encode("index")), crc32Update(0, big), 0]);
  assert.equal(job.read, job.total);
  assert.ok(told > 0);

  const built = await storedZip(files, { sums: false });
  const sums = new ArchiveSums(built, job);
  const first = built.sumAt[1] as { local: number; central: number };
  assert.deepEqual(sums.slotAt(first.local), { crc: crc32Update(0, big) });
  assert.deepEqual(sums.slotAt(first.central), { crc: crc32Update(0, big) });
  assert.equal(sums.slotAt(first.local + 1), null, "only where a field starts");
  const summed = new Uint8Array(await (await sums.summed()).arrayBuffer());
  const before = new Uint8Array(await (await storedZip(files)).blob.arrayBuffer());
  assert.deepEqual(summed, before);
});
