// The Diagram view over a real file, drawn, measured and photographed.
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

// One drawing per format, because each exercises a different shape: PNG is a
// chunk switch with a case per type, ELF a header pointing at two tables, WAV
// the format whose ID3 frames used to be drawn ninety-six times over, and
// ISO 9660 a box with several type edges leaving it at once.
//
// `template` picks the format from the toolbar rather than relying on a sample
// that happens to sniff as it. The diagram is a picture of the template, so
// which file is open only has to be a file.
const cases = [
  { file: join(samples, "pico8/p8png-test.p8.png"), shot: "diagram.png", fitShot: "diagram-fit.png", darkShot: "diagram-dark.png", pick: true },
  { file: join(samples, "elf/busybox-x86_64"), shot: "diagram-elf.png", fitShot: "diagram-elf-fit.png" },
  { file: join(samples, "wav/pcm-s16le-stereo-44100.wav"), template: "wav", shot: "diagram-wav.png", fitShot: "diagram-wav-fit.png" },
  { file: join(samples, "cdrom/hello-mode1.bin"), template: "iso9660", shot: "diagram-iso.png", fitShot: "diagram-iso-fit.png" },
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
    if (c.template !== undefined) {
      await page.selectOption(".tb-tmpl", c.template);
      await page.waitForTimeout(600);
    }
    await page.getByRole("button", { name: "Diagram", exact: true }).click();
    await page.waitForSelector(".dv-box", { timeout: 20000 });
    // The layout runs once the boxes are measured, so wait for it to have put
    // them somewhere rather than photographing a pile at the origin.
    await page.waitForFunction(() => {
      const boxes = [...document.querySelectorAll(".dv-box")];
      return boxes.length > 1 && new Set(boxes.map((b) => b.style.left)).size > 1;
    }, { timeout: 20000 });
    // The fonts settle in a rebuild of their own; measuring before that would
    // measure the fallback font's rows.
    await page.evaluate(() => document.fonts.ready);
    await page.waitForTimeout(300);
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
        pickable: document.querySelectorAll(".dv-row.is-pickable").length,
        note: document.querySelector(".dv-note")?.textContent ?? "",
        stage: stage === null ? null : [stage.style.width, stage.style.height],
        overlaps: overlapping(boxes),
        ...endpointDrift(),
        worstChannel: channelLoad(),
        overlappingLabels: labelClashes(),
        // What the census put on the drawing: how many boxes the file has none
        // of, how many carry a count, and the biggest count shown.
        unusedBoxes: document.querySelectorAll(".dv-box.is-unused").length,
        badges: document.querySelectorAll(".dv-count").length,
        goable: document.querySelectorAll(".is-goable").length,
        partial: document.querySelector(".dv-partial")?.hidden === false,
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

      // How far each arrow's ends sit from the nearest row middle of the box
      // they touch, in stage units. An arrow that leaves two rows above the row
      // it is about says a connection the format does not have, and that is a
      // number rather than something to spot in a picture.
      function endpointDrift() {
        const stage = document.querySelector(".dv-stage");
        if (stage === null) return { worstDrift: -1, driftOver3: -1 };
        const k = new DOMMatrixReadOnly(getComputedStyle(stage).transform).a || 1;
        const sr = stage.getBoundingClientRect();
        const geom = [...document.querySelectorAll(".dv-box")].map((b) => {
          const br = b.getBoundingClientRect();
          const mids = [...b.querySelectorAll(".dv-row, .dv-box-name")].map((r) => {
            const rr = r.getBoundingClientRect();
            return (rr.top + rr.height / 2 - sr.top) / k;
          });
          return { left: (br.left - sr.left) / k, right: (br.right - sr.left) / k, mids };
        });
        let worst = 0;
        let over = 0;
        for (const path of document.querySelectorAll(".dv-edge path")) {
          const d = (path.getAttribute("d") || "").replace(/\s+/g, " ");
          const start = /^M ([-\d.]+) ([-\d.]+)/.exec(d);
          const end = /H ([-\d.]+)$/.exec(d);
          const vs = [...d.matchAll(/V ([-\d.]+)/g)];
          const lastV = vs[vs.length - 1];
          if (start === null) continue;
          const ends = [{ x: +start[1], y: +start[2] }];
          if (end !== null && lastV !== undefined) ends.push({ x: +end[1], y: +lastV[1] });
          for (const p of ends) {
            if (!Number.isFinite(p.x) || !Number.isFinite(p.y)) continue;
            let best = Infinity;
            for (const b of geom) {
              // The box this end touches: the one whose left or right edge it
              // is standing on.
              if (Math.abs(p.x - b.left) > 2 && Math.abs(p.x - b.right) > 2) continue;
              for (const y of b.mids) best = Math.min(best, Math.abs(y - p.y));
            }
            if (best === Infinity) continue;
            worst = Math.max(worst, best);
            if (best > 3) over++;
          }
        }
        return { worstDrift: Math.round(worst * 10) / 10, driftOver3: over };
      }

      // The most arrows sharing one vertical run. One lane per edge is the
      // point of the lane allocation; several on one x is the bundle it
      // replaced.
      function channelLoad() {
        const at = new Map();
        for (const path of document.querySelectorAll(".dv-edge path")) {
          for (const m of (path.getAttribute("d") || "").matchAll(/H ([-\d.]+) V/g)) {
            const x = Math.round(parseFloat(m[1]));
            at.set(x, (at.get(x) ?? 0) + 1);
          }
        }
        return at.size === 0 ? 0 : Math.max(...at.values());
      }

      // Pairs of role words drawn over each other.
      function labelClashes() {
        const rs = [...document.querySelectorAll(".dv-edge-label")].map((t) => t.getBoundingClientRect());
        let n = 0;
        for (let i = 0; i < rs.length; i++) {
          for (let j = i + 1; j < rs.length; j++) {
            const a = rs[i];
            const b = rs[j];
            if (a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom) n++;
          }
        }
        return n;
      }
    });
    await page.screenshot({ path: join(outDir, c.shot) });
    // The toggle: the drawing is of the format, and this narrows it to the part
    // this file is an example of. Photographed both ways, because which of the
    // two a reader is looking at is the thing that must never be in doubt.
    const onlyShot = c.shot.replace(".png", "-only.png");
    const before = found.boxes;
    await page.locator(".dv-only input").check();
    await page.waitForTimeout(400);
    const after = await page.evaluate(() => ({
      boxes: document.querySelectorAll(".dv-box").length,
      edges: document.querySelectorAll(".dv-edge path").length,
    }));
    await page.screenshot({ path: join(outDir, onlyShot) });
    await page.locator(".dv-only input").uncheck();
    await page.waitForTimeout(400);
    console.log(`  only-what-this-file-has: ${before} boxes -> ${after.boxes}, ${after.edges} edges`);
    assert(after.boxes <= before, "the toggle drew more boxes rather than fewer");
    assert(after.boxes >= 1, "the toggle hid everything");

    // A close-up of the busiest channel: the place where several arrows leave
    // one box at once, which is where a bundle would show and where the lanes
    // have to be readable one at a time.
    //
    // Fit first so everything is on screen and the busy place can be found,
    // then zoom into it the way a reader would, with the wheel over it, so what
    // is photographed is what the view actually does rather than a transform
    // the test wrote itself.
    await page.getByRole("button", { name: "Fit", exact: true }).click();
    await page.waitForTimeout(200);
    const aim = await page.evaluate(() => {
      // The box the most arrows leave, and where its right edge is on screen.
      // Aimed at a box rather than at a channel: a channel's middle can be
      // empty space under the drawing, and what is worth looking at is the
      // place the lines come from.
      const stage = document.querySelector(".dv-stage");
      if (stage === null) return null;
      const k = new DOMMatrixReadOnly(getComputedStyle(stage).transform).a || 1;
      const sr = stage.getBoundingClientRect();
      const starts = [];
      for (const path of document.querySelectorAll(".dv-edge path")) {
        const m = /^M ([-\d.]+) ([-\d.]+)/.exec((path.getAttribute("d") || "").replace(/\s+/g, " "));
        if (m !== null) starts.push({ x: +m[1], y: +m[2] });
      }
      let best = null;
      for (const b of document.querySelectorAll(".dv-box")) {
        const left = parseFloat(b.style.left);
        const right = left + b.offsetWidth;
        const mine = starts.filter((s) => Math.abs(s.x - right) < 2 || Math.abs(s.x - left) < 2);
        if (mine.length > 1 && (best === null || mine.length > best.mine.length)) best = { b, mine };
      }
      if (best === null) return null;
      const q = best.b.getBoundingClientRect();
      const mid = best.mine.reduce((a, s) => a + s.y, 0) / best.mine.length;
      return { x: q.right, y: sr.top + mid * k, count: best.mine.length };
    });
    if (aim !== null) {
      await page.mouse.move(aim.x, aim.y);
      for (let i = 0; i < 10; i++) await page.mouse.wheel(0, -120);
      await page.waitForTimeout(300);
      const board = await page.locator(".dv-board").boundingBox();
      const clip = {
        x: Math.max(board.x, aim.x - 420),
        y: Math.max(board.y, aim.y - 300),
        width: Math.min(840, board.x + board.width - Math.max(board.x, aim.x - 420)),
        height: Math.min(600, board.y + board.height - Math.max(board.y, aim.y - 300)),
      };
      await page.screenshot({ path: join(outDir, c.shot.replace(".png", "-close.png")), clip });
    }
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
    // A click on a field of the root type takes the reader to it in the hex
    // view. Only rows a click can reach say so, so this also checks that the
    // ones that cannot are not offering.
    if (c.pick === true) {
      assert(found.pickable > 0, "no row offered to take the reader to the file");
      await page.locator(".dv-row.is-pickable").first().click();
      await page.waitForSelector('.tb-view.is-on:text-is("Hex")', { timeout: 5000 });
    }
    console.log(basename(c.file), JSON.stringify(found));
    assert.deepEqual(errors, [], `page errors for ${c.file}`);
    assert(found.boxes >= 1, "no boxes drawn");
    assert(found.rows >= 1, "no field rows drawn");
    assert(found.edges >= 1, "no edges drawn");
    assert.equal(found.badPaths, 0, "an edge was drawn with a broken path");
    assert.equal(found.overlaps, 0, "boxes overlap each other");
    // Half a row is 8 stage units; anything past that is an arrow pointing at
    // the wrong field.
    assert(found.worstDrift >= 0 && found.worstDrift <= 3, `arrow ends drift ${found.worstDrift} from their rows`);
    assert.equal(found.driftOver3, 0, "some arrow ends are not on a row");
    await page.close();
  }
  console.log("screenshots in", outDir);
} finally {
  await browser.close();
}
