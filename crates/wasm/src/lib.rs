//! wasm-bindgen surface over `qubero-core`.
//!
//! Offsets cross the boundary as `f64` (exact up to 2^53, far past any file size)
//! to avoid BigInt friction on the JS side.

use qubero_core::codec::{inflate, Codec, Step as MapStep, StepKind};
use qubero_core::eval::{leap_seconds, CellAt, Census, CensusState, CensusWalk, Diagram, Explain, Graph, KindWalk, Moment, Origin, ReportWalk, SpaceId, Tab, TimeNote, NO_PARENT};
use qubero_core::template::Zone;
use qubero_core::hexdump;
use qubero_core::hexpat::Includes as _;
use qubero_core::textview;
use qubero_core::source::Source;
use qubero_core::{diescript, dosbasic};
use qubero_core::search::{self, Needle, Search, Step};
use qubero_core::{
    formats, magicrule, overview, ChunkStore, Document, EvalError, Evaluator, ExtentEstimate, NodeInfo,
    RunKind, Span, SpanPart, Value,
};
use serde::Serialize;
use wasm_bindgen::prelude::*;

mod dto;
use dto::*;

/// Everything one reading holds: a document, what reads it, and the working
/// each format keeps beside it. There is one per address space. Space 0 is the
/// file; every other is a `Decoded` stream that was opened as a document of its
/// own, and it has its own byte-class scan, so two tabs never read over each
/// other's working.
///
/// A stream whose template came from looking at its bytes has its own
/// evaluator too. One read by the template it declared has none: its fields
/// name what is outside the stream, so they are read in the file's reading,
/// under the stream, and handed back as the tab's own. See `Editor::tab`.
struct Sheet {
    doc: Document<ChunkStore>,
    /// Which sheet's reading opened this space: 0, the file, or a stream whose
    /// bytes were recognised and so has a reading of its own. Never a stream
    /// read where it was declared, which has none: a stream opened from a tab
    /// of one of those is opened in the reading that tab is read in. 0 for the
    /// file itself.
    home: usize,
    /// Which `Decoded` or joining node of the home sheet's reading this space
    /// was unpacked from. Empty for space 0, which was unpacked from nothing.
    origin: Vec<usize>,
    /// The core's number for the same space in the home sheet's reading, which
    /// is what the trace behind the cursor link is asked by. Zero for the
    /// file. The core renumbers from 1 whenever a reading is thrown away, so
    /// this is set again every time the stream is opened again; a tab's own
    /// number never changes.
    core_space: SpaceId,
    eval: Option<Evaluator>,
    /// For a stream read where it was declared, where its contents are in the
    /// home sheet's reading. `eval` is None for those.
    view: Option<Vec<usize>>,
    /// What such a stream declares it holds, which the diagram is drawn from.
    read_as: Option<qubero_core::template::Template>,
    /// The stream was unpacked from its home as it was before an edit or a
    /// change of template, and has not been opened again since. The bytes and
    /// reading here are the old ones, kept so a tab still waiting on chunks of
    /// the file has a length to show; `Editor::ensure_open` opens the stream
    /// again before anything of the tab is answered from them.
    stale: bool,
    /// What the wasm sections say about the module, when that is the template.
    /// Built on the first listing that needs it, and thrown away whenever the
    /// document changes, since it holds paths that the change may have moved.
    disasm: Option<formats::WasmModule>,
    /// The same for an eBPF object: what its sections, symbols and
    /// relocations say, so an instruction can name the map it loads and the
    /// helper it calls.
    bpf: Option<formats::ElfProgram>,
    /// Whether `bpf` includes symbols and relocations rather than only named
    /// sections for the logical overview.
    bpf_complete: bool,
    /// The same for a 16-bit Windows program, whose relocations say what its
    /// calls into other modules are calls to.
    ne: Option<formats::NeProgram>,
    /// The template in use, which decides whether a listing row goes through
    /// the disassembler.
    template: String,
    /// The byte-class scan behind the overview, run a step at a time. Thrown
    /// away on any edit: the classes describe bytes that may no longer be
    /// there.
    scan: Option<overview::Scan>,
    /// The same over the one block a reader has picked out, at whatever
    /// resolution that block's own size allows.
    focus: Option<overview::Scan>,
    /// The walk that totals the file's bits by field kind and type, run a step
    /// at a time. Thrown away on any edit for the same reason the scan is:
    /// every offset in it describes bytes that may have moved.
    kinds: Option<KindWalk>,
    /// The Diagram view's count of the file's fields against the format's
    /// boxes, run a step at a time and thrown away with the walk above: its
    /// paths are through bytes and a template that may have changed.
    census: Option<CensusWalk>,
    /// The report view's walk: the file's format profile, its byte ledger,
    /// its extent audit and its directories, run a step at a time and thrown
    /// away with the two walks above, for the same reason.
    report: Option<ReportWalk>,
    /// What the last `.ksy` conversion had to say, as the JSON the panel
    /// shows. Empty for a template that did not come from a `.ksy`.
    ksy_report: String,
    /// The same for the last `.hexpat`. The two are kept apart rather than
    /// sharing one slot with a tag, because each panel asks its own question
    /// and an empty answer is how it knows the template is not its own.
    hexpat_report: String,
}

impl Sheet {
    /// A reading of one of the core's spaces.
    ///
    /// The store reads the space's own source, so nothing is missing and
    /// nothing is copied: a space has no file behind it to fetch a missing
    /// chunk back from, and a stream too large to hold whole is decoded a
    /// piece at a time as it is read. The template is the one the core
    /// settled on, which is the stream's own or, where that said only bytes,
    /// whatever the unpacked bytes were recognised as. Only the second is read
    /// here; the first is read in the file's reading.
    fn from_space(space: &qubero_core::eval::Space, home: usize, origin: Vec<usize>) -> Sheet {
        let template = space.read_as().clone();
        let store = ChunkStore::behind(Box::new(space.source().clone()), SPACE_CHUNK);
        let mut sheet = Sheet::new(store, origin);
        sheet.home = home;
        sheet.core_space = space.id;
        sheet.template = template.name.clone();
        match space.view() {
            Some(view) => {
                sheet.view = Some(view.root.clone());
                sheet.read_as = Some(template);
            }
            None => {
                let mut ev = Evaluator::new(template);
                ev.set_slice(Some(WORK_SLICE));
                sheet.eval = Some(ev);
            }
        }
        sheet
    }

    /// A space that is no longer there: the stream it came from was edited away
    /// or would not open a second time. It holds no bytes rather than the
    /// file's, because a tab named after a stream must never quietly show
    /// something else.
    fn empty(home: usize, origin: Vec<usize>) -> Sheet {
        let mut sheet = Sheet::new(ChunkStore::new(0, SPACE_CHUNK, 1), origin);
        sheet.home = home;
        sheet
    }

    fn new(store: ChunkStore, origin: Vec<usize>) -> Sheet {
        Sheet {
            doc: Document::new(store),
            home: 0,
            origin,
            core_space: 0,
            eval: None,
            view: None,
            read_as: None,
            stale: false,
            disasm: None,
            bpf: None,
            bpf_complete: false,
            ne: None,
            template: String::new(),
            scan: None,
            focus: None,
            kinds: None,
            census: None,
            report: None,
            ksy_report: String::new(),
            hexpat_report: String::new(),
        }
    }
}

/// The spaces one file has open, and which of them the caller is reading.
///
/// Every method that reads bytes or fields names a space, because a space is a
/// document: the tab strip in the interface is one `Doc` per space over this
/// one editor. The file's own bytes, its edits and its save plan stay with
/// space 0, since an unpacked stream is read-only this round.
#[wasm_bindgen]
pub struct Editor {
    /// Space 0 first, then one per opened stream, in the order they opened.
    sheets: Vec<Sheet>,
    /// The space the call in hand names. Set by every method that takes one,
    /// so a helper called part-way through a reading still finds it.
    live: usize,
}

/// What a `.ksy` the reader pasted in resolves its `meta/imports` against:
/// the files they supplied alongside it first, then the bundled collection.
///
/// The bundled half is what makes `imports: [/common/vlq_base128_le]` work
/// without the reader hunting down a second file: `common/` is seven small
/// files that half the Kaitai library leans on, and they are already here.
/// Their own copy still wins, so a file of theirs by the same name is the one
/// used.
struct PastedImports(qubero_core::ksy::MapImports);

impl qubero_core::ksy::Imports for PastedImports {
    fn load(&self, name: &str) -> Option<String> {
        self.0.load(name).or_else(|| qubero_core::ksy::BundledImports.load(name))
    }
}

/// `meta/file-extension` read off a `.ksy` without parsing the rest of it:
/// the chooser lists every bundled format, and a hundred full parses to get
/// one word each would be paid on every file opened.
fn ksy_extensions(text: &str) -> Vec<String> {
    let clean = |e: &str| e.trim().trim_matches(['"', '\'']).to_lowercase();
    let mut out = Vec::new();
    let mut in_meta = false;
    let mut in_list = false;
    for line in text.lines() {
        let bare = line.split(" #").next().unwrap_or("").trim_end();
        if bare.is_empty() {
            continue;
        }
        if !bare.starts_with(' ') {
            in_meta = bare == "meta:";
            in_list = false;
            continue;
        }
        if !in_meta {
            continue;
        }
        let t = bare.trim();
        if in_list {
            if let Some(item) = t.strip_prefix("- ") {
                out.push(clean(item));
                continue;
            }
            in_list = false;
        }
        if let Some(rest) = t.strip_prefix("file-extension:") {
            let rest = rest.trim();
            if rest.is_empty() {
                in_list = true;
            } else {
                let rest = rest.trim_start_matches('[').trim_end_matches(']');
                out.extend(rest.split(',').map(clean).filter(|e| !e.is_empty()));
            }
        }
    }
    out
}

/// The `meta/imports` map both `.ksy` entries take, or what was wrong with it.
fn ksy_imports(imports_json: &str) -> Result<std::collections::HashMap<String, String>, String> {
    if imports_json.trim().is_empty() {
        return Ok(std::collections::HashMap::new());
    }
    serde_json::from_str(imports_json).map_err(|e| format!("the imports are not a JSON object of name to text: {e}"))
}

// ---- ImHex patterns ----

/// What a `.hexpat` the reader pasted in resolves its `#include`s and
/// `import`s against: the files they supplied alongside it, then the bundled
/// patterns, and then the declaration-only table the core keeps.
///
/// The upstream `includes/` tree is GPL-2.0 and is not here, so a pattern that
/// asks for a file nothing can answer for has to be told which file to hand
/// over. That is what `asked` records: every path this was asked for and could
/// not answer, which the panel turns into a box to paste that file into.
struct PastedIncludes {
    files: qubero_core::hexpat::MapIncludes,
    asked: std::cell::RefCell<Vec<String>>,
}

impl qubero_core::hexpat::Includes for PastedIncludes {
    fn load(&self, path: &str) -> Option<String> {
        if let Some(text) = self.files.load(path) {
            return Some(text);
        }
        if let Some(text) = qubero_core::hexpat::bundled::BundledIncludes.load(path) {
            return Some(text);
        }
        let mut asked = self.asked.borrow_mut();
        if !asked.iter().any(|p| p == path) {
            asked.push(path.to_string());
        }
        None
    }
}

impl PastedIncludes {
    fn new(files: std::collections::HashMap<String, String>) -> Self {
        PastedIncludes {
            files: qubero_core::hexpat::MapIncludes(files),
            asked: std::cell::RefCell::new(Vec::new()),
        }
    }

    /// The paths this could not answer for and the core's own table cannot
    /// either, which is what the reader still has to supply.
    fn missing(&self) -> Vec<String> {
        self.asked
            .borrow()
            .iter()
            .filter(|path| qubero_core::hexpat::includes::builtin(&strip_pat(path)).is_none())
            .cloned()
            .collect()
    }
}

/// A path with its `.pat` or `.hexpat` off, which is the form the two spellings
/// of an include agree on.
fn strip_pat(path: &str) -> String {
    let path = path.replace('\\', "/");
    for extension in [".hexpat", ".pat"] {
        if let Some(stem) = path.strip_suffix(extension) {
            return stem.to_string();
        }
    }
    path
}

/// Every `#include` and `import` written in a pattern, as the resolver would
/// see the path, minus the ones already answered for.
///
/// Parsing stops at the first path it cannot find, so the recorder alone would
/// name one file at a time and a pattern wanting four would take four rounds.
/// This says what the file asks for up front. It is a scan and not a parse, so
/// it sees only what this file writes: an include's own includes appear as they
/// are supplied.
fn hexpat_wanted(text: &str, supplied: &std::collections::HashMap<String, String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        let path = if let Some(rest) = line.strip_prefix("#include") {
            let rest = rest.trim();
            rest.strip_prefix('<')
                .and_then(|r| r.split('>').next())
                .or_else(|| rest.strip_prefix('"').and_then(|r| r.split('"').next()))
                .map(|p| p.trim().to_string())
        } else if let Some(rest) = line.strip_prefix("import ") {
            // `import std.mem;`, and `import * from fs.mbr as MBR;`, which
            // names its file after the `from`.
            let rest = rest.trim().trim_end_matches(';');
            let rest = rest.split(" as ").next().unwrap_or(rest).trim();
            let rest = rest.rsplit(" from ").next().unwrap_or(rest).trim();
            if rest.is_empty() || rest.contains(' ') {
                None
            } else {
                Some(rest.replace('.', "/"))
            }
        } else {
            None
        };
        let Some(path) = path else { continue };
        let key = strip_pat(&path);
        if qubero_core::hexpat::includes::builtin(&key).is_some() {
            continue;
        }
        if qubero_core::hexpat::bundled::BundledIncludes.load(&path).is_some() {
            continue;
        }
        if supplied.keys().any(|name| strip_pat(name) == key) {
            continue;
        }
        if !out.iter().any(|p| *p == path) {
            out.push(path);
        }
    }
    out
}

/// The error as the panel shows it: `line:col: what was wrong`, with the file
/// named only when it is not the one in the box.
fn hexpat_error(error: &qubero_core::hexpat::HexpatError, name: &str) -> String {
    if error.file.is_empty() || error.file == name {
        format!("{}: {}", error.pos, error.message)
    } else {
        format!("{} {}: {}", error.file, error.pos, error.message)
    }
}

/// The name a converted pattern goes by: what the caller passed, or `pattern`
/// for a paste with no file behind it.
fn hexpat_name(name: &str) -> String {
    let name = name.trim().trim_end_matches(".hexpat").trim_end_matches(".pat").trim();
    if name.is_empty() {
        "pattern".to_string()
    } else {
        name.to_string()
    }
}

/// The chunks a reading waits on, as a byte read reports chunks not loaded.
/// Nothing for any other error: the read then answers from what is there.
fn chunks_of(err: EvalError) -> Vec<f64> {
    match err {
        EvalError::Pending(m) => m.into_iter().map(|m| m.chunk as f64).collect(),
        _ => Vec::new(),
    }
}

/// Build a needle from what the search bar holds. `kind` is "hex", "text" or
/// "regex"; `fold` only means anything for text.
fn needle(kind: &str, text: &str, fold: bool) -> Result<Needle, String> {
    match kind {
        "hex" => search::parse_hex(text).map(Needle::Bytes).ok_or_else(|| hex_trouble(text).to_string()),
        "regex" => search::Pattern::new(text).map(Needle::Regex),
        _ => {
            let bytes = text.as_bytes().to_vec();
            Ok(if fold { Needle::Fold(bytes) } else { Needle::Bytes(bytes) })
        }
    }
}

/// What is wrong with a hex needle. The two are different mistakes: a letter
/// that is not a digit is one, and a byte with one digit so far is the state
/// every valid needle passes through while it is being typed.
fn hex_trouble(text: &str) -> &'static str {
    if text.chars().any(|c| !c.is_whitespace() && !c.is_ascii_hexdigit()) {
        HEX_NOT_A_DIGIT
    } else {
        HEX_HALF_A_BYTE
    }
}

/// How many elements of a list to place before handing back, so the page can
/// draw what it has and say how far it has got. Around a twentieth of a second
/// of work: short enough that the page stays under the hand, long enough that
/// handing back is not most of what is done. Asking again carries on where the
/// last go left off rather than starting over.
const WORK_SLICE: u64 = 5_000;

/// Chunk size of a space's store. Nothing is fetched in chunks for a space, so
/// this only decides what a chunk number means; it matches the file's so the
/// host's chunk arithmetic is the same on both sides.
const SPACE_CHUNK: u64 = 64 * 1024;

const HEX_NOT_A_DIGIT: &str = "Hex is pairs of digits 0-9 a-f, like 89 50 4e 47";
const HEX_HALF_A_BYTE: &str = "Unfinished byte: each byte is two digits, like 4e";

#[wasm_bindgen]
impl Editor {
    /// `len` is the original file length in bytes. Chunks of `chunk_size` bytes
    /// are pushed in by the host with `feed_chunk`; at most `capacity` are kept.
    #[wasm_bindgen(constructor)]
    pub fn new(len: f64, chunk_size: u32, capacity: u32) -> Editor {
        let store = ChunkStore::new(len as u64, chunk_size as u64, capacity as usize);
        Editor { sheets: vec![Sheet::new(store, Vec::new())], live: 0 }
    }

    /// One space's reading, for a call that names it and asks nothing else of
    /// the editor. A number past the spaces open falls back to the file rather
    /// than panicking; nothing should ask, since an edit reopens the spaces in
    /// place rather than dropping them and a tab's number stays good for as
    /// long as the tab does.
    fn at(&self, space: u32) -> &Sheet {
        self.sheets.get(space as usize).unwrap_or(&self.sheets[0])
    }

    fn sm(&mut self) -> &mut Sheet {
        let i = if self.live < self.sheets.len() { self.live } else { 0 };
        &mut self.sheets[i]
    }

