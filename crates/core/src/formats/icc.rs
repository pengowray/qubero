//! ICC colour profiles, version 2 (ICC.1:2001-04) and version 4 (ICC.1:2022).
//!
//! A profile is a 128-byte header, a tag count, a table of twelve-byte tag
//! entries, and the tag data those entries point at. Each entry is a tag
//! signature, an offset and a size. The offset counts from the first byte of
//! the profile: the first byte of the file for an `.icc` file, and the first
//! byte of the inflated stream for a PNG's iCCP chunk. So the profile sits in
//! a window of its own and the entries place their data with
//! `T::at_in_window`, the way a TIFF's entries do (see `tiff.rs`).
//!
//! Every tag's data starts with a four-byte type signature and four reserved
//! bytes, and the type says how the rest is laid out. The tag says what the
//! data means: `rTRC` and `kTRC` are both curves. So the data is read by its
//! type. A type this file does not lay out is its type signature, the
//! reserved bytes, and the rest as bytes.
//!
//! Two entries may point at the same data, and display profiles often do: when
//! the three channels share one tone curve, `rTRC`, `gTRC` and `bTRC` have the
//! same offset. Each entry reads that data, and the hex view places its bytes
//! once.
//!
//! The data of each tag is in a window of its own too, so the offsets inside
//! a tag (the strings of an `mluc`, the curves and tables of an `mAB `) count
//! from the start of the tag, as the spec says.
//!
//! Every number is big-endian. A fixed-point number is shown as the integer
//! the file stores and the value it stands for.

use crate::template::{Encoding, Endian::*, Expr as E, Part, StrLen, Template, Ty as T};

/// A four-letter signature as the number a `u32` field reads it as.
const fn sig(s: &[u8; 4]) -> i128 {
    u32::from_be_bytes(*s) as i128
}

/// Profile classes, ICC.1:2022 table 18.
const DEVICE_CLASS: &[(i128, &str)] = &[
    (sig(b"scnr"), "scnr, input device"),
    (sig(b"mntr"), "mntr, display device"),
    (sig(b"prtr"), "prtr, output device"),
    (sig(b"link"), "link, device link"),
    (sig(b"spac"), "spac, colour space"),
    (sig(b"abst"), "abst, abstract"),
    (sig(b"nmcl"), "nmcl, named colour"),
];

/// Data colour spaces, table 19. The PCS field takes the same signatures.
const COLOUR_SPACE: &[(i128, &str)] = &[
    (sig(b"XYZ "), "XYZ"),
    (sig(b"Lab "), "Lab"),
    (sig(b"Luv "), "Luv"),
    (sig(b"YCbr"), "YCbCr"),
    (sig(b"Yxy "), "Yxy"),
    (sig(b"RGB "), "RGB"),
    (sig(b"GRAY"), "grey"),
    (sig(b"HSV "), "HSV"),
    (sig(b"HLS "), "HLS"),
    (sig(b"CMYK"), "CMYK"),
    (sig(b"CMY "), "CMY"),
    (sig(b"2CLR"), "2 colours"),
    (sig(b"3CLR"), "3 colours"),
    (sig(b"4CLR"), "4 colours"),
    (sig(b"5CLR"), "5 colours"),
    (sig(b"6CLR"), "6 colours"),
    (sig(b"7CLR"), "7 colours"),
    (sig(b"8CLR"), "8 colours"),
    (sig(b"9CLR"), "9 colours"),
    (sig(b"ACLR"), "10 colours"),
    (sig(b"BCLR"), "11 colours"),
    (sig(b"CCLR"), "12 colours"),
    (sig(b"DCLR"), "13 colours"),
    (sig(b"ECLR"), "14 colours"),
    (sig(b"FCLR"), "15 colours"),
];

/// Primary platforms, table 20, and Taligent, which version 2 listed too.
/// Nought means none was named.
const PLATFORM: &[(i128, &str)] = &[
    (0, "none"),
    (sig(b"APPL"), "Apple"),
    (sig(b"MSFT"), "Microsoft"),
    (sig(b"SGI "), "Silicon Graphics"),
    (sig(b"SUNW"), "Sun Microsystems"),
    (sig(b"TGNT"), "Taligent"),
];

/// Rendering intents, table 23. The header's field and the PNG sRGB chunk's
/// byte use the same four numbers.
const RENDERING_INTENT: &[(i128, &str)] = &[
    (0, "perceptual"),
    (1, "media-relative colorimetric"),
    (2, "saturation"),
    (3, "ICC-absolute colorimetric"),
];

/// Tag signatures from clause 9 of ICC.1:2022 and clause 6.4 of the version 2
/// spec, and two private tags that Apple's profiles carry.
const TAG: &[(i128, &str)] = &[
    (sig(b"A2B0"), "A2B0, device to PCS, perceptual"),
    (sig(b"A2B1"), "A2B1, device to PCS, colorimetric"),
    (sig(b"A2B2"), "A2B2, device to PCS, saturation"),
    (sig(b"B2A0"), "B2A0, PCS to device, perceptual"),
    (sig(b"B2A1"), "B2A1, PCS to device, colorimetric"),
    (sig(b"B2A2"), "B2A2, PCS to device, saturation"),
    (sig(b"B2D0"), "B2D0, PCS to device, perceptual, floating point"),
    (sig(b"B2D1"), "B2D1, PCS to device, media-relative, floating point"),
    (sig(b"B2D2"), "B2D2, PCS to device, saturation, floating point"),
    (sig(b"B2D3"), "B2D3, PCS to device, ICC-absolute, floating point"),
    (sig(b"D2B0"), "D2B0, device to PCS, perceptual, floating point"),
    (sig(b"D2B1"), "D2B1, device to PCS, media-relative, floating point"),
    (sig(b"D2B2"), "D2B2, device to PCS, saturation, floating point"),
    (sig(b"D2B3"), "D2B3, device to PCS, ICC-absolute, floating point"),
    (sig(b"rXYZ"), "rXYZ, red matrix column"),
    (sig(b"gXYZ"), "gXYZ, green matrix column"),
    (sig(b"bXYZ"), "bXYZ, blue matrix column"),
    (sig(b"rTRC"), "rTRC, red tone reproduction curve"),
    (sig(b"gTRC"), "gTRC, green tone reproduction curve"),
    (sig(b"bTRC"), "bTRC, blue tone reproduction curve"),
    (sig(b"kTRC"), "kTRC, grey tone reproduction curve"),
    (sig(b"wtpt"), "wtpt, media white point"),
    (sig(b"bkpt"), "bkpt, media black point"),
    (sig(b"calt"), "calt, calibration date and time"),
    (sig(b"targ"), "targ, characterisation target"),
    (sig(b"chad"), "chad, chromatic adaptation"),
    (sig(b"chrm"), "chrm, chromaticity"),
    (sig(b"cicp"), "cicp, coding-independent code points"),
    (sig(b"clro"), "clro, colorant order"),
    (sig(b"clrt"), "clrt, colorant table"),
    (sig(b"clot"), "clot, colorant table out"),
    (sig(b"ciis"), "ciis, colorimetric intent image state"),
    (sig(b"cprt"), "cprt, copyright"),
    (sig(b"desc"), "desc, profile description"),
    (sig(b"dmnd"), "dmnd, device manufacturer description"),
    (sig(b"dmdd"), "dmdd, device model description"),
    (sig(b"gamt"), "gamt, gamut"),
    (sig(b"lumi"), "lumi, luminance"),
    (sig(b"meas"), "meas, measurement"),
    (sig(b"meta"), "meta, metadata"),
    (sig(b"ncol"), "ncol, named colour"),
    (sig(b"ncl2"), "ncl2, named colour 2"),
    (sig(b"resp"), "resp, output response"),
    (sig(b"rig0"), "rig0, perceptual rendering intent gamut"),
    (sig(b"rig2"), "rig2, saturation rendering intent gamut"),
    (sig(b"pre0"), "pre0, preview 0"),
    (sig(b"pre1"), "pre1, preview 1"),
    (sig(b"pre2"), "pre2, preview 2"),
    (sig(b"pseq"), "pseq, profile sequence description"),
    (sig(b"psid"), "psid, profile sequence identifier"),
    (sig(b"tech"), "tech, technology"),
    (sig(b"vued"), "vued, viewing conditions description"),
    (sig(b"view"), "view, viewing conditions"),
    // Version 2 only.
    (sig(b"psd0"), "psd0, PostScript CRD 0"),
    (sig(b"psd1"), "psd1, PostScript CRD 1"),
    (sig(b"psd2"), "psd2, PostScript CRD 2"),
    (sig(b"psd3"), "psd3, PostScript CRD 3"),
    (sig(b"ps2s"), "ps2s, PostScript colour space array"),
    (sig(b"ps2i"), "ps2i, PostScript rendering intent"),
    (sig(b"scrd"), "scrd, screening description"),
    (sig(b"scrn"), "scrn, screening"),
    (sig(b"bfd "), "bfd, undercolour removal and black generation"),
    (sig(b"devs"), "devs, device settings"),
    (sig(b"crdi"), "crdi, CRD info"),
    // Private tags, not in either spec.
    (sig(b"vcgt"), "vcgt, video card gamma (Apple private tag)"),
    (sig(b"dscm"), "dscm, localised description (Apple private tag)"),
];

