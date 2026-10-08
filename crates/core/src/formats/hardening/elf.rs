//! An ELF program's protections, read from its segments, its dynamic section,
//! its symbols and its notes.
//!
//! Each verdict is `checksec`'s rule, with three deliberate departures:
//!
//! - A program with no symbols at all cannot say whether it was built with a
//!   stack canary, and says so (`no-symbols`) rather than "no canary". Most
//!   static stripped programs are this.
//! - Immediate binding counts when any of the three ways of saying it is
//!   there: a `BIND_NOW` entry, `bind now` in `FLAGS`, or `now` in `FLAGS_1`.
//!   `checksec` greps for the words `BIND_NOW` and so misses the third.
//! - A position-independent executable is one whose type is shared object and
//!   which has a `DEBUG` entry, as `checksec` has it, or which says `pie` in
//!   `FLAGS_1`, which is how a linker marks one now.
//!
//! The section headers are what say where the symbols and notes are. A
//! program whose section headers were cut off by `sstrip` still has its
//! program headers, which carry the segments, the dynamic section and the
//! notes, so those rows are still answered; the ones that need a symbol table
//! say `no-sections`.

use super::*;

const PT_LOAD: u64 = 1;
const PT_DYNAMIC: u64 = 2;
const PT_INTERP: u64 = 3;
const PT_NOTE: u64 = 4;
const PT_GNU_STACK: u64 = 0x6474_e551;
const PT_GNU_RELRO: u64 = 0x6474_e552;
const PT_GNU_PROPERTY: u64 = 0x6474_e553;

const PF_X: u64 = 1;
const PF_W: u64 = 2;

const SHT_SYMTAB: u64 = 2;
const SHT_STRTAB: u64 = 3;
const SHT_DYNAMIC: u64 = 6;
const SHT_NOTE: u64 = 7;
const SHT_DYNSYM: u64 = 11;

const DT_NULL: u64 = 0;
const DT_NEEDED: u64 = 1;
const DT_STRTAB: u64 = 5;
const DT_STRSZ: u64 = 10;
const DT_SONAME: u64 = 14;
const DT_RPATH: u64 = 15;
const DT_DEBUG: u64 = 21;
const DT_TEXTREL: u64 = 22;
const DT_BIND_NOW: u64 = 24;
const DT_RUNPATH: u64 = 29;
const DT_FLAGS: u64 = 30;
const DT_FLAGS_1: u64 = 0x6fff_fffb;

const DF_TEXTREL: u64 = 0x4;
const DF_BIND_NOW: u64 = 0x8;
const DF_1_NOW: u64 = 0x1;
const DF_1_PIE: u64 = 0x0800_0000;

const NT_GNU_BUILD_ID: u64 = 3;
const NT_GNU_PROPERTY_TYPE_0: u64 = 5;
const GNU_PROPERTY_AARCH64_FEATURE_1_AND: u64 = 0xc000_0000;
const GNU_PROPERTY_X86_FEATURE_1_AND: u64 = 0xc000_0002;

const EM_386: u64 = 3;
const EM_X86_64: u64 = 62;
const EM_AARCH64: u64 = 183;

/// The names `checksec` greps `readelf -s` for, any of which means the
/// program was built with a stack canary.
const CANARY: &[&[u8]] = &[b"__stack_chk_fail", b"__stack_chk_guard", b"__intel_security_cookie"];
const SAFESTACK: &[u8] = b"__safestack_init";
/// The end of a function name Clang's control flow integrity gives the real
/// body of a checked function. Searched for with the NUL after it, so that
/// what is found is a name that ends there.
const CFI: &[u8] = b".cfi\0";

/// A segment, as much of its program header as a protection needs.
#[derive(Debug, Clone)]
struct Segment {
    kind: u64,
    flags: u64,
    offset: u64,
    file_size: u64,
    address: u64,
    align: u64,
    ev: Evidence,
}

#[derive(Debug, Clone)]
struct Section {
    name: String,
    kind: u64,
    offset: u64,
    size: u64,
    link: u64,
    entry_size: u64,
    /// The section's contents in the parsed tree.
    body: Vec<usize>,
    /// Its header, as evidence.
    ev: Evidence,
}

/// One entry of the dynamic section, up to the first `null` one, which is
/// where the loader stops reading.
#[derive(Debug, Clone)]
struct Entry {
    tag: u64,
    value: u64,
    /// The library or directory, for the tags that name one.
    name: Option<String>,
    ev: Evidence,
}

/// A note GNU owns.
#[derive(Debug, Clone)]
struct Note {
    kind: u64,
    desc: Vec<u8>,
    /// Where the description starts in the file, and the field holding it.
    desc_at: u64,
    desc_path: Vec<usize>,
    ev: Evidence,
}

/// What the pass read of one ELF file.
struct Elf {
    header: Vec<usize>,
    bits64: bool,
    little: bool,
    kind: u64,
    machine: u64,
    kind_ev: Evidence,
    /// The two header tables, as evidence for an answer that rests on
    /// something not being in them.
    program_headers: Option<Evidence>,
    section_headers: Option<Evidence>,
    segments: Vec<Segment>,
    /// Empty when the file has no section header table.
    sections: Vec<Section>,
    dynamic: Vec<Entry>,
    /// The dynamic section's header, or its segment where there are no
    /// sections: where an entry would have been.
    dynamic_ev: Option<Evidence>,
    notes: Vec<Note>,
    /// The path `INTERP` names, without the NUL after it.
    interpreter: Option<String>,
}

/// The rows for the ELF program the evaluator is reading. A file whose class
/// or byte order is neither of the two the format has holds no program this
/// can read, and gives no parts.
pub(super) fn read<S: Source>(ev: &mut Evaluator, doc: &Document<S>) -> R<Vec<Part>> {
    let Some(elf) = Elf::read(ev, doc)? else { return Ok(Vec::new()) };
    let symbols = Symbols::read(doc, &elf)?;
    Ok(vec![Part { name: String::new(), path: elf.header.clone(), rows: elf.rows(&symbols) }])
}

/// A failure to read part of a damaged file, as nothing rather than as the
/// end of the pass: the rows that do not need that part are still worth
/// having. Bytes still on their way and work still to do are passed on.
fn soft<T>(r: R<T>) -> R<Option<T>> {
    match r {
        Ok(v) => Ok(Some(v)),
        Err(e) if e.interrupted() => Err(e),
        Err(_) => Ok(None),
    }
}

