// Run against a Vite dev server. PLAYWRIGHT_MODULE can point to a bundled
// Playwright installation; no browser dependency is needed by the unit suite.
import assert from "node:assert/strict";
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || "playwright");
const browser = await chromium.launch({ channel: "msedge", headless: true });
try {
  const page = await browser.newPage({ viewport: { width: 1000, height: 700 } });
  const errors = [];
  page.on("pageerror", error => errors.push(error.message));
  await page.goto(process.env.TEST_URL || "http://127.0.0.1:17272");
  await page.evaluate(async () => {
    const { Doc } = await import("/src/doc.ts");
    const { TextView } = await import("/src/textview.ts");
    const source = Array.from({length: 800}, (_, i) => `${i}: ${"Words with spaces and 日本語 🙂. ".repeat(i % 5 === 0 ? 80 : 3)}\r\n`).join("");
    const doc = await Doc.open(new File([source], "wrap-test.txt"));
    const view = new TextView(doc);
    document.body.replaceChildren(view.el);
    view.el.style.cssText = "height:600px;width:900px;flex:none";
    window.testView = view;
    await view.setEncoding("UTF-8");
    await view.draw();
  });
  await page.waitForTimeout(300);
  const result = await page.evaluate(async () => {
    const v = window.testView;
    const report = {};
    report.offHeight = v.rows.firstElementChild.offsetHeight;
    report.offCharacters = v.rows.firstElementChild.textContent.length;
    await v.setWrap("word");
    report.wordHeight = v.rows.firstElementChild.offsetHeight;
    report.gutterHeight = v.gutter.firstElementChild.offsetHeight;
    report.characters = v.rows.firstElementChild.textContent.length;
    v.scrollWrapped(250);
    report.offset = v.viewOffset;
    report.scroll = v.scroll.scrollTop;
    const before = v.rows.firstElementChild.getBoundingClientRect().top;
    await v.draw();
    report.drift = v.rows.firstElementChild.getBoundingClientRect().top - before;
    await v.setByte(1000);
    const caret = v.rows.querySelector(".is-cursor, .tv-caret").getBoundingClientRect();
    const port = v.scroll.getBoundingClientRect();
    report.caretVisible = caret.top >= port.top - 1 && caret.bottom <= port.bottom + 1;
    const original = v.cursor;
    const originalY = v.rows.querySelector(".is-cursor").getBoundingClientRect().top;
    await v.moveLine(1);
    report.visualStep = v.rows.querySelector(".is-cursor").getBoundingClientRect().top - originalY;
    report.movement = [original, v.cursor, v.rows.querySelector(".is-cursor").textContent];
    report.visualDown = v.cursor > original && v.cursor < v.lines[0].at + v.lines[0].len;
    v.setSelection(original, v.cursor);
    report.selectionShown = v.rows.querySelector(".is-sel") !== null;
    v.el.style.width = "500px";
    await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    await v.draw();
    report.resizedHeight = v.rows.firstElementChild.offsetHeight;
    report.resizedGutter = v.gutter.firstElementChild.offsetHeight;
    await v.setWrap("line");
    report.lineHeight = v.rows.firstElementChild.offsetHeight;
    await v.setWrap("off");
    report.restoredHeight = v.rows.firstElementChild.offsetHeight;
    report.restoredGutter = v.gutter.firstElementChild.offsetHeight;
    const noWrapCaret = v.rows.querySelector(".is-cursor, .tv-caret").getBoundingClientRect();
    report.noWrapCaretVisible = noWrapCaret.left >= v.rows.getBoundingClientRect().left && noWrapCaret.right <= v.scroll.getBoundingClientRect().right;
    return report;
  });
  assert.equal(result.offHeight, 20);
  assert(result.wordHeight > 20);
  assert.equal(result.wordHeight, result.gutterHeight);
  assert(result.characters > result.offCharacters);
  assert.equal(result.scroll, 250);
  assert(Math.abs(result.drift) < 1, JSON.stringify(result));
  assert(result.caretVisible, JSON.stringify(result));
  assert(result.visualDown, JSON.stringify(result));
  assert.equal(result.visualStep, 20, JSON.stringify(result));
  assert(result.selectionShown);
  assert(result.resizedHeight > result.wordHeight);
  assert.equal(result.resizedHeight, result.resizedGutter);
  assert(result.lineHeight > 20);
  assert.equal(result.restoredHeight, 20);
  assert.equal(result.restoredGutter, 20);
  assert(result.noWrapCaretVisible, JSON.stringify(result));
  // A native thumb move changes scrollTop before its scroll event / RAF.
  // Background indexing or row measurements must not restore the old anchor.
  for (const mode of ["word", "line"]) {
    await page.evaluate(async mode => {
      const { Doc } = await import("/src/doc.ts");
      const { TextView } = await import("/src/textview.ts");
      const bytes = new Uint8Array(3.75 * 1024 ** 2);
      let seed = 17;
      for (let i = 0; i < bytes.length; i++) {
        seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0;
        bytes[i] = seed >>> 24;
      }
      const v = new TextView(await Doc.open(new File([bytes], "binary.bin")));
      document.body.replaceChildren(v.el);
      v.el.style.cssText = "height:600px;width:600px;flex:none";
      window.testView = v;
      await v.draw();
      await v.setWrap(mode);
    }, mode);
    await page.waitForFunction(() => window.testView.index.indexedTo >= 3.75 * 1024 ** 2 && !window.testView.drawing);
    const jump = await page.evaluate(async () => {
      const v = window.testView;
      const target = v.span() * 0.8;
      const expected = v.lineAtY(target);
      v.scroll.scrollTop = target;
      v.reanchor();
      await new Promise(resolve => setTimeout(resolve, 500));
      return { expected, actual: v.viewLine };
    });
    assert(Math.abs(jump.actual - jump.expected) <= 1, `${mode}: ${JSON.stringify(jump)}`);
    const rapid = await page.evaluate(async () => {
      const v = window.testView;
      const read = v.doc.textWindow.bind(v.doc);
      v.doc.textWindow = async (...args) => {
        await new Promise(resolve => setTimeout(resolve, 60));
        return read(...args);
      };
      v.cache.clear();
      v.cacheChars = 0;
      v.scroll.scrollTop = v.span() * 0.15;
      v.onScroll();
      const pending = v.draw();
      await new Promise(resolve => setTimeout(resolve, 10));
      const target = v.span() * 0.65;
      const expected = v.lineAtY(target);
      v.scroll.scrollTop = target;
      v.reanchor();
      await pending;
      await new Promise(resolve => setTimeout(resolve, 150));
      return { expected, actual: v.viewLine };
    });
    assert(Math.abs(rapid.actual - rapid.expected) <= 1, `${mode} rapid: ${JSON.stringify(rapid)}`);
    await page.locator(".tv-scroll").focus();
    await page.keyboard.press("Control+End");
    await page.waitForFunction(() => window.testView.cursor === window.testView.doc.lengthBytes && !window.testView.drawing);
    const end = await page.evaluate(() => {
      const v = window.testView;
      const last = v.lines.at(-1);
      const caret = v.rows.querySelector(".tv-caret");
      return { eof: last.at + last.len, length: v.doc.lengthBytes, bottom: v.rows.lastElementChild.getBoundingClientRect().bottom,
        viewportBottom: v.scroll.getBoundingClientRect().bottom, caret: caret !== null };
    });
    assert.equal(end.eof, end.length);
    assert.equal(end.caret, true);
    assert(Math.abs(end.bottom - end.viewportBottom) <= 2, `${mode} EOF: ${JSON.stringify(end)}`);
    await page.keyboard.press("Control+Home");
    await page.waitForFunction(() => window.testView.cursor === 0 && !window.testView.drawing);
    await page.keyboard.press("Control+Shift+End");
    await page.waitForFunction(() => window.testView.selection?.end === window.testView.doc.lengthBytes && !window.testView.drawing);
    assert.equal(await page.evaluate(() => window.testView.selection.start), 0);
  }
  const huge = await page.evaluate(async () => {
    const { Doc } = await import("/src/doc.ts");
    const { TextView } = await import("/src/textview.ts");
    let read = 0;
    const size = 512 * 1024 ** 3;
    const source = {
      name: "virtual-large.log", size,
      slice(start = 0, end = size) {
        const n = Math.max(0, Math.min(size, end) - start);
        read += n;
        const bytes = new Uint8Array(n).fill(65);
        for (let i = (99 - start % 100 + 100) % 100; i < n; i += 100) bytes[i] = 10;
        return new Blob([bytes]);
      },
    };
    const v = new TextView(await Doc.open(source));
    document.body.replaceChildren(v.el);
    v.el.style.cssText = "height:600px;width:600px;flex:none";
    window.testView = v;
    await v.draw();
    await v.setWrap("word");
    await v.moveFileEdge(size);
    await new Promise(resolve => setTimeout(resolve, 300));
    const caretFound = v.rows.querySelector(".tv-caret") !== null && v.lines.at(-1).at + v.lines.at(-1).len === size;
    const before = v.heights.heightBefore(v.viewLine) + v.viewOffset;
    v.scrollWrapped(-123);
    const distance = before - v.heights.heightBefore(v.viewLine) - v.viewOffset;
    return { read, caretFound, distance, count: v.rows.children.length, total: v.index.totalLines };
  });
  assert(huge.read < 16 * 1024 ** 2, JSON.stringify(huge));
  assert(huge.caretFound, JSON.stringify(huge));
  assert(Math.abs(huge.distance - 123) < 1, JSON.stringify(huge));
  for (const mode of ["off", "word", "line"]) {
    for (const suffix of ["", "\r\n"]) {
      await page.evaluate(async ({ mode, suffix }) => {
        const { Doc } = await import("/src/doc.ts");
        const { TextView } = await import("/src/textview.ts");
        const v = new TextView(await Doc.open(new File(["short\n".repeat(2000) + "long tail ".repeat(2000) + suffix], "end.txt")));
        document.body.replaceChildren(v.el);
        v.el.style.cssText = "height:600px;width:600px;flex:none";
        window.testView = v;
        await v.draw();
        await v.setWrap(mode);
      }, { mode, suffix });
      await page.locator(".tv-scroll").focus();
      await page.keyboard.press("Control+End");
      await page.waitForFunction(() => window.testView.cursor === window.testView.doc.lengthBytes && !window.testView.drawing);
      const tail = await page.evaluate(() => {
        const v = window.testView;
        const caret = v.rows.querySelector(".tv-caret").getBoundingClientRect();
        const port = v.scroll.getBoundingClientRect();
        return { visible: caret.top >= port.top && caret.bottom <= port.bottom + 1, blank: v.lines.at(-1).len === 0 };
      });
      assert(tail.visible, `${mode} ${JSON.stringify(suffix)}: ${JSON.stringify(tail)}`);
      assert.equal(tail.blank, suffix !== "");
      await page.keyboard.press("Control+Home");
      await page.waitForFunction(() => window.testView.cursor === 0 && !window.testView.drawing);
      assert.equal(await page.evaluate(() => window.testView.scroll.scrollTop), 0);
    }
  }
  assert.deepEqual(errors, []);
  console.log("Text wrapping browser checks passed", result, huge);
} finally {
  await browser.close();
}
