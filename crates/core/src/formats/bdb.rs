//! Berkeley DB: the key-value store behind Postfix lookup tables, RPM's own
//! database until it moved, and a great many `.db` files nobody remembers
//! creating.
//!
//! A database is a run of fixed-size pages, and page zero says everything
//! about the rest: which access method built the file, how big a page is, how
//! many there are. That first page opens with a log sequence number and a page
//! number, and the magic that names the access method sits twelve bytes in.
//!
//! The numbers are written the way the machine that wrote them writes numbers,
//! and nothing in the file says which way that was. The magic settles it: read
//! it one way round and either it is one of the four the library uses, or the
//! file is the other way round. Every field after it follows.
//!
//! The first seventy-two bytes are the same whichever access method wrote the
//! file. What comes after them is the method's own: a btree keeps its root
//! page and its minimum key count, a hash keeps its bucket masks and its fill
//! factor, a queue keeps its record numbers. The last fifty-two bytes of the
//! page are the encryption fields, which an unencrypted file leaves zero.
//!
//! Only the meta page is described. The pages after it are read as far as
//! their header, which says what kind of page each is and how many items are
//! on it; the items themselves are an index of offsets growing from the front
//! of the page and the data growing back from the end, and reading a key out
//! of one is not something this does yet.
//!
//! Berkeley DB 1.85 and 1.86 wrote a different header, with the magic at the
//! front of the file rather than twelve bytes in. Those are not read here.

use crate::template::{Endian, Endian::*, Expr as E, Template, Ty as T, Until};

/// Which access method wrote the file, from `db.h`. A log file's magic is in
/// the same place, but a log is not a run of pages and is not read here.
const MAGIC: &[(i128, &str)] = &[
    (0x053162, "btree"),
    (0x061561, "hash"),
    (0x042253, "queue"),
    (0x074582, "heap"),
];

/// The magics as a little-endian reader sees them in a file written on a
/// little-endian machine. A file from a big-endian one reads as these bytes
/// reversed, which is what picks the other layout below.
const LITTLE_MAGICS: &[i128] = &[0x053162, 0x061561, 0x042253, 0x074582];

/// What a page holds, from `db_page.h`.
const PAGE_TYPE: &[(i128, &str)] = &[
    (0, "invalid"),
    (1, "duplicate"),
    (2, "hash, unsorted"),
    (3, "btree internal"),
    (4, "recno internal"),
    (5, "btree leaf"),
    (6, "recno leaf"),
    (7, "overflow"),
    (8, "hash metadata"),
    (9, "btree metadata"),
    (10, "queue metadata"),
    (11, "queue data"),
    (12, "duplicate leaf"),
    (13, "hash"),
    (14, "heap metadata"),
    (15, "heap data"),
    (16, "heap internal"),
];

/// Set on the meta page only.
const META_FLAGS: &[(u32, &str)] = &[(0, "checksums"), (1, "partitioned by range"), (2, "partitioned by callback")];

const ENCRYPT_ALG: &[(i128, &str)] = &[(0, "none"), (1, "aes")];

/// The smallest page the library will make, and the length of everything the
/// meta page describes.
const META_BYTES: i128 = 512;

/// Where the magic sits, in bits, for the peek that settles the byte order.
const MAGIC_AT_BITS: i128 = 12 * 8;

pub fn bdb() -> Template {
    // Read the magic as a little-endian number. One of the four means the file
    // was written little-endian; anything else means it was written the other
    // way round, and every number in it is read that way.
    let little = LITTLE_MAGICS.iter().map(|m| (*m, database(Little))).collect();
    Template::new("bdb", T::switch(E::peek_at(E::lit(MAGIC_AT_BITS), 32, Little), little, database(Big)))
}

fn database(e: Endian) -> T {
    let mut fields = meta(e);
    // A page count is not written down anywhere that can be trusted, so the
    // pages are however many fit in what is left of the file.
    fields.push(("pages", T::repeat(T::sized(page_bytes(), page(e)), Until::End)));
    T::structure("Database", fields)
}

/// How long a page is. A meta page that says something impossible would give
/// the run below elements of no length at all, so the smallest page the
/// library makes is the floor.
fn page_bytes() -> E {
    E::field("pagesize").at_least(E::lit(META_BYTES))
}