impl Elf {
    fn read<S: Source>(ev: &mut Evaluator, doc: &Document<S>) -> R<Option<Elf>> {
        let class = int_field(ev, doc, &[], "class")?;
        let data = int_field(ev, doc, &[], "data")?;
        if !(1..=2).contains(&class) || !(1..=2).contains(&data) {
            return Ok(None);
        }
        let header = named(ev, doc, &[], "header")?;
        let kind_path = named(ev, doc, &header, "type")?;
        let mut elf = Elf {
            bits64: class == 2,
            little: data == 1,
            kind: int_at(ev, doc, &kind_path)?,
            machine: int_field(ev, doc, &header, "machine")?,
            kind_ev: node_evidence(ev, doc, &kind_path, "header", "type")?,
            header,
            program_headers: None,
            section_headers: None,
            segments: Vec::new(),
            sections: Vec::new(),
            dynamic: Vec::new(),
            dynamic_ev: None,
            notes: Vec::new(),
            interpreter: None,
        };
        soft(elf.read_segments(ev, doc))?;
        if let Some(seg) = elf.segment(PT_INTERP) {
            let bytes = read_at(doc, seg.offset, seg.file_size.min(4096))?;
            elf.interpreter = Some(cstr(&bytes, 0).to_string());
        }
        soft(elf.read_sections(ev, doc))?;
        soft(elf.read_dynamic(ev, doc))?;
        soft(elf.read_notes(ev, doc))?;
        Ok(Some(elf))
    }

    /// The program headers, read through the template.
    fn read_segments<S: Source>(&mut self, ev: &mut Evaluator, doc: &Document<S>) -> R<()> {
        if int_field(ev, doc, &self.header, "program_header_count")? == 0 {
            return Ok(());
        }
        let table = child(&named(ev, doc, &self.header, "program_headers")?, 0);
        self.program_headers = Some(node_evidence(ev, doc, &table, "header", "program_headers")?);
        let count = ev.node(doc, &table)?.child_count as usize;
        for k in 0..count {
            let p = child(&table, k);
            let kind = int_field(ev, doc, &p, "type")?;
            self.segments.push(Segment {
                kind,
                flags: int_field(ev, doc, &p, "flags")?,
                offset: int_field(ev, doc, &p, "offset")?,
                file_size: int_field(ev, doc, &p, "file_size")?,
                address: int_field(ev, doc, &p, "virtual_address")?,
                align: int_field(ev, doc, &p, "align")?,
                ev: node_evidence(ev, doc, &p, "segment", &segment_name(kind))?,
            });
        }
        Ok(())
    }

    /// The section headers, read through the template, each with its name
    /// and the path its contents are at.
    fn read_sections<S: Source>(&mut self, ev: &mut Evaluator, doc: &Document<S>) -> R<()> {
        if int_field(ev, doc, &self.header, "section_header_count")? == 0
            || int_field(ev, doc, &self.header, "section_header_offset")? == 0
        {
            return Ok(());
        }
        let table = child(&named(ev, doc, &self.header, "section_headers")?, 0);
        self.section_headers = Some(node_evidence(ev, doc, &table, "header", "section_headers")?);
        let bodies = named(ev, doc, &self.header, "sections")?;
        let count = ev.node(doc, &table)?.child_count as usize;
        for i in 0..count {
            let h = child(&table, i);
            let name = match ev.child_named(doc, &h, "name")? {
                Some(p) => text_at(ev, doc, &p)?.map(|(s, _)| s).unwrap_or_default(),
                None => String::new(),
            };
            self.sections.push(Section {
                kind: int_field(ev, doc, &h, "type")?,
                offset: int_field(ev, doc, &h, "offset")?,
                size: int_field(ev, doc, &h, "size")?,
                link: int_field(ev, doc, &h, "link")?,
                entry_size: int_field(ev, doc, &h, "entry_size")?,
                body: child(&bodies, i),
                ev: node_evidence(ev, doc, &h, "section", &name)?,
                name,
            });
        }
        Ok(())
    }

    /// The dynamic section's entries: through the template where a section
    /// header says where it is, and out of the `DYNAMIC` segment's bytes where
    /// none does.
    fn read_dynamic<S: Source>(&mut self, ev: &mut Evaluator, doc: &Document<S>) -> R<()> {
        if let Some(s) = self.sections.iter().find(|s| s.kind == SHT_DYNAMIC).cloned() {
            self.dynamic_ev = Some(s.ev.clone());
            if soft(self.read_dynamic_section(ev, doc, &s))?.is_some() {
                return Ok(());
            }
            self.dynamic.clear();
        }
        let Some(seg) = self.segments.iter().find(|s| s.kind == PT_DYNAMIC).cloned() else { return Ok(()) };
        if self.dynamic_ev.is_none() {
            self.dynamic_ev = Some(seg.ev.clone());
        }
        self.read_dynamic_segment(doc, &seg)
    }

    fn read_dynamic_section<S: Source>(&mut self, ev: &mut Evaluator, doc: &Document<S>, s: &Section) -> R<()> {
        let entries = named(ev, doc, &s.body, "entries")?;
        let count = ev.node(doc, &entries)?.child_count as usize;
        for k in 0..count {
            let e = child(&entries, k);
            let tag = int_field(ev, doc, &e, "tag")?;
            if tag == DT_NULL {
                break;
            }
            let name = match ev.child_named(doc, &e, "name")? {
                Some(p) => text_at(ev, doc, &p)?.map(|(s, _)| s),
                None => None,
            };
            self.dynamic.push(Entry {
                tag,
                value: int_field(ev, doc, &e, "value")?,
                name,
                ev: node_evidence(ev, doc, &e, "dynamic", &tag_name(tag))?,
            });
        }
        Ok(())
    }

    /// The entries out of the segment's own bytes, with each name read from
    /// the string table `DT_STRTAB` gives the address of. No field covers
    /// these bytes in a file with no section headers.
    fn read_dynamic_segment<S: Source>(&mut self, doc: &Document<S>, seg: &Segment) -> R<()> {
        let w = if self.bits64 { 8 } else { 4 };
        let bytes = read_at(doc, seg.offset, seg.file_size.min(1 << 20))?;
        let mut raw = Vec::new();
        for (k, e) in bytes.chunks_exact(2 * w).enumerate() {
            let tag = word(e, 0, w, self.little);
            if tag == DT_NULL {
                break;
            }
            raw.push((k, tag, word(e, w, w, self.little)));
        }
        let lookup = |tag: u64| raw.iter().find(|r| r.1 == tag).map(|r| r.2);
        let strings = match (lookup(DT_STRTAB).and_then(|a| self.file_offset(a)), lookup(DT_STRSZ)) {
            (Some(at), Some(len)) => Some((at, read_at(doc, at, len.min(1 << 20))?)),
            _ => None,
        };
        for (k, tag, value) in raw {
            let name = match (&strings, [DT_NEEDED, DT_SONAME, DT_RPATH, DT_RUNPATH].contains(&tag)) {
                (Some((_, table)), true) => Some(cstr(table, value).to_string()),
                _ => None,
            };
            let at = seg.offset + (k * 2 * w) as u64;
            self.dynamic.push(Entry { tag, value, name, ev: bytes_evidence(at, 2 * w as u64, "dynamic", &tag_name(tag)) });
        }
        Ok(())
    }

