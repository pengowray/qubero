//! Python's pickle: a program for a stack machine, and the file is the
//! program.
//!
//! There is no structure to a pickle in the sense the other formats here have
//! one. There is no header past two bytes, no directory, no table of contents
//! and nothing that says how long anything is. A pickle is a run of opcodes,
//! each one byte, each followed by whatever operand that byte says it takes,
//! and the last of them is `.`. Running them builds an object on a stack; the
//! object is what the file is *for*, and it is nowhere in the file.
//!
//! So what a reader can show is the program. Sixty-eight opcodes, and which
//! one a byte is decides everything about what follows it: `K` takes one byte
//! and means the number in it, `J` takes four, `\x8a` takes a length and then
//! that many bytes of a two's-complement integer, and `M` takes two. Twelve
//! opcodes take a length-prefixed run of bytes and the width of the length is
//! one, four or eight depending on which of the twelve; six take a line of
//! text ending at a newline; thirty-four take nothing at all and say all they
//! have to say in their own byte.
//!
//! That is a switch over one byte, which is what this is. The opcode's name is
//! the real one out of `pickletools`, because that is the name in the Python
//! source, in the documentation and in every disassembly anyone has ever
//! posted.
//!
//! Two things about the format are worth knowing before reading the table.
//!
//! **The protocol is not a version of the file.** `PROTO` says which protocol
//! the *writer* used, and it is a hint about what to expect rather than a rule
//! about what may appear: every opcode of every earlier protocol is still
//! legal after it, and a pickle written by hand or run through
//! `pickletools.optimize` may hold any mixture. So nothing here is switched on
//! the protocol. The opcode decides, always.
//!
//! **A frame is a real container.** Protocol 4 wraps runs of opcodes in
//! `FRAME`, which carries an eight-byte length, so that a reader can pull a
//! whole run out of a stream in one go. The opcodes inside are ordinary
//! opcodes, so a frame is read as a window holding the same run this file
//! reads at the top level, which is what [`ops`] is for. One thing does not go
//! inside: a `bytes` or `str` of 64 KB or more is written between frames
//! rather than in one, so the top level is a run of opcodes of which some
//! happen to be frames, and never a list of frames.
//!
//! Nothing marks the front of a pickle written at protocol 0 or 1: the first
//! byte is an opcode like any other, and a file of them looks like a file of
//! anything. See [`is_pickle`].

pub mod known;
pub mod machine;
pub mod shapes;

use crate::template::{Deduce, Encoding, Endian::*, Expr as E, StrLen, Template, Ty as T, Until};

/// Every opcode, by the byte that spells it, with the name Python gives it.
///
/// Generated from `pickletools.opcodes` rather than typed out: the widths and
/// the byte order of the operands are the part of this format it is easiest to
/// get quietly wrong, and the table is the one place they are written down.
const OPCODE: &[(i128, &str)] = &[
    (0x28, "MARK"),
    (0x29, "EMPTY_TUPLE"),
    (0x2e, "STOP"),
    (0x30, "POP"),
    (0x31, "POP_MARK"),
    (0x32, "DUP"),
    (0x42, "BINBYTES"),
    (0x43, "SHORT_BINBYTES"),
    (0x46, "FLOAT"),
    (0x47, "BINFLOAT"),
    (0x49, "INT"),
    (0x4a, "BININT"),
    (0x4b, "BININT1"),
    (0x4c, "LONG"),
    (0x4d, "BININT2"),
    (0x4e, "NONE"),
    (0x50, "PERSID"),
    (0x51, "BINPERSID"),
    (0x52, "REDUCE"),
    (0x53, "STRING"),
    (0x54, "BINSTRING"),
    (0x55, "SHORT_BINSTRING"),
    (0x56, "UNICODE"),
    (0x58, "BINUNICODE"),
    (0x5d, "EMPTY_LIST"),
    (0x61, "APPEND"),
    (0x62, "BUILD"),
    (0x63, "GLOBAL"),
    (0x64, "DICT"),
    (0x65, "APPENDS"),
    (0x67, "GET"),
    (0x68, "BINGET"),
    (0x69, "INST"),
    (0x6a, "LONG_BINGET"),
    (0x6c, "LIST"),
    (0x6f, "OBJ"),
    (0x70, "PUT"),
    (0x71, "BINPUT"),
    (0x72, "LONG_BINPUT"),
    (0x73, "SETITEM"),
    (0x74, "TUPLE"),
    (0x75, "SETITEMS"),
    (0x7d, "EMPTY_DICT"),
    (0x80, "PROTO"),
    (0x81, "NEWOBJ"),
    (0x82, "EXT1"),
    (0x83, "EXT2"),
    (0x84, "EXT4"),
    (0x85, "TUPLE1"),
    (0x86, "TUPLE2"),
    (0x87, "TUPLE3"),
    (0x88, "NEWTRUE"),
    (0x89, "NEWFALSE"),
    (0x8a, "LONG1"),
    (0x8b, "LONG4"),
    (0x8c, "SHORT_BINUNICODE"),
    (0x8d, "BINUNICODE8"),
    (0x8e, "BINBYTES8"),
    (0x8f, "EMPTY_SET"),
    (0x90, "ADDITEMS"),
    (0x91, "FROZENSET"),
    (0x92, "NEWOBJ_EX"),
    (0x93, "STACK_GLOBAL"),
    (0x94, "MEMOIZE"),
    (0x95, "FRAME"),
    (0x96, "BYTEARRAY8"),
    (0x97, "NEXT_BUFFER"),
    (0x98, "READONLY_BUFFER"),
];

