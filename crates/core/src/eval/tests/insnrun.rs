//! A run of instructions measured from its bytes gives the answers the
//! ordinary walk gives. The ordinary walk is had on the same bytes by wrapping
//! each instruction in a condition that always holds: the element is then an
//! instruction with the same room, but not one `insnrun` measures.

use super::*;
use crate::code::Isa;

/// A fixed generator, so a failure can be run again.
struct Bytes(u64);

impl Bytes {
    fn next(&mut self) -> u16 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 16) as u16
    }

    /// `n` Thumb instructions, about a third of them four bytes long: a first
    /// halfword whose top five bits are 0b11101, 0b11110 or 0b11111.
    fn thumb(&mut self, n: usize) -> Vec<u8> {
        let mut out = Vec::new();
        for _ in 0..n {
            let r = self.next();
            if r % 3 == 0 {
                let first = ((0b11101 + self.next() % 3) << 11) | (self.next() & 0x7ff);
                out.extend(first.to_le_bytes());
                out.extend(self.next().to_le_bytes());
            } else {
                let first = ((self.next() % 0b11101) << 11) | (self.next() & 0x7ff);
                out.extend(first.to_le_bytes());
            }
        }
        out
    }

    /// `n` RISC-V instructions, about half of them the two-byte compressed
    /// form: low two bits other than 0b11.
    fn riscv(&mut self, n: usize) -> Vec<u8> {
        let mut out = Vec::new();
        for _ in 0..n {
            let r = self.next();
            if r % 2 == 0 {
                out.extend((self.next() | 3).to_le_bytes());
                out.extend(self.next().to_le_bytes());
            } else {
                out.extend((self.next() & !3 | (r >> 8) % 3).to_le_bytes());
            }
        }
        out
    }
}

/// A two-byte header, `size` bytes of code, and the bytes after them. The
/// element is a plain instruction, or with `slow` one wrapped in a condition.
fn code_template(isa: Isa, size: usize, slow: bool) -> Template {
    let insn = if slow { T::when(E::lit(1), T::insn(isa)) } else { T::insn(isa) };
    Template::new(
        "t",
        T::structure(
            "Root",
            vec![("header", T::u16(Little)), ("code", T::sized(E::lit(size as i128), T::repeat(insn, Until::End))), ("after", T::bytes(E::Remaining))],
        ),
    )
}

/// Every instruction's offset and size, as the ordinary walk places them.
fn walked(d: &Document<MemSource>, isa: Isa, size: usize) -> Vec<(u64, u64)> {
    let mut ev = Evaluator::new(code_template(isa, size, true));
    let n = ev.node(d, &[1]).unwrap().child_count;
    (0..n as usize).map(|k| ev.node(d, &[1, k]).unwrap()).map(|i| (i.offset_bits, i.size_bits)).collect()
}

/// The run measured from its bytes agrees with the ordinary walk on how many
/// instructions there are, where every one of them is, and which one each of
/// a spread of bits is in, asked in order, out of order and before the count.
fn agrees(isa: Isa, code: &[u8]) {
    for cut in 0..4 {
        let size = code.len() - cut;
        let mut bytes = vec![0xaa, 0x55];
        bytes.extend_from_slice(code);
        bytes.extend([0xee; 3]);
        let d = doc(&bytes);
        let want = walked(&d, isa, size);
        let what = format!("{} with {size} bytes of code", isa.name());
        assert!(want.len() > 2 * super::super::walk::GUARD_ABOVE, "{what}: long enough to be guarded");

        // In order, after the count.
        let mut ev = Evaluator::new(code_template(isa, size, false));
        assert_eq!(ev.node(&d, &[1]).unwrap().child_count, want.len() as u64, "{what}");
        assert!(ev.memo_len() < 10, "{what}: counted without placing anything, but the memo holds {}", ev.memo_len());
        for (k, w) in want.iter().enumerate() {
            let info = ev.node(&d, &[1, k]).unwrap();
            assert_eq!((info.offset_bits, info.size_bits), *w, "{what}: instruction {k}");
        }

        // Out of order, before anything has counted the run.
        let mut ev = Evaluator::new(code_template(isa, size, false));
        let mut pick = Bytes(size as u64 * 7919);
        for _ in 0..300 {
            let k = (pick.next() as usize * 7) % want.len();
            let info = ev.node(&d, &[1, k]).unwrap();
            assert_eq!((info.offset_bits, info.size_bits), want[k], "{what}: instruction {k} asked cold");
        }
        let last = want.len() - 1;
        assert_eq!((ev.node(&d, &[1, last]).unwrap().offset_bits, ev.node(&d, &[1, last]).unwrap().size_bits), want[last]);

        // Which instruction a bit is in, from the cursor's side, on both walks.
        let mut slow = Evaluator::new(code_template(isa, size, true));
        let mut fast = Evaluator::new(code_template(isa, size, false));
        let end = d.len_bits();
        let mut bits: Vec<u64> = (0..200).map(|_| (pick.next() as u64 * 977) % end).collect();
        bits.extend([0, 15, 16, 17, 23, 24, end - 25, end - 24, end - 17, end - 9, end - 1]);
        bits.extend(want[last - 1..].iter().flat_map(|(at, size)| [*at, at + size - 1]));
        for bit in bits {
            assert_eq!(fast.locate(&d, bit).unwrap(), slow.locate(&d, bit).unwrap(), "{what}: bit {bit}");
        }
    }
}

