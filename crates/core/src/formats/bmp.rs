//! BMP: a file header, a DIB header whose own length says which one it is, a
//! colour table, and then the pixels the file header points at.
//!
//! Every DIB header opens with its size in bytes, and that number is the only
//! thing saying which of the five layouts follows. So there is a switch on it:
//! an unknown size still covers the right bytes, it just has nothing to call
//! the fields.
//!
//! The colour table sits with the header rather than after it, because it is
//! the header that says how many colours are in it, and only a field beside
//! them can ask. Windows calls the two together a BITMAPINFO for the same
//! reason. How many there are is `colours_used`, or, when that is zero, as
//! many as the depth can index: two at one bit, sixteen at four, 256 at eight,
//! and none at all above that, where a pixel carries its own colour.
//!
//! Everything from the header to `pixel_offset` is one window, so a writer
//! that aligns its pixel data leaves room the table does not fill and that
//! room reads as what it is. Before, it was counted as part of the table.

use crate::template::{Encoding, Endian::*, Expr as E, StrLen, Template, Ty as T};

/// How the pixels are stored. The JPEG and PNG cases mean the bitmap is a
/// whole file of another format, embedded.
const COMPRESSION: &[(i128, &str)] = &[
    (0, "none"),
    (1, "rle8"),
    (2, "rle4"),
    (3, "bitfields"),
    (4, "jpeg"),
    (5, "png"),
    (6, "alpha bitfields"),
];

pub fn bmp() -> Template {
    Template::new(
        "bmp",
        T::structure(
            "BMP",
            vec![
                ("magic", T::text(StrLen::Fixed(E::lit(2)), Encoding::Ascii)),
                ("file_size", T::u32(Little)),
                ("reserved1", T::u16(Little)),
                ("reserved2", T::u16(Little)),
                ("pixel_offset", T::u32(Little)),
                ("dib_size", T::u32(Little)),
                // The header and its colour table, in the room the file header
                // leaves before the pixels. A table longer than that room is a
                // broken file and says so rather than reading into the image.
                (
                    "dib",
                    T::sized(
                        // From here, which is past the size just read, to
                        // where the file header said the pixels begin.
                        E::field("pixel_offset").sub(E::lit(18)),
                        T::switch(E::field("dib_size"), vec![(12, core_info())], info()),
                    ),
                ),
                ("pixels", T::bytes(E::Remaining)),
            ],
        ),
    )
}

/// Windows 3.0 and everything since. A negative height means the rows are
/// written top down, which is why it is signed.
///
/// 40, 52, 56, 108 and 124 all start with the same forty bytes and add fields
/// on the end, so one case reads them all and what a later version added stays
/// whole.
fn info() -> T {
    T::structure(
        "BitmapInfo",
        vec![
            ("width", T::i32(Little)),
            ("height", T::i32(Little)),
            ("planes", T::u16(Little)),
            ("bits_per_pixel", T::u16(Little)),
            ("compression", T::enumeration("Compression", T::u32(Little), COMPRESSION)),
            ("image_size", T::u32(Little)),
            ("x_pixels_per_metre", T::i32(Little)),
            ("y_pixels_per_metre", T::i32(Little)),
            ("colours_used", T::u32(Little)),
            ("colours_important", T::u32(Little)),
            // Whatever a later version of the header added. The size read
            // before the header is what says how much that is: the ten fields
            // above take 36 of the bytes it counts, and it counts itself too.
            ("extra", T::bytes(E::field("dib_size").sub(E::lit(40)))),
            (
                "palette",
                T::switch(E::field("colours_used"), vec![(0, by_depth(quad()))], T::array(quad(), E::field("colours_used"))),
            ),
            // What the file header leaves between the table and the pixels.
            ("padding", T::bytes(E::Remaining)),
        ],
    )
}

/// OS/2 1.x, where the dimensions are sixteen bits, there is nothing else, and
/// a colour is three bytes rather than four.
fn core_info() -> T {
    T::structure(
        "BitmapCoreInfo",
        vec![
            ("width", T::u16(Little)),
            ("height", T::u16(Little)),
            ("planes", T::u16(Little)),
            ("bits_per_pixel", T::u16(Little)),
            // No count to go by: this header has none, so the depth decides.
            ("palette", by_depth(triple())),
            ("padding", T::bytes(E::Remaining)),
        ],
    )
}

/// As many colours as the depth can index, for a file that does not say.
fn by_depth(entry: T) -> T {
    let n = |count: i128| T::array(entry.clone(), E::lit(count));
    T::switch(
        E::field("bits_per_pixel"),
        // Two bits is OS/2 only, and the one depth Windows never had.
        vec![(1, n(2)), (2, n(4)), (4, n(16)), (8, n(256))],
        // Above eight bits a pixel carries its own colour and there is no
        // table at all.
        n(0),
    )
}

/// A colour table entry, blue first, as the hardware wanted it.
fn quad() -> T {
    T::inline_structure(
        "Bgra",
        vec![("blue", T::u8()), ("green", T::u8()), ("red", T::u8()), ("reserved", T::u8())],
    )
    .counted_as("colour")
}

