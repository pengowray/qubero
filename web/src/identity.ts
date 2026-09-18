// What to call the open file, when several things have an opinion.
//
// Four sources answer, at different times and with different kinds of
// evidence: a built-in template read the file's structure; the file(1) rules
// matched bytes and wrote a sentence; the Detect It Easy rules named the tool
// that built an executable; and the signature database found formats whose
// identifying bytes are there. This decides which answer is the file's name,
// which are worth listing beside it, and where two of them disagree. Pure:
// nothing here touches the page, so the choice can be tested on its own.

import type { Identification, ToolMatch } from "./doc.ts";
import { namingMatch, type SigMatch } from "./signatures.ts";

/** Where an answer came from. */
export type Source = "template" | "file" | "tools" | "signature";

/** The template's answer: which one, and the sentence it can write, if any. */
export type TemplateAnswer = {
  readonly name: string;
  /** What the template read out of the file, for the few that write one. */
  readonly sentence: string | null;
  /** The plain name for the format, `Parquet file`. */
  readonly label: string;
  /** True when the template was applied to a file whose signature is not the
   *  one it requires: the fields were read, but by a template the file's own
   *  first bytes contradict. Picking PNG for a ZIP is exactly this, and until
   *  the toolbar said so the answer read as if Qubero had recognised the
   *  file. */
  readonly signatureMismatch?: boolean;
};

/** Everything that has answered so far. Null is "not asked or not yet". */
export type Answers = {
  readonly template: TemplateAnswer | null;
  /** Null: the rules had nothing. Undefined: not asked yet. */
  readonly file?: Identification | null;
  readonly tools?: readonly ToolMatch[];
  readonly signatures?: readonly SigMatch[];
};

/** One answer, as the dialog lists it. */
export type Candidate = {
  readonly source: Source;
  readonly name: string;
  /** How it knows, for the muted text after the name. */
  readonly evidence: string;
  /** Says something the chosen name does not: a disagreement worth seeing. */
  readonly disagrees: boolean;
};

export type Identity = {
  /** The name for the toolbar and the overview, or null for no answer. */
  readonly name: string | null;
  readonly source: Source | null;
  /** The chosen answer first, then the rest in order of evidence. */
  readonly candidates: readonly Candidate[];
};

/**
 * Templates whose match is inference rather than a signature. `PROBES` in
 * recognise.rs says why each is asked last: a zlib stream is two bytes that
 * agree with each other, a MATLAB 4 file is five numbers that agree with the
 * file's length, a bencoded file or a pickle is one that parses to the end.
 * file(1) may outrank these; it may not outrank a template that found a
 * magic number and read what follows.
 *
 * `picklefpf` is not one of them, though `pickle` is. Parsing to the end says
 * only that the bytes read as some program; matching a Familiar Pickle Form
 * says a reviewed grammar accounted for every opcode and operand in the file
 * and knows what each one holds. That is stronger evidence than any rule
 * keyed on a file's first bytes, so it does not yield to file(1). `joblib` is
 * not one of them either: its evidence is the opcodes stopping exactly where
 * joblib writes an array's bytes, which no first-bytes rule can see.
 */
export const WEAK_TEMPLATES: ReadonlySet<string> = new Set(["zlib", "mat", "bencode", "pickle", "com", "cue", "godottext"]);

/**
 * The weakest rule a weak template yields to. A rule's strength is 20, plus
 * 10 for every byte it compares, plus 10 for comparing them for equality: 70
 * is four bytes, the length of most magic numbers. Under that the rule has
 * less behind it than the template does. Two bytes, `80 05`, open every
 * protocol 5 pickle, and they are also the whole of the rule for a XENIX
 * object file: with no threshold, every such pickle was called XENIX.
 */
export const WEAK_TEMPLATE_YIELDS_AT = 70;

/**
 * The extensions a template's format goes by, for the templates whose name
 * is not already the extension. Used to tell whether file(1)'s answer is
 * about the same format as the template's: the rules name a PE `PE32+
 * executable` with extensions `exe` and `dll`, and the template is `pe`.
 */
