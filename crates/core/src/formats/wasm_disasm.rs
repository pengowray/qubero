//! Named disassembly for wasm: the code section as text, with indices resolved
//! to the names a person would recognise.
//!
//! This is not a template. A template describes bytes where they sit, and the
//! IR's expressions reach siblings and ancestors only, so `call 47` cannot look
//! up function 47 in another section from inside the code section. Resolving a
//! name is a pass over the parsed tree instead: read the import, function,
//! export and `name` sections once into a [`Module`], then render a body
//! against it.
//!
//! Where a name comes from, best first:
//! 1. the `name` custom section, which is what a debug build carries;
//! 2. the export table, which names whatever the module makes public;
//! 3. `module.field` for an import, which is always present;
//! 4. `funcN`, which is the index and says so.

use std::collections::HashMap;
use std::fmt::Write as _;

use super::wasm_opcodes::thread_align;
use crate::document::Document;
use crate::eval::{EvalError, Evaluator, R, Value};
use crate::source::Source;

/// The alignment each load and store has of its own accord, as the log2 the
/// format stores, for opcodes 0x28 to 0x3e in order. An immediate that matches
/// is the default and is not worth printing.
const NATURAL_ALIGN: [u8; 23] = [
    2, 3, 2, 3, // i32.load, i64.load, f32.load, f64.load
    0, 0, 1, 1, // i32.load8_s/u, i32.load16_s/u
    0, 0, 1, 1, 2, 2, // i64.load8_s/u, i64.load16_s/u, i64.load32_s/u
    2, 3, 2, 3, // i32.store, i64.store, f32.store, f64.store
    0, 1, // i32.store8, i32.store16
    0, 1, 2, // i64.store8, i64.store16, i64.store32
];

/// Section ids this pass looks for. The rest it walks past.
const IMPORT: i128 = 2;
const EXPORT: i128 = 7;
const CODE: i128 = 10;
const CUSTOM: i128 = 0;

/// What the sections say about a module, gathered once so a body can be read
/// without walking the file again.
#[derive(Debug, Clone, Default)]
pub struct Module {
    /// A name per function, in index order: imports first, then the functions
    /// the module defines. This is the order `call` counts in.
    names: Vec<String>,
    /// How many of those are imports, which is the index of the first defined
    /// function.
    imported: usize,
    /// Path to each defined function's instruction list, in the same order as
    /// the code section holds them.
    bodies: Vec<Vec<usize>>,
    /// Names for globals, from the export table and the `name` section.
    globals: HashMap<u32, String>,
}

impl Module {
    /// Read the sections that carry names and code.
    pub fn read<S: Source>(ev: &mut Evaluator, doc: &Document<S>) -> R<Module> {
        let mut m = Module::default();
        let mut from_name_section: HashMap<u32, String> = HashMap::new();
        let mut global_exports: HashMap<u32, String> = HashMap::new();
        let mut defined = 0usize;

        let sections = ev.node(doc, &[2])?.child_count;
        for s in 0..sections as usize {
            let id = match ev.node(doc, &[2, s, 0])?.value {
                Value::Enum { raw, .. } => raw,
                _ => continue,
            };
            match id {
                IMPORT => {
                    let count = ev.node(doc, &[2, s, 2, 1])?.child_count;
                    for k in 0..count as usize {
                        let module = str_at(ev, doc, &[2, s, 2, 1, k, 1])?;
                        let field = str_at(ev, doc, &[2, s, 2, 1, k, 3])?;
                        let kind = int_at(ev, doc, &[2, s, 2, 1, k, 4])?;
                        // Only function imports take a slot in the function
                        // index space; a memory or a global is counted apart.
                        if kind == 0 {
                            m.names.push(format!("{module}.{field}"));
                        }
                    }
                    m.imported = m.names.len();
                }
                EXPORT => {
                    let count = ev.node(doc, &[2, s, 2, 1])?.child_count;
                    for k in 0..count as usize {
                        let name = str_at(ev, doc, &[2, s, 2, 1, k, 1])?;
                        let kind = int_at(ev, doc, &[2, s, 2, 1, k, 2])?;
                        let index = int_at(ev, doc, &[2, s, 2, 1, k, 3])? as u32;
                        match kind {
                            0 => m.exported_func(index, name),
                            3 => {
                                global_exports.insert(index, name);
                            }
                            _ => {}
                        }
                    }
                }
                CODE => {
                    let count = ev.node(doc, &[2, s, 2, 1])?.child_count;
                    defined = count as usize;
                    for k in 0..count as usize {
                        // entries[k].body.code
                        m.bodies.push(vec![2, s, 2, 1, k, 1, 2]);
                    }
                }
                CUSTOM => {
                    if str_at(ev, doc, &[2, s, 2, 1])? == "name" {
                        let n = ev.node(doc, &[2, s, 2, 2])?;
                        let bytes = read(doc, n.offset_bits / 8, n.size_bits / 8)?;
                        read_name_section(&bytes, &mut from_name_section);
                    }
                }
                _ => {}
            }
        }

        // Every defined function needs a slot before the better names can be
        // written into it, since the export table may have arrived first.
        let total = m.imported + defined;
        for i in m.names.len()..total {
            m.names.push(format!("func{i}"));
        }
        for i in m.imported..total {
            if m.names[i].is_empty() {
                m.names[i] = format!("func{i}");
            }
        }
        // The name section wins over an export name, and over an import's
        // module.field, because it is what the author actually called it.
        for (i, name) in from_name_section {
            if let Some(slot) = m.names.get_mut(i as usize) {
                *slot = name;
            }
        }
        m.globals = global_exports;
        Ok(m)
    }

