//! Targa: an eighteen-byte header, an optional id and colour map, and then
//! the pixels. Nothing marks the front of the file, which is why this is a
//! template to pick rather than one to guess.
//!
//! Later files end with a footer naming the format outright, eighteen bytes
//! at the very end reading `TRUEVISION-XFILE`. That is the only signature the
//! format has, and it is at the wrong end to sniff a stream with.

use crate::template::{Encoding, Endian::*, Expr as E, StrLen, Template, Ty as T};

/// What the pixels are and how they are stored. The high bit of the number is
/// run-length encoding, which is why the list runs 1, 2, 3 and then 9, 10, 11.
const IMAGE_TYPE: &[(i128, &str)] = &[
    (0, "none"),
    (1, "indexed"),
    (2, "rgb"),
    (3, "greyscale"),
    (9, "indexed rle"),
    (10, "rgb rle"),
    (11, "greyscale rle"),
];

pub fn tga() -> Template {
    Template::new(
        "tga",
        T::structure(
            "TGA",
            vec![
                ("id_length", T::u8()),
                ("colour_map_type", T::enumeration("ColourMapType", T::u8(), &[(0, "none"), (1, "present")])),
                ("image_type", T::enumeration("ImageType", T::u8(), IMAGE_TYPE)),
                ("colour_map_first", T::u16(Little)),
                ("colour_map_length", T::u16(Little)),
                ("colour_map_depth", T::u8()),
                ("x_origin", T::u16(Little)),
                ("y_origin", T::u16(Little)),
                ("width", T::u16(Little)),
                ("height", T::u16(Little)),
                ("bits_per_pixel", T::u8()),
                // Bits 0 to 3 are how many of those bits are alpha; bits 4 and
                // 5 say which corner the first pixel is.
                ("descriptor", T::u8()),
                ("id", T::text(StrLen::Fixed(E::field("id_length")), Encoding::Ascii)),
                // Entries are whatever width the map declares, rounded up to
                // whole bytes, and there are none at all when there is no map.
                (
                    "colour_map",
                    T::bytes(E::field("colour_map_length").mul(E::field("colour_map_depth").add(E::lit(7)).div(E::lit(8)))),
                ),
                ("image", T::bytes(E::Remaining)),
            ],
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    #[test]
    fn an_indexed_image_places_its_id_map_and_pixels() {
        let mut v = vec![5u8, 1, 1];
        v.extend_from_slice(&0u16.to_le_bytes()); // first entry
        v.extend_from_slice(&4u16.to_le_bytes()); // four entries
        v.push(24); // three bytes each
        v.extend_from_slice(&0u16.to_le_bytes());
        v.extend_from_slice(&0u16.to_le_bytes());
        v.extend_from_slice(&2u16.to_le_bytes()); // width
        v.extend_from_slice(&2u16.to_le_bytes()); // height
        v.push(8);
        v.push(0);
        v.extend_from_slice(b"hello");
        v.extend_from_slice(&[0; 12]);
        v.extend_from_slice(&[1, 2, 3, 0]);

        let d = Document::new(MemSource(v));
        let mut ev = Evaluator::new(tga());
        assert_eq!(ev.node(&d, &[12]).unwrap().value, Value::Str("hello".into()));
        assert_eq!(ev.node(&d, &[13]).unwrap().size_bits, 12 * 8);
        assert_eq!(ev.node(&d, &[14]).unwrap().size_bits, 4 * 8);
        assert_eq!(
            ev.node(&d, &[2]).unwrap().value,
            Value::Enum { raw: 1, name: Some("indexed".into()), hex: false }
        );
    }
}
