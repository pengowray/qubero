//! Where a computed table's cells are, checked against the bytes at the
//! address each one gives.
//!
//! Three kinds of table are read by the core rather than laid out as a run of
//! fields: a pandas frame, a torch tensor, and the summary over a checkpoint's
//! state dict. None of them has a row that is a run of bytes, so a cell rather
//! than a row is what carries an address, and what is checked here is that the
//! address is the right one: the bytes it names are read back and compared
//! with what the cell says.
//!
//! The rest of what those tables hold is checked in `pickle_real.rs` and
//! `torch_real.rs`. This file is only the addresses.
//!
//! Like the rest of the collection this skips when the folder is not beside
//! the repository. Point `QUBERO_SAMPLES` at it to run it elsewhere.

use std::path::PathBuf;

use qubero_core::document::Document;
use qubero_core::eval::{CellAt, Evaluator, FrameCell, Value};
use qubero_core::formats;
use qubero_core::source::MemSource;

/// The run of bytes a cell points at, out of the file or out of the space the
/// file spelled its numbers into. Panics on a cell with no address, which is
/// what every caller of this is claiming it has.
fn bytes_at(ev: &Evaluator, file: &[u8], cell: &FrameCell, where_: &str) -> Vec<u8> {
    let CellAt::Bytes { space, offset_bits, size_bits } = cell.at else {
        panic!("{where_}: no address, {:?}", cell.at);
    };
    assert_eq!(offset_bits % 8, 0, "{where_}: does not start on a byte");
    let (at, len) = ((offset_bits / 8) as usize, (size_bits / 8) as usize);
    let held: &[u8] = match space {
        0 => file,
        id => ev.space_bytes(id).unwrap_or_else(|| panic!("{where_}: space {id} is not open")),
    };
    held.get(at..at + len).unwrap_or_else(|| panic!("{where_}: {at}+{len} is past the end")).to_vec()
}

/// Where a cell is, as the pair the assertions below compare.
fn placed(cell: &FrameCell) -> (u32, u64, u64) {
    match cell.at {
        CellAt::Bytes { space, offset_bits, size_bits } => (space, offset_bits, size_bits),
        other => panic!("no address: {other:?}"),
    }
}

fn f64_at(bytes: &[u8]) -> f64 {
    f64::from_le_bytes(bytes.try_into().expect("eight bytes"))
}

fn f32_at(bytes: &[u8]) -> f32 {
    f32::from_le_bytes(bytes.try_into().expect("four bytes"))
}

fn i64_at(bytes: &[u8]) -> i64 {
    i64::from_le_bytes(bytes.try_into().expect("eight bytes"))
}

/// Whether a run of instructions spells `said`, which is what a cell over a
/// pickled value points at: the opcode, the length it carries and the letters,
/// and whatever the pickler wrote to remember it afterwards.
fn spells(run: &[u8], said: &str) -> bool {
    let want = said.as_bytes();
    run.len() >= want.len() && run.windows(want.len()).any(|w| w == want)
}

/// Whether a run is one of the opcodes that name a value written earlier.
fn names(run: &[u8]) -> bool {
    matches!(run.first(), Some(b'h' | b'j' | b'g'))
}

fn text_of(cell: &FrameCell) -> String {
    match &cell.value {
        Some(Value::Str(s)) => s.clone(),
        Some(Value::Int(n)) => n.to_string(),
        Some(Value::UInt(n)) => n.to_string(),
        Some(Value::Float(f)) => f.to_string(),
        other => panic!("{other:?}"),
    }
}

/// Every environment's copy of one of the matrix's objects, at one protocol.
fn matrix(stem: &str, protocol: &str) -> Vec<(String, Vec<u8>)> {
    let Some(root) = qubero_samples::dir("pickle-matrix") else { return Vec::new() };
    let mut out = Vec::new();
    for dir in std::fs::read_dir(&root).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.is_dir()) {
        let path = dir.join(format!("{stem}.{protocol}.pickle"));
        let Ok(bytes) = std::fs::read(&path) else { continue };
        if formats::pickle::familiar::recognise(&bytes).is_none() {
            continue;
        }
        out.push((format!("{}/{stem}.{protocol}", dir.file_name().unwrap().to_string_lossy()), bytes));
    }
    out
}

