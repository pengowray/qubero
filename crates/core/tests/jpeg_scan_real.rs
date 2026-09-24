//! Every baseline scan in the sample collection's JPEGs, decoded by
//! `codec::jpeg` through the template, and held against a decoder written
//! apart from it.
//!
//! `tools/jpeg_reference.py` is that decoder. It reads the segments with
//! `struct`, looks Huffman codes up as strings of noughts and ones, and takes
//! the stuffing out before reading a bit; with `--pillow` it also runs the
//! inverse DCT and holds every full-resolution channel against Pillow's own
//! decode, which agrees within one level on all four files below. The numbers
//! pinned here are what it prints. Run it again after changing a sample:
//!
//! ```text
//! python3 tools/jpeg_reference.py --pillow ../qubero-samples/jpeg/*.jpg
//! ```
//!
//! Two things are checked of every scan. The coefficients have to be the
//! reference's, all of them, which a CRC-32 of the whole decoded space says.
//! And the scan's bits have to be accounted for exactly: every Huffman code,
//! every value bit, every padding bit, every stuffed zero and every restart
//! marker, adding up to the length the template measured. A decoder that
//! loses one bit passes neither.
//!
//! The collection is not in this repository: point `QUBERO_SAMPLES` at it, or
//! keep it beside the checkout as `qubero-samples`. With neither, this says so
//! and passes.

use qubero_core::checksum::crc32;
use qubero_core::codec::{StepField, StepKind};
use qubero_core::document::Document;
use qubero_core::eval::{Evaluator, Value};
use qubero_core::formats::jpeg;
use qubero_core::source::MemSource;

/// What the reference decoder says about one scan.
struct Pinned {
    blocks: u64,
    crc: u32,
    code_bits: u64,
    value_bits: u64,
    padding_bits: u64,
    stuffed: u64,
    markers: u64,
}

const fn pin(blocks: u64, crc: u32, code_bits: u64, value_bits: u64, padding_bits: u64, stuffed: u64, markers: u64) -> Pinned {
    Pinned { blocks, crc, code_bits, value_bits, padding_bits, stuffed, markers }
}

/// The sample collection's baseline files, and what the reference says of
/// each one's scan.
const BASELINE: &[(&str, &[Pinned])] = &[
    // 227 by 149, 4:2:0, the IJG's own test image, as in the hand-written report.
    ("jpeg/libjpeg-turbo-testorig-baseline.jpg", &[pin(900, 0x01281baf, 26677, 14413, 6, 8, 0)]),
    // Four channels, CMYK by the Adobe segment, with a restart every MCU row.
    ("jpeg/pillow-cmyk-adobe-restart.jpg", &[pin(384, 0x9e970d53, 17367, 10614, 27, 56, 7)]),
    // One channel, so one block per MCU.
    ("jpeg/pillow-grey-comment.jpg", &[pin(96, 0xa1cc9b70, 5280, 3243, 5, 9, 0)]),
    // 4:4:4: three blocks per MCU, one of each channel.
    ("jpeg/pillow-ycc444-exif.jpg", &[pin(288, 0x8e5f5c6a, 12462, 7624, 2, 26, 0)]),
];

/// Layouts the collection has no file of, made by `tools/jpeg_fixtures.py`
/// and kept in `tests/fixtures/jpeg`, so these are checked on every run.
const FIXTURES: &[(&str, &[Pinned])] = &[
    // 4:2:2: two Y blocks side by side, then a Cb and a Cr. 45 by 29, so
    // neither side is a whole number of MCUs.
    ("pillow-422-45x29.jpg", &[pin(48, 0x8948ab25, 6463, 3471, 2, 3, 0)]),
    // 4:2:0 with a restart marker after every MCU.
    ("pillow-420-restart-45x29.jpg", &[pin(36, 0xbe151ffe, 3448, 1622, 26, 4, 5)]),
    // The same picture as three scans of one channel each. A scan of one
    // channel codes that channel's own blocks, 6 by 4 for Y and 3 by 2 for
    // the others, not the MCU grid's; its restart interval counts blocks; and
    // the chroma scans read with a table 0 that was redefined after the Y
    // scan.
    (
        "three-scans-45x29.jpg",
        &[pin(24, 0x03d454ec, 2575, 1287, 18, 2, 4), pin(6, 0x54e3ec12, 582, 203, 7, 0, 0), pin(6, 0x867cba5f, 290, 128, 6, 2, 0)],
    ),
];

