//! A compressed Godot resource, read against the plain one it was made from.
//!
//! `godot4-probe-compressed.res` and `godot4-probe.res` are the same resource
//! saved twice, once with `FLAG_COMPRESS` and once without, so the one thing
//! worth asserting about the compressed reading is that it says what the plain
//! one says. A test that only checked the block opened would pass on a stream
//! that unpacked into nonsense.
//!
//! The fixtures live in `QUBERO_SAMPLES/godot`, or in the sibling
//! `qubero-samples` collection. With neither this says so and passes.

use std::path::PathBuf;

use qubero_core::{document::Document, eval::Evaluator, eval::NodeInfo, eval::Value, formats, source::MemSource};

fn collection() -> Option<PathBuf> {
    let mut roots = Vec::new();
    if let Ok(paths) = std::env::var("QUBERO_SAMPLES") {
        roots.extend(paths.split(';').filter(|s| !s.is_empty()).map(PathBuf::from));
    }
    roots.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../qubero-samples"));
    roots.into_iter().map(|p| p.join("godot")).find(|p| p.is_dir())
}

fn reading(path: &PathBuf) -> (Document<MemSource>, Evaluator) {
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    (Document::new(MemSource(bytes)), Evaluator::new(formats::builtin("godot").unwrap()))
}

/// Where a plain resource keeps the field the compressed one keeps four bytes
/// earlier: `RSRC`, `endianness`, `file` against `endianness`, `file`.
///
/// The compressed side is the block itself (`blocks[0]`) and then its first
/// child, which is what came out of it.
const PLAIN_BODY: &[usize] = &[1];
const PACKED_BODY: &[usize] = &[5, 0, 0, 0];

/// The magic in front of a plain resource, which is the whole of the
/// difference between the two files. Every offset the internal resource table
/// holds is this much larger in the plain copy, because the saver counts the
/// bytes it wrote and the compressed one was never handed a magic to write.
const MAGIC_BYTES: u64 = 4;

#[test]
fn a_compressed_resource_reads_the_same_fields_as_the_plain_one() {
    let Some(dir) = collection() else {
        eprintln!("skipped: set QUBERO_SAMPLES to the sample collection");
        return;
    };
    let plain_path = dir.join("godot4-probe.res");
    let packed_path = dir.join("godot4-probe-compressed.res");
    if !plain_path.is_file() || !packed_path.is_file() {
        eprintln!("skipped: no godot4 probe pair in {}", dir.display());
        return;
    }
    let (plain_doc, mut plain) = reading(&plain_path);
    let (packed_doc, mut packed) = reading(&packed_path);

    // The block opened at all, and into as many bytes as the wrapper said.
    let total = packed.node(&packed_doc, &[3]).unwrap().value;
    let stream = packed.node(&packed_doc, &[5, 0, 0]).unwrap();
    assert_eq!(stream.type_name, "GodotResource");
    assert_eq!(Value::UInt((stream.size_bits / 8).into()), total);
    assert_ne!(stream.space, 0, "the resource should be read in the stream's own space");
    // And it is the plain file minus its magic, which is what the wrapper
    // stands in for.
    assert_eq!(stream.size_bits / 8 + MAGIC_BYTES, plain_doc.len_bytes());

    // Now every field of it, against the same field of the plain file.
    let mut nodes = 0usize;
    let mut offsets = 0usize;
    same(
        &mut plain,
        &plain_doc,
        &mut PLAIN_BODY.to_vec(),
        &mut packed,
        &packed_doc,
        &mut PACKED_BODY.to_vec(),
        "",
        &mut nodes,
        &mut offsets,
    );
    // The walk started at `endianness`; `file` is its sibling, and the whole
    // resource hangs off that.
    let mut plain_file = vec![2];
    let mut packed_file = vec![5, 0, 0, 1];
    same(
        &mut plain,
        &plain_doc,
        &mut plain_file,
        &mut packed,
        &packed_doc,
        &mut packed_file,
        "",
        &mut nodes,
        &mut offsets,
    );
    // A probe file carrying one property of every variant type is a few
    // thousand nodes; a walk that compared six of them and stopped would pass
    // as loudly as one that worked.
    assert!(nodes > 1000, "only {nodes} nodes compared");
    assert!(offsets > 0, "no internal resource offsets were compared");
    eprintln!("{nodes} nodes read alike, {offsets} of them table offsets");
}

