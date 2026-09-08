//! Godot's binary resource: what a `.res` and a `.scn` both are.
//!
//! One container serves the whole engine. A `.res` holds a material or a
//! gradient, a `.scn` holds a packed scene, and the difference between them is
//! a string in the header saying which class to instance. Everything else is
//! the same file: a header, a table of every string the file uses, a table of
//! the files it depends on, a table saying where each resource in it starts,
//! and then the resources, each a class name and a list of property values.
//!
//! A property value is a **variant**, which is the engine's own dynamic type
//! written down: a 32-bit tag and then as many bytes as that tag implies. The
//! tag numbering is deliberately not the engine's runtime `Variant::Type`
//! numbering, so that a new runtime type can be added in the middle without
//! moving every number in every file ever saved. Arrays and dictionaries hold
//! variants, so the type is recursive and so is this template.
//!
//! **Godot 3 against Godot 4.** The two write the same shape and disagree
//! about the details, and reading one as the other gives confident nonsense
//! rather than an error, so the version is settled before anything is read.
//! `format_version` is what decides: Godot 3 tops out at 3 and Godot 4 starts
//! at 4. Below that line the 56 bytes after the import-metadata offset are all
//! reserved; above it Godot 4 spends the first twelve of them on a flags word
//! and a UID and leaves eleven words reserved, so the two headers are the same
//! length and the string table starts at the same place in both. The variant
//! tags differ too: 4 is a `real` in Godot 3 and a `float` in Godot 4, 33 is a
//! `PoolRealArray` against a `PackedFloat32Array`, tag 21 was an inline image
//! and is gone, and everything from 42 up is Godot 4 only. A `format_version`
//! neither engine writes stops the read rather than guessing.
//!
//! **How wide a real is.** Godot 3 writes `real_t` at whatever width it was
//! compiled with and says nothing about it in the file, so this reads Godot 3
//! reals as `f32`, which is what every standard build writes; a
//! double-precision Godot 3 build writes a file nothing can tell apart. Godot
//! 4 fixed that by putting it in the header flags, and bit 2 is read here, so
//! a double-precision Godot 4 file reads correctly. Colours are the exception
//! at both ends: Godot 4 stores them as `float` whatever the flag says.
//!
//! **Byte order.** The word after the magic says it, and it is itself always
//! little-endian because the loader reads it before it knows. Both orders are
//! read here. A big-endian file is rare to the point of being an oddity now
//! (`ResourceSaver.FLAG_SAVE_BIG_ENDIAN` is the only way to make one, and no
//! current export target wants it), but it costs one switch to read and the
//! alternative was to misread every number in it.
//!
//! **What this does not read.**
//!
//! - The contents of a compressed resource (`RSCC`). The header, the block
//!   size and the table of per-block compressed sizes are read, and the blocks
//!   are left as runs of bytes. Each block is separately compressed and the
//!   resource straddles them, so unpacking one block would show half a header;
//!   putting them back together is a decoder, not a layout.
//! - Tag 21, an image written inline, and tag 25, an input event. Both are
//!   Godot 2 leftovers that Godot 3 kept a number for and dropped the reader
//!   for, and neither engine writes one. They are named, and reading one stops.
//! - `import_metadata_offset`. Every file this has been shown writes zero, and
//!   the editor-only structure it once pointed at is gone from both engines.
//! - What a resource *means*: a `PackedScene`'s node tree is a dictionary of
//!   parallel arrays under one property, and taking it apart is the scene
//!   format's business rather than the container's.
//! - Encrypted packs. See [`super::godot_pck`].

use crate::template::{Anchor, Encoding, Endian, Endian::*, Expr as E, StrLen, Template, Ty as T};

/// A resource written plainly. Also the last four bytes of one: the saver
/// writes the magic again at the end, which is how a reader that has been
/// handed a truncated file can tell.
pub const MAGIC: &[u8] = b"RSRC";
/// The same file with its body compressed. Only the third letter differs, and
/// nothing else in the two headers lines up.
pub const MAGIC_COMPRESSED: &[u8] = b"RSCC";

/// Which engine's rules this file follows. Settled from `format_version`,
/// which is the field that governs the encoding; `engine_version_major` says
/// the same thing and is what the file calls itself.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Ver {
    Three,
    Four,
}

const ENDIANNESS: &[(i128, &str)] = &[(0, "little"), (1, "big")];

const BOOL: &[(i128, &str)] = &[(0, "false"), (1, "true")];

const ABSOLUTE: &[(i128, &str)] = &[(0, "relative"), (1, "absolute")];

/// The header flags, Godot 4 only. Bit 0 says subresources are named rather
/// than numbered, bit 1 that the external resource table carries UIDs, bit 2
/// that every `real` in the file is a double, bit 3 that a script class name
/// follows the UID.
const HEADER_FLAGS: &[(u32, &str)] =
    &[(0, "named scene ids"), (1, "uids"), (2, "reals are doubles"), (3, "has script class")];

/// What an object-valued property points at.
const OBJECT_KIND: &[(i128, &str)] = &[
    (0, "nothing"),
    // The old way of naming a dependency, kept for files saved before format
    // 4: the type and the path written out where the value is, rather than an
    // index into the table at the front.
    (1, "external resource by path"),
    (2, "subresource"),
    (3, "ext_resource"),
];

