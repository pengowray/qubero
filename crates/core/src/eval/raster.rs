//! Where each pixel of a [`Ty::Raster`] is.
//!
//! A raster's children are numbered by where they are in the picture and
//! placed by where the run keeps them, and the two orders are not the same
//! for an interlaced image. Both directions are arithmetic, done by
//! [`Geometry`]: from a pixel's number to the bits it is stored in, which is
//! what placing a child asks, and from a bit back to the pixel that holds it,
//! which is what the cursor asks. Neither places any other pixel on the way,
//! so a picture of a million pixels answers for the one under the cursor
//! without a walk.

use std::sync::Arc;

use super::*;
use crate::codec::scanlines::Geometry;
use crate::template::RasterOrder;

// Every function here is kept out of line. Placing a child, counting and
// measuring a node are on the path every nested field recurses through, and a
// frame there that grew by what these hold (a geometry, the words of a
// refusal) would lower how deep a file can nest before the stack runs out.
impl Evaluator {
    /// The shape of the raster at `path`, from its three expressions, worked
    /// out once and kept with the list. The expressions are asked where the
    /// raster is declared, the way an array's count is.
    #[inline(never)]
    pub(super) fn raster_geometry<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> R<Arc<Geometry>> {
        self.resolve(doc, path)?;
        if let Some(g) = &self.list(path).raster {
            return Ok(g.clone());
        }
        let Ty::Raster { width, height, bits_per_pixel, order, .. } = self.memo[path].ty.clone() else {
            return fail("not a raster");
        };
        let w = self.eval_expr(doc, path, &width)?;
        let h = self.eval_expr(doc, path, &height)?;
        let bits = self.eval_expr(doc, path, &bits_per_pixel)?;
        let (Ok(w), Ok(h), Ok(bits)) = (u64::try_from(w), u64::try_from(h), u64::try_from(bits)) else {
            return fail(format!("a picture {w} by {h} at {bits} bits a pixel has no pixels to place"));
        };
        let Some(g) = Geometry::new(w, h, bits, order == RasterOrder::Adam7) else {
            return fail(format!("a picture {w} by {h} at {bits} bits a pixel has no pixels to place"));
        };
        let g = Arc::new(g);
        self.list_mut(path).raster = Some(g.clone());
        Ok(g)
    }

    /// Pixel `idx` of the raster at `parent`: column `idx % width` of row
    /// `idx / width`, at the bits its pass and row put it in. It may read as
    /// far as one pixel reaches and no further.
    #[inline(never)]
    pub(super) fn place_raster<S: Source>(&mut self, doc: &Document<S>, parent: &[usize], pr: &Resolved, idx: usize) -> R<Option<Place>> {
        let Ty::Raster { pixel, .. } = &pr.ty else { return fail("not a raster") };
        let pixel = (**pixel).clone();
        let g = self.raster_geometry(doc, parent)?;
        let (x, y) = (idx as u64 % g.width, idx as u64 / g.width);
        let Some((_, bit)) = g.pixel_bit(x, y) else { return fail("no such pixel") };
        let offset = pr.offset + bit;
        let limit = offset + g.bits_per_pixel;
        if limit > pr.limit {
            return fail("field extends beyond its parent");
        }
        Ok(Some(Place { name: Name::Index(idx), ty: pixel, offset, limit, space: pr.space, machinery: false, elsewhere: false, aside: false }))
    }

    /// How many pixels the raster at `path` has.
    #[inline(never)]
    pub(super) fn raster_count<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> R<u64> {
        let g = self.raster_geometry(doc, path)?;
        match g.width.checked_mul(g.height) {
            Some(n) => Ok(n),
            None => fail("more pixels than can be counted"),
        }
    }

    /// How many bits the raster at `path` covers: every row of every pass,
    /// padding included.
    #[inline(never)]
    pub(super) fn raster_bits<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> R<u64> {
        let g = self.raster_geometry(doc, path)?;
        match g.unfiltered_len().checked_mul(8) {
            Some(n) => Ok(n),
            None => fail("field extends beyond its parent"),
        }
    }

    /// Which pixel of the raster at `path` holds `bit`, by the same
    /// arithmetic run backwards. Nothing for the bits that pad a row out to a
    /// byte, which belong to no pixel.
    #[inline(never)]
    pub(super) fn raster_at<S: Source>(&mut self, doc: &Document<S>, path: &[usize], bit: u64) -> R<Option<usize>> {
        let g = self.raster_geometry(doc, path)?;
        let Some(into) = bit.checked_sub(self.memo[path].offset) else { return Ok(None) };
        Ok(g.pixel_at(into).and_then(|(x, y)| usize::try_from(y * g.width + x).ok()))
    }
}
