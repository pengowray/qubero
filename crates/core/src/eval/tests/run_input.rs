use super::*;

// ----- where an index reads its compressed run from -----
//
// An index made by the evaluator reads its run, once the pass is over,
// through a handle that copies nothing whenever one reaches the run: onto the
// file's source for a run of the file that no edit touched, onto a space's
// bytes, or onto the source of a space read through an index. Only a run no
// handle reaches keeps a copy. The pass reads a copy of its own either way,
// so a handle at the wrong place would not show until the index is read: each
// test here reads the stream through the index after the pass, at places its
// span cache no longer holds, and compares every byte.

use std::sync::Arc;

use crate::codec::lazy::{Limits, Seekable};
use crate::codec::{Codec, Refusal};
use crate::eval::space::{Opened, RunKey};
use crate::source::ArcSource;

/// Spans of 16 KiB of output, with four of them kept decoded, and runs held
/// whole up to 64 KiB.
fn small() -> Limits {
    Limits { whole: 64 << 10, spacing: 16 << 10, checkpoints: 32, span_cache: 64 << 10, ..Default::default() }
}

/// Numbered lines of text, which pack to about a sixth of their length.
fn lines(n: usize) -> Vec<u8> {
    (0..n).flat_map(|i| format!("line {i} of a run read through a handle\n").into_bytes()).collect()
}

/// Bytes that do not pack.
fn noise(n: usize) -> Vec<u8> {
    let mut x = 0x2545_f491_u32;
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x as u8
        })
        .collect()
}

fn zlib(content: &[u8]) -> Vec<u8> {
    miniz_oxide::deflate::compress_to_vec_zlib(content, 6)
}

/// `head` bytes of anything, then a zlib run of `len` bytes, read as bytes.
fn after(head: usize, len: usize) -> Template {
    Template::new(
        "t",
        T::structure("Root", vec![("head", T::bytes(E::lit(head as i128))), ("run", T::decoded(E::lit(len as i128), Codec::Zlib, T::bytes(E::Remaining)))]),
    )
}

/// A two-byte tag, then a zlib run of `outer` bytes whose stream is another
/// two-byte tag and a zlib run of `inner` bytes.
fn nested(outer: usize, inner: usize) -> Template {
    let inside = T::structure("Inner", vec![("tag", T::u16(Big)), ("run", T::decoded(E::lit(inner as i128), Codec::Zlib, T::bytes(E::Remaining)))]);
    Template::new("t", T::structure("Root", vec![("tag", T::u16(Big)), ("outer", T::decoded(E::lit(outer as i128), Codec::Zlib, inside))]))
}

fn reading(template: Template) -> Evaluator {
    let mut ev = Evaluator::new(template);
    ev.spaces.set_lazy_limits(small());
    ev
}

/// Open the stream at `path`, and say which space it is, the index it is
/// read through, and where its run is.
fn opened<S: Source>(ev: &mut Evaluator, d: &Document<S>, path: &[usize]) -> (u32, Arc<Seekable>, RunKey) {
    let run = ev.node(d, path).unwrap();
    ev.node(d, &[path, &[0]].concat()).unwrap();
    assert_eq!(ev.node(d, path).unwrap().refused, None);
    let Some(Opened::Space(id)) = ev.spaces.get(path) else { panic!("the stream at {path:?} did not open") };
    let index = ev.spaces.lazy(id).expect("read through an index, not held whole").clone();
    (id, index, (run.space, run.offset_bits, run.size_bits))
}

/// Every byte of the stream in space `id` is `content`, read through its
/// index a few spans apart, the first of them after the pass has moved on
/// from it, and once all at once.
fn reads_back<S: Source>(ev: &Evaluator, d: &Document<S>, id: u32, index: &Seekable, content: &[u8]) {
    assert!(index.spans() > 3, "{} spans", index.spans());
    for at in [0, content.len() / 3, content.len() / 2, content.len() - 100] {
        let got = ev.read_in(d, id, at as u64 * 8, 800).unwrap();
        assert!(got == content[at..at + 100], "100 bytes at {at} differ");
    }
    let mut all = vec![0u8; content.len()];
    index.read(0, &mut all).unwrap();
    assert!(all == content, "the whole stream differs");
}

