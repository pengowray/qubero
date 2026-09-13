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
//! each descriptor's bytes placed where it says and named by its tag and ref,
//! and then the data itself: a table's rows with its columns named, a
//! scientific dataset's values in the shape its dimension record gives them, a
//! raster image as rows of pixels, a palette as colours, and a vgroup's members
//! as the places they point at.
//!
//! None of that can be read from one descriptor alone. A raster image's own
//! bytes say nothing about how wide a row is; the image dimension record with
//! the same reference number says. A table's rows say nothing about their
//! columns; the vdata header with the same reference number does. A scientific
//! dataset's values are tied to their dimensions by neither, only by the group
//! that names both. So each block reads its twelve-byte table twice: once as
//! the descriptors, which place the bytes, and once as an index with the tag
//! and the reference number as one number, which is a list every lookup here
//! can search. The index is a second reading of the same twelve bytes, counted
//! nowhere and pointing nowhere: it answers where a thing is, and the record
//! itself is then read beside whatever asked, so that a run of bytes is always
//! named by the descriptor that owns it and never by the reading that went
//! looking for it.
//!
//! That index covers one block. A file grown past sixteen descriptors has more
//! than one, and a thing in an earlier block is out of reach of a thing in a
//! later one; where a lookup finds nothing the reading falls back to bytes, so
//! a file loses detail rather than breaking. In the sample files about a tenth
//! of the references cross a block.
//!
//! Everything is big-endian, which HDF4 calls its own on-disk order whatever
//! the machine that wrote the file was, except where a number type record says
//! otherwise: class 4 is the Intel byte order for whole numbers and the PC's
//! IEEE for floating point, and a file written that way is read that way.

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

/// The fourth byte of a number type record, for a whole number: which way
/// round the bytes of one are written. Big-endian is what HDF4 calls its own,
/// and a file written on an Intel machine by a library told not to convert
/// says 4 here and means every number in it reads the other way.
const INT_CLASS: &[(i128, &str)] = &[(1, "big-endian"), (2, "VAX"), (4, "little-endian")];

/// The same byte for a floating point number: whose floating point it is. The
/// two that are still written are IEEE either way round; the rest are machines
/// whose last file is thirty years old.
const FLOAT_CLASS: &[(i128, &str)] =
    &[(1, "IEEE big-endian"), (2, "VAX"), (3, "Cray"), (4, "IEEE little-endian"), (5, "Convex"), (6, "VP")];

/// The same byte again for a character: which character set. One byte either
/// way round is the same byte, so this says nothing about order.
const CHAR_CLASS: &[(i128, &str)] = &[(0, "bytes"), (1, "ASCII"), (5, "EBCDIC")];

/// The three ways the samples of a raster image with more than one component
/// per pixel are laid out.
const PIXEL_INTERLACE: &[(i128, &str)] =
    &[(0, "by pixel"), (1, "by line"), (2, "by component")];

/// Which class byte means a little-endian machine wrote the numbers. The
/// same 4 for whole numbers (Intel byte order) and for floating point (the
/// PC's IEEE), which is why one number answers for both.
const LITTLE_ENDIAN_CLASS: i128 = 4;

/// A descriptor's tag, named where it is one of the known ones and named as a
/// special element where it is one of those.
fn tag() -> T {
    tag_of(u16be())
}

