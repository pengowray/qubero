// What the open file is: the sentence the identification rules produce, the
// signature database's answer about what made it, and the dialog behind both.
// The toolbar shows one line; everything the rules said is a click away.

import { el } from "./dom.js";
import type { Doc, Identification, ToolMatch } from "./doc.js";
import { OWN_SOURCE } from "./doc.js";

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
  bdb: "Berkeley DB",
  braw: "Blackmagic RAW",
  arw: "Sony ARW",
  bardstale: "Bard's Tale I (DOS save)",
  cr2: "Canon CR2",
  c16: "16-bit I/Q samples",
  cab: "Windows cabinet",
  cdr: "CorelDRAW CDR",
  cmx: "Corel Presentation Exchange CMX",
  cpio: "cpio archive (initramfs)",
  deb: "Debian package",
  dng: "Adobe DNG",
  dtb: "Device tree blob",
  gdbm: "GNU dbm",
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
  rar5: "RAR 5 archive",
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

/** Which template the Fields table is being read with: a built-in by name, the
 *  one field the identification rule's own signature makes, or neither. */
export type TemplateNote = { readonly kind: "builtin"; readonly name: string } | { readonly kind: "signature" } | null;

export const builtinTemplate = (name: string): TemplateNote => ({ kind: "builtin", name });
export const SIGNATURE_TEMPLATE: TemplateNote = { kind: "signature" };
const MATCHED_AGAINST = "Matched against the signature database of the Detect It Easy project.";
const READ_FROM_STUB = "Identified from the loader stub the compiler placed at the end of the program.";

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
  /** Ask the signature database as well, and fold in what it makes of the file. */
  addTools(doc: Doc, id: Identification | null, template: string | null): Promise<void>;
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
  dlgBody.replaceChildren(...rows);
  kindInfo.hidden = false;
};

/**
 * Ask the signature rules what made this file, and fold the answer into what
 * is already on screen. A file nothing else could name is named by this if it
 * can be, since for a .COM there is nothing else to go on.
 */
const addToolMatches = async (doc: Doc, id: Identification | null, template: string | null): Promise<void> => {
  let found: ToolMatch[];
  try {
    found = await doc.detectTools(id !== null);
  } catch (e) {
    console.error("detectTools", e);
    return;
  }
  tools = found;
  const note = template === null ? null : builtinTemplate(template);
  showDetails(id, id === null && found.length > 0 ? null : note);
  if (found.length === 0) return;
  if (id === null) {
    // Nothing else knew anything, so this is the answer rather than a note
    // beside one.
    const m = sortTools(found)[0];
    if (m !== undefined) {
      const line = `Signature match: ${nameAndVersion(m)} (${m.category})`;
      kindLabel.textContent = line;
      kindLabel.title = line;
    }
    return;
  }
  const suffix = wrapperSuffix(found);
  if (suffix !== "") kindLabel.textContent = `${id.message}${suffix}`;
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
    addTools: addToolMatches,
  };
}
