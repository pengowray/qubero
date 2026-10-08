//! What a program was built with, read against `checksec`.
//!
//! `checksec` 2.6.0 is a shell script over `readelf`, and its CSV output over
//! the sample collection's ELF files is written down below. The pass is held
//! to it column by column, with one deliberate difference: a program with no
//! symbols at all cannot say whether it has a stack canary. `checksec` says
//! "No Canary found" for it, because its `grep` found nothing; this says
//! `no-symbols`. Most of the busybox samples are static and stripped and are
//! this case.
//!
//! The fortify counts here count each function once. `checksec` counts lines
//! of `readelf --dyn-syms`, so a function listed twice counts twice; where
//! the two differ, the host test below works out `checksec`'s number from
//! `readelf` itself and shows that duplicates are the whole of the
//! difference.
//!
//! The files live in the sample collection rather than here. Point
//! `QUBERO_SAMPLES` at it, or keep it beside the repository as
//! `qubero-samples`. With neither, each test says so and passes.

use std::path::Path;
use std::process::Command;

use qubero_core::document::Document;
use qubero_core::eval::Evaluator;
use qubero_core::formats::{self, hardening};
use qubero_core::source::MemSource;

fn read(path: &Path, template: &str) -> (hardening::Hardening, Document<MemSource>, Evaluator) {
    let doc = Document::new(MemSource(std::fs::read(path).unwrap()));
    let mut ev = Evaluator::new(formats::template(template).expect("a template"));
    let h = hardening::read(&mut ev, &doc).unwrap_or_else(|e| panic!("{}: {e:?}", path.display())).expect("a program");
    (h, doc, ev)
}

fn state<'a>(rows: &'a [hardening::Row], key: &str) -> &'a str {
    rows.iter().find(|r| r.key == key).map_or("", |r| r.state)
}

fn row<'a>(rows: &'a [hardening::Row], key: &str) -> &'a hardening::Row {
    rows.iter().find(|r| r.key == key).unwrap_or_else(|| panic!("no row {key}"))
}

/// The rows as `checksec --output=csv` writes them, up to the file name:
/// RELRO, canary, NX, PIE, RPATH, RUNPATH, symbols, FORTIFY, fortified,
/// fortifiable.
///
/// `no-symbols` is written as `checksec`'s "No Canary found", since that is
/// what it says for the same file; the expectations below say which files
/// those are.
fn checksec_csv(rows: &[hardening::Row]) -> String {
    let relro = match state(rows, "relro") {
        "full" => "Full RELRO",
        "partial" => "Partial RELRO",
        _ => "No RELRO",
    };
    let canary = if state(rows, "canary") == "found" { "Canary found" } else { "No Canary found" };
    let nx = if state(rows, "nx") == "on" { "NX enabled" } else { "NX disabled" };
    let pie = match state(rows, "pie") {
        "pie" => "PIE enabled",
        "dso" => "DSO",
        "exec" => "No PIE",
        "object" => "REL",
        other => other,
    };
    let rpath = if state(rows, "rpath") == "set" { "RPATH" } else { "No RPATH" };
    let runpath = if state(rows, "runpath") == "set" { "RUNPATH" } else { "No RUNPATH" };
    let symbols = if state(rows, "symbols") == "present" { "Symbols" } else { "No Symbols" };
    let fortify = row(rows, "fortify");
    let yes = if fortify.state == "yes" { "Yes" } else { "No" };
    format!(
        "{relro},{canary},{nx},{pie},{rpath},{runpath},{symbols},{yes},{},{}",
        fortify.count.unwrap_or(0),
        fortify.total.unwrap_or(0)
    )
}

/// Every piece of evidence with a path is inside the node it names.
fn evidence_is_where_it_says(name: &str, h: &hardening::Hardening, doc: &Document<MemSource>, ev: &mut Evaluator) {
    for part in &h.parts {
        for r in &part.rows {
            assert!(r.evidence.len() <= hardening::EVIDENCE, "{name} {}", r.key);
            for e in r.evidence.iter().filter(|e| !e.path.is_empty()) {
                let n = ev.node(doc, &e.path).unwrap_or_else(|err| panic!("{name} {} {e:?}: {err:?}", r.key));
                assert!(
                    n.offset_bits <= e.offset_bits && e.offset_bits + e.size_bits <= n.offset_bits + n.size_bits,
                    "{name} {}: {e:?} is not inside {} at {}+{}",
                    r.key,
                    n.name,
                    n.offset_bits,
                    n.size_bits
                );
            }
        }
    }
}

