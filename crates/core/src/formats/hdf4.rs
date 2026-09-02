//! HDF4: a file that is nothing but a list of labelled runs of bytes.
//!
//! Four bytes of signature, and then blocks of data descriptors. A block says
//! how many descriptors it holds and where the next block is, and each
//! descriptor is a tag, a reference number, an offset and a length. The tag
//! says what kind of thing it is and the reference number tells one of that
//! kind from another, so a tag and a ref together name a thing and the offset
//! and length say where its bytes are. Nothing else in the file is structure:
//! an image, a scientific dataset, a group, a table are all a handful of
//! descriptors pointing at runs of bytes that refer to each other by ref.
//!
//! HDF5 shares the name and none of this. So does NetCDF-4, which is HDF5;
//! but HDF-EOS2, which is what MODIS and the older NASA missions publish, is
//! this format, and so are the `.hdf` files a Terra or Aqua granule arrives
//! as.
//!
//! What is read here is the chain of blocks and every descriptor in it, with
//! each descriptor's bytes placed where it says and named by its tag and ref.
//! Three of the tags are opened: the library version that wrote the file, the
//! file identifier and description, and the vdata header, which is the record
//! that says what the columns of a table are called and how wide they are. The
//! rest stay bytes, because the tag alone does not say how to read them and
//! opening a raster or a scientific dataset means following refs to the
//! dimension record, the number type and the palette that go with it.
//!
//! Everything is big-endian, which HDF4 calls its own on-disk order whatever
//! the machine that wrote the file was.

use crate::template::{Anchor, Encoding, Endian::Big, Expr as E, StrLen, Template, Ty as T};

/// What one of these starts with.
pub const MAGIC: &[u8] = &[0x0E, 0x03, 0x13, 0x01];

/// What a descriptor's tag means. The numbers are the ones in `htags.h`, and
/// the gaps in them are tags nothing has written for thirty years.
///
/// A tag with 0x4000 added is the same object stored some other way: linked
/// blocks, compressed, chunked, or held in another file. Every tag has such a
/// twin, so they are named as a run rather than one at a time, and the number
/// the run counts to is the ordinary tag underneath.
const TAG: &[(i128, &str)] = &[
    (1, "null"),
    (20, "linked blocks"),
    (30, "version"),
    (40, "compressed"),
    (50, "variable-length linked blocks"),
    (51, "variable-length linked data"),
    (60, "chunked"),
    (61, "chunk"),
    (100, "file identifier"),
    (101, "file description"),
    (102, "tag identifier"),
    (103, "tag description"),
    (104, "data identifier label"),
    (105, "data identifier annotation"),
    (106, "number type"),
    (107, "machine type"),
    (108, "free space"),
    (200, "8-bit image dimensions"),
    (201, "8-bit image palette"),
    (202, "8-bit raster image"),
    (203, "8-bit run-length image"),
    (204, "8-bit IMCOMP image"),
    (300, "image dimensions"),
    (301, "image palette"),
    (302, "raster image"),
    (303, "compressed image"),
    (304, "new-format raster image"),
    (306, "raster image group"),
    (307, "palette dimensions"),
    (308, "matte dimensions"),
    (309, "matte data"),
    (310, "colour correction"),
    (311, "colour format"),
    (312, "aspect ratio"),
    (400, "image sequence"),
    (401, "program to run"),
    (500, "x-y position"),
    (501, "machine type override"),
    (602, "Tektronix 4014 data"),
    (603, "Tektronix 4105 data"),
    (700, "scientific data group"),
    (701, "scientific data dimensions"),
    (702, "scientific data"),
    (703, "scales"),
    (704, "labels"),
    (705, "units"),
    (706, "formats"),
    (707, "max and min"),
    (708, "coordinate system"),
    (709, "transpose"),
    (710, "dataset links"),
    (720, "numeric data group"),
    (731, "calibration"),
    (732, "fill value"),
    (781, "ragged array line lengths"),
    (1962, "vdata description"),
    (1963, "vdata storage"),
    (1965, "vgroup"),
];

/// The number types a vdata field can have, which are the same numbers a
/// scientific dataset writes in its `number type` record.
const NUMBER_TYPE: &[(i128, &str)] = &[
    (3, "uchar"),
    (4, "char"),
    (5, "float32"),
    (6, "float64"),
    (20, "int8"),
    (21, "uint8"),
    (22, "int16"),
    (23, "uint16"),
    (24, "int32"),
    (25, "uint32"),
    (26, "int64"),
    (27, "uint64"),
];

/// A descriptor's tag, named where it is one of the known ones and named as a
/// special element where it is one of those.
fn tag() -> T {
    T::enum_ranged("Hdf4Tag", u16be(), TAG, &[(0x4000, 1, "special element for tag {n}")])
}

