// Generates crates/core/src/formats/imxrt/registers.rs, the RT1060 register
// names the i.MX RT template gives the addresses in a DCD, from NXP's
// MCUXpresso SDK:
//
//   node tools/imxrtregisters.mjs
//   node tools/imxrtregisters.mjs path/to/MIMXRT1062.h path/to/fsl_iomuxc.h
//
// With no arguments it fetches the two files at COMMIT. Given two paths it
// reads those instead, and refuses them unless their git blob hashes are the
// ones the files have at COMMIT, so the table comes from COMMIT either way.
//
// MIMXRT1062.h is the SDK's device header. It gives each peripheral's base
// address and its registers as a C struct, with each register's offset written
// in a comment beside it. This script works the offsets out from the struct
// and stops if any of them disagrees with the comment.
//
// IOMUXC keeps most of its registers in six arrays: a mux control register
// and a pad control register for each pad, in two pairs of arrays, and a
// select input register for each input, in two more. The header names the
// elements of four of the arrays in enums. It does not name the elements of
// SW_MUX_CTL_PAD_1 and SW_PAD_CTL_PAD_1, which are the registers of the 22
// GPIO_SPI pads. fsl_iomuxc.h gives the addresses of the mux and pad control
// registers of every pad, so the names of those 44 registers come from there,
// and the script stops if fsl_iomuxc.h and the header's enums disagree about
// any other pad.
//
// What it writes:
//
// - REGISTERS, every register the header places inside the seven address
//   ranges that table 9-42 of the reference manual (IMXRT1060RM rev. 3) lets a
//   DCD write. The ranges are copied below by hand; the header does not have
//   them.
// - AREAS, the 32-bit address space in pieces, each with the peripheral whose
//   registers are in it, or none. A peripheral's piece covers its registers
//   and the reserved space between them, for every peripheral the header
//   defines.
//
// The repository was nxp-mcuxpresso/mcux-sdk when this was written and has
// since been renamed legacy-mcux-sdk; GitHub sends the old name to the new.

import { createHash } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

/** The SDK commit the table comes from. Bump the hashes with it. */
const COMMIT = "8a289764d763ad06e0c3a05c885644ed98b970af";
const SOURCE = `https://raw.githubusercontent.com/nxp-mcuxpresso/legacy-mcux-sdk/${COMMIT}`;
const FILES = {
  header: { path: "devices/MIMXRT1062/MIMXRT1062.h", blob: "1bbc601db4adf719e00e86334ec8a021df00d68e" },
  iomuxc: { path: "devices/MIMXRT1062/drivers/fsl_iomuxc.h", blob: "a6ea1f69fb64aae884688acda29ede78c5d38b81" },
};

/**
 * Table 9-42 of the reference manual, "Valid DCD address ranges": where the
 * boot ROM lets a DCD write. Each is a 16 KiB block, written here as its first
 * address and the address past its last.
 */
const DCD_RANGES = [
  [0x400a4000, 0x400a8000], // IOMUXC_SNVS_GPR
  [0x400a8000, 0x400ac000], // IOMUXC_SNVS
  [0x400ac000, 0x400b0000], // IOMUXC_GPR
  [0x401f8000, 0x401fc000], // IOMUXC
  [0x400d8000, 0x400dc000], // CCM Analog (ANATOP)
  [0x400fc000, 0x40100000], // CCM
  [0x402f0000, 0x402f4000], // SEMC
];

/**
 * In the ANATOP block five peripherals share one base address, and the
 * header gives three of them the same three registers: CCM_ANALOG_MISC0,
 * PMU_MISC0 and XTALOSC24M_MISC0 are one register. The name used is the
 * first of these the register has. CCM_ANALOG comes first because table 9-42
 * calls the block CCM Analog.
 */
const SHARED_ORDER = ["CCM_ANALOG", "PMU", "XTALOSC24M", "TEMPMON", "USB_ANALOG"];

