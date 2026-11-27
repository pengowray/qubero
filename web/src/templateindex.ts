// The template text (`doc.templateText()`, written by the core's
// `template_text.rs`) read back just far enough to colour it and to find the
// line that declares a field.
//
// The notation is not parsed. What is used is how the core lays it out: a
// structure's fields sit two spaces deeper than the line that opens it, a line
// too long for 96 columns carries on four spaces deeper than its own start,
// and a block opens with `{` at the end of its last line and closes on a line
// back at the opener's indentation. Indentation decides everything here, never
// counting braces, because braces also turn up inside quoted text
// (`worded "{} bytes"`).

/** What a line is, as far as colouring goes. */
export type LineKind = "field" | "comment" | "plain";

/** One structure block: `root X {`, `type X {`, `field: X (...) {`, `3 => X {`. */
export type Block = {
  /** The structure's own name, which is what a node's `type` says. */
  readonly name: string;
  /** First line of the statement that opens it. */
  readonly open: number;
  /** Last line it covers, the closing `}` included. */
  end: number;
  /** Its own fields, not those of structures nested inside: name to line. */
  readonly fields: Map<string, number>;
  /** True for one written at the top level, after `type` or `root`. */
  readonly top: boolean;
};

export type TemplateIndex = {
  readonly lines: readonly string[];
  readonly kinds: readonly LineKind[];
  /** Per line, the column ranges of names declared there: `type X`, `root X`
   *  and the name in front of a structure's `{`. */
  readonly decls: readonly (readonly (readonly [number, number])[])[];
  readonly blocks: readonly Block[];
  readonly byName: ReadonlyMap<string, readonly Block[]>;
  /** A top-level `type X ...` statement: the lines it covers. */
  readonly typeSpans: ReadonlyMap<string, readonly [number, number]>;
  /** The block `root X {` opens, when the root is a structure. */
  readonly root: Block | null;
  /** Per line that starts a statement, the last line of that statement. */
  readonly stmtEnd: ReadonlyMap<number, number>;
  /** Every `2 => same as case 1` line, and every `type B = same as type A`
   *  line: the arm or type it points at and, for one that ends
   *  `but big-endian` or `but little-endian`, which. */
  readonly sameAs: readonly SameAs[];
  /** A type written as `type B = same as type A`, to the type it names. */
  readonly alias: ReadonlyMap<string, string>;
};

/** One `same as case` or `same as type` line. `target` is the first line of
 *  the arm or type it names, null when there is no such arm above it or no
 *  such type. */
export type SameAs = { readonly line: number; readonly target: number | null; readonly order: ByteOrder | null };

export type ByteOrder = "big" | "little";

/** `arms` is set on the frame a `switch` or `match` opens: each arm's key, to
 *  the arm's first line. */
type Frame = { contentIndent: number; block: Block | null; arms: Map<string, number> | null };

/** The keys an arm's statement starts with and what follows its `=>`, or null
 *  for a statement that is not an arm. Several keys are written with commas
 *  between them, `30, 38, 41 => same as case 29`, and a quoted key may hold a
 *  comma or ` => ` itself. */
function armOf(stmt: string): { keys: string[]; rest: string } | null {
  const keys: string[] = [];
  let from = 0;
  let i = 0;
  while (i < stmt.length) {
    const c = stmt[i];
    if (c === '"' || c === "'") {
      i++;
      while (i < stmt.length && stmt[i] !== c) i += stmt[i] === "\\" ? 2 : 1;
      i++;
      continue;
    }
    if (stmt.startsWith(", ", i)) {
      keys.push(stmt.slice(from, i));
      i += 2;
      from = i;
      continue;
    }
    if (stmt.startsWith(" => ", i)) {
      if (i === from) return null;
      keys.push(stmt.slice(from, i));
      return { keys, rest: stmt.slice(i + 4) };
    }
    // A key is one word or one quoted string, so a space or a bracket before
    // the `=>` means this is not an arm.
    if (c === " " || c === "(" || c === "{" || c === ":") return null;
    i++;
  }
  return null;
}

const SAME_AS = /^same as case (.+?)(?:, but (big|little)-endian)?$/;
const SAME_TYPE = /^type (\S+) = same as type (\S+?)(?:, but (big|little)-endian)?$/;

