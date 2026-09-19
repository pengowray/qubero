// The form that saves a table as a file, and the button that opens it.
//
// A form and not a menu of formats, because a save has three things to settle
// and the reader should see all three answers before anything is written: how
// much of the table, which format, and (for a table drawn turned) which way
// round. The form says how big the file will be before the reader commits to
// it, the save reports how far it has got, and the button that opened the form
// stops the save while one is running.

import { el } from "./dom.ts";
import { saveBlob } from "./save.ts";
import { storedChoice, rememberChoice } from "./stored.ts";
import { TABLE } from "./strings.ts";
import { exportName, exportSize, exportText, FORMAT_FACTS, FORMATS, scopeOf, writtenTurned, type ExportFormat, type ExportJob } from "./tableexport.ts";
import type { TableRow } from "./tableplan.ts";
import type { Lead } from "./tabletext.ts";

/** The format the reader last saved in. Per browser: somebody who wants TSV
 *  wants it next time too. */
const FORMAT_KEY = "qubero.table.export.format";
/** How long to wait on a row that has not been read, and how often to look.
 *  A file still arriving over a network answers null for a while and then
 *  answers; one that never does should not hang the save for ever. */
const WAIT_MS = 100;
const WAIT_TRIES = 100;
/** Past this many rows the plan is told to let go of what it has read as the
 *  save goes, or a run of millions of samples is all in memory at once. */
const RELEASE_OVER = 100_000;

/** What the form needs to know of the table, asked afresh each time, since the
 *  reader changes all of it between one save and the next. */
export type ExportSource = {
  readonly file: string;
  readonly table: string;
  readonly rowWord: string;
  readonly count: number;
  headings: () => string[];
  lead: () => Lead;
  turned: () => boolean;
  selected: () => { from: number; to: number } | null;
  /** Record `i`, or null while it is being read. */
  row: (i: number) => TableRow | null;
  /** Let go of the records read so far. */
  release: () => void;
  say: (text: string) => void;
};

/** Hand the page a turn. A message rather than a timer, because a tab in the
 *  background has its timers slowed to one a second, and a save of a million
 *  rows that the reader looked away from would take the afternoon. */
function pause(): Promise<void> {
  return new Promise((resolve) => {
    const channel = new MessageChannel();
    channel.port1.onmessage = () => resolve();
    channel.port2.postMessage(0);
  });
}

