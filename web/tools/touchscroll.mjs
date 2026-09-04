// Measures how closely the hex view follows a finger during a touch drag, and
// how evenly it glides after the finger lifts.
//
//   node web/tools/touchscroll.mjs --url http://localhost:17282/?url=/samples/notes.sqlite
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
  const a = { url: "http://localhost:17282/?url=/samples/notes.sqlite", px: 1500, step: 12, width: 390, height: 844, json: false, glide: 1500 };
  for (let i = 0; i < argv.length; i++) {
    const k = argv[i];
    const v = argv[i + 1];
    if (k === "--url") { a.url = v; i++; }
    else if (k === "--px") { a.px = Number(v); i++; }
    else if (k === "--step") { a.step = Number(v); i++; }
    else if (k === "--width") { a.width = Number(v); i++; }
    else if (k === "--height") { a.height = Number(v); i++; }
    else if (k === "--glide") { a.glide = Number(v); i++; }
    else if (k === "--json") a.json = true;
    else if (k === "--help" || k === "-h") { a.help = true; }
  }
  return a;
}

const SAMPLER = () => {
  const w = window;
  const rowsEl = document.querySelector(".hexview .hv-rows") || document.querySelector(".hv-rows");
  const state = { samples: [], travel: 0, step: -1, prev: null, cum: 0, seam: 0, stop: false, phase: "idle", lost: 0 };
  w.__ts = state;
  // Every row on screen is an anchor, not one: displacement is the median of
  // how far the rows that survived the frame moved. A row leaving the top or
  // arriving at the bottom then costs nothing, which a single anchor's
  // handover did.
  const snapshot = () => {
    const m = new Map();
    for (const row of rowsEl.querySelectorAll(".hv-row")) {
      const cell = row.querySelector("[data-off]");
      if (cell) m.set(cell.getAttribute("data-off"), row.getBoundingClientRect().top);
    }
    return m;
  };
  const frame = (now) => {
    if (state.stop) return;
    const now2 = snapshot();
    let delta = 0;
    if (state.prev) {
      const ds = [];
      for (const [off, top] of now2) { const was = state.prev.get(off); if (was !== undefined) ds.push(was - top); }
      if (ds.length === 0) state.lost++;
      else { ds.sort((a, b) => a - b); delta = ds[ds.length >> 1]; }
    }
    state.cum += delta;
    // Lifting the finger to reach for more screen throws a flick, and the frames
    // between that lift and the next touch are momentum, not tracking. Bank them
    // in `seam` so the step either side of the seam is scored on drag alone.
    if (state.phase === "restart") state.seam += delta;
    state.prev = now2;
    const view = w.__qubero && w.__qubero.view;
    state.samples.push({ t: now, phase: state.phase, content: delta, cum: state.cum - state.seam, travel: state.travel, step: state.step, topRow: view ? view.topRow : null, rows: now2.size });
    requestAnimationFrame(frame);
  };
  requestAnimationFrame(frame);
};

