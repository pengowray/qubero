//! A BP5 directory kept in a ZIP, read as one dataset.
//!
//! Qubero opens one file, and a BP5 dataset is four: `md.0`'s records are
//! written in formats `mmd.0` holds, and their blocks are in `data.0` at
//! offsets that count from where `md.idx` says each step's data starts. So the
//! directory is read as an archive of its files, which puts every file in one
//! space and every reference from one file into another an offset in it. The
//! archive's records read as any archive's do, and after them `dataset` places
//! each file the dataset needs: the index, the formats, where the data file
//! starts, and the metadata last, since a field finds only fields declared
//! before it.
//!
//! Which entry is which file is written in the archive, not in the template:
//! the names are whatever the archive was made with, `md.0` or
//! `steps.bp5/md.0`. So `dataset` is a [`Ty::Schema`](crate::template::Ty::Schema)
//! node whose builder walks the records and matches their names by the last
//! part, and what it builds is those files placed at literal offsets. The
//! walk reads every record to the end record, so any edit to the archive
//! builds the dataset again.
//!
//! **Two layouts.** When the archive stores every file of the dataset as it
//! is, each file is placed at its entry's bytes in the archive, so a byte of
//! `md.0` in the hex view is a field of its record. When it compressed any of
//! them, the files are joined into one stream instead, unpacked where they
//! were packed and in the order the dataset reads them, and each file is
//! placed at its offset in that stream. The files still share one space, so a
//! pointer from `md.0` into `data.0` is still one offset; what is lost is a
//! byte of the archive leading to the field it is read as, since an unpacked
//! byte is at no place in the archive.
//!
//! A file packed with a method nothing here unpacks, or encrypted, is placed
//! at its entry's bytes in the archive, as bytes saying why.

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

/// The methods a ZIP entry's data opens with, as `zip.rs` declares them: the
/// stored method, deflate, bzip2, zstd and xz.
const UNPACKED: &[i128] = &[0, 8, 12, 93, 95];

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
    flags: i128,
    /// Where the entry's data starts in the archive, and how many bytes of the
    /// archive it takes.
    at: i128,
    len: i128,
    /// What the header says the file unpacks to: nought for an entry whose
    /// writer did not know yet and wrote the sizes after the data.
    unpacked: i128,
    record: Vec<usize>,
}

impl Entry {
    /// Which record of the archive this is.
    fn index(&self) -> usize {
        self.record.last().copied().unwrap_or(0)
    }

    fn stored(&self) -> bool {
        self.method == 0
    }

    /// Why the entry's data cannot be read as the file, or nothing when it can:
    /// what its row's type says it is, and the note on it. The encrypted flag
    /// is asked first, since an AES entry names a method of its own.
    fn unread(&self) -> Option<Unread> {
        if self.flags & 1 != 0 {
            return Some(Unread { kind: "encrypted entry".to_string(), why: ENCRYPTED.to_string() });
        }
        if UNPACKED.contains(&self.method) {
            return None;
        }
        Some(match crate::formats::zip::METHODS.iter().find(|(m, _)| *m == self.method) {
            Some((_, word)) => Unread { kind: format!("{word} entry"), why: not_unpacked(word) },
            None => Unread { kind: "compressed entry".to_string(), why: unknown_method(self.method) },
        })
    }
}

