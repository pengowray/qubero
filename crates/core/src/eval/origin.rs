//! Where a field's shape came from.
//!
//! Most fields are what the template says they are and nothing more. The
//! interesting ones are not: a GGUF string is as long as the number in front of
//! it, a metadata value is whatever type the entry named two fields back, and a
//! tensor's weights sit wherever its record said. A reader looking at 128 bytes
//! of packed nibbles needs to know which field decided that, and be able to go
//! and look at it.
//!
//! So this answers, for one field: which other fields settled its length, its
//! count, its type or its place, and where each of those is. It also answers
//! the question the other way round, for a field holding an offset: which bit
//! of the file does this number point at.

use super::listing::brief;
use super::*;

/// What one other field decided about this one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// How many bytes this field runs for.
    Length,
    /// How many bits wide the number is. Apart from `Length` because it is not
    /// bytes and because it is a different question: a run of grid values is
    /// as long as the count says, and each value in it is as wide as the
    /// packing said. See [`crate::template::Ty::UIntExpr`].
    Width,
    /// How many children it has.
    Count,
    /// Which type it is read as.
    Type,
    /// Where it starts.
    Position,
    /// What it says. A field of no bits is worked out from other fields, and
    /// those are the fields to name.
    Value,
    /// What it is called. Apart from `Value` because it decides nothing about
    /// the field's contents: a FITS column is the same bytes whatever its
    /// `TTYPE` card says, and a reader wondering where the word `flux` came
    /// from is asking a different question from one wondering where the
    /// numbers came from. See [`crate::template::Field::name_from`].
    Name,
    /// Not about this field at all: this field is an offset, and points there.
    Points,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Length => "length",
            Role::Width => "width",
            Role::Count => "count",
            Role::Type => "type",
            Role::Position => "position",
            Role::Value => "value",
            Role::Name => "name",
            Role::Points => "points",
        }
    }
}

/// One answer: a field, what it decided, and what it says.
#[derive(Debug, Clone, PartialEq)]
pub struct Origin {
    pub role: Role,
    /// The field as the reader would name it: `len`, or `tensors[3].offset`.
    pub label: String,
    /// Where that field is, so the reader can go there. Empty when the number
    /// came from somewhere with no field of its own.
    pub path: Vec<usize>,
    /// What it says, in brief. Empty when it could not be read.
    pub value: String,
    /// For `Points`: the bit this field's value points at.
    pub target_bits: Option<u64>,
}

impl Evaluator {
    /// Which fields settled the shape of the one at `path`, and where this one
    /// points if it is an offset. Empty for a field the template placed and
    /// sized outright, which is most of them.
    pub fn origins<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> R<Vec<Origin>> {
        self.resolve(doc, path)?;
        let mut out = Vec::new();
        self.placed_by(doc, path, &mut out)?;
        // Where the name on the row came from, when the file rather than the
        // template says what the field is called.
        if let Some(from) = self.name_from(path) {
            self.from_expr(doc, path, &from, Role::Name, &mut out)?;
        }
        // The declared type, before the switch picked a case and before `Sized`
        // was unwrapped: that is where the deciding expressions are.
        let declared = self.declared_ty(path)?;
        self.wrapper_origins(doc, path, declared, &mut out)?;
        let base = self.memo[path].ty.without_sentinel().clone();
        self.base_origins(doc, path, &base, &mut out)?;
        if let Some((label, bits)) = self.points_at(doc, path)? {
            out.push(Origin {
                role: Role::Points,
                label,
                path: Vec::new(),
                value: String::new(),
                target_bits: Some(bits),
            });
        }
        Ok(out)
    }

    /// Where this field points, for a field an earlier list of pointers reads
    /// its offsets from. The answer is where that list put the matching child,
    /// which is the same arithmetic the list already did.
    fn points_at<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> R<Option<(String, u64)>> {
        let Some((list, idx, name)) = self.pointer_use(doc, path) else { return Ok(None) };
        let mut child = list;
        child.push(idx);
        if self.resolve(doc, &child).is_err() {
            return Ok(None);
        }
        Ok(Some((name, self.memo[&child].offset)))
    }

