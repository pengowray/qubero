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
//! The image vector table (IVT) is the boot ROM's index to the rest. Every
//! pointer in it is an address in the processor's memory map, and the IVT
//! gives its own address as well, so a pointer's offset in the flash is the
//! pointer less the IVT's own address, plus 0x1000. It points at the boot
//! data, which says where the image starts and how big it is, at the device
//! configuration data (DCD), and at the program's entry point.
//!
//! The DCD is a short list of register writes and polls that the boot ROM runs
//! before the program: NXP's own boards use it to bring up the SDRAM
//! controller. It is big-endian, a header and then commands, each with its own
//! tag and length, and the boot ROM steps from one command to the next by that
//! length. So does this template. A tag that is not one of the four commands
//! the boot ROM knows ends the commands, and what is left of the DCD, up to the
//! length its header gives, is one run of bytes: NXP does not say what the ROM
//! does with a tag it does not know.
//!
//! The boot data and the DCD are read straight after the IVT, one after the
//! other, which is where NXP's SDK puts them, and each is checked against the
//! IVT's pointer to it. If a file keeps them somewhere else, they are read
//! from the wrong place and the check on each of them fails.
//!
//! The IVT's entry points at the program, and NXP's startup code puts the
//! Cortex-M7's vector table there: the stack pointer, the fifteen exceptions
//! every Cortex-M has, then the RT1060's interrupts as NXP's `MIMXRT1062.h`
//! names them, filled out to 256 words with interrupt numbers the part does
//! not use. What follows the table is read as Thumb code, the way `hackrffw`
//! reads a HackRF image: nothing in the flash says where the code stops and
//! the data starts.
//!
//! The code stops at the first 4 KiB of erased flash (0xff bytes), which is
//! the smallest piece a NOR chip erases, and everything from there on is
//! bytes: the erased run, and whatever is programmed after it. A file written
//! by NXP's tools holds only the program, so this changes nothing for one of
//! those. It is for a dump of the whole chip, which is mostly erased bytes
//! after the program and may hold a second program further on, as the Tascam
//! X8's flash does. Read as instructions to the end, the X8's 25 MB took
//! nearly three minutes to count in a debug build, and a listing counts the
//! instructions in a run before it shows the run's row.
//!
//! Field names are NXP's own, put into snake case: the configuration block's
//! from the SDK's `fsl_flexspi_nor_config.h`, the IVT's and the boot data's
//! from `fsl_flexspi_nor_boot.h`, and the DCD's from the reference manual.

use crate::code::Isa;
use crate::template::{Endian::*, Expr as E, Template, Ty as T, Until, Valid};

use super::hackrffw::EXCEPTIONS;

/// What the configuration block opens with, and the first half of what says a
/// file is one of these.
pub const TAG: &[u8] = b"FCFB";

/// Where the boot ROM looks for the IVT when it boots from FlexSPI NOR: fixed
/// by the ROM, not by the image.
pub const IVT_AT: usize = 0x1000;

/// Bytes the configuration block takes, `sizeof(flexspi_nor_config_t)`.
const CONFIG_BYTES: i128 = 0x200;

/// Words in the vector table NXP's startup code writes: the stack pointer,
/// fifteen exceptions and 240 interrupts.
const VECTORS: usize = 256;

/// Whether the head of a file is an RT1060 boot image: `FCFB` at the front and
/// an IVT header where the boot ROM looks for one. The IVT's header is its tag,
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
                ("image", image()),
            ],
        ),
    )
}

/// The smallest stretch a NOR flash erases, and so the shortest run of 0xff
/// that says the program has ended rather than paused for padding.
const ERASED_SECTOR: usize = 0x1000;

/// The program at the IVT's entry: the vector table, the code up to the
/// first erased sector, and what comes after that as bytes.
fn image() -> T {
    let more = || E::Remaining.greater_than(E::lit(0));
    T::structure(
        "Image",
        vec![
            ("vectors", vectors()),
            ("code", thumb(E::to_bytes(&[0xff; ERASED_SECTOR]))),
            ("erased", T::when(more(), T::bytes(E::run(&[0xff])))),
            // A second program, a file system, settings: nothing here says.
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
            // Erased flash: the SDK reserves the first 4 KiB for the block and
            // the boot ROM looks for the IVT at the end of it.
            ("padding", T::bytes(E::lit(IVT_AT as i128 - CONFIG_BYTES))),
            ("ivt", ivt()),
            ("boot_data", boot_data()),
            // An image with nothing to set up before it runs has no DCD and
            // says so with a pointer of zero.
            ("dcd", T::when(E::within(&["ivt", "dcd"]).not_equal(E::lit(0)), dcd())),
        ],
    )
    .field_valid("boot_data", Valid::Expr { expr: E::start_of(E::field("boot_data")).equal_to(pointer(&["ivt"], "boot_data")), msg: None })
    .field_valid(
        "dcd",
        Valid::Expr {
            expr: E::within(&["ivt", "dcd"]).equal_to(E::lit(0)).either(E::start_of(E::field("dcd")).equal_to(pointer(&["ivt"], "dcd"))),
            msg: None,
        },
    )
}

