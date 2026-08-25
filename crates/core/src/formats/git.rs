//! The two files git calls an index, which are not the same file.
//!
//! `.git/index` is the staging area: one record per tracked path, holding the
//! object it points at and enough of a stat to tell whether the working copy
//! has moved on. It opens with `DIRC`.
//!
//! A `.idx` beside a pack is the other one: it says where in the pack each
//! object lives, so a fetch can find one without reading the whole thing. It
//! opens with a byte that cannot start a legal pack, then `tOc`.
//!
//! The pack index is the more interesting of the two. Its 256 fanout entries
//! are running totals: entry `n` counts every object whose first byte is `n`
//! or less, so the last of them is the number of objects, and the difference
//! between two neighbours is how many start with that byte. Four parallel
//! tables of that length follow, each holding one column of the record. This
//! IR can say all of that, and the count comes out of the fanout by asking
//! for element 255.

use crate::template::{Encoding, Endian::*, Expr as E, StrLen, Template, Ty as T};

/// The file modes git records, which are four of the many a filesystem has.
const MODE: &[(i128, &str)] = &[
    (0o100_644, "file"),
    (0o100_755, "executable"),
    (0o120_000, "symlink"),
    (0o160_000, "gitlink"),
];

/// A pack index: fanout, then one table per column.
pub fn git_pack_index() -> Template {
    // The last fanout entry counts every object in the pack.
    let count = E::elem("fanout", E::lit(255));
    Template::new(
        "gitpackidx",
        T::structure(
            "PackIndex",
            vec![
                ("magic", T::magic(b"\xfftOc")),
                ("version", T::u32(Big)),
                // Running totals by first byte of the object name.
                ("fanout", T::array(T::u32(Big), E::lit(256))),
                ("names", T::array(sha1(), count.clone()).counted_as("object")),
                ("crcs", T::array(T::u32(Big), count.clone()).counted_as("checksum")),
                // Where in the pack each object starts. A high bit set means
                // the real offset is in the table after this one, for packs
                // over two gigabytes.
                ("offsets", T::array(T::u32(Big), count).counted_as("offset")),
                ("large_offsets", T::bytes(E::Remaining.sub(E::lit(40)))),
                ("pack_checksum", sha1()),
                ("checksum", sha1()),
            ],
        ),
    )
}

/// The staging index: a header, one entry per path, and then extensions.
pub fn git_index() -> Template {
    Template::new(
        "gitindex",
        T::structure(
            "Index",
            vec![
                ("magic", T::magic(b"DIRC")),
                // Version 4 writes each path as a difference from the one
                // before it, which the entry below does not read.
                ("version", T::u32(Big)),
                ("entry_count", T::u32(Big)),
                ("entries", T::array(entry(), E::field("entry_count"))),
                // Cached trees, resolve-undo and whatever else was written,
                // then a checksum of everything above.
                ("extensions", T::bytes(E::Remaining.sub(E::lit(20)))),
                ("checksum", sha1()),
            ],
        ),
    )
}

/// One staged path. Everything above the object name is there so that git can
/// decide a file is unchanged without opening it.
fn entry() -> T {
    // The low twelve bits of the flags are the length of the path.
    let name_length = E::field("flags").sub(E::field("flags").div(E::lit(4096)).mul(E::lit(4096)));
    // An entry is padded with NULs to a multiple of eight, and there is always
    // at least one of them: 62 bytes of fields, then the path.
    let used = E::lit(62).add(name_length.clone());
    let pad = E::lit(8).sub(used.clone().sub(used.div(E::lit(8)).mul(E::lit(8))));

    T::structure_named(
        "Entry",
        "path",
        "",
        vec![
            ("ctime_seconds", T::u32(Big)),
            ("ctime_nanoseconds", T::u32(Big)),
            ("mtime_seconds", T::u32(Big)),
            ("mtime_nanoseconds", T::u32(Big)),
            ("dev", T::u32(Big)),
            ("ino", T::u32(Big)),
            ("mode", T::enumeration("Mode", T::u32(Big), MODE)),
            ("uid", T::u32(Big)),
            ("gid", T::u32(Big)),
            ("size", T::u32(Big)),
            ("object", sha1()),
            // Bit 15 marks a path assumed unchanged; bits 12 and 13 hold the
            // stage a merge conflict left it in.
            ("flags", T::flags("Flags", T::u16(Big), &[(15, "assume valid"), (14, "extended")])),
            ("path", T::text(StrLen::Fixed(name_length), Encoding::Utf8)),
            ("padding", T::bytes(pad)),
        ],
    )
    .counted_as("entry")
}

