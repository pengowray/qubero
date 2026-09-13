//! Real BAM, BAI and CSI files from htslib's and samtools' tests.
//!
//! Four BAMs, each for a different way the stream meets its blocks:
//! `range.bam` keeps its header in the first block and its 112 records in the
//! second, `no_hdr_sq_1.bam` keeps everything in one, `mpileup.1.bam` writes
//! a header too long for one block, and `bgzf_boundaries1.bam` and
//! `bgzf_boundaries3.bam` cut a record across two blocks and across
//! seventeen. The records the side reader finds were checked field by field
//! against bamnostic 1.3 (pysam does not build on Windows); the counts, names
//! and one whole record are pinned here.
//!
//! The files live in the sample collection rather than here. Point
//! `QUBERO_SAMPLES` at it, or keep it beside the repository as
//! `qubero-samples`. With neither, the test says so and passes.

use std::path::PathBuf;

use qubero_core::document::Document;
use qubero_core::eval::{Evaluator, Value};
use qubero_core::formats;
use qubero_core::formats::bam_records::{Block, Tag};
use qubero_core::source::MemSource;

fn sample(name: &str) -> Option<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Ok(set) = std::env::var("QUBERO_SAMPLES") {
        roots.extend(set.split(';').filter(|s| !s.is_empty()).map(PathBuf::from));
    }
    roots.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../qubero-samples"));
    roots.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../qubero-samples"));
    roots.into_iter().map(|r| r.join("bam").join(name)).find(|p| p.exists())
}

/// The file, read with the template it sniffs as, which is checked.
fn read(name: &str, template: &str) -> Option<(Document<MemSource>, Evaluator)> {
    let bytes = std::fs::read(sample(name)?).unwrap();
    let sniffed = formats::sniff(&bytes[..bytes.len().min(formats::SNIFF_WINDOW)], bytes.len() as u64);
    assert_eq!(sniffed, Some(template), "{name} sniffs as {sniffed:?}");
    Some((Document::new(MemSource(bytes)), Evaluator::new(formats::template(template).unwrap())))
}

fn at(ev: &mut Evaluator, d: &Document<MemSource>, from: &[usize], names: &[&str]) -> Vec<usize> {
    let mut p = from.to_vec();
    for name in names {
        p = ev.child_named(d, &p, name).unwrap().unwrap_or_else(|| panic!("no {name} under {p:?}"));
    }
    p
}

/// What block `i` unpacks to, as the template reads it.
fn payload(ev: &mut Evaluator, d: &Document<MemSource>, i: usize) -> Vec<usize> {
    [at(ev, d, &[0, i], &["compressed"]), vec![0]].concat()
}

fn blocks(ev: &mut Evaluator, d: &Document<MemSource>) -> Vec<Block> {
    let n = ev.node(d, &[0]).unwrap().child_count as usize;
    (0..n).map(|i| ev.bam_block(d, &[0, i]).unwrap().expect("a BGZF block")).collect()
}

macro_rules! skip_without {
    ($e:expr) => {
        match $e {
            Some(v) => v,
            None => {
                eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
                return;
            }
        }
    };
}

#[test]
fn every_block_is_as_long_as_its_bc_number_and_checks_out() {
    for (name, count) in [("range.bam", 3), ("mpileup.1.bam", 7), ("no_hdr_sq_1.bam", 2), ("bgzf_boundaries3.bam", 19)] {
        let (d, mut ev) = skip_without!(read(name, "bgzf"));
        assert_eq!(ev.node(&d, &[0]).unwrap().child_count, count, "{name}");
        let mut end = 0;
        for i in 0..count as usize {
            let block = ev.node(&d, &[0, i]).unwrap();
            assert_eq!(block.offset_bits, end, "{name} block {i}");
            end += block.size_bits;
            let bsize = at(&mut ev, &d, &[0, i], &["extra", "subfields"]);
            let bsize = [bsize, vec![0, 2, 0]].concat();
            assert_eq!(ev.node(&d, &bsize).unwrap().value.as_int(), Some((block.size_bits / 8) as i128 - 1), "{name} block {i}");
            let crc = at(&mut ev, &d, &[0, i], &["crc32"]);
            assert!(ev.run_check(&d, &crc).unwrap().expect("a verdict").ok, "{name} block {i}");
        }
        assert_eq!(end, d.len_bits(), "{name}");
        // The last block is the 28-byte one that unpacks to nothing.
        let last = ev.node(&d, &[0, count as usize - 1]).unwrap();
        assert_eq!(last.size_bits, 28 * 8, "{name}");
        let size = at(&mut ev, &d, &[0, count as usize - 1], &["original_size"]);
        assert_eq!(ev.node(&d, &size).unwrap().value.as_int(), Some(0), "{name}");
    }
}

