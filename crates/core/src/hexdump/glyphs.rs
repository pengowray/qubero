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

use crate::text::{cp437_screen_char, CodePage, Settled};

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

    /// The name the reader picks a column's characters by, out of every page
    /// the app offers plus the two rules that are not a page.
    pub fn by_name(name: &str) -> Option<Glyphs> {
        if name == Glyphs::Screen.name() {
            return Some(Glyphs::Screen);
        }
        if name == Settled::Ascii.name() {
            return Some(Glyphs::Printable(Settled::Ascii));
        }
        CodePage::by_name(name).map(|p| Glyphs::Printable(Settled::SingleByte(p)))
    }

    /// Every byte's character in one string, 256 long, U+FFFD standing where
    /// this column writes its stand-in instead.
    ///
    /// What the hex view's text column is drawn from: one crossing of the
    /// wasm boundary per choice of column, rather than one per byte on screen.
    /// No page defines U+FFFD itself, which is what leaves it free to mean
    /// "nothing here".
    pub fn column(self) -> String {
        (0..=u8::MAX).map(|b| self.of(b).unwrap_or('\u{fffd}')).collect()
    }

    /// The character this column would have written for `b`, or nothing where
    /// it would have written its stand-in instead.
    ///
    /// A single-byte page answers from its own table: the page's undefined
    /// bytes are U+FFFD already, and a control character is a byte a column
    /// writing to a pipe has nothing to draw for, which together is the whole
    /// rule. The encodings that are not a single-byte page get the printable
    /// ASCII ninety-five, since that is all a column one character wide can
    /// say about them.
    pub fn of(self, b: u8) -> Option<char> {
        match self {
            Glyphs::Printable(Settled::SingleByte(page)) => {
                let c = page.char_of(b);
                (c != '\u{fffd}' && !c.is_control()).then_some(c)
            }
            Glyphs::Printable(_) => (0x20..=0x7e).contains(&b).then(|| b as char),
            // The same two bytes the screen page leaves undefined, 0x00 and
            // 0x7f, are the two tools disagree about: a screen drew a blank
            // and a house, and what comes through a clipboard is as often the
            // control character itself. @see crate::text::CP437_SCREEN_LOW
            Glyphs::Screen => Some(cp437_screen_char(b)).filter(|c| *c != '\u{fffd}'),
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
    fn a_page_above_latin_1_reaches_the_column() {
        let win = Glyphs::by_name("Windows-1252").expect("Windows-1252 is on offer");
        // 0x80 is the euro sign in this page, and undefined in Latin-1's.
        assert_eq!(win.of(0x80), Some('\u{20ac}'));
        assert_eq!(Glyphs::Printable(Settled::Latin1).of(0x80), None);
        // 0x81 is one of the page's five holes, so the column says nothing.
        assert_eq!(win.of(0x81), None);
    }

    #[test]
    fn a_column_is_two_hundred_and_fifty_six_characters() {
        for g in [Glyphs::Screen, Glyphs::Printable(Settled::Cp437), Glyphs::Printable(Settled::Ascii)] {
            let col: Vec<char> = g.column().chars().collect();
            assert_eq!(col.len(), 256, "{}", g.name());
            for (b, c) in col.iter().enumerate() {
                let want = g.of(b as u8).unwrap_or('\u{fffd}');
                assert_eq!(*c, want, "{} at {b:#04x}", g.name());
            }
        }
    }

    #[test]
    fn every_name_the_chooser_offers_is_one_the_core_knows() {
        assert_eq!(Glyphs::by_name("ASCII"), Some(Glyphs::Printable(Settled::Ascii)));
        assert_eq!(Glyphs::by_name("CP437"), Some(Glyphs::Printable(Settled::Cp437)));
        assert_eq!(Glyphs::by_name(Glyphs::Screen.name()), Some(Glyphs::Screen));
        assert_eq!(Glyphs::by_name("UTF-8"), None);
        assert_eq!(Glyphs::by_name(""), None);
    }

    #[test]
    fn the_high_half_is_the_encoding_either_way() {
        assert_eq!(Glyphs::Screen.of(0x80), Glyphs::Printable(Settled::Cp437).of(0x80));
        assert_eq!(Glyphs::Printable(Settled::Latin1).of(0xff), Some('\u{00ff}'));
    }
}
