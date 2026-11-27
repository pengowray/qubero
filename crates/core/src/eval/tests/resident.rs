use super::*;

// ----- runs longer than the browser's chunk store -----
//
// The browser holds the file in a chunk store that keeps a fixed number of
// chunks and drops the one read longest ago. A read that needs more chunks at
// once than that pushes out, as each chunk it asked for arrives, one it needs,
// and would ask for those again for ever. So a whole run is read a window at
// a time into a copy kept from one call to the next, and only a read that
// still has to be made in one call is refused. Each test here is a document
// over a small store, fed exactly the chunks each `Pending` names, as the
// browser feeds it.

use std::collections::BTreeMap;

use crate::codec::{Codec, Refusal};
use crate::eval::space::Opened;
use crate::source::ChunkStore;

const CHUNK: u64 = 1024;

/// A document over `bytes` in a store that keeps `capacity` chunks of 1 KiB,
/// with nothing fed yet.
fn stored(bytes: &[u8], capacity: usize) -> Document<ChunkStore> {
    Document::new(ChunkStore::new(bytes.len() as u64, CHUNK, capacity))
}

fn feed(d: &mut Document<ChunkStore>, bytes: &[u8], chunk: u64) {
    let from = (chunk * CHUNK) as usize;
    let to = (from + CHUNK as usize).min(bytes.len());
    d.source_mut().insert(chunk, bytes[from..to].to_vec().into_boxed_slice());
}

/// `ask`, asked again until it does not answer `Pending`, with exactly the
/// chunks each `Pending` names fed in between. A chunk may be asked for
/// again once it has been pushed out by later ones, as the head of a file is
/// when a walk has gone on past the store; the same chunk asked for many
/// times over fails the test, since that is the wait that never ends.
fn fed<T>(d: &mut Document<ChunkStore>, bytes: &[u8], ask: impl FnMut(&Document<ChunkStore>) -> R<T>) -> R<T> {
    fed_noting(d, bytes, &mut Vec::new(), ask)
}

/// The same, keeping the chunks each `Pending` named in `asked`.
fn fed_noting<T>(d: &mut Document<ChunkStore>, bytes: &[u8], asked: &mut Vec<Vec<u64>>, mut ask: impl FnMut(&Document<ChunkStore>) -> R<T>) -> R<T> {
    let mut times: BTreeMap<u64, usize> = BTreeMap::new();
    loop {
        match ask(d) {
            Err(EvalError::Pending(missing)) => {
                asked.push(missing.iter().map(|m| m.chunk).collect());
                for m in missing {
                    let n = times.entry(m.chunk).or_default();
                    *n += 1;
                    assert!(*n <= 3, "chunk {} asked for {n} times", m.chunk);
                    feed(d, bytes, m.chunk);
                }
            }
            other => return other,
        }
    }
}

