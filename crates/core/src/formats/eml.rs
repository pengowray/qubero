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
//! The body is read as MIME says it is (RFC 2045 and 2046), from the header
//! fields that describe it:
//!
//! - `Content-Type:` is read as its media type, `multipart/mixed`, and its
//!   parameters, `boundary="b1_9f3c"`, each a field of its own.
//! - A `multipart` body is its parts, each its own small header and body
//!   between two lines that start with `--` and the boundary. A part may be
//!   `multipart` itself: an email with an attachment and an HTML copy of its
//!   text is `multipart/mixed` holding a `multipart/alternative`. What comes
//!   before the first boundary line and after the last is kept as the
//!   preamble and the epilogue. A message that stops before its closing
//!   boundary line has the parts it wrote, the last running to the end.
//! - A body or a part whose `Content-Transfer-Encoding:` is `base64` or
//!   `quoted-printable` is decoded, and what it holds is read from the bytes
//!   that come out, which open as a document of their own.
//! - `text` is read in the `charset` the type names where Qubero has that
//!   encoding: `utf-8`, `us-ascii`, `iso-8859-1` and `ibm437`. Any other, and a
//!   body with no `Content-Type:`, is read as UTF-8 if it is valid UTF-8 and as
//!   Latin-1 otherwise. A `message/rfc822` part is read as the message it is.
//!   Anything else is bytes.
//!
//! Names are compared without regard to case, as the RFCs say they are, so
//! `Content-type:` and `BOUNDARY=` are read as the field and the parameter
//! they name.
//!
//! A header value with an encoded word in it (RFC 2047), such as
//! `=?UTF-8?Q?caf=C3=A9_night?=`, is read as the runs of plain text and the
//! words, and each word's text is decoded and read in the charset it names.
//!
//! What this does not read yet:
//!
//! - A header value with encoded words in it as one line of text. Each word
//!   is a field of its own, and its decoded text is inside it, so a subject
//!   written in two words reads as two pieces of text, with the blank between
//!   them that RFC 2047 says a reader drops kept as a third.
//! - Parameters split into pieces or given a charset (`filename*0=`,
//!   `filename*=utf-8''...`, RFC 2231). Each piece reads as a parameter of
//!   its own.
//! - A backslash escaping a quote inside a quoted parameter value. The value
//!   stops at the first quote.
//! - Comments in parentheses in a structured value. What comes after the
//!   parameters is kept as text.
//! - Charsets Qubero has no encoding for, such as `windows-1252` and
//!   `iso-8859-15`, which read as the fallback above says.

use crate::codec::Codec;
use crate::template::{Encoding, Endian::Big, Expr as E, StrLen, Template, Ty as T, Until};

pub fn eml() -> Template {
    Template::new("eml", T::Named("Message".into()))
        .with_type("Message", entity("Message"))
        .with_type("HeaderLine", line())
        .with_type("Body", body())
        .with_type("Part", part())
        .with_type("Parameter", parameter())
        .with_type("Payload", payload())
        .with_type("Text", text_in(E::lower(parameter_value("charset"))))
        .with_type("EncodedWord", encoded_word())
        .with_type("WordText", text_in(E::lower(E::field("charset"))))
}

/// A message, or a message inside one: a header, the blank line, and the
/// body the header describes.
fn entity(name: &str) -> T {
    T::structure(
        name,
        vec![
            ("header", header()),
            // The blank line, which is absent only when the file ends
            // inside the header.
            ("separator", T::when(E::Remaining.greater_than(E::lit(0)), T::text(StrLen::Fixed(blank_line()), Encoding::Ascii))),
            ("body", T::Named("Body".into())),
        ],
    )
    .machinery(&["separator"])
    .payload(&["body"])
}

fn header() -> T {
    T::repeat(T::Named("HeaderLine".into()), Until::While(more_header()))
}

fn byte(c: u8) -> E {
    E::lit(i128::from(c))
}