    /// Where in the file the program's address `address` is loaded from.
    fn file_offset(&self, address: u64) -> Option<u64> {
        self.segments
            .iter()
            .filter(|s| s.kind == PT_LOAD)
            .find(|s| s.address <= address && address < s.address + s.file_size)
            .map(|s| s.offset + (address - s.address))
    }

    /// The notes GNU wrote: through the template from the note sections where
    /// there are sections, and out of the note segments' bytes where there
    /// are none.
    fn read_notes<S: Source>(&mut self, ev: &mut Evaluator, doc: &Document<S>) -> R<()> {
        if !self.sections.is_empty() {
            for s in self.sections.clone().iter().filter(|s| s.kind == SHT_NOTE) {
                soft(self.read_note_section(ev, doc, s))?;
            }
            return Ok(());
        }
        let mut seen = Vec::new();
        for seg in self.segments.clone().iter().filter(|s| s.kind == PT_NOTE || s.kind == PT_GNU_PROPERTY) {
            if seen.contains(&seg.offset) {
                continue;
            }
            seen.push(seg.offset);
            let bytes = read_at(doc, seg.offset, seg.file_size.min(1 << 20))?;
            let align = if seg.align == 8 { 8 } else { 4 };
            let mut at = 0usize;
            while at + 12 <= bytes.len() {
                let name_size = word(&bytes, at, 4, self.little) as usize;
                let desc_size = word(&bytes, at + 4, 4, self.little) as usize;
                let kind = word(&bytes, at + 8, 4, self.little);
                // Each part starts on the segment's alignment, counted from
                // where the segment starts: an eight-aligned note puts a
                // four-byte name straight after its twelve-byte header.
                let name_at = at + 12;
                let desc_at = (name_at + name_size).next_multiple_of(align);
                let end = (desc_at + desc_size).next_multiple_of(align);
                if desc_at + desc_size > bytes.len() {
                    break;
                }
                if &bytes[name_at..name_at + name_size] == b"GNU\0" {
                    self.notes.push(Note {
                        kind,
                        desc: bytes[desc_at..desc_at + desc_size].to_vec(),
                        desc_at: seg.offset + desc_at as u64,
                        desc_path: Vec::new(),
                        ev: bytes_evidence(seg.offset + at as u64, (end.min(bytes.len()) - at) as u64, "note", &note_name(kind)),
                    });
                }
                at = end;
            }
        }
        Ok(())
    }

    fn read_note_section<S: Source>(&mut self, ev: &mut Evaluator, doc: &Document<S>, s: &Section) -> R<()> {
        let count = ev.node(doc, &s.body)?.child_count as usize;
        for k in 0..count {
            let n = child(&s.body, k);
            let name = named(ev, doc, &n, "name")?;
            let owner = text_at(ev, doc, &name)?.map(|(s, _)| s).unwrap_or_default();
            if owner != "GNU" {
                continue;
            }
            let kind = int_field(ev, doc, &n, "type")?;
            let desc_path = named(ev, doc, &n, "desc")?;
            let d = ev.node(doc, &desc_path)?;
            self.notes.push(Note {
                kind,
                desc: read_at(doc, d.offset_bits / 8, (d.size_bits / 8).min(1 << 16))?,
                desc_at: d.offset_bits / 8,
                desc_path,
                ev: node_evidence(ev, doc, &n, "note", &note_name(kind))?,
            });
        }
        Ok(())
    }

    fn segment(&self, kind: u64) -> Option<&Segment> {
        self.segments.iter().find(|s| s.kind == kind)
    }

    fn entries(&self, tag: u64) -> impl Iterator<Item = &Entry> {
        self.dynamic.iter().filter(move |e| e.tag == tag)
    }

    /// The entries that say what `bit` of `FLAGS` or of `FLAGS_1` says, and
    /// those of `tag`, which says it with a tag of its own.
    fn saying(&self, tag: Option<u64>, flags: u64, flags_1: u64) -> Vec<Evidence> {
        self.dynamic
            .iter()
            .filter(|e| {
                Some(e.tag) == tag
                    || (e.tag == DT_FLAGS && e.value & flags != 0)
                    || (e.tag == DT_FLAGS_1 && e.value & flags_1 != 0)
            })
            .map(|e| e.ev.clone())
            .collect()
    }

    /// The GNU property word of type `kind`, and where its bytes are.
    fn property(&self, kind: u64) -> Option<(u64, Evidence)> {
        let pad = if self.bits64 { 8 } else { 4 };
        for note in self.notes.iter().filter(|n| n.kind == NT_GNU_PROPERTY_TYPE_0) {
            let d = &note.desc;
            let mut at = 0usize;
            while at + 8 <= d.len() {
                let pr_type = word(d, at, 4, self.little);
                let size = word(d, at + 4, 4, self.little) as usize;
                if pr_type == kind && size >= 4 && at + 12 <= d.len() {
                    let value = word(d, at + 8, 4, self.little);
                    let ev = Evidence {
                        path: note.desc_path.clone(),
                        offset_bits: (note.desc_at + at as u64 + 8) * 8,
                        size_bits: 32,
                        what: "bytes",
                        name: property_name(kind).to_string(),
                    };
                    return Some((value, ev));
                }
                at += 8 + size.next_multiple_of(pad);
            }
        }
        None
    }

    fn property_note(&self) -> Option<Evidence> {
        self.notes.iter().find(|n| n.kind == NT_GNU_PROPERTY_TYPE_0).map(|n| n.ev.clone())
    }