/// A run of the file, two bytes in, read through a handle onto the file's
/// source: the index holds no copy of it. A file in a `MemSource`, which
/// gives no handle, is read through a copy of the run, with the same bytes.
#[test]
fn a_run_of_the_file_is_read_through_a_handle_onto_the_file() {
    let content = lines(8_000);
    let run = zlib(&content);
    let file = [vec![0xaa, 0xbb], run.clone()].concat();
    let d = Document::new(ArcSource(Arc::new(file.clone())));
    let mut ev = reading(after(2, run.len()));
    let (id, index, key) = opened(&mut ev, &d, &[1]);
    assert_eq!(key, (0, 16, run.len() as u64 * 8));
    assert_eq!((index.input().len(), index.input().owned_bytes()), (run.len() as u64, 0));
    assert!(index.input().held_copy().is_none());
    reads_back(&ev, &d, id, &index, &content);

    let d = Document::new(MemSource(file));
    let mut ev = reading(after(2, run.len()));
    let (id, index, _) = opened(&mut ev, &d, &[1]);
    assert_eq!(index.input().owned_bytes(), run.len() as u64);
    reads_back(&ev, &d, id, &index, &content);
}

/// Bytes inserted before the run, and a byte deleted before it, move it in
/// the document and not in the file: the handle reads the file from where
/// the run is there. An overwrite inside the run, even with the byte that
/// was there, leaves no handle to read it through, and the run is copied.
#[test]
fn an_edit_moves_the_run_or_has_it_copied() {
    let content = lines(8_000);
    let run = zlib(&content);
    let file = [vec![0xaa, 0xbb], run.clone()].concat();
    let mut d = Document::new(ArcSource(Arc::new(file.clone())));
    d.insert_bytes(1, b"abcd");
    d.delete_bytes(0, 1);
    let mut ev = reading(after(5, run.len()));
    let (id, index, key) = opened(&mut ev, &d, &[1]);
    assert_eq!(key.1, 5 * 8);
    assert_eq!(index.input().owned_bytes(), 0);
    reads_back(&ev, &d, id, &index, &content);

    let mut d = Document::new(ArcSource(Arc::new(file.clone())));
    d.overwrite_bytes(1000, &file[1000..1001]);
    let mut ev = reading(after(2, run.len()));
    let (id, index, _) = opened(&mut ev, &d, &[1]);
    assert_eq!(index.input().owned_bytes(), run.len() as u64, "the edited run is copied");
    reads_back(&ev, &d, id, &index, &content);
}

/// A run inside a stream held whole is read from the stream's bytes, two
/// bytes into them, whatever the file is held in.
#[test]
fn a_run_in_a_stream_held_whole_is_read_from_its_bytes() {
    let content = lines(8_000);
    let inner = zlib(&content);
    let outer = zlib(&[vec![0xcc, 0xdd], inner.clone()].concat());
    assert!(inner.len() + 2 < 64 << 10, "the outer stream is held whole");
    let d = Document::new(MemSource([vec![0xaa, 0xbb], outer.clone()].concat()));
    let mut ev = reading(nested(outer.len(), inner.len()));
    let (id, index, (space, at, _)) = opened(&mut ev, &d, &[1, 0, 1]);
    assert!(ev.spaces.buf(space).is_some(), "the outer stream is held whole");
    assert_eq!(at, 16);
    assert_eq!(index.input().owned_bytes(), 0);
    reads_back(&ev, &d, id, &index, &content);
}

/// A run inside a stream read through an index is read through that
/// stream's source, which decodes the outer run as the inner one is read.
/// Neither index holds a copy when the file gives a handle.
#[test]
fn a_run_in_a_stream_read_through_an_index_is_read_through_that_stream() {
    let content = noise(100 << 10);
    let inner = zlib(&content);
    let outer = zlib(&[vec![0xcc, 0xdd], inner.clone()].concat());
    let d = Document::new(ArcSource(Arc::new([vec![0xaa, 0xbb], outer.clone()].concat())));
    let mut ev = reading(nested(outer.len(), inner.len()));
    let (id, index, (space, at, _)) = opened(&mut ev, &d, &[1, 0, 1]);
    let outer_index = ev.spaces.lazy(space).expect("the outer stream is read through an index").clone();
    assert_eq!(at, 16);
    assert_eq!((index.input().owned_bytes(), outer_index.input().owned_bytes()), (0, 0));
    reads_back(&ev, &d, id, &index, &content);
}