/**
 * Arrays of register groups in the DCD ranges, and what the reference manual
 * calls each element. The header has USB_ANALOG's registers for the two USB
 * ports as INSTANCE[2]; the manual calls them USB1 and USB2, so
 * INSTANCE[1].VBUS_DETECT is USB_ANALOG_USB2_VBUS_DETECT.
 */
const GROUPS = { "USB_ANALOG.INSTANCE": (k) => `USB${k + 1}` };

/** Which header enum names the elements of each IOMUXC array that has one. */
const IOMUXC_ENUMS = {
  SW_MUX_CTL_PAD: "_iomuxc_sw_mux_ctl_pad",
  SW_PAD_CTL_PAD: "_iomuxc_sw_pad_ctl_pad",
  SELECT_INPUT: "_iomuxc_select_input",
  SELECT_INPUT_1: "_iomuxc_select_input_1",
};

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");

const gitBlob = (buf) => createHash("sha1").update(`blob ${buf.length}\0`).update(buf).digest("hex");

async function load(which, local) {
  const { path, blob } = FILES[which];
  let buf;
  if (local) {
    buf = readFileSync(local);
  } else {
    const r = await fetch(`${SOURCE}/${path}`);
    if (!r.ok) throw new Error(`${path}: ${r.status}`);
    buf = Buffer.from(await r.arrayBuffer());
  }
  const got = gitBlob(buf);
  if (got !== blob) throw new Error(`${local ?? path} has blob ${got}, not ${blob}, the one at ${COMMIT}`);
  return buf.toString("utf8");
}

const args = process.argv.slice(2);
if (args.length !== 0 && args.length !== 2) throw new Error("give no paths, or the header and fsl_iomuxc.h");
const header = await load("header", args[0]);
const iomuxcDriver = await load("iomuxc", args[1]);
const lines = header.split(/\r?\n/);

const hex = (n) => `0x${n.toString(16).padStart(8, "0")}`;
const product = (dims) => dims.reduce((a, b) => a * b, 1);

