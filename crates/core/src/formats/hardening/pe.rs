//! A Windows program's protections, read from the flags in its headers and
//! from its load configuration.
//!
//! Most of what a PE says about itself is one bit of `dll_characteristics`:
//! whether it can be loaded anywhere, whether its data can be run, whether
//! its indirect calls are checked. The rest is in the load configuration, a
//! structure the template does not read, which the data directory gives the
//! address of. That address is where the structure is once the program is
//! loaded, so it is turned into a place in the file through the section
//! table here, and its fields are read as bytes. The structure has grown with
//! every release of Windows and says how long it is in its first word, so
//! each field is read only when that length reaches it.

use super::*;

/// `dll_characteristics` bits.
const HIGH_ENTROPY_VA: u64 = 0x20;
const DYNAMIC_BASE: u64 = 0x40;
const FORCE_INTEGRITY: u64 = 0x80;
const NX_COMPAT: u64 = 0x100;
const NO_SEH: u64 = 0x400;
const APPCONTAINER: u64 = 0x1000;
const GUARD_CF: u64 = 0x4000;

/// `characteristics`: the relocations were taken out, so the program can only
/// be loaded at the address it was linked for.
const RELOCS_STRIPPED: u64 = 0x1;

const DIR_SECURITY: usize = 4;
const DIR_BASERELOC: usize = 5;
const DIR_LOAD_CONFIG: usize = 10;

/// A field of the load configuration: its name, and where it is in a 32-bit
/// and in a 64-bit program, with its width. A field a 64-bit program does not
/// have is `None` there.
struct LoadField {
    name: &'static str,
    pe32: Option<(u64, u64)>,
    pe32plus: Option<(u64, u64)>,
}

const SECURITY_COOKIE: LoadField = LoadField { name: "SecurityCookie", pe32: Some((0x3c, 4)), pe32plus: Some((0x58, 8)) };
const SE_HANDLER_TABLE: LoadField = LoadField { name: "SEHandlerTable", pe32: Some((0x40, 4)), pe32plus: None };
const SE_HANDLER_COUNT: LoadField = LoadField { name: "SEHandlerCount", pe32: Some((0x44, 4)), pe32plus: None };
const GUARD_CF_FUNCTION_COUNT: LoadField =
    LoadField { name: "GuardCFFunctionCount", pe32: Some((0x54, 4)), pe32plus: Some((0x88, 8)) };
const GUARD_FLAGS: LoadField = LoadField { name: "GuardFlags", pe32: Some((0x58, 4)), pe32plus: Some((0x90, 4)) };

/// What the pass read of one PE file.
struct Pe {
    header: Vec<usize>,
    pe32plus: bool,
    characteristics: u64,
    characteristics_ev: Evidence,
    dll: u64,
    dll_ev: Evidence,
    /// Each data directory entry: its address, its size, and the entry.
    directories: Vec<(u64, u64, Evidence)>,
    /// The count of directory entries, for an answer that rests on an entry
    /// the table is too short to have.
    directory_count_ev: Evidence,
    /// Each section's address once loaded, its size there, where its bytes
    /// are in the file and how many there are.
    sections: Vec<(u64, u64, u64, u64)>,
    load_config: LoadConfig,
}

/// The load configuration, as far as it was found and read.
enum LoadConfig {
    /// The data directory has no entry for one.
    Absent,
    /// It has one, but its address is in no section, or the structure is
    /// past the end of the file.
    Unreadable,
    /// Where the structure starts in the file, how long it says it is, and
    /// its bytes.
    Read { at: u64, size: u64, bytes: Vec<u8> },
}

pub(super) fn read<S: Source>(ev: &mut Evaluator, doc: &Document<S>) -> R<Vec<Part>> {
    let Some(pe) = Pe::read(ev, doc)? else { return Ok(Vec::new()) };
    Ok(vec![Part { name: String::new(), path: pe.header.clone(), rows: pe.rows() }])
}

