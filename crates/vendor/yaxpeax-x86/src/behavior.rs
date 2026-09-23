//! common types used for instruction behavior analysis across `yaxpeax-x86`
//!
//! these types are, in particular, used in [`yaxpeax_x86::long_mode::behavior`],
//! [`yaxpeax_x86::protected_mode::behavior`], and [`yaxpeax_x86::real_mode::behavior`]. specifics
//! like `Operand` are still mode-dependent, in part because `RegSpec` is different across modes.
//! likewise, there is no generic method to go from an `Instruction` to these kinds of accessors
//! yet. that said, `Instruction::behavior()` returns an `InstBehavior` that works effectively the
//! same way in all modes.

// a lot of people have told me that we don't need to read code anymore, just like we "never look at
// machine code anymore". sit with me and have a sad laugh for a moment. if we weren't here, reading
// and thinking and talking about how to model the computer, where would the training data come
// from? "you're not being left behind, it's a personal choice!", i've heard. a year later this has
// evolved into "it is an abdication of your responsibility as an engineer to not pay Anthropic".
// it is our ~moral responsibility~ to build the highest quality software in service of furthering
// the rot? it is an abdication of ethics to claim all works are good.

/// a collection of possible exceptions an instruction can raise. this covers the handful of
/// well-defined exception vectors with bits matching to the exception vectors listed in SDM
/// chapter 6.5.1 "Call and Return Operation for Interrupt or Exception Handling Procedures"
/// specifically "Table 6-1. Exceptions and Interrupts".
pub struct ExceptionInfo {
    possible_vectors: u32,
}

/// an individual exception vector. these are just a tiny wrapper around `u8` to have some
/// associated constant definitions.
///
/// the associated constants on this type are named according to the Intel SDM chapter 7.3 "SOURCES
/// OF INTERRUPTS" table 7-1 "Protected-Mode Exceptions and Interrupts".  similar descriptions can
/// be found in the AMD APM.
#[derive(Copy, Clone, PartialEq, Eq)]
pub struct Exception {
    vector: u8,
}

impl Exception {
    /// Divide Error
    pub const DE: Exception = Exception::vector(0);
    /// Debug
    pub const DB: Exception = Exception::vector(1);
    /// Non-Maskable Interrupt
    pub const NMI: Exception = Exception::vector(2);
    /// Breakpoint
    pub const BP: Exception = Exception::vector(3);
    /// Overflow
    pub const OF: Exception = Exception::vector(4);
    /// BOUND Range Exceeded
    pub const BR: Exception = Exception::vector(5);
    /// Invalid Opcode (Undefined Opcode)
    pub const UD: Exception = Exception::vector(6);
    /// Device Not Available (No Math Coprocessor)
    pub const NM: Exception = Exception::vector(7);
    /// Double Fault
    pub const DF: Exception = Exception::vector(8);
    // CoProcessor Segment Overrun (reserved)
    // from the SDM:
    // > IA-32 processors after the Intel386 processor do not generate this exception.
    //
    // and as the mnemonic has since been reused for exception vector 16,
    // `Floating-Point Error (Math Fault)`, we won't bother giving vector 9 a nice symbolic name.
    // const MF: Exception = Exception::vector(9);
    /// Invalid TSS
    pub const TS: Exception = Exception::vector(10);
    /// Segment Not Present
    pub const NP: Exception = Exception::vector(11);
    /// Stack Segment Fault
    pub const SS: Exception = Exception::vector(12);
    /// General Protection
    pub const GP: Exception = Exception::vector(13);
    /// Page Fault
    pub const PF: Exception = Exception::vector(14);
    // vector 15 is reserved
    /// Floating-Point Error (Math Fault)
    pub const MF: Exception = Exception::vector(16);
    /// Alignment Check
    pub const AC: Exception = Exception::vector(17);
    /// Machine Check
    pub const MC: Exception = Exception::vector(18);
    /// SIMD Floating-Point Exception
    pub const XM: Exception = Exception::vector(19);
    /// Virtualization Exception
    pub const VE: Exception = Exception::vector(20);
    /// Control Protection Exception
    pub const CP: Exception = Exception::vector(21);

