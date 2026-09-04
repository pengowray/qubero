//! Built-in templates. These double as the test-bed for the IR: anything a
//! format needs that the IR cannot say is a gap in the IR, not in the format.

use crate::template::{Encoding, Expr as E, StrLen, Ty as T};

/// What a compressed stream holds when the stream is the whole file: its text.
///
/// A file called `hello.txt.zst` holds `hello`, and that is the one thing a
/// reader opening it wants to see. Bytes that are not text read as bytes,
/// which is what an encoding that does not fit already reports, so this is
/// safe to say of a stream holding anything.
///
/// One field in a structure rather than the field on its own, because the
/// stream's own name is the compressed run's: the structure is marked as
/// nothing but its contents, so a view that folds those spends no row on it.
pub(crate) fn decoded_text() -> T {
    T::structure_named(
        "DecodedText",
        "",
        "text",
        vec![("text", T::text(StrLen::Fixed(E::Remaining), Encoding::Utf8))],
    )
}

/// What a compressed stream holds when something else in the file says what it
/// is: bytes, until a template is written for whatever they turn out to be.
/// A ROOT record's contents are a streamed object, and reading one takes the
/// streamer information the file keeps elsewhere.
pub(crate) fn decoded_object() -> T {
    T::structure_named("DecodedObject", "", "object", vec![("object", T::bytes(E::Remaining))])
}

mod aiff;
mod ar;
mod aseprite;
mod assimp;
mod appledouble;
mod au;
mod bmp;
mod bzip2;
mod bdb;
mod bencode;
mod c16;
mod cab;
mod braw;
mod bards_tale;
mod cbor;
mod claudetheme;
mod cdf;
mod coff;
mod compress;
mod corel;
mod cpio;
mod dos;
mod dtb;
mod draco;
mod dv;
mod elf;
mod machine;
mod macho;
mod ne;
pub mod ne_disasm;
pub mod bpf_opcodes;
pub mod elf_disasm;
mod eps;
mod fits;
mod ggml;
pub mod ggml_quant;
mod gguf;
mod git;
mod gif;
mod grib;
mod grubenv;
mod gdbm;
mod gwf;
mod gzip;
mod recognise;
pub mod sqlite_overflow;
mod uf2;
mod hackrffw;
mod hdf4;
mod hdf5;
pub mod h5ad;
pub mod hdf5_chunk;
mod id3;
mod iff;
mod ilbm;
mod iso9660;
mod journal;
mod jxr;
mod jpeg;
mod ico;
mod lha;
mod lnk;
mod lz4;
mod lzip;
mod mca;
mod midi;
mod mkv;
mod mp4;
mod mseed;
mod nes;
mod netcdf;
mod npy;
mod old_mac;
mod omf;
mod pak;
mod parquet;
mod pcx;
mod pico8;
mod pdf;
pub mod pdf_objstm;
pub mod pdf_xref;
mod pe;
pub mod pe_tables;
mod pi1;
mod picotron;
mod psd;
mod pnm;
mod qoi;
mod rar5;
mod rpm;
mod sac;
mod png;
mod root;
mod safetensors;
mod le;
mod spp;
pub(crate) mod sqlite;
mod sevenzip;
mod swf;
mod tap;
mod tar;
mod tga;
mod tiff;
mod thumbsdb;
mod tracker;
mod unity;
mod utmp;
mod vpk;
mod w4v;
mod wad;
mod xar;
mod xz;
mod wav;
mod whisper;
mod zip;
mod zlib;
mod zstd;
mod wasm;
pub mod wasm_disasm;
mod wasm_opcodes;

