// The autocorrelation behind the stride raster's line, off the page's thread.
// A quarter of a megabyte through two FFTs is tens of milliseconds, long
// enough to stall a scroll if it ran on the page. The arithmetic is
// `periodicity.ts`; this only carries the request in and the answer out.
// Started by `strideraster.ts`, the way `sumjob.ts` starts `crcworker.ts`.

import { periodicity, type PeriodicityRequest, type PeriodicityResult } from "./periodicity.ts";

const post = (message: PeriodicityResult): void => (self as unknown as { postMessage(m: PeriodicityResult, transfer: Transferable[]): void }).postMessage(message, [message.r.buffer]);

self.onmessage = (event: MessageEvent<PeriodicityRequest>): void => {
  post(periodicity(event.data));
};
