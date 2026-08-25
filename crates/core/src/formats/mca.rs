//! Minecraft Anvil region: 1024 chunks of a world, indexed by two tables of
//! 4096 bytes each and stored in 4096-byte sectors.
//!
//! A location entry is three bytes of sector number and one byte of length in
//! sectors, so the file addresses a gigabyte with a 24-bit number. Multiplying
//! that number by 4096 is what turns it into an offset, and a `PointerList`
//! adds to its offsets rather than scaling them, so each entry carries a
//! computed field holding the offset itself, and the list points with that.
//! It costs no bytes and it is what makes the chunks land where they are.
//!
//! An entry of all zeroes means the chunk has never been generated, and most
//! of the 1024 in a real region are. A pointer list can be told that a zero
//! offset points at nothing, so those entries keep their place in the list
//! and cover no bytes. Without that, a zero is an offset pointing before the
//! list that holds it, which is an error, and one ungenerated chunk would
//! make the region unreadable.

use crate::template::{Anchor, Endian::*, Expr as E, Template, Ty as T};

/// How a chunk was compressed. 3 means it was not, which the game writes only
/// when a chunk is too large for a compressor to help.
const COMPRESSION: &[(i128, &str)] = &[(1, "gzip"), (2, "zlib"), (3, "none"), (4, "lz4"), (127, "custom")];

pub fn mca() -> Template {
    Template::new(
        "mca",
        T::structure(
            "Region",
            vec![
                ("locations", T::array(location(), E::lit(1024))),
                // Seconds since 1970, for the last time each chunk was saved.
                ("timestamps", T::array(T::u32(Big), E::lit(1024))),
                ("chunks", T::pointer_list_sized("locations", &["at"], Anchor::File, E::lit(0), chunk()).skipping_zero()),
            ],
        ),
    )
}

/// Where one chunk is, in sectors of 4096 bytes, and the offset that works out
/// to. All zeroes means the chunk is not in the file.
fn location() -> T {
    T::inline_structure(
        "Location",
        vec![
            ("sector", T::UInt { bits: 24, endian: Big }),
            ("sectors", T::u8()),
            ("at", T::computed(E::field("sector").mul(E::lit(4096)))),
        ],
    )
    .counted_as("chunk")
}

/// A chunk: its length, how it was compressed, and that many bytes of NBT.
/// The length counts the compression byte, so the data is one shorter.
fn chunk() -> T {
    T::structure(
        "Chunk",
        vec![
            ("length", T::u32(Big)),
            ("compression", T::enumeration("Compression", T::u8(), COMPRESSION)),
            ("data", T::bytes(E::field("length").sub(E::lit(1)))),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    /// A region holding two chunks, in sectors 2 and 3.
    fn region() -> Vec<u8> {
        let mut v = vec![0u8; 8192];
        for (i, (sector, len)) in [(2u32, 1u8), (3, 1)].into_iter().enumerate() {
            let at = i * 4;
            v[at..at + 3].copy_from_slice(&sector.to_be_bytes()[1..]);
            v[at + 3] = len;
            let ts = 1_700_000_000u32 + i as u32;
            v[4096 + at..4096 + at + 4].copy_from_slice(&ts.to_be_bytes());
        }
        v.resize(4096 * 4, 0);
        // Sector 2: a zlib chunk of eight bytes.
        v[8192..8196].copy_from_slice(&9u32.to_be_bytes());
        v[8196] = 2;
        v[8197..8205].copy_from_slice(&[0x78; 8]);
        // Sector 3: an uncompressed one of four.
        v[12288..12292].copy_from_slice(&5u32.to_be_bytes());
        v[12292] = 3;
        v
    }

    #[test]
    fn a_location_places_its_chunk_by_the_sector_it_names() {
        let d = Document::new(MemSource(region()));
        let mut ev = Evaluator::new(mca());
        assert_eq!(ev.node(&d, &[0]).unwrap().child_count, 1024);
        assert_eq!(ev.node(&d, &[0, 0, 0]).unwrap().value, Value::UInt(2));
        assert_eq!(ev.node(&d, &[0, 0, 2]).unwrap().value, Value::Int(8192));
        assert_eq!(ev.node(&d, &[1, 0]).unwrap().value, Value::UInt(1_700_000_000));

        let first = ev.node(&d, &[2, 0]).unwrap();
        assert_eq!(first.offset_bits, 8192 * 8);
        assert_eq!(ev.node(&d, &[2, 0, 1]).unwrap().value, Value::Enum { raw: 2, name: Some("zlib".into()), hex: false });
        assert_eq!(ev.node(&d, &[2, 0, 2]).unwrap().size_bits, 8 * 8);
        assert_eq!(ev.node(&d, &[2, 1]).unwrap().offset_bits, 12288 * 8);
    }

    #[test]
    fn a_chunk_the_world_never_reached_covers_no_bytes() {
        let d = Document::new(MemSource(region()));
        let mut ev = Evaluator::new(mca());
        // Every entry keeps its place in the list, generated or not.
        assert_eq!(ev.node(&d, &[2]).unwrap().child_count, 1024);
        // The third one is all zeroes, and it lands on nothing rather than on
        // the location table at the front of the file.
        let ungenerated = ev.node(&d, &[2, 2]).unwrap();
        assert_eq!(ungenerated.size_bits, 0);
        assert_eq!(ungenerated.offset_bits, ev.node(&d, &[2]).unwrap().offset_bits);
    }
}
