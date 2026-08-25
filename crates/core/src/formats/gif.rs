//! GIF: a header, a screen descriptor, and then a stream of blocks that runs
//! until a trailer byte.
//!
//! Two things here are worth the trouble. The global colour table exists only
//! when a bit in the packed byte says so, and its size is two to the power of
//! the low three bits of that same byte plus one, which is a shift the
//! expression language cannot do, so it is a switch over the eight values the
//! three bits can take. And every stream of pixel or extension data is written
//! as sub-blocks: a length byte, that many bytes, repeated until a length of
//! zero. That is a list whose end is a field value, which the IR already says.

use crate::template::{Encoding, Endian::*, Expr as E, StrLen, Template, Ty as T, Until};

/// What a frame does to what was on screen before it.
const DISPOSAL: &[(i128, &str)] = &[
    (0, "unspecified"),
    (1, "leave in place"),
    (2, "restore background"),
    (3, "restore previous"),
];

/// The byte that says what kind of block comes next.
const INTRODUCER: &[(i128, &str)] = &[(0x21, "extension"), (0x2c, "image"), (0x3b, "trailer")];

/// The extensions, of which only the graphic control one matters to a decoder.
const EXTENSION: &[(i128, &str)] =
    &[(0x01, "plain text"), (0xf9, "graphic control"), (0xfe, "comment"), (0xff, "application")];

pub fn gif() -> Template {
    Template::new(
        "gif",
        T::structure(
            "GIF",
            vec![
                ("magic", T::magic(b"GIF")),
                ("version", T::text(StrLen::Fixed(E::lit(3)), Encoding::Ascii)),
                ("width", T::u16(Little)),
                ("height", T::u16(Little)),
                // Bit 7: a global colour table follows. Bits 0 to 2: its size.
                ("packed", T::flags("ScreenPacked", T::u8(), &[(7, "global colour table"), (3, "sorted")])),
                ("background_colour", T::u8()),
                ("pixel_aspect_ratio", T::u8()),
                ("global_colour_table", colour_table("packed")),
                ("blocks", T::repeat(block(), Until::FieldBytes { field: "introducer".into(), bytes: vec![0x3b] })),
            ],
        ),
    )
}

/// The colour table a packed byte describes: present only when the high bit is
/// set, and holding two to the power of the low three bits plus one colours.
///
/// There is no power here and no shift, so the choice is made with a switch.
/// What it switches on is the high bit shifted up to sit above the three size
/// bits, which gives 0 to 7 for absent and 8 to 15 for present, and the eight
/// sizes are then written out.
fn colour_table(packed: &str) -> T {
    let colour = T::inline_structure("Rgb", vec![("r", T::u8()), ("g", T::u8()), ("b", T::u8())]).counted_as("colour");
    let present = E::field(packed).div(E::lit(128)).mul(E::lit(8));
    let size = E::field(packed).sub(E::field(packed).div(E::lit(8)).mul(E::lit(8)));
    let cases =
        (0..8).map(|k| (8 + k as i128, T::array(colour.clone(), E::lit(2i128 << k)))).collect::<Vec<_>>();
    T::switch(present.add(size), cases, T::array(colour, E::lit(0)))
}

fn block() -> T {
    T::structure_named(
        "Block",
        "introducer",
        "body",
        vec![
            ("introducer", T::enumeration("Introducer", T::u8(), INTRODUCER)),
            (
                "body",
                T::switch(
                    E::field("introducer"),
                    vec![(0x21, extension()), (0x2c, image())],
                    // The trailer, which has no body at all.
                    T::bytes(E::lit(0)),
                ),
            ),
        ],
    )
}

fn extension() -> T {
    T::structure_named(
        "Extension",
        "label",
        "data",
        vec![
            ("label", T::enumeration("Extension", T::u8(), EXTENSION)),
            ("data", T::switch(E::field("label"), vec![(0xf9, graphic_control())], sub_blocks())),
        ],
    )
}

