// Clicking a field's line in the Template tab: finding a node of the file's
// tree that the line declares, which is `findLineAt` run backwards.
//
// The search has two halves. `routesTo` reads the template text for the
// fields that enclose the clicked line, from the root down: each structure's
// opening line, out through the switches whose arms hold it, and from a
// top-level `type X` to every field whose statement names X. `jumpTarget` then
// walks the tree along those field names and asks `findLineAt` of each node it
// reaches whether that node is declared on the clicked line. `findLineAt` is
// the judge, so a node the search picks is one the panel then marks on the
// same line, and an arm the file does not take is never reported as found.

import { fieldSpan, findLineAt, sameText, type Block, type ChainNode, type Kid, type TemplateIndex } from "./templateindex.ts";

/** Elements of one list that are tried, after the one holding the cursor,
 *  before the search gives up on that list. 100 keeps a click on busybox's
 *  section headers or symbols under 100 ms; see `templatejump.test.ts` for
 *  what is counted. */
export const ELEMENTS_TRIED = 100;

/** Children of a structure (not a list) read at most, looking for one by name. */
const FIELDS_READ = 4096;

/** Ways from the root to one line kept at most. A type named by many fields
 *  gives one way per field, and a recursive type gives ways without end. */
const ROUTES_KEPT = 64;

/** One node as the search needs it. */
export type JumpNode = {
  readonly name: string;
  readonly type: string;
  readonly list: boolean;
  readonly childCount: number;
  /** The file did not write this field: its condition came to nothing. */
  readonly absent: boolean;
};

/** The file's tree as the search reads it. "pending" is the core not having
 *  read that far into the file yet. */
export type JumpTree = {
  node(path: readonly number[]): JumpNode | null | "pending";
  /** Children `from` up to `to`, `to` not included. */
  children(path: readonly number[], from: number, to: number): readonly JumpNode[] | null | "pending";
  /** The children `findLineAt` is given for a node: the same ones the panel
   *  gives it when marking the cursor's line, so the two answer alike. */
  kids(path: readonly number[]): readonly Kid[] | null | "pending";
};

export type JumpResult =
  /** `line` is the line `findLineAt` gives for `path`: the clicked line, or
   *  for a node it could not place exactly, the line of a field enclosing it
   *  or of a structure written the same. */
  | { readonly kind: "found"; readonly path: readonly number[]; readonly line: number }
  /** No node of the file is declared on the line: its arm is not taken, its
   *  list is empty, or it is an optional field the file left out. */
  | { readonly kind: "absent" }
  /** None found in the first `tried` elements of a list, and the list has
   *  more. `list` is that list node's name, the first such list's when there
   *  were several. */
  | { readonly kind: "gaveUp"; readonly tried: number; readonly list: string }
  /** Part of the tree has not been read yet. */
  | { readonly kind: "pending" };

const indentOf = (s: string): number => s.length - s.trimStart().length;

/** The name a field line declares: the text before its `: `. */
export function fieldName(index: TemplateIndex, line: number): string {
  const t = (index.lines[line] ?? "").trimStart();
  const at = t.indexOf(": ");
  return at < 0 ? "" : t.slice(0, at);
}

/** Per line, the first line of the field statement it is part of, or -1. A
 *  field statement wrapped over several lines can be clicked on any of them. */
export function fieldLines(index: TemplateIndex): Int32Array {
  const out = new Int32Array(index.lines.length).fill(-1);
  for (const [first, last] of index.stmtEnd) {
    if (index.kinds[first] !== "field") continue;
    for (let i = first; i <= last; i++) out[i] = first;
  }
  return out;
}

/** The structure that has `line` among its own fields. */
function blockOf(index: TemplateIndex, line: number): Block | undefined {
  const name = fieldName(index, line);
  return index.blocks.find((b) => b.fields.get(name) === line);
}

/** Per top-level type name, the field lines whose statement names it. */
function typeRefs(index: TemplateIndex): Map<string, number[]> {
  const refs = new Map<string, number[]>();
  for (const [first, last] of index.stmtEnd) {
    if (index.kinds[first] !== "field") continue;
    const words = new Set(index.lines.slice(first, last + 1).join(" ").match(/[A-Za-z_][\w.]*/g) ?? []);
    for (const w of words) {
      if (!index.typeSpans.has(w)) continue;
      const list = refs.get(w);
      if (list === undefined) refs.set(w, [first]);
      else list.push(first);
    }
  }
  return refs;
}

