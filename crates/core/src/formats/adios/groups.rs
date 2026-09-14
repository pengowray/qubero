//! Process groups: what a writer's rank puts down in one step, in a BP3 file
//! or a BP4 data file.

use super::indices::*;
use super::shared::*;
use crate::template::{Endian, Expr as E, Ty as T, Until};

// ---------------------------------------------------------------------------
// Process groups

/// One process group, bracketed in BP4.
///
/// Each count of variables is how many blocks the group holds. The length
/// written beside it is not used: BP3 writes it four bytes short of what the
/// variables take. The attribute count is not used either: it is how many
/// attributes the IO has, and a group writes only the ones no group before it
/// did, so the attributes are read by their length.
pub(super) fn process_group(e: Endian, v: Version) -> T {
    let mut fields = vec![];
    if v == Bp4 {
        fields.push(("begin", T::magic(b"[PGI")));
    }
    fields.extend([
        ("length", T::u64(e)),
        ("array_order", T::enumeration("BpArrayOrder", T::u8(), ARRAY_ORDER)),
        ("io_name", bp_string(e)),
        ("coordination_var", T::u32(e)),
        ("step_name", bp_string(e)),
        ("step", T::u32(e)),
        ("transport_count", T::u8()),
        ("transports_length", T::u16(e)),
        ("transports", T::sized(clamp(E::field("transports_length")), T::array(transport(e), E::field("transport_count")))),
        ("variable_count", T::u32(e)),
        ("variables_length", T::u64(e)),
        ("variables", T::array(variable_record(e, v), E::field("variable_count").at_most(E::Remaining.div(E::lit(8))))),
        ("attribute_count", T::u32(e)),
        ("attributes_length", T::u64(e)),
        ("attributes", T::sized(clamp(E::field("attributes_length").sub(E::lit(8))), T::repeat(attribute_record(e, v), Until::End))),
    ]);
    let tail = if v == Bp4 { 4 } else { 0 };
    fields.push(("rest", T::when(E::lit(tail).less_than(E::Remaining), T::bytes(E::Remaining.sub(E::lit(tail))))));
    if v == Bp4 {
        fields.push(("end", T::magic(b"PGI]")));
    }
    // A BP4 group's length runs from the length to the end of `PGI]`; a BP3
    // group's does not count its own eight bytes.
    let size = match v {
        Bp4 => E::peek_at(E::lit(32), 64, e).add(E::lit(4)),
        Bp3 => E::peek(64, e).add(E::lit(8)),
    };
    T::sized(clamp(size), T::structure_named("ProcessGroup", "step_name", "variables", fields))
}

/// ADIOS 1's transport method IDs, of which ADIOS2 writes three.
pub(super) const TRANSPORT: &[(i128, &str)] = &[(0xFF, "null (discards output)"), (0xFE, "unknown"), (2, "POSIX"), (26, "fstream"), (27, "stdio"), (28, "ZeroMQ")];

pub(super) fn transport(e: Endian) -> T {
    T::structure(
        "Transport",
        vec![
            ("method", T::enumeration("BpTransport", T::u8(), TRANSPORT)),
            ("params_length", T::u16(e)),
            ("params", T::bytes(clamp(E::field("params_length")))),
        ],
    )
}

/// One block of a variable in a process group: its header, and then its
/// values.
///
/// The header has the type and each dimension's count, shape and start, each
/// behind a letter saying whether a variable gives it, which ADIOS 1 used and
/// ADIOS2 never does; ADIOS 1 wrote a four-byte variable ID there instead of
/// the number. The characteristic set after the dimensions holds the bounds.
/// A BP4 block then has a padding length and that much padding ending in
/// `VMD]`, which a writer given a span uses to align the values.
pub(super) fn variable_record(e: Endian, v: Version) -> T {
    let mut fields = vec![];
    if v == Bp4 {
        fields.push(("begin", T::magic(b"[VMD")));
    }
    fields.extend([("length", T::u64(e)), ("member_id", T::u32(e)), ("name", bp_string(e))]);
    match v {
        Bp3 => fields.push(("path", bp_string(e))),
        Bp4 => fields.extend([("array_order", T::enumeration("BpLayout", T::u8(), LAYOUT)), ("unused", T::u8())]),
    }
    let nth = |i: E| E::elem_field("dimensions", i, &["count"]);
    fields.extend([
        ("data_type", data_type()),
        ("is_dimension", T::enumeration("BpYesNo", T::u8(), YES_NO)),
        ("dims_count", T::u8()),
        ("dims_length", T::u16(e)),
        ("dimensions", T::sized(clamp(E::field("dims_length")), T::array(data_dimension(e), E::field("dims_count")))),
        ("characteristics", characteristic_set(e, Kind::Variable, Side::Data, v)),
    ]);
    if v == Bp4 {
        fields.extend([
            ("padding_length", T::u8()),
            ("padding", T::bytes(clamp(E::field("padding_length").sub(E::lit(4))))),
            ("end", T::magic(b"VMD]")),
        ]);
    }
    let some = || E::lit(0).less_than(E::field("dims_count"));
    fields.extend([
        ("counts", T::array(T::computed(nth(E::idx())), E::field("dims_count"))),
        ("element_count", T::computed(E::product_of("counts"))),
        ("row_length", T::computed(E::cond(some(), nth(E::field("dims_count").sub(E::lit(1))), E::lit(1)))),
        ("values", T::sized(E::Remaining, block_values(e, E::field("element_count"), E::field("row_length")))),
    ]);
    // Both versions count the length from the length field to the end of the
    // values.
    let size = match v {
        Bp4 => E::peek_at(E::lit(32), 64, e).add(E::lit(4)),
        Bp3 => E::peek(64, e),
    };
    T::sized(
        clamp(size),
        T::structure_named("VariableBlock", "name", "values", fields).machinery(&["counts", "element_count", "row_length"]),
    )
}

