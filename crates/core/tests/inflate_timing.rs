//! How long a large deflate stream takes to open, so a change to the inflater
//! or to the bit reader under it can be held against what it replaced.
//!
//! Ignored by default for the reason `lzma_timing` is: a timing is not a pass
//! or a fail. Run it by name, in release:
//!
//! ```text
//! cargo test -p qubero-core --release --test inflate_timing -- --ignored --nocapture
//! ```

use std::time::Instant;

use qubero_core::codec::{decode, Codec};

#[test]
#[ignore = "a timing, not a pass"]
fn how_long_a_large_deflate_stream_takes() {
    // Text with enough variety that the blocks are dynamic and the matches
    // are of every length, and a stretch of noise so some blocks are stored
    // or nearly all literals.
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    let mut rand = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let words = ["the", "quick", "brown", "fox", "jumps", "over", "lazy", "dog", "deflate", "stream", "bits", "of"];
    let mut data = Vec::with_capacity(40 << 20);
    while data.len() < 36 << 20 {
        let w = words[(rand() % words.len() as u64) as usize];
        data.extend_from_slice(w.as_bytes());
        data.push(if rand() % 11 == 0 { b'\n' } else { b' ' });
    }
    data.extend((0..4 << 20).map(|_| rand() as u8));

    // Small enough that every symbol is a step of the trace, and large enough
    // that the trace gives up naming them and decodes on regardless.
    time("2 MiB, every symbol traced", &data[..2 << 20], 20);
    time("40 MiB, past the step budget", &data, 3);
}

fn time(name: &str, raw: &[u8], rounds: u32) {
    let packed = miniz_oxide::deflate::compress_to_vec(raw, 6);
    let out = decode(Codec::Deflate, &packed).expect("reads");
    assert_eq!(out, raw);
    let start = Instant::now();
    for _ in 0..rounds {
        std::hint::black_box(decode(Codec::Deflate, &packed).expect("reads"));
    }
    let each = start.elapsed() / rounds;
    let rate = raw.len() as f64 / (1 << 20) as f64 / each.as_secs_f64();
    println!("{name}: {each:?} for {} bytes out of {}, {rate:.1} MiB/s", raw.len(), packed.len());

    let start = Instant::now();
    for _ in 0..rounds {
        std::hint::black_box(miniz_oxide::inflate::decompress_to_vec(&packed).expect("reads"));
    }
    let each = start.elapsed() / rounds;
    let rate = raw.len() as f64 / (1 << 20) as f64 / each.as_secs_f64();
    println!("{name}, through miniz_oxide and no map: {each:?}, {rate:.1} MiB/s");
}
