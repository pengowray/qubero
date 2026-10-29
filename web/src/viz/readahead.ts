// Reading every record of a long list without stopping the page.
//
// The read-ahead helper of docs/DESIGN-diagrams.md, section 3.10. A piece that
// says something about a whole list (the record tiles, the run sparkline)
// reads every record of it, and each record is a call or a few into the core.
// A WAD directory has 3,615 lumps; read in one go that is about half a second
// in which the page answers nothing, which is five times what ui-design allows
// for a press. Read here, the same work is done a few milliseconds at a time
// with the page given its turn in between (`Walk.step`), so a scroll or a
// click lands at once and the figure appears when the reading is done.
//
// Three rules every walk keeps:
// - A walk belongs to the document as it was when it started. Bytes arriving
//   or an edit can change any record, so a walk that finds the document has
//   changed under it stops, and what it found is thrown away (`alive`).
// - Walks run one after another, not side by side (`queued`). A record with
//   twenty number fields would otherwise start twenty walks over the same
//   four thousand records that all crawl along together, and the first line
//   would be as late as the last.
// - A list's records are read from the core once (`recordsOf`), however many
//   walks want them: the tiles and every sparkline of a list share one read.
//
// A record whose bytes have not arrived is `WAIT`: the walk gives up, nothing
// is kept, and the piece is asked again when the bytes land, which changes the
// document and starts a fresh walk.

import type { Doc, TemplateNode } from "../doc.ts";

/** Waiting on bytes: ask again when the document changes. */
export const WAIT = Symbol("wait");
export type Wait = typeof WAIT;

/** How long a walk works before it gives the page a turn, in milliseconds.
 *  Half a frame at 60 Hz. */
const SLICE_MS = 8;
/** Records read per call to the core. */
const PAGE = 512;

type DocState = {
  /** Counts the document's changes. A walk is alive while it is the count the
   *  walk started at. */
  gen: number;
  /** Each list's records, by list path, read once per change. */
  readonly records: Map<string, Promise<TemplateNode[] | Wait>>;
};

const states = new WeakMap<Doc, DocState>();

function stateOf(doc: Doc): DocState {
  let s = states.get(doc);
  if (s === undefined) {
    const fresh: DocState = { gen: 0, records: new Map() };
    s = fresh;
    states.set(doc, fresh);
    doc.onChange(() => {
      fresh.gen++;
      fresh.records.clear();
    });
  }
  return s;
}

/** Give the page a turn: input, paint, and anything else waiting. */
function yieldToPage(): Promise<void> {
  const sched = (globalThis as { scheduler?: { yield?: () => Promise<void> } }).scheduler;
  if (sched?.yield !== undefined) return sched.yield();
  return new Promise((resolve) => setTimeout(resolve, 0));
}

export type Walk = {
  readonly doc: Doc;
  /** False once the document has changed since the walk started. */
  alive(): boolean;
  /** Call between pieces of work: gives the page a turn when this slice has
   *  run its time. */
  step(): Promise<void>;
};

/** A walk over `doc` as it is now. */
export function walk(doc: Doc): Walk {
  const s = stateOf(doc);
  const gen = s.gen;
  let since = performance.now();
  return {
    doc,
    alive: () => s.gen === gen,
    step: async () => {
      if (performance.now() - since < SLICE_MS) return;
      await yieldToPage();
      since = performance.now();
    },
  };
}

let tail: Promise<unknown> = Promise.resolve();

/** Run `job` once every job queued before it has finished. */
export function queued<T>(job: () => Promise<T>): Promise<T> {
  const run = tail.then(job, job);
  tail = run.catch(() => undefined);
  return run;
}

/** The first `count` records of the list at `list`, read a page at a time
 *  with the page given its turn between pages, or `WAIT` while some have not
 *  arrived. Read once per document change, however many ask. A list cut short
 *  by an error gives the records before it. */
export function recordsOf(w: Walk, list: TemplateNode, count: number): Promise<TemplateNode[] | Wait> {
  const s = stateOf(w.doc);
  const key = `${list.path.join(".")}:${count}`;
  let had = s.records.get(key);
  if (had === undefined) {
    had = readRecords(w, list.path, count);
    s.records.set(key, had);
    // A read that met bytes still on the way is not kept: the next walk reads
    // again once they are there.
    void had.then((r) => {
      if (r === WAIT && s.records.get(key) === had) s.records.delete(key);
    });
  }
  return had;
}

async function readRecords(w: Walk, path: readonly number[], count: number): Promise<TemplateNode[] | Wait> {
  const out: TemplateNode[] = [];
  for (let from = 0; from < count; from += PAGE) {
    const reply = w.doc.templateChildren(path, from, Math.min(count, from + PAGE));
    if (reply.status === "error") return out;
    if (reply.status !== "ok") return WAIT;
    out.push(...reply.node);
    await w.step();
    if (!w.alive()) return WAIT;
  }
  return out;
}
