// Fields that hold a whole embedded file: a zip entry's data, a gzip member's
// compressed body, or any plain run of bytes. Each gets a plan saying what the
// bytes are, what they decompress with, and how to load them, so the inspector
// can offer to open them as a document of their own.
//
// Like the integrity checks in inspector.ts, the format knowledge here is keyed
// by template name and field name on the JS side; the core stays a describer of
// the bytes that are literally in the file.

import { formatBytes } from "./doc.js";
import type { Doc, TemplateNode } from "./doc.js";

/** The most this will open into a tab, packed or unpacked. Everything opened
 *  this way is held in memory whole, unlike a file on disk, which is streamed. */
export const OPEN_LIMIT = 512 * 1024 * 1024;

export type OpenPlan = {
  /** What the opened document is called: a zip entry's own name where the
   *  format records one, otherwise the field's name. */
  readonly name: string;
  /** One line about the bytes: `readme.txt · deflate, 1.2 KiB, unpacks to 3.4 KiB`. */
  readonly detail: string;
  /** Where the bytes came from, for the tab's tooltip: parent file, field,
   *  offset, and how they were unpacked. */
  readonly origin: string;
  /** Null when the bytes can be described but not opened: an unsupported
   *  compression method, or a run over `OPEN_LIMIT`. The detail line says why. */
  readonly load: (() => Promise<Uint8Array>) | null;
};

/**
 * A plan for the field at `path`, or null when its bytes are not something to
 * open: a number, a structure, a bit-unaligned run.
 */
export function openPlan(doc: Doc, path: readonly number[], n: TemplateNode): OpenPlan | null {
  if (n.composite || n.kind !== "bytes") return null;
  if (n.offset_bits % 8 !== 0 || n.size_bits % 8 !== 0 || n.size_bits === 0) return null;
  const at = n.offset_bits / 8;
  const packed = n.size_bits / 8;
  const siblings = siblingNodes(doc, path);

  if (doc.isZip && n.name === "data") {
    const method = numberField(siblings, "compression");
    const entry = leafName(textField(doc, siblings, "name")) ?? n.name;
    // `unpacked_size` is the header's number once a ZIP64 entry's placeholder
    // has been answered from its extra fields; older templates have neither.
    const unpacked = numberField(siblings, "unpacked_size") ?? numberField(siblings, "uncompressed_size");
    const where = `${entry} in ${doc.name}, ${formatBytes(packed)} at offset 0x${at.toString(16)}`;
    if (method === 0) {
      return withLimit(packed, {
        name: entry,
        detail: `${entry} · stored (not compressed), ${formatBytes(packed)}`,
        origin: `${where}, stored`,
        load: () => loadBytes(doc, at, packed),
      });
    }
    if (method === 8) {
      // Zero means the header does not say, as a streamed entry's does not:
      // its real size is in the data descriptor after the data.
      const grows = unpacked !== null && unpacked !== 0 && unpacked !== packed ? `, unpacks to ${formatBytes(unpacked)}` : "";
      return withLimit(Math.max(packed, unpacked ?? 0), {
        name: entry,
        detail: `${entry} · deflate, ${formatBytes(packed)}${grows}`,
        origin: `${where}, decompressed with deflate`,
        load: async () => inflateRaw(await loadBytes(doc, at, packed)),
      });
    }
    const named = textField(doc, siblings, "compression");
    return {
      name: entry,
      detail: `${entry} · compressed with ${named ?? `method ${method ?? "?"}`}, which this can't decompress`,
      origin: where,
      load: null,
    };
  }

  if (doc.template === "gzip" && n.name === "compressed") {
    const stored = leafName(textField(doc, siblings, "name"));
    const name = stored ?? gzipInnerName(doc.name);
    const unpacked = numberField(siblings, "original_size");
    const grows = unpacked !== null && unpacked !== 0 && unpacked !== packed ? `, unpacks to ${formatBytes(unpacked)}` : "";
    return withLimit(Math.max(packed, unpacked ?? 0), {
      name,
      detail: `${name} · deflate, ${formatBytes(packed)}${grows}`,
      origin: `${name} in ${doc.name}, ${formatBytes(packed)} at offset 0x${at.toString(16)}, decompressed with deflate`,
      load: async () => inflateRaw(await loadBytes(doc, at, packed)),
    });
  }

  // Any other run of bytes opens as it is. What it turns out to be is the new
  // tab's question: the usual template sniffing runs on it there.
  return withLimit(packed, {
    name: n.name,
    detail: `${n.name} · ${formatBytes(packed)}`,
    origin: `${n.name} in ${doc.name}, ${formatBytes(packed)} at offset 0x${at.toString(16)}`,
    load: () => loadBytes(doc, at, packed),
  });
}