    /// Record an export name against a function, leaving room for the imports
    /// and defined functions whose slots may not exist yet.
    fn exported_func(&mut self, index: u32, name: String) {
        let i = index as usize;
        if self.names.len() <= i {
            self.names.resize(i + 1, String::new());
        }
        self.names[i] = name;
    }

    /// How many functions the module defines, which is how many bodies there
    /// are to disassemble.
    pub fn func_count(&self) -> usize {
        self.bodies.len()
    }

    /// The index of the first function the module defines. Anything below it is
    /// an import and has no body.
    pub fn first_defined(&self) -> usize {
        self.imported
    }

    /// What to call function `index`, counting imports first.
    pub fn func_name(&self, index: u32) -> String {
        self.names.get(index as usize).cloned().unwrap_or_else(|| format!("func{index}"))
    }

    /// Disassemble the `n`th function the module defines. `n` counts from the
    /// start of the code section, so its index in the module is
    /// `n + first_defined()`.
    pub fn disassemble<S: Source>(&self, ev: &mut Evaluator, doc: &Document<S>, n: usize) -> R<String> {
        let path = match self.bodies.get(n) {
            Some(p) => p.clone(),
            None => return Err(EvalError::Failed(format!("no function {n}: the code section has {} functions", self.bodies.len()))),
        };
        let mut out = String::new();
        let _ = writeln!(out, "(func ${}", self.func_name((self.imported + n) as u32));
        let count = ev.node(doc, &path)?.child_count;
        let mut depth = 1usize;
        for i in 0..count as usize {
            let mut p = path.clone();
            p.push(i);
            let line = match self.instruction(ev, doc, &p) {
                Ok(line) => line,
                // A body the template could not finish reading is the SIMD gap,
                // not a broken file. Say where it stopped rather than printing
                // whatever the bytes happened to decode to.
                Err(EvalError::Failed(why)) => {
                    let _ = writeln!(out, "{:indent$};; disassembly stopped: {why}", "", indent = depth * 2);
                    break;
                }
                Err(e) => return Err(e),
            };
            // `end` and `else` close the block they belong to, so they sit at
            // the depth of the instruction that opened it.
            if matches!(line.0.as_str(), "end" | "else") {
                depth = depth.saturating_sub(1);
            }
            let _ = writeln!(out, "{:indent$}{}", "", line.1, indent = depth * 2);
            if matches!(line.0.as_str(), "block" | "loop" | "if" | "else") {
                depth += 1;
            }
        }
        out.push_str(")\n");
        Ok(out)
    }

