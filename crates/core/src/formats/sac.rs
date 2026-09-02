//! SAC, the binary format of Seismic Analysis Code: a header of exactly 632
//! bytes and then the samples.
//!
//! The header is three runs of fixed slots and nothing else: seventy floats,
//! forty integer words, and twenty-four strings of eight characters, of which
//! the second is sixteen because the event name was given two slots. Every
//! slot exists in every file, and one nobody filled in holds -12345, or
//! `-12345` written out in a string. There is no length, no tag and nothing
//! optional, which is what keeps a format from 1978 readable.
//!
//! Nothing in the file says which way round its numbers are. What says it is
//! `nvhdr`, the header version, which is 6 or 7 and reads as 100,663,296 or
//! 117,440,512 the wrong way round: the template peeks at that word both ways
//! and lays the whole file out in whichever answers plausibly. Every reader
//! of this format does the same, because it is all there is to go on.
//!
//! A version 7 file appends a footer of doubles after the samples, so that
//! the fields needing more precision than a float has can have it without
//! anything moving. The footer is left as bytes here: which of the floats it
//! restates is a list as long as the header.

use crate::template::{Encoding, Endian, Endian::*, Expr as E, StrLen, Template, Ty as T};

/// Where `nvhdr` is written, which is the one field that says how to read the
/// rest. Word 6 of the integers, and the integers start at word 70.
pub const NVHDR_AT: usize = (70 + 6) * 4;

/// Where `npts` is written: word 9 of the integers.
pub const NPTS_AT: usize = (70 + 9) * 4;

/// How long the header is, and so where the samples start.
const HEADER: i128 = 632;

/// The value a slot nobody filled in holds, whatever it means. The strings
/// hold it spelled out.
const UNSET: i128 = -12345;

/// The seventy floats, in the order the format fixes them in. Everything
/// physical about the trace is one of these.
///
/// `delta` is the sample spacing in seconds and `b` and `e` the value of the
/// independent variable at the first and last sample. `o` is the origin time
/// of the event and `a` the first arrival, both in seconds from time zero,
/// and `t0` to `t9` are picks somebody or something else put on the trace.
/// `resp0` to `resp9` are the instrument response, which SAC carries as ten
/// numbers and never says the shape of. `stla` through `stdp` place the
/// station, `evla` through `evdp` the event, and `dist`, `az`, `baz` and
/// `gcarc` are the distance and bearings between them. `user0` to `user9`
/// are the format's room for whoever is using it.
const FLOATS: &[&str] = &[
    "delta", "depmin", "depmax", "scale", "odelta", "b", "e", "o", "a", "internal0",
    "t0", "t1", "t2", "t3", "t4", "t5", "t6", "t7", "t8", "t9",
    "f", "resp0", "resp1", "resp2", "resp3", "resp4", "resp5", "resp6", "resp7", "resp8",
    "resp9", "stla", "stlo", "stel", "stdp", "evla", "evlo", "evel", "evdp", "mag",
    "user0", "user1", "user2", "user3", "user4", "user5", "user6", "user7", "user8", "user9",
    "dist", "az", "baz", "gcarc", "internal1", "internal2", "depmen", "cmpaz", "cmpinc", "xminimum",
    "xmaximum", "yminimum", "ymaximum", "unused6", "unused7", "unused8", "unused9", "unused10", "unused11", "unused12",
];

/// The forty integer words. `nzyear` through `nzmsec` are time zero as a GMT
/// calendar time, and every field with a time in it is seconds from there.
/// `npts` is how many samples follow the header, `nvhdr` the header version.
/// The `i` fields are enumerated and are given their own types below; the
/// four `l` fields are true or false, written as whole 32-bit words.
const INTS: &[&str] = &[
    "nzyear", "nzjday", "nzhour", "nzmin", "nzsec", "nzmsec", "nvhdr", "norid", "nevid", "npts",
    "internal3", "nwfid", "nxsize", "nysize", "unused13", "iftype", "idep", "iztype", "unused14", "iinst",
    "istreg", "ievreg", "ievtyp", "iqual", "isynth", "imagtyp", "imagsrc", "unused15", "unused16", "unused17",
    "unused18", "unused19", "unused20", "unused21", "unused22", "leven", "lpspol", "lovrok", "lcalda", "unused23",
];

