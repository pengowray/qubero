//! What a field checks, and whether it passes.
//!
//! Two questions, kept apart on purpose. [`Evaluator::check_of`] says what a
//! checksum field covers and how many bytes that is, and reads none of them: it
//! is asked on every move of the cursor, and a panel that had to inflate eight
//! hundred megabytes to draw a row would not draw the row.
//! [`Evaluator::run_check`] takes the sum, and is asked for by something that
//! has decided the work is worth doing.
//!
//! The rule the whole file is built around: a check that cannot be made must
//! say so, never guess. A run whose bytes have not arrived is `Pending` and the
//! caller asks again; a run past the decoders' cap, or a stream that will not
//! unpack, is a refusal with a reason; a template naming a field that is not
//! there answers nothing at all. What must never come back is `ok: false`
//! because bytes were missing, because a length was measured wrong, or because
//! the sum ran over a compressed stream that the format meant to be read
//! unpacked. A mismatch reported on a file that is fine is worse than no check:
//! it teaches a reader to ignore the one place the interface is meant to be
//! believed.
//!
//! Every expression a check holds is worked out at the *end* of the structure
//! the check field sits in, rather than where the field is. See
//! [`Covers`](crate::template::Covers): a checksum is nearly always written
//! before the thing it covers, and an expression that could only look backwards
//! could not name the size two fields further on.

use super::*;
use crate::checksum::Checksum;
use crate::template::{Check, Covers, Named};

/// What the field at a path checks. No bytes are read to answer this.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckInfo {
    /// "crc32", "crc16", "sum8", "sum", "sha1", "adler32" — what the interface names
    /// it. See [`Checksum::as_str`].
    pub algorithm: &'static str,
    /// The bytes summed, when they are a run of the file: offset and length, in
    /// bytes. A reader can be sent to these.
    pub over: Option<(u64, u64)>,
    /// The run whose unpacked contents are summed, when the sum is not over
    /// bytes of the file at all: offset and length of the *compressed* run, so
    /// that a reader has somewhere to be sent even though the summed bytes are
    /// nowhere in the file.
    pub unpacked_from: Option<(u64, u64)>,
    /// How many bytes the sum is over. The unpacked length where the file
    /// writes one down, so an interface can decide whether to run the check
    /// without being asked. It is what the file claims, not what a decoder
    /// produced: nothing is checked against it, and `run_check` sums whatever
    /// actually came out.
    pub covered_bytes: u64,
    /// The run of the check field's own bytes, and the byte they are read as,
    /// when the sum is over a record the field sits inside. `None` for every
    /// check that sums the bytes as they are.
    ///
    /// A reader looking at the covered run has to be told this or the
    /// arithmetic does not add up in front of them: a tar header sums to a
    /// number that summing those five hundred and twelve bytes by hand will
    /// not give. See [`Check::blank`](crate::template::Check::blank).
    pub blanked: Option<Blanked>,
}

/// The check field's own bytes, and what they are read as while the sum runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Blanked {
    /// Offset and length, in bytes, of the check field itself. Inside the
    /// covered run, always: a blank that does not land there is a check that
    /// is not made at all.
    pub at: u64,
    pub len: u64,
    /// What each of those bytes is summed as. A space for tar, a zero for the
    /// formats that zero the field.
    pub byte: u8,
}

/// What came of taking the sum. The two forms are printed to the algorithm's
/// own width, so an interface can compare them as strings and show them side by
/// side without knowing anything about the arithmetic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    pub computed: String,
    pub stored: String,
    pub ok: bool,
}

/// What a check turned out to cover, once the names and expressions in it have
/// been worked out. The step both queries share.
struct Coverage {
    algorithm: Checksum,
    /// Where the summed bytes are, when they are in the file.
    over: Option<(u64, u64)>,
    /// The compressed run to unpack and sum, and the path of the field holding
    /// it, which is what opens the stream.
    unpacked: Option<((u64, u64), Vec<usize>)>,
    covered_bytes: u64,
    blanked: Option<Blanked>,
}

impl Evaluator {
    /// What the field at `path` checks, or nothing when it checks nothing.
    ///
    /// Cheap: the field and the run it covers are resolved and measured, which
    /// is work a panel showing either of them has already paid for, and not a
    /// byte of the covered run is read.
    ///
    /// Nothing, rather than an error, for every way a check can fail to apply:
    /// the field is not a checksum, its guard says this file does not have one
    /// here, the run it names is not in the file, or the template named a field
    /// that is not there. The last of those is a template bug and is caught by
    /// `checks_resolve` in `formats`, which walks every template and fails on a
    /// name no structure has.
    pub fn check_of<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> R<Option<CheckInfo>> {
        let Some(c) = self.coverage(doc, path)? else { return Ok(None) };
        Ok(Some(CheckInfo {
            algorithm: c.algorithm.as_str(),
            over: c.over,
            unpacked_from: c.unpacked.as_ref().map(|(run, _)| *run),
            covered_bytes: c.covered_bytes,
            blanked: c.blanked,
        }))
    }

    /// Take the checksum at `path` and compare it with what the file wrote.
    ///
    /// Reads the covered bytes, and unpacks the covered stream where the sum is
    /// over what a run comes to rather than over the run: that is the whole
    /// cost of the format, and it is why this is a call of its own rather than
    /// something [`check_of`](Self::check_of) does on the way past.
    ///
    /// Nothing when there is no check here. A refusal, with the reason, when
    /// there is one and it cannot be made: the run is longer than the decoders
    /// will take, or the stream will not unpack. Bytes that have not arrived
    /// are `Pending` as everywhere else, so a caller fetches them and asks
    /// again rather than being told the file is broken.
    pub fn run_check<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> R<Option<Verdict>> {
        let Some(c) = self.coverage(doc, path)? else { return Ok(None) };
        let bytes = match (&c.over, &c.unpacked) {
            (Some((at, len)), _) => {
                if *len > crate::codec::CAP_BYTES as u64 {
                    return fail(too_large(*len));
                }
                let mut bytes = self.read_in(doc, 0, at * 8, len * 8)?;
                // The field reads as something else than what is written
                // there, for the formats that seal a record the checksum is
                // part of. `coverage` has already made sure the run lands
                // inside what was read, so the slice cannot be out of bounds.
                if let Some(b) = c.blanked {
                    let from = (b.at - at) as usize;
                    bytes[from..from + b.len as usize].fill(b.byte);
                }
                bytes
            }
            (None, Some((_, at))) => {
                let at = at.clone();
                match self.open_space_at(doc, &at)? {
                    space::Opened::Space(id) => match self.spaces.buf(id) {
                        Some(bytes) => bytes.as_ref().clone(),
                        None => return fail("this stream is no longer open"),
                    },
                    space::Opened::Refused(why) => return fail(refused(why)),
                }
            }
            // Neither: `coverage` never builds one, and a check with nothing to
            // sum would answer `ok` over no bytes, which is the shape of answer
            // this file exists to prevent.
            (None, None) => return Ok(None),
        };
        let Some(stored) = self.stored_value(doc, path, c.algorithm)? else { return Ok(None) };
        let computed = c.algorithm.over(&bytes);
        Ok(Some(Verdict { ok: computed == stored, computed, stored }))
    }

