//! i.MX RT1060 flash images: what NXP's boot ROM reads out of a FlexSPI NOR
//! flash chip before it runs the program in it.
//!
//! An RT1060 has no flash of its own. It boots from a serial NOR chip on its
//! FlexSPI port, which it maps at 0x60000000 and runs the program from in
//! place. Before it can read that chip at full speed it has to know how to talk
//! to it, so the first 512 bytes of the flash are a FlexSPI configuration
//! block: the clock, how many data lines, and a lookup table of the command
//! sequences the chip answers to. The block opens with `FCFB`. That tag and
//! the image vector table at 0x1000 identify the file.
//!
//! The image vector table (IVT) holds the addresses the boot ROM needs: the
//! program's entry point, the boot data, and the device configuration data
//! (DCD). The boot data gives the start address and size of the whole image.
//! Every pointer in the IVT is an address in the processor's memory map, and
//! the IVT also holds its own address, so a pointer's offset in the flash is
//! the pointer minus the IVT's own address, plus 0x1000.
//!
//! The DCD is a short list of register writes and polls that the boot ROM runs
//! before the program: NXP's own boards use it to bring up the SDRAM
//! controller. It is big-endian, a header and then commands, each with its own
//! tag and length, and the boot ROM steps from one command to the next by that
//! length. This template reads the commands the same way. At the first tag
//! that is not one of the four command tags the boot ROM knows, the template
//! stops reading commands. It reads the rest of the DCD, up to the length in
//! the DCD header, as one bytes field. NXP does not say what the ROM does with
//! an unknown tag.
//!
//! The boot data and the DCD are read straight after the IVT, one after the
//! other, which is where NXP's SDK puts them, and each is checked against the
//! IVT's pointer to it. If a file keeps them somewhere else, they are read
//! from the wrong place and the check on each of them fails.
//!
//! The IVT's `entry` field holds the address of the program, and NXP's
//! startup code puts the Cortex-M7's vector table there: the stack pointer,
//! the fifteen exceptions every Cortex-M has, then the RT1060's interrupts as
//! NXP's `MIMXRT1062.h` names them, filled out to 256 words with interrupt
//! numbers the part does not use. What follows the table is read as Thumb
//! code, the way `hackrffw` reads a HackRF image: nothing in the flash says
//! where the code stops and the data starts.
//!
//! Other i.MX RT parts that lay out their flash the same way are recognised
//! too, and their vector tables get the RT1060's interrupt names.
//!
//! The code run ends at the first run of 4096 0xff bytes (4 KiB is the
//! smallest piece of a NOR chip that can be erased), and everything from
//! there on is read as bytes: the erased run, and whatever is programmed
//! after it. Files written by NXP's tools end with the program, so they have
//! no erased run. A dump of the whole chip has mostly erased bytes after the
//! program, and may have a second program further on, as the Tascam X8's
//! flash does. A listing counts the instructions in a run before it shows the
//! run's row, and counting the X8's 25 MB as instructions took nearly three
//! minutes in a debug build.
//!
//! Fields of NXP's structures keep NXP's names, put into snake case: the
//! configuration block's from the SDK's `fsl_flexspi_nor_config.h`, the IVT's
//! and the boot data's from `fsl_flexspi_nor_boot.h`, and the DCD's from the
//! reference manual. The other names are this template's.
//!
//! The address in each DCD write and check is shown as the name of the RT1060
//! register at that address: `CCM_CCGR0`, `IOMUXC_SW_MUX_CTL_PAD_GPIO_EMC_00`,
//! `SEMC_MCR`. Each name is the peripheral's name, an underscore and the
//! register's name. The `dcd.c` files in NXP's SDK name the addresses they
//! write this way, and so do most chapters of the reference manual. Rev. 3 of
//! the manual differs in two places. Its SEMC chapter writes the SEMC
//! registers without `SEMC_` (`MCR`), and the manual does not list the
//! registers of the GPIO_SPI pads at all. A name in this form is one word, so
//! it can be searched for as one.
//!
//! Table 9-42 of the manual lists the seven address ranges the boot ROM lets a
//! DCD write to (IOMUXC_SNVS_GPR, IOMUXC_SNVS, IOMUXC_GPR, IOMUXC, the CCM
//! analog block, CCM and SEMC). Every register that NXP's device header places
//! in those ranges is named in `registers::REGISTERS`. Every other address
//! gets one of two names, worked out from the registers of all the peripherals
//! the header defines:
//!
//! - `in` and a peripheral's name, such as `in LPUART1`, for a register of that
//!   peripheral outside the seven ranges, for an address partway into one of
//!   its registers, and for a reserved address whose nearest register below and
//!   nearest register above both belong to that peripheral. 0x402F0088 reads as
//!   `in SEMC` because it is between SEMC_DBICR1 and SEMC_IPCR0.
//! - `not a peripheral register` for every other address. That includes the
//!   reserved addresses before a peripheral's first register and after its
//!   last, even inside the seven ranges: 0x402F0100 is past SEMC's last
//!   register. It also includes a reserved address between registers of two
//!   different peripherals, as at 0x402E0808, where the registers of USBNC1 and
//!   USBNC2 alternate.
//!
//! Every address gets a name, so no address in a DCD is marked as a value the
//! format does not define. The boot ROM does not write to an address outside
//! the seven ranges: if any address in a write data command is outside them,
//! the boot ROM writes none of that command's values (the note above table
//! 9-42). This template does not check for that, so such a command is read and
//! shown like any other. The manual sets no range for the address a check data
//! command reads.
//!
//! The names come from `registers`, which `node tools/imxrtregisters.mjs`
//! writes from two files of NXP's MCUXpresso SDK at a fixed commit:
//! `MIMXRT1062.h`, the device header, for each peripheral's base address and
//! the offset of each of its registers, and `fsl_iomuxc.h` for the IOMUXC
//! pads the header does not name one by one. The script works out every
//! offset from the header's structs and stops if one disagrees with the
//! offset the header writes in the comment beside it. The register names are
//! the RT1060's, and an image for another i.MX RT part gets them too.

use std::sync::{Arc, OnceLock};

use crate::code::Isa;
use crate::template::{EnumDef, EnumSpan, Endian::*, Expr as E, Template, Ty as T, Until, Valid};

use super::hackrffw::EXCEPTIONS;

mod registers;

/// The tag at the start of the configuration block. A file is recognised by
/// this tag together with an IVT header at 0x1000.
pub const TAG: &[u8] = b"FCFB";

