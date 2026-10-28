// A PNG's palette, drawn as the colours it is rather than as a list of
// triples.
//
// PLTE is up to 256 entries of three bytes, and read field by field it is
// seven hundred rows of numbers with the colours nowhere in them. The reader's
// question about a palette is what colours it has, so the chunk is drawn as a
// grid of them, with the alphas the tRNS chunk gives the first entries laid
// over a checkerboard. The card is the same kind of thing as a JPEG's
// quantisation square (`jpegcards.ts`), and is found by the same registry
// (`formatcards.ts`).
//
// Every swatch is the entry it was read from, and the chunk's own length and
// checksum are on the card's line, so nothing the card stands in for is out of
// reach: a click on any of them selects that field.

import type { Doc, TemplateNode } from "./doc.ts";
import { paint } from "./colour.ts";
import { fieldButton, head, kid, kids, numberOf } from "./cardparts.ts";
import { problemGlyph, tagged as el } from "./dom.ts";
import type { Item } from "./flatten.ts";
import type { DrawContext } from "./listingdraw.ts";
import { PNG } from "./strings.ts";

/** The chunks a PNG has a card for. Only the palette so far. */
export type PngCardKind = "palette";

/** Chunks after the palette searched for its tRNS. The specification puts
 *  tRNS after PLTE and before the first IDAT, and an encoder writing a dozen
 *  chunks between them is one nobody has met. */
const TRNS_SEARCH = 16;

/** The fields of a chunk, by the place the template gives them. */
const LENGTH = 0;
const TYPE = 1;
const DATA = 2;
const CRC = 3;

/** Whether this node is a PNG chunk with a card, and which one. Asked of every
 *  heading the listing opens, so the first test rules out nearly all of them. */
export function pngCardKind(doc: Doc, node: TemplateNode): PngCardKind | null {
  if (node.type !== "Chunk" || node.child_count !== 4) return null;
  return kid(doc, node, DATA)?.type === "PLTE" ? "palette" : null;
}

export function drawPngCard(c: DrawContext, item: Extract<Item, { kind: "formatcard" }>): HTMLElement {
  const host = el("div", `rp-item rp-jc pc-${item.card}`);
  const chunk = item.node;
  const fields = kids(c.doc, chunk);
  const entries = fields === null ? null : kid(c.doc, fields[DATA] as TemplateNode, 0);
  const colours = entries === null ? null : kids(c.doc, entries);
  if (fields === null || colours === null) {
    host.append(el("div", "bs-wait", PNG.waiting));
    return host;
  }
  const alphas = transparency(c.doc, chunk);
  const seeThrough = alphas.filter((a) => a < 255).length;
  host.append(head(PNG.paletteHead(colours.length), seeThrough === 0 ? "" : PNG.paletteSeeThrough(seeThrough)));
  const grid = el("div", "pc-grid");
  colours.forEach((entry, i) => {
    const colour = entry.colour;
    const alpha = alphas[i] ?? null;
    const said = PNG.paletteEntry(i, colour ?? entry.value, alpha);
    const swatch = fieldButton(c, entry, "pc-swatch", "");
    swatch.title = said;
    swatch.setAttribute("aria-label", said);
    if (colour !== undefined) paint(swatch, alpha === null ? colour : `${colour}${alpha.toString(16).padStart(2, "0")}`);
    grid.append(swatch);
  });
  host.append(grid);
  host.append(chunkLine(c, fields));
  host.append(el("p", "jc-note", PNG.paletteNote));
  return host;
}

/** The alphas the tRNS chunk gives the first entries of the palette, one a
 *  byte, or none where the file has no tRNS. Read from the chunks after this
 *  one, as far as the first IDAT, which tRNS has to come before. */
function transparency(doc: Doc, plte: TemplateNode): number[] {
  const at = plte.path[plte.path.length - 1] ?? 0;
  const list = plte.path.slice(0, -1);
  const reply = doc.templateChildren(list, at + 1, at + 1 + TRNS_SEARCH);
  if (reply.status !== "ok") return [];
  for (const chunk of reply.node) {
    const type = kid(doc, chunk, TYPE)?.value;
    if (type === "IDAT") return [];
    if (type !== "tRNS") continue;
    const data = kid(doc, chunk, DATA);
    // A palette image's tRNS is a structure of one list; any other colour
    // type's is a single colour and gives the palette nothing.
    if (data === null || data.type !== "tRNSPalette") return [];
    const list = kid(doc, data, 0);
    return (list === null ? null : kids(doc, list))?.map(numberOf) ?? [];
  }
  return [];
}

/** The chunk's length and its checksum, which the grid stands in for, as the
 *  card's last line. A checksum that does not match is marked as the listing
 *  marks one. */
function chunkLine(c: DrawContext, fields: readonly TemplateNode[]): HTMLElement {
  const line = el("div", "jc-line");
  const length = fields[LENGTH];
  const crc = fields[CRC];
  if (length !== undefined) line.append(fieldButton(c, length, "jc-fact jc-dim", PNG.chunkLength(numberOf(length))));
  if (crc !== undefined) {
    const sum = numberOf(crc);
    const text = PNG.chunkCrc(Number.isFinite(sum) ? `0x${sum.toString(16).padStart(8, "0")}` : crc.value);
    const fact = fieldButton(c, crc, "jc-fact jc-dim", text);
    if (crc.problem !== undefined) {
      fact.prepend(problemGlyph(crc.problem.tier === "invalid"));
      fact.title = `${fact.title}\n${crc.problem.text}`;
    }
    line.append(fact);
  }
  return line;
}
