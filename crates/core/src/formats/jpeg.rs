//! JPEG: two bytes saying where it starts, and then markers all the way down.
//!
//! Every marker is 0xff and a byte that is not zero. Most of them are followed
//! by a length and that many bytes; a handful stand alone and are the whole of
//! what they say. So the file is a list of segments, read until the one that
//! marks the end, and a segment is a switch on which marker it is.
//!
//! The hard part is the scan. After the marker that starts one, the compressed
//! bits run with no count on them anywhere: they end at the next marker, and
//! nothing else says how long they are. An 0xff that is data rather than a
//! marker is written with a zero after it, and the restart markers are meant
//! to be inside the stream rather than to end it, so the terminator and the
//! escape are the same byte and only the one after it tells them apart. That
//! is what `ToMarker` measures, and it is the only thing this format needed
//! that the IR could not already say.
//!
//! The restart markers are left inside the scan rather than shown as segments
//! of their own. Pulling them out would mean the list of segments defaulting
//! to "a piece of a scan" and switching on some sixty marker values to decide
//! otherwise, and a run of restart intervals reads no better as a thousand
//! rows than as one. What they are is written in the marker table, which is
//! where a reader looks for it.
//!
//! The three bytes that pick this template are the start of image and the
//! marker after it, which is why a JPEG-LS file lands here too: its frame
//! marker is one this does not name, so the segment holding it reads as a
//! length and its bytes and everything around it reads as usual. That is more
//! than the signature rules would have said about it, and it is not wrong.
//!
//! The EXIF block in an `APP1` is a whole TIFF file written into a segment,
//! and it is read as one: the same `tiff_file` the TIFF template is, with
//! nothing said twice. What that took was letting an offset count from the
//! copy of a format rather than from the file, since the offsets inside an
//! embedded TIFF count from its own `II` or `MM` partway through the JPEG.
//! The TIFF sits in a window of its own and its offsets are anchored to that
//! window, which is the start of the file when it is a file and the start of
//! the segment when it is not.
//!
//! One thing is named and not laid out. A thumbnail is a whole JPEG inside an
//! `APP0`, and reading it would mean this template referring to itself across
//! a field nothing bounds.

use super::tiff::tiff_file;
use crate::template::{Encoding, Endian::*, Expr as E, StrLen, Template, Ty as T, Until};

/// Every marker worth naming, by the whole two bytes rather than the second
/// one, since that is what is read. The names are the abbreviations the
/// specification uses, which is what anyone reading a JPEG will be looking
/// for, with what they stand for after them.
const MARKER: &[(i128, &str)] = &[
    (0xffd8, "soi, start of image"),
    (0xffc0, "sof0, baseline dct"),
    (0xffc1, "sof1, extended sequential dct"),
    (0xffc2, "sof2, progressive dct"),
    (0xffc3, "sof3, lossless"),
    (0xffc4, "dht, huffman tables"),
    (0xffc5, "sof5, differential sequential dct"),
    (0xffc6, "sof6, differential progressive dct"),
    (0xffc7, "sof7, differential lossless"),
    (0xffc8, "jpg, reserved"),
    (0xffc9, "sof9, arithmetic sequential dct"),
    (0xffca, "sof10, arithmetic progressive dct"),
    (0xffcb, "sof11, arithmetic lossless"),
    (0xffcc, "dac, arithmetic coding conditioning"),
    (0xffcd, "sof13, differential arithmetic sequential dct"),
    (0xffce, "sof14, differential arithmetic progressive dct"),
    (0xffcf, "sof15, differential arithmetic lossless"),
    (0xffd0, "rst0, restart"),
    (0xffd1, "rst1, restart"),
    (0xffd2, "rst2, restart"),
    (0xffd3, "rst3, restart"),
    (0xffd4, "rst4, restart"),
    (0xffd5, "rst5, restart"),
    (0xffd6, "rst6, restart"),
    (0xffd7, "rst7, restart"),
    (0xffd9, "eoi, end of image"),
    (0xffda, "sos, start of scan"),
    (0xffdb, "dqt, quantisation tables"),
    (0xffdc, "dnl, number of lines"),
    (0xffdd, "dri, restart interval"),
    (0xffde, "dhp, hierarchical progression"),
    (0xffdf, "exp, expand reference"),
    (0xffe0, "app0, jfif"),
    (0xffe1, "app1, exif or xmp"),
    (0xffe2, "app2, icc profile"),
    (0xffe3, "app3"),
    (0xffe4, "app4"),
    (0xffe5, "app5"),
    (0xffe6, "app6"),
    (0xffe7, "app7"),
    (0xffe8, "app8"),
    (0xffe9, "app9"),
    (0xffea, "app10"),
    (0xffeb, "app11"),
    (0xffec, "app12, picture info"),
    (0xffed, "app13, photoshop irb"),
    (0xffee, "app14, adobe"),
    (0xffef, "app15"),
    (0xfffe, "com, comment"),
    (0xff01, "tem, temporary"),
];

