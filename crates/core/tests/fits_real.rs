//! A real FITS binary table, read as its rows and columns.
//!
//! `tb.fits` is one of the files the FITS tools have been tested against since
//! 2001: a header-only primary unit, and then a table of four columns, one of
//! each of the kinds a table is mostly written in. What it checks is that the
//! `TFORMn` cards typed the columns, since that is the part no fixed layout
//! could do: the width of a row is in the header, and what is in it is in the
//! header too, spelled out a keyword at a time.
//!
//! The file lives in the sample collection rather than here. Point
//! `QUBERO_SAMPLES` at it, or keep it beside the repository as
//! `qubero-samples`. With neither, the test says so and passes.

use std::path::PathBuf;

use qubero_core::document::Document;
use qubero_core::eval::{Evaluator, Value};
use qubero_core::formats;
use qubero_core::source::MemSource;

fn sample() -> Option<PathBuf> {
    named("fits/tb.fits")
}

/// A file of the sample collection, wherever the collection is.
fn named(file: &str) -> Option<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Ok(set) = std::env::var("QUBERO_SAMPLES") {
        roots.extend(set.split(';').filter(|s| !s.is_empty()).map(PathBuf::from));
    }
    roots.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../qubero-samples"));
    roots.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../qubero-samples"));
    roots.into_iter().map(|r| r.join(file)).find(|p| p.exists())
}

/// Where a compressed image keeps its rows and its heap: after what the image
/// and a tile are, and the columns.
const ROWS: usize = 4;
const HEAP: usize = 5;

/// `comp.fits` is a tile-compressed image, which FITS writes as a binary table:
/// one row per tile, and in each row a `1PB` cell, a descriptor pointing at
/// that tile's compressed bytes in the heap after the rows. Three hundred rows
/// and a heap of 66,896 bytes.
#[test]
fn a_real_tile_compressed_images_heap_reads_as_the_arrays_its_rows_point_at() {
    let Some(path) = named("fits/comp.fits") else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    let doc = Document::new(MemSource(std::fs::read(&path).unwrap()));
    let mut ev = Evaluator::new(formats::builtin("fits").unwrap());
    let table = [0usize, 1, 3];
    let at = |tail: &[usize]| -> Vec<usize> { table.iter().chain(tail).copied().collect() };

    // The table says it is an image, what shape, and how it was cut up: 440
    // pixels by 300, a tile a row of 440 pixels, each one Rice coded.
    assert_eq!(ev.node(&doc, &table).unwrap().type_name, "Compressed image");
    let text = |v: Value| match v {
        Value::Str(s) => s.trim().to_string(),
        other => panic!("not text: {other:?}"),
    };
    assert_eq!(text(ev.node(&doc, &at(&[0])).unwrap().value), "RICE_1");
    let mut shape = |field: usize| -> Vec<i128> {
        (0..2).map(|i| ev.node(&doc, &at(&[field, i])).unwrap().value.as_int().unwrap()).collect()
    };
    assert_eq!((shape(1), shape(2)), (vec![440, 300], vec![440, 1]));
    let rows = ev.node(&doc, &at(&[ROWS])).unwrap();
    assert_eq!(rows.child_count, 300);
    assert_eq!(ev.node(&doc, &at(&[ROWS, 0])).unwrap().type_name, "Tile");

    let heap = ev.node(&doc, &at(&[HEAP])).unwrap();
    assert_eq!(heap.size_bits, 66_896 * 8);
    // One array per row, since every row has one descriptor.
    assert_eq!(heap.child_count, 300);

    // Every array is where its descriptor says and as long as it counts, and
    // holds bytes, as the `B` after the `P` says.
    let mut claimed = 0u64;
    for row in 0..300usize {
        let count = ev.node(&doc, &at(&[ROWS, row, 0, 0, 1, 0, 0])).unwrap().value.as_int().unwrap() as u64;
        let offset = ev.node(&doc, &at(&[ROWS, row, 0, 0, 1, 0, 1])).unwrap().value.as_int().unwrap() as u64;
        let array = ev.node(&doc, &at(&[HEAP, row])).unwrap();
        assert_eq!(array.type_name, "u8[]");
        assert_eq!((array.offset_bits, array.child_count), (heap.offset_bits + offset * 8, count), "row {row}");
        claimed += count;
    }
    // The tiles fill the heap: nothing in it is left over.
    assert_eq!(claimed, 66_896);

    // The cursor on a byte of a tile finds the tile, and says which row put it
    // there.
    let middle = heap.offset_bits + 40_000 * 8;
    let found = ev.locate(&doc, middle).unwrap();
    assert_eq!(&found[..4], &at(&[HEAP])[..]);
    let origins = ev.origins(&doc, &found[..5]).unwrap();
    let row = found[4];
    assert_eq!(origins[0].label, format!("rows[{row}].cells[0].descriptors[0]"));
}