    /// The bytes a check covers, worked out but not read.
    ///
    /// Every path out of here that is not a run of the file with a length is
    /// `None`. A check whose coverage half resolves is a check that must not be
    /// made.
    fn coverage<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> R<Option<Coverage>> {
        let Some((&idx, parent)) = path.split_last() else { return Ok(None) };
        self.resolve(doc, path)?;
        // Offsets in a decoded stream are offsets of that stream, and an
        // interface handed one would read the file at that number and sum other
        // bytes entirely. A stream opened as a document of its own is asked
        // through its own reading, where its fields are in space 0, so this
        // costs only the fields shown inline under a stream in the listing.
        if self.memo.get(path).is_none_or(|r| r.space != 0) {
            return Ok(None);
        }
        // Either a field of a structure, or an element of a list of sums one
        // level further out. The two are told apart by what the parent is,
        // and a template says which it means: see `Field::elem_check`.
        let (check, parent, fields) = match self.memo.get(parent).map(|r| r.ty.clone()) {
            Some(Ty::Struct(s)) => {
                let Some(check) = s.fields.get(idx).and_then(|f| f.check.clone()) else { return Ok(None) };
                (check, parent.to_vec(), s.fields.len())
            }
            // The list itself is a field of the structure holding it, and the
            // check is declared there. Every expression in it is then worked
            // out at the end of *that* structure, as it is for a plain field:
            // what a sum in a list can name is what its record can name, and
            // the index it sits at, which `Expr::Idx` answers.
            Some(Ty::Array { .. } | Ty::Repeat { .. }) => {
                let Some((&list, grandparent)) = parent.split_last() else { return Ok(None) };
                let Some(Ty::Struct(s)) = self.memo.get(grandparent).map(|r| r.ty.clone()) else { return Ok(None) };
                let Some(check) = s.fields.get(list).and_then(|f| f.elem_check.clone()) else { return Ok(None) };
                (check, grandparent.to_vec(), s.fields.len())
            }
            _ => return Ok(None),
        };
        let parent = &parent[..];
        // Where every expression in the check is worked out from: one past the
        // last field of the structure, so that `find_field`'s "only what is
        // written before you" takes in the whole structure. No node is ever
        // resolved there; it is a place to ask questions from.
        let end = [parent, &[fields]].concat();
        if let Some(when) = &check.when {
            if self.eval_expr(doc, &end, when)? == 0 {
                return Ok(None);
            }
        }
        self.covered(doc, path, parent, &end, &check)
    }

    fn covered<S: Source>(
        &mut self,
        doc: &Document<S>,
        path: &[usize],
        parent: &[usize],
        end: &[usize],
        check: &Check,
    ) -> R<Option<Coverage>> {
        // Where the check field's own bytes are, for a format that reads them
        // as something else while summing. Worked out here, before the match,
        // so that the closure below borrows nothing.
        let own = match check.blank {
            Some(_) => self.field_run(doc, path)?,
            None => None,
        };
        let of = |over: Option<(u64, u64)>, unpacked, covered_bytes| {
            let blanked = match (check.blank, over, own) {
                (None, ..) => None,
                (Some(byte), Some((at, len)), Some((f, n))) if f >= at && f + n <= at + len => {
                    Some(Blanked { at: f, len: n, byte })
                }
                // A blank with nowhere to go: the field is not inside the run,
                // or the sum is over something unpacked, where the field's own
                // bytes are not among the summed ones at all. Either way the
                // template has said something it cannot mean, and summing the
                // bytes as they sit there would report every file as broken.
                (Some(_), ..) => return Ok(None),
            };
            Ok(Some(Coverage { algorithm: check.algorithm, over, unpacked, covered_bytes, blanked }))
        };
        match &check.over {
            Covers::UpToHere => {
                let at = self.memo[path].offset;
                if at % 8 != 0 {
                    return Ok(None);
                }
                self.run_in_file(doc, 0, at / 8).map_or(Ok(None), |run| of(Some(run), None, run.1))
            }
            Covers::Run { at, len } => {
                let start = self.memo[parent].offset;
                if start % 8 != 0 {
                    return Ok(None);
                }
                let (at, len) = (self.eval_expr(doc, end, at)?, self.eval_expr(doc, end, len)?);
                let (Ok(at), Ok(len)) = (u64::try_from(at), u64::try_from(len)) else { return Ok(None) };
                match self.run_in_file(doc, start / 8 + at, len) {
                    Some(run) => of(Some(run), None, run.1),
                    None => Ok(None),
                }
            }
            Covers::Field { name } => {
                let Some(p) = self.named_field(doc, path, name)? else { return Ok(None) };
                let Some(run) = self.field_run(doc, &p)? else { return Ok(None) };
                of(Some(run), None, run.1)
            }
            Covers::Unpacked { name, len } => {
                let Some(p) = self.named_field(doc, path, name)? else { return Ok(None) };
                // A run the format wrote in verbatim is the file: the bytes are
                // there to be pointed at, and a reader sent to a stream instead
                // would be sent to the same bytes with a worse name for them.
                if !matches!(self.memo[&p].ty, Ty::Decoded { .. }) {
                    return Ok(None);
                }
                let stored = matches!(&self.memo[&p].ty, Ty::Decoded { codec, .. } if codec.is_stored());
                let Some(run) = self.field_run(doc, &p)? else { return Ok(None) };
                if stored {
                    return of(Some(run), None, run.1);
                }
                // What the file says it comes to, for an interface deciding
                // whether to unpack it unasked. Only that: the sum is over what
                // the decoder produces, however far off this turns out to be.
                let claimed = match len {
                    Some(e) => u64::try_from(self.eval_expr(doc, end, e)?).unwrap_or(run.1),
                    None => run.1,
                };
                of(None, Some((run, p)), claimed)
            }
        }
    }

