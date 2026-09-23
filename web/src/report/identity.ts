// What the file is, for the title line and the sentences under it: the
// format's name, its Wikipedia article, the sentences the core keeps about it,
// and what identified the file.

import type { Doc, Identification } from "../doc.ts";
import { templateLabel } from "../filetype.ts";
import * as wasm from "../pkg/qubero_wasm.js";
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
 * The core's sentences for a template, or null.
 *
 * Asked through a look-up that admits the binding may not be there: it was
 * added after this view, and a `src/pkg` built before it answers everything
 * else the report asks. Without it the report says what file(1) says instead,
 * which is the rule for a format the core has no sentences for.
 *
 * The binding is taken to answer either the entry as JSON, `{name, wikipedia,
 * text}`, or the sentences alone, and "" or `null` for a template with none.
 */
export function formatAbout(template: string): About | null {
  const fn = (wasm as unknown as Record<string, unknown>)["format_about"];
  if (typeof fn !== "function") return null;
  let raw: unknown;
  try {
    raw = (fn as (t: string) => unknown)(template);
  } catch {
    return null;
  }
  if (raw === undefined || raw === null || raw === "" || raw === "null") return null;
  if (typeof raw === "string") {
    const text = raw;
    try {
      raw = JSON.parse(text) as unknown;
    } catch {
      return { name: "", wikipedia: null, text };
    }
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

/** Where the template reading the file came from. */
function templateSource(doc: Doc, template: string): "builtin" | "kaitai" | "hexpat" | "other" {
  const choice = doc.templateChoices.find((c) => c.name === template);
  return choice?.source ?? "other";
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
  const about = template === null ? null : formatAbout(template);
  const named = sigs === null ? null : namingMatch(sigs.matches);
  const wp = about?.wikipedia ?? named?.format.wp ?? named?.format.parent?.wp ?? null;
  let how: string = RV.notIdentified;
  let name: string | null = about !== null && about.name !== "" ? about.name : null;
  if (template !== null) {
    const source = templateSource(doc, template);
    const label = templateLabel(template);
    if (source === "builtin") how = RV.identifiedByTemplate(label);
    else if (source === "kaitai") how = RV.identifiedByKaitai(label);
    else if (source === "hexpat") how = RV.identifiedByImhex(label);
    else if (rule !== null && rule.source !== "") how = RV.identifiedByRule(rule.source);
    else how = RV.identifiedByTemplate(label);
    name ??= source === "builtin" || source === "kaitai" || source === "hexpat" ? label : null;
  } else if (rule !== null) {
    how = rule.source !== "" ? RV.identifiedByRule(rule.source) : RV.identifiedBySignature;
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
