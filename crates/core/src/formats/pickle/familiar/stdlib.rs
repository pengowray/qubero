//! The standard library's own classes: dates, ordered and defaulting
//! dictionaries, counters, queues, exact numbers, ids and paths.
//!
//! What a pickle of ordinary program state is full of, and every one of them a
//! `REDUCE` of a callable named here with the exact argument shape that
//! callable is written with. Nothing is run, and the safety line is the one
//! [`object`](super::object) holds every other form to: a module prefix says
//! which classes may be *named*, never which callables may be *called*, and a
//! `REDUCE` of anything not in the table below is a non-match however
//! respectable its module looks.
//!
//! `uuid.UUID` needs no row at all. It is `STACK_GLOBAL`, `EMPTY_TUPLE`,
//! `NEWOBJ`, a state dictionary holding one 128-bit `int`, and `BUILD`, which
//! is the object production that already exists; what it needed was an integer
//! wider than sixteen bytes, which is in [`values`](super::values).

use super::cursor::Cursor;
use super::forms::{Args, Reduce, Via};
use super::{Kind, Names, Shape, Storage, Value};

/// The modules a class may be named from under this form. `pathlib._local` is
/// under `pathlib`, which is where Python 3.13 moved the path classes.
///
/// `_datetime` and `_collections` are the spellings the accelerator modules go
/// by under the interpreters outside CPython and PyPy. No file in the
/// collection writes one; they are listed because the same classes are the
/// same classes, and a prefix test cannot reach them from the name without an
/// underscore.
pub(super) const STDLIB_MODULES: &[&str] =
    &["datetime", "_datetime", "collections", "_collections", "decimal", "fractions", "uuid", "pathlib"];

/// The classes a `collections.defaultdict` may be handed as its factory, by
/// their whole dotted path in both of the spellings `fix_imports` writes.
///
/// A factory is a callable the file names and this reader never calls, so what
/// may stand there is a list rather than a rule: a class of the writing
/// program's own, or anything else under `builtins`, is a non-match. The list
/// is the builtin containers and the builtin types their values are.
pub(super) const FACTORIES: &[&str] = &[
    "builtins.list",
    "builtins.dict",
    "builtins.set",
    "builtins.int",
    "builtins.float",
    "builtins.str",
    "builtins.tuple",
    "builtins.bool",
    "__builtin__.list",
    "__builtin__.dict",
    "__builtin__.set",
    "__builtin__.int",
    "__builtin__.float",
    "__builtin__.str",
    "__builtin__.tuple",
    "__builtin__.bool",
    // `fix_imports` renames two of them on the way down to a protocol Python 2
    // could read: an `int` of any size was a `long` there, and text was
    // `unicode`. Python 2 writing its own `int` and `str` is the pair above.
    "__builtin__.long",
    "__builtin__.unicode",
];

/// What the packed bytes of a date, a time or a datetime are called, which is
/// what `datetime.__reduce__` hands its class and nothing else.
const PACKED: &[&str] = &["packed"];
/// The same, for a value that carries the zone it is counted in.
const PACKED_ZONE: &[&str] = &["packed", "tzinfo"];
/// The three numbers a `timedelta` is, in the order Python writes them.
const SPAN: &[&str] = &["days", "seconds", "microseconds"];
/// The fields IronPython hands the three date classes, which are the
/// constructor's own parameters in the constructor's own order.
const YMD: &[&str] = &["year", "month", "day"];
const HMS_ZONE: &[&str] = &["hour", "minute", "second", "microsecond", "tzinfo"];
const YMD_HMS_ZONE: &[&str] = &["year", "month", "day", "hour", "minute", "second", "microsecond", "tzinfo"];
const OFFSET: &[&str] = &["offset"];
const NAMED_OFFSET: &[&str] = &["offset", "name"];
/// A `Decimal` and the one-argument `Fraction` are each written as the text
/// their own `str` gives, which is the whole of what they are.
const LITERAL: &[&str] = &["value"];
const RATIO: &[&str] = &["numerator", "denominator"];
/// The words a path is made of, however many there are.
const PARTS: &[&str] = &["parts"];
/// What a container class is called with, which is Python's own name for the
/// parameter: everything the container is to hold, or, at protocol 4 and 5,
/// the empty tuple standing where those would have been. Not `items`, which is
/// what the rows beneath a filled `deque` are.
const ITERABLE: &[&str] = &["iterable"];
const CAPPED: &[&str] = &["iterable", "maxlen"];
/// What a `defaultdict` is called with, which is the class it makes a missing
/// value with, or nothing at all.
const FACTORY: &[&str] = &["factory"];
/// Nothing at all, for a call whose result is its own contents.
const NOTHING: &[&str] = &[];
/// The two halves a structseq is rebuilt from: the run of numbers the class
/// reads as a sequence, and the dictionary holding the fields past the end of
/// that run. `structseq_reduce` writes both, so both are always there.
const STRUCTSEQ: &[&str] = &["fields", "extra fields"];

