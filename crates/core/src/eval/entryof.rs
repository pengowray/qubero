//! Where the data of an archive entry begins, found by the entry's name:
//! [`Expr::EntryOf`].
//!
//! A file of its own because the walk is a walk and the expression evaluator's
//! recursion is tight. Everything here is behind one `#[inline(never)]`
//! function, so the frame it costs is on the stack only while an entry is
//! being looked for and never while an ordinary expression is worked out.
//!
//! Two readings answer the same question and both are needed. A local file
//! record says where its own data begins: the header's end, which is the name
//! and the extra field it declares. That is what the template already placed,
//! so the records are walked for the name first. An archive written as a
//! stream leaves both sizes out of the local header and writes them in a
//! descriptor after the data, so a walk from the front has no way on to the
//! next record; `torch.save` writes every entry that way. The central
//! directory at the end holds every entry, and it is read when the walk did
//! not find the name.

use super::*;
use crate::template::Expr;

/// How many records the walk looks through before it gives up and asks the
/// directory instead. An archive of more entries than this has a directory,
/// which is one read rather than thousands.
const MOST_RECORDS: u64 = 4_096;

/// The path inside one record to the name the entry goes by, and to the last
/// field of its header. What follows the extra field is the data, which is
/// what a reader of this expression is after.
const NAME_FIELD: &[&str] = &["body", "name"];
const LAST_FIELD: &[&str] = &["body", "extra"];

/// The same two as the walk hands them over, which counts in owned words.
fn parts(of: &[&str]) -> Vec<String> {
    of.iter().map(|s| (*s).to_string()).collect()
}

impl Evaluator {
    /// Where the data of the archive entry named by `name` begins, in bytes
    /// from the front of the space the field asking is read in.
    ///
    /// Cold, and called from one arm of the expression evaluator. See
    /// [`Expr::EntryOf`] for what the two halves are and why both are read.
    #[inline(never)]
    pub(super) fn entry_of<S: Source>(
        &mut self,
        doc: &Document<S>,
        at: &[usize],
        records: &[String],
        name: &Expr,
        here: Option<(u64, u64)>,
    ) -> R<i128> {
        let want = self.text_at(doc, at, name, here)?;
        if want.is_empty() {
            return fail("no entry name to look for");
        }
        if let Some(found) = self.record_named(doc, at, records, &want)? {
            return Ok(found);
        }
        self.directory_named(doc, at, &want)
    }

    /// The entry of that name among the records the template placed, as the
    /// end of its local header, or nothing when no record there is named that.
    ///
    /// A record the walk could not reach is a record this cannot see, which is
    /// what the directory is for. A record in another space is at no offset in
    /// this one, so it is passed over rather than answered with.
    fn record_named<S: Source>(&mut self, doc: &Document<S>, at: &[usize], records: &[String], want: &str) -> R<Option<i128>> {
        let list = self.within_path(doc, at, records)?;
        let count = self.child_count(doc, &list)?.min(MOST_RECORDS);
        let space = self.memo.get(at).map_or(0, |r| r.space);
        let (named_field, last_field) = (parts(NAME_FIELD), parts(LAST_FIELD));
        for i in 0..count {
            let mut named = list.clone();
            named.push(i as usize);
            let mut last = named.clone();
            if !self.descend(doc, &mut named, &named_field)? {
                continue;
            }
            if self.text_of(doc, &named)? != want {
                continue;
            }
            if !self.descend(doc, &mut last, &last_field)? {
                continue;
            }
            let width = self.size_of(doc, &last)?;
            let found = &self.memo[&last];
            if found.space != space {
                continue;
            }
            return Ok(Some(((found.offset + width) / 8) as i128));
        }
        Ok(None)
    }

    /// The same, out of the central directory at the end of the space.
    ///
    /// Read through the evaluator, so a chunk that has not arrived says
    /// `Pending` and comes back rather than answering out of a run of noughts.
    ///
    /// [`Evaluator::archive`] is shared with the torch reading, and it has one
    /// fallback of its own: a space with no directory at all whose first bytes
    /// are a legacy torch checkpoint's answers with the storages that file
    /// lays out. Nothing else can reach it, since such a file holds no ZIP
    /// records for an expression to name.
    fn directory_named<S: Source>(&mut self, doc: &Document<S>, at: &[usize], want: &str) -> R<i128> {
        let space = self.memo.get(at).map_or(0, |r| r.space);
        let held = self.archive(doc, space)?;
        match held.iter().find(|e| e.name == want) {
            Some(entry) => Ok(entry.at as i128),
            None => fail(format!("no archive entry named {want}")),
        }
    }
}
