//! What the SEG-Y template is checked against.
//!
//! `File` writes a small SEG-Y file, either way round and in revision 1 or 2,
//! so a test can say what shape it means rather than carry a blob nobody can
//! read. The tests are apart from the template because they were a quarter of
//! what was one eleven-hundred line file.

use super::*;
use crate::document::Document;
use crate::encode::f64_to_ibm32;
use crate::eval::{Evaluator, Value};
use crate::source::MemSource;

/// Root fields, by index.
const TEXTUAL: usize = 1;
const BINARY: usize = 2;
const EXTENDED: usize = 3;
const TRACES: usize = 10;
const TRAILER: usize = 11;

/// Binary header fields, by index.
const FORMAT_FIELD: usize = 9;
const REV2_COUNTS: usize = 27;
const REV2_LAYOUT: usize = 32;

struct File {
    big: bool,
    ebcdic: bool,
    format: i16,
    /// The binary header's samples per trace.
    samples: u16,
    /// Each trace's own sample count, one trace each.
    traces: Vec<u16>,
    major: u8,
    minor: u8,
    fixed: bool,
    extended: Vec<Vec<u8>>,
    extended_count: i16,
    /// Revision 2's extension headers per trace, each named.
    extension_names: Vec<[u8; 8]>,
}

impl File {
    fn rev1(big: bool) -> File {
        File {
            big,
            ebcdic: big,
            format: 1,
            samples: 3,
            traces: vec![3, 3],
            major: 1,
            minor: 0,
            fixed: false,
            extended: Vec::new(),
            extended_count: 0,
            extension_names: Vec::new(),
        }
    }

    fn text(&self, s: &str, len: usize) -> Vec<u8> {
        let mut t = s.to_string();
        while t.len() < len {
            t.push(' ');
        }
        match self.ebcdic {
            true => encode_settled(Settled::SingleByte(CodePage::Ebcdic037), &t).unwrap(),
            false => t.into_bytes(),
        }
    }

    fn build(&self) -> Vec<u8> {
        let big = self.big;
        let put16 = |v: &mut Vec<u8>, at: usize, n: i16| {
            v[at..at + 2].copy_from_slice(&if big { n.to_be_bytes() } else { n.to_le_bytes() })
        };
        let put32 = |v: &mut Vec<u8>, at: usize, n: u32| {
            v[at..at + 4].copy_from_slice(&if big { n.to_be_bytes() } else { n.to_le_bytes() })
        };
        let mut v = Vec::new();
        for i in 1..=40 {
            v.extend(self.text(&format!("C{i:2} LINE {i}"), 80));
        }
        v.resize(FILE_HEADER, 0);
        put32(&mut v, 3200, 77); // job id
        put16(&mut v, INTERVAL_AT, 4000);
        put16(&mut v, 3220, self.samples as i16);
        put16(&mut v, FORMAT_AT, self.format);
        v[3500] = self.major;
        v[3501] = self.minor;
        put16(&mut v, 3502, self.fixed as i16);
        put16(&mut v, 3504, self.extended_count);
        if self.major >= 2 {
            put32(&mut v, 3296, 0x0102_0304);
            // Revision 2.1's two-byte count.
            put16(&mut v, 3506, self.extension_names.len() as i16);
        }
        for block in &self.extended {
            let mut b = self.text(std::str::from_utf8(block).unwrap(), 3200);
            b.truncate(3200);
            v.extend(b);
        }
        for (t, ns) in self.traces.iter().enumerate() {
            let at = v.len();
            v.resize(at + 240, 0);
            put32(&mut v, at, t as u32 + 1);
            put16(&mut v, at + 114, *ns as i16);
            for name in &self.extension_names {
                let at = v.len();
                v.resize(at + 240, 0);
                v[at + 232..at + 240].copy_from_slice(&self.text(std::str::from_utf8(name).unwrap(), 8));
            }
            let n = if self.fixed || *ns == 0 { self.samples } else { *ns };
            for s in 0..n {
                let x = [1.0, -118.625, 0.0, 2.5][s as usize % 4];
                let word = match self.format {
                    1 => f64_to_ibm32(x).unwrap(),
                    5 => (x as f32).to_bits(),
                    _ => panic!("the tests write IBM or IEEE floats"),
                };
                let at = v.len();
                v.resize(at + 4, 0);
                put32(&mut v, at, word);
            }
        }
        v
    }
}

fn open(v: Vec<u8>) -> (Document<MemSource>, Evaluator) {
    (Document::new(MemSource(v)), Evaluator::new(segy()))
}

