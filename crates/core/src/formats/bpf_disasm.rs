//! Named disassembly for eBPF objects: instructions with the names the file
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
const STRTAB: i128 = 3;
const RELA: i128 = 4;
const REL: i128 = 9;

/// A section, as much of it as naming an instruction needs.
#[derive(Debug, Clone, Default)]
pub struct Section {
    pub name: String,
    pub kind: i128,
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
    /// The section it belongs to, as an index into [`Program::sections`].
    pub section: usize,
    /// Its offset within that section.
    pub value: u64,
}

/// What the tables of an object say, gathered once so a line can be written
/// without walking the file again.
#[derive(Debug, Clone, Default)]
pub struct Program {
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
                offset: int_field(ev, doc, &h, "offset")? as u64,
                size: int_field(ev, doc, &h, "size")? as u64,
            });
        }

        // Section names, from the table the header points at.
        let name_table = int_field(ev, doc, &p.header, "section_name_table")? as usize;
        let names = p.strings(ev, doc, &bodies, name_table)?;
        for i in 0..count {
            let at = int_field(ev, doc, &child(&headers, i), "name_offset")? as u64;
            p.sections[i].name = names.get(&at).cloned().unwrap_or_default();
        }

        // Symbols, and the string table each symbol table names.
        for i in 0..count {
            if p.sections[i].kind != SYMTAB {
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
                    name: strings.get(&at).cloned().unwrap_or_default(),
                    kind: int_field(ev, doc, &info, "type")?,
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

    /// The strings in a string table, by their offset into it, which is how
    /// every name in an ELF is written down.
    fn strings<S: Source>(
        &self,
        ev: &mut Evaluator,
        doc: &Document<S>,
        bodies: &[usize],
        section: usize,
    ) -> R<HashMap<u64, String>> {
        let mut out = HashMap::new();
        let Some(s) = self.sections.get(section) else { return Ok(out) };
        if s.kind != STRTAB {
            return Ok(out);
        }
        let body = child(bodies, section);
        let count = ev.node(doc, &body)?.child_count as usize;
        for k in 0..count {
            let n = ev.node(doc, &child(&body, k))?;
            if let Value::Str(text) = n.value {
                out.insert(n.offset_bits / 8 - s.offset, text);
            }
        }
        Ok(out)
    }

    /// One instruction as a line of text. `path` is the instruction in the
    /// tree, which is a child of a section's body.
    pub fn instruction_line<S: Source>(&self, ev: &mut Evaluator, doc: &Document<S>, path: &[usize]) -> R<String> {
        let (section, index) = match path.len() {
            n if n >= 2 => (path[n - 2], path[n - 1]),
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
        Ok(self.line(code, dst, src, off, imm, high, section, byte, index))
    }

    /// The text of one instruction, once its fields have been read.
    #[allow(clippy::too_many_arguments)]
    fn line(&self, code: u8, dst: u8, src: u8, off: i16, imm: i32, high: i32, section: usize, byte: u64, index: usize) -> String {
        let reg = |n: u8| format!("r{n}");
        match code {
            // A call: to a helper the kernel provides, to another program in
            // this object, or to a kernel function named by its BTF id.
            0x85 => match src {
                0 => match helper(imm) {
                    Some(name) => format!("call {name}"),
                    None => format!("call helper {imm}"),
                },
                1 => {
                    let target = (byte as i64 + 8 * (imm as i64 + 1)).max(0) as u64;
                    match self.functions.get(&(section, target)) {
                        Some(name) => format!("call {name}"),
                        None => format!("call instruction {}", index as i64 + imm as i64 + 1),
                    }
                }
                2 => format!("call kernel function #{imm}"),
                _ => format!("call {imm}"),
            },
            // A 64-bit load. What it loads is the constant in the two halves,
            // unless a relocation says the loader fills it in, in which case
            // the name of what it fills in is the useful half.
            0x18 => {
                let name = self.relocations.get(&(section, byte)).and_then(|s| self.symbols.get(*s)).map(|s| s.name.clone());
                let value = ((high as i64) << 32) | (imm as u32 as i64);
                match (src, name) {
                    (0, _) => format!("{} = {}", reg(dst), number(value)),
                    (1, Some(name)) => format!("{} = map {name}", reg(dst)),
                    (2, Some(name)) => format!("{} = map {name} + {}", reg(dst), number(high as i64)),
                    (3, Some(name)) => format!("{} = &{name}", reg(dst)),
                    (4, Some(name)) => format!("{} = &{name}", reg(dst)),
                    _ => self.substitute(code, dst, src, off, imm, high, index),
                }
            }
            _ => self.substitute(code, dst, src, off, imm, high, index),
        }
    }

    /// The standard's own description of the opcode, with this instruction's
    /// registers and numbers written into it. An opcode with no description is
    /// one the standard does not define, and says so.
    #[allow(clippy::too_many_arguments)]
    fn substitute(&self, code: u8, dst: u8, src: u8, off: i16, imm: i32, high: i32, index: usize) -> String {
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
            text = format!("{text} (instruction {})", index as i64 + off as i64 + 1);
        }
        text
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
                let line = self.instruction_line(ev, doc, &child(&body, k))?;
                out.push_str(&format!("{k:5}  {line}\n"));
            }
        }
        Ok(out)
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
