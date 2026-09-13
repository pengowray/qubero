//! NIfTI, the file a brain scan is kept in: MRI and fMRI volumes, and the maps
//! of statistics worked out from them. FSL, SPM, AFNI and FreeSurfer all read
//! and write it, and so does every Python pipeline, through nibabel.
//!
//! A file is a fixed header, a run of extensions, and the voxels. The NIfTI-1
//! header is 348 bytes and ends in its magic: `n+1` for a `.nii` that carries
//! its own voxels, and `ni1` for a `.hdr` whose voxels are in the `.img` file
//! beside it. The header is laid out as the NIfTI Data Format Working Group's
//! `nifti1.h` lays it out, and every field is called what that file calls it.
//!
//! NIfTI-2 is the same idea for volumes too big for a 16-bit dimension: 540
//! bytes, with the dimensions and `vox_offset` at 64 bits and every float a
//! double. It is not NIfTI-1 at double width. The fields are reordered, the
//! magic moves to the front as `n+2` or `ni2`, and the four bytes after the
//! magic are `\r\n\032\n`, PNG's trick for catching a transfer that rewrote
//! line endings. Analyze's leftovers are gone. So the two layouts are written
//! out separately, from `nifti2.h` for the second, and share everything after
//! the header.
//!
//! NIfTI-1 kept the size and much of the shape of the Analyze 7.5 header it
//! grew from, which SPM wrote for a decade before it and which is still
//! around as `.hdr` and `.img` pairs. That header has no magic, so a 348-byte
//! header without one reads as Analyze here, and the `analyze` template reads
//! any 348-byte header that way. [`analyze_header`] lists what differs.
//!
//! Nothing says which way round the numbers are. `sizeof_hdr` is 348 or 540
//! in the file's own order, and those read the wrong way round are
//! 1,543,569,408 and 469,893,120, so the template peeks at that word both ways
//! and lays the file out in whichever answers, as `sac` does with `nvhdr` and
//! as every NIfTI reader does. The magic cannot say it, since it is text.
//!
//! `vox_offset` says where the voxels start, and in NIfTI-1 it is a float,
//! because Analyze had a float there before it. An expression here is an
//! integer, so the zero-bit field in front of it, `data_offset`, reads the
//! float's bits as the whole number of bytes they hold, and that is what
//! places the extensions and the voxels. A fraction is dropped, which is what
//! nibabel does with one.
//!
//! After the header come four bytes whose first byte says whether any
//! extensions follow. Each one is `esize`, `ecode`, and `esize - 8` bytes of
//! whatever the code says, padded so that `esize` is a multiple of 16. In a
//! `.nii` they run to `vox_offset`, and in a `.hdr` to the end of the file,
//! since nothing else is in it. A comment, and the extensions that hold XML
//! (AFNI's, XCEDE's, Caret's, CIFTI-2's) or JSON (MRS), read as text; the
//! rest keep their bytes.
//!
//! The voxels are an array of whatever `datatype` names, grouped by `dim`:
//! `dim[0]` says how many dimensions there are and `dim[1]` onwards how long
//! each one is. The first index varies fastest, which is Fortran's order and
//! not C's, so the innermost run is along `dim[1]`. A 4-D series of 64 by 64
//! by 30 over 200 volumes reads as 200 volumes of 30 slices of 64 rows of 64
//! voxels, and `voxels[t][z][y][x]` is the voxel nibabel calls
//! `data[x, y, z, t]`. `bitpix` says the width again, and where the two
//! disagree `datatype` is believed, as nibabel believes it.
//!
//! `scl_slope` and `scl_inter` say what a stored number is worth: `scl_slope *
//! stored + scl_inter`, where a slope of 0 says the numbers are worth what
//! they say. When both are whole numbers and the voxels are integers, that sum
//! is worked out and each voxel reads as the integer on disk with what it is
//! worth beside it, the arrangement the FITS template has for `TSCALn`. The
//! two whole numbers are `scale` and `zero`, zero-bit fields in front of the
//! floats they read, and both hold nothing when there is nothing to work out.
//!
//! `dim_info` and `xyzt_units` each pack small numbers into one byte, and are
//! read as the bits they are. `dim_info` says which axis the frequency
//! encoding, the phase encoding and the slices ran along, two bits each.
//! `xyzt_units` says in three bits what `pixdim[1..3]` measure and in the
//! three above them what `pixdim[4]` measures. `nifti1.h` writes the time
//! units as the whole byte, 8 for seconds and 16 for milliseconds, and those
//! are the same bits: the field here holds 1 and 2.
//!
//! What is not read here:
//!
//! - A slope or an intercept with a fraction in it, which is what most
//!   scanners write: an expression here is an integer, so the sum cannot be
//!   made, and the voxels read as stored with the two floats in the header for
//!   the reader to apply. Float voxels are never scaled, for the same reason.
//! - 128-bit floats and the complex numbers made of them, which have no type
//!   here and keep their bytes, 16 and 32 to a voxel.
//! - `DT_BINARY`, a bit a voxel, which `nifti1.h` defines and never says the
//!   bit order of. nibabel does not read it either. The voxels stay bytes.
//! - The voxels of a `.hdr`, which are in the `.img` beside it and are not
//!   opened from here.
//! - A `vox_offset` below the end of the header in a `.nii`, which is
//!   invalid: the voxels are placed straight after the four extension bytes,
//!   where a writer that left it at 0 put them.
//! - Anything past the voxels the dimensions account for, which is left
//!   uncovered.
//!
//! A `.nii.gz` is a gzip file and reads as one, and the stream inside it opens
//! as NIfTI in a space of its own, as an NPY inside an NPZ does. Checked by
//! `tests/nifti_real.rs`.

use crate::template::{Encoding, Endian, Endian::*, Expr as E, StrLen, Template, Ty as T, Until};

/// How long a NIfTI-1 header is.
pub const HEADER_1: i128 = 348;

/// Where NIfTI-1 writes its magic: the last four bytes of the header.
pub const MAGIC_1_AT: usize = 344;

/// How long a NIfTI-2 header is.
pub const HEADER_2: i128 = 540;

/// Where NIfTI-2 writes its magic: straight after `sizeof_hdr`.
pub const MAGIC_2_AT: usize = 4;

/// The magics, as the big-endian numbers their four bytes read as: `n+1\0`
/// for a file that holds its voxels and `ni1\0` for one that does not, and
/// `n+2\0` and `ni2\0` the same for NIfTI-2. The four bytes NIfTI-2 writes
/// after its magic are not part of the test, since they are the ones a
/// transfer that mangled line endings would have changed, and a file like
/// that is still worth reading to see it.
const SINGLE_1: i128 = 0x6e2b_3100;
const PAIR_1: i128 = 0x6e69_3100;
const SINGLE_2: i128 = 0x6e2b_3200;
const PAIR_2: i128 = 0x6e69_3200;

/// The layout of an IEEE float: how many bits hold the fraction and how many
/// the exponent, the sign being the one bit above both.
#[derive(Clone, Copy)]
struct Float {
    fraction: u32,
    exponent: u32,
}

const SINGLE: Float = Float { fraction: 23, exponent: 8 };
const DOUBLE: Float = Float { fraction: 52, exponent: 11 };

/// A float read here as a whole number is one under two to the forty, and
/// anything bigger is not read as one. No offset or scale factor in a real
/// file comes near it, and it keeps a stored 64-bit voxel multiplied by a
/// scale well inside the 128 bits an expression holds.
const WIDEST: i128 = 40;

impl Float {
    fn bias(self) -> i128 {
        (1 << (self.exponent - 1)) - 1
    }

    /// The exponent at which every bit of the fraction is above the point, so
    /// that the significand is the whole number with no shifting.
    fn top(self) -> i128 {
        self.bias() + self.fraction as i128
    }

    fn exp(self, bits: &E) -> E {
        bits.clone().shr(E::lit(self.fraction)).and(E::lit((1i128 << self.exponent) - 1))
    }

    /// The fraction with the one in front of it that the format leaves out.
    fn significand(self, bits: &E) -> E {
        bits.clone().and(E::lit((1i128 << self.fraction) - 1)).add(E::lit(1i128 << self.fraction))
    }

    fn negative(self, bits: &E) -> E {
        bits.clone().shr(E::lit(self.fraction + self.exponent))
    }

