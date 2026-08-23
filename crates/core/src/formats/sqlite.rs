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

use crate::template::{Endian::*, Expr as E, Template, Ty as T, Until};

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

fn btree_body(name: &str, interior: bool) -> T {
    let mut fields = vec![
        ("first_freeblock", T::u16(Big)),
        ("cell_count", T::u16(Big)),
        ("cell_content_start", T::u16(Big)),
        ("fragmented_free_bytes", T::u8()),
    ];
    if interior {
        fields.push(("right_most_page", T::u32(Big)));
    }
    // Offsets from the start of the page. On page 1 that is the start of the
    // file, not the start of this field, since the file header comes first.
    fields.push(("cell_pointers", T::array(T::u16(Big), E::field("cell_count"))));
    fields.push(("cells_and_free_space", T::bytes(E::Remaining)));
    T::structure(name, fields)
}

fn page() -> T {
    let interior = btree_body("BTreeInterior", true);
    let leaf = btree_body("BTreeLeaf", false);
    T::structure(
        "Page",
        vec![
            ("page_type", T::enumeration("PageType", T::u8(), PAGE_TYPE)),
            (
                "body",
                T::switch(
                    E::field("page_type"),
                    vec![(2, interior.clone()), (5, interior), (10, leaf.clone()), (13, leaf)],
                    T::bytes(E::Remaining),
                ),
            ),
        ],
    )
}

pub fn sqlite() -> Template {
    // A page size of 1 means 65536: the field is two bytes and cannot hold it.
    // There is no conditional expression, so a switch says it instead.
    let first = |size: E| T::sized(size.sub(E::lit(100)), page());
    let rest = |size: E| T::repeat(T::sized(size, page()), Until::End);
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

    /// A table leaf page with `cells` pointers in it, padded to `len`.
    fn leaf_page(cells: &[u16], len: usize) -> Vec<u8> {
        let mut p = vec![13u8];
        p.extend_from_slice(&0u16.to_be_bytes()); // first freeblock
        p.extend_from_slice(&(cells.len() as u16).to_be_bytes());
        p.extend_from_slice(&(len as u16).to_be_bytes()); // cell content start
        p.push(0); // fragmented free bytes
        for c in cells {
            p.extend_from_slice(&c.to_be_bytes());
        }
        p.resize(len, 0);
        p
    }

    fn db() -> Vec<u8> {
        let mut b = b"SQLite format 3\0".to_vec();
        b.extend_from_slice(&(PAGE as u16).to_be_bytes());
        b.extend_from_slice(&[1, 1, 0, 64, 32, 32]);
        b.extend_from_slice(&7u32.to_be_bytes()); // change counter
        b.extend_from_slice(&2u32.to_be_bytes()); // page count
        for _ in 0..4 {
            b.extend_from_slice(&0u32.to_be_bytes()); // freelist, schema cookie, schema format
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
        b.extend_from_slice(&leaf_page(&[400, 450], PAGE - 100));
        b.extend_from_slice(&leaf_page(&[500], PAGE));
        b
    }

    #[test]
    fn header_and_pages() {
        let d = Document::new(MemSource(db()));
        let mut ev = Evaluator::new(sqlite());
        assert_eq!(ev.node(&d, &[1]).unwrap().value, Value::UInt(PAGE as u128));
        let enc = ev.node(&d, &[16]).unwrap();
        assert_eq!(enc.value, Value::Enum { raw: 1, name: Some("utf8".into()), hex: false });
        // Page 1 starts after the 100-byte header, and is a table leaf.
        let kind = ev.node(&d, &[23, 0]).unwrap();
        assert_eq!(kind.offset_bits, 100 * 8);
        assert_eq!(kind.value, Value::Enum { raw: 13, name: Some("table leaf".into()), hex: false });
        assert_eq!(ev.node(&d, &[23, 1, 1]).unwrap().value, Value::UInt(2));
        let pointers = ev.node(&d, &[23, 1, 4]).unwrap();
        assert_eq!(pointers.child_count, 2);
        assert_eq!(ev.node(&d, &[23, 1, 4, 1]).unwrap().value, Value::UInt(450));
        // One more page, starting at a page boundary.
        assert_eq!(ev.node(&d, &[24]).unwrap().child_count, 1);
        assert_eq!(ev.node(&d, &[24, 0, 0]).unwrap().offset_bits, PAGE as u64 * 8);
    }

    #[test]
    fn a_page_that_is_not_a_btree_reads_as_bytes() {
        let mut b = db();
        b[PAGE] = 0; // a freelist trunk page: the type byte means nothing here
        let d = Document::new(MemSource(b));
        let mut ev = Evaluator::new(sqlite());
        let body = ev.node(&d, &[24, 0, 1]).unwrap();
        assert_eq!(body.value, Value::Bytes { len: PAGE as u64 - 1, preview: vec![0, 0, 0, 1, 2, 0, 0, 1, 244, 0, 0, 0, 0, 0, 0, 0] });
    }
}
