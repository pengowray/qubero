//! Adobe Photoshop PSD and large-document PSB files.
//!
//! The format is a fixed header followed by four length-delimited sections.
//! This template opens image-resource records and layer records; compressed
//! pixel/channel payloads remain whole because their interpretation depends on
//! compression, image depth, row counts and (for ZIP) decompression.

use crate::template::{Encoding, Endian::Big, Expr as E, StrLen, Template, Ty as T, Until};

const MODES: &[(i128, &str)] = &[
    (0, "bitmap"),
    (1, "grayscale"),
    (2, "indexed"),
    (3, "rgb"),
    (4, "cmyk"),
    (7, "multichannel"),
    (8, "duotone"),
    (9, "lab"),
];
const COMPRESSION: &[(i128, &str)] = &[
    (0, "raw"),
    (1, "rle"),
    (2, "zip"),
    (3, "zip with prediction"),
];
const RESOURCES: &[(i128, &str)] = &[
    (1005, "resolution info"),
    (1028, "IPTC-NAA"),
    (1032, "grid and guides"),
    (1033, "thumbnail (Photoshop 4)"),
    (1036, "thumbnail"),
    (1039, "ICC profile"),
    (1045, "Unicode alpha names"),
    (1058, "EXIF data 1"),
    (1059, "EXIF data 3"),
    (1060, "XMP metadata"),
    (1061, "caption digest"),
    (1062, "print scale"),
    (1069, "layer selection IDs"),
    (1072, "layer group enabled IDs"),
    (1082, "print information"),
    (1083, "print style"),
    (1086, "pixel aspect ratio"),
];

pub fn psd() -> Template {
    Template::new(
        "psd",
        T::structure(
            "PhotoshopDocument",
            vec![
                ("signature", T::magic(b"8BPS")),
                (
                    "version",
                    T::enumeration("PhotoshopVersion", T::u16(Big), &[(1, "PSD"), (2, "PSB")]),
                ),
                ("reserved", T::bytes(E::lit(6))),
                ("channels", T::u16(Big)),
                ("height", T::u32(Big)),
                ("width", T::u32(Big)),
                ("depth", T::u16(Big)),
                (
                    "colour_mode",
                    T::enumeration("ColourMode", T::u16(Big), MODES),
                ),
                (
                    "sections",
                    T::switch(
                        E::field("version"),
                        vec![(2, sections(true))],
                        sections(false),
                    ),
                ),
            ],
        ),
    )
}

fn sections(large: bool) -> T {
    T::structure(
        if large { "PsbSections" } else { "PsdSections" },
        vec![
            ("colour_data_size", T::u32(Big)),
            ("colour_data", T::bytes(E::field("colour_data_size"))),
            ("resources_size", T::u32(Big)),
            (
                "resources",
                T::sized(
                    E::field("resources_size"),
                    T::repeat(resource(), Until::End),
                ),
            ),
            (
                "layer_mask_size",
                if large { T::u64(Big) } else { T::u32(Big) },
            ),
            (
                "layer_and_mask",
                T::sized(E::field("layer_mask_size"), layer_and_mask(large)),
            ),
            ("image_data", composite_image()),
        ],
    )
}

fn resource() -> T {
    let name_total = E::field("name_length").add(E::lit(1));
    let name_pad = name_total
        .clone()
        .add(E::lit(1))
        .div(E::lit(2))
        .mul(E::lit(2))
        .sub(name_total);
    let data_pad = E::field("data_size").sub(E::field("data_size").div(E::lit(2)).mul(E::lit(2)));
    T::structure_named(
        "ImageResource",
        "resource_id",
        "data",
        vec![
            (
                "signature",
                T::text(StrLen::Fixed(E::lit(4)), Encoding::Ascii),
            ),
            (
                "resource_id",
                T::enumeration("ImageResourceId", T::u16(Big), RESOURCES),
            ),
            ("name_length", T::u8()),
            (
                "name",
                T::text(StrLen::Fixed(E::field("name_length")), Encoding::Latin1),
            ),
            ("name_pad", T::bytes(name_pad)),
            ("data_size", T::u32(Big)),
            ("data", T::bytes(E::field("data_size"))),
            ("data_pad", T::bytes(data_pad)),
        ],
    )
    .counted_as("resource")
}