    /// One instruction at `path`, as the text a listing row shows: the
    /// mnemonic and its immediate, with any index that names something given
    /// that name.
    pub fn instruction_line<S: Source>(&self, ev: &mut Evaluator, doc: &Document<S>, path: &[usize]) -> R<String> {
        Ok(self.instruction(ev, doc, path)?.1)
    }

    /// One instruction as its mnemonic and its full text.
    fn instruction<S: Source>(&self, ev: &mut Evaluator, doc: &Document<S>, path: &[usize]) -> R<(String, String)> {
        let mut op_path = path.to_vec();
        op_path.push(0);
        let op = ev.node(doc, &op_path)?;
        let (raw, name) = match op.value {
            Value::Enum { raw, name, .. } => (raw, name),
            _ => return Err(EvalError::Failed("instruction has no opcode".into())),
        };
        let mnemonic = match name {
            Some(n) => n,
            None => return Err(EvalError::Failed(format!("unknown opcode 0x{raw:02x}"))),
        };
        let mut imm_path = path.to_vec();
        imm_path.push(1);
        let imm = ev.node(doc, &imm_path)?;
        // An opcode with no immediate selects a zero-length field, and the
        // mnemonic is the whole instruction.
        if imm.size_bits == 0 && !imm.composite {
            return Ok((mnemonic.clone(), mnemonic));
        }
        let args = self.immediate(ev, doc, &imm_path, raw, &imm.value)?;
        // A prefix byte is not part of what the instruction is called: the
        // group's own table names it, and `0xfd v128.load` names it twice.
        let text = if raw == 0xfc || raw == 0xfd || raw == 0xfe {
            args
        } else if args.is_empty() {
            mnemonic.clone()
        } else {
            format!("{mnemonic} {args}")
        };
        Ok((mnemonic, text))
    }

    /// The immediate as text. Which form it takes is the opcode's business:
    /// a call names a function, a load spells out its alignment and offset.
    fn immediate<S: Source>(
        &self,
        ev: &mut Evaluator,
        doc: &Document<S>,
        path: &[usize],
        op: i128,
        value: &Value,
    ) -> R<String> {
        Ok(match op {
            // call
            0x10 => format!("${}", self.func_name(scalar(value) as u32)),
            // ref.func
            0xd2 => format!("${}", self.func_name(scalar(value) as u32)),
            // call_indirect: a type index and the table it looks in
            0x11 => {
                let ty = int_at(ev, doc, &child(path, 0))?;
                let table = int_at(ev, doc, &child(path, 1))?;
                format!("(type {ty}) (table {table})")
            }
            // global.get, global.set
            0x23 | 0x24 => {
                let i = scalar(value) as u32;
                match self.globals.get(&i) {
                    Some(n) => format!("${n}"),
                    None => i.to_string(),
                }
            }
            // block, loop, if: an empty result type adds nothing to read
            0x02 | 0x03 | 0x04 => match value {
                Value::Enum { name: Some(n), .. } if n == "empty" => String::new(),
                Value::Enum { name: Some(n), .. } => format!("(result {n})"),
                v => format!("(type {})", scalar(v)),
            },
            // br_table
            0x0e => {
                let count = ev.node(doc, &child(path, 1))?.child_count;
                let mut labels = Vec::with_capacity(count as usize + 1);
                for i in 0..count as usize {
                    let mut p = child(path, 1);
                    p.push(i);
                    labels.push(int_at(ev, doc, &p)?.to_string());
                }
                labels.push(int_at(ev, doc, &child(path, 2))?.to_string());
                labels.join(" ")
            }
            // Every load and store. Both parts are left out when they say
            // nothing: a zero offset, and an alignment that is the one the
            // access has anyway. Naming those would put `align=4` on most of
            // the instructions in a file and mean nothing by it.
            0x28..=0x3e => mem_text(ev, doc, path, NATURAL_ALIGN[(op - 0x28) as usize])?,
            // The prefixed groups name themselves in their own table.
            0xfc | 0xfd | 0xfe => self.prefixed(ev, doc, path, op)?,
            // f32.const, f64.const. `scalar` would truncate these to an
            // integer, which is a wrong number rather than a rough one.
            0x43 => float(f64::from(*float_of(value) as f32)),
            0x44 => float(*float_of(value)),
            _ => match value {
                Value::Composite { .. } => composite_args(ev, doc, path)?,
                Value::Enum { name: Some(n), .. } => n.clone(),
                v => scalar(v).to_string(),
            },
        })
    }
}

