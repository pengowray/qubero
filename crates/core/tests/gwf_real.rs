//! The frame files in the collection, read against their own dictionaries.
//!
//! What this is for: the template picks each structure's body by the name the
//! file's own FrSH structures declare, and lays each body out from a field
//! list of its own. A frame file carries the field list its writer used, one
//! FrSE per field, so every structure in these files can be checked field for
//! field against the words beside it, and the check does not have to trust
//! anything the template itself read.
//!
//! The dictionary is read here straight off the bytes, by a walk that knows
//! only the structure header and the three strings of an FrSE. The template
//! is then asked for every structure the dictionary covers: the body must be
//! the class the dictionary names, its fields must be the fields the
//! dictionary lists in the order it lists them, and the last of them must end
//! exactly where the structure's length says the structure does.

use std::collections::HashMap;

use qubero_core::document::Document;
use qubero_core::eval::{Evaluator, Explain, Value};
use qubero_core::formats;
use qubero_core::source::MemSource;

fn sample(name: &str) -> Option<Vec<u8>> {
    let root = match std::env::var_os("QUBERO_SAMPLES") {
        Some(p) => std::path::PathBuf::from(p),
        None => std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../qubero-samples"),
    };
    std::fs::read(root.join("gwf").join(name)).ok()
}

const GWOSC: &str = "H-H1_GWOSC_4KHZ_R1-1126259447-32.gwf";
const FRAMEL: &str = "framel-8.30-test.gwf";
const HLV: &str = "HLV-HW100916-968654552-1.gwf";
const FRAMEL_V6: &str = "framel-6.24-test-v6.gwf";
const CAL_FAC: &str = "H-CAL_FAC_V03-729273600-5094000.gwf";

/// One structure as the bytes have it, found without the template.
struct Raw {
    offset: u64,
    length: u64,
    class: u16,
}

/// A file walked by hand: every structure's place and class, and the
/// dictionary, as class number to (name, field names).
struct Walked {
    structures: Vec<Raw>,
    dictionary: HashMap<u16, (String, Vec<String>)>,
}

fn walk(bytes: &[u8]) -> Walked {
    let little = u16::from_le_bytes([bytes[12], bytes[13]]) == 0x1234;
    let version = bytes[5];
    let u16_at = |at: usize| {
        let b = [bytes[at], bytes[at + 1]];
        if little { u16::from_le_bytes(b) } else { u16::from_be_bytes(b) }
    };
    let u64_at = |at: usize| {
        let b: [u8; 8] = bytes[at..at + 8].try_into().unwrap();
        if little { u64::from_le_bytes(b) } else { u64::from_be_bytes(b) }
    };
    // A counted string, and where the next thing starts.
    let string = |at: usize| {
        let n = u16_at(at) as usize;
        let text = String::from_utf8_lossy(&bytes[at + 2..at + 2 + n]).trim_end_matches('\0').to_string();
        (text, at + 2 + n)
    };
    let mut w = Walked { structures: Vec::new(), dictionary: HashMap::new() };
    let mut described = 0u16;
    let mut at = 40usize;
    while at + 14 <= bytes.len() {
        let length = u64_at(at);
        // Version 8 split the old two-byte class into a checksum byte and a
        // one-byte class; both keep the class in the byte before the instance.
        let class = if version >= 8 { u16::from(bytes[at + 9]) } else { u16_at(at + 8) };
        let body = at + 14;
        match class {
            1 => {
                let (name, next) = string(body);
                described = u16_at(next);
                w.dictionary.insert(described, (name, Vec::new()));
            }
            2 => {
                let (field, _) = string(body);
                w.dictionary.get_mut(&described).expect("an FrSE follows its FrSH").1.push(field);
            }
            _ => {}
        }
        w.structures.push(Raw { offset: at as u64, length, class });
        assert!(length >= 14, "structure at {at} has a length of {length}");
        at += length as usize;
    }
    assert_eq!(at, bytes.len(), "the lengths add up to the file");
    w
}

