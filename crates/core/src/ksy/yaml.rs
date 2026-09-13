//! A `.ksy` document as a tree that remembers where every node came from.
//!
//! This is not a deserialiser. A `.ksy` key holds an int in one file and a
//! string in the next (`size: 4` and `size: len_body`, `contents: "MZ"` and
//! `contents: [0x4d, 0x5a]`), and every error has to name the path it happened
//! at, so the document is walked as a generic tree the way the Kaitai compiler
//! walks it, with the path carried on each node.
//!
//! The accessors below are the compiler's `ParseUtils` under different names:
//! [`YamlNode::as_str`] accepts a number or a boolean and writes it out, the
//! way `asStr` does, because `id: 1` and `id: "1"` have to mean the same field
//! name; [`YamlNode::as_big_int`] accepts the string a very large enum key
//! arrives as. Mapping keys stay in file order, and a key written twice is an
//! error rather than a silent overwrite.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use saphyr::Scalar;
use saphyr_parser::{Event, Parser, ScalarStyle, Span};

/// Anything wrong with a `.ksy`, named by where in the document it is.
///
/// `path` is the slash-joined YAML path, empty for the document itself. The
/// message is the compiler's wording wherever the compiler has one, so that a
/// `.ksy` rejected here and a `.ksy` rejected by `ksc` are rejected in the same
/// words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KsyError {
	pub path: String,
	pub message: String,
}

impl KsyError {
	pub fn new(path: impl Into<String>, message: impl Into<String>) -> Self {
		KsyError { path: path.into(), message: message.into() }
	}
}

impl fmt::Display for KsyError {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		let path = if self.path.is_empty() { "/" } else { &self.path };
		write!(f, "{path}: {}", self.message)
	}
}

impl std::error::Error for KsyError {}

/// What a node holds, once the YAML core schema has resolved the scalar.
#[derive(Debug, Clone, PartialEq)]
pub enum YamlValue {
	Null,
	Bool(bool),
	/// An integer, widened to `i128` so that a `u8` enum key survives.
	Int(i128),
	Float(f64),
	Str(String),
	Seq(Vec<YamlNode>),
	/// Keys in the order the file wrote them.
	Map(Vec<MapEntry>),
}

/// One `key: value` pair, with the key kept as a node so a bad key can be
/// reported at its own line.
#[derive(Debug, Clone, PartialEq)]
pub struct MapEntry {
	pub key: YamlNode,
	/// The key as `ParseUtils.asStr` would write it: the string itself, or a
	/// number or boolean spelled out. This is the path segment as well.
	pub key_text: String,
	pub value: YamlNode,
}

/// A node and where it came from.
#[derive(Debug, Clone, PartialEq)]
pub struct YamlNode {
	/// Slash-joined path from the document root, e.g. `/types/chunk/seq/2/size`.
	/// Empty for the root node itself.
	pub path: Arc<str>,
	/// 1-based line in the source, for the panel and for messages.
	pub line: usize,
	/// 1-based column.
	pub col: usize,
	pub value: YamlValue,
}

impl YamlNode {
	/// The path a child at `segment` would have.
	pub fn child_path(&self, segment: &str) -> String {
		format!("{}/{segment}", self.path)
	}

	fn err(&self, message: impl Into<String>) -> KsyError {
		KsyError::new(self.path.to_string(), message)
	}

	/// What to call this node in a "expected X, got Y" message.
	pub fn describe(&self) -> String {
		match &self.value {
			YamlValue::Null => "null".to_string(),
			YamlValue::Bool(b) => b.to_string(),
			YamlValue::Int(n) => n.to_string(),
			YamlValue::Float(f) => format!("{f:?}"),
			YamlValue::Str(s) => s.clone(),
			YamlValue::Seq(_) => "array".to_string(),
			YamlValue::Map(_) => "map".to_string(),
		}
	}

	pub fn is_null(&self) -> bool {
		matches!(self.value, YamlValue::Null)
	}

