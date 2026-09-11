// Does a change to the hex view's drawing change what is on the screen?
//
// The hex view keeps a row's elements for as long as that row's address is on
// screen and rewrites only what changed, which is what makes a scroll cheap
// and what makes a wrong answer possible: a guard that misses something leaves
// a row saying what it said before. Nothing that times a scroll can see that,
// and a screenshot only catches it where the reader happened to look.
//
// So this runs the same script against two servers -- one on the drawing
// before the change, one on the drawing after -- and compares every cell,
// class, chip and heading the reader can see, after every step. The script is
// the things that change what a row says: scrolling in and back out, a notch
// at a time and a screenful at a time, the jump to the top, the cursor moving
// on and off a row, a selection dragged across rows and cleared, binary mode,
// the field column shown, hidden and condensed, a resize, and the row width.
//
// Run two dev servers, one per checkout, and give it both ports:
//
//   cd web && npx vite --port 17291     # the change
//   cd ../other-checkout/web && npx vite --port 17292     # what it is judged against
//   node tools/staleness.mjs 17292 17291
//
// Hidden elements are skipped. Chips, headings and whole lines a row no longer
// needs are hidden rather than taken out of the document, so that a finger
// resting on one keeps it; what they still say is not something a reader sees,
// and comparing it reports differences that are not there. That is worth
// knowing before believing a failure.
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
const chromium = await loadChromium();

const OLD = process.argv[2] ?? "17292";
const NEW = process.argv[3] ?? "17291";
const SAMPLES = process.argv.length > 4 ? process.argv.slice(4) : ["hello.exe", "notes.sqlite", "bat.wav", "notes.zip", "tagged.mp3", "zarr-zip64.zip", "tiny.png"];

/** What the grid says, in the order the rows are drawn: every cell's text, the
 *  class on it, the byte it stands for, the chips and headings beside it, and
 *  the strip pinned over the top. Everything a reader can see. */
const SNAPSHOT = () => {
  const v = window.__qubero.view;
  const out = [];
  for (const row of v.grid.rows) {
    const bits = [];
    for (const el of row.querySelectorAll("[data-off], .hv-addr, .hv-chip, .hv-heading, .hv-vals td")) {
      // Chips, headings and whole lines a row no longer needs are hidden
      // rather than taken away, so a leftover is not something a reader sees.
      if (el.closest("[hidden]") !== null) continue;
      bits.push(`${el.className}|${el.getAttribute("data-off") ?? ""}|${el.textContent}`);
    }
    out.push(`${row.hidden ? "H" : ""}${bits.join("\u0001")}`);
  }
  return { top: v.topRow, px: Math.round(v.topPx), rows: out, pinned: v.grid.pinned.textContent ?? "" };
};

const ready = async (page) => {
  await page.waitForFunction(() => window.__qubero !== undefined && window.__qubero.view.grid.rows.length > 0);
  await page.waitForTimeout(800);
};

const act = async (page, what, arg) => {
  await page.evaluate(([w, a]) => {
    const v = window.__qubero.view;
    if (w === "wheel") for (let i = 0; i < Math.abs(a); i++) v.el.dispatchEvent(new WheelEvent("wheel", { deltaY: a < 0 ? -40 : 40, cancelable: true, bubbles: true }));
    else if (w === "top") v.scrollToY(0);
    else if (w === "cursor") v.setCursor(a, { select: "clear" });
    else if (w === "select") v.selectRange(a * 8, (a + 300) * 8);
    else if (w === "unselect") v.selectRange(0, 0);
    else if (w === "bpr") v.setBytesPerRow(a);
    else if (w === "mode") v.setMode(a);
    else if (w === "column") v.setRightColumn(a);
    else if (w === "resize") { v.relayout(); }
  }, [what, arg]);
  await page.waitForTimeout(260);
};

// Scroll in, scroll back, jump to the top, then the things that change what a
// row says without moving it: the cursor, a selection, the shape of the view.
const SCRIPT = [];
for (let i = 0; i < 10; i++) SCRIPT.push(["wheel", 3]);
for (let i = 0; i < 6; i++) SCRIPT.push(["wheel", -3]);
SCRIPT.push(["wheel", 1], ["wheel", -1], ["top", 0], ["wheel", 1]);
SCRIPT.push(["cursor", 80], ["cursor", 81], ["cursor", 500], ["wheel", 2], ["cursor", 96], ["wheel", -2]);
SCRIPT.push(["select", 120], ["wheel", 2], ["wheel", -2], ["unselect", 0]);
SCRIPT.push(["mode", "binary"], ["wheel", 2], ["mode", "hex"], ["wheel", -2]);
SCRIPT.push(["column", "text"], ["wheel", 1], ["column", "both"], ["wheel", -1]);
SCRIPT.push(["column", "fields"], ["wheel", 1], ["column", "both-condensed"], ["wheel", 1], ["column", "both"], ["wheel", -2]);
SCRIPT.push(["resize", 0], ["wheel", 2], ["resize", 0], ["wheel", -2]);
SCRIPT.push(["bpr", 8], ["wheel", 2], ["wheel", -2], ["bpr", 32], ["wheel", 2], ["bpr", 16]);
SCRIPT.push(["top", 0], ["wheel", 60], ["wheel", -60]);

const browser = await chromium.launch();
let bad = 0;
let checks = 0;
for (const sample of SAMPLES) {
  const pages = [];
  for (const port of [OLD, NEW]) {
    const p = await browser.newPage({ viewport: { width: 1280, height: 800 } });
    await p.goto(`http://localhost:${port}/?url=/samples/${sample}`);
    await ready(p);
    pages.push(p);
  }
  let said = 0;
  for (const [what, arg] of SCRIPT) {
    for (const p of pages) await act(p, what, arg);
    const [a, b] = [await pages[0].evaluate(SNAPSHOT), await pages[1].evaluate(SNAPSHOT)];
    checks++;
    const where = `${sample} after ${what} ${arg} (old at ${a.top}+${a.px}, new at ${b.top}+${b.px})`;
    if (a.top !== b.top || a.px !== b.px) { console.log(`${where}: landed in different places`); bad++; said++; }
    else if (a.rows.length !== b.rows.length) { console.log(`${where}: ${a.rows.length} rows vs ${b.rows.length}`); bad++; said++; }
    else if (a.pinned !== b.pinned) { console.log(`${where}: pinned ${JSON.stringify(a.pinned)} vs ${JSON.stringify(b.pinned)}`); bad++; said++; }
    else {
      for (let i = 0; i < a.rows.length; i++) {
        if (a.rows[i] === b.rows[i]) continue;
        const x = String(a.rows[i]);
        const y = String(b.rows[i]);
        let at = 0;
        while (at < x.length && x[at] === y[at]) at++;
        console.log(`${where} row ${i} differs at ${at}:`);
        console.log(`  old: ...${x.slice(Math.max(0, at - 60), at + 90)}`);
        console.log(`  new: ...${y.slice(Math.max(0, at - 60), at + 90)}`);
        bad++;
        said++;
        break;
      }
    }
    if (said > 6) { console.log(`${sample}: stopping, too many`); break; }
  }
  for (const p of pages) await p.close();
  console.log(`${sample}: ${said === 0 ? "same throughout" : `${said} steps differed`}`);
}
await browser.close();
console.log(`${checks} steps compared, ${bad} that did not match`);
process.exit(bad === 0 ? 0 : 1);
