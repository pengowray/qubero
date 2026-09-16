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
//! On Linux the same goes for whatever JPEGs the desktop ships under
//! `/usr/share/backgrounds`. It skips where neither is present.

use std::path::{Path, PathBuf};

use qubero_core::document::Document;
use qubero_core::eval::{Evaluator, Value};
use qubero_core::formats::{jpeg, sniff};
use qubero_core::source::MemSource;

const WINDOWS: &[&str] = &[
    "C:/Windows/Web/Wallpaper/Theme1/img1.jpg",
    "C:/Windows/Web/Wallpaper/Theme1/img2.jpg",
    "C:/Windows/Web/Wallpaper/Theme1/img3.jpg",
    "C:/Windows/Web/Wallpaper/Theme1/img4.jpg",
];

const LINUX: &str = "/usr/share/backgrounds";

/// How many of the Linux wallpapers to read. Four, like the Windows set: they
/// are photographs of several megabytes each, and the point is made by a few.
const ENOUGH: usize = 4;

/// The wallpapers this machine has: the Windows set where it exists, and
/// otherwise the first few JPEGs under the Linux backgrounds directory, in
/// path order so the same ones are read every time.
fn wallpapers() -> Vec<PathBuf> {
    let windows: Vec<PathBuf> = WINDOWS.iter().map(PathBuf::from).filter(|p| p.is_file()).collect();
    if !windows.is_empty() {
        return windows;
    }
    let mut found = Vec::new();
    jpegs_under(Path::new(LINUX), &mut found);
    found.sort();
    found.truncate(ENOUGH);
    found
}

fn jpegs_under(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            jpegs_under(&path, out);
        } else if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("jpg") || e.eq_ignore_ascii_case("jpeg")) {
            out.push(path);
        }
    }
}

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
    let files = wallpapers();
    if files.is_empty() {
        eprintln!("skipped: no wallpaper to read, neither {} nor a JPEG under {LINUX}", WINDOWS[0]);
        return;
    }
    for path in &files {
        let path = path.display().to_string();
        let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
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
        eprintln!("{path}: {n} segments, {scans} scan(s), covered to the byte");
    }
}