/// Whether the bytes ahead are another line of the header rather than the
/// blank line that ends it: anything but `\n` or `\r\n` at the start.
fn more_header() -> E {
    let first = E::peek(8, Big);
    let lf = first.clone().equal_to(byte(b'\n'));
    let crlf = first.equal_to(byte(b'\r')).both(E::Remaining.greater_than(E::lit(1))).both(E::peek_at(E::lit(8), 8, Big).equal_to(byte(b'\n')));
    E::Remaining.greater_than(E::lit(0)).both(lf.either(crlf).negate())
}

/// How long the blank line after the header is: `\r\n` or `\n`.
fn blank_line() -> E {
    E::cond(E::peek(8, Big).equal_to(byte(b'\r')), E::lit(2), E::lit(1)).at_most(E::Remaining)
}

/// One header field, with its continuation lines, or a line that is not one.
///
/// Bounded first, so that a field's value is read up to the end of its own
/// lines and no further. The field ends at the first `\n` that is not followed
/// by a space or a tab, and the `\n` is part of it.
fn line() -> T {
    let length = E::to_marker(b'\n', b" \t").add(E::lit(1)).at_most(E::Remaining);
    let name = E::run_except(NOT_NAME);
    let is_field = name.clone().greater_than(E::lit(0)).both(name.clone().less_than(E::Remaining)).both(E::peek_at(name.mul(E::lit(8)), 8, Big).equal_to(byte(b':')));
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
            ("value", T::sized(E::Remaining.sub(line_end()), value())),
            ("end", T::text(StrLen::Fixed(E::Remaining), Encoding::Ascii)),
        ],
    )
    .machinery(&["colon", "space", "end"])
    .payload(&["value"])
    .counted_as("header field")
}

/// A field's value: text, except for the two MIME fields written as a word
/// and parameters, which are read as those, and a value with an encoded word
/// in it, which is read as its pieces.
///
/// UTF-8 because RFC 6532 allows it in a header, and RFC 5322 allows only
/// ASCII. A Latin-1 byte in a header is a mistake, and the value is marked as
/// not valid UTF-8.
fn value() -> T {
    let text = T::text(StrLen::Fixed(E::Remaining), Encoding::Utf8);
    let has_words = E::to_bytes(b"=?").less_than(E::Remaining);
    T::matches(
        E::lower(E::field("name")),
        vec![("content-type", media_type()), ("content-disposition", disposition())],
        T::switch(has_words, vec![(1, encoded_text())], text),
    )
}

/// `Re: =?UTF-8?Q?caf=C3=A9?= night`: the runs of text between encoded
/// words, and the words.
fn encoded_text() -> T {
    let word_here = E::Remaining.greater_than(E::lit(1)).both(E::to_bytes(b"=?").equal_to(E::lit(0)));
    let plain = T::text(StrLen::Fixed(E::to_bytes(b"=?")), Encoding::Utf8);
    T::repeat(T::switch(word_here, vec![(1, T::Named("EncodedWord".into()))], plain), Until::End)
}

/// `=?iso-8859-1?Q?caf=E9?=`: a charset, `B` for base64 or `Q` for the Q
/// encoding, and the text written in it, which is decoded and read in that
/// charset. RFC 2047 says neither encoding's text holds a `?`, so the text
/// runs to the `?=` that closes the word.
fn encoded_word() -> T {
    let to_close = || E::to_bytes(b"?=");
    let read = |codec| T::decoded(to_close(), codec, T::Named("WordText".into()));
    T::structure_named(
        "EncodedWord",
        "",
        "text",
        vec![
            ("open", T::magic(b"=?")),
            ("charset", T::text(StrLen::Fixed(E::run_except(b"?")), Encoding::Ascii)),
            ("charset_end", T::magic(b"?")),
            ("encoding", T::text(StrLen::Fixed(E::run_except(b"?")), Encoding::Ascii)),
            ("encoding_end", T::magic(b"?")),
            (
                "text",
                T::matches(
                    E::lower(E::field("encoding")),
                    vec![("b", read(Codec::Base64)), ("q", read(Codec::QEncoding))],
                    T::text(StrLen::Fixed(to_close()), Encoding::Ascii),
                ),
            ),
            ("close", T::magic(b"?=")),
        ],
    )
    .machinery(&["open", "charset_end", "encoding_end", "close"])
    .payload(&["text"])
    .counted_as("encoded word")
}