impl Module {
    /// An instruction from a prefixed group: its own mnemonic, then whatever it
    /// carries. A vector load reads like a scalar one, so it goes through the
    /// same alignment rule; the rest are numbers.
    fn prefixed<S: Source>(&self, ev: &mut Evaluator, doc: &Document<S>, path: &[usize], op: i128) -> R<String> {
        let sub = ev.node(doc, &child(path, 0))?;
        let (raw, name) = match sub.value {
            Value::Enum { raw, name, .. } => (raw, name),
            _ => return Err(EvalError::Failed(format!("instruction 0x{op:02x} has no sub-opcode"))),
        };
        let Some(mnemonic) = name else {
            return Err(EvalError::Failed(format!("unknown opcode 0x{op:02x} 0x{raw:02x}")));
        };
        let args = child(path, 1);
        let node = ev.node(doc, &args)?;
        if node.size_bits == 0 && !node.composite {
            return Ok(mnemonic);
        }
        let text = match (op, raw) {
            // A vector load or store, with or without a lane number after it.
            (0xfd, 0x00..=0x0b) | (0xfd, 0x5c..=0x5d) => mem_text(ev, doc, &args, simd_align(raw))?,
            // Every atomic access but the fence. An atomic access has to be
            // naturally aligned, so a well-formed file never says otherwise
            // and the alignment is never worth printing.
            (0xfe, _) if raw != 0x03 => mem_text(ev, doc, &args, thread_align(raw))?,
            // The byte after `atomic.fence` is reserved and has to be zero, so
            // saying so adds nothing. A file that says otherwise is worth
            // seeing.
            (0xfe, 0x03) => match int_at(ev, doc, &args)? {
                0 => String::new(),
                other => other.to_string(),
            },
            (0xfd, 0x54..=0x5b) => {
                let mem = mem_text(ev, doc, &args, simd_align(raw))?;
                let lane = int_at(ev, doc, &child(&args, 2))?;
                if mem.is_empty() { lane.to_string() } else { format!("{mem} {lane}") }
            }
            _ => match node.value {
                Value::Composite { .. } => composite_args(ev, doc, &args)?,
                ref v => brief_value(v),
            },
        };
        Ok(if text.is_empty() { mnemonic } else { format!("{mnemonic} {text}") })
    }
}

/// The alignment a vector access has of its own accord, as the log2 the format
/// stores. Eight of these load or store one lane, and are aligned to that lane
/// rather than to the whole vector.
fn simd_align(sub: i128) -> u8 {
    match sub {
        0x00 | 0x0b => 4,                                  // v128.load, v128.store
        0x01..=0x06 | 0x0a => 3,                           // the widening loads, load64_splat
        0x07 => 0,                                         // load8_splat
        0x08 => 1,                                         // load16_splat
        0x09 | 0x5c => 2,                                  // load32_splat, load32_zero
        0x5d => 3,                                         // load64_zero
        0x54 | 0x58 => 0,                                  // load8_lane, store8_lane
        0x55 | 0x59 => 1,
        0x56 | 0x5a => 2,
        0x57 | 0x5b => 3,
        _ => 0,
    }
}

/// `offset=8 align=4`, leaving out a zero offset and an alignment that is the
/// one the access has anyway.
fn mem_text<S: Source>(ev: &mut Evaluator, doc: &Document<S>, path: &[usize], natural: u8) -> R<String> {
    let align = int_at(ev, doc, &child(path, 0))?;
    let offset = int_at(ev, doc, &child(path, 1))?;
    let mut s = String::new();
    if offset != 0 {
        let _ = write!(s, "offset={offset}");
    }
    if align != natural as i128 {
        if !s.is_empty() {
            s.push(' ');
        }
        let _ = write!(s, "align={}", 1i128 << align.max(0));
    }
    Ok(s)
}

fn brief_value(v: &Value) -> String {
    match v {
        Value::Bytes { preview, .. } => preview.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" "),
        Value::Enum { name: Some(n), .. } => n.clone(),
        v => scalar(v).to_string(),
    }
}

