#!/usr/bin/env python3
"""Build an answer key for the two machines in a Raspberry Pi Pico 2.

A boot ROM only uses the instructions it happens to need. To say a decoder
reads a machine's whole instruction set, the set has to be written out and
assembled, and the assembler's own listing kept as what the bytes mean.

This writes one source file per machine, assembles it with clang, disassembles
it with llvm-objdump, and leaves the listing beside the sample collection where
`cargo run --example dis_diff` can measure a decoder against it.

    python tools/isa-sweep.py [--out ../qubero-samples/pico]

Needs clang and llvm-objdump on the path, or in the usual place on Windows.
"""

import argparse
import os
import shutil
import subprocess
import sys
import tempfile

# Every register pairing would be a million lines and prove nothing new: what
# an encoding gets wrong is the shape of a field, not which register went in
# it. So each instruction appears once or twice, with operands chosen to put
# something in every field.

ARM = r"""
.syntax unified
.thumb
.text

@ ---- moving constants about, which is where a shifted field goes wrong ----
movw r3, #31956
movw r0, #0
movw lr, #65535
movt r0, #28671
movt r9, #1
mov.w r0, #0x00ff00ff
mov.w r1, #0xff00ff00
mov.w r2, #0xffffffff
mov.w r3, #0x1fe
mvn.w r0, #9
mvns.w r7, #0
mvn.w r4, #0xff00ff00
movs r0, r2
mov r8, r9
adr r0, .Lhere
add r5, pc, #300

@ ---- arithmetic ----
add.w r0, r1, #0x100
adds.w r0, r1, #0x100
addw r5, r1, #99
sub.w r0, r1, #0x100
subw r5, r1, #7
rsb.w r0, r1, #4
adc.w r0, r1, r2
sbc.w r0, r1, r2
add.w r0, r1, r2, lsl #3
sub.w r0, r1, r2, asr #7
and.w r0, r1, r2, ror #1
orr.w r0, r1, #0x80000000
orn.w r0, r1, #7
eor.w r0, r1, r2
bic.w r0, r1, r2
teq.w r1, #4
tst.w r1, r2
cmp.w r1, #0xff
cmn.w r1, r2
mul r0, r1, r2
mla r0, r1, r2, r3
mls r0, r1, r2, r3
umull r0, r1, r2, r3
smull r0, r1, r2, r3
umlal r0, r1, r2, r3
smlal r0, r1, r2, r3
sdiv r0, r1, r2
udiv r0, r1, r2

@ ---- shifts, bit fields and reversals ----
lsl.w r0, r1, #31
lsr.w r0, r1, #1
asr.w r0, r1, #16
ror.w r0, r1, #8
rrx r0, r1
lsl.w r0, r1, r2
bfi r0, r1, #4, #8
bfc r0, #4, #8
sbfx r0, r1, #4, #8
ubfx r0, r1, #4, #8
clz r0, r1
rbit r0, r1
rev r0, r1
rev16 r0, r1
revsh r0, r1
sxtb r0, r1
sxth r0, r1
uxtb r0, r1
uxth r0, r1
sxtb.w r0, r1, ror #8
uxtab r0, r1, r2
uxtah r0, r1, r2

@ ---- loads and stores, in every addressing mode ----
ldr r0, [r1]
ldr r0, [r1, #4]
ldr.w r0, [r1, #4]
ldr r0, [r1, #-4]
ldr.w r0, [r1, #4]!
ldr.w r0, [r1], #4
ldr.w r0, [r1, r2, lsl #2]
ldr r0, .Lhere
str r0, [r1, #4]
str.w lr, [r4, #4]
str.w r7, [ip]
str r0, [r1, #-8]
str.w r0, [r1, #8]!
str.w r0, [r1], #8
strb.w lr, [r4, #48]
strh.w lr, [r4, #48]
ldrb.w r0, [r1, #48]
ldrh.w r0, [r1, #48]
ldrsb.w r0, [r1, #48]
ldrsh.w r0, [r1, #48]
ldrd r0, r1, [r2, #8]
strd r0, r1, [r2, #8]
push {r4, r5, lr}
pop {r4, r5, pc}
push.w {r4, r5, r8, r9, lr}
pop.w {r4, r5, r8, r9, pc}
ldm r0!, {r1, r2}
stm r0!, {r1, r2}
ldmdb r0, {r1, r2}
stmdb r0!, {r1, r2}

@ ---- exclusives and acquire-release, which is how a lock is taken ----
ldrex r0, [r1]
ldrex r0, [r1, #16]
strex r0, r1, [r2]
ldrexb r0, [r1]
ldrexh r0, [r1]
strexb r0, r1, [r2]
strexh r0, r1, [r2]
clrex
lda r0, [r1]
ldab r0, [r1]
ldah r0, [r1]
stl r0, [r1]
stlb r0, [r1]
stlh r0, [r1]
ldaex r0, [r1]
ldaexb r0, [r1]
ldaexh r0, [r1]
stlex r0, r1, [r2]
stlexb r0, r1, [r2]
stlexh r0, r1, [r2]

@ ---- branches, and the tables a switch statement compiles to ----
.Lhere:
b .Lhere
b.w .Lhere
beq .Lhere
bne.w .Lhere
bl .Lhere
blx r0
bx lr
cbz r0, .Lfwd
cbnz r7, .Lfwd
.Lfwd:
tbb [r0, r1]
tbh [r0, r1, lsl #1]
it eq
moveq r0, r1
ite ne
movne r0, r1
moveq r0, r2

@ ---- the processor's own state ----
mrs r0, PRIMASK
mrs r5, MSPLIM
mrs ip, CONTROL
msr BASEPRI, r0
msr MSPLIM, r6
msr PRIMASK, r1
cpsid i
cpsie f
dsb sy
dmb sy
isb sy
nop
wfi
wfe
sev
yield
bkpt #0
svc #1
udf #0

@ ---- ARMv8-M security ----
sg
bxns r0
blxns r0
tt r0, r1
ttt r2, r5
tta r4, r0
ttat r1, r0

@ ---- the digital signal instructions a Cortex-M33 has ----
qadd r0, r1, r2
qsub r0, r1, r2
qdadd r0, r1, r2
qdsub r0, r1, r2
qadd8 r0, r1, r2
qadd16 r0, r1, r2
qsub8 r0, r1, r2
qsub16 r0, r1, r2
sadd8 r0, r1, r2
sadd16 r0, r1, r2
ssub8 r0, r1, r2
ssub16 r0, r1, r2
uadd8 r0, r1, r2
uadd16 r0, r1, r2
usub8 r0, r1, r2
usub16 r0, r1, r2
uqadd8 r0, r1, r2
uqsub8 r0, r1, r2
uhadd8 r0, r1, r2
uhsub8 r0, r1, r2
shadd8 r0, r1, r2
shsub8 r0, r1, r2
sasx r0, r1, r2
ssax r0, r1, r2
uasx r0, r1, r2
usax r0, r1, r2
sel r0, r1, r2
pkhbt r0, r1, r2
pkhtb r0, r1, r2, asr #8
usad8 r0, r1, r2
usada8 r0, r1, r2, r3
ssat r0, #8, r1
ssat16 r0, #8, r1
usat r0, #8, r1
usat16 r0, #8, r1
smlad r0, r1, r2, r3
smladx r0, r1, r2, r3
smlsd r0, r1, r2, r3
smuad r0, r1, r2
smusd r0, r1, r2
smmul r0, r1, r2
smmla r0, r1, r2, r3
smmls r0, r1, r2, r3
smulbb r0, r1, r2
smulbt r0, r1, r2
smultt r0, r1, r2
smlabb r0, r1, r2, r3
smlawb r0, r1, r2, r3
smulwb r0, r1, r2
smlalbb r0, r1, r2, r3
smlald r0, r1, r2, r3
smlsld r0, r1, r2, r3

@ ---- the floating point unit ----
vadd.f32 s0, s1, s2
vsub.f32 s3, s4, s5
vmul.f32 s0, s1, s2
vdiv.f32 s0, s1, s2
vnmul.f32 s0, s1, s2
vmla.f32 s0, s1, s2
vmls.f32 s0, s1, s2
vfma.f32 s0, s1, s2
vfms.f32 s0, s1, s2
vabs.f32 s0, s1
vneg.f32 s0, s1
vsqrt.f32 s0, s1
vcmp.f32 s0, s1
vcmpe.f32 s0, #0
vmov.f32 s0, s1
vmov.f32 s0, #1.0
vmov r0, s1
vmov s1, r0
vmov r0, r1, s2, s3
vcvt.s32.f32 s0, s1
vcvt.u32.f32 s0, s1
vcvt.f32.s32 s0, s1
vcvt.f32.u32 s0, s1
vcvtr.s32.f32 s0, s1
vcvtb.f32.f16 s0, s1
vcvtt.f16.f32 s0, s1
vrinta.f32 s0, s1
vrintz.f32 s0, s1
vrintx.f32 s0, s1
vmaxnm.f32 s0, s1, s2
vminnm.f32 s0, s1, s2
vseleq.f32 s0, s1, s2
vldr s0, [r0]
vldr s0, [r0, #4]
vstr s0, [r0, #-4]
vldmia r0!, {s0, s1, s2}
vstmia r0!, {s0, s1, s2}
vpush {s0, s1}
vpop {s0, s1}
vmrs apsr_nzcv, fpscr
vmrs r0, fpscr
vmsr fpscr, r0
vlldm r0
vlstm r0
"""


