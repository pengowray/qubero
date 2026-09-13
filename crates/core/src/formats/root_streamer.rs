//! Reading a streamed C++ object out of a ROOT record, with the class
//! descriptions the file carries for the purpose.
//!
//! A ROOT record's contents are whatever the class that wrote them chose to
//! write. There is no tag, no length per member and no names: a `TTree` is a
//! run of numbers whose meaning is the order `TTree::Streamer` put them in.
//! What makes the file readable without the writing program is that ROOT
//! copies its own class descriptions into the file, in the record the header's
//! `fSeekInfo` points at. That record holds a `TList` of `TStreamerInfo`, one
//! per class: its name, the version of it that wrote, a checksum over the
//! layout, and a `TStreamerElement` per member saying the member's name, a
//! type code, how wide it is, and for an array how many.
//!
//! So the schema is in the file and the schema is itself streamed. That is the
//! circle this module cuts: the handful of classes the descriptions are made
//! of (`TObject`, `TNamed`, `TString`, `TList`, `TObjArray`, the `TArray`
//! family) have streamers ROOT wrote by hand and have never changed, and they
//! are written out here as code. Everything else is read by walking its
//! `TStreamerInfo`, so a `TTree` of version 19 and one of version 20 are the
//! same amount of work and neither is a layout written down here.
//!
//! Three things about the encoding are worth saying outright, because each one
//! is a place a reader that guessed would go wrong.
//!
//! **Every object says how long it is.** An object opens with a four-byte
//! count that has bit 30 set to mark it as a count at all, and then the
//! two-byte version of the class that wrote it. The count is what makes a
//! member this module cannot read survivable: the members after it are still
//! found, because the object is stepped over by its own length rather than by
//! adding up what was read. A few classes, `TObject` and `TString` among them,
//! write no count and no version, which is why they are listed by hand.
//!
//! **A class name is written once.** The first time a class appears in a
//! record its name is spelled out; every later object of that class writes a
//! back-reference instead, and the reference is the position in the buffer
//! where the name was written. So reading the second branch of a tree needs a
//! map from position to class name built while reading the first. The
//! positions ROOT counts from the start of the *key*, not from the start of
//! the unpacked contents, so the key's length is added to every position here.
//! The same map holds objects, which is how a leaf's `fLeafCount` points back
//! at the leaf that counts it.
//!
//! **A pointer member writes a marker byte.** A member declared `int*` with a
//! count elsewhere in the class, which is what `fBasketSeek` is, writes one
//! byte and then the elements. Nothing says how many: the count is another
//! member of the same class, named in the element's `fCountName`, and it has
//! to have been read already.
//!
//! Nothing of ROOT's own source was read to write this. The format is
//! described at root.cern under ROOT I/O, and uproot (BSD-3) is a second
//! reading of the same document.

use std::collections::HashMap;
use std::rc::Rc;

/// Bit 30 of the four bytes in front of an object, saying they are a byte
/// count rather than the first field of the object itself.
const BYTE_COUNT_MASK: u32 = 0x4000_0000;
/// Bit 31 of a class tag, saying the tag names a class rather than being a
/// reference to an object already read.
const CLASS_MASK: u32 = 0x8000_0000;
/// The tag that says a class name follows, spelled out, because this is the
/// first object of that class in the record.
const NEW_CLASS_TAG: u32 = 0xFFFF_FFFF;
/// What ROOT adds to a buffer position before writing it as a reference, so
/// that the two low tag values 0 (null) and 1 (the enclosing object) are never
/// a position.
const MAP_OFFSET: u32 = 2;
/// Bit 14 of a `TObject`'s version, saying a byte count was written in front
/// of the version after all.
const BYTE_COUNT_VMASK: u16 = 0x4000;
/// Bit 14 of an object's version, saying the object was written one member at
/// a time across a whole collection rather than object by object. Nothing here
/// reads that arrangement; an object marked with it is reported and stepped
/// over.
const MEMBERWISE: u16 = 0x4000;
/// Bit 4 of a `TObject`'s `fBits`, saying two more bytes follow the two
/// counters. ROOT sets it for an object something else holds a reference to.
const IS_REFERENCED: u32 = 0x10;

/// How deep the reading of one object goes before it stops. A `TTree` is a
/// tree of branches and every level is a class inside a class, but a real file
/// is a handful deep and a file that claims more is one to stop reading.
const MAX_DEPTH: usize = 24;

/// The largest list or array this reads from a count in the file. A count past
/// this is a file saying something impossible about itself, and allocating for
/// it is how a reader turns a bad number into a crash.
const MAX_ELEMENTS: usize = 1 << 22;

/// A value read out of a streamed object.
#[derive(Debug, Clone, PartialEq)]
pub enum Val {
    Int(i64),
    Float(f64),
    Str(String),
    /// A fixed-width array or a counted pointer array of whole numbers.
    Ints(Vec<i64>),
    Floats(Vec<f64>),
    Obj(Rc<Obj>),
    /// The elements of a `TObjArray` or a `TList`. A null element is kept as
    /// `None` so that the positions still line up with what the file said.
    List(Vec<Option<Rc<Obj>>>),
    /// A null pointer, which ROOT writes as a tag of zero.
    Null,
    /// The member is there and this does not read it. The text says why, and
    /// is what a reader is told rather than a silent gap.
    Unread(String),
}

impl Val {
    pub fn as_int(&self) -> Option<i64> {
        match self {
            Val::Int(v) => Some(*v),
            Val::Float(v) => Some(*v as i64),
            _ => None,
        }
    }

