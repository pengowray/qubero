//! Real SEG-Y files, from segyio's test data.
//!
//! Six of them, chosen for what each one decides about how a file is read:
//! the byte order (a big-endian file and the same file swapped), the textual
//! header's encoding (EBCDIC and ASCII), how long a trace is (from the binary
//! header when the fixed length flag is set, from each trace otherwise), the
//! sample format (IBM and IEEE floats), and revision 2's extra trace headers.
//! The numbers checked here are the ones segyio reads from the same files.
//!
//! The files live in the sample collection rather than here. Point
//! `QUBERO_SAMPLES` at it, or keep it beside the repository as
//! `qubero-samples`. With neither, the test says so and passes.

use std::path::PathBuf;

use qubero_core::document::Document;
use qubero_core::eval::{Evaluator, Value};
use qubero_core::formats;
use qubero_core::source::MemSource;

fn sample(name: &str) -> Option<PathBuf> {
    qubero_samples::roots().into_iter().map(|r| r.join("segy").join(name)).find(|p| p.exists())
}

/// The file, opened with whatever template it sniffs as, which has to be
/// this one.
fn read(name: &str) -> Option<(Document<MemSource>, Evaluator)> {
    let bytes = std::fs::read(sample(name)?).unwrap();
    let head = &bytes[..bytes.len().min(formats::SNIFF_WINDOW)];
    assert_eq!(formats::sniff(head, bytes.len() as u64), Some("segy"), "{name} should sniff as SEG-Y");
    Some((Document::new(MemSource(bytes)), Evaluator::new(formats::builtin("segy").unwrap())))
}

/// The path to the field called `name` under `path`. A structure named by one
/// of its fields reads as its name and then that field's value, so the name
/// is matched at the front.
fn at(ev: &mut Evaluator, d: &Document<MemSource>, path: &[usize], name: &str) -> Vec<usize> {
    let n = ev.node(d, path).unwrap().child_count as usize;
    let i = (0..n)
        .find(|i| {
            let mut p = path.to_vec();
            p.push(*i);
            let got = ev.node(d, &p).unwrap().name;
            got == name || got.starts_with(&format!("{name} "))
        })
        .unwrap_or_else(|| panic!("no {name} under {path:?}"));
    let mut p = path.to_vec();
    p.push(i);
    p
}

fn value(ev: &mut Evaluator, d: &Document<MemSource>, path: &[usize]) -> Value {
    ev.node(d, path).unwrap().value
}

/// A field of the trace header of trace `t`.
fn header(ev: &mut Evaluator, d: &Document<MemSource>, t: usize, name: &str) -> Value {
    let traces = at(ev, d, &[], "traces");
    let header = at(ev, d, &[traces[0], t], "header");
    let field = at(ev, d, &header, name);
    value(ev, d, &field)
}

/// Sample `s` of trace `t`, as the f32 segyio reads it as.
fn sample_of(ev: &mut Evaluator, d: &Document<MemSource>, t: usize, s: usize) -> f32 {
    let traces = at(ev, d, &[], "traces");
    let mut samples = at(ev, d, &[traces[0], t], "samples");
    samples.push(s);
    match value(ev, d, &samples) {
        Value::Float(x) => x as f32,
        other => panic!("sample {s} of trace {t} is {other:?}"),
    }
}

fn text(v: Value) -> String {
    match v {
        Value::Str(s) => s,
        other => panic!("not text: {other:?}"),
    }
}

/// segyio's basic file and the same file swapped end to end read the same:
/// 25 traces of 50 IBM floats, with in-line and cross-line numbers in each
/// trace header, and a sample count of zero there that takes the binary
/// header's 50.
#[test]
fn a_real_file_reads_the_same_either_way_round() {
    for name in ["small.sgy", "small-lsb.sgy"] {
        let Some((d, mut ev)) = read(name) else {
            eprintln!("{}", qubero_samples::missing());
            return;
        };
        let card = at(&mut ev, &d, &[], "textual_header");
        let first = ev.node(&d, &[card[0], 0]).unwrap();
        assert_eq!(first.type_name, "ebcdic[]", "{name}");
        assert!(text(first.value).starts_with("C 1 DATE: 2016-09-19"));
        let traces = at(&mut ev, &d, &[], "traces");
        assert_eq!(ev.node(&d, &traces).unwrap().child_count, 25, "{name}");
        assert_eq!(header(&mut ev, &d, 0, "sample_count").as_int(), Some(0));
        let samples = at(&mut ev, &d, &[traces[0], 0], "samples");
        let run = ev.node(&d, &samples).unwrap();
        assert_eq!(run.child_count, 50);
        assert_eq!(run.type_name, if name == "small.sgy" { "ibm32 be[]" } else { "ibm32 le[]" });
        assert_eq!(sample_of(&mut ev, &d, 0, 0), 1.1999998_f32);
        assert_eq!(sample_of(&mut ev, &d, 0, 1), 1.2000093_f32);
        assert_eq!(sample_of(&mut ev, &d, 24, 49), 5.24049_f32);
        for (t, inline, crossline) in [(0, 1, 20), (24, 5, 24)] {
            assert_eq!(header(&mut ev, &d, t, "inline").as_int(), Some(inline));
            assert_eq!(header(&mut ev, &d, t, "crossline").as_int(), Some(crossline));
        }
        let last = ev.node(&d, &[traces[0], 24]).unwrap();
        assert_eq!(last.offset_bits + last.size_bits, d.len_bits(), "{name}: the traces reach the end of the file");
    }
}

