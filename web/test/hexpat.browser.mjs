// The ImHex pattern converter panel, driven the way a reader drives it.
//
// Run against a Vite dev server. PLAYWRIGHT_MODULE can point to a bundled
// Playwright installation; TEST_URL must say localhost, not 127.0.0.1, since
// vite binds IPv6 and refuses the IPv4 address.
//
//   cd web
//   npx vite --port 17282
//   PLAYWRIGHT_MODULE="file:///home/pengo/.nvm/versions/node/v24.21.0/lib/node_modules/playwright/index.mjs" \
//   TEST_URL="http://localhost:17282" node test/hexpat.browser.mjs
//
// The library half never reaches the network. Every request to the raw GitHub
// URL is intercepted and answered from the four patterns bundled in this
// repository, and the test counts the requests to prove that none happens
// before the button in the licence notice is pressed.
//
// SAMPLES points at the sample collection, OUT_DIR at where the screenshots go.
import assert from "node:assert/strict";
import { mkdir, readFile } from "node:fs/promises";
import { basename, join } from "node:path";
import { fileURLToPath } from "node:url";

const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || "playwright");
const samples = process.env.SAMPLES || "/home/pengo/sync/dropbox/projects/qubero-samples";
const outDir = process.env.OUT_DIR || fileURLToPath(new URL("out", import.meta.url));
const bundled = fileURLToPath(new URL("../../crates/core/formats-hexpat/", import.meta.url));
await mkdir(outDir, { recursive: true });

// Big enough for a 512-byte boot record to read, and a format the app knows,
// so the toolbar and the listing are in their ordinary state before the
// converter is opened.
const sample = join(samples, "pico8/0-saka.p8.png");

// A pattern small enough to read in the test itself. Every PNG starts with the
// eight signature bytes and then a big-endian length, so the two fields have
// values a reader could check against the hex view.
const small = `#pragma endian big
struct Signature {
    u8 magic[8];
    u32 ihdrLength;
    char ihdrType[4];
};

Signature signature @ 0x00;
`;

