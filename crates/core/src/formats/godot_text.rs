//! `.tscn` and `.tres`: the same resources as [`super::godot`], written out.
//!
//! An editor saves these while a project is being worked on, because a text
//! file merges and a binary one does not. Exporting a game turns every one of
//! them into the binary form.
//!
//! The shape is INI-like and only INI-like. A line beginning `[` opens a
//! section and carries its own attributes inside the brackets, which no INI
//! file does; every other line is `key = value`, where the value is written in
//! the same syntax GDScript would use for it.
//!
//! **Lines, not values.** A value here is a GDScript expression, and some of
//! them run over several lines: Godot 4 writes a dictionary open-brace, a line
//! per entry, and a closing brace. Parsing that would mean parsing the
//! language. So this reads the file a line at a time and splits each one at
//! its first `=`, which is what the file itself is delimited by. A
//! continuation line has no `=` and reads as one field of text, which is the
//! honest answer: it is a line of an expression and nothing here knows which.
//!
//! **What this does not read.** The value syntax: a `Vector2(1, 2)` is the
//! nine characters that spell it, not two numbers. Neither is the inside of a
//! section header split into attributes, for the same reason: `path="a=b.gd"`
//! is legal and a split on `=` would cut it in half.

use crate::template::{Encoding, Endian::Big, Expr as E, StrLen, Template, Ty as T, Until};

/// The two words a Godot text resource can open with. A `.tscn` is a scene and
/// a `.tres` is a single resource, and the first line says which.
pub const SCENE: &[u8] = b"[gd_scene ";
pub const RESOURCE: &[u8] = b"[gd_resource ";

pub fn godot_text() -> Template {
    Template::new("godottext", T::structure("GodotText", vec![("lines", T::repeat(line(), Until::End))]))
}

/// One line, whatever kind it is.
///
/// Bounded to its own line before anything inside it is read. Without that, a
/// name that runs to the first `=` would run past the end of a line that has
/// none and swallow the line after it, which is exactly what the continuation
/// lines of a multi-line value are.
fn line() -> T {
    let length = E::to_marker(b'\n', &[]).add(E::lit(1)).at_most(E::Remaining);
    T::sized(
        length,
        T::switch(
            E::peek(8, Big),
            vec![
                (i128::from(b'['), section()),
                // A blank line, which is most of the structure a reader sees:
                // Godot puts one between every section.
                (i128::from(b'\n'), blank()),
                (i128::from(b'\r'), blank()),
                (i128::from(b';'), comment()),
            ],
            property(),
        ),
    )
}

/// `[node name="Sprite" type="Sprite2D" parent="."]`, and the several other
/// words that can open one.
///
/// Named by the word after the bracket, so a listing reads `ext_resource`,
/// `sub_resource`, `node`, `connection` down the side and the file's shape is
/// visible without opening anything.
fn section() -> T {
    T::structure_named(
        "Section",
        "kind",
        "attributes",
        vec![
            ("open", T::magic(b"[")),
            // To the space before the attributes, or to the bracket when there
            // are none: `[resource]` is a whole section header.
            ("kind", T::text(StrLen::Scan { skip: vec![], ends: vec![b' ', b']'], comment: None }, Encoding::Ascii)),
            ("attributes", T::text(StrLen::Terminated { end: b'\n', or_end: true }, Encoding::Utf8)),
        ],
    )
    .counted_as("section")
}

/// `position = Vector2(32, 48)`, and every line of a value that spilled over.
///
/// The name keeps the space the file wrote before the `=` and the value the
/// one after it. Trimming them would be inventing an offset: these fields say
/// where in the file the name and the value are, and the space is part of what
/// lies between them.
fn property() -> T {
    T::structure_named(
        "Property",
        "name",
        "value",
        vec![
            ("name", T::text(StrLen::Terminated { end: b'=', or_end: true }, Encoding::Utf8)),
            ("value", T::text(StrLen::Terminated { end: b'\n', or_end: true }, Encoding::Utf8)),
        ],
    )
    .counted_as("line")
}

fn blank() -> T {
    T::structure_named(
        "Blank",
        "",
        "text",
        vec![("text", T::text(StrLen::Terminated { end: b'\n', or_end: true }, Encoding::Utf8))],
    )
    .counted_as("line")
}

fn comment() -> T {
    T::structure_named(
        "Comment",
        "",
        "text",
        vec![("text", T::text(StrLen::Terminated { end: b'\n', or_end: true }, Encoding::Utf8))],
    )
    .counted_as("line")
}

/// Whether this is one of Godot's text resources.
///
/// The first line settles it. Both forms open with a word no other format
/// begins a file with, and asking for the space after it keeps this off a
/// file that merely starts with a bracket.
pub fn is_godot_text(head: &[u8], _len: u64) -> bool {
    head.starts_with(SCENE) || head.starts_with(RESOURCE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    const SAMPLE: &str = concat!(
        "[gd_scene load_steps=2 format=3]\n",
        "\n",
        "[ext_resource type=\"Texture2D\" path=\"res://icon.svg\" id=\"1\"]\n",
        "\n",
        "[node name=\"Root\" type=\"Node2D\"]\n",
        "position = Vector2(32, 48)\n",
        "metadata = {\n",
        "\"one\": 1\n",
        "}\n",
    );

    fn ev(text: &str) -> (Document<MemSource>, Evaluator) {
        (Document::new(MemSource(text.as_bytes().to_vec())), Evaluator::new(godot_text()))
    }

    #[test]
    fn every_line_of_the_file_is_one_row() {
        let (d, mut e) = ev(SAMPLE);
        assert_eq!(e.node(&d, &[0]).unwrap().child_count, 9);
    }

    #[test]
    fn a_section_is_named_by_the_word_after_the_bracket() {
        let (d, mut e) = ev(SAMPLE);
        assert_eq!(e.node(&d, &[0, 0, 1]).unwrap().value, Value::Str("gd_scene".into()));
        assert_eq!(e.node(&d, &[0, 2, 1]).unwrap().value, Value::Str("ext_resource".into()));
        assert_eq!(e.node(&d, &[0, 4, 1]).unwrap().value, Value::Str("node".into()));
    }

    #[test]
    fn a_property_line_splits_at_its_first_equals() {
        let (d, mut e) = ev(SAMPLE);
        assert_eq!(e.node(&d, &[0, 5, 0]).unwrap().value, Value::Str("position ".into()));
        assert_eq!(e.node(&d, &[0, 5, 1]).unwrap().value, Value::Str(" Vector2(32, 48)".into()));
    }

    /// The line after a multi-line value opens has no `=`, and must not reach
    /// into the line below it looking for one.
    #[test]
    fn a_continuation_line_stays_within_its_own_line() {
        let (d, mut e) = ev(SAMPLE);
        let opener = e.node(&d, &[0, 6]).unwrap();
        let carried = e.node(&d, &[0, 7]).unwrap();
        assert_eq!(carried.offset_bits, opener.offset_bits + opener.size_bits);
        // The whole line, newline and all: a field that ran to the end of its
        // container found no terminator to leave out of the reading.
        assert_eq!(e.node(&d, &[0, 7, 0]).unwrap().value, Value::Str("\"one\": 1\n".into()));
    }

    #[test]
    fn only_the_two_opening_words_are_recognised() {
        assert!(is_godot_text(b"[gd_scene load_steps=2 format=3]\n", 32));
        assert!(is_godot_text(b"[gd_resource type=\"Gradient\" format=3]\n", 38));
        assert!(!is_godot_text(b"[section]\nkey=value\n", 20));
        assert!(!is_godot_text(b"[gd_scene]\n", 11));
    }
}
