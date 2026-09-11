//! Doom WAD: a header of three numbers, the lumps, and a directory at the end
//! saying where each of them is.
//!
//! The directory being last is what makes this worth writing down. A pointer
//! list places children at offsets read from an earlier field, so the offsets
//! have to be in hand before the bytes they point at are reached, and here
//! they are written after all of them. What closes the distance is a field
//! that costs no bytes and reads its contents somewhere else: the directory is
//! declared straight after the header, at the offset the header gives, and the
//! cursor does not move. The lumps are then a list over everything after the
//! header, one child per entry, each placed where its entry says.
//!
//! The directory sits inside that stretch and belongs to no lump. Being
//! declared first is what settles it: the cursor lands in the directory rather
//! than in the space between two lumps, and every byte of the file is named
//! once.
//!
//! A lump of size zero is a marker rather than a resource: `F_START`,
//! `S_END` and the names like them exist only to bracket the lumps between
//! them, which is how a level knows where its own graphics are. What a marker
//! writes for its offset was never agreed on: some builders write where the
//! next lump starts, others write zero. A zero could never be a lump, since
//! the header is there, so it is read as pointing at nothing.
//!
//! ## What is inside a lump
//!
//! Nothing in a WAD says what a lump holds. There is no type field, no magic
//! at the front of a lump, and no length beyond the byte count: the game knows
//! `THINGS` is a list of monsters because it is called `THINGS`, and that is
//! the whole of it. So the name is the type, and a lump is read by matching it:
//! see [`lump`].
//!
//! That works for the lumps with fixed names, which is the level data, the
//! palette, the colour maps and the sign-off screen. It does not work for the
//! ones whose name is their content: `D_E1M1` is music, `DSPISTOL` is a sound,
//! `TROOA1` is a sprite, and a template matching exact names cannot say so
//! without listing every lump in every WAD ever built. Those stay bytes, and
//! are the gap this file knows about.
//!
//! One lump with a fixed name stays bytes as well. `REJECT` is a bit per pair
//! of sectors saying whether either can see the other, so its shape is the
//! square of a number written in a different lump, and there is no reading of
//! one bit on its own worth showing. It is a table, and this is a file.
//!
//! The other thing the format cannot say is how many records a lump holds. A
//! level has as many monsters as its `THINGS` lump has room for, so the count
//! is the lump's length divided by the size of one record. That division is
//! where a truncated lump shows up: a `THINGS` of 25 bytes reads as two things
//! and three bytes nobody claims.
//!
//! ## Which game
//!
//! Doom's layout. Hexen widened two of the level records, adding five bytes of
//! script arguments to a thing and two to a linedef, and a Hexen map is told
//! from a Doom one by a `BEHAVIOR` lump. Reading that difference needs the
//! presence of one lump to change the shape of another, which this file does
//! not yet do, so a Hexen map's things and linedefs read at the wrong stride.
//! Heretic and Strife use Doom's own widths and read correctly.

use crate::formats::wad_names::{LINE_SPECIALS, SECTOR_SPECIALS, THING_TYPES};
use crate::template::{Anchor, Encoding, Endian::*, Expr as E, StrLen, Template, Ty as T};

pub fn wad() -> Template {
    Template::new(
        "wad",
        T::structure(
            "WAD",
            vec![
                // IWAD is a game; PWAD is a patch that replaces lumps in one.
                ("magic", T::text(StrLen::Fixed(E::lit(4)), Encoding::Ascii)),
                ("lump_count", T::i32(Little)),
                ("directory_offset", T::i32(Little)),
                // Read at the end of the file without going there, so the
                // lumps below can be placed by what it holds.
                ("directory", T::at(E::field("directory_offset"), T::array(entry(), E::field("lump_count")))),
                // Everything after the header, with each lump at the offset
                // its own entry names. The directory sits in that stretch too
                // and belongs to no lump; it is read by the field above, which
                // is declared first and so is the one the cursor lands in.
                (
                    "lumps",
                    T::pointer_list_sized("directory", &["offset"], Anchor::File, E::lit(0), lump()).skipping_zero(),
                ),
            ],
        ),
    )
}

/// One directory entry: where a lump is, how long it is, and what it is called.
fn entry() -> T {
    T::structure_named(
        "Lump",
        "name",
        "",
        vec![
            ("offset", T::i32(Little)),
            ("size", T::i32(Little)),
            // Eight bytes, NUL padded, and not NUL terminated when the name
            // fills them.
            ("name", T::text(StrLen::Padded { size: E::lit(8), pad: 0 }, Encoding::Ascii)),
        ],
    )
    .counted_as("lump")
}