CORTEX_M0 = r"""
.syntax unified
.thumb
.text

@ The whole of ARMv6-M, which is what an original Pico runs: the two-byte
@ encodings, and the handful of four-byte ones that would not fit in them.
.Lhere:
lsls r0, r1, #3
lsrs r0, r1, #3
asrs r0, r1, #3
adds r0, r1, r2
subs r0, r1, r2
adds r0, r1, #3
subs r0, r1, #3
movs r0, #200
cmp r0, #200
adds r0, #200
subs r0, #200
ands r0, r1
eors r0, r1
lsls r0, r1
lsrs r0, r1
asrs r0, r1
adcs r0, r1
sbcs r0, r1
rors r0, r1
tst r0, r1
rsbs r0, r1, #0
cmp r0, r1
cmn r0, r1
orrs r0, r1
muls r0, r1
bics r0, r1
mvns r0, r1
add r8, r9
cmp r8, r9
mov r8, r9
bx lr
blx r0
ldr r0, .Lpool
str r0, [r1, r2]
strh r0, [r1, r2]
strb r0, [r1, r2]
ldrsb r0, [r1, r2]
ldr r0, [r1, r2]
ldrh r0, [r1, r2]
ldrb r0, [r1, r2]
ldrsh r0, [r1, r2]
str r0, [r1, #4]
ldr r0, [r1, #4]
strb r0, [r1, #1]
ldrb r0, [r1, #1]
strh r0, [r1, #2]
ldrh r0, [r1, #2]
str r0, [sp, #4]
ldr r0, [sp, #4]
adr r0, .Lpool
add r0, sp, #8
add sp, #8
sub sp, #8
sxth r0, r1
sxtb r0, r1
uxth r0, r1
uxtb r0, r1
push {r0, r1, lr}
pop {r0, r1, pc}
cpsid i
cpsie i
rev r0, r1
rev16 r0, r1
revsh r0, r1
bkpt #0
nop
yield
wfe
wfi
sev
stm r0!, {r1, r2}
ldm r0!, {r1, r2}
beq .Lhere
bne .Lhere
bhi .Lhere
svc #1
b .Lhere
bl .Lhere
dsb sy
dmb sy
isb sy
mrs r0, PRIMASK
msr PRIMASK, r0
udf #0
@ A constant to load: on this machine a literal load only reaches forwards,
@ so the pool has to sit after the instructions that read it.
.p2align 2
.Lpool:
.word 0
"""

