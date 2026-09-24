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
//! The type byte is not asked first, though. A page on the freelist keeps
//! whatever it held before it was freed, so a table leaf deleted from the tree
//! still starts with 13, and read by its first byte it would show rows the
//! database no longer has. The freelist is read before the pages for that
//! reason: a page it names is free whatever its first byte says. That much of
//! "what points at a page decides what it is" the template can say, because
//! the freelist is reached from the header and the header comes first.
//!
//! The rest of it the template cannot say. An overflow page is named by a cell
//! on another page, or by the overflow page before it, and those can be
//! anywhere in the run of pages, after the page asking as often as before it.
//! A list element can search the elements before it and a list declared
//! earlier, and nothing else, so no page can ask whether a later one points at
//! it. An overflow page is known here by elimination instead: see
//! [`other_page`]. Saying it properly needs two things the IR does not have:
//!
//! - a number that is a reference to an element of a list, so that
//!   `overflow_page` says it names page n of `pages` rather than being a `u32`;
//! - a list whose elements are typed by the references that reach them, found
//!   before any element is typed. Reaching is transitive, since an overflow
//!   page is reached from a cell or from the overflow page before it, so this
//!   is a walk from the roots the schema names, not a lookup.
//!
//! Elimination gives the same answer for a sound file. Where it differs is a
//! page nothing points at, which SQLite's integrity check reports as never
//! used: here it reads as an overflow page, where a walk would have found it
//! unreached.
//!
//! Where the template stops: a payload too big for its page reads as the bytes
//! that stayed and the number of the page the rest went to, and stops there.
//! The rest is on a chain of pages elsewhere in the file, and a record cut
//! across a page break is not something a field placed at an offset can read.
//!
//! [`sqlite_overflow`](super::sqlite_overflow) is the rest of the answer. It
//! follows that chain and says which runs of the file the row is made of, which
//! [`Gathered`](crate::gather::Gathered) reads as one stream. The template
//! still stops where it stops, because that is what a template can honestly
//! say; the walk is a separate thing a reader asks for.

use crate::template::{Anchor, Encoding, Endian::*, Expr as E, Step, StrLen, Template, Ty as T, Until};

/// What kind of b-tree node a page holds. A page that is not a b-tree has no
/// type byte: a freelist page, an overflow page, or a pointer map. Those are
/// told apart by what points at them, not by this byte.
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

/// Which text encoding a column reads in.
///
/// Normally the database header's own field, which every record can see by
/// looking outwards. A record read on its own cannot: a row assembled from the
/// pages it spilled onto is a stream of its own, with no header above it for a
/// field to reach, so the encoding is handed to it as the number the header
/// held. See [`record_encoded`].
fn encoding_of(encoding: Option<i128>) -> E {
    match encoding {
        Some(n) => E::lit(n),
        None => E::field("text_encoding"),
    }
}

/// What a column holds, by its serial type. The types from 12 up are lengths
/// rather than names: even is a blob, odd is text, and both count from the
/// same place. There is no remainder operator, so the parity is worked out the
/// long way round.
fn column(encoding: Option<i128>, serial: impl Fn() -> E) -> T {
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
                    encoding_of(encoding),
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
            // These two are values, not absences. Read as no bytes they come
            // out blank, which in a table of rows is indistinguishable from a
            // null and is wrong for every boolean column in every database.
            // Computed says the value without inventing a byte for it.
            (8, T::computed(E::lit(0))),
            (9, T::computed(E::lit(1))),
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
    columns_of(None)
}

/// A record read on its own, with the text encoding given rather than looked
/// up. This is what a row assembled from the pages it spilled onto is parsed
/// with: those bytes are a stream of their own, and nothing above them holds
/// the database header for a field to reach.
pub(crate) fn record_encoded(encoding: i128) -> T {
    columns_of(Some(encoding))
}

