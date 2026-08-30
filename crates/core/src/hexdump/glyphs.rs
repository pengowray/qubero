//! What character a dump wrote for a byte.
//!
//! Most tools print the byte where it is printable and a full stop everywhere
//! else, which is the same rule the hex view uses. A tool drawing on a DOS
//! screen does not have to: the hardware has a glyph for all 256 values, so
//! `xtree`'s hex view with its mask off shows a smiling face for 0x01 and a
//! musical note for 0x0D, and the character column then says something about
//! every byte rather than about the printable ninety-five.
//!
//! That is a second rule rather than a second encoding, which is why it is a
//! type of its own: CP437 the encoding says nothing about 0x01, and CP437 the
//! screen says it is U+263A. A dump captured off such a screen may then arrive
//! either as the bytes it was drawn in or as the Unicode something translated
//! them to, and both have to read the same.

use crate::text::{cp437_char, Settled};

/// The glyphs the low half of CP437 has on a screen, which is where a control
/// character has a picture instead of an effect. 0x00 is left out on purpose:
/// a screen has nothing to draw for it, and a tool that writes something there
/// is writing a stand-in, which is worked out from the dump rather than
/// assumed.
const CP437_LOW: [char; 31] = [
    '\u{263a}', '\u{263b}', '\u{2665}', '\u{2666}', '\u{2663}', '\u{2660}', '\u{2022}', '\u{25d8}',
    '\u{25cb}', '\u{25d9}', '\u{2642}', '\u{2640}', '\u{266a}', '\u{266b}', '\u{263c}', '\u{25ba}',
    '\u{25c4}', '\u{2195}', '\u{203c}', '\u{00b6}', '\u{00a7}', '\u{25ac}', '\u{21a8}', '\u{2191}',
    '\u{2193}', '\u{2192}', '\u{2190}', '\u{221f}', '\u{2194}', '\u{25b2}', '\u{25bc}',
];

/// How a dump turned a byte into a character.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Glyphs {
    /// The byte where the encoding has a character for it, a stand-in
    /// elsewhere. Every tool that writes to a pipe does this.
    Printable(Settled),
    /// Every byte but zero, as a DOS screen draws it.
    Screen,
}

impl Glyphs {
    /// Every way a character column might have been written, in the order they
    /// are tried. Plain ASCII first, so a column holding nothing but printable
    /// text is read as the simplest thing that explains it.
    pub const EVERY: [Glyphs; 4] = [
        Glyphs::Printable(Settled::Ascii),
        Glyphs::Printable(Settled::Latin1),
        Glyphs::Printable(Settled::Cp437),
        Glyphs::Screen,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Glyphs::Printable(s) => s.name(),
            Glyphs::Screen => "CP437 as a screen draws it",
        }
    }

    /// The character this column would have written for `b`, or nothing where
    /// it would have written its stand-in instead.
    pub fn of(self, b: u8) -> Option<char> {
        match self {
            Glyphs::Printable(Settled::Latin1) => ((0x20..=0x7e).contains(&b) || b >= 0xa0).then(|| b as char),
            Glyphs::Printable(Settled::Cp437) => (b >= 0x20 && b != 0x7f).then(|| cp437_char(b)),
            Glyphs::Printable(_) => (0x20..=0x7e).contains(&b).then(|| b as char),
            // 0x7f is left out with 0x00: the glyph for it is a house, but
            // what comes through a clipboard is as often the control character
            // itself, and a byte two tools disagree about is one to say
            // nothing about rather than one to call a conflict.
            Glyphs::Screen => match b {
                0 | 0x7f => None,
                b if b < 0x20 => Some(CP437_LOW[b as usize - 1]),
                b => Some(cp437_char(b)),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_screen_has_a_picture_where_a_pipe_has_a_full_stop() {
        assert_eq!(Glyphs::Printable(Settled::Cp437).of(0x01), None);
        assert_eq!(Glyphs::Screen.of(0x01), Some('\u{263a}'));
        assert_eq!(Glyphs::Screen.of(0x0d), Some('\u{266a}'));
        assert_eq!(Glyphs::Screen.of(0x1f), Some('\u{25bc}'));
    }

    #[test]
    fn the_two_bytes_tools_disagree_about_are_left_open() {
        assert_eq!(Glyphs::Screen.of(0), None);
        assert_eq!(Glyphs::Screen.of(0x7f), None);
    }

    #[test]
    fn the_high_half_is_the_encoding_either_way() {
        assert_eq!(Glyphs::Screen.of(0x80), Glyphs::Printable(Settled::Cp437).of(0x80));
        assert_eq!(Glyphs::Printable(Settled::Latin1).of(0xff), Some('\u{00ff}'));
    }
}
