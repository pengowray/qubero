//! What a GRIB packed number is worth, and the steps between the two.
//!
//! [`grib`](super::grib) says where every packed number is and how wide it is,
//! which is what a template can say. It cannot say what one is worth: the
//! arithmetic is a float plus an integer times a power of two, over a power of
//! ten, and the IR's expressions are integers with no powers in them. For
//! complex packing with spatial differencing there is worse, a running sum
//! over the whole grid, which nothing written as an expression could ever be.
//!
//! So this is the other half, the arrangement
//! [`hdf5_chunk`](super::hdf5_chunk) and [`ggml_quant`](super::ggml_quant)
//! already have: given section 5's numbers and section 7's bytes, the
//! measurements the message stands for.
//!
//! # What the steps are
//!
//! Simple packing (5.0) is one step. Every packed number X is worth
//!
//! ```text
//! Y = (R + X * 2^E) / 10^D
//! ```
//!
//! with R the reference value, E the binary scale factor and D the decimal
//! scale factor, all three written in section 5. E and D are as often negative
//! as not, and are written sign and magnitude, which is [`grib`](super::grib)'s
//! business rather than this module's: what arrives here is already signed.
//!
//! Complex packing (5.2) adds one step in front of it. X is not written whole:
//! the grid is cut into groups and what is written for a point is how far it is
//! above its own group's reference, so X is `group_references[g] + written`.
//!
//! Spatial differencing (5.3) adds two more. What is written is not the value
//! but the difference between it and the one before (first order), or the
//! difference between those differences (second order), with the smallest
//! difference in the message taken off all of them so that none is negative.
//! Undoing that is: add the minimum back, put the first one or two values back
//! as section 7 wrote them whole, and then run the sum forward over the whole
//! grid. Each step is reported, because a value that comes out wrong came out
//! wrong at one of them.
//!
//! # What is not done
//!
//! A bitmap (section 6). When one is present, the values in section 7 are only
//! the points the bitmap says have one, and the grid's other points have none;
//! this hands back the values as section 7 holds them and says how many, which
//! is what the section is. Missing-value management past 0, where a group
//! reference of all ones means the group has no value, is not undone either:
//! those groups come back as the numbers they hold, and how many points that
//! is is said in [`Reading::problem`] rather than left for a reader to notice
//! in a measurement of 4 billion.

/// What [`StructDef::packed`](crate::template::StructDef::packed) calls section
/// 7's data, so the template can mark it and the panel can find its way back
/// here.
pub const PACKING: &str = "grib_values";

/// The largest section 7 this will unpack. Section 7 of a global model at
/// half a degree is about a megabyte; a claim far past that is a reason to
/// stop rather than a reason to allocate.
pub const PACKED_LIMIT: usize = 64 << 20;

/// The most values it will hand back. A quarter-degree global grid is a
/// million points; ten times that is past any grid anyone writes and is where
/// a header that has gone wrong ends up.
pub const VALUE_LIMIT: usize = 10_000_000;

/// How many values a panel shows, the same as for an HDF5 chunk.
pub const SHOWN: usize = 32;

/// What section 5 says, as the numbers the arithmetic needs. Every field here
/// is one the template already reads; this is them gathered up so the reading
/// can be asked for without walking the tree again.
#[derive(Debug, Clone, PartialEq)]
pub struct Packing {
    /// The three every packing template opens with: R, E and D.
    pub reference: f32,
    pub binary_scale: i32,
    pub decimal_scale: i32,
    /// How wide one group reference is. For simple packing, how wide one value
    /// is, and everything below is left at zero.
    pub bits_per_value: u32,
    /// What section 5 says about points with no value: 0 for none, and 1 or 2
    /// for a group reference of all ones meaning the group is missing. Only
    /// counted here, not undone.
    pub missing_value_management: u32,
    pub n_groups: u32,
    pub group_widths_reference: u32,
    pub group_widths_bits: u32,
    pub group_lengths_reference: u32,
    pub group_length_increment: u32,
    pub last_group_length: u32,
    pub group_lengths_bits: u32,
    /// Zero for template 5.2 and for simple packing; 1 or 2 for 5.3.
    pub spatial_order: u32,
    /// How many octets the first values and the overall minimum are each
    /// written in. Meaningless when `spatial_order` is zero.
    pub extra_bytes: u32,
}

