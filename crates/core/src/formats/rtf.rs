//! Rich Text Format: braces, backslashes, and everything else is the text.
//!
//! A group is `{`, the things inside it, and `}`. A control word is `\`, a run
//! of letters, an optional number, and an optional single space that is part
//! of the control word rather than part of the text. A control symbol is `\`
//! and one byte that is not a letter: `\\`, `\{` and `\}` are those three
//! characters written literally, and `\'` is the odd one out, taking two hex
//! digits after it for a byte the code page decides the meaning of. Anything
//! that is none of those is text, running to the next byte that is one.
//!
//! So an item is told apart by its own first byte, the way a bencoded value
//! is, and the marker field takes that byte when it is one of the three that
//! mean something and no bytes at all when it is text. The `}` that closes a
//! group is an item in its own right, which is what lets a group's contents be
//! a run of items ending at the one whose marker is that brace.
//!
//! An item's row is named by what it holds: a control word by its word, text
//! by the text, and a group by its first item, which for every group the
//! specification defines is the control word that says what the group is.
//! The font table reads `fonttbl`, and a group that opens `\*` reads as that
//! star, since the word after it is the second item.
//!
//! `\binN` is the one control word with bytes of its own: N of them follow
//! its space, raw, and a brace among them is not markup. They are the control
//! word's operand, so they are read as a field of it, as long as the
//! parameter beside them says.
//!
//! One thing this does not read: `\ansicpg1252` says what the bytes of the
//! text mean, and may be written anywhere before them. Text is read as
//! Latin-1 instead, which agrees with Windows-1252 everywhere but the 32
//! characters at 0x80, and `\uN` spells a character out as a number that is
//! read here as the parameter it is.
//!
//! `\*\name`, which marks a destination a reader that does not know the name
//! is meant to skip, needs nothing special: the `\*` is the control symbol it
//! is, and what follows it reads the same as anything else.

use crate::template::{Encoding, Endian::*, Expr as E, StrLen, Template, Ty as T, Until};

/// The three bytes that mean something. Everything else is text, and a run of
/// text ends at the first of these.
const MARKUP: &[u8] = b"{}\\";

/// What a control word's name is made of. Lower case in every control word the
/// specification defines; capitals are read too, because a file that writes one
/// is better read as the control word it meant than as text.
const LETTERS: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";

/// What a control word's parameter is made of: an optional minus and digits.
const NUMBER: &[u8] = b"-0123456789";

/// What a file opens with: the outer group, and the control word that names
/// the format. The version after it is a parameter, and is 1 in every file
/// anything has written.
pub(super) const MAGIC: &[u8] = b"{\\rtf";

const MARKER: &[(i128, &str)] = &[
    (b'{' as i128, "group"),
    (b'\\' as i128, "control"),
    (b'}' as i128, "end"),
];

pub fn rtf() -> Template {
    Template::new("rtf", T::Named("Item".into())).with_type("Item", item())
}

/// One thing in the file: a group, the brace that ends one, a control word or
/// symbol, or a run of text. Named by its body, and the body by whatever names
/// that: the word of a control word, the text of a text run.
fn item() -> T {
    T::structure_named("Item", "body", "body", vec![("marker", marker()), ("body", body())])
        .encoding_wrapper("body")
        .counted_as("item")
}

/// The byte this item opens with, where it is one of the three that mean
/// something. Text opens with anything else, and the field is then no bytes at
/// all: read as a number that is zero, which is the case [`body`] has no byte
/// for.
fn marker() -> T {
    let byte = T::enumeration("Rtf", T::u8(), MARKER);
    let cases = MARKER.iter().map(|(v, _)| (*v, byte.clone())).collect();
    T::switch(E::peek(8, Big), cases, T::bytes(E::lit(0)))
}

fn body() -> T {
    T::switch(
        E::field("marker"),
        vec![
            (b'{' as i128, group()),
            // The brace that closed a group says everything it has to say in
            // being that brace.
            (b'}' as i128, T::bytes(E::lit(0))),
            (b'\\' as i128, control()),
        ],
        text(),
    )
}

/// A group's contents, up to and including the item that is the brace
/// closing it. A file cut off before that brace stops at the end of what it
/// wrote, the way every run here does.
///
/// Named by its first item. `{\\fonttbl ...}` is the font table and
/// `{\\colortbl ...}` the colour table, and the word that says so is the
/// first thing in the group in every case the specification lists.
fn group() -> T {
    T::structure_named(
        "Group",
        "items.0",
        "items",
        vec![(
            "items",
            T::repeat(T::Named("Item".into()), Until::FieldBytes { field: "marker".into(), bytes: vec![b'}'] }),
        )],
    )
}

