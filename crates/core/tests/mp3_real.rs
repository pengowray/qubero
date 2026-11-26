//! The MP3 samples, read against what FFmpeg says about them.
//!
//! The frame counts are `ffprobe -show_entries packet=pos,size`, which agrees
//! with the template on where every frame starts and how long it is, plus one
//! for the Xing or Info frame that FFmpeg reads as a header and does not count
//! as a packet. The files are in the sample collection's `mp3` folder; with no
//! collection to hand, the test says so and passes.

use qubero_core::document::Document;
use qubero_core::eval::{Evaluator, Value};
use qubero_core::formats;
use qubero_core::source::MemSource;

/// A sample, how many elements its run of frames has, and what is at the end.
struct Sample {
    file: &'static str,
    frames: u64,
    /// The frame count in the first frame's Xing or Info header, when it has one.
    xing_frames: Option<u128>,
    apev2_items: u64,
    lyrics3_fields: u64,
    id3v1: bool,
}

const SAMPLES: &[Sample] = &[
    Sample { file: "ffmpeg-lame-id3v24-mono.mp3", frames: 12, xing_frames: Some(11), apev2_items: 0, lyrics3_fields: 0, id3v1: false },
    Sample { file: "lame-mpeg1-44100-stereo-vbr.mp3", frames: 18, xing_frames: Some(17), apev2_items: 0, lyrics3_fields: 0, id3v1: false },
    Sample { file: "lame-mpeg1-cbr-apev2-id3v1.mp3", frames: 14, xing_frames: Some(13), apev2_items: 3, lyrics3_fields: 0, id3v1: true },
    Sample { file: "lame-mpeg1-lyrics3v2-id3v1.mp3", frames: 14, xing_frames: Some(13), apev2_items: 0, lyrics3_fields: 3, id3v1: true },
    Sample { file: "lame-mpeg2-22050-mono-id3v1.mp3", frames: 23, xing_frames: Some(22), apev2_items: 0, lyrics3_fields: 0, id3v1: true },
    Sample { file: "lame-mpeg25-8000-mono.mp3", frames: 9, xing_frames: None, apev2_items: 0, lyrics3_fields: 0, id3v1: false },
    Sample { file: "shine-mpeg1-32000-mono.mp3", frames: 9, xing_frames: None, apev2_items: 0, lyrics3_fields: 0, id3v1: false },
    Sample { file: "twolame-mpeg1-layer2-crc.mp2", frames: 13, xing_frames: None, apev2_items: 0, lyrics3_fields: 0, id3v1: false },
    Sample { file: "oversample-tone-48k-mono-xing.mp3", frames: 86, xing_frames: Some(85), apev2_items: 0, lyrics3_fields: 0, id3v1: false },
    Sample { file: "oversample-tone-48k-mono-noxing.mp3", frames: 251, xing_frames: None, apev2_items: 0, lyrics3_fields: 0, id3v1: false },
];

// Where things are: the root's fields, and a frame's.
const AUDIO: usize = 1;
const APEV2: usize = 2;
const LYRICS3: usize = 3;
const ID3V1: usize = 4;

fn child(ev: &mut Evaluator, d: &Document<MemSource>, path: &[usize], name: &str) -> Vec<usize> {
    let n = ev.node(d, path).unwrap().child_count as usize;
    for i in 0..n {
        let p = [path, &[i]].concat();
        if ev.node(d, &p).unwrap().name == name {
            return p;
        }
    }
    panic!("{path:?} has no child {name}");
}

fn size(ev: &mut Evaluator, d: &Document<MemSource>, path: &[usize]) -> u64 {
    ev.node(d, path).unwrap().size_bits / 8
}

