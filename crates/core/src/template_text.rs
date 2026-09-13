//! The template IR written out as text, for a reader rather than for a parser.
//!
//! [`render`] takes a [`Template`] and writes every part of it: the root, every
//! named type, every field and everything hanging off a field. Nothing is
//! summarised and nothing is left out, so what a format's template says can be
//! read in one place instead of assembled from Rust constructors. It is
//! read-only: there is no parser for this notation yet, and a value here is
//! written once, not round-tripped.
//!
//! The one rule the notation is built on: a line has one reading. Where two IR
//! ideas would collide on a word, they get different words.
//!
//! # Grammar
//!
//! ```text
//! file        := "template" NAME
//!                ["deducer: in code"]
//!                "root" ty
//!                {"type" NAME ["=" ...] ty}        types in alphabetical order
//!
//! ty          := struct | wrapper ty | ty annotation | leaf
//! struct      := NAME ["(" attr {"," attr} ")"] "{" {field} "}"
//! field       := NAME ":" ty {extra}
//!
//! wrapper     := "sized(" expr ")"                  parsed inside an N-byte window
//!              | "sizedbits(" expr ")"              the same, counted in bits
//!              | "at(" expr "from" anchor ")"       placed there, not here
//!              | "origin"                           offsets inside count from here
//!              | "decoded(" codec ")"               unpacked, and read from that
//!              | "repeat(" until ")"                one after another
//!              | "pointers(" ... ")"                placed at offsets read earlier
//!              | "chain(" ... ")"                   each element points at the next
//!              | "gather(" ... ")"                  placed at offsets found by a walk
//! annotation  := "[" expr "]"                       a list of that many elements
//!              | "enum" NAME "{" {int "=" NAME} "}"
//!              | "flags" NAME "{" {"bit" int "=" NAME} "}"
//!              | "unset" number                     that value means "nothing here"
//! anchor      := "file" | "window" | "origin" | "own start aligned to" int
//! until       := "end" | "element." NAME ("is" bytes | "==" int)
//!
//! extra       := "check" sum "over" covers ["when" expr] ["blanking" byte]
//!              | "element check" ...                the check is on each element
//!              | "time" epoch zone ["unset" int]
//!              | "name from" expr                   the row's label is read here
//!              | "element name from" expr           each element's label is
//!              | "aside"                            a second reading of bytes
//!                                                   counted where they were declared
//! covers      := named                              that field's own bytes
//!              | "unpacked" named ["of" expr "bytes"]
//!              | "unpacked" named "member packed at" named
//!              | expr "bytes at" expr                offsets from this structure
//!              | "everything before this field"
//! named       := NAME | "earlier(" path ")" | path "[" expr "]"
//!
//! attr        := "named by" NAME                    which field names one of these
//!              | "contents" NAME                    which field is merely the body
//!              | "unit" NAME                        what one is called when counted
//!              | "inline"                           one row, not one row per field
//!              | "packed" NAME                      contents only the format unpacks
//!              | "machinery" NAME {"," NAME}
//!              | "payload" NAME {"," NAME}
//!              | "line" part {"," part}              how one reads on a single line
//!              | "type name" NAME                   printed when the structure's own
//!                                                   name differs from the table key
//! part        := NAME ["worded" text] ["except" text]
//! ```
//!
//! ## Leaf types
//!
//! `u16be` `i32le` `signmag8be` are width in bits and then which end the low
//! bits come from, always both, for every width: at eight bits and wider `be`
//! is byte order, and narrower than a byte it is which end of the byte the
//! field is taken from (`be` is MSB-first). `u(expr)be` is a width the file
//! states. Floats are `f16be` `bf16le` `f32be` `f64le` `f80le` `f8e4m3`
//! `f8e5m2`. `fixed 16.16be` is whole bits and then fraction bits, `ufixed`
//! for the unsigned one. Then `leb128` `sleb128` `zigzag` `vlq`
//! `sqlite varint` `7z number` `ebml vint` (`ebml vint with marker` keeps the
//! marker bit), `magic` and a byte literal, `bytes[expr]`, `json object` and
//! the shape of the value, `text[expr]` and its encoding, `computed expr` and
//! `computed text expr` for a value with no bytes of its own.
//!
//! Text says how it ends and what it is in: `text[expr]`,
//! `text[expr] padded with 0x20`, `text until 0x00` (`or end` where a missing
//! terminator is allowed), `text token skipping 20 09 ending at 0A`. The
//! encoding follows: `utf8` `ascii` `latin1` `cp437` `cp437 screen` `utf16le`
//! `utf16be` `p8scii` `bom(latin1)` `encoding unknown`. `text[4] base 16 as
//! number` is a number written out in text.
//!
//! ## Placeholders
//!
//! Four type words stand for a reader written in code rather than for a layout
//! the IR describes. They are written as the words below and nothing else,
//! because there is nothing else to write:
//!
//! * `instruction riscv64` -- one instruction of that machine.
//! * `traced blocks`, `traced block 3`, `traced symbols 2` -- what a decoder
//!   walked, laid over the packed bytes.
//! * `codebits name (expression)` -- one entropy-coded symbol, and where its
//!   width came from.
//! * `deduced(payload shape)` in an expression -- an answer that comes from
//!   running the file. `deducer: in code` at the head says the template has
//!   something to run.
//!
//! ## Expressions
//!
//! Infix, bracketed only where the brackets change the reading. Binding from
//! loosest to tightest:
//!
//! ```text
//!   or else                 the left side, or the right when the left is zero
//!   <                       comparison
//!   &                       mask
//!   << >>                   shifts
//!   + -
//!   * /
//!   name  literal  call()   everything else
//! ```
//!
//! A name is a field declared earlier, in this structure or in one it sits
//! inside. `a.b` is a path down into an earlier field. `index` is this
//! element's place in the list it sits in. `remaining` is from here to the end
//! of the container. Then the calls: `sizeof(f)` `bitsof(f)` `sum(f)`
//! `product(f)` `largest(f)` `setbits(f)` `min(a, b)` `max(a, b)`
//! `ceil(a / b)` `log2(a)` `bit(x, 3)` `padding(n, 4)` (the padding *after* n
//! bytes, to the next multiple of 4), `peek(u8be)` and
//! `peek(u16le at expr bits)` (read without consuming), `tomarker(FF unless
//! 00)`, `find("endobj")` and `findlast("startxref")` (how far it is to
//! there), `previous(f)` (the element before this one), `earlier(a.b)` and
//! `earlier[key = tag].f` (the nearest earlier element that has one),
//! `array[i].f`, `descriptor.f` (the record that placed this element).
//!
//! Literals are decimal, except a mask, which is hex, and a number whose bytes
//! spell printable ASCII with letters in it, which is written as those bytes in
//! single quotes: `'IHDR'` is 1,229,472,850 read big-endian, which is what a
//! switch on a four-letter tag is keyed on. Byte strings are `"RIFF"` when
//! every byte is printable and `89 50 4E 47` otherwise. Text is in double
//! quotes.