fn u16be() -> T {
    T::u16(Big)
}

fn u32be() -> T {
    T::u32(Big)
}

fn i16be() -> T {
    T::Int { bits: 16, endian: Big }
}

/// A length and that many bytes of text, which is how every name in a vdata
/// header is written.
fn counted_name() -> T {
    T::structure_named(
        "Hdf4Name",
        "",
        "text",
        vec![("len", i16be()), ("text", T::utf8(E::field("len").at_least(E::lit(0))))],
    )
}

/// The version record: which release of the library wrote the file, and a line
/// of text saying so in words.
fn version() -> T {
    T::structure(
        "Hdf4Version",
        vec![
            ("major", u32be()),
            ("minor", u32be()),
            ("release", u32be()),
            // Padded out with nuls to whatever room the writer left for it.
            ("description", T::text(StrLen::Padded { size: E::Remaining, pad: 0 }, Encoding::Ascii)),
        ],
    )
}

/// One column of a table: what its values are, how wide one is, where in a
/// record it starts, and how many of them there are per record.
///
/// The four are written as four arrays rather than as one array of four, so
/// this type is what a reader assembles rather than what the file holds.
fn vdata_fields(count: E) -> Vec<(&'static str, T)> {
    let n = || count.clone();
    vec![
        ("field_types", T::array(T::enumeration("Hdf4NumberType", i16be(), NUMBER_TYPE), n())),
        ("field_sizes", T::array(u16be(), n())),
        ("field_offsets", T::array(u16be(), n())),
        ("field_orders", T::array(u16be(), n())),
        ("field_names", T::array(counted_name(), n())),
    ]
}

/// The vdata header: the shape of a table, without any of its rows.
///
/// `ivsize` is one record, `nvertices` is how many of them there are, and the
/// rows themselves are in a separate descriptor tagged `vdata storage` with
/// the same ref.
///
/// The version and the `more` field are written twice over, once here and
/// again at the very end. That is not a mistake in the reading: a library old
/// enough not to know about the attributes in between still finds the pair it
/// expects by counting back from the end of the record. Version 4 is where
/// those attributes came in, and a version 3 header stops before the flags.
fn vdata_header() -> T {
    let n = || E::field("nfields").at_least(E::lit(0));
    let mut fields = vec![
        ("interlace", T::enumeration("Hdf4Interlace", i16be(), &[(0, "by record"), (1, "by field")])),
        ("nvertices", u32be()),
        ("ivsize", u16be()),
        ("nfields", i16be()),
    ];
    fields.extend(vdata_fields(n()));
    fields.extend(vec![
        ("name", counted_name()),
        ("class", counted_name()),
        // Where a record too large for this one is continued. Zero in every
        // file a current library writes.
        ("extension_tag", u16be()),
        ("extension_ref", u16be()),
        ("version", i16be()),
        ("more", i16be()),
        // From version 4 on: whether the table has attributes, and what they
        // are. Each attribute is the column it belongs to, or -1 for the
        // table as a whole, and the tag and ref of the vdata holding its
        // value.
        ("flags", T::present_if(E::lit(3).less_than(E::field("version")), u32be())),
        ("nattrs", T::present_if(E::field("flags").bit(0), T::i32(Big))),
        (
            "attributes",
            T::array(
                T::structure(
                    "Hdf4VdataAttribute",
                    vec![("field_index", T::i32(Big)), ("tag", tag()), ("ref", u16be())],
                ),
                E::field("nattrs").at_least(E::lit(0)),
            ),
        ),
        // The pair again, and the nul the writer ends the record with.
        ("version_again", T::if_room(i16be())),
        ("more_again", T::if_room(i16be())),
        ("terminator", T::bytes(E::Remaining)),
    ]);
    T::structure_named("Hdf4VdataHeader", "name", "", fields)
}

/// What a descriptor's bytes are read as. Only the three tags whose meaning is
/// in the bytes themselves are opened; every other tag needs another
/// descriptor to say what its bytes are, and following those is more than a
/// list of runs can promise.
fn contents() -> T {
    T::switch(
        E::field("tag"),
        vec![
            (30, version()),
            (100, T::text(StrLen::Fixed(E::Remaining), Encoding::Ascii)),
            (101, T::text(StrLen::Fixed(E::Remaining), Encoding::Ascii)),
            (1962, vdata_header()),
        ],
        T::bytes(E::Remaining),
    )
}

