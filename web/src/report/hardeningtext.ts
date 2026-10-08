// Every string of the protections section (`hardening.ts`). The core sends a
// key and a state for each row (see `HardeningRow`); this file writes the
// words. Field terms stay as the field spells them (RELRO, PIE, NX,
// FORTIFY_SOURCE, BIND_NOW), since those are what a reader searches for and
// what `checksec` prints.

import type { HardeningEvidence, HardeningRow } from "./coredata.ts";
import { counted, listText, sentenceCase } from "./text.ts";

/** The verdicts of one program's protections, counted. Rows that do not
 *  apply, and rows that are only facts, are not counted. */
export type Tally = {
  readonly good: number;
  readonly partial: number;
  readonly bad: number;
  readonly unknown: number;
  readonly total: number;
  /** Short names of the protections in each group, in table order. */
  readonly off: readonly string[];
  readonly partly: readonly string[];
  readonly unknownKeys: readonly string[];
};

type Words = {
  /** The row's label. */
  readonly label: string;
  /** The name in a list of names, such as the line under the heading. */
  readonly short?: string;
  /** What the protection does, the same for every file. Empty for a fact. */
  readonly desc?: string;
};

const WORDS: Readonly<Record<string, Words>> = {
  // ELF protections, in the order the core sends them.
  relro: { label: "RELRO", desc: "Makes the GOT and other relocated data read-only after relocation." },
  canary: { label: "Stack canary", short: "stack canary", desc: "Guard value before the return address, checked when a function returns (-fstack-protector)." },
  nx: { label: "NX (non-executable stack)", short: "NX", desc: "Stack memory cannot be executed, so code written onto the stack cannot run." },
  pie: { label: "PIE", desc: "Position-independent executable: loads at a random address each run under ASLR." },
  fortify: { label: "FORTIFY_SOURCE", desc: "Checked libc functions like __memcpy_chk stop writes past a buffer of known size." },
  textrel: { label: "Text relocations", short: "text relocations", desc: "Relocations the loader applies by writing into code pages, which must then be writable." },
  rwx: { label: "Writable and executable segments", short: "writable and executable segments", desc: "Segments that are both writable and executable: code can be written there and then run." },
  ibt: { label: "IBT (Intel CET)", short: "IBT", desc: "Indirect calls and jumps must land on an ENDBR instruction (indirect branch tracking)." },
  shstk: { label: "SHSTK (Intel CET)", short: "SHSTK", desc: "Shadow stack: the CPU keeps its own copy of each return address and checks it on return." },
  bti: { label: "BTI (Arm)", short: "BTI", desc: "Indirect branches must land on a BTI instruction (branch target identification)." },
  pac: { label: "PAC", desc: "Return addresses are signed when saved and checked before use (pointer authentication)." },
  safestack: { label: "SafeStack (Clang)", short: "SafeStack", desc: "Moves buffers that could overflow onto a separate stack, away from return addresses." },
  cfi: { label: "CFI (Clang)", short: "CFI", desc: "Each indirect call is checked against the expected function type (control-flow integrity)." },
  // PE protections.
  aslr: { label: "ASLR (dynamic base)", short: "ASLR", desc: "Windows can load the program at a random address each run." },
  "high-entropy-va": { label: "High-entropy ASLR", short: "high-entropy ASLR", desc: "64-bit ASLR using the whole 64-bit address space (high entropy VA)." },
  dep: { label: "DEP (NX compatible)", short: "DEP", desc: "Memory that holds data cannot be executed." },
  cfg: { label: "Control Flow Guard (CFG)", short: "CFG", desc: "Indirect calls are checked against a table of valid targets." },
  gs: { label: "Stack cookie (/GS)", short: "stack cookie", desc: "The compiler's stack canary: a guard value before the return address, checked on return." },
  safeseh: { label: "SafeSEH", desc: "Exception handlers must be listed in a table (32-bit programs)." },
  // Mach-O protections.
  "nx-heap": { label: "Non-executable heap", short: "non-executable heap", desc: "Heap memory cannot be executed (MH_NO_HEAP_EXECUTION)." },
  "nx-stack": { label: "Non-executable stack", short: "non-executable stack", desc: "Stack memory cannot be executed." },
  // How the program loads.
  linking: { label: "Linking" },
  interpreter: { label: "Dynamic linker" },
  needed: { label: "Needed libraries" },
  soname: { label: "SONAME" },
  rpath: { label: "RPATH" },
  runpath: { label: "RUNPATH" },
  symbols: { label: "Symbols" },
  "build-id": { label: "Build ID" },
  signature: { label: "Authenticode signature" },
  "force-integrity": { label: "Force integrity" },
  appcontainer: { label: "AppContainer" },
  "code-signature": { label: "Code signature" },
  encrypted: { label: "Encrypted" },
  arc: { label: "Objective-C ARC" },
  restrict: { label: "__RESTRICT segment" },
};

