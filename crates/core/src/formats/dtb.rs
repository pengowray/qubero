//! Flattened device trees: the file a bootloader hands an ARM kernel to tell
//! it what hardware is on the board.
//!
//! Three blocks sit behind the header, each at an offset the header gives: the
//! memory the kernel must not touch, the tree itself, and the names of every
//! property in it. The tree is a stream of tokens rather than a nesting of
//! records, so it is read the way it is written, and the depth is left for a
//! logical view to work out.

use crate::template::{Endian::Big, Expr as E, Template, Ty as T, Until};

/// The tokens the tree is written as. A node opens with its name, closes with
/// nothing, and carries its properties in between; `nop` is what an editor
/// leaves behind when it takes something out without moving the rest.
const TOKENS: &[(i128, &str)] = &[
    (0x1, "begin node"),
    (0x2, "end node"),
    (0x3, "property"),
    (0x4, "nop"),
    (0x9, "end"),
];

pub fn dtb() -> Template {
    Template::new("dtb", tree())
}

fn tree() -> T {
    T::structure(
        "DeviceTree",
        vec![
            ("magic", T::magic(b"\xd0\x0d\xfe\xed")),
            ("totalsize", T::u32(Big)),
            ("off_dt_struct", T::u32(Big)),
            ("off_dt_strings", T::u32(Big)),
            ("off_mem_rsvmap", T::u32(Big)),
            ("version", T::u32(Big)),
            ("last_comp_version", T::u32(Big)),
            ("boot_cpuid_phys", T::u32(Big)),
            ("size_dt_strings", T::u32(Big)),
            // Version 17 added this field, and a version 16 tree ends the
            // header without it. Reading it anyway would take the first four
            // bytes of whatever block the writer put next.
            (
                "size_dt_struct",
                T::switch(E::lit(16).less_than(E::field("version")), vec![(1, T::u32(Big))], T::computed(E::lit(0))),
            ),
            (
                "reservations",
                T::at(E::field("off_mem_rsvmap"), T::repeat(reservation(), Until::FieldBytes { field: "size".into(), bytes: vec![0; 8] })),
            ),
            (
                "structure",
                T::at(
                    E::field("off_dt_struct"),
                    // A version 16 header does not say how long the block is,
                    // and what follows it is the strings, so the room between
                    // the two is the answer.
                    T::sized(
                        E::field("size_dt_struct").or(E::field("off_dt_strings").sub(E::field("off_dt_struct"))),
                        T::repeat(token(), Until::FieldBytes { field: "token".into(), bytes: vec![0, 0, 0, 0x9] }),
                    ),
                ),
            ),
            (
                "strings",
                T::at(E::field("off_dt_strings"), T::sized(E::field("size_dt_strings"), T::repeat(T::cstr().counted_as("name"), Until::End))),
            ),
        ],
    )
}

/// A stretch of memory the kernel is told to leave alone, which is how a
/// board hands over a framebuffer or firmware that is already running. The
/// list ends with a pair of zeros rather than a count.
fn reservation() -> T {
    T::structure("MemoryReservation", vec![("address", T::u64(Big)), ("size", T::u64(Big))]).counted_as("reservation")
}

fn token() -> T {
    T::structure_named(
        "FdtToken",
        "token",
        "body",
        vec![
            ("token", T::enumeration("FdtTokenKind", T::u32(Big), TOKENS)),
            (
                "body",
                T::switch(
                    E::field("token"),
                    vec![(0x1, begin_node()), (0x3, property())],
                    // `end node`, `nop` and `end` are the token and nothing
                    // else: the token said all there was to say.
                    T::bytes(E::lit(0)),
                ),
            ),
        ],
    )
    .counted_as("token")
}

fn begin_node() -> T {
    T::structure(
        "BeginNode",
        vec![
            ("name", T::cstr()),
            ("padding", T::bytes(pad4(E::size_of("name")))),
        ],
    )
}

/// A property: how long its value is, where its name is written, and then the
/// value. The name is not here at all. Every property in the tree that says
/// `compatible` says it by pointing at one `compatible` in the strings block,
/// which is what keeps a tree of a thousand nodes small.
fn property() -> T {
    T::structure(
        "Property",
        vec![
            ("len", T::u32(Big)),
            ("nameoff", T::u32(Big)),
            ("name", T::at(E::field("off_dt_strings").add(E::field("nameoff")), T::cstr())),
            ("value", T::bytes(E::field("len"))),
            ("padding", T::bytes(pad4(E::field("len")))),
        ],
    )
}

