//! Anabat zero-crossing files from the sample collection, read against their
//! own bytes.
//!
//! The intervals the `zc` template works out with `Prev` are checked against
//! a decoder written here in plain Rust, the way Oversample decodes type 131
//! and 132 files: a one-byte change from the interval before, a 13, 21 or 29
//! bit interval given outright, or a two-byte status code that gives none.
//! `zcjs-demo.zc`, when the collection has it, is a real recording of 2,219
//! codes; the others are built by `tools/make_zc_wav_samples.py` in the
//! collection.
//!
//! Point `QUBERO_SAMPLES` at the collection, or keep it beside the repository
//! as `qubero-samples`. With neither, the test says so and passes.

use qubero_core::document::Document;
use qubero_core::eval::{Evaluator, Value};
use qubero_core::formats;
use qubero_core::source::MemSource;

/// Every interval in a type 131 or 132 file, in order, with `None` for a
/// status code.
fn decode(data: &[u8]) -> Vec<Option<i128>> {
    let mut out = Vec::new();
    let (mut i, mut last) = (0usize, 0i128);
    while i < data.len() {
        let b = data[i];
        let whole = |n: usize| data[i + 1..i + n].iter().fold((b & 0x1f) as i128, |acc, x| acc << 8 | *x as i128);
        let (step, interval) = match b {
            0x00..=0x7f => (1, Some(last + ((b as i8) << 1 >> 1) as i128)),
            0x80..=0x9f => (2, Some(whole(2))),
            0xa0..=0xbf => (3, Some(whole(3))),
            0xc0..=0xdf => (4, Some(whole(4))),
            0xe0..=0xff => (2, None),
        };
        if let Some(v) = interval {
            last = v;
        }
        out.push(interval);
        i += step;
    }
    out
}

fn int(v: Value) -> i128 {
    match v {
        Value::Int(v) => v,
        Value::UInt(v) => v as i128,
        other => panic!("{other:?}"),
    }
}

/// The child of `path` called `name`.
fn child(ev: &mut Evaluator, d: &Document<MemSource>, path: &[usize], name: &str) -> Vec<usize> {
    let n = ev.node(d, path).unwrap().child_count as usize;
    (0..n)
        .map(|i| [path, &[i]].concat())
        .find(|p| ev.node(d, p).unwrap().name == name)
        .unwrap_or_else(|| panic!("no {name} under {path:?}"))
}

#[test]
fn every_anabat_sample_reads_whole_and_its_intervals_add_up() {
    let Some(dir) = qubero_samples::dir("zc") else {
        eprintln!("{}", qubero_samples::missing_dir("zc"));
        return;
    };
    let mut files: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).collect();
    files.sort();
    for path in files {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let bytes = std::fs::read(&path).unwrap();
        let len = bytes.len() as u64;
        assert_eq!(formats::sniff(&bytes, len), Some("zc"), "{name}");
        let file_type = bytes[3];
        let data_pointer = u16::from_le_bytes([bytes[0x11a], bytes[0x11b]]) as usize;
        let d = Document::new(MemSource(bytes.clone()));
        let mut ev = Evaluator::new(formats::template("zc").unwrap());
        assert_eq!(ev.node(&d, &[]).unwrap().size_bits, len * 8, "{name}: the whole file is read");
        let spans = ev.spans(&d, 0, len * 8, 100_000).unwrap();
        assert!(spans.iter().all(|s| !s.gap), "{name}: a gap at {:?}", spans.iter().find(|s| s.gap));
        if file_type < 131 {
            continue;
        }

        let want = decode(&bytes[data_pointer..]);
        let list = child(&mut ev, &d, &[], "intervals");
        assert_eq!(ev.node(&d, &list).unwrap().child_count, want.len() as u64, "{name}");
        if name == "zcjs-demo.zc" {
            assert_eq!(want.len(), 2219);
        }
        // Every code in order, as a reader scrolling through would ask.
        let mut time = 0i128;
        for (i, w) in want.iter().enumerate() {
            let code = [list.clone(), vec![i]].concat();
            let at = child(&mut ev, &d, &code, "interval");
            let interval = int(ev.node(&d, &at).unwrap().value);
            if let Some(w) = w {
                assert_eq!(interval, *w, "{name}: code {i}");
                time += w;
            }
            let at = child(&mut ev, &d, &code, "time");
            assert_eq!(int(ev.node(&d, &at).unwrap().value), time, "{name}: code {i}");
        }
    }
}
