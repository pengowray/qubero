//! Which structures in the built-in templates name their rows, and which
//! could: a walk over every template's types, read off the declarations
//! rather than off any file.
//!
//! Two lists. The first is structures with a `named_by` that lands on
//! something composite, which is a row the reader sees bare unless the
//! landing structure names itself in turn. The second is structures with no
//! `named_by` holding a field whose name says it is one: `name`, `id`,
//! `tag`, `type`, `key`, `kind`. Each is a candidate, not a finding: a `type`
//! that is a number nobody enumerated names nothing.
//!
//!     cargo run -p qubero-core --example naming_survey
//!
//! Prints tab-separated lines, `template  structure  what  detail`, and a
//! count of each kind at the end.

use std::collections::HashSet;

use qubero_core::formats;
use qubero_core::template::{StructDef, Template, Ty};

/// The field names a format tends to give the thing that names a record.
const NAMING: &[&str] = &["name", "id", "tag", "type", "key", "kind", "fourcc", "signature", "label", "title", "ident"];

fn main() {
    let mut seen = HashSet::new();
    let (mut bare, mut candidates, mut named) = (0, 0, 0);
    for name in formats::builtin_names() {
        let Some(t) = formats::builtin(name) else { continue };
        let mut structs = Vec::new();
        collect(&t.root, &t, &mut structs, &mut HashSet::new());
        for s in structs {
            if !seen.insert(format!("{name}/{}", s.name)) {
                continue;
            }
            match &s.named_by {
                Some(by) => {
                    named += 1;
                    if let Some(what) = lands_bare(&s, by, &t) {
                        bare += 1;
                        println!("{name}\t{}\tbare\tnamed by {by}, which is {what}", s.name);
                    }
                }
                None => {
                    for f in &s.fields {
                        if NAMING.contains(&&*f.name) && names_something(&f.ty, &t) {
                            candidates += 1;
                            println!("{name}\t{}\tcandidate\tholds `{}` ({})", s.name, f.name, kind(&f.ty, &t));
                            break;
                        }
                    }
                }
            }
        }
    }
    eprintln!("{named} structures say what names them; {bare} of those land on something composite; {candidates} unnamed structures hold a field that looks like a name");
}

/// Every structure under `ty`, following named types once each.
fn collect(ty: &Ty, t: &Template, out: &mut Vec<std::sync::Arc<StructDef>>, visited: &mut HashSet<String>) {
    match ty {
        Ty::Struct(s) => {
            out.push(s.clone());
            for f in &s.fields {
                collect(&f.ty, t, out, visited);
            }
        }
        Ty::Named(n) => {
            if visited.insert(n.to_string()) {
                if let Some(inner) = t.types.get(&**n) {
                    collect(inner, t, out, visited);
                }
            }
        }
        Ty::Switch { cases, default, .. } => {
            for (_, c) in cases.iter() {
                collect(c, t, out, visited);
            }
            collect(default, t, out, visited);
        }
        Ty::Match { cases, default, .. } => {
            for (_, c) in cases.iter() {
                collect(c, t, out, visited);
            }
            collect(default, t, out, visited);
        }
        Ty::Array { elem, .. } | Ty::Repeat { elem, .. } | Ty::Chain { elem, .. } | Ty::Gather { elem, .. } => {
            collect(elem, t, out, visited)
        }
        Ty::PointerList { elem, .. } => collect(elem, t, out, visited),
        Ty::At { inner, .. }
        | Ty::Sized { inner, .. }
        | Ty::SizedBits { inner, .. }
        | Ty::Origin { inner }
        | Ty::When { inner, .. }
        | Ty::Nullable { inner, .. }
        | Ty::Enum { inner, .. }
        | Ty::Flags { inner, .. }
        | Ty::Decoded { inner, .. }
        | Ty::Stitched { inner, .. } => collect(inner, t, out, visited),
        _ => {}
    }
}

/// Where a `named_by` path lands, read off the declaration, and what it is
/// when that is nothing a row can be labelled with. None when it lands on a
/// value, or on a structure that names itself or is only its contents.
fn lands_bare(s: &StructDef, by: &str, t: &Template) -> Option<String> {
    let mut steps = by.split('.');
    let first = steps.next()?;
    let f = s.fields.iter().find(|f| *f.name == *first)?;
    let mut ty = unwrap(&f.ty, t);
    for step in steps {
        ty = match ty {
            Ty::Struct(s) => unwrap(&s.fields.iter().find(|f| *f.name == *step)?.ty.clone(), t),
            Ty::Array { elem, .. } | Ty::Repeat { elem, .. } => unwrap(&elem, t),
            _ => return Some(format!("a {} with no `{step}` in it", kind(&ty, t))),
        };
    }
    match &ty {
        Ty::Struct(inner) if inner.named_by.is_some() || inner.contents.is_some() => None,
        Ty::Struct(_) | Ty::Array { .. } | Ty::Repeat { .. } | Ty::PointerList { .. } | Ty::Chain { .. } | Ty::Gather { .. } => {
            Some(kind(&ty, t))
        }
        // A switch is whichever case the file picks; bare only if every case is.
        Ty::Switch { cases, default, .. } => {
            let all: Vec<&Ty> = cases.iter().map(|(_, c)| c).chain(std::iter::once(&**default)).collect();
            all.iter().all(|c| composite(&unwrap(c, t), t)).then(|| "a switch of composites".to_string())
        }
        _ => None,
    }
}

/// Through the wrappers that stand between a declaration and what it holds.
fn unwrap(ty: &Ty, t: &Template) -> Ty {
    match ty {
        Ty::Named(n) => t.types.get(&**n).map(|i| unwrap(i, t)).unwrap_or_else(|| ty.clone()),
        Ty::At { inner, .. } | Ty::Sized { inner, .. } | Ty::SizedBits { inner, .. } | Ty::Origin { inner } | Ty::When { inner, .. } | Ty::Decoded { inner, .. } => {
            unwrap(inner, t)
        }
        other => other.clone(),
    }
}

fn composite(ty: &Ty, t: &Template) -> bool {
    match ty {
        Ty::Struct(s) => s.named_by.is_none() && s.contents.is_none(),
        Ty::Array { .. } | Ty::Repeat { .. } | Ty::PointerList { .. } | Ty::Chain { .. } | Ty::Gather { .. } => true,
        Ty::Switch { cases, default, .. } => {
            cases.iter().all(|(_, c)| composite(&unwrap(c, t), t)) && composite(&unwrap(default, t), t)
        }
        _ => false,
    }
}

/// Whether a field of this type reads as something a row could be called.
fn names_something(ty: &Ty, t: &Template) -> bool {
    matches!(unwrap(ty, t), Ty::Str { .. } | Ty::Magic(_) | Ty::Enum { .. } | Ty::TextInt { .. } | Ty::ComputedText(_))
}

fn kind(ty: &Ty, t: &Template) -> String {
    match unwrap(ty, t) {
        Ty::Struct(s) => format!("structure {}", s.name),
        Ty::Array { .. } | Ty::Repeat { .. } | Ty::PointerList { .. } | Ty::Chain { .. } | Ty::Gather { .. } => "list".into(),
        Ty::Switch { .. } => "switch".into(),
        Ty::Str { .. } => "text".into(),
        Ty::Magic(_) => "magic".into(),
        Ty::Enum { .. } => "enum".into(),
        Ty::TextInt { .. } => "digits".into(),
        Ty::ComputedText(_) => "computed text".into(),
        other => format!("{other:?}").split(['(', ' ', '{']).next().unwrap_or("?").to_string(),
    }
}
