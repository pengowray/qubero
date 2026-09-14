//! BAI and CSI, the indexes beside a BAM: the bins of each reference with the
//! runs of records in them, and where each run starts as a virtual offset.
//!
//! Apart from [`bam`](super::bam), which reads BGZF and the BAM stream,
//! because an index is a file of its own that never holds a record. The one
//! place the two meet is a CSI written through BGZF, which the joined stream
//! in `bam.rs` opens with [`csi_stream`].

use crate::template::{Endian::*, Expr as E, Template, Ty as T};

/// What a BAI opens with.
pub const BAI_MAGIC: &[u8] = b"BAI\x01";

/// What a CSI index opens with, once its BGZF block is unpacked.
pub const CSI_MAGIC: &[u8] = b"CSI\x01";

/// The bin a BAI keeps a reference's summary in rather than records: one past
/// the last real bin of its six levels.
const BAI_METADATA_BIN: i128 = 37450;

/// A BAM index. Not compressed, so every count in it is read over the file.
pub fn bai() -> Template {
    let bin = T::structure_named(
        "Bin",
        "bin",
        "",
        vec![
            ("bin", T::u32(Little)),
            ("n_chunk", T::u32(Little)),
            ("chunks", chunks_or_metadata(E::lit(BAI_METADATA_BIN))),
        ],
    );
    let reference = T::structure(
        "ReferenceIndex",
        vec![
            ("n_bin", T::u32(Little)),
            ("bins", T::array(bin, E::field("n_bin"))),
            ("n_intv", T::u32(Little)),
            // The linear index: for each 16 kbp window of the reference in
            // turn, where the first record overlapping it is.
            ("ioffsets", T::array(virtual_offset(), E::field("n_intv"))),
        ],
    );
    Template::new(
        "bai",
        T::structure(
            "Bai",
            vec![
                ("magic", T::magic(BAI_MAGIC)),
                ("n_ref", T::u32(Little)),
                ("references", T::array(reference.counted_as("reference"), E::field("n_ref"))),
                // How many reads have no coordinates at all. The specification
                // makes it optional, so a file may end before it.
                ("n_no_coor", T::if_room(T::u64(Little))),
            ],
        ),
    )
}

/// A CSI index as it is once unpacked: what a BGZF block holding one opens
/// into, and what a `.csi` reads as if somebody unpacked it.
pub fn csi() -> Template {
    Template::new("csi", csi_stream())
}

/// A CSI index.
///
/// The same index as a BAI with the binning made settings: `min_shift` is how
/// many bits the smallest bin spans and `depth` how many levels there are
/// below the one bin that covers everything, so a reference longer than a BAI
/// can index still has bins. Each bin also says where the first record
/// overlapping it is, which a BAI keeps in its linear index instead.
///
/// A CSI is written through BGZF and a large one fills several blocks, which
/// the joined stream reads through as it does a BAM's.
pub(super) fn csi_stream() -> T {
    let bin_limit = E::lit(1).shl(E::field("depth").add(E::lit(1)).mul(E::lit(3))).sub(E::lit(1)).div(E::lit(7));
    let bin = T::structure_named(
        "Bin",
        "bin",
        "",
        vec![
            ("bin", T::u32(Little)),
            ("loffset", virtual_offset()),
            ("n_chunk", T::Int { bits: 32, endian: Little }),
            ("chunks", chunks_or_metadata(bin_limit.add(E::lit(1)))),
        ],
    );
    let reference = T::structure(
        "ReferenceIndex",
        vec![("n_bin", T::Int { bits: 32, endian: Little }), ("bins", T::array(bin, E::field("n_bin")))],
    );
    T::structure(
        "Csi",
        vec![
            ("magic", T::magic(CSI_MAGIC)),
            ("min_shift", T::Int { bits: 32, endian: Little }),
            ("depth", T::Int { bits: 32, endian: Little }),
            // What the indexed format needs said about itself. Empty for a
            // BAM; for a tab-delimited file, which columns are the sequence
            // name and the positions.
            ("l_aux", T::Int { bits: 32, endian: Little }),
            ("aux", T::bytes(E::field("l_aux"))),
            ("n_ref", T::Int { bits: 32, endian: Little }),
            ("references", T::array(reference.counted_as("reference"), E::field("n_ref"))),
            ("n_no_coor", T::if_room(T::u64(Little))),
        ],
    )
}

