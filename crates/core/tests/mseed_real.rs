//! Real miniSEED records: the two byte orders read the same differences, and
//! the calibration blockettes read what their fixtures put in them.
//!
//! The first of those is the point. A big-endian Steim frame is cut into bit
//! fields where it lies; a little-endian one is read as words and the fields
//! worked out from their values. obspy writes the same samples both ways, so
//! the two files must give the same differences word for word, and any bit
//! named wrongly shows up as a number that does not match.

use qubero_core::document::Document;
use qubero_core::eval::{Evaluator, Value};
use qubero_core::formats;
use qubero_core::source::MemSource;

/// The record, its blockette array, and its data, as field indices.
const BLOCKETTES: usize = 18;
const DATA: usize = 20;

fn samples() -> Option<std::path::PathBuf> {
    let root = match std::env::var_os("QUBERO_SAMPLES") {
        Some(p) => std::path::PathBuf::from(p),
        None => std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../qubero-samples"),
    };
    root.join("seismic").is_dir().then_some(root)
}

fn open(root: &std::path::Path, name: &str) -> (Document<MemSource>, Evaluator) {
    let bytes = std::fs::read(root.join("seismic").join(name)).unwrap();
    (Document::new(MemSource(bytes)), Evaluator::new(formats::builtin("mseed").unwrap()))
}

/// Every number under the frames of one record, in the order they are written,
/// skipping the words that are only words: what is wanted is the differences
/// the codes named.
fn differences(d: &Document<MemSource>, ev: &mut Evaluator, frame: usize) -> Vec<i128> {
    let mut out = Vec::new();
    let base = [0, 0, DATA, frame];
    let words = ev.node(d, &base).unwrap().child_count;
    for w in 0..words as usize {
        let mut p = base.to_vec();
        p.push(w);
        collect(d, ev, &p, &mut out);
    }
    out
}

/// Walk down to the leaves. A big-endian difference is a leaf of the word; a
/// little-endian one is a computed field beside it. Either way it is a number
/// with no bytes under it, and the word itself is skipped: it is the same
/// thirty-two bits read a second time.
fn collect(d: &Document<MemSource>, ev: &mut Evaluator, path: &[usize], out: &mut Vec<i128>) {
    let node = ev.node(d, path).unwrap();
    if node.child_count == 0 {
        // The whole word, or a difference. A word is 32 bits and named `word`
        // or `dnib`; a difference of 32 bits is named `d0`.
        if node.name.starts_with('d') && node.name != "dnib" {
            out.push(node.value.as_int().unwrap());
        }
        return;
    }
    for i in 0..node.child_count as usize {
        let mut p = path.to_vec();
        p.push(i);
        collect(d, ev, &p, out);
    }
}

#[test]
fn a_steim2_record_gives_the_same_differences_whichever_way_round_it_is_written() {
    let Some(root) = samples() else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    let (db, mut evb) = open(&root, "int32_Steim2_bigEndian.mseed");
    let (dl, mut evl) = open(&root, "int32_Steim2_littleEndian.mseed");
    let big = differences(&db, &mut evb, 0);
    let little = differences(&dl, &mut evl, 0);
    assert!(!big.is_empty(), "no differences found at all");
    assert_eq!(big, little, "the two byte orders disagree");

    // The integration constants are plain words either way round, so they are
    // the check that the two frames line up before the differences are read.
    let c = |ev: &mut Evaluator, d: &Document<MemSource>, i| ev.node(d, &[0, 0, DATA, 0, i]).unwrap().value.as_int();
    assert_eq!(c(&mut evb, &db, 1), Some(1));
    assert_eq!(c(&mut evl, &dl, 1), Some(1));
    assert_eq!(c(&mut evb, &db, 2), c(&mut evl, &dl, 2));

    // Not every slot a code names holds a sample. Steim2 packs seven
    // differences into a word whether or not the record has seven left, so the
    // frame shows more differences than the header counts samples, and it is
    // the sample count that says where to stop. Showing the slots is right;
    // adding all of them up is not.
    assert!(big.len() > 50, "a frame of seven-wide words holds more slots than this record's samples");
}