fn columns_of(encoding: Option<i128>) -> T {
    let column = column(encoding, || E::elem("types", E::idx()));
    T::structure("Record", header_fields(vec![("columns", T::array(column, E::field("types")))]))
}

/// The row of `sqlite_master` that every schema entry is. Page 1 holds these
/// and nothing else, and their five columns have names worth more than
/// `columns[4]`: the last of them is the CREATE statement as typed.
fn schema_record() -> T {
    let at = |i: i128| column(None, move || E::elem("types", E::lit(i)));
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
    // Marked as packed so that standing on it opens the row: the rest of it is
    // on a chain of pages this field can only name, and
    // [`sqlite_overflow`](super::sqlite_overflow) is what follows the chain.
    let spilled = T::inline_structure("Spilled", vec![("on_page", on_page), ("overflow_page", T::u32(Big))])
        .packed_as(super::sqlite_overflow::PACKING);
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
    // The rest of the page header is bookkeeping for the free space between
    // the cell pointers and the cells. Nothing else in the template reads any
    // of it, so the shapes cannot tell it apart from a field worth reading;
    // the listing folds it behind the page all the same.
    T::structure(name, fields).machinery(&["page_type", "first_freeblock", "cell_content_start", "fragmented_free_bytes"])
}

/// The page size in bytes, as the header field that works it out.
fn page_bytes() -> E {
    E::field("page_bytes")
}

/// Where page `n` starts, in bytes from the start of the file. Pages count
/// from 1, so page 0 comes out before the file starts, which a chain takes as
/// its end and a gather passes over.
fn page_offset(n: E) -> E {
    n.sub(E::lit(1)).mul(page_bytes())
}

/// A freelist trunk page: the next trunk, then the numbers of free pages it
/// holds. SQLite does not use the rest of the page, which still holds whatever
/// was there before the page was freed, unless `secure_delete` cleared it.
///
/// `next_trunk_offset` is `next_trunk` as a byte offset. A chain follows a
/// link written as an offset, and SQLite writes a page number, so the
/// conversion is a field of its own for the chain to follow. Zero for the
/// last trunk, which is how a chain ends.
fn freelist_trunk() -> T {
    let next = || E::field("next_trunk");
    // A record of one number rather than the number, because a gather reads
    // its offsets from records, and `free_pages` gathers these.
    let entry = T::inline_structure("FreelistEntry", vec![("page", T::u32(Big))]);
    T::structure(
        "FreelistTrunk",
        vec![
            ("next_trunk", T::u32(Big)),
            ("leaf_count", T::u32(Big)),
            ("leaves", T::array(entry, E::field("leaf_count"))),
            ("stale", T::bytes(E::Remaining)),
            (
                "next_trunk_offset",
                T::computed(E::cond(next().equal_to(E::lit(0)), E::lit(0), page_offset(next()))),
            ),
        ],
    )
    .machinery(&["next_trunk_offset"])
}

/// A page on the freelist that is not a trunk. SQLite never reads one, and
/// does not clear it when it frees it unless `secure_delete` is on, so what is
/// here is whatever the page held last.
fn freelist_leaf() -> T {
    T::structure("FreelistLeaf", vec![("stale", T::bytes(E::Remaining))])
}

/// Whether page `n` is on the freelist, and as what: 1 for a trunk, 2 for a
/// leaf, 0 for neither. Asked of `freelist` and `free_pages`, which the
/// database reads before its pages.
fn freelist_role(n: impl Fn() -> E) -> E {
    let trunk = n()
        .equal_to(E::field("first_freelist_page"))
        .either(E::tagged_in_by(E::field("freelist"), &["next_trunk"], n(), &["next_trunk"]));
    let leaf = E::tagged_in_by(E::field("free_pages"), &[], n(), &[]);
    E::cond(trunk, E::lit(1), E::cond(leaf, E::lit(2), E::lit(0)))
}

