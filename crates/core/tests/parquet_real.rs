//! Footer-directed page walks over Apache's Parquet samples. The fixtures live
//! in QUBERO_SAMPLES/parquet, or in the sibling qubero-samples collection.
use std::path::PathBuf;
use qubero_core::{document::Document, eval::Evaluator, formats, source::MemSource};

#[test]
fn pages_and_indexes_are_separate_in_real_files() {
    let mut roots = Vec::new();
    if let Ok(paths) = std::env::var("QUBERO_SAMPLES") {
        roots.extend(paths.split(';').filter(|s| !s.is_empty()).map(PathBuf::from));
    }
    roots.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../qubero-samples"));
    let Some(root) = roots.into_iter().map(|p| p.join("parquet")).find(|p| p.is_dir()) else {
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
