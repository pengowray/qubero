//! RAR 5 archives WinRAR wrote, unpacked and checked against the CRC-32 each
//! entry carries.
//!
//! The collection is not in this repository: point `QUBERO_SAMPLES` at it, or
//! keep it beside the checkout as `qubero-samples`. With neither, this says so
//! and passes.
//!
//! Nothing here builds an archive. Every file is one WinRAR packed, taken from
//! libarchive's test suite, and the entry's own stored checksum is the answer:
//! a decoder that agrees with it on 1200 bytes of one file and on four
//! 4096-byte files is a decoder that read what RAR wrote, and no test written
//! against this implementation could say that.
//!
//! The archives that must *not* open are as much of the point. A solid entry
//! and an encrypted one both have a data area a decompressor would happily
//! chew on, and both would then fail a checksum on an archive that is perfectly
//! sound. Those are checked to offer no check at all.

use std::path::{Path, PathBuf};

use qubero_core::document::Document;
use qubero_core::eval::Evaluator;
use qubero_core::formats;
use qubero_core::source::MemSource;

/// An archive read, with its blocks placed.
fn open(name: &str) -> Option<(Document<MemSource>, Evaluator)> {
    let path = find(name)?;
    let bytes = std::fs::read(&path).expect("reads");
    Some((Document::new(MemSource(bytes)), Evaluator::new(formats::builtin("rar5").expect("rar5"))))
}

/// What the file block at `block` is called.
fn name_of(e: &mut Evaluator, d: &Document<MemSource>, block: usize) -> String {
    let p = e.child_named(d, &[1, block, 2, 4], "name").expect("resolves").expect("a name");
    match e.node(d, &p).expect("a node").value {
        qubero_core::eval::Value::Str(s) => s.to_string(),
        other => panic!("a name read as {other:?}"),
    }
}

/// That the entry at `block` opened as a document of its own, and that its
/// stored CRC-32 is the sum of what came out.
///
/// The one assertion this file exists for. `run_check` unpacks the run and
/// sums whatever the decoder produced; the number it is compared against was
/// written by WinRAR before any of this existed.
fn checks_out(e: &mut Evaluator, d: &Document<MemSource>, block: usize) {
    let name = name_of(e, d, block);
    let data = e.node(d, &[1, block, 4]).expect("a data area");
    assert!(data.decoded, "{name}: the data area did not open: {data:?}");
    assert_eq!(data.refused, None, "{name}: the data area was refused");

    let crc = e.child_named(d, &[1, block, 2, 4], "data_crc32").expect("resolves").expect("a crc field");
    let info = e.check_of(d, &crc).expect("resolves").expect("an entry that opens is checkable");
    assert_eq!(info.algorithm, "crc32");
    let v = e.run_check(d, &crc).expect("resolves").expect("a verdict");
    assert!(v.ok, "{name}: computed {} against the stored {}", v.computed, v.stored);
}

/// That the entry at `block` stayed the bytes it is, and offers no check.
///
/// Both halves matter. A run left as bytes is the template saying it cannot
/// read this entry; a check that then said anything would be summing packed or
/// encrypted bytes against a number that describes neither.
fn stays_bytes(e: &mut Evaluator, d: &Document<MemSource>, block: usize) {
    let name = name_of(e, d, block);
    let data = e.node(d, &[1, block, 4]).expect("a data area");
    assert!(!data.decoded, "{name}: this entry must not open");
    let crc = e.child_named(d, &[1, block, 2, 4], "data_crc32").expect("resolves").expect("a crc field");
    assert_eq!(e.check_of(d, &crc).expect("resolves"), None, "{name}: a check that cannot be made must say nothing");
}

/// One file, packed, and the sum over what came out of it.
#[test]
fn a_packed_entry_unpacks_to_what_its_checksum_says() {
    let Some((d, mut e)) = open("rar5-one-file.rar") else {
        eprintln!("skipped: no rar5-one-file.rar in hand. Set QUBERO_SAMPLES to the collection.");
        return;
    };
    assert_eq!(name_of(&mut e, &d, 1), "test.bin");
    checks_out(&mut e, &d, 1);

    // And the bytes are the 1200 the header said, which is the other half of
    // the claim: a decoder can agree with a checksum over the wrong length only
    // by accident, but a reader is owed the length as well.
    let id = e.open_space(&d, 0, &[1, 1, 4]).expect("resolves").expect("the entry opens");
    assert_eq!(e.space(id).expect("just opened").len_bytes(), 1200);
}

/// Four files in one archive, each packed on its own.
#[test]
fn every_entry_of_a_four_file_archive_checks_out() {
    let Some((d, mut e)) = open("rar5-four-files.rar") else {
        eprintln!("skipped: no rar5-four-files.rar in hand. Set QUBERO_SAMPLES to the collection.");
        return;
    };
    for block in 1..=4 {
        assert_eq!(name_of(&mut e, &d, block), format!("test{block}.bin"));
        checks_out(&mut e, &d, block);
    }
}

