//! systemd journal files: a log that is a small database rather than a stream
//! of lines.
//!
//! A header, and then an arena of objects. Every distinct field value in the
//! file is stored once as a data object, and a log entry is a list of
//! pointers to the ones it used, which is why a journal of a million entries
//! all saying `_SYSTEMD_UNIT=sshd.service` is not a million copies of that
//! string. Hash tables and arrays of entries, both objects themselves, are
//! what make it searchable without reading the file end to end.
//!
//! Two things change the shape of what is in it. `compact`, which systemd
//! turns on by default since 2022, writes the offsets inside entries as 32
//! bits instead of 64, so a reader that does not look at the flag misreads
//! every entry in every journal written since. And a data object's payload
//! may be compressed on its own, which the object's flags say and which is as
//! far as this goes: naming the compressor is not decompressing it.

use crate::template::{Encoding, Endian::Little, Expr as E, StrLen, Template, Ty as T, Until};

/// What one object in the arena is.
const OBJECTS: &[(i128, &str)] = &[
    (0, "unused"),
    (1, "data"),
    (2, "field"),
    (3, "entry"),
    (4, "data hash table"),
    (5, "field hash table"),
    (6, "entry array"),
    (7, "tag"),
];

/// Whether the file is closed, being written, or rotated away.
const STATES: &[(i128, &str)] = &[(0, "offline"), (1, "online"), (2, "archived")];

/// How long an object's own header is, before whatever the type adds.
const OBJECT_HEADER: i128 = 16;

/// Where `header_size` is written, in bytes from the start of the file. The
/// header has to be measured before it can be read, since how much of it a
/// given systemd wrote is the one thing it says about itself.
const HEADER_SIZE_AT: i128 = 88;

/// Bit 4 of `incompatible_flags`: entries and entry arrays keep their offsets
/// in 32 bits, and data objects carry two more fields.
const COMPACT_BIT: u32 = 4;

pub fn journal() -> Template {
    Template::new(
        "journal",
        T::structure(
            "Journal",
            vec![
                ("header", T::sized(E::peek_at(E::lit(HEADER_SIZE_AT * 8), 64, Little), header())),
                ("objects", T::repeat(object(), Until::End)),
            ],
        ),
    )
}

/// One when this file was written in the compact layout.
fn compact() -> E {
    E::within(&["header", "incompatible_flags"]).bit(COMPACT_BIT)
}

fn header() -> T {
    T::structure(
        "JournalHeader",
        vec![
            ("signature", T::magic(b"LPKSHHRH")),
            (
                "compatible_flags",
                T::flags("JournalCompatible", T::u32(Little), &[(0, "sealed"), (1, "tail entry boot id")]),
            ),
            (
                "incompatible_flags",
                T::flags(
                    "JournalIncompatible",
                    T::u32(Little),
                    &[(0, "xz"), (1, "lz4"), (2, "keyed hash"), (3, "zstd"), (4, "compact")],
                ),
            ),
            ("state", T::enumeration("JournalState", T::u8(), STATES)),
            ("reserved", T::bytes(E::lit(7))),
            ("file_id", id128()),
            ("machine_id", id128()),
            ("tail_entry_boot_id", id128()),
            ("seqnum_id", id128()),
            ("header_size", T::u64(Little)),
            ("arena_size", T::u64(Little)),
            ("data_hash_table_offset", T::u64(Little)),
            ("data_hash_table_size", T::u64(Little)),
            ("field_hash_table_offset", T::u64(Little)),
            ("field_hash_table_size", T::u64(Little)),
            ("tail_object_offset", T::u64(Little)),
            ("n_objects", T::u64(Little)),
            ("n_entries", T::u64(Little)),
            ("tail_entry_seqnum", T::u64(Little)),
            ("head_entry_seqnum", T::u64(Little)),
            ("entry_array_offset", T::u64(Little)),
            ("head_entry_realtime", T::u64(Little)),
            ("tail_entry_realtime", T::u64(Little)),
            ("tail_entry_monotonic", T::u64(Little)),
            // Everything past here was added a release at a time, and the
            // header's own size is what says how far a given file goes. A
            // journal from 2012 stops after the monotonic timestamp.
            ("n_data", if_room(8, T::u64(Little))),
            ("n_fields", if_room(8, T::u64(Little))),
            ("n_tags", if_room(8, T::u64(Little))),
            ("n_entry_arrays", if_room(8, T::u64(Little))),
            ("data_hash_chain_depth", if_room(8, T::u64(Little))),
            ("field_hash_chain_depth", if_room(8, T::u64(Little))),
            ("tail_entry_array_offset", if_room(4, T::u32(Little))),
            ("tail_entry_array_n_entries", if_room(4, T::u32(Little))),
            ("tail_entry_offset", if_room(8, T::u64(Little))),
            // A newer systemd than this template knows about wrote something
            // here, and the header said how much.
            ("unread", T::bytes(E::Remaining)),
        ],
    )
}

