// What a resize does to the place the hex view is at.
//
// Run against Vite with TEST_URL and, optionally, a bundled PLAYWRIGHT_MODULE:
//
//   PLAYWRIGHT_MODULE="file:///.../playwright/index.mjs" \
//   TEST_URL="http://localhost:17281" node test/hexresize.browser.mjs
//
// Two rules, and they are about `topPx`, which is how far into the top row the
// top edge of the view falls. A row is as tall as the chips beside it wrapped
// to, so a box of a different width gives the same row a different height, and
// the same `topPx` then lands on different bytes. So:
//
//  - A box of a different width that really did change the top row's height
//    puts that row's own start against the edge. The reader keeps the row they
//    were reading, and the view is where a reload would have put it.
//  - A box of the same width changes no row's height, so nothing moves at all
//    and nothing that was measured is thrown away.
import assert from "node:assert/strict";
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || "playwright");
const browser = await chromium.launch({ headless: true });
try {
  const page = await browser.newPage({ viewport: { width: 1400, height: 900 } });
  await page.goto(process.env.TEST_URL || "http://localhost:17272");
  const result = await page.evaluate(async () => {
    const { Doc } = await import("/src/doc.ts");
    const { HexView } = await import("/src/hexview.ts");
    const chunk = (kind, data) => {
      const b = new Uint8Array(data.length + 12);
      new DataView(b.buffer).setUint32(0, data.length);
      b.set(new TextEncoder().encode(kind), 4);
      b.set(data, 8);
      return b;
    };
    const header = new Uint8Array(13);
    new DataView(header.buffer).setUint32(0, 1024);
    new DataView(header.buffer).setUint32(4, 1024);
    header[8] = 8;
    header[9] = 6;
    const doc = await Doc.open(
      new File(
        [
          Uint8Array.of(137, 80, 78, 71, 13, 10, 26, 10),
          chunk("IHDR", header),
          chunk("IDAT", new Uint8Array(64 * 1024)),
          chunk("IEND", new Uint8Array()),
        ],
        "wide.png",
      ),
    );
    await doc.ensureRange(0, doc.lengthBytes);
    doc.setTemplate("png");
    const view = new HexView(doc);
    document.body.replaceChildren(view.el);
    const size = (w, h) => {
      view.el.style.cssText = `height:${h}px;width:${w}px;flex:none`;
      view.relayout();
    };
    size(1400, 800);
    view.setBytesPerRow(16);
    view.setRightColumn("both");
    await new Promise((r) => requestAnimationFrame(r));
    view.relayout();

    // The row the file opens on: headings above it and chips beside it, so it
    // is several times the height of a plain one and a width that rewraps its
    // chips moves it by a lot.
    const rowHeight = () => view.grid.rows[0].getBoundingClientRect().height;
    const wide = rowHeight();
    // Part way into it, as a scroll would leave it.
    view.topRow = 0;
    view.topPx = 20;
    view.render();
    await new Promise((r) => requestAnimationFrame(r));

    // Narrower: the chips wrap onto more lines and the row grows.
    size(700, 800);
    await new Promise((r) => requestAnimationFrame(r));
    const narrow = rowHeight();
    const onWidth = {
      rowGrew: narrow !== wide,
      // The row is whole against the edge, which is where a reload puts it.
      atRowStart: view.topPx === 0 && view.topRow === 0,
      drawnAtEdge: Math.abs(view.grid.rows[0].getBoundingClientRect().top - view.el.querySelector(".hv-rows").getBoundingClientRect().top) < 1,
      // What the ledger holds for the row is what the browser gave it.
      ledgerAgrees: Math.abs(view.ledger.heightOf(0) - narrow) < 1.5,
    };

    // A box that is only a different height, in both arrangements: the chips
    // beside the bytes, and the chips under them in a box too narrow to hold a
    // column beside. Neither can rewrap for a height, so neither may move.
    const sameWidth = async (w, from, to) => {
      size(w, from);
      await new Promise((r) => requestAnimationFrame(r));
      // Over the first row at this width, so that it has been measured at this
      // width and what the ledger says about it means something.
      view.scrollTo(0);
      await new Promise((r) => requestAnimationFrame(r));
      // Down the file, so there is something measured to keep, and part way
      // into a row again.
      view.scrollTo(40);
      await new Promise((r) => requestAnimationFrame(r));
      view.topPx = 12;
      view.render();
      await new Promise((r) => requestAnimationFrame(r));
      const at = { row: view.topRow, px: view.topPx, height: view.ledger.heightOf(view.topRow), total: view.ledger.totalHeight(), first: view.ledger.heightOf(0) };
      size(w, to);
      await new Promise((r) => requestAnimationFrame(r));
      return {
        arrangement: view.arrangement,
        at,
        checks: {
          keptPlace: view.topRow === at.row && view.topPx === at.px,
          keptHeights: view.ledger.heightOf(at.row) === at.height,
          keptTotal: view.ledger.totalHeight() >= at.total,
          // A row nowhere near the view, and the only way to know how tall it
          // is is to have measured it: forgetting it would take the total with
          // it.
          keptRowsOffScreen: view.ledger.heightOf(0) === at.first && at.first > view.ledger.baseHeight,
        },
      };
    };
    // 800 rather than the widest box there is: a row wide enough to hold every
    // chip on one line is the base height, and a ledger that had been emptied
    // would answer the same for it, so the check would pass without meaning
    // anything.
    const onHeightNarrow = await sameWidth(700, 800, 600);
    const onHeightWide = await sameWidth(800, 800, 600);

    // A viewport that gets smaller during a touch drag. A phone does this by
    // itself: the URL bar comes back while a finger is on the screen. Trimming
    // the row pool there takes an element out of the document, the browser
    // calls the touch off, and the drag stops dead part way down the file. The
    // pool is not in screen order either, so the element trimmed is whichever
    // file row it last drew and can be the one under the finger.
    size(800, 800);
    await new Promise((r) => requestAnimationFrame(r));
    const rowsEl = view.el.querySelector(".hv-rows");
    // A synthetic pointer has no capture to take; the drag itself is real.
    const capture = rowsEl.setPointerCapture.bind(rowsEl);
    rowsEl.setPointerCapture = () => {};
    const pool = [...view.grid.rows];
    const under = pool[Math.min(10, pool.length - 1)];
    const touch = (type, clientY) =>
      rowsEl.dispatchEvent(new PointerEvent(type, { pointerId: 1, pointerType: "touch", clientY, bubbles: true }));
    touch("pointerdown", 400);
    touch("pointermove", 380);
    size(800, 380);
    await new Promise((r) => requestAnimationFrame(r));
    const onShrinkUnderFinger = {
      poolWholeUnderFinger: pool.every((row) => row.isConnected),
      rowUnderFingerStillThere: under.isConnected,
    };
    touch("pointerup", 360);
    await new Promise((r) => requestAnimationFrame(r));
    // And the pool really did have rows to give up, so the check above is not
    // passing because nothing was ever going to be trimmed.
    onShrinkUnderFinger.trimmedOnceTheFingerIsOff = view.grid.rows.length < pool.length;
    rowsEl.setPointerCapture = capture;
    return {
      onWidth,
      onShrinkUnderFinger,
      onHeightNarrow: onHeightNarrow.checks,
      onHeightWide: onHeightWide.checks,
      arrangements: { narrow: onHeightNarrow.arrangement, wide: onHeightWide.arrangement },
      wide,
      narrow,
      at: { narrow: onHeightNarrow.at, wide: onHeightWide.at },
    };
  });
  console.log(JSON.stringify(result, null, 2));
  for (const group of ["onWidth", "onShrinkUnderFinger", "onHeightNarrow", "onHeightWide"]) {
    for (const [name, ok] of Object.entries(result[group])) assert(ok, `${group}.${name}`);
  }
  // The two same-width legs are only worth running if they were in different
  // arrangements: a check that passed twice in one of them says half as much.
  assert(result.arrangements.narrow !== result.arrangements.wide, "arrangements differ");
} finally {
  await browser.close();
}
