//! The reading behind a [`Ty::Schema`]: working out its key, handing the
//! format's builder the descriptions it asks for, and keeping what it built.
//!
//! The evaluator does not know what a description means. It knows how to walk
//! to one, the way a gather walks to its records (see `gather.rs`), and how to
//! read a node. So the builder is handed a [`Descriptions`], which is those two
//! things and nothing else, and says what the node is.
//!
//! The walk is lazy, and that is not an economy but the thing that makes a
//! self-describing format readable at all. ROOT's descriptions are streamed
//! objects themselves, typed by schema nodes of the same kind. If every build
//! walked the table first, reading the first description would walk to the
//! descriptions, which is where it already is. A builder that knows the classes
//! descriptions are written in answers those keys without asking for a record,
//! and nothing is walked.
//!
//! What was built is kept per kind and key: a thousand branches of one class
//! are one build. A key that is being built is refused when something reads it
//! again before the build is done, since that is a description being read with
//! itself, and following it is not slow but endless.
//!
//! Only builds that come out are kept. A build that failed is asked again the
//! next time a node of its key is read: the same key read from inside the
//! descriptions and from outside them can see different tables, and a failure
//! where the table cannot be seen says nothing about a node that can see it.

use rustc_hash::{FxHashMap, FxHashSet};
use std::sync::Arc;

use super::gather::Landing;
use super::origin::{Origin, Role};
use super::*;
use crate::template::{Built, KeyPart, KeyValue, Step};

/// What a builder reads the file's descriptions through.
///
/// Paths are the evaluator's own, so anything a builder finds it can name in
/// what it builds: a member's description is where [`Built::members_from`]
/// points, and a reader asking why a row is typed the way it is is sent there.
pub trait Descriptions {
    /// Every record the table walks to, in the order the walk found them. The
    /// walk is taken the first time this is asked, and not before.
    fn records(&mut self) -> R<Vec<Vec<usize>>>;
    /// The node at `path`, read the way any view reads it.
    fn node(&mut self, path: &[usize]) -> R<NodeInfo>;
    /// Down from `from` a name at a time: a field of a structure, or an
    /// element of a list written as its index. Through a field that points
    /// elsewhere and into what a stream holds, the way every path goes.
    /// Nothing when a name is not there.
    fn find(&mut self, from: &[usize], names: &[&str]) -> R<Option<Vec<usize>>>;
    /// How many children the node at `path` has.
    fn count(&mut self, path: &[usize]) -> R<u64>;
    /// The whole text of the field at `path`.
    fn text(&mut self, path: &[usize]) -> R<String>;
    /// The number the field at `path` holds, if it holds one.
    fn int(&mut self, path: &[usize]) -> R<Option<i128>> {
        Ok(self.node(path)?.value.as_int())
    }
}

/// What the builds have come to, and which are under way.
#[derive(Default)]
pub(super) struct Schemas {
    built: FxHashMap<(Arc<str>, Vec<KeyValue>), Kept>,
    building: FxHashSet<(Arc<str>, Vec<KeyValue>)>,
}

/// One build, and how far into the file what it read reaches.
struct Kept {
    built: Arc<Built>,
    /// The end of the furthest description the build read, in bits of the
    /// file. An edit before this may have changed what the type is. Nought for
    /// a type built by heart, and the largest bit there is for one whose
    /// descriptions were read inside a stream, since where an edit to the
    /// stream's bytes lands in the file is not something the stream can say.
    reach: u64,
}

impl Schemas {
    /// Forget every build. For a change of document or of template.
    pub(super) fn forget(&mut self) {
        self.built.clear();
        self.building.clear();
    }

    /// Whether an overwrite at `bit` may have changed a description some type
    /// was built from.
    pub(super) fn reaches_past(&self, bit: u64) -> bool {
        self.built.values().any(|k| k.reach > bit)
    }
}

/// The reading a builder is handed: the evaluator and the document, the node
/// being built and the walk to its descriptions, and how far what has been
/// read reaches.
struct Reader<'a, S: Source> {
    ev: &'a mut Evaluator,
    doc: &'a Document<S>,
    at: &'a [usize],
    table: &'a [Step],
    reach: u64,
}

