// What the open file is: the sentence the identification rules produce, the
// signature database's answer about what made it, and the dialog behind both.
// The toolbar shows one line; everything the rules said is a click away.

import { el } from "./dom.ts";
import type { Doc, Identification, TemplateNode, ToolMatch, WikiVerdict } from "./doc.ts";
import { OWN_SOURCE } from "./doc.ts";
import { namingMatch, wikidataUrl, wikipediaUrl, type WikiMatch } from "./wikiformats.ts";

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
  sac: "SAC seismogram",
  cdf: "NASA CDF",
  hdf4: "HDF4",
  parquet: "Parquet",
  mat: "MATLAB MAT",
};

/** A built-in's human-facing name; internal names remain stable API values. */
export const templateLabel = (name: string): string => TEMPLATE_LABEL[name] ?? name;

/** A useful identity when the full template recognises a format the rule
 * database does not. A label that is already plural is what the file is:
 * "Login records file" reads as a mistake, "Login records" does not. Only a
 * label of several words, since a template with no label of its own falls
 * back to its own name and half the model formats are called things like
 * `3ds`, which is a file and not a plural of anything. */
export const templateTypeName = (name: string): string => {
  if (name === "bardstale") return "The Bard's Tale I MS-DOS save game";
  const label = templateLabel(name);
  return label.endsWith("s") && label.includes(" ") ? label : `${label} file`;
};

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

/** The plain label, or the fuller sentence where the template has one. */
export const templateIdentity = (doc: Doc, name: string): string =>
  templateSentence(doc, name) ?? templateTypeName(name);

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
const WIKIDATA_INTRO = "Formats on Wikidata with a matching signature:";
const WIKIDATA_LINK = "Wikidata";
const WIKIPEDIA_LINK = "Wikipedia";
const WIKIDATA_CREDIT = (fetched: string): string => `Signatures from Wikidata property P4152, as of ${fetched}.`;
/** The toolbar, for a file only Wikidata could name. */
const WIKIDATA_NAMED = (label: string): string => `${label} (signature listed on Wikidata)`;
/** How many rows the dialog shows before folding the rest away. */
const WIKIDATA_SHOWN = 5;
/** A signature so many formats share that listing them says nothing. */
const WIKIDATA_CROWD = 4;
const WIKIDATA_MORE = (n: number): string => `${n} more formats`;
const WIKIDATA_SHARED = (n: number, where: string, hex: string): string => `${n} formats sharing ${where} (${hex})`;
const WIKIDATA_SHARED_EXT = (n: number, where: string, hex: string, ext: string): string =>
  `${n} formats sharing ${where} (${hex}), all listing .${ext}`;