/// An object name, which is twenty raw bytes rather than the forty characters
/// everything shows.
fn sha1() -> T {
    T::bytes(E::lit(20))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    #[test]
    fn the_object_count_comes_out_of_the_last_fanout_entry() {
        let mut v = b"\xfftOc".to_vec();
        v.extend_from_slice(&2u32.to_be_bytes());
        // Two objects: one starting with 0x00, one with 0xff.
        for n in 0..256u32 {
            v.extend_from_slice(&(if n == 255 { 2u32 } else { 1 }).to_be_bytes());
        }
        v.extend_from_slice(&[0x00; 20]);
        v.extend_from_slice(&[0xff; 20]);
        v.extend_from_slice(&[0; 8]); // the crc table
        v.extend_from_slice(&12u32.to_be_bytes());
        v.extend_from_slice(&512u32.to_be_bytes());
        v.extend_from_slice(&[0xaa; 20]);
        v.extend_from_slice(&[0xbb; 20]);

        let d = Document::new(MemSource(v));
        let mut ev = Evaluator::new(git_pack_index());
        assert_eq!(ev.node(&d, &[3]).unwrap().child_count, 2);
        assert_eq!(ev.node(&d, &[5]).unwrap().child_count, 2);
        assert_eq!(ev.node(&d, &[5, 1]).unwrap().value, Value::UInt(512));
        assert_eq!(ev.node(&d, &[6]).unwrap().size_bits, 0);
    }

    #[test]
    fn an_entry_is_padded_to_a_multiple_of_eight() {
        fn entry_bytes(path: &str, mode: u32) -> Vec<u8> {
            let mut v = vec![0u8; 24];
            v.extend_from_slice(&mode.to_be_bytes());
            v.extend_from_slice(&[0; 12]);
            v.extend_from_slice(&[0x77; 20]);
            v.extend_from_slice(&(path.len() as u16).to_be_bytes());
            v.extend_from_slice(path.as_bytes());
            v.resize((v.len() + 8) / 8 * 8, 0);
            v
        }
        let mut v = b"DIRC".to_vec();
        v.extend_from_slice(&2u32.to_be_bytes());
        v.extend_from_slice(&2u32.to_be_bytes());
        v.extend_from_slice(&entry_bytes("src/main.rs", 0o100_644));
        v.extend_from_slice(&entry_bytes("run.sh", 0o100_755));
        v.extend_from_slice(&[0; 20]);

        let d = Document::new(MemSource(v));
        let mut ev = Evaluator::new(git_index());
        assert_eq!(ev.node(&d, &[3]).unwrap().child_count, 2);
        assert_eq!(ev.node(&d, &[3, 0, 12]).unwrap().value, Value::Str("src/main.rs".into()));
        assert_eq!(
            ev.node(&d, &[3, 1, 6]).unwrap().value,
            Value::Enum { raw: 0o100_755, name: Some("executable".into()), hex: false }
        );
        assert_eq!(ev.node(&d, &[3, 1, 12]).unwrap().value, Value::Str("run.sh".into()));
        // 62 fields plus six of path is 68, so four NULs take it to 72.
        assert_eq!(ev.node(&d, &[3, 1, 13]).unwrap().size_bits, 4 * 8);
        assert_eq!(ev.node(&d, &[4]).unwrap().size_bits, 0);
    }
}
