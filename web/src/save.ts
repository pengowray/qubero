// Save the composed output to a new file. Never writes the original in place.

import type { Doc } from "./doc.js";

type SavePicker = (opts: { suggestedName: string }) => Promise<{
  createWritable(): Promise<{ write(data: Blob): Promise<void>; close(): Promise<void> }>;
}>;

function hasSavePicker(w: Window): w is Window & { showSaveFilePicker: SavePicker } {
  return typeof (w as { showSaveFilePicker?: unknown }).showSaveFilePicker === "function";
}

export type SaveOutcome = { kind: "saved"; bytes: number } | { kind: "cancelled" } | { kind: "failed"; message: string };

export async function saveDoc(doc: Doc): Promise<SaveOutcome> {
  let blob: Blob;
  try {
    blob = await doc.buildOutput();
  } catch (e) {
    return { kind: "failed", message: e instanceof Error ? e.message : String(e) };
  }
  if (hasSavePicker(window)) {
    try {
      const handle = await window.showSaveFilePicker({ suggestedName: doc.name });
      const w = await handle.createWritable();
      await w.write(blob);
      await w.close();
      return { kind: "saved", bytes: blob.size };
    } catch (e) {
      if (e instanceof DOMException && e.name === "AbortError") return { kind: "cancelled" };
      return { kind: "failed", message: e instanceof Error ? e.message : String(e) };
    }
  }
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = doc.name;
  a.click();
  setTimeout(() => URL.revokeObjectURL(url), 60_000);
  return { kind: "saved", bytes: blob.size };
}