    /// Name the space the call is about, opening its stream again first if an
    /// edit has let go of it. Every space-taking method starts here. The error
    /// is the reply to give instead: the reopen waits on chunks of the file.
    fn go(&mut self, space: u32) -> Result<(), String> {
        self.live = space as usize;
        self.ensure_open(space).map_err(|err| reply::<()>(Err(err)))
    }

    /// Let go of every unpacked stream. A space is worked out from bytes of
    /// the file, so an edit or a change of template throws it away and the
    /// tab reopens it by path. The file's own reading, space 0, stays.
    ///
    /// The spaces are not dropped, because the tabs showing them are still
    /// open and a tab has to keep meaning what its title says. Each is marked
    /// stale instead, and `ensure_open` unpacks it again from the stream it
    /// came from the next time anything of the tab is asked about, so the
    /// tab's own number stays good; the core renumbers its own spaces from 1
    /// and the new number is written back then. Opening it then rather than
    /// here is what makes the reopen see the edit: every caller of this has
    /// the edit still to make, or the reading still to tell of it, and a
    /// stream unpacked here would be unpacked from the file as it was. It is
    /// also at most one unpacking per tab however many edits came first.
    fn forget_spaces(&mut self) {
        self.live = 0;
        for sh in self.sheets.iter_mut().skip(1) {
            sh.stale = true;
        }
    }

    /// Open the stream behind `space` again if an edit or a change of template
    /// has let go of it since, whichever way the tab reads it. Nothing to do
    /// for the file, or for a stream still open.
    ///
    /// Its home is opened first, since a stream opened from a recognised one
    /// is opened again in that one's new reading: a sheet's home is always
    /// opened before it. The error is only ever that a reading waits on chunks
    /// of the file, and the sheet then stays as it was until it is asked
    /// again. A stream that no longer opens is not an error: it leaves the tab
    /// empty rather than reading whatever the new reading has at the old path.
    fn ensure_open(&mut self, space: u32) -> Result<(), EvalError> {
        let i = space as usize;
        if i == 0 || i >= self.sheets.len() {
            return Ok(());
        }
        let home = self.sheets[i].home;
        if home != i {
            self.ensure_open(home as u32)?;
        }
        // A sheet still holding a stream its home's reading has since dropped
        // is as stale as one an edit marked. The core lets go of every space
        // whenever a reading is invalidated, so this catches a change that
        // did not pass through `forget_spaces`, and a home opened again since
        // with a reading of its own that has nothing open yet.
        let dropped = self.sheets[i].core_space != 0 && self.core_space(space).is_none();
        if !self.sheets[i].stale && !dropped {
            return Ok(());
        }
        self.reopen(i)
    }

    /// Open the stream at `path` of sheet `home`'s own reading, one go's
    /// worth of work at a time. A deflate, zlib or gzip stream large enough to
    /// take more than a go is unpacked over several: the core keeps the
    /// half-done pass, the call hands back `Busy`, and asking again carries on.
    /// `unpacking` says how far it has got. Nothing when the sheet has no
    /// reading.
    fn open_in(&mut self, home: usize, path: &[usize]) -> Result<Option<SpaceId>, EvalError> {
        let Some(sh) = self.sheets.get_mut(home) else { return Ok(None) };
        let Some(e) = &mut sh.eval else { return Ok(None) };
        e.begin_slice();
        e.open_space(&sh.doc, 0, path)
    }

    /// Which sheet's reading unpacks the streams of the tab over `space`: the
    /// file's for the file, the tab's own for a stream with a reading of its
    /// own, and its home's for one read where it was declared. The same
    /// choice `open_space` makes.
    fn reading_of(&self, space: u32) -> usize {
        match self.sheets.get(space as usize) {
            Some(sh) if space != 0 => match &sh.view {
                Some(_) => sh.home,
                None => space as usize,
            },
            _ => 0,
        }
    }

    /// The sheet for a stream `open_in` opened, or an empty one for a stream
    /// that did not open.
    fn sheet_for(&self, opened: Option<SpaceId>, home: usize, origin: Vec<usize>) -> Sheet {
        let space = opened.and_then(|id| self.sheets[home].eval.as_ref()?.space(id));
        match space {
            Some(sp) => Sheet::from_space(sp, home, origin),
            None => Sheet::empty(home, origin),
        }
    }

