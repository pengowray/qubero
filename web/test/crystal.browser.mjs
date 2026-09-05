import assert from "node:assert/strict";
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || "playwright");
const browser = await chromium.launch({ channel: "msedge", headless: true });
try {
  const page = await browser.newPage({ viewport: { width: 1100, height: 800 }, colorScheme: "light" });
  const errors = [];
  page.on("pageerror", e => errors.push(e.message));
  await page.goto(process.env.TEST_URL || "http://127.0.0.1:17272");
  await page.evaluate(() => document.fonts.ready);
  const logo = page.getByRole("button", { name: "Spin the Qubero crystal" });
  const rest = await logo.locator("svg").innerHTML();
  if (process.env.SCREENSHOT_DIR) await page.screenshot({ path: `${process.env.SCREENSHOT_DIR}/qubero-welcome.png` });
  await logo.click();
  await page.waitForTimeout(700);
  assert.notEqual(await logo.locator("svg").innerHTML(), rest);
  if (process.env.SCREENSHOT_DIR) await page.screenshot({ path: `${process.env.SCREENSHOT_DIR}/qubero-turn.png` });
  await page.waitForFunction(() => !document.querySelector(".welcome-crystal").dataset.spinning);
  assert.equal(await logo.locator("svg").innerHTML(), rest);
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
  await page.getByRole("status").filter({ hasText: "Opening hello.txt" }).waitFor();
  assert.equal(await page.locator(".welcome-crystal").getAttribute("data-spinning"), "true");
  await page.locator(".welcome").waitFor({ state: "detached" });
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.goto(process.env.TEST_URL || "http://127.0.0.1:17272");
  await page.getByRole("button", { name: "Spin the Qubero crystal" }).click();
  assert.equal(await page.locator(".welcome-crystal").getAttribute("data-spinning"), null);
  assert.deepEqual(errors, []);
  console.log("Opening-screen checks passed: spin, rest pose, themes, mobile, file loading.");
} finally {
  await browser.close();
}
