//! Arrow IPC files from pyarrow, read against pyarrow's own answers. The
//! fixtures live in QUBERO_SAMPLES/arrow, or in the sibling qubero-samples
//! collection, and are written by its `tools/make_arrow_samples.py`.
use std::path::PathBuf;
use qubero_core::{document::Document, eval::{Evaluator, Value}, formats, source::MemSource};

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
        messages: &[(8, 1144, 0), (1160, 1232, 536)],
        end_of_stream: 2936,
        footer: 2944,
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

/// Every buffer of a body, as (column, what it is for, offset in the file,
/// length), in the order the batch lists them. The column is what the
/// buffer's row is called after its index, and what it is for is its type.
fn buffers_of(ev: &mut Evaluator, doc: &Document<MemSource>, batch: usize) -> Vec<(String, String, u64, u64)> {
    let body = [BATCHES, batch, 4];
    let n = ev.node(doc, &body).unwrap().child_count as usize;
    (0..n)
        .map(|i| {
            let node = ev.node(doc, &[BATCHES, batch, 4, i]).unwrap();
            let column = node.name.split_once(' ').map_or(String::new(), |(_, c)| c.to_string());
            (column, node.type_name, node.offset_bits / 8, node.size_bits / 8)
        })
        .collect()
}

/// A buffer pyarrow reports as absent is one of no bytes, whose place
/// pyarrow does not say.
const ANYWHERE: u64 = u64::MAX;

fn check_buffers(name: &str, got: &[(String, String, u64, u64)], want: &[(&str, &str, u64, u64)]) {
    assert_eq!(got.len(), want.len(), "{name}: how many buffers");
    for (i, ((column, role, at, len), (w_column, w_role, w_at, w_len))) in got.iter().zip(want).enumerate() {
        assert_eq!((column.as_str(), role.as_str()), (*w_column, *w_role), "{name}: buffer {i}");
        assert_eq!(*len, *w_len, "{name}: buffer {i} length");
        if *w_at != ANYWHERE {
            assert_eq!(*at, *w_at, "{name}: buffer {i} offset");
        }
    }
}