/// JPEGs written inside other files of the collection, by where each starts.
/// Camera raw files carry previews, Motion JPEG is a JPEG a frame, Photoshop
/// keeps a thumbnail, PowerPoint stores one in its ZIP. A camera's preview is
/// where 4:2:2 is found in the wild. Each is read from its start as a JPEG on
/// its own, the way a stored ZIP entry or a preview strip opens in a tab.
const EMBEDDED: &[(&str, usize, &[Pinned])] = &[
    ("avi/mjpeg-pcm-s16le.avi", 9990, &[pin(144, 0xffd0193d, 5029, 2650, 1, 2, 0)]),
    ("avi/mjpeg-pcm-s16le.avi", 17444, &[pin(144, 0x072957bb, 5027, 2663, 6, 4, 0)]),
    ("avi/mjpeg-pcm-s16le.avi", 22846, &[pin(144, 0x7abfdf2b, 5026, 2644, 2, 3, 0)]),
    ("avi/mjpeg-pcm-s16le.avi", 28246, &[pin(144, 0x9784fd40, 5036, 2640, 4, 1, 0)]),
    ("avi/mjpeg-pcm-s16le.avi", 33644, &[pin(144, 0xae7f58bb, 5041, 2648, 7, 2, 0)]),
    ("avi/mjpeg-pcm-s16le.avi", 39042, &[pin(144, 0x99fd3b16, 5046, 2648, 2, 2, 0)]),
    ("avi/mjpeg-pcm-s16le.avi", 44438, &[pin(144, 0x0b7aaf07, 5026, 2625, 5, 0, 0)]),
    ("avi/mjpeg-pcm-s16le.avi", 51886, &[pin(144, 0xae1b7601, 5008, 2589, 3, 1, 0)]),
    ("avi/mjpeg-pcm-s16le.avi", 57276, &[pin(144, 0xc0892e1e, 4977, 2558, 1, 3, 0)]),
    ("avi/mjpeg-pcm-s16le.avi", 62662, &[pin(144, 0x3e75bd52, 4924, 2528, 4, 3, 0)]),
    ("cameraraw/canon-eos-40d-sraw2.cr2", 34072, &[pin(600, 0xe251773e, 36930, 22481, 5, 56, 0)]),
    ("cameraraw/canon-eos-40d-sraw2.cr2", 42426, &[pin(77924, 0x4aceead4, 1931638, 972261, 5, 2860, 0)]),
    ("cameraraw/exiftool-panasonic-lx3-stub.rw2", 1536, &[pin(6, 0xb4f56111, 13, 6, 5, 0, 0)]),
    ("cameraraw/nikon-coolscan-iv-ed-film-scan.nef", 221268, &[pin(16500, 0xe3fa9691, 2058304, 1402526, 2, 1655, 0)]),
    ("cameraraw/nikon-d1h-12bit-uncompressed.nef", 3688, &[pin(6768, 0xbce4a1c6, 138379, 51790, 7, 204, 0)]),
    ("cameraraw/olympus-e-10-16bit-big-endian.orf", 6424, &[pin(600, 0xf9c00219, 29612, 18376, 4, 7, 0)]),
    ("cameraraw/olympus-e-420-16bit.orf", 14496, &[pin(600, 0xb9bec310, 29184, 16611, 5, 18, 0)]),
    // A restart marker every few MCUs: 3,749 of them.
    ("cameraraw/olympus-e-420-16bit.orf", 24576, &[pin(60000, 0x9f91f3ef, 2274674, 1241306, 13228, 1039, 3749)]),
    ("cameraraw/panasonic-dmc-lx7-1x1.rw2", 1536, &[pin(115200, 0x45373f77, 4521669, 2706422, 5, 1239, 0)]),
    ("cameraraw/panasonic-dmc-lx7-1x1.rw2", 13312, &[pin(600, 0xb4955140, 26807, 13884, 5, 29, 0)]),
    ("cameraraw/sony-ilce-7s-14bit-compressed.arw", 38516, &[pin(600, 0xdc7c898b, 41044, 25036, 0, 57, 0)]),
    ("cameraraw/sony-ilce-7s-14bit-compressed.arw", 144034, &[pin(54540, 0x2c95092c, 3587798, 2254693, 5, 3215, 0)]),
    ("psd/psd-tools-2layers.psb", 16584, &[pin(168, 0xc1f32ff5, 8469, 4282, 9, 20, 3)]),
    ("psd/psd-tools-4x4-8bit-duotone.psd", 18460, &[pin(6, 0xebfc1cf5, 168, 60, 4, 3, 0)]),
    ("macarchive/stuffit7-sit5-arsenic.sit", 701, &[pin(1, 0xb7837466, 190, 238, 4, 0, 0)]),
    ("pptx/powerpoint16-one-slide.pptx", 25700, &[pin(864, 0x07c33020, 8886, 3138, 0, 3, 0)]),
];