pub fn pickle() -> Template {
    Template::new("pickle", ops()).with_type("Op", op())
}

/// A run of opcodes, ending at the `.` that stops the machine.
///
/// The same run at the top of the file and inside a frame, because they are
/// the same thing. A pickle whose `STOP` is inside its last frame, which is
/// every protocol 4 pickle, simply runs the outer list to the end of the file
/// instead: a run also stops where its container does.
fn ops() -> T {
    T::repeat(T::Named("Op".into()), Until::FieldBytes { field: "code".into(), bytes: vec![b'.'] })
}

fn op() -> T {
    T::structure_named("Op", "code", "operand", vec![("code", code()), ("operand", operand())])
        .counted_as("opcode")
}

fn code() -> T {
    T::enumeration("Opcode", T::u8(), OPCODE)
}

/// What follows the opcode byte, which the opcode byte decides.
///
/// The default is no operand, which is what thirty-four of the sixty-eight
/// take, and also what an opcode nobody here has heard of gets: a byte that is
/// not in the table is not an opcode, and the run stops at it rather than
/// reading a length out of the bytes after it.
fn operand() -> T {
    let mut cases: Vec<(i128, T)> = Vec::new();
    let mut add = |codes: &[i128], ty: T| cases.extend(codes.iter().map(|c| (*c, ty.clone())));

    // A number as wide as the opcode says, and little-endian, which every
    // number in a pickle is except one.
    add(&[0x4b, 0x80, 0x82], T::u8());
    add(&[0x4d, 0x83], T::u16(Little));
    // BININT and EXT4 are signed, and BININT is how a negative number that
    // fits in four bytes is written. Read unsigned, -1 is four thousand
    // million.
    add(&[0x4a, 0x84], T::i32(Little));

    // The one big-endian field in the format. A pickle writes every length and
    // every integer little-endian and its floats the other way round, because
    // `BINFLOAT` was defined as the bytes `struct.pack(">d")` gives.
    add(&[0x47], T::F64(Big));

    // A length and then that many bytes, at three widths. What the bytes are
    // depends on the opcode: text for the `UNICODE` family, a byte string for
    // the `BYTES` family, and for the two `STRING` ones a Python 2 `str`,
    // which is bytes that were usually text.
    add(&[0x8c], counted_text(T::u8(), Encoding::Utf8));
    add(&[0x58], counted_text(T::u32(Little), Encoding::Utf8));
    add(&[0x8d], counted_text(T::u64(Little), Encoding::Utf8));
    add(&[0x55], counted_text(T::u8(), Encoding::Unknown));
    add(&[0x54], counted_text(T::i32(Little), Encoding::Unknown));
    // The payload opcodes, whose bytes are an array when the program around
    // them says so and a run of bytes when it does not. See [`payload`].
    add(&[0x43], counted_payload(T::u8()));
    add(&[0x42], counted_payload(T::u32(Little)));
    add(&[0x8e, 0x96], counted_payload(T::u64(Little)));

    // An integer of any size at all: a length, and that many bytes of
    // two's-complement magnitude, little end first. A length of zero is the
    // number zero, which is the one integer with no bytes.
    add(&[0x8a], counted_bytes(T::u8()));
    add(&[0x8b], counted_bytes(T::i32(Little)));

    // Opcodes with nothing after them that are still worth a word: what a
    // `REDUCE` calls and what a `BUILD` fills in are on the stack rather than
    // in the file, so the operand slot they were never going to use says it.
    // No bytes, so nothing about the layout changes.
    add(&[0x52, 0x62, 0x93, 0x81, 0x92, 0x6f, 0x51], T::ComputedText(E::deduced(Deduce::Builds)));
    // The memo, whose one-byte index says nothing on its own.
    add(&[0x68, 0x71], memo_ref(T::u8()));
    add(&[0x6a, 0x72], memo_ref(T::u32(Little)));
    add(&[0x67, 0x70], memo_ref(T::decimal(line())));

    // Written as text, one value to a line. The newline belongs to the field,
    // so the opcode after it starts where the field ends.
    add(&[0x49], T::decimal(line()));
    add(&[0x46, 0x53, 0x56, 0x50], T::text(line(), Encoding::Unknown));
    add(&[0x4c], decimal_long());
    add(&[0x63, 0x69], qualified_name());

    // The frame, which is the only opcode holding other opcodes.
    add(&[0x95], frame());

    T::switch(E::field("code"), cases, T::bytes(E::lit(0)))
}

