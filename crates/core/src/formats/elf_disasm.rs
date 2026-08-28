//! Named disassembly for ELF files: instructions with the names the file
//! knows, rather than the numbers it stores.
//!
//! This is not a template, for the same reason the wasm pass is not one. A
//! name in an ELF is an index into another section: a section is named by an
//! offset into the section name table, a symbol by an offset into a string
//! table, and the map an instruction loads by a relocation in a third section
//! that says which symbol patches which byte. The IR's expressions reach
//! siblings and ancestors, so none of that can be said where the bytes are
//! described. Reading the tables once into a [`Program`] and rendering a line
//! against them can.
//!
//! What a line says, best first:
//! 1. a `call` to a helper names the helper, from the kernel's own list;
//! 2. a `call` to another program in the same object names it, from the
//!    symbol table;
//! 3. a 64-bit load of a map names the map, from the relocation covering it;
//! 4. anything else is the standard's own description of the opcode with the
//!    registers and numbers filled in.

use std::collections::HashMap;

use super::bpf_opcodes::{form, helper};
use crate::document::Document;
use crate::eval::{EvalError, Evaluator, R, Value};
use crate::source::Source;

const SYMTAB: i128 = 2;
/// The symbols a program keeps for the loader rather than for a debugger. A
/// stripped file has these and nothing else, and they are what names the
/// functions a library offers.
const DYNSYM: i128 = 11;
const STRTAB: i128 = 3;
const RELA: i128 = 4;
const REL: i128 = 9;

/// A section, as much of it as naming an instruction needs.
#[derive(Debug, Clone, Default)]
pub struct Section {
    pub name: String,
    pub kind: i128,
    /// Where the section is when the program is running, which is what a
    /// symbol's address and a branch's target are counted in. Zero in an
    /// object file, where nothing has been given an address yet.
    pub addr: u64,
    /// Where the section starts in the file, so a relocation's offset into it
    /// can be turned into a place in the tree.
    pub offset: u64,
    pub size: u64,
}

/// A symbol: what it is called, and where it sits.
#[derive(Debug, Clone, Default)]
pub struct Symbol {
    pub name: String,
    pub kind: i128,
    /// How many bytes the symbol covers, which is what says whether an address
    /// is inside it or merely after it.
    pub size: u64,
    /// The section it belongs to, as an index into [`Program::sections`].
    pub section: usize,
    /// Its offset within that section.
    pub value: u64,
}

/// What the tables of an object say, gathered once so a line can be written
/// without walking the file again.
#[derive(Debug, Clone, Default)]
pub struct Program {
    /// Whether this is an object file rather than a program. It decides what a
    /// symbol's value means: an offset into its section here, an address
    /// anywhere it has been given one.
    relocatable: bool,
    /// Path to the header, which every other path here hangs off.
    header: Vec<usize>,
    pub sections: Vec<Section>,
    pub symbols: Vec<Symbol>,
    /// Which symbol patches which instruction: a section index and an offset
    /// into it, as a relocation writes them.
    relocations: HashMap<(usize, u64), usize>,
    /// Functions by where they start, for a call to name what it calls.
    functions: HashMap<(usize, u64), String>,
}