/// Where the stream is, and which child of a structure is its body.
fn stream(version: u8) -> (Vec<usize>, usize) {
    match version {
        6 => (vec![8, 6], 4),
        _ => (vec![8, 7], 5),
    }
}

/// Every structure the dictionary covers reads as the class it names, with
/// the fields it lists, ending where its length says. Returns the body type
/// names in stream order, for the caller's own counts.
fn check_against_dictionary(name: &str, bytes: Vec<u8>) -> Vec<String> {
    let version = bytes[5];
    let walked = walk(&bytes);
    let d = Document::new(MemSource(bytes));
    let mut ev = Evaluator::new(formats::builtin("gwf").unwrap());
    let (stream, body_at) = stream(version);
    let list = ev.node(&d, &stream).unwrap();
    assert_eq!(list.child_count as usize, walked.structures.len(), "{name}: structure count");
    let mut seen = Vec::new();
    for (i, raw) in walked.structures.iter().enumerate() {
        let mut path = stream.clone();
        path.extend([i, body_at]);
        let body = ev.node(&d, &path).unwrap();
        seen.push(body.type_name.clone());
        // The body ends where the structure does.
        assert_eq!(body.offset_bits + body.size_bits, (raw.offset + raw.length) * 8, "{name}: structure {i} body size");
        let Some((class, fields)) = walked.dictionary.get(&raw.class) else { continue };
        assert_eq!(&body.type_name, class, "{name}: structure {i} at {}", raw.offset);
        let names: Vec<String> = (0..body.child_count as usize)
            .map(|j| {
                let mut p = path.clone();
                p.push(j);
                ev.node(&d, &p).unwrap().name
            })
            .collect();
        // A version 6 table of contents lists the fields of one static data
        // group flat, and the template reads a count of those groups as a
        // list of them. The names inside a group are the dictionary's.
        let mut fields = fields.clone();
        if let Some(at) = names.iter().position(|n| n == "stat_types") {
            let group = ["nameStat", "detector", "nStatInstance", "tStart", "tEnd", "version", "positionStat"];
            assert_eq!(&fields[at..at + group.len()], group, "{name}: the dictionary lists one group");
            fields.splice(at..at + group.len(), ["stat_types".to_string()]);
        }
        assert_eq!(names, fields, "{name}: structure {i}, a {class} at {}", raw.offset);
        // And the fields tile the body: the last one ends where it does.
        if let Some(last) = body.child_count.checked_sub(1) {
            let mut p = path.clone();
            p.push(last as usize);
            let last = ev.node(&d, &p).unwrap();
            assert_eq!(last.offset_bits + last.size_bits, body.offset_bits + body.size_bits, "{name}: {class} {i}");
        }
    }
    assert!(!seen.iter().any(|t| t == "bytes[]"), "{name}: nothing falls through to bytes: {seen:?}");
    seen
}

fn count(seen: &[String], name: &str) -> usize {
    seen.iter().filter(|t| *t == name).count()
}

#[test]
fn every_structure_of_the_gwosc_frame_reads_as_the_class_its_dictionary_names() {
    let Some(bytes) = sample(GWOSC) else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    let seen = check_against_dictionary(GWOSC, bytes.clone());
    assert_eq!(seen.len(), 162);
    assert_eq!(count(&seen, "FrSH"), 7);
    assert_eq!(count(&seen, "FrSE"), 144);
    assert_eq!(count(&seen, "FrameH"), 1);
    assert_eq!(count(&seen, "FrDetector"), 1);
    assert_eq!(count(&seen, "FrProcData"), 3);
    assert_eq!(count(&seen, "FrVect"), 3);
    assert_eq!(count(&seen, "FrEndOfFrame"), 1);
    assert_eq!(count(&seen, "FrTOC"), 1);
    assert_eq!(count(&seen, "FrEndOfFile"), 1);

    let d = Document::new(MemSource(bytes));
    let mut ev = Evaluator::new(formats::builtin("gwf").unwrap());
    // The frame header names the project and the second GW150914 arrived in.
    let frame = seen.iter().position(|t| t == "FrameH").unwrap();
    assert_eq!(ev.node(&d, &[8, 7, frame, 5, 0, 1]).unwrap().value, Value::Str("LIGO".into()));
    assert_eq!(ev.node(&d, &[8, 7, frame, 5, 4]).unwrap().value, Value::UInt(1_126_259_447));
    // The structure is labelled by what the file calls its class.
    assert_eq!(ev.node(&d, &[8, 7, frame, 3]).unwrap().value, Value::Str("FrameH".into()));

    let vect = seen.iter().position(|t| t == "FrVect").unwrap();
    assert_eq!(
        ev.node(&d, &[8, 7, vect, 5, 1]).unwrap().value,
        Value::Enum { raw: 257, name: Some("gzip (little-endian words)".into()), hex: false }
    );
}

