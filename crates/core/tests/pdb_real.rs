//! The program databases in the sample collection, read as what a debugger
//! reads them as.
//!
//! `pdb/` holds four files built for the collection and two published by
//! somebody else, and between them they answer both ways on the one question
//! this format turns on: whether a stream's blocks are a single run. A stream
//! that is reads where it lies and everything inside it is placed; a stream
//! that is not stays its block list. A fixture can put either case in front of
//! the template, but only real compiler output says which case a compiler
//! actually writes.
//!
//! Three of these checks are things no fixture can stand in for. The records
//! of a type stream tile the space its header gives them and number as many as
//! the two type numbers in that header are apart, which is a claim about a
//! real compiler's output rather than about our arithmetic. The executable
//! beside the symbols carries the same GUID and age in its debug directory,
//! which is the pairing that decides whether a debugger will use the pair at
//! all, and it can only be checked across two files. And a stream directory
//! spread over two blocks is something modern `link.exe` will not produce at
//! all, so the file that has one had to come from elsewhere.
//!
//! Like the rest of the collection this skips when the folder is not beside
//! the repository. Point `QUBERO_SAMPLES` at it to run it elsewhere.

use std::path::{Path, PathBuf};

use qubero_core::document::Document;
use qubero_core::eval::{Evaluator, NodeInfo, Value};
use qubero_core::formats;
use qubero_core::source::MemSource;

/// Field indices into the root of the `pdb` template.
const BLOCK_SIZE: usize = 1;
const DIRECTORY_BYTES: usize = 4;
const DIRECTORY: usize = 11;
/// Inside the directory: the stream count and the streams.
const STREAM_COUNT: usize = 0;
const STREAMS: usize = 2;
/// Inside one stream: its length, its blocks, and what is in them.
const SIZE: usize = 1;
const BLOCKS: usize = 2;
const CONTENTS: usize = 4;

/// Every file in the folder is the format its name says, and the two binaries
/// beside the symbols are read as what they are rather than as more symbols.
#[test]
fn the_collection_is_recognised_as_the_three_formats_that_share_the_extension() {
    let Some(dir) = folder() else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    let expected = [
        ("msvc-x64-shapes.pdb", "pdb"),
        ("msvc-x64-shapes-compiler.pdb", "pdb"),
        ("msvc-x64-260-modules.pdb", "pdb"),
        ("autoupdater-net-1.5.8-net40.pdb", "pdb"),
        ("dotnet-portable-shapes.pdb", "portablepdb"),
        ("awssdk-core-3.7.100.14-net45.pdb", "portablepdb"),
        ("msvc-x64-shapes.exe", "pe"),
        ("dotnet-portable-shapes.dll", "pe"),
    ];
    for (name, template) in expected {
        let bytes = read_sample(&dir, name);
        assert_eq!(
            formats::sniff(&bytes[..bytes.len().min(formats::SNIFF_WINDOW)], bytes.len() as u64),
            Some(template),
            "{name}"
        );
    }
}