impl Program {
    /// Read the section headers, the names, the symbols and the relocations.
    pub fn read<S: Source>(ev: &mut Evaluator, doc: &Document<S>) -> R<Program> {
        let mut p = Program::default();
        p.header = named(ev, doc, &[], "header")?;
        // An object file's symbols are offsets into their sections; a
        // program's are addresses.
        p.relocatable = int_field(ev, doc, &p.header, "type")? == 1;
        let headers = {
            let at = named(ev, doc, &p.header, "section_headers")?;
            child(&at, 0)
        };
        let bodies = named(ev, doc, &p.header, "sections")?;
        let count = ev.node(doc, &headers)?.child_count as usize;
        for i in 0..count {
            let h = child(&headers, i);
            p.sections.push(Section {
                name: String::new(),
                kind: int_field(ev, doc, &h, "type")?,
                addr: int_field(ev, doc, &h, "address")? as u64,
                offset: int_field(ev, doc, &h, "offset")? as u64,
                size: int_field(ev, doc, &h, "size")? as u64,
            });
        }

        // Section names, from the table the header points at.
        let name_table = int_field(ev, doc, &p.header, "section_name_table")? as usize;
        let names = p.strings(ev, doc, &bodies, name_table)?;
        for i in 0..count {
            let at = int_field(ev, doc, &child(&headers, i), "name_offset")? as u64;
            p.sections[i].name = name_at(&names, at);
        }

        // Symbols, and the string table each symbol table names.
        for i in 0..count {
            if p.sections[i].kind != SYMTAB && p.sections[i].kind != DYNSYM {
                continue;
            }
            let strings = int_field(ev, doc, &child(&headers, i), "link")? as usize;
            let strings = p.strings(ev, doc, &bodies, strings)?;
            let body = child(&bodies, i);
            let entries = ev.node(doc, &body)?.child_count as usize;
            for k in 0..entries {
                let s = child(&body, k);
                let at = int_field(ev, doc, &s, "name_offset")? as u64;
                let info = named(ev, doc, &s, "info")?;
                let sym = Symbol {
                    name: name_at(&strings, at),
                    kind: int_field(ev, doc, &info, "type")?,
                    size: int_field(ev, doc, &s, "size")? as u64,
                    section: int_field(ev, doc, &s, "section_index")? as usize,
                    value: int_field(ev, doc, &s, "value")? as u64,
                };
                // A function symbol is what a call to another program in the
                // same object resolves to.
                if sym.kind == 2 && !sym.name.is_empty() {
                    p.functions.insert((sym.section, sym.value), sym.name.clone());
                }
                p.symbols.push(sym);
            }
        }

        // Relocations. Which section they apply to is the `info` field of the
        // relocation section's own header, which is how a `.rel.text` says it
        // patches `.text`.
        for i in 0..count {
            let kind = p.sections[i].kind;
            if kind != REL && kind != RELA {
                continue;
            }
            let target = int_field(ev, doc, &child(&headers, i), "info")? as usize;
            let body = child(&bodies, i);
            let entries = ev.node(doc, &body)?.child_count as usize;
            for k in 0..entries {
                let r = child(&body, k);
                let at = int_field(ev, doc, &r, "offset")? as u64;
                // A 32-bit object packs the symbol and the type into one word;
                // a 64-bit one writes them as two, which the template reads
                // apart.
                let symbol = match ev.child_named(doc, &r, "symbol")? {
                    Some(path) => int_at(ev, doc, &path)? as usize,
                    None => (int_field(ev, doc, &r, "info")? >> 8) as usize,
                };
                p.relocations.insert((target, at), symbol);
            }
        }
        Ok(p)
    }

    /// The strings in a string table, each with its offset into that table,
    /// which is how every name in an ELF is written down.
    fn strings<S: Source>(
        &self,
        ev: &mut Evaluator,
        doc: &Document<S>,
        bodies: &[usize],
        section: usize,
    ) -> R<Vec<(u64, String)>> {
        let mut out = Vec::new();
        let Some(s) = self.sections.get(section) else { return Ok(out) };
        if s.kind != STRTAB {
            return Ok(out);
        }
        let body = child(bodies, section);
        let count = ev.node(doc, &body)?.child_count as usize;
        for k in 0..count {
            let n = ev.node(doc, &child(&body, k))?;
            if let Value::Str(text) = n.value {
                out.push((n.offset_bits / 8 - s.offset, text));
            }
        }
        Ok(out)
    }

