//! Real TDMS files, read segment by segment into the values of each channel
//! and checked against what npTDMS reads out of the same file.
//!
//! Ten files: four from npTDMS's own tests, two of them written by LabVIEW
//! (one big-endian) and two by DAQmx, and six from the collection's
//! generator, which packs by hand what npTDMS's writer cannot write: a segment
//! that says its channels are laid out as before, one with no metadata, one
//! that adds a channel to the list before it, interleaved data, a big-endian
//! file with strings and timestamps, and a writer that stopped partway.
//!
//! The expected numbers are npTDMS 1.11's: for each channel, how many values
//! it reads across the whole file, the first and the last, and their sum. The
//! walk below gathers the same thing from the template's own fields, segment
//! after segment, so a chunk placed a byte out or a segment laid out by the
//! wrong list changes a sum.
//!
//! The files live in the sample collection rather than here. Point
//! `QUBERO_SAMPLES` at it, or keep it beside the repository as
//! `qubero-samples`. With neither, the test says so and passes.

use std::collections::BTreeMap;
use std::path::PathBuf;

use qubero_core::document::Document;
use qubero_core::eval::{Evaluator, Moment, Value};
use qubero_core::formats;
use qubero_core::source::MemSource;

fn sample(name: &str) -> Option<PathBuf> {
    qubero_samples::roots().into_iter().map(|r| r.join("tdms").join(name)).find(|p| p.exists())
}

fn read(name: &str) -> Option<(Document<MemSource>, Evaluator)> {
    let path = sample(name)?;
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(formats::sniff(&bytes[..bytes.len().min(formats::SNIFF_WINDOW)], bytes.len() as u64), Some("tdms"));
    let doc = Document::new(MemSource(bytes));
    Some((doc, Evaluator::new(formats::builtin("tdms").unwrap())))
}

#[derive(Debug, Clone, PartialEq)]
enum V {
    Num(f64),
    Text(String),
    /// Unix seconds and nanoseconds.
    Time(i64, u32),
}

/// What npTDMS reads for one channel: how many values, and the first, the
/// last and the sum for numbers.
enum Want {
    Num(f64, f64, f64),
    Text(&'static str, &'static str),
    Time((i64, u32), (i64, u32)),
}

/// The child of `path` called `name`.
fn child(ev: &mut Evaluator, d: &Document<MemSource>, path: &[usize], name: &str) -> Option<Vec<usize>> {
    let n = ev.node(d, path).unwrap().child_count as usize;
    (0..n).map(|i| [path, &[i]].concat()).find(|p| ev.node(d, p).unwrap().name == name)
}

/// One value, whatever it was read as.
fn value(ev: &mut Evaluator, d: &Document<MemSource>, path: &[usize]) -> V {
    let n = ev.node(d, path).unwrap();
    match (n.type_name.as_str(), n.value) {
        ("TimeStamp", _) => {
            let seconds = child(ev, d, path, "seconds").unwrap();
            let Some(Moment::At { unix_seconds, nanos }) = ev.time_of(d, &seconds).unwrap().map(|t| t.moment) else {
                panic!("{path:?} is not a moment")
            };
            assert_eq!(nanos, 0, "the seconds are whole");
            let fraction = child(ev, d, path, "nanoseconds").unwrap();
            let nanos = ev.node(d, &fraction).unwrap().value.as_int().unwrap() as u32;
            V::Time(unix_seconds, nanos)
        }
        (_, Value::Float(f)) => V::Num(f),
        (_, Value::Str(s)) => V::Text(s),
        (_, v) => V::Num(v.as_int().unwrap_or_else(|| panic!("{path:?} holds {v:?}")) as f64),
    }
}

