//! TIFF: the format that asks which way round it is before it says anything
//! else.
//!
//! The first two bytes are `II` or `MM`, for Intel or Motorola, and every
//! number in the file after them is written that way. Nothing else in the tree
//! does this: a PNG is big-endian because PNG is big-endian, and a template
//! can say so once and be done. Here the answer is in the file.
//!
//! Saying it needs nothing new. A template is built by running code, so the
//! layout is written once as a function of the order and the switch on those
//! two bytes picks which of the two to use. The two copies share everything
//! they can, and neither of them has to carry the question around.
//!
//! What follows the header is a directory: a count, that many twelve-byte
//! entries, and the offset of the next directory. The entries are what makes
//! this format what it is. Each is a tag, a type, a count, and four bytes that
//! are the value itself when it fits in four bytes and where to find it when
//! it does not. That last part is not read here: which of the two it is
//! depends on the type and the count multiplied together, and a field that is
//! a number or a pointer depending on arithmetic is past what the IR can say.
//!
//! Nor is the chain of directories followed past the first. Each says where
//! the next one is, as an offset into the file with nothing bounding it, so a
//! file whose directory points back at itself is a ring. Every other type here
//! that refers to itself is bounded by something containing it and must
//! therefore end; this one is not, and hunting the cursor around a ring
//! forever is worse than reading one directory and saying where the next is.

use crate::template::{Endian, Endian::*, Expr as E, Template, Ty as T};

/// The tags worth naming. There are hundreds more, and a number with no name
/// is still shown; these are the ones a file in the wild actually carries.
const TAG: &[(i128, &str)] = &[
    (254, "new subfile type"),
    (255, "subfile type"),
    (256, "image width"),
    (257, "image height"),
    (258, "bits per sample"),
    (259, "compression"),
    (262, "photometric interpretation"),
    (263, "thresholding"),
    (266, "fill order"),
    (269, "document name"),
    (270, "image description"),
    (271, "make"),
    (272, "model"),
    (273, "strip offsets"),
    (274, "orientation"),
    (277, "samples per pixel"),
    (278, "rows per strip"),
    (279, "strip byte counts"),
    (280, "min sample value"),
    (281, "max sample value"),
    (282, "x resolution"),
    (283, "y resolution"),
    (284, "planar configuration"),
    (288, "free offsets"),
    (289, "free byte counts"),
    (290, "grey response unit"),
    (291, "grey response curve"),
    (296, "resolution unit"),
    (297, "page number"),
    (305, "software"),
    (306, "date time"),
    (315, "artist"),
    (316, "host computer"),
    (317, "predictor"),
    (318, "white point"),
    (319, "primary chromaticities"),
    (320, "colour map"),
    (321, "halftone hints"),
    (322, "tile width"),
    (323, "tile length"),
    (324, "tile offsets"),
    (325, "tile byte counts"),
    (338, "extra samples"),
    (339, "sample format"),
    (529, "ycbcr coefficients"),
    (530, "ycbcr subsampling"),
    (531, "ycbcr positioning"),
    (532, "reference black white"),
    (700, "xmp"),
    (33432, "copyright"),
    (33723, "iptc"),
    (34665, "exif ifd"),
    (34853, "gps ifd"),
];

/// What one value of an entry is. The count says how many of them there are,
/// so the room an entry describes is this times that.
const FIELD_TYPE: &[(i128, &str)] = &[
    (1, "byte"),
    (2, "ascii"),
    (3, "short"),
    (4, "long"),
    (5, "rational"),
    (6, "signed byte"),
    (7, "undefined"),
    (8, "signed short"),
    (9, "signed long"),
    (10, "signed rational"),
    (11, "float"),
    (12, "double"),
    (13, "ifd"),
    (16, "long8"),
    (17, "signed long8"),
    (18, "ifd8"),
];

pub fn tiff() -> Template {
    Template::new("tiff", tiff_file())
}