/// A line of text, ending at the newline that belongs to it. `or_end` because
/// a pickle cut off in the middle of its last line is a file to show rather
/// than a file to refuse.
fn line() -> StrLen {
    StrLen::Terminated { end: b'\n', or_end: true }
}

/// A length, and that many bytes read as text.
fn counted_text(length: T, enc: Encoding) -> T {
    T::structure_named(
        "CountedText",
        "",
        "text",
        vec![("length", length), ("text", T::text(StrLen::Fixed(E::field("length")), enc))],
    )
}

/// A length, and that many bytes. What the bytes mean is the opcode's
/// business: a byte string, a `bytearray`, or the magnitude of an integer.
fn counted_bytes(length: T) -> T {
    T::structure_named(
        "CountedBytes",
        "",
        "value",
        vec![("length", length), ("value", T::bytes(E::field("length")))],
    )
}

/// A length, and that many bytes read as whatever the program says they are.
fn counted_payload(length: T) -> T {
    T::structure_named(
        "CountedBytes",
        "",
        "value",
        vec![("length", length), ("value", T::sized(E::field("length"), payload()))],
    )
}

/// The bytes of a byte string, typed by what the pickle does with them.
///
/// This is the one place a pickle says something a listing could not work out
/// from the bytes in front of it. A `BINBYTES` is a length and a run, and the
/// run is a run: the shape, the dtype and the byte order that make it 24
/// little-endian floats are three other opcodes away, joined to it only by
/// what the stack machine does with them. So the machine is asked, and its
/// answer picks a case here.
///
/// The cases are [`npy::dtypes`], the same table the `.npy` reader uses,
/// because a pickled array and a `.npy` file hold the same bytes described the
/// same way. Bytes the machine says nothing about stay bytes, which is the
/// default and is what most byte strings in most pickles are.
fn payload() -> T {
    let cases = shapes::cases().into_iter().enumerate().map(|(i, ty)| (i as i128, ty)).collect();
    T::switch(E::deduced(Deduce::PayloadShape), cases, T::bytes(E::Remaining))
}

/// A memo index, and what is in the memo there. The index alone says nothing:
/// `BINGET 5` is a row a reader has to go looking to understand, and the memo
/// is nowhere in the file to look in.
fn memo_ref(index: T) -> T {
    T::structure_named(
        "MemoRef",
        "",
        "holds",
        vec![("index", index), ("holds", T::ComputedText(E::deduced(Deduce::Builds)))],
    )
}

/// `LONG` writes its digits, then an `L`, then the newline: the `L` is the
/// suffix Python 2 put on a long integer, and it is part of the format rather
/// than part of the number.
fn decimal_long() -> T {
    T::structure_named(
        "DecimalLong",
        "",
        "value",
        vec![
            ("value", T::decimal(StrLen::Terminated { end: b'L', or_end: true })),
            ("newline", T::magic(b"\n")),
        ],
    )
}

