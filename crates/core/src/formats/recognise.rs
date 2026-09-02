//! Working out what a file is from the bytes in it.
//!
//! Two kinds of evidence, and the difference between them is the whole design
//! of this module. Most formats put a signature at the front and are done, and
//! those are the table in [`MAGIC`]. The rest have to be reasoned about: an
//! MS-DOS executable opens with two letters that a thousand other files could
//! open with, and what settles it is whether the header's offsets land inside
//! the file. Those are the functions here, and [`PROBES`] is the order they
//! and the table are asked in.
//!
//! Nothing in here builds a template. It answers with a name, and
//! [`builtin`](super::builtin) turns the name into one.

use super::*;

/// Formats a file announces by its first bytes and nothing more. Read in
/// order, so a longer signature that starts with a shorter one goes above it.
///
/// A format that needs more than a prefix is not here: those are the functions
/// below, which `sniff` asks first. Several formats in the tree are in neither,
/// because nothing marks the front of the file at all: a TGA, a Degas screen
/// and a CBOR document are templates to pick rather than templates to guess at.
const MAGIC: &[(&[u8], &str)] = &[
    (b"SQLite format 3\0", "sqlite"),
    (b"\x89PNG\r\n\x1a\n", "png"),
    (b"\xd0\x0d\xfe\xed", "dtb"),
    (grubenv::SIGNATURE, "grubenv"),
    // A static library, and the outside of a Debian package: `sniff` has
    // already asked whether this one is a package.
    (ar::MAGIC, "ar"),
    (rpm::MAGIC, "rpm"),
    (cab::MAGIC, "cab"),
    (xar::MAGIC, "xar"),
    // An initramfs, and every other cpio archive written this century.
    (b"070701", "cpio"),
    (b"070702", "cpio"),
    (b"LPKSHHRH", "journal"),
    (b"FWS", "swf"),
    (b"CWS", "swf"),
    (b"ZWS", "swf"),
    (b"PK\x03\x04", "zip"),
    (b"PK\x05\x06", "zip"),
    (b"\0asm", "wasm"),
    (b"GGUF", "gguf"),
    // Four lower-case letters, which is weaker than most of what is here; a
    // ROOT file is what a `.root` file is, and nothing else in this table
    // opens with them.
    (b"root", "root"),
    (b"DRACO", "draco"),
    (b"MThd", "midi"),
    (b"\x1f\x8b", "gzip"),
    (uf2::MAGIC, "uf2"),
    (b"DIRC", "gitindex"),
    (b"\xfftOc", "gitpackidx"),
    (b"IWAD", "wad"),
    (b"PWAD", "wad"),
    (b"\x34\x12\xaa\x55", "vpk"),
    (b"NES\x1a", "nes"),
    (b"GIF8", "gif"),
    (b"qoif", "qoi"),
    // Three bytes rather than two: the marker after the start-of-image is
    // the first segment, and every JPEG has one.
    (b"\xff\xd8\xff", "jpeg"),
    (b"II*\x00", "tiff"),
    // `II` and then the byte where a TIFF writes 42: a JPEG XR is a TIFF
    // directory the whole way down and says so three bytes in.
    (b"II\xbc", "jxr"),
    (b"MM\x00*", "tiff"),
    (b".snd", "au"),
    (b"%PDF-", "pdf"),
    (b"8BPS", "psd"),
    (b"%!PS-Adobe-", "eps"),
    (b"\xc5\xd0\xd3\xc6", "eps"),
    (b"UnityFS\0", "unitybundle"),
    (b"UnityRaw\0", "unitybundle"),
    (b"UnityWeb\0", "unitybundle"),
    // An `.h5ad` single-cell dataset, a Keras model, a NASA product: all of
    // them are this container and nothing in the first bytes says which.
    (b"\xfe\xed\xfa\xce", "macho"),
    (b"\xfe\xed\xfa\xcf", "macho"),
    (b"\xce\xfa\xed\xfe", "macho"),
    (b"\xcf\xfa\xed\xfe", "macho"),
    (b"\x89HDF\r\n\x1a\n", "hdf5"),
    (b"ID3", "id3"),
    (b"\x00\x05\x16\x07", "appledouble"),
    (b"\x00\x05\x16\x00", "applesingle"),
    (b"{\"", "json"),
];

/// How much of a file `sniff` wants. Most formats are decided by their first
/// few bytes, and a handful are not: a MOD reads its signature at 1080, an
/// Anvil region needs both its 8 KiB tables, and an ISO 9660 image keeps its
/// first volume descriptor at sector 16, which is 32 KiB in. A caller that
/// reads less than this gets a `None` for the deep formats rather than an
/// error, so read this much and the answer is the whole answer.
pub const SNIFF_WINDOW: usize = 0x9000;

/// Pick a built-in template from the first bytes of a file. `len` is the
/// length of the whole file, which a format whose header is a table of
/// offsets needs in order to weigh what the table points at: the head alone
/// cannot say whether the offsets reach past the end.
///
/// The careful tests go first and the table of signatures second. That is the
/// wrong way round from how it reads, and it is deliberate: a prefix of two or
/// three bytes is weaker evidence than a test that looks at several things and
/// weighs them, so the tests get first refusal. `{"` is a JSON file and it is
/// also the size and checksum an LHA archive could open with, and only one of
/// the two knows enough to say so.
/// One question asked of a file, and what it answers.
enum Probe {
    /// A test written for one format, which answers yes or no.
    Is(&'static str, fn(&[u8], u64) -> bool),
    /// A test that says which of several formats it found, because telling
    /// those apart is the same piece of work as recognising any of them: a
    /// camera raw file and the ELF of a particular machine are each one test
    /// with several answers.
    Which(fn(&[u8], u64) -> Option<&'static str>),
    /// The table of leading signatures, asked at the point it reaches in the
    /// order below rather than first or last.
    Signatures,
}

/// The questions, in the order they are asked, which is the whole of what this
/// list says. Order is the meaning here, so it is written down as order rather
/// than left implicit in the shape of a chain of `else`.
///
/// Three bands, and a format belongs to the band that matches how strong its
/// evidence is:
///
/// 1. tests that weigh several things about a file. A prefix of two or three
///    bytes is weaker evidence than a test that looks at a header and checks
///    it against the file's length, so these get first refusal. `{"` is a JSON
///    file and it is also the size and checksum an LHA archive could open
///    with, and only one of the two knows enough to say so.
/// 2. the signatures, for the formats that announce themselves and are done.
/// 3. tests that recognise a file by the shape of everything in it rather than
///    by its front. A file with no header of its own gets to be identified
///    only after everything that does have one has spoken.
const PROBES: &[Probe] = &[
    Probe::Is("braw", |h, _| is_braw(h)),
    Probe::Is("aseprite", |h, _| h.get(4..6) == Some(&[0xe0, 0xa5])),
    Probe::Is("xm", |h, _| h.starts_with(b"Extended Module: ")),
    Probe::Is("it", |h, _| h.starts_with(b"IMPM")),
    Probe::Is("s3m", |h, _| is_s3m(h)),
    Probe::Is("mod", |h, _| is_mod(h)),
    Probe::Is("mca", is_mca),
    Probe::Is("macbinary", is_macbinary),
    Probe::Is("binhex", |h, _| is_binhex(h)),
    Probe::Is("stuffit", |h, _| is_stuffit(h)),
    Probe::Is("compactpro", is_compactpro),
    Probe::Is("bardstale", is_bards_tale),
    Probe::Is("whisper", |h, _| is_whisper(h)),
    Probe::Is("safetensors", |h, _| is_safetensors(h)),
    Probe::Is("hackrffw", |h, _| is_hackrf_firmware(h)),
    Probe::Is("gdbm", |h, _| gdbm::is_gdbm(h)),
    Probe::Is("bdb", |h, _| bdb::is_bdb(h)),
    Probe::Which(|h, _| camera_raw_format(h)),
    Probe::Is("mp4", |h, _| h.len() >= 8 && &h[4..8] == b"ftyp"),
    Probe::Is("mkv", |h, _| is_mkv(h)),
    Probe::Is("iso9660", |h, _| is_iso9660(h)),
    Probe::Is("dv", is_dv),
    Probe::Which(|h, _| elf_format(h)),
    Probe::Is("macho", is_universal),
    Probe::Is("self", |h, _| is_self(h)),
    Probe::Is("pak", is_pak),
    Probe::Is("ne", |h, _| is_ne(h)),
    Probe::Is("le", |h, _| is_le(h)),
    Probe::Is("pe", |h, _| is_pe(h)),
    Probe::Is("coff", is_coff),
    Probe::Is("omf", |h, _| is_omf(h)),
    Probe::Is("msdos", is_dos),
    Probe::Is("zarrzip", |h, _| is_zarr_zip(h)),
    Probe::Is("lha", |h, _| is_lha(h)),
    Probe::Is("lnk", |h, _| is_lnk(h)),
    Probe::Is("bmp", |h, _| is_bmp(h)),
    Probe::Is("pnm", |h, _| is_pnm(h)),
    Probe::Is("pcx", |h, _| is_pcx(h)),
    Probe::Is("ico", is_ico),
    Probe::Is("unityassets", is_unity_assets),
    Probe::Is("thumbsdb", |h, _| is_thumbs_db(h)),
    Probe::Is("deb", |h, _| is_deb(h)),
    Probe::Which(assimp_format),
    Probe::Signatures,
    // The Amiga container, whose form type says which format it holds.
    Probe::Which(|h, _| match h.len() >= 12 && h.starts_with(b"FORM") {
        false => None,
        true => match &h[8..12] {
            b"AIFF" | b"AIFC" => Some("aiff"),
            b"ILBM" | b"PBM " => Some("ilbm"),
            _ => None,
        },
    }),
    // A record of who logged in, which has no header at all: it is recognised
    // by every record in it being the right size and shape.
    Probe::Is("utmp", utmp::is_utmp),
    // The same, and for the same reason: a stream of space packets is
    // recognised by one packet's length landing on the next one's header,
    // which is evidence about the whole file rather than about its front.
    Probe::Is("spp", spp::is_spp),
    // A torrent, and any other bencoded file: a dictionary of byte strings
    // whose parse covers the whole file. Nothing marks the front of one, so
    // what recognises it is reading all of it.
    Probe::Is("bencode", bencode::is_bencode),
    Probe::Is("cdr", |h, _| h.starts_with(b"RIFF") && h.len() >= 12 && h[8..11] == *b"CDR"),
    Probe::Is("cmx", |h, _| h.starts_with(b"RIFF") && h.get(8..12) == Some(b"CMX1")),
    // A sound file, and the one variant of it that is marked by a tag inside
    // the first chunk rather than by anything in the first twelve bytes.
    Probe::Which(|h, _| {
        let riff = h.starts_with(b"RIFF") || h.starts_with(b"RF64") || h.starts_with(b"RIFX");
        if !riff || h.len() < 12 || &h[8..12] != b"WAVE" {
            return None;
        }
        match h.len() >= 22 && &h[12..16] == b"fmt " && &h[20..22] == b"AW" {
            true => Some("w4v"),
            false => Some("wav"),
        }
    }),
];

/// Pick a built-in template from the first bytes of a file. `len` is the
/// length of the whole file, which a format whose header is a table of
/// offsets needs in order to weigh what the table points at: the head alone
/// cannot say whether the offsets reach past the end.
///
/// The order the questions are asked in is [`PROBES`], which says why.
pub fn sniff(head: &[u8], len: u64) -> Option<&'static str> {
    PROBES.iter().find_map(|probe| match probe {
        Probe::Is(name, test) => test(head, len).then_some(*name),
        Probe::Which(test) => test(head, len),
        Probe::Signatures => {
            MAGIC.iter().find(|(magic, _)| head.starts_with(magic)).map(|(_, name)| *name)
        }
    })
}

/// A SELF file: a SQLite database whose application id is the four letters
/// `SELF`. Every SELF file is a valid SQLite database, so this is asked
/// before the magic table sends it to the plain `sqlite` template.
fn is_self(head: &[u8]) -> bool {
    head.starts_with(b"SQLite format 3\0") && head.get(68..72) == Some(b"SELF")
}

/// An ELF, and which of the two templates reads it. The machine is at offset
/// 18 whichever class the file is, and it is written the way the file writes
/// its numbers, so the byte at offset 5 has to be read before it can be.
/// A machine of 247 is BPF, whose instructions the `bpf` template decodes;
/// anything else is an ELF whose code is bytes.
fn elf_format(head: &[u8]) -> Option<&'static str> {
    if !head.starts_with(b"\x7fELF") || head.len() < 20 {
        return None;
    }
    let machine = match head[5] {
        1 => u16::from_le_bytes([head[18], head[19]]),
        2 => u16::from_be_bytes([head[18], head[19]]),
        _ => return None,
    };
    Some(if machine == 247 { "bpf" } else { "elf" })
}

