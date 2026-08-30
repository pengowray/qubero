//! Following a SQLite record that did not fit on its page.
//!
//! A row is written into one page, and a row can be longer than a page. SQLite
//! keeps as much of it as the rules allow where the row is and puts the rest on
//! a chain of pages elsewhere: each of those holds the number of the next one
//! in its first four bytes and payload in the rest, and the last holds zero.
//! The pages of one chain are wherever they were free when the row was
//! written, so a large blob can be scattered the length of the file.
//!
//! The template stops at that boundary and says so: it reads the bytes that
//! stayed and the number of the page the rest went to, and no further. It has
//! to, because the record's own header can be cut in half by the page break,
//! and a field placed at an offset cannot be in two places.
//!
//! This is the other half. [`payload`] reads the chain and answers with the
//! runs of the file the row occupies, which
//! [`Gathered`](crate::gather::Gathered) reads as the one stream they make.
//! Anything that reads bytes then reads the whole row, with the joins in the
//! right places and nothing invented across them.
//!
//! Every step is reported rather than only the answer, as the other unpackers
//! here do. A chain that stops early, doubles back on itself, or points off the
//! end of the file has said something about the file, and a row that is shorter
//! than it claims is a fact worth showing rather than an error worth throwing.

use std::collections::HashSet;

use crate::document::Document;
use crate::eval::{EvalError, Evaluator, R, Value};
use crate::gather::{Extent, Gathered};
use crate::source::{MemSource, Source};

/// What [`StructDef::packed`](crate::template::StructDef::packed) calls this,
/// so that a payload the template had to stop at is marked as a thing with
/// more behind it, and the panel can find its way back here.
pub const PACKING: &str = "sqlite_overflow";

/// The largest row this will assemble for a panel. A row is usually a few
/// hundred bytes and can be a gigabyte, and a viewer that is redrawn as the
/// cursor moves cannot join a gigabyte each time.
pub const ROW_LIMIT: u64 = 64 << 20;

/// How many of a row's columns a panel shows.
pub const COLUMNS_SHOWN: usize = 64;

/// One column of an assembled row, as the record's own header says to read it.
#[derive(Debug, Clone, PartialEq)]
pub struct Column {
    /// What the serial type is called: `i32`, `f64`, `text, 8189 bytes`.
    pub type_name: String,
    /// The value itself, left as a value rather than as text so that whatever
    /// shows it renders it the way it renders every other value.
    pub value: Value,
    /// Where it sits in the assembled row, which is not where it sits in the
    /// file: use [`Payload::extents`] to get back to the file.
    pub at: u64,
    pub len: u64,
}

/// A row assembled and read as the columns it holds.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Row {
    pub columns: Vec<Column>,
    /// How many columns there are, of which `columns` holds the first few.
    pub total: usize,
    /// Why the columns could not be read, where they could not.
    pub problem: Option<String>,
}

/// Assemble a row and read it as a record.
///
/// The bytes are joined into a stream of their own and the ordinary record
/// template is run over it, so a row that spilled reads by exactly the same
/// rules as a row that did not. The one thing that has to be handed over is the
/// text encoding: the record template normally reads it from the database
/// header, and an assembled row has no header above it.
pub fn read<S: Source>(doc: &Document<S>, found: &Payload, encoding: i128) -> Row {
    let size: u64 = found.extents.iter().map(|e| e.len).sum();
    if size > ROW_LIMIT {
        let mb = ROW_LIMIT / (1 << 20);
        return Row {
            problem: Some(format!("Not assembled: the row is over this viewer's {mb} MB limit.")),
            ..Row::default()
        };
    }
    let gathered = Gathered::new(doc.source(), found.extents.iter().copied());
    let mut bytes = vec![0u8; gathered.len_bytes() as usize];
    gathered.read_bytes(0, &mut bytes);

    let row = Document::new(MemSource(bytes));
    let mut ev = Evaluator::new(crate::template::Template::new(
        "row",
        crate::formats::sqlite::record_encoded(encoding),
    ));
    let columns = match ev.child_named(&row, &[], "columns") {
        Ok(Some(p)) => p,
        _ => {
            return Row {
                problem: Some("The assembled bytes do not read as a record.".into()),
                ..Row::default()
            };
        }
    };
    let Ok(node) = ev.node(&row, &columns) else {
        return Row { problem: Some("The record header does not say how many columns.".into()), ..Row::default() };
    };
    let total = node.child_count as usize;
    // The record's own header, which is where a column's type is written. The
    // field's type says how the bytes were read; the serial type says what
    // SQLite calls it, which is the more useful of the two to show.
    let types = ev.child_named(&row, &[], "types").ok().flatten();
    let mut out = Vec::new();
    for i in 0..total.min(COLUMNS_SHOWN) {
        let at = [columns.as_slice(), &[i]].concat();
        let Ok(column) = ev.node(&row, &at) else { break };
        let serial = types
            .as_ref()
            .and_then(|t| ev.node(&row, &[t.as_slice(), &[i]].concat()).ok())
            .and_then(|n| match n.value {
                Value::Enum { name: Some(name), .. } => Some(name),
                _ => None,
            });
        out.push(Column {
            type_name: serial.unwrap_or_else(|| column.type_name.clone()),
            value: column.value.clone(),
            at: column.offset_bits / 8,
            len: column.size_bits / 8,
        });
    }
    Row { columns: out, total, problem: None }
}

