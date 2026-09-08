//! Ogg: pages that carry someone else's stream, and say nothing about it.
//!
//! A page is twenty-seven bytes of header, a table of segment lengths, and
//! the segments those lengths measure. What is inside the segments is a
//! codec's business, not the container's: Vorbis, Opus, FLAC and Theora all
//! travel this way and an Ogg reader knows none of them. So the segments are
//! read as the runs they are and named by the stream they belong to.
//!
//! **The lacing table is the whole trick.** Each byte says how long a segment
//! is, and 255 means "and there is more of this one in the next segment", so
//! a packet is as long as the run of 255s before its terminating byte. A
//! packet that ends exactly on a multiple of 255 writes a zero to say so,
//! which is why a page can hold a segment of no bytes and mean something by
//! it. A packet may also run off the end of a page and continue on the next,
//! which is what the `continued` flag says.
//!
//! **A page seals itself.** The CRC-32 sits inside the run it covers, and
//! the writer summed four zeroes where the number was going to go, so the
//! check reads those bytes as zeroes too. That is [`Check::blanking`], and
//! this is the second format to need it after tar; the sum itself is the
//! unreflected CRC-32, which is a different number from the reflected one
//! every other format here writes. See [`Checksum::Crc32Ogg`].
//!
//! **Streams are interleaved.** Several logical streams may share one file,
//! each with its own serial number, and a player picks the pages it wants by
//! that number. Pages of different streams alternate, so the pages are read
//! in the order they were written rather than gathered by serial: the file is
//! what it is, and grouping it would put the reading somewhere the bytes are
//! not.

use crate::template::{Check, Checksum, Covers, Endian::Little, Expr as E, Template, Until, Ty as T};

/// What every page starts with.
pub const MAGIC: &[u8] = b"OggS";

/// The header before the lacing table: the magic, the version, the flags, the
/// granule position, the serial, the sequence, the sum, and the count.
const HEADER: i128 = 27;

/// What the flags byte says about the page, one bit each.
const FLAGS: &[(u32, &str)] = &[
    // The first packet here is the tail of one that began on an earlier page.
    (0, "continued packet"),
    // The first page of this logical stream.
    (1, "first page"),
    // The last one.
    (2, "last page"),
];

pub fn ogg() -> Template {
    Template::new("ogg", T::structure("OggStream", vec![("pages", T::repeat(page(), Until::End))]))
}

fn page() -> T {
    T::structure_named(
        "OggPage",
        "sequence",
        "segments",
        vec![
            ("magic", T::magic(MAGIC)),
            ("version", T::u8()),
            ("header_type", T::flags("OggHeaderType", T::u8(), FLAGS)),
            // How far into the stream the last packet finishing on this page
            // reaches, in whatever unit the codec counts in: samples for
            // Vorbis and Opus, frames for Theora. -1 says no packet finishes
            // here, which is why it is signed.
            ("granule_position", T::Int { bits: 64, endian: Little }),
            // Which logical stream this page belongs to. Several share a file
            // and a player follows one of them.
            ("serial_number", T::u32(Little)),
            ("page_sequence", T::u32(Little)),
            ("crc32", T::u32(Little)),
            ("page_segments", T::u8()),
            // One byte per segment, and the length of the segment. A 255 says
            // the packet carries on into the next segment, so a packet's
            // length is the run of 255s before its terminating byte.
            ("segment_table", T::array(T::u8(), E::field("page_segments"))),
            // What the table measures, as one run. Divided into packets it is
            // not: the segments of one packet are contiguous and the codec is
            // what says where a packet means anything, and a container that
            // cut them up would be claiming to know a codec it does not.
            ("segments", T::bytes(E::sum_of("segment_table"))),
        ],
    )
    .counted_as("page")
    .field_check("crc32", page_crc())
}

