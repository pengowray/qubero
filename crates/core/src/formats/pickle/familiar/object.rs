//! The library object productions: a class the file names, an object made from
//! it with no arguments, the attributes a BUILD gives that object, and the
//! REDUCE of a callable the form enumerated. Only the forms that read a
//! library pickle allow any of them.
//!
//! The safety line is here rather than anywhere else, so it can be read in one
//! place. A class may be named only by STACK_GLOBAL and only from a module
//! under one of the prefixes its form lists. An object may be made only by
//! EMPTY_TUPLE NEWOBJ, and given its attributes only by BUILD of a dictionary.
//! A REDUCE is accepted only of a callable the form named, with the exact
//! arguments that callable is written with. A REDUCE of any other global under
//! a whitelisted module is a non-match, because a module whitelist can name
//! callables that do work when they are called, and this reader is the wrong
//! place to be deciding which.

use super::cursor::Cursor;
use super::memo::Bound;
use super::{Kind, Shape, Value};

impl Cursor<'_> {
    /// Whether a module is one the form in hand may name a class from.
    pub(super) fn whitelisted(&self, module: &str) -> bool {
        self.allow.classes.iter().any(|prefix| module.starts_with(prefix))
    }

    /// The four opcodes a library object is built with, each folding the two
    /// things written in front of it.
    ///
    /// They are written the same way round: the thing being named or called
    /// first, what it is named or called with second. So the loop takes two
    /// off the stack for any of them and this says what the pair makes.
    pub(super) fn library(&mut self, code: u8, at: usize, items: Vec<Value>) -> Option<Kind> {
        match code {
            0x93 => self.class_named(items),
            0x81 => self.new_object(at, items),
            b'R' => self.reduced(items),
            b'b' => self.built(items),
            _ => None,
        }
    }

    /// STACK_GLOBAL: the module and the name written in front of it.
    ///
    /// Both are ordinary strings, spelled out or named out of the memo, so
    /// they are already on the stack as the text they are. The pair is filed
    /// in the memo, which is how the second block of a frame names the same
    /// callable without spelling it again.
    fn class_named(&mut self, items: Vec<Value>) -> Option<Kind> {
        let [module, name] = <[Value; 2]>::try_from(items).ok()?;
        let module_text = self.text_of(&module)?.to_string();
        let name_text = self.text_of(&name)?.to_string();
        let path = format!("{module_text}.{name_text}");
        if !self.whitelisted(&module_text) && !self.allow.calls.iter().any(|call| call.path == path) {
            return None;
        }
        self.memoize(Bound::Global(path.clone()))?;
        Some(Kind::Class { path, parts: vec![module, name] })
    }

    /// The text a value is, whether the file spelled it here or named the slot
    /// it spelled it in. Anything else is not a module or a class name.
    fn text_of(&self, value: &Value) -> Option<&str> {
        let (at, len) = match value.kind {
            Kind::Text { at, len } => (at, len),
            Kind::Ref(super::Names::Text { at, len }) => (at, len),
            _ => return None,
        };
        std::str::from_utf8(self.bytes.get(at..at.checked_add(len)?)?).ok()
    }

    /// NEWOBJ over an empty tuple: `cls.__new__(cls)`, which is what CPython
    /// writes for an object whose class defines no reduce of its own.
    ///
    /// Only an empty tuple. A NEWOBJ with arguments is a class being handed
    /// values to construct itself from, and what those mean is the class's
    /// business rather than the file's.
    fn new_object(&mut self, at: usize, items: Vec<Value>) -> Option<Kind> {
        let [class, args] = <[Value; 2]>::try_from(items).ok()?;
        let Kind::Class { ref path, .. } = class.kind else { return None };
        let module = path.rsplit_once('.')?.0;
        if !self.whitelisted(module) {
            return None;
        }
        match args.kind {
            Kind::Tuple(ref held) if held.is_empty() => {}
            _ => return None,
        }
        // An object is filed when it is made, before the BUILD that fills it,
        // so a later attribute may name it: a random forest's `estimator_` is
        // the tree it was configured from, named where it was made.
        self.memoize(Bound::Made { what: Shape::Object, at, hashable: false })?;
        self.instances += 1;
        Some(Kind::Instance { class: Box::new(class), state: None })
    }

    /// BUILD: the state written in front of it, handed to the thing below it.
    ///
    /// The state is a dictionary of attribute names, which is what
    /// `object.__setstate__` takes. The names are data; the shape is fixed.
    /// What is below may be an object NEWOBJ made or the result of one of the
    /// enumerated calls: scikit-learn's tree of nodes is a call, and the arrays
    /// in it arrive by BUILD like any other attributes.
    fn built(&mut self, items: Vec<Value>) -> Option<Kind> {
        let [into, state] = <[Value; 2]>::try_from(items).ok()?;
        if !matches!(state.kind, Kind::Dict(_)) {
            return None;
        }
        let state = Some(Box::new(state));
        match into.kind {
            Kind::Instance { class, state: None } => Some(Kind::Instance { class, state }),
            Kind::Made { what, names, callable, items, state: None } => Some(Kind::Made { what, names, callable, items, state }),
            _ => None,
        }
    }

    /// REDUCE of one of the callables this form named, with the arguments that
    /// callable is written with.
    ///
    /// Never a REDUCE of whatever global happens to be under a whitelisted
    /// module. The list is short, each entry was read out of the corpus, and
    /// the argument shape is checked against what the library writes.
    fn reduced(&mut self, items: Vec<Value>) -> Option<Kind> {
        let [callable, args] = <[Value; 2]>::try_from(items).ok()?;
        let Kind::Class { ref path, .. } = callable.kind else { return None };
        let call = self.allow.calls.iter().find(|call| call.path == *path)?;
        let Kind::Tuple(held) = args.kind else { return None };
        if held.len() != call.names.len() {
            return None;
        }
        (call.shape)(&held)?;
        self.memoize(Bound::Opaque)?;
        self.instances += 1;
        Some(Kind::Made { what: call.what, names: call.names, callable: Box::new(callable), items: held, state: None })
    }
}