/// The markers that are the whole of what they say. Everything else is
/// followed by a length and that many bytes.
const STANDALONE: &[i128] =
    &[0xffd8, 0xffd9, 0xff01, 0xffd0, 0xffd1, 0xffd2, 0xffd3, 0xffd4, 0xffd5, 0xffd6, 0xffd7];

/// The markers that open a frame: the same layout in all of them, differing
/// only in what the numbers inside mean to a decoder.
const FRAME: &[i128] = &[
    0xffc0, 0xffc1, 0xffc2, 0xffc3, 0xffc5, 0xffc6, 0xffc7, 0xffc9, 0xffca, 0xffcb, 0xffcd, 0xffce, 0xffcf,
];

/// The bytes that, after an 0xff, mean it is not a marker: zero, which is how
/// an 0xff of data is written, and the eight restarts, which live inside a
/// scan rather than ending one.
const ESCAPES: &[u8] = &[0x00, 0xd0, 0xd1, 0xd2, 0xd3, 0xd4, 0xd5, 0xd6, 0xd7];

pub fn jpeg() -> Template {
    Template::new(
        "jpeg",
        T::structure(
            "JPEG",
            vec![
                ("soi", T::magic(&[0xff, 0xd8])),
                (
                    "segments",
                    T::repeat(segment(), Until::FieldBytes { field: "marker".into(), bytes: vec![0xff, 0xd9] }),
                ),
                // Real files carry all sorts after the end marker: a second
                // copy of the image, a thumbnail, the tail of whatever the
                // file was before it was overwritten. Named as what it is
                // rather than read as segments that are not there.
                ("trailer", T::bytes(E::Remaining)),
            ],
        ),
    )
}

/// One segment: which marker, and then whatever that marker means.
fn segment() -> T {
    let mut cases: Vec<(i128, T)> = STANDALONE.iter().map(|m| (*m, nothing())).collect();
    cases.extend(FRAME.iter().map(|m| (*m, sized(frame()))));
    cases.push((0xffc4, sized(T::repeat(huffman_table(), Until::End))));
    cases.push((0xffdb, sized(T::repeat(quant_table(), Until::End))));
    cases.push((0xffdd, sized(T::inline_structure("Dri", vec![("restart_interval", T::u16(Big))]))));
    cases.push((0xffdc, sized(T::inline_structure("Dnl", vec![("lines", T::u16(Big))]))));
    cases.push((0xffe0, sized(app0())));
    cases.push((0xffe1, sized(app1())));
    cases.push((0xfffe, sized(T::text(StrLen::Fixed(E::Remaining), Encoding::Latin1))));
    cases.push((0xffda, scan()));

    T::structure_named(
        "Segment",
        "marker",
        "",
        vec![
            ("marker", T::enumeration_hex("Marker", T::u16(Big), MARKER)),
            ("body", T::switch(E::field("marker"), cases, sized(T::bytes(E::Remaining)))),
        ],
    )
    .counted_as("segment")
}

