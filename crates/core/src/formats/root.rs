//! CERN ROOT: the file a particle physics experiment writes its events to.
//!
//! A hundred-byte header, and then the whole file is records. Every record
//! opens with a `TKey`: how many bytes it takes up, how many its contents come
//! to once unpacked, when it was written, where it is, and which class wrote
//! it. What follows the key is that class's own bytes, compressed or not.
//!
//! Nothing here is at a fixed place. The header holds three file offsets and
//! each one leads to a record: the top directory at `fBEGIN`, the list of
//! class descriptions at `fSeekInfo`, and the free-space list at `fSeekFree`.
//! The directory holds a fourth, `fSeekKeys`, which reaches the list of every
//! object in the file, and each entry of that list is a key naming where its
//! own record is. So the whole file is walked from thirteen numbers at the
//! front, and every step of it is a `Ty::At`.
//!
//! Everything is big-endian, which is what ROOT calls machine independent.
//!
//! Two version flags change the layout rather than only the meaning. A file
//! whose `fVersion` is over a million was written by the large-file writer and
//! its offsets are 64 bits wide; a key whose own version is over a thousand
//! says the same about its two. Both are read here as a computed one-or-zero
//! that a `Switch` picks the width from.
//!
//! A key whose class is `TDirectory` or `TDirectoryFile` does not point at an
//! object but at another directory, which has a key list of its own. Those are
//! followed, so the file reads as the tree of directories it is, down to a
//! fixed depth: nothing in a ROOT file stops a directory pointing back at one
//! that holds it, and a template has no memory of where it has been.
//!
//! What is left as bytes. A record's contents, once past the compression
//! header, are the streamed form of whatever C++ class wrote them, and reading
//! those needs the class descriptions in `StreamerInfo`, which are themselves
//! written that way. So a `TTree`'s branches and baskets are not taken apart
//! here: the record is placed, named, measured, and what is inside the
//! compressed stream is left whole. The stream itself is not: a `ZL` block
//! holds a zlib stream, an `XZ` block a whole xz stream with its own index and
//! footer, a `ZS` block a zstd frame, and each is read by the template that
//! format already has here. `L4` is the odd one, and is read here rather than
//! borrowed: ROOT writes an eight-byte checksum and then a bare LZ4 block, not
//! an LZ4 frame, so there is no magic number and no frame header to read.

use crate::template::{Endian::*, Expr as E, Template, Ty as T, Until};
use super::{xz, zlib, zstd};

/// The two letters a compressed block opens with, read as one big-endian
/// sixteen-bit number. `CS` is the zlib of ROOT 3 and before, which nothing
/// has written this century but which files still in use were written by.
const ALGORITHM: &[(i128, &str)] = &[
    (0x5a4c, "zlib"),   // ZL
    (0x585a, "xz"),     // XZ
    (0x4c34, "lz4"),    // L4
    (0x5a53, "zstd"),   // ZS
    (0x4353, "old zlib"), // CS
];

/// A file offset, four bytes wide or eight depending on the flag `large`,
/// which the enclosing structure works out from its own version number.
fn seek(large: &str) -> T {
    T::switch(E::field(large), vec![(1, T::Int { bits: 64, endian: Big })], T::i32(Big))
}

/// A count and then that many bytes of it. A string of 255 bytes or more
/// writes 255 as the count and the real length in the four bytes after it,
/// which is why the shape has to be chosen by looking at the first byte
/// before reading it.
fn tstring() -> T {
    let long = T::structure_named(
        "TString",
        "",
        "text",
        vec![("marker", T::u8()), ("len", T::u32(Big)), ("text", T::utf8(E::field("len")))],
    );
    let short =
        T::structure_named("TString", "", "text", vec![("len", T::u8()), ("text", T::utf8(E::field("len")))]);
    T::switch(E::peek(8, Big), vec![(255, long)], short)
}

/// A date and a time packed into thirty-two bits, six of them the year since
/// 1995. There is nothing in the IR that turns that into a date, so the parts
/// are fields of their own and the year is given twice: as written, and as the
/// year it means.
fn datime() -> T {
    T::inline_structure(
        "Datime",
        vec![
            ("year", T::UInt { bits: 6, endian: Big }),
            ("month", T::UInt { bits: 4, endian: Big }),
            ("day", T::UInt { bits: 5, endian: Big }),
            ("hour", T::UInt { bits: 5, endian: Big }),
            ("minute", T::UInt { bits: 6, endian: Big }),
            ("second", T::UInt { bits: 6, endian: Big }),
            ("year_ad", T::computed(E::field("year").add(E::lit(1995)))),
        ],
    )
}

/// The sixteen bytes that name a file or a directory for good, and the version
/// of the class that wrote them.
fn uuid() -> T {
    T::inline_structure("TUUID", vec![("version", T::u16(Big)), ("bytes", T::bytes(E::lit(16)))])
}

/// The key every record opens with. `fNbytes` counts the key itself as well as
/// what follows it, and `fKeylen` says where the split is; `fObjlen` is what
/// the contents come to unpacked, so the two together say whether the record
/// was compressed without anything having to say so outright.
fn key_fields() -> Vec<(&'static str, T)> {
    vec![
        ("fNbytes", T::i32(Big)),
        ("fVersion", T::Int { bits: 16, endian: Big }),
        ("fObjlen", T::i32(Big)),
        ("fDatime", datime()),
        ("fKeylen", T::Int { bits: 16, endian: Big }),
        ("fCycle", T::Int { bits: 16, endian: Big }),
        ("large", T::computed(E::lit(1000).less_than(E::field("fVersion")))),
        ("fSeekKey", seek("large")),
        ("fSeekPdir", seek("large")),
        ("fClassName", tstring()),
        ("fName", tstring()),
        ("fTitle", tstring()),
    ]
}