/// Lossless JPEGs inside camera raw files, which are refused by name.
const EMBEDDED_LOSSLESS: &[(&str, usize)] = &[
    ("cameraraw/canon-eos-40d-sraw2.cr2", 1353650),
    ("cameraraw/canon-eos-5d-mark-iii-mlv-app-14bit.dng", 1184),
];

/// The files baseline does not cover, and the word each scan is refused with.
const REFUSED: &[(&str, &str)] = &[
    ("jpeg/libjpeg-turbo-monkey12-12bit-icc.jpg", "12-bit"),
    ("jpeg/libjpeg-turbo-testimgari-arithmetic.jpg", "arithmetic"),
    ("jpeg/pillow-progressive.jpg", "progressive"),
];

fn read(file: &str) -> Option<Vec<u8>> {
    let root = qubero_samples::root()?;
    std::fs::read(root.join(file)).ok()
}

/// The path of every scan's entropy-coded run: segment, body, and the third
/// field of the scan.
fn scans(d: &Document<MemSource>, ev: &mut Evaluator) -> Vec<Vec<usize>> {
    let n = ev.node(d, &[1]).unwrap().child_count as usize;
    (0..n)
        .filter(|&i| ev.node(d, &[1, i, 0]).unwrap().value.as_int() == Some(0xffda))
        .map(|i| vec![1, i, 1, 2])
        .collect()
}

#[test]
fn every_baseline_scan_decodes_to_the_reference_coefficients_and_every_bit_is_accounted_for() {
    let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/jpeg");
    for &(file, pins) in FIXTURES {
        let bytes = std::fs::read(fixtures.join(file)).unwrap_or_else(|e| panic!("{file}: {e}"));
        check_file(file, bytes, pins);
    }
    if qubero_samples::root().is_none() {
        eprintln!("{}", qubero_samples::missing());
        return;
    }
    for &(file, pins) in BASELINE {
        match read(file) {
            Some(bytes) => check_file(file, bytes, pins),
            None => eprintln!("skipped: no {file} in the sample collection"),
        }
    }
}

#[test]
fn every_baseline_jpeg_inside_another_sample_decodes_to_the_reference_coefficients() {
    if qubero_samples::root().is_none() {
        eprintln!("{}", qubero_samples::missing());
        return;
    }
    for &(file, at, pins) in EMBEDDED {
        match read(file) {
            Some(bytes) => check_file(&format!("{file} at {at}"), bytes[at..].to_vec(), pins),
            None => eprintln!("skipped: no {file} in the sample collection"),
        }
    }
    for &(file, at) in EMBEDDED_LOSSLESS {
        let Some(bytes) = read(file) else { continue };
        let d = Document::new(MemSource(bytes[at..].to_vec()));
        let mut ev = Evaluator::new(jpeg());
        for path in scans(&d, &mut ev) {
            assert_eq!(ev.node(&d, &path).unwrap().refused.as_deref(), Some("lossless"), "{file} at {at}");
        }
    }
}