/// Godot 4's variant tags. Cross-checked against the `enum` at the top of
/// `core/io/resource_format_binary.cpp` in godotengine/godot 4.7-stable.
const VARIANT_TYPE_4: &[(i128, &str)] = &[
    (1, "nil"),
    (2, "bool"),
    (3, "int"),
    (4, "float"),
    (5, "string"),
    (10, "vector2"),
    (11, "rect2"),
    (12, "vector3"),
    (13, "plane"),
    (14, "quaternion"),
    (15, "aabb"),
    (16, "basis"),
    (17, "transform3d"),
    (18, "transform2d"),
    (20, "color"),
    (22, "node_path"),
    (23, "rid"),
    (24, "object"),
    (25, "input_event"),
    (26, "dictionary"),
    (30, "array"),
    (31, "packed_byte_array"),
    (32, "packed_int32_array"),
    (33, "packed_float32_array"),
    (34, "packed_string_array"),
    (35, "packed_vector3_array"),
    (36, "packed_color_array"),
    (37, "packed_vector2_array"),
    (40, "int64"),
    (41, "double"),
    (42, "callable"),
    (43, "signal"),
    (44, "string_name"),
    (45, "vector2i"),
    (46, "rect2i"),
    (47, "vector3i"),
    (48, "packed_int64_array"),
    (49, "packed_float64_array"),
    (50, "vector4"),
    (51, "vector4i"),
    (52, "projection"),
    (53, "packed_vector4_array"),
];

/// Godot 3's, from the same enum on the 3.x branch. The names are Godot 3's
/// own: a `Quat` had not been renamed to a quaternion yet, a `Transform` was
/// the 3D one, and the packed arrays were pools.
const VARIANT_TYPE_3: &[(i128, &str)] = &[
    (1, "nil"),
    (2, "bool"),
    (3, "int"),
    (4, "real"),
    (5, "string"),
    (10, "vector2"),
    (11, "rect2"),
    (12, "vector3"),
    (13, "plane"),
    (14, "quat"),
    (15, "aabb"),
    (16, "basis"),
    (17, "transform"),
    (18, "transform2d"),
    (20, "color"),
    (21, "image"),
    (22, "node_path"),
    (23, "rid"),
    (24, "object"),
    (25, "input_event"),
    (26, "dictionary"),
    (30, "array"),
    (31, "pool_byte_array"),
    (32, "pool_int_array"),
    (33, "pool_real_array"),
    (34, "pool_string_array"),
    (35, "pool_vector3_array"),
    (36, "pool_color_array"),
    (37, "pool_vector2_array"),
    (40, "int64"),
    (41, "double"),
];

/// The block compressors a `RSCC` may have been written with, from
/// `Compression::Mode`.
const COMPRESSION: &[(i128, &str)] = &[(0, "fastlz"), (1, "deflate"), (2, "zstd"), (3, "gzip"), (4, "brotli")];

/// The name a variant type is registered under. Four of them: the tags differ
/// between the two engines and the numbers differ between the two byte orders,
/// and a variant reaches itself by name, so each combination needs its own.
fn variant_name(v: Ver, e: Endian) -> String {
    let engine = match v {
        Ver::Three => "3",
        Ver::Four => "4",
    };
    let order = match e {
        Little => "le",
        Big => "be",
    };
    format!("Variant{engine}{order}")
}

fn variant_ref(v: Ver, e: Endian) -> T {
    T::Named(variant_name(v, e).into())
}

/// Where a read gives up: everything left in the container, unread.
///
/// A variant says how long it is by saying what it is, so a tag with no
/// reading behind it leaves nothing to skip and no way to find the next
/// property. Taking no bytes and carrying on would place every field after
/// this one at the wrong offset and show numbers that look like answers, so
/// the rest of the resource is claimed as one run and the reader can see
/// exactly where the template stopped.
fn stop() -> T {
    T::bytes(E::Remaining)
}

/// A field that is there but empty: a variant whose tag is the whole of it.
fn nothing() -> T {
    T::bytes(E::lit(0))
}

/// A count of bytes and then that many bytes of UTF-8, the last of which is a
/// NUL. Every string in the file is written this way, and the count includes
/// the terminator, so a `length` of 1 is the empty string.
///
/// Named by its own text as well as read as it. The string table is a quarter
/// of a small resource and forty-odd rows of it, and a table of `[0]` to
/// `[45]` is a wall: what a reader wants from it is that row 3 is `position`.
fn string(e: Endian) -> T {
    T::structure_named(
        "String",
        "text",
        "text",
        vec![
            ("length", T::u32(e)),
            ("text", T::text(StrLen::Padded { size: E::field("length"), pad: 0 }, Encoding::Utf8)),
        ],
    )
}

/// Whether this file's reals are doubles, read off the header flags. Godot 4
/// only; see the module doc for what Godot 3 does instead.
fn reals_are_doubles() -> E {
    E::field("flags").bit(2)
}

/// A fixed run of reals under one name: a vector, a matrix, a colour.
fn reals(name: &str, comps: &[&str], e: Endian, wide: bool) -> T {
    let fields = comps.iter().map(|c| (*c, if wide { T::F64(e) } else { T::F32(e) })).collect();
    T::structure(name, fields)
}