    /// A run of the file, if the whole of it is in the file. A length that runs
    /// past the end is a file cut short or a template measuring wrongly, and
    /// summing what is there would report a mismatch on bytes nobody wrote.
    fn run_in_file<S: Source>(&self, doc: &Document<S>, at: u64, len: u64) -> Option<(u64, u64)> {
        (at.checked_add(len)? <= doc.len_bytes()).then_some((at, len))
    }

    /// Where a field is and how long it is, in bytes, when it is a whole number
    /// of them and it is in the file.
    fn field_run<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> R<Option<(u64, u64)>> {
        self.resolve(doc, path)?;
        let size = self.size_of(doc, path)?;
        let r = &self.memo[path];
        if r.space != 0 || r.offset % 8 != 0 || size % 8 != 0 {
            return Ok(None);
        }
        Ok(self.run_in_file(doc, r.offset / 8, size / 8))
    }

    /// The field a check names, wherever [`Named`] says to look for it.
    ///
    /// A field whose contents are somewhere else in the file is its contents,
    /// as it is everywhere else a path is walked (see `descend` in `expr`).
    /// [`Ty::At`] costs no bytes where it is declared, so the node standing
    /// there has an empty run and its first child is the thing. A check that
    /// stopped at the declaration would sum no bytes and print a pass over
    /// nothing, which is the one answer this file exists to prevent. What xz
    /// and lzip need, where the run is the whole of the member and the header
    /// is fields laid over the front of it.
    fn named_field<S: Source>(&mut self, doc: &Document<S>, path: &[usize], name: &Named) -> R<Option<Vec<usize>>> {
        let Some(mut p) = self.field_named(doc, path, name)? else { return Ok(None) };
        while matches!(self.memo[&p].ty, Ty::At { .. }) {
            p.push(0);
            self.resolve(doc, &p)?;
        }
        Ok(Some(p))
    }

    /// Where the name lands, before that peeling.
    fn field_named<S: Source>(&mut self, doc: &Document<S>, path: &[usize], name: &Named) -> R<Option<Vec<usize>>> {
        match name {
            Named::Here(name) => self.field_out_from(doc, path, name),
            // Backwards through the records, and never forwards. A ZIP data
            // descriptor is the only thing here that reaches this way, and it
            // has to: it is a record written after the data it seals, because
            // the writer was streaming and did not know the number until the
            // data had gone by. Everything else a template can say looks
            // backwards or inwards, and so does this: the elements after this
            // one have not been placed, and would not be the answer if they
            // had.
            Named::Earlier(field) => {
                let field = field.clone();
                let Some(p) = self.sibling_field_path(doc, path, &field)? else { return Ok(None) };
                self.resolve(doc, &p)?;
                Ok(Some(p))
            }
            // One of a list, at an index worked out where the check field
            // stands. A list shorter than the one the sums are in, or a path
            // that names nothing here, is a check that cannot be made rather
            // than an error: a format writes its digests for some of its
            // streams and not others, and the ones it left out have to answer
            // nothing at all.
            Named::Elem { array, index } => {
                let (array, index) = (array.clone(), index.clone());
                let here = self.memo.get(path).map(|r| (r.offset, r.limit));
                // How long the list is, before asking it for anything. A list
                // of sums may be longer than the list of things they are
                // about, and an index past the end would otherwise resolve to
                // whatever sits after the last element: a run of the file with
                // a length, which is exactly the shape of a wrong answer this
                // file exists to refuse.
                let Ok(at) = self.within_path(doc, path, &array) else { return Ok(None) };
                let n = self.node(doc, &at)?.child_count;
                let i = self.eval_expr_at(doc, path, &index, here)?;
                if i < 0 || u128::try_from(i).is_ok_and(|i| i >= u128::from(n)) {
                    return Ok(None);
                }
                match self.elem_within_path(doc, path, &array, &index, &[], here) {
                    Ok(p) => {
                        self.resolve(doc, &p)?;
                        Ok(Some(p))
                    }
                    Err(e) if e.interrupted() => Err(e),
                    Err(_) => Ok(None),
                }
            }
        }
    }

    /// A field of the structure the check sits in, or of a structure that one
    /// sits inside.
    ///
    /// Unlike everything else that looks a field up by name, this looks at the
    /// whole of each structure rather than only at what was written before.
    /// Reaching outwards is what a RAR 5 file header needs: its `data_crc32` is
    /// three levels inside the block, and the data it sums is the block's last
    /// field.
    fn field_out_from<S: Source>(&mut self, doc: &Document<S>, path: &[usize], name: &str) -> R<Option<Vec<usize>>> {
        let mut cur = path.to_vec();
        while cur.pop().is_some() {
            let found = match self.memo.get(&cur).map(|r| &r.ty) {
                Some(Ty::Struct(s)) => s.fields.iter().position(|f| &*f.name == name),
                _ => None,
            };
            if let Some(j) = found {
                let mut p = cur.clone();
                p.push(j);
                self.resolve(doc, &p)?;
                return Ok(Some(p));
            }
        }
        Ok(None)
    }

    /// What the file wrote in the check field, printed the way the computed sum
    /// will be so the two can be compared.
    ///
    /// A digest is read as the bytes it is; anything narrower is read as the
    /// number the template gave an endianness to, which is the only thing that
    /// knows whether a PNG's four bytes are the same number as a ZIP's.
    ///
    /// The field's own bytes are read first either way, and that is not a
    /// wasted read: it is what reports bytes still on their way. A value asked
    /// for without it can come back as a run nobody has fetched, which has no
    /// number in it, which would read as a field that checks nothing rather
    /// than as a field whose bytes have not arrived.
    fn stored_value<S: Source>(&mut self, doc: &Document<S>, path: &[usize], of: Checksum) -> R<Option<String>> {
        let want = of.digits() as u64 / 2;
        let (bytes, more) = self.field_bytes(doc, path, want.max(WIDEST_SUM))?;
        if of.is_digest() {
            if more || bytes.len() as u64 != want {
                return Ok(None);
            }
            return Ok(Some(crate::checksum::hex_bytes(&bytes)));
        }
        Ok(self.node(doc, path)?.value.as_int().map(|v| of.stored(v as u128)))
    }
}

/// How many bytes of a check field are read to make sure it is there. Wider
/// than any sum here is written, so a field holding a narrow sum in a wide slot
/// is still read whole and still reports what has not arrived.
const WIDEST_SUM: u64 = 32;