/// `ask` in goes of `slice` elements, the way the web asks: asked again on
/// `Busy`, and with exactly the chunks named fed on `Pending`. The same
/// chunk asked for many times over fails the test. Hands back the answer
/// and how many goes ran out.
fn in_goes<T>(ev: &mut Evaluator, d: &mut Document<ChunkStore>, bytes: &[u8], slice: u64, mut ask: impl FnMut(&mut Evaluator, &Document<ChunkStore>) -> R<T>) -> (R<T>, usize) {
    ev.set_slice(Some(slice));
    let mut times: BTreeMap<u64, usize> = BTreeMap::new();
    let mut busy = 0;
    loop {
        ev.begin_slice();
        match ask(ev, d) {
            Err(EvalError::Pending(missing)) => {
                for m in missing {
                    let n = times.entry(m.chunk).or_default();
                    *n += 1;
                    assert!(*n <= 3, "chunk {} asked for {n} times", m.chunk);
                    feed(d, bytes, m.chunk);
                }
            }
            Err(EvalError::Busy { .. }) => {
                busy += 1;
                assert!(busy < 100_000, "no end to the goes");
            }
            other => return (other, busy),
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

/// A two-byte tag and then runs of the lengths given, each packed with
/// `codec`, called `run0`, `run1` and on.
fn runs(lens: &[usize], codec: Codec) -> Template {
    let mut fields = vec![("tag".to_string(), T::u16(Big))];
    for (i, len) in lens.iter().enumerate() {
        fields.push((format!("run{i}"), T::decoded(E::lit(*len as i128), codec, T::bytes(E::Remaining))));
    }
    Template::new("t", T::structure("Root", fields.iter().map(|(n, t)| (n.as_str(), t.clone())).collect()))
}

/// What the stream the node at `path` opened unpacked to.
fn unpacked(ev: &Evaluator, path: &[usize]) -> Vec<u8> {
    let Some(Opened::Space(id)) = ev.spaces.get(path) else { panic!("the stream at {path:?} did not open") };
    ev.spaces.buf(id).expect("held whole").to_vec()
}

/// A read that still has to be made in one call, longer than the store,
/// fails with the reason: the guard every read of the file goes through. One
/// that fits waits for its chunks.
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

/// A run read a window at a time comes to the bytes one read gives, and a
/// gather that runs out of go or waits for bytes carries on from where it
/// stopped: each go here reads one window, a chunk of the four the store
/// keeps.
#[test]
fn a_run_gathered_across_goes_and_waits_comes_to_the_bytes_one_read_gives() {
    let bytes = noise(10 * 1024);
    let mut d = stored(&bytes, 4);
    let mut ev = Evaluator::new(one_run(0, Codec::Zlib));
    ev.set_slice(Some(1));
    let (mut busy, mut waits) = (0, 0);
    let got = loop {
        ev.begin_slice();
        match ev.gather_run(&d, 0, 100 * 8, 9000 * 8) {
            Ok(Ok(got)) => break got,
            Err(EvalError::Busy { .. }) => busy += 1,
            Err(EvalError::Pending(missing)) => {
                assert_eq!(missing.len(), 1, "a window is one chunk here: {missing:?}");
                feed(&mut d, &bytes, missing[0].chunk);
                waits += 1;
            }
            other => panic!("{other:?}"),
        }
        assert!(busy + waits < 100);
    };
    assert_eq!(got, bytes[100..9100]);
    assert_eq!(waits, 9, "chunks 0 to 8, each asked for once");
    assert!(busy >= 8, "{busy} goes ran out");
    assert!(!ev.spaces.any(), "the copy was handed over and nothing kept");
}

/// A zlib run longer than the store opens, read a window at a time, and
/// each wait names one window's chunks. The same run in memory comes to the
/// same bytes.
#[test]
fn a_run_longer_than_the_store_is_read_a_window_at_a_time() {
    let content = noise(6 * 1024);
    let packed = miniz_oxide::deflate::compress_to_vec_zlib(&content, 6);
    let mut bytes = vec![0xaa, 0xbb];
    bytes.extend_from_slice(&packed);
    let mut d = stored(&bytes, 4);
    let mut ev = Evaluator::new(one_run(packed.len(), Codec::Zlib));
    let mut asked = Vec::new();
    let run = fed_noting(&mut d, &bytes, &mut asked, |d| ev.node(d, &[1])).unwrap();
    assert_eq!(run.refused, None);
    assert!(run.child_count > 0);
    assert_eq!(unpacked(&ev, &[1]), content);
    assert!(asked.iter().all(|chunks| chunks.len() == 1), "{asked:?}");
    assert_eq!(asked.len(), bytes.len().div_ceil(1024), "each chunk once: {asked:?}");
    // In goes, a pass stepped across them reads its run into its own place.
    let mut d = stored(&bytes, 4);
    let mut ev = Evaluator::new(one_run(packed.len(), Codec::Zlib));
    let (run, _) = in_goes(&mut ev, &mut d, &bytes, 600, |ev, d| ev.node(d, &[1]));
    assert_eq!(run.unwrap().refused, None);
    assert_eq!(unpacked(&ev, &[1]), content);
}

/// The same for a codec that is never indexed, which is read whole by
/// another arm of the code: base64, whose `A` is six zero bits.
#[test]
fn a_run_of_a_codec_that_cannot_be_indexed_is_read_the_same_way() {
    let mut bytes = vec![0xaa, 0xbb];
    bytes.extend(std::iter::repeat_n(b'A', 6 * 1024));
    let mut d = stored(&bytes, 4);
    let mut ev = Evaluator::new(one_run(6 * 1024, Codec::Base64));
    let run = fed(&mut d, &bytes, |d| ev.node(d, &[1])).unwrap();
    assert_eq!(run.refused, None);
    assert_eq!(unpacked(&ev, &[1]), vec![0u8; 6 * 1024 / 4 * 3]);
}

/// A zlib stream cut into pages of the file and joined, the way a PNG's IDAT
/// chunks are: the run is in a stitched space, and each window of it reads
/// the pages it covers. Each page fits in the store, and all of them do not.
#[test]
fn a_run_joined_from_pages_longer_than_the_store_is_read_the_same_way() {
    let content = noise(6 * 1024);
    let bytes = miniz_oxide::deflate::compress_to_vec_zlib(&content, 6);
    let pages = bytes.len().div_ceil(1024);
    let page = T::bytes(E::lit(1024).at_most(E::Remaining));
    let inner = T::decoded(E::lit(bytes.len() as i128), Codec::Zlib, T::bytes(E::Remaining));
    let template = Template::new(
        "pages",
        T::structure(
            "Root",
            vec![("pages", T::array(page, E::lit(pages as i128))), ("joined", T::stitched(vec![Step::field("pages"), Step::each()], None, None, inner))],
        ),
    );
    let mut d = stored(&bytes, 4);
    let mut ev = Evaluator::new(template);
    let run = fed(&mut d, &bytes, |d| ev.node(d, &[1, 0])).unwrap();
    assert_ne!(run.space, 0);
    assert_eq!(run.refused, None);
    assert_eq!(unpacked(&ev, &[1, 0]), content);
}

/// A stitched stream whose one part is packed with a codec that is never
/// indexed, and longer than the store: a read of the stream reads the part's
/// run a window at a time and unpacks it.
#[test]
fn a_packed_part_longer_than_the_store_is_read() {
    let mut bytes = vec![0x00, 0x00, 0x18, 0x00];
    bytes.extend(std::iter::repeat_n(b'A', 8 * 1024));
    let member = T::structure("Member", vec![("unpacked", T::u32(Big)), ("run", T::decoded(E::Remaining, Codec::Base64, T::bytes(E::Remaining)))]);
    let joined = T::stitched(vec![Step::field("member"), Step::field("run")], Some(E::field("unpacked")), None, T::bytes(E::Remaining));
    let mut ev = Evaluator::new(Template::new("base64-part", T::structure("File", vec![("member", member), ("joined", joined)])));
    let mut d = stored(&bytes, 4);
    let Opened::Space(space) = fed(&mut d, &bytes, |d| ev.open_stitched_at(d, &[1])).unwrap() else { panic!("the stream did not open") };
    assert_eq!(fed(&mut d, &bytes, |d| ev.read_in(d, space, 0, 8)).unwrap(), [0]);
    assert_eq!(fed(&mut d, &bytes, |d| ev.read_in(d, space, (6 * 1024 - 1) * 8, 8)).unwrap(), [0]);
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

/// Opening a joined stream as a document reads every part, and the parts
/// add up to more than the store keeps. The walk over them keeps the parts
/// read so far from one call to the next, so it opens, held whole. Past the
/// cap it would be read a part at a time from copies, which would come to
/// more than the cap, so it is refused as too large before anything is read.
#[test]
fn a_joined_stream_whose_parts_add_up_to_more_than_the_store_opens() {
    let bytes = noise(6 * 1024);
    let mut d = stored(&bytes, 4);
    let mut ev = Evaluator::new(paged(6));
    let id = fed(&mut d, &bytes, |d| ev.open_space(d, 0, &[1])).unwrap().expect("the stream opened");
    assert_eq!(ev.space(id).unwrap().held(), Some(&bytes[..]));
    for (slice, capacity) in [(1, 4), (40, 4), (600, 2)] {
        let mut d = stored(&bytes, capacity);
        let mut ev = Evaluator::new(paged(6));
        let (opened, _) = in_goes(&mut ev, &mut d, &bytes, slice, |ev, d| ev.open_space(d, 0, &[1]));
        let id = opened.unwrap().expect("the stream opened");
        assert_eq!(ev.space(id).unwrap().held(), Some(&bytes[..]), "goes of {slice}, a store of {capacity}");
    }
    let mut d = stored(&bytes, 4);
    let mut ev = Evaluator::new(paged(6));
    ev.spaces.set_whole_cap(5 * 1024);
    assert_eq!(fed(&mut d, &bytes, |d| ev.open_space(d, 0, &[1])).unwrap(), None);
    assert_eq!(ev.open_refusal(&[1]), Some(Refusal::TooLarge));
    assert!((0..6).all(|c| !d.source().has(c)), "a page was read");
}

/// BGZF blocks that unpack to four times what they take up in the file, read
/// a part at a time because the stream is past the cap, and whose runs add
/// up to more than the store keeps. The walk opens them in goes, and the
/// joined stream reads the same bytes as the blocks hold.
#[test]
fn a_joined_stream_read_a_part_at_a_time_opens_over_a_small_store() {
    use crate::formats::bam::tests::{bgzf_block, EOF_BLOCK};
    // Two bits of noise a byte, which packs to about a quarter.
    let pieces: Vec<Vec<u8>> = (0..6).map(|i| noise(4096 + i).iter().map(|b| b & 3).collect()).collect();
    let mut bytes: Vec<u8> = pieces.iter().flat_map(|p| bgzf_block(p)).collect();
    bytes.extend_from_slice(&EOF_BLOCK);
    assert!(bytes.len() > 4 * 1024 && bytes.len() < 12 * 1024, "{} bytes of blocks", bytes.len());
    let joined: Vec<u8> = pieces.concat();
    for slice in [None, Some(100)] {
        let mut d = stored(&bytes, 4);
        let mut ev = Evaluator::new(crate::formats::bgzf());
        ev.spaces.set_whole_cap(16 * 1024);
        let opened = match slice {
            None => fed(&mut d, &bytes, |d| ev.open_space(d, 0, &[1])).unwrap(),
            Some(n) => in_goes(&mut ev, &mut d, &bytes, n, |ev, d| ev.open_space(d, 0, &[1])).0.unwrap(),
        };
        let id = opened.expect("the stream opened");
        let space = ev.space(id).unwrap();
        assert!(space.held().is_none(), "read a part at a time");
        let mut out = vec![0u8; joined.len()];
        assert!(space.source().read(0, &mut out).is_ok());
        assert_eq!(out, joined, "goes of {slice:?}");
    }
}

/// A gzip with one member, as the bytes of a .gz file.
fn gzip_member(content: &[u8]) -> Vec<u8> {
    let mut gz = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 3];
    gz.extend(miniz_oxide::deflate::compress_to_vec(content, 6));
    gz.extend(crate::checksum::crc32(content).to_le_bytes());
    gz.extend((content.len() as u32).to_le_bytes());
    gz
}

/// A gzip member is measured by inflating from where its run starts, reading
/// its bytes a window at a time as far as the stream goes. A member that
/// ends anywhere is measured there, so a member longer than the store and
/// one after it are two members, as in memory.
#[test]
fn a_gzip_member_longer_than_the_store_is_measured() {
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
    // file inside the store; and one longer than the whole store.
    for first in [2560, 6000] {
        let bytes: Vec<u8> = [gzip_member(&noise(first)), gzip_member(&noise(300))].concat();
        let mut d = stored(&bytes, 4);
        let mut ev = Evaluator::new(crate::formats::gzip());
        assert_eq!(fed(&mut d, &bytes, |d| ev.node(d, &[0])).unwrap().child_count, 2, "a member of {first}");
        let run = fed(&mut d, &bytes, |d| ev.node(d, &[0, 0, 10])).unwrap();
        assert_eq!(run.size_bits / 8, gzip_member(&noise(first)).len() as u64 - 18, "a member of {first}");
        let mut d = stored(&bytes, 4);
        let mut ev = Evaluator::new(crate::formats::gzip());
        let (members, _) = in_goes(&mut ev, &mut d, &bytes, 50, |ev, d| ev.node(d, &[0]));
        assert_eq!(members.unwrap().child_count, 2, "a member of {first}, in goes");
        let mut ev = Evaluator::new(crate::formats::gzip());
        assert_eq!(ev.node(&doc(&bytes), &[0]).unwrap().child_count, 2);
    }
}

/// A gzip member whose output passes the cap on what a measuring walk counts
/// is refused at the step that passes it, and the walk reads about a step
/// past there, not the rest of the file. The template then takes the member
/// to run to the trailer, and its stream opens. Here the cap is 64 KiB of
/// output, and the member is 1 MiB of bytes that do not pack.
#[test]
fn a_gzip_member_past_the_measuring_cap_is_refused_where_it_passes_the_cap() {
    let content = noise(1 << 20);
    let bytes = gzip_member(&content);
    let mut d = stored(&bytes, 4);
    let mut ev = Evaluator::new(crate::formats::gzip());
    ev.spaces.set_lazy_limits(crate::codec::lazy::Limits { measure: 64 << 10, ..Default::default() });
    let mut asked = Vec::new();
    assert_eq!(fed_noting(&mut d, &bytes, &mut asked, |d| ev.node(d, &[0])).unwrap().child_count, 1);
    let chunks: usize = asked.iter().map(Vec::len).sum();
    // The header, the first step's 193 KiB ahead, and the trailer, of the
    // 1,025 chunks of the file.
    assert!(chunks < 300, "measuring asked for {chunks} chunks");
    let run = fed(&mut d, &bytes, |d| ev.node(d, &[0, 0, 10])).unwrap();
    assert_eq!(run.size_bits / 8, bytes.len() as u64 - 18);
    assert_eq!(run.refused, None);
    assert_eq!(unpacked(&ev, &[0, 0, 10]), content);
}

// ----- streams read through an index from the store -----

/// Limits small enough that 160 KiB is past the cap: spans of 16 KiB of
/// output, and four of them kept decoded.
fn small_lazy() -> crate::codec::lazy::Limits {
    crate::codec::lazy::Limits { whole: 64 << 10, spacing: 16 << 10, checkpoints: 32, span_cache: 64 << 10, ..Default::default() }
}

/// A zlib file whose stream is a gzip file of 160 KiB of noise, as the file
/// and the gzip file. Neither packs, so the file's run is as long as the
/// gzip, about three times what the stores below keep.
fn zlib_of_a_gzip() -> (Vec<u8>, Vec<u8>) {
    let member = gzip_member(&noise(160 << 10));
    (miniz_oxide::deflate::compress_to_vec_zlib(&member, 1), member)
}

/// The zlib template's `compressed`.
const ZLIB_RUN: &[usize] = &[6];

/// A zlib run past the cap and longer than the store is read a window at a
/// time to index it, and the index then reads the run through a handle onto
/// the store: it holds no copy of the run, and keeps as much as its span
/// cache. A read of a span whose chunks the store has pushed out since waits
/// for them, and then gives the bytes the run unpacks to.
#[test]
fn a_run_longer_than_the_store_is_read_from_the_store_once_indexed() {
    // Each line with a number in hex that does not repeat, so that the run
    // packs to about half.
    let mut x = 0x9e37_79b9_7f4a_7c15_u64;
    let content: Vec<u8> = (0..12_000)
        .flat_map(|i| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            format!("line {i}: {x:016x}\n").into_bytes()
        })
        .collect();
    let packed = miniz_oxide::deflate::compress_to_vec_zlib(&content, 6);
    let mut bytes = vec![0xaa, 0xbb];
    bytes.extend_from_slice(&packed);
    assert!(bytes.len() > 96 * CHUNK as usize, "{} bytes is too few for a store of 48 chunks", bytes.len());
    let mut d = stored(&bytes, 48);
    let mut ev = Evaluator::new(one_run(packed.len(), Codec::Zlib));
    ev.spaces.set_lazy_limits(small_lazy());
    fed(&mut d, &bytes, |d| ev.node(d, &[1, 0])).unwrap();
    let Some(Opened::Space(id)) = ev.spaces.get(&[1]) else { panic!("the stream did not open") };
    let index = ev.spaces.lazy(id).expect("read through an index").clone();
    assert_eq!(index.input().owned_bytes(), 0, "the index holds a copy of the run");
    assert_eq!(index.resident_limit(), Some(64 << 10));
    let mut waited = 0;
    for at in [content.len() / 3, 10, content.len() / 2, content.len() - 100] {
        let mut asked = Vec::new();
        let got = fed_noting(&mut d, &bytes, &mut asked, |d| ev.read_in(d, id, at as u64 * 8, 800)).unwrap();
        assert!(got == content[at..at + 100], "100 bytes at {at} differ");
        waited += asked.len();
    }
    assert!(waited >= 3, "{waited} waits: the reads found their chunks in the store");
}

/// A tab over a stream read through an index, whose run is read from a store
/// that keeps part of the file, keeps no more than the index keeps decoded.
/// So a run inside the stream that is read whole is read a window at a time,
/// in the file's reading and in the tab's own, and a read that still has to
/// be made in one call and is longer than that is refused with the reason.
/// Without that, each of those reads decodes spans that push out the ones
/// before them, whose chunks of the file the store has pushed out too, and
/// asks for the same chunks for ever.
///
/// The index is made from a copy of the run, and then reads the run through
/// a handle onto the file's store.
#[test]
fn a_reading_over_a_tab_reads_a_long_run_a_window_at_a_time() {
    use crate::codec::lazy::{decode_or_index_with, Decoded, RunInput};
    let (file, member) = zlib_of_a_gzip();
    let mut d = stored(&file, 48);
    let run = 2..file.len() - 4;
    let Ok(Decoded::Indexed(mut index)) = decode_or_index_with(Codec::Deflate, std::sync::Arc::new(file[run.clone()].to_vec()), small_lazy()) else {
        panic!("indexed")
    };
    index.set_input(RunInput::new(d.source().share().expect("a store hands out a handle"), run.start as u64, run.len() as u64));
    let mut ev = Evaluator::new(crate::formats::builtin("zlib").unwrap());
    ev.spaces.set_lazy_limits(small_lazy());
    ev.spaces.keep_index((0, run.start as u64 * 8, run.len() as u64 * 8), Codec::Deflate, std::sync::Arc::new(index));
    let id = fed(&mut d, &file, |d| ev.open_space(d, 0, ZLIB_RUN)).unwrap().expect("the stream opens");
    let len = member.len() as u64;

    // In the file's reading, the whole of the stream read as a run.
    assert!(ev.gathers(&d, id, 0, len * 8));
    let got = fed(&mut d, &file, |d| ev.gather_run(d, id, 0, len * 8)).unwrap().unwrap();
    assert!(got == member, "the stream gathered differs");

    let space = ev.space_mut(id).unwrap();
    assert_eq!((space.template.as_str(), space.recognised), ("gzip", true));
    assert_eq!(space.source().resident_limit(), Some(64 << 10));
    // What the web opens the tab over says the same.
    assert_eq!(ChunkStore::behind(Box::new(space.source().clone()), CHUNK).resident_limit(), Some(64 << 10));
    let (tab, tab_doc) = space.reading().expect("a recognised stream has a reading of its own");
    assert_eq!(tab_doc.resident_limit(), Some(64 << 10));
    match tab.read_in(tab_doc, 0, 0, len * 8) {
        Err(EvalError::Failed(why)) => assert_eq!(why, "Can't load 160.1 KiB of the file at once: the editor keeps at most 64 KiB in memory"),
        other => panic!("{other:?}"),
    }
    // The member's run, between its ten bytes of header and eight of
    // trailer, read whole by the tab's reading.
    let got = fed(&mut d, &file, |_| tab.gather_run(tab_doc, 0, 80, (len - 18) * 8)).unwrap().unwrap();
    assert!(got == member[10..member.len() - 8], "the member's run gathered differs");
}

/// Three runs longer than the store, each asked for in turn in goes, the way
/// a listing showing all three asks. Only two passes are kept unfinished,
/// and a pass reads its run into its place, so the third waits for a place,
/// carrying the others on. All three open, and no pass starts twice.
#[test]
fn three_runs_longer_than_the_store_asked_for_by_turns_all_open() {
    let contents: Vec<Vec<u8>> = (0..3).map(|i| noise(3 * 1024 + i)).collect();
    let packed: Vec<Vec<u8>> = contents.iter().map(|c| miniz_oxide::deflate::compress_to_vec_zlib(c, 6)).collect();
    let mut bytes = vec![0xaa, 0xbb];
    for p in &packed {
        bytes.extend_from_slice(p);
    }
    let mut d = stored(&bytes, 4);
    let mut ev = Evaluator::new(runs(&packed.iter().map(Vec::len).collect::<Vec<_>>(), Codec::Zlib));
    ev.set_slice(Some(600));
    let mut times: BTreeMap<u64, usize> = BTreeMap::new();
    let mut open = [false; 3];
    let mut rounds = 0;
    while open.iter().any(|o| !o) {
        for i in 0..3 {
            if open[i] {
                continue;
            }
            ev.begin_slice();
            match ev.node(&d, &[1 + i]) {
                Ok(run) => {
                    assert_eq!(run.refused, None, "run {i}");
                    open[i] = true;
                }
                Err(EvalError::Busy { .. }) => {}
                Err(EvalError::Pending(missing)) => {
                    for m in missing {
                        let n = times.entry(m.chunk).or_default();
                        *n += 1;
                        assert!(*n <= 3, "chunk {} asked for {n} times", m.chunk);
                        feed(&mut d, &bytes, m.chunk);
                    }
                }
                Err(e) => panic!("{e:?}"),
            }
        }
        rounds += 1;
        assert!(rounds < 1000, "no end to the rounds");
    }
    for (i, content) in contents.iter().enumerate() {
        assert_eq!(&unpacked(&ev, &[1 + i]), content, "run {i}");
    }
    assert_eq!(ev.spaces.passes.get(), 3, "a pass started twice");
}

/// Runs of a codec that is never indexed are read into copies kept beside
/// each other, up to what the copies may come to between them. Three runs
/// that do not fit together, asked for in turn, wait for room and all open.
/// A run longer than all the copies may be is refused, as too much to keep
/// in memory at once, and the same run in memory opens.
#[test]
fn copies_past_what_they_may_come_to_wait_for_room_and_a_run_past_it_is_refused() {
    let lens = [3000, 3200, 3400];
    let mut bytes = vec![0xaa, 0xbb];
    for len in lens {
        bytes.extend(std::iter::repeat_n(b'A', len));
    }
    let mut d = stored(&bytes, 4);
    let mut ev = Evaluator::new(runs(&lens, Codec::Base64));
    ev.spaces.set_gather_cap(7000);
    ev.set_slice(Some(1));
    let mut open = [false; 3];
    let mut rounds = 0;
    while open.iter().any(|o| !o) {
        for i in 0..3 {
            if open[i] {
                continue;
            }
            ev.begin_slice();
            match ev.node(&d, &[1 + i]) {
                Ok(run) => {
                    assert_eq!(run.refused, None, "run {i}");
                    open[i] = true;
                }
                Err(EvalError::Busy { .. }) => {}
                Err(EvalError::Pending(missing)) => missing.iter().for_each(|m| feed(&mut d, &bytes, m.chunk)),
                Err(e) => panic!("{e:?}"),
            }
        }
        rounds += 1;
        assert!(rounds < 1000, "no end to the rounds");
    }
    for (i, len) in lens.iter().enumerate() {
        assert_eq!(unpacked(&ev, &[1 + i]), vec![0u8; len / 4 * 3], "run {i}");
    }

    let mut bytes = vec![0xaa, 0xbb];
    bytes.extend(std::iter::repeat_n(b'A', 6 * 1024));
    let mut d = stored(&bytes, 4);
    let mut ev = Evaluator::new(one_run(6 * 1024, Codec::Base64));
    ev.spaces.set_gather_cap(2048);
    let run = fed(&mut d, &bytes, |d| ev.node(d, &[1])).unwrap();
    assert_eq!((run.refused.as_deref(), run.child_count), (Some("too-much-at-once"), 0));
    let mut ev = Evaluator::new(one_run(6 * 1024, Codec::Base64));
    ev.spaces.set_gather_cap(2048);
    assert_eq!(ev.node(&doc(&bytes), &[1]).unwrap().refused, None, "in memory");
}