/// The same names, over a number that is not two bytes of the file: a vgroup
/// writes its members' tags in a run of their own, and the row that puts a tag
/// back beside its reference number has to name it too.
fn tag_of(inner: T) -> T {
    T::enum_ranged("Hdf4Tag", inner, TAG, &[(0x4000, 1, "special element for tag {n}")])
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

/// The number type record: four bytes saying how to read every value of the
/// thing that names it.
///
/// `type` is the same number a vdata header writes for a column, and it
/// carries the width and the sign on its own. `width` says the same thing in
/// bits, and `class` says which machine's way round the bytes are, which for
/// anything still written means big-endian or little. What `class` means at
/// all depends on the type, since the numbers were handed out three times
/// over: 1 is IEEE for a float, Motorola byte order for a whole number and
/// ASCII for a character.
fn number_type() -> T {
    T::structure(
        "Hdf4NumberType",
        vec![
            ("version", T::u8()),
            ("type", T::enumeration("Hdf4NumberTypeName", T::u8(), NUMBER_TYPE)),
            ("width", T::u8()),
            (
                "class",
                T::switch(
                    E::field("type"),
                    vec![
                        (3, T::enumeration("Hdf4CharClass", T::u8(), CHAR_CLASS)),
                        (4, T::enumeration("Hdf4CharClass", T::u8(), CHAR_CLASS)),
                        (5, T::enumeration("Hdf4FloatClass", T::u8(), FLOAT_CLASS)),
                        (6, T::enumeration("Hdf4FloatClass", T::u8(), FLOAT_CLASS)),
                    ],
                    T::enumeration("Hdf4IntClass", T::u8(), INT_CLASS),
                ),
            ),
        ],
    )
}

/// One value of the number type a number type record describes, read where
/// that record's `type` and `class` can be reached by expression.
///
/// Twelve types and two byte orders is twenty-four cases, which is what it
/// costs to say that a byte order read out of the file decides how a number
/// is read: the type of a field is settled when the template is built, so the
/// choice has to be a switch over the types rather than an order handed to
/// one. Anything the number type record does not name is left as bytes as
/// wide as it says it is, so an unknown type does not push everything after
/// it out of line.
fn number_value(number_type: E, class: E, width: E) -> T {
    let of = |e: crate::template::Endian| {
        T::switch(
            number_type.clone(),
            vec![
                (3, T::u8()),
                (4, T::Int { bits: 8, endian: e }),
                (5, T::F32(e)),
                (6, T::F64(e)),
                (20, T::Int { bits: 8, endian: e }),
                (21, T::u8()),
                (22, T::Int { bits: 16, endian: e }),
                (23, T::u16(e)),
                (24, T::i32(e)),
                (25, T::u32(e)),
                (26, T::Int { bits: 64, endian: e }),
                (27, T::u64(e)),
            ],
            T::bytes(width.clone().div(E::lit(8)).at_least(E::lit(1))),
        )
    };
    T::switch(
        class.equals(E::lit(LITTLE_ENDIAN_CLASS)),
        vec![(1, of(crate::template::Endian::Little))],
        of(Big),
    )
}

/// The image dimension record, which every raster image and every palette
/// that was written with one has beside it.
///
/// `xdim` is how many pixels are across and `ydim` how many rows there are,
/// and the image is written a row at a time. `nelements` is how many samples
/// make a pixel: one for a greyscale image or an index into a palette, three
/// for RGB. A palette's own dimension record says 256 by 1 with three
/// elements, which is the same thing said about a table of colours.
///
/// The compression tag is zero for an image stored as it is. Where it is not,
/// the bytes are compressed and the tag says how.
fn image_dimensions() -> T {
    T::structure(
        "Hdf4ImageDimensions",
        vec![
            ("xdim", T::i32(Big)),
            ("ydim", T::i32(Big)),
            ("number_type_tag", tag()),
            ("number_type_ref", u16be()),
            ("nelements", i16be()),
            ("interlace", T::enumeration("Hdf4PixelInterlace", i16be(), PIXEL_INTERLACE)),
            ("compression_tag", tag()),
            ("compression_ref", u16be()),
        ],
    )
}

/// The dimension record of an eight-bit image, which is all the older tags
/// say: how wide, how tall, and nothing about the number type, because there
/// was only ever one.
fn eight_bit_dimensions() -> T {
    T::structure("Hdf4EightBitDimensions", vec![("xdim", i16be()), ("ydim", i16be())])
}

/// The dimension record of a scientific dataset: how many dimensions it has,
/// how long each of them is, what its values are, and what the scale along
/// each dimension is.
///
/// The first dimension is the slowest to change, the way C counts an array.
/// Every scale has a number type of its own because a dataset of bytes may
/// still be measured in seconds.
fn sd_dimensions() -> T {
    let rank = || E::field("rank").at_least(E::lit(0));
    T::structure(
        "Hdf4SdDimensions",
        vec![
            ("rank", i16be()),
            ("dims", T::array(T::i32(Big), rank())),
            ("number_type_tag", tag()),
            ("number_type_ref", u16be()),
            (
                "scales",
                T::array(
                    T::structure("Hdf4SdScaleType", vec![("tag", tag()), ("ref", u16be())]),
                    rank(),
                ),
            ),
        ],
    )
}

/// A descriptor's tag and reference number read as one number, which is how
/// the block's index holds them and the only way a search can ask for both at
/// once. See [`index_entry`].
fn key(tag: i128, r: E) -> E {
    E::lit(tag * 65536).add(r)
}

/// Where the descriptor this key names keeps its bytes: `offset` or `length`,
/// from the block's index.
///
/// Zero when the block holds no such descriptor, which is what every guard
/// here tests. An offset of zero is the four bytes of signature and no
/// descriptor ever points there, so zero is an answer no file can give.
fn look(k: E, path: &[&str]) -> E {
    E::tagged_by_expr("table", &["key"], k, path)
}

/// The record the descriptor this key names points at, read where it says it
/// is and no further.
///
/// A second reading of bytes the descriptor itself places, so every field
/// declared this way is put aside and counted there. Nothing but the numbers
/// in it is wanted: how wide a row is, how many dimensions there are, which
/// way round the bytes go.
fn record_at(k: E, inner: T) -> T {
    T::at(look(k.clone(), &["offset"]), T::sized(look(k, &["length"]), inner))
}

/// The same, where the key came out of another record and so could not be
/// tested before the field was declared.
///
/// A number type record is named by a reference number written inside the
/// dimension record, and whether the block holds it cannot be asked until the
/// dimension record has been read. So this reads nothing at all rather than
/// failing when it is not there: no room, no record. Whatever the record was
/// wanted for still has to ask [`found`] before it reads a value through it.
fn record_at_or_nothing(k: E, inner: T) -> T {
    T::at(
        look(k.clone(), &["offset"]).at_least(E::size_of("magic")),
        T::sized(look(k, &["length"]), T::if_room(inner)),
    )
}

/// The reference number of the descriptor whose bytes these are, asked from
/// inside whatever they were read as. Everything an HDF4 object is made of
/// carries the same reference number, so this is how a raster image finds its
/// dimensions and a table's rows find their columns.
fn own_ref() -> E {
    E::field("ref")
}

/// One when the block holds a descriptor with this tag and this reference
/// number, and zero otherwise.
fn found(k: E) -> E {
    E::lit(0).less_than(look(k, &["offset"]))
}

/// A tag and a reference number together, which is how a group names one of
/// the things in it, and where in the file that thing is.
///
/// The offset and the length take no bytes: they are the step from a name to
/// a place, so that a reader looking at a group can see what it holds rather
/// than a pair of numbers to go and find by hand.
fn tag_ref() -> T {
    let k = || E::field("tag").mul(E::lit(65536)).add(E::field("ref"));
    T::structure_named(
        "Hdf4GroupMember",
        "tag",
        "",
        vec![
            ("tag", tag()),
            ("ref", u16be()),
            ("offset", T::computed(look(k(), &["offset"]))),
            ("length", T::computed(look(k(), &["length"]))),
        ],
    )
}

/// A group: nothing but the tags and reference numbers of the descriptors
/// that belong together. A raster image group names the dimension record and
/// the image; a scientific data group names the dimensions, the number type
/// and the data.
fn group() -> T {
    T::structure("Hdf4Group", vec![("members", T::array(tag_ref(), E::Remaining.div(E::lit(4))))])
}

/// Where each of a vgroup's members is, worked out from the tag and the
/// reference number that name it.
///
/// A vgroup writes its tags and its reference numbers as two runs, so these
/// rows put the two back together and add where each pair points. They take
/// no bytes of their own.
fn members(tag_from: E, ref_of: E, count: E) -> T {
    let k = || E::field("tag").mul(E::lit(65536)).add(E::field("ref"));
    T::array(
        T::structure_named(
            "Hdf4VgroupMember",
            "tag",
            "",
            vec![
                ("tag", tag_of(T::computed(tag_from))),
                ("ref", T::computed(ref_of)),
                ("offset", T::computed(look(k(), &["offset"]))),
                ("length", T::computed(look(k(), &["length"]))),
            ],
        ),
        count,
    )
}

/// A palette: 256 colours, each a red, a green and a blue byte.
///
/// The count is the length rather than a fixed 256, because the older
/// eight-bit palette tag and the newer lookup table are both written this way
/// and a writer is free to keep fewer.
fn palette() -> T {
    T::structure(
        "Hdf4Palette",
        vec![(
            "colours",
            T::array(
                T::structure("Hdf4Colour", vec![("red", T::u8()), ("green", T::u8()), ("blue", T::u8())]),
                E::Remaining.div(E::lit(3)),
            ),
        )],
    )
}

/// The scales of a scientific dataset: what the steps along each of its
/// dimensions stand for.
///
/// One byte per dimension says whether that dimension has a scale at all, and
/// then the scales that are there follow one after another, each as long as
/// its dimension and in the number type the dimension record gave it. So
/// nothing here can be read without the dimension record with the same
/// reference number, which says how many dimensions there are, how long each
/// is, and what each scale's values are.
fn sd_scales() -> T {
    let dims = || key(701, own_ref());
    let rank = || E::within(&["dimensions", "rank"]).at_least(E::lit(0));
    let body = T::structure(
        "Hdf4SdScales",
        vec![
            ("dimensions", record_at(dims(), sd_dimensions())),
            ("has_scale", T::array(T::u8(), rank())),
            ("scales", T::array(one_scale(), rank())),
        ],
    )
    .machinery(&["dimensions"])
    .field_aside("dimensions");
    T::switch(found(dims()), vec![(1, body)], T::bytes(E::Remaining))
}

/// The scale along one dimension, or nothing where the flag byte for that
/// dimension said there is none.
///
/// Which dimension this is, is written down as a field of no bits rather than
/// asked for as `Idx` where it is needed, for the reason a vdata's columns
/// are: inside the run of values `Idx` is which value this is.
fn one_scale() -> T {
    let which = || E::field("dimension");
    let nt = || key(106, E::elem_within(&["dimensions", "scales"], which(), &["ref"]));
    let run = T::array(
        of_the_number_type(),
        E::elem_within(&["dimensions", "dims"], which(), &[]).at_least(E::lit(0)),
    );
    // A dimension with no scale has no bytes here. One whose number type is in
    // another block stops the reading where it stands: how wide its values are
    // is what says where the next dimension's scale begins, so everything
    // after it is unplaceable and the rest of the record is left as bytes.
    let values = T::switch(found(nt()), vec![(1, run)], T::bytes(E::Remaining));
    T::structure_named(
        "Hdf4SdScale",
        "",
        "values",
        vec![
            ("dimension", T::computed(E::idx())),
            ("number_type", record_at_or_nothing(nt(), number_type())),
            ("values", T::switch(E::elem("has_scale", which()), vec![(1, values)], T::bytes(E::lit(0)))),
        ],
    )
    .machinery(&["dimension", "number_type"])
    .field_aside("number_type")
}

/// One value of whatever the `number_type` field beside it says, which is the
/// shape every reading here takes: a number type record is read where the
/// descriptor that names it says, put aside so its bytes count once, and the
/// values read through it.
fn of_the_number_type() -> T {
    number_value(
        E::within(&["number_type", "type"]),
        E::within(&["number_type", "class"]),
        E::within(&["number_type", "width"]),
    )
}

/// The largest and the smallest value in a scientific dataset, in the number
/// type of the dataset itself, which is the number type record with the same
/// reference number.
fn max_and_min() -> T {
    let body = T::structure(
        "Hdf4MaxAndMin",
        vec![
            ("number_type", record_at(key(106, own_ref()), number_type())),
            ("max", of_the_number_type()),
            ("min", of_the_number_type()),
        ],
    )
    .machinery(&["number_type"])
    .field_aside("number_type");
    T::switch(found(key(106, own_ref())), vec![(1, body)], T::bytes(E::Remaining))
}

/// The header of a special element: the same object kept somewhere other than
/// in one run of bytes.
///
/// Every tag has a twin with 0x4000 added, and a descriptor carrying one of
/// those points at this rather than at the object. The first number says which
/// of the seven ways it is kept, and the two worth reading say where the rest
/// is: linked blocks name the first of a chain of them, and a compressed
/// element names the run of compressed bytes and says what compressed it. The
/// bytes themselves stay bytes either way, under the tag that holds them.
fn special_element() -> T {
    let linked = T::structure(
        "Hdf4LinkedBlocks",
        vec![
            ("length", T::i32(Big)),
            ("block_length", T::i32(Big)),
            ("number_blocks", T::i32(Big)),
            ("link_ref", u16be()),
        ],
    );
    let compressed = T::structure(
        "Hdf4CompressedElement",
        vec![
            ("version", i16be()),
            ("length", T::i32(Big)),
            ("data_ref", u16be()),
            ("model_type", T::enumeration("Hdf4CompressionModel", u16be(), &[(0, "stored as read")])),
            (
                "compression",
                T::enumeration(
                    "Hdf4Compression",
                    u16be(),
                    &[
                        (0, "none"),
                        (1, "run-length"),
                        (2, "n-bit"),
                        (3, "skipping Huffman"),
                        (4, "deflate"),
                        (5, "szip"),
                    ],
                ),
            ),
            ("settings", T::bytes(E::Remaining)),
        ],
    );
    T::structure(
        "Hdf4SpecialElement",
        vec![
            (
                "kind",
                T::enumeration(
                    "Hdf4SpecialKind",
                    i16be(),
                    &[
                        (1, "linked blocks"),
                        (2, "in another file"),
                        (3, "compressed"),
                        (4, "variable-length linked blocks"),
                        (5, "chunked"),
                        (6, "buffered"),
                        (7, "compressed raster"),
                    ],
                ),
            ),
            ("kept", T::switch(E::field("kind"), vec![(1, linked), (3, compressed)], T::bytes(E::Remaining))),
        ],
    )
}

/// The label, unit and format records of a scientific dataset: one string for
/// the dataset and then one for each of its dimensions, run together and each
/// ended with a nul.
fn sd_strings() -> T {
    T::structure(
        "Hdf4SdStrings",
        vec![(
            "strings",
            T::repeat(
                T::text(StrLen::Terminated { end: 0, or_end: true }, Encoding::Ascii),
                crate::template::Until::End,
            ),
        )],
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
        ("field_types", T::array(T::enumeration("Hdf4NumberTypeName", i16be(), NUMBER_TYPE), n())),
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
    ]);
    fields.extend(version_four_attributes());
    fields.extend(vec![
        // The pair again, and the nul the writer ends the record with.
        ("version_again", T::if_room(i16be())),
        ("more_again", T::if_room(i16be())),
        ("terminator", T::bytes(E::Remaining)),
    ]);
    T::structure_named("Hdf4VdataHeader", "name", "", fields)
}

