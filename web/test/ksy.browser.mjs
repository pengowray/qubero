// The .ksy converter panel, driven the way a reader drives it.
//
// Run against a Vite dev server. PLAYWRIGHT_MODULE can point to a bundled
// Playwright installation; TEST_URL must say localhost, not 127.0.0.1, since
// vite binds IPv6 and refuses the IPv4 address.
//
//   cd web
//   PORT=17281 npm run dev
//   PLAYWRIGHT_MODULE="file:///C:/Users/pengo/AppData/Roaming/npm/node_modules/playwright/index.mjs" \
//   TEST_URL="http://localhost:17281" node test/ksy.browser.mjs
//
// KAITAI_FORMATS points at the Kaitai format library, SAMPLES at the sample
// collection, OUT_DIR at where the screenshots go.
import assert from "node:assert/strict";
import { mkdir, readFile } from "node:fs/promises";
import { basename, join } from "node:path";

const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || "playwright");
const formats = process.env.KAITAI_FORMATS || "D:/github/kaitai_struct/formats";
const samples = process.env.SAMPLES || "C:/Users/pengo/Dropbox/projects/qubero-samples";
const outDir = process.env.OUT_DIR || new URL("out", import.meta.url).pathname.replace(/^\//, "");
await mkdir(outDir, { recursive: true });

const gifKsy = await readFile(join(formats, "image/gif.ksy"), "utf8");
// A .ksy of the format the open file is actually in, for the half of the test
// that reads the file with what came out. The collection has no GIF; the PICO-8
// cartridge is a PNG, so png.ksy is what the listing can be checked against.
const pngKsy = await readFile(join(formats, "image/png.ksy"), "utf8");
const sample = join(samples, "pico8/p8png-test.p8.png");

const browser = await chromium.launch({ channel: "msedge", headless: true });
try {
  const page = await browser.newPage({ viewport: { width: 1400, height: 900 } });
  const errors = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.goto(process.env.TEST_URL || "http://localhost:17281");

  // A file first: the converter is a tool over an open document, and the
  // template menu it is opened from belongs to that document's toolbar.
  const chooser = page.waitForEvent("filechooser");
  await page.getByRole("button", { name: "Open a file", exact: true }).click();
  await (await chooser).setFiles({ name: basename(sample), buffer: await readFile(sample) });
  await page.waitForSelector(".rp-row, .hv-hex", { timeout: 20000 });

  // Open the panel the way the menu offers it, rather than by calling into the
  // page: the entry at the end of the template menu is half of what is being
  // tested.
  await page.selectOption(".tb-tmpl", { label: "Convert a .ksy…" });
  await page.waitForSelector(".kp:not([hidden])", { timeout: 10000 });
  // Picking the tool does not change what is reading the file.
  assert.notEqual(await page.inputValue(".tb-tmpl"), "open-ksy-converter", "the menu stayed on the converter entry");

  const paste = async (text) => {
    await page.locator(".kp-source").fill(text);
    // The conversion is debounced; the report and the template arrive after it.
    await page.waitForTimeout(600);
  };

  await paste(gifKsy);
  const gif = await page.evaluate(() => ({
    out: document.querySelector(".kp-out").textContent,
    failed: document.querySelectorAll(".kp-error").length,
    gaps: document.querySelector(".kp-gaps > .kp-group-heading")?.textContent ?? "",
    notes: document.querySelector(".kp-notes > .kp-group-heading")?.textContent ?? "",
    fields: document.querySelector(".kp-fields > .kp-group-heading")?.textContent ?? "",
    lines: document.querySelectorAll(".kp-line").length,
    applyDisabled: document.querySelector(".kp-foot button").disabled,
    // The field list is one line per field, which is every line there is: it
    // starts folded away so the two groups above it are what is read first.
    fieldsOpen: document.querySelector(".kp-fields")?.open === true,
  }));
  console.log("gif.ksy", JSON.stringify(gif).slice(0, 400));
  assert.equal(gif.failed, 0, "gif.ksy did not convert");
  assert.match(gif.out, /\bgif\b/, "the template column does not name the format");
  assert.equal(gif.applyDisabled, false, "a converted .ksy left the apply button disabled");
  assert(gif.lines > 0, "the report listed nothing at all");
  assert.equal(gif.fieldsOpen, false, "the field list did not start folded away");

  await page.screenshot({ path: join(outDir, "ksy.png") });
  await page.emulateMedia({ colorScheme: "dark" });
  await page.screenshot({ path: join(outDir, "ksy-dark.png") });
  await page.emulateMedia({ colorScheme: "light" });

  // A report line goes to the line of the .ksy it came from. Clicking the first
  // line of the field list puts the caret in the text box, not at its start.
  await page.locator(".kp-fields > .kp-group-heading").click();
  await page.locator(".kp-fields .kp-line").first().click();
  const caret = await page.evaluate(() => {
    const t = document.querySelector(".kp-source");
    return { start: t.selectionStart, line: t.value.slice(0, t.selectionStart).split("\n").length };
  });
  console.log("caret after clicking the first field line", JSON.stringify(caret));
  assert(caret.start > 0, "clicking a report line did not move the caret");
  // With the field list open, which is the other half of what the panel looks
  // like.
  await page.screenshot({ path: join(outDir, "ksy-fields.png") });

  // A .ksy that does not convert: one line in place of the report, the path in
  // it first, and nothing to apply. Broken YAML rather than a bad type, which
  // is a gap and not a refusal: the field is read as bytes and the rest of the
  // format still converts.
  await paste("meta:\n  id: broken\nseq:\n  - id: x\n    type: {oops\n");
  const broken = await page.evaluate(() => ({
    path: document.querySelector(".kp-error .kp-line-path")?.textContent ?? "",
    message: document.querySelector(".kp-error .kp-line-message")?.textContent ?? "",
    groups: document.querySelectorAll(".kp-group").length,
    out: document.querySelector(".kp-out").textContent,
    applyDisabled: document.querySelector(".kp-foot button").disabled,
  }));
  console.log("broken.ksy", JSON.stringify(broken));
  // The whole-file path `/` is left off, since a line holding one slash says
  // nothing; a path inside the file is shown, and comes first.
  assert.equal(broken.path, "", "the whole-file path was written out as a slash");
  assert(broken.message.length > 0, "the error line says nothing");
  assert.equal(broken.groups, 0, "the report stayed up beside the error");
  assert.equal(broken.applyDisabled, true, "a .ksy that did not convert could still be applied");
  assert.match(broken.out, /did not convert/, "the template column does not say why it is empty");
  await page.screenshot({ path: join(outDir, "ksy-error.png") });

  // The .ksy that describes the file that is open, applied.
  await paste(pngKsy);
  const png = await page.evaluate(() => ({
    failed: document.querySelectorAll(".kp-error").length,
    gaps: document.querySelector(".kp-gaps > .kp-group-heading")?.textContent ?? "",
    notes: document.querySelector(".kp-notes > .kp-group-heading")?.textContent ?? "",
    fields: document.querySelector(".kp-fields > .kp-group-heading")?.textContent ?? "",
  }));
  console.log("png.ksy", JSON.stringify(png));
  assert.equal(png.failed, 0, "png.ksy did not convert");

  await page.getByRole("button", { name: "Use this template", exact: true }).click();
  await page.waitForSelector(".rp-row", { timeout: 20000 });
  const applied = await page.evaluate(() => ({
    menu: [...document.querySelectorAll(".tb-tmpl option")].find((o) => o.selected)?.textContent ?? "",
    view: document.querySelector(".tb-view.is-on")?.textContent ?? "",
    // A row names its field in `.rp-field`, or in `.rp-name` where the field is
    // a heading of its own.
    names: [...document.querySelectorAll(".rp-row .rp-field, .rp-row .rp-name")].slice(0, 20).map((e) => e.textContent.trim()),
    panelHidden: document.querySelector(".kp").hidden,
  }));
  console.log("applied", JSON.stringify(applied));
  assert.equal(applied.view, "Listing", "applying did not go back to the listing");
  assert.equal(applied.panelHidden, true, "the converter stayed open after applying");
  assert.equal(applied.menu, "Template: png (from .ksy)", "the menu does not name the converted format");
  // The fields png.ksy declares, which is what the listing shows once the file
  // is read with the converted template. `ihdr` and `chunks` are structures, so
  // the listing gives them heading rows and lists what is inside them: `width`
  // and `height` are the ihdr_chunk type the .ksy declares, read here.
  for (const name of ["magic", "ihdr_len", "ihdr_type", "ihdr_crc", "width", "height"]) {
    assert(applied.names.includes(name), `the listing has no ${name} row: ${applied.names.join(" ")}`);
  }

  // Reopening comes back to the text that was in it, and Escape closes.
  await page.selectOption(".tb-tmpl", { label: "Convert a .ksy…" });
  await page.waitForSelector(".kp:not([hidden])", { timeout: 10000 });
  const kept = await page.evaluate(() => document.querySelector(".kp-source").value.length);
  assert(kept > 0, "the converter came back empty");
  await page.keyboard.press("Escape");
  await page.waitForSelector(".kp", { state: "hidden", timeout: 5000 });

  // A .ksy whose meta/imports names another format: the shipped collection is
  // what the import resolves against, so nothing has to be supplied alongside
  // it and the imported types are in the template.
  await page.selectOption(".tb-tmpl", { label: "Convert a .ksy…" });
  await page.waitForSelector(".kp:not([hidden])", { timeout: 10000 });
  await paste(await readFile(join(formats, "media/wav.ksy"), "utf8"));
  const imported = await page.evaluate(() => ({
    failed: document.querySelectorAll(".kp-error").length,
    out: document.querySelector(".kp-out").textContent,
    gaps: document.querySelector(".kp-gaps > .kp-group-heading")?.textContent ?? "",
  }));
  console.log("wav.ksy", JSON.stringify({ ...imported, out: imported.out.slice(0, 80) }));
  assert.equal(imported.failed, 0, "wav.ksy, which imports /common/riff, did not convert");
  assert.match(imported.out, /riff/, "the template does not name anything from the imported riff");

  // A shipped description, read and applied from the panel. Applying one
  // nobody has edited reads the file as that bundled format, so the chooser
  // goes on naming it rather than gaining a second entry.
  const bundledId = await page.evaluate(() => {
    const list = document.querySelector(".kp-bundled");
    return { count: list.options.length, first: list.options[1]?.value ?? "", label: list.options[1]?.textContent ?? "" };
  });
  console.log("bundled list", JSON.stringify(bundledId));
  assert(bundledId.count > 10, "the panel offers no bundled descriptions");
  assert.match(bundledId.label, new RegExp(`^${bundledId.first}`), "a bundled entry does not lead with its id");
  await page.selectOption(".kp-bundled", bundledId.first);
  await page.waitForTimeout(600);
  const shipped = await page.evaluate(() => ({
    // The list is what names a shipped description; the slot beside it names a
    // file, and there is no file here.
    picked: document.querySelector(".kp-bundled").value,
    name: document.querySelector(".kp-name").textContent,
    length: document.querySelector(".kp-source").value.length,
    failed: document.querySelectorAll(".kp-error").length,
    out: document.querySelector(".kp-out").textContent.slice(0, 40),
  }));
  console.log("bundled in the box", JSON.stringify(shipped));
  assert.equal(shipped.picked, bundledId.first, "the list does not hold the description that was picked");
  assert.equal(shipped.name, "", "the toolbar named the shipped description twice");
  assert(shipped.length > 0, "the shipped description came back empty");
  assert.equal(shipped.failed, 0, "a shipped description did not convert");
  await page.screenshot({ path: join(outDir, "ksy-bundled.png") });

  await page.getByRole("button", { name: "Use this template", exact: true }).click();
  await page.waitForTimeout(500);
  const asBundled = await page.evaluate(() => ({
    value: document.querySelector(".tb-tmpl").value,
    note: document.querySelector(".ov-note")?.textContent ?? "",
    action: document.querySelector(".ov-note-action")?.textContent ?? "",
    // The entry for the .ksy that was pasted earlier named a template the
    // converter no longer holds, so it is gone rather than left to apply this
    // one under that name.
    pastedEntry: document.querySelectorAll('.tb-tmpl option[value="converted-ksy"]').length,
  }));
  console.log("applied bundled", JSON.stringify(asBundled));
  assert.equal(asBundled.pastedEntry, 0, "the menu kept an entry for a pasted .ksy that is no longer in the converter");
  assert.equal(asBundled.value, `ksy:${bundledId.first}`, "applying a shipped description did not select it in the chooser");
  assert.match(asBundled.note, /Kaitai Struct/, "the note does not say where the description came from");
  assert.equal(asBundled.action, "Show the .ksy", "the note does not offer the description");

  // The note in place, which is where a reader meets the offer.
  await page.getByRole("button", { name: /Overview/ }).first().click();
  await page.waitForTimeout(300);
  await page.screenshot({ path: join(outDir, "ksy-note.png") });

  // And back the other way: the note opens the converter on the shipped text.
  await page.evaluate(() => document.querySelector(".ov-note-action").click());
  await page.waitForSelector(".kp:not([hidden])", { timeout: 10000 });
  const reopened = await page.evaluate(() => ({
    picked: document.querySelector(".kp-bundled").value,
    length: document.querySelector(".kp-source").value.length,
  }));
  console.log("from the note", JSON.stringify(reopened));
  assert.equal(reopened.picked, bundledId.first, "the note opened the converter on the wrong description");
  assert(reopened.length > 0, "the note opened the converter on nothing");
  await page.keyboard.press("Escape");
  await page.waitForSelector(".kp", { state: "hidden", timeout: 5000 });

  // The other way in: a .ksy dropped on the window opens the converter with the
  // text in it, rather than opening the .ksy as the document.
  await page.evaluate((text) => {
    const dt = new DataTransfer();
    dt.items.add(new File([text], "dropped.ksy", { type: "text/plain" }));
    document.dispatchEvent(new DragEvent("drop", { bubbles: true, cancelable: true, dataTransfer: dt }));
  }, gifKsy);
  await page.waitForSelector(".kp:not([hidden])", { timeout: 10000 });
  await page.waitForTimeout(600);
  const dropped = await page.evaluate(() => ({
    name: document.querySelector(".kp-name").textContent,
    first: document.querySelector(".kp-source").value.slice(0, 20),
    file: document.querySelector(".tb-file")?.textContent ?? "",
    out: document.querySelector(".kp-out").textContent.slice(0, 20),
  }));
  console.log("dropped", JSON.stringify(dropped));
  assert.equal(dropped.name, "dropped.ksy", "the converter does not name the .ksy that was dropped on it");
  assert.match(dropped.out, /template gif/, "the dropped .ksy did not convert");
  assert.match(dropped.file, /p8png-test/, "the dropped .ksy replaced the open file");

  assert.deepEqual(errors, [], "page errors");
  console.log("screenshots in", outDir);
  await page.close();

  // A .ksy dropped with nothing open. It says how to read a file, so the start
  // screen asks for one rather than opening the .ksy as the document.
  const start = await browser.newPage({ viewport: { width: 1000, height: 700 } });
  const startErrors = [];
  start.on("pageerror", (e) => startErrors.push(e.message));
  await start.goto(process.env.TEST_URL || "http://localhost:17283");
  await start.waitForSelector(".welcome", { timeout: 10000 });
  await start.evaluate((text) => {
    const dt = new DataTransfer();
    dt.items.add(new File([text], "gif.ksy", { type: "text/plain" }));
    document.dispatchEvent(new DragEvent("drop", { bubbles: true, cancelable: true, dataTransfer: dt }));
  }, gifKsy);
  await start.waitForFunction(() => (document.querySelector(".welcome-status")?.textContent ?? "") !== "", { timeout: 5000 });
  const welcome = await start.evaluate(() => ({
    status: document.querySelector(".welcome-status").textContent,
    stillWelcome: document.querySelector(".welcome") !== null,
  }));
  console.log("welcome", JSON.stringify(welcome));
  assert.match(welcome.status, /^Open a file first/, "the start screen said nothing about the dropped .ksy");
  assert.equal(welcome.stillWelcome, true, "the dropped .ksy was opened as a document");
  assert.deepEqual(startErrors, [], "page errors on the start screen");
  await start.close();

  // A .ksy dropped on a tab of unpacked bytes, which no template of the
  // reader's choosing reads.
  const unpacked = await browser.newPage({ viewport: { width: 1200, height: 800 } });
  const unpackedErrors = [];
  unpacked.on("pageerror", (e) => unpackedErrors.push(e.message));
  await unpacked.goto(process.env.TEST_URL || "http://localhost:17283");
  const zlib = join(samples, "compressed/hello.zz");
  const zlibChooser = unpacked.waitForEvent("filechooser");
  await unpacked.getByRole("button", { name: "Open a file", exact: true }).click();
  await (await zlibChooser).setFiles({ name: basename(zlib), buffer: await readFile(zlib) });
  await unpacked.waitForSelector(".rp-row, .hv-hex", { timeout: 20000 });
  await unpacked.getByRole("button", { name: "Listing", exact: true }).click();
  // The row offering the unpacked stream arrives with the walk, not with the
  // file, and the button on it shows when its row is pointed at, so it is
  // clicked where it is rather than moved to.
  await unpacked.waitForSelector(".rp-unpacked", { state: "attached", timeout: 20000 });
  await unpacked.evaluate(() => document.querySelector(".rp-unpacked").click());
  await unpacked.waitForFunction(() => document.querySelectorAll(".tab").length > 1, { timeout: 20000 });
  await unpacked.evaluate((text) => {
    const dt = new DataTransfer();
    dt.items.add(new File([text], "gif.ksy", { type: "text/plain" }));
    document.dispatchEvent(new DragEvent("drop", { bubbles: true, cancelable: true, dataTransfer: dt }));
  }, gifKsy);
  await unpacked.waitForFunction(() => (document.querySelector(".tabpage:not([hidden]) .tb-msg")?.textContent ?? "") !== "", {
    timeout: 5000,
  });
  const refused = await unpacked.evaluate(() => ({
    message: document.querySelector(".tabpage:not([hidden]) .tb-msg").textContent,
    panels: document.querySelectorAll(".tabpage:not([hidden]) .kp").length,
  }));
  console.log("unpacked", JSON.stringify(refused));
  assert.match(refused.message, /unpacked stream/, "dropping a .ksy on unpacked bytes said nothing");
  assert.equal(refused.panels, 0, "the converter opened over unpacked bytes");
  assert.deepEqual(unpackedErrors, [], "page errors on the unpacked tab");
  await unpacked.close();
} finally {
  await browser.close();
}