/// The whole of a TIFF, in a window of its own so that every offset inside it
/// counts from where it begins. On its own that is the start of the file and
/// the window changes nothing; written into an EXIF block partway through a
/// JPEG it is the only thing that makes the offsets mean anything.
pub fn tiff_file() -> T {
    T::sized(
        E::Remaining,
        T::structure(
            "TIFF",
            vec![
                // Both letters of each are the same byte, so which way round
                // this is read cannot change what it says, which is the only
                // reason it can be read at all before the answer is known.
                (
                    "byte_order",
                    T::enumeration("ByteOrder", T::u16(Big), &[(0x4949, "intel, little-endian"), (0x4d4d, "motorola, big-endian")]),
                ),
                (
                    "file",
                    T::switch(
                        E::field("byte_order"),
                        vec![(0x4949, file(Little)), (0x4d4d, file(Big))],
                        T::bytes(E::Remaining),
                    ),
                ),
            ],
        ),
    )
}

/// Everything after the two letters, written the way they said.
fn file(e: Endian) -> T {
    T::structure(
        "Header",
        vec![
            // 42, which the specification says was chosen for its deep
            // philosophical significance. BigTIFF says 43 and lays out its
            // directories differently; this reads the header and stops.
            ("version", T::enumeration("Version", T::u16(e), &[(42, "tiff"), (43, "bigtiff")])),
            ("ifd_offset", T::u32(e)),
            // The directory, wherever the header says it is. It costs no bytes
            // here, so the image below still starts where it starts.
            ("ifd", T::at_in_window(E::field("ifd_offset"), ifd(e))),
            // The strips, the tiles, any directory after the first, and the
            // values too big to sit inside an entry. Every one of them is
            // pointed at by an entry above rather than laid out in order.
            ("body", T::bytes(E::Remaining)),
        ],
    )
}

/// One directory: how many entries, the entries, and where the next one is.
fn ifd(e: Endian) -> T {
    T::structure(
        "Ifd",
        vec![
            ("count", T::u16(e)),
            ("entries", T::array(entry(e), E::field("count"))),
            // Zero when this is the last one, which is almost always.
            ("next_ifd_offset", T::u32(e)),
        ],
    )
}

