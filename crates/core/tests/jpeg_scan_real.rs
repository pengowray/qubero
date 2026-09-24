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
    file: &'static str,
    blocks: u64,
    crc: u32,
    code_bits: u64,
    value_bits: u64,
    padding_bits: u64,
    stuffed: u64,
    markers: u64,
}

const BASELINE: &[Pinned] = &[
    // 227 by 149, 4:2:0, the IJG's own test image, as in the hand-written report.
    Pinned { file: "jpeg/libjpeg-turbo-testorig-baseline.jpg", blocks: 900, crc: 0x01281baf, code_bits: 26677, value_bits: 14413, padding_bits: 6, stuffed: 8, markers: 0 },
    // Four channels, CMYK by the Adobe segment, with a restart every MCU row.
    Pinned { file: "jpeg/pillow-cmyk-adobe-restart.jpg", blocks: 384, crc: 0x9e970d53, code_bits: 17367, value_bits: 10614, padding_bits: 27, stuffed: 56, markers: 7 },
    // One channel, so one block per MCU.
    Pinned { file: "jpeg/pillow-grey-comment.jpg", blocks: 96, crc: 0xa1cc9b70, code_bits: 5280, value_bits: 3243, padding_bits: 5, stuffed: 9, markers: 0 },
    // 4:4:4: three blocks per MCU, one of each channel.
    Pinned { file: "jpeg/pillow-ycc444-exif.jpg", blocks: 288, crc: 0x8e5f5c6a, code_bits: 12462, value_bits: 7624, padding_bits: 2, stuffed: 26, markers: 0 },
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
    if qubero_samples::root().is_none() {
        eprintln!("{}", qubero_samples::missing());
        return;
    }
    for want in BASELINE {
        let Some(bytes) = read(want.file) else {
            eprintln!("skipped: no {} in the sample collection", want.file);
            continue;
        };
        let d = Document::new(MemSource(bytes.clone()));
        let mut ev = Evaluator::new(jpeg());
        let runs = scans(&d, &mut ev);
        assert_eq!(runs.len(), 1, "{}: one scan", want.file);
        let path = &runs[0];
        let run = ev.node(&d, path).unwrap();
        assert!(run.refused.is_none(), "{}: refused {:?}", want.file, run.refused);
        let id = ev.open_space(&d, 0, path).unwrap().unwrap_or_else(|| panic!("{}: the scan did not open", want.file));
        let space = ev.space(id).unwrap();
        let out = space.bytes();
        assert_eq!(out.len() as u64, want.blocks * 128, "{}: blocks", want.file);
        assert_eq!(crc32(out), want.crc, "{}: the coefficients differ from the reference's", want.file);

        let trace = space.trace();
        trace.check_tiles().unwrap_or_else(|e| panic!("{}: {e}", want.file));
        assert_eq!(trace.in_bits(), run.size_bits, "{}: the trace does not cover the run", want.file);
        assert_eq!(trace.units().len() as u64, want.blocks, "{}", want.file);
        assert_eq!(trace.stuffed().len() as u64, want.stuffed, "{}: stuffed bytes", want.file);

        // Where the bits went, step by step, and the stuffed zeros inside the
        // steps that straddle them.
        let (mut code, mut value, mut padding, mut markers, mut other) = (0u64, 0u64, 0u64, 0u64, 0u64);
        for s in trace.steps() {
            let width = s.in_bits.end - s.in_bits.start;
            let inside = trace.stuffed_in(s.in_bits.clone()) as u64 * 8;
            match s.kind {
                StepKind::Dc { code: c, size, .. } | StepKind::Ac { code: c, size, .. } => {
                    assert_eq!(width, c as u64 + size as u64 + inside, "{}: {s:?}", want.file);
                    code += c as u64;
                    value += size as u64;
                }
                StepKind::Zrl { code: c, .. } | StepKind::Eob { code: c, .. } => {
                    assert_eq!(width, c as u64 + inside, "{}: {s:?}", want.file);
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
        assert_eq!((code, value, padding, markers, other), (want.code_bits, want.value_bits, want.padding_bits, want.markers, 0), "{}", want.file);
        assert_eq!(code + value + padding + 16 * markers + 8 * want.stuffed, run.size_bits, "{}: the bits do not add up", want.file);
        // Every MCU's blocks, and every block closing on the step that carries
        // its 128 bytes.
        let f = trace.jpeg().expect("a JPEG trace says what it settled");
        assert_eq!(trace.blocks().len() as u64, f.mcus_across as u64 * f.mcus_down as u64, "{}", want.file);
        for u in trace.units() {
            let last = trace.step(u.steps.end as usize - 1).unwrap();
            assert_eq!(last.out_bytes.end - last.out_bytes.start, 128, "{}: {u:?}", want.file);
        }
        eprintln!("{}: {} blocks, {} MCUs, {} steps, matches the reference", want.file, want.blocks, trace.blocks().len(), trace.len());
    }
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