/// A PDB whose streams are each written in one run reads all the way down: the
/// stream table, and then every stream's own fields at the blocks it lies in.
#[test]
fn a_pdb_written_in_one_pass_reads_every_stream_where_it_lies() {
    let Some(dir) = folder() else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    let (doc, mut ev) = open(&dir, "msvc-x64-shapes.pdb", "pdb");
    assert_eq!(ev.node(&doc, &[BLOCK_SIZE]).unwrap().value.as_int(), Some(4096));
    let streams = ev.node(&doc, &[DIRECTORY, 0, STREAMS]).unwrap().child_count as usize;
    assert_eq!(streams, 19);

    let mut read = 0;
    for i in 0..streams {
        let size = ev.node(&doc, &[DIRECTORY, 0, STREAMS, i, SIZE]).unwrap().value.as_int().unwrap();
        let contents = ev.node(&doc, &[DIRECTORY, 0, STREAMS, i, CONTENTS]).unwrap();
        if size == 0 {
            // A stream number that is not in use: no blocks and nothing to read.
            assert_eq!(ev.node(&doc, &[DIRECTORY, 0, STREAMS, i, BLOCKS]).unwrap().child_count, 0, "stream {i}");
            assert_eq!(contents.size_bits, 0, "stream {i}");
            continue;
        }
        // The body is placed where the blocks are, so the stream's own node
        // covers no bytes and its one child covers exactly the stream.
        let body = ev.node(&doc, &[DIRECTORY, 0, STREAMS, i, CONTENTS, 0]).unwrap();
        assert_eq!(body.size_bits, size as u64 * 8, "stream {i} is not read as its whole length");
        read += 1;
    }
    assert_eq!(read, 16, "sixteen of the nineteen streams hold something");

    // The named streams, which is how everything above stream four is found.
    let info = [DIRECTORY, 0, STREAMS, 1, CONTENTS, 0];
    let names = ev.node(&doc, &[info.as_slice(), &[5, 0]].concat()).unwrap().child_count;
    let named = ev.node(&doc, &[info.as_slice(), &[12]].concat()).unwrap().child_count;
    assert!(names >= named, "every named stream's name is in the buffer: {names} names, {named} entries");
    assert!(named >= 1, "a linker PDB always has at least one named stream");

    // What the debug information stream says the program was built for.
    let dbi = [DIRECTORY, 0, STREAMS, 3, CONTENTS, 0];
    assert_eq!(ev.node(&doc, &[dbi.as_slice(), &[0]].concat()).unwrap().value.as_int(), Some(-1));
    assert_eq!(
        ev.node(&doc, &[dbi.as_slice(), &[18]].concat()).unwrap().value,
        Value::Enum { raw: 0x8664, name: Some("amd64".into()), hex: false }
    );
}

/// The type records of every sample whose type stream lies in one run: as many
/// as the header's two type numbers are apart, and tiling the space it gives
/// them exactly. A record read one byte too wide puts every record after it on
/// the wrong byte, and the count is what catches that.
#[test]
fn the_type_records_number_what_the_header_says_and_tile_its_space() {
    let Some(dir) = folder() else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    let mut checked = 0;
    for name in ["msvc-x64-shapes.pdb", "msvc-x64-shapes-compiler.pdb", "msvc-x64-260-modules.pdb"] {
        let (doc, mut ev) = open(&dir, name, "pdb");
        // Stream 2 is the type stream, and only a stream in one run is read.
        let Ok(tpi) = ev.node(&doc, &[DIRECTORY, 0, STREAMS, 2, CONTENTS, 0]) else { continue };
        if tpi.type_name != "TpiStream" {
            continue;
        }
        let field = |ev: &mut Evaluator, i: usize| -> i128 {
            ev.node(&doc, &[DIRECTORY, 0, STREAMS, 2, CONTENTS, 0, i]).unwrap().value.as_int().unwrap()
        };
        let begin = field(&mut ev, 2);
        let end = field(&mut ev, 3);
        let record_bytes = field(&mut ev, 4);
        let records = ev.node(&doc, &[DIRECTORY, 0, STREAMS, 2, CONTENTS, 0, 15, 0]).unwrap();
        assert_eq!(records.child_count as i128, end - begin, "{name}: one record per type number");
        assert_eq!(records.size_bits, record_bytes as u64 * 8, "{name}: the records fill the space exactly");
        assert!(begin >= 0x1000, "{name}: type numbers below 0x1000 are the built-in types");
        checked += 1;
    }
    assert!(checked >= 1, "at least one sample has a type stream in a single run");
}

