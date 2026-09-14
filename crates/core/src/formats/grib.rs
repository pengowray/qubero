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
//! reference time, the grids that are latitude/longitude, Gaussian, polar
//! stereographic or Lambert conformal (grid templates 3.0, 3.40, 3.20 and
//! 3.30), a forecast at a level, an ensemble member and a field processed over
//! an interval (product templates 4.0, 4.1 and 4.8), the packing headers
//! (5.0, 5.2, 5.3, 5.40, 5.41, 5.42), the bitmap indicator, and section 7's
//! values when they are simply or complexly packed. A section written to a
//! template this does not know keeps its length and its number and its
//! contents stay bytes, which is the honest answer: the templates are a WMO
//! table of several hundred and each one is a different set of fields.
//!
//! Section 7 reads as its numbers for simple packing (5.0) and for the complex
//! packings (5.2 and 5.3), which are what an operational forecast is actually
//! distributed as. Complex packing cuts the grid into groups and writes three
//! tables in front of the data saying what each group is worth, how wide its
//! values are and how many it holds; the run under one group is then that
//! group's width a value. See [`complex_packed_data`].
//!
//! What a packed value is worth is a float plus the packed number times a
//! power of two, over a power of ten. For simple packing that is a field of
//! its own beside each value, worked out as a real. For complex packing it is
//! a second question and not a field, since 5.3 adds a running sum on top of
//! it. See [`data`] for the formula and [`grib_values`](super::grib_values)
//! for the reading that carries it out for both.
//!
//! The image packings hold a whole codestream of another format. A PNG one
//! (5.41) opens as a PNG, since that is a format this crate reads; a JPEG 2000
//! one (5.40) is named and stays bytes, since it is not.
//!
//! Edition 1 is still published and is a different layout: three-byte lengths,
//! no section numbers, and the sections identified by their order and by flags
//! in the first two. It reads here as its five sections: the product
//! definition, the grid, the bitmap, the packed data and the end marker.
//!
//! Sign and magnitude is the trap in this format. A negative latitude and a
//! negative scale factor are written as a magnitude with the top bit set, not
//! as two's complement, so 0x80000005 is -5 and reading it as an `Int` would
//! answer with a number near the bottom of the range instead. Those fields are
//! `sign_magnitude` here and read as the numbers they are. Which fields those
//! are is the format's business and not a rule: the increments of a plain
//! latitude/longitude grid are unsigned, while the LaD, LoV and Latin angles
//! of a projected one are not, and section 5's two scale factors are sixteen
//! bits of it.

use super::grib1::edition1;
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
pub(super) const CENTRE: &[(i128, &str)] = &[
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

/// How a field was processed over its interval, from code table 4.10.
const STATISTICAL_PROCESS: &[(i128, &str)] = &[
    (0, "average"),
    (1, "accumulation"),
    (2, "maximum"),
    (3, "minimum"),
    (4, "difference, end less start"),
    (5, "root mean square"),
    (6, "standard deviation"),
    (7, "covariance"),
    (9, "ratio"),
    (10, "standardised anomaly"),
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

pub(super) fn u16be() -> T {
    T::u16(Big)
}

fn u32be() -> T {
    T::u32(Big)
}

/// A signed angle or scale factor, whose top bit says which way it goes rather
/// than being part of the number. See [`T::sign_magnitude`].
fn sm32() -> T {
    T::sign_magnitude(32, Big)
}

fn sm24() -> T {
    T::sign_magnitude(24, Big)
}

pub(super) fn sm16() -> T {
    T::sign_magnitude(16, Big)
}

fn sm8() -> T {
    T::sign_magnitude(8, Big)
}

pub fn grib() -> Template {
    Template::new("grib", T::repeat(T::Named("Chunk".into()), Until::End))
        .with_type("Chunk", chunk())
        .with_type("Message", message())
        .with_type("Section", section())
}

/// What is at the top of the file, over and over. Almost always a message; the
/// two other answers are what a file collected by hand rather than written by
/// a library looks like.
///
/// A message says how long it is and the next one starts there, so nothing
/// separates them and nothing has to. Real files disagree: a message subset
/// pulled out by byte range keeps whatever the tool put between them, an
/// archive concatenated by a shell script has a newline after each one, and
/// some feeds pad to a block. Anything that is not the letters `GRIB` is that,
/// and reads as the run of bytes up to where the next message starts, so one
/// stray byte does not take the rest of the file with it.
fn chunk() -> T {
    T::switch(
        look_ahead(),
        vec![
            (0x4752_4942, T::Named("Message".into())),
            (TRUNCATED, T::structure("Trailing", vec![("bytes", T::bytes(E::Remaining))])),
        ],
        // At least one byte, since the look-ahead has already said these four
        // are not `GRIB`, so the run of chunks always moves on.
        T::structure("BetweenMessages", vec![("bytes", T::bytes(E::to_bytes(b"GRIB")))]),
    )
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
            (
                "template",
                T::switch(
                    E::field("template_number"),
                    vec![
                        (0, latlon_grid()),
                        (20, polar_stereographic()),
                        (30, lambert_conformal()),
                        (40, gaussian_grid()),
                    ],
                    T::bytes(E::Remaining),
                ),
            ),
        ],
    )
}

/// The seven fields every grid template opens with: which ellipsoid the
/// angles are against, and how big it is. Three scale factor and value pairs,
/// of which a given shape uses at most one.
fn earth() -> Vec<(&'static str, T)> {
    vec![
        ("shape_of_earth", T::u8()),
        ("radius_scale_factor", T::u8()),
        ("scaled_radius", u32be()),
        ("major_axis_scale_factor", T::u8()),
        ("scaled_major_axis", u32be()),
        ("minor_axis_scale_factor", T::u8()),
        ("scaled_minor_axis", u32be()),
    ]
}

/// Grid template 3.0. The corners are in millionths of a degree, and a
/// southern latitude is written as a magnitude with the top bit set rather
/// than as a two's complement negative. The increments are unsigned: which way
/// the grid is walked is in `scanning_mode`, not in their sign.
fn latlon_grid() -> T {
    let mut fields = earth();
    fields.extend(vec![
        ("ni", u32be()),
        ("nj", u32be()),
        ("basic_angle", u32be()),
        ("basic_angle_subdivisions", u32be()),
        ("first_latitude", sm32()),
        ("first_longitude", sm32()),
        ("resolution_flags", T::u8()),
        ("last_latitude", sm32()),
        ("last_longitude", sm32()),
        ("i_increment", u32be()),
        ("j_increment", u32be()),
        ("scanning_mode", T::u8()),
        ("rest", T::bytes(E::Remaining)),
    ]);
    T::structure("LatLonGrid", fields).payload(&["ni", "nj"])
}