/// A module and a name, a line each. `GLOBAL` writes the two so that the
/// unpickler can import the first and look the second up in it, and `INST`
/// writes the same pair for a class it is about to call.
fn qualified_name() -> T {
    T::structure_named(
        "QualifiedName",
        "",
        "name",
        vec![
            ("module", T::text(line(), Encoding::Unknown)),
            ("name", T::text(line(), Encoding::Unknown)),
        ],
    )
}

/// An eight-byte length and the opcodes it covers.
///
/// The length counts the bytes after it, so the window starts where the
/// opcodes do. What is in it is a run like any other, and the `STOP` that ends
/// the whole pickle is inside the last one.
fn frame() -> T {
    T::structure_named(
        "Frame",
        "",
        "ops",
        vec![("length", T::u64(Little)), ("ops", T::sized(E::field("length"), ops()))],
    )
}

/// Whether these bytes are a pickle.
///
/// Nothing marks the front of one. Protocol 2 and up open with `PROTO`, which
/// is `\x80` and a protocol number, and that is two bytes of evidence; a
/// protocol 0 or 1 pickle opens with whatever opcode came first, which could
/// be a bracket, a letter or a full stop. So recognising one is running it:
/// the walk below reads opcode after opcode the way the machine would, and a
/// file is a pickle when the walk reaches `STOP` having used every byte.
///
/// Three allowances, and no more. A `PROTO` opener may be trusted over a
/// window that is only the front of a longer file, since the walk cannot reach
/// the end of what it has not been given. The run must use every byte: a
/// pickle is written to be read by something that stops at the full stop, so
/// bytes after it are somebody else's file. And with no opener, at least one
/// opcode has to have taken an operand.
///
/// That last one is not fussiness. Thirty-four opcodes are a single letter and
/// nothing else, `.` among them, so a short lower-case word ending in a full
/// stop walks perfectly: `data.` is DICT, APPEND, TUPLE, APPEND, STOP, and so
/// is every other word spelled out of those letters. Every protocol 0 or 1
/// pickle of anything at all has an operand in it -- the empty list is
/// `(lp0\n.`, which puts itself in the memo -- so the rule costs nothing and
/// turns away a whole class of text file.
pub(super) fn is_pickle(head: &[u8], len: u64) -> bool {
    let opener = matches!(head, [0x80, 2..=5, ..]);
    match walk(head) {
        Walk::Stopped { end, operands } => end as u64 == len && (opener || operands > 0),
        // Cut off by the window rather than by the file. Only where the file
        // really is longer than the window, and only behind an opener: a run
        // of ordinary bytes that happens to read as opcodes for eight
        // kilobytes is not evidence of anything.
        Walk::Cut => opener && len > head.len() as u64,
        Walk::No => false,
    }
}

/// Every opcode in the file, for the machine to run.
///
/// The same walk `is_pickle` does, keeping what it finds rather than counting
/// it, so there is one place that knows how wide an operand is and the machine
/// cannot drift from the listing. It stops where the walk stops: a file that
/// gives up part way is annotated as far as it got.
pub fn opcodes(bytes: &[u8]) -> Vec<machine::Op> {
    let mut out = Vec::new();
    let mut at = 0usize;
    while let Some(&code) = bytes.get(at) {
        if OPCODE.iter().all(|(c, _)| *c != code as i128) {
            break;
        }
        let from = at;
        at += 1;
        let Some(end) = operand_size(bytes, at, code) else { break };
        if end > bytes.len() {
            break;
        }
        // What the opcode is *about*, with the length that measured it left
        // out: a length prefix is how long the value is, not part of it.
        let data = (at + prefix_width(code)).min(end);
        // A trailing newline is the format's, not the value's, the same as a
        // length prefix is.
        let value_end = match line_terminated(code) {
            true => end.saturating_sub(1),
            false => end,
        };
        // A payload may be a gigabyte long and is never read as a value, so
        // only what is short enough to be one is kept.
        let operand = match value_end.saturating_sub(data) <= MOST_OPERAND {
            true => bytes[data..value_end.max(data)].to_vec(),
            false => Vec::new(),
        };
        out.push(machine::Op {
            code,
            at: from as u64,
            end: end as u64,
            data_at: data as u64,
            data_len: (end - data) as u64,
            operand,
        });
        at = end;
        if code == b'.' {
            break;
        }
    }
    out
}

