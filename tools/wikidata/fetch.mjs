// Downloads what Wikidata knows about identifying file formats by their bytes,
// and the extensions English Wikipedia's infoboxes give for the same formats,
// into target/wikidata. Then rebuilds tools/wikidata/formats.json from them,
// and web/public/signatures.json, which the page reads, from that.
//
//   node tools/wikidata/fetch.mjs
//
// There is no commit to pin, as there is for the Detect It Easy rules: the
// data is whatever Wikidata says today. So the downloads are kept, the build
// can be rerun from them without asking again (`node tools/wikidata/build.mjs`),
// and the file it writes is committed, with the date it was fetched inside.
//
// Each question is its own small query rather than one query that joins them
// all. Joined, every optional fact multiplies the rows of every other, and
// the query service stops answering after 60 seconds.

import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const CACHE = join(ROOT, "target", "wikidata");

// Wikimedia turns away requests without a User-Agent that says who is asking.
const USER_AGENT = "Qubero/0.1 (https://github.com/pengowray/qubero; file format patterns)";
const SPARQL = "https://query.wikidata.org/sparql";
const WIKIPEDIA_API = "https://en.wikipedia.org/w/api.php";

/** One query per fact, each keyed to the items that have a pattern at all. */
const QUERIES = {
  // One row per statement. Grouping by the statement rather than by item and
  // value keeps two statements with the same value apart, with their own
  // offsets. Deprecated statements come too, and the build drops them, so
  // the report can say how many there were.
  statements: `
    SELECT ?item ?st ?value ?rank
      (GROUP_CONCAT(DISTINCT STR(?enc); separator=" ") AS ?encodings)
      (GROUP_CONCAT(DISTINCT STR(?syntax); separator=" ") AS ?syntaxes)
      (GROUP_CONCAT(DISTINCT STR(?rel); separator=" ") AS ?relative)
      (GROUP_CONCAT(DISTINCT STR(?off); separator=" ") AS ?offsets)
    WHERE {
      ?item p:P4152 ?st .
      ?st ps:P4152 ?value ; wikibase:rank ?rank .
      OPTIONAL { ?st pq:P3294 ?enc }
      OPTIONAL { ?st pq:P4240 ?syntax }
      OPTIONAL { ?st pq:P2210 ?rel }
      OPTIONAL { ?st pq:P4153 ?off }
    }
    GROUP BY ?item ?st ?value ?rank`,
  // "mul" is the label for every language at once, which some items have
  // instead of an English one.
  labels: `
    SELECT DISTINCT ?item ?label WHERE {
      ?item p:P4152 [] ; rdfs:label ?label .
      FILTER(LANG(?label) = "en" || LANG(?label) = "mul")
    }`,
  extensions: `
    SELECT DISTINCT ?item ?ext WHERE {
      ?item p:P4152 [] ; p:P1195 ?s .
      ?s ps:P1195 ?ext ; wikibase:rank ?rank .
      FILTER(?rank != wikibase:DeprecatedRank)
    }`,
  mediaTypes: `
    SELECT DISTINCT ?item ?mime WHERE {
      ?item p:P4152 [] ; p:P1163 ?s .
      ?s ps:P1163 ?mime ; wikibase:rank ?rank .
      FILTER(?rank != wikibase:DeprecatedRank)
    }`,
  enwiki: `
    SELECT DISTINCT ?item ?title WHERE {
      ?item p:P4152 [] .
      ?article schema:about ?item ; schema:isPartOf <https://en.wikipedia.org/> ; schema:name ?title .
    }`,
  // Most of the items are one version of a format, or one variant, and have
  // no article; the format they are a version of often does. Only parents
  // with an article are worth knowing about here.
  parents: `
    SELECT DISTINCT ?item ?relation ?parent ?parentLabel ?title WHERE {
      VALUES ?relation { wdt:P279 wdt:P361 }
      ?item p:P4152 [] ; ?relation ?parent .
      ?article schema:about ?parent ; schema:isPartOf <https://en.wikipedia.org/> ; schema:name ?title .
      OPTIONAL { ?parent rdfs:label ?parentLabel . FILTER(LANG(?parentLabel) = "en") }
    }`,
};

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/**
 * A request that waits and tries again when the server is busy or briefly
 * down, the way Wikimedia asks: after Retry-After when it says, doubling up to
 * half a minute when it does not. Anything else is an error to report.
 */
