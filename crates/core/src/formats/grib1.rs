//! GRIB edition 1: a message's five sections, from the product definition to
//! the end marker.
//!
//! Apart from [`grib`](super::grib) because it shares next to nothing with
//! edition 2: the lengths are three bytes, there are no section numbers, and
//! which sections are there is said by flags in the first one. What it does
//! share, a few number types and the table of centres, it takes from there.
//! `grib.rs` reads the edition byte and hands the message here when it is 1.

use super::grib::{sm16, u16be, CENTRE};
use crate::template::{Endian::*, Expr as E, Ty as T};

/// A three-byte length, which is how edition 1 counts everything.
fn u24be() -> T {
    T::UInt { bits: 24, endian: Big }
}

/// An edition 1 message. Its sections have three-byte lengths and no numbers
/// in them: which section is which is settled by the order they come in, and
/// two of the five are there only if a flag in the first one says so. So they
/// are declared as five fields rather than as a run, and the two optional ones
/// ask that flag.
pub(super) fn edition1() -> T {
    let body = E::field("total_length").sub(E::lit(8)).at_most(E::Remaining).at_least(E::lit(0));
    T::structure(
        "Grib1",
        vec![
            ("total_length", u24be()),
            ("edition", T::u8()),
            (
                "sections",
                T::sized(
                    body,
                    T::structure(
                        "Grib1Sections",
                        vec![
                            ("pds", grib1_pds()),
                            ("gds", present_when(7, grib1_gds())),
                            ("bms", present_when(6, grib1_bms())),
                            ("bds", grib1_bds()),
                            ("end", T::if_room(T::magic(b"7777"))),
                        ],
                    ),
                ),
            ),
        ],
    )
}

/// A section that is there only when bit `n` of the product definition's flags
/// says it is. Nothing else in the message says so: an edition 1 file has no
/// section numbers to check against, and a reader that guessed by looking
/// would find a three-byte length wherever it looked.
fn present_when(n: u32, section: T) -> T {
    T::switch(E::within(&["pds", "flags"]).bit(n), vec![(1, section)], T::bytes(E::lit(0)))
}

/// What the units of P1 and P2 are, from edition 1's table 4. Not edition 2's
/// table: the two agree as far as year and then stop.
const TIME_UNIT_1: &[(i128, &str)] = &[
    (0, "minute"),
    (1, "hour"),
    (2, "day"),
    (3, "month"),
    (4, "year"),
    (5, "decade"),
    (6, "normal, 30 years"),
    (7, "century"),
    (10, "3 hours"),
    (11, "6 hours"),
    (12, "12 hours"),
    (13, "15 minutes"),
    (14, "30 minutes"),
    (254, "second"),
];

/// How an edition 1 grid is laid out, from its table 6.
const GRID_TYPE_1: &[(i128, &str)] = &[
    (0, "latitude/longitude"),
    (1, "Mercator"),
    (3, "Lambert conformal"),
    (4, "Gaussian latitude/longitude"),
    (5, "polar stereographic"),
    (10, "rotated latitude/longitude"),
    (50, "spherical harmonic"),
];

/// Section 1 of an edition 1 message, the product definition: who wrote it,
/// what the field is, and when. The year is a year of a century and the
/// century is written separately, so 2026 is year 26 of century 21.
fn grib1_pds() -> T {
    T::structure(
        "ProductDefinition1",
        vec![
            ("length", u24be()),
            ("table_version", T::u8()),
            ("centre", T::enumeration("Centre", T::u8(), CENTRE)),
            ("generating_process", T::u8()),
            ("grid_id", T::u8()),
            ("flags", T::flags("Grib1Flags", T::u8(), &[(7, "grid definition"), (6, "bitmap")])),
            ("parameter", T::u8()),
            ("level_type", T::u8()),
            ("level", u16be()),
            ("year_of_century", T::u8()),
            ("month", T::u8()),
            ("day", T::u8()),
            ("hour", T::u8()),
            ("minute", T::u8()),
            ("time_unit", T::enumeration("TimeUnit1", T::u8(), TIME_UNIT_1)),
            ("p1", T::u8()),
            ("p2", T::u8()),
            ("time_range", T::u8()),
            ("number_in_average", u16be()),
            ("missing_from_average", T::u8()),
            ("century", T::u8()),
            ("subcentre", T::u8()),
            ("decimal_scale_factor", sm16()),
            ("reserved", T::bytes(grib1_rest(28))),
        ],
    )
    .machinery(&["reserved"])
    .payload(&["parameter", "level"])
}