/// How long this lump is, from its own directory entry.
fn size() -> E {
    E::elem_field("directory", E::idx(), &["size"])
}

/// What this lump is called, which is the only thing that says what is in it.
fn name() -> E {
    E::elem_field("directory", E::idx(), &["name"])
}

/// One lump, read as whatever its name says it is.
///
/// The names are the ones a level's own lumps have, plus the four resources a
/// WAD holds one of. Every other name falls to the default arm and is bytes,
/// which is the honest answer for a sprite or a sound: the name is the only
/// clue, and `TROOA1` says nothing a template can match.
fn lump() -> T {
    T::matches(
        name(),
        vec![
            // The level data, in the order a map's lumps are written.
            ("THINGS", records(thing(), 10)),
            ("LINEDEFS", records(linedef(), 14)),
            ("SIDEDEFS", records(sidedef(), 30)),
            ("VERTEXES", records(vertex(), 4)),
            ("SEGS", records(seg(), 12)),
            ("SSECTORS", records(subsector(), 4)),
            ("NODES", records(node(), 28)),
            ("SECTORS", records(sector(), 26)),
            ("BLOCKMAP", blockmap()),
            // The resources there is one of per WAD.
            ("PLAYPAL", playpal()),
            ("COLORMAP", colormap()),
            ("ENDOOM", endoom()),
            ("PNAMES", pnames()),
            ("TEXTURE1", textures()),
            ("TEXTURE2", textures()),
        ],
        T::bytes(size()),
    )
}

/// A lump that is a run of one record and nothing else, as many of them as its
/// length holds.
///
/// The count is worked out rather than read: a WAD says how long a lump is and
/// never how many records are in it, so the length divided by the stride is
/// the count, and a lump whose length is not a multiple of the stride ends in
/// bytes no record claims.
fn records(elem: T, each: i128) -> T {
    T::array(elem, size().div(E::lit(each)))
}

/// A monster, a pickup or a decoration, and where it stands.
fn thing() -> T {
    T::structure(
        "Thing",
        vec![
            ("x", T::Int { bits: 16, endian: Little }),
            ("y", T::Int { bits: 16, endian: Little }),
            // Degrees counterclockwise from east, and only the eight multiples
            // of 45 are used by the editors, though the format allows any.
            ("angle", T::Int { bits: 16, endian: Little }),
            ("type", T::enumeration("ThingType", T::u16(Little), THING_TYPES)),
            (
                "flags",
                T::flags(
                    "ThingFlags",
                    T::u16(Little),
                    &[
                        (0, "on skill 1 and 2"),
                        (1, "on skill 3"),
                        (2, "on skill 4 and 5"),
                        // Deaf here means it will not wake to a gunshot
                        // elsewhere, only to seeing the player.
                        (3, "deaf"),
                        (4, "multiplayer only"),
                    ],
                ),
            ),
        ],
    )
    .counted_as("thing")
}

/// A wall, or the boundary between two sectors. The two vertices are its ends,
/// and the two sidedefs are what is drawn on each face of it.
fn linedef() -> T {
    T::structure(
        "Linedef",
        vec![
            ("start_vertex", T::u16(Little)),
            ("end_vertex", T::u16(Little)),
            (
                "flags",
                T::flags(
                    "LinedefFlags",
                    T::u16(Little),
                    &[
                        (0, "blocks players and monsters"),
                        (1, "blocks monsters"),
                        (2, "two-sided"),
                        (3, "upper texture unpegged"),
                        (4, "lower texture unpegged"),
                        (5, "secret, drawn as one-sided"),
                        (6, "blocks sound"),
                        (7, "never on the automap"),
                        (8, "always on the automap"),
                    ],
                ),
            ),
            ("special", T::enumeration("LineSpecial", T::u16(Little), LINE_SPECIALS)),
            // Which sectors a special acts on: the ones whose own tag matches.
            ("sector_tag", T::u16(Little)),
            ("right_sidedef", T::u16(Little)),
            // 0xFFFF for a one-sided line, which has nothing on its back.
            ("left_sidedef", T::unset_int(T::u16(Little), 0xffff)),
        ],
    )
    .counted_as("linedef")
}

