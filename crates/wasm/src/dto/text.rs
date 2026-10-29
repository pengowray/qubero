//! Search steps, text encodings, the text view, strings and hex dumps.

use crate::*;

/// What one step of a search found, as the host reads it. `status` is the same
/// tri-state everything else here answers with, so the caller's chunk-fetching
/// loop is the one it already has.
#[derive(Serialize)]
#[serde(tag = "step")]
pub(crate) enum StepDto {
    #[serde(rename = "found")]
    Found { at: f64, len: f64 },
    #[serde(rename = "more")]
    More { resume: f64 },
    #[serde(rename = "end")]
    End,
}

#[derive(Serialize)]
pub(crate) struct SelectionTextDto {
    pub(crate) readings: Vec<ReadingDto>,
    /// Encodings the bytes do not fit, named rather than shown.
    pub(crate) refused: Vec<String>,
    /// Bytes actually read, which is short of the selection when it is long.
    pub(crate) read: f64,
    pub(crate) all: bool,
}

#[derive(Serialize)]
pub(crate) struct ReadingDto {
    /// The encodings that agree on this reading, the likeliest first.
    pub(crate) encodings: Vec<String>,
    pub(crate) text: String,
}

#[derive(Serialize)]
pub(crate) struct TextEncodeDto {
    pub(crate) bytes: Vec<u8>,
    /// The character the encoding has no room for, or empty.
    pub(crate) refused: String,
}

#[derive(Serialize)]
pub(crate) struct TextReadingDto {
    pub(crate) encoding: String,
    pub(crate) mark: u32,
    pub(crate) guessed: bool,
    pub(crate) unit: u32,
}

#[derive(Serialize)]
pub(crate) struct TextWindowDto {
    pub(crate) lines: Vec<TextLineDto>,
    pub(crate) missing: Vec<f64>,
    pub(crate) next: f64,
}

#[derive(Serialize)]
pub(crate) struct TextLineDto {
    pub(crate) at: f64,
    pub(crate) len: f64,
    pub(crate) ending: String,
    pub(crate) text: String,
    /// Escape sequences as flat pairs of character index and length.
    pub(crate) escapes: Vec<u32>,
    pub(crate) lossy: bool,
}

#[derive(Serialize)]
pub(crate) struct StringsScanDto {
    pub(crate) hits: Vec<StringHitDto>,
    pub(crate) missing: Vec<f64>,
    pub(crate) next: f64,
    /// True when the template ran out of time before it could say anything
    /// about the first string, so nothing came back and the caller asks again.
    pub(crate) busy: bool,
}

/// The run a string lies inside, where the template reads it as numbers or
/// instructions, or as a packed stream.
#[derive(Serialize, Clone)]
pub(crate) struct StringInsideDto {
    /// `numbers`, `code` or `packed`.
    pub(crate) kind: String,
    /// The run, or the stream: what opens a packed one in a tab of its own.
    pub(crate) path: Vec<usize>,
    /// The run's name, as the listing gives it.
    pub(crate) name: String,
    /// What one element is, such as `i16 le` or `x86-64`, or the codec.
    pub(crate) what: String,
    pub(crate) at: f64,
    pub(crate) len: f64,
}

#[derive(Serialize)]
pub(crate) struct StringHitDto {
    pub(crate) at: f64,
    pub(crate) len: f64,
    pub(crate) enc: String,
    pub(crate) chars: u32,
    pub(crate) units: u32,
    pub(crate) text: String,
    pub(crate) lone_surrogates: bool,
    /// Bytes of zero after the text: none, one, or two after a wide string.
    pub(crate) terminator: u32,
    pub(crate) cut: bool,
    pub(crate) prefix: Vec<StringPrefixDto>,
    pub(crate) inside: Option<StringInsideDto>,
}

#[derive(Serialize)]
pub(crate) struct StringPrefixDto {
    pub(crate) kind: String,
    pub(crate) at: f64,
    pub(crate) bytes: Vec<u8>,
    pub(crate) value: f64,
    pub(crate) counts: String,
    pub(crate) with_terminator: bool,
    pub(crate) weak: bool,
}

#[derive(Serialize)]
pub(crate) struct TextBackDto {
    pub(crate) start: f64,
    pub(crate) back: f64,
    pub(crate) missing: Vec<f64>,
}

/// What a text file turned out to be a dump of, if anything.
///
/// Standalone rather than a method on [`Editor`], because a dump is text the
/// host already has in hand and the file it describes is not the file that is
/// open. The host reads this, offers what it says, and opens the recovered
/// bytes as a document of their own.
#[derive(Serialize, Default)]
pub(crate) struct DumpScanDto {
    /// Empty when the layout matches no tool that is recognised here, which
    /// changes nothing about how it was read.
    pub(crate) tool: String,
    /// "regular" when the lines were regular enough to be read by arithmetic.
    pub(crate) tier: String,
    /// The first address described and the end of the last.
    pub(crate) from: f64,
    pub(crate) to: f64,
    /// Bytes the dump actually spells out, which is fewer than `to - from`
    /// when it skips stretches.
    pub(crate) covered: f64,
    pub(crate) address_base: String,
    pub(crate) address_digits: u32,
    pub(crate) bytes_per_line: u32,
    pub(crate) group: u32,
    pub(crate) upper: bool,
    pub(crate) reversed_groups: bool,
    pub(crate) characters: String,
    /// What the dump did not settle, which was taken as the usual thing.
    pub(crate) assumed: Vec<String>,
    /// Stretches of the described file the dump covers, as start/end pairs.
    pub(crate) extents: Vec<f64>,
    /// Stretches inside that span nobody described, as start/end pairs.
    pub(crate) holes: Vec<f64>,
    /// Paths or file names the dump gave.
    pub(crate) names: Vec<String>,
    /// A length the dump stated, or -1. Not the same as what it went on to
    /// write, which is the point of keeping it.
    pub(crate) stated_length: f64,
    /// Command lines a transcript kept.
    pub(crate) commands: Vec<String>,
    /// Lines that were not part of the dump.
    pub(crate) skipped_lines: u32,
    /// Bytes whose two spellings disagree, capped: a dump read the wrong way
    /// disagrees everywhere, and the first few say so as well as all of them.
    pub(crate) conflicts: Vec<DumpConflictDto>,
}

#[derive(Serialize)]
pub(crate) struct DumpConflictDto {
    pub(crate) at: f64,
    pub(crate) wrote: String,
    pub(crate) digits: u8,
}
