//! A Mach-O program's protections, read from its header flags and its load
//! commands, and from the strings of its symbol table.
//!
//! A universal binary holds a whole Mach-O for each machine, and each is a
//! [`Part`] of its own, named by the machine: a program can be built one way
//! for one and another way for the other. Everything a load command says
//! about where something is in the file is counted from the start of its own
//! slice, so each slice's bytes are read from where the slice starts.
//!
//! The symbol table and its strings sit in `__LINKEDIT`, which has no
//! sections and so no field covering it. Evidence found there is bytes with
//! no path, beside the command that says where they are.

use super::*;

const MH_EXECUTE: u64 = 2;
const MH_ALLOW_STACK_EXECUTION: u64 = 1 << 17;
const MH_PIE: u64 = 1 << 21;
const MH_NO_HEAP_EXECUTION: u64 = 1 << 24;

const CPU_ARM64: u64 = 0x0100_000c;
/// The arm64e subtype, the one built for pointer authentication.
const CPU_SUBTYPE_ARM64E: u64 = 2;

const LC_SEGMENT: u64 = 0x1;
const LC_SYMTAB: u64 = 0x2;
const LC_DYSYMTAB: u64 = 0xb;
const LC_LOAD_DYLIB: u64 = 0xc;
const LC_SEGMENT_64: u64 = 0x19;
const LC_CODE_SIGNATURE: u64 = 0x1d;
const LC_ENCRYPTION_INFO: u64 = 0x21;
const LC_ENCRYPTION_INFO_64: u64 = 0x2c;
const LC_LOAD_WEAK_DYLIB: u64 = 0x8000_0018;
const LC_RPATH: u64 = 0x8000_001c;
const LC_REEXPORT_DYLIB: u64 = 0x8000_001f;

/// The names searched for in the string table: the canary's two, with the
/// underscore every Mach-O symbol carries in front, and the Objective-C
/// runtime's release call, which code built with automatic reference
/// counting calls everywhere.
const STRINGS: &[&[u8]] = &[b"___stack_chk_fail", b"___stack_chk_guard", b"_objc_release"];

/// A load command, as much of it as a protection needs.
#[derive(Debug, Clone)]
struct Command {
    kind: u64,
    /// Where the command starts in the file.
    at: u64,
    /// The segment's name, or the library's or directory's, for the commands
    /// that have one.
    name: Option<String>,
    ev: Evidence,
}

/// What the pass read of one Mach-O, a whole file or one slice of a
/// universal one.
struct MachO {
    /// Where this Mach-O starts in the file, which every offset a command
    /// gives is counted from.
    base: u64,
    little: bool,
    bits64: bool,
    file_type: u64,
    cpu_type: u64,
    cpu_subtype: u64,
    flags: u64,
    flags_ev: Evidence,
    file_type_ev: Evidence,
    cpu_subtype_ev: Evidence,
    commands: Vec<Command>,
}

pub(super) fn read<S: Source>(ev: &mut Evaluator, doc: &Document<S>) -> R<Vec<Part>> {
    let Some(slices) = ev.child_named(doc, &[], "slices")? else {
        let Some(m) = MachO::read(ev, doc, &[])? else { return Ok(Vec::new()) };
        return Ok(vec![Part { name: String::new(), path: Vec::new(), rows: m.rows(doc)? }]);
    };
    let architectures = named(ev, doc, &[], "architectures")?;
    let mut parts = Vec::new();
    for i in 0..ev.node(doc, &slices)?.child_count as usize {
        let slice = child(&slices, i);
        // A slice at offset nought is skipped by the template, as it would
        // be the table itself.
        if ev.node(doc, &slice)?.type_name != "MachOFile" {
            continue;
        }
        let Some(m) = MachO::read(ev, doc, &slice)? else { continue };
        let arch = child(&architectures, i);
        let cpu = named(ev, doc, &arch, "cpu_type")?;
        let mut name = match ev.node(doc, &cpu)?.value {
            Value::Enum { name: Some(n), .. } => n,
            v => format!("CPU {}", v.as_int().unwrap_or(0)),
        };
        if m.cpu_type == CPU_ARM64 && m.cpu_subtype & 0xff == CPU_SUBTYPE_ARM64E {
            name.push('e');
        }
        parts.push(Part { name, path: slice, rows: m.rows(doc)? });
    }
    Ok(parts)
}

