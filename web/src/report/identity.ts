// What the file is, for the title line and the sentences under it: the
// format's name, its Wikipedia article, the sentences the core keeps about it,
// and what identified the file.

import type { Doc, Identification } from "../doc.ts";
import { templateLabel } from "../filetype.ts";
import { namingMatch } from "../signatures.ts";
import type { ReportData } from "./data.ts";
import { WAIT } from "./section.ts";
import { RV } from "./text.ts";

/** The core's sentences about a format, from `formats/about.rs`. */
export type About = {
  /** The format's plain name: `JPEG`. */
  readonly name: string;
  /** The English Wikipedia article's title, with any `#section`. */
  readonly wikipedia: string | null;
  readonly text: string;
};

/**
 * The core's sentences for a template, or null: `format_about` answers the
 * entry as JSON, `{name, wikipedia, text}`, and `null` for a template with
 * none, which is every bundled Kaitai and ImHex format.
 */
export function formatAbout(doc: Doc, template: string): About | null {
  let raw: unknown;
  try {
    const json = doc.formatAbout(template);
    if (json === null) return null;
    raw = JSON.parse(json) as unknown;
  } catch {
    return null;
  }
  if (raw === null || typeof raw !== "object") return null;
  const r = raw as Record<string, unknown>;
  const text = typeof r["text"] === "string" ? r["text"] : "";
  if (text === "") return null;
  return {
    name: typeof r["name"] === "string" ? r["name"] : "",
    wikipedia: typeof r["wikipedia"] === "string" && r["wikipedia"] !== "" ? r["wikipedia"] : null,
    text,
  };
}

/** A Wikipedia article's address: spaces as underscores, and a `#section`
 *  kept as the fragment. */
export function wikipediaLink(title: string): string {
  const [page = "", section] = title.split("#", 2);
  const enc = (s: string): string => encodeURIComponent(s.replace(/ /g, "_")).replace(/%2F/g, "/");
  return `https://en.wikipedia.org/wiki/${enc(page)}${section === undefined ? "" : `#${enc(section)}`}`;
}

export type FormatIdentity = {
  /** What to call the format, or null when nothing named it. */
  readonly name: string | null;
  readonly wikipedia: string | null;
  readonly about: About | null;
  /** How the file was identified, as the title line says it. */
  readonly how: string;
  /** What the file(1) rules said, when they said anything. */
  readonly rule: Identification | null;
};

/** Where the template reading the file came from. `other` is a template built
 *  from the file(1) rule that identified the file, or one converted from a
 *  description the reader pasted. */
export function templateSource(doc: Doc, template: string): "builtin" | "kaitai" | "hexpat" | "other" {
  const choice = doc.templateChoices.find((c) => c.name === template);
  return choice?.source ?? "other";
}

/** True when the template describes the file's signature and nothing else:
 *  the one built from a file(1) rule. Bytes it leaves undescribed are not a
 *  finding, since it never claimed to describe them. */
export function signatureOnly(doc: Doc): boolean {
  const t = doc.template;
  // A description the reader converted is not one of the choices either, and
  // says so by having a conversion report.
  return t !== null && templateSource(doc, t) === "other" && doc.ksyReport() === null && doc.hexpatReport() === null;
}

/**
 * Everything the title line needs, once the file(1) rules and the signature
 * list have answered. Both are asked once per report and kept; `WAIT` until
 * they have.
 */
export function formatIdentity(doc: Doc, data: ReportData): FormatIdentity | typeof WAIT {
  const rule = data.later("identify", () => doc.identify());
  const sigs = data.later("signatures", () => doc.signatureMatches());
  if (rule === WAIT || sigs === WAIT) return WAIT;
  const template = doc.template;
  const about = template === null ? null : formatAbout(doc, template);
  // The signature list names a file only when nothing better has: for a JPEG
  // read by its template it can put "DualPhoto JPEG bitmap" first, which is
  // a true match and the wrong article.
  const named = sigs === null || (template !== null && !signatureOnly(doc)) ? null : namingMatch(sigs.matches);
  const wp = about?.wikipedia ?? named?.format.wp ?? null;
  let how: string = RV.notIdentified;
  let name: string | null = about !== null && about.name !== "" ? about.name : null;
  if (template !== null) {
    const source = templateSource(doc, template);
    const label = templateLabel(template);
    if (source === "builtin") how = RV.identifiedByTemplate(label);
    else if (source === "kaitai") how = RV.identifiedByKaitai(label);
    else if (source === "hexpat") how = RV.identifiedByImhex(label);
    else if (rule !== null) how = RV.identifiedByRule;
    else how = RV.identifiedByTemplate(label);
    name ??= source === "builtin" || source === "kaitai" || source === "hexpat" ? label : null;
  } else if (rule !== null) {
    how = RV.identifiedByRule;
  } else if (named !== null) {
    how = RV.identifiedBySignature;
  }
  name ??= named?.format.label ?? (rule !== null ? ruleName(rule) : null);
  return { name, wikipedia: wp, about, how, rule };
}

/** The format as a file(1) sentence names it: the words before its first
 *  comma, `TrueType Font data` out of `TrueType Font data, 10 tables`. */
function ruleName(id: Identification): string {
  return id.message.split(",")[0]?.trim() ?? id.message;
}