/// What is left of an edition 1 section after `read` bytes of it, clamped so
/// that a length shorter than the fields it must hold does not run backwards
/// and one longer than the message does not run off the end.
fn grib1_rest(read: i128) -> E {
    E::field("length").at_least(E::lit(read)).sub(E::lit(read)).at_most(E::Remaining)
}

/// Section 2, the grid. `nv` counts vertical coordinate parameters written
/// after the grid, and `pv_location` says which octet they start at; both are
/// part of the tail here rather than fields of their own.
fn grib1_gds() -> T {
    T::structure(
        "GridDefinition1",
        vec![
            ("length", u24be()),
            ("nv", T::u8()),
            ("pv_location", T::u8()),
            ("data_representation", T::enumeration("Grib1GridType", T::u8(), GRID_TYPE_1)),
            (
                "grid",
                T::sized(
                    grib1_rest(6),
                    T::switch(
                        E::field("data_representation"),
                        vec![(0, grib1_latlon("LatLonGrid1")), (4, grib1_latlon("GaussianGrid1"))],
                        T::bytes(E::Remaining),
                    ),
                ),
            ),
        ],
    )
}

/// An edition 1 latitude/longitude or Gaussian grid. Every angle is three
/// bytes of sign and magnitude, in thousandths of a degree rather than
/// millionths: this is the format from before anyone needed the resolution.
///
/// A count of 0xFFFF is a quasi-regular grid, where the rows are not all the
/// same length and the lengths are written after the grid instead. Read as a
/// number it is 65535 columns, which is a grid nobody has.
fn grib1_latlon(name: &str) -> T {
    let count = || T::unset_int(u16be(), 0xFFFF);
    T::structure(
        name,
        vec![
            ("ni", count()),
            ("nj", count()),
            ("first_latitude", T::sign_magnitude(24, Big)),
            ("first_longitude", T::sign_magnitude(24, Big)),
            ("resolution_flags", T::u8()),
            ("last_latitude", T::sign_magnitude(24, Big)),
            ("last_longitude", T::sign_magnitude(24, Big)),
            ("i_increment", u16be()),
            ("j_increment", u16be()),
            ("scanning_mode", T::u8()),
            ("rest", T::bytes(E::Remaining)),
        ],
    )
    .payload(&["ni", "nj"])
}

/// Section 3, the bitmap: which grid points have a value. A table reference
/// that is not zero means the bitmap is one of the centre's own, kept
/// somewhere else, and there are no bits here.
fn grib1_bms() -> T {
    T::structure(
        "Bitmap1",
        vec![
            ("length", u24be()),
            ("unused_bits", T::u8()),
            ("table_reference", u16be()),
            ("bitmap", T::bytes(grib1_rest(6))),
        ],
    )
}

/// Section 4, the packed data. The reference value is an IBM System/360
/// float: a sign, a seven-bit exponent of a power of sixteen with 64 added to
/// it, and a 24-bit fraction. Those read as the three raw fields they are
/// rather than as a number, because no float type in the IR is that one.
///
/// How many values there are is not written down: it is the bits of the
/// section, less the unused ones the flags count off the end, divided by the
/// width. A width of zero is a field that is the same everywhere and has no
/// data at all.
fn grib1_bds() -> T {
    let flag = |n: u32| E::field("flags").bit(n);
    // The low four bits of the flags, which count the bits of padding at the
    // end. There is no remainder operator to take a nibble with.
    let unused = flag(0).add(flag(1).mul(E::lit(2))).add(flag(2).mul(E::lit(4))).add(flag(3).mul(E::lit(8)));
    let bpv = || E::field("bits_per_value");
    let count = E::lit(0)
        .less_than(bpv())
        .mul(E::Remaining.mul(E::lit(8)).sub(unused).at_least(E::lit(0)).div(bpv().at_least(E::lit(1))));
    T::structure(
        "BinaryData1",
        vec![
            ("length", u24be()),
            (
                "flags",
                T::flags(
                    "Grib1DataFlags",
                    T::u8(),
                    &[(7, "spherical harmonic"), (6, "second order packing"), (5, "integer values"), (4, "extra flags")],
                ),
            ),
            ("binary_scale_factor", sm16()),
            ("reference_sign", T::UInt { bits: 1, endian: Big }),
            ("reference_exponent", T::UInt { bits: 7, endian: Big }),
            ("reference_fraction", T::UInt { bits: 24, endian: Big }),
            ("bits_per_value", T::u8()),
            (
                "data",
                T::sized(
                    grib1_rest(11),
                    T::structure(
                        "Grib1PackedData",
                        vec![
                            ("count", T::computed(count)),
                            ("values", T::array(T::uint_expr(bpv(), Big), E::field("count"))),
                        ],
                    )
                    .machinery(&["count"])
                    .payload(&["values"]),
                ),
            ),
        ],
    )
    .payload(&["bits_per_value"])
}

