//! The values of two real NASA CDF files, against what `cdflib` reads out of
//! them. The fixtures live in QUBERO_SAMPLES/cdf, or in the sibling
//! qubero-samples collection.
//!
//! Every number below came from `cdflib.CDF(path).varget(name)` and
//! `.globalattsget()`, which is the reference implementation's own Python
//! reader, MIT licensed and independent of anything here. What is checked is a
//! few values per variable, the first and the last, which is what catches a
//! byte order read backwards, a shape multiplied out wrongly, or a block found
//! at the wrong offset.
//!
//! The two files between them cover the cases that matter. The Parker Solar
//! Probe one is uncompressed, network byte order, with two of its six
//! variables in gzip blocks. The FAST one is a whole file squeezed with CDF's
//! run-length coding, written by an IBM PC, with gzip blocks inside that.

use std::path::PathBuf;

use qubero_core::{
    document::Document,
    eval::{Evaluator, Moment, TimeInfo, TimeNote, Value},
    formats,
    source::MemSource,
};

fn cdf_samples() -> Option<PathBuf> {
    qubero_samples::dir("cdf")
}

/// One file, opened, with the paths a reader of it needs worked out.
struct Cdf {
    doc: Document<MemSource>,
    ev: Evaluator,
    /// The body of the global descriptor, which is where both chains start.
    gdr: Vec<usize>,
}

impl Cdf {
    fn open(path: &PathBuf) -> Cdf {
        let doc = Document::new(MemSource(std::fs::read(path).unwrap()));
        let mut ev = Evaluator::new(formats::builtin("cdf").unwrap());
        // A compressed file holds the whole of itself in one record, and the
        // descriptor record is inside that rather than at the front.
        let first = ev.node(&doc, &[3, 2]).unwrap();
        let cdr: Vec<usize> = match first.type_name.as_str() {
            "CdfCompressed" => vec![3, 2, 4, 0, 1, 2],
            _ => vec![3, 2],
        };
        assert_eq!(ev.node(&doc, &cdr).unwrap().type_name, "CdfDescriptor", "{}", path.display());
        let mut gdr = cdr.clone();
        gdr.extend([11, 0, 2]);
        assert_eq!(ev.node(&doc, &gdr).unwrap().type_name, "CdfGlobalDescriptor");
        Cdf { doc, ev, gdr }
    }

    fn value(&mut self, at: &[usize]) -> Value {
        self.ev.node(&self.doc, at).unwrap_or_else(|e| panic!("{at:?}: {e:?}")).value.clone()
    }

    fn text(&mut self, at: &[usize]) -> String {
        match self.value(at) {
            Value::Str(s) => s,
            other => panic!("{at:?} is not text: {other:?}"),
        }
    }

    fn count(&mut self, at: &[usize]) -> usize {
        self.ev.node(&self.doc, at).unwrap_or_else(|e| panic!("{at:?}: {e:?}")).child_count as usize
    }

    fn under(&self, at: &[usize], more: &[usize]) -> Vec<usize> {
        let mut p = at.to_vec();
        p.extend(more);
        p
    }

    /// The field of this name under `at`.
    ///
    /// By name rather than by number, because the number moves: a version 2
    /// variable descriptor has a field a version 3 one does not, and an
    /// rVariable has two fewer than a zVariable.
    fn field(&mut self, at: &[usize], want: &str) -> Vec<usize> {
        for i in 0..self.count(at) {
            let p = self.under(at, &[i]);
            if self.ev.node(&self.doc, &p).unwrap().name == want {
                return p;
            }
        }
        panic!("{at:?} has no field called {want}");
    }

    /// The descriptor of the variable of this name, in whichever of the two
    /// lists it is in.
    fn variable(&mut self, want: &str) -> Vec<usize> {
        for which in [15, 14] {
            let list = self.under(&self.gdr.clone(), &[which]);
            for i in 0..self.count(&list) {
                let vdr = self.under(&list, &[i, 2]);
                let name = self.field(&vdr, "name");
                // A version 2 file pads its names out with spaces, so the name
                // in the file is `SST     ` and the variable is called SST.
                if self.text(&name).trim_end() == want {
                    return vdr;
                }
            }
        }
        panic!("no variable called {want}");
    }

