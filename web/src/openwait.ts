// Asking the core again for a stream that did not open at once, until it opens
// or will not: what Open unpacked does after one press. Kept apart from
// main.ts, which cannot be loaded without the editor's wasm, so that the
// asking and the giving up can be tested on their own.

import type { Unpacking } from "./doc.ts";
import { ChunkWait, MAX_ROUNDS } from "./chunkwait.ts";

/** What one ask of the core for a stream came to. */
export type OpenStep<T> =
  /** An answer: the stream opened, would not open, or there was none. */
  | { readonly kind: "done"; readonly value: T }
  /** A large stream is being unpacked a go at a time; ask again after the
   *  page has drawn. */
  | { readonly kind: "working"; readonly unpacking: Unpacking | null }
  /** The core waits on these chunks of the file; ask again once they load.
   *  `unpacking` is how far the pass waiting on them has got, when a pass
   *  reading its run a window at a time is what waits. */
  | { readonly kind: "pending"; readonly chunks: readonly number[]; readonly unpacking?: Unpacking | null };

/** What the wait says and does as it goes. Every one is called only while
 *  `live` says the wait is still wanted. */
export type OpenHooks<T> = {
  /** False once the wait is no longer wanted: another stream was pressed, or
   *  the tab was closed. Asked before every ask after the first. */
  readonly live: () => boolean;
  readonly working: (unpacking: Unpacking | null) => void;
  readonly loading: (unpacking: Unpacking | null) => void;
  readonly done: (value: T) => void;
  /** The core asked again for chunks that loaded for this wait, with no
   *  unpacking done in between: they were pushed out of the store before it
   *  could use them, so loading them again would go round for ever. Also
   *  said after `limit` loads with no progress (see `WaitOptions`). */
  readonly lost: () => void;
  /** A chunk could not be read from the file. */
  readonly failed: (error: unknown) => void;
};

/** How `waitToOpen` goes about it, beyond what it is told to do. */
export type WaitOptions = {
  /** How a "working" answer is asked again: after a timeout of 0 unless
   *  given. */
  readonly later?: (then: () => void) => void;
  /** The most rounds of loading since the core last said it got further:
   *  `MAX_ROUNDS` unless given. A wait whose answers carry no progress
   *  passes `roundLimit` of its file, so that one which keeps loading chunks
   *  it has not loaded before is not stopped part way. */
  readonly limit?: number;
};

/**
 * Carry a stream on from `first`, the answer to the press, until it opens or
 * will not. A "working" answer is asked again after `later`; a "pending" one
 * once `load` has the chunks it names.
 *
 * It gives up by the rule in `ChunkWait`: any chunk asked for again after it
 * loaded ends the wait, unless the core got further in between. A "pending"
 * answer whose progress has moved is the core getting further, and starts
 * the count of rounds again. A "working" answer lets the chunks loaded so
 * far be asked for again, and the count goes on. A stream that needs more
 * chunks at once than the store keeps asks each time for the ones the last
 * load pushed out, which is a different list each time, so comparing each
 * list with the one before it would never stop it.
 */
export function waitToOpen<T>(
  first: OpenStep<T>,
  ask: () => OpenStep<T>,
  load: (chunks: readonly number[]) => Promise<void>,
  hooks: OpenHooks<T>,
  opts: WaitOptions = {},
): void {
  const later = opts.later ?? ((then: () => void): void => void setTimeout(then, 0));
  const wait = new ChunkWait({ limit: opts.limit ?? MAX_ROUNDS });
  /** How far the pass this wait carries had read, the last time it said. A
   *  pass reading its run a window at a time waits for the next window's
   *  chunks with no "working" answer in between, and each wait that says it
   *  read further is the core getting further. */
  let read = -1;
  const further = (u: Unpacking | null | undefined): boolean => {
    if (u === null || u === undefined || u.read_bits <= read) return false;
    read = u.read_bits;
    return true;
  };
  const again = (): void => {
    if (hooks.live()) take(ask());
  };
  const take = (step: OpenStep<T>): void => {
    if (step.kind === "done") {
      hooks.done(step.value);
      return;
    }
    if (step.kind === "working") {
      further(step.unpacking);
      wait.worked();
      hooks.working(step.unpacking);
      later(again);
      return;
    }
    if (further(step.unpacking)) wait.gotFurther();
    const chunks = wait.next(step.chunks);
    if (chunks === null) {
      hooks.lost();
      return;
    }
    hooks.loading(step.unpacking ?? null);
    load(chunks).then(again, (error: unknown) => {
      if (hooks.live()) hooks.failed(error);
    });
  };
  take(first);
}
