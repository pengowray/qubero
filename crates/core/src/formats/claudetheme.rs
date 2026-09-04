//! A Claude Code colour theme: one JSON file per theme, in a `themes`
//! directory.
//!
//! Claude Code, the command line tool, keeps a custom theme in
//! `~/.claude/themes/<slug>.json`, and a plugin may ship a `themes` directory
//! of its own. The file is small and always the same three keys:
//!
//! ```json
//! {
//!   "name": "My theme",
//!   "base": "dark",
//!   "overrides": { "claude": "rgb(215,119,87)", "error": "ansi:red" }
//! }
//! ```
//!
//! `base` is one of the six themes that ship with the tool: `dark`, `light`,
//! `dark-daltonized`, `light-daltonized`, `dark-ansi`, `light-ansi`.
//! `overrides` replaces colours of that base, one entry per token, and a
//! colour is written one of four ways: `rgb(r,g,b)`, `#rrggbb`, `ansi256(n)`
//! or `ansi:name`.
//!
//! Nothing here lists the tokens a theme may override. The tool adds them
//! release by release, so a list would be a list of what was known the day it
//! was written; every entry of `overrides` is a colour whether or not this
//! build has heard of the token naming it.
//!
//! The file has no layout to declare: JSON keys sit wherever the writer put
//! them. What the template says instead is what the keys it knows mean, which
//! is [`JsonSchema`], so `base` reads as a base theme and each override as a
//! colour rather than as three anonymous strings.

use crate::template::{JsonSchema, Template, Ty as T};

/// The six themes that ship with the tool, which are the values `base` takes.
pub const BASES: [&str; 6] = ["dark", "light", "dark-daltonized", "light-daltonized", "dark-ansi", "light-ansi"];

/// What the type column says for each part of the file.
fn schema() -> JsonSchema {
    JsonSchema::object(
        None,
        vec![
            // One of `BASES`. A string has no names for its values the way an
            // integer field has an enum, so which six they are is said in the
            // module's own notes and in the recogniser that checks them.
            ("base", JsonSchema::named("base theme")),
            ("overrides", JsonSchema::table("colours", JsonSchema::named("colour"))),
        ],
    )
}

pub fn claudetheme() -> Template {
    Template::new("claudetheme", T::json_as(schema()))
}

/// Whether these bytes are a Claude Code theme.
///
/// Two ways of answering, because the file is JSON and JSON has no signature.
/// When the whole file is in hand it is parsed, and the answer is the shape of
/// what came back: a top-level object with a `base` naming one of the six
/// built-in themes and an `overrides` object. When the file is longer than the
/// window `sniff` reads, there is no parse to be had, so the same two facts
/// are looked for in the text. Either way `base` has to be one of the six:
/// `{"name": ..., "base": ..., "overrides": ...}` with anything at all in it
/// is a shape other files could share, and the six names are what make it
/// this one.
pub fn is_claude_theme(head: &[u8], len: u64) -> bool {
    let text = head.trim_ascii_start();
    if !text.starts_with(b"{") {
        return false;
    }
    if len <= head.len() as u64 {
        return parsed(head);
    }
    scanned(text)
}

/// The file, read as the JSON it claims to be.
fn parsed(head: &[u8]) -> bool {
    use crate::json::{parse, Kind};
    let Ok(val) = parse(head) else { return false };
    let Kind::Object(members) = &val.kind else { return false };
    let member = |key: &str| members.iter().find(|(k, _)| k == key).map(|(_, v)| &v.kind);
    let base_named = matches!(member("base"), Some(Kind::Text(s)) if BASES.contains(&s.as_str()));
    base_named && matches!(member("overrides"), Some(Kind::Object(_)))
}

/// The same two facts, found in text rather than in a parse, for a file too
/// long to have been read whole.
fn scanned(text: &[u8]) -> bool {
    let base = BASES.iter().any(|b| after(text, b"\"base\"", format!("\"{b}\"").as_bytes()));
    base && after(text, b"\"overrides\"", b"{")
}

