// What the open file is: the name the sources agree on, or the one with the
// best evidence when they do not, in the toolbar; every source's answer, with
// its evidence and its disagreements, in the dialog behind it.
//
// Four sources answer at different times: the template as soon as the first
// bytes are read, the file(1) rules once their module has loaded, the tool
// signatures and the format signatures after their fetches. Each one lands
// in `answers` and everything on screen is redrawn from that, so nothing on
// screen depends on which answer came last.

import { el } from "./dom.ts";
import type { Doc, Identification, TemplateChoice, TemplateNode, ToolMatch, SigVerdict } from "./doc.ts";
import { OWN_SOURCE } from "./doc.ts";
import { wikidataUrl, wikipediaUrl, type SigMatch } from "./signatures.ts";
import { decide, nameAndVersion, type Answers, type Candidate, type Source, type TemplateAnswer } from "./identity.ts";


const IDENTIFYING_MSG = "Identifying file type...";
const IDENTIFY_FAILED_MSG = "Couldn't check the file type";
const IDENTIFY_FAILED_TITLE = "The identification rules failed to download.";
const UNKNOWN_TYPE_MSG = "Unknown file type";
const INFO_LABEL = "File type details";
const DIALOG_TITLE = "File type";
const DIALOG_CLOSE = "Close";
const NO_MATCH_BODY = `No match in the rule database of the Unix "file" command.`;
const TEMPLATE_KEY = "Template";
const SIGNATURE_ONLY = "signature only";
const TEMPLATE_LABEL: Record<string, string> = {
  ar: "Unix archive",
  aseprite: "Aseprite",
  p8png: "PICO-8 cartridge",
  p64rom: "Picotron cartridge ROM",
  p64png: "Picotron cartridge",
  bdb: "Berkeley DB",
  braw: "Blackmagic RAW",
  arw: "Sony ARW",
  bardstale: "Bard's Tale I (DOS save)",
  cr2: "Canon CR2",
  c16: "16-bit I/Q samples",
  cab: "Windows cabinet",
  cdr: "CorelDRAW CDR",
  cdrom: "Raw CD-ROM image, 2352-byte sectors",
  iso9660: "ISO 9660 image, 2048-byte sectors",
  cue: "CD cue sheet",
  claudetheme: "Claude Code theme",
  cmx: "Corel Presentation Exchange CMX",
  cpio: "cpio archive (initramfs)",
  deb: "Debian package",
  dng: "Adobe DNG",
  dtb: "Device tree blob",
  gdbm: "GNU dbm",
  godot: "Godot binary resource/scene",
  godottext: "Godot text resource/scene",
  godotpck: "Godot PCK",
  grubenv: "GRUB environment block",
  hackrffw: "HackRF firmware",
  it: "Impulse Tracker IT",
  journal: "systemd journal",
  jxr: "JPEG XR",
  ico: "Windows icon/cursor",
  lnk: "Windows shortcut",
  mod: "ProTracker MOD",
  nef: "Nikon NEF",
  omezarr: "OME-Zarr metadata",
  pnm: "Netpbm image",
  orf: "Olympus ORF",
  pef: "Pentax PEF",
  pdb: "Microsoft program database (debug symbols)",
  pdb2: "Microsoft program database 2.00 (debug symbols)",
  portablepdb: "Portable PDB (.NET debug symbols)",
  psd: "Adobe Photoshop PSD/PSB",
  rpm: "RPM package",
  rw2: "Panasonic RW2",
  srw: "Samsung SRW",
  s3m: "Scream Tracker S3M",
  spp: "CCSDS space packets",
  xar: "xar archive (macOS .pkg)",
  xm: "FastTracker XM",
  zarrzip: "Zarr ZipStore",
  eps: "Encapsulated PostScript",
  elf: "ELF",
  com: "DOS .COM program",
  msdos: "MS-DOS program",
  ne: "16-bit Windows program",
  le: "LE/LX linear executable",
  macho: "Mach-O",
  bpf: "eBPF object",
  self: "SELF SQLite executable",
  thumbsdb: "Windows Thumbs.db",
  unityassets: "Unity serialized assets",
  utmp: "Login records",
  unitybundle: "Unity AssetBundle",
  zlib: "zlib stream",
  bzip2: "bzip2 stream",
  xz: "xz stream",
  zstd: "Zstandard stream",
  lz4: "LZ4 frame",
  lzip: "lzip stream",
  compress: "compress .Z stream",
  tar: "tar archive",
  "7z": "7-Zip archive",
  rar4: "RAR 4 archive",
  rar5: "RAR 5 archive",
  root: "CERN ROOT",
  gwf: "LIGO/Virgo GWF frame",
  fits: "FITS",
  npy: "NumPy array",
  netcdf: "NetCDF classic",
  grib: "GRIB weather data",
  mseed: "miniSEED seismic records",
  mseed3: "miniSEED 3 seismic records",
  sac: "SAC seismogram",
  nifti: "NIfTI image",
  analyze: "Analyze 7.5 image",
  segy: "SEG-Y seismic traces",
  cdf: "NASA CDF",
  hdf4: "HDF4",
  parquet: "Parquet",
  mat: "MATLAB MAT",
  // The rest were only ever seen in the template menu until every answer
  // started being listed in the dialog, where "pe file" reads as a typo.
  png: "PNG image",
  gif: "GIF image",
  jpeg: "JPEG image",
  tiff: "TIFF image",
  bmp: "Windows bitmap",
  pcx: "PCX image",
  tga: "Targa image",
  qoi: "QOI image",
  pi1: "Degas PI1 image",
  ilbm: "IFF ILBM image",
  swf: "Flash SWF",
  zip: "ZIP archive",
  gzip: "gzip stream",
  lha: "LHA archive",
  wasm: "WebAssembly module",
  mp4: "MP4 container",
  mkv: "Matroska container",
  ogg: "Ogg container",
  dv: "DV video",
  wav: "WAVE audio",
  w4v: "W4V audio",
  aiff: "AIFF audio",
  au: "Sun AU audio",
  midi: "MIDI",
  id3: "ID3 tag",
  sqlite: "SQLite database",
  pe: "Windows PE executable",
  coff: "COFF object",
  omf: "OMF object",
  gguf: "GGUF model",
  safetensors: "safetensors model",
  whisper: "Whisper database",
  json: "JSON",
  cbor: "CBOR",
  pdf: "PDF",
  hdf5: "HDF5",
  draco: "Draco mesh",
  uf2: "UF2 firmware image",
  nes: "NES ROM",
  wad: "Doom WAD",
  pak: "Quake PAK",
  vpk: "Valve VPK",
  mca: "Minecraft Anvil region",
  tap: "ZX Spectrum TAP",
  bencode: "Bencoded data (torrent)",
  pickle: "Python pickle",
  gitindex: "Git index",
  gitpackidx: "Git pack index",
  appledouble: "AppleDouble",
  applesingle: "AppleSingle",
  macbinary: "MacBinary",
  binhex: "BinHex",
  stuffit: "StuffIt archive",
  compactpro: "Compact Pro archive",
};

