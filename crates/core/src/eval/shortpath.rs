//! A path named the way the format's own specification names it.
//!
//! A label for a field another field was read from is a path, and most paths
//! are already in the format's terms: `tensors[3].offset`. A format whose
//! metadata is written in a self-describing encoding is the exception. Thrift's
//! compact protocol writes a struct as a list of (id, value) entries, so the
//! field Parquet's specification calls `meta_data.data_page_offset` is stored
//! at `fields[1].value.fields[7].value`, and the label a walk writes by naming
//! every step says so: true, and four steps of it are the encoding talking.
//!
//! So a label is written twice where that differs. The template marks the
//! structures that are only an encoding's (see
//! [`crate::template::EncodingStep`]), and this names a path through them
//! without them: the step into a wrapper's field is left out, a member of a
//! tagged list is named by its tag, and an index sits on whatever holds the
//! list. The label as stored is kept beside it, for the reader who wants to
//! see how the bytes hold it. See `Origin::stored`.

use super::*;
use crate::template::EncodingStep;

impl Evaluator {
    /// The field at `path` named the way the format names it, counted from
    /// `at`, the node the label was worked out from. None when no step of the
    /// way is only an encoding's, which is nearly every label: the label the
    /// caller already has stands.
    ///
    /// Every label worth shortening names a field found from `at`: a field
    /// before it in some structure around it, and a path down from there. So
    /// the steps to name start where `at` and `path` part. An index with
    /// nothing in front of it is an index into the node they part at, which
    /// is then named too: `columns[0].meta_data`, not `[0].meta_data`.
    pub(super) fn short_label<S: Source>(&mut self, doc: &Document<S>, at: &[usize], path: &[usize]) -> R<Option<String>> {
        let common = at.iter().zip(path).take_while(|(a, b)| a == b).count();
        let (mut text, used) = self.short_steps(doc, common, path)?;
        if !used {
            return Ok(None);
        }
        let mut from = common;
        while from > 0 && (text.is_empty() || text.starts_with('[')) {
            let (head, _) = self.short_steps(doc, from - 1, &path[..from])?;
            text = joined(&head, &text);
            from -= 1;
        }
        Ok(Some(text))
    }

    /// The steps of `path` from depth `from` down, named, and whether any step
    /// was left out or named by a tag on the way.
    fn short_steps<S: Source>(&mut self, doc: &Document<S>, from: usize, path: &[usize]) -> R<(String, bool)> {
        // Every node on the way down is opened first, since the naming reads
        // them all and a walk may have given some of them back since. See
        // `walk_label`.
        for k in 0..=path.len() {
            self.resolve(doc, &path[..k])?;
        }
        let mut out = String::new();
        let mut used = false;
        for k in from..path.len() {
            let (parent, j) = (&path[..k], path[k]);
            let Some(ty) = self.memo.get(parent).map(|r| r.ty.base().clone()) else { break };
            match &ty {
                Ty::Struct(s) => {
                    let Some(field) = s.fields.get(j) else { break };
                    let through = s.encoding.as_ref().is_some_and(|e| *field.name == *e.through());
                    // The list of a record's members is left out when the member
                    // after it has a name to stand in for its index. One that
                    // has none keeps the list's name, since `meta_data[17]`
                    // would read as an element of a list called `meta_data`.
                    if through && !self.unnamed_member(doc, path, k + 1)? {
                        used = true;
                        continue;
                    }
                    out = joined(&out, &field.name);
                }
                Ty::Array { .. } | Ty::Repeat { .. } | Ty::PointerList { .. } | Ty::Chain { .. } | Ty::Gather { .. } | Ty::Raster { .. } => {
                    match self.member_name(doc, &path[..=k])? {
                        Some(name) => {
                            out = joined(&out, &name);
                            used = true;
                        }
                        None => out.push_str(&format!("[{j}]")),
                    }
                }
                // What a placement or an origin holds adds no name, and nor
                // does what a stream holds: the step before named the field,
                // and its contents are it. The same rule a walk's own label
                // follows.
                Ty::At { .. } | Ty::Origin { .. } => {}
                Ty::Decoded { .. } | Ty::Stitched { .. } if j == 0 => {}
                _ => {
                    let Some(name) = self.memo.get(&path[..=k]).map(|r| r.name.text()) else { break };
                    out = joined(&out, &name);
                }
            }
        }
        Ok((out, used))
    }

