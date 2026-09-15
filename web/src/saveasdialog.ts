// Save as, for what came from a folder: the one place a ZIP of the folder is
// asked about. A file opened on its own saves straight to the browser's
// picker, as it always has. The ZIP is always written with its CRC-32s.

import { el } from "./dom.ts";
import { spinner } from "./spinner.ts";
import { SAVE_AS } from "./strings.ts";
import type { SumJob } from "./sumjob.ts";

export type SaveAsOffer = {
  /** The open file and its size, for a file of a plain folder. Null for a
   *  dataset, which has only the ZIP to save. */
  readonly file: { readonly name: string; readonly size: string } | null;
  readonly zip: string;
  readonly files: string;
  readonly size: string;
  /** The open file's name when it has unsaved edits, which the ZIP takes. */
  readonly edited: string | null;
  /** The CRC-32s still being read for a dataset opened without them, or null
   *  when there are none to wait for. */
  readonly sums: SumJob | null;
  /** How a count of bytes reads. */
  readonly formatSize: (bytes: number) => string;
};

export type SaveAsChoice = { readonly whole: boolean };

/** Ask what to save. Null when the reader cancels. */
export function askSaveAs(offer: SaveAsOffer): Promise<SaveAsChoice | null> {
  const dialog = el("dialog", { className: "saveas" });
  const form = el("form", { method: "dialog" });
  let whole: HTMLInputElement | null = null;
  const body: HTMLElement[] = [el("h2", { textContent: SAVE_AS.title })];
  if (offer.file === null) {
    body.push(el("p", { textContent: SAVE_AS.datasetOnly(offer.zip, offer.files, offer.size) }));
  } else {
    const one = el("input", { type: "radio", name: "what", checked: true });
    whole = el("input", { type: "radio", name: "what" });
    const zipChoice = el("div", { className: "saveas-choice" }, el("label", {}, whole, " ", SAVE_AS.wholeFolder(offer.zip, offer.files, offer.size)));
    if (offer.edited !== null) zipChoice.append(el("p", { className: "saveas-note", textContent: SAVE_AS.withEdits(offer.edited) }));
    body.push(el("div", { className: "saveas-choice" }, el("label", {}, one, " ", SAVE_AS.thisFile(offer.file.name, offer.file.size))), zipChoice);
  }
  // Sums still to come are read now rather than when the page is next idle:
  // saving is what they were for.
  let unwatch = (): void => {};
  const job = offer.sums;
  if (job !== null && !job.finished) {
    const words = document.createTextNode("");
    const status = el("p", { className: "saveas-sums" }, spinner(), words);
    status.setAttribute("role", "status");
    const show = (): void => {
      if (job.finished) status.remove();
      else words.data = SAVE_AS.calculating(offer.formatSize(job.read), offer.formatSize(job.total));
    };
    show();
    unwatch = job.onChange(show);
    body.push(status);
    job.start();
  }
  const ok = el("button", { type: "submit", value: "save", className: "primary", textContent: SAVE_AS.ok });
  const cancel = el("button", { type: "submit", value: "cancel", textContent: SAVE_AS.cancel });
  body.push(el("div", { className: "saveas-buttons" }, cancel, ok));
  form.append(...body);
  dialog.append(form);
  document.body.append(dialog);
  return new Promise((resolve) => {
    dialog.addEventListener("close", () => {
      unwatch();
      const saving = dialog.returnValue === "save";
      dialog.remove();
      resolve(saving ? { whole: whole === null || whole.checked } : null);
    });
    dialog.showModal();
    ok.focus();
  });
}
