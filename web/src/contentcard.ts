// The content card: what an image file is a picture of, at the top of its
// listing, before the parts of the file that encode it.
//
// The first step of reading a file for its content rather than its bytes, and
// an honest one: the browser decodes the whole file and the card shows the
// result, and that is all. There is no mapping from a pixel back to the bytes
// that made it, so the card offers none; clicking the picture only changes its
// size on screen.
//
// The decode is asynchronous and the card's element is not: the listing tears
// its rows down and builds them again on every scroll, so the state of the
// decode lives here, once per document, and the element is drawn from it.

import { formatBytes } from "./doc.js";
import type { Doc } from "./doc.js";
import type { CardKind, Item } from "./flatten.js";
import { el } from "./listingdraw.js";
import { REPORT } from "./strings.js";

/** The most the card will hand the browser to decode. Past this the picture
 *  is not worth the memory it costs while the reader is looking at bytes. */
export const IMAGE_LIMIT_BYTES = 64 * 1024 * 1024;

/** Templates whose file the browser can draw as a picture, and what to tell
 *  it the bytes are. Only formats the browser decodes natively: a TIFF or a
 *  PSD has a template and no card. */
const IMAGE_TEMPLATES: ReadonlyMap<string, string> = new Map([
  ["png", "image/png"],
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
  | { readonly status: "ready"; readonly url: string; readonly width: number; readonly height: number }
  | { readonly status: "failed" }
  | { readonly status: "too-large"; readonly bytes: number };

/** How long an edited file waits before its picture is decoded again. A
 *  typed edit is several changes in a row, and each decode is the whole file. */
const REDECODE_MS = 300;

class ImageCard {
  state: ImageState = { status: "loading" };
  /** Whether the picture is shown at every pixel rather than scaled to fit. */
  full = false;
  /** The identification sentence, once the rules have answered. */
  identity: string | null = null;
  private readonly listeners = new Set<() => void>();
  private started = false;
  /** The edit the picture was decoded from, so a later one is noticed. */
  private decodedAt = -1;
  private redecode = 0;

  constructor(private readonly doc: Doc) {}

  watch(fn: () => void): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }

  private notify(): void {
    for (const fn of this.listeners) fn();
  }

  toggleFull(): void {
    if (this.state.status !== "ready") return;
    this.full = !this.full;
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
      this.set({ status: "ready", url, width: img.naturalWidth, height: img.naturalHeight });
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

/** Be told when the card has something new to show: the decode finished, the
 *  identification arrived, or the reader changed the picture's size. The
 *  element is the listing's to draw again; its height will have changed. */
export function watchCard(doc: Doc, fn: () => void): () => void {
  return imageCardFor(doc).watch(fn);
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
      const frame = el("div", `rp-card-pic${card.full ? " is-full" : ""}`);
      const img = document.createElement("img");
      img.src = s.url;
      img.width = s.width;
      img.height = s.height;
      img.alt = REPORT.imagePixels(s.width, s.height);
      img.title = card.full ? REPORT.imageShowFit : REPORT.imageShowFull;
      img.addEventListener("click", (e) => {
        e.stopPropagation();
        card.toggleFull();
      });
      frame.append(img);
      body.append(frame);
      break;
    }
  }
  if (card.identity !== null) body.append(el("p", "rp-card-id", card.identity));
  host.append(body);
  return host;
}