/// A field that is there only while the header still has room for it, which
/// is how a file written by an older systemd stops early without anything
/// after it being read from the wrong bytes.
fn if_room(bytes: i128, ty: T) -> T {
    T::switch(E::lit(bytes - 1).less_than(E::Remaining), vec![(1, ty)], T::bytes(E::lit(0)))
}

/// The 128-bit identifiers systemd stamps on a machine, a boot and a file.
/// They are written as bytes and read as the 32 hex digits everything else
/// prints them as.
fn id128() -> T {
    T::bytes(E::lit(16))
}

fn object() -> T {
    T::structure_named(
        "JournalObject",
        "type",
        "payload",
        vec![
            ("type", T::enumeration("JournalObjectType", T::u8(), OBJECTS)),
            (
                "flags",
                T::flags("ObjectFlags", T::u8(), &[(0, "xz"), (1, "lz4"), (2, "zstd")]),
            ),
            ("reserved", T::bytes(E::lit(6))),
            // The whole object, this header counted. The padding that follows
            // it to the next eight-byte boundary is not counted.
            ("size", T::u64(Little)),
            (
                "payload",
                T::switch(
                    written(),
                    vec![(1, T::sized(E::field("size").sub(E::lit(OBJECT_HEADER)), body()))],
                    // A journal is written into a file that was allocated up
                    // front, so what follows the last object is a run of
                    // zeros that says it is nothing rather than an object of
                    // no length. Reading it as one would place a record per
                    // sixteen bytes of empty file.
                    T::bytes(E::Remaining),
                ),
            ),
            ("padding", T::switch(written(), vec![(1, T::bytes(pad8(E::field("size"))))], T::bytes(E::lit(0)))),
        ],
    )
    .counted_as("object")
}

/// One when this object says it holds something: an object is at least its
/// own header, so a smaller size is a region nobody has written yet.
fn written() -> E {
    E::lit(OBJECT_HEADER - 1).less_than(E::field("size"))
}

fn body() -> T {
    T::switch(
        E::field("type"),
        vec![
            (1, data()),
            (2, field()),
            (3, entry()),
            (4, hash_table("DataHashTable")),
            (5, hash_table("FieldHashTable")),
            (6, entry_array()),
            (7, tag()),
        ],
        T::bytes(E::Remaining),
    )
}

/// One distinct `FIELD=value` in the file, and the chains that find it again:
/// the next data object in its hash bucket, the next value of the same field,
/// and where the entries that used it are.
fn data() -> T {
    T::structure(
        "DataObject",
        vec![
            ("hash", T::u64(Little)),
            ("next_hash_offset", T::u64(Little)),
            ("next_field_offset", T::u64(Little)),
            ("entry_offset", T::u64(Little)),
            ("entry_array_offset", T::u64(Little)),
            ("n_entries", T::u64(Little)),
            ("tail_entry_array_offset", only_compact(T::u32(Little))),
            ("tail_entry_array_n_entries", only_compact(T::u32(Little))),
            (
                "payload",
                // Compressed on its own when the object's flags say so, and
                // the field it holds is then not readable from here.
                T::switch(
                    E::field("flags"),
                    vec![(0, T::text(StrLen::Fixed(E::Remaining), Encoding::Utf8))],
                    T::bytes(E::Remaining),
                ),
            ),
        ],
    )
}

/// The name of a field, once, with the head of the chain of every value it
/// has ever had in this file.
fn field() -> T {
    T::structure(
        "FieldObject",
        vec![
            ("hash", T::u64(Little)),
            ("next_hash_offset", T::u64(Little)),
            ("head_data_offset", T::u64(Little)),
            ("payload", T::text(StrLen::Fixed(E::Remaining), Encoding::Utf8)),
        ],
    )
}

