import assert from "node:assert/strict";
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || "playwright");
const browser = await chromium.launch({ headless: true });
try {
  const page = await browser.newPage({ viewport: { width: 1100, height: 800 }, colorScheme: "light" });
  const errors = [];
  page.on("pageerror", e => errors.push(e.message));
  await page.goto(process.env.TEST_URL || "http://127.0.0.1:17272");
  await page.evaluate(() => document.fonts.ready);
  const logo = page.getByRole("button", { name: "Spin the Qubero logo" });
  const rest = await logo.locator("svg").innerHTML();
  if (process.env.SCREENSHOT_DIR) await page.screenshot({ path: `${process.env.SCREENSHOT_DIR}/qubero-welcome.png` });
  await logo.click();
  await page.waitForTimeout(700);
  assert.notEqual(await logo.locator("svg").innerHTML(), rest);
  if (process.env.SCREENSHOT_DIR) await page.screenshot({ path: `${process.env.SCREENSHOT_DIR}/qubero-turn.png` });
  await page.waitForFunction(() => !document.querySelector(".welcome-crystal").dataset.spinning);
  assert.equal(await logo.locator("svg").innerHTML(), rest);

  // Drag the logo round, and see it come back. A press turns it directly, a
  // release springs it home, and the spring overshoots on the way rather than
  // sliding back: the turn passes rest and keeps going before it settles.
  const box = await logo.boundingBox();
  const mid = { x: box.x + box.width / 2, y: box.y + box.height / 2 };
  await page.mouse.move(mid.x, mid.y);
  await page.mouse.down();
  for (let i = 1; i <= 8; i++) { await page.mouse.move(mid.x + i * 5, mid.y); await page.waitForTimeout(12); }
  // Held still before letting go, so the return is the spring's own and not a
  // flick's, and the overshoot below is the same size every run.
  await page.waitForTimeout(300);
  assert.notEqual(await logo.locator("svg").innerHTML(), rest, "a drag turns the logo");
  assert.equal(await page.locator(".welcome-crystal").getAttribute("data-spinning"), "true");
  // The asymmetric prism can widen or narrow as different facets turn into
  // view. Record the rendered angle to check spring overshoot independently
  // of that changing silhouette; the visible drawing is checked above/below.
  await page.evaluate(async () => {
    const moduleUrl = performance.getEntriesByType("resource").find(entry =>
      new URL(entry.name).pathname === "/src/crystal.ts").name;
    const { Crystal } = await import(moduleUrl);
    const draw = Crystal.prototype.draw;
    window.crystalAngles = [];
    Crystal.prototype.draw = function (angle) {
      window.crystalAngles.push(angle);
      return draw.call(this, angle);
    };
    window.restoreCrystalDraw = () => { Crystal.prototype.draw = draw; };
  });
  await page.mouse.up();
  await page.waitForTimeout(1000);
  const trace = await page.evaluate(() => {
    window.restoreCrystalDraw();
    return window.crystalAngles;
  });
  assert(trace[0] > 0.2, "the spring starts from the held turn");
  const pastHome = trace.findIndex(angle => angle < -0.1);
  assert(pastHome > 0, "the spring carries it past home");
  assert(trace.slice(pastHome).some(angle => angle > 0.05), "and it comes back again");
  await page.waitForFunction(() => !document.querySelector(".welcome-crystal").dataset.spinning, null, { timeout: 4000 });
  assert.equal(await logo.locator("svg").innerHTML(), rest, "the spring puts it back at rest");

  // The keyboard still spins it after a drag. The mark remembers how far the
  // last press wandered so that a drag does not also fire the click spin, and
  // a memory never cleared would swallow every Enter that followed.
  await logo.focus();
  await page.keyboard.press("Enter");
  await page.waitForTimeout(300);
  assert.notEqual(await logo.locator("svg").innerHTML(), rest, "Enter spins it after a drag");
  await page.waitForFunction(() => !document.querySelector(".welcome-crystal").dataset.spinning, null, { timeout: 4000 });

  // A press that turned the mark must not also fire the click spin, which
  // would start a revolution on top of the spring's return.
  await page.mouse.move(mid.x, mid.y);
  await page.mouse.down();
  await page.mouse.move(mid.x + 40, mid.y);
  await page.mouse.up();
  await page.waitForFunction(() => !document.querySelector(".welcome-crystal").dataset.spinning, null, { timeout: 4000 });
  assert.equal(await logo.locator("svg").innerHTML(), rest, "a drag does not leave a spin running");
  await page.emulateMedia({ colorScheme: "dark" });
  if (process.env.SCREENSHOT_DIR) await page.screenshot({ path: `${process.env.SCREENSHOT_DIR}/qubero-dark.png` });
  await page.setViewportSize({ width: 375, height: 750 });
  assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
  if (process.env.SCREENSHOT_DIR) await page.screenshot({ path: `${process.env.SCREENSHOT_DIR}/qubero-mobile.png` });
  await page.evaluate(async () => {
    const { Doc } = await import("/src/doc.ts");
    const open = Doc.open;
    Doc.open = async (...args) => {
      await new Promise(resolve => setTimeout(resolve, 700));
      return open(...args);
    };
  });
  const chooser = page.waitForEvent("filechooser");
  await page.getByRole("button", { name: "Open a file", exact: true }).click();
  await (await chooser).setFiles({ name: "hello.txt", mimeType: "text/plain", buffer: Buffer.from("Hello Qubero\n") });
  // The mark turns while the file opens, and the status line beside it names
  // the file. Both are true at the same moment and the opening screen is taken
  // down as soon as the file is in, so they are asked for together: waiting on
  // one and then the other spends the window between them and arrives to find
  // the screen gone.
  await page.waitForFunction(() => {
    const mark = document.querySelector(".welcome-crystal");
    const status = document.querySelector("[role=status]");
    return mark?.dataset.spinning === "true" && (status?.textContent ?? "").includes("Opening hello.txt");
  });
  await page.locator(".welcome").waitFor({ state: "detached" });
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.goto(process.env.TEST_URL || "http://127.0.0.1:17272");
  // Under reduced motion the mark is not a control at all: out of the tab
  // order, out of the accessibility tree, and no tooltip promising a turn.
  assert.equal(await page.locator(".welcome-crystal").getAttribute("aria-hidden"), "true");
  assert.equal(await page.locator(".welcome-crystal").getAttribute("tabindex"), "-1");
  assert.equal(await page.locator(".welcome-crystal").getAttribute("title"), null);
  const still = await page.locator(".welcome-crystal").locator("svg").innerHTML();
  await page.locator(".welcome-crystal").click();
  assert.equal(await page.locator(".welcome-crystal").getAttribute("data-spinning"), null);
  const stillBox = await page.locator(".welcome-crystal").boundingBox();
  await page.mouse.move(stillBox.x + stillBox.width / 2, stillBox.y + stillBox.height / 2);
  await page.mouse.down();
  await page.mouse.move(stillBox.x + stillBox.width, stillBox.y + stillBox.height / 2);
  await page.mouse.up();
  assert.equal(await page.locator(".welcome-crystal").locator("svg").innerHTML(), still, "reduced motion means it does not turn");
  assert.deepEqual(errors, []);
  console.log("Opening-screen checks passed: spin, drag and spring back, rest pose, themes, mobile, file loading, reduced motion.");
} finally {
  await browser.close();
}
