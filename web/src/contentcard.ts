// The content card: what an image file is a picture of, at the top of its
// listing, before the parts of the file that encode it.
//
// The first step of reading a file for its content rather than its bytes: the
// browser decodes the whole file and the card shows the result, in the pixel
// grid (`viz/pixelgrid.ts`), which zooms and pans. Where the core has traced
// how the pixels were made, which is a baseline JPEG's scan and a PNG's
// scanlines, the grid draws that over the picture and a click on a pixel
// selects the bytes that made it. Any other picture is the browser's decode
// and nothing more, and its pixels lead nowhere.
//
// The decode is asynchronous and the card's element is not: the listing tears
// its rows down and builds them again on every scroll, so the state of the
// decode, the trace drawn over it, and the reader's zoom and overlay choices
// live here, once per document, and the element is drawn from them.

import { formatBytes } from "./doc.ts";
import type { Doc } from "./doc.ts";
import type { CardKind, Item } from "./flatten.ts";
import { tagged as el } from "./dom.ts";
import { REPORT } from "./strings.ts";
import { WAIT } from "./report/section.ts";
import { drawPixelGrid, gridArea, gridState, readLayers, stripWidth, type Layers, type PixelGrid, type PixelGridState } from "./viz/pixelgrid.ts";
import { viewSize } from "./viz/pixelgridmath.ts";

/** The most the card will hand the browser to decode. Past this the picture
 *  is not worth the memory it costs while the reader is looking at bytes. */
export const IMAGE_LIMIT_BYTES = 64 * 1024 * 1024;

/** Templates whose file the browser can draw as a picture, and what to tell
 *  it the bytes are. Only formats the browser decodes natively: a TIFF or a
 *  PSD has a template and no card. A PICO-8 or Picotron cartridge is a PNG
 *  with a program hidden in its pixels, and the picture is the label on the
 *  cartridge, so it is drawn as one. */
const IMAGE_TEMPLATES: ReadonlyMap<string, string> = new Map([
  ["png", "image/png"],
  ["p8png", "image/png"],
  ["p64png", "image/png"],
  ["jpeg", "image/jpeg"],
  ["gif", "image/gif"],
  ["bmp", "image/bmp"],
  ["webp", "image/webp"],
]);

/** What a file with this template opens with, or null for one that is only
 *  its bytes. */
export function cardKind(template: string | null): CardKind | null {
  return template !== null && IMAGE_TEMPLATES.has(template) ? "image" : null;
}

/** Where the decode has got to. */
export type ImageState =
  | { readonly status: "loading" }
  | { readonly status: "decoding" }
  | {
      readonly status: "ready";
      readonly url: string;
      readonly width: number;
      readonly height: number;
      /** The decoded picture, for drawing on a canvas. */
      readonly image: HTMLImageElement;
    }
  | { readonly status: "failed" }
  | { readonly status: "too-large"; readonly bytes: number };

/** How long an edited file waits before its picture is decoded again. A
 *  typed edit is several changes in a row, and each decode is the whole file. */
const REDECODE_MS = 300;
/** How often the card asks again for the trace to draw over the picture
 *  while its bytes or its decode are on the way. Asking is what carries a
 *  large inflate on, a go at a time. */
const LAYERS_RETRY_MS = 150;

class ImageCard {
  state: ImageState = { status: "loading" };
  /** The identification sentence, once the rules have answered. */
  identity: string | null = null;
  /** The overlay, channel and zoom the reader chose, kept across redraws. */
  readonly grid: PixelGridState = gridState();
  /** What the pixel grid draws over the picture, once it has been read. */
  layers: Layers | null = null;
  private layersAsked = false;
  private readonly listeners = new Set<(what: CardChange) => void>();
  private started = false;
  /** The edit the picture was decoded from, so a later one is noticed. */
  private decodedAt = -1;
  private redecode = 0;

  constructor(private readonly doc: Doc) {}