/// A row, and where the file keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Payload {
    /// The runs of the file it occupies, in the order the row reads: what
    /// stayed on the page, then each overflow page's contents in turn.
    pub extents: Vec<Extent>,
    /// How many bytes the cell said the row is.
    pub declared: u64,
    /// How many the chain actually reached. Short of `declared` means the
    /// chain stopped early, and `problem` says why.
    pub found: u64,
    /// The overflow pages walked, in order. Empty for a row that fit.
    pub pages: Vec<u32>,
    /// Why the walk stopped, when it stopped for a reason worth naming.
    pub problem: Option<String>,
}

impl Payload {
    /// Whether the whole row was found.
    pub fn complete(&self) -> bool {
        self.found == self.declared && self.problem.is_none()
    }
}

/// A backstop on how many pages one row may be spread over.
///
/// The walk is already bounded twice over: it stops when it has as many bytes
/// as the row claims, and it refuses a page it has already been to, so it
/// cannot take more steps than the file has pages. This is here for the case
/// those two arguments rest on, which is that the numbers in the file mean
/// what they say.
const LONGEST_CHAIN: usize = 1 << 24;

/// Follow a cell's payload, on its page and wherever else it went.
///
/// `cell` is the path to a `Cell`. A row that fit needs no chain and comes
/// back as the one run it is, so a caller does not have to ask first which
/// kind of row it has.
pub fn payload<S: Source>(ev: &mut Evaluator, doc: &Document<S>, cell: &[usize]) -> R<Payload> {
    let page_size = page_size(ev, doc)?;
    let reserved = int_field(ev, doc, &[], "reserved_space")? as u64;
    // What of a page a payload may use, which is the page less whatever the
    // header holds back at the end of every one of them.
    let usable = page_size.saturating_sub(reserved);
    if usable < 5 {
        return Err(EvalError::Failed(format!("a page has only {usable} usable bytes")));
    }
    let declared = int_field(ev, doc, cell, "payload_size")? as u64;
    let at = named(ev, doc, cell, "payload")?;

    // A row that fit is the bytes where it stands. The template parsed it as
    // the record it is, so the node covers the whole of it.
    let Some(on_page) = ev.child_named(doc, &at, "on_page")? else {
        let node = ev.node(doc, &at)?;
        return Ok(Payload {
            extents: vec![Extent::new(node.offset_bits / 8, node.size_bits / 8)],
            declared,
            found: node.size_bits / 8,
            pages: Vec::new(),
            problem: None,
        });
    };

    let stayed = ev.node(doc, &on_page)?;
    let mut out = Payload {
        extents: vec![Extent::new(stayed.offset_bits / 8, stayed.size_bits / 8)],
        declared,
        found: stayed.size_bits / 8,
        pages: Vec::new(),
        problem: None,
    };
    let mut next = int_field(ev, doc, &at, "overflow_page")? as i64;
    let page_count = doc.len_bytes() / page_size;
    // A page visited twice is a chain that loops, which would otherwise be
    // followed until the file ran out of memory rather than out of pages.
    let mut seen: HashSet<u32> = HashSet::new();

    while out.found < declared {
        if next == 0 {
            out.problem = Some(format!(
                "The chain ended after {} of the {declared} bytes the row claims.",
                out.found
            ));
            break;
        }
        if next < 1 || next as u64 > page_count {
            out.problem =
                Some(format!("Page {next} is not a page of this file, which has {page_count}."));
            break;
        }
        let page = next as u32;
        if !seen.insert(page) {
            out.problem = Some(format!("Page {page} is already in this chain, which loops."));
            break;
        }
        if out.pages.len() >= LONGEST_CHAIN {
            out.problem = Some("The chain is longer than any file could hold.".into());
            break;
        }
        let start = (page as u64 - 1) * page_size;
        // Four bytes for the number of the next page, and the rest is the row.
        let take = (usable - 4).min(declared - out.found);
        out.extents.push(Extent::new(start + 4, take));
        out.found += take;
        out.pages.push(page);
        next = be32(doc, start) as i64;
    }
    Ok(out)
}