/// The same node in both readings, and then the same of every child.
///
/// `parent` is the type the two nodes sit in, which is what tells an offset
/// that counts from the front of its own copy from any other number.
#[allow(clippy::too_many_arguments)]
fn same(
    plain: &mut Evaluator,
    plain_doc: &Document<MemSource>,
    at: &mut Vec<usize>,
    packed: &mut Evaluator,
    packed_doc: &Document<MemSource>,
    to: &mut Vec<usize>,
    parent: &str,
    nodes: &mut usize,
    offsets: &mut usize,
) {
    let a = plain.node(plain_doc, at).unwrap_or_else(|e| panic!("plain {at:?}: {e:?}"));
    let b = packed.node(packed_doc, to).unwrap_or_else(|e| panic!("packed {to:?}: {e:?}"));
    *nodes += 1;
    let what = || format!("plain {at:?} against packed {to:?} ({})", a.name);
    assert_eq!(a.name, b.name, "name: {}", what());
    assert_eq!(a.type_name, b.type_name, "type: {}", what());
    assert_eq!(a.size_bits, b.size_bits, "size: {}", what());
    assert_eq!(a.child_count, b.child_count, "children: {}", what());
    assert_eq!(a.refused, b.refused, "refusal: {}", what());
    if parent == "InternalResource" && a.name == "offset" {
        // The one number that is meant to differ, and by exactly the magic the
        // plain file has in front of it. Asserted rather than skipped: it is
        // the evidence that the origin inside the stream sits at the front of
        // the stream and not four bytes before it.
        *offsets += 1;
        let (Value::UInt(x), Value::UInt(y)) = (&a.value, &b.value) else {
            panic!("a table offset should be a number: {}", what());
        };
        assert_eq!(*x, *y + u128::from(MAGIC_BYTES), "table offset: {}", what());
    } else {
        assert_eq!(a.value, b.value, "value: {}", what());
    }
    // Every child of both, in step. The counts already matched.
    for i in 0..a.child_count as usize {
        at.push(i);
        to.push(i);
        same(plain, plain_doc, at, packed, packed_doc, to, &a.type_name, nodes, offsets);
        at.pop();
        to.pop();
    }
}

/// A PackedByteArray holds whatever the variant types have no word for, and
/// reading it as a run of bytes is the one answer that cannot be wrong and
/// cannot be useful. The probe's is five bytes, so what matters here is that
/// the run became a space at all and that the space holds those five bytes.
#[test]
fn a_packed_byte_array_opens_as_a_document_of_its_own() {
    let Some(dir) = collection() else {
        eprintln!("skipped: set QUBERO_SAMPLES to the sample collection");
        return;
    };
    let path = dir.join("godot4-probe.res");
    if !path.is_file() {
        eprintln!("skipped: no godot4-probe.res in {}", dir.display());
        return;
    }
    let (doc, mut ev) = reading(&path);
    let arrays = find(&mut ev, &doc, "PackedByteArray");
    assert!(!arrays.is_empty(), "the probe carries one property of every type");
    for at in &arrays {
        let count = ev.node(&doc, &[at.clone(), vec![0]].concat()).unwrap().value;
        let run = ev.node(&doc, &[at.clone(), vec![1]].concat()).unwrap();
        assert_eq!(run.type_name, "stored", "a byte array's values should be a stream");
        assert_eq!(run.refused, None, "the run should open: {at:?}");
        assert_eq!(Value::UInt((run.size_bits / 8).into()), count);
        // One child, which is what came out, read in a space of its own.
        assert_eq!(run.child_count, 1);
        let inside = ev.node(&doc, &[at.clone(), vec![1, 0]].concat()).unwrap();
        assert_ne!(inside.space, 0, "the contents belong to the stream's space");
        assert_eq!(inside.offset_bits, 0, "and count from the front of it");
        assert_eq!(inside.size_bits, run.size_bits);
    }
}

/// Every node of a given type, by walking the whole reading.
fn find(ev: &mut Evaluator, doc: &Document<MemSource>, type_name: &str) -> Vec<Vec<usize>> {
    let mut found = Vec::new();
    let mut stack = vec![Vec::new()];
    while let Some(at) = stack.pop() {
        let Ok(node): Result<NodeInfo, _> = ev.node(doc, &at) else { continue };
        if node.type_name == type_name {
            found.push(at.clone());
        }
        for i in (0..node.child_count as usize).rev() {
            let mut next = at.clone();
            next.push(i);
            stack.push(next);
        }
    }
    found
}
