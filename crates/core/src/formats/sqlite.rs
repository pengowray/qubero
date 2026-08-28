//! SQLite 3: the 100-byte database header, then a run of fixed-size pages.
//!
//! The header's own fields are flattened into the root struct rather than
//! grouped, because an expression can only name a field in its own struct or
//! an enclosing one, and the page size decides how big every page is.
//!
//! Page 1 is the schema table, so its rows read as the five columns SQLite
//! keeps there rather than as a numbered list.
//!
//! Only a b-tree page has a type byte. Every other page in the file starts
//! with something else, so the byte is peeked rather than read and the page
//! that is not a b-tree keeps all of its bytes.
//!
//! Where this stops: a payload too big for its page reads as the bytes that
//! stayed and the number of the page the rest went to, and stops there. The
//! rest is on a chain of pages elsewhere in the file, and a record cut across
//! a page break is not something a field placed at an offset can read.

use crate::template::{Anchor, Encoding, Endian::*, Expr as E, StrLen, Template, Ty as T, Until};

/// What kind of b-tree node a page holds. Every other value belongs to a page
/// that is not a b-tree at all: a freelist page, an overflow page, or a
/// pointer map. Those read as bytes.
const PAGE_TYPE: &[(i128, &str)] = &[
    (2, "index interior"),
    (5, "table interior"),
    (10, "index leaf"),
    (13, "table leaf"),
];

const TEXT_ENCODING: &[(i128, &str)] = &[(1, "utf8"), (2, "utf16le"), (3, "utf16be")];

const WRITE_VERSION: &[(i128, &str)] = &[(1, "legacy"), (2, "wal")];

/// What a column's serial type says it holds. From 12 up the number is a
/// length rather than a name, so those show as numbers, which is what an
/// unnamed value in an enum does anyway.
const SERIAL_TYPE: &[(i128, &str)] = &[
    (0, "null"),
    (1, "i8"),
    (2, "i16"),
    (3, "i24"),
    (4, "i32"),
    (5, "i48"),
    (6, "i64"),
    (7, "f64"),
    // Two values that are the value: a column of serial type 8 holds 0, and
    // one of serial type 9 holds 1, in no bytes at all.
    (8, "zero"),
    (9, "one"),
    // Reserved, and no file should hold one. Named so that a file that does
    // says so rather than reading as a blob of minus one bytes.
    (10, "reserved"),
    (11, "reserved"),
];

/// The two runs the named values give out to. Every even number from 12 up is
/// a blob and every odd one from 13 up is text, and how far up says how long.
const SERIAL_RUN: &[(i128, i128, &str)] = &[(12, 2, "blob, {n} bytes"), (13, 2, "text, {n} bytes")];

/// What a column holds, by its serial type. The types from 12 up are lengths
/// rather than names: even is a blob, odd is text, and both count from the
/// same place. There is no remainder operator, so the parity is worked out the
/// long way round.
fn column(serial: impl Fn() -> E) -> T {
    let blob_len = serial().sub(E::lit(12)).div(E::lit(2));
    let text_len = serial().sub(E::lit(13)).div(E::lit(2));
    let text = |enc| T::text(StrLen::Fixed(text_len.clone()), enc);
    let long_form = T::switch(
        serial().sub(serial().div(E::lit(2)).mul(E::lit(2))),
        vec![
            (0, T::bytes(blob_len)),
            // The database header says which encoding its text is in.
            (
                1,
                T::switch(
                    E::field("text_encoding"),
                    vec![(2, text(Encoding::Utf16(Little))), (3, text(Encoding::Utf16(Big)))],
                    text(Encoding::Utf8),
                ),
            ),
        ],
        T::bytes(E::lit(0)),
    );
    T::switch(
        serial(),
        vec![
            // 0, 8 and 9 are the values that need no bytes: null, 0 and 1.
            (0, T::bytes(E::lit(0))),
            (1, T::Int { bits: 8, endian: Big }),
            (2, T::Int { bits: 16, endian: Big }),
            (3, T::Int { bits: 24, endian: Big }),
            (4, T::Int { bits: 32, endian: Big }),
            (5, T::Int { bits: 48, endian: Big }),
            (6, T::Int { bits: 64, endian: Big }),
            (7, T::F64(Big)),
            (8, T::bytes(E::lit(0))),
            (9, T::bytes(E::lit(0))),
            // Reserved: no bytes rather than a length worked out from a number
            // that was never a length.
            (10, T::bytes(E::lit(0))),
            (11, T::bytes(E::lit(0))),
        ],
        long_form,
    )
}

