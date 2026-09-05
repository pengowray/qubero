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
    const cell = view.grid.cellFor(0, 0);
    for (let i = 0; i < 60; i++) {
      view.scrollToY(2000 + i * 20);
      await new Promise(resolve => requestAnimationFrame(resolve));
    }
    const checks = {
      headerReused: headerNode === view.grid.header.firstChild,
      spansReused: timings.spans.length === 1,
      cellsReused: cell === view.grid.cellFor(0, 0),
    };
    const calls = timings.spans.length;
    doc.overwrite(48, Uint8Array.of(123));
    checks.editsInvalidate = timings.spans.length > calls;
    view.setCursor(48);
    checks.editVisible = view.el.querySelector('.hv-hex [data-off="48"]').textContent === "7b";
    view.setCursor(doc.lengthBytes - 8);
    checks.boundaryRefetched = view.fetch.spanCache.spans.some(s => s.value === "IEND");
    // Growing the viewport needs more rows, but leaves existing cells alive.
    const first = view.grid.cellFor(0, 0);
    view.el.style.height = "760px";
    view.relayout();
    checks.resizeReusesCells = first === view.grid.cellFor(0, 0);
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
} finally { await browser.close(); }