    /// Every value of a variable, in the order the file writes them: the index
    /// records in turn, the blocks each one names, the records in a block, and
    /// the values in a record.
    fn values(&mut self, name: &str) -> Vec<Value> {
        let vdr = self.variable(name);
        let index = self.field(&vdr, "values_index");
        let mut out = Vec::new();
        for i in 0..self.count(&index) {
            let blocks = self.under(&index, &[i, 2, 6]);
            for b in 0..self.count(&blocks) {
                let record = self.under(&blocks, &[b, 3, 0]);
                // A compressed block keeps its values inside the stream; an
                // uncompressed one is the values and nothing else. Either way
                // the records are the second field of the block, after the
                // count of values a record holds.
                let values = match self.value(&self.under(&record, &[1])) {
                    Value::Enum { raw: 13, .. } => self.under(&record, &[2, 2, 0, 1]),
                    _ => self.under(&record, &[2, 1]),
                };
                for r in 0..self.count(&values) {
                    let row = self.under(&values, &[r]);
                    for v in 0..self.count(&row) {
                        out.push(self.value(&self.under(&row, &[v])));
                    }
                }
            }
        }
        out
    }

    /// Where every value of a variable is, in the same order as [`Cdf::values`].
    fn value_paths(&mut self, name: &str) -> Vec<Vec<usize>> {
        let vdr = self.variable(name);
        let index = self.field(&vdr, "values_index");
        let mut out = Vec::new();
        for i in 0..self.count(&index) {
            let blocks = self.under(&index, &[i, 2, 6]);
            for b in 0..self.count(&blocks) {
                let record = self.under(&blocks, &[b, 3, 0]);
                let values = match self.value(&self.under(&record, &[1])) {
                    Value::Enum { raw: 13, .. } => self.under(&record, &[2, 2, 0, 1]),
                    _ => self.under(&record, &[2, 1]),
                };
                for r in 0..self.count(&values) {
                    let row = self.under(&values, &[r]);
                    for v in 0..self.count(&row) {
                        out.push(self.under(&row, &[v]));
                    }
                }
            }
        }
        out
    }

    /// The moment at `at`, which has to be declared as one.
    fn moment(&mut self, at: &[usize]) -> TimeInfo {
        self.ev.time_of(&self.doc, at).unwrap_or_else(|e| panic!("{at:?}: {e:?}")).unwrap_or_else(|| panic!("{at:?} is not a time"))
    }

    /// The first value of every entry of the attribute of this name that is a
    /// time, whichever variable it is set on: a time variable's FILLVAL,
    /// VALIDMIN and VALIDMAX, in the order the file chains them.
    fn attribute_moments(&mut self, want: &str) -> Vec<TimeInfo> {
        let list = self.under(&self.gdr.clone(), &[16]);
        let mut out = Vec::new();
        for i in 0..self.count(&list) {
            let adr = self.under(&list, &[i, 2]);
            let name = self.field(&adr, "name");
            if self.text(&name).trim_end() != want {
                continue;
            }
            for entries in ["g_entries", "z_entries"] {
                let entries = self.field(&adr, entries);
                for e in 0..self.count(&entries) {
                    let entry = self.under(&entries, &[e, 2]);
                    let value = self.field(&entry, "value");
                    // Text has no elements to ask.
                    if self.count(&value) == 0 {
                        continue;
                    }
                    if let Some(t) = self.ev.time_of(&self.doc, &self.under(&value, &[0])).unwrap() {
                        out.push(t);
                    }
                }
            }
        }
        out
    }

    /// The value of the global attribute of this name, as text.
    fn global_attribute(&mut self, want: &str) -> String {
        let list = self.under(&self.gdr.clone(), &[16]);
        for i in 0..self.count(&list) {
            let adr = self.under(&list, &[i, 2]);
            let name = self.field(&adr, "name");
            if self.text(&name).trim_end() != want {
                continue;
            }
            let entries = self.field(&adr, "g_entries");
            let entry = self.under(&entries, &[0, 2]);
            let value = self.field(&entry, "value");
            // Padded out with spaces in a version 2 file, the way a name is.
            return self.text(&value).trim_end().to_string();
        }
        panic!("no attribute called {want}");
    }
}

fn ints(values: &[Value]) -> Vec<i128> {
    values.iter().map(|v| v.as_int().expect("a number")).collect()
}

fn floats(values: &[Value]) -> Vec<f64> {
    values
        .iter()
        .map(|v| match v {
            Value::Float(f) => *f,
            other => panic!("not a float: {other:?}"),
        })
        .collect()
}