/// The classes the standard library keeps in a module no class may be named
/// from, by their whole dotted path.
///
/// `time` and `os` are not in [`STDLIB_MODULES`] and must not be: a prefix
/// there says any class under the module may be *named*, and `os` holds
/// `system` as well as `stat_result`. The calls table names the whole path
/// instead, and this is what says such a call is the standard library's own,
/// so a file holding one is read under this form.
pub(super) const OWN_CLASSES: &[&str] = &["time.struct_time", "os.stat_result"];

/// How many numbers each structseq's run holds, which is the class's
/// `n_sequence_fields`: `struct_time` is nine and `stat_result` is ten.
const TIME_FIELDS: usize = 9;
const STAT_FIELDS: usize = 10;

/// How many bytes each of the three packed values is, which is what
/// `Lib/datetime.py` `_getstate` writes.
const DATE_BYTES: usize = 4;
const TIME_BYTES: usize = 6;
const DATETIME_BYTES: usize = 10;

/// The callables the standard library writes, and the whole of what a REDUCE
/// may do under this form.
///
/// A callable written more than one way has a row each: a datetime is aware or
/// naive, a `Fraction` is two integers or one text, and `OrderedDict` and
/// `deque` changed shape between releases. The rows are tried in order and the
/// first whose arity and argument shape both fit is the reading.
pub(super) const STDLIB_CALLS: &[Reduce] = &[
    // A date, a time and a datetime are each their class called with the one
    // run of bytes `_getstate` packs them into, and a second argument holding
    // the zone when the value is aware.
    dated("datetime.datetime", Shape::DateTime, PACKED, DATETIME_BYTES),
    dated("datetime.datetime", Shape::DateTime, PACKED_ZONE, DATETIME_BYTES),
    dated("datetime.date", Shape::Date, PACKED, DATE_BYTES),
    dated("datetime.time", Shape::Time, PACKED, TIME_BYTES),
    dated("datetime.time", Shape::Time, PACKED_ZONE, TIME_BYTES),
    // And the same three as IronPython writes them. Its `datetime` is a
    // managed class whose `__reduce__` hands the constructor the fields
    // themselves rather than the run of bytes `_getstate` packs them into, so
    // the numbers are in the file as numbers. That is the class, not the
    // pickler: both of IronPython 2.7's picklers write it this way.
    Reduce { path: "datetime.date", names: YMD, what: Shape::Date, shape: |_c, args| is_ymd(args), ..PLAIN },
    Reduce { path: "datetime.time", names: HMS_ZONE, what: Shape::Time, shape: |_c, args| is_hms(&args[..4]).and(zone(&args[4])), ..PLAIN },
    Reduce {
        path: "datetime.datetime",
        names: YMD_HMS_ZONE,
        what: Shape::DateTime,
        shape: |_c, args| is_ymd(&args[..3]).and(is_hms(&args[3..7])).and(zone(&args[7])),
        ..PLAIN
    },
    Reduce { path: "datetime.timedelta", names: SPAN, what: Shape::TimeDelta, shape: |_c, args| whole(args), ..PLAIN },
    Reduce { path: "datetime.timezone", names: OFFSET, what: Shape::TimeZone, shape: |_c, args| span(&args[0]), ..PLAIN },
    Reduce {
        path: "datetime.timezone",
        names: NAMED_OFFSET,
        what: Shape::TimeZone,
        shape: |c, args| span(&args[0]).and(c.text_value(&args[1])).map(|_| ()),
        ..PLAIN
    },
    // An exact number, written as the text its own `str` gives.
    Reduce { path: "decimal.Decimal", names: LITERAL, what: Shape::Decimal, shape: |c, args| is_decimal(c, &args[0]), ..PLAIN },
    Reduce { path: "fractions.Fraction", names: RATIO, what: Shape::Fraction, shape: |_c, args| whole(args), ..PLAIN },
    Reduce { path: "fractions.Fraction", names: LITERAL, what: Shape::Fraction, shape: |c, args| is_ratio(c, &args[0]), ..PLAIN },
    // The two structseq classes: the run of numbers the class reads as a
    // sequence, and the dictionary of the fields past the end of that run. A
    // `struct_time` carries the zone it was read in; a `stat_result` carries
    // the times again as floats and as nanoseconds, and whatever else the
    // platform's `stat` has.
    Reduce { path: "time.struct_time", names: STRUCTSEQ, what: Shape::StructTime, shape: |c, args| structseq(c, args, TIME_FIELDS), ..PLAIN },
    Reduce { path: "os.stat_result", names: STRUCTSEQ, what: Shape::StatResult, shape: |c, args| structseq(c, args, STAT_FIELDS), ..PLAIN },
    // A path is its parts, however many there are, and the empty path has
    // none. Every part is a word the file wrote.
    path_call("pathlib.PurePosixPath"),
    path_call("pathlib.PureWindowsPath"),
    path_call("pathlib._local.PurePosixPath"),
    path_call("pathlib._local.PureWindowsPath"),
    // A counter is called with the mapping it holds, which is the counter, so
    // it is read as the contents rather than as an argument beside them.
    Reduce {
        path: "collections.Counter",
        names: NOTHING,
        what: Shape::Counter,
        args: Args::Contents,
        shape: |_c, args| matches!(args[0].kind, Kind::Dict(_)).then_some(()),
        ..PLAIN
    },
    // The three containers a pickler creates empty and fills with the opcodes
    // after the call, which is how every other container in a pickle is built.
    ORDERED_DICT,
    fills("collections.defaultdict", Shape::DefaultDict, NOTHING, Args::FillsDict, |_c, _args| Some(())),
    fills("collections.defaultdict", Shape::DefaultDict, FACTORY, Args::FillsDict, |c, args| is_factory(c, &args[0])),
    fills("collections.deque", Shape::Deque, NOTHING, Args::FillsList, |_c, _args| Some(())),
    fills("collections.deque", Shape::Deque, CAPPED, Args::FillsList, |_c, args| {
        // The empty tuple in front of the cap is where the items would have
        // been; CPython writes them through the call's fourth slot instead,
        // which is the APPENDS after it.
        (matches!(&args[0].kind, Kind::Tuple(held) if held.is_empty()) && matches!(args[1].kind, Kind::Int { .. })).then_some(())
    }),
    // And the way the older releases wrote the same three: everything the
    // container holds, handed over as one list for the class to be called
    // with. `Counter` is above and never changed.
    Reduce {
        path: "collections.OrderedDict",
        names: ITERABLE,
        what: Shape::OrderedDict,
        shape: |_c, args| pairs(&args[0]),
        ..PLAIN
    },
    Reduce { path: "collections.deque", names: ITERABLE, what: Shape::Deque, shape: |_c, args| holds(&args[0]), ..PLAIN },
    Reduce {
        path: "collections.deque",
        names: CAPPED,
        what: Shape::Deque,
        shape: |_c, args| holds(&args[0]).filter(|_| matches!(args[1].kind, Kind::Int { .. })),
        ..PLAIN
    },
];