/// A solid archive: the first entry starts from an empty window and reads, and
/// every entry after it continues the one before and does not.
///
/// The sharpest test in here, because the difference is one bit of one field
/// and both halves look identical from the outside. The first entry's checksum
/// is the same number the non-solid archive writes for the same file, so what
/// it proves is that the entry really was decoded and not merely accepted.
#[test]
fn only_the_first_entry_of_a_solid_archive_opens() {
    let Some((d, mut e)) = open("rar5-four-files-solid.rar") else {
        eprintln!("skipped: no rar5-four-files-solid.rar in hand. Set QUBERO_SAMPLES to the collection.");
        return;
    };
    assert_eq!(name_of(&mut e, &d, 1), "test1.bin");
    checks_out(&mut e, &d, 1);
    for block in 2..=4 {
        stays_bytes(&mut e, &d, block);
    }
}

/// An archive where two entries are behind a password.
///
/// The encrypted ones are packed as well, so nothing but the extra area's
/// record 1 tells them apart from an entry this could read. Left ungated they
/// reach the decompressor, and the one in a couple of hundred that survives the
/// block header's single check byte would report a broken file to somebody
/// whose archive is fine.
#[test]
fn an_encrypted_entry_is_left_alone_and_an_unencrypted_one_beside_it_is_not() {
    let Some((d, mut e)) = open("rar5-encrypted.rar") else {
        eprintln!("skipped: no rar5-encrypted.rar in hand. Set QUBERO_SAMPLES to the collection.");
        return;
    };
    // a.txt and c.txt are stored and in the clear; b.txt and d.txt are packed
    // and encrypted.
    assert_eq!(name_of(&mut e, &d, 1), "a.txt");
    assert_eq!(name_of(&mut e, &d, 2), "b.txt");
    checks_out(&mut e, &d, 1);
    stays_bytes(&mut e, &d, 2);
    checks_out(&mut e, &d, 3);
    stays_bytes(&mut e, &d, 4);
}

/// A file packed with the ARM filter, which rewrites branch targets on the way
/// out and is the one part of the decoder no other sample here exercises.
///
/// It is also the only entry in the collection packed into more than one
/// block, which is a second thing nothing else here reaches: the block chain,
/// the cursor handed from one block to the next, and the bits each block was
/// padded out with in between.
#[test]
fn a_filtered_entry_checks_out_once_the_filter_has_run() {
    let Some((d, mut e)) = open("rar5-arm-filter.rar") else {
        eprintln!("skipped: no rar5-arm-filter.rar in hand. Set QUBERO_SAMPLES to the collection.");
        return;
    };
    checks_out(&mut e, &d, 1);
    let id = e.open_space(&d, 0, &[1, 1, 4]).expect("resolves").expect("the entry opens");
    let trace = e.space(id).expect("just opened").trace();
    trace.check_tiles().expect("the steps tile the run");
    assert_eq!(trace.blocks().len(), 2, "this entry is packed into two blocks");
}

/// That the trace tiles the run: every bit of every block accounted for once,
/// and every byte out attributed to the step that wrote it.
#[test]
fn the_trace_covers_every_bit_of_the_packed_run() {
    let Some((d, mut e)) = open("rar5-one-file.rar") else {
        eprintln!("skipped: no rar5-one-file.rar in hand. Set QUBERO_SAMPLES to the collection.");
        return;
    };
    let id = e.open_space(&d, 0, &[1, 1, 4]).expect("resolves").expect("the entry opens");
    let trace = e.space(id).expect("just opened").trace();
    trace.check_tiles().expect("the steps tile the run");
    assert!(!trace.coarse(), "1200 bytes is nowhere near the step budget");
    assert!(trace.len() > 100, "a per-symbol trace of 1200 bytes has more than {} steps", trace.len());
    // The packed run is 361 bytes and the trace reads all of them.
    assert_eq!(trace.in_bits(), 361 * 8);
    assert_eq!(trace.out_bytes(), 1200);
}

fn find(name: &str) -> Option<PathBuf> {
    for dir in dirs() {
        let mut found = None;
        collect(&dir, 3, name, &mut found);
        if found.is_some() {
            return found;
        }
    }
    None
}

fn dirs() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(named) = std::env::var("QUBERO_SAMPLES") {
        out.extend(named.split(';').filter(|s| !s.is_empty()).map(PathBuf::from));
    }
    out.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../qubero-samples"));
    out.retain(|p| p.is_dir());
    out
}

fn collect(dir: &Path, depth: u32, name: &str, out: &mut Option<PathBuf>) {
    if out.is_some() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let path = e.path();
        if path.is_dir() {
            if depth > 0 {
                collect(&path, depth - 1, name, out);
            }
        } else if path.file_name().is_some_and(|f| f == name) {
            *out = Some(path);
            return;
        }
    }
}
