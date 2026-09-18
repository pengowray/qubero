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
//! scientific dataset's values in the shape its dimension record gives them,
//! joined back together when the library moved them into linked blocks, and
//! its labels, units and formats one per dimension, a raster image as rows
//! of pixels, a palette as colours, and a vgroup's members and attributes as
//! the places they point at.
//!
//! None of that can be read from one descriptor alone. A raster image's own
//! bytes say nothing about how wide a row is; the image dimension record with
//! the same reference number says. A table's rows say nothing about their
//! columns; the vdata header with the same reference number does. A scientific
//! dataset's values are tied to their dimensions by neither, only by the group
//! that names both. So the file's twelve-byte tables are read twice: once as
//! the descriptors, which place the bytes, and once as an index with the tag
//! and the reference number as one number, which is a list every lookup here
//! can search. The index is a second reading of the same twelve bytes, counted
//! nowhere, taking no room and pointing nowhere: it answers where a thing is,
//! and the record itself is then read beside whatever asked, so that a run of
//! bytes is always named by the descriptor that owns it and never by the
//! reading that went looking for it.
//!
//! The index is one list over every block. A file grown past sixteen
//! descriptors has more than one block, and the library writes a thing and the
//! thing that describes it wherever there is a free slot, so a group in the
//! third block names members in the second and a table's rows in the last slot
//! of one block find their header in the first slot of the next. In the sample
//! files about a tenth of the references cross a block. Where a lookup finds
//! nothing at all, the reading falls back to bytes, so a file that names a
//! thing it never wrote loses detail rather than breaking.
//!
//! Everything is big-endian, which HDF4 calls its own on-disk order whatever
//! the machine that wrote the file was, except where a number type record says
//! otherwise: class 4 is the Intel byte order for whole numbers and the PC's
//! IEEE for floating point, and a file written that way is read that way.

use super::hdf4_records::{eight_bit_dimensions, image_dimensions, number_type, number_value, palette, sd_dimensions, sd_strings, special_element, vdata_header, version, vgroup};
use crate::template::{Anchor, Encoding, Endian::Big, Expr as E, Step, StrLen, Template, Ty as T};

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

/// A descriptor's tag, named where it is one of the known ones and named as a
/// special element where it is one of those.
pub(super) fn tag() -> T {
    tag_of(u16be())
}

/// The same names, over a number that is not two bytes of the file: a vgroup
/// writes its members' tags in a run of their own, and the row that puts a tag
/// back beside its reference number has to name it too.
pub(super) fn tag_of(inner: T) -> T {
    T::enum_ranged("Hdf4Tag", inner, TAG, &[(0x4000, 1, "special element for tag {n}")])
}

pub(super) fn u16be() -> T {
    T::u16(Big)
}

pub(super) fn u32be() -> T {
    T::u32(Big)
}

pub(super) fn i16be() -> T {
    T::Int { bits: 16, endian: Big }
}

/// A length and that many bytes of text, which is how every name in a vdata
/// header is written.
pub(super) fn counted_name() -> T {
    T::structure_named(
        "Hdf4Name",
        "",
        "text",
        vec![("len", i16be()), ("text", T::utf8(E::field("len").at_least(E::lit(0))))],
    )
}

/// A descriptor's tag and reference number read as one number, which is how
/// the index holds them and the only way a search can ask for both at
/// once. See [`index_entry`].
pub(super) fn key(tag: i128, r: E) -> E {
    E::lit(tag * 65536).add(r)
}

/// Where the descriptor this key names keeps its bytes: `offset` or `length`,
/// from the file's index, whichever block the descriptor is in.
///
/// Zero when the file holds no such descriptor, which is what every guard
/// here tests. An offset of zero is the four bytes of signature and no
/// descriptor ever points there, so zero is an answer no file can give.
pub(super) fn look(k: E, path: &[&str]) -> E {
    E::tagged_by_expr("index", &["key"], k, path)
}

/// The record the descriptor this key names points at, read where it says it
/// is and no further.
///
/// A second reading of bytes the descriptor itself places, so every field
/// declared this way is put aside and counted there. Nothing but the numbers
/// in it is wanted: how wide a row is, how many dimensions there are, which
/// way round the bytes go.
pub(super) fn record_at(k: E, inner: T) -> T {
    T::at(look(k.clone(), &["offset"]), T::sized(look(k, &["length"]), inner))
}

/// The same, where the key came out of another record and so could not be
/// tested before the field was declared.
///
/// A number type record is named by a reference number written inside the
/// dimension record, and whether the file holds it cannot be asked until the
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
pub(super) fn own_ref() -> E {
    E::field("ref")
}

/// One when the file holds a descriptor with this tag and this reference
/// number, and zero otherwise.
pub(super) fn found(k: E) -> E {
    E::lit(0).less_than(look(k, &["offset"]))
}