/// The same, at whichever width this file's `real_t` turned out to be.
///
/// The choice is made once for the whole value rather than once per component.
/// A `Vector3` built out of three separately switched numbers is three times
/// the nodes for one answer, and a packed array of a million of them would be
/// three million switches over a flag that cannot change while the file is
/// being read.
fn geometry(name: &str, comps: &[&str], v: Ver, e: Endian) -> T {
    let narrow = reals(name, comps, e, false);
    match v {
        Ver::Three => narrow,
        Ver::Four => T::switch(reals_are_doubles(), vec![(1, reals(name, comps, e, true))], narrow),
    }
}

/// One real on its own, which is what a `float` property is.
fn real(v: Ver, e: Endian) -> T {
    match v {
        Ver::Three => T::F32(e),
        Ver::Four => T::switch(reals_are_doubles(), vec![(1, T::F64(e))], T::F32(e)),
    }
}

/// A count and then that many values of a type that does not depend on the
/// header: the packed integer and float arrays, and the packed strings.
fn packed(name: &str, elem: T, e: Endian) -> T {
    T::structure_named(
        name,
        "",
        "values",
        vec![("count", T::u32(e)), ("values", T::array(elem, E::field("count")))],
    )
}

/// The same for an array of reals, switched once above the array rather than
/// inside its elements. See [`geometry`].
fn packed_reals(name: &str, elem_name: &str, comps: &[&str], v: Ver, e: Endian) -> T {
    let narrow = T::array(reals(elem_name, comps, e, false), E::field("count"));
    let values = match v {
        Ver::Three => narrow,
        Ver::Four => {
            let wide = T::array(reals(elem_name, comps, e, true), E::field("count"));
            T::switch(reals_are_doubles(), vec![(1, wide)], narrow)
        }
    };
    T::structure_named(name, "", "values", vec![("count", T::u32(e)), ("values", values)])
}

/// A run of raw bytes, padded out to a multiple of four. The only packed array
/// that is padded, because it is the only one whose element is not already
/// four bytes wide or more.
fn packed_bytes(name: &str, e: Endian) -> T {
    T::structure_named(
        name,
        "",
        "values",
        vec![
            ("count", T::u32(e)),
            ("values", T::bytes(E::field("count"))),
            ("padding", T::bytes(E::field("count").pad_to(4))),
        ],
    )
}

/// A string written either as an index into the string table or as itself.
///
/// The top bit of the word decides. Set, the rest of it is a length and the
/// bytes follow; clear, the whole word is a row of the table at the front of
/// the file. Both read as the same thing here, so a node path shows the names
/// it is made of whichever way this file chose to store them.
fn string_id(e: Endian) -> T {
    T::structure_named(
        "StringId",
        "name",
        "name",
        vec![
            ("id", T::u32(e)),
            (
                "name",
                T::switch(
                    E::field("id").bit(31),
                    vec![(
                        1,
                        T::text(
                            StrLen::Padded { size: E::field("id").and(E::lit(0x7fff_ffff)), pad: 0 },
                            Encoding::Utf8,
                        ),
                    )],
                    // No bytes of its own: the word already read is a row
                    // number, and this is the row.
                    T::computed_text(E::elem_field("string_table", E::field("id"), &["text"])),
                ),
            ),
        ],
    )
}

/// A path to a node and, optionally, to a property of it: `../Sprite/Body`
/// and then `position:x`.
fn node_path(e: Endian) -> T {
    T::structure(
        "NodePath",
        vec![
            ("name_count", T::u16(e)),
            // The count with the "starts at the scene root" bit on top of it.
            // Not split into two fields: at 16 bits the two orders disagree
            // about where the top bit is, and this is the word the file wrote.
            ("subname_count", T::u16(e)),
            ("absolute", T::enumeration("Absolute", T::computed(E::field("subname_count").bit(15)), ABSOLUTE)),
            ("names", T::array(string_id(e), E::field("name_count"))),
            (
                "subnames",
                T::array(
                    string_id(e),
                    // Before format 3 a path always carried a property name as
                    // a subname and did not count it.
                    E::field("subname_count")
                        .and(E::lit(0x7fff))
                        .add(E::field("format_version").less_than(E::lit(3))),
                ),
            ),
        ],
    )
}

/// A property whose value is another resource: one of the tables at the front,
/// by index, or nothing at all.
fn object(e: Endian) -> T {
    T::structure_named(
        "Object",
        "kind",
        "reference",
        vec![
            ("kind", T::enumeration("ObjectKind", T::u32(e), OBJECT_KIND)),
            (
                "reference",
                T::switch(
                    E::field("kind"),
                    vec![
                        (0, nothing()),
                        (1, T::structure("ExternalPath", vec![("type", string(e)), ("path", string(e))])),
                        (2, T::u32(e)),
                        (3, T::u32(e)),
                    ],
                    stop(),
                ),
            ),
        ],
    )
}

/// A tag and then whatever that tag says. The recursive part of the format:
/// the array and dictionary cases hold these again.
fn variant(v: Ver, e: Endian) -> T {
    let names = match v {
        Ver::Three => VARIANT_TYPE_3,
        Ver::Four => VARIANT_TYPE_4,
    };
    T::structure_named(
        "Variant",
        "",
        "value",
        vec![
            ("type", T::enumeration("VariantType", T::u32(e), names)),
            ("value", T::switch(E::field("type"), variant_cases(v, e), stop())),
        ],
    )
}

