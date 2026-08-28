//! Named disassembly for 16-bit Windows programs: a call to another module
//! says which function of which module it calls.
//!
//! A NE file calls into Windows by leaving a far address of nothing in the
//! instruction and a relocation record beside the segment saying what the
//! loader should write there. The record names a module by its number in the
//! module reference table, and a function either by its ordinal or by an
//! offset into the imported name table.
//!
//! One record can stand for many call sites. Where the address would go, the
//! linker writes the offset of the next site that wants the same address, and
//! that chain ends with 0xffff. So the sites are found by walking the segment
//! rather than by counting records, which is why this reads the file directly
//! rather than only the tree.

use std::collections::HashMap;

use crate::document::Document;
use crate::eval::{EvalError, Evaluator, R, Value};
use crate::source::Source;

/// What the relocation records of every segment come to: for each byte of each
/// segment that the loader patches, the name of what it patches it with.
#[derive(Debug, Clone, Default)]
pub struct Program {
    /// The modules this one imports from, in the order the table holds them.
    pub modules: Vec<String>,
    /// Where each segment starts in the file, and how long it is.
    segments: Vec<(u64, u64)>,
    /// A patched site, as the segment it is in and the offset within it.
    sites: HashMap<(usize, u64), String>,
}

impl Program {
    /// Read the module references, the imported names, and every segment's
    /// relocations.
    pub fn read<S: Source>(ev: &mut Evaluator, doc: &Document<S>) -> R<Program> {
        let mut p = Program::default();
        let header = named(ev, doc, &[], "ne")?;
        let names = imported_names(ev, doc, &header)?;

        // The module reference table holds offsets into the imported name
        // table, one per module, and a relocation names a module by its place
        // in this table counting from one.
        let refs = child(&named(ev, doc, &header, "module_references")?, 0);
        let count = ev.node(doc, &refs)?.child_count as usize;
        for i in 0..count {
            let at = int_at(ev, doc, &child(&refs, i))? as u64;
            p.modules.push(names.get(&at).cloned().unwrap_or_else(|| format!("module {}", i + 1)));
        }

        let segments = child(&named(ev, doc, &header, "segments")?, 0);
        let segment_count = ev.node(doc, &segments)?.child_count as usize;
        for i in 0..segment_count {
            let s = child(&segments, i);
            let body = match ev.child_named(doc, &s, "contents")? {
                Some(path) => child(&path, 0),
                None => continue,
            };
            let bytes = match ev.child_named(doc, &body, "bytes")? {
                Some(path) => ev.node(doc, &path)?,
                None => continue,
            };
            p.segments.push((bytes.offset_bits / 8, bytes.size_bits / 8));
            let Some(relocations) = ev.child_named(doc, &body, "relocations")? else { continue };
            let Some(entries) = ev.child_named(doc, &relocations, "entries")? else { continue };
            let n = ev.node(doc, &entries)?.child_count as usize;
            for k in 0..n {
                let r = child(&entries, k);
                let kind = int_field(ev, doc, &r, "type")?;
                // What is being patched: a whole far address, or only the
                // segment half of one. A fixup of the segment alone carries no
                // offset, and writing one would be inventing a zero.
                let address = int_field(ev, doc, &r, "address_type")?;
                let at = int_field(ev, doc, &r, "offset")? as u64;
                let target1 = int_field(ev, doc, &r, "target1")? as u64;
                let target2 = int_field(ev, doc, &r, "target2")? as u64;
                let name = match kind {
                    // A function of another module, by its number there.
                    1 => format!("{}.{target2}", p.module(target1)),
                    // The same, by name.
                    2 => format!("{}.{}", p.module(target1), names.get(&target2).cloned().unwrap_or_default()),
                    // Somewhere in this module: a segment and an offset, or an
                    // entry in the table of the ones that can move.
                    0 if target1 == 0xff => format!("entry {target2}"),
                    0 if address == 2 => format!("segment {target1}"),
                    0 => format!("segment {target1}:0x{target2:x}"),
                    _ => format!("system fixup {target1}:{target2}"),
                };
                p.chain(doc, i, at, &name);
            }
        }
        Ok(p)
    }