/** The titles of the bundled Kaitai formats, keyed by the `ksy:` name, filled
 *  in once from the template list. A title is the format's own `meta/title`,
 *  which a third of them do not carry; those are shown by their id. */
const KAITAI_TITLE = new Map<string, string>();

/** Remember what each bundled Kaitai format calls itself, so that a name
 *  reaching a label anywhere in the app reads as a format and not as an
 *  internal identifier. Called once, when the toolbar reads the list. */
export const rememberKaitaiTitles = (choices: readonly TemplateChoice[]): void => {
  for (const choice of choices) if (choice.source === "kaitai" && choice.title !== "") KAITAI_TITLE.set(choice.name, choice.title);
};

/** A template's human-facing name; internal names remain stable API values. */
export const templateLabel = (name: string): string =>
  TEMPLATE_LABEL[name] ?? KAITAI_TITLE.get(name) ?? (name.startsWith("ksy:") ? name.slice("ksy:".length) : name);

/** A useful identity when the full template recognises a format the rule
 * database does not. A label that is already plural is what the file is:
 * "Login records file" reads as a mistake, "Login records" does not. Only a
 * label of several words, since a template with no label of its own falls
 * back to its own name and half the model formats are called things like
 * `3ds`, which is a file and not a plural of anything. */
export const templateTypeName = (name: string): string => {
  if (name === "bardstale") return "The Bard's Tale I MS-DOS save game";
  const label = templateLabel(name);
  if (label.endsWith("s") && label.includes(" ")) return label;
  // A label whose last word already says what kind of thing it is needs no
  // "file" after it: "ZIP archive file" and "PNG image file" read as typos.
  const last = label.split(/[\s/]+/).pop()?.toLowerCase() ?? "";
  return NOUNS.has(last) ? label : `${label} file`;
};
const NOUNS: ReadonlySet<string> = new Set([
  "archive", "image", "audio", "video", "database", "executable", "container", "module", "stream", "model", "tag", "mesh",
  "index", "region", "package", "object", "firmware", "cartridge", "program", "resource", "frame", "map", "shortcut", "blob",
  "block", "table", "record", "log", "list", "sheet", "cabinet", "journal", "rom", "metadata", "packets", "profile", "wad",
  "pak", "vpk", "tap", "midi", "json", "cbor", "pdf", "hdf5", "hdf4", "fits", "elf", "mach-o", "symbols)", "db",
]);