/// A run joined from parts of the file is read through a copy, since no one
/// place in the file holds it.
#[test]
fn a_run_joined_from_parts_is_read_through_a_copy() {
    // Two parts of the same length, so that they are one list of two.
    let (content, run) = (8_000..)
        .map(|n| {
            let content = lines(n);
            let run = zlib(&content);
            (content, run)
        })
        .find(|(_, run)| run.len() % 2 == 0)
        .unwrap();
    let half = run.len() / 2;
    let part = T::decoded(E::lit(half as i128), Codec::Stored, T::bytes(E::Remaining));
    let joined = T::decoded(E::Remaining, Codec::Zlib, T::bytes(E::Remaining));
    let template = Template::new(
        "t",
        T::structure("Root", vec![("parts", T::array(part, E::lit(2))), ("joined", T::stitched(vec![Step::field("parts"), Step::elements(&[0, 1])], None, None, joined))]),
    );
    let d = Document::new(ArcSource(Arc::new(run.clone())));
    let mut ev = reading(template);
    let (id, index, (space, at, _)) = opened(&mut ev, &d, &[1, 0]);
    assert!(ev.spaces.stitch(space).is_some(), "the run is in a stitched space");
    assert_eq!(at, 0);
    assert_eq!(index.input().owned_bytes(), run.len() as u64);
    reads_back(&ev, &d, id, &index, &content);
}

/// The evaluator keeps the indexes it makes up to a total weight, and an
/// index over a handle weighs what its checkpoints and block starts hold,
/// not nothing: three runs read through handles, with room for the weight of
/// two, leave the last two kept. Each stream still reads, through the index
/// its space holds. In memory each index also weighs its copy of the run, so
/// with room for two indexes over handles only the last is kept.
#[test]
fn indexes_over_handles_are_weighed_and_the_oldest_goes() {
    let contents: Vec<Vec<u8>> = (0..3).map(|i| lines(8_000 + i)).collect();
    let runs: Vec<Vec<u8>> = contents.iter().map(|c| zlib(c)).collect();
    let fields: Vec<(String, T)> = runs.iter().enumerate().map(|(i, r)| (format!("run{i}"), T::decoded(E::lit(r.len() as i128), Codec::Zlib, T::bytes(E::Remaining)))).collect();
    let template = Template::new("t", T::structure("Root", fields.iter().map(|(n, t)| (n.as_str(), t.clone())).collect()));
    let file = runs.concat();
    let d = Document::new(ArcSource(Arc::new(file.clone())));
    let mut ev = reading(template.clone());
    let (_, first, _) = opened(&mut ev, &d, &[0]);
    let weight = first.kept_bytes();
    assert!(weight > runs[0].len() as u64 && first.input().owned_bytes() == 0, "{weight} bytes kept");
    // Room for two indexes over handles, and not for two over copies.
    let cap = 2 * weight + runs[0].len() as u64;
    ev.spaces.set_index_cap(cap);
    let mut keys = Vec::new();
    for i in 0..3 {
        let (id, index, key) = opened(&mut ev, &d, &[i]);
        keys.push(key);
        reads_back(&ev, &d, id, &index, &contents[i]);
    }
    assert_eq!(ev.spaces.kept_indexes(), keys[1..]);

    let d = Document::new(MemSource(file));
    let mut ev = reading(template);
    ev.spaces.set_index_cap(cap);
    let mut keys = Vec::new();
    for i in 0..3 {
        let (id, index, key) = opened(&mut ev, &d, &[i]);
        assert_eq!(index.input().owned_bytes(), runs[i].len() as u64);
        keys.push(key);
        reads_back(&ev, &d, id, &index, &contents[i]);
    }
    assert_eq!(ev.spaces.kept_indexes(), keys[2..]);
}

/// A copy longer than the cap is refused before anything is read. A handle
/// is never refused for its length: the cap on the run is the pass's.
#[test]
fn a_copy_longer_than_the_cap_is_refused() {
    let cap = crate::codec::CAP_BYTES as u64;
    let ev = reading(after(0, 16));
    let key = (0, 0, (cap + 1) * 8);
    let small = Document::new(MemSource(vec![0; 16]));
    assert!(matches!(ev.run_input(&small, key), Ok(Err(Refusal::TooLarge))));
    let large = Document::new(ArcSource(Arc::new(vec![0; cap as usize + 1])));
    let Ok(Ok(input)) = ev.run_input(&large, key) else { panic!("no handle") };
    assert_eq!((input.len(), input.owned_bytes()), (cap + 1, 0));
}