/// The same, back at the width the file wrote them.
///
/// A single-precision value is kept here as the shortest decimal that reads
/// back as the same thirty-two bits, so `-4.2466445` rather than the
/// `-4.246644496917725` that widening spells out. Both are the same number and
/// only one of them is a fact about the file; narrowing again is what compares
/// like with like against what `cdflib` printed.
fn singles(values: &[Value]) -> Vec<f32> {
    floats(values).into_iter().map(|f| f as f32).collect()
}

/// What `cdflib.cdfepoch.encode` printed, as the moment it names:
/// `1982-01-01T00:00:00.000`, to however many places of a second it gave.
///
/// Parsed here rather than written out as seconds from 1970, so the test reads
/// against the reference's own output and a reader can check one against the
/// other by eye.
fn encoded(s: &str) -> Moment {
    let n = |a: usize, b: usize| s[a..b].parse::<i64>().unwrap();
    let (year, month, day, hour, minute, second) = (n(0, 4), n(5, 7), n(8, 10), n(11, 13), n(14, 16), n(17, 19));
    let fraction = s.get(20..).unwrap_or("");
    let nanos = format!("{fraction:0<9}")[..9].parse::<u32>().unwrap();
    // Howard Hinnant's days from civil, as `time.rs` has it.
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let days = era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468;
    let unix_seconds = days * 86_400 + hour * 3_600 + minute * 60 + second;
    Moment::At { unix_seconds, nanos }
}

fn strings(values: &[Value]) -> Vec<String> {
    values
        .iter()
        .map(|v| match v {
            Value::Str(s) => s.clone(),
            other => panic!("not text: {other:?}"),
        })
        .collect()
}

