//! Which BASIC runtime a DOS executable was built against, read off the loader
//! stub the compiler links into it.
//!
//! Microsoft's BASIC compilers of the 1980s produced two kinds of program. One
//! is linked against `BCOM<version>.LIB` and carries its runtime inside it. The
//! other is linked against `BRUN<version>.EXE`, which stays a separate file:
//! the program is a few kilobytes smaller and will not start unless that exact
//! runtime is on the disk. Which one a file is, and which runtime it wants, is
//! the first thing anybody opening one of these needs to know, and no
//! signature database says it: the entry point is a far call into a segment
//! whose address is different in every program, so an entry-point pattern of
//! the kind Detect It Easy uses has nothing fixed to match.
//!
//! The stub itself does say it, in plain ASCII. It ends the load module,
//! carrying the messages it prints when things go wrong (`Cannot find `,
//! `Must link with BCOM20G.LIB`) and, last, the file name it asks DOS to load.
//! So this reads the name out of the tail of the load module, which is where
//! the linker puts the stub.
//!
//! The load module, not the file. Bytes past it are an overlay: a
//! self-extracting archive that happens to carry `BRUN20G.EXE` as its payload
//! is not a program that needs one, and looking only where the loader would
//! have looked is what keeps those apart.

use crate::diescript::Detection;

/// How far back from the end of the load module the stub is looked for.
///
/// In the files in hand the name sits 20 bytes from the end and the library
/// message about 5 KiB, so this is deliberately wider than either: the stub is
/// followed by whatever the linker aligned after it, and that padding is not
/// fixed.
const TAIL: u64 = 8 * 1024;

/// What the file was built against, as the file names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Runtime {
    /// `BRUN20G.EXE`: a separate file, needed at run time.
    Separate(String),
    /// `BCOM20G.LIB`: linked in, so the program stands alone.
    LinkedIn(String),
}

/// Read the runtime out of a DOS executable, if it names one.
///
/// `head` is the file from the start; nothing is claimed unless it reaches the
/// end of the load module, since a shorter read cannot tell the stub from an
/// overlay.
pub fn detect(head: &[u8], file_len: u64) -> Option<Detection> {
    let options = match runtime(head, file_len)? {
        Runtime::Separate(file) => format!("needs {file} to run"),
        Runtime::LinkedIn(lib) => format!("runtime built in, linked with {lib}"),
    };
    Some(Detection {
        category: "compiler".to_string(),
        name: "Microsoft BASIC".to_string(),
        version: None,
        options: Some(options),
        source: SOURCE.to_string(),
    })
}

/// What the answer is credited to, in place of the signature file a rule from
/// the database would name. The page tells the two apart by this exact word,
/// so that a detection of this editor's own is not credited to a database that
/// never made it.
pub const SOURCE: &str = "qubero";

/// The runtime named in the tail of the load module.
pub fn runtime(head: &[u8], file_len: u64) -> Option<Runtime> {
    let end = load_module_end(head, file_len)?;
    // A read that stops short of the stub cannot say anything about it.
    let end = usize::try_from(end).ok()?;
    if end > head.len() {
        return None;
    }
    let from = end.saturating_sub(TAIL as usize);
    let tail = head.get(from..end)?;

    let mut linked: Option<String> = None;
    for at in 0..tail.len() {
        let Some(name) = name_at(&tail[at..]) else { continue };
        // A program that loads a runtime also carries the message telling you
        // to link the other way, so the separate file wins wherever both are
        // there.
        if name.ends_with(".EXE") {
            return Some(Runtime::Separate(name));
        }
        linked.get_or_insert(name);
    }
    linked.map(Runtime::LinkedIn)
}

/// A runtime file name starting at `bytes`, uppercased as the linker wrote it.
///
/// `BRUN` and `BCOM` are the QuickBASIC names; `BRT` is what the BASIC
/// Professional Development System called the same thing. What follows is the
/// version, in a spelling that varied between releases, so it is read as
/// whatever alphanumerics run up to the extension rather than matched against a
/// list of releases that would be out of date the moment it was written.
fn name_at(bytes: &[u8]) -> Option<String> {
    const PREFIXES: [&str; 3] = ["BRUN", "BCOM", "BRT"];
    let prefix = PREFIXES.into_iter().find(|p| bytes.starts_with(p.as_bytes()))?;
    let rest = &bytes[prefix.len()..];
    let version_len = rest.iter().take(8).take_while(|b| b.is_ascii_alphanumeric()).count();
    let ext = rest.get(version_len..version_len + 4)?;
    // `BRUN20G.EXE` is the runtime; `BCOM20G.LIB` is the library. Anything else
    // ending in these letters is a word, not a file name.
    if ext != b".EXE" && ext != b".LIB" {
        return None;
    }
    let whole = &bytes[..prefix.len() + version_len + 4];
    std::str::from_utf8(whole).ok().map(str::to_string)
}

