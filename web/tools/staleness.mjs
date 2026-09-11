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
// the things that change what a row says: going down the file and back up, a
// row or two at a time, the jump to the top and the jump far out, the cursor
// moving on and off a row, a selection across rows and cleared, an edit,
// binary mode, the field column shown, hidden and condensed, a resize, and the
// row width.
//
// Run two dev servers, one per checkout, and give it both ports:
//
//   cd web && npx vite --port 17291     # the change
//   cd ../other-checkout/web && npx vite --port 17292     # what it is judged against
//   node tools/staleness.mjs 17292 17291
//
// A difference says the two disagree. Which of them is right is a separate
// question, and the answer is not always the one that was there first: this
// tool's first catch was the old drawing keeping a heading a byte out of date.
//
// Three things to know before believing or disbelieving a failure:
//
//  - Hidden elements are skipped. Chips, headings and whole lines a row no
//    longer needs are hidden rather than taken out of the document, so that a
//    finger resting on one keeps it; what they still say is not something a
//    reader sees, and comparing it reports differences that are not there.
//  - The view is put on a named row rather than moved by so many pixels.
//    Every way of moving it by so much goes through the ledger of row heights,
//    which fills in as rows are drawn and as the file arrives, so two pages
//    told to go down 120px land on different rows and everything after that
//    differs for a reason that is not about either drawing. How far a scroll
//    goes is `wheelcost.mjs`'s question and `touchscroll.mjs`'s, not this
//    one's; which rows are kept and which are written is the same either way.
//  - Each page gets its own browser. Two pages of one browser are not equals:
//    the one behind has its animation frames throttled, and the view reads the
//    wheel once a frame.
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
  // Whether the rows really are drawn down the screen in the order the view
  // hands them out. Everything else here is read through that same order, so
  // it would agree with itself even if every row were in the wrong place: this
  // is the one line that asks the browser where the rows actually are.
  let falls = 0;
  let was = -Infinity;
  for (const row of v.grid.rows) {
    if (row.hidden) continue;
    const box = row.getBoundingClientRect();
    if (box.height === 0) continue;
    if (box.top < was - 0.5) falls++;
    was = box.top;
  }
  return { top: v.topRow, px: Math.round(v.topPx), rows: out, falls, pinned: v.grid.pinned.textContent ?? "" };
};

/** Wait until the file is open and the grid has rows, twice over. Opening a
 *  file puts its name in the address bar, which is a navigation, and a page
 *  that was ready before it happens is a page whose next instruction lands in
 *  a context that has gone. */
const ready = async (page) => {
  for (let i = 0; i < 2; i++) {
    await page.waitForFunction(() => window.__qubero !== undefined && window.__qubero.view.grid.rows.length > 0);
    await page.waitForTimeout(900);
  }
};

const act = async (page, what, arg) => {
  await page.evaluate(([w, a]) => {
    const v = window.__qubero.view;
    // Put the view on a named row, rather than moving it by so much. Every
    // way of moving it by so much goes through the ledger of row heights,
    // which fills in as rows are drawn and as the file arrives, so two pages
    // told to go down 120px land on different rows -- and then everything
    // downstream differs, from which window the fields were asked for to what
    // the strip over the top carries, for a reason that is not about either
    // drawing. Named rows give the two pages the same history, which is the
    // only footing on which what they drew can be compared. Rows are still
    // reused and released exactly as a scroll would: a view moved from row 8
    // to row 10 keeps the same twenty-seven rows either way.
    if (w === "at") { v.topRow = a; v.topPx = 0; v.render(); }
    else if (w === "top") v.scrollToY(0);
    else if (w === "cursor") v.setCursor(a, { select: "clear" });
    else if (w === "select") v.selectRange(a * 8, (a + 300) * 8);
    else if (w === "unselect") v.selectRange(0, 0);
    else if (w === "bpr") v.setBytesPerRow(a);
    else if (w === "mode") v.setMode(a);
    else if (w === "column") v.setRightColumn(a);
    else if (w === "resize") { v.relayout(); }
    else if (w === "highlight") { v.setHighlight(a === null ? null : { startBit: a * 8, endBit: (a + 40) * 8 }); v.render(); }
    else if (w === "linked") { v.setLinkedRange(a === null ? null : a * 8, ((a ?? 0) + 24) * 8); v.render(); }
    else if (w === "type") { v.el.focus(); v.el.dispatchEvent(new KeyboardEvent("keydown", { key: a, bubbles: true, cancelable: true })); }
  }, [what, arg]);
  await page.waitForTimeout(260);
};