/// Page zero: the header every access method shares, then the part that is the
/// access method's own, then the encryption fields at the end of the page.
fn meta(e: Endian) -> Vec<(&'static str, T)> {
    let u32 = || T::u32(e);
    vec![
        // Where the last change to this page is in the log.
        ("lsn_file", u32()),
        ("lsn_offset", u32()),
        ("pgno", u32()),
        ("magic", T::enumeration_hex("AccessMethod", u32(), MAGIC)),
        ("version", u32()),
        ("pagesize", u32()),
        ("encrypt_alg", T::enumeration("Encryption", T::u8(), ENCRYPT_ALG)),
        ("page_type", T::enumeration("PageType", T::u8(), PAGE_TYPE)),
        ("metaflags", T::flags("MetaFlags", T::u8(), META_FLAGS)),
        ("unused1", T::u8()),
        ("free", u32()),
        ("last_pgno", u32()),
        ("nparts", u32()),
        ("key_count", u32()),
        ("record_count", u32()),
        ("flags", u32()),
        // What makes this file distinguishable from another copy of it.
        ("uid", T::bytes(E::lit(20))),
        ("method", by_method(e)),
        ("crypto_magic", u32()),
        ("trash", T::bytes(E::lit(12))),
        ("iv", T::bytes(E::lit(20))),
        ("checksum", T::bytes(E::lit(16))),
        // A page larger than the 512 bytes described above has the rest of it
        // spare.
        ("unused_page_tail", T::bytes(page_bytes().sub(E::lit(META_BYTES)))),
    ]
}

/// Bytes 72 to 459 of the meta page, which are the access method's own. The
/// page type says which method, and it is the meta page's own type rather than
/// the magic because that is what the library reads.
fn by_method(e: Endian) -> T {
    let u32 = || T::u32(e);
    let btree = T::inline_structure(
        "BtreeMeta",
        vec![
            ("unused1", u32()),
            // The library keeps at least this many keys on a page.
            ("minkey", u32()),
            ("re_len", u32()),
            ("re_pad", u32()),
            ("root", u32()),
            ("unused2", T::bytes(E::lit(368))),
        ],
    );
    let hash = T::inline_structure(
        "HashMeta",
        vec![
            ("max_bucket", u32()),
            ("high_mask", u32()),
            ("low_mask", u32()),
            ("ffactor", u32()),
            ("nelem", u32()),
            ("h_charkey", u32()),
            ("spares", T::array(u32(), E::lit(32))),
            ("unused", T::bytes(E::lit(236))),
        ],
    );
    let queue = T::inline_structure(
        "QueueMeta",
        vec![
            ("first_recno", u32()),
            ("cur_recno", u32()),
            ("re_len", u32()),
            ("re_pad", u32()),
            ("rec_page", u32()),
            ("page_ext", u32()),
            ("unused", T::bytes(E::lit(364))),
        ],
    );
    let heap = T::inline_structure(
        "HeapMeta",
        vec![
            ("curregion", u32()),
            ("nregions", u32()),
            ("gbytes", u32()),
            ("bytes", u32()),
            ("region_size", u32()),
            ("unused2", T::bytes(E::lit(368))),
        ],
    );
    T::switch(
        E::field("page_type"),
        vec![(9, btree), (8, hash), (10, queue), (14, heap)],
        T::bytes(E::lit(388)),
    )
}

/// Every page after the first opens the same way, whatever it holds.
fn page(e: Endian) -> T {
    T::structure_named(
        "Page",
        "page_type",
        "contents",
        vec![
            ("lsn_file", T::u32(e)),
            ("lsn_offset", T::u32(e)),
            ("pgno", T::u32(e)),
            ("prev_pgno", T::u32(e)),
            ("next_pgno", T::u32(e)),
            ("entries", T::u16(e)),
            // Where the items growing back from the end of the page have got
            // to, so the free space is between here and the index.
            ("hf_offset", T::u16(e)),
            ("level", T::u8()),
            ("page_type", T::enumeration("PageType", T::u8(), PAGE_TYPE)),
            ("contents", T::bytes(E::Remaining)),
        ],
    )
}