/// The three-byte entry the OS/2 header uses, with no fourth byte to spare.
fn triple() -> T {
    T::inline_structure("Bgr", vec![("blue", T::u8()), ("green", T::u8()), ("red", T::u8())]).counted_as("colour")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    /// A bitmap with the header size, depth and declared colour count given,
    /// a palette of `entries` colours, and `slack` bytes before the pixels.
    fn bmp_bytes(dib_size: u32, bpp: u16, colours_used: u32, entries: usize, slack: usize) -> Vec<u8> {
        let entry = if dib_size == 12 { 3 } else { 4 };
        let mut dib = dib_size.to_le_bytes().to_vec();
        if dib_size == 12 {
            dib.extend_from_slice(&8u16.to_le_bytes());
            dib.extend_from_slice(&8u16.to_le_bytes());
            dib.extend_from_slice(&1u16.to_le_bytes());
            dib.extend_from_slice(&bpp.to_le_bytes());
        } else {
            dib.extend_from_slice(&8i32.to_le_bytes());
            dib.extend_from_slice(&(-8i32).to_le_bytes());
            dib.extend_from_slice(&1u16.to_le_bytes());
            dib.extend_from_slice(&bpp.to_le_bytes());
            dib.extend_from_slice(&0u32.to_le_bytes()); // no compression
            dib.extend_from_slice(&192u32.to_le_bytes());
            dib.extend_from_slice(&[0; 8]); // resolution
            dib.extend_from_slice(&colours_used.to_le_bytes());
            dib.extend_from_slice(&0u32.to_le_bytes());
            dib.resize(dib_size as usize, 0);
        }
        for i in 0..entries {
            let mut colour = vec![i as u8, 0x40, 0x80];
            if entry == 4 {
                colour.push(0);
            }
            dib.extend_from_slice(&colour);
        }
        dib.extend_from_slice(&vec![0; slack]);

        let mut v = b"BM".to_vec();
        v.extend_from_slice(&(14 + dib.len() as u32 + 192).to_le_bytes());
        v.extend_from_slice(&[0; 4]);
        v.extend_from_slice(&(14 + dib.len() as u32).to_le_bytes());
        v.extend_from_slice(&dib);
        v.extend_from_slice(&[0xff; 192]);
        v
    }

    #[test]
    fn a_declared_colour_count_is_what_the_table_holds() {
        let d = Document::new(MemSource(bmp_bytes(40, 8, 4, 4, 0)));
        let mut ev = Evaluator::new(bmp());
        assert_eq!(ev.node(&d, &[6, 0]).unwrap().value, Value::Int(8));
        // Written top down, which the sign says and nothing else does.
        assert_eq!(ev.node(&d, &[6, 1]).unwrap().value, Value::Int(-8));
        let palette = ev.node(&d, &[6, 11]).unwrap();
        assert_eq!(palette.child_count, 4);
        assert_eq!(palette.offset_bits, 54 * 8);
        // Blue first, which is the order the hardware wanted.
        assert_eq!(ev.node(&d, &[6, 11, 3, 0]).unwrap().value, Value::UInt(3));
        assert_eq!(ev.node(&d, &[6, 11, 3, 2]).unwrap().value, Value::UInt(0x80));
        assert_eq!(ev.node(&d, &[6, 12]).unwrap().size_bits, 0);
        assert_eq!(ev.node(&d, &[7]).unwrap().size_bits, 192 * 8);
    }

    #[test]
    fn a_count_of_zero_means_as_many_as_the_depth_can_index() {
        for (bpp, want) in [(1u16, 2u64), (4, 16), (8, 256)] {
            let d = Document::new(MemSource(bmp_bytes(40, bpp, 0, want as usize, 0)));
            let mut ev = Evaluator::new(bmp());
            assert_eq!(ev.node(&d, &[6, 11]).unwrap().child_count, want, "{bpp} bits");
        }
        // Above eight bits there is no table, whatever room is left.
        let d = Document::new(MemSource(bmp_bytes(40, 24, 0, 0, 0)));
        let mut ev = Evaluator::new(bmp());
        assert_eq!(ev.node(&d, &[6, 11]).unwrap().child_count, 0);
    }

    #[test]
    fn room_left_before_the_pixels_is_not_counted_as_colour() {
        // A writer that aligns the pixels to a multiple of eight.
        let d = Document::new(MemSource(bmp_bytes(40, 8, 4, 4, 6)));
        let mut ev = Evaluator::new(bmp());
        assert_eq!(ev.node(&d, &[6, 11]).unwrap().child_count, 4);
        assert_eq!(ev.node(&d, &[6, 12]).unwrap().size_bits, 6 * 8);
        // And the pixels still start where the file header said they do.
        assert_eq!(ev.node(&d, &[7]).unwrap().offset_bits, (54 + 16 + 6) * 8);
    }

    #[test]
    fn an_os2_bitmap_reads_the_short_header_and_its_three_byte_colours() {
        let d = Document::new(MemSource(bmp_bytes(12, 4, 0, 16, 0)));
        let mut ev = Evaluator::new(bmp());
        let h = ev.node(&d, &[6]).unwrap();
        assert_eq!(h.type_name, "BitmapCoreInfo");
        assert_eq!(ev.node(&d, &[6, 3]).unwrap().value, Value::UInt(4));
        let palette = ev.node(&d, &[6, 4]).unwrap();
        assert_eq!(palette.child_count, 16);
        assert_eq!(palette.size_bits, 16 * 3 * 8);
        assert_eq!(ev.node(&d, &[6, 4, 1, 0]).unwrap().value, Value::UInt(1));
    }

    #[test]
    fn a_later_header_keeps_the_bytes_it_added() {
        let d = Document::new(MemSource(bmp_bytes(124, 24, 0, 0, 0)));
        let mut ev = Evaluator::new(bmp());
        assert_eq!(ev.node(&d, &[6, 10]).unwrap().size_bits, (124 - 40) * 8);
        assert_eq!(ev.node(&d, &[6, 11]).unwrap().child_count, 0);
    }
}
