// The shapes of what the core answers for the report: the format profile, the
// byte ledger, the extent audit, the directories, and the text found inside
// lists of numbers. See "Report data from the
// core" in docs/DESIGN-report-view.md. Offsets and sizes are bits throughout,
// and every string key here is part of the interface: the web writes the words
// for each (`report/text.ts`, `report/profiletext.ts`).

export type ProfileRow = {
  /** `number` | `text` | `varint` | `bit-field` | `enum` | `flags` | `magic` |
   *  `computed` | `opaque` | `padding` | `checksum` | `codec` | `placement` |
   *  `sizing` */
  readonly category: string;
  readonly kind: string;
  /** Bits wide for a number, 0 where the file sets it; the alignment in bytes
   *  for padding. */
  readonly width: number;
  /** `little` | `big` | `none` for a number, "" for anything else. */
  readonly order: string;
  readonly detail: string;
  readonly fields: number;
  /** Always 0 in the template's profile. */
  readonly bits: number;
  readonly unpacked_fields: number;
  readonly unpacked_bits: number;
};

export type ProfileChoice = {
  readonly key: string;
  readonly name: string;
  readonly cases: number;
  /** 0 in the template's profile. */
  readonly taken: number;
  readonly fields: number;
};

export type ProfileFacts = {
  readonly from_end: number;
  readonly placed: number;
  readonly forward: number;
  readonly backward: number;
  readonly lengths_before: number;
  readonly lengths_after: number;
  readonly every_field_follows: boolean | null;
};

export type Profile = {
  readonly done: boolean;
  readonly rows: readonly ProfileRow[];
  readonly choices: readonly ProfileChoice[];
  readonly facts: ProfileFacts;
};

export type LedgerRole = "content" | "machinery" | "padding" | "framing" | "gap";

export type LedgerRow = {
  readonly part: readonly number[];
  readonly part_name: string;
  /** Empty where no list element is above these bits. */
  readonly group: string;
  /** `key` | `case` | `name` | `type` | `none` | `other` */
  readonly group_from: string;
  readonly role: LedgerRole;
  readonly bits: number;
  readonly count: number;
  readonly zero_bits: number;
  readonly unscanned_bits: number;
  readonly first_path: readonly number[];
  readonly first_offset_bits: number;
  /** For padding, the alignment in bytes. */
  readonly align: number;
};

export type Ledger = {
  readonly done: boolean;
  readonly reached_bits: number;
  readonly file_bits: number;
  readonly counted_bits: number;
  /** Largest first. */
  readonly rows: readonly LedgerRow[];
};

export type ExtentVerdict = "fits" | "short" | "stretched" | "past-parent" | "past-file" | "unreadable";

export type ExtentCheck = {
  readonly length_path: readonly number[];
  readonly length_name: string;
  readonly length_value: number | null;
  readonly length_invalid: boolean;
  readonly part_path: readonly number[];
  readonly part_name: string;
  /** `length`: `stated` and `read` are bits. `count`: they are elements. */
  readonly role: "length" | "count";
  readonly offset_bits: number;
  readonly stated: number | null;
  readonly read: number | null;
  readonly content_bits: number | null;
  readonly room_bits: number;
  readonly space_bits: number;
  readonly adjusted: boolean;
  readonly verdict: ExtentVerdict;
  readonly why: string;
};

export type ExtentAudit = {
  readonly done: boolean;
  readonly root_end_bits: number;
  readonly space_bits: number;
  readonly root_failed: string | null;
  readonly counts: readonly { readonly verdict: ExtentVerdict; readonly count: number }[];
  readonly checks: readonly ExtentCheck[];
};

export type DirectoryTarget = {
  readonly path: readonly number[];
  readonly name: string;
  readonly offset_bits: number;
  readonly size_bits: number;
  /** `address` | `offsets` | `descriptors` */
  readonly via: string;
  /** True when these bytes are counted where they are rather than here. */
  readonly aside: boolean;
};