    /// One instruction as a line of text. `path` is the instruction in the
    /// tree, which is a child of a section's body.
    pub fn instruction_line<S: Source>(&self, ev: &mut Evaluator, doc: &Document<S>, path: &[usize]) -> R<String> {
        let section = match path.len() {
            n if n >= 2 => path[n - 2],
            _ => return Err(EvalError::Failed("instruction outside a section".into())),
        };
        let code = int_field(ev, doc, path, "opcode")? as u8;
        let dst = int_field(ev, doc, path, "dst")? as u8;
        let src = int_field(ev, doc, path, "src")? as u8;
        let off = int_field(ev, doc, path, "offset")? as i16;
        let imm = int_field(ev, doc, path, "imm")? as i32;
        let wide = named(ev, doc, path, "wide")?;
        let high = match ev.child_named(doc, &wide, "imm_high")? {
            Some(p) => int_at(ev, doc, &p)? as i32,
            None => 0,
        };
        // Where this instruction is inside its section, which is what a
        // relocation and a call target are both counted in.
        let at = ev.node(doc, path)?.offset_bits / 8;
        let start = self.sections.get(section).map(|s| s.offset).unwrap_or(0);
        let byte = at.saturating_sub(start);
        // A distance in this instruction set is counted in eight-byte slots,
        // not in instructions: the load of a 64-bit immediate takes two of
        // them and is one instruction. So a jump is read against where the
        // instruction sits, not against how many came before it.
        Ok(self.line(code, dst, src, off, imm, high, section, byte, byte / 8))
    }

    /// What a relocation says fills in this instruction, by name.
    ///
    /// A symbol with no name of its own stands for a whole section, which is
    /// what a call to a function the compiler kept to itself is written
    /// against. The function that section starts with is the name a reader
    /// wants; the section's own name is the fallback.
    fn patched(&self, section: usize, byte: u64) -> Option<String> {
        let symbol = self.symbols.get(*self.relocations.get(&(section, byte))?)?;
        if !symbol.name.is_empty() {
            return Some(symbol.name.clone());
        }
        if let Some(name) = self.functions.get(&(symbol.section, symbol.value)) {
            return Some(name.clone());
        }
        self.sections.get(symbol.section).map(|s| s.name.clone()).filter(|n| !n.is_empty())
    }

    /// The text of one instruction, once its fields have been read.
    #[allow(clippy::too_many_arguments)]
    fn line(&self, code: u8, dst: u8, src: u8, off: i16, imm: i32, high: i32, section: usize, byte: u64, slot: u64) -> String {
        let reg = |n: u8| format!("r{n}");
        // What a relocation says patches this instruction, if one does. Both
        // a call to another program and a load of a map are written as a
        // number the loader replaces, and the relocation is what says which
        // symbol it replaces it with.
        let patched = self.patched(section, byte);
        match code {
            // A call: to a helper the kernel provides, to another program in
            // this object, or to a kernel function named by its BTF id.
            0x85 if patched.is_some() => format!("call {}", patched.unwrap_or_default()),
            0x85 => match src {
                0 => match helper(imm) {
                    Some(name) => format!("call {name}"),
                    None => format!("call helper {imm}"),
                },
                1 => {
                    let target = (byte as i64 + 8 * (imm as i64 + 1)).max(0) as u64;
                    match self.functions.get(&(section, target)) {
                        Some(name) => format!("call {name}"),
                        None => format!("call instruction {}", slot as i64 + imm as i64 + 1),
                    }
                }
                2 => format!("call kernel function #{imm}"),
                _ => format!("call {imm}"),
            },
            // A 64-bit load. What it loads is the constant in the two halves,
            // unless a relocation says the loader fills it in, in which case
            // the name of what it fills in is the useful half.
            0x18 => {
                let name = patched;
                let value = ((high as i64) << 32) | (imm as u32 as i64);
                match (name, src) {
                    // A file that has been through the loader says in the
                    // source register that the number is a map; one that has
                    // not says nothing and leaves a relocation instead. Both
                    // mean the same thing, and the name is what to show.
                    (Some(name), 1 | 2) => format!("{} = map {name}", reg(dst)),
                    // The number left in the instruction is an offset into
                    // whatever the relocation names, which is how a load of a
                    // variable in a section is written.
                    (Some(name), _) if value != 0 => format!("{} = {name} + {}", reg(dst), number(value)),
                    (Some(name), _) => format!("{} = {name}", reg(dst)),
                    (None, 0) => format!("{} = {}", reg(dst), number(value)),
                    (None, _) => self.substitute(code, dst, src, off, imm, high, slot),
                }
            }
            _ => self.substitute(code, dst, src, off, imm, high, slot),
        }
    }