/// The values of one channel in one chunk, or of one sample in interleaved
/// data, onto the end of what that channel has so far.
fn gather(ev: &mut Evaluator, d: &Document<MemSource>, values: &[usize], interleaved: bool, out: &mut BTreeMap<String, Vec<V>>) {
    let n = ev.node(d, values).unwrap().child_count as usize;
    for i in 0..n {
        let p = [values, &[i]].concat();
        let node = ev.node(d, &p).unwrap();
        let name = node.name.split_once("] ").map(|(_, path)| path.to_string()).expect("each value is named by its channel");
        let into = out.entry(name).or_default();
        if interleaved {
            into.push(value(ev, d, &p));
        } else if node.type_name == "Strings" {
            let text = child(ev, d, &p, "text").unwrap();
            for j in 0..ev.node(d, &text).unwrap().child_count as usize {
                into.push(value(ev, d, &[&text[..], &[j]].concat()));
            }
        } else {
            for j in 0..node.child_count as usize {
                into.push(value(ev, d, &[&p[..], &[j]].concat()));
            }
        }
    }
}

/// Every channel's values across every segment of the file, and the layout
/// of each segment that has raw data.
fn channels(ev: &mut Evaluator, d: &Document<MemSource>) -> (BTreeMap<String, Vec<V>>, Vec<String>) {
    let mut out = BTreeMap::new();
    let mut layouts = Vec::new();
    let segments = ev.node(d, &[]).unwrap().child_count as usize;
    for k in 0..segments {
        let raw = child(ev, d, &[k, 2], "raw_data").unwrap();
        if ev.node(d, &raw).unwrap().absent {
            continue;
        }
        let Value::Enum { name: Some(layout), .. } = ev.node(d, &[&raw[..], &[0]].concat()).unwrap().value else { panic!() };
        layouts.push(layout);
        let raw = child(ev, d, &raw, "data").unwrap();
        let node = ev.node(d, &raw).unwrap();
        let (list, interleaved, tail) = match node.type_name.as_str() {
            "ContiguousData" => ("chunks", false, "unfinished_chunk"),
            "InterleavedData" => ("samples", true, "unfinished_sample"),
            _ => continue,
        };
        let list = child(ev, d, &raw, list).unwrap();
        for c in 0..ev.node(d, &list).unwrap().child_count as usize {
            gather(ev, d, &[&list[..], &[c, 0]].concat(), interleaved, &mut out);
        }
        let tail = child(ev, d, &raw, tail).unwrap();
        if !interleaved && !ev.node(d, &tail).unwrap().absent {
            gather(ev, d, &[&tail[..], &[0]].concat(), false, &mut out);
        }
    }
    (out, layouts)
}

fn check(name: &str, want: &[(&str, usize, Want)]) -> Option<Vec<String>> {
    let (d, mut ev) = read(name)?;
    let (got, layouts) = channels(&mut ev, &d);
    for (path, count, w) in want {
        let values = got.get(*path).unwrap_or_else(|| panic!("{name}: no values for {path}; have {:?}", got.keys()));
        assert_eq!(values.len(), *count, "{name}: {path}");
        let (first, last) = (values[0].clone(), values[values.len() - 1].clone());
        match w {
            Want::Num(f, l, sum) => {
                assert_eq!((first, last), (V::Num(*f), V::Num(*l)), "{name}: {path}");
                let total: f64 = values.iter().map(|v| if let V::Num(n) = v { *n } else { panic!() }).sum();
                assert!((total - sum).abs() <= 1e-9 * sum.abs().max(1.0), "{name}: {path} sums to {total}, npTDMS {sum}");
            }
            Want::Text(f, l) => assert_eq!((first, last), (V::Text(f.to_string()), V::Text(l.to_string())), "{name}: {path}"),
            Want::Time(f, l) => assert_eq!((first, last), (V::Time(f.0, f.1), V::Time(l.0, l.1)), "{name}: {path}"),
        }
    }
    assert_eq!(got.len(), want.len(), "{name}: read {:?}", got.keys());
    // The last segment ends where the file does.
    let n = ev.node(&d, &[]).unwrap();
    assert_eq!(n.offset_bits + n.size_bits, d.len_bits(), "{name}");
    Some(layouts)
}

