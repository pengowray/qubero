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
  const a = { url: "http://localhost:2416/?url=/samples/notes.sqlite", notches: 6, delta: 100, width: 1280, height: 800, wait: 400, gap: 24, css: "", links: false, spin: 0, every: 8 };
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
    // A sustained spin: how long to keep reporting for, and how often. This is
    // the case the reader complains about and the one `page.mouse.wheel`
    // cannot make, since CDP paces it slower than a frame.
    else if (k === "--spin") { a.spin = Number(v); i++; }
    else if (k === "--every") { a.every = Number(v); i++; }
    else if (k === "--css") { a.css = v; i++; }
    // With the dependency arrows switched on and a field picked, so the cost
    // of the overlay is measured on the same footing as everything else.
    else if (k === "--links") { a.links = true; }
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
  // The drawing moved out of the view and into `HexRows`, so both prototypes
  // are timed: a step missing from this list is time that shows up in the
  // whole draw and nowhere else.
  const time = (owner, names, prefix = "") => {
    for (const name of names) {
      const fn = owner[name];
      if (typeof fn !== "function") continue;
      const key = prefix + name;
      state.parts[key] = { n: 0, ms: 0 };
      owner[name] = function timed(...args) {
        const t = performance.now();
        try {
          return fn.apply(this, args);
        } finally {
          const p = state.parts[key];
          p.n++;
          p.ms += performance.now() - t;
        }
      };
    }
  };
  time(proto, ["frame", "placeSpans", "planValues", "measure", "settleHeights", "finish", "fitRows", "markHover", "relayout"]);
  const rows = v.grid ?? v.rows;
  if (rows !== undefined) {
    time(Object.getPrototypeOf(rows), ["write", "heights", "drawHeader", "drawRow", "drawCells", "drawNotes", "drawPinned", "layOutRow", "fitParts", "ensure", "noteMetrics", "hexPitch"], "rows.");
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

/**
 * A sustained spin, reported from inside the page.
 *
 * The reader's complaint is about keeping the wheel turning, which is a report
 * every frame or two for seconds on end. CDP's own `page.mouse.wheel` is paced
 * slower than that, so a run of those measures a view that is idle between
 * notches and says nothing about one that never catches up. This dispatches
 * the reports itself and asks the only question that matters while it does:
 * how many frames the browser missed.
 */
const spin = async (page, a, metrics) => {
  await page.evaluate(() => window.__wc.reset());
  const before = await metrics();
  const out = await page.evaluate(
    ([ms, every]) =>
      new Promise((done) => {
        const v = window.__qubero.view;
        const stop = performance.now() + ms;
        let sent = 0;
        const tick = () => {
          if (performance.now() >= stop) {
            // Two frames of quiet, so the last draw lands inside the run.
            requestAnimationFrame(() => requestAnimationFrame(() => done({ sent })));
            return;
          }
          v.el.dispatchEvent(new WheelEvent("wheel", { deltaY: 40, cancelable: true, bubbles: true }));
          sent++;
          setTimeout(tick, every);
        };
        tick();
      }),
    [a.spin, a.every],
  );
  const s = await page.evaluate(() => ({ ...window.__wc, reset: undefined, parts: JSON.parse(JSON.stringify(window.__wc.parts)) }));
  const after = await metrics();
  const spent = (k) => ((after[k] ?? 0) - (before[k] ?? 0)) * 1000;
  const gaps = s.gaps.slice().sort((x, y) => x - y);
  const at = (q) => (gaps.length === 0 ? 0 : gaps[Math.min(gaps.length - 1, Math.floor(q * gaps.length))]);
  // A frame the browser could not finish inside its budget is a step the
  // reader sees. 20ms is a 60Hz frame with a little slack; anything past 32ms
  // is a frame dropped outright.
  const late = gaps.filter((g) => g > 20).length;
  console.log(
    `spin   ${a.spin}ms, a report every ${a.every}ms  reports ${out.sent}  draws ${s.top}` +
      `  draw ms ${s.drawMs.toFixed(1)}  longest ${s.longest.toFixed(1)}`,
  );
  console.log(
    `       frames ${gaps.length}  median ${at(0.5).toFixed(1)}  p90 ${at(0.9).toFixed(1)}  worst ${(gaps[gaps.length - 1] ?? 0).toFixed(1)}` +
      `  late (>20ms) ${late}  dropped (>32ms) ${gaps.filter((g) => g > 32).length}`,
  );
  console.log(
    `       browser: style ${spent("RecalcStyleDuration").toFixed(0)}ms/${(after.RecalcStyleCount ?? 0) - (before.RecalcStyleCount ?? 0)}` +
      `  layout ${spent("LayoutDuration").toFixed(0)}ms/${(after.LayoutCount ?? 0) - (before.LayoutCount ?? 0)}` +
      `  script ${spent("ScriptDuration").toFixed(0)}ms`,
  );
  const parts = Object.entries(s.parts)
    .filter(([, p]) => p.ms >= 1)
    .sort((x, y) => y[1].ms - x[1].ms)
    .map(([k, p]) => `${k} ${p.ms.toFixed(0)}ms/${p.n}`)
    .join("  ");
  if (parts !== "") console.log(`       ${parts}`);
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
  if (a.links) {
    await page.evaluate(() => {
      const t = document.querySelector(".tb-links");
      if (t && t.getAttribute("aria-pressed") !== "true") t.click();
      // Any field with a dependency will do; the first chip in the grid is
      // whatever the file starts with.
      const b = [...document.querySelectorAll(".hexview .hv-chip, .hexview button")].find((x) => x.dataset?.path !== undefined);
      b?.click();
    });
    await page.waitForTimeout(a.wait);
  }
  if (a.css !== "") await page.addStyleTag({ content: a.css });
  await page.evaluate(INSTRUMENT);

  // What the browser itself spends, as against what the view's own code does:
  // style and layout are the bill a draw runs up and the forced read at the
  // end of it pays.
  const cdp = await page.context().newCDPSession(page);
  await cdp.send("Performance.enable");
  const metrics = async () => {
    const { metrics: m } = await cdp.send("Performance.getMetrics");
    return Object.fromEntries(m.map((x) => [x.name, x.value]));
  };

  const box = await page.locator(".hv-rows").first().boundingBox();
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);

  const run = async (label, dir) => {
    await page.evaluate(() => window.__wc.reset());
    const before = await metrics();
    const t0 = Date.now();
    for (let i = 0; i < a.notches; i++) {
      await page.mouse.wheel(0, dir * a.delta);
      if (a.gap > 0) await page.waitForTimeout(a.gap);
    }
    await page.waitForTimeout(a.wait);
    const s = await page.evaluate(() => ({ ...window.__wc, reset: undefined, parts: JSON.parse(JSON.stringify(window.__wc.parts)) }));
    const wall = Date.now() - t0;
    const after = await metrics();
    const spent = (k) => ((after[k] ?? 0) - (before[k] ?? 0)) * 1000;
    console.log(
      `${label.padEnd(6)} notches ${a.notches}  wheel-driven draws ${String(s.top).padStart(3)}` +
        `  total draws ${String(s.draws).padStart(3)}  nested deepest ${s.deepest}` +
        `  draw ms ${s.drawMs.toFixed(1).padStart(7)}  longest ${s.longest.toFixed(1).padStart(6)}` +
        `  wall ${wall}ms`,
    );
    console.log(
      `       browser: style ${spent("RecalcStyleDuration").toFixed(0)}ms/${(after.RecalcStyleCount ?? 0) - (before.RecalcStyleCount ?? 0)}` +
        `  layout ${spent("LayoutDuration").toFixed(0)}ms/${(after.LayoutCount ?? 0) - (before.LayoutCount ?? 0)}` +
        `  script ${spent("ScriptDuration").toFixed(0)}ms`,
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

  // Five wheel reports inside one frame. A view that draws for each of them
  // draws five times; one that adds them up draws twice, once at once and once
  // on the frame it booked, or three times when the window it landed on had to
  // book a frame to ask what fields are on it. CDP's own `mouse.wheel` is paced
  // slower than a frame, so only reports made from inside the page tell these
  // apart.
  const burst = await page.evaluate(() => {
    const v = window.__qubero.view;
    const before = window.__wc.draws;
    for (let i = 0; i < 5; i++) {
      v.el.dispatchEvent(new WheelEvent("wheel", { deltaY: 30, cancelable: true, bubbles: true }));
    }
    return new Promise((done) =>
      requestAnimationFrame(() => requestAnimationFrame(() => done(window.__wc.draws - before))),
    );
  });
  console.log(`burst  five wheel reports in one frame -> ${burst} draws (2 or 3 is coalesced, 5 is not)`);

  await run("down", 1);
  await run("up", -1);
  if (a.spin > 0) await spin(page, a, metrics);
  await browser.close();
};

main().catch((e) => { console.error(e); process.exit(1); });
