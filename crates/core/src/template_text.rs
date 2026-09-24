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
//!                {type}                            in alphabetical order
//! type        := "type" struct                     a structure filed under its own name
//!              | "type" NAME "=" ty                anything else, under the name a
//!                                                  `Named` reaches it by
//!
//! ty          := struct | wrapper ty | ty annotation | leaf
//! struct      := NAME ["(" attr {"," attr} ")"] "{" {field} "}"
//! field       := NAME ":" ty {extra}
//!
//! wrapper     := "sized(" expr ")"                  parsed inside an N-byte window
//!              | "sizedbits(" expr ")"              the same, counted in bits
//!              | "optional(when" expr ")"           absent when the expression is zero
//!              | "at(" expr "from" anchor ")"       placed there, not here
//!              | "origin"                           offsets inside count from here
//!              | "decoded(" codec ")"               unpacked, and read from that
//!              | "repeat(" until ")"                one after another
//!              | "pointers(" ... ")"                placed at offsets read earlier
//!              | "chain(" ... ")"                   each element points at the next
//!              | "gather(" ... ")"                  placed at offsets found by a walk
//!              | "stitched(" ... ")"                one stream joined from runs a walk finds
//!              | "raster(" ... ")"                  pixels placed from rows or Adam7 passes
//! annotation  := "[" expr "]"                       a list of that many elements
//!              | "enum" NAME "{" {int "=" NAME} "}"
//!              | "flags" NAME "{" {"bit" int "=" NAME} "}"
//!              | "unset" number                     that value means "nothing here"
//! anchor      := "file" | "window" | "origin" | "own start aligned to" int
//! until       := "until end" | "until element." NAME ("is" bytes | "==" int)
//!              | "until" expr                       asked inside each element
//!              | "while" expr                       asked before each element,
//!                                                   beside the list itself
//!
//! A count binds to the word in front of it, so an element written in more
//! than one word is bracketed first: `(text[4] utf8)[3]` is three of them,
//! where `text[4] utf8[3]` would read as an index into the encoding. An
//! element that closes with a brace has already said where it ends and keeps
//! no brackets.
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
//! attr        := "named by" path {"|" path}         which field names one of these,
//!                                                   or a path down to it; the first
//!                                                   that reads wins
//!              | "contents" NAME                    which field is merely the body
//!              | "unit" NAME                        what one is called when counted
//!              | "inline"                           one row, not one row per field
//!              | "overlap"                           every field starts where the
//!                                                   structure does, as a union does
//!              | "packed" NAME                      contents only the format unpacks
//!              | "machinery" NAME {NAME}            fields that are this structure's
//!                                                   own plumbing, whatever they decide
//!              | "payload" NAME {NAME}              fields that are the point
//!              | "line" part {"then" part}          how one reads on a single line
//!              | "encoding wrapper of" NAME         a path through one is named
//!                                                   without it or that field
//!              | "encoding member tagged" NAME "of" NAME
//!                                                   a path names it by the first
//!                                                   field, without the second
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
//! the shape of the value, `text[expr]` and its encoding, `computed expr`,
//! `computed text expr` and `computed real expr` for a value with no bytes of
//! its own. A `computed real` works its expression out as reals.
//! `schema(kind from walk, key expr, expr)` is a type the file describes: the
//! builder registered as `kind` makes it from the description the key names,
//! among the records the walk reaches. A key part in double quotes is text the
//! template fixed; any other is read where the field is.
//!
//! A walk is written as a path: `.name` into a field, `[]` into every element,
//! `.{a, b}` into each of those fields, `.shown[key = tag]` into the one element
//! labelled that way, `.(stream)` into the compressed run or joined stream
//! under the node, wherever the format that wrote it put it, and `..name` into
//! every field of that name at any depth under it. The brackets are what keep
//! the stream step from reading as a field called `stream`.
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
//!   c ? a : b               a when c is nonzero, else b
//!   or else                 the left side, or the right when the left is zero
//!   or                      boolean or
//!   and                     boolean and
//!   not                     boolean not
//!   < == != <= > >=         comparison
//!   |                       bitwise or
//!   ^                       bitwise exclusive or
//!   &                       mask
//!   << >>                   shifts
//!   + -
//!   * / %
//!   name  literal  call()   everything else
//! ```
//!
//! `~x` flips every bit of a number, over the whole 128 bits the arithmetic
//! here is in, so `~0` is -1 and a complement within a word is written with
//! the mask that says so: `~x & 0xffffffff`. It binds as tightly as a call.
//!
//! Four places take a bracket whatever the table says, because the reading
//! without one is not the only reading: what `not` negates, what `~` flips,
//! what `start of` names, and a ternary inside a ternary on either side of
//! the colon.
//!
//! A name is a field declared earlier, in this structure or in one it sits
//! inside. `a.b` is a path down into an earlier field. `index` is this
//! element's place in the list it sits in. `remaining` is from here to the end
//! of the container, `pos` is from the start of the window to here, `size of
//! window` is the window; `pos in space` and `size of space` are the same two
//! measured from the front of the whole file, or of what a compressed run
//! unpacked to, whatever windows lie in between; all in bytes. `count of f` is
//! how many elements an earlier list has and `start of x` where an element
//! found by expression begins. Then the calls: `sizeof(f)` `bitsof(f)` `sum(f)`
//! `product(f)` `largest(f)` `setbits(f)` `min(a, b)` `max(a, b)`
//! `ceil(a / b)` `log2(a)` `bit(x, 3)` `padding(n, 4)` (the padding *after* n
//! bytes, to the next multiple of 4), `peek(u8be)` and
//! `peek(u16le at expr bits)` and `peek(u32be at expr bits in space)` (read
//! without consuming: here, a distance on from here, or an address in the
//! file), `tomarker(FF unless 00)`, `find("endobj")`, `run(not 7B 7D 5C)` and
//! `findlast("startxref")` (how far it is to
//! there), `previous(f)` (the element before this one), `earlier(a.b)` and
//! `earlier[key = tag].f` (the nearest earlier element that has one),
//! `array[i].f`, `descriptor.f` (the record that placed this element).
//! And the four a real is made with: `real(f)` (the number the text of a field
//! spells, `2.0E+01` included), `pow2(e)` and `pow10(e)` (two or ten to a whole
//! power, a fraction when the power is negative) and `trunc(x)` (the whole part
//! of a real, which is the only way one becomes a size).
//!
//! Numbers are decimal, with three exceptions. A number whose bytes spell
//! printable ASCII with letters in it is written as those bytes in single
//! quotes wherever it appears: `'IHDR'` is 1,229,472,850 read big-endian,
//! which is what a switch on a four-letter tag is keyed on. A mask of ten or
//! more -- the right side of `&` -- is hex, so a run of bits reads as bits. And
//! a constant the format fixed rather than counts with, which is a case of a
//! `switch`, the label on a record, the value a `repeat` stops at, or a value
//! of an enumeration declared in hex, is written in hex from 256 up, to an even
//! number of digits: `0x04034b50`. A real is written with a point, `1.0`, so
//! that it is never read as the whole number beside it. Byte strings are `"SQLite format 3\0"` when
//! they are mostly letters, with the escapes a string uses for the rest, and
//! `89 50 4E 47 0D 0A 1A 0A` otherwise. Text is in double quotes.
//!
//! Every line is folded at [`WIDTH`] columns, carrying on four spaces further
//! in than the line it continues, and folded as shallow in its brackets as it
//! can be.

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
            // A shallow fold is worth having only while it still fills the
            // line. The one place it does not is a field whose whole type is
            // one bracketed term: folding at the only shallow point there
            // would leave a line holding nothing but the field's name.
            let kept: usize = lead + words[start..best].iter().map(|w| w.chars().count() + 1).sum::<usize>();
            if kept * 2 >= WIDTH {
                end = best;
            }
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
/// One type on its own, written the way [`render`] writes it inside a template.
///
/// The diagram tells two type definitions apart by what they say rather than by
/// where they are, and this is what they say: two structures that print
/// identically are one type drawn once, and an ELF's little-endian and
/// big-endian section headers print differently (`u32 le` against `u32 be`) and
/// stay two. Reusing the renderer rather than writing a second one is the whole
/// point: a distinction the IR text draws is a distinction the diagram draws,
/// and neither can drift from the other.
///
/// Unwrapped and unfolded: no line is broken to a width, since nothing reads
/// this and a fold would only make two spellings of one type.
pub(crate) fn ty_text(ty: &Ty) -> String {
    let mut out = Vec::new();
    write_ty(&mut out, 0, "", ty, "");
    out.join("\n")
}

