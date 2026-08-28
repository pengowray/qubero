//! What machine a Microsoft object says it is for, and the instructions that
//! follow from it.
//!
//! PE and COFF number the machines the same way and mark a section of code the
//! same way, so the switch that turns one into the other is written once here.
//! ELF numbers them differently and keeps its own list.

use crate::code::Isa;
use crate::template::{Expr as E, Ty as T, Until};

/// The machines these formats name that there is a decoder for. A file for
/// anything else keeps its code as bytes.
///
/// Both Thumb entries are the two-byte encoding: `thumb` is the old name and
/// `armnt` is what Windows on ARM builds are, and neither runs the four-byte
/// one.
const MACHINES: &[(i128, Isa)] = &[
    (0x14c, Isa::X86_32),
    (0x14d, Isa::X86_32),
    (0x14e, Isa::X86_32),
    (0x1c0, Isa::Arm),
    (0x1c2, Isa::Thumb),
    (0x1c4, Isa::Thumb),
    (0x5032, Isa::Riscv32),
    (0x5064, Isa::Riscv64),
    (0x8664, Isa::X86_64),
    (0xaa64, Isa::Aarch64),
];

/// A section `size` bytes long, read as instructions when the header says the
/// section holds code and the machine is one this knows. `flags` is the
/// section's characteristics word, whose sixth bit is the one that says so.
pub(super) fn section(size: E, flags: E) -> T {
    let machines: Vec<(i128, T)> = MACHINES
        .iter()
        .map(|(machine, isa)| (*machine, T::sized(size.clone(), T::repeat(T::insn(*isa), Until::End))))
        .collect();
    T::switch(
        flags.bit(5),
        vec![(1, T::switch(E::field("machine"), machines, T::bytes(size.clone())))],
        T::bytes(size),
    )
}
