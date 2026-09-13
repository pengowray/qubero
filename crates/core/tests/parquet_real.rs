//! Footer-directed page walks over Apache's Parquet samples. The fixtures live
//! in QUBERO_SAMPLES/parquet, or in the sibling qubero-samples collection.
use std::path::PathBuf;
use qubero_core::{document::Document, eval::Evaluator, formats, source::MemSource};

fn parquet_samples() -> Option<PathBuf> {
    let mut roots = Vec::new();
    if let Ok(paths) = std::env::var("QUBERO_SAMPLES") {
        roots.extend(paths.split(';').filter(|s| !s.is_empty()).map(PathBuf::from));
    }
    roots.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../qubero-samples"));
    roots.into_iter().map(|p| p.join("parquet")).find(|p| p.is_dir())
}

/// The listing of a whole file, asked for the way the browser asks: in goes of
/// 5,000, starting again from the top of the window each time. Every go has to
/// get further than the last. `delta_binary_packed.parquet` never finished,
/// because going back over the rows already listed cost a whole go.
#[test]
fn a_whole_file_listing_settles_in_goes() {
    let Some(root) = parquet_samples() else {
        eprintln!("skipped: set QUBERO_SAMPLES to the sample collection");
        return;
    };
    let mut checked = 0;
    for entry in std::fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "parquet") { continue; }
        let bytes = std::fs::read(&path).unwrap();
        let len = bytes.len() as u64 * 8;
        let doc = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(formats::builtin("parquet").unwrap());
        ev.set_slice(Some(5_000));
        let mut goes = 0;
        let spans = loop {
            goes += 1;
            assert!(goes <= 200, "{}: the listing never settled", path.display());
            ev.begin_slice();
            match ev.spans(&doc, 0, len, 4000) {
                Ok(v) => break v,
                Err(e) if e.interrupted() => continue,
                Err(e) => panic!("{} {e:?}", path.display()),
            }
        };
        eprintln!("{}: {} spans in {goes} goes", path.display(), spans.len());
        checked += 1;
    }
    assert!(checked > 0);
}

#[test]
fn pages_and_indexes_are_separate_in_real_files() {
    let Some(root) = parquet_samples() else {
        eprintln!("skipped: set QUBERO_SAMPLES to the sample collection");
        return;
    };
    let mut checked = 0;
    for entry in std::fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "parquet") { continue; }
        let doc = Document::new(MemSource(std::fs::read(&path).unwrap()));
        let mut ev = Evaluator::new(formats::builtin("parquet").unwrap());
        let footer = ev.node(&doc, &[1, 0]).unwrap().offset_bits;
        let mut stack = vec![Vec::new()];
        let mut pages = Vec::new();
        let mut indexes = Vec::new();
        let mut columns = Vec::new();
        while let Some(at) = stack.pop() {
            let node = ev.node(&doc, &at).unwrap_or_else(|e| panic!("{} {at:?}: {e:?}", path.display()));
            let range = node.offset_bits..node.offset_bits + node.size_bits;
            match node.type_name.as_str() {
                "Page" => pages.push((at.clone(), range.clone())),
                "ColumnPages" => columns.push(range.clone()),
                "ColumnIndex" | "OffsetIndex" | "BloomFilter" => indexes.push(range.clone()),
                _ => {}
            }
            assert!(node.child_count < 100_000, "unbounded children: {} {at:?}", path.display());
            for i in (0..node.child_count as usize).rev() {
                let mut next = at.clone(); next.push(i); stack.push(next);
            }
        }
        assert!(!pages.is_empty(), "no pages in {}", path.display());
        for (at, range) in &pages {
            assert!(range.start >= 32 && range.end <= footer);
            assert!(columns.iter().any(|c| c.start <= range.start && range.end <= c.end));
            assert!(!indexes.iter().any(|ix| ix.start < range.end && range.start < ix.end));
            let found = ev.locate(&doc, range.start).unwrap();
            assert!(found.starts_with(at), "{} page {at:?} located as {found:?}", path.display());
            let spans = ev.spans(&doc, range.start, range.end.min(range.start + 256), 64).unwrap();
            assert!(spans.iter().any(|s| !s.gap), "page missing from hex annotations");
        }
        if path.file_name().unwrap() == "data_index_bloom_encoding_stats.parquet" {
            assert!(indexes.len() >= 3, "column index, offset index and bloom filter must all be placed");
            for range in &indexes {
                let at = ev.locate(&doc, range.start).unwrap();
                assert!(!pages.iter().any(|(p, _)| at.starts_with(p)), "index mistaken for a page");
            }
        }
        eprintln!("{}: {} pages, {} columns, {} indexes/bloom filters", path.display(), pages.len(), columns.len(), indexes.len());
        checked += 1;
    }
    assert!(checked > 0);
}

