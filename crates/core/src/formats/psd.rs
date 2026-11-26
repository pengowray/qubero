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

/// One image resource, as a PSD stores it and as a JPEG's APP13 does too.
pub(crate) fn resource() -> T {
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
            (
                "data",
                T::sized(
                    E::field("data_size"),
                    T::switch(
                        E::field("resource_id"),
                        vec![
                            (1005, resolution_info()),
                            (1028, T::repeat(iptc_dataset(), Until::End)),
                            (1060, T::text(StrLen::Fixed(E::Remaining), Encoding::Utf8)),
                        ],
                        T::bytes(E::Remaining),
                    ),
                ),
            ),
            ("data_pad", T::bytes(data_pad)),
        ],
    )
    .counted_as("resource")
}

/// Resource 1005: the image's resolution as Photoshop's Image Size dialog
/// shows it. Each resolution is a 16.16 fixed-point number of pixels per
/// inch, whatever unit the dialog displays it in.
fn resolution_info() -> T {
    let fixed = |stored: &str| T::computed_real(E::field(stored).div(E::real(65536.0)));
    let res_unit = || T::enumeration("ResolutionUnit", T::u16(Big), &[(1, "pixels per inch"), (2, "pixels per cm")]);
    let size_unit = || {
        T::enumeration(
            "SizeUnit",
            T::u16(Big),
            &[(1, "inches"), (2, "cm"), (3, "points"), (4, "picas"), (5, "columns")],
        )
    };
    T::structure(
        "ResolutionInfo",
        vec![
            ("h_res_fixed", T::u32(Big)),
            ("h_res", fixed("h_res_fixed")),
            ("h_res_unit", res_unit()),
            ("width_unit", size_unit()),
            ("v_res_fixed", T::u32(Big)),
            ("v_res", fixed("v_res_fixed")),
            ("v_res_unit", res_unit()),
            ("height_unit", size_unit()),
        ],
    )
    .field_doc("h_res", "Horizontal resolution in pixels per inch: h_res_fixed divided by 65,536.")
    .field_doc("v_res", "Vertical resolution in pixels per inch: v_res_fixed divided by 65,536.")
    .field_doc("h_res_unit", "The unit Photoshop shows the horizontal resolution in. The stored value is pixels per inch either way.")
    .field_doc("v_res_unit", "The unit Photoshop shows the vertical resolution in. The stored value is pixels per inch either way.")
}

/// The IPTC Application Record (record 2) datasets that photo software writes,
/// by dataset number.
const IPTC_APPLICATION: &[(i128, &str)] = &[
    (0, "record version"),
    (5, "object name"),
    (7, "edit status"),
    (10, "urgency"),
    (15, "category"),
    (20, "supplemental category"),
    (25, "keywords"),
    (40, "special instructions"),
    (55, "date created"),
    (60, "time created"),
    (62, "digital creation date"),
    (63, "digital creation time"),
    (65, "originating program"),
    (70, "program version"),
    (80, "by-line"),
    (85, "by-line title"),
    (90, "city"),
    (92, "sub-location"),
    (95, "province or state"),
    (100, "country code"),
    (101, "country name"),
    (103, "original transmission reference"),
    (105, "headline"),
    (110, "credit"),
    (115, "source"),
    (116, "copyright notice"),
    (118, "contact"),
    (120, "caption"),
    (122, "caption writer"),
];

/// The IPTC Envelope Record (record 1) datasets that photo software writes.
const IPTC_ENVELOPE: &[(i128, &str)] = &[
    (0, "record version"),
    (90, "coded character set"),
];

/// One IPTC-IIM dataset, which is how Photoshop stores captions, keywords and
/// credits: a 0x1c marker, the record and dataset numbers, a two-byte length,
/// and the data. Text datasets are read as UTF-8, which is what current
/// software writes and declares in dataset 1:90; older files may use another
/// character set.
fn iptc_dataset() -> T {
    let record_2 = T::enumeration("IptcApplicationDataset", T::u8(), IPTC_APPLICATION);
    let record_1 = T::enumeration("IptcEnvelopeDataset", T::u8(), IPTC_ENVELOPE);
    let text = || T::text(StrLen::Fixed(E::field("size")), Encoding::Utf8);
    T::structure_named(
        "IptcDataset",
        "dataset",
        "",
        vec![
            ("marker", T::magic(&[0x1c])),
            ("record", T::u8()),
            ("dataset", T::switch(E::field("record"), vec![(1, record_1), (2, record_2)], T::u8())),
            ("size", T::u16(Big)),
            (
                "data",
                // Record 2's version is a number; every other dataset of that
                // record is text.
                T::switch(
                    E::field("record").mul(E::lit(256)).add(E::field("dataset")),
                    vec![(0x200, T::u16(Big)), (0x100, T::u16(Big))],
                    T::switch(E::field("record"), vec![(2, text())], T::bytes(E::field("size"))),
                ),
            ),
        ],
    )
    .counted_as("dataset")
    .field_doc("record", "Which IPTC record the dataset belongs to: 1 is the envelope, 2 the application record that holds captions, keywords and credits.")
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