/// `checksec --output=csv` over the collection, and the canary state this
/// gives where `checksec` can only say it found nothing.
const ORACLE: &[(&str, &str, &str)] = &[
    // A dynamic musl program, built position-independent.
    ("busybox-aarch64", "Full RELRO,Canary found,NX enabled,PIE enabled,No RPATH,No RUNPATH,No Symbols,No,0,38", "found"),
    ("busybox-armv7l", "No RELRO,No Canary found,NX enabled,No PIE,No RPATH,No RUNPATH,No Symbols,No,0,0", "no-symbols"),
    ("busybox-i686", "Partial RELRO,No Canary found,NX enabled,No PIE,No RPATH,No RUNPATH,No Symbols,No,0,0", "no-symbols"),
    ("busybox-mips", "No RELRO,No Canary found,NX disabled,No PIE,No RPATH,No RUNPATH,No Symbols,No,0,0", "no-symbols"),
    ("busybox-powerpc", "Partial RELRO,No Canary found,NX enabled,No PIE,No RPATH,No RUNPATH,No Symbols,No,0,0", "no-symbols"),
    ("busybox-powerpc64", "Partial RELRO,No Canary found,NX enabled,No PIE,No RPATH,No RUNPATH,No Symbols,No,0,0", "no-symbols"),
    ("busybox-riscv64", "Full RELRO,Canary found,NX enabled,PIE enabled,No RPATH,No RUNPATH,No Symbols,No,0,38", "found"),
    ("busybox-s390x", "Partial RELRO,No Canary found,NX enabled,No PIE,No RPATH,No RUNPATH,No Symbols,No,0,0", "no-symbols"),
    ("busybox-x86_64", "Partial RELRO,No Canary found,NX enabled,No PIE,No RPATH,No RUNPATH,No Symbols,No,0,0", "no-symbols"),
    // `checksec` says 76 fortifiable: musl exports `longjmp` under two names,
    // `_longjmp` and `longjmp`, which are one function once the underscore is
    // off, and `checksec` counts the two lines. See the test below.
    ("musl-libc-aarch64.so", "Full RELRO,Canary found,NX enabled,DSO,No RPATH,No RUNPATH,No Symbols,No,0,75", "found"),
];

#[test]
fn the_elf_samples_read_as_checksec_reads_them() {
    let Some(dir) = qubero_samples::dir("elf") else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    let mut read_any = false;
    for &(name, csv, canary) in ORACLE {
        let path = dir.join(name);
        if !path.exists() {
            continue;
        }
        read_any = true;
        let (h, doc, mut ev) = read(&path, "elf");
        assert_eq!(h.format, "elf");
        let rows = &h.parts[0].rows;
        assert_eq!(checksec_csv(rows), csv, "{name}: {rows:#?}");
        assert_eq!(state(rows, "canary"), canary, "{name}");
        evidence_is_where_it_says(name, &h, &doc, &mut ev);
    }
    assert!(read_any, "{}", qubero_samples::missing());
}

/// The one fortify count that differs from `checksec`'s, worked out from
/// `readelf` both ways: 76 lines, 75 functions. Runs only where `readelf` and
/// a glibc to read the `_chk` list from are installed.
#[test]
fn musl_lists_one_function_twice_and_checksec_counts_it_twice() {
    let Some(path) = qubero_samples::dir("elf").map(|d| d.join("musl-libc-aarch64.so")).filter(|p| p.exists()) else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    let chk = libc_chk();
    if chk.is_empty() {
        eprintln!("skipped: needs readelf and a glibc");
        return;
    }
    let r = readelf_fortify(&path, &chk);
    assert_eq!((r.lines, r.names), ((0, 76), (0, 75)));
    let (h, _, _) = read(&path, "elf");
    let fortify = row(&h.parts[0].rows, "fortify");
    assert_eq!((fortify.count, fortify.total), (Some(0), Some(75)));
    assert!(fortify.more.iter().any(|n| n == "longjmp" || n == "_longjmp"));
}