/// Where the boot ROM looks for the IVT when it boots from FlexSPI NOR: fixed
/// by the ROM, not by the image.
pub const IVT_AT: usize = 0x1000;

/// Bytes the configuration block takes, `sizeof(flexspi_nor_config_t)`.
const CONFIG_BYTES: i128 = 0x200;

/// Words in the vector table NXP's startup code writes: the stack pointer,
/// fifteen exceptions and 240 interrupts.
const VECTORS: usize = 256;

/// Whether the head of a file is an i.MX RT boot image: `FCFB` at the front
/// and an IVT header where the boot ROM looks for one. The IVT's header is its tag,
/// its length written big-endian, and a version the ROM accepts as 0x40 or
/// 0x41.
pub fn is_imxrt_flash(head: &[u8]) -> bool {
    head.starts_with(TAG) && matches!(head.get(IVT_AT..IVT_AT + 4), Some([0xd1, 0x00, 0x20, 0x40 | 0x41]))
}

pub fn imxrt() -> Template {
    Template::new(
        "imxrt",
        T::structure(
            "ImxrtFlash",
            vec![
                ("boot_header", boot_header()),
                ("padding", padding_to_entry()),
                ("program", program()),
            ],
        ),
    )
}

/// 4 KiB, the smallest piece of a NOR chip that can be erased. A run of this
/// many 0xff bytes, aligned or not, is taken as the end of the program; a
/// shorter run is taken as padding inside it.
const ERASED_SECTOR: usize = 0x1000;

/// The program at the IVT's entry: the vector table, the code up to the
/// first run of 4096 0xff bytes, and what comes after that as bytes.
fn program() -> T {
    let more = || E::Remaining.greater_than(E::lit(0));
    T::structure(
        "Program",
        vec![
            ("vectors", vectors()),
            ("code", thumb(E::to_bytes(&[0xff; ERASED_SECTOR]))),
            ("erased", T::when(more(), T::bytes(E::run(&[0xff])))),
            // Whatever is programmed after the erased run. The image does not
            // say what it is: a second program, a file system or settings.
            ("rest", T::when(more(), T::bytes(E::Remaining))),
        ],
    )
}

/// The RT1060's interrupts in IRQ number order, as `MIMXRT1062.h` names them.
/// Seven numbers are reserved by NXP: they still take a word in the vector
/// table, and are named for what they are.
const IRQS: &[&str] = &[
    "dma0_dma16", "dma1_dma17", "dma2_dma18", "dma3_dma19", "dma4_dma20", "dma5_dma21", "dma6_dma22",
    "dma7_dma23", "dma8_dma24", "dma9_dma25", "dma10_dma26", "dma11_dma27", "dma12_dma28", "dma13_dma29",
    "dma14_dma30", "dma15_dma31", "dma_error", "cti0_error", "cti1_error", "core", "lpuart1", "lpuart2",
    "lpuart3", "lpuart4", "lpuart5", "lpuart6", "lpuart7", "lpuart8", "lpi2c1", "lpi2c2", "lpi2c3", "lpi2c4",
    "lpspi1", "lpspi2", "lpspi3", "lpspi4", "can1", "can2", "flexram", "kpp", "tsc_dig", "gpr_irq", "lcdif",
    "csi", "pxp", "wdog2", "snvs_hp_wrapper", "snvs_hp_wrapper_tz", "snvs_lp_wrapper", "csu", "dcp",
    "dcp_vmi", "reserved_irq_52", "trng", "sjc", "bee", "sai1", "sai2", "sai3_rx", "sai3_tx", "spdif",
    "pmu_event", "reserved_irq_62", "temp_low_high", "temp_panic", "usb_phy1", "usb_phy2", "adc1", "adc2",
    "dcdc", "reserved_irq_70", "gpio10_combined_0_31", "gpio1_int0", "gpio1_int1", "gpio1_int2",
    "gpio1_int3", "gpio1_int4", "gpio1_int5", "gpio1_int6", "gpio1_int7", "gpio1_combined_0_15",
    "gpio1_combined_16_31", "gpio2_combined_0_15", "gpio2_combined_16_31", "gpio3_combined_0_15",
    "gpio3_combined_16_31", "gpio4_combined_0_15", "gpio4_combined_16_31", "gpio5_combined_0_15",
    "gpio5_combined_16_31", "flexio1", "flexio2", "wdog1", "rtwdog", "ewm", "ccm_1", "ccm_2", "gpc", "src",
    "reserved_irq_99", "gpt1", "gpt2", "pwm1_0", "pwm1_1", "pwm1_2", "pwm1_3", "pwm1_fault", "flexspi2",
    "flexspi", "semc", "usdhc1", "usdhc2", "usb_otg2", "usb_otg1", "enet", "enet_1588_timer",
    "xbar1_irq_0_1", "xbar1_irq_2_3", "adc_etc_irq0", "adc_etc_irq1", "adc_etc_irq2", "adc_etc_error_irq",
    "pit", "acmp1", "acmp2", "acmp3", "acmp4", "reserved_irq_127", "reserved_irq_128", "enc1", "enc2",
    "enc3", "enc4", "tmr1", "tmr2", "tmr3", "tmr4", "pwm2_0", "pwm2_1", "pwm2_2", "pwm2_3", "pwm2_fault",
    "pwm3_0", "pwm3_1", "pwm3_2", "pwm3_3", "pwm3_fault", "pwm4_0", "pwm4_1", "pwm4_2", "pwm4_3",
    "pwm4_fault", "enet2", "enet2_1588_timer", "can3", "reserved_irq_155", "flexio3", "gpio6_7_8_9",
];

/// Everything the boot ROM reads before the program: the configuration block,
/// the IVT, the boot data and the DCD, in the order NXP's SDK lays them out.
/// Shared with `tascamfw`, whose update files are an image of this flash.
pub(crate) fn boot_header() -> T {
    T::structure(
        "BootHeader",
        vec![
            ("flash_config", flash_config()),
            // Erased flash from the end of the 512-byte block to the IVT at
            // 0x1000. The SDK reserves the first 4 KiB of the flash for the
            // block.
            ("padding", T::bytes(E::lit(IVT_AT as i128 - CONFIG_BYTES))),
            ("ivt", ivt()),
            ("boot_data", boot_data()),
            // An image with nothing to set up before it runs has no DCD and
            // says so with a pointer of zero.
            ("dcd", T::when(E::within(&["ivt", "dcd"]).not_equal(E::lit(0)), dcd())),
        ],
    )
    .field_valid(
        "boot_data",
        Valid::Expr {
            expr: E::start_of(E::field("boot_data")).equal_to(pointer(&["ivt"], "boot_data")),
            msg: Some("Not where the IVT's boot_data pointer points".into()),
        },
    )
    .field_valid(
        "dcd",
        Valid::Expr {
            expr: E::within(&["ivt", "dcd"]).equal_to(E::lit(0)).either(E::start_of(E::field("dcd")).equal_to(pointer(&["ivt"], "dcd"))),
            msg: Some("Not where the IVT's dcd pointer points".into()),
        },
    )
}