/// A row: a header of serial types, one per column, then the columns
/// themselves. The header counts its own length in its first number.
fn record() -> T {
    T::structure("Record", header_fields(vec![("columns", T::array(column(|| E::elem("types", E::idx())), E::field("types")))]))
}

/// The row of `sqlite_master` that every schema entry is. Page 1 holds these
/// and nothing else, and their five columns have names worth more than
/// `columns[4]`: the last of them is the CREATE statement as typed.
fn schema_record() -> T {
    let at = |i: i128| column(move || E::elem("types", E::lit(i)));
    T::structure_named(
        "SchemaRecord",
        "name",
        "",
        header_fields(vec![
            ("type", at(0)),
            ("name", at(1)),
            ("tbl_name", at(2)),
            ("rootpage", at(3)),
            ("sql", at(4)),
        ]),
    )
}

/// The part every record starts with: how long the header is, then one serial
/// type per column, which is what says how to read the columns after it.
fn header_fields(columns: Vec<(&str, T)>) -> Vec<(&str, T)> {
    let mut fields = vec![
        ("header_size", T::sqlite_varint()),
        (
            "types",
            T::sized(
                E::field("header_size").sub(E::size_of("header_size")),
                T::repeat(T::enum_ranged("SerialType", T::sqlite_varint(), SERIAL_TYPE, SERIAL_RUN), Until::End),
            ),
        ),
    ];
    fields.extend(columns);
    fields
}

/// How much of a payload stays on the page it was written on, and what
/// follows it when the rest does not. A payload that fits is parsed in a
/// window of its own declared size; one that does not reads as the bytes that
/// are here and the number of the page the rest went to. It is not parsed:
/// the record header itself can be cut in half by the page break, and the
/// bytes it continues into are somewhere else in the file.
///
/// The arithmetic is SQLite's own, written the long way round. There is no
/// remainder operator, so a modulo is the quotient multiplied back out and
/// taken away, the same trick the column parity uses. There is no comparison
/// either, so "P fits in X" is asked as "P divided by X plus one is nothing".
fn payload(page_type: i128, usable: E, rec: T) -> T {
    let u = || usable.clone();
    let four = || u().sub(E::lit(4));
    let p = || E::field("payload_size");
    // A table leaf leaves room for the page header and the cell around it; an
    // index page keeps a quarter of the page for itself so that a search does
    // not have to follow a chain at every step.
    let max = || match page_type {
        13 => u().sub(E::lit(35)),
        _ => u().sub(E::lit(12)).mul(E::lit(64)).div(E::lit(255)).sub(E::lit(23)),
    };
    let min = || u().sub(E::lit(12)).mul(E::lit(32)).div(E::lit(255)).sub(E::lit(23));
    let past = || p().sub(min());
    // The spilled size that keeps the overflow pages full, and the smallest
    // one that is allowed when it does not fit either.
    let k = || min().add(past().sub(past().div(four()).mul(four())));
    let on_page = T::switch(k().div(max().add(E::lit(1))), vec![(0, T::bytes(k()))], T::bytes(min()));
    let spilled =
        T::inline_structure("Spilled", vec![("on_page", on_page), ("overflow_page", T::u32(Big))]);
    T::switch(p().div(max().add(E::lit(1))), vec![(0, T::sized(p(), rec))], spilled)
}