/// `multipart/alternative; boundary="b1_9f3c"`: a type, a subtype, and the
/// parameters after them.
///
/// Read by what ends each piece rather than by what may be in it, so a value
/// written wrong still reads as far as it goes: a type with no `/` is a type
/// with no subtype, and whatever follows the last parameter is kept as text.
fn media_type() -> T {
    let slash = E::Remaining.greater_than(E::lit(0)).both(E::peek(8, Big).equal_to(byte(b'/')));
    T::structure(
        "MediaType",
        vec![
            ("type", T::text(StrLen::Fixed(E::run_except(b"/;( \t\r\n")), Encoding::Ascii)),
            ("slash", T::when(slash, T::magic(b"/"))),
            ("subtype", T::text(StrLen::Fixed(E::run_except(b";( \t\r\n")), Encoding::Ascii)),
            ("parameters", T::repeat(T::Named("Parameter".into()), Until::While(another_parameter()))),
            ("rest", T::text(StrLen::Fixed(E::Remaining), Encoding::Utf8)),
        ],
    )
    .machinery(&["slash"])
}

/// `attachment; filename="counts.csv"`: whether a part is shown in the message
/// or kept as a file (RFC 2183), and the parameters after it, read the way a
/// media type's are.
fn disposition() -> T {
    T::structure(
        "Disposition",
        vec![
            ("type", T::text(StrLen::Fixed(E::run_except(b";( \t\r\n")), Encoding::Ascii)),
            ("parameters", T::repeat(T::Named("Parameter".into()), Until::While(another_parameter()))),
            ("rest", T::text(StrLen::Fixed(E::Remaining), Encoding::Utf8)),
        ],
    )
}

/// Blanks and line breaks, which a folded value puts between its parameters.
const FOLD: &[u8] = b" \t\r\n";

/// Whether a parameter comes next: a `;` after whatever blanks and line
/// breaks there are.
fn another_parameter() -> E {
    let gap = E::run(FOLD);
    E::Remaining.greater_than(gap.clone()).both(E::peek_at(gap.mul(E::lit(8)), 8, Big).equal_to(byte(b';')))
}

/// `; charset="iso-8859-1"`: a name and a value, which may be quoted.
///
/// Named by its name and read as its value, the way a header field is. The
/// quotes are fields of their own, so the value is what is inside them, which
/// is the text a boundary line is written with. Each quote is one byte or
/// none, rather than a field that is there or not, so that whether the value
/// is quoted can be asked of it.
fn parameter() -> T {
    let more = || E::Remaining.greater_than(E::lit(0));
    let at = |c: u8| more().both(E::peek(8, Big).equal_to(byte(c)));
    let quoted = || E::size_of("open").greater_than(E::lit(0));
    let quote = |present: E| T::text(StrLen::Fixed(E::cond(present, E::lit(1), E::lit(0))), Encoding::Ascii);
    T::structure_named(
        "Parameter",
        "name",
        "value",
        vec![
            // The `;`, and the line break and indent of a folded line around it.
            ("separator", T::text(StrLen::Fixed(E::run(b" \t\r\n;")), Encoding::Ascii)),
            ("name", T::text(StrLen::Fixed(E::run_except(b"=; \t\r\n")), Encoding::Ascii)),
            ("equals", T::when(at(b'='), T::magic(b"="))),
            ("open", quote(at(b'"'))),
            ("value", T::text(StrLen::Fixed(E::cond(quoted(), E::run_except(b"\""), E::run_except(b"; \t\r\n"))), Encoding::Utf8)),
            ("close", quote(quoted().both(at(b'"')))),
        ],
    )
    .machinery(&["separator", "equals", "open", "close"])
    .payload(&["value"])
    .counted_as("parameter")
}