    /// The standard's own description of the opcode, with this instruction's
    /// registers and numbers written into it. An opcode with no description is
    /// one the standard does not define, and says so.
    #[allow(clippy::too_many_arguments)]
    fn substitute(&self, code: u8, dst: u8, src: u8, off: i16, imm: i32, high: i32, slot: u64) -> String {
        let Some(form) = form(code, src, off, imm) else {
            return format!("(unknown opcode 0x{code:02x})");
        };
        let value = ((high as i64) << 32) | (imm as u32 as i64);
        let mut text = form
            .replace("next_imm", &number(high as i64))
            .replace("+offset", &signed(off as i64))
            .replace("offset", &number(off as i64))
            .replace("imm", &number(imm as i64))
            .replace("dst", &format!("r{dst}"))
            .replace("src", &format!("r{src}"));
        if code == 0x18 {
            text = format!("r{dst} = {}", number(value));
        }
        // A jump says where it goes as a distance. Which instruction that is
        // is the thing a reader wants and the thing the file does not write.
        if form.contains("goto +offset") {
            text = format!("{text} (instruction {})", slot as i64 + off as i64 + 1);
        }
        text
    }

    /// The address a symbol stands for. An object file writes an offset into
    /// a section, and a program writes the address itself.
    fn symbol_address(&self, symbol: &Symbol) -> u64 {
        match self.relocatable {
            true => self.sections.get(symbol.section).map_or(0, |s| s.addr) + symbol.value,
            false => symbol.value,
        }
    }

    /// What is at this address, by name: a function or a piece of data the
    /// symbol table names, and how far into it the address is.
    ///
    /// A symbol with no size of its own covers only where it starts. Files
    /// written by hand and files written by an assembler both do that, and
    /// treating such a symbol as covering everything after it would name the
    /// whole rest of the section after the first label in it.
    pub fn name_at_address(&self, addr: u64) -> Option<String> {
        let mut best: Option<(&Symbol, u64)> = None;
        for symbol in self.symbols.iter().filter(|s| !s.name.is_empty() && (s.kind == 1 || s.kind == 2)) {
            let start = self.symbol_address(symbol);
            if addr < start {
                continue;
            }
            let into = addr - start;
            if into > 0 && into >= symbol.size {
                continue;
            }
            if best.is_none_or(|(_, was)| into < was) {
                best = Some((symbol, into));
            }
        }
        let (symbol, into) = best?;
        Some(match into {
            0 => symbol.name.clone(),
            n => format!("{}+0x{n:x}", symbol.name),
        })
    }

    /// One machine instruction with the name of whatever it branches to
    /// written in place of the distance the file holds. `None` when the
    /// instruction goes nowhere, or goes somewhere nothing has a name for: the
    /// row the template produced is already right, and a name is the only
    /// thing worth replacing it for.
    pub fn machine_line<S: Source>(&self, ev: &mut Evaluator, doc: &Document<S>, path: &[usize]) -> R<Option<String>> {
        let section = match path.len() {
            n if n >= 2 => path[n - 2],
            _ => return Ok(None),
        };
        let node = ev.node(doc, path)?;
        let Some(isa) = crate::code::Isa::named(&node.type_name) else { return Ok(None) };
        let Value::Str(text) = node.value else { return Ok(None) };
        let len = (node.size_bits / 8) as usize;
        let Some(rel) = crate::code::relative_target(isa, &text, len) else { return Ok(None) };
        // Where this instruction is when the program is running, which is what
        // a branch is counted in and what a symbol is written in.
        let Some(s) = self.sections.get(section) else { return Ok(None) };
        let addr = s.addr + (node.offset_bits / 8).saturating_sub(s.offset);
        let target = addr.checked_add_signed(rel).unwrap_or(addr);
        let Some(name) = self.name_at_address(target) else { return Ok(None) };
        // The distance is what the name replaces: `call $+0x13a` is what the
        // file says, and `call memcpy` is what it means.
        let at = text.find('$').expect("a target came from one");
        let end = text[at..].find(|c: char| c != '$' && c != '+' && c != '-' && c != 'x' && !c.is_ascii_hexdigit());
        let rest = end.map(|e| &text[at + e..]).unwrap_or("");
        Ok(Some(format!("{}{name}{rest}", &text[..at])))
    }