/// What the dynamic musl program says beyond `checksec`'s columns, with the
/// evidence a reader would be taken to.
#[test]
fn the_dynamic_busybox_names_its_library_and_its_build() {
    let Some(path) = qubero_samples::dir("elf").map(|d| d.join("busybox-aarch64")).filter(|p| p.exists()) else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    let (h, _, _) = read(&path, "elf");
    let rows = &h.parts[0].rows;
    assert_eq!(row(rows, "needed").items, ["libc.musl-aarch64.so.1"]);
    assert_eq!(row(rows, "interpreter").items, ["/lib/ld-musl-aarch64.so.1"]);
    assert_eq!(state(rows, "linking"), "dynamic");
    assert_eq!(state(rows, "build-id"), "set");
    // Full RELRO from FLAGS here, where the RISC-V build writes a BIND_NOW
    // entry: both are what `checksec`'s grep for the words finds.
    let relro: Vec<&str> = row(rows, "relro").evidence.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(relro, ["GNU_RELRO", "FLAGS", "FLAGS_1"]);
    // AArch64, so the two AArch64 rows and not the x86 ones.
    assert!(rows.iter().any(|r| r.key == "bti") && !rows.iter().any(|r| r.key == "ibt"));
}

/// The three Windows builds of busybox, against their headers read by hand
/// (a few lines of Python over `struct.unpack`, since nothing like `checksec`
/// for PE is to hand). The i686 build was linked with its relocations taken
/// out and without the dynamic base flag; the other two are relocatable, and
/// only the ARM64 one has a load configuration, whose cookie is zero because
/// llvm-mingw does not use it.
#[test]
fn the_windows_busyboxes_read_as_their_headers_say() {
    let Some(dir) = qubero_samples::dir("pe") else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    let expected: &[(&str, &[(&str, &str)], Option<u64>)] = &[
        (
            "busybox-w32-aarch64.exe",
            &[
                ("aslr", "on"),
                ("high-entropy-va", "on"),
                ("dep", "on"),
                ("cfg", "off"),
                ("gs", "not-found"),
                ("safeseh", "n/a"),
                ("signature", "none"),
                ("force-integrity", "off"),
                ("appcontainer", "off"),
            ],
            Some(0),
        ),
        (
            "busybox-w32-i686.exe",
            &[
                ("aslr", "off"),
                ("high-entropy-va", "n/a"),
                ("dep", "on"),
                ("cfg", "off"),
                ("gs", "unknown"),
                ("safeseh", "off"),
                ("signature", "none"),
                ("force-integrity", "off"),
                ("appcontainer", "off"),
            ],
            None,
        ),
        (
            "busybox-w32-x86_64.exe",
            &[
                ("aslr", "on"),
                ("high-entropy-va", "on"),
                ("dep", "on"),
                ("cfg", "off"),
                ("gs", "unknown"),
                ("safeseh", "n/a"),
                ("signature", "none"),
                ("force-integrity", "off"),
                ("appcontainer", "off"),
            ],
            None,
        ),
    ];
    let mut read_any = false;
    for &(name, states, guard_count) in expected {
        let path = dir.join(name);
        if !path.exists() {
            continue;
        }
        read_any = true;
        let (h, doc, mut ev) = read(&path, "pe");
        assert_eq!(h.format, "pe");
        let rows = &h.parts[0].rows;
        let got: Vec<(&str, &str)> = rows.iter().map(|r| (r.key, r.state)).collect();
        assert_eq!(got, states, "{name}");
        assert_eq!(row(rows, "cfg").count, guard_count, "{name}");
        evidence_is_where_it_says(name, &h, &doc, &mut ev);
    }
    assert!(read_any, "{}", qubero_samples::missing());
}

