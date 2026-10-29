//! Internet messages: one email or one newsgroup post, saved as a file.
//!
//! Outlook Express, Windows Mail and Windows Live Mail save a newsgroup post
//! as `.nws` (`._nws` when it was dragged out of the program) and an email as
//! `.eml`. Both are the message as the server sent it: RFC 5322 for mail, and
//! RFC 5536 for news, which is the same layout with a few more header fields
//! (`Newsgroups:`, `Path:`, `Xref:`). Header fields one to a line, a blank
//! line, and the body.
//!
//! A header field is a name, a colon and a value. A value continues onto the
//! next line when that line begins with a space or a tab, which is how a long
//! `References:` list or a `Content-Type:` with parameters is written. The
//! line break and the indent are kept in the value, as the file has them;
//! RFC 5322 calls taking them out "unfolding", and a reader comparing the
//! value with the bytes needs them in.
//!
//! What this does not read yet:
//!
//! - MIME parts. A message with an attachment or an HTML copy of its text is
//!   `multipart`, and its parts are separated by a boundary line whose text
//!   is a parameter of `Content-Type`. Finding the parts needs a search for
//!   bytes read out of the file, and `Expr::Find` only searches for bytes the
//!   template fixes.
//! - base64 and quoted-printable bodies, which need codecs of their own.
//! - Encoded words in header values (`=?UTF-8?B?...?=`, RFC 2047). They read
//!   as the ASCII they are written in.
//!
//! The body is one run of text, read as UTF-8 if it is valid UTF-8 and as
//! Latin-1 otherwise. The charset `Content-Type` names is not consulted.

use crate::template::{Encoding, Endian::Big, Expr as E, StrLen, Template, Ty as T, Until};

pub fn eml() -> Template {
    Template::new(
        "eml",
        T::structure(
            "Message",
            vec![
                ("header", T::repeat(T::Named("HeaderLine".into()), Until::While(more_header()))),
                // The blank line, which is absent only when the file ends
                // inside the header.
                ("separator", T::when(E::Remaining.greater_than(E::lit(0)), T::text(StrLen::Fixed(blank_line()), Encoding::Ascii))),
                ("body", T::text(StrLen::Fixed(E::Remaining), Encoding::Unknown)),
            ],
        )
        .machinery(&["separator"])
        .payload(&["body"]),
    )
    .with_type("HeaderLine", line())
}

fn byte(c: u8) -> E {
    E::lit(i128::from(c))
}

/// Whether the bytes ahead are another line of the header rather than the
/// blank line that ends it: anything but `\n` or `\r\n` at the start.
fn more_header() -> E {
    let first = E::peek(8, Big);
    let lf = first.clone().equals(byte(b'\n'));
    let crlf = first.equals(byte(b'\r')).both(E::Remaining.greater_than(E::lit(1))).both(E::peek_at(E::lit(8), 8, Big).equals(byte(b'\n')));
    E::Remaining.greater_than(E::lit(0)).both(lf.either(crlf).negate())
}

/// How long the blank line after the header is: `\r\n` or `\n`.
fn blank_line() -> E {
    E::cond(E::peek(8, Big).equals(byte(b'\r')), E::lit(2), E::lit(1)).at_most(E::Remaining)
}

/// One header field, with its continuation lines, or a line that is not one.
///
/// Bounded first, so that a field's value is read up to the end of its own
/// lines and no further. The field ends at the first `\n` that is not followed
/// by a space or a tab, and the `\n` is part of it.
fn line() -> T {
    let length = E::to_marker(b'\n', b" \t").add(E::lit(1)).at_most(E::Remaining);
    let name = E::run_except(NOT_NAME);
    let is_field = name.clone().greater_than(E::lit(0)).both(name.clone().less_than(E::Remaining)).both(E::peek_at(name.mul(E::lit(8)), 8, Big).equals(byte(b':')));
    T::sized(length, T::switch(is_field, vec![(1, field())], other_line()))
}