/// One face of a wall: which textures go on it, how they are shifted, and
/// which sector is behind it.
fn sidedef() -> T {
    T::structure(
        "Sidedef",
        vec![
            ("x_offset", T::Int { bits: 16, endian: Little }),
            ("y_offset", T::Int { bits: 16, endian: Little }),
            // Three slots, and a two-sided line usually fills only the middle
            // one. "-" means none, which is why these are not nullable: the
            // no-texture value is a name, not a number.
            ("upper_texture", texture_name()),
            ("lower_texture", texture_name()),
            ("middle_texture", texture_name()),
            ("sector", T::u16(Little)),
        ],
    )
    .counted_as("sidedef")
}

/// An eight-byte name, NUL padded, which is how a WAD writes every name it
/// has: a lump's, a texture's, a patch's.
fn texture_name() -> T {
    T::text(StrLen::Padded { size: E::lit(8), pad: 0 }, Encoding::Ascii)
}

/// A corner. Map units, and the whole level is built on these.
fn vertex() -> T {
    T::structure(
        "Vertex",
        vec![("x", T::Int { bits: 16, endian: Little }), ("y", T::Int { bits: 16, endian: Little })],
    )
    .counted_as("vertex")
}

/// Part of a linedef, as the node builder cut it up. What the renderer walks,
/// rather than what the level was drawn as.
fn seg() -> T {
    T::structure(
        "Seg",
        vec![
            ("start_vertex", T::u16(Little)),
            ("end_vertex", T::u16(Little)),
            // Binary angle measure: 0x4000 is a right angle, so the whole turn
            // fits the sixteen bits and there is no unit to get wrong.
            ("angle", T::Int { bits: 16, endian: Little }),
            ("linedef", T::u16(Little)),
            // 0 when the seg runs the same way as its linedef, 1 when against.
            ("direction", T::enumeration("SegDirection", T::u16(Little), &[(0, "along linedef"), (1, "against linedef")])),
            // How far along the linedef this seg starts, for lining the
            // texture up across the pieces.
            ("offset", T::Int { bits: 16, endian: Little }),
        ],
    )
    .counted_as("seg")
}

/// A convex piece of a sector, as a run of segs. The leaves of the BSP tree.
fn subsector() -> T {
    T::structure("Subsector", vec![("seg_count", T::u16(Little)), ("first_seg", T::u16(Little))])
        .counted_as("subsector")
}

/// One branch of the BSP tree: a line that splits the level, the box each side
/// of it fits in, and what is on each side.
///
/// A child is a node index, unless its top bit is set, in which case the low
/// fifteen bits are a subsector index instead. That is the tree's own
/// terminator: there is no separate leaf count, and the last node's children
/// are both subsectors. The bit is left in the number here rather than split
/// out, so what is shown is what is written.
fn node() -> T {
    T::structure(
        "Node",
        vec![
            ("x", T::Int { bits: 16, endian: Little }),
            ("y", T::Int { bits: 16, endian: Little }),
            ("dx", T::Int { bits: 16, endian: Little }),
            ("dy", T::Int { bits: 16, endian: Little }),
            ("right_box", bounding_box()),
            ("left_box", bounding_box()),
            ("right_child", T::u16(Little)),
            ("left_child", T::u16(Little)),
        ],
    )
    .counted_as("node")
}

/// The box a side of a node fits in. Top and bottom before left and right,
/// which is the order the file writes and not the order a reader expects.
fn bounding_box() -> T {
    T::inline_structure(
        "BoundingBox",
        vec![
            ("top", T::Int { bits: 16, endian: Little }),
            ("bottom", T::Int { bits: 16, endian: Little }),
            ("left", T::Int { bits: 16, endian: Little }),
            ("right", T::Int { bits: 16, endian: Little }),
        ],
    )
}

/// A room: how high its floor and ceiling are, what they are made of, how
/// bright it is, and what it does to whoever stands in it.
fn sector() -> T {
    T::structure(
        "Sector",
        vec![
            ("floor_height", T::Int { bits: 16, endian: Little }),
            ("ceiling_height", T::Int { bits: 16, endian: Little }),
            ("floor_texture", texture_name()),
            ("ceiling_texture", texture_name()),
            // 0 to 255, though the renderer works in steps of 16.
            ("light_level", T::u16(Little)),
            ("special", T::enumeration("SectorSpecial", T::u16(Little), SECTOR_SPECIALS)),
            // Which linedef specials reach this sector: the ones whose own tag
            // matches this number.
            ("tag", T::u16(Little)),
        ],
    )
    .counted_as("sector")
}