/// The bytes behind each tag. Both engines share the numbering as far as they
/// share the types; see the module doc for where they part.
fn variant_cases(v: Ver, e: Endian) -> Vec<(i128, T)> {
    let pair = T::structure_named(
        "Pair",
        "key",
        "value",
        vec![("key", variant_ref(v, e)), ("value", variant_ref(v, e))],
    );
    // The top bit of both counts once meant the collection was shared between
    // properties rather than copied. Godot masks it off and so does this.
    let count = || E::field("count").and(E::lit(0x7fff_ffff));

    let mut cases: Vec<(i128, T)> = vec![
        (1, nothing()),
        (2, T::enumeration("Bool", T::u32(e), BOOL)),
        (3, T::i32(e)),
        (4, real(v, e)),
        (5, string(e)),
        (10, geometry("Vector2", &["x", "y"], v, e)),
        (11, geometry("Rect2", &["x", "y", "width", "height"], v, e)),
        (12, geometry("Vector3", &["x", "y", "z"], v, e)),
        (13, geometry("Plane", &["x", "y", "z", "d"], v, e)),
        (14, geometry("Quaternion", &["x", "y", "z", "w"], v, e)),
        (15, geometry("AABB", &["x", "y", "z", "width", "height", "depth"], v, e)),
        // Three basis vectors, written a row at a time.
        (16, geometry("Basis", &BASIS_COMPONENTS, v, e)),
        (17, geometry("Transform3D", &TRANSFORM3D_COMPONENTS, v, e)),
        // Two columns and then the translation, which is why it is six and not
        // four: a Transform2D carries its origin.
        (18, geometry("Transform2D", &["xx", "xy", "yx", "yy", "ox", "oy"], v, e)),
        (22, node_path(e)),
        (23, T::u32(e)),
        (24, object(e)),
        (
            26,
            T::structure_named(
                "Dictionary",
                "",
                "pairs",
                vec![("count", T::u32(e)), ("pairs", T::array(pair, count()))],
            ),
        ),
        (
            30,
            T::structure_named(
                "Array",
                "",
                "items",
                vec![("count", T::u32(e)), ("items", T::array(variant_ref(v, e), count()))],
            ),
        ),
        (40, T::Int { bits: 64, endian: e }),
        (41, T::F64(e)),
    ];

    // Where the two engines write the same bytes under different words, the
    // name is the engine's own. Where they write different bytes, so is the
    // reading: a Godot 3 `pool_real_array` is `real_t` wide and a Godot 4
    // `packed_float32_array` is always four bytes.
    match v {
        Ver::Three => cases.extend([
            (20, reals("Color", &["r", "g", "b", "a"], e, false)),
            (31, packed_bytes("PoolByteArray", e)),
            (32, packed("PoolIntArray", T::i32(e), e)),
            (33, packed("PoolRealArray", T::F32(e), e)),
            (34, packed("PoolStringArray", string(e), e)),
            (35, packed_reals("PoolVector3Array", "Vector3", &["x", "y", "z"], v, e)),
            (36, packed("PoolColorArray", reals("Color", &["r", "g", "b", "a"], e, false), e)),
            (37, packed_reals("PoolVector2Array", "Vector2", &["x", "y"], v, e)),
        ]),
        Ver::Four => cases.extend([
            // Always four floats, whatever the header says about reals: the
            // engine keeps colours single-precision on purpose.
            (20, reals("Color", &["r", "g", "b", "a"], e, false)),
            (31, packed_bytes("PackedByteArray", e)),
            (32, packed("PackedInt32Array", T::i32(e), e)),
            (33, packed("PackedFloat32Array", T::F32(e), e)),
            (34, packed("PackedStringArray", string(e), e)),
            (35, packed_reals("PackedVector3Array", "Vector3", &["x", "y", "z"], v, e)),
            (36, packed("PackedColorArray", reals("Color", &["r", "g", "b", "a"], e, false), e)),
            (37, packed_reals("PackedVector2Array", "Vector2", &["x", "y"], v, e)),
            // A callable and a signal are written as the tag alone: neither
            // survives being saved, and the engine reads back an empty one.
            (42, nothing()),
            (43, nothing()),
            (44, string(e)),
            (45, T::structure("Vector2i", vec![("x", T::i32(e)), ("y", T::i32(e))])),
            (
                46,
                T::structure(
                    "Rect2i",
                    vec![
                        ("x", T::i32(e)),
                        ("y", T::i32(e)),
                        ("width", T::i32(e)),
                        ("height", T::i32(e)),
                    ],
                ),
            ),
            (47, T::structure("Vector3i", vec![("x", T::i32(e)), ("y", T::i32(e)), ("z", T::i32(e))])),
            (48, packed("PackedInt64Array", T::Int { bits: 64, endian: e }, e)),
            (49, packed("PackedFloat64Array", T::F64(e), e)),
            (50, geometry("Vector4", &["x", "y", "z", "w"], v, e)),
            (
                51,
                T::structure(
                    "Vector4i",
                    vec![("x", T::i32(e)), ("y", T::i32(e)), ("z", T::i32(e)), ("w", T::i32(e))],
                ),
            ),
            (52, geometry("Projection", &PROJECTION_COMPONENTS, v, e)),
            (53, packed_reals("PackedVector4Array", "Vector4", &["x", "y", "z", "w"], v, e)),
        ]),
    }
    cases
}

/// Row-major, which is how the engine writes a basis and not how it indexes
/// one: `rows[0]` is the first three numbers.
const BASIS_COMPONENTS: [&str; 9] = ["xx", "xy", "xz", "yx", "yy", "yz", "zx", "zy", "zz"];

