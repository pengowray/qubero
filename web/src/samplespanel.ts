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

import type { TypeInfo } from "./doc.ts";
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
export function samplesNote(info: TypeInfo): string {
  if (info.mseed_total === info.mseed_declared) return countText(info.mseed_declared, "sample");
  return `${info.mseed_total.toLocaleString()} of ${countText(info.mseed_declared, "sample")}`;
}

/** The encoding, the byte order and the size of the data, on one line. */
export function encodingLine(info: TypeInfo): string {
  const order = info.mseed_big_endian ? "big-endian" : "little-endian";
  return `${info.mseed_encoding}, ${order}, ${info.mseed_bytes.toLocaleString()} bytes of data`;
}

/** The reverse integration constant check, or null where there is none. */
export function checkLine(info: TypeInfo): { readonly ok: boolean; readonly text: string } | null {
  if (info.mseed_check === null || info.mseed_xn === null) return null;
  const last = info.mseed_last;
  return info.mseed_check
    ? { ok: true, text: `Check passed: last sample ${last} equals the reverse integration constant.` }
    : { ok: false, text: `Check failed: last sample ${last} does not equal the reverse integration constant ${info.mseed_xn}.` };
}

/** What sits under the row of samples: how many are shown of how many, and the
 *  last one, which is only the record's last sample when every sample was
 *  decoded. Null when the row already shows them all. */
export function showingLine(info: TypeInfo): string | null {
  const shown = info.mseed_values.length;
  if (shown >= info.mseed_total) return null;
  if (info.mseed_total === info.mseed_declared) {
    return `Showing the first ${shown.toLocaleString()} of ${countText(info.mseed_declared, "sample")}. Last sample: ${info.mseed_last}`;
  }
  return `Showing the first ${shown.toLocaleString()} of the ${countText(info.mseed_total, "sample")} decoded.`;
}

/** One step of the walk: a short label, what it says, and a note to the right. */
export type StepRow = { readonly label: string; readonly text: string; readonly note: string };

/** Frame rows, with the samples each frame made. Frame 0's count includes the
 *  skipped first difference, which is where sample 0 stands, so adding the
 *  counts up gives sample numbers. */
export function frameRows(info: TypeInfo): StepRow[] {
  let next = 0;
  return info.mseed_frames.map((frame, f) => {
    const from = next;
    next += frame.used;
    const text =
      frame.used === frame.held
        ? countText(frame.held, "difference")
        : `${frame.used.toLocaleString()} of ${countText(frame.held, "difference")} used`;
    const note = frame.used === 0 ? "" : frame.used === 1 ? `sample ${from}` : `samples ${from} to ${next - 1}`;
    return { label: `frame ${f}`, text, note };
  });
}

function stepLine(row: StepRow): HTMLElement {
  const e = document.createElement("div");
  e.className = "insp-orow is-step";
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
export function samplesBody(info: TypeInfo): DocumentFragment {
  const frag = document.createDocumentFragment();
  frag.append(line("insp-qcount", encodingLine(info)));

  const check = checkLine(info);
  if (check !== null) frag.append(line(`insp-check-result ${check.ok ? "ok" : "bad"}`, check.text));
  if (info.problem !== "") frag.append(line("insp-xproblem", info.problem));

  if (info.mseed_values.length > 0) {
    const all = info.mseed_values.length >= info.mseed_total;
    frag.append(span("insp-qsubhead", all ? "Samples" : "First samples"));
    const values = document.createElement("div");
    values.className = "insp-orow";
    values.append(span("insp-orow-text", info.mseed_values.join("  ")));
    frag.append(values);
    const showing = showingLine(info);
    if (showing !== null) frag.append(line("insp-qcount", showing));
  }

  if (info.mseed_steim || info.mseed_rule !== "") {
    frag.append(span("insp-qsubhead", "How the samples were decoded"));
  }
  if (info.mseed_rule !== "") {
    frag.append(stepLine({ label: "rule", text: info.mseed_rule, note: "SEED 2.4" }));
  }
  if (info.mseed_steim && info.mseed_x0 !== null && info.mseed_xn !== null) {
    frag.append(stepLine({ label: "sample 0", text: `${info.mseed_x0}, the forward integration constant`, note: "" }));
    if (info.mseed_first_difference !== null) {
      const text = `first difference ${info.mseed_first_difference}, measured from the previous record's last sample`;
      frag.append(stepLine({ label: "skipped", text, note: "" }));
    }
    if (info.mseed_frames.length > 0) {
      const list = document.createElement("div");
      list.className = "insp-orows";
      for (const row of frameRows(info)) list.append(stepLine(row));
      frag.append(list);
    }
    const reverse = `${info.mseed_xn}, the reverse integration constant; the last sample should equal it`;
    frag.append(stepLine({ label: "check", text: reverse, note: "" }));
    if (info.mseed_frames.length < info.mseed_frames_walked) {
      frag.append(
        line("insp-qcount", `Showing the first ${info.mseed_frames.length.toLocaleString()} of ${countText(info.mseed_frames_walked, "frame")}.`),
      );
    }
    if (info.mseed_frames_walked < info.mseed_frames_in_record) {
      frag.append(
        line("insp-qcount", `${info.mseed_frames_walked.toLocaleString()} of ${countText(info.mseed_frames_in_record, "frame")} used.`),
      );
    }
  }
  return frag;
}