/** The same plan, with loading withheld past `OPEN_LIMIT`. */
function withLimit(largest: number, plan: OpenPlan): OpenPlan {
  if (largest <= OPEN_LIMIT) return plan;
  return { ...plan, detail: `${plan.detail} · too big to open here (over ${formatBytes(OPEN_LIMIT)})`, load: null };
}

function siblingNodes(doc: Doc, path: readonly number[]): readonly TemplateNode[] {
  if (path.length === 0) return [];
  const reply = doc.templateChildren(path.slice(0, -1), 0, 128);
  return reply.status === "ok" ? reply.node : [];
}

function numberField(siblings: readonly TemplateNode[], name: string): number | null {
  const f = siblings.find((x) => x.name === name);
  return f === undefined ? null : fieldNumber(f);
}

/** The field's numeric value, or null when there is none. */
export function fieldNumber(f: TemplateNode): number | null {
  const v = Number(f.edit_text);
  if (Number.isFinite(v) && f.edit_text !== "") return v;
  // An enum's edit text is the value's name; the number rides in its shown
  // value: `deflate (8)`.
  const m = /\((\d+)\)\s*$/.exec(f.value);
  return m === null ? null : Number(m[1]);
}

/** The whole text of a sibling text field, or null when there is none. */
function textField(doc: Doc, siblings: readonly TemplateNode[], name: string): string | null {
  const f = siblings.find((x) => x.name === name);
  if (f === undefined) return null;
  if (f.kind === "enum") return f.value === "" ? null : f.value;
  const r = doc.fieldText(f.path);
  if (r.status !== "ok" || r.node.text === "") return null;
  return r.node.text;
}

/** A zip entry is stored under a path; the document it opens as gets the last
 *  part, which is also a name a save dialog will accept. */
function leafName(path: string | null): string | null {
  if (path === null) return null;
  const leaf = path.replace(/\\/g, "/").split("/").pop() ?? "";
  return leaf === "" ? null : leaf;
}

/** What a .gz held, when the header does not say: the name without the .gz. */
function gzipInnerName(name: string): string {
  if (/\.tgz$/i.test(name)) return name.replace(/\.tgz$/i, ".tar");
  if (/\.gz$/i.test(name)) return name.replace(/\.gz$/i, "");
  return `${name} (uncompressed)`;
}

async function loadBytes(doc: Doc, at: number, len: number): Promise<Uint8Array> {
  await doc.ensureRange(at, len);
  const read = doc.read(at, len);
  if (!read.complete) throw new Error("Some of these bytes could not be loaded from the file.");
  return read.bytes;
}

/**
 * A raw deflate stream, inflated with a ceiling on the output. The header's
 * claimed size is not trusted: a crafted file can claim little and expand
 * without bound, so the ceiling is on what actually comes out.
 */
async function inflateRaw(packed: Uint8Array): Promise<Uint8Array> {
  if (typeof DecompressionStream === "undefined") throw new Error("This browser can't decompress deflate data.");
  // Current Chromium implements deflate-raw; older DOM typings only name
  // gzip and deflate.
  const stream = new Blob([Uint8Array.from(packed)]).stream().pipeThrough(new DecompressionStream("deflate-raw" as CompressionFormat));
  const reader = stream.getReader();
  const parts: Uint8Array[] = [];
  let total = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    total += value.length;
    if (total > OPEN_LIMIT) {
      await reader.cancel();
      throw new Error(`Stopped decompressing at ${formatBytes(OPEN_LIMIT)}: the data keeps expanding past what this can open.`);
    }
    parts.push(value);
  }
  const out = new Uint8Array(total);
  let off = 0;
  for (const p of parts) {
    out.set(p, off);
    off += p.length;
  }
  return out;
}
