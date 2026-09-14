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
use qubero_core::eval::{Evaluator, Explain, Value};
use qubero_core::formats::{self, fits_tile};
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

    // `TFORM3 = '1E'`: one float, which `TSCAL3 = 3` and `TZERO3 = 0.4` say
    // is worth 3.7, as astropy reads it to within a millionth.
    assert_eq!(ev.node(&doc, &at(&[3, 1, 0, 0, 2])).unwrap().type_name, "Scaled column");
    assert_eq!(ev.node(&doc, &at(&[3, 1, 0, 0, 2, 2, 0, 0])).unwrap().value, Value::Float(1.1));
    let Value::Float(worth) = ev.node(&doc, &at(&[3, 1, 0, 0, 2, 2, 0, 1])).unwrap().value else { panic!("not a real") };
    assert!((worth - 3.7000000715255736).abs() < 1e-6, "{worth}");

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
/// genuine scaling with fractions in it, over floats. Every worth checked here
/// is what astropy 7 reads for the same cell.
///
/// The collection holds no file whose scale is written with an exponent over
/// data it scales, so that is checked by
/// `formats::fits::tests::a_scale_written_with_an_exponent_scales_the_column`.
#[test]
fn a_fractional_scale_scales_the_column_as_astropy_reads_it() {
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
    // The middle column is a scaling by 2.5 with a zero point of 0.4: each
    // float is the float on disk, and what it is worth beside it.
    let flux = ev.node(&doc, &[0, 1, 3, 1, 0, 0, 1]).unwrap();
    assert_eq!(flux.type_name, "Scaled column");
    let mut said = |i: usize| ev.node(&doc, &[0, 1, 3, 0, 1, i]).unwrap().value;
    assert_eq!((said(2), said(3)), (Value::Float(2.5), Value::Float(0.4)));
    // astropy works from the float's exact bits, and this from the float as
    // its row reads, so the two meet within a millionth and not at the bit.
    for (row, stored, astropy) in [(0, 0.24000001, 1.0000000238418578), (1, 0.64, 1.999999964237213), (2, 1.04, 2.9999999046325683), (3, 1.4399999, 3.9999998450279235)] {
        assert_eq!(ev.node(&doc, &[0, 1, 3, 1, row, 0, 1, 2, 0, 0]).unwrap().value, Value::Float(stored), "row {row}");
        let worth = ev.node(&doc, &[0, 1, 3, 1, row, 0, 1, 2, 0, 1]).unwrap();
        assert_eq!(worth.type_name, "computed real");
        let Value::Float(got) = worth.value else { panic!("row {row} is worth {:?}", worth.value) };
        assert!((got - astropy).abs() < 1e-6, "row {row}: {got} against astropy's {astropy}");
    }
    // And the sum a reader is shown for the first of them.
    let rel = ev.relations(&doc, &[0, 1, 3, 1, 0, 0, 1, 2, 0, 1]).unwrap();
    assert_eq!((rel[0].written.as_str(), rel[0].substituted.as_str()), ("stored * scale + zero", "0.24000001 * 2.5 + 0.4"));
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

/// Every tile of a compressed image decompressed and put where it goes in the
/// image, the first axis fastest, and the whole image hashed: FNV-1a over each
/// pixel as a big-endian 64-bit float, with every NaN written as the one quiet
/// NaN. Also what each tile's first step was, and how many tiles came from
/// each column.
fn every_tile(doc: &Document<MemSource>, ev: &mut Evaluator, hdu: usize) -> (u64, Vec<f64>, Vec<String>) {
    let rows = ev.node(doc, &[0, hdu, 3, ROWS]).unwrap().child_count;
    let mut image: Vec<f64> = Vec::new();
    let mut shape: Vec<u64> = Vec::new();
    let mut firsts = Vec::new();
    for t in 0..rows as usize {
        let tile = ev.fits_tile(doc, &[0, hdu, 3, ROWS, t]).unwrap().expect("a tile");
        assert_eq!(tile.problem, None, "hdu {hdu} tile {t}: {:?}", tile.steps);
        assert_eq!(Some(tile.pixels.len() as u64), tile.pixel_count(), "hdu {hdu} tile {t}");
        if image.is_empty() {
            let n = ev.node(doc, &[0, hdu, 3, 1]).unwrap().child_count as usize;
            shape = (0..n).map(|i| ev.node(doc, &[0, hdu, 3, 1, i]).unwrap().value.as_int().unwrap() as u64).collect();
            image = vec![f64::INFINITY; shape.iter().product::<u64>() as usize];
        }
        firsts.push(tile.steps.first().map_or(String::new(), |s| s.what.clone()));
        // Every pixel of the tile, by its place along each axis.
        let mut at = vec![0u64; shape.len()];
        for v in &tile.pixels {
            let mut flat = 0u64;
            let mut stride = 1u64;
            for k in 0..shape.len() {
                flat += (tile.start[k] + at[k]) * stride;
                stride *= shape[k];
            }
            assert!(image[flat as usize].is_infinite(), "hdu {hdu} tile {t} overlaps another at {flat}");
            image[flat as usize] = *v;
            for k in 0..shape.len() {
                at[k] += 1;
                if at[k] < tile.shape[k] {
                    break;
                }
                at[k] = 0;
            }
        }
    }
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for v in &image {
        let bits = if v.is_nan() { f64::NAN.to_bits() } else { v.to_bits() };
        for b in bits.to_be_bytes() {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    }
    (h, image, firsts)
}

/// The tiles of every compressed image in the collection against what astropy
/// reads the same file as. The hashes are astropy's `.data` for each unit, as
/// `tools/make_fits_samples.py`'s files and `comp.fits` were read by astropy
/// 8.0.1, hashed the way [`every_tile`] hashes. A hash that matches is every
/// pixel of every tile the same float to the bit, NaNs in the same places.
#[test]
fn every_tile_of_every_compressed_sample_matches_astropy() {
    let samples: &[(&str, usize, u64, &str, &[f64])] = &[
        ("fits/comp.fits", 1, 0xdebc_8305_21f1_52aa, "RICE_1", &[7.0, 7.0, 7.0]),
        ("fits/rice.fits", 1, 0x0c66_fb6d_5905_b465, "RICE_1", &[1200.0, 1265.0, 1311.0]),
        ("fits/rice.fits", 2, 0x0272_bb36_2f98_ce1a, "RICE_1", &[154.0, 161.0, 171.0]),
        ("fits/dithered.fits", 1, 0x0fc2_644e_9d2c_2f52, "RICE_1", &[-1.744675, 11.033096, 22.59725]),
        ("fits/dithered.fits", 2, 0xd529_2601_e47a_dd53, "gzip", &[-1.80230097, 10.99756288, 22.59584548]),
        ("fits/gzip2.fits", 1, 0x6459_9677_2adb_8e88, "gzip", &[1183.0, 1264.0, 1283.0]),
        ("fits/gzip2.fits", 2, 0x45ad_6018_d883_80eb, "gzip", &[21.033499, 36.869576, 44.493217]),
        ("fits/gzip2.fits", 3, 0x9899_bb47_c761_aabf, "stored", &[1183.0, 1264.0, 1283.0]),
        ("fits/plio.fits", 1, 0xd7a1_0a19_52d7_5c89, "PLIO_1", &[0.0, 0.0, 0.0]),
        ("fits/plio.fits", 2, 0xa4b2_0a25_8f45_13ea, "PLIO_1", &[0.0, 0.0, 0.0]),
        ("fits/plio.fits", 3, 0x63ec_8fec_5772_b55e, "PLIO_1", &[0.0, 0.0, 0.0]),
        ("fits/hcompress.fits", 1, 0x5a13_b0de_2060_0a01, "HCOMPRESS_1", &[1199.0, 1280.0, 1331.0]),
        ("fits/hcompress.fits", 2, 0xf7e4_2caf_f7fd_8a51, "HCOMPRESS_1", &[583.0, 738.0, 979.0]),
        ("fits/hcompress.fits", 3, 0xc681_6d1a_5cf7_5b83, "HCOMPRESS_1", &[583.0, 738.0, 977.0]),
        ("fits/hcompress.fits", 4, 0xc9e3_4e1c_8fd6_d463, "HCOMPRESS_1", &[27.23504066467285, 39.18748092651367, 46.35916519165039]),
        ("fits/fallback.fits", 1, 0xdd07_3336_b90f_b70c, "RICE_1", &[78.8850092917391, 66.05480325863505, 112.33276328282162]),
        ("fits/fallback.fits", 2, 0x791a_7ff1_dce7_1c0b, "RICE_1", &[1219.0, 1242.0, 1326.0]),
        ("fits/fallback.fits", 3, 0x502e_12af_1c43_d94c, "RICE_1", &[209020.0, 301610.0, 349476.0]),
    ];
    let mut ran = 0;
    for (file, hdu, want, first_step, first_pixels) in samples {
        let Some((doc, mut ev)) = read(file) else {
            eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
            return;
        };
        let (hash, image, firsts) = every_tile(&doc, &mut ev, *hdu);
        for (got, want) in image.iter().zip(first_pixels.iter()) {
            assert!((got - want).abs() < 1e-5, "{file} hdu {hdu}: {got} against {want}");
        }
        assert_eq!(firsts[0], *first_step, "{file} hdu {hdu}");
        assert_eq!(hash, *want, "{file} hdu {hdu}: 0x{hash:016x}");
        ran += 1;
    }
    assert_eq!(ran, samples.len());
}

/// The steps a tile reports are the ones its bytes took: which kinds of Rice
/// block, a tile that fell back to gzip, the blank and the zero a dithered
/// tile kept, and the unshuffle GZIP_2 needs.
#[test]
fn a_real_tile_reports_the_steps_its_bytes_took() {
    let Some((doc, mut ev)) = read("fits/rice.fits") else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    let steps = |ev: &mut Evaluator, doc: &Document<MemSource>, hdu: usize, tile: usize| -> Vec<(String, String)> {
        let t = ev.fits_tile(doc, &[0, hdu, 3, ROWS, tile]).unwrap().unwrap();
        t.steps.into_iter().map(|s| (s.what, s.note)).collect()
    };
    // One tile of the 32-bit image is a single value: every block is zero.
    let flat = steps(&mut ev, &doc, 1, 1);
    assert_eq!(flat[0].0, "RICE_1");
    assert!(flat[0].1.contains("10 blocks of 32 pixels: 0 Rice-coded, 10 all-zero, 0 uncoded at 32 bits"), "{flat:?}");
    // One is noise across the whole range: every block is written uncoded.
    let noise = steps(&mut ev, &doc, 1, 3);
    assert!(noise[0].1.contains("0 Rice-coded, 0 all-zero, 10 uncoded"), "{noise:?}");
    // And the last tile along the first axis is cut short, 10 by 16.
    let edge = ev.fits_tile(&doc, &[0, 1, 3, ROWS, 2]).unwrap().unwrap();
    assert_eq!((edge.start, edge.shape), (vec![40, 0], vec![10, 16]));
    // The 8-bit image is Rice at one byte a pixel.
    assert!(steps(&mut ev, &doc, 2, 0)[0].1.starts_with("1 byte per pixel; "));

    let Some((doc, mut ev)) = read("fits/dithered.fits") else { return };
    let first = steps(&mut ev, &doc, 1, 0);
    let whats: Vec<&str> = first.iter().map(|(w, _)| w.as_str()).collect();
    assert_eq!(whats, ["RICE_1", "SUBTRACTIVE_DITHER_2", "dither", "blank", "zero"]);
    // ZDITHER0 is 1234, so tile 0 starts at seed 1233, and at the random
    // number 500 times that one says.
    assert!(first[2].1.ends_with("ZDITHER0 = 1234, this tile starting at r[312]"), "{first:?}");
    assert_eq!(first[4].1, "1 pixel stored as -2147483646, which SUBTRACTIVE_DITHER_2 reserves for exactly 0.0");
    // The tile of one value would not quantize, and was gzipped as floats.
    let fallback = ev.fits_tile(&doc, &[0, 1, 3, ROWS, 5]).unwrap().unwrap();
    assert_eq!(fallback.stored, Some(fits_tile::Stored::Gzip));
    let whats: Vec<&str> = fallback.steps.iter().map(|s| s.what.as_str()).collect();
    assert_eq!(whats, ["gzip", "read"]);
    assert_eq!(fallback.steps[1].note, "500 pixels, as big-endian f32");
    assert!(fallback.pixels.iter().all(|p| *p == 2.5));

    let Some((doc, mut ev)) = read("fits/gzip2.fits") else { return };
    let shuffled = steps(&mut ev, &doc, 1, 0);
    let whats: Vec<&str> = shuffled.iter().map(|(w, _)| w.as_str()).collect();
    assert_eq!(whats, ["gzip", "unshuffle", "read"]);
    assert_eq!(shuffled[2].1, "50 pixels, as big-endian i16");
    let stored = steps(&mut ev, &doc, 3, 0);
    assert_eq!(stored[0], ("stored".to_string(), "taken as they are (ZCMPTYPE = NOCOMPRESS)".to_string()));

    // The cursor on a tile's compressed bytes in the heap finds that tile
    // through the descriptor that placed them. Asked of the evaluator that has
    // just read the third unit's rows: reading a later unit once dropped this
    // unit's header, and its heap could not be placed without the cards.
    ev.node(&doc, &[0, 3, 3, ROWS, 0]).unwrap();
    let heap = ev.node(&doc, &[0, 1, 3, HEAP]).unwrap();
    let tile_7 = ev.node(&doc, &[0, 1, 3, HEAP, 7]).unwrap();
    let found = ev.locate(&doc, tile_7.offset_bits + 8).unwrap();
    assert!(found.starts_with(&[0, 1, 3, HEAP]) && tile_7.offset_bits > heap.offset_bits);
    assert_eq!(ev.fits_tile(&doc, &found).unwrap().unwrap().index, 7);
}

/// What the inspector is handed for the cursor on a compressed image: from a
/// byte of a tile in the heap, and from a descriptor four levels into its row.
#[test]
fn the_inspector_explains_a_tile_from_its_bytes_and_from_its_row() {
    let Some((doc, mut ev)) = read("fits/dithered.fits") else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    // Each row has two descriptors, COMPRESSED_DATA and GZIP_COMPRESSED_DATA,
    // so the heap's third array is the second tile's compressed bytes.
    let tile_1 = ev.node(&doc, &[0, 1, 3, HEAP, 2]).unwrap();
    assert!(tile_1.child_count > 0);
    let on_byte = ev.locate(&doc, tile_1.offset_bits + 3 * 8).unwrap();
    let on_count = vec![0, 1, 3, ROWS, 1, 0, 0, 1, 0, 0];
    for path in [on_byte, on_count] {
        let Explain::FitsTile { index, tiles, start, shape, image_shape, algorithm, column, steps, values, total, pixels, element_type, problem, .. } =
            ev.explain(&doc, &path, None).unwrap()
        else {
            panic!("not a tile at {path:?}");
        };
        assert_eq!((index, tiles, start, shape, image_shape), (1, Some(6), vec![25, 0], vec![25, 20], vec![50, 60]));
        assert_eq!((algorithm.as_str(), column, element_type.as_str(), problem), ("RICE_1", Some("COMPRESSED_DATA"), "f32", None));
        assert_eq!((values.len(), total, pixels), (32, 500, Some(500)));
        assert_eq!(steps.len(), 3);
        // The pixels are written as the 32-bit floats they are, the way
        // astropy prints them.
        assert_eq!(values[..3], ["-1.9223341", "10.630576", "22.740988"]);
    }
    // A cell of the row that is a float, the tile's ZSCALE, still explains
    // the float it is.
    assert!(matches!(ev.explain(&doc, &[0, 1, 3, ROWS, 1, 0, 2, 0], None).unwrap(), Explain::Float { .. }));
}

/// The steps a PLIO or HCOMPRESS tile reports: what its list or its stream
/// was made of, and the smoothing a tile with `SMOOTH` had.
#[test]
fn a_plio_or_hcompress_tile_reports_the_steps_its_bytes_took() {
    let Some((doc, mut ev)) = read("fits/plio.fits") else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    let steps = |ev: &mut Evaluator, doc: &Document<MemSource>, hdu: usize, tile: usize| -> Vec<(String, String)> {
        let t = ev.fits_tile(doc, &[0, hdu, 3, ROWS, tile]).unwrap().unwrap();
        assert_eq!(t.problem, None);
        t.steps.into_iter().map(|s| (s.what, s.note)).collect()
    };
    // The first tile of the 32-bit mask has the ramps, a pixel stored for each
    // step of them.
    assert_eq!(
        steps(&mut ev, &doc, 1, 0),
        [(
            "PLIO_1".to_string(),
            "pixel list, 65 words of 16 bits; a 7-word header, then 58 instructions: 8 runs of zeros, 7 runs of the current value, 2 runs of zeros ending in the value, 0 values set from two words, 3 changes to the value with no pixel written, 38 changes to the value with one pixel written".to_string()
        )]
    );
    // 70,000 is too large for twelve bits, so the list sets it from two words.
    assert!(steps(&mut ev, &doc, 1, 5)[0].1.contains("1 value set from two words, "));
    // 4,530 zeros before the one pixel of the 16-bit mask, which is more
    // than one instruction counts.
    assert!(steps(&mut ev, &doc, 2, 0)[0].1.contains(" 2 runs of zeros, 0 runs of the current value, 1 run of zeros ending in the value, "));

    let Some((doc, mut ev)) = read("fits/hcompress.fits") else { return };
    let lossless = steps(&mut ev, &doc, 1, 0);
    assert_eq!(
        lossless,
        [
            (
                "HCOMPRESS_1".to_string(),
                "12 × 16 coefficients, scale 0 (lossless); coarsest coefficient 41,728, stored whole in the header, not in the bit planes; 35 bit planes (12 for quadrant 1, 8 each for quadrants 2 and 3, 7 for quadrant 4): 12 as quadtrees, 23 written directly; 191 sign bits, one per non-zero coefficient".to_string()
            ),
            ("H-transform".to_string(), "inverse H-transform (a 2-D Haar wavelet) over 4 levels, coarsest first".to_string()),
        ]
    );
    // The last tile along both axes is odd both ways.
    let edge = ev.fits_tile(&doc, &[0, 1, 3, ROWS, 8]).unwrap().unwrap();
    assert_eq!((edge.start, edge.shape), (vec![24, 32], vec![11, 13]));
    let smoothed = steps(&mut ev, &doc, 3, 0);
    let whats: Vec<&str> = smoothed.iter().map(|(w, _)| w.as_str()).collect();
    assert_eq!(whats, ["HCOMPRESS_1", "undigitize", "H-transform"]);
    assert_eq!(smoothed[1].1, "each coefficient × 10, the scale it was divided by when compressed");
    assert!(smoothed[0].1.starts_with("24 × 32 coefficients, scale 10 (lossy); coarsest coefficient -736, "));
    assert!(smoothed[2].1.ends_with("; smoothed at every level (SMOOTH = 1): each level's differences adjusted by at most 5"));
    // The same stream without SMOOTH says nothing of smoothing.
    assert!(!steps(&mut ev, &doc, 2, 0)[2].1.contains("SMOOTH"));
    let dithered = steps(&mut ev, &doc, 4, 0);
    let whats: Vec<&str> = dithered.iter().map(|(w, _)| w.as_str()).collect();
    assert_eq!(whats, ["HCOMPRESS_1", "undigitize", "H-transform", "SUBTRACTIVE_DITHER_1", "dither"]);
}

/// A tile of 64-bit floats that would not quantize, and tiles of integer
/// images moved into the two columns astropy only ever fills with floats: each
/// is read from its own column at the width its bytes come to, and not
/// unquantized.
#[test]
fn a_tile_outside_compressed_data_is_read_from_its_column_at_its_width() {
    let Some((doc, mut ev)) = read("fits/fallback.fits") else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    for (hdu, tile, column, read_note) in [
        (1, 3, fits_tile::Stored::Gzip, "500 pixels, as big-endian f64"),
        (2, 1, fits_tile::Stored::Gzip, "500 pixels, as big-endian i16"),
        (2, 2, fits_tile::Stored::Gzip, "500 pixels, as big-endian i16"),
        (3, 3, fits_tile::Stored::Uncompressed, "500 pixels, as big-endian i32"),
    ] {
        let t = ev.fits_tile(&doc, &[0, hdu, 3, ROWS, tile]).unwrap().unwrap();
        assert_eq!((t.stored, t.problem.as_deref()), (Some(column), None), "hdu {hdu} tile {tile}");
        let whats: Vec<&str> = t.steps.iter().map(|s| s.what.as_str()).collect();
        let first = if column == fits_tile::Stored::Gzip { "gzip" } else { "stored" };
        assert_eq!(whats, [first, "read"], "hdu {hdu} tile {tile}");
        assert_eq!(t.steps[1].note, read_note, "hdu {hdu} tile {tile}");
    }
    // The float tile is one value, and the tiles beside it were Rice coded.
    let flat = ev.fits_tile(&doc, &[0, 1, 3, ROWS, 3]).unwrap().unwrap();
    assert!(flat.pixels.iter().all(|p| *p == -12.25));
    assert_eq!(ev.fits_tile(&doc, &[0, 2, 3, ROWS, 0]).unwrap().unwrap().stored, Some(fits_tile::Stored::Compressed));
}