pub use aiff::aiff;
pub use ar::{ar, deb};
pub use appledouble::{appledouble, applesingle};
pub use aseprite::aseprite;
pub use au::au;
pub use bmp::bmp;
pub use bzip2::bzip2;
pub use cab::cab;
pub use braw::braw;
pub use bards_tale::bards_tale;
pub use bencode::bencode;
pub use cbor::cbor;
pub use claudetheme::claudetheme;
pub use cdf::cdf;
pub use coff::coff;
pub use compress::compress;
pub use corel::{cdr, cmx};
pub use cpio::cpio;
pub use dos::{com, dos};
pub use dtb::dtb;
pub use draco::draco;
pub use dv::dv;
pub use eps::eps;
pub use fits::fits;
pub use elf::{bpf, elf};
pub use ne::ne;
pub use macho::macho;
pub use ne_disasm::Program as NeProgram;
pub use elf_disasm::Program as ElfProgram;
pub use gguf::gguf;
pub use git::{git_index, git_pack_index};
pub use gif::gif;
pub use grib::grib;
pub use grubenv::grubenv;
pub use gdbm::gdbm;
pub use gwf::gwf;
pub use gzip::gzip;
pub use uf2::{image as uf2_image, uf2, Image as Uf2Image};
pub use hackrffw::hackrffw;
pub use hdf4::hdf4;
pub use hdf5::hdf5;
pub use id3::id3;
pub use ilbm::ilbm;
pub use iso9660::iso9660;
pub use jpeg::jpeg;
pub use journal::journal;
pub use jxr::jxr;
pub use ico::ico;
pub use lha::lha;
pub use lnk::lnk;
pub use lz4::lz4;
pub use lzip::lzip;
pub use mca::mca;
pub use midi::midi;
pub use mkv::mkv;
pub use mp4::mp4;
pub use mseed::mseed;
pub use nes::nes;
pub use netcdf::netcdf;
pub use npy::npy;
pub use old_mac::{binhex, compactpro, macbinary, stuffit};
pub use omf::omf;
pub use pe::pe;
pub use pak::pak;
pub use parquet::parquet;
pub use pcx::pcx;
pub use pdf::pdf;
pub use pi1::pi1;
pub use picotron::{p64png, p64rom};
pub use psd::psd;
pub use bdb::bdb;
pub use c16::c16;
pub use pnm::pnm;
pub use qoi::qoi;
pub use rar5::rar5;
pub use rpm::rpm;
pub use sac::sac;
pub use pico8::p8png;
pub use png::png;
pub use root::root;
pub use safetensors::safetensors;
pub use le::le;
pub use spp::spp;
pub use sqlite::{self_db, sqlite};
pub use sqlite_overflow::{payload as sqlite_payload, Payload as SqlitePayload};
pub use sevenzip::sevenzip;
pub use swf::swf;
pub use tap::tap;
pub use tar::tar;
pub use tga::tga;
pub use tiff::{camera_raw, tiff};
pub use thumbsdb::thumbsdb;
pub use tracker::{it, mod_file, s3m, xm};
pub use unity::{unity_assets, unity_bundle};
pub use utmp::utmp;
pub use vpk::vpk;
pub use w4v::w4v;
pub use wad::wad;
pub use xar::xar;
pub use xz::xz;
pub use wav::wav;
pub use whisper::whisper;
pub use zip::{zarrzip, zip};
pub use zlib::zlib;
pub use zstd::zstd;
pub use wasm::wasm;
pub use wasm_disasm::Module as WasmModule;

pub use recognise::{sniff, SNIFF_WINDOW};

use crate::template::{Template, Ty};

/// A file that is JSON and nothing else. The values inside it are the
/// structure, and every one of them says where in the file it is written.
pub fn json() -> Template {
    Template::new("json", Ty::json())
}

/// OME-Zarr metadata is JSON. Keeping it as its own template makes the
/// store's root metadata identifiable without inventing a binary layout for
/// the separately stored Zarr chunks.
pub fn omezarr() -> Template {
    Template::new("omezarr", Ty::json())
}