    /// Every instruction of every executable section, as text. What a person
    /// would ask a disassembler for.
    pub fn listing<S: Source>(&self, ev: &mut Evaluator, doc: &Document<S>) -> R<String> {
        let mut out = String::new();
        let bodies = named(ev, doc, &self.header, "sections")?;
        for (i, s) in self.sections.iter().enumerate() {
            let body = child(&bodies, i);
            let n = ev.node(doc, &body)?;
            if n.type_name != "BpfInsn[]" {
                continue;
            }
            out.push_str(&format!("{}:\n", s.name));
            for k in 0..n.child_count as usize {
                let at = child(&body, k);
                let slot = (ev.node(doc, &at)?.offset_bits / 8).saturating_sub(s.offset) / 8;
                let line = self.instruction_line(ev, doc, &at)?;
                out.push_str(&format!("{slot:5}  {line}\n"));
            }
        }
        Ok(out)
    }
}

/// The name at an offset into a string table. An offset need not be the start
/// of a string: a linker that has written `.relsocket/main` has `socket/main`
/// four bytes into it, and writes that offset rather than the name again.
fn name_at(table: &[(u64, String)], at: u64) -> String {
    match table.iter().rev().find(|(start, text)| *start <= at && at <= start + text.len() as u64) {
        Some((start, text)) => text[(at - start) as usize..].to_string(),
        None => String::new(),
    }
}

/// A number as a reader wants it: small ones in decimal, since they are
/// counts and offsets, and large ones in hexadecimal, since they are addresses
/// and masks.
fn number(v: i64) -> String {
    if v.abs() < 0x1_0000 { format!("{v}") } else { format!("0x{:x}", v as u64) }
}

/// A distance, which is only worth reading with its sign.
fn signed(v: i64) -> String {
    if v < 0 { format!("-{}", -v) } else { format!("+{v}") }
}

fn child(path: &[usize], i: usize) -> Vec<usize> {
    let mut p = path.to_vec();
    p.push(i);
    p
}

/// The child a structure calls `name`, or an error naming what was missing.
fn named<S: Source>(ev: &mut Evaluator, doc: &Document<S>, path: &[usize], name: &str) -> R<Vec<usize>> {
    match ev.child_named(doc, path, name)? {
        Some(p) => Ok(p),
        None => Err(EvalError::Failed(format!("no field {name} at {path:?}"))),
    }
}

fn int_at<S: Source>(ev: &mut Evaluator, doc: &Document<S>, path: &[usize]) -> R<i128> {
    match ev.node(doc, path)?.value {
        Value::UInt(v) => Ok(v as i128),
        Value::Int(v) => Ok(v),
        Value::Enum { raw, .. } => Ok(raw),
        other => Err(EvalError::Failed(format!("{path:?} is not a number: {other:?}"))),
    }
}