/// Parker Solar Probe's magnetometer, uncompressed and in network byte order.
#[test]
fn the_psp_magnetometer_reads_the_numbers_cdflib_reads() {
    let Some(root) = cdf_samples() else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    let path = root.join("psp_fld_l2_mag_rtn_1min_20200104_v02.cdf");
    if !path.is_file() {
        eprintln!("skipped: {} is not there", path.display());
        return;
    }
    let mut cdf = Cdf::open(&path);
    assert_eq!(cdf.value(&cdf.under(&cdf.gdr.clone(), &[8])).as_int(), Some(6), "six zVariables");

    // The field's own numbers, in a gzip block: three components a record, 118
    // records. The first record is the fill value, which is what the file has.
    let field = cdf.values("psp_fld_l2_mag_RTN_1min");
    assert_eq!(field.len(), 118 * 3);
    let f = singles(&field);
    assert!(f[0].is_nan() && f[1].is_nan() && f[2].is_nan(), "record nought is the fill value");
    assert_eq!(&f[3..6], &[-4.246644496917725, 6.030132293701172, 2.8181190490722656]);
    assert!(f[117 * 3].is_nan(), "and so is the last");

    // The times beside them, TT2000 nanoseconds. The block the index names
    // holds a thousand and twenty-four records where the variable has a
    // hundred and eighteen: the writer took the room and never gave it back,
    // and what is read here is the block as it was written.
    let times = ints(&cdf.values("epoch_mag_RTN_1min"));
    assert_eq!(times.len(), 1024);
    assert_eq!(&times[..3], &[631377279184000000, 631377339184000000, 631377399184000000]);
    assert_eq!(&times[115..118], &[631438359184000000, 631438419184000000, 631438479184000000]);

    // A character variable: three strings of three characters, one record.
    assert_eq!(strings(&cdf.values("label_RTN")), vec!["B_R", "B_T", "B_N"]);
    // And an int4 one of the same shape.
    assert_eq!(ints(&cdf.values("component_index_RTN")), vec![1, 2, 3]);

    // The quality flags: 1,440 records in a gzip block, every one of them nought.
    let flags = ints(&cdf.values("psp_fld_l2_quality_flags"));
    assert_eq!(flags.len(), 1440);
    assert!(flags.iter().all(|v| *v == 0));

    let quality_times = ints(&cdf.values("epoch_quality_flags"));
    assert_eq!(quality_times.len(), 1440);
    assert_eq!(quality_times[0], 631368069184000000);
    assert_eq!(quality_times[1439], 631454409184000000);

    // The same counts as moments, against what `cdflib.cdfepoch.encode` prints
    // for them. A TT2000 counts leap seconds, and five have been inserted
    // since J2000, so a count read without the table would be five seconds
    // out.
    let times = cdf.value_paths("epoch_mag_RTN_1min");
    let first = cdf.moment(&times[0]);
    assert_eq!(first.moment, encoded("2020-01-04T02:33:30.000000000"));
    assert_eq!((first.step_nanos, first.note), (1, None));
    assert_eq!(cdf.moment(&times[117]).moment, encoded("2020-01-04T19:33:30.000000000"));
    // The slots past the hundred and eighteen records the variable has hold
    // its pad value, which is no time, where as a count it would be 1707.
    for p in &times[118..] {
        assert_eq!(cdf.moment(p).moment, Moment::Unset, "{p:?}");
    }
    let vdr = cdf.variable("epoch_mag_RTN_1min");
    let pad = cdf.field(&vdr, "pad_value");
    assert_eq!(cdf.moment(&pad).moment, Moment::Unset);
    // `cdflib` reads the quality flags' times as every minute of the day, and
    // so does this, all 1,440 of them.
    let Moment::At { unix_seconds: midnight, .. } = encoded("2020-01-04T00:00:00") else { unreachable!() };
    for (i, p) in cdf.value_paths("epoch_quality_flags").iter().enumerate() {
        assert_eq!(cdf.moment(p).moment, Moment::At { unix_seconds: midnight + 60 * i as i64, nanos: 0 }, "minute {i}");
    }
    // The attributes beside them. Both variables' fill value is no time, and
    // the top of their valid range is after the last day the table of leap
    // seconds vouches for, which the answer says.
    let fill = cdf.attribute_moments("FILLVAL");
    assert_eq!(fill.iter().map(|t| t.moment).collect::<Vec<_>>(), vec![Moment::Unset; 2]);
    let valid_min = cdf.attribute_moments("VALIDMIN");
    assert_eq!(valid_min.iter().map(|t| t.moment).collect::<Vec<_>>(), vec![encoded("2010-01-01T00:00:00.000000000"); 2]);
    let valid_max = cdf.attribute_moments("VALIDMAX");
    assert_eq!(
        valid_max.iter().map(|t| (t.moment, t.note)).collect::<Vec<_>>(),
        vec![
            (encoded("2049-12-31T23:59:59.999999999"), Some(TimeNote::PastLeapSecondTable)),
            (encoded("2050-01-01T00:00:00.000000000"), Some(TimeNote::PastLeapSecondTable)),
        ]
    );

    assert_eq!(cdf.global_attribute("Logical_source"), "psp_fld_l2_mag_RTN_1min");
    assert_eq!(cdf.global_attribute("TITLE"), "PSP FIELDS Fluxgate Magnetometer (MAG) data");
}

/// FAST's electrostatic analyser: the whole file squeezed with CDF's run-length
/// coding, written by an IBM PC, with gzip blocks inside that.
#[test]
fn the_fast_analyser_reads_through_two_layers_of_packing() {
    let Some(root) = cdf_samples() else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    let path = root.join("fa_esa_l2_eeb_00000000_v01.cdf");
    if !path.is_file() {
        eprintln!("skipped: {} is not there", path.display());
        return;
    }
    let mut cdf = Cdf::open(&path);
    // An IBM PC wrote it, so every number in it is the other way round from
    // the records that place them.
    let cdr = cdf.under(&cdf.gdr.clone(), &[]);
    assert_eq!(cdf.value(&cdf.under(&cdr[..cdr.len() - 3], &[3])).as_int(), Some(6), "IBM PC encoding");

    // Text, read where the shape says one string of four characters.
    assert_eq!(strings(&cdf.values("project_name")), vec!["FAST"]);
    assert_eq!(strings(&cdf.values("data_name")), vec!["Eesa Burst"]);

    // Little-endian integers, one value each.
    assert_eq!(ints(&cdf.values("orbit_start")), vec![7595]);
    assert_eq!(ints(&cdf.values("charge")), vec![-1]);
    assert_eq!(ints(&cdf.values("compno_96")), (1..=96).collect::<Vec<i128>>());

    // A gzip block inside the run-length coded file: 3 by 32 by 96 floats,
    // which is 9,216 of them in one record.
    let energy = singles(&cdf.values("energy"));
    assert_eq!(energy.len(), 3 * 32 * 96);
    assert_eq!(&energy[..3], &[34119.69921875, 30105.599609375, 26091.5]);
    assert_eq!(&energy[energy.len() - 3..], &[3.9200000762939453; 3]);

    assert_eq!(cdf.global_attribute("Logical_source"), "fa_esa_l2_eeb");
    assert_eq!(cdf.global_attribute("PI_name"), "J. P. McFadden");

    // Two CDF_EPOCH variables with no records, whose attributes are still
    // times: the fill value -1.0E31, which `cdflib` encodes as the last
    // millisecond of 9999 and reads as no time, and the range of the mission.
    let moments = |t: Vec<TimeInfo>| t.into_iter().map(|t| t.moment).collect::<Vec<_>>();
    assert_eq!(moments(cdf.attribute_moments("FILLVAL")), vec![Moment::Unset; 2]);
    assert_eq!(moments(cdf.attribute_moments("VALIDMIN")), vec![encoded("1996-08-21T00:00:00.000"); 2]);
    assert_eq!(moments(cdf.attribute_moments("VALIDMAX")), vec![encoded("2009-05-01T00:00:00.000"); 2]);
}