async function politeFetch(url, init) {
  let wait = 2000;
  for (let attempt = 1; ; attempt++) {
    let res;
    try {
      res = await fetch(url, { ...init, headers: { "User-Agent": USER_AGENT, ...init?.headers } });
    } catch (e) {
      if (attempt >= 5) throw e;
      res = null;
    }
    if (res !== null && res.ok) return res;
    const retryable = res === null || [408, 429, 500, 502, 503, 504].includes(res.status);
    if (!retryable || attempt >= 5) {
      const body = res === null ? "" : (await res.text()).slice(0, 500);
      throw new Error(`${res?.status ?? "no response"} for ${url.slice(0, 120)}\n${body}`);
    }
    const after = Number(res?.headers.get("retry-after"));
    const ms = Number.isFinite(after) && after > 0 ? after * 1000 : wait;
    console.log(`  busy (${res?.status ?? "no response"}), trying again in ${Math.round(ms / 1000)}s`);
    await sleep(ms);
    wait = Math.min(wait * 2, 30000);
  }
}

async function sparql(query) {
  const res = await politeFetch(SPARQL, {
    method: "POST",
    headers: { Accept: "application/sparql-results+json", "Content-Type": "application/x-www-form-urlencoded" },
    body: new URLSearchParams({ query }),
  });
  const json = await res.json();
  // Flattened to plain values, keeping a literal's language as `name@`.
  return json.results.bindings.map((b) =>
    Object.fromEntries(Object.entries(b).flatMap(([k, v]) => (v["xml:lang"] === undefined ? [[k, v.value]] : [[k, v.value], [`${k}@`, v["xml:lang"]]]))),
  );
}

/** The current wikitext of each English Wikipedia article, by title as asked. */
async function wikitexts(titles) {
  const out = {};
  for (let i = 0; i < titles.length; i += 50) {
    const batch = titles.slice(i, i + 50);
    const url = `${WIKIPEDIA_API}?${new URLSearchParams({
      action: "query",
      prop: "revisions",
      rvprop: "content",
      rvslots: "main",
      redirects: "1",
      format: "json",
      formatversion: "2",
      titles: batch.join("|"),
    })}`;
    const json = await (await politeFetch(url)).json();
    // A title that redirects comes back under the page it lands on, so map
    // the landing title back to the one that was asked for.
    const asked = new Map(batch.map((t) => [t, t]));
    for (const n of json.query.normalized ?? []) asked.set(n.to, asked.get(n.from) ?? n.from);
    for (const r of json.query.redirects ?? []) asked.set(r.to, asked.get(r.from) ?? r.from);
    for (const page of json.query.pages) {
      const text = page.revisions?.[0]?.slots?.main?.content;
      if (text !== undefined) out[asked.get(page.title) ?? page.title] = text;
    }
    await sleep(500);
  }
  return out;
}

async function main() {
  mkdirSync(CACHE, { recursive: true });
  const results = {};
  for (const [name, query] of Object.entries(QUERIES)) {
    const rows = await sparql(query);
    results[name] = rows;
    writeFileSync(join(CACHE, `${name}.json`), JSON.stringify(rows));
    console.log(`${name}: ${rows.length} rows`);
    await sleep(1000);
  }
  const titles = [...new Set([...results.enwiki, ...results.parents].map((r) => r.title))].sort();
  const pages = await wikitexts(titles);
  writeFileSync(join(CACHE, "wikipedia.json"), JSON.stringify(pages));
  console.log(`wikipedia: ${Object.keys(pages).length} of ${titles.length} articles`);
  writeFileSync(join(CACHE, "fetched.json"), JSON.stringify({ fetched: new Date().toISOString().slice(0, 10) }));
  await import("./build.mjs");
  await import("../signatures.mjs");
}

await main();
