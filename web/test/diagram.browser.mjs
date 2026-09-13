// The Diagram view over a real file, drawn and photographed.
//
// Run against a Vite dev server. PLAYWRIGHT_MODULE can point to a bundled
// Playwright installation; TEST_URL must say localhost, not 127.0.0.1, since
// vite binds IPv6 and refuses the IPv4 address.
//
//   cd web
//   PLAYWRIGHT_MODULE="file:///C:/Users/pengo/AppData/Roaming/npm/node_modules/playwright/index.mjs" \
//   TEST_URL="http://localhost:17279" node test/diagram.browser.mjs
//
// SAMPLES points at the sample collection; OUT_DIR is where the screenshots go.
import assert from "node:assert/strict";
import { mkdir, readFile } from "node:fs/promises";
import { basename, join } from "node:path";

const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || "playwright");
const samples = process.env.SAMPLES || "C:/Users/pengo/Dropbox/projects/qubero-samples";
const outDir = process.env.OUT_DIR || new URL("out", import.meta.url).pathname.replace(/^\//, "");
await mkdir(outDir, { recursive: true });

// One drawing per format, because a diagram of a PNG and a diagram of an ELF
// exercise different shapes: PNG is a chunk switch with a case per type, ELF is
// a header pointing at two tables.
const cases = [
  { file: join(samples, "pico8/p8png-test.p8.png"), shot: "diagram.png", fitShot: "diagram-fit.png", darkShot: "diagram-dark.png" },
  { file: join(samples, "elf/busybox-x86_64"), shot: "diagram-elf.png", fitShot: "diagram-elf-fit.png" },
];

const browser = await chromium.launch({ channel: "msedge", headless: true });
try {
  for (const c of cases) {
    const page = await browser.newPage({ viewport: { width: 1400, height: 900 } });
    const errors = [];
    page.on("pageerror", (e) => errors.push(e.message));
    await page.goto(process.env.TEST_URL || "http://localhost:17279");
    // The app takes a file through a picker or a drop; the picker is the one a
    // browser tool can drive.
    const chooser = page.waitForEvent("filechooser");
    await page.getByRole("button", { name: "Open a file", exact: true }).click();
    await (await chooser).setFiles({ name: basename(c.file), buffer: await readFile(c.file) });
    await page.waitForSelector(".rp-row, .hv-hex", { timeout: 20000 });
    await page.getByRole("button", { name: "Diagram", exact: true }).click();
    await page.waitForSelector(".dv-box", { timeout: 20000 });
    // The layout runs once the boxes are measured, so wait for it to have put
    // them somewhere rather than photographing a pile at the origin.
    await page.waitForFunction(() => {
      const boxes = [...document.querySelectorAll(".dv-box")];
      return boxes.length > 1 && new Set(boxes.map((b) => b.style.left)).size > 1;
    }, { timeout: 20000 });
    const found = await page.evaluate(() => {
      const boxes = [...document.querySelectorAll(".dv-box")];
      const rows = [...document.querySelectorAll(".dv-row")];
      const edges = [...document.querySelectorAll(".dv-edge path")];
      const labels = [...document.querySelectorAll(".dv-edge-label")];
      const stage = document.querySelector(".dv-stage");
      return {
        boxes: boxes.length,
        rows: rows.length,
        edges: edges.length,
        // An edge whose path is NaN draws nothing and is the one failure a
        // screenshot cannot be trusted to show.
        badPaths: edges.filter((p) => /NaN|undefined/.test(p.getAttribute("d") || "")).length,
        labels: labels.length,
        note: document.querySelector(".dv-note")?.textContent ?? "",
        stage: stage === null ? null : [stage.style.width, stage.style.height],
        overlaps: overlapping(boxes),
      };
      // Two boxes sharing any of the same pixels. The layout places them in
      // layers, so any overlap at all is the layout having failed rather than a
      // matter of taste.
      function overlapping(boxes) {
        const r = boxes.map((b) => ({
          l: parseFloat(b.style.left),
          t: parseFloat(b.style.top),
          w: b.offsetWidth,
          h: b.offsetHeight,
        }));
        let n = 0;
        for (let i = 0; i < r.length; i++) {
          for (let j = i + 1; j < r.length; j++) {
            const a = r[i];
            const b = r[j];
            if (a.l < b.l + b.w && b.l < a.l + a.w && a.t < b.t + b.h && b.t < a.t + a.h) n++;
          }
        }
        return n;
      }
    });
    await page.screenshot({ path: join(outDir, c.shot) });
    // And the whole thing at once, which is the other half of what the view
    // offers and the half a screenshot of one corner cannot show.
    await page.getByRole("button", { name: "Fit", exact: true }).click();
    await page.screenshot({ path: join(outDir, c.fitShot) });
    // The same drawing in the dark theme. The boxes are DOM and the arrows are
    // SVG, so both follow `--field-color` and friends; this is the check that
    // nothing in either was written as a fixed colour.
    if (c.darkShot !== undefined) {
      await page.emulateMedia({ colorScheme: "dark" });
      await page.screenshot({ path: join(outDir, c.darkShot) });
      await page.emulateMedia({ colorScheme: "light" });
    }
    console.log(basename(c.file), JSON.stringify(found));
    assert.deepEqual(errors, [], `page errors for ${c.file}`);
    assert(found.boxes >= 1, "no boxes drawn");
    assert(found.rows >= 1, "no field rows drawn");
    assert(found.edges >= 1, "no edges drawn");
    assert.equal(found.badPaths, 0, "an edge was drawn with a broken path");
    assert.equal(found.overlaps, 0, "boxes overlap each other");
    await page.close();
  }
  console.log("screenshots in", outDir);
} finally {
  await browser.close();
}
