//! The elements of a folded run, one line of text each, for the value table
//! beside the bytes.
//!
//! `spans` folds a long run into one entry: `body 72,000 values`. That entry
//! says what the run is and says nothing about any element of it, so the rows
//! the run covers show no numbers at all. This answers the other half of the
//! question: given the run and the bits on screen, what are the elements over
//! those bits, and what does each one read as.
//!
//! One call per screenful, like `spans`, and the same three shapes of run it
//! has to cope with: elements a fixed width apart, elements whose width is
//! only known by reading them, and the symbols a decoder's trace laid down
//! over a block. None of the three may cost anything per element outside the
//! window, since the run is scrolled through and the window is a screenful.

use super::*;

/// One element of a run, as the value table draws it.
#[derive(Debug, Clone, PartialEq)]
pub struct Cell {
    /// Index of the element in its run: the last path step.
    pub index: u64,
    pub offset_bits: u64,
    pub size_bits: u64,
    /// What the listing would say on a shared row (`brief`), or the symbol's
    /// name for a traced block (`literal 'a'`, `match 3 back 12`, `end of
    /// block`), so the two views agree.
    pub text: String,
    /// What the cell itself shows, which is shorter than `text` wherever the
    /// two words in front of a value are worth saying once in a tooltip and
    /// not on every cell of a screenful: a deflate literal is the byte, so a
    /// row of cells reads as the text the block produces, and a match is its
    /// two numbers. The same as `text` for everything else.
    pub label: String,
    /// "uint" | "int" | "float" | "bytes" | "str" | "enum" | "flags" |
    /// "composite" | "symbol" | "scale"
    pub kind: &'static str,
    /// False when the element's bits are not one contiguous run: a `q5_0`
    /// weight is four bits of `qs` and a fifth a dozen bytes away, so no run
    /// of bits on the row is the weight, and the view lays those out uniformly
    /// rather than against the bytes.
    pub contiguous: bool,
}

/// What family of value this is, in the words the view has layout rules for.
/// Numbers are right-aligned in their cell and everything else is not, so the
/// three kinds that are not in the table's vocabulary -- a signature, bytes
/// that have not arrived, a slot nobody filled in -- read as text, which is
/// what their `brief` is.
fn kind_of(v: &Value) -> &'static str {
    match v {
        Value::UInt(_) => "uint",
        Value::Int(_) => "int",
        Value::Float(_) => "float",
        Value::Bytes { .. } => "bytes",
        Value::Str(_) => "str",
        Value::Enum { .. } => "enum",
        Value::Flags { .. } => "flags",
        Value::Composite { .. } => "composite",
        Value::Magic { .. } | Value::Unread { .. } | Value::Unset(_) => "str",
    }
}

/// A weight as the value panel writes one: four significant digits, which is
/// more than a scale stored as a half float can justify, and the exponent form
/// for the numbers at either end, so that a very small one does not read as
/// zero and a very large one is not a wall of digits.
///
/// The same rule as `quantpanel.ts`'s `num`, down to how JavaScript writes an
/// exponent, so that the panel and the table say the same thing about the same
/// weight.
fn num(x: f64) -> String {
    if !x.is_finite() {
        return x.to_string();
    }
    if x == 0.0 {
        return "0".to_string();
    }
    let size = x.abs();
    if size >= 1e5 || size < 1e-4 {
        let s = format!("{x:.2e}");
        return match s.split_once('e') {
            Some((m, e)) if !e.starts_with('-') => format!("{m}e+{e}"),
            _ => s,
        };
    }
    // `Number(x.toPrecision(4))`: rounded to four digits, then written as
    // shortly as the number it rounded to allows.
    match format!("{x:.3e}").parse::<f64>() {
        Ok(v) => v.to_string(),
        Err(_) => x.to_string(),
    }
}

impl Evaluator {
    /// The elements of the folded run at `path` whose bits overlap
    /// `from_bit..to_bit`, in file order, at most `max`.
    ///
    /// `path` is the run a `Span` with `count > 0` names (its `path`), or a
    /// traced block. Anything else, and anything with no elements over the
    /// window, answers with nothing rather than with an error: this is called
    /// once a frame for whatever the column happens to be showing.
    pub fn run_cells<S: Source>(
        &mut self,
        doc: &Document<S>,
        path: &[usize],
        from_bit: u64,
        to_bit: u64,
        max: usize,
    ) -> R<Vec<Cell>> {
        if max == 0 || to_bit <= from_bit {
            return Ok(Vec::new());
        }
        // Resolving the run first is what propagates `Pending` for a run whose
        // bytes have not arrived, and for a traced block it is what opens the
        // stream, so the trace is there to read below.
        let run = self.node(doc, path)?;
        let ty = self.memo[path].ty.clone();
        let from = from_bit.max(run.offset_bits);
        let to = to_bit.min(run.offset_bits + run.size_bits);
        if to <= from {
            return Ok(Vec::new());
        }
        match &ty {
            Ty::Traced { part: TracedPart::Block(i) } => self.block_cells(*i, path, from, to, max),
            // A stream is one entry with a count of the fields inside it, and
            // those are not elements of a run: the blocks below it are.
            Ty::Decoded { .. } => Ok(Vec::new()),
            _ => self.element_cells(doc, path, &run, &ty, from, to, max),
        }
    }