fn is_ico(head: &[u8], len: u64) -> bool {
    if head.len() < 6 || head[..2] != [0, 0] || !matches!(&head[2..4], [1, 0] | [2, 0]) {
        return false;
    }
    let count = u16::from_le_bytes([head[4], head[5]]) as usize;
    if count == 0 || count > 256 || head.len() < 6 + count * 16 { return false; }
    let table_end = (6 + count * 16) as u64;
    (0..count).all(|i| {
        let at = 6 + i * 16;
        let size = u32::from_le_bytes(head[at + 8..at + 12].try_into().unwrap()) as u64;
        let offset = u32::from_le_bytes(head[at + 12..at + 16].try_into().unwrap()) as u64;
        size > 0 && offset >= table_end && offset.checked_add(size).is_some_and(|end| end <= len)
    })
}

fn is_unity_assets(head: &[u8], len: u64) -> bool {
    if head.len() < 20 { return false; }
    let be32 = |at: usize| u32::from_be_bytes(head[at..at + 4].try_into().unwrap());
    let version = be32(8);
    if !(9..=23).contains(&version) || !matches!(head[16], 0 | 1) { return false; }
    let (metadata, file_size, data_offset, header_size) = if version >= 22 {
        if head.len() < 48 { return false; }
        (
            be32(20) as u64,
            u64::from_be_bytes(head[24..32].try_into().unwrap()),
            u64::from_be_bytes(head[32..40].try_into().unwrap()),
            48u64,
        )
    } else {
        (be32(0) as u64, be32(4) as u64, be32(12) as u64, 20u64)
    };
    metadata >= 8
        && file_size <= len
        && len - file_size < 16
        && data_offset >= header_size + metadata
        && data_offset <= file_size
}

/// A Debian package: an `ar` archive whose first member is the version stamp
/// that says it is one. Every other archive of this shape is a library, and
/// only the name of that first member tells the two apart.
fn is_deb(head: &[u8]) -> bool {
    head.starts_with(ar::MAGIC) && head.get(8..8 + ar::DEBIAN_BINARY.len()) == Some(ar::DEBIAN_BINARY.as_bytes())
}

fn is_thumbs_db(head: &[u8]) -> bool {
    const CFB: &[u8] = b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1";
    const CATALOG: &[u8] = b"C\0a\0t\0a\0l\0o\0g\0";
    head.starts_with(CFB) && head.windows(CATALOG.len()).any(|w| w == CATALOG)
}

/// BRAW is a QuickTime-family container, but unlike ordinary MP4 camera files
/// it commonly starts with a `wide` compatibility box followed by `mdat`.
/// Requiring both per-frame markers keeps this from claiming generic MOVs.
fn is_braw(head: &[u8]) -> bool {
    head.get(4..8) == Some(b"wide")
        && head.get(12..16) == Some(b"mdat")
        && head.windows(4).any(|w| w == b"bmdf")
        && head.windows(4).any(|w| w == b"braw")
}

/// Assimp formats with enough evidence in their bytes to identify safely.
/// Text grammars such as OBJ and NFF have no mandatory opening token and stay
/// manual templates. ZIP packages also remain ZIP unless a model-specific
/// member name is visible in the fetched prefix.
fn assimp_format(head: &[u8], len: u64) -> Option<&'static str> {
    let prefix = &head[..head.len().min(4096)];
    let has = |needle: &[u8]| prefix.windows(needle.len()).any(|w| w == needle);
    let binary_stl = head
        .get(80..84)
        .and_then(|bytes| bytes.try_into().ok())
        .map(u32::from_le_bytes)
        .and_then(|triangles| 84u64.checked_add(u64::from(triangles).checked_mul(50)?))
        == Some(len);

    if head.starts_with(b"ASSIMP.binary-dump.") {
        Some("assbin")
    } else if head.starts_with(b"Kaydara FBX Binary  \0\x1a\0") {
        Some("fbx")
    } else if head.starts_with(b"INTERQUAKEMODEL\0") {
        Some("iqm")
    } else if head.starts_with(b"MS3D000000") {
        Some("ms3d")
    } else if head.starts_with(b"PXR-USDC") {
        Some("usdc")
    } else if head.starts_with(b"TERRAGENTERRAIN ") {
        Some("ter")
    } else if head.starts_with(b"BLENDER") {
        Some("blend")
    } else if head.starts_with(b"BB3D") {
        Some("b3d")
    } else if head.starts_with(b"glTF") {
        Some("glb")
    } else if head.starts_with(b"IDP2") {
        Some("md2")
    } else if head.starts_with(b"IDP3") {
        Some("md3")
    } else if head.starts_with(b"IDPC") {
        Some("mdc")
    } else if matches!(head.get(..4), Some(b"IDPO" | b"IDST" | b"IDSQ" | b"MDL2" | b"MDL3" | b"MDL4" | b"MDL5" | b"MDL7")) {
        Some("mdl")
    } else if matches!(head.get(..4), Some(b"HMP4" | b"HMP5" | b"HMP7")) {
        Some("hmp")
    } else if matches!(head.get(..4), Some(b"3DMO" | b"3dmo")) {
        Some("m3d")
    } else if head.starts_with(b"PMX ") {
        Some("pmx")
    } else if head.starts_with(b"IBSP") {
        Some("q3bsp")
    } else if head.starts_with(b"quick3Do") {
        Some("q3o")
    } else if head.starts_with(b"quick3Ds") {
        Some("q3s")
    } else if head.starts_with(b"nendo ") {
        Some("ndo")
    } else if head.starts_with(b"xof ") {
        Some("x")
    } else if head.starts_with(b"FORM") && matches!(head.get(8..12), Some(b"LWOB" | b"LWO2" | b"LXOB")) {
        match head.get(8..12) {
            Some(b"LXOB") => Some("lxo"),
            _ => Some("lwo"),
        }
    } else if head.starts_with(b"AC3D") {
        Some("ac3d")
    } else if head.starts_with(b"ply\n") || head.starts_with(b"ply\r") {
        Some("ply")
    } else if head.starts_with(b"OFF\n") || head.starts_with(b"OFF\r") || head.starts_with(b"OFF ") {
        Some("off")
    } else if head.starts_with(b"HIERARCHY") {
        Some("bvh")
    } else if head.starts_with(b"MD5Version ") {
        Some("md5")
    } else if head.starts_with(b"#X3D ") {
        Some("x3dv")
    } else if head.starts_with(b"ISO-10303-21;") {
        Some("ifc")
    } else if head.starts_with(b"AutoCAD Binary DXF\r\n\x1a") {
        Some("dxf")
    } else if head.starts_with(b"PK") && has(b"3D/3dmodel.model") {
        Some("3mf")
    } else if head.starts_with(b"{") && has(b"\"asset\"") && has(b"\"version\"")
    {
        Some("gltf")
    } else if has(b"<COLLADA") {
        Some("collada")
    } else if has(b"<X3D") {
        Some("x3d")
    } else if has(b"<amf") {
        Some("amf")
    } else if (head.starts_with(b"solid ") && has(b"facet normal") && has(b"outer loop")) || binary_stl
    {
        Some("stl")
    } else if head.len() >= 6 && head.starts_with(b"MM") {
        let declared = u32::from_le_bytes(head[2..6].try_into().expect("four bytes")) as u64;
        (declared == len && len >= 6).then_some("3ds")
    } else {
        None
    }
}