/// Whether `key` appears with `want` after it, separated by a colon and
/// whatever spacing the writer used. The tool writes `"base": "dark"`; a file
/// written by hand may put the colon anywhere.
fn after(text: &[u8], key: &[u8], want: &[u8]) -> bool {
    text.windows(key.len()).enumerate().filter(|(_, w)| *w == key).any(|(at, _)| {
        let rest = text[at + key.len()..].trim_ascii_start();
        let Some(rest) = rest.strip_prefix(b":") else { return false };
        rest.trim_ascii_start().starts_with(want)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    /// A theme of the shape the tool writes, with all four colour syntaxes
    /// and one token this build has never heard of.
    pub(crate) const SAMPLE: &str = concat!(
        "{\n",
        "  \"name\": \"Ember\",\n",
        "  \"base\": \"dark\",\n",
        "  \"overrides\": {\n",
        "    \"claude\": \"rgb(215,119,87)\",\n",
        "    \"permission\": \"#5769f7\",\n",
        "    \"success\": \"ansi256(34)\",\n",
        "    \"error\": \"ansi:red\",\n",
        "    \"emberGlow\": \"#ff7a18\"\n",
        "  }\n",
        "}\n",
    );

    fn eval() -> (Document<MemSource>, Evaluator) {
        (Document::new(MemSource(SAMPLE.as_bytes().to_vec())), Evaluator::new(claudetheme()))
    }

    #[test]
    fn the_three_keys_are_the_three_rows() {
        let (d, mut ev) = eval();
        let root = ev.node(&d, &[]).unwrap();
        assert_eq!(root.child_count, 3);
        let names: Vec<_> = (0..3).map(|i| ev.node(&d, &[i]).unwrap().name).collect();
        assert_eq!(names, ["name", "base", "overrides"]);
    }

    #[test]
    fn a_key_the_template_knows_is_typed_as_what_it_is() {
        let (d, mut ev) = eval();
        // Nothing renames the display name: a string is what it is.
        assert_eq!(ev.node(&d, &[0]).unwrap().type_name, "string");
        let base = ev.node(&d, &[1]).unwrap();
        assert_eq!(base.type_name, "base theme");
        assert_eq!(base.value, Value::Str("dark".into()));
        assert_eq!(ev.node(&d, &[2]).unwrap().type_name, "colours");
    }

    #[test]
    fn every_override_is_a_colour_whether_or_not_the_token_is_known() {
        let (d, mut ev) = eval();
        assert_eq!(ev.node(&d, &[2]).unwrap().child_count, 5);
        let seen: Vec<_> = (0..5)
            .map(|i| ev.node(&d, &[2, i]).unwrap())
            .map(|n| (n.name, n.type_name, n.value))
            .collect();
        assert_eq!(seen[0], ("claude".into(), "colour".into(), Value::Str("rgb(215,119,87)".into())));
        assert_eq!(seen[3], ("error".into(), "colour".into(), Value::Str("ansi:red".into())));
        // A token added since this was written is still an override, and is
        // still a colour.
        assert_eq!(seen[4], ("emberGlow".into(), "colour".into(), Value::Str("#ff7a18".into())));
    }

    #[test]
    fn a_value_sits_where_its_text_does() {
        let (d, mut ev) = eval();
        let base = ev.node(&d, &[1]).unwrap();
        // The member is its key, its value and the comma after it, so that the
        // members of the theme tile it with no bytes left between them.
        let at = (base.offset_bits / 8) as usize;
        assert_eq!(&SAMPLE.as_bytes()[at..at + (base.size_bits / 8) as usize], b"\"base\": \"dark\",\n  ");
        // The value is still known apart from it, and is what it always was.
        let val = (base.value_offset_bits / 8) as usize;
        assert_eq!(&SAMPLE.as_bytes()[val..val + base.value_bytes as usize], b"\"dark\"");
    }

    #[test]
    fn the_members_of_an_object_tile_it_bar_its_braces() {
        let (d, mut ev) = eval();
        for (path, n) in [(vec![], 3usize), (vec![2], 5)] {
            let parent = ev.node(&d, &path).unwrap();
            let kids: Vec<_> = (0..n)
                .map(|i| {
                    let mut p = path.clone();
                    p.push(i);
                    ev.node(&d, &p).unwrap()
                })
                .collect();
            for pair in kids.windows(2) {
                assert_eq!(pair[0].offset_bits + pair[0].size_bits, pair[1].offset_bits, "{:?} tiles", path);
            }
            // What is left over is the object's own punctuation: the brace and
            // the whitespace inside it at either end, and nothing more.
            let first = kids.first().unwrap();
            let last = kids.last().unwrap();
            let at = (parent.value_offset_bits / 8) as usize;
            let head = ((first.offset_bits / 8) as usize) - at;
            assert!(SAMPLE.as_bytes()[at..at + head].iter().all(|c| c.is_ascii_whitespace() || *c == b'{'), "{:?} head", path);
            let tail_at = ((last.offset_bits + last.size_bits) / 8) as usize;
            let tail_end = at + parent.value_bytes as usize;
            assert!(
                SAMPLE.as_bytes()[tail_at..tail_end].iter().all(|c| c.is_ascii_whitespace() || *c == b'}'),
                "{:?} tail",
                path
            );
        }
        // And the object is framed, so a listing knows the leftovers are its
        // syntax rather than bytes nothing describes.
        assert!(ev.node(&d, &[2]).unwrap().framed);
        assert!(!ev.node(&d, &[1]).unwrap().framed, "a string frames nothing");
    }

    #[test]
    fn a_nested_member_covers_its_key_and_its_comma() {
        let (d, mut ev) = eval();
        let permission = ev.node(&d, &[2, 1]).unwrap();
        let at = (permission.offset_bits / 8) as usize;
        assert_eq!(
            &SAMPLE.as_bytes()[at..at + (permission.size_bits / 8) as usize],
            b"\"permission\": \"#5769f7\",\n    "
        );
        assert_eq!(permission.value, Value::Str("#5769f7".into()));
        let val = (permission.value_offset_bits / 8) as usize;
        assert_eq!(&SAMPLE.as_bytes()[val..val + permission.value_bytes as usize], b"\"#5769f7\"");
    }

    #[test]
    fn the_hex_column_calls_a_brace_the_object_it_belongs_to() {
        let (d, mut ev) = eval();
        let spans = ev.spans(&d, 0, SAMPLE.len() as u64 * 8, 200).unwrap();
        assert!(spans.iter().all(|s| !s.gap), "no run of a theme is unaccounted for");
        // The first entry is the file's own brace, named after the file.
        assert_eq!(spans[0].name, "file");
    }

    /// Write `text` into the member at `path` and hand back what the file
    /// then holds, or what the editor refused to do.
    fn write(path: &[usize], text: &str) -> Result<String, String> {
        let (mut d, mut ev) = eval();
        let w = match ev.prepare_write(&d, path, text) {
            Ok(w) => w,
            Err(crate::eval::EvalError::Failed(why)) => return Err(why),
            Err(other) => panic!("{other:?}"),
        };
        d.replace_bits(w.offset_bits, &w.data, w.n_bits, w.old_bits);
        ev.invalidate();
        let mut out = vec![0u8; (d.len_bits() / 8) as usize];
        d.read_bytes(0, &mut out);
        Ok(String::from_utf8(out).unwrap())
    }

    /// The members of the file after an edit, read back through the template.
    fn members(text: &str) -> Vec<(String, Value)> {
        let d = Document::new(MemSource(text.as_bytes().to_vec()));
        let mut ev = Evaluator::new(claudetheme());
        let n = ev.node(&d, &[2]).unwrap().child_count;
        (0..n as usize).map(|i| ev.node(&d, &[2, i]).unwrap()).map(|n| (n.name, n.value)).collect()
    }

    #[test]
    fn a_longer_string_pushes_the_members_after_it_along() {
        let after = write(&[2, 1], "rgb(87,105,247)").unwrap();
        assert!(after.contains("\"permission\": \"rgb(87,105,247)\","), "{after}");
        // Nothing else moved into anything else: every member is still there,
        // still named what it was, and still worth what it was.
        let seen = members(&after);
        assert_eq!(seen.len(), 5);
        assert_eq!(seen[1], ("permission".into(), Value::Str("rgb(87,105,247)".into())));
        assert_eq!(seen[4], ("emberGlow".into(), Value::Str("#ff7a18".into())));
    }

    #[test]
    fn a_shorter_string_pulls_them_back() {
        let after = write(&[2, 1], "red").unwrap();
        assert!(after.contains("\"permission\": \"red\","), "{after}");
        let seen = members(&after);
        assert_eq!(seen[1], ("permission".into(), Value::Str("red".into())));
        assert_eq!(seen[2], ("success".into(), Value::Str("ansi256(34)".into())));
    }

    #[test]
    fn a_string_of_the_same_length_leaves_everything_where_it_was() {
        let after = write(&[2, 1], "#123456").unwrap();
        assert_eq!(after, SAMPLE.replace("#5769f7", "#123456"));
    }

    #[test]
    fn what_json_has_no_other_way_of_holding_is_escaped() {
        let after = write(&[0], "a \"quoted\" \\ line\nbreak\u{1}\u{e9}").unwrap();
        let acc = char::from_u32(0xe9).unwrap();
        let want = format!("{}{}{}", r#""name": "a \"quoted\" \\ line\nbreak\u0001"#, acc, '"');
        assert!(after.contains(&want), "{after}");
        // And reads back as exactly what was typed.
        let d = Document::new(MemSource(after.as_bytes().to_vec()));
        let mut ev = Evaluator::new(claudetheme());
        assert_eq!(ev.node(&d, &[0]).unwrap().value, Value::Str("a \"quoted\" \\ line\nbreak\u{1}\u{e9}".into()));
    }

    #[test]
    fn an_object_is_edited_by_its_members_rather_than_whole() {
        let err = write(&[2], "{}").unwrap_err();
        assert!(err.contains("can't be edited here"), "{err}");
        assert!(write(&[], "{}").is_err());
    }

    #[test]
    fn a_string_holds_a_string_whatever_it_says() {
        // Typing a word that means something else in JSON writes the word.
        let after = write(&[1], "true").unwrap();
        assert!(after.contains("\"base\": \"true\","), "{after}");
    }

    #[test]
    fn the_six_base_themes_are_what_a_theme_is_recognised_by() {
        for base in BASES {
            let text = SAMPLE.replace("\"dark\"", &format!("\"{base}\""));
            assert!(is_claude_theme(text.as_bytes(), text.len() as u64), "{base}");
        }
        let other = SAMPLE.replace("\"dark\"", "\"midnight\"");
        assert!(!is_claude_theme(other.as_bytes(), other.len() as u64));
    }

    #[test]
    fn a_theme_too_long_to_read_whole_is_told_from_its_text() {
        // The head is all that was read, and the file goes on past it.
        assert!(is_claude_theme(SAMPLE.as_bytes(), 1 << 20));
        let no_base = r##"{"name": "x", "overrides": {"claude": "#000000"}}"##;
        assert!(!is_claude_theme(no_base.as_bytes(), 1 << 20));
    }

    #[test]
    fn a_theme_needs_both_a_base_and_overrides() {
        let only_base = r#"{"name": "x", "base": "dark"}"#;
        assert!(!is_claude_theme(only_base.as_bytes(), only_base.len() as u64));
        let only_overrides = r##"{"overrides": {"claude": "#000000"}}"##;
        assert!(!is_claude_theme(only_overrides.as_bytes(), only_overrides.len() as u64));
    }
}