/// What follows the backslash: a control word when it is a letter, a byte
/// written as hex digits when it is `'`, and one character of control symbol
/// otherwise.
fn control() -> T {
    let mut cases: Vec<(i128, T)> = LETTERS.iter().map(|c| (*c as i128, word())).collect();
    cases.push((b'\'' as i128, hex_escape()));
    T::switch(E::peek(8, Big), cases, symbol())
}

/// A name, a number where there is one, the space that ends the name where
/// there is one, and for `\bin` the bytes that number counts.
///
/// All three are runs of a class of byte rather than fields of a length
/// anything wrote: `\b`, `\b0`, `\b0 ` and `\fs24\b` all have to read, and
/// what ends the name is whatever byte is not a letter. The space belongs to
/// the control word, so `\b x` is a bold run beginning with a space and `\b  x`
/// begins with two.
fn word() -> T {
    T::structure_named(
        "ControlWord",
        "name",
        "",
        vec![
            ("name", T::text(StrLen::Fixed(E::run(LETTERS)), Encoding::Ascii)),
            // No digits is no parameter, and a field of no bytes rather
            // than a number nothing spelt: `\b` and `\b0` mean opposite
            // things, and only the second wrote one.
            (
                "parameter",
                T::switch(E::run(NUMBER), vec![(0, T::bytes(E::lit(0)))], T::decimal(StrLen::Fixed(E::run(NUMBER)))),
            ),
            // One space, and only one: a run of them capped at a byte.
            ("delimiter", T::bytes(E::run(b" ").at_most(E::lit(1)))),
            // `\bin` alone has bytes after it, as many as its parameter says,
            // and they are anything at all: a brace in them opens nothing.
            // No more than are left, so a file cut off inside them reads as
            // far as it got, and a minus sign somebody wrote is no bytes.
            (
                "data",
                T::matches(
                    E::field("name"),
                    vec![("bin", T::bytes(E::field("parameter").at_least(E::lit(0)).at_most(E::Remaining)))],
                    T::bytes(E::lit(0)),
                ),
            ),
        ],
    )
}

/// One byte, written as two hex digits. What it means is the code page's
/// business, and the file says which page in a control word somewhere above.
fn hex_escape() -> T {
    T::structure_named(
        "HexByte",
        "byte",
        "",
        vec![("tick", T::magic(b"'")), ("byte", T::hex_digits(StrLen::Fixed(E::lit(2))))],
    )
}

/// A backslash and one character that is not a letter: the three markup
/// characters written literally, the non-breaking space and hyphens, the `*`
/// that marks a destination, and the newline that means a paragraph break.
fn symbol() -> T {
    T::structure_named(
        "ControlSymbol",
        "symbol",
        "",
        vec![("symbol", T::text(StrLen::Fixed(E::lit(1)), Encoding::Latin1))],
    )
}