/// The grid the game uses to find, for a point, which linedefs are near it.
///
/// A header, then one offset per cell, then the lists those offsets point at.
/// The lists are left as bytes: they are 16-bit words, but the offsets are
/// counted in words from the start of the lump rather than in bytes from
/// anywhere, and two cells with the same linedefs in them are allowed to share
/// one list, so the lists are neither one per cell nor in any order. What is
/// read here is the part that says how big the grid is, which is the part a
/// reader is checking.
fn blockmap() -> T {
    T::structure(
        "Blockmap",
        vec![
            ("origin_x", T::Int { bits: 16, endian: Little }),
            ("origin_y", T::Int { bits: 16, endian: Little }),
            // Cells are 128 map units square.
            ("columns", T::u16(Little)),
            ("rows", T::u16(Little)),
            // One per cell, in words from the front of the lump.
            (
                "cell_offsets",
                T::array(T::u16(Little), E::field("columns").mul(E::field("rows"))).counted_as("cell"),
            ),
            // Clamped at nothing rather than allowed to go negative: a lump
            // cut short mid-grid would otherwise ask for a run of bytes of
            // less than no length, and losing the header's reading with it.
            (
                "lists",
                T::bytes(
                    size()
                        .sub(E::lit(8))
                        .sub(E::field("columns").mul(E::field("rows")).mul(E::lit(2)))
                        .at_least(E::lit(0)),
                ),
            ),
        ],
    )
}

/// The fourteen palettes. The first is the one a level is drawn in; the rest
/// are the tints the screen takes when the player is hurt, picks something up,
/// or puts on the radiation suit.
fn playpal() -> T {
    in_lump(T::array(T::array(rgb(), E::lit(256)).counted_as("colour"), E::lit(14)).counted_as("palette"))
}

/// `inner`, held to the length of the lump it is in.
///
/// The lumps with a fixed shape say their own size rather than reading it: a
/// palette is fourteen of 256 colours whatever the directory says. So a lump
/// cut short would read straight on into the lump after it, and naming bytes
/// twice is worse than running out of them. The window says where it stops.
fn in_lump(inner: T) -> T {
    T::sized(size(), inner)
}

fn rgb() -> T {
    T::inline_structure("Rgb", vec![("red", T::u8()), ("green", T::u8()), ("blue", T::u8())])
}

/// The thirty-four colour maps: for each of them, what each of the 256 palette
/// entries becomes. Thirty-two are the light levels, from full brightness down
/// to black; then the invulnerability map, which is the inverted greyscale, and
/// one all-black map the renderer never uses.
fn colormap() -> T {
    in_lump(T::array(T::array(T::u8(), E::lit(256)).counted_as("entry"), E::lit(34)).counted_as("map"))
}

/// The text screen the game prints as it exits: eighty columns by
/// twenty-five rows of a character and the colour to draw it in, which is a
/// DOS text mode screen written straight to the file.
///
/// The characters are CP437, and the ones below 0x20 are the pictures the
/// hardware drew rather than control codes, which is why this lump looks like
/// nothing at all in a text column showing ASCII.
fn endoom() -> T {
    in_lump(T::array(T::array(endoom_cell(), E::lit(80)).counted_as("column"), E::lit(25)).counted_as("row"))
}

fn endoom_cell() -> T {
    T::inline_structure(
        "ScreenCell",
        vec![
            ("character", T::text(StrLen::Fixed(E::lit(1)), Encoding::Cp437Screen)),
            // The text mode's attribute byte, and then the three things in it.
            // The byte is kept as well as taken apart: it is what is written,
            // and a reader comparing this file against a screen capture is
            // comparing bytes.
            ("attribute", T::u8()),
            ("foreground", T::enumeration("DosColour", T::computed(E::bit_field(E::field("attribute"), 3, 4)), DOS_COLOURS)),
            // Three bits, not four: the background has the eight dark colours
            // only, because the fourth bit went to blinking.
            ("background", T::enumeration("DosColour", T::computed(E::bit_field(E::field("attribute"), 6, 3)), DOS_COLOURS)),
            ("blink", T::computed(E::field("attribute").bit(7))),
        ],
    )
}