/// Which column every buffer of a record batch belongs to, what it is for,
/// and where it is, against pyarrow.
///
/// pyarrow's answers were read on 2026-09-14 with
/// `pyarrow.ipc.open_file(path).get_batch(0)`: for each column,
/// `column.buffers()` lists the column's buffers and then its children's, and
/// each buffer's `address` less the address of the file's own bytes is where
/// it sits in the file. A buffer of no bytes comes back as `None`.
#[test]
fn every_buffer_is_named_from_the_schema() {
    let Some(root) = arrow_samples() else {
        eprintln!("skipped: set QUBERO_SAMPLES to the sample collection");
        return;
    };
    let (doc, mut ev) = open(&root, "columns-uncompressed.arrow");
    // The footer lists the dictionary batch first, then the two record
    // batches, and the gather keeps that order.
    let first = buffers_of(&mut ev, &doc, 1);
    let want: &[(&str, &str, u64, u64)] = &[
        ("id", "validity", ANYWHERE, 0),
        ("id", "data", 1904, 20),
        ("count", "validity", 1928, 1),
        ("count", "data", 1936, 40),
        ("small", "validity", 1976, 1),
        ("small", "data", 1984, 10),
        ("ratio", "validity", 2000, 1),
        ("ratio", "data", 2008, 40),
        ("score", "validity", ANYWHERE, 0),
        ("score", "data", 2048, 20),
        ("flag", "validity", 2072, 1),
        ("flag", "data", 2080, 1),
        ("name", "validity", 2088, 1),
        ("name", "offsets", 2096, 16),
        ("name", "data", 2112, 12),
        ("colour", "validity", ANYWHERE, 0),
        ("colour", "indices", 2128, 20),
        ("seen", "validity", 2152, 1),
        ("seen", "data", 2160, 40),
        ("day", "validity", 2200, 1),
        ("day", "data", 2208, 20),
        ("tags", "validity", 2232, 1),
        ("tags", "offsets", 2240, 16),
        ("item", "validity", ANYWHERE, 0),
        ("item", "data", 2256, 48),
        ("point", "validity", 2304, 1),
        ("x", "validity", ANYWHERE, 0),
        ("x", "data", 2312, 20),
        ("label", "validity", 2336, 1),
        ("label", "offsets", 2344, 16),
        ("label", "data", 2360, 4),
    ];
    check_buffers("columns-uncompressed.arrow batch 0", &first, want);

    // The dictionary batch: the dictionary of `colour`, which pyarrow gives
    // as `column.dictionary.buffers()`, three strings with no nulls.
    let dictionary = buffers_of(&mut ev, &doc, 0);
    let want: &[(&str, &str, u64, u64)] =
        &[("colour", "validity", ANYWHERE, 0), ("colour", "offsets", 1040, 16), ("colour", "data", 1056, 12)];
    check_buffers("columns-uncompressed.arrow dictionary", &dictionary, want);

    // The less common layouts. The last column nests four levels of field,
    // and the fourth level's two buffers are past where the walk follows:
    // they are placed, and they are bytes.
    let (doc, mut ev) = open(&root, "more-types.arrow");
    let got = buffers_of(&mut ev, &doc, 0);
    let want: &[(&str, &str, u64, u64)] = &[
        ("big_text", "validity", 2400, 1),
        ("big_text", "offsets", 2408, 32),
        ("big_text", "data", 2440, 8),
        ("blob", "validity", 2448, 1),
        ("blob", "offsets", 2456, 16),
        ("blob", "data", 2472, 2),
        ("half", "validity", 2480, 1),
        ("half", "data", 2488, 6),
        ("price", "validity", 2496, 1),
        ("price", "data", 2504, 48),
        ("clock", "validity", 2552, 1),
        ("clock", "data", 2560, 12),
        ("wait", "validity", 2576, 1),
        ("wait", "data", 2584, 24),
        ("hash", "validity", 2608, 1),
        ("hash", "data", 2616, 12),
        ("grid", "validity", 2632, 1),
        ("grid", "offsets", 2640, 16),
        ("item", "validity", ANYWHERE, 0),
        ("item", "offsets", 2656, 16),
        ("item", "validity", ANYWHERE, 0),
        ("item", "data", 2672, 6),
        ("lookup", "validity", 2680, 1),
        ("lookup", "offsets", 2688, 16),
        ("entries", "validity", ANYWHERE, 0),
        ("key", "validity", ANYWHERE, 0),
        ("key", "offsets", 2704, 8),
        ("key", "data", 2712, 1),
        ("value", "validity", ANYWHERE, 0),
        ("value", "data", 2720, 4),
        ("either", "type ids", 2728, 3),
        ("either", "offsets", 2736, 12),
        ("int", "validity", ANYWHERE, 0),
        ("int", "data", 2752, 8),
        ("str", "validity", ANYWHERE, 0),
        ("str", "offsets", 2760, 8),
        ("str", "data", 2768, 1),
        ("view", "validity", 2776, 1),
        ("view", "views", 2784, 48),
        ("view", "data", 2832, 33),
        ("deep", "validity", 2872, 1),
        ("deep", "offsets", 2880, 16),
        ("item", "validity", ANYWHERE, 0),
        ("item", "offsets", 2896, 16),
        ("item", "validity", ANYWHERE, 0),
        ("item", "offsets", 2912, 12),
        ("", "bytes[]", ANYWHERE, 0),
        ("", "bytes[]", 2928, 3),
    ];
    check_buffers("more-types.arrow", &got, want);
}

