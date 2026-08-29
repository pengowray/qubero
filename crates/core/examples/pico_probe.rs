//! What the decoders make of the instructions a Pico or Pico 2 actually runs.
use qubero_core::code::{decode, Isa};

fn main() {
    let m33: &[(&str, &[u8])] = &[
        ("push {r4,lr}",        &[0x10,0xb5]),
        ("uxtb r0,r1",          &[0xc8,0xb2]),
        ("cbz r0,.",            &[0x08,0xb1]),
        ("it eq",               &[0x08,0xbf]),
        ("wfi",                 &[0x30,0xbf]),
        ("bl .-2",              &[0xff,0xf7,0xfe,0xff]),
        ("movw r0,#0x1234",     &[0x41,0xf2,0x34,0x20]),
        ("sdiv r0,r1,r2",       &[0x91,0xfb,0xf2,0xf0]),
        ("tbb [r0,r1]",         &[0x00,0xe8,0x01,0xf0]),
        ("strex r0,r1,[r2]",    &[0x42,0xe8,0x00,0x10]),
        ("dsb sy",              &[0xbf,0xf3,0x4f,0x8f]),
        ("mrs r0,primask",      &[0xef,0xf3,0x10,0x80]),
        ("msr basepri,r0",      &[0x80,0xf3,0x11,0x88]),
        ("-- ARMv8-M --",       &[]),
        ("sg",                  &[0x7f,0xe9,0x7f,0xe9]),
        ("bxns r0",             &[0x04,0x47]),
        ("blxns r0",            &[0x84,0x47]),
        ("tt r0,r1",            &[0x41,0xe8,0x00,0x0f]),
        ("ldaex r0,[r1]",       &[0xd1,0xe8,0xef,0x0f]),
        ("-- DSP --",           &[]),
        ("qadd r0,r1,r2",       &[0x82,0xfa,0x81,0xf0]),
        ("smlad r0,r1,r2,r3",   &[0x21,0xfb,0x02,0x30]),
        ("sel r0,r1,r2",        &[0xa1,0xfa,0x82,0xf0]),
        ("uadd8 r0,r1,r2",      &[0x81,0xfa,0x42,0xf0]),
        ("pkhbt r0,r1,r2",      &[0xc1,0xea,0x02,0x00]),
        ("-- FPU --",           &[]),
        ("vadd.f32 s0,s1,s2",   &[0x30,0xee,0x81,0x0a]),
        ("vldr s0,[r0]",        &[0x90,0xed,0x00,0x0a]),
        ("vcvt.s32.f32 s0,s0",  &[0xbd,0xee,0xc0,0x0a]),
        ("vmrs apsr,fpscr",     &[0xf1,0xee,0x10,0xfa]),
    ];
    let hz: &[(&str, &[u8])] = &[
        ("addi a0,zero,1",      &[0x13,0x05,0x10,0x00]),
        ("c.nop",               &[0x01,0x00]),
        ("mul a0,a1,a2",        &[0x33,0x85,0xc5,0x02]),
        ("lr.w a0,(a1)",        &[0x2f,0xa5,0x05,0x10]),
        ("amoadd.w a0,a2,(a1)", &[0x2f,0xa5,0xc5,0x00]),
        ("csrrw a0,mstatus,a1", &[0x73,0x95,0x05,0x30]),
        ("fence.i",             &[0x0f,0x10,0x00,0x00]),
        ("wfi",                 &[0x73,0x00,0x50,0x10]),
        ("-- Zbb --",           &[]),
        ("clz a0,a1",           &[0x13,0x95,0x05,0x60]),
        ("cpop a0,a1",          &[0x13,0x95,0x25,0x60]),
        ("andn a0,a1,a2",       &[0x33,0xf5,0xc5,0x40]),
        ("min a0,a1,a2",        &[0x33,0xc5,0xc5,0x0a]),
        ("rev8 a0,a1",          &[0x13,0x95,0x85,0x69]),
        ("orc.b a0,a1",         &[0x13,0x95,0x75,0x28]),
        ("sext.b a0,a1",        &[0x13,0x95,0x45,0x60]),
        ("zext.h a0,a1",        &[0x33,0xc5,0x05,0x08]),
        ("ror a0,a1,a2",        &[0x33,0xd5,0xc5,0x60]),
        ("-- Zba --",           &[]),
        ("sh1add a0,a1,a2",     &[0x33,0xa5,0xc5,0x20]),
        ("-- Zbs --",           &[]),
        ("bset a0,a1,a2",       &[0x33,0x95,0xc5,0x28]),
        ("bexti a0,a1,5",       &[0x13,0x95,0x55,0x48]),
        ("-- Zbkb --",          &[]),
        ("pack a0,a1,a2",       &[0x33,0xc5,0xc5,0x08]),
        ("brev8 a0,a1",         &[0x13,0x95,0x75,0x68]),
        ("-- Zcb --",           &[]),
        ("c.mul a0,a1",         &[0x4d,0x9d]),
        ("c.zext.b a0",         &[0x61,0x9d]),
        ("c.lbu a0,0(a1)",      &[0x88,0x81]),
        ("-- Zcmp --",          &[]),
        ("cm.push {ra},-16",    &[0x42,0xb8]),
        ("cm.popret {ra},16",   &[0x42,0xbe]),
        ("-- Hazard3 custom --",&[]),
        ("h3.bextm a0,a1,a2,1", &[0x0b,0x85,0xc5,0x00]),
    ];
    for (title, cases) in [("Cortex-M33 (thumb)", m33), ("Hazard3 (riscv32)", hz)] {
        println!("\n=== {title} ===");
        let isa = if cases.as_ptr() == m33.as_ptr() { Isa::Thumb } else { Isa::Riscv32 };
        for (want, bytes) in cases {
            if bytes.is_empty() { println!("{want}"); continue; }
            let insn = decode(isa, bytes);
            let ok = if insn.text.starts_with("(bad)") { "MISS" } else { "    " };
            println!("  {ok} {want:24} -> {:>2}b  {}", insn.len, insn.text);
        }
    }
}