/// Whether these bytes open a Berkeley DB of the modern layout: one of the
/// four access-method magics twelve bytes in, either way round.
pub fn is_bdb(head: &[u8]) -> bool {
    let Some(bytes) = head.get(12..16) else { return false };
    let word: [u8; 4] = bytes.try_into().expect("four bytes");
    let le = u32::from_le_bytes(word) as i128;
    let be = u32::from_be_bytes(word) as i128;
    LITTLE_MAGICS.contains(&le) || LITTLE_MAGICS.contains(&be)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    const PAGE: usize = 512;

    /// A meta page and one page after it, written the way the machine that
    /// wrote them writes numbers.
    fn database_bytes(magic: u32, page_type: u8, big: bool) -> Vec<u8> {
        let w = |v: u32| if big { v.to_be_bytes() } else { v.to_le_bytes() };
        let mut v = Vec::new();
        v.extend_from_slice(&w(1)); // lsn file
        v.extend_from_slice(&w(2)); // lsn offset
        v.extend_from_slice(&w(0)); // pgno
        v.extend_from_slice(&w(magic));
        v.extend_from_slice(&w(9)); // version
        v.extend_from_slice(&w(PAGE as u32));
        v.push(0); // encrypt_alg
        v.push(page_type);
        v.push(1); // metaflags: checksums
        v.push(0); // unused1
        v.extend_from_slice(&w(0)); // free
        v.extend_from_slice(&w(1)); // last_pgno
        v.extend_from_slice(&w(0)); // nparts
        v.extend_from_slice(&w(7)); // key_count
        v.extend_from_slice(&w(7)); // record_count
        v.extend_from_slice(&w(0)); // flags
        v.extend_from_slice(&[0xab; 20]); // uid
        assert_eq!(v.len(), 72);
        // The access method's own part: minkey and root for a btree, the
        // masks for a hash.
        if page_type == 9 {
            v.extend_from_slice(&w(0));
            v.extend_from_slice(&w(2)); // minkey
            v.extend_from_slice(&w(0));
            v.extend_from_slice(&w(0));
            v.extend_from_slice(&w(1)); // root
        } else {
            v.extend_from_slice(&w(3)); // max_bucket
            v.extend_from_slice(&w(7)); // high_mask
            v.extend_from_slice(&w(3)); // low_mask
            v.extend_from_slice(&w(20)); // ffactor
            v.extend_from_slice(&w(5)); // nelem
            v.extend_from_slice(&w(0));
        }
        v.resize(PAGE, 0);
        // Page one: a leaf with three items on it.
        let mut page = Vec::new();
        page.extend_from_slice(&w(3)); // lsn file
        page.extend_from_slice(&w(4)); // lsn offset
        page.extend_from_slice(&w(1)); // pgno
        page.extend_from_slice(&w(0)); // prev
        page.extend_from_slice(&w(0)); // next
        page.extend_from_slice(&if big { 3u16.to_be_bytes() } else { 3u16.to_le_bytes() });
        page.extend_from_slice(&if big { 400u16.to_be_bytes() } else { 400u16.to_le_bytes() });
        page.push(1); // level
        page.push(5); // btree leaf
        page.resize(PAGE, 0);
        v.extend(page);
        v
    }

    #[test]
    fn a_btree_reads_its_meta_page_and_the_page_after_it() {
        let bytes = database_bytes(0x053162, 9, false);
        assert!(is_bdb(&bytes));
        let d = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(bdb());
        assert_eq!(
            ev.node(&d, &[3]).unwrap().value,
            Value::Enum { raw: 0x053162, name: Some("btree".into()), hex: true }
        );
        assert_eq!(ev.node(&d, &[5]).unwrap().value, Value::UInt(512));
        assert_eq!(
            ev.node(&d, &[7]).unwrap().value,
            Value::Enum { raw: 9, name: Some("btree metadata".into()), hex: false }
        );
        // The btree's own fields: at least two keys a page, rooted at page one.
        assert_eq!(ev.node(&d, &[17, 1]).unwrap().value, Value::UInt(2));
        assert_eq!(ev.node(&d, &[17, 4]).unwrap().value, Value::UInt(1));
        // And the page after the meta page.
        let pages = ev.node(&d, &[23]).unwrap();
        assert_eq!(pages.child_count, 1);
        assert_eq!(ev.node(&d, &[23, 0, 5]).unwrap().value, Value::UInt(3));
        assert_eq!(
            ev.node(&d, &[23, 0, 8]).unwrap().value,
            Value::Enum { raw: 5, name: Some("btree leaf".into()), hex: false }
        );
    }

    /// The same file written on a big-endian machine. Nothing in it says so;
    /// the magic reading backwards is what says so.
    #[test]
    fn a_big_endian_database_reads_the_other_way_round() {
        let bytes = database_bytes(0x061561, 8, true);
        assert!(is_bdb(&bytes));
        let d = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(bdb());
        assert_eq!(
            ev.node(&d, &[3]).unwrap().value,
            Value::Enum { raw: 0x061561, name: Some("hash".into()), hex: true }
        );
        assert_eq!(ev.node(&d, &[5]).unwrap().value, Value::UInt(512));
        // The hash's own fields: four buckets in use, filled to twenty.
        assert_eq!(ev.node(&d, &[17, 0]).unwrap().value, Value::UInt(3));
        assert_eq!(ev.node(&d, &[17, 3]).unwrap().value, Value::UInt(20));
    }

    #[test]
    fn the_meta_page_covers_exactly_one_page() {
        let d = Document::new(MemSource(database_bytes(0x053162, 9, false)));
        let mut ev = Evaluator::new(bdb());
        let pages = ev.node(&d, &[23]).unwrap();
        assert_eq!(pages.offset_bits, PAGE as u64 * 8);
    }

    #[test]
    fn a_file_with_no_magic_where_one_belongs_is_not_one_of_these() {
        assert!(!is_bdb(b"SQLite format 3\0"));
        assert!(!is_bdb(&[0; 8]));
    }
}