/// One descriptor: what the thing is, which one of its kind it is, and where
/// its bytes are.
///
/// A null descriptor is the empty slot in a block that was written with room
/// to grow, and its offset and length are both zero. Placing its data would
/// mean claiming the four bytes of signature at the front of the file, so it
/// is left pointing at nothing.
fn descriptor() -> T {
    T::structure_named(
        "Hdf4Descriptor",
        "tag",
        "data",
        vec![
            ("tag", tag()),
            ("ref", u16be()),
            ("offset", u32be()),
            ("length", u32be()),
            (
                "data",
                T::switch(
                    E::field("tag"),
                    vec![(1, T::bytes(E::lit(0)))],
                    T::at(E::field("offset"), T::sized(E::field("length"), contents())),
                ),
            ),
        ],
    )
}

/// One block of descriptors. The blocks are a chain rather than a table
/// because a writer that runs out of slots adds a block wherever there is
/// room, which may be the end of the file or a hole in the middle of it; a
/// block with no next holds zero rather than an offset.
fn block() -> T {
    T::structure(
        "Hdf4DescriptorBlock",
        vec![
            ("ndd", i16be()),
            ("next", u32be()),
            ("descriptors", T::array(T::Named("Hdf4Descriptor".into()), E::field("ndd").at_least(E::lit(0)))),
        ],
    )
    .machinery(&["next"])
    .counted_as("block")
}

