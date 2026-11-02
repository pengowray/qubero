//! The two Tascam Portacapture X8 firmware updates in the sample collection,
//! V2.01 and V2.03, read against what is known to be in them.
//!
//! The values asserted here were worked out by hand from the unscrambled
//! images. Apart from the boot header at 0 and the application at 0x20000,
//! which are fixed, the template finds every position itself: the init tables
//! by searching for IAR's FPU routine, the fonts by the offsets in the font
//! map, and the packed runs by the pointers in the init tables. The numbers
//! below differ between the two releases wherever the application's code
//! grew. The instruction counts were not worked out by hand. They are the
//! counts the template gives, and each was checked against counting the same
//! run one placed element at a time, the way runs of instructions were
//! counted before `insnrun.rs`.
//!
//! The files live in the sample collection rather than here. Point
//! `QUBERO_SAMPLES` at it, or keep it beside the repository as
//! `qubero-samples`. With neither, the test says so and passes.

use std::path::PathBuf;

use qubero_core::document::Document;
use qubero_core::eval::{Evaluator, Value};
use qubero_core::formats;
use qubero_core::source::MemSource;

/// What differs between the two releases.
struct Release {
    file: &'static str,
    version: i128,
    build: i128,
    date: (i128, i128, i128),
    app_reset: i128,
    /// Where the application's init table starts, which is where its code
    /// run ends.
    app_init_table: u64,
    /// The three lz77 runs: where each starts, how long it is, and what it
    /// unpacks to.
    runs: [(u64, u64, i128); 3],
    /// How many Thumb instructions the application's code runs hold, before
    /// the init table and after it.
    instructions: (u64, u64),
}

const RELEASES: &[Release] = &[
    Release {
        file: "#PortaX8_0215.201",
        version: 201,
        build: 215,
        date: (2024, 10, 2),
        app_reset: 0x8173_1c9d,
        app_init_table: 0x17a_43dc,
        runs: [(0x182_6fcc, 0x5c69, 0xa_3486), (0x182_cc35, 0x2_27a1, 0x3_1692), (0x184_f3d6, 0x4e3, 0x416c)],
        instructions: (4_174_301, 253_108),
    },
    Release {
        file: "#PortaX8_0217.203",
        version: 203,
        build: 217,
        date: (2025, 8, 19),
        app_reset: 0x8173_206d,
        app_init_table: 0x17a_47ac,
        runs: [(0x182_739c, 0x5c5f, 0xa_3482), (0x182_cffb, 0x2_279c, 0x3_1692), (0x184_f797, 0x4e3, 0x416c)],
        instructions: (4_173_515, 253_158),
    },
];

/// Every update file is this long. The flash image is the file less the
/// 32-byte header and the 4-byte checksum copy at the end.
const FILE_SIZE: u64 = 0x185_0020;
const IMAGE_SIZE: u64 = 0x184_fffc;

/// The flash is mapped here, which is what the IVT's pointers count in.
const XIP: i128 = 0x6000_0000;

fn sample(name: &str) -> Option<PathBuf> {
    qubero_samples::roots().into_iter().map(|r| r.join("firmware").join("tascam").join(name)).find(|p| p.exists())
}

/// The node a path of names reaches. `0` steps into what a packed run
/// unpacks to, which has no name of its own to be found by.
fn named(ev: &mut Evaluator, d: &Document<MemSource>, path: &[usize], names: &[&str]) -> Vec<usize> {
    let mut p = path.to_vec();
    for name in names {
        if *name == "0" {
            p.push(0);
            continue;
        }
        p = ev.child_named(d, &p, name).unwrap().unwrap_or_else(|| panic!("no {name} under {p:?}"));
    }
    p
}

fn int(ev: &mut Evaluator, d: &Document<MemSource>, path: &[usize], names: &[&str]) -> i128 {
    let p = named(ev, d, path, names);
    let v = ev.node(d, &p).unwrap().value;
    v.as_int().unwrap_or_else(|| panic!("{names:?} is not a number: {v:?}"))
}