    /// The symbols of one block of a decoder's trace. Nothing is placed: the
    /// trace already says where every step read from, so the first one over the
    /// window is found by halving and the rest are read off in order.
    fn block_cells(&mut self, block: u32, path: &[usize], from: u64, to: u64, max: usize) -> R<Vec<Cell>> {
        let Some((base, trace)) = self.trace_for(path) else { return Ok(Vec::new()) };
        let Some(view) = super::traced::BlockView::of(trace, block) else { return Ok(Vec::new()) };
        // A stored block codes nothing; its one step is the bytes copied
        // through, which the block's own entry already says the size of.
        if view.block.kind == crate::codec::BlockKind::Stored || view.symbols.is_empty() {
            return Ok(Vec::new());
        }
        // The trace counts bits from the front of the run and the view counts
        // them from the front of the file.
        let want = from.saturating_sub(base);
        let first = match trace.index_in(want) {
            Some(k) => (k as u32).max(view.symbols.start),
            None => view.symbols.start,
        };
        let mut out = Vec::new();
        for k in first..view.symbols.end {
            if out.len() >= max {
                break;
            }
            let Some(step) = trace.step(k as usize) else { break };
            let (at, size) = (base + step.in_bits.start, step.in_bits.end - step.in_bits.start);
            if at >= to {
                break;
            }
            if size == 0 || at + size <= from {
                continue;
            }
            out.push(Cell {
                index: (k - view.symbols.start) as u64,
                offset_bits: at,
                size_bits: size,
                text: super::traced::symbol_ty(&step).0,
                label: super::traced::symbol_label(&step),
                kind: "symbol",
                contiguous: true,
            });
        }
        Ok(out)
    }

    /// The elements of an array or a repeat over the window.
    ///
    /// Two ways in. Elements a fixed width apart are indexed by division, so a
    /// window in the middle of seventy thousand samples places the thousand it
    /// covers and not one more. Elements whose width is read is a walk, started
    /// from the nearest element the evaluator already knows the start of --
    /// which after the first frame is the previous frame's last element, so
    /// scrolling forward costs a walk of the window rather than of the run.
    fn element_cells<S: Source>(
        &mut self,
        doc: &Document<S>,
        path: &[usize],
        run: &NodeInfo,
        ty: &Ty,
        from: u64,
        to: u64,
        max: usize,
    ) -> R<Vec<Cell>> {
        if !matches!(ty, Ty::Array { .. } | Ty::Repeat { .. }) {
            return Ok(Vec::new());
        }
        // A repeat that ends on what it reads cannot say how many elements it
        // has without being walked to the end, and the window is not that
        // question: the walk below stops at the window instead. `u64::MAX`
        // stands for "as many as there turn out to be".
        let n = self.count_unless_walk(doc, path)?.unwrap_or(u64::MAX);
        if n == 0 {
            return Ok(Vec::new());
        }
        let mut start = 0usize;
        match self.stride(doc, path, ty)? {
            // A width of zero: every element is at the same bit and none of
            // them covers the window. Saying nothing beats spinning.
            Some(0) => return Ok(Vec::new()),
            Some(each) => {
                let i = (from - run.offset_bits) / each;
                if i >= n {
                    return Ok(Vec::new());
                }
                start = i as usize;
            }
            None => {
                let (j, _) = self.nearest_start_before(path, from);
                if (j as u64) < n {
                    start = j;
                }
            }
        }
        let mut out = Vec::new();
        let mut missing: Vec<Missing> = Vec::new();
        let mut p = path.to_vec();
        let mut i = start as u64;
        while i < n && out.len() < max {
            p.push(i as usize);
            let got = self.node(doc, &p);
            p.pop();
            let info = match got {
                Ok(info) => info,
                // The bytes are on their way. Every element after this one
                // needs them too, so ask for them once and stop here rather
                // than walking on over nothing.
                Err(EvalError::Pending(m)) => {
                    missing.extend(m);
                    break;
                }
                Err(e) if e.interrupted() => return Err(e),
                // The bytes after the last whole element of a run do not
                // parse, which is where a run of records ends. Counting it
                // would have stopped here too.
                Err(_) => break,
            };
            if info.offset_bits >= to {
                break;
            }
            // An element covering nothing would leave the window unreachable.
            if info.size_bits == 0 {
                break;
            }
            if info.offset_bits + info.size_bits > from {
                let room = max - out.len();
                match self.quant_cells(doc, &p, i, from, to, room) {
                    // A block whose weights this crate can take apart is its
                    // numbers, not one entry saying how many bytes it is.
                    Ok(Some(cells)) => out.extend(cells),
                    Ok(None) => out.push(self.cell(doc, &p, i, &info)?),
                    Err(EvalError::Pending(m)) => {
                        missing.extend(m);
                        break;
                    }
                    Err(e) if e.interrupted() => return Err(e),
                    // The bytes are there but will not read as a block: the
                    // last one of a tensor cut short by the end of the file.
                    // Saying where it is beats saying nothing about it.
                    Err(_) => out.push(self.cell(doc, &p, i, &info)?),
                }
            }
            i += 1;
        }
        if !missing.is_empty() {
            missing.sort_by_key(|m| m.chunk);
            missing.dedup();
            return Err(EvalError::Pending(missing));
        }
        Ok(out)
    }

    /// One packed block as the weights inside it, or `None` for an element
    /// that is not one of those.
    ///
    /// A tensor's run is blocks, and a block is bytes: eighteen of them for a
    /// `q4_0`, holding a scale and thirty-two numbers in an order the bytes do
    /// not show. One cell per block would put `Q4_0` beside every eighteen
    /// bytes of a gigabyte of weights and say nothing the reader wanted. So a
    /// block is not one cell but the cells of what it holds: the scale where
    /// the scale is, and every weight over the bits it is stored in.
    ///
    /// The order is the order of the bytes rather than the order of the
    /// tensor's row, which are not the same order: weight 16 of a `q4_0` is
    /// the top half of the byte weight 0 is the bottom half of, so it comes
    /// first. That is what the reader is looking at.
    fn quant_cells<S: Source>(
        &mut self,
        doc: &Document<S>,
        run: &[usize],
        i: u64,
        from: u64,
        to: u64,
        max: usize,
    ) -> R<Option<Vec<Cell>>> {
        let mut p = run.to_vec();
        p.push(i as usize);
        let Some((_, block, at)) = self.quant_block(doc, &p)? else { return Ok(None) };
        let mut cells = Vec::new();
        // The scale as the listing writes it, read off the block's own field
        // rather than printed from the unpacked number: `d` is a half float,
        // and the whole of what a half float stands for is nineteen digits of
        // decimal nobody asked for.
        for name in std::iter::once("d").chain(block.second.map(|o| o.name)) {
            let Some(cp) = self.child_named(doc, &p, name)? else { continue };
            let info = self.node(doc, &cp)?;
            let reading = super::listing::brief(&info.value);
            cells.push(Cell {
                index: i,
                offset_bits: info.offset_bits,
                size_bits: info.size_bits,
                text: format!("{name} \u{b7} {reading}"),
                label: reading,
                kind: "scale",
                contiguous: true,
            });
        }
        for (j, w) in block.weights.iter().enumerate() {
            cells.push(Cell {
                index: i,
                offset_bits: at + u64::from(w.bits.bit),
                size_bits: u64::from(w.bits.width),
                text: format!("weight {j} \u{b7} stored {} \u{b7} value {}", w.q, num(w.value)),
                label: w.q.to_string(),
                kind: "int",
                contiguous: w.high.is_none(),
            });
        }
        // A K type keeps its scale after its weights, so nothing may assume
        // the scales come first.
        cells.sort_by_key(|c| c.offset_bits);
        cells.retain(|c| c.offset_bits < to && c.offset_bits + c.size_bits > from);
        cells.truncate(max);
        Ok(Some(cells))
    }