/// Tag type signatures, clause 10 of ICC.1:2022 and clause 6.5 of version 2.
const TYPE: &[(i128, &str)] = &[
    (sig(b"chrm"), "chrm, chromaticity"),
    (sig(b"cicp"), "cicp, coding-independent code points"),
    (sig(b"clro"), "clro, colorant order"),
    (sig(b"clrt"), "clrt, colorant table"),
    (sig(b"curv"), "curv, curve"),
    (sig(b"data"), "data, ASCII or binary data"),
    (sig(b"dtim"), "dtim, date and time"),
    (sig(b"dict"), "dict, dictionary"),
    (sig(b"mft2"), "mft2, 16-bit lookup table"),
    (sig(b"mft1"), "mft1, 8-bit lookup table"),
    (sig(b"mAB "), "mAB, A to B lookup"),
    (sig(b"mBA "), "mBA, B to A lookup"),
    (sig(b"meas"), "meas, measurement"),
    (sig(b"mluc"), "mluc, multi-localized Unicode"),
    (sig(b"mpet"), "mpet, multi-process elements"),
    (sig(b"ncl2"), "ncl2, named colour 2"),
    (sig(b"para"), "para, parametric curve"),
    (sig(b"pseq"), "pseq, profile sequence description"),
    (sig(b"psid"), "psid, profile sequence identifier"),
    (sig(b"rcs2"), "rcs2, response curve set"),
    (sig(b"sf32"), "sf32, s15Fixed16 array"),
    (sig(b"sig "), "sig, signature"),
    (sig(b"text"), "text, ASCII text"),
    (sig(b"uf32"), "uf32, u16Fixed16 array"),
    (sig(b"ui08"), "ui08, 8-bit integer array"),
    (sig(b"ui16"), "ui16, 16-bit integer array"),
    (sig(b"ui32"), "ui32, 32-bit integer array"),
    (sig(b"ui64"), "ui64, 64-bit integer array"),
    (sig(b"view"), "view, viewing conditions"),
    (sig(b"XYZ "), "XYZ, XYZ values"),
    // Version 2 only.
    (sig(b"desc"), "desc, text description"),
    (sig(b"ncol"), "ncol, named colour"),
    (sig(b"scrn"), "scrn, screening"),
    (sig(b"bfd "), "bfd, undercolour removal and black generation"),
    (sig(b"crdi"), "crdi, CRD info"),
    (sig(b"devs"), "devs, device settings"),
    (sig(b"vcgt"), "vcgt, video card gamma (Apple private type)"),
];

/// Device technologies, table 29, which is what a `tech` tag's signature is.
const TECHNOLOGY: &[(i128, &str)] = &[
    (sig(b"fscn"), "fscn, film scanner"),
    (sig(b"dcam"), "dcam, digital camera"),
    (sig(b"rscn"), "rscn, reflective scanner"),
    (sig(b"ijet"), "ijet, ink jet printer"),
    (sig(b"twax"), "twax, thermal wax printer"),
    (sig(b"epho"), "epho, electrophotographic printer"),
    (sig(b"esta"), "esta, electrostatic printer"),
    (sig(b"dsub"), "dsub, dye sublimation printer"),
    (sig(b"rpho"), "rpho, photographic paper printer"),
    (sig(b"fprn"), "fprn, film writer"),
    (sig(b"vidm"), "vidm, video monitor"),
    (sig(b"vidc"), "vidc, video camera"),
    (sig(b"pjtv"), "pjtv, projection television"),
    (sig(b"CRT "), "CRT, cathode ray tube display"),
    (sig(b"PMD "), "PMD, passive matrix display"),
    (sig(b"AMD "), "AMD, active matrix display"),
    (sig(b"LCD "), "LCD, liquid crystal display"),
    (sig(b"OLED"), "OLED, organic LED display"),
    (sig(b"KPCD"), "KPCD, Photo CD"),
    (sig(b"imgs"), "imgs, photographic image setter"),
    (sig(b"grav"), "grav, gravure"),
    (sig(b"offs"), "offs, offset lithography"),
    (sig(b"silk"), "silk, silkscreen"),
    (sig(b"flex"), "flex, flexography"),
    (sig(b"mpfs"), "mpfs, motion picture film scanner"),
    (sig(b"mpfr"), "mpfr, motion picture film recorder"),
    (sig(b"dmpc"), "dmpc, digital motion picture camera"),
    (sig(b"dcpj"), "dcpj, digital cinema projector"),
];

/// Colorimetric intent image states, table 26, which is what a `ciis` tag's
/// signature is.
const IMAGE_STATE: &[(i128, &str)] = &[
    (sig(b"scoe"), "scoe, scene colorimetry estimates"),
    (sig(b"sape"), "sape, scene appearance estimates"),
    (sig(b"fpce"), "fpce, focal plane colorimetry estimates"),
    (sig(b"rhoc"), "rhoc, reflection hardcopy original colorimetry"),
    (sig(b"rpoc"), "rpoc, reflection print output colorimetry"),
];

/// The parametric curve functions, table 68. `Y` is the output and `X` the
/// input; the letters are the parameters that follow.
const FUNCTION: &[(i128, &str)] = &[
    (0, "Y = X^g"),
    (1, "Y = (aX + b)^g for X ≥ -b/a, else 0 (CIE 122-1966)"),
    (2, "Y = (aX + b)^g + c for X ≥ -b/a, else c (IEC 61966-3)"),
    (3, "Y = (aX + b)^g for X ≥ d, else cX (IEC 61966-2-1, sRGB)"),
    (4, "Y = (aX + b)^g + e for X ≥ d, else cX + f"),
];

/// Standard observers, measurement type table 50.
const OBSERVER: &[(i128, &str)] = &[(0, "unknown"), (1, "CIE 1931 standard observer"), (2, "CIE 1964 standard observer")];

/// Measurement geometries, table 51.
const GEOMETRY: &[(i128, &str)] = &[(0, "unknown"), (1, "0°:45° or 45°:0°"), (2, "0°:d or d:0°")];

/// Standard illuminants, table 53, used by the measurement and viewing
/// conditions types.
const ILLUMINANT: &[(i128, &str)] = &[
    (0, "unknown"),
    (1, "D50"),
    (2, "D65"),
    (3, "D93"),
    (4, "F2"),
    (5, "D55"),
    (6, "A"),
    (7, "E, equi-power"),
    (8, "F8"),
];

/// Phosphor or colorant sets a chromaticity tag may name, table 31. Nought
/// means the coordinates that follow are the ones to use.
const COLORANTS: &[(i128, &str)] = &[
    (0, "unknown"),
    (1, "ITU-R BT.709-2"),
    (2, "SMPTE RP145"),
    (3, "EBU Tech. 3213-E"),
    (4, "P22"),
    (5, "P3"),
    (6, "ITU-R BT.2020"),
];

/// ITU-T H.273 colour primaries, shared with PNG's cICP chunk. Names from
/// H.273 and the code points FFmpeg lists for it.
pub(crate) const H273_PRIMARIES: &[(i128, &str)] = &[
    (1, "BT.709, sRGB"),
    (2, "unspecified"),
    (4, "BT.470 System M"),
    (5, "BT.470 System B, G (BT.601 625-line)"),
    (6, "SMPTE 170M (BT.601 525-line)"),
    (7, "SMPTE 240M"),
    (8, "generic film (illuminant C)"),
    (9, "BT.2020, BT.2100"),
    (10, "SMPTE ST 428-1 (CIE 1931 XYZ)"),
    (11, "SMPTE ST 431-2 (DCI-P3)"),
    (12, "SMPTE ST 432-1 (Display P3)"),
    (22, "EBU Tech. 3213-E"),
];

/// ITU-T H.273 transfer characteristics.
pub(crate) const H273_TRANSFER: &[(i128, &str)] = &[
    (1, "BT.709"),
    (2, "unspecified"),
    (4, "gamma 2.2 (BT.470 System M)"),
    (5, "gamma 2.8 (BT.470 System B, G)"),
    (6, "SMPTE 170M (BT.601)"),
    (7, "SMPTE 240M"),
    (8, "linear"),
    (9, "logarithmic, 100:1 range"),
    (10, "logarithmic, 100√10:1 range"),
    (11, "IEC 61966-2-4 (xvYCC)"),
    (12, "BT.1361 extended colour gamut"),
    (13, "IEC 61966-2-1 (sRGB)"),
    (14, "BT.2020, 10-bit"),
    (15, "BT.2020, 12-bit"),
    (16, "SMPTE ST 2084 (PQ)"),
    (17, "SMPTE ST 428-1"),
    (18, "ARIB STD-B67 (HLG)"),
];

