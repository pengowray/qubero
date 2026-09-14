//! NASA CDF's two Huffman codings: one whose tree is built once from counts
//! written in front of the stream, and one whose tree changes after every
//! symbol it codes.
//!
//! Both came into the CDF library in 1996 from the programs in Mark Nelson's
//! *The Data Compression Book*, and neither is the Huffman coding of any other
//! format. There are no code lengths, no canonical codes and no blocks. What
//! decides a code is how the library's tree-building happens to break ties,
//! so the tree here is built the same way step for step, and a tree that came
//! out differently in one tie would read every byte after it as something
//! else. Both read bits from the high end of each byte down, and both end on a
//! symbol of their own, 256, that is not a byte; what follows it to the end of
//! the byte is padding.
//!
//! **Huffman (compression type 2).** The stream opens with the counts, as whole
//! bytes: a first symbol, a last symbol, and a count for each symbol from the
//! one to the other; then another first, last and counts, and so on until a
//! first symbol of zero ends the list. The very first pair is read even when
//! its first symbol is zero, which is how a count for byte 0 gets written at
//! all. Every count is a byte because the encoder scaled them down to fit, so
//! a symbol that occurred once in a million bytes still has a count of one.
//! Symbol 256 always has a count of one and is not written.
//!
//! The tree is built by taking the two lightest nodes, joining them under a
//! new node, and repeating until one is left. "Lightest" is found by scanning
//! every node from the first, keeping a node only when it is strictly lighter
//! than the lightest so far; so of two nodes that weigh the same, the one with
//! the lower number is taken first, and a joined node, which is numbered after
//! every symbol, loses a tie to any symbol. The first node taken is the branch
//! a zero bit follows.
//!
//! **Adaptive Huffman (compression type 3).** Nothing is written in front.
//! Encoder and decoder both start from a tree of two symbols, the end and an
//! escape, and after each byte both add one to that byte's weight and to every
//! node above it, swapping a node that has grown heavier than the one in front
//! of it forward in the list, so the list stays sorted heaviest first and the
//! tree stays a Huffman tree for the counts so far. A byte not yet in the tree
//! is sent as the escape's code and then the byte itself in eight bits, and is
//! then added to the tree beside the lightest node. When the root's weight
//! reaches 32,768, every weight is halved, rounding up, and the tree is built
//! again from its leaves.
//!
//! **What the trace says.** One block of the whole stream, called a dynamic
//! block the way LHA's and deflate's are when the stream brings its own
//! table. For Huffman the counts are one [`StepField::FrequencyTable`] step at
//! its head, whose value is how many byte values were given a count. Then a
//! [`StepKind::Literal`] per byte out, whose bits are its code; symbol 256 is
//! [`StepKind::EndOfBlock`]; and the rest of the run is one
//! [`StepField::Padding`] step. For adaptive Huffman a byte sent through the
//! escape is one literal step covering the escape's code and the eight bits
//! after it, since the two together are how that byte was written.

use crate::bits::Bits;
use crate::codec::{BlockKind, Refusal, StepField, StepKind, Trace, TraceBuilder, CAP_BYTES};

/// The symbol that ends a stream, one past the last byte value.
const END: usize = 256;

/// A stream coded with CDF's Huffman coding, compression type 2.
pub fn huffman(data: &[u8]) -> Result<(Vec<u8>, Trace), Refusal> {
    let byte = |at: usize| data.get(at).map(|&b| b as usize).ok_or(Refusal::Failed);
    // Node weights: the 256 byte values, the end, the joined nodes numbered
    // from 257, and one more at 513 heavier than anything real, which is what
    // the scan for the two lightest starts from.
    let mut weight = [0u32; 514];
    let mut child = [(0u16, 0u16); 514];
    let (mut first, mut last) = (byte(0)?, byte(1)?);
    let mut at = 2;
    loop {
        for sym in first..=last {
            weight[sym] = byte(at)? as u32;
            at += 1;
        }
        first = byte(at)?;
        at += 1;
        if first == 0 {
            break;
        }
        last = byte(at)?;
        at += 1;
    }
    weight[END] = 1;
    let given = weight[..END].iter().filter(|w| **w != 0).count() as u32;
    let root = build_tree(&mut weight, &mut child);
    // A tree whose root is a symbol codes nothing in no bits and would read
    // that symbol forever. No encoder writes one: an empty input is given a
    // count for byte 0 so that there are two leaves.
    if root <= END {
        return Err(Refusal::Failed);
    }

    let mut b = TraceBuilder::default();
    let mut out = Vec::new();
    b.open_block(0, 0);
    b.push(0, 0, StepKind::Header(StepField::FrequencyTable, given));
    let mut bits = Bits::new(data);
    bits.at = at * 8;
    let mut coarse = false;
    loop {
        let from = bits.at as u64;
        if !coarse && b.over_budget() {
            coarse = true;
            b.coarsen();
            b.push(from, out.len() as u64, StepKind::Opaque);
        }
        let mut node = root;
        while node > END {
            let (zero, one) = child[node];
            node = if bits.bit().ok_or(Refusal::Failed)? { one } else { zero } as usize;
        }
        if node == END {
            if !coarse {
                b.push(from, out.len() as u64, StepKind::EndOfBlock);
            }
            break;
        }
        if out.len() >= CAP_BYTES {
            return Err(Refusal::TooLarge);
        }
        if !coarse {
            b.push(from, out.len() as u64, StepKind::Literal(node as u8));
        }
        out.push(node as u8);
    }
    Ok(finish(b, data, bits.at, out))
}

