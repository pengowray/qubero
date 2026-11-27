// The Template tab of the right sidebar: the whole template reading this file,
// as the text `template_text.rs` writes, with the line that declares the field
// at the cursor marked.
//
// The text is built into the page once per template and kept there; a cursor
// move only moves the mark. A big template (PNG's is near 8,000 lines) is
// built in chunks of lines: each chunk starts as an empty block of the right
// height, so the scrollbar and a jump to any line are right from the start,
// and is filled in when it is needed or when the page is idle. Every chunk is
// filled in the end, so selecting the whole text and copying it gets all of it.

import type { Doc } from "./doc.ts";
import { SIDEBAR } from "./strings.ts";
import { findLineAt, indexTemplate, readAs, tokenizeLine, type ChainNode, type Kid, type TemplateIndex } from "./templateindex.ts";

/** One line's height in pixels. Must match `--tp-line` in style.css: the
 *  empty chunks are sized from it before any line is drawn. */
const LINE = 18;
/** Lines per chunk. */
const CHUNK = 400;
/** Chunks drawn at once when the text arrives; the rest wait for idle time. */
const FIRST_CHUNKS = 3;

type Chunk = { readonly el: HTMLElement; readonly from: number; readonly to: number; filled: boolean };

const idle: (fn: () => void) => void =
  typeof requestIdleCallback === "function" ? (fn) => void requestIdleCallback(fn, { timeout: 200 }) : (fn) => void setTimeout(fn, 16);

export class TemplatePanel {
  readonly el: HTMLElement;
  private readonly scroller: HTMLElement;
  private readonly body: HTMLElement;
  private text: string | null = null;
  private index: TemplateIndex | null = null;
  private chunks: Chunk[] = [];
  private lineEls: (HTMLElement | undefined)[] = [];
  /** Bumped on every rebuild, so idle work for an old text stops. */
  private generation = 0;
  private marked: number | null = null;
  /** Said at the end of the marked line: the byte order its field was read
   *  in, when that is not the one the line writes. */
  private note: string | null = null;
  private visible = false;
  /** The template may have changed since the text was fetched. */
  private stale = true;
  private offset = 0;
  private pinned: readonly number[] | null = null;
  /** The cursor's field could not be found yet for want of bytes. */
  private waiting = false;
  /** A parent's children, by path, for choosing between same-named structures. */
  private kids = new Map<string, readonly Kid[] | null>();
  private charWidth = 0;
  /** The marked line moved while the text had no size (the sidebar folded),
   *  so it is scrolled to once the text has one again. */
  private revealOwed = false;
  /** Whether the file has been checked for a template yet. Until it has,
   *  having none is not yet an answer, and nothing is shown. */
  private known = false;
  private builtKnown = false;

  constructor(private readonly doc: Doc) {
    this.el = document.createElement("section");
    this.el.className = "templatepanel";
    this.el.setAttribute("aria-label", SIDEBAR.template);
    this.scroller = document.createElement("div");
    this.scroller.className = "tp-scroll";
    this.scroller.tabIndex = 0;
    this.body = document.createElement("div");
    this.body.className = "tp-text";
    this.scroller.append(this.body);
    this.el.append(this.scroller);
    doc.onChange(() => {
      // An edit can change which structure a node is (ELF's `class` decides
      // which ELFHeader is read), so the children are asked again.
      this.kids.clear();
      if (this.visible && this.waiting) this.follow();
    });
    new ResizeObserver(() => {
      if (this.revealOwed && this.marked !== null && this.scroller.clientHeight > 0) this.reveal(this.marked);
    }).observe(this.scroller);
  }

  /** The tab was shown or hidden. */
  setVisible(visible: boolean): void {
    this.visible = visible;
    if (!visible) return;
    // Asked again each time, not only when the template's name changes: a
    // `.ksy` pasted again is a new template under the same name.
    this.stale = true;
    this.refresh();
  }

  /** The file has been checked for a template, whatever the answer. */
  sniffed(): void {
    this.known = true;
    this.templateChanged();
  }

  /** The file is read by another template now. */
  templateChanged(): void {
    this.stale = true;
    if (this.visible) this.refresh();
  }