/// The bytes between the boot header and the address in the IVT's `entry`
/// field, for a structure that holds `boot_header` as a field. Erased flash in
/// every image NXP's tools write.
pub(crate) fn padding_to_entry() -> T {
    T::bytes(pointer(&["boot_header", "ivt"], "entry").sub(E::SpacePos))
}

/// Where in the flash one of the IVT's pointers lands: the address it holds,
/// less the address the IVT gives for itself, plus where the IVT is. `ivt` is
/// the path to the IVT from the field asking.
///
/// Counted from the front of the space the field is read in, so it is a file
/// offset for a flash dump and an offset in the unpacked image for a Tascam
/// update.
fn pointer(ivt: &[&str], field: &str) -> E {
    let at = |name: &str| E::within(&ivt.iter().copied().chain([name]).collect::<Vec<_>>());
    at(field).sub(at("self")).add(E::lit(IVT_AT as i128))
}

/// `len` bytes read as Thumb code, one instruction after another.
pub(crate) fn thumb(len: E) -> T {
    T::sized(len, T::repeat(T::insn(Isa::Thumb), Until::End))
}

/// The vector table, one named word at a time. Shared with `tascamfw`, whose
/// bootloader and application each start with one.
pub(crate) fn vectors() -> T {
    // The slots past the last interrupt NXP names. In the X8's two programs
    // they mostly hold the default handler, and the last word is 0xffffffff.
    let unnamed: Vec<String> = (IRQS.len()..VECTORS - 1 - EXCEPTIONS.len()).map(|n| format!("irq_{n}")).collect();
    // Not a handler: the core loads the first word into the stack pointer
    // before it runs anything, so it is an address in RAM.
    let names = std::iter::once("stack_top").chain(EXCEPTIONS.iter().copied()).chain(IRQS.iter().copied()).chain(unnamed.iter().map(String::as_str));
    T::structure("Vectors", names.map(|name| (name, T::u32(Little))).collect())
}

/// `flexspi_nor_config_t`: the settings every FlexSPI device has, and then the
/// ones only a NOR chip has.
fn flash_config() -> T {
    T::structure(
        "FlexspiNorConfig",
        vec![
            ("mem_config", mem_config()),
            ("page_size", T::u32(Little)),
            ("sector_size", T::u32(Little)),
            ("ipcmd_serial_clk_freq", T::enumeration("IpcmdSerialClkFreq", T::u8(), &[&[(0i128, "no change")][..], SERIAL_CLOCK].concat())),
            ("is_uniform_block_size", T::u8()),
            ("reserved0", T::bytes(E::lit(2))),
            ("serial_nor_type", T::enumeration("SerialNorType", T::u8(), SERIAL_NOR_TYPE)),
            ("need_exit_no_cmd_mode", T::u8()),
            ("half_clk_for_non_read_cmd", T::u8()),
            ("need_restore_no_cmd_mode", T::u8()),
            ("block_size", T::u32(Little)),
            ("reserve2", T::bytes(E::lit(44))),
        ],
    )
}

/// `flexspi_mem_config_t`, the first 448 bytes of the block.
fn mem_config() -> T {
    let u32s = |n: i128| T::array(T::u32(Little), E::lit(n));
    T::structure(
        "FlexspiMemConfig",
        vec![
            ("tag", T::magic(TAG)),
            ("version", version()),
            ("reserved0", T::u32(Little)),
            ("read_sample_clk_src", T::enumeration("ReadSampleClkSrc", T::u8(), READ_SAMPLE_CLOCK)),
            ("cs_hold_time", T::u8()),
            ("cs_setup_time", T::u8()),
            ("column_address_width", T::u8()),
            ("device_mode_cfg_enable", T::u8()),
            ("device_mode_type", device_mode_type()),
            ("wait_time_cfg_commands", T::u16(Little)),
            ("device_mode_seq", lut_seq()),
            ("device_mode_arg", T::u32(Little)),
            ("config_cmd_enable", T::u8()),
            ("config_mode_type", T::array(device_mode_type(), E::lit(3))),
            ("config_cmd_seqs", T::array(lut_seq(), E::lit(3))),
            ("reserved1", T::u32(Little)),
            ("config_cmd_args", u32s(3)),
            ("reserved2", T::u32(Little)),
            ("controller_misc_option", T::flags("ControllerMiscOption", T::u32(Little), CONTROLLER_MISC_OPTION)),
            ("device_type", T::enumeration_hex("DeviceType", T::u8(), DEVICE_TYPE)),
            ("sflash_pad_type", T::enumeration("SflashPadType", T::u8(), &[(1, "1 pad"), (2, "2 pads"), (4, "4 pads"), (8, "8 pads")])),
            ("serial_clk_freq", T::enumeration("SerialClkFreq", T::u8(), SERIAL_CLOCK)),
            ("lut_custom_seq_enable", T::u8()),
            ("reserved3", T::bytes(E::lit(8))),
            ("sflash_a1_size", T::u32(Little)),
            ("sflash_a2_size", T::u32(Little)),
            ("sflash_b1_size", T::u32(Little)),
            ("sflash_b2_size", T::u32(Little)),
            ("cs_pad_setting_override", T::u32(Little)),
            ("sclk_pad_setting_override", T::u32(Little)),
            ("data_pad_setting_override", T::u32(Little)),
            ("dqs_pad_setting_override", T::u32(Little)),
            ("timeout_in_ms", T::u32(Little)),
            ("command_interval", T::u32(Little)),
            ("data_valid_time", T::array(T::u16(Little), E::lit(2))),
            ("busy_offset", T::u16(Little)),
            ("busy_bit_polarity", T::enumeration("BusyBitPolarity", T::u16(Little), &[(0, "busy when bit is 1"), (1, "busy when bit is 0")])),
            ("lookup_table", lookup_table()),
            ("lut_custom_seq", T::array(lut_seq(), E::lit(12))),
            ("reserved4", T::bytes(E::lit(16))),
        ],
    )
}

