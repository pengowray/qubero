//! The pickles in the sample collection, read as the programs they are.
//!
//! `pickle/` holds two real-world files, the same fixture dumped at every
//! protocol from 0 to 5, and one file each for the corners no ordinary object
//! reaches: an extension code, a persistent id, an out-of-band buffer, a
//! payload too large to frame, and the opcodes CPython 3 reads and never
//! writes. Between them they use all sixty-eight opcodes, which is what makes
//! this worth running against files rather than against a fixture here.
//!
//! What it checks is the thing a pickle can be checked on and few formats can:
//! **the opcodes cover every byte, exactly**. A pickle has no padding, no
//! alignment and no directory, so the run of opcodes either lands on the full
//! stop or it does not, and an operand read one byte too wide puts every
//! opcode after it on the wrong byte. Adding the sizes up is the whole test.
//!
//! Like the rest of the collection this skips when the folder is not beside
//! the repository. Point `QUBERO_SAMPLES` at it to run it elsewhere.

use std::path::{Path, PathBuf};

use qubero_core::document::Document;
use qubero_core::eval::{Evaluator, FrameCell, Value};
use qubero_core::formats;
use qubero_core::source::{ChunkStore, MemSource};

#[test]
fn every_pickle_is_opcodes_all_the_way_to_the_full_stop() {
    let Some(dir) = folder() else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    let mut checked = 0;
    for path in pickles(&dir) {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let bytes = std::fs::read(&path).unwrap();

        // A file a Familiar Pickle Form matches whole is offered the
        // template that shows the data; everything else is the listing. A
        // form matches all of a file or none of it, so a file too long to
        // sniff whole is offered the listing however well it would match.
        let window = &bytes[..bytes.len().min(0x9000)];
        let matched = formats::pickle::familiar::recognise(&bytes).is_some();
        let want = match matched && window.len() == bytes.len() {
            true => "picklefpf",
            false => "pickle",
        };
        assert_eq!(
            formats::sniff(window, bytes.len() as u64),
            Some(want),
            "{name}: not recognised"
        );

        let doc = Document::new(MemSource(bytes.clone()));
        let mut ev = Evaluator::new(formats::builtin("pickle").unwrap());
        let root = ev.node(&doc, &[]).unwrap();
        assert_eq!(root.offset_bits, 0, "{name}");

        let mut ops = Vec::new();
        gather(&doc, &mut ev, &[], &mut ops, 0);
        assert!(!ops.is_empty(), "{name}: no opcodes");

        // Every byte belongs to exactly one opcode. Frames hold opcodes of
        // their own, so only the opcodes holding no others are counted, and
        // they have to come to the length of the file with nothing over.
        let mut covered: Vec<(u64, u64)> = ops.iter().map(|(_, at, size)| (*at, *size)).collect();
        covered.sort_unstable();
        let mut want = 0u64;
        for (at, size) in &covered {
            assert_eq!(*at, want, "{name}: a gap or an overlap before bit {at}");
            want = at + size;
        }
        assert_eq!(want, doc.len_bits(), "{name}: the opcodes do not reach the end of the file");

        let last = ops.last().unwrap();
        assert_eq!(last.0, "STOP", "{name}: the last opcode is {} rather than STOP", last.0);

        checked += 1;
        eprintln!("{name}: {} opcodes over {} bytes", ops.len(), doc.len_bits() / 8);
    }
    assert!(checked >= 10, "only {checked} pickles found; the folder is meant to hold more");
}

/// A pickle written at protocol 4 says so, and the first opcode is where it
/// says it. Worth its own check because `PROTO` is the one opcode a reader can
/// compare against something outside the file: the sample's own name.
#[test]
fn the_protocol_a_file_was_written_at_is_the_one_it_says() {
    let Some(dir) = folder() else { return };
    let mut checked = 0;
    for path in pickles(&dir) {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let Some(want) = name.strip_prefix("proto").and_then(|s| s.as_bytes().first()) else { continue };
        let want = i128::from(want - b'0');
        let doc = Document::new(MemSource(std::fs::read(&path).unwrap()));
        let mut ev = Evaluator::new(formats::builtin("pickle").unwrap());

        let first = ev.node(&doc, &[0]).unwrap();
        if want < 2 {
            assert_ne!(opcode(&first.name), "PROTO", "{name}: protocol {want} has no PROTO opcode");
            checked += 1;
            continue;
        }
        assert_eq!(opcode(&first.name), "PROTO", "{name}: does not open with PROTO");
        assert_eq!(ev.node(&doc, &[0, 1]).unwrap().value, Value::UInt(want as u128), "{name}: wrong protocol");
        checked += 1;
    }
    assert!(checked >= 6, "only {checked} named for their protocol");
}

/// A pickled array is read as the numbers it holds, not as the bytes they are
/// written in.
///
/// This is the whole point of running the program. A `BINBYTES` says nothing
/// about its contents; the dtype, the shape and the byte order are three other
/// opcodes, and they reach the data only across the unpickler's stack. So what
/// is checked here is that the crossing happened: the payload row of every
/// numpy and pandas sample is an array of the right element type, and the
/// object array, whose data is not a buffer at all, is left as bytes.
#[test]
fn an_array_is_read_as_the_numbers_it_holds() {
    let Some(dir) = folder() else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    let mut checked = 0;
    for path in pickles(&dir) {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let Some(rest) = ["proto4-numpy-", "proto3-numpy-", "proto5-pandas-"]
            .iter()
            .find_map(|prefix| name.strip_prefix(prefix))
        else {
            continue;
        };
        let doc = Document::new(MemSource(std::fs::read(&path).unwrap()));
        let mut ev = Evaluator::new(formats::builtin("pickle").unwrap());
        let mut typed = Vec::new();
        payloads(&doc, &mut ev, &[], &mut typed, 0);
        checked += 1;

        // An object array's data is a list of pickled objects rather than a
        // buffer, so nothing should have claimed it.
        if rest.starts_with("object-array") {
            assert!(typed.is_empty(), "{name}: an object array read as {typed:?}");
            continue;
        }
        assert!(!typed.is_empty(), "{name}: no array was read as one");

        if rest.starts_with("array") {
            assert_eq!(typed, vec![("f32 le".to_string(), 24)], "{name}");
        }
        // Every width numpy has, so a reader with one of them wrong fails
        // here rather than quietly reading half an array.
        if rest.starts_with("dtypes") {
            let widths: Vec<&str> = typed.iter().map(|(t, _)| t.as_str()).collect();
            for want in ["i8", "u8", "i16 le", "u16 le", "i32 le", "u32 le", "i64 le", "u64 le", "f16 le", "f32 le", "f64 le"] {
                assert!(widths.contains(&want), "{name}: no {want} among {widths:?}");
            }
        }
        if rest.starts_with("byte-order") {
            let widths: Vec<&str> = typed.iter().map(|(t, _)| t.as_str()).collect();
            assert!(widths.contains(&"f64 be"), "{name}: the big-endian column stayed little: {widths:?}");
            assert!(widths.contains(&"f64 le"), "{name}: {widths:?}");
        }
        // numpy 1 called the module `numpy.core`, and a pickle from before
        // 2024 is the commoner kind.
        if rest.starts_with("1-module-names") {
            assert_eq!(typed, vec![("f64 le".to_string(), 4)], "{name}");
        }
        eprintln!("{name}: {typed:?}");
    }
    assert!(checked >= 8, "only {checked} array samples found");
}

/// A packed record is read as the fields packed into it.
///
/// `datetime.datetime` writes its whole value as ten bytes and hands them to
/// the class. Nothing beside them says the year is big-endian, the
/// microseconds take three bytes, or that the top bit of the month is the
/// `fold` flag that tells the two two-o'clocks apart on the night a clock goes
/// back. The sample holds both folds, so the bit is read and not assumed.
#[test]
fn a_packed_date_is_read_as_the_fields_in_it() {
    let Some(dir) = folder() else { return };
    let path = dir.join("proto4-datetime.pickle");
    let Ok(bytes) = std::fs::read(&path) else { return };
    // The standard library form reads this file now, so the packed runs are
    // read as the values they hold rather than by the symbolic machine. Every
    // row is the text Python writes the value in, worked out from the bytes.
    let rows = familiar_rows(bytes);
    let said = |name: &str| row(&rows, name).value.clone();
    assert_eq!(said("date"), Value::Str("2026-09-06".into()));
    // Half past ten and fifteen seconds, in that order. Three one-byte fields
    // in a row will read as each other if two of them are swapped, and a
    // reading that only checked the date would not notice.
    assert_eq!(said("time"), Value::Str("10:30:15.250000".into()));
    assert_eq!(said("datetime"), Value::Str("2026-09-06T10:30:15.250000".into()));
    // Midnight with a zone on it, which is the offset from UTC written after
    // the clock, and half past two in April, which is the second two o'clock:
    // its fold bit sits in the top of the month byte and is not the month.
    assert_eq!(said("with a zone"), Value::Str("2026-09-06T00:00:00+00:00".into()));
    assert_eq!(said("folded"), Value::Str("2026-04-05T02:30:00".into()));
    assert_eq!(said("length of time"), Value::Str("3 days, 0:01:01.000007".into()));
    // The bytes are still where they were, and the row that holds them says
    // so: a datetime is ten of them and a date is four.
    let packed: Vec<u64> = rows.iter().filter(|r| r.name == "packed").map(|r| r.len).collect();
    assert_eq!(packed, vec![4, 6, 10, 10, 10]);
}

/// The opcode listing reads a packed date as its fields too.
///
/// A file no form matches is read by the symbolic machine, which finds the
/// same packed runs and lays each out as a `Date`, `Time` or `DateTime`
/// structure. This sample holds the dates of `proto4-datetime.pickle` beside
/// an instance of a class the writing script defined, which every form
/// refuses, so the listing is what reads it.
#[test]
fn the_listing_reads_a_packed_date_as_the_fields_in_it() {
    let Some(dir) = folder() else { return };
    let path = dir.join("unfamiliar-dates-beside-a-class.pickle");
    let Ok(bytes) = std::fs::read(&path) else { return };
    let form = formats::pickle::familiar::recognise(&bytes).map(|m| m.form);
    assert_eq!(form, None, "a form matched, so the listing no longer reads this file");
    let doc = Document::new(MemSource(bytes));
    let mut ev = Evaluator::new(formats::builtin("pickle").unwrap());

    let mut found = Vec::new();
    fields(&doc, &mut ev, &[], &mut found, 0);
    let named = |ty: &str, field: &str| -> Vec<i128> {
        found.iter().filter(|(t, f, _)| t == ty && f == field).map(|(_, _, v)| *v).collect()
    };
    assert_eq!(named("DateTime", "year"), vec![2026; 3], "three datetimes, all this year");
    assert_eq!(named("Date", "year"), vec![2026]);
    assert_eq!(named("Date", "month"), vec![9]);
    assert_eq!(named("Date", "day"), vec![6]);
    assert_eq!(named("Time", "microsecond"), vec![250_000]);
    // Half past ten and fifteen seconds, in that order. Three one-byte fields
    // in a row will read as each other if two of them are swapped, and every
    // other assertion here would still pass.
    assert_eq!(named("Time", "hour"), vec![10]);
    assert_eq!(named("Time", "minute"), vec![30]);
    assert_eq!(named("Time", "second"), vec![15]);
    // The three datetimes in the order the dict was filled in: the same time
    // of day, then midnight with a zone on it, then half past two in April.
    assert_eq!(named("DateTime", "day"), vec![6, 6, 5]);
    assert_eq!(named("DateTime", "hour"), vec![10, 0, 2]);
    assert_eq!(named("DateTime", "minute"), vec![30, 0, 30]);
    assert_eq!(named("DateTime", "second"), vec![15, 0, 0]);
    assert_eq!(named("DateTime", "microsecond"), vec![250_000, 0, 0]);
    // The last of the three is the second two o'clock, and the bit that says
    // so is the top of the month byte: 0x84 is April folded, not month 132.
    assert_eq!(named("DateTime", "fold"), vec![0, 0, 1]);
    assert_eq!(named("DateTime", "month"), vec![9, 9, 4]);
}