/// A frame written at protocol 4 keeps its numbers as bytes, so every cell
/// that has an address has one in the file, and the bytes there are the value
/// the cell shows.
///
/// `dataframe-mixed` is four rows of a counted index, an `int64` column, a
/// `float64` column and a column of strings: one cell of each kind the frame
/// reader knows, in one row.
#[test]
fn a_frame_s_cells_are_where_the_file_holds_their_bytes() {
    let files = matrix("dataframe-mixed", "p4");
    if files.is_empty() {
        eprintln!("{}", qubero_samples::missing());
        return;
    }
    for (where_, bytes) in &files {
        let doc = Document::new(MemSource(bytes.clone()));
        let mut ev = Evaluator::new(formats::builtin("picklefpf").unwrap());
        assert!(ev.table_shape(&doc, &[1]).unwrap().is_some(), "{where_}: no table");
        let rows = ev.pickle_cells(&doc, &[1], 0, 4).unwrap();
        assert_eq!(rows.len(), 4, "{where_}");
        for (i, row) in rows.iter().enumerate() {
            // The index is counted from a start and a step and is nowhere in
            // the file, which the cell says rather than pointing at byte 0.
            assert_eq!(row[0].at, CellAt::Counted, "{where_} row {i} index");
            assert_eq!(text_of(&row[0]), i.to_string(), "{where_} row {i} index");
            // `id` is the row number and `score` is one and a half more, both
            // read back out of the bytes the cell names.
            let (space, _, size) = placed(&row[1]);
            assert_eq!((space, size), (0, 64), "{where_} row {i} id");
            assert_eq!(i64_at(&bytes_at(&ev, bytes, &row[1], where_)), i as i64, "{where_} row {i} id");
            assert_eq!(f64_at(&bytes_at(&ev, bytes, &row[2], where_)), i as f64 + 1.5, "{where_} row {i} score");
            // A column of objects is a run of pickled values, and the cell
            // covers the whole instruction: the opcode as well as the letter
            // it writes.
            let said = bytes_at(&ev, bytes, &row[3], where_);
            assert!(spells(&said, &text_of(&row[3])), "{where_} row {i} name: {said:?}");
        }
        // A block holds one column's rows together, so going down a column is
        // a step of one value and going along a row is not.
        let (_, first, _) = placed(&rows[0][1]);
        let (_, next, _) = placed(&rows[1][1]);
        assert_eq!(next - first, 64, "{where_}: the rows of a column are not a value apart");
        let (_, score, _) = placed(&rows[0][2]);
        assert_ne!(score, first + 64, "{where_}: two columns of one row are not next to each other");
    }
}

/// The same frame at protocol 2, where the numbers were never written as
/// bytes: they are the latin-1 text that spells them, and a cell's address is
/// a run of the space that text opens rather than of the file.
///
/// One row is the whole claim. `id` and `score` are in the space; `name` is a
/// pickled string and is still in the file, so the two cells beside each other
/// are addresses in two different spaces, which is why an address on the row
/// would not have done.
#[test]
fn a_spelled_frame_s_numbers_are_addressed_in_the_space_that_holds_them() {
    let files = matrix("dataframe-mixed", "p2");
    if files.is_empty() {
        eprintln!("{}", qubero_samples::missing());
        return;
    }
    for (where_, bytes) in &files {
        let doc = Document::new(MemSource(bytes.clone()));
        let mut ev = Evaluator::new(formats::builtin("picklefpf").unwrap());
        assert!(ev.table_shape(&doc, &[1]).unwrap().is_some(), "{where_}: no table");
        let rows = ev.pickle_cells(&doc, &[1], 0, 4).unwrap();
        for (i, row) in rows.iter().enumerate() {
            let (id_space, id_at, id_size) = placed(&row[1]);
            assert_ne!(id_space, 0, "{where_} row {i} id: protocol 2 writes no bytes");
            assert_eq!(id_size, 64, "{where_} row {i} id");
            // The run starts at the front of the space it opened, so row `i`
            // of the first column is `i` values in.
            assert_eq!(id_at, i as u64 * 64, "{where_} row {i} id");
            assert_eq!(i64_at(&bytes_at(&ev, bytes, &row[1], where_)), i as i64, "{where_} row {i} id");
            let (score_space, ..) = placed(&row[2]);
            assert_ne!(score_space, id_space, "{where_} row {i}: two columns are two blocks and two spaces");
            assert_eq!(f64_at(&bytes_at(&ev, bytes, &row[2], where_)), i as f64 + 1.5, "{where_} row {i} score");
            // The strings were pickled whether or not the numbers were.
            let (name_space, ..) = placed(&row[3]);
            assert_eq!(name_space, 0, "{where_} row {i} name: a pickled string is in the file");
            // A string the pickler had already written is named out of the
            // memo rather than spelled again, and the cell points at the
            // instruction that names it: that run is where the file says what
            // this cell holds.
            let said = bytes_at(&ev, bytes, &row[3], where_);
            assert!(spells(&said, &text_of(&row[3])) || names(&said), "{where_} row {i} name: {said:?}");
        }
    }
}