    fn rows(&self, sym: &Symbols) -> Vec<Row> {
        // Whether the program is loaded at all. An object file and a core dump
        // are not, so how they would be has no answer.
        let not_loaded = match self.kind {
            1 => Some("object"),
            4 => Some("core"),
            _ => None,
        };
        let na = |key: &'static str, state: &'static str| Row::new(key, state, Rating::NotApplicable).evidence([self.kind_ev.clone()]);
        let looked_in = |what: &Option<Evidence>| what.iter().cloned().collect::<Vec<_>>();
        let mut rows = Vec::new();

        // RELRO: a segment the loader makes read-only once it has relocated
        // it, and whether it binds every symbol before the program starts,
        // which is what lets that cover the table of imported addresses.
        rows.push(match (not_loaded, self.segment(PT_GNU_RELRO)) {
            (Some(state), _) => na("relro", state),
            (None, None) => Row::new("relro", "none", Rating::Bad).evidence(looked_in(&self.program_headers)),
            (None, Some(relro)) => {
                let now = self.saying(Some(DT_BIND_NOW), DF_BIND_NOW, DF_1_NOW);
                match now.is_empty() {
                    false => Row::new("relro", "full", Rating::Good).evidence([relro.ev.clone()].into_iter().chain(now)),
                    true => Row::new("relro", "partial", Rating::Partial)
                        .evidence([relro.ev.clone()].into_iter().chain(self.dynamic_ev.clone())),
                }
            }
        });

        rows.push(sym.canary(self));

        // NX: the stack is not executable unless the `GNU_STACK` segment says
        // it is, and with no such segment at all the loader makes it
        // executable to be safe.
        rows.push(match (not_loaded, self.segment(PT_GNU_STACK)) {
            (Some(state), _) => na("nx", state),
            (None, None) => Row::new("nx", "missing", Rating::Bad).evidence(looked_in(&self.program_headers)),
            (None, Some(s)) if s.flags & PF_X != 0 => Row::new("nx", "off", Rating::Bad).evidence([s.ev.clone()]),
            (None, Some(s)) => Row::new("nx", "on", Rating::Good).evidence([s.ev.clone()]),
        });

        rows.push(self.pie());

        for (key, tag) in [("rpath", DT_RPATH), ("runpath", DT_RUNPATH)] {
            rows.push(match not_loaded {
                Some(state) => na(key, state),
                None => self.search_path(key, tag),
            });
        }

        rows.push(sym.symbols(self));
        rows.push(sym.fortify(self));

        rows.push(match not_loaded {
            Some(state) => na("textrel", state),
            None => {
                let found = self.saying(Some(DT_TEXTREL), DF_TEXTREL, 0);
                match found.is_empty() {
                    true => Row::new("textrel", "none", Rating::Good).evidence(looked_in(&self.dynamic_ev)),
                    false => Row::new("textrel", "present", Rating::Bad).evidence(found),
                }
            }
        });

        rows.push(match not_loaded {
            Some(state) => na("rwx", state),
            None => {
                let loads: Vec<&Segment> = self.segments.iter().filter(|s| s.kind == PT_LOAD).collect();
                let rwx: Vec<Evidence> =
                    loads.iter().filter(|s| s.flags & (PF_W | PF_X) == PF_W | PF_X).map(|s| s.ev.clone()).collect();
                match rwx.len() {
                    0 => Row::new("rwx", "none", Rating::Good).count(0).evidence(loads.iter().map(|s| s.ev.clone())),
                    n => Row::new("rwx", "present", Rating::Bad).count(n as u64).evidence(rwx),
                }
            }
        });

        // Control-flow protections the hardware enforces, which the program
        // says it was built for in a GNU property note. Only for the machines
        // that have them.
        let features: &[(&'static str, u64, u64)] = match self.machine {
            EM_386 | EM_X86_64 => &[("ibt", GNU_PROPERTY_X86_FEATURE_1_AND, 0x1), ("shstk", GNU_PROPERTY_X86_FEATURE_1_AND, 0x2)],
            EM_AARCH64 => &[("bti", GNU_PROPERTY_AARCH64_FEATURE_1_AND, 0x1), ("pac", GNU_PROPERTY_AARCH64_FEATURE_1_AND, 0x2)],
            _ => &[],
        };
        for &(key, kind, bit) in features {
            rows.push(match self.property(kind) {
                Some((value, word)) if value & bit != 0 => {
                    Row::new(key, "on", Rating::Good).evidence(self.property_note().into_iter().chain([word]))
                }
                Some((_, word)) => Row::new(key, "off", Rating::Bad).evidence(self.property_note().into_iter().chain([word])),
                None => Row::new(key, "off", Rating::Bad).evidence(self.property_note()),
            });
        }

        rows.push(sym.safestack(self));
        rows.push(sym.cfi(self));

        let interp = self.segment(PT_INTERP);
        let dynamic = self.segment(PT_DYNAMIC);
        rows.push(match not_loaded {
            Some(state) => na("linking", state),
            None => {
                let pie_flag = self.entries(DT_FLAGS_1).find(|e| e.value & DF_1_PIE != 0);
                match (interp, dynamic, pie_flag) {
                    (None, _, Some(flag)) if self.kind == 3 => Row::new("linking", "static-pie", Rating::Info)
                        .evidence(dynamic.map(|s| s.ev.clone()).into_iter().chain([flag.ev.clone(), self.kind_ev.clone()])),
                    (None, None, _) => Row::new("linking", "static", Rating::Info).evidence(looked_in(&self.program_headers)),
                    _ => Row::new("linking", "dynamic", Rating::Info).evidence(interp.into_iter().chain(dynamic).map(|s| s.ev.clone())),
                }
            }
        });

        rows.push(match (not_loaded, interp) {
            (Some(state), _) => na("interpreter", state),
            (None, None) => Row::new("interpreter", "none", Rating::Info).evidence(looked_in(&self.program_headers)),
            (None, Some(seg)) => {
                let path = self.interpreter.clone().unwrap_or_default();
                let mut evidence = vec![seg.ev.clone()];
                // The path's bytes, as the `.interp` section where a section
                // header places them.
                let body = self.sections.iter().find(|s| s.offset == seg.offset && s.kind != 0).map(|s| s.body.clone());
                evidence.push(Evidence {
                    path: body.unwrap_or_default(),
                    offset_bits: seg.offset * 8,
                    size_bits: path.len() as u64 * 8,
                    what: "string",
                    name: path.clone(),
                });
                Row::new("interpreter", "set", Rating::Info).items(vec![path]).evidence(evidence)
            }
        });

        let needed: Vec<&Entry> = self.entries(DT_NEEDED).collect();
        rows.push(match needed.len() {
            0 => Row::new("needed", "none", Rating::Info).count(0).evidence(looked_in(&self.dynamic_ev)),
            n => Row::new("needed", "set", Rating::Info)
                .count(n as u64)
                .items(needed.iter().map(|e| e.name.clone().unwrap_or_default()).collect())
                .evidence(needed.iter().map(|e| e.ev.clone())),
        });

        rows.push(match self.entries(DT_SONAME).next() {
            None => Row::new("soname", "none", Rating::Info).evidence(looked_in(&self.dynamic_ev)),
            Some(e) => Row::new("soname", "set", Rating::Info).items(vec![e.name.clone().unwrap_or_default()]).evidence([e.ev.clone()]),
        });

        rows.push(match self.notes.iter().find(|n| n.kind == NT_GNU_BUILD_ID) {
            None => Row::new("build-id", "none", Rating::Info),
            Some(n) => {
                let hex: String = n.desc.iter().map(|b| format!("{b:02x}")).collect();
                let bytes = Evidence {
                    path: n.desc_path.clone(),
                    offset_bits: n.desc_at * 8,
                    size_bits: n.desc.len() as u64 * 8,
                    what: "bytes",
                    name: hex.clone(),
                };
                Row::new("build-id", "set", Rating::Info).items(vec![hex]).evidence([n.ev.clone(), bytes])
            }
        });

        rows
    }

    /// Whether the program can be loaded anywhere: an executable is loaded
    /// where its addresses say, and a shared object anywhere. A shared object
    /// is an executable built to be loaded anywhere when it has a `DEBUG`
    /// entry for a debugger to find it by, which a library has no use for,
    /// or says `pie` in `FLAGS_1`.
    ///
    /// Not by its interpreter: glibc's `libc.so.6` names one so that it can be
    /// run to print its version, and is a library all the same.
    fn pie(&self) -> Row {
        let kind = self.kind_ev.clone();
        match self.kind {
            1 => Row::new("pie", "object", Rating::NotApplicable).evidence([kind]),
            4 => Row::new("pie", "core", Rating::NotApplicable).evidence([kind]),
            2 => Row::new("pie", "exec", Rating::Bad).evidence([kind]),
            3 => {
                let says = self.saying(Some(DT_DEBUG), 0, DF_1_PIE);
                match says.is_empty() {
                    false => Row::new("pie", "pie", Rating::Good).evidence([kind].into_iter().chain(says)),
                    true => Row::new("pie", "dso", Rating::Info).evidence([kind].into_iter().chain(self.dynamic_ev.clone())),
                }
            }
            _ => Row::new("pie", "unknown", Rating::Unknown).evidence([kind]),
        }
    }

    /// Where the loader is told to look for libraries before the usual
    /// places. A directory that is empty or relative is looked for from
    /// wherever the program happens to be run, which lets whoever controls
    /// that directory choose the libraries.
    fn search_path(&self, key: &'static str, tag: u64) -> Row {
        let entries: Vec<&Entry> = self.entries(tag).collect();
        if entries.is_empty() {
            return Row::new(key, "none", Rating::Good).evidence(self.dynamic_ev.clone());
        }
        let dirs: Vec<String> =
            entries.iter().flat_map(|e| e.name.clone().unwrap_or_default().split(':').map(str::to_string).collect::<Vec<_>>()).collect();
        let unsafe_dir = dirs.iter().any(|d| !(d.starts_with('/') || d.starts_with("$ORIGIN") || d.starts_with("${ORIGIN}")));
        Row::new(key, "set", if unsafe_dir { Rating::Bad } else { Rating::Info }).items(dirs).evidence(entries.iter().map(|e| e.ev.clone()))
    }
}

/// The two symbol tables, as far as a protection needs them: every name the
/// dynamic one holds, which is what the program imports and exports and is
/// small, and the strings of the static one that matter, found by searching
/// its string table's bytes rather than by reading every symbol.
struct Symbols {
    /// The dynamic symbol table's header and its names, each with its
    /// record as evidence.
    dynsym: Option<(Evidence, Vec<(String, Evidence)>)>,
    /// Each static symbol table's header and how many symbols it has.
    symtab: Vec<(Evidence, u64)>,
    /// What the search of the static tables' strings found, and how many of
    /// each needle there were.
    hits: Vec<(Hit, Evidence)>,
    counts: Vec<u64>,
}

/// The needles the static tables' strings are searched for: the canary's
/// three names, SafeStack's, and the end of a CFI name.
fn needles() -> Vec<&'static [u8]> {
    CANARY.iter().copied().chain([SAFESTACK, CFI]).collect()
}

