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
/** `kind` is a line of a `decoder` section that names a kind of trace node,
 *  or a field of a kind's record: coloured as a field is, but not a field of
 *  the template, so it cannot be clicked. */
export type LineKind = "field" | "comment" | "plain" | "kind";

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
  /** The opening statement from the name on, lines joined: `Body (contents
   *  contents)` for `0xffe2 => Body (contents contents) {`. */
  readonly head: string;
};

/** One `switch` or `match` block. */
export type Choice = {
  /** First line of the statement that opens it. */
  readonly line: number;
  /** The field it reads, when that is one name: `marker` in `switch marker`.
   *  Null for an expression, `switch peek(u32be)`. */
  readonly on: string | null;
  /** Each arm's key, as written, to the arm's first line. `_` is the default. */
  readonly arms: Map<string, number>;
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
  /** The line `root ...` is on, or null when there is none. */
  readonly rootLine: number | null;
  /** Per line that starts a statement, the last line of that statement. */
  readonly stmtEnd: ReadonlyMap<number, number>;
  /** Every `2 => same as case 1` line, and every `type B = same as type A`
   *  line: the arm or type it points at and, for one that ends
   *  `but big-endian` or `but little-endian`, which. */
  readonly sameAs: readonly SameAs[];
  /** A type written as `type B = same as type A`, to the type it names. */
  readonly alias: ReadonlyMap<string, string>;
  /** An arm's first line, to the switch or match it is an arm of. */
  readonly armOf: ReadonlyMap<number, Choice>;
  /** A structure's first line, to the structure. */
  readonly opens: ReadonlyMap<number, Block>;
  /** Each `decoder ... {` section: its first line and its closing `}`. Nothing
   *  inside one is a block, a field, a statement or part of a type's span. */
  readonly decoders: readonly (readonly [number, number])[];
};

/** One `same as case` or `same as type` line. `target` is the first line of
 *  the arm or type it names, null when there is no such arm above it or no
 *  such type. */
export type SameAs = { readonly line: number; readonly target: number | null; readonly order: ByteOrder | null };

export type ByteOrder = "big" | "little";

/** `choice` is set on the frame a `switch` or `match` opens. */
type Frame = { contentIndent: number; block: Block | null; choice: Choice | null };

/** The keys an arm's statement starts with and what follows its `=>`, or null
 *  for a statement that is not an arm. Several keys are written with commas
 *  between them, `30, 38, 41 => same as case 29`, and a quoted key may hold a
 *  comma or ` => ` itself. */
