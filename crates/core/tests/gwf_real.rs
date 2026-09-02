//! The GWOSC frame file, read against its own dictionary.
//!
//! What this is for: the template picks each structure's body by the name the
//! file's own FrSH structures declare, and the only way to know that works is
//! a file that carries a dictionary. This one carries 151 structures of
//! nothing else.

use qubero_core::document::Document;
use qubero_core::eval::{Evaluator, Value};
use qubero_core::formats;
use qubero_core::source::MemSource;

/// Where the stream of structures lands under the endianness switch.
const STREAM: &[usize] = &[8, 7];

fn sample() -> Option<Vec<u8>> {
    let root = match std::env::var_os("QUBERO_SAMPLES") {
        Some(p) => std::path::PathBuf::from(p),
        None => std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../qubero-samples"),
    };
    std::fs::read(root.join("gwf/H-H1_GWOSC_4KHZ_R1-1126259447-32.gwf")).ok()
}

#[test]
fn every_structure_of_the_gwosc_frame_reads_as_the_class_its_dictionary_names() {
    let Some(bytes) = sample() else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    let len = bytes.len();
    let d = Document::new(MemSource(bytes));
    let mut ev = Evaluator::new(formats::builtin("gwf").unwrap());
    let stream = ev.node(&d, STREAM).unwrap();
    assert_eq!(stream.child_count, 162);

    // Every structure's body resolves, and the last one ends exactly at the
    // end of the file: every length in the stream added up.
    let mut seen: Vec<String> = Vec::new();
    for i in 0..162usize {
        let body = ev.node(&d, &[8, 7, i, 5]).unwrap();
        seen.push(body.type_name.clone());
    }
    let last = ev.node(&d, &[8, 7, 161]).unwrap();
    assert_eq!(last.offset_bits + last.size_bits, len as u64 * 8);

    let count = |name: &str| seen.iter().filter(|t| *t == name).count();
    assert_eq!(count("FrSH"), 7);
    assert_eq!(count("FrSE"), 144);
    assert_eq!(count("FrameH"), 1);
    assert_eq!(count("FrDetector"), 1);
    assert_eq!(count("FrProcData"), 3);
    assert_eq!(count("FrVect"), 3);
    assert_eq!(count("FrEndOfFrame"), 1);
    assert_eq!(count("FrTOC"), 1);
    assert_eq!(count("FrEndOfFile"), 1);
    assert_eq!(count("bytes[]"), 0, "nothing in this file falls through to bytes");

    // The frame header names the project and the second GW150914 arrived in.
    let frame = seen.iter().position(|t| t == "FrameH").unwrap();
    assert_eq!(ev.node(&d, &[8, 7, frame, 5, 0, 1]).unwrap().value, Value::Str("LIGO".into()));
    assert_eq!(ev.node(&d, &[8, 7, frame, 5, 4]).unwrap().value, Value::UInt(1_126_259_447));

    // Every vector in this file is gzip written on a little-endian machine, so
    // its bytes stay bytes.
    let vect = seen.iter().position(|t| t == "FrVect").unwrap();
    assert_eq!(
        ev.node(&d, &[8, 7, vect, 5, 1]).unwrap().value,
        Value::Enum { raw: 257, name: Some("gzip (little-endian words)".into()), hex: false }
    );
    assert_eq!(ev.node(&d, &[8, 7, vect, 5, 5]).unwrap().type_name, "bytes[]");
}