#[test]
fn a_minimal_file_reads_either_way_round() {
    for big in [true, false] {
        let f = File::rev1(big);
        let bytes = f.build();
        assert!(is_segy(&bytes, bytes.len() as u64), "big={big}");
        let (d, mut ev) = open(bytes);
        // EBCDIC for the big-endian one and ASCII for the other, and the
        // card reads the same either way.
        let card = ev.node(&d, &[TEXTUAL, 0]).unwrap();
        assert_eq!(card.value, Value::Str(format!("{:80}", "C 1 LINE 1")), "big={big}");
        assert_eq!(card.type_name, if big { "ebcdic[]" } else { "ascii[]" });
        assert_eq!(card.size_bits, 80 * 8);
        assert_eq!(ev.node(&d, &[BINARY, 0]).unwrap().value.as_int(), Some(77));
        assert_eq!(ev.node(&d, &[BINARY, 5]).unwrap().value.as_int(), Some(4000));
        assert_eq!(ev.node(&d, &[BINARY, FORMAT_FIELD]).unwrap().value.as_int(), Some(1));
        let traces = ev.node(&d, &[TRACES]).unwrap();
        assert_eq!(traces.offset_bits, FILE_HEADER as u64 * 8);
        assert_eq!(traces.child_count, 2);
        let trace = ev.node(&d, &[TRACES, 1]).unwrap();
        assert_eq!(trace.offset_bits, (FILE_HEADER as u64 + 240 + 12) * 8);
        assert_eq!(ev.node(&d, &[TRACES, 1, 0, 0]).unwrap().value.as_int(), Some(2), "trace_sequence_in_line");
        let samples = ev.node(&d, &[TRACES, 1, 1]).unwrap();
        assert_eq!((samples.name.as_str(), samples.child_count), ("samples", 3));
        assert_eq!(ev.node(&d, &[TRACES, 1, 1, 0]).unwrap().type_name, if big { "ibm32 be" } else { "ibm32 le" });
        assert_eq!(ev.node(&d, &[TRACES, 1, 1, 0]).unwrap().value, Value::Float(1.0));
        assert_eq!(ev.node(&d, &[TRACES, 1, 1, 1]).unwrap().value, Value::Float(-118.625));
    }
}

/// A trace header whose count is zero takes the binary header's, and one
/// that gives its own count is read at that length.
#[test]
fn a_trace_without_the_flag_is_as_long_as_its_own_header_says() {
    let mut f = File::rev1(true);
    f.traces = vec![0, 5, 2];
    let (d, mut ev) = open(f.build());
    let counts: Vec<u64> = (0..3).map(|t| ev.node(&d, &[TRACES, t, 1]).unwrap().child_count).collect();
    assert_eq!(counts, [3, 5, 2]);
    let last = ev.node(&d, &[TRACES, 2]).unwrap();
    assert_eq!(last.offset_bits + last.size_bits, d.len_bits());
}

/// With the flag set, the binary header's count is the length of every
/// trace whatever the trace headers say, and the list is counted by
/// division.
#[test]
fn fixed_length_traces_are_all_the_binary_headers_length() {
    let mut f = File::rev1(false);
    f.fixed = true;
    f.format = 5;
    f.samples = 4;
    f.traces = vec![462; 6];
    let (d, mut ev) = open(f.build());
    assert_eq!(ev.node(&d, &[9]).unwrap().value.as_int(), Some(240 + 16), "trace_bytes");
    assert_eq!(ev.node(&d, &[TRACES]).unwrap().child_count, 6);
    let samples = ev.node(&d, &[TRACES, 5, 1]).unwrap();
    assert_eq!((samples.type_name.as_str(), samples.child_count), ("f32 le[]", 4));
    assert_eq!(ev.node(&d, &[TRACES, 5, 0, 38]).unwrap().value.as_int(), Some(462), "the header's own count, unused");
    assert_eq!(ev.node(&d, &[TRACES, 5, 1, 3]).unwrap().value, Value::Float(2.5));
}

/// A revision 2 file: the byte order integer, the counts after the
/// revision, and an Extension 1 header ahead of each trace's samples.
#[test]
fn revision_2_reads_its_extension_headers() {
    for big in [true, false] {
        let mut f = File::rev1(big);
        f.major = 2;
        f.minor = 1;
        f.extension_names = vec![*b"SEG00001", *b"PRIVATE1"];
        let (d, mut ev) = open(f.build());
        let order = ev.node(&d, &[BINARY, REV2_COUNTS, 7]).unwrap();
        assert_eq!(order.value.as_int(), Some(0x0102_0304), "big={big}");
        assert_eq!(ev.node(&d, &[BINARY, REV2_LAYOUT, 0]).unwrap().value.as_int(), Some(2));
        assert_eq!(ev.node(&d, &[8]).unwrap().value.as_int(), Some(2), "additional_trace_headers");
        assert_eq!(ev.node(&d, &[TRACES, 1, 1]).unwrap().type_name, "SegyTraceHeaderExtension1");
        let ext = ev.node(&d, &[TRACES, 1, 2]).unwrap();
        assert_eq!((ext.type_name.as_str(), ext.child_count), ("SegyTraceHeaderExtension[]", 1));
        assert_eq!(ev.node(&d, &[TRACES, 1, 2, 0, 1]).unwrap().value, Value::Str("PRIVATE1".into()));
        let header = ev.node(&d, &[TRACES, 1, 0]).unwrap();
        let name = ev.node(&d, &[TRACES, 1, 0, header.child_count as usize - 1]).unwrap();
        assert_eq!(name.name, "header_name");
        assert_eq!(ev.node(&d, &[TRACES, 1, 3, 1]).unwrap().value, Value::Float(-118.625));
        let last = ev.node(&d, &[TRACES, 1]).unwrap();
        assert_eq!(last.offset_bits + last.size_bits, d.len_bits());
    }
}