fn with_key(name: &str, named_by: &str, contents: &str, rest: Vec<(&str, T)>) -> T {
    let mut fields = key_fields();
    fields.extend(rest);
    T::structure_named(name, named_by, contents, fields).machinery(&["large"])
}

/// One compressed block: two letters naming the algorithm, the version of it,
/// and the two sizes, three bytes each and little-endian in a format that is
/// big-endian everywhere else.
///
/// ROOT compresses in blocks of at most sixteen mebibytes unpacked, so a large
/// object is several of these one after another and the record's contents are
/// however many fit.
fn compressed(inner: T) -> T {
    T::structure(
        "Compressed",
        vec![
            ("algorithm", T::enumeration_hex("RootAlgorithm", T::u16(Big), ALGORITHM)),
            ("method", T::u8()),
            ("compressed_size", T::UInt { bits: 24, endian: Little }),
            ("uncompressed_size", T::UInt { bits: 24, endian: Little }),
            ("stream", T::sized(E::field("compressed_size"), stream(inner))),
        ],
    )
    .counted_as("block")
}

/// What is inside one block, by the two letters in front of it. Each of these
/// is a whole stream of its format, so the window the block gives it is the
/// container the format measures itself against: xz reads its footer from the
/// end of that window and finds its index from there.
///
/// `CS` is not among them. The zlib of ROOT 3 wrote raw deflate with no
/// two-byte header on it and no checksum after it, so it is not a zlib stream
/// and there is nothing here that reads one.
fn stream(inner: T) -> T {
    T::switch(
        E::field("algorithm"),
        vec![
            (0x5a4c, zlib::part(inner.clone()).root),
            (0x585a, xz::part(inner.clone()).root),
            (0x5a53, zstd::part(inner.clone()).root),
            (0x4c34, lz4_block(inner)),
        ],
        T::bytes(E::Remaining),
    )
}

/// The lz4 of a ROOT block, which is not an LZ4 frame. ROOT writes the
/// xxhash-64 of the compressed bytes itself and then hands LZ4 a single block
/// with no frame header and no length in front of it, so the length is the
/// block header's `compressed_size` less the eight bytes of the checksum.
fn lz4_block(inner: T) -> T {
    T::structure_named(
        "RootLz4",
        "",
        "block",
        vec![
            ("xxhash64", T::u64(Big)),
            ("block", T::decoded(E::Remaining, crate::codec::Codec::Lz4Block, inner)),
        ],
    )
}

/// What follows a key: `fNbytes` less `fKeylen` bytes of it. When that is
/// short of `fObjlen` the record was compressed and opens with a block header;
/// when the two agree the bytes are the object as it stands.
fn body() -> T {
    let size = E::field("fNbytes").sub(E::field("fKeylen"));
    let packed = size.clone().less_than(E::field("fObjlen"));
    T::sized(
        size,
        T::switch(packed, vec![(1, T::repeat(T::Named("Compressed".into()), Until::End))], T::bytes(E::Remaining)),
    )
}

/// Any record reached by a key: the key, and the bytes it covers.
fn record() -> T {
    with_key("Record", "fName", "body", vec![("body", body())])
}

/// The record at `fBEGIN`. Its contents are the file's own name and title
/// again, and then the top directory.
fn file_record() -> T {
    with_key(
        "FileRecord",
        "fName",
        "",
        vec![("name", tstring()), ("title", tstring()), ("directory", T::Named(named("Directory", 0)))],
    )
}

/// How many directories deep the walk goes. Nothing in the format stops a
/// directory naming one that holds it, and a template following names has no
/// way to notice it has been here before, so the tree is cut off at a depth no
/// real file reaches. Below it a directory key is placed as a plain record and
/// its own keys are not walked.
const MAX_DEPTH: usize = 6;

/// A type name for one level of the directory tree. The structures keep their
/// own names, so a reader sees `Directory` however deep it is; only the table
/// the template looks names up in tells the levels apart.
fn named(base: &str, level: usize) -> std::sync::Arc<str> {
    format!("{base}@{level}").into()
}

/// The record a `TDirectory` key points at: a key, and the directory itself.
/// It is not a `Record`, whose contents are the streamed object; a directory
/// record holds the sixty bytes below and nothing else, and is never
/// compressed.
fn dir_record(level: usize) -> T {
    with_key("DirRecord", "fName", "directory", vec![("directory", T::Named(named("Directory", level)))])
}

/// A directory: when it was made and when it was last written, how big its key
/// list is, and where it, its parent and that list are.
///
/// The twelve bytes at the end are ROOT keeping room. A directory written with
/// 32-bit offsets leaves the space three 64-bit ones would have taken, so that
/// a file can grow past two gigabytes without the directory having to move.
fn directory(level: usize) -> T {
    T::structure(
        "Directory",
        vec![
            ("fVersion", T::Int { bits: 16, endian: Big }),
            ("fDatimeC", datime()),
            ("fDatimeM", datime()),
            ("fNbytesKeys", T::i32(Big)),
            ("fNbytesName", T::i32(Big)),
            ("large", T::computed(E::lit(1000).less_than(E::field("fVersion")))),
            ("fSeekDir", seek("large")),
            ("fSeekParent", seek("large")),
            ("fSeekKeys", seek("large")),
            ("fUUID", uuid()),
            ("reserved", T::bytes(E::lit(12).mul(E::lit(1).sub(E::field("large"))))),
            ("keys", at_if_set("fSeekKeys", T::Named(named("KeysList", level)))),
        ],
    )
    .machinery(&["large", "reserved"])
}

