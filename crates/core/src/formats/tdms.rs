//! NI TDMS: the file LabVIEW, DAQmx and the rest of National Instruments'
//! software write measurements into. Read from NI's "TDMS File Format
//! Internal Structure", checked against npTDMS.
//!
//! A file is a run of segments, each a 28-byte lead-in and then as much as the
//! lead-in says follows it. Nothing else holds the file together: no header,
//! no directory, no end record. A writer appends a segment every time it
//! flushes, so a long recording is thousands of them.
//!
//! **The lead-in.** Four letters, `TDSm` in a data file and `TDSh` in the
//! `.tdms_index` file written beside one; a mask of flags NI calls the table of
//! contents; a version; and two offsets, both counted from the end of the
//! lead-in: where the next segment starts and where this one's raw data does.
//! The mask is always little-endian, and one of its bits says whether
//! everything after it is. NI's document says so in as many words, "including
//! the lead in", and the big-endian file npTDMS keeps for its tests is written
//! that way: its mask reads `4e 00 00 00` and its version `00 00 12 69`. So the
//! mask is read first and the rest of the segment is a switch with one arm per
//! byte order, the way a GWF frame file's check word picks its arm.
//!
//! A next-segment offset of all ones is how a segment the writer never came
//! back to finish is marked. Clamping the offset to what is left of the file
//! reads that case with no special arm: the segment runs to the end.
//!
//! An index file is a copy of the data file with the raw data left out, so its
//! segments keep the data file's offsets and each one ends where its metadata
//! does.

use crate::template::{Endian, Endian::*, Expr as E, Template, Ty as T, Until};

/// The bits of the table of contents that mean something, by NI's names less
/// the `kToc`. Bit 0 and bit 4 are unused.
const TOC: &[(u32, &str)] = &[
    (1, "metadata"),
    (2, "new object list"),
    (3, "raw data"),
    (5, "interleaved"),
    (6, "big-endian"),
    (7, "DAQmx raw data"),
];

/// The two versions NI has written. 4713 is TDMS 2.0, which added interleaved
/// data, the big-endian flag and DAQmx raw data.
const VERSION: &[(i128, &str)] = &[(4712, "TDMS 1.0"), (4713, "TDMS 2.0")];

/// A next-segment offset of every bit set.
const UNFINISHED: i128 = 0xFFFF_FFFF_FFFF_FFFF;

/// `TDSm` and `TDSh`, the four letters read as one big-endian number.
const DATA_TAG: i128 = 0x5444_536D;
const INDEX_TAG: i128 = 0x5444_5368;

pub fn tdms() -> Template {
    Template::new("tdms", T::repeat(segment(), Until::End))
}

/// One segment: the tag, the table of contents, and everything the table says
/// about the byte order of the rest.
fn segment() -> T {
    T::structure_named(
        "Segment",
        "",
        "contents",
        vec![
            // Four letters, read as the number they spell so that the rest of
            // the segment can ask which of the two it is. Anything else is
            // not a segment, and reads as a number nothing named.
            ("tag", T::enumeration("SegmentTag", T::u32(Big), &[(DATA_TAG, "TDSm"), (INDEX_TAG, "TDSh")])),
            ("toc", T::flags("TableOfContents", T::u32(Little), TOC)),
            (
                "contents",
                T::switch(E::field("toc").bit(6), vec![(1, contents(Big))], contents(Little)),
            ),
        ],
    )
    .field_doc(
        "tag",
        "TDSm opens a segment of a .tdms file and TDSh a segment of its .tdms_index file, which is the same segment with its raw data left out.",
    )
    .field_doc("toc", "A mask of kToc flags saying what the segment holds. Always little-endian, whatever the big-endian flag in it says about the rest.")
}