#[test]
fn every_sample_reads_as_frames_and_tags_with_nothing_left_over() {
    let Some(dir) = qubero_samples::dir("mp3") else {
        eprintln!("{}", qubero_samples::missing_dir("mp3"));
        return;
    };
    for s in SAMPLES {
        let path = dir.join(s.file);
        let Ok(bytes) = std::fs::read(&path) else {
            eprintln!("skipped: no {}", path.display());
            continue;
        };
        let len = bytes.len() as u64;
        let head = &bytes[..bytes.len().min(formats::SNIFF_WINDOW)];
        assert_eq!(formats::sniff_named(head, len, s.file), Some("mp3"), "{}", s.file);
        let d = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(formats::template("mp3").unwrap());
        assert_eq!(size(&mut ev, &d, &[]), len, "{}: the template does not cover the file", s.file);
        assert_eq!(ev.node(&d, &[AUDIO, 0]).unwrap().child_count, s.frames, "{}: frames", s.file);
        assert_eq!(size(&mut ev, &d, &[AUDIO, 1]), 0, "{}: bytes after the frames", s.file);

        let first = [AUDIO, 0, 0];
        let xing = child(&mut ev, &d, &first, "xing");
        match s.xing_frames {
            Some(n) => {
                let count = child(&mut ev, &d, &xing, "frame_count");
                assert_eq!(ev.node(&d, &count).unwrap().value, Value::UInt(n), "{}: Xing frame count", s.file);
            }
            None => assert_eq!(size(&mut ev, &d, &xing), 0, "{}: a Xing header nobody wrote", s.file),
        }
        let items = |ev: &mut Evaluator, field: usize, list: &str| match size(ev, &d, &[field]) {
            0 => 0,
            _ => {
                let p = child(ev, &d, &[field], list);
                ev.node(&d, &p).unwrap().child_count
            }
        };
        assert_eq!(items(&mut ev, APEV2, "items"), s.apev2_items, "{}: APEv2 items", s.file);
        assert_eq!(items(&mut ev, LYRICS3, "fields"), s.lyrics3_fields, "{}: Lyrics3v2 fields", s.file);
        assert_eq!(size(&mut ev, &d, &[ID3V1]), if s.id3v1 { 128 } else { 0 }, "{}: ID3v1", s.file);
    }
}

/// Layer III frames share their main data: a frame's `main_data_begin` says
/// how many bytes before its own audio its main data starts. So what the
/// template calls the end of one frame, `later_main_data`, has to be as long
/// as the next frame says, and what it calls this frame's `main_data` must
/// not run into it. A side information field read from the wrong bits breaks
/// one or the other within a few frames.
#[test]
fn the_bit_reservoir_split_agrees_from_one_frame_to_the_next() {
    let Some(dir) = qubero_samples::dir("mp3") else {
        eprintln!("{}", qubero_samples::missing_dir("mp3"));
        return;
    };
    let mut reservoir_used = 0;
    for s in SAMPLES.iter().filter(|s| !s.file.ends_with(".mp2")) {
        let Ok(bytes) = std::fs::read(dir.join(s.file)) else { continue };
        let d = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(formats::template("mp3").unwrap());
        let mut before: Option<(u64, u64, u64)> = None;
        for i in 0..s.frames as usize {
            let frame = [AUDIO, 0, i];
            let side = child(&mut ev, &d, &frame, "side_info");
            let begin = child(&mut ev, &d, &side, "main_data_begin");
            let Value::UInt(begin) = ev.node(&d, &begin).unwrap().value else { panic!("main_data_begin") };
            let begin = begin as u64;
            let own = child(&mut ev, &d, &frame, "main_data");
            let ancillary = child(&mut ev, &d, &frame, "ancillary_data");
            let later = child(&mut ev, &d, &frame, "later_main_data");
            let (own, ancillary, later) = (size(&mut ev, &d, &own), size(&mut ev, &d, &ancillary), size(&mut ev, &d, &later));
            if let Some((own_before, room_before, later_before)) = before {
                assert!(own_before == 0 || begin <= room_before, "{} frame {i}: main data starts inside the frame before's own", s.file);
                assert_eq!(later_before, begin.min(room_before), "{} frame {i}", s.file);
            }
            if begin > 0 {
                reservoir_used += 1;
            }
            before = Some((own, ancillary + later, later));
        }
    }
    if dir.join("lame-mpeg1-44100-stereo-vbr.mp3").exists() {
        assert!(reservoir_used > 0, "no frame used the bit reservoir, so this test proves nothing");
    }
}