/// Grid template 3.40: a Gaussian latitude/longitude grid. The same shape as
/// 3.0 as far as the last corner, and then `n`, a quarter of the number of
/// parallels between a pole and the equator, in place of the north-south
/// increment: the rows of a Gaussian grid are not evenly spaced, so there is
/// no increment to write.
fn gaussian_grid() -> T {
    let mut fields = earth();
    fields.extend(vec![
        ("ni", u32be()),
        ("nj", u32be()),
        ("basic_angle", u32be()),
        ("basic_angle_subdivisions", u32be()),
        ("first_latitude", sm32()),
        ("first_longitude", sm32()),
        ("resolution_flags", T::u8()),
        ("last_latitude", sm32()),
        ("last_longitude", sm32()),
        ("i_increment", u32be()),
        ("n", u32be()),
        ("scanning_mode", T::u8()),
        ("rest", T::bytes(E::Remaining)),
    ]);
    T::structure("GaussianGrid", fields).payload(&["ni", "nj", "n"])
}

/// Grid template 3.20: polar stereographic. The grid is on a plane touching
/// the earth at one of the poles, so its spacing is in metres rather than in
/// degrees, and `lad` is the latitude the metres are true at.
fn polar_stereographic() -> T {
    let mut fields = earth();
    fields.extend(vec![
        ("nx", u32be()),
        ("ny", u32be()),
        ("first_latitude", sm32()),
        ("first_longitude", sm32()),
        ("resolution_flags", T::u8()),
        ("lad", sm32()),
        ("lov", sm32()),
        ("dx", u32be()),
        ("dy", u32be()),
        ("projection_centre_flags", T::u8()),
        ("scanning_mode", T::u8()),
        ("rest", T::bytes(E::Remaining)),
    ]);
    T::structure("PolarStereographic", fields).payload(&["nx", "ny"])
}

/// Grid template 3.30: Lambert conformal. A cone rather than a plane, so there
/// are two standard parallels it is true at, and the pole the cone is about is
/// written as well.
fn lambert_conformal() -> T {
    let mut fields = earth();
    fields.extend(vec![
        ("nx", u32be()),
        ("ny", u32be()),
        ("first_latitude", sm32()),
        ("first_longitude", sm32()),
        ("resolution_flags", T::u8()),
        ("lad", sm32()),
        ("lov", sm32()),
        ("dx", u32be()),
        ("dy", u32be()),
        ("projection_centre_flags", T::u8()),
        ("scanning_mode", T::u8()),
        ("latin1", sm32()),
        ("latin2", sm32()),
        ("south_pole_latitude", sm32()),
        ("south_pole_longitude", sm32()),
        ("rest", T::bytes(E::Remaining)),
    ]);
    T::structure("LambertConformal", fields).payload(&["nx", "ny"])
}

/// Section 4: what the numbers are of. `nv` counts the vertical coordinate
/// values written after the template, and is almost always zero.
fn product_definition() -> T {
    T::structure(
        "ProductDefinition",
        vec![
            ("nv", u16be()),
            ("template_number", T::enumeration("ProductTemplate", u16be(), PRODUCT_TEMPLATE)),
            (
                "template",
                T::switch(
                    E::field("template_number"),
                    vec![(0, forecast_at_level()), (1, ensemble_forecast()), (8, statistical_interval())],
                    T::bytes(E::Remaining),
                ),
            ),
        ],
    )
}

/// What every product template starts with, through to the two surfaces that
/// say which level the field is at. Templates 4.1 and 4.8 are template 4.0 and
/// then more, so the common run is written once.
///
/// The parameter is a category and a number within it, read against the
/// discipline in section 0: category 0 number 0 of discipline 0 is
/// temperature. Naming all of those would be the whole of WMO table 4.2.
///
/// A surface's scale factor is signed and its value is not: a level at 0.1 hPa
/// is a value of 1 scaled by -4, and the sign of the -4 is the top bit. The
/// forecast time is signed too, which is how an analysis increment says it is
/// about a moment before its own reference time.
fn product_head() -> Vec<(&'static str, T)> {
    vec![
        ("parameter_category", T::u8()),
        ("parameter_number", T::u8()),
        ("generating_process_type", T::u8()),
        ("background_process", T::u8()),
        ("generating_process", T::u8()),
        ("hours_after_cutoff", u16be()),
        ("minutes_after_cutoff", T::u8()),
        ("time_unit", T::enumeration("TimeUnit", T::u8(), TIME_UNIT)),
        ("forecast_time", sm32()),
        ("first_surface_type", surface_type()),
        ("first_surface_scale_factor", surface_scale()),
        ("first_surface_value", surface_value()),
        ("second_surface_type", surface_type()),
        ("second_surface_scale_factor", surface_scale()),
        ("second_surface_value", surface_value()),
    ]
}

/// A field a message fills in with all ones to say it has nothing to put
/// there. Most fields at one level have no second surface, and every one of
/// its three fields is written that way rather than left out; read as numbers
/// they are a surface of type 255 at a scale of -127.
fn surface_type() -> T {
    T::unset_int(T::u8(), 255)
}

fn surface_scale() -> T {
    T::unset_int(sm8(), -127)
}

fn surface_value() -> T {
    T::unset_int(u32be(), 0xFFFF_FFFF)
}

/// Product template 4.0: an analysis or a forecast at one level.
fn forecast_at_level() -> T {
    let mut fields = product_head();
    fields.push(("rest", T::bytes(E::Remaining)));
    T::structure("ForecastAtLevel", fields)
        .payload(&["parameter_category", "parameter_number", "forecast_time"])
}

/// Product template 4.1: one member of an ensemble. The same as 4.0, and then
/// which member of how many this is, which is what tells apart the fifty
/// copies of a field an ensemble run publishes.
fn ensemble_forecast() -> T {
    let mut fields = product_head();
    fields.extend(vec![
        ("ensemble_type", T::u8()),
        ("perturbation_number", T::u8()),
        ("ensemble_size", T::u8()),
        ("rest", T::bytes(E::Remaining)),
    ]);
    T::structure("EnsembleForecast", fields)
        .payload(&["parameter_category", "parameter_number", "perturbation_number"])
}