function summarise(samples, args) {
  const drag = samples.filter((s) => s.phase === "drag" && s.step >= 0);
  const glide = samples.filter((s) => s.phase === "glide");
  // One driven move takes longer than a frame, so a move is scored against the
  // last frame that was still under it: per move, how far the content went
  // against how far the finger went.
  const byStep = new Map();
  for (const s of drag) byStep.set(s.step, s);
  const steps = [...byStep.keys()].sort((a, b) => a - b).map((k) => byStep.get(k));
  const pairs = [];
  for (let i = 1; i < steps.length; i++)
    pairs.push({ i: steps[i].step, content: steps[i].cum - steps[i - 1].cum, finger: steps[i].travel - steps[i - 1].travel });
  const totalFinger = pairs.reduce((a, s) => a + s.finger, 0);
  const totalContent = pairs.reduce((a, s) => a + s.content, 0);
  let jumpy = 0, maxDev = 0, stalls = 0;
  const jumps = [];
  pairs.forEach((s) => {
    const dev = s.content - s.finger;
    if (Math.abs(dev) > 3) jumpy++;
    if (Math.abs(dev) > Math.abs(maxDev)) maxDev = dev;
    if (Math.abs(s.finger) > 1 && Math.abs(s.content) < 0.5) stalls++;
    jumps.push({ i: s.i, dev, content: s.content, finger: s.finger });
  });
  const dragSteps = pairs.length;
  jumps.sort((a, b) => Math.abs(b.dev) - Math.abs(a.dev));
  let glideMaxJump = 0;
  for (let i = 1; i < glide.length; i++) {
    const d = Math.abs(glide[i].content - glide[i - 1].content);
    if (d > glideMaxJump) glideMaxJump = d;
  }
  const meanDev = pairs.length ? pairs.reduce((a, s) => a + Math.abs(s.content - s.finger), 0) / pairs.length : 0;
  const glideContent = glide.reduce((a, s) => a + s.content, 0);
  return { args, dragSteps, dragFrames: drag.length, glideFrames: glide.length, totalFinger, totalContent, jumpy, maxDev, meanDev, stalls, glideContent, glideMaxJump, top: jumps.slice(0, 8) };
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  if (args.help) { console.log("--url --px --step --width --height --glide --json"); return; }
  const chromium = await loadChromium();
  const browser = await chromium.launch({ headless: true });
  const context = await browser.newContext({ viewport: { width: args.width, height: args.height }, hasTouch: true, isMobile: true, deviceScaleFactor: 1 });
  const page = await context.newPage();
  const errors = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  await page.goto(args.url, { waitUntil: "load" });

  await page.waitForFunction(() => !!window.__qubero, null, { timeout: 30000 });
  await page.evaluate(() => { window.__qubero.setView?.("hex"); });
  await page.waitForSelector(".hexview .hv-row [data-off]", { timeout: 30000 });
  // The outline arrives after the file does; headings change row heights, so
  // wait for the layout to settle before measuring anything.
  await page.waitForFunction(() => {
    const n = document.querySelectorAll(".hexview .hv-heading").length;
    const prev = window.__tsHead;
    window.__tsHead = n;
    return prev !== undefined && prev === n;
  }, null, { timeout: 30000, polling: 500 }).catch(() => {});
  await page.waitForTimeout(500);

  const cdp = await context.newCDPSession(page);
  const box = await page.evaluate(() => {
    const r = (document.querySelector(".hexview .hv-rows") || document.querySelector(".hv-rows")).getBoundingClientRect();
    return { x: r.x, y: r.y, w: r.width, h: r.height };
  });
  const x = Math.round(box.x + box.w / 2);
  let y = Math.round(box.y + Math.min(box.h - 20, box.h * 0.8));

  await page.evaluate(SAMPLER);
  await page.evaluate(() => { window.__ts.phase = "drag"; });
  await page.waitForTimeout(120);

  const touch = (type, py) => cdp.send("Input.dispatchTouchEvent", { type, touchPoints: type === "touchEnd" ? [] : [{ x, y: py, id: 1 }] });
  await touch("touchStart", y);
  const nextFrame = () => page.evaluate(() => new Promise((r) => requestAnimationFrame(() => r(null))));
  const frames = Math.round(args.px / args.step);
  const minY = 20;
  for (let i = 0; i < frames; i++) {
    if (y - args.step < minY) { // The finger ran out of screen; lift and start again lower down.
      if (process.env.TS_DEBUG) console.error(`restart at step ${i}, y=${y}, topRow=${await page.evaluate(() => window.__qubero?.view?.topRow)}`);
      await page.evaluate(() => { window.__ts.phase = "restart"; });
      await touch("touchEnd", y);
      y = Math.round(box.y + Math.min(box.h - 20, box.h * 0.8));
      await nextFrame();
      await touch("touchStart", y);
      await nextFrame();
      await page.evaluate(() => { window.__ts.phase = "drag"; });
    }
    y -= args.step;
    await page.evaluate(([i, d]) => { window.__ts.step = i; window.__ts.travel += d; }, [i, args.step]);
    await touch("touchMove", y);
    await nextFrame();
  }
  await page.evaluate(() => { window.__ts.phase = "glide"; });
  await touch("touchEnd", y);
  await page.waitForTimeout(args.glide);
  const nativeScroll = await page.evaluate(() => { const r = document.querySelector('.hexview .hv-rows'); return { winY: window.scrollY, rowsTop: r?.scrollTop, hexTop: document.querySelector('.hexview')?.scrollTop, bodyTop: document.body.scrollTop, docTop: document.documentElement.scrollTop }; });
  if (process.env.TS_DEBUG) console.error('native scroll', nativeScroll);
  const atEnd = await page.evaluate(() => { const v = window.__qubero?.view; return v ? v.topRow >= v.maxTopRow - 1 : false; }).catch(() => false);
  const samples = await page.evaluate(() => { window.__ts.stop = true; return { samples: window.__ts.samples, lost: window.__ts.lost, rowHeight: window.__qubero?.view?.rowHeight ?? null }; });
  await browser.close();

  // The sampler's finger deltas are negative going up; report distance moved.
  const out = summarise(samples.samples, args);
  out.lostFrames = samples.lost;
  out.rowHeight = samples.rowHeight;
  const dragS = samples.samples.filter((s) => s.phase === "drag" && s.topRow !== null);
  out.rowsScrolled = dragS.length ? dragS[dragS.length - 1].topRow - dragS[0].topRow : 0;
  out.errors = errors;
  out.atEnd = atEnd;
  if (args.json) { console.log(JSON.stringify({ summary: out, samples: samples.samples }, null, 1)); return; }
  const f = (n) => n.toFixed(1);
  console.log(`url            ${args.url}`);
  console.log(`viewport       ${args.width}x${args.height}   step ${args.step}px/frame`);
  console.log(`drag steps     ${out.dragSteps} (${out.dragFrames} sampled frames)   frames with no shared row ${out.lostFrames}`);
  console.log(`finger moved   ${f(out.totalFinger)} px`);
  console.log(`content moved  ${f(out.totalContent)} px  (${(out.totalContent / (out.totalFinger || 1)).toFixed(2)}x finger)${out.atEnd ? "  [hit end of file]" : ""}`);
  console.log(`jumpy steps    ${out.jumpy} of ${out.dragSteps} (|content-finger| > 3px)`);
  console.log(`max deviation  ${f(out.maxDev)} px   mean |deviation| ${f(out.meanDev)} px`);
  console.log(`rows scrolled  ${out.rowsScrolled} (row height at rest ${out.rowHeight}px; it changes with the rows on screen)`);
  console.log(`stalled steps  ${out.stalls} (finger moved, content did not)`);
  console.log(`glide          ${out.glideFrames} frames, ${f(out.glideContent)} px, max step change ${f(out.glideMaxJump)} px`);
  console.log(`biggest jumps  ${out.top.map((j) => `#${j.i}:${f(j.dev)}`).join("  ")}`);
  if (errors.length) console.log(`page errors    ${errors.length}: ${errors[0]}`);
}

main().catch((e) => { console.error(e); process.exit(1); });
