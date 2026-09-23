//! A bat recording from a Pettersson D500X, read against its own bytes.
//!
//! The recorder writes a 980-byte metadata block at the front of the `data`
//! chunk and a `data` size that counts only the samples after it, and a RIFF
//! size that counts the whole file. Read by the sizes alone, the block plays
//! as the first 490 samples and the last 490 fall off the end of the chunk.
//!
//! The file lives in the sample collection rather than here. Point
//! `QUBERO_SAMPLES` at it, or keep it beside the repository as
//! `qubero-samples`. With neither, the test says so and passes.

use qubero_core::document::Document;
use qubero_core::eval::{Evaluator, Value};
use qubero_core::formats;
use qubero_core::source::MemSource;

const FILE: &str = "xc1060673-kuhls-pipistrelle-data-size-short.wav";

/// Where the recorder's audio starts: after the 44-byte header and the block.
const AUDIO_AT: usize = 0x400;

#[test]
fn a_d500x_recording_reads_its_metadata_then_every_sample() {
    let Some(path) = qubero_samples::roots().into_iter().map(|r| r.join("wav").join(FILE)).find(|p| p.exists()) else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    let bytes = std::fs::read(&path).unwrap();
    let len = bytes.len() as u64;
    let sample = |i: usize| i16::from_le_bytes([bytes[AUDIO_AT + 2 * i], bytes[AUDIO_AT + 2 * i + 1]]);
    let count = (bytes.len() - AUDIO_AT) / 2;
    let (first, last) = (sample(0), sample(count - 1));

    let d = Document::new(MemSource(bytes));
    let mut ev = Evaluator::new(formats::template("wav").expect("the wav template"));

    // Two chunks, and nothing left over after them.
    assert_eq!(ev.node(&d, &[]).unwrap().size_bits, len * 8);
    assert_eq!(ev.node(&d, &[3]).unwrap().child_count, 2);
    // The tail is where the chunk used to stop 980 bytes short.
    let tail = ev.spans(&d, (len - 2048) * 8, len * 8, 4096).unwrap();
    assert!(!tail.is_empty());
    assert!(tail.iter().all(|s| !s.gap), "a gap: {:?}", tail.iter().find(|s| s.gap));

    // The data chunk runs to the end of the file: its size, and the block.
    assert_eq!(ev.node(&d, &[3, 1, 1]).unwrap().value, Value::UInt(300_000));
    assert_eq!(ev.node(&d, &[3, 1, 2]).unwrap().size_bits, (300_000 + 980) * 8);
    assert_eq!(ev.node(&d, &[3, 1, 2]).unwrap().type_name, "D500XData");
    assert_eq!(ev.node(&d, &[3, 1, 2, 0]).unwrap().size_bits, 980 * 8);
    assert_eq!(
        ev.node(&d, &[3, 1, 2, 0, 4]).unwrap().value,
        Value::Str("D500X V2.2.6 140516, 17:19:14".into())
    );
    assert_eq!(ev.node(&d, &[3, 1, 2, 0, 9]).unwrap().child_count, 10);

    // Sample 0 is the first one after the block, and the last is the last
    // two bytes of the file.
    let samples = ev.node(&d, &[3, 1, 2, 1]).unwrap();
    assert_eq!(samples.child_count, count as u64);
    assert_eq!(count, 150_000);
    assert_eq!(ev.node(&d, &[3, 1, 2, 1, 0]).unwrap().value, Value::Int(i128::from(first)));
    assert_eq!(ev.node(&d, &[3, 1, 2, 1, count - 1]).unwrap().value, Value::Int(i128::from(last)));

    // The RIFF size counts the whole file, 8 bytes more than there is room for.
    let riff_size = ev.valid_of(&d, &[1]).unwrap().expect("a verdict on the RIFF size");
    assert!(!riff_size.ok);
    assert_eq!(riff_size.text, format!("Out of range: must be at most {}", len - 8));
}