#[cfg(test)]
mod tests {
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::formats::grib;
    use crate::source::MemSource;

    /// An edition 1 message: all five sections, a grid and a bitmap, and six
    /// values packed at eight bits.
    fn message1_bytes(flags: u8) -> Vec<u8> {
        let sm24 = |v: i32| {
            let m = (v.unsigned_abs() | if v < 0 { 0x80_0000 } else { 0 }).to_be_bytes();
            [m[1], m[2], m[3]]
        };
        let mut pds = vec![0, 0, 28, 2, 98, 145, 255, flags, 11, 100];
        pds.extend_from_slice(&850u16.to_be_bytes()); // 850 hPa
        pds.extend_from_slice(&[26, 9, 2, 6, 0, 1, 6, 0, 0]); // 2026-09-02 06:00, 6 hours on
        pds.extend_from_slice(&0u16.to_be_bytes());
        pds.extend_from_slice(&[0, 21, 0]);
        pds.extend_from_slice(&0x8002u16.to_be_bytes()); // a decimal scale of -2
        let mut gds = vec![0, 0, 32, 0, 255, 0];
        gds.extend_from_slice(&3u16.to_be_bytes()); // ni
        gds.extend_from_slice(&2u16.to_be_bytes()); // nj
        gds.extend_from_slice(&sm24(-45_000)); // 45 S, in thousandths
        gds.extend_from_slice(&sm24(0));
        gds.push(0x80);
        gds.extend_from_slice(&sm24(45_000));
        gds.extend_from_slice(&sm24(90_000));
        gds.extend_from_slice(&30_000u16.to_be_bytes());
        gds.extend_from_slice(&45_000u16.to_be_bytes());
        gds.extend_from_slice(&[0, 0, 0, 0, 0]);
        let bms = vec![0, 0, 7, 2, 0, 0, 0xFC];
        let mut bds = vec![0, 0, 17, 0];
        bds.extend_from_slice(&0x8001u16.to_be_bytes()); // a binary scale of -1
        bds.extend_from_slice(&[0x41, 0x10, 0x00, 0x00]); // the reference value, IBM format
        bds.push(8); // eight bits a value
        bds.extend_from_slice(&[1, 2, 3, 4, 5, 6]);
        let mut b = b"GRIB".to_vec();
        let total = 8 + pds.len() + if flags & 0x80 != 0 { gds.len() } else { 0 }
            + if flags & 0x40 != 0 { bms.len() } else { 0 }
            + bds.len() + 4;
        b.extend_from_slice(&(total as u32).to_be_bytes()[1..]);
        b.push(1); // edition 1
        b.extend_from_slice(&pds);
        if flags & 0x80 != 0 {
            b.extend_from_slice(&gds);
        }
        if flags & 0x40 != 0 {
            b.extend_from_slice(&bms);
        }
        b.extend_from_slice(&bds);
        b.extend_from_slice(b"7777");
        b
    }

