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

    /// The descriptor of the zVariable of this name.
    fn variable(&mut self, want: &str) -> Vec<usize> {
        let list = self.under(&self.gdr.clone(), &[15]);
        for i in 0..self.count(&list) {
            let vdr = self.under(&list, &[i, 2]);
            if self.text(&self.under(&vdr, &[14])) == want {
                return vdr;
            }
        }
        panic!("no zVariable called {want}");
    }

    /// Every value of a variable, in the order the file writes them: the index
    /// records in turn, the blocks each one names, the records in a block, and
    /// the values in a record.
    fn values(&mut self, name: &str) -> Vec<Value> {
        let vdr = self.variable(name);
        let index = self.under(&vdr, &[19]);
        let mut out = Vec::new();
        for i in 0..self.count(&index) {
            let blocks = self.under(&index, &[i, 2, 6]);
            for b in 0..self.count(&blocks) {
                let record = self.under(&blocks, &[b, 3, 0]);
                // A compressed block keeps its values inside the stream; an
                // uncompressed one is the values and nothing else.
                let values = match self.value(&self.under(&record, &[1])) {
                    Value::Enum { raw: 13, .. } => self.under(&record, &[2, 2, 0]),
                    _ => self.under(&record, &[2]),
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
            if self.text(&self.under(&adr, &[11])) != want {
                continue;
            }
            return self.text(&self.under(&adr, &[12, 0, 2, 10]));
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