/// What a row says when it says nothing else: the global itself, a fixed few
/// arguments, and a result nothing fills.
const PLAIN: Reduce =
    Reduce { path: "", via: Via::Global, what: Shape::Object, names: NOTHING, args: Args::Fixed, shape: |_c, _args| Some(()) };

/// `collections.OrderedDict()`, called empty and filled by the SETITEMS after
/// it. Named apart from the rest of the table because the torch forms name it
/// too: a state dict is one of these, and torch is not the standard library.
pub(super) const ORDERED_DICT: Reduce = fills("collections.OrderedDict", Shape::OrderedDict, NOTHING, Args::FillsDict, |_c, _args| Some(()));

/// One of the three classes whose value is a run of packed bytes, with the
/// length that run has to be.
const fn dated(path: &'static str, what: Shape, names: &'static [&'static str], wide: usize) -> Reduce {
    Reduce {
        path,
        what,
        names,
        shape: match wide {
            DATE_BYTES => |c: &Cursor, args: &[Value]| is_date(c, &args[0]),
            TIME_BYTES => |c: &Cursor, args: &[Value]| is_time(c, &args[0]),
            _ => |c: &Cursor, args: &[Value]| is_datetime(c, &args[0]),
        },
        ..PLAIN
    }
}

/// One of the path classes, called with however many words the path is made
/// of.
const fn path_call(path: &'static str) -> Reduce {
    Reduce { path, what: Shape::Path, names: PARTS, args: Args::Many, shape: all_words, ..PLAIN }
}

/// One of the containers the opcodes after the call fill.
const fn fills(
    path: &'static str,
    what: Shape,
    names: &'static [&'static str],
    args: Args,
    shape: fn(&Cursor, &[Value]) -> Option<()>,
) -> Reduce {
    Reduce { path, what, names, args, shape, ..PLAIN }
}