/// The two macOS builds of ripgrep, against their load commands and symbol
/// tables read by hand in Python. Both are position-independent executables
/// with the stack not executable and the heap left executable, which is the
/// linker's default. Only the x86-64 one calls the canary and two checked
/// copies, from the C library it links; only the ARM64 one is signed, since
/// macOS will not run an unsigned ARM64 program, and it is plain arm64, not
/// the arm64e built for pointer authentication.
#[test]
fn the_macos_ripgreps_read_as_their_load_commands_say() {
    let Some(dir) = qubero_samples::dir("macho") else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    let expected: &[(&str, &[(&str, &str)], (u64, u64), &[&str])] = &[
        (
            "ripgrep-aarch64",
            &[
                ("pie", "on"),
                ("nx-heap", "off"),
                ("nx-stack", "on"),
                ("code-signature", "present"),
                ("encrypted", "none"),
                ("canary", "not-found"),
                ("fortify", "no"),
                ("arc", "not-found"),
                ("restrict", "none"),
                ("pac", "off"),
                ("rpath", "none"),
                ("needed", "set"),
            ],
            (0, 6),
            &["/opt/homebrew/opt/pcre2/lib/libpcre2-8.0.dylib", "/usr/lib/libiconv.2.dylib", "/usr/lib/libSystem.B.dylib"],
        ),
        (
            "ripgrep-x86_64",
            &[
                ("pie", "on"),
                ("nx-heap", "off"),
                ("nx-stack", "on"),
                ("code-signature", "none"),
                ("encrypted", "none"),
                ("canary", "found"),
                ("fortify", "yes"),
                ("arc", "not-found"),
                ("restrict", "none"),
                ("pac", "n/a"),
                ("rpath", "none"),
                ("needed", "set"),
            ],
            (2, 8),
            &["/usr/lib/libiconv.2.dylib", "/usr/lib/libSystem.B.dylib"],
        ),
    ];
    let mut read_any = false;
    for &(name, states, (fortified, fortifiable), libraries) in expected {
        let path = dir.join(name);
        if !path.exists() {
            continue;
        }
        read_any = true;
        let (h, doc, mut ev) = read(&path, "macho");
        assert_eq!(h.format, "macho");
        assert_eq!(h.parts.len(), 1, "{name}");
        let rows = &h.parts[0].rows;
        let got: Vec<(&str, &str)> = rows.iter().map(|r| (r.key, r.state)).collect();
        assert_eq!(got, states, "{name}");
        let fortify = row(rows, "fortify");
        assert_eq!((fortify.count, fortify.total), (Some(fortified), Some(fortifiable)), "{name}");
        assert_eq!(row(rows, "needed").items, libraries, "{name}");
        evidence_is_where_it_says(name, &h, &doc, &mut ev);
    }
    assert!(read_any, "{}", qubero_samples::missing());
}

/// The fortify columns of `checksec`, worked out from `readelf --dyn-syms`
/// the way the script does it: names with their leading underscores and their
/// version cut off, fortified when the name is one of libc's `_chk` functions
/// and fortifiable when it is that or the plain one. Counted both by line, as
/// the script counts, and by name, as the pass does.
struct ReadelfFortify {
    lines: (u64, u64),
    names: (u64, u64),
}

fn readelf_fortify(path: &Path, chk: &[String]) -> ReadelfFortify {
    let out = Command::new("readelf").args(["-W", "--dyn-syms"]).arg(path).output().expect("readelf runs");
    let text = String::from_utf8_lossy(&out.stdout);
    let names: Vec<String> = text
        .lines()
        .filter_map(|l| l.split_whitespace().nth(7))
        .map(|n| n.trim_start_matches('_').split('@').next().unwrap_or("").to_string())
        .filter(|n| !n.is_empty())
        .collect();
    let fortified = |n: &String| chk.iter().any(|c| *n == format!("{c}_chk"));
    let plain = |n: &String| chk.iter().any(|c| n == c);
    let lines = (names.iter().filter(|n| fortified(n)).count() as u64, names.iter().filter(|n| fortified(n) || plain(n)).count() as u64);
    let mut distinct = names.clone();
    distinct.sort();
    distinct.dedup();
    let by_name = (distinct.iter().filter(|n| fortified(n)).count() as u64, distinct.iter().filter(|n| fortified(n) || plain(n)).count() as u64);
    ReadelfFortify { lines, names: by_name }
}

/// libc's `_chk` functions, as `checksec` reads them: every `__X_chk@@`
/// symbol of the machine's own libc, as `X`.
fn libc_chk() -> Vec<String> {
    if !Command::new("readelf").arg("--version").output().is_ok_and(|o| o.status.success()) {
        return Vec::new();
    }
    let libc = ["/lib/x86_64-linux-gnu/libc.so.6", "/lib/libc.so.6", "/lib64/libc.so.6"].into_iter().find(|p| Path::new(p).exists());
    let Some(libc) = libc else { return Vec::new() };
    let out = Command::new("readelf").args(["-W", "-s", libc]).output().expect("readelf runs");
    let mut names: Vec<String> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.split_whitespace().nth(7))
        .filter_map(|n| n.split_once("_chk@@").map(|(f, _)| f.trim_start_matches('_').to_string()))
        .collect();
    names.sort();
    names.dedup();
    names
}

