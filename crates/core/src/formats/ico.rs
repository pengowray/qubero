//! Windows icon and cursor files: a directory followed by images at offsets.
//!
//! An image is either a PNG file or the DIB part of a BMP. The directory says
//! which without a flag, so the first four bytes at the pointed-to image are
//! used to distinguish them. DIB pixels remain whole: unlike an ordinary BMP,
//! an icon has no bitmap file header and its XOR and AND masks share the body.

use crate::template::{Endian::*, Expr as E, Template, Ty as T};

const KIND: &[(i128, &str)] = &[(1, "icon"), (2, "cursor")];

pub fn ico() -> Template {
    Template::new(
        "ico",
        T::structure(
            "IconFile",
            vec![
                ("reserved", T::u16(Little)),
                ("kind", T::enumeration("IconKind", T::u16(Little), KIND)),
                ("image_count", T::u16(Little)),
                ("entries", T::array(entry(), E::field("image_count"))),
            ],
        ),
    )
}

fn entry() -> T {
    T::structure(
        "IconDirectoryEntry",
        vec![
            ("width", T::u8()),
            ("height", T::u8()),
            ("colour_count", T::u8()),
            ("reserved", T::u8()),
            // Cursors use these same four bytes for their hotspot.
            ("planes_or_hotspot_x", T::u16(Little)),
            ("bits_per_pixel_or_hotspot_y", T::u16(Little)),
            ("image_size", T::u32(Little)),
            ("image_offset", T::u32(Little)),
            (
                "image",
                T::at(
                    E::field("image_offset"),
                    T::sized(
                        E::field("image_size"),
                        T::switch(
                            E::peek(32, Big),
                            vec![(0x8950_4e47, super::png().root)],
                            dib_image(),
                        ),
                    ),
                ),
            ),
        ],
    )
    .counted_as("image")
}

fn dib_image() -> T {
    T::structure(
        "IconDibImage",
        vec![
            ("header_size", T::u32(Little)),
            ("width", T::i32(Little)),
            // This includes both the colour image and the 1-bit AND mask.
            ("combined_height", T::i32(Little)),
            ("planes", T::u16(Little)),
            ("bits_per_pixel", T::u16(Little)),
            ("dib_header_and_pixels", T::bytes(E::Remaining)),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        document::Document,
        eval::{Evaluator, Value},
        source::MemSource,
    };

    #[test]
    fn directory_places_a_dib_image() {
        let mut v = vec![0, 0, 1, 0, 1, 0, 16, 16, 0, 0, 1, 0, 32, 0];
        v.extend_from_slice(&40u32.to_le_bytes());
        v.extend_from_slice(&22u32.to_le_bytes());
        v.extend_from_slice(&40u32.to_le_bytes());
        v.extend_from_slice(&16i32.to_le_bytes());
        v.extend_from_slice(&32i32.to_le_bytes());
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&32u16.to_le_bytes());
        v.extend_from_slice(&[0; 24]);
        let d = Document::new(MemSource(v));
        let mut ev = Evaluator::new(ico());
        assert_eq!(ev.node(&d, &[2]).unwrap().value, Value::UInt(1));
        assert_eq!(ev.node(&d, &[3, 0, 8, 0, 1]).unwrap().value, Value::Int(16));
        assert_eq!(ev.node(&d, &[3, 0, 8, 0]).unwrap().offset_bits, 22 * 8);
    }
}