/// A length and the bytes it covers. The length counts itself, which is why
/// every segment in the format holds two bytes fewer than it says.
fn sized(inner: T) -> T {
    T::inline_structure(
        "Body",
        vec![
            ("length", T::u16(Big)),
            ("contents", T::sized(E::field("length").sub(E::lit(2)), inner)),
        ],
    )
}

fn nothing() -> T {
    T::bytes(E::lit(0))
}

/// What the image is: how big, how many channels, and how each of them is
/// sampled. Everything a decoder needs before a scan means anything.
fn frame() -> T {
    T::inline_structure(
        "Frame",
        vec![
            ("precision", T::u8()),
            ("height", T::u16(Big)),
            ("width", T::u16(Big)),
            ("component_count", T::u8()),
            ("components", T::array(frame_component(), E::field("component_count"))),
        ],
    )
}

/// One channel of the image. The two sampling factors are how many blocks of
/// this channel go with one block of the coarsest: 2 and 2 on the brightness
/// of an ordinary photograph and 1 and 1 on each of the colours, which is the
/// chroma subsampling the format is known for.
fn frame_component() -> T {
    T::structure_named(
        "Component",
        "id",
        "",
        vec![
            ("id", T::u8()),
            ("h_sampling", T::UInt { bits: 4, endian: Big }),
            ("v_sampling", T::UInt { bits: 4, endian: Big }),
            ("quant_table", T::u8()),
        ],
    )
    .counted_as("component")
}

/// One Huffman table. The sixteen counts are how many codes there are of each
/// length from one bit to sixteen, and the symbols follow in that order. The
/// total is never written down: it is those sixteen numbers added up. The
/// codes themselves are not stored at all, since the counts are enough to
/// rebuild every one of them.
fn huffman_table() -> T {
    T::structure_named(
        "HuffmanTable",
        "id",
        "",
        vec![
            ("class", T::enumeration("TableClass", T::UInt { bits: 4, endian: Big }, &[(0, "dc"), (1, "ac")])),
            ("id", T::UInt { bits: 4, endian: Big }),
            ("counts", T::array(T::u8(), E::lit(16))),
            ("symbols", T::bytes(E::sum_of("counts"))),
        ],
    )
    .counted_as("table")
}

/// One quantisation table: sixty-four numbers, in the zigzag order the
/// coefficients are written in rather than in rows. A byte each, or two each
/// when the precision says so.
fn quant_table() -> T {
    T::structure_named(
        "QuantTable",
        "id",
        "",
        vec![
            (
                "precision",
                T::enumeration("QuantPrecision", T::UInt { bits: 4, endian: Big }, &[(0, "8-bit"), (1, "16-bit")]),
            ),
            ("id", T::UInt { bits: 4, endian: Big }),
            (
                "values",
                T::switch(
                    E::field("precision"),
                    vec![(1, T::array(T::u16(Big), E::lit(64)))],
                    T::array(T::u8(), E::lit(64)),
                ),
            ),
        ],
    )
    .counted_as("table")
}

/// `APP0`, which is JFIF in every file that has one, and which is what makes a
/// JPEG on disk a JPEG rather than a bare stream.
fn app0() -> T {
    T::inline_structure(
        "App0",
        vec![
            ("identifier", T::cstr()),
            ("data", T::matches(E::field("identifier"), vec![("JFIF", jfif())], T::bytes(E::Remaining))),
        ],
    )
}