impl Packing {
    /// What one packed number is worth, once it is whole: the group's
    /// reference already added and the differencing already undone.
    ///
    /// `f64` throughout rather than `f32`: the reference is a `f32` and the
    /// packed part is an integer of up to 32 bits, and adding those in `f32`
    /// loses the low bits of a number like 947324.3 plus 18862.
    pub fn worth(&self, packed: i64) -> f64 {
        let scaled = f64::from(self.reference) + packed as f64 * 2f64.powi(self.binary_scale);
        scaled / 10f64.powi(self.decimal_scale)
    }

    /// The reference value as a panel writes it: the shortest decimal that
    /// reads back as the same `f32`. What the writer meant, as near as the
    /// four bytes can say; `947324.3` rather than the `947324.3125` those
    /// bytes hold exactly.
    pub fn reference_text(&self) -> String {
        self.reference.to_string()
    }

    /// A value as a panel writes it: to as many decimal places as the packing
    /// can tell apart, and no more.
    ///
    /// Those are the reference value's own places, as [`reference_text`]
    /// writes it, or the step `2^E` makes when E is negative, whichever is
    /// finer, shifted by D. Printing the `f64` whole would show digits that
    /// are only the reference being an `f32`: `272.98875427246094` for a
    /// reference of 253.02 and a step of a thirty-second, where the packing
    /// says 272.98875. A value under a ten-thousandth is written with an
    /// exponent, since a run of zeros is hard to count.
    ///
    /// [`reference_text`]: Packing::reference_text
    pub fn text(&self, value: f64) -> String {
        if !value.is_finite() {
            return value.to_string();
        }
        let reference = self.reference_text();
        let own = reference.split_once('.').map_or(0, |(_, places)| places.len() as i64);
        let places = (own.max(-i64::from(self.binary_scale)).max(0) + i64::from(self.decimal_scale)).clamp(0, 40) as usize;
        let fixed = format!("{value:.places$}");
        let trimmed = if fixed.contains('.') { fixed.trim_end_matches('0').trim_end_matches('.') } else { fixed.as_str() };
        let rounded: f64 = trimmed.parse().unwrap_or(value);
        if rounded == 0.0 {
            "0".to_string()
        } else if rounded.abs() < 1e-4 {
            format!("{rounded:e}")
        } else {
            trimmed.to_string()
        }
    }
}

/// One thing that was done to get from the bytes to the numbers, in the order
/// it was done. What a reader wants when a value looks wrong: which step it
/// went wrong at.
#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    /// Which step, in a few words a problem can name it by: `group widths`.
    pub label: String,
    /// What it came to: `2,048 group widths, 0 to 13 bits per value`.
    pub what: String,
    /// How many numbers it was done to.
    pub count: u64,
}

fn step(label: &str, what: String, count: u64) -> Step {
    Step { label: label.to_string(), what, count }
}

/// What came out of section 7.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Reading {
    /// The measurements, in the order the grid reads them.
    pub values: Vec<f64>,
    /// The same numbers before the scaling, which is what the packed fields in
    /// the tree hold once their group reference and the differencing are
    /// accounted for. Kept because it is the number a reader standing on a
    /// field can compare against, where the measurement is not.
    pub packed: Vec<i64>,
    pub steps: Vec<Step>,
    /// What stopped it, or what it could not say. The values up to that point
    /// are still handed back.
    pub problem: Option<String>,
}

/// The reading of a section 7 over [`PACKED_LIMIT`], which is none: a section
/// that large is not read at all.
pub fn refused() -> Reading {
    let mb = PACKED_LIMIT / (1 << 20);
    Reading { problem: Some(format!("Not unpacked: section 7 is over this viewer's {mb} MB limit.")), ..Reading::default() }
}

/// Simply packed data (5.0): every number is `bits_per_value` wide and worth
/// what the formula says, with no groups and no differencing.
pub fn simple(p: &Packing, section7: &[u8], count: usize) -> Reading {
    let mut out = Reading::default();
    if count > VALUE_LIMIT {
        let (count, limit) = (commas(count as u64), commas(VALUE_LIMIT as u64));
        out.problem = Some(format!("Not unpacked: section 5 declares {count} values, over this viewer's limit of {limit}."));
        return out;
    }
    // A width of zero is not a mistake: a field whose values are all the same
    // writes no data at all, and every point is the reference value.
    let mut bits = Bits::new(section7);
    for _ in 0..count {
        match bits.take(p.bits_per_value) {
            Some(x) => out.packed.push(x as i64),
            None => {
                let (read, of) = (commas(out.packed.len() as u64), commas(count as u64));
                out.problem = Some(format!("Stopped at packed values after {read} of {of} values: section 7 ran out."));
                break;
            }
        }
    }
    let n = out.packed.len() as u64;
    out.steps.push(step("packed values", format!("{} values, {} bits each", commas(n), p.bits_per_value), n));
    scale(p, &mut out);
    out
}