/**
 * What the file is, in a sentence, read from the fields the template found.
 * Null for a format with nothing to add beyond its name, which is most of
 * them, and for a read that has not finished: neither is worth waiting for,
 * and the caller has the plain label either way.
 *
 * A template that has one wins over the signature rules. The rules see a theme
 * as "JSON text data", which is true and is the least useful true thing that
 * could be said about it; the template has read the file and can say which
 * theme it is.
 */
export const templateSentence = (doc: Doc, name: string): string | null =>
  name === "claudetheme" ? themeSentence(doc) : null;

/** `Claude Code theme "Ember", based on dark, 10 colours changed`. */
const themeSentence = (doc: Doc): string | null => {
  const root = doc.templateChildren([], 0, 3);
  if (root.status !== "ok") return null;
  const of = (key: string): TemplateNode | undefined => root.node.find((n) => n.name === key);
  const base = of("base")?.value;
  const overrides = of("overrides");
  if (base === undefined || overrides === undefined) return null;
  const themeName = of("name")?.value;
  const called = themeName === undefined || themeName === "" ? "Claude Code theme" : `Claude Code theme "${themeName}"`;
  const changed = overrides.child_count;
  return `${called}, based on ${base}, ${changed === 1 ? "1 colour" : `${changed} colours`} changed`;
};

/** Which template the Fields table is being read with: a built-in by name, the
 *  one field the identification rule's own signature makes, or neither. */
export type TemplateNote = { readonly kind: "builtin"; readonly name: string } | { readonly kind: "signature" } | null;

export const builtinTemplate = (name: string): TemplateNote => ({ kind: "builtin", name });
export const SIGNATURE_TEMPLATE: TemplateNote = { kind: "signature" };
const MATCHED_AGAINST = "Matched against the signature database of the Detect It Easy project.";
const READ_FROM_STUB = "Identified from the loader stub the compiler placed at the end of the program.";
const SOURCE_KEY = "Source";
/** Where the name came from, said in the dialog under it. */
const SOURCE_TEXT: Record<Source, (detail: string) => string> = {
  template: (label) => `Qubero's ${label} template, which read the file`,
  file: () => `The rules of the Unix "file" command`,
  tools: () => "The Detect It Easy signature rules",
  signature: (which) => which,
};
const SIGNATURE_SOURCE: Record<SigMatch["format"]["source"], string> = {
  wikidata: "A format signature listed on Wikidata",
  file: `A signature from the rules of the Unix "file" command`,
};
const SIGNATURE_WORD: Record<SigMatch["format"]["source"], string> = { wikidata: "Wikidata", file: "file rules" };
const OTHERS_HEADING = "Other answers:";
const DISAGREES = "names a different format";
const SIGNATURES_INTRO = "Signature matches:";
const WIKIPEDIA_LINK = "Wikipedia";
const SIGNATURES_CREDIT = (fetched: string): string => `Signature sources: ${fetched}.`;
/** The toolbar, for a file only a signature could name. */
const SIGNATURE_NAMED: Record<SigMatch["format"]["source"], (label: string) => string> = {
  wikidata: (label) => `${label} (signature listed on Wikidata)`,
  file: (label) => `${label} (file(1) rule)`,
};
/** How many rows the dialog shows before folding the rest away. */
const SIGNATURES_SHOWN = 5;
/** A signature so many formats share that listing them says nothing. */
const SIGNATURES_CROWD = 4;
const SIGNATURES_MORE = (n: number): string => `${n} more formats`;
const SIGNATURES_SHARED = (n: number, where: string, hex: string): string => `${n} formats sharing ${where} (${hex})`;
const SIGNATURES_SHARED_EXT = (n: number, where: string, hex: string, ext: string): string =>
  `${n} formats sharing ${where} (${hex}), all with extension .${ext}`;
