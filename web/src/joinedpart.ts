// A stream joined from several runs, as the inspector talks about it: where a
// byte of it is kept, and whether the field at the cursor is the place to offer
// the stream as a tab of its own. Kept apart from `inspector.ts` so that both
// can be read and tested without a document.

import { ADDRESS_MARK, formatOffset, offsetDigits } from "./format.ts";
import { JOINED, UNPACKED } from "./strings.ts";
import type { JoinedPart, TemplateNode } from "./doc.ts";

/** The most a joined stream may come to and still open as a document of its
 *  own: `CAP_BYTES` in the core's `codec.rs`, in bits. */
export const JOINED_WHOLE_CAP_BITS = 64 * 1024 * 1024 * 8;

/** One row under the heading. */
export type PartLine = {
  readonly text: string;
  /** What the `+` in `text` counts from, or null for a row with no `+`. */
  readonly plus: string | null;
  /** The hover over the whole row, where it has one. */
  readonly title: string | null;
  /** The field the row leads to, or null for a row with nowhere to go. */
  readonly path: readonly number[] | null;
  /** False for the decoder's step, which says what made the byte rather than
   *  naming a place, and is drawn as the status bar's line is. */
  readonly place: boolean;
};

/** A heading and the rows under it. */
export type PartGroup = { readonly head: string; readonly lines: readonly PartLine[] };

/**
 * Where a byte of a joined stream is kept: the run, with the byte's place in
 * what that run gives, and then, where they have an answer, where the byte is
 * in the file for a run stored as it sits, and the virtual offset a BAI or a
 * CSI names it by for a BGZF block.
 *
 * `file` is null in the file's own view, where the run's path is a field to go
 * to and `the file` is the document the panel is about. In a tab of the
 * stream's own it is the file's name: the path names a field of the file, and
 * followed there would land on some field of the tab instead, and `the file`
 * could be read as the tab.
 */
function placeLines(part: JoinedPart, file: string | null): PartLine[] {
  const at = `${ADDRESS_MARK}+${offsetDigits(part.in_part * 8)}`;
  const lines: PartLine[] = [
    { text: JOINED.at(at, part.label, part.packed), plus: JOINED.plusTitle(part.label, part.packed), title: null, path: file === null ? part.path : null, place: true },
  ];
  if (!part.packed && part.run_space === 0) {
    const inFile = formatOffset(part.run_offset_bits + part.in_part * 8);
    lines.push({ text: file === null ? JOINED.inFile(inFile) : JOINED.inNamedFile(inFile, file), plus: null, title: null, path: null, place: true });
  }
  if (part.block_offset !== null && part.in_block !== null) {
    const voffset = (BigInt(part.block_offset) << 16n) | BigInt(part.in_block);
    lines.push({
      text: `${JOINED.virtualLabel} ${JOINED.virtual(voffset.toString(), String(part.block_offset), String(part.in_block))}`,
      plus: null,
      title: JOINED.virtualTitle(formatOffset(part.block_offset * 8)),
      path: null,
      place: true,
    });
  }
  return lines;
}

/** For a field read where the file declares the stream: the run the field's
 *  first byte is in. */
export function startsInGroup(part: JoinedPart): PartGroup {
  return { head: JOINED.startsIn, lines: placeLines(part, null) };
}

/**
 * For a field of the stream opened as a tab of its own, in the file `file`:
 * where the byte under the cursor is kept, which in a tab over a stream of
 * plain bytes is the only question worth answering, since the one field there
 * starts in the first run wherever the cursor is. When the panel is pinned to
 * a field the cursor is not in, `part` is about the field's first byte and the
 * heading is the file's own view's.
 *
 * `step` is the decoder's line for the same byte, under the heading an
 * unpacked stream's tab gives it, since it says what made the byte and not
 * where the byte is. It is kept only for a run that was unpacked: a run stored
 * as it sits is one step over the whole run, and its line would say again
 * where the run is.
 */
export function tabGroups(part: JoinedPart, file: string, underCursor: boolean, step: string | null): PartGroup[] {
  const groups: PartGroup[] = [{ head: underCursor ? JOINED.underCursor : JOINED.startsIn, lines: placeLines(part, file) }];
  if (part.packed && step !== null) {
    groups.push({ head: UNPACKED.originHead, lines: [{ text: step, plus: null, title: null, path: null, place: false }] });
  }
  return groups;
}

/** What the inspector offers for the field at the cursor: a stream to open, or
 *  the reason one will not open. */
export type StreamOffer =
  | { readonly open: readonly number[]; readonly joined: boolean }
  | { readonly refused: "too-large" };

/**
 * Whether the field is where a stream is offered as a tab of its own, and
 * which stream.
 *
 * A compressed run is offered on its own node, and one that would not open
 * gets nothing here, since the reason is already beside its address. A joined
 * stream is offered where the listing offers it, on the one node the stream
 * holds, and on the node that joined it, which is zero bits in the file and
 * reached from the listing or the trail. `first` is that node's first child.
 *
 * A joined stream past the cap is refused before anyone asks, since its
 * length is known; the core would refuse it the same way. One with a part
 * that will not read is found out only when it is joined whole.
 */
export function streamOffer(path: readonly number[], n: TemplateNode, first: TemplateNode | null): StreamOffer | null {
  if (n.decoded) return n.refused === null ? { open: path, joined: false } : null;
  if (n.joined && n.space_root && path.length > 0) return joinedOffer(path.slice(0, -1), n.size_bits);
  if (first !== null && first.joined && first.space_root) return joinedOffer(path, first.size_bits);
  return null;
}

function joinedOffer(path: readonly number[], bits: number): StreamOffer {
  return bits > JOINED_WHOLE_CAP_BITS ? { refused: "too-large" } : { open: path, joined: true };
}