/// Steim1, the other compressor, on the same ramp. The big-endian file is what
/// is checked against: its differences add up from the first sample to the
/// last.
///
/// Its little-endian twin is not compared word for word. One 32-bit word of
/// that fixture's frame appears to have been left unswapped, so the two files
/// do not hold the same frame; obspy uses both only to test what a record
/// header says and never reads their samples, so nothing there would have
/// caught it. The Steim2 pair above is the clean cross-check.
#[test]
fn a_steim1_record_adds_its_differences_up_to_the_sample_it_ends_on() {
    let Some(root) = samples() else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    let (d, mut ev) = open(&root, "int32_Steim1_bigEndian.mseed");
    let diffs = differences(&d, &mut ev, 0);
    let x0 = ev.node(&d, &[0, 0, DATA, 0, 1]).unwrap().value.as_int().unwrap();
    let xn = ev.node(&d, &[0, 0, DATA, 0, 2]).unwrap().value.as_int().unwrap();
    assert_eq!((x0, xn), (1, 50));
    assert_eq!(diffs.iter().sum::<i128>() + x0, xn);

    // The little-endian file reads, and its words come out the right shape:
    // four 8-bit differences to a word, which is what its code word says.
    let (dl, mut evl) = open(&root, "int32_Steim1_littleEndian.mseed");
    let little = differences(&dl, &mut evl, 0);
    assert_eq!(little.len(), diffs.len());
}

/// The three calibration blockettes, each from the fixture obspy keeps for it.
/// What is checked is that the body is placed and named, and that the fields
/// after the ten-byte time land where they should.
#[test]
fn the_calibration_blockettes_read_from_their_own_fixtures() {
    let Some(root) = samples() else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    // The last of the three is which field of that body is the input channel:
    // the calibration blockettes carry the same fields in different orders.
    for (file, want, channel) in [
        ("blockette300.mseed", "StepCalibration", 6),
        ("blockette310.mseed", "SineCalibration", 6),
        ("blockette320.mseed", "PseudoRandomCalibration", 5),
    ] {
        let (d, mut ev) = open(&root, file);
        let n = ev.node(&d, &[0, 0, BLOCKETTES]).unwrap().child_count;
        let found = (0..n as usize)
            .map(|i| ev.node(&d, &[0, 0, BLOCKETTES, i, 2]).unwrap().type_name)
            .collect::<Vec<_>>();
        assert!(found.iter().any(|t| t == want), "{file}: no {want} among {found:?}");
        let i = found.iter().position(|t| t == want).unwrap();
        // The time is the first field of every one of them, and it is the year
        // the record was recorded in.
        let year = ev.node(&d, &[0, 0, BLOCKETTES, i, 2, 0, 0]).unwrap().value.as_int().unwrap();
        assert!((1800..=2100).contains(&year), "{file}: calibration year {year}");
        // The input channel is three characters of a real channel name.
        let ch = ev.node(&d, &[0, 0, BLOCKETTES, i, 2, channel]).unwrap().value;
        let Value::Str(ch) = ch else { panic!("{file}: input channel is not text") };
        assert!(ch.chars().all(|c| c.is_ascii_alphanumeric()), "{file}: input channel {ch:?}");
    }
}

/// The gain-ranged encodings, which are read at their sample width and no
/// further. What matters is that the count is the count the header gave.
#[test]
fn a_gain_ranged_record_holds_as_many_words_as_it_says() {
    let Some(root) = samples() else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    for file in ["CDSN_encoding.mseed", "DWWSSN_encoding.mseed", "GEOSCOPE16_4_encoding.mseed"] {
        let (d, mut ev) = open(&root, file);
        let count = ev.node(&d, &[0, 0, 8]).unwrap().value.as_int().unwrap();
        let words = ev.node(&d, &[0, 0, DATA, 0]).unwrap();
        assert_eq!(words.child_count as i128, count, "{file}");
        assert_eq!(words.size_bits, count as u64 * 16, "{file}: two bytes a word");
    }
}
