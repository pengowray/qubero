//! BMP: a file header, a DIB header whose own length says which one it is,
//! and then the pixels the file header points at.
//!
//! Every DIB header opens with its size in bytes, and that number is the only
//! thing saying which of the five layouts follows. So the header is a window
//! of exactly that many bytes with a switch inside it: an unknown size still
//! covers the right bytes, it just has nothing to call the fields.

use crate::template::{Endian::*, Expr as E, StrLen, Encoding, Template, Ty as T};

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
                ("dib", T::sized(E::field("dib_size").sub(E::lit(4)), header())),
                // The palette, if any, sits between the header and the offset
                // the file header gave for the pixels.
                ("palette", T::bytes(E::field("pixel_offset").sub(E::lit(14)).sub(E::field("dib_size")))),
                ("pixels", T::bytes(E::Remaining)),
            ],
        ),
    )
}

/// The DIB header, whichever of the five it is. The size read just before it
/// is what says which, and it is also what bounds the window this sits in, so
/// a size nobody has seen before still covers the right bytes.
fn header() -> T {
    T::switch(
        E::field("dib_size"),
        vec![(12, core_header())],
        // 40, 52, 56, 108 and 124 all start with the same forty bytes and add
        // fields on the end, so one case reads them all and what a later
        // version added stays whole.
        info_header(),
    )
}

/// OS/2 1.x, where the dimensions are sixteen bits and there is nothing else.
fn core_header() -> T {
    T::structure(
        "BitmapCoreHeader",
        vec![
            ("width", T::u16(Little)),
            ("height", T::u16(Little)),
            ("planes", T::u16(Little)),
            ("bits_per_pixel", T::u16(Little)),
        ],
    )
}

/// Windows 3.0 and everything since. A negative height means the rows are
/// written top down, which is why it is signed.
fn info_header() -> T {
    T::structure(
        "BitmapInfoHeader",
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
            // Whatever a later version of the header added.
            ("extra", T::bytes(E::Remaining)),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    fn bmp_bytes(dib_size: u32) -> Vec<u8> {
        let mut dib = dib_size.to_le_bytes().to_vec();
        if dib_size == 12 {
            dib.extend_from_slice(&8u16.to_le_bytes());
            dib.extend_from_slice(&8u16.to_le_bytes());
            dib.extend_from_slice(&1u16.to_le_bytes());
            dib.extend_from_slice(&24u16.to_le_bytes());
        } else {
            dib.extend_from_slice(&8i32.to_le_bytes());
            dib.extend_from_slice(&(-8i32).to_le_bytes());
            dib.extend_from_slice(&1u16.to_le_bytes());
            dib.extend_from_slice(&24u16.to_le_bytes());
            dib.extend_from_slice(&0u32.to_le_bytes());
            dib.extend_from_slice(&192u32.to_le_bytes());
            dib.extend_from_slice(&[0; 16]);
            dib.resize(dib_size as usize, 0);
        }
        let mut v = b"BM".to_vec();
        v.extend_from_slice(&(14 + dib.len() as u32 + 192).to_le_bytes());
        v.extend_from_slice(&[0; 4]);
        v.extend_from_slice(&(14 + dib.len() as u32).to_le_bytes());
        v.extend_from_slice(&dib);
        v.extend_from_slice(&[0xff; 192]);
        v
    }

    #[test]
    fn a_windows_bitmap_reads_its_info_header() {
        let d = Document::new(MemSource(bmp_bytes(40)));
        let mut ev = Evaluator::new(bmp());
        assert_eq!(ev.node(&d, &[5]).unwrap().value, Value::UInt(40));
        assert_eq!(ev.node(&d, &[6, 0]).unwrap().value, Value::Int(8));
        // Written top down, which the sign says and nothing else does.
        assert_eq!(ev.node(&d, &[6, 1]).unwrap().value, Value::Int(-8));
        assert_eq!(ev.node(&d, &[7]).unwrap().size_bits, 0);
        assert_eq!(ev.node(&d, &[8]).unwrap().size_bits, 192 * 8);
    }

    #[test]
    fn an_os2_bitmap_reads_the_short_header_the_size_names() {
        let d = Document::new(MemSource(bmp_bytes(12)));
        let mut ev = Evaluator::new(bmp());
        let h = ev.node(&d, &[6]).unwrap();
        assert_eq!(h.type_name, "BitmapCoreHeader");
        assert_eq!(ev.node(&d, &[6, 3]).unwrap().value, Value::UInt(24));
    }

    #[test]
    fn a_later_header_keeps_the_bytes_it_added() {
        let d = Document::new(MemSource(bmp_bytes(124)));
        let mut ev = Evaluator::new(bmp());
        assert_eq!(ev.node(&d, &[6, 10]).unwrap().size_bits, (124 - 40) * 8);
    }
}