/// The four shapes a cell takes, one per kind of page.
fn cell(page_type: i128, usable: E, rec: T) -> T {
    let payload = || payload(page_type, usable.clone(), rec.clone());
    let fields = match page_type {
        5 => vec![("left_child_page", T::u32(Big)), ("rowid", T::sqlite_varint())],
        13 => vec![
            ("payload_size", T::sqlite_varint()),
            ("rowid", T::sqlite_varint()),
            ("payload", payload()),
        ],
        2 => vec![
            ("left_child_page", T::u32(Big)),
            ("payload_size", T::sqlite_varint()),
            ("payload", payload()),
        ],
        _ => vec![("payload_size", T::sqlite_varint()), ("payload", payload())],
    };
    T::structure("Cell", fields)
}

fn btree_body(name: &str, interior: bool, page_type: i128, adjust: E, usable: E, rec: T) -> T {
    let mut fields = vec![
        ("page_type", T::enumeration("PageType", T::u8(), PAGE_TYPE)),
        ("first_freeblock", T::u16(Big)),
        ("cell_count", T::u16(Big)),
        ("cell_content_start", T::u16(Big)),
        ("fragmented_free_bytes", T::u8()),
    ];
    if interior {
        fields.push(("right_most_page", T::u32(Big)));
    }
    fields.push(("cell_pointers", T::array(T::u16(Big), E::field("cell_count"))));
    // The cells fill the rest of the page, at the offsets just read, in no
    // particular order. What none of them covers is free space.
    fields.push((
        "cells",
        T::pointer_list("cell_pointers", Anchor::Window, adjust, cell(page_type, usable, rec)),
    ));
    T::structure(name, fields)
}

/// A page that is not a b-tree, which is a page whose first byte was never a
/// type. It is an overflow page, a freelist page or a pointer map, and the
/// page says which of those it is nowhere: what points at it decides.
///
/// The header can still settle it in the common case. A file with an empty
/// freelist and no auto-vacuum has no freelist pages and no pointer maps, so
/// every page left over is the continuation of a payload too big for its own
/// page, and reads as the next page in that chain and the bytes it carries.
/// Anywhere else the honest answer is the bytes.
fn other_page() -> T {
    T::switch(
        E::field("freelist_page_count").add(E::field("vacuum_root_page")),
        vec![(
            0,
            T::structure(
                "Overflow",
                vec![("next_page", T::u32(Big)), ("content", T::bytes(E::Remaining))],
            ),
        )],
        T::bytes(E::Remaining),
    )
}

/// `adjust` shifts every cell offset, and is what page 1 needs: its offsets
/// count from the start of the file, 100 bytes before the page itself.
///
/// The type byte is peeked rather than read, because only a b-tree page has
/// one. Reading it first and switching on it afterwards would have every other
/// page in the file open with a field the format never wrote, and show the
/// byte a page number happens to start with as a page type nobody defined.
fn page(adjust: E, usable: E, rec: T) -> T {
    let body =
        |name, interior, ty| btree_body(name, interior, ty, adjust.clone(), usable.clone(), rec.clone());
    T::switch(
        E::peek(8, Big),
        vec![
            (2, body("IndexInterior", true, 2)),
            (5, body("TableInterior", true, 5)),
            (10, body("IndexLeaf", false, 10)),
            (13, body("TableLeaf", false, 13)),
        ],
        other_page(),
    )
}

pub fn sqlite() -> Template {
    database("sqlite", "SQLite", T::u32(Big))
}

/// A SELF file: a program stored as a SQLite database, one row per segment.
/// Nothing about the layout differs, so this is the same template under
/// another name, with the application id read as the four letters it is.
pub fn self_db() -> Template {
    database("self", "SELF", T::magic(b"SELF"))
}