/// The strain inside the gzip-packed vector, against the HDF5 file GWOSC
/// publishes for the same 32 seconds.
///
/// The twin is not in the collection. It was compared whole, all 131,072
/// samples equal as doubles, with h5py reading `strain/Strain` from
/// `H-H1_GWOSC_4KHZ_R1-1126259447-32.hdf5` and zlib and numpy reading this
/// vector; the numbers below are three of those samples, and the two masks,
/// which were equal too.
#[test]
fn the_gwosc_strain_opens_as_the_samples_its_hdf5_twin_holds() {
    let Some(bytes) = sample(GWOSC) else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    let mut f = Frames::open(bytes);
    let strain = f.named("FrVect", "H1:GWOSC-4KHZ_R1_STRAIN");
    let data = f.child(&strain, "data");
    let space = f.ev.node(&f.d, &data).unwrap();
    assert_eq!(space.size_bits, 1_015_922 * 8, "the packed run keeps its size in the file");
    let mut numbers = data.clone();
    numbers.push(0);
    assert_eq!(f.ev.node(&f.d, &numbers).unwrap().child_count, 131_072);
    for (i, want) in [(0, 9.067308911592338e-21), (65_536, 2.098363051339207e-19), (131_071, 7.764568284598941e-20)] {
        let mut p = numbers.clone();
        p.push(i);
        assert_eq!(f.ev.node(&f.d, &p).unwrap().value, Value::Float(want), "sample {i}");
    }
    for (name, want) in [("H1:GWOSC-4KHZ_R1_DQMASK", 127), ("H1:GWOSC-4KHZ_R1_INJMASK", 31)] {
        let mask = f.named("FrVect", name);
        let mut p = f.child(&mask, "data");
        p.extend([0, 31]);
        assert_eq!(f.ev.node(&f.d, &p).unwrap().value, Value::Int(want), "{name}");
    }
}

/// FrameL's example writes `fastProc` as twice `fastAdc1` in every frame, the
/// first uncompressed as floats and the second zero-suppressed as shorts. So
/// every one of the 2,000 shorts the side reader unpacks, in each of the ten
/// frames, has its double beside it in bytes nothing packed.
#[test]
fn every_zero_suppressed_short_in_framels_test_file_is_half_the_float_written_beside_it() {
    zero_suppressed_shorts_are_half_the_floats_beside_them(FRAMEL);
}

/// The same program's output in version 6, packed the same way.
#[test]
fn every_zero_suppressed_short_in_framels_version_6_file_is_half_the_float_written_beside_it() {
    zero_suppressed_shorts_are_half_the_floats_beside_them(FRAMEL_V6);
}