/// The strings, in order. All eight characters and space padded, except
/// `kevnm`, which was given two slots and so is sixteen. `kstnm`, `knetwk`,
/// `khole` and `kcmpnm` are the four names that say which channel of which
/// station in which network this is; the rest label the times beside them.
const STRINGS: &[(&str, i128)] = &[
    ("kstnm", 8),
    ("kevnm", 16),
    ("khole", 8),
    ("ko", 8),
    ("ka", 8),
    ("kt0", 8),
    ("kt1", 8),
    ("kt2", 8),
    ("kt3", 8),
    ("kt4", 8),
    ("kt5", 8),
    ("kt6", 8),
    ("kt7", 8),
    ("kt8", 8),
    ("kt9", 8),
    ("kf", 8),
    ("kuser0", 8),
    ("kuser1", 8),
    ("kuser2", 8),
    ("kcmpnm", 8),
    ("knetwk", 8),
    ("kdatrd", 8),
    ("kinst", 8),
];

/// The fields whose values are named rather than counted, and the names.
/// SAC writes the value of `itime` and the like as the number the format
/// assigned it, and those numbers are shared across every one of these
/// fields: 5 is `iunkn` wherever it appears, 44 is `iother`.
const ENUMERATED: &[(&str, &str, &[(i128, &str)])] = &[
    ("iftype", "SacFileType", FILE_TYPE),
    ("idep", "SacDependent", DEPENDENT),
    ("iztype", "SacZeroTime", ZERO_TIME),
    ("ievtyp", "SacEventType", EVENT_TYPE),
    ("iqual", "SacQuality", QUALITY),
    ("isynth", "SacSynthetic", SYNTHETIC),
    ("imagtyp", "SacMagnitudeType", MAGNITUDE_TYPE),
    ("imagsrc", "SacMagnitudeSource", MAGNITUDE_SOURCE),
];

/// What the file holds, which decides how many arrays of samples follow the
/// header.
const FILE_TYPE: &[(i128, &str)] = &[
    (1, "itime: time series"),
    (2, "irlim: spectrum, real and imaginary"),
    (3, "iamph: spectrum, amplitude and phase"),
    (4, "ixy: x against y"),
    (51, "ixyz: three-dimensional"),
    (UNSET, "unset"),
];

/// What the samples measure.
const DEPENDENT: &[(i128, &str)] = &[
    (5, "iunkn: unknown"),
    (6, "idisp: displacement (nm)"),
    (7, "ivel: velocity (nm/s)"),
    (8, "iacc: acceleration (nm/s2)"),
    (50, "ivolts: velocity (volts)"),
    (UNSET, "unset"),
];

/// What time zero of the file is.
const ZERO_TIME: &[(i128, &str)] = &[
    (5, "iunkn: unknown"),
    (9, "ib: begin time"),
    (10, "iday: midnight of the reference day"),
    (11, "io: origin time"),
    (12, "ia: first arrival"),
    (13, "it0"),
    (14, "it1"),
    (15, "it2"),
    (16, "it3"),
    (17, "it4"),
    (18, "it5"),
    (19, "it6"),
    (20, "it7"),
    (21, "it8"),
    (22, "it9"),
    (UNSET, "unset"),
];

/// What happened, for a file cut around an event.
const EVENT_TYPE: &[(i128, &str)] = &[
    (5, "iunkn: unknown"),
    (37, "inucl: nuclear event"),
    (38, "ipren: nuclear pre-shot"),
    (39, "ipostn: nuclear post-shot"),
    (40, "iquake: earthquake"),
    (41, "ipreq: foreshock"),
    (42, "ipostq: aftershock"),
    (43, "ichem: chemical explosion"),
    (44, "iother"),
    (72, "iqb: quarry blast"),
    (77, "ieq: earthquake"),
    (80, "ime: mining event"),
    (81, "iex: explosion"),
    (UNSET, "unset"),
];

/// How good the trace is thought to be.
const QUALITY: &[(i128, &str)] = &[
    (45, "igood"),
    (46, "iglch: glitches"),
    (47, "idrop: dropouts"),
    (48, "ilowsn: low signal to noise"),
    (44, "iother"),
    (UNSET, "unset"),
];

/// Whether the trace was recorded or made up.
const SYNTHETIC: &[(i128, &str)] = &[(49, "irldta: real data"), (UNSET, "unset")];

/// Which magnitude scale `mag` is on.
const MAGNITUDE_TYPE: &[(i128, &str)] = &[
    (52, "imb: body wave"),
    (53, "ims: surface wave"),
    (54, "iml: local"),
    (55, "imw: moment"),
    (56, "imd: duration"),
    (57, "imx: user defined"),
    (UNSET, "unset"),
];

