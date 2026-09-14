// When the inspector shows a name's path as the file stores it: the switch, the
// line under a name, the line under a formula, and which of a row's two
// mentions of one field carries it.
//
// The names are the core's answers for the first column chunk of
// alltypes_plain.parquet, whose footer is Thrift.

import { test } from "node:test";
import assert from "node:assert/strict";

import { anyStored, clauseStored, readShown, storedLine, STORED_PATHS_KEY, templateLine, writeShown } from "../src/storedpath.ts";
import { STORED_PATHS } from "../src/strings.ts";

const DATA_PAGE_OFFSET = { stored: "fields[id = 3].value.fields[7].value" };
const PLAIN = { stored: null };
const FORMULA = { template: "max(min(descriptor.(fields[id = 3].value.fields[id = 7].value), remaining), 0)" };

test("a name stored the way it is written has no second line, whether or not paths as stored are shown", () => {
  assert.equal(storedLine(PLAIN.stored, true), null);
  assert.equal(storedLine(PLAIN.stored, false), null);
  assert.equal(templateLine(null, true), null);
});

test("the stored path is shown only when asked for, after the words that say what it is", () => {
  assert.equal(storedLine(DATA_PAGE_OFFSET.stored, false), null);
  assert.deepEqual(storedLine(DATA_PAGE_OFFSET.stored, true), { word: "stored as", text: "fields[id = 3].value.fields[7].value" });
});

test("a row that leads with an address says whose path the line is", () => {
  assert.deepEqual(storedLine("blocks.elems[3].fields[2].value", true, "blocks[3].data"), {
    word: "blocks[3].data stored as",
    text: "blocks.elems[3].fields[2].value",
  });
});

test("a formula keeps the template's own spelling under the short one, and does not call it stored", () => {
  assert.equal(templateLine(FORMULA.template, false), null);
  const line = templateLine(FORMULA.template, true);
  assert.deepEqual(line, { word: "in the template", text: FORMULA.template });
  assert.ok(!line?.word.includes(STORED_PATHS.storedAs));
});

test("a clause carries the stored path only where its row has no working to carry it", () => {
  // `from header.type = DICTIONARY_PAGE` is the whole of its row.
  assert.equal(clauseStored({ stored: "header.fields[0].value" }, false), "header.fields[0].value");
  // `where descriptor footer.row_groups[0].columns[0] points` names the same
  // field as the first row of its working, which carries the line instead.
  assert.equal(clauseStored({ stored: "footer.fields.row_groups.value.elems[0]" }, true), null);
  assert.equal(clauseStored(null, false), null);
});

test("the switch is offered only when something on the panel is stored another way", () => {
  assert.equal(anyStored([PLAIN, PLAIN], [{ template: null }]), false);
  assert.equal(anyStored([], []), false);
  assert.equal(anyStored([PLAIN, DATA_PAGE_OFFSET], []), true);
  assert.equal(anyStored([PLAIN], [FORMULA]), true);
});

test("the switch is remembered, and storage that refuses reads as off", () => {
  const kept = new Map<string, string>();
  const storage = { getItem: (k: string) => kept.get(k) ?? null, setItem: (k: string, v: string) => void kept.set(k, v) };
  assert.equal(readShown(storage), false);
  writeShown(storage, true);
  assert.equal(kept.get(STORED_PATHS_KEY), "1");
  assert.equal(readShown(storage), true);
  writeShown(storage, false);
  assert.equal(readShown(storage), false);
  const refusing = {
    getItem: (): string | null => {
      throw new Error("blocked");
    },
    setItem: (): void => {
      throw new Error("blocked");
    },
  };
  assert.equal(readShown(refusing), false);
  assert.doesNotThrow(() => writeShown(refusing, true));
  assert.equal(readShown(null), false);
});
