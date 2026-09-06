//! The pickles in the sample collection, read as the programs they are.
//!
//! `pickle/` holds two real-world files, the same fixture dumped at every
//! protocol from 0 to 5, and one file each for the corners no ordinary object
//! reaches: an extension code, a persistent id, an out-of-band buffer, a
//! payload too large to frame, and the opcodes CPython 3 reads and never
//! writes. Between them they use all sixty-eight opcodes, which is what makes
//! this worth running against files rather than against a fixture here.
//!
//! What it checks is the thing a pickle can be checked on and few formats can:
//! **the opcodes cover every byte, exactly**. A pickle has no padding, no
//! alignment and no directory, so the run of opcodes either lands on the full
//! stop or it does not, and an operand read one byte too wide puts every
//! opcode after it on the wrong byte. Adding the sizes up is the whole test.
//!
//! Like the rest of the collection this skips when the folder is not beside
//! the repository. Point `QUBERO_SAMPLES` at it to run it elsewhere.

use std::path::{Path, PathBuf};

use qubero_core::document::Document;
use qubero_core::eval::{Evaluator, Value};
use qubero_core::formats;
use qubero_core::source::{ChunkStore, MemSource};

#[test]
fn every_pickle_is_opcodes_all_the_way_to_the_full_stop() {
    let Some(dir) = folder() else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    let mut checked = 0;
    for path in pickles(&dir) {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let bytes = std::fs::read(&path).unwrap();

        assert_eq!(
            formats::sniff(&bytes[..bytes.len().min(0x9000)], bytes.len() as u64),
            Some("pickle"),
            "{name}: not recognised"
        );

        let doc = Document::new(MemSource(bytes.clone()));
        let mut ev = Evaluator::new(formats::builtin("pickle").unwrap());
        let root = ev.node(&doc, &[]).unwrap();
        assert_eq!(root.offset_bits, 0, "{name}");

        let mut ops = Vec::new();
        gather(&doc, &mut ev, &[], &mut ops, 0);
        assert!(!ops.is_empty(), "{name}: no opcodes");

        // Every byte belongs to exactly one opcode. Frames hold opcodes of
        // their own, so only the opcodes holding no others are counted, and
        // they have to come to the length of the file with nothing over.
        let mut covered: Vec<(u64, u64)> = ops.iter().map(|(_, at, size)| (*at, *size)).collect();
        covered.sort_unstable();
        let mut want = 0u64;
        for (at, size) in &covered {
            assert_eq!(*at, want, "{name}: a gap or an overlap before bit {at}");
            want = at + size;
        }
        assert_eq!(want, doc.len_bits(), "{name}: the opcodes do not reach the end of the file");

        let last = ops.last().unwrap();
        assert_eq!(last.0, "STOP", "{name}: the last opcode is {} rather than STOP", last.0);

        checked += 1;
        eprintln!("{name}: {} opcodes over {} bytes", ops.len(), doc.len_bits() / 8);
    }
    assert!(checked >= 10, "only {checked} pickles found; the folder is meant to hold more");
}

/// A pickle written at protocol 4 says so, and the first opcode is where it
/// says it. Worth its own check because `PROTO` is the one opcode a reader can
/// compare against something outside the file: the sample's own name.
#[test]
fn the_protocol_a_file_was_written_at_is_the_one_it_says() {
    let Some(dir) = folder() else { return };
    let mut checked = 0;
    for path in pickles(&dir) {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let Some(want) = name.strip_prefix("proto").and_then(|s| s.as_bytes().first()) else { continue };
        let want = i128::from(want - b'0');
        let doc = Document::new(MemSource(std::fs::read(&path).unwrap()));
        let mut ev = Evaluator::new(formats::builtin("pickle").unwrap());

        let first = ev.node(&doc, &[0]).unwrap();
        if want < 2 {
            assert_ne!(opcode(&first.name), "PROTO", "{name}: protocol {want} has no PROTO opcode");
            checked += 1;
            continue;
        }
        assert_eq!(opcode(&first.name), "PROTO", "{name}: does not open with PROTO");
        assert_eq!(ev.node(&doc, &[0, 1]).unwrap().value, Value::UInt(want as u128), "{name}: wrong protocol");
        checked += 1;
    }
    assert!(checked >= 6, "only {checked} named for their protocol");
}