/// A naming row names, and leaves what the thing is to the row that makes it.
///
/// A pickle names a class, then calls it, then fills it in, and for a while
/// every one of those rows said "a numpy array": the same four words three
/// times before anything existed. Worse, the factory and the class it is
/// handed are two different names that both read as "a numpy array", so two
/// rows running said it about two different things. Naming is not making.
#[test]
fn a_naming_row_says_the_name_and_nothing_else() {
    let Some(dir) = folder() else { return };
    let mut checked = 0;
    for path in pickles(&dir) {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let doc = Document::new(MemSource(std::fs::read(&path).unwrap()));
        let mut ev = Evaluator::new(formats::builtin("pickle").unwrap());
        let mut rows = Vec::new();
        annotated(&doc, &mut ev, &[], &mut rows, 0);
        for (op, text) in &rows {
            if op == "STACK_GLOBAL" {
                assert!(text.starts_with("names "), "{name}: STACK_GLOBAL says {text:?}");
            }
            // GLOBAL spells the name out in its own operand, so a word from
            // this would be the row telling the reader what the row says.
            assert_ne!(op, "GLOBAL", "{name}: GLOBAL says {text:?}, which its operand already does");
        }
        checked += 1;
    }
    assert!(checked >= 10, "only {checked} pickles found");

    // And the phrase lands once. Two `STACK_GLOBAL`s and a `REDUCE` build one
    // array between them; only the `REDUCE` made anything.
    let Ok(bytes) = std::fs::read(dir.join("proto4-numpy-array.pickle")) else { return };
    let doc = Document::new(MemSource(bytes));
    let mut ev = Evaluator::new(formats::builtin("pickle").unwrap());
    let mut rows = Vec::new();
    annotated(&doc, &mut ev, &[], &mut rows, 0);
    if rows.iter().any(|(op, text)| op == "STOP" && text.starts_with("Matched a Familiar Pickle Form")) {
        assert_eq!(rows.len(), 1, "FPF bypasses symbolic annotations");
        return;
    }
    let said: Vec<&(String, String)> = rows.iter().filter(|(_, t)| t == "a numpy array").collect();
    assert_eq!(said.len(), 1, "one row makes the array, and these said so: {said:?}");
    assert_eq!(said[0].0, "REDUCE");
}

/// Every opcode that has something to say, as its name and what it said.
fn annotated(
    doc: &Document<MemSource>,
    ev: &mut Evaluator,
    at: &[usize],
    out: &mut Vec<(String, String)>,
    depth: usize,
) {
    if depth > 10 {
        return;
    }
    let Ok(node) = ev.node(doc, at) else { return };
    if node.type_name == "Op" {
        if let Ok(word) = ev.node(doc, &[at, &[1]].concat()) {
            if word.type_name == "computed text" {
                if let Value::Str(text) = &word.value {
                    if !text.is_empty() {
                        out.push((opcode(&node.name).to_string(), text.clone()));
                    }
                }
            }
        }
    }
    for i in 0..node.child_count as usize {
        let mut next = at.to_vec();
        next.push(i);
        annotated(doc, ev, &next, out, depth + 1);
    }
}

/// Every field of every packed record, as its record's type, its own name and
/// its value.
fn fields(
    doc: &Document<MemSource>,
    ev: &mut Evaluator,
    at: &[usize],
    out: &mut Vec<(String, String, i128)>,
    depth: usize,
) {
    if depth > 10 {
        return;
    }
    let Ok(node) = ev.node(doc, at) else { return };
    if matches!(node.type_name.as_str(), "Date" | "Time" | "DateTime") {
        for i in 0..node.child_count as usize {
            let mut child = at.to_vec();
            child.push(i);
            if let Ok(f) = ev.node(doc, &child) {
                if let Some(v) = f.value.as_int() {
                    out.push((node.type_name.clone(), f.name.clone(), v));
                }
            }
        }
        return;
    }
    for i in 0..node.child_count as usize {
        let mut next = at.to_vec();
        next.push(i);
        fields(doc, ev, &next, out, depth + 1);
    }
}

/// Each library's samples say what they are.
///
/// A pickle names a callable and nothing else, so a reader that does not know
/// the name shows a module path and leaves the rest to the reader's memory.
/// These are the names worth knowing, and the check is that the phrase reaches
/// the row rather than that any particular row has it: the module paths move
/// between versions and the phrases are matched on the part that does not.
#[test]
fn the_libraries_worth_knowing_are_named() {
    let Some(dir) = folder() else { return };
    let want: &[(&str, &str)] = &[
        ("proto4-numpy-array.pickle", "a numpy array"),
        ("proto5-pandas-dataframe.pickle", "a pandas DataFrame"),
        ("proto5-pandas-series.pickle", "a pandas Series"),
        ("proto5-pandas-index-types.pickle", "a pandas Categorical"),
        ("proto2-torch-state-dict.pickle", "a torch tensor"),
        ("proto4-scipy-csr-matrix.pickle", "a CSR sparse matrix"),
        ("proto4-scipy-csc-matrix.pickle", "a CSC sparse matrix"),
        ("proto4-scipy-coo-matrix.pickle", "a COO sparse matrix"),
        ("proto4-sklearn-random-forest.pickle", "a scikit-learn object"),
        ("proto4-sklearn-pipeline.pickle", "a scikit-learn object"),
        ("proto4-datetime.pickle", "a length of time"),
        ("proto4-collections.pickle", "a dict of counts"),
        ("proto4-builtins.pickle", "a slice"),
        // Python 2 spelled two of these modules differently, and a pickle
        // written then still says so.
        ("handmade-python2-modules.pickle", "an instance via its base class"),
        ("handmade-python2-modules.pickle", "a complex number"),
        ("handmade-python2-modules.pickle", "a range: start, stop and step"),
    ];
    let mut checked = 0;
    for (file, phrase) in want {
        let Ok(bytes) = std::fs::read(dir.join(file)) else { continue };
        // A matched file says one thing and stops: the form it matched, and
        // where to read what it holds.
        let matched = formats::pickle::familiar::recognise(&bytes)
            .map(|m| formats::pickle::familiar::stop_message(m.form));
        let phrase = matched.as_deref().unwrap_or(phrase);
        let doc = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(formats::builtin("pickle").unwrap());
        let mut said = Vec::new();
        words(&doc, &mut ev, &[], &mut said, 0);
        assert!(said.iter().any(|w| w.contains(phrase)), "{file}: nothing said {phrase:?}; it said {said:?}");
        checked += 1;
    }
    assert!(checked >= 10, "only {checked} of the library samples were there");
}

/// Everything the run of the file had to say, one string per row that said
/// anything.
fn words(doc: &Document<MemSource>, ev: &mut Evaluator, at: &[usize], out: &mut Vec<String>, depth: usize) {
    said(doc, ev, at, out, depth, None);
}

/// The same, restricted to one field: `operand` is what an opcode did, `holds`
/// is what a memo slot has in it, and the two answer different questions.
fn said(
    doc: &Document<MemSource>,
    ev: &mut Evaluator,
    at: &[usize],
    out: &mut Vec<String>,
    depth: usize,
    only: Option<&str>,
) {
    if depth > 10 {
        return;
    }
    let Ok(node) = ev.node(doc, at) else { return };
    if node.type_name == "computed text" {
        if let Value::Str(text) = &node.value {
            if !text.is_empty() && only.is_none_or(|f| node.name == f) {
                out.push(text.clone());
            }
        }
        return;
    }
    for i in 0..node.child_count as usize {
        let mut next = at.to_vec();
        next.push(i);
        said(doc, ev, &next, out, depth + 1, only);
    }
}

/// A file that has not all arrived yet is not run as a program.
///
/// The machine reads the whole file, and in the browser a file arrives a chunk
/// at a time. Running it over the chunks that have turned up, with zeros where
/// the rest will be, would give an answer about a file nobody has: a shape
/// read out of zeros, a length that happens to agree, an array of nothing. And
/// the answer is remembered, so it would still be wrong once the bytes landed.
///
/// So the run goes through the same read every field goes through, and says
/// "not yet" the same way. This feeds a real sample in one chunk at a time and
/// checks that the array reads as an array only once the last of it is there.
#[test]
fn a_file_still_arriving_is_not_run_as_a_program() {
    let Some(dir) = folder() else { return };
    let path = dir.join("proto4-numpy-array.pickle");
    let Ok(bytes) = std::fs::read(&path) else { return };

    const CHUNK: u64 = 64;
    let mut doc = Document::new(ChunkStore::new(bytes.len() as u64, CHUNK, 64));
    let mut ev = Evaluator::new(formats::builtin("pickle").unwrap());
    let chunks = bytes.len().div_ceil(CHUNK as usize);

    for n in 0..chunks {
        let from = n * CHUNK as usize;
        let to = (from + CHUNK as usize).min(bytes.len());
        doc.source_mut().insert(n as u64, bytes[from..to].to_vec().into_boxed_slice());
        let mut typed = Vec::new();
        chunked_payloads(&doc, &mut ev, &[], &mut typed, 0);
        // Nothing may be claimed until the file is whole, and everything must
        // be claimed once it is.
        match n + 1 == chunks {
            false => assert!(typed.is_empty(), "read {typed:?} from {} of {chunks} chunks", n + 1),
            true => assert_eq!(typed, vec![("f32 le".to_string(), 24)], "the whole file"),
        }
    }
}

/// The same walk as [`payloads`], over a source that may still be fetching.
/// A node that is not there yet is not a node that read as bytes, so the walk
/// simply stops there.
fn chunked_payloads(
    doc: &Document<ChunkStore>,
    ev: &mut Evaluator,
    at: &[usize],
    out: &mut Vec<(String, u64)>,
    depth: usize,
) {
    if depth > 10 {
        return;
    }
    let Ok(node) = ev.node(doc, at) else { return };
    if node.name == "value" && node.composite && node.type_name != "bytes[]" && node.type_name.ends_with("[]") {
        out.push((node.type_name.trim_end_matches("[]").to_string(), node.child_count));
        return;
    }
    for i in 0..node.child_count as usize {
        let mut next = at.to_vec();
        next.push(i);
        chunked_payloads(doc, ev, &next, out, depth + 1);
    }
}

/// Every payload that was read as an array rather than as bytes, as the
/// element's type and how many of them there are.
fn payloads(
    doc: &Document<MemSource>,
    ev: &mut Evaluator,
    at: &[usize],
    out: &mut Vec<(String, u64)>,
    depth: usize,
) {
    if depth > 10 {
        return;
    }
    let Ok(node) = ev.node(doc, at) else { return };
    // The bytes of a byte string, which the template gives the length of the
    // opcode and the machine gives a type to. Bytes that stayed bytes are not
    // counted: `bytes[]` is the default and means nothing was recognised.
    if node.name == "value" && node.composite && node.type_name != "bytes[]" && node.type_name.ends_with("[]") {
        let elem = node.type_name.trim_end_matches("[]").to_string();
        out.push((elem, node.child_count));
        return;
    }
    for i in 0..node.child_count as usize {
        let mut next = at.to_vec();
        next.push(i);
        payloads(doc, ev, &next, out, depth + 1);
    }
}

/// Every opcode in the file, in the order the bytes are in, as its name, where
/// it starts and how long it is. A frame contributes the opcodes inside it and
/// not itself, since those are the ones covering bytes.
fn gather(
    doc: &Document<MemSource>,
    ev: &mut Evaluator,
    at: &[usize],
    out: &mut Vec<(String, u64, u64)>,
    depth: usize,
) {
    assert!(depth < 8, "nested past anything a pickle does: {at:?}");
    let node = ev.node(doc, at).unwrap_or_else(|e| panic!("{at:?}: {e:?}"));
    if node.type_name == "Op" {
        if opcode(&node.name) != "FRAME" {
            out.push((opcode(&node.name).to_string(), node.offset_bits, node.size_bits));
            return;
        }
        // A frame covers its own header and then the opcodes inside it. The
        // header is what is left of the frame once its contents are taken off:
        // the opcode byte and the eight-byte length.
        let inside: Vec<usize> = at.iter().copied().chain([1, 1]).collect();
        let ops = ev.node(doc, &inside).unwrap_or_else(|e| panic!("{inside:?}: {e:?}"));
        out.push(("FRAME".to_string(), node.offset_bits, node.size_bits - ops.size_bits));
        gather(doc, ev, &inside, out, depth + 1);
        return;
    }
    for i in 0..node.child_count as usize {
        let mut next = at.to_vec();
        next.push(i);
        gather(doc, ev, &next, out, depth + 1);
    }
}

/// An element of a list is named for what it is with its index in front of it,
/// which is `[3] BININT1`. The opcode is the rest.
fn opcode(name: &str) -> &str {
    name.rsplit_once("] ").map_or(name, |(_, rest)| rest)
}

fn folder() -> Option<PathBuf> {
    qubero_samples::dir("pickle")
}

fn pickles(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "pickle"))
        // Syncthing leaves a copy of a file beside it when two machines wrote
        // it, named with the word below. It is not a sample of the collection
        // and the collection does not track it, so it is passed over rather
        // than counted as a file nothing accounts for.
        .filter(|p| !p.to_string_lossy().contains("sync-conflict"))
        .collect();
    out.sort();
    out
}