    /// What reads the fields of `space`, lent for one call: its own reading, or
    /// for a stream read where it was declared, its home's, under the stream.
    /// The error is the reply to give instead, when nothing reads it, or when
    /// opening the stream again waits on chunks of the file.
    fn tab(&mut self, space: u32) -> Result<Tab<'_, ChunkStore>, String> {
        self.go(space)?;
        let i = if self.live < self.sheets.len() { self.live } else { 0 };
        let no_template = || reply::<()>(Err(EvalError::Failed("no template".into())));
        match self.sheets[i].view.clone() {
            None => {
                let sh = &mut self.sheets[i];
                let e = sh.eval.as_mut().ok_or_else(no_template)?;
                Ok(Tab::new(e, &sh.doc, Vec::new()))
            }
            Some(root) => {
                let h = self.sheets[i].home;
                let home = &mut self.sheets[h];
                let e = home.eval.as_mut().ok_or_else(no_template)?;
                Ok(Tab::new(e, &home.doc, root))
            }
        }
    }

    /// Open the stream sheet `i` came from again, in its home's reading as it
    /// is now. A stream that no longer opens leaves an empty sheet; one waiting
    /// on bytes of the file says so, stays stale, and is opened when it is
    /// asked again.
    fn reopen(&mut self, i: usize) -> Result<(), EvalError> {
        let (home, origin) = (self.sheets[i].home, self.sheets[i].origin.clone());
        let opened = match self.open_in(home, &origin) {
            Ok(opened) => opened,
            Err(err) if err.interrupted() => return Err(err),
            Err(_) => None,
        };
        self.sheets[i] = self.sheet_for(opened, home, origin);
        Ok(())
    }

    /// Open the `Decoded` or joined stream at `path` of the tab over `space` as
    /// a document of its own, and give back the space it became:
    /// {status:"ok",node:{space,refused}}.
    ///
    /// `path` is the tab's, which is a path of the file only in the file's own
    /// tab. A tab over a stream read where it was declared has its fields under
    /// the stream in its home's reading, so the stream is opened there, under
    /// the tab's root, and is then the same stream the file's own tab would
    /// open. A tab with a reading of its own opens it in that reading.
    ///
    /// A stream already open answers with the space it already is, so a second
    /// Open unpacked focuses the tab instead of unpacking the run again.
    /// `refused` says which of the three ways a stream would not open, and the
    /// space is then 0.
    pub fn open_space(&mut self, space: u32, path: &[u32]) -> String {
        if let Err(err) = self.ensure_open(space) {
            return reply::<SpaceDto>(Err(err));
        }
        let p: Vec<usize> = path.iter().map(|&x| x as usize).collect();
        let home = self.reading_of(space);
        let at = match self.sheets.get(space as usize).and_then(|sh| sh.view.as_ref()) {
            Some(root) if space != 0 => [root.as_slice(), &p].concat(),
            _ => p,
        };
        let known = self.sheets.iter().enumerate().skip(1).find(|(_, sh)| sh.home == home && sh.origin == at).map(|(i, _)| i);
        if let Some(i) = known {
            let template = self.sheets[i].template.clone();
            let (joined, stored) = self.space_joined(i as u32);
            return reply(Ok(SpaceDto { space: i as f64, template, refused: None, joined, stored }));
        }
        self.live = 0;
        if self.sheets.get(home).is_none_or(|sh| sh.eval.is_none()) {
            return reply::<SpaceDto>(Err(EvalError::Failed("no template".into())));
        }
        let id = match self.open_in(home, &at) {
            Ok(Some(id)) => id,
            // The stream would not open. Which of the three ways is already on
            // the node, so the reply only has to say that it did not.
            Ok(None) => {
                let why = self.refusal_at(home, &at);
                return reply(Ok(SpaceDto { space: 0.0, template: String::new(), refused: Some(why), joined: false, stored: false }));
            }
            Err(err) => return reply::<SpaceDto>(Err(err)),
        };
        if self.sheets[home].eval.as_ref().and_then(|e| e.space(id)).is_none() {
            return reply::<SpaceDto>(Err(EvalError::Failed("space vanished".into())));
        }
        let sheet = self.sheet_for(Some(id), home, at);
        let template = sheet.template.clone();
        self.sheets.push(sheet);
        let space = (self.sheets.len() - 1) as u32;
        let (joined, stored) = self.space_joined(space);
        reply(Ok(SpaceDto { space: space as f64, template, refused: None, joined, stored }))
    }

    /// How far the pass unpacking a large compressed stream has got, in the
    /// reading the tab over `space` unpacks its streams in:
    /// {status:"ok",node:{run_space,run_offset_bits,read_bits,run_bits,unpacked_bytes}},
    /// or a null node when no pass is under way.
    ///
    /// Asked after any reply that said "working": the pass is charged to the
    /// same allowance as the rest of the reading, so a listing that reaches a
    /// stream's bytes, and Open unpacked, both carry it on. The bits count
    /// from the start of the compressed run, and `run_offset_bits` is where
    /// that is in `run_space`, 0 being the file.
    pub fn unpacking(&self, space: u32) -> String {
        let progress = self.sheets.get(self.reading_of(space)).and_then(|sh| sh.eval.as_ref()).and_then(|e| e.unpacking());
        reply(Ok(progress.map(|p| UnpackingDto {
            run_space: p.run_space,
            run_offset_bits: p.run_offset_bits as f64,
            read_bits: p.read_bits as f64,
            run_bits: p.run_bits as f64,
            unpacked_bytes: p.unpacked_bytes as f64,
        })))
    }

    /// Whether one of this editor's spaces is a stream joined from several
    /// runs, and whether every one of those runs is stored as it sits in the
    /// file, which only a stream opened in the file's reading can say.
    fn space_joined(&self, space: u32) -> (bool, bool) {
        let Some((home, sp)) = self.core_space_of(space) else { return (false, false) };
        let runs = sp.runs();
        (!runs.is_empty(), home == 0 && !runs.is_empty() && runs.iter().all(|r| !r.packed && r.run_space == 0))
    }

    /// Why the stream at `path` of sheet `home`'s reading would not open, in
    /// the core's own word for it. A joined stream too long to hold whole says
    /// so only when asked, since its node still reads.
    fn refusal_at(&mut self, home: usize, path: &[usize]) -> String {
        let Some(sh) = self.sheets.get_mut(home) else { return "failed".into() };
        let Some(e) = &mut sh.eval else { return "failed".into() };
        if let Some(why) = e.open_refusal(path) {
            return why.as_str().into();
        }
        match e.node(&sh.doc, path) {
            Ok(n) => n.refused.unwrap_or_else(|| "failed".into()),
            Err(_) => "failed".into(),
        }
    }

    /// Which bits of the compressed run the byte at `byte` of `space` came
    /// from, and by which step: {status:"ok",node:{..}} or a null node when the
    /// codec's map does not reach that far. The step's bits count from the
    /// start of its run, and `run_offset_bits` says where in the file that is.
    ///
    /// Null too for a stream whose run is bits of another stream and not of
    /// the file: one opened in a recognised stream's reading, or one declared
    /// inside a stream the file declares.
    pub fn map_out(&mut self, space: u32, byte: f64) -> String {
        if let Err(err) = self.ensure_open(space) {
            return reply::<Option<MapStepDto>>(Err(err));
        }
        let Some(core) = self.file_core_space(space) else { return reply(Ok(None::<MapStepDto>)) };
        let Some(e) = &self.sheets[0].eval else { return reply(Ok(None::<MapStepDto>)) };
        reply(Ok(e.map_out(core, byte as u64).and_then(|s| space_step_dto(e, core, s, AskedBy::Byte(byte as u64)))))
    }

    /// `map_out`, including a step whose run is inside another stream rather
    /// than in the file, with `in_file` false: what the report draws a codes
    /// ribbon from for a PNG's zlib stream, whose run is the IDAT chunks'
    /// joined data.
    pub fn map_out_any(&mut self, space: u32, byte: f64) -> String {
        if let Err(err) = self.ensure_open(space) {
            return reply::<Option<MapStepDto>>(Err(err));
        }
        let Some(core) = self.file_core_space(space) else { return reply(Ok(None::<MapStepDto>)) };
        let Some(e) = &self.sheets[0].eval else { return reply(Ok(None::<MapStepDto>)) };
        reply(Ok(e.map_out(core, byte as u64).and_then(|s| any_space_step_dto(e, core, s, byte as u64))))
    }

    /// Which step read the bit at `bit` of the file, when that bit is in the
    /// run `space` was unpacked from, and so which of its bytes that bit
    /// produced. Counted in the file and not in the run, since the file tab's
    /// cursor is what asks.
    pub fn map_in(&mut self, space: u32, bit: f64) -> String {
        if let Err(err) = self.ensure_open(space) {
            return reply::<Option<MapStepDto>>(Err(err));
        }
        let Some(core) = self.file_core_space(space) else { return reply(Ok(None::<MapStepDto>)) };
        let Some(e) = &self.sheets[0].eval else { return reply(Ok(None::<MapStepDto>)) };
        reply(Ok(e.map_in(core, bit as u64).and_then(|s| space_step_dto(e, core, s, AskedBy::Bit(bit as u64)))))
    }

    /// Take the deflate symbol at `bit` of the run `space` was unpacked from
    /// apart, so the sidebar can show what it was made of:
    /// {status:"ok",node:{kind,block,block_kind,symbol,distance,bits}}, or a
    /// null node when there is nothing to take apart.
    ///
    /// `bit` is a bit of the compressed run, counted the way the trace counts
    /// them, which is what `map_out` already hands back as `in_start`. The
    /// step it lands in is found the same way `map_in` finds one.
    ///
    /// Asked for rather than done on the way past. Nothing here is in the
    /// trace: a step is a packed twenty bytes that says a match copied three
    /// bytes from four back, and which symbols carried those two numbers, how
    /// wide their codes were and how many extra bits followed each is thrown
    /// away, because keeping it would cost every step of every stream in
    /// memory. So the block's two Huffman tables are rebuilt and the step's
    /// bits read again, which is worth doing for the one step under the cursor
    /// and not for the millions behind it. Nothing calls this while scrolling.
    ///
    /// Only for a stream this editor has opened as a document of its own,
    /// since that is what carries the trace and the number to ask by. A null
    /// node is the answer for every other case: a codec that is not deflate, a
    /// bit nobody read, a step that is a header or a table entry rather than a
    /// symbol, and a block whose symbols the trace stopped naming.
    pub fn decode_step(&mut self, space: u32, bit: f64) -> String {
        if let Err(err) = self.ensure_open(space) {
            return reply::<Option<DecodedStepDto>>(Err(err));
        }
        let none = || reply(Ok(None::<DecodedStepDto>));
        let (Some(core), Some(sh)) = (self.core_space(space), self.sheets.get(space as usize)) else {
            return none();
        };
        let (home, origin) = (sh.home, sh.origin.clone());
        // Which step, and whether this is a codec with symbols at all. Both
        // come off the trace, so a question with no answer is answered before
        // any of the run is read.
        // The trace that names the bit: the whole run's, or for a run too
        // large to hold whole, the span's that read it. Kept across the read
        // of the run below, since a span's trace is made when asked for.
        let (trace, index, want_bytes) = {
            let Some(e) = &self.sheets[home].eval else { return none() };
            let Some(sp) = e.space(core) else { return none() };
            if !matches!(sp.codec, Codec::Deflate | Codec::Zlib | Codec::Gzip) {
                return none();
            }
            let Some(trace) = sp.trace_at_bit(bit as u64) else { return none() };
            let Some(index) = trace.index_in(bit as u64) else { return none() };
            let Some(step) = trace.step(index) else { return none() };
            // As much of the run as the step reaches, and no more. Bit
            // positions count from the front of the run, so a prefix ending
            // after the step is all the walk can touch.
            let want = step.in_bits.end.div_ceil(8);
            (trace, index, want)
        };
        // The compressed bytes, which the space does not hold: what it holds is
        // what came out. They are a field of the reading that opened the
        // stream, the file's or a recognised stream's, and a read of the
        // file's can want chunks that are not here yet.
        self.live = home;
        let at = &mut self.sheets[home];
        let Some(e) = &mut at.eval else { return none() };
        e.begin_slice();
        let run = match e.field_bytes(&at.doc, &origin, want_bytes) {
            Ok((bytes, _)) => bytes,
            Err(err) => return reply::<Option<DecodedStepDto>>(Err(err)),
        };
        reply(Ok(inflate::decode_step(&run, &trace, index).map(decoded_step_dto)))
    }

    /// The `Decoded` node a space was unpacked from, as a path in the reading
    /// that opened it: the file's, or a recognised stream's. Empty for space 0
    /// and for a space that is no longer open.
    pub fn space_origin(&self, space: u32) -> Vec<u32> {
        match self.sheets.get(space as usize) {
            Some(sh) if space != 0 => sh.origin.iter().map(|&x| x as u32).collect(),
            _ => Vec::new(),
        }
    }

    /// True when the template reading a space came from looking at the unpacked
    /// bytes rather than from what the stream declared: a gzip of a tar opens
    /// as a tar, and this is what says so.
    pub fn space_recognised(&mut self, space: u32) -> bool {
        self.open_again(space);
        self.core_space_of(space).is_some_and(|(_, s)| s.recognised)
    }

    /// The core's space for one of this editor's spaces, if it is still open,
    /// with the sheet whose reading holds it.
    ///
    /// Still open as the same stream: the core numbers its spaces from 1 again
    /// once a reading is thrown away, and a stream opened again since then may
    /// have been given the number this one had.
    fn core_space_of(&self, space: u32) -> Option<(usize, &qubero_core::eval::Space)> {
        let sh = self.sheets.get(space as usize)?;
        if space == 0 || sh.core_space == 0 {
            return None;
        }
        let open = self.sheets.get(sh.home)?.eval.as_ref()?.space(sh.core_space)?;
        (open.parent == 0 && open.path == sh.origin).then_some((sh.home, open))
    }

    /// `ensure_open` for a call that answers from whatever the tab holds when
    /// the reopen waits on chunks of the file: the old bytes, the old length,
    /// or the old reading, which the next call opens again.
    fn open_again(&mut self, space: u32) {
        let _ = self.ensure_open(space);
    }

    /// The core's number for one of this editor's spaces, if it is still open.
    fn core_space(&self, space: u32) -> Option<SpaceId> {
        self.core_space_of(space).map(|(_, sp)| sp.id)
    }

    /// The same, only for a space the file's reading opened, which is the only
    /// reading whose space 0 is the file. Whether its run is bits of the file
    /// or of a stream it was declared inside is the run's to say, and
    /// `space_step_dto` asks it, for a stream unpacked from one run and for
    /// each part of a joined one alike.
    fn file_core_space(&self, space: u32) -> Option<SpaceId> {
        match self.core_space_of(space)? {
            (0, sp) => Some(sp.id),
            _ => None,
        }
    }

    fn changed(&mut self) {
        self.forget_spaces();
        let sh = self.sm();
        if let Some(e) = &mut sh.eval {
            e.invalidate();
        }
        sh.disasm = None;
        sh.bpf = None;
        sh.bpf_complete = false;
        sh.ne = None;
        sh.scan = None;
        sh.focus = None;
        sh.kinds = None;
        sh.census = None;
        sh.report = None;
    }

    /// An edit that replaced bits in place at `bit`. What the template made of
    /// the bytes before it still holds, so only the rest is worked out again.
    fn changed_at(&mut self, bit: u64) {
        self.forget_spaces();
        let sh = self.sm();
        if let Some(e) = &mut sh.eval {
            e.invalidate_from(bit);
        }
        sh.disasm = None;
        sh.bpf = None;
        sh.bpf_complete = false;
        sh.ne = None;
        sh.scan = None;
        sh.focus = None;
        sh.kinds = None;
        sh.census = None;
        sh.report = None;
    }

    /// One step of the byte-class scan behind the overview: at most a window
    /// of the file read and classified. The reply is the usual tri-state, with
    /// `node` carrying everything found so far, so the host can draw a partial
    /// map while the rest is read. `done` on the node says when to stop asking.
    pub fn overview_step(&mut self, space: u32, buckets: u32) -> String {
        if let Err(why) = self.go(space) {
            return why;
        }
        let sh = self.sm();
        let len = sh.doc.len_bytes();
        let want = overview::Scan::range(0, len, u64::from(buckets));
        // A scan already covering the same bytes at the same resolution
        // carries on; anything else starts over, since its classes describe a
        // different division of a different file.
        let same = matches!(&sh.scan, Some(s) if s.end() == len && s.bucket_bytes() == want.bucket_bytes());
        if !same {
            sh.scan = Some(want);
        }
        let scan = sh.scan.as_mut().expect("just built");
        match scan.step(&sh.doc) {
            overview::ScanStep::Pending(m) => reply::<OverviewDto>(Err(EvalError::Pending(m))),
            step => reply(Ok(OverviewDto {
                done: step == overview::ScanStep::Done,
                bucket_bytes: scan.bucket_bytes() as f64,
                total_buckets: scan.total_buckets() as f64,
                classes: scan.classes().iter().map(|&c| char::from(b'0' + c)).collect(),
                zero_bytes: scan.zero_bytes() as f64,
                text_bytes: scan.text_bytes() as f64,
                read_bytes: scan.read_bytes() as f64,
                histogram: histogram(scan),
            })),
        }
    }

    /// One step of the scan over a single block, at whatever resolution that
    /// block's own size allows. Asking about a different block starts a new
    /// one; asking about the same block carries the current one on.
    ///
    /// This is what answers the question the whole-file map cannot: every
    /// bucket of a block can read as dense while the first part of it is
    /// zeroes, because a bucket is judged as a whole.
    pub fn overview_focus_step(&mut self, space: u32, from: f64, to: f64, buckets: u32) -> String {
        if let Err(why) = self.go(space) {
            return why;
        }
        let sh = self.sm();
        let (from, to) = (from as u64, to as u64);
        let fresh = !matches!(&sh.focus, Some(f) if f.start() == from && f.end() == to);
        if fresh {
            sh.focus = Some(overview::Scan::range(from, to, u64::from(buckets)));
        }
        let scan = sh.focus.as_mut().expect("just built");
        match scan.step(&sh.doc) {
            overview::ScanStep::Pending(m) => reply::<FocusDto>(Err(EvalError::Pending(m))),
            step => {
                let (entropy, entropy_max) = scan.entropy();
                let hist = scan.histogram();
                let mut common: Vec<CommonByteDto> = hist
                    .iter()
                    .enumerate()
                    .filter(|(_, n)| **n > 0)
                    .map(|(v, n)| CommonByteDto { value: v as f64, count: *n as f64 })
                    .collect();
                common.sort_by(|a, b| b.count.total_cmp(&a.count));
                common.truncate(COMMON_BYTES);
                reply(Ok(FocusDto {
                    done: step == overview::ScanStep::Done,
                    start: scan.start() as f64,
                    end: scan.end() as f64,
                    bucket_bytes: scan.bucket_bytes() as f64,
                    total_buckets: scan.total_buckets() as f64,
                    classes: scan.classes().iter().map(|&c| char::from(b'0' + c)).collect(),
                    zero_bytes: scan.zero_bytes() as f64,
                    text_bytes: scan.text_bytes() as f64,
                    read_bytes: scan.read_bytes() as f64,
                    histogram: hist.iter().map(|&n| n as f64).collect(),
                    entropy,
                    entropy_max,
                    distinct: scan.distinct() as f64,
                    common,
                }))
            }
        }
    }

    /// One step of the walk that totals the file's bits by field kind and
    /// type. The reply is the usual tri-state, with `node` carrying the totals
    /// so far, so a view can draw a partial answer while the rest is worked
    /// out. `done` on the node says when to stop asking.
    ///
    /// What has not been walked yet is in none of the totals. The caller has
    /// `reached_bits` and the file's length and derives the rest from those,
    /// rather than being handed a bucket for it that would look like a finding
    /// about the file.
    ///
    /// An edit throws the walk away, and the next step starts it over.
    pub fn kind_totals_step(&mut self, space: u32) -> String {
        // The walk is this sheet's own, and a stream read where it was
        // declared walks it in the file's reading, so it is taken out of the
        // sheet for the length of the call. After the sheet is opened again,
        // if it has to be, since that starts the sheet over.
        if let Err(why) = self.tab(space) {
            return why;
        }
        let sh = self.sm();
        let len = sh.doc.len_bits();
        let mut kinds = sh.kinds.take();
        let mut tab = match self.tab(space) {
            Ok(tab) => tab,
            Err(why) => return why,
        };
        // A walk built for a file of another length is about other bytes.
        if !matches!(&kinds, Some(w) if w.file_bits() == len) {
            kinds = Some(tab.kind_walk(len));
        }
        let walk = kinds.as_mut().expect("just built");
        tab.ev.begin_slice();
        let out = tab.kind_totals_step(walk);
        let reached = (tab.ev.reached_bits() / 8) as f64;
        self.sm().kinds = kinds;
        reply_with(out.map(kind_totals_dto), reached, Vec::new())
    }

    /// What the format named `template` is, for the report's opening: its
    /// plain name, its Wikipedia article and a few plain sentences. JSON:
    /// `{name, wikipedia, text}`, with `wikipedia` null where no article
    /// covers it, or `null` for a template with no entry, which is every
    /// bundled Kaitai and ImHex format. Not a reply: nothing is read.
    pub fn format_about(&self, template: &str) -> String {
        match formats::about::about(template) {
            Some(a) => serde_json::to_string(&AboutDto { name: a.name, wikipedia: a.wikipedia, text: a.text }).unwrap_or_else(|_| "null".into()),
            None => "null".into(),
        }
    }

    /// How the template in use writes its values and finds its parts, counted
    /// over its declarations: the report's "How the format is built". JSON,
    /// in the usual reply shape, whose node is
    /// `{done, rows: [{category, kind, width, order, detail, fields, bits,
    /// unpacked_fields, unpacked_bits}], choices: [{key, name, cases, taken,
    /// fields}], facts: {from_end, placed, forward, backward, lengths_before,
    /// lengths_after, every_field_follows}}`. `bits`, `taken`, `forward` and
    /// `backward` are 0 here; see `format_profile_step` for the file's.
    ///
    /// About the format rather than the file, like `template_diagram`: the
    /// same for every file the template opens.
    pub fn template_profile(&mut self, space: u32) -> String {
        if let Err(why) = self.go(space) {
            return why;
        }
        let sh = self.sm();
        match (&sh.eval, &sh.read_as) {
            (Some(e), _) => reply(Ok(profile_dto(qubero_core::eval::template_profile(e.template())))),
            (None, Some(t)) => reply(Ok(profile_dto(qubero_core::eval::template_profile(t)))),
            (None, None) => reply::<ProfileDto>(Err(EvalError::Failed("no template".into()))),
        }
    }

    /// One go of the report's walk, answered with whatever `answer` makes of
    /// it. The walk is the sheet's own and is taken out for the call, the
    /// way the kind walk is: see `kind_totals_step`.
    fn report_go<T: Serialize>(&mut self, space: u32, answer: impl FnOnce(&Tab<'_, ChunkStore>, &ReportWalk) -> T) -> String {
        if let Err(why) = self.tab(space) {
            return why;
        }
        let sh = self.sm();
        let len = sh.doc.len_bits();
        let mut report = sh.report.take();
        let mut tab = match self.tab(space) {
            Ok(tab) => tab,
            Err(why) => return why,
        };
        if !matches!(&report, Some(w) if w.file_bits() == len) {
            report = Some(tab.report_walk(len));
        }
        let walk = report.as_mut().expect("just built");
        tab.ev.begin_slice();
        let out = tab.report_step(walk).map(|()| answer(&tab, walk));
        let reached = (walk.reached_bits() / 8) as f64;
        let wanted = wanted(tab.ev);
        self.sm().report = report;
        reply_with(out, reached, wanted)
    }

    /// One go of the report's walk, answered with how this file writes its
    /// values and finds its parts: the same shape as `template_profile`,
    /// counted over the file's fields, with `bits` and the pointer
    /// directions filled in. `done` says when to stop asking.
    ///
    /// The report's stepped answers share one walk, so asking for any of
    /// them carries all of them on. An edit throws the walk away.
    pub fn format_profile_step(&mut self, space: u32) -> String {
        self.report_go(space, |_, w| profile_dto(w.profile()))
    }

    /// One go of the report's walk, answered with where every bit of the
    /// file goes: `{done, reached_bits, file_bits, counted_bits, rows:
    /// [{part, part_name, group, group_from, role, bits, count, zero_bits,
    /// unscanned_bits, first_path, first_offset_bits, align}]}`. `part` and
    /// `first_path` are paths in this space.
    pub fn byte_ledger_step(&mut self, space: u32) -> String {
        self.report_go(space, |tab, w| ledger_dto(tab.report_ledger(w), w.reached_bits()))
    }

    /// One go of the report's walk, answered with every length checked
    /// against the part it sizes: `{done, counts: [{verdict, count}], checks:
    /// [{length_path, length_name, length_value, length_invalid, part_path,
    /// part_name, role, offset_bits, stated, read, content_bits, room_bits,
    /// space_bits, adjusted, verdict, why}], root_end_bits, space_bits,
    /// root_failed}`.
    pub fn extent_audit_step(&mut self, space: u32) -> String {
        self.report_go(space, |tab, w| extent_audit_dto(tab.report_audit(w)))
    }

    /// One go of the report's walk, answered with every list whose elements
    /// place something elsewhere and what each places: `{done, unexamined,
    /// lists: [{path, name, elements, placing, entries: [{path, name,
    /// offset_bits, size_bits, targets: [{path, name, offset_bits, size_bits,
    /// via, aside}]}]}]}`. Found once the walk is over, so `lists` is empty
    /// and `done` false until then.
    pub fn directories_step(&mut self, space: u32) -> String {
        self.report_go(space, |tab, w| directories_dto(tab.report_directories(w)))
    }

    /// One go of the report's walk, answered with every run the template
    /// reads as numbers that holds text: `{done, min_chars, numeric_bytes,
    /// scanned_bytes, runs: [{path, name, what, element_bits, offset_bits,
    /// size_bits, scanned_bytes, texts, text_bytes, first: [{offset_bits,
    /// size_bits, encoding, text}]}]}`. The runs are read once the walk is
    /// over, so `runs` is empty and `done` false until then. `scanned_bytes`
    /// below `numeric_bytes` means only part of the numbers were read.
    pub fn text_in_numbers_step(&mut self, space: u32) -> String {
        self.report_go(space, |tab, w| text_in_numbers_dto(tab.report_text_in_numbers(w)))
    }

    // ----- templates -----

    /// Every template a reader may pick, as JSON: the built-in ones first,
    /// then the bundled Kaitai Struct formats.
    ///
    /// Each carries the name [`set_template`](Self::set_template) takes, the
    /// title to show for it, and which of the two kinds it is, because the
    /// chooser groups them. A built-in has no title of its own: the name is
    /// what it has always been shown as, and the web side already knows how to
    /// spell each one out. A bundled format's title is its `meta/title`, which
    /// is empty for the third of them that carry none.
    pub fn template_names(&self) -> String {
        let builtins = formats::builtin_names().into_iter().map(|name| TemplateChoiceDto {
            name: name.to_string(),
            title: String::new(),
            source: "builtin",
            ext: Vec::new(),
            magic: Vec::new(),
        });
        let kaitai = qubero_core::ksy::bundled::all()
            .iter()
            .filter(|entry| entry.offered)
            .map(|entry| TemplateChoiceDto {
                name: entry.name.to_string(),
                title: entry.title.to_string(),
                source: "kaitai",
                ext: ksy_extensions(entry.text),
                magic: entry.signature.iter().map(|(at, bytes)| (*at, bytes.iter().map(|b| format!("{b:02x}")).collect())).collect(),
            });
        let imhex = qubero_core::hexpat::bundled::all()
            .iter()
            .filter(|entry| entry.offered)
            .map(|entry| TemplateChoiceDto {
                name: entry.name.to_string(),
                title: entry.title.to_string(),
                source: "hexpat",
                // An ImHex pattern says nothing about file extensions; what it
                // says about the files it is for is its magic.
                ext: Vec::new(),
                magic: entry.signature.iter().map(|(at, bytes)| (*at, bytes.iter().map(|b| format!("{b:02x}")).collect())).collect(),
            });
        serde_json::to_string(&builtins.chain(kaitai).chain(imhex).collect::<Vec<_>>()).unwrap_or_default()
    }

    /// The template in use, written out as text: every type, every field and
    /// every expression behind them. Empty when no template is selected.
    ///
    /// The file's own template, space 0, since that is the one `set_template`
    /// sets; an unpacked stream is read by whatever its `Decoded` node declared
    /// and has no template of the reader's choosing. Read-only, and read in one
    /// go: it is a page of text, not a walk.
    pub fn template_text(&self) -> String {
        match self.sheets[0].eval.as_ref() {
            Some(e) => qubero_core::template_text::render(e.template()),
            None => String::new(),
        }
    }

    /// Best current projection for a variable-size array being walked, or an
    /// empty string when no unfinished walk has enough information yet.
    pub fn extent_estimate(&mut self, space: u32) -> String {
        let Ok(tab) = self.tab(space) else { return String::new() };
        tab.extent_estimate()
            .map(extent_estimate_dto)
            .map(|estimate| serde_json::to_string(&estimate).unwrap_or_default())
            .unwrap_or_default()
    }

    /// How many leading bytes `sniff_template` wants. A few formats keep what
    /// identifies them well past the start of the file, so a shorter head
    /// silently misses them.
    pub fn sniff_window(&self) -> f64 {
        formats::SNIFF_WINDOW as f64
    }

    /// Name of the built-in template matching these leading bytes, or "".
    /// `file_len` is the length of the whole file, which a format whose
    /// header is a table of offsets weighs its pointers against. `name` is
    /// the file's name, which picks between two bundled formats that share a
    /// magic, as `.shp` and `.shx` do, and claims a format with no signature,
    /// as `.tga` or `.bson`, when the bytes agree with it.
    pub fn sniff_template(&self, head: &[u8], file_len: f64, name: &str) -> String {
        formats::sniff_named(head, file_len as u64, name).unwrap_or("").to_string()
    }

    /// The longest file worth reading whole to ask `pickle_is_familiar` about.
    pub fn pickle_familiar_limit(&self) -> f64 {
        formats::pickle::FAMILIAR_MOST_BYTES as f64
    }

    /// Whether the whole of a pickle matches a Familiar Pickle Form. Asked
    /// about a file `sniff_template` called `pickle` from a window shorter
    /// than the file, which is every pickle holding an array of any size.
    pub fn pickle_is_familiar(&self, whole: &[u8]) -> bool {
        formats::pickle::is_familiar(whole)
    }

    /// How many trailing bytes `sniff_template_ends` wants: enough for a ZIP's
    /// end record, its comment and a central directory of some thousands of
    /// entries.
    pub fn sniff_tail_window(&self) -> f64 {
        formats::SNIFF_TAIL_WINDOW as f64
    }

    /// The same question asked of the leading bytes and the trailing ones, for
    /// a file the leading bytes alone call a ZIP: its central directory names
    /// every entry, and a BP5 dataset or a Zarr store whose files sit behind a
    /// large data file is told by those names. `tail` is the last bytes of the
    /// file.
    pub fn sniff_template_ends(&self, head: &[u8], tail: &[u8], file_len: f64) -> String {
        formats::sniff_ends(head, tail, file_len as u64).unwrap_or("").to_string()
    }

    /// What a BGZF file holds, from the front of its first block in these
    /// leading bytes: `bam`, `csi`, `vcf`, `bed`, `fasta` or `text`, or "" when
    /// that cannot be told. See `formats::bgzf_contents`.
    pub fn bgzf_contents(&self, head: &[u8]) -> String {
        formats::bgzf_contents(head).unwrap_or("").to_string()
    }

    /// Select a template by name; "" clears it. Returns false if unknown.
    ///
    /// A name starting `ksy:` is one of the bundled Kaitai Struct formats and
    /// one starting `hexpat:` is one of the bundled ImHex patterns, converted
    /// here and now. The conversion report is kept beside the template, so
    /// [`ksy_report`](Self::ksy_report) and
    /// [`hexpat_report`](Self::hexpat_report) answer for a bundled format
    /// exactly as they do for a description the reader pasted in: a bundled
    /// format may have gaps, and they are only honest if they are visible.
    pub fn set_template(&mut self, name: &str) -> bool {
        // A different template may not have the stream a space came from, so
        // the spaces are worked out again against it before anything else.
        self.forget_spaces();
        let sh = self.sm();
        sh.disasm = None;
        sh.bpf = None;
        sh.bpf_complete = false;
        sh.ne = None;
        // Every path in the walk is a path through the template that was in
        // use, and under another template the same path is another field. The
        // byte-class scan beside it is about bytes and stands; this does not.
        sh.kinds = None;
        sh.census = None;
        sh.report = None;
        sh.template = name.to_string();
        sh.ksy_report = String::new();
        sh.hexpat_report = String::new();
        if name.is_empty() {
            sh.eval = None;
            return true;
        }
        let template = if let Some(id) = name.strip_prefix(qubero_core::ksy::bundled::PREFIX) {
            match qubero_core::ksy::bundled::template(id) {
                Some(Ok(converted)) => {
                    sh.ksy_report =
                        serde_json::to_string(&ksy_report_dto(&converted.report, &converted.template.name)).unwrap_or_default();
                    Some(converted.template)
                }
                // A bundled description that no longer converts is a broken
                // build, not a format the reader chose wrongly; there is
                // nothing to select and nothing useful to say here about why.
                Some(Err(_)) | None => None,
            }
        } else if let Some(id) = name.strip_prefix(qubero_core::hexpat::bundled::PREFIX) {
            match qubero_core::hexpat::bundled::template(id) {
                Some(Ok(converted)) => {
                    sh.hexpat_report = serde_json::to_string(&hexpat_report_dto(
                        &converted.report,
                        &converted.template.name,
                        Vec::new(),
                    ))
                    .unwrap_or_default();
                    Some(converted.template)
                }
                Some(Err(_)) | None => None,
            }
        } else {
            formats::builtin(name)
        };
        match template {
            Some(t) => {
                let mut e = Evaluator::new(t);
                e.set_slice(Some(WORK_SLICE));
                sh.eval = Some(e);
                true
            }
            None => false,
        }
    }

    /// Convert a Kaitai Struct `.ksy` and read the file with what comes out.
    ///
    /// `imports_json` is a JSON object mapping an import name to the text of
    /// that `.ksy`, for a format whose `meta/imports` names others; `{}` where
    /// it imports nothing. What comes back is the usual reply envelope: `ok`
    /// with the conversion report, or `error` with the path in the `.ksy` and
    /// what was wrong with it.
    ///
    /// The report is worth reading even on success. A `.ksy` says things the
    /// IR cannot, and each of those is a gap in the report with the path, the
    /// text it came from and the reason; the field is left as bytes rather
    /// than guessed at. See [`ksy_report`](Self::ksy_report).
    pub fn set_ksy_template(&mut self, text: &str, imports_json: &str) -> String {
        let map = match ksy_imports(imports_json) {
            Ok(map) => map,
            Err(message) => {
                return serde_json::to_string(&Reply::<KsyReportDto>::Error { message }).unwrap_or_default();
            }
        };
        let imports = PastedImports(qubero_core::ksy::MapImports(map));
        let converted = match qubero_core::ksy::convert(text, &imports) {
            Ok(converted) => converted,
            Err(e) => {
                return serde_json::to_string(&Reply::<KsyReportDto>::Error { message: e.to_string() })
                    .unwrap_or_default();
            }
        };
        // A different template may not have the stream a space came from; see
        // `set_template`, which throws the same working away for the same
        // reasons.
        self.forget_spaces();
        let report = ksy_report_dto(&converted.report, &converted.template.name);
        let sh = self.sm();
        sh.disasm = None;
        sh.bpf = None;
        sh.bpf_complete = false;
        sh.ne = None;
        sh.kinds = None;
        sh.census = None;
        sh.report = None;
        sh.template = converted.template.name.clone();
        let mut e = Evaluator::new(converted.template);
        e.set_slice(Some(WORK_SLICE));
        sh.eval = Some(e);
        sh.ksy_report = serde_json::to_string(&report).unwrap_or_default();
        sh.hexpat_report = String::new();
        serde_json::to_string(&Reply::Ok { node: report, wanted: Vec::new() }).unwrap_or_default()
    }

    /// The report from the last `.ksy` converted into this space, as JSON.
    /// Empty when the template in use did not come from one.
    pub fn ksy_report(&mut self, space: u32) -> String {
        self.open_again(space);
        self.at(space).ksy_report.clone()
    }

    /// A bundled format's `.ksy`, byte for byte, by its `meta/id` (the part of
    /// `ksy:png` after the colon). Empty for an id nothing here has.
    ///
    /// This is the file as it was copied from the Kaitai Struct library, which
    /// is what the converter panel shows a reader who asks to see the
    /// description behind a shipped format.
    pub fn bundled_ksy_text(&self, id: &str) -> String {
        qubero_core::ksy::bundled::find(id).map(|b| b.text.to_string()).unwrap_or_default()
    }

    /// Convert a `.ksy` and say what it became, without reading anything with
    /// it. The document keeps the template it had.
    ///
    /// This is what the converter panel calls as the text is typed: the reply
    /// carries the same report [`set_ksy_template`](Self::set_ksy_template)
    /// gives, plus the template written out as text, so a reader sees what a
    /// `.ksy` would produce before deciding to read the file with it.
    pub fn preview_ksy_template(&self, text: &str, imports_json: &str) -> String {
        let map = match ksy_imports(imports_json) {
            Ok(map) => map,
            Err(message) => {
                return serde_json::to_string(&Reply::<KsyPreviewDto>::Error { message }).unwrap_or_default();
            }
        };
        let imports = PastedImports(qubero_core::ksy::MapImports(map));
        let converted = match qubero_core::ksy::convert(text, &imports) {
            Ok(converted) => converted,
            Err(e) => {
                return serde_json::to_string(&Reply::<KsyPreviewDto>::Error { message: e.to_string() })
                    .unwrap_or_default();
            }
        };
        let node = KsyPreviewDto {
            report: ksy_report_dto(&converted.report, &converted.template.name),
            text: qubero_core::template_text::render(&converted.template),
        };
        serde_json::to_string(&Reply::Ok { node, wanted: Vec::new() }).unwrap_or_default()
    }

    /// Convert an ImHex pattern and read the file with what comes out.
    ///
    /// `includes_json` is a JSON object mapping an include path such as
    /// `std/mem.pat` to the text of that file, for a pattern that includes one
    /// Qubero does not have; `{}` where it needs none. `name` is what the
    /// template goes by afterwards, which is the file's name where it has one.
    ///
    /// What comes back is the usual reply envelope. `ok` carries the conversion
    /// report; `error` carries `line:col` and what was wrong there, plus the
    /// include paths the pattern asks for and nothing here can answer.
    ///
    /// The report is worth reading even on success. The ImHex pattern language
    /// has an imperative half that no template can hold, and every piece of it
    /// is a gap in the report with the line it came from; the field is left as
    /// bytes rather than guessed at. See [`hexpat_report`](Self::hexpat_report).
    pub fn set_hexpat_template(&mut self, text: &str, includes_json: &str, name: &str) -> String {
        let map = match ksy_imports(includes_json) {
            Ok(map) => map,
            Err(message) => {
                return serde_json::to_string(&Reply::<HexpatReportDto>::Error { message }).unwrap_or_default();
            }
        };
        let wanted = hexpat_wanted(text, &map);
        let includes = PastedIncludes::new(map);
        let name = hexpat_name(name);
        let converted = match qubero_core::hexpat::convert_named(&name, text, &includes) {
            Ok(converted) => converted,
            Err(e) => {
                let mut missing = wanted;
                for path in includes.missing() {
                    if !missing.contains(&path) {
                        missing.push(path);
                    }
                }
                let node = HexpatFailedDto { message: hexpat_error(&e, &name), missing };
                return serde_json::to_string(&HexpatFailedReply { status: "error", node }).unwrap_or_default();
            }
        };
        // A different template may not have the stream a space came from; see
        // `set_template`, which throws the same working away for the same
        // reasons.
        self.forget_spaces();
        let report = hexpat_report_dto(&converted.report, &converted.template.name, includes.missing());
        let sh = self.sm();
        sh.disasm = None;
        sh.bpf = None;
        sh.bpf_complete = false;
        sh.ne = None;
        sh.kinds = None;
        sh.census = None;
        sh.report = None;
        sh.template = converted.template.name.clone();
        let mut e = Evaluator::new(converted.template);
        e.set_slice(Some(WORK_SLICE));
        sh.eval = Some(e);
        sh.ksy_report = String::new();
        sh.hexpat_report = serde_json::to_string(&report).unwrap_or_default();
        serde_json::to_string(&Reply::Ok { node: report, wanted: Vec::new() }).unwrap_or_default()
    }

    /// The report from the last ImHex pattern converted into this space, as
    /// JSON. Empty when the template in use did not come from one.
    pub fn hexpat_report(&mut self, space: u32) -> String {
        self.open_again(space);
        self.at(space).hexpat_report.clone()
    }

    /// A bundled pattern's `.hexpat`, byte for byte, by its id (the part of
    /// `hexpat:vhd` after the colon). Empty for an id nothing here has.
    ///
    /// This is the file as it was copied from ImHex-Patterns, which is what the
    /// converter panel shows a reader who asks to see the pattern behind a
    /// shipped format.
    pub fn bundled_hexpat_text(&self, id: &str) -> String {
        qubero_core::hexpat::bundled::find(id).map(|b| b.text.to_string()).unwrap_or_default()
    }

    /// Convert an ImHex pattern and say what it became, without reading
    /// anything with it. The document keeps the template it had.
    ///
    /// This is what the converter panel calls as the text is typed: the reply
    /// carries the same report [`set_hexpat_template`](Self::set_hexpat_template)
    /// gives, plus the template written out as text.
    pub fn preview_hexpat_template(&self, text: &str, includes_json: &str, name: &str) -> String {
        let map = match ksy_imports(includes_json) {
            Ok(map) => map,
            Err(message) => {
                return serde_json::to_string(&Reply::<HexpatPreviewDto>::Error { message }).unwrap_or_default();
            }
        };
        let wanted = hexpat_wanted(text, &map);
        let includes = PastedIncludes::new(map);
        let name = hexpat_name(name);
        let converted = match qubero_core::hexpat::convert_named(&name, text, &includes) {
            Ok(converted) => converted,
            Err(e) => {
                let mut missing = wanted;
                for path in includes.missing() {
                    if !missing.contains(&path) {
                        missing.push(path);
                    }
                }
                let node = HexpatFailedDto { message: hexpat_error(&e, &name), missing };
                return serde_json::to_string(&HexpatFailedReply { status: "error", node }).unwrap_or_default();
            }
        };
        let node = HexpatPreviewDto {
            report: hexpat_report_dto(&converted.report, &converted.template.name, includes.missing()),
            text: qubero_core::template_text::render(&converted.template),
        };
        serde_json::to_string(&Reply::Ok { node, wanted: Vec::new() }).unwrap_or_default()
    }

    /// Build a template from a `file(1)` rule file and select it, for a format
    /// with no built-in. `rules` is the text of the one rule file the
    /// identification named, `head` the file's first bytes.
    ///
    /// What comes out covers the format's signature and nothing else, so most
    /// of the file stays unannotated. Returns false when the rule pins no fixed
    /// bytes to a fixed place, which is the honest answer for a format found by
    /// searching rather than by looking.
    pub fn set_magic_template(&mut self, name: &str, rules: &str, head: &[u8]) -> bool {
        // A signature template has no stream for a space to have come from;
        // see `set_template`.
        self.forget_spaces();
        let sh = self.sm();
        // A signature template covers a format's first bytes only, so whatever
        // full template was in use no longer applies.
        sh.disasm = None;
        sh.bpf = None;
        sh.bpf_complete = false;
        sh.ne = None;
        // The paths the walk holds are paths through the template it is
        // leaving. See `set_template`.
        sh.kinds = None;
        sh.census = None;
        sh.report = None;
        sh.template = String::new();
        // A signature template came from a `file(1)` rule and from no format
        // description, so the reports of the last one are about another
        // template and would have the panel and the overview note talking
        // about a template nothing is reading the file with.
        sh.ksy_report = String::new();
        sh.hexpat_report = String::new();
        match magicrule::match_signature(rules, head) {
            Some(sig) => {
                sh.eval = Some(Evaluator::new(magicrule::signature_template(name, &sig)));
                true
            }
            None => false,
        }
    }

    /// What the type at `path` permits, beyond what its value shows: the other
    /// values an enum names, the bytes a magic field wanted, or what each bit
    /// of a flags field means. JSON, in the same reply shape as the rest.
    ///
    /// `at_bits` is where the cursor is. Only a block of packed weights uses
    /// it, to say which weight the reader is standing on.
    pub fn type_info(&mut self, space: u32, path: &[u32], at_bits: f64) -> String {
        let p: Vec<usize> = path.iter().map(|&x| x as usize).collect();
        let at = (at_bits >= 0.0).then(|| at_bits as u64);
        let mut tab = match self.tab(space) {
            Ok(tab) => tab,
            Err(why) => return why,
        };
        tab.ev.begin_slice();
        reply(tab.explain(&p, at).map(explain_dto))
    }

    /// Which fields settled the shape of the one at `path`, and where this one
    /// points if it holds an offset. JSON, in the same reply shape as the rest;
    /// usually an empty list, since most fields are placed and sized outright.
    pub fn origins(&mut self, space: u32, path: &[u32]) -> String {
        let p: Vec<usize> = path.iter().map(|&x| x as usize).collect();
        let mut tab = match self.tab(space) {
            Ok(tab) => tab,
            Err(why) => return why,
        };
        tab.ev.begin_slice();
        reply(tab.origins(&p).map(|v| v.into_iter().map(origin_dto).collect::<Vec<_>>()))
    }

    /// How the field at `path` was placed and how it was sized, in one word
    /// each. JSON, in the same reply shape as the rest.
    ///
    /// Answered for every field, which is what tells it from `origins`: a `u32`
    /// in a header has no origins and is still somewhere for a reason, and the
    /// reason is that the field in front of it ended there.
    pub fn shape(&mut self, space: u32, path: &[u32]) -> String {
        let p: Vec<usize> = path.iter().map(|&x| x as usize).collect();
        let mut tab = match self.tab(space) {
            Ok(tab) => tab,
            Err(why) => return why,
        };
        tab.ev.begin_slice();
        reply(tab.shape(&p).map(|s| ShapeDto { placed: s.placed.as_str(), sized: s.sized.as_str() }))
    }

    /// Which part of a stream joined from several runs the field at `path`
    /// starts in, or null for a field that is not inside one. JSON, in the
    /// same reply shape as the rest.
    ///
    /// A PDB stream in pieces and a BAM read through its BGZF blocks are the
    /// two such streams. What comes back names the run the field's first byte
    /// is kept in, which is a field of the file with a place to go to, and how
    /// far into what that run gives the byte is; for a BGZF block, the virtual
    /// offset an index would name the byte by. Nothing is unpacked to answer.
    pub fn part_of(&mut self, space: u32, path: &[u32]) -> String {
        let p: Vec<usize> = path.iter().map(|&x| x as usize).collect();
        let mut tab = match self.tab(space) {
            Ok(tab) => tab,
            Err(why) => return why,
        };
        tab.ev.begin_slice();
        reply(tab.part_of(&p).map(|h| h.map(stitched_part_dto)))
    }

    /// The same answer for byte `byte` of a joined stream opened as a tab of
    /// its own, asked by the byte rather than by a field. Null for a tab that
    /// was unpacked from one run, and for the file.
    ///
    /// The tab's own reading knows nothing of the runs, since its bytes are
    /// its own. The reading that opened it does, under the node that joined them, and
    /// byte `byte` of the tab is byte `byte` of the stream there: the parts are
    /// laid end to end in the same order and cut at the same length. The node
    /// the stream holds says which space that is.
    pub fn part_at(&mut self, space: u32, byte: f64) -> String {
        let none = || reply(Ok(None::<StitchedPartDto>));
        if let Err(err) = self.ensure_open(space) {
            return reply::<Option<StitchedPartDto>>(Err(err));
        }
        if space == 0 || !self.space_joined(space).0 {
            return none();
        }
        let Some((home, mut root)) = self.sheets.get(space as usize).map(|sh| (sh.home, sh.origin.clone())) else { return none() };
        root.push(0);
        let at = &mut self.sheets[home];
        let Some(e) = &mut at.eval else { return none() };
        e.begin_slice();
        let hit = e.node(&at.doc, &root).and_then(|n| e.part_of(&at.doc, n.space, byte as u64));
        reply(hit.map(|h| h.map(stitched_part_dto)))
    }

    /// What the field at `path` checks, or null when it checks nothing. JSON,
    /// in the same reply shape as the rest.
    ///
    /// Cheap, and asked of any field: no bytes of the covered run are read, so
    /// a panel can ask on every move of the cursor and decide from
    /// `covered_bytes` whether to take the sum without being asked. Taking it
    /// is `run_check`.
    pub fn check_of(&mut self, space: u32, path: &[u32]) -> String {
        let p: Vec<usize> = path.iter().map(|&x| x as usize).collect();
        let mut tab = match self.tab(space) {
            Ok(tab) => tab,
            Err(why) => return why,
        };
        tab.ev.begin_slice();
        reply(tab.check_of(&p).map(|c| {
            c.map(|c| CheckDto {
                algorithm: c.algorithm,
                over: c.over.map(|(at, len)| [at as f64, len as f64]),
                unpacked_from: c.unpacked_from.map(|(at, len)| [at as f64, len as f64]),
                covered_bytes: c.covered_bytes as f64,
                covered_exact: c.covered_exact,
                unpacked_member: c.unpacked_member,
                blanked: c.blanked.map(|b| [b.at as f64, b.len as f64, b.byte as f64]),
            })
        }))
    }

    /// The moment the field at `path` means, or null when it means none. JSON,
    /// in the same reply shape as the rest.
    ///
    /// Cheap, and asked of any field: the field's own bytes are read, which a
    /// panel showing its value has already paid for, and nothing else is. The
    /// stored number is not in here, because it is already on the value row and
    /// this is an addition to it rather than a replacement for it.
    pub fn time_of(&mut self, space: u32, path: &[u32]) -> String {
        let p: Vec<usize> = path.iter().map(|&x| x as usize).collect();
        match self.tab(space) {
            Err(why) => why,
            Ok(mut tab) => {
                tab.ev.begin_slice();
                reply(tab.time_of(&p).map(|t| {
                    t.map(|t| {
                        let (state, unix_seconds, nanos) = match t.moment {
                            Moment::At { unix_seconds, nanos } => ("at", Some(unix_seconds as f64), Some(nanos as f64)),
                            Moment::LeapSecond { unix_seconds, nanos } => ("leap", Some(unix_seconds as f64), Some(nanos as f64)),
                            Moment::Unset => ("unset", None, None),
                            Moment::Impossible => ("impossible", None, None),
                        };
                        let note = t.note.map(|n| match n {
                            TimeNote::PastLeapSecondTable => "past_leap_second_table",
                            TimeNote::BeforeLeapSeconds => "before_leap_seconds",
                        });
                        TimeDto {
                            state,
                            unix_seconds,
                            nanos,
                            note,
                            leap_table_expires: matches!(t.note, Some(TimeNote::PastLeapSecondTable))
                                .then_some(leap_seconds::EXPIRES as f64),
                            zone: match t.zone {
                                Zone::Utc => "utc",
                                Zone::Local => "local",
                                Zone::Unknown => "unknown",
                            },
                            step_nanos: t.step_nanos as f64,
                        }
                    })
                }))
            }
        }
    }

    /// The rows `from` up to `to` of the pandas frame at `path`, each as one
    /// cell a column. JSON, in the same reply shape as the rest.
    ///
    /// A frame's cells are not nodes of the tree: its values are in blocks
    /// written the other way up from the frame, a categorical column is codes
    /// into another array, and a counted index is not written down at all. So
    /// the reading is the core's and the view asks for the rows it is showing.
    pub fn pickle_cells(&mut self, space: u32, path: &[u32], from: f64, to: f64) -> String {
        let p: Vec<usize> = path.iter().map(|&x| x as usize).collect();
        let (from, to) = (from.max(0.0) as u64, to.max(0.0) as u64);
        match self.tab(space) {
            Err(why) => why,
            Ok(mut tab) => {
                tab.ev.begin_slice();
                reply(tab.pickle_cells(&p, from, to).map(|rows| {
                    rows.into_iter()
                        .map(|row| {
                            row.into_iter()
                                .map(|cell| {
                                    let at = CellAtDto::from(cell.at);
                                    let masked = cell.masked;
                                    match cell.value {
                                        None => FrameCellDto { text: String::new(), kind: "absent", at, masked },
                                        Some(v) => {
                                            let (kind, text, _) = shown(&v);
                                            FrameCellDto { text, kind, at, masked }
                                        }
                                    }
                                })
                                .collect::<Vec<FrameCellDto>>()
                        })
                        .collect::<Vec<Vec<FrameCellDto>>>()
                }))
            }
        }
    }

    /// What table the field at `path` reads as, or null when the template
    /// makes no such claim about it. JSON, in the same reply shape as the
    /// rest.
    ///
    /// Asked once when a table is opened, not on the way past: it reads the
    /// fields the shape names, which is a handful of numbers somewhere else in
    /// the file. `NodeDto::table` is the cheap answer to whether there is one
    /// at all. A part of the shape this file does not answer comes back null
    /// rather than guessed; bytes that have not arrived answer pending like
    /// every other call.
    pub fn table_shape(&mut self, space: u32, path: &[u32]) -> String {
        let p: Vec<usize> = path.iter().map(|&x| x as usize).collect();
        match self.tab(space) {
            Err(why) => why,
            Ok(mut tab) => {
                tab.ev.begin_slice();
                reply(tab.table_shape(&p).map(|t| {
                    t.map(|t| TableShapeDto {
                        columns: t.columns.map(|c| c as f64),
                        names: t.names,
                        units: t.units,
                        column_word: t.column_word,
                        row_word: t.row_word,
                        rate: t.rate.map(|r| r as f64),
                        facts: t
                            .facts
                            .into_iter()
                            .map(|o| TableFactDto {
                                label: o.label,
                                path: o.path.into_iter().map(|x| x as f64).collect(),
                                value: o.value,
                            })
                            .collect(),
                        cells: t.cells.map(|c| match c {
                            qubero_core::template::Cells::Named { row, cell, value } => CellsDto::Named {
                                row: row.to_string(),
                                cell: cell.to_string(),
                                value: value.map(|v| v.to_string()),
                            },
                            qubero_core::template::Cells::Computed { rows } => CellsDto::Computed { rows: rows as f64 },
                        }),
                    })
                }))
            }
        }
    }

    /// Take the checksum at `path` and compare it with what the file wrote.
    /// JSON, in the same reply shape as the rest; null when the field checks
    /// nothing.
    ///
    /// This reads the covered bytes and unpacks the covered stream, so it is
    /// asked for rather than done on the way past, and it answers `pending`
    /// like any other read when the bytes are not here yet. A check that
    /// cannot be made is an error with the reason in it, never a mismatch.
    pub fn run_check(&mut self, space: u32, path: &[u32]) -> String {
        let p: Vec<usize> = path.iter().map(|&x| x as usize).collect();
        let mut tab = match self.tab(space) {
            Ok(tab) => tab,
            Err(why) => return why,
        };
        tab.ev.begin_slice();
        reply(tab.run_check(&p).map(|v| v.map(|v| VerdictDto { computed: v.computed, stored: v.stored, ok: v.ok })))
    }

    /// The same question asked of every field under `path` at once: the nodes
    /// of the subtree and every connection between two of them, ready to be
    /// laid out. JSON, in the same reply shape as the rest.
    ///
    /// `limit` caps the nodes. The walk is breadth-first, so what a cap keeps
    /// is the top of the format rather than one deep spine of it, and the
    /// answer says how many nodes it left out.
    pub fn graph(&mut self, space: u32, path: &[u32], limit: u32) -> String {
        let p: Vec<usize> = path.iter().map(|&x| x as usize).collect();
        let mut tab = match self.tab(space) {
            Ok(tab) => tab,
            Err(why) => return why,
        };
        tab.ev.begin_slice();
        reply(tab.graph(&p, limit as usize).map(graph_dto))
    }

    /// The template as boxes and arrows: one box per type, one row per field,
    /// and one edge per connection between two of them. JSON, in the same reply
    /// shape as the rest.
    ///
    /// About the format rather than about the file. Nothing here is read from
    /// the document, no node is resolved, and the answer is the same for every
    /// file the same template opens. `space` picks which template, since an
    /// unpacked stream is read by one of its own.
    pub fn template_diagram(&mut self, space: u32) -> String {
        if let Err(why) = self.go(space) {
            return why;
        }
        let sh = self.sm();
        match (&sh.eval, &sh.read_as) {
            (Some(e), _) => reply(Ok(diagram_dto(qubero_core::eval::diagram(e.template())))),
            (None, Some(t)) => reply(Ok(diagram_dto(qubero_core::eval::diagram(t)))),
            (None, None) => reply::<DiagramDto>(Err(EvalError::Failed("no template".into()))),
        }
    }

    /// The open file's nodes counted against the diagram's boxes: how many of
    /// each the file holds, which rows they stood on, and the path to the first
    /// of each. JSON, in the same reply shape as the rest.
    ///
    /// One go of a count kept between calls, so each call carries on from the
    /// last and answers with everything counted so far. `limit` caps the nodes
    /// walked in all, and raising it carries a capped count on. The node's
    /// `state` says whether to ask again; bytes the count is waiting on come
    /// back as `wanted`, so they are fetched and the change they make asks
    /// again. An edit or a new template throws the count away, and the next
    /// call starts it over.
    pub fn diagram_census(&mut self, space: u32, limit: u32) -> String {
        // Taken out of the sheet for the length of the call, the way the kind
        // walk is: see `kind_totals_step`.
        if let Err(why) = self.tab(space) {
            return why;
        }
        let sh = self.sm();
        let len = sh.doc.len_bits();
        let mut census = sh.census.take();
        let mut tab = match self.tab(space) {
            Ok(tab) => tab,
            Err(why) => return why,
        };
        if !matches!(&census, Some(w) if w.file_bits() == len) {
            census = Some(tab.census_walk(len));
        }
        let walk = census.as_mut().expect("just built");
        tab.ev.begin_slice();
        let out = tab.census_step(walk, limit as usize);
        let reached = (tab.ev.reached_bits() / 8) as f64;
        let wanted = wanted(tab.ev);
        self.sm().census = census;
        reply_with(out.map(census_dto), reached, wanted)
    }

    /// The relationships behind the shape of the field at `path`, written out:
    /// the expression as the template holds it, the same with every field's
    /// value in its place, and what it comes to. JSON, in the same reply shape
    /// as the rest. Empty for a field the template placed and sized outright,
    /// and for one whose expression has no reading in that notation.
    pub fn relations(&mut self, space: u32, path: &[u32]) -> String {
        let p: Vec<usize> = path.iter().map(|&x| x as usize).collect();
        let mut tab = match self.tab(space) {
            Ok(tab) => tab,
            Err(why) => return why,
        };
        tab.ev.begin_slice();
        reply(tab.relations(&p).map(|v| {
            v.into_iter()
                .map(|r| RelationDto {
                    role: r.role.as_str(),
                    written: r.written,
                    template: r.template,
                    substituted: r.substituted,
                    result: r.result,
                })
                .collect::<Vec<_>>()
        }))
    }

    /// What tool produced this file, according to a bundle of Detect It Easy
    /// signature rules. `rules` is the bundle text, `head` the file's first
    /// bytes. Returns JSON, an array that is usually empty.
    ///
    /// Rules asking where the file starts running, what its sections are
    /// called or where its overlay begins are answered from the bytes here.
    /// A question the file cannot answer means the rule does not match, rather
    /// than being answered from somewhere else.
    ///
    /// One answer is not from the database: a DOS BASIC program names the
    /// runtime it was built against in its own loader stub, which no rule in
    /// the database can match on because the stub sits at a different place in
    /// every program. It is credited to this editor rather than to a rule file.
    pub fn detect_tools(&self, rules: &str, head: &[u8]) -> String {
        let sh = &self.sheets[0];
        let db = diescript::parse_bundle(rules);
        // What the file says about itself: where it starts running, what its
        // sections are called, where the overlay begins. Worked out once.
        let facts = diescript::Facts::of(head, sh.doc.len_bytes());
        let found: Vec<ToolDto> = diescript::detect(&db, head, &facts)
            .into_iter()
            .chain(dosbasic::detect(head, sh.doc.len_bytes()))
            .map(|d| ToolDto {
                category: d.category,
                name: d.name,
                version: d.version,
                options: d.options,
                source: d.source,
            })
            .collect();
        serde_json::to_string(&found).unwrap_or_else(|_| "[]".to_string())
    }

    /// JSON: {status:"ok",node} | {status:"pending",chunks} | {status:"error",message}
    pub fn template_node(&mut self, space: u32, path: &[u32]) -> String {
        let p: Vec<usize> = path.iter().map(|&x| x as usize).collect();
        let mut tab = match self.tab(space) {
            Ok(tab) => tab,
            Err(why) => return why,
        };
        tab.ev.begin_slice();
        let r = tab.node(&p).map(dto);
        reply_with(r, (tab.ev.reached_bits() / 8) as f64, wanted(tab.ev))
    }

    /// Same envelope as `template_node`, with `node` being an array of children.
    pub fn template_children(&mut self, space: u32, path: &[u32], from: f64, to: f64) -> String {
        let p: Vec<usize> = path.iter().map(|&x| x as usize).collect();
        let mut tab = match self.tab(space) {
            Ok(tab) => tab,
            Err(why) => return why,
        };
        tab.ev.begin_slice();
        let r = tab.children(&p, from as u64, to as u64).map(|v| v.into_iter().map(dto).collect::<Vec<NodeDto>>());
        reply_with(r, (tab.ev.reached_bits() / 8) as f64, wanted(tab.ev))
    }

    /// Where the bits of the JPEG scan at `path` went, for the report's map of
    /// the picture: {status:"ok",node:{..}}, or a null node for a field that
    /// is not a JPEG scan or a scan that did not decode. Same envelope as
    /// `template_node`. The blocks come as parallel arrays, in coding order,
    /// since a photograph has hundreds of thousands of them.
    pub fn jpeg_scan(&mut self, space: u32, path: &[u32]) -> String {
        let p: Vec<usize> = path.iter().map(|&x| x as usize).collect();
        let mut tab = match self.tab(space) {
            Ok(tab) => tab,
            Err(why) => return why,
        };
        let r = tab.jpeg_scan(&p).map(|m| m.map(jpeg_scan_dto));
        reply_with(r, (tab.ev.reached_bits() / 8) as f64, wanted(tab.ev))
    }

    /// Block `index` of the JPEG scan at `path`, in coding order, with every
    /// code the decoder read for it and the 64 coefficients in rows:
    /// {status:"ok",node:{..}}, or a null node. Its `path` and `mcu_path` are
    /// the tab's, for going to the block in the listing.
    pub fn jpeg_block(&mut self, space: u32, path: &[u32], index: u32) -> String {
        let p: Vec<usize> = path.iter().map(|&x| x as usize).collect();
        let mut tab = match self.tab(space) {
            Ok(tab) => tab,
            Err(why) => return why,
        };
        let r = tab.jpeg_block(&p, index as usize).map(|b| b.map(jpeg_block_dto));
        reply_with(r, (tab.ev.reached_bits() / 8) as f64, wanted(tab.ev))
    }

    /// What the program in the tab over `space` was built to withstand, the
    /// way `checksec` reports it, with the bytes behind every answer:
    /// {status:"ok",node:{format,parts}}, or a null node when the template is
    /// not `elf`, `pe` or `macho`, or the tab is over a stream read where it
    /// was declared. Same envelope as `template_node`, so bytes the pass needs
    /// and has not got come back as pending with the chunks to fetch.
    ///
    /// `format` is `elf`, `pe` or `macho`. `parts` is one
    /// {name,path,rows} per program: `name` is empty for a file holding one,
    /// and the CPU for each slice of a universal Mach-O; `path` is the
    /// program's header node.
    ///
    /// A row is {key,state,verdict,count,total,items,more,evidence}. `key`
    /// names the protection (`relro`, `canary`, `nx`, `aslr`, ...) and `state`
    /// is what was found, in a word fixed per key. `verdict` is `good`,
    /// `partial`, `bad`, `info`, `unknown` or `n/a`. `count` and `total` are
    /// numbers or null. `items` and `more` are names: fortified calls and the
    /// unfortified ones, a search path's directories, the libraries needed.
    /// `evidence` is at most a dozen {path,offset_bits,size_bits,what,name},
    /// most direct first: `path` is the tab's node holding the bytes, empty
    /// where no field covers them; `what` is `segment`, `section`, `header`,
    /// `dynamic`, `symbol`, `string`, `note`, `directory`, `command` or
    /// `bytes`; `name` is the format's own spelling, `GNU_STACK` or
    /// `BIND_NOW` or `__stack_chk_fail`.
    ///
    /// Worked out in one go rather than in slices, as `elf_contents` is: the
    /// pass reads a few hundred fields and a symbol table's strings, and
    /// stopping part way would leave nothing to carry on from.
    pub fn hardening(&mut self, space: u32) -> String {
        if let Err(why) = self.go(space) {
            return why;
        }
        let sh = self.sm();
        if !matches!(sh.template.as_str(), "elf" | "pe" | "macho") || sh.view.is_some() {
            return reply::<Option<HardeningDto>>(Ok(None));
        }
        let mut tab = match self.tab(space) {
            Ok(tab) => tab,
            Err(why) => return why,
        };
        tab.ev.set_slice(None);
        let r = tab.hardening().map(|h| h.map(hardening_dto));
        tab.ev.set_slice(Some(WORK_SLICE));
        reply_with(r, (tab.ev.reached_bits() / 8) as f64, wanted(tab.ev))
    }

    /// The elements of the folded run at `path` whose bits overlap
    /// `from_bit..to_bit`, at most `max` of them, for the value table beside
    /// the bytes: {status:"ok",node:[cell,..]}. Same envelope as
    /// `template_children`. `path` is what a span with a count carries, or a
    /// block of a decoded stream's trace.
    pub fn run_cells(&mut self, space: u32, path: &[u32], from_bit: f64, to_bit: f64, max: u32) -> String {
        let p: Vec<usize> = path.iter().map(|&x| x as usize).collect();
        let mut tab = match self.tab(space) {
            Ok(tab) => tab,
            Err(why) => return why,
        };
        tab.ev.begin_slice();
        let r = tab
            .run_cells(&p, from_bit as u64, to_bit as u64, max as usize)
            .map(|v| v.into_iter().map(cell_dto).collect::<Vec<CellDto>>());
        reply_with(r, (tab.ev.reached_bits() / 8) as f64, wanted(tab.ev))
    }

    /// Whole text of a text field, decoded in its own encoding:
    /// {status:"ok",node:{text,truncated}}.
    pub fn field_text(&mut self, space: u32, path: &[u32]) -> String {
        let p: Vec<usize> = path.iter().map(|&x| x as usize).collect();
        let mut tab = match self.tab(space) {
            Ok(tab) => tab,
            Err(why) => return why,
        };
        tab.ev.begin_slice();
        reply(tab.text_value(&p).map(|(text, truncated)| TextDto { text, truncated }))
    }

    /// The first `limit` bytes of a field, read in whatever address space the
    /// field is in: {status:"ok",node:{bytes:[..],truncated}}. Use this rather
    /// than `read_bits` at the node's offset, which is the file and is the
    /// wrong bytes for anything inside a decoded stream.
    pub fn field_bytes(&mut self, space: u32, path: &[u32], limit: u32) -> String {
        let p: Vec<usize> = path.iter().map(|&x| x as usize).collect();
        let mut tab = match self.tab(space) {
            Ok(tab) => tab,
            Err(why) => return why,
        };
        tab.ev.begin_slice();
        reply(tab.field_bytes(&p, u64::from(limit)).map(|(bytes, truncated)| BytesDto { bytes, truncated }))
    }

    /// Every field between two bit offsets, for the annotation column:
    /// {status:"ok",node:[span,..]}. `max` caps how many come back.
    pub fn spans(&mut self, space: u32, from_bit: f64, to_bit: f64, max: u32) -> String {
        let mut tab = match self.tab(space) {
            Ok(tab) => tab,
            Err(why) => return why,
        };
        tab.ev.begin_slice();
        let found = match tab.spans(from_bit as u64, to_bit as u64, max as usize) {
            Ok(v) => v,
            Err(err) => return reply::<Vec<SpanDto>>(Err(err)),
        };
        // Named through the tables of the program the sheet reads, which a
        // stream read where it was declared does not: its rows stay as the
        // template reads them.
        let named = self.name_instructions(found);
        reply(Ok(named))
    }

    /// Whether the space is read as an HDF5 file, whole or inside another
    /// format: {status:"ok",node:true}. What every HDF5 panel is offered on,
    /// rather than the template's name, since `mat` reads a level 5 file with
    /// no HDF5 in it as well as a level 7.3 file that is one. See
    /// `h5ad::holds_hdf5`.
    ///
    /// Pending while the few bytes near the front that decide it are still to
    /// come, so the host asks for them and asks again when they land.
    pub fn holds_hdf5(&mut self, space: u32) -> String {
        if let Err(why) = self.go(space) {
            return why;
        }
        let sh = self.sm();
        // A stream read where it was declared has no reading of its own for a
        // superblock to be the root of, and asked of the file's this would
        // answer for the file.
        let Some(e) = &mut sh.eval else { return reply(Ok(false)) };
        e.begin_slice();
        reply(qubero_core::formats::h5ad::holds_hdf5(e, &sh.doc))
    }

    /// What an HDF5 file holds, read in the file's own terms rather than the
    /// template's: {status:"ok",node:{objects,..}}. Empty for a file that holds
    /// no HDF5, since nothing else here has a group tree to walk.
    pub fn contents(&mut self, space: u32) -> String {
        if let Err(why) = self.go(space) {
            return why;
        }
        let sh = self.sm();
        let Some(e) = &mut sh.eval else {
            return reply::<ContentsDto>(Err(EvalError::Failed("no template".into())));
        };
        e.begin_slice();
        match qubero_core::formats::h5ad::holds_hdf5(e, &sh.doc) {
            Ok(true) => {}
            Ok(false) => {
                return reply(Ok(ContentsDto {
                    objects: Vec::new(),
                    total: 0.0,
                    anndata: false,
                    encoding: String::new(),
                    rows: 0.0,
                    columns: 0.0,
                }))
            }
            Err(err) => return reply::<ContentsDto>(Err(err)),
        }
        let found = match qubero_core::formats::h5ad::contents(e, &sh.doc) {
            Ok(c) => c,
            Err(err) => return reply::<ContentsDto>(Err(err)),
        };
        reply(Ok(ContentsDto {
            total: found.total as f64,
            anndata: found.anndata,
            encoding: found.encoding,
            rows: found.rows as f64,
            columns: found.columns as f64,
            objects: found
                .objects
                .into_iter()
                .map(|o| {
                    use qubero_core::formats::h5ad::Storage;
                    // The words a reader sees are the host's business; what
                    // crosses is which of the three ways the bytes are kept,
                    // how many there are, and the chunk it is kept in.
                    let (storage, bytes, chunk_dims, filters) = match o.storage {
                        Storage::None => ("", 0.0, Vec::new(), Vec::new()),
                        Storage::Contiguous(n) => ("contiguous", n as f64, Vec::new(), Vec::new()),
                        Storage::Compact(n) => ("compact", n as f64, Vec::new(), Vec::new()),
                        Storage::Chunked { dims, filters } => {
                            ("chunked", 0.0, dims.into_iter().map(|d| d as f64).collect(), filters)
                        }
                    };
                    ContentDto {
                        path: o.path,
                        name: o.name,
                        group: o.group,
                        encoding: o.encoding,
                        shape: o.shape.into_iter().map(|d| d as f64).collect(),
                        element: o.element,
                        storage,
                        bytes,
                        chunk_dims,
                        filters,
                        address: o.address as f64,
                    }
                })
                .collect(),
        }))
    }

    /// The HDF5 B-tree the field at `path` belongs to, of either version,
    /// walked into the shape it has in the file:
    /// {status:"ok",node:{job,version,nodes,..}}, or a null node where the file
    /// has no tree to answer with.
    ///
    /// `path` is where the cursor is, which is usually not a node of a tree.
    /// The core works out which tree that means: the one the cursor is inside,
    /// else the one the object header it is inside names, else the root
    /// group's, else the first tree under the root group. `limit` caps the
    /// nodes walked, and the answer says how many children it left out.
    pub fn btree(&mut self, space: u32, path: &[u32], limit: u32) -> String {
        if let Err(why) = self.go(space) {
            return why;
        }
        let sh = self.sm();
        let p: Vec<usize> = path.iter().map(|&x| x as usize).collect();
        let Some(e) = &mut sh.eval else {
            return reply::<Option<TreeDto>>(Err(EvalError::Failed("no template".into())));
        };
        e.begin_slice();
        match qubero_core::formats::h5ad::holds_hdf5(e, &sh.doc) {
            Ok(true) => {}
            Ok(false) => return reply(Ok(None::<TreeDto>)),
            Err(err) => return reply::<Option<TreeDto>>(Err(err)),
        }
        let found = match qubero_core::formats::hdf5_tree::tree(e, &sh.doc, &p, limit as usize) {
            Ok(t) => t,
            Err(err) => return reply::<Option<TreeDto>>(Err(err)),
        };
        reply(Ok(found.map(|t| {
            use qubero_core::formats::hdf5_tree::{Job, Kind, Records, NO_PARENT};
            TreeDto {
                job: match t.job {
                    Job::Group => "group",
                    Job::Chunk => "chunk",
                    Job::Other => "other",
                },
                version: f64::from(t.version),
                record_type: f64::from(t.record_type),
                record_type_name: t.record_type_name,
                records: match t.records {
                    Records::Read => "read",
                    Records::Unread => "unread",
                    Records::Unknown => "unknown",
                },
                records_total: t.records_total as f64,
                omitted: t.omitted as f64,
                coords: t.coords as f64,
                coords_pad: t.coords_pad,
                nodes: t
                    .nodes
                    .into_iter()
                    .map(|n| TreeNodeDto {
                        path: n.path,
                        parent: if n.parent == NO_PARENT { -1.0 } else { n.parent as f64 },
                        kind: match n.kind {
                            Kind::Index => "index",
                            Kind::LinkTable => "links",
                            Kind::Leaf => "leaf",
                        },
                        sign: n.sign,
                        address: n.address as f64,
                        size_bits: n.size_bits as f64,
                        level: n.level as f64,
                        depth: n.depth as f64,
                        entries: n.entries as f64,
                        first_key: n.first_key,
                        last_key: n.last_key,
                        truncated: n.truncated,
                        first_entry_bits: n.first_entry_bits as f64,
                        entry_bits: n.entry_bits as f64,
                    })
                    .collect(),
            }
        })))
    }

    /// Named ELF sections and a bounded prefix of its symbols. The semantic
    /// pass is cached because resolving names crosses several linked tables.
    pub fn elf_contents(&mut self, space: u32, symbol_limit: u32) -> String {
        if let Err(why) = self.go(space) {
            return why;
        }
        let sh = self.sm();
        // A stream read where it was declared carries the file's template name
        // and is not a program: the tables are the file's.
        if (sh.template != "elf" && sh.template != "bpf") || sh.view.is_some() {
            return reply::<ElfContentsDto>(Err(EvalError::Failed("not an ELF template".into())));
        }
        let Some(e) = &mut sh.eval else {
            return reply::<ElfContentsDto>(Err(EvalError::Failed("no template".into())));
        };
        let need_symbols = symbol_limit > 0;
        if sh.bpf.is_none() || (need_symbols && !sh.bpf_complete) {
            e.set_slice(None);
            let read = if need_symbols {
                formats::ElfProgram::read(e, &sh.doc)
            } else {
                formats::ElfProgram::read_sections(e, &sh.doc)
            };
            e.set_slice(Some(WORK_SLICE));
            match read {
                Ok(program) => {
                    sh.bpf = Some(program);
                    sh.bpf_complete = need_symbols;
                }
                Err(error) => return reply::<ElfContentsDto>(Err(error)),
            }
        }
        let Some(program) = &sh.bpf else {
            return reply::<ElfContentsDto>(Err(EvalError::Failed("could not resolve ELF tables".into())));
        };
        let sections = program.sections.iter().map(|section| ElfSectionDto {
            path: section.path.clone(),
            name: section.name.clone(),
            kind: section.kind as f64,
            address: section.addr as f64,
            offset: section.offset as f64,
            size: section.size as f64,
        }).collect();
        let symbols = program.symbols.iter().take(symbol_limit as usize).map(|symbol| ElfSymbolDto {
            path: symbol.path.clone(),
            source_bits: symbol.source_bits as f64,
            name: symbol.name.clone(),
            kind: symbol.kind as f64,
            section: symbol.section as f64,
            value: symbol.value as f64,
            size: symbol.size as f64,
        }).collect();
        reply(Ok(ElfContentsDto {
            sections,
            symbols,
            symbol_total: program.symbol_total as f64,
        }))
    }

    /// What a CERN ROOT file holds, read beside the template: the class
    /// descriptions out of its `StreamerInfo` record, and every tree with its
    /// branches, leaves and baskets.
    ///
    /// A channel of its own rather than a second use of `contents`, which is
    /// HDF5's: what crosses for an HDF5 object is a dataspace and a storage
    /// layout, and what crosses for a ROOT tree is a class description and a
    /// list of offsets nothing in the template placed. The two have no shape in
    /// common but the idea.
    pub fn root_contents(&mut self, space: u32) -> String {
        if let Err(why) = self.go(space) {
            return why;
        }
        let sh = self.sm();
        // The same for a record of a ROOT file opened as a tab, which is not a
        // ROOT file.
        if sh.template != "root" || sh.view.is_some() {
            return reply::<RootContentsDto>(Err(EvalError::Failed("not a ROOT template".into())));
        }
        let Some(e) = &mut sh.eval else {
            return reply::<RootContentsDto>(Err(EvalError::Failed("no template".into())));
        };
        // The walk reads whole records and decodes them; charging it the
        // listing's slice would stop it part way through a tree with nothing
        // to resume from. It is bounded instead by the limits in `root_tree`.
        e.set_slice(None);
        let found = qubero_core::formats::root_tree::contents(e, &sh.doc);
        e.set_slice(Some(WORK_SLICE));
        let found = match found {
            Ok(c) => c,
            Err(err) => return reply::<RootContentsDto>(Err(err)),
        };
        reply(Ok(RootContentsDto {
            schema_path: found.schema_path,
            trouble: found.trouble.unwrap_or_default(),
            tree_total: found.tree_total as f64,
            classes: found
                .classes
                .into_iter()
                .map(|c| RootClassDto {
                    name: c.name,
                    version: c.version as f64,
                    checksum: c.checksum as f64,
                    members: c
                        .members
                        .into_iter()
                        .map(|m| RootMemberDto {
                            name: m.name,
                            type_name: m.type_name,
                            code: m.code as f64,
                            size: m.size as f64,
                            dims: m.dims.into_iter().map(|d| d as f64).collect(),
                            base: m.base,
                            comment: m.comment,
                        })
                        .collect(),
                })
                .collect(),
            trees: found
                .trees
                .into_iter()
                .map(|t| RootTreeDto {
                    path: t.path,
                    name: t.name,
                    title: t.title,
                    entries: t.entries as f64,
                    address: t.at as f64,
                    branch_total: t.branch_total as f64,
                    trouble: t.trouble.unwrap_or_default(),
                    branches: t
                        .branches
                        .into_iter()
                        .map(|b| {
                            use qubero_core::formats::root_tree::Reading;
                            // The words a reader sees are the host's business.
                            // What crosses is the shape of one value and, where
                            // there is no such shape, the sentence saying why.
                            let (width, per_entry, floating, unsigned, unread) = match b.reading {
                                Reading::Fixed { width, per_entry, floating, unsigned } => {
                                    (width as f64, per_entry as f64, floating, unsigned, String::new())
                                }
                                Reading::Not(why) => (0.0, 0.0, false, false, why),
                            };
                            RootBranchDto {
                                name: b.name,
                                title: b.title,
                                class: b.class,
                                depth: b.depth as f64,
                                entries: b.entries as f64,
                                total_bytes: b.total_bytes as f64,
                                zip_bytes: b.zip_bytes as f64,
                                basket_total: b.basket_total as f64,
                                width,
                                per_entry,
                                floating,
                                unsigned,
                                unread,
                                leaves: b
                                    .leaves
                                    .into_iter()
                                    .map(|l| RootLeafDto {
                                        name: l.name,
                                        class: l.class,
                                        len: l.len as f64,
                                        width: l.width as f64,
                                        unsigned: l.unsigned,
                                        counted_by: l.counted_by,
                                    })
                                    .collect(),
                                baskets: b
                                    .baskets
                                    .into_iter()
                                    .map(|k| RootBasketDto {
                                        address: k.at as f64,
                                        bytes: k.bytes as f64,
                                        first_entry: k.first_entry as f64,
                                        entries: k.entries as f64,
                                    })
                                    .collect(),
                            }
                        })
                        .collect(),
                })
                .collect(),
        }))
    }

    /// The primary ISO 9660 volume and its root-directory pointer.
    pub fn iso_volume(&mut self, space: u32) -> String {
        self.open_again(space);
        let sh = self.at(space);
        if sh.template != "iso9660" {
            return reply::<IsoVolumeDto>(Err(EvalError::Failed("not an ISO 9660 template".into())));
        }
        let mut descriptor = vec![0u8; 2048];
        let mut primary = None;
        let mut joliet = None;
        for i in 0..64usize {
            let at = (16 + i as u64) * 2048;
            if at + 2048 > sh.doc.len_bytes() {
                break;
            }
            let missing = sh.doc.read_bytes(at, &mut descriptor);
            if !missing.is_empty() {
                return reply::<IsoVolumeDto>(Err(EvalError::Pending(missing)));
            }
            if &descriptor[1..6] != b"CD001" {
                continue;
            }
            if descriptor[0] == 255 {
                break;
            }
            let is_primary = descriptor[0] == 1;
            let is_joliet = descriptor[0] == 2
                && matches!(&descriptor[88..91], b"%/@" | b"%/C" | b"%/E");
            if !is_primary && !is_joliet {
                continue;
            }
            let le16 = |p: usize| u16::from_le_bytes([descriptor[p], descriptor[p + 1]]) as u64;
            let le32 = |p: usize| u32::from_le_bytes(descriptor[p..p + 4].try_into().unwrap()) as u64;
            let volume = if is_joliet {
                let decoded: String = descriptor[40..72]
                    .chunks_exact(2)
                    .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
                    .filter_map(|c| char::from_u32(c as u32))
                    .collect();
                decoded.trim_end_matches([' ', '\0']).to_string()
            } else {
                String::from_utf8_lossy(&descriptor[40..72]).trim_end_matches([' ', '\0']).to_string()
            };
            let root = 156usize;
            let found = IsoVolumeDto {
                descriptor_path: vec![1, i, 3],
                volume,
                joliet: is_joliet,
                block_size: le16(128) as f64,
                blocks: le32(80) as f64,
                root_extent: le32(root + 2) as f64,
                root_size: le32(root + 10) as f64,
                root_source_bits: ((at + root as u64) * 8) as f64,
            };
            if is_joliet {
                joliet = Some(found);
            } else {
                primary = Some(found);
            }
        }
        match joliet.or(primary) {
            Some(volume) => reply(Ok(volume)),
            None => reply::<IsoVolumeDto>(Err(EvalError::Failed("primary volume descriptor not found".into()))),
        }
    }

    /// One ISO directory, bounded for display but counted in full. Child
    /// directories are read only when their logical row is opened.
    pub fn iso_directory(&mut self, space: u32, extent: f64, size: f64, block_size: f64, limit: u32, joliet: bool) -> String {
        self.open_again(space);
        let sh = self.at(space);
        if sh.template != "iso9660" {
            return reply::<IsoDirectoryDto>(Err(EvalError::Failed("not an ISO 9660 template".into())));
        }
        let block = block_size as u64;
        let len = size as u64;
        let at = (extent as u64).saturating_mul(block);
        if block == 0 || len > 32 * 1024 * 1024 || at.saturating_add(len) > sh.doc.len_bytes() {
            return reply::<IsoDirectoryDto>(Err(EvalError::Failed("invalid or unusually large ISO directory".into())));
        }
        let mut bytes = vec![0u8; len as usize];
        let missing = sh.doc.read_bytes(at, &mut bytes);
        if !missing.is_empty() {
            return reply::<IsoDirectoryDto>(Err(EvalError::Pending(missing)));
        }
        let mut entries: Vec<IsoEntryDto> = Vec::new();
        let mut total = 0usize;
        let mut last_name = String::new();
        let mut last_multi = false;
        let mut pos = 0usize;
        while pos < bytes.len() {
            let record_len = bytes[pos] as usize;
            if record_len == 0 {
                let next = ((pos as u64 / block) + 1) * block;
                pos = next as usize;
                continue;
            }
            if record_len < 34 || pos + record_len > bytes.len() {
                break;
            }
            let name_len = bytes[pos + 32] as usize;
            if pos + 33 + name_len > pos + record_len {
                break;
            }
            let raw = &bytes[pos + 33..pos + 33 + name_len];
            if raw != [0] && raw != [1] {
                let mut name: String = if joliet {
                    raw.chunks_exact(2)
                        .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
                        .filter_map(|c| char::from_u32(c as u32))
                        .collect()
                } else {
                    String::from_utf8_lossy(raw).into_owned()
                };
                // Rock Ridge NM carries the POSIX name in the System Use
                // area. It wins over the restricted ISO identifier.
                let mut system = pos + 33 + name_len + usize::from(name_len % 2 == 0);
                let end = pos + record_len;
                let mut rock_ridge = String::new();
                while system + 4 <= end {
                    let field_len = bytes[system + 2] as usize;
                    if field_len < 4 || system + field_len > end {
                        break;
                    }
                    if &bytes[system..system + 2] == b"NM" && field_len >= 5 {
                        rock_ridge.push_str(&String::from_utf8_lossy(&bytes[system + 5..system + field_len]));
                    }
                    system += field_len;
                }
                if !rock_ridge.is_empty() {
                    name = rock_ridge;
                } else {
                    if let Some(version) = name.rfind(';') {
                        if name[version + 1..].chars().all(|c| c.is_ascii_digit()) {
                            name.truncate(version);
                        }
                    }
                    if name.ends_with('.') {
                        name.pop();
                    }
                }
                let multi = bytes[pos + 25] & 0x80 != 0;
                let continuation = last_multi && name == last_name;
                let le32 = |p: usize| u32::from_le_bytes(bytes[p..p + 4].try_into().unwrap()) as u64;
                let part_size = le32(pos + 10) as f64;
                if continuation {
                    if let Some(previous) = entries.last_mut() {
                        if previous.name == name {
                            previous.size += part_size;
                            previous.extents += 1.0;
                            previous.multi_extent = multi;
                        }
                    }
                } else {
                    total += 1;
                }
                if !continuation && entries.len() < limit as usize {
                    entries.push(IsoEntryDto {
                        name: name.clone(),
                        directory: bytes[pos + 25] & 2 != 0,
                        extent: le32(pos + 2) as f64,
                        size: part_size,
                        source_bits: ((at + pos as u64) * 8) as f64,
                        extents: 1.0,
                        multi_extent: multi,
                    });
                }
                last_name = name;
                last_multi = multi;
            }
            pos += record_len;
        }
        reply(Ok(IsoDirectoryDto { entries, total: total as f64 }))
    }

    /// Rewrite instruction rows through a disassembler, so a call names the
    /// function it calls. Anything that does not work out keeps the row the
    /// template already produced: a name is an improvement on a number, not a
    /// requirement for reading the file.
    fn name_instructions(&mut self, found: Vec<Span>) -> Vec<SpanDto> {
        let sh = self.sm();
        match sh.template.as_str() {
            "wasm" => self.name_wasm(found),
            "bpf" => self.name_bpf(found),
            "elf" => self.name_machine(found),
            "ne" => self.name_ne(found),
            _ => found.into_iter().map(span_dto).collect(),
        }
    }

    fn name_wasm(&mut self, found: Vec<Span>) -> Vec<SpanDto> {
        let sh = self.sm();
        if !found.iter().any(|s| s.type_name == "Instr") {
            return found.into_iter().map(span_dto).collect();
        }
        let Some(e) = &mut sh.eval else { return found.into_iter().map(span_dto).collect() };
        if sh.disasm.is_none() {
            // The module may not have streamed in far enough yet, in which case
            // this is worth trying again on the next screenful.
            sh.disasm = formats::WasmModule::read(e, &sh.doc).ok();
        }
        let Some(m) = &sh.disasm else { return found.into_iter().map(span_dto).collect() };
        found
            .into_iter()
            .map(|s| {
                let named = if s.type_name == "Instr" { m.instruction_line(e, &sh.doc, &s.path).ok() } else { None };
                let mut dto = span_dto(s);
                if let Some(line) = named {
                    dto.line = Some(line);
                }
                dto
            })
            .collect()
    }

    /// The same for eBPF, where what a line needs is in the object's tables
    /// rather than in the instruction.
    fn name_bpf(&mut self, found: Vec<Span>) -> Vec<SpanDto> {
        let sh = self.sm();
        if !found.iter().any(|s| s.type_name == "BpfInsn") {
            return found.into_iter().map(span_dto).collect();
        }
        let Some(e) = &mut sh.eval else { return found.into_iter().map(span_dto).collect() };
        if !sh.bpf_complete {
            sh.bpf = formats::ElfProgram::read(e, &sh.doc).ok();
            sh.bpf_complete = sh.bpf.is_some();
        }
        let Some(p) = &sh.bpf else { return found.into_iter().map(span_dto).collect() };
        found
            .into_iter()
            .map(|s| {
                let named = if s.type_name == "BpfInsn" { p.instruction_line(e, &sh.doc, &s.path).ok() } else { None };
                let mut dto = span_dto(s);
                if let Some(line) = named {
                    dto.line = Some(line);
                }
                dto
            })
            .collect()
    }

    /// Rewrite machine instructions through the symbol table, so a call says
    /// the name of what it calls. A file with no symbols keeps every row it
    /// had, which is most of what is on a disk: a program is usually stripped
    /// before it ships.
    fn name_machine(&mut self, found: Vec<Span>) -> Vec<SpanDto> {
        let sh = self.sm();
        if !found.iter().any(|s| is_machine(&s.type_name)) {
            return found.into_iter().map(span_dto).collect();
        }
        let Some(e) = &mut sh.eval else { return found.into_iter().map(span_dto).collect() };
        if !sh.bpf_complete {
            // Reading the symbol table of a whole program is more than one
            // screenful of work, and stopping halfway would mean starting
            // again on every screenful after it. So this one runs to an
            // answer, and what it costs is paid once.
            e.set_slice(None);
            sh.bpf = formats::ElfProgram::read(e, &sh.doc).ok();
            sh.bpf_complete = sh.bpf.is_some();
            e.set_slice(Some(WORK_SLICE));
        }
        let Some(p) = &sh.bpf else { return found.into_iter().map(span_dto).collect() };
        found
            .into_iter()
            .map(|s| {
                let named = match is_machine(&s.type_name) {
                    true => p.machine_line(e, &sh.doc, &s.path).ok().flatten(),
                    false => None,
                };
                let mut dto = span_dto(s);
                if let Some(line) = named {
                    dto.line = Some(line);
                }
                dto
            })
            .collect()
    }

    /// Rewrite the instructions of a 16-bit Windows program through its
    /// relocations, so a call into another module says which function of it
    /// the loader will point the call at.
    fn name_ne(&mut self, found: Vec<Span>) -> Vec<SpanDto> {
        let sh = self.sm();
        if !found.iter().any(|s| is_machine(&s.type_name)) {
            return found.into_iter().map(span_dto).collect();
        }
        let Some(e) = &mut sh.eval else { return found.into_iter().map(span_dto).collect() };
        if sh.ne.is_none() {
            // The same as for a program's symbols: read once, in full.
            e.set_slice(None);
            sh.ne = formats::NeProgram::read(e, &sh.doc).ok();
            e.set_slice(Some(WORK_SLICE));
        }
        let Some(p) = &sh.ne else { return found.into_iter().map(span_dto).collect() };
        found
            .into_iter()
            .map(|s| {
                let named = match (is_machine(&s.type_name), formats::NeProgram::segment_of(&s.path)) {
                    (true, Some(segment)) => p.instruction_line(e, &sh.doc, &s.path, segment).ok().flatten(),
                    _ => None,
                };
                let mut dto = span_dto(s);
                if let Some(line) = named {
                    dto.line = Some(line);
                }
                dto
            })
            .collect()
    }

    /// Path of the deepest field covering `bit`, as {status:"ok",node:[..]}.
    /// Its ancestors are the prefixes of that path.
    pub fn locate(&mut self, space: u32, bit: f64) -> String {
        let mut tab = match self.tab(space) {
            Ok(tab) => tab,
            Err(why) => return why,
        };
        tab.ev.begin_slice();
        reply(tab.locate(bit as u64))
    }

    /// Write `text` into the field at `path`, encoded as that field's type.
    /// Same envelope as `template_node`; on success `node` is the bit range written.
    pub fn write_node(&mut self, space: u32, path: &[u32], text: &str) -> String {
        // A byte of an unpacked stream is a function of every compressed byte
        // before it, so there is nowhere to put a change to one. The interface
        // says so in its own words before a write gets here; this is the same
        // refusal one layer out, in the same words the evaluator would use if
        // it were reached, rather than five lowercase ones of its own.
        if space != 0 {
            return reply::<WriteDto>(Err(EvalError::Failed(qubero_core::encode::UNPACKED_MSG.into())));
        }
        if let Err(why) = self.go(space) {
            return why;
        }
        let sh = self.sm();
        let p: Vec<usize> = path.iter().map(|&x| x as usize).collect();
        let prepared = match &mut sh.eval {
            None => return reply::<WriteDto>(Err(EvalError::Failed("no template".into()))),
            Some(e) => {
                // An edit is not something to do by halves, so this one runs to
                // an answer however long it takes.
                e.set_slice(None);
                let prepared = e.prepare_write(&sh.doc, &p, text);
                e.set_slice(Some(WORK_SLICE));
                prepared
            }
        };
        match prepared {
            Ok(w) => {
                let resized = w.n_bits != w.old_bits;
                sh.doc.replace_bits(w.offset_bits, &w.data, w.n_bits, w.old_bits);
                // Bytes that moved are bytes the template read at an offset
                // they no longer sit at, so nothing worked out before still
                // stands. An in-place write keeps everything up to it.
                if resized {
                    self.changed();
                } else {
                    self.changed_at(w.offset_bits);
                }
                reply(Ok(WriteDto { offset_bits: w.offset_bits as f64, size_bits: w.n_bits as f64 }))
            }
            Err(e) => reply::<WriteDto>(Err(e)),
        }
    }

    // ----- searching -----

    /// Whether the search bar holds something that can be searched for, and
    /// what is wrong with it when not. Empty string means it is fine.
    /// `typing` suppresses the one complaint that is not a mistake yet: a hex
    /// byte with a single digit so far, which every valid needle passes
    /// through on its way to being typed.
    pub fn check_needle(&self, kind: &str, text: &str, typing: bool) -> String {
        match needle(kind, text, false) {
            Ok(_) => String::new(),
            Err(why) if typing && why == HEX_HALF_A_BYTE => String::new(),
            Err(why) => why,
        }
    }

    /// One step of a search, from byte `from`. The reply is the same tri-state
    /// as the rest: a step, the chunks it wants, or what is wrong with it.
    pub fn search_step(&mut self, space: u32, kind: &str, text: &str, fold: bool, backward: bool, from: f64) -> String {
        if let Err(why) = self.go(space) {
            return why;
        }
        let sh = self.sm();
        let n = match needle(kind, text, fold) {
            Ok(n) => n,
            Err(why) => return reply::<StepDto>(Err(EvalError::Failed(why))),
        };
        if n.is_empty() {
            return reply(Ok(StepDto::End));
        }
        let s = if backward { Search::backward(n) } else { Search::forward(n) };
        reply(match s.step(&sh.doc, from as u64) {
            Step::Found { at, len } => Ok(StepDto::Found { at: at as f64, len: len as f64 }),
            Step::More { resume } => Ok(StepDto::More { resume: resume as f64 }),
            Step::End => Ok(StepDto::End),
            Step::Pending(m) => Err(EvalError::Pending(m)),
        })
    }

    /// Put `with` where a match was found. The caller carries on from the end
    /// of what was written: a replacement of a different length has moved
    /// every byte behind it.
    pub fn replace_at(&mut self, at: f64, len: f64, with: &[u8]) {
        self.live = 0;
        let sh = self.sm();
        search::replace(&mut sh.doc, at as u64, len as u64, with);
        self.changed();
    }

    /// Fold the edits that follow into one undo step.
    pub fn begin_batch(&mut self) {
        self.live = 0;
        let sh = self.sm();
        sh.doc.begin_batch();
    }

    pub fn end_batch(&mut self) {
        self.live = 0;
        let sh = self.sm();
        sh.doc.end_batch();
    }

    pub fn feed_chunk(&mut self, chunk: f64, data: &[u8]) {
        self.live = 0;
        let sh = self.sm();
        sh.doc.source_mut().insert(chunk as u64, data.into());
    }

    pub fn has_chunk(&mut self, space: u32, chunk: f64) -> bool {
        self.open_again(space);
        let sh = self.at(space);
        sh.doc.source().has(chunk as u64)
    }

    pub fn chunk_size(&self) -> u32 {
        let sh = &self.sheets[0];
        sh.doc.source().chunk_size() as u32
    }

    pub fn len_bytes(&mut self, space: u32) -> f64 {
        self.open_again(space);
        let sh = self.at(space);
        sh.doc.len_bytes() as f64
    }

    pub fn len_bits(&mut self, space: u32) -> f64 {
        self.open_again(space);
        let sh = self.at(space);
        sh.doc.len_bits() as f64
    }

    /// Fill `out` with document bytes from `at`. Returns the chunk indices that
    /// were not loaded (those bytes are zero). Empty list means the read is complete.
    ///
    /// A tab whose stream has to be opened again first, and whose reopen waits
    /// on chunks of the file, answers with those: a tab's chunks are always
    /// the file's, and it is asked again once they are fed.
    pub fn read_bytes(&mut self, space: u32, at: f64, out: &mut [u8]) -> Vec<f64> {
        if let Err(waits) = self.ensure_open(space) {
            return chunks_of(waits);
        }
        let sh = self.at(space);
        sh.doc.read_bytes(at as u64, out).into_iter().map(|m| m.chunk as f64).collect()
    }

    pub fn read_bits(&mut self, space: u32, at_bit: f64, n: f64, out: &mut [u8]) -> Vec<f64> {
        if let Err(waits) = self.ensure_open(space) {
            return chunks_of(waits);
        }
        let sh = self.at(space);
        sh.doc.read_bits(at_bit as u64, n as u64, out).into_iter().map(|m| m.chunk as f64).collect()
    }

    pub fn overwrite_bytes(&mut self, at: f64, data: &[u8]) {
        self.changed_at(at as u64 * 8);
        self.live = 0;
        let sh = self.sm();
        sh.doc.overwrite_bytes(at as u64, data);
    }
    /// Overwrite that folds into the previous undo step.
    pub fn amend_overwrite_bytes(&mut self, at: f64, data: &[u8]) {
        self.changed_at(at as u64 * 8);
        self.live = 0;
        let sh = self.sm();
        sh.doc.amend_overwrite_bytes(at as u64, data);
    }
    pub fn insert_bytes(&mut self, at: f64, data: &[u8]) {
        self.changed();
        self.live = 0;
        let sh = self.sm();
        sh.doc.insert_bytes(at as u64, data);
    }
    pub fn delete_bytes(&mut self, at: f64, n: f64) {
        self.changed();
        self.live = 0;
        let sh = self.sm();
        sh.doc.delete_bytes(at as u64, n as u64);
    }
    pub fn overwrite_bits(&mut self, at_bit: f64, data: &[u8], n: f64) {
        self.changed_at(at_bit as u64);
        self.live = 0;
        let sh = self.sm();
        sh.doc.overwrite_bits(at_bit as u64, data, n as u64);
    }
    pub fn insert_bits(&mut self, at_bit: f64, data: &[u8], n: f64) {
        self.changed();
        self.live = 0;
        let sh = self.sm();
        sh.doc.insert_bits(at_bit as u64, data, n as u64);
    }
    pub fn delete_bits(&mut self, at_bit: f64, n: f64) {
        self.changed();
        self.live = 0;
        let sh = self.sm();
        sh.doc.delete_bits(at_bit as u64, n as u64);
    }

    /// Save plan as flat quads: kind (0 orig, 1 add, 2 materialize), doc_off, src_off, len.
    pub fn save_plan(&self) -> Vec<f64> {
        let sh = &self.sheets[0];
        sh.doc
            .save_plan()
            .iter()
            .flat_map(|r| {
                let k = match r.kind {
                    RunKind::Orig => 0.0,
                    RunKind::Add => 1.0,
                    RunKind::Materialize => 2.0,
                };
                [k, r.doc_off as f64, r.src_off as f64, r.len as f64]
            })
            .collect()
    }

    pub fn add_bytes(&self) -> Vec<u8> {
        let sh = &self.sheets[0];
        sh.doc.add_bytes().to_vec()
    }

    pub fn undo(&mut self) -> bool {
        self.changed();
        self.live = 0;
        let sh = self.sm();
        sh.doc.undo()
    }
    pub fn redo(&mut self) -> bool {
        self.changed();
        self.live = 0;
        let sh = self.sm();
        sh.doc.redo()
    }
    pub fn can_undo(&mut self, space: u32) -> bool {
        self.open_again(space);
        let sh = self.at(space);
        sh.doc.can_undo()
    }
    pub fn can_redo(&mut self, space: u32) -> bool {
        self.open_again(space);
        let sh = self.at(space);
        sh.doc.can_redo()
    }
    pub fn is_modified(&mut self, space: u32) -> bool {
        self.open_again(space);
        let sh = self.at(space);
        sh.doc.is_modified()
    }
    pub fn piece_count(&mut self, space: u32) -> u32 {
        self.open_again(space);
        let sh = self.at(space);
        sh.doc.piece_count() as u32
    }

    /// How the file reads as text: the encoding, whether that was a guess, and
    /// how many bytes of byte-order mark sit in front of the first line.
    /// `encoding` names one to use instead, or is empty to let the file decide.
    pub fn text_reading(&self, encoding: &str) -> String {
        let head = self.head(64);
        let r = named_encoding(encoding).map_or_else(|| textview::reading(&head), |s| textview::reading_as(s, &head));
        serde_json::to_string(&TextReadingDto {
            encoding: r.settled.name().to_string(),
            mark: r.mark as u32,
            guessed: r.guessed,
            unit: r.settled.unit() as u32,
        })
        .unwrap_or_default()
    }

    /// Lines starting at `from`, which must be where a line starts.
    pub fn text_window(&mut self, space: u32, encoding: &str, from: f64, want: u32) -> String {
        self.open_again(space);
        let sh = self.at(space);
        let head = self.head(64);
        let r = named_encoding(encoding).map_or_else(|| textview::reading(&head), |s| textview::reading_as(s, &head));
        let w = textview::window(&sh.doc, r, from as u64, want as usize);
        serde_json::to_string(&TextWindowDto {
            next: w.next as f64,
            missing: w.missing.iter().map(|m| m.chunk as f64).collect(),
            lines: w
                .lines
                .iter()
                .map(|l| TextLineDto {
                    at: l.at as f64,
                    len: l.len as f64,
                    ending: l.ending.name().to_string(),
                    text: l.text.clone(),
                    escapes: l.escapes.iter().flat_map(|(a, n)| [*a, *n]).collect(),
                    lossy: l.lossy,
                })
                .collect(),
        })
        .unwrap_or_default()
    }

    /// Where every line in `[from, to)` starts, `from` included, which must be
    /// where a line starts.
    ///
    /// Packed into one array of doubles rather than JSON: an index of a large
    /// file is hundreds of thousands of numbers, and spelling each of them out
    /// and parsing it back costs more than the scan does. The layout is
    /// `[next, lf, cr, crlf, missing count, ...missing chunks, ...starts]`.
    pub fn text_index(&mut self, space: u32, encoding: &str, from: f64, to: f64) -> Vec<f64> {
        self.open_again(space);
        let sh = self.at(space);
        let head = self.head(64);
        let r = named_encoding(encoding).map_or_else(|| textview::reading(&head), |s| textview::reading_as(s, &head));
        let idx = textview::text_index(&sh.doc, r, from as u64, to as u64);
        let mut out = Vec::with_capacity(5 + idx.missing.len() + idx.starts.len());
        out.push(idx.next as f64);
        out.push(idx.lf as f64);
        out.push(idx.cr as f64);
        out.push(idx.crlf as f64);
        out.push(idx.missing.len() as f64);
        out.extend(idx.missing.iter().map(|m| m.chunk as f64));
        out.extend(idx.starts.iter().map(|s| *s as f64));
        out
    }

    /// Where the line holding `at` starts, and where `lines` line starts back
    /// from there is. Both in one call, because scrolling text upwards wants
    /// the second and clicking in it wants the first.
    pub fn text_back(&mut self, space: u32, encoding: &str, at: f64, lines: u32) -> String {
        self.open_again(space);
        let sh = self.at(space);
        let head = self.head(64);
        let r = named_encoding(encoding).map_or_else(|| textview::reading(&head), |s| textview::reading_as(s, &head));
        let (start, missing) = textview::line_start(&sh.doc, r, at as u64);
        let (back, more) = textview::back(&sh.doc, r, at as u64, lines as usize);
        serde_json::to_string(&TextBackDto {
            start: start as f64,
            back: back as f64,
            missing: missing.iter().chain(more.iter()).map(|m| m.chunk as f64).collect(),
        })
        .unwrap_or_default()
    }

    /// The strings in the file from `from` onwards, for a file with no
    /// template and no encoding of its own.
    ///
    /// `encodings` names which readings to look for, comma separated, out of
    /// `ascii`, `utf16le` and `utf16be`; empty means all three. The reply is
    /// the usual tri-state: strings, or the chunks it needs before it can
    /// answer. `next` is where the caller carries on from, which is not the
    /// end of the last string when the scan stopped for want of hits.
    ///
    /// Where a template reads the space, each string also says whether it lies
    /// inside a run the template reads as numbers or instructions, or a packed
    /// stream. See `strings_inside`.
    pub fn strings_scan(&mut self, space: u32, from: f64, want: u32, min_chars: u32, encodings: &str) -> String {
        self.open_again(space);
        use qubero_core::stringscan;
        let sh = self.at(space);
        let pick = |name: &str| encodings.is_empty() || encodings.split(',').any(|e| e.trim() == name);
        let opts = stringscan::Opts {
            min_chars: min_chars as usize,
            ascii: pick("ascii"),
            utf16le: pick("utf16le"),
            utf16be: pick("utf16be"),
        };
        let mut s = stringscan::scan(&sh.doc, from as u64, want as usize, opts);
        let (inside, busy) = self.strings_inside(space, &mut s);
        serde_json::to_string(&StringsScanDto {
            next: s.next as f64,
            missing: s.missing.iter().map(|m| m.chunk as f64).collect(),
            busy,
            hits: s
                .hits
                .iter()
                .zip(inside)
                .map(|(h, inside)| StringHitDto {
                    at: h.at as f64,
                    len: h.len as f64,
                    enc: h.enc.name().to_string(),
                    chars: h.chars,
                    units: h.units,
                    text: h.text.clone(),
                    lone_surrogates: h.lone_surrogates,
                    terminator: h.term.map_or(0, |t| t.bytes()) as u32,
                    cut: h.cut,
                    inside,
                    prefix: h
                        .prefix
                        .iter()
                        .map(|p| StringPrefixDto {
                            kind: p.kind.name().to_string(),
                            at: p.at as f64,
                            bytes: p.raw.clone(),
                            value: p.value as f64,
                            counts: p.counts.name().to_string(),
                            with_terminator: p.with_terminator,
                            weak: p.weak,
                        })
                        .collect(),
                })
                .collect(),
        })
        .unwrap_or_default()
    }

    /// Which of the strings a scan found lie inside a run the template reads as
    /// numbers or instructions, or a packed stream, one answer per string.
    /// Nothing for any of them when the space has no template.
    ///
    /// A template that has to read more of the file before it can say, or that
    /// runs out of the time one call is allowed, answers for the strings in
    /// front of the one it stopped at, and the rest are left for the next call:
    /// the scan is cut there, the way a scan that found more strings than were
    /// asked for is. Where it stopped at the first, the scan gives nothing
    /// back, with the chunks to fetch or with `busy` set, and is asked again.
    fn strings_inside(&mut self, space: u32, s: &mut qubero_core::stringscan::Scan) -> (Vec<Option<StringInsideDto>>, bool) {
        if !s.missing.is_empty() || s.hits.is_empty() {
            return (Vec::new(), false);
        }
        let Ok(mut tab) = self.tab(space) else { return (vec![None; s.hits.len()], false) };
        tab.ev.begin_slice();
        let mut out: Vec<Option<StringInsideDto>> = Vec::with_capacity(s.hits.len());
        for h in &s.hits {
            match tab.read_as(h.at * 8, (h.at + h.len) * 8) {
                Ok(found) => out.push(found.map(|r| StringInsideDto {
                    kind: r.kind.name().to_string(),
                    path: r.path,
                    name: r.name,
                    what: r.what,
                    at: (r.offset_bits / 8) as f64,
                    len: r.size_bits.div_ceil(8) as f64,
                })),
                Err(e) if e.interrupted() => {
                    let busy = matches!(e, EvalError::Busy { .. });
                    match out.len() {
                        0 => {
                            s.next = s.hits.first().map_or(s.next, |h| h.at);
                            s.hits.clear();
                            s.missing = match e {
                                EvalError::Pending(m) => m,
                                _ => Vec::new(),
                            };
                        }
                        n => {
                            s.hits.truncate(n);
                            s.next = s.hits.last().map_or(s.next, |h| h.at + h.len);
                        }
                    }
                    return (out, busy && s.hits.is_empty());
                }
                // A template that cannot read these bytes says nothing about
                // them either way.
                Err(_) => out.push(None),
            }
        }
        (out, false)
    }

    /// What a selected run of bytes says, read every way text can be read.
    ///
    /// Only a run of whole bytes lying together: a selection made over the bits
    /// in binary mode is not characters, which is the same reason the panel's
    /// byte-reversed rows only appear for whole bytes. `first` names the
    /// encoding to put at the front, which is whatever the text view is
    /// reading the file in.
    pub fn selection_text(&mut self, space: u32, at_byte: f64, len: f64, first: &str, page_a: &str, page_b: &str) -> String {
        self.open_again(space);
        use qubero_core::text::CodePage;
        let sh = self.at(space);
        let want = (len as u64).min(SELECTION_TEXT_LIMIT) as usize;
        let mut buf = vec![0u8; want];
        let missing = sh.doc.read_bytes(at_byte as u64, &mut buf);
        if !missing.is_empty() {
            return String::new();
        }
        let a = CodePage::by_name(page_a).unwrap_or(CodePage::Latin1);
        let b = CodePage::by_name(page_b).unwrap_or(CodePage::Cp437);
        let r = qubero_core::text::readings(&buf, named_encoding(first), a, b);
        serde_json::to_string(&SelectionTextDto {
            readings: r
                .agreed
                .iter()
                .map(|(who, text)| ReadingDto {
                    encodings: who.iter().map(|s| s.name().to_string()).collect(),
                    text: text.clone(),
                })
                .collect(),
            refused: r.refused.iter().map(|s| s.name().to_string()).collect(),
            read: want as f64,
            all: want as u64 >= len as u64,
        })
        .unwrap_or_default()
    }

    /// The same bytes written as a string literal in one language: what to
    /// paste into a parser being written against the file. Empty while the
    /// bytes are still being fetched, and cut to the same length the readings
    /// are, so the row says the same run the rows above it do.
    pub fn selection_literal(&mut self, space: u32, at_byte: f64, len: f64, lang: &str) -> String {
        self.open_again(space);
        use qubero_core::text::Lang;
        let sh = self.at(space);
        let want = (len as u64).min(SELECTION_TEXT_LIMIT) as usize;
        let mut buf = vec![0u8; want];
        let missing = sh.doc.read_bytes(at_byte as u64, &mut buf);
        if !missing.is_empty() {
            return String::new();
        }
        qubero_core::text::literal(Lang::by_name(lang).unwrap_or(Lang::C), &buf)
    }

    /// The first `n` bytes, for questions that only the front of the file
    /// answers. A chunk that is not here yet reads as zeros, which settles the
    /// encoding as Latin-1 until it arrives.
    fn head(&self, n: u64) -> Vec<u8> {
        let sh = &self.sheets[0];
        let n = n.min(sh.doc.len_bytes());
        let mut out = vec![0u8; n as usize];
        sh.doc.read_bytes(0, &mut out);
        out
    }
}

