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

import { sampleFile } from "./samples.mjs";

const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || "playwright");
const message = "Matched a Familiar Pickle Form";
const form = "numpy-array-p4-p5-v6";
const stop = (id) =>
  `Matched a Familiar Pickle Form (${id}): bypassed Pickle stack machine decoding. Switch to the "Python pickle (familiar form)" template to see the data.`;
const out = new URL("out/", import.meta.url);
const shot = (page, name) => page.screenshot({ path: new URL(name, out).pathname.replace(/^\/(?=[A-Za-z]:)/, "") });

/** The one sample a form matches whole. The copy committed as a fixture is the
 *  same bytes, for a checkout with no sample collection beside it. */
const sample = () => {
  const collected = sampleFile("pickle", "proto4-numpy-array.pickle");
  if (collected !== null) {
    return readFileSync(collected);
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
    // The decoded data, and the instructions the form fixed around it: the
    // dictionary key, what the array is, the call that rebuilt it, the names
    // that call was made with, and which of CPython's two picklers the file
    // shows, which for this one is neither in particular.
    for (const shown of ["weights", "<f4", "4 x 6", "ndarray reconstruct call", "numpy._core.multiarray",
                         "any (every known pickler writes this data the same way)"]) {
      assert.ok(said.includes(shown), `the listing does not show ${shown}`);
    }
    await mkdir(out, { recursive: true });
    await shot(page, "pickle-familiar.png");

    // The opcode bytes written between the values are not rows until the
    // reader asks for them, and the switch at the top of the listing is the
    // asking. The fields are read off the rows themselves: "memoize" is a word
    // the inspector and the hex view can be saying at the same time.
    const fields = () => page.locator(".rp-row .rp-field").allInnerTexts();
    assert.equal((await fields()).includes("memoize"), false, "an opcode row is drawn before it was asked for");
    const opcodes = page.getByRole("checkbox", { name: "Show opcode rows" });
    await opcodes.check();
    await page.waitForFunction(() => [...document.querySelectorAll(".rp-row .rp-field")].some(f => f.textContent === "memoize"), null, { timeout: 10000 });
    await shot(page, "pickle-familiar-opcodes.png");
    await opcodes.uncheck();
    assert.equal((await fields()).includes("memoize"), false, "the opcode rows stayed after they were put away");

    // The numbers themselves, which the hex view shows against their bytes.
    await page.getByRole("button", { name: "Hex", exact: true }).click();
    await page.getByText("24 values", { exact: false }).first().waitFor({ state: "visible", timeout: 10000 });
    await shot(page, "pickle-familiar-hex.png");
    await page.getByRole("button", { name: "Listing", exact: true }).click();

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
    await page.getByText("basic-p4-p5-v5", { exact: true }).first().waitFor({ state: "visible", timeout: 10000 });
    await choose(page, "pickle");
    await page.getByText("STOP", { exact: true }).first().waitFor({ timeout: 10000 });
    await page.getByText(stop("basic-p4-p5-v5"), { exact: true }).first().waitFor({ state: "visible" });
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