function wait(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

class Stopped extends Error {}

export class TableExportPanel {
  /** The button and the form under it, to go in the table's bar. */
  readonly el: HTMLElement;
  private readonly source: ExportSource;
  private readonly open: HTMLButtonElement;
  private readonly form: HTMLFormElement;
  private readonly selectedBox: HTMLInputElement;
  private readonly selectedText: HTMLElement;
  private readonly asShownBox: HTMLInputElement;
  private readonly asShownLabel: HTMLElement;
  private readonly layout: HTMLElement;
  private readonly size: HTMLElement;
  private running = false;
  private stopping = false;

  constructor(source: ExportSource) {
    this.source = source;
    this.open = el("button", { type: "button", className: "tbl-copy", textContent: TABLE.exportOpen, title: TABLE.exportOpenTitle });
    this.open.setAttribute("aria-haspopup", "dialog");
    this.open.addEventListener("click", () => (this.running ? this.stop() : this.toggle()));

    const all = this.radio("what", "all", true);
    this.selectedBox = this.radio("what", "selected", false);
    this.selectedText = el("span");
    const format = storedChoice(FORMAT_KEY, FORMATS, "csv");
    const formats = FORMATS.map((f) => el("label", { className: "tbl-export-choice" }, this.radio("format", f, f === format), FORMAT_FACTS[f].label));
    const byRow = this.radio("layout", "row", true);
    this.asShownBox = this.radio("layout", "shown", false);
    this.asShownLabel = el("label", { className: "tbl-export-choice" }, this.asShownBox, TABLE.exportAsShown);
    this.layout = this.group(TABLE.exportLayout(source.rowWord), el("label", { className: "tbl-export-choice" }, byRow, TABLE.exportByRow), this.asShownLabel);
    this.size = el("p", { className: "tbl-export-size" });
    this.form = el(
      "form",
      { className: "tbl-export-form", hidden: true },
      this.group(TABLE.exportWhat, el("label", { className: "tbl-export-choice" }, all, TABLE.exportAll), el("label", { className: "tbl-export-choice" }, this.selectedBox, this.selectedText)),
      this.group(TABLE.exportFormat, ...formats),
      this.layout,
      this.size,
      el("button", { type: "submit", className: "tbl-copy tbl-export-save", textContent: TABLE.exportSave }),
    );
    this.form.setAttribute("role", "dialog");
    this.form.setAttribute("aria-label", TABLE.exportOpenTitle);
    this.form.addEventListener("change", () => this.refresh());
    this.form.addEventListener("submit", (e) => {
      e.preventDefault();
      void this.save();
    });
    this.form.addEventListener("keydown", (e) => {
      if (e.key !== "Escape") return;
      e.stopPropagation();
      this.close();
      this.open.focus();
    });
    this.el = el("span", { className: "tbl-export" }, this.open, this.form);
  }

  /** A click anywhere else puts the form away, as it does any popup. Listened
   *  for only while the form is up, so a closed tab's table leaves nothing on
   *  the document. */
  private readonly away = (e: MouseEvent): void => {
    if (e.target instanceof Node && !this.el.contains(e.target)) this.close();
  };

  private radio(name: string, value: string, checked: boolean): HTMLInputElement {
    return el("input", { type: "radio", name, value, checked });
  }

  private group(legend: string, ...choices: HTMLElement[]): HTMLElement {
    return el("fieldset", { className: "tbl-export-group" }, el("legend", { textContent: legend }), ...choices);
  }

  private toggle(): void {
    if (!this.form.hidden) return this.close();
    this.form.hidden = false;
    document.addEventListener("mousedown", this.away);
    this.open.setAttribute("aria-expanded", "true");
    this.refresh();
  }

  private close(): void {
    this.form.hidden = true;
    document.removeEventListener("mousedown", this.away);
    this.open.setAttribute("aria-expanded", "false");
  }

  /** The form's one answer: what would be written if the reader saved now. */
  private job(): ExportJob {
    const data = new FormData(this.form);
    const format = FORMATS.find((f) => f === data.get("format")) ?? "csv";
    const selected = data.get("what") === "selected" ? this.source.selected() : null;
    return {
      format,
      headings: this.source.headings(),
      lead: this.source.lead(),
      count: this.source.count,
      ...scopeOf(this.source.turned(), this.source.count, selected),
      asShown: this.source.turned() && data.get("layout") === "shown",
    };
  }

  /** Bring the form up to date with the table: what is selected, which way
   *  round it is drawn, and so what the file would hold. */
  private refresh(): void {
    const selected = this.source.selected();
    const n = selected === null ? 0 : selected.to - selected.from;
    this.selectedBox.disabled = n === 0;
    if (n === 0 && this.selectedBox.checked) this.form.querySelector<HTMLInputElement>('input[name="what"][value="all"]')?.click();
    this.selectedText.textContent = n === 0 ? TABLE.exportSelectedNone : TABLE.exportSelected(n);
    // Which way round is only a question for a table drawn turned.
    this.layout.hidden = !this.source.turned();
    const job = this.job();
    const json = job.format === "json";
    this.asShownBox.disabled = json;
    this.asShownLabel.title = json ? TABLE.exportJsonLayout(this.source.rowWord) : "";
    this.asShownLabel.classList.toggle("is-off", json);
    const { rows, columns } = exportSize(job);
    if (json) this.size.textContent = TABLE.exportSizeJson(rows, this.source.rowWord);
    else this.size.textContent = writtenTurned(job) ? TABLE.exportSizeShown(rows, columns) : TABLE.exportSize(rows, columns);
  }

  private stop(): void {
    this.stopping = true;
  }

  /** Record `i` once it has been read. */
  private async rowWhenRead(i: number): Promise<TableRow> {
    for (let tries = 0; tries < WAIT_TRIES; tries++) {
      if (this.stopping) throw new Stopped();
      const row = this.source.row(i);
      if (row !== null) return row;
      await wait(WAIT_MS);
    }
    throw new Error(TABLE.copyPending);
  }

  private async save(): Promise<void> {
    if (this.running) return;
    const job = this.job();
    const total = exportSize(job).rows;
    rememberChoice(FORMAT_KEY, job.format);
    this.close();
    this.running = true;
    this.stopping = false;
    this.open.textContent = TABLE.exportStop;
    const outcome = await saveBlob(exportName(this.source.file, this.source.table, job.format), async () => {
      // Each piece becomes a Blob of its own as it is made, which a browser is
      // free to keep on disk; one string of the whole file is not.
      const pieces: Blob[] = [];
      for await (const piece of exportText(job, (i) => this.rowWhenRead(i))) {
        if (this.stopping) throw new Stopped();
        pieces.push(new Blob([piece.text]));
        if (piece.done < total) this.source.say(TABLE.exportProgress(piece.done, total));
        if (this.source.count > RELEASE_OVER) this.source.release();
        await pause();
      }
      return new Blob(pieces, { type: FORMAT_FACTS[job.format].mime });
    });
    this.running = false;
    this.open.textContent = TABLE.exportOpen;
    if (this.stopping) this.source.say(TABLE.exportStopped);
    else if (outcome.kind === "saved") this.source.say(TABLE.exported(total, FORMAT_FACTS[job.format].label));
    else if (outcome.kind === "failed") this.source.say(TABLE.exportFailed(outcome.message));
    this.stopping = false;
  }
}
