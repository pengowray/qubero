//! SQLite 3: the 100-byte database header, then a run of fixed-size pages.
//!
//! The header's own fields are flattened into the root struct rather than
//! grouped, because an expression can only name a field in its own struct or
//! an enclosing one, and the page size decides how big every page is.
//!
//! Where this stops: a b-tree page keeps its cells at the offsets in its cell
//! pointer array, counting from the start of the page, and the IR has no way
//! to say "the thing at this offset". So a page reads down to its pointer
//! array and the rest of it is one run of bytes. Reading the rows themselves
//! needs that, plus SQLite's own varint (nine bytes, the last contributing all
//! eight bits, which `Vlq` cannot stand in for) and a column list whose types
//! come from an array read earlier, element by element, which `Expr::Ref`
//! cannot reach.

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
];

/// What a column holds, by its serial type. The types from 12 up are lengths
/// rather than names: even is a blob, odd is text, and both count from the
/// same place. There is no remainder operator, so the parity is worked out the
/// long way round.
fn column() -> T {
    let serial = || E::elem("types", E::idx());
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
        ],
        long_form,
    )
}

/// A row: a header of serial types, one per column, then the columns
/// themselves. The header counts its own length in its first number.
fn record() -> T {
    T::structure(
        "Record",
        vec![
            ("header_size", T::sqlite_varint()),
            (
                "types",
                T::sized(
                    E::field("header_size").sub(E::size_of("header_size")),
                    T::repeat(T::enumeration("SerialType", T::sqlite_varint(), SERIAL_TYPE), Until::End),
                ),
            ),
            ("columns", T::array(column(), E::field("types"))),
        ],
    )
}

