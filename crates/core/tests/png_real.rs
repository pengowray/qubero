//! Every PNG in the sample collection, read by the `png` template from its
//! bytes all the way to its pixels, and checked against a second reading that
//! owes nothing to Qubero.
//!
//! The second reading is a Python script run once over the collection: the
//! chunks walked with `struct`, the IDAT data joined and inflated by `zlib`,
//! the scanlines unfiltered and the Adam7 passes put back by code of its own,
//! and the pixels compared with Pillow 10.2's, which agreed on every file
//! (on the high byte of each sample for the one 16-bit file, since Pillow
//! hands that one back at 8 bits). What it found is written down below,
//! file by file, and this reads each file and has to find the same: the
//! inflated length, the unfiltered bytes, the filter byte of every scanline in
//! order, and every sample of every pixel.
//!
//! The collection has no image in grey with alpha and none whose zlib stream
//! is cut across several IDAT chunks, so those are made here by an encoder of
//! this file's own, with every filter type, every bit depth, and odd sizes
//! that leave some Adam7 passes empty. A file this cannot read has to say why
//! rather than read as something else: the last tests break files on purpose.
//!
//! Point `QUBERO_SAMPLES` at the collection, or keep it beside the checkout as
//! `qubero-samples`. Without it the collection test says so and passes.

use qubero_core::codec::{BlockKind, StepKind};
use qubero_core::document::Document;
use qubero_core::eval::{Evaluator, NodeInfo, Value};
use qubero_core::formats;
use qubero_core::source::MemSource;