fn entries(table: &mut dyn Descriptions) -> R<Vec<Entry>> {
    let mut out = Vec::new();
    for record in table.records()? {
        let Some(signature) = table.find(&record, &["signature"])? else { continue };
        if table.int(&signature)? != Some(LOCAL) {
            continue;
        }
        let Some(body) = table.find(&record, &["body"])? else { continue };
        let (Some(name), Some(method), Some(flags), Some(size), Some(unpacked), Some(extra)) = (
            table.find(&body, &["name"])?,
            table.find(&body, &["compression"])?,
            table.find(&body, &["flags"])?,
            table.find(&body, &["data_size"])?,
            table.find(&body, &["unpacked_size"])?,
            table.find(&body, &["extra"])?,
        ) else {
            continue;
        };
        let name = table.text(&name)?.replace('\\', "/");
        let method = table.int(&method)?.unwrap_or(-1);
        let flags = table.int(&flags)?.unwrap_or(0);
        let len = table.int(&size)?.unwrap_or(0);
        let unpacked = table.int(&unpacked)?.unwrap_or(0);
        let extra = table.node(&extra)?;
        if extra.space != 0 {
            continue;
        }
        let at = ((extra.offset_bits + extra.size_bits) / 8) as i128;
        let (dir, leaf) = match name.rfind('/') {
            Some(i) => (name[..i].to_string(), name[i + 1..].to_string()),
            None => (String::new(), name),
        };
        out.push(Entry { dir, leaf, method, flags, at, len, unpacked, record });
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
        let files = [("md_idx", find("md.idx")), (MMD, find("mmd.0")), (DATA_AT, find("data.0")), ("md_0", find("md.0"))];
        let readable = |e: &Option<&Entry>| e.is_some_and(|e| e.unread().is_none());
        let (index, data) = (readable(&files[0].1), readable(&files[2].1));
        let packed = files.iter().filter_map(|(_, e)| *e).any(|e| e.unread().is_none() && !e.stored());
        let roots = |field: &str| match field {
            "md_idx" => index_root(),
            MMD => metametadata_root(),
            _ => metadata_root(index, index && data),
        };
        let mut out = Fields::default();
        if !packed {
            // Every file is stored, so each is read where the archive keeps it.
            for (field, e) in files {
                let Some(e) = e else { continue };
                match e.unread() {
                    Some(unread) => out.unread(field, e, unread),
                    None if field == DATA_AT => out.data_at(e, e.at),
                    None => out.push(field, e, T::at(E::lit(e.at), T::sized(E::lit(e.len), roots(field)))),
                }
            }
            return Ok(out.built(None));
        }
        // Some file is packed, so the files are joined into one stream in the
        // order the dataset reads them, and placed at their offsets in it. The
        // data file goes last, so how long it is never moves another file.
        let order = [files[0], files[1], files[3], files[2]];
        let mut parts = Vec::new();
        let mut start = 0i128;
        for (field, e) in order {
            let Some(e) = e else { continue };
            if let Some(unread) = e.unread() {
                out.unread(field, e, unread);
                continue;
            }
            let Some(len) = part_len(table, e, field == DATA_AT)? else {
                let word = crate::formats::zip::METHODS.iter().find(|(m, _)| *m == e.method).map_or("compressed", |(_, w)| *w);
                out.unread(field, e, Unread { kind: format!("{word} entry"), why: unopened(word) });
                continue;
            };
            parts.push(e.index());
            match field {
                DATA_AT => out.data_at(e, start),
                _ => out.push(field, e, T::at_space(E::lit(start), T::sized(E::lit(len), roots(field)))),
            }
            start += len;
        }
        // The fields keep the order the metadata needs them in, whatever order
        // the parts were joined in.
        out.reorder(&["md_idx", MMD, DATA_AT, "md_0"]);
        let walk = vec![Step::field("records"), Step::elements(&parts), Step::field("body"), Step::field("data")];
        Ok(out.built(Some(walk)))
    }

    fn key_text(&self, _key: &[KeyValue]) -> String {
        "BP5 dataset".to_string()
    }
}

/// How many bytes an entry comes to in the joined stream, worked out the way
/// the stream measures its parts: the size the header says it unpacks to, and
/// where the writer left that out, the stored run itself, or the packed run
/// unpacked to find out. The data file is last and its length moves nothing,
/// so it is not unpacked to measure. Nothing when a packed run will not open.
fn part_len(table: &mut dyn Descriptions, e: &Entry, last: bool) -> R<Option<i128>> {
    if e.unpacked > 0 {
        return Ok(Some(if e.stored() { e.unpacked.min(e.len) } else { e.unpacked }));
    }
    if e.stored() || last {
        return Ok(Some(if e.stored() { e.len } else { 0 }));
    }
    let Some(data) = table.find(&e.record, &["body", "data"])? else { return Ok(None) };
    let contents = [&data[..], &[0]].concat();
    match table.node(&contents) {
        Ok(node) if node.space != 0 => Ok(Some((node.size_bits / 8) as i128)),
        Ok(_) => Ok(None),
        Err(e) if e.interrupted() => Err(e),
        Err(_) => Ok(None),
    }
}

/// How long a part of the joined stream is, as the stream works it out one
/// past the last field of the entry's header: what the header says the file
/// unpacks to, or where that is nought, as long as the run or what it unpacks
/// to.
fn claimed_part_len() -> E {
    E::field("unpacked_size").or(E::lit(-1))
}