impl<S: Source> Reader<'_, S> {
    /// Note that the node at `path` has been read, so the build reaches at
    /// least to its end.
    fn note(&mut self, path: &[usize]) -> R<()> {
        let space = self.ev.memo.get(path).map_or(0, |r| r.space);
        if space != 0 {
            self.reach = u64::MAX;
            return Ok(());
        }
        let size = self.ev.size_of(self.doc, path)?;
        let end = self.ev.memo[path].offset + size;
        self.reach = self.reach.max(end);
        Ok(())
    }
}

impl<S: Source> Descriptions for Reader<'_, S> {
    fn records(&mut self) -> R<Vec<Vec<usize>>> {
        let (ev, doc, at) = (&mut *self.ev, self.doc, self.at);
        loop {
            if ev.list(at).gather.as_deref().is_some_and(|g| g.done) {
                break;
            }
            let Some(record) = ev.walk_next(doc, at, self.table, Landing::Records)? else {
                ev.list_mut(at).gather.get_or_insert_with(Default::default).done = true;
                break;
            };
            // Charged once per record reached, before anything is read from
            // it, as a gather's walk is: a go that runs out here stands on
            // this record when it is asked again.
            let reached = ev.memo.get(&record).map_or(0, |r| r.offset);
            ev.spend(reached)?;
            ev.list_mut(at).gather.get_or_insert_with(Default::default).records.push(record);
            ev.walk_past(at, Landing::Records);
        }
        let records = ev.list(at).gather.as_deref().map(|g| g.records.clone()).unwrap_or_default();
        for r in &records {
            self.note(r)?;
        }
        Ok(records)
    }

    fn node(&mut self, path: &[usize]) -> R<NodeInfo> {
        let info = self.ev.node(self.doc, path)?;
        self.note(path)?;
        Ok(info)
    }

    fn find(&mut self, from: &[usize], names: &[&str]) -> R<Option<Vec<usize>>> {
        let mut p = from.to_vec();
        for name in names {
            self.ev.into_contents(self.doc, &mut p)?;
            match self.ev.child_index(self.doc, &p, name)? {
                Some(j) => p.push(j),
                None => return Ok(None),
            }
            self.ev.through_at(self.doc, &mut p)?;
        }
        Ok(Some(p))
    }

    fn count(&mut self, path: &[usize]) -> R<u64> {
        self.note(path)?;
        self.ev.child_count(self.doc, path)
    }

    fn text(&mut self, path: &[usize]) -> R<String> {
        self.note(path)?;
        self.ev.text_of(self.doc, path)
    }
}

impl Evaluator {
    /// Forget what every build came to, for a change after which no
    /// description can be trusted to say what it said.
    pub(super) fn forget_schemas(&mut self) {
        self.schemas.forget();
    }