/// The characters the hex view's text column writes, one per byte value, as a
/// string 256 characters long. U+FFFD stands where the column has nothing to
/// show for a byte and writes its stand-in instead.
///
/// A whole table rather than a character at a time: the column is redrawn a
/// few hundred cells at a go, and a boundary crossing per cell would put the
/// cost of the choice on every frame instead of on the choosing. The caller
/// holds the answer until the reader picks another column.
///
/// An empty string comes back for a name the core does not know, which is what
/// a caller with a stale choice saved gets, and is its cue to fall back.
#[wasm_bindgen]
pub fn glyph_column(name: &str) -> String {
    qubero_core::hexdump::glyphs::Glyphs::by_name(name).map(|g| g.column()).unwrap_or_default()
}

/// Text typed into the text view, as the bytes it is in the file's encoding.
///
/// The answer is a refusal or a run of bytes, never both and never a guess: an
/// encoding that has no room for a character says which character, since the
/// reader is owed the difference between "this file cannot hold that" and
/// "this file was read as the wrong thing". A file read as CP437 that is
/// really Latin-1 will take a character the other would refuse, and the
/// refusal is where that is found out.
#[wasm_bindgen]
pub fn text_encode(encoding: &str, settled: &str, text: &str) -> String {
    use qubero_core::text::{encode_settled, Settled};
    let enc = named_encoding(encoding).or_else(|| named_encoding(settled)).unwrap_or(Settled::Utf8);
    match encode_settled(enc, text) {
        Ok(bytes) => serde_json::to_string(&TextEncodeDto { bytes, refused: String::new() }),
        Err(c) => serde_json::to_string(&TextEncodeDto { bytes: Vec::new(), refused: c.to_string() }),
    }
    .unwrap_or_default()
}

