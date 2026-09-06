//! What a run of bytes in a pickle can turn out to be.
//!
//! A byte string in a pickle is a byte string. What it *holds* is decided by
//! the program around it, and the program says so by handing it to a callable
//! whose name is written a few opcodes earlier. [`known`](super::known) does
//! the recognising; this is the list of answers it may give, and the list is
//! shared with the template so that an answer and the type it picks cannot
//! come apart.
//!
//! Two kinds of answer, and they are the two kinds of thing anyone packs into
//! bytes:
//!
//! * **A run of one type**, which is every numpy array, every pandas column,
//!   every scipy sparse matrix's three vectors and every torch tensor. The
//!   dtypes are [`npy::dtypes`], the same table the `.npy` reader uses,
//!   because a `.npy` file and a pickled array hold the same bytes described
//!   the same way and two tables would drift apart.
//!
//! * **One packed record**, which in the standard library means a date or a
//!   time. `datetime.datetime` writes its whole value as ten bytes and hands
//!   them to the class, and those ten bytes have a year, a month, a day and a
//!   microsecond count in them that nothing else will ever show.
//!
//! The index into this list is what crosses from the machine to the template,
//! as an [`Expr::Deduced`](crate::template::Expr::Deduced), so nothing is
//! reordered here without both sides moving together. Only append.

use std::sync::OnceLock;

use crate::formats::npy;
use crate::template::{Deduce, Endian::*, Expr as E, Ty as T};

/// A record the standard library packs into a byte string, in the order they
/// are appended to the case list after the dtypes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Packed {
    Date,
    Time,
    DateTime,
}

/// Every type a payload may be read as, in index order.
///
/// Built once. The dtype half is a thousand cases and the switch holding them
/// is shared rather than copied, so the cost is one table for the process
/// however many pickles are open.
pub(super) fn cases() -> Vec<T> {
    let mut out: Vec<T> = npy::dtypes()
        .into_iter()
        .map(|(_, elem, _)| T::array(elem, E::deduced(Deduce::PayloadCount)))
        .collect();
    out.push(date());
    out.push(time());
    out.push(datetime());
    out
}

/// Where a dtype sits in the list, and how wide one value of it is.
pub(super) fn dtype(descr: &str) -> Option<(usize, u64)> {
    static TABLE: OnceLock<Vec<(String, u64)>> = OnceLock::new();
    let table = TABLE.get_or_init(|| npy::dtypes().into_iter().map(|(k, _, w)| (k, w as u64)).collect());
    table.iter().position(|(k, _)| k == descr).map(|i| (i, table[i].1))
}

/// Where a packed record sits in the list, and how many bytes one is.
pub(super) fn packed(kind: Packed) -> (usize, u64) {
    static DTYPES: OnceLock<usize> = OnceLock::new();
    let after = *DTYPES.get_or_init(|| npy::dtypes().len());
    match kind {
        Packed::Date => (after, 4),
        Packed::Time => (after + 1, 6),
        Packed::DateTime => (after + 2, 10),
    }
}

/// `datetime.date`: a year, a month and a day, in four bytes.
fn date() -> T {
    T::structure("Date", vec![("year", T::u16(Big)), ("month", T::u8()), ("day", T::u8())])
}

/// `datetime.time`: an hour, a minute, a second and a microsecond count.
///
/// The top bit of the hour is `fold`, which says this is the second of the two
/// times that read alike on the night a clock goes back. One bit in a file
/// nobody looks at, and the difference between two moments an hour apart.
fn time() -> T {
    T::structure(
        "Time",
        vec![
            ("fold", T::UInt { bits: 1, endian: Big }),
            ("hour", T::UInt { bits: 7, endian: Big }),
            ("minute", T::u8()),
            ("second", T::u8()),
            ("microsecond", T::UInt { bits: 24, endian: Big }),
        ],
    )
}

/// `datetime.datetime`: the two above, run together, with `fold` in the top
/// bit of the month rather than of the hour. The two classes pack the same
/// bit in different places, which is the sort of thing a reader finds out by
/// being wrong about it.
fn datetime() -> T {
    T::structure(
        "DateTime",
        vec![
            ("year", T::u16(Big)),
            ("fold", T::UInt { bits: 1, endian: Big }),
            ("month", T::UInt { bits: 7, endian: Big }),
            ("day", T::u8()),
            ("hour", T::u8()),
            ("minute", T::u8()),
            ("second", T::u8()),
            ("microsecond", T::UInt { bits: 24, endian: Big }),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The template's cases and the machine's indices are one list, so every
    /// index the machine can hand back has to name a case.
    #[test]
    fn every_index_the_machine_gives_names_a_case() {
        let cases = cases();
        for kind in [Packed::Date, Packed::Time, Packed::DateTime] {
            let (i, _) = packed(kind);
            assert!(i < cases.len(), "{kind:?} is index {i} of {}", cases.len());
        }
        let (i, width) = dtype("<f4").expect("f4 is a dtype");
        assert!(i < cases.len());
        assert_eq!(width, 4);
    }

    /// The packed records are the widths CPython writes, and being wrong about
    /// one would read a date out of the middle of the next field.
    #[test]
    fn a_packed_record_is_as_wide_as_python_writes_it() {
        assert_eq!(packed(Packed::Date).1, 4);
        assert_eq!(packed(Packed::Time).1, 6);
        assert_eq!(packed(Packed::DateTime).1, 10);
    }
}
