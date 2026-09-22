//! The builtin exception classes, read as the class and the arguments the
//! exception was raised with.
//!
//! `BaseException.__reduce__` hands back the class and `self.args`, and the
//! instance dictionary after them when the exception carries one, so a pickled
//! exception is a `REDUCE` of the class over a tuple, with a `BUILD` of a
//! dictionary behind it or without one. Nothing here is run: a pickled
//! exception is a class name and the message it was raised with, and this
//! reads it as that.
//!
//! The safety line is the one [`object`](super::object) holds every other form
//! to. `builtins` is not a package any class may be named from, so the classes
//! are named one by one: [`HIERARCHY`] is the whole builtin exception
//! hierarchy, and nothing under `builtins` outside it is read as an exception.
//!
//! Three spellings reach the same class. From protocol 3 the module is
//! `builtins`; below it `fix_imports` rewrites the names Python 2 had to
//! `exceptions`, and leaves the rest under `__builtin__`. The rewriting is
//! lossy and the file is read as what it says: a `FileNotFoundError` written
//! at protocol 2 says `exceptions.OSError`, because that is the class a
//! Python 2 reading it would have got.

use super::{Kind, Names, Shape, Value};

/// Every class of the builtin exception hierarchy, by the name it goes by.
///
/// Read off `builtins` at Python 3.12 and 3.13, which is `BaseException` and
/// everything under it. A pickler writes the class by this name from protocol
/// 3 up, whichever of them it is.
pub(super) const HIERARCHY: &[&str] = &[
    "ArithmeticError", "AssertionError", "AttributeError", "BaseException", "BaseExceptionGroup", "BlockingIOError",
    "BrokenPipeError", "BufferError", "BytesWarning", "ChildProcessError", "ConnectionAbortedError", "ConnectionError",
    "ConnectionRefusedError", "ConnectionResetError", "DeprecationWarning", "EOFError", "EncodingWarning", "EnvironmentError",
    "Exception", "ExceptionGroup", "FileExistsError", "FileNotFoundError", "FloatingPointError", "FutureWarning",
    "GeneratorExit", "IOError", "ImportError", "ImportWarning", "IndentationError", "IndexError", "InterruptedError",
    "IsADirectoryError", "KeyError", "KeyboardInterrupt", "LookupError", "MemoryError", "ModuleNotFoundError", "NameError",
    "NotADirectoryError", "NotImplementedError", "OSError", "OverflowError", "PendingDeprecationWarning", "PermissionError",
    "ProcessLookupError", "RecursionError", "ReferenceError", "ResourceWarning", "RuntimeError", "RuntimeWarning",
    "StopAsyncIteration", "StopIteration", "SyntaxError", "SyntaxWarning", "SystemError", "SystemExit", "TabError",
    "TimeoutError", "TypeError", "UnboundLocalError", "UnicodeDecodeError", "UnicodeEncodeError", "UnicodeError",
    "UnicodeTranslateError", "UnicodeWarning", "UserWarning", "ValueError", "Warning", "ZeroDivisionError",
];

/// The classes Python 2's `exceptions` module held, which is what a name below
/// protocol 3 is written under.
///
/// The first forty-seven are `_compat_pickle.PYTHON2_EXCEPTIONS`, which is the
/// table `save_global` rewrites a name through. `StandardError` and
/// `WindowsError` are not in that table because Python 3 has no class to
/// rewrite into them, and they are here because Python 2's own pickler writes
/// them: `StandardError` sat between `Exception` and the concrete errors, and
/// `WindowsError` is the class an `OSError` is on Windows. `VMSError` is the
/// third of that kind and is left out: no such platform is in the collection.
pub(super) const PYTHON2: &[&str] = &[
    "ArithmeticError", "AssertionError", "AttributeError", "BaseException", "BufferError", "BytesWarning", "DeprecationWarning",
    "EOFError", "EnvironmentError", "Exception", "FloatingPointError", "FutureWarning", "GeneratorExit", "IOError",
    "ImportError", "ImportWarning", "IndentationError", "IndexError", "KeyError", "KeyboardInterrupt", "LookupError",
    "MemoryError", "NameError", "NotImplementedError", "OSError", "OverflowError", "PendingDeprecationWarning", "ReferenceError",
    "RuntimeError", "RuntimeWarning", "StandardError", "StopIteration", "SyntaxError", "SyntaxWarning", "SystemError",
    "SystemExit", "TabError", "TypeError", "UnboundLocalError", "UnicodeDecodeError", "UnicodeEncodeError", "UnicodeError",
    "UnicodeTranslateError", "UnicodeWarning", "UserWarning", "ValueError", "Warning", "WindowsError", "ZeroDivisionError",
];

