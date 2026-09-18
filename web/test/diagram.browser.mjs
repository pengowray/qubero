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
// OUT_DIR is where the screenshots go. The sample collection is found beside
// the checkout, or wherever QUBERO_SAMPLES names.
import assert from "node:assert/strict";
import { mkdir, readFile } from "node:fs/promises";
import { basename, join } from "node:path";

import { samplesDir } from "./samples.mjs";

const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || "playwright");
const samples = samplesDir();
if (samples === null) {
  console.log("skipped: no sample collection. Put `qubero-samples` beside this checkout, or point QUBERO_SAMPLES at it.");
  process.exit(0);
}
const outDir = process.env.OUT_DIR || new URL("out", import.meta.url).pathname.replace(/^\//, "");
await mkdir(outDir, { recursive: true });

// One drawing per format, because each exercises a different shape: PNG is a
// chunk switch with a case per type, ELF a header pointing at two tables, WAV
// the format whose ID3 frames used to be drawn ninety-six times over, ISO 9660
// a box with several type edges leaving it at once, and JPEG the figure the
// strip mode is modelled on, whose segment body is a choice of two dozen types
// and so the one drawing where a box opens onto a rail rather than a funnel.
//
// `template` picks the format from the toolbar rather than relying on a sample
// that happens to sniff as it. The diagram is a picture of the template, so
// which file is open only has to be a file.
const cases = [
  { file: join(samples, "pico8/p8png-test.p8.png"), shot: "diagram.png", fitShot: "diagram-fit.png", darkShot: "diagram-dark.png", pick: true },
  { file: join(samples, "elf/busybox-x86_64"), shot: "diagram-elf.png", fitShot: "diagram-elf-fit.png" },
  { file: join(samples, "wav/pcm-s16le-stereo-44100.wav"), template: "wav", shot: "diagram-wav.png", fitShot: "diagram-wav-fit.png" },
  { file: join(samples, "cdrom/hello-mode1.bin"), template: "iso9660", shot: "diagram-iso.png", fitShot: "diagram-iso-fit.png" },
  { file: join(samples, "jpeg/libjpeg-turbo-testorig-baseline.jpg"), shot: "diagram-jpeg.png", fitShot: "diagram-jpeg-fit.png", rail: true },
];

/** How much of its end two arrows into one row may share. Past this they are
 *  two lines drawn over each other rather than a fan into one place. */
const MERGE_STUB = 14;

const browser = await chromium.launch({ headless: true });
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
      // The template chip opens the settings dialog, whose list has a row per template.
      await page.click(".tb-tmpl");
      await page.click(`.settings-dlg [data-template="${c.template}"]`);
      await page.keyboard.press("Escape");
      await page.waitForTimeout(600);
    }
    await page.getByRole("button", { name: "Diagram", exact: true }).click();
    // Strips are the default; the arrows are measured first.
    await page.waitForSelector(".dv-mode", { timeout: 20000 });
    await page.selectOption(".dv-mode", "arrows");
    await page.waitForSelector(".dv-box", { timeout: 20000 });
    // The layout runs once the boxes are measured, so wait for it to have put
    // them somewhere rather than photographing a pile at the origin.
    await page.waitForFunction(() => {
      const boxes = [...document.querySelectorAll(".dv-box")];
      return boxes.length > 1 && new Set(boxes.map((b) => b.style.left)).size > 1;
    }, { timeout: 20000 });
    // The fonts settle in a rebuild of their own; measuring before that would
    // measure the fallback font's rows. The layout runs in a worker, so the
    // boxes are on screen before the arrows are: wait for the arrows too, and
    // for the picture to stop changing, before measuring any of it.
    await page.evaluate(() => document.fonts.ready);
    // Settled, not merely started: the census arriving and the fonts loading
    // each set the whole picture off again, and the layout runs in a worker, so
    // a count taken the moment the first arrows appear is a count of a drawing
    // about to be thrown away.
    let was = -1;
    for (let tries = 0; tries < 60; tries++) {
      await page.waitForTimeout(500);
      const now = await page.evaluate(() => document.querySelectorAll(".dv-edge path").length);
      if (now > 0 && now === was) break;
      was = now;
    }
    const found = await page.evaluate(() => {
      const boxes = [...document.querySelectorAll(".dv-box")];
      const rows = [...document.querySelectorAll(".dv-row")];
      const edges = [...document.querySelectorAll(".dv-edge path")];
      const labels = [...document.querySelectorAll(".dv-edge-label")];
      const stage = document.querySelector(".dv-stage");
      const all = shapes();
      return {
        boxes: boxes.length,
        rows: rows.length,
        edges: edges.length,
        // An edge whose path is NaN draws nothing and is the one failure a
        // screenshot cannot be trusted to show.
        badPaths: edges.filter((p) => /NaN|undefined/.test(p.getAttribute("d") || "")).length,
        labels: labels.length,
        pickable: document.querySelectorAll(".dv-box:first-of-type .dv-row.is-goable").length,
        note: document.querySelector(".dv-note")?.textContent ?? "",
        stage: stage === null ? null : [stage.style.width, stage.style.height],
        overlaps: overlapping(boxes),
        ...endpointDrift(all),
        ...crossings(all),
        ...worstShared(all),
        overlappingLabels: labelClashes(),
        // What the census put on the drawing: how many boxes the file has none
        // of, how many carry a count, and the biggest count shown.
        unusedBoxes: document.querySelectorAll(".dv-box.is-unused").length,
        badges: document.querySelectorAll(".dv-count").length,
        goable: document.querySelectorAll(".is-goable").length,
        status: document.querySelector(".dv-status")?.textContent ?? "",
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

      // Every arrow as the straight pieces it is made of, plus the little
      // arcs where it steps over another. The paths are `M`, `L` and `A` only,
      // which is what an orthogonal route with hops looks like.
      function shapes() {
        const out = [];
        for (const [i, path] of [...document.querySelectorAll(".dv-edge path")].entries()) {
          const d = (path.getAttribute("d") || "").trim();
          const pts = [];
          const hops = [];
          let at = null;
          for (const m of d.matchAll(/([MLA])\s+([-\d.\s]+)/g)) {
            const n = m[2].trim().split(/\s+/).map(Number);
            if (m[1] === "A") {
              // `A r r 0 0 sweep x y`: the arc is the hop, and where it lands
              // is where the line carries on from.
              const to = { x: n[5], y: n[6] };
              if (at !== null) hops.push({ x: (at.x + to.x) / 2, y: at.y });
              at = to;
              pts.push(to);
              continue;
            }
            at = { x: n[0], y: n[1] };
            pts.push(at);
          }
          const segs = [];
          for (let k = 1; k < pts.length; k++) {
            const a = pts[k - 1];
            const b = pts[k];
            const horizontal = Math.abs(a.y - b.y) <= 0.6;
            const vertical = Math.abs(a.x - b.x) <= 0.6;
            // A piece that is neither is the arc's own chord, which is not a
            // run and is not counted as one.
            if (!horizontal && !vertical) continue;
            if (Math.abs(a.x - b.x) < 0.6 && Math.abs(a.y - b.y) < 0.6) continue;
            segs.push({ owner: i, horizontal, a, b });
          }
          out.push({ owner: i, pts, segs, hops });
        }
        return out;
      }

      // How far each arrow's ends sit from the nearest row middle of the box
      // they touch, in stage units. An arrow that leaves two rows above the row
      // it is about says a connection the format does not have, and that is a
      // number rather than something to spot in a picture.
      function endpointDrift(all) {
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
        for (const e of all) {
          for (const p of [e.pts[0], e.pts[e.pts.length - 1]]) {
            if (p === undefined || !Number.isFinite(p.x) || !Number.isFinite(p.y)) continue;
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

      // The longest stretch two different arrows run along together, counted
      // twice: for arrows that end in different places, and for arrows that end
      // in the same one.
      //
      // The first is the fault the lanes never fixed, and is what must not
      // happen: two lines on the same y for a hundred pixels are one line as
      // far as a reader is concerned, and they are going to different places.
      // The second is a fan into one row, which is several arrows arriving at
      // one field and is a true picture of that; they have to share the
      // approach because the field is one place.
      function worstShared(all) {
        let apart = 0;
        let together = 0;
        const segs = all.flatMap((e) => e.segs.map((x) => ({ ...x, start: e.pts[0], end: e.pts[e.pts.length - 1] })));
        for (let i = 0; i < segs.length; i++) {
          for (let j = i + 1; j < segs.length; j++) {
            const a = segs[i];
            const b = segs[j];
            if (a.owner === b.owner || a.horizontal !== b.horizontal) continue;
            let run = 0;
            if (a.horizontal) {
              if (Math.abs(a.a.y - b.a.y) > 1) continue;
              const lo = Math.max(Math.min(a.a.x, a.b.x), Math.min(b.a.x, b.b.x));
              const hi = Math.min(Math.max(a.a.x, a.b.x), Math.max(b.a.x, b.b.x));
              run = hi - lo;
            } else {
              if (Math.abs(a.a.x - b.a.x) > 1) continue;
              const lo = Math.max(Math.min(a.a.y, a.b.y), Math.min(b.a.y, b.b.y));
              const hi = Math.min(Math.max(a.a.y, a.b.y), Math.max(b.a.y, b.b.y));
              run = hi - lo;
            }
            if (run <= 0) continue;
            // Sharing a run is a fan when the two arrows also share the place
            // they leave from or the place they arrive at: several arrows out
            // of one field, or into one field, have to share that end.
            const near = (u, v) => u !== undefined && v !== undefined && Math.abs(u.x - v.x) < 1.5 && Math.abs(u.y - v.y) < 1.5;
            const same = near(a.end, b.end) || near(a.start, b.start);
            if (same) together = Math.max(together, run);
            else apart = Math.max(apart, run);
          }
        }
        return {
          worstShared: Math.round(Math.max(0, apart) * 10) / 10,
          worstSharedFanIn: Math.round(Math.max(0, together) * 10) / 10,
        };
      }

      // Every place a horizontal run of one arrow crosses a vertical run of
      // another, and whether the horizontal one steps over it. A crossing drawn
      // as two lines meeting reads as a join, which on this diagram is the one
      // thing it must never say.
      function crossings(all) {
        let total = 0;
        let missing = 0;
        for (const e of all) {
          for (const h of e.segs.filter((x) => x.horizontal)) {
            const y = h.a.y;
            const lo = Math.min(h.a.x, h.b.x);
            const hi = Math.max(h.a.x, h.b.x);
            for (const other of all) {
              if (other.owner === e.owner) continue;
              for (const v of other.segs.filter((x) => !x.horizontal)) {
                const x = v.a.x;
                if (x <= lo + 1 || x >= hi - 1) continue;
                if (y <= Math.min(v.a.y, v.b.y) + 1 || y >= Math.max(v.a.y, v.b.y) - 1) continue;
                total++;
                // Within one arc's width: two crossings closer than that share
                // a single hop, which is the drawing's own rule.
                if (!e.hops.some((p) => Math.abs(p.x - x) <= 9 && Math.abs(p.y - y) < 3)) missing++;
              }
            }
          }
        }
        return { crossings: total, crossingsWithoutHop: missing };
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
      // A single click marks the row and leaves the reader where they are; the
      // double click is what goes.
      const first = page.locator(".dv-row.is-goable").first();
      await first.click();
      await page.waitForTimeout(300);
      assert(await page.locator('.tb-view.is-on:text-is("Diagram")').count() === 1, "a single click left the diagram");
      assert(await first.evaluate((el) => el.classList.contains("is-selected")), "a single click did not mark the row");
      await first.dblclick();
      await page.waitForSelector('.tb-view.is-on:text-is("Hex")', { timeout: 5000 });
      // And a double click on a row of a type deeper in the file, which is the
      // half the single click cannot reach: it goes to the first one the census
      // found rather than to a field of the root.
      await page.getByRole("button", { name: "Diagram", exact: true }).click();
      await page.waitForSelector(".dv-box", { timeout: 20000 });
      assert(found.goable > 0, "no row offered to go to the first one in the file");
      const deep = page.locator(".dv-row.is-goable").last();
      const before = await page.locator(".hv-offset, .status-offset, footer").first().textContent().catch(() => "");
      await deep.dblclick();
      await page.waitForSelector('.tb-view.is-on:text-is("Hex")', { timeout: 5000 });
      const after = await page.locator(".hv-offset, .status-offset, footer").first().textContent().catch(() => "");
      console.log(`  double-click went from ${JSON.stringify((before ?? "").trim().slice(0, 40))} to ${JSON.stringify((after ?? "").trim().slice(0, 40))}`);
    }
    // The other mode. Same file, same census, same double click: what changes
    // is that the format is drawn as the file is written rather than as the
    // fields decide each other. Checked here rather than in a test of its own
    // because what is worth knowing is that one file gets both pictures.
    // Back to the drawing first: the double click above left the reader in the
    // hex view, which is what it is for.
    await page.getByRole("button", { name: "Diagram", exact: true }).click();
    await page.waitForSelector(".dv-box", { timeout: 20000 });
    await page.selectOption(".dv-mode", "strips");
    await page.waitForSelector(".dv-strip", { timeout: 20000 });
    await page.waitForTimeout(600);
    const strips = await page.evaluate(() => ({
      strips: document.querySelectorAll(".dv-strip").length,
      boxes: document.querySelectorAll(".dv-sbox").length,
      bands: document.querySelectorAll(".dv-band").length,
      funnels: document.querySelectorAll(".dv-funnel").length,
      badges: document.querySelectorAll(".dv-count").length,
      goable: document.querySelectorAll(".is-goable").length,
      // Two strips of one row sharing pixels is the layout having failed: the
      // rows are laid out by hand and nothing else can put one over another.
      overlaps: (() => {
        const at = [...document.querySelectorAll(".dv-strip")].map((e) => ({
          l: parseFloat(e.style.left),
          t: parseFloat(e.style.top),
          w: e.offsetWidth,
          h: e.offsetHeight,
        }));
        let n = 0;
        for (let i = 0; i < at.length; i++) {
          for (let j = i + 1; j < at.length; j++) {
            const a = at[i];
            const b = at[j];
            if (a.l < b.l + b.w && b.l < a.l + a.w && a.t < b.t + b.h && b.t < a.t + a.h) n++;
          }
        }
        return n;
      })(),
      // A funnel drawn from a measurement that was not there is the fault a
      // screenshot cannot show: the line simply is not drawn.
      badPaths: [...document.querySelectorAll(".dv-funnel path")].filter((q) =>
        /NaN|undefined/.test(q.getAttribute("d") || ""),
      ).length,
      widestJoin: Math.max(
        0,
        ...[...document.querySelectorAll(".dv-funnel")].map((g) => g.querySelectorAll("path").length),
      ),
    }));
    await page.screenshot({ path: join(outDir, c.shot.replace(".png", "-strips.png")) });
    if (c.darkShot !== undefined) {
      await page.emulateMedia({ colorScheme: "dark" });
      await page.screenshot({ path: join(outDir, c.darkShot.replace(".png", "-strips.png")) });
      await page.emulateMedia({ colorScheme: "light" });
    }
    console.log(`  strips: ${JSON.stringify(strips)}`);
    assert(strips.strips >= 1, "no strips drawn");
    assert(strips.boxes >= 1, "no field boxes drawn in a strip");
    assert.equal(strips.overlaps, 0, "strips overlap each other");
    assert.equal(strips.badPaths, 0, "a funnel was drawn with a broken path");
    // The rail: one box opening onto two dozen types is drawn as a line over
    // the lot of them with a tick into each, and the check that it is there is
    // that one join holds more lines than a funnel's two.
    if (c.rail === true) assert(strips.widestJoin > 4, `the widest join has ${strips.widestJoin} lines, so no rail was drawn`);
    // The census belongs to the drawing, not to one way of drawing it.
    if (found.badges > 0) assert(strips.badges > 0, "the counts went away with the mode");
    if (found.goable > 0) assert(strips.goable > 0, "nothing offered to go to the file any more");
    // And back, because a reader who tries the other mode and returns should
    // find what they left.
    await page.selectOption(".dv-mode", "arrows");
    await page.waitForSelector(".dv-box", { timeout: 20000 });
    const back = await page.evaluate(() => ({
      boxes: document.querySelectorAll(".dv-box").length,
      strips: document.querySelectorAll(".dv-strip").length,
    }));
    assert(back.boxes >= 1, "the boxes did not come back");
    assert.equal(back.strips, 0, "the strips were left behind on the drawing");

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
    // Two arrows running along each other for longer than the stub two sharing
    // one row port are allowed: that is the fault the lanes never fixed.
    assert(
      found.worstShared <= MERGE_STUB,
      `two arrows going to different places share ${found.worstShared} units of one line`,
    );
    // And every crossing steps over, so none of them reads as a join.
    assert.equal(found.crossingsWithoutHop, 0, `${found.crossingsWithoutHop} of ${found.crossings} crossings have no hop`);
    await page.close();
  }
  console.log("screenshots in", outDir);
} finally {
  await browser.close();
}