/// ITU-T H.273 matrix coefficients.
pub(crate) const H273_MATRIX: &[(i128, &str)] = &[
    (0, "identity (RGB)"),
    (1, "BT.709"),
    (2, "unspecified"),
    (4, "FCC 73.682"),
    (5, "BT.470 System B, G (BT.601 625-line)"),
    (6, "SMPTE 170M (BT.601 525-line)"),
    (7, "SMPTE 240M"),
    (8, "YCgCo"),
    (9, "BT.2020 non-constant luminance"),
    (10, "BT.2020 constant luminance"),
    (11, "SMPTE ST 2085 (Y'D'zD'x)"),
    (12, "chromaticity-derived non-constant luminance"),
    (13, "chromaticity-derived constant luminance"),
    (14, "ICtCp (BT.2100)"),
    (15, "IPT-C2 (SMPTE ST 2128)"),
    (16, "YCgCo-R, even"),
    (17, "YCgCo-R, odd"),
];

/// ITU-T H.273's video full range flag.
pub(crate) const H273_RANGE: &[(i128, &str)] = &[(0, "narrow range"), (1, "full range")];

pub fn icc() -> Template {
    Template::new("icc", profile())
}

/// The name a format that carries a profile reads it by.
const PROFILE: &str = "icc.Profile";

/// The whole of a profile, for a format that carries one: a PNG's iCCP chunk,
/// a JPEG's APP2 segments and the profile joined from them, and a WebP's ICCP
/// chunk. The profile is a named type, so a template that reads it in several
/// places, as a PNG holding a JPEG thumbnail does, has it written once.
pub(crate) fn icc_part() -> Part {
    Part::new(T::Named(PROFILE.into())).with_type(PROFILE, profile())
}

/// Whether a file is a profile: `acsp` 36 bytes in, and a profile size at the
/// front that is the file's length and has room for the header and a tag
/// count.
pub(crate) fn is_icc(head: &[u8], len: u64) -> bool {
    profile_size(head).is_some_and(|size| size == len && size >= 132)
}

/// The same for a file whose name says it is a profile: the size may be less
/// than the file's length, for a profile with bytes after its end.
pub(crate) fn is_named_icc(head: &[u8], len: u64) -> bool {
    profile_size(head).is_some_and(|size| size <= len && size >= 132)
}

fn profile_size(head: &[u8]) -> Option<u64> {
    let size = head.get(..4)?;
    (head.get(36..40) == Some(b"acsp")).then(|| u32::from_be_bytes(size.try_into().expect("four bytes")) as u64)
}

/// The profile, in a window of its own so that the tag offsets count from its
/// first byte.
fn profile() -> T {
    T::sized(
        E::Remaining,
        T::structure(
            "ICCProfile",
            vec![
                ("header", header()),
                ("tag_count", T::u32(Big)),
                ("tags", T::array(entry(), E::field("tag_count"))),
            ],
        ),
    )
}

/// A four-letter signature that may be nought, read as its letters. A CMM,
/// manufacturer, model or creator signature is registered with the ICC, and
/// the letters are what a registry lookup takes.
fn signature_text() -> T {
    T::text(StrLen::Padded { size: E::lit(4), pad: 0 }, Encoding::Latin1)
}

/// The 128-byte header, table 17.
fn header() -> T {
    T::structure(
        "Header",
        vec![
            ("profile_size", T::u32(Big)),
            ("preferred_cmm", signature_text()),
            ("version", version()),
            ("device_class", T::enumeration_hex("DeviceClass", T::u32(Big), DEVICE_CLASS)),
            ("colour_space", T::enumeration_hex("ColourSpace", T::u32(Big), COLOUR_SPACE)),
            ("pcs", T::enumeration_hex("ColourSpace", T::u32(Big), COLOUR_SPACE)),
            ("created", date_time()),
            ("signature", T::magic(b"acsp")),
            ("platform", T::enumeration_hex("Platform", T::u32(Big), PLATFORM)),
            ("flags", T::flags("ProfileFlags", T::u32(Big), &[(0, "embedded"), (1, "use with embedded data only")])),
            ("manufacturer", signature_text()),
            ("model", signature_text()),
            (
                "attributes",
                T::flags(
                    "DeviceAttributes",
                    T::u64(Big),
                    &[(0, "transparency"), (1, "matte"), (2, "negative"), (3, "black and white")],
                ),
            ),
            ("rendering_intent", T::enumeration("RenderingIntent", T::u32(Big), RENDERING_INTENT)),
            ("illuminant", xyz_number()),
            ("creator", signature_text()),
            ("profile_id", T::bytes(E::lit(16))),
            ("reserved", T::bytes(E::lit(28))),
        ],
    )
    .field_doc("profile_size", "The size of the whole profile in bytes, header, tag table and tag data included.")
    .field_doc("preferred_cmm", "The preferred colour management module (CMM), as a registered signature. Four zero bytes when no CMM is named.")
    .field_doc("colour_space", "The data colour space, such as RGB or CMYK. In a device link, the first device's colour space.")
    .field_doc("pcs", "The profile connection space, XYZ or Lab. In a device link, the second device's colour space.")
    .field_doc("created", "When the profile was first created, in UTC.")
    .field_doc("flags", "Bit 0 is set when the profile is embedded in a file. Bit 1 is set when it must only be used with the colour data it is embedded with.")
    .field_doc("attributes", "Bits 0 to 3 describe the media, clear or set: bit 0 reflective or transparency, bit 1 glossy or matte, bit 2 positive or negative, bit 3 colour or black and white.")
    .field_doc("illuminant", "The PCS illuminant, which the spec fixes at D50: X 0.9642, Y 1.0, Z 0.8249.")
    .field_doc("profile_id", "MD5 hash of the whole profile, calculated with the flags, rendering intent and profile ID fields taken as zero. All 16 bytes are zero when no ID was calculated.")
    .field_doc("reserved", "28 bytes reserved by the ICC, which must be zero.")
}

/// The version, written as binary-coded decimal: the major version in the
/// first byte and the minor and bug fix versions a nibble each in the second.
/// 4.4.0 is `04 40 00 00`.
fn version() -> T {
    T::inline_structure(
        "Version",
        vec![
            ("major", T::u8()),
            ("minor", T::UInt { bits: 4, endian: Big }),
            ("bugfix", T::UInt { bits: 4, endian: Big }),
            ("reserved", T::u16(Big)),
        ],
    )
    .reads_as(&[("major", "major", ""), ("minor", "minor", ""), ("bugfix", "bug fix", "")])
}

/// A dateTimeNumber: six 16-bit numbers, UTC.
fn date_time() -> T {
    let names = ["year", "month", "day", "hour", "minute", "second"];
    T::structure("dateTimeNumber", names.iter().map(|n| (*n, T::u16(Big))).collect())
}

/// A fixed-point number: `stored` is what the file holds, `worth` what it
/// stands for, `stored` over `scale`.
fn fixed(name: &str, stored: T, scale: f64) -> T {
    T::inline_structure(name, vec![("stored", stored), ("worth", T::computed_real(E::field("stored").div(E::real(scale))))])
        .payload(&["stored"])
        .reads_as(&[("worth", "", "")])
}

/// s15Fixed16Number: signed, with 16 bits after the point, so 0x00010000 is
/// 1.0.
fn s15() -> T {
    fixed("s15Fixed16Number", T::i32(Big), 65536.0)
}

/// u16Fixed16Number: the same, unsigned.
fn u16f16() -> T {
    fixed("u16Fixed16Number", T::u32(Big), 65536.0)
}

/// u8Fixed8Number: unsigned, with 8 bits after the point, so 0x0100 is 1.0.
fn u8f8() -> T {
    fixed("u8Fixed8Number", T::u16(Big), 256.0)
}

/// An XYZNumber: three s15Fixed16Numbers.
fn xyz_number() -> T {
    T::inline_structure("XYZNumber", vec![("x", s15()), ("y", s15()), ("z", s15())])
        .reads_as(&[("x", "X", ""), ("y", "Y", ""), ("z", "Z", "")])
}

/// One entry of the tag table, and the data it points at.
fn entry() -> T {
    T::structure_named(
        "TagEntry",
        "tag",
        "data",
        vec![
            ("tag", T::enumeration_hex("TagSignature", T::u32(Big), TAG)),
            ("offset", T::u32(Big)),
            ("size", T::u32(Big)),
            // The data, wherever the entry says it is. A window of its own,
            // because the offsets inside a tag count from its first byte.
            ("data", T::at_in_window(E::field("offset"), T::sized(E::field("size"), element()))),
        ],
    )
    .counted_as("tag")
    .field_doc("offset", "Where the tag's data starts, counted from the first byte of the profile.")
    .field_doc("size", "Size of the tag's data in bytes. The spec leaves out the padding after the data, but some writers count it.")
}