/// How many bytes of padding follow a run of `n` bytes, to bring the next
/// token back onto a four-byte boundary.
///
/// There is no remainder operator, so the remainder is written out: `n` less
/// the whole fours in it. Four less that is the padding, except when the run
/// already ended on a boundary, where it comes to four rather than none, so
/// the same subtraction is done again to take it back off.
fn pad4(n: E) -> E {
    let over = n.clone().sub(n.div(E::lit(4)).mul(E::lit(4)));
    let pad = E::lit(4).sub(over);
    pad.clone().sub(pad.div(E::lit(4)).mul(E::lit(4)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        document::Document,
        eval::{Evaluator, Value},
        source::MemSource,
    };

    /// A tree of one node with one property, written the way `dtc` writes one.
    fn built() -> Vec<u8> {
        let strings = b"compatible\0".to_vec();
        let mut structure = Vec::new();
        structure.extend_from_slice(&1u32.to_be_bytes()); // begin node
        structure.extend_from_slice(b"\0\0\0\0"); // the root's name is empty
        structure.extend_from_slice(&3u32.to_be_bytes()); // property
        structure.extend_from_slice(&5u32.to_be_bytes()); // len
        structure.extend_from_slice(&0u32.to_be_bytes()); // nameoff
        structure.extend_from_slice(b"acme\0");
        structure.extend_from_slice(b"\0\0\0"); // to the next boundary
        structure.extend_from_slice(&2u32.to_be_bytes()); // end node
        structure.extend_from_slice(&9u32.to_be_bytes()); // end
        let header = 40usize;
        let rsv_at = header;
        let struct_at = rsv_at + 16;
        let strings_at = struct_at + structure.len();
        let mut v = b"\xd0\x0d\xfe\xed".to_vec();
        v.extend_from_slice(&((strings_at + strings.len()) as u32).to_be_bytes());
        v.extend_from_slice(&(struct_at as u32).to_be_bytes());
        v.extend_from_slice(&(strings_at as u32).to_be_bytes());
        v.extend_from_slice(&(rsv_at as u32).to_be_bytes());
        v.extend_from_slice(&17u32.to_be_bytes());
        v.extend_from_slice(&16u32.to_be_bytes());
        v.extend_from_slice(&0u32.to_be_bytes());
        v.extend_from_slice(&(strings.len() as u32).to_be_bytes());
        v.extend_from_slice(&(structure.len() as u32).to_be_bytes());
        v.extend_from_slice(&[0; 16]); // the reservation list's terminator
        v.extend_from_slice(&structure);
        v.extend_from_slice(&strings);
        v
    }

    #[test]
    fn the_tokens_read_as_a_stream() {
        let d = Document::new(MemSource(built()));
        let mut e = Evaluator::new(dtb());
        // A field placed elsewhere is its contents, so the block is one
        // child in, and then: begin node, property, end node, end.
        assert_eq!(e.node(&d, &[11, 0]).unwrap().child_count, 4);
        let node_name = e.node(&d, &[11, 0, 0, 1, 0]).unwrap();
        assert_eq!(node_name.value, Value::Str(String::new()));
        // The name padded the token back onto its boundary.
        assert_eq!(e.node(&d, &[11, 0, 0, 1, 1]).unwrap().size_bits, 3 * 8);
    }

    /// A property says where its name is, not what it is. The name is read
    /// from the strings block, which is somewhere else in the file entirely.
    #[test]
    fn a_property_reads_its_name_from_the_strings_block() {
        let d = Document::new(MemSource(built()));
        let mut e = Evaluator::new(dtb());
        assert_eq!(
            e.node(&d, &[11, 0, 1, 1, 2, 0]).unwrap().value,
            Value::Str("compatible".into())
        );
        assert_eq!(e.node(&d, &[11, 0, 1, 1, 3]).unwrap().size_bits, 5 * 8);
        // Five bytes of value, so three of padding.
        assert_eq!(e.node(&d, &[11, 0, 1, 1, 4]).unwrap().size_bits, 3 * 8);
    }

    /// One reservation entry, which is the terminator and nothing else.
    #[test]
    fn the_reservation_list_ends_at_its_pair_of_zeros() {
        let d = Document::new(MemSource(built()));
        let mut e = Evaluator::new(dtb());
        assert_eq!(e.node(&d, &[10, 0]).unwrap().child_count, 1);
    }
}