/// Which of the samples a Familiar Pickle Form matches, file by file.
///
/// Written out rather than counted, because both halves matter and neither is
/// a number. A file that starts matching is a form that grew without anyone
/// saying so, and a file that stops matching is coverage lost; the long half
/// is the one that has to keep failing, since a form accepting a scikit-learn
/// estimator or an out-of-band buffer would be publishing values it never
/// validated.
#[test]
fn the_forms_match_these_samples_and_no_others() {
    let Some(dir) = folder() else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    // The whole corpus as it stands, with the form each file matches. Keep
    // this in step with the collection: a file added to it belongs here.
    let want: &[(&str, Option<&str>)] = &[
        ("awa2-pose-antelope.pickle", Some("basic-p4-p5-v5")),
        ("awa2-pose-elephant.pickle", Some("basic-p4-p5-v5")),
        // A dictionary whose big value is written between two frames.
        ("proto4-unframed-payload.pickle", Some("basic-p4-p5-v5")),
        ("proto4-numpy-array.pickle", Some("numpy-array-p4-p5-v6")),
        // Several arrays in one dictionary, the later ones naming numpy's
        // globals, dtype class, byte order or whole dtype out of the memo.
        ("proto4-numpy-byte-order.pickle", Some("numpy-array-p4-p5-v6")),
        ("proto4-numpy-dtypes.pickle", Some("numpy-array-p4-p5-v6")),
        ("proto4-numpy-shapes.pickle", Some("numpy-array-p4-p5-v6")),
        ("proto4-numpy-shared-dtype.pickle", Some("numpy-array-p4-p5-v6")),
        ("proto4-builtins.pickle", Some("builtins-values-p4-p5-v3")),
        // The files written to say what a form takes and what it does not.
        // Every `familiar-` one is plain data written the ordinary way, and
        // every `unfamiliar-` one is a pickle Python loads and a form must
        // still refuse. The two halves are the test: a form that grew far
        // enough to read the second half would be reading a class, a value
        // with no bytes of its own, or a program CPython did not write.
        ("familiar-records.pickle", Some("basic-p4-p5-v5")),
        ("familiar-long-containers.pickle", Some("basic-p4-p5-v5")),
        ("familiar-mixed-keys.pickle", Some("basic-p4-p5-v5")),
        ("familiar-tuples-and-sets.pickle", Some("basic-p4-p5-v5")),
        ("familiar-big-integers.pickle", Some("basic-p4-p5-v5")),
        ("familiar-bytearray-p5.pickle", Some("basic-p4-p5-v5")),
        // Below protocol 5 a bytearray is a call to the class, which is the
        // other form.
        ("familiar-bytearray-p4.pickle", Some("builtins-values-p4-p5-v3")),
        // The same list of 1,001 the C pickler wrote in `familiar-long-
        // containers`, written by the one in `pickle.py`, which ends it with
        // APPEND where the C one writes a batch of one. Both spellings are
        // what a real pickler writes, so a form reads either.
        ("familiar-pure-python-batches.pickle", Some("basic-p4-p5-v5")),
        // An array whose numbers are too large to frame, so the frame
        // boundary lands inside the run of instructions that rebuilds it
        // rather than between two values of the file.
        ("familiar-numpy-large-p5.pickle", Some("numpy-array-p4-p5-v6")),
        // One list in two places, and one holding itself. Both are a name
        // pointing at a container, which the basic form reads as a reference
        // saying what it names and where the file wrote it.
        ("familiar-shared-list.pickle", Some("basic-p4-p5-v5")),
        ("familiar-recursive-list.pickle", Some("basic-p4-p5-v5")),
        // Two to the two hundredth, which LONG1 writes in twenty-six bytes.
        // No integer type here is that wide, so the number is the digits it
        // comes to with the run beneath them.
        ("familiar-huge-integer.pickle", Some("basic-p4-p5-v5")),
        // An instance of a class the file names.
        ("unfamiliar-class-instance.pickle", None),
        // Packed dates beside one of those, which the one instance is enough
        // to hold back: the file stays with the opcode listing, and the
        // listing's own reading of the packed runs is what it tests.
        ("unfamiliar-dates-beside-a-class.pickle", None),
        // A valid program CPython did not write: `pickletools.optimize` drops
        // the memo marks, which no pickler does.
        ("unfamiliar-optimized.pickle", None),
        // A string that is not UTF-8, because it holds half a surrogate pair.
        ("unfamiliar-lone-surrogate.pickle", None),
        // The rest, none of which any form accepts yet. The library files
        // need forms of their own, built from reviewed complete structures.
        //
        // `proto4-collections` is held back by a namedtuple class the writing
        // file defined, handed values by NEWOBJ. A form that took it would be
        // accepting a class under `__main__`, which is the one thing the
        // contract rules out. Its `OrderedDict`, `defaultdict`, `Counter` and
        // `deque` are read.
        ("handmade-python2-modules.pickle", None),
        ("handmade-python2-opcodes.pickle", None),
        ("handmade-wide-lengths.pickle", None),
        ("proto0-everything.pickle", Some("stdlib-values-p0-v2")),
        ("proto0-persistent-id.pickle", None),
        ("proto1-everything.pickle", Some("stdlib-values-p1-v2")),
        ("proto2-everything.pickle", Some("stdlib-values-p2-p3-v2")),
        ("proto2-extension-registry.pickle", None),
        ("proto2-memo-over-256.pickle", Some("basic-p2-p3-v1")),
        ("proto2-torch-state-dict.pickle", Some("torch-tensors-p2-p3-v1")),
        ("proto3-everything.pickle", Some("stdlib-values-p2-p3-v2")),
        ("proto3-numpy-1-module-names.pickle", Some("numpy-array-p2-p3-v1")),
        ("proto4-collections.pickle", None),
        // Packed dates, times and spans, each a call of its class with the
        // run of bytes `_getstate` packed it into.
        ("proto4-datetime.pickle", Some("stdlib-values-p4-p5-v2")),
        // A `ValueError` rebuilt from its message, beside the dates and the
        // exact numbers: an exception is one of the builtin exception classes
        // called with the arguments it was raised with.
        ("proto4-everything.pickle", Some("stdlib-values-p4-p5-v2")),
        // Six exceptions: raised with nothing, with a message, with the
        // errno, strerror and file name an OSError carries, with an attribute
        // of its own through a BUILD, with the tuple a SyntaxError holds, and
        // a group holding two more.
        ("proto4-exceptions.pickle", Some("stdlib-values-p4-p5-v2")),
        // The two structseq classes: a run of whole numbers, and a dictionary
        // of the fields past the end of that run.
        ("proto4-structseq.pickle", Some("stdlib-values-p4-p5-v2")),
        ("proto4-newobj.pickle", None),
        // An array of objects, which is what `pickle.dumps` writes for one and
        // NumPy's own writing: the values are pickled after the array as the
        // list it is handed, and they are read against the same stack.
        ("proto4-numpy-object-array.pickle", Some("numpy-array-p4-p5-v6")),
        ("proto4-persistent-id.pickle", None),
        // Three sparse matrices and a pipeline of two estimators: an object of
        // a class named from a whitelisted module, made with no arguments and
        // given a dictionary of attributes by BUILD.
        ("proto4-scipy-coo-matrix.pickle", Some("scipy-sparse-p4-p5-v1")),
        ("proto4-scipy-csc-matrix.pickle", Some("scipy-sparse-p4-p5-v1")),
        ("proto4-scipy-csr-matrix.pickle", Some("scipy-sparse-p4-p5-v1")),
        ("proto4-sklearn-pipeline.pickle", Some("sklearn-estimator-p4-p5-v1")),
        // A random forest holds decision trees, each with a
        // `sklearn.tree._tree.Tree` rebuilt by REDUCE from a structured array
        // of nodes.
        ("proto4-sklearn-random-forest.pickle", Some("sklearn-estimator-p4-p5-v1")),
        ("proto5-everything.pickle", Some("stdlib-values-p4-p5-v2")),
        ("proto5-out-of-band.pickle", None),
        // A frame and a series, each a block manager over blocks of columns
        // and the two axes.
        ("proto5-pandas-dataframe.pickle", Some("pandas-frame-p4-p5-v1")),
        ("proto5-pandas-series.pickle", Some("pandas-frame-p4-p5-v1")),
        // Two frames, the second made from the first by `assign`. They share
        // the slices that place their blocks, so the second frame names each
        // placement out of the memo rather than spelling it again.
        ("mixed-frames-sharing-placements-p4.pickle", Some("pandas-frame-p4-p5-v1")),
        ("mixed-frames-sharing-placements-p2.pickle", Some("pandas-frame-p2-p3-v1")),
        // Every index kind, the datetime one included.
        ("proto5-pandas-index-types.pickle", Some("pandas-frame-p4-p5-v1")),
        // Files holding values of more than one family, which is what an
        // ordinary pickle of a program's state is. Each is written at
        // protocol 4 and at protocol 2, which are two grammars over the same
        // data. No single-family form takes one of them: each requires the
        // file to have used its own production and nothing else's, and the
        // mixed form is the union of all of them.
        ("mixed-date-and-array-p4.pickle", Some("mixed-values-p4-p5-v2")),
        ("mixed-date-and-array-p2.pickle", Some("mixed-values-p2-p3-v2")),
        ("mixed-ordereddict-of-arrays-p4.pickle", Some("mixed-values-p4-p5-v2")),
        ("mixed-ordereddict-of-arrays-p2.pickle", Some("mixed-values-p2-p3-v2")),
        ("mixed-model-and-metadata-p4.pickle", Some("mixed-values-p4-p5-v2")),
        ("mixed-model-and-metadata-p2.pickle", Some("mixed-values-p2-p3-v2")),
        // Two frames sharing their placements, which pandas writes as a block
        // placed by an array of positions rather than by a slice. No form
        // reads one; `HANDOVER-pickle-libraries.md` has it as the fourth
        // thing left to do, and the samples arrived before the production.
        ("mixed-frames-sharing-placements-p4.pickle", None),
        ("mixed-frames-sharing-placements-p2.pickle", None),
        ("mixed-frame-and-notes-p4.pickle", Some("mixed-values-p4-p5-v2")),
        ("mixed-frame-and-notes-p2.pickle", Some("mixed-values-p2-p3-v2")),
        ("mixed-decimal-and-array-p4.pickle", Some("mixed-values-p4-p5-v2")),
        ("mixed-decimal-and-array-p2.pickle", Some("mixed-values-p2-p3-v2")),
        // One family all the way down, kept beside them: it keeps the name it
        // already had rather than being read as a mixture of one.
        ("mixed-list-of-frames-p4.pickle", Some("pandas-frame-p4-p5-v1")),
        ("mixed-list-of-frames-p2.pickle", Some("pandas-frame-p2-p3-v1")),
        // Columns of objects holding more than text. A column of dates or of
        // exact numbers uses the standard library's productions, so the frame
        // is a mixture; a column of lists is containers of plain values, which
        // every form already reads, so that frame keeps the pandas form.
        ("mixed-frame-of-dates-p4.pickle", Some("mixed-values-p4-p5-v2")),
        ("mixed-frame-of-dates-p2.pickle", Some("mixed-values-p2-p3-v2")),
        ("mixed-frame-of-decimals-p4.pickle", Some("mixed-values-p4-p5-v2")),
        ("mixed-frame-of-decimals-p2.pickle", Some("mixed-values-p2-p3-v2")),
        ("mixed-frame-of-lists-p4.pickle", Some("pandas-frame-p4-p5-v1")),
        ("mixed-frame-of-lists-p2.pickle", Some("pandas-frame-p2-p3-v1")),
        // An array of objects holding tuples, beside a date so that the file
        // is a mixture: an array of objects alone is numpy and nothing else.
        ("mixed-array-of-tuples-p4.pickle", Some("mixed-values-p4-p5-v2")),
        ("mixed-array-of-tuples-p2.pickle", Some("mixed-values-p2-p3-v2")),
        // The same column of objects holding instances of a class no form
        // names, which is where the widening stops.
        ("unfamiliar-frame-of-instances-p4.pickle", None),
        ("unfamiliar-frame-of-instances-p2.pickle", None),
        // The estimators people actually save, each fitted on sixteen rows of
        // three features and pickled at protocol 4. `tools/make_sklearn_
        // breadth_samples.py` in the collection writes them, and the same
        // objects are in `joblib/` under `joblib.dump`.
        ("sklearn-logistic-regression.pickle", Some("sklearn-estimator-p4-p5-v1")),
        // Fitted on labels that are strings, so `classes_` is an array of
        // fixed-width text, whose width is in the dtype's state rather than
        // in its letters. This is the file that had no reading at all.
        ("sklearn-logistic-regression-string-labels.pickle", Some("sklearn-estimator-p4-p5-v1")),
        ("sklearn-ridge.pickle", Some("sklearn-estimator-p4-p5-v1")),
        ("sklearn-lasso.pickle", Some("sklearn-estimator-p4-p5-v1")),
        // Its `_loss_function_` is a Cython class rebuilt by REDUCE from the
        // one number it was configured with.
        ("sklearn-sgd-classifier.pickle", Some("sklearn-estimator-p4-p5-v1")),
        ("sklearn-linear-svc.pickle", Some("sklearn-estimator-p4-p5-v1")),
        // Its `_probA` and `_probB` are both empty, which is one byte string
        // to Python, so the second array names the run the first wrote.
        ("sklearn-svc.pickle", Some("sklearn-estimator-p4-p5-v1")),
        // A KD-tree and a distance metric, each made by the `newObj` its own
        // Cython module keeps and filled by the BUILD after it.
        ("sklearn-k-neighbors-classifier.pickle", Some("sklearn-estimator-p4-p5-v1")),
        ("sklearn-gaussian-nb.pickle", Some("sklearn-estimator-p4-p5-v1")),
        ("sklearn-decision-tree-regressor.pickle", Some("sklearn-estimator-p4-p5-v1")),
        ("sklearn-random-forest-classifier.pickle", Some("sklearn-estimator-p4-p5-v1")),
        // `estimators_` is an array of objects, one tree per stage, and the
        // loss and its link are `sklearn._loss`'s own classes.
        ("sklearn-gradient-boosting-classifier.pickle", Some("sklearn-estimator-p4-p5-v1")),
        // A `Generator` in `_feature_subsample_rng`, and a structured dtype
        // one of whose columns is named out of the memo.
        ("sklearn-hist-gradient-boosting-classifier.pickle", Some("sklearn-estimator-p4-p5-v1")),
        ("sklearn-kmeans.pickle", Some("sklearn-estimator-p4-p5-v1")),
        ("sklearn-pca.pickle", Some("sklearn-estimator-p4-p5-v1")),
        ("sklearn-standard-scaler.pickle", Some("sklearn-estimator-p4-p5-v1")),
        ("sklearn-min-max-scaler.pickle", Some("sklearn-estimator-p4-p5-v1")),
        // Each keeps the NumPy scalar type it will make its numbers in, which
        // is a global the form names and never calls.
        ("sklearn-one-hot-encoder.pickle", Some("sklearn-estimator-p4-p5-v1")),
        ("sklearn-count-vectorizer.pickle", Some("sklearn-estimator-p4-p5-v1")),
        ("sklearn-tfidf-vectorizer.pickle", Some("sklearn-estimator-p4-p5-v1")),
        ("sklearn-label-encoder.pickle", Some("sklearn-estimator-p4-p5-v1")),
        ("sklearn-simple-imputer.pickle", Some("sklearn-estimator-p4-p5-v1")),
        // A `RandomState` in `_random_state`, which is a bit generator, the
        // numbers it has not used yet and where in them it has got to.
        ("sklearn-mlp-classifier.pickle", Some("sklearn-estimator-p4-p5-v1")),
        ("sklearn-pipeline-scaler-and-model.pickle", Some("sklearn-estimator-p4-p5-v1")),
        // Two that hold a second family beside scikit-learn's: a column
        // transformer places its columns with slices, which are the
        // language's own values, and a nearest-neighbour model fitted on a
        // sparse matrix keeps that matrix, which is scipy's.
        ("sklearn-column-transformer.pickle", Some("mixed-values-p4-p5-v2")),
        ("sklearn-k-neighbors-classifier-sparse-input.pickle", Some("mixed-values-p4-p5-v2")),
        // NumPy's own array classes, each of which reaches the reconstructor
        // an ndarray reaches and names itself in it. A matrix is an array
        // held to two dimensions; a memmap is an array a reader may keep in
        // a file, and what it is pickled with is the numbers themselves.
        ("proto4-numpy-matrix.pickle", Some("numpy-array-p4-p5-v6")),
        ("proto2-numpy-matrix.pickle", Some("numpy-array-p2-p3-v1")),
        ("proto4-numpy-memmap.pickle", Some("numpy-array-p4-p5-v6")),
        ("proto2-numpy-memmap.pickle", Some("numpy-array-p2-p3-v1")),
        // `cv_results_` is a dictionary of `numpy.ma.MaskedArray`, which is
        // an array, a mask of which of its entries count and the value a
        // masked entry stands for, rebuilt by `numpy.ma.core._mareconstruct`.
        ("sklearn-grid-search-cv.pickle", Some("sklearn-estimator-p4-p5-v1")),
        // The masked array on its own, which is NumPy's and nothing else's.
        // Two of four entries hidden, the same over two dimensions, one with
        // nothing hidden, whose mask `__getstate__` writes out in full
        // anyway, and one with a fill value the array was given rather than
        // NumPy's default for its dtype.
        ("proto4-numpy-masked-array.pickle", Some("numpy-array-p4-p5-v6")),
        ("proto2-numpy-masked-array.pickle", Some("numpy-array-p2-p3-v1")),
        ("proto4-numpy-masked-2d.pickle", Some("numpy-array-p4-p5-v6")),
        ("proto2-numpy-masked-2d.pickle", Some("numpy-array-p2-p3-v1")),
        ("proto4-numpy-masked-unmasked.pickle", Some("numpy-array-p4-p5-v6")),
        ("proto2-numpy-masked-unmasked.pickle", Some("numpy-array-p2-p3-v1")),
        ("proto4-numpy-masked-fill-value.pickle", Some("numpy-array-p4-p5-v6")),
        ("proto2-numpy-masked-fill-value.pickle", Some("numpy-array-p2-p3-v1")),
    ];
    let mut seen = Vec::new();
    for path in pickles(&dir) {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let bytes = std::fs::read(&path).unwrap();
        let form = formats::pickle::familiar::recognise(&bytes).map(|m| m.form);
        let Some((_, expected)) = want.iter().find(|(f, _)| *f == name) else {
            panic!("{name} is not in the matrix; add it with the form it matches, or None");
        };
        assert_eq!(form, *expected, "{name}");
        seen.push(name);
    }
    for (name, _) in want {
        assert!(seen.iter().any(|s| s == name), "{name} is in the matrix and not in the collection");
    }
}