    /// Whether the step at depth `k` of `path` is into a member of a tagged
    /// list whose tag reads as no name: a Thrift field id the schema does not
    /// know.
    fn unnamed_member<S: Source>(&mut self, doc: &Document<S>, path: &[usize], k: usize) -> R<bool> {
        if k >= path.len() {
            return Ok(false);
        }
        self.resolve(doc, &path[..=k])?;
        let list = matches!(
            self.memo.get(&path[..k]).map(|r| r.ty.base()),
            Some(Ty::Array { .. } | Ty::Repeat { .. } | Ty::PointerList { .. } | Ty::Chain { .. } | Ty::Gather { .. })
        );
        if !list || self.member_tag(&path[..=k]).is_none() {
            return Ok(false);
        }
        Ok(self.member_name(doc, &path[..=k])?.is_none())
    }

    /// Which field of the node at `path` tags it as a member, when it is one.
    fn member_tag(&self, path: &[usize]) -> Option<usize> {
        let Ty::Struct(s) = self.memo.get(path)?.ty.base() else { return None };
        let Some(EncodingStep::Member { tag, .. }) = &s.encoding else { return None };
        s.fields.iter().position(|f| *f.name == **tag)
    }

    /// The name a member of a tagged list goes by, which is what its tag reads
    /// as when that is a name: a field id the schema names, or a key written
    /// as text. None for a node that is not a member, and for a tag that is a
    /// bare number or cannot be read yet.
    fn member_name<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> R<Option<String>> {
        self.resolve(doc, path)?;
        let Some(tag) = self.member_tag(path) else { return Ok(None) };
        Ok(match self.naming_value(doc, path, tag) {
            Some(Value::Enum { name: Some(name), .. }) => Some(name),
            Some(Value::Str(key)) if !key.is_empty() => Some(key_step(&key)),
            _ => None,
        })
    }
}

/// A key as one step of a path: as it is where it reads as a name, and
/// quoted in brackets where it does not, so that `piece length` cannot read
/// as two steps and `a.b` as two fields.
fn key_step(key: &str) -> String {
    let plain = key.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_')
        && key.chars().all(|c| c.is_alphanumeric() || c == '_');
    if plain { key.to_string() } else { format!("[{key:?}]") }
}

