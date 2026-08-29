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

    let (mut agree, mut wrong, mut missing, mut operands) = (0usize, 0usize, 0usize, 0usize);
    // Which mnemonics we fail on, and how often: the work list, longest first.
    let mut misses: BTreeMap<String, usize> = BTreeMap::new();
    let mut wrongs: BTreeMap<String, (usize, String)> = BTreeMap::new();
    let mut differs: BTreeMap<String, (usize, String)> = BTreeMap::new();
    let mut examples: BTreeMap<String, String> = BTreeMap::new();

    for line in text.lines() {
        let Some((at, bytes, want)) = instruction(line) else { continue };
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
            // The right instruction is not yet the right answer: an immediate
            // off by a shift reads as a plausible line and means a different
            // program. Compare what it does as well as what it is called.
            if operands_agree(&want, &got.text, at, got.target) {
                agree += 1;
            } else {
                operands += 1;
                let e = differs.entry(want_mnemonic.clone()).or_insert((0, String::new()));
                e.0 += 1;
                if e.1.is_empty() {
                    e.1 = format!("{want}  |  {}", got.text);
                }
            }
        } else {
            wrong += 1;
            let e = wrongs.entry(want_mnemonic.clone()).or_insert((0, got_mnemonic));
            e.0 += 1;
            examples.entry(want_mnemonic).or_insert_with(|| format!("{:02x?} {want}", bytes));
        }
    }

    let total = agree + wrong + missing;
    println!("{path}  as {}", isa.name());
    println!(
        "  {total} instructions: {agree} read, {missing} unread, {wrong} read as something else, \
         {operands} named right with different operands"
    );
    report("Read as something else (the dangerous ones)", &wrongs.iter().map(|(k, v)| (k.clone(), v.0, format!("-> {}", v.1))).collect::<Vec<_>>(), &examples, show_all);
    report(
        "Named right, operands differ",
        &differs.iter().map(|(k, v)| (k.clone(), v.0, format!("  {}", v.1))).collect::<Vec<_>>(),
        &BTreeMap::new(),
        show_all,
    );
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

/// Whether two lines for the same instruction say the same thing about its
/// operands.
///
/// This has to be forgiving about spelling and strict about values. A listing
/// writes an immediate as `#31956` and a decoder as `0x7cd4`; those agree. A
/// listing writes a branch as the address it lands on and a decoder as the
/// distance to it; those agree once the distance is added to where the
/// instruction sits. But `0x7cd4` and `0x70cd4` do not agree, and that
/// difference is the whole reason to compare operands at all.
///
/// What it cannot check, it does not count against the decoder: a line whose
/// operands do not reduce to a comparable list on both sides is passed.
fn operands_agree(want: &str, got: &str, at: u64, target: Option<i64>) -> bool {
    let a = values(after_mnemonic(want), at, None);
    // Where a branch goes is the decoder's answer, not its text: the text
    // carries whatever the underlying decoder wrote, and `code::decode` has
    // already corrected that into a distance from the first byte.
    let b = values(after_mnemonic(got), at, target);
    // A register list or an addressing mode neither side writes the same way
    // leaves nothing to compare, and a false alarm is worse than no check.
    if a.is_empty() || b.is_empty() || a.len() != b.len() { return true }
    a == b
}

fn after_mnemonic(line: &str) -> &str {
    match line.find([' ', '\t']) {
        Some(i) => line[i..].trim(),
        None => "",
    }
}

/// The operands of one line as comparable tokens: numbers as numbers,
/// registers under one spelling, and everything a listing adds for the
/// reader's benefit thrown away.
fn values(text: &str, at: u64, target: Option<i64>) -> Vec<String> {
    // A listing names the symbol a branch lands in and repeats an immediate
    // in decimal; both are commentary on what the bytes already said. The
    // name is worth one thing on the way past: whatever it follows is the
    // address of a branch, which is the same fact this side writes as a
    // distance.
    let text = text.split(';').next().unwrap_or("");
    let (text, absolute) = match text.find('<') {
        Some(i) => (&text[..i], text[..i].rsplit([',', ' ', '\t']).find(|t| !t.is_empty()).unwrap_or("")),
        None => (text, ""),
    };
    let mut out = Vec::new();
    for token in text.split([',', ' ', '\t', '(', ')', '{', '}', '[', ']', '!']) {
        let token = token.trim().trim_start_matches('#');
        if token.is_empty() { continue }
        // A distance from here and the address it reaches are the same fact.
        if let Some(rest) = token.strip_prefix('$') {
            if let Some(offset) = target.or_else(|| parse_signed(rest).ok()) {
                out.push(format!("@{:x}", at.wrapping_add_signed(offset)));
                continue;
            }
        }
        // The listing's branch target, which is already where the branch
        // lands rather than how far away it is.
        if !absolute.is_empty() && token == absolute {
            if let Ok(address) = i64::from_str_radix(token, 16) {
                out.push(format!("@{address:x}"));
                continue;
            }
        }
        if let Ok(n) = parse_signed(token) {
            out.push(format!("#{n}"));
            continue;
        }
        out.push(register(token));
    }
    out
}

fn parse_signed(token: &str) -> Result<i64, ()> {
    let (sign, rest) = match token.strip_prefix('-') {
        Some(rest) => (-1i64, rest),
        None => (1i64, token.strip_prefix('+').unwrap_or(token)),
    };
    let value = match rest.strip_prefix("0x") {
        Some(hex) => i64::from_str_radix(hex, 16).map_err(|_| ())?,
        None => rest.parse::<i64>().map_err(|_| ())?,
    };
    Ok(sign * value)
}

/// One spelling for a register. ARM's last few have names as well as numbers,
/// and a listing and a decoder do not always pick the same one.
fn register(token: &str) -> String {
    match token {
        "sl" => "r10".into(),
        "fp" => "r11".into(),
        "ip" => "r12".into(),
        "sb" => "r9".into(),
        other => other.to_ascii_lowercase(),
    }
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
        "mv" | "li" | "nop" => "addi",
        "zext.b" => "andi",
        "sltz" | "sgtz" => "slt",
        // The compressed stack forms name the stack in the mnemonic; the
        // decoder here names it in an operand instead.
        "swsp" => "sw",
        "lwsp" => "lw",
        "addi16sp" | "addi4spn" => "addi",
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
fn instruction(line: &str) -> Option<(u64, Vec<u8>, String)> {
    let (addr, rest) = line.split_once(":\t")?;
    if !addr.trim().chars().all(|c| c.is_ascii_hexdigit()) || addr.trim().is_empty() { return None }
    let at = u64::from_str_radix(addr.trim(), 16).ok()?;
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
    Some((at, bytes, text.trim().to_string()))
}