/// Where the loaded program ends, which is short of the end of the file
/// whenever anything was appended to it.
///
/// The header counts the program in 512-byte pages, with the last one usually
/// part full; a count of zero bytes in the last page means it is full, which is
/// the convention every DOS loader followed.
fn load_module_end(head: &[u8], file_len: u64) -> Option<u64> {
    if !head.starts_with(b"MZ") || is_pe(head) {
        return None;
    }
    let word = |at: usize| -> Option<u64> {
        let b = head.get(at..at + 2)?;
        Some(u64::from(u16::from_le_bytes([b[0], b[1]])))
    };
    let last_page = word(0x02)?;
    let pages = word(0x04)?;
    let header_paragraphs = word(0x08)?;
    if pages == 0 || last_page > 512 {
        return None;
    }
    let end = pages * 512 - if last_page == 0 { 0 } else { 512 - last_page };
    // A header longer than the module it heads is a broken or lying file.
    if end <= header_paragraphs * 16 || end > file_len {
        return None;
    }
    Some(end)
}

/// Whether the DOS header points at a Windows one. A PE's DOS stub is a
/// refusal message, not a BASIC program.
fn is_pe(head: &[u8]) -> bool {
    let Some(b) = head.get(0x3c..0x40) else { return false };
    let at = u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize;
    head.get(at..at + 4) == Some(b"PE\0\0")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An MZ executable whose load module ends with `stub`, and `overlay`
    /// bytes appended past it as an archive or installer would.
    fn exe(stub: &[u8], overlay: &[u8]) -> Vec<u8> {
        let header = 0x20usize; // paragraphs
        let module = header * 16 + 0x400 + stub.len();
        let mut v = vec![0u8; module];
        v[0..2].copy_from_slice(b"MZ");
        let pages = module.div_ceil(512);
        let last = module % 512;
        v[0x02..0x04].copy_from_slice(&(last as u16).to_le_bytes());
        v[0x04..0x06].copy_from_slice(&(pages as u16).to_le_bytes());
        v[0x08..0x0a].copy_from_slice(&(header as u16).to_le_bytes());
        v[module - stub.len()..].copy_from_slice(stub);
        v.extend_from_slice(overlay);
        v
    }

    /// The tail of the stub as the QuickBASIC 2 linker writes it.
    const STUB: &[u8] = b"\r\n$\r\nMust link with BCOM20G.LIB\r\n$\r\nCannot find \r\n$BRUN20G.EXE\0PATH=";

    fn found(bytes: &[u8]) -> Option<Runtime> {
        runtime(bytes, bytes.len() as u64)
    }

    #[test]
    fn a_program_that_loads_a_runtime_names_the_file_it_loads() {
        let f = exe(STUB, b"");
        assert_eq!(found(&f), Some(Runtime::Separate("BRUN20G.EXE".into())));
    }

    #[test]
    fn a_program_with_the_runtime_linked_in_names_the_library() {
        let f = exe(b"\r\nMust link with BCOM30.LIB\r\n$", b"");
        assert_eq!(found(&f), Some(Runtime::LinkedIn("BCOM30.LIB".into())));
    }

    #[test]
    fn the_development_systems_runtime_is_the_same_answer() {
        let f = exe(b"Cannot find \r\n$BRT70EFR.EXE\0PATH=", b"");
        assert_eq!(found(&f), Some(Runtime::Separate("BRT70EFR.EXE".into())));
    }

    #[test]
    fn an_archive_carrying_a_runtime_is_not_a_program_that_needs_one() {
        // The name is in the payload, past the end of the load module.
        let mut payload = vec![0u8; 4096];
        payload.extend_from_slice(b"BRUN20G.EXE\0");
        assert_eq!(found(&exe(b"nothing to see", &payload)), None);
    }

    #[test]
    fn a_read_that_stops_before_the_stub_says_nothing() {
        let f = exe(STUB, b"");
        assert_eq!(runtime(&f[..f.len() - 4], f.len() as u64), None);
    }

    #[test]
    fn other_executables_are_left_alone() {
        assert_eq!(found(&exe(b"Program too large\r\n$", b"")), None);
        assert_eq!(found(b"MZ"), None);
        assert_eq!(found(b"not an executable at all"), None);
    }

    #[test]
    fn a_windows_executable_is_not_asked() {
        let mut f = exe(STUB, b"");
        let at = 0x80u32;
        f[0x3c..0x40].copy_from_slice(&at.to_le_bytes());
        f[at as usize..at as usize + 4].copy_from_slice(b"PE\0\0");
        assert_eq!(found(&f), None);
    }

    #[test]
    fn the_detection_says_which_way_it_was_built() {
        let f = exe(STUB, b"");
        let d = detect(&f, f.len() as u64).unwrap();
        assert_eq!(d.category, "compiler");
        assert_eq!(d.name, "Microsoft BASIC");
        assert_eq!(d.options.as_deref(), Some("needs BRUN20G.EXE to run"));
    }
}