    /// construct an `Exception` for the provided exception vector number.
    ///
    /// this is provided for convenience when converting (for example) the number in an x86
    /// exception handler to the kinds of `Exception` in this library.
    pub const fn vector(vector: u8) -> Self {
        Self { vector }
    }

    /// convert this `Exception` to an index into an x86 IDT.
    pub const fn to_u8(&self) -> u8 {
        self.vector
    }

    #[cfg(any(doc, feature = "fmt"))]
    /// get the typical mnemonic for this `Exception`, if one is documented.
    ///
    /// the names returned by helper do not include a leading `#`. they come from the Intel SDM
    /// chapter 7.3 "SOURCES OF INTERRUPTS" table 7-1 "Protected-Mode Exceptions and Interrupts".
    /// similar descriptions can be found in the AMD APM.
    pub fn name(&self) -> Option<&'static str> {
        static NAMES: [Option<&'static str>; 22] = [
            Some("DE"), Some("DB"), Some("NMI"), Some("BP"),
            Some("OF"), Some("BR"), Some("UD"), Some("NM"),
            Some("DF"), None, Some("TS"), Some("NP"),
            Some("SS"), Some("GP"), Some("PF"), None,
            Some("MF"), Some("AC"), Some("MC"), Some("XM"),
            Some("VE"), Some("CP")
        ];

        if let Some(maybe_name) = NAMES.get(self.vector as usize) {
            *maybe_name
        } else {
            None
        }
    }
}

#[cfg(feature = "fmt")]
use core::fmt;
#[cfg(feature = "fmt")]
impl fmt::Debug for Exception {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if let Some(name) = self.name() {
            write!(f, "#{}", name)
        } else {
            write!(f, "#Int{}", self.to_u8())
        }
    }
}

impl ExceptionInfo {
    /// construct an empty set of possible exception vectors.
    pub fn empty() -> Self {
        Self {
            possible_vectors: 0,
        }
    }

    /// test if this `ExceptionInfo` has any possible vector set.
    pub fn any(&self) -> bool {
        self.possible_vectors != 0
    }

    /// test if this `ExceptionInfo` has no vector set.
    pub fn none(&self) -> bool {
        !self.any()
    }

    /// test if this `ExceptionInfo` indicates that exception `e` may be raised.
    pub fn may(&self, e: Exception) -> bool {
        (self.possible_vectors & (1 << e.vector)) != 0
    }

    /// record that exception `e` is or is not (`b`) possible in this `ExceptionInfo` record.
    pub const fn set(&mut self, e: Exception, b: bool) {
        let offset = e.vector;
        assert!(offset < 32);
        let mask = !(1 << offset);
        let bit = (b as u32) << offset;

        self.possible_vectors &= mask;
        self.possible_vectors |= bit;
    }

    /// record that exception `e` is or is not (`b`) possible in this `ExceptionInfo` record, but
    /// in a more chaining-friendly way.
    pub const fn with(mut self, e: Exception, b: bool) -> Self {
        self.set(e, b);
        self
    }
}

#[test]
fn test_exception_info() {
    let mut info = ExceptionInfo::empty();
    info.set(Exception::MF, true);
    assert_eq!(info.possible_vectors, 0x10000);

    info.set(Exception::MF, true);
    assert_eq!(info.possible_vectors, 0x10000);

    info.set(Exception::MF, false);
    assert_eq!(info.possible_vectors, 0x00000);

    info.set(Exception::GP, false);
    assert_eq!(info.possible_vectors, 0x00000);

    info.set(Exception::GP, true);
    assert_eq!(info.possible_vectors, 0x02000);

    info.set(Exception::MF, true);
    assert_eq!(info.possible_vectors, 0x12000);
}