/// Join the two lightest nodes until one is left, and say which that is.
///
/// Weights of nought are nodes no longer in play, either symbols that never
/// occurred or nodes already joined. The scan runs from node 0 up and replaces
/// the lightest only with something strictly lighter, which is the tie-break
/// the encoder used and so the one the codes depend on.
fn build_tree(weight: &mut [u32; 514], child: &mut [(u16, u16); 514]) -> usize {
    const HEAVIEST: usize = 513;
    weight[HEAVIEST] = u32::MAX;
    let mut next = END + 1;
    loop {
        let (mut lightest, mut second) = (HEAVIEST, HEAVIEST);
        for i in 0..next {
            if weight[i] == 0 {
                continue;
            }
            if weight[i] < weight[lightest] {
                second = lightest;
                lightest = i;
            } else if weight[i] < weight[second] {
                second = i;
            }
        }
        if second == HEAVIEST {
            return next - 1;
        }
        weight[next] = weight[lightest] + weight[second];
        weight[lightest] = 0;
        weight[second] = 0;
        child[next] = (lightest as u16, second as u16);
        next += 1;
    }
}

/// The symbol an adaptive stream sends in front of a byte it has not sent
/// before.
const ESCAPE: usize = 257;

/// The nodes an adaptive tree can have: a leaf for each of the 256 bytes, the
/// end and the escape, and one fewer joined nodes than that.
const NODES: usize = 2 * 258 - 1;

/// When the root weighs this much, every weight is halved.
const MAX_WEIGHT: u32 = 0x8000;

#[derive(Clone, Copy, Default)]
struct Node {
    weight: u32,
    /// The node this one hangs from; nothing for the root.
    parent: Option<u16>,
    /// A leaf's symbol, or a joined node's first child, whose sibling is the
    /// next node along.
    child: u16,
    leaf: bool,
}

/// The tree both ends of an adaptive stream keep, in the list order the
/// swapping keeps sorted: node 0 is the root and heaviest.
struct Adaptive {
    nodes: [Node; NODES],
    /// Which node each symbol is, or nothing for a byte not yet sent.
    leaf: [Option<u16>; 258],
    next: usize,
}

impl Adaptive {
    /// A root and two leaves under it, the end first and then the escape, each
    /// weighing one.
    fn new() -> Adaptive {
        let mut t = Adaptive { nodes: [Node::default(); NODES], leaf: [None; 258], next: 3 };
        t.nodes[0] = Node { weight: 2, parent: None, child: 1, leaf: false };
        t.nodes[1] = Node { weight: 1, parent: Some(0), child: END as u16, leaf: true };
        t.nodes[2] = Node { weight: 1, parent: Some(0), child: ESCAPE as u16, leaf: true };
        t.leaf[END] = Some(1);
        t.leaf[ESCAPE] = Some(2);
        t
    }

    /// Walk down from the root a bit at a time to a leaf, and read the byte
    /// after the escape when that is where the walk ended.
    fn decode(&mut self, bits: &mut Bits) -> Result<usize, Refusal> {
        let mut node = 0usize;
        while !self.nodes[node].leaf {
            node = self.nodes[node].child as usize + usize::from(bits.bit().ok_or(Refusal::Failed)?);
        }
        let sym = self.nodes[node].child as usize;
        if sym != ESCAPE {
            return Ok(sym);
        }
        let byte = bits.take(8).ok_or(Refusal::Failed)? as usize;
        // An escape in front of a byte the tree already has is not something
        // the encoder does, and adding it twice would give it two leaves and
        // run the list past the room it has.
        if self.leaf[byte].is_some() {
            return Err(Refusal::Failed);
        }
        // The lightest node is always a leaf, since every joined node weighs
        // at least two and the end symbol weighs one for good; the check is
        // for a tree this has somehow got wrong, which would otherwise take a
        // node's number for a symbol.
        if !self.nodes[self.next - 1].leaf || self.next + 2 > NODES {
            return Err(Refusal::Failed);
        }
        self.add(byte);
        Ok(byte)
    }

