// Back and forward, for the links that jump the cursor somewhere else in the
// file: an offset a field points at, a weight in a block, a breadcrumb, the
// go-to box. Following one of those is the same move as following a link on a
// page, and the way back from a link is the browser's own Back button.
//
// Only jumps go in. Scrolling, arrow keys and clicking in the hex view are the
// reader moving around where they already are, and filling the history with
// those would leave Back doing nothing anyone asked for.

/** One place in one file. */
type Position = {
  /** Which file this was in. A position in the file open before this one is
   *  not somewhere the cursor can go, so it is ignored rather than followed. */
  readonly docId: number;
  /** Absolute bit offset of the cursor. */
  readonly bit: number;
  /** How many bits were marked on arrival, where the jump was to a run of them
   *  rather than to a place. */
  readonly len?: number;
};

let docId = 0;
let go: ((bit: number, len?: number) => void) | null = null;

function isPosition(s: unknown): s is Position {
  return typeof s === "object" && s !== null && "docId" in s && "bit" in s;
}

/**
 * Begin a fresh run of positions, for a file that has just been opened. Entries
 * pushed for the file before it stay in the browser's history and are ignored,
 * which is also what happens to entries left over from before a reload: the
 * file is gone, so there is nowhere to go back to.
 */
export function startFile(bit: number): void {
  docId += 1;
  history.replaceState({ docId, bit } satisfies Position, "");
}

/** Where the cursor goes when the reader goes back. Set once per file opened,
 *  so that only the views on screen are ever moved. */
export function onGo(cb: (bit: number, len?: number) => void): void {
  go = cb;
}

/**
 * Take note of a jump from `from` to `to`, before making it. The place being
 * left is written into the current entry rather than assumed, because the
 * cursor has usually moved since the last jump.
 */
export function recordJump(from: number, to: number, len?: number): void {
  if (from === to) return;
  // Rewriting the entry being left would drop what was marked when the reader
  // arrived on it, so it is only rewritten when the cursor has moved since.
  const cur: unknown = history.state;
  if (!isPosition(cur) || cur.docId !== docId || cur.bit !== from) {
    history.replaceState({ docId, bit: from } satisfies Position, "");
  }
  const at: Position = len === undefined ? { docId, bit: to } : { docId, bit: to, len };
  history.pushState(at, "");
}

window.addEventListener("popstate", (e) => {
  if (!isPosition(e.state) || e.state.docId !== docId) return;
  go?.(e.state.bit, e.state.len);
});