/// The bytes a field name stops at. RFC 5322 allows any printable ASCII but
/// the colon in a name, so a space or a line break before the colon means the
/// line is not a header field: an mbox `From ` line, or a header that has been
/// damaged.
const NOT_NAME: &[u8] = b": \t\r\n";

/// `Subject: Re: rec.arts.sf.written FAQ`, and the lines it continues onto.
///
/// Named by its name and read as its value, so each row of the header in a
/// listing shows a field's name and value, as a mail program shows them.
fn field() -> T {
    T::structure_named(
        "HeaderField",
        "name",
        "value",
        vec![
            ("name", T::text(StrLen::Fixed(E::run_except(NOT_NAME)), Encoding::Ascii)),
            ("colon", T::magic(b":")),
            // The space after the colon is how every program writes a field,
            // and RFC 5322 counts it as part of the value. Kept out of the
            // value here so the value reads as what a mail program shows.
            ("space", T::text(StrLen::Fixed(E::run(b" \t")), Encoding::Ascii)),
            // UTF-8 because RFC 6532 allows it in a header, and RFC 5322
            // allows only ASCII. A Latin-1 byte in a header is a mistake, and
            // the value is marked as not valid UTF-8.
            ("value", T::text(StrLen::Fixed(E::Remaining.sub(line_end())), Encoding::Utf8)),
            ("end", T::text(StrLen::Fixed(E::Remaining), Encoding::Ascii)),
        ],
    )
    .machinery(&["colon", "space", "end"])
    .payload(&["value"])
    .counted_as("header field")
}

/// A line in the header that is not a field, read as the text it is.
fn other_line() -> T {
    T::structure_named(
        "Line",
        "",
        "text",
        vec![
            ("text", T::text(StrLen::Fixed(E::Remaining.sub(line_end())), Encoding::Unknown)),
            ("end", T::text(StrLen::Fixed(E::Remaining), Encoding::Ascii)),
        ],
    )
    .machinery(&["end"])
    .counted_as("line")
}

/// How many bytes at the end of the line are the line break: `\r\n`, `\n`, a
/// lone `\r` at the end of a file cut off after it, or nothing at the end of
/// a file with no line break at all. Read back from the end of the bounded
/// line, so it can be asked from anywhere inside it.
fn line_end() -> E {
    let back = |n: i128| E::peek_at(E::lit(-8 * n), 8, Big);
    let has = |n: i128| E::Remaining.greater_or_equal(E::lit(n));
    let lf = has(1).both(back(1).equals(byte(b'\n')));
    let cr_lf = has(2).both(back(2).equals(byte(b'\r')));
    let cr = has(1).both(back(1).equals(byte(b'\r')));
    E::cond(lf, E::cond(cr_lf, E::lit(2), E::lit(1)), E::cond(cr, E::lit(1), E::lit(0)))
}

/// Header field names that only a message opens with. A file of `Name: value`
/// lines is also a Debian control file, a Python package's metadata, an HTTP
/// response's header and a hundred configuration files, so the probe asks for
/// one of these before it calls a file a message. Compared without regard to
/// case, as RFC 5322 says names are.
const MESSAGE_FIELDS: &[&[u8]] = &[
    b"From",
    b"Newsgroups",
    b"Message-ID",
    b"Path",
    b"Xref",
    b"Received",
    b"Return-Path",
    b"Delivered-To",
    b"Relay-Version",
];

/// How many header fields a file must open with before the bytes alone are
/// enough to call it a message. One `From:` line is any text that starts with
/// the word.
const FIELDS_WITHOUT_A_NAME: usize = 3;

