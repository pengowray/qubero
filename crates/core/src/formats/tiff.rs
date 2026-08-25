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
//! are the value itself when it fits in four and where to find it when it
//! does not. Nothing in the entry says which of the two those bytes are: it is
//! the size of the type times the count, worked out rather than read. So that
//! is what the template does, in a field of no bits called `room`, and the
//! switch below it is on whether the answer is over four.
//!
//! A value that does not fit is read where the entry says it is, which is an
//! offset from the start of the TIFF. That is the same as the start of the
//! file right up until it is not: an EXIF block is a whole TIFF written into a
//! JPEG segment, and every offset in one counts from its own `II` or `MM`
//! partway through. The whole format sits in a window of its own and its
//! offsets are anchored to that window, so the one layout is both a file and a
//! copy of a file inside something else. `tiff_part` is what a JPEG borrows:
//! the type, and the directory types it refers to by name, which have to
//! travel with it because a directory holds directories.
//!
//! Three tags do not hold a value at all. `exif ifd` is where the camera wrote
//! everything it knew, `gps ifd` is where the photograph was taken, and
//! `sub ifd` is another image entirely. All three hold the place a directory
//! begins, and all three are followed. What the first two point at is read
//! against another set of tag names: the numbers in a camera's directory mean
//! nothing like the numbers in the file's own, and reading a latitude as a
//! subfile type would be worse than not reading it at all.
//!
//! The chain of directories is not followed past the first. Each says where
//! the next one is, as an offset into the file with nothing bounding it, so a
//! file whose directory points back at itself is a ring. Every other type here
//! that refers to itself is bounded by something containing it and must
//! therefore end; this one is not. The tags above can ring in the same way,
//! and the cycle guard is what makes following them safe: a pointer landing on
//! a directory of the same kind already open is refused. `sub ifd` holding a
//! list rather than one directory is left as the numbers it is; a raw file
//! with several images in it says so that way.

use crate::template::{Encoding, Endian, Endian::*, Expr as E, Part, StrLen, Template, Ty as T};

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

/// The tags of the directory `exif ifd` points at: what the camera knew when
/// it took the photograph. A different set of numbers from the one above, and
/// the same numbers would mean something else there.
const EXIF_TAG: &[(i128, &str)] = &[
    (33434, "exposure time"),
    (33437, "f number"),
    (34850, "exposure program"),
    (34852, "spectral sensitivity"),
    (34855, "iso speed"),
    (34864, "sensitivity type"),
    (34866, "recommended exposure index"),
    (36864, "exif version"),
    (36867, "date taken"),
    (36868, "date digitised"),
    (36880, "offset from utc"),
    (36881, "offset from utc, taken"),
    (36882, "offset from utc, digitised"),
    (37377, "shutter speed"),
    (37378, "aperture"),
    (37379, "brightness"),
    (37380, "exposure bias"),
    (37381, "widest aperture"),
    (37382, "subject distance"),
    (37383, "metering mode"),
    (37384, "light source"),
    (37385, "flash"),
    (37386, "focal length"),
    (37396, "subject area"),
    // Whatever the manufacturer wanted to keep, in a format of their own that
    // nobody else agreed to. Read as the bytes it is.
    (37500, "maker note"),
    (37510, "comment"),
    (37520, "subsecond"),
    (37521, "subsecond, taken"),
    (37522, "subsecond, digitised"),
    (40960, "flashpix version"),
    (40961, "colour space"),
    (40962, "width"),
    (40963, "height"),
    (40964, "related sound file"),
    (40965, "interoperability ifd"),
    (41486, "focal plane x resolution"),
    (41487, "focal plane y resolution"),
    (41488, "focal plane resolution unit"),
    (41492, "subject position"),
    (41493, "exposure index"),
    (41495, "sensing method"),
    (41728, "file source"),
    (41729, "scene type"),
    (41730, "colour filter pattern"),
    (41985, "custom rendered"),
    (41986, "exposure mode"),
    (41987, "white balance"),
    (41988, "digital zoom"),
    (41989, "focal length in 35mm"),
    (41990, "scene capture type"),
    (41991, "gain control"),
    (41992, "contrast"),
    (41993, "saturation"),
    (41994, "sharpness"),
    (41996, "subject distance range"),
    (42016, "image id"),
    (42032, "camera owner"),
    (42033, "body serial number"),
    (42034, "lens specification"),
    (42035, "lens make"),
    (42036, "lens model"),
    (42037, "lens serial number"),
];

