//! The wallpapers Windows ships, which are ordinary photographs written by an
//! ordinary encoder: several megabytes of compressed bits with restart markers
//! all through them, quantisation and Huffman tables several to a segment, and
//! an Adobe marker saying which way round the colours are.
//!
//! What this checks is the thing a made-up file cannot: that the scan, whose
//! length is written down nowhere, is measured to exactly the right byte. If
//! it stops early the marker after it is not one, and if it runs on the file
//! ends short. Either way the segments stop covering the file, and that is
//! what is asserted.
//!
//! It skips where those files are not present.

use qubero_core::document::Document;
use qubero_core::eval::{Evaluator, Value};
use qubero_core::formats::{jpeg, sniff};
use qubero_core::source::MemSource;

const FILES: &[&str] = &[
    "C:/Windows/Web/Wallpaper/Theme1/img1.jpg",
    "C:/Windows/Web/Wallpaper/Theme1/img2.jpg",
    "C:/Windows/Web/Wallpaper/Theme1/img3.jpg",
    "C:/Windows/Web/Wallpaper/Theme1/img4.jpg",
];

/// Resolve every node under a path. A table whose length is worked out wrongly
/// is an error here rather than a row nobody opened.
fn deep(d: &Document<MemSource>, ev: &mut Evaluator, at: &[usize], depth: usize) {
    if depth > 8 {
        return;
    }
    let n = ev.node(d, at).unwrap_or_else(|e| panic!("{at:?}: {e:?}")).child_count;
    for i in 0..n as usize {
        let mut p = at.to_vec();
        p.push(i);
        deep(d, ev, &p, depth + 1);
    }
}

#[test]
fn every_segment_reads_and_together_they_cover_the_file() {
    let mut checked = 0;
    for path in FILES {
        let Ok(bytes) = std::fs::read(path) else {
            eprintln!("skipped: no file at {path}");
            continue;
        };
        checked += 1;
        assert_eq!(sniff(&bytes[..64], bytes.len() as u64), Some("jpeg"), "{path}");

        let len = bytes.len() as u64;
        let d = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(jpeg());
        let n = ev.node(&d, &[1]).unwrap().child_count;
        let (mut scans, mut end) = (0, 0);
        for i in 0..n as usize {
            let s = ev.node(&d, &[1, i]).unwrap();
            // A segment of no bytes would let the list stand still forever.
            assert!(s.size_bits > 0, "empty segment {i} in {path}");
            deep(&d, &mut ev, &[1, i], 0);
            if ev.node(&d, &[1, i, 0]).unwrap().value.as_int() == Some(0xffda) {
                scans += 1;
            }
            end = s.offset_bits / 8 + s.size_bits / 8;
        }
        assert!(scans >= 1, "no scan in {path}");
        // The last segment is the end marker, and what follows it is the
        // trailer, so between them they reach the end of the file exactly.
        assert_eq!(
            ev.node(&d, &[1, n as usize - 1, 0]).unwrap().value,
            Value::Enum { raw: 0xffd9, name: Some("eoi, end of image".into()), hex: true },
            "{path}"
        );
        let trailer = ev.node(&d, &[2]).unwrap();
        assert_eq!(trailer.offset_bits / 8, end, "{path}");
        assert_eq!(end + trailer.size_bits / 8, len, "the segments do not cover {path}");
    }
    assert!(checked > 0, "no wallpaper to read");
}
