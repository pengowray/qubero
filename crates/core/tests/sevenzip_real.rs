//! A 7z archive read as what it holds: the header 7z packed away, and the
//! files the header describes.
//!
//! The collection is not in this repository: point `QUBERO_SAMPLES` at it, or
//! keep it beside the checkout as `qubero-samples`. With neither, this says so
//! and passes.
//!
//! The same archive is in the collection three times: `nested-dirs-solid.7z`
//! and `nested-dirs-nonsolid.7z` write their headers in the clear, and
//! `nested-dirs-header-lzma.7z` is what 7-Zip writes by default, with the
//! header packed into a stream the file describes and nothing else. So there
//! is an answer to check against that no template produced: what the names
//! are, in a file that says them out loud, and what the files hold, in an
//! archive that packs each of them on its own.
//!
//! All three pack with LZMA2, which is what 7-Zip reaches for unless told
//! otherwise, and pack the header itself with LZMA1.

use std::path::{Path, PathBuf};

use qubero_core::checksum;
use qubero_core::document::Document;
use qubero_core::eval::Evaluator;
use qubero_core::formats;
use qubero_core::source::{MemSource, Source};

/// Where the file table's names are, from a `kHeader`: its `kFilesInfo`, the
/// blocks of that, and the `kName` among them.
///
/// Searched for rather than counted to, because which blocks a file table
/// holds is up to the archiver: the two headers compared here do not put
/// `kName` at the same index, and one that also wrote times or attributes
/// would move it again.
fn names_in<S: Source>(e: &mut Evaluator, d: &Document<S>, header: &[usize]) -> Vec<String> {
    let blocks = [header, &[3, 2]].concat();
    let n = e.node(d, &blocks).expect("a file table").child_count;
    for i in 0..n as usize {
        let id = e.node(d, &[blocks.as_slice(), &[i, 0]].concat()).expect("a block tag");
        if id.value.as_int() != Some(0x11) {
            continue;
        }
        // The block, its body, the value inside it, and the names inside that.
        let list = [blocks.as_slice(), &[i, 1, 1, 1]].concat();
        let count = e.node(d, &list).expect("the names").child_count;
        return (0..count as usize)
            .map(|j| match e.node(d, &[list.as_slice(), &[j]].concat()).expect("a name").value {
                qubero_core::eval::Value::Str(s) => s.to_string(),
                other => panic!("a name read as {other:?}"),
            })
            .collect();
    }
    panic!("no kName block in the file table");
}