/// The dataset's fields as they are made.
#[derive(Default)]
struct Fields {
    fields: Vec<(&'static str, T)>,
    from: Vec<Option<Vec<usize>>>,
    named: Vec<(&'static str, Option<usize>)>,
}

impl Fields {
    fn push(&mut self, field: &'static str, e: &Entry, ty: T) {
        self.fields.push((field, ty));
        self.from.push(Some(e.record.clone()));
        self.named.push((field, Some(e.index())));
    }

    /// Where the data file starts, which is all the metadata needs of it.
    fn data_at(&mut self, e: &Entry, at: i128) {
        self.push(DATA_AT, e, T::computed(E::lit(at)));
    }

    /// A file of the dataset that stays bytes, at its entry's data in the
    /// archive, saying why.
    fn unread(&mut self, field: &'static str, e: &Entry, unread: Unread) {
        // Where the data file starts means nothing when its bytes are not the
        // file's, so it is left out and the blocks say they have no values.
        if field == DATA_AT {
            return;
        }
        let bytes = T::structure(&unread.kind, vec![("bytes", T::bytes(E::Remaining))]).doc(&unread.why);
        self.push(field, e, T::at(E::lit(e.at), T::sized(E::lit(e.len), bytes)));
    }

    /// Put the fields in the order `names` gives, leaving out any not made.
    fn reorder(&mut self, names: &[&str]) {
        let mut order: Vec<usize> = (0..self.fields.len()).collect();
        order.sort_by_key(|&i| names.iter().position(|n| *n == self.fields[i].0).unwrap_or(names.len()));
        self.fields = in_order(std::mem::take(&mut self.fields), &order);
        self.from = in_order(std::mem::take(&mut self.from), &order);
        self.named = in_order(std::mem::take(&mut self.named), &order);
    }

    /// The dataset as one structure, or joined from the parts `walk` lands on
    /// and read as that structure.
    fn built(self, walk: Option<Vec<Step>>) -> Built {
        let mut ty = T::structure("Bp5Dataset", self.fields).machinery(&[DATA_AT]);
        // Each file's row says which entry it is, as the archive named it.
        for (field, record) in self.named {
            if let Some(i) = record {
                ty = ty.field_named_from(field, E::elem_within(&["records"], E::lit(i as i128), &["body", "name"]));
            }
        }
        match walk {
            None => Built { ty, from: None, members_from: self.from },
            Some(walk) => Built { ty: T::stitched(walk, Some(claimed_part_len()), None, ty), from: None, members_from: Vec::new() },
        }
    }
}

/// `items` taken in the order of the indices in `order`.
fn in_order<X>(items: Vec<X>, order: &[usize]) -> Vec<X> {
    let mut items: Vec<Option<X>> = items.into_iter().map(Some).collect();
    order.iter().filter_map(|&i| items.get_mut(i).and_then(Option::take)).collect()
}

/// A file of the dataset that stays bytes: what its row's type says the entry
/// is, and the note saying why.
struct Unread {
    kind: String,
    why: String,
}

/// Why a file of the dataset stays bytes: the archive packed it with a method
/// nothing here unpacks, so its bytes are not the file's. The way out is the
/// folder the archive came from, which opens as a folder.
fn not_unpacked(method: &str) -> String {
    format!("Not unpacked: Qubero doesn't unpack {method} entries. To read this entry, extract the ZIP and open the extracted folder.")
}

/// The same for a method number no ZIP writer is known to use.
fn unknown_method(method: i128) -> String {
    format!("Not unpacked: this entry uses compression method {method}, which Qubero doesn't know. To read this entry, extract the ZIP and open the extracted folder.")
}

/// Why a file of the dataset stays bytes when its entry is encrypted.
const ENCRYPTED: &str = "Encrypted: Qubero doesn't decrypt ZIP entries. To read this entry, extract the ZIP with its password and open the extracted folder.";

/// Why a file of the dataset stays bytes when its entry, written as a stream
/// with no size in its header, would not unpack to find out how long it is.
/// The way out is a test rather than an extraction, which would fail on the
/// same bytes.
fn unopened(method: &str) -> String {
    format!("Not unpacked: unpacking failed, so this entry's {method} data is damaged or cut short. Test the ZIP with another tool, such as unzip -t.")
}