/// Where the thing a `tag` and `ref` beside this field name is: `offset` or
/// `length`, and zero where the file holds no such thing.
///
/// Looked for under the tag as written and then under its special twin, which
/// is how the HDF4 library looks too. A group names a dataset's values as tag
/// 702 however they are kept, and values kept in linked blocks have a
/// descriptor tagged 0x4000 more than that and none tagged 702, so without the
/// second look every member naming such values points nowhere.
pub(super) fn place_of_member(path: &[&str]) -> E {
    let k = E::field("tag").mul(E::lit(65536)).add(E::field("ref"));
    look(k.clone(), path).or(look(k.add(E::lit(0x4000 * 65536)), path))
}

/// A tag and a reference number together, which is how a group names one of
/// the things in it, and where in the file that thing is.
///
/// The offset and the length take no bytes: they are the step from a name to
/// a place, so that a reader looking at a group can see what it holds rather
/// than a pair of numbers to go and find by hand.
fn tag_ref() -> T {
    T::structure_named(
        "Hdf4GroupMember",
        "tag",
        "",
        vec![
            ("tag", tag()),
            ("ref", u16be()),
            ("offset", T::computed(place_of_member(&["offset"]))),
            ("length", T::computed(place_of_member(&["length"]))),
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
    // A dimension with no scale has no bytes here. One whose number type the
    // file never wrote stops the reading where it stands: how wide its values are
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

/// A raster image, read as rows of pixels.
///
/// Nothing in the image's own bytes says how wide a row is: the image
/// dimension record with the same reference number does, and the number type
/// record that names says how wide one sample is. Where the file holds no
/// such record the bytes stay bytes.
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
            // whether the file holds it cannot be asked until that record has
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
/// The interlacing decides the nesting. A table written a record at a time is
/// a list of records, each one value per column. A table written a field at a
/// time holds every value of the first column, then every value of the second,
/// so a row is not a run of bytes at all and the table reads as a list of
/// columns, each one value per record. Anything else stays bytes.
fn vdata_records() -> T {
    let header = || key(1962, own_ref());
    let nvertices = || E::within(&["header", "nvertices"]).at_least(E::lit(0));
    let records = T::array(
        T::sized(E::within(&["header", "ivsize"]).at_least(E::lit(0)), vdata_record()),
        nvertices(),
    );
    let columns = T::structure(
        "Hdf4VdataColumns",
        vec![("columns", T::array(vdata_column(nvertices()), E::within(&["header", "nfields"]).at_least(E::lit(0))))],
    )
    .field_elem_named_from("columns", E::elem_within(&["header", "field_names"], E::idx(), &["text"]));
    let rows = T::structure(
        "Hdf4VdataRecords",
        vec![
            ("header", record_at(header(), vdata_header())),
            (
                "records",
                T::switch(E::within(&["header", "interlace"]), vec![(0, records), (1, columns)], T::bytes(E::Remaining)),
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
fn vdata_value() -> T {
    T::structure_named(
        "Hdf4VdataField",
        "",
        "value",
        vec![("column", T::computed(E::idx())), ("value", column_value())],
    )
    .machinery(&["column"])
}

/// Every value of one column, in a table written a field at a time: one per
/// record, each read the way [`vdata_value`] reads it. The column's number is
/// written down for the same reason.
fn vdata_column(records: E) -> T {
    T::structure_named(
        "Hdf4VdataColumn",
        "",
        "values",
        vec![("column", T::computed(E::idx())), ("values", T::array(column_value(), records))],
    )
    .machinery(&["column"])
}

/// One value of the column the `column` field beside or around it names.
///
/// A field type with bit 14 set is the same type written the other way round,
/// which is how a vdata written on an Intel machine and left unconverted says
/// so. A column of characters reads as one string of its order rather than as
/// that many one-byte numbers. A type nothing here knows is as many bytes as
/// the header gives one of it, which is the column's size shared out over its
/// order, so the columns after it still line up.
fn column_value() -> T {
    let col = |f: &str| E::elem_within(&["header", f], E::field("column"), &[]);
    let kind = || col("field_types").and(E::lit(0xff));
    let order = || col("field_orders").at_least(E::lit(0));
    let width = || col("field_sizes").mul(E::lit(8)).div(order().at_least(E::lit(1)));
    let one = || number_value(kind(), col("field_types").bit(14).mul(E::lit(4)), width());
    let text = || T::text(StrLen::Padded { size: order(), pad: 0 }, Encoding::Ascii);
    T::switch(
        kind(),
        vec![(3, text()), (4, text())],
        T::switch(order().equals(E::lit(1)), vec![(1, one())], T::array(one(), order())),
    )
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
///
/// A dataset that grew, which is any with an unlimited dimension, has its
/// values in linked blocks. The group still names them as tag 702, and the
/// file holds no 702 with that reference number, only its special twin
/// 0x4000 higher, pointing at the header that says where the blocks are. So
/// the values are looked for under the plain tag and then under the twin, the
/// way the library looks, and read through the header when that is where they
/// are.
fn scientific_data_group() -> T {
    let member = |t: i128| key(t, E::tagged("members", &["tag"], t, &["ref"]));
    let special = || key(0x4000 + 702, E::tagged("members", &["tag"], 702, &["ref"]));
    // The number type is named by the dimension record rather than by the
    // group: a file written by the old interface lists it among the members
    // and one written by the SD interface gives it a reference number of its
    // own, and the dimension record is right either way.
    let nt = || key(106, E::within(&["dimensions", "number_type_ref"]));
    let dim = |i: i128| E::elem_within(&["dimensions", "dims"], E::lit(i), &[]).at_least(E::lit(0));
    // The last dimension is wrapped first, so it ends up innermost and the
    // first dimension outermost, which is the order the values are written
    // in: the last index is the one that changes from one value to the next.
    //
    // A dataset that grew is counted along its first dimension by what it
    // holds rather than by its dimension record. The record is written when
    // the dataset is made, and growing an unlimited dimension does not write
    // it again: two of `tdata.hdf`'s three datasets say four records and hold
    // five, and five is what pyhdf reads and what the file's own `rec`
    // dimension says. So there the first dimension is as many records as the
    // joined values have room for, each one value wide times every other
    // dimension.
    let cases = |grown: bool| -> Vec<(i128, T)> {
        (1..=4i128)
            .map(|rank| {
                let mut shape = of_the_number_type();
                for i in (0..rank).rev() {
                    let count = if grown && i == 0 {
                        let width = E::within(&["number_type", "width"]).div(E::lit(8)).at_least(E::lit(1));
                        let record = (1..rank).fold(width, |n, j| n.mul(dim(j)));
                        E::Remaining.div(record.at_least(E::lit(1)))
                    } else {
                        dim(i)
                    };
                    shape = T::array(shape, count);
                }
                (rank, shape)
            })
            .collect()
    };
    let shape = |grown: bool| T::switch(E::within(&["dimensions", "rank"]), cases(grown), T::bytes(E::Remaining));
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
                        T::switch(
                            found(member(702)),
                            vec![(1, record_at(member(702), shape(false)))],
                            record_at(special(), special_element(shape(true))),
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
                    found(member(701)).mul(found(member(702)).or(found(special()))),
                    vec![(1, dataset)],
                    T::bytes(E::lit(0)),
                ),
            ),
        ],
    )
}

/// What a descriptor's bytes are read as.
///
/// A tag on its own says what a run of bytes is for, not how to read it. The
/// ones here are the ones a reader can take apart with the file's index in
/// hand: a raster image asks the image dimension record with its own reference
/// number how wide the rows are, a table's rows ask the vdata header with the
/// same number what the columns are, and a scientific dataset's values are
/// reached from the group that names the dimension record and the data
/// together. Where the file holds no descriptor with that tag and number, the
/// reading falls back to the bytes.
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
        T::switch(E::field("tag").bit(14), vec![(1, special_element(T::bytes(E::Remaining)))], T::bytes(E::Remaining)),
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

/// The same twelve bytes as a descriptor, read a second time as an entry in
/// an index the rest of the template can search.
///
/// A lookup reaches a list by naming it, and a list cannot name itself: a name
/// reaches the fields declared before the field asking, and the descriptors
/// are the field the asking descriptor sits inside. So the file reads its
/// tables twice over, once for the index and once for the descriptors, and
/// the index comes first. See [`hdf4`].
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
    ).named_by("tag")
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

/// The same block again, read for its table and nothing else: the count, the
/// next, and each descriptor's twelve bytes. See [`hdf4`].
///
/// The table is read where the block is and takes no room there, which is
/// what keeps the block itself the thing a byte of it belongs to. The cursor
/// finds a byte outside the root by asking what was placed over it, and a
/// reading that covers exactly the bytes a block covers would be the one it
/// found, since the first placement over a stretch is the one kept, and the
/// descriptors would never be reached from their own bytes.
///
/// The table is placed at its own start rather than from an origin, because
/// the index places each entry where `start_of` says its descriptor is, and
/// `start_of` counts from the nearest origin: inside one, every entry would
/// land that many bytes early.
fn index_block() -> T {
    let table = T::structure(
        "Hdf4IndexTable",
        vec![
            ("ndd", i16be()),
            ("next", u32be()),
            ("entries", T::array(index_entry(), E::field("ndd").at_least(E::lit(0)))),
        ],
    )
    .machinery(&["next"]);
    let here = T::At { anchor: Anchor::SelfAligned(1), at: E::lit(0), inner: Box::new(table) };
    T::structure("Hdf4IndexBlock", vec![("table", here)])
}

/// One entry of the file's index: a descriptor's key, and where its bytes are.
///
/// Every number is asked of the entry in the table that placed this one, and
/// none is read here: the entry takes no room, so that nothing but the
/// descriptor owns the twelve bytes it sits on. See [`index_block`].
fn gathered_entry() -> T {
    let from = |f: &str| T::computed(E::placer(E::field(f)));
    T::structure(
        "Hdf4IndexEntry",
        vec![("key", from("key")), ("offset", from("offset")), ("length", from("length"))],
    )
}

pub fn hdf4() -> Template {
    // The first block sits straight after the signature, and each one says
    // where the next is. A list, rather than a block that holds the block that
    // holds the block: a file that has grown a dozen times is a dozen rows.
    let chain = |next: &[&str], elem: &str| T::chain(E::size_of("magic"), next, Anchor::File, T::Named(elem.into()));
    // A thing in one block is as often named by a thing in another as by one
    // beside it: a group written when its block was full lists members in the
    // block before, and a table's rows can be the last slot of one block and
    // their header the first slot of the next. A search runs over one list,
    // and the blocks are a list of lists, so the index is every block's table
    // flattened into one: the chain walked once for the tables alone, and a
    // gather that steps into each table and places one entry on each
    // descriptor's twelve bytes. Both come before `blocks`, which is what lets
    // a descriptor name them, and neither takes any room: the descriptors are
    // what those bytes are.
    let index = T::gather(
        vec![Step::field("tables"), Step::each(), Step::field("table"), Step::field("entries"), Step::each()],
        E::start_of(E::field("tag")),
        Anchor::File,
        E::lit(0),
        gathered_entry(),
    );
    let root = T::structure(
        "Hdf4",
        vec![
            ("magic", T::magic(MAGIC)),
            ("tables", chain(&["table", "next"], "Hdf4IndexBlock")),
            ("index", index),
            ("blocks", chain(&["next"], "Hdf4DescriptorBlock")),
        ],
    )
    .machinery(&["tables", "index"])
    .field_aside("tables")
    .field_aside("index");
    Template::new("hdf4", root)
        .with_type("Hdf4IndexBlock", index_block())
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

    /// A version 4 vgroup naming the table in its own block, the image in the
    /// next one, and values two blocks on that are kept in linked blocks and so
    /// have only a special element's tag, with one attribute whose value is
    /// the second table's header, two blocks on.
    fn vg() -> Vec<u8> {
        let mut v = be16(3);
        v.extend(be16(1962));
        v.extend(be16(302));
        v.extend(be16(702));
        v.extend(be16(4));
        v.extend(be16(7));
        v.extend(be16(13));
        v.extend(nm("grp"));
        v.extend(nm(""));
        v.extend(be16(0)); // extension tag
        v.extend(be16(0)); // extension ref
        v.extend(be32(1)); // flags: has attributes
        v.extend(be32(1)); // one of them
        v.extend(be16(1962));
        v.extend(be16(12));
        v.extend(be16(4)); // version, last
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

    /// The header of values kept in one linked block, as the special element
    /// descriptor for them points at.
    fn linked() -> Vec<u8> {
        let mut v = be16(1); // linked blocks
        v.extend(be32(12)); // twelve bytes of values
        v.extend(be32(12)); // in blocks of twelve
        v.extend(be32(1)); // one block
        v.extend(be16(14)); // whose table is reference number 14
        v
    }

    /// A table of three records written a field at a time, header and rows, as
    /// pyhdf's library wrote them with `NO_INTERLACE`: an int16 `a`, a pair of
    /// float32 `b`, and three characters `c`.
    fn by_field() -> (Vec<u8>, Vec<u8>) {
        let hex = |s: &str| (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect();
        (
            hex("000100000003000d000300160005000400020008000300000002000a000100020003000161000162000163\
                 000762796669656c64000000000000000300000003000000"),
            hex("0001000200033fc0000040200000406000004090000040b0000040d00000616263646566676869"),
        )
    }

    /// A file of three blocks. The first holds the version record, a table's
    /// rows and then the header that describes them, a vgroup, and a second
    /// table's rows whose header is in the third block; the second holds an
    /// image before the dimension record that says how wide it is, and a
    /// scientific dataset with the group that names its parts and its labels;
    /// the third holds a second group whose members are all a block back, an
    /// image whose dimension record is here but whose number type record is
    /// nowhere, an empty slot, that header, a special element, units for a
    /// dataset the file has no dimension record for, and a table written a
    /// field at a time. Both orders are on
    /// purpose: an HDF4 writer puts a thing before the thing that describes it
    /// as often as after, and in whatever block has room.
    fn file() -> Vec<u8> {
        let (v, h, r, g) = (ver(), vh(), vs(), vg());
        let (image, ntype, dims) = (vec![10u8, 11, 12, 13, 14, 15], nt(21, 8, 1), id(7));
        let (values, shape, kind, grp) = (sd(), sdd(), nt(22, 16, 4), ndg());
        let (far, special) = (id(60), linked());
        // Labels for the two by three dataset, and units for a dataset the
        // file has no dimension record for.
        let (labels, units) = (b"values\0row\0column\0".to_vec(), b"\0m\0s\0".to_vec());
        let (by_field_header, by_field_rows) = by_field();
        let none = Vec::new();
        let first: [(u16, u16, &Vec<u8>); 5] =
            [(30, 1, &v), (1963, 4, &r), (1962, 4, &h), (1965, 2, &g), (1963, 12, &r)];
        let second: [(u16, u16, &Vec<u8>); 8] = [
            (302, 7, &image),
            (106, 7, &ntype),
            (300, 7, &dims),
            (702, 9, &values),
            (701, 9, &shape),
            (106, 8, &kind),
            (720, 9, &grp),
            (704, 9, &labels),
        ];
        let third: [(u16, u16, &Vec<u8>); 9] = [
            (720, 11, &grp),
            (302, 50, &image),
            (300, 50, &far),
            (1, 0, &none),
            (1962, 12, &h),
            (0x4000 + 702, 13, &special),
            (705, 77, &units),
            (1963, 20, &by_field_rows),
            (1962, 20, &by_field_header),
        ];
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
        let blocks = e.node(&d, &[3]).unwrap();
        assert_eq!(blocks.child_count, 3);
        assert_eq!(blocks.unit.as_deref(), Some("block"));
        assert_eq!(e.node(&d, &[3, 0, 0]).unwrap().value, Value::Int(5));
        assert_eq!(e.node(&d, &[3, 0, 2]).unwrap().child_count, 5);
        // The last block, found by following the chain, with its own slots. It
        // has no next of its own, so the walk stops rather than pointing back
        // at the signature.
        assert_eq!(e.node(&d, &[3, 2, 2]).unwrap().child_count, 9);
    }

    #[test]
    fn a_descriptor_is_named_by_its_tag_and_places_its_own_bytes() {
        let vgroup = read(&[3, 0, 2, 3]);
        assert_eq!(vgroup.name, "[3] vgroup");
        assert_eq!(read(&[3, 0, 2, 3, 4, 0]).size_bits, vg().len() as u64 * 8);
        // The null slot points at nothing rather than at the signature.
        assert_eq!(read(&[3, 2, 2, 3, 4]).child_count, 0);
    }

    #[test]
    fn the_version_record_is_read_and_the_rest_of_it_is_the_line_of_text() {
        assert_eq!(read(&[3, 0, 2, 0, 4, 0, 0]).value, Value::UInt(4));
        assert_eq!(read(&[3, 0, 2, 0, 4, 0, 2]).value, Value::UInt(15));
        assert_eq!(read(&[3, 0, 2, 0, 4, 0, 3]).value, Value::Str("HDF Version 4.2 Release 15".into()));
    }

    #[test]
    fn a_vdata_header_says_what_the_columns_are() {
        assert_eq!(read(&[3, 0, 2, 2, 4, 0]).type_name, "Hdf4VdataHeader");
        assert_eq!(read(&[3, 0, 2, 2, 4, 0, 1]).value, Value::UInt(4));
        assert_eq!(read(&[3, 0, 2, 2, 4, 0, 3]).value, Value::Int(1));
        let ty = read(&[3, 0, 2, 2, 4, 0, 4, 0]);
        assert_eq!(ty.value, Value::Enum { raw: 24, name: Some("int32".into()), hex: false });
        assert_eq!(read(&[3, 0, 2, 2, 4, 0, 8, 0, 1]).value, Value::Str("VALUES".into()));
        assert_eq!(read(&[3, 0, 2, 2, 4, 0, 9, 1]).value, Value::Str("attname1".into()));
        assert_eq!(read(&[3, 0, 2, 2, 4, 0, 10, 1]).value, Value::Str("Attr0.0".into()));
    }

    #[test]
    fn a_version_three_header_has_no_flags_and_no_attributes() {
        // `version`, then the flags that are not there.
        assert_eq!(read(&[3, 0, 2, 2, 4, 0, 13]).value, Value::Int(3));
        assert_eq!(read(&[3, 0, 2, 2, 4, 0, 15]).size_bits, 0);
        assert_eq!(read(&[3, 0, 2, 2, 4, 0, 16]).size_bits, 0);
        assert_eq!(read(&[3, 0, 2, 2, 4, 0, 17]).child_count, 0);
        // And the pair written again at the end of the record.
        assert_eq!(read(&[3, 0, 2, 2, 4, 0, 18]).value, Value::Int(3));
        assert_eq!(read(&[3, 0, 2, 2, 4, 0, 20]).size_bits, 8);
    }

    /// The header is the descriptor after this one, so this is the lookup
    /// running forward over the index rather than back over what has already
    /// been placed.
    #[test]
    fn a_tables_rows_take_their_columns_from_a_header_written_after_them() {
        let rows = read(&[3, 0, 2, 1, 4, 0, 1]);
        assert_eq!(rows.child_count, 4);
        assert_eq!(read(&[3, 0, 2, 1, 4, 0, 1, 0]).size_bits, 8 * 8);
        // One column of two int32 in each record, named as the header names it.
        assert_eq!(read(&[3, 0, 2, 1, 4, 0, 1, 2, 0]).child_count, 1);
        assert_eq!(read(&[3, 0, 2, 1, 4, 0, 1, 2, 0, 0]).name, "[0] VALUES");
        // Two per record, so the column is a pair rather than a number.
        assert_eq!(read(&[3, 0, 2, 1, 4, 0, 1, 2, 0, 0, 1]).child_count, 2);
        assert_eq!(read(&[3, 0, 2, 1, 4, 0, 1, 2, 0, 0, 1, 0]).value, Value::Int(4));
        assert_eq!(read(&[3, 0, 2, 1, 4, 0, 1, 2, 0, 0, 1, 1]).value, Value::Int(5));
    }

    /// The second table's rows are the last slot of the first block, and the
    /// header they need is in the third: forward, and past a whole block that
    /// holds neither.
    #[test]
    fn a_tables_rows_find_their_header_two_blocks_on() {
        assert_eq!(read(&[3, 0, 2, 4, 4, 0]).type_name, "Hdf4VdataRecords");
        assert_eq!(read(&[3, 0, 2, 4, 4, 0, 1]).child_count, 4);
        assert_eq!(read(&[3, 0, 2, 4, 4, 0, 1, 3, 0, 0]).name, "[0] VALUES");
        assert_eq!(read(&[3, 0, 2, 4, 4, 0, 1, 3, 0, 0, 1, 1]).value, Value::Int(7));
    }

    /// A table written a field at a time holds every `a`, then every pair of
    /// `b`, then every `c`, so it reads as columns, each one value per record
    /// and named as the header names it. The values are the ones pyhdf wrote
    /// and reads back.
    #[test]
    fn a_table_written_a_field_at_a_time_reads_as_columns() {
        assert_eq!(read(&[3, 2, 2, 7, 4, 0, 1]).type_name, "Hdf4VdataColumns");
        let columns = read(&[3, 2, 2, 7, 4, 0, 1, 0]);
        assert_eq!(columns.child_count, 3);
        assert_eq!(read(&[3, 2, 2, 7, 4, 0, 1, 0, 1]).name, "[1] b");
        assert_eq!(read(&[3, 2, 2, 7, 4, 0, 1, 0, 0, 1]).child_count, 3);
        assert_eq!(read(&[3, 2, 2, 7, 4, 0, 1, 0, 0, 1, 2]).value, Value::Int(3));
        assert_eq!(read(&[3, 2, 2, 7, 4, 0, 1, 0, 1, 1, 2, 1]).value, Value::Float(6.5));
        assert_eq!(read(&[3, 2, 2, 7, 4, 0, 1, 0, 2, 1, 1]).value, Value::Str("def".into()));
        assert_eq!(read(&[3, 2, 2, 7, 4, 0, 1, 0, 2]).size_bits, 9 * 8, "three records of three characters");
    }

    /// The image comes before its dimension record too, and the number type it
    /// names is a third descriptor again.
    #[test]
    fn an_image_is_rows_of_pixels_once_its_dimension_record_is_found() {
        assert_eq!(read(&[3, 1, 2, 0, 4, 0]).type_name, "Hdf4RasterImage");
        assert_eq!(read(&[3, 1, 2, 0, 4, 0, 2, 0]).child_count, 2);
        assert_eq!(read(&[3, 1, 2, 0, 4, 0, 2, 0, 0]).child_count, 3);
        assert_eq!(read(&[3, 1, 2, 0, 4, 0, 2, 0, 1, 2, 0]).value, Value::UInt(15));
    }

    /// Two by three, and the number type says the little end comes first, so a
    /// value of 1 is written `01 00` and has to read as 1 and not as 256.
    #[test]
    fn a_dataset_takes_its_shape_from_the_dimensions_and_its_order_from_the_number_type() {
        let values = read(&[3, 1, 2, 6, 4, 0, 1, 2, 0]);
        assert_eq!(values.child_count, 2);
        assert_eq!(read(&[3, 1, 2, 6, 4, 0, 1, 2, 0, 0]).child_count, 3);
        assert_eq!(read(&[3, 1, 2, 6, 4, 0, 1, 2, 0, 0, 1]).value, Value::Int(1));
        assert_eq!(read(&[3, 1, 2, 6, 4, 0, 1, 2, 0, 1, 2]).value, Value::Int(5));
    }

    /// A dataset's labels are its own and then one per dimension, and the
    /// dimension record with the same reference number says how many that is.
    #[test]
    fn labels_are_the_datasets_own_and_then_one_per_dimension() {
        assert_eq!(read(&[3, 1, 2, 7, 4, 0]).type_name, "Hdf4SdStrings");
        assert_eq!(read(&[3, 1, 2, 7, 4, 0, 1]).value, Value::Str("values".into()));
        assert_eq!(read(&[3, 1, 2, 7, 4, 0, 2]).child_count, 2);
        assert_eq!(read(&[3, 1, 2, 7, 4, 0, 2, 0]).value, Value::Str("row".into()));
        assert_eq!(read(&[3, 1, 2, 7, 4, 0, 2, 1]).value, Value::Str("column".into()));
        assert_eq!(read(&[3, 1, 2, 7, 4, 0, 2, 1]).size_bits, 7 * 8, "the nul is part of each");
    }

    /// Units whose dimension record the file never wrote are still every
    /// string in the record, the first of them the dataset's own, which here
    /// is empty.
    #[test]
    fn units_with_no_dimension_record_are_a_list_of_strings() {
        assert_eq!(read(&[3, 2, 2, 6, 4, 0]).type_name, "Hdf4SdStringList");
        assert_eq!(read(&[3, 2, 2, 6, 4, 0, 0]).child_count, 3);
        assert_eq!(read(&[3, 2, 2, 6, 4, 0, 0, 0]).value, Value::Str("".into()));
        assert_eq!(read(&[3, 2, 2, 6, 4, 0, 0, 2]).value, Value::Str("s".into()));
    }

    /// The same group again in the next block, with every member it names a
    /// block back: each pair says where its member is, and the dataset they
    /// make up is read from there.
    #[test]
    fn a_group_reaches_members_a_block_back() {
        assert_eq!(read(&[3, 2, 2, 0, 4, 0, 0]).child_count, 3);
        let values_at = read(&[3, 1, 2, 3, 2]).value.as_int();
        assert_eq!(read(&[3, 2, 2, 0, 4, 0, 0, 0, 2]).value.as_int(), values_at);
        assert_eq!(read(&[3, 2, 2, 0, 4, 0, 1]).type_name, "Hdf4ScientificDataset");
        assert_eq!(read(&[3, 2, 2, 0, 4, 0, 1, 2, 0, 1, 2]).value, Value::Int(5));
    }

    /// Which number type an image has is written in its dimension record, so
    /// an image can find how wide its rows are and still not find how wide a
    /// sample is, where the file never wrote that record. That is a reading
    /// that stops rather than one that fails: the bytes stay bytes, all of
    /// them.
    #[test]
    fn an_image_whose_number_type_is_nowhere_keeps_its_bytes() {
        let image = read(&[3, 2, 2, 1, 4, 0]);
        assert_eq!(image.type_name, "Hdf4RasterImage");
        assert_eq!(read(&[3, 2, 2, 1, 4, 0, 0, 0, 0]).value, Value::Int(3), "the dimensions still read");
        let samples = read(&[3, 2, 2, 1, 4, 0, 2]);
        assert_eq!(samples.child_count, 0);
        assert_eq!(samples.size_bits, 6 * 8, "every byte of it is still named");
    }

    /// A vgroup says where each of its members is. The first is in its own
    /// block and the second is an image in the next.
    #[test]
    fn a_vgroup_says_where_its_members_are_whichever_block_they_are_in() {
        assert_eq!(read(&[3, 0, 2, 3, 4, 0, 3]).child_count, 3);
        let first = read(&[3, 0, 2, 3, 4, 0, 3, 0, 0]);
        assert_eq!(first.value, Value::Enum { raw: 1962, name: Some("vdata description".into()), hex: false });
        assert_eq!(read(&[3, 0, 2, 3, 4, 0, 3, 0, 3]).value, Value::Int(vh().len() as i128));
        let image_at = read(&[3, 1, 2, 0, 2]).value.as_int();
        assert_eq!(read(&[3, 0, 2, 3, 4, 0, 3, 1, 2]).value.as_int(), image_at);
        assert_eq!(read(&[3, 0, 2, 3, 4, 0, 3, 1, 3]).value, Value::Int(6));
    }

    /// A version 4 vgroup writes its flags and attributes straight after the
    /// extension pair and its version last, which is not where a vdata header
    /// writes them. Read the vdata way, the flag word would be a version of 0
    /// and a `more` of 1, and the attribute would be lost in the terminator.
    #[test]
    fn a_vgroup_keeps_its_version_last_and_its_attributes_before_it() {
        let flags = read(&[3, 0, 2, 3, 4, 0, 8]);
        assert_eq!((flags.value, flags.size_bits), (Value::UInt(1), 32));
        assert_eq!(read(&[3, 0, 2, 3, 4, 0, 10]).child_count, 1);
        assert_eq!(read(&[3, 0, 2, 3, 4, 0, 10, 0, 1]).value, Value::UInt(12));
        let header_at = read(&[3, 2, 2, 4, 2]).value.as_int();
        assert_eq!(read(&[3, 0, 2, 3, 4, 0, 10, 0, 2]).value.as_int(), header_at);
        assert_eq!(read(&[3, 0, 2, 3, 4, 0, 11]).value, Value::Int(4));
        assert_eq!(read(&[3, 0, 2, 3, 4, 0, 13]).size_bits, 8);
    }

    /// A version 3 vgroup has no flag word: the five bytes after the extension
    /// pair are the version, `more` and the nul.
    #[test]
    fn a_version_three_vgroup_has_no_flags() {
        // The same vgroup with the flag word, the count, the attribute and
        // the version 4 taken off the end, and a version 3 put back.
        let mut old = vg();
        old.truncate(old.len() - 17);
        old.extend(be16(3));
        old.extend(be16(0));
        old.push(0);
        let doc = Document::new(MemSource(old));
        let mut ev = Evaluator::new(Template::new("vgroup", vgroup()));
        assert_eq!(ev.node(&doc, &[8]).unwrap().size_bits, 0);
        assert_eq!(ev.node(&doc, &[10]).unwrap().child_count, 0);
        assert_eq!(ev.node(&doc, &[11]).unwrap().value, Value::Int(3));
        assert_eq!(ev.node(&doc, &[13]).unwrap().size_bits, 8);
    }

    /// The vgroup's third member names values as tag 702, and the file holds
    /// them only as a special element, 0x4000 more. The member is found under
    /// that tag, which is where the library looks when the plain one is not
    /// there, and it points at the special element's header.
    #[test]
    fn a_member_kept_as_a_special_element_is_found_under_that_tag() {
        assert_eq!(read(&[3, 2, 2, 5, 4, 0]).type_name, "Hdf4SpecialElement");
        let special_at = read(&[3, 2, 2, 5, 2]).value.as_int();
        assert_eq!(read(&[3, 0, 2, 3, 4, 0, 3, 2, 2]).value.as_int(), special_at);
        assert_eq!(read(&[3, 0, 2, 3, 4, 0, 3, 2, 3]).value, Value::Int(linked().len() as i128));
    }

    /// A run kept in three linked blocks across two link tables, written to the
    /// file in none of the orders it goes in: the tables are a chain whose
    /// next is a reference number looked up in the index, the blocks are
    /// joined in the order the tables list them, and the run is cut at the
    /// length the header gives, short of the last block's end.
    #[test]
    fn values_in_linked_blocks_are_joined_across_every_link_table() {
        let (a, b, c) = (b"first ".to_vec(), b"second".to_vec(), b" third and unused".to_vec());
        let mut header = be16(1); // linked blocks
        header.extend(be32(18)); // eighteen bytes of run
        header.extend(be32(6)); // in blocks of six
        header.extend(be32(2)); // two slots a table
        header.extend(be16(6)); // the first table
        let mut first_table = be16(8); // the next table
        first_table.extend(be16(9)); // a
        first_table.extend(be16(7)); // b
        let mut second_table = be16(0); // no table after it
        second_table.extend(be16(10)); // c
        second_table.extend(be16(0)); // and a slot not used
        let items: [(u16, u16, &Vec<u8>); 6] = [
            (0x4000 + 702, 5, &header),
            (20, 6, &first_table),
            (20, 7, &b),
            (20, 8, &second_table),
            (20, 10, &c),
            (20, 9, &a),
        ];
        let mut file = MAGIC.to_vec();
        file.extend(be16(items.len() as i16));
        file.extend(be32(0));
        let mut at = (file.len() + 12 * items.len()) as u32;
        let mut data = Vec::new();
        for (tag, r, payload) in items {
            file.extend(tag.to_be_bytes());
            file.extend(r.to_be_bytes());
            file.extend(be32(at));
            file.extend(be32(payload.len() as u32));
            at += payload.len() as u32;
            data.extend_from_slice(payload);
        }
        file.extend_from_slice(&data);
        let d = Document::new(MemSource(file));
        let mut e = Evaluator::new(hdf4());
        let linked = [3, 0, 2, 0, 4, 0, 1];
        assert_eq!(e.node(&d, &linked).unwrap().type_name, "Hdf4LinkedBlocks");
        assert_eq!(e.node(&d, &[&linked[..], &[4]].concat()).unwrap().child_count, 2, "two tables, the second found by reference");
        let values = [&linked[..], &[5, 0]].concat();
        let node = e.node(&d, &values).unwrap();
        assert!(node.joined);
        assert_eq!(e.field_bytes(&d, &values, 64).unwrap().0, b"first second third");
        // Three parts: the unused slot is past the cut, and goes with it.
        let hit = e.part_of(&d, node.space, 12).unwrap().unwrap();
        assert_eq!((hit.index, hit.parts, hit.label.as_str()), (2, 3, "tables[1].blocks[0]"));
        // And the run says what cut it, as a formula and as the field.
        let cut: Vec<_> = e
            .relations(&d, &values)
            .unwrap()
            .into_iter()
            .filter(|r| r.role == crate::eval::Role::Length)
            .map(|r| (r.written, r.substituted, r.result))
            .collect();
        assert_eq!(cut.len(), 1, "{cut:?}");
        assert!(cut[0].0.contains("length") && cut[0].1.contains("18") && cut[0].2 == "18", "{cut:?}");
        let length = e.origins(&d, &values).unwrap().into_iter().find(|o| o.role == crate::eval::Role::Length).unwrap();
        assert_eq!((length.label.as_str(), length.path), ("length", [&linked[..], &[0]].concat()));
    }

    /// The index is every block's twelve bytes read a second way, as one list,
    /// and it takes none of those bytes: a byte of a descriptor belongs to the
    /// descriptor, not to the reading that goes looking for it.
    #[test]
    fn the_index_is_one_list_over_every_block_and_owns_no_bytes() {
        let d = Document::new(MemSource(file()));
        let mut e = Evaluator::new(hdf4());
        assert_eq!(e.node(&d, &[2]).unwrap().child_count, 5 + 8 + 9);
        // The sixth entry is the first descriptor of the second block, and sits
        // on its bytes without covering them.
        let entry = e.node(&d, &[2, 5]).unwrap();
        let descriptor = e.node(&d, &[3, 1, 2, 0]).unwrap();
        assert_eq!(entry.offset_bits, descriptor.offset_bits);
        assert_eq!(entry.size_bits, 0);
        assert_eq!(e.node(&d, &[2, 5, 0]).unwrap().value, Value::Int(302 * 65536 + 7));
        assert_eq!(e.node(&d, &[2, 5, 1]).unwrap().value.as_int(), e.node(&d, &[3, 1, 2, 0, 2]).unwrap().value.as_int());
        // A byte of that descriptor is found as the descriptor.
        let at = e.locate(&d, descriptor.offset_bits + 8).unwrap();
        assert_eq!(&at[..4], &[3, 1, 2, 0]);
    }
}
