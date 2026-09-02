//! GRIB: the file a weather forecast or a reanalysis is distributed as.
//!
//! One file holds any number of messages, written back to back, and each
//! message is one field: a grid, a parameter, a moment in time, and the packed
//! numbers. A message opens with the letters `GRIB` and says how long it is, so
//! the messages are read as a run of sized windows, and the sections inside one
//! are read until that window ends.
//!
//! Edition 2 is what everything current writes. A message is section 0, the
//! indicator, and then sections 1 to 8, each one a length and a number saying
//! which section it is. Sections 3 to 7 may repeat inside one message, which is
//! how a file packs several fields on the same grid, and nothing here has to
//! say so: the sections are a run until the window ends and the numbers say
//! what each of them is.
//!
//! What reads as its fields: the indicator, the identification with its
//! reference time, a latitude/longitude grid (grid template 3.0), an analysis
//! or forecast at a level (product template 4.0), simple packing (data
//! representation template 5.0), and the bitmap indicator. A section written to
//! a template this does not know keeps its length and its number and its
//! contents stay bytes, which is the honest answer: the templates are a WMO
//! table of several hundred and each one is a different set of fields.
//!
//! Section 7, the data, stays bytes as well. Its numbers are packed at whatever
//! width section 5 declared, and that width is a field of the file rather than
//! a constant, which the IR cannot yet express: an integer type's width is set
//! when the template is built. See the note on [`data`].
//!
//! Edition 1 is still published and is a different layout: three-byte lengths,
//! no section numbers, and the sections identified by their order and by flags.
//! It reads here as far as that goes, which is the message boundaries and each
//! section's extent, and no further. Reading its sections as edition 2's would
//! be worse than not reading them.
//!
//! Sign and magnitude is the trap in this format. A negative latitude, a
//! negative scale factor and a negative forecast time are written as a
//! magnitude with the top bit set, not as two's complement, so every one of
//! those fields is read here as unsigned: the bit pattern is what the file
//! holds, and reading 0x80000005 as -5 would need a numeric type the IR does
//! not have. The field names say which ones they are.

use crate::template::{Endian::*, Expr as E, Template, Ty as T, Until};

/// What kind of data a message holds, from WMO code table 0.0.
const DISCIPLINE: &[(i128, &str)] = &[
    (0, "meteorological"),
    (1, "hydrological"),
    (2, "land surface"),
    (3, "space"),
    (4, "space weather"),
    (10, "oceanographic"),
    (255, "missing"),
];

/// The sections of an edition 2 message, in the order they appear.
const SECTION: &[(i128, &str)] = &[
    (1, "identification"),
    (2, "local use"),
    (3, "grid definition"),
    (4, "product definition"),
    (5, "data representation"),
    (6, "bitmap"),
    (7, "data"),
];

/// The centres that publish most of what anyone reads, from WMO table C-11.
const CENTRE: &[(i128, &str)] = &[
    (7, "US National Weather Service, NCEP"),
    (8, "US National Weather Service, NWSTG"),
    (34, "Tokyo, JMA"),
    (54, "Montreal"),
    (58, "US Navy, FNMOC"),
    (74, "Exeter, UK Met Office"),
    (78, "Offenbach, DWD"),
    (85, "Toulouse, Meteo France"),
    (98, "Reading, ECMWF"),
    (160, "US NOAA/NESDIS"),
];

/// What the reference time of a message means, from code table 1.2.
const TIME_SIGNIFICANCE: &[(i128, &str)] =
    &[(0, "analysis"), (1, "start of forecast"), (2, "verifying time"), (3, "observation time")];

const PRODUCTION_STATUS: &[(i128, &str)] = &[
    (0, "operational"),
    (1, "operational test"),
    (2, "research"),
    (3, "re-analysis"),
    (4, "TIGGE"),
    (5, "TIGGE test"),
];

