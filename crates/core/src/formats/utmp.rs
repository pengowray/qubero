//! Who logged in, who tried and failed, and who is logged in now: the three
//! files (`wtmp`, `btmp`, `utmp`) that share one record layout.
//!
//! There is no header and no magic. The file is a `struct utmp` repeated
//! until it runs out, which is what lets a login program append to it with
//! one write and what makes it a file that has to be recognised by its shape
//! rather than by anything it says about itself.
//!
//! The layout here is glibc's on a 64-bit Linux, which is 384 bytes and is
//! what everything in the distributions writes. It is not the kernel's
//! business and never was: a 32-bit box, or a musl one, writes a record of a
//! different size, and a file from either reads as bytes rather than as
//! records. The two timestamps stay 32-bit even on a 64-bit machine, which is
//! a Y2038 problem the format has decided to keep.

use crate::template::{Encoding, Endian::Big, Endian::Little, Expr as E, StrLen, Template, Ty as T, Until};

/// How long one record is. Everything about recognising the file is this
/// number: a login record file is a whole number of them and nothing else.
pub const RECORD: usize = 384;

/// What a record is about. `run level` is the one that carries something
/// other than a login: its user field holds the runlevel the system changed
/// to, written as a character.
const TYPES: &[(i128, &str)] = &[
    (0, "empty"),
    (1, "run level"),
    (2, "boot time"),
    (3, "new time"),
    (4, "old time"),
    (5, "init process"),
    (6, "login process"),
    (7, "user process"),
    (8, "dead process"),
    (9, "accounting"),
];

pub fn utmp() -> Template {
    Template::new(
        "utmp",
        T::structure("LoginRecords", vec![("records", T::repeat(record(), Until::End))]),
    )
}

fn record() -> T {
    T::structure_named(
        "UtmpRecord",
        "ut_user",
        "ut_type",
        vec![
            ("ut_type", T::enumeration("UtmpType", T::u16(Little), TYPES)),
            // The compiler's, not the format's: `ut_pid` is four bytes and
            // wants to start on a four-byte boundary.
            ("padding", T::bytes(E::lit(2))),
            ("ut_pid", T::i32(Little)),
            ("ut_line", text(32)),
            // The last four characters of the terminal, which is what a
            // record is matched by when a session ends.
            ("ut_id", text(4)),
            ("ut_user", text(32)),
            ("ut_host", text(256)),
            (
                "ut_exit",
                T::inline_structure("ExitStatus", vec![("e_termination", T::u16(Little)), ("e_exit", T::u16(Little))]),
            ),
            ("ut_session", T::i32(Little)),
            (
                "ut_tv",
                T::inline_structure("UtmpTime", vec![("tv_sec", T::i32(Little)), ("tv_usec", T::i32(Little))]),
            ),
            // The address the session came from, in network order, as IPv6
            // reads it. An IPv4 login fills the first word and leaves the
            // other three at zero.
            ("ut_addr_v6", T::array(T::u32(Big), E::lit(4))),
            ("reserved", T::bytes(E::lit(20))),
        ],
    )
    .counted_as("record")
}

/// A fixed-width field holding text and then nothing: everything after the
/// first NUL is padding the writer did not clear.
fn text(size: i128) -> T {
    T::text(StrLen::Padded { size: E::lit(size), pad: 0 }, Encoding::Utf8)
}

/// Whether this file is a run of login records.
///
/// Nothing says so, so this asks whether the first record could be one and
/// whether the file is a whole number of them. A record starts with a type
/// nobody has added to since the 1990s, keeps two bytes of alignment and
/// twenty reserved bytes that glibc zeroes, and writes its terminal and its
/// user as text with a NUL after it.
///
/// `empty` is a valid type and is deliberately not accepted here: a file of
/// zeros would otherwise be login records, and so would a great many files
/// that begin with one.
pub fn is_utmp(head: &[u8], len: u64) -> bool {
    if len == 0 || len % RECORD as u64 != 0 {
        return false;
    }
    let Some(r) = head.get(..RECORD) else { return false };
    let kind = u16::from_le_bytes([r[0], r[1]]);
    if !(1..=9).contains(&kind) || r[2..4] != [0, 0] || r[364..384].iter().any(|&b| b != 0) {
        return false;
    }
    // A session that started before 1980 is a file that is not this.
    let seconds = i32::from_le_bytes([r[340], r[341], r[342], r[343]]);
    seconds > 315_532_800 && padded_text(&r[8..40]) && padded_text(&r[44..76])
}

/// Whether a fixed-width field holds text and then nothing: printable
/// characters, a NUL, and no more characters after it.
fn padded_text(field: &[u8]) -> bool {
    let Some(end) = field.iter().position(|&b| b == 0) else { return false };
    field[..end].iter().all(|&b| (0x20..0x7f).contains(&b)) && field[end..].iter().all(|&b| b == 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        document::Document,
        eval::{Evaluator, Value},
        source::MemSource,
    };

    fn record_bytes(kind: u16, line: &[u8], user: &[u8], seconds: i32) -> Vec<u8> {
        let mut v = vec![0u8; RECORD];
        v[0..2].copy_from_slice(&kind.to_le_bytes());
        v[4..8].copy_from_slice(&1234i32.to_le_bytes());
        v[8..8 + line.len()].copy_from_slice(line);
        v[44..44 + user.len()].copy_from_slice(user);
        v[340..344].copy_from_slice(&seconds.to_le_bytes());
        v
    }

    #[test]
    fn a_login_reads_as_one_record() {
        let mut v = record_bytes(2, b"~", b"reboot", 1_700_000_000);
        v.extend_from_slice(&record_bytes(7, b"pts/0", b"pengo", 1_700_000_100));
        let d = Document::new(MemSource(v));
        let mut e = Evaluator::new(utmp());
        assert_eq!(e.node(&d, &[0]).unwrap().child_count, 2);
        assert_eq!(e.node(&d, &[0, 1, 5]).unwrap().value, Value::Str("pengo".into()));
        assert_eq!(e.node(&d, &[0, 1, 3]).unwrap().value, Value::Str("pts/0".into()));
        assert_eq!(e.node(&d, &[0, 1, 9, 0]).unwrap().value.as_int(), Some(1_700_000_100));
        // Every field of the record is accounted for, to the byte.
        assert_eq!(e.node(&d, &[0, 1]).unwrap().size_bits, RECORD as u64 * 8);
    }

    #[test]
    fn a_file_that_is_not_records_is_turned_away() {
        let good = record_bytes(7, b"pts/0", b"pengo", 1_700_000_000);
        assert!(is_utmp(&good, RECORD as u64));
        // Whole records or nothing.
        assert!(!is_utmp(&good, RECORD as u64 + 1));
        // A file of zeros is `empty` records, which is anything at all.
        assert!(!is_utmp(&vec![0; RECORD], RECORD as u64));
        // Text where the terminal goes, and a NUL after it.
        let mut binary = good.clone();
        binary[8..40].copy_from_slice(&[0xff; 32]);
        assert!(!is_utmp(&binary, RECORD as u64));
        // The reserved bytes at the end are glibc's and are zero.
        let mut used = good.clone();
        used[380] = 1;
        assert!(!is_utmp(&used, RECORD as u64));
    }
}