/// Offset and size in bytes, in whichever space the node is read in.
fn extent(ev: &mut Evaluator, d: &Document<MemSource>, path: &[usize], names: &[&str]) -> (u64, u64) {
    let p = named(ev, d, path, names);
    let n = ev.node(d, &p).unwrap();
    (n.offset_bits / 8, n.size_bits / 8)
}

/// Whether the constraint on a field holds, failing if it has none.
fn holds(ev: &mut Evaluator, d: &Document<MemSource>, path: &[usize], names: &[&str]) -> bool {
    let p = named(ev, d, path, names);
    let verdict = ev.valid_of(d, &p).unwrap().unwrap_or_else(|| panic!("{names:?} has no constraint"));
    assert!(verdict.ok || !verdict.text.is_empty());
    verdict.ok
}

/// How many children the node a path of names reaches has.
fn count(ev: &mut Evaluator, d: &Document<MemSource>, path: &[usize], names: &[&str]) -> u64 {
    let p = named(ev, d, path, names);
    ev.node(d, &p).unwrap().child_count
}

fn text(ev: &mut Evaluator, d: &Document<MemSource>, path: &[usize]) -> String {
    match ev.node(d, path).unwrap().value {
        Value::Str(s) => s,
        other => panic!("not text at {path:?}: {other:?}"),
    }
}

/// The unscrambled flash: the image run, and the one thing it unpacks to.
const FLASH: [usize; 2] = [1, 0];

#[test]
fn both_releases_read_as_x8_updates() {
    let mut read = 0;
    for r in RELEASES {
        let Some(path) = sample(r.file) else { continue };
        read += 1;
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(bytes.len() as u64, FILE_SIZE);
        let head = &bytes[..bytes.len().min(formats::SNIFF_WINDOW)];
        assert_eq!(formats::sniff(head, FILE_SIZE), Some("tascamfw"), "{}", r.file);
        let d = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(formats::template("tascamfw").expect("the tascamfw template"));
        let ev = &mut ev;
        let d = &d;
        let f = r.file;

        // The header, and the whole file read with nothing left over.
        assert_eq!(ev.node(d, &[]).unwrap().size_bits, FILE_SIZE * 8);
        assert_eq!(int(ev, d, &[0], &["version"]), r.version, "{f}");
        assert_eq!(int(ev, d, &[0], &["build"]), r.build, "{f}");
        assert_eq!((int(ev, d, &[0], &["year"]), int(ev, d, &[0], &["month"]), int(ev, d, &[0], &["day"])), r.date, "{f}");
        assert_eq!(int(ev, d, &[0], &["file_size"]), FILE_SIZE as i128);
        assert!(holds(ev, d, &[0], &["file_size"]), "{f}");
        let sum = named(ev, d, &[0], &["checksum"]);
        let verdict = ev.run_check(d, &sum).unwrap().expect("the header's checksum is a check");
        assert!(verdict.ok, "{f}: computed {}, stored {}", verdict.computed, verdict.stored);
        assert_eq!(ev.node(d, &FLASH).unwrap().size_bits, IMAGE_SIZE * 8);
        assert!(holds(ev, d, &[2, 0], &["checksum"]), "{f}: the image's last word is the header's checksum");

        boot_header(ev, d, f);
        bootloader(ev, d, f);
        application(ev, d, r);
    }
    if read == 0 {
        eprintln!("{}", qubero_samples::missing());
    }
}