/// A tag's data: its type signature, four reserved bytes, and what the type
/// says follows.
fn element() -> T {
    T::structure_named(
        "TagData",
        "",
        "contents",
        vec![
            ("type", T::enumeration_hex("TagType", T::u32(Big), TYPE)),
            ("reserved", T::u32(Big)),
            ("contents", T::switch(E::field("type"), contents(), T::bytes(E::Remaining))),
            // Not after the types that keep strings or tables at offsets of
            // their own: the bytes after their records are what those offsets
            // point at.
            ("padding", T::switch(E::field("type"), PLACES_ITS_OWN.iter().map(|t| (*t, T::bytes(E::lit(0)))).collect(), T::bytes(E::Remaining))),
        ],
    )
    .field_doc("padding", "Bytes after the tag's contents that the tag's size still counts, usually padding to a four-byte boundary.")
}

/// The tag types whose contents are records with offsets to the rest.
const PLACES_ITS_OWN: &[i128] = &[sig(b"mluc"), sig(b"dict"), sig(b"mAB "), sig(b"mBA ")];

/// What each tag type reads as. A type not named here is its bytes.
fn contents() -> Vec<(i128, T)> {
    let array = |name: &str, field: &str, elem: T, width: i128| {
        T::structure_named(name, "", field, vec![(field, T::array(elem, E::Remaining.div(E::lit(width))))])
    };
    vec![
        (sig(b"desc"), text_description()),
        (sig(b"mluc"), mluc()),
        (sig(b"text"), T::structure_named("textType", "", "text", vec![("text", padded_ascii(E::Remaining))])),
        (sig(b"XYZ "), xyz_type()),
        (sig(b"curv"), curve()),
        (sig(b"para"), parametric()),
        (sig(b"sf32"), array("s15Fixed16ArrayType", "values", s15(), 4)),
        (sig(b"uf32"), array("u16Fixed16ArrayType", "values", u16f16(), 4)),
        (sig(b"ui08"), array("uInt8ArrayType", "values", T::u8(), 1)),
        (sig(b"ui16"), array("uInt16ArrayType", "values", T::u16(Big), 2)),
        (sig(b"ui32"), array("uInt32ArrayType", "values", T::u32(Big), 4)),
        (sig(b"ui64"), array("uInt64ArrayType", "values", T::u64(Big), 8)),
        (sig(b"sig "), signature_type()),
        (sig(b"meas"), measurement()),
        (sig(b"view"), viewing_conditions()),
        (sig(b"chrm"), chromaticity()),
        (sig(b"dtim"), T::structure("dateTimeType", vec![("date_time", date_time())])),
        (sig(b"data"), data()),
        (sig(b"cicp"), cicp()),
        (sig(b"clro"), colorant_order()),
        (sig(b"clrt"), colorant_table()),
        (sig(b"ncl2"), named_colour_2()),
        (sig(b"mft2"), lut16()),
        (sig(b"mft1"), lut8()),
        (sig(b"mAB "), lut_ab(true)),
        (sig(b"mBA "), lut_ab(false)),
        (sig(b"dict"), dict()),
        (sig(b"vcgt"), vcgt()),
    ]
}

/// 7-bit ASCII ending at its first nul, in a field of `size` bytes.
fn padded_ascii(size: E) -> T {
    T::text(StrLen::Padded { size, pad: 0 }, Encoding::Ascii)
}

/// textDescriptionType, version 2's description: an ASCII string, then the
/// same text in Unicode and in a Macintosh ScriptCode, each with a count in
/// front. Many writers leave the last two empty, and some stop after the
/// ASCII, so each part is read only where the tag has room for it.
fn text_description() -> T {
    T::structure(
        "textDescriptionType",
        vec![
            ("ascii_count", T::u32(Big)),
            ("ascii", padded_ascii(E::field("ascii_count"))),
            ("unicode_language", T::if_room(T::u32(Big))),
            ("unicode_count", T::if_room(T::u32(Big))),
            (
                "unicode",
                T::if_room(T::text(StrLen::Padded { size: E::field("unicode_count").mul(E::lit(2)), pad: 0 }, Encoding::Utf16(Big))),
            ),
            ("scriptcode_code", T::if_room(T::u16(Big))),
            ("scriptcode_count", T::if_room(T::u8())),
            ("scriptcode", T::if_room(T::text(StrLen::Padded { size: E::lit(67), pad: 0 }, Encoding::Latin1))),
        ],
    )
    .field_doc("ascii_count", "Bytes of ASCII text, its nul included.")
    .field_doc("unicode_count", "UTF-16 code units of text, two bytes each, its nul included.")
    .field_doc("scriptcode", "67 bytes, of which the first scriptcode_count are the Macintosh description.")
}

/// multiLocalizedUnicodeType: a list of records, each a language and country
/// and where in the tag its UTF-16 string is.
fn mluc() -> T {
    let record = T::structure_named(
        "LocalizedString",
        "",
        "string",
        vec![
            ("language", T::text(StrLen::Padded { size: E::lit(2), pad: 0 }, Encoding::Ascii)),
            ("country", T::text(StrLen::Padded { size: E::lit(2), pad: 0 }, Encoding::Ascii)),
            ("length", T::u32(Big)),
            ("offset", T::u32(Big)),
            ("string", T::at_in_window(E::field("offset"), T::text(StrLen::Fixed(E::field("length")), Encoding::Utf16(Big)))),
        ],
    )
    .counted_as("string");
    T::structure(
        "multiLocalizedUnicodeType",
        vec![("record_count", T::u32(Big)), ("record_size", T::u32(Big)), ("records", T::array(record, E::field("record_count")))],
    )
    .field_doc("record_size", "Bytes per record: 12.")
}

/// XYZType: one XYZNumber for nearly every tag, and a list of them where a
/// tag holds several.
fn xyz_type() -> T {
    let count = E::Remaining.div(E::lit(12));
    T::structure_named(
        "XYZType",
        "",
        "xyz",
        vec![("xyz", T::switch(count.clone(), vec![(1, xyz_number())], T::array(xyz_number(), count)))],
    )
}

/// curveType: a count, then that many 16-bit entries. A count of 1 is a gamma
/// and a count of 0 is the identity curve.
fn curve() -> T {
    let count = || E::field("count");
    T::structure(
        "curveType",
        vec![
            ("count", T::u32(Big)),
            ("gamma", T::when(count().equal_to(E::lit(1)), u8f8())),
            ("entries", T::when(count().greater_than(E::lit(1)), T::array(T::u16(Big), count()))),
        ],
    )
    .field_doc("count", "Number of entries. 0 means the identity curve, and with 1 the entry is the gamma exponent.")
    .field_doc("gamma", "The exponent: Y = X^gamma.")
    .field_doc("entries", "Y values of the curve at evenly spaced X from 0.0 to 1.0. Each Y value is stored from 0 for 0.0 to 65535 for 1.0.")
}

/// parametricCurveType: which function, then its parameters, of which there
/// are one, three, four, five or seven.
fn parametric() -> T {
    // Defined only for the five function types; an undefined one reads `g`
    // and leaves the rest.
    let from = |n: i128| {
        E::field("function").greater_or_equal(E::lit(n)).both(E::field("function").less_or_equal(E::lit(4)))
    };
    T::structure(
        "parametricCurveType",
        vec![
            ("function", T::enumeration("ParametricFunction", T::u16(Big), FUNCTION)),
            ("reserved", T::u16(Big)),
            ("g", s15()),
            ("a", T::when(from(1), s15())),
            ("b", T::when(from(1), s15())),
            ("c", T::when(from(2), s15())),
            ("d", T::when(from(3), s15())),
            ("e", T::when(from(4), s15())),
            ("f", T::when(from(4), s15())),
        ],
    )
}

/// signatureType: one four-letter signature. What it is a signature of is up
/// to the tag, so a `tech` tag's reads as a technology, a `ciis` tag's as an
/// image state, and any other as its letters.
fn signature_type() -> T {
    let named = vec![
        (sig(b"tech"), T::enumeration_hex("Technology", T::u32(Big), TECHNOLOGY)),
        (sig(b"ciis"), T::enumeration_hex("ImageState", T::u32(Big), IMAGE_STATE)),
    ];
    T::structure_named("signatureType", "", "signature", vec![("signature", T::switch(E::field("tag"), named, signature_text()))])
}

/// measurementType: the conditions the profile's data was measured under.
fn measurement() -> T {
    T::structure(
        "measurementType",
        vec![
            ("observer", T::enumeration("StandardObserver", T::u32(Big), OBSERVER)),
            ("backing", xyz_number()),
            ("geometry", T::enumeration("MeasurementGeometry", T::u32(Big), GEOMETRY)),
            ("flare", u16f16()),
            ("illuminant", T::enumeration("StandardIlluminant", T::u32(Big), ILLUMINANT)),
        ],
    )
    .field_doc("backing", "The nCIEXYZ tristimulus values of the measurement backing.")
    .field_doc("flare", "Measurement flare, from 0.0 for 0% to 1.0 for 100%.")
}

