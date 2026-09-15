// Run against Vite with a freshly rebuilt WASM core. PLAYWRIGHT_MODULE may
// point to an existing Playwright installation; TEST_URL selects the server.
import assert from "node:assert/strict";
import { mkdir } from "node:fs/promises";

const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || "playwright");
const message = "Matched a Familiar Pickle Form: bypassed Pickle stack machine decoding.";
const browser = await chromium.launch({ channel: "msedge", headless: true });
try {
  for (const [name, bytes, matched] of [
    ["familiar.pickle", [0x80, 4, 0x4e, 0x2e], true],
    ["unfamiliar.pickle", [0x80, 4, 0x4e, 0x30, 0x4e, 0x2e], false],
  ]) {
    const page = await browser.newPage({ viewport: { width: 1400, height: 900 } });
    const errors = [];
    page.on("pageerror", error => errors.push(error.message));
    await page.goto(process.env.TEST_URL || "http://localhost:17326");
    const chooser = page.waitForEvent("filechooser");
    await page.getByRole("button", { name: "Open a file", exact: true }).click();
    await (await chooser).setFiles({ name, buffer: Buffer.from(bytes) });
    await page.waitForSelector(".rp-row", { timeout: 20000 });
    // A complete STOP row proves the listing finished before checking absence.
    await page.getByText("STOP", { exact: true }).first().waitFor({ timeout: 10000 });
    if (matched) {
      await page.getByText(message, { exact: true }).first().waitFor({ state: "visible" });
      const out = new URL("out/", import.meta.url);
      await mkdir(out, { recursive: true });
      await page.screenshot({ path: new URL("pickle-familiar.png", out).pathname.replace(/^\/(?=[A-Za-z]:)/, "") });
    } else {
      assert.equal(await page.getByText(message, { exact: true }).count(), 0);
      await page.getByText("POP", { exact: true }).first().waitFor();
    }
    assert.deepEqual(errors, []);
    await page.close();
  }
  console.log("FPF match message and unfamiliar PVM fallback passed in browser.");
} finally {
  await browser.close();
}