pub fn hdf4() -> Template {
    // The first block sits straight after the signature, and each one says
    // where the next is. A list, rather than a block that holds the block that
    // holds the block: a file that has grown a dozen times is a dozen rows.
    let blocks =
        T::chain(E::size_of("magic"), &["next"], Anchor::File, T::Named("Hdf4DescriptorBlock".into()));
    let root = T::structure("Hdf4", vec![("magic", T::magic(MAGIC)), ("blocks", blocks)]);
    Template::new("hdf4", root)
        .with_type("Hdf4DescriptorBlock", block())
        .with_type("Hdf4Descriptor", descriptor())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{document::Document, eval::{Evaluator, Value}, source::MemSource};

    fn be16(v: i16) -> Vec<u8> {
        v.to_be_bytes().to_vec()
    }
    fn be32(v: u32) -> Vec<u8> {
        v.to_be_bytes().to_vec()
    }

    /// A counted name, as a vdata header writes one.
    fn nm(s: &str) -> Vec<u8> {
        let mut v = be16(s.len() as i16);
        v.extend_from_slice(s.as_bytes());
        v
    }

    /// A version 3 vdata header for a one-column table.
    fn vh() -> Vec<u8> {
        let mut v = be16(0); // interlace: by record
        v.extend(be32(4)); // four records
        v.extend(be16(8)); // eight bytes each
        v.extend(be16(1)); // one field
        v.extend(be16(24)); // int32
        v.extend(be16(4));
        v.extend(be16(0));
        v.extend(be16(2)); // two per record
        v.extend(nm("VALUES"));
        v.extend(nm("attname1"));
        v.extend(nm("Attr0.0"));
        v.extend(be16(0)); // extension tag
        v.extend(be16(0)); // extension ref
        v.extend(be16(3)); // version
        v.extend(be16(0)); // more
        v.extend(be16(3)); // and both again
        v.extend(be16(0));
        v.push(0);
        v
    }

    /// A version record, as the library writes one.
    fn ver() -> Vec<u8> {
        let mut v = be32(4);
        v.extend(be32(2));
        v.extend(be32(15));
        v.extend_from_slice(b"HDF Version 4.2 Release 15");
        v
    }

    /// A file of two blocks: the first holds the version record and a vdata
    /// header, the second a group and an empty slot.
    fn file() -> Vec<u8> {
        let (v, h) = (ver(), vh());
        // Two blocks of two descriptors, then the data they point at.
        let first = 4;
        let second = first + 6 + 24;
        let data = second + 6 + 24;
        let (ver_at, vh_at) = (data as u32, (data + v.len()) as u32);
        let vg_at = vh_at + h.len() as u32;
        let mut b = MAGIC.to_vec();
        b.extend(be16(2));
        b.extend(be32(second as u32));
        for (tag, r, off, len) in [(30u16, 1u16, ver_at, v.len() as u32), (1962, 4, vh_at, h.len() as u32)] {
            b.extend(tag.to_be_bytes());
            b.extend(r.to_be_bytes());
            b.extend(be32(off));
            b.extend(be32(len));
        }
        b.extend(be16(2));
        b.extend(be32(0));
        for (tag, r, off, len) in [(1965u16, 2u16, vg_at, 4u32), (1, 0, 0, 0)] {
            b.extend(tag.to_be_bytes());
            b.extend(r.to_be_bytes());
            b.extend(be32(off));
            b.extend(be32(len));
        }
        b.extend_from_slice(&v);
        b.extend_from_slice(&h);
        b.extend_from_slice(b"vgrp");
        b
    }

    #[test]
    fn the_blocks_are_a_flat_list_however_many_of_them_there_are() {
        let d = Document::new(MemSource(file()));
        let mut e = Evaluator::new(hdf4());
        // Two blocks side by side, not a block holding a block.
        let blocks = e.node(&d, &[1]).unwrap();
        assert_eq!(blocks.child_count, 2);
        assert_eq!(blocks.unit.as_deref(), Some("block"));
        assert_eq!(e.node(&d, &[1, 0, 0]).unwrap().value, Value::Int(2));
        assert_eq!(e.node(&d, &[1, 0, 2]).unwrap().child_count, 2);
        // The second block, found by following the first's `next`, with its
        // own two slots. It has no next of its own, so the walk stops rather
        // than pointing back at the signature.
        assert_eq!(e.node(&d, &[1, 1, 2]).unwrap().child_count, 2);
    }

    #[test]
    fn a_descriptor_is_named_by_its_tag_and_places_its_own_bytes() {
        let d = Document::new(MemSource(file()));
        let mut e = Evaluator::new(hdf4());
        let vg = e.node(&d, &[1, 1, 2, 0]).unwrap();
        assert_eq!(vg.name, "[0] vgroup");
        assert_eq!(e.node(&d, &[1, 1, 2, 0, 4, 0]).unwrap().size_bits, 4 * 8);
        // The null slot points at nothing rather than at the signature.
        assert_eq!(e.node(&d, &[1, 1, 2, 1, 4]).unwrap().child_count, 0);
    }

    #[test]
    fn the_version_record_is_read_and_the_rest_of_it_is_the_line_of_text() {
        let d = Document::new(MemSource(file()));
        let mut e = Evaluator::new(hdf4());
        assert_eq!(e.node(&d, &[1, 0, 2, 0, 4, 0, 0]).unwrap().value, Value::UInt(4));
        assert_eq!(e.node(&d, &[1, 0, 2, 0, 4, 0, 2]).unwrap().value, Value::UInt(15));
        assert_eq!(e.node(&d, &[1, 0, 2, 0, 4, 0, 3]).unwrap().value, Value::Str("HDF Version 4.2 Release 15".into()));
    }

    #[test]
    fn a_vdata_header_says_what_the_columns_are() {
        let d = Document::new(MemSource(file()));
        let mut e = Evaluator::new(hdf4());
        let vh = e.node(&d, &[1, 0, 2, 1, 4, 0]).unwrap();
        assert_eq!(vh.type_name, "Hdf4VdataHeader");
        assert_eq!(e.node(&d, &[1, 0, 2, 1, 4, 0, 1]).unwrap().value, Value::UInt(4));
        assert_eq!(e.node(&d, &[1, 0, 2, 1, 4, 0, 3]).unwrap().value, Value::Int(1));
        let ty = e.node(&d, &[1, 0, 2, 1, 4, 0, 4, 0]).unwrap();
        assert_eq!(ty.value, Value::Enum { raw: 24, name: Some("int32".into()), hex: false });
        assert_eq!(e.node(&d, &[1, 0, 2, 1, 4, 0, 8, 0, 1]).unwrap().value, Value::Str("VALUES".into()));
        assert_eq!(e.node(&d, &[1, 0, 2, 1, 4, 0, 9, 1]).unwrap().value, Value::Str("attname1".into()));
        assert_eq!(e.node(&d, &[1, 0, 2, 1, 4, 0, 10, 1]).unwrap().value, Value::Str("Attr0.0".into()));
    }

    #[test]
    fn a_version_three_header_has_no_flags_and_no_attributes() {
        let d = Document::new(MemSource(file()));
        let mut e = Evaluator::new(hdf4());
        // `version`, then the flags that are not there.
        assert_eq!(e.node(&d, &[1, 0, 2, 1, 4, 0, 13]).unwrap().value, Value::Int(3));
        assert_eq!(e.node(&d, &[1, 0, 2, 1, 4, 0, 15]).unwrap().size_bits, 0);
        assert_eq!(e.node(&d, &[1, 0, 2, 1, 4, 0, 16]).unwrap().size_bits, 0);
        assert_eq!(e.node(&d, &[1, 0, 2, 1, 4, 0, 17]).unwrap().child_count, 0);
        // And the pair written again at the end of the record.
        assert_eq!(e.node(&d, &[1, 0, 2, 1, 4, 0, 18]).unwrap().value, Value::Int(3));
        assert_eq!(e.node(&d, &[1, 0, 2, 1, 4, 0, 20]).unwrap().size_bits, 8);
    }
}
