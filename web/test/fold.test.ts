// When a list is folded behind a "… N more" row: never to hide a row or three
// that the fold row itself would take the room of.

import { test } from "node:test";
import assert from "node:assert/strict";

import { FOLD_SLACK, folds, shownBeforeFold } from "../src/fold.ts";
import { plan } from "../src/strips.ts";
import type { DiagramBox, TemplateDiagram } from "../src/doc.ts";
import { DIAGRAM } from "../src/strings.ts";

test("a list at or under its cap is shown whole", () => {
  assert.equal(shownBeforeFold(24, 24), 24);
  assert.equal(shownBeforeFold(3, 24), 3);
  assert.equal(folds(24, 24), false);
});

test("a list a few rows over its cap is shown whole rather than folding one row away", () => {
  for (let over = 1; over <= FOLD_SLACK; over++) {
    assert.equal(shownBeforeFold(24 + over, 24), 24 + over, `${over} over`);
    assert.equal(folds(24 + over, 24), false, `${over} over`);
  }
});

test("past the slack the cap is kept and the fold hides more than the slack", () => {
  const total = 24 + FOLD_SLACK + 1;
  assert.equal(shownBeforeFold(total, 24), 24);
  assert.equal(folds(total, 24), true);
  assert.ok(total - shownBeforeFold(total, 24) > FOLD_SLACK);
});

test("no fold anywhere hides exactly one row", () => {
  for (let cap = 1; cap <= 30; cap++) {
    for (let total = 0; total <= 60; total++) {
      assert.notEqual(total - shownBeforeFold(total, cap), 1, `${total} rows under a cap of ${cap}`);
    }
  }
});

test("the fold row says field or fields by the count", () => {
  assert.equal(DIAGRAM.more(1), "… 1 more field");
  assert.equal(DIAGRAM.more(5), "… 5 more fields");
});

function box(name: string, rows: number): DiagramBox {
  return {
    key: name,
    name,
    path: name,
    kind: "struct",
    rows: Array.from({ length: rows }, (_, i) => ({
      name: `f${i}`,
      pos_text: "",
      size_text: "1",
      type_text: "u8",
      kind: "number",
      list: false,
    })),
  } as unknown as DiagramBox;
}

function strip(rows: number) {
  const d = { types: [box("Root", rows)], edges: [], omitted: 0 } as unknown as TemplateDiagram;
  const made = plan(d, 24, () => true);
  return made.strips[0]?.items ?? [];
}

test("a strip a row over its cap draws every field and no fold", () => {
  const items = strip(25);
  assert.equal(items.filter((i) => i.kind === "more").length, 0);
  assert.equal(items.filter((i) => i.kind === "field").length, 25);
});

test("a strip well over its cap folds, and the fold counts what it hides", () => {
  const items = strip(40);
  const more = items.filter((i) => i.kind === "more");
  assert.equal(more.length, 1);
  // The first 23, the fold, and the last: the fold stands for the 16 between.
  assert.equal(more[0]?.name, "16");
});
