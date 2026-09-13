//! Real NIfTI and Analyze files, read as nibabel reads them.
//!
//! Six files from nibabel's own test data: a big-endian NIfTI-1 volume, a
//! little-endian 4-D series whose scaling has fractions in it, a gzipped
//! NIfTI-2 file with extensions, a gzipped NIfTI-1 volume of a few bytes, the
//! header half of a NIfTI-1 pair, and an Analyze 7.5 header from SPM2. Every
//! number checked here is one `nibabel` 5.4 gave for the same file: a header
//! field, or a voxel at an index nibabel writes `data[x, y, z]`, which the
//! template reaches as `voxels[z][y][x]` because the first index varies
//! fastest.
//!
//! The files live in the sample collection rather than here. Point
//! `QUBERO_SAMPLES` at it, or keep it beside the repository as
//! `qubero-samples`. With neither, the test says so and passes.

use std::path::PathBuf;

use qubero_core::document::Document;
use qubero_core::eval::{Evaluator, NodeInfo, Value};
use qubero_core::formats;
use qubero_core::source::{MemSource, Source};

fn sample(name: &str) -> Option<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Ok(set) = std::env::var("QUBERO_SAMPLES") {
        roots.extend(set.split(';').filter(|s| !s.is_empty()).map(PathBuf::from));
    }
    roots.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../qubero-samples"));
    roots.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../qubero-samples"));
    roots.into_iter().map(|r| r.join("nifti").join(name)).find(|p| p.exists())
}

/// The file, the template `sniff` picks for it, and a reading by that
/// template.
fn read(name: &str) -> Option<(Document<MemSource>, &'static str, Evaluator)> {
    let bytes = std::fs::read(sample(name)?).unwrap();
    let sniffed = formats::sniff(&bytes[..bytes.len().min(formats::SNIFF_WINDOW)], bytes.len() as u64).expect("sniffs");
    let ev = Evaluator::new(formats::template(sniffed).unwrap());
    Some((Document::new(MemSource(bytes)), sniffed, ev))
}

fn path<S: Source>(ev: &mut Evaluator, d: &Document<S>, names: &[&str]) -> Vec<usize> {
    let mut p = Vec::new();
    for n in names {
        p = ev.child_named(d, &p, n).unwrap().unwrap_or_else(|| panic!("no {n} under {p:?}"));
    }
    p
}

fn node<S: Source>(ev: &mut Evaluator, d: &Document<S>, names: &[&str]) -> NodeInfo {
    let p = path(ev, d, names);
    ev.node(d, &p).unwrap()
}

/// The voxel nibabel calls `data[index]`.
fn voxel<S: Source>(ev: &mut Evaluator, d: &Document<S>, index: &[usize]) -> Value {
    let mut p = path(ev, d, &["voxels"]);
    p.extend(index.iter().rev());
    ev.node(d, &p).unwrap().value
}

/// A gzipped file's stream, opened in a space of its own: the template the
/// space was given, and the reading over it.
fn open_gzip(name: &str, check: impl FnOnce(&str, bool, &mut Evaluator, &Document<qubero_core::source::ArcSource>)) -> bool {
    let Some((d, sniffed, mut ev)) = read(name) else { return false };
    assert_eq!(sniffed, "gzip", "{name} is a gzip file first");
    let at = path(&mut ev, &d, &["compressed"]);
    let id = ev.open_space(&d, 0, &at).expect("resolves").expect("the stream opens");
    let space = ev.space_mut(id).expect("just opened");
    let (template, recognised) = (space.template.clone(), space.recognised);
    let (inner, doc) = space.reading();
    check(&template, recognised, inner, doc);
    true
}

#[test]
fn a_big_endian_volume_reads_as_nibabel_reads_it() {
    let Some((d, sniffed, mut ev)) = read("anatomical.nii") else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    assert_eq!(sniffed, "nifti");
    assert_eq!(ev.node(&d, &[]).unwrap().type_name, "NIfTI-1");
    assert_eq!(node(&mut ev, &d, &["header", "sizeof_hdr"]).type_name, "i32 be");
    assert_eq!(node(&mut ev, &d, &["header", "datatype"]).value.as_int(), Some(4));
    assert_eq!(node(&mut ev, &d, &["header", "descrip"]).value, Value::Str("spm - 3D normalized".into()));
    assert_eq!(node(&mut ev, &d, &["header", "qform_code"]).value.as_int(), Some(2));
    assert_eq!(node(&mut ev, &d, &["header", "xyzt_units", "space"]).value.as_int(), Some(2));
    // A slope of 1 and an intercept of 0 is no scaling, so nothing is worked
    // out and the voxels are the integers.
    assert_eq!(node(&mut ev, &d, &["header", "scaled"]).value.as_int(), Some(0));
    // (33, 41, 25): 25 slices of 41 rows of 33, ending with the file.
    let voxels = node(&mut ev, &d, &["voxels"]);
    assert_eq!((voxels.type_name.as_str(), voxels.child_count), ("i16 be[][][]", 25));
    assert_eq!(voxels.offset_bits, 352 * 8);
    assert_eq!(voxels.offset_bits + voxels.size_bits, d.len_bits());
    for (index, want) in [([0, 0, 0], 10712), ([1, 0, 0], 10463), ([0, 1, 0], 6349), ([32, 40, 24], 2971), ([16, 20, 12], 11881)] {
        assert_eq!(voxel(&mut ev, &d, &index).as_int(), Some(want), "data{index:?}");
    }
}

