//! The `.ksy` format description as a model, one `from_yaml` per the Kaitai
//! compiler's `format/*.scala`.
//!
//! This is the strict tier: a `.ksy` the compiler would reject is rejected
//! here, at the same path and mostly in the same words. An unknown key is an
//! error, and which keys are legal depends on what the field turned out to be,
//! so `type: u4` with a `size:` is an error the same way it is there. Keys
//! starting with `-` are the exception the compiler makes and they are kept:
//! `-webide-representation` and `-orig-id` are read later.
//!
//! What this deliberately does *not* do is resolve names. A type reference, an
//! enum reference and every name inside an expression are read and left as
//! written; whether they point at anything is the lowering's question, and the
//! Kaitai compiler asks it in a separate pass too.

use std::fmt;

use crate::template::Endian;

use super::expr::{Expr, TypeId, parse as parse_expr, parse_list as parse_expr_list, parse_type_ref};
use super::yaml::{KsyError, MapEntry, YamlNode, YamlValue};

/// A key the compiler ignores, kept here because something downstream reads it.
pub type VendorKeys = Vec<(String, YamlNode)>;

// ---------------------------------------------------------------------------
// The model
// ---------------------------------------------------------------------------

/// One type: the top-level format, or any entry under `types`.
#[derive(Debug, Clone, PartialEq)]
pub struct ClassSpec {
	pub path: String,
	pub is_top_level: bool,
	/// This type's own `meta`, with `endian`, `bit-endian` and `encoding`
	/// already filled in from the enclosing type where it did not say.
	pub meta: MetaSpec,
	pub doc: DocSpec,
	pub to_string_expr: Option<Expr>,
	pub params: Vec<ParamSpec>,
	pub seq: Vec<AttrSpec>,
	/// Nested types, in the order the file wrote them.
	pub types: Vec<(String, ClassSpec)>,
	pub instances: Vec<(String, InstanceSpec)>,
	pub enums: Vec<(String, EnumSpec)>,
	pub vendor: VendorKeys,
}

/// `meta`.
#[derive(Debug, Clone, PartialEq)]
pub struct MetaSpec {
	pub path: String,
	pub id: Option<String>,
	pub endian: Option<Endianness>,
	pub bit_endian: Option<Endian>,
	pub encoding: Option<String>,
	pub imports: Vec<String>,
	pub ks_version: Option<String>,
	pub ks_debug: bool,
	pub ks_opaque_types: Option<bool>,
	pub ks_zero_copy_substream: Option<bool>,
	pub title: Option<String>,
	pub license: Option<String>,
	pub file_extension: Vec<String>,
	pub application: Vec<String>,
	pub tags: Vec<String>,
	/// `xref`, kept as written: it is a bag of identifiers in other databases
	/// and has no fixed shape.
	pub xref: Option<YamlNode>,
	pub vendor: VendorKeys,
}

/// What `meta/endian` said.
#[derive(Debug, Clone, PartialEq)]
pub enum Endianness {
	Fixed(Endian),
	/// `endian: {switch-on: ..., cases: ...}`: chosen while reading the file.
	Calc(CalcEndian),
	/// Not said here, and the enclosing type chooses per file.
	Inherited,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CalcEndian {
	pub on: Expr,
	pub cases: Vec<(Expr, Endian)>,
}

/// `doc` and `doc-ref`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DocSpec {
	pub summary: Option<String>,
	pub refs: Vec<DocRef>,
}

