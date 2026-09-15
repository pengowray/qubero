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
        let mut row_groups = Vec::new();
        while let Some(at) = stack.pop() {
            let node = ev.node(&doc, &at).unwrap_or_else(|e| panic!("{} {at:?}: {e:?}", path.display()));
            let range = node.offset_bits..node.offset_bits + node.size_bits;
            // A column chunk and a row group are regions of the file before
            // the footer. The footer's entries for them carry the same names.
            match node.type_name.as_str() {
                "Page" => pages.push((at.clone(), range.clone())),
                "ColumnChunk" if range.start < footer => columns.push(range.clone()),
                "RowGroup" if range.start < footer => row_groups.push(range.clone()),
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
            assert!(row_groups.iter().any(|g| g.start <= range.start && range.end <= g.end));
            assert!(!indexes.iter().any(|ix| ix.start < range.end && range.start < ix.end));
            let found = ev.locate(&doc, range.start).unwrap();
            assert!(found.starts_with(at), "{} page {at:?} located as {found:?}", path.display());
            // The cursor's path goes through the row group, then the column
            // chunk, then the page, and not through the footer.
            let kinds: Vec<String> = (0..=at.len()).map(|k| ev.node(&doc, &at[..k]).unwrap().type_name).collect();
            let place = |name: &str| kinds.iter().position(|k| k == name);
            assert!(place("RowGroup").is_some() && place("FileMetaData").is_none(), "{} {kinds:?}", path.display());
            assert!(place("RowGroup") < place("ColumnChunk") && place("ColumnChunk") < place("Page"), "{} {kinds:?}", path.display());
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
        eprintln!(
            "{}: {} pages, {} row groups, {} columns, {} indexes/bloom filters",
            path.display(),
            pages.len(),
            row_groups.len(),
            columns.len(),
            indexes.len()
        );
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
        ("lz4_raw_compressed.parquet", "lz4 block", true),
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

/// The first dictionary page of a file, read as the values it holds.
///
/// Every expectation is what pyarrow gives for the same column, read on
/// 2026-09-13 with `pq.read_table(path).column(i).to_pylist()`: a dictionary
/// page holds the distinct values of its column, so for these files it is the
/// column itself with the repeats taken out.
#[test]
fn dictionary_pages_read_as_their_values() {
    let Some(root) = parquet_samples() else {
        eprintln!("skipped: set QUBERO_SAMPLES to the sample collection");
        return;
    };
    // file, which dictionary page counting from the front of the file, the
    // type the values read as, and them. Counted that way because a column
    // without one writes no page: `alltypes_dictionary`'s booleans are the
    // second column and there is no second dictionary page.
    let cases: &[(&str, usize, &str, &[i64])] = &[
        ("alltypes_dictionary.parquet", 0, "i32 le", &[0, 1]),
        ("alltypes_dictionary.parquet", 4, "i64 le", &[0, 10]),
        ("repeated_no_annotation.parquet", 0, "i32 le", &[1, 2, 3, 4, 5, 6]),
    ];
    for (name, nth, ty, want) in cases {
        let path = root.join(name);
        let doc = Document::new(MemSource(std::fs::read(&path).unwrap()));
        let mut ev = Evaluator::new(formats::builtin("parquet").unwrap());
        let dict = dictionary_values(&mut ev, &doc, *nth);
        let values: Vec<i64> = dict
            .iter()
            .map(|p| ev.node(&doc, p).unwrap().value.as_int().unwrap() as i64)
            .collect();
        assert_eq!(&values[..], *want, "{name} dictionary page {nth}");
        assert_eq!(ev.node(&doc, &dict[0]).unwrap().type_name, *ty, "{name} dictionary page {nth}");
        eprintln!("{name} dictionary page {nth}: {values:?}");
    }

    // A byte array dictionary carries its own lengths, so each value is a
    // record rather than a number. pyarrow: [b'0', b'1'].
    let path = root.join("alltypes_dictionary.parquet");
    let doc = Document::new(MemSource(std::fs::read(&path).unwrap()));
    let mut ev = Evaluator::new(formats::builtin("parquet").unwrap());
    let dict = dictionary_values(&mut ev, &doc, 8);
    assert_eq!(dict.len(), 2);
    for (at, want) in dict.iter().zip([b'0', b'1']) {
        assert_eq!(ev.node(&doc, at).unwrap().type_name, "ByteArray");
        let mut length = at.clone();
        length.push(0);
        assert_eq!(ev.node(&doc, &length).unwrap().value.as_int(), Some(1));
        let mut bytes = at.clone();
        bytes.push(1);
        let read = ev.node(&doc, &bytes).unwrap().value;
        assert_eq!(read, qubero_core::eval::Value::Bytes { len: 1, preview: vec![want] });
    }
}

/// The paths of the values inside the `nth` dictionary page of the file,
/// counting in the order the column chunks are declared.
fn dictionary_values(ev: &mut Evaluator, doc: &Document<MemSource>, nth: usize) -> Vec<Vec<usize>> {
    let mut stack = vec![Vec::new()];
    let mut pages = Vec::new();
    while let Some(at) = stack.pop() {
        let node = ev.node(doc, &at).unwrap();
        for i in (0..node.child_count as usize).rev() {
            let mut next = at.clone();
            next.push(i);
            stack.push(next);
        }
        // A page's second field is its type, worked out from the header.
        // DICTIONARY_PAGE is 2.
        if node.type_name == "Page" {
            let mut kind = at.clone();
            kind.push(1);
            if ev.node(doc, &kind).unwrap().value.as_int() == Some(2) {
                pages.push(at);
            }
        }
    }
    pages.sort();
    let page = pages.get(nth).unwrap_or_else(|| panic!("no dictionary page {nth}"));
    // The page's payload, the space the codec opened, and the values in it.
    let mut values = page.clone();
    values.extend([2, 0]);
    let n = ev.node(doc, &values).unwrap().child_count as usize;
    (0..n)
        .map(|i| {
            let mut p = values.clone();
            p.push(i);
            p
        })
        .collect()
}

/// A `DATA_PAGE_V2` keeps its levels out of the packed run, and its dictionary
/// indices read as the hybrid's runs.
///
/// pyarrow for `datapage_v2.snappy.parquet` column `c`: [2.0, 3.0, 4.0, 5.0,
/// 2.0]. Four distinct values, so an index needs two bits, and five indices
/// fit in one bit-packed group of eight: one run of two bytes behind a width
/// byte of 2.
#[test]
fn a_v2_data_page_reads_its_levels_and_its_indices() {
    let Some(root) = parquet_samples() else {
        eprintln!("skipped: set QUBERO_SAMPLES to the sample collection");
        return;
    };
    let path = root.join("datapage_v2.snappy.parquet");
    let doc = Document::new(MemSource(std::fs::read(&path).unwrap()));
    let mut ev = Evaluator::new(formats::builtin("parquet").unwrap());
    let mut stack = vec![Vec::new()];
    let mut widths = Vec::new();
    let mut levels = Vec::new();
    while let Some(at) = stack.pop() {
        let node = ev.node(&doc, &at).unwrap();
        for i in (0..node.child_count as usize).rev() {
            let mut next = at.clone();
            next.push(i);
            stack.push(next);
        }
        if node.type_name == "DictionaryIndices" {
            let mut width = at.clone();
            width.push(0);
            widths.push((at.clone(), ev.node(&doc, &width).unwrap().value.as_int().unwrap()));
        }
        if node.type_name == "DataPageV2Payload" {
            let mut rep = at.clone();
            rep.push(0);
            let mut def = at.clone();
            def.push(1);
            levels.push((
                ev.node(&doc, &rep).unwrap().size_bits / 8,
                ev.node(&doc, &def).unwrap().size_bits / 8,
            ));
        }
    }
    widths.sort();
    // Three of the five columns are dictionary encoded: a, c and e.
    let found: Vec<i128> = widths.iter().map(|(_, w)| *w).collect();
    assert_eq!(found, vec![0, 2, 2], "index widths, in column order");
    // Column a is optional, so it has definition levels and no repetition
    // levels; b, c and d are required and have neither; e is a list, so it has
    // both. pyarrow: a has one null, e has two.
    levels.sort();
    assert!(levels.contains(&(0, 0)), "a required column writes no levels");
    assert!(levels.contains(&(0, 2)), "an optional column writes definition levels only");
    assert!(levels.iter().any(|(r, d)| *r > 0 && *d > 0), "a list column writes both");
    eprintln!("datapage_v2.snappy.parquet: index widths {found:?}, levels {levels:?}");
}

/// Every page of every sample, read the whole way by the side reader.
///
/// The first values of each are pyarrow's, read on 2026-09-13 with
/// `pq.read_table(path).column(i).to_pylist()`. A data page holds the column;
/// a dictionary page holds its distinct values, which for these files is the
/// column with the repeats taken out.
#[test]
fn the_side_reader_reads_every_page() {
    use qubero_core::eval::Explain;
    let Some(root) = parquet_samples() else {
        eprintln!("skipped: set QUBERO_SAMPLES to the sample collection");
        return;
    };
    // file, which page counting from the front, and the first values of it.
    let cases: &[(&str, usize, &[&str])] = &[
        // PLAIN v1 with BIT_PACKED levels, gzip.
        ("data_index_bloom_encoding_stats.parquet", 0, &["Hello", "This is", "a", "test"]),
        // PLAIN v1, LZ4_RAW, a required INT64 column with no levels at all.
        ("lz4_raw_compressed.parquet", 0, &["1593604800", "1593604800", "1593604801", "1593604801"]),
        // DELTA_BINARY_PACKED, uncompressed: 200 values a block.
        ("delta_binary_packed.parquet", 1, &["0", "-1", "-1", "-1"]),
        // BYTE_STREAM_SPLIT under zstd.
        ("byte_stream_split.zstd.parquet", 0, &["1.7640524", "0.4001572", "0.978738", "2.2408931"]),
        // RLE booleans behind their four-byte length. pyarrow gives
        // [True, False, None, True, ...]: a null is written in the definition
        // levels and nowhere else, so the values are the list with the Nones
        // taken out.
        ("rle_boolean_encoding.parquet", 0, &["true", "false", "true", "true"]),
        // A dictionary page of doubles, snappy.
        ("nan_in_stats.parquet", 0, &["1", "NaN"]),
    ];
    for (name, nth, want) in cases {
        let path = root.join(name);
        let doc = Document::new(MemSource(std::fs::read(&path).unwrap()));
        let mut ev = Evaluator::new(formats::builtin("parquet").unwrap());
        let pages = every_page(&mut ev, &doc);
        let at = &pages[*nth];
        let Explain::ParquetPage { steps, values, total, element_type, problem, .. } =
            ev.explain(&doc, at, None).unwrap()
        else {
            panic!("{name}: page {nth} did not read as a page");
        };
        assert_eq!(problem, None, "{name} page {nth}");
        assert!(!steps.is_empty(), "{name} page {nth}: no steps");
        let shown: Vec<&str> = values.iter().take(want.len()).map(String::as_str).collect();
        assert_eq!(&shown[..], *want, "{name} page {nth}, as {element_type}");
        eprintln!(
            "{name} page {nth}: {total} {element_type}, steps {:?}",
            steps.iter().map(|s| format!("{} {}->{} {}", s.what, s.in_bytes, s.out_bytes, s.note)).collect::<Vec<_>>()
        );
    }
}

/// Every page of every sample reads, or says in words why it does not.
#[test]
fn no_page_of_any_sample_is_left_unexplained() {
    use qubero_core::eval::Explain;
    let Some(root) = parquet_samples() else {
        eprintln!("skipped: set QUBERO_SAMPLES to the sample collection");
        return;
    };
    let mut read = 0;
    let mut refused = Vec::new();
    for entry in std::fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "parquet") {
            continue;
        }
        let doc = Document::new(MemSource(std::fs::read(&path).unwrap()));
        let mut ev = Evaluator::new(formats::builtin("parquet").unwrap());
        for at in every_page(&mut ev, &doc) {
            let Explain::ParquetPage { values, total, problem, .. } = ev.explain(&doc, &at, None).unwrap() else {
                panic!("{}: {at:?} did not read as a page", path.display());
            };
            match problem {
                Some(why) => refused.push(format!("{}: {why}", path.file_name().unwrap().to_string_lossy())),
                None => {
                    assert!(total > 0 || values.is_empty(), "{}: {at:?} read no values and said nothing", path.display());
                    read += 1;
                }
            }
        }
    }
    // The two brotli pages of `large_string_map.brotli.parquet`, which claim
    // 2,147,483,749 uncompressed bytes from three kilobytes of input. That is
    // what the file is for.
    assert_eq!(refused.len(), 2, "refused: {refused:?}");
    assert!(refused.iter().all(|r| r.starts_with("large_string_map")), "{refused:?}");
    assert!(read > 50, "only {read} pages read");
    eprintln!("{read} pages read, {} refused", refused.len());
}