  /** `bitOffset` is absolute, counting from the top bit of byte 0. */
  setOffset(bitOffset: number): void {
    this.offset = bitOffset;
    this.pinned = null;
    if (this.visible) this.follow();
  }

  /** Mark this field rather than the innermost one at the cursor. */
  setPath(path: readonly number[]): void {
    this.pinned = path;
    if (this.visible) this.follow();
  }

  private refresh(): void {
    if (this.stale) {
      this.stale = false;
      this.build(this.doc.templateText());
    }
    this.follow();
  }

  private build(text: string): void {
    if (text === this.text && (text !== "" || this.builtKnown === this.known)) return;
    this.builtKnown = this.known;
    this.text = text;
    this.generation++;
    this.kids.clear();
    this.marked = null;
    this.lineEls = [];
    this.chunks = [];
    this.scroller.scrollTop = 0;
    this.scroller.scrollLeft = 0;
    if (text === "") {
      this.index = null;
      const empty = document.createElement("div");
      empty.className = "tp-empty";
      empty.textContent = this.known ? SIDEBAR.noTemplate : "";
      this.body.replaceChildren(empty);
      this.body.style.minWidth = "";
      return;
    }
    const index = indexTemplate(text);
    this.index = index;
    let widest = 0;
    for (const l of index.lines) widest = Math.max(widest, l.length);
    // Off-screen chunks have no width until drawn, so the text says how wide
    // it is up front and the horizontal scrollbar does not grow as it fills.
    this.body.style.minWidth = `calc(${widest}ch + 16px)`;
    this.lineEls = new Array<HTMLElement | undefined>(index.lines.length);
    const parts: HTMLElement[] = [];
    for (let from = 0; from < index.lines.length; from += CHUNK) {
      const to = Math.min(index.lines.length, from + CHUNK);
      const el = document.createElement("div");
      el.className = "tp-chunk";
      el.style.height = `${(to - from) * LINE}px`;
      this.chunks.push({ el, from, to, filled: false });
      parts.push(el);
    }
    this.body.replaceChildren(...parts);
    for (const c of this.chunks.slice(0, FIRST_CHUNKS)) this.fill(c);
    const gen = this.generation;
    const next = (): void => {
      if (gen !== this.generation) return;
      const c = this.chunks.find((x) => !x.filled);
      if (c === undefined) return;
      this.fill(c);
      idle(next);
    };
    idle(next);
  }

  private fill(c: Chunk): void {
    if (c.filled || this.index === null) return;
    c.filled = true;
    const { lines, kinds, decls } = this.index;
    const frag = document.createDocumentFragment();
    for (let i = c.from; i < c.to; i++) {
      const row = document.createElement("div");
      row.className = "tp-line";
      // An empty block copies as nothing at all, so a blank line would be
      // lost from copied text without something in it.
      if (lines[i] === "") row.append(document.createElement("br"));
      for (const s of tokenizeLine(lines[i] ?? "", kinds[i] ?? "plain", decls[i])) {
        if (s.cls === "") row.append(s.text);
        else {
          const span = document.createElement("span");
          span.className = `tp-${s.cls}`;
          span.textContent = s.text;
          row.append(span);
        }
      }
      if (i === this.marked) this.dress(row);
      this.lineEls[i] = row;
      frag.append(row);
    }
    c.el.replaceChildren(frag);
    c.el.style.height = "";
  }

  /** Find the cursor's field and mark its line. */
  private follow(): void {
    this.waiting = false;
    const index = this.index;
    if (index === null) return;
    let path = this.pinned;
    if (path === null) {
      const found = this.doc.locate(this.offset);
      if (found.status === "pending" || found.status === "working") {
        this.waiting = true;
        return;
      }
      if (found.status !== "ok") return this.mark(null);
      path = found.node;
    }
    const chain: ChainNode[] = [];
    for (let d = 0; d <= path.length; d++) {
      const r = this.doc.templateNode(path.slice(0, d));
      if (r.status === "pending" || r.status === "working") {
        this.waiting = true;
        return;
      }
      if (r.status !== "ok") break;
      chain.push({ name: r.node.name, type: r.node.type, list: r.node.list });
    }
    const p = path;
    let loading = false;
    const kidsAt = (depth: number): readonly Kid[] | null => {
      const kids = this.childrenOf(p.slice(0, depth));
      if (kids === "loading") loading = true;
      return kids === "loading" ? null : kids;
    };
    const found = findLineAt(index, chain, kidsAt);
    let note: string | null = null;
    const node = found === null ? undefined : chain[found.depth];
    if (found !== null && node !== undefined) {
      const order = readAs(index, found.line, node, () => kidsAt(found.depth - 1) ?? []);
      if (order !== null) note = SIDEBAR.readAs(order);
    }
    // Marked now at the enclosing line, and again when the children arrive.
    this.waiting = loading;
    this.mark(found?.line ?? null, note);
  }