/// A stream whose blocks are scattered is its block list and nothing else.
/// Nothing built in one pass has one, which is why this is the sample with 260
/// translation units in it.
#[test]
fn a_scattered_stream_is_read_as_its_blocks_and_no_further() {
    let Some(dir) = folder() else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    let (doc, mut ev) = open(&dir, "msvc-x64-260-modules.pdb", "pdb");
    assert_eq!(ev.node(&doc, &[DIRECTORY, 0, STREAM_COUNT]).unwrap().value.as_int(), Some(278));
    let streams = ev.node(&doc, &[DIRECTORY, 0, STREAMS]).unwrap().child_count as usize;

    let mut scattered = 0;
    for i in 0..streams {
        let size = ev.node(&doc, &[DIRECTORY, 0, STREAMS, i, SIZE]).unwrap().value.as_int().unwrap();
        let blocks = ev.node(&doc, &[DIRECTORY, 0, STREAMS, i, BLOCKS]).unwrap().child_count;
        let contents = ev.node(&doc, &[DIRECTORY, 0, STREAMS, i, CONTENTS]).unwrap();
        if size <= 0 || contents.child_count > 0 {
            continue;
        }
        // Nothing read, and yet the stream has blocks: they are not one run.
        assert!(blocks > 1, "stream {i}: a stream of one block is always a run");
        assert_eq!(contents.size_bits, 0, "stream {i}");
        scattered += 1;
    }
    assert!(scattered >= 1, "this sample is here for its scattered streams");
}

/// The GUID and the age in the executable's debug directory are the ones in
/// the PDB's info stream. That pairing is what a debugger checks before it
/// trusts the symbols, and it is a fact about two files at once.
#[test]
fn the_executable_and_its_symbols_agree_on_which_build_they_are() {
    let Some(dir) = folder() else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    let symbols = read_sample(&dir, "msvc-x64-shapes.pdb");
    let doc = Document::new(MemSource(symbols.clone()));
    let mut ev = Evaluator::new(formats::builtin("pdb").unwrap());
    let info = [DIRECTORY, 0, STREAMS, 1, CONTENTS, 0];
    let guid = bytes_of(&mut ev, &doc, &[info.as_slice(), &[3]].concat(), &symbols);
    let age = ev.node(&doc, &[info.as_slice(), &[2]].concat()).unwrap().value.as_int().unwrap();
    assert_eq!(guid.len(), 16);

    // The CodeView record in the executable: `RSDS`, the same sixteen bytes,
    // the age, and the path the linker wrote the PDB to.
    let program = read_sample(&dir, "msvc-x64-shapes.exe");
    let at = program
        .windows(4)
        .position(|w| w == b"RSDS")
        .expect("the executable carries a CodeView record naming its PDB");
    assert_eq!(&program[at + 4..at + 20], &guid[..], "the GUID in the executable is the one in the PDB");
    let written_age = u32::from_le_bytes(program[at + 20..at + 24].try_into().unwrap());
    assert_eq!(i128::from(written_age), age, "the age in the executable is the one in the PDB");
}

/// A stream directory that does not fit in one block, which reads because its
/// blocks are consecutive. Modern `link.exe` refuses any page size below 4096
/// and so never writes one this small; this file came from Roslyn.
#[test]
fn a_directory_over_two_blocks_reads_when_the_two_are_side_by_side() {
    let Some(dir) = folder() else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    let (doc, mut ev) = open(&dir, "autoupdater-net-1.5.8-net40.pdb", "pdb");
    let block_size = ev.node(&doc, &[BLOCK_SIZE]).unwrap().value.as_int().unwrap();
    let directory_bytes = ev.node(&doc, &[DIRECTORY_BYTES]).unwrap().value.as_int().unwrap();
    assert_eq!(block_size, 512, "the small block size nothing built today produces");
    assert!(directory_bytes > block_size, "the directory needs more than one block");
    // And it reads all the same: the table, not the bytes of two blocks.
    assert_eq!(ev.node(&doc, &[DIRECTORY, 0, STREAM_COUNT]).unwrap().value.as_int(), Some(32));
    assert_eq!(ev.node(&doc, &[DIRECTORY, 0]).unwrap().size_bits, directory_bytes as u64 * 8);
}