/// A basis and then the translation after it.
const TRANSFORM3D_COMPONENTS: [&str; 12] =
    ["xx", "xy", "xz", "yx", "yy", "yz", "zx", "zy", "zz", "ox", "oy", "oz"];

/// Four columns of four, written a column at a time.
const PROJECTION_COMPONENTS: [&str; 16] = [
    "xx", "xy", "xz", "xw", "yx", "yy", "yz", "yw", "zx", "zy", "zz", "zw", "wx", "wy", "wz", "ww",
];

/// One property of a resource: which string names it, and its value.
///
/// The name is either a row of the string table or written out here, by the
/// same top-bit rule a node path's names follow. Naming the row matters more
/// here than anywhere else in the file: a listing of forty properties reading
/// `[0]` to `[39]` says nothing, and the same listing reading `position`,
/// `texture`, `script` is the resource itself.
fn property(v: Ver, e: Endian) -> T {
    T::structure_named(
        "Property",
        "name",
        "value",
        vec![
            ("name_id", T::u32(e)),
            (
                "name",
                T::switch(
                    E::field("name_id").bit(31),
                    vec![(
                        1,
                        T::text(
                            StrLen::Padded { size: E::field("name_id").and(E::lit(0x7fff_ffff)), pad: 0 },
                            Encoding::Utf8,
                        ),
                    )],
                    T::computed_text(E::elem_field("string_table", E::field("name_id"), &["text"])),
                ),
            ),
            ("value", variant_ref(v, e)),
        ],
    )
}

/// One resource: the class to instance, and every property that had a value
/// worth storing.
///
/// Named by its own type, so that the listing and the treemap say
/// `StandardMaterial3D` and `PackedScene` rather than `[0]` and `[1]`.
fn resource(v: Ver, e: Endian) -> T {
    T::structure_named(
        "Resource",
        "type",
        "",
        vec![
            ("type", string(e)),
            ("property_count", T::u32(e)),
            ("properties", T::array(property(v, e), E::field("property_count"))),
        ],
    )
    .counted_as("resource")
}

/// A file this one depends on. Named by its path, which is the useful half:
/// the type is a class name and the path is where the texture actually is.
fn ext_resource(v: Ver, e: Endian) -> T {
    let mut fields = vec![("type", string(e)), ("path", string(e))];
    if v == Ver::Four {
        // A stable id for the dependency, so that moving a file in the editor
        // does not break every scene that used it. Only written when the
        // header says so, and absent rather than zero when it is not.
        fields.push((
            "uid",
            T::switch(E::field("flags").bit(1), vec![(1, T::u64(e))], nothing()),
        ));
    }
    T::structure_named("ExtResource", "path", "type", fields).counted_as("dependency")
}

/// Where one resource in this file starts. The path is `local://` and a name
/// in Godot 4 and `local://` and a number in Godot 3; the offset counts from
/// the start of the resource file.
fn internal_resource(e: Endian) -> T {
    T::structure_named(
        "InternalResource",
        "path",
        "",
        vec![("path", string(e)), ("offset", T::u64(e))],
    )
    .counted_as("resource")
}

/// Everything after the format version, laid out the way that version says.
fn contents(v: Ver, e: Endian) -> T {
    let mut fields: Vec<(&str, T)> = vec![
        // What to instance: `PackedScene` for a `.scn`, the resource's own
        // class for a `.res`.
        ("resource_type", string(e)),
        // Where the editor's import settings used to be kept. Zero in every
        // file either engine still writes.
        ("import_metadata_offset", T::u64(e)),
    ];
    match v {
        // Godot 3 leaves the whole run reserved.
        Ver::Three => fields.push(("reserved", T::array(T::u32(e), E::lit(14)))),
        Ver::Four => {
            fields.extend([
                ("flags", T::flags("ResourceFlags", T::u32(e), HEADER_FLAGS)),
                // Always written, and only meaningful when the flags say so;
                // an unset one is every bit set.
                ("uid", T::u64(e)),
                // The name of the script class this resource is an instance
                // of, for the loader to find without loading the script.
                ("script_class", T::switch(E::field("flags").bit(3), vec![(1, string(e))], nothing())),
                ("reserved", T::array(T::u32(e), E::lit(11))),
            ]);
        }
    }
    fields.extend([
        // Every name and short string in the file, written once and referred
        // to by row number from then on.
        ("string_table_size", T::u32(e)),
        ("string_table", T::array(string(e), E::field("string_table_size"))),
        ("ext_resource_count", T::u32(e)),
        ("ext_resources", T::array(ext_resource(v, e), E::field("ext_resource_count"))),
        ("internal_resource_count", T::u32(e)),
        ("internal_resources", T::array(internal_resource(e), E::field("internal_resource_count"))),
        // The resources themselves, each at the offset its own table row
        // gives. Bounded four bytes short of the end because the saver writes
        // the magic again there, and a resource that ran to the end of the
        // file would swallow it.
        (
            "resources",
            T::sized(
                E::Remaining.sub(E::lit(4)).at_least(E::lit(0)),
                T::pointer_list_records("internal_resources", &["offset"], Anchor::Origin, E::lit(0), resource(v, e)),
            ),
        ),
        ("end_magic", T::magic(MAGIC)),
    ]);
    let name = match v {
        Ver::Three => "Godot3Resource",
        Ver::Four => "Godot4Resource",
    };
    T::structure(name, fields)
}