const DATA_TYPE: &[(i128, &str)] =
    &[(0, "analysis"), (1, "forecast"), (2, "analysis and forecast"), (3, "control forecast"), (7, "radar"), (8, "satellite")];

/// The grid templates, from code table 3.1. Only 0 reads as its fields here.
const GRID_TEMPLATE: &[(i128, &str)] = &[
    (0, "latitude/longitude"),
    (1, "rotated latitude/longitude"),
    (10, "Mercator"),
    (20, "polar stereographic"),
    (30, "Lambert conformal"),
    (40, "Gaussian latitude/longitude"),
    (50, "spherical harmonic"),
    (90, "space view"),
    (101, "unstructured"),
];

/// The product templates, from code table 4.0.
const PRODUCT_TEMPLATE: &[(i128, &str)] = &[
    (0, "analysis or forecast at a level"),
    (1, "individual ensemble forecast"),
    (2, "derived ensemble forecast"),
    (8, "statistically processed over an interval"),
    (11, "individual ensemble, over an interval"),
    (20, "radar"),
    (30, "satellite"),
];

/// The data representation templates, from code table 5.0.
const PACKING: &[(i128, &str)] = &[
    (0, "simple packing"),
    (1, "matrix simple packing"),
    (2, "complex packing"),
    (3, "complex packing with spatial differencing"),
    (4, "IEEE floating point"),
    (40, "JPEG 2000"),
    (41, "PNG"),
    (42, "CCSDS"),
];

/// What the units of a forecast time are, from code table 4.4.
const TIME_UNIT: &[(i128, &str)] = &[
    (0, "minute"),
    (1, "hour"),
    (2, "day"),
    (3, "month"),
    (4, "year"),
    (10, "3 hours"),
    (11, "6 hours"),
    (12, "12 hours"),
    (13, "second"),
];

fn u16be() -> T {
    T::u16(Big)
}

fn u32be() -> T {
    T::u32(Big)
}

/// A three-byte length, which is how edition 1 counts everything.
fn u24be() -> T {
    T::UInt { bits: 24, endian: Big }
}

pub fn grib() -> Template {
    Template::new("grib", T::repeat(T::Named("Message".into()), Until::End))
        .with_type("Message", message())
        .with_type("Section", section())
        .with_type("Section1", legacy_section())
}

/// One message. The edition is the eighth byte in both editions, and it decides
/// everything after the letters, including where the length is written and how
/// wide it is, so it is looked at before anything is read.
fn message() -> T {
    T::structure_named(
        "Message",
        "",
        "body",
        vec![
            ("magic", T::magic(b"GRIB")),
            (
                "body",
                T::switch(
                    E::peek_at(E::lit(24), 8, Big),
                    vec![(2, edition2()), (1, edition1())],
                    // An edition nobody has published yet. How long the message
                    // is, is written in a place only its own edition knows, so
                    // the rest of the file is what is left to say about it.
                    T::bytes(E::Remaining),
                ),
            ),
        ],
    )
}

/// Section 0 of an edition 2 message, and the sections it contains. The length
/// counts the letters as well, so sixteen of it is already read by here.
fn edition2() -> T {
    let body = E::field("total_length").sub(E::lit(16)).at_most(E::Remaining).at_least(E::lit(0));
    T::structure(
        "Grib2",
        vec![
            ("reserved", T::bytes(E::lit(2))),
            ("discipline", T::enumeration("Discipline", T::u8(), DISCIPLINE)),
            ("edition", T::u8()),
            ("total_length", T::u64(Big)),
            ("sections", T::sized(body, T::repeat(T::Named("Section".into()), Until::End))),
        ],
    )
    .machinery(&["reserved"])
}