/// SPM's 4-D series, scaled by a slope and an intercept with fractions in
/// them. Each voxel is the integer on disk, the one `dataobj.get_unscaled()`
/// gives, and what it is worth, the one `get_fdata()` gives.
#[test]
fn functional_voxels_are_worth_what_scl_slope_says() {
    let Some((d, sniffed, mut ev)) = read("functional.nii") else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    assert_eq!(sniffed, "nifti");
    assert_eq!(node(&mut ev, &d, &["header", "sizeof_hdr"]).type_name, "i32 le");
    assert_eq!(node(&mut ev, &d, &["header", "scaled"]).value.as_int(), Some(1));
    // As the rows show them: the shortest decimals that read back as the
    // file's four bytes, which nibabel reads as 0.07540696859359741 and
    // 3100.76171875.
    assert_eq!(node(&mut ev, &d, &["header", "scl_slope"]).value, Value::Float(0.07540697));
    assert_eq!(node(&mut ev, &d, &["header", "scl_inter"]).value, Value::Float(3100.7617));
    // (17, 21, 3, 20): 20 volumes of 3 slices of 21 rows of 17.
    let voxels = node(&mut ev, &d, &["voxels"]);
    assert_eq!((voxels.type_name.as_str(), voxels.child_count), ("Scaled[][][][]", 20));
    assert_eq!(voxels.offset_bits + voxels.size_bits, d.len_bits());
    let at = |ev: &mut Evaluator, index: &[usize], part: usize| {
        let mut p = path(ev, &d, &["voxels"]);
        p.extend(index.iter().rev());
        p.push(part);
        ev.node(&d, &p).unwrap()
    };
    // nibabel 5.4's `get_fdata()`, which works in doubles from the exact bits
    // of the two floats. This works from the floats as their rows read, so
    // the two agree to within a few hundred-thousandths and not to the bit.
    for (index, stored, fdata) in [
        ([0, 0, 0, 0], 11980, 4004.137202501297),
        ([1, 0, 0, 0], 13831, 4143.715501368046),
        ([0, 1, 0, 0], 14493, 4193.634914577007),
        ([0, 0, 1, 0], 7910, 3697.2308403253555),
        ([0, 0, 0, 1], 12452, 4039.729291677475),
        ([16, 20, 2, 19], 379, 3129.3409598469734),
        ([8, 10, 1, 10], 11093, 3937.251221358776),
    ] {
        assert_eq!(at(&mut ev, &index, 0).value.as_int(), Some(stored), "data{index:?}");
        let worth = at(&mut ev, &index, 1);
        assert_eq!(worth.type_name, "computed real");
        let Value::Float(got) = worth.value else { panic!("data{index:?} is worth {:?}", worth.value) };
        assert!((got - fdata).abs() < 1e-4, "data{index:?}: {got} against nibabel's {fdata}");
    }
    // And the formula a reader is shown for the first voxel.
    let mut first = path(&mut ev, &d, &["voxels"]);
    first.extend([0, 0, 0, 0, 1]);
    let rel = ev.relations(&d, &first).unwrap();
    assert_eq!(rel[0].written, "stored * header.scl_slope + header.scl_inter");
    assert_eq!(rel[0].substituted, "11980 * 0.07540697 + 3100.7617");
}

/// A `.nii.gz` is a gzip file, and its stream is recognised as NIfTI-2 by the
/// magic four bytes into it.
#[test]
fn a_gzipped_nifti_2_file_opens_as_nifti_inside_the_gzip() {
    let opened = open_gzip("example_nifti2.nii.gz", |template, recognised, ev, d| {
        assert_eq!((template, recognised), ("nifti", true));
        assert_eq!(ev.node(d, &[]).unwrap().type_name, "NIfTI-2");
        assert_eq!(node(ev, d, &["header", "vox_offset"]).value.as_int(), Some(608));
        assert_eq!(node(ev, d, &["header", "descrip"]).value, Value::Str("FSL3.3".into()));
        // nibabel's `get_dim_info()` says (0, 1, 2), counting axes from 0;
        // the bits say 1, 2 and 3, counting them as `dim` does.
        for (name, axis) in [("freq_dim", 1), ("phase_dim", 2), ("slice_dim", 3)] {
            assert_eq!(node(ev, d, &["header", "dim_info", name]).value.as_int(), Some(axis), "{name}");
        }
        assert_eq!(node(ev, d, &["header", "xyzt_units", "space"]).value.as_int(), Some(2));
        assert_eq!(node(ev, d, &["header", "xyzt_units", "time"]).value.as_int(), Some(1));
        // Two comments, 32 bytes each on disk.
        let extensions = node(ev, d, &["extensions"]);
        assert_eq!((extensions.type_name.as_str(), extensions.child_count), ("NiftiExtension[]", 2));
        let list = path(ev, d, &["extensions"]);
        for (i, text) in ["extcomment1", "extlongcomment2"].into_iter().enumerate() {
            let record = [list.clone(), vec![i]].concat();
            assert_eq!(ev.node(d, &record).unwrap().size_bits, 32 * 8);
            assert_eq!(ev.node(d, &[record.clone(), vec![1]].concat()).unwrap().value.as_int(), Some(6));
            assert_eq!(ev.node(d, &[record, vec![2]].concat()).unwrap().value, Value::Str(text.into()));
        }
        // (32, 20, 12, 2).
        let voxels = node(ev, d, &["voxels"]);
        assert_eq!((voxels.offset_bits, voxels.child_count), (608 * 8, 2));
        for (index, want) in [([0, 0, 0, 0], 424), ([1, 0, 0, 0], 428), ([0, 1, 0, 0], 394), ([31, 19, 11, 1], 457), ([16, 10, 6, 1], 266)] {
            assert_eq!(voxel(ev, d, &index).as_int(), Some(want), "data{index:?}");
        }
    });
    if !opened {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
    }
}