/// The header, read the way round this file says it was written.
fn file(e: Endian) -> T {
    let three = contents(Ver::Three, e);
    let four = contents(Ver::Four, e);
    T::structure(
        "ResourceFile",
        vec![
            // Written by both engines as zero and read back by neither. What
            // says a file's reals are doubles is bit 2 of the Godot 4 flags.
            ("use_real64", T::u32(e)),
            ("engine_version_major", T::u32(e)),
            ("engine_version_minor", T::u32(e)),
            ("format_version", T::u32(e)),
            (
                "contents",
                T::switch(
                    E::field("format_version"),
                    vec![
                        (1, three.clone()),
                        (2, three.clone()),
                        (3, three),
                        (4, four.clone()),
                        (5, four.clone()),
                        (6, four),
                    ],
                    // A version from an engine that does not exist yet. Every
                    // offset and every table below here would be read at the
                    // wrong place, so none of them is.
                    stop(),
                ),
            ),
        ],
    )
}

/// A resource written plainly.
///
/// Wrapped in an origin so that the offsets in the internal resource table
/// count from the start of the resource rather than the start of the file,
/// which is the same place until a copy of one is embedded in something else.
fn uncompressed() -> T {
    T::origin(T::structure(
        "GodotResource",
        vec![
            ("magic", T::magic(MAGIC)),
            // Read little-endian whatever it says, because the loader reads it
            // before it knows which way round the file is.
            ("endianness", T::enumeration("Endianness", T::u32(Little), ENDIANNESS)),
            // Anything but zero means big-endian, which is how the loader
            // tests it.
            ("file", T::switch(E::field("endianness"), vec![(0, file(Little))], file(Big))),
        ],
    ))
}

/// The same file with its body compressed, block by block.
///
/// The wrapper is `FileAccessCompressed`, which is not specific to resources:
/// a fixed uncompressed block size, the total that unpacks to, and then one
/// compressed size per block. The leading `RSRC` is not in the compressed
/// stream, since this magic stands in for it; the trailing one is.
fn compressed() -> T {
    // One more block than the division gives, which is how the engine counts
    // them: a file that fills its blocks exactly still writes a last empty
    // one. Guarded against a block size of zero, which no writer produces and
    // a corrupt file might.
    let blocks = || E::field("uncompressed_size").div(E::field("block_size").at_least(E::lit(1))).add(E::lit(1));
    T::structure(
        "GodotResourceCompressed",
        vec![
            ("magic", T::magic(MAGIC_COMPRESSED)),
            ("compression", T::enumeration("Compression", T::u32(Little), COMPRESSION)),
            ("block_size", T::u32(Little)),
            ("uncompressed_size", T::u32(Little)),
            ("block_sizes", T::array(T::u32(Little), blocks())),
            ("blocks", T::array(T::bytes(E::elem("block_sizes", E::idx())), blocks())),
            // The wrapper writes its own magic at the end as well, the same
            // way the resource writes `RSRC` at the end of an uncompressed
            // one. Nothing reads it back; it is there to be looked at.
            ("end_magic", T::magic(MAGIC_COMPRESSED)),
        ],
    )
}