	/// The node as a string, spelling out a number or boolean. `ParseUtils.asStr`.
	pub fn as_str(&self) -> Result<String, KsyError> {
		match &self.value {
			YamlValue::Str(s) => Ok(s.clone()),
			YamlValue::Int(n) => Ok(n.to_string()),
			YamlValue::Float(f) => Ok(format!("{f:?}")),
			YamlValue::Bool(b) => Ok(b.to_string()),
			_ => Err(self.err(format!("expected string, got {}", self.describe()))),
		}
	}

	/// The node as an integer. `ParseUtils.asBigInt`, which also takes the
	/// string a key too large for the YAML parser's integer arrives as.
	pub fn as_big_int(&self) -> Result<i128, KsyError> {
		match &self.value {
			YamlValue::Int(n) => Ok(*n),
			YamlValue::Str(s) => {
				parse_int_literal(s).ok_or_else(|| self.err(format!("unable to parse `{s}` as int")))
			}
			_ => Err(self.err(format!("expected int, got {}", self.describe()))),
		}
	}

	/// The node as an integer that fits a byte. `ParseUtils.getOptValueByte`.
	pub fn as_byte(&self) -> Result<u8, KsyError> {
		let n = self.as_int()?;
		if !(0..=255).contains(&n) {
			return Err(self.err(format!("expected an integer from 0 to 255, got {n}")));
		}
		Ok(n as u8)
	}

	/// The node as an integer, refusing a string. `ParseUtils.getOptValueInt`.
	pub fn as_int(&self) -> Result<i128, KsyError> {
		match &self.value {
			YamlValue::Int(n) => Ok(*n),
			_ => Err(self.err(format!("expected int, got {}", self.describe()))),
		}
	}

	pub fn as_bool(&self) -> Result<bool, KsyError> {
		match &self.value {
			YamlValue::Bool(b) => Ok(*b),
			_ => Err(self.err(format!("expected boolean, got {}", self.describe()))),
		}
	}

	pub fn as_seq(&self) -> Result<&[YamlNode], KsyError> {
		match &self.value {
			YamlValue::Seq(items) => Ok(items),
			_ => Err(self.err(format!("expected array, found {}", self.describe()))),
		}
	}

	pub fn as_map(&self) -> Result<&[MapEntry], KsyError> {
		match &self.value {
			YamlValue::Map(entries) => Ok(entries),
			_ => Err(self.err(format!("expected map, got {}", self.describe()))),
		}
	}

	/// The value under `key`, if the node is a map that has one.
	pub fn get(&self, key: &str) -> Option<&YamlNode> {
		match &self.value {
			YamlValue::Map(entries) => {
				entries.iter().find(|e| e.key_text == key).map(|e| &e.value)
			}
			_ => None,
		}
	}

	/// A list of strings from `key`. A lone scalar counts as a list of one,
	/// which is how `doc-ref` and `imports` are usually written.
	pub fn get_list_str(&self, key: &str) -> Result<Vec<String>, KsyError> {
		let Some(node) = self.get(key) else { return Ok(Vec::new()) };
		match &node.value {
			YamlValue::Seq(items) => items.iter().map(|n| n.as_str()).collect(),
			YamlValue::Null | YamlValue::Map(_) => {
				Err(node.err(format!("expected array, got {}", node.describe())))
			}
			_ => Ok(vec![node.as_str()?]),
		}
	}
}

/// A YAML 1.1/1.2 integer literal, in any of the bases a mapping key may use.
///
/// The YAML parser resolves these itself up to 64 bits; this exists for the
/// ones that do not fit, which arrive as strings, and for the string form the
/// compiler's `asBigInt` also accepts.
fn parse_int_literal(s: &str) -> Option<i128> {
	let (neg, body) = match s.strip_prefix('-') {
		Some(rest) => (true, rest),
		None => (false, s.strip_prefix('+').unwrap_or(s)),
	};
	let (radix, digits) = if let Some(d) = body.strip_prefix("0x").or_else(|| body.strip_prefix("0X")) {
		(16, d)
	} else if let Some(d) = body.strip_prefix("0o").or_else(|| body.strip_prefix("0O")) {
		(8, d)
	} else if let Some(d) = body.strip_prefix("0b").or_else(|| body.strip_prefix("0B")) {
		(2, d)
	} else {
		(10, body)
	};
	let digits: String = digits.chars().filter(|c| *c != '_').collect();
	if digits.is_empty() {
		return None;
	}
	let n = i128::from_str_radix(&digits, radix).ok()?;
	Some(if neg { -n } else { n })
}

