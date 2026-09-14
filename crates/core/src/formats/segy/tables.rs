//! Names for the codes a SEG-Y binary header and trace header hold: the
//! sample format, the sorting, what a trace is a recording of, units, and the
//! rest.
//!
//! Apart from [`segy`](super) because they are lists to look a number up in,
//! not a layout to follow; the template names each one where it reads the
//! field.

/// How each data sample is written, by the code in the binary header.
pub(super) const FORMAT: &[(i128, &str)] = &[
    (1, "4-byte IBM float"),
    (2, "4-byte signed integer"),
    (3, "2-byte signed integer"),
    (4, "4-byte fixed point with gain (obsolete)"),
    (5, "4-byte IEEE float"),
    (6, "8-byte IEEE float"),
    (7, "3-byte signed integer"),
    (8, "1-byte signed integer"),
    (9, "8-byte signed integer"),
    (10, "4-byte unsigned integer"),
    (11, "2-byte unsigned integer"),
    (12, "8-byte unsigned integer"),
    (15, "3-byte unsigned integer"),
    (16, "1-byte unsigned integer"),
];

/// What kind of ensemble the traces are gathered into.
pub(super) const SORTING: &[(i128, &str)] = &[
    (-1, "other"),
    (0, "unknown"),
    (1, "as recorded"),
    (2, "CDP ensemble"),
    (3, "single fold continuous profile"),
    (4, "horizontally stacked"),
    (5, "common source point"),
    (6, "common receiver point"),
    (7, "common offset point"),
    (8, "common mid-point"),
    (9, "common conversion point"),
];

pub(super) const SWEEP_TYPE: &[(i128, &str)] = &[(1, "linear"), (2, "parabolic"), (3, "exponential"), (4, "other")];

pub(super) const TAPER_TYPE: &[(i128, &str)] = &[(1, "linear"), (2, "cosine squared"), (3, "other")];

/// Correlated or not, which the standard writes as 1 for no and 2 for yes.
pub(super) const NO_YES: &[(i128, &str)] = &[(1, "no"), (2, "yes")];

/// And binary gain recovered, which it writes the other way round.
pub(super) const YES_NO: &[(i128, &str)] = &[(1, "yes"), (2, "no")];

pub(super) const AMPLITUDE_RECOVERY: &[(i128, &str)] = &[(1, "none"), (2, "spherical divergence"), (3, "AGC"), (4, "other")];

pub(super) const MEASUREMENT_SYSTEM: &[(i128, &str)] = &[(1, "metres"), (2, "feet")];

/// Which sign a rise in pressure, or the geophone case moving up, is written
/// with.
pub(super) const IMPULSE_POLARITY: &[(i128, &str)] = &[(1, "pressure rise or upward motion stored as negative"), (2, "pressure rise or upward motion stored as positive")];

/// How far the seismic signal lags the pilot signal of a vibrator, in
/// 45-degree sectors.
pub(super) const VIBRATORY_POLARITY: &[(i128, &str)] = &[
    (1, "lags pilot by 337.5° to 22.5°"),
    (2, "lags pilot by 22.5° to 67.5°"),
    (3, "lags pilot by 67.5° to 112.5°"),
    (4, "lags pilot by 112.5° to 157.5°"),
    (5, "lags pilot by 157.5° to 202.5°"),
    (6, "lags pilot by 202.5° to 247.5°"),
    (7, "lags pilot by 247.5° to 292.5°"),
    (8, "lags pilot by 292.5° to 337.5°"),
];

pub(super) const FIXED_LENGTH: &[(i128, &str)] = &[(0, "trace length may vary"), (1, "all traces same length")];

/// What the times in a trace header are measured against. GPS is revision
/// 2's addition.
pub(super) const TIME_BASIS: &[(i128, &str)] = &[(1, "local"), (2, "GMT"), (3, "other"), (4, "UTC"), (5, "GPS")];

/// What a trace is a recording of. Values from 22 up are for whoever writes
/// the file to assign.
pub(super) const TRACE_ID: &[(i128, &str)] = &[
    (-1, "other"),
    (0, "unknown"),
    (1, "seismic data"),
    (2, "dead"),
    (3, "dummy"),
    (4, "time break"),
    (5, "uphole"),
    (6, "sweep"),
    (7, "timing"),
    (8, "water break"),
    (9, "near-field gun signature"),
    (10, "far-field gun signature"),
    (11, "seismic pressure sensor"),
    (12, "multicomponent sensor, vertical"),
    (13, "multicomponent sensor, cross-line"),
    (14, "multicomponent sensor, in-line"),
    (15, "rotated multicomponent sensor, vertical"),
    (16, "rotated multicomponent sensor, transverse"),
    (17, "rotated multicomponent sensor, radial"),
    (18, "vibrator reaction mass"),
    (19, "vibrator baseplate"),
    (20, "vibrator estimated ground force"),
    (21, "vibrator reference"),
];

pub(super) const DATA_USE: &[(i128, &str)] = &[(1, "production"), (2, "test")];

pub(super) const COORDINATE_UNITS: &[(i128, &str)] =
    &[(1, "length (metres or feet)"), (2, "seconds of arc"), (3, "decimal degrees"), (4, "degrees, minutes, seconds")];

pub(super) const GAIN_TYPE: &[(i128, &str)] = &[(1, "fixed"), (2, "binary"), (3, "floating point")];

pub(super) const OVER_TRAVEL: &[(i128, &str)] = &[(1, "down or behind"), (2, "up or ahead")];

/// The unit a trace's samples are in, and the unit they are in once the
/// transduction constant has been applied.
pub(super) const UNIT: &[(i128, &str)] = &[
    (-1, "other"),
    (0, "unknown"),
    (1, "Pa"),
    (2, "V"),
    (3, "mV"),
    (4, "A"),
    (5, "m"),
    (6, "m/s"),
    (7, "m/s²"),
    (8, "N"),
    (9, "W"),
];