/// How big a page is. The field is two bytes and the largest page is 65536,
/// which does not fit in two bytes, so a one means the large size.
fn page_size<S: Source>(ev: &mut Evaluator, doc: &Document<S>) -> R<u64> {
    match int_field(ev, doc, &[], "page_size")? {
        1 => Ok(65536),
        n if n >= 512 => Ok(n as u64),
        n => Err(EvalError::Failed(format!("a page size of {n}"))),
    }
}

/// The four-byte number at `at`, which is how a page says which page follows
/// it. Read from the file rather than through a field, because an overflow
/// page's own bytes are described by the template only when the file's
/// header proves that every leftover page is one of these.
fn be32<S: Source>(doc: &Document<S>, at: u64) -> u32 {
    let mut bytes = [0u8; 4];
    doc.read_bytes(at, &mut bytes);
    u32::from_be_bytes(bytes)
}

fn named<S: Source>(ev: &mut Evaluator, doc: &Document<S>, path: &[usize], name: &str) -> R<Vec<usize>> {
    match ev.child_named(doc, path, name)? {
        Some(p) => Ok(p),
        None => Err(EvalError::Failed(format!("no field {name} at {path:?}"))),
    }
}

fn int_field<S: Source>(ev: &mut Evaluator, doc: &Document<S>, path: &[usize], name: &str) -> R<i128> {
    let p = named(ev, doc, path, name)?;
    match ev.node(doc, &p)?.value {
        Value::UInt(v) => Ok(v as i128),
        Value::Int(v) => Ok(v),
        other => Err(EvalError::Failed(format!("{name} is {other:?}, not a number"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formats::sqlite;
    use crate::gather::Gathered;
    use crate::source::MemSource;

    const PAGE: usize = 512;
    /// Where the pages are in the tree, and where a page's cells are.
    const PAGES: usize = 24;
    const CELLS: usize = 6;

    /// How much of a row of `total` bytes stays on a 512-byte table leaf.
    ///
    /// SQLite's own rules, worked here rather than written down as numbers:
    /// which sizes make a chain of one page and which make two is not obvious,
    /// and a test that hardcodes them tests the number rather than the walk.
    /// A page keeps at least `m` and at most `x`, and the size in between is
    /// chosen so that the overflow pages come out full.
    fn stays(total: usize) -> usize {
        let x = PAGE - 35;
        let m = ((PAGE - 12) * 32 / 255) - 23;
        if total <= x {
            return total;
        }
        let k = m + ((total - m) % (PAGE - 4));
        if k <= x { k } else { m }
    }

    /// A number as SQLite writes one: seven bits to a byte, and the high bit
    /// set on every byte but the last.
    fn varint(mut value: usize) -> Vec<u8> {
        let mut out = vec![(value & 0x7f) as u8];
        value >>= 7;
        while value > 0 {
            out.insert(0, 0x80 | (value & 0x7f) as u8);
            value >>= 7;
        }
        out
    }

    /// A database whose page two holds one row of `total` bytes, with the rest
    /// of it on the pages `chain` names, in that order. The file is `pages`
    /// pages long whatever the chain says, so that a chain can be made to
    /// point past the end.
    ///
    /// Each overflow page is filled with a letter of its own, so that reading
    /// the row back says which page each part of it came from, and in what
    /// order they were joined.
    fn spilled(total: usize, chain: &[u32], pages: usize) -> Vec<u8> {
        let mut cell = varint(total);
        cell.push(1); // the row id
        cell.extend(std::iter::repeat_n(b'A', stays(total)));
        cell.extend_from_slice(&chain.first().copied().unwrap_or(0).to_be_bytes());

        let mut b = header(PAGE);
        b.extend_from_slice(&leaf_page(&[], PAGE - 100, 100));
        b.extend_from_slice(&leaf_page(&[cell], PAGE, 0));
        b.resize(pages * PAGE, 0);
        for (i, page) in chain.iter().enumerate() {
            let at = (*page as usize - 1) * PAGE;
            if at + PAGE > b.len() {
                continue;
            }
            let next = chain.get(i + 1).copied().unwrap_or(0);
            b[at..at + 4].copy_from_slice(&next.to_be_bytes());
            b[at + 4..at + PAGE].fill(b'a' + i as u8);
        }
        b
    }

    /// The hundred-byte database header, and the fields this reads from it.
    fn header(page_size: usize) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(b"SQLite format 3\0");
        b.extend_from_slice(&(page_size as u16).to_be_bytes());
        b.extend_from_slice(&[1, 1, 0, 64, 32, 32]);
        b.extend_from_slice(&[0; 76]);
        b.resize(100, 0);
        b
    }

    /// A b-tree leaf holding `cells`, laid out the way SQLite lays one out:
    /// the header and the cell pointers at the front, the cells at the back.
    fn leaf_page(cells: &[Vec<u8>], len: usize, base: usize) -> Vec<u8> {
        let mut page = vec![0u8; len];
        let mut at = len;
        let mut pointers = Vec::new();
        for cell in cells {
            at -= cell.len();
            page[at..at + cell.len()].copy_from_slice(cell);
            pointers.push((at + base) as u16);
        }
        page[0] = 13;
        page[3..5].copy_from_slice(&(cells.len() as u16).to_be_bytes());
        page[5..7].copy_from_slice(&(at as u16).to_be_bytes());
        for (i, pointer) in pointers.iter().enumerate() {
            page[8 + i * 2..10 + i * 2].copy_from_slice(&pointer.to_be_bytes());
        }
        page
    }

    fn read(doc: &Document<MemSource>, extents: &[Extent]) -> Vec<u8> {
        let gathered = Gathered::new(doc.source(), extents.iter().copied());
        let mut out = vec![0u8; gathered.len_bytes() as usize];
        gathered.read_bytes(0, &mut out);
        out
    }

    fn follow(bytes: Vec<u8>) -> (Document<MemSource>, Payload) {
        let doc = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(sqlite());
        let found = payload(&mut ev, &doc, &[PAGES, 0, CELLS, 0]).expect("a payload");
        (doc, found)
    }

    /// The whole row, read across the pages it was cut into.
    #[test]
    fn a_row_reads_across_the_pages_it_was_cut_into() {
        // 1200 bytes: 184 stay where the row is, and the other 1016 fill two
        // overflow pages exactly.
        let (doc, found) = follow(spilled(1200, &[4, 6], 8));
        assert!(found.complete(), "{found:?}");
        assert_eq!(found.declared, 1200);
        assert_eq!(found.found, 1200);
        assert_eq!(found.pages, vec![4, 6]);
        let bytes = read(&doc, &found.extents);
        assert_eq!(bytes.len(), 1200);
        // What stayed, then the two pages the chain named, in that order.
        assert_eq!(&bytes[..184], &[b'A'; 184]);
        assert_eq!(&bytes[184..184 + 508], &[b'a'; 508]);
        assert_eq!(&bytes[184 + 508..], &[b'b'; 508]);
    }

    /// Standing on a spilled payload opens the row into its columns, which is
    /// what the panel shows. They are read by the ordinary record template
    /// over the assembled bytes, so a row that spilled reads by the same rules
    /// as a row that did not.
    #[test]
    fn standing_on_a_spilled_row_opens_its_columns() {
        // Two columns: a small integer, and a text long enough that the row
        // does not fit on its page.
        let text = "z".repeat(1000);
        let mut header_bytes = vec![1u8]; // the integer's serial type
        header_bytes.extend(varint(13 + text.len() * 2));
        let mut record = varint(header_bytes.len() + 1);
        record.extend(header_bytes);
        record.push(42);
        record.extend(text.as_bytes());

        let total = record.len();
        let local = stays(total);
        let chain = [4u32, 6];
        let mut cell = varint(total);
        cell.push(1); // the row id
        cell.extend_from_slice(&record[..local]);
        cell.extend_from_slice(&chain[0].to_be_bytes());

        let mut b = header(PAGE);
        b.extend_from_slice(&leaf_page(&[], PAGE - 100, 100));
        b.extend_from_slice(&leaf_page(&[cell], PAGE, 0));
        b.resize(8 * PAGE, 0);
        // The rest of the record, cut across the pages the chain names, each
        // holding the next page's number and then its share.
        for (i, page) in chain.iter().enumerate() {
            let at = (*page as usize - 1) * PAGE;
            let from = local + i * (PAGE - 4);
            let take = (record.len() - from).min(PAGE - 4);
            let next = chain.get(i + 1).copied().unwrap_or(0);
            b[at..at + 4].copy_from_slice(&next.to_be_bytes());
            b[at + 4..at + 4 + take].copy_from_slice(&record[from..from + take]);
        }

        let doc = Document::new(MemSource(b));
        let mut ev = Evaluator::new(sqlite());
        // The cursor on the payload, which is the structure marked as packed.
        let explained = ev.explain(&doc, &[PAGES, 0, CELLS, 0, 2], None).expect("an explanation");
        let crate::eval::Explain::SqliteRow { declared, found, columns, total_columns, problem, .. } =
            explained
        else {
            panic!("not a row: {explained:?}");
        };
        assert_eq!(problem, None);
        assert_eq!(declared, total as u64);
        assert_eq!(found, total as u64);
        assert_eq!(total_columns, 2);
        // The integer that stayed on the page, and the text that did not: the
        // second column begins on the page and ends on the pages the chain
        // named. Its value shows as far as the tree shows any long string, so
        // what is checked is its length and that it reads as the text it is.
        assert_eq!(columns[0].value, Value::Int(42));
        assert_eq!(columns[1].type_name, format!("text, {} bytes", text.len()));
        assert_eq!(columns[1].len, text.len() as u64);
        let Value::Str(shown) = &columns[1].value else { panic!("not text: {:?}", columns[1].value) };
        assert!(shown.starts_with("zzzz"), "{shown}");
    }

    /// The chain is followed in the order it gives, not in the order the pages
    /// sit in the file: the pages of one row are wherever they were free.
    #[test]
    fn the_order_is_the_chain_not_the_file() {
        let (doc, found) = follow(spilled(1200, &[6, 4], 8));
        assert_eq!(found.pages, vec![6, 4]);
        // The page named first comes first, though it sits later in the file.
        assert_eq!(found.extents[1].at, 5 * PAGE as u64 + 4);
        assert_eq!(found.extents[2].at, 3 * PAGE as u64 + 4);
        let bytes = read(&doc, &found.extents);
        assert_eq!(&bytes[184..184 + 8], &[b'a'; 8]);
    }

    /// A chain that comes back to a page it has already used is a broken file,
    /// and is reported as one rather than followed for ever.
    #[test]
    fn a_chain_that_loops_is_reported_and_not_followed() {
        // A row long enough to want three pages, given two that point at each
        // other.
        let mut bytes = spilled(2000, &[4, 6], 8);
        let at = 5 * PAGE;
        bytes[at..at + 4].copy_from_slice(&4u32.to_be_bytes());
        let (_, found) = follow(bytes);
        assert_eq!(found.pages, vec![4, 6]);
        assert!(found.problem.as_deref().is_some_and(|p| p.contains("loops")), "{found:?}");
        assert!(!found.complete());
    }

    /// A chain that stops before the row is whole says how far it got, rather
    /// than making the rest up or throwing away what it found.
    #[test]
    fn a_chain_that_ends_early_says_how_far_it_got() {
        let (_, found) = follow(spilled(2000, &[4], 8));
        assert_eq!(found.pages, vec![4]);
        assert!(found.problem.as_deref().is_some_and(|p| p.contains("ended after")), "{found:?}");
        assert_eq!(found.found, stays(2000) as u64 + 508);
    }

    /// A row that fits needs no chain, and comes back as the one run it is, so
    /// that a caller does not have to ask first which kind of row it has.
    #[test]
    fn a_row_that_fits_is_one_run() {
        let mut cell = vec![9, 1]; // a payload of nine bytes, row id 1
        cell.extend_from_slice(&[2, 25, b'h', b'e', b'l', b'l', b'o', 0, 0]);
        let mut b = header(PAGE);
        b.extend_from_slice(&leaf_page(&[], PAGE - 100, 100));
        b.extend_from_slice(&leaf_page(&[cell], PAGE, 0));
        let (_, found) = follow(b);
        assert!(found.pages.is_empty());
        assert_eq!(found.extents.len(), 1);
        assert_eq!(found.found, 9);
        assert!(found.complete());
    }

    /// A chain that points off the end of the file stops, and says so. What
    /// stayed on the row's own page is still known, which is why this reports
    /// rather than fails.
    #[test]
    fn a_page_that_is_not_in_the_file_stops_the_walk() {
        let (_, found) = follow(spilled(1200, &[400], 8));
        assert!(!found.complete());
        assert!(found.problem.as_deref().is_some_and(|p| p.contains("not a page of this file")));
        assert_eq!(found.found, 184);
        assert_eq!(found.extents.len(), 1);
    }
}