/// The block's version, one little-endian word whose bytes are, from the top,
/// the letter `V` and the major, minor and bugfix numbers: 0x56010400 is
/// V1.4.0. Read a byte at a time from the bottom, which is the order the file
/// holds them in.
fn version() -> T {
    T::inline_structure("ConfigVersion", vec![("bugfix", T::u8()), ("minor", T::u8()), ("major", T::u8()), ("letter_v", T::magic(b"V"))])
}

/// The clock the boot ROM drives the flash at, as the RT1060's
/// `kFlexSpiSerialClk_*` numbers it. The boot ROM uses 30 MHz for any other
/// value.
const SERIAL_CLOCK: &[(i128, &str)] = &[
    (1, "30 MHz"),
    (2, "50 MHz"),
    (3, "60 MHz"),
    (4, "75 MHz"),
    (5, "80 MHz"),
    (6, "100 MHz"),
    (7, "120 MHz"),
    (8, "133 MHz"),
    (9, "166 MHz"),
];

/// Where FlexSPI takes the clock it samples read data with,
/// `kFlexSPIReadSampleClk_*`.
const READ_SAMPLE_CLOCK: &[(i128, &str)] = &[
    (0, "internal loopback"),
    (1, "loopback from DQS pad"),
    (2, "loopback from SCK pad"),
    (3, "flash-provided DQS"),
];

/// What a configuration command does, `kDeviceConfigCmdType_*`.
fn device_mode_type() -> T {
    T::enumeration(
        "DeviceConfigCmdType",
        T::u8(),
        &[(0, "generic"), (1, "quad enable"), (2, "SPI to XPI"), (3, "XPI to SPI"), (4, "SPI to no-command mode"), (5, "reset")],
    )
}

/// `kFlexSpiMiscOffset_*`, one bit each.
const CONTROLLER_MISC_OPTION: &[(u32, &str)] = &[
    (0, "differential clock"),
    (1, "CK2"),
    (2, "parallel mode"),
    (3, "word addressable"),
    (4, "safe config frequency"),
    (5, "pad setting override"),
    (6, "DDR mode"),
];

/// `kFlexSpiDeviceType_*`. The reference manual lists the first two.
const DEVICE_TYPE: &[(i128, &str)] = &[
    (1, "serial NOR"),
    (2, "serial NAND"),
    (3, "serial RAM"),
    (0x12, "multi-chip NOR and NAND"),
    (0x13, "multi-chip NOR and RAM"),
];

/// `kSerialNorType_*`.
const SERIAL_NOR_TYPE: &[(i128, &str)] = &[(0, "standard SPI"), (1, "HyperBus"), (2, "XPI"), (3, "no-command mode")];

/// `flexspi_lut_seq_t`: how many lookup table sequences a command takes, and
/// the first of them.
fn lut_seq() -> T {
    T::structure("LutSeq", vec![("seq_num", T::u8()), ("seq_id", T::u8()), ("reserved", T::u16(Little))])
}

/// The sixteen lookup table sequences, each named for what the boot ROM uses
/// that index for. The reference manual assigns nine of them and the SDK three
/// more; the other four are free for a custom sequence.
const LUT_SEQUENCES: &[&str] = &[
    "read",
    "read_status",
    "read_status_xpi",
    "write_enable",
    "write_enable_xpi",
    "erase_sector",
    "seq_6",
    "seq_7",
    "erase_block",
    "page_program",
    "seq_10",
    "chip_erase",
    "seq_12",
    "read_sfdp",
    "restore_no_cmd",
    "dummy",
];

/// `lookupTable[64]`: sixteen sequences of four words, and each word is two
/// instructions, the low half first. So a sequence is eight instructions in
/// the order they run.
fn lookup_table() -> T {
    T::structure("LookupTable", LUT_SEQUENCES.iter().map(|name| (*name, T::array(lut_instruction(), E::lit(8)))).collect())
}

/// One FlexSPI instruction, sixteen bits: an opcode in the top six, how many
/// data lines in the two below, and an operand in the low byte. Read from the
/// bottom of the little-endian halfword, so the operand comes first.
fn lut_instruction() -> T {
    let bits = |n: u32| T::UInt { bits: n, endian: Little };
    T::inline_structure(
        "LutInstruction",
        vec![
            ("operand", T::u8()),
            ("num_pads", T::enumeration("NumPads", bits(2), &[(0, "1 pad"), (1, "2 pads"), (2, "4 pads"), (3, "8 pads")])),
            ("opcode", T::enumeration_hex("LutOpcode", bits(6), LUT_OPCODES)),
        ],
    )
    .reads_as(&[("opcode", "", ""), ("num_pads", "", ""), ("operand", "", "")])
}

/// The FlexSPI instruction set, as the reference manual names it.
const LUT_OPCODES: &[(i128, &str)] = &[
    (0x00, "STOP"),
    (0x01, "CMD_SDR"),
    (0x02, "RADDR_SDR"),
    (0x03, "CADDR_SDR"),
    (0x04, "MODE1_SDR"),
    (0x05, "MODE2_SDR"),
    (0x06, "MODE4_SDR"),
    (0x07, "MODE8_SDR"),
    (0x08, "WRITE_SDR"),
    (0x09, "READ_SDR"),
    (0x0a, "LEARN_SDR"),
    (0x0b, "DATSZ_SDR"),
    (0x0c, "DUMMY_SDR"),
    (0x0d, "DUMMY_RWDS_SDR"),
    (0x1f, "JMP_ON_CS"),
    (0x21, "CMD_DDR"),
    (0x22, "RADDR_DDR"),
    (0x23, "CADDR_DDR"),
    (0x24, "MODE1_DDR"),
    (0x25, "MODE2_DDR"),
    (0x26, "MODE4_DDR"),
    (0x27, "MODE8_DDR"),
    (0x28, "WRITE_DDR"),
    (0x29, "READ_DDR"),
    (0x2a, "LEARN_DDR"),
    (0x2b, "DATSZ_DDR"),
    (0x2c, "DUMMY_DDR"),
    (0x2d, "DUMMY_RWDS_DDR"),
];

/// The header the IVT and the DCD both open with: a tag, a length written
/// big-endian whatever the rest of the structure is, and a version.
fn hdr(name: &str, tag: u8) -> T {
    T::structure(name, vec![("tag", T::magic(&[tag])), ("length", T::u16(Big)), ("version", T::u8())])
}