use std::fmt::Write as _;
use std::sync::Arc;

use crate::eval::Sizing;
use crate::template::*;

/// How wide a line is allowed to get before it is broken.
const WIDTH: usize = 96;

/// The whole template as text: the root, then every named type in alphabetical
/// order. Two renders of one template are the same string.
pub fn render(t: &Template) -> String {
    let mut out: Vec<String> = Vec::new();
    out.push(format!("template {}", t.name));
    if t.deducer.is_some() {
        // Nothing generic can be said about what the run answers, only that
        // there is one. See `Template::deducer`.
        out.push("deducer: in code".to_string());
    }
    out.push(String::new());
    write_ty(&mut out, 0, "root ", &t.root, "");
    let mut names: Vec<&String> = t.types.keys().collect();
    names.sort();
    for name in names {
        let ty = &t.types[name];
        out.push(String::new());
        // The key is the name a `Named` reaches this type by. A structure
        // carries a name of its own as well, and the two are nearly always the
        // same word; where they are not, both are written.
        let head = match ty {
            Ty::Struct(d) if &d.name == name => "type ".to_string(),
            _ => format!("type {name} = "),
        };
        write_ty(&mut out, 0, &head, ty, "");
    }
    let mut wrapped = Vec::with_capacity(out.len());
    for line in &out {
        wrap_line(line, &mut wrapped);
    }
    let mut s = wrapped.join("\n");
    s.push('\n');
    s
}

/// One line, folded onto as many as it takes to stay inside [`WIDTH`].
///
/// Every line is folded the same way, whatever it holds, so there is one rule
/// to know rather than one per construct: what does not fit carries on four
/// spaces past the line's own indentation, which is deeper than a field of the
/// structure could be and so never reads as one.
fn wrap_line(line: &str, out: &mut Vec<String>) {
    if line.chars().count() <= WIDTH {
        out.push(line.to_string());
        return;
    }
    let indent = line.len() - line.trim_start().len();
    // Splitting on a single space keeps the empty pieces a double space leaves
    // behind, so the gap between a field and what is said about it survives the
    // fold.
    let words: Vec<&str> = line.trim_start().split(' ').collect();
    // How deep in brackets each word starts. A fold is taken as shallow as it
    // can be, so a long expression breaks between its own terms rather than in
    // the middle of one.
    let mut depth = vec![0i32; words.len()];
    let mut d = 0;
    for (i, w) in words.iter().enumerate() {
        depth[i] = d;
        for c in w.chars() {
            match c {
                '(' | '[' | '{' => d += 1,
                ')' | ']' | '}' => d -= 1,
                _ => {}
            }
        }
    }
    let mut start = 0;
    let mut first = true;
    while start < words.len() {
        let lead = if first { indent } else { indent + 4 };
        let mut end = start + 1;
        let mut len = lead + words[start].chars().count();
        while end < words.len() && len + 1 + words[end].chars().count() <= WIDTH {
            len += 1 + words[end].chars().count();
            end += 1;
        }
        if end < words.len() {
            let (mut best, mut shallowest) = (end, i32::MAX);
            for i in start + 1..=end {
                if !words[i].is_empty() && depth[i] <= shallowest {
                    shallowest = depth[i];
                    best = i;
                }
            }
            end = best;
        }
        let mut s = " ".repeat(lead);
        s.push_str(&words[start..end].join(" "));
        out.push(s.trim_end().to_string());
        start = end;
        first = false;
        while start < words.len() && words[start].is_empty() {
            start += 1;
        }
    }
}

fn pad(ind: usize) -> String {
    "  ".repeat(ind)
}

