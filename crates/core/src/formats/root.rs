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
//! What is left as bytes. A record's contents, once past the compression
//! header, are the streamed form of whatever C++ class wrote them, and reading
//! those needs the class descriptions in `StreamerInfo`, which are themselves
//! written that way. So a `TTree`'s branches and baskets are not taken apart
//! here: the record is placed, named, measured, and its compressed stream is
//! left whole. There is no zlib, xz, lz4 or zstd template in this tree to hand
//! the stream on to either, so the block header names the algorithm and says
//! both sizes, and the bytes after it stay a run of bytes.

use crate::template::{Endian::*, Expr as E, Template, Ty as T, Until};

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
fn compressed() -> T {
    T::structure(
        "Compressed",
        vec![
            ("algorithm", T::enumeration_hex("RootAlgorithm", T::u16(Big), ALGORITHM)),
            ("method", T::u8()),
            ("compressed_size", T::UInt { bits: 24, endian: Little }),
            ("uncompressed_size", T::UInt { bits: 24, endian: Little }),
            ("stream", T::bytes(E::field("compressed_size"))),
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
        vec![("name", tstring()), ("title", tstring()), ("directory", T::Named("Directory".into()))],
    )
}

/// A directory: when it was made and when it was last written, how big its key
/// list is, and where it, its parent and that list are.
///
/// The twelve bytes at the end are ROOT keeping room. A directory written with
/// 32-bit offsets leaves the space three 64-bit ones would have taken, so that
/// a file can grow past two gigabytes without the directory having to move.
fn directory() -> T {
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
            ("keys", at_if_set("fSeekKeys", T::Named("KeysList".into()))),
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
fn keys_list() -> T {
    with_key(
        "KeysList",
        "",
        "",
        vec![("nkeys", T::i32(Big)), ("keys", T::array(T::Named("KeyEntry".into()), E::field("nkeys")))],
    )
}

/// One entry of the key list, and the record it names.
fn key_entry() -> T {
    with_key("KeyEntry", "fName", "record", vec![("record", at_if_set("fSeekKey", T::Named("Record".into())))])
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

    Template::new("root", header)
        .with_type("FileRecord", file_record())
        .with_type("Directory", directory())
        .with_type("KeysList", keys_list())
        .with_type("KeyEntry", key_entry())
        .with_type("Record", record())
        .with_type("FreeRecord", free_record())
        .with_type("Free", free())
        .with_type("Compressed", compressed())
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

    /// A whole small file, laid out the way the real ones are: the header, the
    /// top directory's record, one compressed data record, the key list, the
    /// streamer information, and the free list.
    fn file() -> Vec<u8> {
        const BEGIN: i32 = 100;
        // The directory's record: name, title, and the sixty-byte directory.
        let dir_body = rstr("t.root").len() as i32 + rstr("").len() as i32 + 60;
        let file_kl = keylen("TFile", "t.root", "");
        let data_at = BEGIN + file_kl + dir_body;
        // The data record: a nine-byte block header and four bytes of stream,
        // standing for twenty bytes unpacked.
        let data_kl = keylen("TTree", "tree", "");
        let keys_at = data_at + data_kl + 13;
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

        b.extend(key("TTree", "tree", "", 20, 13, data_at, BEGIN));
        b.extend_from_slice(b"ZL");
        b.push(8);
        b.extend_from_slice(&[4, 0, 0]); // four bytes of stream
        b.extend_from_slice(&[20, 0, 0]); // twenty unpacked
        b.extend_from_slice(&[0x78, 0x9c, 0, 0]);
        assert_eq!(b.len() as i32, keys_at);

        b.extend(key("TFile", "t.root", "", keys_body, keys_body, keys_at, BEGIN));
        b.extend(be32(1)); // one key in the list
        b.extend(key("TTree", "tree", "", 20, 13, data_at, BEGIN));
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
        assert_eq!(ev.node(&d, &down(&block, &[2])).unwrap().value, Value::UInt(4));
        assert_eq!(ev.node(&d, &down(&block, &[3])).unwrap().value, Value::UInt(20));
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