/// The same table written as a stream is the file's messages with nothing
/// around them: every message eight bytes earlier than in the file, the
/// marker last, and the first batch's buffers named as the file's are.
#[test]
fn a_stream_is_its_messages_one_after_another() {
    let Some(root) = arrow_samples() else {
        eprintln!("skipped: set QUBERO_SAMPLES to the sample collection");
        return;
    };
    let bytes = std::fs::read(root.join("columns.arrows")).unwrap();
    assert_eq!(formats::sniff(&bytes[..bytes.len().min(formats::SNIFF_WINDOW)], bytes.len() as u64), Some("arrowstream"));
    let doc = Document::new(MemSource(bytes));
    let mut ev = Evaluator::new(formats::builtin("arrowstream").unwrap());
    // pyarrow.ipc.read_message from byte 0 onwards, read on 2026-09-14.
    let schema = ev.node(&doc, &[0, 0]).unwrap();
    assert_eq!((schema.offset_bits / 8, schema.size_bits / 8), (0, 856));
    let messages = ev.node(&doc, &[1, 0]).unwrap();
    let placed: Vec<(u64, u64)> = (0..messages.child_count as usize)
        .map(|i| {
            let n = ev.node(&doc, &[1, 0, i]).unwrap();
            (n.offset_bits / 8, n.size_bits / 8)
        })
        .collect();
    assert_eq!(placed, [(856, 208), (1064, 1296), (2360, 1048), (3408, 8)]);
    let body = [1, 0, 1, 4];
    let n = ev.node(&doc, &body).unwrap().child_count as usize;
    assert_eq!(n, 31);
    let tags = ev.node(&doc, &[1, 0, 1, 4, 22]).unwrap();
    assert_eq!((tags.name.as_str(), tags.type_name.as_str(), tags.offset_bits / 8), ("[22] tags", "offsets", 2240 - 8));
}

/// The numbers in a buffer, read as its column's type.
fn values(ev: &mut Evaluator, doc: &Document<MemSource>, at: &[usize]) -> Vec<Value> {
    let n = ev.node(doc, at).unwrap().child_count as usize;
    (0..n)
        .map(|i| {
            let mut p = at.to_vec();
            p.push(i);
            ev.node(doc, &p).unwrap().value
        })
        .collect()
}