/// A Portable PDB names its streams and says how many rows each metadata table
/// has. The row counts are what a reader wants first and what every index into
/// a table is bounded by.
#[test]
fn a_portable_pdb_says_which_tables_it_has_and_how_many_rows() {
    let Some(dir) = folder() else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    for name in ["dotnet-portable-shapes.pdb", "awssdk-core-3.7.100.14-net45.pdb"] {
        let (doc, mut ev) = open(&dir, name, "portablepdb");
        assert_eq!(ev.node(&doc, &[5]).unwrap().value, Value::Str("PDB v1.0".into()), "{name}");
        let streams = ev.node(&doc, &[8]).unwrap().child_count as usize;
        let named: Vec<String> = (0..streams)
            .map(|i| match ev.node(&doc, &[8, i, 2]).unwrap().value {
                Value::Str(s) => s.to_string(),
                other => panic!("{name}: stream {i} has no name, but {other:?}"),
            })
            .collect();
        for wanted in ["#Pdb", "#~", "#Strings", "#GUID", "#Blob"] {
            assert!(named.iter().any(|n| n == wanted), "{name}: no {wanted} stream, only {named:?}");
        }

        // The table stream's header: one row count per table the bit vector
        // says is present, which is what `pop_count` is for.
        let tables = named.iter().position(|n| n == "#~").unwrap();
        let valid = ev.node(&doc, &[9, tables, 5]).unwrap();
        let rows = ev.node(&doc, &[9, tables, 7]).unwrap();
        let present = match valid.value {
            Value::Flags { raw, .. } => raw.count_ones(),
            other => panic!("{name}: the valid tables are not flags, but {other:?}"),
        };
        assert_eq!(rows.child_count, u64::from(present), "{name}: a row count for every table present");
        assert!(present >= 1, "{name}: a Portable PDB has debug tables in it");

        // The string heap reads as the strings it holds, the first of them
        // empty so that an index of zero means no name.
        let strings = named.iter().position(|n| n == "#Strings").unwrap();
        assert_eq!(
            ev.node(&doc, &[9, strings, 0, 0]).unwrap().value,
            Value::Str(String::new().into()),
            "{name}: the string heap opens with the empty string"
        );
    }
}

/// The bytes a node covers, taken from the file rather than from the node's
/// preview: a value column shows the first few bytes of a long field, and this
/// wants all sixteen of a GUID.
fn bytes_of(ev: &mut Evaluator, doc: &Document<MemSource>, path: &[usize], bytes: &[u8]) -> Vec<u8> {
    let NodeInfo { offset_bits, size_bits, .. } = ev.node(doc, path).unwrap();
    assert_eq!(offset_bits % 8, 0, "{path:?} does not start on a byte");
    let at = (offset_bits / 8) as usize;
    bytes[at..at + (size_bits / 8) as usize].to_vec()
}

fn open(dir: &Path, name: &str, template: &str) -> (Document<MemSource>, Evaluator) {
    let bytes = read_sample(dir, name);
    let sniffed = formats::sniff(&bytes[..bytes.len().min(formats::SNIFF_WINDOW)], bytes.len() as u64);
    assert_eq!(sniffed, Some(template), "{name}");
    (Document::new(MemSource(bytes)), Evaluator::new(formats::builtin(template).unwrap()))
}

fn read_sample(dir: &Path, name: &str) -> Vec<u8> {
    std::fs::read(dir.join(name)).unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn folder() -> Option<PathBuf> {
    let named = std::env::var_os("QUBERO_SAMPLES").map(PathBuf::from);
    let beside = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../qubero-samples");
    named.into_iter().chain(std::iter::once(beside)).map(|p| p.join("pdb")).find(|p| p.is_dir())
}