    /// What the builder of `kind` makes of the node at `path`, which starts at
    /// `offset` and may read to `limit`: kept from before where it was built
    /// before, and built now where it was not. `windowed` says a `Sized` round
    /// the node has settled how long it is, which is what lets a build that
    /// fails read as bytes rather than fail the node.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn schema_type<S: Source>(
        &mut self,
        doc: &Document<S>,
        path: &[usize],
        kind: &Arc<str>,
        table: &Arc<[Step]>,
        key: &[KeyPart],
        here: (u64, u64),
        windowed: bool,
    ) -> R<Arc<Built>> {
        let Some(builder) = self.template.schemas.get(&**kind).cloned() else {
            return fail(format!("this template does not build a {kind} schema"));
        };
        let key = self.key_values(doc, path, key, Some(here))?;
        let slot = (kind.clone(), key.clone());
        if let Some(kept) = self.schemas.built.get(&slot) {
            return Ok(kept.built.clone());
        }
        if self.schemas.building.contains(&slot) {
            let named = builder.key_text(&key);
            return fail(format!("{named}'s description depends on itself"));
        }
        self.schemas.building.insert(slot.clone());
        // The walk the builder may ask for is kept on the node's own list
        // state, the way a gather's is, so a go that runs out part way through
        // carries on from the record it stood on. It is dropped once the build
        // is over: the node is about to be something else.
        let (out, reach) = {
            let mut reader = Reader { ev: self, doc, at: path, table, reach: 0 };
            let out = builder.build(&key, &mut reader);
            (out, reader.reach)
        };
        self.schemas.building.remove(&slot);
        let built = match out {
            Ok(built) => Arc::new(built),
            // Waiting for bytes, or out of go: the walk is kept and the build
            // is asked again.
            Err(e) if e.interrupted() => return Err(e),
            // A node whose room is already settled reads as that room's bytes,
            // with the reason on it, rather than failing: the window was
            // measured by something outside the description, and what is
            // after it is still where it was. A ROOT object's members are
            // this, inside the byte count that says how long the object is,
            // and a class the file does not describe is one object's worth of
            // bytes and not a broken record. Not kept, since a build asked
            // from somewhere that can see more of the file may come out.
            Err(EvalError::Failed(why)) if windowed => {
                self.list_mut(path).gather = None;
                let unbuilt = Ty::structure(&builder.key_text(&key), vec![("bytes", Ty::bytes(Expr::Remaining))]).doc(&why);
                return Ok(Arc::new(Built::by_heart(unbuilt)));
            }
            Err(e) => {
                self.list_mut(path).gather = None;
                return Err(e);
            }
        };
        self.list_mut(path).gather = None;
        self.schemas.built.insert(slot, Kept { built: built.clone(), reach });
        Ok(built)
    }

    /// The key a schema node is looked up by, each part worked out in the
    /// node's own frame.
    pub(super) fn key_values<S: Source>(
        &mut self,
        doc: &Document<S>,
        path: &[usize],
        key: &[KeyPart],
        here: Option<(u64, u64)>,
    ) -> R<Vec<KeyValue>> {
        let mut out = Vec::with_capacity(key.len());
        for part in key {
            out.push(match part {
                KeyPart::Int(e) => KeyValue::Int(self.eval_expr_at(doc, path, e, here)?),
                KeyPart::Text(e) => KeyValue::Text(self.text_at(doc, path, e, here)?.into()),
                KeyPart::TextLit(s) => KeyValue::Text(s.clone()),
            });
        }
        Ok(out)
    }

    /// Step `path` from a stream to what the stream holds, where it is one. A
    /// name taken from a compressed run or a joined stream means a field of
    /// its contents, the way a name taken from a field that points elsewhere
    /// means a field of what it points at.
    pub(super) fn into_contents<S: Source>(&mut self, doc: &Document<S>, path: &mut Vec<usize>) -> R<()> {
        self.resolve(doc, path)?;
        if matches!(self.memo[path.as_slice()].ty, Ty::Decoded { .. } | Ty::Stitched { .. }) {
            path.push(0);
            self.resolve(doc, path)?;
        }
        Ok(())
    }

    /// The first compressed run or joined stream under `node`, going down
    /// through structures and the fields that point elsewhere and never into
    /// a list. See [`Step::Stream`].
    ///
    /// A field on the way that will not read is passed over rather than
    /// ending the search: the stream is further on, and a broken checksum in
    /// front of it is not the stream.
    pub(super) fn stream_under<S: Source>(&mut self, doc: &Document<S>, node: &[usize]) -> R<Option<Vec<usize>>> {
        /// How many nodes the search opens before it gives up. The wrappers a
        /// container borrows put their run a handful of fields in; a search
        /// that has opened this many is in something that is not a wrapper.
        const LOOKED_AT: usize = 256;
        let mut stack = vec![node.to_vec()];
        let mut looked = 0;
        while let Some(p) = stack.pop() {
            looked += 1;
            if looked > LOOKED_AT {
                break;
            }
            match self.resolve(doc, &p) {
                Ok(()) => {}
                Err(e) if e.interrupted() => return Err(e),
                Err(_) => continue,
            }
            match &self.memo[&p].ty {
                Ty::Decoded { .. } | Ty::Stitched { .. } => return Ok(Some(p)),
                Ty::Struct(s) => {
                    for j in (0..s.fields.len()).rev() {
                        let mut child = p.clone();
                        child.push(j);
                        stack.push(child);
                    }
                }
                Ty::At { .. } => {
                    let mut child = p;
                    child.push(0);
                    stack.push(child);
                }
                _ => {}
            }
        }
        Ok(None)
    }

    /// The schema a node was declared as, seen through the wrappers a
    /// declaration may put round it: the kind, the walk and the key.
    fn declared_schema(&self, path: &[usize]) -> Option<(Arc<str>, Arc<[Step]>, Arc<[KeyPart]>)> {
        let mut ty = self.declared_ty(path).ok()?;
        for _ in 0..64 {
            match ty {
                Ty::Named(n) => ty = self.template.types.get(&*n)?.clone(),
                Ty::Sized { inner, .. } | Ty::SizedBits { inner, .. } | Ty::Origin { inner } | Ty::When { inner, .. } => {
                    ty = *inner
                }
                // The case the file took, which is what a switch round a
                // schema node resolved to.
                Ty::Switch { .. } | Ty::Match { .. } => return None,
                Ty::Schema { kind, table, key } => return Some((kind, table, key)),
                _ => return None,
            }
        }
        None
    }

    /// Where the type of the node at `path` was looked up, for a node declared
    /// as a schema, and where each of its members was, for a node inside a
    /// structure built that way.
    ///
    /// The key first, since it is what the file said this node is, and then
    /// the record the builder found for it. A member's own description last,
    /// as a type: `fBasketSeek` is a pointer because the element in
    /// `TBranch`'s description says so.
    pub(super) fn schema_origins<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> R<Vec<Origin>> {
        let mut out = Vec::new();
        if let Some((kind, table, key)) = self.declared_schema(path) {
            if let Some(built) = self.built_at(doc, path, &kind, &key)? {
                if let Some(from) = &built.from {
                    let label = self.description_label(doc, path, &table, from)?;
                    out.push(schema_origin(label, from.clone()));
                }
            }
        }
        // A field of a structure a builder made, and the description the
        // builder laid that field out from.
        if let Some((&idx, parent)) = path.split_last() {
            if let Some((kind, table, key)) = self.declared_schema(parent) {
                if let Some(built) = self.built_at(doc, parent, &kind, &key)? {
                    if let Some(Some(from)) = built.members_from.get(idx) {
                        let label = self.description_label(doc, parent, &table, from)?;
                        out.push(schema_origin(label, from.clone()));
                    }
                }
            }
        }
        Ok(out)
    }

    /// What was built for the schema node at `path`, from what is kept. The
    /// node has been read, so its build is kept unless it failed.
    fn built_at<S: Source>(&mut self, doc: &Document<S>, path: &[usize], kind: &Arc<str>, key: &[KeyPart]) -> R<Option<Arc<Built>>> {
        let here = self.memo.get(path).map(|r| (r.offset, r.limit));
        let key = match self.key_values(doc, path, key, here) {
            Ok(k) => k,
            Err(e) if e.interrupted() => return Err(e),
            Err(_) => return Ok(None),
        };
        Ok(self.schemas.built.get(&(kind.clone(), key)).map(|k| k.built.clone()))
    }

    /// A description as a reader would name it: the field the table starts
    /// from, and every name down from there to the part meant, with the index
    /// each list took. `streamer_info.object.members.elements[7].ref.object`.
    ///
    /// Named from the path rather than from the walk's steps, because what is
    /// named is often further down than a record: a member's description is
    /// an element inside a class's record, and the walk says nothing past the
    /// record. A step through a field that points elsewhere, or into what a
    /// stream holds, adds no name, since the name before it already says it.
    fn description_label<S: Source>(&mut self, doc: &Document<S>, at: &[usize], table: &[Step], to: &[usize]) -> R<String> {
        let Some(Step::Field(first)) = table.first() else { return Ok(String::new()) };
        let Some(start) = self.find_field(at, first) else { return Ok(String::new()) };
        if !to.starts_with(&start) {
            return Ok(String::new());
        }
        for k in 0..=to.len() {
            self.resolve(doc, &to[..k])?;
        }
        let mut label = first.to_string();
        for k in start.len()..to.len() {
            match self.memo.get(&to[..k]).map(|r| r.ty.base().clone()) {
                Some(Ty::Struct(s)) => {
                    if let Some(f) = s.fields.get(to[k]) {
                        label.push('.');
                        label.push_str(&f.name);
                    }
                }
                Some(Ty::Array { .. } | Ty::Repeat { .. } | Ty::PointerList { .. } | Ty::Chain { .. } | Ty::Gather { .. }) => {
                    label.push_str(&format!("[{}]", to[k]));
                }
                _ => {}
            }
        }
        Ok(label)
    }

    /// The relation a schema node's key makes: the key as the template writes
    /// it, and as this file filled it in. `fClassName.text, version` and
    /// `"TTree", 19`, coming to `TTree v19`.
    pub(super) fn schema_relation<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> Option<Relation> {
        let (kind, _, key) = self.declared_schema(path)?;
        let builder = self.template.schemas.get(&*kind)?.clone();
        let here = self.memo.get(path).map(|r| (r.offset, r.limit));
        let values = self.key_values(doc, path, &key, here).ok()?;
        // Nothing to say about a key the template fixed outright: that is the
        // type the declaration names, and the type column says it already.
        if key.iter().all(|k| matches!(k, KeyPart::TextLit(_))) {
            return None;
        }
        let written = key
            .iter()
            .map(|k| match k {
                KeyPart::Int(e) | KeyPart::Text(e) => write_expr(e),
                KeyPart::TextLit(s) => Some(format!("{s:?}")),
            })
            .collect::<Option<Vec<_>>>()?
            .join(", ");
        let substituted = values
            .iter()
            .map(|v| match v {
                KeyValue::Int(n) => n.to_string(),
                KeyValue::Text(s) => format!("{s:?}"),
            })
            .collect::<Vec<_>>()
            .join(", ");
        Some(Relation { role: Role::Type, written, substituted, result: builder.key_text(&values) })
    }
}