fn zero_suppressed_shorts_are_half_the_floats_beside_them(name: &str) {
    let Some(bytes) = sample(name) else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    let file = bytes.clone();
    let mut f = Frames::open(bytes);
    let vects = count(&f.seen, "FrVect");
    let mut adcs = 0;
    for nth in 0..vects {
        let vect = f.body("FrVect", nth);
        if f.text(&vect, "name") != "fastAdc1" {
            continue;
        }
        // The next vector called fastProc is this frame's.
        let mut proc = None;
        for k in nth + 1..vects {
            let p = f.body("FrVect", k);
            if f.text(&p, "name") == "fastProc" {
                proc = Some(p);
                break;
            }
        }
        let proc = proc.expect("a fastProc after each fastAdc1");
        let data = f.child(&vect, "data");
        let (steps, values, total, problem) = match f.ev.explain(&f.d, &data, None).unwrap() {
            Explain::Hdf5Chunk { steps, values, total, problem, .. } => (steps, values, total, problem),
            other => panic!("{other:?}"),
        };
        assert_eq!(problem, None);
        assert_eq!(total, 2000);
        let names: Vec<&str> = steps.iter().map(|s| s.filter.as_str()).collect();
        assert_eq!(names, ["zero suppression", "differencing"]);
        // The panel shows the first few; the side reader is asked for all of
        // them, from the same bytes the template placed.
        let packed = f.ev.node(&f.d, &data).unwrap();
        let run = &file[(packed.offset_bits / 8) as usize..((packed.offset_bits + packed.size_bits) / 8) as usize];
        let unpacked = qubero_core::formats::gwf_vect::decode(run, 261, 1, 2000);
        let shorts: Vec<i16> = unpacked.bytes.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect();
        assert_eq!(shorts.len(), 2000);
        assert_eq!(values[0], shorts[0].to_string());
        let floats = f.child(&proc, "data");
        for (j, s) in shorts.iter().enumerate() {
            let mut p = floats.clone();
            p.push(j);
            let Value::Float(x) = f.ev.node(&f.d, &p).unwrap().value else { panic!() };
            assert_eq!(x, 2.0 * f64::from(*s), "frame {adcs}, sample {j}");
        }
        adcs += 1;
    }
    assert_eq!(adcs, 10);
}

/// FrameL's own test file: ten frames of everything the format has, which is
/// where the eleven classes the GWOSC file lacks are checked against bytes.
#[test]
fn every_structure_of_framels_test_file_reads_as_the_class_its_dictionary_names() {
    let Some(bytes) = sample(FRAMEL) else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    let seen = check_against_dictionary(FRAMEL, bytes);
    for (class, n) in [
        ("FrameH", 10),
        ("FrDetector", 10),
        ("FrStatData", 2),
        ("FrHistory", 20),
        ("FrRawData", 10),
        ("FrAdcData", 80),
        ("FrSerData", 10),
        ("FrTable", 10),
        ("FrMsg", 10),
        ("FrProcData", 10),
        ("FrSimData", 20),
        ("FrEvent", 30),
        ("FrSimEvent", 20),
        ("FrSummary", 20),
        ("FrVect", 132),
        ("FrEndOfFrame", 10),
        ("FrTOC", 1),
        ("FrEndOfFile", 1),
    ] {
        assert_eq!(count(&seen, class), n, "{class}");
    }
}

/// A reader over one file: finds bodies by class and fields by name, so a
/// test can say what it expects without a wall of child indices.
struct Frames {
    d: Document<MemSource>,
    ev: Evaluator,
    stream: Vec<usize>,
    body_at: usize,
    seen: Vec<String>,
}

impl Frames {
    fn open(bytes: Vec<u8>) -> Frames {
        let (stream, body_at) = stream(bytes[5]);
        let d = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(formats::builtin("gwf").unwrap());
        let n = ev.node(&d, &stream).unwrap().child_count as usize;
        let mut seen = Vec::with_capacity(n);
        for i in 0..n {
            let mut p = stream.clone();
            p.extend([i, body_at]);
            seen.push(ev.node(&d, &p).unwrap().type_name);
        }
        Frames { d, ev, stream, body_at, seen }
    }

    /// The body of the `nth` structure of this class.
    fn body(&self, class: &str, nth: usize) -> Vec<usize> {
        let i = self.seen.iter().enumerate().filter(|(_, t)| *t == class).nth(nth).map(|(i, _)| i);
        let mut p = self.stream.clone();
        p.extend([i.unwrap_or_else(|| panic!("no {class} number {nth}")), self.body_at]);
        p
    }

