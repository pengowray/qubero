use super::*;

// ----- reads the browser's chunk store can never answer -----
//
// The browser holds the file in a chunk store that keeps a fixed number of
// chunks and drops the one read longest ago. A read that needs more chunks at
// once than that pushes out, as each chunk it asked for arrives, one it needs,
// and would ask for those again for ever. Each test here is a document over a
// small store, fed exactly the chunks each `Pending` names, as the browser
// feeds it.

use std::collections::BTreeMap;

use crate::codec::{Codec, Refusal};
use crate::source::ChunkStore;

const CHUNK: u64 = 1024;

/// A document over `bytes` in a store that keeps `capacity` chunks of 1 KiB,
/// with nothing fed yet.
fn stored(bytes: &[u8], capacity: usize) -> Document<ChunkStore> {
    Document::new(ChunkStore::new(bytes.len() as u64, CHUNK, capacity))
}

/// `ask`, asked again until it does not answer `Pending`, with exactly the
/// chunks each `Pending` names fed in between. A chunk may be asked for
/// again once it has been pushed out by later ones, as the head of a file is
/// when a walk has gone on past the store; the same chunk asked for many
/// times over fails the test, since that is the wait that never ends.
fn fed<T>(d: &mut Document<ChunkStore>, bytes: &[u8], mut ask: impl FnMut(&Document<ChunkStore>) -> R<T>) -> R<T> {
    let mut asked: BTreeMap<u64, usize> = BTreeMap::new();
    loop {
        match ask(d) {
            Err(EvalError::Pending(missing)) => {
                for m in missing {
                    let times = asked.entry(m.chunk).or_default();
                    *times += 1;
                    assert!(*times <= 3, "chunk {} asked for {times} times", m.chunk);
                    let from = (m.chunk * CHUNK) as usize;
                    let to = (from + CHUNK as usize).min(bytes.len());
                    d.source_mut().insert(m.chunk, bytes[from..to].to_vec().into_boxed_slice());
                }
            }
            other => return other,
        }
    }
}

/// Bytes that do not pack, so a compressed run of them is as long as they
/// are.
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

/// A two-byte tag and then a run of `len` bytes packed with `codec`, read as
/// bytes.
fn one_run(len: usize, codec: Codec) -> Template {
    Template::new(
        "t",
        T::structure("Root", vec![("tag", T::u16(Big)), ("run", T::decoded(E::lit(len as i128), codec, T::bytes(E::Remaining)))]),
    )
}

#[test]
fn a_read_longer_than_the_store_fails_with_the_reason_and_a_shorter_one_waits() {
    let bytes = noise(8 * 1024);
    let d = stored(&bytes, 4);
    let ev = Evaluator::new(Template::new("t", T::structure("Root", vec![("all", T::bytes(E::Remaining))])));
    match ev.read_in(&d, 0, 0, 5 * 1024 * 8) {
        Err(EvalError::Failed(why)) => assert_eq!(why, "Can't load 5 KiB of the file at once: the editor keeps at most 4 KiB in memory"),
        other => panic!("{other:?}"),
    }
    match ev.read_in(&d, 0, 1024 * 8, 4 * 1024 * 8) {
        Err(EvalError::Pending(missing)) => assert_eq!(missing.len(), 4),
        other => panic!("{other:?}"),
    }
    // A store with a source behind it reads that source, and has no limit.
    let behind = Document::new(ChunkStore::behind(Box::new(crate::source::MemSource(bytes.clone())), CHUNK));
    assert_eq!(ev.read_in(&behind, 0, 0, 5 * 1024 * 8).unwrap(), bytes[..5 * 1024]);
}

/// A zlib run longer than the store is refused before any of it is asked
/// for, and the refusal is kept. The same run in memory opens.
#[test]
fn a_run_longer_than_the_store_is_refused_before_it_is_read() {
    let packed = miniz_oxide::deflate::compress_to_vec_zlib(&noise(6 * 1024), 6);
    let mut bytes = vec![0xaa, 0xbb];
    bytes.extend_from_slice(&packed);
    let mut d = stored(&bytes, 4);
    let mut ev = Evaluator::new(one_run(packed.len(), Codec::Zlib));
    let run = fed(&mut d, &bytes, |d| ev.node(d, &[1])).unwrap();
    assert_eq!((run.refused.as_deref(), run.child_count), (Some("too-much-at-once"), 0));
    assert_eq!(ev.spaces.run_refusal((0, 16, packed.len() as u64 * 8), Codec::Zlib), Some(Refusal::TooMuchAtOnce));
    // Only the chunk with the tag in it was asked for.
    assert!((1..6).all(|c| !d.source().has(c)));
    let mut ev = Evaluator::new(one_run(packed.len(), Codec::Zlib));
    assert_eq!(ev.node(&doc(&bytes), &[1]).unwrap().refused, None);
}