impl Symbols {
    fn read<S: Source>(doc: &Document<S>, elf: &Elf) -> R<Symbols> {
        let mut out = Symbols { dynsym: None, symtab: Vec::new(), hits: Vec::new(), counts: vec![0; needles().len()] };
        let record = if elf.bits64 { 24 } else { 16 };
        let strings_of = |s: &Section| elf.sections.get(s.link as usize).filter(|t| t.kind == SHT_STRTAB);
        if let Some(s) = elf.sections.iter().find(|s| s.kind == SHT_DYNSYM) {
            // The records are read as bytes rather than as nodes: a library's
            // table runs to tens of thousands of them. Each is `record` bytes,
            // which is what the template counts them by too, so the one at `k`
            // is the template's element `k`.
            let bytes = read_at(doc, s.offset, s.size)?;
            let table = match strings_of(s) {
                Some(t) => read_at(doc, t.offset, t.size)?,
                None => Vec::new(),
            };
            let names = bytes
                .chunks_exact(record)
                .enumerate()
                .skip(1)
                .filter_map(|(k, r)| {
                    let name = cstr(&table, word(r, 0, 4, elf.little));
                    (!name.is_empty()).then(|| {
                        let at = s.offset + (k * record) as u64;
                        let ev = Evidence { path: child(&s.body, k), offset_bits: at * 8, size_bits: record as u64 * 8, what: "symbol", name: name.to_string() };
                        (name.to_string(), ev)
                    })
                })
                .collect();
            out.dynsym = Some((s.ev.clone(), names));
        }
        let wanted = needles();
        for s in elf.sections.iter().filter(|s| s.kind == SHT_SYMTAB) {
            let entry = if s.entry_size > 0 { s.entry_size } else { record as u64 };
            out.symtab.push((s.ev.clone(), s.size / entry));
            let Some(t) = strings_of(s) else { continue };
            let (hits, counts) = search(doc, t.offset, t.offset + t.size, &wanted, EVIDENCE)?;
            for (total, n) in out.counts.iter_mut().zip(counts) {
                *total += n;
            }
            for h in hits {
                let path = match h.index {
                    Some(i) => child(&t.body, i as usize),
                    None => t.body.clone(),
                };
                let ev = Evidence { path, offset_bits: h.at * 8, size_bits: h.text.len() as u64 * 8, what: "string", name: h.text.clone() };
                out.hits.push((h, ev));
            }
        }
        Ok(out)
    }

    /// Whether there is any symbol table to look in.
    fn none(&self) -> bool {
        self.dynsym.is_none() && self.symtab.is_empty()
    }

    /// Where a search that found nothing looked.
    fn looked(&self) -> Vec<Evidence> {
        self.dynsym.iter().map(|(h, _)| h.clone()).chain(self.symtab.iter().map(|(h, _)| h.clone())).collect()
    }