    /// Nought, of either sign: every bit below the sign clear.
    fn is_zero(self, bits: &E) -> E {
        bits.clone().and(E::lit((1i128 << (self.fraction + self.exponent)) - 1)).equal_to(E::lit(0))
    }

    /// One or more, and under two to the [`WIDEST`].
    fn in_range(self, bits: &E) -> E {
        let exp = self.exp(bits);
        exp.clone().greater_or_equal(E::lit(self.bias())).both(exp.less_than(E::lit(self.bias() + WIDEST)))
    }

    /// How big the number is, with any fraction dropped. Only asked in range.
    fn magnitude(self, bits: &E) -> E {
        let (exp, top) = (self.exp(bits), E::lit(self.top()));
        E::cond(
            exp.clone().less_or_equal(top.clone()),
            self.significand(bits).shr(top.clone().sub(exp.clone())),
            self.significand(bits).shl(exp.sub(top)),
        )
    }

    /// One when the float holds a whole number this reads, and zero for a
    /// fraction, a number too big, an infinity or a NaN.
    fn whole(self, bits: &E) -> E {
        let (exp, top) = (self.exp(bits), E::lit(self.top()));
        let below_point = E::lit(1).shl(top.clone().sub(exp.clone())).sub(E::lit(1));
        let no_fraction = exp.greater_or_equal(top).either(self.significand(bits).and(below_point).equal_to(E::lit(0)));
        self.is_zero(bits).either(self.in_range(bits).both(no_fraction))
    }

    /// The whole number, signed. Nought for anything out of range, which is
    /// only asked after [`Float::whole`] has said it is not.
    fn value(self, bits: &E) -> E {
        let magnitude = self.magnitude(bits);
        let signed = E::cond(self.negative(bits), E::lit(0).sub(magnitude.clone()), magnitude);
        E::cond(self.in_range(bits), signed, E::lit(0))
    }

    /// A byte offset: the whole part of a positive number, and nought for
    /// anything that cannot be one.
    fn offset(self, bits: &E) -> E {
        E::cond(self.in_range(bits).both(self.negative(bits).negate()), self.magnitude(bits), E::lit(0))
    }
}

/// Whether the voxels are scaled here: both floats whole, a slope that is not
/// 0, and not the pair 1 and 0, which changes nothing. `slope` and `inter`
/// are the floats' bits.
fn scaled(f: Float, slope: &E, inter: &E) -> E {
    let identity = f.value(slope).equal_to(E::lit(1)).both(f.value(inter).equal_to(E::lit(0)));
    f.whole(slope).both(f.is_zero(slope).negate()).both(f.whole(inter)).both(identity.negate())
}

/// `scale` or `zero`: the whole number a scaling float holds, when the voxels
/// are scaled here, and a field of nothing when they are not. Nothing rather
/// than 0, since 0 beside a voxel's worth would read as a slope nobody wrote;
/// the floats themselves are still in the rows after it. An expression that
/// reads a field of no bytes reads 0, which is how the voxels ask.
fn scaling(when: E, value: E) -> T {
    T::switch(when, vec![(1, T::computed(value))], T::bytes(E::lit(0)))
}

/// What `datatype` names, by the number `nifti1.h` gives it. Analyze 7.5 used
/// the same numbers for the types it had, which are the ones up to 255.
const VOXEL_TYPES: &[(i128, &str)] = &[
    (0, "unknown"),
    (1, "binary: 1 bit per voxel"),
    (2, "uint8"),
    (4, "int16"),
    (8, "int32"),
    (16, "float32"),
    (32, "complex64: 2 x float32"),
    (64, "float64"),
    (128, "rgb24: 3 x uint8"),
    (255, "DT_ALL: not a type"),
    (256, "int8"),
    (512, "uint16"),
    (768, "uint32"),
    (1024, "int64"),
    (1280, "uint64"),
    (1536, "float128"),
    (1792, "complex128: 2 x float64"),
    (2048, "complex256: 2 x float128"),
    (2304, "rgba32: 4 x uint8"),
];

/// What the voxels mean, and so what `intent_p1` to `intent_p3` are the
/// parameters of. The codes from 2006 up are FSL's own, and are in files FSL
/// writes rather than in `nifti1.h`.
const INTENTS: &[(i128, &str)] = &[
    (0, "NONE"),
    (2, "CORREL: correlation coefficient"),
    (3, "TTEST: Student t statistic"),
    (4, "FTEST: F statistic"),
    (5, "ZSCORE: standard normal z"),
    (6, "CHISQ: chi-squared statistic"),
    (7, "BETA: beta distribution"),
    (8, "BINOM: binomial distribution"),
    (9, "GAMMA: gamma distribution"),
    (10, "POISSON: Poisson distribution"),
    (11, "NORMAL: normal distribution"),
    (12, "FTEST_NONC: noncentral F statistic"),
    (13, "CHISQ_NONC: noncentral chi-squared"),
    (14, "LOGISTIC: logistic distribution"),
    (15, "LAPLACE: Laplace distribution"),
    (16, "UNIFORM: uniform distribution"),
    (17, "TTEST_NONC: noncentral t statistic"),
    (18, "WEIBULL: Weibull distribution"),
    (19, "CHI: chi distribution"),
    (20, "INVGAUSS: inverse Gaussian"),
    (21, "EXTVAL: extreme value type I"),
    (22, "PVAL: p-value"),
    (23, "LOGPVAL: ln(p-value)"),
    (24, "LOG10PVAL: log10(p-value)"),
    (1001, "ESTIMATE: parameter estimate"),
    (1002, "LABEL: label index"),
    (1003, "NEURONAME: NeuroNames label index"),
    (1004, "GENMATRIX: M x N matrix per voxel"),
    (1005, "SYMMATRIX: symmetric matrix per voxel"),
    (1006, "DISPVECT: displacement vector per voxel"),
    (1007, "VECTOR: vector per voxel"),
    (1008, "POINTSET: spatial coordinates"),
    (1009, "TRIANGLE: vertex index triples"),
    (1010, "QUATERNION: quaternion per voxel"),
    (1011, "DIMLESS: dimensionless value"),
    (2001, "TIME_SERIES: time series"),
    (2002, "NODE_INDEX: surface node index"),
    (2003, "RGB_VECTOR: RGB triple"),
    (2004, "RGBA_VECTOR: RGBA quadruple"),
    (2005, "SHAPE: shape value, e.g. curvature"),
    (2006, "FSL_FNIRT_DISPLACEMENT_FIELD"),
    (2007, "FSL_CUBIC_SPLINE_COEFFICIENTS"),
    (2008, "FSL_DCT_COEFFICIENTS"),
    (2009, "FSL_QUADRATIC_SPLINE_COEFFICIENTS"),
    (2016, "FSL_TOPUP_CUBIC_SPLINE_COEFFICIENTS"),
    (2017, "FSL_TOPUP_QUADRATIC_SPLINE_COEFFICIENTS"),
    (2018, "FSL_TOPUP_FIELD"),
];

/// Which space `qform_code` and `sform_code` say their transform lands in.
/// One table for both, so 0 says what it means on either row.
const XFORMS: &[(i128, &str)] = &[
    (0, "UNKNOWN: transform not used"),
    (1, "SCANNER_ANAT: scanner coordinates"),
    (2, "ALIGNED_ANAT: aligned to another file"),
    (3, "TALAIRACH: Talairach-Tournoux space"),
    (4, "MNI_152: MNI 152 space"),
    (5, "TEMPLATE_OTHER: other template space"),
];

/// The order the slices were taken in, between `slice_start` and `slice_end`.
const SLICE_ORDERS: &[(i128, &str)] = &[
    (0, "UNKNOWN"),
    (1, "SEQ_INC: sequential, ascending"),
    (2, "SEQ_DEC: sequential, descending"),
    (3, "ALT_INC: interleaved from slice_start"),
    (4, "ALT_DEC: interleaved from slice_end"),
    (5, "ALT_INC2: interleaved from slice_start+1"),
    (6, "ALT_DEC2: interleaved from slice_end-1"),
];

/// Which axis of `dim` one of `dim_info`'s pairs of bits names.
const AXES: &[(i128, &str)] = &[(0, "not set"), (1, "dim[1] (i)"), (2, "dim[2] (j)"), (3, "dim[3] (k)")];

/// What `pixdim[1..3]` measure: the low three bits of `xyzt_units`.
const SPACE_UNITS: &[(i128, &str)] =
    &[(0, "UNKNOWN"), (1, "METER: metres"), (2, "MM: millimetres"), (3, "MICRON: micrometres")];