/// The same for a codec that is never indexed, which is read whole by
/// another arm of the code: base64, whose `A` is six zero bits.
#[test]
fn a_run_of_a_codec_that_cannot_be_indexed_is_refused_the_same_way() {
    let mut bytes = vec![0xaa, 0xbb];
    bytes.extend(std::iter::repeat_n(b'A', 6 * 1024));
    let mut d = stored(&bytes, 4);
    let mut ev = Evaluator::new(one_run(6 * 1024, Codec::Base64));
    let run = fed(&mut d, &bytes, |d| ev.node(d, &[1])).unwrap();
    assert_eq!((run.refused.as_deref(), run.child_count), (Some("too-much-at-once"), 0));
    let mut d = stored(&bytes, 8);
    let mut ev = Evaluator::new(one_run(6 * 1024, Codec::Base64));
    assert_eq!(fed(&mut d, &bytes, |d| ev.node(d, &[1])).unwrap().refused, None, "room for the run");
}

/// A zlib stream cut into pages of the file and joined, the way a PNG's IDAT
/// chunks are: the run is in a stitched space, and reading it reads every
/// page one after another in one call. Each page fits in the store, and all
/// of them do not.
#[test]
fn a_run_joined_from_pages_longer_than_the_store_is_refused() {
    let bytes = miniz_oxide::deflate::compress_to_vec_zlib(&noise(6 * 1024), 6);
    let pages = bytes.len().div_ceil(1024);
    let template = |len: usize| {
        let page = T::bytes(E::lit(1024).at_most(E::Remaining));
        let inner = T::decoded(E::lit(len as i128), Codec::Zlib, T::bytes(E::Remaining));
        Template::new(
            "pages",
            T::structure(
                "Root",
                vec![("pages", T::array(page, E::lit(pages as i128))), ("joined", T::stitched(vec![Step::field("pages"), Step::each()], None, None, inner))],
            ),
        )
    };
    let mut d = stored(&bytes, 4);
    let mut ev = Evaluator::new(template(bytes.len()));
    let run = fed(&mut d, &bytes, |d| ev.node(d, &[1, 0])).unwrap();
    assert_ne!(run.space, 0);
    assert_eq!((run.refused.as_deref(), run.child_count), (Some("too-much-at-once"), 0));
    let mut d = stored(&bytes, 8);
    let mut ev = Evaluator::new(template(bytes.len()));
    assert_eq!(fed(&mut d, &bytes, |d| ev.node(d, &[1, 0])).unwrap().refused, None, "room for every page");
}

/// A stitched stream whose one part is packed with a codec that is never
/// indexed, and longer than the store: a read of the stream fails naming the
/// part, as a part too large for the cap does.
#[test]
fn a_packed_part_longer_than_the_store_says_so_when_read() {
    let mut bytes = vec![0x00, 0x00, 0x18, 0x00];
    bytes.extend(std::iter::repeat_n(b'A', 8 * 1024));
    let member = T::structure("Member", vec![("unpacked", T::u32(Big)), ("run", T::decoded(E::Remaining, Codec::Base64, T::bytes(E::Remaining)))]);
    let joined = T::stitched(vec![Step::field("member"), Step::field("run")], Some(E::field("unpacked")), None, T::bytes(E::Remaining));
    let mut ev = Evaluator::new(Template::new("base64-part", T::structure("File", vec![("member", member), ("joined", joined)])));
    let mut d = stored(&bytes, 4);
    let crate::eval::space::Opened::Space(space) = fed(&mut d, &bytes, |d| ev.open_stitched_at(d, &[1])).unwrap() else {
        panic!("the stream did not open")
    };
    match fed(&mut d, &bytes, |d| ev.read_in(d, space, 0, 8)) {
        Err(EvalError::Failed(why)) => assert_eq!(why, "member.run is too large for the editor to keep in memory at once"),
        other => panic!("{other:?}"),
    }
}

/// Pages stored as they sit, `count` of 1 KiB, and the stream they join.
fn paged(count: usize) -> Template {
    Template::new(
        "paged",
        T::structure(
            "Root",
            vec![
                ("pages", T::array(T::bytes(E::lit(1024)), E::lit(count as i128))),
                ("joined", T::stitched(vec![Step::field("pages"), Step::each()], None, None, T::bytes(E::Remaining))),
            ],
        ),
    )
}

