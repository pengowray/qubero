/**
 * The format as strips, the way a specification draws one.
 *
 * The JPEG standard's figure B.2 is the model: a type is a row of boxes, its
 * fields left to right in the order the file writes them, and a field that
 * holds another type has that type's own row drawn under it and joined to it.
 * Nothing about it is a graph. It is a picture of a file read from the front,
 * which is what a reader opening one actually does, and it says in one glance
 * the thing the arrows say only after you have followed them: what comes after
 * what.
 *
 * This works out the plan — which strips, in what order, at what depth, and
 * what joins to what. It measures nothing and draws nothing: the widths of the
 * boxes are their text's, so only the page can say what they are, and
 * `diagramview.ts` lays them out once it has. Keeping the two apart is what
 * lets this be read and tested on its own.
 */
import type { DiagramBox, DiagramEdge, TemplateDiagram } from "./doc.ts";

/** How many strips the plan may hold. A format whose types run past this is a
 *  picture nobody can read anyway, and the drawing says what it left out. */
const STRIP_CAP = 240;

/** What one box in a strip stands for. */
export type ItemKind =
  /** One field, drawn as itself. */
  | "field"
  /** The first of a run: the same field, with how many there are under it. */
  | "first"
  /** The dotted band between the first of a run and the last. */
  | "band"
  /** The last of a run, which the file has however many of the count says. */
  | "last"
  /** What a fold hides, counted. */
  | "more";

/** One box of one strip. */
export type Item = {
  kind: ItemKind;
  /** Which row of the strip's type this is, or -1 for a band or a fold. */
  row: number;
  /** The word on the first line. */
  name: string;
  /** What it is, small, under the name. Empty where there is nothing to say. */
  note: string;
  /** The size column's words, smaller again. */
  size: string;
  /** The listing's word for the field's kind, for its colour. */
  fieldKind: string;
  /** True for a field that may not be there at all: drawn with a dashed
   *  outline, with what decides it as its note. */
  optional: boolean;
  /** True for a field read at an address rather than where it is declared:
   *  marked, and joined to what it points at. */
  placed: boolean;
  /** For a field whose type is a choice, the cases as `'IHDR' → IHDR` lines
   *  drawn inside the box. */
  cases: string[];
  /**
   * The strips this box opens onto, by index into `strips`.
   *
   * More than one for a choice: each case is a type of its own and gets a strip
   * of its own, the way any other composite does. `reference` is true where the
   * strip is drawn somewhere else and this is only a mention of it: one dashed
   * line to its title rather than a funnel, because two copies of one type are
   * two things as far as the eye is concerned and the format has one.
   */
  links: { strip: number; reference: boolean }[];
};

/** One type, as a row of boxes. */
export type Strip = {
  /** Which box of the diagram this is. */
  box: number;
  /** How far down the tree: the root is 0, what it opens onto is 1. */
  depth: number;
  /** What it is called, and how it was reached. */
  name: string;
  path: string;
  /** The diagram's key for it, which is what the census counts by. */
  key: string;
  items: Item[];
  /** The strip and box this one hangs under, for the funnel. Null for the
   *  root. */
  from: { strip: number; item: number } | null;
};

export type Plan = {
  strips: Strip[];
  /** Types the plan leaves out, past the cap. */
  omitted: number;
};

/** One case of a choice: what is read for it to be taken, and what it reads. */
type Case = { label: string; to: number };

/** Where a row's type edge goes, and where a switch's cases go. */
function targets(d: TemplateDiagram): { type: Map<string, number>; caseOf: Map<number, Case[]> } {
  const type = new Map<string, number>();
  const caseOf = new Map<number, Case[]>();
  for (const e of d.edges) {
    if (e.role === "type" && e.to_row === undefined) type.set(`${e.from[0]}:${e.from[1]}`, e.to);
    if (e.role === "case" && e.to_row === undefined) {
      const row = d.types[e.from[0]]?.rows[e.from[1]];
      if (row === undefined || d.types[e.to] === undefined) continue;
      caseOf.set(e.from[0], [...(caseOf.get(e.from[0]) ?? []), { label: row.name, to: e.to }]);
    }
  }
  return { type, caseOf };
}

/**
 * A choice's cases, read through any choice they land on.
 *
 * ELF reads the class and then the endianness, so the header is a choice of
 * choices; drawn as it is written that would be a row of nothing but the word
 * `1` and another below it of `2`. What a reader wants from those two questions
 * is the four headers they pick between, so the labels are joined and the
 * strips are of what is actually read.
 */
function choices(d: TemplateDiagram, caseOf: Map<number, Case[]>, box: number, depth = 0): Case[] {
  const out: Case[] = [];
  for (const c of caseOf.get(box) ?? []) {
    const to = d.types[c.to];
    if (to?.kind === "switch" && depth < 4) {
      for (const deeper of choices(d, caseOf, c.to, depth + 1)) {
        out.push({ label: `${c.label} · ${deeper.label}`, to: deeper.to });
      }
      continue;
    }
    out.push(c);
  }
  return out;
}