/// A masked array's table: the numbers, with the entries the mask hides shown
/// empty and still carrying the address of the bytes they came from.
///
/// The four samples in `pickle/` are the four things a mask can be: two of
/// four hidden, the same over two dimensions, nothing hidden, and an int array
/// with a fill value of its own. Each is written at protocol 4, where the
/// numbers are bytes of the file, and at protocol 2, where they are the
/// latin-1 text that spells them and every cell's address is a run of the
/// space that text opens.
#[test]
fn a_masked_array_s_hidden_cells_are_empty_and_still_say_where_they_are() {
    let Some(dir) = qubero_samples::dir("pickle") else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    // The file, how the table comes out row by row, and how wide the values
    // are. `None` is an entry the mask hides.
    let want: &[(&str, &[&[Option<f64>]], u64)] = &[
        ("numpy-masked-array", &[&[Some(1.0)], &[None], &[Some(3.0)], &[None]], 64),
        ("numpy-masked-2d", &[&[Some(0.0), Some(1.0), None], &[None, Some(4.0), Some(5.0)]], 64),
        ("numpy-masked-unmasked", &[&[Some(1.5)], &[Some(2.5)], &[Some(3.5)]], 64),
        ("numpy-masked-fill-value", &[&[None], &[Some(2.0)], &[Some(3.0)], &[None]], 32),
    ];
    for protocol in ["proto4", "proto2"] {
        for (stem, rows, width) in want {
            let where_ = format!("{protocol}-{stem}");
            let bytes = std::fs::read(dir.join(format!("{where_}.pickle"))).unwrap_or_else(|e| panic!("{where_}: {e}"));
            let doc = Document::new(MemSource(bytes.clone()));
            let mut ev = Evaluator::new(formats::builtin("picklefpf").unwrap());
            let shape = ev.table_shape(&doc, &[1]).unwrap().unwrap_or_else(|| panic!("{where_}: no table"));
            assert!(shape.cells.is_some(), "{where_}: the cells are worked out");
            let held = ev.pickle_cells(&doc, &[1], 0, rows.len() as u64 + 1).unwrap();
            assert_eq!(held.len(), rows.len(), "{where_}: rows");
            for (i, (row, said)) in held.iter().zip(rows.iter()).enumerate() {
                assert_eq!(row.len(), said.len(), "{where_} row {i}: columns");
                for (j, (cell, want)) in row.iter().zip(said.iter()).enumerate() {
                    // Masked or not, the cell says where its bytes are, and
                    // the number stored under it is still readable there.
                    let (space, _, size) = placed(cell);
                    assert_eq!(size, *width, "{where_} row {i} column {j}");
                    let run = bytes_at(&ev, &bytes, cell, &where_);
                    let stored = match width {
                        64 => f64_at(&run),
                        _ => i32::from_le_bytes(run[..].try_into().unwrap()) as f64,
                    };
                    match want {
                        Some(number) => {
                            assert!(!cell.masked, "{where_} row {i} column {j}: not masked");
                            assert_eq!(text_of(cell), number.to_string(), "{where_} row {i} column {j}");
                            assert_eq!(stored, *number, "{where_} row {i} column {j}: the bytes the cell names");
                        }
                        None => {
                            assert!(cell.masked, "{where_} row {i} column {j}: masked");
                            assert_eq!(cell.value, None, "{where_} row {i} column {j}: a masked cell shows nothing");
                            assert!(stored.is_finite(), "{where_} row {i} column {j}: the stored number is still there");
                        }
                    }
                    // Protocol 2 writes no numbers at all: they are the
                    // latin-1 text that spells them, and the address is a run
                    // of the space that text opened.
                    match protocol {
                        "proto2" => assert_ne!(space, 0, "{where_} row {i} column {j}"),
                        _ => assert_eq!(space, 0, "{where_} row {i} column {j}"),
                    }
                }
            }
        }
    }
}