/// Every page of a file, in the order the column chunks are declared.
fn every_page(ev: &mut Evaluator, doc: &Document<MemSource>) -> Vec<Vec<usize>> {
    let mut stack = vec![Vec::new()];
    let mut pages = Vec::new();
    while let Some(at) = stack.pop() {
        let node = ev.node(doc, &at).unwrap();
        for i in (0..node.child_count as usize).rev() {
            let mut next = at.clone();
            next.push(i);
            stack.push(next);
        }
        if node.type_name == "Page" {
            pages.push(at);
        }
    }
    pages.sort();
    pages
}

/// The schema walk: how deep each column sits, as the side reader worked it
/// out, against pyarrow's own answer.
///
/// A wrong maximum level does not stop a page reading. It reads the levels at
/// the wrong width, the values start at the wrong byte, and what comes out is
/// plausible nonsense with no problem attached, so nothing else here would
/// notice. The expectations are `pq.ParquetFile(path).schema.column(i)`'s
/// `max_definition_level`, `max_repetition_level` and `length`, read on
/// 2026-09-14.
#[test]
fn the_schema_walk_finds_each_columns_levels() {
    use qubero_core::eval::Explain;
    let Some(root) = parquet_samples() else {
        eprintln!("skipped: set QUBERO_SAMPLES to the sample collection");
        return;
    };
    // file, and for each column chunk in order: max definition level, max
    // repetition level.
    let cases: &[(&str, &[(u32, u32)])] = &[
        ("nested_lists.snappy.parquet", &[(7, 3), (0, 0)]),
        ("repeated_no_annotation.parquet", &[(0, 0), (2, 1), (3, 1)]),
        ("datapage_v2.snappy.parquet", &[(1, 0), (0, 0), (0, 0), (0, 0), (2, 1)]),
        ("rle_boolean_encoding.parquet", &[(1, 0)]),
        ("fixed_length_byte_array.parquet", &[(1, 0)]),
        ("int32_with_null_pages.parquet", &[(1, 0)]),
    ];
    let mut checked = 0;
    for (name, want) in cases {
        let path = root.join(name);
        let doc = Document::new(MemSource(std::fs::read(&path).unwrap()));
        let mut ev = Evaluator::new(formats::builtin("parquet").unwrap());
        for at in every_page(&mut ev, &doc) {
            let column = column_of(&mut ev, &doc, &at);
            // A dictionary page has no levels at all, whatever its column's
            // depth; its second field is its type, and DICTIONARY_PAGE is 2.
            let mut kind = at.clone();
            kind.push(1);
            if ev.node(&doc, &kind).unwrap().value.as_int() == Some(2) {
                continue;
            }
            let Explain::ParquetPage { steps, .. } = ev.explain(&doc, &at, None).unwrap() else {
                panic!("{name}: {at:?} did not read as a page");
            };
            let (definition, repetition) = want[column];
            // A level list names its maximum in its note; one that is not
            // there at all was left out because its maximum is zero, which is
            // the only reason a v1 page leaves one out.
            for (list, max) in [("definition levels", definition), ("repetition levels", repetition)] {
                match steps.iter().find(|s| s.what == list) {
                    Some(step) => {
                        let said = step.note.split("max level ").nth(1).and_then(|t| t.split(',').next());
                        assert_eq!(said, Some(max.to_string().as_str()), "{name} column {column}: {}", step.note);
                        checked += 1;
                    }
                    None => {
                        assert!(max == 0, "{name} column {column}: no {list} step, but pyarrow says {max}");
                    }
                }
            }
        }
    }
    assert!(checked > 10, "only {checked} level steps checked");

    // The width of a fixed-length value is the other thing only the schema
    // knows. pyarrow: `flba_field` is 4 bytes, and `x` is a float16 in 2.
    for (name, word) in [("fixed_length_byte_array.parquet", "4-byte array"), ("float16_nonzeros_and_nans.parquet", "2-byte array")] {
        let path = root.join(name);
        let doc = Document::new(MemSource(std::fs::read(&path).unwrap()));
        let mut ev = Evaluator::new(formats::builtin("parquet").unwrap());
        let pages = every_page(&mut ev, &doc);
        let Explain::ParquetPage { element_type, problem, .. } = ev.explain(&doc, &pages[0], None).unwrap() else {
            panic!("{name}: the first page did not read as a page");
        };
        assert_eq!(problem, None, "{name}");
        assert_eq!(element_type, word, "{name}");
    }
}

