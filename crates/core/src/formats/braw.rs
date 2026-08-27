//! Blackmagic RAW (BRAW) container.
//!
//! BRAW uses the QuickTime box model. The public SDK deliberately owns image
//! decoding, so the shared MP4 template exposes its box/index metadata and
//! the known Blackmagic frame records while leaving compressed essence opaque.

use crate::formats::mp4;
use crate::template::Template;

pub fn braw() -> Template {
    mp4::template_named("braw")
}