/// What each object in the matrix comes to, per library family and per object
/// where one object of a family reads and another does not yet, at protocol 4
/// and 5 and then at protocol 2 and 3.
///
/// A file's family is the word in front of the first dash of its name, and its
/// object is everything in front of the first dot. The row is the object where
/// there is one and the family otherwise, so the day a form reaches one more
/// object its `None` here becomes that form's ID and nothing else changes.
///
/// The two columns are two grammars over the same data: below protocol 4 a
/// memo mark carries its index, a callable is two lines, and a set and a byte
/// string are calls rather than literals.
const FAMILIES: &[(&str, [Option<&str>; 4])] = &[
    ("basic", [Some("basic-p4-p5-v5"), Some("basic-p2-p3-v1"), Some("basic-p1-v1"), Some("basic-p0-v1")]),
    // An array and a scalar, which are two productions of one form.
    ("numpy", [Some("numpy-array-p4-p5-v6"), Some("numpy-array-p2-p3-v1"), Some("numpy-array-p1-v1"), Some("numpy-array-p0-v1")]),
    ("dataframe", [Some("pandas-frame-p4-p5-v1"), Some("pandas-frame-p2-p3-v1"), Some("pandas-frame-p1-v1"), Some("pandas-frame-p0-v1")]),
    ("series", [Some("pandas-frame-p4-p5-v1"), Some("pandas-frame-p2-p3-v1"), Some("pandas-frame-p1-v1"), Some("pandas-frame-p0-v1")]),
    ("sklearn", [
        Some("sklearn-estimator-p4-p5-v1"),
        Some("sklearn-estimator-p2-p3-v1"),
        Some("sklearn-estimator-p1-v1"),
        Some("sklearn-estimator-p0-v1"),
    ]),
    ("scipy", [Some("scipy-sparse-p4-p5-v1"), Some("scipy-sparse-p2-p3-v1"), Some("scipy-sparse-p1-v1"), Some("scipy-sparse-p0-v1")]),
    // The standard library's own classes: dates, ordered and defaulting
    // dictionaries, counters, queues, exact numbers, ids and paths.
    ("stdlib", [Some("stdlib-values-p4-p5-v2"), Some("stdlib-values-p2-p3-v2"), Some("stdlib-values-p1-v2"), Some("stdlib-values-p0-v2")]),
    // The two whose values are builtins and nothing else, which the builtins
    // form already reads and keeps reading. A file has to use a form's own
    // productions to be read under it, and neither of these names a class from
    // the standard library at all.
    ("stdlib-complex", [
        Some("builtins-values-p4-p5-v3"),
        Some("builtins-values-p2-p3-v1"),
        Some("builtins-values-p1-v1"),
        Some("builtins-values-p0-v1"),
    ]),
    ("stdlib-range-slice", [
        Some("builtins-values-p4-p5-v3"),
        Some("builtins-values-p2-p3-v1"),
        Some("builtins-values-p1-v1"),
        Some("builtins-values-p0-v1"),
    ]),
];

/// The row of [`FAMILIES`] a file falls under: its object where that is named,
/// and its family otherwise.
fn family_of(name: &str) -> usize {
    let object = name.split('.').next().unwrap_or("");
    let family = name.split('-').next().unwrap_or("");
    let row = FAMILIES.iter().position(|(f, ..)| *f == object);
    match row.or_else(|| FAMILIES.iter().position(|(f, ..)| *f == family)) {
        Some(i) => i,
        None => panic!("{name}: no row called {object:?} or {family:?} in FAMILIES; add it with the forms it matches, or None"),
    }
}

/// The files in the matrix that no form reads. Empty, and kept: a form that
/// stops reading a file it read is what this catches, and the list is where
/// such a file would be written down with the reason.
const UNREAD: &[&str] = &[];

/// The same objects as sixteen environments wrote them, from Python 2.7 to
/// 3.14 and PyPy 2.7 and 3.10, with numpy 1.19 to 2.5 beside them, and from
/// the four interpreters outside that family: Jython 2.7, IronPython 2.7 and
/// 3.4 and GraalPy for Python 3.11. Every protocol each has and every pickler
/// each ships: `pickle-matrix/` in the collection, where a file is kept once
/// under the oldest environment that wrote those bytes.
///
/// A rule rather than a list, since the rule is what the forms claim: plain
/// data and numpy arrays and scalars match at protocol 4 and 5 whoever wrote
/// them, and nothing else matches at all. Not the older protocols, which the
/// forms do not take yet, and not pandas, scikit-learn or scipy, which have no
/// form. A library file that starts matching is a form that grew without being
/// asked.
#[test]
fn the_forms_match_every_environment_s_plain_data_and_arrays() {
    let Some(root) = qubero_samples::dir("pickle-matrix") else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    let mut environments: Vec<PathBuf> = std::fs::read_dir(&root).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
    environments.sort();
    assert!(environments.len() >= 16, "only {} environments under {}", environments.len(), root.display());
    // Every folder has to be one this test knows the rule for. A new
    // environment arriving with no line here is a run that silently covers
    // less than it says it does.
    const KNOWN: &[&str] = &["py2.7", "pypy2.7", "ironpython2.7", "jython2.7", "py3.4", "ironpython3.4", "py3.6", "py3.7", "py3.8", "py3.10", "pypy3.10", "graalpy3.11", "py3.12", "py3.13", "py3.14"];
    for dir in &environments {
        let env = dir.file_name().unwrap().to_string_lossy().into_owned();
        assert!(KNOWN.iter().any(|k| env == *k || env.starts_with(&format!("{k}-"))), "{env} is a new environment; add it to KNOWN and say what it writes");
    }
    // How many files of each family matched at each of the two protocol
    // ranges, and how many there were, so that the run says what it covered
    // rather than only that it passed.
    let mut tally: Vec<[(usize, usize); 4]> = vec![[(0, 0); 4]; FAMILIES.len()];
    let mut wrong = Vec::new();
    for dir in &environments {
        let env = dir.file_name().unwrap().to_string_lossy().into_owned();
        for path in pickles(dir) {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            let i = family_of(&name);
            // Which of the two grammars the file is written in, from either
            // pickler: a `.pypickle` file was written by `pickle.py` alone, a
            // `.cpickle` one by Python 2's `cPickle`, and every spelling of
            // theirs is familiar. Protocols 0 and 1 are still nobody's.
            let column = match true {
                _ if name.contains(".p4.") || name.contains(".p5.") => Some(0),
                _ if name.contains(".p2.") || name.contains(".p3.") => Some(1),
                _ if name.contains(".p1.") => Some(2),
                _ if name.contains(".p0.") => Some(3),
                _ => None,
            };
            let mut expected = column.and_then(|c| FAMILIES[i].1[c]);
            if UNREAD.contains(&format!("{env}/{name}").as_str()) {
                expected = None;
            }
            let bytes = std::fs::read(&path).unwrap();
            let form = formats::pickle::familiar::recognise(&bytes).map(|m| m.form);
            if let Some(column) = column {
                tally[i][column].1 += 1;
                tally[i][column].0 += usize::from(form.is_some());
            }
            if form != expected {
                let hint = match expected {
                    None => format!("; if a form for {} has landed, its row in FAMILIES is the edit", FAMILIES[i].0),
                    _ => String::new(),
                };
                wrong.push(format!("{env}/{name}: {form:?}, not {expected:?}{hint}"));
            }
        }
    }
    for ((family, forms), counts) in FAMILIES.iter().zip(&tally) {
        for (at, said) in ["protocol 4 and 5", "protocol 2 and 3", "protocol 1", "protocol 0"].iter().enumerate() {
            eprintln!("{family}: {} of {} at {said} read as {:?}", counts[at].0, counts[at].1, forms[at]);
        }
    }
    assert!(wrong.is_empty(), "{} files:\n  {}", wrong.len(), wrong.join("\n  "));
    // Fewer than were written, since the same bytes from two environments are
    // kept once, and enough to know the folder was not empty.
    let matched: usize = tally.iter().flatten().map(|(n, _)| n).sum();
    assert!(matched >= 520, "only {matched} files matched a form");
}