const bytesAt = (m: SigMatch): string => {
  const n = m.fixed === 1 ? "1 byte" : `${m.fixed} bytes`;
  return m.fromEnd ? `${n} within the last ${m.offset.toLocaleString("en")} bytes` : `${n} at offset ${m.offset}`;
};

/**
 * The database writes its categories as slugs. Two of them are not words, and
 * one needs saying what it immunised against, so they are written out here.
 * A category not in this list is shown as the database wrote it: inventing a
 * label would claim to know something the rule did not say.
 */
const CATEGORY: Record<string, string> = {
  packer: "Packer",
  cryptor: "Cryptor",
  protector: "Protector",
  compiler: "Compiler",
  converter: "Converter",
  installer: "Installer",
  linker: "Linker",
  archive: "Archive",
  format: "File format",
  data: "Data",
  extender: "DOS extender",
  sfx: "Self-extracting archive",
  "self-displayer": "Self-displaying program",
  immunizer: "Antivirus immunizer",
};

/**
 * What the bytes on screen are wrapped in comes first, then what built them,
 * then the rest. A packed file is showing compressed output rather than the
 * program, which changes how everything else on screen should be read.
 */
const CATEGORY_ORDER = [
  "cryptor",
  "protector",
  "packer",
  "sfx",
  "installer",
  "compiler",
  "linker",
  "converter",
  "extender",
];

/** The categories that change how to read the bytes, and how to say each. */
const WRAPPER: Record<string, (m: ToolMatch) => string> = {
  packer: (m) => `packed with ${nameAndVersion(m)}`,
  protector: (m) => `protected with ${nameAndVersion(m)}`,
  cryptor: (m) => `encrypted with ${nameAndVersion(m)}`,
  sfx: (m) => `self-extracting (${nameAndVersion(m)})`,
};

const categoryLabel = (slug: string): string => CATEGORY[slug] ?? slug;

/** `Packer: UPX v3.96 (1985)`, with the author's own words in the brackets. */
const toolLine = (m: ToolMatch): string => {
  const head = `${categoryLabel(m.category)}: ${nameAndVersion(m)}`;
  return m.options === null ? head : `${head} (${m.options})`;
};

const sortTools = (found: readonly ToolMatch[]): ToolMatch[] => {
  const rank = (m: ToolMatch): number => {
    const i = CATEGORY_ORDER.indexOf(m.category);
    return i === -1 ? CATEGORY_ORDER.length : i;
  };
  return [...found].sort((a, b) => rank(a) - rank(b));
};

/** What to append to the readout, for a match that changes how to read it. */
const wrapperSuffix = (found: readonly ToolMatch[]): string => {
  const m = sortTools(found).find((x) => WRAPPER[x.category] !== undefined);
  return m === undefined ? "" : ` \u00b7 ${WRAPPER[m.category]?.(m) ?? ""}`;
};