/// Product template 4.8: a field processed over an interval rather than read
/// at a moment. Total precipitation and a daily maximum are both this: the
/// time in `forecast_time` is when the interval starts, and the end of it is
/// written out as a date. `n_ranges` counts the descriptions of how the
/// processing was done, and each is twelve bytes; they stay bytes here,
/// because one is almost always all there is and the fields of the second
/// mean nothing without the first.
fn statistical_interval() -> T {
    let mut fields = product_head();
    fields.extend(vec![
        ("end_year", u16be()),
        ("end_month", T::u8()),
        ("end_day", T::u8()),
        ("end_hour", T::u8()),
        ("end_minute", T::u8()),
        ("end_second", T::u8()),
        ("n_ranges", T::u8()),
        ("missing_values", u32be()),
        ("statistical_process", T::enumeration("StatisticalProcess", T::u8(), STATISTICAL_PROCESS)),
        ("time_increment_type", T::u8()),
        ("range_unit", T::enumeration("TimeUnit", T::u8(), TIME_UNIT)),
        ("range_length", u32be()),
        ("increment_unit", T::enumeration("TimeUnit", T::u8(), TIME_UNIT)),
        ("increment_length", u32be()),
        ("rest", T::bytes(E::Remaining)),
    ]);
    T::structure("StatisticalInterval", fields)
        .payload(&["parameter_category", "parameter_number", "statistical_process", "range_length"])
}

/// Section 5: how many numbers there are and how they were packed.
fn data_representation() -> T {
    T::structure(
        "DataRepresentation",
        vec![
            ("number_of_values", u32be()),
            ("template_number", T::enumeration("PackingTemplate", u16be(), PACKING)),
            (
                "template",
                T::switch(
                    E::field("template_number"),
                    vec![
                        (0, simple_packing()),
                        (2, complex_packing(false)),
                        (3, complex_packing(true)),
                        (40, image_packing("Jpeg2000Packing", true)),
                        (41, image_packing("PngPacking", false)),
                        (42, ccsds_packing()),
                    ],
                    T::bytes(E::Remaining),
                ),
            ),
        ],
    )
}

/// The five fields every packing template opens with: what one value is worth.
/// Both exponents are sixteen bits of sign and magnitude, which is the one
/// place in the format where a negative number is written that narrow.
fn packing_head() -> Vec<(&'static str, T)> {
    vec![
        ("reference_value", T::F32(Big)),
        ("binary_scale_factor", sm16()),
        ("decimal_scale_factor", sm16()),
        ("bits_per_value", T::u8()),
        ("original_field_type", T::u8()),
    ]
}

/// Data template 5.0, simple packing. Each packed number is `bits_per_value`
/// bits wide, and section 7 reads them as that. See [`data`] for the formula
/// that turns one back into a measurement.
fn simple_packing() -> T {
    let mut fields = packing_head();
    fields.push(("rest", T::bytes(E::Remaining)));
    T::structure("SimplePacking", fields).payload(&["reference_value", "bits_per_value"])
}

/// Data templates 5.2 and 5.3, complex packing, with spatial differencing in
/// 5.3. The values are cut into groups, each group gets its own reference and
/// its own width, and the widths and references are themselves packed at the
/// front of section 7.
///
/// This is the header, which says how the groups were chosen and how wide each
/// of the three tables in front of the data is. Every number in it is read
/// again by section 7, which is where the groups are; see
/// [`complex_packed_data`].
fn complex_packing(spatial: bool) -> T {
    let mut fields = packing_head();
    fields.extend(vec![
        ("group_splitting_method", T::u8()),
        ("missing_value_management", T::u8()),
        ("primary_missing_value", u32be()),
        ("secondary_missing_value", u32be()),
        ("n_groups", u32be()),
        ("group_widths_reference", T::u8()),
        ("group_widths_bits", T::u8()),
        ("group_lengths_reference", u32be()),
        ("group_length_increment", T::u8()),
        ("last_group_length", u32be()),
        ("group_lengths_bits", T::u8()),
    ]);
    if spatial {
        fields.extend(vec![
            // First-order differencing subtracts each value from the one
            // before it, second-order the differences again; `extra_bytes` is
            // how wide the first values and the overall minimum are written.
            ("spatial_differencing_order", T::u8()),
            ("extra_bytes", T::u8()),
        ]);
    }
    fields.push(("rest", T::bytes(E::Remaining)));
    let name = if spatial { "ComplexPackingSpatial" } else { "ComplexPacking" };
    T::structure(name, fields).payload(&["reference_value", "bits_per_value", "n_groups"])
}

/// Data templates 5.40 and 5.41: the grid packed as an image, JPEG 2000 or
/// PNG. The header says what a sample is worth, the same as simple packing
/// does, and section 7 is then a whole codestream of that format rather than a
/// run of numbers. See [`png_packed_data`] and [`jpeg2000_packed_data`] for
/// what becomes of it.
fn image_packing(name: &str, jpeg: bool) -> T {
    let mut fields = packing_head();
    if jpeg {
        // Zero is lossless, and anything else is the ratio it was told to
        // reach. PNG has neither field: it is lossless and that is all.
        fields.push(("compression_type", T::u8()));
        fields.push(("target_compression_ratio", T::u8()));
    }
    fields.push(("rest", T::bytes(E::Remaining)));
    T::structure(name, fields).payload(&["reference_value", "bits_per_value"])
}

