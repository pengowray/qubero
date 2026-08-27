//! Aseprite `.ase`/`.aseprite`: a fixed file header followed by size-bounded
//! frames, each containing size-bounded typed chunks.
//!
//! Unknown chunk kinds deliberately keep their payload as bytes. The chunk's
//! size still places the next one correctly, so a newer producer cannot make
//! the remainder of the file disappear merely by adding a chunk type.

use crate::template::{Encoding, Endian::Little, Expr as E, StrLen, Template, Ty as T};

const DEPTHS: &[(i128, &str)] = &[(8, "indexed"), (16, "grayscale"), (32, "rgba")];
const LAYER_TYPES: &[(i128, &str)] = &[(0, "image"), (1, "group"), (2, "tilemap")];
const CEL_TYPES: &[(i128, &str)] = &[
    (0, "raw image"),
    (1, "linked"),
    (2, "zlib image"),
    (3, "zlib tilemap"),
];
const PROFILE_TYPES: &[(i128, &str)] = &[(0, "none"), (1, "sRGB"), (2, "embedded ICC")];
const EXTERNAL_TYPES: &[(i128, &str)] = &[
    (0, "palette"),
    (1, "tileset"),
    (2, "properties extension"),
    (3, "tile-management extension"),
];
const LOOP_DIRECTIONS: &[(i128, &str)] = &[
    (0, "forward"),
    (1, "reverse"),
    (2, "ping-pong"),
    (3, "ping-pong reverse"),
];
const CHUNK_TYPES: &[(i128, &str)] = &[
    (0x0004, "old palette 8-bit"),
    (0x0011, "old palette 6-bit"),
    (0x2004, "layer"),
    (0x2005, "cel"),
    (0x2006, "cel extra"),
    (0x2007, "color profile"),
    (0x2008, "external files"),
    (0x2016, "mask (deprecated)"),
    (0x2017, "path (unused)"),
    (0x2018, "tags"),
    (0x2019, "palette"),
    (0x2020, "user data"),
    (0x2022, "slice"),
    (0x2023, "tileset"),
];

const BLENDS: &[(i128, &str)] = &[
    (0, "normal"),
    (1, "multiply"),
    (2, "screen"),
    (3, "overlay"),
    (4, "darken"),
    (5, "lighten"),
    (6, "color dodge"),
    (7, "color burn"),
    (8, "hard light"),
    (9, "soft light"),
    (10, "difference"),
    (11, "exclusion"),
    (12, "hue"),
    (13, "saturation"),
    (14, "color"),
    (15, "luminosity"),
    (16, "addition"),
    (17, "subtract"),
    (18, "divide"),
];

fn i16() -> T {
    T::Int {
        bits: 16,
        endian: Little,
    }
}
fn i32() -> T {
    T::Int {
        bits: 32,
        endian: Little,
    }
}

/// One or zero for bit `n` in an integer expression.
fn bit(value: E, n: u32) -> E {
    value
        .clone()
        .div(E::lit(1i128 << n))
        .sub(value.div(E::lit(1i128 << (n + 1))).mul(E::lit(2)))
}

fn string() -> T {
    T::inline_structure(
        "String",
        vec![
            ("length", T::u16(Little)),
            (
                "text",
                T::text(StrLen::Fixed(E::field("length")), Encoding::Utf8),
            ),
        ],
    )
}

pub fn aseprite() -> Template {
    Template::new(
        "aseprite",
        T::structure(
            "Aseprite",
            vec![
                ("header", header()),
                (
                    "frames",
                    T::array(frame(), E::within(&["header", "frame_count"])),
                ),
            ],
        ),
    )
}

