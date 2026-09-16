// The ImHex pattern converter: the pattern on the left, what it could not
// carry across in the middle, the template it produced on the right.
//
// The layout and the report are `convertpanel.ts`, shared with the `.ksy`
// converter. Two things are this panel's own.
//
// The include files. ImHex patterns lean on a tree of `.pat` files that is
// GPL-2.0 and is not shipped here; the converter answers the common `std::` and
// `type::` names out of its own table, and for anything else the reader has to
// hand the file over. Those are listed in the report column with a box each.
//
// The library. 310 patterns live in a repository Qubero does not copy, so the
// list is metadata and the pattern itself is fetched when a reader asks for it.
// Nothing reaches the network until the button in the notice is pressed.

import { ConvertPanel, type PanelResult } from "./convertpanel.ts";
import type { Doc } from "./doc.ts";
import { el } from "./dom.ts";
import { fetchPattern, filesFor, loadLibrary, search, type Library, type LibraryEntry } from "./hexpatlibrary.ts";
import { HEXPAT } from "./strings.ts";

/** What a bundled pattern's template name starts with: `hexpat:vhd`. */
const HEXPAT_PREFIX = "hexpat:";

/** What one tab inserts. ImHex patterns are written four-space. */
const TAB = "    ";

export class HexpatPanel extends ConvertPanel {
  private readonly doc: Doc;
  private readonly bundledList: HTMLSelectElement;
  /** The shipped pattern in the box and the text it arrived as, or null for
   *  text from anywhere else. Applying it unedited reads the file as that
   *  bundled pattern rather than as a one-off. */
  private bundled: { readonly id: string; readonly text: string } | null = null;
  /** The name the converted template goes by: the file it came from, or the
   *  library entry that was fetched. Empty for a bare paste. */
  private name = "";
  /** Include files the reader has pasted in or the library fetched, keyed by
   *  the path the pattern asks for. */
  private includes: Record<string, string> = {};
  /** Include files this pattern still wants, from the last conversion. */
  private missing: readonly string[] = [];
  private readonly library: LibraryDialog;

  constructor(doc: Doc) {
    super(HEXPAT, ".hexpat,.pat", TAB, "kp-hexpat");
    this.doc = doc;

    this.bundledList = el("select", { className: "kp-bundled", title: HEXPAT.bundledTitle });
    this.bundledList.setAttribute("aria-label", HEXPAT.bundledLabel);
    this.bundledList.append(el("option", { value: "", textContent: HEXPAT.bundledPlaceholder }));
    for (const c of doc.templateChoices) {
      if (c.source !== "hexpat") continue;
      const id = c.name.startsWith(HEXPAT_PREFIX) ? c.name.slice(HEXPAT_PREFIX.length) : c.name;
      this.bundledList.append(el("option", { value: id, textContent: HEXPAT.bundledOption(id, c.title) }));
    }
    this.bundledList.addEventListener("change", () => {
      const id = this.bundledList.value;
      if (id === "") return;
      const text = this.doc.bundledHexpatText(id);
      if (text === "") return;
      this.loadBundled(id, text);
    });

    const libraryBtn = el("button", {
      type: "button",
      className: "kp-library-open",
      textContent: HEXPAT.libraryButton,
      title: HEXPAT.libraryButtonTitle,
    });
    libraryBtn.addEventListener("click", () => this.library.open());
    this.library = new LibraryDialog();
    this.library.onPick = (entry, fetched) => {
      this.bundled = null;
      this.bundledList.value = "";
      this.name = entry.name;
      this.includes = { ...fetched.includes };
      this.put(fetched.text, entry.path);
    };
    this.el.append(this.library.el);

    this.bar.append(this.bundledList, libraryBtn, this.fileName);
    this.draw();
  }

  override load(text: string, name: string | null): void {
    this.bundled = null;
    this.bundledList.value = "";
    this.includes = {};
    this.name = name ?? "";
    this.put(text, name);
  }

  /** Put a shipped pattern in the box, and remember that it is one. */
  loadBundled(id: string, text: string): void {
    this.bundled = { id, text };
    this.bundledList.value = id;
    this.includes = {};
    this.name = id;
    this.put(text, this.bundledList.value === id ? null : HEXPAT.bundledName(id));
  }