/// What the second reading found in one file.
struct Expect {
    /// Under the collection's root.
    file: &'static str,
    size: (u64, u64),
    depth: u8,
    color_type: u8,
    interlaced: bool,
    /// How many bytes the joined IDAT data inflates to.
    inflated: u64,
    /// How many bytes the scanlines come to unfiltered, and their CRC-32.
    unfiltered: (u64, u32),
    /// The CRC-32 of every sample of every pixel, in picture order, each
    /// written as a big-endian 16-bit number whatever the bit depth.
    samples_crc: u32,
    /// The filter byte of every scanline in the order they are stored.
    filters: &'static str,
    /// The samples of the pixels at the top left, at the middle (column
    /// `width / 2` of row `height / 2`) and at the bottom right.
    corners: [&'static [u16]; 3],
}

const EXPECTED: &[Expect] = &[
    Expect {
        file: "apng/default-is-first-frame.apng",
        size: (12, 8),
        depth: 8,
        color_type: 6,
        interlaced: false,
        inflated: 392,
        unfiltered: (384, 0x1a1c80e7),
        samples_crc: 0x0dfeb0c2,
        filters: "00122200",
        corners: [&[0, 0, 0, 0], &[0, 0, 0, 0], &[0, 0, 0, 0]],
    },
    Expect {
        file: "apng/hidden-default-two-frames.apng",
        size: (12, 8),
        depth: 8,
        color_type: 6,
        interlaced: false,
        inflated: 392,
        unfiltered: (384, 0x385b1820),
        samples_crc: 0x20e60252,
        filters: "12222222",
        corners: [&[50, 50, 50, 255], &[50, 50, 50, 255], &[50, 50, 50, 255]],
    },
    Expect {
        file: "pico8/0-saka.p8.png",
        size: (160, 205),
        depth: 8,
        color_type: 6,
        interlaced: false,
        inflated: 131405,
        unfiltered: (131200, 0x6e115656),
        samples_crc: 0x4673eb24,
        filters: "1311111221211421222222244222222222222222222222222222222422422222244244224224422231114242411222222222241242222000033300031122222222222222222222222222222244222222222422222222222222222222222422221224222222224",
        corners: [&[100, 100, 104, 0], &[0, 0, 0, 252], &[100, 100, 104, 0]],
    },
    Expect {
        file: "pico8/p8png-test.p8.png",
        size: (160, 205),
        depth: 8,
        color_type: 6,
        interlaced: false,
        inflated: 131405,
        unfiltered: (131200, 0xd939ea9d),
        samples_crc: 0x9d0526f1,
        filters: "1411222222224421222222244222222222222222222222222222222222222222222222222222222212222222222222222222222222221222222222222222222222222222222222222222222244222222222422242222422422224222222422221224222222224",
        corners: [&[100, 100, 104, 0], &[0, 0, 0, 252], &[100, 100, 104, 0]],
    },
    Expect {
        file: "picotron/picotls.p64.png",
        size: (512, 384),
        depth: 8,
        color_type: 6,
        interlaced: false,
        inflated: 786816,
        unfiltered: (786432, 0x7ede792e),
        samples_crc: 0xeb2b433f,
        filters: "303221123221223222232212112112222222422222222424242442224242444442242424444422424244444224224444422244242244222222222222224222222242222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222242222222242222244222444444444424444222224422222224242142222222242121214111",
        corners: [&[0, 6, 1, 3], &[0, 0, 0, 252], &[0, 0, 0, 0]],
    },
    Expect {
        file: "png/pillow-rgba-icc.png",
        size: (4, 4),
        depth: 8,
        color_type: 6,
        interlaced: false,
        inflated: 68,
        unfiltered: (64, 0xa3ad11ea),
        samples_crc: 0xe408877d,
        filters: "1444",
        corners: [&[0, 0, 130, 90], &[140, 140, 130, 90], &[210, 210, 130, 90]],
    },
    Expect {
        file: "png/png10-palette-trns-text.png",
        size: (4, 4),
        depth: 8,
        color_type: 3,
        interlaced: false,
        inflated: 20,
        unfiltered: (16, 0xa0d13af0),
        samples_crc: 0x261e21e8,
        filters: "0000",
        corners: [&[0], &[1], &[0]],
    },
    Expect {
        file: "png/png10-rgb-critical-only.png",
        size: (5, 3),
        depth: 8,
        color_type: 2,
        interlaced: false,
        inflated: 48,
        unfiltered: (45, 0x3e942e67),
        samples_crc: 0x96316ed2,
        filters: "144",
        corners: [&[0, 0, 100], &[80, 80, 100], &[160, 160, 100]],
    },
    Expect {
        file: "png/png11-iccp.png",
        size: (5, 3),
        depth: 8,
        color_type: 2,
        interlaced: false,
        inflated: 48,
        unfiltered: (45, 0x3e942e67),
        samples_crc: 0x96316ed2,
        filters: "144",
        corners: [&[0, 0, 100], &[80, 80, 100], &[160, 160, 100]],
    },
    Expect {
        file: "png/png11-srgb-splt.png",
        size: (5, 3),
        depth: 8,
        color_type: 2,
        interlaced: false,
        inflated: 48,
        unfiltered: (45, 0x3e942e67),
        samples_crc: 0x96316ed2,
        filters: "144",
        corners: [&[0, 0, 100], &[80, 80, 100], &[160, 160, 100]],
    },
    Expect {
        file: "png/png12-itxt-deflated-ja.png",
        size: (5, 3),
        depth: 8,
        color_type: 2,
        interlaced: false,
        inflated: 48,
        unfiltered: (45, 0x3e942e67),
        samples_crc: 0x96316ed2,
        filters: "144",
        corners: [&[0, 0, 100], &[80, 80, 100], &[160, 160, 100]],
    },
    Expect {
        file: "png/png12-itxt-plain-fr.png",
        size: (5, 3),
        depth: 8,
        color_type: 2,
        interlaced: false,
        inflated: 48,
        unfiltered: (45, 0x3e942e67),
        samples_crc: 0x96316ed2,
        filters: "144",
        corners: [&[0, 0, 100], &[80, 80, 100], &[160, 160, 100]],
    },
    Expect {
        file: "png/pngsuite-basi6a16-rgba-16bit-interlaced.png",
        size: (32, 32),
        depth: 16,
        color_type: 6,
        interlaced: true,
        inflated: 8252,
        unfiltered: (8192, 0x3e3ca2cc),
        samples_crc: 0x632e0a2a,
        filters: "103010441441144244441444444401414224444242241144444444442421",
        corners: [&[65535, 65535, 0, 0], &[0, 0, 65535, 63421], &[0, 0, 65535, 0]],
    },
    Expect {
        file: "png/pngsuite-basn0g01-grey-1bit.png",
        size: (32, 32),
        depth: 1,
        color_type: 0,
        interlaced: false,
        inflated: 160,
        unfiltered: (128, 0xb71a0667),
        samples_crc: 0x57316490,
        filters: "00000000000000000000000000000000",
        corners: [&[1], &[0], &[0]],
    },
    Expect {
        file: "png/pngsuite-ctzn0g04-text-ztxt.png",
        size: (32, 32),
        depth: 4,
        color_type: 0,
        interlaced: false,
        inflated: 544,
        unfiltered: (512, 0x7dbd9bfd),
        samples_crc: 0x5c0c1c47,
        filters: "00000000000000000000000000000000",
        corners: [&[15], &[2], &[15]],
    },
    Expect {
        file: "png/pngsuite-tbbn3p08-palette-trns-bkgd.png",
        size: (32, 32),
        depth: 8,
        color_type: 3,
        interlaced: false,
        inflated: 1056,
        unfiltered: (1024, 0x2111c7b4),
        samples_crc: 0x6f4b7d2c,
        filters: "00000000000000000000000000000000",
        corners: [&[0], &[139], &[0]],
    },
];

/// The folders the collection keeps PNGs in, every file of which has to be in
/// [`EXPECTED`], so a sample added later is not quietly left out.
const FOLDERS: &[(&str, &str)] = &[("png", ".png"), ("apng", ".apng"), ("pico8", ".png"), ("picotron", ".png")];

/// Where the steps from the file to the pixels are, found by name.
struct Steps {
    /// The zlib stream the IDAT chunks' data joins into.
    zlib: Vec<usize>,
    /// What it inflates to, read as scanlines.
    scanlines: Vec<usize>,
    /// The scanlines' trace: one block a scanline.
    blocks: Vec<usize>,
    /// The pixels, in picture order.
    pixels: Vec<usize>,
}

fn named(ev: &mut Evaluator, d: &Document<MemSource>, at: &[usize], name: &str) -> Vec<usize> {
    ev.child_named(d, at, name).unwrap().unwrap_or_else(|| panic!("no field {name} under {at:?}"))
}

fn steps(ev: &mut Evaluator, d: &Document<MemSource>) -> Steps {
    let image = named(ev, d, &[], "image");
    let zlib = [image.as_slice(), &[0]].concat();
    let compressed = named(ev, d, &zlib, "compressed");
    let inflated = [compressed.as_slice(), &[0]].concat();
    let scanlines = named(ev, d, &inflated, "scanlines");
    let unfiltered = [scanlines.as_slice(), &[0]].concat();
    let pixels = named(ev, d, &unfiltered, "pixels");
    let blocks = [scanlines.as_slice(), &[1]].concat();
    Steps { zlib, scanlines, blocks, pixels }
}

fn node(ev: &mut Evaluator, d: &Document<MemSource>, path: &[usize]) -> NodeInfo {
    ev.node(d, path).unwrap_or_else(|e| panic!("{path:?}: {e:?}"))
}

/// The samples of pixel `i`, in the order the colour type names them.
fn samples(ev: &mut Evaluator, d: &Document<MemSource>, pixels: &[usize], i: u64) -> Vec<u16> {
    let at = [pixels, &[i as usize]].concat();
    let n = node(ev, d, &at).child_count as usize;
    (0..n)
        .map(|k| {
            let v = node(ev, d, &[at.as_slice(), &[k]].concat()).value;
            v.as_int().unwrap_or_else(|| panic!("pixel {i} sample {k} is {v:?}")) as u16
        })
        .collect()
}

/// The filter byte of a scanline, read off the field of its block that the
/// trace calls `filter`.
fn filter_of(ev: &mut Evaluator, d: &Document<MemSource>, block: &[usize]) -> u8 {
    let n = node(ev, d, block).child_count as usize;
    for k in 0..n {
        let f = node(ev, d, &[block, &[k]].concat());
        if f.name == "filter" {
            match f.value {
                Value::Enum { raw, .. } => return raw as u8,
                other => panic!("a filter of {other:?}"),
            }
        }
    }
    panic!("no filter in the scanline at {block:?}")
}

/// Whether the zlib stream's Adler-32 is the Adler-32 of what it inflated to.
///
/// Worked out here rather than asked of the field's check: a check is only
/// taken of a field of the file itself, and this one is a field of the joined
/// IDAT data.
fn adler_holds(ev: &mut Evaluator, d: &Document<MemSource>, s: &Steps) -> bool {
    let at = named(ev, d, &s.zlib, "adler32");
    let stored = node(ev, d, &at).value.as_int().unwrap() as u32;
    let (inflated, _) = space_of(ev, d, &s.scanlines);
    let (mut a, mut b) = (1u32, 0u32);
    for &x in &inflated {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    stored == (b << 16 | a)
}

/// The unpacked bytes and the trace of the space a node was read in.
fn space_of(ev: &mut Evaluator, d: &Document<MemSource>, path: &[usize]) -> (Vec<u8>, qubero_core::codec::Trace) {
    let space = node(ev, d, path).space;
    let bytes = ev.space_bytes(space).expect("the space is open").to_vec();
    let trace = ev.space_trace(space).expect("the space has a trace").clone();
    (bytes, trace)
}

#[test]
fn every_png_in_the_collection_reads_to_the_pixels_a_second_reading_found() {
    let Some(root) = qubero_samples::dir("png").and_then(|p| p.parent().map(|p| p.to_path_buf())) else {
        eprintln!("{}", qubero_samples::missing_dir("png"));
        return;
    };
    // Every PNG the collection holds is one of the files below.
    for (folder, ext) in FOLDERS {
        let Ok(entries) = std::fs::read_dir(root.join(folder)) else { continue };
        for entry in entries.flatten() {
            let name = format!("{folder}/{}", entry.file_name().to_string_lossy());
            if name.ends_with(ext) && !name.ends_with(".rom") {
                assert!(EXPECTED.iter().any(|e| e.file == name), "{name} is a PNG with nothing expected of it");
            }
        }
    }
    for e in EXPECTED {
        let bytes = std::fs::read(root.join(e.file)).unwrap_or_else(|_| panic!("{} is not in the collection", e.file));
        let d = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(formats::builtin("png").unwrap());
        let s = steps(&mut ev, &d);
        let file = e.file;

        // The zlib stream's own checksum, over what it inflated to.
        assert!(adler_holds(&mut ev, &d, &s), "{file}: the Adler-32 does not match");

        // What the IDAT data inflates to, and the trace of the inflating.
        let (inflated, deflate) = space_of(&mut ev, &d, &s.scanlines);
        assert_eq!(inflated.len() as u64, e.inflated, "{file}: inflated length");
        deflate.check_tiles().unwrap_or_else(|why| panic!("{file}: the deflate trace {why}"));

        // The unfiltered scanlines, and the trace of unfiltering them.
        let (unfiltered, trace) = space_of(&mut ev, &d, &s.pixels);
        assert_eq!((unfiltered.len() as u64, crc32(&unfiltered)), e.unfiltered, "{file}: unfiltered bytes");
        trace.check_tiles().unwrap_or_else(|why| panic!("{file}: the scanline trace {why}"));
        assert!(trace.blocks().iter().all(|b| b.kind == BlockKind::Scanline), "{file}: a block that is not a scanline");

        // The filter byte of every scanline, read off the scanlines' blocks.
        let n = node(&mut ev, &d, &s.blocks).child_count as usize;
        assert_eq!(n, e.filters.len(), "{file}: scanline count");
        let filters: String = (0..n)
            .map(|i| char::from(b'0' + filter_of(&mut ev, &d, &[s.blocks.as_slice(), &[i]].concat())))
            .collect();
        assert_eq!(filters, e.filters, "{file}: filter bytes");

        // Every sample of every pixel, in picture order.
        let (w, h) = e.size;
        let raster = node(&mut ev, &d, &s.pixels);
        assert_eq!(raster.child_count, w * h, "{file}: pixel count");
        let order = if e.interlaced { "raster, Adam7 order" } else { "raster, row order" };
        assert_eq!(raster.type_name, order, "{file}");
        // About five thousand pixels a file are read as fields, which keeps a
        // debug build's run of this test to seconds.
        let every = (w * h / 5_000).max(1);
        if every == 1 {
            let mut all = Vec::new();
            for i in 0..w * h {
                for v in samples(&mut ev, &d, &s.pixels, i) {
                    all.extend_from_slice(&v.to_be_bytes());
                }
            }
            assert_eq!(crc32(&all), e.samples_crc, "{file}: the samples of the pixels");
        } else {
            // A larger picture is read a pixel in `every`, each against the
            // unfiltered bytes the second reading has already vouched for,
            // unpacked here: rows in order, a byte a sample.
            assert!(!e.interlaced && e.depth == 8, "{file}: only a plain eight-bit picture is sampled");
            let channels = unfiltered.len() as u64 / (w * h);
            for i in (0..w * h).step_by(every as usize) {
                let want: Vec<u16> = (0..channels).map(|k| unfiltered[(i * channels + k) as usize] as u16).collect();
                assert_eq!(samples(&mut ev, &d, &s.pixels, i), want, "{file}: pixel {i}");
            }
        }
        let at = |x: u64, y: u64| y * w + x;
        for (i, want) in [at(0, 0), at(w / 2, h / 2), at(w - 1, h - 1)].into_iter().zip(e.corners) {
            assert_eq!(samples(&mut ev, &d, &s.pixels, i), want, "{file}: pixel {i}");
        }
        // The depth and colour type the file declares, as the pixel read them.
        let first = node(&mut ev, &d, &[s.pixels.as_slice(), &[0, 0]].concat());
        assert_eq!(first.size_bits, e.depth as u64, "{file}: sample width");
        assert_eq!(first.path.len(), s.pixels.len() + 2);
        let _ = e.color_type;
    }
}

/// A pixel of the interlaced 16-bit file, followed back up the chain: which
/// scanline its bytes were unfiltered from, and which deflate codes wrote
/// those bytes. The notes on the hand-written report traced the same pixel by
/// hand, at column 13 of row 9: pass 7, which is every odd row.
#[test]
fn a_pixel_of_an_interlaced_file_leads_back_to_its_scanline_and_its_codes() {
    let Some(dir) = qubero_samples::dir("png") else {
        eprintln!("{}", qubero_samples::missing_dir("png"));
        return;
    };
    let bytes = std::fs::read(dir.join("pngsuite-basi6a16-rgba-16bit-interlaced.png")).unwrap();
    let d = Document::new(MemSource(bytes));
    let mut ev = Evaluator::new(formats::builtin("png").unwrap());
    let s = steps(&mut ev, &d);
    let pixel = node(&mut ev, &d, &[s.pixels.as_slice(), &[9 * 32 + 13]].concat());
    assert_eq!(pixel.size_bits, 64);
    // Pass 7 starts after passes 1 to 6, which hold 16 + 16 + 32 + 64 + 128
    // + 256 pixels of eight bytes; row 4 of pass 7 is image row 9, and each
    // of its rows is 32 pixels.
    let pass7 = (16 + 16 + 32 + 64 + 128 + 256) * 8;
    assert_eq!(pixel.offset_bits / 8, pass7 + (4 * 32 + 13) * 8);
    let (_, unfilter) = space_of(&mut ev, &d, &s.pixels);
    let step = unfilter.map_out(pixel.offset_bits / 8).expect("a step made it");
    assert_eq!(step.kind, StepKind::Filtered);
    // The scanline that step belongs to says where it is.
    let block = unfilter.blocks().iter().position(|b| b.in_bits.start <= step.in_bits.start && step.in_bits.end <= b.in_bits.end).unwrap();
    let at = [s.blocks.as_slice(), &[block]].concat();
    assert_eq!(node(&mut ev, &d, &at).name, "pass 7, row 4, filter paeth");
    // The last field of the scanline is its filtered row, read as the bytes
    // it is in the inflated stream, where the filter byte in front of it is.
    let row = node(&mut ev, &d, &[at.as_slice(), &[3]].concat());
    assert_eq!(row.name, "filtered row");
    assert_eq!((row.offset_bits, row.size_bits), (step.in_bits.start, 256 * 8));
    let (inflated, _) = space_of(&mut ev, &d, &s.scanlines);
    let from = (step.in_bits.start / 8) as usize;
    match row.value {
        Value::Bytes { len, preview } => {
            assert_eq!(len, 256);
            assert_eq!(preview, inflated[from..from + preview.len()]);
        }
        other => panic!("a filtered row of {other:?}"),
    }
    // And the inflated byte at the start of that row was written by a code of
    // the deflate stream, in the joined IDAT data.
    let (_, deflate) = space_of(&mut ev, &d, &s.scanlines);
    let code = deflate.map_out(step.in_bits.start / 8).expect("a code wrote it");
    assert!(matches!(code.kind, StepKind::Literal(_) | StepKind::Match { .. }), "{code:?}");
}

/// The shapes the collection has no file of, made here: grey with alpha,
/// every depth below eight bits interlaced, palettes, sixteen-bit RGB, sizes
/// that leave Adam7 passes empty, and zlib streams cut across several IDAT
/// chunks, one of them after the first byte and one of them empty. Every
/// filter type is used, and every pixel has to come back as it went in.
///
/// With `PNG_MADE_DIR` set, each file is written there as well, so another
/// decoder can be asked whether they are the images this says they are.
/// Pillow 10.2 read all thirteen to the same pixels on 2026-09-24, to the
/// high byte of each sample where it hands sixteen-bit colour back at eight.
#[test]
fn made_images_of_every_shape_read_back_to_the_pixels_they_were_made_from() {
    let cases = [
        // (width, height, depth, colour type, interlaced, IDAT chunks)
        (7, 5, 8, 4, false, 1),
        (9, 9, 16, 4, true, 3),
        (13, 7, 1, 0, true, 2),
        (10, 3, 2, 0, true, 1),
        (5, 11, 4, 0, true, 4),
        (17, 5, 1, 3, true, 1),
        (17, 5, 2, 3, false, 2),
        (9, 6, 4, 3, true, 3),
        (3, 2, 8, 3, true, 1),
        (6, 6, 16, 2, true, 5),
        (1, 1, 8, 6, true, 1),
        (33, 17, 8, 6, false, 7),
        (4, 9, 16, 0, true, 2),
    ];
    for (w, h, depth, ct, interlaced, pieces) in cases {
        let what = format!("{w}x{h} depth {depth} colour type {ct}{} in {pieces} IDAT", if interlaced { " interlaced" } else { "" });
        let mut img = Made::new(w, h, depth, ct, interlaced);
        let bytes = img.encode(pieces);
        if let Some(dir) = std::env::var_os("PNG_MADE_DIR") {
            let name = format!("{w}x{h}-d{depth}-ct{ct}-{}-{pieces}idat.png", if interlaced { "adam7" } else { "rows" });
            std::fs::write(std::path::Path::new(&dir).join(&name), &bytes).unwrap();
            // Beside it, every sample it was made from, as big-endian u16s.
            let samples: Vec<u8> = img.pixels.iter().flatten().flat_map(|v| v.to_be_bytes()).collect();
            std::fs::write(std::path::Path::new(&dir).join(format!("{name}.samples")), samples).unwrap();
        }
        let d = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(formats::builtin("png").unwrap());
        let s = steps(&mut ev, &d);
        assert_eq!(node(&mut ev, &d, &s.pixels).child_count, (w * h) as u64, "{what}");
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) as u64;
                assert_eq!(samples(&mut ev, &d, &s.pixels, i), img.pixels[i as usize], "{what}: pixel {x},{y}");
            }
        }
        let n = node(&mut ev, &d, &s.blocks).child_count as usize;
        assert_eq!(n, img.filters.len(), "{what}: scanlines");
        for (i, want) in img.filters.iter().enumerate() {
            assert_eq!(filter_of(&mut ev, &d, &[s.blocks.as_slice(), &[i]].concat()), *want, "{what}: scanline {i}");
        }
        let (_, trace) = space_of(&mut ev, &d, &s.pixels);
        trace.check_tiles().unwrap_or_else(|why| panic!("{what}: {why}"));
        assert!(adler_holds(&mut ev, &d, &s), "{what}: Adler-32");
    }
}

