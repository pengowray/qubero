//! The LZ4 frame format: a magic number, two bytes of options, whatever those
//! two bytes said to expect, and then blocks until one of size zero.
//!
//! Every block is a four-byte size and that many bytes. The top bit of the
//! size says the block was stored rather than compressed, which is what an
//! encoder writes when compressing made the block larger, so the size is the
//! other thirty-one bits. A size of zero is not a block: it is the end mark,
//! and the frame is over.
//!
//! The byte after the options is a checksum of the options, which is why a
//! file with a plausible header is almost certainly one of these.
//!
//! **A file is frames, not a frame.** Concatenating two `.lz4` files gives a
//! valid one, and `lz4 -m` writes them that way, so the magic is read per
//! frame rather than once at the front. Three kinds carry it. The frame above
//! is the format as it stands. A *skippable* frame is one of sixteen magics,
//! a length, and that many bytes an application put there for its own
//! purposes, which a decoder steps over without looking; zstd borrowed the
//! idea and the shape, and its magics sit next to these in the same range. A
//! *legacy* frame is what the first releases wrote: a magic of its own and
//! then blocks of a size and that many bytes, with no descriptor in front and
//! no end mark behind, so it runs to the end of the file.
//!
//! Not read: the frames' data as literals and matches, which the codec opens
//! rather than the template dividing.

use crate::codec::Codec;
use crate::template::{Endian::{Big, Little}, Expr as E, Template, Until, Ty as T};

/// What one of these starts with.
pub const MAGIC: &[u8] = b"\x04\x22\x4d\x18";
/// What the frames of the first releases start with. A file of these is an
/// LZ4 file and nothing else reads it, so it is sniffed as one.
pub const LEGACY_MAGIC: &[u8] = b"\x02\x21\x4c\x18";

/// The three magics as the little-endian numbers a switch compares.
const FRAME: i128 = 0x184d_2204;
const LEGACY: i128 = 0x184c_2102;
/// The first of sixteen: the low nibble is the application's to choose, and
/// every one of them means the same thing to a decoder.
const SKIPPABLE: i128 = 0x184d_2a50;

/// The largest block the encoder said it would write, which is what a decoder
/// sizes its buffer by.
const BLOCK_MAX: &[(i128, &str)] = &[(4, "64 KiB"), (5, "256 KiB"), (6, "1 MiB"), (7, "4 MiB")];

/// The top bit of a block's size word, which says the block was stored as it
/// came.
const UNCOMPRESSED_BIT: i128 = 1 << 31;

pub fn lz4() -> Template {
    Template::new("lz4", T::structure("Lz4File", vec![("frames", T::repeat(frame(), Until::End))]))
}

/// One frame, of whichever of the three kinds its magic says.
///
/// A magic nobody defined ends the reading: the bytes to the end of the file
/// are one run rather than a four-byte frame out of every position of them.
/// That is what a `.lz4` with something appended to it looks like, and saying
/// so once is better than dividing it wrongly a thousand times.
fn frame() -> T {
    let mut magics: Vec<(i128, &str)> = vec![(FRAME, "frame"), (LEGACY, "legacy frame")];
    // Named by the nibble, since that is the only thing telling two of them
    // apart and an application chose it on purpose.
    const NIBBLE: [&str; 16] = [
        "skippable frame 0", "skippable frame 1", "skippable frame 2", "skippable frame 3",
        "skippable frame 4", "skippable frame 5", "skippable frame 6", "skippable frame 7",
        "skippable frame 8", "skippable frame 9", "skippable frame 10", "skippable frame 11",
        "skippable frame 12", "skippable frame 13", "skippable frame 14", "skippable frame 15",
    ];
    magics.extend(NIBBLE.iter().enumerate().map(|(i, name)| (SKIPPABLE + i as i128, *name)));
    let mut cases = vec![(FRAME, modern()), (LEGACY, legacy())];
    cases.extend(NIBBLE.iter().enumerate().map(|(i, _)| (SKIPPABLE + i as i128, skippable())));
    T::structure_named(
        "Lz4Frame",
        "magic",
        "body",
        vec![
            ("magic", T::enumeration_hex("Lz4Magic", T::u32(Little), &magics)),
            ("body", T::switch(E::field("magic"), cases, T::bytes(E::Remaining))),
        ],
    )
    .counted_as("frame")
}