/// Revision 2.0 gives the extension count four bytes. One written that
/// way reads as itself, and one written in two bytes the way segyio
/// writes a 2.0 file reads as the two bytes.
#[test]
fn a_revision_2_0_extension_count_reads_either_width() {
    for two_bytes in [false, true] {
        let mut f = File::rev1(true);
        f.major = 2;
        f.minor = 0;
        f.extension_names = vec![*b"SEG00001"];
        let mut v = f.build();
        v[3506..3510].copy_from_slice(if two_bytes { &[0, 1, 0, 0] } else { &[0, 0, 0, 1] });
        let (d, mut ev) = open(v);
        assert_eq!(ev.node(&d, &[8]).unwrap().value.as_int(), Some(1), "two_bytes={two_bytes}");
        assert_eq!(ev.node(&d, &[TRACES, 0, 1]).unwrap().type_name, "SegyTraceHeaderExtension1");
        assert_eq!(ev.node(&d, &[TRACES, 0, 2]).unwrap().child_count, 0);
    }
}

#[test]
fn extended_textual_headers_are_counted_or_end_at_end_text() {
    let mut f = File::rev1(true);
    f.extended = vec![b"((SEG: Something))".to_vec(), b"((SEG: EndText))".to_vec()];
    f.extended_count = 2;
    let mut v = f.build();
    // The first block in ASCII behind an EBCDIC textual header, which
    // segyio's own files do: each block says what it is in.
    let ascii = format!("{:3200}", "((SEG: Something))");
    v[FILE_HEADER..FILE_HEADER + 3200].copy_from_slice(ascii.as_bytes());
    let (d, mut ev) = open(v.clone());
    assert_eq!(ev.node(&d, &[EXTENDED]).unwrap().child_count, 2);
    let first = ev.node(&d, &[EXTENDED, 0, 0]).unwrap();
    assert_eq!(first.type_name, "ascii[]");
    assert!(matches!(first.value, Value::Str(s) if s.starts_with("((SEG: Something))  ")));
    assert_eq!(ev.node(&d, &[EXTENDED, 1, 0]).unwrap().type_name, "ebcdic[]");
    assert_eq!(ev.node(&d, &[TRACES]).unwrap().offset_bits, (FILE_HEADER as u64 + 6400) * 8);
    assert_eq!(ev.node(&d, &[TRACES]).unwrap().child_count, 2);

    // The same file with a count of -1 finds the end on its own.
    v[3504..3506].copy_from_slice(&(-1i16).to_be_bytes());
    let (d, mut ev) = open(v);
    assert_eq!(ev.node(&d, &[EXTENDED]).unwrap().child_count, 2);
    assert_eq!(ev.node(&d, &[EXTENDED, 1, 0]).unwrap().value.as_int(), Some(1), "end_text");
    assert_eq!(ev.node(&d, &[TRACES]).unwrap().child_count, 2);
    assert_eq!(ev.node(&d, &[TRAILER]).unwrap().child_count, 0);
}

/// A format code nobody defined leaves the traces unmeasurable, and they
/// stay bytes rather than being read at a guessed width.
#[test]
fn an_unknown_format_leaves_the_traces_as_bytes() {
    let mut v = File::rev1(true).build();
    v[FORMAT_AT..FORMAT_AT + 2].copy_from_slice(&13i16.to_be_bytes());
    let (d, mut ev) = open(v);
    let traces = ev.node(&d, &[TRACES]).unwrap();
    assert_eq!(traces.type_name, "bytes[]");
    assert_eq!(traces.size_bits, (240 + 12) * 2 * 8);
}

#[test]
fn recognised_by_its_first_card_and_a_plausible_binary_header() {
    let good = File::rev1(true).build();
    assert!(is_segy(&good, good.len() as u64));
    let mut ascii = File::rev1(false).build();
    assert!(is_segy(&ascii, ascii.len() as u64));
    // A C and a digit is not enough without a format code the standard
    // defines.
    ascii[FORMAT_AT..FORMAT_AT + 2].copy_from_slice(&[0, 0]);
    assert!(!is_segy(&ascii, ascii.len() as u64));
    let mut text = vec![b' '; 4000];
    text[..3].copy_from_slice(b"C 1");
    assert!(!is_segy(&text, 4000));
    assert!(!is_segy(&good[..3000], 3000));
}
