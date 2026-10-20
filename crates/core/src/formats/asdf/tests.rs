use super::*;
use crate::document::Document;
use crate::eval::{Evaluator, Value};
use crate::source::MemSource;

/// A block as the standard lays it out, with no checksum.
fn block(compression: &[u8; 4], stored: &[u8], data_size: usize, flags: u32) -> Vec<u8> {
    let mut b = BLOCK_MAGIC.to_vec();
    b.extend_from_slice(&48u16.to_be_bytes());
    b.extend_from_slice(&flags.to_be_bytes());
    b.extend_from_slice(compression);
    let (used, data) = if flags & 1 == 1 { (0, 0) } else { (stored.len() as u64, data_size as u64) };
    b.extend_from_slice(&used.to_be_bytes());
    b.extend_from_slice(&used.to_be_bytes());
    b.extend_from_slice(&data.to_be_bytes());
    b.extend_from_slice(&[0; 16]);
    b.extend_from_slice(stored);
    b
}

/// asdf's lz4: chunks of a big-endian length, then python-lz4's block with its
/// little-endian unpacked size in front.
fn lz4(data: &[u8], chunk: usize) -> Vec<u8> {
    let mut out = Vec::new();
    for part in data.chunks(chunk) {
        let packed = lz4_flex::block::compress(part);
        out.extend_from_slice(&(packed.len() as u32 + 4).to_be_bytes());
        out.extend_from_slice(&(part.len() as u32).to_le_bytes());
        out.extend_from_slice(&packed);
    }
    out
}

const TREE: &str = "%YAML 1.1\n%TAG ! tag:stsci.edu:asdf/\n--- !core/asdf-1.1.0\n\
    image: !core/ndarray-1.1.0\n  source: 0\n  datatype: float32\n  byteorder: little\n  shape: [2, 3]\n\
    packed: !core/ndarray-1.1.0\n  source: 1\n  datatype: int16\n  byteorder: big\n  shape: [600]\n\
    ...\n";

fn file() -> Vec<u8> {
    let mut b = b"#ASDF 1.0.0\n#ASDF_STANDARD 1.6.0\n".to_vec();
    b.extend_from_slice(TREE.as_bytes());
    let image: Vec<u8> = [0.5f32, 1.0, 1.5, 2.0, 2.5, 3.0].iter().flat_map(|v| v.to_le_bytes()).collect();
    b.extend_from_slice(&block(b"\0\0\0\0", &image, image.len(), 0));
    let packed: Vec<u8> = (0..600i16).flat_map(|v| (v % 7).to_be_bytes()).collect();
    // Two chunks, so the array is read across the join.
    b.extend_from_slice(&block(b"lz4\0", &lz4(&packed, 1000), packed.len(), 0));
    b.extend_from_slice(b"#ASDF BLOCK INDEX\n%YAML 1.1\n---\n- 0\n...\n");
    b
}

fn eval(bytes: Vec<u8>) -> (Document<MemSource>, Evaluator) {
    (Document::new(MemSource(bytes)), Evaluator::new(asdf()))
}

/// The node down this run of field names and list indices.
fn at(ev: &mut Evaluator, d: &Document<MemSource>, names: &[&str]) -> Vec<usize> {
    let mut path = Vec::new();
    for n in names {
        match n.parse::<usize>() {
            Ok(i) => path.push(i),
            Err(_) => path = ev.child_named(d, &path, n).unwrap().unwrap_or_else(|| panic!("no {n} under {path:?}")),
        }
    }
    path
}

#[test]
fn the_tree_runs_from_the_directive_to_the_end_of_its_document() {
    let (d, mut ev) = eval(file());
    let comments = ev.node(&d, &[1]).unwrap();
    assert_eq!(comments.child_count, 1);
    let tree = ev.node(&d, &[2]).unwrap();
    assert_eq!(tree.size_bits / 8, TREE.len() as u64);
    assert_eq!(ev.node(&d, &[3]).unwrap().size_bits, 0);
}

#[test]
fn an_uncompressed_block_reads_as_the_array_the_tree_names() {
    let (d, mut ev) = eval(file());
    assert_eq!(ev.node(&d, &[4]).unwrap().child_count, 2);
    let arrays = at(&mut ev, &d, &["blocks", "0", "data"]);
    let image = ev.node(&d, &[arrays.as_slice(), &[0]].concat()).unwrap();
    assert_eq!(image.name, "image");
    assert_eq!(image.child_count, 2);
    let last = ev.node(&d, &[arrays.as_slice(), &[0, 1, 2]].concat()).unwrap();
    assert_eq!(last.value, Value::Float(3.0));
}

#[test]
fn an_lz4_block_is_its_chunks_joined() {
    let (d, mut ev) = eval(file());
    let chunks = at(&mut ev, &d, &["blocks", "1", "data", "chunks"]);
    assert_eq!(ev.node(&d, &chunks).unwrap().child_count, 2);
    let packed = at(&mut ev, &d, &["blocks", "1", "data", "1", "0", "packed"]);
    let n = ev.node(&d, &packed).unwrap();
    assert_eq!(n.child_count, 600);
    // Element 500 is past the first chunk's thousand bytes.
    let v = ev.node(&d, &[packed.as_slice(), &[500]].concat()).unwrap();
    assert_eq!(v.value, Value::Int(500 % 7));
}

#[test]
fn a_streamed_block_runs_to_the_end_of_the_file() {
    let tree = "%YAML 1.1\n--- !<tag:stsci.edu:asdf/core/asdf-1.1.0>\n\
        rows: !<tag:stsci.edu:asdf/core/ndarray-1.1.0> {source: -1, datatype: uint8, byteorder: little, shape: ['*', 2]}\n...\n";
    let mut b = b"#ASDF 1.0.0\n".to_vec();
    b.extend_from_slice(tree.as_bytes());
    b.extend_from_slice(&block(b"\0\0\0\0", &[1, 2, 3, 4, 5, 6], 0, 1));
    let (d, mut ev) = eval(b);
    let rows = at(&mut ev, &d, &["blocks", "0", "data", "rows"]);
    assert_eq!(ev.node(&d, &rows).unwrap().child_count, 3);
    let v = ev.node(&d, &[rows.as_slice(), &[2, 1]].concat()).unwrap();
    assert_eq!(v.value, Value::UInt(6));
}

#[test]
fn a_tree_longer_than_the_text_shown_still_says_how_long_it_is() {
    // A Roman tree is 50 KB, and only the first 4 KB of a text field is read.
    let tree = format!("%YAML 1.1\n---\nnote: '{}'\n...\n", "x".repeat(20_000));
    let mut b = b"#ASDF 1.0.0\n".to_vec();
    b.extend_from_slice(tree.as_bytes());
    let (d, mut ev) = eval(b);
    let n = ev.node(&d, &[2]).unwrap();
    assert_eq!(n.size_bits / 8, tree.len() as u64);
    assert_eq!(n.value_bytes, tree.len() as u64);
}