const TEMPLATE_EXT: Record<string, readonly string[]> = {
  pe: ["exe", "dll", "sys", "ocx", "scr", "drv", "efi", "cpl"],
  msdos: ["exe"],
  ne: ["exe", "dll", "drv", "fon"],
  le: ["exe", "dll", "386", "vxd"],
  com: ["com"],
  elf: ["so", "o", "ko", "elf"],
  bpf: ["o"],
  macho: ["dylib", "o"],
  coff: ["obj", "o"],
  omf: ["obj"],
  sqlite: ["db", "sqlite", "sqlite3"],
  self: ["self", "db"],
  iso9660: ["iso"],
  cdrom: ["bin", "img"],
  ar: ["a", "lib", "deb"],
  deb: ["deb"],
  cpio: ["cpio"],
  tar: ["tar"],
  zip: ["zip", "jar", "epub", "docx", "xlsx", "pptx", "odt", "apk", "npz"],
  zarrzip: ["zip"],
  adioszip: ["zip"],
  "7z": ["7z"],
  rar4: ["rar"],
  rar5: ["rar"],
  gzip: ["gz", "tgz"],
  // A `.vcf.gz`, `.bed.gz` or `.fa.gz` is `gz` to the rules, which name one
  // extension each.
  bgzf: ["bam", "csi", "gz", "bgz"],
  compress: ["z"],
  bzip2: ["bz2"],
  lzip: ["lz"],
  xz: ["xz"],
  zstd: ["zst"],
  lz4: ["lz4"],
  lha: ["lzh", "lha"],
  cab: ["cab"],
  xar: ["xar", "pkg"],
  rpm: ["rpm"],
  wasm: ["wasm"],
  gguf: ["gguf"],
  safetensors: ["safetensors"],
  npy: ["npy"],
  parquet: ["parquet"],
  arrow: ["arrow", "feather", "ipc"],
  arrowstream: ["arrows", "stream"],
  hdf5: ["h5", "hdf5", "h5ad", "nc"],
  hdf4: ["hdf", "h4"],
  netcdf: ["nc", "cdf"],
  cdf: ["cdf"],
  fits: ["fits", "fit", "fts"],
  root: ["root"],
  gwf: ["gwf"],
  grib: ["grib", "grb", "grib2"],
  bufr: ["bufr", "bfr"],
  mseed: ["mseed", "msd"],
  mseed3: ["mseed3", "ms3"],
  sac: ["sac"],
  segy: ["sgy", "segy"],
  tdms: ["tdms", "tdms_index"],
  adiosbp3: ["bp"],
  adiosbp4idx: ["idx"],
  adiosbp5idx: ["idx"],
  nifti: ["nii", "hdr"],
  analyze: ["hdr", "img"],
  mat: ["mat"],
  mp4: ["mp4", "m4a", "m4v", "mov", "3gp", "heic", "avif"],
  ftyp: ["mp4", "m4a", "m4v", "mov", "3gp", "heic", "avif"],
  mkv: ["mkv", "webm", "mka"],
  ogg: ["ogg", "oga", "ogv", "opus"],
  wav: ["wav"],
  w4v: ["wav", "w4v"],
  midi: ["mid", "midi"],
  swf: ["swf"],
  png: ["png"],
  p8png: ["png"],
  p64png: ["png"],
  bmp: ["bmp"],
  pcx: ["pcx"],
  dbf: ["dbf"],
  pnm: ["pbm", "pgm", "ppm", "pnm", "pam"],
  ico: ["ico", "cur"],
  psd: ["psd", "psb"],
  jxr: ["jxr", "wdp"],
  // A bare codestream, a JP2 file, and the JPX and JPM files that open with
  // the same boxes.
  jpeg2000: ["jp2", "j2k", "j2c", "jpc", "jpx", "jpf", "jpm"],
  dng: ["dng"],
  cr2: ["cr2"],
  nef: ["nef"],
  arw: ["arw"],
  orf: ["orf"],
  pef: ["pef"],
  rw2: ["rw2"],
  srw: ["srw"],
  braw: ["braw"],
  dv: ["dv"],
  aseprite: ["aseprite", "ase"],
  xm: ["xm"],
  it: ["it"],
  s3m: ["s3m"],
  mod: ["mod"],
  mca: ["mca", "mcr"],
  cue: ["cue"],
  godot: ["res", "scn", "tres", "tscn"],
  godottext: ["tres", "tscn"],
  godotpck: ["pck"],
  lnk: ["lnk"],
  dtb: ["dtb"],
  journal: ["journal"],
  utmp: ["utmp", "wtmp"],
  pdb: ["pdb"],
  pdb2: ["pdb"],
  portablepdb: ["pdb"],
  draco: ["drc"],
  thumbsdb: ["db"],
  unityassets: ["assets"],
  unitybundle: ["unity3d", "bundle"],
  exp: ["exp"],
  pak: ["pak"],
  macbinary: ["bin"],
  binhex: ["hqx"],
  stuffit: ["sit"],
  compactpro: ["cpt"],
  whisper: ["wsp"],
  claudetheme: ["json"],
  hackrffw: ["bin"],
  gdbm: ["gdbm", "db"],
  bdb: ["db"],
  grubenv: ["env"],
  spp: ["bin"],
  bencode: ["torrent"],
  pickle: ["pickle", "pkl", "p"],
  picklefpf: ["pickle", "pkl", "p"],
  joblib: ["joblib", "pkl"],
  eps: ["eps", "epsf", "epsi"],
  c16: ["c16"],
  omezarr: ["json"],
  cdr: ["cdr"],
  cmx: ["cmx"],
  bardstale: ["tpw"],
  p64rom: ["p64"],
};