fn jfif() -> T {
    T::inline_structure(
        "Jfif",
        vec![
            ("version_major", T::u8()),
            ("version_minor", T::u8()),
            // What the two densities below are counted in. None of them means
            // the two are an aspect ratio and nothing more.
            (
                "density_units",
                T::enumeration(
                    "DensityUnits",
                    T::u8(),
                    &[(0, "none, aspect ratio only"), (1, "per inch"), (2, "per centimetre")],
                ),
            ),
            ("x_density", T::u16(Big)),
            ("y_density", T::u16(Big)),
            ("thumbnail_width", T::u8()),
            ("thumbnail_height", T::u8()),
            // Three bytes a pixel, uncompressed, and almost always none at all.
            ("thumbnail", T::bytes(E::Remaining)),
        ],
    )
}

/// `APP1`, which is where a camera writes what it knew when it took the
/// picture. `Exif` is a TIFF file, offsets and all; see the module doc for why
/// it is not read as one here.
fn app1() -> T {
    T::inline_structure(
        "App1",
        vec![
            ("identifier", T::cstr()),
            (
                "data",
                T::matches(
                    E::field("identifier"),
                    vec![
                        (
                            "Exif",
                            T::inline_structure(
                                "Exif",
                                vec![("pad", T::magic(&[0])), ("tiff", tiff_file())],
                            ),
                        ),
                        ("http://ns.adobe.com/xap/1.0/", T::text(StrLen::Fixed(E::Remaining), Encoding::Utf8)),
                    ],
                    T::bytes(E::Remaining),
                ),
            ),
        ],
    )
}

/// The scan: which tables each channel reads with, and then the compressed
/// bits. The header has a length on it and the bits do not.
fn scan() -> T {
    T::inline_structure(
        "Scan",
        vec![
            ("length", T::u16(Big)),
            (
                "header",
                T::sized(
                    E::field("length").sub(E::lit(2)),
                    T::inline_structure(
                        "ScanHeader",
                        vec![
                            ("component_count", T::u8()),
                            ("components", T::array(scan_component(), E::field("component_count"))),
                            // Which coefficients this scan carries and at what
                            // precision. A baseline image writes 0, 63 and 0
                            // once; a progressive one writes a different band
                            // in every scan, which is why one of those arrives
                            // over a slow line as a whole blurry picture that
                            // sharpens rather than as a picture with the
                            // bottom missing.
                            ("spectral_start", T::u8()),
                            ("spectral_end", T::u8()),
                            ("approximation_high", T::UInt { bits: 4, endian: Big }),
                            ("approximation_low", T::UInt { bits: 4, endian: Big }),
                        ],
                    ),
                ),
            ),
            // The compressed bits, ending at the next marker that is neither a
            // stuffed 0xff nor a restart.
            ("entropy", T::bytes(E::to_marker(0xff, ESCAPES))),
        ],
    )
}