/// One entry: what it is, what kind of thing it holds, how many, and then four
/// bytes that are either the thing itself or where to find it.
fn entry(e: Endian) -> T {
    T::structure_named(
        "Entry",
        "tag",
        "",
        vec![
            ("tag", T::enumeration("Tag", T::u16(e), TAG)),
            ("type", T::enumeration("FieldType", T::u16(e), FIELD_TYPE)),
            ("count", T::u32(e)),
            // Four bytes wide whatever it holds. A value needing more room
            // than that is somewhere else and this is where.
            ("value_or_offset", T::u32(e)),
        ],
    )
    .counted_as("entry")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    /// The same picture written both ways round: a two-entry directory at the
    /// end, and eight bytes of image before it.
    fn tiff_bytes(little: bool) -> Vec<u8> {
        let u16b = |v: u16| if little { v.to_le_bytes().to_vec() } else { v.to_be_bytes().to_vec() };
        let u32b = |v: u32| if little { v.to_le_bytes().to_vec() } else { v.to_be_bytes().to_vec() };

        let mut v = if little { b"II".to_vec() } else { b"MM".to_vec() };
        v.extend_from_slice(&u16b(42));
        v.extend_from_slice(&u32b(16)); // the directory sits after the image
        v.extend_from_slice(&[0xaa; 8]); // the image, at byte 8
        assert_eq!(v.len(), 16);

        v.extend_from_slice(&u16b(2));
        for (tag, kind, count, value) in [(256u16, 3u16, 1u32, 640u32), (273, 4, 1, 8)] {
            v.extend_from_slice(&u16b(tag));
            v.extend_from_slice(&u16b(kind));
            v.extend_from_slice(&u32b(count));
            v.extend_from_slice(&u32b(value));
        }
        v.extend_from_slice(&u32b(0)); // no directory after this one
        v
    }

    #[test]
    fn the_same_file_both_ways_round_reads_the_same() {
        for little in [true, false] {
            let d = Document::new(MemSource(tiff_bytes(little)));
            let mut ev = Evaluator::new(tiff());
            let order = ev.node(&d, &[0]).unwrap();
            let want = if little { "intel, little-endian" } else { "motorola, big-endian" };
            assert_eq!(order.value, Value::Enum { raw: if little { 0x4949 } else { 0x4d4d }, name: Some(want.into()), hex: false });
            assert_eq!(ev.node(&d, &[1, 0]).unwrap().value, Value::Enum { raw: 42, name: Some("tiff".into()), hex: false });
            assert_eq!(ev.node(&d, &[1, 1]).unwrap().value, Value::UInt(16));

            // The directory, at the far end of the file and costing nothing
            // where it is named.
            assert_eq!(ev.node(&d, &[1, 2]).unwrap().size_bits, 0);
            let ifd = ev.node(&d, &[1, 2, 0]).unwrap();
            assert_eq!(ifd.offset_bits, 16 * 8);
            assert_eq!(ifd.type_name, "Ifd");
            assert_eq!(ev.node(&d, &[1, 2, 0, 0]).unwrap().value, Value::UInt(2));

            let entries = ev.node(&d, &[1, 2, 0, 1]).unwrap();
            assert_eq!(entries.child_count, 2);
            assert_eq!(
                ev.node(&d, &[1, 2, 0, 1, 0, 0]).unwrap().value,
                Value::Enum { raw: 256, name: Some("image width".into()), hex: false }
            );
            assert_eq!(
                ev.node(&d, &[1, 2, 0, 1, 0, 1]).unwrap().value,
                Value::Enum { raw: 3, name: Some("short".into()), hex: false }
            );
            assert_eq!(ev.node(&d, &[1, 2, 0, 1, 0, 3]).unwrap().value, Value::UInt(640), "{little}");
            assert_eq!(
                ev.node(&d, &[1, 2, 0, 1, 1, 0]).unwrap().value,
                Value::Enum { raw: 273, name: Some("strip offsets".into()), hex: false }
            );
            assert_eq!(ev.node(&d, &[1, 2, 0, 1, 1, 3]).unwrap().value, Value::UInt(8));
            assert_eq!(ev.node(&d, &[1, 2, 0, 2]).unwrap().value, Value::UInt(0));
        }
    }

    #[test]
    fn the_image_is_still_covered_and_the_cursor_finds_both_of_them() {
        let d = Document::new(MemSource(tiff_bytes(true)));
        let mut ev = Evaluator::new(tiff());
        // Everything after the header, which the entries point into.
        let body = ev.node(&d, &[1, 3]).unwrap();
        assert_eq!(body.offset_bits, 8 * 8);
        // A byte of the image, and a byte of the directory that describes it.
        assert_eq!(ev.locate(&d, 10 * 8).unwrap(), vec![1, 3]);
        // The count sits at 16, so the first entry starts at 18: its tag is
        // the two bytes there, and its type the two after them.
        assert_eq!(ev.locate(&d, 18 * 8).unwrap(), vec![1, 2, 0, 1, 0, 0]);
        assert_eq!(ev.locate(&d, 20 * 8).unwrap(), vec![1, 2, 0, 1, 0, 1]);
    }

    #[test]
    fn a_byte_order_the_format_does_not_have_is_left_alone() {
        let mut v = b"XX".to_vec();
        v.extend_from_slice(&[0; 20]);
        let d = Document::new(MemSource(v));
        let mut ev = Evaluator::new(tiff());
        assert_eq!(ev.node(&d, &[0]).unwrap().value, Value::Enum { raw: 0x5858, name: None, hex: false });
        // Nothing is read as a number that could be either way round.
        assert_eq!(ev.node(&d, &[1]).unwrap().size_bits, 20 * 8);
    }
}