/// Which column chunk a page belongs to, counting from zero in the order the
/// row group lists them.
fn column_of(ev: &mut Evaluator, doc: &Document<MemSource>, page: &[usize]) -> usize {
    let mut at = page.to_vec();
    while !at.is_empty() {
        if ev.node(doc, &at).unwrap().type_name == "ColumnChunk" {
            return *at.last().unwrap();
        }
        at.pop();
    }
    panic!("{page:?} is not under a column chunk");
}

/// A column chunk names the footer entry that placed it and the fields its
/// offset and length were read from in Parquet's own names, and keeps the path
/// through Thrift's lists, entries and values beside each.
///
/// The indices in the stored paths are places in a list of fields, not field
/// ids: `alltypes_plain.parquet` writes no `key_value_metadata` in its column
/// metadata, so `dictionary_page_offset`, id 11, is ninth.
#[test]
fn a_column_chunk_names_its_footer_entry_in_parquets_own_terms() {
    use qubero_core::eval::Role;
    let Some(root) = parquet_samples() else {
        eprintln!("skipped: set QUBERO_SAMPLES to the sample collection");
        return;
    };
    let path = root.join("alltypes_plain.parquet");
    if !path.is_file() {
        eprintln!("skipped: {} is not in the collection", path.display());
        return;
    }
    let doc = Document::new(MemSource(std::fs::read(&path).unwrap()));
    let mut ev = Evaluator::new(formats::builtin("parquet").unwrap());
    // The first column chunk of the first row group, which starts at 0x4.
    let chunk = [4, 0, 0, 0];
    assert_eq!(ev.node(&doc, &chunk).unwrap().offset_bits, 4 * 8);
    let mut seen: Vec<(Role, String, Option<String>, String)> = Vec::new();
    for o in ev.origins(&doc, &chunk).unwrap() {
        let row = (o.role, o.label, o.stored, o.value);
        if !seen.contains(&row) {
            seen.push(row);
        }
    }
    let row = |role, label: &str, stored: &str, value: &str| (role, label.to_string(), Some(stored.to_string()), value.to_string());
    assert_eq!(
        seen,
        [
            row(Role::Position, "footer.row_groups[0].columns[0]", "footer.fields.row_groups.value.elems[0].fields.columns.value.elems[0]", ""),
            row(Role::Position, "meta_data.dictionary_page_offset", "fields[id = 3].value.fields[8].value", "4"),
            row(Role::Position, "meta_data.data_page_offset", "fields[id = 3].value.fields[7].value", "49"),
            row(Role::Length, "meta_data.total_compressed_size", "fields[id = 3].value.fields[6].value", "73"),
        ]
    );
    // The formula names the same field the row above it does, and keeps the
    // template's own spelling beside it.
    let length = ev.relations(&doc, &chunk).unwrap().into_iter().find(|r| r.role == Role::Length).expect("a length formula");
    assert_eq!(length.written, "max(min(descriptor.meta_data.total_compressed_size, remaining), 0)");
    assert_eq!(length.template.as_deref(), Some("max(min(descriptor.(fields[id = 3].value.fields[id = 7].value), remaining), 0)"));
    assert_eq!((length.substituted.as_str(), length.result.as_str()), ("max(min(73, 1064), 0)", "73"));
    // A page's payload is read as whatever its header's type says.
    let payload = [4, 0, 0, 0, 0, 0, 2];
    let kind = ev.origins(&doc, &payload).unwrap().swap_remove(0);
    assert_eq!(
        (kind.role, kind.label.as_str(), kind.stored.as_deref(), kind.value.as_str()),
        (Role::Type, "header.type", Some("header.fields[0].value"), "DICTIONARY_PAGE")
    );
}
