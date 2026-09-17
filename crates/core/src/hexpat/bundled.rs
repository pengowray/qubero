//! The ImHex patterns shipped inside Qubero.
//!
//! The ImHex-Patterns repository as a whole is GPL-2.0, so almost none of it
//! can be copied in here. What can is the handful of files that carry a licence
//! of their own in the header: `gltf.hexpat` under the MIT licence, and
//! `bink_container.hexpat`, `vhd.hexpat` and `fs/mbr.hexpat` under the MPL-2.0.
//! Those four are in `crates/core/formats-hexpat`, byte for byte, and are
//! compiled in with `include_str!` and converted on demand. Everything else the
//! library offers is fetched when a reader asks for it and is never stored:
//! that is `web/src/hexpatlibrary.ts`.
//!
//! The include files (`includes/std/*.pat`, `includes/type/*.pat`) are GPL-2.0
//! too and are not here either. A bundled pattern that imports one gets the
//! declaration-only table in [`super::includes::builtin`] instead, which is
//! written out fresh and says only what the interface is.
//!
//! A bundled pattern is opened by a name like `hexpat:vhd`, and once converted
//! it is a [`Template`](crate::template::Template) like any other.
//!
//! The table below is generated. `node tools/hexpat_bundle.mjs` rewrites it and
//! `crates/core/formats-hexpat/README.md` from the directory.

use super::includes::Includes;
use super::lexer::HexpatError;
use super::lower::Converted;

/// One shipped `.hexpat`.
pub struct Bundled {
	/// The file's stem, which is also what `hexpat:<id>` opens.
	pub id: &'static str,
	/// The id with [`PREFIX`] on the front, which is the name a caller opens
	/// this by. Written out rather than joined, so it can be a `&'static str`.
	pub name: &'static str,
	/// `#pragma description`, empty for a file that carries none. Nothing
	/// invents one: a pattern with no description is shown by its id.
	pub title: &'static str,
	/// `#pragma author`, empty for a file that carries none.
	pub author: &'static str,
	/// The licence in the file's own header, as an SPDX identifier.
	pub licence: &'static str,
	/// Where this file came from, relative to the root of the ImHex-Patterns
	/// repository. Also what an import naming a path resolves against.
	pub source: &'static str,
	/// The file, byte for byte.
	pub text: &'static str,
	/// Whether a reader may open a file as this. False for a file that is here
	/// only because another one imports it.
	pub offered: bool,
	/// What `#pragma magic` says a file of this format opens with, as
	/// `(offset, bytes)`, counted from the front. Empty where the pattern
	/// declares none, and empty for a magic measured back from the end of the
	/// file: `vhd.hexpat` writes `@ -0x0200` and the converter reads those
	/// bytes where they are, but the sniffer has no table of end-anchored
	/// signatures to put it in, only `sniff_ends`, whose one tail rule is the
	/// ZIP central directory and is written by hand.
	///
	/// This is what the template search matches a typed-in byte pattern
	/// against. Nothing claims a dropped file by it: the built-in probes and
	/// the Kaitai collection already answer that question, and a pattern
	/// language's own idea of a magic is not evidence they lack.
	pub signature: &'static [(u64, &'static [u8])],
}

/// The name a bundled pattern is opened by. `hexpat:` and the file's stem.
pub const PREFIX: &str = "hexpat:";

/// The whole shipped collection, the import-only files included.
pub fn all() -> &'static [Bundled] {
	BUNDLED
}

/// The ids a reader may open a file as.
pub fn names() -> Vec<&'static str> {
	BUNDLED.iter().filter(|b| b.offered).map(|b| b.id).collect()
}

/// One entry by its id, whether or not it is offered.
pub fn find(id: &str) -> Option<&'static Bundled> {
	BUNDLED.iter().find(|b| b.id == id)
}

/// Convert a bundled pattern, its imports resolved against the rest of the
/// collection and then against the built-in declarations. `None` for an id
/// nothing here has.
///
/// The conversion happens now rather than at build time: the IR is Rust and is
/// never serialised, so there is nothing to bake.
///
/// The report comes back with the template because it is half the answer. Every
/// one of these patterns says things the IR cannot, and the panel shows each of
/// them rather than letting the template look complete.
pub fn template(id: &str) -> Option<Result<Converted, HexpatError>> {
	let entry = find(id)?;
	Some(super::convert_named(entry.id, entry.text, &BundledIncludes))
}

/// Resolve an `#include` or an `import` against the bundled collection.
///
/// `import * from fs.mbr` reaches here as `fs/mbr`, and the shipped
/// `fs/mbr.hexpat` is what answers it. A path nothing here has falls through to
/// the built-in declarations, which is what the resolver behind this does.
pub struct BundledIncludes;

impl Includes for BundledIncludes {
	fn load(&self, path: &str) -> Option<String> {
		let want = path.replace('\\', "/");
		let want = want.trim_start_matches('/');
		let stem = |p: &str| p.trim_end_matches(".hexpat").trim_end_matches(".pat").to_string();
		let want = stem(want);
		BUNDLED
			.iter()
			.find(|b| {
				let source = stem(b.source.trim_start_matches("patterns/"));
				source == want || source.ends_with(&format!("/{want}")) || b.id == want
			})
			.map(|b| b.text.to_string())
	}
}

// BEGIN GENERATED by tools/hexpat_bundle.mjs -- do not edit by hand
pub const BUNDLED: &[Bundled] = &[
	Bundled {
		id: "bink_container",
		name: "hexpat:bink_container",
		title: "Bink Video Container - RAD Game Tools",
		author: "Xavier \"XJDHDR\" du Hecquet de Rauville",
		licence: "MPL-2.0",
		source: "patterns/bink_container.hexpat",
		text: include_str!("../../formats-hexpat/bink_container.hexpat"),
		offered: true,
		signature: &[(0, &[0x42, 0x49, 0x4b])],
	},
	Bundled {
		id: "mbr",
		name: "hexpat:mbr",
		title: "Master boot record - IBM",
		author: "Xavier \"XJDHDR\" du Hecquet de Rauville",
		licence: "MPL-2.0",
		source: "patterns/fs/mbr.hexpat",
		text: include_str!("../../formats-hexpat/fs/mbr.hexpat"),
		offered: true,
		signature: &[(510, &[0x55, 0xaa])],
	},
	Bundled {
		id: "gltf",
		name: "hexpat:gltf",
		title: "GL Transmission Format binary 3D model (.glb)",
		author: "H. Utku Maden, xZise",
		licence: "MIT",
		source: "patterns/gltf.hexpat",
		text: include_str!("../../formats-hexpat/gltf.hexpat"),
		offered: true,
		signature: &[(0, &[0x67, 0x6c, 0x54, 0x46])],
	},
	Bundled {
		id: "vhd",
		name: "hexpat:vhd",
		title: "Virtual Hard Disk file - Connectix / Microsoft",
		author: "Xavier \"XJDHDR\" du Hecquet de Rauville",
		licence: "MPL-2.0",
		source: "patterns/vhd.hexpat",
		text: include_str!("../../formats-hexpat/vhd.hexpat"),
		offered: true,
		signature: &[],
	},
];
// END GENERATED