/// A header field of the nearest header above, found by its name in any case.
///
/// The nearest header is the one this body belongs to. A part's body is read
/// inside the part, where its own header is the nearest; the list of parts is
/// read outside every part, where the nearest is the header of the message
/// the parts are in. That is what lets a part be `multipart` with a boundary
/// of its own while the parts around it end at the boundary of the message.
fn header_field(name: &str, field: &[&str]) -> E {
    E::tagged_text_any_case(E::field("header"), &["name"], name, field)
}

/// The value of one parameter of the nearest `Content-Type:`, by its name in
/// any case. Nothing when there is no such field or no such parameter.
fn parameter_value(name: &str) -> E {
    E::tagged_text_any_case(header_field("content-type", &["value", "parameters"]), &["name"], name, &["value"])
}

/// The nearest `Content-Type:`'s type or subtype, in lower case.
fn media(piece: &str) -> E {
    E::lower(header_field("content-type", &["value", piece]))
}

/// What a body holds, as its header says.
fn body() -> T {
    T::matches(media("type"), vec![("multipart", multipart())], content())
}

/// A body that is not parts: decoded first if its transfer encoding says it
/// was encoded, and then read as its type.
fn content() -> T {
    let payload = || T::Named("Payload".into());
    let decoded = |codec| T::decoded(E::Remaining, codec, payload());
    T::matches(
        E::lower(header_field("content-transfer-encoding", &["value"])),
        vec![("base64", decoded(Codec::Base64)), ("quoted-printable", decoded(Codec::QuotedPrintable))],
        payload(),
    )
}

/// The bytes of a body once any transfer encoding is off them.
///
/// Text when the type says it is, and when there is no type at all, which RFC
/// 2045 says means `text/plain`.
fn payload() -> T {
    let message = T::matches(media("subtype"), vec![("rfc822", T::Named("Message".into())), ("global", T::Named("Message".into()))], T::bytes(E::Remaining));
    let text = || T::Named("Text".into());
    T::matches(media("type"), vec![("text", text()), ("", text()), ("message", message)], T::bytes(E::Remaining))
}

/// Text in the charset `charset` names, in lower case, where there is an
/// encoding for it: the `charset` parameter of `Content-Type:` for a body,
/// and the charset an encoded word names for its text.
fn text_in(charset: E) -> T {
    let read = |enc| T::text(StrLen::Fixed(E::Remaining), enc);
    T::matches(
        charset,
        vec![
            ("utf-8", read(Encoding::Utf8)),
            ("utf8", read(Encoding::Utf8)),
            ("us-ascii", read(Encoding::Ascii)),
            ("ascii", read(Encoding::Ascii)),
            ("iso-8859-1", read(Encoding::Latin1)),
            ("iso8859-1", read(Encoding::Latin1)),
            ("latin1", read(Encoding::Latin1)),
            ("ibm437", read(Encoding::Cp437)),
            ("cp437", read(Encoding::Cp437)),
        ],
        read(Encoding::Unknown),
    )
}

/// The text a boundary line is made of, from the nearest `Content-Type:`.
fn boundary() -> E {
    parameter_value("boundary")
}

/// How far it is to the `\n` that ends the line before the next boundary
/// line, or to the end when there is none. RFC 2046 says a boundary line
/// starts a line, so the search is for a line break and then the line.
fn to_boundary_line() -> E {
    E::to_text(b"\n--", boundary(), b"")
}

/// Whether a boundary line starts here.
fn boundary_here() -> E {
    E::Remaining.greater_than(E::lit(0)).both(E::to_text(b"--", boundary(), b"").equal_to(E::lit(0)))
}