// Down the file two rows at a time and back up, which is the case a reader
// notices and the one where a row keeps its element; then the things that
// change what a row says without moving it at all.
const SCRIPT = [];
for (let r = 0; r <= 30; r += 2) SCRIPT.push(["at", r]);
for (let r = 28; r >= 0; r -= 2) SCRIPT.push(["at", r]);
// One row at a time, the jump to the top, and the jump back out: a move of one
// row keeps all but one, and a jump keeps none.
SCRIPT.push(["at", 1], ["at", 2], ["at", 1], ["top", 0], ["at", 400], ["at", 401], ["at", 3]);
SCRIPT.push(["cursor", 80], ["cursor", 81], ["cursor", 500], ["at", 5], ["cursor", 96], ["at", 3]);
SCRIPT.push(["select", 120], ["at", 5], ["at", 3], ["unselect", 0]);
SCRIPT.push(["mode", "binary"], ["at", 5], ["mode", "hex"], ["at", 3]);
SCRIPT.push(["column", "text"], ["at", 4], ["column", "both"], ["at", 3]);
SCRIPT.push(["column", "fields"], ["at", 4], ["column", "both-condensed"], ["at", 5], ["column", "both"], ["at", 3]);
SCRIPT.push(["resize", 0], ["at", 5], ["resize", 0], ["at", 3]);
SCRIPT.push(["bpr", 8], ["at", 5], ["at", 3], ["bpr", 32], ["at", 5], ["bpr", 16]);
// A field lit from the sidebar, and the arrows drawn to what reads it: both
// mark bytes on rows that are not moving.
SCRIPT.push(["highlight", 200], ["at", 5], ["at", 3], ["highlight", null]);
SCRIPT.push(["linked", 260], ["at", 4], ["linked", null], ["at", 3]);
// An edit: the bytes a row draws change under it while the row stays where it
// is, which no scroll can bring about.
SCRIPT.push(["cursor", 300], ["type", "a"], ["type", "5"], ["at", 5], ["at", 3]);
SCRIPT.push(["top", 0], ["at", 60], ["at", 0]);

// One browser each. Two pages of one browser are not equals: the one behind
// has its animation frames throttled, the view reads the wheel once a frame,
// and the page behind then takes a shorter scroll than the page in front for a
// reason that has nothing to do with either build.
const browsers = [await chromium.launch(), await chromium.launch()];
let bad = 0;
let checks = 0;
for (const sample of SAMPLES) {
  const pages = [];
  for (const [i, port] of [OLD, NEW].entries()) {
    const p = await browsers[i].newPage({ viewport: { width: 1280, height: 800 } });
    await p.goto(`http://localhost:${port}/?url=/samples/${sample}`);
    await ready(p);
    pages.push(p);
  }
  let said = 0;
  try {
  for (const [what, arg] of SCRIPT) {
    for (const p of pages) await act(p, what, arg);
    const a = await pages[0].evaluate(SNAPSHOT);
    const b = await pages[1].evaluate(SNAPSHOT);
    checks++;
    const where = `${sample} after ${what} ${arg} (old at ${a.top}+${a.px}, new at ${b.top}+${b.px})`;
    if (a.top !== b.top || a.px !== b.px) { console.log(`${where}: stopped in different places`); bad++; said++; }
    else if (a.falls > 0 || b.falls > 0) { console.log(`${where}: rows out of order down the screen (old ${a.falls}, new ${b.falls})`); bad++; said++; }
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
  } catch (e) {
    // One file that will not open, or a page that went away, is a result about
    // that file and not a reason to lose the ones still to run.
    console.log(`${sample}: could not be driven to the end: ${e.message.split("\n")[0]}`);
    bad++;
    said++;
  }
  for (const p of pages) await p.close();
  console.log(`${sample}: ${said === 0 ? "same throughout" : `${said} steps differed`}`);
}
for (const br of browsers) await br.close();
console.log(`${checks} steps compared, ${bad} that did not match`);
process.exit(bad === 0 ? 0 : 1);