// Base addresses, and which struct each peripheral instance is.
const bases = new Map();
for (const m of header.matchAll(/^#define (\w+)\s+\((0x[0-9A-Fa-f]+)u\)/gm)) bases.set(m[1], Number(m[2]));
const instances = [];
for (const m of header.matchAll(/^#define (\w+)\s+\(\((\w+)_Type \*\)(\w+)\)/gm)) {
  const base = bases.get(m[3]);
  if (base === undefined) throw new Error(`${m[1]}: no address for ${m[3]}`);
  instances.push({ name: m[1], type: m[2], base });
}

// The register structs. A struct is a list of members: registers, padding
// named RESERVED_*, and nested structs and unions.
const MEMBER = /^\s*(?:__IO|__I|__O)?\s*(?:const\s+)?u?int(8|16|32)_t\s+(\w+)((?:\[\d+\])*)\s*;(.*)$/;
const OPEN = /^\s*(struct|union)\s*\{/;
const CLOSE = /^\s*\}\s*(\w*)\s*((?:\[\d+\])*)\s*;/;
const dimsOf = (s) => [...s.matchAll(/\[(\d+)\]/g)].map((d) => Number(d[1]));

const types = new Map();
for (let i = 0; i < lines.length; i++) {
  if (lines[i] !== "typedef struct {") continue;
  const stack = [{ kind: "struct", members: [] }];
  let j = i + 1;
  for (; j < lines.length; j++) {
    const line = lines[j];
    const top = line.match(/^\} (\w+)_Type;/);
    if (top && stack.length === 1) {
      types.set(top[1], stack[0]);
      break;
    }
    let m;
    if ((m = line.match(OPEN))) {
      stack.push({ kind: m[1], members: [], line: j + 1 });
    } else if ((m = line.match(CLOSE))) {
      const node = stack.pop();
      node.name = m[1];
      node.dims = dimsOf(m[2]);
      stack.at(-1).members.push(node);
    } else if ((m = line.match(MEMBER))) {
      stack.at(-1).members.push({ kind: "reg", bytes: Number(m[1]) / 8, name: m[2], dims: dimsOf(m[3]), comment: m[4], line: j + 1 });
    } else if (line.trim() !== "") {
      throw new Error(`line ${j + 1}: not a member: ${line}`);
    }
  }
  i = j;
}

/**
 * Lays out a struct or union from `at`, giving every member its offset and
 * size, and returns the size. Checks each register's offset against the one
 * the header's comment gives.
 */
function layout(node, at) {
  let off = 0;
  let size = 0;
  for (const m of node.members) {
    m.offset = at + (node.kind === "union" ? 0 : off);
    m.elem = m.kind === "reg" ? m.bytes : layout(m, m.offset);
    m.size = m.elem * product(m.dims);
    if (m.kind === "reg") check(m);
    if (node.kind === "union") size = Math.max(size, m.size);
    else off += m.size;
  }
  return node.kind === "union" ? size : off;
}

function check(m) {
  if (m.name.startsWith("RESERVED")) return;
  const array = m.comment.match(/array offset: (0x[0-9A-Fa-f]+), array step: (0x[0-9A-Fa-f]+)/);
  const plain = m.comment.match(/offset: (0x[0-9A-Fa-f]+)/);
  const said = array ? Number(array[1]) : plain ? Number(plain[1]) : null;
  if (said === null) throw new Error(`line ${m.line}: ${m.name} has no offset comment`);
  if (said !== m.offset) throw new Error(`line ${m.line}: ${m.name} works out at ${hex(m.offset)}, the comment says ${hex(said)}`);
  if (array && m.dims.length === 1 && Number(array[2]) !== m.elem) throw new Error(`line ${m.line}: ${m.name} step`);
}

for (const t of types.values()) t.size = layout(t, 0);

/**
 * Every register of a struct, element by element, as its offset, its size and
 * the members it is reached through. A member's offset is that of its first
 * element; `shift` is how far the elements of the arrays around it move it.
 */
function* leaves(node, shift, path) {
  for (const m of node.members) {
    if (m.name.startsWith("RESERVED")) continue;
    for (let k = 0; k < product(m.dims); k++) {
      const step = { member: m, index: m.dims.length ? k : null };
      const moved = shift + k * m.elem;
      if (m.kind === "reg") yield { offset: m.offset + moved, bytes: m.elem, path: [...path, step] };
      else yield* leaves(m, moved, [...path, step]);
    }
  }
}

// The pads fsl_iomuxc.h knows, by the address of their mux and pad control
// registers. Each pad has one GPIO function, which is the line taken.
const padsByMux = new Map();
const padsByConfig = new Map();
for (const m of iomuxcDriver.matchAll(/^#define IOMUXC_(\w+?)_GPIO\d+_IO\d+ (0x[0-9A-F]+)U, 0x[0-9A-F]+U, \w+, \w+, (0x[0-9A-F]+)U$/gm)) {
  padsByMux.set(Number(m[2]), m[1]);
  padsByConfig.set(Number(m[3]), m[1]);
}

function enumNames(tag) {
  const at = header.indexOf(`typedef enum ${tag}\n`);
  if (at < 0) throw new Error(`no enum ${tag}`);
  const body = header.slice(at, header.indexOf("}", at));
  const names = [];
  for (const m of body.matchAll(/kIOMUXC_(\w+)\s*=\s*(\d+)U/g)) {
    if (Number(m[2]) !== names.length) throw new Error(`${tag}: ${m[1]} is ${m[2]}, expected ${names.length}`);
    names.push(m[1]);
  }
  return names;
}

const inDcdRange = (a) => DCD_RANGES.some(([lo, hi]) => a >= lo && a < hi);
const padChecks = { agreed: 0, fromDriver: 0 };

/** The reference manual's name for one register of one instance. */
function registerName(inst, leaf) {
  const { member, index } = leaf.path.at(-1);
  let prefix = inst.name;
  if (leaf.path.length === 2) {
    const group = GROUPS[`${inst.name}.${leaf.path[0].member.name}`];
    if (!group || leaf.path[0].member.dims.length !== 1) throw new Error(`${inst.name}: no name for the group at ${hex(inst.base + leaf.offset)}`);
    prefix = `${inst.name}_${group(leaf.path[0].index)}`;
  } else if (leaf.path.length > 2) {
    throw new Error(`${inst.name}: nested register at ${hex(inst.base + leaf.offset)}`);
  }
  if (member.dims.length > 1) throw new Error(`${inst.name}_${member.name}: an array of arrays`);
  // The header calls USBPHY's DEBUG register DEBUGr, and says in the comment
  // beside it that the r is there so the name does not clash with a macro.
  // The manual calls it DEBUG.
  const own = member.comment.includes("'r' suffix has been added") ? member.name.replace(/r$/, "") : member.name;
  if (index === null) return `${prefix}_${own}`;
  const address = inst.base + leaf.offset;
  if (inst.name === "IOMUXC") {
    const fromDriver = member.name.startsWith("SW_MUX_CTL_PAD")
      ? padsByMux.get(address)
      : member.name.startsWith("SW_PAD_CTL_PAD")
        ? padsByConfig.get(address)
        : undefined;
    const kind = member.name.replace(/_1$/, "");
    const tag = IOMUXC_ENUMS[member.name];
    if (tag) {
      const name = enumNames(tag)[index];
      if (name === undefined) throw new Error(`${member.name}[${index}] has no enum name`);
      if (fromDriver !== undefined) {
        if (`${kind}_${fromDriver}` !== name) throw new Error(`${hex(address)}: header says ${name}, fsl_iomuxc.h says ${fromDriver}`);
        padChecks.agreed++;
      }
      return `IOMUXC_${name}`;
    }
    if (fromDriver === undefined) throw new Error(`${member.name}[${index}] at ${hex(address)}: no pad in fsl_iomuxc.h`);
    padChecks.fromDriver++;
    return `IOMUXC_${kind}_${fromDriver}`;
  }
  return `${inst.name}_${member.name}${index}`;
}

// Registers in the DCD ranges by address, and every register of every
// peripheral as the bytes it takes.
const named = new Map(); // address -> [{ instance, name }]
const cellsAt = new Map(); // address -> [{ lo, hi, holder }]
for (const inst of instances) {
  const type = types.get(inst.type);
  if (!type) throw new Error(`${inst.name}: no struct ${inst.type}_Type`);
  for (const leaf of leaves(type, 0, [])) {
    const address = inst.base + leaf.offset;
    const cells = cellsAt.get(address) ?? [];
    cells.push({ lo: address, hi: address + leaf.bytes, holder: inst.name });
    cellsAt.set(address, cells);
    if (!inDcdRange(address)) continue;
    const list = named.get(address) ?? [];
    list.push({ instance: inst.name, name: registerName(inst, leaf) });
    named.set(address, list);
  }
}

// The IOMUXC_SNVS pads, which the header names one register at a time,
// checked against fsl_iomuxc.h the same way.
for (const [address, list] of named) {
  if (address < 0x400a8000 || address >= 0x400ac000) continue;
  const pad = padsByMux.get(address) ?? padsByConfig.get(address);
  if (pad === undefined) continue;
  const kind = padsByMux.has(address) ? "SW_MUX_CTL_PAD" : "SW_PAD_CTL_PAD";
  const want = `IOMUXC_SNVS_${kind}_${pad.replace(/^SNVS_/, "")}`;
  if (list[0].name !== want) throw new Error(`${hex(address)}: header says ${list[0].name}, fsl_iomuxc.h says ${pad}`);
  padChecks.agreed++;
}

/** Which of several peripherals that put a register at one address keeps it. */
function preferred(address, holders) {
  const rank = (name) => {
    const r = SHARED_ORDER.indexOf(name);
    if (r < 0) throw new Error(`${hex(address)}: ${holders.join(", ")} each have a register here`);
    return r;
  };
  return [...holders].sort((a, b) => rank(a) - rank(b))[0];
}

const registers = [];
const shared = [];
for (const [address, list] of [...named].sort((a, b) => a[0] - b[0])) {
  const keep = list.length > 1 ? preferred(address, list.map((e) => e.instance)) : list[0].instance;
  if (list.length > 1) shared.push(list.map((e) => e.name).join(" = "));
  registers.push([address, list.find((e) => e.instance === keep).name]);
}

// Where each peripheral's registers are. Registers of one peripheral that
// overlap or touch make one run. The gap between two runs of the same
// peripheral is reserved space in it, and any other gap belongs to none: the
// registers of USBNC1 and USBNC2 interleave, so each keeps its own registers
// and the gaps between them belong to neither.
const cells = [];
for (const [address, list] of cellsAt) {
  const holders = [...new Set(list.map((c) => c.holder))];
  const keep = holders.length > 1 ? preferred(address, holders) : holders[0];
  cells.push({ lo: address, hi: Math.max(...list.filter((c) => c.holder === keep).map((c) => c.hi)), holder: keep });
}
cells.sort((a, b) => a.lo - b.lo);
const runs = [];
for (const c of cells) {
  const last = runs.at(-1);
  if (last && c.lo <= last.hi && c.holder === last.holder) {
    last.hi = Math.max(last.hi, c.hi);
  } else if (last && c.lo < last.hi) {
    throw new Error(`${hex(c.lo)}: a ${c.holder} register inside the ${last.holder} run from ${hex(last.lo)}`);
  } else {
    runs.push({ ...c });
  }
}
const areas = [{ at: 0, holder: null }];
const put = (at, holder) => {
  if (areas.at(-1).holder !== holder) areas.push({ at, holder });
};
runs.forEach((r, i) => {
  const before = runs[i - 1];
  if (before && before.hi < r.lo) put(before.hi, before.holder === r.holder ? r.holder : null);
  put(r.lo, r.holder);
});
put(runs.at(-1).hi, null);

const quote = (s) => JSON.stringify(s);
let out = `//! RT1060 register names, generated by \`node tools/imxrtregisters.mjs\`.
//!
//! Do not edit. They come from NXP's MCUXpresso SDK at commit
//! \`${COMMIT}\`, from \`${FILES.header.path}\` and
//! \`${FILES.iomuxc.path}\`, and are used under its BSD-3-Clause licence.
//! Each name is written the way the reference manual writes it: the
//! peripheral, an underscore, and the register.

/// The ${registers.length} registers in the address ranges the boot ROM lets a DCD
/// write, by address.
pub const REGISTERS: &[(u32, &str)] = &[
`;
for (const [address, name] of registers) out += `    (${hex(address)}, ${quote(name)}),\n`;
out += `];

/// The address space in ${areas.length} pieces, in address order. Each piece starts at
/// its address and runs to the start of the next, and names the peripheral
/// whose registers, or reserved space between them, it holds. None for the
/// space outside every peripheral's registers.
pub const AREAS: &[(u32, Option<&str>)] = &[
`;
for (const a of areas) out += `    (${hex(a.at)}, ${a.holder === null ? "None" : `Some(${quote(a.holder)})`}),\n`;
out += `];\n`;

writeFileSync(join(ROOT, "crates", "core", "src", "formats", "imxrt", "registers.rs"), out, "utf8");
console.log(`${registers.length} registers, ${areas.length} areas, ${instances.length} peripherals, ${types.size} structs`);
console.log(`pads: ${padChecks.agreed} agree between the header and fsl_iomuxc.h, ${padChecks.fromDriver} named from fsl_iomuxc.h alone`);
for (const s of shared) console.log(`shared: ${s}`);