/** The type names that are `name` or written as `same as type` it, at any
 *  number of steps. */
function namesOf(index: TemplateIndex, name: string): string[] {
  const out = [name];
  for (let k = 0; k < out.length && k < 32; k++) {
    for (const [b, a] of index.alias) if (a === out[k] && !out.includes(b)) out.push(b);
  }
  return out;
}

/**
 * The ways down from the root to the field on `line`, each a list of field
 * lines with the root's field first and `line` last. `rootType` is the type of
 * the tree's root node, for a template whose root is a type written elsewhere.
 */
export function routesTo(index: TemplateIndex, line: number, rootType: string): number[][] {
  const refs = typeRefs(index);
  /** The type `root X` names, for a root written as a type elsewhere. */
  const rootName = /^root ([^\s({=]+)/.exec(index.lines.find((l) => l.startsWith("root ")) ?? "")?.[1] ?? rootType;
  /** What holds the structure or switch opened on `open`: field lines, or
   *  "root" for the root itself. */
  const holders = (open: number, hops = 0): (number | "root")[] => {
    if (hops > 16) return [];
    if (index.kinds[open] === "field") return [open];
    const choice = index.armOf.get(open);
    if (choice !== undefined) return holders(choice.line, hops + 1);
    const text = index.lines[open] ?? "";
    if (indentOf(text) === 0) {
      if (text.startsWith("root ")) return ["root"];
      const name = /^type ([^\s({=]+)/.exec(text)?.[1];
      if (name === undefined) return [];
      const names = namesOf(index, name);
      const out: (number | "root")[] = names.includes(rootType) || names.includes(rootName) ? ["root"] : [];
      for (const n of names) out.push(...(refs.get(n) ?? []));
      return out;
    }
    // A switch written among a structure's fields with no field name of its
    // own: its arms' fields are fields of that structure.
    let outer: Block | undefined;
    for (const b of index.blocks) if (b.open < open && b.end >= open && (outer === undefined || b.open > outer.open)) outer = b;
    return outer === undefined ? [] : holders(outer.open, hops + 1);
  };
  const out: number[][] = [];
  const seen = new Set<string>();
  const climb = (field: number, below: readonly number[]): void => {
    if (out.length >= ROUTES_KEPT || below.includes(field) || below.length > 40) return;
    const route = [field, ...below];
    const b = blockOf(index, field);
    if (b === undefined) return;
    for (const up of holders(b.open)) {
      if (up !== "root") climb(up, route);
      else if (!seen.has(route.join(","))) {
        seen.add(route.join(","));
        out.push(route);
      }
    }
  };
  climb(line, []);
  return out;
}

/** How a node stands to a field line: declared on it, placed by `findLineAt`
 *  on a line that may be it, or declared somewhere else. */
type Verdict = { readonly verdict: "hit" | "near" | "miss"; readonly line: number };

/** One field line of the routes, with the field lines that can follow it.
 *  Routes that start alike share their first steps, so the tree is walked
 *  once for them all. */
type Step = { readonly line: number; readonly name: string; readonly next: Step[] };

function stepsOf(routes: readonly (readonly number[])[], nameOf: (line: number) => string): Step[] {
  const top: Step[] = [];
  for (const r of routes) {
    let level = top;
    for (const l of r) {
      let s = level.find((x) => x.line === l);
      if (s === undefined) {
        s = { line: l, name: nameOf(l), next: [] };
        level.push(s);
      }
      level = s.next;
    }
  }
  return top;
}

/** `tree` with each answer kept for the length of one search, since the
 *  routes and `findLineAt` ask for the same nodes over and over. */
function remembering(tree: JumpTree): JumpTree {
  const nodes = new Map<string, ReturnType<JumpTree["node"]>>();
  const lists = new Map<string, ReturnType<JumpTree["children"]>>();
  const kids = new Map<string, ReturnType<JumpTree["kids"]>>();
  const keep = <T>(m: Map<string, T>, key: string, ask: () => T): T => {
    if (m.has(key)) return m.get(key)!;
    const v = ask();
    m.set(key, v);
    return v;
  };
  return {
    node: (path) => keep(nodes, path.join("/"), () => tree.node(path)),
    children: (path, from, to) => keep(lists, `${path.join("/")}:${from}:${to}`, () => tree.children(path, from, to)),
    kids: (path) => keep(kids, path.join("/"), () => tree.kids(path)),
  };
}

/**
 * A node of the file declared on `line`, found by walking `tree` from its
 * root along the routes [`routesTo`] gives.
 *
 * A list's elements, and a child that repeats its parent's name (`at(...)`
 * makes a node for the placing and one for what is placed), are passed
 * through. Of a list's elements the one on `cursor`'s path is tried first,
 * then the first [`ELEMENTS_TRIED`], and the search does not go further into
 * that list.
 *
 * Every node a name leads to is checked with `findLineAt` against the route's
 * line for it, so a structure in an arm the file did not take is left at once
 * rather than searched. The last node is taken when `findLineAt` gives
 * `line`. Where it gives the line of a field enclosing the route's line
 * (`choose` could not tell two same-named structures apart) or the same field
 * of a structure written out twice the same, the search carries on through
 * it, and a last node found that way is kept in case nothing better is found.
 */
export function jumpTarget(index: TemplateIndex, line: number, asked: JumpTree, cursor: readonly number[] | null, tried = ELEMENTS_TRIED): JumpResult {
  const tree = remembering(asked);
  const root = tree.node([]);
  if (root === "pending") return { kind: "pending" };
  if (root === null) return { kind: "absent" };
  const routes = routesTo(index, line, root.type);
  let pending = false;
  /** The list the search stopped short in, first one first; "" for a structure too big to read whole. */
  let truncated: { list: string; tried: number } | null = null;
  let near: { path: number[]; line: number } | null = null;

  /** How the node at `path` stands to the field on `want`: declared on it,
   *  placed on a line that may be it, or declared somewhere else. */
  const judged = new Map<string, Verdict>();
  const judge = (path: readonly number[], want: number): Verdict => {
    const key = `${path.join("/")}|${want}`;
    let v = judged.get(key);
    if (v === undefined) judged.set(key, (v = judgeOnce(path, want)));
    return v;
  };
  const judgeOnce = (path: readonly number[], want: number): Verdict => {
    const miss = { verdict: "miss", line: -1 } as const;
    const chain: ChainNode[] = [];
    for (let d = 0; d <= path.length; d++) {
      const n = tree.node(path.slice(0, d));
      if (n === "pending") pending = true;
      if (n === null || n === "pending") return miss;
      chain.push({ name: n.name, type: n.type, list: n.list });
    }
    const found = findLineAt(index, chain, (depth) => {
      const k = tree.kids(path.slice(0, depth));
      if (k === "pending") pending = true;
      return k === "pending" ? null : k;
    });
    // No line at all is `findLineAt` not knowing the node, which a field
    // `named by` another is when it is the first below the root: no reason
    // to leave it.
    if (found === null) return { verdict: "near", line: -1 };
    if (found.line === want) return { verdict: "hit", line: want };
    const [a, z] = fieldSpan(index, found.line);
    if (a < want && z >= fieldSpan(index, want)[1]) return { verdict: "near", line: found.line };
    const there = blockOf(index, found.line);
    const home = blockOf(index, want);
    const twin = fieldName(index, found.line) === fieldName(index, want) && there !== undefined && home !== undefined && sameText(index, there, home);
    return twin ? { verdict: "near", line: found.line } : miss;
  };

  /** The types a node must have to have the field on `field` as a child of
   *  its own: the name of the structure that declares it, and the names of
   *  types written as `same as type` that one. Null when the field may also
   *  be a child of some other structure: one declared in an arm of a switch
   *  that has no field of its own, whose arms' fields are the fields of the
   *  structure around it. */
  const holderTypes = (field: number): Set<string> | null => {
    const b = blockOf(index, field);
    if (b === undefined) return null;
    const choice = index.armOf.get(b.open);
    let open = choice?.line;
    while (open !== undefined && index.armOf.has(open)) open = index.armOf.get(open)?.line;
    if (open !== undefined && index.kinds[open] !== "field" && indentOf(index.lines[open] ?? "") > 0) return null;
    const names = new Set([b.name]);
    for (const [alias, of] of index.alias) if (of === b.name) names.add(alias);
    return names;
  };

  /** Holder types for any of several fields; null when any one is null. */
  const anyHolder = (steps: readonly Step[]): Set<string> | null => {
    const out = new Set<string>();
    for (const s of steps) {
      const t = holderTypes(s.line);
      if (t === null) return null;
      for (const n of t) out.add(n);
    }
    return out;
  };

  const visit = (path: number[], node: JumpNode, steps: readonly Step[]): number[] | null => {
    if (node.list) {
      const count = node.childCount;
      const first = Math.min(count, tried);
      const own = cursor !== null && cursor.length > path.length && path.every((p, k) => cursor[k] === p) ? (cursor[path.length] ?? -1) : -1;
      const order: [number, JumpNode | null | "pending"][] = [];
      if (own >= 0 && own < count) order.push([own, tree.node([...path, own])]);
      if (first > 0) {
        const els = tree.children(path, 0, first);
        if (els === "pending") pending = true;
        else if (els !== null) els.forEach((e, k) => k !== own && order.push([k, e]));
      }
      const types = anyHolder(steps);
      /** The types of the elements passed over for their type, while they
       *  are the only kind seen. */
      const passed = new Set<string>();
      let entered = false;
      for (const [k, e] of order) {
        if (e === "pending") pending = true;
        if (e === null || e === "pending" || e.absent) continue;
        // An element of another structure has no field of that name. Only a
        // list inside a list is gone into whatever its type.
        if (!e.list && types !== null && !types.has(e.type)) {
          passed.add(e.type);
          continue;
        }
        entered = true;
        const hit = visit([...path, k], e, steps);
        if (hit !== null) return hit;
      }
      // A list whose every element tried was one other structure is taken to
      // be all that structure: 245,678 instructions hold no symbol.
      if (count > first && (entered || passed.size !== 1)) truncated ??= { list: node.name, tried };
      return null;
    }
    if (node.childCount === 0) return null;
    const kids = tree.children(path, 0, Math.min(node.childCount, FIELDS_READ));
    if (kids === "pending") {
      pending = true;
      return null;
    }
    if (kids === null) return null;
    if (node.childCount > FIELDS_READ) truncated ??= { list: "", tried: FIELDS_READ };
    // A field `named by` one of its own fields is called `body __TEXT`.
    const isField = (c: JumpNode, s: Step): boolean => c.name === s.name || c.name.startsWith(`${s.name} `);
    for (let k = 0; k < kids.length; k++) {
      const c = kids[k]!;
      if (c.absent) continue;
      for (const s of steps) {
        if (!isField(c, s)) continue;
        const at = [...path, k];
        const { verdict, line: found } = judge(at, s.line);
        if (verdict === "miss") continue;
        if (s.line === line) {
          if (verdict === "hit") return at;
          near ??= { path: at, line: found };
        }
        if (s.next.length > 0) {
          const hit = visit(at, c, s.next);
          if (hit !== null) return hit;
        }
      }
    }
    for (let k = 0; k < kids.length; k++) {
      const c = kids[k]!;
      if (c.name !== node.name || c.absent || steps.some((s) => isField(c, s))) continue;
      const hit = visit([...path, k], c, steps);
      if (hit !== null) return hit;
    }
    return null;
  };

  const hit = visit([], root, stepsOf(routes, (l) => fieldName(index, l)));
  if (hit !== null) return { kind: "found", path: hit, line };
  if (near !== null) {
    const n: { path: number[]; line: number } = near;
    return { kind: "found", path: n.path, line: n.line };
  }
  if (pending) return { kind: "pending" };
  // Set inside `visit`, which the compiler does not follow.
  const short = truncated as { list: string; tried: number } | null;
  if (short !== null) return { kind: "gaveUp", list: short.list, tried: short.tried };
  return { kind: "absent" };
}