/// A page that is not a b-tree and not on the freelist, which is a page whose
/// first byte was never a type. It is an overflow page, a pointer map, or the
/// page that holds the lock byte, and the page says which of those it is
/// nowhere: what points at it decides.
///
/// The template cannot follow what points at it (see the module comment), but
/// it can rule out everything else. A database with no auto-vacuum has no
/// pointer maps. The lock byte is at 1 GiB, so the page holding it is known by
/// its number. And the page is not on the freelist, which is exact when the
/// freelist as read holds as many pages as the header says it does. With all
/// of that true, every page left over is the continuation of a payload too big
/// for its own page, and reads as the next page in that chain and the bytes it
/// carries. Anywhere else the honest answer is the bytes.
fn other_page(n: E) -> T {
    let lock_byte_page = E::lit(1 << 30).div(page_bytes()).add(E::lit(1));
    let freelist_whole = E::len_of("freelist")
        .add(E::len_of("free_pages"))
        .equal_to(E::field("freelist_page_count"));
    let settled = E::field("vacuum_root_page")
        .equal_to(E::lit(0))
        .both(n.not_equal(lock_byte_page))
        .both(freelist_whole);
    T::switch(
        settled,
        vec![(
            1,
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
/// `number` is the page's number, for every page but the first. Page 1 is the
/// schema and is never free, so it is not looked up on the freelist.
///
/// The type byte is peeked rather than read, because only a b-tree page has
/// one. Reading it first and switching on it afterwards would have every other
/// page in the file open with a field the format never wrote, and show the
/// byte a page number happens to start with as a page type nobody defined.
fn page(adjust: E, usable: E, number: Option<E>, rec: T) -> T {
    let body =
        |name, interior, ty| btree_body(name, interior, ty, adjust.clone(), usable.clone(), rec.clone());
    let by_type_byte = |other: T| {
        T::switch(
            E::peek(8, Big),
            vec![
                (2, body("IndexInterior", true, 2)),
                (5, body("TableInterior", true, 5)),
                (10, body("IndexLeaf", false, 10)),
                (13, body("TableLeaf", false, 13)),
            ],
            other,
        )
    };
    let Some(n) = number else { return by_type_byte(T::bytes(E::Remaining)) };
    T::switch(
        freelist_role(|| n.clone()),
        vec![(1, freelist_trunk()), (2, freelist_leaf())],
        by_type_byte(other_page(n.clone())),
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
    // What of a page a payload may use: the page less whatever the header
    // holds back at the end of every one of them.
    let usable = || page_bytes().sub(E::field("reserved_space"));
    let page1 = T::sized(page_bytes().sub(E::lit(100)), page(E::lit(-100), usable(), None, schema_record()));
    // Page `i` of the run is page `i + 2` of the file.
    let each = page(E::lit(0), usable(), Some(E::idx().add(E::lit(2))), record());
    let pages = T::repeat(T::sized(page_bytes(), each), Until::End);
    Template::new(
        name,
        T::structure(
            root,
            vec![
                ("magic", T::magic(b"SQLite format 3\0")),
                ("page_size", T::u16(Big)),
                // A page size of 65536 does not fit the two bytes above, so it
                // is written as 1. This is the size in bytes either way, and
                // what everything below is sized by.
                (
                    "page_bytes",
                    T::computed(E::cond(
                        E::field("page_size").equal_to(E::lit(1)),
                        E::lit(65536),
                        E::field("page_size"),
                    )),
                ),
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
                ("page1", page1),
                // The freelist, read from the header's pointer to its first
                // trunk before any page is typed, so that a page it names is
                // free whatever its first byte says. The trunks are also pages
                // of the run below, and this is the second reading of them.
                (
                    "freelist",
                    T::chain(
                        page_offset(E::field("first_freelist_page")),
                        &["next_trunk_offset"],
                        Anchor::Space,
                        T::sized(page_bytes(), freelist_trunk()),
                    ),
                ),
                // Every free page that is not a trunk, from all the trunks as
                // one list, so that a page can ask whether it is one of them.
                // No bytes: each entry is only the page number, placed where
                // that page starts.
                (
                    "free_pages",
                    T::gather(
                        vec![Step::field("freelist"), Step::each(), Step::field("leaves"), Step::each()],
                        page_offset(E::field("page")),
                        Anchor::Space,
                        E::lit(0),
                        T::computed(E::placer(E::field("page"))),
                    )
                    .skipping_zero(),
                ),
                ("pages", pages),
            ],
        )
        // Both freelist fields read pages the run below also reads, and the
        // run is where those bytes belong. Counted in both places, a trunk
        // would be two pages.
        .field_aside("freelist")
        .field_aside("free_pages"),
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
    const TEXT_ENCODING: usize = 17;
    const APPLICATION_ID: usize = 20;
    const PAGE1: usize = 24;
    const PAGES: usize = 27;
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
        assert_eq!(id.value, Value::Magic { ok: true, bytes: b"SELF".to_vec(), expected: b"SELF".to_vec() });
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

    /// A cell's length prefix is a varint, and the span the listing draws
    /// carries the split between the bit that ends it and the bits that are
    /// the number. Nothing else on the page does: a page's cell count is two
    /// plain bytes.
    #[test]
    fn a_varint_span_carries_its_bit_split() {
        use crate::varintbits::{BitRole, RULE_HIGH_BIT};
        let d = Document::new(MemSource(db()));
        let mut ev = Evaluator::new(sqlite());
        let size = ev.node(&d, &[PAGES, 0, CELLS, 0, 0]).unwrap();
        assert_eq!(size.name, "payload_size");
        let spans = ev.spans(&d, size.offset_bits, size.offset_bits + size.size_bits, 4).unwrap();
        let bits = spans[0].bits.as_ref().expect("a varint says how its bits divide");
        assert_eq!(bits.rule, RULE_HIGH_BIT);
        assert_eq!(bits.groups.len(), 2);
        assert_eq!(bits.groups[0].role, BitRole::Stop);
        assert_eq!(bits.groups[1].role, BitRole::Payload);
        assert_eq!(bits.groups[1].bits.len(), 7);
        let count = ev.node(&d, &[PAGES, 0, CELL_COUNT]).unwrap();
        let plain = ev.spans(&d, count.offset_bits, count.offset_bits + count.size_bits, 4).unwrap();
        assert!(plain[0].bits.is_none());
    }

    /// A record's serial-type list is as long as the header says less the
    /// bytes the header spent saying so, and that sentence is the expression
    /// the template holds. Written out with the numbers in it, a reader can
    /// check the 5 rather than take it.
    #[test]
    fn a_length_says_what_it_was_worked_out_from() {
        use crate::eval::Role;
        let d = Document::new(MemSource(db()));
        let mut ev = Evaluator::new(sqlite());
        let rel = ev.relations(&d, &[PAGES, 0, CELLS, 0, 2, 1]).unwrap();
        assert_eq!(rel.len(), 1);
        assert_eq!(rel[0].role, Role::Length);
        assert_eq!(rel[0].written, "header_size - sizeof(header_size)");
        assert_eq!(rel[0].substituted, "4 - 1");
        assert_eq!(rel[0].result, "3");
        // A field the template placed and sized outright has no relationship
        // to write out, and says nothing rather than restating its own size.
        assert!(ev.relations(&d, &[PAGE_SIZE]).unwrap().is_empty());
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
        // field. Asked from partway into it, it is still the whole stretch:
        // it begins where the pointer array ends and stops where that row
        // starts.
        let from = (PAGE + 20) as u64 * 8;
        let free = ev.spans(&d, from, 2 * PAGE as u64 * 8, 4).unwrap();
        assert!(free[0].gap);
        let pointers = ev.node(&d, &[PAGES, 0, POINTERS]).unwrap();
        assert_eq!(free[0].offset_bits, pointers.offset_bits + pointers.size_bits);
        assert!(free[0].offset_bits < from);
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
        // rather than a type. The header counts a free page and names no
        // trunk, so the freelist as read is short of what the header says and
        // cannot rule this page out. All of it reads as bytes, first byte
        // included.
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

    /// A freelist of two trunks, each listing one page that was a table leaf
    /// before it was freed, and one overflow page. Pages 2 to 7: a live leaf,
    /// trunk, free leaf, trunk, free leaf, overflow page.
    fn freed() -> Vec<u8> {
        let trunk = |next: u32, leaf: u32| {
            let mut p = vec![0u8; PAGE];
            p[0..4].copy_from_slice(&next.to_be_bytes());
            p[4..8].copy_from_slice(&1u32.to_be_bytes());
            p[8..12].copy_from_slice(&leaf.to_be_bytes());
            p
        };
        let stale = leaf_page(&[cell_bytes(9, 1, "gone")], PAGE, 0);
        let mut b = header(PAGE);
        b[28..32].copy_from_slice(&7u32.to_be_bytes()); // page count
        b[32..36].copy_from_slice(&3u32.to_be_bytes()); // first trunk
        b[36..40].copy_from_slice(&4u32.to_be_bytes()); // free pages
        b.extend_from_slice(&leaf_page(&[schema_cell(1, "m", 2, "CREATE TABLE m(x)")], PAGE - 100, 100));
        b.extend_from_slice(&leaf_page(&[cell_bytes(1, 42, "hi")], PAGE, 0));
        b.extend_from_slice(&trunk(5, 4));
        b.extend_from_slice(&stale);
        b.extend_from_slice(&trunk(0, 6));
        b.extend_from_slice(&stale);
        b.extend_from_slice(&[0u8; PAGE]);
        b
    }

    #[test]
    fn a_page_on_the_freelist_is_free_whatever_its_first_byte_says() {
        let d = Document::new(MemSource(freed()));
        let mut ev = Evaluator::new(sqlite());
        let kinds: Vec<String> = (0..6).map(|i| ev.node(&d, &[PAGES, i]).unwrap().type_name).collect();
        let want = ["TableLeaf", "FreelistTrunk", "FreelistLeaf", "FreelistTrunk", "FreelistLeaf", "Overflow"];
        assert_eq!(kinds, want);
        // The chain went from the first trunk to the second by page number,
        // and the leaves of both are one list.
        let freelist = ev.child_named(&d, &[], "freelist").unwrap().unwrap();
        assert_eq!(ev.node(&d, &freelist).unwrap().child_count, 2);
        assert_eq!(ev.node(&d, &[freelist.as_slice(), &[1]].concat()).unwrap().offset_bits, 4 * PAGE as u64 * 8);
        let free = ev.child_named(&d, &[], "free_pages").unwrap().unwrap();
        let listed: Vec<Value> = (0..2).map(|i| ev.node(&d, &[free.as_slice(), &[i]].concat()).unwrap().value).collect();
        assert_eq!(listed, [Value::Int(4), Value::Int(6)]);
        // The freelist is a second reading of the trunks. A trunk's bytes
        // belong to its page in the run, and that is where the cursor lands.
        assert_eq!(ev.locate(&d, 2 * PAGE as u64 * 8).unwrap(), vec![PAGES, 1, 0]);
    }

    #[test]
    fn a_leftover_page_is_bytes_when_the_freelist_is_short_of_the_header() {
        let mut b = freed();
        b[39] = 5; // one more free page than the trunks list
        let d = Document::new(MemSource(b));
        let mut ev = Evaluator::new(sqlite());
        assert_eq!(ev.node(&d, &[PAGES, 1]).unwrap().type_name, "FreelistTrunk");
        assert_eq!(ev.node(&d, &[PAGES, 5]).unwrap().type_name, "bytes[]");
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