  watch(fn: (what: CardChange) => void): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }

  private notify(what: CardChange = "state"): void {
    for (const fn of this.listeners) fn(what);
  }

  /** The picture on screen has its size now. An image element is nothing
   *  tall until its bytes are decoded for display, which happens after it
   *  is in the document, so a card measured on mounting is a card without
   *  its picture. */
  resized(): void {
    this.notify("size");
  }

  /** Read what the grid draws over the picture, asking again until it is
   *  there; the card is drawn again when it arrives. */
  private readLayers(): void {
    const got = readLayers(this.doc);
    if (got === WAIT) {
      window.setTimeout(() => this.readLayers(), LAYERS_RETRY_MS);
      return;
    }
    this.layers = got;
    this.notify();
  }

  /** Begin the decode, once. Drawing the card asks for this rather than the
   *  card's creation, so a listing that is hidden does not decode a picture
   *  nobody is looking at. */
  start(): void {
    if (this.started) return;
    this.started = true;
    void this.decode();
    this.doc.identify().then(
      (id) => {
        this.identity = id?.message ?? null;
        this.notify();
      },
      () => {},
    );
    // An edit changes the bytes, and the picture is of the bytes. Chunks of a
    // file still arriving change nothing the decode is not already waiting
    // for, and a template change is not an edit; the piece count is what
    // an edit moves.
    this.doc.onChange(() => {
      if (this.decodedAt < 0 || this.doc.pieceCount === this.decodedAt) return;
      clearTimeout(this.redecode);
      this.redecode = window.setTimeout(() => void this.decode(), REDECODE_MS);
    });
  }

  private async decode(): Promise<void> {
    const doc = this.doc;
    const len = doc.lengthBytes;
    if (len > IMAGE_LIMIT_BYTES) {
      this.set({ status: "too-large", bytes: len });
      return;
    }
    // The bytes are read the way every other view reads them, and a file
    // being streamed may not have them all yet: wait for it to change and
    // ask again, until it has.
    try {
      await doc.ensureRange(0, len);
    } catch {
      // A chunk that would not load reads as zeros, and the decode below says
      // what it makes of that.
    }
    let read = doc.read(0, len);
    while (!read.complete) {
      await new Promise<void>((resolve) => {
        const stop = doc.onChange(() => {
          stop();
          resolve();
        });
      });
      read = doc.read(0, len);
    }
    const at = doc.pieceCount;
    this.set({ status: "decoding" });
    const type = IMAGE_TEMPLATES.get(doc.template ?? "") ?? "application/octet-stream";
    // `read` allocates a buffer of exactly this length, so the buffer is the
    // bytes; the cast is for a type that allows it to be shared, which this
    // one never is.
    const url = URL.createObjectURL(new Blob([read.bytes.buffer as ArrayBuffer], { type }));
    const img = new Image();
    img.src = url;
    try {
      await img.decode();
      this.set({ status: "ready", url, width: img.naturalWidth, height: img.naturalHeight, image: img });
      // An edit makes a new trace too: the old one is of the old bytes.
      if (this.layersAsked) this.layers = null;
      this.layersAsked = true;
      this.readLayers();
    } catch {
      URL.revokeObjectURL(url);
      this.set({ status: "failed" });
    }
    this.decodedAt = at;
  }

  private set(state: ImageState): void {
    if (this.state.status === "ready") URL.revokeObjectURL(this.state.url);
    this.state = state;
    this.notify();
  }
}

const cards = new WeakMap<Doc, ImageCard>();

function imageCardFor(doc: Doc): ImageCard {
  let card = cards.get(doc);
  if (card === undefined) {
    card = new ImageCard(doc);
    cards.set(doc, card);
  }
  return card;
}

/** What a watcher is told. `state`: the card has something new to show (the
 *  decode finished, the identification arrived, the reader changed the
 *  picture's size) and is the listing's to draw again. `size`: the element
 *  already up has become the height it is going to be, and only needs
 *  measuring again. */
export type CardChange = "state" | "size";

export function watchCard(doc: Doc, fn: (what: CardChange) => void): () => void {
  return imageCardFor(doc).watch(fn);
}

/** How far the picture of an image file has got, for a view that draws its
 *  own frame round it (the report's facts and its heading over the picture).
 *  Asking starts the decode, as drawing the card does. Null for a file that
 *  is not a picture the browser can draw. */
export function cardState(doc: Doc): ImageState | null {
  if (cardKind(doc.template) === null) return null;
  const card = imageCardFor(doc);
  card.start();
  return card.state;
}

/** The card as one element of the listing. `shown` says whether the listing
 *  is on screen, which is when the decode is worth starting. */
export function drawCard(doc: Doc, item: Extract<Item, { kind: "card" }>, shown: boolean): HTMLElement {
  const card = imageCardFor(doc);
  if (shown) card.start();
  const host = el("div", `rp-item rp-card rp-card-${item.card}`);
  const head = el("div", "rp-card-head");
  head.append(el("b", "rp-name", REPORT.imageCard));
  const s = card.state;
  if (s.status === "ready") head.append(el("span", "rp-size", REPORT.imagePixels(s.width, s.height)));
  host.append(head);
  const body = el("div", "rp-card-body");
  switch (s.status) {
    case "loading":
      body.append(el("p", "rp-card-note", REPORT.imageLoading));
      break;
    case "decoding":
      body.append(el("p", "rp-card-note", REPORT.imageDecoding));
      break;
    case "failed":
      body.append(el("p", "rp-card-note", REPORT.imageFailed));
      break;
    case "too-large":
      body.append(el("p", "rp-card-note", REPORT.imageTooLarge(formatBytes(s.bytes), formatBytes(IMAGE_LIMIT_BYTES))));
      break;
    case "ready": {
      // The grid takes the card's width, which is known only once the card
      // is on the page; it is drawn then, and again if the width changes,
      // with the reader's zoom and overlay kept in the card.
      body.classList.add("is-grid");
      const frame = el("div", "rp-card-grid");
      let grid: PixelGrid | null = null;
      let drawnAt = 0;
      const draw = (width: number): void => {
        if (width <= 0 || width === drawnAt) return;
        drawnAt = width;
        grid?.destroy();
        const area = gridArea(s.width, s.height, card.layers);
        const size = viewSize(area.w, area.h, width, stripWidth(card.layers));
        grid = drawPixelGrid(doc, { width: s.width, height: s.height, image: s.image, layers: card.layers }, { ...size, state: card.grid });
        frame.replaceChildren(grid.el);
        card.resized();
      };
      const watch = new ResizeObserver((entries) => {
        if (!frame.isConnected) {
          watch.disconnect();
          grid?.destroy();
          return;
        }
        const w = entries[0]?.contentRect.width ?? 0;
        draw(Math.floor(w));
      });
      watch.observe(frame);
      body.append(frame);
      break;
    }
  }
  if (card.identity !== null) body.append(el("p", "rp-card-id", card.identity));
  host.append(body);
  return host;
}