macro_rules! skip_without_samples {
    ($e:expr) => {
        match $e {
            Some(v) => v,
            None => {
                eprintln!("{}", qubero_samples::missing());
                return;
            }
        }
    };
}

#[test]
fn one_segment_of_every_type_nptdms_writes() {
    let layouts = skip_without_samples!(check(
        "groups-types.tdms",
        &[
            ("/'wave'/'sine'", 12, Want::Num(0.0, -0.7055403255703919, 0.21770064810948386)),
            ("/'wave'/'counts'", 12, Want::Num(-20.0, 57.0, 222.0)),
            ("/'wave'/'small'", 5, Want::Num(0.0, 255.0, 511.0)),
            ("/'wave'/'single'", 5, Want::Num(-1.0, 1.0, 0.0)),
            ("/'log'/'message'", 4, Want::Text("started", "stopped: µV drift")),
            ("/'log'/'when'", 4, Want::Time((1789378200, 250000000), (1789464600, 250000000))),
            ("/'log'/'flags'", 3, Want::Num(1.0, 1.0, 2.0)),
            ("/'log'/'wide'", 2, Want::Num(-1099511627776.0, 1099511627781.0, 5.0)),
        ],
    ));
    assert_eq!(layouts, ["contiguous"]);
}

#[test]
fn a_layout_carried_from_segment_to_segment() {
    let layouts = skip_without_samples!(check(
        "reused-layout.tdms",
        &[
            ("/'run'/'a'", 21, Want::Num(0.5, 62.5, 661.5)),
            ("/'run'/'b'", 21, Want::Num(0.0, -62.0, -651.0)),
            ("/'run'/'n'", 4, Want::Num(0.0, 110.0, 220.0)),
            ("/'run'/'s'", 2, Want::Text("x", "yy")),
        ],
    ));
    // A new list, a new list of channels laid out as before, no metadata, and
    // a string channel added to the list before.
    assert_eq!(layouts, ["contiguous", "contiguous", "contiguous, laid out by an earlier segment", "contiguous"]);
}

#[test]
fn interleaved_samples_across_two_segments() {
    let layouts = skip_without_samples!(check(
        "interleaved.tdms",
        &[
            ("/'daq'/'i16'", 12, Want::Num(-3.0, 8.0, 30.0)),
            ("/'daq'/'f32'", 12, Want::Num(0.0, 2.75, 16.5)),
            ("/'daq'/'f64'", 12, Want::Num(-0.0, -16.5, -99.0)),
        ],
    ));
    assert_eq!(layouts, ["interleaved", "interleaved, laid out by an earlier segment"]);
}

#[test]
fn a_big_endian_file_with_strings_and_timestamps() {
    skip_without_samples!(check(
        "big-endian.tdms",
        &[
            ("/'be'/'counts'", 12, Want::Num(1.0, 8.0, -32417.0)),
            ("/'be'/'labels'", 3, Want::Text("one", "six")),
            ("/'be'/'times'", 2, Want::Time((1757155200, 500000000), (1757155260, 0))),
        ],
    ));
}

/// npTDMS reads the two values of the chunk the writer did not finish, and so
/// does this.
#[test]
fn a_file_whose_writer_stopped_partway_through_a_chunk() {
    skip_without_samples!(check("unfinished.tdms", &[("/'log'/'v'", 10, Want::Num(1.0, 10.0, 55.0))]));
}

/// LabVIEW's: every number after each segment's mask big-endian, and a second
/// segment that lists both channels again as laid out before and holds six
/// chunks of them.
#[test]
fn nptdms_big_endian_file_from_labview() {
    let layouts = skip_without_samples!(check(
        "nptdms-big_endian.tdms",
        &[
            ("/'Measured Data'/'Amplitude sweep'", 3500, Want::Num(0.0, 5.067986572324634, 92.4168263064218)),
            ("/'Measured Data'/'Phase sweep'", 3500, Want::Num(0.0, 0.8446644287207723, 24.607279472921544)),
        ],
    ));
    assert_eq!(layouts, ["contiguous", "contiguous"]);
}

