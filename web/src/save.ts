// Save the composed output to a new file. Never writes the original in place.

import type { ByteSource, Doc } from "./doc.ts";

type SavePicker = (opts: { suggestedName: string }) => Promise<{
  createWritable(): Promise<{ write(data: Blob): Promise<void>; close(): Promise<void> }>;
}>;

function hasSavePicker(w: Window): w is Window & { showSaveFilePicker: SavePicker } {
  return typeof (w as { showSaveFilePicker?: unknown }).showSaveFilePicker === "function";
}

export type SaveOutcome = { kind: "saved"; bytes: number } | { kind: "cancelled" } | { kind: "failed"; message: string };

const failed = (e: unknown): SaveOutcome => ({ kind: "failed", message: e instanceof Error ? e.message : String(e) });

/**
 * Save `doc`. `source`, when given, is what the document's unchanged bytes
 * are read from instead of the file it was opened from, and may take a while
 * to have: a dataset read from a large folder waits for its CRC-32s.
 */
export function saveDoc(doc: Doc, source?: () => Promise<Pick<ByteSource, "slice">>): Promise<SaveOutcome> {
  return saveBlob(doc.name, async () => doc.buildOutput(source === undefined ? undefined : await source()));
}

/**
 * Save what `build` makes, as `name`. The file picker is asked first, while
 * the click that asked to save still counts as the reader's, and the wait for
 * the bytes comes after.
 */
export async function saveBlob(name: string, build: () => Promise<Blob>): Promise<SaveOutcome> {
  if (hasSavePicker(window)) {
    let handle: Awaited<ReturnType<SavePicker>>;
    try {
      handle = await window.showSaveFilePicker({ suggestedName: name });
    } catch (e) {
      if (e instanceof DOMException && e.name === "AbortError") return { kind: "cancelled" };
      return failed(e);
    }
    try {
      const blob = await build();
      const w = await handle.createWritable();
      await w.write(blob);
      await w.close();
      return { kind: "saved", bytes: blob.size };
    } catch (e) {
      return failed(e);
    }
  }
  let blob: Blob;
  try {
    blob = await build();
  } catch (e) {
    return failed(e);
  }
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = name;
  a.click();
  setTimeout(() => URL.revokeObjectURL(url), 60_000);
  return { kind: "saved", bytes: blob.size };
}
