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
use super::forms::{Via, BASE_CLASS, PARTIAL, RECONSTRUCTOR};
use super::memo::Bound;
use super::{Kind, Shape, Value};

/// The dotted path of a class, or nothing for anything that is not one.
fn path_of(value: &Value) -> Option<&str> {
    match &value.kind {
        Kind::Class { path, .. } => Some(path),
        _ => None,
    }
}

impl Cursor<'_> {
    /// Whether a module is one the form in hand may name a class from.
    ///
    /// The package itself or anything under it, and nothing that merely starts
    /// with the same letters: pandas 3.0 spells its frame's module `pandas`
    /// where 2.x spelled it `pandas.core.frame`, and `sklearnish` is neither.
    pub(super) fn whitelisted(&self, module: &str) -> bool {
        self.allow.classes.iter().any(|package| {
            module.strip_prefix(package).is_some_and(|rest| rest.is_empty() || rest.starts_with('.'))
        })
    }

    /// Whether this form may name this global at all: a class from a module
    /// under one of its prefixes, or one of the callables it lists, which is
    /// how `functools.partial` is named without `functools` being a package
    /// any class may come from.
    pub(super) fn may_name(&self, path: &str) -> bool {
        match path.rsplit_once('.') {
            // `object` is the base class every `_reconstructor` call is
            // handed, and the one class a form may name and never call.
            Some((module, _)) => {
                self.module_fits(module)
                    && (self.whitelisted(module) || path == BASE_CLASS || self.calls().any(|call| call.path == path))
            }
            None => false,
        }
    }

    /// Whether this module name belongs to this protocol.
    ///
    /// A pickler below protocol 3 writes the names Python 2 knew the builtins
    /// by, which is what `fix_imports` is for: `__builtin__` there and
    /// `builtins` from protocol 3 up. Each spelling belongs to one side, and a
    /// file using the other side's is a file no pickler wrote.
    pub(super) fn module_fits(&self, module: &str) -> bool {
        match module {
            "builtins" => self.proto >= 3,
            "__builtin__" => self.proto < 3,
            _ => true,
        }
    }

    /// Whether this form reads a call at all, which is what says the opcodes
    /// that name, make and fill an object may appear.
    pub(super) fn reads_calls(&self) -> bool {
        !self.allow.classes.is_empty() || self.calls().next().is_some()
    }

    /// Every callable this form accepts a REDUCE of, whichever list it was
    /// written in.
    pub(super) fn calls(&self) -> impl Iterator<Item = super::forms::Reduce> + '_ {
        self.allow.calls.iter().flat_map(|group| group.iter().copied())
    }

    /// GLOBAL, which is how protocols 2 and 3 name a class or a callable: one
    /// opcode and two newline-terminated lines, filed in one memo slot.
    ///
    /// The safety line is the same one STACK_GLOBAL is held to. The module has
    /// to be one the form lists, or the whole path one of the callables it
    /// enumerated, and nothing else is named at all. The two lines sit inside
    /// the opcode rather than being values of their own, so the node carries
    /// the dotted path and no parts.
    pub(super) fn global_line(&mut self) -> Option<Value> {
        let start = self.at;
        self.exact(b"c")?;
        let (module_at, module_len) = self.line()?;
        let module = std::str::from_utf8(self.bytes.get(module_at..module_at + module_len)?).ok()?;
        let (name_at, name_len) = self.line()?;
        let name = std::str::from_utf8(self.bytes.get(name_at..name_at + name_len)?).ok()?;
        let path = format!("{module}.{name}");
        if !self.may_name(&path) {
            return None;
        }
        self.memoize(Bound::Global(path.clone()))?;
        Some(self.span(start, Kind::Class { path, parts: Vec::new() }))
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
            b'R' => self.reduced(at, items),
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
        if !self.may_name(&path) {
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
    /// `object.__setstate__` takes, or a tuple, which is what a class with a
    /// `__setstate__` of its own is handed. The contents are data; the shape
    /// is fixed.
    /// What is below may be an object NEWOBJ made or the result of one of the
    /// enumerated calls: scikit-learn's tree of nodes is a call, and the arrays
    /// in it arrive by BUILD like any other attributes.
    fn built(&mut self, items: Vec<Value>) -> Option<Kind> {
        let [into, state] = <[Value; 2]>::try_from(items).ok()?;
        // A dictionary of attribute names, or the tuple a class that spells
        // its own state out writes: pandas hands a block manager its axes, its
        // blocks and the dictionary it versions them with as one tuple.
        if !matches!(state.kind, Kind::Dict(_) | Kind::Tuple(_)) {
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
    fn reduced(&mut self, at: usize, items: Vec<Value>) -> Option<Kind> {
        let [callable, args] = <[Value; 2]>::try_from(items).ok()?;
        let (path, via) = match &callable.kind {
            Kind::Class { path, .. } => (path.as_str(), Via::Global),
            // The one callable that is itself a call: a `functools.partial`
            // over the one global a form lists it with. Which global the
            // partial was made over was checked when the partial was made, and
            // it is checked again here, so neither half stands on its own.
            Kind::Made { callable: made, items, .. } => {
                if path_of(made)? != PARTIAL {
                    return None;
                }
                (path_of(items.first()?)?, Via::Partial)
            }
            _ => return None,
        };
        let call = self.calls().find(|call| call.path == path && call.via == via)?;
        let Kind::Tuple(held) = args.kind else { return None };
        if held.len() != call.names.len() {
            return None;
        }
        (call.shape)(&held)?;
        // A set and a frozenset are containers, not objects. Below protocol 4
        // a pickler builds one by calling the class with the list of members,
        // and what comes out is the container protocol 4 writes as a literal,
        // so it is read as that and not as a call of something.
        if matches!(call.what, Shape::Set | Shape::FrozenSet) {
            return self.set_made(call.what, at, held);
        }
        // An object made the way protocol 1 makes one. What comes out is the
        // same object NEWOBJ makes at protocol 2, so it is read as that and
        // the BUILD after it fills it in the same way.
        if path == RECONSTRUCTOR {
            return self.reconstructed_object(at, held);
        }
        // What the call made, which a later part of the file may name: pandas
        // writes a block's values once and names them again in the dictionary
        // it versions its state with.
        self.memoize(Bound::Made { what: call.what, at, hashable: false })?;
        // Only a class from the package the form is for says the form read
        // what it is for. The calls every form shares, such as the one that
        // makes a set, say nothing about which form a file belongs to.
        if self.whitelisted(path.rsplit_once('.')?.0) {
            self.instances += 1;
        }
        Some(Kind::Made { what: call.what, names: call.names, callable: Box::new(callable), items: held, state: None })
    }

    /// `copy_reg._reconstructor(cls, object, None)`, which is `cls.__new__(cls)`
    /// written the long way round: protocol 1 had no NEWOBJ.
    ///
    /// The class has to be one this form may name, which is the same rule
    /// NEWOBJ is held to, and the base and the argument were checked against
    /// the call's own shape before this.
    fn reconstructed_object(&mut self, at: usize, held: Vec<Value>) -> Option<Kind> {
        let class = held.into_iter().next()?;
        let Kind::Class { ref path, .. } = class.kind else { return None };
        if !self.whitelisted(path.rsplit_once('.')?.0) {
            return None;
        }
        self.memoize(Bound::Made { what: Shape::Object, at, hashable: false })?;
        self.instances += 1;
        Some(Kind::Instance { class: Box::new(class), state: None })
    }

    /// `set(members)` or `frozenset(members)`, which is how both are written
    /// below protocol 4. The members are written out for the call and never
    /// named out of the memo: CPython hands the class a list of them and PyPy
    /// a tuple. Everything in it has to be something Python could hash.
    fn set_made(&mut self, what: Shape, at: usize, held: Vec<Value>) -> Option<Kind> {
        let held = held.into_iter().next()?;
        let (Kind::List(members) | Kind::Tuple(members)) = held.kind else { return None };
        if !members.iter().all(super::basic::hashable) {
            return None;
        }
        let frozen = what == Shape::FrozenSet;
        self.memoize(Bound::Made { what, at, hashable: frozen })?;
        Some(match frozen {
            true => Kind::FrozenSet(members),
            false => Kind::Set(members),
        })
    }
}