/// The delay and the transparent colour for the frame that follows.
fn graphic_control() -> T {
    T::structure(
        "GraphicControl",
        vec![
            ("block_size", T::u8()),
            ("reserved", T::UInt { bits: 3, endian: Big }),
            ("disposal", T::enumeration("Disposal", T::UInt { bits: 3, endian: Big }, DISPOSAL)),
            ("user_input", T::UInt { bits: 1, endian: Big }),
            ("has_transparent", T::UInt { bits: 1, endian: Big }),
            // Hundredths of a second, which is why nothing plays at 60fps.
            ("delay", T::u16(Little)),
            ("transparent_colour", T::u8()),
            ("terminator", T::u8()),
        ],
    )
}

fn image() -> T {
    T::structure(
        "Image",
        vec![
            ("left", T::u16(Little)),
            ("top", T::u16(Little)),
            ("width", T::u16(Little)),
            ("height", T::u16(Little)),
            ("packed", T::flags("ImagePacked", T::u8(), &[(7, "local colour table"), (6, "interlaced"), (5, "sorted")])),
            ("local_colour_table", colour_table("packed")),
            ("lzw_minimum_code_size", T::u8()),
            ("data", sub_blocks()),
        ],
    )
}

/// A run of sub-blocks: a length byte, that many bytes, and on until a length
/// of zero, which is the block that ends the run.
fn sub_blocks() -> T {
    let sub = T::structure(
        "SubBlock",
        vec![("length", T::u8()), ("data", T::bytes(E::field("length")))],
    )
    .counted_as("sub-block");
    T::repeat(sub, Until::FieldBytes { field: "length".into(), bytes: vec![0] })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    fn file() -> Vec<u8> {
        let mut v = b"GIF89a".to_vec();
        v.extend_from_slice(&4u16.to_le_bytes());
        v.extend_from_slice(&4u16.to_le_bytes());
        v.push(0x80 | 0x01); // global table, four colours
        v.extend_from_slice(&[0, 0]);
        for i in 0..4u8 {
            v.extend_from_slice(&[i * 64, 0, 0]);
        }
        // A graphic control extension, then one image, then the trailer.
        v.extend_from_slice(&[0x21, 0xf9, 4, 0x05, 10, 0, 0, 0]);
        v.push(0x2c);
        v.extend_from_slice(&0u16.to_le_bytes());
        v.extend_from_slice(&0u16.to_le_bytes());
        v.extend_from_slice(&4u16.to_le_bytes());
        v.extend_from_slice(&4u16.to_le_bytes());
        v.push(0); // no local table
        v.push(2); // minimum code size
        v.extend_from_slice(&[3, 0x44, 0x01, 0x00, 0]);
        v.push(0x3b);
        v
    }

    #[test]
    fn the_global_colour_table_is_as_big_as_the_packed_byte_says() {
        let d = Document::new(MemSource(file()));
        let mut ev = Evaluator::new(gif());
        assert_eq!(ev.node(&d, &[7]).unwrap().child_count, 4);
        assert_eq!(ev.node(&d, &[7, 3, 0]).unwrap().value, Value::UInt(192));
    }

    #[test]
    fn the_blocks_run_to_the_trailer_and_the_image_data_is_sub_blocks() {
        let d = Document::new(MemSource(file()));
        let mut ev = Evaluator::new(gif());
        assert_eq!(ev.node(&d, &[8]).unwrap().child_count, 3);
        // The delay, in hundredths of a second.
        assert_eq!(ev.node(&d, &[8, 0, 1, 1, 5]).unwrap().value, Value::UInt(10));
        // No local table, so the image has none and the pixels follow.
        assert_eq!(ev.node(&d, &[8, 1, 1, 5]).unwrap().child_count, 0);
        assert_eq!(ev.node(&d, &[8, 1, 1, 7]).unwrap().child_count, 2);
        assert_eq!(ev.node(&d, &[8, 1, 1, 7, 0, 1]).unwrap().size_bits, 3 * 8);
        assert_eq!(
            ev.node(&d, &[8, 2, 0]).unwrap().value,
            Value::Enum { raw: 0x3b, name: Some("trailer".into()), hex: false }
        );
    }
}