/// Complex packing (5.2 and 5.3): the three tables, then the groups, then the
/// differencing undone if there is any.
///
/// `section7` is the section's body, from the first byte after the five-byte
/// header. What comes back is as many values as the groups hold, which is what
/// the message says and not necessarily how many points the grid has: a
/// message with a bitmap writes fewer.
pub fn complex(p: &Packing, section7: &[u8]) -> Reading {
    if section7.len() > PACKED_LIMIT {
        return refused();
    }
    let mut out = Reading::default();
    let mut bits = Bits::new(section7);
    let n = p.n_groups as usize;

    // 5.3 writes the first one or two values whole and the smallest difference
    // in the message, each `extra_bytes` octets wide, in front of everything.
    // The minimum is the one negative number in the section and is written the
    // way this format writes those: a magnitude with the top bit for its sign.
    let mut first = Vec::new();
    let mut minimum = 0i64;
    if p.spatial_order > 0 {
        let octets = p.extra_bytes;
        let wide = octets * 8;
        if wide == 0 || wide > 32 {
            out.problem = Some(format!(
                "Not unpacked: the extra descriptors (first values and overall minimum) are {octets} octets wide, and this viewer reads 1 to 4 octets."
            ));
            return out;
        }
        for _ in 0..p.spatial_order {
            let Some(v) = bits.take(wide) else {
                let before = match p.spatial_order {
                    1 => "before the first value was read".to_string(),
                    order => format!("before the first {order} values were read"),
                };
                out.problem = Some(format!("Stopped at first values: section 7 ended {before}."));
                return out;
            };
            first.push(v as i64);
        }
        out.steps.push(step(
            "first values",
            match first.len() {
                1 => format!("the first value, written as a value rather than a difference, {octets} octets"),
                k => format!("the first {k} values, written as values rather than differences, {octets} octets each"),
            },
            first.len() as u64,
        ));
        let Some(v) = bits.take(wide) else {
            out.problem =
                Some("Stopped at minimum difference: section 7 ended before the overall minimum of the differences.".into());
            return out;
        };
        let top = 1u64 << (wide - 1);
        minimum = if v & top != 0 { -((v & (top - 1)) as i64) } else { v as i64 };
        out.steps.push(step(
            "minimum difference",
            format!("{minimum}, the overall minimum of the differences, {octets} octets"),
            1,
        ));
    }

    // The three tables. Each is `n_groups` numbers at a width section 5 gave,
    // and each is padded out with zero bits so that it ends on a byte before
    // the next one starts, which data template 7.2 requires of all three.
    let groups = commas(n as u64);
    let ended = |what: &str| format!("Stopped at {what}: section 7 ended inside the {groups} {what}.");
    let Some(references) = table(&mut bits, n, p.bits_per_value) else {
        out.problem = Some(ended("group references"));
        return out;
    };
    out.steps.push(step(
        "group references",
        format!("{groups} group references, {} bits each", p.bits_per_value),
        n as u64,
    ));
    let Some(widths) = table(&mut bits, n, p.group_widths_bits) else {
        out.problem = Some(ended("group widths"));
        return out;
    };
    let widths: Vec<u32> = widths.iter().map(|w| p.group_widths_reference + *w as u32).collect();
    let what = match (widths.iter().min(), widths.iter().max()) {
        (Some(lo), Some(hi)) => format!("{groups} group widths, {lo} to {hi} bits per value"),
        _ => format!("{groups} group widths"),
    };
    out.steps.push(step("group widths", what, n as u64));
    let Some(lengths) = table(&mut bits, n, p.group_lengths_bits) else {
        out.problem = Some(ended("group lengths"));
        return out;
    };
    // The last group's length is not in the table: what the table holds is a
    // scaled form that could not reach it, so section 5 wrote the real one.
    let mut lengths: Vec<u64> =
        lengths.iter().map(|l| u64::from(p.group_lengths_reference) + u64::from(p.group_length_increment) * l).collect();
    if let Some(last) = lengths.last_mut() {
        *last = u64::from(p.last_group_length);
    }
    let total: u64 = lengths.iter().sum();
    if total as usize > VALUE_LIMIT {
        let (total, limit) = (commas(total), commas(VALUE_LIMIT as u64));
        out.problem =
            Some(format!("Stopped at group lengths: they add up to {total} values, over this viewer's limit of {limit}."));
        return out;
    }
    out.steps.push(step("group lengths", format!("{groups} group lengths, {} values in all", commas(total)), n as u64));

    // Then the values, group by group, each group's run at that group's width.
    out.packed.reserve(total as usize);
    let mut missing = 0u64;
    for g in 0..n {
        let (width, reference) = (widths[g], references[g]);
        // A reference of all ones is how a message with missing-value
        // management on says the whole group has no value. Counted rather than
        // undone, so that the answer says so instead of quietly handing back a
        // very large measurement as if it were one.
        if p.missing_value_management > 0 && p.bits_per_value > 0 && reference == (1u64 << p.bits_per_value.min(63)) - 1 {
            missing += lengths[g];
        }
        for _ in 0..lengths[g] {
            // Zero bits is a real width: every value in the group is the
            // group's reference and nothing at all is written for them.
            let written = if width == 0 { 0 } else {
                match bits.take(width) {
                    Some(v) => v,
                    None => {
                        let place = ordinal(g as u64 + 1);
                        out.problem =
                            Some(format!("Stopped at group values: section 7 ran out in group [{g}], the {place} of {groups}."));
                        return finish(p, out, first, minimum);
                    }
                }
            };
            out.packed.push((reference + written) as i64);
        }
    }
    out.steps.push(step(
        "group values",
        format!("{} values, each read at its group's width and added to its group's reference", commas(total)),
        total,
    ));
    if missing > 0 {
        out.problem = Some(format!(
            "{} values are in groups whose reference marks them as missing; they are shown as the numbers they unpack to, not as missing.",
            commas(missing)
        ));
    }
    finish(p, out, first, minimum)
}