/// One file, or a note that it is not there. The three files below ship with
/// NASA's own CDF distribution rather than with cdflib, so a collection that
/// has the two above may not have these.
fn sample(name: &str) -> Option<PathBuf> {
    let path = cdf_samples()?.join(name);
    match path.is_file() {
        true => Some(path),
        false => {
            eprintln!("skipped: {} is not there", path.display());
            None
        }
    }
}

/// A version 2.5 file whose variables do not vary along every dimension they
/// declare, which is what says a block holds fewer values than the shape
/// multiplies out to.
///
/// The numbers are `cdflib`'s again: it reads a version 2 file as happily as a
/// version 3 one, and `varinq` is what says which dimensions vary.
#[test]
fn a_version_two_file_reads_its_values_by_the_dimensions_that_vary() {
    let Some(path) = sample("cacsst2.cdf") else { return };
    let mut cdf = Cdf::open(&path);
    // Two dimensions, 180 by 91, declared once in the global descriptor and
    // shared by all four rVariables.
    let dims = cdf.field(&cdf.gdr.clone(), "dim_sizes");
    assert_eq!(ints(&[cdf.value(&cdf.under(&dims, &[0])), cdf.value(&cdf.under(&dims, &[1]))]), vec![180, 91]);

    // Latitude varies along the second dimension only, so a record of it is 91
    // numbers and not 16,380.
    let latitude = ints(&cdf.values("LATITUDE"));
    assert_eq!(latitude.len(), 91);
    assert_eq!(&latitude[..4], &[-90, -88, -86, -84]);
    assert_eq!(&latitude[89..], &[88, 90]);

    // Longitude varies along the first, so 180.
    let longitude = ints(&cdf.values("LONGITUD"));
    assert_eq!(longitude.len(), 180);
    assert_eq!(&longitude[..4], &[-178, -176, -174, -172]);
    assert_eq!(&longitude[178..], &[178, 180]);

    // The temperatures vary along both.
    //
    // In the order the file writes them, which for a column-major file is the
    // first dimension fastest: `cdflib` hands back an array of 180 by 91 and
    // the same run of numbers is its transpose flattened.
    let sst = singles(&cdf.values("SST"));
    assert_eq!(sst.len(), 180 * 91);
    assert_eq!(&sst[..3], &[-1e9; 3]);
    assert_eq!(&sst[sst.len() - 3..], &[-1.7999999523162842; 3]);

    // And the time varies along neither, so one value for the whole grid. It
    // is a CDF_EPOCH, which is a count of milliseconds in a float, and reads
    // as that number with the moment it counts to beside it.
    assert_eq!(floats(&cdf.values("EPOCH")), vec![62545910400000.0]);
    let epoch = cdf.value_paths("EPOCH");
    let time = cdf.moment(&epoch[0]);
    assert_eq!(time.moment, encoded("1982-01-01T00:00:00.000"));
    assert_eq!(time.step_nanos, 1_000_000);
    // Its range, in two attributes of its own type.
    let moments = |t: Vec<TimeInfo>| t.into_iter().map(|t| t.moment).collect::<Vec<_>>();
    assert_eq!(moments(cdf.attribute_moments("VALIDMIN")), vec![encoded("1982-01-01T00:00:00.000")]);
    assert_eq!(moments(cdf.attribute_moments("VALIDMAX")), vec![encoded("1992-12-01T00:00:00.000")]);
    // And its pad value, which `cdflib` gives as 0.0: no time.
    let vdr = cdf.variable("EPOCH");
    let pad = cdf.field(&vdr, "pad_value");
    assert_eq!(cdf.moment(&pad).moment, Moment::Unset);

    assert_eq!(cdf.global_attribute("TITLE"), "Climate Analysis Center SST blended analysis");
}