impl MachO {
    fn read<S: Source>(ev: &mut Evaluator, doc: &Document<S>, path: &[usize]) -> R<Option<MachO>> {
        let n = ev.node(doc, path)?;
        if n.type_name != "MachOFile" {
            return Ok(None);
        }
        let base = n.offset_bits / 8;
        // Which way round the numbers go is which way round the magic is.
        let magic = read_at(doc, base, 4)?;
        let little = magic.first() == Some(&0xcf) || magic.first() == Some(&0xce);
        let field = |ev: &mut Evaluator, name: &str| -> R<(u64, Evidence)> {
            let p = named(ev, doc, path, name)?;
            Ok((int_at(ev, doc, &p)?, node_evidence(ev, doc, &p, "header", name)?))
        };
        let (magic, _) = field(ev, "magic")?;
        let (cpu_type, _) = field(ev, "cpu_type")?;
        let (cpu_subtype, cpu_subtype_ev) = field(ev, "cpu_subtype")?;
        let (file_type, file_type_ev) = field(ev, "file_type")?;
        let (flags, flags_ev) = field(ev, "flags")?;
        let table = named(ev, doc, path, "commands")?;
        let mut commands = Vec::new();
        for k in 0..ev.node(doc, &table)?.child_count as usize {
            let c = child(&table, k);
            let kind = int_field(ev, doc, &c, "command")?;
            let body = named(ev, doc, &c, "body")?;
            let name = match ev.child_named(doc, &body, "name")? {
                Some(p) => text_at(ev, doc, &p)?.map(|(s, _)| s),
                None => None,
            };
            let label = match (kind, &name) {
                (LC_SEGMENT | LC_SEGMENT_64, Some(segment)) => segment.clone(),
                _ => command_name(kind),
            };
            let ev_c = node_evidence(ev, doc, &c, "command", &label)?;
            commands.push(Command { kind, at: ev_c.offset_bits / 8, name, ev: ev_c });
        }
        Ok(Some(MachO {
            base,
            little,
            bits64: magic == 0xfeed_facf,
            file_type,
            cpu_type,
            cpu_subtype,
            flags,
            flags_ev,
            file_type_ev,
            cpu_subtype_ev,
            commands,
        }))
    }

    fn command(&self, kind: u64) -> Option<&Command> {
        self.commands.iter().find(|c| c.kind == kind)
    }

    /// `n` words of four bytes from the body of the command at `at`.
    fn words<S: Source>(&self, doc: &Document<S>, at: u64, n: usize) -> R<Vec<u64>> {
        let b = read_at(doc, at + 8, 4 * n as u64)?;
        Ok((0..n).map(|i| word(&b, 4 * i, 4, self.little)).collect())
    }