#[test]
fn nptdms_file_of_128_doubles() {
    skip_without_samples!(check(
        "nptdms-raw_timestamps.tdms",
        &[("/'Untitled'/'Untitled'", 128, Want::Num(0.0, -0.04906767432741799, 3.247402347028583e-15))],
    ));
}

/// A DAQmx digital input log: metadata-only segments between the ones with
/// data, and three channels of the same name in three groups.
#[test]
fn nptdms_digital_input_log() {
    skip_without_samples!(check(
        "nptdms-Digital_Input.tdms",
        &[
            ("/'07/09/2012 06:58:23 PM - Digital Input - All Data'/'Dev1_port3_line7 - line 0'", 20000, Want::Num(0.0, 1.0, 10000.0)),
            ("/'07/09/2012 06:58:23 PM - Digital Input - Decimated Data_Level1'/'Dev1_port3_line7 - line 0'", 400, Want::Num(0.0, 1.0, 200.0)),
            ("/'07/09/2012 06:58:23 PM - Digital Input - Decimated Data_Level2'/'Dev1_port3_line7 - line 0'", 8, Want::Num(0.0, 1.0, 4.0)),
        ],
    ));
}

/// DAQmx raw data is named and left as bytes; its index is read in full.
#[test]
fn nptdms_daqmx_raw_data_is_named_and_left_as_bytes() {
    let layouts = skip_without_samples!(check("nptdms-raw1.tdms", &[]));
    assert_eq!(layouts, ["DAQmx"]);
    let (d, mut ev) = read("nptdms-raw1.tdms").unwrap();
    // The second segment's seven channels, each with one format changing
    // scaler and one raw data width, and 28,000 bytes of samples.
    let first = [1, 2, 3, 1, 0];
    assert_eq!(ev.node(&d, &[&first[..], &[0, 1]].concat()).unwrap().value, Value::Str("/'Layer Data'/'First  Channel'".into()));
    let index = [&first[..], &[2]].concat();
    assert_eq!(ev.node(&d, &index).unwrap().type_name, "DaqmxIndex");
    assert_eq!(ev.node(&d, &[&index[..], &[2]].concat()).unwrap().value, Value::UInt(2000));
    assert_eq!(ev.node(&d, &[&index[..], &[4]].concat()).unwrap().child_count, 1);
    let raw = ev.node(&d, &[1, 2, 4, 1]).unwrap();
    assert_eq!((raw.type_name.as_str(), raw.size_bits / 8), ("bytes[]", 28000));
}

/// The index file is the data file's segments with the raw data left out:
/// the same objects, `TDSh` for `TDSm`, and nothing after the metadata.
#[test]
fn an_index_file_holds_the_data_files_metadata_and_nothing_else() {
    let (d, mut ev) = skip_without_samples!(read("groups-types.tdms_index"));
    assert_eq!(ev.node(&d, &[]).unwrap().child_count, 1);
    assert_eq!(ev.node(&d, &[0, 0]).unwrap().value, Value::Enum { raw: 0x5444_5368, name: Some("TDSh".into()), hex: false });
    let objects = [0, 2, 3, 1];
    assert_eq!(ev.node(&d, &objects).unwrap().child_count, 11);
    assert_eq!(ev.node(&d, &[&objects[..], &[7, 0, 1]].concat()).unwrap().value, Value::Str("/'log'/'message'".into()));
    assert!(ev.node(&d, &[0, 2, 4]).unwrap().absent);
    let segment = ev.node(&d, &[0]).unwrap();
    assert_eq!(segment.offset_bits + segment.size_bits, d.len_bits());
}