/// What `pixdim[4]` measures: the three bits above those. `nifti1.h` writes
/// these constants as the whole byte, so its 8 for seconds is the 1 here.
const TIME_UNITS: &[(i128, &str)] = &[
    (0, "UNKNOWN"),
    (1, "SEC: seconds"),
    (2, "MSEC: milliseconds"),
    (3, "USEC: microseconds"),
    (4, "HZ: hertz"),
    (5, "PPM: parts per million"),
    (6, "RADS: radians per second"),
];

/// Who an extension belongs to, by the code registered for it.
const ECODES: &[(i128, &str)] = &[
    (0, "IGNORE"),
    (2, "DICOM: raw DICOM attributes"),
    (4, "AFNI: AFNI attributes (XML)"),
    (6, "COMMENT: plain ASCII text"),
    (8, "XCEDE: XCEDE metadata (XML)"),
    (10, "JIMDIMINFO: Jim dimension info"),
    (12, "WORKFLOW_FWDS: Fiswidgets workflow"),
    (14, "FREESURFER"),
    (16, "PYPICKLE: pickled Python objects"),
    (18, "MIND_IDENT: LONI MiND identifier"),
    (20, "B_VALUE: diffusion b-value"),
    (22, "SPHERICAL_DIRECTION: gradient direction"),
    (24, "DT_COMPONENT: diffusion tensor component"),
    (26, "SHC_DEGREEORDER: spherical harmonic degree, order"),
    (28, "VOXBO"),
    (30, "CARET"),
    (32, "CIFTI: CIFTI-2 header (XML)"),
    (34, "VARIABLE_FRAME_TIMING: per-frame timing"),
    (38, "EVAL"),
    (40, "MATLAB"),
    (42, "QUANTIPHYSE"),
    (44, "MRS: MR spectroscopy header (JSON)"),
];

/// The extensions whose data is text: a comment, XML from AFNI, XCEDE, Caret
/// and CIFTI-2, and the JSON of an MRS header.
const TEXT_ECODES: &[i128] = &[4, 6, 8, 30, 32, 44];

/// A NIfTI file of either version, in either byte order, or an Analyze 7.5
/// header when the magic that would make it NIfTI-1 is not there.
pub fn nifti() -> Template {
    let by_size = |e: Endian, otherwise: T| {
        T::switch(E::peek(32, e), vec![(HEADER_1, version(e, Version::One)), (HEADER_2, version(e, Version::Two))], otherwise)
    };
    let root = T::switch(
        E::Remaining.less_than(E::lit(4)),
        vec![(1, T::bytes(E::Remaining))],
        by_size(Little, by_size(Big, T::bytes(E::Remaining))),
    );
    Template::new("nifti", root)
}

/// Which of the two layouts a header is in.
#[derive(Clone, Copy, PartialEq)]
enum Version {
    One,
    Two,
}

/// A header of either size, told apart by its magic. A file too short to hold
/// the header keeps its bytes.
fn version(e: Endian, v: Version) -> T {
    let (size, magic, single, pair) = match v {
        Version::One => (HEADER_1, E::peek_at(E::lit(MAGIC_1_AT as i128 * 8), 32, Big), SINGLE_1, PAIR_1),
        Version::Two => (HEADER_2, E::peek_at(E::lit(MAGIC_2_AT as i128 * 8), 32, Big), SINGLE_2, PAIR_2),
    };
    // A 348-byte header with no magic is the Analyze header NIfTI-1 grew
    // from, and reads as that. A 540-byte one with no magic is nothing known.
    let otherwise = match v {
        Version::One => analyze_file(e),
        Version::Two => T::bytes(E::Remaining),
    };
    T::switch(
        E::Remaining.less_than(E::lit(size)),
        vec![(1, T::bytes(E::Remaining))],
        T::switch(magic, vec![(single, file(e, v, true)), (pair, file(e, v, false))], otherwise),
    )
}

/// An Analyze 7.5 header, in either byte order, whatever its last four bytes
/// hold: the template to pick for a `.hdr` from before NIfTI, or to read a
/// NIfTI-1 header the way a program that predates it would.
pub fn analyze() -> Template {
    let by_size = |e: Endian, otherwise: T| T::switch(E::peek(32, e), vec![(HEADER_1, analyze_file(e))], otherwise);
    let root = T::switch(
        E::Remaining.less_than(E::lit(HEADER_1)),
        vec![(1, T::bytes(E::Remaining))],
        by_size(Little, by_size(Big, T::bytes(E::Remaining))),
    );
    Template::new("analyze", root)
}

/// An Analyze file is its header and nothing else: the voxels are always in
/// the `.img` beside it, starting `vox_offset` bytes in.
fn analyze_file(e: Endian) -> T {
    T::structure("Analyze 7.5", vec![("header", analyze_header(e))])
}

/// The 348 bytes of Analyze 7.5's `dsr`, as the Mayo Clinic's `dbh.h` has
/// them and nibabel names them: `header_key`, `image_dimension` and
/// `data_history` one after another.
///
/// Where NIfTI-1 differs is most of the point of reading this one:
///
/// - No magic. The last four bytes are `smin`, and nothing but `sizeof_hdr`
///   and fields that agree with each other says a file is this format.
/// - `hkey_un0` is where NIfTI put `dim_info`, and `vox_units`, `cal_units`
///   and `unused1` are where it put the three intent parameters and
///   `intent_code`: Analyze named the units in text, NIfTI in two bit fields.
/// - `dim_un0` became `slice_start`, `funused1` and `funused2` became
///   `scl_slope` and `scl_inter` (SPM2 had already used `funused1` as a
///   scale), `funused3` became `slice_end`, `slice_code` and `xyzt_units`, and
///   `compressed` and `verified` became `slice_duration` and `toffset`.
/// - Everything after `aux_file` is different. Analyze describes orientation
///   with one `orient` code and a scanner's bookkeeping in short strings;
///   NIfTI replaced all of it with two affine transforms, `intent_name` and
///   the magic.
/// - No extensions, and no voxels in the same file.
///
/// `compressed` and `verified` are floats in Mayo's own listing and 32-bit
/// integers in nibabel; they are read as nibabel reads them, and are 0 in
/// every file seen.
fn analyze_header(e: Endian) -> T {
    let i16_ = || T::Int { bits: 16, endian: e };
    let text = |n: i128| T::text(StrLen::Padded { size: E::lit(n), pad: 0 }, Encoding::Ascii);
    T::structure(
        "AnalyzeHeader",
        vec![
            ("sizeof_hdr", T::i32(e)),
            ("data_type", text(10)),
            ("db_name", text(18)),
            ("extents", T::i32(e)),
            ("session_error", i16_()),
            ("regular", text(1)),
            ("hkey_un0", T::u8()),
            ("dim", T::array(i16_(), E::lit(8))),
            ("vox_units", text(4)),
            ("cal_units", text(8)),
            ("unused1", i16_()),
            ("datatype", T::enumeration("VoxelType", i16_(), VOXEL_TYPES)),
            ("bitpix", i16_()),
            ("dim_un0", i16_()),
            ("pixdim", T::array(T::F32(e), E::lit(8))),
            ("vox_offset", T::F32(e)),
            ("funused1", T::F32(e)),
            ("funused2", T::F32(e)),
            ("funused3", T::F32(e)),
            ("cal_max", T::F32(e)),
            ("cal_min", T::F32(e)),
            ("compressed", T::i32(e)),
            ("verified", T::i32(e)),
            ("glmax", T::i32(e)),
            ("glmin", T::i32(e)),
            ("descrip", text(80)),
            ("aux_file", text(24)),
            ("orient", T::u8()),
            // Text in Analyze's own listing; SPM99 wrote the origin here as
            // five 16-bit numbers, which is most of the files there are, so
            // the bytes are shown rather than a string they do not make.
            ("originator", T::bytes(E::lit(10))),
            ("generated", text(10)),
            ("scannum", text(10)),
            ("patient_id", text(10)),
            ("exp_date", text(10)),
            ("exp_time", text(10)),
            ("hist_un0", T::bytes(E::lit(3))),
            ("views", T::i32(e)),
            ("vols_added", T::i32(e)),
            ("start_field", T::i32(e)),
            ("field_skip", T::i32(e)),
            ("omax", T::i32(e)),
            ("omin", T::i32(e)),
            ("smax", T::i32(e)),
            ("smin", T::i32(e)),
        ],
    )
    .machinery(&["hkey_un0", "unused1", "dim_un0", "hist_un0"])
    .payload(&["dim", "datatype", "pixdim"])
}