fn header() -> T {
    T::sized(
        E::lit(128),
        T::structure(
            "Header",
            vec![
                ("file_size", T::u32(Little)),
                ("magic", T::magic(&0xa5e0u16.to_le_bytes())),
                ("frame_count", T::u16(Little)),
                ("width", T::u16(Little)),
                ("height", T::u16(Little)),
                (
                    "color_depth",
                    T::enumeration("ColorDepth", T::u16(Little), DEPTHS),
                ),
                (
                    "flags",
                    T::flags(
                        "HeaderFlags",
                        T::u32(Little),
                        &[
                            (0, "layer opacity valid"),
                            (1, "group blend/opacity valid"),
                            (2, "layer UUIDs present"),
                        ],
                    ),
                ),
                ("deprecated_speed_ms", T::u16(Little)),
                ("reserved_1", T::u32(Little)),
                ("reserved_2", T::u32(Little)),
                ("transparent_palette_index", T::u8()),
                ("ignored", T::bytes(E::lit(3))),
                ("color_count", T::u16(Little)),
                ("pixel_width", T::u8()),
                ("pixel_height", T::u8()),
                ("grid_x", i16()),
                ("grid_y", i16()),
                ("grid_width", T::u16(Little)),
                ("grid_height", T::u16(Little)),
                ("future", T::bytes(E::lit(84))),
            ],
        ),
    )
}

fn frame() -> T {
    T::structure(
        "Frame",
        vec![
            ("size", T::u32(Little)),
            (
                "body",
                T::sized(
                    E::field("size").sub(E::lit(4)),
                    T::structure(
                        "FrameBody",
                        vec![
                            ("magic", T::magic(&0xf1fau16.to_le_bytes())),
                            ("old_chunk_count", T::u16(Little)),
                            ("duration_ms", T::u16(Little)),
                            ("future", T::bytes(E::lit(2))),
                            ("new_chunk_count", T::u32(Little)),
                            (
                                "chunk_count",
                                T::computed(
                                    E::field("new_chunk_count").or(E::field("old_chunk_count")),
                                ),
                            ),
                            ("chunks", T::array(chunk(), E::field("chunk_count"))),
                        ],
                    ),
                ),
            ),
        ],
    )
    .counted_as("frame")
}

fn chunk() -> T {
    T::structure(
        "Chunk",
        vec![
            ("size", T::u32(Little)),
            (
                "record",
                T::sized(
                    E::field("size").sub(E::lit(4)),
                    T::structure_named(
                        "ChunkRecord",
                        "type",
                        "body",
                        vec![
                            (
                                "type",
                                T::enumeration_hex("ChunkType", T::u16(Little), CHUNK_TYPES),
                            ),
                            (
                                "body",
                                T::switch(E::field("type"), chunk_cases(), T::bytes(E::Remaining)),
                            ),
                        ],
                    ),
                ),
            ),
        ],
    )
    .counted_as("chunk")
}

fn chunk_cases() -> Vec<(i128, T)> {
    vec![
        (0x0004, old_palette()),
        (0x0011, old_palette()),
        (0x2004, layer()),
        (0x2005, cel()),
        (0x2006, cel_extra()),
        (0x2007, color_profile()),
        (0x2008, external_files()),
        (0x2016, mask()),
        (0x2017, T::bytes(E::Remaining)),
        (0x2018, tags()),
        (0x2019, palette()),
        (0x2020, user_data()),
        (0x2022, slice()),
        (0x2023, tileset()),
    ]
}

fn old_palette() -> T {
    let color = T::inline_structure(
        "Rgb",
        vec![("red", T::u8()), ("green", T::u8()), ("blue", T::u8())],
    );
    let packet = T::structure(
        "PalettePacket",
        vec![
            ("skip", T::u8()),
            ("color_count", T::u8()),
            (
                "colors",
                T::array(color, E::field("color_count").or(E::lit(256))),
            ),
        ],
    );
    T::structure(
        "OldPalette",
        vec![
            ("packet_count", T::u16(Little)),
            ("packets", T::array(packet, E::field("packet_count"))),
        ],
    )
}

fn layer() -> T {
    let uuid_present = bit(E::within(&["header", "flags"]), 2);
    T::structure(
        "Layer",
        vec![
            (
                "flags",
                T::flags(
                    "LayerFlags",
                    T::u16(Little),
                    &[
                        (0, "visible"),
                        (1, "editable"),
                        (2, "movement locked"),
                        (3, "background"),
                        (4, "prefer linked cels"),
                        (5, "collapsed"),
                        (6, "reference"),
                    ],
                ),
            ),
            (
                "layer_type",
                T::enumeration("LayerType", T::u16(Little), LAYER_TYPES),
            ),
            ("child_level", T::u16(Little)),
            ("default_width", T::u16(Little)),
            ("default_height", T::u16(Little)),
            (
                "blend_mode",
                T::enumeration("BlendMode", T::u16(Little), BLENDS),
            ),
            ("opacity", T::u8()),
            ("future", T::bytes(E::lit(3))),
            ("name", string()),
            (
                "tileset_index",
                T::switch(
                    E::field("layer_type"),
                    vec![(2, T::u32(Little))],
                    T::bytes(E::lit(0)),
                ),
            ),
            ("uuid", T::bytes(uuid_present.mul(E::lit(16)))),
        ],
    )
}

