//! GNU dbm: the key-value store behind a `.db` that Postfix, sendmail or an
//! old system utility left behind.
//!
//! The file opens with a header saying where the hash directory is, how big a
//! bucket is, and where the free space starts, and that header is a C struct
//! written straight to disk. Which struct depends on the machine and on the
//! version, and the magic number at the front says which: a four-byte file
//! offset or an eight-byte one, with or without the sixteen-byte extension
//! GDBM 1.15 added to count synchronisations. The same magic read backwards is
//! a file written on a machine of the other byte order, so it settles that
//! too. Between them that is ten headers, all of the same fields.
//!
//! After the header comes the first block of free space: a count, a link to
//! the next block, and a table of address-and-size pairs, one per stretch of
//! the file that has been given back. The table is as long as the header says
//! it is; only the first `count` entries mean anything.
//!
//! The buckets and the records themselves are elsewhere in the file, at the
//! addresses the directory holds. Following those is not something this does
//! yet, so the rest of the file is bytes.

use crate::template::{Endian, Endian::*, Expr as E, Template, Ty as T};

/// The magics from `gdbmconst.h`, and what each says about the header behind
/// it. `off_bits` is the width of a file offset, which is what moves every
/// field after the first two.
struct Shape {
    magic: i128,
    endian: Endian,
    off_bits: u32,
    /// The 1.15 extension: a version and a count of synchronisations, with
    /// room after them for more.
    numsync: bool,
}

const SHAPES: &[Shape] = &[
    // The original, from before file offsets could be either width. It was
    // written with whatever `off_t` the machine had, and four is what that
    // was everywhere it was written.
    Shape { magic: 0x13579ace, endian: Little, off_bits: 32, numsync: false },
    Shape { magic: 0x13579acd, endian: Little, off_bits: 32, numsync: false },
    Shape { magic: 0x13579acf, endian: Little, off_bits: 64, numsync: false },
    Shape { magic: 0x13579ad0, endian: Little, off_bits: 32, numsync: true },
    Shape { magic: 0x13579ad1, endian: Little, off_bits: 64, numsync: true },
    Shape { magic: 0xce9a5713, endian: Big, off_bits: 32, numsync: false },
    Shape { magic: 0xcd9a5713, endian: Big, off_bits: 32, numsync: false },
    Shape { magic: 0xcf9a5713, endian: Big, off_bits: 64, numsync: false },
    Shape { magic: 0xd09a5713, endian: Big, off_bits: 32, numsync: true },
    Shape { magic: 0xd19a5713, endian: Big, off_bits: 64, numsync: true },
];

/// What each magic means, for the field that holds it. The number is shown as
/// the file's own bytes read little-endian, which is how the shape above is
/// picked, so a big-endian file's magic is listed as the bytes reversed.
const MAGIC: &[(i128, &str)] = &[
    (0x13579ace, "gdbm, original"),
    (0x13579acd, "gdbm, 32-bit offsets"),
    (0x13579acf, "gdbm, 64-bit offsets"),
    (0x13579ad0, "gdbm, 32-bit offsets, sync-counted"),
    (0x13579ad1, "gdbm, 64-bit offsets, sync-counted"),
    (0xce9a5713, "gdbm, original, big-endian"),
    (0xcd9a5713, "gdbm, 32-bit offsets, big-endian"),
    (0xcf9a5713, "gdbm, 64-bit offsets, big-endian"),
    (0xd09a5713, "gdbm, 32-bit offsets, sync-counted, big-endian"),
    (0xd19a5713, "gdbm, 64-bit offsets, sync-counted, big-endian"),
];

pub fn gdbm() -> Template {
    let cases = SHAPES.iter().map(|s| (s.magic, header(s))).collect();
    // Nothing but the magic says which header this is, so it is read before
    // anything is read out of it.
    Template::new("gdbm", T::switch(E::peek(32, Little), cases, T::bytes(E::Remaining)))
}

fn header(s: &Shape) -> T {
    let int = || T::Int { bits: 32, endian: s.endian };
    let off = || T::Int { bits: s.off_bits, endian: s.endian };
    // Both file offsets already land on a multiple of their own width, so a
    // wider one moves the fields after it without a hole in front of it. The
    // one hole the compiler does leave is inside the free-space table below.
    let mut fields: Vec<(&'static str, T)> = vec![
        ("magic", T::enumeration_hex("Header", T::u32(Little), MAGIC)),
        ("block_size", int()),
        // Where the table of bucket addresses is.
        ("dir", off()),
        ("dir_size", int()),
        ("dir_bits", int()),
        ("bucket_size", int()),
        ("bucket_elems", int()),
        // Where the file grows from, when nothing free is big enough.
        ("next_block", off()),
    ];
    if s.numsync {
        fields.push(("ext_version", int()));
        fields.push(("numsync", T::u32(s.endian)));
        fields.push(("ext_pad", T::bytes(E::lit(24))));
    }
    fields.push(("avail", avail(s)));
    fields.push(("rest", T::bytes(E::Remaining)));
    T::structure("Gdbm", fields)
}

