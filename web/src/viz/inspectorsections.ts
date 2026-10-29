// Sections a diagram piece adds to the inspector, after the inspector's own.
//
// The inspector (`inspector.ts`, the At cursor panel) shows the field under
// the cursor. A piece that has a picture of that field, such as the bits of a
// packed flags word or the bytes a varint gathers, registers a section here
// and the inspector draws it under the type section, the way it draws
// `typepanel.ts`'s. The inspector owns the width and passes it; a section
// never measures the panel.
//
// The inspector asks `applies` on every render, which is every cursor move
// and every chunk of the file arriving, so it must be cheap: a look at the
// node's kind, type or size, not a walk. `draw` is asked only when the field
// changes, and again on the next render if it answered null (its bytes were
// still on the way).

import type { Doc, TemplateNode } from "../doc.ts";
import { Registry } from "./registry.ts";

export type InspectorSectionContext = {
  readonly doc: Doc;
  readonly node: TemplateNode;
  readonly path: readonly number[];
  /** The width the section has, in CSS pixels. */
  readonly width: number;
};

export type InspectorSection = {
  readonly id: string;
  readonly applies: (doc: Doc, node: TemplateNode) => boolean;
  readonly draw: (ctx: InspectorSectionContext) => HTMLElement | null;
};

const SECTIONS = new Registry<InspectorSection>("inspector section");

export function registerInspectorSection(section: InspectorSection): void {
  SECTIONS.add(section);
}

/** The sections that have something to say about this field, in the order
 *  they were registered. */
export function inspectorSectionsFor(doc: Doc, node: TemplateNode): InspectorSection[] {
  return SECTIONS.all().filter((s) => s.applies(doc, node));
}
