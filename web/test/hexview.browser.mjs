// Run against Vite with TEST_URL and, optionally, a bundled PLAYWRIGHT_MODULE.
// Timings are diagnostic; assertions check bounded work and visible behavior.
import assert from "node:assert/strict";
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || "playwright");
const browser = await chromium.launch({ channel: "msedge", headless: true });
try {
  const page = await browser.newPage({ viewport: { width: 1100, height: 800 } });
  await page.goto(process.env.TEST_URL || "http://127.0.0.1:17272");
  const result = await page.evaluate(async () => {
    const { Doc } = await import("/src/doc.ts");
    const { HexView } = await import("/src/hexview.ts");
    const chunk = (kind, data) => {
      const b = new Uint8Array(data.length + 12);
      new DataView(b.buffer).setUint32(0, data.length);
      b.set(new TextEncoder().encode(kind), 4); b.set(data, 8);
      return b;
    };
    const header = new Uint8Array(13);
    new DataView(header.buffer).setUint32(0, 1024);
    new DataView(header.buffer).setUint32(4, 1024);
    header[8] = 8; header[9] = 6;
    // Structural fixture: PNG's template treats IDAT as opaque bytes. The
    // full application loading check uses a valid compressed image and CRCs.
    const doc = await Doc.open(new File([
      Uint8Array.of(137,80,78,71,13,10,26,10), chunk("IHDR", header),
      chunk("IDAT", new Uint8Array(4 * 1024 * 1024)), chunk("IEND", new Uint8Array()),
    ], "large.png"));
    await doc.ensureRange(0, doc.lengthBytes);
    doc.setTemplate("png");
    const timings = {};
    const wrap = (obj, name) => {
      const original = obj[name];
      obj[name] = function (...args) {
        const start = performance.now();
        try { return original.apply(this, args); }
        finally { (timings[name] ??= []).push(performance.now() - start); }
      };
    };
    wrap(doc, "spans");
    const view = new HexView(doc);
    document.body.replaceChildren(view.el);
    view.el.style.cssText = "height:700px;width:1000px;flex:none";
    for (const name of ["frame", "render"]) wrap(view, name);
    for (const name of ["write", "heights"]) wrap(view.grid, name);
    const start = performance.now();
    view.setRightColumn("both");
    const load = performance.now() - start;
    await new Promise(resolve => requestAnimationFrame(resolve));
    const headerNode = view.grid.header.firstChild;
    // A cell by the address it holds. An element stands for a file row and not
    // for a place on screen, so asking for "the cell at slot 0" after a scroll
    // asks about a different row and tells you nothing about reuse.
    const cellAt = off => view.el.querySelector(`.hv-hex [data-off="${off}"]`);
    // The pool is never taken apart. Which row is drawn in which element
    // changes as the view moves; the set of elements does not.
    const pool = new Set(view.grid.rows);
    for (let i = 0; i < 60; i++) {
      view.scrollToY(2000 + i * 20);
      await new Promise(resolve => requestAnimationFrame(resolve));
    }
    const checks = {
      headerReused: headerNode === view.grid.header.firstChild,
      spansReused: timings.spans.length === 1,
      poolReused: view.grid.rows.every(row => pool.has(row)),
    };
    // And an element follows the address it holds: a row still on screen after
    // a scroll is still drawn in the element it was drawn in before. Taken
    // from the middle of the view so it survives the scroll either way.
    const held = view.grid.cellFor(15, 0);
    const heldOff = held.dataset.off;
    view.scrollToY(2000 + 59 * 20 + 120);
    await new Promise(resolve => requestAnimationFrame(resolve));
    // It really did move up the screen, so identity is not being read off an
    // element that never left its slot.
    checks.cellMoved = view.grid.cellFor(15, 0) !== held;
    checks.cellFollowsAddress = cellAt(heldOff) === held;
    const calls = timings.spans.length;
    doc.overwrite(48, Uint8Array.of(123));
    checks.editsInvalidate = timings.spans.length > calls;
    view.setCursor(48);
    checks.editVisible = view.el.querySelector('.hv-hex [data-off="48"]').textContent === "7b";
    view.setCursor(doc.lengthBytes - 8);
    checks.boundaryRefetched = view.fetch.spanCache.spans.some(s => s.value === "IEND");
    // Growing the viewport needs more rows, but leaves the ones already drawn
    // alive and in the elements they were drawn in. Not asked of the top row:
    // a resize keeps the reader on the same byte, which can put a different
    // row at the top, and that is the resize working rather than failing.
    const anchored = view.grid.cellFor(2, 0);
    const anchoredOff = anchored.dataset.off;
    view.el.style.height = "760px";
    view.relayout();
    checks.resizeKeepsCells = cellAt(anchoredOff) === anchored;
    // A different row width is a different row, so here the elements do go.
    const first = view.grid.cellFor(0, 0);
    view.setBytesPerRow(32);
    checks.widthChangesCells = first !== view.grid.cellFor(0, 0) && view.grid.cellFor(0, 31) !== undefined;
    view.el.hidden = true;
    const writes = timings.write.length;
    doc.overwrite(48, Uint8Array.of(124));
    view.render();
    checks.hiddenDoesNotDraw = timings.write.length === writes;
    view.el.hidden = false;
    view.relayout();
    view.setCursor(48);
    checks.showRefreshes = view.el.querySelector('.hv-hex [data-off="48"]').textContent === "7c";
    view.setRightColumn("text");
    checks.columnChanges = !view.grid.header.textContent.includes("Fields");
    return { load, checks, rows: view.grid.rows.length, timings: Object.fromEntries(Object.entries(timings).map(([k,v]) => [k, { count: v.length, total: v.reduce((a,b)=>a+b,0), max: Math.max(...v) }])) };
  });
  console.log(JSON.stringify(result, null, 2));
  assert(result.rows < 100);
  for (const [name, ok] of Object.entries(result.checks)) assert(ok, name);

  // What a chip hands back when it is pressed. Here rather than in the unit
  // tests because filling a chip wants a DOM, and here rather than against a
  // sample file because no file the other checks build has a run to fold.
  const chip = await page.evaluate(async () => {
    const { fillChip, newChip } = await import("/src/hexchips.ts");
    const { chipText } = await import("/src/chipplan.ts");
    const span = (name, offsetBytes, sizeBytes) => ({
      path: [4, offsetBytes], name, trail: ["chunk", "elements"], type: "u32 le", offset_bits: offsetBytes * 8,
      size_bits: sizeBytes * 8, value: String(offsetBytes), kind: "uint", gap: false, count: 0, unit: null,
      line: null, sample: [], parts: [], bits: null, opens: false,
    });
    const picks = [];
    const el = newChip((path, throughBit) => picks.push({ path, throughBit }));
    document.body.append(el);
    // A run of three four-byte elements starting at byte 100, drawn as one
    // chip: the pick has to be all twelve bytes and not the first four.
    const run = [span("[0]", 100, 4), span("[1]", 104, 4), span("[2]", 108, 4)];
    const folded = { span: run[0], carried: false, run };
    fillChip(el, folded, chipText(folded));
    el.click();
    const one = { span: span("page_size", 200, 2), carried: false, run: [] };
    fillChip(el, one, chipText(one));
    el.click();
    return {
      runEnd: el._runEnd,
      picks,
      ends: { run: (108 + 4) * 8, one: undefined },
    };
  });
  console.log(JSON.stringify(chip, null, 2));
  assert.equal(chip.picks.length, 2, "both presses reached the handler");
  assert.equal(chip.picks[0].throughBit, chip.ends.run, "a folded chip picks its whole run");
  assert.equal(chip.picks[1].throughBit, undefined, "a chip that folded nothing picks one field");
  // The element is reused, so the run's end has to be cleared and not left to
  // widen the next field drawn in it.
  assert.equal(chip.runEnd, undefined, "a refilled chip does not keep the old run's end");
} finally { await browser.close(); }
