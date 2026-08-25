//! A smoke test over a real whisper.cpp model. None is in the repository, so
//! this skips unless someone has one to hand: any `*.bin` opening with the ggml
//! magic, in `web/public` or in the directory `QUBERO_SAMPLES` names.

use qubero_core::document::Document;
use qubero_core::eval::{Evaluator, Value};
use qubero_core::formats::whisper;
use qubero_core::source::MemSource;

#[test]
fn reads_a_real_model_end_to_end() {
    let mut dirs = vec![concat!(env!("CARGO_MANIFEST_DIR"), "/../../web/public").to_string()];
    if let Ok(extra) = std::env::var("QUBERO_SAMPLES") {
        dirs.push(extra);
    }
    let mut found = 0;
    for dir in &dirs {
        let Ok(entries) = std::fs::read_dir(dir) else { continue };
        for e in entries.flatten() {
            let path = e.path();
            if path.extension().is_none_or(|x| x != "bin") {
                continue;
            }
            let Ok(bytes) = std::fs::read(&path) else { continue };
            if !bytes.starts_with(b"lmgg") {
                continue;
            }
            found += 1;
            check(&path.display().to_string(), bytes);
        }
    }
    if found == 0 {
        eprintln!("skipped: no ggml model in {dirs:?}. Put one there, or set QUBERO_SAMPLES.");
    }
}

fn check(path: &str, bytes: Vec<u8>) {
    eprintln!("--- {path}: {} bytes", bytes.len());
    let d = Document::new(MemSource(bytes));
    let mut ev = Evaluator::new(whisper());

    let int = |ev: &mut Evaluator, path: &[usize]| match ev.node(&d, path).unwrap().value {
        Value::Int(v) => v,
        other => panic!("{path:?} is {other:?}"),
    };
    let n_vocab = int(&mut ev, &[1]);
    eprintln!(
        "n_vocab {n_vocab}, audio {}x{} heads {}, text {}x{} heads {}, mels {}",
        int(&mut ev, &[5]),
        int(&mut ev, &[3]),
        int(&mut ev, &[4]),
        int(&mut ev, &[9]),
        int(&mut ev, &[7]),
        int(&mut ev, &[8]),
        int(&mut ev, &[10]),
    );
    let weights = ev.node(&d, &[13]).unwrap();
    eprintln!("ftype {} = version {}, {:?}", int(&mut ev, &[11]), int(&mut ev, &[12]), weights.value);

    // The filterbank, then the vocabulary the header counted.
    let filters = ev.node(&d, &[14, 2]).unwrap();
    eprintln!("filterbank {} weights", filters.child_count);
    assert_eq!(filters.child_count as i128, int(&mut ev, &[14, 0]) * int(&mut ev, &[14, 1]));
    let tokens = ev.node(&d, &[15, 1]).unwrap();
    eprintln!("vocabulary {} tokens", tokens.child_count);
    assert_eq!(tokens.child_count as i128, int(&mut ev, &[15, 0]));
    for i in [0usize, 1, 2] {
        eprintln!("  [{i}] {:?}", ev.node(&d, &[15, 1, i, 1]).unwrap().value);
    }

    // Every tensor, to the end of the file. Reading them all is the test: a
    // shape or a block size that is wrong anywhere puts the next tensor in the
    // wrong place, and the last one then fails to land on the end.
    let tensors = ev.node(&d, &[16]).unwrap();
    let n = tensors.child_count as usize;
    eprintln!("{n} tensors");
    for i in [0, 1, n - 2, n - 1] {
        let t = ev.node(&d, &[16, i]).unwrap();
        let data = ev.node(&d, &[16, i, 5]).unwrap();
        eprintln!(
            "  [{i}] {:?} at 0x{:x}, {} of {}",
            ev.node(&d, &[16, i, 4]).unwrap().value,
            t.offset_bits / 8,
            data.child_count,
            data.type_name,
        );
    }
    let last = ev.node(&d, &[16, n - 1]).unwrap();
    assert_eq!(last.offset_bits + last.size_bits, d.len_bits(), "the last tensor does not end at the end of the file");
}
