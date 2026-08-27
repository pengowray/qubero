//! Formats covered by Assimp's importer registry.
//!
//! Assimp groups several byte formats behind one importer (notably STL, PLY,
//! glTF and USD), and several extensions are merely aliases.  These templates
//! follow the bytes rather than the extension: structured containers and
//! binary headers expose their fields, JSON stays JSON, ZIP packages borrow
//! the ZIP template, and formats whose grammar is textual remain a run of
//! source lines.  The latter is intentionally not a pretend 3D parser, but it
//! still makes every source record addressable and editable.

use crate::template::{Encoding, Endian::*, Expr as E, StrLen, Template, Ty as T, Until};

use super::{iff::iff, zip};

/// Selectable names for every importer family in Assimp's current registry.
/// Common extension aliases are present too, because that is how Assimp's own
/// format table presents them to users.
#[cfg_attr(not(test), allow(dead_code))]
pub const NAMES: &[&str] = &[
    "3ds",
    "3mf",
    "ac3d",
    "amf",
    "ase",
    "assbin",
    "b3d",
    "blend",
    "bvh",
    "c4d",
    "cob",
    "collada",
    "csm",
    "dxf",
    "fbx",
    "glb",
    "gltf",
    "hmp",
    "ifc",
    "ifczip",
    "iqm",
    "irr",
    "irrmesh",
    "lwo",
    "lws",
    "lxo",
    "m3d",
    "md2",
    "md3",
    "md5",
    "mdc",
    "mdl",
    "ms3d",
    "ndo",
    "nff",
    "obj",
    "off",
    "ogre",
    "ogrexml",
    "ogex",
    "ply",
    "pmx",
    "q3bsp",
    "pk3",
    "q3o",
    "q3s",
    "raw3d",
    "sib",
    "smd",
    "stl",
    "ter",
    "unreal3d",
    "usd",
    "usda",
    "usdc",
    "usdz",
    "vta",
    "x",
    "x3d",
    "x3db",
    "x3dv",
    "xgl",
    "3d",
    "ac",
    "acc",
    "a3d",
    "amj",
    "ask",
    "dae",
    "enff",
    "md5anim",
    "md5camera",
    "md5mesh",
    "mesh",
    "mesh.xml",
    "mot",
    "prj",
    "raw",
    "scn",
    "step",
    "stp",
    "uc",
    "vrm",
    "zae",
    "zgl",
];

pub fn template(name: &str) -> Option<Template> {
    let template = match name {
        "3ds" | "prj" => chunks_3ds(),
        "3mf" | "ifczip" | "pk3" | "usdz" | "zae" => zip(),
        "glb" => glb(),
        "gltf" | "vrm" => Template::new(name, T::json()),
        "lwo" | "lxo" => iff(name, T::bytes(E::Remaining)),
        "b3d" => b3d(),
        "md2" => md2(),
        "md3" => md3(),
        "iqm" => iqm(),
        "assbin" => assbin(),
        "fbx" => fbx(),
        "x" => x_file(),
        "usdc" => usdc(),
        "usda" => text_document(name),
        // `.usd` can contain either USDA text or USDC binary, so its neutral
        // template does not assert one representation.
        "usd" => opaque(name, "UsdData"),
        "ply" => text_document(name),
        "stl" => opaque(name, "StlData"),
        "hmp" | "mdl" | "m3d" | "mdc" | "ms3d" | "ndo" | "pmx" | "q3bsp" | "q3o" | "q3s"
        | "sib" | "ter" | "blend" => tagged_binary(name),
        "c4d" | "cob" | "dxf" | "mesh" | "ogre" | "scn" | "x3db" | "zgl" => {
            opaque(name, "BinaryModelData")
        }
        "amf" | "collada" | "dae" | "irr" | "irrmesh" | "mesh.xml" | "ogrexml" | "x3d" => {
            encoded_document(name)
        }
        "ac" | "acc" | "ac3d" | "amj" | "ase" | "ask" | "bvh" | "csm" | "enff" | "ifc" | "lws"
        | "md5" | "md5anim" | "md5camera" | "md5mesh" | "mot" | "nff" | "obj" | "off" | "ogex"
        | "raw" | "raw3d" | "smd" | "step" | "stp" | "3d" | "uc" | "unreal3d" | "vta" | "x3dv"
        | "xgl" => text_document(name),
        "a3d" => tagged_binary("m3d"),
        _ => return None,
    };
    Some(renamed(template, name))
}