    /// The pointer list that reads this field as one of its offsets: where the
    /// list is, which of its children this field places, and what that child is
    /// called.
    fn pointer_use<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> Option<(Vec<usize>, usize, String)> {
        for k in 0..path.len().saturating_sub(1) {
            let (parent, arr, idx) = (&path[..k], path[k], path[k + 1]);
            let Some(Ty::Struct(s)) = self.memo.get(parent).map(|r| &r.ty).cloned() else { continue };
            let Some(array) = s.fields.get(arr).cloned() else { continue };
            for (j, f) in s.fields.iter().enumerate() {
                let Ty::PointerList { offsets, field, .. } = &f.ty else { continue };
                if **offsets != *array.name || !self.names_field(doc, path, k + 2, &field.clone()) {
                    continue;
                }
                let mut list = parent.to_vec();
                list.push(j);
                return Some((list, idx, format!("{}[{idx}]", f.name)));
            }
        }
        None
    }

    /// Whether what is left of the path past `from` is the field a pointer list
    /// names, which is one named field of the element, or the element itself.
    fn names_field<S: Source>(&mut self, doc: &Document<S>, path: &[usize], from: usize, field: &[String]) -> bool {
        if path.len() != from + field.len() {
            return false;
        }
        let mut p = path[..from].to_vec();
        matches!(self.descend(doc, &mut p, field), Ok(true)) && p == path
    }

    /// The offset that placed a child of a pointer list.
    fn placed_by<S: Source>(&mut self, doc: &Document<S>, path: &[usize], out: &mut Vec<Origin>) -> R<()> {
        let Some((&idx, list)) = path.split_last() else { return Ok(()) };
        let Some(r) = self.memo.get(list) else { return Ok(()) };
        let Ty::PointerList { offsets, field, .. } = &r.ty else { return Ok(()) };
        let (offsets, field) = (offsets.to_string(), field.clone());
        let Some(mut p) = self.find_field(list, &offsets) else { return Ok(()) };
        p.push(idx);
        let mut label = format!("{offsets}[{idx}]");
        if !field.is_empty() {
            if !self.descend(doc, &mut p, &field)? {
                return Ok(());
            }
            label = format!("{label}.{}", field.join("."));
        }
        let o = self.origin(doc, Role::Position, label, p);
        out.push(o);
        Ok(())
    }

    /// What the type says before the file has been consulted: the field's own
    /// declaration, with nothing chosen and no window unwrapped yet.
    pub(super) fn declared_ty(&self, path: &[usize]) -> R<Ty> {
        let Some((&idx, parent)) = path.split_last() else { return Ok(self.template.root.clone()) };
        match self.memo.get(parent).map(|r| &r.ty) {
            Some(Ty::Struct(s)) => match s.fields.get(idx) {
                Some(f) => Ok(f.ty.clone()),
                None => fail("no such field"),
            },
            Some(Ty::Array { elem, .. } | Ty::Repeat { elem, .. } | Ty::PointerList { elem, .. } | Ty::Chain { elem, .. }) => {
                Ok((**elem).clone())
            }
            _ => fail("not a composite"),
        }
    }

    /// The wrappers around a type: a window with a size, and a switch that
    /// picks by an earlier value. Stops at the switch, since which case it took
    /// is already in the memo and is where `base_origins` reads from.
    fn wrapper_origins<S: Source>(
        &mut self,
        doc: &Document<S>,
        path: &[usize],
        mut ty: Ty,
        out: &mut Vec<Origin>,
    ) -> R<()> {
        for _ in 0..64 {
            match ty {
                Ty::Named(n) => match self.template.types.get(&*n) {
                    Some(t) => ty = t.clone(),
                    None => return Ok(()),
                },
                Ty::Sized { size, inner } => {
                    self.from_expr(doc, path, &size, Role::Length, out)?;
                    ty = *inner;
                }
                Ty::Switch { on, .. } | Ty::Match { on, .. } => {
                    self.from_expr(doc, path, &on, Role::Type, out)?;
                    return Ok(());
                }
                _ => return Ok(()),
            }
        }
        Ok(())
    }

    /// The type the field is actually read as: how long it runs, or how many
    /// children it has.
    fn base_origins<S: Source>(&mut self, doc: &Document<S>, path: &[usize], ty: &Ty, out: &mut Vec<Origin>) -> R<()> {
        match ty {
            Ty::Bytes(e) => self.from_expr(doc, path, &e.clone(), Role::Length, out),
            Ty::Str { len: StrLen::Fixed(e) | StrLen::Padded { size: e, .. }, .. } => {
                self.from_expr(doc, path, &e.clone(), Role::Length, out)
            }
            // Which field decided how many bits this number is.
            Ty::UIntExpr { bits, .. } => self.from_expr(doc, path, &(**bits).clone(), Role::Width, out),
            Ty::Array { count, .. } => self.from_expr(doc, path, &count.clone(), Role::Count, out),
            Ty::Computed(e) | Ty::ComputedText(e) => self.from_expr(doc, path, &e.clone(), Role::Value, out),
            _ => Ok(()),
        }
    }