pub fn godot() -> Template {
    // Told apart by the magic. Two roots rather than two templates: a reader
    // who opens a `.scn` should not have to know in advance whether the
    // project had compression turned on.
    let rscc = i128::from(u32::from_be_bytes([b'R', b'S', b'C', b'C']));
    let root = T::switch(E::peek(32, Big), vec![(rscc, compressed())], uncompressed());
    let mut t = Template::new("godot", root);
    for e in [Little, Big] {
        for v in [Ver::Three, Ver::Four] {
            t = t.with_type(&variant_name(v, e), variant(v, e));
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

    /// A string the way the format writes one: the length counts the NUL.
    fn gstr(s: &str) -> Vec<u8> {
        let mut v = ((s.len() + 1) as u32).to_le_bytes().to_vec();
        v.extend_from_slice(s.as_bytes());
        v.push(0);
        v
    }

    fn u32le(n: u32) -> Vec<u8> {
        n.to_le_bytes().to_vec()
    }

    /// A whole Godot 4 resource built by hand: one string in the table, one
    /// dependency, one resource with three properties, one of them a vector
    /// and one an array holding two more variants.
    fn godot4_file() -> Vec<u8> {
        let mut b = MAGIC.to_vec();
        b.extend(u32le(0)); // little-endian
        b.extend(u32le(0)); // use_real64
        b.extend(u32le(4)); // engine major
        b.extend(u32le(3)); // engine minor
        b.extend(u32le(6)); // format version
        b.extend(gstr("Resource"));
        b.extend(0u64.to_le_bytes()); // import metadata offset
        b.extend(u32le(3)); // flags: named scene ids, uids
        b.extend(u64::MAX.to_le_bytes()); // uid: unset
        b.extend([0; 44]); // eleven reserved words

        b.extend(u32le(2)); // string table
        b.extend(gstr("position"));
        b.extend(gstr("tint"));

        b.extend(u32le(1)); // one dependency
        b.extend(gstr("Texture2D"));
        b.extend(gstr("res://icon.svg"));
        b.extend(0x1122_3344_5566_7788u64.to_le_bytes()); // its uid

        b.extend(u32le(1)); // one internal resource
        b.extend(gstr("local://1"));
        let offset_at = b.len();
        b.extend(0u64.to_le_bytes()); // patched below

        let start = b.len() as u64;
        b[offset_at..offset_at + 8].copy_from_slice(&start.to_le_bytes());

        b.extend(gstr("Resource"));
        b.extend(u32le(3)); // three properties

        b.extend(u32le(0)); // name: string table row 0, "position"
        b.extend(u32le(10)); // vector2
        b.extend(1.5f32.to_le_bytes());
        b.extend((-2.5f32).to_le_bytes());

        b.extend(u32le(1)); // name: row 1, "tint"
        b.extend(u32le(20)); // color
        for c in [1.0f32, 0.5, 0.25, 1.0] {
            b.extend(c.to_le_bytes());
        }

        // A name written out rather than pointed at, which is the other half
        // of the string rule.
        let inline = b"script\0";
        b.extend(u32le(0x8000_0000 | inline.len() as u32));
        b.extend_from_slice(inline);
        b.extend(u32le(30)); // array
        b.extend(u32le(2));
        b.extend(u32le(3)); // int
        b.extend((-7i32).to_le_bytes());
        b.extend(u32le(5)); // string
        b.extend(gstr("two"));

        b.extend_from_slice(MAGIC); // the magic again at the end
        b
    }

    fn ev(bytes: Vec<u8>) -> (Document<MemSource>, Evaluator) {
        (Document::new(MemSource(bytes)), Evaluator::new(godot()))
    }

    /// magic, endianness, then the file: version fields, then the contents.
    const CONTENTS: &[usize] = &[2, 4];

    #[test]
    fn the_header_says_which_engine_and_which_way_round() {
        let (d, mut e) = ev(godot4_file());
        assert_eq!(e.node(&d, &[1]).unwrap().value.as_int(), Some(0));
        assert_eq!(e.node(&d, &[2, 1]).unwrap().value, Value::UInt(4));
        assert_eq!(e.node(&d, &[2, 3]).unwrap().value, Value::UInt(6));
        // resource_type: the class this file instances.
        assert_eq!(e.node(&d, &[2, 4, 0, 1]).unwrap().value, Value::Str("Resource".into()));
    }

    #[test]
    fn the_tables_are_counted_by_the_words_before_them() {
        let (d, mut e) = ev(godot4_file());
        let at = |i: usize| [CONTENTS, &[i][..]].concat();
        assert_eq!(e.node(&d, &at(7)).unwrap().child_count, 2); // string table
        assert_eq!(e.node(&d, &at(9)).unwrap().child_count, 1); // ext resources
        assert_eq!(e.node(&d, &at(11)).unwrap().child_count, 1); // internal
        // Each row of the string table reads as the string in it, so a table
        // that is a quarter of the file is a list of words rather than a list
        // of numbers.
        assert!(e.node(&d, &at(7)).unwrap().child_count > 0);
        assert!(e.node(&d, &[CONTENTS, &[7, 0][..]].concat()).unwrap().name.contains("position"));
        assert!(e.node(&d, &[CONTENTS, &[7, 1][..]].concat()).unwrap().name.contains("tint"));
    }

    #[test]
    fn a_dependency_reads_as_its_type_path_and_uid() {
        let (d, mut e) = ev(godot4_file());
        let ext = [CONTENTS, &[9, 0][..]].concat();
        assert_eq!(e.node(&d, &[ext.clone(), vec![0, 1]].concat()).unwrap().value, Value::Str("Texture2D".into()));
        assert_eq!(
            e.node(&d, &[ext.clone(), vec![1, 1]].concat()).unwrap().value,
            Value::Str("res://icon.svg".into())
        );
        assert_eq!(e.node(&d, &[ext, vec![2]].concat()).unwrap().value, Value::UInt(0x1122_3344_5566_7788));
    }

    /// The resource is placed by the offset its own table row holds, not by
    /// where the tables happened to end.
    #[test]
    fn the_resource_sits_where_the_internal_table_points() {
        let (d, mut e) = ev(godot4_file());
        let table_offset = e.node(&d, &[CONTENTS, &[11, 0, 1][..]].concat()).unwrap().value;
        let placed = e.node(&d, &[CONTENTS, &[12, 0][..]].concat()).unwrap();
        assert_eq!(table_offset, Value::UInt((placed.offset_bits / 8).into()));
        assert_eq!(e.node(&d, &[CONTENTS, &[12, 0, 0, 1][..]].concat()).unwrap().value, Value::Str("Resource".into()));
        assert_eq!(e.node(&d, &[CONTENTS, &[12, 0, 2][..]].concat()).unwrap().child_count, 3);
    }

    /// A property name is a row of the string table, and the row is what the
    /// listing shows.
    #[test]
    fn a_property_is_named_by_the_string_table_row_it_points_at() {
        let (d, mut e) = ev(godot4_file());
        let prop = |i: usize| [CONTENTS, &[12, 0, 2, i][..]].concat();
        assert_eq!(e.node(&d, &[prop(0), vec![1]].concat()).unwrap().value, Value::Str("position".into()));
        assert_eq!(e.node(&d, &[prop(1), vec![1]].concat()).unwrap().value, Value::Str("tint".into()));
    }

    /// The other half of the rule: a name with the top bit set is written out
    /// where it is used and costs the table nothing.
    #[test]
    fn a_property_name_with_the_top_bit_set_is_written_out_in_place() {
        let (d, mut e) = ev(godot4_file());
        let name = [CONTENTS, &[12, 0, 2, 2, 1][..]].concat();
        assert_eq!(e.node(&d, &name).unwrap().value, Value::Str("script".into()));
    }

    #[test]
    fn a_vector_reads_as_two_floats_and_a_colour_as_four() {
        let (d, mut e) = ev(godot4_file());
        let value = |i: usize| [CONTENTS, &[12, 0, 2, i, 2, 1][..]].concat();
        assert_eq!(e.node(&d, &[value(0), vec![0]].concat()).unwrap().value, Value::Float(1.5));
        assert_eq!(e.node(&d, &[value(0), vec![1]].concat()).unwrap().value, Value::Float(-2.5));
        assert_eq!(e.node(&d, &[value(1), vec![1]].concat()).unwrap().value, Value::Float(0.5));
    }

    /// An array holds variants, so reading one is reading this type again.
    #[test]
    fn an_array_holds_variants_of_its_own() {
        let (d, mut e) = ev(godot4_file());
        let items = [CONTENTS, &[12, 0, 2, 2, 2, 1, 1][..]].concat();
        assert_eq!(e.node(&d, &items).unwrap().child_count, 2);
        assert_eq!(e.node(&d, &[items.clone(), vec![0, 1]].concat()).unwrap().value, Value::Int(-7));
        assert_eq!(e.node(&d, &[items, vec![1, 1, 1]].concat()).unwrap().value, Value::Str("two".into()));
    }

    /// The same file written the other way round. Only the first word after
    /// the magic is read the same in both, and it is what decides.
    #[test]
    fn a_big_endian_file_reads_the_same_values() {
        let mut b = MAGIC.to_vec();
        b.extend(1u32.to_le_bytes()); // big-endian from here on
        for n in [0u32, 3, 5, 3] {
            b.extend(n.to_be_bytes()); // real64, major, minor, format
        }
        b.extend(9u32.to_be_bytes());
        b.extend_from_slice(b"Resource\0");
        b.extend(0u64.to_be_bytes());
        b.extend([0; 56]); // Godot 3's fourteen reserved words
        b.extend(0u32.to_be_bytes()); // no strings
        b.extend(0u32.to_be_bytes()); // no dependencies
        b.extend(1u32.to_be_bytes()); // one resource
        b.extend(11u32.to_be_bytes());
        b.extend_from_slice(b"local://1\0\0");
        let offset_at = b.len();
        b.extend(0u64.to_be_bytes());
        let start = b.len() as u64;
        b[offset_at..offset_at + 8].copy_from_slice(&start.to_be_bytes());
        b.extend(9u32.to_be_bytes());
        b.extend_from_slice(b"Resource\0");
        b.extend(1u32.to_be_bytes()); // one property
        b.extend(0x8000_0005u32.to_be_bytes()); // an inline name, "size"
        b.extend_from_slice(b"size\0");
        b.extend(10u32.to_be_bytes()); // vector2
        b.extend(3.5f32.to_be_bytes());
        b.extend(4.5f32.to_be_bytes());
        b.extend_from_slice(MAGIC);

        let (d, mut e) = ev(b);
        assert_eq!(e.node(&d, &[1]).unwrap().value.as_int(), Some(1));
        assert_eq!(e.node(&d, &[2, 1]).unwrap().value, Value::UInt(3));
        let prop = [CONTENTS, &[9, 0, 2, 0][..]].concat();
        assert_eq!(e.node(&d, &[prop.clone(), vec![1]].concat()).unwrap().value, Value::Str("size".into()));
        assert_eq!(e.node(&d, &[prop, vec![2, 1, 0]].concat()).unwrap().value, Value::Float(3.5));
    }

    /// A compressed resource is a different file with a different header, and
    /// the magic is the only thing that says which one is in front of you.
    #[test]
    fn a_compressed_resource_reads_its_block_table() {
        let mut b = MAGIC_COMPRESSED.to_vec();
        b.extend(u32le(2)); // zstd
        b.extend(u32le(4096)); // block size
        b.extend(u32le(5000)); // what it unpacks to: two blocks
        b.extend(u32le(40));
        b.extend(u32le(9));
        b.extend(std::iter::repeat_n(0xccu8, 49));
        b.extend_from_slice(MAGIC_COMPRESSED);

        let (d, mut e) = ev(b);
        assert_eq!(e.node(&d, &[1]).unwrap().value.as_int(), Some(2));
        assert_eq!(e.node(&d, &[3]).unwrap().value, Value::UInt(5000));
        assert_eq!(e.node(&d, &[4]).unwrap().child_count, 2);
        // Each block is as long as its own row of the table says.
        assert_eq!(e.node(&d, &[5, 0]).unwrap().size_bits / 8, 40);
        assert_eq!(e.node(&d, &[5, 1]).unwrap().size_bits / 8, 9);
        assert_eq!(e.node(&d, &[6]).unwrap().size_bits / 8, 4);
    }

    /// A format version from no engine leaves everything below it unread
    /// rather than placing the tables at offsets guessed from the wrong
    /// layout.
    #[test]
    fn an_unknown_format_version_stops_rather_than_guessing() {
        let mut b = godot4_file();
        b[20] = 99; // format_version
        let (d, mut e) = ev(b);
        let contents = e.node(&d, &[2, 4]).unwrap();
        assert_eq!(contents.child_count, 0);
        assert!(contents.size_bits > 0);
    }
}