/** What a status cell says: a word or two, then a detail where the word alone
 *  would be misread. */
export type Status = { readonly word: string; readonly detail: string };

const plain = (word: string, detail = ""): Status => ({ word, detail });

/** Why a check that needs the symbol tables cannot be made on a file whose
 *  section headers have been removed. */
const NO_SECTIONS = "No section headers, so the symbol tables cannot be found.";

/** An RPATH or RUNPATH directory searched from the current directory. */
function relative(dir: string): boolean {
  return dir !== "" && !dir.startsWith("/") && !dir.startsWith("$ORIGIN") && !dir.startsWith("${ORIGIN}");
}

function onOff(state: string): Status | null {
  if (state === "on") return plain("On");
  if (state === "off") return plain("Off");
  return null;
}

function fortifyStatus(r: HardeningRow): Status {
  const total = r.total ?? 0;
  const count = r.count ?? 0;
  switch (r.state) {
    case "yes":
      return plain("Yes", `${count.toLocaleString()} of the ${total.toLocaleString()} functions that have a checked version use it.`);
    case "no-symbols":
      return plain("Unknown", "No dynamic symbol table to search.");
    case "no-sections":
      return plain("Unknown", NO_SECTIONS);
    default:
      if (total === 0) return plain("N/A", "None of the library functions it calls have a checked version.");
      return plain("No", `${count.toLocaleString()} of the ${total.toLocaleString()} functions that have a checked version use it.`);
  }
}

function pathStatus(r: HardeningRow): Status {
  if (r.state === "none" || r.items.length === 0) return plain("None");
  const rel = r.items.filter(relative);
  const notes: string[] = [];
  if (rel.length > 0) {
    notes.push(`Relative: ${listText(rel)} ${rel.length === 1 ? "is" : "are"} searched from the current directory, so whoever controls that directory can supply a library.`);
  }
  if (r.items.some((d) => d === "")) notes.push("Empty entry: the current directory is searched, so whoever controls it can supply a library.");
  return plain("", notes.join(" "));
}