/// One channel of a scan, and which of the four Huffman tables of each kind it
/// reads its coefficients with.
fn scan_component() -> T {
    T::structure_named(
        "ScanComponent",
        "id",
        "",
        vec![
            ("id", T::u8()),
            ("dc_table", T::UInt { bits: 4, endian: Big }),
            ("ac_table", T::UInt { bits: 4, endian: Big }),
        ],
    )
    .counted_as("component")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    /// A segment with a length written for it.
    fn seg(marker: u16, body: &[u8]) -> Vec<u8> {
        let mut v = marker.to_be_bytes().to_vec();
        v.extend_from_slice(&((body.len() + 2) as u16).to_be_bytes());
        v.extend_from_slice(body);
        v
    }

    /// A baseline greyscale JPEG: JFIF, a quantisation table, a frame, a
    /// Huffman table, a scan, and something after the end marker.
    fn jpeg_bytes() -> Vec<u8> {
        let mut v = vec![0xff, 0xd8];

        let mut jfif = b"JFIF\0".to_vec();
        jfif.extend_from_slice(&[1, 2, 1]);
        jfif.extend_from_slice(&300u16.to_be_bytes());
        jfif.extend_from_slice(&300u16.to_be_bytes());
        jfif.extend_from_slice(&[0, 0]);
        v.extend_from_slice(&seg(0xffe0, &jfif));

        let mut dqt = vec![0x00]; // eight-bit, table zero
        dqt.extend_from_slice(&[16u8; 64]);
        v.extend_from_slice(&seg(0xffdb, &dqt));

        let mut sof = vec![8u8];
        sof.extend_from_slice(&32u16.to_be_bytes()); // height
        sof.extend_from_slice(&64u16.to_be_bytes()); // width
        sof.extend_from_slice(&[1, 1, 0x22, 0]); // one channel, sampled 2 by 2
        v.extend_from_slice(&seg(0xffc0, &sof));

        let mut dht = vec![0x00]; // dc, table zero
        dht.extend_from_slice(&[0, 1, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]); // three symbols
        dht.extend_from_slice(&[5, 6, 7]);
        v.extend_from_slice(&seg(0xffc4, &dht));

        v.extend_from_slice(&seg(0xffda, &[1, 1, 0x00, 0, 63, 0]));
        // The bits, holding a stuffed 0xff and a restart, then the end marker.
        v.extend_from_slice(&[0xaa, 0xff, 0x00, 0xbb, 0xff, 0xd0, 0xcc]);
        v.extend_from_slice(&[0xff, 0xd9]);
        v.extend_from_slice(b"junk");
        v
    }

    #[test]
    fn the_segments_are_read_and_the_end_marker_stops_them() {
        let d = Document::new(MemSource(jpeg_bytes()));
        let mut ev = Evaluator::new(jpeg());
        assert_eq!(ev.node(&d, &[0]).unwrap().value, Value::Magic { ok: true, bytes: vec![0xff, 0xd8] });
        // Five segments and the end marker, which is one of them.
        assert_eq!(ev.node(&d, &[1]).unwrap().child_count, 6);
        assert_eq!(
            ev.node(&d, &[1, 0, 0]).unwrap().value,
            Value::Enum { raw: 0xffe0, name: Some("app0, jfif".into()), hex: true }
        );
        // The end marker is the whole of what it says, so its body is nothing.
        assert_eq!(
            ev.node(&d, &[1, 5, 0]).unwrap().value,
            Value::Enum { raw: 0xffd9, name: Some("eoi, end of image".into()), hex: true }
        );
        assert_eq!(ev.node(&d, &[1, 5, 1]).unwrap().size_bits, 0);
        // What follows the end marker is named rather than read.
        assert_eq!(ev.node(&d, &[2]).unwrap().value, Value::Bytes { len: 4, preview: b"junk".to_vec() });
    }

    #[test]
    fn the_jfif_and_the_frame_say_what_the_picture_is() {
        let d = Document::new(MemSource(jpeg_bytes()));
        let mut ev = Evaluator::new(jpeg());
        // app0, its length, its contents, the JFIF, its density.
        assert_eq!(ev.node(&d, &[1, 0, 1, 0]).unwrap().value, Value::UInt(16));
        assert_eq!(ev.node(&d, &[1, 0, 1, 1, 0]).unwrap().value, Value::Str("JFIF".into()));
        assert_eq!(
            ev.node(&d, &[1, 0, 1, 1, 1, 2]).unwrap().value,
            Value::Enum { raw: 1, name: Some("per inch".into()), hex: false }
        );
        assert_eq!(ev.node(&d, &[1, 0, 1, 1, 1, 3]).unwrap().value, Value::UInt(300));

        // The frame: width and height the way round the format writes them,
        // which is the other way round from how anyone says them.
        let frame = &[1usize, 2, 1, 1];
        assert_eq!(ev.node(&d, &[frame, &[1][..]].concat()).unwrap().value, Value::UInt(32));
        assert_eq!(ev.node(&d, &[frame, &[2][..]].concat()).unwrap().value, Value::UInt(64));
        // One channel, sampled two by two.
        assert_eq!(ev.node(&d, &[frame, &[4, 0, 1][..]].concat()).unwrap().value, Value::UInt(2));
        assert_eq!(ev.node(&d, &[frame, &[4, 0, 2][..]].concat()).unwrap().value, Value::UInt(2));
    }

    #[test]
    fn a_huffman_table_is_as_long_as_its_counts_add_up_to() {
        let d = Document::new(MemSource(jpeg_bytes()));
        let mut ev = Evaluator::new(jpeg());
        let table = &[1usize, 3, 1, 1, 0];
        assert_eq!(
            ev.node(&d, &[table, &[0][..]].concat()).unwrap().value,
            Value::Enum { raw: 0, name: Some("dc".into()), hex: false }
        );
        // Nothing writes the total down: it is the sixteen counts added up.
        assert_eq!(ev.node(&d, &[table, &[3][..]].concat()).unwrap().value, Value::Bytes { len: 3, preview: vec![5, 6, 7] });
        // One table, filling the segment exactly.
        assert_eq!(ev.node(&d, &[1, 3, 1, 1]).unwrap().child_count, 1);
    }

    #[test]
    fn the_scan_runs_to_the_next_marker_and_not_to_the_ones_inside_it() {
        let d = Document::new(MemSource(jpeg_bytes()));
        let mut ev = Evaluator::new(jpeg());
        // The header says how long it is; the bits that follow do not.
        assert_eq!(ev.node(&d, &[1, 4, 1, 0]).unwrap().value, Value::UInt(8));
        assert_eq!(ev.node(&d, &[1, 4, 1, 1, 1, 0, 1]).unwrap().value, Value::UInt(0));
        let entropy = ev.node(&d, &[1, 4, 1, 2]).unwrap();
        assert_eq!(entropy.size_bits, 7 * 8);
        assert_eq!(entropy.value, Value::Bytes { len: 7, preview: vec![0xaa, 0xff, 0x00, 0xbb, 0xff, 0xd0, 0xcc] });
    }

    #[test]
    fn a_file_cut_off_in_the_middle_of_a_scan_still_reads() {
        let mut v = jpeg_bytes();
        v.truncate(v.len() - 9); // no end marker, no junk, and a scan cut short
        let d = Document::new(MemSource(v));
        let mut ev = Evaluator::new(jpeg());
        // The bits run to the end of the file, since nothing ends them.
        assert_eq!(ev.node(&d, &[1, 4, 1, 2]).unwrap().value, Value::Bytes { len: 4, preview: vec![0xaa, 0xff, 0x00, 0xbb] });
        assert_eq!(ev.node(&d, &[1]).unwrap().child_count, 5);
    }

    #[test]
    fn an_exif_block_is_read_as_the_tiff_file_it_is() {
        // `Exif`, a NUL, a pad byte, and then a whole little-endian TIFF whose
        // offsets count from its own first byte rather than from the file.
        let mut tiff = b"II".to_vec();
        tiff.extend_from_slice(&42u16.to_le_bytes());
        tiff.extend_from_slice(&8u32.to_le_bytes()); // the directory, eight in
        tiff.extend_from_slice(&1u16.to_le_bytes()); // one entry
        tiff.extend_from_slice(&271u16.to_le_bytes()); // make
        tiff.extend_from_slice(&2u16.to_le_bytes()); // ascii
        tiff.extend_from_slice(&6u32.to_le_bytes());
        tiff.extend_from_slice(&26u32.to_le_bytes()); // too long to sit here
        tiff.extend_from_slice(&0u32.to_le_bytes()); // no directory after this
        tiff.extend_from_slice(b"Nikon\0");
        assert_eq!(tiff.len(), 32);

        let mut app1 = b"Exif\0\0".to_vec();
        app1.extend_from_slice(&tiff);
        let mut v = vec![0xff, 0xd8];
        v.extend_from_slice(&seg(0xffe1, &app1));
        v.extend_from_slice(&[0xff, 0xd9]);

        let d = Document::new(MemSource(v));
        let mut ev = Evaluator::new(jpeg());
        // segment, body, contents, App1, data, Exif, tiff, TIFF.
        let exif = &[1usize, 0, 1, 1, 1, 1];
        assert_eq!(ev.node(&d, &[1, 0, 1, 1, 0]).unwrap().value, Value::Str("Exif".into()));
        assert_eq!(
            ev.node(&d, &[exif, &[0][..]].concat()).unwrap().value,
            Value::Enum { raw: 0x4949, name: Some("intel, little-endian".into()), hex: false }
        );

        // The TIFF starts twelve bytes in: two of start-of-image, two of
        // marker, two of length, and six of `Exif` and its padding. Its
        // directory says eight, and eight from the TIFF is twenty from the
        // file. That difference is the whole point.
        let ifd = ev.node(&d, &[exif, &[1, 2, 0][..]].concat()).unwrap();
        assert_eq!(ifd.type_name, "Ifd");
        assert_eq!(ifd.offset_bits, 20 * 8);
        assert_eq!(
            ev.node(&d, &[exif, &[1, 2, 0, 1, 0, 0][..]].concat()).unwrap().value,
            Value::Enum { raw: 271, name: Some("make".into()), hex: false }
        );
        // Six bytes of text is past what an entry can hold, so the entry says
        // where they are and they are read there. Twenty-six from the TIFF is
        // thirty-eight from the file, and the string is at neither of those
        // places by accident.
        assert_eq!(ev.node(&d, &[exif, &[1, 2, 0, 1, 0, 3][..]].concat()).unwrap().value, Value::Int(6));
        assert_eq!(ev.node(&d, &[exif, &[1, 2, 0, 1, 0, 4, 0][..]].concat()).unwrap().value, Value::UInt(26));
        let make = ev.node(&d, &[exif, &[1, 2, 0, 1, 0, 4, 1, 0][..]].concat()).unwrap();
        assert_eq!(make.offset_bits, 38 * 8);
        assert_eq!(make.value, Value::Str("Nikon".into()));
    }

    #[test]
    fn a_progressive_image_has_a_scan_for_each_band_and_each_ends_itself() {
        // Two scans, the second carrying a different band of coefficients.
        // Nothing in the template counts them: each stops at the marker that
        // starts the next, so a second one is a third and a tenth as well.
        let mut v = vec![0xff, 0xd8];
        v.extend_from_slice(&seg(0xffc2, &[8, 0, 8, 0, 8, 1, 1, 0x11, 0]));
        v.extend_from_slice(&seg(0xffda, &[1, 1, 0x00, 0, 0, 0]));
        v.extend_from_slice(&[1, 2, 3]);
        v.extend_from_slice(&seg(0xffda, &[1, 1, 0x00, 1, 63, 0]));
        v.extend_from_slice(&[4, 5, 0xff, 0x00, 6]);
        v.extend_from_slice(&[0xff, 0xd9]);

        let d = Document::new(MemSource(v));
        let mut ev = Evaluator::new(jpeg());
        assert_eq!(ev.node(&d, &[1]).unwrap().child_count, 4);
        assert_eq!(
            ev.node(&d, &[1, 0, 0]).unwrap().value,
            Value::Enum { raw: 0xffc2, name: Some("sof2, progressive dct".into()), hex: true }
        );
        // The first scan carries the direct current alone, the second the rest.
        assert_eq!(ev.node(&d, &[1, 1, 1, 1, 2]).unwrap().value, Value::UInt(0));
        assert_eq!(ev.node(&d, &[1, 2, 1, 1, 3]).unwrap().value, Value::UInt(63));
        assert_eq!(ev.node(&d, &[1, 1, 1, 2]).unwrap().value, Value::Bytes { len: 3, preview: vec![1, 2, 3] });
        assert_eq!(
            ev.node(&d, &[1, 2, 1, 2]).unwrap().value,
            Value::Bytes { len: 5, preview: vec![4, 5, 0xff, 0x00, 6] }
        );
    }
}