/// Matroska is EBML with a `DocType` of `matroska`; the EBML signature alone
/// would also claim WebM and unrelated EBML documents.
fn is_mkv(head: &[u8]) -> bool {
    head.starts_with(b"\x1a\x45\xdf\xa3")
        && head[..256.min(head.len())].windows(8).any(|w| w == b"matroska")
}

/// ECMA-119 records its first descriptor at sector 16. The identifier is five
/// bytes into that sector after the descriptor type.
fn is_iso9660(head: &[u8]) -> bool {
    matches!(head.get(16 * 2048), Some(0..=3))
        && head.get(16 * 2048 + 1..16 * 2048 + 6) == Some(b"CD001")
}

/// Raw DV has no single magic number. Eight consecutive DIF IDs establish the
/// fixed header/subcode/VAUX/audio/video order of the first sequence, and a
/// whole file is an integral count of 525/60 or 625/50 frames.
fn is_dv(head: &[u8], len: u64) -> bool {
    if len == 0 || (!len.is_multiple_of(120_000) && !len.is_multiple_of(144_000)) || head.len() < 8 * 80 {
        return false;
    }
    let section = [0u8, 1, 1, 2, 2, 2, 3, 4];
    let number = [0u8, 0, 1, 0, 1, 2, 0, 0];
    (0..8).all(|i| {
        let at = i * 80;
        head[at] >> 5 == section[i] && head[at + 1] >> 4 == 0 && head[at + 2] == number[i]
    })
}

/// A standalone Microsoft COFF object has no magic. Its known machine, zero
/// optional-header length, bounded section table, and every file pointer have
/// to agree with the file length before it is claimed.
fn is_coff(head: &[u8], len: u64) -> bool {
    if head.len() < 20 {
        return false;
    }
    let u16_at = |at: usize| u16::from_le_bytes([head[at], head[at + 1]]);
    let u32_at = |at: usize| u32::from_le_bytes([head[at], head[at + 1], head[at + 2], head[at + 3]]);
    let machine = u16_at(0);
    if !matches!(machine, 0x014c | 0x0166 | 0x01c0 | 0x01c4 | 0x01f0 | 0x0200 | 0x5032 | 0x5064 | 0x5128 | 0x8664 | 0xaa64)
        || u16_at(16) != 0
    {
        return false;
    }
    let sections = usize::from(u16_at(2));
    let Some(table_size) = sections.checked_mul(40) else { return false };
    let Some(table_end) = 20usize.checked_add(table_size) else { return false };
    if sections == 0 || sections > 96 || table_end > head.len() || table_end as u64 > len {
        return false;
    }
    for i in 0..sections {
        let at = 20 + i * 40;
        let raw_size = u64::from(u32_at(at + 16));
        let raw_at = u64::from(u32_at(at + 20));
        let reloc_at = u64::from(u32_at(at + 24));
        let relocs = u64::from(u16_at(at + 32));
        if (raw_size != 0 && (raw_at < table_end as u64 || raw_at.checked_add(raw_size).is_none_or(|end| end > len)))
            || (relocs != 0 && (reloc_at < table_end as u64 || reloc_at.checked_add(relocs * 10).is_none_or(|end| end > len)))
        {
            return false;
        }
    }
    let symbols_at = u64::from(u32_at(8));
    let symbols = u64::from(u32_at(12));
    (symbols_at == 0 && symbols == 0)
        || (symbols_at >= table_end as u64
            && symbols_at.checked_add(symbols.saturating_mul(18)).is_some_and(|end| end + 4 <= len))
}

/// OMF modules begin with a named THEADR/LHEADR record. The length includes
/// the checksum; the checksum is either zero (explicitly permitted) or makes
/// the byte sum of the complete record zero modulo 256.
fn is_omf(head: &[u8]) -> bool {
    if !matches!(head.first(), Some(0x80 | 0x82)) || head.len() < 5 {
        return false;
    }
    let len = usize::from(u16::from_le_bytes([head[1], head[2]]));
    let end = 3usize.saturating_add(len);
    if len < 2 || end > head.len() || usize::from(head[3]) > len - 2 {
        return false;
    }
    head[end - 1] == 0 || head[..end].iter().fold(0u8, |sum, &b| sum.wrapping_add(b)) == 0
}

fn is_s3m(head: &[u8]) -> bool {
    head.get(28) == Some(&0x1a) && head.get(29) == Some(&16) && head.get(44..48) == Some(b"SCRM")
}

fn is_mod(head: &[u8]) -> bool {
    let Some(sig) = head.get(1080..1084) else { return false };
    matches!(sig, b"M.K." | b"M!K!" | b"M&K!" | b"FLT4" | b"FLT8" | b"4CHN" | b"6CHN" | b"8CHN" | b"OKTA" | b"CD81")
        || (sig[0].is_ascii_digit() && sig[1..] == *b"CHN")
        || (sig[..2].iter().all(u8::is_ascii_digit) && matches!(&sig[2..], b"CH" | b"CN"))
}

/// Identify the TIFF-based camera RAW formats whose bytes distinguish them.
/// Extension names are deliberately not involved: dropped files can be
/// renamed, and a normal TIFF should not become a NEF merely because of its
/// filename. DNG has its own version tag; CR2, ORF, and RW2 have fixed header
/// markers; the remaining TIFF variants require both a camera make and RAW
/// sensor metadata.
fn camera_raw_format(head: &[u8]) -> Option<&'static str> {
    if head.get(8..12) == Some(b"CR\x02\0") && (head.starts_with(b"II*\0") || head.starts_with(b"MM\0*")) {
        return Some("cr2");
    }
    if head.starts_with(b"IIRO") || head.starts_with(b"MMOR") {
        return Some("orf");
    }
    if head.starts_with(b"IIU\0") {
        return Some("rw2");
    }

    let little = if head.starts_with(b"II*\0") {
        true
    } else if head.starts_with(b"MM\0*") {
        false
    } else {
        return None;
    };
    let u16_at = |at: usize| -> Option<u16> {
        let b: [u8; 2] = head.get(at..at + 2)?.try_into().ok()?;
        Some(if little { u16::from_le_bytes(b) } else { u16::from_be_bytes(b) })
    };
    let u32_at = |at: usize| -> Option<u32> {
        let b: [u8; 4] = head.get(at..at + 4)?.try_into().ok()?;
        Some(if little { u32::from_le_bytes(b) } else { u32::from_be_bytes(b) })
    };
    let ifd = usize::try_from(u32_at(4)?).ok()?;
    let count = usize::from(u16_at(ifd)?);
    let mut make: Option<&[u8]> = None;
    let mut raw_sensor = false;
    for i in 0..count.min(4096) {
        let at = ifd.checked_add(2)?.checked_add(i.checked_mul(12)?)?;
        let tag = u16_at(at)?;
        let kind = u16_at(at + 2)?;
        let values = usize::try_from(u32_at(at + 4)?).ok()?;
        if tag == 50706 {
            return Some("dng");
        }
        raw_sensor |= matches!(tag, 33421 | 33422 | 34713 | 37398 | 50710..=50741 | 50778..=50834);
        if tag == 271 && kind == 2 && values > 0 {
            let start = if values <= 4 { at + 8 } else { usize::try_from(u32_at(at + 8)?).ok()? };
            make = head.get(start..start.checked_add(values.min(64))?);
        }
    }
    if !raw_sensor {
        return None;
    }
    let make = make?;
    if starts_ascii_case_insensitive(make, b"NIKON") {
        Some("nef")
    } else if starts_ascii_case_insensitive(make, b"SONY") {
        Some("arw")
    } else if starts_ascii_case_insensitive(make, b"PENTAX") || starts_ascii_case_insensitive(make, b"RICOH") {
        Some("pef")
    } else if starts_ascii_case_insensitive(make, b"SAMSUNG") {
        Some("srw")
    } else {
        None
    }
}

fn starts_ascii_case_insensitive(value: &[u8], prefix: &[u8]) -> bool {
    value.get(..prefix.len()).is_some_and(|s| s.eq_ignore_ascii_case(prefix))
}

/// Whether these leading bytes are a MacBinary envelope.
///
/// The format has no magic number at the front: the header opens with a zero
/// byte, the length of the name and the name itself, and the fork lengths it
/// carries have to add up to the file. That much is only a handful of bytes,
/// and a table of small big-endian numbers can satisfy all of it by accident,
/// so the version at 122 decides how the rest is weighed. MacBinary II and
/// III sign their header with a CRC of it; MacBinary I has no CRC, so instead
/// the type and creator have to be four printable characters each, which the
/// four-letter codes of a real file always are and a table of offsets is not.
fn is_macbinary(head: &[u8], len: u64) -> bool {
    if head.len() < 128 || head[0] != 0 || head[74] != 0 || head[82] != 0 || !(1..=63).contains(&head[1]) {
        return false;
    }
    let name = &head[2..2 + head[1] as usize];
    if name.iter().any(|&b| b < 0x20 || b == 0x7f) {
        return false;
    }
    let be32 = |at: usize| u32::from_be_bytes(head[at..at + 4].try_into().expect("four bytes")) as u64;
    let be16 = |at: usize| u16::from_be_bytes(head[at..at + 2].try_into().expect("two bytes")) as u64;
    let signed = match head[122] {
        129 | 130 => header_crc(&head[..124]) == be16(124) as u16,
        0 => head[65..73].iter().all(|&b| (0x20..0x7f).contains(&b)),
        _ => false,
    };
    let blocks = |n: u64| n.saturating_add(127) / 128 * 128;
    signed
        && 128u64.saturating_add(blocks(be16(120))).saturating_add(blocks(be32(83))).saturating_add(blocks(be32(87))).saturating_add(blocks(be16(99))) <= len
}