function armKeys(stmt: string): { keys: string[]; rest: string } | null {
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
/** The one name a choice reads, at the end of its statement without the `{`. */
const CHOICE_ON = /(?:^|[\s:=(])(?:switch|match) ([A-Za-z_]\w*)\s*$/;

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
  const armOf = new Map<number, Choice>();
  const opens = new Map<number, Block>();
  /** `same as type` lines, whose type may be written further down. */
  const sameType: { line: number; target: string; order: ByteOrder | null }[] = [];
  let root: Block | null = null;
  let rootLine: number | null = null;
  const stack: Frame[] = [];
  /** The statement being read: its first line and kind. */
  let stmt = -1;
  let stmtKind: LineKind = "plain";
  /** The choice the statement is an arm of, when it starts at that choice's
   *  arm indentation. */
  let stmtChoice: Choice | null = null;
  let topType: { name: string; start: number } | null = null;
  const decoders: [number, number][] = [];
  /** The decoder section being read. */
  let section: [number, number] | null = null;

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
    if (stmtChoice !== null) {
      const arm = armKeys(joined);
      if (arm !== null) {
        const arms = stmtChoice.arms;
        const same = SAME_AS.exec(arm.rest.trimEnd());
        if (same !== null) {
          const order = same[2] === "big" || same[2] === "little" ? same[2] : null;
          sameAs.push({ line: first, target: arms.get(same[1] ?? "") ?? null, order });
        }
        for (const k of arm.keys) if (!arms.has(k)) arms.set(k, first);
        armOf.set(first, stmtChoice);
      }
    }
    if (!ln(last).trimEnd().endsWith("{")) return;
    const ind = indentOf(ln(first));
    const body = joined.replace(/\{\s*$/, "");
    const o = opener(body);
    let block: Block | null = null;
    if (o.kind === "struct") {
      const top = ind === 0 && /^(type|root) /.test(joined);
      block = { name: o.name, open: first, end: last, fields: new Map(), top, head: body.slice(o.at).trimEnd() };
      blocks.push(block);
      opens.set(first, block);
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
    const choice = o.kind === "choice" ? { line: first, on: CHOICE_ON.exec(body)?.[1] ?? null, arms: new Map<string, number>() } : null;
    stack.push({ contentIndent: ind + 2, block, choice });
  };

  for (let i = 0; i < lines.length; i++) {
    const line = ln(i);
    // A decoder section describes what a decoder lays down, not the
    // template's own fields, and runs to the `}` at the start of a line.
    if (section !== null) {
      section[1] = i;
      if (line === "}") section = null;
      else if (/^\s+[a-z_]+: /.test(line)) kinds[i] = "kind";
      continue;
    }
    if (line.startsWith("decoder ")) {
      finish(i - 1);
      while (stack.length > 0) {
        const f = stack.pop()!;
        if (f.block !== null) f.block.end = i - 1;
      }
      closeTopType(i - 1);
      section = [i, i];
      decoders.push(section);
      continue;
    }
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
      if (line.startsWith("root ")) rootLine ??= i;
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
    stmtChoice = top?.choice != null && d === top.contentIndent ? top.choice : null;
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
  return { lines, kinds, decls, blocks, byName, typeSpans, root, rootLine, stmtEnd, sameAs, alias, armOf, opens, decoders };
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
    const block = index.blocks.find((b) => b.fields.get(fieldOf(node.name)) === line);
    for (const c of siblings()) {
      const at = block?.fields.get(fieldOf(c.name));
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
 *  none inside it, falls back to those declared at the top level, then to all.
 *  With `strict` there is no falling back: only the ones inside `scope`. */
export function blocksNamed(index: TemplateIndex, name: string, scope: readonly (readonly [number, number])[], strict = false): readonly Block[] {
  const all = index.byName.get(name) ?? [];
  const within = (b: Block): boolean => scope.some(([a, z]) => b.open >= a && b.open <= z);
  if (strict) return all.filter(within);
  if (all.length <= 1) return all;
  const inside = all.filter(within);
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

/** One child of a node. `value` and `edit` are the node's `value` and
 *  `edit_text`, which say which arm a switch on this field took. */
export type Kid = { readonly name: string; readonly type: string; readonly value?: string; readonly edit?: string };

/** The children of the node at `depth` in the chain, or null when they are not
 *  known yet. */
export type Children = (depth: number) => readonly Kid[] | null;

/** The field name a node's name is for. A structure `named by` one of its own
 *  fields gives its node the field's name and that field's value, `body
 *  __TEXT` for the field `body`, and a field's name in the text has no spaces. */
export function fieldOf(name: string): string {
  const sp = name.indexOf(" ");
  return sp < 0 ? name : name.slice(0, sp);
}

/** The count after a structure's closing `}` when the structure is written as
 *  a list's element, `[remaining / 12]` for `}[remaining / 12]`, or null. */
function countOf(index: TemplateIndex, b: Block): string | null {
  return /^\s*\}(\[.*\])/.exec(index.lines[b.end] ?? "")?.[1] ?? null;
}

/** What a field's type text says first, spaces dropped: `u64 le` is `u64le`. */
const squash = (s: string): string => s.replace(/\s+/g, "");

/**
 * Of several structures that could be this node, the ones its children fit
 * best: each child whose name is a field of the block counts, and counts again
 * when the field's type text names the child's type, or when the field opens a
 * structure of the child's type's name (`contents: sized(length - 2) App2 {`).
 *
 * A block inside an arm that a `same as case N, but big-endian` line points at
 * is also the block of that line's own arm, so its types are compared a
 * second time as that arm reads them, and the better of the two counts. That
 * is what finds the written-out little-endian header for a big-endian file.
 */
function fits(index: TemplateIndex, blocks: readonly Block[], children: readonly { name: string; type: string }[]): Block[] {
  let best: Block[] = [];
  let bestScore = -1;
  for (const b of blocks) {
    const order = orderOf(index, b);
    let score = 0;
    let other = 0;
    for (const c of children) {
      const at = b.fields.get(fieldOf(c.name));
      if (at === undefined) continue;
      score++;
      other++;
      const line = index.lines[at] ?? "";
      const said = squash(line.slice(line.indexOf(": ") + 2));
      const t = squash(c.type);
      if (t === "") continue;
      if (index.opens.get(at)?.name === t) {
        score++;
        other++;
        continue;
      }
      if (said.startsWith(t)) score++;
      if (order !== null && inOrder(said, order).startsWith(t)) other++;
    }
    score = Math.max(score, other);
    if (score > bestScore) {
      best = [b];
      bestScore = score;
    } else if (score === bestScore) best.push(b);
  }
  return best;
}

/** A number as a key or a value writes it: `12`, `-3`, `0xffe2`. */
export function numberOf(s: string): bigint | null {
  const t = s.trim();
  return /^-?\d+$/.test(t) || /^0x[0-9a-f]+$/i.test(t) ? BigInt(t) : null;
}

/** Text without the quotes around it, when it has them. Spaces inside the
 *  quotes are kept: `'XYZ '` is four characters. */
export function unquote(t: string): string {
  if (t.length < 2 || (t[0] !== '"' && t[0] !== "'") || t.at(-1) !== t[0]) return t;
  return t.slice(1, -1).replace(/\\(.)/g, "$1");
}

/** An arm's first line or, for an arm written as `same as case N`, the first
 *  line of arm N. */
export function armTarget(index: TemplateIndex, line: number): number {
  for (let hop = 0; hop < 8; hop++) {
    const same = index.sameAs.find((s) => s.line === line && s.target !== null);
    if (same?.target == null) return line;
    line = same.target;
  }
  return line;
}

/** The number a quoted key of one to eight characters is as bytes, read
 *  big-endian and little-endian: `'curv'` is 0x63757276 or 0x76727563. */
export function keyNumbers(s: string): bigint[] {
  if (s.length < 1 || s.length > 8 || [...s].some((c) => c.charCodeAt(0) > 0xff)) return [];
  let be = 0n;
  let le = 0n;
  for (let k = 0; k < s.length; k++) {
    be = (be << 8n) | BigInt(s.charCodeAt(k));
    le = (le << 8n) | BigInt(s.charCodeAt(s.length - 1 - k));
  }
  return [be, le];
}

/**
 * The first line of the arm `choice` takes when the field it reads holds
 * `kid`'s value, through [`armTarget`]. A number in decimal or hex is
 * compared with the value's number (an enumeration's value ends with its
 * number in brackets, `app2 (0xffe2)`). A key in quotes is compared with the
 * value as text and, when the value has a number, with that number as bytes
 * ([`keyNumbers`]). Any other key is compared with the value as text.
 *
 * Null when two arms match, or when no arm matches and it is not certain the
 * default arm `_` was taken: there is none, or a key could not be compared
 * with the value. A key compared only as text that does not match is one that
 * could not be compared, since a value shown as text may be cut short or
 * escaped.
 */
export function takenArm(index: TemplateIndex, choice: Choice, kid: Kid): number | null {
  const texts = [kid.value, kid.edit].filter((t): t is string => t !== undefined);
  // An enumeration's label may be a number of its own (`{1 = 16}` shows
  // `16 (0x1)`), so the bracketed number, when there is one, is the only one.
  const bracketed = numberOf(/\((-?\d+|0x[0-9a-f]+)\)$/i.exec(kid.value?.trim() ?? "")?.[1] ?? "");
  const nums = bracketed !== null ? [bracketed] : texts.map(numberOf).filter((n): n is bigint => n !== null);
  const strs = texts.flatMap((t) => [t, unquote(t)]);
  const hits = new Set<number>();
  let sure = true;
  for (const [key, line] of choice.arms) {
    if (key === "_") continue;
    if (key.startsWith('"') || key.startsWith("'")) {
      const text = unquote(key);
      const asNumbers = nums.length > 0 ? keyNumbers(text) : [];
      if (strs.includes(text) || asNumbers.some((n) => nums.includes(n))) hits.add(line);
      else if (asNumbers.length === 0) sure = false;
      continue;
    }
    const n = numberOf(key);
    if (n !== null && nums.length > 0) {
      if (nums.includes(n)) hits.add(line);
    } else if (n === null && strs.includes(key)) hits.add(line);
    else sure = false;
  }
  if (hits.size > 0) {
    const [only] = hits;
    return hits.size === 1 && only !== undefined ? armTarget(index, only) : null;
  }
  const other = choice.arms.get("_");
  return sure && other !== undefined ? armTarget(index, other) : null;
}

/**
 * Whether the arms that lead to `b` are the ones taken. The arm whose
 * statement opens `b` is checked against the field its choice reads, then that
 * choice's own arm when the choice is itself an arm (`1 => switch data {`),
 * and so on out to a statement that is not an arm. That statement must be one
 * of `anchors`, the field `b` was reached through or a type it names. A switch
 * written anywhere else reads other fields than the ones `field` finds.
 *
 * `field(name)` is the field a choice reads, or null when it is not known.
 * True when at least one arm's key was compared with a value and every
 * compared arm was taken, false when a compared arm was not taken, null when
 * no arm could be compared.
 */
function armsAgree(index: TemplateIndex, b: Block, anchors: readonly number[], field: (name: string) => Kid | null): boolean | null {
  const path: [Choice, number][] = [];
  let line = b.open;
  for (let choice = index.armOf.get(line); choice !== undefined; choice = index.armOf.get(line)) {
    path.push([choice, line]);
    line = choice.line;
  }
  if (!anchors.includes(line)) return null;
  let compared = false;
  for (const [choice, arm] of path) {
    const kid = choice.on === null ? null : field(choice.on);
    const taken = kid === null ? null : takenArm(index, choice, kid);
    if (taken !== null) {
      if (taken !== arm) return false;
      compared = true;
    }
  }
  return compared ? true : null;
}

/** A field a switch above `chain[depth]` reads: the nearest field called
 *  `name` among the children of the nodes that hold `chain[depth]`, nearest
 *  first. Null when it is not found, or when a level on the way is not known
 *  yet. */
function fieldAbove(chain: readonly ChainNode[], children: Children, depth: number, name: string): Kid | null {
  for (let d = depth - 1; d >= 0; d--) {
    if (chain[d]?.list) continue;
    const kids = children(d);
    if (kids === null) return null;
    const kid = kids.find((k) => fieldOf(k.name) === name);
    if (kid !== undefined) return kid;
  }
  return null;
}

/** True when two structures are written the same from their names on. */
export function sameText(index: TemplateIndex, a: Block, b: Block): boolean {
  if (a === b) return true;
  if (a.name !== b.name || a.head !== b.head) return false;
  const fromA = (index.stmtEnd.get(a.open) ?? a.open) + 1;
  const fromB = (index.stmtEnd.get(b.open) ?? b.open) + 1;
  if (a.end - fromA !== b.end - fromB) return false;
  const da = indentOf(index.lines[a.open] ?? "");
  const db = indentOf(index.lines[b.open] ?? "");
  for (let k = 0; fromA + k <= a.end; k++) {
    if ((index.lines[fromA + k] ?? "").slice(da) !== (index.lines[fromB + k] ?? "").slice(db)) return false;
  }
  return true;
}

/**
 * Of several structures `chain[i - 1]` may be, the ones written as what that
 * node is: an element of a list, or a value on its own. A structure closed
 * with `}[count]` is written as a list's element, and one closed with `}[0]`
 * has no elements. An arm closed with `}` is a value on its own when another
 * arm of the same switch is written as a list's element. All of `blocks` are
 * kept when none is left.
 */
function shaped(index: TemplateIndex, chain: readonly ChainNode[], i: number, blocks: readonly Block[]): readonly Block[] {
  const node = chain[i - 1];
  if (node === undefined) return blocks;
  // A node placed by `at(...)` comes after a node of the same name for the
  // placing, so the list is above both.
  let j = i - 2;
  while (j >= 0 && chain[j]?.name === node.name) j--;
  const element = chain[j]?.list === true;
  const kept = blocks.filter((b) => {
    const count = countOf(index, b);
    if (!element) return count === null;
    if (count === "[0]") return false;
    if (count !== null) return true;
    const choice = index.armOf.get(b.open);
    return choice === undefined || !blocks.some((o) => index.armOf.get(o.open) === choice && countOf(index, o) !== null);
  });
  return kept.length > 0 ? kept : blocks;
}

/** A line found for the node at `depth` in the chain. */
type Found = { line: number; depth: number };

/**
 * Of several structures that have the field `chain[i]`, the one `chain[i - 1]`
 * is, or null when that cannot be told. The evidence, most trusted first:
 *
 * 1. The arms that lead to each structure, against the value of the field
 *    each `switch` or `match` reads ([`armsAgree`]), for an arm of a switch
 *    written on the field the last line was found for (the first line of
 *    `scope`) or in a type that field names.
 * 2. The children of `chain[i - 1]`, by name and type ([`fits`]).
 * 3. How far down the rest of the chain each structure leads, when each next
 *    structure is looked for only inside the field above it.
 *
 * Structures still left that are written the same are one structure written
 * twice, and the first is as right as any other.
 */
function choose(
  index: TemplateIndex,
  chain: readonly ChainNode[],
  children: Children,
  i: number,
  having: readonly Block[],
  scope: readonly (readonly [number, number])[],
): Block | null {
  let left = having;
  const anchors = scope.map(([a]) => a);
  const field = (name: string): Kid | null => fieldAbove(chain, children, i - 1, name);
  const verdicts = left.map((b) => armsAgree(index, b, anchors, field));
  const taken = left.filter((_, k) => verdicts[k] === true);
  const possible = left.filter((_, k) => verdicts[k] !== false);
  if (taken.length > 0) left = taken;
  else if (possible.length > 0) left = possible;
  if (left.length > 1) left = shaped(index, chain, i, left);
  if (left.length > 1) {
    const kids = children(i - 1);
    if (kids !== null) left = fits(index, left, kids);
  }
  if (left.length > 1) {
    const reach = left.map((b) => walk(index, chain, children, i, [b], null, [], true)?.depth ?? -1);
    const deepest = Math.max(...reach);
    left = left.filter((_, k) => reach[k] === deepest);
  }
  const first = left[0];
  return first !== undefined && left.every((b) => sameText(index, b, first)) ? first : null;
}

/** For a root written as a switch, `root switch peek(u64be) {`, the structures
 *  called `name` that are its arms or arms of switches in its arms. The value
 *  the switch reads is not a field of the file, so [`choose`] tells these
 *  apart by the root's children. */
function rootArms(index: TemplateIndex, name: string): readonly Block[] {
  const root = index.rootLine;
  if (root === null) return [];
  return (index.byName.get(name) ?? []).filter((b) => {
    let line = b.open;
    for (let choice = index.armOf.get(line); choice !== undefined; choice = index.armOf.get(line)) line = choice.line;
    return line === root && line !== b.open;
  });
}

/** [`findLineAt`] from `chain[from]` on, with `start` the structures
 *  `chain[from - 1]` may be. With `strict`, a node's structure is looked for
 *  only inside the field above it. */
function walk(
  index: TemplateIndex,
  chain: readonly ChainNode[],
  children: Children,
  from: number,
  start: readonly Block[],
  found: Found | null,
  scope: [number, number][],
  strict: boolean,
): Found | null {
  let blocks = start;
  for (let i = from; i < chain.length; i++) {
    const n = chain[i];
    const parent = chain[i - 1];
    if (n === undefined || parent === undefined) break;
    if (!parent.list && blocks.length > 0) {
      const field = fieldOf(n.name);
      const having = blocks.filter((b) => b.fields.has(field));
      let chosen: Block | null = having.length === 1 ? (having[0] ?? null) : null;
      if (having.length > 1) {
        chosen = choose(index, chain, children, i, having, scope);
        if (chosen === null) return found;
      }
      if (chosen !== null) {
        found = { line: chosen.fields.get(field)!, depth: i };
        scope = scopeOf(index, found.line);
      }
    }
    blocks = blocksNamed(index, n.type, scope, strict);
  }
  return found;
}

/**
 * The line that declares the deepest node in `chain` that has a line of its
 * own, or null when not even the first field below the root can be found.
 *
 * `chain` runs from the root to the node at the cursor. A list's elements, and
 * a node that repeats its parent's name (a field placed elsewhere is a node
 * for the placing and a node for what is placed), have no line of their own
 * and give the nearest enclosing field's line. Where the parent's type names
 * more than one structure that has the field (each arm of JPEG's `switch
 * marker` writes a `Body`), [`choose`] picks one, asking `children(depth)` for
 * the values and children it needs; when it cannot, the search stops at the
 * line it already has.
 *
 * A node read by an arm written as `2 => same as case 1` has no line of its
 * own and is found in arm 1, since that arm's structures are the only ones
 * with its name. One read by `same as case 1, but big-endian` is found there
 * too, through [`fits`].
 */
export function findLine(index: TemplateIndex, chain: readonly ChainNode[], children: Children): number | null {
  return findLineAt(index, chain, children)?.line ?? null;
}

/** [`findLine`]'s line, and the place in `chain` of the node it declares. */
export function findLineAt(index: TemplateIndex, chain: readonly ChainNode[], children: Children): Found | null {
  const head = chain[0];
  if (head === undefined) return null;
  let blocks = index.root !== null && index.root.name === head.type ? [index.root] : rootArms(index, head.type);
  if (blocks.length === 0) blocks = blocksNamed(index, head.type, []);
  return walk(index, chain, children, 1, blocks, null, [], false);
}

/** A run of a line with one colour. `cls` is empty for plain text. */
export type Segment = { readonly text: string; readonly cls: "" | "kw" | "decl" | "field" | "comment" };

/** Words of the notation that are always its own, wherever they appear. */
const KEYWORDS = new Set(["switch", "match", "enum", "flags", "from", "until", "while", "when", "aside", "check", "over", "unset", "origin", "valid"]);
/** Words of the notation only when a bracket follows: `at(`, `sized(`. A
 *  field may well be called `chain`. */
const CALL_KEYWORDS = new Set(["sized", "sizedbits", "optional", "at", "decoded", "repeat", "pointers", "chain", "gather", "stitched", "raster", "schema"]);
/** Words of the notation at the start of a line. */
const LEAD_KEYWORDS = new Set(["template", "root", "type", "deducer", "decoder"]);

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
  if (kind === "field" || kind === "kind") {
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

/** Which line of the template text one decoder kind is on, as the core's
 *  `template_kind_lines` gives it: `deflate.literal`, the kind of the row it
 *  is written under (empty for a family's blocks row), and the line. */
export type KindLine = { readonly kind: string; readonly parent: string; readonly line: number };

/**
 * The line for the node at the cursor when it is one a decoder laid down.
 * `chain` is the trace kind of each node from the root to the cursor, or
 * undefined for a node that has none. The deepest node with a kind decides,
 * so a field of a literal's record marks the literal's line. A kind written
 * under two rows, as LZ4's token is, is found under its parent's kind. Null
 * when no node in the chain has a kind the text has a line for.
 */
export function kindLineAt(lines: readonly KindLine[], chain: readonly (string | undefined)[]): number | null {
  for (let d = chain.length - 1; d >= 0; d--) {
    const kind = chain[d];
    if (kind === undefined) continue;
    const parent = chain[d - 1] ?? "";
    const all = lines.filter((l) => l.kind === kind);
    const hit = all.find((l) => l.parent === parent) ?? all[0];
    if (hit !== undefined) return hit.line;
  }
  return null;
}
