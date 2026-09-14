//! BP4: a directory of an index, metadata and data files, each opening with
//! the same header.

use super::groups::*;
use super::indices::*;
use super::shared::*;
use crate::template::{Endian, Expr as E, Template, Time, Ty as T, Until};

// ---------------------------------------------------------------------------
// BP4

/// A BP4 `md.idx`: the header, and a 64-byte record for each step saying which
/// rank wrote it, where in `md.0` its three indices start and its end is, and
/// when it was written.
pub fn adios_bp4_index() -> Template {
    let record = |e: Endian| {
        T::structure(
            "Bp4StepIndex",
            vec![
                ("step", T::u64(e)),
                ("rank", T::u64(e)),
                ("pg_index_offset", T::u64(e)),
                ("variables_index_offset", T::u64(e)),
                ("attributes_index_offset", T::u64(e)),
                ("step_end_offset", T::u64(e)),
                ("timestamp", T::u64(e)),
                ("reserved", T::u64(e)),
            ],
        )
        .field_time("timestamp", Time::unix())
    };
    let root = T::structure(
        "Bp4Index",
        vec![
            ("header", header(false)),
            (
                "steps",
                by_byte_order(E::within(&["header", "byte_order"]), |e| T::array(record(e), E::Remaining.div(E::lit(64)))),
            ),
            ("rest", T::when(E::lit(0).less_than(E::Remaining), T::bytes(E::Remaining))),
        ],
    );
    Template::new("adiosbp4idx", root)
}

/// A BP4 `md.0`: the header, and each step's three indices one after another.
/// Every offset in them is into a data file of the same directory, the one a
/// set's file index names, and is not followed.
pub fn adios_bp4_metadata() -> Template {
    let step = |e: Endian| {
        T::structure_named(
            "Bp4StepMetadata",
            "step",
            "",
            vec![
                ("pg_index", pg_index(e, E::field("length"))),
                (
                    "step",
                    T::computed(E::cond(
                        E::lit(0).less_than(E::within(&["pg_index", "count"])),
                        E::elem_within(&["pg_index", "entries"], E::lit(0), &["step"]),
                        E::lit(0),
                    )),
                ),
                ("variables_index", element_index(e, Bp4, Kind::Variable, E::field("length"))),
                ("attributes_index", element_index(e, Bp4, Kind::Attribute, E::field("length"))),
            ],
        )
    };
    let root = T::structure(
        "Bp4Metadata",
        vec![("header", header(false)), ("steps", by_byte_order(E::within(&["header", "byte_order"]), |e| T::repeat(step(e), Until::End)))],
    );
    Template::new("adiosbp4md", root)
}

/// A BP4 `data.N`: the header, and process groups to the end.
pub fn adios_bp4_data() -> Template {
    let root = T::structure(
        "Bp4Data",
        vec![
            ("header", header(false)),
            ("process_groups", by_byte_order(E::within(&["header", "byte_order"]), |e| T::repeat(process_group(e, Bp4), Until::End))),
        ],
    );
    Template::new("adiosbp4data", root)
}