/// A NIfTI file: the header, the four bytes that say whether extensions
/// follow, the extensions, and in a `.nii` the voxels.
fn file(e: Endian, v: Version, single: bool) -> T {
    let (header, size, offset, name) = match (v, single) {
        (Version::One, _) => (header_1(e), HEADER_1, "data_offset", ["NIfTI-1", "NIfTI-1 header"]),
        (Version::Two, _) => (header_2(e), HEADER_2, "vox_offset", ["NIfTI-2", "NIfTI-2 header"]),
    };
    // In a `.nii` the extensions stop where the voxels start; in a `.hdr`
    // there is nothing after them but the end of the file.
    let room = match single {
        true => E::within(&["header", offset]).sub(E::lit(size)).sub(E::size_of("extender")).at_least(E::lit(0)),
        false => E::Remaining,
    };
    let mut fields = vec![("header", header), ("extender", extender()), ("extensions", extensions(e, room))];
    if single {
        fields.push(("voxels", voxels(e)));
    }
    T::structure(name[usize::from(!single)], fields).payload(&["header", "voxels"])
}

/// The 348 bytes, field by field as `nifti1.h` has them. The first 40 are
/// Analyze's `header_key` with only `dim_info` put to use; the rest are
/// Analyze's slots given new jobs, which is why a float says where the voxels
/// start.
fn header_1(e: Endian) -> T {
    let i16_ = || T::Int { bits: 16, endian: e };
    let text = |n: i128| T::text(StrLen::Padded { size: E::lit(n), pad: 0 }, Encoding::Ascii);
    let floats = |n: i128| T::array(T::F32(e), E::lit(n));
    // The float at the next four bytes, and the one after it, as bits.
    let here = E::peek(32, e);
    let next = E::peek_at(E::lit(32), 32, e);
    let is_scaled = scaled(SINGLE, &here, &next);
    T::structure(
        "Nifti1Header",
        vec![
            ("sizeof_hdr", T::i32(e)),
            ("data_type", text(10)),
            ("db_name", text(18)),
            ("extents", T::i32(e)),
            ("session_error", i16_()),
            ("regular", text(1)),
            ("dim_info", dim_info()),
            ("dim", T::array(i16_(), E::lit(8))),
            ("intent_p1", T::F32(e)),
            ("intent_p2", T::F32(e)),
            ("intent_p3", T::F32(e)),
            ("intent_code", T::enumeration("NiftiIntent", i16_(), INTENTS)),
            ("datatype", T::enumeration("VoxelType", i16_(), VOXEL_TYPES)),
            ("bitpix", i16_()),
            ("slice_start", i16_()),
            ("pixdim", floats(8)),
            // The float after this, read as the whole number of bytes it
            // holds. It covers no bits: `vox_offset` is still the field.
            ("data_offset", T::computed(SINGLE.offset(&here))),
            ("vox_offset", T::F32(e)),
            // The two floats after these, read as the whole numbers they
            // hold when the voxels are scaled here. See [`scaling`].
            ("scale", scaling(is_scaled.clone(), SINGLE.value(&here))),
            ("zero", scaling(is_scaled, SINGLE.value(&next))),
            ("scl_slope", T::F32(e)),
            ("scl_inter", T::F32(e)),
            ("slice_end", i16_()),
            ("slice_code", T::enumeration("NiftiSliceOrder", T::u8(), SLICE_ORDERS)),
            ("xyzt_units", xyzt_units(None, e)),
            ("cal_max", T::F32(e)),
            ("cal_min", T::F32(e)),
            ("slice_duration", T::F32(e)),
            ("toffset", T::F32(e)),
            ("glmax", T::i32(e)),
            ("glmin", T::i32(e)),
            ("descrip", text(80)),
            ("aux_file", text(24)),
            ("qform_code", T::enumeration("NiftiXform", i16_(), XFORMS)),
            ("sform_code", T::enumeration("NiftiXform", i16_(), XFORMS)),
            ("quatern_b", T::F32(e)),
            ("quatern_c", T::F32(e)),
            ("quatern_d", T::F32(e)),
            ("qoffset_x", T::F32(e)),
            ("qoffset_y", T::F32(e)),
            ("qoffset_z", T::F32(e)),
            ("srow_x", floats(4)),
            ("srow_y", floats(4)),
            ("srow_z", floats(4)),
            ("intent_name", text(16)),
            ("magic", text(4)),
        ],
    )
    .machinery(&["data_offset", "scale", "zero", "data_type", "db_name", "extents", "session_error", "regular", "glmax", "glmin"])
    .payload(&["dim", "datatype", "pixdim"])
}

/// The 540 bytes of a NIfTI-2 header, as `nifti2.h` has them. The same fields
/// as NIfTI-1 less Analyze's leftovers, at 64 bits where NIfTI-1 had fewer, and
/// in a different order: the magic moved to the front and the small fields to
/// the back. `vox_offset` is an integer at last, so nothing has to read a
/// float to find the voxels.
///
/// The eight bytes `nifti2.h` calls `magic` are split as nibabel splits them:
/// the text, and the four bytes after it that are there to show whether a
/// transfer changed `\r\n` or `\032`.
fn header_2(e: Endian) -> T {
    let i64_ = || T::Int { bits: 64, endian: e };
    let text = |n: i128| T::text(StrLen::Padded { size: E::lit(n), pad: 0 }, Encoding::Ascii);
    let doubles = |n: i128| T::array(T::F64(e), E::lit(n));
    let here = E::peek(64, e);
    let next = E::peek_at(E::lit(64), 64, e);
    let is_scaled = scaled(DOUBLE, &here, &next);
    T::structure(
        "Nifti2Header",
        vec![
            ("sizeof_hdr", T::i32(e)),
            ("magic", text(4)),
            ("eol_check", T::bytes(E::lit(4))),
            ("datatype", T::enumeration("VoxelType", T::Int { bits: 16, endian: e }, VOXEL_TYPES)),
            ("bitpix", T::Int { bits: 16, endian: e }),
            ("dim", T::array(i64_(), E::lit(8))),
            ("intent_p1", T::F64(e)),
            ("intent_p2", T::F64(e)),
            ("intent_p3", T::F64(e)),
            ("pixdim", doubles(8)),
            ("vox_offset", i64_()),
            ("scale", scaling(is_scaled.clone(), DOUBLE.value(&here))),
            ("zero", scaling(is_scaled, DOUBLE.value(&next))),
            ("scl_slope", T::F64(e)),
            ("scl_inter", T::F64(e)),
            ("cal_max", T::F64(e)),
            ("cal_min", T::F64(e)),
            ("slice_duration", T::F64(e)),
            ("toffset", T::F64(e)),
            ("slice_start", i64_()),
            ("slice_end", i64_()),
            ("descrip", text(80)),
            ("aux_file", text(24)),
            ("qform_code", T::enumeration("NiftiXform", T::i32(e), XFORMS)),
            ("sform_code", T::enumeration("NiftiXform", T::i32(e), XFORMS)),
            ("quatern_b", T::F64(e)),
            ("quatern_c", T::F64(e)),
            ("quatern_d", T::F64(e)),
            ("qoffset_x", T::F64(e)),
            ("qoffset_y", T::F64(e)),
            ("qoffset_z", T::F64(e)),
            ("srow_x", doubles(4)),
            ("srow_y", doubles(4)),
            ("srow_z", doubles(4)),
            ("slice_code", T::enumeration("NiftiSliceOrder", T::i32(e), SLICE_ORDERS)),
            ("xyzt_units", xyzt_units(Some(T::UInt { bits: 24, endian: e }), e)),
            ("intent_code", T::enumeration("NiftiIntent", T::i32(e), INTENTS)),
            ("intent_name", text(16)),
            ("dim_info", dim_info()),
            ("unused_str", text(15)),
        ],
    )
    .machinery(&["scale", "zero", "eol_check", "unused_str"])
    .payload(&["dim", "datatype", "pixdim", "vox_offset"])
}