/// Who said what the magnitude was.
const MAGNITUDE_SOURCE: &[(i128, &str)] = &[
    (58, "ineic"),
    (59, "ipdeq"),
    (60, "ipdew"),
    (61, "ipde"),
    (62, "iisc"),
    (63, "ireb"),
    (64, "iusgs"),
    (65, "ibrk"),
    (66, "icaltech"),
    (67, "illnl"),
    (68, "ievloc"),
    (69, "ijsop"),
    (70, "iuser"),
    (71, "iunknown"),
    (UNSET, "unset"),
];

pub fn sac() -> Template {
    // The header version read big-endian: 6 or 7 in a big-endian file, and a
    // number in the hundred millions in a little-endian one.
    let nvhdr = E::peek_at(E::lit(NVHDR_AT as i128 * 8), 32, Big);
    let plausible = nvhdr.clone().less_than(E::lit(8)).mul(E::lit(5).less_than(nvhdr));
    Template::new("sac", T::switch(plausible, vec![(1, file(Big))], file(Little)))
}

fn file(e: Endian) -> T {
    let mut fields: Vec<(&str, T)> = Vec::with_capacity(FLOATS.len() + INTS.len() + STRINGS.len() + 1);
    // A float slot holds -12345.0 when nobody filled it in, and it is still a
    // float: the type column says `f32 be` and only the value reads as unset.
    // A one-case enum said it the other way round, and enums have to sit on
    // integers, so the floats could not say it at all.
    fields.extend(FLOATS.iter().map(|name| (*name, T::unset_float(T::F32(e), UNSET as f64))));
    fields.extend(INTS.iter().map(|name| {
        let ty = match ENUMERATED.iter().find(|(field, ..)| field == name) {
            Some((_, ty_name, cases)) => T::enumeration(ty_name, T::i32(e), cases),
            None if name.starts_with('l') => {
                T::enumeration("SacLogical", T::i32(e), &[(0, "false"), (1, "true"), (UNSET, "unset")])
            }
            None => T::unset_int(T::i32(e), UNSET),
        };
        (*name, ty)
    }));
    fields.extend(
        STRINGS
            .iter()
            .map(|(name, size)| (*name, T::text(StrLen::Padded { size: E::lit(*size), pad: b' ' }, Encoding::Ascii))),
    );
    fields.push(("data", data(e)));
    T::structure("SAC", fields)
        .machinery(&["internal0", "internal1", "internal2", "internal3"])
        .payload(&["delta", "npts", "b", "e"])
}

/// The samples. How many arrays of them there are is what `iftype` and
/// `leven` between them say: a spectrum holds two components, and so does a
/// trace whose samples are not evenly spaced, since then every sample needs
/// its own x written beside it.
///
/// A version 7 file has a footer of doubles after all of that, which lands in
/// `footer` because the arrays are counted rather than run to the end.
fn data(e: Endian) -> T {
    let pair = |first: &'static str, second: &'static str| {
        T::structure("SacData", vec![(first, samples(e)), (second, samples(e)), ("footer", T::bytes(E::Remaining))])
    };
    let single = T::structure("SacData", vec![("y", samples(e)), ("footer", T::bytes(E::Remaining))]);
    T::switch(
        E::field("iftype"),
        vec![(2, pair("real", "imaginary")), (3, pair("amplitude", "phase"))],
        // Unevenly spaced samples are the values and then the times, whatever
        // the file type says the file is.
        T::switch(E::field("leven"), vec![(0, pair("y", "x"))], single),
    )
}

fn samples(e: Endian) -> T {
    // A file cut off short still reads what is there, and `npts` of -12345 is
    // a header nobody filled in: it clamps to nothing rather than counting
    // backwards.
    let count = E::field("npts").at_least(E::lit(0)).at_most(E::Remaining.div(E::lit(4)));
    T::array(T::F32(e), count)
}