const indentOf = (s: string): number => s.length - s.trimStart().length;

/** Where each bracket group at the top level of `s` starts and ends, skipping
 *  brackets inside quotes. Quotes that do not close on the line are taken as
 *  characters, since an apostrophe can be part of an enum's label. */
function topGroups(s: string): [number, number][] {
  const out: [number, number][] = [];
  let depth = 0;
  let start = -1;
  for (let i = 0; i < s.length; i++) {
    const c = s[i];
    if (c === '"' || c === "'") {
      const close = s.indexOf(c, i + 1);
      if (close > i) {
        i = close;
        continue;
      }
    }
    if (c === "(") {
      if (depth === 0) start = i;
      depth++;
    } else if (c === ")" && depth > 0) {
      depth--;
      if (depth === 0) out.push([start, i]);
    }
  }
  return out;
}

/** What an opening statement opens, from its text without the final `{`. A
 *  structure gives its name and where that name starts in `body`. */
function opener(body: string): { kind: "struct"; name: string; at: number } | { kind: "choice" } | { kind: "other" } {
  const groups = topGroups(body);
  let outside = "";
  let from = 0;
  for (const [a, b] of groups) {
    outside += body.slice(from, a) + " ";
    from = b + 1;
  }
  outside += body.slice(from);
  if (/(^|[\s:=(])(switch|match)\s/.test(outside)) return { kind: "choice" };
  if (/(^|\s)(enum|flags)\s+\S+\s*$/.test(outside)) return { kind: "other" };
  let end = body.trimEnd().length;
  const last = groups.at(-1);
  if (last !== undefined && last[1] === end - 1) end = body.slice(0, last[0]).trimEnd().length;
  const m = /[^\s()[\]{}:,]+$/.exec(body.slice(0, end));
  if (m === null) return { kind: "other" };
  return { kind: "struct", name: m[0], at: m.index };
}

/** Read the layout of a whole template text once. */
export function indexTemplate(text: string): TemplateIndex {
  const lines = text.replace(/\n$/, "").split("\n");
  const ln = (i: number): string => lines[i] ?? "";
  const kinds: LineKind[] = lines.map(() => "plain");
  const decls: [number, number][][] = lines.map(() => []);
  const blocks: Block[] = [];
  const byName = new Map<string, Block[]>();
  const typeSpans = new Map<string, readonly [number, number]>();
  const stmtEnd = new Map<number, number>();
  const sameAs: SameAs[] = [];
  const alias = new Map<string, string>();
  /** `same as type` lines, whose type may be written further down. */
  const sameType: { line: number; target: string; order: ByteOrder | null }[] = [];
  let root: Block | null = null;
  const stack: Frame[] = [];
  /** The statement being read: its first line and kind. */
  let stmt = -1;
  let stmtKind: LineKind = "plain";
  /** The arms of the choice the statement is an arm of, when it starts at
   *  that choice's arm indentation. */
  let stmtArms: Map<string, number> | null = null;
  let topType: { name: string; start: number } | null = null;

  const closeTopType = (last: number): void => {
    if (topType !== null) typeSpans.set(topType.name, [topType.start, last]);
    topType = null;
  };
  /** The statement ends at `last`; open a block if it ended with `{`. */
  const finish = (last: number): void => {
    if (stmt < 0) return;
    stmtEnd.set(stmt, last);
    const first = stmt;
    stmt = -1;
    if (stmtKind === "comment") return;
    // The statement joined back up, remembering where each line starts in it.
    const starts: number[] = [];
    let joined = "";
    for (let i = first; i <= last; i++) {
      if (i > first) joined += " ";
      starts.push(joined.length);
      joined += ln(i).trim();
    }
    const typed = indentOf(ln(first)) === 0 ? SAME_TYPE.exec(joined) : null;
    if (typed !== null) {
      const [, name = "", target = "", order] = typed;
      alias.set(name, target);
      sameType.push({ line: first, target, order: order === "big" || order === "little" ? order : null });
    }
    if (stmtArms !== null) {
      const arm = armOf(joined);
      if (arm !== null) {
        const same = SAME_AS.exec(arm.rest.trimEnd());
        if (same !== null) {
          const order = same[2] === "big" || same[2] === "little" ? same[2] : null;
          sameAs.push({ line: first, target: stmtArms.get(same[1] ?? "") ?? null, order });
        }
        for (const k of arm.keys) if (!stmtArms.has(k)) stmtArms.set(k, first);
      }
    }
    if (!ln(last).trimEnd().endsWith("{")) return;
    const ind = indentOf(ln(first));
    const body = joined.replace(/\{\s*$/, "");
    const o = opener(body);
    let block: Block | null = null;
    if (o.kind === "struct") {
      const top = ind === 0 && /^(type|root) /.test(joined);
      block = { name: o.name, open: first, end: last, fields: new Map(), top };
      blocks.push(block);
      const list = byName.get(o.name);
      if (list === undefined) byName.set(o.name, [block]);
      else list.push(block);
      if (top && joined.startsWith("root ")) root ??= block;
      // Back to a line and a column for colouring.
      let li = starts.length - 1;
      while (li > 0 && (starts[li] ?? 0) > o.at) li--;
      const col = indentOf(ln(first + li)) + (o.at - (starts[li] ?? 0));
      decls[first + li]?.push([col, col + o.name.length]);
    }
    stack.push({ contentIndent: ind + 2, block, arms: o.kind === "choice" ? new Map() : null });
  };

  for (let i = 0; i < lines.length; i++) {
    const line = ln(i);
    if (line.trim() === "") {
      finish(i - 1);
      continue;
    }
    const d = indentOf(line);
    const ci = stack.at(-1)?.contentIndent ?? 0;
    if (stmt >= 0 && d > ci) {
      // Carried on from the line above.
      kinds[i] = stmtKind === "comment" ? "comment" : "plain";
      if (stmtKind !== "comment" && line.trimEnd().endsWith("{")) finish(i);
      continue;
    }
    finish(i - 1);
    while (d < (stack.at(-1)?.contentIndent ?? 0)) {
      const f = stack.pop()!;
      if (f.block !== null) f.block.end = line.trimStart().startsWith("}") && d === f.contentIndent - 2 ? i : i - 1;
    }
    if (d === 0) {
      closeTopType(i - 1);
      const t = /^(type|root) ([^\s({=]+)/.exec(line) as [string, string, string] | null;
      if (t !== null) {
        if (t[1] === "type") topType = { name: t[2], start: i };
        decls[i]?.push([t[1].length + 1, t[1].length + 1 + t[2].length]);
      }
    }
    stmt = i;
    const trimmed = line.trimStart();
    const top = stack.at(-1);
    if (trimmed.startsWith("//")) stmtKind = "comment";
    else if (top?.block != null && d === top.contentIndent && /^[^\s:]+: /.test(trimmed)) {
      stmtKind = "field";
      const name = trimmed.slice(0, trimmed.indexOf(": "));
      if (!top.block.fields.has(name)) top.block.fields.set(name, i);
    } else stmtKind = "plain";
    stmtArms = top?.arms != null && d === top.contentIndent ? top.arms : null;
    kinds[i] = stmtKind;
    if (stmtKind !== "comment" && line.trimEnd().endsWith("{")) finish(i);
  }
  finish(lines.length - 1);
  while (stack.length > 0) {
    const f = stack.pop()!;
    if (f.block !== null) f.block.end = lines.length - 1;
  }
  closeTopType(lines.length - 1);
  for (const t of sameType) sameAs.push({ line: t.line, target: typeSpans.get(t.target)?.[0] ?? null, order: t.order });
  // A name declared twice on one line is coloured once.
  for (const d of decls) {
    const seen = new Set<number>();
    for (let k = d.length - 1; k >= 0; k--) {
      const at = d[k]?.[0] ?? -1;
      if (seen.has(at)) d.splice(k, 1);
      else seen.add(at);
    }
    d.sort((a, b) => a[0] - b[0]);
  }
  return { lines, kinds, decls, blocks, byName, typeSpans, root, stmtEnd, sameAs, alias };
}

/** The byte order a block is also read in: the order of a `same as case N,
 *  but big-endian` line whose arm N holds the block, or null when none does.
 *  The innermost such arm decides. */
function orderOf(index: TemplateIndex, b: Block): ByteOrder | null {
  let best: { order: ByteOrder; from: number } | null = null;
  for (const s of index.sameAs) {
    if (s.order === null || s.target === null) continue;
    const [a, z] = fieldSpan(index, s.target);
    if (b.open >= a && b.open <= z && (best === null || s.target > best.from)) best = { order: s.order, from: s.target };
  }
  return best?.order ?? null;
}

/** A field's type text as the arm that says `same as case N, but big-endian`
 *  reads it: every `le` or `be` after a width wider than a byte set to that
 *  order. One byte or narrower stays as written, as it does in the core. */
function inOrder(said: string, order: ByteOrder): string {
  const to = order === "big" ? "be" : "le";
  return said.replace(/(\d+)(le|be)(?![a-z0-9])/g, (m: string, bits: string) => (Number(bits) > 8 ? bits + to : m));
}

/** The byte orders of the numbers wider than a byte in some type text, first
 *  one first: `u32be` and a node's `u32 be` are big, `u8le` is none. */
function wideOrders(said: string): ByteOrder[] {
  const out: ByteOrder[] = [];
  for (const m of said.matchAll(/(\d+) ?(le|be)(?![a-z0-9])/g)) {
    if (Number(m[1]) > 8) out.push(m[2] === "be" ? "big" : "little");
  }
  return out;
}

/** What a field's line says its type is: the text after the field's name. */
function typeText(index: TemplateIndex, line: number): string {
  const l = index.lines[line] ?? "";
  const at = l.indexOf(": ");
  return at < 0 ? "" : l.slice(at + 2);
}

/**
 * The byte order a field was read in, when that is not the order its line
 * writes: the field was read through a `same as ..., but little-endian` line
 * (an arm's or a type's) that points at the arm or type the line is in.
 * Null when the line is in no such arm or type, when it writes no number wider
 * than a byte, or when the order it was read in is the one it writes.
 *
 * The order it was read in comes from the node's own type (`u32 le`) or,
 * when that names no byte order (an enumeration's name), from the types of
 * the other fields of the same structure, which `siblings` is asked for only
 * then. A field whose type and line differ
 * in byte order can only have been read through such a line, and such a line
 * sets every number in the arm to one order, so one of them is enough.
 */
export function readAs(
  index: TemplateIndex,
  line: number,
  node: { name: string; type: string },
  siblings: () => readonly { name: string; type: string }[],
): ByteOrder | null {
  const reached = index.sameAs.some((s) => {
    if (s.order === null || s.target === null) return false;
    const [a, z] = fieldSpan(index, s.target);
    return line >= a && line <= z;
  });
  if (!reached) return null;
  const written = wideOrders(typeText(index, line));
  if (written.length === 0) return null;
  const differs = (c: { type: string }, at: number): ByteOrder | null => {
    const was = wideOrders(c.type)[0];
    const says = wideOrders(typeText(index, at))[0];
    return was !== undefined && says !== undefined && was !== says ? was : null;
  };
  let read: ByteOrder | null = null;
  if (wideOrders(node.type).length > 0) read = differs(node, line);
  else {
    // Asked for only here, since they may not have been read yet.
    const block = index.blocks.find((b) => b.fields.get(node.name) === line);
    for (const c of siblings()) {
      const at = block?.fields.get(c.name);
      read = at === undefined ? null : differs(c, at);
      if (read !== null) break;
    }
  }
  return read !== null && written.some((w) => w !== read) ? read : null;
}

/** The lines a field covers: its own statement and every line nested under it. */
export function fieldSpan(index: TemplateIndex, line: number): readonly [number, number] {
  const d = indentOf(index.lines[line] ?? "");
  let end = line;
  for (let i = line + 1; i < index.lines.length; i++) {
    const l = index.lines[i] ?? "";
    if (l.trim() === "") break;
    if (indentOf(l) <= d) {
      // A closing `}` of a block the field opened belongs to the field.
      if (indentOf(l) === d && l.trimStart().startsWith("}")) end = i;
      break;
    }
    end = i;
  }
  return [line, end];
}

/** The structures called `name`, the ones inside `scope` first. Null scope, or
 *  none inside it, falls back to those declared at the top level, then to all. */
export function blocksNamed(index: TemplateIndex, name: string, scope: readonly (readonly [number, number])[]): readonly Block[] {
  const all = index.byName.get(name) ?? [];
  if (all.length <= 1) return all;
  const inside = all.filter((b) => scope.some(([a, z]) => b.open >= a && b.open <= z));
  if (inside.length > 0) return inside;
  const top = all.filter((b) => b.top);
  return top.length > 0 ? top : all;
}

/** Where to look for the structure a field's value is: the field's own lines,
 *  and every top-level type its statement names (`value: Value` reaches the
 *  structures written inside `type Value = switch ...`). A type written as
 *  `same as type` another is looked for in that other. */
function scopeOf(index: TemplateIndex, line: number): [number, number][] {
  const out: [number, number][] = [[...fieldSpan(index, line)]];
  const last = index.stmtEnd.get(line) ?? line;
  const words = index.lines.slice(line, last + 1).join(" ").match(/[A-Za-z_][\w.]*/g) ?? [];
  for (const w of words) {
    const span = index.typeSpans.get(index.alias.get(w) ?? w);
    if (span !== undefined) out.push([span[0], span[1]]);
  }
  return out;
}

/** One node on the way down from the root to the cursor. */
export type ChainNode = { readonly name: string; readonly type: string; readonly list: boolean };

/** What a field's type text says first, spaces dropped: `u64 le` is `u64le`. */
const squash = (s: string): string => s.replace(/\s+/g, "");

/**
 * Of several structures that could be this node, the one its children fit
 * best: each child whose name is a field of the block counts, and counts again
 * when the field's type text names the child's type. Null when two fit
 * equally well, since picking one of them would be a guess.
 *
 * A block inside an arm that a `same as case N, but big-endian` line points at
 * is also the block of that line's own arm, so its types are compared a
 * second time as that arm reads them, and the better of the two counts. That
 * is what finds the written-out little-endian header for a big-endian file.
 */
export function bestFit(index: TemplateIndex, blocks: readonly Block[], children: readonly { name: string; type: string }[]): Block | null {
  let best: Block | null = null;
  let bestScore = -1;
  let tied = false;
  for (const b of blocks) {
    const order = orderOf(index, b);
    let score = 0;
    let other = 0;
    for (const c of children) {
      const at = b.fields.get(c.name);
      if (at === undefined) continue;
      score++;
      other++;
      const line = index.lines[at] ?? "";
      const said = squash(line.slice(line.indexOf(": ") + 2));
      const t = squash(c.type);
      if (t === "") continue;
      if (said.startsWith(t)) score++;
      if (order !== null && inOrder(said, order).startsWith(t)) other++;
    }
    score = Math.max(score, other);
    if (score > bestScore) {
      best = b;
      bestScore = score;
      tied = false;
    } else if (score === bestScore) tied = true;
  }
  return tied ? null : best;
}

/**
 * The line that declares the deepest node in `chain` that has a line of its
 * own, or null when not even the first field below the root can be found.
 *
 * `chain` runs from the root to the node at the cursor. A list's elements, and
 * a node that repeats its parent's name (a field placed elsewhere is a node
 * for the placing and a node for what is placed), have no line of their own
 * and give the nearest enclosing field's line. Where the parent's type names
 * more than one structure that has the field (ELF writes `ELFHeader` once for
 * each word size and byte order), `children(depth)` is asked for the parent's
 * children to choose between them; when they cannot choose, the search stops
 * at the line it already has.
 *
 * A node read by an arm written as `2 => same as case 1` has no line of its
 * own and is found in arm 1, since that arm's structures are the only ones
 * with its name. One read by `same as case 1, but big-endian` is found there
 * too, through [`bestFit`].
 */
export function findLine(
  index: TemplateIndex,
  chain: readonly ChainNode[],
  children: (depth: number) => readonly { name: string; type: string }[] | null,
): number | null {
  return findLineAt(index, chain, children)?.line ?? null;
}

/** [`findLine`]'s line, and the place in `chain` of the node it declares. */
export function findLineAt(
  index: TemplateIndex,
  chain: readonly ChainNode[],
  children: (depth: number) => readonly { name: string; type: string }[] | null,
): { line: number; depth: number } | null {
  const head = chain[0];
  if (head === undefined) return null;
  let found: { line: number; depth: number } | null = null;
  let scope: [number, number][] = [];
  let blocks: readonly Block[] = index.root !== null && index.root.name === head.type ? [index.root] : blocksNamed(index, head.type, []);
  for (let i = 1; i < chain.length; i++) {
    const n = chain[i];
    const parent = chain[i - 1];
    if (n === undefined || parent === undefined) break;
    if (!parent.list && blocks.length > 0) {
      const having = blocks.filter((b) => b.fields.has(n.name));
      let chosen: Block | null = having.length === 1 ? (having[0] ?? null) : null;
      if (having.length > 1) {
        const kids = children(i - 1);
        chosen = kids === null ? null : bestFit(index, having, kids);
        if (chosen === null) return found;
      }
      if (chosen !== null) {
        found = { line: chosen.fields.get(n.name)!, depth: i };
        scope = scopeOf(index, found.line);
      }
    }
    blocks = blocksNamed(index, n.type, scope);
  }
  return found;
}

/** A run of a line with one colour. `cls` is empty for plain text. */
export type Segment = { readonly text: string; readonly cls: "" | "kw" | "decl" | "field" | "comment" };

/** Words of the notation that are always its own, wherever they appear. */
const KEYWORDS = new Set(["switch", "match", "enum", "flags", "from", "until", "while", "when", "aside", "check", "over", "unset", "origin", "valid"]);
/** Words of the notation only when a bracket follows: `at(`, `sized(`. A
 *  field may well be called `chain`. */
const CALL_KEYWORDS = new Set(["sized", "sizedbits", "optional", "at", "decoded", "repeat", "pointers", "chain", "gather", "stitched", "raster", "schema"]);
/** Words of the notation at the start of a line. */
const LEAD_KEYWORDS = new Set(["template", "root", "type", "deducer"]);

/**
 * One line cut into coloured runs. `kind` and `decls` come from the index: a
 * field's name is coloured only on a line the index says declares a field,
 * since `name: ` also appears in a wrapped expression's ternary.
 */
export function tokenizeLine(text: string, kind: LineKind, decls: readonly (readonly [number, number])[] = []): Segment[] {
  if (kind === "comment") return [{ text, cls: "comment" }];
  const marks: { at: number; end: number; cls: Segment["cls"] }[] = [];
  for (const [a, z] of decls) marks.push({ at: a, end: z, cls: "decl" });
  const lead = indentOf(text);
  if (kind === "field") {
    const colon = text.indexOf(": ", lead);
    if (colon > lead) marks.push({ at: lead, end: colon, cls: "field" });
  }
  // Words outside quotes.
  const word = /"[^"]*"|'[^']*'|[A-Za-z_][\w]*/g;
  for (let m = word.exec(text); m !== null; m = word.exec(text)) {
    const w = m[0];
    if (w.startsWith('"') || w.startsWith("'")) continue;
    const at = m.index;
    const end = at + w.length;
    const before = text[at - 1] ?? " ";
    if (before === "." || /[\w]/.test(before)) continue;
    const next = text[end] ?? "";
    const isKw =
      (at === lead && lead === 0 && LEAD_KEYWORDS.has(w)) ||
      (CALL_KEYWORDS.has(w) && next === "(") ||
      (KEYWORDS.has(w) && next !== "." && next !== "[" && next !== ":" && next !== "(");
    if (isKw) marks.push({ at, end, cls: "kw" });
  }
  marks.sort((a, b) => a.at - b.at);
  const out: Segment[] = [];
  let pos = 0;
  for (const m of marks) {
    if (m.at < pos) continue;
    if (m.at > pos) out.push({ text: text.slice(pos, m.at), cls: "" });
    out.push({ text: text.slice(m.at, m.end), cls: m.cls });
    pos = m.end;
  }
  if (pos < text.length) out.push({ text: text.slice(pos), cls: "" });
  return out;
}