/// What the whole exercise is for. The names in an archive whose header is
/// compressed are the names in the same archive written in the clear, and the
/// only way to have them is to have unpacked the header.
#[test]
fn a_compressed_header_reads_as_the_names_the_plain_one_says_out_loud() {
    let (Some(packed), Some(plain), Some(solid)) = (
        find("nested-dirs-header-lzma.7z"),
        find("nested-dirs-nonsolid.7z"),
        find("nested-dirs-solid.7z"),
    ) else {
        eprintln!("skipped: no nested-dirs 7z samples in hand. Set QUBERO_SAMPLES to the collection.");
        return;
    };

    // The answer, from the archive that says it in the clear.
    let bytes = std::fs::read(&plain).expect("reads");
    let d = Document::new(MemSource(bytes));
    let mut e = Evaluator::new(formats::builtin("7z").expect("7z"));
    let want = names_in(&mut e, &d, &[8]);
    assert_eq!(
        want,
        ["docs", "docs/notes", "img", "empty.dat", "docs/chapter one.md", "docs/notes/deep.txt", "img/blob.bin", "readme.txt"],
        "the plain header is not the archive this expected"
    );

    // And the same archive with its header packed away.
    let bytes = std::fs::read(&packed).expect("reads");
    let d = Document::new(MemSource(bytes));
    let mut e = Evaluator::new(formats::builtin("7z").expect("7z"));
    // Out here the file is a run of bytes and one stream, and that stream is
    // the header: the encoded header, its streams info, its unpack info, and
    // the size the folder comes out at.
    let unpacked = e.node(&d, &[8, 1, 1, 6, 0]).expect("kCodersUnPackSize").value.as_int().expect("a number");
    let id = e.open_space(&d, 0, &[7, 2, 0]).expect("no error").expect("the header stream opens");

    let space = e.space(id).expect("it is there");
    assert_eq!(space.len_bytes() as i128, unpacked, "the space is as long as kCodersUnPackSize said");
    // The folder digest is over what came out of the folder, so it is a check
    // on the decoding that the file itself wrote down.
    let stored = e.node(&d, &[8, 1, 1, 7, 2, 0]).expect("the folder kCRC").value.as_int().expect("a number");
    assert_eq!(checksum::crc32(e.space(id).unwrap().bytes()) as i128, stored, "kCRC of the unpacked folder");
    // The solid sibling is this archive with the header left in the clear, so
    // what came out of the stream is that file's header, byte for byte.
    let solid = std::fs::read(&solid).expect("reads");
    let at = u64::from_le_bytes(solid[12..20].try_into().unwrap()) as usize + 32;
    assert_eq!(e.space(id).unwrap().bytes(), &solid[at..], "the unpacked header is the plain one");

    // The point: the space reads as a header, and its file table names the
    // same eight entries.
    let space = e.space_mut(id).expect("it is there");
    let (se, sd) = space.reading();
    assert_eq!(names_in(se, sd, &[]), want);
}

/// The four files that have bytes, in the order the archives pack them, which
/// is the order `kCodersUnPackSize` puts their folders in: the two documents,
/// the blob, and the readme.
///
/// Written out here rather than taken from an archive. What the streams unpack
/// to has to be checked against something no reading of a 7z produced, and
/// this is that thing: the same four values `tools/make_7z_samples.py` in the
/// sample collection writes before it calls the archiver.
fn contents() -> Vec<Vec<u8>> {
    vec![
        "# Chapter One\n\nA paragraph with enough text to compress.\n".repeat(12).into_bytes(),
        "nested file, repeated line\n".repeat(40).into_bytes(),
        (0..2048u32).map(|i| ((i * 37) % 251) as u8).collect(),
        b"Qubero sample archive. The quick brown fox jumps over the lazy dog. 0123456789.".to_vec(),
    ]
}