/// Write `ty` starting with `head` on its first line and ending with `tail` on
/// its last, at `ind` levels of indentation.
///
/// Wrappers push their word onto `head` and recurse, so `sized(length) switch
/// type {` is one line rather than three, and a structure several wrappers deep
/// still opens its brace at the end of the field's own line.
fn write_ty(out: &mut Vec<String>, ind: usize, head: &str, ty: &Ty, tail: &str) {
    if let Some(head) = wrapper(head, ty) {
        return write_ty(out, ind, &head.0, head.1, tail);
    }
    match ty {
        // Postfix: a list is its element type and how many of them.
        Ty::Array { elem, count } => return write_ty(out, ind, head, elem, &format!("[{}]{tail}", expr(count))),
        Ty::Nullable { inner, unset } => {
            return write_ty(out, ind, head, inner, &format!(" unset {}{tail}", unset_text(unset)))
        }
        _ => {}
    }
    if let Some(one) = inline(ty) {
        let line = format!("{}{head}{one}{tail}", pad(ind));
        if line.chars().count() <= WIDTH {
            out.push(line);
            return;
        }
    }
    match ty {
        Ty::Struct(def) => write_struct(out, ind, head, def, tail),
        Ty::Switch { on, cases, default } => {
            let cases: Vec<(String, &Ty)> = cases.iter().map(|(k, t)| (tag_lit(*k), t)).collect();
            write_choice(out, ind, head, &format!("switch {}", expr(on)), &cases, default, tail);
        }
        Ty::Match { on, cases, default } => {
            let cases: Vec<(String, &Ty)> = cases.iter().map(|(k, t)| (format!("{k:?}"), t)).collect();
            write_choice(out, ind, head, &format!("match {}", expr(on)), &cases, default, tail);
        }
        Ty::Enum { inner, def } => {
            let cases = enum_cases(def);
            write_body(out, ind, &format!("{head}{} enum {} ", brief(inner), def.name), &cases, tail);
        }
        Ty::Flags { inner, def } => {
            let cases: Vec<String> = def.bits.iter().map(|(b, n)| format!("bit {b} = {n}")).collect();
            write_body(out, ind, &format!("{head}{} flags {} ", brief(inner), def.name), &cases, tail);
        }
        // Everything else has an inline form and no way to break it, so it goes
        // out over width rather than mangled.
        other => out.push(format!("{}{head}{}{tail}", pad(ind), inline(other).unwrap_or_default())),
    }
}

/// The wrappers: a word in front, and the type they contain.
///
/// Separate from [`write_ty`] so that the recursion reads as what it is, and so
/// that a new wrapper is one arm here.
fn wrapper<'a>(head: &str, ty: &'a Ty) -> Option<(String, &'a Ty)> {
    let with = |word: String, inner: &'a Ty| Some((format!("{head}{word} "), inner));
    match ty {
        Ty::Sized { size, inner } => with(format!("sized({})", expr(size)), inner),
        Ty::SizedBits { bits, inner } => with(format!("sizedbits({})", expr(bits)), inner),
        Ty::At { anchor, at, inner } => with(format!("at({} from {})", expr(at), anchor_text(*anchor)), inner),
        Ty::Origin { inner } => with("origin".to_string(), inner),
        Ty::Decoded { codec, inner } => with(format!("decoded({})", packing(codec)), inner),
        Ty::Repeat { elem, until } => with(format!("repeat({})", until_text(until)), elem),
        Ty::PointerList { offsets, field, anchor, adjust, elem, to_next, skip_missing, skip_zero } => {
            let mut s = format!("pointers(at {} from {}", path_of(offsets, field), anchor_text(*anchor));
            adjustment(&mut s, adjust);
            if *to_next {
                s.push_str(", to the next");
            }
            if *skip_missing {
                s.push_str(", skip missing");
            }
            if *skip_zero {
                s.push_str(", skip zero");
            }
            s.push(')');
            with(s, elem)
        }
        Ty::Chain { first, next, elem, anchor, adjust } => {
            let mut s = format!("chain(first {}, next .{}, from {}", expr(first), next.join("."), anchor_text(*anchor));
            adjustment(&mut s, adjust);
            s.push(')');
            with(s, elem)
        }
        Ty::Gather { from, offset, anchor, adjust, elem, skip_zero } => {
            let walk: String = from.iter().map(step_text).collect::<Vec<_>>().join("");
            let walk = walk.strip_prefix('.').unwrap_or(&walk).to_string();
            let mut s = format!("gather(from {walk}, at {} from {}", expr(offset), anchor_text(*anchor));
            adjustment(&mut s, adjust);
            if *skip_zero {
                s.push_str(", skip zero");
            }
            s.push(')');
            with(s, elem)
        }
        _ => None,
    }
}

/// `field` of `array`'s elements, or the array alone when there is no path.
fn path_of(array: &str, field: &[String]) -> String {
    if field.is_empty() {
        format!("{array}[]")
    } else {
        format!("{array}[].{}", field.join("."))
    }
}

/// How far the offsets are shifted, left off entirely when they are not.
fn adjustment(s: &mut String, adjust: &Expr) {
    if let Expr::Lit(0) = adjust {
        return;
    }
    let _ = write!(s, " + {}", expr(adjust));
}

fn write_struct(out: &mut Vec<String>, ind: usize, head: &str, def: &StructDef, tail: &str) {
    let attrs = struct_attrs(def);
    let open = if attrs.is_empty() {
        format!("{head}{} ", def.name)
    } else {
        format!("{head}{} ({}) ", def.name, attrs.join(", "))
    };
    if def.fields.is_empty() {
        out.push(format!("{}{open}{{}}{tail}", pad(ind)));
        return;
    }
    out.push(format!("{}{open}{{", pad(ind)));
    for f in &def.fields {
        write_field(out, ind + 1, f);
    }
    out.push(format!("{}}}{tail}", pad(ind)));
}