/// A trace cut from a real survey, with an ASCII textual header and the time
/// fields scaled: a delay of 10000 with a scalar of -10 is a second.
#[test]
fn a_real_ascii_header_and_its_scalars() {
    let Some((d, mut ev)) = read("delay-scalar.sgy") else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    let cards = at(&mut ev, &d, &[], "textual_header");
    let first = ev.node(&d, &[cards[0], 0]).unwrap();
    assert_eq!(first.type_name, "ascii[]");
    assert!(text(first.value).starts_with("C 1 CLIENT"));
    assert!(text(value(&mut ev, &d, &[cards[0], 38])).starts_with("C39 SEG Y REV1"));
    assert_eq!(header(&mut ev, &d, 0, "delay_recording_time").as_int(), Some(10000));
    assert_eq!(header(&mut ev, &d, 0, "time_scalar").as_int(), Some(-10));
    assert_eq!(header(&mut ev, &d, 0, "inline").as_int(), Some(2500));
    assert_eq!(header(&mut ev, &d, 0, "crossline").as_int(), Some(1883));
    assert_eq!(header(&mut ev, &d, 0, "cdp_x").as_int(), Some(46709336));
    assert_eq!(sample_of(&mut ev, &d, 0, 250), 250.0);
}

/// F3 converted to IEEE floats, with the fixed length flag set and trace
/// headers that still say 462 samples from before the file was cropped to 75.
/// The binary header wins, and the 414 traces are counted by division.
#[test]
fn real_fixed_length_traces_follow_the_binary_header() {
    let Some((d, mut ev)) = read("Format5msb.sgy") else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    let traces = at(&mut ev, &d, &[], "traces");
    assert_eq!(ev.node(&d, &traces).unwrap().child_count, 414);
    assert_eq!(header(&mut ev, &d, 413, "sample_count").as_int(), Some(462));
    let samples = at(&mut ev, &d, &[traces[0], 413], "samples");
    let run = ev.node(&d, &samples).unwrap();
    assert_eq!((run.type_name.as_str(), run.child_count), ("f32 be[]", 75));
    assert_eq!(sample_of(&mut ev, &d, 0, 74), -394.0);
    assert_eq!(sample_of(&mut ev, &d, 413, 74), -121.0);
    assert_eq!(header(&mut ev, &d, 413, "inline").as_int(), Some(133));
    assert_eq!(header(&mut ev, &d, 413, "crossline").as_int(), Some(892));
}

/// small.sgy again, as revision 2.0 with an Extension 1 header on every trace
/// holding CDP coordinates as doubles. The samples are small.sgy's, after
/// 480 bytes of header rather than 240.
#[test]
fn a_real_revision_2_file_carries_extension_1() {
    let Some((d, mut ev)) = read("rotated-small-rev2.sgy") else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    let binary = at(&mut ev, &d, &[], "binary_header");
    let major = at(&mut ev, &d, &binary, "major_revision");
    assert_eq!(value(&mut ev, &d, &major).as_int(), Some(2));
    let count = at(&mut ev, &d, &[], "additional_trace_headers");
    assert_eq!(value(&mut ev, &d, &count).as_int(), Some(1));
    let traces = at(&mut ev, &d, &[], "traces");
    assert_eq!(ev.node(&d, &traces).unwrap().child_count, 25);
    let ext = at(&mut ev, &d, &[traces[0], 1], "extension_1");
    assert_eq!(ev.node(&d, &ext).unwrap().type_name, "SegyTraceHeaderExtension1");
    let name = at(&mut ev, &d, &ext, "header_name");
    assert_eq!(value(&mut ev, &d, &name), Value::Str("SEG00001".into()));
    let cdp_x = at(&mut ev, &d, &ext, "cdp_x");
    assert_eq!(value(&mut ev, &d, &cdp_x), Value::Float(2079.0));
    assert_eq!(header(&mut ev, &d, 1, "cdp_x").as_int(), Some(1));
    assert_eq!(sample_of(&mut ev, &d, 0, 0), 1.1999998_f32);
    assert_eq!(sample_of(&mut ev, &d, 24, 49), 5.24049_f32);
}

/// Revision 2.1: an ASCII layout stanza behind an EBCDIC textual header, and
/// two traces each carrying Extension 1 and a proprietary header after it.
#[test]
fn a_real_revision_2_1_file_names_its_extra_headers() {
    let Some((d, mut ev)) = read("trace-header-extensions.sgy") else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    let extended = at(&mut ev, &d, &[], "extended_textual_headers");
    assert_eq!(ev.node(&d, &extended).unwrap().child_count, 1);
    let stanza = ev.node(&d, &[extended[0], 0, 0]).unwrap();
    assert_eq!(stanza.type_name, "ascii[]");
    assert!(text(stanza.value).starts_with("((SEG:Layout:text/xml))"));
    let traces = at(&mut ev, &d, &[], "traces");
    assert_eq!(ev.node(&d, &traces).unwrap().child_count, 2);
    for t in 0..2 {
        let others = at(&mut ev, &d, &[traces[0], t], "extensions");
        assert_eq!(ev.node(&d, &others).unwrap().child_count, 1);
        let name = at(&mut ev, &d, &[others[0], others[1], others[2], 0], "header_name");
        assert_eq!(value(&mut ev, &d, &name), Value::Str("PRIVATE1".into()));
    }
    assert_eq!(sample_of(&mut ev, &d, 0, 0), 1.1999998_f32);
    assert_eq!(sample_of(&mut ev, &d, 1, 3), 1.2100296_f32);
    let last = ev.node(&d, &[traces[0], 1]).unwrap();
    assert_eq!(last.offset_bits + last.size_bits, d.len_bits());
}