/// Which of CPython's two picklers each batch edge in the matrix shows.
///
/// The two agree everywhere but the tail of a container longer than a batch,
/// so these are the files that say anything at all, and most files say the
/// third thing. The strings are what the `pickler` row of the familiar-form
/// template shows, so they are written out here rather than named.
#[test]
fn the_batch_edges_say_which_pickler_wrote_them() {
    let Some(root) = qubero_samples::dir("pickle-matrix") else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    const C: &str = "_pickle (CPython's in C, or GraalPy's in Java)";
    const PY: &str = "pickle.py (pure Python), or an IronPython pickler (C#)";
    const EITHER: &str = "any (every known pickler writes this data the same way)";
    // Python 2's third pickler, which is a different program from Python 3's
    // C one and is known by the slot it starts the memo at rather than by a
    // batch edge. PyPy 2.7's copy of it numbers the same way, and so does
    // Jython's, which is written in Java.
    const CPICKLE: &str = "cPickle (Python 2's in C, PyPy 2.7's in Python, or Jython's in Java)";
    // The three sharper readings: a spelling that narrows the broad one above
    // it to one program.
    const JYTHON: &str = "cPickle (Jython's, in Java)";
    const IRON: &str = "cPickle (IronPython's, in C#)";
    const GRAAL: &str = "_pickle (GraalPy's, in Java)";
    // A file is kept under the oldest environment that wrote those bytes, so
    // every one of these is Python 3.4's copy, and each is also what every
    // later CPython wrote. `basic-list-1001.p4.pypickle.pickle` is byte for
    // byte what PyPy 3.10 wrote as well, which is the PyPy claim: PyPy has
    // only the pure Python pickler, and this is that pickler's spelling.
    let want: &[(&str, &str)] = &[
        // One item over a full batch: a batch of one from the C pickler, and
        // an APPEND or a SETITEM from the other.
        ("py3.4/basic-list-1001.p4.pickle", C),
        ("py3.4/basic-list-1001.p4.pypickle.pickle", PY),
        ("py3.4/basic-dict-1001.p4.pickle", C),
        ("py3.4/basic-dict-1001.p4.pypickle.pickle", PY),
        // Exactly a full batch and nothing left: the C pickler writes the
        // empty batch that says so, and pickle.py writes nothing.
        ("py3.4/basic-dict-1000.p4.pickle", C),
        ("py3.4/basic-dict-1000.p4.pypickle.pickle", PY),
        // A set of 1,001, where neither pickler has a shorthand, and a list
        // of exactly 1,000, which both end the same way.
        ("py3.4/basic-set-1001.p4.pickle", EITHER),
        ("py3.4/basic-list-1000.p4.pickle", EITHER),
        // And a file with no batch edge in it at all, which is most files.
        ("py3.4/basic-records.p4.pickle", EITHER),
        // The same edges at protocol 3 and 2, where the tells are the same
        // ones: the two picklers part at the tail of a long container and
        // nowhere else.
        ("py3.4/basic-list-1001.p3.pickle", C),
        ("py3.4/basic-list-1001.p3.pypickle.pickle", PY),
        ("py3.4/basic-dict-1000.p3.pickle", C),
        ("py3.4/basic-dict-1000.p3.pypickle.pickle", PY),
        ("py3.4/basic-list-1001.p2.pickle", C),
        ("py3.4/basic-dict-1001.p2.pickle", C),
        ("py3.4/basic-list-1000.p3.pickle", EITHER),
        // A set of 1,001 does say which pickler wrote it below protocol 4,
        // where a set is a call over a list rather than a container of its
        // own: the list ends the way that pickler ends a list.
        ("py3.4/basic-set-1001.p3.pickle", C),
        ("py3.4/basic-set-1001.p3.pypickle.pickle", PY),
        ("py3.4/basic-set-1001.p2.pickle", C),
        // Python 2. `pickle.py` there is the pure pickler and ends every
        // container its way; `cPickle` numbers the memo from one, which is
        // what says it wrote the file whether or not a batch edge is in it.
        ("py2.7/basic-list-1001.p2.pickle", PY),
        ("py2.7/basic-dict-1000.p2.pickle", PY),
        ("py2.7/basic-set-1001.p2.pickle", PY),
        ("py2.7/basic-list-1000.p2.pickle", EITHER),
        ("py2.7/basic-records.p2.pickle", EITHER),
        ("py2.7/basic-list-1001.p2.cpickle.pickle", CPICKLE),
        ("py2.7/basic-dict-1000.p2.cpickle.pickle", CPICKLE),
        // A file with no batch edge at all still says `cPickle`, because the
        // memo numbering says it.
        ("py2.7/basic-records.p2.cpickle.pickle", CPICKLE),
        // The four interpreters outside CPython's family. Each has one
        // spelling of its own, except IronPython 3.4, whose single pickler
        // writes what `pickle.py` writes and says nothing more than that.
        //
        // A batch of 1,024, or a list of one filled by APPENDS rather than
        // APPEND. Either narrows "numbers the memo from one" to Jython's.
        ("jython2.7/basic-dict-1001.p1.cpickle.pickle", JYTHON),
        ("jython2.7/basic-json-document.p1.cpickle.pickle", JYTHON),
        // A memo slot taken for an object before the slots its callable and
        // arguments take, which is IronPython's `cPickle` and no other.
        ("ironpython2.7/basic-int-keys.p1.cpickle.pickle", IRON),
        // A protocol 0 float line in Java's spelling, which GraalPy's
        // `_pickle` writes and its `pickle.py` does not.
        ("graalpy3.11/basic-json-document.p0.pickle", GRAAL),
        // And one file from each of the four that says nothing of its own.
        // A Jython file numbering from one is still one of three `cPickle`s;
        // every other file here could have come from any pickler at all.
        // IronPython 3.4 has no sharper line: the files that would have shown
        // a batch edge are byte for byte CPython 3.4's and are kept under its
        // folder, where the row already says `pickle.py`, which is true of
        // IronPython's `_pickle` as well.
        ("jython2.7/basic-records.p1.cpickle.pickle", CPICKLE),
        ("ironpython2.7/basic-records.p1.cpickle.pickle", EITHER),
        ("ironpython3.4/basic-int-keys.p4.pickle", EITHER),
        ("graalpy3.11/stdlib-datetime-aware.p4.pickle", EITHER),
        // Files the four wrote byte for byte as CPython did, which are kept
        // under CPython's folder and keep the broader statement: naming the
        // interpreter would be naming what the bytes do not show.
        ("py2.7/basic-dict-1000.p0.cpickle.pickle", CPICKLE),
        ("py2.7/basic-dict-1000.p1.pickle", PY),
        ("py3.4/basic-list-1001.p4.pickle", C),
        ("py3.4/basic-records.p4.pickle", EITHER),
        // PyPy's `cPickle` is a Python copy of CPython's: it numbers the memo
        // the same way and ends a dictionary the way `pickle.py` does, so the
        // row names the module and not which of the two wrote it.
        ("pypy2.7/basic-dict-1000.p2.cpickle.pickle", CPICKLE),
        ("pypy2.7/basic-records.p2.cpickle.pickle", CPICKLE),
        // And PyPy 2.7's pure pickler, which numbers from nought like every
        // other one and ends its containers the `pickle.py` way.
        ("pypy2.7/basic-records.p2.pickle", EITHER),
        ("pypy2.7/basic-long-dict.p2.pickle", EITHER),
    ];
    for (file, said) in want {
        let Ok(bytes) = std::fs::read(root.join(file)) else { panic!("{file} is not in the collection") };
        let found = formats::pickle::familiar::recognise(&bytes).unwrap_or_else(|| panic!("{file} matched no form"));
        assert_eq!(found.pickler.name(), *said, "{file}");
    }
    // PyPy 3.10 wrote its own copy of the first of those and the collection
    // keeps one, so the claim about PyPy is a claim about a file in another
    // folder. Its versions.json is what says PyPy wrote it.
    let listed = std::fs::read_to_string(root.join("pypy3.10/versions.json")).unwrap_or_default();
    assert!(
        listed.contains("\"basic-list-1001.p4.pickle\""),
        "pypy3.10 no longer claims basic-list-1001.p4.pickle, so py3.4's .pypickle copy is not PyPy's any more"
    );
}

/// A matched sample stops being that file when any instruction byte changes.
///
/// A form fixes its instructions, so a byte of one that could be changed
/// without the reading changing is a byte the form is not really matching. The
/// check is the weaker of the two things that could be asserted, and the
/// truthful one: the file either stops matching or is read as something else.
/// Flipping the bit that tells NEWFALSE from NEWTRUE really does leave a
/// matching file, holding an array in the other storage order.
///
/// Bytes inside a captured value are left alone on purpose: those are data, and
/// a payload that changed the reading would mean random bytes were being read
/// as instructions.
#[test]
fn a_matched_sample_stops_matching_when_its_instructions_change() {
    let Some(dir) = folder() else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    let read = |bytes: &[u8]| {
        formats::pickle::familiar::recognise(bytes).map(|m| (m.form, format!("{:?}", m.value)))
    };
    let mut checked = 0;
    for path in pickles(&dir) {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let bytes = std::fs::read(&path).unwrap();
        let Some(found) = formats::pickle::familiar::recognise(&bytes) else { continue };
        let before = (found.form, format!("{:?}", found.value));
        let starts: Vec<usize> = found.ops.iter().map(|op| op.at).collect();

        for at in &starts {
            for bit in [0, 2] {
                let mut changed = bytes.clone();
                changed[*at] ^= 1 << bit;
                assert_ne!(read(&changed).as_ref(), Some(&before), "{name}: bit {bit} at {at:#x} changed nothing");
            }
            // Truncation at every instruction boundary. A form reaches the
            // STOP and the end of the file or it has not matched.
            assert!(read(&bytes[..*at]).is_none(), "{name}: matched the first {at:#x} bytes");
        }
        let mut longer = bytes.clone();
        longer.push(b'N');
        assert!(read(&longer).is_none(), "{name}: matched with a value after the STOP");
        checked += 1;
        eprintln!("{name}: {} instructions, none of them spare", starts.len());
    }
    assert!(checked >= 20, "only {checked} samples matched a form");
}

/// One row of the familiar-form template: how deep it sits, what it is called,
/// what type it is, where it starts, how long it is and what it says.
#[derive(Debug)]
struct Row {
    path: Vec<usize>,
    depth: usize,
    name: String,
    ty: String,
    at: u64,
    len: u64,
    value: Value,
    /// Which address space the row's bytes are in. 0 is the file; a run the
    /// file spelled rather than wrote opens a space of its own, and what is
    /// inside it counts from that space's own start.
    space: u32,
}

/// Every row the template shows for this file, in file order.
fn familiar_rows(bytes: Vec<u8>) -> Vec<Row> {
    let doc = Document::new(MemSource(bytes));
    let mut ev = Evaluator::new(formats::builtin("picklefpf").unwrap());
    let mut out = Vec::new();
    walk_rows(&doc, &mut ev, &[], 0, &mut out);
    out
}

fn walk_rows(doc: &Document<MemSource>, ev: &mut Evaluator, path: &[usize], depth: usize, out: &mut Vec<Row>) {
    let node = ev.node(doc, path).unwrap_or_else(|e| panic!("{path:?}: {e:?}"));
    out.push(Row {
        path: path.to_vec(),
        depth,
        name: node.name.clone(),
        ty: node.type_name.clone(),
        at: node.offset_bits / 8,
        len: node.size_bits / 8,
        value: node.value.clone(),
        space: node.space,
    });
    // A run of numbers is counted rather than walked: it is a value, not a
    // part of the file's shape.
    if node.type_name.ends_with("[]") {
        return;
    }
    for i in 0..node.child_count as usize {
        let mut next = path.to_vec();
        next.push(i);
        walk_rows(doc, ev, &next, depth + 1, out);
    }
}

fn row<'a>(rows: &'a [Row], name: &str) -> &'a Row {
    rows.iter().find(|r| r.name == name).unwrap_or_else(|| panic!("no {name} row"))
}

