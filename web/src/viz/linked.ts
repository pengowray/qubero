// One hover and one selection per document, shared by every picture of it.
//
// Pointing at a bit, a field or a record in any diagram should light it in
// the hex view and in every other piece that is open, so that the pieces can
// be explored as one instrument rather than each on its own. Before this,
// each view synced with the others through `main.ts` by hand, one pair of
// views at a time.
//
// The bus carries two things and nothing else:
// - `hover`: the bits the pointer is over, in file bit offsets, or null.
//   Published by whatever the pointer is on; the hex view marks it.
// - `selection`: the field the cursor is in, as a `FieldPick`, or null.
//   Published by `main.ts` from what it already tracks; pieces light it.
// A piece that subscribes keeps the function `subscribe` gave back and calls
// it when its element goes, or it will go on being told about a document it
// is no longer drawing.
//
// DOM-free, and keyed by document object rather than imported from `doc.ts`,
// so the tests can make a bus without a wasm module behind it.

import type { FieldPick } from "../doc.ts";

/** A run of bits, counted from the top bit of byte 0, end exclusive. The same
 *  shape as `hexview.ts`'s, written out here so this module imports nothing
 *  that touches the page. */
export type BitRange = { readonly startBit: number; readonly endBit: number };

export type LinkedState = {
  readonly hover: BitRange | null;
  readonly selection: FieldPick | null;
};

export type LinkedChange = "hover" | "selection";

export type LinkedListener = (state: LinkedState, changed: LinkedChange) => void;

function sameRange(a: BitRange | null, b: BitRange | null): boolean {
  if (a === null || b === null) return a === b;
  return a.startBit === b.startBit && a.endBit === b.endBit;
}

function samePick(a: FieldPick | null, b: FieldPick | null): boolean {
  if (a === null || b === null) return a === b;
  return (
    a.startBit === b.startBit &&
    a.endBit === b.endBit &&
    a.path.length === b.path.length &&
    a.path.every((step, i) => step === b.path[i])
  );
}

export class LinkedBus {
  private current: LinkedState = { hover: null, selection: null };
  private readonly listeners = new Set<LinkedListener>();

  get state(): LinkedState {
    return this.current;
  }

  /** Be told of every change from now on. Returns the way to stop. */
  subscribe(listener: LinkedListener): () => void {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  }

  /** Say what is hovered or selected now. A field left out of `next` keeps
   *  its value. Listeners hear only about a field that actually changed, so a
   *  piece that publishes on every pointer move costs nothing while the
   *  pointer stays on one thing. */
  publish(next: Partial<LinkedState>): void {
    const hover = next.hover === undefined ? this.current.hover : next.hover;
    const selection = next.selection === undefined ? this.current.selection : next.selection;
    const hoverChanged = !sameRange(hover, this.current.hover);
    const selectionChanged = !samePick(selection, this.current.selection);
    if (!hoverChanged && !selectionChanged) return;
    this.current = { hover, selection };
    // Copied first: a listener that unsubscribes itself must not stop the
    // ones after it hearing this change.
    const listeners = [...this.listeners];
    if (hoverChanged) for (const l of listeners) l(this.current, "hover");
    if (selectionChanged) for (const l of listeners) l(this.current, "selection");
  }

  /** How many are listening. For the tests, and for a host checking that a
   *  piece it removed let go. */
  get listening(): number {
    return this.listeners.size;
  }
}

/**
 * Listen for as long as `el` stays on the page. A piece drawn inside a host
 * that rebuilds itself (the inspector, a listing card) is thrown away without
 * being told, so it cannot unsubscribe at the right moment; this lets go at
 * the first change after the element has left the page. An element that has
 * not been put on the page yet is waited for.
 */
export function subscribeWhileConnected(bus: LinkedBus, el: { readonly isConnected: boolean }, listener: LinkedListener): () => void {
  let seen = el.isConnected;
  const stop = bus.subscribe((state, changed) => {
    if (el.isConnected) seen = true;
    else if (seen) {
      stop();
      return;
    }
    listener(state, changed);
  });
  return stop;
}

const buses = new WeakMap<object, LinkedBus>();

/** The bus for one document. Every piece drawing that document, and the
 *  document's tab in `main.ts`, asks for it here, so they all get the same
 *  one; a second document open in another tab has a bus of its own and its
 *  hover never lights the first one's bytes. */
export function linkedFor(doc: object): LinkedBus {
  let bus = buses.get(doc);
  if (bus === undefined) {
    bus = new LinkedBus();
    buses.set(doc, bus);
  }
  return bus;
}