#[test]
fn a_real_binary_tables_columns_are_typed_by_its_header() {
    let Some(path) = sample() else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    let doc = Document::new(MemSource(std::fs::read(&path).unwrap()));
    let mut ev = Evaluator::new(formats::builtin("fits").unwrap());
    // The primary unit holds nothing, so the table is the second one.
    let hdu = [0usize, 1];
    let at = |tail: &[usize]| -> Vec<usize> { hdu.iter().chain(tail).copied().collect() };

    // Two rows of twelve bytes, and no heap after them.
    let rows = ev.node(&doc, &at(&[3, 1])).unwrap();
    assert_eq!(rows.child_count, 2);
    assert_eq!(ev.node(&doc, &at(&[3, 1, 0])).unwrap().size_bits, 12 * 8);
    assert_eq!(ev.node(&doc, &at(&[3, 2])).unwrap().size_bits, 0);

    // `TFORM1 = '1J'`: one 32-bit integer, big-endian as everything here is.
    let c1 = ev.node(&doc, &at(&[3, 1, 0, 0, 0])).unwrap();
    assert_eq!((c1.type_name.as_str(), c1.child_count), ("i32 be[]", 1));
    assert_eq!(ev.node(&doc, &at(&[3, 1, 0, 0, 0, 0])).unwrap().value, Value::Int(1));
    assert_eq!(ev.node(&doc, &at(&[3, 1, 1, 0, 0, 0])).unwrap().value, Value::Int(2));

    // `TFORM2 = '3A'`: three characters, read as one run of text.
    let c2 = ev.node(&doc, &at(&[3, 1, 0, 0, 1])).unwrap();
    assert_eq!(c2.size_bits, 3 * 8);
    assert!(matches!(&c2.value, Value::Str(s) if s.trim() == "abc"), "{:?}", c2.value);

    // `TFORM3 = '1E'`: one float.
    assert_eq!(ev.node(&doc, &at(&[3, 1, 0, 0, 2, 0])).unwrap().value, Value::Float(1.1));

    // `TFORM4 = '1L'`: a logical, written as the letter T or F; the first row says F.
    let c4 = ev.node(&doc, &at(&[3, 1, 0, 0, 3])).unwrap();
    assert_eq!(c4.size_bits, 8);
    assert!(matches!(&c4.value, Value::Str(s) if s == "F"), "{:?}", c4.value);

    // The columns add up to the width the header gave the row.
    let widths: u64 = (0..4).map(|i| ev.node(&doc, &at(&[3, 1, 0, 0, i])).unwrap().size_bits).sum();
    assert_eq!(widths, 12 * 8);
}

/// Read a file of the collection with the FITS template, or say why not.
fn read(file: &str) -> Option<(Document<MemSource>, Evaluator)> {
    let path = named(file)?;
    let doc = Document::new(MemSource(std::fs::read(&path).unwrap()));
    Some((doc, Evaluator::new(formats::builtin("fits").unwrap())))
}

/// `wide.fits` is a binary table of forty columns, which is more than a
/// template that wrote a field per column could ever have read: it stopped at
/// 32 and said so.
#[test]
fn a_real_table_of_forty_columns_reads_every_one_of_them() {
    let Some((doc, mut ev)) = read("fits/wide.fits") else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    let cells = ev.node(&doc, &[0, 1, 3, 1, 0, 0]).unwrap();
    assert_eq!(cells.child_count, 40);
    let last = ev.node(&doc, &[0, 1, 3, 1, 0, 0, 39]).unwrap();
    assert_eq!((last.type_name.as_str(), last.name.as_str()), ("i32 be[]", "[39] c40"));
    // Forty columns of one 32-bit integer fill the row the header declared.
    assert_eq!(ev.node(&doc, &[0, 1, 3, 1, 0]).unwrap().size_bits, 40 * 4 * 8);
}