/// NIfTI-1 signs the end of its header, so the stream is recognised by bytes
/// 344 in rather than by its front.
#[test]
fn a_gzipped_nifti_1_file_opens_as_nifti_inside_the_gzip() {
    let opened = open_gzip("standard.nii.gz", |template, recognised, ev, d| {
        assert_eq!((template, recognised), ("nifti", true));
        assert_eq!(ev.node(d, &[]).unwrap().type_name, "NIfTI-1");
        let voxels = node(ev, d, &["voxels"]);
        assert_eq!((voxels.type_name.as_str(), voxels.child_count), ("u8[][][]", 7));
        assert_eq!(voxels.offset_bits + voxels.size_bits, d.len_bits());
        for (index, want) in [([0, 0, 0], 0), ([1, 0, 0], 255), ([0, 1, 0], 255), ([3, 4, 6], 255), ([2, 2, 3], 0)] {
            assert_eq!(voxel(ev, d, &index).as_int(), Some(want), "data{index:?}");
        }
    });
    if !opened {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
    }
}

#[test]
fn the_header_of_a_pair_has_no_voxels() {
    let Some((d, sniffed, mut ev)) = read("nifti1.hdr") else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    assert_eq!(sniffed, "nifti");
    assert_eq!(ev.node(&d, &[]).unwrap().type_name, "NIfTI-1 header");
    assert_eq!(ev.child_named(&d, &[], "voxels").unwrap(), None);
    assert_eq!(node(&mut ev, &d, &["header", "magic"]).value, Value::Str("ni1".into()));
    assert_eq!(node(&mut ev, &d, &["header", "sizeof_hdr"]).type_name, "i32 le");
    // The voxels would be at the front of the `.img`.
    assert_eq!(node(&mut ev, &d, &["header", "data_offset"]).value.as_int(), Some(0));
    assert_eq!(node(&mut ev, &d, &["header", "sform_code"]).value.as_int(), Some(4));
    assert_eq!(node(&mut ev, &d, &["header", "descrip"]).value, Value::Str("FSL4.0".into()));
    let dim = path(&mut ev, &d, &["header", "dim"]);
    let dims: Vec<_> = (0..4).map(|i| ev.node(&d, &[dim.clone(), vec![i]].concat()).unwrap().value.as_int().unwrap()).collect();
    assert_eq!(dims, vec![3, 91, 109, 91]);
    // The four extension bytes, saying none follow, and then the end.
    assert_eq!(node(&mut ev, &d, &["extender"]).size_bits, 32);
    assert_eq!(node(&mut ev, &d, &["extensions"]).size_bits, 0);
}

#[test]
fn an_analyze_header_reads_as_analyze() {
    let Some((d, sniffed, mut ev)) = read("analyze.hdr") else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    assert_eq!(sniffed, "analyze");
    assert_eq!(ev.node(&d, &[]).unwrap().type_name, "Analyze 7.5");
    assert_eq!(node(&mut ev, &d, &["header", "sizeof_hdr"]).type_name, "i32 be");
    assert_eq!(node(&mut ev, &d, &["header", "datatype"]).value.as_int(), Some(2));
    assert_eq!(node(&mut ev, &d, &["header", "bitpix"]).value.as_int(), Some(8));
    assert_eq!(node(&mut ev, &d, &["header", "vox_units"]).value, Value::Str("mm".into()));
    assert_eq!(node(&mut ev, &d, &["header", "glmax"]).value.as_int(), Some(255));
    assert_eq!(node(&mut ev, &d, &["header", "descrip"]).value, Value::Str("ICBM AVG 152 T1 TAL LIN".into()));
    // SPM's origin, which nibabel reads as (46, 64, 37, 0, 0), written as
    // big-endian 16-bit numbers where Analyze put a string.
    let Value::Bytes { preview, .. } = node(&mut ev, &d, &["header", "originator"]).value else { panic!("bytes") };
    assert_eq!(preview, vec![0, 46, 0, 64, 0, 37, 0, 0, 0, 0]);
}
