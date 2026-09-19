//! Every word a row of a recognised pickle goes by, and the few bounds the
//! reading works under.
//!
//! Apart from [`pickleparts`](super::pickleparts) because they are two
//! different kinds of thing to get right: these are strings a reader sees, and
//! that is a walk over the tree. Written out once each, with what each one is
//! for, so that the word a reader compares two files by is decided in one
//! place and the arm that uses it says which word it means.

/// What the file holds: what matched, and then the object it matched.
pub(super) const HEADER_FIELD: &str = "header";
pub(super) const DATA_FIELD: &str = "data";
/// What the header says: the contract's sentence, the form that matched, and
/// the protocol the writer used.
pub(super) const MESSAGE_FIELD: &str = "message";
pub(super) const FORM_FIELD: &str = "form";
/// Which extensions of the basic grammar the file turned out to use, which
/// the form only
/// says for a file that holds one. A mixed file needs the row to say anything
/// at all, and every other file gets it for the same reason the `pickler` row
/// is always there: a reader comparing two files wants the same rows in both.
/// The first name is always `basic`, which is the grammar every form reads.
pub(super) const EXTENSIONS_FIELD: &str = "form extensions";
/// Which of CPython's two picklers wrote the file. Most files say nothing
/// either way, and the row says that rather than going missing: a reader
/// comparing two files wants to see the same rows in both.
pub(super) const PICKLER_FIELD: &str = "pickler";
pub(super) const PROTOCOL_FIELD: &str = "protocol";
/// What an array says about itself, and then the numbers themselves. Not
/// `data` again: the object the file holds is already called that, and one
/// name for two things is one thing a reader has to work out.
pub(super) const NUMBERS_FIELD: &str = "numbers";
/// What an array says about where its numbers are, for an array whose numbers
/// are a run some earlier array wrote.
pub(super) const WRITTEN_FIELD: &str = "written as";
/// What a protocol 0 line's own run is called, under the value it spells. The
/// row above says what the string is; this one is the bytes the file holds.
pub(super) const LINE_FIELD: &str = "line";
/// The run a whole number too wide to be read as one sits in, under the digits
/// it comes to. `line` where the file spelled the digits, which is what
/// protocols 0 and 1 write, and this where it wrote two's-complement bytes.
pub(super) const BYTES_FIELD: &str = "bytes";
/// The same row for an array whose numbers are a run some earlier array
/// wrote. Two arrays holding the same bytes are one byte string to Python, so
/// the second names that run rather than spelling it again. The run is outside
/// this array's own bytes, so it is said here and placed under the array that
/// wrote it.
pub(super) const EARLIER_RUN: &str = "bytes an earlier array wrote";
/// A masked array's mask, and what a masked entry stands for. NumPy's own
/// words: `mask` is the attribute, and `fill_value` is the property.
pub(super) const MASK_FIELD: &str = "mask";
pub(super) const FILL_FIELD: &str = "fill value";
pub(super) const DTYPE_FIELD: &str = "dtype";
pub(super) const SHAPE_FIELD: &str = "shape";
pub(super) const ORDER_FIELD: &str = "order";
pub(super) const KEY_FIELD: &str = "key";
pub(super) const VALUE_FIELD: &str = "value";
/// The two words STACK_GLOBAL joined to make a class's path. `class` is what
/// an object and a call name the class or callable they were made by.
pub(super) const MODULE_FIELD: &str = "module";
pub(super) const NAME_FIELD: &str = "name";
pub(super) const CLASS_FIELD: &str = "class";
/// What a BINGET says: the file wrote this value earlier and named it here
/// rather than writing it again. The row carries what is at the other end,
/// since the reference itself is two bytes that say nothing. A string or a
/// byte string is shown; a container is named and located, so the reader goes
/// to the bytes rather than being handed a copy of them.
pub(super) const REFERS_FIELD: &str = "refers to";
/// The storage orders, spelled the way NumPy spells them.
pub(super) const C_ORDER: &str = "C";
pub(super) const FORTRAN_ORDER: &str = "Fortran";
/// How a shape with no dimensions is written, which is NumPy's own spelling
/// for the shape of a single value.
pub(super) const NO_DIMENSIONS: &str = "()";
/// What a structured dtype's record is called, and what the bytes NumPy left
/// between two of its columns are called. The number after it is how far into
/// the record the run starts, so two runs of padding are told apart.
pub(super) const RECORD_NAME: &str = "record";
pub(super) const PADDING_FIELD: &str = "padding at";
/// What a tensor says about itself beyond its dtype and its shape: how far
/// apart two neighbouring values of each axis are, how far into the storage
/// its first value sits, which storage it is a window onto, which device that
/// storage was on, and whether gradients are kept for it.
pub(super) const STRIDE_FIELD: &str = "stride";
pub(super) const STORAGE_OFFSET_FIELD: &str = "storage offset";
pub(super) const STORAGE_FIELD: &str = "storage";
pub(super) const LOCATION_FIELD: &str = "location";
pub(super) const REQUIRES_GRAD_FIELD: &str = "requires grad";
/// What a quantised tensor's stored whole numbers stand for. Said only of one,
/// because every other tensor's numbers are what they say.
pub(super) const SCALE_FIELD: &str = "scale";
pub(super) const ZERO_POINT_FIELD: &str = "zero point";
/// What a tensor is, for the row that says so, when the reader wants the word
/// rather than the shape. The entry row over it says both.
pub(super) const IS_FIELD: &str = "is";
/// What a container was given beyond what is in it, which is what
/// `nn.Module.state_dict()` hangs its `_metadata` off.
pub(super) const ATTRIBUTES_FIELD: &str = "attributes";
/// The whole pickle `joblib.dump` writes in place of an array's numbers when
/// the array holds pickled objects. The values are inside it, and so is a
/// protocol of its own.
pub(super) const NESTED_FIELD: &str = "nested pickle";

/// The fewest dictionaries that make a list of records. One dictionary is a
/// record, not a list of them.
pub(super) const FEWEST_ROWS: usize = 2;
/// What one dictionary of such a list is, which is what the table calls its
/// rows. Not "field": a pickled list's children are instructions as well as
/// dictionaries, and counting those as rows counts neither.
pub(super) const ROW_WORD: &str = "row";

/// The largest file a form is run over. The recogniser reads the whole
/// document at once, the same limit the deduced readings work under.
pub(super) const MOST_BYTES: u64 = 256 << 20;