/// One section of an edition 2 message. The end section is four letters with no
/// length in front of them, so which of the two shapes this is has to be
/// settled by looking rather than by reading.
fn section() -> T {
    T::switch(
        look_ahead(),
        vec![
            (0x3737_3737, T::structure("EndSection", vec![("marker", T::magic(b"7777"))])),
            (TRUNCATED, T::structure("Truncated", vec![("bytes", T::bytes(E::Remaining))])),
        ],
        numbered_section(),
    )
}

/// The number a look-ahead answers when there is not enough of the message left
/// to look at. No section can be it: a length of four thousand million and a
/// section number of 255 is not a section, and neither is the end marker.
const TRUNCATED: i128 = -1;

/// The first four bytes of the next section, or [`TRUNCATED`] when fewer than
/// four are left. A message cut off partway through is the file this is for:
/// looking past the end of the window is an error, and the bytes that did
/// arrive are still worth placing.
fn look_ahead() -> E {
    E::Remaining.less_than(E::lit(4)).mul(E::lit(TRUNCATED)).or(E::peek(32, Big))
}

fn numbered_section() -> T {
    // A length of less than five is a file that has gone wrong; the section
    // still takes up the five bytes of its own header, so the run of sections
    // moves on rather than reading the same bytes for ever.
    let body = E::field("length").at_least(E::lit(5)).sub(E::lit(5)).at_most(E::Remaining);
    T::structure_named(
        "Section",
        "number",
        "body",
        vec![
            ("length", u32be()),
            ("number", T::enumeration("SectionNumber", T::u8(), SECTION)),
            ("body", T::sized(body, section_body())),
        ],
    )
}

fn section_body() -> T {
    T::switch(
        E::field("number"),
        vec![
            (1, identification()),
            // Whatever the centre that wrote the file wanted to keep. Only that
            // centre knows what is in it.
            (2, T::bytes(E::Remaining)),
            (3, grid_definition()),
            (4, product_definition()),
            (5, data_representation()),
            (6, bitmap()),
            (7, data()),
        ],
        T::bytes(E::Remaining),
    )
}

/// Section 1: who wrote the message, which tables they wrote it against, and
/// what moment it is about.
fn identification() -> T {
    T::structure(
        "Identification",
        vec![
            ("centre", T::enumeration("Centre", u16be(), CENTRE)),
            ("subcentre", u16be()),
            ("master_tables_version", T::u8()),
            ("local_tables_version", T::u8()),
            ("time_significance", T::enumeration("TimeSignificance", T::u8(), TIME_SIGNIFICANCE)),
            ("year", u16be()),
            ("month", T::u8()),
            ("day", T::u8()),
            ("hour", T::u8()),
            ("minute", T::u8()),
            ("second", T::u8()),
            ("production_status", T::enumeration("ProductionStatus", T::u8(), PRODUCTION_STATUS)),
            ("data_type", T::enumeration("DataType", T::u8(), DATA_TYPE)),
            ("reserved", T::bytes(E::Remaining)),
        ],
    )
    .machinery(&["reserved"])
}

/// Section 3: the grid the numbers are on. The template number says which of
/// several hundred layouts the rest of the section is; template 0, a plain
/// latitude/longitude grid, is much the commonest and is the one read here.
fn grid_definition() -> T {
    T::structure(
        "GridDefinition",
        vec![
            ("source", T::u8()),
            ("number_of_data_points", u32be()),
            ("optional_list_octets", T::u8()),
            ("optional_list_interpretation", T::u8()),
            ("template_number", T::enumeration("GridTemplate", u16be(), GRID_TEMPLATE)),
            ("template", T::switch(E::field("template_number"), vec![(0, latlon_grid())], T::bytes(E::Remaining))),
        ],
    )
}

