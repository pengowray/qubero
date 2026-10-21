//! A compressed file whose contents are past the 64 MiB a run may be held
//! whole at, read a piece at a time.
//!
//! The file is a raw from the Rubin Observatory's LATISS camera, gzipped:
//! 94,940 bytes that unpack to 75,677,760, a FITS file of a primary HDU, 16
//! image HDUs named `Segment00` to `Segment07` and `Segment10` to `Segment17`,
//! and two binary tables. It is in `qubero2-extras/rubin`, beside the checkout
//! or an ancestor of it; without that folder the test says so and passes.

use std::path::PathBuf;

use qubero_core::document::Document;
use qubero_core::eval::Evaluator;
use qubero_core::formats;
use qubero_core::source::MemSource;

fn rubin() -> Option<PathBuf> {
    let file = qubero_samples::checkout("qubero2-extras")?.join("rubin").join("3018092000065-det000.fits");
    file.exists().then_some(file)
}

/// Open unpacked on the gzip's `decoded` stream gives a FITS tab with all 19
/// HDUs, and the listing of the gzip reads the unpacked text where it is
/// declared. Both read through one index of the one member, and neither holds
/// the 75 MB.
#[test]
fn a_gzipped_rubin_raw_opens_as_fits() {
    let Some(file) = rubin() else {
        eprintln!("no qubero2-extras/rubin beside the checkout");
        return;
    };
    let bytes = std::fs::read(&file).unwrap();
    let found = formats::sniff(&bytes, bytes.len() as u64).expect("sniffs");
    let d = Document::new(MemSource(bytes));
    let mut e = Evaluator::new(formats::template(found).unwrap());
    assert_eq!(e.children(&d, &[], 0, 2).unwrap()[1].name, "decoded");

    // Where it is declared, the stream is text, read a span at a time.
    let text = e.node(&d, &[1, 0]).unwrap();
    assert_eq!(text.size_bits, 75_677_760 * 8);
    let first = e.children(&d, &[1, 0], 0, 1).unwrap();
    let qubero_core::eval::Value::Str(head) = &first[0].value else { panic!("{:?}", first[0].value) };
    assert!(head.starts_with("SIMPLE  =                    T"), "{}", &head[..80.min(head.len())]);

    let id = e.open_space(&d, 0, &[1]).unwrap().expect("opens as a document of its own");
    assert_eq!(e.open_refusal(&[1]), None);
    let space = e.space_mut(id).unwrap();
    assert_eq!((space.template.as_str(), space.recognised), ("fits", true));
    assert_eq!(space.len_bytes(), 75_677_760);
    assert!(space.held().is_none(), "a 75 MB stream is not held whole");

    // A byte deep in the stream maps to the deflate code that made it, with
    // its bits counted in the member's compressed run.
    let step = space.map_out(70_000_000).expect("a step made it");
    assert!(step.out_bytes.contains(&70_000_000));
    let run = &space.runs()[0];
    assert!(step.in_bits.end <= run.run_bits);
    assert_eq!(space.map_in(run.run_offset_bits + step.in_bits.start), Some(step));

    let (ev, doc) = space.reading().expect("a recognised stream has a reading of its own");
    let hdus = ev.children(doc, &[0], 0, 100).unwrap();
    assert_eq!(hdus.len(), 19);
    let mut segments: Vec<String> = hdus.iter().filter_map(|h| h.name.split_whitespace().nth(1).map(str::to_string)).collect();
    segments.sort();
    let want: Vec<String> = (0..8).chain(10..18).map(|n| format!("Segment{n:02}")).collect();
    assert_eq!(segments, want);
}