    /// The dynamic symbols whose names `like` accepts, and the static
    /// strings holding needle `needles`.
    fn matching(&self, like: impl Fn(&str) -> bool, needles: &[usize]) -> Vec<(String, Evidence)> {
        let dynamic = self.dynsym.iter().flat_map(|(_, names)| names.iter()).filter(|(n, _)| like(n)).cloned();
        let found = self.hits.iter().filter(|(h, _)| needles.contains(&h.needle)).map(|(h, e)| (h.text.clone(), e.clone()));
        dynamic.chain(found).collect()
    }

    /// A row that is only whether some symbol is there, given the rows a
    /// program with no symbols at all, or no section headers, gets instead.
    fn presence(&self, elf: &Elf, key: &'static str, found: Vec<(String, Evidence)>, missing: Rating) -> Row {
        if elf.sections.is_empty() {
            return Row::new(key, "no-sections", Rating::Unknown);
        }
        if found.is_empty() {
            return match self.none() {
                true => Row::new(key, "no-symbols", Rating::Unknown),
                false => Row::new(key, "not-found", missing).evidence(self.looked()),
            };
        }
        let mut items: Vec<String> = Vec::new();
        for (n, _) in &found {
            if !items.contains(n) {
                items.push(n.clone());
            }
        }
        Row::new(key, "found", Rating::Good).items(items).evidence(found.into_iter().map(|(_, e)| e))
    }

    fn canary(&self, elf: &Elf) -> Row {
        let like = |n: &str| CANARY.iter().any(|c| n.contains(std::str::from_utf8(c).unwrap_or("")));
        self.presence(elf, "canary", self.matching(like, &[0, 1, 2]), Rating::Bad)
    }

    fn safestack(&self, elf: &Elf) -> Row {
        self.presence(elf, "safestack", self.matching(|n| n.contains("__safestack_init"), &[3]), Rating::Info)
    }

    /// Functions Clang's control flow integrity checks, each of which keeps
    /// its real body under its own name with `.cfi` after it. Counted in
    /// full; listed and shown only as far as the first dozen.
    fn cfi(&self, elf: &Elf) -> Row {
        let found = self.matching(|n| n.ends_with(".cfi"), &[4]);
        let dynamic = self.dynsym.iter().flat_map(|(_, names)| names.iter()).filter(|(n, _)| n.ends_with(".cfi")).count() as u64;
        let mut row = self.presence(elf, "cfi", found, Rating::Info);
        if row.state == "found" || row.state == "not-found" {
            row.count = Some(dynamic + self.counts[4]);
            row.items.truncate(EVIDENCE);
        }
        row
    }

    /// Whether the program keeps its static symbol table, and any debugging
    /// information beside it.
    fn symbols(&self, elf: &Elf) -> Row {
        if elf.sections.is_empty() {
            return Row::new("symbols", "no-sections", Rating::Unknown);
        }
        let debug: Vec<&Section> = elf
            .sections
            .iter()
            .filter(|s| s.name.starts_with(".debug_") || s.name.starts_with(".zdebug_") || s.name == ".gnu_debuglink")
            .collect();
        let items = debug.iter().map(|s| s.name.clone()).collect();
        let debug_ev = debug.iter().map(|s| s.ev.clone());
        match self.symtab.first() {
            None => Row::new("symbols", "stripped", Rating::Info).items(items).evidence(elf.section_headers.iter().cloned().chain(debug_ev)),
            Some((h, n)) => Row::new("symbols", "present", Rating::Info).count(*n).items(items).evidence([h.clone()].into_iter().chain(debug_ev)),
        }
    }

    /// Which of the functions glibc has a checked version of the program
    /// imports, and which version. Read from the dynamic symbols alone, as
    /// `checksec` reads them: a statically linked program has the checked
    /// functions inside it and nothing here says which it calls.
    fn fortify(&self, elf: &Elf) -> Row {
        if elf.sections.is_empty() {
            return Row::new("fortify", "no-sections", Rating::Unknown);
        }
        let Some((header, names)) = &self.dynsym else { return Row::new("fortify", "no-symbols", Rating::Unknown) };
        let f = Fortify::of(names.iter().enumerate().map(|(i, (n, _))| (n.as_str(), i)));
        let mut row = f.row(|i| names[i].1.clone());
        if row.evidence.is_empty() {
            row.evidence = vec![header.clone()];
        }
        row
    }
}

/// A segment type as `readelf` writes it.
fn segment_name(kind: u64) -> String {
    match kind {
        0 => "NULL".into(),
        PT_LOAD => "LOAD".into(),
        PT_DYNAMIC => "DYNAMIC".into(),
        PT_INTERP => "INTERP".into(),
        PT_NOTE => "NOTE".into(),
        5 => "SHLIB".into(),
        6 => "PHDR".into(),
        7 => "TLS".into(),
        0x6474_e550 => "GNU_EH_FRAME".into(),
        PT_GNU_STACK => "GNU_STACK".into(),
        PT_GNU_RELRO => "GNU_RELRO".into(),
        PT_GNU_PROPERTY => "GNU_PROPERTY".into(),
        other => format!("0x{other:x}"),
    }
}

/// A dynamic tag as `readelf` writes it, for the ones a row is about.
fn tag_name(tag: u64) -> String {
    match tag {
        DT_NEEDED => "NEEDED".into(),
        DT_STRTAB => "STRTAB".into(),
        DT_SONAME => "SONAME".into(),
        DT_RPATH => "RPATH".into(),
        DT_DEBUG => "DEBUG".into(),
        DT_TEXTREL => "TEXTREL".into(),
        DT_BIND_NOW => "BIND_NOW".into(),
        DT_RUNPATH => "RUNPATH".into(),
        DT_FLAGS => "FLAGS".into(),
        DT_FLAGS_1 => "FLAGS_1".into(),
        other => format!("0x{other:x}"),
    }
}

fn note_name(kind: u64) -> String {
    match kind {
        1 => "NT_GNU_ABI_TAG".into(),
        2 => "NT_GNU_HWCAP".into(),
        NT_GNU_BUILD_ID => "NT_GNU_BUILD_ID".into(),
        4 => "NT_GNU_GOLD_VERSION".into(),
        NT_GNU_PROPERTY_TYPE_0 => "NT_GNU_PROPERTY_TYPE_0".into(),
        other => format!("0x{other:x}"),
    }
}

