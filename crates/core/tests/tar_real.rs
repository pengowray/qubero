//! A real GNU tar with sparse files in it, one of whose maps spills past the
//! header into extension blocks.
//!
//! The extension blocks are the point. They sit between a header and its
//! data, hold nothing but map entries and a flag, and have no checksum. Read
//! as headers they were checked as headers, and a block of offsets never
//! sums to what its "checksum" field holds: `checks_real` reported three
//! mismatches on a file GNU tar wrote and reads back without complaint.
//!
//! The file lives in the sample collection rather than here. Point
//! `QUBERO_SAMPLES` at it, or keep it beside the repository as
//! `qubero-samples`. With neither, the test says so and passes.

use std::path::PathBuf;

use qubero_core::document::Document;
use qubero_core::eval::{Evaluator, Value};
use qubero_core::formats;
use qubero_core::source::MemSource;

fn sample(name: &str) -> Option<PathBuf> {
    qubero_samples::roots().into_iter().map(|r| r.join("tar").join(name)).find(|p| p.exists())
}

fn read(name: &str) -> Option<(Document<MemSource>, Evaluator)> {
    let path = sample(name)?;
    let doc = Document::new(MemSource(std::fs::read(&path).unwrap()));
    Some((doc, Evaluator::new(formats::builtin("tar").unwrap())))
}

fn named(ev: &mut Evaluator, d: &Document<MemSource>, path: &[usize], name: &str) -> Vec<usize> {
    ev.child_named(d, path, name).unwrap().unwrap_or_else(|| panic!("no {name} under {path:?}"))
}

/// The node called `name` under `path`.
fn field(ev: &mut Evaluator, d: &Document<MemSource>, path: &[usize], name: &str) -> qubero_core::eval::NodeInfo {
    let at = named(ev, d, path, name);
    ev.node(d, &at).unwrap()
}

#[test]
fn a_sparse_map_that_outgrows_its_header_is_read_as_extension_blocks() {
    let Some((d, mut ev)) = read("gnu-sparse-old-extended.tar") else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    // Two sparse files, a plain one, and ten blocks of zeros to fill out
    // GNU tar's default record of twenty blocks.
    let entries = ev.node(&d, &[0]).unwrap();
    assert_eq!(entries.child_count, 13);

    // The first sparse file fits its map in the header: no extensions, and
    // the data starts in the block after the header.
    let first = [0, 0];
    assert_eq!(field(&mut ev, &d, &first, "name").value, Value::Str("sparse".into()));
    assert_eq!(field(&mut ev, &d, &first, "typeflag").value.as_int(), Some(b'S' as i128));
    assert_eq!(field(&mut ev, &d, &first, "isextended").value.as_int(), Some(0));
    assert_eq!(field(&mut ev, &d, &first, "extensions").size_bits, 0);
    assert_eq!(field(&mut ev, &d, &first, "data").offset_bits, 512 * 8);
    assert_eq!(field(&mut ev, &d, &first, "realsize").value.as_int(), Some(0o14000000));

    // The second has 95 runs of data in its map: four in the header, then
    // five extension blocks, the last only half full and flagged as the last.
    let second = [0, 1];
    assert_eq!(field(&mut ev, &d, &second, "name").value, Value::Str("sparse2".into()));
    assert_eq!(field(&mut ev, &d, &second, "isextended").value.as_int(), Some(1));
    let extensions = named(&mut ev, &d, &second, "extensions");
    let ext = ev.node(&d, &extensions).unwrap();
    assert_eq!(ext.child_count, 5);
    assert_eq!((ext.offset_bits, ext.size_bits), (5 * 512 * 8, 5 * 512 * 8));
    for (i, flag) in [1, 1, 1, 1, 0].into_iter().enumerate() {
        let block = [&extensions[..], &[i]].concat();
        assert_eq!(ev.node(&d, &block).unwrap().type_name, "TarSparseExtension");
        assert_eq!(field(&mut ev, &d, &block, "isextended").value.as_int(), Some(flag));
        // No checksum in an extension block, so nothing to check.
        let flag_at = named(&mut ev, &d, &block, "isextended");
        assert!(ev.check_of(&d, &flag_at).unwrap().is_none());
    }
    let last_map = named(&mut ev, &d, &[&extensions[..], &[4]].concat(), "sparse");
    assert_eq!(ev.node(&d, &[&last_map[..], &[0, 0]].concat()).unwrap().value.as_int(), Some(0o523404000));
    // The data comes after the extensions, and is as long as the header says.
    let data = field(&mut ev, &d, &second, "data");
    assert_eq!((data.offset_bits, data.size_bits), (10 * 512 * 8, 50369 * 8));

    // Then a plain ustar entry, placed right after the sparse file's padding.
    let third = [0, 2];
    assert_eq!(ev.node(&d, &third).unwrap().type_name, "TarEntry");
    assert_eq!(ev.node(&d, &third).unwrap().offset_bits, 109 * 512 * 8);
    assert_eq!(field(&mut ev, &d, &third, "typeflag").value.as_int(), Some(b'0' as i128));

    // Every real header sums to what it wrote down, and nothing else is
    // checked as one.
    for entry in [first, second, third] {
        let checksum = named(&mut ev, &d, &entry, "checksum");
        let v = ev.run_check(&d, &checksum).unwrap().expect("a header's checksum is checked");
        assert!(v.ok, "entry {entry:?}: computed {}, stored {}", v.computed, v.stored);
    }
}
