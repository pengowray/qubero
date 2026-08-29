//! HackRF firmware images, which is what a PortaPack build is one of.
//!
//! The file is a flash image with no container around it: the first bytes are
//! what the LPC43xx's Cortex-M4 fetches at reset, and everything after them is
//! code. Two things make it readable anyway.
//!
//! The first is the vector table the architecture defines. Word zero is the
//! value the core loads into the stack pointer, and every word after it is the
//! address of a handler, with bit 0 set because the core runs Thumb code. The
//! fifteen exceptions after the stack pointer are the ones every Cortex-M has;
//! the fifty-three after those are the LPC43xx's own interrupts, named here as
//! libopencm3 names them. A build that has no handler for one leaves the word
//! zero, and six of the numbers are reserved by NXP and never used at all.
//!
//! The second is the record HackRF's own build puts at 0x400, which is what
//! `hackrf_info` reads to report the firmware version. It opens with
//! `HACKRFFW`, and that is what identifies the file: a flash image has no
//! magic number of its own.
//!
//! From the end of that record to the end of the file is Thumb code. Nothing
//! in the image says where the code stops and the data begins, so all of it is
//! read as instructions, the same reading a `.COM` program gets. A PortaPack
//! image is the application and the baseband image one after the other, padded
//! with 0xff and with a checksum word at the very end; none of those three
//! boundaries is written down anywhere in the file, so none of them is a field
//! here.

use crate::code::Isa;
use crate::template::{Endian::*, Expr as E, Template, Ty as T, Until};

/// Where every HackRF build puts the record that names the firmware.
const INFO_AT: i128 = 0x400;

/// The magic that opens it, and what says the file is one of these.
pub const MAGIC: &[u8] = b"HACKRFFW";
pub const MAGIC_AT: usize = INFO_AT as usize;

/// Which boards the firmware will run on. One bit each, as the build's own
/// `platform_detect.h` numbers them.
const PLATFORM: &[(u32, &str)] = &[
    (0, "jawbreaker"),
    (1, "hackrf one"),
    (2, "rad1o"),
    (3, "hackrf one r9"),
    (4, "praline"),
];

/// The exceptions every Cortex-M defines, in the order they sit in, after the
/// stack pointer that comes first. A reserved slot is a word the architecture
/// never uses; the build fills it with the handler it fills everything else
/// with.
const EXCEPTIONS: &[&str] = &[
    "reset",
    "nmi",
    "hard_fault",
    "mem_manage",
    "bus_fault",
    "usage_fault",
    "reserved_7",
    "reserved_8",
    "reserved_9",
    "reserved_10",
    "svcall",
    "debug_monitor",
    "reserved_13",
    "pendsv",
    "systick",
];

/// The LPC43xx's interrupts, in order. Six of the fifty-three are reserved by
/// NXP: they still take a word in the table, and are named for what they are.
const IRQS: &[&str] = &[
    "dac",
    "m0core",
    "dma",
    "reserved_irq_3",
    "reserved_irq_4",
    "ethernet",
    "sdio",
    "lcd",
    "usb0",
    "usb1",
    "sct",
    "ritimer",
    "timer0",
    "timer1",
    "timer2",
    "timer3",
    "mcpwm",
    "adc0",
    "i2c0",
    "i2c1",
    "spi",
    "adc1",
    "ssp0",
    "ssp1",
    "usart0",
    "uart1",
    "usart2",
    "usart3",
    "i2s0",
    "i2s1",
    "spifi",
    "sgpio",
    "pin_int0",
    "pin_int1",
    "pin_int2",
    "pin_int3",
    "pin_int4",
    "pin_int5",
    "pin_int6",
    "pin_int7",
    "gint0",
    "gint1",
    "eventrouter",
    "c_can1",
    "reserved_irq_44",
    "reserved_irq_45",
    "atimer",
    "rtc",
    "reserved_irq_48",
    "wwdt",
    "reserved_irq_50",
    "c_can0",
    "qei",
];

/// Bytes the whole table takes: the stack pointer, the exceptions, and one
/// word per interrupt.
const TABLE_BYTES: i128 = 4 * (1 + EXCEPTIONS.len() as i128 + IRQS.len() as i128);

pub fn hackrffw() -> Template {
    Template::new(
        "hackrffw",
        T::structure(
            "Firmware",
            vec![
                ("vectors", T::structure("Vectors", vectors())),
                // The build leaves the table's last interrupts and everything
                // up to the version record unwritten.
                ("padding", T::bytes(E::lit(INFO_AT - TABLE_BYTES))),
                ("info", info()),
                ("code", T::sized(E::Remaining, T::repeat(T::insn(Isa::Thumb), Until::End))),
            ],
        ),
    )
}