    /// The module a relocation names, which it does by counting from one.
    fn module(&self, index: u64) -> String {
        match index.checked_sub(1).and_then(|i| self.modules.get(i as usize)) {
            Some(name) => name.clone(),
            None => format!("module {index}"),
        }
    }

    /// Every site one record stands for. The linker wrote the offset of the
    /// next site into the space the address will go in, so the record points
    /// at the head of a list rather than at one instruction.
    fn chain<S: Source>(&mut self, doc: &Document<S>, segment: usize, first: u64, name: &str) {
        let Some(&(start, len)) = self.segments.get(segment) else { return };
        let mut at = first;
        // A chain cannot be longer than the segment holds links, and a file
        // that says otherwise is one that would hang this.
        for _ in 0..len / 2 + 1 {
            if at == 0xffff || at + 2 > len {
                return;
            }
            self.sites.insert((segment, at), name.to_string());
            let mut buf = [0u8; 2];
            if !doc.read_bits((start + at) * 8, 16, &mut buf).is_empty() {
                return;
            }
            let next = u16::from_le_bytes(buf) as u64;
            if next == at {
                return;
            }
            at = next;
        }
    }

    /// One instruction with the name of what the loader writes into it, when a
    /// relocation covers any of its bytes. `None` when nothing does.
    pub fn instruction_line<S: Source>(
        &self,
        ev: &mut Evaluator,
        doc: &Document<S>,
        path: &[usize],
        segment: usize,
    ) -> R<Option<String>> {
        let node = ev.node(doc, path)?;
        let Value::Str(text) = node.value else { return Ok(None) };
        let Some(&(start, _)) = self.segments.get(segment) else { return Ok(None) };
        let at = (node.offset_bits / 8).saturating_sub(start);
        let len = node.size_bits / 8;
        let name = (at..at + len).find_map(|b| self.sites.get(&(segment, b)));
        let Some(name) = name else { return Ok(None) };
        // The address is the part the loader replaces, so the name replaces
        // it: what the file holds there is a link in a list, not an address.
        let mnemonic = text.split_whitespace().next().unwrap_or(&text);
        Ok(Some(format!("{mnemonic} {name}")))
    }

    /// Which segment a path is in, for a caller holding a path and nothing
    /// else. The segments are an array in the header, and an instruction sits
    /// under the segment's own contents.
    pub fn segment_of(path: &[usize]) -> Option<usize> {
        // [1, segments, 0, i, contents, 0, bytes, instruction]
        match path.len() {
            n if n >= 5 => Some(path[n - 5]),
            _ => None,
        }
    }
}

/// The imported name table, by the offset into it that a relocation writes.
/// Every name in the file is referred to this way: the module reference table
/// holds offsets into this one too.
fn imported_names<S: Source>(ev: &mut Evaluator, doc: &Document<S>, header: &[usize]) -> R<HashMap<u64, String>> {
    let mut out = HashMap::new();
    let table = child(&named(ev, doc, header, "imported_names")?, 0);
    let start = ev.node(doc, &table)?.offset_bits / 8;
    let count = ev.node(doc, &table)?.child_count as usize;
    for i in 0..count {
        let entry = child(&table, i);
        let node = ev.node(doc, &entry)?;
        let Some(text) = ev.child_named(doc, &entry, "text")? else { continue };
        if let Value::Str(name) = ev.node(doc, &text)?.value {
            out.insert(node.offset_bits / 8 - start, name);
        }
    }
    Ok(out)
}

fn child(path: &[usize], i: usize) -> Vec<usize> {
    let mut p = path.to_vec();
    p.push(i);
    p
}

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