fn cel() -> T {
    let image = |name| {
        T::structure(
            name,
            vec![
                ("width", T::u16(Little)),
                ("height", T::u16(Little)),
                ("pixels", T::bytes(E::Remaining)),
            ],
        )
    };
    let tilemap = T::structure(
        "CompressedTilemap",
        vec![
            ("width_tiles", T::u16(Little)),
            ("height_tiles", T::u16(Little)),
            ("bits_per_tile", T::u16(Little)),
            ("tile_id_mask", T::u32(Little)),
            ("x_flip_mask", T::u32(Little)),
            ("y_flip_mask", T::u32(Little)),
            ("diagonal_flip_mask", T::u32(Little)),
            ("reserved", T::bytes(E::lit(10))),
            ("zlib_tiles", T::bytes(E::Remaining)),
        ],
    );
    T::structure(
        "Cel",
        vec![
            ("layer_index", T::u16(Little)),
            ("x", i16()),
            ("y", i16()),
            ("opacity", T::u8()),
            (
                "cel_type",
                T::enumeration("CelType", T::u16(Little), CEL_TYPES),
            ),
            ("z_index", i16()),
            ("future", T::bytes(E::lit(5))),
            (
                "data",
                T::switch(
                    E::field("cel_type"),
                    vec![
                        (0, image("RawImage")),
                        (
                            1,
                            T::structure("LinkedCel", vec![("frame", T::u16(Little))]),
                        ),
                        (2, image("CompressedImage")),
                        (3, tilemap),
                    ],
                    T::bytes(E::Remaining),
                ),
            ),
        ],
    )
}

fn cel_extra() -> T {
    T::structure(
        "CelExtra",
        vec![
            (
                "flags",
                T::flags(
                    "CelExtraFlags",
                    T::u32(Little),
                    &[(0, "precise bounds set")],
                ),
            ),
            ("precise_x", T::fixed(32, 16, Little)),
            ("precise_y", T::fixed(32, 16, Little)),
            ("width", T::fixed(32, 16, Little)),
            ("height", T::fixed(32, 16, Little)),
            ("future", T::bytes(E::lit(16))),
        ],
    )
}

fn color_profile() -> T {
    T::structure(
        "ColorProfile",
        vec![
            (
                "profile_type",
                T::enumeration("ProfileType", T::u16(Little), PROFILE_TYPES),
            ),
            (
                "flags",
                T::flags("ProfileFlags", T::u16(Little), &[(0, "fixed gamma")]),
            ),
            ("gamma", T::fixed(32, 16, Little)),
            ("reserved", T::bytes(E::lit(8))),
            (
                "icc",
                T::switch(
                    E::field("profile_type"),
                    vec![(
                        2,
                        T::structure(
                            "IccProfile",
                            vec![
                                ("length", T::u32(Little)),
                                ("data", T::bytes(E::field("length"))),
                            ],
                        ),
                    )],
                    T::bytes(E::lit(0)),
                ),
            ),
        ],
    )
}

fn external_files() -> T {
    let entry = T::structure(
        "ExternalFile",
        vec![
            ("id", T::u32(Little)),
            (
                "type",
                T::enumeration("ExternalType", T::u8(), EXTERNAL_TYPES),
            ),
            ("reserved", T::bytes(E::lit(7))),
            ("name", string()),
        ],
    );
    T::structure(
        "ExternalFiles",
        vec![
            ("count", T::u32(Little)),
            ("reserved", T::bytes(E::lit(8))),
            ("entries", T::array(entry, E::field("count"))),
        ],
    )
}

