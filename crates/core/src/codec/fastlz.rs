//! FastLZ: a byte-oriented LZ77 with no entropy coding behind it.
//!
//! Not written yet. The variant is in [`Codec`](crate::codec::Codec) so that
//! a template can name it and so that everything downstream of one, the
//! listing's Open unpacked, the chips in the hex view and a check over what a
//! run comes to, is already wired; until this reads bytes the run stays bytes
//! and the node says the decoder would not take it.

use crate::codec::{Refusal, Trace};

/// One FastLZ block, with nothing wrapped round it.
pub fn block(_data: &[u8]) -> Result<(Vec<u8>, Trace), Refusal> {
    Err(Refusal::Failed)
}