/// `struct ivt`: a header and seven words. entry, dcd, boot_data, self and
/// csf are addresses in the processor's memory map.
fn ivt() -> T {
    T::structure(
        "Ivt",
        vec![
            ("hdr", hdr("IvtHeader", 0xd1)),
            ("entry", T::u32(Little)),
            ("reserved1", T::u32(Little)),
            ("dcd", T::u32(Little)),
            ("boot_data", T::u32(Little)),
            ("self", T::u32(Little)),
            // The signed image's command sequence, for a part that checks
            // signatures before it boots. Zero when the image is not signed.
            ("csf", T::u32(Little)),
            ("reserved2", T::u32(Little)),
        ],
    )
}

/// `BOOT_DATA_T`. The reference manual stops after `plugin`; the SDK pads the
/// structure to sixteen bytes with a word of all ones.
fn boot_data() -> T {
    T::structure(
        "BootData",
        vec![("start", T::u32(Little)), ("size", T::u32(Little)), ("plugin", T::u32(Little)), ("placeholder", T::u32(Little))],
    )
}

/// The commands the boot ROM knows, by tag. Unlock is a HAB command the RT1060
/// manual does not list for the DCD, and is here because NXP's signing tools
/// write it there.
const DCD_TAGS: &[(i128, &str)] = &[(0xcc, "write data"), (0xcf, "check data"), (0xc0, "nop"), (0xb2, "unlock")];

/// The DCD, as long as its header says, header included.
fn dcd() -> T {
    T::sized(
        E::peek_at(E::lit(8), 16, Big),
        T::structure(
            "Dcd",
            vec![
                ("hdr", hdr("DcdHeader", 0xd2)),
                ("commands", T::repeat(dcd_command(), Until::While(command_ahead()))),
                // The rest of the DCD, from the first tag that is not one of
                // the four commands the boot ROM knows to the length in the
                // DCD header. Absent when the commands fill the DCD.
                ("rest_from_unknown_tag", T::when(E::Remaining.greater_than(E::lit(0)), T::bytes(E::Remaining))),
            ],
        ),
    )
}

/// Whether the bytes ahead start a command the boot ROM knows: room for a
/// command header, and one of the four tags.
fn command_ahead() -> E {
    let tag = E::peek(8, Big);
    let known = DCD_TAGS.iter().map(|(t, _)| tag.clone().equal_to(E::lit(*t))).reduce(E::either).expect("four tags");
    E::Remaining.greater_than(E::lit(3)).both(known)
}

/// One command: a tag, a length that includes this 4-byte header, a
/// parameter byte, and a body whose layout depends on the tag.
fn dcd_command() -> T {
    let tag = || E::field("tag");
    T::structure_named(
        "DcdCommand",
        "tag",
        "body",
        vec![
            ("tag", T::enumeration_hex("DcdTag", T::u8(), DCD_TAGS)),
            ("length", T::u16(Big)),
            // The width and action of each write or check. For an unlock
            // command, this byte names the engine to unlock. A nop command
            // ignores it.
            ("parameter", T::switch(tag(), vec![(0xcc, write_parameter()), (0xcf, check_parameter())], T::u8())),
            (
                "body",
                T::sized(
                    E::field("length").sub(E::lit(4)),
                    T::switch(
                        tag(),
                        vec![
                            (0xcc, T::repeat(T::structure_named("WriteData", "address", "", vec![("address", register_address()), ("value", T::u32(Big))]), Until::End)),
                            (0xcf, check_data()),
                            (0xb2, T::repeat(T::u32(Big), Until::End)),
                        ],
                        T::bytes(E::Remaining),
                    ),
                ),
            ),
        ],
    )
}

/// The parameter byte of a write: the mask and set flags in bits 3 and 4, and
/// the width in bytes in the three bits below.
fn write_parameter() -> T {
    T::inline_structure(
        "WriteParameter",
        vec![
            ("flags", T::enumeration("WriteAction", T::UInt { bits: 5, endian: Big }, &[(0, "write value"), (1, "clear bitmask"), (2, "write value (set flag ignored)"), (3, "set bitmask")])),
            ("bytes", T::UInt { bits: 3, endian: Big }),
        ],
    )
}

/// The same byte for a check, where the two flags say which condition the
/// boot ROM polls for.
fn check_parameter() -> T {
    T::inline_structure(
        "CheckParameter",
        vec![
            ("flags", T::enumeration("CheckCondition", T::UInt { bits: 5, endian: Big }, &[(0, "all bits clear"), (1, "any bit clear"), (2, "all bits set"), (3, "any bit set")])),
            ("bytes", T::UInt { bits: 3, endian: Big }),
        ],
    )
}

/// What a check polls: an address and a mask, and how many times to try. With
/// no count the boot ROM polls until the condition holds.
fn check_data() -> T {
    T::structure_named(
        "CheckData",
        "address",
        "",
        vec![
            ("address", register_address()),
            ("mask", T::u32(Big)),
            ("count", T::when(E::Remaining.greater_than(E::lit(0)), T::u32(Big))),
        ],
    )
}

/// The address in a DCD write or check, named for the RT1060 register there.
/// See the module docs for what the names are and where they come from.
fn register_address() -> T {
    T::Enum { inner: Box::new(T::u32(Big)), def: Arc::clone(register_names()) }
}