/// A bin's chunks, or, in the one bin numbered `metadata`, what htslib keeps
/// about the reference there instead: where its records start and end, and
/// how many are mapped and unmapped. It is laid out as two chunks so that a
/// reader that does not know it passes over it, and it is read as the four
/// numbers only when it says it has two.
fn chunks_or_metadata(metadata: E) -> T {
    let chunk = T::structure("Chunk", vec![("chunk_beg", virtual_offset()), ("chunk_end", virtual_offset())]);
    let summary = T::structure(
        "ReferenceSummary",
        vec![
            ("ref_beg", virtual_offset()),
            ("ref_end", virtual_offset()),
            ("n_mapped", T::u64(Little)),
            ("n_unmapped", T::u64(Little)),
        ],
    );
    let is_metadata = E::field("bin").equal_to(metadata).both(E::field("n_chunk").equal_to(E::lit(2)));
    T::switch(is_metadata, vec![(1, summary)], T::array(chunk, E::field("n_chunk")))
}

/// A virtual offset: where a BGZF block starts in the file, in the top 48
/// bits, and a byte of what that block unpacks to, in the bottom 16. Shown as
/// the number and as its two halves, since the number itself is not an
/// offset of anything and cannot be added to or subtracted from.
fn virtual_offset() -> T {
    T::structure(
        "VirtualOffset",
        vec![
            ("voffset", T::u64(Little)),
            ("block_offset", T::computed(E::field("voffset").shr(E::lit(16)))),
            ("in_block", T::computed(E::field("voffset").and(E::lit(0xffff)))),
        ],
    )
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::Evaluator;
    use crate::formats::bam::bgzf;
    use crate::formats::bam::tests::{at_named, bgzf_block, joined, EOF_BLOCK};
    use crate::source::MemSource;

    fn voffset(block: u64, within: u16) -> [u8; 8] {
        (block << 16 | u64::from(within)).to_le_bytes()
    }

    /// One reference's index: a bin of two chunks, the summary bin, and two
    /// windows of the linear index.
    fn bai_reference(summary_bin: u32, with_loffset: bool) -> Vec<u8> {
        let mut v = Vec::new();
        let bin = |v: &mut Vec<u8>, number: u32, n_chunk: u32| {
            v.extend_from_slice(&number.to_le_bytes());
            if with_loffset {
                v.extend_from_slice(&voffset(100, 7));
            }
            v.extend_from_slice(&n_chunk.to_le_bytes());
        };
        v.extend_from_slice(&2u32.to_le_bytes());
        bin(&mut v, 4681, 2);
        for (b, e) in [((100, 7), (100, 900)), ((2000, 0), (2000, 30))] {
            v.extend_from_slice(&voffset(b.0, b.1));
            v.extend_from_slice(&voffset(e.0, e.1));
        }
        bin(&mut v, summary_bin, 2);
        v.extend_from_slice(&voffset(100, 7));
        v.extend_from_slice(&voffset(2000, 30));
        v.extend_from_slice(&12u64.to_le_bytes());
        v.extend_from_slice(&3u64.to_le_bytes());
        if !with_loffset {
            v.extend_from_slice(&2u32.to_le_bytes());
            v.extend_from_slice(&voffset(100, 7));
            v.extend_from_slice(&voffset(2000, 0));
        }
        v
    }

    #[test]
    fn a_bai_reads_its_bins_chunks_and_summary_with_each_offset_in_halves() {
        let mut file = BAI_MAGIC.to_vec();
        file.extend_from_slice(&2u32.to_le_bytes());
        file.extend_from_slice(&bai_reference(37450, false));
        // A reference with nothing on it.
        file.extend_from_slice(&0u32.to_le_bytes());
        file.extend_from_slice(&0u32.to_le_bytes());
        file.extend_from_slice(&9u64.to_le_bytes());
        assert_eq!(super::super::sniff(&file, file.len() as u64), Some("bai"));
        let d = Document::new(MemSource(file.clone()));
        let mut ev = Evaluator::new(bai());
        let bins = at_named(&mut ev, &d, &[], &["references"]);
        let bins = at_named(&mut ev, &d, &[bins, vec![0]].concat(), &["bins"]);
        let chunks = at_named(&mut ev, &d, &[bins.clone(), vec![0]].concat(), &["chunks"]);
        assert_eq!(ev.node(&d, &chunks).unwrap().child_count, 2);
        let end = at_named(&mut ev, &d, &[chunks, vec![0]].concat(), &["chunk_end"]);
        let half = |ev: &mut Evaluator, name: &str| {
            let p = at_named(ev, &d, &end, &[name]);
            ev.node(&d, &p).unwrap().value.as_int()
        };
        assert_eq!((half(&mut ev, "block_offset"), half(&mut ev, "in_block")), (Some(100), Some(900)));
        let summary = at_named(&mut ev, &d, &[bins, vec![1]].concat(), &["chunks"]);
        assert_eq!(ev.node(&d, &summary).unwrap().type_name, "ReferenceSummary");
        let mapped = at_named(&mut ev, &d, &summary, &["n_mapped"]);
        assert_eq!(ev.node(&d, &mapped).unwrap().value.as_int(), Some(12));
        let no_coor = at_named(&mut ev, &d, &[], &["n_no_coor"]);
        assert_eq!(ev.node(&d, &no_coor).unwrap().value.as_int(), Some(9));
        let root = ev.node(&d, &[]).unwrap();
        assert_eq!(root.size_bits, d.len_bits());

        // The count of unplaced reads is optional.
        let d = Document::new(MemSource(file[..file.len() - 8].to_vec()));
        let mut ev = Evaluator::new(bai());
        let no_coor = at_named(&mut ev, &d, &[], &["n_no_coor"]);
        assert_eq!(ev.node(&d, &no_coor).unwrap().size_bits, 0);
    }

    pub(in crate::formats) fn csi_stream_bytes(depth: i32, refs: usize) -> Vec<u8> {
        let mut v = CSI_MAGIC.to_vec();
        for n in [14i32, depth, 0, refs as i32] {
            v.extend_from_slice(&n.to_le_bytes());
        }
        // In a CSI the summary bin is one past the last bin the depth allows.
        let limit = ((1u32 << ((depth + 1) * 3)) - 1) / 7;
        for _ in 0..refs {
            v.extend_from_slice(&bai_reference(limit + 1, true));
        }
        v
    }

    #[test]
    fn a_csi_in_a_bgzf_block_reads_its_settings_and_finds_its_summary_bin_by_them() {
        let stream = csi_stream_bytes(2, 1);
        let mut file = bgzf_block(&stream);
        file.extend_from_slice(&EOF_BLOCK);
        let d = Document::new(MemSource(file));
        let mut ev = Evaluator::new(bgzf());
        let payload = joined(&mut ev, &d);
        assert_eq!(ev.node(&d, &payload).unwrap().type_name, "Csi");
        let refs = at_named(&mut ev, &d, &payload, &["references"]);
        let bins = at_named(&mut ev, &d, &[refs, vec![0]].concat(), &["bins"]);
        let loffset = at_named(&mut ev, &d, &[bins.clone(), vec![0]].concat(), &["loffset", "in_block"]);
        assert_eq!(ev.node(&d, &loffset).unwrap().value.as_int(), Some(7));
        // Bin 74 is the summary at depth 2, where 37450 would be a real bin.
        let summary = at_named(&mut ev, &d, &[bins, vec![1]].concat(), &["chunks"]);
        assert_eq!(ev.node(&d, &summary).unwrap().type_name, "ReferenceSummary");
    }

    /// A CSI too large for its first block: the block cuts a bin 26 bytes
    /// in, and every bin of every reference reads all the same, the cut one
    /// included.
    #[test]
    fn a_csi_across_two_blocks_reads_every_bin() {
        let stream = csi_stream_bytes(5, 3);
        // The header, one whole reference, and 30 bytes of the next: its bin
        // count and 26 bytes of its first bin.
        let cut = 20 + 100 + 30;
        let mut file = bgzf_block(&stream[..cut]);
        file.extend_from_slice(&bgzf_block(&stream[cut..]));
        file.extend_from_slice(&EOF_BLOCK);
        let d = Document::new(MemSource(file));
        let mut ev = Evaluator::new(bgzf());
        let payload = joined(&mut ev, &d);
        let refs = at_named(&mut ev, &d, &payload, &["references"]);
        assert_eq!(ev.node(&d, &refs).unwrap().child_count, 3);
        let bins = at_named(&mut ev, &d, &[refs, vec![1]].concat(), &["bins"]);
        assert_eq!(ev.node(&d, &bins).unwrap().child_count, 2);
        let cut_bin = [bins, vec![0]].concat();
        assert_eq!(ev.node(&d, &cut_bin).unwrap().type_name, "Bin");
        let end = at_named(&mut ev, &d, &cut_bin, &["chunks"]);
        let end = at_named(&mut ev, &d, &[end, vec![1]].concat(), &["chunk_end", "in_block"]);
        assert_eq!(ev.node(&d, &end).unwrap().value.as_int(), Some(30));
        let node = ev.node(&d, &payload).unwrap();
        assert_eq!(node.size_bits, stream.len() as u64 * 8);
    }
}