/// A pickled array is read as the numbers it holds, not as the bytes they are
/// written in.
///
/// This is the whole point of running the program. A `BINBYTES` says nothing
/// about its contents; the dtype, the shape and the byte order are three other
/// opcodes, and they reach the data only across the unpickler's stack. So what
/// is checked here is that the crossing happened: the payload row of every
/// numpy and pandas sample is an array of the right element type, and the
/// object array, whose data is not a buffer at all, is left as bytes.
#[test]
fn an_array_is_read_as_the_numbers_it_holds() {
    let Some(dir) = folder() else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    let mut checked = 0;
    for path in pickles(&dir) {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let Some(rest) = ["proto4-numpy-", "proto3-numpy-", "proto5-pandas-"]
            .iter()
            .find_map(|prefix| name.strip_prefix(prefix))
        else {
            continue;
        };
        let doc = Document::new(MemSource(std::fs::read(&path).unwrap()));
        let mut ev = Evaluator::new(formats::builtin("pickle").unwrap());
        let mut typed = Vec::new();
        payloads(&doc, &mut ev, &[], &mut typed, 0);
        checked += 1;

        // An object array's data is a list of pickled objects rather than a
        // buffer, so nothing should have claimed it.
        if rest.starts_with("object-array") {
            assert!(typed.is_empty(), "{name}: an object array read as {typed:?}");
            continue;
        }
        assert!(!typed.is_empty(), "{name}: no array was read as one");

        if rest.starts_with("array") {
            assert_eq!(typed, vec![("f32 le".to_string(), 24)], "{name}");
        }
        // Every width numpy has, so a reader with one of them wrong fails
        // here rather than quietly reading half an array.
        if rest.starts_with("dtypes") {
            let widths: Vec<&str> = typed.iter().map(|(t, _)| t.as_str()).collect();
            for want in ["i8", "u8", "i16 le", "u16 le", "i32 le", "u32 le", "i64 le", "u64 le", "f16 le", "f32 le", "f64 le"] {
                assert!(widths.contains(&want), "{name}: no {want} among {widths:?}");
            }
        }
        if rest.starts_with("byte-order") {
            let widths: Vec<&str> = typed.iter().map(|(t, _)| t.as_str()).collect();
            assert!(widths.contains(&"f64 be"), "{name}: the big-endian column stayed little: {widths:?}");
            assert!(widths.contains(&"f64 le"), "{name}: {widths:?}");
        }
        // numpy 1 called the module `numpy.core`, and a pickle from before
        // 2024 is the commoner kind.
        if rest.starts_with("1-module-names") {
            assert_eq!(typed, vec![("f64 le".to_string(), 4)], "{name}");
        }
        eprintln!("{name}: {typed:?}");
    }
    assert!(checked >= 8, "only {checked} array samples found");
}

/// A file that has not all arrived yet is not run as a program.
///
/// The machine reads the whole file, and in the browser a file arrives a chunk
/// at a time. Running it over the chunks that have turned up, with zeros where
/// the rest will be, would give an answer about a file nobody has: a shape
/// read out of zeros, a length that happens to agree, an array of nothing. And
/// the answer is remembered, so it would still be wrong once the bytes landed.
///
/// So the run goes through the same read every field goes through, and says
/// "not yet" the same way. This feeds a real sample in one chunk at a time and
/// checks that the array reads as an array only once the last of it is there.
#[test]
fn a_file_still_arriving_is_not_run_as_a_program() {
    let Some(dir) = folder() else { return };
    let path = dir.join("proto4-numpy-array.pickle");
    let Ok(bytes) = std::fs::read(&path) else { return };

    const CHUNK: u64 = 64;
    let mut doc = Document::new(ChunkStore::new(bytes.len() as u64, CHUNK, 64));
    let mut ev = Evaluator::new(formats::builtin("pickle").unwrap());
    let chunks = bytes.len().div_ceil(CHUNK as usize);

    for n in 0..chunks {
        let from = n * CHUNK as usize;
        let to = (from + CHUNK as usize).min(bytes.len());
        doc.source_mut().insert(n as u64, bytes[from..to].to_vec().into_boxed_slice());
        let mut typed = Vec::new();
        chunked_payloads(&doc, &mut ev, &[], &mut typed, 0);
        // Nothing may be claimed until the file is whole, and everything must
        // be claimed once it is.
        match n + 1 == chunks {
            false => assert!(typed.is_empty(), "read {typed:?} from {} of {chunks} chunks", n + 1),
            true => assert_eq!(typed, vec![("f32 le".to_string(), 24)], "the whole file"),
        }
    }
}