/// The configuration block, the IVT, the boot data and the DCD, which are the
/// same in both releases.
fn boot_header(ev: &mut Evaluator, d: &Document<MemSource>, f: &str) {
    let header = named(ev, d, &FLASH, &["boot_header"]);
    let config = named(ev, d, &header, &["flash_config", "mem_config"]);
    let tag = named(ev, d, &config, &["tag"]);
    assert!(matches!(ev.node(d, &tag).unwrap().value, Value::Magic { ok: true, .. }), "{f}: FCFB");
    assert_eq!(int(ev, d, &config, &["version", "major"]), 1);
    assert_eq!(int(ev, d, &config, &["version", "minor"]), 4);
    assert_eq!(int(ev, d, &config, &["sflash_pad_type"]), 4);
    assert_eq!(int(ev, d, &config, &["serial_clk_freq"]), 5);
    assert_eq!(int(ev, d, &config, &["sflash_a1_size"]), 0x0200_0000);

    // Sequence 0 is the boot ROM's read: command 0xEC on one line, a 32-bit
    // address on four, six dummy cycles, the data on four, then stops.
    let read = named(ev, d, &config, &["lookup_table", "read"]);
    let want = [(0x01, 0, 0xec), (0x02, 2, 0x20), (0x0c, 2, 0x06), (0x09, 2, 0x04), (0x00, 0, 0x00)];
    for (i, (opcode, pads, operand)) in want.into_iter().enumerate() {
        let insn = [&read[..], &[i]].concat();
        let got = (int(ev, d, &insn, &["opcode"]), int(ev, d, &insn, &["num_pads"]), int(ev, d, &insn, &["operand"]));
        assert_eq!(got, (opcode, pads, operand), "{f}: read instruction {i}");
    }

    let ivt = named(ev, d, &header, &["ivt"]);
    assert_eq!(extent(ev, d, &ivt, &[]), (0x1000, 0x20));
    assert_eq!(int(ev, d, &ivt, &["hdr", "length"]), 0x20);
    assert_eq!(int(ev, d, &ivt, &["hdr", "version"]), 0x41);
    assert_eq!(int(ev, d, &ivt, &["entry"]), XIP + 0x2000);
    assert_eq!(int(ev, d, &ivt, &["dcd"]), XIP + 0x1030);
    assert_eq!(int(ev, d, &ivt, &["boot_data"]), XIP + 0x1020);
    assert_eq!(int(ev, d, &ivt, &["self"]), XIP + 0x1000);
    assert_eq!(int(ev, d, &ivt, &["csf"]), 0);
    assert!(holds(ev, d, &header, &["boot_data"]), "{f}: the boot data is where the IVT points");
    assert!(holds(ev, d, &header, &["dcd"]), "{f}: the DCD is where the IVT points");
    assert_eq!(int(ev, d, &header, &["boot_data", "start"]), XIP);
    assert_eq!(int(ev, d, &header, &["boot_data", "size"]), 0x0200_0000);

    // Read by the lengths in their headers, the DCD has nine commands. The
    // ninth command's length covers one address-value pair, but a second pair
    // follows it, so the next byte the boot ROM would read as a tag is 0x40,
    // which is not a command tag. The rest of the DCD is one bytes field.
    let dcd = named(ev, d, &header, &["dcd"]);
    assert_eq!(extent(ev, d, &dcd, &[]), (0x1030, 0x4b8));
    let commands = named(ev, d, &dcd, &["commands"]);
    assert_eq!(ev.node(d, &commands).unwrap().child_count, 9, "{f}");
    let first = [&commands[..], &[0]].concat();
    assert_eq!(int(ev, d, &first, &["tag"]), 0xcc);
    assert_eq!(count(ev, d, &first, &["body"]), 117);
    let ninth = [&commands[..], &[8]].concat();
    assert_eq!(extent(ev, d, &ninth, &[]), (0x1454, 0xc));
    let check = [&commands[..], &[1]].concat();
    assert_eq!(int(ev, d, &check, &["tag"]), 0xcf);
    assert_eq!(int(ev, d, &check, &["parameter", "flags"]), 3, "{f}: poll until any bit is set");
    let stray = named(ev, d, &dcd, &["rest_from_unknown_tag"]);
    assert_eq!(extent(ev, d, &stray, &[]), (0x1460, 0x88), "{f}");
    let Value::Bytes { preview, .. } = ev.node(d, &stray).unwrap().value else { panic!("{f}: not bytes") };
    assert_eq!(preview[0], 0x40);

    // Every address in the nine commands is named for the RT1060 register it
    // writes or polls, and none is marked. The extra pair at 0x1460 is in the
    // bytes after the ninth command, so it is not read as an address.
    let mut writes = Vec::new();
    let mut checks = Vec::new();
    for i in 0..9 {
        let command = [&commands[..], &[i]].concat();
        let body = named(ev, d, &command, &["body"]);
        match int(ev, d, &command, &["tag"]) {
            0xcc => {
                for j in 0..ev.node(d, &body).unwrap().child_count as usize {
                    writes.push(register(ev, d, &[&body[..], &[j]].concat(), f));
                }
            }
            0xcf => checks.push(register(ev, d, &body, f)),
            tag => panic!("{f}: command {i} has tag {tag:#x}"),
        }
    }
    assert_eq!((writes.len(), checks.len()), (125, 4), "{f}");
    assert_eq!(writes[0], "CCM_CCGR0");
    assert_eq!(writes[7], "CCM_ANALOG_PLL_SYS");
    assert_eq!(writes[9], "CCM_CBCDR");
    assert_eq!(writes[10], "IOMUXC_SW_MUX_CTL_PAD_GPIO_EMC_00");
    assert_eq!(writes[93], "IOMUXC_SW_PAD_CTL_PAD_GPIO_EMC_41");
    assert_eq!(writes[94], "SEMC_MCR");
    assert_eq!(writes[124], "SEMC_IPCR0");
    assert!(checks.iter().all(|c| c == "SEMC_INTR"), "{f}: {checks:?}");

    assert_eq!(extent(ev, d, &FLASH, &["padding"]), (0x14e8, 0x2000 - 0x14e8));
}

