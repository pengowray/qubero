//! zstd and xz files of several frames or blocks, written by the tools that
//! write them, read a piece at a time with the limits made small.
//!
//! The three files are in the sample collection's `compressed` folder, made by
//! its `tools/make_frame_samples.py`, and hold the same 6 MiB of text:
//! `pzstd-frames.zst` is `pzstd -1`, three frames and an empty fourth, each
//! behind a skippable frame; `xz-T0-blocks.xz` is `xz -0 -T0`, six blocks;
//! and `zstd-T0-B1M.zst` is `zstd -1 -T0 -B1MiB`, which is one frame however
//! many jobs `-B` cut it into. Without the collection the tests say so and
//! pass.

use std::sync::Arc;

use qubero_core::codec::lazy::{decode_or_index_with, Decoded, Limits, Seekable};
use qubero_core::codec::{decode_traced, Codec, Refusal, Trace};

fn sample(name: &str) -> Option<Vec<u8>> {
    let path = qubero_samples::dir("compressed")?.join(name);
    path.exists().then(|| std::fs::read(path).unwrap())
}

/// Held whole up to 256 KiB, so 6 MiB is far past it, with a span allowed to
/// be up to 4 MiB: a `pzstd -1` frame is 2 MiB and an `xz -0 -T0` block is
/// 1 MiB. The span cache holds less than the file, so reading it all turns
/// the cache over.
fn small() -> Limits {
    Limits { whole: 256 << 10, spacing: 256 << 10, checkpoints: 16, span_cache: 3 << 20, span: 4 << 20, ..Limits::default() }
}

fn indexed(codec: Codec, packed: &[u8]) -> Seekable {
    match decode_or_index_with(codec, Arc::new(packed.to_vec()), small()) {
        Ok(Decoded::Indexed(s)) => s,
        Ok(Decoded::Whole(bytes, _)) => panic!("{codec:?}: {} bytes came back whole", bytes.len()),
        Err(why) => panic!("{codec:?}: refused: {why:?}"),
    }
}

fn rand_from(mut seed: u64) -> impl FnMut() -> u64 {
    move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    }
}

/// The file read a piece at a time is the file decoded whole, byte for byte,
/// from anywhere: random reads, reads either side of every span boundary, and
/// one read of everything. Every byte and bit sampled maps to the step the
/// whole trace says.
fn reads_back(codec: Codec, packed: &[u8], whole: &[u8], trace: &Trace) {
    let s = indexed(codec, packed);
    assert_eq!(s.len(), whole.len() as u64);
    assert!(s.spans() >= 3, "{codec:?}: {} spans", s.spans());
    let mut rand = rand_from(codec.as_str().len() as u64 + 1);
    for _ in 0..200 {
        let at = rand() % whole.len() as u64;
        let n = (rand() % 300_000).min(whole.len() as u64 - at) as usize;
        let mut got = vec![0u8; n];
        s.read(at, &mut got).expect("reads");
        assert!(got == whole[at as usize..at as usize + n], "{codec:?}: {n} bytes at {at} differ");
    }
    // The last span of the pzstd file is its empty last frame, which starts
    // where the output ends.
    for k in 1..s.spans() {
        let at = s.span_bytes(k).start - 10;
        let n = 20.min(whole.len() - at as usize);
        let mut got = vec![0u8; n];
        s.read(at, &mut got).expect("reads");
        assert!(got == whole[at as usize..at as usize + n], "{codec:?}: across the start of span {k}");
    }
    let mut all = vec![0u8; whole.len()];
    s.read(0, &mut all).expect("reads");
    assert!(all == whole, "{codec:?}: the whole output differs");

    // Every span's trace tiles its own bits and bytes, the empty last frame
    // of the pzstd file included.
    for k in 0..s.spans() {
        let t = s.span_trace(k).expect("traces");
        t.check_tiles().unwrap_or_else(|e| panic!("{codec:?} span {k}: {e}"));
        assert_eq!((t.start().1, t.out_bytes()), (s.span_bytes(k).start, s.span_bytes(k).end), "{codec:?} span {k}");
    }

    let bits = packed.len() as u64 * 8;
    let mut bytes: Vec<u64> = (0..500).map(|_| rand() % whole.len() as u64).collect();
    let mut ins: Vec<u64> = (0..500).map(|_| rand() % bits).collect();
    bytes.sort_unstable();
    ins.sort_unstable();
    for byte in bytes {
        assert_eq!(s.map_out(byte), trace.map_out(byte), "{codec:?}: byte {byte}");
    }
    for bit in ins {
        assert_eq!(s.map_in(bit), trace.map_in(bit), "{codec:?}: bit {bit}");
    }
}

#[test]
fn a_pzstd_file_reads_back_a_frame_at_a_time() {
    let Some(packed) = sample("pzstd-frames.zst") else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    let (whole, trace) = decode_traced(Codec::Zstd, &packed).expect("decodes whole");
    assert_eq!(whole.len(), 6 << 20);
    trace.check_tiles().expect("tiles");
    reads_back(Codec::Zstd, &packed, &whole, &trace);
}

#[test]
fn an_xz_t0_file_reads_back_a_block_at_a_time() {
    let Some(packed) = sample("xz-T0-blocks.xz") else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    let (whole, trace) = decode_traced(Codec::Xz, &packed).expect("decodes whole");
    assert_eq!(whole.len(), 6 << 20);
    assert_eq!(trace.members().len(), 6);
    reads_back(Codec::Xz, &packed, &whole, &trace);
}

/// `zstd -T0 -B` writes one frame, which no span can hold under these limits,
/// so it is refused as too large. Decoded whole it is the same text the
/// other two files hold, which puts our zstd reading of both files beside
/// our xz reading of the third.
#[test]
fn a_zstd_file_of_one_frame_stays_refused() {
    let (Some(one), Some(frames), Some(blocks)) = (sample("zstd-T0-B1M.zst"), sample("pzstd-frames.zst"), sample("xz-T0-blocks.xz")) else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    assert_eq!(decode_or_index_with(Codec::Zstd, Arc::new(one.clone()), small()).err(), Some(Refusal::TooLarge));
    let text = decode_traced(Codec::Xz, &blocks).expect("decodes").0;
    assert!(decode_traced(Codec::Zstd, &one).expect("decodes").0 == text);
    assert!(decode_traced(Codec::Zstd, &frames).expect("decodes").0 == text);
}