RISCV = r"""
.option arch, rv32i2p1_m2p0_a2p1_zicsr2p0_zifencei2p0_zba1p0_zbb1p0_zbs1p0_zbkb1p0_zca1p0_zcb1p0_zcmp1p0
.text

# ---- the base integer set ----
lui a0, 0xb
auipc a0, 0x1
.Lhere:
jal ra, .Lhere
jalr a0, 8(a1)
beq a0, a1, .Lhere
bne a0, a1, .Lhere
blt a0, a1, .Lhere
bge a0, a1, .Lhere
bltu a0, a1, .Lhere
bgeu a0, a1, .Lhere
lb a0, 4(a1)
lh a0, 4(a1)
lw a0, 4(a1)
lbu a0, 4(a1)
lhu a0, 4(a1)
sb a0, 4(a1)
sh a0, 4(a1)
sw a0, 4(a1)
addi a0, a1, -2048
slti a0, a1, 5
sltiu a0, a1, 5
xori a0, a1, 5
ori a0, a1, 5
andi a0, a1, 5
slli a0, a1, 31
srli a0, a1, 1
srai a0, a1, 16
add a0, a1, a2
sub a0, a1, a2
sll a0, a1, a2
slt a0, a1, a2
sltu a0, a1, a2
xor a0, a1, a2
srl a0, a1, a2
sra a0, a1, a2
or a0, a1, a2
and a0, a1, a2
fence rw, rw
fence.i
ecall
ebreak
mret
wfi

# ---- multiply, divide, and the control registers ----
mul a0, a1, a2
mulh a0, a1, a2
mulhsu a0, a1, a2
mulhu a0, a1, a2
div a0, a1, a2
divu a0, a1, a2
rem a0, a1, a2
remu a0, a1, a2
csrrw a0, mstatus, a1
csrrs a0, mie, a1
csrrc a0, mip, a1
csrrwi a0, mtvec, 5
csrrsi a0, mepc, 5
csrrci a0, mcause, 5
csrr a7, mhartid
csrw 0xbe0, a0

# ---- the atomics, which is how two cores share ----
lr.w a0, (a1)
lr.w.aq a0, (a1)
sc.w a0, a2, (a1)
sc.w.rl a0, a2, (a1)
amoswap.w a0, a2, (a1)
amoadd.w a0, a2, (a1)
amoxor.w a0, a2, (a1)
amoand.w a0, a2, (a1)
amoor.w a0, a2, (a1)
amomin.w a0, a2, (a1)
amomax.w a0, a2, (a1)
amominu.w a0, a2, (a1)
amomaxu.w a0, a2, (a1)
amoadd.w.aqrl a0, a2, (a1)

# ---- address generation, and the bit manipulation sets ----
sh1add a0, a1, a2
sh2add a0, a1, a2
sh3add a0, a1, a2
andn a0, a1, a2
orn a0, a1, a2
xnor a0, a1, a2
clz a0, a1
ctz a0, a1
cpop a0, a1
max a0, a1, a2
maxu a0, a1, a2
min a0, a1, a2
minu a0, a1, a2
sext.b a0, a1
sext.h a0, a1
zext.h a0, a1
rol a0, a1, a2
ror a0, a1, a2
rori a0, a1, 16
orc.b a0, a1
rev8 a0, a1
bclr a0, a1, a2
bclri a0, a1, 11
bext a0, a1, a2
bexti a0, a1, 8
binv a0, a1, a2
binvi a0, a1, 11
bset a0, a1, a2
bseti a0, a1, 11
pack a0, a1, a2
packh a0, a1, a2
brev8 a0, a1
zip a0, a1
unzip a0, a1

# ---- the compressed encodings, including the ones Zcb and Zcmp add ----
c.addi4spn a0, sp, 16
c.lw a0, 4(a1)
c.sw a0, 4(a1)
c.nop
c.addi a0, 1
c.jal .Lhere
c.li a0, 14
c.addi16sp sp, -48
c.lui a0, 0xb
c.srli a0, 3
c.srai a0, 3
c.andi a0, -4
c.sub a0, a1
c.xor a0, a1
c.or a0, a1
c.and a0, a1
c.j .Lhere
c.beqz a0, .Lhere
c.bnez a0, .Lhere
c.slli a0, 2
c.lwsp a0, 4(sp)
c.jr a0
c.mv a0, a1
c.ebreak
c.jalr a0
c.add a0, a1
c.swsp a0, 4(sp)
c.lbu a0, 2(a1)
c.lhu a0, 2(a1)
c.lh a0, 2(a1)
c.sb a0, 1(a1)
c.sh a0, 2(a1)
c.zext.b a0
c.sext.b a0
c.zext.h a0
c.sext.h a0
c.not a0
c.mul a0, a1
cm.push {ra, s0-s2}, -16
cm.pop {ra, s0-s2}, 16
cm.popret {ra, s0-s2}, 64
cm.popretz {ra}, 16
cm.mvsa01 s0, s1
cm.mva01s s0, s1

# ---- Hazard3's own, which no assembler knows by name ----
.insn r 0x0b, 0, 0x08, a0, a1, a2   # h3.bextm a0, a1, a2, 3
.insn i 0x0b, 4, a3, a5, 0x083      # h3.bextmi a3, a5, 3, 3
slt x0, x0, x0                      # h3.block
slt x0, x0, x1                      # h3.unblock
"""