/// The tags of the directory `gps ifd` points at: where the photograph was
/// taken, and when, by the satellites rather than by the camera's clock. The
/// format calls every one of them "gps something"; inside a GPS directory
/// that is not worth repeating on every row.
const GPS_TAG: &[(i128, &str)] = &[
    (0, "version id"),
    (1, "latitude ref"),
    (2, "latitude"),
    (3, "longitude ref"),
    (4, "longitude"),
    (5, "altitude ref"),
    (6, "altitude"),
    (7, "time"),
    (8, "satellites"),
    (9, "status"),
    (10, "measure mode"),
    (11, "precision"),
    (12, "speed ref"),
    (13, "speed"),
    (14, "track ref"),
    (15, "track"),
    (16, "direction ref"),
    (17, "direction"),
    (18, "map datum"),
    (19, "destination latitude ref"),
    (20, "destination latitude"),
    (21, "destination longitude ref"),
    (22, "destination longitude"),
    (23, "destination bearing ref"),
    (24, "destination bearing"),
    (25, "destination distance ref"),
    (26, "destination distance"),
    (27, "processing method"),
    (28, "area information"),
    (29, "date"),
    (30, "differential"),
    (31, "horizontal error"),
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
    let part = tiff_part();
    Template::new("tiff", part.root.clone()).with_part(&part)
}

/// The whole of a TIFF and the names it refers to, for a format that carries
/// one inside it. A directory holds entries that point at directories, so the
/// vocabulary has to travel with the type.
pub fn tiff_part() -> Part {
    let mut part = Part::new(tiff_file());
    for e in [Little, Big] {
        for space in SPACES {
            part = part.with_type(space.named(e), ifd(e, *space));
        }
    }
    part
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
            ("ifd", T::at_in_window(E::field("ifd_offset"), T::Named(ifd_name(e).into()))),
            // The strips, the tiles, any directory after the first, and the
            // values too big to sit inside an entry. Every one of them is
            // pointed at by an entry above rather than laid out in order.
            ("body", T::bytes(E::Remaining)),
        ],
    )
}

/// Which set of names an entry's tag is read against. A directory is a
/// directory whichever one it is, but the numbers in it mean different things:
/// tag 1 is `subfile type` in a TIFF directory and `latitude ref` in a GPS
/// one, and reading a photograph's latitude as a subfile type would be worse
/// than not reading it at all.
#[derive(Clone, Copy, PartialEq)]
enum Space {
    Tiff,
    Exif,
    Gps,
}

impl Space {
    fn tags(self) -> &'static [(i128, &'static str)] {
        match self {
            Space::Tiff => TAG,
            Space::Exif => EXIF_TAG,
            Space::Gps => GPS_TAG,
        }
    }
    /// What a reader sees the directory called.
    fn shown(self) -> &'static str {
        match self {
            Space::Tiff => "Ifd",
            Space::Exif => "ExifIfd",
            Space::Gps => "GpsIfd",
        }
    }
    /// What it is called in the type table. A directory that holds directories
    /// has to name the one it is written like, and the whole layout is a
    /// function of the byte order, so each space is in the table twice.
    fn named(self, e: Endian) -> &'static str {
        match (self, e) {
            (Space::Tiff, Little) => "tiff.Ifd.le",
            (Space::Tiff, Big) => "tiff.Ifd.be",
            (Space::Exif, Little) => "tiff.ExifIfd.le",
            (Space::Exif, Big) => "tiff.ExifIfd.be",
            (Space::Gps, Little) => "tiff.GpsIfd.le",
            (Space::Gps, Big) => "tiff.GpsIfd.be",
        }
    }
}

