//! A BP5 directory kept in a ZIP, read as one dataset.
//!
//! Qubero opens one file, and a BP5 dataset is four: `md.0`'s records are
//! written in formats `mmd.0` holds, and their blocks are in `data.0` at
//! offsets that count from where `md.idx` says each step's data starts. So the
//! directory is read as an archive of its files, stored rather than
//! compressed, which puts every file in the one space and every reference from
//! one file into another an offset in it. The archive's records read as any
//! archive's do, and after them `dataset` places each file the dataset needs
//! at its entry's bytes: the index, the formats, where the data file starts,
//! and the metadata last, since a field finds only fields declared before it.
//!
//! Which entry is which file is written in the archive, not in the template:
//! the names are whatever the archive was made with, `md.0` or
//! `steps.bp5/md.0`. So `dataset` is a [`Ty::Schema`](crate::template::Ty::Schema)
//! node whose builder walks the records and matches their names by the last
//! part, and what it builds is those files placed at literal offsets. The
//! walk reads every record to the end record, so any edit to the archive
//! builds the dataset again.
//!
//! A file the archive compressed cannot be read in place, since its bytes are
//! not the file's. It is placed as bytes saying so, and still opens by itself
//! as an entry of the archive.

use std::sync::Arc;

use super::bp5::{index_root, metadata_root};
use super::ffs::metametadata_root;
use super::ffs_schema::{self, DATA_AT, MMD};
use crate::eval::{Descriptions, R};
use crate::template::{Built, Expr as E, KeyPart, KeyValue, SchemaBuilder, Step, Template, Ty as T};

/// The schema kind of the dataset node.
const DATASET: &str = "adios dataset";

/// A local file record's signature.
const LOCAL: i128 = 0x0403_4b50;

/// An ADIOS2 BP5 directory in a ZIP: the archive's records, and the dataset.
///
/// Each entry's data is read as bytes by the archive and as a file of the
/// dataset here, and the dataset's reading is the one counted, since it says
/// what the bytes are.
pub fn adios_zip() -> Template {
    let root = T::structure(
        "AdiosZip",
        vec![
            ("records", crate::formats::zip::records(true)),
            ("dataset", T::schema(DATASET, vec![Step::field("records"), Step::each()], vec![KeyPart::TextLit("BP5".into())])),
        ],
    );
    ffs_schema::register(Template::new("adioszip", root).with_schema(DATASET, Arc::new(Dataset)))
}

/// One entry of the archive, as far as placing it needs.
struct Entry {
    dir: String,
    leaf: String,
    method: i128,
    at: i128,
    len: i128,
    record: Vec<usize>,
}

fn entries(table: &mut dyn Descriptions) -> R<Vec<Entry>> {
    let mut out = Vec::new();
    for record in table.records()? {
        let Some(signature) = table.find(&record, &["signature"])? else { continue };
        if table.int(&signature)? != Some(LOCAL) {
            continue;
        }
        let Some(body) = table.find(&record, &["body"])? else { continue };
        let (Some(name), Some(method), Some(size), Some(extra)) = (
            table.find(&body, &["name"])?,
            table.find(&body, &["compression"])?,
            table.find(&body, &["data_size"])?,
            table.find(&body, &["extra"])?,
        ) else {
            continue;
        };
        let name = table.text(&name)?.replace('\\', "/");
        let method = table.int(&method)?.unwrap_or(-1);
        let len = table.int(&size)?.unwrap_or(0);
        let extra = table.node(&extra)?;
        if extra.space != 0 {
            continue;
        }
        let at = ((extra.offset_bits + extra.size_bits) / 8) as i128;
        let (dir, leaf) = match name.rfind('/') {
            Some(i) => (name[..i].to_string(), name[i + 1..].to_string()),
            None => (String::new(), name),
        };
        out.push(Entry { dir, leaf, method, at, len, record });
    }
    Ok(out)
}

/// The builder for [`DATASET`].
#[derive(Debug)]
struct Dataset;

impl SchemaBuilder for Dataset {
    fn build(&self, _key: &[KeyValue], table: &mut dyn Descriptions) -> R<Built> {
        let entries = entries(table)?;
        // The dataset is the directory the first BP5 file is in: an archive of
        // several datasets is read as its first.
        let dir = entries.iter().find(|e| matches!(e.leaf.as_str(), "md.idx" | "mmd.0" | "md.0")).map(|e| e.dir.clone()).unwrap_or_default();
        let find = |leaf: &str| entries.iter().find(|e| e.dir == dir && e.leaf == leaf);
        let stored = |e: Option<&Entry>| e.is_some_and(|e| e.method == 0);
        let (index, mmd, data, md) = (find("md.idx"), find("mmd.0"), find("data.0"), find("md.0"));
        let mut out = Fields::default();
        if let Some(e) = index {
            out.file("md_idx", e, index_root());
        }
        if let Some(e) = mmd {
            out.file(MMD, e, metametadata_root());
        }
        // Where the data file starts is all the metadata needs of it, and it
        // is declared before the metadata so the metadata can ask.
        if let Some(e) = data.filter(|e| e.method == 0) {
            out.fields.push((DATA_AT, T::computed(E::lit(e.at))));
            out.from.push(Some(e.record.clone()));
            out.named.push((DATA_AT, e.record.last().copied()));
        }
        if let Some(e) = md {
            out.file("md_0", e, metadata_root(stored(index), stored(index) && stored(data)));
        }
        let mut ty = T::structure("Bp5Dataset", out.fields).machinery(&[DATA_AT]);
        // Each file's row says which entry it is, as the archive named it.
        for (field, record) in out.named {
            if let Some(i) = record {
                ty = ty.field_named_from(field, E::elem_within(&["records"], E::lit(i as i128), &["body", "name"]));
            }
        }
        Ok(Built { ty, from: None, members_from: out.from })
    }

    fn key_text(&self, _key: &[KeyValue]) -> String {
        "BP5 dataset".to_string()
    }
}

/// The dataset's fields as they are made.
#[derive(Default)]
struct Fields {
    fields: Vec<(&'static str, T)>,
    from: Vec<Option<Vec<usize>>>,
    named: Vec<(&'static str, Option<usize>)>,
}

impl Fields {
    /// A file of the dataset at its entry's data, read as `root`, or as bytes
    /// saying why when the archive packed it.
    fn file(&mut self, field: &'static str, e: &Entry, root: T) {
        let read = match e.method {
            0 => root,
            method => {
                let word = crate::formats::zip::METHODS.iter().find(|(m, _)| *m == method).map_or("compressed", |(_, w)| *w);
                T::structure(&format!("{word} entry"), vec![("bytes", T::bytes(E::Remaining))]).doc(&packed(word))
            }
        };
        self.fields.push((field, T::at(E::lit(e.at), T::sized(E::lit(e.len), read))));
        self.from.push(Some(e.record.clone()));
        self.named.push((field, e.record.last().copied()));
    }
}

/// Why a file of the dataset stays bytes: the archive packed it, and a packed
/// file's bytes are not the file's, so nothing in it is where its offsets say.
fn packed(method: &str) -> String {
    format!("{method} entry: only a stored entry is read in place (zip -0)")
}