/// The same walk as [`payloads`], over a source that may still be fetching.
/// A node that is not there yet is not a node that read as bytes, so the walk
/// simply stops there.
fn chunked_payloads(
    doc: &Document<ChunkStore>,
    ev: &mut Evaluator,
    at: &[usize],
    out: &mut Vec<(String, u64)>,
    depth: usize,
) {
    if depth > 10 {
        return;
    }
    let Ok(node) = ev.node(doc, at) else { return };
    if node.name == "value" && node.composite && node.type_name != "bytes[]" && node.type_name.ends_with("[]") {
        out.push((node.type_name.trim_end_matches("[]").to_string(), node.child_count));
        return;
    }
    for i in 0..node.child_count as usize {
        let mut next = at.to_vec();
        next.push(i);
        chunked_payloads(doc, ev, &next, out, depth + 1);
    }
}

/// Every payload that was read as an array rather than as bytes, as the
/// element's type and how many of them there are.
fn payloads(
    doc: &Document<MemSource>,
    ev: &mut Evaluator,
    at: &[usize],
    out: &mut Vec<(String, u64)>,
    depth: usize,
) {
    if depth > 10 {
        return;
    }
    let Ok(node) = ev.node(doc, at) else { return };
    // The bytes of a byte string, which the template gives the length of the
    // opcode and the machine gives a type to. Bytes that stayed bytes are not
    // counted: `bytes[]` is the default and means nothing was recognised.
    if node.name == "value" && node.composite && node.type_name != "bytes[]" && node.type_name.ends_with("[]") {
        let elem = node.type_name.trim_end_matches("[]").to_string();
        out.push((elem, node.child_count));
        return;
    }
    for i in 0..node.child_count as usize {
        let mut next = at.to_vec();
        next.push(i);
        payloads(doc, ev, &next, out, depth + 1);
    }
}

/// Every opcode in the file, in the order the bytes are in, as its name, where
/// it starts and how long it is. A frame contributes the opcodes inside it and
/// not itself, since those are the ones covering bytes.
fn gather(
    doc: &Document<MemSource>,
    ev: &mut Evaluator,
    at: &[usize],
    out: &mut Vec<(String, u64, u64)>,
    depth: usize,
) {
    assert!(depth < 8, "nested past anything a pickle does: {at:?}");
    let node = ev.node(doc, at).unwrap_or_else(|e| panic!("{at:?}: {e:?}"));
    if node.type_name == "Op" {
        if opcode(&node.name) != "FRAME" {
            out.push((opcode(&node.name).to_string(), node.offset_bits, node.size_bits));
            return;
        }
        // A frame covers its own header and then the opcodes inside it. The
        // header is what is left of the frame once its contents are taken off:
        // the opcode byte and the eight-byte length.
        let inside: Vec<usize> = at.iter().copied().chain([1, 1]).collect();
        let ops = ev.node(doc, &inside).unwrap_or_else(|e| panic!("{inside:?}: {e:?}"));
        out.push(("FRAME".to_string(), node.offset_bits, node.size_bits - ops.size_bits));
        gather(doc, ev, &inside, out, depth + 1);
        return;
    }
    for i in 0..node.child_count as usize {
        let mut next = at.to_vec();
        next.push(i);
        gather(doc, ev, &next, out, depth + 1);
    }
}

/// An element of a list is named for what it is with its index in front of it,
/// which is `[3] BININT1`. The opcode is the rest.
fn opcode(name: &str) -> &str {
    name.rsplit_once("] ").map_or(name, |(_, rest)| rest)
}

fn folder() -> Option<PathBuf> {
    let named = std::env::var_os("QUBERO_SAMPLES").map(PathBuf::from);
    let beside = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../qubero-samples");
    named.into_iter().chain(std::iter::once(beside)).map(|p| p.join("pickle")).find(|p| p.is_dir())
}

fn pickles(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "pickle"))
        .collect();
    out.sort();
    out
}