  override dispose(): void {
    super.dispose();
    this.library.el.remove();
  }

  protected override textEdited(): void {
    if (this.bundled !== null && this.bundled.text !== this.text) {
      this.bundled = null;
      this.bundledList.value = "";
    }
  }

  protected override preview(text: string): PanelResult {
    const reply = this.doc.previewHexpatTemplate(text, this.includes, this.name);
    if (reply.status === "ok") {
      this.missing = reply.report.missing ?? [];
      return { ok: true, report: reply.report, text: reply.text };
    }
    this.missing = reply.missing;
    return { ok: false, message: reply.message };
  }

  protected override applyConverted(): void {
    const shipped = this.bundled;
    if (shipped !== null && shipped.text === this.text) {
      this.onApply?.(shipped.id, true);
      return;
    }
    try {
      const report = this.doc.setHexpatTemplate(this.text, this.includes, this.name);
      this.onMessage?.(HEXPAT.applied(report.name));
      this.onApply?.(report.name, false);
    } catch (e) {
      this.onMessage?.(e instanceof Error ? e.message : String(e), true);
    }
  }

  /**
   * A report path is `line:column` in the pattern, so the line is the number
   * before the colon. One-based there, zero-based here.
   */
  protected override lineOfPath(_text: string, path: string): number | null {
    const line = /^(\d+):\d+$/.exec(path.trim());
    if (line === null) return null;
    const n = Number(line[1]);
    return Number.isFinite(n) && n > 0 ? n - 1 : null;
  }

  /** The include files the pattern is still waiting for, each with a box to
   *  paste it into. Above the report, because until they are here the report
   *  is about a pattern that did not finish reading. */
  protected override reportPrelude(): readonly HTMLElement[] {
    if (this.missing.length === 0) return [];
    const box = el("div", { className: "kp-needs" });
    box.append(
      el("p", { className: "kp-group-heading kp-needs-heading", textContent: HEXPAT.includesHeading(this.missing.length) }),
      el("p", { className: "kp-needs-note", textContent: HEXPAT.includesNote }),
    );
    for (const path of this.missing) {
      const area = el("textarea", { className: "kp-need-text", spellcheck: false, placeholder: HEXPAT.includePlaceholder(path) });
      area.setAttribute("aria-label", HEXPAT.includeLabel(path));
      area.addEventListener("input", () => {
        const text = area.value;
        if (text.trim() === "") delete this.includes[path];
        else this.includes[path] = text;
        this.schedule();
      });
      const held = this.includes[path];
      if (held !== undefined) area.value = held;
      box.append(el("div", { className: "kp-need" }, el("span", { className: "kp-need-path", textContent: path }), area));
    }
    return [box];
  }
}

/**
 * The library list, as a dialog over the panel.
 *
 * A dialog rather than a third state of the left column: the list is a detour,
 * and a detour that rearranges the panel behind it costs the reader their place
 * in what they were reading.
 *
 * Nothing here fetches on selection. Picking a row shows what would be
 * downloaded, from where, and under what licence; the button below that is the
 * only thing in this file that reaches the network.
 */
class LibraryDialog {
  readonly el: HTMLDialogElement;
  onPick: ((entry: LibraryEntry, fetched: { text: string; includes: Record<string, string> }) => void) | null = null;

  private readonly searchBox: HTMLInputElement;
  private readonly count: HTMLElement;
  private readonly list: HTMLElement;
  private readonly detail: HTMLElement;
  private library: Library | null = null;
  private picked: LibraryEntry | null = null;
  private loading = false;

  constructor() {
    this.searchBox = el("input", { type: "search", className: "kp-lib-search", placeholder: HEXPAT.librarySearchPlaceholder });
    this.searchBox.setAttribute("aria-label", HEXPAT.librarySearchLabel);
    this.searchBox.addEventListener("input", () => this.render());
    this.count = el("p", { className: "kp-lib-count" });
    this.list = el("div", { className: "kp-lib-list" });
    this.list.setAttribute("role", "listbox");
    this.list.setAttribute("aria-label", HEXPAT.libraryTitle);
    this.detail = el("div", { className: "kp-lib-detail" });

    const close = el("button", { type: "button", className: "kp-lib-close", textContent: HEXPAT.libraryClose });
    close.setAttribute("aria-label", HEXPAT.libraryCloseLabel);
    close.addEventListener("click", () => this.el.close());

    this.el = el(
      "dialog",
      { className: "kp-lib" },
      el("div", { className: "kp-lib-head" }, el("h3", { textContent: HEXPAT.libraryTitle }), close),
      el("div", { className: "kp-lib-searchbox" }, this.searchBox, this.count),
      this.list,
      this.detail,
    );
  }