const browser = await chromium.launch({ headless: true });
try {
  const page = await browser.newPage({ viewport: { width: 1400, height: 900 } });
  const errors = [];
  page.on("pageerror", (e) => errors.push(e.message));

  // Every fetch of a pattern is answered from this repository's own copies.
  // `fetched` is the proof that nothing is fetched without a click.
  const fetched = [];
  await page.route("https://raw.githubusercontent.com/WerWolv/ImHex-Patterns/master/**", async (route) => {
    const path = new URL(route.request().url()).pathname.replace("/WerWolv/ImHex-Patterns/master/patterns/", "");
    fetched.push(path);
    try {
      const body = await readFile(join(bundled, path), "utf8");
      await route.fulfill({ status: 200, contentType: "text/plain", body });
    } catch {
      await route.fulfill({ status: 404, contentType: "text/plain", body: "not in the fixture" });
    }
  });

  await page.goto(process.env.TEST_URL || "http://localhost:17282");

  const chooser = page.waitForEvent("filechooser");
  await page.getByRole("button", { name: "Open a file", exact: true }).click();
  await (await chooser).setFiles({ name: basename(sample), buffer: await readFile(sample) });
  await page.waitForSelector(".rp-row, .hv-hex", { timeout: 20000 });

  // Opened the way the settings dialog offers it, rather than by calling into
  // the page: the button under the template list is half of what is tested.
  const openConverter = async () => {
    await page.click(".tb-tmpl");
    await page.getByRole("button", { name: "Convert an ImHex pattern…", exact: true }).click();
  };
  await openConverter();
  await page.waitForSelector(".kp-hexpat:not([hidden])", { timeout: 10000 });

  const paste = async (text) => {
    await page.locator(".kp-hexpat .kp-source").fill(text);
    await page.waitForTimeout(600);
  };

  await paste(small);
  const pasted = await page.evaluate(() => ({
    out: document.querySelector(".kp-hexpat .kp-out").textContent,
    failed: document.querySelectorAll(".kp-hexpat .kp-error").length,
    gaps: document.querySelector(".kp-hexpat .kp-gaps > .kp-group-heading")?.textContent ?? "",
    fields: document.querySelector(".kp-hexpat .kp-fields > .kp-group-heading")?.textContent ?? "",
    lines: document.querySelectorAll(".kp-hexpat .kp-line").length,
    applyDisabled: document.querySelector(".kp-hexpat .kp-foot button").disabled,
    needs: document.querySelectorAll(".kp-hexpat .kp-need").length,
  }));
  console.log("pasted pattern", JSON.stringify(pasted).slice(0, 400));
  assert.equal(pasted.failed, 0, "the pasted pattern did not convert");
  assert.equal(pasted.applyDisabled, false, "a converted pattern left the apply button disabled");
  assert.match(pasted.out, /ihdrLength/, "the template column does not name the pattern's fields");
  assert(pasted.lines > 0, "the report listed nothing at all");
  assert.equal(pasted.needs, 0, "a pattern that includes nothing asked for a file");
  assert.equal(fetched.length, 0, "something reached the network before the library was opened");

  // A report line is `line:column` in the pattern, and clicking it goes there.
  await page.locator(".kp-hexpat .kp-fields > .kp-group-heading").click();
  await page.locator(".kp-hexpat .kp-fields .kp-line").first().click();
  const caret = await page.evaluate(() => {
    const t = document.querySelector(".kp-hexpat .kp-source");
    return { start: t.selectionStart, line: t.value.slice(0, t.selectionStart).split("\n").length };
  });
  console.log("caret after clicking the first field line", JSON.stringify(caret));
  assert(caret.start > 0, "clicking a report line did not move the caret");
  await page.screenshot({ path: join(outDir, "hexpat.png") });

  // A pattern that does not convert: one line in place of the report, the line
  // and column first, and nothing to apply.
  await paste("struct S {\n    u8 a\n};\nS s @ 0x00;\n");
  const broken = await page.evaluate(() => ({
    path: document.querySelector(".kp-hexpat .kp-error .kp-line-path")?.textContent ?? "",
    message: document.querySelector(".kp-hexpat .kp-error .kp-line-message")?.textContent ?? "",
    groups: document.querySelectorAll(".kp-hexpat .kp-group").length,
    out: document.querySelector(".kp-hexpat .kp-out").textContent,
    applyDisabled: document.querySelector(".kp-hexpat .kp-foot button").disabled,
  }));
  console.log("broken pattern", JSON.stringify(broken));
  assert.match(broken.path, /^\d+:\d+$/, "the error line does not lead with the line and column");
  assert(broken.message.length > 0, "the error line says nothing");
  assert.equal(broken.groups, 0, "the report stayed up beside the error");
  assert.equal(broken.applyDisabled, true, "a pattern that did not convert could still be applied");
  assert.match(broken.out, /did not convert/, "the template column does not say why it is empty");

  // A pattern that includes a file Qubero has not got. The ImHex include tree
  // is GPL-2.0 and is not shipped, so the panel names the file and takes it
  // from the reader.
  await paste("#include <mystuff/thing.pat>\n\nThing thing @ 0x00;\n");
  const wants = await page.evaluate(() => ({
    heading: document.querySelector(".kp-hexpat .kp-needs-heading")?.textContent ?? "",
    paths: [...document.querySelectorAll(".kp-hexpat .kp-need-path")].map((e) => e.textContent),
    failed: document.querySelectorAll(".kp-hexpat .kp-error").length,
    applyDisabled: document.querySelector(".kp-hexpat .kp-foot button").disabled,
  }));
  console.log("wants an include", JSON.stringify(wants));
  assert.equal(wants.heading, "Needs 1 more file", "the panel does not say a file is missing");
  assert.deepEqual(wants.paths, ["mystuff/thing.pat"], "the panel does not name the file it needs");
  assert.equal(wants.failed, 1, "a pattern with no include converted anyway");
  assert.equal(wants.applyDisabled, true, "a pattern missing an include could still be applied");
  // The panel asking for a file, which is where a reader meets the box.
  await page.screenshot({ path: join(outDir, "hexpat-includes.png") });

  await page.locator(".kp-hexpat .kp-need-text").fill("#pragma once\nstruct Thing { u32 count; };\n");
  await page.waitForTimeout(700);
  const supplied = await page.evaluate(() => ({
    failed: document.querySelectorAll(".kp-hexpat .kp-error").length,
    needs: document.querySelectorAll(".kp-hexpat .kp-need").length,
    out: document.querySelector(".kp-hexpat .kp-out").textContent,
    applyDisabled: document.querySelector(".kp-hexpat .kp-foot button").disabled,
  }));
  console.log("include supplied", JSON.stringify({ ...supplied, out: supplied.out.slice(0, 80) }));
  assert.equal(supplied.failed, 0, "the pattern did not convert with the include pasted in");
  assert.equal(supplied.needs, 0, "the panel still asks for a file it has been given");
  assert.match(supplied.out, /count/, "the template has nothing from the included file");
  assert.equal(supplied.applyDisabled, false, "the pattern converted but could not be applied");

  // Apply the small pattern, and see its fields in the listing.
  await paste(small);
  await page.getByRole("button", { name: "Use this template", exact: true }).click();
  await page.waitForSelector(".rp-row", { timeout: 20000 });
  const applied = await page.evaluate(() => ({
    menu: document.querySelector(".tb-tmpl")?.textContent ?? "",
    view: document.querySelector(".tb-view.is-on")?.textContent ?? "",
    names: [...document.querySelectorAll(".rp-row .rp-field, .rp-row .rp-name")].slice(0, 20).map((e) => e.textContent.trim()),
    panelHidden: document.querySelector(".kp-hexpat").hidden,
  }));
  console.log("applied", JSON.stringify(applied));
  assert.equal(applied.view, "Listing", "applying did not go back to the listing");
  assert.equal(applied.panelHidden, true, "the converter stayed open after applying");
  assert.equal(applied.menu, "Template: pattern (from .hexpat)", "the menu does not name the converted pattern");
  for (const name of ["magic", "ihdrLength", "ihdrType"]) {
    assert(applied.names.includes(name), `the listing has no ${name} row: ${applied.names.join(" ")}`);
  }

  // ---- the library ----

  await openConverter();
  await page.waitForSelector(".kp-hexpat:not([hidden])", { timeout: 10000 });
  await page.getByRole("button", { name: "ImHex library…", exact: true }).click();
  await page.waitForSelector(".kp-lib[open]", { timeout: 10000 });
  await page.waitForFunction(() => document.querySelectorAll(".kp-lib-row").length > 0, { timeout: 10000 });
  const list = await page.evaluate(() => ({
    count: document.querySelector(".kp-lib-count").textContent,
    rows: document.querySelectorAll(".kp-lib-row").length,
    first: document.querySelector(".kp-lib-row .kp-lib-name")?.textContent ?? "",
    detail: document.querySelector(".kp-lib-detail").textContent,
  }));
  console.log("library", JSON.stringify(list));
  assert.match(list.count, /patterns$/, "the list does not say how many patterns there are");
  assert(list.rows > 0, "the library list came back empty");
  assert.equal(list.detail, "", "the list showed a pattern nobody picked");
  assert.equal(fetched.length, 0, "opening the list fetched a pattern");

  // Filtering by name, then picking a pattern that needs a second file.
  await page.locator(".kp-lib-search").fill("vhd");
  await page.waitForTimeout(200);
  await page.locator('.kp-lib-row[data-pattern="vhd.hexpat"]').click();
  const notice = await page.evaluate(() => ({
    picked: document.querySelector(".kp-lib-picked")?.textContent ?? "",
    extra: document.querySelector(".kp-lib-extra")?.textContent ?? "",
    heading: document.querySelector(".kp-lib-notice-heading")?.textContent ?? "",
    notice: document.querySelector(".kp-lib-notice")?.textContent ?? "",
    fetch: document.querySelector(".kp-lib-fetch")?.textContent ?? "",
  }));
  console.log("notice", JSON.stringify(notice));
  assert.equal(notice.picked, "vhd.hexpat", "the notice does not name the pattern that was picked");
  assert.equal(notice.extra, "1 extra file", "the notice does not say the pattern needs another file");
  assert.match(notice.notice, /GPL-2\.0/, "the notice does not say what licence the library is under");
  assert.match(notice.notice, /does not save it/, "the notice does not say the pattern is not stored");
  assert.equal(notice.fetch, "Fetch vhd.hexpat and 1 more file", "the button does not say what it will fetch");
  assert.equal(fetched.length, 0, "picking a pattern fetched it without being asked");
  await page.screenshot({ path: join(outDir, "hexpat-library.png") });

  // The one click that reaches the network. Both files come back, and the
  // pattern is converted with the one it imports beside it.
  await page.locator(".kp-lib-fetch").click();
  await page.waitForSelector(".kp-lib", { state: "hidden", timeout: 10000 });
  await page.waitForTimeout(700);
  console.log("fetched", JSON.stringify(fetched));
  assert.deepEqual(fetched, ["vhd.hexpat", "fs/mbr.hexpat"], "the wrong files were fetched");
  const library = await page.evaluate(() => ({
    name: document.querySelector(".kp-hexpat .kp-name").textContent,
    length: document.querySelector(".kp-hexpat .kp-source").value.length,
    failed: document.querySelectorAll(".kp-hexpat .kp-error").length,
    needs: document.querySelectorAll(".kp-hexpat .kp-need").length,
    gaps: document.querySelector(".kp-hexpat .kp-gaps > .kp-group-heading")?.textContent ?? "",
    out: document.querySelector(".kp-hexpat .kp-out").textContent.slice(0, 60),
  }));
  console.log("from the library", JSON.stringify(library));
  assert.equal(library.name, "vhd.hexpat", "the toolbar does not name the pattern that was fetched");
  assert(library.length > 0, "the fetched pattern came back empty");
  assert.equal(library.failed, 0, "the fetched pattern did not convert");
  assert.equal(library.needs, 0, "the imported file was fetched but not handed to the converter");
  // The index is regenerated from the converter, so the count it records is
  // the one the panel must show; a literal here would go stale with every
  // converter gain.
  const index = JSON.parse(await readFile(new URL("../public/hexpat-index.json", import.meta.url), "utf8"));
  const recorded = index.patterns.find((p) => p.path === "vhd.hexpat")?.gaps;
  assert.match(library.gaps, new RegExp(`^Could not be expressed \\(${recorded}\\)`), "the gap count is not the one the index recorded");
  // The panel with a library pattern in it, converted, before it is applied.
  await page.screenshot({ path: join(outDir, "hexpat-fetched.png") });

  await page.getByRole("button", { name: "Use this template", exact: true }).click();
  await page.waitForTimeout(800);
  const fromLibrary = await page.evaluate(() => ({
    menu: document.querySelector(".tb-tmpl")?.textContent ?? "",
    rows: document.querySelectorAll(".rp-row").length,
  }));
  console.log("applied from the library", JSON.stringify(fromLibrary));
  assert.equal(fromLibrary.menu, "Template: vhd (from .hexpat)", "the menu does not name the pattern from the library");
  assert(fromLibrary.rows > 0, "the listing has no rows under the pattern from the library");
  await page.screenshot({ path: join(outDir, "hexpat-applied.png") });

  // ---- the shipped patterns, and a drop ----

  await openConverter();
  await page.waitForSelector(".kp-hexpat:not([hidden])", { timeout: 10000 });
  const shipped = await page.evaluate(() => {
    const list = document.querySelector(".kp-hexpat .kp-bundled");
    return { count: list.options.length, first: list.options[1]?.value ?? "", label: list.options[1]?.textContent ?? "" };
  });
  console.log("bundled list", JSON.stringify(shipped));
  assert(shipped.count >= 5, "the panel offers no bundled patterns");
  await page.selectOption(".kp-hexpat .kp-bundled", shipped.first);
  await page.waitForTimeout(600);
  const inBox = await page.evaluate(() => ({
    picked: document.querySelector(".kp-hexpat .kp-bundled").value,
    length: document.querySelector(".kp-hexpat .kp-source").value.length,
    failed: document.querySelectorAll(".kp-hexpat .kp-error").length,
  }));
  console.log("bundled in the box", JSON.stringify(inBox));
  assert.equal(inBox.picked, shipped.first, "the list does not hold the pattern that was picked");
  assert(inBox.length > 0, "the shipped pattern came back empty");
  assert.equal(inBox.failed, 0, "a shipped pattern did not convert");
  await page.keyboard.press("Escape");
  await page.waitForSelector(".kp-hexpat", { state: "hidden", timeout: 5000 });

  // A .hexpat dropped on the window opens the converter with the text in it,
  // rather than opening the pattern as the document.
  await page.evaluate((text) => {
    const dt = new DataTransfer();
    dt.items.add(new File([text], "dropped.hexpat", { type: "text/plain" }));
    document.dispatchEvent(new DragEvent("drop", { bubbles: true, cancelable: true, dataTransfer: dt }));
  }, small);
  await page.waitForSelector(".kp-hexpat:not([hidden])", { timeout: 10000 });
  await page.waitForTimeout(600);
  const dropped = await page.evaluate(() => ({
    name: document.querySelector(".kp-hexpat .kp-name").textContent,
    file: document.querySelector(".tb-file")?.textContent ?? "",
    out: document.querySelector(".kp-hexpat .kp-out").textContent.slice(0, 40),
  }));
  console.log("dropped", JSON.stringify(dropped));
  assert.equal(dropped.name, "dropped.hexpat", "the converter does not name the pattern that was dropped on it");
  assert.match(dropped.out, /template dropped/, "the dropped pattern did not convert");
  assert.match(dropped.file, /0-saka/, "the dropped pattern replaced the open file");

  assert.deepEqual(errors, [], "page errors");
  console.log("screenshots in", outDir);
  await page.close();
} finally {
  await browser.close();
}