    fn rows<S: Source>(&self, doc: &Document<S>) -> R<Vec<Row>> {
        let flags = || self.flags_ev.clone();
        let mut rows = Vec::new();

        rows.push(match (self.file_type == MH_EXECUTE, self.flags & MH_PIE != 0) {
            (false, _) => Row::new("pie", "n/a", Rating::NotApplicable).evidence([self.file_type_ev.clone()]),
            (true, true) => Row::new("pie", "on", Rating::Good).evidence([flags(), self.file_type_ev.clone()]),
            (true, false) => Row::new("pie", "off", Rating::Bad).evidence([flags(), self.file_type_ev.clone()]),
        });
        rows.push(match self.flags & MH_NO_HEAP_EXECUTION != 0 {
            true => Row::new("nx-heap", "on", Rating::Good).evidence([flags()]),
            false => Row::new("nx-heap", "off", Rating::Info).evidence([flags()]),
        });
        rows.push(match self.flags & MH_ALLOW_STACK_EXECUTION != 0 {
            true => Row::new("nx-stack", "off", Rating::Bad).evidence([flags()]),
            false => Row::new("nx-stack", "on", Rating::Good).evidence([flags()]),
        });

        rows.push(match self.command(LC_CODE_SIGNATURE) {
            None => Row::new("code-signature", "none", Rating::Info),
            Some(c) => {
                let w = self.words(doc, c.at, 2)?;
                let bytes = bytes_evidence(self.base + w[0], w[1], "bytes", "code signature");
                Row::new("code-signature", "present", Rating::Info).evidence([c.ev.clone(), bytes])
            }
        });

        rows.push(match self.commands.iter().find(|c| c.kind == LC_ENCRYPTION_INFO || c.kind == LC_ENCRYPTION_INFO_64) {
            None => Row::new("encrypted", "none", Rating::Info),
            Some(c) => {
                // `cryptoff`, `cryptsize`, `cryptid`: nought for the last says
                // the range is not encrypted, which is how an App Store build
                // reads once it has been decrypted.
                let w = self.words(doc, c.at, 3)?;
                let id = bytes_evidence(c.at + 16, 4, "bytes", "cryptid");
                let state = if w[2] != 0 { "yes" } else { "no" };
                Row::new("encrypted", state, Rating::Info).evidence([c.ev.clone(), id])
            }
        });

        let symbols = Symbols::read(doc, self)?;
        rows.push(symbols.presence("canary", &[0, 1], Rating::Bad));
        rows.push(symbols.fortify());
        rows.push(symbols.presence("arc", &[2], Rating::Info));

        let restrict = self.commands.iter().find(|c| matches!(c.kind, LC_SEGMENT | LC_SEGMENT_64) && c.name.as_deref() == Some("__RESTRICT"));
        rows.push(match restrict {
            Some(c) => Row::new("restrict", "present", Rating::Info).evidence([c.ev.clone()]),
            None => Row::new("restrict", "none", Rating::Info),
        });

        rows.push(match (self.cpu_type == CPU_ARM64, self.cpu_subtype & 0xff == CPU_SUBTYPE_ARM64E) {
            (false, _) => Row::new("pac", "n/a", Rating::NotApplicable).evidence([self.cpu_subtype_ev.clone()]),
            (true, true) => Row::new("pac", "on", Rating::Good).evidence([self.cpu_subtype_ev.clone()]),
            (true, false) => Row::new("pac", "off", Rating::Info).evidence([self.cpu_subtype_ev.clone()]),
        });

        // Where the loader looks for a library named `@rpath/...`. A relative
        // directory is looked for from wherever the program is run.
        let rpaths: Vec<&Command> = self.commands.iter().filter(|c| c.kind == LC_RPATH).collect();
        rows.push(match rpaths.is_empty() {
            true => Row::new("rpath", "none", Rating::Good),
            false => {
                let dirs: Vec<String> = rpaths.iter().map(|c| c.name.clone().unwrap_or_default()).collect();
                let safe = |d: &String| d.starts_with('/') || d.starts_with("@executable_path") || d.starts_with("@loader_path");
                let rating = if dirs.iter().all(safe) { Rating::Info } else { Rating::Bad };
                Row::new("rpath", "set", rating).items(dirs).evidence(rpaths.iter().map(|c| c.ev.clone()))
            }
        });

        let libraries: Vec<&Command> =
            self.commands.iter().filter(|c| matches!(c.kind, LC_LOAD_DYLIB | LC_LOAD_WEAK_DYLIB | LC_REEXPORT_DYLIB)).collect();
        rows.push(match libraries.len() {
            0 => Row::new("needed", "none", Rating::Info).count(0),
            n => Row::new("needed", "set", Rating::Info)
                .count(n as u64)
                .items(libraries.iter().map(|c| c.name.clone().unwrap_or_default()).collect())
                .evidence(libraries.iter().map(|c| c.ev.clone())),
        });
        Ok(rows)
    }
}