/// Bytes an application put in the middle of a stream for itself. A decoder
/// steps over them; a reader looking at the file is the one person who wants
/// to see them.
fn skippable() -> T {
    T::structure(
        "Lz4SkippableFrame",
        vec![("size", T::u32(Little)), ("user_data", T::bytes(E::field("size")))],
    )
}

/// What the first releases wrote: blocks of a size and that many bytes, with
/// no descriptor in front and no end mark behind.
///
/// It runs to the end of the file, because nothing in it says where it stops.
/// A legacy frame followed by anything else is therefore read as one frame
/// swallowing the rest, which is what every legacy reader does too.
fn legacy() -> T {
    T::structure(
        "Lz4LegacyFrame",
        vec![("blocks", T::repeat(legacy_block(), Until::End))],
    )
}

/// One legacy block. Always compressed: the stored bit came in with the frame
/// format, so the size is the whole four bytes.
fn legacy_block() -> T {
    T::structure(
        "Lz4LegacyBlock",
        vec![
            ("block_size", T::u32(Little)),
            ("data", T::decoded(E::field("block_size"), Codec::Lz4Block, super::decoded_text())),
        ],
    )
    .counted_as("block")
}

fn modern() -> T {
    T::structure(
            "Lz4FrameBody",
            vec![
                // FLG, most significant bit first.
                ("version", T::UInt { bits: 2, endian: Big }),
                // Set when each block can be decompressed on its own, clear
                // when a block may refer back into the one before it.
                ("block_independence", T::UInt { bits: 1, endian: Big }),
                ("block_checksum_flag", T::UInt { bits: 1, endian: Big }),
                ("content_size_flag", T::UInt { bits: 1, endian: Big }),
                ("content_checksum_flag", T::UInt { bits: 1, endian: Big }),
                ("flg_reserved", T::UInt { bits: 1, endian: Big }),
                ("dictionary_id_flag", T::UInt { bits: 1, endian: Big }),
                // BD, which is one number and six reserved bits.
                ("bd_reserved", T::UInt { bits: 1, endian: Big }),
                ("block_max_size", T::enumeration("Lz4BlockMax", T::UInt { bits: 3, endian: Big }, BLOCK_MAX)),
                ("bd_reserved_low", T::UInt { bits: 4, endian: Big }),
                ("content_size", T::present_if(E::field("content_size_flag"), T::u64(Little))),
                ("dictionary_id", T::present_if(E::field("dictionary_id_flag"), T::u32(Little))),
                // A byte of the xxhash-32 of everything between the magic and
                // here, which is what makes a header this small checkable.
                ("header_checksum", T::u8()),
                // Blocks, up to and including the end mark: a size of zero
                // with nothing after it.
                ("blocks", T::repeat(block(), Until::FieldValue { field: "block_header".into(), value: 0 })),
                ("content_checksum", T::present_if(E::field("content_checksum_flag"), T::u32(Little))),
            ],
        )
}