/// The sum a page writes about itself: every byte of the page, with the four
/// its own checksum sits in read as zeroes.
///
/// The writer had to put something there while adding up, and zeroes are what
/// the format says to put, so a reader adding those bytes up as they stand
/// gets another number entirely. The run is the page from its first byte,
/// since offsets in a check count from the structure the field sits in.
fn page_crc() -> Check {
    let length = E::lit(HEADER).add(E::field("page_segments")).add(E::sum_of("segment_table"));
    Check::of(Checksum::Crc32Ogg, Covers::Run { at: E::lit(0), len: length }).blanking(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{checksum::crc32_msb, document::Document, eval::Evaluator, source::MemSource};

    /// A page carrying `segments`, with the sum the format asks for.
    fn page_bytes(serial: u32, sequence: u32, flags: u8, segments: &[&[u8]]) -> Vec<u8> {
        let mut table = Vec::new();
        let mut body = Vec::new();
        for s in segments {
            // Only lengths under 255 here: a longer segment is a lacing run,
            // which the tests below build by hand where they mean one.
            table.push(s.len() as u8);
            body.extend_from_slice(s);
        }
        let mut v = MAGIC.to_vec();
        v.push(0);
        v.push(flags);
        v.extend_from_slice(&0i64.to_le_bytes());
        v.extend_from_slice(&serial.to_le_bytes());
        v.extend_from_slice(&sequence.to_le_bytes());
        // Zeroes while the sum runs, which is what the format says to write.
        v.extend_from_slice(&0u32.to_le_bytes());
        v.push(table.len() as u8);
        v.extend_from_slice(&table);
        v.extend_from_slice(&body);
        let sum = crc32_msb(&v, 0, 0);
        v[22..26].copy_from_slice(&sum.to_le_bytes());
        v
    }

    fn at(e: &mut Evaluator, d: &Document<MemSource>, n: usize, name: &str) -> Vec<usize> {
        e.child_named(d, &[0, n], name).unwrap().unwrap_or_else(|| panic!("no field called {name}"))
    }

    #[test]
    fn a_page_is_its_header_its_lacing_table_and_what_the_table_measures() {
        let bytes = page_bytes(0x1234, 0, 0x02, &[b"one", b"two and a bit"]);
        let d = Document::new(MemSource(bytes.clone()));
        let mut e = Evaluator::new(ogg());
        let count = at(&mut e, &d, 0, "page_segments");
        assert_eq!(e.node(&d, &count).unwrap().value.as_int(), Some(2));
        let segments = at(&mut e, &d, 0, "segments");
        assert_eq!(e.node(&d, &segments).unwrap().size_bits, (3 + 13) * 8);
        let serial = at(&mut e, &d, 0, "serial_number");
        assert_eq!(e.node(&d, &serial).unwrap().value.as_int(), Some(0x1234));
        // The whole page and nothing after it.
        assert_eq!(e.node(&d, &[0, 0]).unwrap().size_bits, bytes.len() as u64 * 8);
    }

    /// The sum covers the page including the four bytes it sits in, read as
    /// zeroes. Adding those bytes up as they stand gives another number, which
    /// is the whole reason `blanking` exists.
    #[test]
    fn a_page_sums_to_what_it_wrote_down_with_its_own_four_bytes_read_as_zeroes() {
        let bytes = page_bytes(7, 3, 0, &[b"a packet of some length"]);
        let d = Document::new(MemSource(bytes.clone()));
        let mut e = Evaluator::new(ogg());
        let sum = at(&mut e, &d, 0, "crc32");
        let info = e.check_of(&d, &sum).unwrap().expect("a page checks itself");
        assert_eq!(info.algorithm, "crc32");
        assert_eq!(info.over, Some((0, bytes.len() as u64)));
        let blanked = info.blanked.expect("its own four bytes are read as something else");
        assert_eq!((blanked.at, blanked.len, blanked.byte), (22, 4, 0));
        let v = e.run_check(&d, &sum).unwrap().expect("the sum can be taken");
        assert!(v.ok, "computed {}, stored {}", v.computed, v.stored);
        // And the sum taken over the bytes as they sit there is a different
        // number, so the substitution is doing the work.
        assert_ne!(crc32_msb(&bytes, 0, 0), crc32_msb(&{ let mut z = bytes.clone(); z[22..26].fill(0); z }, 0, 0));
    }

    #[test]
    fn a_page_somebody_edited_no_longer_sums() {
        let mut bytes = page_bytes(7, 3, 0, &[b"a packet of some length"]);
        let last = bytes.len() - 1;
        bytes[last] ^= 0x20;
        let d = Document::new(MemSource(bytes));
        let mut e = Evaluator::new(ogg());
        let sum = at(&mut e, &d, 0, "crc32");
        let v = e.run_check(&d, &sum).unwrap().expect("the check still applies");
        assert!(!v.ok, "computed {}, stored {}", v.computed, v.stored);
    }

    /// Several logical streams share a file and their pages alternate, so the
    /// reading follows the file rather than gathering by serial number.
    #[test]
    fn pages_of_two_streams_read_in_the_order_they_were_written() {
        let mut v = page_bytes(0xaa, 0, 0x02, &[b"stream one"]);
        v.extend_from_slice(&page_bytes(0xbb, 0, 0x02, &[b"stream two"]));
        v.extend_from_slice(&page_bytes(0xaa, 1, 0x00, &[b"one again"]));
        let d = Document::new(MemSource(v));
        let mut e = Evaluator::new(ogg());
        assert_eq!(e.node(&d, &[0]).unwrap().child_count, 3);
        for (n, serial) in [(0usize, 0xaa), (1, 0xbb), (2, 0xaa)] {
            let p = at(&mut e, &d, n, "serial_number");
            assert_eq!(e.node(&d, &p).unwrap().value.as_int(), Some(serial), "page {n}");
            let sum = at(&mut e, &d, n, "crc32");
            assert!(e.run_check(&d, &sum).unwrap().expect("a verdict").ok, "page {n}");
        }
    }

    /// A segment of 255 says the packet carries on into the next one, and a
    /// zero says a packet ended exactly on a boundary. Both are lengths the
    /// table states and the run measures; nothing here tries to divide the
    /// segments into packets.
    #[test]
    fn a_lacing_run_and_an_empty_segment_are_both_measured_by_the_table() {
        let long = vec![b'x'; 255];
        let bytes = page_bytes(1, 0, 0, &[&long, b"tail", b""]);
        let d = Document::new(MemSource(bytes));
        let mut e = Evaluator::new(ogg());
        let segments = at(&mut e, &d, 0, "segments");
        assert_eq!(e.node(&d, &segments).unwrap().size_bits, (255 + 4) * 8);
        let sum = at(&mut e, &d, 0, "crc32");
        assert!(e.run_check(&d, &sum).unwrap().expect("a verdict").ok);
    }
}