fn layer_and_mask(large: bool) -> T {
    T::structure(
        "LayerAndMaskInformation",
        vec![
            (
                "layer_info_size",
                if large { T::u64(Big) } else { T::u32(Big) },
            ),
            (
                "layer_info",
                T::sized(E::field("layer_info_size"), layer_info(large)),
            ),
            ("global_mask_size", T::u32(Big)),
            ("global_mask", T::bytes(E::field("global_mask_size"))),
            ("additional_layer_information", T::bytes(E::Remaining)),
        ],
    )
}

fn layer_info(large: bool) -> T {
    T::structure(
        "LayerInformation",
        vec![
            // A negative signed count means the first alpha channel stores
            // transparency for the merged composite. Splitting the sign bit
            // recovers the absolute count instead of asking for 65,535 layers.
            (
                "merged_alpha",
                T::UInt {
                    bits: 1,
                    endian: Big,
                },
            ),
            (
                "layer_count_bits",
                T::UInt {
                    bits: 15,
                    endian: Big,
                },
            ),
            (
                "layer_count",
                T::switch(
                    E::field("merged_alpha"),
                    vec![(
                        1,
                        T::computed(E::lit(32768).sub(E::field("layer_count_bits"))),
                    )],
                    T::computed(E::field("layer_count_bits")),
                ),
            ),
            ("layers", T::array(layer(large), E::field("layer_count"))),
            ("channel_image_data", T::bytes(E::Remaining)),
        ],
    )
}

fn layer(large: bool) -> T {
    T::structure(
        "LayerRecord",
        vec![
            ("top", T::i32(Big)),
            ("left", T::i32(Big)),
            ("bottom", T::i32(Big)),
            ("right", T::i32(Big)),
            ("channel_count", T::u16(Big)),
            (
                "channels",
                T::array(channel(large), E::field("channel_count")),
            ),
            (
                "blend_signature",
                T::text(StrLen::Fixed(E::lit(4)), Encoding::Ascii),
            ),
            (
                "blend_mode",
                T::text(StrLen::Fixed(E::lit(4)), Encoding::Ascii),
            ),
            ("opacity", T::u8()),
            ("clipping", T::u8()),
            (
                "flags",
                T::flags(
                    "LayerFlags",
                    T::u8(),
                    &[
                        (0, "transparency protected"),
                        (1, "hidden"),
                        (3, "pixel data irrelevant"),
                    ],
                ),
            ),
            ("filler", T::u8()),
            ("extra_size", T::u32(Big)),
            ("extra", T::bytes(E::field("extra_size"))),
        ],
    )
    .counted_as("layer")
}

fn channel(large: bool) -> T {
    T::inline_structure(
        "LayerChannel",
        vec![
            (
                "id",
                T::Int {
                    bits: 16,
                    endian: Big,
                },
            ),
            ("data_size", if large { T::u64(Big) } else { T::u32(Big) }),
        ],
    )
    .counted_as("channel")
}

fn composite_image() -> T {
    T::structure(
        "CompositeImageData",
        vec![
            (
                "compression",
                T::enumeration("Compression", T::u16(Big), COMPRESSION),
            ),
            ("data", T::bytes(E::Remaining)),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        document::Document,
        eval::{Evaluator, Value},
        source::MemSource,
    };

    #[test]
    fn header_and_resource_record_are_bounded() {
        let mut v = b"8BPS\0\x01\0\0\0\0\0\0\0\x03\0\0\0\x02\0\0\0\x04\0\x08\0\x03".to_vec();
        v.extend_from_slice(&0u32.to_be_bytes());
        let mut resource = b"8BIM".to_vec();
        resource.extend_from_slice(&1060u16.to_be_bytes());
        resource.extend_from_slice(&[1, b'x']);
        resource.extend_from_slice(&3u32.to_be_bytes());
        resource.extend_from_slice(b"abc\0");
        v.extend_from_slice(&(resource.len() as u32).to_be_bytes());
        v.extend_from_slice(&resource);
        v.extend_from_slice(&0u32.to_be_bytes());
        v.extend_from_slice(&0u16.to_be_bytes());
        let d = Document::new(MemSource(v));
        let mut ev = Evaluator::new(psd());
        assert_eq!(ev.node(&d, &[4]).unwrap().value, Value::UInt(2));
        assert!(matches!(
            ev.node(&d, &[8, 3, 0, 1]).unwrap().value,
            Value::Enum { raw: 1060, .. }
        ));
        assert_eq!(ev.node(&d, &[8, 3, 0, 6]).unwrap().size_bits, 24);
    }
}