export const HV = {
  name: (key: string): string => WORDS[key]?.label ?? key,
  short: (key: string): string => WORDS[key]?.short ?? WORDS[key]?.label ?? key,
  desc: (key: string): string => WORDS[key]?.desc ?? "",

  /** Whether a loading fact's `items` are its value, drawn as a list. */
  listsItems: (key: string): boolean => ["interpreter", "needed", "soname", "rpath", "runpath", "build-id"].includes(key),
  /** The count the rest of a long list opens from. */
  itemsMore: (key: string, n: number): string => (key === "needed" ? `and ${counted(n, "more library")}` : `and ${n.toLocaleString()} more`),

  /** The words for a row's state. `linking` is the `linking` row's state, for
   *  the one protection whose meaning depends on it. */
  status(r: HardeningRow, linking: string): Status {
    const s = r.state;
    switch (r.key) {
      case "relro":
        if (s === "full") return plain("Full", "BIND_NOW set.");
        if (s === "partial") {
          return linking === "dynamic" || linking === ""
            ? plain("Partial", "No BIND_NOW, so GOT entries for library functions stay writable.")
            : plain("Partial", "GNU_RELRO segment, no BIND_NOW. Statically linked, so no library functions are resolved by a dynamic linker.");
        }
        if (s === "none") return plain("None", "No GNU_RELRO segment.");
        break;
      case "canary":
        if (s === "found") return plain("Found");
        if (s === "not-found") return plain("Not found");
        if (s === "no-symbols") return plain("Unknown", "No symbol tables to search.");
        if (s === "no-sections") return plain("Unknown", NO_SECTIONS);
        break;
      case "nx":
        if (s === "on") return plain("On");
        if (s === "off") return plain("Off", "The stack is executable: GNU_STACK has execute permission.");
        if (s === "missing") return plain("Missing", "No GNU_STACK segment, so the stack may be executable.");
        break;
      case "pie":
        if (s === "pie" || s === "on") return plain("On");
        if (s === "exec" || s === "off") return plain("Off", "Fixed load address.");
        if (s === "dso") return plain("N/A", "Shared library, always position-independent.");
        if (s === "object") return plain("N/A", "Object file, not linked yet.");
        if (s === "core") return plain("N/A", "Core dump.");
        if (s === "n/a") return plain("N/A", "Not an executable.");
        if (s === "unknown") return plain("Unknown", "The ELF header gives a file type this does not know.");
        break;
      case "fortify":
        return fortifyStatus(r);
      case "textrel":
        if (s === "none") return plain("None");
        if (s === "present") return plain("Present");
        break;
      case "rwx":
        if (s === "none") return plain("None");
        if (s === "present") return plain(counted(r.count ?? r.evidence.length, "segment"));
        break;
      case "safestack":
      case "cfi":
        if (s === "found") return plain("Found", r.key === "cfi" && r.count !== null ? `${counted(r.count, "function")} checked.` : "");
        if (s === "not-found") return plain("Not found");
        if (s === "no-symbols") return plain("Unknown", "No symbol tables to search.");
        if (s === "no-sections") return plain("Unknown", NO_SECTIONS);
        break;
      case "aslr":
        if (s === "no-relocations") return plain("No relocations", "Dynamic base flag set, but without a relocation table the load address is fixed.");
        return onOff(s) ?? plain(s);
      case "high-entropy-va":
        if (s === "n/a") return plain("N/A", "32-bit program.");
        return onOff(s) ?? plain(s);
      case "cfg":
        if (s === "on") return plain("On", r.count !== null ? `${counted(r.count, "function")} in the table.` : "");
        return onOff(s) ?? plain(s);
      case "gs":
        if (s === "found") return plain("Found", "Security cookie in the load configuration.");
        if (s === "not-found") return plain("Not found");
        if (s === "unknown") return plain("Unknown", "The load configuration could not be read.");
        if (s === "no-load-config") return plain("Unknown", "No load configuration to read the security cookie from.");
        break;
      case "safeseh":
        if (s === "on" && r.count !== null) return plain("On", `${counted(r.count, "handler")} in the table.`);
        if (s === "no-seh") return plain("No SEH", "The program declares no structured exception handlers.");
        if (s === "n/a") return plain("N/A", "64-bit programs use table-based exception handling.");
        if (s === "unknown") return plain("Unknown", "The load configuration could not be read.");
        return onOff(s) ?? plain(s);
      case "nx-heap":
        if (s === "off") return plain("Off", "64-bit macOS makes the heap non-executable by default.");
        return onOff(s) ?? plain(s);
      case "nx-stack":
        if (s === "off") return plain("Off", "MH_ALLOW_STACK_EXECUTION is set.");
        return onOff(s) ?? plain(s);

      // How the program loads.
      case "linking":
        if (s === "dynamic") return plain("Dynamic");
        if (s === "static") return plain("Static");
        if (s === "static-pie") return plain("Static PIE", "Statically linked and position-independent.");
        break;
      case "interpreter":
      case "needed":
      case "soname":
      case "build-id":
        if (s === "none" || r.items.length === 0) return plain("None");
        return plain("");
      case "rpath":
      case "runpath":
        return pathStatus(r);
      case "symbols":
        if (s === "stripped") return plain("Stripped: no .symtab", debugLine(r.items));
        if (s === "present") return plain(`${counted(r.count ?? 0, "symbol")} in .symtab`, debugLine(r.items));
        if (s === "no-sections") return plain("Unknown", NO_SECTIONS);
        break;
      case "signature":
        if (s === "present") return plain("Present", "Validity not checked.");
        if (s === "none") return plain("None");
        break;
      case "force-integrity":
        if (s === "on") return plain("On", "Windows loads this program only with a valid signature.");
        return onOff(s) ?? plain(s);
      case "appcontainer":
        if (s === "on") return plain("On", "Must run in an AppContainer sandbox.");
        return onOff(s) ?? plain(s);
      case "code-signature":
        if (s === "present") return plain("Present");
        if (s === "none") return plain("None");
        break;
      case "encrypted":
        if (s === "yes") return plain("Yes", "FairPlay (App Store).");
        if (s === "no") return plain("No", "Encryption info present, cryptid 0.");
        if (s === "none") return plain("No");
        break;
      case "arc":
        if (s === "found") return plain("Found");
        if (s === "not-found") return plain("Not found");
        if (s === "no-symbols") return plain("Unknown", "No symbol table to search.");
        break;
      case "restrict":
        if (s === "present") return plain("Present", "dyld ignores DYLD_ environment variables.");
        if (s === "none") return plain("None");
        break;
    }
    return onOff(s) ?? plain(sentenceCase(s));
  },

  /** The heading: how many protections are in place, then the rest by
   *  verdict. */
  heading(t: Tally): string {
    if (t.total === 0) return "No protections apply to this file";
    const rest: string[] = [];
    if (t.partial > 0) rest.push(`${t.partial.toLocaleString()} partial`);
    if (t.bad > 0) rest.push(`${t.bad.toLocaleString()} off`);
    if (t.unknown > 0) rest.push(`${t.unknown.toLocaleString()} unknown`);
    if (rest.length === 0) return t.total === 1 ? "1 protection, in place" : `All ${t.total.toLocaleString()} protections in place`;
    return `${t.good.toLocaleString()} of ${t.total.toLocaleString()} ${t.total === 1 ? "protection" : "protections"} in place, ${rest.join(", ")}`;
  },
  /** The heading over a universal binary, with a program for each
   *  processor, and each program's own heading under it. */
  headingSlices: (n: number, _all: Tally): string => `${n.toLocaleString()} programs in one universal binary`,
  sliceHeading: (cpu: string, t: Tally): string => `${cpu === "" ? "Program" : cpu}: ${HV.heading(t)}`,

  /** The line under the heading, naming what is not in place. */
  lede(t: Tally, _static: boolean): string {
    const parts: string[] = [];
    if (t.off.length > 0) parts.push(`Off: ${t.off.join(", ")}.`);
    if (t.partly.length > 0) parts.push(`Partial: ${t.partly.join(", ")}.`);
    if (t.unknownKeys.length > 0) parts.push(`Unknown: ${t.unknownKeys.join(", ")}.`);
    return parts.join(" ");
  },

  source: "Verdicts are read from the file's headers and symbol tables, with the same checks as checksec. Each link in the Evidence column goes to the bytes a verdict is based on.",
  fortifyNote:
    "The FORTIFY_SOURCE total counts the library functions the program calls that have a checked version in glibc 2.39 (83 functions have one). checksec counts against the libc installed where it runs, so its totals can differ.",
  hardwareNote: "IBT, SHSTK, BTI and PAC are enforced only when the processor, the kernel and every module loaded into the process support them.",

  colProtection: "Protection",
  colStatus: "Status",
  colEvidence: "Evidence",
  loadingCaption: "Linking and loading",

  fortifiedList: (n: number): string => `${counted(n, "function")} ${n === 1 ? "uses" : "use"} the checked version`,
  unfortifiedList: (n: number): string => `${counted(n, "function")} ${n === 1 ? "uses" : "use"} the plain version`,

  /** What kind of thing a piece of evidence is, after its name. */
  evidenceKind(what: string): string {
    switch (what) {
      case "segment":
        return "segment";
      case "section":
        return "section";
      case "header":
        return "header field";
      case "dynamic":
        return "dynamic entry";
      case "symbol":
        return "symbol";
      case "string":
        return "string";
      case "note":
        return "note";
      case "directory":
        return "data directory entry";
      case "command":
        return "load command";
      default:
        return "bytes";
    }
  },
  /** The bold line of a link's hover. */
  evidenceLabel: (e: HardeningEvidence): string => (e.name === "" ? sentenceCase(HV.evidenceKind(e.what)) : `${e.name} ${HV.evidenceKind(e.what)}`),
  evidenceMore: (n: number): string => `and ${n.toLocaleString()} more`,
};

function debugLine(sections: readonly string[]): string {
  return sections.length === 0 ? "" : `Debug sections: ${sections.join(", ")}`;
}
