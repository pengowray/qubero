//! PCX: a 128-byte header, then run-length encoded scanlines.
//!
//! The header is fixed, which makes this the smallest real format in the tree.
//! What it does not say is where the image ends: the encoding stops when the
//! decoder has enough scanlines, and a 256-colour file then writes one more
//! byte, 0x0C, followed by 768 bytes of palette. Nothing in the header points
//! at that palette, so it is read from the tail rather than pointed at, and
//! only for the files whose depth calls for one.

use crate::template::{Endian::*, Expr as E, Template, Ty as T};

/// PCX versions, which is really a history of what Paintbrush could store.
const VERSION: &[(i128, &str)] = &[
    (0, "2.5"),
    (2, "2.8 with palette"),
    (3, "2.8 without palette"),
    (4, "for windows"),
    (5, "3.0"),
];

pub fn pcx() -> Template {
    Template::new(
        "pcx",
        T::structure(
            "PCX",
            vec![
                ("magic", T::magic(b"\x0a")),
                ("version", T::enumeration("Version", T::u8(), VERSION)),
                ("encoding", T::enumeration("Encoding", T::u8(), &[(0, "none"), (1, "rle")])),
                ("bits_per_pixel", T::u8()),
                ("xmin", T::u16(Little)),
                ("ymin", T::u16(Little)),
                ("xmax", T::u16(Little)),
                ("ymax", T::u16(Little)),
                ("hdpi", T::u16(Little)),
                ("vdpi", T::u16(Little)),
                ("palette", T::array(rgb(), E::lit(16))),
                ("reserved", T::u8()),
                ("planes", T::u8()),
                ("bytes_per_line", T::u16(Little)),
                ("palette_info", T::enumeration("PaletteInfo", T::u16(Little), &[(1, "colour"), (2, "greyscale")])),
                ("h_screen_size", T::u16(Little)),
                ("v_screen_size", T::u16(Little)),
                ("filler", T::bytes(E::lit(54))),
                // Everything after the header. Where the pixels stop and a
                // 256-colour palette starts is not written down anywhere, so
                // the two are not split here.
                ("image", T::bytes(E::Remaining)),
            ],
        ),
    )
}

fn rgb() -> T {
    T::inline_structure("Rgb", vec![("r", T::u8()), ("g", T::u8()), ("b", T::u8())])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    fn header() -> Vec<u8> {
        let mut v = vec![0x0a, 5, 1, 8];
        v.extend_from_slice(&0u16.to_le_bytes()); // xmin
        v.extend_from_slice(&0u16.to_le_bytes()); // ymin
        v.extend_from_slice(&319u16.to_le_bytes()); // xmax
        v.extend_from_slice(&199u16.to_le_bytes()); // ymax
        v.extend_from_slice(&72u16.to_le_bytes());
        v.extend_from_slice(&72u16.to_le_bytes());
        v.extend_from_slice(&[0; 48]); // the sixteen-colour palette
        v.push(0); // reserved
        v.push(1); // planes
        v.extend_from_slice(&320u16.to_le_bytes());
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&[0; 4]);
        v.extend_from_slice(&[0; 54]);
        assert_eq!(v.len(), 128);
        v
    }

    #[test]
    fn the_header_reads_and_the_image_is_what_is_left() {
        let mut b = header();
        b.extend_from_slice(&[0xc2, 0xff, 0x01]);
        let d = Document::new(MemSource(b));
        let mut ev = Evaluator::new(pcx());
        assert_eq!(ev.node(&d, &[6]).unwrap().value, Value::UInt(319));
        assert_eq!(ev.node(&d, &[1]).unwrap().value, Value::Enum { raw: 5, name: Some("3.0".into()), hex: false });
        let image = ev.node(&d, &[18]).unwrap();
        assert_eq!(image.offset_bits, 128 * 8);
        assert_eq!(image.size_bits, 3 * 8);
    }
}