/// The four shapes a cell takes, one per kind of page. A payload is parsed in
/// a window of its own declared size, so a spilled one errors there and
/// nowhere else.
fn cell(page_type: i128) -> T {
    let payload = || T::sized(E::field("payload_size"), record());
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

fn btree_body(name: &str, interior: bool, page_type: i128, adjust: E) -> T {
    let mut fields = vec![
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
    fields.push(("cells", T::pointer_list("cell_pointers", Anchor::Window, adjust, cell(page_type))));
    T::structure(name, fields)
}

/// `adjust` shifts every cell offset, and is what page 1 needs: its offsets
/// count from the start of the file, 100 bytes before the page itself.
fn page(adjust: E) -> T {
    let body = |name, interior, ty| btree_body(name, interior, ty, adjust.clone());
    T::structure(
        "Page",
        vec![
            ("page_type", T::enumeration("PageType", T::u8(), PAGE_TYPE)),
            (
                "body",
                T::switch(
                    E::field("page_type"),
                    vec![
                        (2, body("IndexInterior", true, 2)),
                        (5, body("TableInterior", true, 5)),
                        (10, body("IndexLeaf", false, 10)),
                        (13, body("TableLeaf", false, 13)),
                    ],
                    T::bytes(E::Remaining),
                ),
            ),
        ],
    )
}

pub fn sqlite() -> Template {
    // A page size of 1 means 65536: the field is two bytes and cannot hold it.
    // There is no conditional expression, so a switch says it instead.
    let first = |size: E| T::sized(size.sub(E::lit(100)), page(E::lit(-100)));
    let rest = |size: E| T::repeat(T::sized(size, page(E::lit(0))), Until::End);
    Template::new(
        "sqlite",
        T::structure(
            "SQLite",
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
                ("application_id", T::u32(Big)),
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
    const PAGE1: usize = 23;
    const PAGES: usize = 24;
    // Field indices inside a b-tree page: the header is one struct in, and the
    // cells are the last field of it.
    const BODY: usize = 1;
    const CELL_COUNT: usize = 1;
    const POINTERS: usize = 4;
    const CELLS: usize = 5;

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

    /// Two pages: rows on page 1, and an empty page after it.
    fn db() -> Vec<u8> {
        let mut b = header(PAGE);
        let cells = [cell_bytes(1, 42, "hi"), cell_bytes(2, -3, "there")];
        b.extend_from_slice(&leaf_page(&cells, PAGE - 100, 100));
        b.extend_from_slice(&leaf_page(&[], PAGE, 0));
        b
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
        assert_eq!(ev.node(&d, &[PAGE1, BODY, CELL_COUNT]).unwrap().value, Value::UInt(2));
        assert_eq!(ev.node(&d, &[PAGE1, BODY, POINTERS]).unwrap().child_count, 2);
        // One more page, starting at a page boundary.
        assert_eq!(ev.node(&d, &[PAGES]).unwrap().child_count, 1);
        assert_eq!(ev.node(&d, &[PAGES, 0, 0]).unwrap().offset_bits, PAGE as u64 * 8);
    }

    #[test]
    fn a_row_reads_as_its_columns() {
        let d = Document::new(MemSource(db()));
        let mut ev = Evaluator::new(sqlite());
        let cells = ev.node(&d, &[PAGE1, BODY, CELLS]).unwrap();
        assert_eq!(cells.child_count, 2);
        // The second row sits before the first one in the file, and reading it
        // means following its offset rather than walking forward.
        let second = ev.node(&d, &[PAGE1, BODY, CELLS, 1]).unwrap();
        let first = ev.node(&d, &[PAGE1, BODY, CELLS, 0]).unwrap();
        assert!(second.offset_bits < first.offset_bits);
        assert_eq!(ev.node(&d, &[PAGE1, BODY, CELLS, 1, 1]).unwrap().value, Value::Int(2)); // row id
        let types = ev.node(&d, &[PAGE1, BODY, CELLS, 0, 2, 1]).unwrap();
        assert_eq!(types.child_count, 3);
        assert_eq!(
            ev.node(&d, &[PAGE1, BODY, CELLS, 0, 2, 1, 0]).unwrap().value,
            Value::Enum { raw: 0, name: Some("null".into()), hex: false }
        );
        let columns = ev.node(&d, &[PAGE1, BODY, CELLS, 0, 2, 2]).unwrap();
        assert_eq!(columns.child_count, 3);
        // A column with no bytes, a number, and text, each typed by the header.
        assert_eq!(ev.node(&d, &[PAGE1, BODY, CELLS, 0, 2, 2, 0]).unwrap().size_bits, 0);
        assert_eq!(ev.node(&d, &[PAGE1, BODY, CELLS, 0, 2, 2, 1]).unwrap().value, Value::Int(42));
        assert_eq!(ev.node(&d, &[PAGE1, BODY, CELLS, 0, 2, 2, 2]).unwrap().value, Value::Str("hi".into()));
        assert_eq!(ev.node(&d, &[PAGE1, BODY, CELLS, 1, 2, 2, 1]).unwrap().value, Value::Int(-3));
        assert_eq!(ev.node(&d, &[PAGE1, BODY, CELLS, 1, 2, 2, 2]).unwrap().value, Value::Str("there".into()));
    }

    #[test]
    fn the_cursor_lands_in_the_row_it_is_standing_in() {
        let d = Document::new(MemSource(db()));
        let mut ev = Evaluator::new(sqlite());
        let text = ev.node(&d, &[PAGE1, BODY, CELLS, 0, 2, 2, 2]).unwrap();
        assert_eq!(ev.locate(&d, text.offset_bits).unwrap(), vec![PAGE1, BODY, CELLS, 0, 2, 2, 2]);
        // Free space between the pointer array and the first row belongs to no
        // field, and stops where that row starts.
        let free = ev.spans(&d, 120 * 8, PAGE as u64 * 8, 4).unwrap();
        assert!(free[0].gap);
        assert_eq!(free[0].offset_bits, 120 * 8);
        let first_cell = ev.node(&d, &[PAGE1, BODY, CELLS, 1]).unwrap();
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
        cells.extend_from_slice(&leaf_page(&[cell], PAGE - 100, 100));
        cells[59] = 2; // text encoding: utf16le
        let d = Document::new(MemSource(cells));
        let mut ev = Evaluator::new(sqlite());
        let col = ev.node(&d, &[PAGE1, BODY, CELLS, 0, 2, 2, 2]).unwrap();
        assert_eq!(col.value, Value::Str("hi".into()));
        assert_eq!(col.type_name, "utf16le[]");
    }

    #[test]
    fn a_page_that_is_not_a_btree_reads_as_bytes() {
        let mut b = db();
        b[PAGE] = 0; // a freelist trunk page: the type byte means nothing here
        let d = Document::new(MemSource(b));
        let mut ev = Evaluator::new(sqlite());
        let body = ev.node(&d, &[PAGES, 0, 1]).unwrap();
        assert_eq!(body.type_name, "bytes[]");
        assert!(matches!(body.value, Value::Bytes { len, .. } if len == PAGE as u64 - 1));
    }

    #[test]
    fn a_payload_too_big_for_its_page_is_an_error_for_that_row_alone() {
        let mut b = db();
        // The first row claims a payload longer than the page can hold.
        let at = b.len() - PAGE - 9;
        b[at] = 250;
        let d = Document::new(MemSource(b));
        let mut ev = Evaluator::new(sqlite());
        assert!(ev.node(&d, &[PAGE1, BODY, CELLS, 0, 2]).is_err());
        assert_eq!(ev.node(&d, &[PAGE1, BODY, CELLS, 1, 2, 2, 2]).unwrap().value, Value::Str("there".into()));
    }
}