/// A version 2.6 file, the first release with a signature of its own: the same
/// walk again at half the width, down to compressed blocks of values.
#[test]
fn a_version_two_point_six_file_reads_its_z_variables() {
    let Some(path) = sample("geocpi0.cdf") else { return };
    let mut cdf = Cdf::open(&path);
    let density = singles(&cdf.values("SW_P_Den"));
    assert_eq!(density.len(), 50, "fifty records of one number");
    assert_eq!(&density[..4], &[4.254288196563721, 5.111618518829346, 4.48162317276001, 5.032914638519287]);
    assert_eq!(&density[48..], &[3.2359378337860107, 2.1136810779571533]);

    // Two components a record, in a block written in two pieces: the index has
    // two entries, forty records and then ten.
    let velocity = singles(&cdf.values("SW_V"));
    assert_eq!(velocity.len(), 100);
    assert_eq!(&velocity[..4], &[-421.58056640625, 43.2878532409668, -425.0941467285156, 49.079673767089844]);
    assert_eq!(&velocity[98..], &[-422.4188537597656, 47.61986541748047]);

    assert_eq!(cdf.global_attribute("Source_name"), "GEOTAIL>Geomagnetic Tail");

    // The times themselves are in a block `cdflib` cannot open, so what is
    // checked is the attributes about them, which it can.
    let moments = |t: Vec<TimeInfo>| t.into_iter().map(|t| t.moment).collect::<Vec<_>>();
    assert_eq!(moments(cdf.attribute_moments("VALIDMIN")), vec![encoded("1992-09-08T00:00:00.000")]);
    assert_eq!(moments(cdf.attribute_moments("VALIDMAX")), vec![encoded("2020-12-31T20:00:00.000")]);
    // Its FILLVAL is -1.0E31 too, but written as a CDF_REAL8 rather than a
    // CDF_EPOCH, which `cdflib`'s `attget` says as well. A double is not
    // declared a time whatever number is in it, so there is none here.
    assert_eq!(moments(cdf.attribute_moments("FILLVAL")), vec![]);
}