fn write_ty(out: &mut Vec<String>, ind: usize, head: &str, ty: &Ty, tail: &str) {
    if let Some(head) = wrapper(head, ty) {
        return write_ty(out, ind, &head.0, head.1, tail);
    }
    match ty {
        // Postfix: a list is its element type and how many of them.
        Ty::Array { elem, count } => {
            // An element of more than one word is bracketed first, so that the
            // count cannot read as an index into the last of them. See
            // [`brackets`].
            let (open, close) = brackets(elem);
            let head = format!("{head}{open}");
            return write_ty(out, ind, &head, elem, &format!("{close}[{}]{tail}", expr(count)));
        }
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
        // An enumeration over a structure is nothing any format writes, and
        // naming the values of one would say nothing. It is still written out
        // whole rather than summarised: the structure opens its own block and
        // the names follow it.
        Ty::Enum { inner, def } => {
            let cases = enum_cases(def);
            match inline(inner) {
                Some(one) => write_body(out, ind, &format!("{head}{one} enum {} ", def.name), &cases, tail),
                None => write_ty(out, ind, head, inner, &format!(" enum {} {{{}}}{tail}", def.name, cases.join(", "))),
            }
        }
        Ty::Flags { inner, def } => {
            let cases: Vec<String> = def.bits.iter().map(|(b, n)| format!("bit {b} = {n}")).collect();
            match inline(inner) {
                Some(one) => write_body(out, ind, &format!("{head}{one} flags {} ", def.name), &cases, tail),
                None => write_ty(out, ind, head, inner, &format!(" flags {} {{{}}}{tail}", def.name, cases.join(", "))),
            }
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
        Ty::When { cond, inner } => with(format!("optional(when {})", expr(cond)), inner),
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
        // The walk to the parts, written the way a gather's is, and the two
        // lengths only where the template gives them.
        Ty::Stitched { from, part_len, len, inner } => {
            let walk: String = from.iter().map(step_text).collect::<Vec<_>>().join("");
            let walk = walk.strip_prefix('.').unwrap_or(&walk).to_string();
            let mut s = format!("stitched(from {walk}");
            if let Some(e) = part_len {
                s.push_str(&format!(", each {}", expr(e)));
            }
            if let Some(e) = len {
                s.push_str(&format!(", total {}", expr(e)));
            }
            s.push(')');
            with(s, inner)
        }
        Ty::Raster { width, height, bits_per_pixel, order, pixel } => with(
            format!("raster({} by {}, {} bits a pixel, {})", expr(width), expr(height), expr(bits_per_pixel), order.as_str()),
            pixel,
        ),
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

/// How far the offsets are shifted, left off entirely when they are not, and
/// written as a subtraction when it shifts them back.
fn adjustment(s: &mut String, adjust: &Expr) {
    match adjust {
        Expr::Lit(0) => {}
        Expr::Lit(v) if *v < 0 => {
            let _ = write!(s, " - {}", v.unsigned_abs());
        }
        other => {
            let _ = write!(s, " + {}", expr(other));
        }
    }
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
    write_doc(out, ind + 1, def.doc.as_deref());
    for f in &def.fields {
        write_doc(out, ind + 1, f.doc.as_deref());
        write_field(out, ind + 1, f);
    }
    out.push(format!("{}}}{tail}", pad(ind)));
}

/// Prose about whatever comes next, as a comment line above it.
///
/// Not a clause on the field's own line: a sentence and a notation on one row
/// would be two readings of the same row. A doc written across several lines
/// keeps its lines.
fn write_doc(out: &mut Vec<String>, ind: usize, doc: Option<&str>) {
    let Some(doc) = doc else { return };
    for line in doc.lines() {
        let line = line.trim_end();
        if line.is_empty() {
            out.push(format!("{}//", pad(ind)));
        } else {
            out.push(format!("{}// {line}", pad(ind)));
        }
    }
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
    if def.overlap {
        a.push("overlap".to_string());
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
    match &def.encoding {
        Some(EncodingStep::Wrapper { through }) => a.push(format!("encoding wrapper of {through}")),
        Some(EncodingStep::Member { tag, through }) => a.push(format!("encoding member tagged {tag} of {through}")),
        None => {}
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
        // Two spaces, so that what a field *is* and what is said about it do
        // not read as one clause.
        if line.chars().count() + 2 + extra.chars().count() > WIDTH {
            out.push(line);
            line = format!("{cont}{extra}");
        } else {
            line.push_str("  ");
            line.push_str(&extra);
        }
    }
    out.push(line);
}

/// What a template says about a field on top of its type, in a fixed order.
///
/// The `doc` slot being added goes here too, but not as a clause: prose is a
/// `// text` line written above the field, since a sentence and a notation on
/// one line is two readings of the same row.
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
    if let Some(v) = &f.valid {
        out.push(valid_text(v));
    }
    for c in &f.checks {
        out.push(format!("check {}", check_text(c)));
    }
    if let Some(c) = &f.elem_check {
        out.push(format!("element check {}", check_text(c)));
    }
    if let Some(t) = &f.table {
        out.push(table_text(t));
    }
    out
}

/// What a field's constraint reads as on the field's own line.
///
/// One clause per shape rather than one clause with an operator in it, so that
/// the common ones read as English and the general one reads as the expression
/// it is. A message the source wrote down is kept: it is the sentence a reader
/// of that pattern was meant to see.
fn valid_text(v: &Valid) -> String {
    match v {
        Valid::Eq(e) => format!("valid {}", expr(e)),
        Valid::Min(e) => format!("valid min {}", expr(e)),
        Valid::Max(e) => format!("valid max {}", expr(e)),
        Valid::Range { min, max } => format!("valid min {} max {}", expr(min), expr(max)),
        Valid::AnyOf(items) => {
            format!("valid one of {}", items.iter().map(expr).collect::<Vec<_>>().join(", "))
        }
        Valid::InEnum => "valid in enum".to_string(),
        Valid::Expr { expr: e, msg } => match msg {
            Some(m) => format!("valid {} saying {m:?}", expr(e)),
            None => format!("valid {}", expr(e)),
        },
        Valid::Finite => "valid finite".to_string(),
    }
}

/// What a field claims about reading as a table, in the order a reader asks:
/// how a row is made, how fast rows come, what a row and a column are called,
/// what the columns are, and which fields describe the whole thing.
///
/// The lists are bracketed rather than run on, because every part of this is
/// separated by commas already and `named left, right, describes ...` reads as
/// one list of four things.
fn table_text(t: &TableShape) -> String {
    let mut parts = Vec::new();
    if let Some(e) = &t.columns {
        parts.push(format!("columns {}", expr(e)));
    }
    if let Some(e) = &t.rate {
        parts.push(format!("rate {}", expr(e)));
    }
    if let Some(w) = &t.row_word {
        parts.push(format!("rows {w}"));
    }
    if let Some(w) = &t.column_word {
        parts.push(format!("column word {w}"));
    }
    if !t.names.is_empty() {
        parts.push(format!("named [{}]", t.names.iter().map(|n| n.to_string()).collect::<Vec<_>>().join(", ")));
    }
    if t.units.iter().any(|u| !u.is_empty()) {
        parts.push(format!("units [{}]", t.units.iter().map(|u| u.to_string()).collect::<Vec<_>>().join(", ")));
    }
    if !t.facts.is_empty() {
        parts.push(format!("describes [{}]", t.facts.iter().map(expr).collect::<Vec<_>>().join(", ")));
    }
    format!("table {}", parts.join(", "))
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
            (YEAR_ZERO, 1_000_000) => "year zero millis".to_string(),
            (YEAR_ZERO, 1_000_000_000) => "year zero".to_string(),
            (zero, step) => format!("counted from {zero} by {step}ns"),
        },
        Epoch::Atomic(a) => match (a.zero_tai_nanos, a.step_nanos) {
            (J2000_TAI_NANOS, 1) => "tt2000".to_string(),
            (GPS_TAI_NANOS, 1_000_000_000) => "gps seconds".to_string(),
            (zero, step) => format!("atomic from {zero}ns tai by {step}ns"),
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
    for u in &t.unset {
        let _ = write!(s, " unset {}", unset_text(u));
    }
    s
}

fn anchor_text(a: Anchor) -> String {
    match a {
        Anchor::Window => "window".to_string(),
        Anchor::File => "file".to_string(),
        Anchor::Origin => "origin".to_string(),
        Anchor::Space => "stream".to_string(),
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
        Until::Cond(e) => format!("until {}", expr(e)),
        Until::While(e) => format!("while {}", expr(e)),
    }
}

fn step_text(s: &Step) -> String {
    match s {
        Step::Field(n) => format!(".{n}"),
        // The word a formula uses for `Expr::Placer`, bracketed as `(stream)`
        // is, so it cannot read as a field called `descriptor`.
        Step::Placer => "(descriptor)".to_string(),
        Step::Tagged { key, tag, shown } => {
            format!(".{shown}[{} = {}]", key.join("."), tag_text(tag, true).unwrap_or_default())
        }
        Step::Each => "[]".to_string(),
        Step::Elements(indices) => format!("[{}]", indices.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(", ")),
        Step::Fields(names) => format!(".{{{}}}", names.join(", ")),
        Step::Stream => ".(stream)".to_string(),
        Step::Deep(n) => format!("..{n}"),
    }
}

/// A walk written as one path, without the dot in front of its first name.
fn walk_text(from: &[Step]) -> String {
    let walk: String = from.iter().map(step_text).collect::<Vec<_>>().join("");
    walk.strip_prefix('.').unwrap_or(&walk).to_string()
}

/// A schema node: which builder, the walk to its descriptions, and the key.
fn schema_text(kind: &str, table: &[Step], key: &[KeyPart]) -> String {
    let parts: Vec<String> = key
        .iter()
        .map(|k| match k {
            KeyPart::Int(e) | KeyPart::Text(e) => expr(e),
            KeyPart::TextLit(t) => format!("{t:?}"),
        })
        .collect();
    format!("schema({kind} from {}, key {})", walk_text(table), parts.join(", "))
}

/// The label a search looks for. `text` marks a key compared as text rather
/// than as a number, which is the whole difference between two searches that
/// would otherwise be written the same way.
///
/// None on the same terms as [`readable`]: a label worked out by reading bytes
/// is not something the reader can go and look at.
///
/// Public to the crate because the relations panel writes the same searches,
/// with the value the file gave in place of the expression that found it, and
/// the constants either side of that have to read the same way in both.
pub(crate) fn tag_text(t: &Tag, probes: bool) -> Option<String> {
    Some(match t {
        Tag::Int(v) => tag_lit(*v),
        Tag::Bytes(b) => bytes_lit(b),
        Tag::Computed(e) => spelled(e, probes)?,
        Tag::ComputedText(e) => format!("text {}", spelled(e, probes)?),
        Tag::Text(s) => format!("{s:?}"),
    })
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
        Packing::PngScanlines(h) => format!(
            "png scanlines width {} height {} depth {} colour {} interlace {}",
            expr(&h.width),
            expr(&h.height),
            expr(&h.bit_depth),
            expr(&h.color_type),
            expr(&h.interlace)
        ),
        Packing::JpegScan { segments } => format!("jpeg baseline segments {}", expr(segments)),
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
        Codec::PngScanlines { width, height, bits_per_pixel, interlace } => {
            format!("png scanlines {width}x{height} bits {bits_per_pixel}{}", if interlace { " adam7" } else { "" })
        }
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
        Ty::IbmF32(e) => format!("ibm32{}", end(*e)),
        Ty::F8 { e4m3 } => if *e4m3 { "f8e4m3" } else { "f8e5m2" }.to_string(),
        Ty::Computed(e) => format!("computed {}", expr(e)),
        Ty::ComputedText(e) => format!("computed text {}", expr(e)),
        Ty::ComputedReal(e) => format!("computed real {}", expr(e)),
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
        Ty::Array { elem, count } => {
            let (open, close) = brackets(elem);
            format!("{open}{}{close}[{}]", inline(elem)?, expr(count))
        }
        Ty::Repeat { elem, until } => format!("repeat({}) {}", until_text(until), inline(elem)?),
        Ty::PointerList { .. } | Ty::Chain { .. } | Ty::Gather { .. } => {
            let (head, inner) = wrapper("", ty)?;
            format!("{head}{}", inline(inner)?)
        }
        Ty::Stitched { .. } | Ty::Raster { .. } => {
            let (head, inner) = wrapper("", ty)?;
            format!("{head}{}", inline(inner)?)
        }
        Ty::Schema { kind, table, key } => schema_text(kind, table, key),
        Ty::SqliteVarint => "sqlite varint".to_string(),
        Ty::SevenZipNumber => "7z number".to_string(),
        Ty::At { anchor, at, inner } => format!("at({} from {}) {}", expr(at), anchor_text(*anchor), inline(inner)?),
        Ty::Sized { size, inner } => format!("sized({}) {}", expr(size), inline(inner)?),
        Ty::When { cond, inner } => format!("optional(when {}) {}", expr(cond), inline(inner)?),
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
        Ty::Json(shape, schema) => match schema.as_ref().map(|s| json_schema(s)) {
            Some(s) if !s.is_empty() => format!("json {} {s}", shape.name()),
            _ => format!("json {}", shape.name()),
        },
        Ty::Pickle(shape) => format!("pickle {}", shape.name()),
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
            TracedPart::Unit(i) => format!("traced unit {i}"),
        },
        Ty::CodeBits { name, width } => format!("codebits {name} ({})", sizing(*width)),
    };
    Some(s)
}

/// The brackets a type needs around it to be the element of a list.
///
/// A count binds to the word in front of it, so only a one-word type can take
/// one bare: `u8be[4]` is four numbers, but `text[4] utf8[3]` would read as an
/// index into the encoding and `sized(n) u8be[4]` as a window holding a list
/// rather than a list of windows. A type that closes with a brace has already
/// said where it ends and needs nothing.
fn brackets(elem: &Ty) -> (&'static str, &'static str) {
    match inline(elem) {
        Some(s) if !s.ends_with('}') && (s.contains(' ') || s.ends_with(']')) => ("(", ")"),
        _ => ("", ""),
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
        Encoding::Ebcdic => "ebcdic".to_string(),
    }
}

/// A run of bytes: as text when it is text, and as hex otherwise. Both forms
/// say the same thing about the same bytes.
///
/// A signature is often a word with a byte or two of punctuation in it, and
/// `"SQLite format 3\0"` is the thing a reader is looking for while
/// `53 51 4C 69 ...` is sixteen numbers to decode. So the escapes a string
/// carries count as text, as long as most of the run is still letters; a
/// signature with a byte outside that, as PNG's is, stays hex rather than
/// reading as a line of escapes.
fn bytes_lit(b: &[u8]) -> String {
    let printable = |c: &u8| (0x20..0x7f).contains(c);
    let escaped = |c: &u8| matches!(c, 0x00 | 0x09 | 0x0a | 0x0d);
    let text = !b.is_empty()
        && b.iter().all(|c| printable(c) || escaped(c))
        && b.iter().filter(|c| printable(c)).count() * 2 >= b.len();
    if text {
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
/// Public to the crate because the diagram prints the same constants: a switch
/// case is `'IHDR'` in the IR text and must be `'IHDR'` in the box beside it,
/// or the two readings of one template disagree in the one place a reader
/// checks them against each other.
pub(crate) fn tag_lit(v: i128) -> String {
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
/// renumbering: `c ? a : b` goes at 5, the booleans at 20 and 30, the
/// comparisons beside `<` at 40, and `%` beside `*` at 80.
fn prec(e: &Expr) -> u32 {
    match e {
        Expr::Cond { .. } => 5,
        Expr::Or(..) => 10,
        Expr::Either(..) => 20,
        Expr::Both(..) => 25,
        Expr::Not(..) => 30,
        Expr::Less(..) | Expr::Eq(..) | Expr::Ne(..) | Expr::Le(..) | Expr::Gt(..) | Expr::Ge(..) => 40,
        // The three bitwise operators in C's order among themselves, `|`
        // loosest and `&` tightest, all of them tighter than a comparison so
        // that a mask written beside one needs no brackets.
        Expr::BitOr(..) => 44,
        Expr::BitXor(..) => 47,
        Expr::And(..) => 50,
        Expr::Shl(..) | Expr::Shr(..) => 60,
        Expr::Add(..) | Expr::Sub(..) => 70,
        Expr::Mul(..) | Expr::Div(..) | Expr::Mod(..) => 80,
        _ => CALL,
    }
}

/// What a leaf and a call bind at, which is tighter than every operator: they
/// carry their own brackets or need none. Written at this, everything else is
/// bracketed, which is what `not` and `start of` write their operand at:
/// `not a == b` has two readings and only one of them is what it means.
const CALL: u32 = 100;

/// An expression as text.
pub fn expr(e: &Expr) -> String {
    spelled(e, true).expect("every leaf has a spelling when the probes are written")
}

/// The expression, when every leaf in it is something the reader can go and
/// look at. None when one of them reads bytes rather than naming a field: a
/// peek, a search, a shape the code deduced. One such leaf drops the whole of
/// it, since half a formula invites the reader to believe the half they can
/// see is all of it.
///
/// What the relations panel and the diagram write, so that one expression
/// reads one way wherever it is shown. See [`crate::eval::write_expr`].
pub fn readable(e: &Expr) -> Option<String> {
    spelled(e, false)
}

/// The same expression with the leaves spelled by `leaf` instead of by the
/// template: what the relations panel writes to put each field's value in its
/// place. Everything between the leaves is written here: the operators, the
/// brackets, the words for `min` and `ceil`. So the two forms of one
/// relationship differ only where they are meant to. See
/// [`crate::eval::relate`].
///
/// None when `leaf` gives up on a leaf, which drops the expression whole.
pub fn with_leaves(e: &Expr, leaf: &mut dyn FnMut(&Expr) -> Option<String>) -> Option<String> {
    write_expr(e, 0, false, leaf)
}

/// The expression with every leaf as the template writes it. `probes` says
/// whether the leaves that read bytes are among them.
fn spelled(e: &Expr, probes: bool) -> Option<String> {
    write_expr(e, 0, false, &mut |l| leaf_text(l, probes))
}

/// `mask` says the literal here is a mask rather than a count, and is written
/// in hex: `flags >> 3 & 0x3f` reads as a run of bits and `flags >> 3 & 63`
/// does not.
///
/// Adding a variant is one arm here and one in `prec`: an operator among the
/// arms below, and anything else in the one arm that hands the leaves to
/// `leaf`, which then has to spell it in `leaf_text`.
fn write_expr(e: &Expr, outer: u32, mask: bool, leaf: &mut dyn FnMut(&Expr) -> Option<String>) -> Option<String> {
    let here = prec(e);
    let wrap = |s: String| if here < outer { format!("({s})") } else { s };
    // The right side of a non-associating operator binds one step tighter, so
    // `a - (b + c)` keeps its brackets and `a - b - c` does not grow any.
    let two = |a: &Expr, b: &Expr, op: &str, leaf: &mut dyn FnMut(&Expr) -> Option<String>| {
        Some(wrap(format!("{} {op} {}", write_expr(a, here, false, leaf)?, write_expr(b, here + 1, false, leaf)?)))
    };
    Some(match e {
        // `Expr::Or` is written `or else` so that `or` is free for the boolean
        // `Either`.
        Expr::Lit(v) => {
            if mask && *v > 9 {
                hex_lit(*v)
            } else {
                int_lit(*v)
            }
        }
        Expr::Or(a, b) => two(a, b, "or else", leaf)?,
        Expr::Add(a, b) => two(a, b, "+", leaf)?,
        Expr::Sub(a, b) => two(a, b, "-", leaf)?,
        Expr::Mul(a, b) => two(a, b, "*", leaf)?,
        Expr::Div(a, b) => two(a, b, "/", leaf)?,
        Expr::Less(a, b) => two(a, b, "<", leaf)?,
        Expr::Shl(a, b) => two(a, b, "<<", leaf)?,
        Expr::Shr(a, b) => two(a, b, ">>", leaf)?,
        Expr::And(a, b) => {
            wrap(format!("{} & {}", write_expr(a, here, true, leaf)?, write_expr(b, here + 1, true, leaf)?))
        }
        // Masks like `&` is: the numbers in these are runs of bits, and a
        // reader who has to convert 0x3f back out of 63 to see that is being
        // shown the wrong thing.
        Expr::BitOr(a, b) => {
            wrap(format!("{} | {}", write_expr(a, here, true, leaf)?, write_expr(b, here + 1, true, leaf)?))
        }
        Expr::BitXor(a, b) => {
            wrap(format!("{} ^ {}", write_expr(a, here, true, leaf)?, write_expr(b, here + 1, true, leaf)?))
        }
        // Bracketed unless what it flips is a leaf or a call, as `not` is.
        Expr::BitNot(a) => wrap(format!("~{}", write_expr(a, CALL, true, leaf)?)),
        Expr::Min(a, b) => format!("min({}, {})", write_expr(a, 0, false, leaf)?, write_expr(b, 0, false, leaf)?),
        Expr::Max(a, b) => format!("max({}, {})", write_expr(a, 0, false, leaf)?, write_expr(b, 0, false, leaf)?),
        Expr::PadTo { n, align } => format!("padding({}, {align})", write_expr(n, 0, false, leaf)?),
        Expr::DivCeil(a, b) => {
            format!("ceil({} / {})", write_expr(a, 80, false, leaf)?, write_expr(b, 81, false, leaf)?)
        }
        Expr::Log2(a) => format!("log2({})", write_expr(a, 0, false, leaf)?),
        // A real is a literal like any other, written here rather than handed
        // to `leaf`: it reads nothing, and a leaf is what the relations panel
        // puts a value in place of.
        Expr::Real(v) => real_lit(*v),
        Expr::Pow2(a) => format!("pow2({})", write_expr(a, 0, false, leaf)?),
        Expr::Pow10(a) => format!("pow10({})", write_expr(a, 0, false, leaf)?),
        Expr::Trunc(a) => format!("trunc({})", write_expr(a, 0, false, leaf)?),
        Expr::Bit(a, i) => format!("bit({}, {i})", write_expr(a, 0, false, leaf)?),
        Expr::Mod(a, b) => two(a, b, "%", leaf)?,
        Expr::Eq(a, b) => two(a, b, "==", leaf)?,
        Expr::Ne(a, b) => two(a, b, "!=", leaf)?,
        Expr::Le(a, b) => two(a, b, "<=", leaf)?,
        Expr::Gt(a, b) => two(a, b, ">", leaf)?,
        Expr::Ge(a, b) => two(a, b, ">=", leaf)?,
        Expr::Either(a, b) => two(a, b, "or", leaf)?,
        Expr::Both(a, b) => two(a, b, "and", leaf)?,
        // Bracketed unless what it negates is a leaf or a call: `not a == b`
        // reads two ways and only one of them is what this means.
        Expr::Not(a) => wrap(format!("not {}", write_expr(a, CALL, false, leaf)?)),
        // Every part bracketed when it is itself a ternary, on the right as
        // well as the left: `a ? b : (c ? d : e)` leaves nothing to work out
        // about which colon belongs to which question.
        Expr::Cond { when, then, otherwise } => wrap(format!(
            "{} ? {} : {}",
            write_expr(when, here + 1, false, leaf)?,
            write_expr(then, here + 1, false, leaf)?,
            write_expr(otherwise, here + 1, false, leaf)?
        )),
        // Where a field is, so what it names keeps its brackets: `start of
        // (a + b)` is one place and `start of a + b` reads as two things
        // added.
        Expr::StartOf(a) => format!("start of {}", write_expr(a, CALL, false, leaf)?),
        // The leaves: everything that names a field or reads the file rather
        // than combining two other expressions.
        Expr::Ref(..)
        | Expr::Remaining
        | Expr::SizeOf(..)
        | Expr::BitsOf(..)
        | Expr::Idx
        | Expr::This
        | Expr::Pos
        | Expr::WindowSize
        | Expr::SpacePos
        | Expr::SpaceSize
        | Expr::PeekIn { .. }
        | Expr::LenOf(..)
        | Expr::Elem { .. }
        | Expr::ElemWithin { .. }
        | Expr::Tagged(..)
        | Expr::EntryOf { .. }
        | Expr::Placer(..)
        | Expr::Product { .. }
        | Expr::ProductOf(..)
        | Expr::SumOf(..)
        | Expr::MaxOf(..)
        | Expr::PopCount(..)
        | Expr::Deduced(..)
        | Expr::Peek { .. }
        | Expr::PeekAt { .. }
        | Expr::ToMarker { .. }
        | Expr::Find { .. }
        | Expr::Run { .. }
        | Expr::StreamLen(..)
        | Expr::Prev(..)
        | Expr::Sibling(..)
        | Expr::Within(..)
        // One leaf, the way `descriptor.(...)` is, rather than a call around
        // one: what a panel puts in its place is the number the text spelled,
        // not the text, and certainly not the search that found the card.
        | Expr::RealText(..) => leaf(e)?,
    })
}

/// A real as the template writes it: with a point, always, so that `1.0` is
/// never read as the whole number 1, and the shortest digits that read back as
/// the same double. An exponent is written where Rust writes one, `1e-7`,
/// which says it is a real as plainly as a point does.
pub(crate) fn real_lit(v: f64) -> String {
    let s = format!("{v:?}");
    if s.contains(['.', 'e', 'E']) || !v.is_finite() { s } else { format!("{s}.0") }
}

/// One leaf as the template writes it. `probes` says whether to write the ones
/// that read bytes rather than name a field; without them the expression they
/// are in has no reading at all, and nothing is written.
///
/// None for an operator, which never reaches here: `write_expr` writes those
/// itself.
fn leaf_text(e: &Expr, probes: bool) -> Option<String> {
    let path = |array: &str, index: &Expr, field: &[String]| -> Option<String> {
        let mut s = format!("{array}[{}]", spelled(index, probes)?);
        for f in field {
            s.push('.');
            s.push_str(f);
        }
        Some(s)
    };
    Some(match e {
        Expr::Ref(n) => n.to_string(),
        Expr::Remaining => "remaining".to_string(),
        Expr::SizeOf(n) => format!("sizeof({n})"),
        Expr::BitsOf(n) => format!("bitsof({n})"),
        Expr::Idx => "index".to_string(),
        // What the field the constraint sits on holds. Only ever inside a
        // `valid`, where the field has no name to write: it is the row the
        // clause is on.
        Expr::This => "this".to_string(),
        Expr::Pos => "pos".to_string(),
        Expr::WindowSize => "size of window".to_string(),
        // The same two measured from the front of the file, or of what a
        // compressed run unpacked to, rather than from the nearest window.
        Expr::SpacePos => "pos in space".to_string(),
        Expr::SpaceSize => "size of space".to_string(),
        // Not `sizeof(x)`, which is the same list measured in bytes. A reader
        // seeing both beside each other has to be able to tell them apart.
        Expr::LenOf(n) => format!("count of {n}"),
        Expr::Elem { array, index, field } => path(array, index, field)?,
        Expr::ElemWithin { path: into, index, field } => path(&into.join("."), index, field)?,
        // The list, the question asked of each element, and what is read from
        // the one that answers. A search over the elements before this one has
        // no field to name, so it is named for what it searches: `earlier`.
        Expr::Tagged(t) => {
            let array = match &t.array {
                Some(a) => spelled(a, probes)?,
                None => "earlier".to_string(),
            };
            let field = if t.field.is_empty() { String::new() } else { format!(".{}", t.field.join(".")) };
            format!("{array}[{} = {}]{field}", t.key.join("."), tag_text(&t.tag, probes)?)
        }
        // The archive entry a name points at, written the way a search over a
        // list is, with `data of` in front to say it is where the entry's
        // bytes begin rather than anything the entry holds.
        Expr::EntryOf { records, name } => format!("data of {}[name = {}]", records.join("."), spelled(name, probes)?),
        // A question for another record, so it says whose: the names inside
        // are that record's fields, and written bare they would read as fields
        // beside this one. A name or a path reads as a path into the
        // descriptor, `descriptor.count`; anything longer is bracketed whole,
        // since qualifying only its first name would claim the rest were
        // fields beside this one.
        Expr::Placer(e) => match &**e {
            Expr::Ref(n) => format!("descriptor.{n}"),
            Expr::Within(f) => format!("descriptor.{}", f.join(".")),
            other => format!("descriptor.({})", spelled(other, probes)?),
        },
        Expr::Product { array, index, field } => format!("product({})", path(array, index, field)?),
        Expr::ProductOf(n) => format!("product({n})"),
        Expr::SumOf(n) => format!("sum({n})"),
        Expr::MaxOf(n) => format!("largest({n})"),
        Expr::PopCount(n) => format!("setbits({n})"),
        Expr::Prev(n) => format!("previous({n})"),
        Expr::Sibling(f) => format!("earlier({})", f.join(".")),
        Expr::Within(f) => f.join("."),
        Expr::RealText(e) => format!("real({})", spelled(e, probes)?),
        // What the code worked out, and what it read to work it out. Nothing
        // to point at: a reader cannot go and look at where these came from,
        // so an expression holding one is written only where the whole
        // template is being written out.
        Expr::Deduced(d) if probes => format!(
            "deduced({})",
            match d {
                Deduce::PayloadShape => "payload shape",
                Deduce::PayloadCount => "payload count",
                Deduce::Builds => "builds",
            }
        ),
        Expr::Peek { bits, endian } if probes => format!("peek(u{bits}{})", end(*endian)),
        Expr::PeekAt { skip, bits, endian } if probes => {
            format!("peek(u{bits}{} at {} bits)", end(*endian), spelled(skip, probes)?)
        }
        // `in space` is the whole difference from the one above: that one
        // counts on from where the field is, and this one counts from the
        // front of the file.
        Expr::PeekIn { at, bits, endian } if probes => {
            format!("peek(u{bits}{} at {} bits in space)", end(*endian), spelled(at, probes)?)
        }
        Expr::ToMarker { lead, unless } if probes => {
            if unless.is_empty() {
                format!("tomarker({})", bytes_hex(lead))
            } else {
                format!("tomarker({} unless {})", bytes_hex(lead), bytes_hex(unless))
            }
        }
        Expr::Find { needle, last } if probes => {
            let word = if *last { "findlast" } else { "find" };
            format!("{word}({})", bytes_lit(needle))
        }
        Expr::Run { chars, negate } if probes => {
            format!("run({}{})", if *negate { "not " } else { "" }, bytes_hex(chars))
        }
        Expr::StreamLen(codec) if probes => format!("streamlen({})", codec.as_str()),
        _ => return None,
    })
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

    /// A real keeps its point, and the four that make and take apart a real
    /// are calls, so a scaled value reads as the formula a specification
    /// writes.
    #[test]
    fn real_expressions_read_as_written() {
        assert_eq!(expr(&E::real(1.0)), "1.0");
        assert_eq!(expr(&E::real(0.5)), "0.5");
        assert_eq!(expr(&E::real(-2.5)), "-2.5");
        assert_eq!(expr(&E::real(1e-7)), "1e-7");
        let worth = E::field("stored").mul(E::within(&["header", "scl_slope"])).add(E::within(&["header", "scl_inter"]));
        assert_eq!(expr(&worth), "stored * header.scl_slope + header.scl_inter");
        let grib = E::field("reference_value")
            .add(E::field("stored").mul(E::pow2(E::field("binary_scale_factor"))))
            .div(E::pow10(E::field("decimal_scale_factor")));
        assert_eq!(expr(&grib), "(reference_value + stored * pow2(binary_scale_factor)) / pow10(decimal_scale_factor)");
        let scale = E::real_text(E::within(&["card", "text"])).or(E::real(1.0));
        assert_eq!(expr(&scale), "real(card.text) or else 1.0");
        assert_eq!(expr(&E::trunc(E::field("vox_offset")).at_least(E::lit(0))), "max(trunc(vox_offset), 0)");
        assert_eq!(inline(&Ty::computed_real(E::field("a").mul(E::real(0.5)))).as_deref(), Some("computed real a * 0.5"));
        assert_eq!(Ty::computed_real(E::real(0.5)).display_name(), "computed real");
        // The panel and the IR text are one writer here too.
        for e in [worth, grib, scale] {
            assert_eq!(readable(&e), Some(expr(&e)));
        }
    }

    /// The archive entry a name points at: the records it is looked for
    /// among, and the expression that says which one. Written the way a search
    /// over a list is, with `data of` in front, so a reader can tell it from
    /// anything the entry holds.
    #[test]
    fn an_archive_entry_reads_as_the_records_and_the_name() {
        let held = E::entry_of(&["records"], E::field("key"));
        assert_eq!(expr(&held), "data of records[name = key]");
        // The name may be a path into an earlier field, and the whole is a
        // leaf, so arithmetic over it takes the brackets it needs.
        let deep = E::entry_of(&["archive", "records"], E::within(&["storage", "key"]));
        assert_eq!(expr(&deep), "data of archive.records[name = storage.key]");
        assert_eq!(expr(&deep.clone().add(E::lit(8))), "data of archive.records[name = storage.key] + 8");
        assert_eq!(expr(&E::lit(2).mul(deep.clone())), "2 * data of archive.records[name = storage.key]");
        // The panel and the IR text are one writer here as well.
        assert_eq!(readable(&deep), Some(expr(&deep)));
        // And it reads as a place to be at, which is what it is for.
        let run = Ty::at_space(deep, Ty::u8());
        assert_eq!(inline(&run).as_deref(), Some("at(data of archive.records[name = storage.key] from stream) u8be"));
    }

    #[test]
    fn a_four_letter_tag_reads_as_its_letters() {
        assert_eq!(int_lit(0x4948_4452), "'IHDR'");
        // Not a number that merely happens to be printable.
        assert_eq!(int_lit(0x3132), "12594");
        assert_eq!(int_lit(4), "4");
    }

    #[test]
    fn an_element_of_more_than_one_word_keeps_its_brackets() {
        let elem = Ty::Nullable { inner: Box::new(Ty::u32(Endian::Little)), unset: Unset::Int(0) };
        let list = Ty::Array { elem: Box::new(elem), count: E::lit(4) };
        assert_eq!(inline(&list).as_deref(), Some("(u32le unset 0)[4]"));
        // One word takes the count bare.
        let plain = Ty::Array { elem: Box::new(Ty::u32(Endian::Big)), count: E::field("n") };
        assert_eq!(inline(&plain).as_deref(), Some("u32be[n]"));
    }

    #[test]
    fn a_template_renders_the_same_way_twice() {
        let t = Template::new("demo", Ty::u32(Endian::Big));
        assert_eq!(render(&t), render(&t));
        assert_eq!(render(&t), "template demo\n\nroot u32be\n");
    }

    /// The builder, the walk to the descriptions and the key, in that order.
    /// A key the template fixed is quoted, so it cannot read as a field of that
    /// name, and a walk into a stream is bracketed for the same reason.
    #[test]
    fn a_schema_reads_as_written() {
        let table = vec![
            Step::field("streamer_info"),
            Step::field("body"),
            Step::each(),
            Step::stream(),
            Step::field("members"),
            Step::field("elements"),
            Step::each(),
        ];
        let read = Ty::schema(
            "streamers",
            table.clone(),
            vec![KeyPart::Text(E::within(&["fClassName", "text"])), KeyPart::Int(E::field("version").and(E::lit(0x3fff)))],
        );
        assert_eq!(
            inline(&read).as_deref(),
            Some("schema(streamers from streamer_info.body[].(stream).members.elements[], key fClassName.text, version & 0x3fff)")
        );
        let fixed = Ty::schema("streamers", table, vec![KeyPart::TextLit("TList".into())]);
        assert!(inline(&fixed).expect("one line").ends_with("key \"TList\")"));
        // The declared type says only that the file will say.
        assert_eq!(read.display_name(), "schema");
    }
}