/// viewingConditionsType: the illuminant and surround, in cd/m², and the
/// illuminant's type.
fn viewing_conditions() -> T {
    T::structure(
        "viewingConditionsType",
        vec![
            ("illuminant", xyz_number()),
            ("surround", xyz_number()),
            ("illuminant_type", T::enumeration("StandardIlluminant", T::u32(Big), ILLUMINANT)),
        ],
    )
    .field_doc("illuminant", "Un-normalized CIEXYZ of the illuminant, with Y in cd/m².")
    .field_doc("surround", "Un-normalized CIEXYZ of the surround, with Y in cd/m².")
}

/// chromaticityType: the CIE xy coordinates of each device channel, and which
/// standard set of phosphors or colorants they are, if any.
fn chromaticity() -> T {
    let xy = T::inline_structure("xy", vec![("x", u16f16()), ("y", u16f16())]).reads_as(&[("x", "x", ""), ("y", "y", "")]);
    T::structure(
        "chromaticityType",
        vec![
            ("channel_count", T::u16(Big)),
            ("colorant", T::enumeration("Colorant", T::u16(Big), COLORANTS)),
            ("channels", T::array(xy, E::field("channel_count"))),
        ],
    )
}

/// dataType: a flag saying ASCII or binary, then the data.
fn data() -> T {
    T::structure(
        "dataType",
        vec![
            ("flag", T::enumeration("DataFlag", T::u32(Big), &[(0, "ASCII"), (1, "binary")])),
            ("data", T::switch(E::field("flag"), vec![(0, padded_ascii(E::Remaining))], T::bytes(E::Remaining))),
        ],
    )
}

/// cicpType: the four ITU-T H.273 code points PNG's cICP chunk holds too.
fn cicp() -> T {
    T::structure(
        "cicpType",
        vec![
            ("colour_primaries", T::enumeration("ColourPrimaries", T::u8(), H273_PRIMARIES)),
            ("transfer_characteristics", T::enumeration("TransferCharacteristics", T::u8(), H273_TRANSFER)),
            ("matrix_coefficients", T::enumeration("MatrixCoefficients", T::u8(), H273_MATRIX)),
            ("video_full_range_flag", T::enumeration("VideoFullRange", T::u8(), H273_RANGE)),
        ],
    )
}

/// colorantOrderType: the order the colorants are laid down in, as channel
/// numbers.
fn colorant_order() -> T {
    T::structure("colorantOrderType", vec![("count", T::u32(Big)), ("order", T::array(T::u8(), E::field("count")))])
}

/// colorantTableType: each colorant's name and its PCS value.
fn colorant_table() -> T {
    let colorant = T::structure(
        "Colorant",
        vec![("name", padded_ascii(E::lit(32))), ("pcs", T::array(T::u16(Big), E::lit(3)))],
    )
    .counted_as("colorant");
    T::structure("colorantTableType", vec![("count", T::u32(Big)), ("colorants", T::array(colorant, E::field("count")))])
}

/// namedColor2Type: a list of named colours, each with its PCS value and as
/// many device coordinates as the tag says.
fn named_colour_2() -> T {
    let colour = T::structure(
        "NamedColour",
        vec![
            ("name", padded_ascii(E::lit(32))),
            ("pcs", T::array(T::u16(Big), E::lit(3))),
            ("device", T::array(T::u16(Big), E::field("device_coordinates"))),
        ],
    )
    .counted_as("colour");
    T::structure(
        "namedColor2Type",
        vec![
            ("vendor_flags", T::u32(Big)),
            ("count", T::u32(Big)),
            ("device_coordinates", T::u32(Big)),
            ("prefix", padded_ascii(E::lit(32))),
            ("suffix", padded_ascii(E::lit(32))),
            ("colours", T::array(colour, E::field("count"))),
        ],
    )
}

/// The 3 by 3 matrix of a version 2 lookup table, e1 to e9 by rows.
fn matrix(n: usize) -> T {
    let names = ["e1", "e2", "e3", "e4", "e5", "e6", "e7", "e8", "e9", "e10", "e11", "e12"];
    T::structure("Matrix", names[..n].iter().map(|e| (*e, s15())).collect())
}

/// lut16Type: a matrix, the input tables, a lookup table of grid points to
/// the power of the input channels times the output channels, and the output
/// tables, every entry 16 bits.
fn lut16() -> T {
    let input = E::field("input_entries").mul(E::field("input_channels"));
    let output = E::field("output_entries").mul(E::field("output_channels"));
    T::structure(
        "lut16Type",
        vec![
            ("input_channels", T::u8()),
            ("output_channels", T::u8()),
            ("grid_points", T::u8()),
            ("padding", T::u8()),
            ("matrix", matrix(9)),
            ("input_entries", T::u16(Big)),
            ("output_entries", T::u16(Big)),
            ("input_tables", T::array(T::u16(Big), input)),
            // grid_points to the power input_channels times output_channels,
            // which is what is left once the output tables are set aside.
            ("clut", T::array(T::u16(Big), E::Remaining.div(E::lit(2)).sub(output.clone()))),
            ("output_tables", T::array(T::u16(Big), output)),
        ],
    )
    .field_doc("matrix", "A 3 by 3 matrix applied before the input tables. It must be the identity unless the input colour space is XYZ.")
}

/// lut8Type: the same with 8-bit entries and 256 entries in every input and
/// output table.
fn lut8() -> T {
    let input = E::lit(256).mul(E::field("input_channels"));
    let output = E::lit(256).mul(E::field("output_channels"));
    T::structure(
        "lut8Type",
        vec![
            ("input_channels", T::u8()),
            ("output_channels", T::u8()),
            ("grid_points", T::u8()),
            ("padding", T::u8()),
            ("matrix", matrix(9)),
            ("input_tables", T::array(T::u8(), input)),
            ("clut", T::array(T::u8(), E::Remaining.sub(output.clone()))),
            ("output_tables", T::array(T::u8(), output)),
        ],
    )
    .field_doc("matrix", "A 3 by 3 matrix applied before the input tables. It must be the identity unless the input colour space is XYZ.")
}

/// lutAToBType and lutBToAType: five offsets from the start of the tag, each
/// to a processing element or nought when there is none, and the elements
/// read where the offsets say. The two types differ in the order the elements
/// apply and in which channels each set of curves has.
///
/// The curves are as many as their side's channels: in an mAB, the A curves
/// are one per input channel and the M and B curves one per output channel;
/// in an mBA, the B and M curves are one per input channel and the A curves
/// one per output channel.
fn lut_ab(a_to_b: bool) -> T {
    let (inputs, outputs) = (|| E::field("input_channels"), || E::field("output_channels"));
    let (a, m, b) = if a_to_b { (inputs(), outputs(), outputs()) } else { (outputs(), inputs(), inputs()) };
    let placed = |offset: &str, ty: T| T::when(E::field(offset).not_equal(E::lit(0)), T::at_in_window(E::field(offset), ty));
    T::structure(
        if a_to_b { "lutAToBType" } else { "lutBToAType" },
        vec![
            ("input_channels", T::u8()),
            ("output_channels", T::u8()),
            ("padding", T::u16(Big)),
            ("b_curves_offset", T::u32(Big)),
            ("matrix_offset", T::u32(Big)),
            ("m_curves_offset", T::u32(Big)),
            ("clut_offset", T::u32(Big)),
            ("a_curves_offset", T::u32(Big)),
            ("b_curves", placed("b_curves_offset", curves(b))),
            ("matrix", placed("matrix_offset", matrix(12))),
            ("m_curves", placed("m_curves_offset", curves(m))),
            ("clut", placed("clut_offset", clut())),
            ("a_curves", placed("a_curves_offset", curves(a))),
        ],
    )
    .field_doc("matrix", "A 3 by 3 matrix, e1 to e9 by rows, then the offsets e10 to e12.")
}

/// A run of curves, each a whole curv or para element with its own type
/// signature, padded to a four-byte boundary.
fn curves(count: E) -> T {
    let curve = T::structure_named(
        "Curve",
        "",
        "contents",
        vec![
            ("type", T::enumeration_hex("TagType", T::u32(Big), TYPE)),
            ("reserved", T::u32(Big)),
            ("contents", T::switch(E::field("type"), vec![(sig(b"curv"), curve()), (sig(b"para"), parametric())], T::bytes(E::lit(0)))),
            // Up to three bytes, read as the zero they should be.
            (
                "padding",
                T::uint_expr(
                    E::size_of("type").add(E::size_of("reserved")).add(E::size_of("contents")).pad_to(4).at_most(E::Remaining).mul(E::lit(8)),
                    Big,
                ),
            ),
        ],
    )
    .counted_as("curve");
    T::array(curve, count)
}