/// A SAC file, told by its header version. `nvhdr` is 6 or 7 and is written
/// at a fixed place, so a file that reads as one of those in either byte
/// order, with a sample count that fits in the file, is one.
///
/// There is nothing else to go on: the format has no magic, and its first
/// bytes are a sample spacing, which can be any number at all.
pub fn is_sac(head: &[u8], len: u64) -> bool {
    if len < HEADER as u64 {
        return false;
    }
    let word = |at: usize, big: bool| -> Option<i64> {
        let b: [u8; 4] = head.get(at..at + 4)?.try_into().ok()?;
        Some(if big { i32::from_be_bytes(b) } else { i32::from_le_bytes(b) } as i64)
    };
    [true, false].into_iter().any(|big| {
        let (Some(nvhdr), Some(npts)) = (word(NVHDR_AT, big), word(NPTS_AT, big)) else { return false };
        // The samples have to fit. A spectral file holds two of them per
        // point and a version 7 file a footer as well, so this is the floor
        // rather than the whole size.
        (nvhdr == 6 || nvhdr == 7) && npts > 0 && HEADER as u64 + npts as u64 * 4 <= len
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    /// Where each run of slots starts, as a field index.
    const FIRST_INT: usize = 70;
    const FIRST_STRING: usize = 110;
    const DATA: usize = 133;

    /// A minimal file: the header slot by slot, and `npts` samples.
    fn build(big: bool, npts: i32, iftype: i32, leven: i32) -> Vec<u8> {
        let i32b = |v: i32| if big { v.to_be_bytes() } else { v.to_le_bytes() };
        let f32b = |v: f32| if big { v.to_be_bytes() } else { v.to_le_bytes() };
        let mut v = Vec::new();
        for i in 0..70 {
            v.extend_from_slice(&f32b(if i == 0 { 0.01 } else { -12345.0 }));
        }
        let mut ints = [-12345i32; 40];
        ints[0] = 1978; // nzyear
        ints[6] = 6; // nvhdr
        ints[9] = npts;
        ints[15] = iftype;
        ints[35] = leven;
        for n in ints {
            v.extend_from_slice(&i32b(n));
        }
        v.extend_from_slice(b"BGLD    ");
        v.extend_from_slice(b"-12345          ");
        for _ in 0..21 {
            v.extend_from_slice(b"-12345  ");
        }
        assert_eq!(v.len(), HEADER as usize);
        for i in 0..npts.max(0) {
            v.extend_from_slice(&f32b(i as f32));
        }
        v
    }

    #[test]
    fn the_header_is_632_bytes_and_the_samples_follow_it() {
        for big in [true, false] {
            let d = Document::new(MemSource(build(big, 4, 1, 1)));
            let mut ev = Evaluator::new(sac());
            assert_eq!(ev.node(&d, &[FIRST_INT]).unwrap().value.as_int(), Some(1978));
            assert_eq!(ev.node(&d, &[FIRST_INT + 6]).unwrap().value.as_int(), Some(6), "nvhdr, big={big}");
            assert_eq!(ev.node(&d, &[FIRST_STRING]).unwrap().value, Value::Str("BGLD".into()));
            let data = ev.node(&d, &[DATA]).unwrap();
            assert_eq!(data.offset_bits, HEADER as u64 * 8, "the data starts where the header ends");
            assert_eq!(ev.node(&d, &[DATA, 0]).unwrap().child_count, 4);
        }
    }

    /// A slot nobody filled in reads as unset rather than as a number the
    /// reader has to recognise, and says so for a float as well as for an
    /// integer. The type column still says what the field is: an unset slot is
    /// an `i32` or an `f32` holding a sentinel, not a type of its own.
    #[test]
    fn an_unfilled_slot_reads_as_unset() {
        let d = Document::new(MemSource(build(false, 4, 1, 1)));
        let mut ev = Evaluator::new(sac());
        let nzjday = ev.node(&d, &[FIRST_INT + 1]).unwrap();
        assert_eq!(nzjday.value, Value::Unset(Box::new(Value::Int(UNSET))));
        assert_eq!(nzjday.type_name, "i32 le");
        // `depmin`, the second float slot, which this file leaves alone.
        let depmin = ev.node(&d, &[1]).unwrap();
        assert_eq!(depmin.value, Value::Unset(Box::new(Value::Float(UNSET as f64))));
        assert_eq!(depmin.type_name, "f32 le");
        // A slot that was filled in is the number it holds, and nothing has
        // been wrapped away: `delta` is the sample spacing.
        assert_eq!(ev.node(&d, &[0]).unwrap().value, Value::Float(0.01));
        // Unset or not, the field is still one an expression can read and one
        // the reader can edit.
        assert_eq!(nzjday.value.as_int(), Some(UNSET));
        assert!(depmin.editable);
    }

    /// Unevenly spaced samples are two arrays: the values, and then the times
    /// they were taken at.
    #[test]
    fn uneven_spacing_stores_the_times_as_well() {
        let d = Document::new(MemSource(build(false, 4, 4, 0)));
        let mut ev = Evaluator::new(sac());
        assert_eq!(ev.node(&d, &[DATA]).unwrap().child_count, 3); // y, x, footer
        assert_eq!(ev.node(&d, &[DATA, 1]).unwrap().child_count, 0, "only four samples were written");
    }

    #[test]
    fn recognised_either_way_round_and_not_otherwise() {
        assert!(is_sac(&build(true, 4, 1, 1), 632 + 16));
        assert!(is_sac(&build(false, 4, 1, 1), 632 + 16));
        assert!(!is_sac(&[0u8; 700], 700));
    }
}