/// The longest operand kept as bytes. A module name, a dtype and a line of
/// digits are all far under this; an array's data is far over, and the machine
/// wants only where it is.
const MOST_OPERAND: usize = 4096;

/// How many bytes of an operand are the length that measured the rest. Zero
/// for every opcode whose operand is not measured that way.
fn prefix_width(code: u8) -> usize {
    match code {
        0x43 | 0x55 | 0x8a | 0x8c => 1,
        0x42 | 0x54 | 0x58 | 0x8b => 4,
        0x8d | 0x8e | 0x96 => 8,
        _ => 0,
    }
}

/// Whether the operand is a line, whose newline is the format's rather than
/// the value's. `GLOBAL` and `INST` write two lines and keep the first one's
/// newline, which is what separates the module from the name.
fn line_terminated(code: u8) -> bool {
    matches!(code, 0x46 | 0x49 | 0x4c | 0x50 | 0x53 | 0x56 | 0x67 | 0x70 | 0x63 | 0x69)
}

/// How far a walk over the opcodes got.
enum Walk {
    /// Reached `STOP`, one past the full stop, having read this many operands.
    Stopped { end: usize, operands: usize },
    /// Ran out of bytes in the middle of an opcode or its operand.
    Cut,
    /// A byte that is not an opcode, or an operand that cannot be read.
    No,
}

/// Read opcode after opcode, the way the machine would.
///
/// Frames are stepped over rather than descended into: the opcodes inside one
/// are at the top level as far as this is concerned, so a frame whose length
/// is wrong shows up as the opcode after it not being one.
fn walk(bytes: &[u8]) -> Walk {
    let mut at = 0usize;
    let mut operands = 0usize;
    // A file of nothing is not a pickle, and neither is one that never stops.
    // The cap is the opcode count rather than the byte count, since the
    // shortest opcode is one byte and this is only asked of a window.
    for _ in 0..bytes.len().max(1) {
        let Some(&code) = bytes.get(at) else { return Walk::Cut };
        if OPCODE.iter().all(|(c, _)| *c != code as i128) {
            return Walk::No;
        }
        at += 1;
        if code == b'.' {
            return Walk::Stopped { end: at, operands };
        }
        let next = match operand_size(bytes, at, code) {
            Some(next) => next,
            None => return if at >= bytes.len() { Walk::Cut } else { Walk::No },
        };
        if next > at {
            operands += 1;
        }
        at = next;
        if at > bytes.len() {
            return Walk::Cut;
        }
    }
    Walk::No
}

