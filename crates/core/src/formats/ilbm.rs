//! ILBM: the Amiga picture format, bit planes and all.
//!
//! `BMHD` says how many planes there are rather than how many bits a pixel is,
//! because that is how the hardware was wired: a five-plane picture is five
//! separate bitmaps read in parallel, and thirty-two colours is what falls out.
//! `CAMG` carries the viewport flags, which is where HAM and half-brite live,
//! and those are the modes that let a machine with five planes show 4096
//! colours.
//!
//! `BODY` is either raw planes or ByteRun1, the run-length encoding Electronic
//! Arts specified alongside the format. Neither is unpacked here.

use crate::template::{Endian::*, Expr as E, Template, Ty as T};

use super::iff::{cc, chunk_text, iff};

/// What the extra plane in a masked picture is for.
const MASKING: &[(i128, &str)] = &[(0, "none"), (1, "mask plane"), (2, "transparent colour"), (3, "lasso")];

/// The viewport bits `CAMG` carries. These are the display flags of the
/// hardware itself, so the numbers are the ones the chipset used.
const CAMG: &[(u32, &str)] = &[
    (2, "lace"),
    (3, "extra half-brite"),
    (7, "ham"),
    (10, "hires"),
    (11, "super hires"),
    (15, "hires sprites"),
];

pub fn ilbm() -> Template {
    iff("ilbm", body())
}

fn body() -> T {
    T::switch(
        E::field("id"),
        vec![
            (cc("BMHD"), bmhd()),
            (cc("CMAP"), cmap()),
            (cc("CAMG"), T::structure("Viewport", vec![("modes", T::flags("Camg", T::u32(Big), CAMG))])),
            (
                cc("GRAB"),
                T::structure(
                    "Hotspot",
                    vec![("x", T::Int { bits: 16, endian: Big }), ("y", T::Int { bits: 16, endian: Big })],
                ),
            ),
            (cc("ANNO"), chunk_text()),
            (cc("AUTH"), chunk_text()),
            (cc("NAME"), chunk_text()),
            (cc("(c) "), chunk_text()),
        ],
        T::bytes(E::Remaining),
    )
}

fn bmhd() -> T {
    T::structure(
        "BitmapHeader",
        vec![
            ("width", T::u16(Big)),
            ("height", T::u16(Big)),
            ("x", T::Int { bits: 16, endian: Big }),
            ("y", T::Int { bits: 16, endian: Big }),
            // Planes, not bits per pixel: the colour count is two to this power.
            ("planes", T::u8()),
            ("masking", T::enumeration("Masking", T::u8(), MASKING)),
            ("compression", T::enumeration("Compression", T::u8(), &[(0, "none"), (1, "byterun1")])),
            ("pad", T::u8()),
            ("transparent_colour", T::u16(Big)),
            // Pixels were not square: 10 by 11 on a low-resolution screen.
            ("x_aspect", T::u8()),
            ("y_aspect", T::u8()),
            ("page_width", T::Int { bits: 16, endian: Big }),
            ("page_height", T::Int { bits: 16, endian: Big }),
        ],
    )
}

/// The palette: three bytes a colour, however many the chunk holds. Files from
/// the original hardware write the four-bit values in the high nibbles, so a
/// colour reads as 0x00 to 0xf0 rather than the full range.
fn cmap() -> T {
    let colour = T::inline_structure("Rgb", vec![("r", T::u8()), ("g", T::u8()), ("b", T::u8())]).counted_as("colour");
    T::array(colour, E::Remaining.div(E::lit(3)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    fn chunk(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut v = id.to_vec();
        v.extend_from_slice(&(body.len() as u32).to_be_bytes());
        v.extend_from_slice(body);
        if body.len() % 2 == 1 {
            v.push(0);
        }
        v
    }

    #[test]
    fn a_five_plane_picture_reads_its_header_palette_and_modes() {
        let mut bmhd = 320u16.to_be_bytes().to_vec();
        bmhd.extend_from_slice(&200u16.to_be_bytes());
        bmhd.extend_from_slice(&[0, 0, 0, 0]); // x, y
        bmhd.extend_from_slice(&[5, 0, 1, 0]); // planes, masking, byterun1, pad
        bmhd.extend_from_slice(&0u16.to_be_bytes());
        bmhd.extend_from_slice(&[10, 11]);
        bmhd.extend_from_slice(&320i16.to_be_bytes());
        bmhd.extend_from_slice(&200i16.to_be_bytes());

        let mut cmap = Vec::new();
        for i in 0..32u8 {
            cmap.extend_from_slice(&[i << 4, 0, 0]);
        }

        let mut chunks = chunk(b"BMHD", &bmhd);
        chunks.extend_from_slice(&chunk(b"CMAP", &cmap));
        chunks.extend_from_slice(&chunk(b"CAMG", &(1u32 << 7).to_be_bytes()));
        chunks.extend_from_slice(&chunk(b"BODY", &[0xff; 40]));

        let mut v = b"FORM".to_vec();
        v.extend_from_slice(&((4 + chunks.len()) as u32).to_be_bytes());
        v.extend_from_slice(b"ILBM");
        v.extend_from_slice(&chunks);

        let d = Document::new(MemSource(v));
        let mut ev = Evaluator::new(ilbm());
        assert_eq!(ev.node(&d, &[3]).unwrap().child_count, 4);
        assert_eq!(ev.node(&d, &[3, 0, 2, 4]).unwrap().value, Value::UInt(5));
        assert_eq!(
            ev.node(&d, &[3, 0, 2, 6]).unwrap().value,
            Value::Enum { raw: 1, name: Some("byterun1".into()), hex: false }
        );
        // The palette has as many colours as the chunk has room for.
        assert_eq!(ev.node(&d, &[3, 1, 2]).unwrap().child_count, 32);
        assert_eq!(ev.node(&d, &[3, 1, 2, 31, 0]).unwrap().value, Value::UInt(0xf0));
        // HAM, which is bit 7 of the viewport flags and nothing else.
        let camg = ev.node(&d, &[3, 2, 2, 0]).unwrap();
        assert_eq!(camg.value, Value::Flags { raw: 128, set: vec!["ham".into()], unnamed: 0 });
        assert_eq!(ev.node(&d, &[3, 3, 2]).unwrap().size_bits, 40 * 8);
    }
}
