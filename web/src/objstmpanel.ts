// A PDF object stream, opened. Most of the small objects in a modern PDF are
// not written in the file on their own: they are compressed together inside
// another object, so the hex view can only show the compressed bytes. This is
// where the objects are. Nothing here can be clicked through to, because none
// of these bytes are in the file.

import type { ObjStmObject, TypeInfo } from "./doc.js";
import { countText } from "./strings.js";

function span(cls: string, text: string): HTMLElement {
  const e = document.createElement("span");
  e.className = cls;
  e.textContent = text;
  return e;
}

/** How many objects are in here, for the note beside the heading. */
export function objstmNote(info: TypeInfo): string {
  return countText(info.objstm_total, "object");
}

/** One object: its number, and the text it is written in. */
function objectLine(o: ObjStmObject): HTMLElement {
  const line = document.createElement("div");
  line.className = "insp-orow";
  line.append(span("insp-orow-object", `${o.number.toLocaleString()} 0`));
  line.append(span("insp-orow-text", o.cut ? `${o.text}…` : o.text));
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
export function objstmBody(info: TypeInfo): DocumentFragment {
  const frag = document.createDocumentFragment();

  const sizes = document.createElement("div");
  sizes.className = "insp-qcount";
  const packed = `${info.objstm_packed.toLocaleString()} bytes compressed`;
  const unpacked = info.objstm_decoded > 0 ? `, ${info.objstm_decoded.toLocaleString()} decompressed` : "";
  sizes.textContent = packed + unpacked;
  frag.append(sizes);

  if (info.problem !== "") {
    const p = document.createElement("div");
    p.className = "insp-xproblem";
    p.textContent = info.problem;
    frag.append(p);
    return frag;
  }

  frag.append(span("insp-qsubhead", "These objects are in the compressed data, not at offsets in the file."));

  if (info.objstm_extends >= 0) {
    frag.append(
      span("insp-qcount", `More objects continue in object ${info.objstm_extends.toLocaleString()} (/Extends).`),
    );
  }

  const head = document.createElement("div");
  head.className = "insp-orow is-head";
  head.append(span("insp-orow-object", "Object"), span("insp-orow-text", "Contents"));
  frag.append(head);

  const list = document.createElement("div");
  list.className = "insp-orows";
  for (const o of info.objstm_objects) list.append(objectLine(o));
  frag.append(list);

  if (info.objstm_objects.length < info.objstm_total) {
    frag.append(
      span(
        "insp-qcount",
        `Showing the first ${info.objstm_objects.length.toLocaleString()} of ${info.objstm_total.toLocaleString()} objects.`,
      ),
    );
  }
  return frag;
}