    /// A new byte, as a leaf of weight nought. The lightest node, which is
    /// the last in the list, becomes a joined node over two new ones: itself
    /// moved down, and the new leaf after it.
    fn add(&mut self, sym: usize) {
        let lightest = self.next - 1;
        let (moved, new) = (self.next, self.next + 1);
        self.next += 2;
        self.nodes[moved] = Node { parent: Some(lightest as u16), ..self.nodes[lightest] };
        self.leaf[self.nodes[moved].child as usize] = Some(moved as u16);
        self.nodes[lightest].child = moved as u16;
        self.nodes[lightest].leaf = false;
        self.nodes[new] = Node { weight: 0, parent: Some(lightest as u16), child: sym as u16, leaf: true };
        self.leaf[sym] = Some(new as u16);
    }

    /// One more of `sym`: its leaf and every node above it gain a weight, and
    /// a node now heavier than the ones in front of it trades places with the
    /// first of them that it outweighs, taking its subtree with it.
    fn update(&mut self, sym: usize) {
        if self.nodes[0].weight == MAX_WEIGHT {
            self.rebuild();
        }
        let mut node = self.leaf[sym].map(|n| n as usize);
        while let Some(mut at) = node {
            self.nodes[at].weight += 1;
            let mut to = at;
            while to > 0 && self.nodes[to - 1].weight < self.nodes[at].weight {
                to -= 1;
            }
            if to != at {
                self.swap(at, to);
                at = to;
            }
            node = self.nodes[at].parent.map(|n| n as usize);
        }
    }

    /// Two nodes trade places in the list, each keeping the parent of the
    /// place it moves into and taking its children with it.
    fn swap(&mut self, i: usize, j: usize) {
        for (from, to) in [(i, j), (j, i)] {
            let n = self.nodes[from];
            match n.leaf {
                true => self.leaf[n.child as usize] = Some(to as u16),
                false => {
                    self.nodes[n.child as usize].parent = Some(to as u16);
                    self.nodes[n.child as usize + 1].parent = Some(to as u16);
                }
            }
        }
        let (a, b) = (self.nodes[i], self.nodes[j]);
        self.nodes[i] = Node { parent: a.parent, ..b };
        self.nodes[j] = Node { parent: b.parent, ..a };
    }

    /// Halve every leaf's weight, rounding up, and join the leaves again from
    /// the lightest pair up.
    ///
    /// The leaves are gathered at the far end of the list, keeping their
    /// order. Then, from the end, each pair of adjacent nodes is joined into a
    /// node placed as far along the list as its weight allows while the list
    /// stays sorted, the nodes it passes each moving one place forward. Last,
    /// every parent and every leaf's place is written down again from where
    /// the nodes ended up.
    fn rebuild(&mut self) {
        let mut j = self.next as isize - 1;
        for i in (0..self.next).rev() {
            if self.nodes[i].leaf {
                self.nodes[j as usize] = self.nodes[i];
                self.nodes[j as usize].weight = self.nodes[j as usize].weight.div_ceil(2);
                j -= 1;
            }
        }
        let mut i = self.next as isize - 2;
        while j >= 0 {
            let (ju, iu) = (j as usize, i as usize);
            let weight = self.nodes[iu].weight + self.nodes[iu + 1].weight;
            self.nodes[ju].weight = weight;
            self.nodes[ju].leaf = false;
            let mut k = ju + 1;
            while k < NODES && weight < self.nodes[k].weight {
                k += 1;
            }
            k -= 1;
            self.nodes.copy_within(ju + 1..k + 1, ju);
            self.nodes[k].weight = weight;
            self.nodes[k].child = iu as u16;
            self.nodes[k].leaf = false;
            i -= 2;
            j -= 1;
        }
        for i in (0..self.next).rev() {
            let n = self.nodes[i];
            match n.leaf {
                true => self.leaf[n.child as usize] = Some(i as u16),
                false => {
                    self.nodes[n.child as usize].parent = Some(i as u16);
                    self.nodes[n.child as usize + 1].parent = Some(i as u16);
                }
            }
        }
    }
}