/// The values the buffers read as, against `pyarrow.ipc.open_file(path)
/// .read_all().column(name).to_pylist()`, read on 2026-09-14. A null reads
/// as whatever its slot holds, which for pyarrow is nought, so the nulls are
/// noughts here.
///
/// Only the batch's own rows are values. pyarrow writing batches of three
/// writes the first batch's fixed-width buffers whole, all five rows of them,
/// and the node says three: the other two are `beyond_length`.
#[test]
fn buffers_read_as_their_columns() {
    let Some(root) = arrow_samples() else {
        eprintln!("skipped: set QUBERO_SAMPLES to the sample collection");
        return;
    };
    for name in ["columns-uncompressed.arrow", "columns-legacy.arrow", "columns-zstd.arrow", "columns-lz4.arrow"] {
        let (doc, mut ev) = open(&root, name);
        // A buffer of a compressed body is its uncompressed length and the
        // stream, and the stream opens into the same structure a plain one
        // is: [values, beyond_length].
        let data = |buffer: usize| -> Vec<usize> {
            let mut p = vec![BATCHES, 1, 4, buffer];
            if name.contains("zstd") || name.contains("lz4") {
                p.extend([1, 0]);
            }
            p.push(0);
            p
        };
        let ints = |v: Vec<Value>| v.iter().map(|x| x.as_int().unwrap()).collect::<Vec<_>>();
        assert_eq!(ints(values(&mut ev, &doc, &data(1))), [1, 2, 3], "{name}: id");
        assert_eq!(ints(values(&mut ev, &doc, &data(3))), [10, 0, 30], "{name}: count");
        assert_eq!(ints(values(&mut ev, &doc, &data(5))), [7, 8, 0], "{name}: small");
        assert_eq!(values(&mut ev, &doc, &data(7)), [Value::Float(0.5), Value::Float(1.25), Value::Float(0.0)], "{name}: ratio");
        // Booleans are bits, low bit first: the byte's bits, then the values.
        let mut flag = data(11);
        flag.push(0);
        let flags: Vec<i128> = ints(values(&mut ev, &doc, &flag));
        assert_eq!(flags.len(), 3, "{name}: one bit per row of the batch");
        assert_eq!(&flags[..2], [1, 0], "{name}: flag");
        assert_eq!(ints(values(&mut ev, &doc, &data(13))), [0, 3, 6, 6], "{name}: name offsets");
        // Indices into ['red', 'green', 'blue']: red, green, red.
        assert_eq!(ints(values(&mut ev, &doc, &data(16))), [0, 1, 0], "{name}: colour indices");
        // Milliseconds since 1970 for 2026-09-14 09:30 and 12:00:00.25 UTC.
        let mut seen = data(18);
        seen.push(0);
        assert_eq!(ints(values(&mut ev, &doc, &seen)), [1_789_378_200_000, 1_789_387_200_250, 0], "{name}: seen");
        // Days since 1970 for 2026-09-14, a null, and 1969-12-31.
        let mut day = data(20);
        day.push(0);
        assert_eq!(ints(values(&mut ev, &doc, &day)), [20_710, 0, -1], "{name}: day");
        assert_eq!(ints(values(&mut ev, &doc, &data(22))), [0, 2, 2, 2], "{name}: tags offsets");
        assert_eq!(ints(values(&mut ev, &doc, &data(24))), [1, 2], "{name}: tags values");
        assert_eq!(ints(values(&mut ev, &doc, &data(27))), [1, 2, 0], "{name}: point.x");
        eprintln!("{name}: values of batch 0 match pyarrow");
    }
    // The views of a string view column: a short value whole, a null, and a
    // long one's first four bytes and where the rest is.
    let (doc, mut ev) = open(&root, "more-types.arrow");
    // views -> values -> [i] -> length, value.
    let views = [BATCHES, 0, 4, 38, 0];
    assert_eq!(ev.node(&doc, &views).unwrap().child_count, 3);
    assert_eq!(ev.node(&doc, &[BATCHES, 0, 4, 38, 0, 0, 1]).unwrap().value, Value::Str("short".into()));
    assert_eq!(ev.node(&doc, &[BATCHES, 0, 4, 38, 0, 2, 0]).unwrap().value.as_int(), Some(33));
    assert_eq!(ev.node(&doc, &[BATCHES, 0, 4, 38, 0, 2, 1]).unwrap().type_name, "ViewReference");
}

