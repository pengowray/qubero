// The column names come out of SQL a person wrote, so the extractor has to
// survive what people write. Getting a name wrong is worse than having none:
// it puts one column's data under another column's heading.

import { test } from "node:test";
import assert from "node:assert/strict";

import { columnNames } from "../src/records.ts";

test("the plain case", () => {
  assert.deepEqual(columnNames("CREATE TABLE notes(id INTEGER PRIMARY KEY, title TEXT, body TEXT)"), [
    "id",
    "title",
    "body",
  ]);
});

test("a constraint on the table is not a column of it", () => {
  assert.deepEqual(columnNames("CREATE TABLE t(a INT, b INT, PRIMARY KEY (a, b))"), ["a", "b"]);
  assert.deepEqual(columnNames("CREATE TABLE t(a INT, FOREIGN KEY (a) REFERENCES u(x))"), ["a"]);
  assert.deepEqual(columnNames('CREATE TABLE t(a INT, CONSTRAINT "c" UNIQUE (a))'), ["a"]);
});

test("brackets inside a definition are not the end of it", () => {
  assert.deepEqual(columnNames("CREATE TABLE t(a VARCHAR(20), b DECIMAL(10, 2), c INT)"), ["a", "b", "c"]);
  assert.deepEqual(columnNames("CREATE TABLE t(a INT CHECK (a > 0 AND a < 9), b INT)"), ["a", "b"]);
});

test("a quoted name may hold anything, including a comma", () => {
  assert.deepEqual(columnNames('CREATE TABLE t("first, last" TEXT, [odd name] INT, `back` INT)'), [
    "first, last",
    "odd name",
    "back",
  ]);
});

test("a quote doubled inside a quoted name is one character", () => {
  assert.deepEqual(columnNames('CREATE TABLE t("say ""hi""" TEXT, b INT)'), ['say "hi"', "b"]);
});

test("nothing to read gives nothing rather than a guess", () => {
  assert.deepEqual(columnNames("CREATE TABLE t"), []);
  assert.deepEqual(columnNames("CREATE TABLE t(a INT"), []);
  assert.deepEqual(columnNames(""), []);
});

test("a virtual table's arguments are not columns and are not claimed to be", () => {
  // Read positionally this looks like two columns; the gate on column count
  // is what keeps a wrong reading off the screen, not this.
  assert.deepEqual(columnNames("CREATE VIRTUAL TABLE t USING fts5(content, tokenize = 'porter')"), [
    "content",
    "tokenize",
  ]);
});
