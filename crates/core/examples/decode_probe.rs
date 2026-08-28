//! Print what each decoder makes of a few instructions, for eyeballing.
use qubero_core::code::{decode, Isa};

fn main() {
    let cases: &[(Isa, &[u8])] = &[
        (Isa::X86_64, &[0x48, 0x89, 0xe5]),
        (Isa::X86_64, &[0xe8, 0x10, 0x00, 0x00, 0x00]),
        (Isa::X86_64, &[0x0f, 0xb6, 0x44, 0x24, 0x08]),
        (Isa::X86_32, &[0x55]),
        (Isa::X86_16, &[0xcd, 0x21]),
        (Isa::Aarch64, &[0xfd, 0x7b, 0xbf, 0xa9]),
        (Isa::Aarch64, &[0x00, 0x00, 0x00, 0x94]),
        (Isa::Arm, &[0x04, 0xe0, 0x2d, 0xe5]),
        (Isa::Thumb, &[0x80, 0xb5]),
        (Isa::Riscv64, &[0x13, 0x05, 0x10, 0x00]),
        (Isa::Riscv64, &[0x01, 0x00]),
        (Isa::Riscv32, &[0x67, 0x80, 0x00, 0x00]),
    ];
    for (isa, bytes) in cases {
        let insn = decode(*isa, bytes);
        println!("{:8} {:02x?} -> {} bytes: {}", isa.name(), bytes, insn.len, insn.text);
    }
}