#[test]
fn the_first_block_reads_the_header_and_the_records_that_fit() {
    let (d, mut ev) = skip_without!(read("range.bam", "bgzf"));
    let p = payload(&mut ev, &d, 0);
    assert_eq!(ev.node(&d, &p).unwrap().type_name, "Bam");
    let n_ref = at(&mut ev, &d, &p, &["n_ref"]);
    assert_eq!(ev.node(&d, &n_ref).unwrap().value.as_int(), Some(7));
    let refs = at(&mut ev, &d, &p, &["references"]);
    let name = at(&mut ev, &d, &[refs.clone(), vec![6]].concat(), &["name"]);
    assert_eq!(ev.node(&d, &name).unwrap().value, Value::Str("CHROMOSOME_MtDNA".into()));
    // htslib ends the block where the header ends, so no record is in it.
    let records = at(&mut ev, &d, &p, &["records"]);
    assert_eq!(ev.node(&d, &records).unwrap().child_count, 0);

    // A file written another way keeps all six of its records in the first
    // block, and they read as fields.
    let (d, mut ev) = skip_without!(read("no_hdr_sq_1.bam", "bgzf"));
    let p = payload(&mut ev, &d, 0);
    let records = at(&mut ev, &d, &p, &["records"]);
    assert_eq!(ev.node(&d, &records).unwrap().child_count, 6);
    let cigar = at(&mut ev, &d, &[records, vec![1]].concat(), &["cigar"]);
    assert_eq!(ev.node(&d, &cigar).unwrap().child_count, 3);
}

/// `mpileup.1.bam` writes 119,396 bytes of header text, and its first block
/// unpacks to 65,280 bytes. The text is read as far as the block goes, the
/// reference count is not taken from the middle of the text, and the next
/// block, which is the rest of the text, reads as text.
#[test]
fn a_header_longer_than_a_block_is_read_as_far_as_the_block_goes() {
    let (d, mut ev) = skip_without!(read("mpileup.1.bam", "bgzf"));
    let p = payload(&mut ev, &d, 0);
    let l_text = at(&mut ev, &d, &p, &["l_text"]);
    assert_eq!(ev.node(&d, &l_text).unwrap().value.as_int(), Some(119_396));
    let text = at(&mut ev, &d, &p, &["text"]);
    assert_eq!(ev.node(&d, &text).unwrap().size_bits, (65_280 - 8) * 8);
    let n_ref = at(&mut ev, &d, &p, &["n_ref"]);
    assert_eq!(ev.node(&d, &n_ref).unwrap().size_bits, 0);
    let next = payload(&mut ev, &d, 1);
    let text = at(&mut ev, &d, &next, &["text"]);
    assert!(matches!(ev.node(&d, &text).unwrap().value, Value::Str(s) if s.starts_with("ariate\n@PG")));

    // The side reader follows the header into the second block and finds no
    // record in either; the first records are in the third.
    let found = blocks(&mut ev, &d);
    assert_eq!((found[0].header_bytes, found[0].records.len()), (65_280, 0));
    assert_eq!((found[1].header_bytes, found[1].carried, found[1].records.len()), (55_546, 0, 0));
    assert_eq!(found[1].references.len(), 86);
    assert_eq!(found[2].references[16].name, "17");
    assert_eq!(found[2].records[0].read_name, "ERR013140.3521432");
}

