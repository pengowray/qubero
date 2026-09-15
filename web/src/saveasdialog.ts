// Save as, for what came from a folder: the one place a ZIP of the folder and
// its CRC-32s are asked about. A file opened on its own saves straight to the
// browser's picker, as it always has.

import { el } from "./dom.ts";
import { SAVE_AS } from "./strings.ts";

export type SaveAsOffer = {
  /** The open file and its size, for a file of a plain folder. Null for a
   *  dataset, which has only the ZIP to save. */
  readonly file: { readonly name: string; readonly size: string } | null;
  readonly zip: string;
  readonly files: string;
  readonly size: string;
  /** The open file's name when it has unsaved edits, which the ZIP takes. */
  readonly edited: string | null;
  /** How much reading the checksums still costs, or null when they are
   *  already known. */
  readonly crcCost: string | null;
};

export type SaveAsChoice = { readonly whole: boolean; readonly crc: boolean };

/** Ask what to save. Null when the reader cancels. */
export function askSaveAs(offer: SaveAsOffer): Promise<SaveAsChoice | null> {
  const dialog = el("dialog", { className: "saveas" });
  const form = el("form", { method: "dialog" });
  const crc = el("input", { type: "checkbox", checked: true });
  const crcRow = el("div", { className: "saveas-crc" }, el("label", {}, crc, " ", SAVE_AS.crc(offer.crcCost)), el("p", { className: "saveas-note", textContent: SAVE_AS.crcOffNote }));
  let whole: HTMLInputElement | null = null;
  const body: HTMLElement[] = [el("h2", { textContent: SAVE_AS.title })];
  if (offer.file === null) {
    body.push(el("p", { textContent: SAVE_AS.datasetOnly(offer.zip, offer.files, offer.size) }), crcRow);
  } else {
    const one = el("input", { type: "radio", name: "what", checked: true });
    whole = el("input", { type: "radio", name: "what" });
    const zipChoice = el("div", { className: "saveas-choice" }, el("label", {}, whole, " ", SAVE_AS.wholeFolder(offer.zip, offer.files, offer.size)));
    if (offer.edited !== null) zipChoice.append(el("p", { className: "saveas-note", textContent: SAVE_AS.withEdits(offer.edited) }));
    zipChoice.append(crcRow);
    const sync = (): void => {
      crc.disabled = !(whole as HTMLInputElement).checked;
      crcRow.classList.toggle("is-off", crc.disabled);
    };
    one.addEventListener("change", sync);
    whole.addEventListener("change", sync);
    sync();
    body.push(el("div", { className: "saveas-choice" }, el("label", {}, one, " ", SAVE_AS.thisFile(offer.file.name, offer.file.size))), zipChoice);
  }
  const ok = el("button", { type: "submit", value: "save", className: "primary", textContent: SAVE_AS.ok });
  const cancel = el("button", { type: "submit", value: "cancel", textContent: SAVE_AS.cancel });
  body.push(el("div", { className: "saveas-buttons" }, cancel, ok));
  form.append(...body);
  dialog.append(form);
  document.body.append(dialog);
  return new Promise((resolve) => {
    dialog.addEventListener("close", () => {
      const saving = dialog.returnValue === "save";
      dialog.remove();
      resolve(saving ? { whole: whole === null || whole.checked, crc: crc.checked } : null);
    });
    dialog.showModal();
    ok.focus();
  });
}