/** The toolbar readout and the dialog behind it, as one thing. */
export type FileType = {
  /** The sentence itself, for the toolbar. */
  readonly label: HTMLElement;
  /** The button that opens the details. */
  readonly info: HTMLElement;
  readonly dialog: HTMLElement;
  /** Told the name whenever it changes, so the overview can show the same one. */
  onIdentity: (name: string) => void;
  /** The rules are still being asked, and the wait is long enough to say so. */
  identifying(): void;
  /** The rules could not be asked at all. */
  failed(): void;
  /** Which template is reading the file, and what it can say about it. */
  setTemplate(answer: TemplateAnswer | null): void;
  /** Which template the Fields table is being read with, for the dialog. */
  setNote(note: TemplateNote): void;
  /** What the file(1) rules said: null when they had nothing. */
  setFile(id: Identification | null): void;
  /** Ask the tool signatures and the format signatures, and fold in what they make of the file. */
  addMatches(doc: Doc): Promise<void>;
};

export function fileType(): FileType {
  // What the file is, for a file no template covers. Its own element rather
  // than the message slot: a save message is an event and passes, this is a
  // fact about the file and stays.
  const kindLabel = el("span", { className: "tb-kind" });
  // The details behind the readout. A button and a dialog rather than a
  // tooltip: the rule's sentence is long, worth copying, and worth reading at
  // leisure, none of which a title attribute allows.
  const kindInfo = el("button", { type: "button", className: "tb-info", textContent: "i" });
  kindInfo.setAttribute("aria-label", INFO_LABEL);
  kindInfo.hidden = true;
  const dlgBody = el("div", { className: "dlg-body" });
  const dialog = el(
    "dialog",
    { className: "dlg" },
    el("h2", { textContent: DIALOG_TITLE }),
    dlgBody,
    el("form", { method: "dialog", className: "dlg-close" }, el("button", { type: "submit", textContent: DIALOG_CLOSE })),
  );
  kindInfo.addEventListener("click", () => dialog.showModal());
  // The dialog element covers only the middle of the screen, so a click that
  // lands on it rather than on its contents is a click on the backdrop.
  dialog.addEventListener("click", (e) => {
    if (e.target === dialog) dialog.close();
  });

  // Everything that has answered. `file` undefined means the rules have not
  // answered yet; `tools` and `signatures` likewise.
  let answers: Answers = { template: null };
  let note: TemplateNote = null;
  let sigs: SigVerdict | null = null;
  let failed = false;
  let identifying = false;

  const api: FileType = {
    label: kindLabel,
    info: kindInfo,
    dialog,
    onIdentity: () => {},
    identifying: () => {
      identifying = true;
      render();
    },
    failed: () => {
      failed = true;
      render();
    },
    setTemplate: (answer) => {
      answers = { ...answers, template: answer };
      render();
    },
    setNote: (n) => {
      note = n;
      render();
    },
    setFile: (id) => {
      identifying = false;
      answers = { ...answers, file: id };
      render();
    },
    addMatches: async (doc) => {
      let found: ToolMatch[];
      try {
        found = await doc.detectTools(answers.file !== null && answers.file !== undefined);
      } catch (e) {
        console.error("detectTools", e);
        found = [];
      }
      answers = { ...answers, tools: sortTools(found) };
      render();
      try {
        sigs = await doc.signatureMatches();
      } catch (e) {
        console.error("signatureMatches", e);
        return;
      }
      answers = { ...answers, signatures: sigs?.matches ?? [] };
      render();
    },
  };

  /** Redraw the toolbar, tell the overview, and refill the dialog. */
  const render = (): void => {
    const id = decide(answers);
    const tools = answers.tools ?? [];
    let line: string;
    if (id.name !== null) {
      const named = id.source === "signature" ? (answers.signatures ?? []).find((s) => s.format.label === id.name) : undefined;
      line = named === undefined ? id.name : SIGNATURE_NAMED[named.format.source](id.name);
      // A packed program is showing compressed output rather than the
      // program, which is worth saying in the toolbar line whatever named it.
      if (id.source !== "tools") line += wrapperSuffix(tools);
    } else if (failed) line = IDENTIFY_FAILED_MSG;
    else if (identifying) line = IDENTIFYING_MSG;
    // Unknown as soon as the rules say so; a signature that lands later
    // replaces it, and a blank line in the meantime would say nothing.
    else if (answers.file === null) line = UNKNOWN_TYPE_MSG;
    else line = "";
    kindLabel.textContent = line;
    // The toolbar copy is cut short, so the whole sentence stays reachable
    // on hover as well as in the dialog.
    kindLabel.title = failed ? IDENTIFY_FAILED_TITLE : line;
    // The overview hears the name once there is one, or once the rules have
    // said there is none: an empty name before that reads as "no answer"
    // when the answer is still on its way.
    if (id.name !== null || answers.file !== undefined) api.onIdentity(id.name ?? "");
    // Nothing to open until something has answered or the rules have given
    // up: an empty dialog is worse than no button.
    if (answers.file === undefined && answers.template === null) return;
    dlgBody.replaceChildren(...details(id));
    kindInfo.hidden = false;
  };

  const row = (label: string, value: Node | string): HTMLElement =>
    el("div", { className: "dlg-row" }, el("span", { className: "dlg-key", textContent: label }), value);

  /** Where the chosen name came from, in a sentence. */
  const sourceText = (id: ReturnType<typeof decide>): string => {
    switch (id.source) {
      case "template":
        return SOURCE_TEXT.template(answers.template === null ? "" : templateLabel(answers.template.name));
      case "signature": {
        const m = (answers.signatures ?? []).find((s) => s.format.label === id.name);
        return SOURCE_TEXT.signature(SIGNATURE_SOURCE[m?.format.source ?? "wikidata"]);
      }
      case "file":
        return SOURCE_TEXT.file("");
      case "tools":
        return SOURCE_TEXT.tools("");
      case null:
        return "";
    }
  };

  /** The dialog's contents for what has answered so far. */
  const details = (id: ReturnType<typeof decide>): HTMLElement[] => {
    const rows: HTMLElement[] = [];
    const file = answers.file ?? null;
    if (id.name === null) {
      rows.push(el("p", { textContent: NO_MATCH_BODY }));
    } else {
      rows.push(el("p", { className: "dlg-sentence", textContent: id.name }));
      rows.push(row(SOURCE_KEY, sourceText(id)));
    }
    // What the rules know about the format, whichever answer was chosen: a
    // media type and extensions are facts about the file either way.
    if (file !== null) {
      if (file.mime !== "") rows.push(row("Media type", file.mime));
      if (file.ext.length > 0) rows.push(row("Extensions", file.ext.join(", ")));
      if (file.source !== "") rows.push(row("Rule file", file.source));
    }
    // Which template the Fields table is reading with, next to what the file
    // is rather than under the credits: it is an answer about this file too.
    if (note !== null) {
      const value = note.kind === "builtin" ? templateLabel(note.name) : el("em", { textContent: SIGNATURE_ONLY });
      rows.push(row(TEMPLATE_KEY, value));
    }
    // Every other answer, with what it rests on. The signatures come last and
    // grouped, since most files match a crowd of them.
    const others = id.candidates.slice(1).filter((c) => c.source !== "signature");
    const tools = answers.tools ?? [];
    if (others.length > 0) {
      rows.push(el("p", { className: "dlg-muted", textContent: OTHERS_HEADING }), el("ul", { className: "dlg-others" }, ...others.map(candidateRow)));
    }
    // Each credit covers only the answers it found. An answer the editor
    // read out of the file itself is not the database's to be credited
    // with, and the database's rules are not this editor's.
    if (tools.some((m) => m.source !== OWN_SOURCE)) rows.push(el("p", { className: "dlg-muted", textContent: MATCHED_AGAINST }));
    if (tools.some((m) => m.source === OWN_SOURCE)) rows.push(el("p", { className: "dlg-muted", textContent: READ_FROM_STUB }));
    if (sigs !== null && sigs.matches.length > 0) {
      rows.push(el("p", { textContent: SIGNATURES_INTRO }), ...signatureRows(sigs.matches, sigs.extension));
      rows.push(el("p", { className: "dlg-muted", textContent: SIGNATURES_CREDIT(sigs.fetched) }));
    }
    return rows;
  };

  /** One answer that was not chosen: its name, then how it knows. */
  const candidateRow = (c: Candidate): HTMLElement => {
    const parts: (Node | string)[] = [];
    if (c.source === "tools") {
      const m = (answers.tools ?? []).find((t) => `${nameAndVersion(t)} (${t.category})` === c.name);
      parts.push(el("span", { className: "dlg-tool", textContent: m === undefined ? c.name : toolLine(m) }));
    } else parts.push(el("span", { textContent: c.name }));
    parts.push(el("span", { className: "dlg-muted", textContent: c.evidence }));
    if (c.disagrees) parts.push(el("span", { className: "dlg-disagrees", textContent: DISAGREES }));
    return el("li", {}, ...parts);
  };

  /** One format a signature names: its name, linked, what matched, and where to read more. */
  const signatureRow = (m: SigMatch): HTMLElement => {
    const f = m.format;
    const parts: (Node | string)[] = [];
    if (f.source === "wikidata") {
      parts.push(el("a", { href: wikidataUrl(f.id), target: "_blank", rel: "noopener", textContent: f.label }));
    } else parts.push(el("span", { textContent: f.unfinished === true ? `${f.label}\u2026` : f.label }));
    parts.push(el("span", { className: "dlg-muted", textContent: SIGNATURE_WORD[f.source] }));
    const exts = [...(f.ext ?? []), ...(f.wpExt ?? [])];
    if (exts.length > 0) {
      parts.push(el("span", { className: "dlg-wiki-ext", textContent: exts.map((e) => `.${e}`).join(" ") }));
    }
    parts.push(el("span", { className: "dlg-muted", textContent: bytesAt(m) }));
    // An article about the format itself, or failing that about the format it
    // is a version or part of, named so the link says where it goes.
    const wp = f.wp !== undefined ? { title: f.wp, text: WIKIPEDIA_LINK } : f.parent !== undefined ? { title: f.parent.wp, text: `${WIKIPEDIA_LINK}: ${f.parent.label}` } : null;
    if (wp !== null) parts.push(el("a", { href: wikipediaUrl(wp.title), target: "_blank", rel: "noopener", textContent: wp.text }));
    const li = el("li", { title: m.pattern }, ...parts);
    if (m.extensionAgrees) li.classList.add("dlg-wiki-ext-agrees");
    return li;
  };

  /**
   * The matches as rows: the best few in the open, the rest folded away, and
   * a crowd that all matched the same bytes folded into one line, since a
   * hundred formats that are all ZIP inside say only that the file is a ZIP.
   * A crowd whose members all list the file's extension is a crowd of its own,
   * ahead of the rest, and says so.
   */
  const signatureRows = (matches: readonly SigMatch[], extension: string): HTMLElement[] => {
    type Group = { readonly key: string; readonly members: SigMatch[] };
    const groups: Group[] = [];
    const byKey = new Map<string, Group>();
    for (const m of matches) {
      const key = `${m.pattern} ${m.offset} ${m.fromEnd} ${m.extensionAgrees}`;
      let g = byKey.get(key);
      if (g === undefined) {
        g = { key, members: [] };
        byKey.set(key, g);
        groups.push(g);
      }
      g.members.push(m);
    }
    const render = (g: Group): HTMLElement => {
      const first = g.members[0];
      if (first === undefined) throw new Error("empty group");
      if (g.members.length < SIGNATURES_CROWD) return el("ul", { className: "dlg-wiki" }, ...g.members.map(signatureRow));
      const hex = first.pattern.replace(/(..)(?=.)/g, "$1 ");
      return el(
        "details",
        { className: "dlg-more" },
        el("summary", {
          textContent: first.extensionAgrees
            ? SIGNATURES_SHARED_EXT(g.members.length, bytesAt(first), hex, extension)
            : SIGNATURES_SHARED(g.members.length, bytesAt(first), hex),
        }),
        el("ul", { className: "dlg-wiki" }, ...g.members.map(signatureRow)),
      );
    };
    const shown = groups.slice(0, SIGNATURES_SHOWN).map(render);
    const rest = groups.slice(SIGNATURES_SHOWN);
    if (rest.length === 0) return shown;
    const count = rest.reduce((n, g) => n + g.members.length, 0);
    return [...shown, el("details", { className: "dlg-more" }, el("summary", { textContent: SIGNATURES_MORE(count) }), ...rest.map(render))];
  };

  return api;
}