/// Grid template 3.0. The corners and the increments are in millionths of a
/// degree, and a southern latitude or a westward increment is written as a
/// magnitude with the top bit set rather than as a negative number, so these
/// read as the unsigned patterns they are.
fn latlon_grid() -> T {
    T::structure(
        "LatLonGrid",
        vec![
            ("shape_of_earth", T::u8()),
            ("radius_scale_factor", T::u8()),
            ("scaled_radius", u32be()),
            ("major_axis_scale_factor", T::u8()),
            ("scaled_major_axis", u32be()),
            ("minor_axis_scale_factor", T::u8()),
            ("scaled_minor_axis", u32be()),
            ("ni", u32be()),
            ("nj", u32be()),
            ("basic_angle", u32be()),
            ("basic_angle_subdivisions", u32be()),
            ("first_latitude_sign_magnitude", u32be()),
            ("first_longitude_sign_magnitude", u32be()),
            ("resolution_flags", T::u8()),
            ("last_latitude_sign_magnitude", u32be()),
            ("last_longitude_sign_magnitude", u32be()),
            ("i_increment", u32be()),
            ("j_increment", u32be()),
            ("scanning_mode", T::u8()),
            ("rest", T::bytes(E::Remaining)),
        ],
    )
    .payload(&["ni", "nj"])
}

/// Section 4: what the numbers are of. `nv` counts the vertical coordinate
/// values written after the template, and is almost always zero.
fn product_definition() -> T {
    T::structure(
        "ProductDefinition",
        vec![
            ("nv", u16be()),
            ("template_number", T::enumeration("ProductTemplate", u16be(), PRODUCT_TEMPLATE)),
            ("template", T::switch(E::field("template_number"), vec![(0, forecast_at_level())], T::bytes(E::Remaining))),
        ],
    )
}

/// Product template 4.0. The parameter is a category and a number within it,
/// read against the discipline in section 0: category 0 number 0 of discipline
/// 0 is temperature. Naming all of those would be the whole of WMO table 4.2.
fn forecast_at_level() -> T {
    T::structure(
        "ForecastAtLevel",
        vec![
            ("parameter_category", T::u8()),
            ("parameter_number", T::u8()),
            ("generating_process_type", T::u8()),
            ("background_process", T::u8()),
            ("generating_process", T::u8()),
            ("hours_after_cutoff", u16be()),
            ("minutes_after_cutoff", T::u8()),
            ("time_unit", T::enumeration("TimeUnit", T::u8(), TIME_UNIT)),
            ("forecast_time", u32be()),
            ("first_surface_type", T::u8()),
            ("first_surface_scale_factor", T::u8()),
            ("first_surface_value", u32be()),
            ("second_surface_type", T::u8()),
            ("second_surface_scale_factor", T::u8()),
            ("second_surface_value", u32be()),
            ("rest", T::bytes(E::Remaining)),
        ],
    )
    .payload(&["parameter_category", "parameter_number", "forecast_time"])
}

/// Section 5: how many numbers there are and how they were packed.
fn data_representation() -> T {
    T::structure(
        "DataRepresentation",
        vec![
            ("number_of_values", u32be()),
            ("template_number", T::enumeration("PackingTemplate", u16be(), PACKING)),
            ("template", T::switch(E::field("template_number"), vec![(0, simple_packing())], T::bytes(E::Remaining))),
        ],
    )
}

/// Data template 5.0. Each packed number is `bits_per_value` bits wide, and the
/// value it stands for is the reference value plus that number scaled by the
/// two exponents. Both exponents are sign and magnitude, so they read as the
/// patterns the file holds.
fn simple_packing() -> T {
    T::structure(
        "SimplePacking",
        vec![
            ("reference_value", T::F32(Big)),
            ("binary_scale_sign_magnitude", u16be()),
            ("decimal_scale_sign_magnitude", u16be()),
            ("bits_per_value", T::u8()),
            ("original_field_type", T::u8()),
            ("rest", T::bytes(E::Remaining)),
        ],
    )
    .payload(&["reference_value", "bits_per_value"])
}

