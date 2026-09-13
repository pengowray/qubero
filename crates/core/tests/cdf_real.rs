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

use qubero_core::{document::Document, eval::Evaluator, eval::Value, formats, source::MemSource};

fn cdf_samples() -> Option<PathBuf> {
    let mut roots = Vec::new();
    if let Ok(paths) = std::env::var("QUBERO_SAMPLES") {
        roots.extend(paths.split(';').filter(|s| !s.is_empty()).map(PathBuf::from));
    }
    roots.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../qubero-samples"));
    roots.into_iter().map(|p| p.join("cdf")).find(|p| p.is_dir())
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
        eprintln!("skipped: set QUBERO_SAMPLES to the sample collection");
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

    assert_eq!(cdf.global_attribute("Logical_source"), "psp_fld_l2_mag_RTN_1min");
    assert_eq!(cdf.global_attribute("TITLE"), "PSP FIELDS Fluxgate Magnetometer (MAG) data");
}

/// FAST's electrostatic analyser: the whole file squeezed with CDF's run-length
/// coding, written by an IBM PC, with gzip blocks inside that.
#[test]
fn the_fast_analyser_reads_through_two_layers_of_packing() {
    let Some(root) = cdf_samples() else {
        eprintln!("skipped: set QUBERO_SAMPLES to the sample collection");
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
    // is a CDF_EPOCH, which is a count of milliseconds in a float and reads
    // here as that number rather than as a date.
    assert_eq!(floats(&cdf.values("EPOCH")), vec![62545910400000.0]);

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