/// The CRC-16 a MacBinary II header is signed with: XMODEM, polynomial
/// 0x1021, starting at zero, high bit first.
fn header_crc(bytes: &[u8]) -> u16 {
    let mut crc = 0u16;
    for &b in bytes {
        crc ^= u16::from(b) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 { (crc << 1) ^ 0x1021 } else { crc << 1 };
        }
    }
    crc
}

fn is_binhex(head: &[u8]) -> bool {
    head.windows(40).any(|w| w == b"(This file must be converted with BinHex")
}

fn is_stuffit(head: &[u8]) -> bool {
    const SIGS: [&[u8; 4]; 9] = [b"SIT!", b"ST46", b"ST50", b"ST60", b"ST65", b"STin", b"STi2", b"STi3", b"STi4"];
    let classic = head.get(10..14) == Some(b"rLau") && head.get(..4).is_some_and(|s| SIGS.iter().any(|magic| s == *magic));
    let sit5 = head.starts_with(b"StuffIt (c)1997-") && head.get(20..78) == Some(b" Aladdin Systems, Inc., http://www.aladdinsys.com/StuffIt/");
    classic || sit5
}

fn is_compactpro(head: &[u8], len: u64) -> bool {
    let Some(raw) = head.get(4..8) else { return false };
    let directory = u32::from_be_bytes(raw.try_into().expect("four bytes")) as u64;
    head.get(..2) == Some(&[1, 1]) && (8..len).contains(&directory)
}

/// Whether these bytes are a Bard's Tale I DOS `.TPW` file.
///
/// The format has no magic number. Its two record sizes, the discriminator at
/// byte 16, a printable NUL-padded name and the zero high bytes of the first
/// three character enums together make a much narrower signature than size or
/// extension alone.
fn is_bards_tale(head: &[u8], len: u64) -> bool {
    if head.len() < 17 || !matches!((len, head[16]), (109, 1) | (113, 2)) {
        return false;
    }
    let name = &head[..16];
    let Some(end) = name.iter().position(|&b| b == 0) else { return false };
    if end == 0 || !name[..end].iter().all(|&b| b.is_ascii_graphic() || b == b' ') || name[end..].iter().any(|&b| b != 0) {
        return false;
    }
    if head[16] == 2 {
        return true;
    }
    head.len() >= 23
        && head[18] == 0
        && head[20] == 0
        && head[22] == 0
        && head[17] & 1 == 0
        && head[19] <= 6
        && head[21] <= 9
}

/// Whether these leading bytes are a Windows bitmap.
///
/// `BM` on its own is two letters a text file could open with, so this also
/// wants the DIB header after it to be one of the sizes the five versions of
/// that header have.
fn is_bmp(head: &[u8]) -> bool {
    let Some(b) = head.get(14..18) else { return false };
    head.starts_with(b"BM")
        && matches!(u32::from_le_bytes([b[0], b[1], b[2], b[3]]), 12 | 40 | 52 | 56 | 64 | 108 | 124)
}

/// Whether these leading bytes are a PCX.
///
/// The first byte is 0x0A and nothing else, but one byte is not enough on its
/// own, so the version and the encoding after it have to be values the format
/// defines too. Version 1 was never used and 0x0A is a newline, so a text file
/// starting with a blank line is turned away by the encoding byte.
fn is_pcx(head: &[u8]) -> bool {
    head.first() == Some(&0x0a) && matches!(head.get(1), Some(0 | 2 | 3 | 4 | 5)) && head.get(2) == Some(&1)
}

/// Whether these leading bytes are an LHA archive.
///
/// Nothing marks the front of the file: the first two bytes are a header size
/// and a checksum, which can be anything. What is fixed is the method at
/// offset 2, five characters of `-lh`, a digit or letter, and `-`. That is the
/// signature every tool that identifies these files uses.
fn is_lha(head: &[u8]) -> bool {
    matches!(head.get(2..7), Some([b'-', b'l', b'h' | b'z', _, b'-']))
}

/// Whether these leading bytes are a Zarr store written into a ZIP.
///
/// A ZipStore is an ordinary archive whose entries are a store's keys, so the
/// only thing that tells one from any other ZIP is the names inside it. This
/// walks the local file headers in the head window looking for a metadata key:
/// `.zarray`, `.zgroup` or `.zattrs` for a v2 store, `zarr.json` for a v3 one.
///
/// The walk needs each record's length to reach the next, and a streamed entry
/// writes zero there and its real size in a descriptor after the data. Rather
/// than scan for that descriptor, the walk stops: what it has read so far is
/// still allowed to answer. It stops at the central directory too, which is
/// where the names run out.
///
/// Entry order is a writer's choice, so a store whose metadata lands past the
/// window is not recognised here. The archive still opens as a ZIP, and the
/// contents view names the store from the entries themselves.
fn is_zarr_zip(head: &[u8]) -> bool {
    let mut at = 0usize;
    while let Some(record) = head.get(at..at.saturating_add(30)) {
        if record[..4] != *b"PK\x03\x04" {
            return false;
        }
        let u16_at = |i: usize| u16::from_le_bytes([record[i], record[i + 1]]) as usize;
        let u32_at = |i: usize| u32::from_le_bytes([record[i], record[i + 1], record[i + 2], record[i + 3]]) as usize;
        let flags = u16_at(6);
        let compressed = u32_at(18);
        let name_length = u16_at(26);
        let extra_length = u16_at(28);
        let names_at = at + 30;
        let Some(name) = head.get(names_at..names_at.saturating_add(name_length)) else {
            return false;
        };
        if is_zarr_key(name) {
            return true;
        }
        // A streamed entry writes zero for its size here and the real one in a
        // descriptor after the data, so there is no way on to the next record.
        if flags & 8 != 0 {
            return false;
        }
        let extra_at = names_at + name_length;
        let Some(extra) = head.get(extra_at..extra_at.saturating_add(extra_length)) else {
            return false;
        };
        let Some(compressed) = entry_size(compressed, extra) else {
            return false;
        };
        let Some(next) = extra_at.checked_add(extra_length).and_then(|n| n.checked_add(compressed)) else {
            return false;
        };
        at = next;
    }
    false
}

/// How long a local entry's data is, from wherever the header put it: the
/// size field, or the extra field tagged 1 when the size field is the
/// placeholder a ZIP64 entry writes there. Nothing when the placeholder is
/// there and no extra field answers it, since there is then no way on to the
/// next record from here.
fn entry_size(size: usize, extra: &[u8]) -> Option<usize> {
    if size != 0xFFFF_FFFF {
        return Some(size);
    }
    let mut at = 0usize;
    while let Some(record) = extra.get(at..at.saturating_add(4)) {
        let id = u16::from_le_bytes([record[0], record[1]]) as usize;
        let len = u16::from_le_bytes([record[2], record[3]]) as usize;
        let body = extra.get(at + 4..at + 4 + len)?;
        // The local header's record holds the unpacked size and then the
        // compressed one, both eight bytes, both always written.
        if id == 1 {
            let packed = body.get(8..16)?;
            return usize::try_from(u64::from_le_bytes(packed.try_into().ok()?)).ok();
        }
        at += 4 + len;
    }
    None
}

/// Whether an archive entry's name is one of a Zarr store's metadata keys.
fn is_zarr_key(name: &[u8]) -> bool {
    let leaf = name.rsplit(|&b| b == b'/' || b == b'\\').next().unwrap_or(name);
    matches!(leaf, b".zarray" | b".zgroup" | b".zattrs" | b"zarr.json")
}

/// Whether these leading bytes are a Shell Link (`.lnk`).  Its header size
/// and LinkCLSID together are fixed by the format, which is strong enough to
/// identify a shortcut without relying on its filename extension.
fn is_lnk(head: &[u8]) -> bool {
    head.len() >= 20
        && head[..4] == [0x4c, 0, 0, 0]
        && head[4..20] == [
            0x01, 0x14, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00,
            0xc0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46,
        ]
}

/// Whether these leading bytes are a netpbm file.
///
/// P and a digit from 1 to 6, and then whitespace. Two characters alone would
/// claim any text file that happened to start with them, so the byte after the
/// digit has to be one that could separate the magic from the width.
fn is_pnm(head: &[u8]) -> bool {
    head.first() == Some(&b'P')
        && matches!(head.get(1), Some(b'1'..=b'6'))
        && matches!(head.get(2), Some(b' ' | b'\t' | b'\n' | b'\r'))
}

/// A HackRF firmware image, which is what a PortaPack build is. A flash image
/// starts with the processor's vector table and has no magic number of its
/// own, so what marks one is the record HackRF's build puts at a fixed offset
/// to say which version it is.
fn is_hackrf_firmware(head: &[u8]) -> bool {
    head.get(hackrffw::MAGIC_AT..hackrffw::MAGIC_AT + hackrffw::MAGIC.len()) == Some(hackrffw::MAGIC)
}

/// Whether these leading bytes are a whisper.cpp model.
///
/// `lmgg` is `ggml` written as a 32-bit number, and every model file ggml wrote
/// before GGUF opens with it, including the llama.cpp ones of the same era.
/// What tells a whisper model apart is `n_mels` at 0x28: an audio model has 80
/// mel bands, or 128 for large-v3, and a language model has something else
/// there entirely.
fn is_whisper(head: &[u8]) -> bool {
    if !head.starts_with(b"lmgg") || head.len() < 0x2c {
        return false;
    }
    matches!(u32::from_le_bytes([head[0x28], head[0x29], head[0x2a], head[0x2b]]), 80 | 128)
}

