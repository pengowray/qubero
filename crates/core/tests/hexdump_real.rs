//! Reading the dumps in the sample collection back into the bytes they are
//! dumps of.
//!
//! `hexdump/` holds one small binary and a dozen text files describing it,
//! written by the tools people actually have: `xxd` with several of its
//! options, `od` with three address bases, `certutil`, PowerShell's
//! `Format-Hex` in two encodings, and a terminal transcript with the prompts
//! left in. Every one of them has to come back as the same bytes.
//!
//! Like the rest of the collection this skips when the folder is not beside
//! the repository. Point `QUBERO_SAMPLES` at it to run it elsewhere.

use std::path::{Path, PathBuf};

use qubero_core::hexdump::{self, Note};

/// The transcript is the one file that is not a dump of the whole thing: it
/// holds two runs of a dumping tool over different stretches, and the bytes
/// between them are in neither.
const PARTIAL: &str = "session-bash-prompt.txt";

/// XTree Gold's hex view is a screen rather than a stream. Nineteen lines fit,
/// so a capture of it covers the first 0x130 bytes and stops, and the file goes
/// on for another 158.
const SCREEN: usize = 0x130;

#[test]
fn every_dump_reads_back_as_the_bytes_it_describes() {
    let Some(dir) = folder() else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    let want = std::fs::read(dir.join("source-bytes.bin")).expect("the file the dumps describe");
    let mut checked = 0;
    for path in dumps(&dir) {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let text = std::fs::read(&path).unwrap();
        let dump = hexdump::read(&text, 0).unwrap_or_else(|| panic!("no dump found in {name}"));

        assert!(dump.conflicts().is_empty(), "{name}: the two columns disagree at {:x?}", dump.conflicts().first());

        for e in dump.extents() {
            let mut got = vec![0u8; e.len as usize];
            let n = dump.read_at(e.at, &mut got);
            assert_eq!(n, e.len as usize, "{name}: short read over {:#x}..{:#x}", e.at, e.end());
            assert_eq!(got, want[e.at as usize..e.end() as usize], "{name}: wrong bytes at {:#x}", e.at);
        }

        if name == PARTIAL {
            assert_eq!(dump.extents().len(), 2, "{name}: two runs of a dumping tool, so two stretches");
            assert!(
                dump.notes.contains(&Note::Named("source-bytes.bin".into())),
                "{name}: the transcript names the file on the command line"
            );
        } else if name.starts_with("xtreegold") {
            assert_eq!(dump.byte_count(), SCREEN as u64, "{name}: a screen holds nineteen lines and no more");
            assert!(
                dump.notes.contains(&Note::Named("C:\\SAMPLE.BIN".into())),
                "{name}: the header across the top names the file, and only the file"
            );
        } else {
            assert_eq!(dump.byte_count(), want.len() as u64, "{name}: did not cover the whole file");
        }
        checked += 1;
    }
    assert!(checked >= 10, "only {checked} dumps found; the folder is meant to hold more");
}

/// A dump written from the layout a dump was read in is the same text again.
/// Anything the reader failed to notice about the layout comes back in the
/// wrong column here.
#[test]
fn a_dump_written_out_again_is_the_same_text() {
    let Some(dir) = folder() else { return };
    // The tools whose every column this reproduces, squeezed runs and all.
    // The others are read correctly but carry lines that are not lines of a
    // dump: `certutil` writes two header lines and `Format-Hex` a label and a
    // column heading, and writing those back is not this function's job.
    const EXACT: [&str; 7] = [
        "xxd-default.txt",
        "xxd-uppercase.txt",
        "xxd-g1.txt",
        "xxd-c8.txt",
        "xxd-autoskip.txt",
        "xxd-endian-swapped.txt",
        "od-ax-tx1z.txt",
    ];
    for path in dumps(&dir) {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        if !EXACT.contains(&name.as_str()) {
            continue;
        }
        let text = std::fs::read(&path).unwrap();
        let dump = hexdump::read(&text, 0).unwrap();
        let (from, _) = dump.span().unwrap();
        let mut bytes = vec![0u8; dump.byte_count() as usize];
        dump.read_at(from, &mut bytes);
        let written = hexdump::write::dump(&dump.layout, &bytes, from, dump.layout.squeeze.is_some());
        let original = String::from_utf8_lossy(&text).replace("\r\n", "\n");
        assert_eq!(written, original, "{name}: written out again, it is not the text it was");
    }
}

fn folder() -> Option<PathBuf> {
    let named = std::env::var_os("QUBERO_SAMPLES").map(PathBuf::from);
    let beside = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../qubero-samples");
    named.into_iter().chain(std::iter::once(beside)).map(|p| p.join("hexdump")).find(|p| p.is_dir())
}

fn dumps(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "txt"))
        .collect();
    out.sort();
    out
}