/// A raster image, read as rows of pixels.
///
/// Nothing in the image's own bytes says how wide a row is: the image
/// dimension record with the same reference number does, and the number type
/// record that names says how wide one sample is. Where either is missing the
/// bytes stay bytes, which is what happens to an image whose dimension record
/// the writer put in another block.
///
/// Interlacing decides the nesting rather than the count, so the three ways
/// an image can be written are three shapes and each says in its name which
/// index is which. By pixel, the samples of one pixel are together, which is
/// how an RGB image is usually written; by line, a row holds one run per
/// component; by component, the whole of the red plane comes before the whole
/// of the green.
fn raster_image() -> T {
    let id = |f: &str| E::within(&["dimensions", f]);
    let (x, y) = (id("xdim").at_least(E::lit(0)), id("ydim").at_least(E::lit(0)));
    let n = || id("nelements").at_least(E::lit(1));
    let s = of_the_number_type;
    let samples = T::switch(
        E::within(&["dimensions", "interlace"]),
        vec![
            (
                2,
                T::structure(
                    "Hdf4RasterComponentPlanes",
                    vec![("components", T::array(T::array(T::array(s(), x.clone()), y.clone()), n()))],
                ),
            ),
            (
                1,
                T::structure(
                    "Hdf4RasterComponentLines",
                    vec![("rows", T::array(T::array(T::array(s(), x.clone()), n()), y.clone()))],
                ),
            ),
        ],
        T::structure("Hdf4RasterPixels", vec![("rows", T::array(T::array(T::array(s(), n()), x), y))]),
    );
    let nt = || key(106, E::within(&["dimensions", "number_type_ref"]));
    let body = T::structure(
        "Hdf4RasterImage",
        vec![
            ("dimensions", record_at(key(300, own_ref()), image_dimensions())),
            ("number_type", record_at_or_nothing(nt(), number_type())),
            // Which number type it is, is written in the dimension record, so
            // whether the block holds it cannot be asked until that record has
            // been read: the question belongs here rather than around the
            // whole image.
            ("samples", T::switch(found(nt()), vec![(1, samples)], T::bytes(E::Remaining))),
        ],
    )
    .machinery(&["dimensions", "number_type"])
    .field_aside("dimensions")
    .field_aside("number_type");
    T::switch(found(key(300, own_ref())), vec![(1, body)], T::bytes(E::Remaining))
}

