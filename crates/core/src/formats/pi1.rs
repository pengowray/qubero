//! Degas PI1: an Atari ST screen, saved as the machine held it.
//!
//! There is no header worth the name. Two bytes of resolution, sixteen
//! hardware palette words, and then 32000 bytes that are the screen memory
//! itself: four bit planes interleaved a word at a time, which is how the
//! Shifter read them out. The file is always 32034 bytes.
//!
//! A palette word is three bits per channel in the low nibbles, so the whole
//! machine had 512 colours. The bits above them are what the STE later used to
//! reach 4096, by putting the new low bit of each channel in bit 3.

use crate::template::{Endian::*, Expr as E, Template, Ty as T};

/// The three screen modes, which fix the size as well as the colour count.
const RESOLUTION: &[(i128, &str)] = &[
    (0, "low, 320x200, 16 colours"),
    (1, "medium, 640x200, 4 colours"),
    (2, "high, 640x400, mono"),
];

pub fn pi1() -> Template {
    Template::new(
        "pi1",
        T::structure(
            "Degas",
            vec![
                ("resolution", T::enumeration("Resolution", T::u16(Big), RESOLUTION)),
                ("palette", T::array(colour(), E::lit(16)).counted_as("colour")),
                ("screen", T::bytes(E::lit(32000))),
            ],
        ),
    )
}

/// One hardware palette register: three bits per channel, and on an STE a
/// fourth bit above each of them.
fn colour() -> T {
    T::inline_structure(
        "Colour",
        vec![
            ("unused", T::UInt { bits: 4, endian: Big }),
            ("red", T::UInt { bits: 4, endian: Big }),
            ("green", T::UInt { bits: 4, endian: Big }),
            ("blue", T::UInt { bits: 4, endian: Big }),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    #[test]
    fn a_low_resolution_screen_reads_its_palette_and_its_32000_bytes() {
        let mut v = 0u16.to_be_bytes().to_vec();
        v.extend_from_slice(&0x0777u16.to_be_bytes()); // white
        for _ in 1..16 {
            v.extend_from_slice(&0u16.to_be_bytes());
        }
        v.extend_from_slice(&[0; 32000]);
        assert_eq!(v.len(), 32034);

        let d = Document::new(MemSource(v));
        let mut ev = Evaluator::new(pi1());
        assert_eq!(
            ev.node(&d, &[0]).unwrap().value,
            Value::Enum { raw: 0, name: Some("low, 320x200, 16 colours".into()), hex: false }
        );
        assert_eq!(ev.node(&d, &[1, 0, 1]).unwrap().value, Value::UInt(7));
        assert_eq!(ev.node(&d, &[2]).unwrap().size_bits, 32000 * 8);
    }
}
