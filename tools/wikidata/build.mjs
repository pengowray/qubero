// Builds web/public/wikidata/formats.json, the file format patterns the page
// matches a file against, from what fetch.mjs downloaded into target/wikidata.
// Also writes tools/wikidata/cleanup.md, which lists every pattern that had to
// be read some other way than as written, and every one that was left out.
//
//   node tools/wikidata/build.mjs
//
// The report is the place to look before fixing anything on Wikidata: each
// entry links to the item, and says what was wrong with its value.

import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { cleanPattern } from "./patterns.mjs";
import { infoboxExtensions } from "./infobox.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = join(HERE, "..", "..");
const CACHE = join(ROOT, "target", "wikidata");
const OUT = join(ROOT, "web", "public", "wikidata", "formats.json");
const REPORT = join(HERE, "cleanup.md");

const ENTITY = "http://www.wikidata.org/entity/";
const BEGINNING_OF_FILE = "Q35436009";
const END_OF_FILE = "Q1148480";
const DEPRECATED = "http://wikiba.se/ontology#DeprecatedRank";

const load = (name) => JSON.parse(readFileSync(join(CACHE, `${name}.json`), "utf8"));
const qid = (uri) => uri.replace(ENTITY, "");
const words = (s) => (s === undefined || s === "" ? [] : s.split(" "));

const { fetched } = load("fetched");
const statements = load("statements");
const wikipedia = load("wikipedia");

/** Everything known about one item, built up fact by fact. */
const items = new Map();
const item = (uri) => {
  const id = qid(uri);
  let it = items.get(id);
  if (it === undefined) {
    it = { id, label: null, mul: null, desc: null, ext: new Set(), mime: new Set(), wp: null, parents: [], sigs: new Map() };
    items.set(id, it);
  }
  return it;
};

// What happened to each value that was not used exactly as written.
const skipped = new Map();
const noted = new Map();
const file = (map, reason, entry) => {
  if (!map.has(reason)) map.set(reason, []);
  map.get(reason).push(entry);
};

let deprecated = 0;
for (const s of statements) {
  const it = item(s.item);
  if (s.rank === DEPRECATED) {
    deprecated++;
    continue;
  }
  const encodings = [...new Set([...words(s.encodings), ...words(s.syntaxes)].map(qid))];
  const entry = { id: it.id, value: s.value, encodings };
  const relative = words(s.relative).map(qid);
  if (relative.some((r) => r !== BEGINNING_OF_FILE && r !== END_OF_FILE) || relative.length > 1) {
    file(skipped, "measured from something other than the start or end of the file", entry);
    continue;
  }
  const fromEnd = relative[0] === END_OF_FILE;
  const offsets = words(s.offsets);
  if (offsets.some((o) => o.includes("/.well-known/genid/"))) {
    file(skipped, "offset given as unknown", entry);
    continue;
  }
  const numbers = offsets.map(Number);
  if (numbers.some((n) => !Number.isInteger(n) || n < 0)) {
    file(skipped, "offset is not a whole number of bytes", entry);
    continue;
  }
  const result = cleanPattern(s.value, encodings);
  if ("skip" in result) {
    file(skipped, result.skip, entry);
    continue;
  }
  entry.pattern = result.pattern;
  if (result.note !== null) file(noted, result.note, entry);
  if (numbers.length === 0) {
    file(noted, "no offset given, so 0", entry);
    numbers.push(0);
  }
  for (const n of numbers) {
    const sig = fromEnd ? [result.pattern, n, "eof"] : [result.pattern, n];
    it.sigs.set(sig.join(" "), sig);
  }
}

for (const r of load("labels")) {
  const it = items.get(qid(r.item));
  if (it === undefined) continue;
  if (r["label@"] === "en") it.label = r.label;
  else it.mul = r.label;
}
for (const r of load("descriptions")) {
  const it = items.get(qid(r.item));
  if (it !== undefined) it.desc = r.description;
}
const extension = (e) => e.trim().replace(/^\*?\./, "").toLowerCase();
for (const r of load("extensions")) {
  const it = items.get(qid(r.item));
  const e = extension(r.ext);
  if (it !== undefined && e !== "") it.ext.add(e);
}
for (const r of load("mediaTypes")) {
  const it = items.get(qid(r.item));
  const mime = r.mime.trim().toLowerCase();
  // Says only that the format is bytes, which every format is.
  if (it !== undefined && mime !== "application/octet-stream") it.mime.add(mime);
}
for (const r of load("enwiki")) {
  const it = items.get(qid(r.item));
  if (it !== undefined) it.wp = r.title;
}
for (const r of load("parents")) {
  const it = items.get(qid(r.item));
  if (it !== undefined) {
    const subclass = r.relation.endsWith("P279");
    it.parents.push({ id: qid(r.parent), label: r.parentLabel ?? qid(r.parent), wp: r.title, subclass });
  }
}

