//! A 7z archive whose header 7z compressed into a stream of its own, read as
//! its contents.
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
//! are, in a file that says them out loud.

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