/// The first block of free space, which lives in the header block.
fn avail(s: &Shape) -> T {
    let int = || T::Int { bits: 32, endian: s.endian };
    let off = || T::Int { bits: s.off_bits, endian: s.endian };
    let wide = s.off_bits == 64;
    let element = T::inline_structure(
        "Free",
        {
            let mut f: Vec<(&'static str, T)> = vec![("av_size", int())];
            if wide {
                f.push(("pad", T::bytes(E::lit(4))));
            }
            f.push(("av_adr", off()));
            f
        },
    );
    T::inline_structure(
        "Available",
        vec![
            // How many entries the table has room for, and how many are used.
            ("size", int()),
            ("count", int()),
            ("next_block", off()),
            (
                "table",
                // A header that claims more entries than the file could hold
                // would read past the end, so the count is what there is room
                // for at most.
                T::array(element, E::field("size").at_most(E::Remaining.div(E::lit(if wide { 16 } else { 8 })))),
            ),
        ],
    )
}

/// Whether these bytes open a GDBM file: one of the ten magics, which are
/// distinct enough that reading the first word one way round is enough.
pub fn is_gdbm(head: &[u8]) -> bool {
    let Some(bytes) = head.get(0..4) else { return false };
    let word = u32::from_le_bytes(bytes.try_into().expect("four bytes")) as i128;
    SHAPES.iter().any(|s| s.magic == word)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    /// A header of the shape one of the magics asks for, filled with numbers
    /// that can be told apart.
    fn file(magic: u32, wide: bool, big: bool, numsync: bool) -> Vec<u8> {
        let w32 = |v: u32| if big { v.to_be_bytes().to_vec() } else { v.to_le_bytes().to_vec() };
        let off = |v: u64| {
            if wide {
                if big { v.to_be_bytes().to_vec() } else { v.to_le_bytes().to_vec() }
            } else {
                w32(v as u32)
            }
        };
        let pad = |v: &mut Vec<u8>| {
            if wide {
                v.extend_from_slice(&[0; 4]);
            }
        };
        // The magic is written as the bytes the constant names, whichever way
        // round the rest of the file is.
        let mut v = magic.to_le_bytes().to_vec();
        v.extend(w32(1024)); // block_size
        v.extend(off(2048)); // dir
        v.extend(w32(128)); // dir_size
        v.extend(w32(5)); // dir_bits
        v.extend(w32(1024)); // bucket_size
        v.extend(w32(51)); // bucket_elems
        v.extend(off(4096)); // next_block
        if numsync {
            v.extend(w32(0)); // ext version
            v.extend(w32(9)); // numsync
            v.extend_from_slice(&[0; 24]);
        }
        v.extend(w32(2)); // avail size
        v.extend(w32(1)); // avail count
        v.extend(off(0)); // avail next_block
        for (size, adr) in [(64u32, 3000u64), (0, 0)] {
            v.extend(w32(size));
            pad(&mut v);
            v.extend(off(adr));
        }
        v.resize(1024, 0);
        v
    }

    #[test]
    fn a_32_bit_header_reads_its_fields() {
        let bytes = file(0x13579acd, false, false, false);
        assert!(is_gdbm(&bytes));
        let d = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(gdbm());
        assert_eq!(
            ev.node(&d, &[0]).unwrap().value,
            Value::Enum { raw: 0x13579acd, name: Some("gdbm, 32-bit offsets".into()), hex: true }
        );
        assert_eq!(ev.node(&d, &[1]).unwrap().value, Value::Int(1024));
        assert_eq!(ev.node(&d, &[2]).unwrap().value, Value::Int(2048));
        assert_eq!(ev.node(&d, &[6]).unwrap().value, Value::Int(51));
        assert_eq!(ev.node(&d, &[7]).unwrap().value, Value::Int(4096));
        // The free-space table: two slots, one of them used.
        assert_eq!(ev.node(&d, &[8, 0]).unwrap().value, Value::Int(2));
        assert_eq!(ev.node(&d, &[8, 1]).unwrap().value, Value::Int(1));
        assert_eq!(ev.node(&d, &[8, 3]).unwrap().child_count, 2);
        assert_eq!(ev.node(&d, &[8, 3, 0, 0]).unwrap().value, Value::Int(64));
        assert_eq!(ev.node(&d, &[8, 3, 0, 1]).unwrap().value, Value::Int(3000));
    }

    /// The 64-bit header is the same fields with a hole in front of each file
    /// offset, which is what the compiler that wrote it put there.
    #[test]
    fn a_64_bit_header_reads_past_the_alignment_holes() {
        let bytes = file(0x13579acf, true, false, false);
        assert!(is_gdbm(&bytes));
        let d = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(gdbm());
        assert_eq!(ev.node(&d, &[2]).unwrap().offset_bits, 8 * 8);
        assert_eq!(ev.node(&d, &[2]).unwrap().value, Value::Int(2048));
        assert_eq!(ev.node(&d, &[7]).unwrap().offset_bits, 32 * 8);
        assert_eq!(ev.node(&d, &[7]).unwrap().value, Value::Int(4096));
        assert_eq!(ev.node(&d, &[8, 3, 0, 2]).unwrap().value, Value::Int(3000));
    }

    #[test]
    fn a_big_endian_header_reads_the_other_way_round() {
        let bytes = file(0xcf9a5713, true, true, false);
        assert!(is_gdbm(&bytes));
        let d = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(gdbm());
        assert_eq!(ev.node(&d, &[1]).unwrap().value, Value::Int(1024));
        assert_eq!(ev.node(&d, &[2]).unwrap().value, Value::Int(2048));
    }

    /// GDBM 1.15 added a count of how many times the file has been written
    /// out, between the header and the free space.
    #[test]
    fn a_sync_counted_header_has_the_extension_in_the_middle() {
        let bytes = file(0x13579ad1, true, false, true);
        assert!(is_gdbm(&bytes));
        let d = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(gdbm());
        assert_eq!(ev.node(&d, &[9]).unwrap().name, "numsync");
        assert_eq!(ev.node(&d, &[9]).unwrap().value, Value::UInt(9));
        assert_eq!(ev.node(&d, &[11, 0]).unwrap().value, Value::Int(2));
    }

    #[test]
    fn other_bytes_are_not_a_gdbm_file() {
        assert!(!is_gdbm(b"SQLite format 3\0"));
        assert!(!is_gdbm(&[0; 2]));
    }
}