/// A photograph of 3,840 by 2,160 at 4:4:4, when the machine has the one
/// Pop!_OS ships: 388,800 blocks, which is 50 MB of coefficients, under the
/// 64 MiB a stream may come to, and more codes than a trace names one by one.
/// The coefficients are still all the reference's, and the trace still tiles
/// and still accounts for every bit.
///
/// Left out of the ordinary run, since a debug build takes a minute and a
/// half over it:
///
/// ```text
/// cargo test -p qubero-core --test jpeg_scan_real -- --ignored
/// ```
#[test]
#[ignore]
fn a_four_k_photograph_decodes_whole() {
    const WALLPAPER: &str = "/usr/share/backgrounds/cosmic/webb-inspired-wallpaper-system76.jpg";
    let Ok(bytes) = std::fs::read(WALLPAPER) else {
        eprintln!("skipped: no {WALLPAPER}");
        return;
    };
    let d = Document::new(MemSource(bytes));
    let mut ev = Evaluator::new(jpeg());
    let path = scans(&d, &mut ev).remove(0);
    let run = ev.node(&d, &path).unwrap();
    let id = ev.open_space(&d, 0, &path).unwrap().expect("the scan opens");
    let space = ev.space(id).unwrap();
    assert_eq!(space.bytes().len(), 388_800 * 128);
    assert_eq!(crc32(space.bytes()), 0x9d77343c);
    let trace = space.trace();
    trace.check_tiles().unwrap();
    assert_eq!(trace.in_bits(), run.size_bits);
    assert_eq!(trace.units().len(), 388_800);
    assert_eq!(trace.stuffed().len(), 10_820);
    assert!(trace.coarse(), "{} steps, and every code still named", trace.len());
    eprintln!("{WALLPAPER}: {} steps", trace.len());
    // Past the limit a block is one step, and the listing says its codes
    // were not named rather than calling them something else.
    let last = trace.blocks().len() - 1;
    let block = ev.node(&d, &[path.as_slice(), &[1, last, 0]].concat()).unwrap();
    assert_eq!(block.child_count, 1);
    let codes = ev.node(&d, &[path.as_slice(), &[1, last, 0, 0]].concat()).unwrap();
    assert_eq!(codes.name, "unnamed codes");
    assert_eq!(codes.size_bits, block.size_bits);
}

/// Every scan of one file against what the reference says of it.
fn check_file(file: &str, bytes: Vec<u8>, pins: &[Pinned]) {
    let d = Document::new(MemSource(bytes));
    let mut ev = Evaluator::new(jpeg());
    let runs = scans(&d, &mut ev);
    assert_eq!(runs.len(), pins.len(), "{file}: scans");
    for (path, want) in runs.iter().zip(pins) {
        check_scan(&d, &mut ev, &format!("{file} {path:?}"), path, want);
    }
}

fn check_scan(d: &Document<MemSource>, ev: &mut Evaluator, what: &str, path: &[usize], want: &Pinned) {
    let run = ev.node(d, path).unwrap();
    assert!(run.refused.is_none(), "{what}: refused {:?}", run.refused);
    let id = ev.open_space(d, 0, path).unwrap().unwrap_or_else(|| panic!("{what}: the scan did not open"));
    let space = ev.space(id).unwrap();
    let out = space.bytes();
    assert_eq!(out.len() as u64, want.blocks * 128, "{what}: blocks");
    assert_eq!(crc32(out), want.crc, "{what}: the coefficients differ from the reference's");

    let trace = space.trace();
    trace.check_tiles().unwrap_or_else(|e| panic!("{what}: {e}"));
    assert_eq!(trace.in_bits(), run.size_bits, "{what}: the trace does not cover the run");
    assert_eq!(trace.units().len() as u64, want.blocks, "{what}");
    assert_eq!(trace.stuffed().len() as u64, want.stuffed, "{what}: stuffed bytes");

    // Where the bits went, step by step, and the stuffed zeros inside the
    // steps that straddle them.
    let (mut code, mut value, mut padding, mut markers, mut other) = (0u64, 0u64, 0u64, 0u64, 0u64);
    for s in trace.steps() {
        let width = s.in_bits.end - s.in_bits.start;
        let inside = trace.stuffed_in(s.in_bits.clone()) as u64 * 8;
        match s.kind {
            StepKind::Dc { code: c, size, .. } | StepKind::Ac { code: c, size, .. } => {
                assert_eq!(width, c as u64 + size as u64 + inside, "{what}: {s:?}");
                code += c as u64;
                value += size as u64;
            }
            StepKind::Zrl { code: c, .. } | StepKind::Eob { code: c, .. } => {
                assert_eq!(width, c as u64 + inside, "{what}: {s:?}");
                code += c as u64;
            }
            StepKind::Header(StepField::Padding, _) => padding += width - inside,
            StepKind::Header(StepField::Restart, _) => {
                assert_eq!(width, 16);
                markers += 1;
            }
            _ => other += width,
        }
    }
    assert_eq!((code, value, padding, markers, other), (want.code_bits, want.value_bits, want.padding_bits, want.markers, 0), "{what}");
    assert_eq!(code + value + padding + 16 * markers + 8 * want.stuffed, run.size_bits, "{what}: the bits do not add up");
    // Every MCU's blocks, and every block closing on the step that carries
    // its 128 bytes.
    let f = trace.jpeg().expect("a JPEG trace says what it settled");
    assert_eq!(trace.blocks().len() as u64, f.mcus_across as u64 * f.mcus_down as u64, "{what}");
    for u in trace.units() {
        let last = trace.step(u.steps.end as usize - 1).unwrap();
        assert_eq!(last.out_bytes.end - last.out_bytes.start, 128, "{what}: {u:?}");
    }
    eprintln!("{what}: {} blocks, {} MCUs, {} steps, matches the reference", want.blocks, trace.blocks().len(), trace.len());
}