#[test]
fn a_run_of_thumb_measured_from_its_bytes_is_the_run_the_walk_reads() {
    let code = Bytes(0x9e37_79b9_7f4a_7c15).thumb(10_000);
    // The last instruction is four bytes long, so cutting the run one, two
    // and three bytes short leaves three, two and one byte of it.
    let mut code = code;
    code.extend([0x00, 0xf0, 0x00, 0xf8]);
    agrees(Isa::Thumb, &code);
}

#[test]
fn a_run_of_risc_v_measured_from_its_bytes_is_the_run_the_walk_reads() {
    let mut code = Bytes(0xd1b5_4a32_d192_ed03).riscv(10_000);
    // addi a0, zero, 1: four bytes, cut short as for Thumb.
    code.extend([0x13, 0x05, 0x10, 0x00]);
    agrees(Isa::Riscv32, &code);
}

#[test]
fn a_run_of_instructions_counted_in_goes_reaches_the_same_count() {
    let code = Bytes(0x1234_5678_9abc_def1).thumb(200_000);
    let mut bytes = vec![0, 0];
    bytes.extend_from_slice(&code);
    let d = doc(&bytes);
    let want = Evaluator::new(code_template(Isa::Thumb, code.len(), false)).node(&d, &[1]).unwrap().child_count;
    assert_eq!(want, 200_000);

    let mut ev = Evaluator::new(code_template(Isa::Thumb, code.len(), false));
    ev.set_slice(Some(4));
    let mut goes = 0;
    let counted = loop {
        goes += 1;
        assert!(goes < 1000, "the count is not getting anywhere");
        ev.begin_slice();
        match ev.node(&d, &[1]) {
            Ok(info) => break info.child_count,
            Err(e) if e.interrupted() => continue,
            Err(e) => panic!("{e:?}"),
        }
    };
    assert_eq!(counted, want);
    assert!(goes > 5, "the count was not interrupted, so this proves nothing");
}

#[test]
fn what_the_listing_asks_of_a_long_run_of_instructions_costs_less_than_counting_it() {
    // 200,000 instructions, about 530 KB: counting them is charged as more
    // than a hundred elements, and nothing else asked here may cost that.
    let code = Bytes(0x0bad_cafe_f00d_d00d).thumb(200_000);
    let mut bytes = vec![0, 0];
    bytes.extend_from_slice(&code);
    bytes.extend(b"after the code");
    let d = doc(&bytes);
    let middle = (2 + code.len() as u64 / 2) * 8;
    let mut ev = Evaluator::new(code_template(Isa::Thumb, code.len(), false));
    ev.set_slice(Some(8));
    ev.begin_slice();
    assert_eq!(ev.size_of(&d, &[1]).unwrap(), code.len() as u64 * 8);
    let run = ev.locate_run_under(&d, &[], middle).unwrap();
    assert_eq!(run, vec![1]);
    let read_as = ev.read_as_under(&d, &[], middle, middle + 64).unwrap().unwrap();
    assert_eq!((read_as.kind, read_as.what.as_str(), read_as.size_bits), (NotText::Code, "thumb", code.len() as u64 * 8));
    assert!(!ev.list(&[1]).repeat_done, "something above counted the run");
    // The cursor in the last instruction steps there from the start once. A
    // bit five bytes before it is then found by stepping forward from the
    // nearest checkpoint, which fits in a go of one element.
    ev.set_slice(None);
    let end = (2 + code.len() as u64) * 8 - 1;
    let last = ev.locate(&d, end).unwrap();
    assert_eq!(last[..2], [1, 199_999]);
    ev.set_slice(Some(1));
    ev.begin_slice();
    assert_eq!(ev.locate(&d, end - 40).unwrap()[0], 1);
}