/// Why a broken image stops where it stops. Each one is left as the bytes it
/// is, with the reason on the node, rather than read into pixels that are
/// not there.
#[test]
fn a_broken_image_says_why_it_has_no_pixels() {
    // Why the first of the two streams that did not open would not: the
    // joined IDAT data's inflating, or the scanlines' unfiltering.
    let refused = |bytes: Vec<u8>| {
        let d = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(formats::builtin("png").unwrap());
        let image = named(&mut ev, &d, &[], "image");
        let zlib = [image.as_slice(), &[0]].concat();
        let compressed = named(&mut ev, &d, &zlib, "compressed");
        let n = node(&mut ev, &d, &compressed);
        if n.refused.is_some() {
            return n.refused;
        }
        let scanlines = named(&mut ev, &d, &[compressed.as_slice(), &[0]].concat(), "scanlines");
        let n = node(&mut ev, &d, &scanlines);
        if n.refused.is_some() {
            assert_eq!(n.child_count, 0, "a stream that did not open holds nothing");
        }
        n.refused
    };
    let img = Made::new(8, 8, 8, 2, true);
    assert_eq!(refused(img.clone().encode(3)), None, "the image before it was broken");

    // A header one row taller than the scanlines: the stream is too short for
    // the image it is said to be.
    let mut taller = img.clone();
    taller.header_height = Some(9);
    assert_eq!(refused(taller.encode(1)).as_deref(), Some("failed"), "a row short");

    // A filter byte of 5, which is no filter.
    let mut bad = img.clone();
    bad.bad_filter = true;
    assert_eq!(refused(bad.encode(2)).as_deref(), Some("failed"), "filter 5");

    // RGB at four bits, which the specification does not allow: no decoder
    // was asked, and the reason says so rather than calling the data damaged.
    let mut odd = img.clone();
    odd.header_depth = Some(4);
    assert_eq!(refused(odd.encode(1)).as_deref(), Some("settings"), "rgb at 4 bits");

    // The zlib stream cut short: the last IDAT chunk loses its last ten bytes.
    let mut cut = img.clone();
    cut.cut = 10;
    assert_eq!(refused(cut.encode(2)).as_deref(), Some("failed"), "cut short");
}