#[test]
fn what_baseline_does_not_cover_stays_bytes_and_says_why() {
    if qubero_samples::root().is_none() {
        eprintln!("{}", qubero_samples::missing());
        return;
    }
    for &(file, why) in REFUSED {
        let Some(bytes) = read(file) else {
            eprintln!("skipped: no {file} in the sample collection");
            continue;
        };
        let d = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(jpeg());
        let runs = scans(&d, &mut ev);
        assert!(!runs.is_empty(), "{file}: no scan");
        for path in runs {
            let run = ev.node(&d, &path).unwrap();
            assert_eq!(run.refused.as_deref(), Some(why), "{file}: {path:?}");
            assert_eq!(run.child_count, 0, "{file}: a refused scan has nothing inside");
            assert!(run.size_bits > 0, "{file}: the run is still its bytes");
            assert_eq!(ev.open_space(&d, 0, &path).unwrap(), None, "{file}: opened anyway");
        }
    }
}

/// The listing over one scan: MCUs, then 8×8 blocks, then codes, each with
/// its place in the file.
#[test]
fn the_listing_goes_from_mcus_to_blocks_to_codes() {
    let Some(bytes) = read("jpeg/libjpeg-turbo-testorig-baseline.jpg") else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    let d = Document::new(MemSource(bytes.clone()));
    let mut ev = Evaluator::new(jpeg());
    let path = scans(&d, &mut ev).remove(0);
    let run = ev.node(&d, &path).unwrap();
    assert_eq!(run.child_count, 2, "the coefficients, and the MCUs they came from");
    let at = |tail: &[usize]| [path.as_slice(), tail].concat();
    let mcus = ev.node(&d, &at(&[1])).unwrap();
    assert_eq!(mcus.child_count, 150, "15 MCUs across and 10 down");
    assert_eq!(mcus.unit.as_deref(), Some("MCU"));
    assert_eq!(mcus.offset_bits, run.offset_bits);
    let mcu = ev.node(&d, &at(&[1, 17])).unwrap();
    assert_eq!(mcu.name, "MCU 2, 1");
    // Four Y blocks, a Cb and a Cr.
    assert_eq!(mcu.child_count, 6);
    let names: Vec<String> = (0..6).map(|i| ev.node(&d, &at(&[1, 17, i])).unwrap().name).collect();
    assert_eq!(names, ["Y block 4, 2", "Y block 5, 2", "Y block 4, 3", "Y block 5, 3", "Cb block 2, 1", "Cr block 2, 1"]);
    // A block's first code is its DC, and it starts where the block does.
    let block = ev.node(&d, &at(&[1, 17, 0])).unwrap();
    let dc = ev.node(&d, &at(&[1, 17, 0, 0])).unwrap();
    assert!(dc.name.starts_with("DC diff "), "{}", dc.name);
    assert_eq!(dc.offset_bits, block.offset_bits);
    // Its value is its bits, read out of the file.
    let Value::Str(bits) = &dc.value else { panic!("{:?}", dc.value) };
    assert_eq!(bits.len() as u64, dc.size_bits);
    let file_bits: String = (dc.offset_bits..dc.offset_bits + dc.size_bits)
        .map(|b| if bytes[(b / 8) as usize] >> (7 - b % 8) & 1 == 1 { '1' } else { '0' })
        .collect();
    assert_eq!(*bits, file_bits);
    // The children of a block follow one another and fill it.
    let mut at_bit = block.offset_bits;
    for i in 0..block.child_count as usize {
        let c = ev.node(&d, &at(&[1, 17, 0, i])).unwrap();
        assert_eq!(c.offset_bits, at_bit, "{}", c.name);
        at_bit += c.size_bits;
    }
    assert_eq!(at_bit, block.offset_bits + block.size_bits);
    let last = ev.node(&d, &at(&[1, 17, 0, block.child_count as usize - 1])).unwrap();
    assert!(last.name.starts_with("EOB") || last.name.contains("zigzag position 63"), "{}", last.name);
    // The last MCU ends with the padding, which is ones.
    let end = ev.node(&d, &at(&[1, 149])).unwrap();
    let pad = ev.node(&d, &at(&[1, 149, end.child_count as usize - 1])).unwrap();
    assert_eq!(pad.name, "padding");
    assert_eq!(pad.size_bits, 6);
    assert_eq!(pad.offset_bits + pad.size_bits, run.offset_bits + run.size_bits);

    // A code names the table it was read with, and that is the table's own
    // record in a DHT segment before the scan. Y block codes read with the
    // tables the scan header gives Y, which in this file are the ones with
    // id 0.
    let origins = ev.origins(&d, &at(&[1, 17, 0, 0])).unwrap();
    let dc = origins.iter().find(|o| o.label == "DC table 0").unwrap_or_else(|| panic!("no DC table in {origins:?}"));
    assert_eq!(ev.node(&d, &dc.path).unwrap().type_name, "HuffmanTable");
    let class = ev.node(&d, &[dc.path.as_slice(), &[0]].concat()).unwrap();
    assert_eq!(class.value.as_int(), Some(0), "a DC table is class 0");
    assert!(dc.path[1] < path[1], "the table is in a segment before the scan");
    let cr_eob = {
        let cr = ev.node(&d, &at(&[1, 17, 5])).unwrap();
        at(&[1, 17, 5, cr.child_count as usize - 1])
    };
    let origins = ev.origins(&d, &cr_eob).unwrap();
    assert!(origins.iter().any(|o| o.label == "AC table 1"), "a Cr code reads with AC table 1: {origins:?}");

    // Every byte of the coefficients maps back to the code that closed its
    // block, and that code's bits lead back to it.
    let id = ev.open_space(&d, 0, &path).unwrap().unwrap();
    let space = ev.space(id).unwrap();
    for byte in [0u64, 127, 128, 1000, 900 * 128 - 1] {
        let step = space.map_out(byte).unwrap_or_else(|| panic!("byte {byte} came from nowhere"));
        assert!(step.out_bytes.contains(&byte));
        assert_eq!(step.out_bytes.end - step.out_bytes.start, 128);
        assert!(!step.in_bits.is_empty());
        assert_eq!(space.map_in(run.offset_bits + step.in_bits.start), Some(step));
    }
}