    /// Every field an expression reads, in the order it reads them. An
    /// expression made only of numbers names no field and produces nothing.
    fn from_expr<S: Source>(
        &mut self,
        doc: &Document<S>,
        at: &[usize],
        e: &Expr,
        role: Role,
        out: &mut Vec<Origin>,
    ) -> R<()> {
        match e {
            Expr::Ref(name) => {
                if let Some(p) = self.find_field(at, name) {
                    let o = self.origin(doc, role, name.to_string(), p);
                    out.push(o);
                }
            }
            Expr::SizeOf(name) | Expr::BitsOf(name) => {
                if let Some(p) = self.find_field(at, name) {
                    let mut o = self.origin(doc, role, format!("size of {name}"), p);
                    // How long the field is, not what it says. The row names a
                    // size and used to answer with the value, which is a
                    // different number and reads as this one being wrong.
                    o.value = self.eval_expr(doc, at, e)?.to_string();
                    out.push(o);
                }
            }
            Expr::ProductOf(name) | Expr::SumOf(name) | Expr::MaxOf(name) => {
                if let Some(p) = self.find_field(at, name) {
                    let mut o = self.origin(doc, role, name.to_string(), p);
                    o.value = self.eval_expr(doc, at, e)?.to_string();
                    out.push(o);
                }
            }
            Expr::Elem { array, index, field } | Expr::Product { array, index, field } => {
                // A shape is an array, and its own value is a count of numbers
                // rather than a number. What it decided is the numbers
                // multiplied together, so that is what this says it says.
                let product = matches!(e, Expr::Product { .. });
                let i = self.eval_expr(doc, at, index)?;
                if i < 0 {
                    return Ok(());
                }
                let Some(mut p) = self.find_field(at, array) else { return Ok(()) };
                p.push(i as usize);
                let mut label = format!("{array}[{i}]");
                for name in field.iter() {
                    match self.child_index(doc, &p, name)? {
                        Some(j) => p.push(j),
                        None => return Ok(()),
                    }
                    label = format!("{label}.{name}");
                }
                let mut o = self.origin(doc, role, label, p);
                if product {
                    o.value = self.eval_expr(doc, at, e)?.to_string();
                }
                out.push(o);
            }
            // The element that says what it is, rather than one at a known
            // place: the answer came from wherever in the list the tag was,
            // and that is the element to point at.
            Expr::Tagged(t) => {
                // The label may be read from this record rather than fixed by
                // the format, and then the field holding it is half the
                // connection: a GWF structure's class byte is what sent the
                // lookup to the structure it found. Naming only the far end
                // would leave the reader at the answer with no way back to the
                // question.
                if let crate::template::Tag::Computed(e) | crate::template::Tag::ComputedText(e) = &t.tag {
                    self.from_expr(doc, at, &e.clone(), role, out)?;
                }
                let t = t.clone();
                let here = self.memo.get(at).map(|r| (r.offset, r.limit));
                if let Some((p, label)) = self.tagged_path(doc, at, &t, here)? {
                    let o = self.origin(doc, role, label, p);
                    out.push(o);
                }
            }
            Expr::Add(a, b)
            | Expr::Sub(a, b)
            | Expr::Mul(a, b)
            | Expr::Div(a, b)
            | Expr::Or(a, b)
            | Expr::Less(a, b)
            | Expr::Shl(a, b)
            | Expr::Shr(a, b)
            | Expr::And(a, b)
            | Expr::Min(a, b)
            | Expr::Max(a, b) => {
                self.from_expr(doc, at, a, role, out)?;
                self.from_expr(doc, at, b, role, out)?;
            }
            // Padding is decided by whatever said how long the run before it
            // was, which is the field worth pointing at.
            Expr::PadTo { n, .. } => self.from_expr(doc, at, n, role, out)?,
            _ => {}
        }
        Ok(())
    }

    /// One answer, with the named field's value if it can be read. A value that
    /// cannot be read yet is left out rather than waited for: where the number
    /// came from is worth saying without it.
    fn origin<S: Source>(&mut self, doc: &Document<S>, role: Role, label: String, path: Vec<usize>) -> Origin {
        let value = match self.node(doc, &path) {
            Ok(info) => brief(&info.value),
            Err(_) => String::new(),
        };
        Origin { role, label, path, value, target_bits: None }
    }
}
