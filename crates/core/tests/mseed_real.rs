//! Real miniSEED records: the two byte orders read the same differences, and
//! the calibration blockettes read what their fixtures put in them.
//!
//! The first of those is the point. A big-endian Steim frame is cut into bit
//! fields where it lies; a little-endian one is read as words and the fields
//! worked out from their values. obspy writes the same samples both ways, so
//! the two files must give the same differences word for word, and any bit
//! named wrongly shows up as a number that does not match.
//!
//! And, at the end, miniSEED 3, which is a different format under the same
//! name. Its reference files are libmseed's own, and what is checked of them
//! is the thing the format changed: a record's length is its three lengths
//! added to a forty-byte header and nothing is rounded up, so the records
//! have to tile the file exactly. obspy cannot read these, its reader being
//! the 2.x one, so the numbers below come from the headers themselves as the
//! specification lays them out.
//!
//! And the samples, as `mseed_steim` works them out of the bytes the template
//! placed. Every 2.4 record is checked against what obspy reads from the same
//! record, every Steim record against its own reverse integration constant,
//! and the miniSEED 3 series against obspy's reading of the 2.4 file libmseed
//! wrote the same series into.

use qubero_core::document::Document;
use qubero_core::eval::{Evaluator, Value};
use qubero_core::formats;
use qubero_core::formats::mseed_steim;
use qubero_core::source::MemSource;

/// The record, its blockette array, and its data, as field indices.
const BLOCKETTES: usize = 18;
const DATA: usize = 20;

/// The same for a miniSEED 3 record, whose fields are in a different order and
/// whose data is not behind a blockette.
mod ms3 {
    pub const ENCODING: usize = 4;
    pub const SAMPLE_COUNT: usize = 6;
    pub const IDENTIFIER_LENGTH: usize = 9;
    pub const EXTRA_LENGTH: usize = 10;
    pub const DATA_LENGTH: usize = 11;
    pub const SOURCE_IDENTIFIER: usize = 12;
    pub const EXTRA_HEADERS: usize = 13;
    pub const DATA: usize = 14;
}

fn samples() -> Option<std::path::PathBuf> {
    let root = match std::env::var_os("QUBERO_SAMPLES") {
        Some(p) => std::path::PathBuf::from(p),
        None => std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../qubero-samples"),
    };
    root.join("seismic").is_dir().then_some(root)
}

fn open(root: &std::path::Path, name: &str) -> (Document<MemSource>, Evaluator) {
    open_as(root, name, "mseed")
}