/// An image made by this file's own encoder, which shares nothing with the
/// reader: the passes are laid out here, and the filters run forwards.
#[derive(Clone)]
struct Made {
    width: usize,
    height: usize,
    depth: u8,
    color_type: u8,
    interlaced: bool,
    /// Every pixel's samples, in picture order.
    pixels: Vec<Vec<u16>>,
    /// The filter used for each scanline, in the order they are written.
    filters: Vec<u8>,
    header_height: Option<u32>,
    header_depth: Option<u8>,
    bad_filter: bool,
    cut: usize,
}

const PASSES: [(usize, usize, usize, usize); 7] =
    [(0, 0, 8, 8), (4, 0, 8, 8), (0, 4, 4, 8), (2, 0, 4, 4), (0, 2, 2, 4), (1, 0, 2, 2), (0, 1, 1, 2)];

impl Made {
    fn new(width: usize, height: usize, depth: u8, color_type: u8, interlaced: bool) -> Made {
        let channels = match color_type {
            2 => 3,
            4 => 2,
            6 => 4,
            _ => 1,
        };
        let top = if depth == 16 { 0xffff } else { (1u32 << depth) - 1 };
        // Numbers that vary in every sample and in every bit, from a small
        // linear congruential generator, so no two runs of the test differ.
        let mut seed = 0x1234_5678u32 ^ (width * 31 + height * 7 + depth as usize + color_type as usize * 101) as u32;
        let mut next = || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            seed >> 8
        };
        let pixels = (0..width * height).map(|_| (0..channels).map(|_| (next() % (top + 1)) as u16).collect()).collect();
        Made {
            width,
            height,
            depth,
            color_type,
            interlaced,
            pixels,
            filters: Vec::new(),
            header_height: None,
            header_depth: None,
            bad_filter: false,
            cut: 0,
        }
    }

    /// The whole file, its zlib stream cut into `pieces` IDAT chunks: the first
    /// one byte long, then one empty, then the rest split evenly.
    fn encode(&mut self, pieces: usize) -> Vec<u8> {
        let raw = self.scanlines();
        let mut z = miniz_oxide::deflate::compress_to_vec_zlib(&raw, 6);
        z.truncate(z.len() - self.cut);
        let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut ihdr = (self.width as u32).to_be_bytes().to_vec();
        ihdr.extend_from_slice(&self.header_height.unwrap_or(self.height as u32).to_be_bytes());
        ihdr.extend_from_slice(&[self.header_depth.unwrap_or(self.depth), self.color_type, 0, 0, self.interlaced as u8]);
        chunk(&mut out, b"IHDR", &ihdr);
        if self.color_type == 3 {
            let palette: Vec<u8> = (0..(1usize << self.depth) * 3).map(|i| (i * 37) as u8).collect();
            chunk(&mut out, b"PLTE", &palette);
        }
        let mut cuts = vec![0];
        if pieces > 1 {
            cuts.push(1);
        }
        if pieces > 2 {
            cuts.push(1);
        }
        let rest = pieces.saturating_sub(cuts.len());
        for k in 1..=rest {
            cuts.push(1 + (z.len() - 1) * k / (rest + 1));
        }
        cuts.push(z.len());
        for pair in cuts.windows(2) {
            chunk(&mut out, b"IDAT", &z[pair[0]..pair[1]]);
        }
        chunk(&mut out, b"IEND", &[]);
        out
    }

    /// Every scanline, filtered, with its filter byte in front: the filters
    /// taken in turn so that each of the five is used.
    fn scanlines(&mut self) -> Vec<u8> {
        let bits = self.depth as usize * self.pixels[0].len();
        let bpp = bits.div_ceil(8);
        let passes: Vec<(usize, usize, usize, usize)> = if self.interlaced { PASSES.to_vec() } else { vec![(0, 0, 1, 1)] };
        let mut out = Vec::new();
        self.filters.clear();
        for (p, (x0, y0, dx, dy)) in passes.into_iter().enumerate() {
            let cols = self.width.saturating_sub(x0).div_ceil(dx);
            let rows = self.height.saturating_sub(y0).div_ceil(dy);
            if cols == 0 || rows == 0 {
                continue;
            }
            let stride = (cols * bits).div_ceil(8);
            let mut prev = vec![0u8; stride];
            for r in 0..rows {
                let mut row = vec![0u8; stride];
                let mut bit = 0;
                for c in 0..cols {
                    for &v in &self.pixels[(y0 + r * dy) * self.width + x0 + c * dx] {
                        if self.depth == 16 {
                            row[bit / 8..bit / 8 + 2].copy_from_slice(&v.to_be_bytes());
                        } else if self.depth == 8 {
                            row[bit / 8] = v as u8;
                        } else {
                            row[bit / 8] |= (v as u8) << (8 - self.depth as usize - bit % 8);
                        }
                        bit += self.depth as usize;
                    }
                }
                let filter = ((p * 3 + r) % 5) as u8;
                self.filters.push(filter);
                out.push(if self.bad_filter && self.filters.len() == 3 { 5 } else { filter });
                for i in 0..stride {
                    let a = if i >= bpp { row[i - bpp] } else { 0 };
                    let b = prev[i];
                    let c = if i >= bpp { prev[i - bpp] } else { 0 };
                    let guess = match filter {
                        0 => 0,
                        1 => a,
                        2 => b,
                        3 => ((a as u16 + b as u16) / 2) as u8,
                        _ => {
                            let p = a as i16 + b as i16 - c as i16;
                            let (pa, pb, pc) = ((p - a as i16).abs(), (p - b as i16).abs(), (p - c as i16).abs());
                            if pa <= pb && pa <= pc {
                                a
                            } else if pb <= pc {
                                b
                            } else {
                                c
                            }
                        }
                    };
                    out.push(row[i].wrapping_sub(guess));
                }
                prev = row;
            }
        }
        out
    }
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let from = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let crc = crc32(&out[from..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

/// CRC-32 as PNG and zlib's `crc32` compute it, a bit at a time.
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in bytes {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xedb8_8320 } else { crc >> 1 };
        }
    }
    !crc
}
