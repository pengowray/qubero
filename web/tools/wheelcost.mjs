// Measures what one wheel notch costs the hex view: how many draws it sets
// off, how long they take, and how long the browser then spends laying the
// page out again.
//
//   node web/tools/wheelcost.mjs --url http://localhost:2416/?url=/samples/notes.sqlite
//
// Playwright comes from the global install; this package does not depend on it.
import { createRequire } from "node:module";
import { pathToFileURL } from "node:url";
import path from "node:path";

const GLOBAL_NPM = process.env.NPM_GLOBAL_ROOT || "C:/Users/pengo/AppData/Roaming/npm/node_modules";

async function loadChromium() {
  const tries = ["playwright", path.join(GLOBAL_NPM, "playwright", "index.mjs"), path.join(GLOBAL_NPM, "playwright-core", "index.mjs")];
  for (const t of tries) {
    try {
      const req = createRequire(pathToFileURL(path.join(GLOBAL_NPM, "x.js")).href);
      const resolved = t === "playwright" ? req.resolve("playwright") : t;
      const mod = await import(pathToFileURL(resolved).href);
      if (mod.chromium) return mod.chromium;
    } catch { /* try the next one */ }
  }
  throw new Error(`Could not load Playwright. Looked in ${GLOBAL_NPM}. Set NPM_GLOBAL_ROOT or npm i -g playwright.`);
}

function parseArgs(argv) {
  const a = { url: "http://localhost:2416/?url=/samples/notes.sqlite", notches: 6, delta: 100, width: 1280, height: 800, wait: 400, gap: 24 };
  for (let i = 0; i < argv.length; i++) {
    const k = argv[i];
    const v = argv[i + 1];
    if (k === "--url") { a.url = v; i++; }
    else if (k === "--notches") { a.notches = Number(v); i++; }
    else if (k === "--delta") { a.delta = Number(v); i++; }
    else if (k === "--width") { a.width = Number(v); i++; }
    else if (k === "--height") { a.height = Number(v); i++; }
    else if (k === "--wait") { a.wait = Number(v); i++; }
    else if (k === "--gap") { a.gap = Number(v); i++; }
  }
  return a;
}

/** Count every draw and time it, and time the layout the browser does after. */
const INSTRUMENT = () => {
  const v = window.__qubero.view;
  const proto = Object.getPrototypeOf(v);
  const state = { draws: 0, drawMs: 0, deepest: 0, depth: 0, top: 0, longest: 0, frames: 0 };
  window.__wc = state;
  const render = proto.render;
  proto.render = function patched() {
    const outer = state.depth === 0;
    if (outer) state.top++;
    state.depth++;
    if (state.depth > state.deepest) state.deepest = state.depth;
    state.draws++;
    const t = performance.now();
    try {
      return render.call(this);
    } finally {
      state.depth--;
      const took = performance.now() - t;
      if (outer) {
        state.drawMs += took;
        if (took > state.longest) state.longest = took;
      }
    }
  };
  // Where a draw's time goes, by the step that spent it. A step called from
  // another is counted in its own right as well as in the whole.
  state.parts = {};
  for (const name of ["frame", "placeSpans", "planValues", "drawHeader", "fitParts", "drawRow", "drawCells", "drawNotes", "drawPinned", "measure", "settleHeights", "finish", "fitRows"]) {
    const fn = proto[name];
    if (typeof fn !== "function") continue;
    state.parts[name] = { n: 0, ms: 0 };
    proto[name] = function timed(...args) {
      const t = performance.now();
      try {
        return fn.apply(this, args);
      } finally {
        const p = state.parts[name];
        p.n++;
        p.ms += performance.now() - t;
      }
    };
  }
  // How evenly the browser got to paint: a frame it could not finish inside
  // its budget is a step the reader sees as a stall.
  state.gaps = [];
  let last = -1;
  const tick = (now) => {
    state.frames++;
    if (last >= 0) state.gaps.push(now - last);
    last = now;
    requestAnimationFrame(tick);
  };
  requestAnimationFrame(tick);
  state.reset = () => {
    state.draws = 0; state.drawMs = 0; state.deepest = 0; state.top = 0; state.longest = 0; state.frames = 0;
    for (const p of Object.values(state.parts)) { p.n = 0; p.ms = 0; }
    state.gaps.length = 0;
  };
};

const main = async () => {
  const a = parseArgs(process.argv.slice(2));
  const chromium = await loadChromium();
  const browser = await chromium.launch();
  const page = await browser.newPage({ viewport: { width: a.width, height: a.height } });
  await page.goto(a.url, { waitUntil: "load" });
  await page.waitForFunction(() => window.__qubero?.view !== undefined, null, { timeout: 15000 });
  await page.waitForTimeout(a.wait);
  await page.evaluate(() => {
    const s = document.querySelector(".tb-width");
    if (s) { s.value = "16"; s.dispatchEvent(new Event("change", { bubbles: true })); }
  });
  await page.waitForTimeout(a.wait);
  await page.evaluate(INSTRUMENT);

  const box = await page.locator(".hv-rows").first().boundingBox();
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);

  const run = async (label, dir) => {
    await page.evaluate(() => window.__wc.reset());
    const t0 = Date.now();
    for (let i = 0; i < a.notches; i++) {
      await page.mouse.wheel(0, dir * a.delta);
      if (a.gap > 0) await page.waitForTimeout(a.gap);
    }
    await page.waitForTimeout(a.wait);
    const s = await page.evaluate(() => ({ ...window.__wc, reset: undefined, parts: JSON.parse(JSON.stringify(window.__wc.parts)) }));
    const wall = Date.now() - t0;
    console.log(
      `${label.padEnd(6)} notches ${a.notches}  wheel-driven draws ${String(s.top).padStart(3)}` +
        `  total draws ${String(s.draws).padStart(3)}  nested deepest ${s.deepest}` +
        `  draw ms ${s.drawMs.toFixed(1).padStart(7)}  longest ${s.longest.toFixed(1).padStart(6)}` +
        `  wall ${wall}ms`,
    );
    const gaps = s.gaps.slice().sort((x, y) => x - y);
    const at = (q) => (gaps.length === 0 ? 0 : gaps[Math.min(gaps.length - 1, Math.floor(q * gaps.length))]);
    console.log(
      `       frames ${gaps.length}  frame ms median ${at(0.5).toFixed(1)}  p90 ${at(0.9).toFixed(1)}` +
        `  worst ${(gaps[gaps.length - 1] ?? 0).toFixed(1)}  over 32ms ${gaps.filter((g) => g > 32).length}`,
    );
    const parts = Object.entries(s.parts)
      .filter(([, p]) => p.ms >= 1)
      .sort((x, y) => y[1].ms - x[1].ms)
      .map(([k, p]) => `${k} ${p.ms.toFixed(0)}ms/${p.n}`)
      .join("  ");
    if (parts !== "") console.log(`       ${parts}`);
    return s;
  };

  console.log(a.url);
  await run("down", 1);
  await run("up", -1);
  await browser.close();
};

main().catch((e) => { console.error(e); process.exit(1); });