/// A stream coded with CDF's adaptive Huffman coding, compression type 3.
pub fn adaptive(data: &[u8]) -> Result<(Vec<u8>, Trace), Refusal> {
    let mut tree = Adaptive::new();
    let mut b = TraceBuilder::default();
    let mut out = Vec::new();
    b.open_block(0, 0);
    let mut bits = Bits::new(data);
    let mut coarse = false;
    loop {
        let from = bits.at as u64;
        if !coarse && b.over_budget() {
            coarse = true;
            b.coarsen();
            b.push(from, out.len() as u64, StepKind::Opaque);
        }
        let sym = tree.decode(&mut bits)?;
        if sym == END {
            if !coarse {
                b.push(from, out.len() as u64, StepKind::EndOfBlock);
            }
            break;
        }
        if out.len() >= CAP_BYTES {
            return Err(Refusal::TooLarge);
        }
        if !coarse {
            b.push(from, out.len() as u64, StepKind::Literal(sym as u8));
        }
        out.push(sym as u8);
        tree.update(sym);
    }
    Ok(finish(b, data, bits.at, out))
}

/// Close the one block at the end symbol, and name whatever is left of the
/// run after it as padding: the rest of the last byte, and any bytes the
/// record holding the stream has room for past that.
fn finish(mut b: TraceBuilder, data: &[u8], at: usize, out: Vec<u8>) -> (Vec<u8>, Trace) {
    let (end, len) = (data.len() as u64 * 8, out.len() as u64);
    if (at as u64) < end {
        b.push(at as u64, len, StepKind::Header(StepField::Padding, 0));
    }
    b.close_block(end, len, BlockKind::Dynamic, true);
    b.finish_at(end, len);
    (out, b.done())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    /// Streams written by the CDF 2.7 library's own `CompressHUFF0` and
    /// `CompressAHUFF0`, compiled from the distribution's `cdfhuff.c` and run
    /// over each input on 2026-09-14. Short enough to read by eye: the counts
    /// in front of `abracadabra` are `20 20 01`, `2c 2c 01`, `61 64 0a 04 02
    /// 02`, `72 72 04` and a closing `00`.
    const LIBRARY: &[(&[u8], &str, &str)] = &[
        (b"", "0000010080", "00"),
        (b"A", "4141010040", "a0c0"),
        (b"abracadabra, abracadabra", "2020012c2c0161640a040202727204005c972e7fcb92e5c8", "b08c4395b1bb20a025824026300b7000"),
        (b"\0\0\0\0\0\0\0\0\x01\x38", "0001080138380100ff4c", "80304039c4"),
    ];

    #[test]
    fn what_the_cdf_library_wrote_reads_as_what_went_in() {
        for (want, huff, ahuff) in LIBRARY {
            for (packed, read) in [(hex(huff), huffman as fn(&[u8]) -> _), (hex(ahuff), adaptive)] {
                let (out, trace) = read(&packed).unwrap_or_else(|e| panic!("{packed:02x?}: {e:?}"));
                assert_eq!(&out, want);
                trace.check_tiles().expect("the trace tiles");
                assert_eq!(trace.in_bits(), packed.len() as u64 * 8);
                assert_eq!(trace.out_bytes(), want.len() as u64);
            }
        }
    }

    /// The counts, a literal per byte, the end, and the padding after it.
    #[test]
    fn a_huffman_stream_is_its_counts_then_a_code_per_byte() {
        let packed = hex(LIBRARY[2].1);
        let (_, trace) = huffman(&packed).unwrap();
        let steps: Vec<_> = trace.steps().collect();
        // Seven byte values have counts: space, comma, a, b, c, d and r, in
        // sixteen bytes of runs and the zero that ends them.
        assert_eq!(steps[0].kind, StepKind::Header(StepField::FrequencyTable, 7));
        assert_eq!(steps[0].in_bits, 0..16 * 8);
        assert_eq!(steps[1].kind, StepKind::Literal(b'a'));
        assert_eq!(steps.iter().filter(|s| matches!(s.kind, StepKind::Literal(_))).count(), 24);
        // This one's end code finishes its last byte exactly, so there is no
        // padding after it; the empty stream's has seven bits of it.
        assert_eq!(steps.last().unwrap().kind, StepKind::EndOfBlock);
        let (_, empty) = huffman(&hex(LIBRARY[0].1)).unwrap();
        let tail: Vec<_> = empty.steps().skip(1).map(|s| (s.kind, s.in_bits)).collect();
        assert_eq!(tail, [(StepKind::EndOfBlock, 32..33), (StepKind::Header(StepField::Padding, 0), 33..40)]);
        assert_eq!(trace.blocks().len(), 1);
        // The commonest byte has the shortest code.
        let a = steps.iter().find(|s| s.kind == StepKind::Literal(b'a')).unwrap();
        let comma = steps.iter().find(|s| s.kind == StepKind::Literal(b',')).unwrap();
        assert!(a.in_bits.end - a.in_bits.start < comma.in_bits.end - comma.in_bits.start);
    }

    /// Of two nodes that weigh the same, the lower-numbered is joined first
    /// and takes the zero branch. Counts of one for 'x' and 'y' and the end
    /// make 'x' and 'y' the first pair, and the end joins them after.
    #[test]
    fn a_tie_goes_to_the_lower_numbered_node() {
        let mut weight = [0u32; 514];
        let mut child = [(0u16, 0u16); 514];
        weight[b'x' as usize] = 1;
        weight[b'y' as usize] = 1;
        weight[END] = 1;
        let root = build_tree(&mut weight, &mut child);
        assert_eq!(child[257], (b'x' as u16, b'y' as u16));
        assert_eq!(root, 258);
        assert_eq!(child[258], (END as u16, 257));
    }

    /// An adaptive tree written by an encoder that shares nothing with the
    /// decoder but the arithmetic: the code for a symbol is the path up from
    /// its leaf, a node with an even number being the one branch.
    fn encode_adaptive(data: &[u8]) -> Vec<u8> {
        let mut tree = Adaptive::new();
        let mut bits: Vec<bool> = Vec::new();
        let send = |tree: &mut Adaptive, bits: &mut Vec<bool>, sym: usize| {
            let known = tree.leaf[sym];
            let mut node = known.unwrap_or_else(|| tree.leaf[ESCAPE].unwrap()) as usize;
            let mut path = Vec::new();
            while node != 0 {
                path.push(node % 2 == 0);
                node = tree.nodes[node].parent.unwrap() as usize;
            }
            bits.extend(path.iter().rev());
            if known.is_none() {
                bits.extend((0..8).rev().map(|i| (sym >> i) & 1 == 1));
                tree.add(sym);
            }
        };
        for &byte in data {
            send(&mut tree, &mut bits, byte as usize);
            tree.update(byte as usize);
        }
        send(&mut tree, &mut bits, END);
        bits.chunks(8).map(|c| c.iter().enumerate().fold(0u8, |v, (i, &b)| v | (u8::from(b) << (7 - i)))).collect()
    }

    /// The encoder above agrees with the library on the short streams, and
    /// then carries the check past what they reach: 100,000 bytes, so the
    /// root reaches 32,768 and the tree is halved and rebuilt, more than once.
    #[test]
    fn an_adaptive_stream_survives_the_tree_being_rebuilt() {
        for (want, _, ahuff) in LIBRARY {
            assert_eq!(encode_adaptive(want), hex(ahuff));
        }
        let mut seed = 0x9e37_79b9u64;
        let data: Vec<u8> = (0..100_000)
            .map(|i| {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                // Skewed, so the weights differ and the swaps have work to do.
                if i % 3 == 0 { (seed % 251) as u8 } else { (seed % 7) as u8 }
            })
            .collect();
        let packed = encode_adaptive(&data);
        let (out, trace) = adaptive(&packed).expect("reads");
        assert_eq!(out, data);
        trace.check_tiles().expect("tiles");
    }

    #[test]
    fn broken_streams_are_refused_rather_than_panicking() {
        // Counts and no codes after them.
        assert!(huffman(&hex("2020010000")).is_err());
        // Counts that end before the list does.
        assert!(huffman(&hex("2021")).is_err());
        // Only symbol 256 has a count, so the root is a symbol.
        assert!(huffman(&hex("000100000000")).is_err());
        assert!(adaptive(b"").is_err());
        // 'A' sent through the escape as it should be, `1` and `01000001`,
        // and then the escape's new code `00` in front of 'A' a second time.
        assert_eq!(adaptive(&hex("a08820")).err(), Some(Refusal::Failed));
        for (_, huff, ahuff) in LIBRARY {
            for (packed, read) in [(hex(huff), huffman as fn(&[u8]) -> _), (hex(ahuff), adaptive)] {
                for n in 0..packed.len() {
                    let _ = read(&packed[..n]);
                }
            }
        }
    }
}