fn mask() -> T {
    T::structure(
        "Mask",
        vec![
            ("x", i16()),
            ("y", i16()),
            ("width", T::u16(Little)),
            ("height", T::u16(Little)),
            ("future", T::bytes(E::lit(8))),
            ("name", string()),
            ("bitmap", T::bytes(E::Remaining)),
        ],
    )
}

fn tags() -> T {
    let tag = T::structure(
        "Tag",
        vec![
            ("from_frame", T::u16(Little)),
            ("to_frame", T::u16(Little)),
            (
                "direction",
                T::enumeration("LoopDirection", T::u8(), LOOP_DIRECTIONS),
            ),
            ("repeat", T::u16(Little)),
            ("future", T::bytes(E::lit(6))),
            ("deprecated_color", T::bytes(E::lit(3))),
            ("extra", T::u8()),
            ("name", string()),
        ],
    );
    T::structure(
        "Tags",
        vec![
            ("count", T::u16(Little)),
            ("future", T::bytes(E::lit(8))),
            ("tags", T::array(tag, E::field("count"))),
        ],
    )
}

fn palette() -> T {
    let entry = T::structure(
        "PaletteEntry",
        vec![
            (
                "flags",
                T::flags("PaletteEntryFlags", T::u16(Little), &[(0, "has name")]),
            ),
            ("red", T::u8()),
            ("green", T::u8()),
            ("blue", T::u8()),
            ("alpha", T::u8()),
            (
                "name",
                T::switch(
                    bit(E::field("flags"), 0),
                    vec![(1, string())],
                    T::bytes(E::lit(0)),
                ),
            ),
        ],
    );
    T::structure(
        "Palette",
        vec![
            ("size", T::u32(Little)),
            ("first", T::u32(Little)),
            ("last", T::u32(Little)),
            ("future", T::bytes(E::lit(8))),
            (
                "entries",
                T::array(
                    entry,
                    E::field("last").sub(E::field("first")).add(E::lit(1)),
                ),
            ),
        ],
    )
}

fn user_data() -> T {
    T::structure(
        "UserData",
        vec![
            (
                "flags",
                T::flags(
                    "UserDataFlags",
                    T::u32(Little),
                    &[(0, "has text"), (1, "has color"), (2, "has properties")],
                ),
            ),
            (
                "text",
                T::switch(
                    bit(E::field("flags"), 0),
                    vec![(1, string())],
                    T::bytes(E::lit(0)),
                ),
            ),
            ("color", T::bytes(bit(E::field("flags"), 1).mul(E::lit(4)))),
            (
                "properties",
                T::switch(
                    bit(E::field("flags"), 2),
                    vec![(1, T::bytes(E::Remaining))],
                    T::bytes(E::lit(0)),
                ),
            ),
        ],
    )
}

fn slice() -> T {
    let center = || {
        T::structure(
            "Center",
            vec![
                ("x", i32()),
                ("y", i32()),
                ("width", T::u32(Little)),
                ("height", T::u32(Little)),
            ],
        )
    };
    let pivot = || T::structure("Pivot", vec![("x", i32()), ("y", i32())]);
    let key = T::structure(
        "SliceKey",
        vec![
            ("frame", T::u32(Little)),
            ("x", i32()),
            ("y", i32()),
            ("width", T::u32(Little)),
            ("height", T::u32(Little)),
            (
                "center",
                T::switch(
                    bit(E::field("flags"), 0),
                    vec![(1, center())],
                    T::bytes(E::lit(0)),
                ),
            ),
            (
                "pivot",
                T::switch(
                    bit(E::field("flags"), 1),
                    vec![(1, pivot())],
                    T::bytes(E::lit(0)),
                ),
            ),
        ],
    );
    T::structure(
        "Slice",
        vec![
            ("key_count", T::u32(Little)),
            (
                "flags",
                T::flags(
                    "SliceFlags",
                    T::u32(Little),
                    &[(0, "nine-patch"), (1, "has pivot")],
                ),
            ),
            ("reserved", T::u32(Little)),
            ("name", string()),
            ("keys", T::array(key, E::field("key_count"))),
        ],
    )
}

