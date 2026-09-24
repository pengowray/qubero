// One bounded walk of the template for the things several sections look for
// anywhere in the file: compressed streams, tables, runs of text, and values
// that are wrong.
//
// Bounded two ways, so a gigabyte file costs the same as a small one: at most
// `NODES` nodes in all, and at most `PER_LIST` elements of any one list, since
// the thousandth record of a list is almost always the same kind of thing as
// the first. What the walk did not reach, it says it did not reach.
//
// Wrong values are found by their own walk, which follows the counts every
// node carries of the wrong values under it (`problems_within`), so it goes
// straight to them however deep they are, the way the rail's "Show first"
// does.

import type { Doc, TemplateNode } from "../doc.ts";
import { ok } from "./model.ts";
import { WAIT } from "./section.ts";

const NODES = 4000;
const PER_LIST = 32;
/** Wrong values collected at most, and nodes looked at to find them. */
const PROBLEMS = 200;
const PROBLEM_NODES = 6000;
/** Text runs shorter than this are names and labels, not text to read. */
const TEXT_MIN_BYTES = 128;

export type WalkResult = {
  /** Compressed runs, opened or refused, in file order. */
  readonly streams: readonly TemplateNode[];
  /** Fields the template says are rows of a table. */
  readonly tables: readonly TemplateNode[];
  /** Runs of text long enough to read, largest first. */
  readonly texts: readonly TemplateNode[];
  /** True when the walk stopped at its limit or skipped elements of a list. */
  readonly partial: boolean;
};

export function walkTemplate(doc: Doc): WalkResult | typeof WAIT {
  const streams: TemplateNode[] = [];
  const tables: TemplateNode[] = [];
  const texts: TemplateNode[] = [];
  let seen = 0;
  let partial = false;
  const stack: TemplateNode[] = [];
  const root = ok(doc.templateNode([]));
  if (root === WAIT) return WAIT;
  if (root === null) return { streams, tables, texts, partial };
  stack.push(root);
  while (stack.length > 0) {
    const n = stack.pop() as TemplateNode;
    if (n.decoded) {
      streams.push(n);
      continue;
    }
    if (n.table === true) tables.push(n);
    if (n.kind === "str" && n.value_bytes >= TEXT_MIN_BYTES) texts.push(n);
    if (!n.composite || n.child_count === 0) continue;
    if (seen >= NODES) {
      partial = true;
      continue;
    }
    // A list of plain values is values, and none of them is a stream or a
    // table of its own.
    if (n.list && n.table === true) continue;
    const want = n.list ? Math.min(n.child_count, PER_LIST) : Math.min(n.child_count, NODES - seen);
    if (want < n.child_count) partial = true;
    const kids = ok(doc.templateChildren(n.path, 0, want));
    // A subtree whose bytes have not arrived is passed over rather than
    // waited for: in a large file those are the bytes a walk of the start has
    // no business pulling in, and the walk says it is partial.
    if (kids === WAIT) {
      partial = true;
      continue;
    }
    if (kids === null) continue;
    seen += kids.length;
    // Pushed backwards, so they come off the stack in file order.
    for (let i = kids.length - 1; i >= 0; i--) {
      const k = kids[i];
      if (k === undefined || k.absent || k.space !== n.space) continue;
      if (n.list && !k.composite && !k.decoded) continue;
      stack.push(k);
    }
  }
  streams.sort((a, b) => a.offset_bits - b.offset_bits);
  texts.sort((a, b) => b.value_bytes - a.value_bytes);
  return { streams, tables, texts, partial };
}

export type ProblemWalk = {
  readonly nodes: readonly TemplateNode[];
  /** The counts the root carries, which cover everything read so far. */
  readonly invalid: number;
  readonly undefined: number;
  /** True when there are more than were collected. */
  readonly more: boolean;
};

/** Every value the core has marked as wrong, down to `PROBLEMS` of them. */
export function walkProblems(doc: Doc): ProblemWalk | typeof WAIT {
  const root = ok(doc.templateNode([]));
  if (root === WAIT) return WAIT;
  if (root === null) return { nodes: [], invalid: 0, undefined: 0, more: false };
  const own = root.problem;
  const invalid = root.problems_within[0] + (own?.tier === "invalid" ? 1 : 0);
  const undef = root.problems_within[1] + (own?.tier === "undefined" ? 1 : 0);
  const nodes: TemplateNode[] = [];
  let spent = 0;
  let more = false;
  const visit = (n: TemplateNode): boolean => {
    if (n.problem !== undefined) {
      if (nodes.length >= PROBLEMS) {
        more = true;
        return false;
      }
      nodes.push(n);
    }
    if (n.problems_within[0] + n.problems_within[1] === 0 || !n.composite) return true;
    let from = 0;
    while (from < n.child_count) {
      if (spent > PROBLEM_NODES) {
        more = true;
        return false;
      }
      const to = Math.min(n.child_count, from + 256);
      const kids = ok(doc.templateChildren(n.path, from, to));
      // Not arrived: counted by the root already, and not walked to here.
      if (kids === WAIT) {
        more = true;
        return true;
      }
      if (kids === null) return true;
      spent += kids.length;
      for (const k of kids) {
        if (k.problem === undefined && k.problems_within[0] + k.problems_within[1] === 0) continue;
        if (!visit(k)) return false;
      }
      from = to;
    }
    return true;
  };
  visit(root);
  return { nodes, invalid, undefined: undef, more };
}
