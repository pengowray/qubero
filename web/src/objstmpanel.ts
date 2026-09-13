// A PDF object stream, opened. Most of the small objects in a modern PDF are
// not written in the file on their own: they are compressed together inside
// another object, so the hex view can only show the compressed bytes. This is
// where the objects are. Nothing here can be clicked through to, because none
// of these bytes are in the file.

import type { ObjStmObject, ObjStmInfo } from "./doc.ts";
import { countText } from "./strings.ts";

function span(cls: string, text: string): HTMLElement {
  const e = document.createElement("span");
  e.className = cls;
  e.textContent = text;
  return e;
}

/** How many objects are in here, for the note beside the heading. */
export function objstmNote(info: ObjStmInfo): string {
  return countText(info.total, "object");
}

/** One object: its number, and the text it is written in. */
function objectLine(o: ObjStmObject): HTMLElement {
  const line = document.createElement("div");
  line.className = "insp-orow";
  line.append(span("insp-orow-object", `${o.number.toLocaleString()} 0`));
  const text = span("insp-orow-text", o.text);
  // The ellipsis is grey and outside the object's own text, so an object that
  // really does end in dots is not read as a cut one.
  if (o.cut) text.append(span("insp-orow-cut", "…"));
  line.append(text);
  line.append(span("insp-orow-size", `${o.len.toLocaleString()} B`));
  return line;
}

/**
 * The whole panel: what the stream held, then the objects themselves.
 *
 * A stream that would not open still gets a panel, for the same reason the
 * cross-reference one does: what the dictionary asked for and the reason it
 * could not be done are together how a reader tells an odd file from a gap in
 * this program.
 */
export function objstmBody(info: ObjStmInfo): DocumentFragment {
  const frag = document.createDocumentFragment();

  const sizes = document.createElement("div");
  sizes.className = "insp-qcount";
  const packed = `${info.packed.toLocaleString()} bytes compressed`;
  const unpacked = info.decoded > 0 ? `, ${info.decoded.toLocaleString()} decompressed` : "";
  sizes.textContent = packed + unpacked;
  frag.append(sizes);

  if (info.problem !== "") {
    const p = document.createElement("div");
    p.className = "insp-xproblem";
    p.textContent = info.problem;
    frag.append(p);
    return frag;
  }

  frag.append(span("insp-qsubhead", "These objects are stored in the compressed data and have no file offsets."));

  if (info.extends >= 0) {
    frag.append(
      span(
        "insp-qcount",
        `Extends object stream ${info.extends.toLocaleString()}; its objects are not listed here (/Extends).`,
      ),
    );
  }

  const head = document.createElement("div");
  head.className = "insp-orow is-head";
  head.append(
    span("insp-orow-object", "Object"),
    span("insp-orow-text", "Contents"),
    span("insp-orow-size", "Size"),
  );
  frag.append(head);

  const list = document.createElement("div");
  list.className = "insp-orows";
  for (const o of info.objects) list.append(objectLine(o));
  frag.append(list);

  if (info.objects.length < info.total) {
    frag.append(
      span(
        "insp-qcount",
        `Showing the first ${info.objects.length.toLocaleString()} of ${info.total.toLocaleString()} objects.`,
      ),
    );
  }
  return frag;
}