/** The extensions a template's format goes by, its own name first when that
 *  reads as one: `png` is its own extension, `pe` is not listed as one. */
export const templateExtensions = (template: string): string[] => {
  const listed = TEMPLATE_EXT[template] ?? [];
  return /^[a-z0-9]+$/.test(template) && !listed.includes(template) && listed.length === 0 ? [template] : [...listed];
};

/**
 * What a file(1) sentence calls a template's format, where that is a word the
 * label does not have. An OMF module opens with a THEADR record, `80` and a
 * length, and the rules call one whose length is five a XENIX object: XENIX
 * used the same Intel object format, and the rule is those two bytes. The
 * template read the records, so the two are one answer and not a disagreement.
 */
const ALSO_CALLED: Record<string, readonly string[]> = { omf: ["xenix"] };

/** Words in a file(1) sentence that say nothing about which format it is. */
const STOP_WORDS = new Set(["data", "file", "archive", "image", "executable", "format", "document", "text", "binary", "compressed", "audio", "video", "the", "for", "with", "and", "or", "of", "a", "an", "v", "version"]);

const words = (s: string): Set<string> =>
  new Set(
    s
      .toLowerCase()
      .split(/[^a-z0-9+]+/)
      .filter((w) => w.length >= 3 && !STOP_WORDS.has(w)),
  );

/**
 * Whether file(1)'s answer is about the same format the template read. The
 * rules carry extensions and a media type, which is the reliable test; the
 * words of the sentence are the last resort, and only the ones that name
 * something.
 */
export function agrees(file: Identification, template: string, label: string): boolean {
  const exts = new Set([template, ...(TEMPLATE_EXT[template] ?? [])]);
  if (file.ext.some((e) => exts.has(e.toLowerCase()))) return true;
  const mime = file.mime.toLowerCase();
  if (mime !== "" && [...exts].some((e) => e.length >= 3 && mime.includes(e))) return true;
  const said = words(file.message);
  const ours = new Set([...words(label), ...[...exts].filter((e) => e.length >= 3), ...(ALSO_CALLED[template] ?? [])]);
  return [...ours].some((w) => said.has(w));
}

/** A file(1) sentence that ends in a comma is one whose first line matched and
 *  whose every branch under it did not: `National Instruments,` for a Godot
 *  file that starts `RSRC` as a LabVIEW file does. */
const isHalfMatch = (file: Identification): boolean => /,\s*$/.test(file.message);
const trimmed = (file: Identification): string => file.message.replace(/,\s*$/, "");

/** `UPX v3.96`. The v matters: names in this database end in digits. */
export const nameAndVersion = (m: ToolMatch): string => (m.version === null ? m.name : `${m.name} v${m.version}`);

const bytesAt = (m: SigMatch): string => {
  const n = m.fixed === 1 ? "1 byte" : `${m.fixed} bytes`;
  return m.fromEnd ? `${n} within the last ${m.offset.toLocaleString("en")} bytes` : `${n} at offset ${m.offset}`;
};

/** What the file type dialog says about a template applied to a file whose
 *  signature is not the one it requires. The label is the format the template
 *  reads, which is the thing the file is being read as rather than the thing
 *  it is. */
const SIGNATURE_MISMATCH = (label: string): string => `Template ${label} was applied, but the signature does not match`;

/** How a file(1) rule knows: which rule file it is in, since a rule that
 *  lost has no other row to say so, and how much it compared. */
const fileEvidence = (f: Identification): string =>
  f.source === "" ? `file(1) rule, strength ${Math.round(f.strength)}` : `file(1) rule in ${f.source}, strength ${Math.round(f.strength)}`;