const SPACES: &[Space] = &[Space::Tiff, Space::Exif, Space::Gps];

fn ifd_name(e: Endian) -> &'static str {
    Space::Tiff.named(e)
}

/// One directory: how many entries, the entries, and where the next one is.
fn ifd(e: Endian, space: Space) -> T {
    T::structure(
        space.shown(),
        vec![
            ("count", T::u16(e)),
            ("entries", T::array(entry(e, space), E::field("count"))),
            // Zero when this is the last one, which is almost always.
            ("next_ifd_offset", T::u32(e)),
        ],
    )
}

/// One entry: what it is, what kind of thing it holds, how many, and then four
/// bytes that are either the thing itself or where to find it.
///
/// Which of the two those four bytes are is not written down anywhere. It is
/// the size of the type times the count: four bytes or fewer and the value is
/// there, more and they are an offset to it. So the size is worked out first,
/// in a field of no bits, and the switch below is on whether it fits.
fn entry(e: Endian, space: Space) -> T {
    T::structure_named(
        "Entry",
        "tag",
        "",
        vec![
            ("tag", T::enumeration("Tag", T::u16(e), space.tags())),
            ("type", T::enumeration("FieldType", T::u16(e), FIELD_TYPE)),
            ("count", T::u32(e)),
            ("room", room()),
            // Dividing by five is how "four or fewer" is asked, since the
            // switch takes one number rather than a comparison.
            ("value", sub_ifd(e, space, T::switch(E::field("room").div(E::lit(5)), vec![(0, here(e))], elsewhere(e)))),
        ],
    )
    .counted_as("entry")
}

/// The tags whose four bytes are not a value but the place another directory
/// begins. A camera writes almost nothing about the photograph in the
/// directory the file points at: the exposure, the lens and the time are in
/// the one that `exif ifd` points at, and where the photograph was taken is in
/// the one after that.
/// Only a TIFF directory holds these. Inside the two they point at, the same
/// numbers mean something else entirely.
const SUB_IFD: &[(i128, Space)] = &[
    // Whole other images: the preview inside a raw file, and the pages of a
    // multi-resolution one.
    (330, Space::Tiff),
    (34665, Space::Exif),
    (34853, Space::Gps),
];

/// An entry that points at a directory reads it, rather than showing the
/// offset and stopping. Only when it points at one: `sub ifd` may hold a list
/// of them, and a list is written elsewhere and is left as the numbers it is.
///
/// This sits at the entry rather than inside the four bytes of the value,
/// because those four bytes are a window of their own and an offset counted
/// from inside it would land four bytes into nowhere.
fn sub_ifd(e: Endian, space: Space, otherwise: T) -> T {
    if space != Space::Tiff {
        return otherwise;
    }
    let cases = SUB_IFD
        .iter()
        .map(|(tag, to)| {
            let one = T::inline_structure(
                "SubIfd",
                vec![
                    ("offset", T::u32(e)),
                    ("directory", T::at_in_window(E::field("offset"), T::Named(to.named(e).into()))),
                ],
            );
            (*tag, T::switch(E::field("count"), vec![(1, one)], otherwise.clone()))
        })
        .collect();
    T::switch(E::field("tag"), cases, otherwise)
}

/// How many bytes the values of this entry need: the size of one of them times
/// how many there are. A type nobody here knows comes to nothing, which sends
/// it down the branch that reads the four bytes and says no more than that.
fn room() -> T {
    let cases = SIZE.iter().map(|(t, w)| (*t, T::computed(E::lit(*w).mul(E::field("count"))))).collect();
    T::switch(E::field("type"), cases, T::computed(E::lit(0)))
}

/// What one value of each type takes up. `FIELD_TYPE` names them; this is what
/// they cost.
const SIZE: &[(i128, i128)] = &[
    (1, 1),
    (2, 1),
    (3, 2),
    (4, 4),
    (5, 8),
    (6, 1),
    (7, 1),
    (8, 2),
    (9, 4),
    (10, 8),
    (11, 4),
    (12, 8),
    (13, 4),
    (16, 8),
    (17, 8),
    (18, 8),
];