/// Section 6: whether some of the grid points have no value, and which.
fn bitmap() -> T {
    T::structure(
        "Bitmap",
        vec![
            ("indicator", T::enumeration("BitmapIndicator", T::u8(), &[(0, "in this section"), (254, "as an earlier one"), (255, "none")])),
            ("bitmap", T::bytes(E::Remaining)),
        ],
    )
}

/// Section 7: the packed numbers.
///
/// These stay bytes, and the reason is an IR gap rather than a choice. One
/// value is `bits_per_value` bits wide, from section 5, and that is a field of
/// the file; `Ty::UInt` takes its width when the template is built, so there is
/// no way yet to say "as many values as section 5 counted, each as wide as
/// section 5 says". Reaching section 5 from here is not the missing part:
/// `Expr::Sibling` already walks back through the sections for exactly this.
fn data() -> T {
    T::structure("PackedData", vec![("values", T::bytes(E::Remaining))])
}

/// An edition 1 message. Its sections have three-byte lengths and no numbers in
/// them: which section is which is settled by the order they come in and by
/// flags in the first two. The extents are what reads here.
fn edition1() -> T {
    let body = E::field("total_length").sub(E::lit(8)).at_most(E::Remaining).at_least(E::lit(0));
    T::structure(
        "Grib1",
        vec![
            ("total_length", u24be()),
            ("edition", T::u8()),
            ("sections", T::sized(body, T::repeat(T::Named("Section1".into()), Until::End))),
        ],
    )
}