/// `scaled.fits` says what its columns and its pixels are worth. Two of the
/// three columns are the unsigned convention, one each way, and the third is a
/// genuine scaling that stays the float it is written as.
#[test]
fn a_real_scaled_table_reads_the_convention_as_a_type_and_a_scaling_as_a_note() {
    let Some((doc, mut ev)) = read("fits/scaled.fits") else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    // `TFORM1 = I` with `TZERO1 = 32768` is the unsigned convention: astropy
    // wrote 65535, and what is on disk is the signed 32767.
    let counts = ev.node(&doc, &[0, 1, 3, 1, 0, 0, 0]).unwrap();
    assert_eq!((counts.type_name.as_str(), counts.name.as_str()), ("Scaled column", "[0] counts"));
    assert_eq!(ev.node(&doc, &[0, 1, 3, 1, 3, 0, 0, 2, 0, 0]).unwrap().value, Value::Int(32767));
    assert_eq!(ev.node(&doc, &[0, 1, 3, 1, 3, 0, 0, 2, 0, 1]).unwrap().value, Value::Int(65535));
    // The first row is the other end of it: 0 written as the signed -32768.
    assert_eq!(ev.node(&doc, &[0, 1, 3, 1, 0, 0, 0, 2, 0, 0]).unwrap().value, Value::Int(-32768));
    assert_eq!(ev.node(&doc, &[0, 1, 3, 1, 0, 0, 0, 2, 0, 1]).unwrap().value, Value::Int(0));
    // `TFORM3 = B` with `TZERO3 = -128` goes the other way: FITS writes that
    // type unsigned, so -128 is written as the unsigned 0.
    assert_eq!(ev.node(&doc, &[0, 1, 3, 1, 0, 0, 2, 2, 0, 0]).unwrap().value, Value::UInt(0));
    assert_eq!(ev.node(&doc, &[0, 1, 3, 1, 0, 0, 2, 2, 0, 1]).unwrap().value, Value::Int(-128));
    // The middle column is a scaling, not a type: the float stays a float and
    // the column says what its numbers are worth.
    let flux = ev.node(&doc, &[0, 1, 3, 1, 0, 0, 1]).unwrap();
    assert_eq!(flux.type_name, "f32 be[]");
    let mut said = |i: usize| match &ev.node(&doc, &[0, 1, 3, 0, 1, i]).unwrap().value {
        Value::Str(s) => s.trim().to_string(),
        other => panic!("not text: {other:?}"),
    };
    assert_eq!((said(2), said(3)), ("2.5".to_string(), "0.4".to_string()));
    // And the image after it says what its pixels are worth, by `BZERO`.
    let pixels = ev.node(&doc, &[0, 2, 3]).unwrap();
    assert_eq!(pixels.type_name, "Scaled[]");
    assert_eq!(ev.node(&doc, &[0, 2, 3, 0, 0]).unwrap().value, Value::Int(-32768));
    assert_eq!(ev.node(&doc, &[0, 2, 3, 0, 1]).unwrap().value, Value::Int(0));
}

/// `continue.fits` keeps a value too long for one card in the cards after it.
#[test]
fn a_real_long_string_reads_as_the_pieces_the_cards_hold() {
    let Some((doc, mut ev)) = read("fits/continue.fits") else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    let mut piece = |card: usize| match &ev.node(&doc, &[0, 0, 0, card, 2, 1, 1, 0, 0]).unwrap().value {
        Value::Str(s) => s.to_string(),
        other => panic!("not text: {other:?}"),
    };
    // The card that opens the value ends in `&`, and the card after it, which
    // was one undifferentiated run of text before, holds the next piece.
    assert!(piece(4).ends_with('&'), "{:?}", piece(4));
    assert_eq!(piece(5), "astropy writes it across CONTINUE cards&");
}
