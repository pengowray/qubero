//! Items: what HEIF and AVIF keep their pictures in, inside the file's `meta`.
//!
//! A still image is not a track of samples but an item, numbered by its item
//! ID. `iinf` says what type each item is, `iloc` where its bytes are, `iprp`
//! which properties (size, colour, codec configuration) it has, `iref` how
//! items refer to each other (an alpha plane to its picture, a thumbnail to
//! the full image), and `pitm` which item is the picture to show.
//!
//! `iloc` places each extent of each item over the `mdat` (or the `idat`) it
//! is in, so the bytes of an AVIF read as the item they belong to.

use super::{bits, child_boxes, fourcc, full_box, full_box_flags, media, u16be, u32be};
use crate::template::{Endian::*, Expr as E, Ty as T, Until};

pub(super) fn boxes() -> Vec<(&'static str, T)> {
    vec![
        ("pitm", pitm()),
        ("iloc", iloc()),
        ("iinf", iinf()),
        ("infe", infe()),
        ("iref", iref()),
        ("ipma", ipma()),
        ("ispe", ispe()),
        ("pixi", pixi()),
        ("auxC", auxc()),
        ("irot", T::structure("ImageRotation", vec![("reserved", bits(6)), ("angle", bits(2))]).field_doc("angle", "How far to turn the picture anticlockwise, in quarter turns.")),
        ("imir", T::structure("ImageMirror", vec![("reserved", bits(7)), ("axis", T::enumeration("MirrorAxis", bits(1), &[(0, "vertical"), (1, "horizontal")]))])),
        ("idat", T::structure("ItemData", vec![("data", T::bytes(E::Remaining))])),
        ("grpl", child_boxes()),
    ]
}

/// An item ID: 16 bits in version 0 of most item boxes, 32 after.
fn item_id(wide_from: i128) -> T {
    T::switch(E::field("version").greater_or_equal(E::lit(wide_from)), vec![(1, u32be())], u16be())
}

fn pitm() -> T {
    let mut fields = full_box();
    fields.push(("item_id", item_id(1)));
    T::structure("PrimaryItem", fields).field_doc("item_id", "The primary item: the picture a viewer shows for this file.")
}

/// A number `field` bytes wide, or nothing when that is 0.
fn sized_by(field: &str) -> T {
    T::when(E::field(field).greater_than(E::lit(0)), T::uint_expr(E::field(field).mul(E::lit(8)), Big))
}

fn or_zero(field: &str, size: &str) -> E {
    E::cond(E::field(size).greater_than(E::lit(0)), E::field(field), E::lit(0))
}

/// The type of the item whose ID is `item_id`, from its `infe` in the
/// `iinf` beside this box: 0 where there is none.
fn item_type() -> E {
    E::tagged_in_by(
        E::tagged("boxes", &["type"], super::cc("iinf"), &["body", "entries"]),
        &["body", "item_id"],
        E::field("item_id"),
        &["body", "item_type"],
    )
}

/// What an item's bytes are, by its type.
fn item_content() -> T {
    T::switch(
        item_type(),
        vec![
            (super::cc("av01"), media::obus()),
            (
                super::cc("Exif"),
                T::structure("ExifItem", vec![("tiff_header_offset", u32be()), ("exif", T::bytes(E::Remaining))])
                    .field_doc("tiff_header_offset", "How many bytes after this field the TIFF header of the Exif data starts."),
            ),
        ],
        T::bytes(E::Remaining),
    )
}

/// `iloc`: where each item's bytes are, as extents.
fn iloc() -> T {
    let v12 = E::field("version").greater_or_equal(E::lit(1));
    let extent = T::structure(
        "ItemExtent",
        vec![
            ("extent_index", T::when(v12.clone().both(E::field("index_size").greater_than(E::lit(0))), T::uint_expr(E::field("index_size").mul(E::lit(8)), Big))),
            ("extent_offset", sized_by("offset_size")),
            ("extent_length", sized_by("length_size")),
            (
                "data",
                T::when(
                    E::cond(v12.clone(), E::field("construction_method"), E::lit(0))
                        .equal_to(E::lit(0))
                        .both(or_zero("extent_length", "length_size").greater_than(E::lit(0))),
                    T::at(
                        or_zero("base_offset", "base_offset_size").add(or_zero("extent_offset", "offset_size")),
                        T::sized(E::field("extent_length"), item_content()),
                    ),
                ),
            ),
        ],
    )
    .field_doc("extent_offset", "Where the extent starts, counted from base_offset.")
    .field_doc("data", "The extent's bytes, where extent_offset says they are.");
    let item = T::structure(
        "ItemLocation",
        vec![
            ("item_id", item_id(2)),
            ("item_type", T::when(item_type().not_equal(E::lit(0)), T::computed_text(item_type()))),
            ("reserved", T::when(v12.clone(), bits(12))),
            ("construction_method", T::when(v12, T::enumeration("ConstructionMethod", bits(4), &[(0, "file offset"), (1, "idat offset"), (2, "item offset")]))),
            ("data_reference_index", u16be()),
            ("base_offset", sized_by("base_offset_size")),
            ("extent_count", u16be()),
            ("extents", T::array(extent, E::field("extent_count"))),
        ],
    )
    .named_by("item_type|item_id")
    .field_doc("construction_method", "Where the offsets count from: the file, the idat box, or another item.")
    .field_doc("data_reference_index", "Which entry of dref says where the data is, counting from 1; 0 means this file.");
    let mut fields = full_box();
    fields.extend(vec![
        ("offset_size", bits(4)),
        ("length_size", bits(4)),
        ("base_offset_size", bits(4)),
        ("index_size", bits(4)),
        ("item_count", T::switch(E::field("version").less_than(E::lit(2)), vec![(1, u16be())], u32be())),
        ("items", T::array(item, E::field("item_count"))),
    ]);
    T::structure("ItemLocations", fields)
        .field_doc("offset_size", "How many bytes each extent_offset has: 0, 4 or 8. The next three fields give the same for extent_length, base_offset and extent_index.")
        .field_doc("index_size", "How many bytes each extent_index has: 0, 4 or 8. Reserved in version 0.")
}