    /// One element, as its one line of text.
    ///
    /// A leaf reads as its value does on a shared row. A record reads as its
    /// one-line reading when the format says it has one, and as its name
    /// otherwise: a tensor row saying `x_embedder.bias` is what the listing
    /// shows for it, and the alternative is `[81]`.
    fn cell<S: Source>(&mut self, doc: &Document<S>, run: &[usize], i: u64, info: &NodeInfo) -> R<Cell> {
        let mut p = run.to_vec();
        p.push(i as usize);
        let text = if info.composite {
            let inline = matches!(self.memo[&p].ty.base(), Ty::Struct(s) if s.inline);
            if inline {
                let mut parts = Vec::new();
                self.one_line(doc, &p, &mut parts)?;
                parts.join(" ")
            } else {
                // The index is what the path already says; the name a record
                // carries is what the reader cannot see from where the cell
                // is. A record with no name but its index keeps the index,
                // since an empty cell says less than `[81]` does.
                let index = format!("[{i}]");
                let named = info.name.strip_prefix(&index).unwrap_or(&info.name).trim();
                if named.is_empty() { info.name.clone() } else { named.to_string() }
            }
        } else {
            super::listing::brief(&info.value)
        };
        Ok(Cell {
            index: i,
            offset_bits: info.offset_bits,
            size_bits: info.size_bits,
            // Only a symbol has anything shorter to say in a cell than it says
            // in a tooltip; a number reads the same in both.
            label: text.clone(),
            text,
            kind: kind_of(&info.value),
            contiguous: true,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::MemSource;
    use crate::template::{Endian::*, Expr as E, Ty as T};

    fn doc(bytes: Vec<u8>) -> Document<MemSource> {
        Document::new(MemSource(bytes))
    }

    fn chunk_bytes(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut v = id.to_vec();
        v.extend_from_slice(&(body.len() as u32).to_le_bytes());
        v.extend_from_slice(body);
        if body.len() % 2 == 1 {
            v.push(0);
        }
        v
    }

    /// A WAVE file of 16-bit mono samples, built the way `formats::wav`'s own
    /// tests build one: a `fmt ` chunk saying how wide a sample is, and a
    /// `data` chunk of that many of them. `[3, 2, 2]` is the run of samples.
    const SAMPLES: &[usize] = &[3, 2, 2];

    fn wav_of(samples: &[i16]) -> Document<MemSource> {
        let mut fmt = Vec::new();
        fmt.extend_from_slice(&1u16.to_le_bytes()); // PCM
        fmt.extend_from_slice(&1u16.to_le_bytes()); // one channel
        fmt.extend_from_slice(&44_100u32.to_le_bytes());
        fmt.extend_from_slice(&88_200u32.to_le_bytes());
        fmt.extend_from_slice(&2u16.to_le_bytes());
        fmt.extend_from_slice(&16u16.to_le_bytes());
        let body: Vec<u8> = samples.iter().flat_map(|v| v.to_le_bytes()).collect();
        let mut inner = chunk_bytes(b"fmt ", &fmt);
        inner.extend_from_slice(&chunk_bytes(b"fact", &1234u32.to_le_bytes()));
        inner.extend_from_slice(&chunk_bytes(b"data", &body));
        let mut out = b"RIFF".to_vec();
        out.extend_from_slice(&((inner.len() + 4) as u32).to_le_bytes());
        out.extend_from_slice(b"WAVE");
        out.extend_from_slice(&inner);
        doc(out)
    }

    fn wav_eval() -> Evaluator {
        Evaluator::new(crate::formats::builtin("wav").expect("the wav template"))
    }

    /// Seventy-two thousand samples: the case the value table is for, where a
    /// run is far longer than any screenful of it.
    fn long_wav() -> (Document<MemSource>, Evaluator) {
        let samples: Vec<i16> = (0..72_000i32).map(|i| (i % 3001 - 1500) as i16).collect();
        (wav_of(&samples), wav_eval())
    }

    /// A window in the middle of a long run answers with the elements over it
    /// and no others, numbered as they are in the run.
    #[test]
    fn a_window_in_the_middle_of_a_run_is_the_elements_over_it() {
        let (d, mut e) = long_wav();
        let run = e.node(&d, SAMPLES).unwrap();
        assert_eq!(run.child_count, 72_000);
        let from = run.offset_bits + 36_000 * 16;
        let cells = e.run_cells(&d, SAMPLES, from, from + 1_000 * 16, 4_000).unwrap();
        assert_eq!(cells.len(), 1_000);
        assert_eq!(cells[0].index, 36_000);
        assert_eq!(cells[0].offset_bits, from);
        assert_eq!(cells[0].size_bits, 16);
        assert_eq!(cells.last().unwrap().index, 36_999);
        assert_eq!(cells[0].kind, "int");
        assert!(cells.iter().all(|c| c.contiguous));
        // The text is the same reading the listing gives the field.
        assert_eq!(cells[0].text, (36_000i32 % 3001 - 1500).to_string());
        // A number has nothing shorter to say in a cell than in a tooltip.
        assert!(cells.iter().all(|c| c.label == c.text));
    }

    /// The window is clamped to the run at both ends: a screen that starts
    /// before the run begins starts at its first element, and one that runs
    /// past the end stops at its last.
    #[test]
    fn a_window_over_the_edges_of_a_run_stops_at_them() {
        let (d, mut e) = long_wav();
        let run = e.node(&d, SAMPLES).unwrap();
        let end = run.offset_bits + run.size_bits;
        let head = e.run_cells(&d, SAMPLES, 0, run.offset_bits + 4 * 16, 100).unwrap();
        assert_eq!(head.len(), 4);
        assert_eq!(head[0].index, 0);
        assert_eq!(head[0].offset_bits, run.offset_bits);
        let tail = e.run_cells(&d, SAMPLES, end - 3 * 16, end + 8_000, 100).unwrap();
        assert_eq!(tail.len(), 3);
        assert_eq!(tail.last().unwrap().index, 71_999);
        assert_eq!(tail.last().unwrap().offset_bits + tail.last().unwrap().size_bits, end);
        // Wholly past the run, and wholly before it: nothing, not an error.
        assert!(e.run_cells(&d, SAMPLES, end, end + 8_000, 100).unwrap().is_empty());
        assert!(e.run_cells(&d, SAMPLES, 0, 8, 100).unwrap().is_empty());
    }

    /// `max` cuts the answer short at the front of the window rather than
    /// sampling it.
    #[test]
    fn max_cuts_the_answer_short_at_the_front() {
        let (d, mut e) = long_wav();
        let run = e.node(&d, SAMPLES).unwrap();
        let from = run.offset_bits + 100 * 16;
        let cells = e.run_cells(&d, SAMPLES, from, from + 500 * 16, 7).unwrap();
        assert_eq!(cells.len(), 7);
        assert_eq!(cells[0].index, 100);
        assert_eq!(cells[6].index, 106);
    }

    /// Reaching an element in the middle of a fixed-width run is arithmetic,
    /// not a walk: the run is placed by `stride`, so asking for the element at
    /// bit 36,000 x 16 puts a handful of nodes in memory rather than 36,000.
    #[test]
    fn an_element_of_a_fixed_width_run_costs_a_constant() {
        let (d, mut e) = long_wav();
        e.node(&d, SAMPLES).unwrap();
        let before = e.memo.len();
        e.node(&d, &[3, 2, 2, 36_000]).unwrap();
        let grew = e.memo.len() - before;
        assert!(grew < 8, "{grew} nodes placed to reach element 36,000 of a fixed-width run");
    }

    /// And so is a screenful of them: a thousand cells cost a thousand nodes
    /// and not thirty-seven thousand.
    #[test]
    fn a_screenful_of_a_fixed_width_run_places_only_the_screenful() {
        let (d, mut e) = long_wav();
        let run = e.node(&d, SAMPLES).unwrap();
        let before = e.memo.len();
        let from = run.offset_bits + 36_000 * 16;
        let cells = e.run_cells(&d, SAMPLES, from, from + 1_000 * 16, 4_000).unwrap();
        let grew = e.memo.len() - before;
        assert_eq!(cells.len(), 1_000);
        assert!(grew < 1_100, "{grew} nodes placed for a window of 1,000 elements");
    }

    /// Roughly how long a screenful costs, cold and then warm. Not an
    /// assertion about a machine's speed; the bound is that neither call is
    /// the length of the run.
    #[test]
    fn a_screenful_of_a_long_run_is_quick() {
        let (d, mut e) = long_wav();
        let run = e.node(&d, SAMPLES).unwrap();
        let from = run.offset_bits + 36_000 * 16;
        let cold = std::time::Instant::now();
        e.run_cells(&d, SAMPLES, from, from + 1_000 * 16, 4_000).unwrap();
        let cold = cold.elapsed();
        let warm = std::time::Instant::now();
        e.run_cells(&d, SAMPLES, from, from + 1_000 * 16, 4_000).unwrap();
        let warm = warm.elapsed();
        eprintln!("run_cells over 1,000 of 72,000 samples: cold {cold:?}, warm {warm:?}");
        assert!(cold.as_millis() < 500, "a screenful took {cold:?}");
    }

    /// Elements whose width is only known by reading them: a run of leb128
    /// numbers, where nothing but the walk says where element 20,000 starts.
    fn varint_run() -> (Document<MemSource>, Evaluator) {
        fn leb(mut v: u64, out: &mut Vec<u8>) {
            loop {
                let b = (v & 0x7f) as u8;
                v >>= 7;
                out.push(if v == 0 { b } else { b | 0x80 });
                if v == 0 {
                    break;
                }
            }
        }
        let n = 30_000u64;
        let mut bytes = Vec::new();
        leb(n, &mut bytes);
        // Values that need one, two and three bytes in turn, so no stride
        // could ever place them.
        for i in 0..n {
            let v = match i % 3 {
                0 => i % 100,
                1 => 200 + i % 1000,
                _ => 40_000 + i % 1000,
            };
            leb(v, &mut bytes);
        }
        let t = Template::new(
            "t",
            T::structure("Root", vec![("n", T::leb_u()), ("xs", T::array(T::leb_u(), E::field("n")))]),
        );
        (doc(bytes), Evaluator::new(t))
    }

    #[test]
    fn a_run_of_variable_width_elements_is_walked_to_the_window() {
        let (d, mut e) = varint_run();
        let run = e.node(&d, &[1]).unwrap();
        assert_eq!(run.child_count, 30_000);
        // Where element 20,000 is, asked the ordinary way, so the window can
        // be checked against it.
        let at = e.node(&d, &[1, 20_000]).unwrap();
        let cells = e.run_cells(&d, &[1], at.offset_bits, at.offset_bits + 40 * 8, 4_000).unwrap();
        assert_eq!(cells[0].index, 20_000);
        assert_eq!(cells[0].offset_bits, at.offset_bits);
        assert_eq!(cells[0].text, super::listing::brief(&at.value));
        // In order, with no gaps: each starts where the one before it ended.
        for w in cells.windows(2) {
            assert_eq!(w[1].index, w[0].index + 1);
            assert_eq!(w[1].offset_bits, w[0].offset_bits + w[0].size_bits);
        }
        // Nothing runs past the window's far edge.
        let last = cells.last().unwrap();
        assert!(last.offset_bits < at.offset_bits + 40 * 8);
        assert!(cells.iter().any(|c| c.size_bits == 24), "no three-byte element in {} cells", cells.len());
    }

    /// A window further on does not walk the run again from the start: the
    /// walk carries on from where the last one stopped.
    #[test]
    fn a_second_window_of_a_walked_run_carries_on_from_the_first() {
        let (d, mut e) = varint_run();
        let at = e.node(&d, &[1, 20_000]).unwrap();
        e.run_cells(&d, &[1], at.offset_bits, at.offset_bits + 200 * 8, 4_000).unwrap();
        let settled = e.memo.len();
        let next = e.node(&d, &[1, 20_100]).unwrap();
        let cells = e.run_cells(&d, &[1], next.offset_bits, next.offset_bits + 200 * 8, 4_000).unwrap();
        assert_eq!(cells[0].index, 20_100);
        let grew = e.memo.len().saturating_sub(settled);
        assert!(grew < 400, "{grew} nodes placed for the next window of a walked run");
    }

    /// One cell per record, reading as the record's name, since these say
    /// nothing on one line of their own.
    #[test]
    fn a_run_of_records_is_one_cell_each() {
        let t = Template::new(
            "t",
            T::structure(
                "Root",
                vec![(
                    "rows",
                    T::array(T::structure("Row", vec![("a", T::u8()), ("b", T::u16(Big))]), E::lit(64)),
                )],
            ),
        );
        let d = doc((0..64u8).flat_map(|i| [i, 0, i]).collect());
        let mut e = Evaluator::new(t);
        let run = e.node(&d, &[0]).unwrap();
        assert_eq!(run.child_count, 64);
        let from = run.offset_bits + 10 * 24;
        let cells = e.run_cells(&d, &[0], from, from + 5 * 24, 100).unwrap();
        assert_eq!(cells.len(), 5);
        assert_eq!(cells[0].index, 10);
        assert_eq!(cells[0].size_bits, 24, "a record is one cell, not one per field");
        assert_eq!(cells[0].kind, "composite");
        assert_eq!(cells[0].text, "[10]");
    }

    /// A record the format says reads on one row reads that way in a cell too,
    /// so the value table and the listing say the same thing about it.
    #[test]
    fn a_record_that_reads_on_one_row_reads_that_way_in_a_cell() {
        let t = Template::new(
            "t",
            T::structure(
                "Root",
                vec![(
                    "rows",
                    T::array(T::inline_structure("Row", vec![("a", T::u8()), ("b", T::u8())]), E::lit(64)),
                )],
            ),
        );
        let d = doc((0..64u8).flat_map(|i| [i, i + 1]).collect());
        let mut e = Evaluator::new(t);
        let run = e.node(&d, &[0]).unwrap();
        let from = run.offset_bits + 10 * 16;
        let cells = e.run_cells(&d, &[0], from, from + 16, 100).unwrap();
        assert_eq!(cells.len(), 1);
        assert_eq!(cells[0].text, "10 11");
    }

    /// Bytes that have not arrived come back as `Pending` with the chunks to
    /// fetch, the way `children` does, rather than as an empty table that
    /// would read as a run of no elements.
    #[test]
    fn a_window_over_bytes_that_have_not_arrived_asks_for_them() {
        use crate::source::ChunkStore;
        let t = Template::new(
            "t",
            T::structure("Root", vec![("n", T::u8()), ("xs", T::array(T::u16(Big), E::field("n")))]),
        );
        let mut d = Document::new(ChunkStore::new(65, 8, 16));
        let mut e = Evaluator::new(t);
        // The count is in the first chunk, so the run is there to be asked
        // about; the elements the window wants are in one that is not.
        d.source_mut().insert(0, vec![32, 0, 1, 0, 2, 0, 3, 0].into_boxed_slice());
        let run = e.node(&d, &[1]).unwrap();
        assert_eq!(run.child_count, 32);
        let from = run.offset_bits + 20 * 16;
        match e.run_cells(&d, &[1], from, from + 4 * 16, 100) {
            Err(EvalError::Pending(missing)) => assert!(!missing.is_empty()),
            other => panic!("a window over missing bytes answered {other:?}"),
        }
    }

    /// The symbols of a deflate block, read off the trace rather than placed:
    /// each is the bits the decoder read for it, named as the listing names it.
    fn zlib_of(content: &[u8]) -> (Document<MemSource>, Evaluator) {
        let packed = miniz_oxide::deflate::compress_to_vec_zlib(content, 6);
        (doc(packed), Evaluator::new(crate::formats::builtin("zlib").unwrap()))
    }

    #[test]
    fn the_symbols_of_a_traced_block_are_cells() {
        let (d, mut e) = zlib_of(b"abcabcabcabcabcabcabcabcabcabc");
        let block = e.node(&d, &[6, 1, 0]).unwrap();
        let cells = e
            .run_cells(&d, &[6, 1, 0], block.offset_bits, block.offset_bits + block.size_bits, 10_000)
            .unwrap();
        assert!(!cells.is_empty());
        assert!(cells.iter().all(|c| c.kind == "symbol"));
        assert_eq!(cells[0].index, 0);
        assert_eq!(cells[0].text, "literal 'a'");
        assert_eq!(cells.last().unwrap().text, "end of block");
        assert!(cells.iter().any(|c| c.text.starts_with("match ")), "no match among the symbols");
        // What the cell shows is shorter than what the tooltip says: a literal
        // is the byte itself, so a row of them reads as the text the block
        // produces, and a match keeps its numbers without the word.
        assert_eq!(cells[0].label, "a");
        assert_eq!(cells.last().unwrap().label, "end of block");
        let m = cells.iter().find(|c| c.text.starts_with("match ")).expect("a match");
        assert_eq!(m.label, m.text.strip_prefix("match ").unwrap());
        // The symbols follow one another with nothing in between, and the
        // first starts after the block's header rather than at the block.
        assert!(cells[0].offset_bits > block.offset_bits);
        for w in cells.windows(2) {
            assert_eq!(w[1].index, w[0].index + 1);
            assert_eq!(w[1].offset_bits, w[0].offset_bits + w[0].size_bits);
        }
        // The count agrees with what the block's own entry claims.
        let spans = e.spans(&d, block.offset_bits, block.offset_bits + block.size_bits, 100).unwrap();
        let entry = spans.iter().find(|s| s.name.contains("block")).expect("a block entry");
        assert_eq!(entry.count, cells.len() as u64);
    }

    /// A window over part of a block is the symbols over that part, numbered
    /// where they are in the block.
    #[test]
    fn a_window_over_part_of_a_block_is_the_symbols_over_it() {
        let (d, mut e) = zlib_of(&"the shape of the shape of the shape. ".repeat(400).into_bytes());
        let block = e.node(&d, &[6, 1, 0]).unwrap();
        let whole = e
            .run_cells(&d, &[6, 1, 0], block.offset_bits, block.offset_bits + block.size_bits, 100_000)
            .unwrap();
        assert!(whole.len() > 40, "only {} symbols", whole.len());
        let mid = whole[whole.len() / 2].clone();
        let window = e.run_cells(&d, &[6, 1, 0], mid.offset_bits, mid.offset_bits + 200, 10_000).unwrap();
        assert_eq!(window[0], mid);
        for w in window.windows(2) {
            assert_eq!(w[1].index, w[0].index + 1);
        }
        assert!(window.last().unwrap().offset_bits < mid.offset_bits + 200);
    }

    // ----- packed quantised blocks -----

    /// A GGUF file of one tensor of `ty`, holding `weights` numbers, with
    /// `payload` as its bytes. The same file `gguf`'s own tests build for
    /// `a_quantised_tensor_reads_as_the_blocks_its_type_packs`, with the count
    /// left open: a K type packs 256 weights to a block, so the sixty-four a
    /// fixed count would give are no block at all.
    fn gguf_tensor(ty: u32, weights: u64, payload: &[u8]) -> Document<MemSource> {
        let name = b"blk.0.ffn_up.weight";
        let mut b = b"GGUF".to_vec();
        b.extend_from_slice(&3u32.to_le_bytes());
        b.extend_from_slice(&1u64.to_le_bytes()); // tensors
        b.extend_from_slice(&0u64.to_le_bytes()); // metadata entries
        b.extend_from_slice(&(name.len() as u64).to_le_bytes());
        b.extend_from_slice(name);
        b.extend_from_slice(&1u32.to_le_bytes()); // one dimension
        b.extend_from_slice(&weights.to_le_bytes());
        b.extend_from_slice(&ty.to_le_bytes());
        b.extend_from_slice(&0u64.to_le_bytes()); // at the start of the data
        b.resize(b.len().div_ceil(32) * 32, 0);
        b.extend_from_slice(payload);
        doc(b)
    }

    /// The half-precision bits of 1.0, so that a weight's value is the stored
    /// integer and the arithmetic is checkable by eye.
    const ONE: [u8; 2] = [0x00, 0x3C];

    fn gguf() -> Evaluator {
        Evaluator::new(crate::formats::builtin("gguf").expect("the gguf template"))
    }

    /// The tensor's run of blocks, which is what the value table is asked
    /// about.
    const BLOCKS: &[usize] = &[6, 0];

    /// Two `q4_0` blocks: a scale and thirty-two nibbles apiece.
    fn q4_0_pair() -> (Document<MemSource>, Evaluator) {
        let mut payload = Vec::new();
        for block in 0..2u8 {
            payload.extend_from_slice(&ONE);
            // Block 0: every byte 0x9A, so the low nibble is 10 and the high
            // one 9, which are 2 and 1 once the bias of eight comes off.
            // Block 1: 0x10, which is -8 and -7.
            payload.extend(std::iter::repeat_n(if block == 0 { 0x9A } else { 0x10 }, 16));
        }
        (gguf_tensor(2, 64, &payload), gguf())
    }

    /// A tensor of packed weights reads as the weights, not as one entry per
    /// block, and they come in the order of the bytes: the scale, then the
    /// high half of the first `qs` byte, then its low half.
    #[test]
    fn a_quantised_run_is_the_weights_the_blocks_hold() {
        let (d, mut e) = q4_0_pair();
        let run = e.node(&d, BLOCKS).unwrap();
        assert_eq!(run.child_count, 2, "two blocks");
        let at = run.offset_bits;
        let cells = e.run_cells(&d, BLOCKS, at, at + run.size_bits, 10_000).unwrap();
        // A scale and thirty-two weights from each block.
        assert_eq!(cells.len(), 2 * 33);
        // The scale, as the listing writes the half float and not as the
        // nineteen digits the number expands to.
        assert_eq!(cells[0].kind, "scale");
        assert_eq!((cells[0].offset_bits, cells[0].size_bits), (at, 16));
        assert_eq!(cells[0].label, "1");
        assert_eq!(cells[0].text, "d \u{b7} 1");
        // Weight 16 is the top half of the byte weight 0 is the bottom half
        // of, so the reader meets it first.
        assert_eq!((cells[1].offset_bits, cells[1].size_bits), (at + 16, 4));
        assert_eq!(cells[1].kind, "int");
        assert_eq!(cells[1].label, "1");
        assert_eq!(cells[1].text, "weight 16 \u{b7} stored 1 \u{b7} value 1");
        assert_eq!(cells[2].offset_bits, at + 20);
        assert_eq!(cells[2].text, "weight 0 \u{b7} stored 2 \u{b7} value 2");
        assert_eq!(cells[3].text, "weight 17 \u{b7} stored 1 \u{b7} value 1");
        // Every cell belongs to the block it is in, since that is all a click
        // can select, and the whole run is in the order of the bytes.
        assert!(cells[..33].iter().all(|c| c.index == 0));
        assert!(cells[33..].iter().all(|c| c.index == 1));
        for w in cells.windows(2) {
            assert!(w[1].offset_bits >= w[0].offset_bits + w[0].size_bits, "{:?} then {:?}", w[0], w[1]);
        }
        // A nibble is four bits in a row, so the table can lay it against the
        // byte it is half of.
        assert!(cells.iter().all(|c| c.contiguous));
        // The second block's numbers are its own.
        assert_eq!(cells[34].text, "weight 16 \u{b7} stored -7 \u{b7} value -7");
        assert_eq!(cells[35].text, "weight 0 \u{b7} stored -8 \u{b7} value -8");
    }

    /// A window over the second block is the second block's cells: nothing of
    /// the block before it, and the first weight is the one the window starts
    /// on rather than the first of the tensor.
    #[test]
    fn a_window_over_one_block_is_that_blocks_weights() {
        let (d, mut e) = q4_0_pair();
        let second = e.node(&d, &[6, 0, 1]).unwrap();
        let cells = e.run_cells(&d, BLOCKS, second.offset_bits, second.offset_bits + second.size_bits, 10_000).unwrap();
        assert_eq!(cells.len(), 33);
        assert!(cells.iter().all(|c| c.index == 1));
        assert_eq!(cells[0].offset_bits, second.offset_bits);
        // Half a block: the cells over those bits and no others.
        let half = second.offset_bits + 9 * 8;
        let part = e.run_cells(&d, BLOCKS, half, half + 2 * 8, 10_000).unwrap();
        assert_eq!(part.len(), 4, "two bytes of nibbles");
        assert!(part.iter().all(|c| c.offset_bits >= half && c.offset_bits < half + 2 * 8));
        assert_eq!(part[0].text, "weight 23 \u{b7} stored -7 \u{b7} value -7");
    }

    /// `max` counts cells, not blocks: a screenful of a tensor is thousands of
    /// weights, and the answer stops where it is asked to.
    #[test]
    fn max_counts_the_weights_and_not_the_blocks() {
        let (d, mut e) = q4_0_pair();
        let run = e.node(&d, BLOCKS).unwrap();
        let cells = e.run_cells(&d, BLOCKS, run.offset_bits, run.offset_bits + run.size_bits, 5).unwrap();
        assert_eq!(cells.len(), 5);
        assert_eq!(cells[0].kind, "scale");
    }

    /// A `q5_0` weight keeps its fifth bit in `qh`, bytes away from the four
    /// in `qs`, so no run of bits on the row is the weight. The cell says so,
    /// and the view lays those out uniformly rather than against the bytes.
    #[test]
    fn a_five_bit_weight_is_not_one_run_of_bits() {
        let mut payload = Vec::new();
        payload.extend_from_slice(&ONE); // d
        payload.extend_from_slice(&1u32.to_le_bytes()); // qh: weight 0's fifth bit
        payload.extend(std::iter::repeat_n(0x21u8, 16)); // low 1, high 2
        let d = gguf_tensor(6, 32, &payload);
        let mut e = gguf();
        let run = e.node(&d, BLOCKS).unwrap();
        let cells = e.run_cells(&d, BLOCKS, run.offset_bits, run.offset_bits + run.size_bits, 10_000).unwrap();
        assert_eq!(cells.len(), 33);
        // The scale is where it is written and is one run; no weight is.
        assert_eq!(cells[0].kind, "scale");
        assert!(cells[0].contiguous);
        assert!(cells[1..].iter().all(|c| !c.contiguous), "a five-bit weight claimed to be one run");
        // The weights start after `qh`, at the first byte of `qs`.
        assert_eq!(cells[1].offset_bits, run.offset_bits + 6 * 8);
        // Weight 16 is the high nibble, 2, and nothing in `qh` is set for it.
        assert_eq!(cells[1].text, "weight 16 \u{b7} stored -14 \u{b7} value -14");
        // Weight 0 is the low nibble, 1, with the fifth bit set above it: 17,
        // which is 1 once the bias of sixteen comes off.
        assert_eq!(cells[2].text, "weight 0 \u{b7} stored 1 \u{b7} value 1");
    }

    /// Eight-bit weights are a byte each, so a `q8_0` block is thirty-two
    /// cells of eight bits, one under every byte after the scale.
    #[test]
    fn a_q8_0_block_is_a_cell_a_byte() {
        let mut payload = ONE.to_vec();
        payload.extend((0..32u8).map(|i| i.wrapping_sub(4)));
        let d = gguf_tensor(8, 32, &payload);
        let mut e = gguf();
        let run = e.node(&d, BLOCKS).unwrap();
        let at = run.offset_bits;
        let cells = e.run_cells(&d, BLOCKS, at, at + run.size_bits, 10_000).unwrap();
        assert_eq!(cells.len(), 33);
        let weights = &cells[1..];
        assert_eq!(weights.len(), 32);
        assert!(weights.iter().all(|c| c.size_bits == 8 && c.contiguous && c.kind == "int"));
        for (j, c) in weights.iter().enumerate() {
            assert_eq!(c.offset_bits, at + (2 + j as u64) * 8);
        }
        // Read signed rather than biased, which is what the eight-bit types do.
        assert_eq!(weights[0].text, "weight 0 \u{b7} stored -4 \u{b7} value -4");
        assert_eq!(weights[0].label, "-4");
        assert_eq!(weights[31].text, "weight 31 \u{b7} stored 27 \u{b7} value 27");
    }

    /// A block whose weights come out of a table only ggml has stays one cell:
    /// the editor can say how big it is and will not invent what is inside it.
    #[test]
    fn a_block_this_crate_cannot_unpack_is_still_one_cell() {
        // IQ2_XXS: 256 weights in 66 bytes.
        let d = gguf_tensor(16, 256, &[0x5A; 66]);
        let mut e = gguf();
        let run = e.node(&d, BLOCKS).unwrap();
        let cells = e.run_cells(&d, BLOCKS, run.offset_bits, run.offset_bits + run.size_bits, 10_000).unwrap();
        assert_eq!(cells.len(), 1);
        assert_eq!(cells[0].kind, "composite");
        assert_eq!((cells[0].offset_bits, cells[0].size_bits), (run.offset_bits, 66 * 8));
    }

    /// A K type's scale is written after its weights, so the cells are in the
    /// order of the bytes and not scale-first, and the six-bit group scales in
    /// between get no cells of their own this round.
    #[test]
    fn a_k_types_scale_is_a_cell_where_the_scale_is() {
        // Q2_K: scales[16], qs[64], d, dmin.
        let d = gguf_tensor(10, 256, &[0x11; 84]);
        let mut e = gguf();
        let run = e.node(&d, BLOCKS).unwrap();
        let at = run.offset_bits;
        let cells = e.run_cells(&d, BLOCKS, at, at + run.size_bits, 10_000).unwrap();
        assert_eq!(cells.len(), 256 + 2, "256 weights, a scale and a minimum");
        // The sixteen bytes of group scales are nobody's cell; the weights
        // start at `qs`.
        assert_eq!(cells[0].offset_bits, at + 16 * 8);
        assert_eq!(cells[0].size_bits, 2);
        let scales: Vec<&Cell> = cells.iter().filter(|c| c.kind == "scale").collect();
        assert_eq!(scales.len(), 2);
        assert_eq!(scales[0].offset_bits, at + 80 * 8);
        assert!(scales[0].text.starts_with("d \u{b7} "));
        assert!(scales[1].text.starts_with("dmin \u{b7} "));
        // And they are last, because that is where they are written.
        assert_eq!(cells[cells.len() - 2].kind, "scale");
        assert_eq!(cells[cells.len() - 1].kind, "scale");
    }

    /// A `q4_1` pairs its scale with a minimum, and the cell names which is
    /// which so the two are not read as two scales.
    #[test]
    fn a_scale_cell_says_which_scale_it_is() {
        let mut payload = ONE.to_vec();
        payload.extend_from_slice(&ONE); // m
        payload.extend(std::iter::repeat_n(0x30u8, 16));
        let d = gguf_tensor(3, 32, &payload);
        let mut e = gguf();
        let run = e.node(&d, BLOCKS).unwrap();
        let cells = e.run_cells(&d, BLOCKS, run.offset_bits, run.offset_bits + run.size_bits, 10_000).unwrap();
        assert_eq!(cells[0].text, "d \u{b7} 1");
        assert_eq!(cells[1].text, "m \u{b7} 1");
        // Nothing is taken off a `q4_1` nibble, and the minimum is added.
        assert_eq!(cells[2].text, "weight 16 \u{b7} stored 3 \u{b7} value 4");
        assert_eq!(cells[3].text, "weight 0 \u{b7} stored 0 \u{b7} value 1");
    }

    /// A scale off a real model, rather than the 1.0 the other tests use so
    /// that their arithmetic is checkable. The cell says what the listing says
    /// about the same field, which for a half float is the few digits it takes
    /// to tell it from its neighbours and not the nineteen the number expands
    /// to. How short that is stays `brief`'s business, since the chip and the
    /// cell have to agree.
    #[test]
    fn a_scale_cell_reads_as_the_listing_reads_the_field() {
        let mut payload = vec![0x35, 0x1C]; // f16 0.00411
        payload.extend(std::iter::repeat_n(0x9Au8, 16));
        let d = gguf_tensor(2, 32, &payload);
        let mut e = gguf();
        let run = e.node(&d, BLOCKS).unwrap();
        let cells = e.run_cells(&d, BLOCKS, run.offset_bits, run.offset_bits + run.size_bits, 10_000).unwrap();
        let scale = e.node(&d, &[6, 0, 0, 0]).unwrap();
        assert_eq!(cells[0].label, super::listing::brief(&scale.value));
        assert_eq!(cells[0].label, "0.00411");
        assert_eq!(cells[0].text, "d \u{b7} 0.00411");
        // A weight's value is cut to four digits, which is as much as a scale
        // stored in sixteen bits can justify. It is the scale times the stored
        // integer worked out in full and then shortened, which the value panel
        // does the same way, so the two agree with one another rather than
        // with the shortest decimal the scale itself reads as.
        assert_eq!(cells[1].text, "weight 16 \u{b7} stored 1 \u{b7} value 0.004108");
        assert_eq!(cells[2].text, "weight 0 \u{b7} stored 2 \u{b7} value 0.008217");
    }

    /// Four significant digits, the way the value panel writes a weight, so
    /// the panel and the table say the same thing about the same number.
    #[test]
    fn a_weights_value_is_four_significant_digits() {
        assert_eq!(num(0.0), "0");
        assert_eq!(num(-0.0), "0");
        assert_eq!(num(2.0), "2");
        assert_eq!(num(-0.012345678), "-0.01235");
        assert_eq!(num(0.004110336303710938), "0.00411");
        assert_eq!(num(123456.0), "1.23e+5");
        assert_eq!(num(0.0000123), "1.23e-5");
    }

    /// A tensor big enough to scroll through, asked for one screenful of it.
    /// The bound is that a window costs the window and not the tensor.
    #[test]
    fn a_screenful_of_a_quantised_tensor_is_quick() {
        // Sixty-four `q4_k` blocks: 144 bytes and 256 weights each.
        let d = gguf_tensor(12, 256 * 64, &vec![0x5Au8; 144 * 64]);
        let mut e = gguf();
        let run = e.node(&d, BLOCKS).unwrap();
        assert_eq!(run.child_count, 64);
        // A screenful of hex at sixteen bytes a row: thirty-two rows, which is
        // three and a half blocks and something over nine hundred weights.
        let from = run.offset_bits + 30 * 144 * 8;
        let cold = std::time::Instant::now();
        let cells = e.run_cells(&d, BLOCKS, from, from + 512 * 8, 4_000).unwrap();
        let cold = cold.elapsed();
        let warm = std::time::Instant::now();
        e.run_cells(&d, BLOCKS, from, from + 512 * 8, 4_000).unwrap();
        let warm = warm.elapsed();
        eprintln!("run_cells over {} weights of a q4_k tensor: cold {cold:?}, warm {warm:?}", cells.len());
        assert!(cells.len() > 900, "only {} cells over 512 bytes of q4_k", cells.len());
        assert!(cold.as_millis() < 500, "a screenful took {cold:?}");
    }
}
