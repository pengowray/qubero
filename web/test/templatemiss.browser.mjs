// The Template tab's message when a clicked line has no field in the file.
//
// Run against Vite with TEST_URL and, optionally, a bundled PLAYWRIGHT_MODULE:
//
//   PLAYWRIGHT_MODULE="file:///.../playwright/index.mjs" \
//   TEST_URL="http://localhost:17281" node test/templatemiss.browser.mjs
//
// The message goes in the sidebar's bar beside the Copy button, and the line
// that was clicked gets a grey fill. The template text must not move: not
// sideways, not down, and nothing in the bar moves either. A copy and a miss
// share the one message, the newer replacing the older.
import assert from "node:assert/strict";
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || "playwright");
const url = process.env.TEST_URL || "http://localhost:17272";
const browser = await chromium.launch({ headless: true });
try {
  const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 } });
  await ctx.grantPermissions(["clipboard-read", "clipboard-write"], { origin: new URL(url).origin });
  const page = await ctx.newPage();
  await page.goto(url);
  await page.evaluate(async () => {
    const { Doc } = await import("/src/doc.ts");
    const { TemplatePanel } = await import("/src/templatepanel.ts");
    const chunk = (kind, data) => {
      const b = new Uint8Array(data.length + 12);
      new DataView(b.buffer).setUint32(0, data.length);
      b.set(new TextEncoder().encode(kind), 4);
      b.set(data, 8);
      return b;
    };
    const header = new Uint8Array(13);
    new DataView(header.buffer).setUint32(0, 4);
    new DataView(header.buffer).setUint32(4, 4);
    header[8] = 8;
    header[9] = 2;
    // No tEXt chunk, so a field of the tEXt arm is not in this file.
    const doc = await Doc.open(
      new File([Uint8Array.of(137, 80, 78, 71, 13, 10, 26, 10), chunk("IHDR", header), chunk("IDAT", new Uint8Array(16)), chunk("IEND", new Uint8Array())], "plain.png"),
    );
    await doc.ensureRange(0, doc.lengthBytes);
    doc.setTemplate("png");
    const panel = new TemplatePanel(doc);
    const side = document.createElement("section");
    side.className = "panel panel-right";
    side.style.cssText = "width:308px;height:600px;display:flex;flex-direction:column;position:fixed;right:0;top:0";
    const bar = document.createElement("header");
    bar.className = "panel-bar";
    const tabs = document.createElement("span");
    tabs.className = "panel-tabs";
    tabs.textContent = "At cursor Template";
    const tools = document.createElement("span");
    tools.className = "panel-tools";
    tools.append(panel.tools);
    bar.append(tabs, tools);
    side.append(bar, panel.el);
    document.body.replaceChildren(side);
    panel.setVisible(true);
    panel.sniffed();
    window.__panel = panel;
  });
  await page.waitForSelector(".tp-line");
  // Lines are drawn in chunks at idle time; wait for all of them, so a chunk
  // drawn later does not count as something that moved.
  await page.waitForFunction(() => [...document.querySelectorAll(".tp-chunk")].every((c) => c.childElementCount > 0));
  const line = await page.evaluate(() => {
    const rows = [...document.querySelectorAll(".tp-line")];
    const arm = rows.findIndex((r) => /'tEXt'\s*=>/.test(r.textContent));
    return rows.slice(arm).find((r) => r.classList.contains("is-field") && /keyword/.test(r.textContent))?.dataset.line;
  });
  assert.ok(line !== undefined, "the tEXt arm has a keyword line");
  const layout = () =>
    page.evaluate(() => {
      const s = document.querySelector(".tp-scroll");
      const rects = [...document.querySelectorAll(".panel-tabs, .tp-copy, .tp-line")].map((e) => JSON.stringify(e.getBoundingClientRect()));
      return { left: s.scrollLeft, top: s.scrollTop, rects };
    });
  const before = await layout();
  await page.locator(`.tp-line[data-line="${line}"]`).click({ position: { x: 40, y: 9 } });
  await page.waitForSelector(".tp-copied:not([hidden])");
  const after = await layout();
  assert.equal(after.left, before.left, "the text is not scrolled sideways");
  assert.equal(after.top, before.top, "the text is not scrolled down");
  assert.deepEqual(after.rects, before.rects, "nothing moves when the message appears");
  const shown = await page.evaluate(() => {
    const n = document.querySelector(".tp-copied");
    return { text: n.textContent, missed: document.querySelector(".tp-line.is-missed")?.dataset.line, inline: document.querySelector(".tp-miss") !== null, fits: n.scrollWidth <= n.clientWidth && n.scrollHeight <= n.clientHeight };
  });
  assert.match(shown.text, /^Not in this file: type is never 'tEXt'/);
  assert.equal(shown.missed, line, "the clicked line is marked");
  assert.equal(shown.inline, false, "no note is put on the line itself");
  assert.ok(shown.fits, "the message fits in the bar");

  // A copy replaces the miss message and clears the mark on the line.
  await page.locator(".tp-copy").click();
  await page.waitForFunction(() => document.querySelector(".tp-copied").textContent.startsWith("Copied"));
  assert.equal(await page.evaluate(() => document.querySelector(".tp-line.is-missed")), null);
  assert.deepEqual((await layout()).rects, before.rects, "nothing moves when the copy message appears");

  // A miss replaces the copy message.
  await page.locator(`.tp-line[data-line="${line}"]`).click({ position: { x: 40, y: 9 } });
  await page.waitForFunction(() => document.querySelector(".tp-copied").textContent.startsWith("Not in this file"));

  // Moving the cursor takes the miss message and the mark away together.
  await page.evaluate(() => window.__panel.setOffset(0));
  const gone = await page.evaluate(() => ({ hidden: document.querySelector(".tp-copied").hidden, missed: document.querySelector(".tp-line.is-missed") }));
  assert.equal(gone.hidden, true);
  assert.equal(gone.missed, null);
  assert.deepEqual((await layout()).rects, before.rects, "nothing moves when the message goes");
  console.log("templatemiss: ok");
} finally {
  await browser.close();
}