/// What the symbol table says: the strings found in it, and the names of
/// the symbols the program imports and exports.
struct Symbols {
    /// The `LC_SYMTAB` command, when there is a symbol table at all.
    symtab: Option<Evidence>,
    hits: Vec<Hit>,
    /// The external and undefined symbols, by name, each with its record as
    /// evidence. None without `LC_DYSYMTAB`, which is what says which they
    /// are.
    dynamic: Option<Vec<(String, Evidence)>>,
}

impl Symbols {
    fn read<S: Source>(doc: &Document<S>, m: &MachO) -> R<Symbols> {
        let Some(c) = m.command(LC_SYMTAB) else { return Ok(Symbols { symtab: None, hits: Vec::new(), dynamic: None }) };
        let w = m.words(doc, c.at, 4)?;
        let (symoff, nsyms, stroff, strsize) = (m.base + w[0], w[1], m.base + w[2], w[3]);
        if strsize == 0 {
            return Ok(Symbols { symtab: None, hits: Vec::new(), dynamic: None });
        }
        let (hits, _) = search(doc, stroff, stroff + strsize, STRINGS, EVIDENCE)?;
        let mut out = Symbols { symtab: Some(c.ev.clone()), hits, dynamic: None };
        let Some(d) = m.command(LC_DYSYMTAB) else { return Ok(out) };
        // `iextdefsym`, `nextdefsym`, `iundefsym`, `nundefsym`: the symbols a
        // program exports and the ones it imports, as runs of the table.
        let d = m.words(doc, d.at, 6)?;
        let record = if m.bits64 { 16 } else { 12 };
        let mut names = Vec::new();
        let mut records = Vec::new();
        for (first, count) in [(d[2], d[3]), (d[4], d[5])] {
            let count = count.min(nsyms.saturating_sub(first));
            let bytes = read_at(doc, symoff + first * record, count * record)?;
            for (k, r) in bytes.chunks_exact(record as usize).enumerate() {
                records.push((symoff + (first + k as u64) * record, word(r, 0, 4, m.little)));
            }
        }
        let runs: Vec<(u64, u64)> =
            records.iter().map(|&(_, strx)| (stroff + strx, 512.min(strsize.saturating_sub(strx)))).collect();
        for ((at, _), bytes) in records.iter().zip(read_many(doc, &runs)?) {
            let name = cstr(&bytes, 0).to_string();
            if !name.is_empty() {
                names.push((name.clone(), bytes_evidence(*at, record, "symbol", &name)));
            }
        }
        out.dynamic = Some(names);
        Ok(out)
    }

    /// A row that is only whether one of the needles is in the string table.
    fn presence(&self, key: &'static str, needles: &[usize], missing: Rating) -> Row {
        let Some(symtab) = &self.symtab else { return Row::new(key, "no-symbols", Rating::Unknown) };
        let found: Vec<&Hit> = self.hits.iter().filter(|h| needles.contains(&h.needle)).collect();
        if found.is_empty() {
            return Row::new(key, "not-found", missing).evidence([symtab.clone()]);
        }
        let mut items: Vec<String> = Vec::new();
        for h in &found {
            if !items.contains(&h.text) {
                items.push(h.text.clone());
            }
        }
        let rating = if missing == Rating::Bad { Rating::Good } else { Rating::Info };
        Row::new(key, "found", rating)
            .items(items)
            .evidence(found.iter().map(|h| bytes_evidence(h.at, h.text.len() as u64, "string", &h.text)))
    }

    fn fortify(&self) -> Row {
        let Some(names) = &self.dynamic else { return Row::new("fortify", "no-symbols", Rating::Unknown) };
        let f = Fortify::of(names.iter().enumerate().map(|(i, (n, _))| (n.as_str(), i)));
        let mut row = f.row(|i| names[i].1.clone());
        if row.evidence.is_empty() {
            row.evidence = self.symtab.iter().cloned().collect();
        }
        row
    }
}