fn renamed(mut template: Template, name: &str) -> Template {
    template.name = name.to_string();
    template
}

fn text_document(name: &str) -> Template {
    let line = T::structure_named(
        "SourceLine",
        "text",
        "",
        vec![(
            "text",
            T::text(
                StrLen::Terminated {
                    end: b'\n',
                    or_end: true,
                },
                Encoding::Unknown,
            ),
        )],
    )
    .counted_as("line");
    Template::new(name, T::repeat(line, Until::End))
}

fn encoded_document(name: &str) -> Template {
    Template::new(
        name,
        T::structure(
            "TextDocument",
            vec![(
                "text",
                T::text(
                    StrLen::Fixed(E::Remaining),
                    Encoding::Bom {
                        fallback: Box::new(Encoding::Utf8),
                    },
                ),
            )],
        ),
    )
}

fn opaque(name: &str, kind: &str) -> Template {
    Template::new(
        name,
        T::structure(kind, vec![("data", T::bytes(E::Remaining))]),
    )
}

fn tagged_binary(name: &str) -> Template {
    let (kind, tag_len) = match name {
        "blend" => ("BlenderFile", 12),
        "m3d" => ("M3dModel", 4),
        "mdc" => ("MdcModel", 4),
        "ms3d" => ("MilkshapeModel", 10),
        "ndo" => ("NendoModel", 9),
        "pmx" => ("PmxModel", 4),
        "q3bsp" => ("Quake3Bsp", 4),
        "q3o" | "q3s" => ("Quick3dModel", 10),
        "sib" => ("SiloModel", 4),
        "ter" => ("TerragenTerrain", 16),
        "hmp" => ("HmpTerrain", 4),
        "mdl" => ("MdlModel", 4),
        _ => ("BinaryModel", 4),
    };
    Template::new(
        name,
        T::structure(
            kind,
            vec![
                (
                    "signature",
                    T::text(StrLen::Fixed(E::lit(tag_len)), Encoding::Ascii),
                ),
                ("data", T::bytes(E::Remaining)),
            ],
        ),
    )
}

fn chunks_3ds() -> Template {
    let chunk = T::structure_named(
        "Chunk3ds",
        "id",
        "body",
        vec![
            ("id", T::u16(Little)),
            ("length", T::u32(Little)),
            ("body", T::bytes(E::field("length").sub(E::lit(6)))),
        ],
    )
    .counted_as("chunk");
    Template::new("3ds", T::repeat(chunk, Until::End))
}

fn b3d() -> Template {
    let chunk = T::structure_named(
        "B3dChunk",
        "tag",
        "body",
        vec![
            ("tag", T::text(StrLen::Fixed(E::lit(4)), Encoding::Ascii)),
            ("size", T::u32(Little)),
            ("body", T::bytes(E::field("size"))),
        ],
    );
    Template::new(
        "b3d",
        T::structure(
            "Blitz3dFile",
            vec![
                ("magic", T::magic(b"BB3D")),
                ("size", T::u32(Little)),
                ("version", T::u32(Little)),
                ("chunks", T::repeat(chunk, Until::End)),
            ],
        ),
    )
}

fn glb() -> Template {
    let chunk = T::structure_named(
        "GlbChunk",
        "kind",
        "data",
        vec![
            ("length", T::u32(Little)),
            (
                "kind",
                T::enumeration_hex(
                    "GlbChunkType",
                    T::u32(Little),
                    &[(0x4e4f_534a, "JSON"), (0x004e_4942, "BIN")],
                ),
            ),
            ("data", T::bytes(E::field("length"))),
        ],
    )
    .counted_as("chunk");
    Template::new(
        "glb",
        T::structure(
            "GlbFile",
            vec![
                ("magic", T::magic(b"glTF")),
                ("version", T::u32(Little)),
                ("length", T::u32(Little)),
                ("chunks", T::repeat(chunk, Until::End)),
            ],
        ),
    )
}

fn md2() -> Template {
    let names = [
        "version",
        "skin_width",
        "skin_height",
        "frame_size",
        "skin_count",
        "vertex_count",
        "texcoord_count",
        "triangle_count",
        "gl_command_count",
        "frame_count",
        "skins_offset",
        "texcoords_offset",
        "triangles_offset",
        "frames_offset",
        "gl_commands_offset",
        "end_offset",
    ];
    u32_header("md2", "Md2Header", b"IDP2", &names)
}