fn too_large(len: u64) -> String {
    format!(
        "Too much to check: {} bytes, and the limit is {}.",
        crate::encode::commas(len),
        crate::encode::commas(crate::codec::CAP_BYTES as u64)
    )
}

/// Why a stream a check is over would not unpack. The reasons are the decoders'
/// own; nothing is added to them here beyond saying what was being attempted.
fn refused(why: Refusal) -> String {
    format!("Can't check this: the data it covers {}.", match why {
        Refusal::TooLarge => "unpacks to more than the limit",
        Refusal::Unaligned => "doesn't start on a byte",
        Refusal::Failed => "wouldn't unpack",
        Refusal::Settings => "doesn't say how it was packed",
    })
}

#[cfg(test)]
mod tests {
    use crate::checksum::{crc16_arc, crc32, sha1, sum8};
    use crate::document::Document;
    use crate::eval::{EvalError, Evaluator};
    use crate::formats;
    use crate::source::MemSource;
    use crate::template::{Check, Checksum, Covers, Endian::Big, Expr as E, Named, Template, Ty as T};

    /// A reading of `bytes` as the named format, and the path of the first
    /// field called `name` under `at`, so a test can say which field it means
    /// in the format's own words rather than in child indices.
    struct Read {
        doc: Document<MemSource>,
        ev: Evaluator,
    }

    impl Read {
        fn of(template: &str, bytes: Vec<u8>) -> Read {
            Read {
                doc: Document::new(MemSource(bytes)),
                ev: Evaluator::new(formats::builtin(template).expect("a template by that name")),
            }
        }
        fn with(template: Template, bytes: Vec<u8>) -> Read {
            Read { doc: Document::new(MemSource(bytes)), ev: Evaluator::new(template) }
        }
        fn at(&mut self, path: &[usize], name: &str) -> Vec<usize> {
            self.ev
                .child_named(&self.doc, path, name)
                .unwrap()
                .unwrap_or_else(|| panic!("no field called {name}"))
        }
        fn info(&mut self, path: &[usize]) -> Option<super::CheckInfo> {
            self.ev.check_of(&self.doc, path).unwrap()
        }
        fn verdict(&mut self, path: &[usize]) -> Option<super::Verdict> {
            self.ev.run_check(&self.doc, path).unwrap()
        }
        /// A verdict that has to exist, since every one of these is over a file
        /// this test built and knows the answer for.
        fn must(&mut self, path: &[usize]) -> super::Verdict {
            self.verdict(path).expect("this field checks something")
        }
    }

    /// A PNG of one IHDR chunk and an IEND, both with the CRC the format asks
    /// for. Small enough to write out, real enough that the template reads it.
    fn png() -> Vec<u8> {
        let mut v = b"\x89PNG\r\n\x1a\n".to_vec();
        for (ty, data) in [(&b"IHDR"[..], &[0, 0, 0, 1, 0, 0, 0, 1, 8, 0, 0, 0, 0][..]), (b"IEND", &[])] {
            v.extend_from_slice(&(data.len() as u32).to_be_bytes());
            let mut covered = ty.to_vec();
            covered.extend_from_slice(data);
            v.extend_from_slice(&covered);
            v.extend_from_slice(&crc32(&covered).to_be_bytes());
        }
        v
    }

    #[test]
    fn a_png_chunk_says_what_its_crc_covers_without_reading_it() {
        let mut r = Read::of("png", png());
        let crc = r.at(&[1, 0], "crc");
        let info = r.info(&crc).expect("a chunk crc checks something");
        assert_eq!(info.algorithm, "crc32");
        // The type and the data: four bytes of name and thirteen of header,
        // starting where the length ends.
        assert_eq!(info.over, Some((12, 17)));
        assert_eq!(info.unpacked_from, None);
        assert_eq!(info.covered_bytes, 17);
    }

    #[test]
    fn a_png_chunk_crc_passes_and_then_catches_a_changed_byte() {
        let mut r = Read::of("png", png());
        let crc = r.at(&[1, 0], "crc");
        assert!(r.must(&crc).ok, "the chunk this test wrote is not broken");

        // The width, inside the covered run. One bit of it.
        let mut broken = png();
        broken[19] ^= 1;
        let mut r = Read::of("png", broken);
        let crc = r.at(&[1, 0], "crc");
        let v = r.must(&crc);
        assert!(!v.ok);
        assert_ne!(v.computed, v.stored);
        assert_eq!(v.stored.len(), 10, "printed as a CRC-32 is printed: {}", v.stored);
    }

    #[test]
    fn a_png_length_is_outside_what_the_crc_covers() {
        // The four bytes in front of the type are not summed, so changing one
        // does not make the sum wrong. It makes the file wrong in a way the
        // chunk after it notices, which is not this field's business.
        let mut broken = png();
        broken[11] = 12;
        let mut r = Read::of("png", broken);
        let crc = r.at(&[1, 0], "crc");
        // Sixteen bytes now, since the declared length settles the run.
        assert_eq!(r.info(&crc).unwrap().covered_bytes, 16);
    }