impl Pe {
    fn read<S: Source>(ev: &mut Evaluator, doc: &Document<S>) -> R<Option<Pe>> {
        let header = named(ev, doc, &[], "pe")?;
        let optional = named(ev, doc, &header, "optional")?;
        let pe32plus = match int_field(ev, doc, &optional, "magic")? {
            0x10b => false,
            0x20b => true,
            // A layout neither the template nor this knows.
            _ => return Ok(None),
        };
        let addresses = named(ev, doc, &optional, "addresses")?;
        let chars_path = named(ev, doc, &header, "characteristics")?;
        let dll_path = named(ev, doc, &addresses, "dll_characteristics")?;
        let count_path = named(ev, doc, &addresses, "data_directory_count")?;
        let table = named(ev, doc, &addresses, "data_directory")?;
        let mut directories = Vec::new();
        for i in 0..ev.node(doc, &table)?.child_count as usize {
            let e = child(&table, i);
            let name = directory_name(i);
            directories.push((int_field(ev, doc, &e, "rva")?, int_field(ev, doc, &e, "size")?, node_evidence(ev, doc, &e, "directory", &name)?));
        }
        let table = named(ev, doc, &header, "sections")?;
        let mut sections = Vec::new();
        for i in 0..ev.node(doc, &table)?.child_count as usize {
            let s = child(&table, i);
            sections.push((
                int_field(ev, doc, &s, "virtual_address")?,
                int_field(ev, doc, &s, "virtual_size")?,
                int_field(ev, doc, &s, "raw_offset")?,
                int_field(ev, doc, &s, "raw_size")?,
            ));
        }
        let mut pe = Pe {
            pe32plus,
            characteristics: int_at(ev, doc, &chars_path)?,
            characteristics_ev: node_evidence(ev, doc, &chars_path, "header", "characteristics")?,
            dll: int_at(ev, doc, &dll_path)?,
            dll_ev: node_evidence(ev, doc, &dll_path, "header", "dll_characteristics")?,
            directory_count_ev: node_evidence(ev, doc, &count_path, "header", "data_directory_count")?,
            directories,
            sections,
            header,
            load_config: LoadConfig::Absent,
        };
        pe.load_config = pe.read_load_config(doc)?;
        Ok(Some(pe))
    }

    /// Where in the file the loaded address `rva` comes from: inside the
    /// bytes some section takes from the file.
    fn file_offset(&self, rva: u64) -> Option<u64> {
        self.sections
            .iter()
            .find(|&&(va, vsize, _, raw)| va <= rva && rva < va + vsize.max(raw) && rva - va < raw)
            .map(|&(va, _, at, _)| at + (rva - va))
    }

    fn directory(&self, i: usize) -> Option<&(u64, u64, Evidence)> {
        self.directories.get(i)
    }

    fn read_load_config<S: Source>(&self, doc: &Document<S>) -> R<LoadConfig> {
        let Some((rva, _, _)) = self.directory(DIR_LOAD_CONFIG).filter(|d| d.0 != 0 && d.1 != 0) else {
            return Ok(LoadConfig::Absent);
        };
        let Some(at) = self.file_offset(*rva) else { return Ok(LoadConfig::Unreadable) };
        let head = read_at(doc, at, 4)?;
        if head.len() < 4 {
            return Ok(LoadConfig::Unreadable);
        }
        // The structure's own length, which is what the loader goes by. Its
        // directory entry says the same for a recent linker, and something
        // shorter for an old one.
        let own = word(&head, 0, 4, true);
        let bytes = read_at(doc, at, own.min(0x200))?;
        Ok(LoadConfig::Read { at, size: own.min(bytes.len() as u64), bytes })
    }

    /// A field of the load configuration and where it is, or nothing where
    /// there is no load configuration, it is too short to reach the field, or
    /// this kind of program has no such field.
    fn load_field(&self, f: &LoadField) -> Option<(u64, Evidence)> {
        let LoadConfig::Read { at, size, bytes, .. } = &self.load_config else { return None };
        let (offset, width) = if self.pe32plus { f.pe32plus? } else { f.pe32? };
        if offset + width > *size {
            return None;
        }
        let value = word(bytes, offset as usize, width as usize, true);
        Some((value, bytes_evidence(at + offset, width, "bytes", f.name)))
    }

    /// The load configuration's directory entry, or what says there is none.
    fn load_config_ev(&self) -> Evidence {
        match self.directory(DIR_LOAD_CONFIG) {
            Some((_, _, e)) => e.clone(),
            None => self.directory_count_ev.clone(),
        }
    }