/// An eight-bit raster image, which the older tags write with a dimension
/// record that says only how wide and how tall: one byte a pixel, and what
/// the byte means is in the palette beside it.
fn eight_bit_image() -> T {
    let id = |f: &str| E::within(&["dimensions", f]).at_least(E::lit(0));
    let body = T::structure(
        "Hdf4EightBitImage",
        vec![
            ("dimensions", record_at(key(200, own_ref()), eight_bit_dimensions())),
            ("rows", T::array(T::array(T::u8(), id("xdim")), id("ydim"))),
        ],
    )
    .machinery(&["dimensions"])
    .field_aside("dimensions");
    T::switch(found(key(200, own_ref())), vec![(1, body)], T::bytes(E::Remaining))
}

/// The rows of a table, read through the vdata header with the same reference
/// number.
///
/// The header says how many records there are, how wide one is, and what the
/// columns of one hold; these bytes say none of it. So the header is read
/// again here, at no cost in bytes, and every column's type, width and name
/// is taken from it: a column of characters reads as text as long as its
/// order, a column of numbers as that many numbers, and a column of a type
/// nothing here knows as the bytes the header set aside for it.
///
/// Only a table written a record at a time is opened. With the other
/// interlacing the file holds every value of the first column, then every
/// value of the second, and a row is not a run of bytes at all.
fn vdata_records() -> T {
    let header = || key(1962, own_ref());
    let records = T::array(
        T::sized(E::within(&["header", "ivsize"]).at_least(E::lit(0)), vdata_record()),
        E::within(&["header", "nvertices"]).at_least(E::lit(0)),
    );
    let rows = T::structure(
        "Hdf4VdataRecords",
        vec![
            ("header", record_at(header(), vdata_header())),
            (
                "records",
                T::switch(
                    E::within(&["header", "interlace"]).equals(E::lit(0)),
                    vec![(1, records)],
                    T::bytes(E::Remaining),
                ),
            ),
        ],
    )
    .machinery(&["header"])
    .field_aside("header");
    T::switch(found(header()), vec![(1, rows)], T::bytes(E::Remaining))
}

/// One record: one value per column, in the order the header lists them, each
/// labelled with the name the header gives that column.
fn vdata_record() -> T {
    T::structure(
        "Hdf4VdataRecord",
        vec![(
            "values",
            T::array(vdata_value(), E::within(&["header", "nfields"]).at_least(E::lit(0))),
        )],
    )
    .field_elem_named_from("values", E::elem_within(&["header", "field_names"], E::idx(), &["text"]))
}

/// One column's worth of one record.
///
/// Everything about the column is in the header's arrays at this column's
/// index, and the index is written down here rather than asked for as `Idx`
/// where it is needed: a column of several values is a list, and inside a list
/// `Idx` is which value this is, not which column. So the number is a field of
/// the row, worth no bytes, and every question about the column names it.
///
/// A field type with bit 14 set is the same type written the other way round,
/// which is how a vdata written on an Intel machine and left unconverted says
/// so. A column of characters reads as one string of its order rather than as
/// that many one-byte numbers.
fn vdata_value() -> T {
    let col = |f: &str| E::elem_within(&["header", f], E::field("column"), &[]);
    let kind = || col("field_types").and(E::lit(0xff));
    let order = || col("field_orders").at_least(E::lit(0));
    let one = || number_value(kind(), col("field_types").bit(14).mul(E::lit(4)), E::lit(8));
    let text = || T::text(StrLen::Padded { size: order(), pad: 0 }, Encoding::Ascii);
    T::structure_named(
        "Hdf4VdataField",
        "",
        "value",
        vec![
            ("column", T::computed(E::idx())),
            (
                "value",
                T::switch(
                    kind(),
                    vec![(3, text()), (4, text())],
                    T::switch(order().equals(E::lit(1)), vec![(1, one())], T::array(one(), order())),
                ),
            ),
        ],
    )
    .machinery(&["column"])
}