fn struct_attrs(def: &StructDef) -> Vec<String> {
    let mut a = Vec::new();
    if let Some(n) = &def.named_by {
        a.push(format!("named by {n}"));
    }
    if let Some(n) = &def.contents {
        a.push(format!("contents {n}"));
    }
    if let Some(u) = &def.unit {
        a.push(format!("unit {u}"));
    }
    if def.inline {
        a.push("inline".to_string());
    }
    if let Some(p) = &def.packed {
        a.push(format!("packed {p}"));
    }
    if !def.machinery.is_empty() {
        a.push(format!("machinery {}", join_names(&def.machinery)));
    }
    if !def.payload.is_empty() {
        a.push(format!("payload {}", join_names(&def.payload)));
    }
    if !def.line.is_empty() {
        let parts: Vec<String> = def.line.iter().map(line_part).collect();
        a.push(format!("line {}", parts.join(" then ")));
    }
    a
}

fn join_names(names: &[Arc<str>]) -> String {
    names.iter().map(|n| n.to_string()).collect::<Vec<_>>().join(" ")
}

/// One field of a structure's one-line reading: which field, how it is worded,
/// and the reading that says nothing and is left off.
fn line_part(p: &LinePart) -> String {
    let mut s = p.field.to_string();
    if !p.word.is_empty() {
        let _ = write!(s, " worded {:?}", p.word);
    }
    if !p.quiet.is_empty() {
        let _ = write!(s, " except {:?}", p.quiet);
    }
    s
}

/// A choice of types: the cases, then the default under `_`.
fn write_choice(out: &mut Vec<String>, ind: usize, head: &str, on: &str, cases: &[(String, &Ty)], default: &Ty, tail: &str) {
    out.push(format!("{}{head}{on} {{", pad(ind)));
    for (key, ty) in cases {
        write_ty(out, ind + 1, &format!("{key} => "), ty, "");
    }
    write_ty(out, ind + 1, "_ => ", default, "");
    out.push(format!("{}}}{tail}", pad(ind)));
}

/// A braced body of short items, greedily filled across lines.
fn write_body(out: &mut Vec<String>, ind: usize, head: &str, items: &[String], tail: &str) {
    out.push(format!("{}{head}{{", pad(ind)));
    let inner = pad(ind + 1);
    let mut line = String::new();
    for (i, item) in items.iter().enumerate() {
        let last = i + 1 == items.len();
        let piece = if last { item.clone() } else { format!("{item}, ") };
        if !line.is_empty() && inner.len() + line.chars().count() + piece.chars().count() > WIDTH {
            out.push(format!("{inner}{}", line.trim_end()));
            line = String::new();
        }
        line.push_str(&piece);
    }
    if !line.is_empty() {
        out.push(format!("{inner}{}", line.trim_end()));
    }
    out.push(format!("{}}}{tail}", pad(ind)));
}

/// A field: its name, its type, and everything the template says about it on
/// top of its type. The extras fill out the field's own line and spill onto
/// continuation lines when there is no room.
fn write_field(out: &mut Vec<String>, ind: usize, f: &Field) {
    write_ty(out, ind, &format!("{}: ", f.name), &f.ty, "");
    let extras = field_extras(f);
    if extras.is_empty() {
        return;
    }
    let cont = format!("{}    ", pad(ind));
    let mut line = out.pop().unwrap_or_default();
    for extra in extras {
        let sep = if line.trim_end() == line.trim_end().trim_start_matches(' ') && line.is_empty() { "" } else { "  " };
        if line.chars().count() + sep.len() + extra.chars().count() > WIDTH {
            out.push(line);
            line = format!("{cont}{extra}");
        } else {
            line.push_str(sep);
            line.push_str(&extra);
        }
    }
    out.push(line);
}

fn field_extras(f: &Field) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(e) = &f.name_from {
        out.push(format!("name from {}", expr(e)));
    }
    if let Some(e) = &f.elem_name_from {
        out.push(format!("element name from {}", expr(e)));
    }
    if f.aside {
        out.push("aside".to_string());
    }
    if let Some(t) = &f.time {
        out.push(time_text(t));
    }
    for c in &f.checks {
        out.push(format!("check {}", check_text(c)));
    }
    if let Some(c) = &f.elem_check {
        out.push(format!("element check {}", check_text(c)));
    }
    out
}

fn check_text(c: &Check) -> String {
    let mut s = format!("{} over {}", c.algorithm.as_str(), covers_text(&c.over));
    if let Some(w) = &c.when {
        let _ = write!(s, " when {}", expr(w));
    }
    if let Some(b) = c.blank {
        let _ = write!(s, " blanking 0x{b:02x}");
    }
    s
}

fn covers_text(c: &Covers) -> String {
    match c {
        Covers::Field { name } => named_text(name),
        Covers::Unpacked { name, len } => match len {
            Some(l) => format!("unpacked {} of {} bytes", named_text(name), expr(l)),
            None => format!("unpacked {}", named_text(name)),
        },
        Covers::UnpackedMember { name, packed } => {
            format!("unpacked {} member packed at {}", named_text(name), named_text(packed))
        }
        Covers::Run { at, len } => format!("{} bytes at {}", expr(len), expr(at)),
        Covers::UpToHere => "everything before this field".to_string(),
    }
}

fn named_text(n: &Named) -> String {
    match n {
        Named::Here(name) => name.to_string(),
        Named::Earlier(path) => format!("earlier({})", path.join(".")),
        Named::Elem { array, index } => format!("{}[{}]", array.join("."), expr(index)),
    }
}