/// Two parts of a path, with a dot between them where the second is a name.
fn joined(head: &str, tail: &str) -> String {
    if head.is_empty() {
        tail.to_string()
    } else if tail.is_empty() || tail.starts_with('[') {
        format!("{head}{tail}")
    } else {
        format!("{head}.{tail}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::MemSource;

    #[test]
    fn a_key_that_is_not_a_name_is_quoted() {
        assert_eq!(key_step("announce"), "announce");
        assert_eq!(key_step("piece length"), "[\"piece length\"]");
        assert_eq!(key_step("a.b"), "[\"a.b\"]");
    }

    /// A bencode dictionary holding a dictionary of two numbers, one of them
    /// under a key with a space in it.
    fn torrent() -> (Document<MemSource>, Evaluator) {
        (Document::new(MemSource(b"d4:infod6:lengthi7e12:piece lengthi9eee".to_vec())), Evaluator::new(crate::formats::bencode()))
    }

    #[test]
    fn a_bencode_path_is_named_by_its_keys() {
        let (d, mut ev) = torrent();
        // The root value's body, its first entry, that entry's value, the
        // value's body, the first entry there, its value, and that value's
        // body, which is the number.
        let length = [1, 0, 1, 1, 0, 1, 1];
        assert_eq!(ev.node(&d, &length).unwrap().value, Value::Int(7));
        assert_eq!(ev.short_label(&d, &[], &length).unwrap().as_deref(), Some("info.length"));
        let spaced = [1, 0, 1, 1, 1, 1, 1];
        assert_eq!(ev.node(&d, &spaced).unwrap().value, Value::Int(9));
        assert_eq!(ev.short_label(&d, &[], &spaced).unwrap().as_deref(), Some("info[\"piece length\"]"));
    }

    #[test]
    fn a_cbor_path_is_named_by_its_text_keys_and_indexed_by_the_others() {
        // {"a": {"b": 2}, 7: 3}
        let bytes = vec![0xa2, 0x61, b'a', 0xa1, 0x61, b'b', 0x02, 0x07, 0x03];
        let d = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(crate::formats::cbor());
        // The root's pairs, the first, its value, that value's pairs, the
        // first, its value.
        let b = [3, 0, 1, 3, 0, 1];
        assert_eq!(ev.node(&d, &b).unwrap().size_bits, 8);
        assert_eq!(ev.short_label(&d, &[], &b).unwrap().as_deref(), Some("a.b"));
        // A key that is a number is no name: the pairs keep their list.
        assert_eq!(ev.short_label(&d, &[], &[3, 1, 1]).unwrap().as_deref(), Some("value[1]"));
    }

    /// A Thrift struct with a field the schema names, one it does not, and a
    /// struct inside it.
    fn thrift() -> (Document<MemSource>, Evaluator) {
        use crate::formats::thrift::{types, Field, Struct, What};
        const ROOT: Struct = Struct {
            name: "Root",
            fields: &[
                Field { id: 1, name: "count", what: What::Plain },
                Field { id: 22, name: "inner", what: What::Struct("Inner") },
            ],
        };
        const INNER: Struct = Struct { name: "Inner", fields: &[Field { id: 1, name: "flag", what: What::Plain }] };
        let mut t = crate::template::Template::new("t", Ty::structure("Test", vec![("meta", Ty::Named("t.Root".into()))]));
        for (name, ty) in types("t", &[ROOT, INNER]) {
            t = t.with_type(&name, ty);
        }
        let bytes = vec![
            0x15, 0x0e, // field 1, i32 7
            0x45, 0x02, // field 5, which the schema does not name, i32 1
            0x0c, 0x2c, // field 22, written out because the gap is over fifteen: a struct
            0x11, 0x00, // its field 1 is true, then its stop
            0x00, // stop
        ];
        (Document::new(MemSource(bytes)), Evaluator::new(t))
    }

    #[test]
    fn a_thrift_path_is_named_by_the_schema() {
        let (d, mut ev) = thrift();
        // meta, its list of fields, the first, and its value.
        let count = [0, 0, 0, 3];
        assert_eq!(ev.node(&d, &count).unwrap().value.as_int(), Some(7));
        assert_eq!(ev.short_label(&d, &[], &count).unwrap().as_deref(), Some("meta.count"));
        let flag = [0, 0, 2, 3, 0, 0, 3];
        assert_eq!(ev.short_label(&d, &[], &flag).unwrap().as_deref(), Some("meta.inner.flag"));
        // Counted from a field inside the struct, the name starts there.
        assert_eq!(ev.short_label(&d, &[0, 0, 2, 3, 0, 1], &flag).unwrap().as_deref(), Some("flag"));
    }

    #[test]
    fn a_field_the_schema_does_not_name_keeps_its_stored_steps() {
        let (d, mut ev) = thrift();
        let unknown = [0, 0, 1, 3];
        assert_eq!(ev.node(&d, &unknown).unwrap().value.as_int(), Some(1));
        // Not `meta[1]`, which would read as an element of a list called
        // `meta`.
        assert_eq!(ev.short_label(&d, &[], &unknown).unwrap().as_deref(), Some("meta.fields[1]"));
    }

    #[test]
    fn a_path_with_no_encoding_step_has_no_short_form() {
        let t = crate::template::Template::new(
            "t",
            Ty::structure("R", vec![("n", Ty::u8()), ("xs", Ty::array(Ty::structure("X", vec![("v", Ty::u8())]), Expr::field("n")))]),
        );
        let d = Document::new(MemSource(vec![2, 5, 6]));
        let mut ev = Evaluator::new(t);
        ev.node(&d, &[1, 1, 0]).unwrap();
        assert_eq!(ev.short_label(&d, &[], &[1, 1, 0]).unwrap(), None);
    }
}