fn md3() -> Template {
    Template::new(
        "md3",
        T::structure(
            "Md3Header",
            vec![
                ("magic", T::magic(b"IDP3")),
                ("version", T::u32(Little)),
                ("name", T::utf8_padded(E::lit(64), 0)),
                ("flags", T::u32(Little)),
                ("frame_count", T::u32(Little)),
                ("tag_count", T::u32(Little)),
                ("surface_count", T::u32(Little)),
                ("skin_count", T::u32(Little)),
                ("frames_offset", T::u32(Little)),
                ("tags_offset", T::u32(Little)),
                ("surfaces_offset", T::u32(Little)),
                ("end_offset", T::u32(Little)),
                ("data", T::bytes(E::Remaining)),
            ],
        ),
    )
}

fn iqm() -> Template {
    let names = [
        "version",
        "file_size",
        "flags",
        "text_count",
        "text_offset",
        "mesh_count",
        "mesh_offset",
        "vertex_array_count",
        "vertex_count",
        "vertex_array_offset",
        "triangle_count",
        "triangle_offset",
        "adjacency_offset",
        "joint_count",
        "joint_offset",
        "pose_count",
        "pose_offset",
        "animation_count",
        "animation_offset",
        "frame_count",
        "frame_channel_count",
        "frame_offset",
        "bounds_offset",
        "comment_count",
        "comment_offset",
        "extension_count",
        "extension_offset",
    ];
    u32_header("iqm", "IqmHeader", b"INTERQUAKEMODEL\0", &names)
}

fn u32_header(name: &str, kind: &str, magic: &'static [u8], names: &[&str]) -> Template {
    let mut fields = vec![("magic", T::magic(magic))];
    fields.extend(names.iter().map(|&field| (field, T::u32(Little))));
    fields.push(("data", T::bytes(E::Remaining)));
    Template::new(name, T::structure(kind, fields))
}

fn assbin() -> Template {
    Template::new(
        "assbin",
        T::structure(
            "AssimpBinaryDump",
            vec![
                ("signature", T::magic(b"ASSIMP.binary-dump.")),
                ("version_major", T::u32(Little)),
                ("version_minor", T::u32(Little)),
                ("version_revision", T::u32(Little)),
                ("compile_flags", T::u32(Little)),
                ("data", T::bytes(E::Remaining)),
            ],
        ),
    )
}

fn fbx() -> Template {
    Template::new(
        "fbx",
        T::structure(
            "FbxFile",
            vec![
                ("prefix", T::text(StrLen::Fixed(E::lit(4)), Encoding::Ascii)),
                (
                    "body",
                    T::switch(
                        E::field("prefix"),
                        vec![(
                            0x4b61_7964,
                            T::structure(
                                "FbxBinaryBody",
                                vec![
                                    ("signature_tail", T::magic(b"ara FBX Binary  \0\x1a\0")),
                                    ("version", T::u32(Little)),
                                    ("nodes", T::bytes(E::Remaining)),
                                ],
                            ),
                        )],
                        T::bytes(E::Remaining),
                    ),
                ),
            ],
        ),
    )
}

fn x_file() -> Template {
    Template::new(
        "x",
        T::structure(
            "DirectXFile",
            vec![
                ("magic", T::magic(b"xof ")),
                (
                    "version",
                    T::text(StrLen::Fixed(E::lit(4)), Encoding::Ascii),
                ),
                ("format", T::text(StrLen::Fixed(E::lit(4)), Encoding::Ascii)),
                (
                    "float_size",
                    T::text(StrLen::Fixed(E::lit(4)), Encoding::Ascii),
                ),
                ("data", T::bytes(E::Remaining)),
            ],
        ),
    )
}

fn usdc() -> Template {
    Template::new(
        "usdc",
        T::structure(
            "UsdCrate",
            vec![
                ("magic", T::magic(b"PXR-USDC")),
                ("version", T::u64(Little)),
                ("table_of_contents_offset", T::u64(Little)),
                ("data", T::bytes(E::Remaining)),
            ],
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_registry_name_builds() {
        let mut unique = std::collections::HashSet::new();
        for &name in NAMES {
            assert!(unique.insert(name), "duplicate Assimp template {name}");
            assert_eq!(template(name).as_ref().map(|t| t.name.as_str()), Some(name));
            assert!(
                super::super::builtin_names().contains(&name),
                "{name} is not offered by the app"
            );
        }
    }
}
