//! Targa: an eighteen-byte header, an optional id and colour map, and then
//! the pixels. Nothing marks the front of the file, which is why this is a
//! template to pick rather than one to guess.
//!
//! Later files end with a footer naming the format outright, eighteen bytes
//! at the very end reading `TRUEVISION-XFILE`. That is the only signature the
//! format has, and it is at the wrong end to sniff a stream with. It is not at
//! the wrong end to read: a peek can be told to count back from the end of
//! what holds it, so the template looks at the last eight bytes of the file
//! before it places anything, and the image then runs to the footer or to the
//! end of the file depending on what it saw.
//!
//! Eight bytes is what a peek holds at once, so it is eight of the eighteen
//! that are checked. A file whose pixels happen to end with those same eight
//! is read as having a footer it does not have, and says so: the eighteen-byte
//! signature in it then fails to match, which is a row the reader can see.
//!
//! What the footer holds is two offsets, to an extension area of authorship
//! and timestamps and to a chain of developer-defined records. Both sit
//! between the pixels and the footer, so they are inside what is read here as
//! the image. Reaching them needs an offset read from the middle of a peeked
//! run rather than from a field, and the peek reads big-endian while those
//! offsets are little-endian, so that is where this stops.

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
                // The last eight bytes of the signature, if there is one. It
                // costs no bytes, and everything below reads as one thing or
                // the other depending on it. A file with less than a footer
                // left in it is not asked: there is nowhere for one to be.
                (
                    "footer_signature",
                    T::switch(
                        E::Remaining.div(E::lit(26)),
                        vec![(0, T::computed(E::lit(0)))],
                        T::computed(E::peek_at(E::lit(-64), 64)),
                    ),
                ),
                // The pixels, and anything the footer points at, which sits
                // between them and the footer itself.
                (
                    "image",
                    T::switch(
                        E::field("footer_signature"),
                        vec![(SIGNATURE_TAIL, T::bytes(E::Remaining.sub(E::lit(26))))],
                        T::bytes(E::Remaining),
                    ),
                ),
                ("footer", T::switch(E::field("footer_signature"), vec![(SIGNATURE_TAIL, footer())], nothing())),
            ],
        ),
    )
}

/// The last eight bytes of `TRUEVISION-XFILE.` and its NUL, as the big-endian
/// number a peek reads them as. Eight is what a peek can hold at once, and
/// eight of those eighteen bytes is already more than any file lands on by
/// accident.
const SIGNATURE_TAIL: i128 = 0x2d58_4649_4c45_2e00;

/// What a TGA 2.0 file ends with. A file without one is a TGA 1.0, and there
/// is nothing in it that says so.
fn footer() -> T {
    T::structure(
        "Footer",
        vec![
            ("extension_offset", T::u32(Little)),
            ("developer_offset", T::u32(Little)),
            ("signature", T::magic(b"TRUEVISION-XFILE.\0")),
        ],
    )
}

fn nothing() -> T {
    T::bytes(E::lit(0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    /// The eighteen fixed bytes, an id, a four-entry colour map and pixels.
    fn tga_bytes(footer: bool) -> Vec<u8> {
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
        if footer {
            v.extend_from_slice(&0u32.to_le_bytes()); // no extension area
            v.extend_from_slice(&0u32.to_le_bytes()); // no developer records
            v.extend_from_slice(b"TRUEVISION-XFILE.\0");
        }
        v
    }

    #[test]
    fn an_indexed_image_places_its_id_map_and_pixels() {
        let d = Document::new(MemSource(tga_bytes(false)));
        let mut ev = Evaluator::new(tga());
        assert_eq!(ev.node(&d, &[12]).unwrap().value, Value::Str("hello".into()));
        assert_eq!(ev.node(&d, &[13]).unwrap().size_bits, 12 * 8);
        assert_eq!(ev.node(&d, &[15]).unwrap().size_bits, 4 * 8);
        assert_eq!(
            ev.node(&d, &[2]).unwrap().value,
            Value::Enum { raw: 1, name: Some("indexed".into()), hex: false }
        );
        // No signature at the end, so there is no footer and the pixels run
        // all the way to it.
        assert_eq!(ev.node(&d, &[16]).unwrap().size_bits, 0);
    }

    #[test]
    fn a_tga_2_file_is_told_by_the_signature_at_the_far_end_of_it() {
        let d = Document::new(MemSource(tga_bytes(true)));
        let mut ev = Evaluator::new(tga());
        // The pixels stop where the footer starts, not at the end of the file.
        assert_eq!(ev.node(&d, &[15]).unwrap().size_bits, 4 * 8);
        let footer = ev.node(&d, &[16]).unwrap();
        assert_eq!(footer.type_name, "Footer");
        assert_eq!(footer.size_bits, 26 * 8);
        assert_eq!(ev.node(&d, &[16, 0]).unwrap().value, Value::UInt(0));
        let signature = ev.node(&d, &[16, 2]).unwrap();
        assert_eq!(signature.value, Value::Magic { ok: true, bytes: b"TRUEVISION-XFILE.\0".to_vec() });
        // Looking for it costs no bytes of its own.
        assert_eq!(ev.node(&d, &[14]).unwrap().size_bits, 0);
    }
}
