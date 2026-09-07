//! Cue sheets: the text file that says what the `.bin` beside it holds.
//!
//! A disc image carries no description of itself. The cue sheet is where the
//! tracks are named, where each one starts, and how many bytes a sector of it
//! takes, which is the one thing that tells a raw image from a cooked one
//! without counting sync patterns. See [`crate::formats::cdrom`].
//!
//! One command a line, indented as the writer felt like. The indentation is
//! not structure: `TRACK` belongs to the `FILE` above it because it came
//! after it, not because it is further in.

use crate::template::{Encoding, StrLen, Template, Ty as T, Until};

pub fn cue() -> Template {
    Template::new("cue", T::structure("CueSheet", vec![("lines", T::repeat(line(), Until::End))]))
}

/// One line, kept whole.
///
/// A command and its arguments are not split into fields here. The arguments
/// differ per command, a quoted filename may hold spaces, and the leading
/// indentation would land in a field of its own; a line of text with its
/// offset and its length is the more useful answer and the honest one.
fn line() -> T {
    T::structure_named(
        "CueLine",
        "text",
        "text",
        vec![("text", T::text(StrLen::Terminated { end: b'\n', or_end: true }, Encoding::Utf8))],
    )
    .counted_as("line")
}

/// The longest a cue sheet plausibly is. A disc holds 99 tracks, and a line
/// for each with a title, a performer and two indexes is a few kilobytes.
const LONGEST: u64 = 1 << 20;

/// Whether this text is a cue sheet.
///
/// Two commands have to be there. `FILE` names the image and `TRACK` says what
/// is in it, and a cue sheet without both describes nothing. Asking for both
/// is what keeps this off the many text files that mention one of the words:
/// `TRACK` alone is a playlist and a log, and `FILE` alone is half the
/// configuration files ever written.
pub fn is_cue(head: &[u8], len: u64) -> bool {
    if len == 0 || len > LONGEST {
        return false;
    }
    let text = &head[..head.len().min(4096)];
    if !text.iter().all(|b| matches!(b, b'\t' | b'\r' | b'\n' | 0x20..=0x7e)) {
        return false;
    }
    let mut file = false;
    let mut track = false;
    for line in text.split(|b| *b == b'\n') {
        let t = line.iter().position(|b| !b.is_ascii_whitespace()).map_or(&line[..0], |i| &line[i..]);
        file |= t.starts_with(b"FILE ");
        track |= t.starts_with(b"TRACK ");
    }
    file && track
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::Evaluator;
    use crate::source::MemSource;
    use crate::Value;

    const SHEET: &[u8] = b"REM GENRE Electronic\r\nFILE \"disc one.bin\" BINARY\r\n  TRACK 01 MODE2/2352\r\n    INDEX 01 00:00:00\r\n";

    #[test]
    fn a_cue_sheet_is_read_a_line_at_a_time() {
        assert!(is_cue(SHEET, SHEET.len() as u64));
        let d = Document::new(MemSource(SHEET.to_vec()));
        let mut e = Evaluator::new(cue());
        assert_eq!(e.node(&d, &[0]).unwrap().child_count, 4);
        assert_eq!(e.node(&d, &[0, 1, 0]).unwrap().value, Value::Str("FILE \"disc one.bin\" BINARY\r".into()));
    }

    #[test]
    fn text_that_is_not_a_cue_sheet_is_turned_away() {
        // Both commands are needed. Either alone is any text file at all.
        assert!(!is_cue(b"FILE \"x\" BINARY\n", 16));
        assert!(!is_cue(b"  TRACK 01 AUDIO\n", 17));
        // A word in the middle of a line is not a command.
        let prose = b"The FILE was on TRACK 3 of the tape.\n";
        assert!(!is_cue(prose, prose.len() as u64));
        // Text is text: a disc image holds both words somewhere and is not
        // this. Anything with a byte no keyboard makes is out.
        assert!(!is_cue(b"FILE \"x\" BINARY\nTRACK 01 AUDIO\n\x00\x01", 34));
        assert!(!is_cue(b"", 0));
    }
}
