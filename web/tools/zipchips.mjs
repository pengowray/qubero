// Times the annotation column over a deflated zip entry: how long a spans
// query takes cold and warm, how long one wheel step takes, and how many chips
// a row ends up with.
//
//   node web/tools/zipchips.mjs --url "http://localhost:17296/?url=/local/big.zip"
import { createRequire } from "node:module";
import { pathToFileURL } from "node:url";
import path from "node:path";

const GLOBAL_NPM = process.env.NPM_GLOBAL_ROOT || "C:/Users/pengo/AppData/Roaming/npm/node_modules";

async function loadChromium() {
  const req = createRequire(pathToFileURL(path.join(GLOBAL_NPM, "x.js")).href);
  const tries = [req.resolve("playwright"), path.join(GLOBAL_NPM, "playwright", "index.mjs")];
  for (const t of tries) {
    const mod = await import(pathToFileURL(t).href);
    if (mod.chromium) return mod.chromium;
  }
  throw new Error("no chromium");
}

const args = process.argv.slice(2);
const arg = (k, d) => { const i = args.indexOf(k); return i < 0 ? d : args[i + 1]; };
const url = arg("--url", "http://localhost:17296/?url=/local/big.zip");
const shot = arg("--shot", null);

const chromium = await loadChromium();
const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
page.on("console", (m) => { if (m.type() === "error") console.log("[page error]", m.text()); });
await page.goto(url, { waitUntil: "load" });
await page.waitForFunction(() => window.__qubero?.doc !== undefined, null, { timeout: 120000 });
await page.waitForTimeout(3000);

// Find a deflated entry's data: the first local file header's payload, a few
// megabytes in.
await page.evaluate(() => {
  window.__spans = async (fromBit, toBit, max) => {
    for (let i = 0; i < 400; i++) {
      const r = await window.__qubero.doc.spans(fromBit, toBit, max);
      if (r.status === "ok") return r.node;
      if (r.status === "error") throw new Error(r.message);
      await new Promise((res) => setTimeout(res, 25));
    }
    throw new Error("spans never settled");
  };
});
const found = await page.evaluate(async () => {
  const spans = await window.__spans(0, 64 * 8, 20);
  return spans.map((s) => ({ name: s.name, trail: s.trail, off: s.offset_bits, size: s.size_bits }));
});
console.log("top spans:", JSON.stringify(found.slice(0, 6)));

// Jump a few MB into the file, which is inside the first entry's deflate run.
const AT = 4 * 1024 * 1024;
const q = async (at, bytes = 4096) => page.evaluate(async ([at, bytes]) => {
  const t = performance.now();
  const spans = await window.__spans(at * 8, (at + bytes) * 8, 400);
  const ms = performance.now() - t;
  return { ms, n: spans.length, first: spans.slice(0, 6).map((s) => `${s.gap ? "GAP " : ""}${s.name} [${s.size_bits}b] ${s.count}`) };
}, [at, bytes]);

const cold = await q(AT);
const warm = await q(AT);
const other = await q(AT + 1024 * 1024);
console.log("cold spans:", JSON.stringify(cold));
console.log("warm spans:", JSON.stringify(warm));
console.log("1MB further (cold-ish):", JSON.stringify(other));

// Scroll the view there and measure a wheel step.
await page.evaluate((at) => {
  window.__qubero.view.setCursor(at, { select: undefined });
}, AT);
await page.waitForTimeout(2500);
console.log("top of view after the jump:", await page.evaluate(() => document.querySelector(".hv-rows [data-off]")?.getAttribute("data-off")));
await page.mouse.move(400, 500);

const wheel = [];
for (let i = 0; i < 5; i++) {
  const t = await page.evaluate(async () => {
    const rows = document.querySelector(".hv-rows");
    const first = () => rows.querySelector("[data-off]")?.getAttribute("data-off");
    const before = first();
    const t0 = performance.now();
    window.__wheelStart = t0;
    window.__wheelBefore = before;
    return t0;
  });
  await page.mouse.wheel(0, 100);
  const ms = await page.evaluate(async () => {
    const rows = document.querySelector(".hv-rows");
    const first = () => rows.querySelector("[data-off]")?.getAttribute("data-off");
    const deadline = performance.now() + 5000;
    while (performance.now() < deadline) {
      if (first() !== window.__wheelBefore) return performance.now() - window.__wheelStart;
      await new Promise((r) => requestAnimationFrame(r));
    }
    return -1;
  });
  wheel.push(Math.round(ms));
  void t;
}
console.log("wheel step ms (bytes moved):", JSON.stringify(wheel));

// A cold window: an entry nothing has opened yet, where the spans query is
// the slow one. The bytes must be on screen without waiting for it.
const COLD = Number(arg("--cold", String(10 * 1024 * 1024)));
await page.evaluate((at) => window.__qubero.view.setCursor(at, { select: undefined }), COLD - 4096);
await page.waitForTimeout(2000);
await page.mouse.move(400, 500);
const bytesFirst = await (async () => {
  await page.evaluate(() => {
    const rows = document.querySelector(".hexview .hv-rows") ?? document.querySelector(".hv-rows");
    window.__before = rows.querySelector("[data-off]")?.getAttribute("data-off");
    window.__t0 = performance.now();
  });
  await page.mouse.wheel(0, 100);
  return page.evaluate(async () => {
    const rows = document.querySelector(".hexview .hv-rows") ?? document.querySelector(".hv-rows");
    const at = () => rows.querySelector("[data-off]")?.getAttribute("data-off");
    let moved = -1;
    let chipped = -1;
    const deadline = performance.now() + 8000;
    while (performance.now() < deadline && (moved < 0 || chipped < 0)) {
      if (moved < 0 && at() !== window.__before) moved = performance.now() - window.__t0;
      if (moved >= 0 && chipped < 0 && rows.querySelectorAll(".hv-chip").length > 0) {
        chipped = performance.now() - window.__t0;
      }
      await new Promise((r) => requestAnimationFrame(r));
    }
    return { bytesMs: Math.round(moved), chipsMs: Math.round(chipped) };
  });
})();
console.log("cold window: bytes drawn / chips drawn (ms after the wheel):", JSON.stringify(bytesFirst));

const chips = await page.evaluate(() => {
  // Only the rows the reader can see: the view keeps rows around off screen,
  // and their chips are whatever the window they were last drawn for said.
  const box = document.querySelector(".hexview .hv-rows") ?? document.querySelector(".hv-rows");
  const view = box.getBoundingClientRect();
  const top = Number(box.querySelector(".hv-row [data-off]")?.getAttribute("data-off") ?? 0);
  const rows = [...box.querySelectorAll(".hv-row")].filter((r) => {
    const b = r.getBoundingClientRect();
    const off = Number(r.querySelector("[data-off]")?.getAttribute("data-off") ?? -1);
    return b.bottom > view.top && b.top < view.bottom && off >= top && off < top + 4096;
  });
  const counts = rows.map((r) => [...r.querySelectorAll(".hv-chip")].filter((c) => c.offsetParent !== null).length);
  const offsets = rows.map((r) => r.querySelector("[data-off]")?.getAttribute("data-off"));
  const sample = rows
    .slice(0, 4)
    .map((r) => [...r.querySelectorAll(".hv-chip")].filter((c) => c.offsetParent !== null).map((c) => c.textContent.trim()));
  return { rows: rows.length, at: offsets[0], counts, sample };
});
console.log("chips per row at", chips.at, ":", JSON.stringify(chips.counts));
console.log("sample chips:", JSON.stringify(chips.sample, null, 1));

if (shot) await page.screenshot({ path: shot, fullPage: false });
await browser.close();