/// The multi-dimensional lookup table of an mAB or mBA: the grid points along
/// each input dimension, how wide each value is, and the values.
fn clut() -> T {
    // Only the first input-channels entries of the sixteen are used, and the
    // rest are nought, so the product of the non-zero ones is the number of
    // grid points.
    let points = (0..16).fold(E::lit(1), |acc, k| {
        let g = || E::elem("grid_points", E::lit(k));
        acc.mul(E::cond(g().equal_to(E::lit(0)), E::lit(1), g()))
    });
    let count = points.mul(E::field("output_channels"));
    T::structure(
        "CLUT",
        vec![
            ("grid_points", T::array(T::u8(), E::lit(16))),
            ("precision", T::u8()),
            ("padding", T::UInt { bits: 24, endian: Big }),
            ("values", T::switch(E::field("precision"), vec![(1, T::array(T::u8(), count.clone()))], T::array(T::u16(Big), count))),
        ],
    )
    .field_doc("grid_points", "Number of grid points along each input dimension. Only the first input_channels entries are used.")
    .field_doc("precision", "Bytes per value: 1 or 2.")
}

/// dictType: name and value pairs, each a pair of UTF-16 strings with an
/// optional display name and display value that are whole mluc elements. The
/// records hold offsets from the start of the tag and sizes, and are 16, 24
/// or 32 bytes long as the tag says: the display name's pair is there from 24
/// and the display value's from 32.
///
/// An mluc inside a dict counts its own string offsets from its own first
/// byte, the way LittleCMS reads one, so each sits in a window of its own.
fn dict() -> T {
    let size = || E::field("record_length");
    let present = |offset: &str| E::field(offset).not_equal(E::lit(0));
    let utf16 = |offset: &str, size: &str| {
        T::when(present(offset), T::at_in_window(E::field(offset), T::text(StrLen::Fixed(E::field(size)), Encoding::Utf16(Big))))
    };
    let localized = |offset: &str, size: &str| {
        let embedded = T::structure_named(
            "DisplayText",
            "",
            "contents",
            vec![
                ("type", T::enumeration_hex("TagType", T::u32(Big), TYPE)),
                ("reserved", T::u32(Big)),
                ("contents", T::switch(E::field("type"), vec![(sig(b"mluc"), mluc())], T::bytes(E::Remaining))),
            ],
        );
        let shown = present(offset).both(E::field(size).not_equal(E::lit(0)));
        T::when(shown, T::at_in_window(E::field(offset), T::sized(E::field(size), embedded)))
    };
    let record = T::structure_named(
        "NameValue",
        "name",
        "",
        vec![
            ("name_offset", T::u32(Big)),
            ("name_size", T::u32(Big)),
            ("value_offset", T::u32(Big)),
            ("value_size", T::u32(Big)),
            ("display_name_offset", T::when(size().greater_or_equal(E::lit(24)), T::u32(Big))),
            ("display_name_size", T::when(size().greater_or_equal(E::lit(24)), T::u32(Big))),
            ("display_value_offset", T::when(size().greater_or_equal(E::lit(32)), T::u32(Big))),
            ("display_value_size", T::when(size().greater_or_equal(E::lit(32)), T::u32(Big))),
            ("name", utf16("name_offset", "name_size")),
            ("value", utf16("value_offset", "value_size")),
            ("display_name", T::when(size().greater_or_equal(E::lit(24)), localized("display_name_offset", "display_name_size"))),
            ("display_value", T::when(size().greater_or_equal(E::lit(32)), localized("display_value_offset", "display_value_size"))),
        ],
    )
    .counted_as("entry");
    T::structure(
        "dictType",
        vec![("record_count", T::u32(Big)), ("record_length", T::u32(Big)), ("records", T::array(record, E::field("record_count")))],
    )
    .field_doc("record_length", "Bytes in each record: 16, 24 or 32.")
}