/// Read a `.ksy` document into a tree.
///
/// A stream with no document, or with more than one, is an error: a `.ksy` is
/// one type definition.
pub fn load(text: &str) -> Result<YamlNode, KsyError> {
	let mut builder = Builder::new();
	for item in Parser::new_from_str(text) {
		let (event, span) = item.map_err(|e| KsyError::new("", e.to_string()))?;
		builder.on_event(event, span)?;
	}
	match builder.documents.len() {
		0 => Err(KsyError::new("", "expected map, got null")),
		1 => Ok(builder.documents.pop().unwrap()),
		n => Err(KsyError::new("", format!("expected one YAML document, found {n}"))),
	}
}

/// What is being filled in at each level of the document.
enum Frame {
	Seq { path: Arc<str>, line: usize, col: usize, anchor: usize, items: Vec<YamlNode> },
	Map {
		path: Arc<str>,
		line: usize,
		col: usize,
		anchor: usize,
		entries: Vec<MapEntry>,
		/// The key read but not yet paired with its value.
		pending: Option<(YamlNode, String)>,
	},
}

struct Builder {
	stack: Vec<Frame>,
	documents: Vec<YamlNode>,
	anchors: HashMap<usize, YamlNode>,
}

impl Builder {
	fn new() -> Self {
		Builder { stack: Vec::new(), documents: Vec::new(), anchors: HashMap::new() }
	}

	/// The path the node about to start will have.
	fn next_path(&self) -> Arc<str> {
		match self.stack.last() {
			None => Arc::from(""),
			Some(Frame::Seq { path, items, .. }) => Arc::from(format!("{path}/{}", items.len())),
			// A key is reported at the path of the map that holds it, the way
			// the compiler reports one; only the value gets a segment of its own.
			Some(Frame::Map { path, pending, .. }) => match pending {
				None => path.clone(),
				Some((_, key_text)) => Arc::from(format!("{path}/{key_text}")),
			},
		}
	}

	fn on_event(&mut self, event: Event<'_>, span: Span) -> Result<(), KsyError> {
		let line = span.start.line();
		let col = span.start.col() + 1;
		match event {
			Event::Nothing | Event::StreamStart | Event::StreamEnd | Event::DocumentStart(_) | Event::DocumentEnd => Ok(()),
			Event::Scalar(text, style, anchor, tag) => {
				let path = self.next_path();
				let value = resolve_scalar(&text, style, tag.as_deref(), &path)?;
				let node = YamlNode { path, line, col, value };
				self.finish(node, anchor)
			}
			Event::SequenceStart(anchor, _) => {
				let path = self.next_path();
				self.stack.push(Frame::Seq { path, line, col, anchor, items: Vec::new() });
				Ok(())
			}
			Event::MappingStart(anchor, _) => {
				let path = self.next_path();
				self.stack.push(Frame::Map { path, line, col, anchor, entries: Vec::new(), pending: None });
				Ok(())
			}
			Event::SequenceEnd | Event::MappingEnd => {
				let frame = self.stack.pop().expect("parser balances collection events");
				let (node, anchor) = match frame {
					Frame::Seq { path, line, col, anchor, items } => {
						(YamlNode { path, line, col, value: YamlValue::Seq(items) }, anchor)
					}
					Frame::Map { path, line, col, anchor, entries, .. } => {
						(YamlNode { path, line, col, value: YamlValue::Map(entries) }, anchor)
					}
				};
				self.finish(node, anchor)
			}
			Event::Alias(id) => {
				let path = self.next_path();
				let Some(target) = self.anchors.get(&id) else {
					return Err(KsyError::new(path.to_string(), "alias to an anchor that was never defined"));
				};
				let node = repath(target, &path);
				self.finish(node, 0)
			}
		}
	}