/// Every child of a composite immediate, in order, which is the right reading
/// for the groups that are just a list of indices.
fn composite_args<S: Source>(ev: &mut Evaluator, doc: &Document<S>, path: &[usize]) -> R<String> {
    let count = ev.node(doc, path)?.child_count;
    let mut parts = Vec::with_capacity(count as usize);
    for i in 0..count as usize {
        let n = ev.node(doc, &child(path, i))?;
        parts.push(match n.value {
            Value::Enum { name: Some(name), .. } => name,
            Value::Composite { .. } => composite_args(ev, doc, &child(path, i))?,
            v => scalar(&v).to_string(),
        });
    }
    Ok(parts.join(" "))
}

/// A float as text, keeping the point so a whole number still reads as one.
fn float(v: f64) -> String {
    if v.is_finite() { format!("{v:?}") } else { format!("{v}") }
}

fn float_of(v: &Value) -> &f64 {
    match v {
        Value::Float(f) => f,
        _ => &0.0,
    }
}

fn child(path: &[usize], i: usize) -> Vec<usize> {
    let mut p = path.to_vec();
    p.push(i);
    p
}

fn scalar(v: &Value) -> i128 {
    match v {
        Value::UInt(n) => *n as i128,
        Value::Int(n) => *n,
        Value::Enum { raw, .. } => *raw,
        _ => 0,
    }
}

fn int_at<S: Source>(ev: &mut Evaluator, doc: &Document<S>, path: &[usize]) -> R<i128> {
    Ok(scalar(&ev.node(doc, path)?.value))
}

fn str_at<S: Source>(ev: &mut Evaluator, doc: &Document<S>, path: &[usize]) -> R<String> {
    Ok(match ev.node(doc, path)?.value {
        Value::Str(s) => s,
        _ => String::new(),
    })
}

/// Read a byte range, refusing rather than guessing when the file has not
/// streamed that far in yet.
fn read<S: Source>(doc: &Document<S>, at: u64, len: u64) -> R<Vec<u8>> {
    let mut buf = vec![0u8; len as usize];
    let missing = doc.read_bytes(at, &mut buf);
    if !missing.is_empty() {
        return Err(EvalError::Pending(missing));
    }
    Ok(buf)
}

/// The `name` custom section: subsections by id, of which 1 maps function
/// indices to names. Anything else is skipped by its recorded size, so an
/// unknown subsection costs nothing.
fn read_name_section(bytes: &[u8], out: &mut HashMap<u32, String>) {
    let mut p = 0usize;
    while p < bytes.len() {
        let id = bytes[p];
        p += 1;
        let Some((size, n)) = leb(bytes, p) else { return };
        p += n;
        let end = match p.checked_add(size as usize) {
            Some(e) if e <= bytes.len() => e,
            // A size past the end means this is not the section it claims to
            // be; stop rather than read the neighbours as names.
            _ => return,
        };
        if id == 1 {
            read_name_map(&bytes[p..end], out);
        }
        p = end;
    }
}

fn read_name_map(bytes: &[u8], out: &mut HashMap<u32, String>) {
    let mut p = 0usize;
    let Some((count, n)) = leb(bytes, p) else { return };
    p += n;
    for _ in 0..count {
        let Some((index, n)) = leb(bytes, p) else { return };
        p += n;
        let Some((len, n)) = leb(bytes, p) else { return };
        p += n;
        let end = match p.checked_add(len as usize) {
            Some(e) if e <= bytes.len() => e,
            _ => return,
        };
        if let Ok(s) = std::str::from_utf8(&bytes[p..end]) {
            out.insert(index as u32, s.to_string());
        }
        p = end;
    }
}

