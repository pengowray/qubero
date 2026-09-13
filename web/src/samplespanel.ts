// A miniSEED record's samples, decoded. The hex view shows what a record
// stores, which for Steim1 and Steim2 is frames of differences rather than
// samples: a sample is the one before it plus a difference, so none of them is
// at any one place in the file. This is where the samples are, with the steps
// that made them and the check the format carries, whichever word, frame or
// sample of the data the cursor is on.
//
// Sample values and the integration constants are written as the core writes
// them, without digit grouping, so a constant can be compared with the sample
// it should equal by eye. Counts are grouped.

import type { SamplesInfo } from "./doc.ts";
import { countText } from "./strings.ts";

function span(cls: string, text: string): HTMLElement {
  const e = document.createElement("span");
  e.className = cls;
  e.textContent = text;
  return e;
}

function line(cls: string, text: string): HTMLElement {
  const e = document.createElement("div");
  e.className = cls;
  e.textContent = text;
  return e;
}

/** How many samples, for the note beside the heading. A record that decoded
 *  fewer than its header gives says both, since the shortfall is the news. */
export function samplesNote(info: SamplesInfo): string {
  if (info.total === info.declared) return countText(info.declared, "sample");
  return `${info.total.toLocaleString()} of ${countText(info.declared, "sample")}`;
}

/** The encoding, the byte order and the size of the data, on one line. */
export function encodingLine(info: SamplesInfo): string {
  const order = info.big_endian ? "big-endian" : "little-endian";
  return `${info.encoding}, ${order}, ${info.bytes.toLocaleString()} bytes of data`;
}

/** The reverse integration constant check, or null where there is none. */
export function checkLine(info: SamplesInfo): { readonly ok: boolean; readonly text: string } | null {
  if (info.check === null || info.xn === null) return null;
  const last = info.last;
  return info.check
    ? { ok: true, text: `Check passed: last sample ${last} equals the reverse integration constant.` }
    : { ok: false, text: `Check failed: last sample ${last} does not equal the reverse integration constant ${info.xn}.` };
}

/** What sits under the row of samples: how many are shown of how many, and the
 *  last one, which is only the record's last sample when every sample was
 *  decoded. Null when the row already shows them all. */
export function showingLine(info: SamplesInfo): string | null {
  const shown = info.values.length;
  if (shown >= info.total) return null;
  if (info.total === info.declared) {
    return `Showing the first ${shown.toLocaleString()} of ${countText(info.declared, "sample")}. Last sample: ${info.last}`;
  }
  return `Showing the first ${shown.toLocaleString()} of the ${countText(info.total, "sample")} decoded.`;
}

/** One step of the walk: a short label, what it says, and a note to the right. */
export type StepRow = { readonly label: string; readonly text: string; readonly note: string };

/** The heading row over the frames, whose columns the frame rows fill. */
export const FRAME_COLUMNS: StepRow = { label: "frame", text: "differences", note: "samples" };

/** Frame rows, under [`FRAME_COLUMNS`]: the frame's number, the differences it
 *  held (and how many were used, where not all were), and the samples they
 *  made. Frame 0's count includes the skipped first difference, which is where
 *  sample 0 stands, so adding the counts up gives sample numbers. */
export function frameRows(info: SamplesInfo): StepRow[] {
  let next = 0;
  return info.frames.map((frame, f) => {
    const from = next;
    next += frame.used;
    const text =
      frame.used === frame.held ? frame.held.toLocaleString() : `${frame.used.toLocaleString()} of ${frame.held.toLocaleString()}`;
    const note =
      frame.used === 0 ? "" : frame.used === 1 ? from.toLocaleString() : `${from.toLocaleString()} to ${(next - 1).toLocaleString()}`;
    return { label: f.toLocaleString(), text, note };
  });
}

function stepLine(row: StepRow, cls = "insp-orow is-step"): HTMLElement {
  const e = document.createElement("div");
  e.className = cls;
  e.append(span("insp-orow-object", row.label), span("insp-orow-text", row.text), span("insp-orow-size", row.note));
  return e;
}

/**
 * The whole panel: what the data is, whether it decoded and checked, the
 * samples, and then the steps.
 *
 * The check or the problem comes before the samples, because a reader should
 * know the row of numbers is partial before reading it. The constants sit
 * outside the scrolling list of frames, so sixty frames cannot push the reverse
 * constant out of sight.
 */
export function samplesBody(info: SamplesInfo): DocumentFragment {
  const frag = document.createDocumentFragment();
  frag.append(line("insp-qcount", encodingLine(info)));

  const check = checkLine(info);
  if (check !== null) frag.append(line(`insp-check-result ${check.ok ? "ok" : "bad"}`, check.text));
  if (info.problem !== "") frag.append(line("insp-xproblem", info.problem));

  if (info.values.length > 0) {
    const all = info.values.length >= info.total;
    frag.append(span("insp-qsubhead", all ? "Samples" : "First samples"));
    const values = document.createElement("div");
    values.className = "insp-orow is-values";
    values.append(span("insp-orow-text", info.values.join("  ")));
    frag.append(values);
    const showing = showingLine(info);
    if (showing !== null) frag.append(line("insp-qcount", showing));
  }

  if (info.steim || info.rule !== "") {
    frag.append(span("insp-qsubhead", "How the samples were decoded"));
  }
  if (info.rule !== "") {
    frag.append(stepLine({ label: "rule", text: info.rule, note: "SEED 2.4" }));
  }
  if (info.steim && info.x0 !== null && info.xn !== null) {
    frag.append(stepLine({ label: "sample 0", text: `${info.x0}, the forward integration constant`, note: "" }));
    if (info.first_difference !== null) {
      const text = `first difference ${info.first_difference}, measured from the previous record's last sample`;
      frag.append(stepLine({ label: "skipped", text, note: "" }));
    }
    if (info.frames.length > 0) {
      // A table rather than a sentence a frame: sixty rows of the same words
      // hide the one number that changes.
      const list = document.createElement("div");
      list.className = "insp-orows";
      list.append(stepLine(FRAME_COLUMNS, "insp-orow is-step is-head"));
      for (const row of frameRows(info)) list.append(stepLine(row));
      frag.append(list);
    }
    const reverse = `${info.xn}, the reverse integration constant; the last sample should equal it`;
    frag.append(stepLine({ label: "check", text: reverse, note: "" }));
    if (info.frames.length < info.frames_walked) {
      frag.append(
        line("insp-qcount", `Showing the first ${info.frames.length.toLocaleString()} of ${countText(info.frames_walked, "frame")}.`),
      );
    }
    if (info.frames_walked < info.frames_in_record) {
      frag.append(
        line("insp-qcount", `${info.frames_walked.toLocaleString()} of ${countText(info.frames_in_record, "frame")} used.`),
      );
    }
  }
  return frag;
}