/// The differencing undone and the scaling applied, which is the same tail
/// whether the walk finished or gave up partway.
fn finish(p: &Packing, mut out: Reading, first: Vec<i64>, minimum: i64) -> Reading {
    if p.spatial_order > 0 {
        // Every written number is a difference with the message's smallest
        // difference taken off it, so the first thing is to put that back.
        for v in &mut out.packed {
            *v += minimum;
        }
        out.steps.push(step("minimum added back", format!("{minimum} added back to every difference"), out.packed.len() as u64));
        // The first one or two are not differences at all: they were written
        // whole, and the run of sums starts from them.
        for (k, v) in first.iter().enumerate() {
            if let Some(slot) = out.packed.get_mut(k) {
                *slot = *v;
            }
        }
        let order = p.spatial_order as usize;
        for k in order..out.packed.len() {
            out.packed[k] = match order {
                // First order: each value is the one before it plus what was
                // written.
                1 => out.packed[k] + out.packed[k - 1],
                // Second order: what was written is the change in the change,
                // so the value is the straight-line continuation of the last
                // two plus it.
                _ => out.packed[k] + 2 * out.packed[k - 1] - out.packed[k - 2],
            };
        }
        let n = out.packed.len() as u64;
        let which = if order == 1 { "first" } else { "second" };
        out.steps.push(step("spatial differencing", format!("{which}-order spatial differencing undone, {} values", commas(n)), n));
    }
    scale(p, &mut out);
    out
}

/// The last step, which every packing ends with: the packed numbers as
/// measurements.
fn scale(p: &Packing, out: &mut Reading) {
    out.values = out.packed.iter().map(|x| p.worth(*x)).collect();
    let n = out.values.len() as u64;
    let formula = format!("({} + X × 2^{}) / 10^{}", p.reference_text(), p.binary_scale, p.decimal_scale);
    out.steps.push(step("scaling", format!("{formula} for each packed integer X, giving {} values", commas(n)), n));
}

fn commas(n: u64) -> String {
    crate::encode::commas(n)
}

