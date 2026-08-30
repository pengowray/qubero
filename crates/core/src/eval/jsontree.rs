//! JSON inside a file, as nodes of the tree like any other field.
//!
//! A `Ty::Json` field is read once, when something first asks what is inside
//! it, and the parsed values are kept beside the memo. Every value in it then
//! answers like a field the template declared: it has a place in the file, a
//! size, a name, and children. That is what lets the rest of the evaluator
//! reach into a JSON header without knowing it is one, so a safetensors
//! template can say "the offsets are in `header[i].data_offsets`" the same way
//! a GGUF template says they are in `tensors[i].offset`.

use std::sync::Arc;

use super::*;
use crate::json::{self, Kind, Shape, Val};

impl Evaluator {
    /// The path of the JSON field `path` sits in, and its parsed text.
    /// `path` may be the field itself or any value inside it.
    pub(super) fn json_doc<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> R<(Vec<usize>, Arc<Val>)> {
        let root = (0..=path.len())
            .rev()
            .find(|k| matches!(self.memo.get(&path[..*k]).map(|r| &r.ty), Some(Ty::Json(Shape::Doc))));
        let Some(k) = root else { return fail("not inside a JSON field") };
        let root = path[..k].to_vec();
        if let Some(v) = self.memo.json(&root) {
            return Ok((root, v.clone()));
        }
        let r = self.memo[&root].clone();
        let size = r.declared_size.unwrap_or(r.limit - r.offset);
        let text = self.read(doc, &r, r.offset, size)?;
        let val = match json::parse(&text) {
            Ok(v) => Arc::new(v),
            Err(e) => return fail(format!("this isn't JSON: {e}")),
        };
        self.memo.remember_json(root.clone(), val.clone());
        Ok((root, val))
    }

    /// The parsed value at `path`, which must be a JSON node.
    fn json_val<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> R<(u64, Arc<Val>)> {
        let (root, tree) = self.json_doc(doc, path)?;
        let base = self.memo[&root].offset;
        let mut val = tree;
        for &i in &path[root.len()..] {
            let Some(child) = val.child(i) else { return fail("no such value") };
            val = Arc::new(child.clone());
        }
        Ok((base, val))
    }

    /// Place child `idx` of the JSON node at `parent`. Where it sits is where
    /// its text sits, which the parse already worked out, so nothing is read
    /// and nothing is walked.
    pub(super) fn resolve_json_child<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> R<()> {
        let (parent, idx) = (&path[..path.len() - 1], path[path.len() - 1]);
        let (base, val) = self.json_val(doc, parent)?;
        let (Some(child), Some(name)) = (val.child(idx), val.child_name(idx)) else {
            return fail("no such value");
        };
        let offset = base + child.start as u64 * 8;
        let end = base + child.end as u64 * 8;
        let r = Resolved {
            name: Name::Field(name.into()),
            ty: Ty::Json(child.kind.shape()),
            offset,
            limit: end,
            declared_size: None,
            size: Some(end - offset),
            computed: None,
        };
        self.remember(path, r);
        Ok(())
    }

    /// How many values are inside the JSON node at `path`.
    pub(super) fn json_child_count<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> R<u64> {
        Ok(self.json_val(doc, path)?.1.child_count() as u64)
    }

    /// What the JSON node at `path` is worth. Objects and arrays hold their
    /// values rather than being one, and are counted instead.
    pub(super) fn json_value<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> R<Value> {
        let (_, val) = self.json_val(doc, path)?;
        Ok(match &val.kind {
            Kind::Object(_) | Kind::Array(_) => Value::Composite { count: val.child_count() as u64 },
            Kind::Text(s) => Value::Str(s.clone()),
            Kind::Int(v) => Value::Int(*v),
            Kind::Float(v) => Value::Float(*v),
            // Written as words in the file, and shown as the words they are.
            Kind::Bool(b) => Value::Str(if *b { "true".into() } else { "false".into() }),
            Kind::Null => Value::Str("null".into()),
        })
    }

    /// Which child of the JSON node at `path` is called `name`: a key of an
    /// object, or an index of an array written as a number.
    pub(super) fn json_index<S: Source>(&mut self, doc: &Document<S>, path: &[usize], name: &str) -> R<Option<usize>> {
        Ok(self.json_val(doc, path)?.1.index_of(name))
    }
}