    fn bit(&self, bit: u64) -> bool {
        self.dll & bit != 0
    }

    fn rows(&self) -> Vec<Row> {
        let dll = || self.dll_ev.clone();
        let on_off = |key: &'static str, bit: u64, on: Rating, off: Rating| match self.bit(bit) {
            true => Row::new(key, "on", on).evidence([dll()]),
            false => Row::new(key, "off", off).evidence([dll()]),
        };
        let mut rows = Vec::new();

        // ASLR: the program says it can be loaded anywhere, and it kept the
        // relocations that let it be. Without them it is loaded where it was
        // linked for whatever the flag says.
        let reloc = self.directory(DIR_BASERELOC);
        rows.push(match self.bit(DYNAMIC_BASE) {
            false => Row::new("aslr", "off", Rating::Bad).evidence([dll()]),
            true => {
                let stripped = self.characteristics & RELOCS_STRIPPED != 0;
                let none = reloc.is_none_or(|d| d.1 == 0);
                let reloc_ev = reloc.map(|d| d.2.clone());
                match stripped || none {
                    true => Row::new("aslr", "no-relocations", Rating::Bad)
                        .evidence([dll(), self.characteristics_ev.clone()].into_iter().chain(reloc_ev)),
                    false => Row::new("aslr", "on", Rating::Good).evidence([dll()].into_iter().chain(reloc_ev)),
                }
            }
        });

        rows.push(match self.pe32plus {
            false => Row::new("high-entropy-va", "n/a", Rating::NotApplicable).evidence([dll()]),
            true => on_off("high-entropy-va", HIGH_ENTROPY_VA, Rating::Good, Rating::Partial),
        });

        rows.push(on_off("dep", NX_COMPAT, Rating::Good, Rating::Bad));

        // Control Flow Guard: the flag, and how many functions the program
        // lists as places an indirect call may go.
        let mut cfg = on_off("cfg", GUARD_CF, Rating::Good, Rating::Info);
        if let Some((n, e)) = self.load_field(&GUARD_CF_FUNCTION_COUNT) {
            cfg.count = Some(n);
            cfg.evidence.push(e);
        }
        if let Some((_, e)) = self.load_field(&GUARD_FLAGS) {
            cfg.evidence.push(e);
        }
        rows.push(cfg);

        // /GS: the address of the cookie the stack checks compare against,
        // which the loader fills in with a random value. A program with no
        // load configuration may still check its stack another way, as
        // MinGW's `-fstack-protector` does, so that is not a "no".
        rows.push(match (&self.load_config, self.load_field(&SECURITY_COOKIE)) {
            (_, Some((cookie, e))) if cookie != 0 => Row::new("gs", "found", Rating::Good).evidence([e, self.load_config_ev()]),
            (_, Some((_, e))) => Row::new("gs", "not-found", Rating::Bad).evidence([e, self.load_config_ev()]),
            (LoadConfig::Absent, _) => Row::new("gs", "no-load-config", Rating::Unknown).evidence([self.load_config_ev()]),
            _ => Row::new("gs", "unknown", Rating::Unknown).evidence([self.load_config_ev()]),
        });

        // SafeSEH, for a 32-bit program: a table of the exception handlers it
        // may use, or a flag saying it uses none. A 64-bit program's handlers
        // are in tables of their own and this does not apply.
        rows.push(match (self.pe32plus, self.bit(NO_SEH)) {
            (true, _) => Row::new("safeseh", "n/a", Rating::NotApplicable).evidence([dll()]),
            (false, true) => Row::new("safeseh", "no-seh", Rating::Good).evidence([dll()]),
            (false, false) => match (&self.load_config, self.load_field(&SE_HANDLER_TABLE), self.load_field(&SE_HANDLER_COUNT)) {
                (LoadConfig::Absent, _, _) => Row::new("safeseh", "off", Rating::Bad).evidence([dll(), self.load_config_ev()]),
                (_, Some((table, t)), Some((count, c))) if table != 0 && count > 0 => {
                    Row::new("safeseh", "on", Rating::Good).count(count).evidence([t, c])
                }
                (_, Some((_, t)), Some((_, c))) => Row::new("safeseh", "off", Rating::Bad).evidence([t, c, dll()]),
                _ => Row::new("safeseh", "unknown", Rating::Unknown).evidence([self.load_config_ev()]),
            },
        });