/// `inner` at the offset the field `name` holds, and nothing at all when that
/// offset is zero. A file with no streamer information writes zero there, and
/// following it would read the file's own magic as a key.
fn at_if_set(name: &str, inner: T) -> T {
    T::switch(
        E::lit(0).less_than(E::field(name)),
        vec![(1, T::at(E::field(name), inner))],
        T::bytes(E::lit(0)),
    )
}

/// The key list: a record whose contents are a count and then one key per
/// object in the directory. These keys are copies of the ones in front of the
/// records themselves, which is what makes a directory readable without
/// walking the whole file.
fn keys_list(level: usize) -> T {
    with_key(
        "KeysList",
        "",
        "keys",
        vec![
            ("nkeys", T::i32(Big)),
            ("keys", T::array(T::Named(named("KeyEntry", level)), E::field("nkeys"))),
        ],
    )
}

/// One entry of the key list, and whatever it names. The entry says both what
/// the object is called and which class wrote it, so the list reads the way
/// `ls` does in ROOT: `Events` is a `TTree`, `histograms` is a
/// `TDirectoryFile`, `ntuple` is a `ROOT::RNTuple`.
fn key_entry(level: usize) -> T {
    with_key("KeyEntry", "fName", "record", vec![("record", by_class(level))]).counted_as("key")
}

/// What a key points at, chosen by the class name written in the key itself.
///
/// Three classes are records this template can go further into. A directory is
/// another key list, and is followed until the walk runs out of depth. An
/// RNTuple key holds the anchor, which is thirteen numbers saying where the
/// rest of that format lives. Everything else is a record whose contents are
/// the streamed object, and those are left alone.
fn by_class(level: usize) -> T {
    let deeper = match level < MAX_DEPTH {
        true => at_if_set("fSeekKey", T::Named(named("DirRecord", level + 1))),
        false => at_if_set("fSeekKey", T::Named("Record".into())),
    };
    let anchor = at_if_set("fSeekKey", T::Named("RNTupleRecord".into()));
    T::matches(
        E::within(&["fClassName", "text"]),
        vec![
            ("TDirectory", deeper.clone()),
            ("TDirectoryFile", deeper),
            ("ROOT::RNTuple", anchor.clone()),
            // What the class was called while the format was being settled.
            ("ROOT::Experimental::RNTuple", anchor),
        ],
        at_if_set("fSeekKey", T::Named("Record".into())),
    )
}

/// The record an `ROOT::RNTuple` key points at: the anchor of the new ROOT
/// format, which is a TFile key and nothing else of a TFile. Everything an
/// RNTuple is made of sits outside the directory structure, and the anchor is
/// what says where.
fn rntuple_record() -> T {
    let size = E::field("fNbytes").sub(E::field("fKeylen"));
    let packed = size.clone().less_than(E::field("fObjlen"));
    with_key(
        "RNTupleRecord",
        "fName",
        "anchor",
        vec![(
            "anchor",
            T::sized(
                size,
                T::switch(
                    packed,
                    vec![(1, T::repeat(compressed(anchor()), Until::End))],
                    anchor(),
                ),
            ),
        )],
    )
}

/// The anchor: which version of the format wrote the ntuple, and where its two
/// envelopes are.
///
/// It is written as a streamed object, so it opens the way one does: a byte
/// count with a bit set to say it is one, and the version of the class. What
/// follows is thirteen numbers and nothing that needs the streamer
/// information to read.
///
/// The two envelopes hold everything else: the header describes the fields and
/// columns, the footer lists the clusters and where their pages are. Neither
/// is taken apart here.
fn anchor() -> T {
    T::structure(
        "RNTupleAnchor",
        vec![
            // The high bit is a marker rather than a size: it says the four
            // bytes are a byte count at all.
            ("byte_count_raw", T::u32(Big)),
            ("byte_count", T::computed(E::field("byte_count_raw").sub(E::field("byte_count_raw").bit(30).mul(E::lit(0x4000_0000))))),
            ("class_version", T::u16(Big)),
            // The format's own version, which is not the class's: an epoch
            // that has only ever been zero, and then the three numbers a
            // release of the format is named by.
            ("version_epoch", T::u16(Big)),
            ("version_major", T::u16(Big)),
            ("version_minor", T::u16(Big)),
            ("version_patch", T::u16(Big)),
            ("seek_header", T::u64(Big)),
            ("nbytes_header", T::u64(Big)),
            ("len_header", T::u64(Big)),
            ("seek_footer", T::u64(Big)),
            ("nbytes_footer", T::u64(Big)),
            ("len_footer", T::u64(Big)),
            // The largest key the writer would write. An envelope longer than
            // this is split across several keys, which is not read here.
            ("max_key_size", T::u64(Big)),
            // xxhash-3 of everything above it.
            ("checksum", T::u64(Big)),
            ("header", envelope("seek_header", "nbytes_header", "len_header")),
            ("footer", envelope("seek_footer", "nbytes_footer", "len_footer")),
        ],
    )
    .machinery(&["byte_count_raw", "class_version"])
}

/// An envelope where the anchor says it is: `nbytes` bytes of it, holding
/// `len` once unpacked. It is not behind a key of its own, so what is at the
/// offset is the bytes themselves, compressed the same nine-byte way a record
/// is when the two lengths disagree.
fn envelope(seek: &str, nbytes: &str, len: &str) -> T {
    let inner = T::sized(
        E::field(nbytes),
        T::switch(
            E::field(nbytes).less_than(E::field(len)),
            vec![(1, T::repeat(T::Named("Compressed".into()), Until::End))],
            T::bytes(E::Remaining),
        ),
    );
    T::switch(
        E::lit(0).less_than(E::field(seek)),
        vec![(1, T::at(E::field(seek), inner))],
        T::bytes(E::lit(0)),
    )
}