/// A matched file's decoded values are reachable under the other template, an
/// unmatched one has nothing there, and a matched one has no byte left over.
///
/// The last of those is the point of a form. A form fixes its instructions, so
/// a byte no field covers is a byte the grammar matched and the template could
/// not account for, which is the thing this reading is supposed to rule out.
#[test]
fn the_familiar_template_reads_a_matched_sample_and_refuses_the_rest() {
    let Some(dir) = folder() else { return };
    let mut checked = 0;
    for path in pickles(&dir) {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let bytes = std::fs::read(&path).unwrap();
        let Some(form) = formats::pickle::familiar::recognise(&bytes).map(|m| m.form.to_string()) else {
            let doc = Document::new(MemSource(bytes));
            let mut ev = Evaluator::new(formats::builtin("picklefpf").unwrap());
            assert!(ev.node(&doc, &[]).is_err(), "{name}: read as a familiar form");
            continue;
        };
        let rows = familiar_rows(bytes.clone());
        assert_eq!(rows[0].len, bytes.len() as u64, "{name}: the root is not the file");
        assert_eq!(row(&rows, "message").value, Value::Str(formats::pickle::familiar::MESSAGE.to_string()), "{name}");
        assert_eq!(row(&rows, "form").value, Value::Str(form), "{name}");
        // The protocol the file declared, which is the byte after PROTO.
        // Protocols 0 and 1 have no opener to declare one, so the row is the
        // protocol the form read the file under, said rather than read.
        let protocol = &row(&rows, "protocol").value;
        match bytes.first() {
            Some(0x80) => assert_eq!(*protocol, Value::UInt(u128::from(bytes[1])), "{name}"),
            _ => assert!(matches!(protocol, Value::Str(said) if said == "0" || said == "1"), "{name}: {protocol:?}"),
        }
        covers(&rows, &name);
        checked += 1;
    }
    assert!(checked >= 20, "only {checked} samples matched a form");
}

/// The same reading, over the matrix's protocol 2 and 3 files.
///
/// The sibling `pickle/` folder has two of those and the matrix has hundreds,
/// including every shape the older grammar reads that the newer one does not:
/// a byte string written as a call to `_codecs`, a set built from a list, a
/// Python 2 `str`, an `INT` text line, and an array whose numbers reached the
/// file as latin-1 text. A handful of each is walked here rather than all of
/// them, since walking a file of a hundred thousand rows says nothing the
/// first one did not.
#[test]
fn the_older_protocols_read_as_a_tree_with_no_bytes_left_over() {
    let Some(root) = qubero_samples::dir("pickle-matrix") else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    let old = "py3.6-numpy1.19-pandas1.1-sklearn0.24";
    let want: &[&str] = &[
        // Python 3 at both protocols: text, a byte string written as a call,
        // records, one list under several names and a list holding itself.
        "py3.4/basic-records.p3.pickle",
        "py3.4/basic-records.p2.pickle",
        "py3.4/basic-nested.p3.pickle",
        "py3.4/basic-nested.p2.pickle",
        "py3.4/basic-shared-list.p2.pickle",
        "py3.4/basic-self-reference.p3.pickle",
        "py3.4/basic-int-keys.p2.pickle",
        // A set of 1,001, which below protocol 4 is a call over a list.
        "py3.4/basic-set-1001.p3.pickle",
        "py3.4/basic-set-1001.p2.pickle",
        // Python 2: `str` rather than text, `long`, and the `INT` text line
        // an `int` too wide for BININT went out as.
        "py2.7/basic-nested.p2.pickle",
        "py2.7/basic-nested.p2.cpickle.pickle",
        "py2.7/basic-int-keys.p2.cpickle.pickle",
        "pypy2.7/basic-records.p2.cpickle.pickle",
        // Arrays and scalars at both protocols, and a library object.
        &format!("{old}/numpy-1d-int64.p3.pickle"),
        &format!("{old}/numpy-1d-int64.p2.pickle"),
        &format!("{old}/numpy-2d-float32.p2.pickle"),
        &format!("{old}/numpy-scalar-float64.p2.pickle"),
        &format!("{old}/numpy-dict-of-arrays.p2.pickle"),
        &format!("{old}/sklearn-standard-scaler.p2.pickle"),
        &format!("{old}/series-float.p3.pickle"),
        &format!("{old}/dataframe-numeric.p2.pickle"),
    ];
    for file in want {
        let Ok(bytes) = std::fs::read(root.join(file)) else { panic!("{file} is not in the collection") };
        let form = formats::pickle::familiar::recognise(&bytes)
            .unwrap_or_else(|| panic!("{file} matched no form"))
            .form
            .to_string();
        let rows = familiar_rows(bytes.clone());
        assert_eq!(rows[0].len, bytes.len() as u64, "{file}: the root is not the file");
        assert_eq!(row(&rows, "form").value, Value::Str(form), "{file}");
        assert_eq!(row(&rows, "protocol").value, Value::UInt(u128::from(bytes[1])), "{file}");
        covers(&rows, file);
    }
}

/// Every node's children tile it: they start where it starts, they follow each
/// other, and the last of them ends where it ends. Rows worked out from the
/// match, rather than read from the file, are not part of the tiling. A
/// field the file did write is part of it even when it reads no bytes, such as
/// the numbers of an array whose shape is 0.
fn covers(rows: &[Row], what: &str) {
    for (i, row) in rows.iter().enumerate() {
        // A child in another space is not part of this row's tiling: what a
        // decoded run holds starts at byte 0 of the space it opened, which is
        // no address in the file at all.
        let kids: Vec<&Row> = rows[i + 1..]
            .iter()
            .take_while(|r| r.depth > row.depth)
            .filter(|r| r.depth == row.depth + 1 && r.space == row.space)
            .collect();
        if kids.is_empty() {
            continue;
        }
        let mut want = row.at;
        for kid in kids {
            if kid.ty.starts_with("computed") {
                assert_eq!(kid.at, row.at, "{what}: {} is worked out from the match, so it belongs where {} starts", kid.name, row.name);
                continue;
            }
            assert_eq!(kid.at, want, "{what}: {} in {} leaves {want:#x} over", kid.name, row.name);
            want = kid.at + kid.len;
        }
        assert_eq!(want, row.at + row.len, "{what}: {} has bytes over at {want:#x}", row.name);
    }
}

/// The numbers of the one matched array, and the call that rebuilt it.
#[test]
fn a_matched_array_reads_as_its_numbers_under_the_familiar_template() {
    let Some(dir) = folder() else { return };
    let Ok(bytes) = std::fs::read(dir.join("proto4-numpy-array.pickle")) else { return };
    let rows = familiar_rows(bytes);
    // One entry, called `weights`, holding an array of 24 floats 0 to 23.
    assert_eq!(row(&rows, "weights").ty, "entry");
    assert_eq!(row(&rows, "value").ty, "array");
    assert_eq!(row(&rows, "dtype").value, Value::Str("<f4".into()));
    assert_eq!(row(&rows, "shape").value, Value::Str("4 x 6".into()));
    assert_eq!(row(&rows, "order").value, Value::Str("C".into()));
    // The call, with the names the form matched inside it.
    assert_eq!(row(&rows, "array reconstruct call").ty, "call");
    assert_eq!(row(&rows, "module").value, Value::Str("numpy._core.multiarray".into()));
    assert_eq!(row(&rows, "callable").value, Value::Str("_reconstruct".into()));
    assert_eq!(row(&rows, "class module").value, Value::Str("numpy".into()));
    assert_eq!(row(&rows, "class").value, Value::Str("ndarray".into()));
    let numbers = row(&rows, "numbers");
    assert_eq!((numbers.ty.as_str(), numbers.len), ("f32 le[]", 96));

    // And the numbers themselves, read through the template.
    let doc = Document::new(MemSource(std::fs::read(dir.join("proto4-numpy-array.pickle")).unwrap()));
    let mut ev = Evaluator::new(formats::builtin("picklefpf").unwrap());
    let read: Vec<Value> = (0..24)
        .map(|i| {
            let mut at = numbers.path.clone();
            at.push(i);
            ev.node(&doc, &at).unwrap().value
        })
        .collect();
    assert_eq!(read, (0..24).map(|n| Value::Float(n as f64)).collect::<Vec<_>>());
}

/// A pickled frame opens as the table it holds, not as the program that
/// rebuilds it.
///
/// This is the whole point of reading a frame. The values are in blocks, each
/// written the other way up from the frame; the column names are in an index
/// beside them; a `RangeIndex` is not written down at all; and a categorical
/// column is codes into a third array. A reader who is shown any of that
/// instead of the data has been shown the pickle and not the frame.
#[test]
fn a_pickled_frame_opens_as_the_table_it_holds() {
    let Some(root) = qubero_samples::dir("pickle-matrix") else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    // What each object in the matrix is, from `tools/make_pickle_matrix.py`.
    let cases: &[(&str, &[&str], &[&str], &[&[&str]])] = &[
        (
            "dataframe-numeric",
            &["index", "id", "score"],
            &["int64", "int64", "float64"],
            &[&["0", "0", "1.5"], &["1", "1", "2.5"], &["2", "2", "3.5"], &["3", "3", "4.5"]],
        ),
        (
            "dataframe-mixed",
            &["index", "id", "score", "name"],
            &["int64", "int64", "float64", "str"],
            &[
                &["0", "0", "1.5", "a"],
                &["1", "1", "2.5", "b"],
                &["2", "2", "3.5", "c"],
                &["3", "3", "4.5", "d"],
            ],
        ),
        // A series is the same table with one value column, headed by the
        // name the series was given.
        ("series-float", &["index", "score"], &["int64", "float64"], &[&["0", "1.5"], &["1", "2.5"], &["2", "3.5"]]),
        // A date is written as a count of the unit its dtype names, and the
        // cell shows the date rather than the count.
        (
            "dataframe-datetime-index",
            &["index", "v"],
            &["datetime64", "float64"],
            &[&["2020-01-01", "1"], &["2020-01-02", "2"], &["2020-01-03", "3"]],
        ),
        // A categorical cell shows the category its code names.
        (
            "series-categorical",
            &["index", "value"],
            &["int64", "category"],
            &[&["0", "lo"], &["1", "hi"], &["2", "lo"], &["3", "mid"]],
        ),
    ];
    let mut checked = 0;
    for dir in std::fs::read_dir(&root).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.is_dir()) {
        for path in pickles(&dir) {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            // Every protocol a form reads. A protocol 2 frame keeps its
            // numbers as the latin-1 text they spell, so the same table
            // arriving cell for cell is the whole claim being made here.
            if ![".p0.", ".p1.", ".p2.", ".p3.", ".p4.", ".p5."].iter().any(|p| name.contains(p)) {
                continue;
            }
            let Some((_, columns, units, want)) = cases.iter().find(|(stem, ..)| name.starts_with(&format!("{stem}."))) else {
                continue;
            };
            let bytes = std::fs::read(&path).unwrap();
            if formats::pickle::familiar::recognise(&bytes).is_none() {
                continue;
            }
            let doc = Document::new(MemSource(bytes));
            let mut ev = Evaluator::new(formats::builtin("picklefpf").unwrap());
            let where_ = format!("{}/{name}", dir.file_name().unwrap().to_string_lossy());
            let shape = ev.table_shape(&doc, &[1]).unwrap().unwrap_or_else(|| panic!("{where_}: no table"));
            assert_eq!(shape.names, *columns, "{where_}");
            // The unit a date counts in is the writer's: pandas 3.0 counts in
            // microseconds where every release before it counted nanoseconds,
            // and the column's type says which.
            let said: Vec<&str> = shape.units.iter().map(|u| u.split('[').next().unwrap_or(u)).collect();
            assert_eq!(said, *units, "{where_}");
            assert_eq!(shape.row_word.as_deref(), Some("row"), "{where_}");
            let read = ev.pickle_cells(&doc, &[1], 0, want.len() as u64 + 1).unwrap();
            let said: Vec<Vec<String>> = read.iter().map(|row| row.iter().map(cell_text).collect()).collect();
            let want: Vec<Vec<String>> =
                want.iter().map(|row| row.iter().map(|c| (*c).to_string()).collect()).collect();
            assert_eq!(said, want, "{where_}");
            checked += 1;
        }
    }
    assert!(checked >= 36, "only {checked} frames read as tables");
}