/// What the report's figures are drawn from: the costs of every MCU and block
/// read off the trace in one pass, and one block in full.
#[test]
fn a_scans_costs_and_one_block_come_off_the_trace() {
    let Some(bytes) = read("jpeg/libjpeg-turbo-testorig-baseline.jpg") else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    let d = Document::new(MemSource(bytes.clone()));
    let mut ev = Evaluator::new(jpeg());
    let path = scans(&d, &mut ev).remove(0);
    let run = ev.node(&d, &path).unwrap();
    let map = ev.jpeg_scan(&d, &path).unwrap().expect("a JPEG scan");
    assert_eq!((map.mcus_across, map.mcus_down), (15, 10));
    assert_eq!(map.run_offset_bits, run.offset_bits);
    assert_eq!(map.run_bits, run.size_bits);
    assert_eq!(map.mcu_bits.iter().map(|&b| b as u64).sum::<u64>(), run.size_bits, "the MCUs cover the run");
    assert_eq!(map.blocks.len(), 900);
    let names: Vec<&str> = map.channels.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["Y", "Cb", "Cr"]);
    // The totals are the reference's, and add up to the run.
    let t = map.totals;
    assert_eq!(t.dc_code + t.ac_code + t.eob + t.zrl, 26677);
    assert_eq!(t.dc_value + t.ac_value, 14413);
    assert_eq!((t.padding, t.stuffed, t.markers, t.unnamed), (6, 64, 0, 0));
    assert_eq!(t.dc_code + t.dc_value + t.ac_code + t.ac_value + t.eob + t.zrl + t.padding + t.stuffed + t.markers + t.unnamed, run.size_bits);
    // IJG quality 75 halves Annex K's first luminance step, 16, to 8, and
    // the chrominance one, 17, to 9.
    assert_eq!(map.channels[0].quant.as_ref().map(|q| q[0]), Some(8));
    assert_eq!(map.channels[1].quant.as_ref().map(|q| q[0]), Some(9));

    // One block in full: its codes' bits end to end are the block's bits,
    // and its coefficients are the ones in the decoded space.
    let block = ev.jpeg_block(&d, &path, 102).unwrap().expect("a block");
    let cost = map.blocks[102];
    assert_eq!((block.channel, block.x, block.y), (cost.channel, cost.x, cost.y));
    assert_eq!(block.codes.len(), cost.codes as usize);
    assert_eq!(block.codes.first().map(|c| c.kind), Some("dc"));
    let mut at = block.codes[0].start_bit;
    for c in &block.codes {
        assert_eq!(c.start_bit, at);
        assert_eq!(c.bits.len(), (c.code_bits + c.value_bits) as usize, "{c:?}");
        at = c.end_bit;
    }
    assert_eq!((at - block.codes[0].start_bit) as u32, cost.bits);
    assert_eq!(block.coefficients.len(), 64);
    assert_eq!(block.coefficients[0], block.codes[0].dc);
    let node = ev.node(&d, &block.path).unwrap();
    assert_eq!(node.name, format!("Y block {}, {}", block.x, block.y));
    assert_eq!(node.offset_bits, block.codes[0].start_bit);
    assert_eq!(ev.node(&d, &block.mcu_path).unwrap().name, format!("MCU {}, {}", block.mcu % 15, block.mcu / 15));
}