/// The free-space list: a record whose contents are the stretches of the file
/// nothing is using. The last one runs to 2,000,000,000, which is how ROOT
/// writes "and everything after this".
fn free_record() -> T {
    with_key(
        "FreeRecord",
        "",
        "free",
        vec![(
            "free",
            T::sized(
                E::field("fNbytes").sub(E::field("fKeylen")),
                T::repeat(T::Named("Free".into()), Until::End),
            ),
        )],
    )
}

fn free() -> T {
    T::structure(
        "Free",
        vec![
            ("fVersion", T::Int { bits: 16, endian: Big }),
            ("large", T::computed(E::lit(1000).less_than(E::field("fVersion")))),
            ("fFirst", seek("large")),
            ("fLast", seek("large")),
        ],
    )
    .machinery(&["large"])
}

pub fn root() -> Template {
    let header = T::structure(
        "TFile",
        vec![
            ("magic", T::magic(b"root")),
            ("fVersion", T::i32(Big)),
            ("fBEGIN", T::i32(Big)),
            // Over a million means the large-file writer, and every offset in
            // the header is eight bytes rather than four. The version under
            // the million is the ROOT release, so 61005 is 6.10.05.
            ("large", T::computed(E::lit(1_000_000).less_than(E::field("fVersion")))),
            ("fEND", seek("large")),
            ("fSeekFree", seek("large")),
            ("fNbytesFree", T::i32(Big)),
            ("nfree", T::i32(Big)),
            ("fNbytesName", T::i32(Big)),
            ("fUnits", T::u8()),
            // The algorithm and the level in one number: 404 is lz4 at 4.
            ("fCompress", T::i32(Big)),
            ("compression_algorithm", T::computed(E::field("fCompress").div(E::lit(100)))),
            (
                "compression_level",
                T::computed(E::field("fCompress").sub(E::field("fCompress").div(E::lit(100)).mul(E::lit(100)))),
            ),
            ("fSeekInfo", seek("large")),
            ("fNbytesInfo", T::i32(Big)),
            ("fUUID", uuid()),
            // The header stops at 63 bytes and `fBEGIN` is 100 in every file
            // anyone has written, so what is between them reads as a gap. The
            // three fields below take up no room where they stand: each one
            // places a record somewhere else in the file.
            ("directory", T::at(E::field("fBEGIN"), T::Named("FileRecord".into()))),
            ("streamer_info", at_if_set("fSeekInfo", T::Named("Record".into()))),
            ("free_list", at_if_set("fSeekFree", T::Named("FreeRecord".into()))),
        ],
    )
    .machinery(&["large", "fUnits"]);

    let mut t = Template::new("root", header)
        .with_type("FileRecord", file_record())
        .with_type("Record", record())
        .with_type("RNTupleRecord", rntuple_record())
        .with_type("FreeRecord", free_record())
        .with_type("Free", free())
        .with_type("Compressed", compressed(super::decoded_object()));
    // One set of directory types per level of the tree, so that a template
    // made of names can be walked into a fixed number of times. See
    // [`MAX_DEPTH`].
    for level in 0..=MAX_DEPTH {
        t = t
            .with_type(&format!("Directory@{level}"), directory(level))
            .with_type(&format!("KeysList@{level}"), keys_list(level))
            .with_type(&format!("KeyEntry@{level}"), key_entry(level));
        if level > 0 {
            t = t.with_type(&format!("DirRecord@{level}"), dir_record(level));
        }
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    fn be32(v: i32) -> Vec<u8> {
        v.to_be_bytes().to_vec()
    }

    fn rstr(s: &str) -> Vec<u8> {
        let mut v = vec![s.len() as u8];
        v.extend_from_slice(s.as_bytes());
        v
    }

    /// 2017-06-08 06:58:24, the date the real samples carry, packed the way
    /// ROOT packs one.
    const DATIME: u32 = ((2017 - 1995) << 26) | (6 << 22) | (8 << 17) | (6 << 12) | (58 << 6) | 24;

    fn keylen(cls: &str, name: &str, title: &str) -> i32 {
        (26 + cls.len() + name.len() + title.len() + 3) as i32
    }

    /// A key header. `objlen` is what the contents come to unpacked and
    /// `bodylen` how many bytes they take up here, which is how a record says
    /// it was compressed.
    fn key(cls: &str, name: &str, title: &str, objlen: i32, bodylen: i32, seekkey: i32, seekpdir: i32) -> Vec<u8> {
        let kl = keylen(cls, name, title);
        let mut b = be32(kl + bodylen);
        b.extend_from_slice(&4i16.to_be_bytes());
        b.extend(be32(objlen));
        b.extend_from_slice(&DATIME.to_be_bytes());
        b.extend_from_slice(&(kl as i16).to_be_bytes());
        b.extend_from_slice(&1i16.to_be_bytes());
        b.extend(be32(seekkey));
        b.extend(be32(seekpdir));
        b.extend(rstr(cls));
        b.extend(rstr(name));
        b.extend(rstr(title));
        assert_eq!(b.len() as i32, kl);
        b
    }

    /// The sixty bytes of a directory, written with 32-bit offsets.
    fn dir_bytes(nbytes_keys: i32, nbytes_name: i32, seekdir: i32, seekparent: i32, seekkeys: i32) -> Vec<u8> {
        let mut b = 5i16.to_be_bytes().to_vec();
        b.extend_from_slice(&DATIME.to_be_bytes());
        b.extend_from_slice(&DATIME.to_be_bytes());
        b.extend(be32(nbytes_keys));
        b.extend(be32(nbytes_name));
        b.extend(be32(seekdir));
        b.extend(be32(seekparent));
        b.extend(be32(seekkeys));
        b.extend_from_slice(&1u16.to_be_bytes());
        b.extend_from_slice(&[0xcd; 16]);
        b.extend_from_slice(&[0; 12]);
        assert_eq!(b.len(), 60);
        b
    }

    /// The first hundred bytes: the header, with no streamer list and no free
    /// list, and everything padded out to where `fBEGIN` says the records
    /// start.
    fn header_bytes(end: i32) -> Vec<u8> {
        let mut b = b"root".to_vec();
        b.extend(be32(61005));
        b.extend(be32(100));
        b.extend(be32(end));
        b.extend(be32(0)); // fSeekFree
        b.extend(be32(0));
        b.extend(be32(0));
        b.extend(be32(keylen("TFile", "t.root", "") + 7));
        b.push(4);
        b.extend(be32(101));
        b.extend(be32(0)); // fSeekInfo
        b.extend(be32(0));
        b.extend_from_slice(&1u16.to_be_bytes());
        b.extend_from_slice(&[0xab; 16]);
        b.resize(100, 0);
        b
    }

    /// A whole small file, laid out the way the real ones are: the header, the
    /// top directory's record, one compressed data record, the key list, the
    /// streamer information, and the free list.
    fn file() -> Vec<u8> {
        const BEGIN: i32 = 100;
        // The directory's record: name, title, and the sixty-byte directory.
        let dir_body = rstr("t.root").len() as i32 + rstr("").len() as i32 + 60;
        let file_kl = keylen("TFile", "t.root", "");
        let data_at = BEGIN + file_kl + dir_body;
        // The data record: a nine-byte block header and eight bytes of zlib
        // stream, standing for twenty bytes unpacked.
        let data_kl = keylen("TTree", "tree", "");
        let keys_at = data_at + data_kl + 17;
        let keys_body = 4 + data_kl;
        let keys_kl = keylen("TFile", "t.root", "");
        let info_at = keys_at + keys_kl + keys_body;
        let info_kl = keylen("TList", "StreamerInfo", "");
        let free_at = info_at + info_kl + 8;
        let free_kl = keylen("TFile", "t.root", "");
        let end = free_at + free_kl + 10;

        let mut b = b"root".to_vec();
        b.extend(be32(61005));
        b.extend(be32(BEGIN));
        b.extend(be32(end));
        b.extend(be32(free_at));
        b.extend(be32(free_kl + 10));
        b.extend(be32(1)); // nfree
        b.extend(be32(file_kl + 7)); // fNbytesName
        b.push(4); // fUnits
        b.extend(be32(101)); // zlib, level 1
        b.extend(be32(info_at));
        b.extend(be32(info_kl + 8));
        b.extend_from_slice(&1u16.to_be_bytes());
        b.extend_from_slice(&[0xab; 16]);
        b.resize(BEGIN as usize, 0);

        b.extend(key("TFile", "t.root", "", dir_body, dir_body, BEGIN, 0));
        b.extend(rstr("t.root"));
        b.extend(rstr(""));
        // The directory itself.
        b.extend_from_slice(&5i16.to_be_bytes());
        b.extend_from_slice(&DATIME.to_be_bytes());
        b.extend_from_slice(&DATIME.to_be_bytes());
        b.extend(be32(keys_kl + keys_body));
        b.extend(be32(file_kl + 7));
        b.extend(be32(BEGIN));
        b.extend(be32(0));
        b.extend(be32(keys_at));
        b.extend_from_slice(&1u16.to_be_bytes());
        b.extend_from_slice(&[0xcd; 16]);
        b.extend_from_slice(&[0; 12]);
        assert_eq!(b.len() as i32, data_at);

        b.extend(key("TTree", "tree", "", 20, 17, data_at, BEGIN));
        b.extend_from_slice(b"ZL");
        b.push(8);
        b.extend_from_slice(&[8, 0, 0]); // eight bytes of stream
        b.extend_from_slice(&[20, 0, 0]); // twenty unpacked
        // A zlib stream: the two header bytes, an empty deflate block, and an
        // Adler-32 of 1, which is what the checksum of nothing comes to.
        b.extend_from_slice(&[0x78, 0x9c, 0x03, 0x00, 0, 0, 0, 1]);
        assert_eq!(b.len() as i32, keys_at);

        b.extend(key("TFile", "t.root", "", keys_body, keys_body, keys_at, BEGIN));
        b.extend(be32(1)); // one key in the list
        b.extend(key("TTree", "tree", "", 20, 17, data_at, BEGIN));
        assert_eq!(b.len() as i32, info_at);

        b.extend(key("TList", "StreamerInfo", "", 8, 8, info_at, BEGIN));
        b.extend_from_slice(&[0; 8]);
        assert_eq!(b.len() as i32, free_at);

        b.extend(key("TFile", "t.root", "", 10, 10, free_at, BEGIN));
        b.extend_from_slice(&1i16.to_be_bytes());
        b.extend(be32(end));
        b.extend(be32(2_000_000_000));
        assert_eq!(b.len() as i32, end);
        b
    }

    /// Where the header's fields sit, so the paths below read as something. A
    /// field that places a record elsewhere takes up no bytes and has the
    /// record as its one child, which is the extra `0` in every path here.
    const F_VERSION: usize = 1;
    const F_UUID: usize = 15;
    const DIRECTORY: [usize; 2] = [16, 0];
    const STREAMER: [usize; 2] = [17, 0];
    const FREE: [usize; 2] = [18, 0];
    /// A record's own fields: twelve of key, and then whatever it holds.
    const K_FIELDS: usize = 12;

    /// `path` and then the indices after it, since a path here is built up a
    /// record at a time.
    fn down(path: &[usize], more: &[usize]) -> Vec<usize> {
        [path, more].concat()
    }

    #[test]
    fn the_header_says_which_release_wrote_it_and_where_everything_is() {
        let d = Document::new(MemSource(file()));
        let mut ev = Evaluator::new(root());
        assert_eq!(ev.node(&d, &[0]).unwrap().value, Value::Magic { ok: true, bytes: b"root".to_vec(), expected: b"root".to_vec() });
        assert_eq!(ev.node(&d, &[F_VERSION]).unwrap().value, Value::Int(61005));
        // 32-bit offsets, so the header is 63 bytes and the UUID ends it.
        let u = ev.node(&d, &[F_UUID]).unwrap();
        assert_eq!(u.offset_bits + u.size_bits, 63 * 8);
        // 101 is zlib at level one.
        assert_eq!(ev.node(&d, &[11]).unwrap().value, Value::Int(1));
        assert_eq!(ev.node(&d, &[12]).unwrap().value, Value::Int(1));
    }

    #[test]
    fn the_top_directory_is_the_record_at_the_offset_the_header_gives() {
        let d = Document::new(MemSource(file()));
        let mut ev = Evaluator::new(root());
        let rec = ev.node(&d, &DIRECTORY).unwrap();
        assert_eq!(rec.offset_bits, 100 * 8);
        assert_eq!(rec.name, "directory t.root");
        // fClassName, and the directory that follows the file's name.
        assert_eq!(ev.node(&d, &down(&DIRECTORY, &[9, 1])).unwrap().value, Value::Str("TFile".into()));
        let dir = ev.node(&d, &down(&DIRECTORY, &[K_FIELDS + 2])).unwrap();
        assert_eq!(dir.size_bits, 60 * 8);
    }

    #[test]
    fn a_date_is_the_year_since_1995_and_five_fields_after_it() {
        let d = Document::new(MemSource(file()));
        let mut ev = Evaluator::new(root());
        // fDatime is the fourth field of the key, and its parts are bit wide.
        let mut at = |i: usize| ev.node(&d, &down(&DIRECTORY, &[3, i])).unwrap().value;
        assert_eq!(at(0), Value::UInt(22));
        assert_eq!(at(1), Value::UInt(6));
        assert_eq!(at(2), Value::UInt(8));
        assert_eq!(at(6), Value::Int(2017));
    }

    #[test]
    fn the_key_list_reaches_the_record_each_key_names() {
        let d = Document::new(MemSource(file()));
        let mut ev = Evaluator::new(root());
        // The record, its directory, the key list that directory points at,
        // and the list of keys inside it.
        let keys = down(&DIRECTORY, &[K_FIELDS + 2, 11, 0, K_FIELDS + 1]);
        let list = ev.node(&d, &keys).unwrap();
        assert_eq!(list.child_count, 1);
        let entry = down(&keys, &[0]);
        assert_eq!(ev.node(&d, &entry).unwrap().name, "[0] tree");
        // The record the entry points at, and the block its contents open
        // with.
        let block = down(&entry, &[K_FIELDS, 0, K_FIELDS, 0]);
        let algorithm = ev.node(&d, &down(&block, &[0])).unwrap().value;
        assert_eq!(algorithm, Value::Enum { raw: 0x5a4c, name: Some("zlib".into()), hex: true });
        // Both sizes are three bytes and little-endian, which nothing else in
        // this file is.
        assert_eq!(ev.node(&d, &down(&block, &[2])).unwrap().value, Value::UInt(8));
        assert_eq!(ev.node(&d, &down(&block, &[3])).unwrap().value, Value::UInt(20));
    }

    #[test]
    fn a_zl_block_holds_a_zlib_stream_read_by_the_zlib_template() {
        let d = Document::new(MemSource(file()));
        let mut ev = Evaluator::new(root());
        let keys = down(&DIRECTORY, &[K_FIELDS + 2, 11, 0, K_FIELDS + 1]);
        let block = down(&keys, &[0, K_FIELDS, 0, K_FIELDS, 0]);
        // The stream is the block's fifth field, and inside it is a zlib
        // stream: window, method, and the checksum at the end of it.
        let stream = down(&block, &[4]);
        assert_eq!(ev.node(&d, &stream).unwrap().size_bits, 8 * 8);
        assert_eq!(ev.node(&d, &down(&stream, &[0])).unwrap().value.as_int(), Some(7));
        assert_eq!(
            ev.node(&d, &down(&stream, &[1])).unwrap().value,
            Value::Enum { raw: 8, name: Some("deflate".into()), hex: false }
        );
        // Two bytes of deflate between the header and the four of checksum.
        assert_eq!(ev.node(&d, &down(&stream, &[6])).unwrap().size_bits, 2 * 8);
        assert_eq!(ev.node(&d, &down(&stream, &[7])).unwrap().value.as_int(), Some(1));
    }

    #[test]
    fn an_l4_block_is_a_checksum_and_a_raw_block_rather_than_an_lz4_frame() {
        // The same file with the block header rewritten as lz4: eight bytes
        // of xxhash-64 and then the block itself, with no frame magic.
        let mut bytes = file();
        let at = bytes.windows(2).position(|w| w == b"ZL").unwrap();
        bytes[at..at + 2].copy_from_slice(b"L4");
        let d = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(root());
        let keys = down(&DIRECTORY, &[K_FIELDS + 2, 11, 0, K_FIELDS + 1]);
        let stream = down(&keys, &[0, K_FIELDS, 0, K_FIELDS, 0, 4]);
        assert_eq!(
            ev.node(&d, &down(&stream, &[0])).unwrap().value,
            Value::UInt(0x789c_0300_0000_0001)
        );
        // Eight of the block's bytes went to the checksum, so nothing is left.
        assert_eq!(ev.node(&d, &down(&stream, &[1])).unwrap().size_bits, 0);
    }

    /// A file whose top directory holds one subdirectory, which holds one
    /// tree. Nothing else: no streamer list and no free list, so what the
    /// walk finds is the tree of directories and nothing beside it.
    fn nested() -> Vec<u8> {
        const BEGIN: i32 = 100;
        let file_kl = keylen("TFile", "t.root", "");
        let dir_body = rstr("t.root").len() as i32 + rstr("").len() as i32 + 60;
        let top_keys_at = BEGIN + file_kl + dir_body;
        let sub_key = keylen("TDirectoryFile", "sub", "");
        let top_keys_body = 4 + sub_key;
        let sub_at = top_keys_at + file_kl + top_keys_body;
        let sub_keys_at = sub_at + sub_key + 60;
        let leaf_key = keylen("TTree", "leaf", "");
        let sub_keys_body = 4 + leaf_key;
        let leaf_at = sub_keys_at + sub_key + sub_keys_body;
        let end = leaf_at + leaf_key + 8;

        let mut b = header_bytes(end);
        b.extend(key("TFile", "t.root", "", dir_body, dir_body, BEGIN, 0));
        b.extend(rstr("t.root"));
        b.extend(rstr(""));
        b.extend(dir_bytes(file_kl + top_keys_body, file_kl + 7, BEGIN, 0, top_keys_at));
        assert_eq!(b.len() as i32, top_keys_at);

        // The top directory's key list: one key, and it is a directory.
        b.extend(key("TFile", "t.root", "", top_keys_body, top_keys_body, top_keys_at, BEGIN));
        b.extend(be32(1));
        b.extend(key("TDirectoryFile", "sub", "", 60, 60, sub_at, BEGIN));
        assert_eq!(b.len() as i32, sub_at);

        // The subdirectory's own record: a key, and sixty bytes of directory.
        b.extend(key("TDirectoryFile", "sub", "", 60, 60, sub_at, BEGIN));
        b.extend(dir_bytes(sub_key + sub_keys_body, sub_key, sub_at, BEGIN, sub_keys_at));
        assert_eq!(b.len() as i32, sub_keys_at);

        b.extend(key("TDirectoryFile", "sub", "", sub_keys_body, sub_keys_body, sub_keys_at, sub_at));
        b.extend(be32(1));
        b.extend(key("TTree", "leaf", "", 8, 8, leaf_at, sub_at));
        assert_eq!(b.len() as i32, leaf_at);

        b.extend(key("TTree", "leaf", "", 8, 8, leaf_at, sub_at));
        b.extend_from_slice(&[0; 8]);
        assert_eq!(b.len() as i32, end);
        b
    }

    #[test]
    fn a_directory_key_is_walked_into_and_its_own_keys_read() {
        let d = Document::new(MemSource(nested()));
        let mut ev = Evaluator::new(root());
        let top = down(&DIRECTORY, &[K_FIELDS + 2, 11, 0, K_FIELDS + 1]);
        assert_eq!(ev.node(&d, &top).unwrap().child_count, 1);
        let entry = down(&top, &[0]);
        assert_eq!(ev.node(&d, &entry).unwrap().name, "[0] sub");
        assert_eq!(ev.node(&d, &down(&entry, &[9, 1])).unwrap().value, Value::Str("TDirectoryFile".into()));
        // The key points at a directory record, not at a plain one: past the
        // key are sixty bytes of directory rather than a body of bytes.
        let dir = down(&entry, &[K_FIELDS, 0, K_FIELDS]);
        assert_eq!(ev.node(&d, &dir).unwrap().size_bits, 60 * 8);
        // And that directory has a key list of its own, naming the tree.
        let inner = down(&dir, &[11, 0, K_FIELDS + 1]);
        assert_eq!(ev.node(&d, &inner).unwrap().child_count, 1);
        assert_eq!(ev.node(&d, &down(&inner, &[0])).unwrap().name, "[0] leaf");
        assert_eq!(ev.node(&d, &down(&inner, &[0, 9, 1])).unwrap().value, Value::Str("TTree".into()));
    }

    /// A record compressed with `ZL` holds its object on the other side of the
    /// block header, at offsets of the decoded bytes rather than of the file.
    #[test]
    fn a_compressed_record_holds_its_object_in_a_space_of_its_own() {
        let d = Document::new(MemSource(file()));
        let mut ev = Evaluator::new(root());
        let keys = down(&DIRECTORY, &[K_FIELDS + 2, 11, 0, K_FIELDS + 1]);
        let block = down(&keys, &[0, K_FIELDS, 0, K_FIELDS, 0]);
        // The zlib stream inside the block, and the deflate run inside that.
        let run = down(&block, &[4, 6]);
        let info = ev.node(&d, &run).unwrap();
        assert_eq!(info.type_name, "deflate");
        assert_eq!(info.space, 0);
        assert_eq!(info.size_bits, 2 * 8);
        assert_eq!(info.child_count, 2);
        assert_eq!(info.refused, None);

        // What came out of it: the object, counted from the front of the
        // decoded bytes. This stream is an empty deflate block, so there is
        // nothing in it and the field sits where the object would start.
        let object = down(&run, &[0, 0]);
        let held = ev.node(&d, &object).unwrap();
        assert_eq!(held.name, "object");
        assert_eq!((held.offset_bits, held.space), (0, 1));
        assert!(!held.editable);

        // The cursor never lands inside the stream. Whatever it names for a
        // byte of the run, it is a field of the file: this file reaches its
        // records by address, and what it answers here is whatever the index
        // of placements has got to.
        let at = ev.node(&d, &run).unwrap().offset_bits;
        for bit in [at, at + 8] {
            let found = ev.locate(&d, bit).unwrap();
            assert!(!found.starts_with(&run) || found == run, "landed inside the stream: {found:?}");
        }
    }

    /// A file whose one key is an RNTuple anchor, and the two envelopes that
    /// anchor points at: a compressed header and an uncompressed footer.
    fn with_rntuple() -> Vec<u8> {
        const BEGIN: i32 = 100;
        let file_kl = keylen("TFile", "t.root", "");
        let dir_body = rstr("t.root").len() as i32 + rstr("").len() as i32 + 60;
        let keys_at = BEGIN + file_kl + dir_body;
        let anchor_key = keylen("ROOT::RNTuple", "nt", "");
        let keys_body = 4 + anchor_key;
        let anchor_at = keys_at + file_kl + keys_body;
        let header_at = anchor_at + anchor_key + 78;
        let footer_at = header_at + 17;
        let end = footer_at + 6;

        let mut b = header_bytes(end);
        b.extend(key("TFile", "t.root", "", dir_body, dir_body, BEGIN, 0));
        b.extend(rstr("t.root"));
        b.extend(rstr(""));
        b.extend(dir_bytes(file_kl + keys_body, file_kl + 7, BEGIN, 0, keys_at));
        b.extend(key("TFile", "t.root", "", keys_body, keys_body, keys_at, BEGIN));
        b.extend(be32(1));
        b.extend(key("ROOT::RNTuple", "nt", "", 78, 78, anchor_at, BEGIN));
        assert_eq!(b.len() as i32, anchor_at);

        b.extend(key("ROOT::RNTuple", "nt", "", 78, 78, anchor_at, BEGIN));
        let start = b.len();
        b.extend(be32(0x4000_0042u32 as i32));
        b.extend_from_slice(&2u16.to_be_bytes()); // class version
        for v in [1u16, 0, 0, 0] {
            b.extend_from_slice(&v.to_be_bytes());
        }
        // The header is compressed, the footer is not, which is what the two
        // lengths say and nothing else does.
        for v in [header_at as u64, 17, 20, footer_at as u64, 6, 6, 0x4000_0000, 0x0123_4567_89ab_cdef] {
            b.extend_from_slice(&v.to_be_bytes());
        }
        assert_eq!(b.len() - start, 78);

        b.extend_from_slice(b"ZL");
        b.push(8);
        b.extend_from_slice(&[8, 0, 0]);
        b.extend_from_slice(&[20, 0, 0]);
        b.extend_from_slice(&[0x78, 0x9c, 0x03, 0x00, 0, 0, 0, 1]);
        assert_eq!(b.len() as i32, footer_at);
        b.extend_from_slice(&[0xee; 6]);
        assert_eq!(b.len() as i32, end);
        b
    }

    #[test]
    fn an_rntuple_anchor_says_where_the_two_envelopes_are() {
        let d = Document::new(MemSource(with_rntuple()));
        let mut ev = Evaluator::new(root());
        let keys = down(&DIRECTORY, &[K_FIELDS + 2, 11, 0, K_FIELDS + 1]);
        let entry = down(&keys, &[0]);
        assert_eq!(ev.node(&d, &down(&entry, &[9, 1])).unwrap().value, Value::Str("ROOT::RNTuple".into()));
        // The record the key points at, and the anchor inside it: seventy-
        // eight bytes of numbers rather than a streamed object left as bytes.
        let anchor = down(&entry, &[K_FIELDS, 0, K_FIELDS]);
        assert_eq!(ev.node(&d, &anchor).unwrap().size_bits, 78 * 8);
        // The byte count with its marker bit taken back off.
        assert_eq!(ev.node(&d, &down(&anchor, &[1])).unwrap().value, Value::Int(0x42));
        assert_eq!(ev.node(&d, &down(&anchor, &[3])).unwrap().value, Value::UInt(1));
        assert_eq!(ev.node(&d, &down(&anchor, &[13])).unwrap().value, Value::UInt(0x4000_0000));

        // The header envelope is where the anchor says, is as long as it
        // says, and opens with a block header because the two lengths differ.
        let header = down(&anchor, &[15, 0]);
        let seek = ev.node(&d, &down(&anchor, &[7])).unwrap().value.as_int().unwrap();
        let h = ev.node(&d, &header).unwrap();
        assert_eq!(h.offset_bits / 8, seek as u64);
        assert_eq!(h.size_bits, 17 * 8);
        assert_eq!(
            ev.node(&d, &down(&header, &[0, 0])).unwrap().value,
            Value::Enum { raw: 0x5a4c, name: Some("zlib".into()), hex: true }
        );
        // The footer's lengths agree, so it is six bytes as they stand.
        let footer = down(&anchor, &[16, 0]);
        assert_eq!(ev.node(&d, &footer).unwrap().size_bits, 6 * 8);
        assert_eq!(ev.node(&d, &footer).unwrap().child_count, 0);
    }

    #[test]
    fn the_streamer_information_and_the_free_list_are_records_of_their_own() {
        let d = Document::new(MemSource(file()));
        let mut ev = Evaluator::new(root());
        assert_eq!(ev.node(&d, &down(&STREAMER, &[9, 1])).unwrap().value, Value::Str("TList".into()));
        assert_eq!(ev.node(&d, &down(&STREAMER, &[10, 1])).unwrap().value, Value::Str("StreamerInfo".into()));
        // Its contents are as long unpacked as they are here, so they are not
        // a compressed block but the object as it stands.
        let body = ev.node(&d, &down(&STREAMER, &[K_FIELDS])).unwrap();
        assert_eq!(body.value, Value::Bytes { len: 8, preview: vec![0; 8] });
        // One free span, running to where ROOT says the file may grow to.
        let free = ev.node(&d, &down(&FREE, &[K_FIELDS])).unwrap();
        assert_eq!(free.child_count, 1);
        assert_eq!(ev.node(&d, &down(&FREE, &[K_FIELDS, 0, 3])).unwrap().value, Value::Int(2_000_000_000));
    }
}