/// One unsigned LEB128, and how many bytes it took. None when it runs off the
/// end or is longer than a `u64` can hold.
fn leb(bytes: &[u8], at: usize) -> Option<(u64, usize)> {
    let mut value = 0u64;
    let mut shift = 0u32;
    let mut i = at;
    loop {
        let b = *bytes.get(i)?;
        i += 1;
        value |= u64::from(b & 0x7f).checked_shl(shift)?;
        if b & 0x80 == 0 {
            return Some((value, i - at));
        }
        shift += 7;
        if shift >= 64 {
            return None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formats::wasm;
    use crate::source::MemSource;

    fn leb_u(mut v: u64, out: &mut Vec<u8>) {
        loop {
            let b = (v & 0x7f) as u8;
            v >>= 7;
            if v == 0 {
                out.push(b);
                return;
            }
            out.push(b | 0x80);
        }
    }

    fn section(id: u8, body: &[u8], out: &mut Vec<u8>) {
        out.push(id);
        leb_u(body.len() as u64, out);
        out.extend_from_slice(body);
    }

    fn name(s: &str, out: &mut Vec<u8>) {
        leb_u(s.len() as u64, out);
        out.extend_from_slice(s.as_bytes());
    }

    /// A module with one imported function, one defined one that calls it, and
    /// a `name` section that names both.
    fn module() -> Vec<u8> {
        let mut b = b"\0asm".to_vec();
        b.extend_from_slice(&1u32.to_le_bytes());

        // type: one function taking nothing and returning nothing
        section(1, &[1, 0x60, 0, 0], &mut b);

        // import: env.log as a function of type 0
        let mut imports = vec![1u8];
        name("env", &mut imports);
        name("log", &mut imports);
        imports.extend_from_slice(&[0, 0]);
        section(2, &imports, &mut b);

        // function: one defined function, of type 0
        section(3, &[1, 0], &mut b);

        // export: the defined function, which is index 1
        let mut exports = vec![1u8];
        name("run", &mut exports);
        exports.extend_from_slice(&[0, 1]);
        section(7, &exports, &mut b);

        // code: block ... call 0 ... end, end
        let body: &[u8] = &[
            0, // no locals
            0x02, 0x40, // block (empty result)
            0x10, 0x00, // call 0
            0x0b, // end
            0x28, 0x02, 0x08, // i32.load offset=8, aligned 4, which is its natural alignment
            0x0b, // end
        ];
        let mut code = vec![1u8];
        leb_u(body.len() as u64, &mut code);
        code.extend_from_slice(body);
        section(10, &code, &mut b);

        b
    }

    fn with_name_section(mut b: Vec<u8>) -> Vec<u8> {
        // name subsection 1: function names for indices 0 and 1
        let mut map = vec![2u8];
        leb_u(0, &mut map);
        name("host_log", &mut map);
        leb_u(1, &mut map);
        name("main_loop", &mut map);

        let mut payload = Vec::new();
        name("name", &mut payload);
        payload.push(1);
        leb_u(map.len() as u64, &mut payload);
        payload.extend_from_slice(&map);
        section(0, &payload, &mut b);
        b
    }

    #[test]
    fn indices_resolve_to_names() {
        let d = Document::new(MemSource(module()));
        let mut ev = Evaluator::new(wasm());
        let m = Module::read(&mut ev, &d).unwrap();
        assert_eq!(m.first_defined(), 1);
        assert_eq!(m.func_count(), 1);
        // The import keeps its module and field; the defined one takes the
        // name it is exported under.
        assert_eq!(m.func_name(0), "env.log");
        assert_eq!(m.func_name(1), "run");
    }

    #[test]
    fn the_name_section_wins() {
        let d = Document::new(MemSource(with_name_section(module())));
        let mut ev = Evaluator::new(wasm());
        let m = Module::read(&mut ev, &d).unwrap();
        assert_eq!(m.func_name(0), "host_log");
        assert_eq!(m.func_name(1), "main_loop");
    }

    #[test]
    fn a_body_reads_as_indented_text() {
        let d = Document::new(MemSource(module()));
        let mut ev = Evaluator::new(wasm());
        let m = Module::read(&mut ev, &d).unwrap();
        assert_eq!(
            m.disassemble(&mut ev, &d, 0).unwrap(),
            "(func $run\n  block\n    call $env.log\n  end\n  i32.load offset=8\nend\n)\n"
        );
    }

    #[test]
    fn an_unknown_opcode_stops_the_body_and_says_so() {
        let mut b = b"\0asm".to_vec();
        b.extend_from_slice(&1u32.to_le_bytes());
        section(3, &[1, 0], &mut b);
        // 0x06 is not assigned in the core spec.
        let body: &[u8] = &[0, 0x01, 0x06, 0x0b];
        let mut code = vec![1u8];
        leb_u(body.len() as u64, &mut code);
        code.extend_from_slice(body);
        section(10, &code, &mut b);

        let d = Document::new(MemSource(b));
        let mut ev = Evaluator::new(wasm());
        let m = Module::read(&mut ev, &d).unwrap();
        let text = m.disassemble(&mut ev, &d, 0).unwrap();
        assert!(text.contains("nop"), "{text}");
        assert!(text.contains(";; disassembly stopped: unknown opcode 0x06"), "{text}");
    }

    #[test]
    fn a_simd_instruction_reads_its_immediate() {
        let mut b = b" asm".to_vec();
        b.extend_from_slice(&1u32.to_le_bytes());
        section(3, &[1, 0], &mut b);
        // f64.const 3.5, v128.const with its sixteen bytes, then v128.load with
        // a non-default alignment, then i8x16.extract_lane_u 3.
        let mut body = vec![0u8, 0x44];
        body.extend_from_slice(&3.5f64.to_le_bytes());
        body.extend_from_slice(&[0xfd, 0x0c]);
        body.extend_from_slice(&[0x41; 16]);
        body.extend_from_slice(&[0xfd, 0x00, 0x02, 0x08]);
        body.extend_from_slice(&[0xfd, 0x16, 0x03]);
        body.push(0x0b);
        let mut code = vec![1u8];
        leb_u(body.len() as u64, &mut code);
        code.extend_from_slice(&body);
        section(10, &code, &mut b);

        let d = Document::new(MemSource(b));
        let mut ev = Evaluator::new(wasm());
        let m = Module::read(&mut ev, &d).unwrap();
        let text = m.disassemble(&mut ev, &d, 0).unwrap();
        // A float immediate keeps its point rather than truncating.
        assert!(text.contains("f64.const 3.5"), "{text}");
        // The prefix is not part of the name, and the sixteen bytes are read.
        assert!(text.contains("v128.const 41 41 41 41 41 41 41 41 41 41 41 41 41 41 41 41"), "{text}");
        // A vector load is aligned to sixteen of its own accord, so four is
        // worth saying.
        assert!(text.contains("v128.load offset=8 align=4"), "{text}");
        assert!(text.contains("i8x16.extract_lane_u 3"), "{text}");
        // The body now runs to its end rather than stopping partway.
        assert!(text.trim_end().ends_with("end
)"), "{text}");
        assert!(!text.contains(";; disassembly stopped"), "{text}");
    }

    #[test]
    fn an_atomic_instruction_reads_its_immediate() {
        let mut b = b" asm".to_vec();
        b.extend_from_slice(&1u32.to_le_bytes());
        section(3, &[1, 0], &mut b);
        // i32.atomic.load offset=16 (aligned 4, which it must be), then
        // atomic.fence, then i64.atomic.rmw8.add_u offset=0.
        let body: &[u8] = &[
            0, //
            0xfe, 0x10, 0x02, 0x10, //
            0xfe, 0x03, 0x00, //
            0xfe, 0x22, 0x00, 0x00, //
            0x0b,
        ];
        let mut code = vec![1u8];
        leb_u(body.len() as u64, &mut code);
        code.extend_from_slice(body);
        section(10, &code, &mut b);

        let d = Document::new(MemSource(b));
        let mut ev = Evaluator::new(wasm());
        let m = Module::read(&mut ev, &d).unwrap();
        let text = m.disassemble(&mut ev, &d, 0).unwrap();
        // An atomic access must be naturally aligned, so its alignment is
        // never worth printing and only the offset is.
        assert!(text.contains("i32.atomic.load offset=16"), "{text}");
        assert!(!text.contains("align="), "{text}");
        // The reserved byte is zero in every valid file, so it is not printed.
        assert!(text.contains("atomic.fence
"), "{text}");
        assert!(text.contains("i64.atomic.rmw8.add_u"), "{text}");
        assert!(text.trim_end().ends_with("end
)"), "{text}");
        assert!(!text.contains(";; disassembly stopped"), "{text}");
    }

}