/// The rest of a segment, every number in it read the way `e` says.
fn contents(e: Endian) -> T {
    let offset = E::field("raw_data_offset");
    // How much of the file this segment's raw data takes. An index file holds
    // none of it, though its offsets still count it. What the next-segment
    // offset claims past the metadata, never below nothing and never past the
    // end of the file: a writer that stopped wrote all ones there, and the
    // clamp is what reads its segment to the end of the file.
    let raw_size = || E::field("next_segment_offset").sub(offset.clone()).at_least(E::lit(0)).at_most(E::Remaining);
    let is_data = || E::field("tag").not_equal(E::lit(INDEX_TAG));
    T::structure(
        "SegmentContents",
        vec![
            ("version", T::enumeration("TdmsVersion", T::u32(e), VERSION)),
            (
                "next_segment_offset",
                T::enumeration("NextSegmentOffset", T::u64(e), &[(UNFINISHED, "unfinished")]),
            ),
            ("raw_data_offset", T::u64(e)),
            ("metadata", T::sized(offset.clone().at_least(E::lit(0)).at_most(E::Remaining), T::bytes(E::Remaining))),
            ("raw_data", T::when(is_data().both(E::lit(0).less_than(raw_size())), T::sized(raw_size(), T::bytes(E::Remaining)))),
        ],
    )
    .field_doc("next_segment_offset", "Where the next segment starts, in bytes from the end of this lead-in. All ones when the writer stopped before it could come back and fill it in, which can only happen to the last segment.")
    .field_doc("raw_data_offset", "Where this segment's raw data starts, in bytes from the end of the lead-in: how long its metadata is.")
    .machinery(&["next_segment_offset", "raw_data_offset"])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    /// A lead-in and what follows it, `big` deciding every number after the
    /// mask.
    fn segment(toc: u32, big: bool, meta: &[u8], raw: &[u8], next: Option<u64>) -> Vec<u8> {
        let mut b = b"TDSm".to_vec();
        b.extend_from_slice(&(toc | if big { 1 << 6 } else { 0 }).to_le_bytes());
        let w = |v: u64, n: usize| -> Vec<u8> {
            let bytes = if big { v.to_be_bytes() } else { v.to_le_bytes() };
            if big { bytes[8 - n..].to_vec() } else { bytes[..n].to_vec() }
        };
        b.extend(w(4713, 4));
        b.extend(w(next.unwrap_or((meta.len() + raw.len()) as u64), 8));
        b.extend(w(meta.len() as u64, 8));
        b.extend_from_slice(meta);
        b.extend_from_slice(raw);
        b
    }

    #[test]
    fn a_segment_is_as_long_as_its_lead_in_says() {
        let mut file = segment(0b1110, false, &[0; 12], &[7; 16], None);
        file.extend(segment(0b1000, true, &[], &[9; 8], None));
        let d = Document::new(MemSource(file.clone()));
        let mut ev = Evaluator::new(tdms());
        assert_eq!(ev.node(&d, &[]).unwrap().child_count, 2);
        assert_eq!(ev.node(&d, &[0, 2, 0]).unwrap().value, Value::Enum { raw: 4713, name: Some("TDMS 2.0".into()), hex: false });
        assert_eq!(ev.node(&d, &[0, 2, 3]).unwrap().size_bits, 12 * 8);
        assert_eq!(ev.node(&d, &[0, 2, 4]).unwrap().size_bits, 16 * 8);
        // The second is big-endian after its mask, and reads the same.
        assert_eq!(ev.node(&d, &[1, 2, 1]).unwrap().value.as_int(), Some(8));
        let last = ev.node(&d, &[1]).unwrap();
        assert_eq!(last.offset_bits + last.size_bits, file.len() as u64 * 8);
    }

    #[test]
    fn a_segment_the_writer_never_finished_runs_to_the_end_of_the_file() {
        let file = segment(0b1000, false, &[], &[1; 40], Some(u64::MAX));
        let d = Document::new(MemSource(file.clone()));
        let mut ev = Evaluator::new(tdms());
        assert_eq!(
            ev.node(&d, &[0, 2, 1]).unwrap().value,
            Value::Enum { raw: UNFINISHED, name: Some("unfinished".into()), hex: false }
        );
        assert_eq!(ev.node(&d, &[0, 2, 4]).unwrap().size_bits, 40 * 8);
    }

    #[test]
    fn an_index_file_segment_ends_with_its_metadata() {
        // The offsets are the data file's: eight bytes of metadata and a
        // hundred of raw data that the index file does not hold.
        let mut file = segment(0b1110, false, &[0; 8], &[], Some(108));
        file.extend(segment(0b1110, false, &[0; 8], &[], Some(108)));
        file[..4].copy_from_slice(b"TDSh");
        file[36..40].copy_from_slice(b"TDSh");
        let n = file.len();
        let d = Document::new(MemSource(file));
        let mut ev = Evaluator::new(tdms());
        assert_eq!(ev.node(&d, &[]).unwrap().child_count, 2);
        assert!(ev.node(&d, &[0, 2, 4]).unwrap().absent);
        let last = ev.node(&d, &[1]).unwrap();
        assert_eq!(last.offset_bits + last.size_bits, n as u64 * 8);
    }
}