/// Whether this file opens with a message header: header fields from its first
/// byte, each a name and a colon or a continuation line, up to a blank line or
/// the end of what was read. Bytes below 0x20 other than a tab and the line
/// breaks mean the header is not text. The body is not looked at, since a
/// body may hold anything.
///
/// `cut` says the window stops before the file does, so a last line with no
/// colon may be a field the window cut in half rather than a damaged one.
/// `envelope` lets the first line be an mbox `From ` line, which some programs
/// keep when they save one message out of a mailbox. Thunderbird's mailboxes
/// separate messages with `From - ` and a date.
///
/// Returns how many fields were read, and whether one of them is a field only a
/// message has.
fn header_fields(head: &[u8], cut: bool, envelope: bool) -> Option<(usize, bool)> {
    let blank = blank_line_at(head);
    let header = &head[..blank.unwrap_or(head.len())];
    if header.iter().any(|b| (*b < 0x20 && !matches!(b, b'\t' | b'\r' | b'\n')) || *b == 0x7f) {
        return None;
    }
    let lines: Vec<&[u8]> = header.split(|b| *b == b'\n').collect();
    let mut fields = 0;
    let mut message = false;
    for (i, raw) in lines.iter().enumerate() {
        let line = raw.strip_suffix(b"\r").unwrap_or(raw);
        // The piece after the last `\n`: empty before a blank line or at the
        // end of a file that is nothing but a header.
        let last = i + 1 == lines.len();
        if line.is_empty() && last {
            continue;
        }
        if line.first().is_some_and(|b| matches!(b, b' ' | b'\t')) && i > 0 {
            continue;
        }
        if i == 0 && envelope && line.starts_with(b"From ") {
            continue;
        }
        let name_len = line.iter().position(|b| NOT_NAME.contains(b)).unwrap_or(line.len());
        if name_len == 0 || line.get(name_len) != Some(&b':') {
            if last && cut && blank.is_none() {
                break;
            }
            return None;
        }
        fields += 1;
        message |= MESSAGE_FIELDS.iter().any(|m| m.eq_ignore_ascii_case(&line[..name_len]));
    }
    (fields > 0).then_some((fields, message))
}