/** What decides whether an optional field is there, as the template writes it. */
function conditions(d: TemplateDiagram): Map<string, string> {
  const out = new Map<string, string>();
  for (const e of d.edges) {
    if (e.role !== "condition" || e.to_row === undefined) continue;
    out.set(`${e.to}:${e.to_row}`, e.label);
  }
  return out;
}

/**
 * The plan: which types get a strip, in what order, and what joins to what.
 *
 * Breadth-first from the root, so a strip's depth is how far from the front of
 * the file it is and the rows fill up in the order a reader meets them. A type
 * reached twice is drawn once, under its first use; the second use gets a line
 * to it rather than a copy of it, because two copies of one type are two
 * things as far as the eye is concerned and the format has one.
 */
export function plan(d: TemplateDiagram, rowCap: number, shown: (box: number) => boolean): Plan {
  const { type, caseOf } = targets(d);
  const cond = conditions(d);
  const strips: Strip[] = [];
  const at = new Map<number, number>();
  let omitted = 0;

  const open = (box: number, depth: number, from: { strip: number; item: number } | null): number | null => {
    const b = d.types[box];
    if (b === undefined || !shown(box)) return null;
    if (strips.length >= STRIP_CAP) {
      omitted++;
      return null;
    }
    const here = strips.length;
    at.set(box, here);
    strips.push({ box, depth, name: b.name, path: b.path, key: b.key, items: [], from });
    return here;
  };

  const root = open(0, 0, null);
  if (root === null) return { strips, omitted };

  for (let i = 0; i < strips.length; i++) {
    const strip = strips[i];
    if (strip === undefined) continue;
    const b = d.types[strip.box];
    if (b === undefined) continue;
    strip.items = items(b, strip.box, rowCap, cond);
    // What each box opens onto, once the strip itself is settled: a child is
    // queued behind every strip already waiting, which is what makes the rows
    // fill in the order a reader meets them.
    for (const [k, item] of strip.items.entries()) {
      if (item.row < 0 || item.kind === "last") continue;
      const to = type.get(`${strip.box}:${item.row}`);
      if (to === undefined) continue;
      // A choice is not a type of its own on the page: it is one box saying
      // what it reads, with its cases listed inside it, and each case's own
      // type drawn below like any other composite. Drawing the choice as a
      // strip as well would put a row of nothing but case labels between a
      // field and what it actually holds.
      const target = d.types[to];
      let outs = [to];
      if (target?.kind === "switch") {
        const picks = choices(d, caseOf, to);
        item.cases = picks.map((c) => `${c.label} → ${d.types[c.to]?.name ?? ""}`);
        item.note = target.name;
        outs = picks.map((c) => c.to);
      }
      for (const box of outs) {
        if (!shown(box)) continue;
        const already = at.get(box);
        if (already !== undefined) {
          item.links.push({ strip: already, reference: true });
          continue;
        }
        const made = open(box, strip.depth + 1, { strip: i, item: k });
        if (made !== null) item.links.push({ strip: made, reference: false });
      }
    }
  }
  return { strips, omitted };
}

/** One type's boxes, in the order the file writes them. */
function items(b: DiagramBox, box: number, rowCap: number, cond: Map<string, string>): Item[] {
  const out: Item[] = [];
  const push = (row: number, kind: ItemKind, name: string, note: string, size: string, fieldKind: string): Item => {
    const item: Item = {
      kind,
      row,
      name,
      note,
      size,
      fieldKind,
      optional: false,
      placed: false,
      cases: [],
      links: [],
    };
    out.push(item);
    return item;
  };
  // A long type is folded the way the other mode folds it: the first rows, a
  // box saying how many are hidden, and the last, so the end of the type is
  // still on the page. A strip that ran off the side would be worse than one
  // that says it did.
  const many = b.rows.length > rowCap;
  const upto = many ? rowCap - 1 : b.rows.length;
  const draw = (i: number): void => {
    const r = b.rows[i];
    if (r === undefined) return;
    // A run: the first of it, a dotted band, and the last. The count or the
    // condition it stops at goes under the first, which is where a reader
    // looking for "how many" looks.
    const run = r.type_text.endsWith("[]") || r.size_text.startsWith("until ") || r.size_text === "to the end";
    // What makes a field optional is that something decides whether it is
    // there, and what says so is the condition edge: a type named in the
    // template prints as its name, so the type column cannot be relied on to
    // start with the word.
    const optional = cond.has(`${box}:${i}`) || r.type_text.startsWith("optional ");
    const placed = r.pos_text !== "" && !r.pos_text.startsWith("0x");
    const what = optional ? r.type_text.slice("optional ".length) : r.type_text;
    const first = push(i, run ? "first" : "field", r.name, what, run ? r.size_text : "", r.kind);
    first.optional = optional;
    first.placed = placed;
    if (optional) first.size = cond.get(`${box}:${i}`) ?? "";
    if (run) {
      push(-1, "band", "", "", "", r.kind);
      const last = push(i, "last", r.name, "", "", r.kind);
      last.optional = optional;
    }
  };
  for (let i = 0; i < upto; i++) draw(i);
  if (many) {
    push(-1, "more", String(b.rows.length - rowCap), "", "", "note");
    draw(b.rows.length - 1);
  }
  return out;
}