    /// The first body of this class whose `name` string reads `name`.
    fn named(&mut self, class: &str, name: &str) -> Vec<usize> {
        for nth in 0.. {
            let p = self.body(class, nth);
            if self.text(&p, "name") == name {
                return p;
            }
        }
        unreachable!()
    }

    fn child(&mut self, at: &[usize], name: &str) -> Vec<usize> {
        let n = self.ev.node(&self.d, at).unwrap().child_count as usize;
        for j in 0..n {
            let mut p = at.to_vec();
            p.push(j);
            if self.ev.node(&self.d, &p).unwrap().name == name {
                return p;
            }
        }
        panic!("no field {name} under {at:?}")
    }

    fn value(&mut self, at: &[usize], name: &str) -> Value {
        let p = self.child(at, name);
        self.ev.node(&self.d, &p).unwrap().value
    }

    /// A counted string's text.
    fn text(&mut self, at: &[usize], name: &str) -> String {
        let mut p = self.child(at, name);
        p.push(1);
        match self.ev.node(&self.d, &p).unwrap().value {
            Value::Str(s) => s,
            other => panic!("{name} is {other:?}"),
        }
    }

    fn float(&mut self, at: &[usize], name: &str) -> f64 {
        match self.value(at, name) {
            Value::Float(f) => f,
            other => panic!("{name} is {other:?}"),
        }
    }

    fn int(&mut self, at: &[usize], name: &str) -> i128 {
        self.value(at, name).as_int().unwrap_or_else(|| panic!("{name} is not a number"))
    }

    /// Element `i` of a list field, or a field of that element.
    fn elem(&mut self, at: &[usize], name: &str, i: usize) -> Value {
        let mut p = self.child(at, name);
        p.push(i);
        self.ev.node(&self.d, &p).unwrap().value
    }

    fn elem_text(&mut self, at: &[usize], name: &str, i: usize) -> String {
        let mut p = self.child(at, name);
        p.extend([i, 1]);
        match self.ev.node(&self.d, &p).unwrap().value {
            Value::Str(s) => s,
            other => panic!("{name}[{i}] is {other:?}"),
        }
    }
}

/// The values FrameL's exampleFull.c puts in the classes the GWOSC file has
/// none of, read back out of the file it wrote. The program is short enough
/// that every number here is one it names: a slope of 20/65536, an event with
/// masses 1.4 and 1.3 and a chi-squared of 2.3, a table of 0.2 cos(0.1 j).
#[test]
fn framels_test_file_holds_what_its_example_program_wrote() {
    holds_what_the_example_program_wrote(FRAMEL);
}

/// The same program in FrameL 6.24, in version 6, where the event parameters
/// and two sample rates are 4-byte floats. It made no FrStatData then.
#[test]
fn framels_version_6_file_holds_what_its_example_program_wrote() {
    holds_what_the_example_program_wrote(FRAMEL_V6);
}