/// What is left between the boot header and the program the IVT's entry
/// points at, for a structure that holds `boot_header` as a field. Erased
/// flash in every image NXP's tools write.
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
    // The slots past the last interrupt NXP names, which the startup code
    // fills with its default handler all the same.
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
            ("busy_bit_polarity", T::enumeration("BusyBitPolarity", T::u16(Little), &[(0, "1 means busy"), (1, "0 means busy")])),
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
    T::inline_structure("ConfigVersion", vec![("bugfix", T::u8()), ("minor", T::u8()), ("major", T::u8()), ("v", T::magic(b"V"))])
}

/// The clock the boot ROM drives the flash at, as the RT1060's
/// `kFlexSpiSerialClk_*` numbers it. Any other number is 30 MHz.
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

/// The sixteen sequences of the lookup table, by the index the boot ROM uses
/// each one for. The reference manual assigns nine of them and the SDK three
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

/// `struct ivt`: eight words, every one but the header an address in the
/// processor's memory map.
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
                // The rest of the DCD, from a tag that is not a command on.
                // Absent when every command was one the boot ROM knows.
                ("not_a_known_command", T::when(E::Remaining.greater_than(E::lit(0)), T::bytes(E::Remaining))),
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

/// One command: a header of its own, then whatever its tag says, up to the
/// length it gives, which counts the header.
fn dcd_command() -> T {
    let tag = || E::field("tag");
    T::structure_named(
        "DcdCommand",
        "tag",
        "body",
        vec![
            ("tag", T::enumeration_hex("DcdTag", T::u8(), DCD_TAGS)),
            ("length", T::u16(Big)),
            // How wide each write or check is and what it does. Unlock puts
            // the engine it unlocks here, and nop ignores it.
            ("parameter", T::switch(tag(), vec![(0xcc, write_parameter()), (0xcf, check_parameter())], T::u8())),
            (
                "body",
                T::sized(
                    E::field("length").sub(E::lit(4)),
                    T::switch(
                        tag(),
                        vec![
                            (0xcc, T::repeat(T::structure("WriteData", vec![("address", T::u32(Big)), ("value", T::u32(Big))]), Until::End)),
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
            ("flags", T::enumeration("WriteAction", T::UInt { bits: 5, endian: Big }, &[(0, "write value"), (1, "clear bitmask"), (2, "write value"), (3, "set bitmask")])),
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
    T::structure(
        "CheckData",
        vec![
            ("address", T::u32(Big)),
            ("mask", T::u32(Big)),
            ("count", T::when(E::Remaining.greater_than(E::lit(0)), T::u32(Big))),
        ],
    )
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
        let stray = named(&mut ev, &d, &dcd, "not_a_known_command");
        let stray = ev.node(&d, &stray).unwrap();
        assert_eq!((stray.offset_bits / 8, stray.size_bits / 8), (0x1054, 8));
        // Both pieces are where the IVT says.
        for name in ["boot_data", "dcd"] {
            let at = named(&mut ev, &d, &[0], name);
            let verdict = ev.valid_of(&d, &at).unwrap().expect("a constraint");
            assert!(verdict.ok, "{name}: {}", verdict.text);
        }
    }

    #[test]
    fn a_dcd_the_ivt_does_not_point_at_is_not_there() {
        let d = Document::new(MemSource(image(&[])));
        let mut ev = Evaluator::new(imxrt());
        let dcd = named(&mut ev, &d, &[0], "dcd");
        assert!(ev.node(&d, &dcd).unwrap().absent);
        // A pointer of zero is no DCD, which is no fault to report.
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
    }

    #[test]
    fn the_image_at_the_entry_is_a_vector_table_and_thumb_code() {
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
