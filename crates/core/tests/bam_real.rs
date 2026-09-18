//! Real BAM, BAI and CSI files from htslib's and samtools' tests.
//!
//! Five BAMs, for the different ways the stream meets its blocks:
//! `range.bam` keeps its header in the first block and its 112 records in the
//! second, `no_hdr_sq_1.bam` keeps everything in one, `mpileup.1.bam` writes
//! a header too long for one block, and `bgzf_boundaries1.bam` and
//! `bgzf_boundaries3.bam` cut a record across two blocks and across
//! seventeen. The records the side reader finds were checked field by field
//! against bamnostic 1.3 (pysam does not build on Windows); the counts, names
//! and one whole record are pinned here. The template reads every record out
//! of the blocks joined, and is checked against the side reader record by
//! record.
//!
//! Two BAIs, checked against bamnostic and against the records their BAMs
//! hold, and two CSIs, which are BGZF files and read out of their joined
//! stream.
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
    qubero_samples::roots().into_iter().map(|r| r.join("bam").join(name)).find(|p| p.exists())
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

/// What the blocks join to, as the template reads it.
fn stream(ev: &mut Evaluator, d: &Document<MemSource>) -> Vec<usize> {
    [at(ev, d, &[], &["stream"]), vec![0]].concat()
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
                eprintln!("{}", qubero_samples::missing());
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

/// `range.bam` keeps its header in the first block and its 112 records in the
/// second, and every one of them is a field of the joined stream. The last one
/// ends where the stream does.
#[test]
fn every_record_of_range_bam_is_a_field() {
    let (d, mut ev) = skip_without!(read("range.bam", "bgzf"));
    let p = stream(&mut ev, &d);
    assert_eq!(ev.node(&d, &p).unwrap().type_name, "Bam");
    let n_ref = at(&mut ev, &d, &p, &["n_ref"]);
    assert_eq!(ev.node(&d, &n_ref).unwrap().value.as_int(), Some(7));
    let refs = at(&mut ev, &d, &p, &["references"]);
    let name = at(&mut ev, &d, &[refs.clone(), vec![6]].concat(), &["name"]);
    assert_eq!(ev.node(&d, &name).unwrap().value, Value::Str("CHROMOSOME_MtDNA".into()));
    let records = at(&mut ev, &d, &p, &["records"]);
    assert_eq!(ev.node(&d, &records).unwrap().child_count, 112);
    let last = [records.clone(), vec![111]].concat();
    let name = at(&mut ev, &d, &last, &["read_name"]);
    assert_eq!(ev.node(&d, &name).unwrap().value, Value::Str("HS18_09653:4:2302:14941:52811".into()));
    let (node, whole) = (ev.node(&d, &last).unwrap(), ev.node(&d, &p).unwrap());
    assert_eq!(node.offset_bits + node.size_bits, whole.size_bits);

    // A file written another way keeps all six of its records in the first
    // block, and they read as fields just the same.
    let (d, mut ev) = skip_without!(read("no_hdr_sq_1.bam", "bgzf"));
    let p = stream(&mut ev, &d);
    let records = at(&mut ev, &d, &p, &["records"]);
    assert_eq!(ev.node(&d, &records).unwrap().child_count, 6);
    let cigar = at(&mut ev, &d, &[records, vec![1]].concat(), &["cigar"]);
    assert_eq!(ev.node(&d, &cigar).unwrap().child_count, 3);
}

/// `mpileup.1.bam` writes 119,396 bytes of header text, and its first block
/// unpacks to 65,280 bytes. Joined, the text is as long as `l_text` says, and
/// the 86 references after it are read from the second block.
#[test]
fn mpileup_header_reads_past_its_first_block() {
    let (d, mut ev) = skip_without!(read("mpileup.1.bam", "bgzf"));
    let p = stream(&mut ev, &d);
    let l_text = at(&mut ev, &d, &p, &["l_text"]);
    assert_eq!(ev.node(&d, &l_text).unwrap().value.as_int(), Some(119_396));
    let text = at(&mut ev, &d, &p, &["text"]);
    assert_eq!(ev.node(&d, &text).unwrap().size_bits, 119_396 * 8);
    let n_ref = at(&mut ev, &d, &p, &["n_ref"]);
    assert_eq!(ev.node(&d, &n_ref).unwrap().value.as_int(), Some(86));
    let refs = at(&mut ev, &d, &p, &["references"]);
    let name = at(&mut ev, &d, &[refs, vec![16]].concat(), &["name"]);
    assert_eq!(ev.node(&d, &name).unwrap().value, Value::Str("17".into()));
    let records = at(&mut ev, &d, &p, &["records"]);
    let first = at(&mut ev, &d, &[records, vec![0]].concat(), &["read_name"]);
    assert_eq!(ev.node(&d, &first).unwrap().value, Value::Str("ERR013140.3521432".into()));

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

/// The same record, read by the template: one field of the joined stream,
/// whole, from seventeen blocks, with the fields bamnostic reads.
#[test]
fn a_record_cut_across_seventeen_blocks_reads_whole() {
    let (d, mut ev) = skip_without!(read("bgzf_boundaries3.bam", "bgzf"));
    let p = stream(&mut ev, &d);
    let records = at(&mut ev, &d, &p, &["records"]);
    assert_eq!(ev.node(&d, &records).unwrap().child_count, 1);
    let r = [records, vec![0]].concat();
    let name = at(&mut ev, &d, &r, &["read_name"]);
    assert_eq!(ev.node(&d, &name).unwrap().value, Value::Str("SRR065390.14978392".into()));
    assert_eq!(int(&mut ev, &d, &r, &["pos"]), 1);
    assert_eq!(int(&mut ev, &d, &r, &["bin"]), 4681);
    let cigar = at(&mut ev, &d, &r, &["cigar"]);
    assert_eq!(ev.node(&d, &cigar).unwrap().child_count, 3);
    assert_eq!(int(&mut ev, &d, &[cigar, vec![1]].concat(), &["op_len"]), 1);
    let tags = at(&mut ev, &d, &r, &["tags"]);
    assert_eq!(ev.node(&d, &tags).unwrap().child_count, 7);
    let yt = at(&mut ev, &d, &[tags, vec![6]].concat(), &["value"]);
    assert_eq!(ev.node(&d, &yt).unwrap().value, Value::Str("UU".into()));
}

/// Every record of every BAM here, read by the template out of the joined
/// stream and by the side reader walking the blocks with no template at all,
/// agrees field by field. The side reader was checked against bamnostic, so
/// this ties the template to it without a Python in the loop.
///
/// Both halves of a record are compared: the fields the template places, and
/// the record's bytes as the template's field covers them, decoded the side
/// reader's way. The first says the fields are in the right places; the
/// second says the join put the right bytes under them.
#[test]
fn the_template_and_the_side_reader_agree_on_every_record() {
    use qubero_core::formats::bam_records::decode_record;
    for name in ["range.bam", "mpileup.1.bam", "no_hdr_sq_1.bam", "bgzf_boundaries1.bam", "bgzf_boundaries3.bam"] {
        let (d, mut ev) = skip_without!(read(name, "bgzf"));
        let oracle: Vec<_> = blocks(&mut ev, &d).into_iter().flat_map(|b| b.records).collect();
        let p = stream(&mut ev, &d);
        let records = at(&mut ev, &d, &p, &["records"]);
        assert_eq!(ev.node(&d, &records).unwrap().child_count as usize, oracle.len(), "{name}");
        for (i, want) in oracle.iter().enumerate() {
            let r = [records.clone(), vec![i]].concat();
            let fixed = [
                ("refID", want.ref_id as i128),
                ("pos", want.pos as i128),
                ("mapq", want.mapq as i128),
                ("bin", want.bin as i128),
                ("flag", want.flag as i128),
                ("l_seq", want.l_seq as i128),
                ("next_refID", want.next_ref_id as i128),
                ("next_pos", want.next_pos as i128),
                ("tlen", want.tlen as i128),
            ];
            for (field, value) in fixed {
                assert_eq!(int(&mut ev, &d, &r, &[field]), value, "{name} record {i} {field}");
            }
            let read_name = at(&mut ev, &d, &r, &["read_name"]);
            let read_name = ev.node(&d, &read_name).unwrap().value;
            assert_eq!(read_name, Value::Str(want.read_name.clone()), "{name} record {i}");
            let cigar = at(&mut ev, &d, &r, &["cigar"]);
            let mut written = String::new();
            for k in 0..ev.node(&d, &cigar).unwrap().child_count as usize {
                let op = [cigar.clone(), vec![k]].concat();
                let len = int(&mut ev, &d, &op, &["op_len"]);
                let code = int(&mut ev, &d, &op, &["op"]) as usize;
                written.push_str(&format!("{len}{}", b"MIDNSHP=X"[code] as char));
            }
            assert_eq!(if written.is_empty() { "*".to_string() } else { written }, want.cigar, "{name} record {i}");
            let qual = at(&mut ev, &d, &r, &["qual"]);
            assert_eq!(ev.field_bytes(&d, &qual, 1 << 20).unwrap().0, want.qual, "{name} record {i}");
            let tags = at(&mut ev, &d, &r, &["tags"]);
            let mut letters: Vec<String> = Vec::new();
            for k in 0..ev.node(&d, &tags).unwrap().child_count as usize {
                let tag = at(&mut ev, &d, &[tags.clone(), vec![k]].concat(), &["tag"]);
                match ev.node(&d, &tag).unwrap().value {
                    Value::Str(s) => letters.push(s),
                    other => panic!("{name} record {i}: a tag's letters are {other:?}"),
                }
            }
            assert_eq!(letters, want.tags.iter().map(|t| t.tag.clone()).collect::<Vec<_>>(), "{name} record {i}");
            // And the bytes: the whole record as the template's field covers
            // it, decoded the way the side reader decodes one.
            let (bytes, cut) = ev.field_bytes(&d, &r, 1 << 20).unwrap();
            assert!(!cut);
            let mut again = decode_record(&bytes[4..]);
            (again.block_offset, again.in_block, again.blocks) = (want.block_offset, want.in_block, want.blocks);
            assert_eq!(&again, want, "{name} record {i}");
        }
    }
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

/// Where every record the template reads out of the joined stream starts, as
/// a virtual offset worked out from the part its first byte is in, is where a
/// BAI's chunks and linear index say records start. The side reader already
/// agrees with the index; this is the template's own account of the same
/// places, from its part table rather than from a walk of the blocks.
#[test]
fn a_records_virtual_offset_matches_the_bai() {
    for name in ["range.bam", "mpileup.1.bam"] {
        let (bam, mut bam_ev) = skip_without!(read(name, "bgzf"));
        let p = stream(&mut bam_ev, &bam);
        let records = at(&mut bam_ev, &bam, &p, &["records"]);
        let mut starts = std::collections::HashSet::new();
        for i in 0..bam_ev.node(&bam, &records).unwrap().child_count as usize {
            let r = bam_ev.node(&bam, &[records.clone(), vec![i]].concat()).unwrap();
            let hit = bam_ev.part_of(&bam, r.space, r.offset_bits / 8).unwrap().expect("a record's first byte is in a part");
            starts.insert(hit.virtual_offset.expect("a BGZF block's part has a virtual offset"));
        }
        let (d, mut ev) = skip_without!(read(&format!("{name}.bai"), "bai"));
        let refs = at(&mut ev, &d, &[], &["references"]);
        let mut checked = 0;
        for i in 0..ev.node(&d, &refs).unwrap().child_count as usize {
            let ioffsets = at(&mut ev, &d, &[refs.clone(), vec![i]].concat(), &["ioffsets"]);
            for k in 0..ev.node(&d, &ioffsets).unwrap().child_count as usize {
                let v = int(&mut ev, &d, &[ioffsets.clone(), vec![k]].concat(), &["voffset"]) as u64;
                assert!(starts.contains(&v), "{name}: window {k} of reference {i} at {}:{}", v >> 16, v & 0xffff);
                checked += 1;
            }
        }
        assert!(checked > 0, "{name}");
    }
}

/// What the toolbar says a BGZF file holds, told from the sniff window alone,
/// is the case the template's switch on the joined stream takes.
#[test]
fn the_first_block_says_what_the_joined_stream_holds() {
    for (name, holds, switch) in [
        ("range.bam", "bam", "Bam"),
        ("mpileup.1.bam", "bam", "Bam"),
        ("bgzf_boundaries3.bam", "bam", "Bam"),
        ("index.bam.csi", "csi", "Csi"),
        ("no_hdr_sq_1.bam.csi", "csi", "Csi"),
    ] {
        let (d, mut ev) = skip_without!(read(name, "bgzf"));
        let bytes = std::fs::read(sample(name).unwrap()).unwrap();
        assert_eq!(formats::bgzf_contents(&bytes[..bytes.len().min(formats::SNIFF_WINDOW)]), Some(holds), "{name}");
        let p = stream(&mut ev, &d);
        assert_eq!(ev.node(&d, &p).unwrap().type_name, switch, "{name}");
    }
}

fn int(ev: &mut Evaluator, d: &Document<MemSource>, from: &[usize], names: &[&str]) -> i128 {
    let p = at(ev, d, from, names);
    ev.node(d, &p).unwrap().value.as_int().unwrap_or_else(|| panic!("{names:?} is not a number"))
}

/// `range.bam.bai` as bamnostic reads it: seven references, the first four
/// with bin 4681 and the summary bin, the last three empty, and no unplaced
/// reads. bamnostic stops at the first empty reference, so those three are
/// checked against the specification's layout rather than against it.
#[test]
fn a_bai_reads_as_bamnostic_reads_it() {
    let (d, mut ev) = skip_without!(read("range.bam.bai", "bai"));
    assert_eq!(int(&mut ev, &d, &[], &["n_ref"]), 7);
    let refs = at(&mut ev, &d, &[], &["references"]);
    let expected = [(32_964_608, 32_969_740, 18), (32_969_740, 32_979_376, 34), (32_979_376, 32_990_967, 41), (32_990_967, 872_218_624, 19)];
    for (i, (beg, end, mapped)) in expected.into_iter().enumerate() {
        let r = [refs.clone(), vec![i]].concat();
        assert_eq!(int(&mut ev, &d, &r, &["n_bin"]), 2, "reference {i}");
        let bins = at(&mut ev, &d, &r, &["bins"]);
        let bin = [bins.clone(), vec![0]].concat();
        assert_eq!(int(&mut ev, &d, &bin, &["bin"]), 4681);
        let chunk = [at(&mut ev, &d, &bin, &["chunks"]), vec![0]].concat();
        assert_eq!(int(&mut ev, &d, &chunk, &["chunk_beg", "voffset"]), beg);
        assert_eq!(int(&mut ev, &d, &chunk, &["chunk_end", "voffset"]), end);
        assert_eq!(int(&mut ev, &d, &chunk, &["chunk_beg", "block_offset"]), beg >> 16);
        assert_eq!(int(&mut ev, &d, &chunk, &["chunk_beg", "in_block"]), beg & 0xffff);
        let summary = [bins, vec![1]].concat();
        assert_eq!(int(&mut ev, &d, &summary, &["bin"]), 37450);
        assert_eq!(int(&mut ev, &d, &summary, &["chunks", "n_mapped"]), mapped);
        assert_eq!(int(&mut ev, &d, &summary, &["chunks", "n_unmapped"]), 0);
        let ioffsets = at(&mut ev, &d, &r, &["ioffsets"]);
        assert_eq!(int(&mut ev, &d, &[ioffsets, vec![0]].concat(), &["voffset"]), beg);
    }
    for i in 4..7 {
        assert_eq!(int(&mut ev, &d, &[refs.clone(), vec![i]].concat(), &["n_bin"]), 0);
    }
    assert_eq!(int(&mut ev, &d, &[], &["n_no_coor"]), 0);
    assert_eq!(ev.node(&d, &[]).unwrap().size_bits, d.len_bits());
}

/// Every place a BAI points into its BAM, the start of each chunk and each
/// window of the linear index, is where the side reader found a record start,
/// and every chunk end is a record start or the end of the stream. That ties the
/// index template and the side reader to each other, and both to htslib,
/// which wrote the index.
#[test]
fn every_offset_a_bai_gives_is_where_a_record_starts() {
    for name in ["range.bam", "mpileup.1.bam"] {
        let (bam, mut bam_ev) = skip_without!(read(name, "bgzf"));
        let found = blocks(&mut bam_ev, &bam);
        let starts: std::collections::HashSet<u64> =
            found.iter().flat_map(|b| b.records.iter().map(|r| r.virtual_offset())).collect();
        // The end of the last record, which htslib writes either as the
        // start of the empty last block or as the end of the file after it:
        // `range.bam.bai` does the first and `mpileup.1.bam.bai` the second.
        let ends_of_stream = [found.last().unwrap().block_offset << 16, bam.len_bytes() << 16];

        let (d, mut ev) = skip_without!(read(&format!("{name}.bai"), "bai"));
        let refs = at(&mut ev, &d, &[], &["references"]);
        let (mut begins, mut ends) = (0, 0);
        for i in 0..ev.node(&d, &refs).unwrap().child_count as usize {
            let r = [refs.clone(), vec![i]].concat();
            let bins = at(&mut ev, &d, &r, &["bins"]);
            for j in 0..ev.node(&d, &bins).unwrap().child_count as usize {
                let bin = [bins.clone(), vec![j]].concat();
                let chunks = at(&mut ev, &d, &bin, &["chunks"]);
                if ev.node(&d, &chunks).unwrap().type_name == "ReferenceSummary" {
                    continue;
                }
                for k in 0..ev.node(&d, &chunks).unwrap().child_count as usize {
                    let c = [chunks.clone(), vec![k]].concat();
                    let beg = int(&mut ev, &d, &c, &["chunk_beg", "voffset"]) as u64;
                    let end = int(&mut ev, &d, &c, &["chunk_end", "voffset"]) as u64;
                    assert!(starts.contains(&beg), "{name}: chunk starts at {}:{}", beg >> 16, beg & 0xffff);
                    assert!(starts.contains(&end) || ends_of_stream.contains(&end),"{name}: chunk ends at {}:{}", end >> 16, end & 0xffff);
                    begins += 1;
                    ends += 1;
                }
            }
            let ioffsets = at(&mut ev, &d, &r, &["ioffsets"]);
            for k in 0..ev.node(&d, &ioffsets).unwrap().child_count as usize {
                let v = int(&mut ev, &d, &[ioffsets.clone(), vec![k]].concat(), &["voffset"]) as u64;
                assert!(starts.contains(&v), "{name}: window {k} of reference {i} at {}:{}", v >> 16, v & 0xffff);
            }
        }
        assert!(begins > 0 && ends > 0, "{name}");
    }
}

/// Two CSI indexes, which are BGZF files: the one beside `no_hdr_sq_1.bam`
/// and htslib's `index.bam.csi`, whose BAM is not in the collection. Both use
/// depth 2, so the summary bin is 74 and not a BAI's 37450.
#[test]
fn a_csi_reads_inside_its_bgzf_block() {
    let (d, mut ev) = skip_without!(read("no_hdr_sq_1.bam.csi", "bgzf"));
    let p = stream(&mut ev, &d);
    assert_eq!(ev.node(&d, &p).unwrap().type_name, "Csi");
    assert_eq!((int(&mut ev, &d, &p, &["min_shift"]), int(&mut ev, &d, &p, &["depth"])), (14, 2));
    let refs = at(&mut ev, &d, &p, &["references"]);
    assert_eq!(ev.node(&d, &refs).unwrap().child_count, 5);
    let bins = at(&mut ev, &d, &[refs.clone(), vec![0]].concat(), &["bins"]);
    let bin = [bins.clone(), vec![0]].concat();
    assert_eq!(int(&mut ev, &d, &bin, &["bin"]), 1);
    // The first record of `no_hdr_sq_1.bam` is at 0:268, and the chunk runs
    // to the last block, at 1663.
    assert_eq!(int(&mut ev, &d, &bin, &["loffset", "voffset"]), 268);
    let chunk = [at(&mut ev, &d, &bin, &["chunks"]), vec![0]].concat();
    assert_eq!(int(&mut ev, &d, &chunk, &["chunk_beg", "in_block"]), 268);
    assert_eq!(int(&mut ev, &d, &chunk, &["chunk_end", "block_offset"]), 1663);
    let summary = [bins, vec![1]].concat();
    assert_eq!(int(&mut ev, &d, &summary, &["bin"]), 74);
    assert_eq!(int(&mut ev, &d, &summary, &["chunks", "n_mapped"]), 6);
    assert_eq!(int(&mut ev, &d, &p, &["n_no_coor"]), 0);

    let (d, mut ev) = skip_without!(read("index.bam.csi", "bgzf"));
    let p = stream(&mut ev, &d);
    let refs = at(&mut ev, &d, &p, &["references"]);
    let bins_per_ref: Vec<u64> = (0..7)
        .map(|i| {
            let bins = at(&mut ev, &d, &[refs.clone(), vec![i]].concat(), &["bins"]);
            ev.node(&d, &bins).unwrap().child_count
        })
        .collect();
    assert_eq!(bins_per_ref, [2, 2, 0, 0, 2, 0, 0]);
    assert_eq!(int(&mut ev, &d, &p, &["n_no_coor"]), 50);
    let node = ev.node(&d, &p).unwrap();
    assert_eq!(node.size_bits, 296 * 8);
}