fn time_text(t: &Time) -> String {
    let epoch = match &t.epoch {
        Epoch::Counted(c) => match (c.zero, c.step_nanos) {
            (0, 1_000_000_000) => "unix".to_string(),
            (0, 1_000_000) => "unix millis".to_string(),
            (0, 1_000) => "unix micros".to_string(),
            (0, 1) => "unix nanos".to_string(),
            (-2_082_844_800, 1_000_000_000) => "mac".to_string(),
            (-11_644_473_600, 100) => "filetime".to_string(),
            (zero, step) => format!("counted from {zero} by {step}ns"),
        },
        Epoch::Dos => "dos".to_string(),
        Epoch::DosHalves { date, time } => format!("dos halves(date {date}, time {time})"),
    };
    let zone = match t.zone {
        Zone::Utc => "utc",
        Zone::Local => "local",
        Zone::Unknown => "zone unknown",
    };
    let mut s = format!("time {epoch} {zone}");
    if let Some(u) = t.unset {
        let _ = write!(s, " unset {u}");
    }
    s
}

fn anchor_text(a: Anchor) -> String {
    match a {
        Anchor::Window => "window".to_string(),
        Anchor::File => "file".to_string(),
        Anchor::Origin => "origin".to_string(),
        Anchor::SelfAligned(n) => format!("own start aligned to {n}"),
    }
}

fn until_text(u: &Until) -> String {
    match u {
        Until::End => "until end".to_string(),
        // `is` compares the bytes as they are written; `==` compares the value
        // the field reads as. Two questions, two words.
        Until::FieldBytes { field, bytes } => format!("until element.{field} is {}", bytes_lit(bytes)),
        Until::FieldValue { field, value } => format!("until element.{field} == {}", tag_lit(*value)),
    }
}

fn step_text(s: &Step) -> String {
    match s {
        Step::Field(n) => format!(".{n}"),
        Step::Tagged { key, tag, shown } => format!(".{shown}[{} = {}]", key.join("."), tag_text(tag)),
        Step::Each => "[]".to_string(),
        Step::Fields(names) => format!(".{{{}}}", names.join(", ")),
    }
}

fn tag_text(t: &Tag) -> String {
    match t {
        Tag::Int(v) => tag_lit(*v),
        Tag::Bytes(b) => bytes_lit(b),
        Tag::Computed(e) => expr(e),
        Tag::ComputedText(e) => format!("text {}", expr(e)),
        Tag::Text(s) => format!("{s:?}"),
    }
}

fn packing(p: &Packing) -> String {
    match p {
        Packing::Fixed(c) => codec(*c),
        Packing::Lzma1 { props, dict_size, unpacked } => {
            let mut s = format!("lzma props {} dict {}", expr(props), expr(dict_size));
            if let Some(u) = unpacked {
                let _ = write!(s, " unpacked {}", expr(u));
            }
            s
        }
        Packing::Rar5 { dictionary, unpacked } => {
            format!("rar5 dictionary {} unpacked {}", expr(dictionary), expr(unpacked))
        }
    }
}

/// A codec, with the numbers that pick it apart where it has any: `lzhuf` is
/// not one decoder but thirteen, one per window size.
fn codec(c: crate::codec::Codec) -> String {
    use crate::codec::Codec;
    match c {
        Codec::Lzma1 { props, dict_size, unpacked } => match unpacked {
            Some(u) => format!("lzma props {props} dict {dict_size} unpacked {u}"),
            None => format!("lzma props {props} dict {dict_size}"),
        },
        Codec::Lha { window_bits } => format!("lzhuf window {window_bits}"),
        Codec::Rar5 { window_bits, unpacked } => format!("rar5 window {window_bits} unpacked {unpacked}"),
        Codec::PngUnfilter { stride, bpp } => format!("png unfilter stride {stride} bpp {bpp}"),
        other => other.as_str().to_string(),
    }
}

fn unset_text(u: &Unset) -> String {
    match u {
        Unset::Int(v) => v.to_string(),
        Unset::Float(f) => format!("{f:?}"),
    }
}

fn enum_cases(def: &EnumDef) -> Vec<String> {
    let num = |v: i128| if def.hex { hex_lit(v) } else { v.to_string() };
    let mut out: Vec<String> = def.cases.iter().map(|(v, n)| format!("{} = {n}", num(*v))).collect();
    for s in &def.spans {
        out.push(format!("from {} by {} = {:?}", num(s.from), s.step, s.label));
    }
    out
}