/// One block, or the end mark, which is a block whose size word is zero and
/// which has nothing else in it at all.
fn block() -> T {
    T::structure(
        "Lz4Block",
        vec![
            ("block_header", T::u32(Little)),
            ("uncompressed", T::computed(E::field("block_header").bit(31))),
            ("block_size", T::computed(E::field("block_header").sub(E::field("block_header").bit(31).mul(E::lit(UNCOMPRESSED_BIT))))),
            // A compressed block is one LZ4 block and opens on its own; a
            // stored block is the bytes as they came and has nothing to open.
            (
                "data",
                T::switch(
                    E::field("uncompressed"),
                    vec![(1, T::bytes(E::field("block_size")))],
                    T::decoded(E::field("block_size"), Codec::Lz4Block, super::decoded_text()),
                ),
            ),
            // Only a real block has one: the end mark is the size word and
            // nothing more.
            (
                "block_checksum",
                T::present_if(
                    E::field("block_checksum_flag").mul(E::lit(0).less_than(E::field("block_size"))),
                    T::u32(Little),
                ),
            ),
        ],
    )
    .counted_as("block")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{document::Document, eval::Evaluator, source::MemSource};

    /// A frame of one stored block, with a content size and a content
    /// checksum but no per-block checksum.
    pub fn frame(content: &[u8]) -> Vec<u8> {
        let mut v = MAGIC.to_vec();
        v.push(0b0110_1100); // version 1, blocks independent, both sizes checked
        v.push(0b0100_0000); // 64 KiB blocks
        v.extend_from_slice(&(content.len() as u64).to_le_bytes());
        v.push(0x00); // the header checksum, which nothing here computes
        v.extend_from_slice(&((content.len() as u32) | 0x8000_0000).to_le_bytes());
        v.extend_from_slice(content);
        v.extend_from_slice(&0u32.to_le_bytes()); // the end mark
        v.extend_from_slice(&0u32.to_le_bytes()); // the content checksum
        v
    }

    /// A frame of one compressed block, which is what an encoder writes when
    /// compressing helped.
    fn packed_frame(content: &[u8]) -> Vec<u8> {
        let block = lz4_flex::block::compress(content);
        let mut v = MAGIC.to_vec();
        v.push(0b0110_1100);
        v.push(0b0100_0000);
        v.extend_from_slice(&(content.len() as u64).to_le_bytes());
        v.push(0x00);
        v.extend_from_slice(&(block.len() as u32).to_le_bytes());
        v.extend_from_slice(&block);
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes());
        v
    }

    /// The body of frame `n`, found by name rather than by counting: the
    /// fields of a descriptor move whenever a flag turns one on.
    fn body(e: &mut Evaluator, d: &Document<MemSource>, n: usize) -> Vec<usize> {
        e.child_named(d, &[0, n], "body").unwrap().expect("every frame has a body")
    }

    fn field(e: &mut Evaluator, d: &Document<MemSource>, at: &[usize], name: &str) -> Vec<usize> {
        e.child_named(d, at, name).unwrap().unwrap_or_else(|| panic!("no field called {name}"))
    }

    /// A compressed block opens into what it holds; the block keeps the length
    /// it has in the file.
    #[test]
    fn a_compressed_block_reads_as_the_text_inside_it() {
        let d = Document::new(MemSource(packed_frame(b"hello hello hello lz4")));
        let mut e = Evaluator::new(lz4());
        let b = body(&mut e, &d, 0);
        let blocks = field(&mut e, &d, &b, "blocks");
        let first = [blocks.clone(), vec![0]].concat();
        let data = field(&mut e, &d, &first, "data");
        let n = e.node(&d, &data).unwrap();
        assert_eq!(n.type_name, "lz4");
        assert_eq!((n.space, n.child_count, n.refused), (0, 2, None));
        let text = e.node(&d, &[data, vec![0, 0]].concat()).unwrap();
        assert_eq!(text.value, crate::eval::Value::Str("hello hello hello lz4".into()));
        assert_eq!((text.offset_bits, text.space), (0, 1));
        assert!(!text.editable);
    }

    #[test]
    fn blocks_run_to_the_end_mark_and_a_stored_block_says_so() {
        let d = Document::new(MemSource(frame(b"hello lz4")));
        let mut e = Evaluator::new(lz4());
        let b = body(&mut e, &d, 0);
        let size = field(&mut e, &d, &b, "content_size");
        assert_eq!(e.node(&d, &size).unwrap().value.as_int(), Some(9));
        let blocks = field(&mut e, &d, &b, "blocks");
        // Two elements: the block, and the end mark that stopped the run.
        assert_eq!(e.node(&d, &blocks).unwrap().child_count, 2);
        let first = [blocks.clone(), vec![0]].concat();
        let stored = field(&mut e, &d, &first, "uncompressed");
        assert_eq!(e.node(&d, &stored).unwrap().value.as_int(), Some(1));
        let len = field(&mut e, &d, &first, "block_size");
        assert_eq!(e.node(&d, &len).unwrap().value.as_int(), Some(9));
        assert_eq!(e.node(&d, &[blocks, vec![1]].concat()).unwrap().size_bits, 4 * 8);
        let sum = field(&mut e, &d, &b, "content_checksum");
        assert_eq!(e.node(&d, &sum).unwrap().size_bits, 4 * 8);
    }

    /// Two files put end to end are one file, which is what `lz4 -m` writes
    /// and what `cat` gives. Reading one frame and calling the rest bytes was
    /// the shape this template had.
    #[test]
    fn a_file_of_two_frames_reads_as_two() {
        let mut v = frame(b"the first");
        v.extend_from_slice(&packed_frame(b"the second, packed this time"));
        let d = Document::new(MemSource(v));
        let mut e = Evaluator::new(lz4());
        assert_eq!(e.node(&d, &[0]).unwrap().child_count, 2);
        for (n, want) in [(0usize, 9i128), (1, 28)] {
            let b = body(&mut e, &d, n);
            let size = field(&mut e, &d, &b, "content_size");
            assert_eq!(e.node(&d, &size).unwrap().value.as_int(), Some(want), "frame {n}");
        }
    }

    /// Bytes an application put in the middle of a stream for itself. A
    /// decoder steps over them; the frame after one still reads.
    #[test]
    fn a_skippable_frame_is_stepped_over_and_the_next_frame_still_reads() {
        let note = b"put here by something that is not lz4";
        let mut v = (SKIPPABLE as u32 + 7).to_le_bytes().to_vec();
        v.extend_from_slice(&(note.len() as u32).to_le_bytes());
        v.extend_from_slice(note);
        v.extend_from_slice(&frame(b"after the note"));
        let d = Document::new(MemSource(v));
        let mut e = Evaluator::new(lz4());
        assert_eq!(e.node(&d, &[0]).unwrap().child_count, 2);
        let magic = e.node(&d, &[0, 0, 0]).unwrap();
        assert_eq!(magic.value, crate::eval::Value::Enum {
            raw: SKIPPABLE + 7,
            name: Some("skippable frame 7".into()),
            hex: true
        });
        let b = body(&mut e, &d, 0);
        let data = field(&mut e, &d, &b, "user_data");
        assert_eq!(e.node(&d, &data).unwrap().size_bits, note.len() as u64 * 8);
        let b = body(&mut e, &d, 1);
        let size = field(&mut e, &d, &b, "content_size");
        assert_eq!(e.node(&d, &size).unwrap().value.as_int(), Some(14));
    }

    /// What the first releases wrote: a magic of its own, and then blocks of a
    /// size and that many bytes with no descriptor and no end mark.
    #[test]
    fn a_legacy_frame_is_blocks_to_the_end_of_the_file() {
        let (one, two) = (b"the first legacy block".as_slice(), b"and a second one".as_slice());
        let mut v = LEGACY_MAGIC.to_vec();
        for part in [one, two] {
            let block = lz4_flex::block::compress(part);
            v.extend_from_slice(&(block.len() as u32).to_le_bytes());
            v.extend_from_slice(&block);
        }
        let d = Document::new(MemSource(v));
        let mut e = Evaluator::new(lz4());
        let b = body(&mut e, &d, 0);
        let blocks = field(&mut e, &d, &b, "blocks");
        assert_eq!(e.node(&d, &blocks).unwrap().child_count, 2);
        for (n, want) in [(0usize, one), (1usize, two)] {
            let data = field(&mut e, &d, &[blocks.clone(), vec![n]].concat(), "data");
            let text = e.node(&d, &[data, vec![0, 0]].concat()).unwrap();
            assert_eq!(text.value, crate::eval::Value::Str(String::from_utf8_lossy(want).into_owned()));
        }
    }
}