/// How far it is to the last boundary line, which has `--` after the
/// boundary as well as before it, or to the end when there is none.
fn to_closing() -> E {
    E::to_text(b"--", boundary(), b"--")
}

/// A `multipart` body: what comes before the first boundary line, the parts,
/// the closing boundary line, and what comes after it.
///
/// The parts are found inside the stretch that ends where the closing line
/// starts, so the closing line is looked for once rather than once a part.
/// A part runs from its boundary line to the line break before the next one,
/// and that break is the part's last field. RFC 2046 says the break belongs to
/// the boundary line after it, which is why it is not in the part's body.
fn multipart() -> T {
    let to_next = || to_boundary_line().add(E::lit(1)).at_most(E::Remaining);
    T::structure(
        "Multipart",
        vec![
            ("preamble", T::when(boundary_here().negate(), T::sized(to_next(), lines("Preamble")))),
            ("parts", T::sized(to_closing(), T::repeat(T::sized(to_next(), T::Named("Part".into())), Until::End))),
            ("closing", T::when(E::Remaining.greater_than(E::lit(0)), T::sized(line_length(), lines("BoundaryLine")))),
            ("epilogue", T::text(StrLen::Fixed(E::Remaining), Encoding::Unknown)),
        ],
    )
    .machinery(&["closing"])
}

/// One part of a `multipart` body: the boundary line that starts it, its own
/// header and body, and the line break before the next boundary line.
fn part() -> T {
    T::structure(
        "Part",
        vec![
            ("boundary", T::sized(line_length(), lines("BoundaryLine"))),
            ("header", header()),
            ("separator", T::when(E::Remaining.greater_than(E::lit(0)), T::text(StrLen::Fixed(blank_line()), Encoding::Ascii))),
            ("body", T::sized(E::Remaining.sub(line_end()), T::Named("Body".into()))),
            ("end", T::text(StrLen::Fixed(E::Remaining), Encoding::Ascii)),
        ],
    )
    .machinery(&["boundary", "separator", "end"])
    .payload(&["body"])
    .counted_as("part")
}

/// How long the line starting here is, its line break included.
fn line_length() -> E {
    E::to_bytes(b"\n").add(E::lit(1)).at_most(E::Remaining)
}