pub(super) const YES_NO: &[(i128, &str)] = &[(b'y' as i128, "yes"), (b'n' as i128, "no")];

/// One dimension in a data file's block header. See [`variable_record`].
pub(super) fn data_dimension(e: Endian) -> T {
    let part = |flag: &'static str| T::switch(E::field(flag).equal_to(E::lit(b'y' as i128)), vec![(1, T::u32(e))], T::u64(e));
    let flag = || T::enumeration("BpYesNo", T::u8(), YES_NO);
    T::structure(
        "DataDimension",
        vec![
            ("count_is_var", flag()),
            ("count", part("count_is_var")),
            ("shape_is_var", flag()),
            ("shape", part("shape_is_var")),
            ("start_is_var", flag()),
            ("start", part("start_is_var")),
        ],
    )
}

/// One attribute in a process group: its name, whether a variable owns it,
/// its type and its value, bracketed in BP4. A string's value is a length and
/// the text; a string array is a count and that many, each a length and text
/// with its nul; anything else is a byte count and that many bytes of values.
pub(super) fn attribute_record(e: Endian, v: Version) -> T {
    let mut fields = vec![];
    if v == Bp4 {
        fields.push(("begin", T::magic(b"[AMD")));
    }
    let sized_text = |name: &str| T::structure_named(name, "", "text", vec![("size", T::u32(e)), ("text", T::utf8_padded(clamp(E::field("size")), 0))]);
    let arrays = scalars(e)
        .into_iter()
        .map(|(k, t)| {
            let w = WIDTH.iter().find(|(x, _)| *x == k).map_or(1, |(_, w)| *w);
            (k, T::array(t, E::Remaining.div(E::lit(w))))
        })
        .collect();
    let numbers = T::structure(
        "AttributeValues",
        vec![
            ("size", T::u32(e)),
            ("values", T::sized(clamp(E::field("size")), T::switch(E::field("data_type"), arrays, T::bytes(E::Remaining)))),
        ],
    );
    fields.extend([
        ("length", T::u32(e)),
        ("member_id", T::u32(e)),
        ("name", bp_string(e)),
        ("path", bp_string(e)),
        ("is_variable", T::enumeration("BpYesNo", T::u8(), YES_NO)),
        ("variable_id", T::when(E::field("is_variable").equal_to(E::lit(b'y' as i128)), T::u32(e))),
        ("data_type", data_type()),
        (
            "value",
            T::switch(
                E::field("data_type"),
                vec![
                    (STRING, sized_text("AttributeString")),
                    (
                        STRING_ARRAY,
                        T::structure(
                            "AttributeStrings",
                            vec![("count", T::u32(e)), ("strings", T::array(sized_text("AttributeString"), E::field("count").at_most(E::Remaining.div(E::lit(4)))))],
                        ),
                    ),
                ],
                numbers,
            ),
        ),
    ]);
    let tail = if v == Bp4 { 4 } else { 0 };
    fields.push(("rest", T::when(E::lit(tail).less_than(E::Remaining), T::bytes(E::Remaining.sub(E::lit(tail))))));
    if v == Bp4 {
        fields.push(("end", T::magic(b"AMD]")));
    }
    let size = match v {
        Bp4 => E::peek_at(E::lit(32), 32, e).add(E::lit(4)),
        Bp3 => E::peek(32, e),
    };
    T::sized(clamp(size), T::structure_named("AttributeRecord", "name", "value", fields))
}