/// Where the opcode's operand ends, or nothing when the bytes run out or the
/// operand will not read.
fn operand_size(bytes: &[u8], at: usize, code: u8) -> Option<usize> {
    let fixed = |n: usize| (at + n <= bytes.len()).then_some(at + n);
    // A length of `n` bytes and then that many. Refused when the length is
    // negative or larger than any window, so that a wrong guess about where a
    // pickle starts does not turn into a jump across the file.
    let prefixed = |n: usize| {
        let raw = bytes.get(at..at + n)?;
        let mut wide = [0u8; 8];
        wide[..n].copy_from_slice(raw);
        let length = u64::from_le_bytes(wide);
        // The signed ones cannot be negative and stay readable.
        if n == 4 && length > i32::MAX as u64 {
            return None;
        }
        usize::try_from(length).ok().and_then(|l| at.checked_add(n)?.checked_add(l))
    };
    // A line, ending at the newline that belongs to it.
    let lines = |n: usize| {
        let mut end = at;
        for _ in 0..n {
            end += bytes.get(end..)?.iter().position(|b| *b == b'\n')? + 1;
        }
        Some(end)
    };
    match code {
        0x4b | 0x68 | 0x71 | 0x80 | 0x82 => fixed(1),
        0x4d | 0x83 => fixed(2),
        0x4a | 0x6a | 0x72 | 0x84 => fixed(4),
        0x47 => fixed(8),
        0x43 | 0x55 | 0x8a | 0x8c => prefixed(1),
        0x42 | 0x54 | 0x58 | 0x8b => prefixed(4),
        0x8d | 0x8e | 0x96 => prefixed(8),
        // A frame's length is the bytes after it, and the opcodes inside are
        // read as if the frame were not there, so it costs its length field
        // and nothing more.
        0x95 => fixed(8),
        0x46 | 0x49 | 0x4c | 0x50 | 0x53 | 0x56 | 0x67 | 0x70 => lines(1),
        0x63 | 0x69 => lines(2),
        _ => Some(at),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `pickle.dumps({"a": 1}, protocol=n)` for each n, which is the shortest
    /// thing that is a whole pickle at every protocol there is.
    const EVERY_PROTOCOL: &[&[u8]] = &[
        b"(dp0\nVa\np1\nI1\ns.",
        b"}q\x00X\x01\x00\x00\x00aq\x01K\x01s.",
        b"}q\x00X\x01\x00\x00\x00aq\x01K\x01s.",
        b"}q\x00X\x01\x00\x00\x00aq\x01K\x01s.",
        b"\x80\x04\x95\x0b\x00\x00\x00\x00\x00\x00\x00}\x94\x8c\x01a\x94K\x01s.",
        b"\x80\x05\x95\x0b\x00\x00\x00\x00\x00\x00\x00}\x94\x8c\x01a\x94K\x01s.",
    ];

    #[test]
    fn a_pickle_at_any_protocol_is_recognised() {
        for (n, bytes) in EVERY_PROTOCOL.iter().enumerate() {
            assert!(is_pickle(bytes, bytes.len() as u64), "protocol {n} not recognised");
        }
    }

    /// The walk has to use every byte. A pickle with anything after its full
    /// stop is not a file this reads, and neither is one cut short.
    #[test]
    fn what_is_not_a_whole_pickle_is_not_one() {
        let whole = EVERY_PROTOCOL[4];
        assert!(!is_pickle(whole, whole.len() as u64 + 1), "a file longer than the pickle in it");
        let mut trailing = whole.to_vec();
        trailing.push(b'x');
        assert!(!is_pickle(&trailing, trailing.len() as u64));
        assert!(!is_pickle(&whole[..whole.len() - 1], whole.len() as u64 - 1), "no STOP");
    }

    /// A protocol 4 opener is enough to trust a window that stops in the
    /// middle, because the walk cannot reach an end it has not been given.
    /// Without one, a run of bytes that happen to read as opcodes is not
    /// evidence of anything.
    #[test]
    fn only_a_proto_opener_is_trusted_over_a_short_window() {
        let head = b"\x80\x04\x95\xff\xff\x00\x00\x00\x00\x00\x00}\x94\x8c\x01a\x94K\x01";
        assert!(is_pickle(head, 1 << 20));
        assert!(!is_pickle(head, head.len() as u64), "the file is not longer than the window");
        // The same opcodes with the opener taken off: still a legal run, and
        // still not enough to call a file a pickle.
        assert!(!is_pickle(&head[11..], 1 << 20));
    }

    /// Text that is not a pickle at all, which is the case that matters:
    /// protocol 0 is printable and a text file is printable.
    ///
    /// The words are the sharp end of it. Thirty-four opcodes are one letter
    /// and nothing else, so `data.` walks as five opcodes using every byte,
    /// and only having no operand in it says it is a word rather than a
    /// program.
    #[test]
    fn ordinary_text_is_not_a_pickle() {
        for text in [
            &b"hello, world\n"[..],
            b"# a magic file\n0\tstring\tGIF\tGIF image\n",
            b"{\n  \"name\": \"qubero\"\n}\n",
            b"data.",
            b"steal.",
            b"Nadal.",
            b"",
        ] {
            assert!(!is_pickle(text, text.len() as u64), "{:?} read as a pickle", &text[..text.len().min(20)]);
        }
    }

    /// Every opcode in the table is one the walk knows the width of. A byte
    /// added to `OPCODE` and forgotten in `operand_size` would read as taking
    /// no operand, and the walk would carry on into the middle of its length.
    #[test]
    fn the_walk_knows_the_width_of_every_opcode() {
        // Enough bytes after the opcode for the widest fixed operand, with a
        // newline so the text ones terminate and a zero length so the
        // prefixed ones do.
        let tail = [0u8, 0, 0, 0, 0, 0, 0, 0, b'\n', b'\n'];
        for (code, name) in OPCODE {
            let mut bytes = vec![*code as u8];
            bytes.extend_from_slice(&tail);
            assert!(operand_size(&bytes, 1, *code as u8).is_some(), "{name} has no width");
        }
    }
}