/// A whole file squeezed with CDF's Huffman coding, from NASA's distribution,
/// and the same file squeezed again with the adaptive Huffman coding.
///
/// `cdflib` reads neither: it opens gzip and nothing else. So the numbers
/// below came from `cdflib` reading the unpacked stream put back behind an
/// uncompressed signature, where the unpacking was done by `DecompressHUFF0`
/// from the distribution's own `cdfhuff.c`, compiled and run on 2026-09-14.
/// The FNV-1a hash is of those 78,606 bytes, so the stream is checked byte for
/// byte against the library as well as value by value against `cdflib`.
///
/// The adaptive file is `tools/make_cdf_huffman_samples.py`'s, and its stream
/// is the one `CompressAHUFF0` from the same file writes over those bytes.
/// Holding 78,606 bytes, it halves and rebuilds its tree more than once on
/// the way through.
#[test]
fn both_huffman_codings_unpack_to_the_file_the_library_unpacks() {
    let files = [("d103a2x.cdf", "cdf huffman"), ("d103a2x-ahuff.cdf", "cdf adaptive huffman")];
    for (name, codec) in files {
        let Some(path) = sample(name) else { continue };
        let mut cdf = Cdf::open(&path);
        let stream = cdf.ev.node(&cdf.doc, &[3, 2, 4]).unwrap();
        assert_eq!(stream.type_name, codec, "{name}");
        let id = cdf.ev.open_space(&cdf.doc, 0, &[3, 2, 4]).unwrap().expect("the stream opens");
        let bytes = cdf.ev.space(id).unwrap().bytes().to_vec();
        let fnv = bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3));
        assert_eq!((bytes.len(), fnv), (78_606, 0xe634_5974_1700_6f22), "{name}: the bytes the library unpacks");
        cdf.ev.space(id).unwrap().trace().check_tiles().unwrap_or_else(|e| panic!("{name}: {e}"));
        // The one block reads as rows: the frequency table or nothing at its
        // head, then a code per byte, and the padding after the end.
        let block = cdf.ev.node(&cdf.doc, &[3, 2, 4, 1, 0]).unwrap();
        assert_eq!(block.name, "dynamic block, last", "{name}");
        let rows: Vec<String> =
            (0..block.child_count as usize).map(|k| cdf.ev.node(&cdf.doc, &[3, 2, 4, 1, 0, k]).unwrap().name).collect();
        let codes = cdf.ev.node(&cdf.doc, &[3, 2, 4, 1, 0, rows.len() - 1]).unwrap();
        let n = codes.child_count as usize;
        let ends: Vec<String> = (0..3)
            .chain(n - 3..n)
            .map(|k| cdf.ev.node(&cdf.doc, &[3, 2, 4, 1, 0, rows.len() - 1, k]).unwrap().name)
            .collect();
        eprintln!("{name}: the block reads as {rows:?}, {n} codes from {:?} to {:?}", &ends[..3], &ends[3..]);

        assert_eq!(ints(&cdf.values("D103SCAN")), (1..=120).collect::<Vec<i128>>(), "{name}");
        assert_eq!(ints(&cdf.values("D103PIXL")), (1..=150).collect::<Vec<i128>>(), "{name}");
        // An image of 150 rows by 120 scans, mostly dark. The file is column
        // major, so in the order it writes them the first dimension runs
        // fastest, and the positions are those of `cdflib`'s array flattened
        // in Fortran order.
        let image = singles(&cdf.values("D103KRAY"));
        assert_eq!(image.len(), 18_000, "{name}");
        assert_eq!(image.iter().filter(|v| **v != 0.0).count(), 8_202, "{name}");
        let first: Vec<usize> = image.iter().enumerate().filter(|(_, v)| **v != 0.0).map(|(i, _)| i).take(3).collect();
        assert_eq!(first, [12, 13, 14], "{name}");
        assert_eq!(&image[12..15], &[0.9740260243415833, 0.9740260243415833, 2.597402572631836], "{name}");
        assert_eq!((image[17_657], image[17_864]), (0.3246753215789795, 0.3246753215789795), "{name}");
        assert!(image[17_865..].iter().all(|v| *v == 0.0), "{name}: nothing lit after the last");
        assert_eq!(image[6921], 13.636363983154297, "{name}: the brightest");
        let sum: f64 = image.iter().map(|v| f64::from(*v)).sum();
        assert!((sum - 21_588.961_379_587_65).abs() < 1e-6, "{name}: {sum}");

        assert_eq!(floats(&cdf.values("EPOCH")), vec![62_679_947_859_039.0], "{name}");
        let epoch = cdf.value_paths("EPOCH");
        assert_eq!(cdf.moment(&epoch[0]).moment, encoded("1986-04-01T08:37:39.039"), "{name}");
        assert_eq!(cdf.global_attribute("TITLE"), "DE-1 Auroral Imager (SAI), images, 12 m,CDAW-9", "{name}");
        eprintln!("{name}: {codec}, 78,606 bytes as the library unpacks them, values as cdflib reads them");
    }
}

/// A file from before version 2.5, which leaves 128 bytes of nothing in the
/// middle of every variable descriptor. Without reading past that, the name of
/// a variable is 128 bytes further on than the template looks and comes back
/// empty.
///
/// Not cross-checked against `cdflib`, which refuses the file: it is a
/// multi-file CDF, so its values are in `.v0` to `.v3` beside it and not in
/// here at all. What is checked is the descriptors.
#[test]
fn a_file_older_than_version_two_point_five_reads_past_its_unused_space() {
    let Some(path) = sample("example1.cdf") else { return };
    let mut cdf = Cdf::open(&path);
    for want in ["Time", "Longitude", "Latitude", "Temperature"] {
        let vdr = cdf.variable(want);
        let wasted = cdf.field(&vdr, "wasted");
        assert_eq!(cdf.ev.node(&cdf.doc, &wasted).unwrap().size_bits, 128 * 8, "{want}");
    }
    assert_eq!(cdf.global_attribute("TITLE"), "An example CDF (1).");
}