/// Every buffer of the compressed files, opened, is byte for byte the buffer
/// the uncompressed file holds in the same place: the same table written three
/// ways by the same script. A buffer compressing did not help is written as it
/// was behind a length of -1, and is compared as it stands.
///
/// This is the check on the LZ4 frames that does not go through the frame
/// reader's own idea of what a frame is: pyarrow wrote them with liblz4, and
/// what they have to come to is what pyarrow wrote uncompressed.
#[test]
fn every_compressed_buffer_opens_to_the_uncompressed_files_buffer() {
    let Some(root) = arrow_samples() else {
        eprintln!("skipped: set QUBERO_SAMPLES to the sample collection");
        return;
    };
    let (plain_doc, mut plain) = open(&root, "columns-uncompressed.arrow");
    let plain_bytes = std::fs::read(root.join("columns-uncompressed.arrow")).unwrap();
    for name in ["columns-lz4.arrow", "columns-zstd.arrow"] {
        let (doc, mut ev) = open(&root, name);
        let bytes = std::fs::read(root.join(name)).unwrap();
        let batches = ev.node(&doc, &[BATCHES]).unwrap().child_count as usize;
        assert_eq!(batches, plain.node(&plain_doc, &[BATCHES]).unwrap().child_count as usize, "{name}");
        let (mut opened, mut stored, mut empty) = (0, 0, 0);
        let mut shapes = std::collections::BTreeSet::new();
        for batch in 0..batches {
            let count = ev.node(&doc, &[BATCHES, batch, 4]).unwrap().child_count as usize;
            assert_eq!(count, plain.node(&plain_doc, &[BATCHES, batch, 4]).unwrap().child_count as usize, "{name} batch {batch}");
            for i in 0..count {
                let want = plain.node(&plain_doc, &[BATCHES, batch, 4, i]).unwrap();
                let want = &plain_bytes[(want.offset_bits / 8) as usize..((want.offset_bits + want.size_bits) / 8) as usize];
                let node = ev.node(&doc, &[BATCHES, batch, 4, i]).unwrap();
                if node.size_bits == 0 {
                    assert!(want.is_empty(), "{name} batch {batch} buffer {i}: no bytes here, {} there", want.len());
                    empty += 1;
                    continue;
                }
                let at = (node.offset_bits / 8) as usize;
                let end = ((node.offset_bits + node.size_bits) / 8) as usize;
                let length = i64::from_le_bytes(bytes[at..at + 8].try_into().unwrap());
                if length == -1 {
                    assert_eq!(&bytes[at + 8..end], want, "{name} batch {batch} buffer {i}, stored");
                    stored += 1;
                    continue;
                }
                assert_eq!(length as usize, want.len(), "{name} batch {batch} buffer {i}: the length in front");
                let id = ev
                    .open_space(&doc, 0, &[BATCHES, batch, 4, i, 1])
                    .unwrap()
                    .unwrap_or_else(|| panic!("{name} batch {batch} buffer {i} opens"));
                assert_eq!(ev.space(id).unwrap().bytes(), want, "{name} batch {batch} buffer {i}");
                ev.space(id).unwrap().trace().check_tiles().unwrap_or_else(|e| panic!("{name} batch {batch} buffer {i}: {e}"));
                // And the blocks read as rows, the way the listing reads them.
                // The frames here are liblz4's rather than the ones the unit
                // tests pack, so this is where a shape those never make turns
                // up.
                let mut names = Vec::new();
                walk(&doc, &mut ev, &[BATCHES, batch, 4, i, 1, 1], 3, &mut names);
                shapes.extend(names.into_iter().filter(|n| !n.starts_with("literal ") && !n.starts_with("match ")));
                opened += 1;
            }
        }
        assert!(opened > 20, "{name}: {opened} buffers opened");
        eprintln!("{name}: {opened} buffers opened, {stored} stored, {empty} empty, all as the uncompressed file has them");
        eprintln!("{name}: the rows under their blocks, symbols aside, are {shapes:?}");
    }
}