  private childrenOf(path: readonly number[]): readonly Kid[] | null | "loading" {
    const key = path.join("/");
    if (this.kids.has(key)) return this.kids.get(key)!;
    const node = this.doc.templateNode(path);
    if (node.status === "pending" || node.status === "working") return "loading";
    if (node.status !== "ok") return null;
    const r = this.doc.templateChildren(path, 0, Math.min(node.node.child_count, 256));
    // Not kept while the answer is still coming, so the next move asks again.
    if (r.status === "pending" || r.status === "working") return "loading";
    if (r.status !== "ok") return null;
    const kids = r.node.map((n) => ({ name: n.name, type: n.type, value: n.value, edit: n.edit_text }));
    this.kids.set(key, kids);
    return kids;
  }

  private mark(line: number | null, note: string | null = null): void {
    if (line === this.marked && note === this.note) return;
    const moved = line !== this.marked;
    if (this.marked !== null) {
      const old = this.lineEls[this.marked];
      old?.classList.remove("is-at");
      if (old !== undefined) this.undress(old);
    }
    this.marked = line;
    this.note = note;
    if (line === null) return;
    const chunk = this.chunks[Math.floor(line / CHUNK)];
    if (chunk !== undefined) this.fill(chunk);
    const row = this.lineEls[line];
    if (row !== undefined) this.dress(row);
    if (moved) this.reveal(line);
  }

  /** Mark a drawn row as the marked line, with its note. The note goes
   *  straight after the number type it is about (`u32be`), so it covers
   *  nothing and the field's name stays in line with its neighbours. It is
   *  drawn by CSS from `data-note`, so a selection does not copy it. */
  private dress(row: HTMLElement): void {
    row.classList.add("is-at");
    this.undress(row);
    if (this.note === null) return;
    const note = document.createElement("span");
    note.className = "tp-note";
    note.dataset.note = this.note;
    const walker = document.createTreeWalker(row, NodeFilter.SHOW_TEXT);
    for (let t = walker.nextNode() as Text | null; t !== null; t = walker.nextNode() as Text | null) {
      const m = /\b[a-z]+\d+(?:le|be)\b/.exec(t.data);
      if (m === null) continue;
      t.splitText(m.index + m[0].length).before(note);
      return;
    }
    row.append(note);
  }

  /** Take the note off a row and join the text it was put into. */
  private undress(row: HTMLElement): void {
    const note = row.querySelector(".tp-note");
    if (note === null) return;
    note.remove();
    row.normalize();
  }

  /** Scroll the marked line into view, leaving the view alone if it already is. */
  private reveal(line: number): void {
    const s = this.scroller;
    this.revealOwed = s.clientHeight === 0;
    if (this.revealOwed) return;
    const top = line * LINE;
    if (top < s.scrollTop || top + LINE > s.scrollTop + s.clientHeight) s.scrollTop = Math.max(0, top - s.clientHeight / 3);
    if (this.charWidth === 0) {
      const probe = document.createElement("span");
      probe.textContent = "0".repeat(10);
      probe.style.visibility = "hidden";
      this.body.append(probe);
      this.charWidth = probe.getBoundingClientRect().width / 10;
      probe.remove();
    }
    const text = this.index?.lines[line] ?? "";
    const indent = (text.length - text.trimStart().length) * this.charWidth;
    const room = s.clientWidth - 12 * this.charWidth;
    if (indent < s.scrollLeft || indent > s.scrollLeft + room) s.scrollLeft = Math.max(0, indent - 2 * this.charWidth);
  }
}