fn int_field<S: Source>(ev: &mut Evaluator, doc: &Document<S>, path: &[usize], name: &str) -> R<i128> {
    let p = named(ev, doc, path, name)?;
    int_at(ev, doc, &p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval::Evaluator;
    use crate::formats::ne;
    use crate::source::MemSource;

    /// A module that calls one imported function twice. The two sites are a
    /// chain: the first holds the offset of the second where its address will
    /// go, and the second holds the end of the chain.
    fn sample() -> Vec<u8> {
        let ne_at = 0x40usize;
        let mut v = vec![0u8; ne_at];
        v[0..2].copy_from_slice(b"MZ");
        v[0x18..0x1a].copy_from_slice(&0x40u16.to_le_bytes());
        v[0x3c..0x40].copy_from_slice(&(ne_at as u32).to_le_bytes());

        let mut h = vec![0u8; 0x40];
        h[0..2].copy_from_slice(b"NE");
        h[0x1c..0x1e].copy_from_slice(&1u16.to_le_bytes()); // one segment
        h[0x1e..0x20].copy_from_slice(&1u16.to_le_bytes()); // one module referred to
        h[0x22..0x24].copy_from_slice(&0x40u16.to_le_bytes()); // segment table
        h[0x26..0x28].copy_from_slice(&0x48u16.to_le_bytes()); // resident names
        h[0x28..0x2a].copy_from_slice(&0x50u16.to_le_bytes()); // module references
        h[0x2a..0x2c].copy_from_slice(&0x52u16.to_le_bytes()); // imported names
        h[0x04..0x06].copy_from_slice(&0x58u16.to_le_bytes()); // entry table
        h[0x32..0x34].copy_from_slice(&4u16.to_le_bytes()); // sector shift
        h[0x36] = 2; // Windows
        v.extend_from_slice(&h);

        // One segment: code, twelve bytes of it, at sector 0x10, with
        // relocations written after it.
        v.extend_from_slice(&0x10u16.to_le_bytes());
        v.extend_from_slice(&12u16.to_le_bytes());
        v.extend_from_slice(&0x0100u16.to_le_bytes());
        v.extend_from_slice(&0x100u16.to_le_bytes());

        // The module's own name, then the byte that ends that table.
        v.push(3);
        v.extend_from_slice(b"WIN");
        v.extend_from_slice(&0u16.to_le_bytes());
        v.push(0);
        v.push(0); // padding, so the next table starts where the header says
        // One module, whose name is at the front of the imported name table.
        v.extend_from_slice(&0u16.to_le_bytes());
        v.push(4);
        v.extend_from_slice(b"USER");
        v.push(0);
        // An entry table with nothing in it.
        v.push(0);

        v.resize(0x10 << 4, 0);
        // A call whose address field holds the offset of the next site, and a
        // call whose address field ends the chain.
        v.extend_from_slice(&[0x9a, 0x06, 0x00, 0x00, 0x00]);
        v.extend_from_slice(&[0x9a, 0xff, 0xff, 0x00, 0x00]);
        v.extend_from_slice(&[0x90, 0xc3]);
        // One relocation: a far pointer, an imported ordinal, at offset one,
        // of module one, function five.
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&[3, 1]);
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&5u16.to_le_bytes());
        v
    }

    fn line(index: usize) -> Option<String> {
        let d = Document::new(MemSource(sample()));
        let mut ev = Evaluator::new(ne());
        let p = Program::read(&mut ev, &d).unwrap();
        p.instruction_line(&mut ev, &d, &[1, 33, 0, 0, 4, 0, 0, index], 0).unwrap()
    }

    #[test]
    fn the_modules_are_read_by_name() {
        let d = Document::new(MemSource(sample()));
        let mut ev = Evaluator::new(ne());
        let p = Program::read(&mut ev, &d).unwrap();
        assert_eq!(p.modules, ["USER"]);
    }

    #[test]
    fn a_call_says_which_function_of_which_module_it_calls() {
        assert_eq!(line(0).as_deref(), Some("callf USER.5"));
    }

    /// The second site is reached by following the chain from the first, which
    /// is the only thing in the file that says it is the same call.
    #[test]
    fn every_site_the_chain_reaches_is_named() {
        assert_eq!(line(1).as_deref(), Some("callf USER.5"));
    }

    #[test]
    fn an_instruction_nothing_patches_keeps_the_row_it_had() {
        assert_eq!(line(2), None);
    }
}