	/// Attach a completed node to whatever is waiting for it.
	fn finish(&mut self, node: YamlNode, anchor: usize) -> Result<(), KsyError> {
		if anchor != 0 {
			self.anchors.insert(anchor, node.clone());
		}
		match self.stack.last_mut() {
			None => {
				self.documents.push(node);
				Ok(())
			}
			Some(Frame::Seq { items, .. }) => {
				items.push(node);
				Ok(())
			}
			Some(Frame::Map { entries, pending, path, .. }) => match pending.take() {
				None => {
					let key_text = match &node.value {
						YamlValue::Str(s) => s.clone(),
						YamlValue::Int(n) => n.to_string(),
						YamlValue::Float(f) => format!("{f:?}"),
						YamlValue::Bool(b) => b.to_string(),
						_ => {
							return Err(KsyError::new(
								path.to_string(),
								format!("expected string, got {}", node.describe()),
							));
						}
					};
					if entries.iter().any(|e| e.key_text == key_text) {
						return Err(KsyError::new(
							path.to_string(),
							format!("found duplicate key {key_text}"),
						));
					}
					*pending = Some((node, key_text));
					Ok(())
				}
				Some((key, key_text)) => {
					entries.push(MapEntry { key, key_text, value: node });
					Ok(())
				}
			},
		}
	}
}

/// A copy of an anchored node, re-rooted at the path the alias sits under.
fn repath(node: &YamlNode, path: &Arc<str>) -> YamlNode {
	let value = match &node.value {
		YamlValue::Seq(items) => YamlValue::Seq(
			items
				.iter()
				.enumerate()
				.map(|(i, item)| repath(item, &Arc::from(format!("{path}/{i}"))))
				.collect(),
		),
		YamlValue::Map(entries) => YamlValue::Map(
			entries
				.iter()
				.map(|e| MapEntry {
					key: repath(&e.key, path),
					key_text: e.key_text.clone(),
					value: repath(&e.value, &Arc::from(format!("{path}/{}", e.key_text))),
				})
				.collect(),
		),
		other => other.clone(),
	};
	YamlNode { path: path.clone(), line: node.line, col: node.col, value }
}

/// One scalar, through the YAML core schema.
///
/// A quoted scalar is always a string, which is what makes `id: "1"` a name and
/// `id: 1` the same name by a different route.
///
/// An unquoted, untagged integer is read here rather than by saphyr, because
/// saphyr reads only what YAML 1.2 calls an integer and in 64 bits: a `u8` enum
/// key of `18446744073709551615` comes back from it as a float, and `0b1101`
/// as a string. SnakeYAML, which is what the Kaitai compiler reads a `.ksy`
/// with, is a YAML 1.1 parser and calls both of those integers.
fn resolve_scalar(
	text: &str,
	style: ScalarStyle,
	tag: Option<&saphyr_parser::Tag>,
	path: &Arc<str>,
) -> Result<YamlValue, KsyError> {
	if style == ScalarStyle::Plain && tag.is_none() {
		if let Some(n) = parse_int_literal(text) {
			return Ok(YamlValue::Int(n));
		}
	}
	let tag = tag.map(|t| std::borrow::Cow::Borrowed(t));
	let scalar = Scalar::parse_from_cow_and_metadata(std::borrow::Cow::Borrowed(text), style, tag.as_ref())
		.ok_or_else(|| KsyError::new(path.to_string(), format!("unable to read tagged scalar `{text}`")))?;
	Ok(match scalar {
		Scalar::Null => YamlValue::Null,
		Scalar::Boolean(b) => YamlValue::Bool(b),
		Scalar::Integer(n) => YamlValue::Int(i128::from(n)),
		Scalar::FloatingPoint(f) => YamlValue::Float(f.into_inner()),
		Scalar::String(s) => match (style, parse_int_literal(&s)) {
			(ScalarStyle::Plain, Some(n)) => YamlValue::Int(n),
			_ => YamlValue::Str(s.into_owned()),
		},
	})
}