fn property_name(kind: u64) -> &'static str {
    match kind {
        GNU_PROPERTY_X86_FEATURE_1_AND => "GNU_PROPERTY_X86_FEATURE_1_AND",
        GNU_PROPERTY_AARCH64_FEATURE_1_AND => "GNU_PROPERTY_AARCH64_FEATURE_1_AND",
        _ => "GNU_PROPERTY",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formats::elf::fixture::{self, Covers, Elf as Build};
    use crate::source::MemSource;

    const R: u32 = 4;
    const W: u32 = 2;
    const X: u32 = 1;

    fn rows_of(bytes: Vec<u8>) -> (Vec<Row>, Document<MemSource>, Evaluator) {
        let doc = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(crate::formats::elf());
        let h = super::super::read(&mut ev, &doc).unwrap().expect("an ELF");
        assert_eq!(h.format, "elf");
        assert_eq!(h.parts.len(), 1);
        assert_eq!(h.parts[0].path, ev.child_named(&doc, &[], "header").unwrap().unwrap());
        (h.parts[0].rows.clone(), doc, ev)
    }

    fn row<'a>(rows: &'a [Row], key: &str) -> &'a Row {
        rows.iter().find(|r| r.key == key).unwrap_or_else(|| panic!("no row {key}"))
    }

    /// Every piece of evidence with a path is inside the node the path names,
    /// which is what lets the view go to it.
    fn evidence_is_where_it_says(rows: &[Row], doc: &Document<MemSource>, ev: &mut Evaluator) {
        for r in rows {
            assert!(r.evidence.len() <= EVIDENCE, "{}", r.key);
            for e in r.evidence.iter().filter(|e| !e.path.is_empty()) {
                let n = ev.node(doc, &e.path).unwrap_or_else(|err| panic!("{} {e:?}: {err:?}", r.key));
                assert!(
                    n.offset_bits <= e.offset_bits && e.offset_bits + e.size_bits <= n.offset_bits + n.size_bits,
                    "{}: {e:?} is not inside {} at {}+{}",
                    r.key,
                    n.name,
                    n.offset_bits,
                    n.size_bits
                );
            }
        }
    }

    /// A position-independent executable built the way a distribution builds
    /// one: imports the canary and a fortified call, binds everything now,
    /// marks the stack not executable, and says in a property note that it
    /// was built for indirect branch tracking and a shadow stack.
    fn hardened() -> Build {
        let (dynstr, at) = fixture::strings(&["libc.so.6", "__stack_chk_fail", "__printf_chk", "memcpy", "printf", "$ORIGIN/../lib:lib"]);
        let mut property = Vec::new();
        property.extend_from_slice(&0xc000_0002u32.to_le_bytes());
        property.extend_from_slice(&4u32.to_le_bytes());
        property.extend_from_slice(&3u32.to_le_bytes()); // IBT and SHSTK
        property.extend_from_slice(&0u32.to_le_bytes()); // padded to eight
        let dynamic = fixture::dynamic(&[(1, at[0]), (29, at[5]), (21, 0), (30, 0x8), (0x6fff_fffb, 0x0800_0001), (0, 0)]);
        Build::new(3, 62)
            .section(".interp", 1, b"/lib64/ld-linux-x86-64.so.2\0".to_vec())
            .section(".note.gnu.property", 7, fixture::note("GNU", 5, &property))
            .section(".note.gnu.build-id", 7, fixture::note("GNU", 3, &[0xab, 0xcd, 0xef, 0x01]))
            .section(".dynstr", 3, dynstr)
            .linked(".dynsym", 11, ".dynstr", 24, fixture::symbols(&at[1..5]))
            .linked(".dynamic", 6, ".dynstr", 16, dynamic)
            .section(".text", 1, vec![0x90; 16])
            .segment(3, R, Covers::Section(".interp".into()))
            .segment(1, R | X, Covers::WholeFile)
            .segment(2, R | W, Covers::Section(".dynamic".into()))
            .segment(4, R, Covers::Section(".note.gnu.property".into()))
            .segment(0x6474_e551, R | W, Covers::Nothing)
            .segment(0x6474_e552, R, Covers::Section(".dynamic".into()))
    }

    #[test]
    fn a_hardened_program_reads_as_hardened_and_says_where() {
        let elf = hardened();
        let (rows, doc, mut ev) = rows_of(elf.build());
        let states: Vec<(&str, &str)> = rows.iter().map(|r| (r.key, r.state)).collect();
        assert_eq!(
            states,
            [
                ("relro", "full"),
                ("canary", "found"),
                ("nx", "on"),
                ("pie", "pie"),
                ("rpath", "none"),
                ("runpath", "set"),
                ("symbols", "stripped"),
                ("fortify", "yes"),
                ("textrel", "none"),
                ("rwx", "none"),
                ("ibt", "on"),
                ("shstk", "on"),
                ("safestack", "not-found"),
                ("cfi", "not-found"),
                ("linking", "dynamic"),
                ("interpreter", "set"),
                ("needed", "set"),
                ("soname", "none"),
                ("build-id", "set"),
            ]
        );
        evidence_is_where_it_says(&rows, &doc, &mut ev);

        let relro = row(&rows, "relro");
        let names: Vec<(&str, &str)> = relro.evidence.iter().map(|e| (e.what, e.name.as_str())).collect();
        // Both of the entries that say "bind now", FLAGS and FLAGS_1.
        assert_eq!(names, [("segment", "GNU_RELRO"), ("dynamic", "FLAGS"), ("dynamic", "FLAGS_1")]);

        let canary = row(&rows, "canary");
        assert_eq!(canary.items, ["__stack_chk_fail"]);
        assert_eq!((canary.evidence[0].what, canary.evidence[0].name.as_str()), ("symbol", "__stack_chk_fail"));
        // The dynamic symbol is the template's record of it.
        let sym = ev.node(&doc, &canary.evidence[0].path).unwrap();
        assert_eq!(sym.offset_bits, (elf.offset_of(".dynsym") + 24) * 8);
        assert_eq!(sym.size_bits, 24 * 8);

        let pie = row(&rows, "pie");
        let names: Vec<&str> = pie.evidence.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["type", "DEBUG", "FLAGS_1"]);

        let runpath = row(&rows, "runpath");
        assert_eq!(runpath.items, ["$ORIGIN/../lib", "lib"]);
        assert_eq!(runpath.rating, Rating::Bad, "a relative directory is looked for wherever the program is run from");

        let fortify = row(&rows, "fortify");
        assert_eq!((fortify.count, fortify.total), (Some(1), Some(3)));
        assert_eq!(fortify.items, ["__printf_chk"]);
        assert_eq!(fortify.more, ["memcpy", "printf"]);
        assert_eq!(fortify.rating, Rating::Good);

        let ibt = row(&rows, "ibt");
        assert_eq!((ibt.evidence[0].what, ibt.evidence[0].name.as_str()), ("note", "NT_GNU_PROPERTY_TYPE_0"));
        assert_eq!(ibt.evidence[1].name, "GNU_PROPERTY_X86_FEATURE_1_AND");
        // The four bytes of the feature word, inside the note's description.
        assert_eq!(ibt.evidence[1].offset_bits, (elf.offset_of(".note.gnu.property") + 16 + 8) * 8);

        let interp = row(&rows, "interpreter");
        assert_eq!(interp.items, ["/lib64/ld-linux-x86-64.so.2"]);
        assert_eq!(interp.evidence[1].what, "string");
        assert!(!interp.evidence[1].path.is_empty(), "the .interp section holds the path");

        assert_eq!(row(&rows, "needed").items, ["libc.so.6"]);
        assert_eq!(row(&rows, "build-id").items, ["abcdef01"]);
        assert_eq!(row(&rows, "rwx").count, Some(0));
    }

    /// A static program with no symbols: nothing can say whether it has a
    /// canary, which is not the same as saying it has none.
    #[test]
    fn a_static_program_with_no_symbols_cannot_say_whether_it_has_a_canary() {
        let elf = Build::new(2, 62)
            .section(".text", 1, vec![0x90; 16])
            .segment(1, R | W | X, Covers::WholeFile)
            .segment(0x6474_e551, R | W | X, Covers::Nothing);
        let (rows, doc, mut ev) = rows_of(elf.build());
        evidence_is_where_it_says(&rows, &doc, &mut ev);
        let canary = row(&rows, "canary");
        assert_eq!((canary.state, canary.rating), ("no-symbols", Rating::Unknown));
        let fortify = row(&rows, "fortify");
        assert_eq!((fortify.state, fortify.rating), ("no-symbols", Rating::Unknown));
        assert_eq!(row(&rows, "nx").state, "off");
        assert_eq!(row(&rows, "pie").state, "exec");
        assert_eq!(row(&rows, "relro").state, "none");
        assert_eq!(row(&rows, "linking").state, "static");
        let rwx = row(&rows, "rwx");
        assert_eq!((rwx.state, rwx.count), ("present", Some(1)));
        // Not an x86 property note in sight, so both are off.
        assert_eq!(row(&rows, "ibt").state, "off");
    }

    /// An object file is not loaded, so the rows about loading have no
    /// answer. The rest still do.
    #[test]
    fn an_object_file_has_no_answer_for_how_it_is_loaded() {
        let elf = Build::new(1, 183).section(".text", 1, vec![0; 16]);
        let (rows, _, _) = rows_of(elf.build());
        for key in ["relro", "nx", "pie", "rpath", "runpath", "textrel", "rwx", "linking", "interpreter"] {
            let r = row(&rows, key);
            assert_eq!((r.state, r.rating), ("object", Rating::NotApplicable), "{key}");
        }
        // AArch64 asks about branch targets and pointer authentication.
        assert!(rows.iter().any(|r| r.key == "bti") && rows.iter().any(|r| r.key == "pac"));
        assert!(!rows.iter().any(|r| r.key == "ibt"));
    }

    /// A static symbol table is searched as the bytes of its string table,
    /// and what is found is the string, at the place the template reads it.
    #[test]
    fn the_static_symbols_are_searched_as_strings() {
        let (strtab, at) = fixture::strings(&["main", "__stack_chk_fail@GLIBC_2.4", "check.cfi", "other.cfi"]);
        let elf = Build::new(2, 62)
            .section(".strtab", 3, strtab)
            .linked(".symtab", 2, ".strtab", 24, fixture::symbols(&at))
            .section(".debug_info", 1, vec![0; 8])
            .segment(0x6474_e551, R | W, Covers::Nothing);
        let (rows, doc, mut ev) = rows_of(elf.build());
        evidence_is_where_it_says(&rows, &doc, &mut ev);
        let canary = row(&rows, "canary");
        assert_eq!(canary.state, "found");
        assert_eq!(canary.items, ["__stack_chk_fail@GLIBC_2.4"]);
        let e = &canary.evidence[0];
        assert_eq!(e.what, "string");
        assert_eq!(ev.node(&doc, &e.path).unwrap().value, Value::Str("__stack_chk_fail@GLIBC_2.4".into()));
        assert_eq!(e.offset_bits, (elf.offset_of(".strtab") + at[1]) * 8);
        let cfi = row(&rows, "cfi");
        assert_eq!((cfi.state, cfi.count), ("found", Some(2)));
        assert_eq!(cfi.items, ["check.cfi", "other.cfi"]);
        let symbols = row(&rows, "symbols");
        assert_eq!((symbols.state, symbols.count), ("present", Some(5)));
        assert_eq!(symbols.items, [".debug_info"]);
        // No dynamic symbols, so nothing says what the program imports.
        assert_eq!(row(&rows, "fortify").state, "no-symbols");
    }

    /// With the section headers cut off, the segments still carry the dynamic
    /// section and the notes, read out of the bytes. What needs a symbol
    /// table says it has no sections to find one by.
    #[test]
    fn a_program_with_no_section_headers_is_read_from_its_segments() {
        let mut elf = hardened();
        // The string table's address, which is its offset in this file.
        let strtab = elf.offset_of(".dynstr");
        let strsz = elf.sections[3].data.len() as u64;
        let i = elf.sections.iter().position(|s| s.name == ".dynamic").unwrap();
        let mut entries = fixture::dynamic(&[(5, strtab), (10, strsz)]);
        entries.extend_from_slice(&elf.sections[i].data);
        elf.sections[i].data = entries;
        elf.no_section_headers = true;
        let (rows, doc, mut ev) = rows_of(elf.build());
        evidence_is_where_it_says(&rows, &doc, &mut ev);
        assert_eq!(row(&rows, "relro").state, "full");
        assert_eq!(row(&rows, "nx").state, "on");
        assert_eq!(row(&rows, "pie").state, "pie");
        let needed = row(&rows, "needed");
        assert_eq!(needed.items, ["libc.so.6"]);
        assert!(needed.evidence[0].path.is_empty(), "no field covers the dynamic section without its header");
        assert_eq!(needed.evidence[0].offset_bits, (elf.offset_of(".dynamic") + 32) * 8);
        assert_eq!(row(&rows, "runpath").items, ["$ORIGIN/../lib", "lib"]);
        assert_eq!(row(&rows, "ibt").state, "on");
        for key in ["canary", "symbols", "fortify", "safestack", "cfi"] {
            let r = row(&rows, key);
            assert_eq!((r.state, r.rating), ("no-sections", Rating::Unknown), "{key}");
        }
    }
}