impl DocSpec {
	pub fn is_empty(&self) -> bool {
		self.summary.is_none() && self.refs.is_empty()
	}
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocRef {
	/// A link, with the words after the URL as its text.
	Url { url: String, text: String },
	Text(String),
}

/// One entry of `seq`, and the parsing half of a `pos` instance.
#[derive(Debug, Clone, PartialEq)]
pub struct AttrSpec {
	pub path: String,
	/// `id`, or `None` where the field was written without one and is known by
	/// its place in `seq`.
	pub id: Option<String>,
	/// Position in `seq`, which is the name of an unnamed field.
	pub index: usize,
	pub ty: TypeRef,
	pub size: Option<Expr>,
	pub size_eos: bool,
	/// `encoding` as written on this field; the effective one is inside
	/// [`TypeRef::Str`].
	pub encoding: Option<String>,
	/// The bytes a `strz` or a terminated field stops at, after the default
	/// for the encoding has been applied.
	pub terminator: Option<Vec<u8>>,
	pub include: bool,
	pub consume: bool,
	pub eos_error: bool,
	pub pad_right: Option<u8>,
	pub contents: Option<Vec<u8>>,
	pub enum_ref: Option<String>,
	pub parent: Option<Expr>,
	pub process: Option<ProcessSpec>,
	pub if_expr: Option<Expr>,
	pub repeat: RepeatSpec,
	pub valid: Option<ValidSpec>,
	pub doc: DocSpec,
	pub vendor: VendorKeys,
}

impl AttrSpec {
	/// What to call this field: its `id`, or its place in `seq`.
	pub fn name(&self) -> String {
		match &self.id {
			Some(id) => id.clone(),
			None => format!("_unnamed{}", self.index),
		}
	}
}

/// Where a field's bytes come from, before anything is made of them.
#[derive(Debug, Clone, PartialEq)]
pub enum ByteSource {
	/// `size: e`.
	Limit(Expr),
	/// `size-eos: true`.
	Eos,
	/// Neither, so the `terminator` ends it.
	Terminated,
}

/// What `type:` said, after the type-string grammar.
#[derive(Debug, Clone, PartialEq)]
pub enum TypeRef {
	/// No `type:` at all: the bytes themselves.
	Bytes { source: ByteSource },
	/// `u1`..`u8`, `s1`..`s8`, with `le`/`be` from the suffix or from `meta`.
	/// `endian` is `None` only where `meta/endian` is chosen per file.
	Int { signed: bool, width: u8, endian: Option<Endian> },
	/// `f4`, `f8`.
	Float { width: u8, endian: Option<Endian> },
	/// `bN`, with `le`/`be` from the suffix, from `meta/bit-endian`, or big.
	Bits { width: u32, endian: Endian },
	/// `str` or `strz`, with the encoding resolved.
	Str { zero_terminated: bool, encoding: String, source: ByteSource },
	/// A user type read straight from the stream.
	User { name: TypeId, args: Vec<Expr> },
	/// A user type read out of a slice of bytes, because the field also said
	/// `size`, `size-eos` or `terminator`.
	UserFromBytes { name: TypeId, args: Vec<Expr>, source: ByteSource },
	/// `type: {switch-on: ..., cases: ...}`.
	Switch(Box<SwitchSpec>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct SwitchSpec {
	pub on: Expr,
	pub cases: Vec<SwitchCase>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SwitchCase {
	/// The case key, which is an expression: an integer, an enum label, a
	/// string, or the bare `_` that means every other value.
	pub key: Expr,
	/// The key as the file wrote it, for messages and the report.
	pub key_text: String,
	pub path: String,
	pub ty: TypeRef,
	/// The `_` case the compiler adds itself when the field has a `size` and
	/// the file named no default.
	pub implicit: bool,
}

impl SwitchCase {
	/// Whether this is the `_` case.
	pub fn is_else(&self) -> bool {
		matches!(&self.key, Expr::Name(name) if name == "_")
	}
}

/// `instances`.
#[derive(Debug, Clone, PartialEq)]
pub enum InstanceSpec {
	/// `value:` — computed, never read from the file.
	Value(ValueInstanceSpec),
	/// Anything else — read from the file, usually at a `pos:`.
	Parse(ParseInstanceSpec),
}

impl InstanceSpec {
	pub fn path(&self) -> &str {
		match self {
			InstanceSpec::Value(v) => &v.path,
			InstanceSpec::Parse(p) => &p.path,
		}
	}

	pub fn name(&self) -> &str {
		match self {
			InstanceSpec::Value(v) => &v.name,
			InstanceSpec::Parse(p) => &p.name,
		}
	}

	pub fn doc(&self) -> &DocSpec {
		match self {
			InstanceSpec::Value(v) => &v.doc,
			InstanceSpec::Parse(p) => &p.attr.doc,
		}
	}
}

#[derive(Debug, Clone, PartialEq)]
pub struct ValueInstanceSpec {
	pub path: String,
	pub name: String,
	pub value: Expr,
	pub if_expr: Option<Expr>,
	pub enum_ref: Option<String>,
	pub doc: DocSpec,
	pub vendor: VendorKeys,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParseInstanceSpec {
	pub path: String,
	pub name: String,
	/// Everything a `seq` field has, read the same way.
	pub attr: AttrSpec,
	pub pos: Option<Expr>,
	/// Which stream to read in. `_root._io` and `_io` are the ones the corpus
	/// uses; the rest the lowering has to refuse.
	pub io: Option<Expr>,
}

/// `enums/<name>`.
#[derive(Debug, Clone, PartialEq)]
pub struct EnumSpec {
	pub path: String,
	pub values: Vec<EnumValueSpec>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EnumValueSpec {
	pub path: String,
	pub value: i128,
	pub name: String,
	pub doc: DocSpec,
	pub vendor: VendorKeys,
}

/// One entry of `params`.
#[derive(Debug, Clone, PartialEq)]
pub struct ParamSpec {
	pub path: String,
	pub id: Option<String>,
	pub index: usize,
	pub ty: ParamType,
	pub enum_ref: Option<String>,
	pub doc: DocSpec,
	pub vendor: VendorKeys,
}

impl ParamSpec {
	pub fn name(&self) -> String {
		match &self.id {
			Some(id) => id.clone(),
			None => format!("_unnamed{}", self.index),
		}
	}
}

/// A `params` type, which uses a smaller grammar than a `seq` field does:
/// there is no stream here, so no endianness and no size. `u4le` written as a
/// parameter type is a user type called `u4le`, and that is the compiler's
/// reading too.
#[derive(Debug, Clone, PartialEq)]
pub enum ParamType {
	Bytes,
	Int { signed: bool, width: u8 },
	Float { width: u8 },
	Bits { width: u32 },
	Str,
	Bool,
	Struct,
	Io,
	Any,
	User(TypeId),
	Array(Box<ParamType>),
}

/// `valid`.
#[derive(Debug, Clone, PartialEq)]
pub enum ValidSpec {
	Eq(Expr),
	Min(Expr),
	Max(Expr),
	Range { min: Expr, max: Expr },
	AnyOf(Vec<Expr>),
	InEnum,
	Expr(Expr),
}

/// `repeat`.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum RepeatSpec {
	#[default]
	No,
	/// `repeat: eos`.
	Eos,
	/// `repeat: expr` with `repeat-expr`.
	Expr(Expr),
	/// `repeat: until` with `repeat-until`.
	Until(Expr),
}

/// `process`.
#[derive(Debug, Clone, PartialEq)]
pub enum ProcessSpec {
	Zlib,
	Xor(Expr),
	Rotate { left: bool, key: Expr },
	Custom { name: Vec<String>, args: Vec<Expr> },
}

// ---------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------

const ID_PATTERN: &str = "/^[a-z][a-z0-9_]*$/";

/// The identifier rule the compiler enforces everywhere a name is written.
fn is_identifier(s: &str) -> bool {
	let mut chars = s.chars();
	match chars.next() {
		Some(c) if c.is_ascii_lowercase() => {}
		_ => return false,
	}
	chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

fn check_identifier(id: &str, entity: &str, path: &str) -> Result<(), KsyError> {
	if is_identifier(id) {
		return Ok(());
	}
	Err(KsyError::new(path, format!("invalid {entity} ID: '{id}', expected {ID_PATTERN}")))
}

/// Every key of a map, minus the ones starting with `-`, has to be in `legal`.
/// The `-` keys are handed back rather than dropped.
fn legal_keys(
	node: &YamlNode,
	legal: &[&str],
	ignore: &[&str],
	context: Option<&str>,
) -> Result<VendorKeys, KsyError> {
	let mut vendor = Vec::new();
	for entry in node.as_map()? {
		let key = entry.key_text.as_str();
		if key.starts_with('-') {
			vendor.push((key.to_string(), entry.value.clone()));
			continue;
		}
		if legal.contains(&key) || ignore.contains(&key) {
			continue;
		}
		let mut allowed: Vec<&str> = legal.to_vec();
		allowed.sort_unstable();
		let lead = match context {
			Some(ctx) => format!("invalid key found in {ctx}, allowed"),
			None => "unknown key found, expected".to_string(),
		};
		return Err(KsyError::new(node.child_path(key), format!("{lead}: {}", allowed.join(", "))));
	}
	Ok(vendor)
}

fn opt_str(node: &YamlNode, key: &str) -> Result<Option<String>, KsyError> {
	node.get(key).map(YamlNode::as_str).transpose()
}

fn opt_bool(node: &YamlNode, key: &str) -> Result<Option<bool>, KsyError> {
	node.get(key).map(YamlNode::as_bool).transpose()
}

fn opt_byte(node: &YamlNode, key: &str) -> Result<Option<u8>, KsyError> {
	node.get(key).map(YamlNode::as_byte).transpose()
}

/// One expression from a scalar node, with a parse failure named at the node.
fn expr_at(node: &YamlNode) -> Result<Expr, KsyError> {
	let text = node.as_str()?;
	parse_expr(&text).map_err(|e| KsyError::new(node.path.to_string(), e.to_string()))
}

fn opt_expr(node: &YamlNode, key: &str) -> Result<Option<Expr>, KsyError> {
	node.get(key).map(expr_at).transpose()
}

fn value_expr(node: &YamlNode, key: &str) -> Result<Expr, KsyError> {
	match node.get(key) {
		Some(child) => expr_at(child),
		None => Err(KsyError::new(node.path.to_string(), format!("missing mandatory argument `{key}`"))),
	}
}

impl ClassSpec {
	/// Read a whole `.ksy` document.
	pub fn from_document(root: &YamlNode) -> Result<Self, KsyError> {
		ClassSpec::from_yaml(root, "", true, &MetaSpec::empty(""))
	}

	fn from_yaml(
		node: &YamlNode,
		path: &str,
		is_top_level: bool,
		meta_def: &MetaSpec,
	) -> Result<Self, KsyError> {
		let entries = node.as_map()?;
		let vendor = legal_keys(
			node,
			&["meta", "doc", "doc-ref", "to-string", "params", "seq", "types", "instances", "enums"],
			&[],
			None,
		)?;

		let meta_path = format!("{path}/meta");
		let explicit_meta = match node.get("meta") {
			Some(child) => MetaSpec::from_yaml(child, &meta_path)?,
			None => MetaSpec::empty(&meta_path),
		};
		let meta = explicit_meta.fill_in_defaults(meta_def);

		if is_top_level && explicit_meta.id.is_none() {
			return Err(KsyError::new(meta_path, "no `meta/id` encountered in top-level class spec"));
		}

		let doc = DocSpec::from_yaml(node)?;
		let to_string_expr = opt_expr(node, "to-string")?;

		let mut params = Vec::new();
		if let Some(child) = node.get("params") {
			for (index, item) in child.as_seq()?.iter().enumerate() {
				params.push(ParamSpec::from_yaml(item, index)?);
			}
		}

		let mut seq = Vec::new();
		if let Some(child) = node.get("seq") {
			for (index, item) in child.as_seq()?.iter().enumerate() {
				seq.push(AttrSpec::from_yaml(item, &meta, index)?);
			}
		}

		let mut instances = Vec::new();
		if let Some(child) = node.get("instances") {
			for entry in child.as_map()? {
				check_identifier(&entry.key_text, "instance", &child.child_path(&entry.key_text))?;
				instances.push((
					entry.key_text.clone(),
					InstanceSpec::from_yaml(&entry.value, &entry.key_text, &meta)?,
				));
			}
		}

		check_duplicate_names(&params, &seq, &instances)?;

		let mut types = Vec::new();
		if let Some(child) = node.get("types") {
			for entry in child.as_map()? {
				let type_path = child.child_path(&entry.key_text);
				check_identifier(&entry.key_text, "type", &type_path)?;
				types.push((
					entry.key_text.clone(),
					ClassSpec::from_yaml(&entry.value, &type_path, false, &meta)?,
				));
			}
		}

		let mut enums = Vec::new();
		if let Some(child) = node.get("enums") {
			for entry in child.as_map()? {
				let enum_path = child.child_path(&entry.key_text);
				check_identifier(&entry.key_text, "enum", &enum_path)?;
				enums.push((entry.key_text.clone(), EnumSpec::from_yaml(&entry.value, &enum_path)?));
			}
		}

		let _ = entries;
		Ok(ClassSpec {
			path: path.to_string(),
			is_top_level,
			meta,
			doc,
			to_string_expr,
			params,
			seq,
			types,
			instances,
			enums,
			vendor,
		})
	}

	/// This type and every type nested under it, outermost first, each with the
	/// `outer.inner` name the lowering gives it.
	pub fn walk_types(&self, prefix: &str, f: &mut impl FnMut(&str, &ClassSpec)) {
		f(prefix, self);
		for (name, inner) in &self.types {
			let nested =
				if prefix.is_empty() { name.clone() } else { format!("{prefix}.{name}") };
			inner.walk_types(&nested, f);
		}
	}

	/// Call `f` on every expression written anywhere in this type or under it,
	/// with the path of the key that held it.
	///
	/// This is what the report walks, and it is also how the tests find every
	/// expression in a corpus file without reading the YAML a second time.
	pub fn for_each_expr(&self, f: &mut impl FnMut(&str, &Expr)) {
		if let Some(e) = &self.to_string_expr {
			f(&format!("{}/to-string", self.path), e);
		}
		if let Some(Endianness::Calc(calc)) = &self.meta.endian {
			let base = format!("{}/endian", self.meta.path);
			f(&format!("{base}/switch-on"), &calc.on);
			for (key, _) in &calc.cases {
				f(&format!("{base}/cases"), key);
			}
		}
		for attr in &self.seq {
			attr.for_each_expr(f);
		}
		for (_, instance) in &self.instances {
			match instance {
				InstanceSpec::Value(v) => {
					f(&format!("{}/value", v.path), &v.value);
					if let Some(e) = &v.if_expr {
						f(&format!("{}/if", v.path), e);
					}
				}
				InstanceSpec::Parse(p) => {
					if let Some(e) = &p.pos {
						f(&format!("{}/pos", p.path), e);
					}
					if let Some(e) = &p.io {
						f(&format!("{}/io", p.path), e);
					}
					p.attr.for_each_expr(f);
				}
			}
		}
		for (_, inner) in &self.types {
			inner.for_each_expr(f);
		}
	}
}

/// No two members of a type may share a name.
fn check_duplicate_names(
	params: &[ParamSpec],
	seq: &[AttrSpec],
	instances: &[(String, InstanceSpec)],
) -> Result<(), KsyError> {
	let mut seen: Vec<(String, String)> = Vec::new();
	let mut check = |name: Option<String>, path: &str| -> Result<(), KsyError> {
		let Some(name) = name else { return Ok(()) };
		if let Some((_, prev)) = seen.iter().find(|(n, _)| *n == name) {
			return Err(KsyError::new(
				path,
				format!("duplicate attribute ID '{name}', previously defined at {prev}"),
			));
		}
		seen.push((name, path.to_string()));
		Ok(())
	};
	for param in params {
		check(param.id.clone(), &param.path)?;
	}
	for attr in seq {
		check(attr.id.clone(), &attr.path)?;
	}
	for (name, instance) in instances {
		check(Some(name.clone()), instance.path())?;
	}
	Ok(())
}

impl MetaSpec {
	fn empty(path: &str) -> Self {
		MetaSpec {
			path: path.to_string(),
			id: None,
			endian: None,
			bit_endian: None,
			encoding: None,
			imports: Vec::new(),
			ks_version: None,
			ks_debug: false,
			ks_opaque_types: None,
			ks_zero_copy_substream: None,
			title: None,
			license: None,
			file_extension: Vec::new(),
			application: Vec::new(),
			tags: Vec::new(),
			xref: None,
			vendor: Vec::new(),
		}
	}

	/// Take `endian`, `bit-endian` and `encoding` from the enclosing type where
	/// this one did not say. A parent that chooses endianness per file leaves
	/// the child inheriting the choice rather than a fixed answer.
	fn fill_in_defaults(&self, parent: &MetaSpec) -> MetaSpec {
		let mut out = self.clone();
		if out.encoding.is_none() {
			out.encoding = parent.encoding.clone();
		}
		if out.endian.is_none() {
			out.endian = match &parent.endian {
				None => None,
				Some(Endianness::Calc(_)) => Some(Endianness::Inherited),
				Some(other) => Some(other.clone()),
			};
		}
		if out.bit_endian.is_none() {
			out.bit_endian = parent.bit_endian;
		}
		out
	}

	fn from_yaml(node: &YamlNode, path: &str) -> Result<Self, KsyError> {
		let ks_version = opt_str(node, "ks-version")?;
		if let Some(version) = &ks_version {
			check_ks_version(version, &node.child_path("ks-version"))?;
		}
		let endian = match node.get("endian") {
			None => None,
			Some(child) => Some(Endianness::from_yaml(child)?),
		};
		let bit_endian = match node.get("bit-endian") {
			None => None,
			Some(child) => Some(read_endian_word(child, "bit endianness")?),
		};

		let vendor = legal_keys(
			node,
			&[
				"id",
				"imports",
				"endian",
				"bit-endian",
				"encoding",
				"title",
				"ks-version",
				"ks-debug",
				"ks-opaque-types",
				"ks-zero-copy-substream",
				"license",
				"file-extension",
				"xref",
				"tags",
				"application",
			],
			&[],
			None,
		)?;

		let id = opt_str(node, "id")?;
		if let Some(id) = &id {
			check_identifier(id, "meta", &node.child_path("id"))?;
		}

		Ok(MetaSpec {
			path: path.to_string(),
			id,
			endian,
			bit_endian,
			encoding: opt_str(node, "encoding")?,
			imports: node.get_list_str("imports")?,
			ks_version,
			ks_debug: opt_bool(node, "ks-debug")?.unwrap_or(false),
			ks_opaque_types: opt_bool(node, "ks-opaque-types")?,
			ks_zero_copy_substream: opt_bool(node, "ks-zero-copy-substream")?,
			title: opt_str(node, "title")?,
			license: opt_str(node, "license")?,
			file_extension: node.get_list_str("file-extension")?,
			application: node.get_list_str("application")?,
			tags: node.get_list_str("tags")?,
			xref: node.get("xref").cloned(),
			vendor,
		})
	}
}

/// The compiler version this `.ksy` asks for: `X.Y` or `X.Y.Z`, no leading
/// zeros, and 0.6 at the earliest.
fn check_ks_version(version: &str, path: &str) -> Result<(), KsyError> {
	let bad = || {
		KsyError::new(
			path,
			format!(
				"invalid compiler version '{version}', expected 'X.Y' or 'X.Y.Z', \
				 where X, Y, Z are non-negative integers without leading zeros"
			),
		)
	};
	let parts: Vec<&str> = version.split('.').collect();
	if parts.len() < 2 || parts.len() > 3 {
		return Err(bad());
	}
	let mut nums = Vec::new();
	for part in parts {
		if part.is_empty() || (part.len() > 1 && part.starts_with('0')) {
			return Err(bad());
		}
		nums.push(part.parse::<u32>().map_err(|_| bad())?);
	}
	if (nums[0], nums[1]) < (0, 6) {
		let extra = if nums[0] == 0 && nums[1] == 1 {
			" (if you meant 0.10, use ks-version: '0.10' to prevent YAML from interpreting it as a float)"
		} else {
			""
		};
		return Err(KsyError::new(
			path,
			format!("minimum allowed version is 0.6, but got {version}{extra}"),
		));
	}
	Ok(())
}

/// `le` or `be`, and nothing else.
fn read_endian_word(node: &YamlNode, what: &str) -> Result<Endian, KsyError> {
	match node.as_str()?.as_str() {
		"le" => Ok(Endian::Little),
		"be" => Ok(Endian::Big),
		other => Err(KsyError::new(
			node.path.to_string(),
			format!("unable to parse {what}: expected `le` or `be`, found {other}"),
		)),
	}
}

impl Endianness {
	fn from_yaml(node: &YamlNode) -> Result<Self, KsyError> {
		if let YamlValue::Map(_) = node.value {
			let on = value_expr(node, "switch-on")?;
			let mut cases = Vec::new();
			if let Some(child) = node.get("cases") {
				for entry in child.as_map()? {
					let key = parse_expr(&entry.key_text).map_err(|e| {
						KsyError::new(child.child_path(&entry.key_text), e.to_string())
					})?;
					cases.push((key, read_endian_word(&entry.value, "endianness")?));
				}
			}
			legal_keys(node, &["switch-on", "cases"], &[], None)?;
			return Ok(Endianness::Calc(CalcEndian { on, cases }));
		}
		match node.as_str().ok().as_deref() {
			Some("le") => Ok(Endianness::Fixed(Endian::Little)),
			Some("be") => Ok(Endianness::Fixed(Endian::Big)),
			_ => Err(KsyError::new(
				node.path.to_string(),
				"unable to parse endianness: `le`, `be` or calculated endianness map is expected",
			)),
		}
	}
}

impl DocSpec {
	fn from_yaml(node: &YamlNode) -> Result<Self, KsyError> {
		let summary = opt_str(node, "doc")?;
		let refs = node.get_list_str("doc-ref")?.into_iter().map(|r| DocRef::parse(&r)).collect();
		Ok(DocSpec { summary, refs })
	}
}

impl DocRef {
	fn parse(text: &str) -> Self {
		if !text.starts_with("http://") && !text.starts_with("https://") {
			return DocRef::Text(text.to_string());
		}
		match text.find(' ') {
			None => DocRef::Url { url: text.to_string(), text: "Source".to_string() },
			Some(at) => DocRef::Url {
				url: text[..at].trim().to_string(),
				text: text[at + 1..].trim().to_string(),
			},
		}
	}
}

/// Everything a field says about where its bytes come from, which is what the
/// type string is read against.
struct AttrArgs {
	size: Option<Expr>,
	size_eos: bool,
	encoding: Option<String>,
	terminator: Option<Vec<u8>>,
	contents: Option<Vec<u8>>,
	enum_ref: Option<String>,
	parent: Option<Expr>,
	has_process: bool,
}

impl AttrArgs {
	/// `size`, `size-eos` or a terminator, exactly one of them.
	fn byte_source(&self, path: &str) -> Result<ByteSource, KsyError> {
		match (&self.size, self.size_eos) {
			(Some(size), false) => Ok(ByteSource::Limit(size.clone())),
			(None, true) => Ok(ByteSource::Eos),
			(None, false) => match &self.terminator {
				Some(_) => Ok(ByteSource::Terminated),
				None => Err(KsyError::new(
					path,
					"'size', 'size-eos' or 'terminator' must be specified",
				)),
			},
			(Some(_), true) => {
				Err(KsyError::new(path, "only one of 'size' or 'size-eos' must be specified"))
			}
		}
	}
}

impl AttrSpec {
	fn from_yaml(node: &YamlNode, meta: &MetaSpec, index: usize) -> Result<Self, KsyError> {
		let id = opt_str(node, "id")?;
		if let Some(id) = &id {
			check_identifier(id, "attribute", &node.child_path("id"))?;
		}
		AttrSpec::from_map(node, meta, id, index, &[])
	}

	/// The body of a field, wherever it is written. `ignore` names keys that
	/// belong to whatever wrapped it, which is how a `pos` instance keeps its
	/// `pos` and `io` out of the field's own key check.
	fn from_map(
		node: &YamlNode,
		meta: &MetaSpec,
		id: Option<String>,
		index: usize,
		ignore: &[&str],
	) -> Result<Self, KsyError> {
		let path = node.path.to_string();
		let doc = DocSpec::from_yaml(node)?;
		let process = match node.get("process") {
			Some(child) => Some(ProcessSpec::from_str_node(child)?),
			None => None,
		};
		let contents = match node.get("contents") {
			Some(child) => Some(read_contents(child)?),
			None => None,
		};
		let size = opt_expr(node, "size")?;
		let size_eos = opt_bool(node, "size-eos")?.unwrap_or(false);
		let if_expr = opt_expr(node, "if")?;
		let encoding = opt_str(node, "encoding")?;
		let terminator = opt_byte(node, "terminator")?.map(|b| vec![b]);
		let consume = opt_bool(node, "consume")?.unwrap_or(true);
		let include = opt_bool(node, "include")?.unwrap_or(false);
		let eos_error = opt_bool(node, "eos-error")?.unwrap_or(true);
		let pad_right = opt_byte(node, "pad-right")?;
		let enum_ref = opt_str(node, "enum")?;
		let parent = opt_expr(node, "parent")?;
		let valid = match node.get("valid") {
			Some(child) => Some(ValidSpec::from_yaml(child)?),
			None => None,
		};

		// `contents` is the same statement as `valid/eq`, so writing both is
		// writing the same thing twice.
		let valid = match (&contents, valid) {
			(None, valid) => valid,
			(Some(bytes), None) => Some(ValidSpec::Eq(Expr::List(
				bytes.iter().map(|b| Expr::IntNum(i128::from(*b))).collect(),
			))),
			(Some(_), Some(_)) => {
				return Err(KsyError::new(path.clone(), "`contents` and `valid` can't be used together"));
			}
		};

		let mut args = AttrArgs {
			size: size.clone(),
			size_eos,
			encoding: encoding.clone(),
			terminator: terminator.clone(),
			contents: contents.clone(),
			enum_ref: enum_ref.clone(),
			parent: parent.clone(),
			has_process: process.is_some(),
		};

		let ty = match node.get("type") {
			None => type_from_str(None, &path, meta, &mut args)?,
			Some(child) => match &child.value {
				YamlValue::Map(_) => TypeRef::Switch(Box::new(SwitchSpec::from_yaml(child, meta, &mut args)?)),
				YamlValue::Seq(_) | YamlValue::Null => {
					return Err(KsyError::new(
						child.path.to_string(),
						format!("expected map or string, found {}", child.describe()),
					));
				}
				_ => type_from_str(Some(&child.as_str()?), &path, meta, &mut args)?,
			},
		};

		// `strz` picks its terminator from the encoding, so it may have been
		// filled in while the type string was read.
		let terminator = args.terminator.clone();

		let (repeat, repeat_keys) = RepeatSpec::from_yaml(node)?;

		let mut legal: Vec<&str> = vec![
			"id", "doc", "doc-ref", "type", "if", "terminator", "consume", "include", "eos-error",
			"valid", "repeat",
		];
		legal.extend(repeat_keys);
		legal.extend(legal_keys_for(&ty, enum_ref.is_some()));
		let vendor = legal_keys(node, &legal, ignore, None)?;

		Ok(AttrSpec {
			path,
			id,
			index,
			ty,
			size,
			size_eos,
			encoding,
			terminator,
			include,
			consume,
			eos_error,
			pad_right,
			contents,
			enum_ref,
			parent,
			process,
			if_expr,
			repeat,
			valid,
			doc,
			vendor,
		})
	}

	/// Every expression written on this field.
	pub fn for_each_expr(&self, f: &mut impl FnMut(&str, &Expr)) {
		if let Some(e) = &self.size {
			f(&format!("{}/size", self.path), e);
		}
		if let Some(e) = &self.if_expr {
			f(&format!("{}/if", self.path), e);
		}
		if let Some(e) = &self.parent {
			f(&format!("{}/parent", self.path), e);
		}
		match &self.repeat {
			RepeatSpec::Expr(e) => f(&format!("{}/repeat-expr", self.path), e),
			RepeatSpec::Until(e) => f(&format!("{}/repeat-until", self.path), e),
			_ => {}
		}
		if let Some(process) = &self.process {
			let path = format!("{}/process", self.path);
			match process {
				ProcessSpec::Zlib => {}
				ProcessSpec::Xor(e) | ProcessSpec::Rotate { key: e, .. } => f(&path, e),
				ProcessSpec::Custom { args, .. } => {
					for arg in args {
						f(&path, arg);
					}
				}
			}
		}
		if let Some(valid) = &self.valid {
			let path = format!("{}/valid", self.path);
			match valid {
				ValidSpec::Eq(e) | ValidSpec::Min(e) | ValidSpec::Max(e) | ValidSpec::Expr(e) => {
					f(&path, e);
				}
				ValidSpec::Range { min, max } => {
					f(&path, min);
					f(&path, max);
				}
				ValidSpec::AnyOf(values) => {
					for value in values {
						f(&path, value);
					}
				}
				ValidSpec::InEnum => {}
			}
		}
		type_for_each_expr(&self.ty, &self.path, f);
	}
}

fn type_for_each_expr(ty: &TypeRef, path: &str, f: &mut impl FnMut(&str, &Expr)) {
	match ty {
		TypeRef::User { args, .. } | TypeRef::UserFromBytes { args, .. } => {
			for arg in args {
				f(&format!("{path}/type"), arg);
			}
		}
		TypeRef::Switch(switch) => {
			f(&format!("{path}/type/switch-on"), &switch.on);
			for case in &switch.cases {
				if !case.implicit {
					f(&case.path, &case.key);
				}
				type_for_each_expr(&case.ty, path, f);
			}
		}
		_ => {}
	}
}

/// Which extra keys a field may write, given what its type turned out to be.
fn legal_keys_for(ty: &TypeRef, has_enum: bool) -> &'static [&'static str] {
	const BYTES: &[&str] = &["contents", "size", "size-eos", "pad-right", "parent", "process"];
	const STR: &[&str] = &["size", "size-eos", "pad-right", "encoding"];
	const ENUM: &[&str] = &["enum"];
	const NONE: &[&str] = &[];
	match ty {
		TypeRef::Bytes { .. } | TypeRef::User { .. } | TypeRef::UserFromBytes { .. } | TypeRef::Switch(_) => BYTES,
		TypeRef::Str { .. } => STR,
		TypeRef::Int { .. } | TypeRef::Float { .. } | TypeRef::Bits { .. } => {
			if has_enum { ENUM } else { NONE }
		}
	}
}

/// The `type:` string grammar.
fn type_from_str(
	dt: Option<&str>,
	path: &str,
	meta: &MetaSpec,
	args: &mut AttrArgs,
) -> Result<TypeRef, KsyError> {
	let ty = match dt {
		None => match &args.contents {
			// `contents` fixes the length itself.
			Some(bytes) => TypeRef::Bytes {
				source: ByteSource::Limit(Expr::IntNum(bytes.len() as i128)),
			},
			None => TypeRef::Bytes { source: args.byte_source(path)? },
		},
		Some(dt) => match parse_primitive(dt) {
			// A single byte has no ends to put in an order, so `u1` and `s1`
			// ask nothing of `meta/endian`.
			Some(Primitive::Int { signed, width }) => TypeRef::Int { signed, width, endian: None },
			Some(Primitive::IntSized { signed, width, endian }) => {
				TypeRef::Int { signed, width, endian: fixed_endian(endian, meta, dt, path)? }
			}
			Some(Primitive::Float { width, endian }) => {
				TypeRef::Float { width, endian: fixed_endian(endian, meta, dt, path)? }
			}
			Some(Primitive::Bits { width, endian }) => TypeRef::Bits {
				width,
				endian: endian.or(meta.bit_endian).unwrap_or(Endian::Big),
			},
			Some(Primitive::Str { zero_terminated }) => {
				let encoding = match args.encoding.clone().or_else(|| meta.encoding.clone()) {
					Some(encoding) => encoding,
					None => {
						return Err(KsyError::new(path, "string type, but no encoding found"));
					}
				};
				if zero_terminated && args.terminator.is_none() {
					args.terminator = Some(default_terminator(&encoding));
				}
				TypeRef::Str { zero_terminated, encoding, source: args.byte_source(path)? }
			}
			None => {
				let (name, type_args) = parse_type_ref(dt)
					.map_err(|e| KsyError::new(format!("{path}/type"), e.to_string()))?;
				if args.size.is_none() && !args.size_eos && args.terminator.is_none() {
					if args.has_process {
						return Err(KsyError::new(
							path,
							format!(
								"user type '{dt}': need 'size' / 'size-eos' / 'terminator' if 'process' is used"
							),
						));
					}
					TypeRef::User { name, args: type_args }
				} else {
					TypeRef::UserFromBytes {
						name,
						args: type_args,
						source: args.byte_source(path)?,
					}
				}
			}
		},
	};
	if args.enum_ref.is_some() && !matches!(ty, TypeRef::Int { .. } | TypeRef::Bits { .. }) {
		return Err(KsyError::new(path, format!("tried to resolve non-integer {ty} to enum")));
	}
	let _ = &args.parent;
	Ok(ty)
}

impl fmt::Display for TypeRef {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			TypeRef::Bytes { .. } => f.write_str("a byte array"),
			TypeRef::Int { signed, width, .. } => {
				write!(f, "{}{width}", if *signed { 's' } else { 'u' })
			}
			TypeRef::Float { width, .. } => write!(f, "f{width}"),
			TypeRef::Bits { width, .. } => write!(f, "b{width}"),
			TypeRef::Str { zero_terminated, .. } => {
				f.write_str(if *zero_terminated { "strz" } else { "str" })
			}
			TypeRef::User { name, .. } | TypeRef::UserFromBytes { name, .. } => write!(f, "{name}"),
			TypeRef::Switch(_) => f.write_str("a switch"),
		}
	}
}

/// What the null terminator of a `strz` is, which depends on the encoding: two
/// zero bytes for UTF-16, four for UTF-32, one for everything else.
fn default_terminator(encoding: &str) -> Vec<u8> {
	let flat: String =
		encoding.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_lowercase();
	if flat.starts_with("utf16") {
		vec![0, 0]
	} else if flat.starts_with("utf32") {
		vec![0, 0, 0, 0]
	} else {
		vec![0]
	}
}

enum Primitive {
	/// `u1` or `s1`, which have no endianness to give.
	Int { signed: bool, width: u8 },
	IntSized { signed: bool, width: u8, endian: Option<Endian> },
	Float { width: u8, endian: Option<Endian> },
	Bits { width: u32, endian: Option<Endian> },
	Str { zero_terminated: bool },
}

/// The built-in type names: `[us](1|2|4|8)(le|be)?`, `f(4|8)(le|be)?`,
/// `b\d+(le|be)?`, `str` and `strz`. Anything else is a user type, and `u1le`
/// is one of those: only the two-, four- and eight-byte forms take a suffix.
fn parse_primitive(dt: &str) -> Option<Primitive> {
	match dt {
		"u1" => return Some(Primitive::Int { signed: false, width: 1 }),
		"s1" => return Some(Primitive::Int { signed: true, width: 1 }),
		"str" => return Some(Primitive::Str { zero_terminated: false }),
		"strz" => return Some(Primitive::Str { zero_terminated: true }),
		_ => {}
	}
	let (body, endian) = split_endian(dt);
	let mut chars = body.chars();
	let first = chars.next()?;
	let rest: String = chars.collect();
	match first {
		'u' | 's' => {
			let width = match rest.as_str() {
				"2" => 2,
				"4" => 4,
				"8" => 8,
				_ => return None,
			};
			Some(Primitive::IntSized { signed: first == 's', width, endian })
		}
		'f' => {
			let width = match rest.as_str() {
				"4" => 4,
				"8" => 8,
				_ => return None,
			};
			Some(Primitive::Float { width, endian })
		}
		'b' => {
			if rest.is_empty() || !rest.chars().all(|c| c.is_ascii_digit()) {
				return None;
			}
			Some(Primitive::Bits { width: rest.parse().ok()?, endian })
		}
		_ => None,
	}
}

fn split_endian(dt: &str) -> (&str, Option<Endian>) {
	if let Some(body) = dt.strip_suffix("le") {
		return (body, Some(Endian::Little));
	}
	if let Some(body) = dt.strip_suffix("be") {
		return (body, Some(Endian::Big));
	}
	(dt, None)
}

/// The endianness a numeric field reads with: its own suffix, else `meta`.
/// `None` means the file chooses while it is being read, and an error means
/// nobody ever said.
fn fixed_endian(
	explicit: Option<Endian>,
	meta: &MetaSpec,
	dt: &str,
	path: &str,
) -> Result<Option<Endian>, KsyError> {
	if let Some(endian) = explicit {
		return Ok(Some(endian));
	}
	match &meta.endian {
		Some(Endianness::Fixed(endian)) => Ok(Some(*endian)),
		Some(Endianness::Calc(_) | Endianness::Inherited) => Ok(None),
		None => Err(KsyError::new(
			format!("{path}/type"),
			format!("unable to use type '{dt}' without default endianness"),
		)),
	}
}

/// `contents`: a string, a list of ints, or a list mixing the two. A string in
/// the list that reads as a number is that one byte, which is the compiler's
/// rule and the reason `contents: ["0x0a"]` is a newline and not four
/// characters.
fn read_contents(node: &YamlNode) -> Result<Vec<u8>, KsyError> {
	match &node.value {
		YamlValue::Str(s) => Ok(s.as_bytes().to_vec()),
		YamlValue::Seq(items) => {
			let mut out = Vec::new();
			for item in items {
				match &item.value {
					YamlValue::Str(s) => out.extend(str_to_bytes(s, item)?),
					YamlValue::Int(n) => out.push(clamp_to_byte(*n, item)?),
					_ => {
						return Err(KsyError::new(
							item.path.to_string(),
							format!("unable to parse fixed content in array: {}", item.describe()),
						));
					}
				}
			}
			Ok(out)
		}
		_ => Err(KsyError::new(
			node.path.to_string(),
			format!("unable to parse fixed content: {}", node.describe()),
		)),
	}
}

fn str_to_bytes(s: &str, node: &YamlNode) -> Result<Vec<u8>, KsyError> {
	let decimal = {
		let body = s.strip_prefix('-').unwrap_or(s);
		!body.is_empty() && body.chars().all(|c| c.is_ascii_digit())
	};
	if decimal {
		let n: i128 = s.parse().map_err(|_| {
			KsyError::new(node.path.to_string(), format!("value {s} outside of byte range"))
		})?;
		return Ok(vec![clamp_to_byte(n, node)?]);
	}
	if let Some(hex) = s.strip_prefix("0x") {
		if !hex.is_empty() && hex.chars().all(|c| c.is_ascii_hexdigit()) {
			let n = i128::from_str_radix(hex, 16).map_err(|_| {
				KsyError::new(node.path.to_string(), format!("value {s} outside of byte range"))
			})?;
			return Ok(vec![clamp_to_byte(n, node)?]);
		}
	}
	Ok(s.as_bytes().to_vec())
}

fn clamp_to_byte(n: i128, node: &YamlNode) -> Result<u8, KsyError> {
	if (-128..256).contains(&n) {
		return Ok(n as u8);
	}
	Err(KsyError::new(node.path.to_string(), format!("value {n} outside of byte range")))
}

impl SwitchSpec {
	fn from_yaml(node: &YamlNode, meta: &MetaSpec, args: &mut AttrArgs) -> Result<Self, KsyError> {
		let on = value_expr(node, "switch-on")?;
		let Some(cases_node) = node.get("cases") else {
			return Err(KsyError::new(node.path.to_string(), "missing mandatory argument `cases`"));
		};
		let entries: &[MapEntry] = cases_node.as_map()?;
		legal_keys(node, &["switch-on", "cases"], &[], None)?;

		let mut cases = Vec::new();
		for entry in entries {
			let case_path = cases_node.child_path(&entry.key_text);
			let key = parse_expr(&entry.key_text)
				.map_err(|e| KsyError::new(case_path.clone(), e.to_string()))?;
			let type_name = entry.value.as_str()?;
			let ty = type_from_str(Some(&type_name), &case_path, meta, args)?;
			cases.push(SwitchCase {
				key,
				key_text: entry.key_text.clone(),
				path: case_path,
				ty,
				implicit: false,
			});
		}

		// A field with a `size` and no default case still has to say what the
		// other values leave behind, and that is the bytes themselves.
		let has_else = cases.iter().any(SwitchCase::is_else);
		if !has_else {
			let fallback = match (&args.size, args.size_eos) {
				(Some(size), false) => Some(ByteSource::Limit(size.clone())),
				(None, true) => Some(ByteSource::Eos),
				(None, false) => None,
				(Some(_), true) => {
					return Err(KsyError::new(
						node.path.to_string(),
						"can't have both `size` and `size-eos` defined",
					));
				}
			};
			if let Some(source) = fallback {
				cases.push(SwitchCase {
					key: Expr::Name("_".to_string()),
					key_text: "_".to_string(),
					path: cases_node.child_path("_"),
					ty: TypeRef::Bytes { source },
					implicit: true,
				});
			}
		}

		Ok(SwitchSpec { on, cases })
	}
}

impl InstanceSpec {
	fn from_yaml(node: &YamlNode, name: &str, meta: &MetaSpec) -> Result<Self, KsyError> {
		let path = node.path.to_string();
		if let Some(value_node) = node.get("value") {
			let vendor = legal_keys(
				node,
				&["value", "doc", "doc-ref", "enum", "if"],
				&[],
				Some("value instance"),
			)?;
			return Ok(InstanceSpec::Value(ValueInstanceSpec {
				path,
				name: name.to_string(),
				value: expr_at(value_node)?,
				if_expr: opt_expr(node, "if")?,
				enum_ref: opt_str(node, "enum")?,
				doc: DocSpec::from_yaml(node)?,
				vendor,
			}));
		}
		let pos = opt_expr(node, "pos")?;
		let io = opt_expr(node, "io")?;
		let attr = AttrSpec::from_map(node, meta, Some(name.to_string()), 0, &["pos", "io"])?;
		Ok(InstanceSpec::Parse(ParseInstanceSpec { path, name: name.to_string(), attr, pos, io }))
	}
}

impl EnumSpec {
	fn from_yaml(node: &YamlNode, path: &str) -> Result<Self, KsyError> {
		let mut values = Vec::new();
		for entry in node.as_map()? {
			let value = entry.key.as_big_int()?;
			let value_path = format!("{path}/{value}");
			let spec = EnumValueSpec::from_yaml(&entry.value, &value_path, value)?;
			if let Some(prev) = values.iter().find(|v: &&EnumValueSpec| v.name == spec.name) {
				return Err(KsyError::new(
					value_path,
					format!(
						"duplicate enum member ID: '{}', previously defined at {}",
						spec.name, prev.path
					),
				));
			}
			values.push(spec);
		}
		Ok(EnumSpec { path: path.to_string(), values })
	}
}

impl EnumValueSpec {
	fn from_yaml(node: &YamlNode, path: &str, value: i128) -> Result<Self, KsyError> {
		match &node.value {
			YamlValue::Str(name) => {
				check_identifier(name, "enum member", path)?;
				Ok(EnumValueSpec {
					path: path.to_string(),
					value,
					name: name.clone(),
					doc: DocSpec::default(),
					vendor: Vec::new(),
				})
			}
			YamlValue::Bool(b) => {
				let name = b.to_string();
				check_identifier(&name, "enum member", path)?;
				Ok(EnumValueSpec {
					path: path.to_string(),
					value,
					name,
					doc: DocSpec::default(),
					vendor: Vec::new(),
				})
			}
			YamlValue::Map(_) => {
				let vendor =
					legal_keys(node, &["id", "doc", "doc-ref"], &[], Some("enum value spec"))?;
				let name = match node.get("id") {
					Some(child) => child.as_str()?,
					None => {
						return Err(KsyError::new(path, "missing mandatory argument `id`"));
					}
				};
				check_identifier(&name, "enum value spec id", path)?;
				Ok(EnumValueSpec {
					path: path.to_string(),
					value,
					name,
					doc: DocSpec::from_yaml(node)?,
					vendor,
				})
			}
			_ => Err(KsyError::new(
				path,
				format!("expected string or map, got {}", node.describe()),
			)),
		}
	}
}

impl ParamSpec {
	fn from_yaml(node: &YamlNode, index: usize) -> Result<Self, KsyError> {
		let path = node.path.to_string();
		let id = opt_str(node, "id")?;
		if let Some(id) = &id {
			check_identifier(id, "parameter", &node.child_path("id"))?;
		}
		let doc = DocSpec::from_yaml(node)?;
		let type_str = opt_str(node, "type")?;
		let enum_ref = opt_str(node, "enum")?;
		let ty = param_type_from_str(type_str.as_deref(), &path)?;
		if enum_ref.is_some()
			&& !matches!(ty, ParamType::Int { .. } | ParamType::Bits { .. })
		{
			return Err(KsyError::new(
				path,
				format!("tried to resolve non-integer {} to enum", type_str.unwrap_or_default()),
			));
		}
		let vendor = legal_keys(
			node,
			&["id", "type", "enum", "doc", "doc-ref"],
			&[],
			Some("parameter definition"),
		)?;
		Ok(ParamSpec { path, id, index, ty, enum_ref, doc, vendor })
	}
}

/// The `params` type grammar. Much smaller than the `seq` one: no endianness,
/// no size, but `bytes`, `bool`, `struct`, `io`, `any` and a `[]` suffix.
fn param_type_from_str(dt: Option<&str>, path: &str) -> Result<ParamType, KsyError> {
	let Some(dt) = dt else { return Ok(ParamType::Bytes) };
	if let Some(single) = dt.strip_suffix("[]") {
		return Ok(ParamType::Array(Box::new(param_type_from_str(Some(single), path)?)));
	}
	Ok(match dt {
		"bytes" => ParamType::Bytes,
		"u1" => ParamType::Int { signed: false, width: 1 },
		"s1" => ParamType::Int { signed: true, width: 1 },
		"u2" => ParamType::Int { signed: false, width: 2 },
		"u4" => ParamType::Int { signed: false, width: 4 },
		"u8" => ParamType::Int { signed: false, width: 8 },
		"s2" => ParamType::Int { signed: true, width: 2 },
		"s4" => ParamType::Int { signed: true, width: 4 },
		"s8" => ParamType::Int { signed: true, width: 8 },
		"f4" => ParamType::Float { width: 4 },
		"f8" => ParamType::Float { width: 8 },
		"str" => ParamType::Str,
		"bool" => ParamType::Bool,
		"struct" => ParamType::Struct,
		"io" => ParamType::Io,
		"any" => ParamType::Any,
		_ => {
			if let Some(width) = dt.strip_prefix('b') {
				if !width.is_empty() && width.chars().all(|c| c.is_ascii_digit()) {
					return Ok(ParamType::Bits { width: width.parse().unwrap_or(1) });
				}
			}
			ParamType::User(TypeId {
				absolute: false,
				names: dt.split("::").map(str::to_string).collect(),
				is_array: false,
			})
		}
	})
}

impl ValidSpec {
	fn from_yaml(node: &YamlNode) -> Result<Self, KsyError> {
		let path = node.path.to_string();
		match &node.value {
			YamlValue::Str(_) | YamlValue::Bool(_) | YamlValue::Int(_) => {
				Ok(ValidSpec::Eq(expr_at(node)?))
			}
			YamlValue::Map(_) => {
				if let Some(child) = node.get("eq") {
					legal_keys(node, &["eq"], &[], None)?;
					return Ok(ValidSpec::Eq(expr_at(child)?));
				}
				let min = opt_expr(node, "min")?;
				let max = opt_expr(node, "max")?;
				if min.is_some() || max.is_some() {
					legal_keys(node, &["min", "max"], &[], None)?;
					return Ok(match (min, max) {
						(Some(min), Some(max)) => ValidSpec::Range { min, max },
						(Some(min), None) => ValidSpec::Min(min),
						(None, Some(max)) => ValidSpec::Max(max),
						(None, None) => unreachable!("one of them is set"),
					});
				}
				if let Some(child) = node.get("any-of") {
					legal_keys(node, &["any-of"], &[], None)?;
					let mut values = Vec::new();
					for item in child.as_seq()? {
						values.push(expr_at(item)?);
					}
					return Ok(ValidSpec::AnyOf(values));
				}
				if let Some(child) = node.get("in-enum") {
					legal_keys(node, &["in-enum"], &[], None)?;
					if !child.as_bool()? {
						return Err(KsyError::new(
							child.path.to_string(),
							"only `true` is supported as value, got `false` \
							 (if you don't want any validation, omit the `valid` key)",
						));
					}
					return Ok(ValidSpec::InEnum);
				}
				if let Some(child) = node.get("expr") {
					legal_keys(node, &["expr"], &[], None)?;
					return Ok(ValidSpec::Expr(expr_at(child)?));
				}
				legal_keys(node, &["eq", "min", "max", "any-of", "in-enum", "expr"], &[], None)?;
				Err(KsyError::new(
					path,
					"expected at least one of: any-of, eq, expr, in-enum, max, min",
				))
			}
			_ => Err(KsyError::new(path, format!("expected string or map, got {}", node.describe()))),
		}
	}
}

impl RepeatSpec {
	/// The repeat, and which of `repeat-expr` and `repeat-until` the field is
	/// then allowed to write. Writing the other one is writing an unknown key.
	fn from_yaml(node: &YamlNode) -> Result<(Self, &'static [&'static str]), KsyError> {
		let repeat_expr = opt_expr(node, "repeat-expr")?;
		let repeat_until = opt_expr(node, "repeat-until")?;
		let Some(repeat) = opt_str(node, "repeat")? else { return Ok((RepeatSpec::No, &[])) };
		match repeat.as_str() {
			"eos" => Ok((RepeatSpec::Eos, &[])),
			"expr" => match repeat_expr {
				Some(e) => Ok((RepeatSpec::Expr(e), &["repeat-expr"])),
				None => Err(KsyError::new(
					node.child_path("repeat"),
					"`repeat: expr` requires a `repeat-expr` expression",
				)),
			},
			"until" => match repeat_until {
				Some(e) => Ok((RepeatSpec::Until(e), &["repeat-until"])),
				None => Err(KsyError::new(
					node.child_path("repeat"),
					"`repeat: until` requires a `repeat-until` expression",
				)),
			},
			other => Err(KsyError::new(
				node.child_path("repeat"),
				format!("expected eos / expr / until, got '{other}'"),
			)),
		}
	}
}

impl ProcessSpec {
	/// `process` is written as a call: `zlib`, `xor(key)`, `rol(n)`, `ror(n)`,
	/// or the name of something the compiler does not ship.
	fn from_str_node(node: &YamlNode) -> Result<Self, KsyError> {
		let text = node.as_str()?;
		let path = node.path.to_string();
		let fail = |e: super::expr::ParseError| KsyError::new(path.clone(), e.to_string());
		if text == "zlib" {
			return Ok(ProcessSpec::Zlib);
		}
		if let Some((name, arg_text)) = split_call(&text) {
			let arg_text = arg_text.trim();
			return Ok(match name {
				"xor" => ProcessSpec::Xor(parse_expr(arg_text).map_err(fail)?),
				"rol" => ProcessSpec::Rotate { left: true, key: parse_expr(arg_text).map_err(fail)? },
				"ror" => {
					ProcessSpec::Rotate { left: false, key: parse_expr(arg_text).map_err(fail)? }
				}
				_ => {
					if !is_process_name(name) {
						return Err(KsyError::new(path, format!("invalid process: '{text}'")));
					}
					let args = if arg_text.is_empty() {
						Vec::new()
					} else {
						parse_expr_list(arg_text).map_err(fail)?
					};
					ProcessSpec::Custom { name: name.split('.').map(str::to_string).collect(), args }
				}
			});
		}
		if is_process_name(&text) {
			return Ok(ProcessSpec::Custom {
				name: text.split('.').map(str::to_string).collect(),
				args: Vec::new(),
			});
		}
		Err(KsyError::new(path, format!("invalid process: '{text}'")))
	}
}

fn is_process_name(name: &str) -> bool {
	let mut chars = name.chars();
	match chars.next() {
		Some(c) if c.is_ascii_lowercase() => {}
		_ => return false,
	}
	chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '.')
}

/// `name(inside)`, where `inside` runs to the last `)`.
fn split_call(text: &str) -> Option<(&str, &str)> {
	let open = text.find('(')?;
	if !text.ends_with(')') {
		return None;
	}
	Some((&text[..open], &text[open + 1..text.len() - 1]))
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::ksy::yaml::load;

	fn spec(text: &str) -> Result<ClassSpec, KsyError> {
		ClassSpec::from_document(&load(text)?)
	}

	fn ok(text: &str) -> ClassSpec {
		spec(text).unwrap_or_else(|e| panic!("{e}"))
	}

	const HEAD: &str = "meta:\n  id: t\n  endian: le\n";

	#[test]
	fn a_top_level_type_needs_an_id() {
		let err = spec("seq:\n  - id: a\n    type: u1\n").unwrap_err();
		assert_eq!(err.message, "no `meta/id` encountered in top-level class spec");
		assert_eq!(err.path, "/meta");
	}

	#[test]
	fn endianness_comes_from_meta_and_from_the_suffix() {
		let c = ok(&format!("{HEAD}seq:\n  - id: a\n    type: u2\n  - id: b\n    type: u4be\n"));
		assert_eq!(c.seq[0].ty, TypeRef::Int { signed: false, width: 2, endian: Some(Endian::Little) });
		assert_eq!(c.seq[1].ty, TypeRef::Int { signed: false, width: 4, endian: Some(Endian::Big) });
	}

	#[test]
	fn a_wide_number_without_a_default_endianness_is_refused() {
		let err = spec("meta:\n  id: t\nseq:\n  - id: a\n    type: u2\n").unwrap_err();
		assert_eq!(err.message, "unable to use type 'u2' without default endianness");
		assert_eq!(err.path, "/seq/0/type");
		// A single byte has no ends to order, so it needs no default.
		ok("meta:\n  id: t\nseq:\n  - id: a\n    type: u1\n");
	}

	#[test]
	fn switch_on_endianness_leaves_the_choice_open() {
		let c = ok(
			"meta:\n  id: t\n  endian:\n    switch-on: kind\n    cases:\n      1: le\n      _: be\nseq:\n  - id: a\n    type: u4\n",
		);
		assert!(matches!(c.meta.endian, Some(Endianness::Calc(_))));
		assert_eq!(c.seq[0].ty, TypeRef::Int { signed: false, width: 4, endian: None });
	}

	#[test]
	fn legal_keys_depend_on_what_the_type_turned_out_to_be() {
		let err = spec(&format!("{HEAD}seq:\n  - id: a\n    type: u4\n    size: 8\n")).unwrap_err();
		assert!(err.message.starts_with("unknown key found"), "{}", err.message);
		assert_eq!(err.path, "/seq/0/size");
		ok(&format!("{HEAD}seq:\n  - id: a\n    size: 8\n"));
		ok(&format!("{HEAD}seq:\n  - id: a\n    type: u4\n    enum: e\nenums:\n  e:\n    1: one\n"));
	}

	#[test]
	fn a_repeat_only_allows_its_own_key() {
		ok(&format!("{HEAD}seq:\n  - id: a\n    type: u1\n    repeat: expr\n    repeat-expr: 3\n"));
		let err =
			spec(&format!("{HEAD}seq:\n  - id: a\n    type: u1\n    repeat: eos\n    repeat-expr: 3\n"))
				.unwrap_err();
		assert!(err.message.starts_with("unknown key found"), "{}", err.message);
		let err = spec(&format!("{HEAD}seq:\n  - id: a\n    type: u1\n    repeat: expr\n")).unwrap_err();
		assert_eq!(err.message, "`repeat: expr` requires a `repeat-expr` expression");
	}

	#[test]
	fn a_dash_key_is_kept_rather_than_refused() {
		let c = ok(&format!(
			"{HEAD}-webide-representation: '{{a}}'\nseq:\n  - id: a\n    -orig-id: A\n    type: u1\n"
		));
		assert_eq!(c.vendor[0].0, "-webide-representation");
		assert_eq!(c.seq[0].vendor[0].0, "-orig-id");
	}

	#[test]
	fn contents_reads_as_bytes_however_it_is_written() {
		let c = ok(&format!(
			"{HEAD}seq:\n  - id: a\n    contents: MZ\n  - id: b\n    contents: [0x4d, 0x5a]\n  - id: c\n    contents: [CAFE, 0, '0x0a']\n"
		));
		assert_eq!(c.seq[0].contents, Some(b"MZ".to_vec()));
		assert_eq!(c.seq[1].contents, Some(vec![0x4d, 0x5a]));
		assert_eq!(c.seq[2].contents, Some(vec![b'C', b'A', b'F', b'E', 0, 0x0a]));
	}

	#[test]
	fn a_switch_takes_a_default_case_when_the_field_has_a_size() {
		// A case key is an expression, so a key meant as a string literal is
		// written with its quotes inside the YAML string: `'"DATA"'`.
		let c = ok(&format!(
			"{HEAD}seq:\n  - id: a\n    type:\n      switch-on: kind\n      cases:\n        1: one\n        '\"DATA\"': two\n    size: 8\ntypes:\n  one: {{}}\n  two: {{}}\n"
		));
		let TypeRef::Switch(switch) = &c.seq[0].ty else { panic!("a switch") };
		assert_eq!(switch.cases.len(), 3);
		assert!(switch.cases[2].implicit && switch.cases[2].is_else());
		assert_eq!(switch.cases[0].key, Expr::IntNum(1));
		assert_eq!(switch.cases[1].key, Expr::Str("DATA".into()));
	}

	#[test]
	fn strz_takes_its_terminator_from_the_encoding() {
		let c = ok(&format!(
			"{HEAD}seq:\n  - id: a\n    type: strz\n    encoding: ASCII\n  - id: b\n    type: strz\n    encoding: UTF-16LE\n"
		));
		assert_eq!(c.seq[0].terminator, Some(vec![0]));
		assert_eq!(c.seq[1].terminator, Some(vec![0, 0]));
	}

	#[test]
	fn a_string_without_an_encoding_anywhere_is_refused() {
		let err = spec(&format!("{HEAD}seq:\n  - id: a\n    type: str\n    size: 4\n")).unwrap_err();
		assert_eq!(err.message, "string type, but no encoding found");
	}

	#[test]
	fn an_enum_on_something_that_is_not_a_number_is_refused() {
		let err = spec(&format!("{HEAD}seq:\n  - id: a\n    size: 4\n    enum: e\n")).unwrap_err();
		assert!(err.message.starts_with("tried to resolve non-integer"), "{}", err.message);
	}

	#[test]
	fn two_members_may_not_share_a_name() {
		let err = spec(&format!("{HEAD}seq:\n  - id: a\n    type: u1\n  - id: a\n    type: u1\n"))
			.unwrap_err();
		assert!(err.message.starts_with("duplicate attribute ID 'a'"), "{}", err.message);
		let err = spec(&format!(
			"{HEAD}seq:\n  - id: a\n    type: u1\ninstances:\n  a:\n    value: 1\n"
		))
		.unwrap_err();
		assert!(err.message.starts_with("duplicate attribute ID 'a'"), "{}", err.message);
	}

	#[test]
	fn a_value_instance_takes_only_its_own_keys() {
		let err = spec(&format!("{HEAD}instances:\n  a:\n    value: 1\n    size: 4\n")).unwrap_err();
		assert_eq!(
			err.message,
			"invalid key found in value instance, allowed: doc, doc-ref, enum, if, value"
		);
	}

	#[test]
	fn a_pos_instance_reads_as_a_field_with_a_position() {
		let c = ok(&format!(
			"{HEAD}instances:\n  a:\n    pos: 4\n    io: _root._io\n    type: u2\n"
		));
		let InstanceSpec::Parse(p) = &c.instances[0].1 else { panic!("a parse instance") };
		assert_eq!(p.pos, Some(Expr::IntNum(4)));
		assert!(p.io.is_some());
		assert_eq!(p.attr.ty, TypeRef::Int { signed: false, width: 2, endian: Some(Endian::Little) });
	}

	#[test]
	fn an_enum_takes_int_keys_and_either_form_of_value() {
		let c = ok(&format!(
			"{HEAD}enums:\n  e:\n    0: zero\n    0x10: sixteen\n    2:\n      id: two\n      doc: the third\n"
		));
		let e = &c.enums[0].1;
		assert_eq!(e.values[1].value, 16);
		assert_eq!(e.values[1].name, "sixteen");
		assert_eq!(e.values[2].name, "two");
		assert_eq!(e.values[2].doc.summary.as_deref(), Some("the third"));
	}

	#[test]
	fn a_parameter_type_is_not_a_field_type() {
		let c = ok(&format!("{HEAD}params:\n  - id: a\n    type: u4\n  - id: b\n    type: u4le\n  - id: c\n    type: bytes\n  - id: d\n    type: str[]\n"));
		assert_eq!(c.params[0].ty, ParamType::Int { signed: false, width: 4 });
		assert!(matches!(c.params[1].ty, ParamType::User(_)));
		assert_eq!(c.params[2].ty, ParamType::Bytes);
		assert_eq!(c.params[3].ty, ParamType::Array(Box::new(ParamType::Str)));
	}

	#[test]
	fn process_reads_as_a_call() {
		let c = ok(&format!(
			"{HEAD}seq:\n  - id: a\n    size: 4\n    process: zlib\n  - id: b\n    size: 4\n    process: xor(0x2a)\n  - id: c\n    size: 4\n    process: rol(3)\n  - id: d\n    size: 4\n    process: my.codec(1, 2)\n"
		));
		assert_eq!(c.seq[0].process, Some(ProcessSpec::Zlib));
		assert_eq!(c.seq[1].process, Some(ProcessSpec::Xor(Expr::IntNum(42))));
		assert_eq!(c.seq[2].process, Some(ProcessSpec::Rotate { left: true, key: Expr::IntNum(3) }));
		let Some(ProcessSpec::Custom { name, args }) = &c.seq[3].process else { panic!("custom") };
		assert_eq!(name, &vec!["my".to_string(), "codec".to_string()]);
		assert_eq!(args.len(), 2);
	}

	#[test]
	fn nested_types_carry_their_path_and_their_parents_meta() {
		let c = ok(&format!("{HEAD}types:\n  inner:\n    seq:\n      - id: a\n        type: u2\n"));
		let inner = &c.types[0].1;
		assert_eq!(inner.path, "/types/inner");
		assert_eq!(inner.seq[0].path, "/types/inner/seq/0");
		assert_eq!(inner.seq[0].ty, TypeRef::Int { signed: false, width: 2, endian: Some(Endian::Little) });
	}

	#[test]
	fn every_expression_in_the_file_can_be_walked() {
		let c = ok(&format!(
			"{HEAD}seq:\n  - id: n\n    type: u1\n  - id: a\n    size: n * 2\n    if: n > 0\n    repeat: expr\n    repeat-expr: n\ninstances:\n  b:\n    value: n + 1\n"
		));
		let mut found = Vec::new();
		c.for_each_expr(&mut |path, e| found.push(format!("{path} = {e}")));
		assert!(found.contains(&"/seq/1/size = n * 2".to_string()), "{found:?}");
		assert!(found.contains(&"/seq/1/if = n > 0".to_string()), "{found:?}");
		assert!(found.contains(&"/seq/1/repeat-expr = n".to_string()), "{found:?}");
		assert!(found.contains(&"/instances/b/value = n + 1".to_string()), "{found:?}");
	}
}
