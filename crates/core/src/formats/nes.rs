//! iNES: the header an emulator author invented in 1996 so that a cartridge
//! dumped to a file would say what was on the board.
//!
//! Sixteen bytes, then the program ROM and the character ROM one after the
//! other, in the sizes the header gives in banks. The mapper number, which
//! says what extra hardware the cartridge carried, is split across two flag
//! bytes: the low nibble sits in the top of flags6 and the high nibble in the
//! top of flags7. A computed field puts it back together, because a reader
//! wants the number and not the two halves.
//!
//! An optional 512-byte trainer sits between the header and the program when
//! bit 2 of flags6 is set. Its size is that bit times 512, which is what the
//! expression language can say without an `if`.

use crate::template::{Expr as E, Template, Ty as T};

/// The bits of flags6 that are not the mapper nibble.
const FLAGS6: &[(u32, &str)] = &[
    (0, "vertical mirroring"),
    (1, "battery-backed ram"),
    (2, "trainer"),
    (3, "four-screen vram"),
];

/// The bits of flags7. The two in the middle mark a file written to the NES
/// 2.0 revision of this header, which adds fields the rest of it wastes.
const FLAGS7: &[(u32, &str)] = &[(0, "vs unisystem"), (1, "playchoice-10"), (2, "nes 2.0"), (3, "nes 2.0")];

pub fn nes() -> Template {
    // The nibble of the mapper number in each flag byte, put back together.
    let mapper = E::field("flags7").div(E::lit(16)).mul(E::lit(16)).add(E::field("flags6").div(E::lit(16)));
    // Bit 2 of flags6, on its own: n/4 less twice n/8.
    let trainer = E::field("flags6").div(E::lit(4)).sub(E::field("flags6").div(E::lit(8)).mul(E::lit(2)));

    Template::new(
        "nes",
        T::structure(
            "iNES",
            vec![
                ("magic", T::magic(b"NES\x1a")),
                ("prg_banks", T::u8()),
                ("chr_banks", T::u8()),
                ("flags6", T::flags("Flags6", T::u8(), FLAGS6)),
                ("flags7", T::flags("Flags7", T::u8(), FLAGS7)),
                ("prg_ram_banks", T::u8()),
                ("flags9", T::u8()),
                ("flags10", T::u8()),
                ("padding", T::bytes(E::lit(5))),
                ("mapper", T::computed(mapper)),
                ("trainer", T::bytes(trainer.mul(E::lit(512)))),
                // Banks are 16K of program and 8K of characters. A file with
                // no character ROM has the graphics in RAM instead.
                ("prg_rom", T::bytes(E::field("prg_banks").mul(E::lit(16 * 1024)))),
                ("chr_rom", T::bytes(E::field("chr_banks").mul(E::lit(8 * 1024)))),
            ],
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    fn rom(flags6: u8, flags7: u8, trainer: bool) -> Vec<u8> {
        let mut v = b"NES\x1a".to_vec();
        v.extend_from_slice(&[2, 1, flags6, flags7, 0, 0, 0]);
        v.extend_from_slice(&[0; 5]);
        if trainer {
            v.extend_from_slice(&[0xaa; 512]);
        }
        v.extend_from_slice(&[0x11; 32 * 1024]);
        v.extend_from_slice(&[0x22; 8 * 1024]);
        v
    }

    #[test]
    fn the_mapper_number_is_put_back_together_from_two_nibbles() {
        // Mapper 4, MMC3: the low nibble in the top of flags6.
        let d = Document::new(MemSource(rom(0x40, 0x00, false)));
        let mut ev = Evaluator::new(nes());
        assert_eq!(ev.node(&d, &[9]).unwrap().value, Value::Int(4));
        // Mapper 66 wants a nibble from each byte.
        let d = Document::new(MemSource(rom(0x20, 0x40, false)));
        let mut ev = Evaluator::new(nes());
        assert_eq!(ev.node(&d, &[9]).unwrap().value, Value::Int(66));
    }

    #[test]
    fn a_trainer_exists_only_when_its_bit_is_set() {
        let d = Document::new(MemSource(rom(0, 0, false)));
        let mut ev = Evaluator::new(nes());
        assert_eq!(ev.node(&d, &[10]).unwrap().size_bits, 0);
        assert_eq!(ev.node(&d, &[11]).unwrap().offset_bits, 16 * 8);
        assert_eq!(ev.node(&d, &[11]).unwrap().size_bits, 32 * 1024 * 8);
        assert_eq!(ev.node(&d, &[12]).unwrap().size_bits, 8 * 1024 * 8);

        let d = Document::new(MemSource(rom(0b100, 0, true)));
        let mut ev = Evaluator::new(nes());
        assert_eq!(ev.node(&d, &[10]).unwrap().size_bits, 512 * 8);
        assert_eq!(ev.node(&d, &[11]).unwrap().offset_bits, (16 + 512) * 8);
    }
}
