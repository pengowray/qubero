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
//! values when they are simply packed. A section written to a template this
//! does not know keeps its length and its number and its contents stay bytes,
//! which is the honest answer: the templates are a WMO table of several
//! hundred and each one is a different set of fields.
//!
//! Section 7 reads as its numbers only for simple packing. The complex
//! packings cut the grid into groups whose widths are themselves packed at the
//! front of the section, and the image packings hold a whole JPEG 2000 or PNG
//! codestream; neither is a run of values with a stride. See [`data`] for what
//! a packed value is worth.
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

/// A signed angle or scale factor, whose top bit says which way it goes rather
/// than being part of the number. See [`T::sign_magnitude`].
fn sm32() -> T {
    T::sign_magnitude(32, Big)
}

fn sm16() -> T {
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
/// front of section 7. That is a layout no template can describe: how long the
/// three runs are depends on numbers inside them.
///
/// What reads is the header, which says how the groups were chosen and how
/// wide the three tables are. Section 7 stays bytes for these.
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
/// run of numbers, so it stays bytes.
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
/// `binary_scale_factor` and D is `decimal_scale_factor`. That is not written
/// as a computed field, and cannot be: R is a float, E and D are negative as
/// often as not, and the IR's arithmetic is over integers with no power of
/// ten in it.
///
/// Any other packing stays bytes. A complex-packed section is three runs whose
/// lengths are inside themselves, and a JPEG 2000 or PNG one is a whole
/// codestream of another format; neither is a run of numbers a template can
/// place.
fn data() -> T {
    T::switch(
        // Which packing, from the nearest earlier section that says: section
        // 5. Nothing earlier saying anything answers 0, which is also simple
        // packing, and the counts then come to nothing as well, so a section 7
        // with no section 5 in front of it reads as no values rather than as
        // a wrong number of them.
        E::sibling(&["body", "template_number"]),
        vec![(0, simple_packed_data())],
        T::structure("PackedData", vec![("values", T::bytes(E::Remaining))]),
    )
}

fn simple_packed_data() -> T {
    T::structure(
        "PackedData",
        vec![
            ("bits_per_value", T::computed(E::sibling(&["body", "template", "bits_per_value"]))),
            ("count", T::computed(E::sibling(&["body", "number_of_values"]))),
            ("values", T::array(T::uint_expr(E::field("bits_per_value"), Big), E::field("count"))),
        ],
    )
    .machinery(&["bits_per_value", "count"])
    .payload(&["values"])
}

/// An edition 1 message. Its sections have three-byte lengths and no numbers
/// in them: which section is which is settled by the order they come in, and
/// two of the five are there only if a flag in the first one says so. So they
/// are declared as five fields rather than as a run, and the two optional ones
/// ask that flag.
fn edition1() -> T {
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
        let values = ev.node(&d, &[0, 1, 4, 5, 2, 2]).unwrap();
        assert_eq!((values.child_count, values.size_bits), (6, 6 * 8));
        assert_eq!(ev.node(&d, &[0, 1, 4, 5, 2, 2, 0]).unwrap().value, Value::UInt(1));
        assert_eq!(ev.node(&d, &[0, 1, 4, 5, 2, 2, 5]).unwrap().value, Value::UInt(6));
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
        let values = ev.node(&d, &[0, 1, 4, 1, 2, 2]).unwrap();
        assert_eq!((values.child_count, values.size_bits), (4, 48));
        assert_eq!(ev.node(&d, &[0, 1, 4, 1, 2, 2, 0]).unwrap().value, Value::UInt(0x123));
        assert_eq!(ev.node(&d, &[0, 1, 4, 1, 2, 2, 1]).unwrap().value, Value::UInt(0x456));
        assert_eq!(ev.node(&d, &[0, 1, 4, 1, 2, 2, 3]).unwrap().value, Value::UInt(0xabc));
    }

    #[test]
    fn a_packing_this_does_not_read_leaves_section_7_alone() {
        // Section 5 written as complex packing: its header reads, and the
        // data stays bytes because the widths are inside the data.
        let mut packing = packing_bytes();
        packing[5] = 2; // template 2 rather than 0
        packing.extend_from_slice(&[0, 0]); // splitting method, missing management
        packing.extend_from_slice(&0u32.to_be_bytes());
        packing.extend_from_slice(&0u32.to_be_bytes());
        packing.extend_from_slice(&3u32.to_be_bytes()); // three groups
        packing.extend_from_slice(&[0, 8]);
        packing.extend_from_slice(&0u32.to_be_bytes());
        packing.push(1);
        packing.extend_from_slice(&2u32.to_be_bytes());
        packing.push(8);
        let d = Document::new(MemSource(one_message(&[(5, packing), (7, vec![1, 2, 3, 4, 5, 6])])));
        let mut ev = Evaluator::new(grib());
        let kind = ev.node(&d, &[0, 1, 4, 0, 2, 1]).unwrap();
        assert_eq!(kind.value, Value::Enum { raw: 2, name: Some("complex packing".into()), hex: false });
        // Three groups, from the header this does read.
        assert_eq!(ev.node(&d, &[0, 1, 4, 0, 2, 2, 9]).unwrap().value, Value::UInt(3));
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
}
