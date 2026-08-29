//! Measure a decoder against a toolchain's own listing.
//!
//! An objdump `.dis` is the answer key: it holds the bytes and the line the
//! assembler's own tables write for them. Reading both and comparing tells us
//! what this can read, what it cannot, and â€” worst of the three â€” what it
//! reads as something else.
//!
//! Usage: `cargo run --example dis_diff -- <file.dis> <isa> [--all]`

use std::collections::BTreeMap;

use qubero_core::code::{decode, Isa};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = args.first().expect("a .dis file");
    let isa = Isa::named(args.get(1).map(String::as_str).unwrap_or("thumb")).expect("a machine name");
    let show_all = args.iter().any(|a| a == "--all");
    let text = std::fs::read_to_string(path).expect("readable");

    let (mut agree, mut wrong, mut missing) = (0usize, 0usize, 0usize);
    // Which mnemonics we fail on, and how often: the work list, longest first.
    let mut misses: BTreeMap<String, usize> = BTreeMap::new();
    let mut wrongs: BTreeMap<String, (usize, String)> = BTreeMap::new();
    let mut examples: BTreeMap<String, String> = BTreeMap::new();

    for line in text.lines() {
        let Some((bytes, want)) = instruction(line) else { continue };
        // The listing writes data as a directive; only instructions are ours.
        if want.starts_with('.') { continue }
        let want_mnemonic = want.split([' ', '\t']).next().unwrap_or("").to_string();
        let got = decode(isa, &bytes);
        let got_mnemonic = got.text.split([' ', '\t']).next().unwrap_or("").to_string();

        if got.text.starts_with("(bad)") || got.len != bytes.len() {
            missing += 1;
            *misses.entry(want_mnemonic.clone()).or_default() += 1;
            examples.entry(want_mnemonic).or_insert_with(|| format!("{:02x?} {want}", bytes));
        } else if same(&want_mnemonic, &got_mnemonic) {
            agree += 1;
        } else {
            wrong += 1;
            let e = wrongs.entry(want_mnemonic.clone()).or_insert((0, got_mnemonic));
            e.0 += 1;
            examples.entry(want_mnemonic).or_insert_with(|| format!("{:02x?} {want}", bytes));
        }
    }

    let total = agree + wrong + missing;
    println!("{path}  as {}", isa.name());
    println!("  {total} instructions: {agree} read, {missing} unread, {wrong} read as something else");
    report("Read as something else (the dangerous ones)", &wrongs.iter().map(|(k, v)| (k.clone(), v.0, format!("-> {}", v.1))).collect::<Vec<_>>(), &examples, show_all);
    report("Unread", &misses.iter().map(|(k, v)| (k.clone(), *v, String::new())).collect::<Vec<_>>(), &examples, show_all);
}

fn report(title: &str, rows: &[(String, usize, String)], examples: &BTreeMap<String, String>, all: bool) {
    if rows.is_empty() { return }
    let mut rows = rows.to_vec();
    rows.sort_by_key(|(_, n, _)| std::cmp::Reverse(*n));
    let shown = if all { rows.len() } else { rows.len().min(25) };
    println!("\n{title}: {} kinds", rows.len());
    for (name, n, note) in &rows[..shown] {
        println!("  {n:6}  {name:20} {note}  e.g. {}", examples.get(name).map(String::as_str).unwrap_or(""));
    }
    if shown < rows.len() { println!("  ... and {} more kinds", rows.len() - shown) }
}

/// Two names for the same instruction.
///
/// A listing and a decoder disagree about spelling far more often than about
/// meaning, and counting spelling as an error would bury the real ones. Three
/// kinds of disagreement are not errors:
///
/// 1. a suffix that says how wide the encoding was, which the bytes already
///    said: Thumb's `.w` and `.n`, RISC-V's `c.` on a compressed form;
/// 2. a pseudo-instruction, where the assembler has a friendlier name for a
///    base instruction used a particular way: `ret` is `jalr` to the return
///    address, `mv` is `addi` of zero;
/// 3. an alias the standard itself defines, like `zext.b` for `andi` of 255.
fn same(want: &str, got: &str) -> bool {
    let norm = |s: &str| {
        let s = s.trim_end_matches(".w").trim_end_matches(".n");
        let s = s.strip_prefix("c.").unwrap_or(s);
        base(s).to_string()
    };
    norm(want) == norm(got)
}

/// The instruction a friendlier name stands for.
fn base(name: &str) -> &str {
    match name {
        "beqz" => "beq",
        "bnez" => "bne",
        "bltz" | "bgtz" => "blt",
        "bgez" | "blez" => "bge",
        "j" | "jal" => "jal",
        "jr" | "ret" | "jalr" => "jalr",
        "mv" | "li" | "nop" | "zext.b" => "addi",
        "not" => "xori",
        "neg" => "sub",
        "seqz" => "sltiu",
        "snez" => "sltu",
        "csrw" | "csrrw" => "csrrw",
        "csrr" | "csrs" | "csrrs" => "csrrs",
        "csrc" | "csrrc" => "csrrc",
        "csrwi" | "csrrwi" => "csrrwi",
        "csrsi" | "csrrsi" => "csrrsi",
        "csrci" | "csrrci" => "csrrci",
        "unimp" => "unimp",
        other => other,
    }
}

/// The bytes and the text of one line of a listing, or nothing if the line is
/// a heading, a symbol or blank.
fn instruction(line: &str) -> Option<(Vec<u8>, String)> {
    let (addr, rest) = line.split_once(":\t")?;
    if !addr.trim().chars().all(|c| c.is_ascii_hexdigit()) || addr.trim().is_empty() { return None }
    let (hex, text) = rest.split_once('\t')?;
    let mut bytes = Vec::new();
    for group in hex.split_whitespace() {
        if !group.chars().all(|c| c.is_ascii_hexdigit()) || group.len() % 2 != 0 { return None }
        // A group is one number the listing wrote big-endian; the machine
        // holds it little-endian, so it goes in backwards.
        let mut g: Vec<u8> = (0..group.len() / 2)
            .map(|i| u8::from_str_radix(&group[i * 2..i * 2 + 2], 16).unwrap())
            .collect();
        g.reverse();
        bytes.extend(g);
    }
    if bytes.is_empty() { return None }
    Some((bytes, text.trim().to_string()))
}
