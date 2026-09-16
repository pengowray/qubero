// Run against Vite with a freshly rebuilt WASM core. PLAYWRIGHT_MODULE may
// point to an existing Playwright installation; TEST_URL selects the server.
//
// Two templates read a pickle, and the file decides which one it opens with: a
// file a Familiar Pickle Form matches whole opens as the object it builds, and
// everything else opens as the program that builds it. Each has to be reachable
// from the other, so the chooser is opened and the templates swapped here
// rather than being asked for by name.
import assert from "node:assert/strict";
import { mkdir } from "node:fs/promises";
import { readFileSync } from "node:fs";

const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || "playwright");
const message = "Matched a Familiar Pickle Form";
const form = "numpy-numeric-array-p4-p5-v2";
const stop = (id) =>
  `Matched a Familiar Pickle Form (${id}): bypassed Pickle stack machine decoding. Switch to the "Python pickle (familiar form)" template to see the data.`;
const out = new URL("out/", import.meta.url);
const shot = (page, name) => page.screenshot({ path: new URL(name, out).pathname.replace(/^\/(?=[A-Za-z]:)/, "") });

/** The one sample a form matches whole. The copy committed as a fixture is the
 *  same bytes, for a checkout with no sample collection beside it. */
const sample = () => {
  const named = process.env.QUBERO_SAMPLES === undefined ? null : new URL(`file://${process.env.QUBERO_SAMPLES}/pickle/proto4-numpy-array.pickle`);
  const beside = new URL("../../../qubero-samples/pickle/proto4-numpy-array.pickle", import.meta.url);
  for (const at of [named, beside]) {
    if (at === null) continue;
    try {
      return readFileSync(at);
    } catch {
      continue;
    }
  }
  return readFileSync(new URL("../../crates/core/tests/fixtures/pickle/numpy-f32-matrix.pickle", import.meta.url));
};

const open = async (browser, name, bytes) => {
  const page = await browser.newPage({ viewport: { width: 1400, height: 900 } });
  const errors = [];
  page.on("pageerror", error => errors.push(error.message));
  await page.goto(process.env.TEST_URL || "http://localhost:17326");
  const chooser = page.waitForEvent("filechooser");
  await page.getByRole("button", { name: "Open a file", exact: true }).click();
  await (await chooser).setFiles({ name, buffer: Buffer.from(bytes) });
  await page.waitForSelector(".rp-item", { state: "attached", timeout: 20000 });
  return { page, errors };
};

/** Open the chooser and search it, the way a reader picks a template. */
const search = async (page, query) => {
  await page.locator(".tb-tmpl").click();
  const box = page.locator("dialog[open] .set-search");
  await box.waitFor({ state: "visible", timeout: 10000 });
  await box.fill(query);
  // The results are redrawn after the keystroke, so wait for the list the
  // query asked for rather than for the one that was already there.
  await page.waitForFunction(() => document.querySelectorAll(".set-row[data-template]").length === 2, null, { timeout: 10000 });
};

/** Swap the template through that chooser. */
const choose = async (page, template) => {
  await search(page, "pickle");
  await page.locator(`.set-row[data-template="${template}"]`).click();
  await page.keyboard.press("Escape");
  await page.waitForSelector(".rp-item", { state: "attached", timeout: 20000 });
};

const browser = await chromium.launch({ headless: true });
try {
  // A matched NumPy array: the form that matched, and the numbers it holds.
  {
    const { page, errors } = await open(browser, "proto4-numpy-array.pickle", sample());
    await page.getByRole("button", { name: "Listing", exact: true }).click();
    await page.getByText(message, { exact: false }).first().waitFor({ state: "visible", timeout: 10000 });
    await page.getByText(form, { exact: true }).first().waitFor({ state: "visible" });
    const said = await page.locator("body").innerText();
    assert.match(said, /Template: Python pickle \(familiar form\)/, "the chooser did not pick the familiar form");
    // The decoded data: the dictionary key, what the array is, and its numbers.
    for (const shown of ["weights", "<f4", "4 x 6", "24 values"]) {
      assert.ok(said.includes(shown), `the listing does not show ${shown}`);
    }
    await mkdir(out, { recursive: true });
    await shot(page, "pickle-familiar.png");

    // Both templates are offered, and only those two.
    await search(page, "pickle");
    const offered = await page.locator(".set-row[data-template]").evaluateAll(rows => rows.map(r => r.dataset.template));
    assert.deepEqual([...offered].sort(), ["pickle", "picklefpf"], "the chooser offered other templates");
    await page.keyboard.press("Escape");

    // The program is one template away, and reads as the program.
    await choose(page, "pickle");
    await page.getByText("PROTO", { exact: false }).first().waitFor({ timeout: 10000 });
    assert.match(await page.locator("body").innerText(), /Template: Python pickle$/m);
    await shot(page, "pickle-listing.png");
    assert.deepEqual(errors, []);
    await page.close();
  }

  // A matched file small enough for the whole listing to be on screen, which
  // is where the STOP row's message can be read.
  {
    const { page, errors } = await open(browser, "familiar.pickle", [0x80, 4, 0x4e, 0x2e]);
    await page.getByRole("button", { name: "Listing", exact: true }).click();
    await page.getByText("basic-p4-p5-v2", { exact: true }).first().waitFor({ state: "visible", timeout: 10000 });
    await choose(page, "pickle");
    await page.getByText("STOP", { exact: true }).first().waitFor({ timeout: 10000 });
    await page.getByText(stop("basic-p4-p5-v2"), { exact: true }).first().waitFor({ state: "visible" });
    assert.deepEqual(errors, []);
    await page.close();
  }

  // An unfamiliar program: the listing, with no claim that anything matched.
  {
    const { page, errors } = await open(browser, "unfamiliar.pickle", [0x80, 4, 0x4e, 0x30, 0x4e, 0x2e]);
    await page.getByText("STOP", { exact: true }).first().waitFor({ timeout: 10000 });
    assert.equal(await page.getByText(message, { exact: false }).count(), 0);
    await page.getByText("POP", { exact: true }).first().waitFor();
    assert.match(await page.locator("body").innerText(), /Template: Python pickle$/m, "an unmatched pickle did not open as the program");
    assert.deepEqual(errors, []);
    await page.close();
  }
  console.log("FPF template, chooser, decoded values and PVM fallback passed in browser.");
} finally {
  await browser.close();
}