/// Data template 5.42: CCSDS 121.0, the Rice coding a spacecraft downlink
/// uses. Section 7 is that stream and stays bytes.
fn ccsds_packing() -> T {
    let mut fields = packing_head();
    fields.extend(vec![
        ("ccsds_flags", T::u8()),
        ("block_size", T::u8()),
        ("reference_sample_interval", u16be()),
        ("rest", T::bytes(E::Remaining)),
    ]);
    T::structure("CcsdsPacking", fields).payload(&["reference_value", "bits_per_value"])
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
/// Simply packed data reads as its values. How wide one is and how many there
/// are is section 5's business, and `Expr::Sibling` walks back through the
/// sections to ask it: `bits_per_value` and `number_of_values`, from the
/// nearest section before this one that has them. Both are copied into fields
/// of no bits first, so that the run is measured once by arithmetic rather
/// than a million times by walking; see [`T::uint_expr`].
///
/// A packed value is not the measurement. What section 5 says about it is
///
/// ```text
/// Y = (R + X * 2^E) / 10^D
/// ```
///
/// where X is the packed number, R is `reference_value`, E is
/// `binary_scale_factor` and D is `decimal_scale_factor`. For simple packing
/// that is written as a real beside each value, so a value reads as the packed
/// integer and what it is worth; see [`simple_packed_data`]. R is a float and
/// E and D are negative as often as not, which is what a real and its powers
/// are for.
///
/// Complex packing reads as its values too, by way of the three tables the
/// groups are described by; see [`complex_packed_data`]. What one of those
/// values is worth takes the same formula and one or three more steps in front
/// of it. For 5.2, X is not what is written: what is written is how far the
/// point is above its own group's reference, so
///
/// ```text
/// X = group_references[g] + written
/// ```
///
/// For 5.3 what is written is not the value at all but a difference: between
/// each value and the one before it at first order, or between those
/// differences at second order, with the smallest difference in the message
/// taken off every one of them so that none is negative. Undoing that is to
/// add the minimum back, put the first one or two values back as section 7
/// wrote them whole, and then run the sum forward over the whole grid. A
/// running sum is the one thing no expression here could ever be, however much
/// arithmetic were added to the IR: every value depends on the one before it.
/// So it is a reading in Rust, next to the template rather than in it. See
/// [`grib_values`](super::grib_values).
///
/// An image-packed section holds a whole codestream of another format. The PNG
/// one opens as a PNG, since that is a format this already reads; the JPEG 2000
/// one is named and stays bytes, since it is not.
fn data() -> T {
    T::switch(
        // Which packing, from the nearest earlier section that says: section
        // 5. Nothing earlier saying anything answers 0, which is also simple
        // packing, and the counts then come to nothing as well, so a section 7
        // with no section 5 in front of it reads as no values rather than as
        // a wrong number of them.
        E::sibling(&["body", "template_number"]),
        vec![
            (0, simple_packed_data()),
            (2, complex_packed_data(false)),
            (3, complex_packed_data(true)),
            (40, jpeg2000_packed_data()),
            (41, png_packed_data()),
        ],
        T::structure("PackedData", vec![("values", T::bytes(E::Remaining))]),
    )
}

/// Section 7 for simple packing: each value the packed integer, and beside it
/// what section 5 says it is worth.
///
/// What goes into the worth is copied in from section 5 first, the reference
/// value as the real it is and the two scale factors as whole numbers, for the
/// reason the width is: a value asking section 5 itself would walk back
/// through every value before it to get there, and a grid is a million of
/// them. The worth then names fields beside the run, and a run of values is
/// still placed by arithmetic, since the worth takes no bits.
fn simple_packed_data() -> T {
    let worth = E::field("reference_value")
        .add(E::field("stored").mul(E::pow2(E::field("binary_scale_factor"))))
        .div(E::pow10(E::field("decimal_scale_factor")));
    let value = T::inline_structure(
        "Packed",
        vec![("stored", T::uint_expr(E::field("bits_per_value"), Big)), ("worth", T::computed_real(worth))],
    )
    .payload(&["stored"]);
    T::structure(
        "PackedData",
        vec![
            ("bits_per_value", T::computed(E::sibling(&["body", "template", "bits_per_value"]))),
            ("count", T::computed(E::sibling(&["body", "number_of_values"]))),
            ("reference_value", T::computed_real(E::sibling(&["body", "template", "reference_value"]))),
            from_packing("binary_scale_factor"),
            from_packing("decimal_scale_factor"),
            ("values", T::array(value, E::field("count"))),
        ],
    )
    .machinery(&["bits_per_value", "count", "reference_value", "binary_scale_factor", "decimal_scale_factor"])
    .packed_as(super::grib_values::PACKING)
    .payload(&["values"])
}

/// What one of section 5's numbers is called when section 7 copies it in to
/// place its own runs by. Always the same walk: back to the nearest earlier
/// section with a `template` in it, which is section 5, and then the named
/// field of whichever packing template it wrote.
fn from_packing(name: &'static str) -> (&'static str, T) {
    // The name is handed back with the type so that a run of these reads as
    // one line each rather than as the same expression written out twice, and
    // so that section 7's field goes by the name section 5 gave it.
    (name, T::computed(E::sibling(&["body", "template", name])))
}

/// Section 7 for data templates 5.2 and 5.3: complex packing, with spatial
/// differencing in 5.3.
///
/// The grid is cut into groups of neighbouring points that are alike, and each
/// group is packed to whatever width its own numbers need: a run of sea-level
/// pressure over flat ocean is a group whose values differ by so little that
/// three bits hold each one, next to a group crossing a front that needs
/// eleven. What makes that readable is three tables at the front of the
/// section, each with one entry per group and each packed to a width section 5
/// gives:
///
/// - `group_references`, the number every value in the group is counted from,
///   each `bits_per_value` wide;
/// - `group_widths`, how many bits one of that group's values takes, each
///   `group_widths_bits` wide and counted up from `group_widths_reference`;
/// - `group_lengths`, how many values the group holds, each
///   `group_lengths_bits` wide, multiplied by `group_length_increment` and
///   counted up from `group_lengths_reference`. The last group is the
///   exception: its length is `last_group_length`, written in section 5,
///   because the scaled form could not hold it.
///
/// Each of the three is padded out with zero bits so that it ends on a byte,
/// which data template 7.2 requires of each sequence in turn. Nothing in the
/// file says how much: it is what the count and the width come to, rounded up.
/// Two of the three messages in the GFS sample have tables that land on a byte
/// anyway and read the same either way; the third does not and is unreadable
/// without it.
///
/// Then the values, group by group, each group's run at that group's width.
/// That is the connection worth seeing on screen: `groups[7] width` is
/// `group_widths[7]` plus the reference, and the run under it is that many bits
/// a value. So it is written as a field and not folded into the array's type.
///
/// 5.3 puts three more numbers in front of everything, each
/// `octets_for_extra_descriptors` wide: the first one or two values of the
/// grid, kept whole because a difference has to start somewhere, and the
/// smallest difference in the message, which is taken off every other one so
/// that none of them is negative. See [`grib_values`](super::grib_values) for
/// undoing that.
///
/// What this does not say: `missing_value_management` past 0. A message that
/// uses it writes a group reference of all ones to mean the group is missing,
/// and the widths and lengths are unchanged, so the fields here still land on
/// the right bits and only what they are worth differs.
fn complex_packed_data(spatial: bool) -> T {
    let mut fields = vec![
        from_packing("bits_per_value"),
        from_packing("n_groups"),
        from_packing("group_widths_reference"),
        from_packing("group_widths_bits"),
        from_packing("group_lengths_reference"),
        from_packing("group_length_increment"),
        from_packing("last_group_length"),
        from_packing("group_lengths_bits"),
    ];
    if spatial {
        fields.extend(vec![from_packing("spatial_differencing_order"), from_packing("extra_bytes")]);
        let wide = E::field("extra_bytes").mul(E::lit(8));
        fields.extend(vec![
            // One value for first-order differencing and two for second, which
            // is what the order says. Read as they were written, whole, with
            // no reference and no scaling of their own.
            ("first_values", T::array(T::uint_expr(wide.clone(), Big), E::field("spatial_differencing_order"))),
            // The one negative number in the section, and written the way this
            // format writes those: a magnitude with the top bit set. How wide
            // it is is not a constant, so which of the four widths it is read
            // at is a switch rather than a type.
            ("overall_minimum", extra_descriptor()),
        ]);
    }
    fields.extend(vec![
        ("group_references", T::array(T::uint_expr(E::field("bits_per_value"), Big), E::field("n_groups"))),
        ("group_references_padding", table_padding("bits_per_value")),
        ("group_widths", T::array(T::uint_expr(E::field("group_widths_bits"), Big), E::field("n_groups"))),
        ("group_widths_padding", table_padding("group_widths_bits")),
        ("group_lengths", T::array(T::uint_expr(E::field("group_lengths_bits"), Big), E::field("n_groups"))),
        ("group_lengths_padding", table_padding("group_lengths_bits")),
        ("groups", T::array(group(), E::field("n_groups"))),
    ]);
    let machinery: Vec<&str> = fields
        .iter()
        .map(|(n, _)| *n)
        .filter(|n| !matches!(*n, "first_values" | "overall_minimum" | "group_references" | "group_widths" | "group_lengths" | "groups"))
        .collect();
    T::structure("ComplexPackedData", fields).machinery(&machinery).payload(&["groups"]).packed_as(super::grib_values::PACKING)
}

/// The bits between the end of a table of `n_groups` entries `bits` wide and
/// the byte the next table starts on. Zero when the table already ends on one,
/// which is the usual case and is why a reader that leaves this out still gets
/// most messages right.
fn table_padding(bits: &str) -> T {
    T::uint_expr(E::field("n_groups").mul(E::field(bits)).pad_to(8), Big)
}

/// The overall minimum of the differences, read at whatever width
/// `extra_bytes` gives it. One to four octets is what the field can say; a
/// message claiming anything else has the run stay bytes rather than read at a
/// width nothing supports.
fn extra_descriptor() -> T {
    T::switch(
        E::field("extra_bytes"),
        vec![(1, sm8()), (2, sm16()), (3, sm24()), (4, sm32())],
        T::bytes(E::field("extra_bytes")),
    )
}

/// One group: how wide its values are, how many there are, and them.
///
/// Both numbers are read out of the tables in front rather than out of the
/// file here, which is the whole of what complex packing is. `Expr::Idx` is
/// this group's place in the run, so `group_widths[Idx]` is its own entry and
/// not another group's, and the width is copied into a field of no bits so the
/// run under it is measured once by arithmetic; see [`T::uint_expr`].
fn group() -> T {
    // The last group's length is not in the table: the table holds a scaled
    // form that could not reach it, so section 5 wrote the real one. Which
    // makes the count a choice rather than a sum, and a switch says so on
    // screen better than the arithmetic that would stand in for it.
    let not_last = E::Idx.add(E::lit(1)).less_than(E::field("n_groups"));
    let scaled = E::field("group_lengths_reference")
        .add(E::field("group_length_increment").mul(E::elem("group_lengths", E::Idx)));
    T::structure(
        "Group",
        vec![
            ("width", T::computed(E::elem("group_widths", E::Idx).add(E::field("group_widths_reference")))),
            ("count", T::switch(not_last, vec![(1, T::computed(scaled))], T::computed(E::field("last_group_length")))),
            ("values", T::array(T::uint_expr(E::field("width"), Big), E::field("count"))),
        ],
    )
    .machinery(&["width", "count"])
    .payload(&["values"])
    .counted_as("group")
}

/// Section 7 for data template 5.41: the grid as a PNG.
///
/// The section holds a whole PNG file, signature and chunks and all, whose
/// pixels are the packed values. So it is declared as a run that is already
/// what it is, and the reader opens it as a PNG the way a stored ZIP member
/// opens as whatever it holds: the inner template says only bytes, which is
/// what a reading sniffs.
///
/// What the pixels are worth is section 5's business and the same formula
/// simple packing uses, one sample being one value. Inflating the image is
/// not done here.
fn png_packed_data() -> T {
    T::structure("PngPackedData", vec![("png", T::decoded(E::Remaining, crate::codec::Codec::Stored, super::decoded_text()))])
}

/// Section 7 for data template 5.40: the grid as a JPEG 2000 codestream.
///
/// Named and left as bytes. There is no JPEG 2000 template in this crate to
/// open it with, so sniffing it would find nothing, and a codestream is not a
/// run of values with a stride: it is wavelet coefficients in code blocks,
/// arithmetic coded. The section says which format it is holding and stops
/// there.
fn jpeg2000_packed_data() -> T {
    T::structure("Jpeg2000PackedData", vec![("codestream", T::bytes(E::Remaining))])
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
        b.extend_from_slice(&0x8001u16.to_be_bytes()); // binary scale of -1
        b.extend_from_slice(&0u16.to_be_bytes()); // decimal scale
        b.extend_from_slice(&[8, 0]);
        b
    }

    /// An edition 2 message holding just these sections, for a test that is
    /// about one of them.
    fn one_message(sections: &[(u8, Vec<u8>)]) -> Vec<u8> {
        let mut body = Vec::new();
        for (number, bytes) in sections {
            body.extend_from_slice(&sec(*number, bytes));
        }
        body.extend_from_slice(b"7777");
        let mut b = b"GRIB".to_vec();
        b.extend_from_slice(&[0, 0, 0, 2]);
        b.extend_from_slice(&((body.len() + 16) as u64).to_be_bytes());
        b.extend_from_slice(&body);
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
        // A southern latitude, which the file writes as sign and magnitude:
        // the top bit and a magnitude, not a two's complement negative.
        let first = ev.node(&d, &[0, 1, 4, 1, 2, 5, 11]).unwrap();
        assert_eq!(first.value, Value::Int(-45_000_000));
        assert_eq!(ev.node(&d, &[0, 1, 4, 1, 2, 5, 14]).unwrap().value, Value::Int(45_000_000));
    }

    #[test]
    fn a_lambert_conformal_grid_reads_as_its_projection() {
        let mut g = vec![0]; // source: from a template
        g.extend_from_slice(&12u32.to_be_bytes());
        g.extend_from_slice(&[0, 0]);
        g.extend_from_slice(&30u16.to_be_bytes()); // template 30
        g.extend_from_slice(&[6, 0]);
        g.extend_from_slice(&0u32.to_be_bytes());
        g.push(0);
        g.extend_from_slice(&0u32.to_be_bytes());
        g.push(0);
        g.extend_from_slice(&0u32.to_be_bytes());
        g.extend_from_slice(&4u32.to_be_bytes()); // nx
        g.extend_from_slice(&3u32.to_be_bytes()); // ny
        g.extend_from_slice(&30_000_000u32.to_be_bytes()); // first latitude
        g.extend_from_slice(&(0x8000_0000u32 | 100_000_000).to_be_bytes()); // 100 W
        g.push(0x30);
        g.extend_from_slice(&40_000_000u32.to_be_bytes()); // lad
        g.extend_from_slice(&(0x8000_0000u32 | 95_000_000).to_be_bytes()); // lov
        g.extend_from_slice(&12_000_000u32.to_be_bytes()); // dx, in millimetres
        g.extend_from_slice(&12_000_000u32.to_be_bytes());
        g.extend_from_slice(&[0, 64]);
        g.extend_from_slice(&33_000_000u32.to_be_bytes()); // latin1
        g.extend_from_slice(&45_000_000u32.to_be_bytes()); // latin2
        g.extend_from_slice(&(0x8000_0000u32 | 90_000_000).to_be_bytes()); // south pole
        g.extend_from_slice(&0u32.to_be_bytes());
        let d = Document::new(MemSource(one_message(&[(3, g)])));
        let mut ev = Evaluator::new(grib());
        let template = ev.node(&d, &[0, 1, 4, 0, 2, 5]).unwrap();
        assert_eq!(template.type_name, "LambertConformal");
        assert_eq!(ev.node(&d, &[0, 1, 4, 0, 2, 5, 7]).unwrap().value, Value::UInt(4));
        // A western longitude: the top bit, and the magnitude below it.
        assert_eq!(ev.node(&d, &[0, 1, 4, 0, 2, 5, 10]).unwrap().value, Value::Int(-100_000_000));
        assert_eq!(ev.node(&d, &[0, 1, 4, 0, 2, 5, 13]).unwrap().value, Value::Int(-95_000_000));
        // The two standard parallels the cone is true at.
        assert_eq!(ev.node(&d, &[0, 1, 4, 0, 2, 5, 18]).unwrap().value, Value::Int(33_000_000));
        assert_eq!(ev.node(&d, &[0, 1, 4, 0, 2, 5, 19]).unwrap().value, Value::Int(45_000_000));
    }

    #[test]
    fn the_product_says_which_parameter_and_how_far_ahead() {
        let d = Document::new(MemSource(message_bytes()));
        let mut ev = Evaluator::new(grib());
        let template = ev.node(&d, &[0, 1, 4, 2, 2, 1]).unwrap();
        assert_eq!(template.value, Value::Enum { raw: 0, name: Some("analysis or forecast at a level".into()), hex: false });
        let unit = ev.node(&d, &[0, 1, 4, 2, 2, 2, 7]).unwrap();
        assert_eq!(unit.value, Value::Enum { raw: 1, name: Some("hour".into()), hex: false });
        assert_eq!(ev.node(&d, &[0, 1, 4, 2, 2, 2, 8]).unwrap().value, Value::Int(6));
        // A second surface of all ones, which is what a field at one level
        // writes rather than leaving the fields out.
        let second = ev.node(&d, &[0, 1, 4, 2, 2, 2, 12]).unwrap();
        assert_eq!(second.value, Value::Unset(Box::new(Value::UInt(255))));
        // And the field is still the u8 it was: a sentinel says how one value
        // reads and nothing else.
        assert_eq!(second.type_name, "u8");
    }

    #[test]
    fn simple_packing_says_the_reference_value_and_the_width() {
        let d = Document::new(MemSource(message_bytes()));
        let mut ev = Evaluator::new(grib());
        assert_eq!(ev.node(&d, &[0, 1, 4, 3, 2, 0]).unwrap().value, Value::UInt(6));
        let kind = ev.node(&d, &[0, 1, 4, 3, 2, 1]).unwrap();
        assert_eq!(kind.value, Value::Enum { raw: 0, name: Some("simple packing".into()), hex: false });
        assert_eq!(ev.node(&d, &[0, 1, 4, 3, 2, 2, 0]).unwrap().value, Value::Float(270.0));
        // A scale factor of -1, written as the top bit and a one.
        assert_eq!(ev.node(&d, &[0, 1, 4, 3, 2, 2, 1]).unwrap().value, Value::Int(-1));
        assert_eq!(ev.node(&d, &[0, 1, 4, 3, 2, 2, 3]).unwrap().value, Value::UInt(8));
    }

    #[test]
    fn simply_packed_data_reads_as_the_values_section_5_described() {
        let d = Document::new(MemSource(message_bytes()));
        let mut ev = Evaluator::new(grib());
        // Six values of eight bits each, from a section 5 two sections back.
        let values = ev.node(&d, &[0, 1, 4, 5, 2, 5]).unwrap();
        assert_eq!((values.type_name.as_str(), values.child_count, values.size_bits), ("Packed[]", 6, 6 * 8));
        assert_eq!(ev.node(&d, &[0, 1, 4, 5, 2, 5, 0, 0]).unwrap().value, Value::UInt(1));
        assert_eq!(ev.node(&d, &[0, 1, 4, 5, 2, 5, 5, 0]).unwrap().value, Value::UInt(6));
    }

    /// Each value is the packed integer and what section 5 says it is worth:
    /// `(R + X * 2^E) / 10^D`, with a reference of 270 and a binary scale of
    /// -1, so a packed 1 is worth 270.5.
    #[test]
    fn a_simply_packed_value_is_worth_what_section_5_says() {
        let d = Document::new(MemSource(message_bytes()));
        let mut ev = Evaluator::new(grib());
        let first = ev.node(&d, &[0, 1, 4, 5, 2, 5, 0]).unwrap();
        assert_eq!(first.type_name, "Packed");
        let worth = ev.node(&d, &[0, 1, 4, 5, 2, 5, 0, 1]).unwrap();
        assert_eq!((worth.type_name.as_str(), worth.size_bits, worth.value), ("computed real", 0, Value::Float(270.5)));
        assert_eq!(ev.node(&d, &[0, 1, 4, 5, 2, 5, 5, 1]).unwrap().value, Value::Float(273.0));
        let rel = ev.relations(&d, &[0, 1, 4, 5, 2, 5, 5, 1]).unwrap();
        assert_eq!(rel[0].written, "(reference_value + stored * pow2(binary_scale_factor)) / pow10(decimal_scale_factor)");
        assert_eq!(rel[0].substituted, "(270 + 6 * pow2(-1)) / pow10(0)");
        assert_eq!(rel[0].result, "273");

        // A decimal scale of 2 and twelve bits a value, over a grid long
        // enough that walking it would show: the last value is still found
        // by arithmetic.
        let mut packing = packing_bytes();
        let n = packing.len();
        packing[n - 2] = 12;
        packing[0..4].copy_from_slice(&200_000u32.to_be_bytes());
        packing[12..14].copy_from_slice(&2u16.to_be_bytes());
        let data = vec![0xff; 300_000];
        let d = Document::new(MemSource(one_message(&[(5, packing), (7, data)])));
        let mut ev = Evaluator::new(grib());
        let last = ev.node(&d, &[0, 1, 4, 1, 2, 5, 199_999, 1]).unwrap();
        assert_eq!(last.value, Value::Float((270.0 + 4095.0 * 0.5) / 100.0));
        assert!(ev.memo_len() < 1000, "the grid was walked: {} nodes", ev.memo_len());
    }

    #[test]
    fn a_packed_value_is_as_wide_as_section_5_said() {
        // The same message packed at twelve bits: four values in six bytes,
        // and the second of them starts partway through a byte.
        let mut packing = packing_bytes();
        let n = packing.len();
        packing[n - 2] = 12; // bits per value
        packing[3] = 4; // four values
        let data = vec![0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc];
        let d = Document::new(MemSource(one_message(&[(5, packing), (7, data)])));
        let mut ev = Evaluator::new(grib());
        let values = ev.node(&d, &[0, 1, 4, 1, 2, 5]).unwrap();
        assert_eq!((values.child_count, values.size_bits), (4, 48));
        assert_eq!(ev.node(&d, &[0, 1, 4, 1, 2, 5, 0, 0]).unwrap().value, Value::UInt(0x123));
        assert_eq!(ev.node(&d, &[0, 1, 4, 1, 2, 5, 1, 0]).unwrap().value, Value::UInt(0x456));
        assert_eq!(ev.node(&d, &[0, 1, 4, 1, 2, 5, 1, 0]).unwrap().offset_bits % 8, 4);
        assert_eq!(ev.node(&d, &[0, 1, 4, 1, 2, 5, 3, 0]).unwrap().value, Value::UInt(0xabc));
    }

    #[test]
    fn a_packing_this_does_not_read_leaves_section_7_alone() {
        // Section 5 written as CCSDS 121.0, the Rice coding a spacecraft
        // downlink uses: its header reads, and the data stays bytes because
        // what follows is an entropy-coded stream and not a run of anything.
        let mut packing = packing_bytes();
        packing[5] = 42; // template 42 rather than 0
        packing.extend_from_slice(&[0x10, 32]); // flags, and a block of 32
        packing.extend_from_slice(&128u16.to_be_bytes()); // a reference every 128
        let d = Document::new(MemSource(one_message(&[(5, packing), (7, vec![1, 2, 3, 4, 5, 6])])));
        let mut ev = Evaluator::new(grib());
        let kind = ev.node(&d, &[0, 1, 4, 0, 2, 1]).unwrap();
        assert_eq!(kind.value, Value::Enum { raw: 42, name: Some("CCSDS".into()), hex: false });
        assert_eq!(ev.node(&d, &[0, 1, 4, 0, 2, 2]).unwrap().type_name, "CcsdsPacking");
        let data = ev.node(&d, &[0, 1, 4, 1, 2, 0]).unwrap();
        assert_eq!((data.type_name.as_str(), data.size_bits), ("bytes[]", 6 * 8));
    }

    #[test]
    fn a_section_written_to_a_template_this_does_not_know_stays_bytes() {
        // The same message with a grid template of 50, spherical harmonic
        // coefficients: named, and its fields left alone.
        let mut bytes = message_bytes();
        let at = bytes.windows(4).position(|w| w == [0, 0, 0, 6]).expect("the grid's point count");
        // The template number is two bytes after the count and the two
        // optional-list bytes.
        bytes[at + 6] = 0;
        bytes[at + 7] = 50;
        let d = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(grib());
        let number = ev.node(&d, &[0, 1, 4, 1, 2, 4]).unwrap();
        assert_eq!(number.value, Value::Enum { raw: 50, name: Some("spherical harmonic".into()), hex: false });
        let template = ev.node(&d, &[0, 1, 4, 1, 2, 5]).unwrap();
        assert_eq!(template.type_name, "bytes[]");
    }

    #[test]
    fn padding_between_messages_does_not_take_the_rest_of_the_file_with_it() {
        // A file put together with a newline after each message, which is what
        // a shell script concatenating byte ranges leaves behind.
        let one = message_bytes();
        let mut b = one.clone();
        b.push(b'\n');
        b.extend_from_slice(&one);
        b.extend_from_slice(b"\n\n");
        let d = Document::new(MemSource(b));
        let mut ev = Evaluator::new(grib());
        // Message, newline, message, newlines.
        assert_eq!(ev.node(&d, &[]).unwrap().child_count, 4);
        let filler = ev.node(&d, &[1]).unwrap();
        assert_eq!((filler.type_name.as_str(), filler.size_bits), ("BetweenMessages", 8));
        let second = ev.node(&d, &[2]).unwrap();
        assert_eq!(second.type_name, "Message");
        assert_eq!(second.offset_bits, (one.len() as u64 + 1) * 8);
        assert_eq!(ev.node(&d, &[3]).unwrap().size_bits, 2 * 8);
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

    /// Section 5 for a complex-packed message of two groups. Every number in
    /// it is picked so that the one it decides can be told apart from the
    /// others: the two group widths differ, the reference width is not zero,
    /// the length increment is not one, and the last group's length is not
    /// what the scaled form would have given.
    ///
    /// `spatial` writes it as template 5.3, first order, two octets of extra
    /// descriptors.
    fn complex_packing_bytes(spatial: bool) -> Vec<u8> {
        let mut b = 7u32.to_be_bytes().to_vec(); // seven values
        b.extend_from_slice(&(if spatial { 3u16 } else { 2u16 }).to_be_bytes());
        b.extend_from_slice(&10.0f32.to_be_bytes()); // reference value
        b.extend_from_slice(&0u16.to_be_bytes()); // binary scale
        b.extend_from_slice(&0u16.to_be_bytes()); // decimal scale
        b.extend_from_slice(&[6, 0]); // six bits a group reference
        b.extend_from_slice(&[1, 0]); // split by row, no missing values
        b.extend_from_slice(&0u32.to_be_bytes()); // primary missing value
        b.extend_from_slice(&0u32.to_be_bytes()); // secondary missing value
        b.extend_from_slice(&2u32.to_be_bytes()); // two groups
        b.extend_from_slice(&[1, 3]); // widths count up from 1, three bits each
        b.extend_from_slice(&2u32.to_be_bytes()); // lengths count up from 2
        b.push(2); // in steps of 2
        b.extend_from_slice(&3u32.to_be_bytes()); // and the last group holds 3
        b.push(4); // four bits a scaled length
        if spatial {
            b.extend_from_slice(&[1, 2]); // first order, two octets
        }
        b
    }

    /// Section 7 to match: two group references at six bits, padded to a byte;
    /// two widths at three bits, padded; two scaled lengths at four bits; then
    /// four values four bits wide and three values one bit wide.
    fn complex_data_bytes(spatial: bool) -> Vec<u8> {
        let mut b = Vec::new();
        if spatial {
            b.extend_from_slice(&100u16.to_be_bytes()); // the first value, kept whole
            b.extend_from_slice(&0x8003u16.to_be_bytes()); // and a minimum of -3
        }
        // 000101 101000 0000: references of 5 and 40, and four bits of padding.
        b.extend_from_slice(&[0x16, 0x80]);
        // 011 000 00: raw widths of 3 and 0, which with the reference are 4
        // and 1, and two bits of padding.
        b.push(0x60);
        // 0001 0000: a raw length of 1, which scaled is 2 + 2 * 1 = 4, and one
        // the reader must ignore, since the last group's length is in
        // section 5.
        b.push(0x10);
        // 0001 0010 0011 0100 101 00000: four values four bits wide and three
        // one bit wide, and five bits of padding to the end of the section.
        b.extend_from_slice(&[0x12, 0x34, 0xA0]);
        b
    }

    fn complex_message(spatial: bool) -> Vec<u8> {
        one_message(&[(5, complex_packing_bytes(spatial)), (7, complex_data_bytes(spatial))])
    }

    #[test]
    fn a_complex_packed_section_reads_as_its_three_tables_and_its_groups() {
        let d = Document::new(MemSource(complex_message(false)));
        let mut ev = Evaluator::new(grib());
        let body = ev.node(&d, &[0, 1, 4, 1, 2]).unwrap();
        assert_eq!(body.type_name, "ComplexPackedData");
        let ints = |ev: &mut Evaluator, at: [usize; 6], n: usize| -> Vec<i128> {
            (0..n)
                .map(|i| {
                    let mut p = at.to_vec();
                    p.push(i);
                    ev.node(&d, &p).unwrap().value.as_int().unwrap()
                })
                .collect()
        };
        assert_eq!(ints(&mut ev, [0, 1, 4, 1, 2, 8], 2), vec![5, 40]);
        assert_eq!(ints(&mut ev, [0, 1, 4, 1, 2, 10], 2), vec![3, 0]);
        assert_eq!(ints(&mut ev, [0, 1, 4, 1, 2, 12], 2), vec![1, 0]);
        // The padding between the tables: four bits after twelve, two after
        // six, none after eight.
        let pad = |ev: &mut Evaluator, i: usize| ev.node(&d, &[0, 1, 4, 1, 2, i]).unwrap().size_bits;
        assert_eq!((pad(&mut ev, 9), pad(&mut ev, 11), pad(&mut ev, 13)), (4, 2, 0));
    }

    #[test]
    fn a_group_is_as_wide_and_as_long_as_the_tables_in_front_of_it_say() {
        let d = Document::new(MemSource(complex_message(false)));
        let mut ev = Evaluator::new(grib());
        let groups = ev.node(&d, &[0, 1, 4, 1, 2, 14]).unwrap();
        assert_eq!(groups.child_count, 2);
        // Group 0: a raw width of 3 counted up from the reference of 1, and a
        // raw length of 1 scaled by the increment of 2 from the reference
        // of 2.
        let width = ev.node(&d, &[0, 1, 4, 1, 2, 14, 0, 0]).unwrap().value.as_int().unwrap();
        let count = ev.node(&d, &[0, 1, 4, 1, 2, 14, 0, 1]).unwrap().value.as_int().unwrap();
        assert_eq!((width, count), (4, 4));
        let values: Vec<i128> = (0..4)
            .map(|i| ev.node(&d, &[0, 1, 4, 1, 2, 14, 0, 2, i]).unwrap().value.as_int().unwrap())
            .collect();
        assert_eq!(values, vec![1, 2, 3, 4]);
        // Group 1 is the last, so its length is section 5's `last_group_length`
        // and not the entry in the table, which says something else.
        let width = ev.node(&d, &[0, 1, 4, 1, 2, 14, 1, 0]).unwrap().value.as_int().unwrap();
        let count = ev.node(&d, &[0, 1, 4, 1, 2, 14, 1, 1]).unwrap().value.as_int().unwrap();
        assert_eq!((width, count), (1, 3));
        let values: Vec<i128> = (0..3)
            .map(|i| ev.node(&d, &[0, 1, 4, 1, 2, 14, 1, 2, i]).unwrap().value.as_int().unwrap())
            .collect();
        assert_eq!(values, vec![1, 0, 1]);
        // Seven values in all, which is what section 5 counted.
        assert_eq!(ev.node(&d, &[0, 1, 4, 0, 2, 0]).unwrap().value.as_int().unwrap(), 7);
    }

    #[test]
    fn spatial_differencing_puts_its_first_value_and_its_minimum_in_front() {
        let d = Document::new(MemSource(complex_message(true)));
        let mut ev = Evaluator::new(grib());
        let body = ev.node(&d, &[0, 1, 4, 1, 2]).unwrap();
        assert_eq!(body.type_name, "ComplexPackedData");
        let first = ev.node(&d, &[0, 1, 4, 1, 2, 10]).unwrap();
        assert_eq!((first.child_count, first.size_bits), (1, 16));
        assert_eq!(ev.node(&d, &[0, 1, 4, 1, 2, 10, 0]).unwrap().value.as_int().unwrap(), 100);
        // The one negative number in the section, written as a magnitude with
        // the top bit set rather than as two's complement.
        let minimum = ev.node(&d, &[0, 1, 4, 1, 2, 11]).unwrap();
        assert_eq!(minimum.value.as_int().unwrap(), -3);
        assert_eq!(minimum.size_bits, 16);
        // And everything after them is where it was without them.
        let width = ev.node(&d, &[0, 1, 4, 1, 2, 18, 0, 0]).unwrap().value.as_int().unwrap();
        let count = ev.node(&d, &[0, 1, 4, 1, 2, 18, 0, 1]).unwrap().value.as_int().unwrap();
        assert_eq!((width, count), (4, 4));
    }
}