/// The number a named field of a structure holds. Two steps, because finding
/// the field and reading it both want the evaluator.
fn int_field<S: Source>(ev: &mut Evaluator, doc: &Document<S>, path: &[usize], name: &str) -> R<i128> {
    let p = named(ev, doc, path, name)?;
    int_at(ev, doc, &p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval::Evaluator;
    use crate::formats::bpf;
    use crate::source::MemSource;

    fn insn(op: u8, dst: u8, src: u8, off: i16, imm: i32) -> Vec<u8> {
        let mut v = vec![op, (src << 4) | dst];
        v.extend_from_slice(&off.to_le_bytes());
        v.extend_from_slice(&imm.to_le_bytes());
        v
    }

    /// A 64-bit section header. The fields are in the order the standard puts
    /// them in, which is what the template reads.
    #[allow(clippy::too_many_arguments)]
    fn shdr(name: u32, kind: u32, flags: u64, offset: u64, size: u64, link: u32, info: u32, entry: u64) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&name.to_le_bytes());
        v.extend_from_slice(&kind.to_le_bytes());
        v.extend_from_slice(&flags.to_le_bytes());
        v.extend_from_slice(&0u64.to_le_bytes());
        v.extend_from_slice(&offset.to_le_bytes());
        v.extend_from_slice(&size.to_le_bytes());
        v.extend_from_slice(&link.to_le_bytes());
        v.extend_from_slice(&info.to_le_bytes());
        v.extend_from_slice(&8u64.to_le_bytes());
        v.extend_from_slice(&entry.to_le_bytes());
        v
    }

    fn symbol(name: u32, info: u8, section: u16, value: u64, size: u64) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&name.to_le_bytes());
        v.push(info);
        v.push(0);
        v.extend_from_slice(&section.to_le_bytes());
        v.extend_from_slice(&value.to_le_bytes());
        v.extend_from_slice(&size.to_le_bytes());
        v
    }

    /// An object of the shape a compiler writes: a program section, the map it
    /// looks something up in, the relocation that ties the two together, and
    /// the tables that give everything a name.
    fn object() -> Vec<u8> {
        let mut text = Vec::new();
        text.extend(insn(0xbf, 6, 1, 0, 0)); // r6 = r1
        text.extend(insn(0x18, 1, 1, 0, 0)); // r1 = the map, once the loader fills it in
        text.extend(insn(0x00, 0, 0, 0, 0)); // the second half of that load
        text.extend(insn(0x85, 0, 0, 0, 1)); // call bpf_map_lookup_elem
        text.extend(insn(0x15, 0, 0, 3, 0)); // if r0 == 0 goto +3, over the load below
        text.extend(insn(0x18, 2, 0, 0, 7)); // r2 = 7, in two slots
        text.extend(insn(0x00, 0, 0, 0, 0)); // the second half of that load
        text.extend(insn(0xbf, 0, 6, 0, 0)); // r0 = r6
        text.extend(insn(0x95, 0, 0, 0, 0)); // exit
        text.extend(insn(0x18, 3, 0, 0, 0)); // r3 = the same map, as a compiler writes it
        text.extend(insn(0x00, 0, 0, 0, 0)); // the second half of that load
        text.extend(insn(0x85, 0, 1, 0, -1)); // call a function the compiler kept to itself

        let maps = vec![0u8; 32];
        let mut rel = Vec::new();
        rel.extend_from_slice(&8u64.to_le_bytes()); // the immediate of the load
        rel.extend_from_slice(&1u32.to_le_bytes()); // R_BPF_64_64
        rel.extend_from_slice(&1u32.to_le_bytes()); // symbol 1
        rel.extend_from_slice(&72u64.to_le_bytes()); // the load at the end
        rel.extend_from_slice(&1u32.to_le_bytes());
        rel.extend_from_slice(&1u32.to_le_bytes());
        rel.extend_from_slice(&88u64.to_le_bytes()); // the call after it
        rel.extend_from_slice(&10u32.to_le_bytes()); // R_BPF_64_32
        rel.extend_from_slice(&3u32.to_le_bytes()); // the section symbol below

        let strtab = b"\0counter_map\0xdp_prog\0".to_vec();
        let mut symtab = symbol(0, 0, 0, 0, 0);
        symtab.extend(symbol(1, 0x11, 3, 0, 32)); // global object, in the maps section
        symtab.extend(symbol(13, 0x12, 1, 0, text.len() as u64)); // global function, in the code
        symtab.extend(symbol(0, 3, 1, 0, 0)); // the code section itself, which has no name of its own
        let shstrtab = b"\0xdp\0.relxdp\0.maps\0.symtab\0.strtab\0.shstrtab\0".to_vec();

        // Everything after the header, in order, each section aligned to eight.
        let mut body = Vec::new();
        let mut offsets = Vec::new();
        for part in [&text, &rel, &maps, &symtab, &strtab, &shstrtab] {
            while (64 + body.len()) % 8 != 0 {
                body.push(0);
            }
            offsets.push(64 + body.len() as u64);
            body.extend_from_slice(part);
        }
        while (64 + body.len()) % 8 != 0 {
            body.push(0);
        }
        let shoff = 64 + body.len() as u64;

        let mut v = b"\x7fELF".to_vec();
        v.extend_from_slice(&[2, 1, 1, 0, 0]);
        v.extend_from_slice(&[0; 7]);
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&247u16.to_le_bytes());
        v.extend_from_slice(&1u32.to_le_bytes());
        v.extend_from_slice(&0u64.to_le_bytes());
        v.extend_from_slice(&0u64.to_le_bytes());
        v.extend_from_slice(&shoff.to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&64u16.to_le_bytes());
        v.extend_from_slice(&0u16.to_le_bytes());
        v.extend_from_slice(&0u16.to_le_bytes());
        v.extend_from_slice(&64u16.to_le_bytes());
        v.extend_from_slice(&7u16.to_le_bytes());
        v.extend_from_slice(&6u16.to_le_bytes()); // names are in section 6
        v.extend_from_slice(&body);
        v.extend(shdr(0, 0, 0, 0, 0, 0, 0, 0));
        v.extend(shdr(1, 1, 6, offsets[0], text.len() as u64, 0, 0, 0)); // xdp
        v.extend(shdr(5, 9, 0x40, offsets[1], rel.len() as u64, 4, 1, 16)); // .relxdp
        v.extend(shdr(13, 1, 3, offsets[2], maps.len() as u64, 0, 0, 0)); // .maps
        v.extend(shdr(19, 2, 0, offsets[3], symtab.len() as u64, 5, 1, 24)); // .symtab
        v.extend(shdr(27, 3, 0, offsets[4], strtab.len() as u64, 0, 0, 0)); // .strtab
        v.extend(shdr(35, 3, 0, offsets[5], shstrtab.len() as u64, 0, 0, 0)); // .shstrtab
        v
    }

    fn listing() -> String {
        let d = Document::new(MemSource(object()));
        let mut ev = Evaluator::new(bpf());
        let p = Program::read(&mut ev, &d).unwrap();
        p.listing(&mut ev, &d).unwrap()
    }

    /// An object for a real machine, with a call to a function the symbol
    /// table names and a jump into the middle of that function.
    fn x86_object() -> Vec<u8> {
        let mut text = Vec::new();
        text.extend([0xe8, 0x04, 0x00, 0x00, 0x00]); // call the four bytes past it, which is `work`
        text.extend([0xeb, 0x07]); // jmp +7, into the middle of it
        text.extend([0x90, 0x90]); // padding between the two
        text.extend([0x55, 0x48, 0x89, 0xe5, 0x5d, 0xc3]); // work: push rbp, mov, pop, ret

        let strtab = b"\0work\0".to_vec();
        let mut symtab = symbol(0, 0, 0, 0, 0);
        symtab.extend(symbol(1, 0x12, 1, 9, 6)); // a function, nine bytes in, six long
        let shstrtab = b"\0.text\0.symtab\0.strtab\0".to_vec();

        let mut body = Vec::new();
        let mut offsets = Vec::new();
        for part in [&text, &symtab, &strtab, &shstrtab] {
            while (64 + body.len()) % 8 != 0 {
                body.push(0);
            }
            offsets.push(64 + body.len() as u64);
            body.extend_from_slice(part);
        }
        while (64 + body.len()) % 8 != 0 {
            body.push(0);
        }
        let shoff = 64 + body.len() as u64;

        let mut v = b"\x7fELF".to_vec();
        v.extend_from_slice(&[2, 1, 1, 0, 0]);
        v.extend_from_slice(&[0; 7]);
        v.extend_from_slice(&1u16.to_le_bytes()); // relocatable
        v.extend_from_slice(&62u16.to_le_bytes()); // x86-64
        v.extend_from_slice(&1u32.to_le_bytes());
        v.extend_from_slice(&0u64.to_le_bytes());
        v.extend_from_slice(&0u64.to_le_bytes());
        v.extend_from_slice(&shoff.to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&64u16.to_le_bytes());
        v.extend_from_slice(&0u16.to_le_bytes());
        v.extend_from_slice(&0u16.to_le_bytes());
        v.extend_from_slice(&64u16.to_le_bytes());
        v.extend_from_slice(&5u16.to_le_bytes());
        v.extend_from_slice(&4u16.to_le_bytes()); // names are in section 4
        v.extend_from_slice(&body);
        v.extend(shdr(0, 0, 0, 0, 0, 0, 0, 0));
        v.extend(shdr(1, 1, 6, offsets[0], text.len() as u64, 0, 0, 0)); // .text
        v.extend(shdr(7, 2, 0, offsets[1], symtab.len() as u64, 3, 1, 24)); // .symtab
        v.extend(shdr(15, 3, 0, offsets[2], strtab.len() as u64, 0, 0, 0)); // .strtab
        v.extend(shdr(23, 3, 0, offsets[3], shstrtab.len() as u64, 0, 0, 0)); // .shstrtab
        v
    }

    fn named_line(index: usize) -> Option<String> {
        let d = Document::new(MemSource(x86_object()));
        let mut ev = Evaluator::new(crate::formats::elf());
        let p = Program::read(&mut ev, &d).unwrap();
        p.machine_line(&mut ev, &d, &[7, 15, 1, index]).unwrap()
    }

    #[test]
    fn a_call_says_the_name_of_what_it_calls() {
        assert_eq!(named_line(0).as_deref(), Some("call work"));
    }

    #[test]
    fn a_jump_into_a_function_says_how_far_into_it() {
        assert_eq!(named_line(1).as_deref(), Some("jmp work+0x5"));
    }

    #[test]
    fn an_instruction_that_goes_nowhere_keeps_the_row_it_had() {
        assert_eq!(named_line(2), None);
    }

    #[test]
    fn the_tables_name_the_sections_and_the_symbols() {
        let d = Document::new(MemSource(object()));
        let mut ev = Evaluator::new(bpf());
        let p = Program::read(&mut ev, &d).unwrap();
        let names: Vec<&str> = p.sections.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["", "xdp", ".relxdp", ".maps", ".symtab", ".strtab", ".shstrtab"]);
        let symbols: Vec<&str> = p.symbols.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(symbols, ["", "counter_map", "xdp_prog", ""]);
    }

    #[test]
    fn a_load_of_a_map_names_the_map() {
        let text = listing();
        // With the source register saying the number is a map.
        assert!(text.contains("r1 = map counter_map"), "{text}");
        // And without, which is what a compiler writes and a relocation says.
        assert!(text.contains("r3 = counter_map"), "{text}");
    }

    #[test]
    fn a_call_names_the_helper_it_calls() {
        assert!(listing().contains("call bpf_map_lookup_elem"), "{}", listing());
    }

    #[test]
    fn a_call_a_relocation_stands_for_names_what_it_calls() {
        // Written against the section rather than the function, which is what
        // a compiler does for a function it kept to itself. The name comes
        // from the function that section starts with.
        assert!(listing().contains("call xdp_prog"), "{}", listing());
    }

    #[test]
    fn a_jump_says_which_instruction_it_goes_to() {
        // The jump is in slot 4 and goes three slots past the next one, which
        // lands on the exit in slot 8. Counting instructions rather than slots
        // would land a slot short: the load between them takes two.
        let text = listing();
        assert!(text.contains("if r0 == 0 goto +3 (instruction 8)"), "{text}");
        assert!(text.contains("    8  return"), "{text}");
    }

    #[test]
    fn an_ordinary_instruction_is_the_standards_own_description() {
        let text = listing();
        assert!(text.contains("r6 = r1"), "{text}");
        assert!(text.contains("return"), "{text}");
    }
}
