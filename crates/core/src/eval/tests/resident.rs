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
use crate::eval::SpaceSource;
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
fn in_goes<T>(ev: &mut Evaluator, d: &mut Document<ChunkStore>, bytes: &[u8], slice: u64, ask: impl FnMut(&mut Evaluator, &Document<ChunkStore>) -> R<T>) -> (R<T>, usize) {
    in_goes_noting(ev, d, bytes, slice, &mut Vec::new(), ask)
}

/// The same, keeping the chunks each `Pending` named in `asked`.
fn in_goes_noting<T>(
    ev: &mut Evaluator,
    d: &mut Document<ChunkStore>,
    bytes: &[u8],
    slice: u64,
    asked: &mut Vec<Vec<u64>>,
    mut ask: impl FnMut(&mut Evaluator, &Document<ChunkStore>) -> R<T>,
) -> (R<T>, usize) {
    ev.set_slice(Some(slice));
    let mut times: BTreeMap<u64, usize> = BTreeMap::new();
    let mut busy = 0;
    loop {
        ev.begin_slice();
        match ask(ev, d) {
            Err(EvalError::Pending(missing)) => {
                asked.push(missing.iter().map(|m| m.chunk).collect());
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
/// cap it is read a part at a time, each page where it is in the store, and
/// opens too. In memory, with no handle onto the file, it would be read
/// from copies of the pages, which come to more than the cap, so it is
/// refused as too large.
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
    let id = fed(&mut d, &bytes, |d| ev.open_space(d, 0, &[1])).unwrap().expect("the stream opened");
    let SpaceSource::Joined(joined) = ev.space(id).unwrap().source().clone() else { panic!("held whole") };
    assert_eq!(joined.owned_bytes(), 0, "the stream holds copies of pages");
    assert_eq!(joined_reads(&joined, &mut d, &bytes, 2048).0, bytes);
    let mut ev = Evaluator::new(paged(6));
    ev.spaces.set_whole_cap(5 * 1024);
    assert_eq!(ev.open_space(&doc(&bytes), 0, &[1]).unwrap(), None);
    assert_eq!(ev.open_refusal(&[1]), Some(Refusal::TooLargeCopy));
}

/// All of the joined stream `joined`, read in order `piece` bytes at a
/// time, as a tab's reads are made: each read asked again, with exactly the
/// chunks it names fed, until it reads. Also how many chunks the widest
/// answer named. A chunk asked for many times over fails the test.
fn joined_reads(joined: &crate::eval::Joined, d: &mut Document<ChunkStore>, bytes: &[u8], piece: usize) -> (Vec<u8>, usize) {
    let mut all = Vec::with_capacity(joined.len() as usize);
    let mut widest = 0;
    for at in (0..joined.len()).step_by(piece) {
        let (got, named) = joined_read(joined, d, bytes, at, piece.min((joined.len() - at) as usize));
        all.extend(got);
        widest = widest.max(named);
    }
    (all, widest)
}

/// `n` bytes of the joined stream `joined` at `at`, the same way, and how
/// many chunks the widest answer named. Each answer is checked against what
/// the stream says the read would find missing.
fn joined_read(joined: &crate::eval::Joined, d: &mut Document<ChunkStore>, bytes: &[u8], at: u64, n: usize) -> (Vec<u8>, usize) {
    use crate::codec::lazy::ReadError;
    let mut asked = Vec::new();
    let got = fed_noting(d, bytes, &mut asked, |_| {
        let mut out = vec![0u8; n];
        let said = joined.missing(at, n as u64);
        match joined.read(at, &mut out) {
            Ok(()) => {
                assert!(said.is_empty(), "{said:?} said to be missing at {at}, and the read found none");
                Ok(out)
            }
            Err(ReadError::Missing(missing)) => {
                assert_eq!(said, missing, "{n} bytes at {at}");
                Err(EvalError::Pending(missing))
            }
            Err(ReadError::Refused(why)) => panic!("{n} bytes at {at}: {why:?}"),
        }
    })
    .unwrap();
    (got, asked.iter().map(Vec::len).max().unwrap_or(0))
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

/// Lines of text, each with a number in hex that does not repeat, so that
/// they pack to about half.
fn hex_lines(n: usize) -> Vec<u8> {
    let mut x = 0x9e37_79b9_7f4a_7c15_u64;
    (0..n)
        .flat_map(|i| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            format!("line {i}: {x:016x}\n").into_bytes()
        })
        .collect()
}

/// A zlib run past the cap and longer than the store is read a window at a
/// time to index it, and the index then reads the run through a handle onto
/// the store: it holds no copy of the run, and keeps as much as its span
/// cache. A read of a span whose chunks the store has pushed out since waits
/// for them, and then gives the bytes the run unpacks to.
#[test]
fn a_run_longer_than_the_store_is_read_from_the_store_once_indexed() {
    let content = hex_lines(12_000);
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

/// Whether `got` is the failure of a read that can never fit, against the
/// limit of an index that keeps 64 KiB decoded.
fn cannot_fit<T: std::fmt::Debug>(got: R<T>) -> bool {
    match got {
        Err(EvalError::Failed(why)) => why.starts_with("Can't load ") && why.ends_with(" of the file at once: the editor keeps at most 64 KiB in memory"),
        other => panic!("{other:?}"),
    }
}

/// A read of a stream read through an index from the store, made in one
/// call, such as a field read whole where a tab reads the stream in the
/// file's reading, that is longer than the index keeps decoded can never
/// find all its spans decoded at once: each span it decodes pushes out the
/// ones before it, whose chunks the store has pushed out too. It fails with
/// the reason, as such a read of the file does, instead of asking for the
/// same chunks for ever. A read that fits waits for its chunks and reads.
/// The same holds for a read of a part of a joined stream that is read
/// through the part's index.
#[test]
fn a_read_in_one_call_longer_than_an_index_keeps_fails_with_the_reason() {
    let content = hex_lines(12_000);
    let packed = miniz_oxide::deflate::compress_to_vec_zlib(&content, 6);
    let mut bytes = vec![0xaa, 0xbb];
    bytes.extend_from_slice(&packed);
    let mut d = stored(&bytes, 48);
    let mut ev = Evaluator::new(one_run(packed.len(), Codec::Zlib));
    ev.spaces.set_lazy_limits(small_lazy());
    fed(&mut d, &bytes, |d| ev.node(d, &[1, 0])).unwrap();
    let Some(Opened::Space(id)) = ev.spaces.get(&[1]) else { panic!("the stream did not open") };
    assert!(cannot_fit(fed(&mut d, &bytes, |d| ev.read_in(d, id, 0, content.len() as u64 * 8))));
    let (at, n) = (1000, 48 << 10);
    let got = fed(&mut d, &bytes, |d| ev.read_in(d, id, at as u64 * 8, n as u64 * 8)).unwrap();
    assert!(got == content[at..at + n], "48 KiB at {at} differ");

    // The joined stream of a gzip's members, its one member past the cap.
    let bytes = gzip_member(&content);
    let mut d = stored(&bytes, 48);
    let mut ev = Evaluator::new(crate::formats::gzip());
    ev.spaces.set_lazy_limits(small_lazy());
    let joined = fed(&mut d, &bytes, |d| ev.node(d, &[1, 0])).unwrap().space;
    assert!(ev.spaces.stitch(joined).is_some(), "not a stitched space");
    assert!(cannot_fit(fed(&mut d, &bytes, |d| ev.read_in(d, joined, 0, content.len() as u64 * 8))));
    let got = fed(&mut d, &bytes, |d| ev.read_in(d, joined, at as u64 * 8, n as u64 * 8)).unwrap();
    assert!(got == content[at..at + n], "48 KiB of the joined stream at {at} differ");
}

/// A gzip past the cap opens as a stream joined from its one member and read
/// a part at a time, the way Open unpacked opens one. The member's index
/// reads its run from the store, so the joined stream keeps as much as that
/// index keeps decoded, and a read of it names chunks of the file the store
/// has pushed out, and gives the member's bytes once they are fed. Opening
/// it reads the joined stream's first bytes after the walk over its parts has
/// finished, which can wait for chunks, and the next call opens the stream
/// that walk made.
#[test]
fn a_gzip_past_the_cap_opens_as_a_joined_stream_read_from_the_store() {
    use crate::codec::lazy::ReadError;
    use crate::eval::SpaceSource;
    let content = hex_lines(12_000);
    let bytes = gzip_member(&content);
    assert!(bytes.len() > 96 * CHUNK as usize, "{} bytes is too few for a store of 48 chunks", bytes.len());
    let mut d = stored(&bytes, 48);
    let mut ev = Evaluator::new(crate::formats::gzip());
    ev.spaces.set_lazy_limits(small_lazy());
    // Under the stream's 288 KB and over its run of about 140 KB, which a
    // stream read a part at a time may not pass between its parts' runs.
    ev.spaces.set_whole_cap(200 << 10);
    let id = fed(&mut d, &bytes, |d| ev.open_space(d, 0, &[1])).unwrap().expect("the stream opens");
    assert_eq!(ev.spaces.walks.get(), 1, "joined more than once");
    let source = ev.space(id).unwrap().source().clone();
    assert!(matches!(source, SpaceSource::Joined(_)), "not read a part at a time");
    assert_eq!(source.resident_limit(), Some(64 << 10));
    let mut waited = 0;
    for at in [content.len() / 3, 10, content.len() / 2, content.len() - 100] {
        let mut asked = Vec::new();
        let got = fed_noting(&mut d, &bytes, &mut asked, |_| {
            let mut out = vec![0u8; 100];
            match source.read(at as u64, &mut out) {
                Ok(()) => Ok(out),
                Err(ReadError::Missing(missing)) => Err(EvalError::Pending(missing)),
                Err(ReadError::Refused(why)) => panic!("{why:?}"),
            }
        })
        .unwrap();
        assert!(got == content[at..at + 100], "100 bytes at {at} differ");
        waited += asked.len();
    }
    assert!(waited >= 2, "{waited} waits: the reads found their chunks in the store");
}

/// A zstd frame whose compressed bytes are longer than one read from a store
/// may gather is refused as too much to keep in memory at once when the
/// stream is read from the store, and not as too large to unpack, which it
/// is not. The same stream in memory opens.
#[test]
fn a_zstd_frame_too_long_to_read_from_the_store_is_refused_as_too_much_at_once() {
    let content = noise(300 << 10);
    let bytes = crate::codec::lazy::tests::zstd_of(&content, 100_000, false);
    let limits = crate::codec::lazy::Limits { unit_input: 64 << 10, ..small_lazy() };
    let mut d = stored(&bytes, 48);
    let mut ev = Evaluator::new(crate::formats::builtin("zstd").unwrap());
    ev.spaces.set_lazy_limits(limits);
    let decoded = fed(&mut d, &bytes, |d| ev.child_named(d, &[], "decoded")).unwrap().expect("a decoded field");
    let run = [decoded[0], 0];
    let node = fed(&mut d, &bytes, |d| ev.node(d, &run)).unwrap();
    assert_eq!(node.refused.as_deref(), Some("too-much-at-once"));
    let mut ev = Evaluator::new(crate::formats::builtin("zstd").unwrap());
    ev.spaces.set_lazy_limits(limits);
    let d = doc(&bytes);
    let decoded = ev.child_named(&d, &[], "decoded").unwrap().expect("a decoded field");
    assert_eq!(ev.node(&d, &[decoded[0], 0]).unwrap().refused, None, "in memory");
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

// ----- an index pass reading its run a window at a time -----

/// [`small_lazy`], with steps that read 8 KiB of the run and the margin past
/// it, about 72 KiB at once, so that a store of 160 chunks has room for two
/// such windows and a pass reads its run through a handle onto it.
fn windowed_lazy() -> crate::codec::lazy::Limits {
    crate::codec::lazy::Limits { step_input: 8 << 10, ..small_lazy() }
}

/// A zlib run past the cap and several times longer than the store is
/// indexed by a pass that reads it through a handle onto the store, a
/// window at a time, and keeps no copy of it. Each wait names the chunks of
/// at most one window, and while the pass waits the evaluator says how far
/// it has got, as it does when a go runs out. A missing chunk is never kept
/// as the run's refusal. The pass starts once, and the index it comes to is
/// the one made from the run in memory.
#[test]
fn a_pass_over_a_run_longer_than_the_store_reads_it_a_window_at_a_time() {
    use crate::codec::lazy::{decode_or_index_with, Decoded};
    let content = hex_lines(40_000);
    let packed = miniz_oxide::deflate::compress_to_vec_zlib(&content, 6);
    let mut bytes = vec![0xaa, 0xbb];
    bytes.extend_from_slice(&packed);
    let capacity = 160;
    assert!(bytes.len() > 3 * capacity * CHUNK as usize, "{} bytes is too few for a store of {capacity} chunks", bytes.len());
    let mut d = stored(&bytes, capacity);
    let mut ev = Evaluator::new(one_run(packed.len(), Codec::Zlib));
    ev.spaces.set_lazy_limits(windowed_lazy());
    ev.set_slice(Some(600));
    let mut times: BTreeMap<u64, usize> = BTreeMap::new();
    let (mut waits, mut was) = (0, 0);
    loop {
        ev.begin_slice();
        match ev.node(&d, &[1, 0]) {
            Ok(_) => break,
            Err(EvalError::Pending(missing)) => {
                waits += 1;
                assert!(missing.len() <= 76, "a wait named {} chunks", missing.len());
                let p = ev.unpacking().expect("a pass waiting for bytes says how far it has got");
                assert_eq!((p.run_space, p.run_offset_bits, p.run_bits), (0, 16, packed.len() as u64 * 8));
                assert!(p.read_bits >= was, "the pass went back from {was} to {}", p.read_bits);
                was = p.read_bits;
                for m in missing {
                    let n = times.entry(m.chunk).or_default();
                    *n += 1;
                    assert!(*n <= 2, "chunk {} asked for {n} times", m.chunk);
                    feed(&mut d, &bytes, m.chunk);
                }
            }
            Err(EvalError::Busy { .. }) => {}
            Err(e) => panic!("{e:?}"),
        }
        assert_eq!(ev.spaces.refusals(), 0, "a run waiting for bytes was refused");
        assert!(waits < 1000, "no end to the waits");
    }
    assert!(waits > packed.len() / (72 << 10), "{waits} waits");
    assert_eq!(ev.spaces.passes.get(), 1, "the pass started more than once");
    let Some(Opened::Space(id)) = ev.spaces.get(&[1]) else { panic!("the stream did not open") };
    let index = ev.spaces.lazy(id).expect("read through an index").clone();
    assert_eq!(index.input().owned_bytes(), 0, "the index holds a copy of the run");
    let Ok(Decoded::Indexed(whole)) = decode_or_index_with(Codec::Zlib, std::sync::Arc::new(packed.clone()), windowed_lazy()) else { panic!("indexed") };
    assert_eq!((index.spans(), index.len(), index.blocks(), index.adler32(), index.held_bytes()), (whole.spans(), whole.len(), whole.blocks(), whole.adler32(), whole.held_bytes()));
    for k in 0..index.spans() {
        assert_eq!(index.span_bytes(k), whole.span_bytes(k), "span {k}");
    }
    for at in [10, content.len() / 3, content.len() - 100] {
        let got = fed(&mut d, &bytes, |d| ev.read_in(d, id, at as u64 * 8, 800)).unwrap();
        assert!(got == content[at..at + 100], "100 bytes at {at} differ");
    }
}

/// Three runs past the cap, each longer than the store, asked for by turns
/// in goes, as a listing showing three large entries asks. Each pass reads
/// its run through the store a window at a time, and the third waits for a
/// place, carrying the others on, which wait for chunks there in turn. No
/// pass waiting for chunks is ever kept as refused, all three open, no pass
/// starts twice, and each stream reads its own bytes.
#[test]
fn three_passes_reading_windows_asked_for_by_turns_all_open() {
    let contents: Vec<Vec<u8>> = (0..3).map(|i| hex_lines(24_000 + i)).collect();
    let packed: Vec<Vec<u8>> = contents.iter().map(|c| miniz_oxide::deflate::compress_to_vec_zlib(c, 6)).collect();
    let capacity = 240;
    let lens: Vec<usize> = packed.iter().map(Vec::len).collect();
    assert!(lens.iter().all(|&n| n > capacity * CHUNK as usize), "{lens:?} bytes is too few for a store of {capacity} chunks");
    let mut bytes = vec![0xaa, 0xbb];
    for p in &packed {
        bytes.extend_from_slice(p);
    }
    let mut d = stored(&bytes, capacity);
    let mut ev = Evaluator::new(runs(&lens, Codec::Zlib));
    ev.spaces.set_lazy_limits(windowed_lazy());
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
            match ev.node(&d, &[1 + i, 0]) {
                Ok(_) => open[i] = true,
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
            assert_eq!(ev.spaces.refusals(), 0, "a run waiting for bytes was refused");
        }
        rounds += 1;
        assert!(rounds < 2000, "no end to the rounds");
    }
    assert_eq!(ev.spaces.passes.get(), 3, "a pass started twice");
    for (i, content) in contents.iter().enumerate() {
        let Some(Opened::Space(id)) = ev.spaces.get(&[1 + i]) else { panic!("run {i} did not open") };
        assert_eq!(ev.spaces.lazy(id).expect("read through an index").input().owned_bytes(), 0, "run {i}'s index holds a copy");
        for at in [10, content.len() - 100] {
            let got = fed(&mut d, &bytes, |d| ev.read_in(d, id, at as u64 * 8, 800)).unwrap();
            assert!(got == content[at..at + 100], "run {i}: 100 bytes at {at} differ");
        }
    }
}

// ----- the two limits on how long a compressed run may be -----

/// A zlib run past the limit on a copy, and several times longer than the
/// store, opens through its handle: its pass reads it a window at a time and
/// keeps no copy, so only `Limits::input` applies, and a run of exactly that
/// length opens. One byte past `Limits::input`, the same run is refused as
/// `too-large-input` before anything is read: the store has nothing in it,
/// and nothing is asked for. In a store with too little room for two
/// windows, or in memory with no handle, the pass would read a copy, which
/// is past the copy limit, so it is refused as `too-large-copy`, again before
/// anything is read, whatever `Limits::input` is.
#[test]
fn a_run_is_held_to_the_limit_of_the_way_its_pass_reads_it() {
    use crate::codec::lazy::Limits;
    let content = hex_lines(40_000);
    let packed = miniz_oxide::deflate::compress_to_vec_zlib(&content, 6);
    let mut bytes = vec![0xaa, 0xbb];
    bytes.extend_from_slice(&packed);
    let len = packed.len();
    let capacity = 160;
    assert!(len > 3 * capacity * CHUNK as usize, "{len} bytes is too few for a store of {capacity} chunks");
    let reading = |limits: Limits| {
        let mut ev = Evaluator::new(one_run(len, Codec::Zlib));
        ev.spaces.set_lazy_limits(limits);
        ev
    };
    let copy_input = len / 2;

    let mut d = stored(&bytes, capacity);
    let mut ev = reading(Limits { copy_input, input: len as u64, ..windowed_lazy() });
    fed(&mut d, &bytes, |d| ev.node(d, &[1, 0])).unwrap();
    assert_eq!(fed(&mut d, &bytes, |d| ev.node(d, &[1])).unwrap().refused, None);
    let Some(Opened::Space(id)) = ev.spaces.get(&[1]) else { panic!("the stream did not open") };
    assert_eq!(ev.spaces.lazy(id).expect("read through an index").input().owned_bytes(), 0);
    let at = content.len() / 2;
    assert!(fed(&mut d, &bytes, |d| ev.read_in(d, id, at as u64 * 8, 800)).unwrap() == content[at..at + 100]);

    let refused = |d: &Document<ChunkStore>, limits: Limits, why: Refusal, at: &str| {
        let mut ev = reading(limits);
        assert_eq!(ev.open_space(d, 0, &[1]).unwrap_or_else(|e| panic!("{at}: {e:?}")), None, "{at}");
        assert_eq!(ev.open_refusal(&[1]), Some(why), "{at}");
        assert_eq!(ev.node(d, &[1]).unwrap().refused.as_deref(), Some(why.as_str()), "{at}");
    };
    let past_input = Limits { copy_input, input: len as u64 - 1, ..windowed_lazy() };
    refused(&stored(&bytes, capacity), past_input, Refusal::TooLargeInput, "past the input limit");
    // Room for less than two windows of 8 KiB and the margin.
    let too_small = 2 * ((8 << 10) + 65_541) / CHUNK as usize - 4;
    let roomy = Limits { copy_input, input: 2 * len as u64, ..windowed_lazy() };
    refused(&stored(&bytes, too_small), roomy, Refusal::TooLargeCopy, "a copy, from a small store");
    refused(&stored(&bytes, too_small), past_input, Refusal::TooLargeCopy, "a copy past both limits");
    let mut ev = reading(roomy);
    assert_eq!(ev.node(&doc(&bytes), &[1]).unwrap().refused.as_deref(), Some("too-large-copy"), "a copy, in memory");
}

/// A .gz of one member, past the limit on a copy and several times longer
/// than the store, opens as a joined stream read a part at a time: its one
/// part is read through an index over the store, which holds no copy of its
/// run, so the run does not count against the cap on what a joined stream
/// read a part at a time copies. Past `Limits::input` it is refused as
/// `too-large-input`, and the member's own stream says so too. In memory,
/// with no handle onto the file, the part's index would hold a copy, so the
/// run counts against the cap and the stream is refused as too large.
#[test]
fn a_gzip_member_past_the_copy_limit_opens_as_a_joined_stream_through_its_handle() {
    use crate::codec::lazy::Limits;
    use crate::eval::SpaceSource;
    let content = noise(400 << 10);
    let bytes = gzip_member(&content);
    let run = bytes.len() - 18;
    let capacity = 160;
    assert!(bytes.len() > 2 * capacity * CHUNK as usize, "{} bytes is too few for a store of {capacity} chunks", bytes.len());
    let limits = Limits { copy_input: 64 << 10, input: run as u64, ..windowed_lazy() };
    let reading = |limits: Limits| {
        let mut ev = Evaluator::new(crate::formats::gzip());
        ev.spaces.set_lazy_limits(limits);
        ev.spaces.set_whole_cap(64 << 10);
        ev
    };

    let mut d = stored(&bytes, capacity);
    let mut ev = reading(limits);
    let (id, _) = in_goes(&mut ev, &mut d, &bytes, 600, |ev, d| ev.open_space(d, 0, &[1]));
    let id = id.unwrap().expect("the joined stream opens");
    let SpaceSource::Joined(_) = ev.space(id).unwrap().source() else { panic!("not read a part at a time") };
    let mut got = vec![0u8; 100];
    for at in [0, content.len() / 2, content.len() - 100] {
        for _ in 0..10 {
            // The joined stream says which chunks a read would find
            // missing, and the read finds those.
            let said = ev.space(id).unwrap().source().missing(at as u64, 100);
            let wanted = ev.space(id).unwrap().source().read_bytes(at as u64, &mut got);
            assert_eq!(said, wanted, "100 bytes at {at}");
            if wanted.is_empty() {
                break;
            }
            for m in wanted {
                feed(&mut d, &bytes, m.chunk);
            }
        }
        assert!(got == content[at..at + 100], "100 bytes at {at} differ");
    }

    let mut d = stored(&bytes, capacity);
    let mut ev = reading(Limits { input: run as u64 - 1, ..limits });
    let (id, _) = in_goes(&mut ev, &mut d, &bytes, 600, |ev, d| ev.open_space(d, 0, &[1]));
    assert_eq!(id.unwrap(), None);
    assert_eq!(ev.open_refusal(&[1]), Some(Refusal::TooLargeInput));
    let (member, _) = in_goes(&mut ev, &mut d, &bytes, 600, |ev, d| ev.node(d, &[0, 0, 10]));
    assert_eq!(member.unwrap().refused.as_deref(), Some("too-large-input"));

    let mut ev = reading(Limits { copy_input: 1 << 20, ..limits });
    assert_eq!(ev.open_space(&doc(&bytes), 0, &[1]).unwrap(), None, "in memory");
    assert_eq!(ev.open_refusal(&[1]), Some(Refusal::TooLargeCopy), "in memory");
}

// ----- joined streams read through handles onto the store -----

/// A hundred BGZF blocks of a couple of kilobytes each, in two bits of noise
/// a byte, which packs to about a quarter: what each block holds, and the
/// file.
fn hundred_blocks() -> (Vec<Vec<u8>>, Vec<u8>) {
    use crate::formats::bam::tests::{bgzf_block, EOF_BLOCK};
    let pieces: Vec<Vec<u8>> = (0..100).map(|i| noise(2048 + i).iter().map(|b| b & 3).collect()).collect();
    let mut bytes: Vec<u8> = pieces.iter().flat_map(|p| bgzf_block(p)).collect();
    bytes.extend_from_slice(&EOF_BLOCK);
    (pieces, bytes)
}

/// BGZF blocks of a couple of kilobytes each, read as a stream joined a part
/// at a time because the stream is past the cap, whose runs come to three
/// times the store and to more than the cap. Each block's run is read
/// through a handle onto the store, so the joined stream holds no copy of
/// any of them, and the cap on copies does not refuse it as it did. Opening
/// the stream reads every block's header and trailer, which say where its
/// run is and what it unpacks to, and none of the runs. The stream then
/// reads a block at a time from the store: each read names the chunks of
/// every block it reaches, all at once, and gives the bytes and steps the
/// same stream gives held whole.
#[test]
fn a_bgzf_stream_past_the_store_and_the_cap_opens_through_handles() {
    use crate::codec::lazy::Limits;
    let (pieces, bytes) = hundred_blocks();
    let capacity = 16;
    assert!(bytes.len() > 3 * capacity * CHUNK as usize, "{} bytes is too few for a store of {capacity} chunks", bytes.len());
    let text = pieces.concat();
    let whole_cap = 16 << 10;
    let mut whole = Evaluator::new(crate::formats::bgzf());
    let held = whole.open_space(&doc(&bytes), 0, &[1]).unwrap().expect("opens whole");
    let held = whole.space(held).unwrap();
    assert_eq!(held.bytes(), text);
    for slice in [None, Some(100)] {
        let mut d = stored(&bytes, capacity);
        let mut ev = Evaluator::new(crate::formats::bgzf());
        ev.set_stream_limits(Limits::default(), whole_cap);
        ev.spaces.set_cache_cap(8 << 10);
        let mut asked = Vec::new();
        let opened = match slice {
            None => fed_noting(&mut d, &bytes, &mut asked, |d| ev.open_space(d, 0, &[1])).unwrap(),
            Some(n) => in_goes_noting(&mut ev, &mut d, &bytes, n, &mut asked, |ev, d| ev.open_space(d, 0, &[1])).0.unwrap(),
        };
        let id = opened.unwrap_or_else(|| panic!("goes of {slice:?}: refused as {:?}", ev.open_refusal(&[1])));
        assert_eq!(ev.spaces.walks.get(), 1, "goes of {slice:?}: joined more than once");
        // A window is a quarter of the store, and the chunks of the run a
        // walk waits on may straddle it.
        let widest = asked.iter().map(Vec::len).max().unwrap_or(0);
        assert!(widest <= 6, "goes of {slice:?}: an answer named {widest} chunks");
        let space = ev.space(id).unwrap();
        let SpaceSource::Joined(joined) = space.source().clone() else { panic!("held whole") };
        assert_eq!(joined.owned_bytes(), 0, "goes of {slice:?}: the stream holds copies of runs");
        // Half the store, less two blocks' runs of about 600 bytes.
        let limit = joined.resident_limit().expect("a limit on one read");
        assert!((6 << 10..8 << 10).contains(&limit), "a limit of {limit}");

        for at in [text.len() / 2, 0, text.len() - 2000, text.len() / 3] {
            let (got, widest) = joined_read(&joined, &mut d, &bytes, at as u64, 2000);
            assert!(got == text[at..at + 2000], "goes of {slice:?}: 2000 bytes at {at} differ");
            assert!(widest <= 3, "goes of {slice:?}: a read of two blocks named {widest} chunks");
        }
        let (got, widest) = joined_reads(&joined, &mut d, &bytes, 6 << 10);
        assert!(got == text, "goes of {slice:?}: the stream read in pieces differs");
        assert!(widest <= 8, "goes of {slice:?}: a read of 6 KiB named {widest} chunks");
        // Each block is decoded when a read reaches it, and kept as a
        // stitched space keeps its parts, the ones read longest ago going
        // first.
        let (cached, peak) = joined.cached();
        assert!(cached.len() <= 4 && peak <= 8 << 10, "goes of {slice:?}: {} blocks, {peak} bytes kept decoded", cached.len());

        // A byte's step needs its block's run to make the block's trace.
        for byte in [0, 12_345, text.len() as u64 / 2, text.len() as u64 - 1] {
            let mut waits = 0;
            let step = loop {
                match space.map_out(byte) {
                    Ok(step) => break step,
                    Err(missing) => {
                        waits += 1;
                        assert!(waits < 4 && missing.len() <= 2, "byte {byte}: {missing:?}");
                        for m in missing {
                            feed(&mut d, &bytes, m.chunk);
                        }
                    }
                }
            };
            assert_eq!(step, held.map_out(byte).unwrap(), "goes of {slice:?}: byte {byte}");
        }
    }
    // In memory, with no handle onto the file, the blocks' runs are copied,
    // which comes to more than the cap.
    let mut ev = Evaluator::new(crate::formats::bgzf());
    ev.set_stream_limits(Limits::default(), whole_cap);
    assert_eq!(ev.open_space(&doc(&bytes), 0, &[1]).unwrap(), None);
    assert_eq!(ev.open_refusal(&[1]), Some(Refusal::TooLargeCopy));
}

/// `n` bytes of the joined stream `joined` at `at`, as [`joined_read`]
/// reads them, for a read that may be refused: what it gave, and the
/// refusal. A refused read gives the bytes of the parts that read, and
/// zeroes for the one that did not.
fn joined_read_or_refused(joined: &crate::eval::Joined, d: &mut Document<ChunkStore>, bytes: &[u8], at: u64, n: usize) -> (Vec<u8>, Option<Refusal>) {
    use crate::codec::lazy::ReadError;
    fed(d, bytes, |_| {
        let mut out = vec![0xee; n];
        match joined.read(at, &mut out) {
            Ok(()) => Ok((out, None)),
            Err(ReadError::Missing(missing)) => Err(EvalError::Pending(missing)),
            Err(ReadError::Refused(why)) => Ok((out, Some(why))),
        }
    })
    .unwrap()
}

/// Opening a joined stream of a hundred blocks as a document unpacks none
/// of them, from the store or from copies: each block is as long as its
/// trailer says, and is checked when a read first unpacks it. A read of one
/// block then unpacks that block, and a read of it again unpacks nothing.
/// Before, opening unpacked every block once to check its length, which is
/// most of what a BAM of 300 MiB took to open.
#[test]
fn opening_a_joined_stream_of_many_blocks_unpacks_none_of_them() {
    use crate::codec::decodes_on_this_thread;
    use crate::codec::lazy::Limits;
    let (pieces, bytes) = hundred_blocks();
    let text = pieces.concat();
    for in_store in [true, false] {
        let mut d = stored(&bytes, bytes.len().div_ceil(CHUNK as usize));
        for chunk in 0..bytes.len().div_ceil(CHUNK as usize) {
            feed(&mut d, &bytes, chunk as u64);
        }
        let held = doc(&bytes);
        let mut ev = Evaluator::new(crate::formats::bgzf());
        // Past the cap, which the copies in memory, about 60 KiB, are not.
        ev.set_stream_limits(Limits::default(), 100 << 10);
        let before = decodes_on_this_thread();
        let id = match in_store {
            true => ev.open_space(&d, 0, &[1]),
            false => ev.open_space(&held, 0, &[1]),
        };
        let id = id.unwrap().unwrap_or_else(|| panic!("in the store: {in_store}: refused as {:?}", ev.open_refusal(&[1])));
        assert_eq!(decodes_on_this_thread() - before, 0, "in the store: {in_store}");
        let SpaceSource::Joined(joined) = ev.space(id).unwrap().source().clone() else { panic!("held whole") };
        assert_eq!(joined.owned_bytes() > 0, !in_store);
        let at = pieces[..30].iter().map(Vec::len).sum::<usize>() + 10;
        for decoded in [1, 0] {
            let before = decodes_on_this_thread();
            let mut out = vec![0u8; 100];
            joined.read(at as u64, &mut out).expect("reads");
            assert_eq!(out, text[at..at + 100]);
            assert_eq!(decodes_on_this_thread() - before, decoded, "in the store: {in_store}");
        }
    }
}

/// The same blocks, with block 50 spoiled: its trailer says it unpacks to
/// one byte more than it does, or the first byte of its deflate makes a
/// block type that does not exist. Neither refuses the stream any more,
/// since opening it unpacks no block. A read that reaches block 50 is
/// refused. It gives the bytes of the blocks either side, zeroes for block
/// 50 and nothing shifted, and the stream says which bytes those zeroes are
/// and why, naming the block as the listing does. Reads of other blocks are
/// not touched, block 50 is unpacked once however often it is read, and its
/// bytes map to no step.
#[test]
fn a_bgzf_block_that_lies_about_its_length_or_is_damaged_fails_where_it_is_read() {
    use crate::codec::decodes_on_this_thread;
    use crate::codec::lazy::Limits;
    use crate::formats::bam::tests::bgzf_block;
    let (pieces, bytes) = hundred_blocks();
    let text = pieces.concat();
    let in_file = pieces[..50].iter().map(|p| bgzf_block(p).len()).sum::<usize>();
    let end = in_file + bgzf_block(&pieces[50]).len();
    let start = pieces[..50].iter().map(Vec::len).sum::<usize>();
    let n = pieces[50].len();
    let long: &dyn Fn(&mut Vec<u8>) = &|f| f[end - 4..end].copy_from_slice(&(n as u32 + 1).to_le_bytes());
    let damaged: &dyn Fn(&mut Vec<u8>) = &|f| f[in_file + 18] = 0xff;
    for (what, spoil, claimed, said) in [
        ("a trailer one byte long", long, n + 1, format!("blocks[50].compressed unpacks to {n} bytes, not the {} the file says", n + 1)),
        ("a damaged deflate", damaged, n, "blocks[50].compressed could not be unpacked".to_string()),
    ] {
        let mut spoiled = bytes.clone();
        spoil(&mut spoiled);
        let mut d = stored(&spoiled, 16);
        let mut ev = Evaluator::new(crate::formats::bgzf());
        ev.set_stream_limits(Limits::default(), 16 << 10);
        let id = fed(&mut d, &spoiled, |d| ev.open_space(d, 0, &[1])).unwrap();
        let id = id.unwrap_or_else(|| panic!("{what}: refused as {:?}", ev.open_refusal(&[1])));
        let space = ev.space(id).unwrap();
        let SpaceSource::Joined(joined) = space.source().clone() else { panic!("held whole") };
        // The blocks after block 50 start where its trailer says it ends.
        let stream = [&text[..start], &vec![0; claimed][..], &text[start + n..]].concat();
        assert_eq!(joined.len(), stream.len() as u64, "{what}");

        // The last 100 bytes of block 49, block 50, and the first 100 of 51.
        let (at, len) = (start - 100, claimed + 200);
        let before = decodes_on_this_thread();
        for _ in 0..3 {
            let (got, refused) = joined_read_or_refused(&joined, &mut d, &spoiled, at as u64, len);
            assert_eq!(refused, Some(Refusal::Failed), "{what}");
            assert!(got == stream[at..at + len], "{what}: the bytes either side of block 50 differ, or its own are not zeroes");
        }
        assert_eq!(decodes_on_this_thread() - before, 3, "{what}: blocks 49 and 51 once each, and block 50 once");
        let named = ev.unreadable(id, at as u64, len as u64).unwrap_or_else(|| panic!("{what}: nothing said why"));
        assert_eq!((named.at, named.len, named.why.as_str()), (start as u64, claimed as u64, said.as_str()), "{what}");
        // A reading with nothing but the stream names the block by its index.
        let unnamed = space.source().unreadable(at as u64, len as u64).unwrap();
        assert_eq!(unnamed.why, said.replace("blocks[50].compressed", "part index 50"), "{what}");
        assert_eq!(ev.unreadable(id, 0, start as u64), None, "{what}: before block 50");
        assert_eq!(space.map_out(start as u64 + 3), Ok(None), "{what}: a byte of block 50 maps to a step");

        // Away from block 50, reads give the blocks' bytes.
        for at in [0, start / 2, start + claimed + 3000, stream.len() - 2000] {
            let (got, refused) = joined_read_or_refused(&joined, &mut d, &spoiled, at as u64, 2000);
            assert_eq!(refused, None, "{what}: 2000 bytes at {at}");
            assert!(got == stream[at..at + 2000], "{what}: 2000 bytes at {at} differ");
        }
    }
}

/// A .gz of several members, whose joined stream is a gzip of its own and
/// is recognised as one, so the tab over it reads its fields with a reading
/// of its own, over the stream's bytes. When one member's trailer says it
/// unpacks to more than it does, a read of the tab's reading that reaches
/// that member fails with the reason, rather than reading zeroes, and reads
/// before it still read. When the first member is the one, the stream's
/// first bytes cannot be looked at, and it opens as the bytes it declares.
#[test]
fn a_tab_reading_its_own_fields_fails_where_a_member_will_not_unpack() {
    use crate::codec::lazy::Limits;
    use crate::formats::bam::tests::bgzf_block;
    use crate::source::ArcSource;
    let inner = gzip_member(&noise(6000));
    let pieces: Vec<&[u8]> = inner.chunks(1000).collect();
    let blocks: Vec<Vec<u8>> = pieces.iter().map(|p| bgzf_block(p)).collect();
    for bad in [2, 0] {
        let mut file = Vec::new();
        for (i, block) in blocks.iter().enumerate() {
            let mut block = block.clone();
            if i == bad {
                let at = block.len() - 4;
                block[at..].copy_from_slice(&(pieces[i].len() as u32 + 1).to_le_bytes());
            }
            file.extend(block);
        }
        let d = Document::new(ArcSource(std::sync::Arc::new(file)));
        let mut ev = Evaluator::new(crate::formats::gzip());
        ev.set_stream_limits(Limits::default(), 1000);
        let id = ev.open_space(&d, 0, &[1]).unwrap().unwrap_or_else(|| panic!("member {bad}: refused as {:?}", ev.open_refusal(&[1])));
        let space = ev.space_mut(id).unwrap();
        assert!(matches!(space.source(), SpaceSource::Joined(_)), "member {bad}: held whole");
        if bad == 0 {
            assert!(!space.recognised, "the first member's zeroes were looked at");
            continue;
        }
        assert_eq!((space.template.as_str(), space.recognised), ("gzip", true));
        let (tab, tab_doc) = space.reading().expect("a recognised stream has a reading of its own");
        assert_eq!(tab.read_in(tab_doc, 0, 0, 16).unwrap(), inner[..2]);
        match tab.read_in(tab_doc, 0, 1500 * 8, 3000 * 8) {
            Err(EvalError::Failed(why)) => assert_eq!(why, "part index 2 unpacks to 1000 bytes, not the 1001 the file says"),
            other => panic!("{other:?}"),
        }
    }
}

/// The same kind of blocks read as a .gz of many members, whose joined
/// stream says only that it holds bytes, so opening it reads the stream's
/// first 36 KiB to see what they are. The walk over the members' runs, more
/// than the store keeps, has pushed those first chunks out of the store by
/// then, so opening the stream waits for them. The next call opens the
/// stream the walk made. Joining it again, from the first member, would push
/// the first chunks out again before they are read, and so on for ever.
#[test]
fn a_gzip_of_many_members_past_the_store_opens_once_its_first_bytes_are_loaded() {
    use crate::codec::lazy::Limits;
    use crate::formats::bam::tests::{bgzf_block, EOF_BLOCK};
    // 64 KiB of two bits of noise a byte in each member, which packs to
    // about 16 KiB.
    let pieces: Vec<Vec<u8>> = (0..40).map(|i| noise((64 << 10) - i).iter().map(|b| b & 3).collect()).collect();
    let mut bytes: Vec<u8> = pieces.iter().flat_map(|p| bgzf_block(p)).collect();
    bytes.extend_from_slice(&EOF_BLOCK);
    let capacity = 512;
    assert!(bytes.len() > capacity * CHUNK as usize, "{} bytes is too few for a store of {capacity} chunks", bytes.len());
    let text = pieces.concat();
    let mut d = stored(&bytes, capacity);
    let mut ev = Evaluator::new(crate::formats::gzip());
    ev.set_stream_limits(Limits::default(), 64 << 10);
    let id = fed(&mut d, &bytes, |d| ev.open_space(d, 0, &[1])).unwrap().expect("the stream opened");
    assert_eq!(ev.spaces.walks.get(), 1, "joined more than once");
    let SpaceSource::Joined(joined) = ev.space(id).unwrap().source().clone() else { panic!("held whole") };
    assert_eq!(joined.owned_bytes(), 0, "the stream holds copies of runs");
    assert!(joined_reads(&joined, &mut d, &bytes, 64 << 10).0 == text, "the stream read in pieces differs");
}

/// Pages stored as they sit, past the cap and three times the store, joined
/// and read a part at a time: each page is read where it is in the store, so
/// the walk reads none of them and the stream holds none of them. Opening
/// the stream then looks at its first bytes to see what they are, as many as
/// one read of the stream may ask for, and waits for their chunks. The next
/// call opens the stream the walk made, rather than joining it again.
///
/// A page an edit has written into is read from a copy, which has the
/// edited bytes. In memory, with no handle onto the file, every page would
/// be copied, which comes to more than the cap, so the stream is refused as
/// too large.
#[test]
fn a_joined_stream_of_stored_pages_past_the_store_and_the_cap_opens_through_handles() {
    use crate::codec::lazy::Limits;
    let pages = 48;
    let bytes = noise(pages * 1024);
    let capacity = 16;
    let whole_cap = 8 << 10;
    let mut d = stored(&bytes, capacity);
    let mut ev = Evaluator::new(paged(pages));
    ev.set_stream_limits(Limits::default(), whole_cap);
    let mut asked = Vec::new();
    let id = fed_noting(&mut d, &bytes, &mut asked, |d| ev.open_space(d, 0, &[1])).unwrap().expect("the stream opened");
    assert_eq!(ev.spaces.walks.get(), 1, "joined more than once");
    assert_eq!(asked, [(0..8).collect::<Vec<u64>>()], "the first 8 KiB, and nothing else");
    let SpaceSource::Joined(joined) = ev.space(id).unwrap().source().clone() else { panic!("held whole") };
    assert_eq!(joined.owned_bytes(), 0, "the stream holds copies of pages");
    assert_eq!(joined.resident_limit(), Some(8 << 10), "half the store");
    for at in [30 << 10, 5000, (pages << 10) - 3000] {
        let (got, widest) = joined_read(&joined, &mut d, &bytes, at as u64, 3000);
        assert!(got == bytes[at..at + 3000], "3000 bytes at {at} differ");
        assert!(widest <= 4, "3000 bytes at {at} named {widest} chunks");
    }
    let (got, widest) = joined_reads(&joined, &mut d, &bytes, 8 << 10);
    assert!(got == bytes, "the stream read in pieces differs");
    assert_eq!(widest, 8);

    // A reading over the stream, as a tab's own reading is, refuses a read
    // in one call longer than that limit, and one within it waits and reads.
    // The store holds the last 16 pages now.
    let tab = Document::new(ChunkStore::behind(Box::new(SpaceSource::Joined(joined.clone())), CHUNK));
    let reader = Evaluator::new(Template::new("t", T::structure("Root", vec![("all", T::bytes(E::Remaining))])));
    match reader.read_in(&tab, 0, 0, 9 << 13) {
        Err(EvalError::Failed(why)) => assert_eq!(why, "Can't load 9 KiB of the file at once: the editor keeps at most 8 KiB in memory"),
        other => panic!("{other:?}"),
    }
    assert!(fed(&mut d, &bytes, |_| reader.read_in(&tab, 0, 0, 6 << 13)).unwrap() == bytes[..6 << 10], "6 KiB of the tab differ");

    // An edit inside page 3, and the stream opened again.
    d.overwrite_bytes(3 * 1024 + 10, b"edited");
    ev.invalidate_from((3 * 1024 + 10) * 8);
    let id = fed(&mut d, &bytes, |d| ev.open_space(d, 0, &[1])).unwrap().expect("the stream opened again");
    let SpaceSource::Joined(joined) = ev.space(id).unwrap().source().clone() else { panic!("held whole") };
    assert_eq!(joined.owned_bytes(), 1024, "a copy of the edited page, and of nothing else");
    let mut edited = bytes.clone();
    edited[3 * 1024 + 10..3 * 1024 + 16].copy_from_slice(b"edited");
    assert!(joined_reads(&joined, &mut d, &bytes, 4 << 10).0 == edited, "the stream differs from the edited file");

    let mut ev = Evaluator::new(paged(pages));
    ev.set_stream_limits(Limits::default(), whole_cap);
    assert_eq!(ev.open_space(&doc(&bytes), 0, &[1]).unwrap(), None);
    assert_eq!(ev.open_refusal(&[1]), Some(Refusal::TooLargeCopy));
}