/// One record cut across blocks 1 to 17 of `bgzf_boundaries3.bam`, read whole
/// from block 1, and every block after it reported as carrying its bytes.
/// The fields are the ones bamnostic reads.
#[test]
fn a_record_cut_across_seventeen_blocks_is_read_from_the_block_it_starts_in() {
    let (d, mut ev) = skip_without!(read("bgzf_boundaries3.bam", "bgzf"));
    let found = blocks(&mut ev, &d);
    assert_eq!((found[0].header_bytes, found[0].carried, found[0].records.len()), (64, 0, 0));
    let block = &found[1];
    assert_eq!(block.problem, None);
    assert_eq!(block.records.len(), 1);
    let r = &block.records[0];
    assert_eq!(r.read_name, "SRR065390.14978392");
    assert_eq!((r.flag, r.ref_id, r.pos, r.mapq, r.bin), (16, 0, 1, 1, 4681));
    assert_eq!((r.next_ref_id, r.next_pos, r.tlen), (-1, -1, 0));
    assert_eq!(r.cigar, "27M1D73M");
    assert_eq!(r.seq, "CCTAGCCCTAACCCTAACCCTAACCCTAGCCTAAGCCTAAGCCTAAGCCTAAGCCTAAGCCTAAGCCTAAGCCTAAGCCTAAGCCTAAGCCTAAGCCTAA");
    assert_eq!(&r.qual[..3], &[2, 2, 2]);
    assert_eq!(r.qual.len(), 100);
    let tags: Vec<String> = r.tags.iter().map(Tag::sam).collect();
    assert_eq!(tags, ["XG:i:1", "XM:i:5", "XN:i:0", "XO:i:1", "AS:i:-18", "XS:i:-18", "YT:Z:UU"]);
    assert_eq!(r.tags.iter().map(|t| t.type_code).collect::<String>(), "CCCCccZ");
    assert_eq!(r.blocks, 17);
    // The record's first byte is the first byte block 1 unpacks to, and that
    // is the offset htslib's own index writer would give it.
    assert_eq!(r.virtual_offset(), 95 << 16);
    for (i, b) in found.iter().enumerate().take(18).skip(2) {
        assert_eq!((b.records.len(), b.carried, b.problem.as_deref()), (0, b.decoded_bytes, None), "block {i}");
    }
    assert_eq!(found[18].decoded_bytes, 0);

    // The same file with the cut after the record's first two bytes.
    let (d, mut ev) = skip_without!(read("bgzf_boundaries1.bam", "bgzf"));
    let found = blocks(&mut ev, &d);
    assert_eq!(found[1].records.len(), 1);
    assert_eq!(found[1].records[0].blocks, 2);
    assert_eq!(found[2].carried, found[2].decoded_bytes);
}

/// Every record the side reader finds, over every block, against the count
/// and the first and last read names bamnostic gives.
#[test]
fn the_side_reader_finds_every_record_once() {
    for (name, count, first, last, last_offset) in [
        ("range.bam", 112, "HS18_09653:4:1315:19857:61712", "HS18_09653:4:2302:14941:52811", 32_996_620u64),
        ("mpileup.1.bam", 569, "ERR013140.3521432", "ERR013140.6157908", 4_434_898_483),
        ("no_hdr_sq_1.bam", 6, "I", "VI", 1_431),
    ] {
        let (d, mut ev) = skip_without!(read(name, "bgzf"));
        let found = blocks(&mut ev, &d);
        assert!(found.iter().all(|b| b.problem.is_none()), "{name}: {:?}", found.iter().find_map(|b| b.problem.clone()));
        let records: Vec<_> = found.iter().flat_map(|b| b.records.iter()).collect();
        assert_eq!(records.len(), count, "{name}");
        assert_eq!(records[0].read_name, first, "{name}");
        assert_eq!(records[count - 1].read_name, last, "{name}");
        assert_eq!(records[count - 1].virtual_offset(), last_offset, "{name}");
        assert!(records.iter().all(|r| r.problem.is_none()), "{name}");
    }
}