fn open_as(root: &std::path::Path, name: &str, template: &str) -> (Document<MemSource>, Evaluator) {
    let bytes = std::fs::read(root.join("seismic").join(name)).unwrap();
    (Document::new(MemSource(bytes)), Evaluator::new(formats::builtin(template).unwrap()))
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

/// The two byte orders read the same differences, for both compressors.
///
/// A word that holds whole differences is not swapped as a word: only what is
/// inside it is. A word that packs bit fields is swapped whole and then cut
/// up. Getting either of those the wrong way round shows here as two files
/// that were written from the same samples and do not agree.
#[test]
fn a_steim_record_gives_the_same_differences_whichever_way_round_it_is_written() {
    let Some(root) = samples() else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    for kind in ["Steim1", "Steim2"] {
        let (db, mut evb) = open(&root, &format!("int32_{kind}_bigEndian.mseed"));
        let (dl, mut evl) = open(&root, &format!("int32_{kind}_littleEndian.mseed"));
        let big = differences(&db, &mut evb, 0);
        let little = differences(&dl, &mut evl, 0);
        assert!(!big.is_empty(), "{kind}: no differences found at all");
        assert_eq!(big, little, "{kind}: the two byte orders disagree");

        // The integration constants are plain words either way round, and are
        // the check that the two frames line up before the differences do.
        let c =
            |ev: &mut Evaluator, d: &Document<MemSource>, i| ev.node(d, &[0, 0, DATA, 0, i]).unwrap().value.as_int();
        assert_eq!(c(&mut evb, &db, 1), Some(1), "{kind}: x0");
        assert_eq!(c(&mut evl, &dl, 1), Some(1), "{kind}: x0");
        assert_eq!(c(&mut evb, &db, 2), Some(50), "{kind}: xn");
    }
}

/// Both files are a ramp from 1 to 50, so the differences a reader would
/// actually use add up from the first sample to the last.
///
/// Only the first `sample_count` of them are used. Steim2 packs seven
/// differences into a word whether or not the record has seven left, so a
/// frame shows more slots than the record has samples; showing them is right,
/// and the sample count is what says where to stop.
#[test]
fn the_differences_a_record_uses_add_up_to_the_sample_it_ends_on() {
    let Some(root) = samples() else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    for kind in ["Steim1", "Steim2"] {
        for order in ["bigEndian", "littleEndian"] {
            let (d, mut ev) = open(&root, &format!("int32_{kind}_{order}.mseed"));
            let diffs = differences(&d, &mut ev, 0);
            let count = ev.node(&d, &[0, 0, 8]).unwrap().value.as_int().unwrap() as usize;
            let x0 = ev.node(&d, &[0, 0, DATA, 0, 1]).unwrap().value.as_int().unwrap();
            let xn = ev.node(&d, &[0, 0, DATA, 0, 2]).unwrap().value.as_int().unwrap();
            assert_eq!((x0, xn, count), (1, 50, 50), "{kind} {order}");
            // The first difference is the dummy the format opens a record with.
            assert!(diffs.len() >= count, "{kind} {order}: {} slots for {count} samples", diffs.len());
            assert_eq!(diffs[..count].iter().sum::<i128>() + x0, xn, "{kind} {order}");
        }
    }
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

/// libmseed's three miniSEED 3 reference files, each read for the things the
/// format changed: the source identifier that replaced four space-padded
/// fields, the encoding and sample count that came up out of a blockette into
/// the fixed header, and the record length that is now the parts added up.
///
/// The records tiling the file with nothing between them is the real check. A
/// 2.4 record is a power of two and a reader that got the length wrong landed
/// on the next header anyway; a miniSEED 3 record is 507 or 511 or 512 bytes
/// as its own three lengths decide, so a reader that is a byte out reads the
/// rest of the file as rubble.
#[test]
fn a_miniseed_3_record_is_as_long_as_its_three_lengths_say() {
    let Some(root) = samples() else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    // file, records, encoding, samples in the first record, extra header bytes
    for (file, records, encoding, count, extra) in [
        ("reference-testdata-steim2.mseed3", 4, "Steim2", 247, 0),
        ("reference-testdata-float32.mseed3", 5, "32-bit floats", 113, 0),
        ("reference-testdata-nsec.mseed3", 12, "32-bit integers", 45, 273),
    ] {
        let (d, mut ev) = open_as(&root, file, "mseed3");
        let all = ev.node(&d, &[0]).unwrap();
        assert_eq!(all.child_count, records, "{file}: record count");
        let mut at = 0u64;
        for i in 0..records as usize {
            let record = ev.node(&d, &[0, i]).unwrap();
            assert_eq!(record.type_name, "MiniSEED3Record", "{file} record {i}");
            // Every record is the forty-byte header and its three lengths,
            // and the next one starts exactly where this one stops.
            let lengths = [ms3::IDENTIFIER_LENGTH, ms3::EXTRA_LENGTH, ms3::DATA_LENGTH]
                .map(|f| ev.node(&d, &[0, i, f]).unwrap().value.as_int().unwrap());
            let want = 40 + lengths.iter().sum::<i128>();
            assert_eq!(record.size_bits, want as u64 * 8, "{file} record {i}: length");
            assert_eq!(record.offset_bits, at, "{file} record {i}: placed");
            at += record.size_bits;
            // The identifier every record of these files carries, which is
            // network, station, location and the three channel codes.
            let id = ev.node(&d, &[0, i, ms3::SOURCE_IDENTIFIER]).unwrap().value;
            let Value::Str(id) = id else { panic!("{file} record {i}: no source identifier") };
            assert!(id.starts_with("FDSN:XX_TEST_"), "{file} record {i}: {id:?}");
        }
        assert_eq!(at, d.len_bits(), "{file}: the records do not fill the file");
        // The first record, against what its header says it holds.
        let first = ev.node(&d, &[0, 0, ms3::SOURCE_IDENTIFIER]).unwrap().value;
        assert_eq!(first, Value::Str("FDSN:XX_TEST__B_H_Z".into()), "{file}");
        let enc = ev.node(&d, &[0, 0, ms3::ENCODING]).unwrap().value;
        let Value::Enum { name, .. } = enc else { panic!("{file}: the encoding is not named") };
        assert_eq!(name.as_deref(), Some(encoding), "{file}");
        let samples = ev.node(&d, &[0, 0, ms3::SAMPLE_COUNT]).unwrap().value.as_int();
        assert_eq!(samples, Some(count), "{file}");
        // The extra headers, which are JSON when there are any and nothing
        // where the length is zero rather than an empty document.
        let headers = ev.node(&d, &[0, 0, ms3::EXTRA_HEADERS]).unwrap();
        assert_eq!(headers.size_bits, extra as u64 * 8, "{file}: extra headers");
        assert_eq!(headers.child_count > 0, extra > 0, "{file}: extra headers read");
        // And the data, typed by the encoding rather than left as bytes.
        let data = ev.node(&d, &[0, 0, ms3::DATA, 0]).unwrap();
        assert_ne!(data.type_name, "bytes[]", "{file}: the payload is untyped");
        if encoding == "32-bit floats" {
            assert_eq!((data.type_name.as_str(), data.child_count), ("f32 le[]", count as u64), "{file}");
        }
    }
}

/// The one place a miniSEED 3 record is not little-endian. Its header is,
/// always; a Steim payload is big-endian, because the frame was defined that
/// way in 1991 and the FDSN left it alone. Read the other way round the
/// integration constants are tens of millions instead of a sample value.
#[test]
fn a_miniseed_3_steim_payload_is_big_endian_inside_a_little_endian_record() {
    let Some(root) = samples() else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    let (d, mut ev) = open_as(&root, "reference-testdata-steim2.mseed3", "mseed3");
    // Frame 0 of the first record: its first two words are the first and last
    // samples of the record, and a byte-swapped reading of either is enormous.
    let x0 = ev.node(&d, &[0, 0, ms3::DATA, 0, 1]).unwrap().value.as_int().unwrap();
    let xn = ev.node(&d, &[0, 0, ms3::DATA, 0, 2]).unwrap().value.as_int().unwrap();
    assert!(x0.abs() < 1 << 20 && xn.abs() < 1 << 20, "read the wrong way round: {x0} then {xn}");
    // Seven 64-byte frames in 448 bytes of payload, and the last of them read.
    let steim = ev.node(&d, &[0, 0, ms3::DATA]).unwrap();
    assert_eq!(steim.type_name, "SteimData");
    let frames = ev.node(&d, &[0, 0, ms3::DATA, 1]).unwrap();
    assert_eq!(frames.child_count, 6, "one frame0 and six after it");
    assert_eq!(steim.size_bits, 448 * 8);
}

/// One record's data through `mseed_steim`, from the bytes the template placed
/// as the data, with the encoding and byte order blockette 1000 gives.
fn decode_24(file: &[u8], d: &Document<MemSource>, ev: &mut Evaluator, record: usize) -> mseed_steim::Record {
    let data = ev.node(d, &[0, record, DATA]).unwrap();
    let count = ev.node(d, &[0, record, 8]).unwrap().value.as_int().unwrap() as usize;
    let chain = ev.node(d, &[0, record, BLOCKETTES]).unwrap().child_count as usize;
    let b1000 = (0..chain)
        .find(|&i| ev.node(d, &[0, record, BLOCKETTES, i, 2]).unwrap().type_name == "DataOnlySEED")
        .expect("blockette 1000");
    let field = |ev: &mut Evaluator, f| match ev.node(d, &[0, record, BLOCKETTES, b1000, 2, f]).unwrap().value {
        Value::Enum { raw, .. } => raw,
        other => panic!("{other:?}"),
    };
    let (encoding, big) = (field(ev, 0) as u8, field(ev, 1) == 1);
    let at = (data.offset_bits / 8) as usize;
    decode_bytes(&file[at..at + (data.size_bits / 8) as usize], encoding, big, count)
}

fn decode_bytes(payload: &[u8], encoding: u8, big: bool, count: usize) -> mseed_steim::Record {
    mseed_steim::decode(payload, encoding, big, count)
}

/// What a run of samples is checked by: how many, the first two, the last,
/// their sum, and their sum weighted by position, which a sample out of place
/// changes even where the plain sum does not.
#[derive(Debug, PartialEq)]
struct Facts {
    count: usize,
    first: Vec<f64>,
    last: f64,
    sum: f64,
    weighted: f64,
}

fn facts(samples: &[f64]) -> Facts {
    let (mut sum, mut weighted) = (0.0, 0.0);
    for (i, &s) in samples.iter().enumerate() {
        sum += s;
        weighted += s * (i + 1) as f64;
    }
    Facts {
        count: samples.len(),
        first: samples.iter().take(2).copied().collect(),
        last: *samples.last().unwrap(),
        sum,
        weighted,
    }
}

/// Every record of the 2.4 files, against what obspy 1.5.1 reads from the same
/// record cut out of the file on its own: `obspy.read(record)[0].data`.
///
/// obspy decodes with libmseed, so this is a second reader of the same bytes
/// and not the same code run twice. Steim records also pass the check they
/// carry, and the Steim1 file of ten records is there because 412 samples in
/// seven frames exercise every word form Steim1 has.
#[test]
fn every_record_decodes_to_the_samples_obspy_reads() {
    let Some(root) = samples() else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    // file, record, count, first two, last, sum, weighted sum
    let table: &[(&str, usize, usize, [f64; 2], f64, f64, f64)] = &[
        ("BW.BGLD.__.EHE.D.2008.001.first_10_records", 0, 412, [-363.0, -382.0], -389.0, -165813.0, -34527084.0),
        ("BW.BGLD.__.EHE.D.2008.001.first_10_records", 1, 412, [-397.0, -387.0], -413.0, -162847.0, -33725996.0),
        ("BW.BGLD.__.EHE.D.2008.001.first_10_records", 2, 412, [-427.0, -416.0], -398.0, -162479.0, -33392311.0),
        ("BW.BGLD.__.EHE.D.2008.001.first_10_records", 3, 412, [-418.0, -428.0], -388.0, -160954.0, -33082426.0),
        ("BW.BGLD.__.EHE.D.2008.001.first_10_records", 4, 412, [-407.0, -385.0], -383.0, -163137.0, -33211033.0),
        ("BW.BGLD.__.EHE.D.2008.001.first_10_records", 5, 412, [-396.0, -399.0], -409.0, -161461.0, -33120133.0),
        ("BW.BGLD.__.EHE.D.2008.001.first_10_records", 6, 412, [-413.0, -401.0], -390.0, -161036.0, -33189418.0),
        ("BW.BGLD.__.EHE.D.2008.001.first_10_records", 7, 412, [-407.0, -440.0], -345.0, -162073.0, -33536624.0),
        ("BW.BGLD.__.EHE.D.2008.001.first_10_records", 8, 412, [-397.0, -409.0], -341.0, -161654.0, -33735713.0),
        ("BW.BGLD.__.EHE.D.2008.001.first_10_records", 9, 412, [-389.0, -428.0], -386.0, -162432.0, -34005243.0),
        ("CDSN_encoding.mseed", 0, 100, [294.0, 32.0], -124.0, 1594.0, 23889.0),
        ("DWWSSN_encoding.mseed", 0, 200, [-38.0, -38.0], -38.0, 940.0, 72656.0),
        ("blockette300.mseed", 0, 20, [-948.0, -947.0], -1610.0, -22964.0, -267572.0),
        ("blockette310.mseed", 0, 10, [-5620.0, -4278.0], -5565.0, -49108.0, -273364.0),
        ("brokenlastrecord.mseed", 0, 5980, [2787.0, 2776.0], 2863.0, 16640837.0, 49754365513.0),
        ("int32_Steim1_bigEndian.mseed", 0, 50, [1.0, 2.0], 50.0, 1275.0, 42925.0),
        ("int32_Steim1_littleEndian.mseed", 0, 50, [1.0, 2.0], 50.0, 1275.0, 42925.0),
        ("int32_Steim2_bigEndian.mseed", 0, 50, [1.0, 2.0], 50.0, 1275.0, 42925.0),
        ("int32_Steim2_littleEndian.mseed", 0, 50, [1.0, 2.0], 50.0, 1275.0, 42925.0),
        ("steim2.mseed", 0, 5980, [2787.0, 2776.0], 2863.0, 16640837.0, 49754365513.0),
        ("test.mseed", 0, 5980, [2787.0, 2776.0], 2863.0, 16640837.0, 49754365513.0),
        ("test.mseed", 1, 5967, [2870.0, 2870.0], 2853.0, 16600615.0, 49525994748.0),
    ];
    for &(file, record, count, first, last, sum, weighted) in table {
        let bytes = std::fs::read(root.join("seismic").join(file)).unwrap();
        let (d, mut ev) = open(&root, file);
        let r = decode_24(&bytes, &d, &mut ev, record);
        assert_eq!(r.problem, None, "{file} record {record}");
        assert_eq!(r.declared, count, "{file} record {record}: the header's count");
        let want = Facts { count, first: first.to_vec(), last, sum, weighted };
        assert_eq!(facts(&r.samples), want, "{file} record {record}");
        if r.steim.is_some() {
            let check = r.check().expect("a check");
            assert!(check.passed(), "{file} record {record}: last {} against reverse constant {}", check.last, check.xn);
        }
    }
    // The one sample of blockette320.mseed is its forward and reverse constant
    // both, and no difference is used at all.
    let bytes = std::fs::read(root.join("seismic").join("blockette320.mseed")).unwrap();
    let (d, mut ev) = open(&root, "blockette320.mseed");
    let r = decode_24(&bytes, &d, &mut ev, 0);
    assert_eq!(r.samples, vec![-3824.0]);
    assert!(r.check().unwrap().passed());
    assert_eq!(r.steim.unwrap().frames[0].used, 1);
    // GEOSCOPE has no documented rule, and says so rather than guessing.
    let bytes = std::fs::read(root.join("seismic").join("GEOSCOPE16_4_encoding.mseed")).unwrap();
    let (d, mut ev) = open(&root, "GEOSCOPE16_4_encoding.mseed");
    let r = decode_24(&bytes, &d, &mut ev, 0);
    assert!(r.samples.is_empty() && r.problem.is_some());
}

/// The reader takes every word apart the way the template does, in every
/// frame of the record and in both byte orders.
///
/// The reader reads bytes and the template lays out fields, and these are the
/// same rules written twice. The little-endian files are where they could
/// part: a word of whole differences swaps each difference, and a bit-packed
/// Steim2 word swaps whole, and getting either backwards would still give
/// numbers.
#[test]
fn the_reader_takes_each_word_apart_as_the_template_does() {
    let Some(root) = samples() else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    for (file, two, big) in [
        ("int32_Steim1_bigEndian.mseed", false, true),
        ("int32_Steim1_littleEndian.mseed", false, false),
        ("int32_Steim2_bigEndian.mseed", true, true),
        ("int32_Steim2_littleEndian.mseed", true, false),
        ("BW.BGLD.__.EHE.D.2008.001.first_10_records", false, true),
        ("steim2.mseed", true, true),
    ] {
        let bytes = std::fs::read(root.join("seismic").join(file)).unwrap();
        let (d, mut ev) = open(&root, file);
        let mut template: Vec<i128> = differences(&d, &mut ev, 0);
        template.extend(differences(&d, &mut ev, 1));
        let data = ev.node(&d, &[0, 0, DATA]).unwrap();
        let at = (data.offset_bits / 8) as usize;
        let payload = &bytes[at..at + (data.size_bits / 8) as usize];
        let mut reader = Vec::new();
        for (f, frame) in payload.chunks_exact(64).enumerate() {
            let word = |w: usize| -> [u8; 4] { frame[w * 4..w * 4 + 4].try_into().unwrap() };
            let nibbles = if big { u32::from_be_bytes(word(0)) } else { u32::from_le_bytes(word(0)) };
            for w in if f == 0 { 3 } else { 1 }..16 {
                let code = nibbles >> (30 - 2 * w) & 3;
                let mut out = Vec::new();
                mseed_steim::differences(word(w), code, two, big, &mut out).unwrap();
                reader.extend(out.into_iter().map(i128::from));
            }
        }
        assert!(template.len() > 40, "{file}: {} differences", template.len());
        assert_eq!(reader, template, "{file}");
    }
}

/// A record the file stops in the middle of has fewer frames than its header
/// has samples. What the frames hold is decoded, the rest is said to be
/// missing, and there is no check, since there is no last sample to make one.
#[test]
fn a_record_cut_short_decodes_what_its_frames_hold_and_says_how_far_that_went() {
    let Some(root) = samples() else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    let mut bytes = std::fs::read(root.join("seismic").join("steim2.mseed")).unwrap();
    bytes.truncate(2048);
    let d = Document::new(MemSource(bytes.clone()));
    let mut ev = Evaluator::new(formats::builtin("mseed").unwrap());
    let r = decode_24(&bytes, &d, &mut ev, 0);
    let steim = r.steim.as_ref().unwrap();
    let data_at = ev.node(&d, &[0, 0, DATA]).unwrap().offset_bits / 8;
    assert_eq!(steim.frames_in_record as u64, (2048 - data_at) / 64);
    assert!(r.samples.len() > 1000 && r.samples.len() < 5980, "{} samples", r.samples.len());
    assert_eq!(&r.samples[..2], &[2787.0, 2776.0]);
    let problem = r.problem.clone().expect("a problem");
    assert!(problem.contains(&format!("{} of the 5980", r.samples.len())), "{problem}");
    assert_eq!(r.check(), None);
}

/// miniSEED 3, which obspy cannot read. Two checks instead.
///
/// The first is the one the format carries: every Steim2 record's last sample
/// equals its reverse integration constant. The second is that libmseed wrote
/// its reference series both ways: beside each `.mseed3` file in the sample
/// collection, libmseed's `test/data` has a `.mseed2` of the same name, and
/// the numbers below are what obspy 1.5.1 reads from that 2.4 file, every
/// record joined. The records are of different lengths in the two versions,
/// so the series is compared whole rather than a record at a time.
#[test]
fn a_miniseed_3_record_decodes_to_the_series_its_2_4_twin_holds() {
    let Some(root) = samples() else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    // file, the 2.4 twin's count, first two, last, sum, weighted sum
    let table: &[(&str, usize, [f64; 2], f64, f64, f64)] = &[
        ("reference-testdata-steim2.mseed3", 499, [0.0, 6.0], -556206270.0, -1499709039.0, -749567035618.0),
        ("reference-testdata-nsec.mseed3", 500, [0.0, 6.0], 0.0, -1499709039.0, -749567035618.0),
        ("reference-testdata-float32.mseed3", 500, [0.0, 6.109208106994629], 0.0, -1499709037.3653364, -749567036640.0464),
    ];
    for &(file, count, first, last, sum, weighted) in table {
        let bytes = std::fs::read(root.join("seismic").join(file)).unwrap();
        let (d, mut ev) = open_as(&root, file, "mseed3");
        let records = ev.node(&d, &[0]).unwrap().child_count as usize;
        let mut series = Vec::new();
        for i in 0..records {
            let encoding = match ev.node(&d, &[0, i, ms3::ENCODING]).unwrap().value {
                Value::Enum { raw, .. } => raw as u8,
                other => panic!("{other:?}"),
            };
            let n = ev.node(&d, &[0, i, ms3::SAMPLE_COUNT]).unwrap().value.as_int().unwrap() as usize;
            let data = ev.node(&d, &[0, i, ms3::DATA]).unwrap();
            let at = (data.offset_bits / 8) as usize;
            let payload = &bytes[at..at + (data.size_bits / 8) as usize];
            // Steim frames are big-endian in a miniSEED 3 record, and
            // everything else is little-endian like the header.
            let r = decode_bytes(payload, encoding, matches!(encoding, 10 | 11), n);
            assert_eq!(r.problem, None, "{file} record {i}");
            assert_eq!(r.samples.len(), n, "{file} record {i}");
            if encoding == 11 {
                let check = r.check().unwrap();
                assert!(check.passed(), "{file} record {i}: last {} against reverse constant {}", check.last, check.xn);
            }
            series.extend(r.samples);
        }
        let got = facts(&series);
        assert_eq!((got.count, got.first.clone(), got.last), (count, first.to_vec(), last), "{file}");
        // A float series is summed in a different order by numpy, so its sums
        // are compared to a part in a billion rather than to the bit.
        let close = |a: f64, b: f64| (a - b).abs() <= b.abs() * 1e-9;
        assert!(close(got.sum, sum) && close(got.weighted, weighted), "{file}: {got:?}");
    }
}