fn database(name: &str, root: &str, application_id: T) -> Template {
    // A page size of 1 means 65536: the field is two bytes and cannot hold it.
    // There is no conditional expression, so a switch says it instead.
    // What of a page a payload may use: the page less whatever the header
    // holds back at the end of every one of them.
    let usable = |size: &E| size.clone().sub(E::field("reserved_space"));
    let first = |size: E| {
        T::sized(size.clone().sub(E::lit(100)), page(E::lit(-100), usable(&size), schema_record()))
    };
    let rest =
        |size: E| T::repeat(T::sized(size.clone(), page(E::lit(0), usable(&size), record())), Until::End);
    Template::new(
        name,
        T::structure(
            root,
            vec![
                ("magic", T::magic(b"SQLite format 3\0")),
                ("page_size", T::u16(Big)),
                ("write_version", T::enumeration("WriteVersion", T::u8(), WRITE_VERSION)),
                ("read_version", T::enumeration("ReadVersion", T::u8(), WRITE_VERSION)),
                ("reserved_space", T::u8()),
                ("max_payload_fraction", T::u8()),
                ("min_payload_fraction", T::u8()),
                ("leaf_payload_fraction", T::u8()),
                ("change_counter", T::u32(Big)),
                // Worth trusting only when it matches the change counter, so
                // the page run below reads to the end of the file instead.
                ("page_count", T::u32(Big)),
                ("first_freelist_page", T::u32(Big)),
                ("freelist_page_count", T::u32(Big)),
                ("schema_cookie", T::u32(Big)),
                ("schema_format", T::u32(Big)),
                ("default_cache_size", T::i32(Big)),
                ("vacuum_root_page", T::u32(Big)),
                ("text_encoding", T::enumeration("TextEncoding", T::u32(Big), TEXT_ENCODING)),
                ("user_version", T::i32(Big)),
                ("incremental_vacuum", T::u32(Big)),
                ("application_id", application_id),
                ("reserved", T::bytes(E::lit(20))),
                ("version_valid_for", T::u32(Big)),
                ("sqlite_version", T::u32(Big)),
                (
                    "page1",
                    T::switch(E::field("page_size"), vec![(1, first(E::lit(65536)))], first(E::field("page_size"))),
                ),
                (
                    "pages",
                    T::switch(E::field("page_size"), vec![(1, rest(E::lit(65536)))], rest(E::field("page_size"))),
                ),
            ],
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    const PAGE: usize = 512;
    // Field indices into the root: the page size, the text encoding, page 1,
    // and the pages after it.
    const PAGE_SIZE: usize = 1;
    const TEXT_ENCODING: usize = 16;
    const APPLICATION_ID: usize = 19;
    const PAGE1: usize = 23;
    const PAGES: usize = 24;
    // Field indices inside a b-tree leaf page. A page is its own struct: the
    // type byte is peeked to choose which one, and read again inside it.
    const CELL_COUNT: usize = 2;
    const POINTERS: usize = 5;
    const CELLS: usize = 6;

    /// One row: no id (the row id stands in for it), a small number, and some
    /// text. The header names one serial type per column and counts itself.
    fn record(n: i8, text: &str) -> Vec<u8> {
        let mut r = vec![4, 0, 1, 13 + 2 * text.len() as u8];
        r.push(n as u8);
        r.extend_from_slice(text.as_bytes());
        r
    }

    /// A table leaf cell: how long the payload is, which row it is, the row.
    fn cell_bytes(rowid: u8, n: i8, text: &str) -> Vec<u8> {
        let rec = record(n, text);
        let mut c = vec![rec.len() as u8, rowid];
        c.extend_from_slice(&rec);
        c
    }

    /// A table leaf page of `len` bytes holding `cells`, laid out the way
    /// SQLite lays them out: pointers from the front, cells from the back.
    /// `base` is what the offsets count from, which for page 1 is 100 bytes
    /// before the page itself.
    fn leaf_page(cells: &[Vec<u8>], len: usize, base: usize) -> Vec<u8> {
        let mut p = vec![13u8];
        let mut content = len;
        let mut ptrs = Vec::new();
        let mut tail = vec![0u8; len];
        for c in cells {
            content -= c.len();
            tail[content..content + c.len()].copy_from_slice(c);
            ptrs.push((content + base) as u16);
        }
        p.extend_from_slice(&0u16.to_be_bytes()); // first freeblock
        p.extend_from_slice(&(cells.len() as u16).to_be_bytes());
        p.extend_from_slice(&((content + base) as u16).to_be_bytes());
        p.push(0); // fragmented free bytes
        for ptr in &ptrs {
            p.extend_from_slice(&ptr.to_be_bytes());
        }
        let head = p.len();
        p.extend_from_slice(&tail[head..]);
        p
    }

    fn header(page_size: usize) -> Vec<u8> {
        let mut b = b"SQLite format 3\0".to_vec();
        b.extend_from_slice(&(page_size as u16).to_be_bytes());
        b.extend_from_slice(&[1, 1, 0, 64, 32, 32]);
        b.extend_from_slice(&7u32.to_be_bytes()); // change counter
        b.extend_from_slice(&2u32.to_be_bytes()); // page count
        for _ in 0..4 {
            b.extend_from_slice(&0u32.to_be_bytes()); // freelist, cookie, schema format
        }
        b.extend_from_slice(&0u32.to_be_bytes()); // default cache size
        b.extend_from_slice(&0u32.to_be_bytes()); // vacuum root
        b.extend_from_slice(&1u32.to_be_bytes()); // text encoding: utf8
        for _ in 0..3 {
            b.extend_from_slice(&0u32.to_be_bytes()); // user version, vacuum, app id
        }
        b.extend_from_slice(&[0; 20]);
        b.extend_from_slice(&7u32.to_be_bytes());
        b.extend_from_slice(&3_045_000u32.to_be_bytes());
        assert_eq!(b.len(), 100);
        b
    }

    /// One row of `sqlite_master`: what it is, what it is called, which table
    /// it belongs to, which page it starts on, and how it was declared.
    fn schema_cell(rowid: u8, name: &str, root: u8, sql: &str) -> Vec<u8> {
        let text = |s: &str| 13 + 2 * s.len() as u8;
        let mut r = vec![6, text("table"), text(name), text(name), 1, text(sql)];
        r.extend_from_slice(b"table");
        r.extend_from_slice(name.as_bytes());
        r.extend_from_slice(name.as_bytes());
        r.push(root);
        r.extend_from_slice(sql.as_bytes());
        let mut c = vec![r.len() as u8, rowid];
        c.extend_from_slice(&r);
        c
    }

    /// Two pages, laid out the way SQLite lays a database out: the schema on
    /// page 1, and the table it describes on the page after it.
    fn db() -> Vec<u8> {
        let mut b = header(PAGE);
        b.extend_from_slice(&leaf_page(&[schema_cell(1, "m", 2, "CREATE TABLE m(x)")], PAGE - 100, 100));
        let cells = [cell_bytes(1, 42, "hi"), cell_bytes(2, -3, "there")];
        b.extend_from_slice(&leaf_page(&cells, PAGE, 0));
        b
    }

    /// The same database with the four letters that say the rows are a
    /// program: `SELF` where SQLite keeps the application id.
    fn program() -> Vec<u8> {
        let mut b = db();
        b[68..72].copy_from_slice(b"SELF");
        b
    }

    #[test]
    fn a_program_kept_in_a_database_is_told_from_a_plain_one() {
        let bytes = program();
        assert_eq!(crate::formats::sniff(&bytes, bytes.len() as u64), Some("self"));
        assert_eq!(crate::formats::sniff(&db(), PAGE as u64 * 2), Some("sqlite"));

        // Same layout, read under the name of what it holds. The application
        // id is the four letters rather than the number they add up to.
        let d = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(self_db());
        assert_eq!(ev.node(&d, &[]).unwrap().type_name, "SELF");
        let id = ev.node(&d, &[APPLICATION_ID]).unwrap();
        assert_eq!(id.value, Value::Magic { ok: true, bytes: b"SELF".to_vec() });
        assert_eq!(ev.node(&d, &[PAGE1, CELL_COUNT]).unwrap().value, Value::UInt(1));
    }

    #[test]
    fn header_and_pages() {
        let d = Document::new(MemSource(db()));
        let mut ev = Evaluator::new(sqlite());
        assert_eq!(ev.node(&d, &[PAGE_SIZE]).unwrap().value, Value::UInt(PAGE as u128));
        let enc = ev.node(&d, &[TEXT_ENCODING]).unwrap();
        assert_eq!(enc.value, Value::Enum { raw: 1, name: Some("utf8".into()), hex: false });
        // Page 1 starts after the 100-byte header, and is a table leaf.
        let kind = ev.node(&d, &[PAGE1, 0]).unwrap();
        assert_eq!(kind.offset_bits, 100 * 8);
        assert_eq!(kind.value, Value::Enum { raw: 13, name: Some("table leaf".into()), hex: false });
        assert_eq!(ev.node(&d, &[PAGE1, CELL_COUNT]).unwrap().value, Value::UInt(1));
        assert_eq!(ev.node(&d, &[PAGE1, POINTERS]).unwrap().child_count, 1);
        // One more page, starting at a page boundary.
        assert_eq!(ev.node(&d, &[PAGES]).unwrap().child_count, 1);
        assert_eq!(ev.node(&d, &[PAGES, 0, 0]).unwrap().offset_bits, PAGE as u64 * 8);
    }

    #[test]
    fn the_schema_row_reads_as_the_five_columns_sqlite_keeps_there() {
        let d = Document::new(MemSource(db()));
        let mut ev = Evaluator::new(sqlite());
        let row = [PAGE1, CELLS, 0, 2];
        let col = |i: usize| [row.as_slice(), &[i]].concat();
        assert_eq!(ev.node(&d, &col(2)).unwrap().value, Value::Str("table".into()));
        assert_eq!(ev.node(&d, &col(3)).unwrap().name, "name");
        assert_eq!(ev.node(&d, &col(3)).unwrap().value, Value::Str("m".into()));
        assert_eq!(ev.node(&d, &col(5)).unwrap().value, Value::Int(2)); // rootpage
        assert_eq!(ev.node(&d, &col(6)).unwrap().value, Value::Str("CREATE TABLE m(x)".into()));
    }

    #[test]
    fn a_row_reads_as_its_columns() {
        let d = Document::new(MemSource(db()));
        let mut ev = Evaluator::new(sqlite());
        let cells = ev.node(&d, &[PAGES, 0, CELLS]).unwrap();
        assert_eq!(cells.child_count, 2);
        // The second row sits before the first one in the file, and reading it
        // means following its offset rather than walking forward.
        let second = ev.node(&d, &[PAGES, 0, CELLS, 1]).unwrap();
        let first = ev.node(&d, &[PAGES, 0, CELLS, 0]).unwrap();
        assert!(second.offset_bits < first.offset_bits);
        assert_eq!(ev.node(&d, &[PAGES, 0, CELLS, 1, 1]).unwrap().value, Value::Int(2)); // row id
        let types = ev.node(&d, &[PAGES, 0, CELLS, 0, 2, 1]).unwrap();
        assert_eq!(types.child_count, 3);
        assert_eq!(
            ev.node(&d, &[PAGES, 0, CELLS, 0, 2, 1, 0]).unwrap().value,
            Value::Enum { raw: 0, name: Some("null".into()), hex: false }
        );
        let columns = ev.node(&d, &[PAGES, 0, CELLS, 0, 2, 2]).unwrap();
        assert_eq!(columns.child_count, 3);
        // A column with no bytes, a number, and text, each typed by the header.
        assert_eq!(ev.node(&d, &[PAGES, 0, CELLS, 0, 2, 2, 0]).unwrap().size_bits, 0);
        assert_eq!(ev.node(&d, &[PAGES, 0, CELLS, 0, 2, 2, 1]).unwrap().value, Value::Int(42));
        assert_eq!(ev.node(&d, &[PAGES, 0, CELLS, 0, 2, 2, 2]).unwrap().value, Value::Str("hi".into()));
        assert_eq!(ev.node(&d, &[PAGES, 0, CELLS, 1, 2, 2, 1]).unwrap().value, Value::Int(-3));
        assert_eq!(ev.node(&d, &[PAGES, 0, CELLS, 1, 2, 2, 2]).unwrap().value, Value::Str("there".into()));
    }

    #[test]
    fn a_serial_type_past_the_named_ones_is_named_by_what_it_counts() {
        let d = Document::new(MemSource(db()));
        let mut ev = Evaluator::new(sqlite());
        // The third column is the two letters of "hi", so its serial type is
        // 13 plus twice the length, and the length is the whole of its name.
        let ty = ev.node(&d, &[PAGES, 0, CELLS, 0, 2, 1, 2]).unwrap();
        assert_eq!(ty.value, Value::Enum { raw: 17, name: Some("text, 2 bytes".into()), hex: false });
    }

    #[test]
    fn the_cursor_lands_in_the_row_it_is_standing_in() {
        let d = Document::new(MemSource(db()));
        let mut ev = Evaluator::new(sqlite());
        let text = ev.node(&d, &[PAGES, 0, CELLS, 0, 2, 2, 2]).unwrap();
        assert_eq!(ev.locate(&d, text.offset_bits).unwrap(), vec![PAGES, 0, CELLS, 0, 2, 2, 2]);
        // Free space between the pointer array and the first row belongs to no
        // field, and stops where that row starts.
        let from = (PAGE + 20) as u64 * 8;
        let free = ev.spans(&d, from, 2 * PAGE as u64 * 8, 4).unwrap();
        assert!(free[0].gap);
        assert_eq!(free[0].offset_bits, from);
        let first_cell = ev.node(&d, &[PAGES, 0, CELLS, 1]).unwrap();
        assert_eq!(free[0].offset_bits + free[0].size_bits, first_cell.offset_bits);
        assert!(!free[1].gap);
    }

    #[test]
    fn text_columns_read_in_the_encoding_the_header_names() {
        let mut cells = header(PAGE);
        let mut utf16 = vec![4u8, 0, 1, 13 + 2 * 4];
        utf16.push(9);
        utf16.extend_from_slice(&[b'h', 0, b'i', 0]);
        let cell = [vec![utf16.len() as u8, 1], utf16].concat();
        cells.extend_from_slice(&leaf_page(&[], PAGE - 100, 100));
        cells.extend_from_slice(&leaf_page(&[cell], PAGE, 0));
        cells[59] = 2; // text encoding: utf16le
        let d = Document::new(MemSource(cells));
        let mut ev = Evaluator::new(sqlite());
        let col = ev.node(&d, &[PAGES, 0, CELLS, 0, 2, 2, 2]).unwrap();
        assert_eq!(col.value, Value::Str("hi".into()));
        assert_eq!(col.type_name, "utf16le[]");
    }

    #[test]
    fn an_interior_page_reads_its_child_pages() {
        let mut b = header(PAGE);
        b.extend_from_slice(&leaf_page(&[], PAGE - 100, 100));
        // A table interior page: a rightmost child, then one cell holding the
        // child page to its left and the last row id in it.
        let mut p = vec![5u8];
        p.extend_from_slice(&0u16.to_be_bytes()); // first freeblock
        p.extend_from_slice(&1u16.to_be_bytes()); // one cell
        p.extend_from_slice(&(PAGE as u16 - 5).to_be_bytes()); // cell content start
        p.push(0); // fragmented free bytes
        p.extend_from_slice(&9u32.to_be_bytes()); // rightmost child page
        p.extend_from_slice(&(PAGE as u16 - 5).to_be_bytes()); // the one cell pointer
        p.resize(PAGE - 5, 0);
        p.extend_from_slice(&4u32.to_be_bytes()); // left child page
        p.push(30); // last row id on it
        b.extend_from_slice(&p);
        let d = Document::new(MemSource(b));
        let mut ev = Evaluator::new(sqlite());
        let kind = ev.node(&d, &[PAGES, 0, 0]).unwrap();
        assert_eq!(kind.value, Value::Enum { raw: 5, name: Some("table interior".into()), hex: false });
        assert_eq!(ev.node(&d, &[PAGES, 0, 5]).unwrap().value, Value::UInt(9)); // rightmost
        assert_eq!(ev.node(&d, &[PAGES, 0, 7, 0, 0]).unwrap().value, Value::UInt(4)); // left child
        assert_eq!(ev.node(&d, &[PAGES, 0, 7, 0, 1]).unwrap().value, Value::Int(30)); // row id
    }

    #[test]
    fn a_page_that_is_not_a_btree_keeps_the_byte_a_type_would_have_taken() {
        let mut b = db();
        // Not a b-tree page, and its first byte is the top of a page number
        // rather than a type. A file with a freelist cannot say which kind of
        // page it is, so all of it reads as bytes, first byte included.
        b[PAGE] = 0;
        b[39] = 1; // one page on the freelist
        let d = Document::new(MemSource(b));
        let mut ev = Evaluator::new(sqlite());
        let page = ev.node(&d, &[PAGES, 0]).unwrap();
        assert_eq!(page.type_name, "bytes[]");
        assert!(matches!(page.value, Value::Bytes { len, .. } if len == PAGE as u64));
    }

    #[test]
    fn a_leftover_page_in_a_file_with_no_freelist_is_an_overflow_page() {
        let mut b = db();
        b[PAGE] = 0;
        // Nothing on the freelist and no auto-vacuum, so there is nothing else
        // this page could be: it holds the rest of a payload, and the number
        // of the page holding the rest of that.
        let d = Document::new(MemSource(b));
        let mut ev = Evaluator::new(sqlite());
        assert_eq!(ev.node(&d, &[PAGES, 0]).unwrap().type_name, "Overflow");
        assert_eq!(ev.node(&d, &[PAGES, 0, 0]).unwrap().name, "next_page");
    }

    #[test]
    fn a_payload_that_does_not_fit_reads_what_stayed_and_where_the_rest_went() {
        // A payload of 600 bytes on a page of 512. SQLite keeps 39 bytes at
        // least and as many as 477, and the size that fills the overflow pages
        // exactly is 92, so 92 bytes stay and a page number follows them.
        let mut cell = vec![0x84, 0x58, 1]; // payload size 600, row id 1
        cell.extend(std::iter::repeat(b'A').take(92));
        cell.extend_from_slice(&7u32.to_be_bytes());
        let mut b = header(PAGE);
        b.extend_from_slice(&leaf_page(&[], PAGE - 100, 100));
        b.extend_from_slice(&leaf_page(&[cell], PAGE, 0));
        let d = Document::new(MemSource(b));
        let mut ev = Evaluator::new(sqlite());
        let spilled = [PAGES, 0, CELLS, 0, 2];
        let on_page = ev.node(&d, &[spilled.as_slice(), &[0]].concat()).unwrap();
        assert_eq!(on_page.name, "on_page");
        assert!(matches!(on_page.value, Value::Bytes { len, .. } if len == 92));
        let next = ev.node(&d, &[spilled.as_slice(), &[1]].concat()).unwrap();
        assert_eq!(next.name, "overflow_page");
        assert_eq!(next.value, Value::UInt(7));
    }

    #[test]
    fn a_payload_that_runs_past_its_page_is_an_error_for_that_row_alone() {
        let mut b = db();
        // The first row claims a payload longer than the page can hold. It is
        // the last cell in the file, since the cells fill a page from the back.
        let at = b.len() - 9;
        b[at] = 250;
        let d = Document::new(MemSource(b));
        let mut ev = Evaluator::new(sqlite());
        assert!(ev.node(&d, &[PAGES, 0, CELLS, 0, 2]).is_err());
        assert_eq!(ev.node(&d, &[PAGES, 0, CELLS, 1, 2, 2, 2]).unwrap().value, Value::Str("there".into()));
    }
}