/// Whether these leading bytes are a safetensors file.
///
/// The format has no magic number: it opens with the length of its JSON
/// header, and then the header. So what is looked for is a length that could
/// be one, followed by the two characters an object whose first key is a
/// string starts with. A header is tens of kilobytes on the smallest real
/// model and a few megabytes on the largest.
fn is_safetensors(head: &[u8]) -> bool {
    let Some(len) = head.get(..8) else { return false };
    let len = u64::from_le_bytes(len.try_into().expect("eight bytes"));
    (2..=64 << 20).contains(&len) && head.get(8..10) == Some(b"{\"".as_slice())
}

/// Whether these leading bytes are a Minecraft Anvil region.
///
/// The format has no magic number: the file opens with its two tables, 1024
/// entries of four bytes each, and then the chunks the first table points at.
/// Nothing about one entry distinguishes it from noise, so the file is told by
/// the shape of all of them at once. Most entries of a real region are all
/// zeroes, the chunks never having been generated, and every one that is not
/// points past both tables, stays inside the file, and carries a length of at
/// least one sector. A single entry against that is a table of something
/// else, so it turns the file away rather than being averaged out.
///
/// The second table backs the first: entries there are seconds since 1970,
/// and a chunk a world has saved has a stamp between 2010, the years the game
/// is from, and 2100. The counts are weighed apart, one entry to the two
/// tables or more, because a world is saved a chunk at a time and stamps
/// outlive their entries and entries their stamps.
///
/// The head must hold both tables whole: an 8 KiB read is the smallest that
/// says anything about this format, and a shorter one is no evidence at all.
/// The length of the file is the other witness, since the pointers are only
/// worth weighing against the size they address.
fn is_mca(head: &[u8], len: u64) -> bool {
    if head.len() < 8192 || len < 12288 || len % 4096 != 0 {
        return false;
    }
    let mut present = 0;
    for i in 0..1024 {
        let at = i * 4;
        let sector = u32::from_be_bytes([0, head[at], head[at + 1], head[at + 2]]);
        let sectors = u32::from(head[at + 3]);
        if sector | sectors == 0 {
            continue;
        }
        let within = sector >= 2 && (u64::from(sector) + u64::from(sectors)) * 4096 <= len;
        if !within {
            return false;
        }
        present += 1;
    }
    if present == 0 {
        return false;
    }
    let mut dated = 0;
    for i in 0..1024 {
        let at = 4096 + i * 4;
        let stamp = u32::from_be_bytes([head[at], head[at + 1], head[at + 2], head[at + 3]]);
        // Seconds since 1970 between the start of 2010 and the start of 2100.
        if (1262304000..4102444800).contains(&stamp) {
            dated += 1;
        } else if stamp != 0 {
            return false;
        }
    }
    dated > 0 && dated * 2 >= present
}

/// Whether these leading bytes are a DOS executable and nothing newer.
///
/// Everything in the `MZ` family opens the same way, and what says whether a
/// header of a later format follows is `relocation_table` at 0x18: a DOS
/// program's relocations start before 0x40, which is where the pointer to such
/// a header would have to be. A file that leaves room for one is claimed here
/// only once the bytes it points at have been seen and are none of `PE`, `NE`,
/// `LE` or `LX`. A pointer past what has been read leaves the file unclaimed, which is
/// the same answer `is_pe` gives to a short read and for the same reason.
fn is_dos(head: &[u8], len: u64) -> bool {
    if !head.starts_with(b"MZ") || head.len() < 0x1c {
        return false;
    }
    if u16::from_le_bytes([head[0x18], head[0x19]]) < 0x40 {
        return true;
    }
    let Some(b) = head.get(0x3c..0x40) else { return false };
    let at = u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize;
    match head.get(at..at + 2) {
        Some(sig) => !matches!(sig, b"PE" | b"NE" | b"LE" | b"LX"),
        // An offset past the end of the file is not pointing at a header, so
        // there is none and this is a DOS program. Several packers leave
        // whatever they like in those four bytes: PKLITE writes the middle of
        // its copyright notice there, which reads as an offset of a gigabyte.
        //
        // Past what has been read but still inside the file is a different
        // answer: the header may be there, and the file stays unclaimed until
        // enough of it has arrived to say.
        None => at as u64 + 2 > len,
    }
}

/// Whether this is a universal binary: several Mach-O files in one, with a
/// table at the front saying which machine each of them is for.
///
/// The four bytes it opens with are a Java class file's as well, and the four
/// after them are what tells the two apart. A universal binary counts the
/// files inside it, and nobody ships one with hundreds; a class file writes
/// its version there, which has been at least 45 since 1996.
fn is_universal(head: &[u8], len: u64) -> bool {
    let sixty_four = match head.get(..4) {
        Some(b"\xca\xfe\xba\xbe") => false,
        Some(b"\xca\xfe\xba\xbf") => true,
        _ => return false,
    };
    let count = match head.get(4..8) {
        Some(b) => u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as u64,
        None => return false,
    };
    if !(1..=16).contains(&count) {
        return false;
    }
    // Where the first file inside is, and how long it is: both are in the
    // file if this is one.
    let (at, size) = match sixty_four {
        false => match (head.get(16..20), head.get(20..24)) {
            (Some(a), Some(s)) => (
                u32::from_be_bytes([a[0], a[1], a[2], a[3]]) as u64,
                u32::from_be_bytes([s[0], s[1], s[2], s[3]]) as u64,
            ),
            _ => return false,
        },
        true => match (head.get(16..24), head.get(24..32)) {
            (Some(a), Some(s)) => (
                u64::from_be_bytes(a.try_into().expect("eight bytes")),
                u64::from_be_bytes(s.try_into().expect("eight bytes")),
            ),
            _ => return false,
        },
    };
    at >= 8 && at.saturating_add(size) <= len
}

/// Whether this is a Quake archive rather than one of the other things that
/// open with `PACK`. A git packfile is the one that turns up: it writes a
/// big-endian version of 2 where the archive writes the offset of its
/// directory, and a directory of an archive is after the header and inside the
/// file, which 33 million rarely is.
fn is_pak(head: &[u8], len: u64) -> bool {
    if !head.starts_with(b"PACK") || head.len() < 12 {
        return false;
    }
    let at = u32::from_le_bytes([head[4], head[5], head[6], head[7]]) as u64;
    let size = u32::from_le_bytes([head[8], head[9], head[10], head[11]]) as u64;
    at >= 12 && at + size <= len
}

/// Whether these leading bytes are a 16-bit Windows or OS/2 program: an `MZ`
/// whose pointer at 0x3c reaches a header saying `NE`.
fn is_ne(head: &[u8]) -> bool {
    if !head.starts_with(b"MZ") || head.len() < 0x40 {
        return false;
    }
    let at = u32::from_le_bytes([head[0x3c], head[0x3d], head[0x3e], head[0x3f]]) as usize;
    match at.checked_add(2) {
        Some(end) if end <= head.len() => &head[at..end] == b"NE",
        _ => false,
    }
}

/// Whether these leading bytes are a linear executable: an `MZ` whose pointer
/// at 0x3c reaches a header saying `LE` or `LX`. One template reads both.
fn is_le(head: &[u8]) -> bool {
    if !head.starts_with(b"MZ") || head.len() < 0x40 {
        return false;
    }
    let at = u32::from_le_bytes([head[0x3c], head[0x3d], head[0x3e], head[0x3f]]) as usize;
    match at.checked_add(2) {
        Some(end) if end <= head.len() => matches!(&head[at..end], b"LE" | b"LX"),
        _ => false,
    }
}