/// The values, in the four bytes of the entry itself, written from the first
/// of them and padded out to four whichever way round the file is.
fn here(e: Endian) -> T {
    T::sized(
        E::lit(4),
        T::inline_structure("Here", vec![("values", values(e)), ("padding", T::bytes(E::Remaining))]),
    )
}

/// The values, somewhere else in the file, with the entry saying where. The
/// offset counts from the start of the TIFF, which is why it is anchored to
/// the window rather than to the file: in an EXIF block those are not the
/// same place.
fn elsewhere(e: Endian) -> T {
    T::inline_structure(
        "Elsewhere",
        vec![
            ("offset", T::u32(e)),
            ("values", T::at_in_window(E::field("offset"), values(e))),
        ],
    )
}

/// However many values of whatever type this entry holds, wherever they are.
/// Text and undefined bytes are a run rather than a list of one-byte things,
/// which is what they are; everything else is one value when the count says
/// one and a list when it says more.
fn values(e: Endian) -> T {
    let mut cases = vec![
        (2, T::text(StrLen::Padded { size: E::field("count"), pad: 0 }, Encoding::Ascii)),
        (7, T::bytes(E::field("count"))),
    ];
    for (t, _) in SIZE {
        if let Some(one) = one(e, *t) {
            cases.push((
                *t,
                T::switch(E::field("count"), vec![(1, one.clone())], T::array(one, E::field("count"))),
            ));
        }
    }
    // A type this does not know, in the room it turned out to need, which for
    // an unknown type is the four bytes of the entry and nothing more.
    T::switch(E::field("type"), cases, T::bytes(E::Remaining))
}

/// One value of a type, or nothing for the two that are read as a run.
fn one(e: Endian, t: i128) -> Option<T> {
    Some(match t {
        1 => T::u8(),
        3 => T::u16(e),
        4 | 13 => T::u32(e),
        5 => ratio(e, false),
        6 => T::Int { bits: 8, endian: e },
        8 => T::Int { bits: 16, endian: e },
        9 => T::i32(e),
        10 => ratio(e, true),
        11 => T::F32(e),
        12 => T::F64(e),
        16 | 18 => T::u64(e),
        17 => T::Int { bits: 64, endian: e },
        _ => return None,
    })
}

