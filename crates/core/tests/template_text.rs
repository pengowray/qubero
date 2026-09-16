//! The IR as text, over every built-in template.
//!
//! Two things are checked of all of them, and the whole text of ten. The ten
//! are snapshots because the notation is for a reader: a change that makes a
//! line worse shows up as a diff in a file somebody can read, rather than as a
//! test that still passes. Rewrite them with `UPDATE_SNAPSHOTS=1 cargo test
//! --test template_text`, and read what changed before committing it.

use std::path::PathBuf;

use qubero_core::formats;
use qubero_core::template::{Endian, Expr as E, StructDef, Template, Ty, Until};
use qubero_core::template_text::render;

/// The formats whose whole text is kept: a signature and a chunk stream, a
/// header with times and a checksum, RIFF, an archive with two directories, a
/// program, a database, a stream of events, a table of fixed-width text, a box
/// tree that nests by name, and a file that is a graph of addresses.
const SNAPSHOTS: &[&str] = &["png", "gzip", "wav", "zip", "elf", "sqlite", "midi", "tar", "mp4", "hdf5"];

fn snapshot_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("snapshots").join("template_text")
}

#[test]
fn every_builtin_renders() {
    for name in formats::builtin_names() {
        let Some(t) = formats::builtin(name) else { panic!("{name} has no template") };
        let text = render(&t);
        assert!(!text.is_empty(), "{name} rendered nothing");
        assert!(text.starts_with(&format!("template {}\n", t.name)), "{name} has no heading");
    }
}

#[test]
fn a_template_renders_the_same_way_twice() {
    for name in formats::builtin_names() {
        let Some(t) = formats::builtin(name) else { continue };
        assert_eq!(render(&t), render(&t), "{name} renders differently each time");
    }
}

#[test]
fn every_field_of_the_root_is_named() {
    for name in formats::builtin_names() {
        let Some(t) = formats::builtin(name) else { continue };
        let text = render(&t);
        for field in root_fields(&t.root) {
            assert!(text.contains(&format!("{field}: ")), "{name} does not name its field {field}");
        }
    }
}

/// The fields of the root structure, through whatever wrappers the root has
/// around it.
fn root_fields(ty: &Ty) -> Vec<String> {
    fn def(ty: &Ty) -> Option<&StructDef> {
        match ty {
            Ty::Struct(d) => Some(d),
            Ty::Sized { inner, .. }
            | Ty::SizedBits { inner, .. }
            | Ty::At { inner, .. }
            | Ty::Origin { inner }
            | Ty::Decoded { inner, .. }
            | Ty::Nullable { inner, .. } => def(inner),
            Ty::Array { elem, .. } | Ty::Repeat { elem, .. } => def(elem),
            _ => None,
        }
    }
    def(ty).map(|d| d.fields.iter().map(|f| f.name.to_string()).collect()).unwrap_or_default()
}

#[test]
fn the_snapshots_still_read_the_same_way() {
    let update = std::env::var("UPDATE_SNAPSHOTS").is_ok_and(|v| v == "1");
    let dir = snapshot_dir();
    if update {
        std::fs::create_dir_all(&dir).expect("snapshot directory");
    }
    let mut stale = Vec::new();
    for name in SNAPSHOTS {
        let t = formats::builtin(name).unwrap_or_else(|| panic!("no {name} template"));
        let text = render(&t);
        let path = dir.join(format!("{name}.txt"));
        if update {
            std::fs::write(&path, &text).expect("write snapshot");
            continue;
        }
        let Ok(want) = std::fs::read_to_string(&path) else {
            stale.push(format!("{name}: no snapshot at {}", path.display()));
            continue;
        };
        // Written with `\n`, and read back the same way whatever the checkout
        // did to the line endings.
        if want.replace("\r\n", "\n") != text {
            stale.push(format!("{name}: the rendering has changed"));
        }
    }
    assert!(stale.is_empty(), "{}\nRerun with UPDATE_SNAPSHOTS=1 to rewrite them.", stale.join("\n"));
}

/// The notation for the parts of the IR no built-in format uses yet, kept in a
/// snapshot for the same reason the ten formats are: the notation is for a
/// reader, and a line that gets worse should show up as a diff somebody can
/// read. Everything here exists for the ImHex pattern converter, and this is
/// what it will write.
#[test]
fn the_newest_notation_still_reads_the_same_way() {
    let t = written_out();
    let text = render(&t);
    let path = snapshot_dir().join("notation.txt");
    if std::env::var("UPDATE_SNAPSHOTS").is_ok_and(|v| v == "1") {
        std::fs::create_dir_all(snapshot_dir()).expect("snapshot directory");
        std::fs::write(&path, &text).expect("write snapshot");
        return;
    }
    let want = std::fs::read_to_string(&path).expect("no snapshot for the notation");
    assert_eq!(want.replace("\r\n", "\n"), text, "Rerun with UPDATE_SNAPSHOTS=1 to rewrite it.");
}

/// One structure using each of the bitwise operators, both kinds of repeat
/// question, both space measurements, a peek at an address, and a union.
fn written_out() -> Template {
    let flags = E::field("flags");
    let union = Ty::union_structure(
        "Colour",
        vec![("packed", Ty::u32(Endian::Big)), ("bytes", Ty::array(Ty::u8(), E::lit(4))), ("high", Ty::u16(Endian::Big))],
    );
    let root = Ty::structure(
        "Header",
        vec![
            ("flags", Ty::u8()),
            ("count", Ty::u8()),
            ("set", Ty::computed(flags.clone().bit_or(E::lit(0x20)))),
            ("toggled", Ty::computed(flags.clone().bit_xor(E::lit(0x0f)))),
            ("clear", Ty::computed(flags.clone().bit_not().and(E::lit(0xff)))),
            ("here", Ty::computed(E::SpacePos)),
            ("whole", Ty::computed(E::SpaceSize)),
            ("magic", Ty::computed(E::PeekIn { at: Box::new(E::lit(0)), bits: 32, endian: Endian::Big })),
            ("colour", union),
            ("counted", Ty::repeat(Ty::u8(), Until::While(E::Idx.less_than(E::field("count"))))),
            ("while_room", Ty::repeat(Ty::u16(Endian::Big), Until::While(E::SpacePos.less_than(E::SpaceSize)))),
            ("until_zero", Ty::repeat(Ty::u8(), Until::Cond(E::field("value").equal_to(E::lit(0))))),
        ],
    );
    Template::new("notation", root)
}