/// One answer naming a description. Its value is left empty: what a
/// description reads as is a count of its fields, which says nothing about why
/// this is the type it is, and the label already names it.
fn schema_origin(label: String, path: Vec<usize>) -> Origin {
    Origin { role: Role::Type, label, path, value: String::new(), target_bits: None }
}

/// A made-up format whose records say which description lays them out, so
/// the evaluator's half of [`Ty::Schema`] is tested apart from any real one.
///
/// A description is an id and a list of named fields, each a name and a
/// width in bytes. A record is an id and then its fields, laid out as the
/// description with that id says.
#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::source::MemSource;
    use crate::template::{Endian::Big, Expr as E, Ty as T, Until};

    /// Builds a record's fields from the description with the record's id,
    /// and counts how many times it was asked.
    #[derive(Debug, Default)]
    struct Fields {
        builds: Arc<AtomicUsize>,
        /// Read what the description's own `layout` field is before building,
        /// for the description that is typed by itself.
        read_layout: bool,
    }

    impl crate::template::SchemaBuilder for Fields {
        fn build(&self, key: &[KeyValue], table: &mut dyn Descriptions) -> R<Built> {
            self.builds.fetch_add(1, Ordering::SeqCst);
            let Some(want) = key.first().and_then(KeyValue::as_int) else { return fail("no id") };
            for record in table.records()? {
                let Some(id) = table.find(&record, &["id"])? else { continue };
                if table.int(&id)? != Some(want) {
                    continue;
                }
                if self.read_layout {
                    let layout = table.find(&record, &["layout"])?.expect("a layout");
                    table.node(&layout)?;
                }
                let list = table.find(&record, &["fields"])?.expect("fields");
                let mut fields = Vec::new();
                let mut from = Vec::new();
                for i in 0..table.count(&list)? {
                    let i = i.to_string();
                    let name = table.find(&list, &[&i, "name"])?.expect("name");
                    let width = table.find(&list, &[&i, "width"])?.expect("width");
                    let name = table.text(&name)?;
                    let bits = table.int(&width)?.unwrap_or(0) as u32 * 8;
                    fields.push((name, T::UInt { bits, endian: Big }));
                    from.push(Some(table.find(&list, &[&i])?.expect("a field description")));
                }
                let fields: Vec<(&str, T)> = fields.iter().map(|(n, t)| (n.as_str(), t.clone())).collect();
                return Ok(Built { ty: T::structure(&format!("Laid{want}"), fields), from: Some(record), members_from: from });
            }
            fail(format!("no description {want}"))
        }

        fn key_text(&self, key: &[KeyValue]) -> String {
            format!("layout {}", key.first().and_then(KeyValue::as_int).unwrap_or(0))
        }
    }

    fn field_desc() -> T {
        T::structure("FieldDesc", vec![("len", T::u8()), ("name", T::utf8(E::field("len"))), ("width", T::u8())])
    }

    fn desc() -> T {
        T::structure("Desc", vec![("id", T::u8()), ("n", T::u8()), ("fields", T::array(field_desc(), E::field("n")))])
    }

    fn laid(table: Vec<Step>) -> T {
        T::structure("Record", vec![("id", T::u8()), ("body", T::schema("fields", table, vec![KeyPart::Int(E::field("id"))]))])
    }

    fn template(builder: Fields) -> Template {
        let root = T::structure(
            "Root",
            vec![
                ("count", T::u8()),
                ("descs", T::array(desc(), E::field("count"))),
                ("records", T::repeat(laid(vec![Step::field("descs"), Step::each()]), Until::End)),
            ],
        );
        Template::new("t", root).with_schema("fields", Arc::new(builder))
    }

    /// Two descriptions and three records. Description 1 is a byte called `a`
    /// and two bytes called `cd`; description 2 is four bytes called `x`.
    fn bytes() -> Vec<u8> {
        let mut b = vec![2];
        b.extend([1, 2, 1, b'a', 1, 2, b'c', b'd', 2]);
        b.extend([2, 1, 1, b'x', 4]);
        b.extend([1, 0x11, 0x22, 0x33]);
        b.extend([2, 0, 0, 1, 0]);
        b.extend([1, 0x44, 0x55, 0x66]);
        b
    }

    fn doc(bytes: Vec<u8>) -> Document<MemSource> {
        Document::new(MemSource(bytes))
    }

    #[test]
    fn a_type_is_built_from_the_records_the_file_describes_it_with() {
        let d = doc(bytes());
        let mut ev = Evaluator::new(template(Fields::default()));
        let body = [2, 0, 1];
        let node = ev.node(&d, &body).unwrap();
        assert_eq!(node.type_name, "Laid1");
        assert_eq!(node.child_count, 2);
        assert_eq!(node.size_bits, 3 * 8);
        assert_eq!(ev.node(&d, &[2, 0, 1, 1]).unwrap().name, "cd");
        assert_eq!(ev.node(&d, &[2, 0, 1, 1]).unwrap().value, Value::UInt(0x2233));
        // The second record is laid out by the other description.
        assert_eq!(ev.node(&d, &[2, 1, 1, 0]).unwrap().value, Value::UInt(256));

        // The key is a relation like a switch's: as written, as read, and the
        // description that names.
        let relations = ev.relations(&d, &body).unwrap();
        let typed = relations.iter().find(|r| r.role == Role::Type).expect("a type relation");
        assert_eq!((typed.written.as_str(), typed.substituted.as_str(), typed.result.as_str()), ("id", "1", "layout 1"));
        // And a member says which description of a field it was laid out from.
        let origins = ev.origins(&d, &[2, 0, 1, 1]).unwrap();
        let from = origins.iter().find(|o| o.role == Role::Type).expect("a description");
        assert_eq!((from.label.as_str(), from.path.as_slice()), ("descs[0].fields[1]", &[1, 0, 2, 1][..]));
        let whole = ev.origins(&d, &body).unwrap();
        assert!(whole.iter().any(|o| o.role == Role::Type && o.label == "descs[0]" && o.path == vec![1, 0]), "{whole:?}");
    }

    #[test]
    fn two_nodes_of_one_key_share_one_built_type() {
        let d = doc(bytes());
        let builds = Arc::new(AtomicUsize::new(0));
        let mut ev = Evaluator::new(template(Fields { builds: builds.clone(), read_layout: false }));
        assert_eq!(ev.node(&d, &[2, 0, 1, 0]).unwrap().value, Value::UInt(0x11));
        assert_eq!(builds.load(Ordering::SeqCst), 1);
        assert_eq!(ev.node(&d, &[2, 1, 1, 0]).unwrap().size_bits, 4 * 8);
        assert_eq!(builds.load(Ordering::SeqCst), 2);
        // The third record is laid out like the first, and nothing is built.
        assert_eq!(ev.node(&d, &[2, 2, 1, 1]).unwrap().value, Value::UInt(0x5566));
        assert_eq!(ev.node(&d, &[2, 2, 1]).unwrap().type_name, "Laid1");
        assert_eq!(builds.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_schema_that_describes_itself_is_refused_with_its_name() {
        // A description whose own layout is typed by the description with its
        // id: building layout 1 reads description 1's layout, which is layout 1.
        let desc = T::structure(
            "Desc",
            vec![
                ("id", T::u8()),
                ("n", T::u8()),
                ("fields", T::array(field_desc(), E::field("n"))),
                ("layout", T::schema("fields", vec![Step::field("descs"), Step::each()], vec![KeyPart::Int(E::field("id"))])),
            ],
        );
        // The descriptions are pointed at rather than laid out in front, so
        // that placing `data` does not measure them first: measured from the
        // front, a description's layout is asked before `data` is, and from
        // there the table is not in sight at all.
        let root = T::structure(
            "Root",
            vec![
                ("count", T::u8()),
                ("descs", T::at(E::lit(1), T::array(desc, E::field("count")))),
                ("data", T::schema("fields", vec![Step::field("descs"), Step::each()], vec![KeyPart::Int(E::lit(1))])),
            ],
        );
        let t = Template::new("t", root).with_schema("fields", Arc::new(Fields { read_layout: true, ..Default::default() }));
        let d = doc(vec![1, 1, 1, 1, b'a', 1, 0x7f]);
        let mut ev = Evaluator::new(t);
        match ev.node(&d, &[2]) {
            Err(EvalError::Failed(why)) => assert!(why.contains("layout 1"), "{why}"),
            other => panic!("a description read with itself should be refused, not {other:?}"),
        }
    }

    #[test]
    fn an_edit_to_a_description_rebuilds_what_it_typed() {
        let mut d = doc(bytes());
        let mut ev = Evaluator::new(template(Fields::default()));
        assert_eq!(ev.node(&d, &[2, 0, 1]).unwrap().size_bits, 3 * 8);
        // `cd` is two bytes wide; the description now says one. The width is
        // the ninth byte of the file.
        d.overwrite_bits(9 * 8, &[1], 8);
        ev.invalidate_from(9 * 8);
        let body = ev.node(&d, &[2, 0, 1]).unwrap();
        assert_eq!(body.size_bits, 2 * 8);
        assert_eq!(ev.node(&d, &[2, 0, 1, 1]).unwrap().value, Value::UInt(0x22));
        // An edit after every description leaves what was built standing: a
        // record's own byte is not what typed it.
        let builds = Arc::new(AtomicUsize::new(0));
        let mut ev = Evaluator::new(template(Fields { builds: builds.clone(), read_layout: false }));
        ev.node(&d, &[2, 0, 1, 1]).unwrap();
        d.overwrite_bits(17 * 8, &[0x99], 8);
        ev.invalidate_from(17 * 8);
        assert_eq!(ev.node(&d, &[2, 0, 1, 1]).unwrap().value, Value::UInt(0x99));
        assert_eq!(builds.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_walk_steps_into_the_stream_under_a_field() {
        // The descriptions are inside a stream, under a wrapper that puts a
        // byte in front of it. The walk says "the stream in here" rather than
        // naming the run, and a name taken from the run means its contents.
        let descs = T::structure("Descs", vec![("count", T::u8()), ("descs", T::array(desc(), E::field("count")))]);
        let packed = T::structure(
            "Packed",
            vec![("tag", T::u8()), ("run", T::decoded(E::Remaining, crate::codec::Codec::Stored, descs))],
        );
        let table = vec![Step::field("packed"), Step::stream(), Step::field("descs"), Step::each()];
        let root = T::structure(
            "Root",
            vec![("packed_len", T::u8()), ("packed", T::sized(E::field("packed_len"), packed)), ("records", T::repeat(laid(table), Until::End))],
        );
        let t = Template::new("t", root).with_schema("fields", Arc::new(Fields::default()));
        let mut b = vec![16, 0xee];
        b.extend(bytes().iter().take(15));
        b.extend([1, 0x11, 0x22, 0x33]);
        let d = doc(b);
        let mut ev = Evaluator::new(t);
        assert_eq!(ev.node(&d, &[2, 0, 1]).unwrap().type_name, "Laid1");
        assert_eq!(ev.node(&d, &[2, 0, 1, 1]).unwrap().value, Value::UInt(0x2233));
        let origins = ev.origins(&d, &[2, 0, 1, 1]).unwrap();
        let from = origins.iter().find(|o| o.role == Role::Type && !o.path.is_empty()).expect("a description");
        assert_eq!(from.label, "packed.run.descs[0].fields[1]");
        // A description in a stream is a claim about bytes the stream cannot
        // place in the file, so any edit rebuilds.
        assert!(ev.schemas.reaches_past(0));
    }
}