/// Whether this is a structseq the way `structseq_reduce` writes one: the run
/// of whole numbers the class reads as a sequence, as long as the class says,
/// and a dictionary of the fields past the end of it.
///
/// Which fields that dictionary holds is the platform's: a `stat_result` on
/// Linux carries `st_blocks` and `st_rdev` and on Windows does not. So the
/// keys are words and the values are numbers, text or nothing, which is what
/// a field of one of these is, and the names themselves are the file's to
/// state.
fn structseq(c: &Cursor, args: &[Value], fields: usize) -> Option<()> {
    let Kind::Tuple(held) = &args[0].kind else { return None };
    if held.len() != fields || !held.iter().all(|x| matches!(x.kind, Kind::Int { .. } | Kind::Wide { .. })) {
        return None;
    }
    let Kind::Dict(entries) = &args[1].kind else { return None };
    entries.iter().all(|(key, value)| c.text_value(key).is_some() && is_field(c, value)).then_some(())
}

/// What one field of a structseq is worth: a whole number, a float, a word or
/// nothing. A `struct_time`'s `tm_zone` is the word, and its `tm_gmtoff` is
/// nothing where the zone is unknown.
fn is_field(c: &Cursor, value: &Value) -> bool {
    matches!(value.kind, Kind::Int { .. } | Kind::Wide { .. } | Kind::Float { .. } | Kind::None) || c.text_value(value).is_some()
}

/// Whether every argument is a whole number, which is what a `timedelta` and
/// the two-argument `Fraction` are made of.
fn whole(args: &[Value]) -> Option<()> {
    args.iter().all(|a| matches!(a.kind, Kind::Int { .. })).then_some(())
}

/// The number an argument spells, for the date classes IronPython hands their
/// fields rather than their packed bytes.
fn number(value: &Value) -> Option<i128> {
    match value.kind {
        Kind::Int { value, .. } => Some(value),
        _ => None,
    }
}

/// A year, a month and a day a calendar has, written as three numbers.
fn is_ymd(args: &[Value]) -> Option<()> {
    let [year, month, day] = [number(&args[0])?, number(&args[1])?, number(&args[2])?];
    fits_date(u16::try_from(year).ok()?, u8::try_from(month).ok()?, u8::try_from(day).ok()?)
}

/// An hour, a minute, a second and a microsecond a clock has, written as four
/// numbers. There is no fold bit here: a field is a field.
fn is_hms(args: &[Value]) -> Option<()> {
    let [hour, minute, second, micro] = [number(&args[0])?, number(&args[1])?, number(&args[2])?, number(&args[3])?];
    ((0..24).contains(&hour) && (0..60).contains(&minute) && (0..60).contains(&second) && (0..1_000_000).contains(&micro)).then_some(())
}

/// Whether this is the zone an aware value carries, or the nothing a naive one
/// carries in its place.
fn zone(value: &Value) -> Option<()> {
    matches!(value.kind, Kind::None | Kind::Made { what: Shape::TimeZone, .. }).then_some(())
}

/// Whether this is the `timedelta` a `timezone` is the offset of.
fn span(value: &Value) -> Option<()> {
    matches!(value.kind, Kind::Made { what: Shape::TimeDelta, .. }).then_some(())
}

/// Whether this is a list of pairs, which is what the older releases hand
/// `OrderedDict`: the items of a mapping, each written as its own short list.
fn pairs(value: &Value) -> Option<()> {
    let (Kind::List(items) | Kind::Tuple(items)) = &value.kind else { return None };
    items.iter().all(|x| matches!(&x.kind, Kind::List(pair) | Kind::Tuple(pair) if pair.len() == 2)).then_some(())
}