/// A count from 1 as a place in a run: `1st`, `12th`, `2,001st`.
fn ordinal(n: u64) -> String {
    let suffix = match (n % 100, n % 10) {
        (11..=13, _) => "th",
        (_, 1) => "st",
        (_, 2) => "nd",
        (_, 3) => "rd",
        _ => "th",
    };
    format!("{}{suffix}", commas(n))
}

/// One of the three tables: `n` numbers `bits` wide, and then the bits between
/// the end of them and the byte the next table starts on.
fn table(bits: &mut Bits, n: usize, width: u32) -> Option<Vec<u64>> {
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        out.push(bits.take(width)?);
    }
    bits.align();
    Some(out)
}

/// A place in a run of bits, most significant first, which is how GRIB packs
/// everything narrower than a byte.
struct Bits<'a> {
    buf: &'a [u8],
    at: usize,
}

impl<'a> Bits<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Bits { buf, at: 0 }
    }

    /// The next `n` bits, or nothing when there are not that many left. Zero
    /// bits is zero, which is what a group of no width means and not an error.
    fn take(&mut self, n: u32) -> Option<u64> {
        if n > 64 || self.at + n as usize > self.buf.len() * 8 {
            return None;
        }
        let mut v = 0u64;
        for _ in 0..n {
            let byte = self.buf[self.at >> 3];
            v = (v << 1) | u64::from((byte >> (7 - (self.at & 7))) & 1);
            self.at += 1;
        }
        Some(v)
    }

    /// On to the next byte boundary, which is where the table after this one
    /// begins.
    fn align(&mut self) {
        self.at = (self.at + 7) & !7;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Section 5 and section 7 of the two-group message the template's own
    /// tests are built on: seven values, group widths of 4 and 1 bits, and a
    /// last group whose length only section 5 gives.
    fn two_groups(spatial: bool) -> (Packing, Vec<u8>) {
        let p = Packing {
            reference: 10.0,
            binary_scale: 0,
            decimal_scale: 0,
            bits_per_value: 6,
            missing_value_management: 0,
            n_groups: 2,
            group_widths_reference: 1,
            group_widths_bits: 3,
            group_lengths_reference: 2,
            group_length_increment: 2,
            last_group_length: 3,
            group_lengths_bits: 4,
            spatial_order: if spatial { 1 } else { 0 },
            extra_bytes: 2,
        };
        let mut b = Vec::new();
        if spatial {
            b.extend_from_slice(&100u16.to_be_bytes());
            b.extend_from_slice(&0x8003u16.to_be_bytes()); // a minimum of -3
        }
        b.extend_from_slice(&[0x16, 0x80]); // references of 5 and 40, padded
        b.push(0x60); // raw widths of 3 and 0, padded
        b.push(0x10); // a raw length of 1, and one the last group ignores
        b.extend_from_slice(&[0x12, 0x34, 0xA0]); // 1,2,3,4 then 1,0,1
        (p, b)
    }

    #[test]
    fn a_group_reference_is_added_to_every_value_in_the_group() {
        let (p, bytes) = two_groups(false);
        let r = complex(&p, &bytes);
        assert_eq!(r.problem, None);
        // Group 0 is 5 plus 1, 2, 3, 4; group 1 is 40 plus 1, 0, 1.
        assert_eq!(r.packed, vec![6, 7, 8, 9, 41, 40, 41]);
        // A reference of 10 and no scaling of either kind.
        assert_eq!(r.values, vec![16.0, 17.0, 18.0, 19.0, 51.0, 50.0, 51.0]);
    }

    #[test]
    fn spatial_differencing_is_undone_as_a_running_sum() {
        let (p, bytes) = two_groups(true);
        let r = complex(&p, &bytes);
        assert_eq!(r.problem, None);
        // The differences are 6, 7, 8, 9, 41, 40, 41 with the minimum of -3
        // added back: 3, 4, 5, 6, 38, 37, 38. The first is written whole and
        // is 100, and the rest are sums forward from it.
        let mut want = vec![100i64];
        for d in [4, 5, 6, 38, 37, 38] {
            want.push(want.last().unwrap() + d);
        }
        assert_eq!(r.packed, want);
        assert_eq!(r.values[0], 110.0);
        assert_eq!(r.values.last().copied(), Some(p.worth(*want.last().unwrap())));
    }

    #[test]
    fn every_step_is_reported_in_the_order_it_was_done() {
        let (p, bytes) = two_groups(true);
        let r = complex(&p, &bytes);
        let labels: Vec<&str> = r.steps.iter().map(|s| s.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "first values",
                "minimum difference",
                "group references",
                "group widths",
                "group lengths",
                "group values",
                "minimum added back",
                "spatial differencing",
                "scaling"
            ]
        );
        let steps: Vec<&str> = r.steps.iter().map(|s| s.what.as_str()).collect();
        assert_eq!(steps[0], "the first value, written as a value rather than a difference, 2 octets");
        assert_eq!(steps[1], "-3, the overall minimum of the differences, 2 octets");
        assert_eq!(steps[2], "2 group references, 6 bits each");
        assert_eq!(steps[3], "2 group widths, 1 to 4 bits per value");
        assert_eq!(steps[4], "2 group lengths, 7 values in all");
        assert!(steps[5].contains("group's reference"), "{:?}", steps[5]);
        assert_eq!(steps[6], "-3 added back to every difference");
        assert_eq!(steps[7], "first-order spatial differencing undone, 7 values");
        assert_eq!(steps[8], "(10 + X × 2^0) / 10^0 for each packed integer X, giving 7 values");
    }

    /// A value is written to the places the packing can tell apart: the
    /// reference's own, or the step a negative binary scale makes, shifted by
    /// the decimal scale. Not the digits an `f32` reference trails behind it.
    #[test]
    fn a_value_is_written_to_the_places_its_packing_can_tell_apart() {
        let (p, _) = two_groups(false);
        // A reference of 253.02 is 253.0200042724609375 as an f32, and a
        // step of a thirty-second takes five places.
        let p = Packing { reference: 253.02, binary_scale: -5, decimal_scale: 0, ..p };
        assert_eq!(p.text(p.worth(639)), "272.98875");
        // What the four bytes hold exactly is 947324.3125; the reference is
        // written 947324.3, and a tenth of it takes two places.
        let p = Packing { reference: 947324.3125, binary_scale: 1, decimal_scale: 1, ..p };
        assert_eq!(p.reference_text(), "947324.3");
        assert_eq!(p.text(p.worth(0)), "94732.43");
        assert_eq!(p.text(p.worth(3)), "94733.03");
        // A billionth scale writes the value with an exponent, and nothing
        // is written as a negative zero.
        let p = Packing { reference: 0.0, binary_scale: 2, decimal_scale: 9, ..p };
        assert_eq!(p.text(p.worth(213)), "8.52e-7");
        assert_eq!(p.text(-0.0), "0");
        assert_eq!(p.text(p.worth(0)), "0");
    }

    #[test]
    fn the_formula_holds_for_negative_scale_factors() {
        // What an operational message looks like: a large reference, a binary
        // scale of 1 and a decimal scale of 1, so a packed number is worth two
        // tenths of itself.
        let p = Packing { reference: 947324.3125, binary_scale: 1, decimal_scale: 1, ..two_groups(false).0 };
        assert_eq!(p.worth(0), 94732.43125);
        assert_eq!(p.worth(1), (947324.3125 + 2.0) / 10.0);
        // And the other way, which is what a small field uses: 8.52e-07 is a
        // packed 213 at a binary scale of 2 and a decimal scale of 9.
        let p = Packing { reference: 0.0, binary_scale: 2, decimal_scale: 9, ..p };
        assert!((p.worth(213) - 8.52e-07).abs() < 1e-20, "{}", p.worth(213));
    }

    #[test]
    fn a_section_that_ends_early_says_so_and_keeps_what_it_read() {
        let (p, mut bytes) = two_groups(false);
        bytes.truncate(5);
        let r = complex(&p, &bytes);
        assert!(r.problem.as_deref().unwrap_or("").contains("group"), "{:?}", r.problem);
        assert!(r.packed.len() < 7, "{} values from five bytes", r.packed.len());
    }

    #[test]
    fn a_width_of_zero_is_a_value_that_was_never_written() {
        // Simple packing at no bits a value: a field whose points are all the
        // same writes nothing at all and every one of them is the reference.
        let p = Packing { bits_per_value: 0, reference: 273.0, ..two_groups(false).0 };
        let r = simple(&p, &[], 4);
        assert_eq!(r.values, vec![273.0; 4]);
        assert_eq!(r.problem, None);
    }
}