/**
 * Decide the file's name from whatever has answered, and list every answer
 * with the chosen one first.
 *
 * A template that read the file outranks the rules that matched its bytes,
 * unless the two agree about the format, in which case the rules' sentence
 * is the name because it says more: the template calls a PNG a PNG, the rule
 * says it is 1280 by 720. A weak template (see `WEAK_TEMPLATES`) yields to
 * a rule that compared four bytes or more either way. With no template, the
 * rules name the file, unless theirs compared fewer than four bytes and a
 * signature has the file's extension behind it; failing them, the tool that
 * built it; failing that, a signature the file's
 * extension vouches for or that is long enough to vouch for itself.
 */
export function decide(a: Answers): Identity {
  const t = a.template;
  const f = a.file ?? null;
  const tools = a.tools ?? [];
  const sigs = a.signatures ?? [];
  const candidates: Candidate[] = [];
  let name: string | null = null;
  let source: Source | null = null;
  const choose = (n: string, s: Source): void => {
    if (name === null) {
      name = n;
      source = s;
    }
  };

  const templateName = t === null ? null : (t.sentence ?? t.label);
  const fileAgrees = t !== null && f !== null && !isHalfMatch(f) && agrees(f, t.name, t.label);

  // The order of these blocks is the order of evidence, and `choose` takes
  // the first name offered. A template the file's own signature contradicts
  // is the weakest evidence there is, whatever it read: it goes last, so a
  // ZIP read with the PNG template is called a ZIP by the rules and the
  // template's answer is listed under it with what is wrong.
  const mismatch = t !== null && t.signatureMismatch === true;
  if (t !== null && !mismatch && t.sentence !== null) choose(t.sentence, "template");
  const outweighs = f !== null && t !== null && WEAK_TEMPLATES.has(t.name) && f.strength >= WEAK_TEMPLATE_YIELDS_AT;
  if (f !== null && t !== null && !isHalfMatch(f) && (fileAgrees || outweighs)) choose(f.message, "file");
  if (t !== null && !mismatch && templateName !== null) choose(templateName, "template");
  // A rule that compared fewer than four bytes is ambiguous, and the file's
  // name settles it: a signature that lists the extension the file has names
  // it instead, unless the rule lists that extension too, in which case they
  // are one answer and the rule's sentence says more. The XENIX rule is two
  // bytes, `80 05`, and says nothing about a file that is called something else.
  const best = namingMatch(sigs);
  const ruleIsWeak = f !== null && f.strength < WEAK_TEMPLATE_YIELDS_AT;
  const sameFormat = f !== null && best !== null && f.ext.some((e) => (best.format.ext ?? []).includes(e.toLowerCase()));
  if (ruleIsWeak && best !== null && best.extensionAgrees && !sameFormat) choose(best.format.label, "signature");
  if (f !== null) choose(trimmed(f), "file");
  const tool = tools[0];
  if (tool !== undefined) choose(`${nameAndVersion(tool)} (${tool.category})`, "tools");
  if (best !== null) choose(best.format.label, "signature");
  if (t !== null && templateName !== null) choose(templateName, "template");

  if (t !== null) {
    candidates.push({
      source: "template",
      name: templateName ?? t.label,
      // A template whose signature does not match read the file's structure
      // in the sense that it laid its fields over the bytes; it did not
      // recognise the file, and the answer says which of the two happened.
      evidence: t.signatureMismatch === true ? SIGNATURE_MISMATCH(t.label) : "Qubero read the file's structure",
      disagrees: t.signatureMismatch === true || (f !== null && !fileAgrees && !isHalfMatch(f) && source === "file"),
    });
  }
  if (f !== null) {
    candidates.push({
      source: "file",
      name: trimmed(f),
      evidence: isHalfMatch(f) ? "only the first line of a file(1) rule matched" : fileEvidence(f),
      disagrees: t !== null && !fileAgrees && source !== "file",
    });
  }
  for (const m of tools) {
    candidates.push({ source: "tools", name: `${nameAndVersion(m)} (${m.category})`, evidence: `Detect It Easy (${m.source})`, disagrees: false });
  }
  for (const m of sigs) {
    candidates.push({
      source: "signature",
      name: m.format.label,
      evidence: `${bytesAt(m)}${m.extensionAgrees ? ", extension agrees" : ""}`,
      disagrees: false,
    });
  }
  // The chosen answer first, whichever block offered it.
  const chosen = candidates.findIndex((c) => c.source === source && c.name === name);
  if (chosen > 0) {
    const [c] = candidates.splice(chosen, 1);
    if (c !== undefined) candidates.unshift(c);
  }
  return { name, source, candidates };
}
