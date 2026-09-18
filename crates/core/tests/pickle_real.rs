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
use qubero_core::eval::{Evaluator, Value};
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
    let doc = Document::new(MemSource(bytes));
    let mut ev = Evaluator::new(formats::builtin("pickle").unwrap());

    let mut found = Vec::new();
    fields(&doc, &mut ev, &[], &mut found, 0);
    let named = |ty: &str, field: &str| -> Vec<i128> {
        found.iter().filter(|(t, f, _)| t == ty && f == field).map(|(_, _, v)| *v).collect()
    };
    assert_eq!(named("DateTime", "year"), vec![2026; 3], "three datetimes, all this year");
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
    assert_eq!(named("DateTime", "hour"), vec![10, 0, 2]);
    assert_eq!(named("DateTime", "minute"), vec![30, 0, 30]);
    assert_eq!(named("DateTime", "second"), vec![15, 0, 0]);
    // One of the three datetimes is the second two o'clock, and the bit that
    // says so is inside the month byte.
    let folds = named("DateTime", "fold");
    assert_eq!(folds.iter().filter(|f| **f == 1).count(), 1, "folds were {folds:?}");
    assert!(named("DateTime", "month").contains(&4), "the folded one is in April: {:?}", named("DateTime", "month"));
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
        ("awa2-pose-antelope.pickle", Some("basic-p4-p5-v4")),
        ("awa2-pose-elephant.pickle", Some("basic-p4-p5-v4")),
        // A dictionary whose big value is written between two frames.
        ("proto4-unframed-payload.pickle", Some("basic-p4-p5-v4")),
        ("proto4-numpy-array.pickle", Some("numpy-numeric-array-p4-p5-v4")),
        // Several arrays in one dictionary, the later ones naming numpy's
        // globals, dtype class, byte order or whole dtype out of the memo.
        ("proto4-numpy-byte-order.pickle", Some("numpy-numeric-array-p4-p5-v4")),
        ("proto4-numpy-dtypes.pickle", Some("numpy-numeric-array-p4-p5-v4")),
        ("proto4-numpy-shapes.pickle", Some("numpy-numeric-array-p4-p5-v4")),
        ("proto4-numpy-shared-dtype.pickle", Some("numpy-numeric-array-p4-p5-v4")),
        ("proto4-builtins.pickle", Some("builtins-values-p4-p5-v2")),
        // The rest, none of which any form accepts yet. Some are grammar the
        // forms have not reached (nonempty tuples, big integers, shared
        // container references, more than one batch); the library files need
        // forms of their own, built from reviewed complete structures.
        //
        // The `everything` files at every protocol, and `proto4-collections`,
        // are held back by one thing between them: each calls a class the
        // forms do not name. `datetime.datetime`, `decimal.Decimal`,
        // `fractions.Fraction` and `ValueError` are rebuilt by REDUCE, and
        // the namedtuple in `collections` by NEWOBJ of a class defined in the
        // file that wrote it. A form that took those would be accepting any
        // class at all, which is the one thing the contract rules out.
        ("handmade-python2-modules.pickle", None),
        ("handmade-python2-opcodes.pickle", None),
        ("handmade-wide-lengths.pickle", None),
        ("proto0-everything.pickle", None),
        ("proto0-persistent-id.pickle", None),
        ("proto1-everything.pickle", None),
        ("proto2-everything.pickle", None),
        ("proto2-extension-registry.pickle", None),
        ("proto2-memo-over-256.pickle", None),
        ("proto2-torch-state-dict.pickle", None),
        ("proto3-everything.pickle", None),
        ("proto3-numpy-1-module-names.pickle", None),
        ("proto4-collections.pickle", None),
        ("proto4-datetime.pickle", None),
        ("proto4-everything.pickle", None),
        ("proto4-newobj.pickle", None),
        ("proto4-numpy-object-array.pickle", None),
        ("proto4-persistent-id.pickle", None),
        ("proto4-scipy-coo-matrix.pickle", None),
        ("proto4-scipy-csc-matrix.pickle", None),
        ("proto4-scipy-csr-matrix.pickle", None),
        ("proto4-sklearn-pipeline.pickle", None),
        ("proto4-sklearn-random-forest.pickle", None),
        ("proto5-everything.pickle", None),
        ("proto5-out-of-band.pickle", None),
        ("proto5-pandas-dataframe.pickle", None),
        ("proto5-pandas-index-types.pickle", None),
        ("proto5-pandas-series.pickle", None),
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
    assert!(checked >= 9, "only {checked} samples matched a form");
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
        assert_eq!(row(&rows, "protocol").value, Value::UInt(4), "{name}");
        covers(&rows, &name);
        checked += 1;
    }
    assert!(checked >= 9, "only {checked} samples matched a form");
}

/// Every node's children tile it: they start where it starts, they follow each
/// other, and the last of them ends where it ends. Rows worked out from the
/// match, rather than read from the file, are not part of the tiling. A
/// field the file did write is part of it even when it reads no bytes, such as
/// the numbers of an array whose shape is 0.
fn covers(rows: &[Row], what: &str) {
    for (i, row) in rows.iter().enumerate() {
        let kids: Vec<&Row> = rows[i + 1..]
            .iter()
            .take_while(|r| r.depth > row.depth)
            .filter(|r| r.depth == row.depth + 1)
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
    assert_eq!(row(&rows, "ndarray reconstruct call").ty, "call");
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