/// A scientific data group, and the dataset it names.
///
/// The group is a list of tags and reference numbers, and the three that
/// matter are the dimension record, the number type and the data. None of
/// them shares a reference number with the data in a file written by the SD
/// interface, so the group is the only thing that ties them together, and the
/// values are read here rather than under the descriptor that holds them.
/// Those bytes are counted there; this is a second reading of them.
///
/// The first dimension is the slowest to change. Past four dimensions the
/// values stay bytes: the nesting is written out a rank at a time, and no
/// expression can say how deep to go.
fn scientific_data_group() -> T {
    let member = |t: i128| key(t, E::tagged("members", &["tag"], t, &["ref"]));
    // The number type is named by the dimension record rather than by the
    // group: a file written by the old interface lists it among the members
    // and one written by the SD interface gives it a reference number of its
    // own, and the dimension record is right either way.
    let nt = || key(106, E::within(&["dimensions", "number_type_ref"]));
    let dim = |i: i128| E::elem_within(&["dimensions", "dims"], E::lit(i), &[]).at_least(E::lit(0));
    // The last dimension is wrapped first, so it ends up innermost and the
    // first dimension outermost, which is the order the values are written
    // in: the last index is the one that changes from one value to the next.
    let cases: Vec<(i128, T)> = (1..=4i128)
        .map(|rank| {
            let mut shape = of_the_number_type();
            for i in (0..rank).rev() {
                shape = T::array(shape, dim(i));
            }
            (rank, shape)
        })
        .collect();
    let dataset = T::structure(
        "Hdf4ScientificDataset",
        vec![
            ("dimensions", record_at(member(701), sd_dimensions())),
            ("number_type", record_at_or_nothing(nt(), number_type())),
            (
                "values",
                T::switch(
                    found(nt()),
                    vec![(
                        1,
                        record_at(
                            member(702),
                            T::switch(E::within(&["dimensions", "rank"]), cases, T::bytes(E::Remaining)),
                        ),
                    )],
                    T::bytes(E::lit(0)),
                ),
            ),
        ],
    )
    .machinery(&["dimensions", "number_type"])
    .field_aside("dimensions")
    .field_aside("number_type")
    .field_aside("values");
    // The members have to be read before anything can ask what they name, so
    // the dataset is a field after them rather than a shape the whole record
    // is switched onto.
    T::structure(
        "Hdf4ScientificDataGroup",
        vec![
            ("members", T::array(tag_ref(), E::Remaining.div(E::lit(4)))),
            (
                "dataset",
                T::switch(
                    found(member(701)).mul(found(member(702))),
                    vec![(1, dataset)],
                    T::bytes(E::lit(0)),
                ),
            ),
        ],
    )
}

/// The attributes a vdata header or a vgroup grew in version 4: a flag word,
/// a count, and one record per attribute saying which field it belongs to and
/// where its value is kept.
///
/// A field index of -1 is an attribute of the whole thing rather than of one
/// of its columns. The value itself is a vdata of one record, named by the tag
/// and ref here.
fn version_four_attributes() -> Vec<(&'static str, T)> {
    vec![
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
    ]
}

/// A vgroup: a name, a class, and the tags and reference numbers of whatever
/// belongs to it, which may be other vgroups. This is the only tag that makes
/// a file a tree rather than a list, and it is what the HDF4 interfaces call a
/// group, a scientific dataset's container and an image's container alike.
///
/// The tags and the reference numbers are written as two runs rather than as
/// pairs, so `members` reads them back together and is where each one points.
fn vgroup() -> T {
    let n = || E::field("nvelt").at_least(E::lit(0));
    let mut fields = vec![
        ("nvelt", i16be()),
        ("tags", T::array(tag(), n())),
        ("refs", T::array(u16be(), n())),
        ("members", members(E::elem("tags", E::idx()), E::elem("refs", E::idx()), n())),
        ("name", counted_name()),
        ("class", counted_name()),
        ("extension_tag", u16be()),
        ("extension_ref", u16be()),
        ("version", i16be()),
        ("more", i16be()),
    ];
    fields.extend(version_four_attributes());
    fields.push(("terminator", T::bytes(E::Remaining)));
    // The two runs are where the bytes are; `members` is the reading of them a
    // person wants, so it is the one the listing leads with.
    T::structure_named("Hdf4Vgroup", "name", "", fields).machinery(&["tags", "refs"])
}

/// What a descriptor's bytes are read as.
///
/// A tag on its own says what a run of bytes is for, not how to read it. The
/// ones here are the ones a reader can take apart with the block's index in
/// hand: a raster image asks the image dimension record with its own reference
/// number how wide the rows are, a table's rows ask the vdata header with the
/// same number what the columns are, and a scientific dataset's values are
/// reached from the group that names the dimension record and the data
/// together. Where the descriptor being asked for is not in the same block,
/// the reading falls back to the bytes, since the index covers one block.
fn contents() -> T {
    T::switch(
        E::field("tag"),
        vec![
            (30, version()),
            (100, T::text(StrLen::Fixed(E::Remaining), Encoding::Ascii)),
            (101, T::text(StrLen::Fixed(E::Remaining), Encoding::Ascii)),
            (104, T::text(StrLen::Fixed(E::Remaining), Encoding::Ascii)),
            (105, T::text(StrLen::Fixed(E::Remaining), Encoding::Ascii)),
            (106, number_type()),
            (200, eight_bit_dimensions()),
            (201, palette()),
            (202, eight_bit_image()),
            (300, image_dimensions()),
            (301, palette()),
            (302, raster_image()),
            (306, group()),
            (307, image_dimensions()),
            (700, scientific_data_group()),
            (701, sd_dimensions()),
            (703, sd_scales()),
            (704, sd_strings()),
            (705, sd_strings()),
            (706, sd_strings()),
            (707, max_and_min()),
            (720, scientific_data_group()),
            (1962, vdata_header()),
            (1963, vdata_records()),
            (1965, vgroup()),
        ],
        // Anything with 0x4000 added is a tag naming an object kept somewhere
        // other than in one run of bytes, and what the descriptor points at is
        // the header that says where.
        T::switch(E::field("tag").bit(14), vec![(1, special_element())], T::bytes(E::Remaining)),
    )
}