/// `dim_info`: which axis, 1 to 3 or 0 for not said, the frequency encoding,
/// the phase encoding and the slices each ran along. Two bits apiece from the
/// bottom of the byte, which is the order `nifti1.h`'s macros take them in.
fn dim_info() -> T {
    let two = || T::UInt { bits: 2, endian: Little };
    let axis = || T::enumeration("NiftiAxis", two(), AXES);
    T::inline_structure("DimInfo", vec![("freq_dim", axis()), ("phase_dim", axis()), ("slice_dim", axis()), ("unused", two())])
        .machinery(&["unused"])
}

/// `xyzt_units`: three bits for what the spatial `pixdim` measure and three
/// for what the fourth one does, from the bottom of the byte. `upper` is what
/// a wider field holds above that byte, and NIfTI-1's has nothing above it.
fn xyzt_units(upper: Option<T>, e: Endian) -> T {
    let bits = |n: u32| T::UInt { bits: n, endian: Little };
    let low = vec![
        ("space", T::enumeration("NiftiSpaceUnit", bits(3), SPACE_UNITS)),
        ("time", T::enumeration("NiftiTimeUnit", bits(3), TIME_UNITS)),
        ("unused", bits(2)),
    ];
    // The byte the units are in is the low one, which a big-endian number
    // writes last.
    let fields = match (upper, e) {
        (None, _) => low,
        (Some(upper), Little) => low.into_iter().chain([("upper", upper)]).collect(),
        (Some(upper), Big) => [("upper", upper)].into_iter().chain(low).collect(),
    };
    T::inline_structure("XyztUnits", fields).machinery(&["unused", "upper"])
}

/// The four bytes after the header. Only the first one means anything: not
/// zero says extensions follow. A `.hdr` of exactly 348 bytes leaves them out.
fn extender() -> T {
    T::if_room(T::bytes(E::lit(4)))
}

/// The extensions, in `room` bytes, when the byte before them says there are
/// any. When it says not, whatever is in the room is padding before the
/// voxels, and keeps its bytes.
fn extensions(e: Endian, room: E) -> T {
    // The four bytes read as a big-endian number, so the first of them is not
    // zero when this is at least two to the 24.
    let follow = E::field("extender").greater_or_equal(E::lit(1 << 24));
    let size = || E::field("esize").sub(E::lit(8)).at_least(E::lit(0)).at_most(E::Remaining);
    let text = T::text(StrLen::Padded { size: size(), pad: 0 }, Encoding::Utf8);
    let record = T::structure(
        "NiftiExtension",
        vec![
            ("esize", T::i32(e)),
            ("ecode", T::enumeration("NiftiEcode", T::i32(e), ECODES)),
            ("edata", T::switch(E::field("ecode"), TEXT_ECODES.iter().map(|c| (*c, text.clone())).collect(), T::bytes(size()))),
        ],
    );
    // The records are read inside the room, so one whose size runs past it
    // fails on its own rather than reading a size out of the voxels.
    let room = room.at_most(E::Remaining);
    T::switch(follow, vec![(1, T::sized(room.clone(), T::repeat(record, Until::End)))], T::bytes(room))
}

/// How long dimension `k` of the header's `dim` is, with `dim[0]` the count.
fn dim(k: i128) -> E {
    E::elem_within(&["header", "dim"], E::lit(k), &[])
}

/// The voxels, as the type `datatype` names, grouped by `dim`. Integers are
/// scaled when the header says to; see [`scalable`].
fn voxels(e: Endian) -> T {
    let int = |bits: u32| T::Int { bits, endian: e };
    let uint = |bits: u32| T::UInt { bits, endian: e };
    let pair = |part: T| T::inline_structure("Complex", vec![("re", part.clone()), ("im", part)]);
    let colour = |names: &[&'static str]| {
        let name = if names.len() == 3 { "Rgb" } else { "Rgba" };
        T::inline_structure(name, names.iter().map(|n| (*n, T::u8())).collect())
    };
    T::switch(
        E::within(&["header", "datatype"]),
        vec![
            (2, scalable(uint(8), 1)),
            (4, scalable(int(16), 2)),
            (8, scalable(int(32), 4)),
            (16, shaped(T::F32(e), 4)),
            (32, shaped(pair(T::F32(e)), 8)),
            (64, shaped(T::F64(e), 8)),
            (128, shaped(colour(&["r", "g", "b"]), 3)),
            (256, scalable(int(8), 1)),
            (512, scalable(uint(16), 2)),
            (768, scalable(uint(32), 4)),
            (1024, scalable(int(64), 8)),
            (1280, scalable(uint(64), 8)),
            (1536, shaped(T::bytes(E::lit(16)), 16)),
            (1792, shaped(pair(T::F64(e)), 16)),
            (2048, shaped(T::bytes(E::lit(32)), 32)),
            (2304, shaped(colour(&["r", "g", "b", "a"]), 4)),
        ],
        // A bit a voxel, or a datatype nobody defined: the bytes.
        T::bytes(E::Remaining),
    )
}

/// Integer voxels, read as stored or, when the header's `scale` is not 0, as
/// the integer on disk and what `scale` and `zero` say it is worth.
fn scalable(stored: T, width: i128) -> T {
    let worth = E::field("stored").mul(E::within(&["header", "scale"])).add(E::within(&["header", "zero"]));
    let with = T::inline_structure("Scaled", vec![("stored", stored.clone()), ("worth", T::computed(worth))]).payload(&["stored"]);
    T::switch(E::within(&["header", "scale"]), vec![(0, shaped(stored, width))], shaped(with, width))
}

/// The most dimensions `dim` can say, and so the deepest the voxels nest.
const DIMS: i128 = 7;

/// A dimension is read as no longer than two to the 40, and the size of one
/// element of a level as no more than two to the 62, so that multiplying a
/// header of nonsense out cannot overflow the arithmetic.
const LONGEST_DIM: i128 = 1 << 40;
const WIDEST_STRIDE: i128 = 1 << 62;

/// Voxels of `width` bytes, nested as `dim` says: the innermost run is
/// `dim[1]` long and the outermost `dim[dim[0]]`, since the first index varies
/// fastest. Every count is capped by the room left, so a file cut short shows
/// the voxels it has.
fn shaped(elem: T, width: i128) -> T {
    let nest = |n: i128| {
        let (mut ty, mut stride) = (elem.clone(), E::lit(width));
        for k in 1..=n {
            let d = dim(k).at_least(E::lit(0)).at_most(E::lit(LONGEST_DIM));
            ty = T::array(ty, d.clone().at_most(E::Remaining.div(stride.clone().at_least(E::lit(1)))));
            stride = stride.mul(d).at_most(E::lit(WIDEST_STRIDE));
        }
        ty
    };
    // A `dim[0]` outside 1 to 7 says nothing usable about the shape, and the
    // voxels are one run.
    T::switch(dim(0), (1..=DIMS).map(|n| (n, nest(n))).collect(), T::array(elem.clone(), E::Remaining.div(E::lit(width))))
}

/// A NIfTI file of either version, of either kind and in either byte order:
/// `sizeof_hdr` is 348 or 540 read one way or the other, and the magic that
/// size puts it at is one of the two. Eight bytes that have to agree, which
/// nothing else is likely to write.
pub fn is_nifti(head: &[u8], _len: u64) -> bool {
    let Some(size) = head.get(..4) else { return false };
    let sized = |n: u32| size == n.to_le_bytes() || size == n.to_be_bytes();
    let magic = |at: usize| head.get(at..at + 4);
    (sized(348) && matches!(magic(MAGIC_1_AT), Some(b"n+1\0" | b"ni1\0")))
        || (sized(540) && matches!(magic(MAGIC_2_AT), Some(b"n+2\0" | b"ni2\0")))
}