/// The masked arrays a real file holds. `GridSearchCV.cv_results_` is a
/// dictionary of them, one per thing the search recorded, and they are the
/// reason the production was written.
#[test]
fn a_fitted_search_s_masked_arrays_read_as_their_numbers() {
    let Some(dir) = qubero_samples::dir("pickle") else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    let bytes = std::fs::read(dir.join("sklearn-grid-search-cv.pickle")).unwrap();
    let doc = Document::new(MemSource(bytes.clone()));
    let mut ev = Evaluator::new(formats::builtin("picklefpf").unwrap());
    let found = typed(&doc, &mut ev, &[], "masked array", 0);
    assert!(!found.is_empty(), "no masked array in a search whose cv_results_ is made of them");
    for at in &found {
        let shape = ev.table_shape(&doc, at).unwrap().unwrap_or_else(|| panic!("{at:?}: no table"));
        let rows = shape.cells.as_ref().map(|c| match c {
            qubero_core::template::Cells::Computed { rows } => *rows,
            other => panic!("{at:?}: {other:?}"),
        });
        let rows = rows.unwrap_or_else(|| panic!("{at:?}: the cells are not worked out"));
        assert!(rows > 0, "{at:?}: no rows");
        let held = ev.pickle_cells(&doc, at, 0, rows).unwrap();
        assert_eq!(held.len() as u64, rows, "{at:?}");
        for (i, row) in held.iter().enumerate() {
            for (j, cell) in row.iter().enumerate() {
                // Whether the mask hides it or not, every cell of this file
                // says where its bytes are, in the file itself: the search was
                // pickled at a protocol with an opcode for a run of bytes.
                let (space, _, size) = placed(cell);
                assert_eq!(space, 0, "{at:?} row {i} column {j}");
                assert_eq!(bytes_at(&ev, &bytes, cell, "grid search").len() as u64, size / 8, "{at:?} row {i} column {j}");
                assert_eq!(cell.value.is_none(), cell.masked, "{at:?} row {i} column {j}: a cell shows nothing exactly when it is masked");
            }
        }
    }
}

/// Every node of this type, by path, as far down as the walk goes.
fn typed(doc: &Document<MemSource>, ev: &mut Evaluator, at: &[usize], want: &str, depth: usize) -> Vec<Vec<usize>> {
    if depth > 24 {
        return Vec::new();
    }
    let Ok(node) = ev.node(doc, at) else { return Vec::new() };
    if node.type_name == want {
        return vec![at.to_vec()];
    }
    let mut out = Vec::new();
    for i in 0..node.child_count as usize {
        let mut next = at.to_vec();
        next.push(i);
        out.extend(typed(doc, ev, &next, want, depth + 1));
    }
    out
}

/// A categorical cell is a code in one run naming a category in another, and
/// the address it carries is the code's: that byte is what this row holds.
#[test]
fn a_categorical_cell_is_at_its_code() {
    let files = matrix("series-categorical", "p4");
    if files.is_empty() {
        eprintln!("{}", qubero_samples::missing());
        return;
    }
    for (where_, bytes) in &files {
        let doc = Document::new(MemSource(bytes.clone()));
        let mut ev = Evaluator::new(formats::builtin("picklefpf").unwrap());
        assert!(ev.table_shape(&doc, &[1]).unwrap().is_some(), "{where_}: no table");
        let rows = ev.pickle_cells(&doc, &[1], 0, 4).unwrap();
        // `lo hi lo mid`, as codes into the three categories. The codes are a
        // run of one byte each, so the four cells are four bytes in a row.
        let said: Vec<String> = rows.iter().map(|row| text_of(&row[1])).collect();
        assert_eq!(said, ["lo", "hi", "lo", "mid"], "{where_}");
        let codes: Vec<u8> = rows.iter().map(|row| bytes_at(&ev, bytes, &row[1], where_)[0]).collect();
        assert_eq!(codes[0], codes[2], "{where_}: the same category is the same code");
        assert_ne!(codes[0], codes[1], "{where_}");
        let (_, first, size) = placed(&rows[0][1]);
        assert_eq!(size, 8, "{where_}: a code of three categories is one byte");
        for (i, row) in rows.iter().enumerate() {
            assert_eq!(placed(&row[1]).1, first + i as u64 * 8, "{where_} row {i}");
        }
    }
}