/// How much of a selection is read as text. Long enough to hold a paragraph,
/// which is what someone selecting a stretch to read is after; past it the
/// rows say how much they are showing.
const SELECTION_TEXT_LIMIT: u64 = 4096;

/// An encoding named across the boundary, or nothing to let the file decide.
fn named_encoding(name: &str) -> Option<qubero_core::text::Settled> {
    use qubero_core::text::{CodePage, Settled};
    use qubero_core::Endian;
    Some(match name {
        "UTF-8" => Settled::Utf8,
        "ASCII" => Settled::Ascii,
        "UTF-16 LE" => Settled::Utf16(Endian::Little),
        "UTF-16 BE" => Settled::Utf16(Endian::Big),
        // Every single-byte page is named the same way it is shown.
        _ => Settled::SingleByte(CodePage::by_name(name)?),
    })
}

/// Read `text` as a hex dump and say what it holds. Returns "" when it is not
/// one, or when it is too big to read in one go.
#[wasm_bindgen]
pub fn dump_scan(text: &[u8]) -> String {
    if text.len() > hexdump::LIMIT {
        return String::new();
    }
    let Some(dump) = hexdump::read(text, 0) else { return String::new() };
    let Some((from, to)) = dump.span() else { return String::new() };
    let l = &dump.layout;
    let mut dto = DumpScanDto {
        tool: l.looks_like().unwrap_or("").to_string(),
        tier: match dump.tier() {
            hexdump::Tier::Regular => "regular",
            hexdump::Tier::Irregular => "irregular",
        }
        .to_string(),
        from: from as f64,
        to: to as f64,
        covered: dump.byte_count() as f64,
        address_base: l.address.as_ref().map_or("", |a| a.base.name()).to_string(),
        address_digits: l.address.as_ref().and_then(|a| a.digits).unwrap_or(0) as u32,
        bytes_per_line: l.bytes_per_line as u32,
        group: l.group as u32,
        upper: l.upper,
        reversed_groups: l.order == hexdump::layout::Order::ReversedInGroup,
        characters: l.text.as_ref().map_or(String::new(), |t| t.glyphs.name().to_string()),
        assumed: l.assumed.iter().map(|a| format!("{a:?}")).collect(),
        stated_length: -1.0,
        skipped_lines: dump.skipped.len() as u32,
        ..Default::default()
    };
    let mut at = from;
    for e in dump.extents() {
        dto.extents.push(e.at as f64);
        dto.extents.push(e.end() as f64);
        if e.at > at {
            dto.holes.push(at as f64);
            dto.holes.push(e.at as f64);
        }
        at = e.end().max(at);
    }
    for n in &dump.notes {
        match n {
            hexdump::Note::Named(s) => dto.names.push(s.clone()),
            hexdump::Note::Length(v) => dto.stated_length = *v as f64,
            hexdump::Note::Command(s) => dto.commands.push(s.clone()),
        }
    }
    dto.conflicts = dump
        .conflicts()
        .into_iter()
        .take(64)
        .map(|(at, wrote, digits)| DumpConflictDto { at: at as f64, wrote: wrote.to_string(), digits })
        .collect();
    serde_json::to_string(&dto).unwrap_or_default()
}

/// The bytes a dump describes, from its first address to the end of its last.
/// A stretch the dump skipped reads as zeros; `dump_scan` says where those
/// are, so a reader is never left to guess which zeros were written down.
#[wasm_bindgen]
pub fn dump_bytes(text: &[u8]) -> Vec<u8> {
    let Some(source) = hexdump::source::DumpSource::new(text.to_vec()) else { return Vec::new() };
    let mut out = vec![0u8; source.len_bytes() as usize];
    source.read_bytes(0, &mut out);
    out
}