/// The sixteen colours of a DOS text screen, in the order the attribute byte
/// numbers them. Not a Doom fact: this is the IBM PC's own palette, which the
/// sign-off screen is written in because it is written straight to the screen.
const DOS_COLOURS: &[(i128, &str)] = &[
    (0, "black"),
    (1, "blue"),
    (2, "green"),
    (3, "cyan"),
    (4, "red"),
    (5, "magenta"),
    (6, "brown"),
    (7, "light grey"),
    (8, "dark grey"),
    (9, "light blue"),
    (10, "light green"),
    (11, "light cyan"),
    (12, "light red"),
    (13, "light magenta"),
    (14, "yellow"),
    (15, "white"),
];

/// The names of the patches a texture may be built from, which the texture
/// lumps refer to by their index in this list rather than by name.
fn pnames() -> T {
    in_lump(T::structure(
        "Pnames",
        vec![("count", T::i32(Little)), ("names", T::array(texture_name(), E::field("count")).counted_as("patch"))],
    ))
}

/// The wall textures: a count, one offset per texture, and the definitions
/// those offsets point at.
///
/// A texture is not an image. It is a width, a height, and a list of patches
/// to paste into that rectangle, which is how the game builds a wall out of
/// smaller pictures and why two walls can share most of their pixels.
fn textures() -> T {
    // The window is what makes the offsets land as well as what caps the
    // reading: they count from the front of this lump, and `Anchor::Window`
    // is the nearest `Sized` around the list, which this is.
    in_lump(
        T::structure(
            "TextureList",
            vec![
                ("count", T::i32(Little)),
                ("offsets", T::array(T::i32(Little), E::field("count")).counted_as("texture")),
                ("textures", T::pointer_list("offsets", Anchor::Window, E::lit(0), texture())),
            ],
        ),
    )
}

fn texture() -> T {
    T::structure_named(
        "Texture",
        "name",
        "",
        vec![
            ("name", texture_name()),
            // Written by the original build tools and read by nothing: the
            // game decides for itself whether a texture has holes in it.
            ("masked", T::i32(Little)),
            ("width", T::Int { bits: 16, endian: Little }),
            ("height", T::Int { bits: 16, endian: Little }),
            // A pointer the game overwrote at load time in 1993 and whose
            // written value means nothing.
            ("column_directory", T::i32(Little)),
            ("patch_count", T::Int { bits: 16, endian: Little }),
            ("patches", T::array(patch(), E::field("patch_count")).counted_as("patch")),
        ],
    )
    .counted_as("texture")
}