/// Every built-in template, by the name a file is opened as.
///
/// This is the one list. It used to be three — the names a caller could pick
/// from, the match that turned a name into a template, and the signatures
/// `sniff` reads — and keeping three lists in step by hand is a way of
/// forgetting the third. Adding a format is now one line here, plus a
/// signature or a test in [`MAGIC`] or [`PROBES`] if the file announces
/// itself.
///
/// Each entry is handed the name it was found under, because a few templates
/// serve several: every camera maker's raw format is a TIFF whose fields that
/// maker chose, and the template says which maker on the strength of the name.
const BUILTIN: &[(&str, fn(&str) -> Template)] = &[
    ("png", |_| png()),
    // A PICO-8 cartridge: a PNG with the program in the low bits of the picture.
    ("p8png", |_| p8png()),
    ("aseprite", |_| aseprite()),
    ("braw", |_| braw()),
    ("swf", |_| swf()),
    ("zip", |_| zip()),
    ("wasm", |_| wasm()),
    ("mp4", |_| mp4()),
    ("mseed", |_| mseed()),
    ("sac", |_| sac()),
    ("mkv", |_| mkv()),
    ("dtb", |_| dtb()),
    ("draco", |_| draco()),
    ("dv", |_| dv()),
    ("iso9660", |_| iso9660()),
    ("id3", |_| id3()),
    ("wav", |_| wav()),
    ("w4v", |_| w4v()),
    ("midi", |_| midi()),
    ("mod", |_| mod_file()),
    ("s3m", |_| s3m()),
    ("xm", |_| xm()),
    ("it", |_| it()),
    ("spp", |_| spp()),
    ("sqlite", |_| sqlite()),
    ("self", |_| self_db()),
    ("pe", |_| pe()),
    ("coff", |_| coff()),
    ("omf", |_| omf()),
    ("msdos", |_| dos()),
    ("com", |_| com()),
    ("ne", |_| ne()),
    ("macho", |_| macho()),
    ("gguf", |_| gguf()),
    ("root", |_| root()),
    ("whisper", |_| whisper()),
    ("safetensors", |_| safetensors()),
    ("claudetheme", |_| claudetheme()),
    ("json", |_| json()),
    ("omezarr", |_| omezarr()),
    ("zarrzip", |_| zarrzip()),
    ("bmp", |_| bmp()),
    ("pcx", |_| pcx()),
    ("tga", |_| tga()),
    ("au", |_| au()),
    ("pi1", |_| pi1()),
    ("nes", |_| nes()),
    ("p64rom", |_| p64rom()),
    ("p64png", |_| p64png()),
    ("netcdf", |_| netcdf()),
    ("grib", |_| grib()),
    ("npy", |_| npy()),
    ("fits", |_| fits()),
    ("grubenv", |_| grubenv()),
    ("gzip", |_| gzip()),
    ("zlib", |_| zlib()),
    ("bzip2", |_| bzip2()),
    ("lzip", |_| lzip()),
    ("compress", |_| compress()),
    ("xz", |_| xz()),
    ("zstd", |_| zstd()),
    ("lz4", |_| lz4()),
    ("tar", |_| tar()),
    ("7z", |_| sevenzip()),
    ("rar5", |_| rar5()),
    ("gwf", |_| gwf()),
    ("uf2", |_| uf2()),
    ("hackrffw", |_| hackrffw()),
    ("gif", |_| gif()),
    ("aiff", |_| aiff()),
    ("ilbm", |_| ilbm()),
    ("pnm", |_| pnm()),
    ("c16", |_| c16()),
    ("bdb", |_| bdb()),
    ("gdbm", |_| gdbm()),
    ("wad", |_| wad()),
    ("pak", |_| pak()),
    ("parquet", |_| parquet()),
    ("hdf4", |_| hdf4()),
    ("cdf", |_| cdf()),
    ("vpk", |_| vpk()),
    ("mca", |_| mca()),
    ("tap", |_| tap()),
    ("lha", |_| lha()),
    ("lnk", |_| lnk()),
    ("cbor", |_| cbor()),
    ("bencode", |_| bencode()),
    ("cpio", |_| cpio()),
    ("ar", |_| ar()),
    ("rpm", |_| rpm()),
    ("cab", |_| cab()),
    ("xar", |_| xar()),
    ("deb", |_| deb()),
    ("gitindex", |_| git_index()),
    ("gitpackidx", |_| git_pack_index()),
    ("qoi", |_| qoi()),
    ("tiff", |_| tiff()),
    ("jxr", |_| jxr()),
    ("dng", camera_raw),
    ("nef", camera_raw),
    ("cr2", camera_raw),
    ("arw", camera_raw),
    ("orf", camera_raw),
    ("rw2", camera_raw),
    ("pef", camera_raw),
    ("srw", camera_raw),
    ("jpeg", |_| jpeg()),
    ("journal", |_| journal()),
    ("pdf", |_| pdf()),
    ("hdf5", |_| hdf5()),
    ("appledouble", |_| appledouble()),
    ("applesingle", |_| applesingle()),
    ("macbinary", |_| macbinary()),
    ("binhex", |_| binhex()),
    ("stuffit", |_| stuffit()),
    ("compactpro", |_| compactpro()),
    ("bardstale", |_| bards_tale()),
    ("cdr", |_| cdr()),
    ("cmx", |_| cmx()),
    ("psd", |_| psd()),
    ("eps", |_| eps()),
    ("utmp", |_| utmp()),
    ("unityassets", |_| unity_assets()),
    ("unitybundle", |_| unity_bundle()),
    ("thumbsdb", |_| thumbsdb()),
    ("ico", |_| ico()),
    ("elf", |_| elf()),
    ("le", |_| le()),
    ("bpf", |_| bpf()),
];

/// The names a caller may open a file as. The model importer's formats are
/// listed where that module lists them rather than copied to here, because a
/// copy of a hundred extensions is a copy that goes stale.
pub fn builtin_names() -> Vec<&'static str> {
    BUILTIN.iter().map(|(name, _)| *name).chain(assimp::NAMES.iter().copied()).collect()
}

/// The template a name opens, or nothing if this build has no such format.
pub fn builtin(name: &str) -> Option<Template> {
    if let Some((_, make)) = BUILTIN.iter().find(|(known, _)| *known == name) {
        return Some(make(name));
    }
    assimp::template(name)
}