/// The vector table, one named word at a time.
fn vectors() -> Vec<(&'static str, T)> {
    // Not a handler: the core loads the first word into the stack pointer
    // before it runs anything, so it is an address in RAM rather than in the
    // image.
    std::iter::once("stack_top")
        .chain(EXCEPTIONS.iter().copied())
        .chain(IRQS.iter().copied())
        .map(|name| (name, T::u32(Little)))
        .collect()
}

/// What `hackrf_info` reads out of a firmware image: `struct firmware_info_t`
/// from the HackRF sources, packed, forty-eight bytes.
fn info() -> T {
    T::structure(
        "FirmwareInfo",
        vec![
            ("magic", T::magic(MAGIC)),
            ("struct_version", T::u16(Little)),
            // Set in the build that boots straight into DFU rather than into
            // the firmware.
            ("dfu_mode", T::u16(Little)),
            ("supported_platform", T::flags("Platform", T::u32(Little), PLATFORM)),
            ("version_string", T::utf8_padded(E::lit(32), 0)),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    /// A vector table, a version record where the build puts one, and a couple
    /// of instructions after it.
    fn image() -> Vec<u8> {
        let mut v = 0x2000_0400u32.to_le_bytes().to_vec();
        // Reset, then the same default handler in every other slot.
        v.extend_from_slice(&0x0000_7fb9u32.to_le_bytes());
        while v.len() < TABLE_BYTES as usize {
            v.extend_from_slice(&0x0000_8069u32.to_le_bytes());
        }
        v.resize(INFO_AT as usize, 0);
        v.extend_from_slice(MAGIC);
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&0u16.to_le_bytes());
        v.extend_from_slice(&0x0au32.to_le_bytes());
        let mut version = b"n_260825".to_vec();
        version.resize(32, 0);
        v.extend_from_slice(&version);
        // `push {r3-r7, lr}` and `movs r4, #0`, which is how the reset handler
        // in a real image opens.
        v.extend_from_slice(&[0xf8, 0xb5, 0x00, 0x24]);
        v
    }

    #[test]
    fn the_version_record_reads_where_the_build_puts_it() {
        let d = Document::new(MemSource(image()));
        let mut ev = Evaluator::new(hackrffw());
        let info = ev.node(&d, &[2]).unwrap();
        assert_eq!(info.offset_bits, INFO_AT as u64 * 8);
        assert_eq!(info.size_bits, 48 * 8);
        assert_eq!(ev.node(&d, &[2, 1]).unwrap().value, Value::UInt(1));
        assert_eq!(ev.node(&d, &[2, 4]).unwrap().value, Value::Str("n_260825".into()));
        // Two boards, one bit each: a HackRF One and a HackRF One r9.
        let boards = ev.node(&d, &[2, 3]).unwrap().value;
        assert_eq!(boards, Value::Flags { raw: 0x0a, set: vec!["hackrf one".into(), "hackrf one r9".into()], unnamed: 0 });
    }

    #[test]
    fn the_vector_table_names_the_handlers() {
        let d = Document::new(MemSource(image()));
        let mut ev = Evaluator::new(hackrffw());
        let table = ev.node(&d, &[0]).unwrap();
        assert_eq!(table.child_count, 1 + EXCEPTIONS.len() as u64 + IRQS.len() as u64);
        assert_eq!(ev.node(&d, &[0, 0]).unwrap().name, "stack_top");
        assert_eq!(ev.node(&d, &[0, 0]).unwrap().value, Value::UInt(0x2000_0400));
        assert_eq!(ev.node(&d, &[0, 1]).unwrap().name, "reset");
        assert_eq!(ev.node(&d, &[0, 1]).unwrap().value, Value::UInt(0x7fb9));
        // Interrupt 8 on this part is the first USB controller.
        assert_eq!(ev.node(&d, &[0, 16 + 8]).unwrap().name, "usb0");
        assert_eq!(ev.node(&d, &[0, 16 + 3]).unwrap().name, "reserved_irq_3");
    }

    #[test]
    fn what_follows_the_record_reads_as_thumb_code() {
        let d = Document::new(MemSource(image()));
        let mut ev = Evaluator::new(hackrffw());
        let first = ev.node(&d, &[3, 0]).unwrap();
        assert_eq!(first.offset_bits, (INFO_AT as u64 + 48) * 8);
        let Value::Str(text) = first.value else { panic!("not an instruction: {:?}", first.value) };
        assert!(text.starts_with("push"), "{text}");
    }
}