/// A type on one line, or nothing for the types that cannot be written on one.
fn inline(ty: &Ty) -> Option<String> {
    let s = match ty {
        Ty::UInt { bits, endian } => format!("u{bits}{}", end(*endian)),
        Ty::Int { bits, endian } => format!("i{bits}{}", end(*endian)),
        Ty::SignMagnitude { bits, endian } => format!("signmag{bits}{}", end(*endian)),
        Ty::UIntExpr { bits, endian } => format!("u({}){}", expr(bits), end(*endian)),
        Ty::Nullable { inner, unset } => format!("{} unset {}", inline(inner)?, unset_text(unset)),
        Ty::F16(e) => format!("f16{}", end(*e)),
        Ty::BF16(e) => format!("bf16{}", end(*e)),
        Ty::F32(e) => format!("f32{}", end(*e)),
        Ty::F64(e) => format!("f64{}", end(*e)),
        Ty::F80(e) => format!("f80{}", end(*e)),
        Ty::F8 { e4m3 } => if *e4m3 { "f8e4m3" } else { "f8e5m2" }.to_string(),
        Ty::Computed(e) => format!("computed {}", expr(e)),
        Ty::ComputedText(e) => format!("computed text {}", expr(e)),
        Ty::Leb128 { signed } => if *signed { "sleb128" } else { "leb128" }.to_string(),
        Ty::Zigzag => "zigzag".to_string(),
        Ty::EbmlVint { strip_marker } => {
            if *strip_marker { "ebml vint" } else { "ebml vint with marker" }.to_string()
        }
        Ty::Vlq => "vlq".to_string(),
        Ty::Fixed { bits, frac, endian, signed } => {
            let word = if *signed { "fixed" } else { "ufixed" };
            format!("{word} {}.{frac}{}", bits.saturating_sub(*frac), end(*endian))
        }
        Ty::Magic(b) => format!("magic {}", bytes_lit(b)),
        Ty::Bytes(e) => format!("bytes[{}]", expr(e)),
        Ty::Str { len, enc } => format!("{} {}", strlen(len), encoding(enc)),
        Ty::TextInt { len, radix } => format!("{} base {radix} as number", strlen(len)),
        Ty::Struct(_) => return None,
        Ty::Array { elem, count } => format!("{}[{}]", inline(elem)?, expr(count)),
        Ty::Repeat { elem, until } => format!("repeat({}) {}", until_text(until), inline(elem)?),
        Ty::PointerList { .. } | Ty::Chain { .. } | Ty::Gather { .. } => {
            let (head, inner) = wrapper("", ty)?;
            format!("{head}{}", inline(inner)?)
        }
        Ty::SqliteVarint => "sqlite varint".to_string(),
        Ty::SevenZipNumber => "7z number".to_string(),
        Ty::At { anchor, at, inner } => format!("at({} from {}) {}", expr(at), anchor_text(*anchor), inline(inner)?),
        Ty::Sized { size, inner } => format!("sized({}) {}", expr(size), inline(inner)?),
        Ty::Origin { inner } => format!("origin {}", inline(inner)?),
        Ty::SizedBits { bits, inner } => format!("sizedbits({}) {}", expr(bits), inline(inner)?),
        Ty::Switch { on, cases, default } => {
            let mut parts = Vec::new();
            for (k, t) in cases.iter() {
                parts.push(format!("{} => {}", tag_lit(*k), inline(t)?));
            }
            parts.push(format!("_ => {}", inline(default)?));
            format!("switch {} {{{}}}", expr(on), parts.join(", "))
        }
        Ty::Enum { inner, def } => format!("{} enum {} {{{}}}", inline(inner)?, def.name, enum_cases(def).join(", ")),
        Ty::Flags { inner, def } => {
            let bits: Vec<String> = def.bits.iter().map(|(b, n)| format!("bit {b} = {n}")).collect();
            format!("{} flags {} {{{}}}", inline(inner)?, def.name, bits.join(", "))
        }
        Ty::Json(shape, schema) => match schema {
            Some(s) => format!("json {} {}", shape.name(), json_schema(s)),
            None => format!("json {}", shape.name()),
        },
        Ty::Match { on, cases, default } => {
            let mut parts = Vec::new();
            for (k, t) in cases.iter() {
                parts.push(format!("{k:?} => {}", inline(t)?));
            }
            parts.push(format!("_ => {}", inline(default)?));
            format!("match {} {{{}}}", expr(on), parts.join(", "))
        }
        // The four placeholders. What these read is in a decoder, not in the
        // IR, so the word is the whole of what can be written.
        Ty::Insn { isa } => format!("instruction {}", isa.name()),
        Ty::Named(n) => n.to_string(),
        Ty::Decoded { codec, inner } => format!("decoded({}) {}", packing(codec), inline(inner)?),
        Ty::Traced { part } => match part {
            TracedPart::Blocks => "traced blocks".to_string(),
            TracedPart::Block(i) => format!("traced block {i}"),
            TracedPart::Symbols(i) => format!("traced symbols {i}"),
        },
        Ty::CodeBits { name, width } => format!("codebits {name} ({})", sizing(*width)),
    };
    Some(s)
}

/// A type in as few words as identify it, for the places a full reading cannot
/// go: a structure answers with its name.
fn brief(ty: &Ty) -> String {
    match inline(ty) {
        Some(s) => s,
        None => match ty {
            Ty::Struct(d) => d.name.clone(),
            _ => String::new(),
        },
    }
}

fn sizing(s: Sizing) -> &'static str {
    s.as_str()
}

/// What a JSON schema says over the top of the text: the type it renames the
/// value to, and what its members are.
fn json_schema(s: &JsonSchema) -> String {
    let mut parts: Vec<String> = Vec::new();
    for (k, v) in &s.members {
        parts.push(format!("{k}: {}", json_schema(v)));
    }
    if let Some(rest) = &s.rest {
        parts.push(format!("_: {}", json_schema(rest)));
    }
    let name = s.type_name.clone().unwrap_or_default();
    if parts.is_empty() {
        name
    } else {
        format!("{name} {{{}}}", parts.join(", "))
    }
}

fn end(e: Endian) -> &'static str {
    match e {
        Endian::Little => "le",
        Endian::Big => "be",
    }
}

fn strlen(l: &StrLen) -> String {
    match l {
        StrLen::Fixed(e) => format!("text[{}]", expr(e)),
        StrLen::Padded { size, pad } => format!("text[{}] padded with 0x{pad:02x}", expr(size)),
        StrLen::Scan { skip, ends, comment } => {
            let mut s = "text token".to_string();
            if !skip.is_empty() {
                let _ = write!(s, " skipping {}", bytes_hex(skip));
            }
            if !ends.is_empty() {
                let _ = write!(s, " ending at {}", bytes_hex(ends));
            }
            if let Some((a, b)) = comment {
                let _ = write!(s, " past 0x{a:02X} to 0x{b:02X}");
            }
            s
        }
        StrLen::Terminated { end, or_end } => {
            format!("text until 0x{end:02x}{}", if *or_end { " or end" } else { "" })
        }
    }
}