let infoboxes = 0;
let fromWikipedia = 0;
const formats = [];
for (const it of [...items.values()].sort((a, b) => Number(a.id.slice(1)) - Number(b.id.slice(1)))) {
  if (it.sigs.size === 0) continue;
  const f = { id: it.id, label: it.label ?? it.mul ?? it.id };
  if (it.desc !== null) f.desc = it.desc;
  if (it.ext.size > 0) f.ext = [...it.ext].sort();
  const text = it.wp === null ? undefined : wikipedia[it.wp];
  if (text !== undefined) {
    const found = infoboxExtensions(text);
    if (found !== null) infoboxes++;
    const extra = (found ?? []).filter((e) => !it.ext.has(e));
    if (extra.length > 0) {
      f.wpExt = extra;
      fromWikipedia += extra.length;
    }
  }
  if (it.mime.size > 0) f.mime = [...it.mime].sort();
  if (it.wp !== null) f.wp = it.wp;
  else if (it.parents.length > 0) {
    // One is enough to follow. A subclass is closer kin than a part.
    const p = [...it.parents].sort((a, b) => Number(b.subclass) - Number(a.subclass) || a.label.localeCompare(b.label))[0];
    f.parent = { id: p.id, label: p.label, wp: p.wp };
  }
  f.sigs = [...it.sigs.values()];
  formats.push(f);
}

mkdirSync(dirname(OUT), { recursive: true });
const json = JSON.stringify({
  source: "Wikidata property P4152 (file format identification pattern), CC0",
  fetched,
  formats,
});
// One format to a line, so a refresh reads as a diff of the formats that changed.
writeFileSync(OUT, json.replace(/,"formats":\[/, ',"formats":[\n').replace(/\},\{"id"/g, '},\n{"id"').replace(/\]\}$/, "\n]}\n"));

// The report.
const labelOf = (id) => items.get(id)?.label ?? id;
const code = (s) => {
  const ticks = "`".repeat(Math.max(0, ...(s.match(/`+/g) ?? []).map((t) => t.length)) + 1);
  return `${ticks}${ticks.length > 1 ? " " : ""}${s}${ticks.length > 1 ? " " : ""}${ticks}`;
};
const lines = [
  "# Wikidata file format patterns: cleanup report",
  "",
  "Generated by `node tools/wikidata/build.mjs`. Do not edit by hand.",
  "",
  `Fetched ${fetched}. ${statements.length} statements of [P4152](https://www.wikidata.org/wiki/Property:P4152) on ${items.size} items.`,
  `${formats.length} formats kept, with ${formats.reduce((n, f) => n + f.sigs.length, 0)} patterns.`,
  `${deprecated} deprecated statements dropped.`,
  `${infoboxes} Wikipedia infoboxes listed extensions; ${fromWikipedia} of those were not already on Wikidata.`,
  "",
];
const section = (title, map, withPattern) => {
  lines.push(`## ${title}`, "");
  for (const [reason, entries] of [...map].sort((a, b) => b[1].length - a[1].length)) {
    lines.push(`### ${reason} (${entries.length})`, "");
    for (const e of entries) {
      const enc = e.encodings.length > 0 ? ` (${e.encodings.join(", ")})` : "";
      const to = withPattern && e.pattern !== e.value ? ` read as ${code(e.pattern)}` : "";
      lines.push(`- [${e.id}](https://www.wikidata.org/wiki/${e.id}) ${labelOf(e.id)}: ${code(e.value)}${enc}${to}`);
    }
    lines.push("");
  }
};
section("Left out", skipped, false);
section("Read some other way than as written", noted, true);
writeFileSync(REPORT, lines.join("\n"));

const skippedCount = [...skipped.values()].reduce((n, e) => n + e.length, 0);
console.log(`formats.json: ${formats.length} formats, ${(json.length / 1024).toFixed(0)} KiB; ${skippedCount} patterns left out, see tools/wikidata/cleanup.md`);