/// The register a DCD write or check at `pair` names by its address, which
/// must be one of the registers in the table rather than a peripheral or an
/// area, and must not be marked.
fn register(ev: &mut Evaluator, d: &Document<MemSource>, pair: &[usize], f: &str) -> String {
    let at = named(ev, d, pair, &["address"]);
    let node = ev.node(d, &at).unwrap();
    assert_eq!(node.problem, None, "{f}: {pair:?}");
    let Value::Enum { raw, name: Some(name), .. } = node.value else { panic!("{f}: {pair:?} is {:?}", node.value) };
    assert!(name.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'), "{f}: {raw:#x} is {name}");
    name
}

/// The bootloader at the IVT's entry, which is the same in both releases.
fn bootloader(ev: &mut Evaluator, d: &Document<MemSource>, f: &str) {
    let boot = named(ev, d, &FLASH, &["bootloader"]);
    assert_eq!(extent(ev, d, &boot, &[]).0, 0x2000);
    assert_eq!(int(ev, d, &boot, &["vectors", "stack_top"]), 0x2008_0000);
    assert_eq!(int(ev, d, &boot, &["vectors", "reset"]), XIP + 0xa0d5, "{f}");
    assert_eq!(int(ev, d, &boot, &["vectors", "irq_239"]), 0xffff_ffff);
    assert_eq!(extent(ev, d, &boot, &["code"]), (0x2400, 0xa180 - 0x2400), "{f}: code up to the init table");
    let table = named(ev, d, &boot, &["init_table"]);
    assert_eq!(extent(ev, d, &table, &[]), (0xa180, 0x34));
    let zeroed = named(ev, d, &table, &["zero_init", "ranges"]);
    assert_eq!(ev.node(d, &zeroed).unwrap().child_count, 2);
    assert_eq!(extent(ev, d, &boot, &["code_after_init_table"]), (0xa1b4, 0xa3f4 - 0xa1b4));
    assert_eq!(extent(ev, d, &boot, &["dtcm_data"]), (0xa3f4, 0x44a), "{f}: the packbits run is where its entry points");
    assert_eq!(int(ev, d, &boot, &["dtcm_data", "0", "unpacked_length"]), 0x5144);
    assert!(holds(ev, d, &boot, &["dtcm_data", "0", "unpacked_length"]), "{f}");
    assert_eq!(extent(ev, d, &boot, &["padding"]), (0xa83e, 0x2_0000 - 0xa83e));
}

/// The application at 0x20000. Offsets inside it differ between the releases.
fn application(ev: &mut Evaluator, d: &Document<MemSource>, r: &Release) {
    let f = r.file;
    let app = named(ev, d, &FLASH, &["application"]);
    assert_eq!(extent(ev, d, &app, &[]).0, 0x2_0000);
    assert_eq!(int(ev, d, &app, &["vectors", "stack_top"]), 0x2001_1cc8);
    assert_eq!(int(ev, d, &app, &["vectors", "reset"]), r.app_reset, "{f}");
    // The RTOS's two handlers run from the ITCM.
    assert_eq!(int(ev, d, &app, &["vectors", "pendsv"]), 0x0003_023d);
    assert_eq!(int(ev, d, &app, &["vectors", "systick"]), 0x0003_0f13);
    assert_eq!(int(ev, d, &app, &["vectors", "irq_239"]), 0xffff_ffff);
    assert_eq!(int(ev, d, &app, &["version_stamp", "version"]), r.version);
    assert_eq!(int(ev, d, &app, &["version_stamp", "build"]), r.build);
    assert!(holds(ev, d, &app, &["version_stamp", "version"]), "{f}");
    assert!(holds(ev, d, &app, &["version_stamp", "build"]), "{f}");

    // Three fonts, each where the font map's offsets put it and as long as it
    // says. The first font's length takes in the two bytes of padding before
    // the second, so nothing lies between them.
    let map = named(ev, d, &app, &["font_map"]);
    assert_eq!(extent(ev, d, &map, &[]), (0x2_0410, 0xbb_ef84));
    let name = named(ev, d, &map, &["name"]);
    assert_eq!(text(ev, d, &name), "Fontmap1");
    assert_eq!(count(ev, d, &map, &["ranges"]), 19);
    let fonts = named(ev, d, &map, &["fonts"]);
    assert_eq!(ev.node(d, &fonts).unwrap().child_count, 3);
    let want = [(0x25c, 0x4b_5670), (0x4b_58cc, 0x2e_003c), (0x79_5908, 0x42_967c)];
    for (i, (offset, size)) in want.into_iter().enumerate() {
        let font = [&fonts[..], &[i]].concat();
        assert_eq!(extent(ev, d, &font, &["padding"]).1, 0, "{f}: font {i}");
        let file = named(ev, d, &font, &["file"]);
        assert_eq!(extent(ev, d, &file, &[]), (0x2_0410 + offset, size), "{f}: font {i}");
        assert!(ev.node(d, &file).unwrap().decoded, "{f}: font {i} opens as a file");
        let Value::Bytes { preview, .. } = ev.node(d, &[&file[..], &[0, 0]].concat()).unwrap().value else { panic!("{f}: font {i}") };
        assert_eq!(preview[..4], [0, 1, 0, 0], "{f}: font {i} is TrueType");
    }

    // Four WAVs straight after the map, each padded to four bytes.
    let sounds = named(ev, d, &app, &["sounds"]);
    assert_eq!(ev.node(d, &sounds).unwrap().child_count, 4, "{f}");
    let want = [(0xbd_f394, 0x6_82ac, 0), (0xc4_7640, 0x5_e4cc, 0), (0xca_5b0c, 0x5_53ce, 2), (0xcf_aedc, 0x3_a92e, 2)];
    for (i, (at, size, padding)) in want.into_iter().enumerate() {
        let sound = [&sounds[..], &[i]].concat();
        assert_eq!(extent(ev, d, &sound, &["wav"]), (at, size), "{f}: sound {i}");
        assert_eq!(extent(ev, d, &sound, &["padding"]).1, padding, "{f}: sound {i}");
        let object = named(ev, d, &sound, &["wav", "0", "object"]);
        let Value::Bytes { preview, .. } = ev.node(d, &object).unwrap().value else { panic!("{f}") };
        assert_eq!(&preview[..4], b"RIFF");
        assert_eq!(&preview[8..12], b"WAVE");
    }

    // The code runs from the end of the WAVs to the end of IAR's FPU
    // routine, which is where the init table starts. It is read as Thumb, and
    // its last three instructions are the routine's: it sets the FPSCR,
    // returns, and is padded to a word with two zero bytes.
    assert_eq!(extent(ev, d, &app, &["code"]), (0xd3_580c, r.app_init_table - 0xd3_580c), "{f}");
    let code = named(ev, d, &app, &["code"]);
    let n = count(ev, d, &app, &["code"]);
    assert_eq!(n, r.instructions.0, "{f}");
    assert_eq!(text(ev, d, &[&code[..], &[0]].concat()), "push {r4, r5, lr}", "{f}");
    let end = [(n - 3, 8, "vmsr fpscr, r0"), (n - 2, 4, "bx lr"), (n - 1, 2, "lsls r0, r0, 0x0")];
    for (k, back, said) in end {
        let insn = [&code[..], &[k as usize]].concat();
        assert_eq!(ev.node(d, &insn).unwrap().offset_bits / 8, r.app_init_table - back, "{f}: instruction {k}");
        assert_eq!(text(ev, d, &insn), said, "{f}: instruction {k}");
    }
    let table = named(ev, d, &app, &["init_table"]);
    assert_eq!(extent(ev, d, &table, &[]), (r.app_init_table, 0x98), "{f}");
    let zeroed = named(ev, d, &table, &["zero_init", "ranges"]);
    assert_eq!(ev.node(d, &zeroed).unwrap().child_count, 7);
    // IAR's startup code and the interrupt stubs, up to the first lz77 run.
    // The last instruction is a four-byte branch that ends where the run
    // starts.
    let after = named(ev, d, &app, &["code_after_init_table"]);
    let n = count(ev, d, &app, &["code_after_init_table"]);
    assert_eq!(n, r.instructions.1, "{f}");
    let last = [&after[..], &[n as usize - 1]].concat();
    assert_eq!(extent(ev, d, &last, &[]), (r.runs[0].0 - 4, 4), "{f}");
    assert!(text(ev, d, &last).starts_with("b.w "), "{f}");

    // The three lz77 runs, each where its entry points, as long as its entry
    // says, and unpacking to the length its entry gives.
    let entries = ["lz77_init_sdram", "lz77_init_itcm", "lz77_init_dtcm"];
    let runs = ["sdram_data", "itcm_code", "dtcm_data"];
    let destinations = [0x8190_0000, 0x400, 0x2000_0000];
    for (i, ((entry, run), (at, len, unpacked))) in entries.iter().zip(runs).zip(r.runs).enumerate() {
        assert_eq!(int(ev, d, &table, &[*entry, "sources", "0", "length"]), len as i128, "{f}: {entry}");
        assert_eq!(int(ev, d, &table, &[*entry, "destinations", "0", "address"]), destinations[i]);
        assert_eq!(int(ev, d, &table, &[*entry, "destinations", "0", "length"]), unpacked, "{f}: {entry}");
        assert_eq!(extent(ev, d, &app, &[run]), (at, len), "{f}: {run}");
        assert_eq!(int(ev, d, &app, &[run, "0", "unpacked_length"]), unpacked, "{f}: {run}");
        assert!(holds(ev, d, &app, &[run, "0", "unpacked_length"]), "{f}: {run}");
    }
    assert!(holds(ev, d, &app, &["itcm_code"]), "{f}");
    assert!(holds(ev, d, &app, &["dtcm_data"]), "{f}");
    let (at, len, _) = r.runs[2];
    assert_eq!(extent(ev, d, &app, &["padding"]), (at + len, IMAGE_SIZE - at - len), "{f}: erased flash to the end");

    // The ITCM code opens with a veneer, `ldr.w pc, [pc]` and the address it
    // loads, and then a function.
    let code = named(ev, d, &app, &["itcm_code", "0", "code"]);
    assert_eq!(text(ev, d, &[&code[..], &[0]].concat()), "ldr.w pc, [pc]", "{f}");
    let fourth = [&code[..], &[3]].concat();
    assert_eq!(ev.node(d, &fourth).unwrap().offset_bits / 8, 8);
    assert_eq!(text(ev, d, &fourth), "push {r4, r5, r6, r7, r8, lr}", "{f}");
}