/// The names of the RT1060's registers as one enum, built the first time a
/// template asks for it and shared after that, so building the template
/// clones an `Arc` instead of allocating a thousand strings.
///
/// The registers are its cases. Every other address takes its name from
/// the span it falls in: an enum tries its spans in order and takes the first
/// that starts at or below the value, so the areas of the address space go in
/// from the top down, and each one names everything from its start to the
/// start of the one above it.
fn register_names() -> &'static Arc<EnumDef> {
    static NAMES: OnceLock<Arc<EnumDef>> = OnceLock::new();
    NAMES.get_or_init(|| {
        let spans = registers::AREAS
            .iter()
            .rev()
            .map(|&(from, peripheral)| EnumSpan {
                from: i128::from(from),
                step: 1,
                label: match peripheral {
                    Some(p) => format!("in {p}"),
                    None => "not a peripheral register".to_string(),
                },
            })
            .collect();
        Arc::new(EnumDef {
            name: "Rt1060Register".to_string(),
            cases: registers::REGISTERS.iter().map(|&(at, name)| (i128::from(at), name.to_string())).collect(),
            docs: Vec::new(),
            spans,
            hex: true,
        })
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    /// The address the flash is mapped at, which the IVT's pointers count in.
    pub(crate) const XIP: u32 = 0x6000_0000;

    /// A boot image laid out the way NXP's SDK lays one out: the block, the
    /// IVT at 0x1000, the boot data and a DCD straight after it, and a vector
    /// table and two instructions at 0x2000.
    pub(crate) fn image(dcd: &[u8]) -> Vec<u8> {
        let mut v = boot_header(dcd);
        v.extend_from_slice(&0x2008_0000u32.to_le_bytes());
        v.extend_from_slice(&(XIP + 0xa0d5).to_le_bytes());
        while v.len() < 0x2000 + VECTORS * 4 {
            v.extend_from_slice(&(XIP + 0xa3f1).to_le_bytes());
        }
        // `push {r3-r7, lr}` and `movs r4, #0`.
        v.extend_from_slice(&[0xf8, 0xb5, 0x00, 0x24]);
        v
    }

    /// The same image up to the IVT's entry at 0x2000, for a test that puts
    /// its own program there.
    pub(crate) fn boot_header(dcd: &[u8]) -> Vec<u8> {
        let mut v = b"FCFB".to_vec();
        v.extend_from_slice(&0x5601_0400u32.to_le_bytes());
        v.resize(0x45, 0);
        // Four data lines at 80 MHz.
        v.extend_from_slice(&[4, 5]);
        v.resize(0x80, 0);
        // A quad read: command 0xEC on one line, a 32-bit address on four,
        // six dummy cycles, then the data on four.
        v.extend_from_slice(&[0xec, 0x04, 0x20, 0x0a, 0x06, 0x32, 0x04, 0x26]);
        v.resize(CONFIG_BYTES as usize, 0);
        v.resize(IVT_AT, 0xff);
        let dcd_at = if dcd.is_empty() { 0 } else { XIP + 0x1030 };
        for word in [0x4120_00d1, XIP + 0x2000, 0, dcd_at, XIP + 0x1020, XIP + 0x1000, 0, 0] {
            v.extend_from_slice(&u32::to_le_bytes(word));
        }
        for word in [XIP, 0x0200_0000, 0, 0xffff_ffff] {
            v.extend_from_slice(&u32::to_le_bytes(word));
        }
        v.extend_from_slice(dcd);
        v.resize(0x2000, 0xff);
        v
    }

    /// A DCD of one write, one check, and then eight bytes whose tag the boot
    /// ROM does not know.
    pub(crate) fn dcd_with_a_stray() -> Vec<u8> {
        let mut d = vec![0xd2, 0x00, 0x2c, 0x41];
        d.extend_from_slice(&[0xcc, 0x00, 0x14, 0x04]);
        d.extend_from_slice(&[0x40, 0x0f, 0xc0, 0x68, 0xff, 0xff, 0xff, 0xff]);
        d.extend_from_slice(&[0x40, 0x0f, 0xc0, 0x6c, 0xff, 0xff, 0xff, 0xff]);
        d.extend_from_slice(&[0xcf, 0x00, 0x0c, 0x1c, 0x40, 0x2f, 0x00, 0x3c, 0x00, 0x00, 0x00, 0x01]);
        d.extend_from_slice(&[0x40, 0x2f, 0x00, 0x9c, 0xa5, 0x5a, 0x00, 0x0f]);
        assert_eq!(d.len(), 0x2c);
        d
    }

    fn named(ev: &mut Evaluator, d: &Document<MemSource>, path: &[usize], name: &str) -> Vec<usize> {
        ev.child_named(d, path, name).unwrap().unwrap_or_else(|| panic!("no {name} under {path:?}"))
    }

    fn value(ev: &mut Evaluator, d: &Document<MemSource>, path: &[usize], names: &[&str]) -> Value {
        let mut p = path.to_vec();
        for name in names {
            p = named(ev, d, &p, name);
        }
        ev.node(d, &p).unwrap().value
    }

    #[test]
    fn the_head_of_a_boot_image_is_recognised() {
        let v = image(&dcd_with_a_stray());
        assert!(is_imxrt_flash(&v));
        let mut wrong = v.clone();
        wrong[IVT_AT + 3] = 0x42;
        assert!(!is_imxrt_flash(&wrong));
    }

    #[test]
    fn the_lookup_table_splits_each_instruction_into_its_three_parts() {
        let d = Document::new(MemSource(image(&[])));
        let mut ev = Evaluator::new(imxrt());
        let config = [0, 0, 0];
        assert_eq!(ev.node(&d, &config).unwrap().name, "mem_config");
        assert_eq!(value(&mut ev, &d, &config, &["serial_clk_freq"]).as_int(), Some(5));
        assert_eq!(value(&mut ev, &d, &config, &["version", "major"]).as_int(), Some(1));
        assert_eq!(value(&mut ev, &d, &config, &["version", "minor"]).as_int(), Some(4));
        let table = named(&mut ev, &d, &config, "lookup_table");
        let read = named(&mut ev, &d, &table, "read");
        let first = [&read[..], &[0]].concat();
        assert_eq!(value(&mut ev, &d, &first, &["opcode"]).as_int(), Some(0x01));
        assert_eq!(value(&mut ev, &d, &first, &["num_pads"]).as_int(), Some(0));
        assert_eq!(value(&mut ev, &d, &first, &["operand"]).as_int(), Some(0xec));
        let second = [&read[..], &[1]].concat();
        assert_eq!(value(&mut ev, &d, &second, &["opcode"]).as_int(), Some(0x02));
        assert_eq!(value(&mut ev, &d, &second, &["num_pads"]).as_int(), Some(2));
        assert_eq!(value(&mut ev, &d, &second, &["operand"]).as_int(), Some(0x20));
        let fourth = [&read[..], &[3]].concat();
        assert_eq!(value(&mut ev, &d, &fourth, &["opcode"]).as_int(), Some(0x09));
        // The whole block is the size the SDK says, so the IVT lands at 0x1000.
        assert_eq!(ev.node(&d, &[0, 0]).unwrap().size_bits, CONFIG_BYTES as u64 * 8);
        let ivt = named(&mut ev, &d, &[0], "ivt");
        assert_eq!(ev.node(&d, &ivt).unwrap().offset_bits, IVT_AT as u64 * 8);
    }

    #[test]
    fn the_dcd_is_read_command_by_command_and_stops_at_a_tag_the_rom_does_not_know() {
        let d = Document::new(MemSource(image(&dcd_with_a_stray())));
        let mut ev = Evaluator::new(imxrt());
        let dcd = named(&mut ev, &d, &[0], "dcd");
        let node = ev.node(&d, &dcd).unwrap();
        assert_eq!((node.offset_bits / 8, node.size_bits / 8), (0x1030, 0x2c));
        let commands = named(&mut ev, &d, &dcd, "commands");
        assert_eq!(ev.node(&d, &commands).unwrap().child_count, 2);
        let write = [&commands[..], &[0]].concat();
        assert_eq!(value(&mut ev, &d, &write, &["parameter", "flags"]).as_int(), Some(0));
        assert_eq!(value(&mut ev, &d, &write, &["parameter", "bytes"]).as_int(), Some(4));
        let body = named(&mut ev, &d, &write, "body");
        assert_eq!(ev.node(&d, &body).unwrap().child_count, 2);
        let check = [&commands[..], &[1]].concat();
        // 0x1C: poll until any bit of the mask is set.
        assert_eq!(value(&mut ev, &d, &check, &["parameter", "flags"]).as_int(), Some(3));
        assert_eq!(value(&mut ev, &d, &check, &["body", "mask"]).as_int(), Some(1));
        let stray = named(&mut ev, &d, &dcd, "rest_from_unknown_tag");
        let stray = ev.node(&d, &stray).unwrap();
        assert_eq!((stray.offset_bits / 8, stray.size_bits / 8), (0x1054, 8));
        // Both pieces are where the IVT says.
        for name in ["boot_data", "dcd"] {
            let at = named(&mut ev, &d, &[0], name);
            let verdict = ev.valid_of(&d, &at).unwrap().expect("a constraint");
            assert!(verdict.ok, "{name}: {}", verdict.text);
        }
    }

    /// A DCD of one write command, with a zero to write at each address.
    fn dcd_writing(addresses: &[u32]) -> Vec<u8> {
        let mut d = vec![0xd2, 0x00, 0x00, 0x41, 0xcc];
        d.extend_from_slice(&(4 + 8 * addresses.len() as u16).to_be_bytes());
        d.push(0x04);
        for at in addresses {
            d.extend_from_slice(&at.to_be_bytes());
            d.extend_from_slice(&[0; 4]);
        }
        let len = d.len() as u16;
        d[1..3].copy_from_slice(&len.to_be_bytes());
        d
    }

    /// The name each address in a DCD's first command reads as, and whether
    /// any of them is marked as a problem.
    fn written_names(dcd: &[u8]) -> (Vec<String>, bool) {
        let d = Document::new(MemSource(image(dcd)));
        let mut ev = Evaluator::new(imxrt());
        let dcd = named(&mut ev, &d, &[0], "dcd");
        let commands = named(&mut ev, &d, &dcd, "commands");
        let body = named(&mut ev, &d, &[&commands[..], &[0]].concat(), "body");
        let mut names = Vec::new();
        let mut marked = false;
        for i in 0..ev.node(&d, &body).unwrap().child_count as usize {
            let at = named(&mut ev, &d, &[&body[..], &[i]].concat(), "address");
            let node = ev.node(&d, &at).unwrap();
            marked |= node.problem.is_some();
            let Value::Enum { name: Some(name), hex: true, .. } = node.value else { panic!("not a named address: {:?}", node.value) };
            names.push(name);
        }
        (names, marked)
    }

    #[test]
    fn each_address_in_the_dcd_is_named_for_its_register() {
        let d = Document::new(MemSource(image(&dcd_with_a_stray())));
        let mut ev = Evaluator::new(imxrt());
        let commands = named(&mut ev, &d, &[0, 4], "commands");
        let write = named(&mut ev, &d, &[&commands[..], &[0]].concat(), "body");
        let check = named(&mut ev, &d, &[&commands[..], &[1]].concat(), "body");
        let name = |ev: &mut Evaluator, path: &[usize]| match value(ev, &d, path, &["address"]) {
            Value::Enum { raw, name: Some(name), .. } => (raw, name),
            other => panic!("not a named address: {other:?}"),
        };
        assert_eq!(name(&mut ev, &[&write[..], &[0]].concat()), (0x400f_c068, "CCM_CCGR0".to_string()));
        assert_eq!(name(&mut ev, &[&write[..], &[1]].concat()), (0x400f_c06c, "CCM_CCGR1".to_string()));
        assert_eq!(name(&mut ev, &check), (0x402f_003c, "SEMC_INTR".to_string()));
        // Each pair is named for the register it writes.
        assert!(ev.node(&d, &[&write[..], &[1]].concat()).unwrap().name.contains("CCM_CCGR1"));
    }

    #[test]
    fn an_address_that_is_not_a_register_is_named_for_where_it_is_and_not_marked() {
        let (names, marked) = written_names(&dcd_writing(&[
            0x2000_0000, // DTCM
            0x402f_0100, // SEMC's 16 KiB, past its last register
            0x4018_4010, // LPUART1's BAUD, which is not in a DCD range
            0x402f_0002, // two bytes into SEMC_MCR
            0x402f_0088, // reserved, between SEMC_DBICR1 and SEMC_IPCR0
            0x400d_81c4, // reserved, inside USB_ANALOG's registers for USB1
            0x402e_0808, // between USBNC2's first register and USBNC1's second
            0xffff_fffc,
            0x0000_0000,
        ]));
        assert!(!marked, "{names:?}");
        let outside = "not a peripheral register";
        assert_eq!(names, [outside, outside, "in LPUART1", "in SEMC", "in SEMC", "in USB_ANALOG", outside, outside, outside]);
    }

    /// Registers checked one by one against the offsets in the comments of
    /// NXP's `MIMXRT1062.h` and the base addresses it defines, and against
    /// the pad macros of `fsl_iomuxc.h` for the GPIO_SPI pads.
    #[test]
    fn the_register_names_match_nxps_header() {
        let names = register_names();
        let want = [
            (0x400f_c000 + 0x14, "CCM_CBCDR"),
            (0x400f_c000 + 0x68, "CCM_CCGR0"),
            (0x400d_8000 + 0x30, "CCM_ANALOG_PLL_SYS"),
            (0x400d_8000 + 0x100, "CCM_ANALOG_PFD_528"),
            // PMU_MISC0 and XTALOSC24M_MISC0 are the same register.
            (0x400d_8000 + 0x150, "CCM_ANALOG_MISC0"),
            // USB_ANALOG's second INSTANCE in the header.
            (0x400d_8000 + 0x1a0 + 0x60, "USB_ANALOG_USB2_VBUS_DETECT"),
            // DEBUGr in the header.
            (0x400d_9000 + 0x50, "USBPHY1_DEBUG"),
            (0x402f_0000, "SEMC_MCR"),
            (0x402f_0000 + 0x10, "SEMC_BR0"),
            (0x402f_0000 + 0x30, "SEMC_BR8"),
            (0x402f_0000 + 0x3c, "SEMC_INTR"),
            (0x402f_0000 + 0x9c, "SEMC_IPCMD"),
            // The first element of each IOMUXC array.
            (0x401f_8000 + 0x14, "IOMUXC_SW_MUX_CTL_PAD_GPIO_EMC_00"),
            (0x401f_8000 + 0x204, "IOMUXC_SW_PAD_CTL_PAD_GPIO_EMC_00"),
            (0x401f_8000 + 0x3f4, "IOMUXC_ANATOP_USB_OTG1_ID_SELECT_INPUT"),
            (0x401f_8000 + 0x65c, "IOMUXC_SW_MUX_CTL_PAD_GPIO_SPI_B0_00"),
            (0x401f_8000 + 0x6b0, "IOMUXC_SW_MUX_CTL_PAD_GPIO_SPI_B1_07"),
            (0x401f_8000 + 0x6b4, "IOMUXC_SW_PAD_CTL_PAD_GPIO_SPI_B0_00"),
            (0x401f_8000 + 0x70c, "IOMUXC_ENET2_IPG_CLK_RMII_SELECT_INPUT"),
            (0x400a_c000, "IOMUXC_GPR_GPR0"),
            (0x400a_8000, "IOMUXC_SNVS_SW_MUX_CTL_PAD_WAKEUP"),
            (0x400a_4000, "IOMUXC_SNVS_GPR_GPR0"),
        ];
        for (at, name) in want {
            assert_eq!(names.label(at), Some(name), "{at:#x}");
        }
    }

    /// The lookup reads the areas from the top down and takes the first one
    /// at or below an address, which is only right if they are in order.
    #[test]
    fn the_generated_tables_are_in_address_order_and_inside_the_dcd_ranges() {
        assert!(registers::REGISTERS.windows(2).all(|w| w[0].0 < w[1].0));
        assert!(registers::AREAS.windows(2).all(|w| w[0].0 < w[1].0 && w[0].1 != w[1].1));
        assert_eq!(registers::AREAS[0], (0, None));
        // Table 9-42 of the reference manual.
        let ranges = [0x400a_4000, 0x400a_8000, 0x400a_c000, 0x401f_8000, 0x400d_8000, 0x400f_c000, 0x402f_0000];
        for (at, name) in registers::REGISTERS {
            assert!(ranges.iter().any(|r| (*r..r + 0x4000).contains(at)), "{name} at {at:#x}");
        }
    }

    #[test]
    fn a_dcd_the_ivt_does_not_point_at_is_not_there() {
        let d = Document::new(MemSource(image(&[])));
        let mut ev = Evaluator::new(imxrt());
        let dcd = named(&mut ev, &d, &[0], "dcd");
        assert!(ev.node(&d, &dcd).unwrap().absent);
        // A dcd pointer of 0 means the image has no DCD, so the constraint
        // passes.
        assert!(ev.valid_of(&d, &dcd).unwrap().is_none_or(|v| v.ok));
        // So the padding runs from the end of the boot data to the entry.
        let padding = ev.node(&d, &[1]).unwrap();
        assert_eq!((padding.offset_bits / 8, padding.size_bits / 8), (0x1030, 0x2000 - 0x1030));
    }

    #[test]
    fn a_boot_data_somewhere_else_is_flagged() {
        let mut v = image(&[]);
        // The IVT's boot_data pointer, moved four bytes on.
        v[IVT_AT + 16..IVT_AT + 20].copy_from_slice(&(XIP + 0x1024).to_le_bytes());
        let d = Document::new(MemSource(v));
        let mut ev = Evaluator::new(imxrt());
        let at = named(&mut ev, &d, &[0], "boot_data");
        let verdict = ev.valid_of(&d, &at).unwrap().expect("a constraint");
        assert!(!verdict.ok);
        assert_eq!(verdict.text, "Fails the check: Not where the IVT's boot_data pointer points");
    }

    #[test]
    fn the_program_at_the_entry_is_a_vector_table_and_thumb_code() {
        let d = Document::new(MemSource(image(&dcd_with_a_stray())));
        let mut ev = Evaluator::new(imxrt());
        let vectors = ev.node(&d, &[2, 0]).unwrap();
        assert_eq!(vectors.offset_bits / 8, 0x2000);
        assert_eq!(vectors.child_count, VECTORS as u64);
        assert_eq!(ev.node(&d, &[2, 0, 1]).unwrap().name, "reset");
        assert_eq!(ev.node(&d, &[2, 0, 1]).unwrap().value.as_int(), Some(0x6000_a0d5));
        // Interrupt 20 on this part is the first LPUART, and 52 is reserved.
        assert_eq!(ev.node(&d, &[2, 0, 16 + 20]).unwrap().name, "lpuart1");
        assert_eq!(ev.node(&d, &[2, 0, 16 + 52]).unwrap().name, "reserved_irq_52");
        assert_eq!(ev.node(&d, &[2, 0, 16 + 157]).unwrap().name, "gpio6_7_8_9");
        assert_eq!(ev.node(&d, &[2, 0, 16 + 158]).unwrap().name, "irq_158");
        assert_eq!(ev.node(&d, &[2, 0, VECTORS - 1]).unwrap().name, "irq_239");
        let first = ev.node(&d, &[2, 1, 0]).unwrap();
        assert_eq!(first.offset_bits / 8, 0x2400);
        let Value::Str(text) = first.value else { panic!("not an instruction: {:?}", first.value) };
        assert!(text.starts_with("push"), "{text}");
        // Nothing after the code, so neither of the two runs after it is there.
        assert!(ev.node(&d, &[2, 2]).unwrap().absent);
        assert!(ev.node(&d, &[2, 3]).unwrap().absent);
    }

    #[test]
    fn the_code_stops_at_the_first_erased_sector() {
        let mut v = image(&[]);
        // Two bytes of padding that are not the end, a sector and a half of
        // erased flash, and then something programmed after it.
        v.extend_from_slice(&[0xff, 0xff, 0xf8, 0xb5]);
        let erased_at = v.len();
        v.resize(erased_at + ERASED_SECTOR * 3 / 2, 0xff);
        v.extend_from_slice(b"settings");
        let d = Document::new(MemSource(v));
        let mut ev = Evaluator::new(imxrt());
        let code = ev.node(&d, &[2, 1]).unwrap();
        assert_eq!((code.offset_bits / 8 + code.size_bits / 8) as usize, erased_at);
        let erased = ev.node(&d, &[2, 2]).unwrap();
        assert_eq!((erased.offset_bits / 8, erased.size_bits / 8), (erased_at as u64, (ERASED_SECTOR * 3 / 2) as u64));
        assert_eq!(ev.node(&d, &[2, 3]).unwrap().size_bits, 8 * 8);
    }
}