/// What the whole exercise is for, one level further in: the front of the file
/// stops being four runs of compressed bytes and becomes the four files.
///
/// Checked three ways, because "something opened" is not the claim. The bytes
/// are the bytes the sample builder wrote; they match the CRC-32 7-Zip itself
/// put in `kSubStreamsInfo`, which is a number no reader here produced; and
/// the solid archive, which holds the same files as one block, unpacks to the
/// four of them run together in the same order.
#[test]
fn a_folder_opens_as_the_files_it_packed() {
    let (Some(nonsolid), Some(solid)) = (find("nested-dirs-nonsolid.7z"), find("nested-dirs-solid.7z")) else {
        eprintln!("skipped: no nested-dirs 7z samples in hand. Set QUBERO_SAMPLES to the collection.");
        return;
    };
    let want = contents();

    // One folder per file, so one packed stream per file.
    let bytes = std::fs::read(&nonsolid).expect("reads");
    let d = Document::new(MemSource(bytes));
    let mut e = Evaluator::new(formats::builtin("7z").expect("7z"));
    assert_eq!(e.node(&d, &[7, 2]).expect("the streams").child_count as usize, want.len());
    for (i, file) in want.iter().enumerate() {
        let id = e.open_space(&d, 0, &[7, 2, i]).expect("no error").unwrap_or_else(|| panic!("stream {i} opens"));
        assert_eq!(e.space(id).expect("it is there").bytes(), &file[..], "stream {i}");
        // And the archive says the same, in a digest it wrote itself. One
        // file to a folder means one substream to a folder, so these are in
        // the same order as the streams.
        let crc = e.node(&d, &[8, 2, 3, 5, 2, i]).expect("a substream kCRC").value.as_int().expect("a number");
        assert_eq!(i128::from(checksum::crc32(file)), crc, "stream {i} against the kCRC 7-Zip wrote");
    }

    // The same files packed as one solid block. There is one stream and it
    // opens as all four at once: where one of them stops is in the substream
    // sizes, and laying those over the unpacked run is the thing that cannot
    // be said. So the bytes are checked whole, and then at the boundaries the
    // header does name.
    let bytes = std::fs::read(&solid).expect("reads");
    let d = Document::new(MemSource(bytes));
    let mut e = Evaluator::new(formats::builtin("7z").expect("7z"));
    assert_eq!(e.node(&d, &[7, 2]).expect("the streams").child_count, 1, "one folder, one stream");
    let id = e.open_space(&d, 0, &[7, 2, 0]).expect("no error").expect("the solid folder opens");
    assert_eq!(e.space(id).expect("it is there").bytes(), &want.concat()[..]);
    let mut at = 0;
    for (i, file) in want.iter().enumerate() {
        let crc = e.node(&d, &[8, 2, 3, 5, 2, i]).expect("a substream kCRC").value.as_int().expect("a number");
        let run = &e.space(id).expect("it is there").bytes()[at..at + file.len()];
        assert_eq!(i128::from(checksum::crc32(run)), crc, "the solid folder's file {i}");
        at += file.len();
    }
    assert_eq!(at, e.space(id).expect("it is there").len_bytes() as usize, "and nothing left over");
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

fn collect(dir: &Path, depth: u32, name: &str, found: &mut Option<PathBuf>) {
    if found.is_some() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let path = e.path();
        if path.is_dir() {
            if depth > 0 {
                collect(&path, depth - 1, name, found);
            }
        } else if path.file_name().is_some_and(|f| f == name) {
            *found = Some(path);
            return;
        }
    }
}

/// The `kCRC` a `kSubStreamsInfo` writes is one sum per file, and where every
/// folder holds one file it is a sum of that folder's whole output. So the
/// non-solid archive declares one per stream and they pass; the solid one,
/// whose four files share a folder, declares none at all.
///
/// The guard is the point. A solid folder's substream *i* is a slice of one
/// output taken at an offset, and summing the whole folder against one file's
/// number would call every solid archive broken.
#[test]
fn substream_sums_are_declared_where_a_folder_holds_one_file_and_not_otherwise() {
    for (name, want) in [("nested-dirs-nonsolid.7z", 4usize), ("nested-dirs-solid.7z", 0)] {
        let Some(path) = find(name) else {
            eprintln!("skipped: no {name} (set QUBERO_SAMPLES)");
            return;
        };
        let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let doc = Document::new(MemSource(bytes));
        let mut e = Evaluator::new(formats::builtin("7z").expect("the 7z template"));
        let mut found = 0;
        let mut walk = vec![Vec::new()];
        while let Some(at) = walk.pop() {
            if at.len() > 10 {
                continue;
            }
            let Ok(n) = e.node(&doc, &at) else { continue };
            // An element of a list, which is what a substream sum is. The two
            // sums at the front of the file are named fields and are counted
            // by the sweep, not here.
            if n.name.starts_with('[') && matches!(e.check_of(&doc, &at), Ok(Some(_))) {
                found += 1;
                let v = e.run_check(&doc, &at).unwrap().expect("a declared sum has a verdict");
                assert!(v.ok, "{name} at {at:?}: computed {} stored {}", v.computed, v.stored);
            }
            for i in 0..n.child_count as usize {
                walk.push([at.clone(), vec![i]].concat());
            }
        }
        assert_eq!(found, want, "{name}: substream sums declared");
    }
}