/// What the arguments of an exception are called, which is Python's own name
/// for them. One word for however many there are: `args` is a tuple, and an
/// exception has as many of them as it was raised with.
pub(super) const ARGS: &[&str] = &["args"];

/// Whether this dotted path names one of the builtin exception classes,
/// spelled the way the protocol that wrote it spells one.
///
/// The protocol is not asked here.
/// [`Cursor::module_fits`](super::cursor::Cursor) already holds each of the
/// three module names to the side of protocol 3 it belongs to, and every
/// caller has been through it.
pub(super) fn names_exception(path: &str) -> bool {
    match path.rsplit_once('.') {
        Some(("builtins", name)) => HIERARCHY.contains(&name),
        Some(("exceptions", name)) => PYTHON2.contains(&name),
        // The classes Python 2 never had. `fix_imports` has no name to rewrite
        // them to, so they go out under the module name Python 2 knew the
        // builtins by, and a name Python 2 did have is never written this way.
        Some(("__builtin__", name)) => HIERARCHY.contains(&name) && !PYTHON2.contains(&name),
        _ => false,
    }
}

/// Whether every argument an exception was raised with is plain data.
///
/// What Python puts in `args` is whatever was passed to `raise`, and what a
/// file holding an exception holds is a message: a word, a number, the errno
/// and the strerror of an `OSError`, the file name after them, and the tuple
/// of a file, a line and a column a `SyntaxError` carries. An argument that is
/// a class, an object of one, an array or a tensor is not a message, and a
/// form that read one would be reading a class it never wrote down.
///
/// One exception may hold others: `ExceptionGroup` is a message and the list
/// of what was raised. Those are read, because an exception is already one of
/// the things this file enumerates.
pub(super) fn plain(args: &[Value]) -> Option<()> {
    args.iter().all(is_plain).then_some(())
}

fn is_plain(value: &Value) -> bool {
    match &value.kind {
        Kind::None | Kind::Bool(_) | Kind::Int { .. } | Kind::Wide { .. } | Kind::Float { .. } => true,
        Kind::Text { .. } | Kind::Bytes { .. } | Kind::Spelled { .. } => true,
        // A name stands for whatever the slot holds. A name for a container is
        // as plain as the container would have been; a name for a call is
        // plain when the call made one of the values above, which is what a
        // byte string below protocol 3 and a bytearray are.
        Kind::Ref(Names::Text { .. } | Names::Bytes { .. }) => true,
        Kind::Ref(Names::Made { what, .. }) => plain_shape(*what),
        Kind::List(items) | Kind::Tuple(items) | Kind::Set(items) | Kind::FrozenSet(items) => items.iter().all(is_plain),
        Kind::Dict(entries) => entries.iter().all(|(key, held)| is_plain(key) && is_plain(held)),
        Kind::Made { what, items, state, .. } => {
            plain_shape(*what) && items.iter().all(is_plain) && state.as_deref().is_none_or(is_plain)
        }
        _ => false,
    }
}

/// Which of the things a call makes counts as plain data inside a message: the
/// values the language writes as a call of the class rather than as a literal,
/// the standard library's own values, and another exception.
fn plain_shape(what: Shape) -> bool {
    matches!(
        what,
        Shape::Bytes
            | Shape::ByteArray
            | Shape::Text
            | Shape::Set
            | Shape::FrozenSet
            | Shape::Complex
            | Shape::Slice
            | Shape::Range
            | Shape::Exception
    ) || super::values::hashes(what)
}
