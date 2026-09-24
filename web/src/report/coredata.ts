// The shapes of what the core answers for the report: the format profile, the
// byte ledger, the extent audit and the directories. See "Report data from the
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

/** The four stepped answers, which share one walk in the core. */
export type ReportStep = "format_profile_step" | "byte_ledger_step" | "extent_audit_step" | "directories_step";