fn iinf() -> T {
    let mut fields = full_box();
    fields.extend(vec![
        ("entry_count", T::switch(E::field("version").equal_to(E::lit(0)), vec![(1, u16be())], u32be())),
        ("entries", child_boxes()),
    ]);
    T::structure("ItemInfo", fields)
}

/// `infe`: one item's ID, type and name.
fn infe() -> T {
    let v2 = E::field("version").greater_or_equal(E::lit(2));
    let mime = E::field("item_type").equal_to(E::lit(super::cc("mime")));
    let uri = E::field("item_type").equal_to(E::lit(super::cc("uri ")));
    let mut fields = full_box_flags("ItemInfoFlags", &[(0, "hidden")]);
    fields.extend(vec![
        ("item_id", T::switch(E::field("version").equal_to(E::lit(3)), vec![(1, u32be())], u16be())),
        ("item_protection_index", u16be()),
        ("item_type", T::when(v2.clone(), fourcc())),
        ("item_name", T::cstr()),
        ("content_type", T::when(v2.clone().negate().either(mime.clone()), T::cstr())),
        ("content_encoding", T::when(v2.clone().negate().either(mime).both(E::Remaining.greater_than(E::lit(0))), T::cstr())),
        ("item_uri_type", T::when(v2.both(uri), T::cstr())),
    ]);
    T::structure("ItemInfoEntry", fields)
        .named_by("item_type")
        .field_doc("item_type", "av01: an AV1 picture. hvc1: an HEVC picture. grid: a picture made of other pictures. Exif or mime: metadata.")
        .field_doc("item_protection_index", "0: not protected.")
}

fn iref() -> T {
    let reference = T::structure(
        "ItemReference",
        vec![
            ("size", u32be()),
            ("reference_type", fourcc()),
            ("from_item_id", item_id(1)),
            ("reference_count", u16be()),
            ("to_item_ids", T::array(item_id(1), E::field("reference_count"))),
        ],
    )
    .named_by("reference_type")
    .field_doc("reference_type", "auxl: an auxiliary image such as an alpha plane. thmb: a thumbnail. cdsc: metadata describing the item. dimg: the pictures a grid is made of.");
    let mut fields = full_box();
    fields.push(("references", T::repeat(reference, Until::End)));
    T::structure("ItemReferences", fields)
}

fn ipma() -> T {
    let association = T::structure(
        "PropertyAssociation",
        vec![
            ("essential", bits(1)),
            ("property_index", T::switch(E::field("flags").bit(0), vec![(1, bits(15))], bits(7))),
        ],
    )
    .field_doc("property_index", "Which box of ipco, counting from 1. 0: none.")
    .field_doc("essential", "1: a reader that does not understand the property must not show the item.");
    let entry = T::structure(
        "ItemProperties",
        vec![
            ("item_id", item_id(1)),
            ("association_count", T::u8()),
            ("associations", T::array(association, E::field("association_count"))),
        ],
    )
    .named_by("item_id");
    let mut fields = full_box_flags("PropertyAssociationFlags", &[(0, "15-bit indices")]);
    fields.extend(vec![("entry_count", u32be()), ("entries", T::array(entry, E::field("entry_count")))]);
    T::structure("ItemPropertyAssociation", fields)
}

fn ispe() -> T {
    let mut fields = full_box();
    fields.extend(vec![("image_width", u32be()), ("image_height", u32be())]);
    T::structure("ImageSpatialExtents", fields)
}

fn pixi() -> T {
    let mut fields = full_box();
    fields.extend(vec![("num_channels", T::u8()), ("bits_per_channel", T::array(T::u8(), E::field("num_channels")))]);
    T::structure("PixelInformation", fields)
}

fn auxc() -> T {
    let mut fields = full_box();
    fields.extend(vec![("aux_type", T::cstr()), ("aux_subtype", T::bytes(E::Remaining))]);
    T::structure("AuxiliaryType", fields)
        .field_doc("aux_type", "What the auxiliary image is: urn:mpeg:mpegB:cicp:systems:auxiliary:alpha for an alpha plane.")
}