/// A frame whose blocks are placed by a name for a slice opens as its table.
///
/// `assign` makes a frame that shares the first one's placements, and pickle
/// writes a shared object once: the second frame's blocks say where they sit
/// with a `BINGET`. The slice is in the first frame, which is nowhere under the
/// second, so the table is only right if the name was followed across them.
#[test]
fn a_frame_placed_by_a_named_slice_opens_as_the_table_it_holds() {
    let Some(dir) = folder() else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    let tables: [&[&[&str]]; 2] = [
        &[&["0", "1", "0.5", "a"], &["1", "2", "1.5", "b"], &["2", "3", "2.5", "c"]],
        &[&["0", "2", "0.5", "a"], &["1", "4", "1.5", "b"], &["2", "6", "2.5", "c"]],
    ];
    for name in ["mixed-frames-sharing-placements-p4.pickle", "mixed-frames-sharing-placements-p2.pickle"] {
        let bytes = std::fs::read(dir.join(name)).unwrap();
        let doc = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(formats::builtin("picklefpf").unwrap());
        for (i, want) in tables.iter().enumerate() {
            // The list's own rows come first, and then its items.
            let path = [1, 3 + i];
            let shape = ev.table_shape(&doc, &path).unwrap().unwrap_or_else(|| panic!("{name}: frame {i} is no table"));
            assert_eq!(shape.names, ["index", "x", "y", "label"], "{name}: frame {i}");
            assert_eq!(shape.units, ["int64", "int64", "float64", "str"], "{name}: frame {i}");
            let read = ev.pickle_cells(&doc, &path, 0, want.len() as u64 + 1).unwrap();
            let said: Vec<Vec<String>> = read.iter().map(|row| row.iter().map(cell_text).collect()).collect();
            let want: Vec<Vec<String>> = want.iter().map(|row| row.iter().map(|c| (*c).to_string()).collect()).collect();
            assert_eq!(said, want, "{name}: frame {i}");
        }
    }
}

/// A column of objects opens as its values, whatever was pickled into it.
///
/// A pandas column of objects holds whatever Python was holding: dates, lists,
/// exact numbers. Each value is a whole object of the pickle rather than a run
/// of bytes, so the claim is cell for cell: the date as a date, the list as
/// what kind of thing it is and how much of it, the exact number as its
/// digits. The header says `object` rather than `str`, which is pandas' own
/// word for the dtype and is what a reader compares against.
#[test]
fn a_column_of_objects_opens_as_the_values_it_holds() {
    let Some(dir) = folder() else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    let cases: &[(&str, &[&str], &[&str], &[&[&str]])] = &[
        (
            "mixed-frame-of-dates",
            &["index", "day", "reading"],
            &["int64", "object", "float64"],
            &[&["0", "2026-09-17", "1.5"], &["1", "2026-09-18", "2.5"], &["2", "2026-09-19", "3.5"]],
        ),
        (
            "mixed-frame-of-lists",
            &["index", "items", "n"],
            &["int64", "object", "int64"],
            &[&["0", "list of 3", "3"], &["1", "list of 1", "1"], &["2", "list of 2", "2"]],
        ),
        (
            "mixed-frame-of-decimals",
            &["index", "price"],
            &["int64", "object"],
            &[&["0", "1.25"], &["1", "0.10"], &["2", "99.99"]],
        ),
    ];
    for (stem, columns, units, want) in cases {
        for protocol in ["p4", "p2"] {
            let name = format!("{stem}-{protocol}.pickle");
            let bytes = std::fs::read(dir.join(&name)).unwrap();
            let doc = Document::new(MemSource(bytes));
            let mut ev = Evaluator::new(formats::builtin("picklefpf").unwrap());
            let shape = ev.table_shape(&doc, &[1]).unwrap().unwrap_or_else(|| panic!("{name}: no table"));
            assert_eq!(shape.names, *columns, "{name}");
            assert_eq!(shape.units, *units, "{name}");
            let read = ev.pickle_cells(&doc, &[1], 0, want.len() as u64 + 1).unwrap();
            let said: Vec<Vec<String>> = read.iter().map(|row| row.iter().map(cell_text).collect()).collect();
            let want: Vec<Vec<String>> = want.iter().map(|row| row.iter().map(|c| (*c).to_string()).collect()).collect();
            assert_eq!(said, want, "{name}");
        }
    }
}

/// An array of objects holding a class no form names stays a non-match.
///
/// The values in such an array are read by the same grammar as the rest of the
/// file, and that grammar names a class only from the modules the form listed.
/// So widening what an array of objects may hold widens it by exactly the
/// productions the file's form already allows, and not by one more.
#[test]
fn an_array_of_objects_holding_an_unnamed_class_is_a_non_match() {
    let Some(dir) = folder() else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    for name in ["unfamiliar-frame-of-instances-p4.pickle", "unfamiliar-frame-of-instances-p2.pickle"] {
        let bytes = std::fs::read(dir.join(name)).unwrap();
        let found = formats::pickle::familiar::recognise(&bytes);
        assert!(found.is_none(), "{name}: matched {:?}", found.map(|f| f.form));
    }
}

/// An array reads as the same numbers at every protocol a form takes.
///
/// Protocol 2 has no opcode for a byte string, so an array's numbers go out as
/// the latin-1 text they spell and are nowhere in the file as numbers. They
/// are decoded once when the form matches, and this is the claim that makes:
/// the same values, in the same order, however the file spelled them.
#[test]
fn an_array_reads_as_the_same_numbers_at_every_protocol() {
    let Some(root) = qubero_samples::dir("pickle-matrix") else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    // What `tools/make_pickle_matrix.py` pickled, in storage order: a Fortran
    // array's numbers go down its first axis, which is why its run is not the
    // count in order.
    let whole: Vec<String> = (0..24).map(|n| n.to_string()).collect();
    let cases: &[(&str, Vec<String>)] = &[
        ("numpy-1d-int64", (0..10).map(|n| n.to_string()).collect()),
        ("numpy-2d-float32", whole.clone()),
        ("numpy-big-endian", (0..6).map(|n| n.to_string()).collect()),
        ("numpy-bool", vec!["1".into(), "0".into(), "1".into()]),
        (
            "numpy-2d-float64-fortran",
            [0, 4, 8, 1, 5, 9, 2, 6, 10, 3, 7, 11].iter().map(|n| n.to_string()).collect(),
        ),
    ];
    let mut checked = 0;
    for dir in std::fs::read_dir(&root).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.is_dir()) {
        for path in pickles(&dir) {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            let Some((_, want)) = cases.iter().find(|(stem, _)| name.starts_with(&format!("{stem}."))) else { continue };
            let bytes = std::fs::read(&path).unwrap();
            if formats::pickle::familiar::recognise(&bytes).is_none() {
                continue;
            }
            let where_ = format!("{}/{name}", dir.file_name().unwrap().to_string_lossy());
            assert_eq!(&array_numbers(&bytes, &where_), want, "{where_}");
            checked += 1;
        }
    }
    assert!(checked >= 34, "only {checked} arrays read as their numbers");
}

/// An array's numbers open as a space of their own, and every value has an
/// address in it.
///
/// Protocol 2 writes an array's numbers as the latin-1 text they spell, and
/// protocol 0 escapes that text again to fit it on a line, so at neither
/// protocol are the numbers in the file as numbers. The run opens the way
/// every other packed run in a file opens: as a space, with the values as
/// ordinary typed fields at ordinary offsets in it. What this asserts is that
/// those offsets are real, which is what "Show byte addresses" over the table
/// shows a reader, and that the space still says which bytes of the file it
/// came out of.
#[test]
fn a_spelled_array_opens_its_numbers_as_a_space() {
    let Some(root) = qubero_samples::dir("pickle-matrix") else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    let dir = root.join("py3.6-numpy1.19-pandas1.1-sklearn0.24");
    // 24 floats 0 to 23, four rows of six, at the two protocols that spell
    // them rather than writing them.
    for (name, codec) in
        [("numpy-2d-float32.p2.pickle", "latin-1 text"), ("numpy-2d-float32.p0.pickle", "latin-1 text, escaped")]
    {
        let Ok(bytes) = std::fs::read(dir.join(name)) else { panic!("{name} is not in the collection") };
        let rows = familiar_rows(bytes.clone());
        // The run the file holds, which is text and is in the file where the
        // pickler wrote it.
        let run = rows
            .iter()
            .find(|r| r.name == "numbers" && r.ty == codec)
            .unwrap_or_else(|| panic!("{name}: no run of numbers"));
        assert_eq!(run.space, 0, "{name}: the run is not in the file");

        let doc = Document::new(MemSource(bytes.clone()));
        let mut ev = Evaluator::new(formats::builtin("picklefpf").unwrap());
        // The numbers themselves, one step under it and in a space of their
        // own, starting at the front of it.
        let numbers = [run.path.as_slice(), &[0]].concat();
        let node = ev.node(&doc, &numbers).unwrap();
        assert_eq!(node.type_name, "f32 le[]", "{name}");
        assert_ne!(node.space, 0, "{name}: the numbers are not in a space of their own");
        assert_eq!(node.offset_bits, 0, "{name}");
        assert_eq!(node.child_count, 24, "{name}");
        // A value's address is where that value sits in the decoded bytes.
        for i in [0usize, 7, 23] {
            let cell = ev.node(&doc, &[numbers.as_slice(), &[i]].concat()).unwrap();
            assert_eq!(cell.offset_bits, i as u64 * 32, "{name}: value {i}");
            assert_eq!(cell.size_bits, 32, "{name}: value {i}");
            assert_eq!(cell.space, node.space, "{name}: value {i}");
            assert_eq!(cell.value, Value::Float(i as f64), "{name}: value {i}");
        }
        // The table hangs on the numbers rather than on the run, and its cells
        // are those fields rather than anything the core works out.
        let shape = ev.table_shape(&doc, &numbers).unwrap().unwrap_or_else(|| panic!("{name}: no table"));
        assert_eq!(shape.columns, Some(6), "{name}");
        assert_eq!(shape.row_word.as_deref(), Some("row"), "{name}");
        assert_eq!(shape.cells, None, "{name}: the cells are fields, not worked out");
        assert!(ev.table_shape(&doc, &run.path).unwrap().is_none(), "{name}: the run itself is no table");

        // And the space says which bytes of the file it came out of.
        let id = ev
            .open_space(&doc, 0, &run.path)
            .unwrap()
            .unwrap_or_else(|| panic!("{name}: the run does not open"));
        let space = ev.space(id).unwrap();
        assert_eq!(space.codec.as_str(), codec, "{name}");
        assert_eq!(space.len_bytes(), 96, "{name}");
        space.trace().check_tiles().unwrap_or_else(|e| panic!("{name}: {e}"));
        let at = space.run().unwrap().run_offset_bits;
        assert_eq!(at, run.at * 8, "{name}: the run is not where the file wrote it");
        let first = space.map_out(0).unwrap();
        assert!(first.in_bits.end <= run.len * 8, "{name}: the first byte came from outside the run");
        assert!(space.map_in(at).is_some(), "{name}: the run's first bit made nothing");
        assert!(space.map_in(at - 8).is_none(), "{name}: a bit in front of the run is not this space's");
    }
}

/// The numbers of the one array in a file, in storage order, read the way the
/// interface reads them: as a table where the core says the cells are its to
/// work out, and as the node's own values where they sit in the file.
fn array_numbers(bytes: &[u8], where_: &str) -> Vec<String> {
    let doc = Document::new(MemSource(bytes.to_vec()));
    let mut ev = Evaluator::new(formats::builtin("picklefpf").unwrap());
    let rows = familiar_rows(bytes.to_vec());
    // The run of numbers, which below protocol 3 is a step under the node the
    // match placed: that node is the text the numbers were spelled as, and its
    // one child is the numbers themselves. Both are called `numbers`, so the
    // type is what tells them apart.
    let row = rows
        .iter()
        .find(|r| r.name == "numbers" && r.ty.ends_with("[]"))
        .unwrap_or_else(|| panic!("{where_}: no numbers row"));
    let shape = ev.table_shape(&doc, &row.path).unwrap();
    if let Some(qubero_core::template::Cells::Computed { rows: count }) = shape.as_ref().and_then(|s| s.cells.clone()) {
        let read = ev.pickle_cells(&doc, &row.path, 0, count).unwrap();
        return read.iter().flatten().map(cell_text).collect();
    }
    let node = ev.node(&doc, &row.path).unwrap();
    (0..node.child_count as usize)
        .map(|i| {
            let mut at = row.path.clone();
            at.push(i);
            value_text(&Some(ev.node(&doc, &at).unwrap().value))
        })
        .collect()
}

/// A cell as the interface shows it, which for a value the frame has not got
/// is nothing at all.
fn cell_text(cell: &FrameCell) -> String {
    value_text(&cell.value)
}

/// The same for a value read off a node of the tree rather than out of a
/// table.
fn value_text(cell: &Option<Value>) -> String {
    match cell {
        None => String::new(),
        Some(Value::Int(n)) => n.to_string(),
        Some(Value::UInt(n)) => n.to_string(),
        Some(Value::Float(f)) => f.to_string(),
        Some(Value::Str(s)) => s.clone(),
        Some(other) => format!("{other:?}"),
    }
}