fn encoding(e: &Encoding) -> String {
    match e {
        Encoding::Utf8 => "utf8".to_string(),
        Encoding::Ascii => "ascii".to_string(),
        Encoding::Latin1 => "latin1".to_string(),
        Encoding::Cp437 => "cp437".to_string(),
        Encoding::Cp437Screen => "cp437 screen".to_string(),
        Encoding::Utf16(e) => format!("utf16{}", end(*e)),
        Encoding::Bom { fallback } => format!("bom({})", encoding(fallback)),
        Encoding::Unknown => "encoding unknown".to_string(),
        Encoding::P8scii => "p8scii".to_string(),
    }
}

/// A run of bytes: as text when every one of them is printable, and as hex
/// otherwise. Both forms say the same thing about the same bytes.
fn bytes_lit(b: &[u8]) -> String {
    if !b.is_empty() && b.iter().all(|c| (0x20..0x7f).contains(c)) {
        format!("{:?}", String::from_utf8_lossy(b))
    } else {
        bytes_hex(b)
    }
}

fn bytes_hex(b: &[u8]) -> String {
    b.iter().map(|c| format!("{c:02X}")).collect::<Vec<_>>().join(" ")
}

/// A number as the template means it. A four-letter tag read big-endian is
/// written as its letters, since that is the number a reader can check against
/// the file; anything else is decimal.
fn int_lit(v: i128) -> String {
    if let Some(text) = as_ascii(v) {
        return format!("'{text}'");
    }
    v.to_string()
}

/// The same, for the constants a format fixes rather than the numbers it
/// counts with: a case of a switch, a label on a record. A tag of 256 or more
/// is written in hex, which is the form a specification writes it in and the
/// form a reader checks against the bytes.
fn tag_lit(v: i128) -> String {
    if let Some(text) = as_ascii(v) {
        return format!("'{text}'");
    }
    if v.unsigned_abs() >= 256 {
        return hex_lit(v);
    }
    v.to_string()
}

/// A number in hex, with a leading zero where it would otherwise be written to
/// an odd number of digits, so that a tag of four bytes looks like four bytes.
fn hex_lit(v: i128) -> String {
    let sign = if v < 0 { "-" } else { "" };
    let digits = format!("{:x}", v.unsigned_abs());
    let pad = if digits.len() % 2 == 1 { "0" } else { "" };
    format!("{sign}0x{pad}{digits}")
}

/// The bytes of `v` read big-endian, when they spell something a reader would
/// recognise: printable, at least two of them, and with letters in it, so that
/// an ordinary number is never dressed up as text.
fn as_ascii(v: i128) -> Option<String> {
    if v <= 0 {
        return None;
    }
    let mut bytes = Vec::new();
    let mut n = v;
    while n > 0 {
        bytes.push((n & 0xff) as u8);
        n >>= 8;
    }
    bytes.reverse();
    if bytes.len() < 2 || bytes.len() > 8 {
        return None;
    }
    if !bytes.iter().all(|c| (0x20..0x7f).contains(c)) {
        return None;
    }
    if bytes.iter().filter(|c| c.is_ascii_alphabetic()).count() < 2 {
        return None;
    }
    Some(String::from_utf8_lossy(&bytes).to_string())
}

/// How tightly an expression binds, loosest first. Brackets are written only
/// where a looser expression sits inside a tighter one.
///
/// Spaced out so that a new operator lands between two existing ones without
/// renumbering: the boolean and comparison variants being added go at 20, 30
/// and 40, and `%` beside `*` at 80.
fn prec(e: &Expr) -> u32 {
    match e {
        Expr::Or(..) => 10,
        Expr::Less(..) => 40,
        Expr::And(..) => 50,
        Expr::Shl(..) | Expr::Shr(..) => 60,
        Expr::Add(..) | Expr::Sub(..) => 70,
        Expr::Mul(..) | Expr::Div(..) => 80,
        _ => 100,
    }
}

/// An expression as text.
pub fn expr(e: &Expr) -> String {
    write_expr(e, 0, false)
}

