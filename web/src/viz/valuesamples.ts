// A miniSEED record's decoded samples as a line, in the At cursor panel.
//
// Part of candidate `value-plot` (`valueplot.ts` has the whole piece's
// story). The core decodes a Steim record's samples (`SamplesInfo`, the
// first 32 of them) and the samples panel (`samplespanel.ts`) lists them as
// numbers; this draws them with the same line as a run (`valueline.ts`).
//
// A Steim sample is the sum of the differences before it, so it is at no one
// place in the file. Hovering one lights the 64-byte frame its difference was
// read from, and a click selects that frame. Sample 0 is the exception: it is
// stored whole, as the forward integration constant, and lights that word.

import type { Doc, SamplesInfo } from "../doc.ts";
import { formatOffset } from "../format.ts";
import { frameOfSample, showingLine } from "../samplespanel.ts";
import { VIZ } from "../strings.ts";
import type { BitRange } from "./linked.ts";
import { drawLinePlot, type LineData } from "./valueline.ts";

/** The At cursor panel's figure is this tall: a glance at the record, with the
 *  pane for anything longer. */
const SECTION_PX = 150;

/** The last field asked about and its answer, since the inspector asks
 *  `applies` and then `draw` of the same field on one render. */
let lastAsked: { doc: Doc; key: string; info: SamplesInfo | null } | null = null;

/** The documents whose edits already forget `lastAsked`. */
const watched = new WeakSet<Doc>();

export function samplesAt(doc: Doc, path: readonly number[]): SamplesInfo | null {
  if (!watched.has(doc)) {
    watched.add(doc);
    // An edit to a Steim word changes the samples, and the bytes arriving can
    // turn a record that would not decode into one that does.
    doc.onChange(() => {
      if (lastAsked?.doc === doc) lastAsked = null;
    });
  }
  const key = path.join(".");
  if (lastAsked !== null && lastAsked.doc === doc && lastAsked.key === key) return lastAsked.info;
  const reply = doc.typeInfo(path);
  const info = reply.status === "ok" && reply.node.kind === "samples" ? reply.node : null;
  lastAsked = { doc, key, info };
  return info;
}

/** Where the record's data starts: the field around `path` that is exactly the
 *  data the samples were decoded from. */
export function dataStart(doc: Doc, path: readonly number[], info: SamplesInfo): number | null {
  for (let n = path.length; n > 0; n--) {
    const r = doc.templateNode(path.slice(0, n));
    if (r.status === "ok" && r.node.size_bits === info.bytes * 8) return r.node.offset_bits;
  }
  return null;
}

/** A Steim frame is 64 bytes, by the SEED manual. */
const STEIM_FRAME_BITS = 64 * 8;

export function samplesSection(doc: Doc, path: readonly number[], width: number): HTMLElement | null {
  const info = samplesAt(doc, path);
  if (info === null || info.values.length < 2) return null;
  const values = Float64Array.from(info.values, Number);
  const start = dataStart(doc, path, info);
  const root = document.createElement("figure");
  root.className = "vp";
  const head = document.createElement("figcaption");
  head.className = "vp-head";
  const title = document.createElement("span");
  title.className = "vp-title";
  title.textContent = VIZ.valuePlot.samplesTitle;
  const note = document.createElement("span");
  note.className = "vp-note";
  // Only when the plot is short of the record: the panel's own heading above
  // already gives the count.
  const showing = showingLine(info);
  note.textContent = showing ?? "";
  note.hidden = showing === null;
  const reset = document.createElement("button");
  reset.type = "button";
  reset.className = "viz-reset";
  reset.textContent = VIZ.resetZoom;
  head.append(title, reset);
  /** Sample 0 is the forward integration constant, the second 32-bit word of
   *  frame 0; every other sample is lit by the frame its difference is in. */
  const constant = info.steim && info.x0 !== null && start !== null;
  const frameBits = (frame: number): BitRange | null => {
    if (frame === 0 && constant && start !== null) return { startBit: start + 32, endBit: start + 64 };
    const f = info.steim ? frameOfSample(info, frame) : null;
    if (f === null || start === null) return null;
    return { startBit: start + f * STEIM_FRAME_BITS, endBit: start + (f + 1) * STEIM_FRAME_BITS };
  };
  const data: LineData = {
    channels: [{ values, stride: 1, channel: 0 }],
    names: [""],
    rowWord: "sample",
    unit: "sample",
    rate: null,
    float: false,
    locate: (frame) => {
      const bits = frameBits(frame) ?? { startBit: start ?? 0, endBit: (start ?? 0) + info.bytes * 8 };
      const f = info.steim ? frameOfSample(info, frame) : null;
      const at = formatOffset(bits.startBit);
      const foot = frame === 0 && constant ? VIZ.valuePlot.fromConstant(at) : f === null ? at : VIZ.valuePlot.fromFrame(f, at);
      return { pick: null, bits, foot };
    },
    // A sample is not a field of its own, so no selection is one.
    selected: () => null,
  };
  const line = drawLinePlot(doc, data, { width, height: SECTION_PX });
  reset.addEventListener("click", () => line.board.home(true));
  root.append(head, line.el, note);
  return root;
}
