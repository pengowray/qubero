//! Real JPEG XL files, read box by box.
//!
//! Four codestreams written by libjxl 0.7 through ffmpeg and one by
//! ImageMagick, and two containers put together around two of them by
//! `tools/make_jxl_samples.py` in the sample collection. Each codestream's
//! headers, TOC and sections are checked against what a reference decoder
//! written from the specification reads. The containers are checked against the codestreams
//! they were made from: the one in `jxlc` and the three `jxlp` parts joined
//! are each the bytes of the bare file, and the Exif and XMP read the same
//! whether they are stored or Brotli-compressed in a `brob` box.
//!
//! The files live in the sample collection rather than here. Point
//! `QUBERO_SAMPLES` at it, or keep it beside the repository as
//! `qubero-samples`. With neither, the test says so and passes.

use std::path::PathBuf;

use qubero_core::document::Document;
use qubero_core::eval::{Evaluator, NodeInfo, Value};
use qubero_core::formats;
use qubero_core::source::MemSource;

fn sample(name: &str) -> Option<PathBuf> {
    qubero_samples::roots().into_iter().map(|r| r.join("jxl").join(name)).find(|p| p.exists())
}

/// A file read with the template the sniffer picks for it.
struct Reading {
    bytes: Vec<u8>,
    doc: Document<MemSource>,
    ev: Evaluator,
}

impl Reading {
    fn open(name: &str) -> Option<Reading> {
        let bytes = std::fs::read(sample(name)?).unwrap();
        let sniffed = formats::sniff(&bytes[..bytes.len().min(formats::SNIFF_WINDOW)], bytes.len() as u64);
        assert_eq!(sniffed, Some("jxl"), "{name}");
        let doc = Document::new(MemSource(bytes.clone()));
        Some(Reading { bytes, doc, ev: Evaluator::new(formats::builtin("jxl").unwrap()) })
    }

    /// The node the names lead to, where a name that is a number is an
    /// element of a list.
    fn at(&mut self, steps: &[&str]) -> NodeInfo {
        let mut path = Vec::new();
        for step in steps {
            if let Ok(i) = step.parse::<usize>() {
                path.push(i);
                continue;
            }
            let n = self.ev.node(&self.doc, &path).unwrap().child_count as usize;
            let i = (0..n)
                .find(|i| self.ev.node(&self.doc, &[&path[..], &[*i]].concat()).unwrap().name == *step)
                .unwrap_or_else(|| panic!("no {step} under {path:?}"));
            path.push(i);
        }
        self.ev.node(&self.doc, &path).unwrap_or_else(|e| panic!("{steps:?}: {e:?}"))
    }

    /// The bytes of the file a node covers.
    fn bytes_of(&mut self, steps: &[&str]) -> Vec<u8> {
        let n = self.at(steps);
        self.bytes[(n.offset_bits / 8) as usize..((n.offset_bits + n.size_bits) / 8) as usize].to_vec()
    }

    /// Which box of the file is the first of type `kind`.
    fn find_box(&mut self, kind: &str) -> String {
        let n = self.at(&["boxes"]).child_count as usize;
        (0..n)
            .find(|i| self.at(&["boxes", &i.to_string(), "TBox"]).value == Value::Str(kind.into()))
            .unwrap_or_else(|| panic!("no {kind} box"))
            .to_string()
    }
}

/// Each codestream's size and TOC, as a reference decoder written from the
/// specification reads them: `tools/check_jxl_headers.py` in the sample
/// collection. Every TOC adds up to the end of its file.
const CODESTREAMS: &[(&str, (i128, i128), &[i128])] = &[
    ("magick-rgba-8x8.jxl", (8, 8), &[107]),
    ("ffmpeg-vardct-96x64.jxl", (96, 64), &[1003]),
    ("ffmpeg-modular-lossless-96x64.jxl", (96, 64), &[2331]),
    ("ffmpeg-vardct-640x480.jxl", (640, 480), &[392, 7604, 622, 1257, 1055, 388, 1589, 541, 323]),
    ("ffmpeg-modular-lossless-300x260.jxl", (300, 260), &[1102, 0, 0, 13663, 2862, 912, 181]),
];

#[test]
fn every_codestream_reads_as_its_headers_toc_and_sections() {
    for (name, (width, height), toc) in CODESTREAMS {
        let Some(mut r) = Reading::open(name) else {
            eprintln!("skipped: {}", qubero_samples::missing());
            return;
        };
        assert_eq!(r.at(&[]).type_name, "Codestream", "{name}");
        let size = |r: &mut Reading, side| r.at(&["image_header", "size", side]).value.as_int();
        assert_eq!((size(&mut r, "width"), size(&mut r, "height")), (Some(*width), Some(*height)), "{name}");
        assert_eq!(r.at(&["frames"]).child_count, 1, "{name}");
        let n = r.at(&["frames", "0", "toc", "entries"]).child_count as usize;
        let read: Vec<i128> = (0..n).map(|i| r.at(&["frames", "0", "toc", "entries", &i.to_string(), "value"]).value.as_int().unwrap()).collect();
        assert_eq!(&read, toc, "{name}");
        // The sections end where the file does, with nothing after them.
        let sections = r.at(&["frames", "0", "sections"]);
        assert_eq!(sections.offset_bits + sections.size_bits, r.bytes.len() as u64 * 8, "{name}");
        assert_eq!(r.at(&["trailer"]).size_bits, 0, "{name}");
    }
}

