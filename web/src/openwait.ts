// Asking the core again for a stream that did not open at once, until it opens
// or will not: what Open unpacked does after one press. Kept apart from
// main.ts, which cannot be loaded without the editor's wasm, so that the
// asking and the giving up can be tested on their own.

import type { Unpacking } from "./doc.ts";

/** What one ask of the core for a stream came to. */
export type OpenStep<T> =
  /** An answer: the stream opened, would not open, or there was none. */
  | { readonly kind: "done"; readonly value: T }
  /** A large stream is being unpacked a go at a time; ask again after the
   *  page has drawn. */
  | { readonly kind: "working"; readonly unpacking: Unpacking | null }
  /** The core waits on these chunks of the file; ask again once they load. */
  | { readonly kind: "pending"; readonly chunks: readonly number[] };

/** What the wait says and does as it goes. Every one is called only while
 *  `live` says the wait is still wanted. */
export type OpenHooks<T> = {
  /** False once the wait is no longer wanted: another stream was pressed, or
   *  the tab was closed. Asked before every ask after the first. */
  readonly live: () => boolean;
  readonly working: (unpacking: Unpacking | null) => void;
  readonly loading: () => void;
  readonly done: (value: T) => void;
  /** The core asked again for chunks that loaded for this wait, with no
   *  unpacking done in between: they were pushed out of the store before it
   *  could use them, so loading them again would go round for ever. Also
   *  said after `MAX_ROUNDS` loads. */
  readonly lost: () => void;
  /** A chunk could not be read from the file. */
  readonly failed: (error: unknown) => void;
};

/**
 * How many times one wait loads chunks before it gives up. A backstop only:
 * a stream that needs more chunks at once than the store keeps is stopped by
 * the rule in `waitToOpen` within a round or two, and every other round loads
 * chunks this wait had not loaded since the core last got further.
 */
export const MAX_ROUNDS = 512;

/**
 * Carry a stream on from `first`, the answer to the press, until it opens or
 * will not. A "working" answer is asked again after `later`; a "pending" one
 * once `load` has the chunks it names.
 *
 * The rule for giving up is the one `Doc.textWindow` uses, widened: there a
 * round that asks for the same chunks as the one before it ends the loop.
 * Here any chunk asked for again after it loaded ends it, unless the core
 * said it got further in between. A stream that needs more chunks at once
 * than the store keeps asks each time for the ones the last load pushed out,
 * which is a different list each time, so the narrower rule never stops it.
 */
export function waitToOpen<T>(
  first: OpenStep<T>,
  ask: () => OpenStep<T>,
  load: (chunks: readonly number[]) => Promise<void>,
  hooks: OpenHooks<T>,
  later: (then: () => void) => void = (then) => void setTimeout(then, 0),
): void {
  /** Chunks loaded for this wait since the core last said it got further. */
  const loaded = new Set<number>();
  let rounds = 0;
  const again = (): void => {
    if (hooks.live()) take(ask());
  };
  const take = (step: OpenStep<T>): void => {
    if (step.kind === "done") {
      hooks.done(step.value);
      return;
    }
    if (step.kind === "working") {
      loaded.clear();
      hooks.working(step.unpacking);
      later(again);
      return;
    }
    const { chunks } = step;
    rounds += 1;
    if (chunks.length === 0 || rounds > MAX_ROUNDS || chunks.some((c) => loaded.has(c))) {
      hooks.lost();
      return;
    }
    hooks.loading();
    load(chunks).then(
      () => {
        for (const c of chunks) loaded.add(c);
        again();
      },
      (error: unknown) => {
        if (hooks.live()) hooks.failed(error);
      },
    );
  };
  take(first);
}
