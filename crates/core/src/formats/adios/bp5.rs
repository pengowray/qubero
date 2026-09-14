//! BP5: an index of records by kind and length, and metadata of FFS records.

use super::ffs::*;
use super::shared::*;
use crate::template::{Endian, Endian::*, Expr as E, Template, Ty as T, Until};

// ---------------------------------------------------------------------------
// BP5

/// The kinds of record in a BP5 index, by the letter each is written as.
pub(super) const RECORD_KIND: &[(i128, &str)] = &[(b's' as i128, "step"), (b'w' as i128, "writer map")];

/// A BP5 `md.idx`: the header, and records to the end, each a kind, a length
/// and a body.
///
/// A step record says where its metadata is in `md.0` and how long, how many
/// times each writer flushed, and for each writer where each flush and the
/// rest of its data went in its data file. How many writers there are is in
/// the writer map record before it, and is worked out here from the length
/// instead, so a step record reads without that one.
pub fn adios_bp5_index() -> Template {
    let step = |e: Endian| {
        let flush = T::structure("Flush", vec![("offset", T::u64(e)), ("size", T::u64(e))]);
        let per_writer = E::field("flush_count").mul(E::lit(16)).add(E::lit(8));
        T::structure(
            "Bp5StepRecord",
            vec![
                ("metadata_offset", T::u64(e)),
                ("metadata_size", T::u64(e)),
                ("flush_count", T::u64(e)),
                ("writer_count", T::computed(E::Remaining.div(per_writer))),
                (
                    "writers",
                    T::array(
                        T::structure(
                            "WriterData",
                            vec![("flushes", T::array(flush, E::field("flush_count"))), ("data_offset", T::u64(e))],
                        ),
                        E::field("writer_count"),
                    ),
                ),
                ("rest", T::when(E::lit(0).less_than(E::Remaining), T::bytes(E::Remaining))),
            ],
        )
    };
    let writers = |e: Endian| {
        T::structure(
            "Bp5WriterMap",
            vec![
                ("writer_count", T::u64(e)),
                ("aggregator_count", T::u64(e)),
                ("subfile_count", T::u64(e)),
                ("subfile_of_writer", T::array(T::u64(e), E::field("writer_count").at_most(E::Remaining.div(E::lit(8))))),
                ("rest", T::when(E::lit(0).less_than(E::Remaining), T::bytes(E::Remaining))),
            ],
        )
    };
    let record = |e: Endian| {
        T::structure_named(
            "Bp5IndexRecord",
            "kind",
            "body",
            vec![
                ("kind", T::enumeration("Bp5RecordKind", T::u8(), RECORD_KIND)),
                ("length", T::u64(e)),
                (
                    "body",
                    T::sized(
                        clamp(E::field("length")),
                        T::switch(E::field("kind"), vec![(b's' as i128, step(e)), (b'w' as i128, writers(e))], T::bytes(E::Remaining)),
                    ),
                ),
            ],
        )
    };
    let root = T::structure(
        "Bp5Index",
        vec![
            ("header", header(true)),
            ("records", by_byte_order(E::within(&["header", "byte_order"]), |e| T::repeat(record(e), Until::End))),
        ],
    );
    Template::new("adiosbp5idx", root)
}

/// The byte order of a BP5 file with no header, which is written in the
/// writer's own. Little-endian unless the first eight bytes only make sense
/// the other way round as a length that fits in the file.
pub(super) fn plausible_order(make: impl Fn(Endian) -> T) -> T {
    let fits = |e: Endian| E::lit(8).less_or_equal(E::Remaining).both(E::peek(64, e).less_or_equal(E::Remaining));
    T::switch(fits(Little).negate().both(fits(Big)), vec![(1, make(Big))], make(Little))
}

/// A BP5 `md.0`: steps one after another, each a total and then its blocks.
///
/// The blocks are, for each writer, the size of its metadata block, then for
/// each writer the size of its attribute block, then the metadata blocks and
/// then the attribute blocks that are not empty. How many writers is in
/// `md.idx`. One writer's step is recognised by its two sizes and their
/// sixteen bytes coming to the total, and read into its two blocks; a step of
/// several writers is left as bytes.
pub fn adios_bp5_metadata() -> Template {
    let root = plausible_order(|e| {
        let one_writer = E::lit(16)
            .less_or_equal(E::Remaining)
            .both(E::peek(64, e).add(E::peek_at(E::lit(64), 64, e)).add(E::lit(16)).equal_to(E::Remaining));
        let blocks = T::structure(
            "Bp5WriterBlocks",
            vec![
                ("metadata_size", T::u64(e)),
                ("attributes_size", T::u64(e)),
                ("metadata", T::sized(clamp(E::field("metadata_size")), ffs_record(e))),
                (
                    "attributes",
                    T::when(E::lit(0).less_than(E::field("attributes_size")), T::sized(clamp(E::field("attributes_size")), ffs_record(e))),
                ),
            ],
        );
        let step = T::structure(
            "Bp5StepMetadata",
            vec![
                ("total_size", T::u64(e)),
                ("blocks", T::sized(clamp(E::field("total_size")), T::switch(one_writer, vec![(1, blocks)], T::bytes(E::Remaining)))),
            ],
        );
        T::repeat(step, Until::End)
    });
    Template::new("adiosbp5md", root)
}