fn holds_what_the_example_program_wrote(name: &str) {
    let Some(bytes) = sample(name) else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    let six = bytes[5] == 6;
    let mut f = Frames::open(bytes);

    let adc = f.named("FrAdcData", "fastAdc0");
    assert_eq!(f.int(&adc, "nBits"), 16);
    assert_eq!(f.float(&adc, "sampleRate"), 2000.0);
    if !six {
        // FrameL 6.24's example made its fast channels without a comment,
        // a slope or units; 8.30's gives them all three.
        assert_eq!(f.text(&adc, "comment"), "test");
        // A 4-byte float comes back as the shortest decimal that reads as the
        // same 4 bytes, so it is compared as one.
        assert_eq!(f.float(&adc, "slope") as f32, 20.0f32 / 65536.0);
        assert_eq!(f.text(&adc, "units"), "Volts");
    }
    let bad = f.named("FrAdcData", "Bad_data_(data[0]=nan)");
    assert_eq!(f.int(&bad, "dataValid"), 1);

    let sim = f.named("FrSimData", "sim1");
    assert_eq!(f.float(&sim, "sampleRate"), 200.0);

    let ser = f.named("FrSerData", "sms1");
    assert_eq!(f.text(&ser, "data"), "sms data are here");
    assert_eq!(f.float(&ser, "sampleRate"), 1.0);

    let msg = f.body("FrMsg", 0);
    assert_eq!(f.text(&msg, "alarm"), "Test");
    assert_eq!(f.text(&msg, "message"), "Test message");

    let summary = f.named("FrSummary", "Quality_1");
    assert_eq!(f.text(&summary, "comment"), "main quality");

    // The trigger that had a third parameter added after it was made.
    let event = f.body("FrEvent", 0);
    assert_eq!(f.text(&event, "name"), "trigger_1");
    assert_eq!(f.text(&event, "inputs"), "V0:Pr_B1_ACq");
    assert_eq!(f.int(&event, "nParam"), 3);
    assert_eq!(f.elem(&event, "parameters", 0), Value::Float(1.4));
    assert_eq!(f.elem(&event, "parameters", 2), Value::Float(2.3));
    assert_eq!(f.elem_text(&event, "parameterNames", 2), "chisquare");

    let inspiral = f.named("FrSimEvent", "Sim_Inspiral");
    assert_eq!(f.int(&inspiral, "nParam"), 2);
    assert_eq!(f.elem(&inspiral, "parameters", 1), Value::Float(1.333));
    assert_eq!(f.elem_text(&inspiral, "parameterNames", 1), "M2");

    if six {
        // Four bytes wide in version 6: the event's parameters, and the
        // sample rates of the serial and simulated channels.
        let width = |f: &mut Frames, at: &[usize], field: &str| {
            let p = f.child(at, field);
            f.ev.node(&f.d, &p).unwrap().size_bits
        };
        assert_eq!(width(&mut f, &event, "parameters"), 3 * 32);
        assert_eq!(width(&mut f, &ser, "sampleRate"), 32);
        assert_eq!(width(&mut f, &sim, "sampleRate"), 32);
        assert_eq!(width(&mut f, &adc, "sampleRate"), 64);
    } else {
        let gain = f.named("FrStatData", "gain");
        assert_eq!(f.text(&gain, "comment"), "ADC 1 gain");
        assert_eq!(f.text(&gain, "representation"), "calibration");
        assert_eq!(f.int(&gain, "timeEnd") - f.int(&gain, "timeStart"), 20);
    }

    let table = f.named("FrTable", "table_1");
    assert_eq!(f.text(&table, "comment"), "slow monitoring");
    assert_eq!((f.int(&table, "nColumn"), f.int(&table, "nRow")), (2, 5));

    let raw = f.body("FrRawData", 0);
    assert_eq!(f.text(&raw, "name"), "rawData");

    let proc = f.named("FrProcData", "fastProc");
    assert_eq!(f.elem(&proc, "auxParam", 0), Value::Float(1.5));
    assert_eq!(f.elem_text(&proc, "auxParamNames", 0), "gain");

    let history = f.named("FrHistory", "fastProc");
    assert_eq!(f.text(&history, "comment"), "fastProc = 2 *fastAdc");

    // Three vectors FrameL left unpacked, marked 256, read as their numbers.
    let d1 = f.named("FrVect", "D1");
    assert_eq!(f.int(&d1, "compress"), 256);
    let data = f.child(&d1, "data");
    let mut p = data.clone();
    p.push(100);
    let as_f32 = |v: Value| match v {
        Value::Float(x) => x as f32,
        other => panic!("{other:?}"),
    };
    // sin(.01*j) worked out in doubles and stored in a float.
    assert_eq!(as_f32(f.ev.node(&f.d, &p).unwrap().value), 1.0f64.sin() as f32);
    let column = f.named("FrVect", "value");
    assert_eq!(as_f32(f.elem(&column, "data", 1)), (0.2 * 0.1f64.cos()) as f32);
    let names = f.named("FrVect", "channel");
    let mut p = f.child(&names, "data");
    p.extend([0, 1]);
    assert_eq!(f.ev.node(&f.d, &p).unwrap().value, Value::Str(String::new()));
}

