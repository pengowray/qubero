//! Project files a Tascam Portacapture X8 recorded, the 268-byte `.bin` file
//! beside each take's WAV files, read with the `x8project` template.
//!
//! Every file has to be recognised from its bytes alone, read to its last
//! byte, and pass every range check the template declares. The six track
//! blocks and the 12 bytes at 0x100 are compared with the firmware's default
//! values, which all 127 files the test was written against have there. A
//! take recorded in app 9, 10 or 11 can have other values there, and this
//! test would fail on it. One take, `260430_0070`, is compared with what is known of it:
//! its WAV files hold 157.7 seconds of audio.
//!
//! The files are not in the sample collection that is kept beside the
//! repository. The test reads every `.bin` file under an `x8project` folder
//! of a collection, and under that folder's subfolders, so a folder of takes
//! copied from the recorder's card can be linked there. Point
//! `QUBERO_SAMPLES` at a collection that has the folder. With no such
//! folder, the test says so and passes.

use std::path::{Path, PathBuf};

use qubero_core::document::Document;
use qubero_core::eval::{Evaluator, Value};
use qubero_core::formats;
use qubero_core::source::MemSource;

/// The collection folder the project files are looked for in.
const FOLDER: &str = "x8project";

/// The length of every project file, in bytes.
const FILE_SIZE: u64 = 268;

/// Every `.bin` file under a directory and its subdirectories. A symbolic
/// link to a directory is followed.
fn bin_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            bin_files(&path, out);
        } else if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("bin")) {
            out.push(path);
        }
    }
}

fn samples() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for root in qubero_samples::roots() {
        bin_files(&root.join(FOLDER), &mut out);
    }
    out.sort();
    out
}

fn named(ev: &mut Evaluator, d: &Document<MemSource>, path: &[usize], names: &[&str]) -> Vec<usize> {
    let mut p = path.to_vec();
    for name in names {
        p = ev.child_named(d, &p, name).unwrap().unwrap_or_else(|| panic!("no {name} under {p:?}"));
    }
    p
}

fn int(ev: &mut Evaluator, d: &Document<MemSource>, path: &[usize], names: &[&str]) -> i128 {
    let p = named(ev, d, path, names);
    let v = ev.node(d, &p).unwrap().value;
    v.as_int().unwrap_or_else(|| panic!("{names:?} is not a number: {v:?}"))
}

/// Whether the constraint on a field holds, failing if it has none.
fn holds(ev: &mut Evaluator, d: &Document<MemSource>, path: &[usize], names: &[&str]) -> bool {
    let p = named(ev, d, path, names);
    ev.valid_of(d, &p).unwrap().unwrap_or_else(|| panic!("{names:?} has no constraint")).ok
}

#[test]
fn every_project_file_is_recognised_and_read_whole() {
    let files = samples();
    if files.is_empty() {
        eprintln!("{}", qubero_samples::missing_dir(FOLDER));
        return;
    }
    for path in &files {
        let f = path.display();
        let bytes = std::fs::read(path).unwrap();
        assert_eq!(bytes.len() as u64, FILE_SIZE, "{f}");
        // By the bytes alone, with no file name to help.
        assert_eq!(formats::sniff(&bytes, FILE_SIZE), Some("x8project"), "{f}");
        let take = path.file_stem().and_then(|s| s.to_str()).unwrap_or_default().to_string();
        let d = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(formats::template("x8project").expect("the x8project template"));
        let (ev, d) = (&mut ev, &d);

        // Every byte of the file is in a field.
        assert_eq!(ev.node(d, &[]).unwrap().size_bits, FILE_SIZE * 8, "{f}");
        assert_eq!(int(ev, d, &[], &["version"]), 1, "{f}");
        for field in ["version", "ms_decode", "ms_decode_width", "unknown_0e", "master_level", "unknown_32"] {
            assert!(holds(ev, d, &[], &[field]), "{f}: {field}");
        }
        let ms_decode = named(ev, d, &[], &["ms_decode"]);
        assert!(matches!(ev.node(d, &ms_decode).unwrap().value, Value::Enum { name: Some(_), .. }), "{f}");

        let channels = named(ev, d, &[], &["channels"]);
        assert_eq!(ev.node(d, &channels).unwrap().child_count, 6, "{f}");
        for track in 0..6 {
            let channel = [channels.clone(), vec![track]].concat();
            assert_eq!(ev.node(d, &channel).unwrap().offset_bits, (0x10 + 4 * track as u64) * 8, "{f}");
            for field in ["level", "pan", "unknown_2", "out_of_mix"] {
                assert!(holds(ev, d, &channel, &[field]), "{f}: track {} {field}", track + 1);
            }
        }

        // The firmware's default values in each track's EQ and in the metronome's settings.
        let blocks = named(ev, d, &[], &["eq"]);
        assert_eq!(ev.node(d, &blocks).unwrap().offset_bits, 0x40 * 8, "{f}");
        for track in 0..6 {
            for (index, first) in [16, 32, 32, 16].into_iter().enumerate() {
                let entry = [blocks.clone(), vec![track, 0, index]].concat();
                let bytes = [int(ev, d, &entry, &["frequency"]), int(ev, d, &entry, &["gain"]), int(ev, d, &entry, &["q"])];
                assert_eq!(bytes, [first, 12, 3], "{f}: track {} entry {index}", track + 1);
            }
        }
        let tail = named(ev, d, &[], &["metronome"]);
        assert_eq!(ev.node(d, &tail).unwrap().offset_bits, 0x100 * 8, "{f}");
        let values = ["tempo", "beat", "level", "unknown_8", "unknown_flags", "unknown_a", "unknown_b"]
            .map(|field| int(ev, d, &tail, &[field]));
        assert_eq!(values, [120, 4, 50, 0, 2, 27, 0], "{f}");

        if take == "260430_0070" {
            assert_eq!(int(ev, d, &[], &["length_seconds"]), 157, "{f}");
            let app = named(ev, d, &[], &["recording_app"]);
            assert_eq!(ev.node(d, &app).unwrap().value, Value::Enum { raw: 2, name: Some("MANUAL".into()), hex: false });
        }
    }
    eprintln!("{} project files read", files.len());
}