/// Where the blank line that ends the header starts, if the bytes hold one.
fn blank_line_at(head: &[u8]) -> Option<usize> {
    let lf = head.windows(2).position(|w| w == b"\n\n").map(|i| i + 1);
    let crlf = head.windows(4).position(|w| w == b"\r\n\r\n").map(|i| i + 2);
    match (lf, crlf) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

/// Whether a file called `.eml` or `.nws` is the message its name says: it
/// opens with a header field, or an mbox `From ` line and then a header field,
/// and every line up to the blank line is one.
pub fn is_named_message(head: &[u8], len: u64) -> bool {
    len > 0 && header_fields(head, (head.len() as u64) < len, true).is_some()
}

/// Whether a file nothing names is a message: its header has at least
/// [`FIELDS_WITHOUT_A_NAME`] fields and one of them is in [`MESSAGE_FIELDS`].
/// What finds a mail server's spool files and a maildir's messages, which have
/// no extension. A file opening with an mbox `From ` line is left alone here:
/// without a name saying it is one message, it is a mailbox of many.
pub fn is_message(head: &[u8], len: u64) -> bool {
    len > 0 && matches!(header_fields(head, (head.len() as u64) < len, false), Some((n, true)) if n >= FIELDS_WITHOUT_A_NAME)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::Evaluator;
    use crate::source::MemSource;
    use crate::Value;

    // `\x20` rather than a space: a `\` line continuation drops the spaces
    // at the front of the next line, and the folded line needs its indent.
    const POST: &[u8] = b"Path: news.example.net!not-for-mail\r\n\
From: \"A. Writer\" <writer@example.org>\r\n\
Newsgroups: comp.lang.rust\r\n\
Subject: Folded\r\n\
\x20subject line\r\n\
Message-ID: <1@example.org>\r\n\
\r\n\
Body text.\r\nSecond line.\r\n";

    fn node_value(e: &mut Evaluator, d: &Document<MemSource>, path: &[usize]) -> Value {
        e.node(d, path).unwrap().value
    }

    #[test]
    fn a_post_reads_as_header_fields_a_blank_line_and_a_body() {
        let d = Document::new(MemSource(POST.to_vec()));
        let mut e = Evaluator::new(eml());
        assert_eq!(e.node(&d, &[0]).unwrap().child_count, 5);
        assert_eq!(node_value(&mut e, &d, &[0, 1, 0]), Value::Str("From".into()));
        assert_eq!(node_value(&mut e, &d, &[0, 1, 3]), Value::Str("\"A. Writer\" <writer@example.org>".into()));
        assert_eq!(node_value(&mut e, &d, &[0, 1, 4]), Value::Str("\r\n".into()));
        // The continuation line is part of the value, break and indent included.
        assert_eq!(node_value(&mut e, &d, &[0, 3, 3]), Value::Str("Folded\r\n subject line".into()));
        assert_eq!(node_value(&mut e, &d, &[1]), Value::Str("\r\n".into()));
        assert_eq!(node_value(&mut e, &d, &[2]), Value::Str("Body text.\r\nSecond line.\r\n".into()));
    }

    #[test]
    fn unix_line_breaks_and_a_header_with_no_body() {
        let text = b"From: a@example.org\nSubject: hi\n";
        let d = Document::new(MemSource(text.to_vec()));
        let mut e = Evaluator::new(eml());
        assert_eq!(e.node(&d, &[0]).unwrap().child_count, 2);
        assert_eq!(node_value(&mut e, &d, &[0, 1, 3]), Value::Str("hi".into()));
        assert_eq!(node_value(&mut e, &d, &[0, 1, 4]), Value::Str("\n".into()));
        assert_eq!(node_value(&mut e, &d, &[2]), Value::Str("".into()));
    }

    #[test]
    fn a_line_that_is_not_a_field_reads_as_a_line() {
        let text = b"From someone@example.org Sat Jan  3 01:05:34 1996\nFrom: someone@example.org\n\nhi\n";
        let d = Document::new(MemSource(text.to_vec()));
        let mut e = Evaluator::new(eml());
        assert_eq!(e.node(&d, &[0]).unwrap().child_count, 2);
        assert_eq!(node_value(&mut e, &d, &[0, 0, 0]), Value::Str("From someone@example.org Sat Jan  3 01:05:34 1996".into()));
        assert_eq!(node_value(&mut e, &d, &[0, 1, 3]), Value::Str("someone@example.org".into()));
        assert_eq!(node_value(&mut e, &d, &[2]), Value::Str("hi\n".into()));
    }

    #[test]
    fn a_post_is_recognised_by_its_header() {
        assert!(is_message(POST, POST.len() as u64));
        assert!(is_named_message(POST, POST.len() as u64));
    }

    #[test]
    fn text_shaped_like_a_header_is_not_a_message_without_a_message_field() {
        // A Debian control file has the same layout, and none of the names.
        let control = b"Package: hello\nVersion: 2.10-3\nArchitecture: amd64\nMaintainer: Someone <a@example.org>\n\nPackage: other\n";
        assert!(!is_message(control, control.len() as u64));
        // Named `.eml`, the name is the evidence, and the layout agrees.
        assert!(is_named_message(control, control.len() as u64));
        // One field is any text that begins with the word.
        let one = b"From: me\n\nhello\n";
        assert!(!is_message(one, one.len() as u64));
        // Prose is not a header, whatever it opens with.
        let prose = b"From: the start, this was a letter.\nIt went on for a while\n\n";
        assert!(!is_named_message(prose, prose.len() as u64));
        // Nor is a binary file that happens to open with a field.
        let binary = b"From: a\nPath: b\nXref: c\x00\x01\n\n";
        assert!(!is_message(binary, binary.len() as u64));
        assert!(!is_named_message(b"", 0));
    }

    #[test]
    fn a_message_exported_from_a_mailbox_keeps_its_from_line() {
        let text = b"From - Mon Oct 12 08:14:02 2026\r\nFrom: a@example.org\r\nNewsgroups: alt.test\r\nMessage-ID: <1@example.org>\r\n\r\nhi\r\n";
        // Called `.eml`, it is one message.
        assert!(is_named_message(text, text.len() as u64));
        // With no name, the same opening is a mailbox of many messages.
        assert!(!is_message(text, text.len() as u64));
    }

    #[test]
    fn a_binary_body_does_not_stop_a_message_being_recognised() {
        let mut text = POST.to_vec();
        text.extend_from_slice(b"\x00\x01\x02");
        assert!(is_message(&text, text.len() as u64));
    }
}