/// Which codec every page payload opened with, and what came out of it.
///
/// The codec names and the first bytes are pyarrow's answers, read on
/// 2026-09-13 with `pq.ParquetFile(path).metadata.row_group(0).column(i)` and
/// `pq.read_table(path)`. A page that opens says the codec's word where it
/// used to say `bytes[]`, and the bytes behind it are the ones the values are
/// encoded in.
#[test]
fn page_payloads_open_with_the_codec_the_footer_names() {
    let Some(root) = parquet_samples() else {
        eprintln!("skipped: set QUBERO_SAMPLES to the sample collection");
        return;
    };
    // file, the word every payload's node carries, and how many pages opened.
    let expected: &[(&str, &str, bool)] = &[
        ("alltypes_plain.parquet", "stored", true),
        ("alltypes_dictionary.parquet", "stored", true),
        ("delta_binary_packed.parquet", "stored", true),
        ("datapage_v2.snappy.parquet", "snappy", true),
        ("nested_lists.snappy.parquet", "snappy", true),
        ("nan_in_stats.parquet", "snappy", true),
        ("data_index_bloom_encoding_stats.parquet", "gzip", true),
        ("byte_stream_split.zstd.parquet", "zstd", true),
        ("lz4_raw_compressed.parquet", "lz4", true),
        // Every page but two opens. The two are the point of the file: its
        // first column claims 2,147,483,749 uncompressed bytes from three
        // kilobytes of input, which is past what this will hold in memory.
        ("large_string_map.brotli.parquet", "brotli", false),
    ];
    for (name, codec, every) in expected {
        let path = root.join(name);
        let doc = Document::new(MemSource(std::fs::read(&path).unwrap()));
        let mut ev = Evaluator::new(formats::builtin("parquet").unwrap());
        let mut seen = 0;
        let mut opened = 0;
        let mut stack = vec![Vec::new()];
        while let Some(at) = stack.pop() {
            let node = ev.node(&doc, &at).unwrap();
            for i in (0..node.child_count as usize).rev() {
                let mut next = at.clone();
                next.push(i);
                stack.push(next);
            }
            // The payload of a page, or the values of a v2 one: both are the
            // run the codec was run over and both carry its word.
            if node.type_name != *codec {
                continue;
            }
            seen += 1;
            if node.child_count > 0 {
                opened += 1;
            }
        }
        assert!(seen > 0, "{name}: no payload said {codec}");
        if *every {
            assert_eq!(seen, opened, "{name}: {} of {seen} payloads did not open", seen - opened);
        } else {
            assert!(opened > 0 && opened < seen, "{name}: {opened} of {seen} opened");
        }
        eprintln!("{name}: {opened} of {seen} {codec} payloads opened");
    }
}

/// The codecs nothing here reads keep their bytes and say why by staying
/// bytes: LZO, which has no pure-Rust decoder, and the framed LZ4 of codec 5,
/// which no sample holds. Neither reaches a `Decoded`, so neither can be
/// mistaken for a run that opened.
#[test]
fn the_unread_codecs_are_not_in_the_switch() {
    let t = formats::builtin("parquet").unwrap();
    let printed = format!("{t:?}");
    assert!(printed.contains("Snappy"), "snappy should be declared");
    assert!(printed.contains("Brotli"), "brotli should be declared");
    // Nothing names an LZO or a Hadoop-framed LZ4 codec at all, here or in
    // `codec`: there is no decoder to name.
    assert!(!printed.contains("Lzo"));
}