/// Opening a joined stream as a document reads every part in one call. Each
/// part fits in the store, so it is the parts added up that are refused, for
/// a stream held whole and for one read a part at a time.
#[test]
fn a_joined_stream_whose_parts_add_up_to_more_than_the_store_is_refused() {
    let bytes = noise(6 * 1024);
    for whole_cap in [64 << 20, 5 * 1024] {
        let mut d = stored(&bytes, 4);
        let mut ev = Evaluator::new(paged(6));
        ev.spaces.set_whole_cap(whole_cap);
        assert_eq!(fed(&mut d, &bytes, |d| ev.open_space(d, 0, &[1])).unwrap(), None);
        // A stream past the cap is too large for that before it is too much
        // for the store, as it is anywhere it is read.
        let why = if whole_cap < bytes.len() { Refusal::TooLarge } else { Refusal::TooMuchAtOnce };
        assert_eq!(ev.open_refusal(&[1]), Some(why), "whole cap {whole_cap}");
        // With room for every part, it opens.
        let mut d = stored(&bytes, 8);
        let mut ev = Evaluator::new(paged(6));
        ev.spaces.set_whole_cap(whole_cap);
        let opened = fed(&mut d, &bytes, |d| ev.open_space(d, 0, &[1])).unwrap();
        assert_eq!(opened.is_some(), whole_cap > bytes.len(), "whole cap {whole_cap}");
    }
}

/// BGZF blocks that unpack to four times what they take up in the file, read
/// a part at a time because the stream is past the cap, and refused because
/// their runs add up to more than the store keeps.
#[test]
fn a_joined_stream_read_a_part_at_a_time_is_refused_by_what_its_runs_take_up() {
    use crate::formats::bam::tests::{bgzf_block, EOF_BLOCK};
    // Two bits of noise a byte, which packs to about a quarter.
    let pieces: Vec<Vec<u8>> = (0..6).map(|i| noise(4096 + i).iter().map(|b| b & 3).collect()).collect();
    let mut bytes: Vec<u8> = pieces.iter().flat_map(|p| bgzf_block(p)).collect();
    bytes.extend_from_slice(&EOF_BLOCK);
    assert!(bytes.len() > 4 * 1024 && bytes.len() < 12 * 1024, "{} bytes of blocks", bytes.len());
    let mut d = stored(&bytes, 4);
    let mut ev = Evaluator::new(crate::formats::bgzf());
    ev.spaces.set_whole_cap(16 * 1024);
    assert_eq!(fed(&mut d, &bytes, |d| ev.open_space(d, 0, &[1])).unwrap(), None);
    assert_eq!(ev.open_refusal(&[1]), Some(Refusal::TooMuchAtOnce));
    let mut d = stored(&bytes, 16);
    let mut ev = Evaluator::new(crate::formats::bgzf());
    ev.spaces.set_whole_cap(16 * 1024);
    assert!(fed(&mut d, &bytes, |d| ev.open_space(d, 0, &[1])).unwrap().is_some(), "room for every block");
}

/// A gzip with one member, as the bytes of a .gz file.
fn gzip_member(content: &[u8]) -> Vec<u8> {
    let mut gz = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 3];
    gz.extend(miniz_oxide::deflate::compress_to_vec(content, 6));
    gz.extend(crate::checksum::crc32(content).to_le_bytes());
    gz.extend((content.len() as u32).to_le_bytes());
    gz
}

/// A gzip member is measured by inflating from where its run starts, in one
/// read, which in the browser reads at most half the store. A member that
/// ends inside that is measured as anywhere; one that does not is taken to
/// run up to the trailer at the end of the file, which is right for a file of
/// one member and wrong for a member followed by others.
#[test]
fn a_gzip_member_is_measured_from_at_most_half_the_store() {
    // Three small members, each measured where it ends.
    let pieces: Vec<Vec<u8>> = (0..3).map(|i| noise(700 + i)).collect();
    let bytes: Vec<u8> = pieces.iter().flat_map(|p| gzip_member(p)).collect();
    let mut d = stored(&bytes, 4);
    let mut ev = Evaluator::new(crate::formats::gzip());
    assert_eq!(fed(&mut d, &bytes, |d| ev.node(d, &[0])).unwrap().child_count, 3);
    let mut at = 10;
    for (i, piece) in pieces.iter().enumerate() {
        let run = fed(&mut d, &bytes, |d| ev.node(d, &[0, i, 10])).unwrap();
        let len = gzip_member(piece).len() as u64 - 18;
        assert_eq!((run.offset_bits / 8, run.size_bits / 8), (at, len), "member {i}");
        at += len + 18;
    }

    // One member longer than half the store and then a short one, the whole
    // file well inside the store: the first is measured as nothing, and the
    // template's fallback takes it to the last trailer, so the file reads as
    // one member. The same file in memory is measured as two.
    let bytes: Vec<u8> = [gzip_member(&noise(2560)), gzip_member(&noise(300))].concat();
    assert!(bytes.len() < 3 * 1024, "{} bytes", bytes.len());
    let mut d = stored(&bytes, 4);
    let mut ev = Evaluator::new(crate::formats::gzip());
    assert_eq!(fed(&mut d, &bytes, |d| ev.node(d, &[0])).unwrap().child_count, 1);
    let run = fed(&mut d, &bytes, |d| ev.node(d, &[0, 0, 10])).unwrap();
    assert_eq!(run.size_bits / 8, bytes.len() as u64 - 18);
    let mut ev = Evaluator::new(crate::formats::gzip());
    assert_eq!(ev.node(&doc(&bytes), &[0]).unwrap().child_count, 2);
}