/// A second FrameL, numbering its classes in the order it first wrote one:
/// FrVect is 5 here and 20 in the file above, and both read as vectors.
#[test]
fn every_structure_of_the_gwpy_frame_reads_as_the_class_its_dictionary_names() {
    let Some(bytes) = sample(HLV) else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    let seen = check_against_dictionary(HLV, bytes);
    assert_eq!(count(&seen, "FrVect"), 3);
    assert_eq!(count(&seen, "FrProcData"), 3);
    assert_eq!(count(&seen, "FrHistory"), 1);
}

/// Version 6, from FrameL 6.24: every class but FrStatData, with no checksum
/// on any structure and a table of contents with no totals in it.
#[test]
fn every_structure_of_framels_version_6_file_reads_as_the_class_its_dictionary_names() {
    let Some(bytes) = sample(FRAMEL_V6) else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    let seen = check_against_dictionary(FRAMEL_V6, bytes.clone());
    for (class, n) in [
        ("FrameH", 10),
        ("FrDetector", 10),
        ("FrHistory", 20),
        ("FrRawData", 10),
        ("FrAdcData", 80),
        ("FrSerData", 10),
        ("FrTable", 10),
        ("FrMsg", 10),
        ("FrProcData", 10),
        ("FrSimData", 20),
        ("FrEvent", 30),
        ("FrSimEvent", 20),
        ("FrSummary", 20),
        ("FrVect", 130),
        ("FrEndOfFrame", 10),
        ("FrTOC", 1),
        ("FrEndOfFile", 1),
    ] {
        assert_eq!(count(&seen, class), n, "{class}");
    }
    let mut f = Frames::open(bytes);
    // The header ends in the two letters version 8 gave to the library.
    assert_eq!(f.ev.node(&f.d, &[8, 5]).unwrap().name, "check_ascii");
    // The event columns of the table of contents are as long as the counts
    // per type add up to: 20 of trigger_1 and 10 of Burst_1.
    let toc = f.body("FrTOC", 0);
    assert_eq!(f.int(&toc, "nEventType"), 2);
    assert_eq!(f.int(&toc, "GTimeSEvent"), 30);
    assert_eq!(f.int(&toc, "nStatType"), 0);
    let end = f.body("FrEndOfFile", 0);
    assert_eq!(f.int(&end, "nFrames"), 10);
}

/// Version 6 again, from FrameCPP: calibration factors from 2003, as complex
/// numbers FrameCPP left unpacked.
#[test]
fn every_structure_of_the_version_6_calibration_frame_reads_as_the_class_its_dictionary_names() {
    let Some(bytes) = sample(CAL_FAC) else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    let seen = check_against_dictionary(CAL_FAC, bytes.clone());
    assert_eq!(count(&seen, "FrProcData"), 2);
    assert_eq!(count(&seen, "FrVect"), 2);
    assert_eq!(count(&seen, "FrHistory"), 1);
    let mut f = Frames::open(bytes);
    let vect = f.named("FrVect", "H1:CAL-OLOOP_FAC");
    assert_eq!(f.int(&vect, "nData"), 84_900);
    let mut p = f.child(&vect, "data");
    assert_eq!(f.ev.node(&f.d, &p).unwrap().child_count, 84_900);
    // The first factor: its real part the float whose bits are 0x3f6b4db8,
    // and nothing imaginary.
    p.extend([0, 0]);
    let Value::Float(re) = f.ev.node(&f.d, &p).unwrap().value else { panic!("not a float") };
    assert_eq!(re as f32, f32::from_bits(0x3f6b_4db8));
    p.pop();
    p.push(1);
    assert_eq!(f.ev.node(&f.d, &p).unwrap().value, Value::Float(0.0));
}