        // An Authenticode signature: whether there is one, not whether it is
        // good. Its directory entry gives an offset in the file, not an
        // address, and the bytes there are past every section.
        rows.push(match self.directory(DIR_SECURITY) {
            Some((at, size, e)) if *size > 0 => {
                let bytes = bytes_evidence(*at, *size, "bytes", "certificate table");
                Row::new("signature", "present", Rating::Info).evidence([e.clone(), bytes])
            }
            Some((_, _, e)) => Row::new("signature", "none", Rating::Info).evidence([e.clone()]),
            None => Row::new("signature", "none", Rating::Info).evidence([self.directory_count_ev.clone()]),
        });

        rows.push(on_off("force-integrity", FORCE_INTEGRITY, Rating::Info, Rating::Info));
        rows.push(on_off("appcontainer", APPCONTAINER, Rating::Info, Rating::Info));
        rows
    }
}

/// A data directory entry's name, as the template calls it.
fn directory_name(i: usize) -> String {
    crate::formats::pe_tables::DIRECTORY
        .iter()
        .find(|(n, _)| *n == i as i128)
        .map_or_else(|| format!("[{i}]"), |(_, name)| name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::MemSource;

    /// A PE with one section, `.rdata`, at address 0x1000 and file offset
    /// 0x200, holding `load_config` at its start when there is one.
    fn build(pe32plus: bool, characteristics: u16, dll: u16, directories: &[(usize, u32, u32)], load_config: Option<Vec<u8>>) -> Vec<u8> {
        let mut v = vec![0u8; 0x80];
        v[0..2].copy_from_slice(b"MZ");
        v[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        let optional_size: u16 = if pe32plus { 0xf0 } else { 0xe0 };
        v.extend_from_slice(b"PE\0\0");
        v.extend_from_slice(&(if pe32plus { 0x8664u16 } else { 0x14c }).to_le_bytes());
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&[0; 12]);
        v.extend_from_slice(&optional_size.to_le_bytes());
        v.extend_from_slice(&characteristics.to_le_bytes());
        let start = v.len();
        v.extend_from_slice(&(if pe32plus { 0x20bu16 } else { 0x10b }).to_le_bytes());
        v.resize(start + 70, 0);
        v[start + 70 - 2..start + 70].copy_from_slice(&3u16.to_le_bytes()); // console
        v.extend_from_slice(&dll.to_le_bytes());
        let count_at = start + if pe32plus { 108 } else { 92 };
        v.resize(count_at, 0);
        v.extend_from_slice(&16u32.to_le_bytes());
        let mut dirs = [(0u32, 0u32); 16];
        for &(i, rva, size) in directories {
            dirs[i] = (rva, size);
        }
        if let Some(lc) = &load_config {
            dirs[DIR_LOAD_CONFIG] = (0x1000, lc.len() as u32);
        }
        for (rva, size) in dirs {
            v.extend_from_slice(&rva.to_le_bytes());
            v.extend_from_slice(&size.to_le_bytes());
        }
        assert_eq!(v.len(), start + optional_size as usize);
        v.extend_from_slice(b".rdata\0\0");
        for word in [0x200u32, 0x1000, 0x200, 0x200, 0, 0] {
            v.extend_from_slice(&word.to_le_bytes());
        }
        v.extend_from_slice(&[0; 4]);
        v.extend_from_slice(&0x4000_0040u32.to_le_bytes());
        v.resize(0x200, 0);
        v.extend_from_slice(&load_config.unwrap_or_default());
        v.resize(0x400, 0);
        v
    }

    /// A load configuration of `size` bytes with `fields` written into it.
    fn load_config(size: usize, fields: &[(usize, u64, usize)]) -> Vec<u8> {
        let mut v = vec![0u8; size];
        v[0..4].copy_from_slice(&(size as u32).to_le_bytes());
        for &(at, value, width) in fields {
            v[at..at + width].copy_from_slice(&value.to_le_bytes()[..width]);
        }
        v
    }

    fn rows_of(bytes: Vec<u8>) -> (Vec<Row>, Document<MemSource>, Evaluator) {
        let doc = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(crate::formats::pe());
        let h = super::super::read(&mut ev, &doc).unwrap().expect("a PE");
        assert_eq!(h.format, "pe");
        (h.parts[0].rows.clone(), doc, ev)
    }

    fn row<'a>(rows: &'a [Row], key: &str) -> &'a Row {
        rows.iter().find(|r| r.key == key).unwrap_or_else(|| panic!("no row {key}"))
    }

    #[test]
    fn a_64_bit_program_built_with_every_flag_reads_as_built() {
        let lc = load_config(0x140, &[(0x58, 0x1_4000_3000, 8), (0x88, 1234, 8), (0x90, 0x0001_0500, 4)]);
        let bytes = build(true, 0x22, 0x4160, &[(DIR_BASERELOC, 0x2000, 0x40), (DIR_SECURITY, 0x300, 0x100)], Some(lc));
        let (rows, doc, mut ev) = rows_of(bytes);
        let states: Vec<(&str, &str)> = rows.iter().map(|r| (r.key, r.state)).collect();
        assert_eq!(
            states,
            [
                ("aslr", "on"),
                ("high-entropy-va", "on"),
                ("dep", "on"),
                ("cfg", "on"),
                ("gs", "found"),
                ("safeseh", "n/a"),
                ("signature", "present"),
                ("force-integrity", "off"),
                ("appcontainer", "off"),
            ]
        );
        let cfg = row(&rows, "cfg");
        assert_eq!(cfg.count, Some(1234));
        let names: Vec<&str> = cfg.evidence.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["dll_characteristics", "GuardCFFunctionCount", "GuardFlags"]);
        let gs = row(&rows, "gs");
        assert_eq!((gs.evidence[0].what, gs.evidence[0].offset_bits), ("bytes", (0x200 + 0x58) * 8));
        assert!(gs.evidence[0].path.is_empty());
        assert_eq!((gs.evidence[1].what, gs.evidence[1].name.as_str()), ("directory", "load config"));
        // The header fields are the template's own.
        for e in rows.iter().flat_map(|r| r.evidence.iter()).filter(|e| !e.path.is_empty()) {
            let n = ev.node(&doc, &e.path).unwrap();
            assert_eq!((n.offset_bits, n.size_bits), (e.offset_bits, e.size_bits), "{e:?}");
        }
    }

    /// A program that says it can be loaded anywhere and kept no
    /// relocations is loaded where it was linked all the same.
    #[test]
    fn dynamic_base_without_relocations_is_not_aslr() {
        let (rows, _, _) = rows_of(build(true, 0x23, 0x0160, &[], None));
        let aslr = row(&rows, "aslr");
        assert_eq!((aslr.state, aslr.rating), ("no-relocations", Rating::Bad));
        let names: Vec<&str> = aslr.evidence.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["dll_characteristics", "characteristics", "basereloc"]);
        // No load configuration: nothing says whether the stack is checked.
        assert_eq!(row(&rows, "gs").state, "no-load-config");
        assert_eq!(row(&rows, "cfg").count, None);
    }

    #[test]
    fn a_32_bit_program_lists_its_exception_handlers() {
        let lc = load_config(0x48, &[(0x3c, 0x40_3000, 4), (0x40, 0x40_2000, 4), (0x44, 7, 4)]);
        let (rows, _, _) = rows_of(build(false, 0x102, 0x0040, &[(DIR_BASERELOC, 0x2000, 0x40)], Some(lc)));
        let safeseh = row(&rows, "safeseh");
        assert_eq!((safeseh.state, safeseh.count), ("on", Some(7)));
        assert_eq!(row(&rows, "high-entropy-va").state, "n/a");
        assert_eq!(row(&rows, "dep").state, "off");
        // Too short a structure to reach the guard fields: no count.
        assert_eq!(row(&rows, "cfg").count, None);

        // With no load configuration there is no table of handlers, and with
        // the flag that says it has none, none is needed.
        let (rows, _, _) = rows_of(build(false, 0x102, 0x0000, &[], None));
        assert_eq!(row(&rows, "safeseh").state, "off");
        let (rows, _, _) = rows_of(build(false, 0x102, 0x0400, &[], None));
        assert_eq!(row(&rows, "safeseh").state, "no-seh");
    }
}
