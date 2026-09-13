//! Arrow IPC files from pyarrow, read against pyarrow's own answers. The
//! fixtures live in QUBERO_SAMPLES/arrow, or in the sibling qubero-samples
//! collection, and are written by its `tools/make_arrow_samples.py`.
use std::path::PathBuf;
use qubero_core::{document::Document, eval::Evaluator, formats, source::MemSource};

fn arrow_samples() -> Option<PathBuf> {
    let mut roots = Vec::new();
    if let Ok(paths) = std::env::var("QUBERO_SAMPLES") {
        roots.extend(paths.split(';').filter(|s| !s.is_empty()).map(PathBuf::from));
    }
    roots.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../qubero-samples"));
    roots.into_iter().map(|p| p.join("arrow")).find(|p| p.is_dir())
}

fn open(root: &std::path::Path, name: &str) -> (Document<MemSource>, Evaluator) {
    let bytes = std::fs::read(root.join(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
    assert_eq!(formats::sniff(&bytes[..bytes.len().min(formats::SNIFF_WINDOW)], bytes.len() as u64), Some("arrow"), "{name}");
    (Document::new(MemSource(bytes)), Evaluator::new(formats::builtin("arrow").unwrap()))
}

/// Where every message is and how long it is, as pyarrow walks the stream.
///
/// Read on 2026-09-14 with `pyarrow.ipc.read_message` from byte 8 onwards,
/// one message after another: the offset, the bytes of marker and length in
/// front of the FlatBuffer (eight, or four for the legacy layout), the
/// FlatBuffer's length and the body's. The last row is the end-of-stream
/// marker, and then where the footer starts.
struct Layout {
    name: &'static str,
    prefix: u64,
    messages: &'static [(u64, u64, u64)],
    end_of_stream: u64,
    footer: u64,
}

const LAYOUTS: &[Layout] = &[
    Layout {
        name: "columns-uncompressed.arrow",
        prefix: 8,
        messages: &[(8, 848, 0), (864, 168, 32), (1072, 824, 464), (2368, 824, 216)],
        end_of_stream: 3416,
        footer: 3424,
    },
    Layout {
        name: "columns-lz4.arrow",
        prefix: 8,
        messages: &[(8, 848, 0), (864, 184, 80), (1136, 840, 952), (2936, 840, 616)],
        end_of_stream: 4400,
        footer: 4408,
    },
    Layout {
        name: "columns-zstd.arrow",
        prefix: 8,
        messages: &[(8, 848, 0), (864, 192, 72), (1136, 848, 856), (2848, 848, 560)],
        end_of_stream: 4264,
        footer: 4272,
    },
    Layout {
        name: "columns-legacy.arrow",
        prefix: 4,
        messages: &[(8, 844, 0), (856, 172, 32), (1064, 828, 464), (2360, 828, 216)],
        end_of_stream: 3408,
        footer: 3412,
    },
    Layout {
        name: "more-types.arrow",
        prefix: 8,
        messages: &[(8, 960, 0), (976, 1040, 472)],
        end_of_stream: 2496,
        footer: 2504,
    },
];

// Children of the root: magic, padding, footer, footer_length, footer_magic,
// schema, batches, end_of_stream.
const FOOTER: usize = 2;
const SCHEMA: usize = 5;
const BATCHES: usize = 6;
const END: usize = 7;

/// Every message is where the stream puts it: the schema at byte 8, and the
/// dictionary and record batches where the footer's blocks say, which is the
/// same place. The end-of-stream marker is where the last block ends, and the
/// footer is found from the back.
#[test]
fn every_block_the_footer_lists_is_placed() {
    let Some(root) = arrow_samples() else {
        eprintln!("skipped: set QUBERO_SAMPLES to the sample collection");
        return;
    };
    for layout in LAYOUTS {
        let (doc, mut ev) = open(&root, layout.name);
        let name = layout.name;
        let schema = ev.node(&doc, &[SCHEMA, 0]).unwrap();
        let (at, meta, body) = layout.messages[0];
        assert_eq!(schema.offset_bits, at * 8, "{name}: schema");
        assert_eq!(schema.size_bits, (layout.prefix + meta + body) * 8, "{name}: schema");
        let batches = ev.node(&doc, &[BATCHES]).unwrap();
        assert_eq!(batches.child_count as usize, layout.messages.len() - 1, "{name}: one child per block");
        let mut placed: Vec<(u64, u64)> = (0..batches.child_count as usize)
            .map(|i| {
                let n = ev.node(&doc, &[BATCHES, i]).unwrap();
                (n.offset_bits / 8, n.size_bits / 8)
            })
            .collect();
        placed.sort();
        let want: Vec<(u64, u64)> =
            layout.messages[1..].iter().map(|(at, meta, body)| (*at, layout.prefix + meta + body)).collect();
        assert_eq!(placed, want, "{name}: batches");
        let end = ev.node(&doc, &[END, 0]).unwrap();
        assert_eq!(end.offset_bits / 8, layout.end_of_stream, "{name}: end of stream");
        assert_eq!(end.size_bits / 8, layout.prefix, "{name}: the marker is a length of zero");
        let footer = ev.node(&doc, &[FOOTER, 0]).unwrap();
        assert_eq!(footer.offset_bits / 8, layout.footer, "{name}: footer");
        assert_eq!(footer.offset_bits + footer.size_bits, doc.len_bits() - 80, "{name}: footer runs to its length");
        eprintln!("{name}: {} batches placed, end of stream at {}", placed.len(), layout.end_of_stream);
    }
}

/// The listing of a whole file, asked for the way the browser asks: in goes
/// of 5,000, starting again from the top each time.
#[test]
fn a_whole_file_listing_settles_in_goes() {
    let Some(root) = arrow_samples() else {
        eprintln!("skipped: set QUBERO_SAMPLES to the sample collection");
        return;
    };
    let mut checked = 0;
    for entry in std::fs::read_dir(&root).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "arrow") {
            continue;
        }
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let (doc, mut ev) = open(&root, &name);
        let len = doc.len_bits();
        ev.set_slice(Some(5_000));
        let mut goes = 0;
        let spans = loop {
            goes += 1;
            assert!(goes <= 400, "{name}: the listing never settled");
            ev.begin_slice();
            match ev.spans(&doc, 0, len, 20_000) {
                Ok(v) => break v,
                Err(e) if e.interrupted() => continue,
                Err(e) => panic!("{name}: {e:?}"),
            }
        };
        let gap: u64 = spans.iter().filter(|s| s.gap).map(|s| s.size_bits / 8).sum();
        // What nothing places is the padding FlatBuffers and the body put in
        // to line things up on four or eight bytes: zeros, and never a whole
        // eight of them. A table or a buffer left unplaced would be longer, or
        // would hold something.
        let bytes = std::fs::read(root.join(&name)).unwrap();
        for s in spans.iter().filter(|s| s.gap) {
            let (a, b) = ((s.offset_bits / 8) as usize, ((s.offset_bits + s.size_bits) / 8) as usize);
            assert!(b - a < 8 && bytes[a..b].iter().all(|&x| x == 0), "{name}: gap {a:#x}..{b:#x} is {:02x?}", &bytes[a..b.min(a + 16)]);
        }
        eprintln!("{name}: {} spans in {goes} goes, {gap} bytes of gap", spans.len());
        checked += 1;
    }
    assert!(checked > 0);
}
