//! BP3: process groups, three indices and a footer, in one file.

use super::groups::*;
use super::indices::*;
use super::shared::*;
use crate::template::{Endian, Endian::*, Expr as E, Template, Ty as T, Until};

// ---------------------------------------------------------------------------
// BP3

/// A BP3 file: process groups, the three indices, and the 56 bytes at the end
/// that say where each index starts. Read from the back, as Parquet is: the
/// byte order is the fourth byte from the end, and the footer is placed first
/// so that everything before it can be measured by it.
///
/// Each index is measured by where the next one starts rather than by the
/// length written in it, which is what ADIOS2's reader does, since ADIOS 1
/// wrote those lengths too small.
///
/// A writer of more than one rank puts the data in subfiles and writes this
/// file with the indices alone, starting at nought; its offsets are into the
/// subfile its subfile index names. A subfile is itself a BP3 file with its own
/// footer.
pub fn adios_bp3() -> Template {
    let root = by_byte_order(E::peek_at(E::lit(-32), 8, Big), |e| {
        let at = |name: &str| E::within(&["footer", name]);
        // The whole file, since the fields after the process groups are
        // declared past them and what is left from there is not the file.
        let footer_at = || E::WindowSize.sub(E::lit(FOOTER)).at_least(E::lit(0));
        let between = |from: E, to: E| to.sub(from.clone()).at_least(E::lit(0)).at_most(footer_at().sub(from).at_least(E::lit(0)));
        T::structure(
            "Bp3",
            vec![
                ("footer", T::at(footer_at(), minifooter(e))),
                ("process_groups", T::sized(clamp(at("pg_index_offset")), T::repeat(process_group(e, Bp3), Until::End))),
                (
                    "pg_index",
                    T::at(
                        at("pg_index_offset"),
                        T::sized(between(at("pg_index_offset"), at("variables_index_offset")), pg_index(e, E::Remaining)),
                    ),
                ),
                (
                    "variables_index",
                    T::at(
                        at("variables_index_offset"),
                        T::sized(
                            between(at("variables_index_offset"), at("attributes_index_offset")),
                            element_index(e, Bp3, Kind::Variable, E::Remaining),
                        ),
                    ),
                ),
                (
                    "attributes_index",
                    T::at(
                        at("attributes_index_offset"),
                        T::sized(between(at("attributes_index_offset"), footer_at()), element_index(e, Bp3, Kind::Attribute, E::Remaining)),
                    ),
                ),
            ],
        )
    });
    Template::new("adiosbp3", root)
}

/// How long a BP3 footer is: a 28-byte version and the 28 ADIOS 1 wrote.
pub(super) const FOOTER: i128 = 56;

/// The footer: the version string of the release that wrote the file, the
/// release again as characters, where each index starts, the byte order, two
/// flags, and the BP version.
///
/// The flags are the byte ADIOS 1 kept its version flags in, one bit for data
/// in subfiles and one for a time index characteristic in every set. ADIOS2
/// writes 3 in a file of indices alone and nothing in a file holding its data,
/// and its reader takes 3 as subfiles and 0 or 2 as none, which is the low bit.
pub(super) fn minifooter(e: Endian) -> T {
    let digit = |name: &str| T::computed(E::field(name).sub(E::lit(b'0' as i128)));
    T::structure(
        "Bp3Footer",
        vec![
            ("version_tag", T::utf8_padded(E::lit(24), 0)),
            ("major_char", T::u8()),
            ("minor_char", T::u8()),
            ("patch_char", T::u8()),
            ("adios_major", digit("major_char")),
            ("adios_minor", digit("minor_char")),
            ("adios_patch", digit("patch_char")),
            ("unused", T::u8()),
            ("pg_index_offset", T::u64(e)),
            ("variables_index_offset", T::u64(e)),
            ("attributes_index_offset", T::u64(e)),
            ("byte_order", T::enumeration("BpByteOrder", T::u8(), ENDIANNESS)),
            ("reserved", T::u8()),
            ("flags", T::flags("BpFooterFlags", T::u8(), &[(0, "data in subfiles"), (1, "time index")])),
            ("bp_version", T::u8()),
        ],
    )
}