/// a description of the privilege level (that is, value of `CPL` in the current code selector)
/// that allows executing the corresponding instruction.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PrivilegeLevel {
    /// the corresponding instruction can run at any privilege level.
    Any = 0b00,
    /// the corresponding instruction can only run when `CPL=0` (aka "in ring 0").
    PL0 = 0b01,
    /// the corresponding instruction has more complex rule for when it is allowed.
    ///
    /// this may mean the instruction is either "Any" or "PL0" depending on other processor state
    /// (such as `rdtsc`), or it may mean the instruction simply does not relate directly to
    /// `CPL=3`/`CPL=0` (such as for `iret`).
    Special = 0b10,
}

/// a description of how an operand is used.
///
/// `Access::ReadWrite` can be processed in the same manner as that operand listed as
/// `Access::Read` followed by that same operand listed as `Access::Write`.
///
/// **important**: the meaning of `Access` is different for `flags`/`eflags`/`rflags` than other
/// operands! these differences are documented on enum variants below.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Access {
    /// the corresponding operand is read.
    ///
    /// for memory operands, this describes the referenced memory; implicitly the registers used in
    /// the operand's address calculation are also read.
    Read = 0b01,
    /// the corresponding operand is written.
    ///
    /// for memory operands, this describes the referenced memory; implicitly the registers used in
    /// the operand's address calculation are also read.
    ///
    /// for flags/eflags/rflags, "write" refers to some subset of flag bits as appropriate for the
    /// instruction, and implies that the instruction does not depend on the initial state of those
    /// bits. this is in contrast to `Write` for other operands, where it implies a full write of
    /// the corresponding operand. as a concrete example, `add` reports the flags register as a
    /// `Write` since the resulting flag bits are purely a function of the `add` register/memory
    /// operands.
    Write = 0b10,
    /// the corresponding operand is read and written.
    ///
    /// in some cases `Access::ReadWrite` is chosen in particular to represent a parital-write;
    /// this is especially true with SIMD instructions as `yaxpeax-x86` does not currently have the
    /// ability to express individual SIMD lane read/write operations. the `vmov{h,l}{ps,pd}`
    /// instructions are more common examples of this access form. this kind of partial-write
    /// access is reported as `Access::Write` for flags/eflags/rflags.
    ///
    /// for flags/eflags/rflags, "read-write" refers to some subset of flag bits as appropriate for the
    /// instruction, and implies that the instruction does depends on the initial state of those
    /// bits as well as modifying some (possibly different) bits in flags as a result.
    /// as a concrete example, `adc` reports the flags register as a `ReadWrite` because the
    /// initial state of `cf` is an input to the addition, and the normal arithmetic flags are
    /// written based on the result.
    ///
    /// for memory operands, this describes the referenced memory; implicitly the registers used in
    /// the operand's address calculation are also read.
    ReadWrite = 0b11,
    /// the corresponding operand is not actually accessed for reading or writing.
    ///
    /// this is only used to describe the operand of `nop` or `ud1` instructions.
    None = 0b00,
}

impl Access {
    // translate two bits to an `Access`. panics if the bit pattern has anything other than the low
    // two bits set. don't do that.
    pub(crate) fn from_bits(bits: u8) -> Option<Access> {
        const LUT: [Option<Access>; 4] = [
            Some(Access::None), Some(Access::Read),
            Some(Access::Write), Some(Access::ReadWrite),
        ];

        assert!(bits <= 0b11);

        LUT[bits as usize]
    }

    /// is this access a read?
    ///
    /// if it is `ReadWrite`, this will be `true` as will `is_write`.
    pub fn is_read(&self) -> bool {
        *self as u8 & 0b01 != 0
    }

    /// is this access a write?
    ///
    /// if it is `ReadWrite`, this will be `true` as will `is_read`.
    pub fn is_write(&self) -> bool {
        *self as u8 & 0b10 != 0
    }
}