const bytesAt = (m: WikiMatch): string => {
  const n = m.fixed === 1 ? "1 byte" : `${m.fixed} bytes`;
  return m.fromEnd ? `${n} in the last ${m.offset}` : `${n} at offset ${m.offset}`;
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

/** `UPX v3.96`. The v matters: names in this database end in digits. */
const nameAndVersion = (m: ToolMatch): string => (m.version === null ? m.name : `${m.name} v${m.version}`);

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
  /** The rules are still being asked, and the wait is long enough to say so. */
  identifying(): void;
  /** They could not be asked at all. */
  failed(): void;
  /** They were asked, and had nothing to say. */
  unknown(): void;
  /** The rule's own sentence. */
  named(message: string): void;
  /** Fill the dialog for one outcome, and show the button that opens it. */
  details(id: Identification | null, template: TemplateNote): void;
  /** Ask the signature database and Wikidata as well, and fold in what they make of the file. */
  addMatches(doc: Doc, id: Identification | null, template: string | null): Promise<void>;
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

/** Fill the dialog for one outcome, and show the button that opens it. */
// Filled in once the signature rules have answered, so reopening the
// dialog shows them without asking again.
let tools: ToolMatch[] | null = null;
let wiki: WikiVerdict | null = null;
const showDetails = (id: Identification | null, template: TemplateNote): void => {
  const rows: HTMLElement[] = [];
  const row = (label: string, value: Node | string): void => {
    rows.push(el("div", { className: "dlg-row" }, el("span", { className: "dlg-key", textContent: label }), value));
  };
  if (id === null) {
    rows.push(el("p", { textContent: NO_MATCH_BODY }));
  } else {
    rows.push(el("p", { className: "dlg-sentence", textContent: id.message }));
    if (id.mime !== "") row("Media type", id.mime);
    if (id.ext.length > 0) row("Extensions", id.ext.join(", "));
    if (id.source !== "") row("Rule file", id.source);
  }
  // Which template the Fields table is reading with, next to what the file is
  // rather than under the credits: it is an answer about this file too.
  if (template !== null) {
    const value =
      template.kind === "builtin" ? templateLabel(template.name) : el("em", { textContent: SIGNATURE_ONLY });
    row(TEMPLATE_KEY, value);
  }
  // What made the file, when anything knows. Its own block after the file
  // type's, so each muted credit line sits under the answers it covers.
  // Nothing to add when the signature database found nothing: a line saying
  // so is a line about the check rather than about the file.
  if (tools !== null && tools.length > 0) {
    for (const m of sortTools(tools)) {
      rows.push(el("p", { className: "dlg-tool", textContent: toolLine(m) }));
    }
    // Each credit covers only the answers it found. An answer the editor
    // read out of the file itself is not the database's to be credited
    // with, and the database's rules are not this editor's.
    if (tools.some((m) => m.source !== OWN_SOURCE)) {
      rows.push(el("p", { className: "dlg-muted", textContent: MATCHED_AGAINST }));
    }
    if (tools.some((m) => m.source === OWN_SOURCE)) {
      rows.push(el("p", { className: "dlg-muted", textContent: READ_FROM_STUB }));
    }
  }
  // What Wikidata lists, after the answers with real evidence behind them.
  // Most of these patterns are a few bytes long and shared by hundreds of
  // formats, so the best few are shown and the rest are a click away.
  if (wiki !== null && wiki.matches.length > 0) {
    rows.push(el("p", { textContent: WIKIDATA_INTRO }), ...wikiRows(wiki.matches, wiki.extension));
    rows.push(el("p", { className: "dlg-muted", textContent: WIKIDATA_CREDIT(wiki.fetched) }));
  }
  dlgBody.replaceChildren(...rows);
  kindInfo.hidden = false;
};

/** One format Wikidata lists: its name, linked, what matched, and where to read more. */
const wikiRow = (m: WikiMatch): HTMLElement => {
  const f = m.format;
  const parts: (Node | string)[] = [el("a", { href: wikidataUrl(f.id), target: "_blank", rel: "noopener", textContent: f.label })];
  const exts = [...(f.ext ?? []), ...(f.wpExt ?? [])];
  if (exts.length > 0) {
    parts.push(el("span", { className: "dlg-wiki-ext", textContent: exts.map((e) => `.${e}`).join(" ") }));
  }
  parts.push(el("span", { className: "dlg-muted", textContent: bytesAt(m) }));
  // An article about the format itself, or failing that about the format it
  // is a version or part of, named so the link says where it goes.
  const wp = f.wp !== undefined ? { title: f.wp, text: WIKIPEDIA_LINK } : f.parent !== undefined ? { title: f.parent.wp, text: `${WIKIPEDIA_LINK}: ${f.parent.label}` } : null;
  if (wp !== null) parts.push(el("a", { href: wikipediaUrl(wp.title), target: "_blank", rel: "noopener", textContent: wp.text }));
  const row = el("li", { title: m.pattern }, ...parts);
  if (m.extensionAgrees) row.classList.add("dlg-wiki-ext-agrees");
  return row;
};

/**
 * The matches as rows: the best few in the open, the rest folded away, and
 * a crowd that all matched the same bytes folded into one line, since a
 * hundred formats that are all ZIP inside say only that the file is a ZIP.
 * A crowd whose members all list the file's extension is a crowd of its own,
 * ahead of the rest, and says so.
 */
const wikiRows = (matches: readonly WikiMatch[], extension: string): HTMLElement[] => {
  type Group = { readonly key: string; readonly members: WikiMatch[] };
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
    if (g.members.length < WIKIDATA_CROWD) return el("ul", { className: "dlg-wiki" }, ...g.members.map(wikiRow));
    const hex = first.pattern.replace(/(..)(?=.)/g, "$1 ");
    return el(
      "details",
      { className: "dlg-more" },
      el("summary", {
        textContent: first.extensionAgrees
          ? WIKIDATA_SHARED_EXT(g.members.length, bytesAt(first), hex, extension)
          : WIKIDATA_SHARED(g.members.length, bytesAt(first), hex),
      }),
      el("ul", { className: "dlg-wiki" }, ...g.members.map(wikiRow)),
    );
  };
  const shown = groups.slice(0, WIKIDATA_SHOWN).map(render);
  const rest = groups.slice(WIKIDATA_SHOWN);
  if (rest.length === 0) return shown;
  const count = rest.reduce((n, g) => n + g.members.length, 0);
  return [...shown, el("details", { className: "dlg-more" }, el("summary", { textContent: WIKIDATA_MORE(count) }), ...rest.map(render))];
};