    pub fn as_float(&self) -> Option<f64> {
        match self {
            Val::Float(v) => Some(*v),
            Val::Int(v) => Some(*v as f64),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Val::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_ints(&self) -> Option<&[i64]> {
        match self {
            Val::Ints(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_list(&self) -> Option<&[Option<Rc<Obj>>]> {
        match self {
            Val::List(v) => Some(v),
            _ => None,
        }
    }
}

/// One streamed object: which class wrote it, which version of that class, and
/// its members in the order they were written.
///
/// A base class does not get a member of its own. Its members are merged in
/// where they stand, which is what the C++ object looks like from outside and
/// what makes `fName` a member of a `TBranch` even though `TNamed` wrote it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Obj {
    pub class: String,
    pub version: i32,
    pub members: Vec<(String, Val)>,
    /// What stopped the reading part way, if anything did. The members before
    /// it are still good: an object is stepped over by its own byte count.
    pub trouble: Option<String>,
}

impl Obj {
    pub fn get(&self, name: &str) -> Option<&Val> {
        self.members.iter().find(|(n, _)| n == name).map(|(_, v)| v)
    }

    pub fn int(&self, name: &str) -> Option<i64> {
        self.get(name).and_then(Val::as_int)
    }

    pub fn float(&self, name: &str) -> Option<f64> {
        self.get(name).and_then(Val::as_float)
    }

    pub fn str(&self, name: &str) -> &str {
        self.get(name).and_then(Val::as_str).unwrap_or("")
    }

    pub fn ints(&self, name: &str) -> &[i64] {
        self.get(name).and_then(Val::as_ints).unwrap_or(&[])
    }

    pub fn obj(&self, name: &str) -> Option<&Rc<Obj>> {
        match self.get(name) {
            Some(Val::Obj(o)) => Some(o),
            _ => None,
        }
    }

    pub fn list(&self, name: &str) -> &[Option<Rc<Obj>>] {
        self.get(name).and_then(Val::as_list).unwrap_or(&[])
    }
}

/// One member of a class, as a `TStreamerElement` describes it.
#[derive(Debug, Clone, PartialEq)]
pub struct Element {
    /// Which `TStreamerElement` subclass wrote this, which is what says
    /// whether the element is a base class, a plain number, a counted pointer
    /// or an STL container. `TStreamerBase` is the one that changes the shape
    /// of the reading rather than only its type.
    pub kind: String,
    pub name: String,
    pub title: String,
    /// The type code. 0 to 19 are the basic types; add 20 for a fixed array,
    /// 40 for a pointer counted by another member. 61 up are objects, strings
    /// and containers.
    pub etype: i32,
    /// How many bytes one of these takes in memory, which is not always how
    /// many it takes in the file.
    pub size: i32,
    /// How many elements a fixed array holds, zero for a single value.
    pub array_length: i32,
    /// How many dimensions that array has.
    pub array_dim: i32,
    /// The five dimensions, of which `array_dim` are used. A base class
    /// element puts the base class's checksum in the second of these rather
    /// than a dimension, which is why they are not read as sizes anywhere.
    pub max_index: [i32; 5],
    /// The C++ type as written: `double`, `TObjArray`, `long long*`.
    pub type_name: String,
    /// For a base class, the version of it that wrote.
    pub base_version: i32,
    /// For a counted pointer, the member of this class that says how many.
    pub count_name: String,
    pub count_class: String,
}

impl Element {
    /// Whether this element is a base class rather than a member of its own.
    pub fn is_base(&self) -> bool {
        self.kind == "TStreamerBase" || self.type_name == "BASE"
    }
}

/// One class description: everything needed to read an object of that class.
#[derive(Debug, Clone, PartialEq)]
pub struct StreamerInfo {
    pub class: String,
    pub title: String,
    pub version: i32,
    /// A number over the class's layout. Two versions of a class that hash the
    /// same are the same layout, which is how ROOT matches a class whose
    /// version was never bumped.
    pub checksum: u32,
    pub elements: Vec<Element>,
}

/// Every class description a file carries, which is the schema every other
/// object in it is read with.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Schema {
    pub infos: Vec<StreamerInfo>,
}

impl Schema {
    /// The description of `class` at `version`, or the only one there is when
    /// the version does not match. A file usually carries one version of each
    /// class, and an object whose version is not among them is likelier to be
    /// readable by the description that is there than by nothing.
    pub fn find(&self, class: &str, version: i32) -> Option<&StreamerInfo> {
        let mut only = None;
        for info in &self.infos {
            if info.class != class {
                continue;
            }
            if info.version == version {
                return Some(info);
            }
            only = match only {
                None => Some(info),
                Some(_) => return self.infos.iter().find(|i| i.class == class),
            };
        }
        only
    }

    pub fn has(&self, class: &str) -> bool {
        self.infos.iter().any(|i| i.class == class)
    }
}

/// What a reference in the class map stands for: a class name written out
/// earlier in the record, or an object read earlier in it.
#[derive(Debug, Clone)]
enum Seen {
    Class(String),
    Object(Rc<Obj>),
}

/// A position in the unpacked contents of one record, with the class map that
/// the back-references in it are resolved against.
pub struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
    /// What is added to a position here to get the position ROOT wrote into a
    /// back-reference. ROOT counts from the start of the key, and these bytes
    /// start after it, so this is the key's length.
    base: usize,
    seen: HashMap<u32, Seen>,
    /// The counter an old-style record's references are keyed by, for a record
    /// written without byte counts in front of its objects.
    counted: u32,
}

/// Anything that stopped the reading, as a sentence rather than a code. These
/// reach a reader, so they say what was met and not only that something was.
pub type Trouble = String;

type S<T> = Result<T, Trouble>;

impl<'a> Cursor<'a> {
    /// A cursor over `data`, whose back-references count from `base` bytes
    /// before the start of it. For a record that is the key's `fKeylen`.
    pub fn new(data: &'a [u8], base: usize) -> Self {
        Cursor { data, pos: 0, base, seen: HashMap::new(), counted: 0 }
    }

    pub fn at(&self) -> usize {
        self.pos
    }

    pub fn left(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    /// Where ROOT would have written this position into a reference.
    fn displacement(&self) -> u32 {
        (self.pos + self.base) as u32
    }

    fn seek(&mut self, to: usize) -> S<()> {
        if to > self.data.len() {
            return Err(format!("a length ran {} bytes past the end of the record", to - self.data.len()));
        }
        self.pos = to;
        Ok(())
    }

    fn take(&mut self, n: usize) -> S<&'a [u8]> {
        if self.left() < n {
            return Err(format!("{n} bytes wanted and {} left in the record", self.left()));
        }
        let out = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(out)
    }

    fn u8(&mut self) -> S<u8> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> S<u16> {
        let b = self.take(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }

    fn u32(&mut self) -> S<u32> {
        let b = self.take(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn i32(&mut self) -> S<i32> {
        Ok(self.u32()? as i32)
    }

    fn u64(&mut self) -> S<u64> {
        let b = self.take(8)?;
        Ok(u64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
    }

    /// A `TString`: one byte of length, or 255 and then four bytes of it.
    fn tstring(&mut self) -> S<String> {
        let first = self.u8()?;
        let len = match first {
            255 => self.u32()? as usize,
            n => n as usize,
        };
        if len > MAX_ELEMENTS {
            return Err(format!("a string of {len} bytes, which is longer than anything a record holds"));
        }
        Ok(String::from_utf8_lossy(self.take(len)?).into_owned())
    }

    /// A class name, which is written with a zero byte after it rather than a
    /// length in front of it.
    fn cstring(&mut self) -> S<String> {
        let rest = &self.data[self.pos..];
        let Some(end) = rest.iter().position(|&b| b == 0) else {
            return Err("a class name with no zero byte after it".into());
        };
        let out = String::from_utf8_lossy(&rest[..end]).into_owned();
        self.pos += end + 1;
        Ok(out)
    }

    /// The count and version an object opens with. The count comes back as
    /// what to add to the position of the count itself to step over the whole
    /// object, and is `None` for a class that writes no count.
    fn numbytes_version(&mut self) -> S<(Option<usize>, i32, bool)> {
        if self.left() < 6 {
            // Not enough for a count and a version; whatever is here is a bare
            // version, which is how a short object with no count reads.
            let v = self.u16()?;
            return Ok((None, i32::from(v & !MEMBERWISE), v & MEMBERWISE != 0));
        }
        let head = &self.data[self.pos..self.pos + 6];
        let count = u32::from_be_bytes([head[0], head[1], head[2], head[3]]);
        if count & BYTE_COUNT_MASK != 0 {
            self.pos += 6;
            let version = u16::from_be_bytes([head[4], head[5]]);
            let n = (count & !BYTE_COUNT_MASK) as usize + 4;
            Ok((Some(n), i32::from(version & !MEMBERWISE), version & MEMBERWISE != 0))
        } else {
            let version = self.u16()?;
            Ok((None, i32::from(version & !MEMBERWISE), version & MEMBERWISE != 0))
        }
    }

    fn remember(&mut self, key: u32, what: Seen, keyed: bool) {
        if keyed {
            self.seen.insert(key, what);
        } else {
            self.counted += 1;
            let n = self.counted;
            self.seen.insert(n, what);
        }
    }
}

/// Read whatever object is at the cursor, whichever class it turns out to be.
///
/// This is the one place a class name is discovered rather than known, and so
/// the one place the class map is written to. A tag of zero is a null pointer,
/// a tag with bit 31 clear is a reference to an object already read, and
/// anything else names a class: spelled out the first time, and as the
/// position of that spelling afterwards.
pub fn read_any(cur: &mut Cursor, schema: &Schema, depth: usize) -> S<Val> {
    if depth > MAX_DEPTH {
        return Err(format!("objects nested more than {MAX_DEPTH} deep"));
    }
    let beg = cur.displacement();
    let mut count = cur.u32()?;
    let (keyed, start, tag) = if count & BYTE_COUNT_MASK == 0 || count == NEW_CLASS_TAG {
        let tag = count;
        count = 0;
        (false, 0, tag)
    } else {
        let start = cur.displacement();
        (true, start, cur.u32()?)
    };

    if tag & CLASS_MASK == 0 {
        // A reference rather than a class: null, the object that holds this
        // one, or something read earlier in the record.
        if tag == 0 {
            return Ok(Val::Null);
        }
        if tag == 1 {
            return Ok(Val::Unread("a pointer back to the object that holds it".into()));
        }
        if let Some(Seen::Object(o)) = cur.seen.get(&tag) {
            return Ok(Val::Obj(o.clone()));
        }
        // A reference to something this reading never saw. The byte count says
        // where the object would have ended, so the reading carries on from
        // there rather than stopping.
        let skip = (count & !BYTE_COUNT_MASK) as usize;
        cur.seek(beg as usize - cur.base + skip + 4)?;
        return Ok(Val::Unread("a reference to an object written elsewhere".into()));
    }

    let class = if tag == NEW_CLASS_TAG {
        let name = cur.cstring()?;
        cur.remember(start + MAP_OFFSET, Seen::Class(name.clone()), keyed);
        name
    } else {
        let reference = tag & !CLASS_MASK;
        match cur.seen.get(&reference) {
            Some(Seen::Class(name)) => name.clone(),
            _ => return Err(format!("a class written back at position {reference}, which was never written")),
        }
    };

    let obj = Rc::new(read_class(cur, schema, &class, depth + 1)?);
    cur.remember(beg + MAP_OFFSET, Seen::Object(obj.clone()), keyed);
    Ok(Val::Obj(obj))
}

/// Read an object of a class that is already known: as a member of another
/// object, or as the whole of a record.
///
/// The classes ROOT streams by hand are here; everything else is read from its
/// `TStreamerInfo`. An object that opens with a byte count is stepped over by
/// that count whatever happened inside it, so one member this cannot read
/// costs that member and not the rest of the record.
pub fn read_class(cur: &mut Cursor, schema: &Schema, class: &str, depth: usize) -> S<Obj> {
    if depth > MAX_DEPTH {
        return Err(format!("objects nested more than {MAX_DEPTH} deep"));
    }
    // The three that write neither a count nor a version. Nothing may be
    // stepped over inside them, so a failure in one is a failure outright.
    match class {
        "TObject" => return read_tobject(cur),
        "TString" => {
            let text = cur.tstring()?;
            return Ok(Obj {
                class: "TString".into(),
                version: 0,
                members: vec![("fString".into(), Val::Str(text))],
                trouble: None,
            });
        }
        _ => {}
    }
    if let Some(width) = array_width(class) {
        return read_tarray(cur, class, width);
    }

    let opened = cur.at();
    let (count, version, memberwise) = cur.numbytes_version()?;
    let mut obj = Obj { class: class.to_string(), version, members: Vec::new(), trouble: None };
    if memberwise {
        obj.trouble = Some("written one member at a time across the collection, which is not read here".into());
    } else {
        let body = read_body(cur, schema, &mut obj, depth);
        if let Err(why) = body {
            // A count in front of the object is what makes this recoverable:
            // the reading goes on from the end of the object rather than from
            // wherever it stopped.
            if count.is_none() {
                return Err(why);
            }
            obj.trouble = Some(why);
        }
    }
    if let Some(n) = count {
        cur.seek(opened + n)?;
    }
    Ok(obj)
}

/// `TObject`: a version, two counters, and two places it can be longer than
/// that. It writes no byte count of its own, so every one of those has to be
/// read rather than skipped.
fn read_tobject(cur: &mut Cursor) -> S<Obj> {
    let version = cur.u16()?;
    if version & BYTE_COUNT_VMASK != 0 {
        cur.take(4)?;
    }
    let unique = cur.u32()?;
    let bits = cur.u32()?;
    if bits & IS_REFERENCED != 0 {
        cur.take(2)?;
    }
    Ok(Obj {
        class: "TObject".into(),
        version: i32::from(version & !BYTE_COUNT_VMASK),
        members: vec![
            ("fUniqueID".into(), Val::Int(i64::from(unique))),
            ("fBits".into(), Val::Int(i64::from(bits))),
        ],
        trouble: None,
    })
}

/// How wide one element of a `TArray` is, by the letter at the end of its
/// name. These are ROOT's own arrays of numbers, written as a count and then
/// the elements, with nothing in front of the count.
fn array_width(class: &str) -> Option<(usize, bool)> {
    // (bytes, is it a floating point number)
    Some(match class {
        "TArrayC" => (1, false),
        "TArrayS" => (2, false),
        "TArrayI" => (4, false),
        "TArrayL" => (8, false),
        "TArrayL64" => (8, false),
        "TArrayF" => (4, true),
        "TArrayD" => (8, true),
        _ => return None,
    })
}

fn read_tarray(cur: &mut Cursor, class: &str, (width, floating): (usize, bool)) -> S<Obj> {
    let n = cur.i32()?;
    if n < 0 || n as usize > MAX_ELEMENTS {
        return Err(format!("a {class} of {n} elements"));
    }
    let n = n as usize;
    let value = match floating {
        true => Val::Floats(read_floats(cur, width, n)?),
        false => Val::Ints(read_ints(cur, width, n, true)?),
    };
    Ok(Obj {
        class: class.to_string(),
        version: 0,
        members: vec![("fN".into(), Val::Int(n as i64)), ("fArray".into(), value)],
        trouble: None,
    })
}

fn read_ints(cur: &mut Cursor, width: usize, n: usize, signed: bool) -> S<Vec<i64>> {
    let mut out = Vec::with_capacity(n.min(4096));
    for _ in 0..n {
        let v = match (width, signed) {
            (1, true) => i64::from(cur.u8()? as i8),
            (1, false) => i64::from(cur.u8()?),
            (2, true) => i64::from(cur.u16()? as i16),
            (2, false) => i64::from(cur.u16()?),
            (4, true) => i64::from(cur.u32()? as i32),
            (4, false) => i64::from(cur.u32()?),
            (8, true) => cur.u64()? as i64,
            (8, false) => cur.u64()? as i64,
            _ => return Err(format!("a whole number {width} bytes wide")),
        };
        out.push(v);
    }
    Ok(out)
}

fn read_floats(cur: &mut Cursor, width: usize, n: usize) -> S<Vec<f64>> {
    let mut out = Vec::with_capacity(n.min(4096));
    for _ in 0..n {
        let v = match width {
            4 => f64::from(f32::from_bits(cur.u32()?)),
            8 => f64::from_bits(cur.u64()?),
            _ => return Err(format!("a floating point number {width} bytes wide")),
        };
        out.push(v);
    }
    Ok(out)
}

/// The members of one class, once its count and version have been read.
fn read_body(cur: &mut Cursor, schema: &Schema, obj: &mut Obj, depth: usize) -> S<()> {
    match obj.class.as_str() {
        "TNamed" => {
            let base = read_class(cur, schema, "TObject", depth + 1)?;
            obj.members.extend(base.members);
            obj.members.push(("fName".into(), Val::Str(cur.tstring()?)));
            obj.members.push(("fTitle".into(), Val::Str(cur.tstring()?)));
            return Ok(());
        }
        // The collections write their own streamers, which is not what their
        // `TStreamerInfo` describes: the description lists the members of the
        // C++ class and the streamer writes the elements.
        "TObjArray" => {
            let base = read_class(cur, schema, "TObject", depth + 1)?;
            obj.members.extend(base.members);
            obj.members.push(("fName".into(), Val::Str(cur.tstring()?)));
            let size = cur.i32()?;
            let lower = cur.i32()?;
            obj.members.push(("fSize".into(), Val::Int(i64::from(size))));
            obj.members.push(("fLowerBound".into(), Val::Int(i64::from(lower))));
            obj.members.push(("elements".into(), Val::List(read_elements(cur, schema, size, depth)?)));
            return Ok(());
        }
        "TList" | "THashList" => {
            let base = read_class(cur, schema, "TObject", depth + 1)?;
            obj.members.extend(base.members);
            obj.members.push(("fName".into(), Val::Str(cur.tstring()?)));
            let size = cur.i32()?;
            obj.members.push(("fSize".into(), Val::Int(i64::from(size))));
            if size < 0 || size as usize > MAX_ELEMENTS {
                return Err(format!("a list of {size} items"));
            }
            let mut items = Vec::with_capacity((size as usize).min(4096));
            for _ in 0..size {
                items.push(match read_any(cur, schema, depth + 1)? {
                    Val::Obj(o) => Some(o),
                    _ => None,
                });
                // Each item carries an option string after it, which nothing
                // in a file anyone writes has ever used.
                let n = cur.u8()? as usize;
                cur.take(n)?;
            }
            obj.members.push(("elements".into(), Val::List(items)));
            return Ok(());
        }
        _ => {}
    }

    let Some(info) = schema.find(&obj.class, obj.version) else {
        return Err(format!("no description of {} in the file's StreamerInfo", obj.class));
    };
    // Cloned because the elements are walked while the cursor is borrowed, and
    // a class description is a few dozen small structures.
    let elements = info.elements.clone();
    for element in &elements {
        read_element(cur, schema, obj, element, depth)?;
    }
    Ok(())
}

fn read_elements(cur: &mut Cursor, schema: &Schema, size: i32, depth: usize) -> S<Vec<Option<Rc<Obj>>>> {
    if size < 0 || size as usize > MAX_ELEMENTS {
        return Err(format!("an array of {size} objects"));
    }
    let mut out = Vec::with_capacity((size as usize).min(4096));
    for _ in 0..size {
        out.push(match read_any(cur, schema, depth + 1)? {
            Val::Obj(o) => Some(o),
            _ => None,
        });
    }
    Ok(out)
}

/// One member, by the type code its `TStreamerElement` gives it.
fn read_element(cur: &mut Cursor, schema: &Schema, obj: &mut Obj, el: &Element, depth: usize) -> S<()> {
    if el.is_base() {
        let base = read_class(cur, schema, &el.name, depth + 1)?;
        if let Some(why) = base.trouble {
            obj.trouble.get_or_insert(why);
        }
        obj.members.extend(base.members);
        return Ok(());
    }
    let code = el.etype;
    // 0 to 19 on their own are one value; add 20 and the member is a fixed
    // array of `fArrayLength`; add 40 and it is a pointer whose count is
    // another member of this class.
    let (basic, shape) = match code {
        0..=19 => (code, Shape::One),
        20..=39 => (code - 20, Shape::Fixed),
        40..=59 => (code - 40, Shape::Counted),
        _ => (-1, Shape::One),
    };
    if basic >= 0 {
        let value = read_basic(cur, obj, el, basic, shape)?;
        obj.members.push((el.name.clone(), value));
        return Ok(());
    }
    let value = match code {
        // An object written where it stands, with no pointer in front of it.
        61 | 62 | 63 | 68 => {
            let name = el.type_name.trim_end_matches('*').trim();
            Val::Obj(Rc::new(read_class(cur, schema, name, depth + 1)?))
        }
        // A pointer to an object, which may be null and may be a
        // back-reference to one already read.
        64 | 69 | 70 => read_any(cur, schema, depth + 1)?,
        65 => Val::Str(cur.tstring()?),
        _ => {
            return Err(format!(
                "member {} of {} has type code {code} ({}), which is not read here",
                el.name, obj.class, el.type_name
            ))
        }
    };
    obj.members.push((el.name.clone(), value));
    Ok(())
}

/// How many of a basic type a member holds and where the count comes from.
#[derive(Clone, Copy, PartialEq)]
enum Shape {
    One,
    /// As many as the element's own `fArrayLength`.
    Fixed,
    /// As many as the member named in `fCountName`, with a marker byte in
    /// front of them.
    Counted,
}

fn read_basic(cur: &mut Cursor, obj: &Obj, el: &Element, basic: i32, shape: Shape) -> S<Val> {
    // Code, width in the file, whether it is signed, whether it floats.
    let (width, signed, floating) = match basic {
        1 | 10 => (1, true, false),    // char, and the char of an older writer
        2 => (2, true, false),         // short
        3 | 6 => (4, true, false),     // int, and a counter
        4 | 16 => (8, true, false),    // long, long long
        5 | 19 => (4, false, true),    // float, and the half that is stored as one
        8 | 9 => (8, false, true),     // double, and the truncated double
        11 => (1, false, false),       // unsigned char
        12 => (2, false, false),       // unsigned short
        13 | 15 => (4, false, false),  // unsigned int, and a bit field
        14 | 17 => (8, false, false),  // unsigned long, unsigned long long
        18 => (1, false, false),       // bool
        _ => {
            return Err(format!(
                "member {} of {} is basic type {basic} ({}), which is not read here",
                el.name, obj.class, el.type_name
            ))
        }
    };
    let count = match shape {
        Shape::One => {
            return Ok(match floating {
                true => Val::Float(read_floats(cur, width, 1)?[0]),
                false => Val::Int(read_ints(cur, width, 1, signed)?[0]),
            })
        }
        Shape::Fixed => el.array_length.max(0) as usize,
        Shape::Counted => {
            // The marker byte ROOT writes in front of a pointer's elements.
            cur.u8()?;
            let Some(n) = obj.int(&el.count_name) else {
                return Err(format!(
                    "member {} of {} is counted by {}, which was not read",
                    el.name, obj.class, el.count_name
                ));
            };
            if n < 0 {
                return Err(format!("member {} of {} has a count of {n}", el.name, obj.class));
            }
            n as usize
        }
    };
    if count > MAX_ELEMENTS {
        return Err(format!("member {} of {} holds {count} elements", el.name, obj.class));
    }
    Ok(match floating {
        true => Val::Floats(read_floats(cur, width, count)?),
        false => Val::Ints(read_ints(cur, width, count, signed)?),
    })
}

/// One `TStreamerInfo` object, read as a class description.
fn streamer_info(obj: &Obj) -> StreamerInfo {
    let mut info = StreamerInfo {
        class: obj.str("fName").to_string(),
        title: obj.str("fTitle").to_string(),
        version: obj.int("fClassVersion").unwrap_or(0) as i32,
        checksum: obj.int("fCheckSum").unwrap_or(0) as u32,
        elements: Vec::new(),
    };
    let Some(elements) = obj.obj("fElements") else { return info };
    for element in elements.list("elements").iter().flatten() {
        info.elements.push(streamer_element(element));
    }
    info
}

fn streamer_element(obj: &Obj) -> Element {
    let mut max_index = [0i32; 5];
    for (i, v) in obj.ints("fMaxIndex").iter().take(5).enumerate() {
        max_index[i] = *v as i32;
    }
    Element {
        kind: obj.class.clone(),
        name: obj.str("fName").to_string(),
        title: obj.str("fTitle").to_string(),
        etype: obj.int("fType").unwrap_or(-1) as i32,
        size: obj.int("fSize").unwrap_or(0) as i32,
        array_length: obj.int("fArrayLength").unwrap_or(0) as i32,
        array_dim: obj.int("fArrayDim").unwrap_or(0) as i32,
        max_index,
        type_name: obj.str("fTypeName").to_string(),
        base_version: obj.int("fBaseVersion").unwrap_or(0) as i32,
        count_name: obj.str("fCountName").to_string(),
        count_class: obj.str("fCountClass").to_string(),
    }
}

/// `TStreamerInfo` and its element classes as they are written, for a file
/// whose descriptions are read before any schema exists.
///
/// These are the classes the schema itself is made of, so they cannot be read
/// from the schema. ROOT has not changed them since the format settled, and a
/// file that wrote them differently would say so in its own description of
/// them, which is in the same record and is decoded with this.
pub fn bootstrap() -> Schema {
    fn el(kind: &str, name: &str, etype: i32, size: i32, type_name: &str) -> Element {
        Element {
            kind: kind.into(),
            name: name.into(),
            title: String::new(),
            etype,
            size,
            array_length: 0,
            array_dim: 0,
            max_index: [0; 5],
            type_name: type_name.into(),
            base_version: 0,
            count_name: String::new(),
            count_class: String::new(),
        }
    }
    fn base(name: &str) -> Element {
        el("TStreamerBase", name, 0, 0, "BASE")
    }
    let mut infos = Vec::new();
    infos.push(StreamerInfo {
        class: "TStreamerInfo".into(),
        title: String::new(),
        version: 9,
        checksum: 0,
        elements: vec![
            base("TNamed"),
            el("TStreamerBasicType", "fCheckSum", 13, 4, "unsigned int"),
            el("TStreamerBasicType", "fClassVersion", 3, 4, "int"),
            el("TStreamerObjectPointer", "fElements", 64, 8, "TObjArray*"),
        ],
    });
    // Every TStreamerElement subclass writes its parent first and then its own
    // few members. The parent is the same for all of them. `fMaxIndex` is five
    // ints where it stands rather than a pointer, hence the type code of 20
    // plus int and the length beside it.
    let mut max_index = el("TStreamerBasicType", "fMaxIndex", 23, 20, "int");
    max_index.array_length = 5;
    max_index.array_dim = 1;
    let element_members = vec![
        base("TNamed"),
        el("TStreamerBasicType", "fType", 3, 4, "int"),
        el("TStreamerBasicType", "fSize", 3, 4, "int"),
        el("TStreamerBasicType", "fArrayLength", 3, 4, "int"),
        el("TStreamerBasicType", "fArrayDim", 3, 4, "int"),
        max_index,
        el("TStreamerString", "fTypeName", 65, 24, "TString"),
    ];
    infos.push(StreamerInfo {
        class: "TStreamerElement".into(),
        title: String::new(),
        version: 4,
        checksum: 0,
        elements: element_members,
    });
    for (class, extra) in [
        ("TStreamerBase", vec![el("TStreamerBasicType", "fBaseVersion", 3, 4, "int")]),
        (
            "TStreamerBasicPointer",
            vec![
                el("TStreamerBasicType", "fCountVersion", 3, 4, "int"),
                el("TStreamerString", "fCountName", 65, 24, "TString"),
                el("TStreamerString", "fCountClass", 65, 24, "TString"),
            ],
        ),
        (
            "TStreamerLoop",
            vec![
                el("TStreamerBasicType", "fCountVersion", 3, 4, "int"),
                el("TStreamerString", "fCountName", 65, 24, "TString"),
                el("TStreamerString", "fCountClass", 65, 24, "TString"),
            ],
        ),
        (
            "TStreamerSTL",
            vec![
                el("TStreamerBasicType", "fSTLtype", 3, 4, "int"),
                el("TStreamerBasicType", "fCtype", 3, 4, "int"),
            ],
        ),
        ("TStreamerBasicType", vec![]),
        ("TStreamerObject", vec![]),
        ("TStreamerObjectAny", vec![]),
        ("TStreamerObjectPointer", vec![]),
        ("TStreamerObjectAnyPointer", vec![]),
        ("TStreamerString", vec![]),
        ("TStreamerArtificial", vec![]),
        ("TStreamerSTLstring", vec![]),
    ] {
        let mut elements = vec![base(if class == "TStreamerSTLstring" { "TStreamerSTL" } else { "TStreamerElement" })];
        elements.extend(extra);
        infos.push(StreamerInfo { class: class.into(), title: String::new(), version: 2, checksum: 0, elements });
    }
    Schema { infos }
}

/// Read the `StreamerInfo` record with the hand-written descriptions above,
/// and then read it again with the descriptions it turned out to hold.
///
/// The second pass is not ceremony. A file may describe `TStreamerElement`
/// with members this does not expect, and the only place that description can
/// come from is the record itself. Where the second pass fails the first one's
/// answer stands, since a schema that read something is worth more than none.
pub fn read_streamer_info(data: &[u8], key_len: usize) -> S<Schema> {
    let mut cur = Cursor::new(data, key_len);
    let boot = bootstrap();
    let list = read_class(&mut cur, &boot, "TList", 0)?;
    let mut schema = Schema::default();
    for item in list.list("elements").iter().flatten() {
        if item.class == "TStreamerInfo" {
            schema.infos.push(streamer_info(item));
        }
    }
    Ok(schema)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `TObject` as ROOT writes one with nothing unusual set: a version and
    /// two counters.
    fn tobject() -> Vec<u8> {
        let mut v = 1u16.to_be_bytes().to_vec();
        v.extend_from_slice(&0u32.to_be_bytes());
        v.extend_from_slice(&0x0300_0000u32.to_be_bytes());
        v
    }

    fn tstring(s: &str) -> Vec<u8> {
        let mut v = vec![s.len() as u8];
        v.extend_from_slice(s.as_bytes());
        v
    }

    /// Wrap a body in the count and version an object opens with.
    fn framed(version: u16, body: Vec<u8>) -> Vec<u8> {
        let count = (body.len() + 2) as u32 | BYTE_COUNT_MASK;
        let mut v = count.to_be_bytes().to_vec();
        v.extend_from_slice(&version.to_be_bytes());
        v.extend(body);
        v
    }

    fn tnamed(name: &str, title: &str) -> Vec<u8> {
        let mut body = tobject();
        body.extend(tstring(name));
        body.extend(tstring(title));
        framed(1, body)
    }

    /// One `TStreamerBasicType` describing a plain `int` member.
    fn basic_element(name: &str, etype: i32, size: i32, type_name: &str) -> Vec<u8> {
        let mut inner = tnamed(name, "");
        inner.extend_from_slice(&etype.to_be_bytes());
        inner.extend_from_slice(&size.to_be_bytes());
        inner.extend_from_slice(&0i32.to_be_bytes()); // fArrayLength
        inner.extend_from_slice(&0i32.to_be_bytes()); // fArrayDim
        for _ in 0..5 {
            inner.extend_from_slice(&0i32.to_be_bytes()); // fMaxIndex
        }
        inner.extend(tstring(type_name));
        // TStreamerElement wraps that, and TStreamerBasicType wraps it again
        // with nothing of its own.
        framed(2, framed(4, inner))
    }

    #[test]
    fn a_streamer_element_says_its_name_type_and_width() {
        let bytes = basic_element("fEntries", 16, 8, "long long");
        let mut cur = Cursor::new(&bytes, 0);
        let obj = read_class(&mut cur, &bootstrap(), "TStreamerBasicType", 0).expect("read");
        let el = streamer_element(&obj);
        assert_eq!(el.name, "fEntries");
        assert_eq!(el.etype, 16);
        assert_eq!(el.size, 8);
        assert_eq!(el.type_name, "long long");
        assert!(!el.is_base());
        assert_eq!(cur.at(), bytes.len(), "the whole element was read");
    }

    #[test]
    fn a_fixed_array_element_keeps_its_length() {
        let mut inner = tnamed("fMaxIndex", "");
        inner.extend_from_slice(&23i32.to_be_bytes()); // fType: int plus 20
        inner.extend_from_slice(&20i32.to_be_bytes());
        inner.extend_from_slice(&5i32.to_be_bytes()); // fArrayLength
        inner.extend_from_slice(&1i32.to_be_bytes()); // fArrayDim
        for n in [5i32, 0, 0, 0, 0] {
            inner.extend_from_slice(&n.to_be_bytes());
        }
        inner.extend(tstring("int"));
        let bytes = framed(2, framed(4, inner));
        let mut cur = Cursor::new(&bytes, 0);
        let obj = read_class(&mut cur, &bootstrap(), "TStreamerBasicType", 0).expect("read");
        let el = streamer_element(&obj);
        assert_eq!(el.array_length, 5);
        assert_eq!(el.array_dim, 1);
        assert_eq!(el.max_index[0], 5);
    }

    /// The one that a reader which skipped the class map gets wrong: the
    /// second object of a class writes the position where the first one's name
    /// was, not the name again.
    #[test]
    fn the_second_object_of_a_class_is_named_by_where_the_first_was() {
        // A TObjArray holding two TNamed. The first spells the class out, the
        // second points back at it.
        let mut body = tobject();
        body.extend(tstring(""));
        body.extend_from_slice(&2i32.to_be_bytes()); // two elements
        body.extend_from_slice(&0i32.to_be_bytes()); // lower bound

        // The array's own frame is six bytes and `base` is zero here, so the
        // first element starts at 6 + the bytes above.
        let first_at = 6 + body.len();
        let first = tnamed("one", "");
        let mut element = ((first.len() + 4 + 4) as u32 | BYTE_COUNT_MASK).to_be_bytes().to_vec();
        element.extend_from_slice(&NEW_CLASS_TAG.to_be_bytes());
        element.extend_from_slice(b"TNamed\0");
        // The count above was worked out before the name was added, so rebuild
        // it now that the whole element is known.
        let payload = 4 + 7 + first.len();
        element = (payload as u32 | BYTE_COUNT_MASK).to_be_bytes().to_vec();
        element.extend_from_slice(&NEW_CLASS_TAG.to_be_bytes());
        element.extend_from_slice(b"TNamed\0");
        element.extend(first);
        body.extend(element);

        // The class name was written four bytes into the first element, and
        // two is added to every position before it is written as a reference.
        let reference = (first_at + 4) as u32 + MAP_OFFSET;
        let second = tnamed("two", "");
        let mut element = ((4 + second.len()) as u32 | BYTE_COUNT_MASK).to_be_bytes().to_vec();
        element.extend_from_slice(&(reference | CLASS_MASK).to_be_bytes());
        element.extend(second);
        body.extend(element);

        let bytes = framed(3, body);
        let mut cur = Cursor::new(&bytes, 0);
        let obj = read_class(&mut cur, &bootstrap(), "TObjArray", 0).expect("read");
        let items = obj.list("elements");
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].as_ref().expect("first").class, "TNamed");
        assert_eq!(items[0].as_ref().expect("first").str("fName"), "one");
        assert_eq!(items[1].as_ref().expect("second").class, "TNamed");
        assert_eq!(items[1].as_ref().expect("second").str("fName"), "two");
    }

    /// A member with a type code nothing here reads costs that member, not the
    /// rest of the object: the count in front of the object says where it
    /// ends.
    #[test]
    fn a_member_that_cannot_be_read_leaves_the_object_placed() {
        let mut schema = bootstrap();
        schema.infos.push(StreamerInfo {
            class: "Odd".into(),
            title: String::new(),
            version: 1,
            checksum: 0,
            elements: vec![
                Element {
                    kind: "TStreamerSTL".into(),
                    name: "fThing".into(),
                    title: String::new(),
                    etype: 500,
                    size: 24,
                    array_length: 0,
                    array_dim: 0,
                    max_index: [0; 5],
                    type_name: "vector<string>".into(),
                    base_version: 0,
                    count_name: String::new(),
                    count_class: String::new(),
                },
            ],
        });
        let bytes = framed(1, vec![0xAA; 10]);
        let mut cur = Cursor::new(&bytes, 0);
        let obj = read_class(&mut cur, &schema, "Odd", 0).expect("read");
        assert!(obj.trouble.as_deref().unwrap_or("").contains("500"));
        assert_eq!(cur.at(), bytes.len(), "stepped over by its own count");
    }

    #[test]
    fn a_counted_pointer_reads_as_many_as_its_count_says() {
        let mut schema = bootstrap();
        schema.infos.push(StreamerInfo {
            class: "Counted".into(),
            title: String::new(),
            version: 1,
            checksum: 0,
            elements: vec![
                Element {
                    kind: "TStreamerBasicType".into(),
                    name: "fN".into(),
                    title: String::new(),
                    etype: 6,
                    size: 4,
                    array_length: 0,
                    array_dim: 0,
                    max_index: [0; 5],
                    type_name: "int".into(),
                    base_version: 0,
                    count_name: String::new(),
                    count_class: String::new(),
                },
                Element {
                    kind: "TStreamerBasicPointer".into(),
                    name: "fSeek".into(),
                    title: String::new(),
                    etype: 56,
                    size: 8,
                    array_length: 0,
                    array_dim: 0,
                    max_index: [0; 5],
                    type_name: "long long*".into(),
                    base_version: 0,
                    count_name: "fN".into(),
                    count_class: "Counted".into(),
                },
            ],
        });
        let mut body = 3i32.to_be_bytes().to_vec();
        body.push(1); // the marker byte in front of a pointer's elements
        for v in [100i64, 200, 300] {
            body.extend_from_slice(&v.to_be_bytes());
        }
        let bytes = framed(1, body);
        let mut cur = Cursor::new(&bytes, 0);
        let obj = read_class(&mut cur, &schema, "Counted", 0).expect("read");
        assert_eq!(obj.int("fN"), Some(3));
        assert_eq!(obj.ints("fSeek"), &[100, 200, 300]);
        assert_eq!(obj.trouble, None);
    }
}
