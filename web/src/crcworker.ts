/**
 * The CRC-32s of a large folder's files, taken off the main thread so the
 * editor stays as quick while they are read. See `sumjob.ts`, which starts
 * this and says what the sums are for.
 */

import { crc32Update } from "./crc32.ts";

/** What the page hands the worker: the files, in the archive's order. */
export type SumRequest = { readonly files: readonly Blob[] };

/** What the worker tells the page as it goes. */
export type SumMessage =
  | { readonly kind: "read"; readonly bytes: number }
  | { readonly kind: "sum"; readonly index: number; readonly crc: number }
  | { readonly kind: "failed"; readonly message: string };

/** How many bytes are read between the counts sent back. Enough to keep the
 *  messages few, and small enough that a save waiting on them moves. */
const REPORT_EVERY = 8 * 1024 * 1024;

const post = (message: SumMessage): void => (self as unknown as { postMessage(m: SumMessage): void }).postMessage(message);

self.onmessage = async (event: MessageEvent<SumRequest>): Promise<void> => {
  try {
    for (const [index, file] of event.data.files.entries()) {
      const reader = file.stream().getReader();
      let crc = 0;
      let unreported = 0;
      for (;;) {
        const { done, value } = await reader.read();
        if (done) break;
        crc = crc32Update(crc, value);
        unreported += value.length;
        if (unreported >= REPORT_EVERY) {
          post({ kind: "read", bytes: unreported });
          unreported = 0;
        }
      }
      if (unreported > 0) post({ kind: "read", bytes: unreported });
      post({ kind: "sum", index, crc });
    }
  } catch (error) {
    post({ kind: "failed", message: error instanceof Error ? error.message : String(error) });
  }
};