/**
 * Ask the signature rules what made this file, and Wikidata what it might be,
 * and fold the answers into what is already on screen. A file nothing else
 * could name is named by the first of these that can, since for a .COM there
 * is nothing else to go on, and for a Parquet file only Wikidata has a word.
 */
const addOtherMatches = async (doc: Doc, id: Identification | null, template: string | null): Promise<void> => {
  let found: ToolMatch[];
  try {
    found = await doc.detectTools(id !== null);
  } catch (e) {
    console.error("detectTools", e);
    found = [];
  }
  tools = found;
  const note = template === null ? null : builtinTemplate(template);
  showDetails(id, id === null && found.length > 0 ? null : note);
  const named = (line: string): void => {
    kindLabel.textContent = line;
    kindLabel.title = line;
  };
  if (found.length > 0) {
    if (id === null) {
      // Nothing else knew anything, so this is the answer rather than a note
      // beside one.
      const m = sortTools(found)[0];
      if (m !== undefined) named(`Signature match: ${nameAndVersion(m)} (${m.category})`);
    } else {
      const suffix = wrapperSuffix(found);
      if (suffix !== "") kindLabel.textContent = `${id.message}${suffix}`;
    }
  }
  try {
    wiki = await doc.wikidataMatches();
  } catch (e) {
    console.error("wikidataMatches", e);
    return;
  }
  if (wiki === null) return;
  showDetails(id, id === null && found.length > 0 ? null : note);
  if (id === null && template === null && found.length === 0) {
    const best = namingMatch(wiki.matches);
    if (best !== null) named(WIKIDATA_NAMED(best.format.label));
  }
};

  return {
    label: kindLabel,
    info: kindInfo,
    dialog,
    identifying: () => {
      kindLabel.textContent = IDENTIFYING_MSG;
    },
    failed: () => {
      kindLabel.textContent = IDENTIFY_FAILED_MSG;
      kindLabel.title = IDENTIFY_FAILED_TITLE;
    },
    unknown: () => {
      kindLabel.textContent = UNKNOWN_TYPE_MSG;
    },
    named: (message: string) => {
      kindLabel.textContent = message;
      // The toolbar copy is cut short, so the whole sentence stays reachable
      // on hover as well as in the dialog.
      kindLabel.title = message;
    },
    details: showDetails,
    addMatches: addOtherMatches,
  };
}