fn legacy_section() -> T {
    let body = E::field("length").at_least(E::lit(3)).sub(E::lit(3)).at_most(E::Remaining);
    T::switch(
        look_ahead(),
        vec![
            (0x3737_3737, T::structure("EndSection", vec![("marker", T::magic(b"7777"))])),
            (TRUNCATED, T::structure("Truncated", vec![("bytes", T::bytes(E::Remaining))])),
        ],
        T::structure_named(
            "Section1",
            "",
            "body",
            vec![("length", u24be()), ("body", T::sized(body, T::bytes(E::Remaining)))],
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    /// A section: its number, and the bytes after the five-byte header.
    fn sec(number: u8, body: &[u8]) -> Vec<u8> {
        let mut v = ((body.len() + 5) as u32).to_be_bytes().to_vec();
        v.push(number);
        v.extend_from_slice(body);
        v
    }

    /// Section 1, with a reference time of 2026-09-02 06:00:00 from ECMWF.
    fn identification_bytes() -> Vec<u8> {
        let mut b = 98u16.to_be_bytes().to_vec(); // centre
        b.extend_from_slice(&0u16.to_be_bytes()); // subcentre
        b.extend_from_slice(&[32, 0, 1]); // tables, and a start of forecast
        b.extend_from_slice(&2026u16.to_be_bytes());
        b.extend_from_slice(&[9, 2, 6, 0, 0, 0, 1]);
        b
    }

    /// Section 3, a two by three latitude/longitude grid whose first corner is
    /// in the southern hemisphere, written sign and magnitude.
    fn grid_bytes() -> Vec<u8> {
        let mut b = vec![0]; // source: from a template
        b.extend_from_slice(&6u32.to_be_bytes()); // six points
        b.extend_from_slice(&[0, 0]); // no optional list
        b.extend_from_slice(&0u16.to_be_bytes()); // template 0
        b.extend_from_slice(&[6, 0]);
        b.extend_from_slice(&0u32.to_be_bytes()); // scaled radius
        b.extend_from_slice(&[0]);
        b.extend_from_slice(&0u32.to_be_bytes());
        b.extend_from_slice(&[0]);
        b.extend_from_slice(&0u32.to_be_bytes());
        b.extend_from_slice(&3u32.to_be_bytes()); // ni
        b.extend_from_slice(&2u32.to_be_bytes()); // nj
        b.extend_from_slice(&0u32.to_be_bytes());
        b.extend_from_slice(&0u32.to_be_bytes());
        b.extend_from_slice(&(0x8000_0000u32 | 45_000_000).to_be_bytes()); // 45 S
        b.extend_from_slice(&0u32.to_be_bytes());
        b.push(0x30);
        b.extend_from_slice(&45_000_000u32.to_be_bytes()); // 45 N
        b.extend_from_slice(&90_000_000u32.to_be_bytes());
        b.extend_from_slice(&45_000_000u32.to_be_bytes());
        b.extend_from_slice(&45_000_000u32.to_be_bytes());
        b.push(0);
        b
    }

    /// Section 4: a temperature forecast six hours out.
    fn product_bytes() -> Vec<u8> {
        let mut b = 0u16.to_be_bytes().to_vec(); // no vertical coordinates
        b.extend_from_slice(&0u16.to_be_bytes()); // template 0
        b.extend_from_slice(&[0, 0, 2, 0, 96]);
        b.extend_from_slice(&0u16.to_be_bytes());
        b.extend_from_slice(&[0, 1]); // in hours
        b.extend_from_slice(&6u32.to_be_bytes()); // six of them
        b.extend_from_slice(&[1, 0]);
        b.extend_from_slice(&0u32.to_be_bytes());
        b.extend_from_slice(&[255, 0]);
        b.extend_from_slice(&0u32.to_be_bytes());
        b
    }

    /// Section 5: six values, simply packed at eight bits each.
    fn packing_bytes() -> Vec<u8> {
        let mut b = 6u32.to_be_bytes().to_vec();
        b.extend_from_slice(&0u16.to_be_bytes()); // template 0
        b.extend_from_slice(&270.0f32.to_be_bytes());
        b.extend_from_slice(&0u16.to_be_bytes()); // binary scale
        b.extend_from_slice(&0u16.to_be_bytes()); // decimal scale
        b.extend_from_slice(&[8, 0]);
        b
    }

    /// A whole edition 2 message: every section, and six packed bytes.
    fn message_bytes() -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(&sec(1, &identification_bytes()));
        body.extend_from_slice(&sec(3, &grid_bytes()));
        body.extend_from_slice(&sec(4, &product_bytes()));
        body.extend_from_slice(&sec(5, &packing_bytes()));
        body.extend_from_slice(&sec(6, &[255]));
        body.extend_from_slice(&sec(7, &[1, 2, 3, 4, 5, 6]));
        body.extend_from_slice(b"7777");
        let mut b = b"GRIB".to_vec();
        b.extend_from_slice(&[0, 0, 0, 2]); // reserved, meteorological, edition 2
        b.extend_from_slice(&((body.len() + 16) as u64).to_be_bytes());
        b.extend_from_slice(&body);
        b
    }

    #[test]
    fn a_file_is_a_run_of_messages() {
        // The same message twice over, which is what a forecast of two fields
        // looks like.
        let one = message_bytes();
        let mut two = one.clone();
        two.extend_from_slice(&one);
        let d = Document::new(MemSource(two));
        let mut ev = Evaluator::new(grib());
        assert_eq!(ev.node(&d, &[]).unwrap().child_count, 2);
        // The second one starts where the first said it would end.
        let second = ev.node(&d, &[1]).unwrap();
        assert_eq!(second.offset_bits, one.len() as u64 * 8);
    }

    #[test]
    fn the_indicator_says_the_edition_and_the_length() {
        let bytes = message_bytes();
        let len = bytes.len() as u128;
        let d = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(grib());
        let discipline = ev.node(&d, &[0, 1, 1]).unwrap();
        assert_eq!(discipline.value, Value::Enum { raw: 0, name: Some("meteorological".into()), hex: false });
        assert_eq!(ev.node(&d, &[0, 1, 2]).unwrap().value, Value::UInt(2));
        assert_eq!(ev.node(&d, &[0, 1, 3]).unwrap().value, Value::UInt(len));
    }

    #[test]
    fn the_sections_run_to_the_end_of_the_message() {
        let d = Document::new(MemSource(message_bytes()));
        let mut ev = Evaluator::new(grib());
        // Six numbered sections and the end marker.
        let sections = ev.node(&d, &[0, 1, 4]).unwrap();
        assert_eq!(sections.child_count, 7);
        let numbers: Vec<_> = (0..6)
            .map(|i| ev.node(&d, &[0, 1, 4, i, 1]).unwrap().value.as_int().unwrap())
            .collect();
        assert_eq!(numbers, vec![1, 3, 4, 5, 6, 7]);
        assert_eq!(ev.node(&d, &[0, 1, 4, 6, 0]).unwrap().size_bits, 4 * 8);
    }

    #[test]
    fn a_section_is_named_by_the_number_in_it() {
        let d = Document::new(MemSource(message_bytes()));
        let mut ev = Evaluator::new(grib());
        assert_eq!(ev.node(&d, &[0, 1, 4, 0]).unwrap().name, "[0] identification");
        assert_eq!(ev.node(&d, &[0, 1, 4, 3]).unwrap().name, "[3] data representation");
    }

    #[test]
    fn the_reference_time_reads_as_the_moment_it_is() {
        let d = Document::new(MemSource(message_bytes()));
        let mut ev = Evaluator::new(grib());
        let centre = ev.node(&d, &[0, 1, 4, 0, 2, 0]).unwrap();
        assert_eq!(centre.value, Value::Enum { raw: 98, name: Some("Reading, ECMWF".into()), hex: false });
        assert_eq!(ev.node(&d, &[0, 1, 4, 0, 2, 5]).unwrap().value, Value::UInt(2026));
        assert_eq!(ev.node(&d, &[0, 1, 4, 0, 2, 6]).unwrap().value, Value::UInt(9));
        assert_eq!(ev.node(&d, &[0, 1, 4, 0, 2, 8]).unwrap().value, Value::UInt(6));
    }

    #[test]
    fn a_latitude_longitude_grid_reads_as_its_corners() {
        let d = Document::new(MemSource(message_bytes()));
        let mut ev = Evaluator::new(grib());
        let grid = ev.node(&d, &[0, 1, 4, 1, 2]).unwrap();
        assert_eq!(grid.type_name, "GridDefinition");
        assert_eq!(ev.node(&d, &[0, 1, 4, 1, 2, 1]).unwrap().value, Value::UInt(6));
        let template = ev.node(&d, &[0, 1, 4, 1, 2, 4]).unwrap();
        assert_eq!(template.value, Value::Enum { raw: 0, name: Some("latitude/longitude".into()), hex: false });
        // Three by two points.
        assert_eq!(ev.node(&d, &[0, 1, 4, 1, 2, 5, 7]).unwrap().value, Value::UInt(3));
        assert_eq!(ev.node(&d, &[0, 1, 4, 1, 2, 5, 8]).unwrap().value, Value::UInt(2));
        // A southern latitude, which the file writes as sign and magnitude.
        let first = ev.node(&d, &[0, 1, 4, 1, 2, 5, 11]).unwrap();
        assert_eq!(first.value, Value::UInt(0x8000_0000 | 45_000_000));
    }

    #[test]
    fn the_product_says_which_parameter_and_how_far_ahead() {
        let d = Document::new(MemSource(message_bytes()));
        let mut ev = Evaluator::new(grib());
        let template = ev.node(&d, &[0, 1, 4, 2, 2, 1]).unwrap();
        assert_eq!(template.value, Value::Enum { raw: 0, name: Some("analysis or forecast at a level".into()), hex: false });
        let unit = ev.node(&d, &[0, 1, 4, 2, 2, 2, 7]).unwrap();
        assert_eq!(unit.value, Value::Enum { raw: 1, name: Some("hour".into()), hex: false });
        assert_eq!(ev.node(&d, &[0, 1, 4, 2, 2, 2, 8]).unwrap().value, Value::UInt(6));
    }

    #[test]
    fn simple_packing_says_the_reference_value_and_the_width() {
        let d = Document::new(MemSource(message_bytes()));
        let mut ev = Evaluator::new(grib());
        assert_eq!(ev.node(&d, &[0, 1, 4, 3, 2, 0]).unwrap().value, Value::UInt(6));
        let kind = ev.node(&d, &[0, 1, 4, 3, 2, 1]).unwrap();
        assert_eq!(kind.value, Value::Enum { raw: 0, name: Some("simple packing".into()), hex: false });
        assert_eq!(ev.node(&d, &[0, 1, 4, 3, 2, 2, 0]).unwrap().value, Value::Float(270.0));
        assert_eq!(ev.node(&d, &[0, 1, 4, 3, 2, 2, 3]).unwrap().value, Value::UInt(8));
        // The data itself is bytes, as many as the section holds.
        assert_eq!(ev.node(&d, &[0, 1, 4, 5, 2, 0]).unwrap().size_bits, 6 * 8);
    }

    #[test]
    fn a_section_written_to_a_template_this_does_not_know_stays_bytes() {
        // The same message with a grid template of 30, a Lambert conformal
        // grid: named, and its fields left alone.
        let mut bytes = message_bytes();
        let at = bytes.windows(4).position(|w| w == [0, 0, 0, 6]).expect("the grid's point count");
        // The template number is two bytes after the count and the two
        // optional-list bytes.
        bytes[at + 6] = 0;
        bytes[at + 7] = 30;
        let d = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(grib());
        let number = ev.node(&d, &[0, 1, 4, 1, 2, 4]).unwrap();
        assert_eq!(number.value, Value::Enum { raw: 30, name: Some("Lambert conformal".into()), hex: false });
        let template = ev.node(&d, &[0, 1, 4, 1, 2, 5]).unwrap();
        assert_eq!(template.type_name, "bytes[]");
    }

    #[test]
    fn an_edition_1_message_reads_as_its_sections_and_no_further() {
        // Four sections of three-byte length, and the end marker.
        let mut body = Vec::new();
        for (len, fill) in [(28usize, 1u8), (32, 2), (6, 3), (11, 4)] {
            body.extend_from_slice(&(len as u32).to_be_bytes()[1..]);
            body.extend(std::iter::repeat_n(fill, len - 3));
        }
        body.extend_from_slice(b"7777");
        let mut b = b"GRIB".to_vec();
        b.extend_from_slice(&((body.len() + 8) as u32).to_be_bytes()[1..]);
        b.push(1); // edition 1
        b.extend_from_slice(&body);
        let d = Document::new(MemSource(b));
        let mut ev = Evaluator::new(grib());
        let grib1 = ev.node(&d, &[0, 1]).unwrap();
        assert_eq!(grib1.type_name, "Grib1");
        assert_eq!(ev.node(&d, &[0, 1, 1]).unwrap().value, Value::UInt(1));
        let sections = ev.node(&d, &[0, 1, 2]).unwrap();
        assert_eq!(sections.child_count, 5);
        assert_eq!(ev.node(&d, &[0, 1, 2, 1, 0]).unwrap().value, Value::UInt(32));
        assert_eq!(ev.node(&d, &[0, 1, 2, 1, 1]).unwrap().size_bits, 29 * 8);
    }

    #[test]
    fn a_message_cut_off_in_the_middle_reads_as_far_as_it_goes() {
        let mut bytes = message_bytes();
        bytes.truncate(bytes.len() - 20);
        let d = Document::new(MemSource(bytes.clone()));
        let mut ev = Evaluator::new(grib());
        let sections = ev.node(&d, &[0, 1, 4]).unwrap();
        // The window the length asked for is clamped to what is there.
        assert_eq!(sections.offset_bits + sections.size_bits, bytes.len() as u64 * 8);
    }
}