fn placed_at(cell: &FrameCell) -> [u64; 3] {
    let (space, at, size) = placed(cell);
    [u64::from(space), at, size]
}

fn torch() -> Option<PathBuf> {
    let dir = qubero_samples::root()?.join("torch");
    dir.is_dir().then_some(dir)
}

fn open_torch(dir: &PathBuf, name: &str, template: &str) -> (Document<MemSource>, Evaluator, Vec<u8>) {
    let bytes = std::fs::read(dir.join(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
    (Document::new(MemSource(bytes.clone())), Evaluator::new(formats::builtin(template).unwrap()), bytes)
}

/// The node at `name` under `at`, walked by asking each node what its children
/// are called. The parts are separated by a slash, because a state dict's own
/// keys hold dots.
fn under(doc: &Document<MemSource>, ev: &mut Evaluator, at: &[usize], name: &str) -> Vec<usize> {
    let mut here = at.to_vec();
    for part in name.split('/') {
        let node = ev.node(doc, &here).unwrap_or_else(|e| panic!("{name} at {here:?}: {e:?}"));
        let mut found = None;
        for i in 0..node.child_count as usize {
            let mut next = here.clone();
            next.push(i);
            if ev.node(doc, &next).map(|n| n.name == part).unwrap_or(false) {
                found = Some(i);
                break;
            }
        }
        here.push(found.unwrap_or_else(|| panic!("no {part} under {here:?} for {name}")));
    }
    here
}

/// The state dict inside this container's data pickle, which is the node the
/// summary table is of and the tensors hang under.
fn dict_of(doc: &Document<MemSource>, ev: &mut Evaluator, template: &str) -> Vec<usize> {
    let held = match template {
        "torchzip" => {
            let mut here = under(doc, ev, &[], "checkpoint");
            here.push(0);
            under(doc, ev, &here, "data")
        }
        _ => under(doc, ev, &[], "data/data"),
    };
    under(doc, ev, &held, "data")
}

/// A tensor's cell is at the entry's data, the tensor's offset into its
/// storage, and one step of the stride for each axis.
///
/// `layer.weight` is `arange(12).reshape(3, 4)` as float32, stored at 0x380,
/// so `[1][2]` is element six, twenty-four bytes in, and reads 6.
#[test]
fn a_tensor_s_cell_is_at_the_stride_from_the_entry_s_data() {
    let Some(dir) = torch() else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    let (doc, mut ev, bytes) = open_torch(&dir, "state-dict-zip.pt", "torchzip");
    let data = dict_of(&doc, &mut ev, "torchzip");
    let weight = under(&doc, &mut ev, &data, "layer.weight/value");
    let rows = ev.pickle_cells(&doc, &weight, 0, 3).unwrap();
    assert_eq!(placed_at(&rows[1][2]), [0, (0x380 + 6 * 4) * 8, 32]);
    assert_eq!(f32_at(&bytes_at(&ev, &bytes, &rows[1][2], "layer.weight[1][2]")), 6.0);
    // The whole table, cell for cell: element `row * 4 + column`, each four
    // bytes on from the one before it.
    for (r, row) in rows.iter().enumerate() {
        for (c, cell) in row.iter().enumerate() {
            let elem = (r * 4 + c) as u64;
            assert_eq!(placed_at(cell), [0, (0x380 + elem * 4) * 8, 32], "[{r}][{c}]");
            assert_eq!(f32_at(&bytes_at(&ev, &bytes, cell, "layer.weight")), elem as f32, "[{r}][{c}]");
        }
    }
}

/// A transposed tensor is the same storage read the other way round, and the
/// addresses follow the strides rather than the table.
///
/// `grid` is `arange(24, float32).reshape(4, 6).t()`: six rows of four, stride
/// 1 down and 6 along, so going along a row of the table is a step of six
/// values through the file.
#[test]
fn a_transposed_tensor_s_cells_follow_its_strides() {
    let Some(dir) = torch() else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    let (doc, mut ev, bytes) = open_torch(&dir, "shared-storage-views-zip.pt", "torchzip");
    let data = dict_of(&doc, &mut ev, "torchzip");
    // One storage behind all three tensors, so the whole one says where it
    // begins and the views are read against that.
    let whole = under(&doc, &mut ev, &data, "whole/value");
    let (_, start, _) = placed(&ev.pickle_cells(&doc, &whole, 0, 1).unwrap()[0][0]);
    let grid = under(&doc, &mut ev, &data, "grid/value");
    let rows = ev.pickle_cells(&doc, &grid, 0, 6).unwrap();
    // Element `row + column * 6`, which for [1][2] is thirteen.
    assert_eq!(placed_at(&rows[1][2]), [0, start + 13 * 32, 32]);
    assert_eq!(f32_at(&bytes_at(&ev, &bytes, &rows[1][2], "grid[1][2]")), 13.0);
    for (r, row) in rows.iter().enumerate() {
        for (c, cell) in row.iter().enumerate() {
            let elem = (r + c * 6) as u64;
            assert_eq!(placed_at(cell), [0, start + elem * 32, 32], "[{r}][{c}]");
        }
    }
    // A view further into the same storage starts where its offset says.
    let tail = under(&doc, &mut ev, &data, "tail/value");
    let tails = ev.pickle_cells(&doc, &tail, 0, 1).unwrap();
    assert_eq!(placed_at(&tails[0][0]), [0, start + 12 * 32, 32]);
}

/// A legacy checkpoint keeps its storages in the file rather than in an
/// archive entry, and a cell is addressed the same way: inside the run the
/// storage's own field covers.
#[test]
fn a_legacy_tensor_s_cells_are_inside_the_storage_s_field() {
    let Some(dir) = torch() else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    let (doc, mut ev, bytes) = open_torch(&dir, "state-dict-legacy.pt", "torchlegacy");
    let data = dict_of(&doc, &mut ev, "torchlegacy");
    let weight = under(&doc, &mut ev, &data, "layer.weight/value");
    let rows = ev.pickle_cells(&doc, &weight, 0, 3).unwrap();
    let (_, start, _) = placed(&rows[0][0]);
    for (r, row) in rows.iter().enumerate() {
        for (c, cell) in row.iter().enumerate() {
            let elem = (r * 4 + c) as u64;
            assert_eq!(placed_at(cell), [0, start + elem * 32, 32], "[{r}][{c}]");
            assert_eq!(f32_at(&bytes_at(&ev, &bytes, cell, "layer.weight")), elem as f32, "[{r}][{c}]");
        }
    }
    // The storage is a field of the file here, so the cells sit inside the
    // run of numbers the storage's own field covers. One of the file's
    // storages holds them, and the cells are inside that one and no other.
    let (_, last, size) = placed(&rows[2][3]);
    let held = runs_of(&doc, &mut ev);
    let inside: Vec<&(String, u64, u64)> = held.iter().filter(|(_, at, len)| *at <= start && last + size <= at + len).collect();
    assert_eq!(inside.len(), 1, "the cells are inside {} of {} storages: {held:?}", inside.len(), held.len());
    assert_eq!(inside[0].0, "f32 le[]", "{:?}", inside[0]);
}

/// Every storage of a legacy checkpoint as the tree places it: what one value
/// of it is, where the run starts and how long it is.
///
/// A storage is a count and then its numbers, both placed by the pickle that
/// named them, so the run is a step below the field: the field says where to
/// look and the node under it is the numbers.
fn runs_of(doc: &Document<MemSource>, ev: &mut Evaluator) -> Vec<(String, u64, u64)> {
    let root = ev.node(doc, &[]).unwrap();
    let mut out = Vec::new();
    for i in 0..root.child_count as usize {
        if ev.node(doc, &[i]).map(|n| n.type_name != "storage").unwrap_or(true) {
            continue;
        }
        let at = under(doc, ev, &[i], "numbers");
        let run = ev.node(doc, &[at.as_slice(), &[0]].concat()).unwrap();
        out.push((run.type_name.clone(), run.offset_bits, run.size_bits));
    }
    out
}

/// The summary over a checkpoint's state dict is about tensors rather than
/// about bytes, and only two of its four columns are anywhere.
///
/// The name is the instructions that spell it, in the pickle; the count of
/// values is the numbers it counts, which is the tensor's own window in the
/// file. The dtype and the shape are said by the instructions that rebuild the
/// tensor rather than written as a value of their own, so those cells say so.
#[test]
fn a_checkpoint_summary_points_at_the_name_and_at_the_numbers() {
    let Some(dir) = torch() else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    let (doc, mut ev, bytes) = open_torch(&dir, "state-dict-zip.pt", "torchzip");
    let data = dict_of(&doc, &mut ev, "torchzip");
    let rows = ev.pickle_cells(&doc, &data, 0, 3).unwrap();
    let said: Vec<String> = rows[0].iter().map(text_of).collect();
    assert_eq!(said, ["layer.weight", "float32", "3 x 4", "12"]);
    // The name, spelled where the cell says it is.
    let name = bytes_at(&ev, &bytes, &rows[0][0], "name");
    assert!(spells(&name, "layer.weight"), "{name:?}");
    // The dtype is where the pickle names what one element is, which for a
    // typed storage is its class, and the shape is the size tuple: both are
    // written in the pickle, so both cells point at what they were read from.
    let class = bytes_at(&ev, &bytes, &rows[0][1], "dtype");
    assert!(spells(&class, "FloatStorage"), "{class:?}");
    let size = bytes_at(&ev, &bytes, &rows[0][2], "shape");
    assert!(size.windows(4).any(|w| w == b"K\x03K\x04"), "the size tuple is BININT1 3, BININT1 4: {size:?}");
    // The numbers: the tensor's whole window, which for `layer.weight` is
    // twelve float32 at the entry's data.
    assert_eq!(placed_at(&rows[0][3]), [0, 0x380 * 8, 12 * 32]);
    let held = bytes_at(&ev, &bytes, &rows[0][3], "values");
    assert_eq!(f32_at(&held[..4]), 0.0);
    assert_eq!(f32_at(&held[44..]), 11.0);
}


/// A masked array of a structured dtype: a row is the named columns of one
/// record, and the mask hides a column of a row rather than an entry of a run.
///
/// The two runs have different widths. A record of an `i4` and an `f8` is
/// sixteen bytes with the second column eight in; its mask is two bytes, one
/// boolean a column with nothing between them, which is what `make_mask_descr`
/// builds. So a cell and the mask over it are found by two different sums.
#[test]
fn a_masked_record_s_hidden_cells_are_the_column_of_the_row() {
    let Some(dir) = qubero_samples::dir("pickle") else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    // `[(1, --), (3, 4.5)]`: the second column of the first row is hidden.
    let want: &[&[Option<f64>]] = &[&[Some(1.0), None], &[Some(3.0), Some(4.5)]];
    for era in ["numpy-1", "numpy-2"] {
        for protocol in ["proto4", "proto2"] {
            let where_ = format!("{protocol}-{era}-masked-record");
            let bytes = std::fs::read(dir.join(format!("{where_}.pickle"))).unwrap_or_else(|e| panic!("{where_}: {e}"));
            let doc = Document::new(MemSource(bytes.clone()));
            let mut ev = Evaluator::new(formats::builtin("picklefpf").unwrap());
            let shape = ev.table_shape(&doc, &[1]).unwrap().unwrap_or_else(|| panic!("{where_}: no table"));
            let names: Vec<String> = shape.names.iter().map(|n| n.to_string()).collect();
            assert_eq!(names, ["a", "b"], "{where_}: the columns are the dtype's");
            let held = ev.pickle_cells(&doc, &[1], 0, 3).unwrap();
            assert_eq!(held.len(), want.len(), "{where_}: rows");
            for (i, (row, said)) in held.iter().zip(want.iter()).enumerate() {
                assert_eq!(row.len(), said.len(), "{where_} row {i}: columns");
                for (j, (cell, number)) in row.iter().zip(said.iter()).enumerate() {
                    // The `a` column is four bytes and the `b` column eight,
                    // so a cell says the width of the column it is in.
                    let (_, _, size) = placed(cell);
                    assert_eq!(size, if j == 0 { 32 } else { 64 }, "{where_} row {i} column {j}");
                    match number {
                        Some(number) => {
                            assert!(!cell.masked, "{where_} row {i} column {j}: not masked");
                            assert_eq!(text_of(cell), number.to_string(), "{where_} row {i} column {j}");
                        }
                        None => {
                            assert!(cell.masked, "{where_} row {i} column {j}: masked");
                            assert_eq!(cell.value, None, "{where_} row {i} column {j}: a masked cell shows nothing");
                        }
                    }
                }
            }
        }
    }
}