    /// A gzip member over `content`, deflated with a stored block, with the
    /// header check written when `header_crc` says to.
    fn gzip(content: &[u8], header_crc: bool) -> Vec<u8> {
        let mut v = vec![0x1f, 0x8b, 8, if header_crc { 0x02 } else { 0 }];
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&[0, 3]);
        if header_crc {
            let low = (crc32(&v) & 0xffff) as u16;
            v.extend_from_slice(&low.to_le_bytes());
        }
        // One stored deflate block: final, type zero, then the length and its
        // complement, then the bytes.
        v.push(0x01);
        v.extend_from_slice(&(content.len() as u16).to_le_bytes());
        v.extend_from_slice(&(!(content.len() as u16)).to_le_bytes());
        v.extend_from_slice(content);
        v.extend_from_slice(&crc32(content).to_le_bytes());
        v.extend_from_slice(&(content.len() as u32).to_le_bytes());
        v
    }

    #[test]
    fn a_gzip_checks_its_header_and_the_file_that_went_in() {
        let mut r = Read::of("gzip", gzip(b"the contents of the file", true));
        let head = r.at(&[], "header_crc");
        let info = r.info(&head).expect("the header check is there when the flag is set");
        assert_eq!(info.algorithm, "crc16");
        assert_eq!(info.over, Some((0, 10)));
        let v = r.must(&head);
        assert!(v.ok, "computed {} stored {}", v.computed, v.stored);
        assert_eq!(v.stored.len(), 6, "printed as sixteen bits: {}", v.stored);

        // The trailer is over the unpacked file, which is nowhere in the gzip:
        // the run it came out of is what a reader can be sent to instead.
        let crc = r.at(&[], "crc32");
        let info = r.info(&crc).expect("the trailer checks the file");
        assert_eq!(info.over, None);
        assert!(info.unpacked_from.is_some());
        assert_eq!(info.covered_bytes, 24, "what the trailer says the file came to");
        assert!(r.must(&crc).ok);
    }

    #[test]
    fn a_gzip_without_the_flag_has_no_header_check_to_make() {
        // The field is nothing at all, and a sum of the header against a
        // stored nothing would be a mismatch on a file that is fine.
        let mut r = Read::of("gzip", gzip(b"no header check here", false));
        let head = r.at(&[], "header_crc");
        assert_eq!(r.info(&head), None);
        assert_eq!(r.verdict(&head), None);
    }

    #[test]
    fn a_changed_gzip_header_fails_its_own_check_and_not_the_trailer() {
        let mut broken = gzip(b"the contents of the file", true);
        broken[4] = 0x40; // the timestamp, which the header check covers.
        let mut r = Read::of("gzip", broken);
        let head = r.at(&[], "header_crc");
        assert!(!r.must(&head).ok);
        let crc = r.at(&[], "crc32");
        assert!(r.must(&crc).ok, "the file that went in has not changed");
    }

    /// A ZIP of one entry, stored or deflated, with the sizes written in the
    /// local header the way a writer that knew them writes them.
    fn zip(content: &[u8], deflate: bool) -> Vec<u8> {
        let packed = if deflate {
            miniz_oxide::deflate::compress_to_vec(content, 6)
        } else {
            content.to_vec()
        };
        let name = b"one.txt";
        let mut v = Vec::new();
        v.extend_from_slice(b"PK\x03\x04");
        v.extend_from_slice(&20u16.to_le_bytes());
        v.extend_from_slice(&0u16.to_le_bytes());
        v.extend_from_slice(&(if deflate { 8u16 } else { 0 }).to_le_bytes());
        v.extend_from_slice(&0u16.to_le_bytes());
        v.extend_from_slice(&0u16.to_le_bytes());
        v.extend_from_slice(&crc32(content).to_le_bytes());
        v.extend_from_slice(&(packed.len() as u32).to_le_bytes());
        v.extend_from_slice(&(content.len() as u32).to_le_bytes());
        v.extend_from_slice(&(name.len() as u16).to_le_bytes());
        v.extend_from_slice(&0u16.to_le_bytes());
        v.extend_from_slice(name);
        v.extend_from_slice(&packed);
        v.extend_from_slice(b"PK\x05\x06");
        v.extend_from_slice(&[0; 16]);
        v.extend_from_slice(&0u16.to_le_bytes());
        v
    }

    #[test]
    fn a_stored_zip_entry_is_summed_where_its_bytes_are() {
        let mut r = Read::of("zip", zip(b"stored, so these bytes are the file", false));
        let crc = r.at(&[0, 0, 1], "crc32");
        let info = r.info(&crc).expect("a local entry checks its file");
        // Stored, so the bytes are in the file and a reader can be sent to
        // them rather than to a stream that copies them.
        assert_eq!(info.over, Some((37, 35)));
        assert_eq!(info.unpacked_from, None);
        assert!(r.must(&crc).ok);
    }

    #[test]
    fn a_deflated_zip_entry_is_summed_over_what_it_unpacks_to() {
        let content = b"deflate me, deflate me, deflate me, deflate me".repeat(4);
        let mut r = Read::of("zip", zip(&content, true));
        let crc = r.at(&[0, 0, 1], "crc32");
        let info = r.info(&crc).expect("a local entry checks its file");
        assert_eq!(info.over, None, "the summed bytes are nowhere in the archive");
        assert!(info.unpacked_from.is_some());
        assert_eq!(info.covered_bytes, content.len() as u64);
        assert!(r.must(&crc).ok);

        // A byte of the compressed run: the sum is of what comes out, so a
        // change to what goes in either fails the check or fails to unpack.
        let mut broken = zip(&content, true);
        let at = broken.len() - 30;
        broken[at] ^= 0xff;
        let mut r = Read::of("zip", broken);
        let crc = r.at(&[0, 0, 1], "crc32");
        match r.ev.run_check(&r.doc, &crc) {
            Ok(Some(v)) => assert!(!v.ok),
            // Deflate notices most damage itself, and a stream that will not
            // unpack is a refusal with a reason rather than a mismatch: there
            // is no sum to compare, so nothing is claimed about one.
            Err(EvalError::Failed(why)) => assert!(why.contains("unpack"), "{why}"),
            other => panic!("a changed stream is wrong or unreadable, not {other:?}"),
        }
    }

    /// An LHA level 0 entry holding `content` with the method `-lh0-`, which
    /// writes the file in verbatim.
    fn lha(content: &[u8]) -> Vec<u8> {
        let mut head = Vec::new();
        head.extend_from_slice(b"-lh0-");
        head.extend_from_slice(&(content.len() as u32).to_le_bytes());
        head.extend_from_slice(&(content.len() as u32).to_le_bytes());
        head.extend_from_slice(&0u32.to_le_bytes());
        head.push(0x20);
        head.push(0); // level 0
        head.push(5);
        head.extend_from_slice(b"a.txt");
        head.extend_from_slice(&crc16_arc(content).to_le_bytes());
        let mut v = vec![head.len() as u8, sum8(&head)];
        v.extend_from_slice(&head);
        v.extend_from_slice(content);
        v.push(0);
        v
    }

    #[test]
    fn an_lha_entry_checks_its_header_and_its_stored_file() {
        let mut r = Read::of("lha", lha(b"verbatim bytes"));
        let header = r.at(&[0, 0], "header");
        let sum = r.at(&header, "header_checksum");
        let info = r.info(&sum).expect("the header sums itself");
        assert_eq!(info.algorithm, "sum8");
        // Every byte of the header after the checksum, which is what the size
        // byte counts.
        assert_eq!(info.over, Some((2, 27)));
        let v = r.must(&sum);
        assert!(v.ok, "computed {} stored {}", v.computed, v.stored);
        assert_eq!(v.stored.len(), 4, "printed as eight bits: {}", v.stored);

        let crc = r.at(&header, "crc");
        let info = r.info(&crc).expect("a stored entry checks its file");
        assert_eq!(info.algorithm, "crc16");
        assert_eq!(info.covered_bytes, 14);
        assert!(r.must(&crc).ok);
    }

    #[test]
    fn an_lha_entry_packed_by_a_decoder_we_lack_offers_no_file_check() {
        // The same entry under `-lh1-`, which packs against an adaptive
        // Huffman tree that no decoder here reads. The bytes after the header
        // are not the file, and a CRC-16 of them matches nothing: the check
        // has to disappear rather than fail.
        //
        // `-lh5-` used to stand here for the same reason and no longer can,
        // since it unpacks: see `codec::lha`. What the rule turns on is
        // whether the run opens, not whether it was compressed.
        let mut bytes = lha(b"verbatim bytes");
        bytes[2..7].copy_from_slice(b"-lh1-");
        let head = bytes[0] as usize;
        bytes[1] = sum8(&bytes[2..2 + head]);
        let mut r = Read::of("lha", bytes);
        let header = r.at(&[0, 0], "header");
        let crc = r.at(&header, "crc");
        assert_eq!(r.info(&crc), None);
        assert_eq!(r.verdict(&crc), None);
        // The header check is not conditional and is still made.
        let sum = r.at(&header, "header_checksum");
        assert!(r.must(&sum).ok);
    }

    /// A method that *does* unpack declares its check over what came out,
    /// rather than over the packed bytes that are not the file. The run it
    /// points a reader at is the compressed one, since the summed bytes are
    /// nowhere in the file at all.
    #[test]
    fn an_lha_entry_we_can_unpack_checks_the_file_it_unpacks_to() {
        let mut bytes = lha(b"verbatim bytes");
        bytes[2..7].copy_from_slice(b"-lh5-");
        let head = bytes[0] as usize;
        bytes[1] = sum8(&bytes[2..2 + head]);
        let mut r = Read::of("lha", bytes);
        let header = r.at(&[0, 0], "header");
        let crc = r.at(&header, "crc");
        let info = r.info(&crc).expect("a method that unpacks checks its file");
        assert_eq!(info.algorithm, "crc16");
        assert_eq!(info.over, None, "the summed bytes are not a run of the file");
        assert_eq!(info.unpacked_from, Some((29, 14)), "but the packed run is, and a reader can be sent to it");
    }

    /// A git index with no entries: a header, nothing, and the SHA-1 that
    /// seals it.
    fn git_index(version: u32) -> Vec<u8> {
        let mut v = b"DIRC".to_vec();
        v.extend_from_slice(&version.to_be_bytes());
        v.extend_from_slice(&0u32.to_be_bytes());
        let seal = sha1(&v);
        v.extend_from_slice(&seal);
        v
    }

    #[test]
    fn a_git_index_seals_everything_above_its_hash() {
        let mut r = Read::of("gitindex", git_index(2));
        let seal = r.at(&[], "checksum");
        let info = r.info(&seal).expect("the index seals itself");
        assert_eq!(info.algorithm, "sha1");
        assert_eq!(info.over, Some((0, 12)));
        let v = r.must(&seal);
        assert!(v.ok, "computed {} stored {}", v.computed, v.stored);
        assert_eq!(v.stored.len(), 40, "a digest is forty characters, not a number");

        let mut broken = git_index(2);
        broken[7] = 3;
        let mut r = Read::of("gitindex", broken);
        let seal = r.at(&[], "checksum");
        assert!(!r.must(&seal).ok);
    }

    #[test]
    fn a_field_that_checks_nothing_answers_nothing() {
        let mut r = Read::of("png", png());
        let length = r.at(&[1, 0], "length");
        assert_eq!(r.info(&length), None);
        assert_eq!(r.verdict(&length), None);
        // And so does the root, which has no field above it to hold a check.
        assert_eq!(r.info(&[]), None);
    }

    /// A template of one structure: two bytes, a sum over them, and whatever
    /// coverage the test wants to try.
    fn made_up(over: Covers) -> Template {
        Template::new(
            "made-up",
            T::structure("Made", vec![("a", T::u8()), ("b", T::u8()), ("sum", T::u8())])
                .field_check("sum", Check::of(Checksum::Sum8, over)),
        )
    }

    #[test]
    fn a_template_naming_a_field_that_is_not_there_checks_nothing() {
        // The shape of a typo. Nothing is answered, so nothing is claimed, and
        // `every_check_resolves` in `formats` fails on it before a file is
        // ever loaded.
        let bytes = vec![1, 2, 3];
        let mut r = Read::with(made_up(Covers::Field { name: Named::here("nowhere") }), bytes.clone());
        assert_eq!(r.info(&[2]), None);
        assert_eq!(r.verdict(&[2]), None);

        // And a run that reaches past the end of the file, which is the other
        // way a coverage fails to resolve: a file cut short, or a length the
        // template worked out wrongly.
        let mut r = Read::with(made_up(Covers::Run { at: E::lit(0), len: E::lit(64) }), bytes);
        assert_eq!(r.info(&[2]), None);
        assert_eq!(r.verdict(&[2]), None);
    }

    #[test]
    fn a_sum_over_bytes_that_have_not_arrived_is_asked_for_again() {
        // The one answer that must never be a mismatch. A source holding none
        // of the file says so, and the caller fetches and asks again; a sum
        // taken over the zeroes that stand in for the bytes would report a
        // broken file on every unloaded chunk.
        let mut r = Read::with(made_up(Covers::Run { at: E::lit(0), len: E::lit(2) }), vec![1, 2, 3]);
        assert!(r.must(&[2]).ok, "the sum of one and two is three");

        let doc = Document::new(crate::source::ChunkStore::new(3, 4096, 8));
        let mut ev = Evaluator::new(made_up(Covers::Run { at: E::lit(0), len: E::lit(2) }));
        match ev.run_check(&doc, &[2]) {
            Err(EvalError::Pending(_)) => {}
            other => panic!("bytes that have not arrived are pending, not {other:?}"),
        }
    }

    #[test]
    fn a_sum_field_whose_own_bytes_are_missing_is_asked_for_again() {
        // The other half of the rule, and the easier one to get wrong: the
        // covered run is here and the field holding the sum is not. A number
        // read out of bytes nobody fetched is not a number, and answering "no
        // check here" would make the check vanish from the panel the moment a
        // reader scrolled past the end of a loaded chunk.
        let made = || made_up(Covers::Run { at: E::lit(0), len: E::lit(2) });
        let mut store = crate::source::ChunkStore::new(3, 2, 8);
        store.insert(0, Box::new([1, 2]));
        let doc = Document::new(store);
        let mut ev = Evaluator::new(made());
        match ev.run_check(&doc, &[2]) {
            Err(EvalError::Pending(_)) => {}
            other => panic!("the sum field's own bytes are pending, not {other:?}"),
        }
        // And with them, the same file answers.
        let mut store = crate::source::ChunkStore::new(3, 2, 8);
        store.insert(0, Box::new([1, 2]));
        store.insert(1, Box::new([3]));
        let doc = Document::new(store);
        let mut ev = Evaluator::new(made());
        assert!(ev.run_check(&doc, &[2]).unwrap().unwrap().ok);
    }

    #[test]
    fn a_run_past_the_cap_is_refused_rather_than_taken() {
        // Nothing this large is ever summed: the answer says why, and says it
        // in words an interface can put in front of a reader.
        let big = crate::codec::CAP_BYTES as i128 + 1;
        let mut r = Read::with(
            made_up(Covers::Run { at: E::lit(0), len: E::lit(big) }),
            vec![0; 8],
        );
        // The run is not in the file at all here, so it does not resolve.
        assert_eq!(r.info(&[2]), None);
    }

    #[test]
    fn a_check_reaches_a_field_of_a_structure_it_sits_inside() {
        // What RAR 5 needs: the sum is written three levels inside a block and
        // covers a field of the block itself.
        let inner = T::structure("Inner", vec![("sum", T::u8())])
            .field_check("sum", Check::of(Checksum::Sum8, Covers::Field { name: Named::here("body") }));
        let t = Template::new(
            "nested",
            T::structure("Outer", vec![("inner", inner), ("body", T::bytes(E::lit(3)))]),
        );
        let mut r = Read::with(t, vec![6, 1, 2, 3]);
        let info = r.info(&[0, 0]).expect("the sum reaches out to the body");
        assert_eq!(info.over, Some((1, 3)));
        assert!(r.must(&[0, 0]).ok, "one and two and three come to six");
    }

    #[test]
    fn a_check_expression_may_name_a_field_written_after_it() {
        // The one place a template looks forward. A checksum is nearly always
        // the first field of the header it seals, and the size of what it
        // covers is written after it.
        let t = Template::new(
            "forward",
            T::structure("Sealed", vec![("sum", T::u8()), ("len", T::u8()), ("body", T::bytes(E::field("len")))])
                .field_check(
                    "sum",
                    Check::of(Checksum::Sum8, Covers::Run { at: E::lit(2), len: E::field("len") }),
                ),
        );
        let mut r = Read::with(t, vec![15, 3, 4, 5, 6]);
        let info = r.info(&[0]).expect("the run is named by a later field");
        assert_eq!(info.over, Some((2, 3)));
        assert!(r.must(&[0]).ok, "four and five and six come to fifteen");
    }

    #[test]
    fn a_guard_that_is_zero_removes_the_check() {
        let guarded = || {
            Template::new(
                "guarded",
                T::structure("Guarded", vec![("kind", T::u8()), ("sum", T::u8()), ("body", T::u8())]).field_check(
                    "sum",
                    Check::of(Checksum::Sum8, Covers::Field { name: Named::here("body") })
                        .only_when(E::field("kind").equals(E::lit(7))),
                ),
            )
        };
        let mut r = Read::with(guarded(), vec![7, 5, 5]);
        assert!(r.must(&[1]).ok);
        // The same bytes and the same field, and the only difference is the
        // byte the guard reads.
        let mut r = Read::with(guarded(), vec![8, 5, 5]);
        assert_eq!(r.info(&[1]), None, "the guard says this file has no check here");
        assert_eq!(r.verdict(&[1]), None);
    }

    #[test]
    fn a_digest_field_of_the_wrong_width_answers_nothing() {
        // Twenty bytes are read as a digest. A template pointing SHA-1 at a
        // field that is not twenty bytes has said something it cannot mean,
        // and a hash compared against four bytes would always mismatch.
        let t = Template::new(
            "short",
            T::structure("Short", vec![("body", T::u8()), ("hash", T::bytes(E::lit(4)))]).field_check(
                "hash",
                Check::of(Checksum::Sha1, Covers::Field { name: Named::here("body") }),
            ),
        );
        let mut r = Read::with(t, vec![1, 2, 3, 4, 5]);
        assert!(r.info(&[1]).is_some(), "the coverage resolves");
        assert_eq!(r.verdict(&[1]), None, "and the stored form does not");
    }

    #[test]
    fn an_unpacked_check_over_a_run_nothing_decodes_answers_nothing() {
        // `Unpacked` needs a stream. A template that pointed it at plain bytes
        // would otherwise sum the packed bytes, which matches nothing.
        let t = Template::new(
            "plain",
            T::structure("Plain", vec![("sum", T::u8()), ("body", T::bytes(E::lit(2)))]).field_check(
                "sum",
                Check::of(Checksum::Sum8, Covers::Unpacked { name: Named::here("body"), len: None }),
            ),
        );
        let mut r = Read::with(t, vec![3, 1, 2]);
        assert_eq!(r.info(&[0]), None);
    }

    /// A template of a sealed record: a byte, the sum, a byte, and the sum is
    /// over all three with its own read as `blank`.
    fn sealed(blank: Option<u8>) -> Template {
        let mut check = Check::of(Checksum::Sum8, Covers::Run { at: E::lit(0), len: E::lit(3) });
        if let Some(b) = blank {
            check = check.blanking(b);
        }
        Template::new(
            "sealed",
            T::structure("Sealed", vec![("a", T::u8()), ("sum", T::u8()), ("b", T::u8())])
                .field_check("sum", check),
        )
    }

    /// What tar, Ogg and a PE header all do: the checksum is inside the run it
    /// covers, and the writer summed a placeholder where it now sits.
    #[test]
    fn a_check_inside_the_run_it_covers_sums_its_own_bytes_as_the_blank() {
        // 1 + 0x20 + 3 is 0x24, which is what a writer that read its own field
        // as a space would have arrived at.
        let mut r = Read::with(sealed(Some(b' ')), vec![1, 0x24, 3]);
        let info = r.info(&[1]).expect("the sum covers the record");
        assert_eq!(info.over, Some((0, 3)));
        assert_eq!(info.blanked.map(|b| (b.at, b.len, b.byte)), Some((1, 1, b' ')));
        assert!(r.must(&[1]).ok, "the field reads as a space while the sum runs");

        // The same bytes with nothing blanked come to something else, so the
        // substitution is doing the work rather than the arithmetic happening
        // to agree.
        let mut r = Read::with(sealed(None), vec![1, 0x24, 3]);
        assert_eq!(r.info(&[1]).and_then(|i| i.blanked), None);
        assert!(!r.must(&[1]).ok);
    }

    #[test]
    fn a_blank_that_does_not_land_in_the_covered_run_checks_nothing() {
        // The sum covers the two bytes after the field, so there is nothing of
        // the field's own in it to blank. A template saying otherwise has said
        // something it cannot mean, and summing the run as it is would report
        // a mismatch on a file nobody touched.
        let t = Template::new(
            "outside",
            T::structure("Outside", vec![("sum", T::u8()), ("a", T::u8()), ("b", T::u8())]).field_check(
                "sum",
                Check::of(Checksum::Sum8, Covers::Run { at: E::lit(1), len: E::lit(2) }).blanking(0),
            ),
        );
        let mut r = Read::with(t, vec![5, 2, 3]);
        assert_eq!(r.info(&[0]), None);
        assert_eq!(r.verdict(&[0]), None);
    }

    #[test]
    fn a_blank_over_an_unpacked_stream_checks_nothing() {
        // The summed bytes are not bytes of the file, so the field is nowhere
        // among them and there is nothing to substitute.
        let t = Template::new(
            "unpacked-blank",
            T::structure("Blanked", vec![("sum", T::u8()), ("body", T::bytes(E::lit(2)))]).field_check(
                "sum",
                Check::of(Checksum::Sum8, Covers::Unpacked { name: Named::here("body"), len: None }).blanking(0),
            ),
        );
        let mut r = Read::with(t, vec![3, 1, 2]);
        assert_eq!(r.info(&[0]), None);
    }

    /// A check over a field that put its bytes somewhere else reaches the
    /// bytes, not the empty slot the field stands in.
    ///
    /// The failure this stops is the worst kind here: the node at the
    /// declaration costs no bytes, so the run resolves, it is zero long, and
    /// the sum of nothing is a number a stored zero can agree with. A pass
    /// over no bytes reads exactly like a pass.
    #[test]
    fn a_check_over_a_field_placed_elsewhere_sums_what_is_there() {
        let t = Template::new(
            "placed",
            T::structure(
                "Placed",
                vec![
                    ("sum", T::u8()),
                    ("at", T::u8()),
                    ("body", T::at_in_window(E::field("at"), T::bytes(E::lit(3)))),
                ],
            )
            .field_check("sum", Check::of(Checksum::Sum8, Covers::Field { name: Named::here("body") })),
        );
        let mut r = Read::with(t, vec![15, 3, 0, 4, 5, 6]);
        let info = r.info(&[0]).expect("the sum reaches the placed run");
        assert_eq!(info.over, Some((3, 3)), "the bytes it points at, not the slot it stands in");
        assert!(r.must(&[0]).ok, "four and five and six come to fifteen");
    }

    /// A list of sums beside the list of things they are about, which is how
    /// 7z writes its digests and how any format with a table of checksums has
    /// to write them.
    #[test]
    fn a_sum_in_a_parallel_list_covers_the_element_at_its_own_index() {
        // Three runs of two bytes, then a sum for each, in the same order.
        let t = Template::new(
            "parallel",
            T::structure(
                "Parallel",
                vec![
                    ("runs", T::array(T::bytes(E::lit(2)), E::lit(3))),
                    (
                        "sums",
                        T::array(T::u8(), E::lit(3)),
                    ),
                ],
            )
            .field_elem_check(
                "sums",
                Check::of(Checksum::Sum8, Covers::Field { name: Named::elem(&["runs"], E::Idx) }),
            ),
        );
        let bytes = vec![1, 2, 10, 20, 100, 101, 3, 30, 201];
        let mut r = Read::with(t, bytes);
        for (i, want) in [(0usize, (0u64, 2u64)), (1, (2, 2)), (2, (4, 2))] {
            let at = [1, i];
            let info = r.info(&at).unwrap_or_else(|| panic!("sum {i} checks something"));
            assert_eq!(info.over, Some(want), "sum {i} covers the run at its own index");
            assert!(r.must(&at).ok, "sum {i}");
        }
    }

    /// And an index with nothing at it answers nothing, rather than failing or
    /// reaching for whatever is nearest. A format writes digests for some of
    /// its streams and not others.
    #[test]
    fn a_sum_whose_index_is_past_the_list_checks_nothing() {
        let t = Template::new(
            "past-the-end",
            T::structure(
                "Short",
                vec![
                    ("runs", T::array(T::bytes(E::lit(2)), E::lit(1))),
                    (
                        "sums",
                        T::array(T::u8(), E::lit(2)),
                    ),
                ],
            )
            .field_elem_check(
                "sums",
                Check::of(Checksum::Sum8, Covers::Field { name: Named::elem(&["runs"], E::Idx) }),
            ),
        );
        let mut r = Read::with(t, vec![1, 2, 3, 3]);
        assert!(r.info(&[1, 0]).is_some(), "the first has a run to cover");
        assert_eq!(r.info(&[1, 1]), None, "the second has none");
    }

    #[test]
    fn every_algorithm_prints_to_its_own_width() {
        // The interface compares two strings, so a sum and the number a field
        // holds have to be printed the same way whatever the field's type is.
        let mut r = Read::of("png", png());
        let crc = r.at(&[1, 0], "crc");
        let v = r.must(&crc);
        assert_eq!(v.computed, v.stored);
        assert!(v.computed.starts_with("0x"), "{}", v.computed);
        assert_eq!(u32::from_str_radix(&v.computed[2..], 16).unwrap(), crc32(&png()[12..29]));
    }

    /// The big-endian sums are read as the numbers their templates declare
    /// rather than as bytes, which is the only thing that knows a PNG's four
    /// bytes are the other way round from a ZIP's.
    #[test]
    fn a_sum_is_read_the_way_round_its_template_says() {
        let t = Template::new(
            "endian",
            T::structure("Endian", vec![("body", T::u8()), ("sum", T::UInt { bits: 32, endian: Big })])
                .field_check("sum", Check::of(Checksum::Crc32, Covers::Field { name: Named::here("body") })),
        );
        let mut v = vec![0x42];
        v.extend_from_slice(&crc32(&[0x42]).to_be_bytes());
        let mut r = Read::with(t, v);
        assert!(r.must(&[1]).ok, "big-endian, as the template declared it");
    }
}