/// One log entry: when it happened, and a pointer to each of the values it
/// was made of. The text is not here at all.
fn entry() -> T {
    T::structure(
        "EntryObject",
        vec![
            ("seqnum", T::u64(Little)),
            ("realtime", T::u64(Little)),
            ("monotonic", T::u64(Little)),
            ("boot_id", id128()),
            ("xor_hash", T::u64(Little)),
            ("items", T::repeat(entry_item(), Until::End)),
        ],
    )
}

/// A pointer to one data object. In the compact layout that is all it is; in
/// the older one it carries a copy of that object's hash beside it.
fn entry_item() -> T {
    T::switch(
        compact(),
        vec![(1, T::u32(Little).counted_as("item"))],
        T::inline_structure("EntryItem", vec![("object_offset", T::u64(Little)), ("hash", T::u64(Little))])
            .counted_as("item"),
    )
}

/// The list that makes a journal seekable: the offsets of a run of entries,
/// and where the next such list is.
fn entry_array() -> T {
    T::structure(
        "EntryArrayObject",
        vec![
            ("next_entry_array_offset", T::u64(Little)),
            (
                "items",
                T::repeat(
                    T::switch(compact(), vec![(1, T::u32(Little))], T::u64(Little)).counted_as("item"),
                    Until::End,
                ),
            ),
        ],
    )
}

/// A bucket per hash, each holding the two ends of a chain of objects that
/// landed in it. There is one of these for data and one for field names, and
/// both are objects in the arena like everything else.
fn hash_table(name: &str) -> T {
    T::structure(
        name,
        vec![(
            "items",
            T::repeat(
                T::inline_structure(
                    "HashItem",
                    vec![("head_hash_offset", T::u64(Little)), ("tail_hash_offset", T::u64(Little))],
                )
                .counted_as("bucket"),
                Until::End,
            ),
        )],
    )
}

/// What a sealed journal signs itself with, so that a log that was tampered
/// with after the fact can be told from one that was not.
fn tag() -> T {
    T::structure(
        "TagObject",
        vec![("seqnum", T::u64(Little)), ("epoch", T::u64(Little)), ("tag", T::bytes(E::lit(32)))],
    )
}

/// A field the compact layout added, and the older layout does not have.
fn only_compact(ty: T) -> T {
    T::switch(compact(), vec![(1, ty)], T::bytes(E::lit(0)))
}