/// A rational: two numbers rather than one. A resolution of 300 dots an inch
/// is written 300 over 1, and an exposure of a two-hundredth of a second is
/// written 1 over 200, which is the reason the format has the type at all.
fn ratio(e: Endian, signed: bool) -> T {
    let n = || if signed { T::i32(e) } else { T::u32(e) };
    T::inline_structure("Rational", vec![("numerator", n()), ("denominator", n())])
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
            // A value that fits is written from the first of the four bytes
            // and padded out, whichever way round the file is. Writing a
            // short as a long would put it in the wrong two bytes.
            if kind == 3 {
                v.extend_from_slice(&u16b(value as u16));
                v.extend_from_slice(&[0, 0]);
            } else {
                v.extend_from_slice(&u32b(value));
            }
        }
        v.extend_from_slice(&u32b(0)); // no directory after this one
        v
    }

    /// A file whose one entry is `exif ifd`, pointing at a directory with one
    /// entry of its own. `exif_at` is where that second directory begins, so a
    /// test can send it somewhere it should not go.
    fn with_sub_ifd(tag: u16, exif_at: u32) -> Vec<u8> {
        let u16b = |v: u16| v.to_le_bytes().to_vec();
        let u32b = |v: u32| v.to_le_bytes().to_vec();
        let entry = |tag: u16, kind: u16, count: u32, value: u32| {
            let mut v = u16b(tag);
            v.extend_from_slice(&u16b(kind));
            v.extend_from_slice(&u32b(count));
            v.extend_from_slice(&u32b(value));
            v
        };
        let mut v = b"II".to_vec();
        v.extend_from_slice(&u16b(42));
        v.extend_from_slice(&u32b(8));
        // The first directory, at byte 8, ending at byte 26.
        v.extend_from_slice(&u16b(1));
        v.extend_from_slice(&entry(tag, 4, 1, exif_at));
        v.extend_from_slice(&u32b(0));
        assert_eq!(v.len(), 26);
        // The one it points at: the focal length, as a rational just past the
        // end of it, because eight bytes do not fit in four.
        v.extend_from_slice(&u16b(1));
        v.extend_from_slice(&entry(37386, 5, 1, 44));
        v.extend_from_slice(&u32b(0));
        assert_eq!(v.len(), 44);
        v.extend_from_slice(&u32b(50));
        v.extend_from_slice(&u32b(1));
        v
    }

    #[test]
    fn a_tag_that_points_at_a_directory_is_read_as_the_directory_it_points_at() {
        let d = Document::new(MemSource(with_sub_ifd(34665, 26)));
        let mut ev = Evaluator::new(tiff());
        let entry = [1, 2, 0, 1, 0];
        let at = |tail: &[usize]| [entry.as_slice(), tail].concat();
        assert_eq!(
            ev.node(&d, &at(&[0])).unwrap().value,
            Value::Enum { raw: 34665, name: Some("exif ifd".into()), hex: false }
        );
        // The four bytes are the offset, and the directory they point at costs
        // nothing where it is named, so the entry is still twelve bytes.
        assert_eq!(ev.node(&d, &at(&[4, 0])).unwrap().value, Value::UInt(26));
        let sub = ev.node(&d, &at(&[4, 1])).unwrap();
        assert_eq!(sub.size_bits, 0);
        let ifd = ev.node(&d, &at(&[4, 1, 0])).unwrap();
        assert_eq!(ifd.type_name, "ExifIfd");
        assert_eq!(ifd.offset_bits, 26 * 8);
        // Read against the camera's names rather than the file's: 37386 is a
        // tag nobody has in a TIFF directory.
        assert_eq!(
            ev.node(&d, &at(&[4, 1, 0, 1, 0, 0])).unwrap().value,
            Value::Enum { raw: 37386, name: Some("focal length".into()), hex: false }
        );
        // Eight bytes do not fit in four, so the value is elsewhere again:
        // an offset, and the pair of numbers it points at.
        let num = ev.node(&d, &at(&[4, 1, 0, 1, 0, 4, 1, 0, 0])).unwrap();
        assert_eq!(num.name, "numerator");
        assert_eq!(num.value, Value::UInt(50));
    }

    #[test]
    fn a_directory_that_points_back_at_the_one_holding_it_is_refused() {
        // A `sub ifd` entry saying the directory it points at is the one
        // holding it. Following that would read the same directory for ever.
        // The two the camera writes cannot do this: what they point at is read
        // against another set of names, and nothing in those points back.
        let d = Document::new(MemSource(with_sub_ifd(330, 8)));
        let mut ev = Evaluator::new(tiff());
        assert!(ev.node(&d, &[1, 2, 0, 1, 0, 4, 1, 0]).is_err());
        // The rest of the file still reads, and so does the cursor.
        assert_eq!(ev.node(&d, &[1, 2, 0, 0]).unwrap().value, Value::UInt(1));
        assert_eq!(ev.locate(&d, 8 * 8).unwrap(), vec![1, 2, 0, 0]);
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
            // A short with a count of one fits in the four bytes, so it is
            // read there and the two bytes left over are padding.
            assert_eq!(ev.node(&d, &[1, 2, 0, 1, 0, 3]).unwrap().value, Value::Int(2), "room");
            assert_eq!(ev.node(&d, &[1, 2, 0, 1, 0, 4, 0]).unwrap().value, Value::UInt(640), "{little}");
            assert_eq!(ev.node(&d, &[1, 2, 0, 1, 0, 4, 1]).unwrap().size_bits, 16);
            assert_eq!(
                ev.node(&d, &[1, 2, 0, 1, 1, 0]).unwrap().value,
                Value::Enum { raw: 273, name: Some("strip offsets".into()), hex: false }
            );
            assert_eq!(ev.node(&d, &[1, 2, 0, 1, 1, 4, 0]).unwrap().value, Value::UInt(8));
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
    fn a_value_too_big_for_its_entry_is_read_where_the_entry_says_it_is() {
        // Two entries whose values do not fit: a resolution, which is two
        // numbers, and a name, which is longer than four letters.
        let mut v = b"MM".to_vec();
        v.extend_from_slice(&42u16.to_be_bytes());
        v.extend_from_slice(&8u32.to_be_bytes());
        v.extend_from_slice(&2u16.to_be_bytes());
        for (tag, kind, count, at) in [(282u16, 5u16, 1u32, 38u32), (305, 2, 8, 46)] {
            v.extend_from_slice(&tag.to_be_bytes());
            v.extend_from_slice(&kind.to_be_bytes());
            v.extend_from_slice(&count.to_be_bytes());
            v.extend_from_slice(&at.to_be_bytes());
        }
        v.extend_from_slice(&0u32.to_be_bytes());
        assert_eq!(v.len(), 38);
        v.extend_from_slice(&300u32.to_be_bytes()); // three hundred
        v.extend_from_slice(&1u32.to_be_bytes()); // over one
        v.extend_from_slice(b"qubero\0\0");

        let d = Document::new(MemSource(v));
        let mut ev = Evaluator::new(tiff());
        // Eight bytes needed and four to hold them, so the four are an offset.
        assert_eq!(ev.node(&d, &[1, 2, 0, 1, 0, 3]).unwrap().value, Value::Int(8));
        assert_eq!(ev.node(&d, &[1, 2, 0, 1, 0, 4, 0]).unwrap().value, Value::UInt(38));
        let ratio = ev.node(&d, &[1, 2, 0, 1, 0, 4, 1, 0]).unwrap();
        assert_eq!(ratio.type_name, "Rational");
        assert_eq!(ratio.offset_bits, 38 * 8);
        assert_eq!(ev.node(&d, &[1, 2, 0, 1, 0, 4, 1, 0, 0]).unwrap().value, Value::UInt(300));
        assert_eq!(ev.node(&d, &[1, 2, 0, 1, 0, 4, 1, 0, 1]).unwrap().value, Value::UInt(1));

        // Text keeps the room the count gives it and reads as what is written
        // before the padding, which is how the format stores a name.
        let name = ev.node(&d, &[1, 2, 0, 1, 1, 4, 1, 0]).unwrap();
        assert_eq!(name.value, Value::Str("qubero".into()));
        assert_eq!(name.size_bits, 8 * 8);
    }

    #[test]
    fn an_entry_claiming_more_than_the_file_holds_is_an_error_for_that_entry_alone() {
        // Two entries, the first saying its text is two hundred letters long
        // when there are eight. The second is written correctly.
        let mut v = b"MM".to_vec();
        v.extend_from_slice(&42u16.to_be_bytes());
        v.extend_from_slice(&8u32.to_be_bytes());
        v.extend_from_slice(&2u16.to_be_bytes());
        for (tag, kind, count, at) in [(305u16, 2u16, 200u32, 38u32), (306, 2, 8, 38)] {
            v.extend_from_slice(&tag.to_be_bytes());
            v.extend_from_slice(&kind.to_be_bytes());
            v.extend_from_slice(&count.to_be_bytes());
            v.extend_from_slice(&at.to_be_bytes());
        }
        v.extend_from_slice(&0u32.to_be_bytes());
        v.extend_from_slice(b"qubero\0\0");

        let d = Document::new(MemSource(v));
        let mut ev = Evaluator::new(tiff());
        // The tag and the type of the broken entry still read: what it says
        // about itself is readable even where what it points at is not.
        assert_eq!(
            ev.node(&d, &[1, 2, 0, 1, 0, 0]).unwrap().value,
            Value::Enum { raw: 305, name: Some("software".into()), hex: false }
        );
        assert!(ev.node(&d, &[1, 2, 0, 1, 0, 4, 1, 0]).is_err());
        // And the entry beside it is unharmed.
        assert_eq!(ev.node(&d, &[1, 2, 0, 1, 1, 4, 1, 0]).unwrap().value, Value::Str("qubero".into()));
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