/// Whether these leading bytes are a Windows executable rather than a DOS one.
///
/// Both open with `MZ`. What separates them is a PE signature at the offset
/// held at 0x3c, so this needs to see that far into the file: on the files
/// Windows ships that is 0x80 to 0x100, but nothing fixes it. A file whose
/// header sits past what has been read is left unclaimed rather than guessed
/// at, since claiming it would put a template on every DOS program too.
fn is_pe(head: &[u8]) -> bool {
    if !head.starts_with(b"MZ") || head.len() < 0x40 {
        return false;
    }
    let at = u32::from_le_bytes([head[0x3c], head[0x3d], head[0x3e], head[0x3f]]) as usize;
    match at.checked_add(4) {
        Some(end) if end <= head.len() => &head[at..end] == b"PE\0\0",
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `sniff` over a file that is exactly these bytes and no more.
    ///
    /// The length is a real argument, not a formality: a format with no
    /// signature is recognised by whether the table in its head points inside
    /// the file, which the head alone cannot say. Where a test cares what that
    /// length is, it calls `sniff` and gives one; where it does not, saying
    /// "the file is what you see" is the honest stand-in for a number nobody
    /// chose.
    fn sniffed(head: &[u8]) -> Option<&'static str> {
        sniff(head, head.len() as u64)
    }

    /// A name in the signature table with no template behind it would send a
    /// file to a template that does not exist, and nothing else here would
    /// notice: the table is data, and a name misspelt in it still compiles.
    #[test]
    fn every_name_a_signature_gives_has_a_template_and_is_listed() {
        for (_, name) in MAGIC {
            assert!(builtin(name).is_some(), "no template named {name}");
            assert!(builtin_names().contains(name), "{name} is not in the list of built-ins");
        }
    }

    /// A name written twice in the table would shadow the first quietly. The
    /// match this replaced made that a compile error; a list makes it data, so
    /// the guarantee has to be asked for.
    #[test]
    fn no_template_name_is_listed_twice() {
        let mut names = builtin_names();
        names.sort_unstable();
        let mut seen = names.clone();
        seen.dedup();
        assert_eq!(names, seen, "a name is listed more than once");
    }

    /// Both start with the same eight bytes, and only the name of the first
    /// member says which of the two a file is.
    #[test]
    fn a_debian_package_is_told_from_a_library_by_its_first_member() {
        assert_eq!(sniffed(b"!<arch>\ndebian-binary   1700000000  0     0     100644  4         `\n2.0\n"), Some("deb"));
        assert_eq!(sniffed(b"!<arch>\nhello.o/        1700000000  0     0     100644  4         `\n\x7fELF"), Some("ar"));
    }

    #[test]
    fn blackmagic_raw_needs_container_and_frame_markers() {
        let braw = b"\0\0\0\x08wide\0\0\0\x30mdat\0\0\0\x14bmdf\0\0\0\x0cexpo\x3f\xc0\0\0braw";
        assert_eq!(sniffed(braw), Some("braw"));

        let generic_mov = b"\0\0\0\x08wide\0\0\0\x18mdatgeneric payload";
        assert_ne!(sniffed(generic_mov), Some("braw"));
    }

    /// An `MZ` file that leaves room for a header of a later format, as
    /// everything from a Windows executable to a DOS extender does: its
    /// relocations start at 0x40, past the pointer at 0x3c.
    fn mz(pe_at: u32, len: usize, signature: bool) -> Vec<u8> {
        let mut v = vec![0u8; len];
        v[0..2].copy_from_slice(b"MZ");
        v[0x18..0x1a].copy_from_slice(&0x40u16.to_le_bytes());
        v[0x3c..0x40].copy_from_slice(&pe_at.to_le_bytes());
        if signature {
            let at = pe_at as usize;
            v[at..at + 4].copy_from_slice(b"PE\0\0");
        }
        v
    }

    #[test]
    fn a_windows_executable_is_a_pe() {
        assert_eq!(sniff(&mz(0x80, 0x100, true), 0x100), Some("pe"));
    }

    #[test]
    fn a_dos_executable_is_not() {
        // Relocations before 0x40, so there is no room for a later header.
        let mut v = vec![0u8; 0x100];
        v[0..2].copy_from_slice(b"MZ");
        v[0x18..0x1a].copy_from_slice(&0x1eu16.to_le_bytes());
        assert_eq!(sniff(&v, v.len() as u64), Some("msdos"));
        // Room left for one, and nothing in it.
        assert_eq!(sniff(&mz(0x80, 0x100, false), 0x100), Some("msdos"));
    }

    #[test]
    fn a_header_past_what_was_read_is_not_claimed() {
        // It may be a Windows executable, and reading it as a DOS one would
        // describe the stub that exists to say the program needs Windows.
        // The file is long enough to hold the header; only the read is short.
        assert_eq!(sniff(&mz(0x400, 0x100, false), 0x800), None);
        // Even a short read of a real PE: better unclaimed than wrong.
        assert_eq!(sniff(&mz(0x80, 0x40, false), 0x400), None);
    }

    /// A pointer past the end of the file points at nothing, whatever it says.
    /// PKLITE leaves the middle of its copyright notice in those four bytes,
    /// which reads as an offset of a gigabyte into a program of thirty
    /// kilobytes.
    /// Both open with `PACK`. What tells them apart is what comes next: an
    /// archive says where its directory is, and a git packfile says which
    /// version of itself it is.
    #[test]
    fn a_git_packfile_is_not_a_quake_archive() {
        let mut git = b"PACK".to_vec();
        git.extend_from_slice(&2u32.to_be_bytes());
        git.extend_from_slice(&14u32.to_be_bytes());
        git.resize(0x100, 0);
        assert_eq!(sniff(&git, git.len() as u64), None);

        let mut pak = b"PACK".to_vec();
        pak.extend_from_slice(&0x40u32.to_le_bytes());
        pak.extend_from_slice(&0x40u32.to_le_bytes());
        pak.resize(0x100, 0);
        assert_eq!(sniff(&pak, pak.len() as u64), Some("pak"));
    }

    #[test]
    fn a_pointer_past_the_end_of_the_file_is_not_a_header() {
        assert_eq!(sniff(&mz(0x4120_2e63, 0x100, false), 0x100), Some("msdos"));
    }

    #[test]
    fn a_16_bit_windows_program_is_an_ne_rather_than_a_dos_one() {
        let mut v = mz(0x80, 0x100, false);
        v[0x80..0x82].copy_from_slice(b"NE");
        assert_eq!(sniff(&v, v.len() as u64), Some("ne"));
    }

    #[test]
    fn both_linear_executables_read_with_the_one_template() {
        // A Windows VxD driver and a 32-bit OS/2 program: the same header
        // under two signatures.
        let mut v = mz(0x80, 0x100, false);
        v[0x80..0x82].copy_from_slice(b"LE");
        assert_eq!(sniff(&v, v.len() as u64), Some("le"));
        v[0x80..0x82].copy_from_slice(b"LX");
        assert_eq!(sniff(&v, v.len() as u64), Some("le"));
        // A signature nothing knows is a DOS program with something after it,
        // which is what the file is until a header says otherwise.
        v[0x80..0x82].copy_from_slice(b"LC");
        assert_eq!(sniff(&v, v.len() as u64), Some("msdos"));
    }

    #[test]
    fn a_whisper_model_is_told_from_the_other_ggml_files_by_its_mel_bands() {
        let mut v = b"lmgg".to_vec();
        v.resize(0x2c, 0);
        v[0x28..0x2c].copy_from_slice(&80u32.to_le_bytes());
        assert_eq!(sniff(&v, v.len() as u64), Some("whisper"));
        // A language model of the same era, which this cannot read.
        v[0x28..0x2c].copy_from_slice(&11008u32.to_le_bytes());
        assert_eq!(sniff(&v, v.len() as u64), None);
    }

    #[test]
    fn a_safetensors_file_is_told_by_its_header_length_and_the_json_after_it() {
        let mut v = 1024u64.to_le_bytes().to_vec();
        v.extend_from_slice(br#"{"a.weight":{"dtype":"F16""#);
        assert_eq!(sniff(&v, v.len() as u64), Some("safetensors"));
        // A length no header could have, whatever follows it.
        let mut v = u64::MAX.to_le_bytes().to_vec();
        v.extend_from_slice(br#"{"a":1}"#);
        assert_eq!(sniff(&v, v.len() as u64), None);
    }

    /// The front of a region: `present` chunks in consecutive sectors after
    /// the two tables, a saved stamp apiece, and every other entry zero, as
    /// most of the 1024 in a real region are.
    fn region(present: usize) -> Vec<u8> {
        let mut v = vec![0u8; 8192];
        for i in 0..present {
            let at = i * 4;
            let sector = 2u32 + i as u32;
            v[at..at + 3].copy_from_slice(&sector.to_be_bytes()[1..]);
            v[at + 3] = 1;
            v[4096 + at..4096 + at + 4].copy_from_slice(&1_700_000_000u32.to_be_bytes());
        }
        v
    }

    #[test]
    fn an_anvil_region_is_told_by_the_shape_of_its_tables() {
        assert_eq!(sniff(&region(182), (2 + 182) * 4096), Some("mca"));
        // Two chunks in the smallest region the tables allow.
        assert_eq!(sniff(&region(2), 4 * 4096), Some("mca"));
    }

    /// A region whose chunks sit past sector 256 has a non-zero second byte,
    /// which is the name length a MacBinary header would have there. The
    /// tables are the stronger evidence and are weighed first.
    #[test]
    fn a_region_is_not_taken_for_a_macbinary_envelope() {
        let mut v = region(182);
        let sector = 300u32;
        v[..3].copy_from_slice(&sector.to_be_bytes()[1..]);
        assert_eq!(sniff(&v, 400 * 4096), Some("mca"));
    }

    #[test]
    fn a_region_table_with_a_pointer_past_the_end_is_not_one() {
        let mut v = region(182);
        let at = 5 * 4;
        v[at..at + 3].copy_from_slice(&9_000u32.to_be_bytes()[1..]);
        assert_eq!(sniff(&v, (2 + 182) * 4096), None);
        // As are stamps that are seconds from no year a world was saved in.
        let mut v = region(182);
        let at = 4096 + 5 * 4;
        v[at..at + 4].copy_from_slice(&31_536_000u32.to_be_bytes());
        assert_eq!(sniff(&v, (2 + 182) * 4096), None);
    }

    #[test]
    fn tables_that_say_too_little_say_nothing() {
        let len = (2 + 182) * 4096;
        // Nothing but zeroes is not a region, whatever its length.
        assert_eq!(sniff(&region(0), 4 * 4096), None);
        // Less than both tables arriving is no evidence at all.
        assert_eq!(sniff(&region(182)[..8191], len), None);
        // A length that is not a count of sectors is not a region either.
        assert_eq!(sniff(&region(182), len + 1), None);
    }

    /// One stored ZIP local file record, from its signature to the end of its
    /// data. Enough for the sniff, which never reaches the central directory.
    fn zip_entry(name: &[u8], data: &[u8], streamed: bool) -> Vec<u8> {
        let mut v = b"PK\x03\x04".to_vec();
        v.extend_from_slice(&20u16.to_le_bytes()); // version needed
        v.extend_from_slice(&(if streamed { 8u16 } else { 0u16 }).to_le_bytes());
        v.extend_from_slice(&[0; 10]); // method, time, date, crc
        v.extend_from_slice(&(if streamed { 0 } else { data.len() as u32 }).to_le_bytes());
        v.extend_from_slice(&(data.len() as u32).to_le_bytes());
        v.extend_from_slice(&(name.len() as u16).to_le_bytes());
        v.extend_from_slice(&0u16.to_le_bytes()); // extra
        v.extend_from_slice(name);
        v.extend_from_slice(data);
        v
    }

    /// The same entry as a writer that uses ZIP64 for everything writes it:
    /// placeholders in the header and the sizes in an extra field.
    fn zip64_entry(name: &[u8], data: &[u8]) -> Vec<u8> {
        let mut v = b"PK\x03\x04".to_vec();
        v.extend_from_slice(&45u16.to_le_bytes()); // version needed
        v.extend_from_slice(&0u16.to_le_bytes()); // flags
        v.extend_from_slice(&[0; 10]); // method, time, date, crc
        v.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
        v.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
        v.extend_from_slice(&(name.len() as u16).to_le_bytes());
        v.extend_from_slice(&24u16.to_le_bytes()); // extra
        v.extend_from_slice(name);
        v.extend_from_slice(&0x5455u16.to_le_bytes()); // a timestamp first
        v.extend_from_slice(&0u16.to_le_bytes());
        v.extend_from_slice(&1u16.to_le_bytes()); // then the sizes
        v.extend_from_slice(&16u16.to_le_bytes());
        v.extend_from_slice(&(data.len() as u64).to_le_bytes());
        v.extend_from_slice(&(data.len() as u64).to_le_bytes());
        v.extend_from_slice(data);
        v
    }

    #[test]
    fn a_zarr_store_in_a_zip_is_told_from_an_ordinary_archive() {
        // v2: the root group's marker, ahead of a chunk that says nothing.
        let mut v2 = zip_entry(b"image.zarr/0/0.0.0", b"chunkbytes", false);
        v2.extend_from_slice(&zip_entry(b"image.zarr/.zgroup", br#"{"zarr_format":2}"#, false));
        assert_eq!(sniffed(&v2), Some("zarrzip"));

        // v3, where one file name carries both the format and the metadata.
        let v3 = zip_entry(b"zarr.json", br#"{"zarr_format":3,"node_type":"group"}"#, false);
        assert_eq!(sniffed(&v3), Some("zarrzip"));

        // An archive of ordinary files is still an archive.
        let plain = zip_entry(b"notes.txt", b"hello", false);
        assert_eq!(sniffed(&plain), Some("zip"));

        // A streamed entry gives no way on to the next record, so the walk
        // stops rather than reading the data as a header.
        let streamed = zip_entry(b"a.txt", b"data", true);
        assert_eq!(sniffed(&streamed), Some("zip"));

        // An archive written with ZIP64 sizes throughout: the walk reaches the
        // second entry only by reading the extra field the first one wrote.
        let mut wide = zip64_entry(b"image.zarr/0/0.0.0", b"chunkbytes");
        wide.extend_from_slice(&zip64_entry(b"image.zarr/.zgroup", br#"{"zarr_format":2}"#));
        assert_eq!(sniffed(&wide), Some("zarrzip"));
    }

    /// The files a Linux system leaves lying about, none of which look like
    /// anything else.
    #[test]
    fn linux_system_files_are_recognised() {
        assert_eq!(sniffed(b"\xd0\x0d\xfe\xed\0\0\x01\x00"), Some("dtb"));
        assert_eq!(sniffed(b"# GRUB Environment Block\n####"), Some("grubenv"));

        // A login record file says nothing about itself: it is recognised by
        // being a whole number of records, the first of which could be one.
        let mut login = vec![0u8; 384];
        login[0] = 7; // a user process
        login[8..13].copy_from_slice(b"pts/0");
        login[44..49].copy_from_slice(b"pengo");
        login[340..344].copy_from_slice(&1_700_000_000i32.to_le_bytes());
        assert_eq!(sniffed(&login), Some("utmp"));
        assert_eq!(sniffed(b"0707010000000A"), Some("cpio"));
        assert_eq!(sniffed(b"LPKSHHRH\0\0\0\0"), Some("journal"));
        assert_eq!(sniffed(&login[..383]), None);
    }

    /// A stream of CCSDS space packets, which says nothing about itself: what
    /// makes it one is that each packet's length lands on the next header.
    #[test]
    fn a_space_packet_stream_is_recognised_by_its_lengths_chaining() {
        let mut v = Vec::new();
        for i in 0..10u16 {
            v.extend_from_slice(&0x0817u16.to_be_bytes()); // telemetry, APID 0x17
            v.extend_from_slice(&(0xc000 | i).to_be_bytes());
            v.extend_from_slice(&9u16.to_be_bytes());
            v.extend_from_slice(b"ten bytes!");
        }
        assert_eq!(sniffed(&v), Some("spp"));
        // One byte in the wrong place and no length lands anywhere.
        let mut broken = v.clone();
        broken[5] = 0xff;
        assert_eq!(sniffed(&broken), None);
    }

    #[test]
    fn the_other_formats_still_answer() {
        assert_eq!(sniffed(b"\x89PNG\r\n\x1a\n"), Some("png"));
        // HDF5 containers remain openable through the ordinary file picker;
        // `.h5ad` is one of their application-level conventions, not tied to
        // the separate OME-Zarr directory action on the welcome screen.
        assert_eq!(sniffed(b"\x89HDF\r\n\x1a\n"), Some("hdf5"));
        assert_eq!(sniffed(b"\0asm\x01\0\0\0"), Some("wasm"));
        assert_eq!(sniffed(b"SQLite format 3\0"), Some("sqlite"));
        assert_eq!(sniffed(b"qoif\0\0\x01\0\0\0\x01\0\x04\0"), Some("qoi"));
        assert_eq!(sniffed(b"GIF89a"), Some("gif"));
        // Both ways round, and the 42 after the letters is written the way
        // the letters just said it would be.
        assert_eq!(sniffed(b"II*\x00\x08\x00\x00\x00"), Some("tiff"));
        assert_eq!(sniffed(b"MM\x00*\x00\x00\x00\x08"), Some("tiff"));
    }

    #[test]
    fn design_graphics_and_unity_formats_are_recognised() {
        assert_eq!(sniffed(b"8BPS\0\x01\0\0\0\0\0\0"), Some("psd"));
        assert_eq!(sniffed(b"%!PS-Adobe-3.0 EPSF-3.0\n"), Some("eps"));
        assert_eq!(sniffed(b"RIFF\x10\0\0\0CDR9vers\0\0\0\0"), Some("cdr"));
        assert_eq!(sniffed(b"RIFF\x10\0\0\0CMX1cont\0\0\0\0"), Some("cmx"));
        assert_eq!(sniffed(b"UnityFS\0\0\0\0\x08version\0revision\0"), Some("unitybundle"));

        let mut ico = vec![0, 0, 1, 0, 1, 0];
        ico.extend_from_slice(&[16, 16, 0, 0, 1, 0, 32, 0]);
        ico.extend_from_slice(&4u32.to_le_bytes());
        ico.extend_from_slice(&22u32.to_le_bytes());
        ico.extend_from_slice(&40u32.to_le_bytes());
        assert_eq!(sniff(&ico, ico.len() as u64), Some("ico"));

        let mut assets = vec![0; 84];
        assets[8..12].copy_from_slice(&22u32.to_be_bytes());
        assets[16] = 0;
        assets[20..24].copy_from_slice(&24u32.to_be_bytes());
        assets[24..32].copy_from_slice(&84u64.to_be_bytes());
        assets[32..40].copy_from_slice(&80u64.to_be_bytes());
        assert_eq!(sniff(&assets, assets.len() as u64), Some("unityassets"));

        let mut thumbs = vec![0; 600];
        thumbs[..8].copy_from_slice(b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1");
        let catalog = b"C\0a\0t\0a\0l\0o\0g\0";
        thumbs[512..512 + catalog.len()].copy_from_slice(catalog);
        assert_eq!(sniff(&thumbs, thumbs.len() as u64), Some("thumbsdb"));
        assert_eq!(sniff(&thumbs[..512], 512), None, "an arbitrary compound file is not necessarily Thumbs.db");
    }

    /// The formats that hide what identifies them deep in the file are the
    /// reason `SNIFF_WINDOW` is not a few hundred bytes. A caller reads that
    /// much and no more, so each of them has to be decided inside it.
    #[test]
    fn the_deep_formats_are_decided_inside_the_sniff_window() {
        let mut iso = vec![0; SNIFF_WINDOW];
        iso[16 * 2048] = 1;
        iso[16 * 2048 + 1..16 * 2048 + 6].copy_from_slice(b"CD001");
        iso[16 * 2048 + 6] = 1;
        assert_eq!(sniff(&iso[..SNIFF_WINDOW], 40 * 2048), Some("iso9660"));

        let mut region = vec![0u8; SNIFF_WINDOW];
        region[..4].copy_from_slice(&[0, 0, 2, 1]);
        region[4096..4100].copy_from_slice(&1_700_000_000u32.to_be_bytes());
        assert_eq!(sniff(&region[..SNIFF_WINDOW], 3 * 4096), Some("mca"));

        let mut module = vec![0; SNIFF_WINDOW];
        module[1080..1084].copy_from_slice(b"M.K.");
        assert_eq!(sniff(&module[..SNIFF_WINDOW], 1 << 16), Some("mod"));
    }

    #[test]
    fn wave_container_variants_are_recognised() {
        assert_eq!(sniffed(b"RIFF\x10\0\0\0WAVEfmt \0\0\0\0"), Some("wav"));
        assert_eq!(sniffed(b"RF64\xff\xff\xff\xffWAVEds64\0\0\0\0"), Some("wav"));
        assert_eq!(sniffed(b"RIFX\0\0\0\x10WAVEfmt \0\0\0\0"), Some("wav"));
    }

    #[test]
    fn media_containers_and_retro_objects_are_recognised() {
        let mkv = b"\x1a\x45\xdf\xa3\x8b\x42\x82\x88matroska";
        assert_eq!(sniff(mkv, mkv.len() as u64), Some("mkv"));
        // EBML by itself, and WebM in particular, is not Matroska.
        assert_eq!(sniff(b"\x1a\x45\xdf\xa3\x84webm", 9), None);

        let mut iso = vec![0; 16 * 2048 + 7];
        iso[16 * 2048] = 1;
        iso[16 * 2048 + 1..16 * 2048 + 6].copy_from_slice(b"CD001");
        iso[16 * 2048 + 6] = 1;
        assert_eq!(sniff(&iso, 40 * 2048), Some("iso9660"));

        let mut dv = vec![0xff; 8 * 80];
        for (i, (section, number)) in [(0, 0), (1, 0), (1, 1), (2, 0), (2, 1), (2, 2), (3, 0), (4, 0)].into_iter().enumerate() {
            let at = i * 80;
            dv[at] = section << 5 | 0x1f;
            dv[at + 1] = 0x07;
            dv[at + 2] = number;
        }
        assert_eq!(sniff(&dv, 120_000), Some("dv"));

        let mut coff = vec![0; 60];
        coff[0..2].copy_from_slice(&0x014cu16.to_le_bytes());
        coff[2..4].copy_from_slice(&1u16.to_le_bytes());
        assert_eq!(sniff(&coff, coff.len() as u64), Some("coff"));

        let mut omf = vec![0x80, 5, 0, 3, b'F', b'O', b'O'];
        let checksum = 0u8.wrapping_sub(omf.iter().fold(0u8, |sum, &b| sum.wrapping_add(b)));
        omf.push(checksum);
        assert_eq!(sniffed(&omf), Some("omf"));
    }

    #[test]
    fn assimp_binary_families_are_recognised_without_extensions() {
        for (bytes, want) in [
            (b"Kaydara FBX Binary  \0\x1a\0\xe8\x1c\0\0".as_slice(), "fbx"),
            (b"INTERQUAKEMODEL\0\x02\0\0\0".as_slice(), "iqm"),
            (b"BB3D\x10\0\0\0\x01\0\0\0".as_slice(), "b3d"),
            (b"glTF\x02\0\0\0\x0c\0\0\0".as_slice(), "glb"),
            (b"PXR-USDC\0\x09\0\0\0\0\0\0".as_slice(), "usdc"),
            (b"IDP2\x0f\0\0\0".as_slice(), "md2"),
            (b"IDP3\x0f\0\0\0".as_slice(), "md3"),
            (b"IDPC\x02\0\0\0".as_slice(), "mdc"),
            (b"HMP7\0\0\0\0".as_slice(), "hmp"),
            (b"3DMO\x20\0\0\0".as_slice(), "m3d"),
            (b"MS3D000000\x04\0\0\0".as_slice(), "ms3d"),
            (b"nendo 1.2\0\0\0".as_slice(), "ndo"),
            (b"TERRAGENTERRAIN SIZE".as_slice(), "ter"),
        ] {
            assert_eq!(sniffed(bytes), Some(want), "{want}");
        }

        let mut lwo = b"FORM\0\0\0\x0cLWO2".to_vec();
        lwo.extend_from_slice(b"TAGS\0\0\0\0");
        assert_eq!(sniffed(&lwo), Some("lwo"));

        let mut three_ds = b"MM".to_vec();
        three_ds.extend_from_slice(&10u32.to_le_bytes());
        three_ds.extend_from_slice(&[0; 4]);
        assert_eq!(sniffed(&three_ds), Some("3ds"));
        // The two-byte token alone is far too common to claim.
        assert_eq!(sniff(b"MM\x06\0\0\0", 100), None);
    }

    #[test]
    fn assimp_text_and_json_families_use_grammar_markers() {
        assert_eq!(sniffed(b"AC3Db\nMATERIAL \"white\""), Some("ac3d"));
        assert_eq!(sniffed(b"HIERARCHY\nROOT hips\n"), Some("bvh"));
        assert_eq!(sniffed(b"OFF\n8 6 0\n"), Some("off"));
        assert_eq!(sniffed(b"MD5Version 10\ncommandline \"\"\n"), Some("md5"));
        assert_eq!(sniffed(br#"{"asset":{"version":"2.0"},"meshes":[]}"#), Some("gltf"));
        assert_eq!(sniffed(br#"<?xml version="1.0"?><COLLADA/>"#), Some("collada"));
        assert_eq!(sniffed(br#"<?xml version="1.0"?><X3D/>"#), Some("x3d"));
        // An ordinary JSON object remains JSON; the two glTF keys together
        // are the evidence, not merely its representation.
        assert_eq!(sniffed(br#"{"version":"2.0","name":"not a model"}"#), Some("json"));
    }

    #[test]
    fn binary_stl_uses_its_exact_facet_count_and_file_length() {
        let mut stl = vec![0; 84 + 2 * 50];
        stl[80..84].copy_from_slice(&2u32.to_le_bytes());
        assert_eq!(sniffed(&stl), Some("stl"));
        assert_eq!(sniff(&stl, stl.len() as u64 + 1), None);
    }

    fn little_tiff(entries: &[(u16, u16, u32, [u8; 4])], tail: &[u8]) -> Vec<u8> {
        let mut v = b"II*\0\x08\0\0\0".to_vec();
        v.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        for (tag, kind, count, value) in entries {
            v.extend_from_slice(&tag.to_le_bytes());
            v.extend_from_slice(&kind.to_le_bytes());
            v.extend_from_slice(&count.to_le_bytes());
            v.extend_from_slice(value);
        }
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(tail);
        v
    }

    #[test]
    fn dng_is_distinguished_from_an_ordinary_tiff_by_its_version_tag() {
        let dng = little_tiff(&[(50706, 1, 4, [1, 6, 0, 0])], &[]);
        assert_eq!(sniffed(&dng), Some("dng"));

        // A camera make by itself does not prove that an otherwise ordinary
        // TIFF is the maker's proprietary RAW format.
        let make_at = 8 + 2 + 12 + 4;
        let tiff = little_tiff(&[(271, 2, 6, (make_at as u32).to_le_bytes())], b"NIKON\0");
        assert_eq!(sniffed(&tiff), Some("tiff"));
    }

    #[test]
    fn tiff_raw_variants_need_both_sensor_metadata_and_the_camera_make() {
        for (make, want) in [(b"NIKON\0".as_slice(), "nef"), (b"SONY\0", "arw"), (b"PENTAX\0", "pef"), (b"SAMSUNG\0", "srw")] {
            let make_at = 8 + 2 + 2 * 12 + 4;
            let raw = little_tiff(
                &[
                    (271, 2, make.len() as u32, (make_at as u32).to_le_bytes()),
                    (33421, 3, 2, [2, 0, 2, 0]),
                ],
                make,
            );
            assert_eq!(sniffed(&raw), Some(want), "{}", String::from_utf8_lossy(make));
        }
    }

    #[test]
    fn raw_formats_with_header_markers_are_recognised_directly() {
        assert_eq!(sniffed(b"II*\0\x10\0\0\0CR\x02\0"), Some("cr2"));
        assert_eq!(sniffed(b"IIRO\x08\0\0\0"), Some("orf"));
        assert_eq!(sniffed(b"MMOR\0\0\0\x08"), Some("orf"));
        assert_eq!(sniffed(b"IIU\0\x08\0\0\0"), Some("rw2"));
    }

    #[test]
    fn a_test_that_weighs_several_things_beats_a_two_byte_prefix() {
        // An LHA archive whose header size and checksum happen to be the two
        // characters a JSON file opens with. The method at offset 2 is what
        // settles it, and the table of prefixes never gets asked.
        assert_eq!(sniffed(b"{\"-lh5-\0\0\0\0"), Some("lha"));
        // And an ordinary JSON file is still JSON.
        assert_eq!(sniffed(b"{\"name\": 1}"), Some("json"));
    }

    #[test]
    fn one_magic_number_covering_several_formats_is_settled_by_what_follows() {
        assert_eq!(sniffed(b"FORM\0\0\0\x10AIFF"), Some("aiff"));
        assert_eq!(sniffed(b"FORM\0\0\0\x10ILBM"), Some("ilbm"));
        // A JPEG opens with the start of image and then the first marker,
        // which is three bytes; the two on their own are not enough.
        assert_eq!(sniffed(b"\xff\xd8\xff\xe0\x00\x10JFIF\x00"), Some("jpeg"));
        assert_eq!(sniffed(b"\xff\xd8\xff\xdb\x00\x43\x00"), Some("jpeg"));
        assert_eq!(sniffed(b"\xff\xd8hello"), None);
        // An IFF file holding something with no template here is left alone
        // rather than read as one of the two that do.
        assert_eq!(sniffed(b"FORM\0\0\0\x108SVX"), None);
    }

    /// Three unrelated formats have made themselves at home in `.db`, and the
    /// extension says nothing about which. Each announces itself in a place of
    /// its own: SQLite at the front, GDBM at the front, Berkeley DB twelve
    /// bytes in, which is why one file cannot look like two of them.
    #[test]
    fn the_three_kinds_of_db_file_are_told_apart_by_their_bytes() {
        let mut sqlite = b"SQLite format 3\0".to_vec();
        sqlite.resize(32, 0);
        assert_eq!(sniffed(&sqlite), Some("sqlite"));

        let mut gdbm = 0x13579acdu32.to_le_bytes().to_vec();
        gdbm.resize(64, 0);
        assert_eq!(sniffed(&gdbm), Some("gdbm"));
        // The same file written on a machine of the other byte order.
        let mut swapped = 0xcd9a5713u32.to_le_bytes().to_vec();
        swapped.resize(64, 0);
        assert_eq!(sniffed(&swapped), Some("gdbm"));

        // Berkeley DB keeps its magic behind a log sequence number and a page
        // number, so the first twelve bytes say nothing. The page type at 25
        // has to agree with the magic, or four bytes in the middle of some
        // other header would be enough to claim the file.
        let mut bdb = vec![0u8; 12];
        bdb.extend_from_slice(&0x00053162u32.to_le_bytes());
        bdb.resize(64, 0);
        bdb[25] = 9;
        assert_eq!(sniffed(&bdb), Some("bdb"));
        let mut big = vec![0u8; 12];
        big.extend_from_slice(&0x00061561u32.to_be_bytes());
        big.resize(64, 0);
        big[25] = 8;
        assert_eq!(sniffed(&big), Some("bdb"));
    }
}