#[cfg(test)]
mod tests {
	use super::*;

	fn root(text: &str) -> YamlNode {
		load(text).expect("document parses")
	}

	#[test]
	fn keys_keep_file_order_and_paths() {
		let doc = root("meta:\n  id: foo\nseq:\n  - id: a\n  - id: b\n");
		let seq = doc.get("seq").unwrap();
		assert_eq!(&*seq.path, "/seq");
		let items = seq.as_seq().unwrap();
		assert_eq!(&*items[1].path, "/seq/1");
		assert_eq!(&*items[1].get("id").unwrap().path, "/seq/1/id");
		assert_eq!(items[1].get("id").unwrap().as_str().unwrap(), "b");
	}

	#[test]
	fn a_quoted_number_is_still_a_string_and_reads_the_same() {
		let doc = root("a: \"1\"\nb: 1\n");
		assert!(matches!(doc.get("a").unwrap().value, YamlValue::Str(_)));
		assert!(matches!(doc.get("b").unwrap().value, YamlValue::Int(1)));
		assert_eq!(doc.get("a").unwrap().as_str().unwrap(), "1");
		assert_eq!(doc.get("b").unwrap().as_str().unwrap(), "1");
	}

	#[test]
	fn hex_and_boolean_keys_become_their_written_out_form() {
		let doc = root("cases:\n  0x10: a\n  true: b\n  '_': c\n");
		let cases = doc.get("cases").unwrap();
		let entries = cases.as_map().unwrap();
		assert_eq!(entries[0].key_text, "16");
		assert_eq!(entries[1].key_text, "true");
		assert_eq!(entries[2].key_text, "_");
		assert_eq!(&*entries[0].value.path, "/cases/16");
	}

	#[test]
	fn an_enum_key_wider_than_64_bits_is_still_a_number() {
		let doc = root("enums:\n  e:\n    18446744073709551615: big\n");
		let e = doc.get("enums").unwrap().get("e").unwrap();
		assert_eq!(e.as_map().unwrap()[0].key.as_big_int().unwrap(), 18446744073709551615i128);
	}

	#[test]
	fn contents_reads_as_a_string_a_list_or_a_mixed_list() {
		let doc = root("a:\n  contents: MZ\nb:\n  contents: [0x4d, 0x5a]\nc:\n  contents: [CAFE, 0, 0xff]\n");
		assert!(matches!(doc.get("a").unwrap().get("contents").unwrap().value, YamlValue::Str(_)));
		let b = doc.get("b").unwrap().get("contents").unwrap().as_seq().unwrap();
		assert_eq!(b[0].as_int().unwrap(), 0x4d);
		let c = doc.get("c").unwrap().get("contents").unwrap().as_seq().unwrap();
		assert!(matches!(c[0].value, YamlValue::Str(_)));
		assert_eq!(c[2].as_int().unwrap(), 255);
	}

	#[test]
	fn a_repeated_key_is_an_error() {
		let err = load("seq:\n  - id: a\nseq:\n  - id: b\n").unwrap_err();
		assert_eq!(err.message, "found duplicate key seq");
	}

	#[test]
	fn a_single_scalar_reads_as_a_list_of_one() {
		let doc = root("doc-ref: https://example.invalid/spec\n");
		assert_eq!(doc.get_list_str("doc-ref").unwrap().len(), 1);
		let doc = root("imports:\n  - a\n  - b\n");
		assert_eq!(doc.get_list_str("imports").unwrap(), vec!["a".to_string(), "b".to_string()]);
	}

	#[test]
	fn a_size_is_an_int_or_an_expression_string() {
		let doc = root("a:\n  size: 4\nb:\n  size: len_body - 4\n");
		assert_eq!(doc.get("a").unwrap().get("size").unwrap().as_str().unwrap(), "4");
		assert_eq!(doc.get("b").unwrap().get("size").unwrap().as_str().unwrap(), "len_body - 4");
	}
}