/// The programs on the machine running the test, against `checksec` itself.
/// Runs only where `checksec` and `readelf` are installed.
#[test]
fn the_host_programs_read_as_checksec_reads_them() {
    let has = |tool: &str| Command::new(tool).arg("--version").output().is_ok_and(|o| o.status.success());
    if !Path::new("/usr/bin/checksec").exists() || !has("readelf") {
        eprintln!("skipped: needs /usr/bin/checksec and readelf");
        return;
    }
    let chk = libc_chk();
    // The pass counts against the list bundled with it, from glibc 2.39;
    // `checksec` counts against this machine's libc. The counts are only
    // comparable where the two lists are the same.
    let mut bundled: Vec<String> = hardening::FORTIFIABLE.iter().map(|s| s.to_string()).collect();
    bundled.sort();
    let same_list = chk == bundled;
    if !same_list {
        eprintln!("not comparing fortify counts: this machine's libc has a different _chk list from the bundled glibc 2.39 one");
    }
    for name in ["/bin/bash", "/usr/bin/ls", "/usr/bin/ssh", "/usr/bin/python3"] {
        let path = Path::new(name);
        if !path.exists() {
            eprintln!("skipped {name}: not on this machine");
            continue;
        }
        let out = Command::new("/usr/bin/checksec").arg("--output=csv").arg(format!("--file={name}")).output().expect("checksec runs");
        let line = String::from_utf8_lossy(&out.stdout).trim().to_string();
        let theirs: Vec<&str> = line.split(',').collect();
        assert!(theirs.len() >= 11, "{name}: {line}");
        let (h, doc, mut ev) = read(&std::fs::canonicalize(path).unwrap(), "elf");
        let rows = &h.parts[0].rows;
        let ours_csv = checksec_csv(rows);
        let ours: Vec<&str> = ours_csv.split(',').collect();
        // Every column but the two counts, exactly.
        assert_eq!(ours[..8], theirs[..8], "{name}: ours {ours_csv}, checksec {line}");
        if state(rows, "canary") != "found" {
            assert_eq!(theirs[1], "No Canary found", "{name}");
        }
        let fortify = row(rows, "fortify");
        let counted = (fortify.count.unwrap(), fortify.total.unwrap());
        let said = (theirs[8].parse::<u64>().unwrap(), theirs[9].parse::<u64>().unwrap());
        if same_list && counted != said {
            // `checksec` counts lines of `readelf --dyn-syms`; this counts
            // names. Worked out from `readelf` both ways, the line count is
            // `checksec`'s and the name count is this one's, so a function
            // the table lists twice is the whole of the difference.
            assert!(!chk.is_empty(), "{name}: no libc to read the _chk list from");
            let r = readelf_fortify(path, &chk);
            assert_eq!(r.lines, said, "{name}: checksec's counts are its line counts");
            assert_eq!(r.names, counted, "{name}: these counts are the distinct names");
        }
        // The notes, against `readelf -n`: the x86 features the property note
        // names, and the build ID.
        let notes = Command::new("readelf").args(["-W", "-n"]).arg(path).output().expect("readelf runs");
        let notes = String::from_utf8_lossy(&notes.stdout).to_string();
        if let Some(features) = notes.lines().find(|l| l.contains("x86 feature:")) {
            let names: Vec<&str> = features.split("x86 feature:").nth(1).unwrap_or("").split(',').map(|s| s.trim()).collect();
            assert_eq!(state(rows, "ibt") == "on", names.contains(&"IBT"), "{name}: {features}");
            assert_eq!(state(rows, "shstk") == "on", names.contains(&"SHSTK"), "{name}: {features}");
        }
        if let Some(id) = notes.lines().find_map(|l| l.split("Build ID: ").nth(1)) {
            assert_eq!(row(rows, "build-id").items, [id.trim()], "{name}");
        }
        evidence_is_where_it_says(name, &h, &doc, &mut ev);
    }
}
