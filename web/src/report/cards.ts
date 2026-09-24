// The listing's format cards, drawn in the report: a JPEG's quantization
// square, its Huffman code set, its frame summary and its scan line
// (`jpegcards.ts`). Where the listing draws a card for a node, the part's
// section in the report opens with the same card, above its fields.
//
// The card is drawn by the listing's own code with a context made for it
// here. Every number on a card names the field it was read from (`data-reads`,
// `data-at`, `data-size`), and those become the report's own byte references,
// so a click on one moves the cursor there as any other reference does.

import type { Doc, TemplateNode } from "../doc.ts";
import { drawJpegCard, jpegCardKind } from "../jpegcards.ts";
import type { DrawContext } from "../listingdraw.ts";
import { pointAt } from "./refs.ts";

/** The card for a node, or null for a node the listing draws no card for. */
export function formatCard(doc: Doc, node: TemplateNode): HTMLElement | null {
  const kind = jpegCardKind(doc, node);
  if (kind === null) return null;
  const host = document.createElement("div");
  host.className = "rv-card";
  const opened = new Set<string>();
  const draw = (): void => {
    const c: DrawContext = {
      doc,
      segments: [],
      selected: null,
      nearest: undefined,
      bytes: new Set(),
      dumps: new Set(),
      dumpTops: new Map(),
      cards: opened,
      // The long half of a card, opened and put away in place.
      toggleCard: (key) => {
        if (opened.has(key)) opened.delete(key);
        else opened.add(key);
        draw();
      },
      toggleSwitch: () => {},
      toggleBytes: () => {},
      toggleDump: () => {},
      verdict: () => "too-large",
      streams: new Set(),
      shown: true,
    };
    const card = drawJpegCard(c, {
      kind: "formatcard",
      key: `card:${node.path.join("/")}`,
      section: -1,
      depth: 0,
      offsetBits: node.offset_bits,
      sizeBits: node.size_bits,
      card: kind,
      path: node.path,
      node,
    });
    for (const b of card.querySelectorAll<HTMLElement>("[data-reads]")) {
      const at = Number(b.dataset["at"]);
      const size = Number(b.dataset["size"]);
      // The listing's own path key, steps joined with dots.
      const path = (b.dataset["reads"] ?? "").split(".").filter((s) => s !== "").map(Number);
      if (Number.isFinite(at) && Number.isFinite(size)) pointAt(b, { path, startBit: at, endBit: at + size }, b.title);
    }
    host.replaceChildren(card);
  };
  draw();
  return host;
}