/// An Analyze 7.5 header, which has no magic. `sizeof_hdr` is 348 one way
/// round or the other and the NIfTI-1 magic is not where it would be, and
/// then the fields that describe the voxels have to agree: `dim[0]` a number
/// of dimensions from 1 to 7, each of those dimensions at least 1, and a
/// `datatype` Analyze defined with the `bitpix` that goes with it.
///
/// Four bytes of size and a handful of small numbers that have to make sense
/// together, which is weaker than a signature, so this is asked after every
/// format that has one.
pub fn is_analyze(head: &[u8], len: u64) -> bool {
    if len < HEADER_1 as u64 || head.len() < HEADER_1 as usize || is_nifti(head, len) {
        return false;
    }
    let big = match head[..4] {
        [0x5c, 1, 0, 0] => false,
        [0, 0, 1, 0x5c] => true,
        _ => return false,
    };
    let short = |at: usize| {
        let b = [head[at], head[at + 1]];
        if big { i16::from_be_bytes(b) } else { i16::from_le_bytes(b) }
    };
    let rank = short(40);
    if !(1..=7).contains(&rank) || (1..=rank as usize).any(|k| short(40 + 2 * k) < 1) {
        return false;
    }
    // Analyze's types, each with the only width it can be.
    let widths = [(1, 1), (2, 8), (4, 16), (8, 32), (16, 32), (32, 64), (64, 64), (128, 24)];
    widths.contains(&(short(70), short(72)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    /// A header of the fields these tests look at, in either byte order. The
    /// rest are zero, which is what most writers leave them.
    struct Header {
        big: bool,
        magic: &'static [u8; 4],
        dims: Vec<i16>,
        datatype: i16,
        bitpix: i16,
        vox_offset: f32,
        slope: f32,
        inter: f32,
        dim_info: u8,
        xyzt_units: u8,
    }

    impl Header {
        fn new(big: bool) -> Header {
            Header {
                big,
                magic: b"n+1\0",
                dims: vec![3, 4, 3, 2],
                datatype: 4,
                bitpix: 16,
                vox_offset: 352.0,
                slope: 0.0,
                inter: 0.0,
                dim_info: 0,
                xyzt_units: 0,
            }
        }

        fn bytes(&self) -> Vec<u8> {
            let mut v = vec![0u8; HEADER_1 as usize];
            let big = self.big;
            let put = |v: &mut Vec<u8>, at: usize, b: &[u8]| v[at..at + b.len()].copy_from_slice(b);
            let i16b = |x: i16| if big { x.to_be_bytes() } else { x.to_le_bytes() };
            let i32b = |x: i32| if big { x.to_be_bytes() } else { x.to_le_bytes() };
            let f32b = |x: f32| if big { x.to_be_bytes() } else { x.to_le_bytes() };
            put(&mut v, 0, &i32b(348));
            put(&mut v, 38, b"r");
            v[39] = self.dim_info;
            for (i, d) in self.dims.iter().enumerate() {
                put(&mut v, 40 + 2 * i, &i16b(*d));
            }
            put(&mut v, 70, &i16b(self.datatype));
            put(&mut v, 72, &i16b(self.bitpix));
            put(&mut v, 108, &f32b(self.vox_offset));
            put(&mut v, 112, &f32b(self.slope));
            put(&mut v, 116, &f32b(self.inter));
            v[123] = self.xyzt_units;
            put(&mut v, 148, b"hand-built");
            put(&mut v, 344, self.magic);
            v
        }
    }

    fn eval(bytes: Vec<u8>) -> (Document<MemSource>, Evaluator) {
        (Document::new(MemSource(bytes)), Evaluator::new(nifti()))
    }

    fn at(ev: &mut Evaluator, d: &Document<MemSource>, names: &[&str]) -> Vec<usize> {
        let mut p = Vec::new();
        for n in names {
            p = ev.child_named(d, &p, n).unwrap().unwrap_or_else(|| panic!("no {n} under {p:?}"));
        }
        p
    }

    fn named(ev: &mut Evaluator, d: &Document<MemSource>, names: &[&str]) -> crate::eval::NodeInfo {
        let p = at(ev, d, names);
        ev.node(d, &p).unwrap()
    }

    fn value(ev: &mut Evaluator, d: &Document<MemSource>, names: &[&str]) -> Value {
        named(ev, d, names).value
    }

    /// Four bytes of extender saying none follow, and then `count` voxels of
    /// 16 bits counting up from 0.
    fn with_voxels(h: &Header, count: i16) -> Vec<u8> {
        let mut v = h.bytes();
        v.extend_from_slice(&[0; 4]);
        for i in 0..count {
            v.extend_from_slice(&if h.big { i.to_be_bytes() } else { i.to_le_bytes() });
        }
        v
    }

    #[test]
    fn the_header_reads_either_way_round() {
        for big in [false, true] {
            let (d, mut ev) = eval(with_voxels(&Header::new(big), 24));
            assert_eq!(ev.node(&d, &[]).unwrap().type_name, "NIfTI-1", "big={big}");
            assert_eq!(value(&mut ev, &d, &["header", "sizeof_hdr"]).as_int(), Some(348));
            let dim = at(&mut ev, &d, &["header", "dim"]);
            assert_eq!(ev.node(&d, &[dim.clone(), vec![1]].concat()).unwrap().value.as_int(), Some(4));
            let datatype = named(&mut ev, &d, &["header", "datatype"]);
            assert_eq!(datatype.type_name, "VoxelType");
            assert_eq!(datatype.value.as_int(), Some(4));
            assert_eq!(value(&mut ev, &d, &["header", "vox_offset"]), Value::Float(352.0));
            assert_eq!(value(&mut ev, &d, &["header", "descrip"]), Value::Str("hand-built".into()));
            assert_eq!(value(&mut ev, &d, &["header", "magic"]), Value::Str("n+1".into()));
            let header = named(&mut ev, &d, &["header"]);
            assert_eq!(header.size_bits, 348 * 8);
        }
    }

    /// `vox_offset` is a float, and what places the voxels is the whole number
    /// it holds.
    #[test]
    fn the_voxels_start_where_the_float_says() {
        for big in [false, true] {
            let mut h = Header::new(big);
            h.vox_offset = 368.0;
            let mut v = h.bytes();
            v.extend_from_slice(&[0; 20]);
            v.extend_from_slice(&[0xab; 48]);
            let (d, mut ev) = eval(v);
            assert_eq!(value(&mut ev, &d, &["header", "data_offset"]).as_int(), Some(368), "big={big}");
            let voxels = named(&mut ev, &d, &["voxels"]);
            assert_eq!(voxels.offset_bits, 368 * 8);
            // With no extensions said to follow, the sixteen bytes between the
            // extender and the voxels are padding.
            let ext = named(&mut ev, &d, &["extensions"]);
            assert_eq!((ext.type_name.as_str(), ext.size_bits), ("bytes[]", 16 * 8));
        }
    }

    /// The first index varies fastest: 4 by 3 by 2 is two slices of three rows
    /// of four, and the voxel nibabel calls `data[1, 0, 0]` is the second one
    /// written.
    #[test]
    fn the_first_dimension_is_the_innermost_run() {
        for big in [false, true] {
            let (d, mut ev) = eval(with_voxels(&Header::new(big), 24));
            let voxels = at(&mut ev, &d, &["voxels"]);
            let top = ev.node(&d, &voxels).unwrap();
            assert_eq!(top.child_count, 2, "big={big}");
            assert_eq!(ev.node(&d, &[voxels.clone(), vec![0]].concat()).unwrap().child_count, 3);
            let row = ev.node(&d, &[voxels.clone(), vec![0, 0]].concat()).unwrap();
            assert_eq!((row.type_name.as_str(), row.child_count), (if big { "i16 be[]" } else { "i16 le[]" }, 4));
            assert_eq!(ev.node(&d, &[voxels.clone(), vec![0, 0, 1]].concat()).unwrap().value.as_int(), Some(1));
            // data[1, 2, 1] is 1 + 4 * 2 + 12 * 1.
            assert_eq!(ev.node(&d, &[voxels.clone(), vec![1, 2, 1]].concat()).unwrap().value.as_int(), Some(21));
            assert_eq!(top.offset_bits + top.size_bits, d.len_bits());
        }
    }

    /// Whole-number scaling is worked out on each voxel, beside the integer on
    /// disk.
    #[test]
    fn a_whole_slope_and_intercept_are_worked_out() {
        for big in [false, true] {
            let mut h = Header::new(big);
            h.slope = 2.0;
            h.inter = -3.0;
            let (d, mut ev) = eval(with_voxels(&h, 24));
            assert_eq!(value(&mut ev, &d, &["header", "scale"]).as_int(), Some(2), "big={big}");
            assert_eq!(value(&mut ev, &d, &["header", "zero"]).as_int(), Some(-3));
            let voxels = at(&mut ev, &d, &["voxels"]);
            let voxel = [voxels.clone(), vec![1, 2, 1]].concat();
            assert_eq!(ev.node(&d, &voxel).unwrap().type_name, "Scaled");
            assert_eq!(ev.node(&d, &[voxel.clone(), vec![0]].concat()).unwrap().value.as_int(), Some(21));
            assert_eq!(ev.node(&d, &[voxel.clone(), vec![1]].concat()).unwrap().value.as_int(), Some(39));
            // The worth takes no bits, so the voxels still end with the file.
            let top = ev.node(&d, &voxels).unwrap();
            assert_eq!(top.offset_bits + top.size_bits, d.len_bits());
        }
    }

    /// A slope that is 0, NaN, a fraction, or 1 with an intercept of 0 leaves
    /// the voxels as stored, and so does a fraction in the intercept.
    #[test]
    fn scaling_that_cannot_be_worked_out_or_changes_nothing_is_left_alone() {
        for (slope, inter) in [(0.0, 5.0), (f32::NAN, 0.0), (0.5, 0.0), (1.0, 0.0), (2.0, 0.25), (1e30, 0.0), (2.0, f32::INFINITY)] {
            let mut h = Header::new(false);
            (h.slope, h.inter) = (slope, inter);
            let (d, mut ev) = eval(with_voxels(&h, 24));
            for name in ["scale", "zero"] {
                let n = named(&mut ev, &d, &["header", name]);
                assert_eq!((n.type_name.as_str(), n.size_bits), ("bytes[]", 0), "{name} for {slope} {inter}");
            }
            let row = at(&mut ev, &d, &["voxels"]);
            assert_eq!(ev.node(&d, &[row, vec![0, 0]].concat()).unwrap().type_name, "i16 le[]");
        }
        // An intercept on its own, with a slope of 1, is scaling.
        let mut h = Header::new(true);
        (h.slope, h.inter) = (1.0, 32768.0);
        let (d, mut ev) = eval(with_voxels(&h, 24));
        let voxels = at(&mut ev, &d, &["voxels"]);
        assert_eq!(ev.node(&d, &[voxels, vec![0, 0, 3, 1]].concat()).unwrap().value.as_int(), Some(32771));
    }

    /// Floats are decoded from their bits across the range that matters: small
    /// whole numbers, a large one, one just under the limit, and the fractions
    /// either side of a whole one.
    #[test]
    fn floats_read_as_whole_numbers_from_their_bits() {
        for (x, whole, want) in [
            (0.0f32, 1, 0i128),
            (-0.0, 1, 0),
            (1.0, 1, 1),
            (-7.0, 1, -7),
            (352.0, 1, 352),
            (65536.0, 1, 65536),
            (16777216.0, 1, 16777216),
            (1099511562240.0, 1, 1099511562240),
            (0.75, 0, 0),
            (352.5, 0, 0),
            (f32::NAN, 0, 0),
            (f32::INFINITY, 0, 0),
            (2e12, 0, 0),
        ] {
            let mut v = x.to_le_bytes().to_vec();
            v.extend_from_slice(&[0; 4]);
            let bits = E::peek(32, Little);
            let t = T::structure(
                "Probe",
                vec![("whole", T::computed(SINGLE.whole(&bits))), ("value", T::computed(E::cond(SINGLE.whole(&bits), SINGLE.value(&bits), E::lit(0))))],
            );
            let d = Document::new(MemSource(v));
            let mut ev = Evaluator::new(Template::new("probe", t));
            assert_eq!(ev.node(&d, &[0]).unwrap().value.as_int(), Some(whole), "{x}");
            assert_eq!(ev.node(&d, &[1]).unwrap().value.as_int(), Some(want), "{x}");
        }
    }

    /// Two extensions, a comment and a code with bytes, running up to
    /// `vox_offset`, which counts them.
    #[test]
    fn extensions_run_to_the_voxels() {
        for big in [false, true] {
            let mut h = Header::new(big);
            h.vox_offset = 352.0 + 32.0 + 16.0;
            let i32b = |x: i32| if big { x.to_be_bytes() } else { x.to_le_bytes() };
            let mut v = h.bytes();
            v.extend_from_slice(&[1, 0, 0, 0]);
            v.extend_from_slice(&i32b(32));
            v.extend_from_slice(&i32b(6));
            let mut note = b"motion corrected".to_vec();
            note.resize(24, 0);
            v.extend_from_slice(&note);
            v.extend_from_slice(&i32b(16));
            v.extend_from_slice(&i32b(16));
            v.extend_from_slice(&[9; 8]);
            v.extend_from_slice(&[0; 48]);
            let (d, mut ev) = eval(v);
            let list = at(&mut ev, &d, &["extensions"]);
            let node = ev.node(&d, &list).unwrap();
            assert_eq!((node.child_count, node.size_bits), (2, 48 * 8), "big={big}");
            assert_eq!(node.type_name, "NiftiExtension[]");
            let first = [list.clone(), vec![0]].concat();
            assert_eq!(ev.node(&d, &first).unwrap().type_name, "NiftiExtension");
            assert_eq!(ev.node(&d, &[first.clone(), vec![1]].concat()).unwrap().value.as_int(), Some(6));
            assert_eq!(ev.node(&d, &[first, vec![2]].concat()).unwrap().value, Value::Str("motion corrected".into()));
            let second = [list, vec![1, 2]].concat();
            assert_eq!(ev.node(&d, &second).unwrap().type_name, "bytes[]");
            assert_eq!(named(&mut ev, &d, &["voxels"]).offset_bits, 400 * 8);
        }
    }

    /// A `.hdr` of a pair: no voxels, and extensions to the end of the file.
    #[test]
    fn a_pair_header_has_no_voxels_and_its_extensions_run_to_the_end() {
        let mut h = Header::new(false);
        h.magic = b"ni1\0";
        h.vox_offset = 0.0;
        let mut v = h.bytes();
        v.extend_from_slice(&[1, 0, 0, 0]);
        v.extend_from_slice(&16i32.to_le_bytes());
        v.extend_from_slice(&0i32.to_le_bytes());
        v.extend_from_slice(&[0; 8]);
        let (d, mut ev) = eval(v);
        assert_eq!(ev.node(&d, &[]).unwrap().type_name, "NIfTI-1 header");
        assert_eq!(ev.child_named(&d, &[], "voxels").unwrap(), None);
        assert_eq!(named(&mut ev, &d, &["extensions"]).child_count, 1);
        // And one with no extender at all, exactly 348 bytes.
        let (d, mut ev) = eval(h.bytes());
        assert_eq!(named(&mut ev, &d, &["extensions"]).size_bits, 0);
    }

    /// `dim_info` and `xyzt_units` read as their bits: 57 is frequency along 1,
    /// phase along 2 and slices along 3, and 10 is millimetres and seconds.
    #[test]
    fn the_packed_bytes_read_as_their_bits() {
        let mut h = Header::new(false);
        (h.dim_info, h.xyzt_units) = (57, 10);
        let (d, mut ev) = eval(with_voxels(&h, 24));
        assert_eq!(value(&mut ev, &d, &["header", "dim_info", "freq_dim"]).as_int(), Some(1));
        assert_eq!(value(&mut ev, &d, &["header", "dim_info", "phase_dim"]).as_int(), Some(2));
        assert_eq!(value(&mut ev, &d, &["header", "dim_info", "slice_dim"]).as_int(), Some(3));
        let space = named(&mut ev, &d, &["header", "xyzt_units", "space"]);
        assert_eq!((space.type_name.as_str(), space.value.as_int()), ("NiftiSpaceUnit", Some(2)));
        let time = named(&mut ev, &d, &["header", "xyzt_units", "time"]);
        assert_eq!(time.value.as_int(), Some(1));
        assert_eq!(time.offset_bits, 123 * 8 + 2, "bits 3 to 5, counted from the top of the byte");
    }

    /// A NIfTI-2 `.nii` in either byte order: 540 bytes of header, no
    /// extensions, and 4 by 3 by 2 voxels of 16 bits counting up, with the
    /// slope and intercept doubles given.
    fn nifti2(big: bool, slope: f64, inter: f64) -> Vec<u8> {
        let mut v = vec![0u8; 540];
        let put = |v: &mut Vec<u8>, at: usize, b: &[u8]| v[at..at + b.len()].copy_from_slice(b);
        let i16b = |x: i16| if big { x.to_be_bytes() } else { x.to_le_bytes() };
        let i32b = |x: i32| if big { x.to_be_bytes() } else { x.to_le_bytes() };
        let i64b = |x: i64| if big { x.to_be_bytes() } else { x.to_le_bytes() };
        let f64b = |x: f64| if big { x.to_be_bytes() } else { x.to_le_bytes() };
        put(&mut v, 0, &i32b(540));
        put(&mut v, 4, b"n+2\0\r\n\x1a\n");
        put(&mut v, 12, &i16b(512));
        put(&mut v, 14, &i16b(16));
        for (i, d) in [3i64, 4, 3, 2, 1, 1, 1, 1].iter().enumerate() {
            put(&mut v, 16 + 8 * i, &i64b(*d));
        }
        put(&mut v, 168, &i64b(544));
        put(&mut v, 176, &f64b(slope));
        put(&mut v, 184, &f64b(inter));
        put(&mut v, 240, b"two");
        put(&mut v, 344, &i32b(4));
        // Millimetres and milliseconds, with a stray bit in the upper bytes
        // to show those are not the units.
        put(&mut v, 500, &i32b(0x0100 | 2 | (2 << 3)));
        put(&mut v, 504, &i32b(2001));
        v[524] = 57;
        v.extend_from_slice(&[0; 4]);
        for i in 0..24u16 {
            v.extend_from_slice(&if big { i.to_be_bytes() } else { i.to_le_bytes() });
        }
        v
    }

    #[test]
    fn a_nifti_2_file_reads_at_the_wider_layout() {
        for big in [false, true] {
            let (d, mut ev) = eval(nifti2(big, f64::NAN, 0.0));
            assert_eq!(ev.node(&d, &[]).unwrap().type_name, "NIfTI-2", "big={big}");
            let header = named(&mut ev, &d, &["header"]);
            assert_eq!((header.type_name.as_str(), header.size_bits), ("Nifti2Header", 540 * 8));
            assert_eq!(value(&mut ev, &d, &["header", "magic"]), Value::Str("n+2".into()));
            assert_eq!(value(&mut ev, &d, &["header", "vox_offset"]).as_int(), Some(544));
            assert_eq!(value(&mut ev, &d, &["header", "descrip"]), Value::Str("two".into()));
            assert_eq!(value(&mut ev, &d, &["header", "qform_code"]).as_int(), Some(4));
            assert_eq!(value(&mut ev, &d, &["header", "intent_code"]).as_int(), Some(2001));
            assert_eq!(value(&mut ev, &d, &["header", "xyzt_units", "space"]).as_int(), Some(2));
            assert_eq!(value(&mut ev, &d, &["header", "xyzt_units", "time"]).as_int(), Some(2));
            assert_eq!(value(&mut ev, &d, &["header", "xyzt_units", "upper"]).as_int(), Some(1));
            assert_eq!(value(&mut ev, &d, &["header", "dim_info", "slice_dim"]).as_int(), Some(3));
            let voxels = at(&mut ev, &d, &["voxels"]);
            let top = ev.node(&d, &voxels).unwrap();
            assert_eq!((top.offset_bits, top.child_count), (544 * 8, 2));
            let row = ev.node(&d, &[voxels.clone(), vec![1, 2]].concat()).unwrap();
            assert_eq!((row.type_name.as_str(), row.child_count), (if big { "u16 be[]" } else { "u16 le[]" }, 4));
            assert_eq!(ev.node(&d, &[voxels, vec![1, 2, 3]].concat()).unwrap().value.as_int(), Some(23));
        }
    }

    #[test]
    fn a_nifti_2_file_scales_by_its_doubles() {
        for big in [false, true] {
            let (d, mut ev) = eval(nifti2(big, 3.0, 1000.0));
            assert_eq!(value(&mut ev, &d, &["header", "scale"]).as_int(), Some(3), "big={big}");
            assert_eq!(value(&mut ev, &d, &["header", "zero"]).as_int(), Some(1000));
            let voxels = at(&mut ev, &d, &["voxels"]);
            assert_eq!(ev.node(&d, &[voxels, vec![1, 2, 3, 1]].concat()).unwrap().value.as_int(), Some(1069));
        }
        let (d, mut ev) = eval(nifti2(false, 0.1, 0.0));
        assert_eq!(named(&mut ev, &d, &["header", "scale"]).size_bits, 0);
        let voxels = at(&mut ev, &d, &["voxels"]);
        assert_eq!(ev.node(&d, &[voxels, vec![0, 0]].concat()).unwrap().type_name, "u16 le[]");
    }

    /// An Analyze header: the NIfTI-1 test header with its magic taken away,
    /// and units written where Analyze writes them.
    fn analyze_bytes(big: bool) -> Vec<u8> {
        let mut h = Header::new(big);
        h.magic = &[0; 4];
        let mut v = h.bytes();
        v[56..58].copy_from_slice(b"mm");
        let smin: i32 = -7;
        v[344..348].copy_from_slice(&if big { smin.to_be_bytes() } else { smin.to_le_bytes() });
        v
    }

    #[test]
    fn an_analyze_header_reads_as_analyze_from_either_template() {
        for big in [false, true] {
            for template in [analyze(), nifti()] {
                let d = Document::new(MemSource(analyze_bytes(big)));
                let mut ev = Evaluator::new(template);
                assert_eq!(ev.node(&d, &[]).unwrap().type_name, "Analyze 7.5", "big={big}");
                assert_eq!(named(&mut ev, &d, &["header"]).type_name, "AnalyzeHeader");
                assert_eq!(named(&mut ev, &d, &["header"]).size_bits, 348 * 8);
                assert_eq!(value(&mut ev, &d, &["header", "vox_units"]), Value::Str("mm".into()));
                assert_eq!(value(&mut ev, &d, &["header", "datatype"]).as_int(), Some(4));
                assert_eq!(value(&mut ev, &d, &["header", "regular"]), Value::Str("r".into()));
                assert_eq!(value(&mut ev, &d, &["header", "descrip"]), Value::Str("hand-built".into()));
                assert_eq!(value(&mut ev, &d, &["header", "smin"]).as_int(), Some(-7));
            }
        }
        // And the `analyze` template reads a NIfTI-1 header as Analyze too,
        // the way a program from before NIfTI would.
        let d = Document::new(MemSource(Header::new(false).bytes()));
        let mut ev = Evaluator::new(analyze());
        assert_eq!(value(&mut ev, &d, &["header", "funused1"]), Value::Float(0.0));
    }

    #[test]
    fn analyze_is_recognised_only_when_its_fields_agree() {
        assert!(is_analyze(&analyze_bytes(false), 348));
        assert!(is_analyze(&analyze_bytes(true), 348));
        // A NIfTI-1 header is NIfTI, not Analyze.
        assert!(!is_analyze(&Header::new(false).bytes(), 348));
        let broken = |f: &dyn Fn(&mut Vec<u8>)| {
            let mut v = analyze_bytes(false);
            f(&mut v);
            is_analyze(&v, 348)
        };
        assert!(!broken(&|v| v[72] = 8), "int16 is 16 bits, not 8");
        assert!(!broken(&|v| v[40] = 0), "no dimensions");
        assert!(!broken(&|v| v[40] = 8), "more than 7");
        assert!(!broken(&|v| v[44..46].copy_from_slice(&0i16.to_le_bytes())), "a dimension of nothing");
        assert!(!broken(&|v| v[70] = 3), "no such datatype");
        assert!(!is_analyze(&analyze_bytes(false)[..300], 300));
        // And the whole of recognition agrees, with NIfTI asked first.
        assert_eq!(crate::formats::sniff(&analyze_bytes(true), 348), Some("analyze"));
        assert_eq!(crate::formats::sniff(&with_voxels(&Header::new(true), 24), 400), Some("nifti"));
        assert_eq!(crate::formats::sniff(&nifti2(false, 0.0, 0.0), 588), Some("nifti"));
    }

    #[test]
    fn recognised_by_size_and_magic_either_way_round() {
        assert!(is_nifti(&nifti2(false, 0.0, 0.0), 588));
        assert!(is_nifti(&nifti2(true, 0.0, 0.0), 588));
        assert!(is_nifti(&Header::new(false).bytes(), 348));
        assert!(is_nifti(&Header::new(true).bytes(), 348));
        let mut pair = Header::new(true);
        pair.magic = b"ni1\0";
        assert!(is_nifti(&pair.bytes(), 348));
        let mut none = Header::new(false);
        none.magic = &[0; 4];
        assert!(!is_nifti(&none.bytes(), 348));
        assert!(!is_nifti(&[0u8; 400], 400));
    }
}