  open(): void {
    this.el.showModal();
    this.searchBox.focus();
    if (this.library === null && !this.loading) void this.fetchIndex();
    else this.render();
  }

  private async fetchIndex(): Promise<void> {
    this.loading = true;
    this.count.textContent = HEXPAT.libraryLoading;
    try {
      this.library = await loadLibrary();
      this.render();
    } catch {
      this.count.textContent = HEXPAT.libraryFailed;
      this.list.replaceChildren();
    } finally {
      this.loading = false;
    }
  }

  private render(): void {
    const library = this.library;
    if (library === null) return;
    const q = this.searchBox.value;
    const found = search(library.patterns, q);
    this.count.textContent = HEXPAT.libraryCount(found.length, library.patterns.length);
    if (found.length === 0) {
      this.list.replaceChildren(el("p", { className: "kp-lib-empty", textContent: HEXPAT.libraryEmpty(q.trim()) }));
      this.detail.replaceChildren();
      return;
    }
    this.list.replaceChildren(...found.slice(0, 200).map((entry) => this.row(entry)));
    this.drawDetail();
  }

  private row(entry: LibraryEntry): HTMLElement {
    const state =
      entry.gaps === null ? HEXPAT.libraryRefused : entry.gaps === 0 ? HEXPAT.libraryWhole : HEXPAT.libraryGaps(entry.gaps);
    const stateClass = entry.gaps === null ? "is-refused" : entry.gaps === 0 ? "is-whole" : "is-gaps";
    const row = el(
      "button",
      { type: "button", className: "kp-lib-row" },
      el("span", { className: "kp-lib-name", textContent: entry.name }),
      el("span", { className: "kp-lib-desc", textContent: entry.description }),
      el("span", { className: `kp-lib-state ${stateClass}`, textContent: state }),
    );
    row.dataset.pattern = entry.path;
    if (entry.path === this.picked?.path) row.classList.add("is-picked");
    row.addEventListener("click", () => {
      this.picked = entry;
      for (const other of this.list.querySelectorAll(".kp-lib-row")) other.classList.remove("is-picked");
      row.classList.add("is-picked");
      this.drawDetail();
    });
    return row;
  }

  /** What picking a row shows: the pattern, what comes with it, where it comes
   *  from, and the one button that goes and gets it. */
  private drawDetail(): void {
    const entry = this.picked;
    if (entry === null) {
      this.detail.replaceChildren();
      return;
    }
    const files = filesFor(entry);
    const fetchBtn = el("button", {
      type: "button",
      className: "primary kp-lib-fetch",
      textContent: HEXPAT.fetch(entry.path, files.length - 1),
    });
    const status = el("p", { className: "kp-lib-status" });
    fetchBtn.addEventListener("click", () => {
      fetchBtn.disabled = true;
      status.textContent = HEXPAT.fetching;
      void fetchPattern(entry)
        .then((fetched) => {
          this.el.close();
          this.onPick?.(entry, fetched);
        })
        .catch((e: unknown) => {
          fetchBtn.disabled = false;
          status.textContent = HEXPAT.fetchFailed(entry.path, e instanceof Error ? e.message : String(e));
        });
    });
    const lines: HTMLElement[] = [
      el("p", { className: "kp-lib-picked", textContent: entry.path }),
    ];
    if (entry.author !== "") lines.push(el("p", { className: "kp-lib-author", textContent: entry.author }));
    if (entry.needs.length > 0) {
      lines.push(el("p", { className: "kp-lib-extra", textContent: HEXPAT.libraryNeeds(entry.needs.length) }));
    }
    this.detail.replaceChildren(
      ...lines,
      el("p", { className: "kp-lib-notice-heading", textContent: HEXPAT.noticeHeading }),
      el("p", { className: "kp-lib-notice", textContent: HEXPAT.noticeBody }),
      fetchBtn,
      status,
    );
  }
}