/// Whether this is everything a container was handed to hold, which the older
/// releases write as a list and PyPy as a tuple.
fn holds(value: &Value) -> Option<()> {
    matches!(value.kind, Kind::List(_) | Kind::Tuple(_)).then_some(())
}

/// Whether this is one of the classes a `defaultdict` may be handed, or
/// nothing at all, which is what a `defaultdict` with no factory carries.
fn is_factory(c: &Cursor, value: &Value) -> Option<()> {
    match &value.kind {
        Kind::None => Some(()),
        Kind::Class { path, .. } => c.allow.names.contains(&path.as_str()).then_some(()),
        _ => None,
    }
}

/// Whether every part of a path is a word the file wrote. A path has as many
/// parts as it has, so these are the parts themselves rather than one argument
/// holding them, and a path of none at all is the empty one.
fn all_words(c: &Cursor, parts: &[Value]) -> Option<()> {
    parts.iter().all(|part| c.text_value(part).is_some()).then_some(())
}

impl Cursor<'_> {
    /// The text a value is, spelled here, named out of the memo, or spelled as
    /// a protocol 0 line. Nothing for a value that is not text at all.
    pub(super) fn text_value(&self, value: &Value) -> Option<String> {
        match value.kind {
            Kind::Text { at, len } | Kind::Ref(Names::Text { at, len }) => {
                Some(std::str::from_utf8(self.bytes.get(at..at.checked_add(len)?)?).ok()?.to_string())
            }
            // A line that spells its value rather than being it, which is the
            // one way protocol 0 writes a word with anything awkward in it.
            Kind::Spelled { at, len, quote, bytes: false } => String::from_utf8(self.spelled_at(at, len, quote)?).ok(),
            _ => None,
        }
    }

    /// The bytes a value stands for, however the protocol had to write them.
    ///
    /// Four spellings reach here. Protocol 3 and up write a byte string and
    /// the run is the bytes. Protocol 2 has no opcode for one, so Python 3
    /// hands the bytes to `_codecs.encode` as the latin-1 text they spell and
    /// the run is that text; protocol 0 writes the same text as an escaped
    /// line. And Python 2 had a type for a run of bytes, its `str`, which
    /// reads as text when those bytes happen to be UTF-8 and as bytes when
    /// they do not.
    pub(super) fn packed_bytes(&self, value: &Value) -> Option<Vec<u8>> {
        match &value.kind {
            Kind::Bytes { at, len } | Kind::Text { at, len } => Some(self.bytes.get(*at..at.checked_add(*len)?)?.to_vec()),
            Kind::Ref(Names::Bytes { at, len }) | Kind::Ref(Names::Text { at, len }) => {
                Some(self.bytes.get(*at..at.checked_add(*len)?)?.to_vec())
            }
            // A Python 2 `str` written as a protocol 0 line, which is the
            // `repr` of it: the bytes are what the escaping spells, worked out
            // once when the form read the line.
            Kind::Spelled { at, len, quote, .. } => self.spelled_at(*at, *len, *quote),
            Kind::Made { what: Shape::Bytes, items, .. } => match items.first()?.kind {
                Kind::Text { at, len } => Storage::Latin1.read(self.bytes.get(at..at.checked_add(len)?)?),
                // The text named where the file wrote it earlier, which is how
                // GraalPy writes the second of two equal ones.
                Kind::Ref(Names::Text { at, len }) => self.named_latin1(at, len),
                Kind::Spelled { at, len, quote, .. } => Storage::Latin1.read(&self.spelled_at(at, len, quote)?),
                _ => None,
            },
            _ => None,
        }
    }
}

/// A packed `date`: the year big-endian, the month and the day.
fn is_date(c: &Cursor, value: &Value) -> Option<()> {
    let packed = c.packed_bytes(value)?;
    let [yhi, ylo, month, day] = packed[..] else { return None };
    fits_date(u16::from_be_bytes([yhi, ylo]), month, day)
}

/// A packed `time`: the hour, minute and second, and the microsecond in three
/// bytes, most significant first.
fn is_time(c: &Cursor, value: &Value) -> Option<()> {
    let packed = c.packed_bytes(value)?;
    let [hour, minute, second, us1, us2, us3] = packed[..] else { return None };
    fits_time(c, hour, minute, second, [us1, us2, us3])
}