/// A run of text, ending before the next byte the format has a meaning for.
///
/// Latin-1 rather than the code page the file named, which is a thing a
/// control word says and a thing no field here can reach. The two agree for
/// every byte below 0x80 and for most above it.
fn text() -> T {
    T::structure_named(
        "Text",
        "text",
        "text",
        vec![("text", T::text(StrLen::Fixed(E::run_except(MARKUP)), Encoding::Latin1))],
    )
    .encoding_wrapper("text")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    fn read(bytes: &[u8]) -> (Document<MemSource>, Evaluator) {
        (Document::new(MemSource(bytes.to_vec())), Evaluator::new(rtf()))
    }

    #[test]
    fn a_group_runs_to_the_item_that_is_its_closing_brace() {
        let (d, mut ev) = read(b"{\\rtf1 hi}");
        assert_eq!(
            ev.node(&d, &[0]).unwrap().value,
            Value::Enum { raw: b'{' as i128, name: Some("group".into()), hex: false }
        );
        // The control word, the text, and the brace that ended the group.
        assert_eq!(ev.node(&d, &[1, 0]).unwrap().child_count, 3);
        assert_eq!(ev.node(&d, &[]).unwrap().size_bits, 10 * 8);
        // Rows named by what they hold: the file by its first word, the word
        // by its name, the text by itself.
        assert_eq!(ev.node(&d, &[]).unwrap().name, "file rtf");
        assert_eq!(ev.node(&d, &[1, 0, 0]).unwrap().name, "[0] rtf");
        assert_eq!(ev.node(&d, &[1, 0, 1]).unwrap().name, "[1] hi");
        assert_eq!(ev.node(&d, &[1, 0, 2]).unwrap().name, "[2]");
    }

    #[test]
    fn a_control_word_takes_its_number_and_the_space_after_it() {
        let (d, mut ev) = read(b"{\\rtf1 hi}");
        let word = &[1, 0, 0, 1];
        assert_eq!(ev.node(&d, &[1, 0, 0, 1, 0]).unwrap().value, Value::Str("rtf".into()));
        assert_eq!(ev.node(&d, &[1, 0, 0, 1, 1]).unwrap().value, Value::Int(1));
        // The space is the control word's, not the text's.
        assert_eq!(ev.node(&d, &[1, 0, 0, 1, 2]).unwrap().size_bits, 8);
        assert_eq!(ev.node(&d, word).unwrap().size_bits, 5 * 8);
        assert_eq!(ev.node(&d, &[1, 0, 1, 1, 0]).unwrap().value, Value::Str("hi".into()));
    }

    #[test]
    fn a_control_word_with_no_number_and_no_space_is_just_its_name() {
        let (d, mut ev) = read(b"{\\b\\i0x}");
        assert_eq!(ev.node(&d, &[1, 0, 0, 1, 0]).unwrap().value, Value::Str("b".into()));
        // No digits and no space: two fields of no bytes, so the word is the
        // backslash and the letter.
        assert_eq!(ev.node(&d, &[1, 0, 0, 1, 1]).unwrap().size_bits, 0);
        assert_eq!(ev.node(&d, &[1, 0, 0, 1, 2]).unwrap().size_bits, 0);
        assert_eq!(ev.node(&d, &[1, 0, 0]).unwrap().size_bits, 2 * 8);
        assert_eq!(ev.node(&d, &[1, 0, 1, 1, 0]).unwrap().value, Value::Str("i".into()));
        assert_eq!(ev.node(&d, &[1, 0, 1, 1, 1]).unwrap().value, Value::Int(0));
        assert_eq!(ev.node(&d, &[1, 0, 2, 1, 0]).unwrap().value, Value::Str("x".into()));
    }

    #[test]
    fn a_negative_parameter_keeps_its_minus() {
        let (d, mut ev) = read(b"{\\li-360 }");
        assert_eq!(ev.node(&d, &[1, 0, 0, 1, 1]).unwrap().value, Value::Int(-360));
    }

    #[test]
    fn a_control_symbol_is_the_backslash_and_one_character() {
        // A literal brace, a literal backslash, and the star that marks a
        // destination. None of the three opens a group or ends one.
        let (d, mut ev) = read(b"{\\{\\\\\\*x}");
        assert_eq!(ev.node(&d, &[1, 0, 0, 1, 0]).unwrap().value, Value::Str("{".into()));
        assert_eq!(ev.node(&d, &[1, 0, 1, 1, 0]).unwrap().value, Value::Str("\\".into()));
        assert_eq!(ev.node(&d, &[1, 0, 2, 1, 0]).unwrap().value, Value::Str("*".into()));
        assert_eq!(ev.node(&d, &[1, 0, 3, 1, 0]).unwrap().value, Value::Str("x".into()));
        // Four items and the closing brace.
        assert_eq!(ev.node(&d, &[1, 0]).unwrap().child_count, 5);
    }

    #[test]
    fn a_hex_escape_reads_as_the_byte_it_spells() {
        let (d, mut ev) = read(b"{\\'e9}");
        assert_eq!(ev.node(&d, &[1, 0, 0, 1, 1]).unwrap().value, Value::Int(0xe9));
        assert_eq!(ev.node(&d, &[1, 0, 0]).unwrap().size_bits, 4 * 8);
    }

    #[test]
    fn a_text_run_stops_before_the_next_brace_or_backslash() {
        let (d, mut ev) = read(b"{hello {x}\\b}");
        assert_eq!(ev.node(&d, &[1, 0, 0, 1, 0]).unwrap().value, Value::Str("hello ".into()));
        // The group after it, read as a group and not as more text.
        assert_eq!(
            ev.node(&d, &[1, 0, 1, 0]).unwrap().value,
            Value::Enum { raw: b'{' as i128, name: Some("group".into()), hex: false }
        );
        assert_eq!(ev.node(&d, &[1, 0, 1, 1, 0, 0, 1, 0]).unwrap().value, Value::Str("x".into()));
    }

    #[test]
    fn a_group_nested_in_a_group_ends_at_its_own_brace() {
        let (d, mut ev) = read(b"{{\\fonttbl{\\f0 Times;}}\\b hi}");
        // The font table, the bold control word, the text, and the brace.
        assert_eq!(ev.node(&d, &[1, 0]).unwrap().child_count, 4);
        assert_eq!(ev.node(&d, &[1, 0, 1, 1, 0]).unwrap().value, Value::Str("b".into()));
        assert_eq!(ev.node(&d, &[1, 0, 2, 1, 0]).unwrap().value, Value::Str("hi".into()));
        assert_eq!(ev.node(&d, &[]).unwrap().size_bits, 29 * 8);
    }

    #[test]
    fn a_run_longer_than_the_walk_reads_in_one_go_is_still_one_run() {
        // The walk that measures a run reads the file a block at a time, and
        // a run of 100,000 bytes crosses two dozen of those seams. A picture
        // pasted into a document is written as hex digits and is exactly this
        // long, so it is the case a real file runs into rather than a corner.
        let mut v = b"{\\pict ".to_vec();
        v.extend(std::iter::repeat(b'a').take(100_000));
        v.push(b'}');
        let (d, mut ev) = read(&v);
        // The size is the run, whole. What the field reads *as* is shortened
        // for a row to hold, so the length of that says nothing.
        assert_eq!(ev.node(&d, &[1, 0, 1, 1, 0]).unwrap().size_bits, 100_000 * 8);
        assert_eq!(ev.node(&d, &[]).unwrap().size_bits, v.len() as u64 * 8);
    }

    #[test]
    fn the_bytes_after_bin_are_its_own_and_a_brace_in_them_opens_nothing() {
        let (d, mut ev) = read(b"{\\bin5 a{}b}\\b}");
        // The `\bin` and its five bytes, the `\b`, and the closing brace:
        // the braces among the five did not open a group or close this one.
        assert_eq!(ev.node(&d, &[1, 0]).unwrap().child_count, 3);
        assert_eq!(ev.node(&d, &[1, 0, 0, 1, 1]).unwrap().value, Value::Int(5));
        assert_eq!(ev.node(&d, &[1, 0, 0, 1, 3]).unwrap().size_bits, 5 * 8);
        assert_eq!(ev.node(&d, &[1, 0, 0]).unwrap().name, "[0] bin");
        assert_eq!(ev.node(&d, &[1, 0, 1]).unwrap().name, "[1] b");
        assert_eq!(ev.node(&d, &[]).unwrap().size_bits, 15 * 8);
    }

    #[test]
    fn bin_counts_no_more_than_it_says_and_no_more_than_there_is() {
        // None at all: the `x` is text, as it would be after any other word.
        let (d, mut ev) = read(b"{\\bin0 x}");
        assert_eq!(ev.node(&d, &[1, 0, 0, 1, 3]).unwrap().size_bits, 0);
        assert_eq!(ev.node(&d, &[1, 0, 1]).unwrap().name, "[1] x");
        // No parameter is no bytes, and so is a negative one.
        let (d, mut ev) = read(b"{\\bin x}");
        assert_eq!(ev.node(&d, &[1, 0, 0, 1, 3]).unwrap().size_bits, 0);
        let (d, mut ev) = read(b"{\\bin-3 x}");
        assert_eq!(ev.node(&d, &[1, 0, 0, 1, 3]).unwrap().size_bits, 0);
        // A file that ends inside them reads what it wrote.
        let (d, mut ev) = read(b"{\\bin9 ab");
        assert_eq!(ev.node(&d, &[1, 0, 0, 1, 3]).unwrap().size_bits, 2 * 8);
        assert_eq!(ev.node(&d, &[]).unwrap().size_bits, 9 * 8);
        // And no other word has any, whatever its parameter.
        let (d, mut ev) = read(b"{\\fs5 abcde}");
        assert_eq!(ev.node(&d, &[1, 0, 0, 1, 3]).unwrap().size_bits, 0);
        assert_eq!(ev.node(&d, &[1, 0, 1]).unwrap().name, "[1] abcde");
    }

    #[test]
    fn a_file_cut_off_before_its_closing_brace_reads_as_far_as_it_got() {
        let (d, mut ev) = read(b"{\\rtf1 hi");
        // Two items, and no brace to close them.
        assert_eq!(ev.node(&d, &[1, 0]).unwrap().child_count, 2);
        assert_eq!(ev.node(&d, &[]).unwrap().size_bits, 9 * 8);
    }

    #[test]
    fn a_rich_text_file_is_recognised_by_the_group_its_first_word_opens() {
        let head = b"{\\rtf1\\ansi\\deff0{\\fonttbl{\\f0 Times New Roman;}}}";
        assert_eq!(crate::formats::sniff(head, head.len() as u64), Some("rtf"));
        // The brace alone settles nothing: a JSON document opens with one too,
        // and is still read as JSON.
        assert_eq!(crate::formats::sniff(b"{\"a\": 1}", 8), Some("json"));
    }
}