/// One picture pasted into a texture, and where in it.
fn patch() -> T {
    T::inline_structure(
        "TexturePatch",
        vec![
            ("origin_x", T::Int { bits: 16, endian: Little }),
            ("origin_y", T::Int { bits: 16, endian: Little }),
            // Into PNAMES, not a lump number.
            ("patch", T::Int { bits: 16, endian: Little }),
            // Both written and both ignored, the same story as
            // `column_directory`.
            ("step_dir", T::Int { bits: 16, endian: Little }),
            ("colormap", T::Int { bits: 16, endian: Little }),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    /// Three lumps: a named picture, a marker, and a music lump. None of them
    /// is one the template reads inside, which is what keeps this the test of
    /// the outer shape.
    fn wad_bytes() -> Vec<u8> {
        let lumps: [(&[u8], &[u8; 8]); 3] =
            [(b"pixels", b"TITLEPIC"), (b"", b"F_START\0"), (b"mus", b"D_E1M1\0\0")];
        build(&lumps)
    }

    /// The name an enumerated field read as, or None where the value is not
    /// one the table covers.
    fn named(v: &Value) -> Option<&str> {
        match v {
            Value::Enum { name, .. } => name.as_deref(),
            _ => None,
        }
    }

    /// A WAD holding whatever lumps it is given, header and directory worked
    /// out to match. A lump with no bytes writes zero for its offset, as some
    /// builders do.
    fn build(lumps: &[(&[u8], &[u8; 8])]) -> Vec<u8> {
        let mut data = Vec::new();
        let mut dir = Vec::new();
        for (body, name) in lumps {
            let at = if body.is_empty() { 0 } else { (12 + data.len()) as i32 };
            dir.extend_from_slice(&at.to_le_bytes());
            dir.extend_from_slice(&(body.len() as i32).to_le_bytes());
            dir.extend_from_slice(*name);
            data.extend_from_slice(body);
        }
        let mut v = b"IWAD".to_vec();
        v.extend_from_slice(&(lumps.len() as i32).to_le_bytes());
        v.extend_from_slice(&((12 + data.len()) as i32).to_le_bytes());
        v.extend_from_slice(&data);
        v.extend_from_slice(&dir);
        v
    }

    #[test]
    fn the_directory_reads_where_the_header_says_it_is() {
        let d = Document::new(MemSource(wad_bytes()));
        let mut ev = Evaluator::new(wad());
        assert_eq!(ev.node(&d, &[0]).unwrap().value, Value::Str("IWAD".into()));
        // The field costs nothing where it is declared.
        let directory = ev.node(&d, &[3]).unwrap();
        assert_eq!(directory.size_bits, 0);
        assert_eq!(directory.offset_bits, 12 * 8);
        // And its contents are at the end of the file.
        let entries = ev.node(&d, &[3, 0]).unwrap();
        assert_eq!(entries.offset_bits, 21 * 8);
        assert_eq!(entries.child_count, 3);
        assert_eq!(ev.node(&d, &[3, 0, 0, 2]).unwrap().value, Value::Str("TITLEPIC".into()));
        assert_eq!(ev.node(&d, &[3, 0, 2, 2]).unwrap().value, Value::Str("D_E1M1".into()));
    }

    #[test]
    fn every_lump_is_placed_by_its_own_entry() {
        let d = Document::new(MemSource(wad_bytes()));
        let mut ev = Evaluator::new(wad());
        let lumps = ev.node(&d, &[4]).unwrap();
        assert_eq!(lumps.child_count, 3);
        // The list covers everything after the header.
        assert_eq!(lumps.offset_bits, 12 * 8);

        let first = ev.node(&d, &[4, 0]).unwrap();
        assert_eq!(first.offset_bits, 12 * 8);
        assert_eq!(first.size_bits, 6 * 8);
        // A marker whose offset is zero points at nothing rather than at the
        // header, and the lump after it is not moved by it.
        assert_eq!(ev.node(&d, &[4, 1]).unwrap().size_bits, 0);
        assert_eq!(ev.node(&d, &[3, 0, 1, 0]).unwrap().value, Value::Int(0));
        assert_eq!(ev.node(&d, &[4, 2]).unwrap().offset_bits, 18 * 8);
        assert_eq!(ev.node(&d, &[4, 2]).unwrap().size_bits, 3 * 8);
    }

    #[test]
    fn the_cursor_finds_a_lump_and_a_directory_entry_alike() {
        let d = Document::new(MemSource(wad_bytes()));
        let mut ev = Evaluator::new(wad());
        // A byte of the third lump.
        assert_eq!(ev.locate(&d, 19 * 8).unwrap(), vec![4, 2]);
        // A byte of the directory, which sits past everything declared above
        // it and is still found.
        assert_eq!(ev.locate(&d, (21 + 8) * 8).unwrap(), vec![3, 0, 0, 2]);
    }

    #[test]
    fn the_linear_view_reads_the_file_in_the_order_it_is_written() {
        let d = Document::new(MemSource(wad_bytes()));
        let mut ev = Evaluator::new(wad());
        let spans = ev.spans(&d, 0, 21 * 8 + 48 * 8, 100).unwrap();
        let seen: Vec<_> = spans.iter().map(|s| (s.offset_bits / 8, s.size_bits / 8, s.name.clone(), s.gap)).collect();
        // The header, the lumps that hold bytes, and then the directory,
        // every byte named once and nothing named twice. The marker between
        // the two lumps covers nothing, so there is no row for it here; it is
        // still an entry in the directory below and a child of the list.
        let want: Vec<(u64, u64, &str, bool)> = vec![
            (0, 4, "magic", false),
            (4, 4, "lump_count", false),
            (8, 4, "directory_offset", false),
            (12, 6, "[0] TITLEPIC", false),
            (18, 3, "[2] D_E1M1", false),
            (21, 4, "offset", false),
            (25, 4, "size", false),
            (29, 8, "name", false),
            (37, 4, "offset", false),
            (41, 4, "size", false),
            (45, 8, "name", false),
            (53, 4, "offset", false),
            (57, 4, "size", false),
            (61, 8, "name", false),
        ];
        assert_eq!(seen, want.into_iter().map(|(a, b, c, d)| (a, b, c.to_string(), d)).collect::<Vec<_>>());
    }

    /// Two things: a player start facing east, and a shotgun guy that wakes to
    /// gunfire only on the top two skills.
    fn things_lump() -> Vec<u8> {
        let mut v = Vec::new();
        for (x, y, angle, kind, flags) in [(-64i16, 128i16, 90i16, 1u16, 0x07u16), (256, -32, 180, 9, 0x0c)] {
            v.extend_from_slice(&x.to_le_bytes());
            v.extend_from_slice(&y.to_le_bytes());
            v.extend_from_slice(&angle.to_le_bytes());
            v.extend_from_slice(&kind.to_le_bytes());
            v.extend_from_slice(&flags.to_le_bytes());
        }
        v
    }

    #[test]
    fn a_lump_is_read_by_the_name_it_is_filed_under() {
        let things = things_lump();
        let d = Document::new(MemSource(build(&[(&things, b"THINGS\0\0")])));
        let mut ev = Evaluator::new(wad());
        // Two records, because twenty bytes hold two of them. Nothing in the
        // file said "two".
        let lump = ev.node(&d, &[4, 0]).unwrap();
        assert_eq!(lump.child_count, 2);
        assert_eq!(ev.node(&d, &[4, 0, 0, 0]).unwrap().value, Value::Int(-64));
        assert_eq!(ev.node(&d, &[4, 0, 0, 1]).unwrap().value, Value::Int(128));
        assert_eq!(ev.node(&d, &[4, 0, 1, 2]).unwrap().value, Value::Int(180));
        // The type is named, not left as a number.
        let kind = ev.node(&d, &[4, 0, 1, 3]).unwrap();
        assert_eq!(named(&kind.value), Some("Former human sergeant"));
    }

    #[test]
    fn a_lump_nobody_named_is_still_its_own_bytes() {
        // A sprite, whose name cannot be matched by anything and should not be.
        let d = Document::new(MemSource(build(&[(b"pixels", b"TROOA1\0\0")])));
        let mut ev = Evaluator::new(wad());
        let lump = ev.node(&d, &[4, 0]).unwrap();
        assert_eq!(lump.size_bits, 6 * 8);
        assert_eq!(lump.child_count, 0);
    }

    #[test]
    fn a_lump_short_of_a_whole_record_reads_the_records_it_has() {
        // Twenty-five bytes of a fourteen-byte record: one linedef, and eleven
        // bytes the array does not reach.
        let d = Document::new(MemSource(build(&[(&[0u8; 25], b"LINEDEFS")])));
        let mut ev = Evaluator::new(wad());
        let lump = ev.node(&d, &[4, 0]).unwrap();
        assert_eq!(lump.child_count, 1);
        assert_eq!(ev.node(&d, &[4, 0, 0]).unwrap().size_bits, 14 * 8);
    }

    #[test]
    fn a_one_sided_line_has_nothing_on_its_back() {
        let mut line = Vec::new();
        // Two vertices, no flags, no special, no tag, right side 0, no left.
        for w in [0u16, 1, 0, 0, 0, 0, 0xffff] {
            line.extend_from_slice(&w.to_le_bytes());
        }
        let d = Document::new(MemSource(build(&[(&line, b"LINEDEFS")])));
        let mut ev = Evaluator::new(wad());
        assert_eq!(ev.node(&d, &[4, 0, 0, 5]).unwrap().value, Value::UInt(0));
        assert!(matches!(ev.node(&d, &[4, 0, 0, 6]).unwrap().value, Value::Unset(_)));
    }

    #[test]
    fn the_palette_is_fourteen_of_them_and_the_first_is_the_level() {
        let mut pal = Vec::new();
        for i in 0..14 * 256 {
            // A ramp, so that a colour read back can be checked against where
            // it came from.
            pal.extend_from_slice(&[(i % 256) as u8, 0, 255]);
        }
        let d = Document::new(MemSource(build(&[(&pal, b"PLAYPAL\0")])));
        let mut ev = Evaluator::new(wad());
        assert_eq!(ev.node(&d, &[4, 0]).unwrap().child_count, 14);
        assert_eq!(ev.node(&d, &[4, 0, 0]).unwrap().child_count, 256);
        // Palette 1, colour 2, green then blue.
        assert_eq!(ev.node(&d, &[4, 0, 1, 2, 1]).unwrap().value, Value::UInt(0));
        assert_eq!(ev.node(&d, &[4, 0, 1, 2, 2]).unwrap().value, Value::UInt(255));
    }

    #[test]
    fn the_exit_screen_is_a_dos_screen_and_reads_as_one() {
        let mut screen = Vec::new();
        // A heart in bright white on blue, which on a screen is a picture and
        // in ASCII is nothing.
        screen.extend_from_slice(&[0x03, 0x1f]);
        screen.extend_from_slice(&[b'D', 0x0c]);
        screen.resize(80 * 25 * 2, 0);
        let d = Document::new(MemSource(build(&[(&screen, b"ENDOOM\0\0")])));
        let mut ev = Evaluator::new(wad());
        assert_eq!(ev.node(&d, &[4, 0]).unwrap().child_count, 25);
        assert_eq!(ev.node(&d, &[4, 0, 0]).unwrap().child_count, 80);
        // 0x03 is a heart on a screen and a control character in the
        // encoding, which is the whole difference the screen page exists for.
        assert_eq!(ev.node(&d, &[4, 0, 0, 0, 0]).unwrap().value, Value::Str("\u{2665}".into()));
        assert_eq!(ev.node(&d, &[4, 0, 0, 1, 0]).unwrap().value, Value::Str("D".into()));
        // White on blue, not blinking: the attribute taken apart.
        assert_eq!(named(&ev.node(&d, &[4, 0, 0, 0, 2]).unwrap().value), Some("white"));
        assert_eq!(named(&ev.node(&d, &[4, 0, 0, 0, 3]).unwrap().value), Some("blue"));
        assert_eq!(ev.node(&d, &[4, 0, 0, 0, 4]).unwrap().value, Value::Int(0));
    }

    #[test]
    fn a_texture_names_the_patches_it_is_built_from() {
        let mut t = Vec::new();
        t.extend_from_slice(&1i32.to_le_bytes());
        // One texture, whose definition starts right after the offset.
        t.extend_from_slice(&8i32.to_le_bytes());
        t.extend_from_slice(b"STARTAN3");
        t.extend_from_slice(&0i32.to_le_bytes());
        t.extend_from_slice(&128i16.to_le_bytes());
        t.extend_from_slice(&128i16.to_le_bytes());
        t.extend_from_slice(&0i32.to_le_bytes());
        t.extend_from_slice(&2i16.to_le_bytes());
        for (x, patch) in [(0i16, 3i16), (64, 4)] {
            t.extend_from_slice(&x.to_le_bytes());
            t.extend_from_slice(&0i16.to_le_bytes());
            t.extend_from_slice(&patch.to_le_bytes());
            t.extend_from_slice(&0i16.to_le_bytes());
            t.extend_from_slice(&0i16.to_le_bytes());
        }
        let d = Document::new(MemSource(build(&[(&t, b"TEXTURE1")])));
        let mut ev = Evaluator::new(wad());
        assert_eq!(ev.node(&d, &[4, 0, 0]).unwrap().value, Value::Int(1));
        let texture = ev.node(&d, &[4, 0, 2, 0]).unwrap();
        assert_eq!(texture.child_count, 7);
        assert_eq!(ev.node(&d, &[4, 0, 2, 0, 0]).unwrap().value, Value::Str("STARTAN3".into()));
        assert_eq!(ev.node(&d, &[4, 0, 2, 0, 2]).unwrap().value, Value::Int(128));
        // The second patch sits 64 across and is PNAMES entry 4.
        assert_eq!(ev.node(&d, &[4, 0, 2, 0, 6, 1, 0]).unwrap().value, Value::Int(64));
        assert_eq!(ev.node(&d, &[4, 0, 2, 0, 6, 1, 2]).unwrap().value, Value::Int(4));
    }

    #[test]
    fn the_blockmap_reads_its_grid_and_leaves_the_lists_alone() {
        let mut b = Vec::new();
        for w in [-512i16, -256, 2, 3] {
            b.extend_from_slice(&w.to_le_bytes());
        }
        // Six cells, so six offsets, then whatever the lists are.
        for i in 0..6u16 {
            b.extend_from_slice(&(10 + i).to_le_bytes());
        }
        b.extend_from_slice(&[0xff; 8]);
        let d = Document::new(MemSource(build(&[(&b, b"BLOCKMAP")])));
        let mut ev = Evaluator::new(wad());
        assert_eq!(ev.node(&d, &[4, 0, 0]).unwrap().value, Value::Int(-512));
        assert_eq!(ev.node(&d, &[4, 0, 4]).unwrap().child_count, 6);
        assert_eq!(ev.node(&d, &[4, 0, 5]).unwrap().size_bits, 8 * 8);
    }
}