/// A packed `datetime`, which is the two runs above written one after the
/// other with the year in front.
fn is_datetime(c: &Cursor, value: &Value) -> Option<()> {
    let packed = c.packed_bytes(value)?;
    let [yhi, ylo, month, day, hour, minute, second, us1, us2, us3] = packed[..] else { return None };
    // From Python 3.6 a datetime carries which side of a repeated hour it
    // fell on, and `_getstate` writes that bit in the top of the month byte
    // rather than in a byte of its own. It is written only from protocol 4,
    // so below that the high bit is a month no calendar has.
    let fold = month & 0x80 != 0;
    if fold && c.proto <= 3 {
        return None;
    }
    fits_date(u16::from_be_bytes([yhi, ylo]), month & 0x7f, day)?;
    fits_time(c, hour, minute, second, [us1, us2, us3])
}

/// A year, a month and a day a calendar has. The day is not checked against
/// the month: the file says what it says, and a reader is told the numbers.
fn fits_date(year: u16, month: u8, day: u8) -> Option<()> {
    ((1..=9999).contains(&year) && (1..=12).contains(&month) && (1..=31).contains(&day)).then_some(())
}

/// An hour, a minute, a second and a microsecond a clock has. A `time` carries
/// its fold bit in the top of the hour byte, as a datetime carries it in the
/// month.
fn fits_time(c: &Cursor, hour: u8, minute: u8, second: u8, micro: [u8; 3]) -> Option<()> {
    let fold = hour & 0x80 != 0;
    if fold && c.proto <= 3 {
        return None;
    }
    let microsecond = u32::from_be_bytes([0, micro[0], micro[1], micro[2]]);
    ((hour & 0x7f) < 24 && minute < 60 && second < 60 && microsecond < 1_000_000).then_some(())
}

/// Whether this is a number `Decimal.__str__` would have written.
///
/// A sign, and then digits with at most one point in them and an exponent
/// after them, or one of the three words a decimal that is not a number is
/// spelled with. Nothing else: a `Decimal` is built from the text it is given,
/// so the text is the value and a text Python would not have written is a
/// value this reader has not seen.
fn is_decimal(c: &Cursor, value: &Value) -> Option<()> {
    let said = c.text_value(value)?;
    let body = said.strip_prefix('-').or_else(|| said.strip_prefix('+')).unwrap_or(&said);
    // `str(Decimal("NaN"))` is `NaN`, `str(Decimal("sNaN"))` is `sNaN`, and an
    // infinity is `Infinity`. A quiet not-a-number may carry a payload of
    // digits after it, which is what `Decimal("NaN123")` is.
    if let Some(rest) = body.strip_prefix("NaN").or_else(|| body.strip_prefix("sNaN")) {
        return rest.bytes().all(|b| b.is_ascii_digit()).then_some(());
    }
    if body == "Infinity" {
        return Some(());
    }
    let (mantissa, exponent) = match body.split_once(['e', 'E']) {
        Some((m, e)) => (m, Some(e)),
        None => (body, None),
    };
    if let Some(exponent) = exponent {
        let digits = exponent.strip_prefix('+').or_else(|| exponent.strip_prefix('-')).unwrap_or(exponent);
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
    }
    let (whole, fraction) = match mantissa.split_once('.') {
        Some((w, f)) => (w, f),
        None => (mantissa, ""),
    };
    let digits = whole.len() + fraction.len();
    (digits > 0 && whole.bytes().chain(fraction.bytes()).all(|b| b.is_ascii_digit())).then_some(())
}

/// Whether this is a fraction `Fraction.__str__` would have written, which is
/// the numerator, a slash and the denominator, or a whole number on its own.
fn is_ratio(c: &Cursor, value: &Value) -> Option<()> {
    let said = c.text_value(value)?;
    let (top, bottom) = match said.split_once('/') {
        Some((top, bottom)) => (top, Some(bottom)),
        None => (said.as_str(), None),
    };
    if !super::lines::is_whole(top) {
        return None;
    }
    match bottom {
        None => Some(()),
        // A denominator is written out only when it is not one, and it is
        // never negative: `Fraction` keeps the sign on the numerator.
        Some(bottom) => (super::lines::is_whole(bottom) && !bottom.starts_with('-') && bottom != "1").then_some(()),
    }
}
