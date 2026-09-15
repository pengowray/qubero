// A folder's files, listed to open one at a time.
//
// Opening a folder of plain files opens none of them: the reader asked for the
// folder, and the first of ten thousand files, or a `.DS_Store`, would be a
// guess. The list takes the place of the start screen until a file is picked,
// and after that is one press away from the bar above the views, laid over
// them. It is one element in both places, so the filter and the scroll are
// where they were left.

import { el } from "./dom.ts";
import { FOLDER } from "./strings.ts";

/** A row of the list: the file's path inside the folder, and its size. */
export type ListedFile = { readonly label: string; readonly size: number };

/** How many rows are put in the page at once. Past this the filter narrows. */
const MAX_ROWS = 1000;

export class FolderList {
  readonly el: HTMLElement;
  /** A file's row was pressed, by its index in the list given. */
  onOpen: (i: number) => void = () => {};
  /** The whole-dataset row was pressed. Only offered when set before `render`. */
  onDataset: (() => void) | null = null;
  /** Escape, or anything else that asks the list to get out of the way. */
  onDismiss: () => void = () => {};

  private readonly files: readonly ListedFile[];
  private readonly formatSize: (bytes: number) => string;
  private readonly filter: HTMLInputElement;
  private readonly rows: HTMLElement;
  private readonly note: HTMLElement;
  private current: number | null = null;
  private datasetCurrent = false;

  constructor(heading: string, files: readonly ListedFile[], formatSize: (bytes: number) => string) {
    this.files = files;
    this.formatSize = formatSize;
    const total = files.reduce((n, f) => n + f.size, 0);
    this.filter = el("input", { type: "search", className: "folderlist-filter", placeholder: FOLDER.filterPlaceholder });
    this.filter.setAttribute("aria-label", FOLDER.filterLabel);
    this.filter.addEventListener("input", () => this.render());
    this.filter.addEventListener("keydown", (e) => {
      if (e.key !== "Enter") return;
      const first = this.rows.querySelector<HTMLButtonElement>("button");
      first?.click();
    });
    this.rows = el("ul", { className: "folderlist-rows" });
    this.note = el("p", { className: "folderlist-note" });
    this.el = el(
      "section",
      { className: "folderlist" },
      el("header", { className: "folderlist-head" }, el("h2", { textContent: heading }), el("span", { className: "folderlist-sub", textContent: FOLDER.sub(FOLDER.files(files.length), formatSize(total)) })),
      this.filter,
      this.note,
      this.rows,
    );
    this.el.addEventListener("keydown", (e) => {
      if (e.key === "Escape") this.onDismiss();
    });
  }

  /** Mark the row of the file that is open, or the dataset row. */
  setCurrent(i: number | null, dataset = false): void {
    this.current = i;
    this.datasetCurrent = dataset;
    this.render();
  }

  focus(): void {
    this.filter.focus();
  }

  render(): void {
    const text = this.filter.value.trim().toLowerCase();
    const items: HTMLElement[] = [];
    if (this.onDataset !== null && text === "") {
      const open = this.onDataset;
      items.push(this.row(FOLDER.wholeDataset, "", this.datasetCurrent, () => open(), "is-dataset"));
    }
    let matched = 0;
    this.files.forEach((f, i) => {
      if (text !== "" && !f.label.toLowerCase().includes(text)) return;
      matched++;
      if (matched > MAX_ROWS) return;
      items.push(this.row(f.label, this.formatSize(f.size), i === this.current, () => this.onOpen(i)));
    });
    this.rows.replaceChildren(...items);
    this.note.textContent = matched === 0 && text !== "" ? FOLDER.filterNone(this.filter.value.trim()) : matched > MAX_ROWS ? FOLDER.capped(MAX_ROWS, matched) : "";
    this.note.hidden = this.note.textContent === "";
  }

  private row(label: string, size: string, current: boolean, open: () => void, extra = ""): HTMLElement {
    const button = el("button", { type: "button", className: `folderlist-row ${extra}` }, el("span", { className: "folderlist-name", textContent: label }), el("span", { className: "folderlist-size", textContent: size }));
    if (current) button.setAttribute("aria-current", "true");
    button.addEventListener("click", open);
    return el("li", {}, button);
  }
}