/// vcgt: Apple's video card gamma, a private type the ICC spec does not
/// define. Either a table of ramps, one per channel, or a gamma, a minimum
/// and a maximum for each of red, green and blue, laid out as LittleCMS's
/// `Type_vcgt_Read` reads them.
fn vcgt() -> T {
    let ramp = || T::switch(E::field("entry_size"), vec![(1, T::u8())], T::u16(Big));
    let table = T::structure(
        "VideoCardGammaTable",
        vec![
            ("channels", T::u16(Big)),
            ("entry_count", T::u16(Big)),
            ("entry_size", T::u16(Big)),
            ("ramps", T::array(T::array(ramp(), E::field("entry_count")), E::field("channels"))),
        ],
    )
    .field_doc("entry_size", "Bytes per entry: 1 or 2.");
    let names = ["red_gamma", "red_min", "red_max", "green_gamma", "green_min", "green_max", "blue_gamma", "blue_min", "blue_max"];
    let formula = T::structure("VideoCardGammaFormula", names.iter().map(|n| (*n, s15())).collect());
    T::structure(
        "vcgtType",
        vec![
            ("form", T::enumeration("VideoCardGammaForm", T::u32(Big), &[(0, "table"), (1, "formula")])),
            ("gamma", T::switch(E::field("form"), vec![(0, table), (1, formula)], T::bytes(E::Remaining))),
        ],
    )
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    /// A profile holding these tags, laid out after the table in order and
    /// padded to four bytes each. A tag whose data is `None` shares the data
    /// of the tag before it.
    pub(crate) fn profile_bytes(version: u8, tags: &[(&[u8; 4], Option<Vec<u8>>)]) -> Vec<u8> {
        let mut head = vec![0u8; 128];
        head[4..8].copy_from_slice(b"lcms");
        head[8] = version;
        head[9] = 0x40;
        head[12..16].copy_from_slice(b"mntr");
        head[16..20].copy_from_slice(b"RGB ");
        head[20..24].copy_from_slice(b"XYZ ");
        for (i, v) in [2026u16, 11, 26, 9, 30, 5].iter().enumerate() {
            head[24 + 2 * i..26 + 2 * i].copy_from_slice(&v.to_be_bytes());
        }
        head[36..40].copy_from_slice(b"acsp");
        head[40..44].copy_from_slice(b"APPL");
        head[44..48].copy_from_slice(&1u32.to_be_bytes());
        head[56..64].copy_from_slice(&0b1001u64.to_be_bytes());
        head[64..68].copy_from_slice(&1u32.to_be_bytes());
        for (i, v) in [0x0000_f6d6i32, 0x0001_0000, 0x0000_d32d].iter().enumerate() {
            head[68 + 4 * i..72 + 4 * i].copy_from_slice(&v.to_be_bytes());
        }
        let mut table = (tags.len() as u32).to_be_bytes().to_vec();
        let mut body = Vec::new();
        let start = 128 + 4 + 12 * tags.len();
        let mut last = (0, 0);
        for (sig, data) in tags {
            if let Some(data) = data {
                last = (start + body.len(), data.len());
                body.extend_from_slice(data);
                while body.len() % 4 != 0 {
                    body.push(0);
                }
            }
            table.extend_from_slice(*sig);
            table.extend_from_slice(&(last.0 as u32).to_be_bytes());
            table.extend_from_slice(&(last.1 as u32).to_be_bytes());
        }
        let mut b = head;
        b.extend_from_slice(&table);
        b.extend_from_slice(&body);
        let len = b.len() as u32;
        b[..4].copy_from_slice(&len.to_be_bytes());
        b
    }

    /// A tag's data: its type signature, four reserved bytes, and `rest`.
    pub(crate) fn tag(kind: &[u8; 4], rest: &[u8]) -> Vec<u8> {
        let mut v = kind.to_vec();
        v.extend_from_slice(&[0; 4]);
        v.extend_from_slice(rest);
        v
    }

    fn s15(v: f64) -> [u8; 4] {
        ((v * 65536.0).round() as i32).to_be_bytes()
    }

    fn down(ev: &mut Evaluator, d: &Document<MemSource>, names: &[&str]) -> Vec<usize> {
        let mut at = Vec::new();
        for name in names {
            if name.is_empty() {
                at.push(0);
            } else if let Ok(i) = name.parse::<usize>() {
                at.push(i);
            } else {
                at = ev.child_named(d, &at, name).unwrap().unwrap_or_else(|| panic!("no {name} under {at:?}"));
            }
        }
        at
    }

    fn info(ev: &mut Evaluator, d: &Document<MemSource>, names: &[&str]) -> crate::eval::NodeInfo {
        let at = down(ev, d, names);
        ev.node(d, &at).unwrap_or_else(|e| panic!("{names:?}: {e:?}"))
    }

    fn value(ev: &mut Evaluator, d: &Document<MemSource>, names: &[&str]) -> Value {
        info(ev, d, names).value
    }

    fn real(ev: &mut Evaluator, d: &Document<MemSource>, names: &[&str]) -> f64 {
        match value(ev, d, names) {
            Value::Float(f) => f,
            other => panic!("{names:?} is {other:?}"),
        }
    }

    fn text(ev: &mut Evaluator, d: &Document<MemSource>, names: &[&str]) -> String {
        match value(ev, d, names) {
            Value::Str(s) => s,
            other => panic!("{names:?} is {other:?}"),
        }
    }

    fn named(ev: &mut Evaluator, d: &Document<MemSource>, names: &[&str]) -> Option<String> {
        match value(ev, d, names) {
            Value::Enum { name, .. } => name,
            other => panic!("{names:?} is {other:?}"),
        }
    }

    /// The contents of tag `i`, as a path to go on from.
    fn data(i: usize) -> Vec<&'static str> {
        let i: &'static str = Box::leak(i.to_string().into_boxed_str());
        vec!["tags", i, "data", "", "contents"]
    }

    fn at<'a>(base: &[&'a str], more: &[&'a str]) -> Vec<&'a str> {
        [base, more].concat()
    }

    #[test]
    fn the_header_reads_its_signatures_version_and_illuminant() {
        let d = Document::new(MemSource(profile_bytes(4, &[])));
        let mut ev = Evaluator::new(icc());
        assert_eq!(text(&mut ev, &d, &["header", "preferred_cmm"]), "lcms");
        assert_eq!(value(&mut ev, &d, &["header", "version", "major"]), Value::UInt(4));
        assert_eq!(value(&mut ev, &d, &["header", "version", "minor"]), Value::UInt(4));
        assert_eq!(named(&mut ev, &d, &["header", "device_class"]).as_deref(), Some("mntr, display device"));
        assert_eq!(named(&mut ev, &d, &["header", "colour_space"]).as_deref(), Some("RGB"));
        assert_eq!(named(&mut ev, &d, &["header", "platform"]).as_deref(), Some("Apple"));
        assert_eq!(named(&mut ev, &d, &["header", "rendering_intent"]).as_deref(), Some("media-relative colorimetric"));
        assert_eq!(value(&mut ev, &d, &["header", "created", "year"]), Value::UInt(2026));
        assert_eq!(text(&mut ev, &d, &["header", "manufacturer"]), "");
        let Value::Flags { set, .. } = value(&mut ev, &d, &["header", "attributes"]) else { panic!("attributes are flags") };
        assert_eq!(set, ["transparency", "black and white"]);
        let x = real(&mut ev, &d, &["header", "illuminant", "x", "worth"]);
        assert!((x - 0.9642).abs() < 0.0001, "{x}");
        assert_eq!(value(&mut ev, &d, &["header", "illuminant", "y", "stored"]), Value::Int(65536));
        assert_eq!(real(&mut ev, &d, &["header", "illuminant", "y", "worth"]), 1.0);
    }

    #[test]
    fn tags_that_share_data_each_read_it() {
        let mut para = 3u16.to_be_bytes().to_vec();
        para.extend_from_slice(&[0, 0]);
        for v in [2.4, 1.0 / 1.055, 0.055 / 1.055, 1.0 / 12.92, 0.04045] {
            para.extend_from_slice(&s15(v));
        }
        let mut xyz = Vec::new();
        for v in [0.4361, 0.2225, 0.0139] {
            xyz.extend_from_slice(&s15(v));
        }
        let d = Document::new(MemSource(profile_bytes(
            4,
            &[(b"rXYZ", Some(tag(b"XYZ ", &xyz))), (b"rTRC", Some(tag(b"para", &para))), (b"gTRC", None), (b"bTRC", None)],
        )));
        let mut ev = Evaluator::new(icc());
        assert_eq!(value(&mut ev, &d, &["tag_count"]), Value::UInt(4));
        assert_eq!(named(&mut ev, &d, &["tags", "1", "tag"]).as_deref(), Some("rTRC, red tone reproduction curve"));
        let x = real(&mut ev, &d, &at(&data(0), &["xyz", "x", "worth"]));
        assert!((x - 0.4361).abs() < 0.0001, "{x}");
        for i in 1..4 {
            let base = data(i);
            assert_eq!(named(&mut ev, &d, &at(&base[..4], &["type"])).as_deref(), Some("para, parametric curve"));
            let g = real(&mut ev, &d, &at(&base, &["g", "worth"]));
            assert!((g - 2.4).abs() < 0.0001, "{g}");
            let dd = real(&mut ev, &d, &at(&base, &["d", "worth"]));
            assert!((dd - 0.04045).abs() < 0.0001, "{dd}");
            assert!(info(&mut ev, &d, &at(&base, &["e"])).absent);
        }
        // The three entries point at one place.
        let a = down(&mut ev, &d, &at(&data(1), &["g"]));
        let b = down(&mut ev, &d, &at(&data(3), &["g"]));
        assert_eq!(ev.node(&d, &a).unwrap().offset_bits, ev.node(&d, &b).unwrap().offset_bits);
    }

    #[test]
    fn mluc_strings_are_read_from_where_their_records_say_in_the_tag() {
        let mut m = 2u32.to_be_bytes().to_vec();
        m.extend_from_slice(&12u32.to_be_bytes());
        let strings: [&str; 2] = ["sRGB", "sRVB"];
        let mut at_ = 16 + 12 * 2;
        let mut store = Vec::new();
        for (lang, s) in [(b"enUS", strings[0]), (b"frFR", strings[1])] {
            let units: Vec<u8> = s.encode_utf16().flat_map(|u| u.to_be_bytes()).collect();
            m.extend_from_slice(lang);
            m.extend_from_slice(&(units.len() as u32).to_be_bytes());
            m.extend_from_slice(&(at_ as u32).to_be_bytes());
            at_ += units.len();
            store.extend_from_slice(&units);
        }
        m.extend_from_slice(&store);
        let d = Document::new(MemSource(profile_bytes(4, &[(b"desc", Some(tag(b"mluc", &m)))])));
        let mut ev = Evaluator::new(icc());
        let records = at(&data(0), &["records"]);
        assert_eq!(text(&mut ev, &d, &at(&records, &["0", "language"])), "en");
        assert_eq!(text(&mut ev, &d, &at(&records, &["1", "country"])), "FR");
        assert_eq!(text(&mut ev, &d, &at(&records, &["0", "string", ""])), "sRGB");
        assert_eq!(text(&mut ev, &d, &at(&records, &["1", "string", ""])), "sRVB");
    }

    #[test]
    fn a_version_2_description_reads_all_three_texts_and_stops_where_the_tag_does() {
        let mut desc = 5u32.to_be_bytes().to_vec();
        desc.extend_from_slice(b"sRGB\0");
        desc.extend_from_slice(&0u32.to_be_bytes());
        desc.extend_from_slice(&5u32.to_be_bytes());
        desc.extend("sRGB\0".encode_utf16().flat_map(|u| u.to_be_bytes()));
        desc.extend_from_slice(&0u16.to_be_bytes());
        desc.push(0);
        desc.extend_from_slice(&[0; 67]);
        let short = {
            let mut v = 5u32.to_be_bytes().to_vec();
            v.extend_from_slice(b"Gray\0");
            v
        };
        let d = Document::new(MemSource(profile_bytes(2, &[(b"desc", Some(tag(b"desc", &desc))), (b"dmdd", Some(tag(b"desc", &short)))])));
        let mut ev = Evaluator::new(icc());
        assert_eq!(text(&mut ev, &d, &at(&data(0), &["ascii"])), "sRGB");
        assert_eq!(text(&mut ev, &d, &at(&data(0), &["unicode"])), "sRGB");
        assert_eq!(text(&mut ev, &d, &at(&data(0), &["scriptcode"])), "");
        assert_eq!(text(&mut ev, &d, &at(&data(1), &["ascii"])), "Gray");
        assert_eq!(info(&mut ev, &d, &at(&data(1), &["unicode_count"])).size_bits, 0);
    }

    #[test]
    fn curves_read_as_a_gamma_or_a_table_and_text_types_as_text() {
        let gamma = [1u32.to_be_bytes().to_vec(), 0x0233u16.to_be_bytes().to_vec()].concat();
        let mut table = 3u32.to_be_bytes().to_vec();
        for v in [0u16, 0x8000, 0xffff] {
            table.extend_from_slice(&v.to_be_bytes());
        }
        let d = Document::new(MemSource(profile_bytes(
            2,
            &[
                (b"rTRC", Some(tag(b"curv", &gamma))),
                (b"kTRC", Some(tag(b"curv", &table))),
                (b"cprt", Some(tag(b"text", b"No copyright, use freely\0"))),
                (b"tech", Some(tag(b"sig ", b"CRT "))),
                (b"ciis", Some(tag(b"sig ", b"scoe"))),
                (b"meta", Some(tag(b"sig ", b"abcd"))),
                (b"calt", Some(tag(b"dtim", &[0x07, 0xea, 0, 11, 0, 26, 0, 12, 0, 0, 0, 0]))),
            ],
        )));
        let mut ev = Evaluator::new(icc());
        assert_eq!(real(&mut ev, &d, &at(&data(0), &["gamma", "worth"])), 0x233 as f64 / 256.0);
        assert!(info(&mut ev, &d, &at(&data(0), &["entries"])).absent);
        assert_eq!(value(&mut ev, &d, &at(&data(1), &["entries", "1"])), Value::UInt(0x8000));
        assert_eq!(text(&mut ev, &d, &at(&data(2), &["text"])), "No copyright, use freely");
        assert_eq!(named(&mut ev, &d, &at(&data(3), &["signature"])).as_deref(), Some("CRT, cathode ray tube display"));
        assert_eq!(named(&mut ev, &d, &at(&data(4), &["signature"])).as_deref(), Some("scoe, scene colorimetry estimates"));
        assert_eq!(text(&mut ev, &d, &at(&data(5), &["signature"])), "abcd");
        assert_eq!(value(&mut ev, &d, &at(&data(6), &["date_time", "year"])), Value::UInt(2026));
    }

    #[test]
    fn measurement_viewing_chromaticity_and_cicp_tags_read_their_fields() {
        let mut meas = 1u32.to_be_bytes().to_vec();
        meas.extend_from_slice(&[0; 12]);
        meas.extend_from_slice(&2u32.to_be_bytes());
        meas.extend_from_slice(&0x0000_8000u32.to_be_bytes());
        meas.extend_from_slice(&2u32.to_be_bytes());
        let mut view = Vec::new();
        for v in [19.6445, 20.3718, 16.8089, 3.92889, 4.07439, 3.36179] {
            view.extend_from_slice(&s15(v));
        }
        view.extend_from_slice(&1u32.to_be_bytes());
        let mut chrm = 3u16.to_be_bytes().to_vec();
        chrm.extend_from_slice(&1u16.to_be_bytes());
        for v in [0.64, 0.33, 0.30, 0.60, 0.15, 0.06] {
            chrm.extend_from_slice(&(((v * 65536.0) as f64).round() as u32).to_be_bytes());
        }
        let d = Document::new(MemSource(profile_bytes(
            4,
            &[
                (b"meas", Some(tag(b"meas", &meas))),
                (b"view", Some(tag(b"view", &view))),
                (b"chrm", Some(tag(b"chrm", &chrm))),
                (b"cicp", Some(tag(b"cicp", &[1, 13, 0, 1]))),
            ],
        )));
        let mut ev = Evaluator::new(icc());
        assert_eq!(named(&mut ev, &d, &at(&data(0), &["observer"])).as_deref(), Some("CIE 1931 standard observer"));
        assert_eq!(real(&mut ev, &d, &at(&data(0), &["flare", "worth"])), 0.5);
        assert_eq!(named(&mut ev, &d, &at(&data(0), &["illuminant"])).as_deref(), Some("D65"));
        let y = real(&mut ev, &d, &at(&data(1), &["illuminant", "y", "worth"]));
        assert!((y - 20.3718).abs() < 0.0001, "{y}");
        assert_eq!(named(&mut ev, &d, &at(&data(1), &["illuminant_type"])).as_deref(), Some("D50"));
        assert_eq!(named(&mut ev, &d, &at(&data(2), &["colorant"])).as_deref(), Some("ITU-R BT.709-2"));
        let gy = real(&mut ev, &d, &at(&data(2), &["channels", "1", "y", "worth"]));
        assert!((gy - 0.60).abs() < 0.0001, "{gy}");
        assert_eq!(named(&mut ev, &d, &at(&data(3), &["transfer_characteristics"])).as_deref(), Some("IEC 61966-2-1 (sRGB)"));
        assert_eq!(named(&mut ev, &d, &at(&data(3), &["video_full_range_flag"])).as_deref(), Some("full range"));
    }

    #[test]
    fn lookup_tables_read_their_tables_and_unknown_types_read_as_bytes() {
        // A lut16 with one input, one output, two grid points and two entries
        // in each table.
        let mut mft2 = vec![1, 1, 2, 0];
        for i in 0..9 {
            mft2.extend_from_slice(&s15(if i % 4 == 0 { 1.0 } else { 0.0 }));
        }
        mft2.extend_from_slice(&2u16.to_be_bytes());
        mft2.extend_from_slice(&2u16.to_be_bytes());
        for v in [0u16, 0xffff, 0x1000, 0xf000, 0, 0xffff] {
            mft2.extend_from_slice(&v.to_be_bytes());
        }
        // An mAB with B curves only: one input, one output, the identity curve.
        let mut mab = vec![1, 1, 0, 0];
        mab.extend_from_slice(&32u32.to_be_bytes());
        mab.extend_from_slice(&[0; 16]);
        mab.extend_from_slice(&tag(b"curv", &0u32.to_be_bytes()));
        let d = Document::new(MemSource(profile_bytes(
            4,
            &[(b"A2B0", Some(tag(b"mft2", &mft2))), (b"A2B1", Some(tag(b"mAB ", &mab))), (b"arts", Some(tag(b"abcd", &[1, 2, 3])))],
        )));
        let mut ev = Evaluator::new(icc());
        assert_eq!(info(&mut ev, &d, &at(&data(0), &["clut"])).child_count, 2);
        assert_eq!(value(&mut ev, &d, &at(&data(0), &["output_tables", "1"])), Value::UInt(0xffff));
        assert_eq!(real(&mut ev, &d, &at(&data(0), &["matrix", "e5", "worth"])), 1.0);
        let curve = at(&data(1), &["b_curves", "", "0"]);
        assert_eq!(named(&mut ev, &d, &at(&curve, &["type"])).as_deref(), Some("curv, curve"));
        assert_eq!(value(&mut ev, &d, &at(&curve, &["contents", "count"])), Value::UInt(0));
        assert!(info(&mut ev, &d, &at(&data(1), &["clut"])).absent);
        let unknown = info(&mut ev, &d, &data(2));
        assert_eq!(unknown.size_bits, 3 * 8);
    }

    #[test]
    fn dictionaries_video_card_gamma_and_counted_padding_read_their_fields() {
        // One 16-byte record: name "k" and value "v", each two bytes of UTF-16.
        let mut dict = 1u32.to_be_bytes().to_vec();
        dict.extend_from_slice(&16u32.to_be_bytes());
        for v in [32u32, 2, 36, 2] {
            dict.extend_from_slice(&v.to_be_bytes());
        }
        dict.extend_from_slice(&[0, b'k', 0, 0, 0, b'v']);
        // Two channels of three 16-bit entries.
        let mut vcgt = 0u32.to_be_bytes().to_vec();
        for v in [2u16, 3, 2, 0, 0x8000, 0xffff, 0, 0x4000, 0xffff] {
            vcgt.extend_from_slice(&v.to_be_bytes());
        }
        // A gamma curve whose size counts the two bytes of padding after it.
        let mut curv = tag(b"curv", &[0, 0, 0, 1, 1, 0]);
        curv.extend_from_slice(&[0, 0]);
        let d = Document::new(MemSource(profile_bytes(
            4,
            &[(b"meta", Some(tag(b"dict", &dict))), (b"vcgt", Some(tag(b"vcgt", &vcgt))), (b"kTRC", Some(curv))],
        )));
        let mut ev = Evaluator::new(icc());
        let record = at(&data(0), &["records", "0"]);
        assert_eq!(text(&mut ev, &d, &at(&record, &["name", ""])), "k");
        assert_eq!(text(&mut ev, &d, &at(&record, &["value", ""])), "v");
        assert!(info(&mut ev, &d, &at(&record, &["display_name"])).absent);
        assert_eq!(named(&mut ev, &d, &at(&data(1), &["form"])).as_deref(), Some("table"));
        assert_eq!(value(&mut ev, &d, &at(&data(1), &["gamma", "ramps", "1", "1"])), Value::UInt(0x4000));
        assert_eq!(real(&mut ev, &d, &at(&data(2), &["gamma", "worth"])), 1.0);
        let mut padding = down(&mut ev, &d, &data(2));
        padding.pop();
        let padding = ev.child_named(&d, &padding, "padding").unwrap().unwrap();
        assert_eq!(ev.node(&d, &padding).unwrap().size_bits, 2 * 8);
    }

    #[test]
    fn a_profile_is_recognised_by_its_signature_and_its_size() {
        let b = profile_bytes(2, &[]);
        assert!(is_icc(&b, b.len() as u64));
        assert!(!is_icc(&b, b.len() as u64 + 1));
        assert!(is_named_icc(&b, b.len() as u64 + 1));
        let mut wrong = b.clone();
        wrong[36] = b'b';
        assert!(!is_icc(&wrong, wrong.len() as u64));
    }
}