/// Every row under `at`, `depth` levels down, read one at a time the way the
/// listing reads them, each inside the row it is under. A run of thousands of
/// codes is read at its two ends, which is where a row that does not fit
/// would be.
fn walk(doc: &Document<MemSource>, ev: &mut Evaluator, at: &[usize], depth: u32, names: &mut Vec<String>) {
    let node = ev.node(doc, at).unwrap_or_else(|e| panic!("{at:?}: {e:?}"));
    names.push(node.name.clone());
    if depth == 0 {
        return;
    }
    let n = node.child_count as usize;
    for k in (0..n.min(64)).chain(n.saturating_sub(4).max(64)..n) {
        let path = [at, &[k]].concat();
        let child = ev.node(doc, &path).unwrap_or_else(|e| panic!("{path:?}: {e:?}"));
        assert!(
            child.offset_bits >= node.offset_bits && child.offset_bits + child.size_bits <= node.offset_bits + node.size_bits,
            "{path:?} ({}) is not inside {at:?} ({})",
            child.name,
            node.name
        );
        walk(doc, ev, &path, depth - 1, names);
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
        if path.extension().is_none_or(|e| e != "arrow" && e != "arrows") {
            continue;
        }
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let bytes = std::fs::read(&path).unwrap();
        let template = formats::sniff(&bytes[..bytes.len().min(formats::SNIFF_WINDOW)], bytes.len() as u64).unwrap();
        let doc = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(formats::builtin(template).unwrap());
        let len = doc.len_bits();
        ev.set_slice(Some(5_000));
        let mut goes = 0;
        let spans = loop {
            goes += 1;
            assert!(goes <= 200, "{name}: the listing never settled");
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
        eprintln!("{name}: {} spans in {goes} goes, {} of {} bytes named, {gap} bytes of padding", spans.len(), len / 8 - gap, len / 8);
        checked += 1;
    }
    assert!(checked > 0);
}

/// Where `steps` lead from the root, by field name. A name that is not a child
/// of the node reached is looked for inside the one thing that node holds,
/// which is how a pointer is stepped through.
fn path_to(ev: &mut Evaluator, doc: &Document<MemSource>, steps: &[&str]) -> Vec<usize> {
    let mut path = Vec::new();
    for step in steps {
        if let Ok(i) = step.parse::<usize>() {
            path.push(i);
            continue;
        }
        path = match ev.child_named(doc, &path, step).unwrap() {
            Some(p) => p,
            None => {
                path.push(0);
                ev.child_named(doc, &path, step).unwrap().unwrap_or_else(|| panic!("no {step} under {path:?}"))
            }
        };
    }
    path
}

/// What three of the fields the node walk adds say for node `i`, by name and
/// value.
fn node_walk_of(ev: &mut Evaluator, doc: &Document<MemSource>, nodes: &[usize], i: usize) -> Vec<(String, Value)> {
    ["first_buffer", "depth", "column"]
        .iter()
        .map(|field| {
            let mut p = nodes.to_vec();
            p.push(i);
            let p = ev.child_named(doc, &p, field).unwrap().unwrap_or_else(|| panic!("node {i} has no {field}"));
            let n = ev.node(doc, &p).unwrap_or_else(|e| panic!("node {i} {field}: {e:?}"));
            (n.name, n.value)
        })
        .collect()
}

/// The nodes of a record batch, each read first with nothing asked before it,
/// say what they say when the nodes are read in order.
///
/// A node works out which schema field it stands for from the node before, so
/// read first, a late node of this file asks its way back through every node
/// before it: more expressions open at once than a read may have. Such a read
/// is refused, then asked again a stretch of the chain at a time, and reads
/// through. Before that was done, a field tree opened straight at the last
/// node said "nested too deep" there.
#[test]
fn every_node_read_first_says_what_it_says_in_order() {
    let Some(root) = arrow_samples() else {
        eprintln!("skipped: set QUBERO_SAMPLES to the sample collection");
        return;
    };
    let steps = ["batches", "0", "metadata", "root", "table", "header", "table", "nodes", "vector", "elements"];
    let (doc, mut in_order) = open(&root, "more-types.arrow");
    let nodes = path_to(&mut in_order, &doc, &steps);
    let count = in_order.node(&doc, &nodes).unwrap().child_count as usize;
    assert!(count > 20, "{count} nodes");
    let want: Vec<_> = (0..count).map(|i| node_walk_of(&mut in_order, &doc, &nodes, i)).collect();
    assert!(in_order.deepest_question() < 88, "in order: {} deep", in_order.deepest_question());
    let mut refused_before = 0;
    for (i, want) in want.iter().enumerate() {
        let mut first = Evaluator::new(formats::builtin("arrow").unwrap());
        assert_eq!(&node_walk_of(&mut first, &doc, &nodes, i), want, "node {i} read first");
        // The limit was reached on the way, which is the case this is for.
        if first.deepest_question() == 88 {
            refused_before += 1;
        }
    }
    assert!(refused_before > 0, "no node read first went as deep as the limit");
    eprintln!("more-types.arrow: {count} nodes read first say what they say in order, {refused_before} of them past the limit");
}