/// Where a frame has more than one group, each section is its own byte range:
/// the 640 by 480 picture is six groups in one pass, and its last PassGroup
/// ends the file.
#[test]
fn a_frame_of_six_groups_places_each_section() {
    let Some(mut r) = Reading::open("ffmpeg-vardct-640x480.jxl") else {
        eprintln!("skipped: {}", qubero_samples::missing());
        return;
    };
    let s = |steps: &[&'static str]| [&["frames", "0", "sections"], steps].concat();
    assert_eq!(r.at(&s(&["LfGlobal"])).size_bits, 392 * 8);
    assert_eq!(r.at(&s(&["LfGroup"])).child_count, 1);
    assert_eq!(r.at(&s(&["HfGlobal"])).size_bits, 622 * 8);
    assert_eq!(r.at(&s(&["PassGroup"])).child_count, 6);
    let last = r.at(&s(&["PassGroup", "5"]));
    assert_eq!((last.size_bits, last.offset_bits + last.size_bits), (323 * 8, r.bytes.len() as u64 * 8));
}

#[test]
fn a_container_holds_the_codestream_it_was_made_from_in_one_box() {
    let (Some(mut r), Some(bare)) = (Reading::open("container-exif-xmp-jumbf.jxl"), sample("ffmpeg-vardct-96x64.jxl")) else {
        eprintln!("skipped: {}", qubero_samples::missing());
        return;
    };
    let kinds: Vec<Value> = (0..7).map(|i| r.at(&["boxes", &i.to_string(), "TBox"]).value).collect();
    assert_eq!(kinds, ["JXL ", "ftyp", "jxll", "Exif", "xml ", "jumb", "jxlc"].map(|k| Value::Str(k.into())));
    assert_eq!(r.bytes_of(&["boxes", "6", "DBox"]), std::fs::read(bare).unwrap());
    assert!(r.at(&["codestream"]).absent);

    // Orientation, software and a date, in the TIFF's one directory.
    let tiff = r.at(&["boxes", "3", "DBox", "tiff"]);
    assert_eq!(tiff.type_name, "TIFF");
    let Value::Str(xmp) = r.at(&["boxes", "4", "DBox"]).value else { panic!("XMP is not text") };
    assert!(xmp.starts_with("<?xpacket begin="), "{xmp}");
    assert_eq!(r.at(&["boxes", "5", "DBox", "0", "DBox", "TYPE", "code"]).value, Value::Str("json".into()));
}

#[test]
fn a_codestream_in_three_parts_joins_into_the_one_it_was_cut_from() {
    let (Some(mut r), Some(bare)) = (Reading::open("container-jxlp-brob.jxl"), sample("ffmpeg-modular-lossless-96x64.jxl")) else {
        eprintln!("skipped: {}", qubero_samples::missing());
        return;
    };
    let bare = std::fs::read(bare).unwrap();
    let parts: Vec<u8> = ["4", "5", "6"].iter().flat_map(|i| r.bytes_of(&["boxes", i, "DBox", "part"])).collect();
    assert_eq!(parts, bare);
    assert_eq!(r.at(&["boxes", "6", "DBox", "last"]).value.as_int(), Some(1));
    let joined = r.at(&["codestream", "codestream"]);
    assert_eq!((joined.type_name.as_str(), joined.size_bits), ("Codestream", bare.len() as u64 * 8));
    assert!(matches!(r.at(&["codestream", "codestream", "signature"]).value, Value::Magic { ok: true, .. }));
}

#[test]
fn metadata_reads_the_same_compressed_as_stored() {
    let (Some(mut stored), Some(mut packed)) = (Reading::open("container-exif-xmp-jumbf.jxl"), Reading::open("container-jxlp-brob.jxl"))
    else {
        eprintln!("skipped: {}", qubero_samples::missing());
        return;
    };
    let exif = packed.find_box("brob");
    assert_eq!(packed.at(&["boxes", &exif, "DBox", "original_type"]).value, Value::Str("Exif".into()));
    let unpacked = packed.at(&["boxes", &exif, "DBox", "contents", "contents"]);
    assert_eq!(unpacked.type_name, "Exif");
    let a = stored.at(&["boxes", "3", "DBox", "tiff", "byte_order"]).value;
    let b = packed.at(&["boxes", &exif, "DBox", "contents", "contents", "tiff", "byte_order"]).value;
    assert_eq!(a, b);

    let xml = (exif.parse::<usize>().unwrap() + 1).to_string();
    assert_eq!(packed.at(&["boxes", &xml, "DBox", "original_type"]).value, Value::Str("xml ".into()));
    let a = stored.at(&["boxes", "4", "DBox"]).value;
    let b = packed.at(&["boxes", &xml, "DBox", "contents", "contents"]).value;
    assert_eq!(a, b);
}