/// One when this descriptor has bytes somewhere and zero when it has none.
///
/// A null descriptor is the empty slot in a block that was written with room
/// to grow; a descriptor of a real tag whose offset is all ones is a thing the
/// library has named and not yet written, which is what an empty vdata looks
/// like. Placing either would mean claiming bytes at the front of the file or
/// four gigabytes past the end of it.
fn has_bytes(tag_of: E, offset_of: E) -> E {
    E::lit(1).sub(tag_of.equals(E::lit(1))).mul(E::lit(1).sub(offset_of.equals(E::lit(0xFFFF_FFFFi64))))
}

/// The same twelve bytes as a descriptor, read a second time as an index the
/// rest of the template can search.
///
/// A lookup reaches a list by naming it, and a list cannot name itself: a name
/// reaches the fields declared before the field asking, and the descriptors
/// are the field the asking descriptor sits inside. So a block reads its own
/// table twice over.
///
/// The same four numbers, read the same way, so that a reader who lands on
/// these bytes sees the same rows either way. `key` is the tag and the
/// reference number as one number, and is worth no bytes of its own: a search
/// matches one field, and a thing in an HDF4 file is named by both. Nothing
/// here points anywhere, so nothing here can take the naming of a run of bytes
/// away from the descriptor that owns it.
fn index_entry() -> T {
    T::structure(
        "Hdf4Reference",
        vec![
            ("tag", tag()),
            ("ref", u16be()),
            ("offset", u32be()),
            ("length", u32be()),
            ("key", T::computed(E::field("tag").mul(E::lit(65536)).add(E::field("ref")))),
        ],
    )
    .machinery(&["key"])
}

/// One descriptor: what the thing is, which one of its kind it is, and where
/// its bytes are.
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
                    has_bytes(E::field("tag"), E::field("offset")),
                    vec![(1, T::at(E::field("offset"), T::sized(E::field("length"), contents())))],
                    T::bytes(E::lit(0)),
                ),
            ),
        ],
    )
}