/// `mask` says the literal here is a mask rather than a count, and is written
/// in hex: `flags >> 3 & 0x3f` reads as a run of bits and `flags >> 3 & 63`
/// does not.
fn write_expr(e: &Expr, outer: u32, mask: bool) -> String {
    let here = prec(e);
    let wrap = |s: String| if here < outer { format!("({s})") } else { s };
    // The right side of a non-associating operator binds one step tighter, so
    // `a - (b + c)` keeps its brackets and `a - b - c` does not grow any.
    let two = |a: &Expr, b: &Expr, op: &str| {
        wrap(format!("{} {op} {}", write_expr(a, here, false), write_expr(b, here + 1, false)))
    };
    let path = |array: &str, index: &Expr, field: &[String]| {
        let mut s = format!("{array}[{}]", expr(index));
        for f in field {
            s.push('.');
            s.push_str(f);
        }
        s
    };
    match e {
        // Adding a variant is one arm here and one in `prec`. The words the
        // ones being written now are to use: `%` for `Mod` (at the precedence
        // of `*`), `== != <= >= >` for the comparisons (beside `<`), `and`,
        // `or` and `not` for the booleans (looser than a comparison, and
        // `Expr::Or` is written `or else` here so the word is free),
        // `when a then b else c` for `Cond`, `pos` for `Pos`, `size of window`
        // for `WindowSize`, `count of x` for `LenOf(x)`, and a `doc` slot
        // renders as a `// text` line above the field it is on.
        Expr::Lit(v) => {
            if mask && *v > 9 {
                hex_lit(*v)
            } else {
                int_lit(*v)
            }
        }
        Expr::Ref(n) => n.to_string(),
        Expr::Remaining => "remaining".to_string(),
        Expr::SizeOf(n) => format!("sizeof({n})"),
        Expr::BitsOf(n) => format!("bitsof({n})"),
        Expr::Idx => "index".to_string(),
        Expr::Elem { array, index, field } => path(array, index, field),
        Expr::ElemWithin { path: into, index, field } => path(&into.join("."), index, field),
        Expr::Tagged(t) => {
            let array = match &t.array {
                Some(a) => expr(a),
                None => "earlier".to_string(),
            };
            let field = if t.field.is_empty() { String::new() } else { format!(".{}", t.field.join(".")) };
            format!("{array}[{} = {}]{field}", t.key.join("."), tag_text(&t.tag))
        }
        Expr::Placer(e) => match &**e {
            Expr::Ref(n) => format!("descriptor.{n}"),
            Expr::Within(f) => format!("descriptor.{}", f.join(".")),
            other => format!("descriptor.({})", expr(other)),
        },
        Expr::Product { array, index, field } => format!("product({})", path(array, index, field)),
        Expr::ProductOf(n) => format!("product({n})"),
        Expr::SumOf(n) => format!("sum({n})"),
        Expr::MaxOf(n) => format!("largest({n})"),
        Expr::PopCount(n) => format!("setbits({n})"),
        Expr::Deduced(d) => format!(
            "deduced({})",
            match d {
                Deduce::PayloadShape => "payload shape",
                Deduce::PayloadCount => "payload count",
                Deduce::Builds => "builds",
            }
        ),
        Expr::Peek { bits, endian } => format!("peek(u{bits}{})", end(*endian)),
        Expr::PeekAt { skip, bits, endian } => format!("peek(u{bits}{} at {} bits)", end(*endian), expr(skip)),
        Expr::ToMarker { lead, unless } => {
            if unless.is_empty() {
                format!("tomarker({})", bytes_hex(lead))
            } else {
                format!("tomarker({} unless {})", bytes_hex(lead), bytes_hex(unless))
            }
        }
        Expr::Find { needle, last } => {
            let word = if *last { "findlast" } else { "find" };
            format!("{word}({})", bytes_lit(needle))
        }
        Expr::Prev(n) => format!("previous({n})"),
        Expr::Sibling(f) => format!("earlier({})", f.join(".")),
        Expr::Within(f) => f.join("."),
        Expr::Or(a, b) => two(a, b, "or else"),
        Expr::Add(a, b) => two(a, b, "+"),
        Expr::Sub(a, b) => two(a, b, "-"),
        Expr::Mul(a, b) => two(a, b, "*"),
        Expr::Div(a, b) => two(a, b, "/"),
        Expr::Less(a, b) => two(a, b, "<"),
        Expr::Shl(a, b) => two(a, b, "<<"),
        Expr::Shr(a, b) => two(a, b, ">>"),
        Expr::And(a, b) => {
            wrap(format!("{} & {}", write_expr(a, here, true), write_expr(b, here + 1, true)))
        }
        Expr::Min(a, b) => format!("min({}, {})", expr(a), expr(b)),
        Expr::Max(a, b) => format!("max({}, {})", expr(a), expr(b)),
        Expr::PadTo { n, align } => format!("padding({}, {align})", expr(n)),
        Expr::DivCeil(a, b) => format!("ceil({} / {})", write_expr(a, 80, false), write_expr(b, 81, false)),
        Expr::Log2(a) => format!("log2({})", expr(a)),
        Expr::Bit(a, i) => format!("bit({}, {i})", expr(a)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::template::Expr as E;

    #[test]
    fn brackets_appear_only_where_they_change_the_reading() {
        let sum = E::field("a").add(E::field("b"));
        assert_eq!(expr(&sum.clone().mul(E::lit(2))), "(a + b) * 2");
        assert_eq!(expr(&E::field("a").mul(E::lit(2)).add(E::field("b"))), "a * 2 + b");
        assert_eq!(expr(&E::field("a").sub(sum)), "a - (a + b)");
        // A mask binds tighter than a comparison and looser than a shift, so
        // the shape a bit field is written in needs no brackets at all.
        let field = E::field("w").shr(E::lit(3)).and(E::lit(0x3f));
        assert_eq!(expr(&field), "w >> 3 & 0x3f");
        assert_eq!(expr(&field.less_than(E::lit(4))), "w >> 3 & 0x3f < 4");
    }

    #[test]
    fn a_four_letter_tag_reads_as_its_letters() {
        assert_eq!(int_lit(0x4948_4452), "'IHDR'");
        // Not a number that merely happens to be printable.
        assert_eq!(int_lit(0x3132), "12594");
        assert_eq!(int_lit(4), "4");
    }

    #[test]
    fn a_template_renders_the_same_way_twice() {
        let t = Template::new("demo", Ty::u32(Endian::Big));
        assert_eq!(render(&t), render(&t));
        assert_eq!(render(&t), "template demo\n\nroot u32be\n");
    }
}
