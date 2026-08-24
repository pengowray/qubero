//! The three MIDI files Windows ships. Running status is the normal case, and
//! all three use it, so this is the test that says whether a real file reads.
//! It skips where those files are not present.

use qubero_core::document::Document;
use qubero_core::eval::{Evaluator, Value};
use qubero_core::formats::midi;
use qubero_core::source::MemSource;

const FILES: &[&str] = &["C:/Windows/Media/flourish.mid", "C:/Windows/Media/onestop.mid", "C:/Windows/Media/town.mid"];

#[test]
fn every_track_reads_to_its_end() {
    let mut checked = 0;
    for path in FILES {
        let Ok(bytes) = std::fs::read(path) else {
            eprintln!("skipped: no file at {path}");
            continue;
        };
        checked += 1;
        let d = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(midi());
        let chunks = ev.node(&d, &[]).unwrap().child_count;
        let mut tracks = 0;
        let mut running = 0;
        for c in 0..chunks as usize {
            if ev.node(&d, &[c, 0]).unwrap().value != Value::Str("MTrk".into()) {
                continue;
            }
            tracks += 1;
            let body = ev.node(&d, &[c, 2]).unwrap();
            let events = body.child_count;
            assert!(events > 0, "{path}: a track with no events");
            for e in 0..events as usize {
                // A status field of no bits is an event running on the last
                // one's status. Every event still has a message.
                if ev.node(&d, &[c, 2, e, 1]).unwrap().size_bits == 0 {
                    running += 1;
                }
                let message = ev.node(&d, &[c, 2, e, 3]).unwrap();
                assert_ne!(message.type_name, "NoStatus", "{path}: track {c} event {e} had no status to run from");
            }
            // The last event of a track is the end-of-track meta event, which
            // is only reachable if every event before it was read right.
            let last = events as usize - 1;
            let status = ev.node(&d, &[c, 2, last, 2]).unwrap().value;
            assert_eq!(
                status,
                Value::Enum { raw: 0xff, name: Some("meta".into()), hex: true },
                "{path}: track {c} does not end on a meta event"
            );
            let kind = ev.node(&d, &[c, 2, last, 3, 0]).unwrap().value;
            assert_eq!(
                kind,
                Value::Enum { raw: 0x2f, name: Some("end of track".into()), hex: true },
                "{path}: track {c} does not end on end of track"
            );
        }
        eprintln!("{path}: {tracks} tracks, {running} events running on the status before them");
        assert!(tracks > 0, "{path}: no tracks");
        assert!(running > 0, "{path}: no running status, so this file proves nothing");
    }
    if checked == 0 {
        eprintln!("skipped: none of the sample files are on this machine");
    }
}