    #[test]
    fn an_edition_1_message_reads_as_its_five_sections() {
        let d = Document::new(MemSource(message1_bytes(0xC0)));
        let mut ev = Evaluator::new(grib());
        let grib1 = ev.node(&d, &[0, 1]).unwrap();
        assert_eq!(grib1.type_name, "Grib1");
        assert_eq!(ev.node(&d, &[0, 1, 1]).unwrap().value, Value::UInt(1));
        let sections = ev.node(&d, &[0, 1, 2]).unwrap();
        assert_eq!(sections.child_count, 5);
        // The product definition: which centre, which parameter, which level.
        let centre = ev.node(&d, &[0, 1, 2, 0, 2]).unwrap();
        assert_eq!(centre.value, Value::Enum { raw: 98, name: Some("Reading, ECMWF".into()), hex: false });
        assert_eq!(ev.node(&d, &[0, 1, 2, 0, 6]).unwrap().value, Value::UInt(11));
        assert_eq!(ev.node(&d, &[0, 1, 2, 0, 8]).unwrap().value, Value::UInt(850));
        assert_eq!(ev.node(&d, &[0, 1, 2, 0, 22]).unwrap().value, Value::Int(-2));
        // The end marker is the last four bytes and nothing is left over.
        let end = ev.node(&d, &[0, 1, 2, 4]).unwrap();
        assert_eq!(end.offset_bits + end.size_bits, sections.offset_bits + sections.size_bits);
    }

    #[test]
    fn an_edition_1_grid_writes_its_angles_in_three_bytes() {
        let d = Document::new(MemSource(message1_bytes(0xC0)));
        let mut ev = Evaluator::new(grib());
        let kind = ev.node(&d, &[0, 1, 2, 1, 3]).unwrap();
        assert_eq!(kind.value, Value::Enum { raw: 0, name: Some("latitude/longitude".into()), hex: false });
        let grid = ev.node(&d, &[0, 1, 2, 1, 4]).unwrap();
        assert_eq!(grid.type_name, "LatLonGrid1");
        assert_eq!(ev.node(&d, &[0, 1, 2, 1, 4, 0]).unwrap().value, Value::UInt(3));
        // 45 south, in thousandths of a degree and sign and magnitude.
        let first = ev.node(&d, &[0, 1, 2, 1, 4, 2]).unwrap();
        assert_eq!((first.value, first.size_bits), (Value::Int(-45_000), 24));
        assert_eq!(ev.node(&d, &[0, 1, 2, 1, 4, 5]).unwrap().value, Value::Int(45_000));
    }

    #[test]
    fn an_edition_1_data_section_reads_as_its_values() {
        let d = Document::new(MemSource(message1_bytes(0xC0)));
        let mut ev = Evaluator::new(grib());
        assert_eq!(ev.node(&d, &[0, 1, 2, 3, 2]).unwrap().value, Value::Int(-1)); // binary scale
        // The reference value as the three fields an IBM float is.
        assert_eq!(ev.node(&d, &[0, 1, 2, 3, 3]).unwrap().value, Value::UInt(0));
        assert_eq!(ev.node(&d, &[0, 1, 2, 3, 4]).unwrap().value, Value::UInt(0x41));
        assert_eq!(ev.node(&d, &[0, 1, 2, 3, 5]).unwrap().value, Value::UInt(0x10_0000));
        // Six bytes of section, eight bits a value, no unused bits.
        let values = ev.node(&d, &[0, 1, 2, 3, 7, 1]).unwrap();
        assert_eq!((values.child_count, values.size_bits), (6, 48));
        assert_eq!(ev.node(&d, &[0, 1, 2, 3, 7, 1, 5]).unwrap().value, Value::UInt(6));
    }

    #[test]
    fn an_edition_1_message_without_a_grid_or_a_bitmap_skips_them() {
        // The flags in the product definition are the only thing that says
        // which sections are there.
        let d = Document::new(MemSource(message1_bytes(0)));
        let mut ev = Evaluator::new(grib());
        assert_eq!(ev.node(&d, &[0, 1, 2, 1]).unwrap().size_bits, 0);
        assert_eq!(ev.node(&d, &[0, 1, 2, 2]).unwrap().size_bits, 0);
        // And the data section is where the grid would have been.
        assert_eq!(ev.node(&d, &[0, 1, 2, 3, 6]).unwrap().value, Value::UInt(8));
    }
}
