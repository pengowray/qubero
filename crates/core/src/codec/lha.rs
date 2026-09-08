//! LHA's `-lh4-` through `-lh7-`: LZSS against a window, under Huffman codes
//! rebuilt every few thousand symbols.
//!
//! Not written yet. The variant is in [`Codec`](crate::codec::Codec) so that
//! a template can name it and so that everything downstream of one, the
//! listing's Open unpacked, the chips in the hex view and a check over what a
//! run comes to, is already wired; until this reads bytes the run stays bytes
//! and the node says the decoder would not take it.

use crate::codec::{Refusal, Trace};

/// One entry's packed data. `window_bits` is how far back a match may reach:
/// 12 for `-lh4-`, 13 for `-lh5-`, 15 for `-lh6-`, 16 for `-lh7-`.
pub fn entry(_data: &[u8], _window_bits: u8) -> Result<(Vec<u8>, Trace), Refusal> {
    Err(Refusal::Failed)
}