/// A matched frame, series or sparse matrix opens with what a reader came for,
/// before the structure that holds it.
#[test]
fn a_library_object_says_what_it_holds_before_how() {
    let Some(root) = qubero_samples::dir("pickle-matrix") else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    let want: &[(&str, &[(&str, &str)])] = &[
        (
            "dataframe-mixed",
            &[
                ("columns", "id, score, name"),
                ("rows", "4"),
                ("index", "RangeIndex 0 to 4"),
                ("dtypes", "id int64, score float64, name str"),
            ],
        ),
        (
            "series-categorical",
            &[("columns", "value"), ("rows", "4"), ("index", "RangeIndex 0 to 4"), ("dtypes", "value category")],
        ),
        ("scipy-csr-matrix", &[("shape", "4 x 4"), ("stored values", "4"), ("format", "csr")]),
    ];
    let mut checked = 0;
    for dir in std::fs::read_dir(&root).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.is_dir()) {
        for path in pickles(&dir) {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            if !name.contains(".p4.") && !name.contains(".p5.") {
                continue;
            }
            let Some((_, rows)) = want.iter().find(|(stem, _)| name.starts_with(&format!("{stem}."))) else { continue };
            let bytes = std::fs::read(&path).unwrap();
            if formats::pickle::familiar::recognise(&bytes).is_none() {
                continue;
            }
            let doc = Document::new(MemSource(bytes));
            let mut ev = Evaluator::new(formats::builtin("picklefpf").unwrap());
            let where_ = format!("{}/{name}", dir.file_name().unwrap().to_string_lossy());
            let said: Vec<(String, String)> = (0..rows.len())
                .map(|i| {
                    let n = ev.node(&doc, &[1, i]).unwrap();
                    let value = match &n.value {
                        Value::Str(s) => s.clone(),
                        other => format!("{other:?}"),
                    };
                    (n.name.clone(), value)
                })
                .collect();
            let want: Vec<(String, String)> =
                rows.iter().map(|(n, v)| ((*n).to_string(), (*v).to_string())).collect();
            assert_eq!(said, want, "{where_}");
            checked += 1;
        }
    }
    assert!(checked >= 12, "only {checked} objects said what they hold");
}

/// A file holding two families reads as both of them, and everything under it
/// keeps working where it sits rather than only at the root.
///
/// A frame at the root of a file is the easy case, and it is not the case
/// anyone has: a frame arrives inside the dictionary that carries the note and
/// the date beside it. So the claim is the placing as much as the match: the
/// form, the row that says which extensions the file used, and then the frame
/// deep in the tree offering the same table and the same summary rows it
/// offers at the root.
#[test]
fn a_mixed_file_reads_as_every_family_it_holds() {
    let Some(dir) = folder() else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    // Each file, the form, and what the `form extensions` row says.
    let want: &[(&str, &str, &str)] = &[
        ("mixed-date-and-array-p4.pickle", "mixed-values-p4-p5-v2", "stdlib, numpy"),
        ("mixed-date-and-array-p2.pickle", "mixed-values-p2-p3-v2", "stdlib, numpy"),
        ("mixed-ordereddict-of-arrays-p4.pickle", "mixed-values-p4-p5-v2", "stdlib, numpy"),
        ("mixed-decimal-and-array-p4.pickle", "mixed-values-p4-p5-v2", "stdlib, numpy"),
        // A fitted model with the day it was fitted: the estimator brings its
        // arrays with it, so three names rather than two.
        ("mixed-model-and-metadata-p4.pickle", "mixed-values-p4-p5-v2", "stdlib, numpy, sklearn"),
        ("mixed-model-and-metadata-p2.pickle", "mixed-values-p2-p3-v2", "stdlib, numpy, sklearn"),
        // pandas places a block by writing a `slice`, which is the builtins
        // production, so a frame brings that name with it too.
        ("mixed-frame-and-notes-p4.pickle", "mixed-values-p4-p5-v2", "builtins, stdlib, numpy, pandas"),
        ("mixed-frame-and-notes-p2.pickle", "mixed-values-p2-p3-v2", "builtins, stdlib, numpy, pandas"),
        // One family, which keeps the name it already had. The row says so
        // too, which is why it is worth having on every file and not only on
        // a mixed one.
        ("mixed-list-of-frames-p4.pickle", "pandas-frame-p4-p5-v1", "builtins, numpy, pandas"),
        // A column of objects holding dates, which makes the file use the
        // standard library's own production: the values in such a column are
        // read by the same grammar as the rest of the file, so a date in one
        // counts the way a date anywhere else does.
        ("mixed-frame-of-dates-p4.pickle", "mixed-values-p4-p5-v2", "builtins, stdlib, numpy, pandas"),
        ("mixed-frame-of-dates-p2.pickle", "mixed-values-p2-p3-v2", "builtins, stdlib, numpy, pandas"),
        ("mixed-frame-of-decimals-p4.pickle", "mixed-values-p4-p5-v2", "builtins, stdlib, numpy, pandas"),
        // A column of lists is containers of plain values, which every form
        // already reads, so the frame keeps the pandas form.
        ("mixed-frame-of-lists-p4.pickle", "pandas-frame-p4-p5-v1", "builtins, numpy, pandas"),
        ("mixed-frame-of-lists-p2.pickle", "pandas-frame-p2-p3-v1", "builtins, numpy, pandas"),
        // An array of objects holding tuples, beside a date. The array alone
        // is numpy and nothing else, and the mixed form wants two.
        ("mixed-array-of-tuples-p4.pickle", "mixed-values-p4-p5-v2", "stdlib, numpy"),
        ("mixed-array-of-tuples-p2.pickle", "mixed-values-p2-p3-v2", "stdlib, numpy"),
    ];
    for (name, form, extensions) in want {
        let bytes = std::fs::read(dir.join(name)).unwrap();
        let found = formats::pickle::familiar::recognise(&bytes).unwrap_or_else(|| panic!("{name} matched no form"));
        assert_eq!(found.form, *form, "{name}");
        assert_eq!(found.extensions(), *extensions, "{name}");
        let rows = familiar_rows(bytes);
        assert_eq!(row(&rows, "form").value, Value::Str((*form).into()), "{name}");
        assert_eq!(row(&rows, "form extensions").value, Value::Str((*extensions).into()), "{name}");
        // The row every file has, whatever else it holds.
        assert!(rows.iter().any(|r| r.name == "pickler"), "{name}: no pickler row");
    }

    // The frame inside the dictionary, which is where a frame really lives.
    // It opens with the four rows a frame opens with and it offers the table
    // it holds, both of them on the frame's own node rather than the file's.
    for (name, made) in [("mixed-frame-and-notes-p4.pickle", "2026-09-19T10:08:00"), ("mixed-frame-and-notes-p2.pickle", "2026-09-19T10:08:00")] {
        let bytes = std::fs::read(dir.join(name)).unwrap();
        let doc = Document::new(MemSource(bytes.clone()));
        let mut ev = Evaluator::new(formats::builtin("picklefpf").unwrap());
        let mut rows = Vec::new();
        walk_rows(&doc, &mut ev, &[], 0, &mut rows);
        // The three entries of the dictionary, read as what they are.
        assert_eq!(row(&rows, "note").value, Value::Str("three rows, two numbers and a label".into()), "{name}");
        assert_eq!(row(&rows, "made").value, Value::Str(made.into()), "{name}");
        // The entry the frame arrived under, and the frame itself, which is
        // the `value` half of that entry and not the entry.
        // The entry, not the FRAME opcode of the envelope, which goes by the
        // same word.
        let entry = rows.iter().find(|r| r.name == "frame" && r.ty == "entry").unwrap_or_else(|| panic!("{name}: no frame entry"));
        let frame = rows
            .iter()
            .find(|r| r.name == "value" && r.path.len() == entry.path.len() + 1 && r.path.starts_with(&entry.path))
            .unwrap_or_else(|| panic!("{name}: the frame entry holds no value"));
        assert!(frame.path.len() > 2, "{name}: the frame is at the root, not inside the dictionary");
        // What a reader came for, before the structure that holds it.
        let summary: Vec<(String, String)> = (0..4)
            .map(|i| {
                let mut at = frame.path.clone();
                at.push(i);
                let n = ev.node(&doc, &at).unwrap();
                (n.name.clone(), value_text(&Some(n.value)))
            })
            .collect();
        let expected = [
            ("columns", "x, y, label"),
            ("rows", "3"),
            ("index", "RangeIndex 0 to 3"),
            ("dtypes", "x int64, y float64, label str"),
        ];
        assert_eq!(summary, expected.map(|(n, v)| (n.to_string(), v.to_string())).to_vec(), "{name}");
        // And the table, worked out from the blocks wherever the frame sits.
        let shape = ev.table_shape(&doc, &frame.path).unwrap().unwrap_or_else(|| panic!("{name}: the frame offers no table"));
        assert_eq!(shape.names, ["index", "x", "y", "label"], "{name}");
        let read = ev.pickle_cells(&doc, &frame.path, 0, 4).unwrap();
        let said: Vec<Vec<String>> = read.iter().map(|r| r.iter().map(cell_text).collect()).collect();
        assert_eq!(said, vec![vec!["0", "1", "0.5", "a"], vec!["1", "2", "1.5", "b"], vec!["2", "3", "2.5", "c"]], "{name}");
    }

    // The array beside the date, which is a run of numbers wherever it sits.
    // Protocol 2 writes those numbers as the latin-1 text they spell, so the
    // claim is the same six values from two different spellings.
    for name in ["mixed-date-and-array-p4.pickle", "mixed-date-and-array-p2.pickle"] {
        let bytes = std::fs::read(dir.join(name)).unwrap();
        let rows = familiar_rows(bytes.clone());
        assert_eq!(row(&rows, "when").value, Value::Str("2026-09-19T10:08:00".into()), "{name}");
        assert_eq!(array_numbers(&bytes, name), (0..6).map(|n| n.to_string()).collect::<Vec<_>>(), "{name}");
    }
}

/// A list of `OrderedDict`s is a table, the same way a list of dictionaries
/// is, and its cells read as the values they hold rather than as the bytes.
///
/// This is what a program's own records look like once the standard library is
/// in them: a date, an exact number and a counter under keys the file wrote
/// beside them. The claim is the same table at every protocol from every
/// pickler, since a date reaches protocol 2 as latin-1 text and protocol 0 as
/// an escaped line and is the same date either way.
#[test]
fn a_list_of_ordered_dicts_opens_as_the_table_it_holds() {
    let Some(root) = qubero_samples::dir("pickle-matrix") else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    let columns = ["id", "when", "price", "tags"];
    // `{"id": i, "when": datetime(2020, 1, 1 + i, 12, 0), "price":
    // Decimal("9.99") * i, "tags": Counter(["a"] * i)}` for i from 1 to 5,
    // which is `stdlib_objects` in `tools/make_pickle_matrix.py`.
    let prices = ["9.99", "19.98", "29.97", "39.96", "49.95"];
    let want: Vec<Vec<String>> = (1..=5)
        .map(|i| {
            vec![
                i.to_string(),
                format!("2020-01-{:02}T12:00:00", i + 1),
                prices[i - 1].to_string(),
                "Counter of 1".to_string(),
            ]
        })
        .collect();
    let mut checked: Vec<String> = Vec::new();
    for dir in std::fs::read_dir(&root).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.is_dir()) {
        let env = dir.file_name().unwrap().to_string_lossy().into_owned();
        // Python 2 hands `OrderedDict` everything it is to hold as one list of
        // pairs, where every release since creates it empty and fills it with
        // the opcodes after the call. So a Python 2 record is a list of short
        // lists rather than a mapping, and it is not this table. Jython and
        // IronPython 2.7 are Python 2 as well, whatever they are written in.
        if env.contains("2.7") {
            continue;
        }
        for path in pickles(&dir) {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            if !name.starts_with("stdlib-records.") {
                continue;
            }
            let bytes = std::fs::read(&path).unwrap();
            let doc = Document::new(MemSource(bytes));
            let mut ev = Evaluator::new(formats::builtin("picklefpf").unwrap());
            let where_ = format!("{}/{name}", dir.file_name().unwrap().to_string_lossy());
            let shape = ev.table_shape(&doc, &[1]).unwrap().unwrap_or_else(|| panic!("{where_}: no table"));
            assert_eq!(shape.names, columns, "{where_}");
            assert_eq!(shape.row_word.as_deref(), Some("row"), "{where_}");
            // The rows are the nodes under the list and a cell is the entry
            // named by its column, so the reading is the tree's rather than a
            // run of values the view walks.
            let rows = familiar_rows(std::fs::read(&path).unwrap());
            let said: Vec<Vec<String>> = rows
                .iter()
                .filter(|r| r.ty == "OrderedDict")
                .map(|r| {
                    columns
                        .iter()
                        .map(|column| {
                            let cell = rows.iter().find(|c| c.name == *column && c.path.starts_with(&r.path)).unwrap();
                            value_text(&Some(cell.value.clone()))
                        })
                        .collect()
                })
                .collect();
            assert_eq!(said, want, "{where_}");
            checked.push(name);
        }
    }
    // Every protocol, since the collection keeps one copy of each distinct
    // byte string and the same records written twice are one file.
    for proto in 0..=5 {
        assert!(checked.iter().any(|name| name.contains(&format!(".p{proto}."))), "no record list at protocol {proto}: {checked:?}");
    }
}
