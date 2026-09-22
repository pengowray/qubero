//! A `torch.save` checkpoint, which is a ZIP holding a pickle and the numbers
//! the pickle names.
//!
//! Since torch 1.6 `torch.save` writes an archive under one folder: `data.pkl`
//! is the pickle, `data/0`, `data/1` and so on are the raw numbers of one
//! storage each, and `version`, `byteorder` and a few dotted names say how it
//! was written. Every entry is stored, never compressed, and aligned so that a
//! reader can map the numbers straight out of the file.
//!
//! So the archive's records read as any archive's do, and after them
//! `checkpoint` places the pickle where the archive keeps it, as the pickle it
//! is. That is the whole of the template: one [`Ty::Schema`](crate::template::Ty::Schema)
//! node, built the way `adios/dataset.rs` builds a BP5 dataset, because which
//! entry is the pickle is written in the archive rather than in the template.
//!
//! Placing the pickle in the archive's own space rather than opening the entry
//! as a stream is what makes a tensor's numbers reachable. A tensor names a
//! storage and the storage is another entry, so the two are an offset apart in
//! one space: `eval/pickletorch.rs` walks the archive's records for the entry
//! the tensor named and reads its numbers there, with the byte addresses the
//! hex view needs.

use std::sync::Arc;

use crate::eval::{Descriptions, R};
use crate::template::{Built, Expr as E, KeyPart, KeyValue, SchemaBuilder, Step, Template, Ty as T};

/// The schema kind of the checkpoint node.
const CHECKPOINT: &str = "torch checkpoint";

/// A local file record's signature.
const LOCAL: i128 = 0x0403_4b50;

/// The entry holding the pickle, which is torch's own name for it and is in
/// every file it has written since 1.6.
pub(crate) const PICKLE_ENTRY: &str = "data.pkl";
/// The folder the storages are under, which is what a tensor's key names an
/// entry of.
pub(crate) const DATA_FOLDER: &str = "data";

/// A `torch.save` archive: the records, and the pickle they hold.
pub fn torch_zip() -> Template {
    let root = T::structure(
        "TorchZip",
        vec![
            ("records", crate::formats::zip::records(true)),
            ("checkpoint", T::schema(CHECKPOINT, vec![Step::field("records"), Step::each()], vec![KeyPart::TextLit("torch".into())])),
        ],
    );
    Template::new("torchzip", root).with_schema(CHECKPOINT, Arc::new(Checkpoint))
}

/// One entry of the archive, as far as placing the pickle needs.
pub(crate) struct Entry {
    pub(crate) name: String,
    pub(crate) method: i128,
    /// How long the entry's data is. Where it starts is not kept: the type is
    /// placed by [`Expr::EntryOf`](crate::template::Expr::EntryOf) now, which
    /// says the same thing in the IR rather than in a number.
    pub(crate) len: i128,
    record: Vec<usize>,
}

impl Entry {
    /// The last part of the name, which is what says which entry this is.
    fn leaf(&self) -> &str {
        self.name.rsplit_once('/').map_or(self.name.as_str(), |(_, leaf)| leaf)
    }
}

/// Every local record of the archive, read out of the reading already made of
/// it.
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
        // An entry whose header was read inside a stream is at no offset in
        // the archive, so nothing can be placed at it.
        if table.node(&extra)?.space != 0 {
            continue;
        }
        out.push(Entry { name, method, len, record });
    }
    Ok(out)
}

/// The builder for [`CHECKPOINT`].
#[derive(Debug)]
struct Checkpoint;

impl SchemaBuilder for Checkpoint {
    fn build(&self, _key: &[KeyValue], table: &mut dyn Descriptions) -> R<Built> {
        let entries = entries(table)?;
        // The checkpoint is the folder the first `data.pkl` is in: an archive
        // holding two is read as its first.
        let held = entries.iter().find(|e| e.leaf() == PICKLE_ENTRY && e.method == 0);
        let Some(pickle) = held else {
            // Recognition said this was a torch archive and the records say
            // otherwise, which is an archive cut short or one whose pickle was
            // compressed by something that rewrote it.
            let ty = T::structure(EMPTY_NAME, Vec::new()).doc(NO_PICKLE);
            return Ok(Built { ty, from: None, members_from: Vec::new() });
        };
        // Where the pickle is, said rather than counted: the entry of the
        // archive whose name the record that holds it writes. The offset is
        // the same number `entries` worked out, and saying it this way is
        // what puts the archive's records in the field's depends-on rows and
        // in the diagram. See [`Expr::EntryOf`](crate::template::Expr::EntryOf).
        let named = E::elem_within(&["records"], E::lit(index_of(pickle) as i128), &["body", "name"]);
        let held = T::at_space(E::entry_of(&["records"], named.clone()), T::sized(E::lit(pickle.len), T::pickle()));
        let ty = T::structure("TorchCheckpoint", vec![(PICKLE_FIELD, held)]).field_named_from(PICKLE_FIELD, named);
        Ok(Built { ty, from: None, members_from: vec![Some(pickle.record.clone())] })
    }

    fn key_text(&self, _key: &[KeyValue]) -> String {
        "PyTorch checkpoint".to_string()
    }
}

/// Which record of the archive an entry is.
fn index_of(entry: &Entry) -> usize {
    entry.record.last().copied().unwrap_or(0)
}

/// What the one field is called before the archive renames it after the entry
/// it is.
const PICKLE_FIELD: &str = "data";
/// What the node is when the archive holds no pickle to place.
const EMPTY_NAME: &str = "TorchCheckpoint";
const NO_PICKLE: &str =
    "No data.pkl: this archive was named a PyTorch checkpoint by its entries, but the records hold no stored data.pkl to read. Open it as a ZIP archive to see what it does hold.";