fn tileset() -> T {
    let external = T::structure(
        "ExternalTileset",
        vec![("file_id", T::u32(Little)), ("tileset_id", T::u32(Little))],
    );
    let image = T::structure(
        "TilesetImage",
        vec![
            ("compressed_length", T::u32(Little)),
            ("zlib_pixels", T::bytes(E::field("compressed_length"))),
        ],
    );
    T::structure(
        "Tileset",
        vec![
            ("id", T::u32(Little)),
            (
                "flags",
                T::flags(
                    "TilesetFlags",
                    T::u32(Little),
                    &[
                        (0, "external link"),
                        (1, "embedded tiles"),
                        (2, "tile zero is empty"),
                        (3, "auto x-flip"),
                        (4, "auto y-flip"),
                        (5, "auto diagonal-flip"),
                    ],
                ),
            ),
            ("tile_count", T::u32(Little)),
            ("tile_width", T::u16(Little)),
            ("tile_height", T::u16(Little)),
            ("base_index", i16()),
            ("reserved", T::bytes(E::lit(14))),
            ("name", string()),
            (
                "external",
                T::switch(
                    bit(E::field("flags"), 0),
                    vec![(1, external)],
                    T::bytes(E::lit(0)),
                ),
            ),
            (
                "image",
                T::switch(
                    bit(E::field("flags"), 1),
                    vec![(1, image)],
                    T::bytes(E::lit(0)),
                ),
            ),
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

    fn sample() -> Vec<u8> {
        let mut header = Vec::new();
        header.extend_from_slice(&0u32.to_le_bytes());
        header.extend_from_slice(&0xa5e0u16.to_le_bytes());
        header.extend_from_slice(&1u16.to_le_bytes());
        header.extend_from_slice(&16u16.to_le_bytes());
        header.extend_from_slice(&8u16.to_le_bytes());
        header.extend_from_slice(&32u16.to_le_bytes());
        header.extend_from_slice(&1u32.to_le_bytes());
        header.extend_from_slice(&100u16.to_le_bytes());
        header.resize(128, 0);

        let mut layer = Vec::new();
        layer.extend_from_slice(&1u16.to_le_bytes());
        layer.extend_from_slice(&0u16.to_le_bytes());
        layer.extend_from_slice(&[0; 8]);
        layer.push(255);
        layer.extend_from_slice(&[0; 3]);
        layer.extend_from_slice(&4u16.to_le_bytes());
        layer.extend_from_slice(b"Ink!");
        let chunk_size = layer.len() as u32 + 6;

        let frame_size = 16 + chunk_size;
        header.extend_from_slice(&frame_size.to_le_bytes());
        header.extend_from_slice(&0xf1fau16.to_le_bytes());
        header.extend_from_slice(&1u16.to_le_bytes());
        header.extend_from_slice(&80u16.to_le_bytes());
        header.extend_from_slice(&[0; 2]);
        header.extend_from_slice(&0u32.to_le_bytes());
        header.extend_from_slice(&chunk_size.to_le_bytes());
        header.extend_from_slice(&0x2004u16.to_le_bytes());
        header.extend_from_slice(&layer);
        let len = header.len() as u32;
        header[..4].copy_from_slice(&len.to_le_bytes());
        header
    }

    #[test]
    fn reads_header_frame_and_layer_chunk() {
        let d = Document::new(MemSource(sample()));
        let mut ev = Evaluator::new(aseprite());
        assert_eq!(
            ev.node(&d, &[0, 5]).unwrap().value,
            Value::Enum {
                raw: 32,
                name: Some("rgba".into()),
                hex: false
            }
        );
        assert_eq!(ev.node(&d, &[1]).unwrap().child_count, 1);
        assert_eq!(ev.node(&d, &[1, 0, 1, 6]).unwrap().child_count, 1);
        assert_eq!(
            ev.node(&d, &[1, 0, 1, 6, 0, 1, 1, 8, 1]).unwrap().value,
            Value::Str("Ink!".into())
        );
    }

    #[test]
    fn the_header_magic_sniffs_without_using_the_extension() {
        let bytes = sample();
        assert_eq!(
            super::super::sniff(&bytes[..8], bytes.len() as u64),
            Some("aseprite")
        );
    }
}