/// A code that straddles a stuffed zero is as wide as the bits it covers, and
/// its value leaves the zero out.
#[test]
fn a_code_over_a_stuffed_byte_reads_without_it() {
    let Some(bytes) = read("jpeg/libjpeg-turbo-testorig-baseline.jpg") else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    let d = Document::new(MemSource(bytes.clone()));
    let mut ev = Evaluator::new(jpeg());
    let path = scans(&d, &mut ev).remove(0);
    let run = ev.node(&d, &path).unwrap();
    let id = ev.open_space(&d, 0, &path).unwrap().unwrap();
    let trace = ev.space(id).unwrap().trace().clone();
    let mut seen = 0;
    for (k, s) in trace.steps().enumerate() {
        if trace.stuffed_in(s.in_bits.clone()) == 0 || s.kind.code_bits().is_none() {
            continue;
        }
        // Which MCU, which block, which code.
        let mcu = trace.blocks().iter().position(|b| b.steps.contains(&(k as u32))).unwrap();
        let first = trace.units_of(mcu).start;
        let unit = trace.units().iter().position(|u| u.steps.contains(&(k as u32))).unwrap();
        let code = k - trace.units()[unit].steps.start as usize;
        let node = ev.node(&d, &[path.as_slice(), &[1, mcu, unit - first, code]].concat()).unwrap();
        assert_eq!(node.offset_bits, run.offset_bits + s.in_bits.start);
        assert_eq!(node.size_bits, s.in_bits.end - s.in_bits.start);
        assert!(node.type_name.ends_with("with a stuffed 00 byte"), "{}", node.type_name);
        let Value::Str(bits) = &node.value else { panic!() };
        assert_eq!(bits.len() as u64, node.size_bits - 8);
        seen += 1;
    }
    assert!(seen > 0, "no code in the scan covers a stuffed byte");
}