/// One block of descriptors. The blocks are a chain rather than a table
/// because a writer that runs out of slots adds a block wherever there is
/// room, which may be the end of the file or a hole in the middle of it; a
/// block with no next holds zero rather than an offset.
///
/// The block is an origin so that its index can be placed six bytes into it,
/// over the same run the descriptors themselves read. See [`index_entry`].
fn block() -> T {
    let n = || E::field("ndd").at_least(E::lit(0));
    T::origin(
        T::structure(
            "Hdf4DescriptorBlock",
            vec![
                ("ndd", i16be()),
                ("next", u32be()),
                ("table", T::at_origin(E::lit(6), T::array(index_entry(), n()))),
                ("descriptors", T::array(T::Named("Hdf4Descriptor".into()), n())),
            ],
        )
        .machinery(&["next", "table"])
        .field_aside("table")
        .counted_as("block"),
    )
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

    /// A version 3 vdata header for a one-column table of pairs of int32.
    fn vh() -> Vec<u8> {
        let mut v = be16(0); // interlace: by record
        v.extend(be32(4)); // four records
        v.extend(be16(8)); // eight bytes each
        v.extend(be16(1)); // one field
        v.extend(be16(24)); // int32
        v.extend(be16(8));
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

    /// The four records the header above describes: two int32 in each.
    fn vs() -> Vec<u8> {
        (0..8u32).flat_map(be32).collect()
    }

    /// A version record, as the library writes one.
    fn ver() -> Vec<u8> {
        let mut v = be32(4);
        v.extend(be32(2));
        v.extend(be32(15));
        v.extend_from_slice(b"HDF Version 4.2 Release 15");
        v
    }

    /// A vgroup naming the table in its own block and the image in the next
    /// one, which the block's index cannot reach.
    fn vg() -> Vec<u8> {
        let mut v = be16(2);
        v.extend(be16(1962));
        v.extend(be16(302));
        v.extend(be16(4));
        v.extend(be16(7));
        v.extend(nm("grp"));
        v.extend(nm(""));
        v.extend(be16(0)); // extension tag
        v.extend(be16(0)); // extension ref
        v.extend(be16(3)); // version
        v.extend(be16(0)); // more
        v.push(0);
        v
    }

    /// A number type record: `kind` and the class byte that says which way
    /// round its bytes are.
    fn nt(kind: u8, width: u8, class: u8) -> Vec<u8> {
        vec![1, kind, width, class]
    }

    /// An image dimension record for a three by two image of single bytes,
    /// whose number type record is the one with reference number `nt`.
    fn id(nt: i16) -> Vec<u8> {
        let mut v = be32(3); // xdim
        v.extend(be32(2)); // ydim
        v.extend(be16(106)); // number type tag
        v.extend(be16(nt)); // and ref
        v.extend(be16(1)); // one sample a pixel
        v.extend(be16(0)); // by pixel
        v.extend(be16(0)); // no compression
        v.extend(be16(0));
        v
    }

    /// A dimension record for a two by three dataset of int16.
    fn sdd() -> Vec<u8> {
        let mut v = be16(2); // rank
        v.extend(be32(2));
        v.extend(be32(3));
        v.extend(be16(106)); // the number type of the values
        v.extend(be16(8));
        for _ in 0..2 {
            v.extend(be16(106)); // and of each dimension's scale
            v.extend(be16(8));
        }
        v
    }

    /// The six int16 that dimension record describes, written the little end
    /// first, which is what its number type record says.
    fn sd() -> Vec<u8> {
        (0..6i16).flat_map(|v| v.to_le_bytes().to_vec()).collect()
    }

    /// A group naming the data, the dimensions and the number type that go
    /// with it, each under a reference number of its own, which is how the SD
    /// interface writes one.
    fn ndg() -> Vec<u8> {
        let mut v = be16(702);
        v.extend(be16(9));
        v.extend(be16(701));
        v.extend(be16(9));
        v.extend(be16(106));
        v.extend(be16(8));
        v
    }

    /// A file of three blocks. The first holds the version record, a table's
    /// rows and then the header that describes them, and a vgroup; the second
    /// holds an image before the dimension record that says how wide it is,
    /// and a scientific dataset with the group that names its parts; the third
    /// holds a second group whose members are all a block away, and an image
    /// whose dimension record is here but whose number type record is not.
    /// Both orders are on purpose: an HDF4 writer puts a thing before the
    /// thing that describes it as often as after.
    fn file() -> Vec<u8> {
        let (v, h, r, g) = (ver(), vh(), vs(), vg());
        let (image, ntype, dims) = (vec![10u8, 11, 12, 13, 14, 15], nt(21, 8, 1), id(7));
        let (values, shape, kind, grp) = (sd(), sdd(), nt(22, 16, 4), ndg());
        let far = id(60);
        let none = Vec::new();
        let first: [(u16, u16, &Vec<u8>); 4] = [(30, 1, &v), (1963, 4, &r), (1962, 4, &h), (1965, 2, &g)];
        let second: [(u16, u16, &Vec<u8>); 7] = [
            (302, 7, &image),
            (106, 7, &ntype),
            (300, 7, &dims),
            (702, 9, &values),
            (701, 9, &shape),
            (106, 8, &kind),
            (720, 9, &grp),
        ];
        let third: [(u16, u16, &Vec<u8>); 4] =
            [(720, 11, &grp), (302, 50, &image), (300, 50, &far), (1, 0, &none)];
        let block_at = |n: usize, from: usize| from + 6 + 12 * n;
        let b1 = block_at(first.len(), 4);
        let b2 = block_at(second.len(), b1);
        let mut at = block_at(third.len(), b2) as u32;
        let mut b = MAGIC.to_vec();
        let mut data = Vec::new();
        for (block, next) in [(&first[..], b1), (&second[..], b2), (&third[..], 0)] {
            b.extend(be16(block.len() as i16));
            b.extend(be32(next as u32));
            for (tag, r, payload) in block {
                b.extend(tag.to_be_bytes());
                b.extend(r.to_be_bytes());
                if *tag == 1 {
                    b.extend(be32(0));
                    b.extend(be32(0));
                    continue;
                }
                b.extend(be32(at));
                b.extend(be32(payload.len() as u32));
                at += payload.len() as u32;
                data.extend_from_slice(payload);
            }
        }
        b.extend_from_slice(&data);
        b
    }

    fn read(at: &[usize]) -> crate::eval::NodeInfo {
        let d = Document::new(MemSource(file()));
        let mut e = Evaluator::new(hdf4());
        e.node(&d, at).unwrap()
    }

    #[test]
    fn the_blocks_are_a_flat_list_however_many_of_them_there_are() {
        let d = Document::new(MemSource(file()));
        let mut e = Evaluator::new(hdf4());
        // Three blocks side by side, not a block holding a block.
        let blocks = e.node(&d, &[1]).unwrap();
        assert_eq!(blocks.child_count, 3);
        assert_eq!(blocks.unit.as_deref(), Some("block"));
        assert_eq!(e.node(&d, &[1, 0, 0]).unwrap().value, Value::Int(4));
        assert_eq!(e.node(&d, &[1, 0, 3]).unwrap().child_count, 4);
        // The last block, found by following the chain, with its own slots. It
        // has no next of its own, so the walk stops rather than pointing back
        // at the signature.
        assert_eq!(e.node(&d, &[1, 2, 3]).unwrap().child_count, 4);
    }

    #[test]
    fn a_descriptor_is_named_by_its_tag_and_places_its_own_bytes() {
        let vgroup = read(&[1, 0, 3, 3]);
        assert_eq!(vgroup.name, "[3] vgroup");
        assert_eq!(read(&[1, 0, 3, 3, 4, 0]).size_bits, vg().len() as u64 * 8);
        // The null slot points at nothing rather than at the signature.
        assert_eq!(read(&[1, 2, 3, 3, 4]).child_count, 0);
    }

    #[test]
    fn the_version_record_is_read_and_the_rest_of_it_is_the_line_of_text() {
        assert_eq!(read(&[1, 0, 3, 0, 4, 0, 0]).value, Value::UInt(4));
        assert_eq!(read(&[1, 0, 3, 0, 4, 0, 2]).value, Value::UInt(15));
        assert_eq!(read(&[1, 0, 3, 0, 4, 0, 3]).value, Value::Str("HDF Version 4.2 Release 15".into()));
    }

    #[test]
    fn a_vdata_header_says_what_the_columns_are() {
        assert_eq!(read(&[1, 0, 3, 2, 4, 0]).type_name, "Hdf4VdataHeader");
        assert_eq!(read(&[1, 0, 3, 2, 4, 0, 1]).value, Value::UInt(4));
        assert_eq!(read(&[1, 0, 3, 2, 4, 0, 3]).value, Value::Int(1));
        let ty = read(&[1, 0, 3, 2, 4, 0, 4, 0]);
        assert_eq!(ty.value, Value::Enum { raw: 24, name: Some("int32".into()), hex: false });
        assert_eq!(read(&[1, 0, 3, 2, 4, 0, 8, 0, 1]).value, Value::Str("VALUES".into()));
        assert_eq!(read(&[1, 0, 3, 2, 4, 0, 9, 1]).value, Value::Str("attname1".into()));
        assert_eq!(read(&[1, 0, 3, 2, 4, 0, 10, 1]).value, Value::Str("Attr0.0".into()));
    }

    #[test]
    fn a_version_three_header_has_no_flags_and_no_attributes() {
        // `version`, then the flags that are not there.
        assert_eq!(read(&[1, 0, 3, 2, 4, 0, 13]).value, Value::Int(3));
        assert_eq!(read(&[1, 0, 3, 2, 4, 0, 15]).size_bits, 0);
        assert_eq!(read(&[1, 0, 3, 2, 4, 0, 16]).size_bits, 0);
        assert_eq!(read(&[1, 0, 3, 2, 4, 0, 17]).child_count, 0);
        // And the pair written again at the end of the record.
        assert_eq!(read(&[1, 0, 3, 2, 4, 0, 18]).value, Value::Int(3));
        assert_eq!(read(&[1, 0, 3, 2, 4, 0, 20]).size_bits, 8);
    }

    /// The header is the descriptor after this one, so this is the lookup
    /// running forward over the block's index rather than back over what has
    /// already been placed.
    #[test]
    fn a_tables_rows_take_their_columns_from_a_header_written_after_them() {
        let rows = read(&[1, 0, 3, 1, 4, 0, 1]);
        assert_eq!(rows.child_count, 4);
        assert_eq!(read(&[1, 0, 3, 1, 4, 0, 1, 0]).size_bits, 8 * 8);
        // One column of two int32 in each record, named as the header names it.
        assert_eq!(read(&[1, 0, 3, 1, 4, 0, 1, 2, 0]).child_count, 1);
        assert_eq!(read(&[1, 0, 3, 1, 4, 0, 1, 2, 0, 0]).name, "[0] VALUES");
        // Two per record, so the column is a pair rather than a number.
        assert_eq!(read(&[1, 0, 3, 1, 4, 0, 1, 2, 0, 0, 1]).child_count, 2);
        assert_eq!(read(&[1, 0, 3, 1, 4, 0, 1, 2, 0, 0, 1, 0]).value, Value::Int(4));
        assert_eq!(read(&[1, 0, 3, 1, 4, 0, 1, 2, 0, 0, 1, 1]).value, Value::Int(5));
    }

    /// The image comes before its dimension record too, and the number type it
    /// names is a third descriptor again.
    #[test]
    fn an_image_is_rows_of_pixels_once_its_dimension_record_is_found() {
        assert_eq!(read(&[1, 1, 3, 0, 4, 0]).type_name, "Hdf4RasterImage");
        assert_eq!(read(&[1, 1, 3, 0, 4, 0, 2, 0]).child_count, 2);
        assert_eq!(read(&[1, 1, 3, 0, 4, 0, 2, 0, 0]).child_count, 3);
        assert_eq!(read(&[1, 1, 3, 0, 4, 0, 2, 0, 1, 2, 0]).value, Value::UInt(15));
    }

    /// Two by three, and the number type says the little end comes first, so a
    /// value of 1 is written `01 00` and has to read as 1 and not as 256.
    #[test]
    fn a_dataset_takes_its_shape_from_the_dimensions_and_its_order_from_the_number_type() {
        let values = read(&[1, 1, 3, 6, 4, 0, 1, 2, 0]);
        assert_eq!(values.child_count, 2);
        assert_eq!(read(&[1, 1, 3, 6, 4, 0, 1, 2, 0, 0]).child_count, 3);
        assert_eq!(read(&[1, 1, 3, 6, 4, 0, 1, 2, 0, 0, 1]).value, Value::Int(1));
        assert_eq!(read(&[1, 1, 3, 6, 4, 0, 1, 2, 0, 1, 2]).value, Value::Int(5));
    }

    /// The same group again in the next block, where the index reaches none of
    /// its members: the pairs are still read, and the dataset is not.
    #[test]
    fn a_group_whose_members_are_in_another_block_keeps_its_pairs_and_no_more() {
        assert_eq!(read(&[1, 2, 3, 0, 4, 0, 0]).child_count, 3);
        assert_eq!(read(&[1, 2, 3, 0, 4, 0, 0, 0, 2]).value, Value::Int(0));
        assert_eq!(read(&[1, 2, 3, 0, 4, 0, 1]).size_bits, 0);
    }

    /// Which number type an image has is written in its dimension record, so
    /// an image can find how wide its rows are and still not find how wide a
    /// sample is. That is a reading that stops rather than one that fails: the
    /// bytes stay bytes, all of them.
    #[test]
    fn an_image_whose_number_type_is_a_block_away_keeps_its_bytes() {
        let image = read(&[1, 2, 3, 1, 4, 0]);
        assert_eq!(image.type_name, "Hdf4RasterImage");
        assert_eq!(read(&[1, 2, 3, 1, 4, 0, 0, 0, 0]).value, Value::Int(3), "the dimensions still read");
        let samples = read(&[1, 2, 3, 1, 4, 0, 2]);
        assert_eq!(samples.child_count, 0);
        assert_eq!(samples.size_bits, 6 * 8, "every byte of it is still named");
    }

    /// A vgroup says where each of its members is. The second names an image
    /// in the next block, which this block's index does not cover, so it comes
    /// back with no place rather than with the wrong one.
    #[test]
    fn a_vgroup_says_where_its_members_are_and_says_nothing_for_one_it_cannot_reach() {
        assert_eq!(read(&[1, 0, 3, 3, 4, 0, 3]).child_count, 2);
        let first = read(&[1, 0, 3, 3, 4, 0, 3, 0, 0]);
        assert_eq!(first.value, Value::Enum { raw: 1962, name: Some("vdata description".into()), hex: false });
        assert_eq!(read(&[1, 0, 3, 3, 4, 0, 3, 0, 3]).value, Value::Int(vh().len() as i128));
        assert_eq!(read(&[1, 0, 3, 3, 4, 0, 3, 1, 2]).value, Value::Int(0));
        assert_eq!(read(&[1, 0, 3, 3, 4, 0, 3, 1, 3]).value, Value::Int(0));
    }

    /// The index is the block's own twelve bytes read a second way, so it is a
    /// view of them and not another claim on them.
    #[test]
    fn the_index_covers_the_same_bytes_as_the_descriptors() {
        let d = Document::new(MemSource(file()));
        let mut e = Evaluator::new(hdf4());
        let table = e.node(&d, &[1, 0, 2, 0]).unwrap();
        let descriptors = e.node(&d, &[1, 0, 3]).unwrap();
        assert_eq!(table.offset_bits, descriptors.offset_bits);
        assert_eq!(table.size_bits, descriptors.size_bits);
        assert_eq!(e.node(&d, &[1, 0, 2, 0, 2, 4]).unwrap().value, Value::Int(1962 * 65536 + 4));
    }
}