export type DirectoryEntry = {
  readonly path: readonly number[];
  readonly name: string;
  readonly offset_bits: number;
  readonly size_bits: number;
  readonly targets: readonly DirectoryTarget[];
};

export type Directory = {
  readonly path: readonly number[];
  readonly name: string;
  readonly elements: number;
  readonly placing: number;
  /** The first 256 elements that place something, in stored order. */
  readonly entries: readonly DirectoryEntry[];
};

export type Directories = {
  readonly done: boolean;
  readonly unexamined: number;
  readonly lists: readonly Directory[];
};

/** A run of text inside a list of numbers. */
export type TextRun = {
  readonly offset_bits: number;
  readonly size_bits: number;
  /** `ASCII` | `UTF-8` */
  readonly encoding: string;
  /** The start of the text, at most 48 characters. */
  readonly text: string;
};

/** A list the template reads as numbers, with text inside it. */
export type NumbersWithText = {
  readonly path: readonly number[];
  readonly name: string;
  /** What one element is, as the strings view says it: `i16 le`. */
  readonly what: string;
  readonly element_bits: number;
  readonly offset_bits: number;
  readonly size_bits: number;
  /** Bytes of the list scanned for text: all of it, or its first and last
   *  512 KiB when it is longer than 1 MiB, or less where the file's 4 MiB of
   *  scanning ran out. See `crates/core/src/eval/textnum.rs`. */
  readonly scanned_bytes: number;
  readonly texts: number;
  readonly text_bytes: number;
  /** The first five runs of text, in file order. */
  readonly first: readonly TextRun[];
};

export type TextInNumbers = {
  readonly done: boolean;
  /** The shortest text counted, in characters. */
  readonly min_chars: number;
  /** Bytes the template reads as numbers, and how many were scanned. */
  readonly numeric_bytes: number;
  readonly scanned_bytes: number;
  readonly runs: readonly NumbersWithText[];
};

/** The stepped answers, which share one walk in the core. */
export type ReportStep = "format_profile_step" | "byte_ledger_step" | "extent_audit_step" | "directories_step" | "text_in_numbers_step";

/** Bytes a protection's verdict rests on. */
export type HardeningEvidence = {
  /** Empty where no field covers the bytes, such as a word read out of a PE
   *  load configuration the template does not read. */
  readonly path: readonly number[];
  readonly offset_bits: number;
  readonly size_bits: number;
  /** `segment` | `section` | `header` | `dynamic` | `symbol` | `string` |
   *  `note` | `directory` | `command` | `bytes` */
  readonly what: string;
  /** The name as the file writes it: `GNU_STACK`, `BIND_NOW`,
   *  `__stack_chk_fail`, `.symtab`, `dll_characteristics`. */
  readonly name: string;
};

export type HardeningVerdict = "good" | "partial" | "bad" | "info" | "unknown" | "n/a";

/** One protection, or one fact about how the program loads. `key` and
 *  `state` are the interface: `report/hardeningtext.ts` writes the words for
 *  each. */
export type HardeningRow = {
  readonly key: string;
  readonly state: string;
  readonly verdict: HardeningVerdict;
  readonly count: number | null;
  readonly total: number | null;
  /** What `key` lists: the directories of an RPATH, the libraries needed,
   *  the checked functions a program calls. */
  readonly items: readonly string[];
  /** A second list: the functions with a checked version called unchecked. */
  readonly more: readonly string[];
  /** Most direct first. */
  readonly evidence: readonly HardeningEvidence[];
};

/** One program: the file, or one slice of a universal Mach-O. */
export type HardeningPart = {
  /** Empty for a file that holds one program; the processor's name for a
   *  slice of a universal binary. */
  readonly name: string;
  readonly path: readonly number[];
  readonly rows: readonly HardeningRow[];
};

/** What an executable was built to protect itself with, as `checksec` reads
 *  it. See `Editor.hardening`. */
export type Hardening = {
  readonly format: "elf" | "pe" | "macho";
  readonly parts: readonly HardeningPart[];
};
