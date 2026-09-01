// The column names of a SQLite table, out of the `CREATE` statement a person
// wrote. Kept apart from the rest of the record reading because it is the one
// piece with no file in it: text in, names out, which is what makes it worth
// testing on its own (`web/test/records.test.ts`).

/**
 * The column names written in a `CREATE TABLE` statement.
 *
 * Not a parser for the grammar, which is deep enough that a wrong answer is
 * likely and a wrong answer here puts a real column's data under another
 * column's name. This takes the names positionally and gives up rather than
 * guessing: the text between the first bracket and its match, split on the
 * commas that are not inside brackets or quotes, and the first identifier of
 * each part. A part that opens with a constraint word describes the table
 * rather than a column, and ends the list.
 */
export function columnNames(sql: string): string[] {
  const names: string[] = [];
  for (const part of splitDefinitions(sql)) {
    const name = leadingName(part);
    if (name === null) break;
    names.push(name);
  }
  return names;
}

const CONSTRAINT = /^(primary|unique|check|foreign|constraint)\b/i;

/** The identifier a column definition starts with, or null when the part is a
 *  constraint on the table rather than a column of it. */
function leadingName(part: string): string | null {
  if (part === "" || CONSTRAINT.test(part)) return null;
  const first = part[0] ?? "";
  const close = first === '"' ? '"' : first === "`" ? "`" : first === "[" ? "]" : first === "'" ? "'" : "";
  if (close !== "") {
    let out = "";
    for (let i = 1; i < part.length; i++) {
      const c = part[i] ?? "";
      if (c === close && part[i + 1] === close) {
        out += c;
        i++;
      } else if (c === close) return out;
      else out += c;
    }
    return out === "" ? null : out;
  }
  const bare = /^[A-Za-z_][\w$]*/.exec(part);
  return bare === null ? null : bare[0];
}

/** Whether a column definition makes its column an alias for the rowid, which
 *  SQLite stores as a null and reads back from the row's own number. */
export function isRowidAlias(part: string): boolean {
  return /\binteger\b[\s\S]*\bprimary\s+key\b/i.test(part) && !/\bdesc\b/i.test(part);
}

/** The bracketed list of a `CREATE` statement, split on its top-level commas.
 *  `columnNames` takes the names off the front of these; the rowid test needs
 *  the whole definition. */
export function splitDefinitions(sql: string): string[] {
  const open = sql.indexOf("(");
  if (open < 0) return [];
  const out: string[] = [];
  let depth = 0;
  let quote = "";
  let from = open + 1;
  let i = open;
  for (; i < sql.length; i++) {
    const c = sql[i] ?? "";
    if (quote !== "") {
      // Doubling is how every one of SQLite's quotes escapes itself.
      if (c === quote && sql[i + 1] === quote) i++;
      else if (c === quote) quote = "";
      continue;
    }
    if (c === '"' || c === "'" || c === "`") quote = c;
    else if (c === "[") quote = "]";
    else if (c === "(") depth++;
    else if (c === ")") {
      depth--;
      if (depth === 0) break;
    } else if (c === "," && depth === 1) {
      out.push(sql.slice(from, i).trim());
      from = i + 1;
    }
  }
  if (depth !== 0) return [];
  out.push(sql.slice(from, i).trim());
  return out;
}