/// Lines of text read as one, and the line break at the end of them.
fn lines(name: &str) -> T {
    T::structure_named(
        name,
        "",
        "text",
        vec![
            ("text", T::text(StrLen::Fixed(E::Remaining.sub(line_end())), Encoding::Unknown)),
            ("end", T::text(StrLen::Fixed(E::Remaining), Encoding::Ascii)),
        ],
    )
    .machinery(&["end"])
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
    let lf = has(1).both(back(1).equal_to(byte(b'\n')));
    let cr_lf = has(2).both(back(2).equal_to(byte(b'\r')));
    let cr = has(1).both(back(1).equal_to(byte(b'\r')));
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

    fn text_at(e: &mut Evaluator, d: &Document<MemSource>, path: &[usize]) -> String {
        match node_value(e, d, path) {
            Value::Str(s) => s,
            other => panic!("{path:?} is {other:?}, not text"),
        }
    }

    // A message, then a part of it, then its body: the paths the tests below
    // walk. The body of a multipart is its preamble, its parts, the closing
    // boundary line and its epilogue, and a part is its boundary line, its
    // header, the blank line, its body and the line break after it.
    const BODY: usize = 2;
    const PARTS: usize = 1;
    const CLOSING: usize = 2;
    const PART_BODY: usize = 3;

    const NESTED: &[u8] = b"From: a@example.org\r\n\
Content-Type: multipart/mixed; boundary=outer\r\n\
\r\n\
--outer\r\n\
Content-Type: multipart/alternative; boundary=\"inner\"\r\n\
\r\n\
--inner\r\n\
Content-Type: text/plain\r\n\
\r\n\
plain\r\n\
--inner\r\n\
Content-Type: text/html\r\n\
\r\n\
<b>html</b>\r\n\
--inner--\r\n\
\r\n\
--outer\r\n\
Content-Type: text/plain; charset=us-ascii\r\n\
\r\n\
second\r\n\
--outer--\r\n";

    #[test]
    fn a_part_that_is_multipart_ends_at_its_own_boundary() {
        let d = Document::new(MemSource(NESTED.to_vec()));
        let mut e = Evaluator::new(eml());
        assert_eq!(e.node(&d, &[BODY, PARTS]).unwrap().child_count, 2);
        // The first part holds two of its own, between lines of its own
        // boundary, and the outer boundary does not end either of them.
        assert_eq!(e.node(&d, &[BODY, PARTS, 0, PART_BODY, PARTS]).unwrap().child_count, 2);
        assert_eq!(text_at(&mut e, &d, &[BODY, PARTS, 0, PART_BODY, PARTS, 0, PART_BODY]), "plain");
        assert_eq!(text_at(&mut e, &d, &[BODY, PARTS, 0, PART_BODY, PARTS, 1, PART_BODY]), "<b>html</b>");
        assert_eq!(text_at(&mut e, &d, &[BODY, PARTS, 0, PART_BODY, CLOSING, 0]), "--inner--");
        // The line break before a boundary line is the part's, not its body's.
        assert_eq!(text_at(&mut e, &d, &[BODY, PARTS, 1, PART_BODY]), "second");
        assert_eq!(text_at(&mut e, &d, &[BODY, PARTS, 1, 4]), "\r\n");
        assert_eq!(text_at(&mut e, &d, &[BODY, PARTS, 0, 0, 0]), "--outer");
        assert_eq!(text_at(&mut e, &d, &[BODY, CLOSING, 0]), "--outer--");
        assert_eq!(text_at(&mut e, &d, &[BODY, 3]), "");
    }

    #[test]
    fn a_message_cut_off_before_its_closing_boundary_keeps_its_parts() {
        let text = b"Content-Type: multipart/mixed; boundary=b\n\nbefore\n--b\n\nfirst\n--b\n\nsecond, cut off";
        let d = Document::new(MemSource(text.to_vec()));
        let mut e = Evaluator::new(eml());
        assert_eq!(text_at(&mut e, &d, &[BODY, 0, 0]), "before");
        assert_eq!(e.node(&d, &[BODY, PARTS]).unwrap().child_count, 2);
        assert_eq!(text_at(&mut e, &d, &[BODY, PARTS, 0, PART_BODY]), "first");
        // The last part runs to the end, and there is no closing line.
        assert_eq!(text_at(&mut e, &d, &[BODY, PARTS, 1, PART_BODY]), "second, cut off");
        assert_eq!(e.node(&d, &[BODY, CLOSING]).unwrap().size_bits, 0);
    }

    #[test]
    fn names_in_any_case_and_a_boundary_on_a_folded_line() {
        // `Content-type`, `BOUNDARY`, a type in capitals, the parameter on the
        // line after the type, and line breaks of `\n` alone.
        let text = b"Content-type: MULTIPART/Alternative;\n\tBOUNDARY=\"=_x y\"\n\n--=_x y\nContent-Type: text/plain; Charset=ISO-8859-1\nContent-Transfer-Encoding: Quoted-Printable \n\ncaf=E9\n--=_x y--\n";
        let d = Document::new(MemSource(text.to_vec()));
        let mut e = Evaluator::new(eml());
        assert_eq!(text_at(&mut e, &d, &[0, 0, 3, 3, 0, 1]), "BOUNDARY");
        assert_eq!(text_at(&mut e, &d, &[0, 0, 3, 3, 0, 4]), "=_x y");
        assert_eq!(e.node(&d, &[BODY, PARTS]).unwrap().child_count, 1);
        // Decoded, and read in the charset the part names.
        assert_eq!(text_at(&mut e, &d, &[BODY, PARTS, 0, PART_BODY, 0]), "caf\u{e9}");
        assert_eq!(text_at(&mut e, &d, &[BODY, CLOSING, 0]), "--=_x y--");
    }

    #[test]
    fn a_body_with_no_parts_is_decoded_as_its_header_says() {
        // base64 on the whole message, and a type with no parameters.
        let text = b"Content-Type: text/plain\r\nContent-Transfer-Encoding: base64\r\n\r\naGVs\r\nbG8=\r\n";
        let d = Document::new(MemSource(text.to_vec()));
        let mut e = Evaluator::new(eml());
        assert_eq!(e.node(&d, &[0, 0, 3, 3]).unwrap().child_count, 0);
        assert_eq!(text_at(&mut e, &d, &[BODY, 0]), "hello");
        // A type that is not text is the bytes it decodes to.
        let text = b"Content-Type: application/octet-stream\nContent-Transfer-Encoding: base64\n\nAAEC\n";
        let d = Document::new(MemSource(text.to_vec()));
        let mut e = Evaluator::new(eml());
        assert!(matches!(node_value(&mut e, &d, &[BODY, 0]), Value::Bytes { .. }));
        assert_eq!(e.node(&d, &[BODY, 0]).unwrap().size_bits, 24);
    }

    #[test]
    fn a_multipart_with_no_boundary_is_all_preamble() {
        let text = b"Content-Type: multipart/mixed\n\n--x\n\nhi\n--x--\n";
        let d = Document::new(MemSource(text.to_vec()));
        let mut e = Evaluator::new(eml());
        assert_eq!(e.node(&d, &[BODY, PARTS]).unwrap().child_count, 0);
        assert_eq!(text_at(&mut e, &d, &[BODY, 0, 0]), "--x\n\nhi\n--x--");
    }

    #[test]
    fn a_forwarded_message_is_read_as_a_message() {
        let text = b"Content-Type: multipart/mixed; boundary=b\n\n--b\nContent-Type: message/rfc822\n\nSubject: inside\n\nforwarded\n--b--\n";
        let d = Document::new(MemSource(text.to_vec()));
        let mut e = Evaluator::new(eml());
        assert_eq!(text_at(&mut e, &d, &[BODY, PARTS, 0, PART_BODY, 0, 0, 3]), "inside");
        assert_eq!(text_at(&mut e, &d, &[BODY, PARTS, 0, PART_BODY, BODY]), "forwarded");
    }

    #[test]
    fn an_encoded_word_is_decoded_and_read_in_its_charset() {
        let text = b"Subject: Re: =?UTF-8?Q?caf=C3=A9_night?= =?iso-8859-1?B?bm8=?=\r\nFrom: a@example.org\r\n\r\nhi\r\n";
        let d = Document::new(MemSource(text.to_vec()));
        let mut e = Evaluator::new(eml());
        let value = [0, 0, 3];
        assert_eq!(e.node(&d, &value).unwrap().child_count, 4);
        assert_eq!(text_at(&mut e, &d, &[0, 0, 3, 0]), "Re: ");
        assert_eq!(text_at(&mut e, &d, &[0, 0, 3, 1, 1]), "UTF-8");
        assert_eq!(text_at(&mut e, &d, &[0, 0, 3, 1, 5, 0]), "caf\u{e9} night");
        assert_eq!(text_at(&mut e, &d, &[0, 0, 3, 2]), " ");
        assert_eq!(text_at(&mut e, &d, &[0, 0, 3, 3, 5, 0]), "no");
        // A value with none in it is the text it is.
        assert_eq!(text_at(&mut e, &d, &[0, 1, 3]), "a@example.org");
    }

    #[test]
    fn a_binary_body_does_not_stop_a_message_being_recognised() {
        let mut text = POST.to_vec();
        text.extend_from_slice(b"\x00\x01\x02");
        assert!(is_message(&text, text.len() as u64));
    }
}