/// A load command's name, as the headers that define it spell it.
fn command_name(kind: u64) -> String {
    match kind {
        LC_SEGMENT => "LC_SEGMENT".into(),
        LC_SYMTAB => "LC_SYMTAB".into(),
        LC_DYSYMTAB => "LC_DYSYMTAB".into(),
        LC_LOAD_DYLIB => "LC_LOAD_DYLIB".into(),
        LC_SEGMENT_64 => "LC_SEGMENT_64".into(),
        LC_CODE_SIGNATURE => "LC_CODE_SIGNATURE".into(),
        LC_ENCRYPTION_INFO => "LC_ENCRYPTION_INFO".into(),
        LC_ENCRYPTION_INFO_64 => "LC_ENCRYPTION_INFO_64".into(),
        LC_LOAD_WEAK_DYLIB => "LC_LOAD_WEAK_DYLIB".into(),
        LC_RPATH => "LC_RPATH".into(),
        LC_REEXPORT_DYLIB => "LC_REEXPORT_DYLIB".into(),
        other => format!("0x{other:x}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::MemSource;

    /// One load command: its number, and a body padded to eight bytes.
    fn command(kind: u32, body: &[u8]) -> Vec<u8> {
        let size = (8 + body.len()).next_multiple_of(8);
        let mut v = Vec::new();
        v.extend_from_slice(&kind.to_le_bytes());
        v.extend_from_slice(&(size as u32).to_le_bytes());
        v.extend_from_slice(body);
        v.resize(size, 0);
        v
    }

    fn words(w: &[u32]) -> Vec<u8> {
        w.iter().flat_map(|x| x.to_le_bytes()).collect()
    }

    /// A command whose body is a name after `head` bytes of other fields,
    /// with the offset of the name first.
    fn named(kind: u32, head: &[u32], name: &str) -> Vec<u8> {
        let mut body = words(&[8 + 4 * (head.len() as u32 + 1)]);
        body.extend(words(head));
        body.extend_from_slice(name.as_bytes());
        body.push(0);
        command(kind, &body)
    }

    /// A 64-bit little-endian program with a `__RESTRICT` segment, a symbol
    /// table importing the canary and two of the functions glibc and macOS
    /// both have checked versions of, a library, two run paths, a signature
    /// and an encrypted range.
    fn thin(cpu: u32, subtype: u32, flags: u32) -> Vec<u8> {
        let strings = b"\0_main\0___stack_chk_fail\0___printf_chk\0_memcpy\0_objc_release\0".to_vec();
        let strx = [1u32, 7, 25, 39, 47];
        let mut segment = b"__RESTRICT\0\0\0\0\0\0".to_vec();
        segment.extend_from_slice(&[0; 48]);
        let mut cmds = vec![
            command(0x19, &segment),
            command(0x2, &[0; 16]), // filled in below
            command(0xb, &words(&[0, 0, 0, 1, 1, 4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])),
            named(0xc, &[2, 0x1_0000, 0x1_0000], "/usr/lib/libSystem.B.dylib"),
            named(0x8000_001c, &[], "@loader_path/../lib"),
            named(0x8000_001c, &[], "lib"),
            command(0x1d, &words(&[0, 0])), // filled in below
            command(0x2c, &words(&[0x1000, 0x2000, 1, 0])),
        ];
        let commands: usize = cmds.iter().map(Vec::len).sum();
        let symoff = 32 + commands;
        let stroff = symoff + 16 * strx.len();
        let signature = stroff + strings.len();
        cmds[1] = command(0x2, &words(&[symoff as u32, strx.len() as u32, stroff as u32, strings.len() as u32]));
        cmds[6] = command(0x1d, &words(&[signature as u32, 16]));
        let mut v = words(&[0xfeed_facf, cpu, subtype, 2, cmds.len() as u32, commands as u32, flags, 0]);
        for c in &cmds {
            v.extend_from_slice(c);
        }
        for (k, x) in strx.iter().enumerate() {
            v.extend_from_slice(&x.to_le_bytes());
            v.push(if k == 0 { 0x0f } else { 0x01 }); // defined external, or undefined
            v.extend_from_slice(&[0; 11]);
        }
        v.extend_from_slice(&strings);
        v.extend_from_slice(&[0xfa; 16]);
        v
    }

    fn parts_of(bytes: Vec<u8>) -> Vec<Part> {
        let doc = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(crate::formats::macho());
        let h = super::super::read(&mut ev, &doc).unwrap().expect("a Mach-O");
        assert_eq!(h.format, "macho");
        h.parts
    }

    fn row<'a>(rows: &'a [Row], key: &str) -> &'a Row {
        rows.iter().find(|r| r.key == key).unwrap_or_else(|| panic!("no row {key}"))
    }

    #[test]
    fn a_program_built_with_everything_reads_as_built() {
        let parts = parts_of(thin(0x0100_000c, 2, 0x0120_0085));
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].name, "");
        let rows = &parts[0].rows;
        let states: Vec<(&str, &str)> = rows.iter().map(|r| (r.key, r.state)).collect();
        assert_eq!(
            states,
            [
                ("pie", "on"),
                ("nx-heap", "on"),
                ("nx-stack", "on"),
                ("code-signature", "present"),
                ("encrypted", "yes"),
                ("canary", "found"),
                ("fortify", "yes"),
                ("arc", "found"),
                ("restrict", "present"),
                ("pac", "on"),
                ("rpath", "set"),
                ("needed", "set"),
            ]
        );
        assert_eq!(row(rows, "canary").items, ["___stack_chk_fail"]);
        let fortify = row(rows, "fortify");
        assert_eq!((fortify.count, fortify.total), (Some(1), Some(2)));
        assert_eq!(fortify.items, ["___printf_chk"]);
        assert_eq!(fortify.more, ["_memcpy"]);
        let rpath = row(rows, "rpath");
        assert_eq!(rpath.items, ["@loader_path/../lib", "lib"]);
        assert_eq!(rpath.rating, Rating::Bad);
        assert_eq!(row(rows, "needed").items, ["/usr/lib/libSystem.B.dylib"]);
        assert_eq!(row(rows, "restrict").evidence[0].name, "__RESTRICT");
        assert_eq!(row(rows, "code-signature").evidence[0].name, "LC_CODE_SIGNATURE");
    }

    /// A universal binary is one part per slice, named by its machine, and a
    /// slice's offsets are counted from where the slice starts.
    #[test]
    fn each_slice_of_a_universal_binary_is_a_part_of_its_own() {
        let slices = [(0x0100_0007u32, 3u32, 0x85u32), (0x0100_000c, 2, 0x0020_0085)];
        let mut v = Vec::new();
        v.extend_from_slice(&0xcafe_babeu32.to_be_bytes());
        v.extend_from_slice(&2u32.to_be_bytes());
        let bodies: Vec<Vec<u8>> = slices.iter().map(|&(cpu, sub, flags)| thin(cpu, sub, flags)).collect();
        for (i, &(cpu, sub, _)) in slices.iter().enumerate() {
            for w in [cpu, sub, 0x1000 * (i as u32 + 1), bodies[i].len() as u32, 12] {
                v.extend_from_slice(&w.to_be_bytes());
            }
        }
        for b in &bodies {
            v.resize(v.len().next_multiple_of(0x1000), 0);
            v.extend_from_slice(b);
        }
        let file = v.clone();
        let parts = parts_of(v);
        let names: Vec<&str> = parts.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["x86-64", "ARM64e"]);
        assert_eq!(row(&parts[0].rows, "pie").state, "off");
        assert_eq!(row(&parts[0].rows, "pac").state, "n/a");
        assert_eq!(row(&parts[1].rows, "pie").state, "on");
        assert_eq!(row(&parts[1].rows, "pac").state, "on");
        // The canary's string, found at the second slice's own offset.
        let e = &row(&parts[1].rows, "canary").evidence[0];
        let at = (e.offset_bits / 8) as usize;
        assert!(at > 0x2000);
        assert_eq!(&file[at..at + e.name.len()], b"___stack_chk_fail");
    }
}
