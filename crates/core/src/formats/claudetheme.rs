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
            ("name", JsonSchema::named("theme name")),
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
        assert_eq!(ev.node(&d, &[0]).unwrap().type_name, "theme name");
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
        let at = (base.offset_bits / 8) as usize;
        assert_eq!(&SAMPLE.as_bytes()[at..at + (base.size_bits / 8) as usize], b"\"dark\"");
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