/// How many bytes of padding follow an object of `n` bytes, to bring the next
/// one back onto an eight-byte boundary. The same subtraction as `dtb` and
/// `cpio` do to four.
fn pad8(n: E) -> E {
    let over = n.clone().sub(n.div(E::lit(8)).mul(E::lit(8)));
    let pad = E::lit(8).sub(over);
    pad.clone().sub(pad.div(E::lit(8)).mul(E::lit(8)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        document::Document,
        eval::{Evaluator, Value},
        source::MemSource,
    };

    const HEADER: usize = 272;

    /// A header of the length systemd 254 writes, with `compact` set or not.
    fn header_bytes(compact: bool) -> Vec<u8> {
        let mut v = b"LPKSHHRH".to_vec();
        v.extend_from_slice(&0u32.to_le_bytes()); // compatible flags
        v.extend_from_slice(&(if compact { 1u32 << COMPACT_BIT } else { 0 }).to_le_bytes());
        v.push(2); // archived
        v.extend_from_slice(&[0; 7]);
        v.extend_from_slice(&[0x11; 16 * 4]); // the four identifiers
        v.extend_from_slice(&(HEADER as u64).to_le_bytes());
        v.resize(HEADER, 0);
        v
    }

    /// One object: its header, its payload, and the padding to the next
    /// eight-byte boundary.
    fn object_bytes(kind: u8, flags: u8, payload: &[u8]) -> Vec<u8> {
        let mut v = vec![kind, flags, 0, 0, 0, 0, 0, 0];
        v.extend_from_slice(&((payload.len() + 16) as u64).to_le_bytes());
        v.extend_from_slice(payload);
        while v.len() % 8 != 0 {
            v.push(0);
        }
        v
    }

    /// A data object holding one `FIELD=value`, with the six offsets in front
    /// of it that every one of them carries.
    fn data_bytes(text: &[u8], compact: bool) -> Vec<u8> {
        let mut p = vec![0u8; 40]; // hash and the four offsets
        p.extend_from_slice(&7u64.to_le_bytes()); // n_entries
        if compact {
            p.extend_from_slice(&0u32.to_le_bytes());
            p.extend_from_slice(&0u32.to_le_bytes());
        }
        p.extend_from_slice(text);
        object_bytes(1, 0, &p)
    }

    #[test]
    fn the_header_says_how_much_of_itself_there_is() {
        let d = Document::new(MemSource(header_bytes(false)));
        let mut e = Evaluator::new(journal());
        assert_eq!(e.node(&d, &[0, 4]).unwrap().size_bits, 7 * 8); // the reserved run
        assert_eq!(e.node(&d, &[0, 9]).unwrap().value.as_int(), Some(HEADER as i128));
        // The last field this template knows about is written, and nothing
        // is left over after it.
        assert_eq!(e.node(&d, &[0, 32]).unwrap().size_bits, 8 * 8);
        assert_eq!(e.node(&d, &[0, 33]).unwrap().size_bits, 0);
    }

    /// A header from before those fields existed stops where it stops, rather
    /// than reading the first object as more of itself.
    #[test]
    fn a_header_from_an_older_systemd_stops_early() {
        let mut v = header_bytes(false);
        v.truncate(208);
        v[88..96].copy_from_slice(&208u64.to_le_bytes());
        v.extend_from_slice(&object_bytes(3, 0, &[0; 48]));
        let d = Document::new(MemSource(v));
        let mut e = Evaluator::new(journal());
        assert_eq!(e.node(&d, &[0, 24]).unwrap().size_bits, 0); // n_data
        assert_eq!(e.node(&d, &[1]).unwrap().child_count, 1);
        let kind = e.node(&d, &[1, 0, 0]).unwrap().value;
        assert!(matches!(kind, Value::Enum { raw: 3, .. }), "not an entry: {kind:?}");
    }

    #[test]
    fn a_data_object_reads_the_field_it_holds() {
        let mut v = header_bytes(false);
        v.extend_from_slice(&data_bytes(b"MESSAGE=hello", false));
        let d = Document::new(MemSource(v));
        let mut e = Evaluator::new(journal());
        assert_eq!(e.node(&d, &[1]).unwrap().child_count, 1);
        assert_eq!(
            e.node(&d, &[1, 0, 4, 8]).unwrap().value,
            Value::Str("MESSAGE=hello".into())
        );
        // Thirteen characters of payload, so three bytes of padding.
        assert_eq!(e.node(&d, &[1, 0, 5]).unwrap().size_bits, 3 * 8);
    }

    /// The flag that changed every journal written since 2022: an entry's
    /// items are half the size, so a reader that ignores it finds half as
    /// many and reads each of them from the wrong bytes.
    #[test]
    fn compact_entries_carry_offsets_and_no_hashes() {
        let mut wide = header_bytes(false);
        let mut items = vec![0u8; 48]; // seqnum, timestamps, boot id, xor hash
        items.extend_from_slice(&0x1000u64.to_le_bytes());
        items.extend_from_slice(&0xabcdu64.to_le_bytes());
        wide.extend_from_slice(&object_bytes(3, 0, &items));
        let d = Document::new(MemSource(wide));
        let mut e = Evaluator::new(journal());
        assert_eq!(e.node(&d, &[1, 0, 4, 5]).unwrap().child_count, 1);
        assert_eq!(e.node(&d, &[1, 0, 4, 5, 0, 0]).unwrap().value.as_int(), Some(0x1000));

        let mut tight = header_bytes(true);
        let mut items = vec![0u8; 48];
        items.extend_from_slice(&0x1000u32.to_le_bytes());
        items.extend_from_slice(&0x2000u32.to_le_bytes());
        tight.extend_from_slice(&object_bytes(3, 0, &items));
        let d = Document::new(MemSource(tight));
        let mut e = Evaluator::new(journal());
        assert_eq!(e.node(&d, &[1, 0, 4, 5]).unwrap().child_count, 2);
        assert_eq!(e.node(&d, &[1, 0, 4, 5, 1]).unwrap().value.as_int(), Some(0x2000));
    }

    /// The room a journal was given and has not used yet is one run of zeros,
    /// not an object every sixteen bytes.
    #[test]
    fn the_unwritten_tail_is_one_run() {
        let mut v = header_bytes(false);
        v.extend_from_slice(&data_bytes(b"MESSAGE=hello", false));
        v.resize(v.len() + 4096, 0);
        let d = Document::new(MemSource(v));
        let mut e = Evaluator::new(journal());
        assert_eq!(e.node(&d, &[1]).unwrap().child_count, 2);
        assert_eq!(e.node(&d, &[1, 1, 4]).unwrap().size_bits, (4096 - 16) * 8);
    }
}