def tool(name: str) -> str:
    found = shutil.which(name)
    if found:
        return found
    usual = rf"C:\Program Files\LLVM\bin\{name}.exe"
    if os.path.exists(usual):
        return usual
    sys.exit(f"{name} not found; install LLVM or put it on the path")


def sweep(work: str, out: str, name: str, source: str, clang_args: list, objdump_args: list) -> None:
    asm = os.path.join(work, f"{name}.s")
    obj = os.path.join(work, f"{name}.o")
    with open(asm, "w", newline="\n") as f:
        f.write(source)
    subprocess.run([tool("clang"), *clang_args, "-c", asm, "-o", obj], check=True)
    listing = subprocess.run(
        [tool("llvm-objdump"), "-d", *objdump_args, obj],
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    path = os.path.join(out, f"{name}.dis")
    with open(path, "w", newline="\n") as f:
        f.write(listing)
    count = sum(1 for line in listing.splitlines() if ":" in line and "\t" in line)
    print(f"{path}: {count} instructions")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", default=os.path.join("..", "qubero-samples", "pico"))
    args = parser.parse_args()
    os.makedirs(args.out, exist_ok=True)
    with tempfile.TemporaryDirectory() as work:
        sweep(
            work,
            args.out,
            "cortex-m33-sweep",
            ARM,
            ["-target", "thumbv8m.main-none-eabi", "-mcpu=cortex-m33", "-mfpu=fpv5-sp-d16", "-mfloat-abi=hard"],
            ["--triple=thumbv8m.main", "--mcpu=cortex-m33"],
        )
        sweep(
            work,
            args.out,
            "cortex-m0plus-sweep",
            CORTEX_M0,
            ["-target", "thumbv6m-none-eabi", "-mcpu=cortex-m0plus"],
            ["--triple=thumbv6m", "--mcpu=cortex-m0plus"],
        )
        sweep(
            work,
            args.out,
            "hazard3-sweep",
            RISCV,
            ["-target", "riscv32-none-elf", "-march=rv32imac_zba_zbb_zbs_zbkb_zcb_zcmp", "-mno-relax"],
            ["--triple=riscv32", "--mattr=+m,+a,+c,+zba,+zbb,+zbs,+zbkb,+zcb,+zcmp"],
        )


if __name__ == "__main__":
    main()
