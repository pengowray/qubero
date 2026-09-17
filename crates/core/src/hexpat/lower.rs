//! The declarative half of a `.hexpat` lowered to the template IR.
//!
//! The rule this file is built on is the one `ksy/lower.rs` is built on: a
//! construct the IR cannot express is named, never approximated. Every field
//! ends up in the report saying what it became; anything that could not be said
//! ends up there as a gap, with the line and column, the source text and the
//! reason, and the field is left as bytes of whatever length is still known.
//! Where the mapping is exact but indirect, that is a note: a `[[format]]`
//! whose function would have printed a decoded time is a note saying the raw
//! value is shown, not a guess at what the function would have printed.
//!
//! The imperative half of the language -- `fn`, statement `while` and `for`,
//! assignments, `in`/`out`, `try`, and the `std::` calls that compute -- is
//! never run. The parser keeps each such construct as an [`ast::Statement`]
//! with its source text, and every one of them arrives here as a gap.
//!
//! Nothing is ported from the reference implementation. It was read for the
//! semantics, which is why the names here follow its names.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::report::Report;
use crate::template::{
	Encoding, Endian, EnumDef, Expr, Field, StrLen, StructDef, Template, Time, Ty, Until,
};

use super::ast::{
	ArraySize, AssignTarget, Attribute, BitEntry, BitSign, CasePattern, CaseValue, Decl, EnumEntry, Field as AField,
	FieldKind, MatchArm, Member, NamespaceDecl, Program, StatementKind, TemplateArg, TemplateParam,
	TypeKind, TypeRef,
};
use super::expr::{self, BinOp, ExprKind, PathSeg, TypeOp, TypeOpArg, UnOp};
use super::lexer::{HexpatError, Lit, Pos, ValueType};
use super::types::{self, Known};

/// A `.hexpat` read into the IR, with everything the conversion had to say.
pub struct Converted {
	pub template: Template,
	pub report: Report,
}

/// Attributes the converter acts on.
///
/// An attribute that is not here is read and noted, not refused. The plan said
/// to reject one, and the reference does not: `Attributable::hasAttribute`
/// looks its named attributes up by name and never asks what the others were,
/// so `[[attribute("hidden")]]` in `job.hexpat` runs there and does nothing.
const KNOWN_ATTRIBUTES: &[&str] = &[
	"name", "comment", "inline", "hidden", "sealed", "format", "format_read", "format_write",
	"format_entries", "transform", "transform_entries", "color", "single_color", "static",
	"export", "fixed_size", "no_unique_address", "pointer_base", "bitfield_order", "highlight_hidden",
	"hex::visualize", "hex::inline_visualize", "hex::spec_name", "hex::editor_visualize",
];

/// Pragmas the converter reads, or reads and ignores by name.
///
/// A pragma that is not here is noted, not refused, for the same reason an
/// unknown attribute is: `Preprocessor::process` runs the handler for a pragma
/// it has one for and passes over every other, so `#pragma organization` and
/// `#pragma authors` in the corpus are read by the reference and do nothing.
const KNOWN_PRAGMAS: &[&str] = &[
	"endian", "magic", "description", "author", "once", "array_limit", "pattern_limit",
	"loop_limit", "eval_depth", "source", "sources", "repo", "history", "filename", "name",
	"debug", "base_address", "MIME", "mime", "bitfield_order", "allow_edits", "immediate",
];

/// Lower a parsed pattern, and everything it included, to a template and a
/// report.
pub fn lower(name: &str, program: &Program) -> Result<Converted, HexpatError> {
	let mut decls = Decls::default();
	decls.collect(program, "", &mut HashSet::new());

	let mut lower = Lower {
		file: program.file.clone(),
		decls,
		report: Report::new(),
		types: Vec::new(),
		emitted: HashSet::new(),
		mono: HashMap::new(),
		endian: Endian::Little,
		stack: Vec::new(),
		blocks: 0,
		depth: 0,
		resolving: HashSet::new(),
		folded: HashSet::new(),
		previous_conditional: false,
	};

	let pragmas = lower.read_pragmas(program)?;
	lower.endian = pragmas.endian;

	let root = lower.root(program, &pragmas)?;
	let mut template = Template::new(name, root);
	for (name, ty) in lower.types.drain(..) {
		template = template.with_type(&name, ty);
	}
	Ok(Converted { template, report: lower.report })
}

/// What the `#pragma` lines said.
struct Pragmas {
	endian: Endian,
	/// `#pragma magic [ 4D 5A ] @ 0x00`, as bytes and an address. The address
	/// may be negative, which counts back from the end of the file.
	magic: Option<(Vec<u8>, i64)>,
	description: Option<String>,
}

/* ------------------------------------------------------------------ */
/* Where a declared name lives                                         */
/* ------------------------------------------------------------------ */

/// One declared type, and the namespace whose names its body can see.
enum Declared<'a> {
	Struct(&'a super::ast::StructDef, String),
	Enum(&'a super::ast::EnumDef, String),
	Bitfield(&'a super::ast::BitfieldDef, String),
	Using(&'a super::ast::UsingDecl, String),
}

impl Declared<'_> {
	fn namespace(&self) -> &str {
		match self {
			Declared::Struct(_, ns) | Declared::Enum(_, ns) | Declared::Bitfield(_, ns) | Declared::Using(_, ns) => ns,
		}
	}
	fn template(&self) -> &[TemplateParam] {
		match self {
			Declared::Struct(def, _) => &def.template,
			Declared::Bitfield(def, _) => &def.template,
			Declared::Using(def, _) => &def.template,
			Declared::Enum(_, _) => &[],
		}
	}
}

/// Every type the pattern and its includes declare, by `::`-joined name.
#[derive(Default)]
struct Decls<'a> {
	by_name: HashMap<String, Declared<'a>>,
}

impl<'a> Decls<'a> {
	fn collect(&mut self, program: &'a Program, ns: &str, seen: &mut HashSet<String>) {
		if !seen.insert(program.file.clone()) {
			return;
		}
		self.collect_decls(&program.decls, ns, seen);
	}

	fn collect_decls(&mut self, decls: &'a [Decl], ns: &str, seen: &mut HashSet<String>) {
		for decl in decls {
			match decl {
				Decl::Include { program: Some(inner), .. } => self.collect(inner, ns, seen),
				Decl::Import(import) => {
					if let Some(inner) = &import.program {
						self.collect(inner, ns, seen);
					}
				}
				Decl::Namespace(NamespaceDecl { path, decls, .. }) => {
					let inner = join(ns, &path.join("::"));
					self.collect_decls(decls, &inner, seen);
				}
				// A declaration's name is already the qualified one: the
				// parser resolves names as it goes, because it has to know what
				// a name in type position means before it can read the rest.
				Decl::Struct(def) => {
					self.by_name.entry(def.name.clone()).or_insert(Declared::Struct(def, ns.to_string()));
				}
				Decl::Enum(def) => {
					self.by_name.entry(def.name.clone()).or_insert(Declared::Enum(def, ns.to_string()));
				}
				Decl::Bitfield(def) => {
					self.by_name.entry(def.name.clone()).or_insert(Declared::Bitfield(def, ns.to_string()));
				}
				Decl::Using(def) if def.ty.is_some() => {
					self.by_name.entry(def.name.clone()).or_insert(Declared::Using(def, ns.to_string()));
				}
				_ => {}
			}
		}
	}

	/// The declaration `path` names. A path at a use site is already the
	/// qualified one, for the reason a declaration's name is.
	fn find(&self, _ns: &str, path: &str) -> Option<(&str, &Declared<'a>)> {
		self.by_name.get_key_value(path).map(|(name, decl)| (name.as_str(), decl))
	}

	/// The one declaration whose qualified name ends in `path`, for a name the
	/// parser did not resolve.
	///
	/// A type in type position arrives already qualified; an `E::Label` in an
	/// expression does not, and `[[bitfield_order(BitfieldOrder::..., 8)]]` in
	/// `gzip.hexpat` names an enumeration `std/core.pat` declares inside two
	/// `namespace auto`s. Only taken when exactly one name ends that way, so a
	/// tail two files both use is a miss rather than a guess.
	fn find_by_tail(&self, path: &str) -> Option<(&str, &Declared<'a>)> {
		let suffix = format!("::{path}");
		let mut found = None;
		for (name, decl) in &self.by_name {
			if name.ends_with(&suffix) {
				if found.is_some() {
					return None;
				}
				found = Some((name.as_str(), decl));
			}
		}
		found
	}
}

fn join(ns: &str, name: &str) -> String {
	if ns.is_empty() {
		name.to_string()
	} else {
		format!("{ns}::{name}")
	}
}

/* ------------------------------------------------------------------ */
/* Scope                                                               */
/* ------------------------------------------------------------------ */

/// One level of the name scope a pattern's expressions are read in.
struct Frame {
	/// Fields declared here so far, in order.
	names: Vec<String>,
	/// Names declared inside an `if` or a `match` arm at this level. They exist
	/// in the pattern's scope and sit one structure deeper in the IR, where a
	/// later sibling's `Ref` does not reach them.
	hidden: Vec<String>,
	/// Names this level declares and the converter cannot read a value for,
	/// with the reason. A pattern that names one of these is not naming
	/// something that does not exist, which is what "not a field in scope"
	/// would say; it is naming a value only the imperative half settles.
	unreadable: HashMap<String, String>,
	/// Whether anything has been placed at this level yet, and how to name
	/// where the structure starts once something has. `addressof(this)` is the
	/// enclosing structure's own start, which the IR has no expression for:
	/// before the first field it is where the reading is, and after it it is
	/// the start of the first field. A first field only some condition reads
	/// leaves this `None`, since a name that may not be there cannot say where
	/// the structure began.
	started: bool,
	start: Option<Expr>,
	/// Template value parameters bound at this level.
	params: HashMap<String, Expr>,
	/// Template type parameters bound at this level, so that `sizeof(T)` in a
	/// structure's body reaches what the use site passed.
	bound: HashMap<String, TypeRef>,
	/// True for the inline structure an `if` block or a `match` arm became,
	/// which is a level in the IR and not one in the pattern.
	block: bool,
}

impl Frame {
	fn new(block: bool) -> Frame {
		Frame {
			names: Vec::new(),
			hidden: Vec::new(),
			unreadable: HashMap::new(),
			started: false,
			start: None,
			params: HashMap::new(),
			bound: HashMap::new(),
			block,
		}
	}
}

/* ------------------------------------------------------------------ */
/* The lowering                                                        */
/* ------------------------------------------------------------------ */

struct Lower<'a> {
	file: String,
	decls: Decls<'a>,
	report: Report,
	types: Vec<(String, Ty)>,
	emitted: HashSet<String>,
	/// Monomorphised instantiations, from `name<args>@endian` to the IR name.
	mono: HashMap<String, String>,
	endian: Endian,
	stack: Vec<Frame>,
	/// How many `if`/`match` blocks have been named, so each gets its own name.
	blocks: u32,
	depth: u32,
	/// Enumerations being read, so that an entry naming its own enumeration
	/// stops rather than reading it again from the top.
	resolving: HashSet<String>,
	/// Assignments already said as part of a local one `if` settles, by the
	/// address of the statement, so the walk through the `if`'s own members
	/// passes over them rather than reporting the same thing twice.
	folded: HashSet<usize>,
	/// Whether the last thing placed at the top level was placed inside a
	/// block of a top-level `if`. A `@ $` after one of those means the end of
	/// a field that is only there when the condition held, which is not one
	/// address, so it is a gap rather than a guess at which.
	previous_conditional: bool,
}

/// What happened to one field that could not be said.
struct Gap {
	reason: String,
	/// What to leave in the field's place, if anything can be.
	fallback: Option<Ty>,
}

impl Gap {
	fn new(reason: impl Into<String>) -> Gap {
		Gap { reason: reason.into(), fallback: None }
	}
}

type R<T> = Result<T, Gap>;

impl<'a> Lower<'a> {
	fn error(&self, pos: Pos, message: impl Into<String>) -> HexpatError {
		HexpatError::new(self.file.clone(), pos, message)
	}

	fn at(&self, pos: Pos) -> String {
		format!("{}:{}", pos.line, pos.col)
	}

	/* -------------------------------------------------------------- */
	/* Pragmas                                                         */
	/* -------------------------------------------------------------- */

	fn read_pragmas(&mut self, program: &Program) -> Result<Pragmas, HexpatError> {
		let mut out = Pragmas { endian: Endian::Little, magic: None, description: None };
		for pragma in every_pragma(program) {
			let value = pragma.value.trim();
			if !KNOWN_PRAGMAS.contains(&pragma.key.as_str()) {
				self.report.note(
					self.at(pragma.pos),
					format!("#pragma {} {value}", pragma.key),
					"a pragma the reference has no handler for, which it reads and does nothing with",
				);
				continue;
			}
			match pragma.key.as_str() {
				"endian" => match value {
					"little" => out.endian = Endian::Little,
					"big" => out.endian = Endian::Big,
					"native" => out.endian = Endian::Little,
					other => return Err(self.error(pragma.pos, format!("Invalid endian {other}"))),
				},
				"magic" => match parse_magic(value) {
					Some(found) => out.magic = Some(found),
					None => self.report.note(
						self.at(pragma.pos),
						format!("#pragma magic {value}"),
						"a magic that starts with a byte it does not care about, which nothing can claim a file by",
					),
				},
				"description" => out.description = Some(value.to_string()),
				"author" => {
					self.report.note(self.at(pragma.pos), format!("#pragma author {value}"), "kept for the panel");
				}
				_ => {
					self.report.note(
						self.at(pragma.pos),
						format!("#pragma {} {value}", pragma.key),
						"read and ignored: it says how the reference runs the pattern, not what the bytes are",
					);
				}
			}
		}
		Ok(out)
	}

	/* -------------------------------------------------------------- */
	/* The root                                                        */
	/* -------------------------------------------------------------- */

	fn root(&mut self, program: &'a Program, pragmas: &Pragmas) -> Result<Ty, HexpatError> {
		let mut fields: Vec<Field> = Vec::new();
		let mut machinery: Vec<Arc<str>> = Vec::new();

		if let Some((bytes, at)) = &pragmas.magic {
			// The one thing that claims a dropped file. The pattern reads those
			// bytes again where it declares them, so this reading is a second
			// one and is not counted twice.
			let magic = Ty::Magic(bytes.clone());
			// A negative address counts back from the end of the file, which
			// is what a VHD footer is: the last 512 bytes.
			let ty = match at {
				0 => magic,
				at if *at > 0 => Ty::at(Expr::lit(i128::from(*at)), magic),
				at => Ty::at(Expr::SpaceSize.add(Expr::lit(i128::from(*at))), magic),
			};
			if *at < 0 {
				self.report.note(
					"0:0",
					format!("#pragma magic @ -{:#x}", -at),
					"a magic measured back from the end of the file is read there, and nothing claims a dropped file by it: the file sniffer matches a signature at a fixed offset from the front",
				);
			} else if *at != 0 {
				self.report.note(
					"0:0",
					format!("#pragma magic @ {at:#x}"),
					"a magic away from the start of the file is read, but nothing claims a dropped file by it",
				);
			}
			fields.push(named_field("magic", ty, true));
			machinery.push("magic".into());
		}

		self.stack.push(Frame::new(false));
		let mut previous: Option<String> = None;
		self.root_decls(&program.decls, &mut fields, &mut machinery, &mut previous, &mut HashSet::new())?;
		self.stack.pop();

		let mut def = empty_struct(&program_name(program));
		def.doc = pragmas.description.as_deref().map(Arc::from);
		def.fields = fields;
		def.machinery = machinery;
		Ok(Ty::Struct(Arc::new(def)))
	}

	fn root_decls(
		&mut self,
		decls: &'a [Decl],
		fields: &mut Vec<Field>,
		machinery: &mut Vec<Arc<str>>,
		previous: &mut Option<String>,
		seen: &mut HashSet<String>,
	) -> Result<(), HexpatError> {
		for decl in decls {
			self.mark_start(fields);
			match decl {
				Decl::Include { program: Some(inner), .. } => {
					if seen.insert(inner.file.clone()) {
						self.root_decls(&inner.decls, fields, machinery, previous, seen)?;
					}
				}
				Decl::Namespace(NamespaceDecl { decls, .. }) => {
					self.root_decls(decls, fields, machinery, previous, seen)?
				}
				Decl::Placement(placement) => {
					self.placement(placement, fields, machinery, previous)?;
				}
				Decl::Call { path, args, pos } => self.top_call(path, args, *pos),
				Decl::Statement(statement) => self.top_statement(statement, fields, machinery, previous)?,
				_ => {}
			}
		}
		Ok(())
	}

	/// One imperative statement at the top level.
	///
	/// Two of them are not imperative at all once they are read: a call is the
	/// same call it would be outside an `if`, and an `if` whose blocks place
	/// fields is a condition over those placements, which is a `When`. The
	/// rest are gaps, as they were.
	fn top_statement(
		&mut self,
		statement: &'a super::ast::Statement,
		fields: &mut Vec<Field>,
		machinery: &mut Vec<Arc<str>>,
		previous: &mut Option<String>,
	) -> Result<(), HexpatError> {
		match statement.kind {
			StatementKind::Call => {
				if let Some(call) = &statement.call {
					self.top_call(&call.0, &call.1, statement.pos);
					return Ok(());
				}
			}
			StatementKind::Local => {
				if let Some(field) = &statement.decl {
					self.placement(field, fields, machinery, previous)?;
					return Ok(());
				}
			}
			StatementKind::If => {
				// Only worth taking apart when a block places something. An
				// `if` of `std::print` calls, assignments and `return`s reads
				// nothing, and one gap naming the `if` says more than one gap
				// per statement inside it.
				if let Some(branches) = &statement.branches {
					let halves = [&branches.then, &branches.otherwise];
					if halves.iter().any(|half| places_anything(half))
						&& halves.iter().all(|half| every_statement_says_something(half))
					{
						return self.top_conditional(statement, fields, machinery, previous);
					}
				}
			}
			_ => {}
		}
		self.report.gap(
			self.at(statement.pos),
			first_line(&statement.text),
			format!("{} at the top level, which the converter does not run", kind_name(statement.kind)),
		);
		Ok(())
	}

	/// A top-level `if`, as one `When` per block over an inline structure of
	/// what the block places.
	///
	/// This is the same lowering an `if` inside a structure gets, and it works
	/// at the top level for the same reason the placements around it do: every
	/// top-level field reads at an address of its own, so a block that could
	/// not be said moves nothing. A block that places nothing leaves no field
	/// behind, which is what a block of `std::print` calls should leave.
	fn top_conditional(
		&mut self,
		statement: &'a super::ast::Statement,
		fields: &mut Vec<Field>,
		machinery: &mut Vec<Arc<str>>,
		previous: &mut Option<String>,
	) -> Result<(), HexpatError> {
		let branches = statement.branches.as_ref().expect("the two halves");
		let path = self.at(statement.pos);
		let source = format!("if ({})", branches.cond);
		let cond = match self.expr(&branches.cond) {
			Ok(cond) => cond,
			Err(gap) => {
				self.report.gap(path, source, gap.reason);
				return Ok(());
			}
		};
		self.report.became(path, source, "one condition over the whole block, not one per field");
		// What `@ $` means outside the `if` is the end of the last placement
		// outside it, which the blocks' own placements must not move.
		let before = previous.clone();
		let mut placed_in_a_block = false;
		for (half, negated) in [(&branches.then, false), (&branches.otherwise, true)] {
			if half.is_empty() {
				continue;
			}
			*previous = before.clone();
			self.blocks += 1;
			let name = format!("{}_{}", if negated { "else" } else { "if" }, self.blocks);
			let mut inner: Vec<Field> = Vec::new();
			let mut inner_machinery: Vec<Arc<str>> = Vec::new();
			self.stack.push(Frame::new(true));
			for member in half {
				self.top_statement(member, &mut inner, &mut inner_machinery, previous)?;
			}
			let frame = self.stack.pop().expect("a frame");
			if let Some(outer) = self.stack.last_mut() {
				outer.hidden.extend(frame.names);
				outer.hidden.extend(frame.hidden);
			}
			if inner.is_empty() {
				continue;
			}
			let mut def = empty_struct(&name);
			def.inline = true;
			def.fields = inner;
			def.machinery.extend(inner_machinery);
			let ty = Ty::Struct(Arc::new(def));
			let when = if negated { cond.clone().negate() } else { cond.clone() };
			machinery.push(Arc::from(name.as_str()));
			fields.push(named_field(&name, Ty::when(when, ty), false));
			placed_in_a_block = true;
		}
		*previous = before;
		if placed_in_a_block {
			self.previous_conditional = true;
		}
		Ok(())
	}

	/// One top-level variable. Only a declaration with an `@` reads the file.
	fn placement(
		&mut self,
		placement: &'a AField,
		fields: &mut Vec<Field>,
		machinery: &mut Vec<Arc<str>>,
		previous: &mut Option<String>,
	) -> Result<(), HexpatError> {
		let name = placement.name().to_string();
		let source = field_source(placement);
		let path = self.at(placement.pos);
		match placement.kind {
			FieldKind::Local => {
				// `T x;` and `T x = e;` at the top level read nothing. The
				// second is a value the pattern works out, and later
				// placements name it, so it is kept as a zero-width field the
				// way a local inside a structure is.
				let name = unique(&name, fields);
				match &placement.init {
					None => {
						self.report.note(path, source, "a global variable, which reads nothing");
						self.cannot_read(
							&name,
							format!("{name} is a global the pattern fills in while it runs, and the converter runs nothing"),
						);
					}
					Some(init) => match self.expr(init) {
						Ok(value) => {
							self.report.became(path, source, "a value worked out before anything is read");
							machinery.push(Arc::from(name.as_str()));
							self.stack.last_mut().expect("a frame").names.push(name.clone());
							fields.push(named_field(&name, Ty::computed(value), false));
						}
						Err(gap) => {
							let at = self.at(placement.pos);
							self.report.gap(path, source, gap.reason);
							self.cannot_read(&name, format!("{name}, declared at {at}, has no value the converter could work out"));
						}
					},
				}
				return Ok(());
			}
			FieldKind::In | FieldKind::Out => {
				self.report.gap(path, source, "an in/out variable, which the host supplies rather than the file");
				self.cannot_read(&name, format!("{name} is a variable the host supplies rather than the file"));
				return Ok(());
			}
			FieldKind::Normal => {}
		}
		let Some(address) = &placement.placement else {
			self.report.gap(path, source, "a top-level variable with no address, which reads nothing");
			return Ok(());
		};
		// `@ $` means the end of the placement before it: nothing has been read
		// at the top level except what the earlier placements placed.
		let at = self.at(placement.pos);
		let address = match self.top_address(address, previous.as_deref()) {
			Ok(address) => address,
			Err(gap) => {
				self.report.gap(path, source, gap.reason);
				self.cannot_read(&name, format!("{name}, at {at}, is placed at an address the converter could not work out"));
				return Ok(());
			}
		};
		match self.one_field(placement) {
			Ok(mut field) => {
				field.ty = Ty::at(address, field.ty);
				self.report.became(path, source, format!("placed at an address, as {}", field.ty.display_name()));
				if is_machinery(placement) {
					machinery.push(field.name.clone());
				}
				*previous = Some(field.name.to_string());
				self.previous_conditional = false;
				self.stack.last_mut().expect("a frame").names.push(name);
				fields.push(field);
			}
			Err(gap) => {
				self.report.gap(path, source, gap.reason);
				if gap.fallback.is_none() {
					self.cannot_read(&name, format!("{name}, at {at}, is a field the converter could not read"));
				}
				if let Some(ty) = gap.fallback {
					let mut field = named_field(&name, Ty::at(address, ty), false);
					field.doc = Some(Arc::from("the converter could not say what this is; see the report"));
					*previous = Some(field.name.to_string());
					self.previous_conditional = false;
					self.stack.last_mut().expect("a frame").names.push(name);
					fields.push(field);
				}
			}
		}
		Ok(())
	}

	/// The address of a top-level placement, where `$` is the end of the
	/// placement before it rather than a position in a structure being read.
	fn top_address(&mut self, address: &super::expr::Expr, previous: Option<&str>) -> R<Expr> {
		if let ExprKind::Path(segments) = &address.kind {
			if segments.len() == 1 && matches!(segments[0], PathSeg::Dollar) {
				if self.previous_conditional {
					return Err(Gap::new(
						"`@ $` after a top-level `if` that placed a field, where the end of the placement before this one depends on the condition and is not one address",
					));
				}
				return match previous {
					Some(name) => Ok(Expr::start_of(Expr::field(name)).add(Expr::size_of(name))),
					None => Ok(Expr::lit(0)),
				};
			}
		}
		self.expr(address)
	}

	fn top_call(&mut self, path: &str, args: &[super::expr::Expr], pos: Pos) {
		let source = format!("{path}({})", args.iter().map(|a| a.to_string()).collect::<Vec<_>>().join(", "));
		match self.call_note(path, args) {
			Some(message) => self.report.note(self.at(pos), source, message),
			None => self.report.gap(
				self.at(pos),
				source,
				format!("{path} is run by the reference, and the converter runs nothing"),
			),
		}
	}

	/// What a call written as a statement is worth saying about, for the calls
	/// that only display or check something.
	fn call_note(&self, path: &str, args: &[super::expr::Expr]) -> Option<String> {
		Some(match path {
			"std::assert" | "std::assert_warn" => match args.first() {
				Some(condition) => format!("the pattern checks that {condition}; the IR reads the field either way"),
				None => "a check the IR does not make".to_string(),
			},
			"std::print" | "std::format" => "text the reference prints, which changes nothing about the bytes".to_string(),
			"std::warning" | "std::error" | "std::unimplemented" => {
				"a message the reference shows; the bytes are read the same way".to_string()
			}
			"std::core::set_display_name" => "the reference renames the field it is called in".to_string(),
			_ => return None,
		})
	}

	/* -------------------------------------------------------------- */
	/* Structures                                                      */
	/* -------------------------------------------------------------- */

	/// The IR name for one instantiation of a declared type, lowering it the
	/// first time it is asked for.
	fn instantiate(&mut self, ir_name: &str, decl_name: &str, ns: &str, args: &[TemplateArg], endian: Endian) -> R<Ty> {
		let key = format!("{decl_name}<{}>@{endian:?}", args.iter().map(arg_key).collect::<Vec<_>>().join(","));
		if let Some(name) = self.mono.get(&key) {
			return Ok(Ty::Named(Arc::from(name.as_str())));
		}
		if self.depth > 48 {
			return Err(Gap::new("the type nests deeper than the converter follows"));
		}
		let name = if args.is_empty() && endian == self.endian {
			ir_name.to_string()
		} else {
			let mut candidate = ir_name.to_string();
			let mut n = 2;
			while self.emitted.contains(&candidate) || self.mono.values().any(|v| *v == candidate) {
				candidate = format!("{ir_name}#{n}");
				n += 1;
			}
			candidate
		};
		self.mono.insert(key, name.clone());
		self.emitted.insert(name.clone());
		self.depth += 1;
		let lowered = self.declared(decl_name, ns, args, endian, &name);
		self.depth -= 1;
		match lowered {
			Ok(ty) => {
				self.types.push((name.clone(), ty));
				Ok(Ty::Named(Arc::from(name.as_str())))
			}
			Err(gap) => {
				// The name was claimed before the body was lowered, so that a
				// type referring to itself terminates. A body that could not be
				// lowered leaves the name meaning nothing rather than nothing
				// at all, since something may already refer to it.
				self.types.push((name.clone(), gap.fallback.clone().unwrap_or(Ty::Bytes(Expr::lit(0)))));
				Err(gap)
			}
		}
	}

	fn declared(&mut self, decl_name: &str, ns: &str, args: &[TemplateArg], endian: Endian, ir_name: &str) -> R<Ty> {
		let params: Vec<TemplateParam> = match self.decls.by_name.get(decl_name) {
			Some(decl) => decl.template().to_vec(),
			None => return Err(Gap::new(format!("{decl_name} is not declared"))),
		};
		let mut values: HashMap<String, Expr> = HashMap::new();
		let mut types: HashMap<String, TypeRef> = HashMap::new();
		for (param, arg) in params.iter().zip(args.iter()) {
			match (param.is_type, arg) {
				(true, TemplateArg::Type(ty)) => {
					types.insert(param.name.clone(), ty.clone());
				}
				(false, TemplateArg::Value(value)) => {
					let lowered = self.expr(value)?;
					values.insert(param.name.clone(), lowered);
				}
				(true, TemplateArg::Value(value)) => {
					return Err(Gap::new(format!("{} wants a type and was given {value}", param.name)))
				}
				(false, TemplateArg::Type(ty)) => {
					return Err(Gap::new(format!("{} wants a value and was given {}", param.name, expr::type_name(ty))))
				}
			}
		}
		// Re-borrowed after the arguments are lowered, which needs `self`.
		let decl = self.decls.by_name.get(decl_name).expect("checked above");
		let ns = if decl.namespace().is_empty() { ns.to_string() } else { decl.namespace().to_string() };
		match decl {
			Declared::Struct(def, _) => {
				let def = *def;
				self.structure(def, &ns, endian, ir_name, values, types)
			}
			Declared::Enum(def, _) => {
				let def = *def;
				self.enumeration(def, &ns, endian, ir_name)
			}
			Declared::Bitfield(def, _) => {
				let def = *def;
				self.bitfield(def, &ns, endian, ir_name, values, types)
			}
			Declared::Using(def, _) => {
				let def = *def;
				let Some(ty) = &def.ty else { return Err(Gap::new(format!("{decl_name} has no body"))) };
				let mut frame = Frame::new(false);
				frame.params = values;
				frame.bound = types.clone();
				self.stack.push(frame);
				let lowered = self.type_ref_in(ty, &ns, endian, &types);
				self.stack.pop();
				let ty = lowered?;
				self.apply_attributes(&def.attrs, ty, def.pos, &def.name)
			}
		}
	}

	#[allow(clippy::too_many_arguments)]
	fn structure(
		&mut self,
		def: &'a super::ast::StructDef,
		ns: &str,
		endian: Endian,
		ir_name: &str,
		values: HashMap<String, Expr>,
		types: HashMap<String, TypeRef>,
	) -> R<Ty> {
		let mut frame = Frame::new(false);
		frame.params = values;
		frame.bound = types.clone();
		self.stack.push(frame);

		let mut fields: Vec<Field> = Vec::new();
		let mut machinery: Vec<Arc<str>> = Vec::new();

		// `struct S : Base` reads Base's members first.
		for base in &def.inherits {
			match self.base_members(base, ns, endian, &types) {
				Ok(mut inherited) => {
					for field in &inherited {
						self.stack.last_mut().expect("a frame").names.push(field.name.to_string());
					}
					fields.append(&mut inherited);
				}
				Err(gap) => self.report.gap(
					self.at(base.pos),
					format!("struct {} : {}", def.name, expr::type_name(base)),
					gap.reason,
				),
			}
		}

		self.members(&def.members, ns, endian, &types, &mut fields, &mut machinery);
		self.stack.pop();

		let mut out = empty_struct(ir_name);
		out.doc = def.doc.as_deref().map(Arc::from);
		out.overlap = def.union;
		out.fields = fields;
		out.machinery.extend(machinery);
		let ty = Ty::Struct(Arc::new(out));
		self.apply_attributes(&def.attrs, ty, def.pos, &def.name)
	}

	fn base_members(
		&mut self,
		base: &TypeRef,
		ns: &str,
		endian: Endian,
		types: &HashMap<String, TypeRef>,
	) -> R<Vec<Field>> {
		let TypeKind::Named { path, args } = &base.kind else {
			return Err(Gap::new("a struct can only inherit from a declared type"));
		};
		let Some((name, decl)) = self.decls.find(ns, path) else {
			return Err(Gap::new(format!("{path} is not declared")));
		};
		let Declared::Struct(def, _) = decl else {
			return Err(Gap::new(format!("{path} is not a struct, so its members cannot be read first")));
		};
		let def: &'a super::ast::StructDef = def;
		let inner_ns = decl.namespace().to_string();
		let params = def.template.to_vec();
		let _ = name;
		// The base's members are read here, so its parameters are bound here
		// too: `struct S : Record<RecordType::Header>` reads Record's members
		// with `RecordType::Header` put in for its parameter.
		let args = self.substitute(args, types);
		let mut inner_types = types.clone();
		let mut values: HashMap<String, Expr> = HashMap::new();
		for (param, arg) in params.iter().zip(args.iter()) {
			match (param.is_type, arg) {
				(true, TemplateArg::Type(ty)) => {
					inner_types.insert(param.name.clone(), ty.clone());
				}
				(false, TemplateArg::Value(value)) => {
					let lowered = self.expr(value)?;
					values.insert(param.name.clone(), lowered);
				}
				_ => return Err(Gap::new(format!("{} was given the wrong kind of argument", param.name))),
			}
		}
		if let Some(frame) = self.stack.last_mut() {
			frame.params.extend(values);
			frame.bound.extend(inner_types.clone());
		}
		let mut fields = Vec::new();
		let mut machinery = Vec::new();
		self.members(&def.members, &inner_ns, endian, &inner_types, &mut fields, &mut machinery);
		Ok(fields)
	}

	fn members(
		&mut self,
		members: &'a [Member],
		ns: &str,
		endian: Endian,
		types: &HashMap<String, TypeRef>,
		fields: &mut Vec<Field>,
		machinery: &mut Vec<Arc<str>>,
	) {
		// A local the pattern declares and then sets in the two halves of one
		// `if` is a value that depends on the condition and on nothing else,
		// which the IR says with `Cond`. Working that out before the walk is
		// what lets the declaration wait until after the `if`, where the value
		// is settled, and lets the assignments themselves pass without a gap.
		let folded = self.conditional_locals(members);
		for (index, member) in members.iter().enumerate() {
			if folded.decls.contains_key(&index) {
				continue;
			}
			self.mark_start(fields);
			// A member that should have read bytes and could not moves
			// everything after it, so the structure ends there rather than
			// carrying on at an offset nothing in the file agrees with.
			let carried_on = match member {
				Member::Field(field) => self.member_field(field, ns, endian, types, members, fields, machinery),
				Member::Padding { size, pos, .. } => {
					let ty = match size {
						ArraySize::Count(count) => self.expr(count).map(Ty::bytes),
						ArraySize::Unbounded => Ok(Ty::bytes(Expr::Remaining)),
						ArraySize::While(_) => Err(Gap::new("padding whose length is a condition is not lowered")),
					};
					match ty {
						Ok(ty) => {
							let name = unique("padding", fields);
							self.report.became(self.at(*pos), "padding", "bytes read and not shown");
							machinery.push(Arc::from(name.as_str()));
							self.stack.last_mut().expect("a frame").names.push(name.clone());
							fields.push(named_field(&name, ty, false));
							true
						}
						Err(gap) => {
							self.report.gap(self.at(*pos), "padding", gap.reason);
							false
						}
					}
				}
				Member::If { cond, then, otherwise, pos } => {
					let carried_on =
						self.conditional(cond, then, otherwise, *pos, ns, endian, types, fields, machinery);
					if let Some(waiting) = folded.at_if.get(&index) {
						for decl in waiting {
							self.folded_local(members, cond, *decl, fields, machinery);
						}
					}
					carried_on
				}
				Member::Match { scrutinee, arms, pos } => {
					self.match_member(scrutinee, arms, *pos, ns, endian, types, fields, machinery)
				}
				Member::Try { pos, .. } => {
					self.report.gap(
						self.at(*pos),
						"try",
						"a try/catch reads one shape and falls back on another while the pattern runs",
					);
					false
				}
				Member::Call { path, args, pos } => {
					self.top_call(path, args, *pos);
					true
				}
				Member::Statement(statement) => self.statement_member(statement, fields, machinery),
			};
			if !carried_on {
				self.report.gap(
					self.at(member_pos(member)),
					"the rest of the structure",
					"left unread: the member before it was not placed, so everything after it would be at an offset nothing in the file agrees with",
				);
				break;
			}
		}
	}

	/// One statement inside a structure. Two shapes of assignment to `$` are
	/// exact and the rest are gaps.
	///
	/// `$ += e` moves the cursor forward by `e` bytes and nothing else, which
	/// is what `padding[e]` says, so it lowers to the same bytes read and not
	/// shown. `$ = e` and `$ -= e` put the cursor somewhere the structure
	/// cannot follow: the reference sizes a structure as the distance from
	/// where it started to where the cursor ended
	/// (`ASTNodeStruct::createPatterns`), and an `At` in the IR advances
	/// nothing, so wrapping the rest of the structure in one would read the
	/// right bytes and report the wrong length. The structure ends there
	/// instead.
	fn statement_member(
		&mut self,
		statement: &'a super::ast::Statement,
		fields: &mut Vec<Field>,
		machinery: &mut Vec<Arc<str>>,
	) -> bool {
		if self.folded.contains(&(statement as *const _ as usize)) {
			return true;
		}
		let path = self.at(statement.pos);
		let source = first_line(&statement.text);
		if let Some(assign) = &statement.assign {
			if assign.target == AssignTarget::Dollar {
				if assign.op == Some(BinOp::Add) {
					return match self.expr(&assign.value) {
						Ok(value) => {
							let name = unique("padding", fields);
							self.report.became(path, source, "bytes skipped, read and not shown");
							machinery.push(Arc::from(name.as_str()));
							self.stack.last_mut().expect("a frame").names.push(name.clone());
							fields.push(named_field(&name, Ty::bytes(value), false));
							true
						}
						Err(gap) => {
							self.report.gap(path, source, gap.reason);
							false
						}
					};
				}
				self.report.gap(
					path,
					source,
					"the cursor moved to an address of its own, and a structure in the IR reads straight through from where it started",
				);
				return false;
			}
		}
		self.report.gap(path, source, format!("{}, which the converter does not run", kind_name(statement.kind)));
		// An assignment to a name, or a loop, reads nothing of its own, so what
		// comes after it is still where the file says it is.
		matches!(
			statement.kind,
			StatementKind::Assign
				| StatementKind::Call | StatementKind::Return
				| StatementKind::Break | StatementKind::Continue
				| StatementKind::Local
		)
	}

	/// Which locals of this level are settled by exactly one `if`, and which
	/// members the walk should therefore pass over.
	///
	/// The shape looked for is one declaration with a value, and then one `if`
	/// among the later members whose two halves hold every assignment to that
	/// name, at most one in each half and directly in it. Anything else --
	/// an assignment in a loop, in a nested `if`, in two different `if`s --
	/// stays what it was, a value that changes as the pattern runs.
	fn conditional_locals(&mut self, members: &'a [Member]) -> Folded {
		let mut out = Folded::default();
		for (index, member) in members.iter().enumerate() {
			let Member::Field(field) = member else { continue };
			if field.kind != FieldKind::Local || field.init.is_none() || !field.init_list.is_empty() {
				continue;
			}
			if field.names.len() != 1 {
				continue;
			}
			let name = field.name();
			let Some(fold) = fold_of(members, index, name) else { continue };
			for (then_half, at) in fold.assignments() {
				if let Some(statement) = branch_statement(&members[fold.at], then_half, at) {
					self.folded.insert(statement as *const _ as usize);
				}
			}
			// The declaration waits for its `if`, so a reference inside the
			// `if` itself has nothing to name yet. Say that rather than let it
			// fall through to "not a field in scope here".
			let at = self.at(member_pos(&members[fold.at]));
			self.cannot_read(
				name,
				format!("{name} is settled by the `if` at {at} and can be read after it, not inside it"),
			);
			out.at_if.entry(fold.at).or_default().push(index);
			out.decls.insert(index, fold);
		}
		out
	}

	/// The `Cond` a folded local became, emitted where the `if` ends because
	/// that is where the pattern has settled the value.
	fn folded_local(
		&mut self,
		members: &'a [Member],
		cond: &super::expr::Expr,
		decl: usize,
		fields: &mut Vec<Field>,
		machinery: &mut Vec<Arc<str>>,
	) {
		let Member::Field(field) = &members[decl] else { return };
		let name = field.name().to_string();
		let source = field_source(field);
		let path = self.at(field.pos);
		let Some(fold) = fold_of(members, decl, &name) else { return };
		let lowered: R<Expr> = (|| {
			let cond = self.expr(cond)?;
			let start = self.expr(field.init.as_ref().expect("a value"))?;
			let then = match fold.then {
				Some(at) => self.expr(assigned_value(&members[fold.at], true, at).expect("an assignment"))?,
				None => start.clone(),
			};
			let otherwise = match fold.otherwise {
				Some(at) => self.expr(assigned_value(&members[fold.at], false, at).expect("an assignment"))?,
				None => start,
			};
			Ok(Expr::cond(cond, then, otherwise))
		})();
		match lowered {
			Ok(value) => {
				self.report.became(
					path,
					source,
					"a value the condition of the `if` after it settles, worked out and never read",
				);
				machinery.push(Arc::from(name.as_str()));
				self.stack.last_mut().expect("a frame").names.push(name.clone());
				fields.push(named_field(&name, Ty::computed(value), false));
			}
			Err(gap) => {
				let at = self.at(field.pos);
				self.report.gap(path, source, gap.reason);
				self.cannot_read(&name, format!("{name}, declared at {at}, has no value the converter could work out"));
			}
		}
	}

	#[allow(clippy::too_many_arguments)]
	fn member_field(
		&mut self,
		field: &'a AField,
		ns: &str,
		endian: Endian,
		types: &HashMap<String, TypeRef>,
		siblings: &'a [Member],
		fields: &mut Vec<Field>,
		machinery: &mut Vec<Arc<str>>,
	) -> bool {
		let source = field_source(field);
		let path = self.at(field.pos);
		if field.kind == FieldKind::In || field.kind == FieldKind::Out {
			// An in/out variable reads nothing of the file, so what follows it
			// is still where the file says it is.
			self.report.gap(path, source, "an in/out variable, which the host supplies rather than the file");
			for name in &field.names {
				self.cannot_read(name, format!("{name} is a variable the host supplies rather than the file"));
			}
			return true;
		}
		if field.kind == FieldKind::Local {
			self.local(field, siblings, fields, machinery);
			return true;
		}
		for name in &field.names {
			let lowered = self.field_type(field, ns, endian, types);
			let display = name.clone();
			match lowered {
				Ok(ty) => {
					self.report.became(path.clone(), source.clone(), ty.display_name());
					let mut out = named_field(&display, ty, takes_no_address(field));
					out.doc = self.doc_of(field);
					if is_machinery(field) {
						machinery.push(out.name.clone());
					}
					self.stack.last_mut().expect("a frame").names.push(name.clone());
					fields.push(out);
				}
				Err(gap) => {
					self.report.gap(path.clone(), source.clone(), gap.reason);
					let fallback = gap.fallback.or_else(|| self.static_fallback(field, ns, endian, types));
					match fallback {
						Some(ty) => {
							let mut out = named_field(&display, ty, false);
							out.doc = Some(Arc::from("the converter could not say what this is; see the report"));
							self.stack.last_mut().expect("a frame").names.push(name.clone());
							fields.push(out);
						}
						// A placed field reads at an address of its own, so
						// what follows it has not moved.
						None => return field.placement.is_some(),
					}
				}
			}
		}
		true
	}

	/// The bytes a field takes when the converter could not say what it holds
	/// but its width does not depend on the file.
	fn static_fallback(
		&mut self,
		field: &AField,
		ns: &str,
		endian: Endian,
		types: &HashMap<String, TypeRef>,
	) -> Option<Ty> {
		if field.placement.is_some() || field.pointer.is_some() {
			return None;
		}
		let one = match &field.ty.kind {
			TypeKind::Builtin(value) => u64::from(value.bits()?),
			TypeKind::Named { .. } => {
				let lowered = self.type_ref_in(&field.ty, ns, endian, types).ok()?;
				u64::from(self.static_bits(&lowered)?)
			}
		};
		let count = match &field.array {
			None => 1,
			Some(ArraySize::Count(count)) => u64::try_from(self.constant(count)?).ok()?,
			Some(_) => return None,
		};
		let bits = one.checked_mul(count)?;
		(bits % 8 == 0).then(|| Ty::bytes(Expr::lit(i128::from(bits / 8))))
	}

	/// `u32 x = <expr>;` inside a structure: a value the pattern works out, with
	/// no bytes of its own.
	fn local(&mut self, field: &'a AField, siblings: &'a [Member], fields: &mut Vec<Field>, machinery: &mut Vec<Arc<str>>) {
		let source = field_source(field);
		let path = self.at(field.pos);
		let name = field.name().to_string();
		let at = self.at(field.pos);
		let Some(init) = &field.init else {
			self.report.gap(path, source, "a local with no value, which only a later assignment fills in");
			self.cannot_read(&name, format!("{name}, declared at {at}, is filled in by a later assignment the converter does not run"));
			return;
		};
		if !field.init_list.is_empty() {
			self.report.gap(path, source, "a local list, which the converter does not compute");
			self.cannot_read(&name, format!("{name}, declared at {at}, is a list the converter does not compute"));
			return;
		}
		if assigned_later(siblings, &name) {
			self.report.gap(
				path,
				source,
				"a local the pattern assigns to again, which is a value that changes as the pattern runs",
			);
			self.cannot_read(&name, format!("{name}, declared at {at}, is a value the pattern changes as it runs"));
			return;
		}
		match self.expr(init) {
			Ok(value) => {
				self.report.became(path, source, "a value worked out from the fields around it");
				machinery.push(Arc::from(name.as_str()));
				self.stack.last_mut().expect("a frame").names.push(name.clone());
				fields.push(named_field(&name, Ty::computed(value), false));
			}
			Err(gap) => {
				self.report.gap(path, source, gap.reason);
				self.cannot_read(&name, format!("{name}, declared at {at}, has no value the converter could work out"));
			}
		}
	}

	#[allow(clippy::too_many_arguments)]
	fn conditional(
		&mut self,
		cond: &super::expr::Expr,
		then: &'a [Member],
		otherwise: &'a [Member],
		pos: Pos,
		ns: &str,
		endian: Endian,
		types: &HashMap<String, TypeRef>,
		fields: &mut Vec<Field>,
		machinery: &mut Vec<Arc<str>>,
	) -> bool {
		let source = format!("if ({cond})");
		let lowered = match self.expr(cond) {
			Ok(lowered) => lowered,
			Err(gap) => {
				self.report.gap(self.at(pos), source, gap.reason);
				return false;
			}
		};
		self.report.became(self.at(pos), source, "one condition over the whole block, not one per field");
		let (then_name, then_ty) = self.block(then, "if", ns, endian, types);
		if let Some(ty) = then_ty {
			machinery.push(Arc::from(then_name.as_str()));
			fields.push(named_field(&then_name, Ty::when(lowered.clone(), ty), false));
		}
		if !otherwise.is_empty() {
			let (else_name, else_ty) = self.block(otherwise, "else", ns, endian, types);
			if let Some(ty) = else_ty {
				machinery.push(Arc::from(else_name.as_str()));
				fields.push(named_field(&else_name, Ty::when(lowered.negate(), ty), false));
			}
		}
		true
	}

	/// One block of members as an inline structure, and the name the field
	/// holding it goes by.
	fn block(
		&mut self,
		members: &'a [Member],
		what: &str,
		ns: &str,
		endian: Endian,
		types: &HashMap<String, TypeRef>,
	) -> (String, Option<Ty>) {
		self.blocks += 1;
		let name = format!("{what}_{}", self.blocks);
		self.stack.push(Frame::new(true));
		let mut fields = Vec::new();
		let mut machinery = Vec::new();
		self.members(members, ns, endian, types, &mut fields, &mut machinery);
		let frame = self.stack.pop().expect("a frame");
		// The block's names exist in the pattern's scope from here on and sit a
		// structure deeper in the IR, where a later sibling cannot name them.
		if let Some(outer) = self.stack.last_mut() {
			outer.hidden.extend(frame.names);
			outer.hidden.extend(frame.hidden);
		}
		if fields.is_empty() {
			return (name, None);
		}
		let mut def = empty_struct(&name);
		def.inline = true;
		def.fields = fields;
		def.machinery.extend(machinery);
		(name, Some(Ty::Struct(Arc::new(def))))
	}

	#[allow(clippy::too_many_arguments)]
	fn match_member(
		&mut self,
		scrutinee: &[super::expr::Expr],
		arms: &'a [MatchArm<Member>],
		pos: Pos,
		ns: &str,
		endian: Endian,
		types: &HashMap<String, TypeRef>,
		fields: &mut Vec<Field>,
		machinery: &mut Vec<Arc<str>>,
	) -> bool {
		let source = format!("match ({})", scrutinee.iter().map(|e| e.to_string()).collect::<Vec<_>>().join(", "));
		if scrutinee.len() != 1 {
			self.report.gap(
				self.at(pos),
				source,
				"a match on more than one value, which is a table of combinations the IR has no switch for",
			);
			return false;
		}
		let on = match self.expr(&scrutinee[0]) {
			Ok(on) => on,
			Err(gap) => {
				self.report.gap(self.at(pos), source, gap.reason);
				return false;
			}
		};
		let mut cases: Vec<(i128, Ty)> = Vec::new();
		let mut default = Ty::Bytes(Expr::lit(0));
		for arm in arms {
			let (name, body) = self.block(&arm.body, "case", ns, endian, types);
			let body = body.unwrap_or(Ty::Bytes(Expr::lit(0)));
			let _ = name;
			match &arm.patterns[0] {
				CasePattern::Wildcard => default = body,
				CasePattern::Values(values) => {
					let mut numbers = Vec::new();
					for value in values {
						match self.case_values(value) {
							Ok(mut found) => numbers.append(&mut found),
							Err(gap) => {
								self.report.gap(self.at(arm.pos), source.clone(), gap.reason);
								numbers.clear();
								break;
							}
						}
					}
					for number in numbers {
						cases.push((number, body.clone()));
					}
				}
			}
		}
		if cases.is_empty() {
			self.report.gap(self.at(pos), source, "no arm of the match named a value the converter could read");
			return false;
		}
		self.report.became(self.at(pos), source, "a switch on one value");
		let name = unique("chosen", fields);
		machinery.push(Arc::from(name.as_str()));
		fields.push(named_field(&name, Ty::switch(on, cases, default), false));
		true
	}

	/// The numbers one `match` alternative stands for.
	fn case_values(&mut self, value: &CaseValue) -> R<Vec<i128>> {
		match value {
			CaseValue::One(e) => {
				if let ExprKind::Lit(Lit::Str(text)) = &e.kind {
					let bytes = text.as_bytes();
					if bytes.is_empty() || bytes.len() > 16 {
						return Err(Gap::new(format!(
							"the case {text:?} is {} bytes, which is not a number the IR compares with",
							bytes.len()
						)));
					}
					return Ok(vec![bytes.iter().fold(0i128, |acc, &b| (acc << 8) | i128::from(b))]);
				}
				let Some(n) = self.constant(e) else {
					return Err(Gap::new(format!("the case {e} is not a number the converter can read")));
				};
				Ok(vec![n])
			}
			CaseValue::Range(from, to) => {
				let (Some(from), Some(to)) = (self.constant(from), self.constant(to)) else {
					return Err(Gap::new(format!("the range {from} ... {to} is not a pair of numbers")));
				};
				if to < from || to - from > 255 {
					return Err(Gap::new(format!(
						"the range {from} ... {to} covers {} values, too many to write out as cases",
						(to - from).max(0) + 1
					)));
				}
				Ok((from..=to).collect())
			}
		}
	}

	/* -------------------------------------------------------------- */
	/* One field's type                                                */
	/* -------------------------------------------------------------- */

	/// A top-level placement's field, without the `@` wrapper: the address is
	/// worked out by the caller, where `$` means the end of the placement
	/// before this one.
	fn one_field(&mut self, field: &'a AField) -> Result<Field, Gap> {
		let endian = self.endian;
		let mut bare = field.clone();
		bare.placement = None;
		let ty = self.field_type(&bare, "", endian, &HashMap::new())?;
		let display = field.name().to_string();
		let mut out = named_field(&display, ty, false);
		out.doc = self.doc_of(field);
		Ok(out)
	}

	fn field_type(
		&mut self,
		field: &AField,
		ns: &str,
		endian: Endian,
		types: &HashMap<String, TypeRef>,
	) -> R<Ty> {
		if field.section.is_some() {
			return Err(Gap::new("a field placed in a section the pattern created"));
		}
		if let Some(pointer) = &field.pointer {
			return self.pointer(field, pointer, ns, endian, types);
		}
		let mut ty = self.element(field, ns, endian, types)?;
		ty = self.apply_attributes(&field.attrs, ty, field.pos, field.name())?;
		if let Some(placement) = &field.placement {
			let address = self.expr(placement)?;
			ty = Ty::at(address, ty);
		} else if takes_no_address(field) {
			// `[[no_unique_address]]`: the field is read where it stands and
			// the field after it starts in the same place. An `At` at the
			// current position says exactly that, since a field whose contents
			// are elsewhere takes no room of its own, and here "elsewhere" is
			// here.
			ty = Ty::at_in_window(Expr::Pos, ty);
		}
		Ok(ty)
	}

	/// `T *p : u32`, whose value is an offset read as the given type.
	fn pointer(
		&mut self,
		field: &AField,
		offset_ty: &TypeRef,
		ns: &str,
		endian: Endian,
		types: &HashMap<String, TypeRef>,
	) -> R<Ty> {
		if field.attrs.iter().any(|a| a.path == "pointer_base") {
			return Err(Gap::new("a pointer whose base a function works out"));
		}
		let offset = self.type_ref_in(offset_ty, ns, endian, types)?;
		let mut bare = field.clone();
		bare.pointer = None;
		bare.array = None;
		let target = self.type_ref_in(&bare.ty, ns, endian, types)?;
		Ok(Ty::inline_structure(
			field.name(),
			vec![("offset", offset), ("target", Ty::at(Expr::field("offset"), target))],
		))
	}

	/// The field's type with its array wrapper, if it has one.
	fn element(
		&mut self,
		field: &AField,
		ns: &str,
		endian: Endian,
		types: &HashMap<String, TypeRef>,
	) -> R<Ty> {
		let builtin = match &field.ty.kind {
			TypeKind::Builtin(value) => Some(*value),
			TypeKind::Named { .. } => None,
		};
		let endian = field.ty.endian.map(from_ast_endian).unwrap_or(endian);
		// Text is an array of characters in the language and one field in the
		// IR, so it is settled before anything else looks at the array.
		if let Some(value) = builtin {
			if matches!(value, ValueType::Char | ValueType::Char16) {
				return self.text(field, value, endian);
			}
			if value == ValueType::Str {
				return Err(Gap::new("a `str` field, which only a function fills in"));
			}
		}
		let elem = self.type_ref_in(&field.ty, ns, endian, types)?;
		match &field.array {
			None => Ok(elem),
			Some(ArraySize::Count(count)) => Ok(Ty::array(elem, self.expr(count)?)),
			Some(ArraySize::Unbounded) => Ok(Ty::repeat(elem, Until::End)),
			Some(ArraySize::While(cond)) => Ok(Ty::repeat(elem, self.until(cond)?)),
		}
	}

	/// `T x[while(c)]`. The condition is checked before each element, over the
	/// position that element would start at.
	fn until(&mut self, cond: &super::expr::Expr) -> R<Until> {
		// `while(!std::mem::eof())` is "until the end", written the long way.
		if let ExprKind::Unary { op: UnOp::Not, value } = &cond.kind {
			if let ExprKind::Call { path, args } = &value.kind {
				if path == "std::mem::eof" && args.is_empty() {
					return Ok(Until::End);
				}
			}
		}
		Ok(Until::While(self.expr(cond)?))
	}

	/// `char x[N]`, `char x[]`, `char16 x[N]`.
	fn text(&mut self, field: &AField, value: ValueType, endian: Endian) -> R<Ty> {
		let utf16 = value == ValueType::Char16;
		let enc = if utf16 { Encoding::Utf16(endian) } else { Encoding::Ascii };
		match &field.array {
			// A lone `char` is one byte of text.
			None => Ok(Ty::text(StrLen::Fixed(Expr::lit(if utf16 { 2 } else { 1 })), enc)),
			// The field owns all N characters and its value is all of them,
			// embedded NULs included: `PatternString::getValue` reads exactly
			// `size` bytes and returns a string of that length, and the only
			// trimming is in `formatDisplayValue`, which drops *trailing* NULs
			// before printing.
			//
			// So `StrLen::Fixed`, not `StrLen::Padded`, which the plan called
			// for. The IR's `Padded` ends the value at the *first* pad byte,
			// which is the reading the plan itself ruled out: `char s[6]`
			// holding `ab\0cd` would read as `ab`, and `s == "ab"` would come
			// out true where the reference says false.
			Some(ArraySize::Count(count)) => {
				let count = self.expr(count)?;
				let size = if utf16 { count.mul(Expr::lit(2)) } else { count };
				Ok(Ty::text(StrLen::Fixed(size), enc))
			}
			Some(ArraySize::Unbounded) => Ok(Ty::text(StrLen::Terminated { end: 0, or_end: false }, enc)),
			// The IR's text has no condition for its length, and a run of
			// one-character fields reads exactly the same bytes and stops at
			// exactly the same place. So this is a note, not a gap: what a
			// reader would not guess is that the characters are a list.
			Some(ArraySize::While(cond)) => {
				let until = self.until(cond)?;
				let one = if utf16 { 2 } else { 1 };
				self.report.note(
					self.at(field.pos),
					field_source(field),
					"a run of one-character fields: the IR's text has no condition for its length",
				);
				Ok(Ty::repeat(Ty::text(StrLen::Fixed(Expr::lit(one)), enc), until))
			}
		}
	}

	/// One written type: a built-in, or a declared name, or a name the standard
	/// library gives a meaning the converter knows.
	fn type_ref_in(
		&mut self,
		ty: &TypeRef,
		ns: &str,
		endian: Endian,
		types: &HashMap<String, TypeRef>,
	) -> R<Ty> {
		let endian = ty.endian.map(from_ast_endian).unwrap_or(endian);
		match &ty.kind {
			TypeKind::Builtin(value) => builtin_type(*value, endian),
			TypeKind::Named { path, args } => {
				// A type parameter stands for whatever the use site passed.
				if args.is_empty() {
					if let Some(bound) = types.get(path) {
						let bound = bound.clone();
						return self.type_ref_in(&bound, ns, endian, &HashMap::new());
					}
				}
				if let Some(known) = types::known(path, endian) {
					return self.known_type(path, known, args, ns, endian, types, ty.pos);
				}
				let Some((name, decl)) = self.decls.find(ns, path) else {
					return Err(Gap::new(format!("{path} is not declared")));
				};
				let name = name.to_string();
				let decl_ns = decl.namespace().to_string();
				let args = self.substitute(args, types);
				self.instantiate(&ir_name_of(&name), &name, &decl_ns, &args, endian)
			}
		}
	}

	/// Template arguments with the enclosing instantiation's type parameters
	/// put in, so `Wrapper<T>` inside `Outer<T>` passes on what `Outer` was
	/// given rather than the word `T`.
	fn substitute(&self, args: &[TemplateArg], types: &HashMap<String, TypeRef>) -> Vec<TemplateArg> {
		args.iter()
			.map(|arg| match arg {
				TemplateArg::Type(ty) => match &ty.kind {
					TypeKind::Named { path, args } if args.is_empty() => match types.get(path) {
						Some(bound) => TemplateArg::Type(bound.clone()),
						None => arg.clone(),
					},
					_ => arg.clone(),
				},
				TemplateArg::Value(_) => arg.clone(),
			})
			.collect()
	}

	#[allow(clippy::too_many_arguments)]
	fn known_type(
		&mut self,
		path: &str,
		known: Known,
		args: &[TemplateArg],
		ns: &str,
		endian: Endian,
		types: &HashMap<String, TypeRef>,
		pos: Pos,
	) -> R<Ty> {
		match known {
			Known::Fixed(ty) => Ok(ty),
			Known::Noted(ty, message) => {
				self.report.note(self.at(pos), path.to_string(), message);
				Ok(ty)
			}
			Known::Moment(ty, time) => {
				// A `Time` hangs off the field, not the type, so it is written
				// on a one-field structure the field reads through.
				Ok(moment(path, ty, time))
			}
			Known::DosDate | Known::DosTime => {
				self.report.note(
					self.at(pos),
					path.to_string(),
					"a packed MS-DOS half: the date and the time are paired when both are declared in one structure",
				);
				Ok(Ty::UInt { bits: 16, endian })
			}
			Known::Magic => {
				let Some(TemplateArg::Value(value)) = args.first() else {
					return Err(Gap::new("type::Magic without a value to expect"));
				};
				let ExprKind::Lit(Lit::Str(text)) = &value.kind else {
					return Err(Gap::new(format!("type::Magic<{value}>, whose expected value is not a literal")));
				};
				Ok(Ty::Magic(text.as_bytes().to_vec()))
			}
			Known::Passthrough(message) => {
				let Some(TemplateArg::Type(inner)) = args.first() else {
					return Err(Gap::new(format!("{path} without a type to wrap")));
				};
				let inner = inner.clone();
				let ty = self.type_ref_in(&inner, ns, endian, types)?;
				self.report.note(self.at(pos), path.to_string(), message);
				Ok(ty)
			}
			Known::MemBytes => {
				let Some(TemplateArg::Value(count)) = args.first() else {
					return Err(Gap::new("std::mem::Bytes without a length"));
				};
				let count = count.clone();
				Ok(Ty::bytes(self.expr(&count)?))
			}
			Known::SizedString(enc) => {
				let Some(TemplateArg::Type(size)) = args.first() else {
					return Err(Gap::new(format!("{path} without a type for its length")));
				};
				let size = size.clone();
				let len = self.type_ref_in(&size, ns, endian, types)?;
				Ok(types::sized_string(len, enc))
			}
			Known::NullString(enc) => Ok(types::null_string(enc)),
			Known::Gap(reason) => Err(Gap::new(reason)),
		}
	}

	/* -------------------------------------------------------------- */
	/* Enums and bitfields                                             */
	/* -------------------------------------------------------------- */

	fn enumeration(&mut self, def: &'a super::ast::EnumDef, ns: &str, endian: Endian, ir_name: &str) -> R<Ty> {
		let inner = self.type_ref_in(&def.ty, ns, endian, &HashMap::new())?;
		let mut cases: Vec<(i128, String)> = Vec::new();
		let mut here: HashMap<String, i128> = HashMap::new();
		let mut next: i128 = 0;
		for entry in &def.entries {
			let value = match &entry.value {
				None => next,
				Some(e) => match self.constant_in(e, &here) {
					Some(value) => value,
					None => {
						self.report.gap(
							self.at(entry.pos),
							format!("{}::{}", def.name, entry.name),
							format!("the value {e} is not a number the converter can read"),
						);
						next
					}
				},
			};
			here.insert(entry.name.clone(), value);
			match &entry.upto {
				None => {
					cases.push((value, entry.name.clone()));
					next = value + 1;
				}
				Some(upto) => {
					let Some(top) = self.constant_in(upto, &here) else {
						self.report.gap(
							self.at(entry.pos),
							format!("{}::{} = ... {upto}", def.name, entry.name),
							"the end of the range is not a number the converter can read",
						);
						continue;
					};
					self.range_cases(def, entry, value, top, &mut cases);
					next = top + 1;
				}
			}
		}
		let hex = def.entries.len() > 8;
		let enum_def = EnumDef { name: ir_name.to_string(), cases, docs: Vec::new(), spans: Vec::new(), hex };
		Ok(Ty::Enum { inner: Box::new(inner), def: Arc::new(enum_def) })
	}

	/// `A = 0x10 ... 0x1F`, written out as one case per value.
	///
	/// An `EnumSpan` was the plan, and it is the wrong shape: a span has a
	/// start and a step and no end, so it would name every value above the
	/// range as well. Writing the values out says exactly what the pattern
	/// said, and a range too long to write out is a gap rather than a guess.
	fn range_cases(
		&mut self,
		def: &super::ast::EnumDef,
		entry: &EnumEntry,
		from: i128,
		to: i128,
		cases: &mut Vec<(i128, String)>,
	) {
		let source = format!("{}::{} = {from} ... {to}", def.name, entry.name);
		if to < from {
			self.report.gap(self.at(entry.pos), source, "the range ends before it starts");
			return;
		}
		if to - from > 4095 {
			self.report.gap(
				self.at(entry.pos),
				source,
				format!("a range of {} values, too many to name one at a time", to - from + 1),
			);
			return;
		}
		self.report.note(
			self.at(entry.pos),
			source,
			format!("{} values, each named {}", to - from + 1, entry.name),
		);
		for value in from..=to {
			cases.push((value, entry.name.clone()));
		}
	}

	fn bitfield(
		&mut self,
		def: &'a super::ast::BitfieldDef,
		ns: &str,
		endian: Endian,
		ir_name: &str,
		values: HashMap<String, Expr>,
		types: HashMap<String, TypeRef>,
	) -> R<Ty> {
		let endian = self.bit_order(&def.attrs, endian, def.pos)?;
		let mut frame = Frame::new(false);
		frame.params = values;
		frame.bound = types.clone();
		self.stack.push(frame);
		let mut fields: Vec<Field> = Vec::new();
		let mut machinery: Vec<Arc<str>> = Vec::new();
		let mut offset: u32 = 0;
		self.bit_entries(&def.entries, ns, endian, &types, &mut fields, &mut machinery, &mut offset);
		self.stack.pop();

		let mut out = empty_struct(ir_name);
		out.doc = def.doc.as_deref().map(Arc::from);
		out.fields = fields;
		out.machinery = machinery;
		out.inline = true;
		let bits = offset.div_ceil(8) * 8;
		let ty = Ty::SizedBits { bits: Expr::lit(i128::from(bits)), inner: Box::new(Ty::Struct(Arc::new(out))) };
		self.apply_attributes(&def.attrs, ty, def.pos, &def.name)
	}

	/// Which end of each byte a bitfield's first entry sits at.
	///
	/// Confirmed against the reference: bits pack from the low bit up under
	/// `little`, from the high bit down under `big`, and the default is
	/// `little`. `[[bitfield_order(direction, size)]]` reverses the layout when
	/// its direction disagrees with that.
	fn bit_order(&mut self, attrs: &[Attribute], endian: Endian, pos: Pos) -> Result<Endian, Gap> {
		for attr in attrs {
			match attr.path.as_str() {
				"left_to_right" | "right_to_left" => {
					return Err(Gap::new(format!(
						"[[{}]] is no longer supported by the reference, which rejects the pattern",
						attr.path
					)))
				}
				"bitfield_order" => {
					if attr.args.len() != 2 {
						return Err(Gap::new("[[bitfield_order]] takes a direction and a size in bits"));
					}
					let Some(direction) = self.constant(&attr.args[0]) else {
						return Err(Gap::new(format!(
							"[[bitfield_order({})]]'s direction is not a number the converter can read",
							attr.args[0]
						)));
					};
					// 0 is most-to-least significant, 1 least-to-most.
					let wanted = if direction == 0 { Endian::Big } else { Endian::Little };
					if wanted != endian {
						self.report.note(
							self.at(pos),
							format!("[[bitfield_order({direction}, ..)]]"),
							"the layout is reversed against the bitfield's endian",
						);
					}
					return Ok(wanted);
				}
				_ => {}
			}
		}
		Ok(endian)
	}

	#[allow(clippy::too_many_arguments)]
	fn bit_entries(
		&mut self,
		entries: &'a [BitEntry],
		ns: &str,
		endian: Endian,
		types: &HashMap<String, TypeRef>,
		fields: &mut Vec<Field>,
		machinery: &mut Vec<Arc<str>>,
		offset: &mut u32,
	) {
		for entry in entries {
			match entry {
				BitEntry::Bits { name, sign, width, pos, attrs, doc } => {
					let Some(bits) = self.constant(width).and_then(|n| u32::try_from(n).ok()) else {
						self.report.gap(self.at(*pos), format!("{name} : {width}"), "a width the converter cannot read as a number");
						continue;
					};
					if bits == 0 || bits > 128 {
						self.report.gap(self.at(*pos), format!("{name} : {bits}"), "a width of no bits, or more than a number holds");
						continue;
					}
					let source = format!("{name} : {bits}");
					if let Err(gap) = self.bits_fit(bits, *offset, endian) {
						self.report.gap(self.at(*pos), source, gap.reason);
						*offset += bits;
						continue;
					}
					let ty = match sign {
						BitSign::Signed => Ty::Int { bits, endian },
						_ => Ty::UInt { bits, endian },
					};
					let ty = match self.apply_attributes(attrs, ty, *pos, name) {
						Ok(ty) => ty,
						Err(gap) => {
							self.report.gap(self.at(*pos), source, gap.reason);
							*offset += bits;
							continue;
						}
					};
					let field_name = match sign {
						BitSign::Padding => unique("padding", fields),
						_ => name.clone(),
					};
					self.report.became(self.at(*pos), source, format!("{bits} bits"));
					if matches!(sign, BitSign::Padding) || attrs.iter().any(|a| a.path == "hidden") {
						machinery.push(Arc::from(field_name.as_str()));
					}
					let mut out = named_field(&field_name, ty, false);
					out.doc = doc.as_deref().map(Arc::from);
					self.stack.last_mut().expect("a frame").names.push(name.clone());
					fields.push(out);
					*offset += bits;
				}
				BitEntry::Typed(field) => {
					let width = field.width.as_ref().and_then(|w| self.constant(w)).and_then(|n| u32::try_from(n).ok());
					let source = field_source(field);
					let pos = field.pos;
					let lowered = self.bit_typed(field, ns, endian, types, width, *offset);
					match lowered {
						Ok((ty, bits)) => {
							let name = field.name().to_string();
							self.report.became(self.at(pos), source, format!("{bits} bits"));
							if field.attrs.iter().any(|a| a.path == "hidden") {
								machinery.push(Arc::from(name.as_str()));
							}
							let mut out = named_field(&name, ty, false);
							out.doc = self.doc_of(field);
							self.stack.last_mut().expect("a frame").names.push(field.name().to_string());
							fields.push(out);
							*offset += bits;
						}
						Err(gap) => {
							self.report.gap(self.at(pos), source, gap.reason);
							if let Some(bits) = width {
								*offset += bits;
							}
						}
					}
				}
				BitEntry::If { cond, pos, .. } => self.report.gap(
					self.at(*pos),
					format!("if ({cond}) inside a bitfield"),
					"a condition inside a bitfield, where each branch moves the bits after it by a different amount",
				),
				BitEntry::Match { pos, .. } => self.report.gap(
					self.at(*pos),
					"match inside a bitfield",
					"a match inside a bitfield, where each arm moves the bits after it by a different amount",
				),
				BitEntry::Try { pos, .. } => {
					self.report.gap(self.at(*pos), "try inside a bitfield", "a try/catch, which the converter does not run")
				}
				BitEntry::Call { path, args, pos } => self.top_call(path, args, *pos),
				BitEntry::Statement(statement) => self.report.gap(
					self.at(statement.pos),
					first_line(&statement.text),
					format!("{}, which the converter does not run", kind_name(statement.kind)),
				),
			}
		}
	}

	/// A bitfield entry written with a type: `bool f : 1`, `E e : 2`, a nested
	/// bitfield, or an array of one.
	fn bit_typed(
		&mut self,
		field: &AField,
		ns: &str,
		endian: Endian,
		types: &HashMap<String, TypeRef>,
		width: Option<u32>,
		offset: u32,
	) -> R<(Ty, u32)> {
		if let Some(bits) = width {
			if bits == 0 || bits > 128 {
				return Err(Gap::new("a width of no bits, or more than a number holds"));
			}
			self.bits_fit(bits, offset, endian)?;
			let inner = Ty::UInt { bits, endian };
			let ty = match &field.ty.kind {
				TypeKind::Builtin(ValueType::Bool) => boolean(inner),
				TypeKind::Builtin(ValueType::Char) => return Err(Gap::new("a character narrower than a byte")),
				TypeKind::Builtin(_) => inner,
				TypeKind::Named { path, .. } => {
					let named = self.type_ref_in(&field.ty, ns, endian, types)?;
					match self.narrowed_enum(&named, bits, endian) {
						Some(ty) => ty,
						None => return Err(Gap::new(format!("{path} as a {bits}-bit field"))),
					}
				}
			};
			return Ok((ty, bits));
		}
		// A nested bitfield or an array of one, which brings its own width.
		let ty = self.type_ref_in(&field.ty, ns, endian, types)?;
		let Some(bits) = self.static_bits(&ty) else {
			return Err(Gap::new("a bitfield member whose width is not settled before the file is read"));
		};
		match &field.array {
			None => Ok((ty, bits)),
			Some(ArraySize::Count(count)) => {
				let Some(n) = self.constant(count).and_then(|n| u32::try_from(n).ok()) else {
					return Err(Gap::new("an array inside a bitfield whose length is not a fixed number"));
				};
				Ok((Ty::array(ty, Expr::lit(i128::from(n))), bits * n))
			}
			Some(_) => Err(Gap::new("an array inside a bitfield with no fixed length")),
		}
	}

	/// Whether a field of `bits` bits at `offset` bits into a bitfield is a
	/// single stretch the IR can place.
	///
	/// Under `big` every width works: the IR's own packing is most significant
	/// bit first, which is what `big` means. Under `little` a field is packed
	/// from the bottom of its byte, and a field with a byte boundary inside it
	/// is the whole of one byte and part of the next, which is not one range of
	/// bits in an address space numbered from the top of each byte.
	fn bits_fit(&self, bits: u32, offset: u32, endian: Endian) -> Result<(), Gap> {
		if endian == Endian::Big {
			return Ok(());
		}
		if offset % 8 == 0 && bits % 8 == 0 {
			return Ok(());
		}
		if offset % 8 + bits <= 8 {
			return Ok(());
		}
		Err(Gap::new(format!(
			"a {bits}-bit field {} bits into a byte, packed from the low bit up, crosses into the next byte, which the IR cannot read as one number",
			offset % 8
		)))
	}

	/// An enumeration read from a narrower field than it was declared over.
	fn narrowed_enum(&self, ty: &Ty, bits: u32, endian: Endian) -> Option<Ty> {
		match ty {
			Ty::Enum { def, .. } => Some(Ty::Enum { inner: Box::new(Ty::UInt { bits, endian }), def: def.clone() }),
			Ty::Named(name) => {
				let found = self.types.iter().find(|(n, _)| n == &**name)?;
				self.narrowed_enum(&found.1.clone(), bits, endian)
			}
			_ => None,
		}
	}

	/// How wide a type is, when nothing about the file changes it.
	fn static_bits(&self, ty: &Ty) -> Option<u32> {
		match ty {
			Ty::Named(name) => {
				let found = self.types.iter().find(|(n, _)| n == &**name)?;
				self.static_bits(&found.1.clone())
			}
			Ty::SizedBits { bits: Expr::Lit(n), .. } => u32::try_from(*n).ok(),
			other => crate::decode::fixed_bits(other).and_then(|n| u32::try_from(n).ok()),
		}
	}

	/* -------------------------------------------------------------- */
	/* Attributes                                                      */
	/* -------------------------------------------------------------- */

	fn apply_attributes(&mut self, attrs: &[Attribute], ty: Ty, pos: Pos, name: &str) -> R<Ty> {
		let mut ty = ty;
		for attr in attrs {
			let source = format!("[[{}]]", attr.path);
			if !KNOWN_ATTRIBUTES.contains(&attr.path.as_str()) {
				self.report.note(
					self.at(pos),
					source,
					"an attribute the reference does not act on either, so nothing was lost",
				);
				continue;
			}
			match attr.path.as_str() {
				"name" => {
					// The IR has one name per field and every expression,
					// path and edit goes through it, so the declared name is
					// what the field is called and the display name goes in
					// the prose beside it.
					self.report.note(
						self.at(pos),
						source,
						format!("the reference shows {name} under another name; the declared name is what the IR uses"),
					);
				}
				"comment" | "hex::spec_name" => {}
				"inline" => ty = self.inlined(ty),
				"sealed" => self.report.note(
					self.at(pos),
					source,
					"the reference shows the whole structure as one value; the listing joins short structures already",
				),
				"hidden" | "highlight_hidden" => {
					self.report.note(self.at(pos), source, "the field is read and shown")
				}
				"format" | "format_read" | "format_write" | "format_entries" => self.report.note(
					self.at(pos),
					source,
					"the raw value is shown: the function that would have formatted it is not run",
				),
				// Not a note. A `[[format]]` only changes what is printed, and
				// a `[[transform]]` changes what the field *is*: a `cpio`
				// header's `filesize` is a byte-swapped u32, and the field
				// after it is that many bytes long. Showing the raw number
				// would be a note; letting a length read it would be reading
				// the file wrongly and saying nothing.
				"transform" | "transform_entries" => {
					return Err(Gap::new(
						"[[transform]] replaces the value with what a function returns, and everything reading the field reads that",
					))
				}
				"pointer_base" => return Err(Gap::new("[[pointer_base]] works the address out with a function")),
				"no_unique_address" => self.report.note(
					self.at(pos),
					source,
					"read where it stands and taking no room: the field after it starts in the same place",
				),
				"bitfield_order" => {}
				_ => self.report.note(self.at(pos), source, "read and ignored: it says how the reference draws the field"),
			}
		}
		Ok(ty)
	}

	fn inlined(&mut self, ty: Ty) -> Ty {
		match ty {
			Ty::Struct(def) => {
				let mut copy = (*def).clone();
				copy.inline = true;
				Ty::Struct(Arc::new(copy))
			}
			Ty::Named(name) => {
				let inline_name = format!("{name}#inline");
				if !self.emitted.contains(&inline_name) {
					let Some((_, found)) = self.types.iter().find(|(n, _)| n == &*name) else { return Ty::Named(name) };
					let found = found.clone();
					let inlined = self.inlined(found);
					self.emitted.insert(inline_name.clone());
					self.types.push((inline_name.clone(), inlined));
				}
				Ty::Named(Arc::from(inline_name.as_str()))
			}
			other => other,
		}
	}

	/// What `[[name("x")]]` says the field is shown as, which goes in the
	/// field's prose rather than in its name. See `apply_attributes`.
	fn display_named(&self, attrs: &[Attribute]) -> Option<String> {
		let attr = attrs.iter().find(|a| a.path == "name")?;
		match &attr.args.first()?.kind {
			ExprKind::Lit(Lit::Str(text)) => Some(text.clone()),
			_ => None,
		}
	}

	fn doc_of(&self, field: &AField) -> Option<Arc<str>> {
		let mut lines: Vec<String> = Vec::new();
		if let Some(doc) = &field.doc {
			lines.push(doc.clone());
		}
		if let Some(shown) = self.display_named(&field.attrs) {
			lines.push(format!("Shown by the reference as {shown}."));
		}
		for attr in &field.attrs {
			if attr.path == "comment" || attr.path == "hex::spec_name" {
				if let Some(ExprKind::Lit(Lit::Str(text))) = attr.args.first().map(|a| &a.kind) {
					lines.push(text.clone());
				}
			}
		}
		(!lines.is_empty()).then(|| Arc::from(lines.join("\n").as_str()))
	}

	/* -------------------------------------------------------------- */
	/* Expressions                                                     */
	/* -------------------------------------------------------------- */

	/// A constant the converter can work out without reading anything.
	fn constant(&mut self, e: &super::expr::Expr) -> Option<i128> {
		self.constant_in(e, &HashMap::new())
	}

	fn constant_in(&mut self, e: &super::expr::Expr, here: &HashMap<String, i128>) -> Option<i128> {
		match &e.kind {
			ExprKind::Lit(Lit::Int { value, .. }) => i128::try_from(*value).ok(),
			ExprKind::Lit(Lit::Bool(value)) => Some(i128::from(*value)),
			ExprKind::Path(segments) => match segments.as_slice() {
				[PathSeg::Name(name)] => here.get(name).copied().or_else(|| match self.lookup(name) {
					Some(Expr::Lit(value)) => Some(value),
					_ => None,
				}),
				_ => None,
			},
			ExprKind::ScopeRes { ty, name } => self.enum_value(ty, name),
			ExprKind::Unary { op, value } => {
				let value = self.constant_in(value, here)?;
				Some(match op {
					UnOp::Plus => value,
					UnOp::Neg => value.checked_neg()?,
					UnOp::Not => i128::from(value == 0),
					UnOp::BitNot => !value,
				})
			}
			ExprKind::Binary { op, lhs, rhs } => {
				let lhs = self.constant_in(lhs, here)?;
				let rhs = self.constant_in(rhs, here)?;
				constant_binary(*op, lhs, rhs)
			}
			ExprKind::Ternary { cond, then, otherwise } => {
				let cond = self.constant_in(cond, here)?;
				if cond != 0 {
					self.constant_in(then, here)
				} else {
					self.constant_in(otherwise, here)
				}
			}
			ExprKind::Cast { value, .. } => self.constant_in(value, here),
			_ => None,
		}
	}

	/// The number an `E::Label` stands for, from the enumeration as declared.
	fn enum_value(&mut self, ty: &str, name: &str) -> Option<i128> {
		// `using BitfieldOrder = std::core::BitfieldOrder;` is an ordinary way
		// to shorten a name, and the enumeration is at the end of the chain.
		let mut ty = ty.to_string();
		let mut hops = 0;
		let (found, decl) = loop {
			let Some(found) = self.decls.find("", &ty).or_else(|| self.decls.find_by_tail(&ty)) else {
				return None;
			};
			match found.1 {
				Declared::Using(alias, _) => {
					hops += 1;
					if hops > 16 {
						return None;
					}
					let Some(TypeRef { kind: TypeKind::Named { path, .. }, .. }) = &alias.ty else {
						return None;
					};
					ty = path.clone();
				}
				_ => break found,
			}
		};
		let Declared::Enum(def, _) = decl else { return None };
		let def: &'a super::ast::EnumDef = def;
		let found = found.to_string();
		if !self.resolving.insert(found.clone()) {
			return None;
		}
		let value = self.enum_value_in(def, name);
		self.resolving.remove(&found);
		value
	}

	fn enum_value_in(&mut self, def: &'a super::ast::EnumDef, name: &str) -> Option<i128> {
		let mut here: HashMap<String, i128> = HashMap::new();
		let mut next: i128 = 0;
		for entry in &def.entries {
			let value = match &entry.value {
				None => next,
				Some(e) => self.constant_in(e, &here)?,
			};
			here.insert(entry.name.clone(), value);
			if entry.name == name {
				return Some(value);
			}
			next = match &entry.upto {
				Some(upto) => self.constant_in(upto, &here)? + 1,
				None => value + 1,
			};
		}
		None
	}

	/// What a name means here: a bound template parameter, or a field in scope.
	fn lookup(&self, name: &str) -> Option<Expr> {
		for frame in self.stack.iter().rev() {
			if let Some(value) = frame.params.get(name) {
				return Some(value.clone());
			}
			if frame.names.iter().any(|n| n == name) {
				return Some(Expr::field(name));
			}
		}
		None
	}

	/// Note what the first field of this level turned out to be, which is what
	/// `addressof(this)` names once anything has been read.
	fn mark_start(&mut self, fields: &[Field]) {
		let Some(frame) = self.stack.last_mut() else { return };
		if frame.started {
			return;
		}
		let Some(first) = fields.first() else { return };
		frame.started = true;
		frame.start = match first.ty {
			// A field only a condition reads may not be there to be the start
			// of, and a placed field's start is where it points rather than
			// where it stands, so neither says where the structure began.
			Ty::When { .. } | Ty::Switch { .. } | Ty::At { .. } => None,
			_ => Some(Expr::start_of(Expr::field(&first.name))),
		};
	}

	/// Where the structure being read began, which is what `addressof(this)`
	/// asks for.
	///
	/// Only the start of the first field will do. `SpacePos` is where the
	/// structure began when nothing has been read yet, but only if the
	/// expression is worked out once: a `[while(..)]` condition is worked out
	/// again before every element, and `$ == addressof(this)` written as
	/// `SpacePos == SpacePos` is true every time, which reads the file wrongly
	/// and says nothing. So a structure that has read nothing yet is a gap.
	fn here_start(&self) -> R<Expr> {
		for frame in self.stack.iter().rev() {
			if frame.block {
				continue;
			}
			if !frame.started {
				return Err(Gap::new(
					"addressof(this), where the structure has read nothing yet, so the IR has no field whose start says where it began",
				));
			}
			return frame.start.clone().ok_or_else(|| {
				Gap::new(
					"addressof(this), where the first field of the structure is one only a condition reads, so nothing in the IR names where the structure began",
				)
			});
		}
		Err(Gap::new("addressof(this), which names a structure rather than a field"))
	}

	/// Why a name the pattern declares holds no value the converter can read.
	fn unreadable(&self, name: &str) -> Option<String> {
		self.stack.iter().rev().find_map(|frame| frame.unreadable.get(name).cloned())
	}

	/// Remember that `name` is declared here and holds no value the converter
	/// can read, so that a later use says why rather than saying the name is
	/// not there at all.
	fn cannot_read(&mut self, name: &str, why: impl Into<String>) {
		if let Some(frame) = self.stack.last_mut() {
			frame.unreadable.insert(name.to_string(), why.into());
		}
	}

	/// Whether a name is one declared inside an `if` block or a `match` arm,
	/// which the pattern can still see and an IR expression cannot reach.
	fn is_hidden(&self, name: &str) -> bool {
		self.stack.iter().any(|frame| frame.hidden.iter().any(|n| n == name))
	}

	fn expr(&mut self, e: &super::expr::Expr) -> R<Expr> {
		match &e.kind {
			ExprKind::Lit(Lit::Int { value, .. }) => match i128::try_from(*value) {
				Ok(value) => Ok(Expr::lit(value)),
				Err(_) => Err(Gap::new(format!("the literal {value} does not fit in the IR's arithmetic"))),
			},
			ExprKind::Lit(Lit::Bool(value)) => Ok(Expr::lit(i128::from(*value))),
			ExprKind::Lit(Lit::Float(value)) => {
				Err(Gap::new(format!("the number {value} is a float, and the IR's expressions count in whole numbers")))
			}
			ExprKind::Lit(Lit::Str(text)) => Err(Gap::new(format!(
				"the text {text:?} used as a number; only a comparison against a fixed-size field reads it as one"
			))),
			ExprKind::Path(segments) => self.path(segments),
			ExprKind::ScopeRes { ty, name } => match self.enum_value(ty, name) {
				Some(value) => Ok(Expr::lit(value)),
				None => Err(Gap::new(format!("{ty}::{name} is not a value the converter could read"))),
			},
			ExprKind::Call { path, args } => self.call(path, args),
			ExprKind::Unary { op, value } => {
				let value = self.expr(value)?;
				Ok(match op {
					UnOp::Plus => value,
					UnOp::Neg => Expr::lit(0).sub(value),
					UnOp::Not => value.negate(),
					UnOp::BitNot => value.bit_not(),
				})
			}
			ExprKind::Binary { op, lhs, rhs } => self.binary(*op, lhs, rhs),
			ExprKind::Ternary { cond, then, otherwise } => {
				Ok(Expr::cond(self.expr(cond)?, self.expr(then)?, self.expr(otherwise)?))
			}
			ExprKind::TypeOp { op, arg } => self.type_op(*op, arg),
			ExprKind::Cast { ty, value } => {
				// A cast narrows, and the IR has no narrowing. Widening says
				// nothing, so a cast to something at least as wide as the IR
				// counts in is the value itself.
				let value = self.expr(value)?;
				match &ty.kind {
					// Widening says nothing, and narrowing to an unsigned type
					// keeps the low bits, which is exactly a mask.
					TypeKind::Builtin(v) if v.bits() == Some(128) => Ok(value),
					TypeKind::Builtin(v) if unsigned_bits(*v).is_some() => {
						let bits = unsigned_bits(*v).expect("checked");
						Ok(value.and(Expr::lit((1i128 << bits) - 1)))
					}
					_ => Err(Gap::new(format!(
						"a cast to {}, which changes the number and the IR has no way to say so",
						expr::type_name(ty)
					))),
				}
			}
			ExprKind::Reinterpret { ty, .. } => Err(Gap::new(format!(
				"reading a value again as {}, which the IR does not do inside an expression",
				expr::type_name(ty)
			))),
		}
	}

	fn binary(&mut self, op: BinOp, lhs: &super::expr::Expr, rhs: &super::expr::Expr) -> R<Expr> {
		// `"IHDR" == f` is the field's bytes read as a big-endian number, which
		// is the same comparison said a way a reader would not guess.
		if matches!(op, BinOp::Eq | BinOp::Ne) {
			if let Some(pair) = self.text_comparison(lhs, rhs)? {
				return Ok(match op {
					BinOp::Eq => pair.0.equal_to(pair.1),
					_ => pair.0.not_equal(pair.1),
				});
			}
		}
		let left = self.expr(lhs)?;
		let right = self.expr(rhs)?;
		Ok(match op {
			BinOp::Add => left.add(right),
			BinOp::Sub => left.sub(right),
			BinOp::Mul => left.mul(right),
			BinOp::Div => left.div(right),
			BinOp::Rem => left.modulo(right),
			BinOp::Shl => left.shl(right),
			BinOp::Shr => left.shr(right),
			BinOp::BitAnd => left.and(right),
			BinOp::BitOr => left.bit_or(right),
			BinOp::BitXor => left.bit_xor(right),
			BinOp::Eq => left.equal_to(right),
			BinOp::Ne => left.not_equal(right),
			BinOp::Lt => left.less_than(right),
			BinOp::Le => left.less_or_equal(right),
			BinOp::Gt => left.greater_than(right),
			BinOp::Ge => left.greater_or_equal(right),
			BinOp::BoolAnd => left.both(right),
			BinOp::BoolOr => left.either(right),
			BinOp::BoolXor => return Err(Gap::new("`^^`, which the IR has no operator for")),
		})
	}

	/// A comparison with a string literal on one side, as the bytes of the
	/// other side read big-endian.
	fn text_comparison(
		&mut self,
		lhs: &super::expr::Expr,
		rhs: &super::expr::Expr,
	) -> R<Option<(Expr, Expr)>> {
		let (text, other) = match (&lhs.kind, &rhs.kind) {
			(ExprKind::Lit(Lit::Str(text)), _) => (text, rhs),
			(_, ExprKind::Lit(Lit::Str(text))) => (text, lhs),
			_ => return Ok(None),
		};
		let bytes = text.as_bytes();
		if bytes.is_empty() || bytes.len() > 16 {
			return Err(Gap::new(format!(
				"the text {text:?} is {} bytes, which is not a number the IR compares with",
				bytes.len()
			)));
		}
		let value = bytes.iter().fold(0i128, |acc, &b| (acc << 8) | i128::from(b));
		let other = self.expr(other)?;
		Ok(Some((other, Expr::lit(value))))
	}

	fn path(&mut self, segments: &[PathSeg]) -> R<Expr> {
		// `$` on its own: where the field would start, measured from the front
		// of the whole space, which is what a hexpat address is.
		if segments.len() == 1 && matches!(segments[0], PathSeg::Dollar) {
			return Ok(Expr::SpacePos);
		}
		let mut climbs = 0usize;
		let mut rest = segments;
		loop {
			match rest.first() {
				Some(PathSeg::Parent) => {
					climbs += 1;
					rest = &rest[1..];
				}
				Some(PathSeg::This) => rest = &rest[1..],
				_ => break,
			}
		}
		if rest.is_empty() {
			return Err(Gap::new("a path naming a structure rather than a value"));
		}
		// `$[e]` is the one byte the file holds at the address `e`, which is a
		// read rather than a position: the reference indexes the file with it.
		if matches!(rest[0], PathSeg::Dollar) {
			if climbs == 0 && rest.len() == 2 {
				if let PathSeg::Index(address) = &rest[1] {
					return self.byte_at(address);
				}
			}
			return Err(Gap::new("`$` reached through a path, which is not a position the IR names"));
		}
		if matches!(rest[0], PathSeg::Null) {
			return Err(Gap::new("`null`, which is not a number"));
		}
		let PathSeg::Name(first) = &rest[0] else {
			return Err(Gap::new("a path starting with an index"));
		};
		if climbs > 0 {
			self.check_climb(first, climbs)?;
		} else if self.lookup(first).is_none() {
			if self.is_hidden(first) {
				return Err(Gap::new(format!(
					"{first} is declared inside an `if` block, and a field after the block cannot name it in the IR"
				)));
			}
			if let Some(reason) = self.unreadable(first) {
				return Err(Gap::new(reason));
			}
			return Err(Gap::new(format!("{first} is not a field in scope here")));
		}
		// A bound template value parameter stands for the expression it was
		// given, and nothing may be read out of it.
		if let Some(value) = self.parameter(first) {
			if rest.len() == 1 {
				return Ok(value);
			}
			return Err(Gap::new(format!("{first} is a template argument, and nothing is read out of one")));
		}
		self.steps(first, &rest[1..])
	}

	/// The type parameters bound around here, innermost last.
	fn bound_types(&self) -> HashMap<String, TypeRef> {
		let mut out = HashMap::new();
		for frame in &self.stack {
			for (name, ty) in &frame.bound {
				out.insert(name.clone(), ty.clone());
			}
		}
		out
	}

	fn parameter(&self, name: &str) -> Option<Expr> {
		self.stack.iter().rev().find_map(|frame| frame.params.get(name).cloned())
	}

	/// Whether `parent.` ... `name` lands where the IR's own outward search
	/// would land: the name has to be missing from every level in between.
	fn check_climb(&self, name: &str, climbs: usize) -> Result<(), Gap> {
		// An `if` block is a level in the IR and not one in the pattern.
		let levels: Vec<&Frame> = self.stack.iter().filter(|frame| !frame.block).collect();
		if climbs >= levels.len() {
			return Err(Gap::new(format!("`parent` climbs past the outermost structure looking for {name}")));
		}
		for frame in levels.iter().rev().take(climbs) {
			if frame.names.iter().any(|n| n == name) || frame.params.contains_key(name) {
				return Err(Gap::new(format!(
					"`parent.{name}` skips a nearer {name}, and the IR's names are searched outwards from the nearest"
				)));
			}
		}
		let target = levels[levels.len() - 1 - climbs];
		if !target.names.iter().any(|n| n == name) && !target.params.contains_key(name) {
			return Err(Gap::new(format!("{name} is not a field of the structure `parent` climbs to")));
		}
		Ok(())
	}

	/// The rest of a path after its first name: `.b`, `[i]`, `[i].b`.
	fn steps(&mut self, first: &str, rest: &[PathSeg]) -> R<Expr> {
		if rest.is_empty() {
			return Ok(Expr::field(first));
		}
		// `arr[i]` and `arr[i].field`.
		if let PathSeg::Index(index) = &rest[0] {
			let index = self.expr(index)?;
			let names = plain_names(&rest[1..])?;
			return Ok(if names.is_empty() {
				Expr::elem(first, index)
			} else {
				Expr::elem_field(first, index, &names.iter().map(String::as_str).collect::<Vec<_>>())
			});
		}
		let mut names = vec![first.to_string()];
		names.extend(plain_names(rest)?);
		Ok(Expr::within(&names.iter().map(String::as_str).collect::<Vec<_>>()))
	}

	fn call(&mut self, path: &str, args: &[super::expr::Expr]) -> R<Expr> {
		match path {
			"std::mem::eof" => Ok(Expr::Remaining.equal_to(Expr::lit(0))),
			"std::mem::size" => Ok(Expr::SpaceSize),
			"std::mem::base_address" => Ok(Expr::lit(0)),
			"std::core::array_index" => Ok(Expr::Idx),
			"std::core::member_count" => {
				let Some(arg) = args.first() else { return Err(Gap::new("member_count with no argument")) };
				match &arg.kind {
					ExprKind::Path(segments) => match plain_names(segments) {
						Ok(names) if names.len() == 1 => Ok(Expr::len_of(&names[0])),
						_ => Err(Gap::new("member_count of something other than a field in scope")),
					},
					_ => Err(Gap::new("member_count of something other than a field in scope")),
				}
			}
			"std::mem::read_unsigned" | "std::mem::read_signed" => self.peek(path, args),
			"std::math::abs" | "std::math::min" | "std::math::max" | "std::math::pow"
			| "std::math::ceil" | "std::math::floor" | "std::math::round" | "std::math::clamp" => {
				Err(Gap::new(format!("{path}, which the IR has no expression for")))
			}
			_ => Err(Gap::new(format!("{path} is run by the reference, and the converter runs nothing"))),
		}
	}

	/// `$[e]`, the one byte the file holds at the address `e`.
	fn byte_at(&mut self, address: &super::expr::Expr) -> R<Expr> {
		let endian = self.endian;
		if let Some(skip) = self.relative_to_dollar(address)? {
			return Ok(Expr::peek_at(Expr::lit(i128::from(skip) * 8), 8, endian));
		}
		let at = self.expr(address)?;
		Ok(Expr::PeekIn { at: Box::new(at.mul(Expr::lit(8))), bits: 8, endian })
	}

	/// `std::mem::read_unsigned(address, size)`. A bare `$` or `$ + k` is a
	/// peek a fixed distance from here; everything else is an address.
	fn peek(&mut self, path: &str, args: &[super::expr::Expr]) -> R<Expr> {
		if args.len() < 2 {
			return Err(Gap::new(format!("{path} wants an address and a size")));
		}
		if path.ends_with("read_signed") {
			return Err(Gap::new("std::mem::read_signed, whose value the IR's peek reads unsigned"));
		}
		let Some(size) = self.constant(&args[1]) else {
			return Err(Gap::new("a read whose size is not settled before the file is read"));
		};
		let Ok(size) = u32::try_from(size) else { return Err(Gap::new("a read of a negative number of bytes")) };
		if size == 0 || size > 16 {
			return Err(Gap::new(format!("a read of {size} bytes, which is not a number the IR holds")));
		}
		let endian = match args.get(2) {
			None => self.endian,
			Some(e) => match &e.kind {
				ExprKind::ScopeRes { ty, name } if ty.ends_with("Endian") => match name.as_str() {
					"Big" => Endian::Big,
					"Little" => Endian::Little,
					_ => self.endian,
				},
				_ => return Err(Gap::new("a read whose byte order the pattern works out while it runs")),
			},
		};
		let bits = size * 8;
		if let Some(skip) = self.relative_to_dollar(&args[0])? {
			return Ok(Expr::peek_at(Expr::lit(i128::from(skip) * 8), bits, endian));
		}
		let at = self.expr(&args[0])?;
		Ok(Expr::PeekIn { at: Box::new(at.mul(Expr::lit(8))), bits, endian })
	}

	/// `$` or `$ + k` for a constant `k`, as that constant.
	fn relative_to_dollar(&mut self, e: &super::expr::Expr) -> R<Option<i64>> {
		if let ExprKind::Path(segments) = &e.kind {
			if segments.len() == 1 && matches!(segments[0], PathSeg::Dollar) {
				return Ok(Some(0));
			}
		}
		let ExprKind::Binary { op: BinOp::Add, lhs, rhs } = &e.kind else { return Ok(None) };
		let ExprKind::Path(segments) = &lhs.kind else { return Ok(None) };
		if segments.len() != 1 || !matches!(segments[0], PathSeg::Dollar) {
			return Ok(None);
		}
		match self.constant(rhs) {
			Some(k) if k >= 0 => Ok(i64::try_from(k).ok()),
			_ => Ok(None),
		}
	}

	fn type_op(&mut self, op: TypeOp, arg: &TypeOpArg) -> R<Expr> {
		match (op, arg) {
			(TypeOp::SizeOf, TypeOpArg::Space) => Ok(Expr::SpaceSize),
			(TypeOp::AddressOf, TypeOpArg::Space) => Ok(Expr::lit(0)),
			(TypeOp::SizeOf, TypeOpArg::Value(value)) => match &value.kind {
				ExprKind::Path(segments) => match plain_names(segments) {
					Ok(names) if names.len() == 1 => Ok(Expr::size_of(&names[0])),
					_ => Err(Gap::new("sizeof of something other than a field in scope")),
				},
				_ => Err(Gap::new("sizeof of something other than a field in scope")),
			},
			(TypeOp::AddressOf, TypeOpArg::Value(value)) => match &value.kind {
				// `addressof(this)` is the enclosing structure's own start,
				// which is not a field and so not somewhere the IR can name.
				ExprKind::Path(segments)
					if !segments.is_empty() && segments.iter().all(|s| matches!(s, PathSeg::This)) =>
				{
					self.here_start()
				}
				ExprKind::Path(segments) if segments.iter().all(|s| matches!(s, PathSeg::This | PathSeg::Parent)) => {
					Err(Gap::new("addressof(parent), which names a structure further out rather than a field"))
				}
				ExprKind::Path(_) => Ok(Expr::start_of(self.expr(value)?)),
				_ => Err(Gap::new("addressof of something other than a field in scope")),
			},
			(TypeOp::SizeOf, TypeOpArg::Type(ty)) => {
				let endian = self.endian;
				let bound = self.bound_types();
				let lowered = self.type_ref_in(ty, "", endian, &bound)?;
				match self.static_bits(&lowered) {
					Some(bits) if bits % 8 == 0 => Ok(Expr::lit(i128::from(bits / 8))),
					_ => Err(Gap::new(format!("sizeof({}), whose size is not settled before the file is read", expr::type_name(ty)))),
				}
			}
			(TypeOp::AddressOf, TypeOpArg::Type(_)) => Err(Gap::new("addressof of a type")),
			(TypeOp::TypeNameOf, _) => Err(Gap::new("typenameof, which is text rather than a number")),
		}
	}
}

/* ------------------------------------------------------------------ */
/* Helpers                                                             */
/* ------------------------------------------------------------------ */

fn builtin_type(value: ValueType, endian: Endian) -> R<Ty> {
	Ok(match value {
		ValueType::U8 => Ty::u8(),
		ValueType::U16 | ValueType::U24 | ValueType::U32 | ValueType::U48 | ValueType::U64
		| ValueType::U96 | ValueType::U128 => Ty::UInt { bits: value.bits().expect("a width"), endian },
		ValueType::S8 | ValueType::S16 | ValueType::S24 | ValueType::S32 | ValueType::S48
		| ValueType::S64 | ValueType::S96 | ValueType::S128 => {
			Ty::Int { bits: value.bits().expect("a width"), endian }
		}
		ValueType::Float => Ty::F32(endian),
		ValueType::Double => Ty::F64(endian),
		ValueType::Bool => boolean(Ty::u8()),
		ValueType::Char => Ty::text(StrLen::Fixed(Expr::lit(1)), Encoding::Ascii),
		ValueType::Char16 => Ty::text(StrLen::Fixed(Expr::lit(2)), Encoding::Utf16(endian)),
		ValueType::Str => return Err(Gap::new("a `str` field, which only a function fills in")),
		ValueType::Padding => Ty::bytes(Expr::lit(1)),
		ValueType::Auto | ValueType::Any => {
			return Err(Gap::new("an `auto` field, whose type the pattern settles while it runs"))
		}
	})
}

fn boolean(inner: Ty) -> Ty {
	Ty::Enum {
		inner: Box::new(inner),
		def: Arc::new(EnumDef {
			name: "bool".to_string(),
			cases: vec![(0, "false".to_string()), (1, "true".to_string())],
			docs: Vec::new(),
			spans: Vec::new(),
			hex: false,
		}),
	}
}

/// A number that holds a moment, as a structure of one field so that the
/// `Time` has a field to hang on.
fn moment(name: &str, ty: Ty, time: Time) -> Ty {
	let short = name.rsplit("::").next().unwrap_or(name);
	Ty::inline_structure(short, vec![("value", ty)]).field_time("value", time)
}

fn named_field(name: &str, ty: Ty, aside: bool) -> Field {
	Field {
		name: Arc::from(name),
		ty,
		doc: None,
		name_from: None,
		elem_name_from: None,
		aside,
		checks: Vec::new(),
		valid: None,
		time: None,
		elem_check: None,
	}
}

fn empty_struct(name: &str) -> StructDef {
	StructDef {
		name: name.to_string(),
		fields: Vec::new(),
		doc: None,
		named_by: None,
		contents: None,
		unit: None,
		inline: false,
		overlap: false,
		packed: None,
		cut: false,
		machinery: Vec::new(),
		payload: Vec::new(),
		line: Vec::new(),
		encoding: None,
	}
}

/// A name no field of `fields` already has.
fn unique(base: &str, fields: &[Field]) -> String {
	if !fields.iter().any(|f| &*f.name == base) {
		return base.to_string();
	}
	let mut n = 2;
	loop {
		let candidate = format!("{base}_{n}");
		if !fields.iter().any(|f| *f.name == *candidate) {
			return candidate;
		}
		n += 1;
	}
}

fn from_ast_endian(endian: super::ast::Endian) -> Endian {
	match endian {
		super::ast::Endian::Little => Endian::Little,
		super::ast::Endian::Big => Endian::Big,
	}
}

/// How many bits an unsigned built-in holds, for the cast that is a mask.
fn unsigned_bits(value: ValueType) -> Option<u32> {
	match value {
		ValueType::U8 | ValueType::U16 | ValueType::U24 | ValueType::U32 | ValueType::U48
		| ValueType::U64 | ValueType::U96 => value.bits(),
		_ => None,
	}
}

fn constant_binary(op: BinOp, lhs: i128, rhs: i128) -> Option<i128> {
	Some(match op {
		BinOp::Add => lhs.checked_add(rhs)?,
		BinOp::Sub => lhs.checked_sub(rhs)?,
		BinOp::Mul => lhs.checked_mul(rhs)?,
		BinOp::Div => lhs.checked_div(rhs)?,
		BinOp::Rem => lhs.checked_rem(rhs)?,
		BinOp::Shl => lhs.checked_shl(u32::try_from(rhs).ok()?)?,
		BinOp::Shr => lhs.checked_shr(u32::try_from(rhs).ok()?)?,
		BinOp::BitAnd => lhs & rhs,
		BinOp::BitOr => lhs | rhs,
		BinOp::BitXor => lhs ^ rhs,
		BinOp::Eq => i128::from(lhs == rhs),
		BinOp::Ne => i128::from(lhs != rhs),
		BinOp::Lt => i128::from(lhs < rhs),
		BinOp::Le => i128::from(lhs <= rhs),
		BinOp::Gt => i128::from(lhs > rhs),
		BinOp::Ge => i128::from(lhs >= rhs),
		BinOp::BoolAnd => i128::from(lhs != 0 && rhs != 0),
		BinOp::BoolOr => i128::from(lhs != 0 || rhs != 0),
		BinOp::BoolXor => i128::from((lhs != 0) != (rhs != 0)),
	})
}

/// The plain names of a path with no indexes in it.
fn plain_names(segments: &[PathSeg]) -> R<Vec<String>> {
	let mut out = Vec::new();
	for segment in segments {
		match segment {
			PathSeg::Name(name) => out.push(name.clone()),
			PathSeg::This => {}
			_ => return Err(Gap::new("a path the converter cannot follow")),
		}
	}
	Ok(out)
}

fn arg_key(arg: &TemplateArg) -> String {
	match arg {
		TemplateArg::Type(ty) => expr::type_name(ty),
		TemplateArg::Value(value) => value.to_string(),
	}
}

/// The name a declared type goes by in the IR's table of types.
fn ir_name_of(name: &str) -> String {
	name.to_string()
}

fn program_name(program: &Program) -> String {
	let base = program.file.rsplit('/').next().unwrap_or(&program.file);
	base.strip_suffix(".hexpat").unwrap_or(base).to_string()
}

/// Every `#pragma` of a pattern and of everything it included.
fn every_pragma(program: &Program) -> Vec<super::lexer::Pragma> {
	let mut out = program.pragmas.clone();
	let mut seen = HashSet::new();
	fn walk(decls: &[Decl], out: &mut Vec<super::lexer::Pragma>, seen: &mut HashSet<String>) {
		for decl in decls {
			if let Decl::Include { program: Some(inner), .. } = decl {
				if seen.insert(inner.file.clone()) {
					// An include's pragmas belong to the include: only the ones
					// the file itself wrote say how it is read.
					walk(&inner.decls, out, seen);
				}
			}
		}
		let _ = out;
	}
	walk(&program.decls, &mut out, &mut seen);
	out
}

/// `#pragma magic [ 4D 5A ] @ 0x00`, as its bytes and its address.
///
/// The corpus writes one several ways: with a trailing semicolon, with a `//`
/// comment after it, with the bytes as a quoted string, and with `??` or a
/// single `?` nibble for a byte it does not care about. A wildcard cannot be
/// part of a fixed sequence, so the bytes before the first one are the magic
/// and the rest is dropped; a magic that starts with one claims nothing.
fn parse_magic(value: &str) -> Option<(Vec<u8>, i64)> {
	let value = value.split("//").next().unwrap_or(value).trim().trim_end_matches(';').trim();
	let at = value.rfind('@')?;
	let (list, address) = value.split_at(at);
	let list = list.trim();
	let list = list.strip_prefix('[')?.trim_end().strip_suffix(']')?.trim();
	let mut bytes = Vec::new();
	if let Some(text) = list.strip_prefix('"').and_then(|t| t.strip_suffix('"')) {
		bytes.extend_from_slice(text.as_bytes());
	} else {
		for word in list.split_whitespace() {
			if word.contains('?') {
				break;
			}
			bytes.push(u8::from_str_radix(word, 16).ok()?);
		}
	}
	if bytes.is_empty() {
		return None;
	}
	// The address may be negative, and then it counts back from the end of the
	// file: `vhd.hexpat` writes `@ -0x0200` for a footer 512 bytes from the end.
	let address = address[1..].trim();
	let (negative, digits) = match address.strip_prefix('-') {
		Some(rest) => (true, rest.trim()),
		None => (false, address.strip_prefix('+').unwrap_or(address).trim()),
	};
	let magnitude = match digits.strip_prefix("0x").or_else(|| digits.strip_prefix("0X")) {
		Some(hex) => i64::from_str_radix(hex, 16).ok()?,
		None => digits.parse::<i64>().ok()?,
	};
	Some((bytes, if negative { -magnitude } else { magnitude }))
}

fn field_source(field: &AField) -> String {
	let mut text = expr::type_name(&field.ty);
	if field.pointer.is_some() {
		text.push_str(" *");
	} else {
		text.push(' ');
	}
	text.push_str(field.name());
	match &field.array {
		None => {}
		Some(ArraySize::Unbounded) => text.push_str("[]"),
		Some(ArraySize::Count(count)) => text.push_str(&format!("[{count}]")),
		Some(ArraySize::While(cond)) => text.push_str(&format!("[while({cond})]")),
	}
	if let Some(width) = &field.width {
		text.push_str(&format!(" : {width}"));
	}
	if let Some(at) = &field.placement {
		text.push_str(&format!(" @ {at}"));
	}
	if let Some(init) = &field.init {
		text.push_str(&format!(" = {init}"));
	}
	text
}

fn first_line(text: &str) -> String {
	let line = text.lines().next().unwrap_or("").trim();
	if line.len() > 120 {
		format!("{}...", &line[..117])
	} else {
		line.to_string()
	}
}

/// Where a member is written, for the note that says the structure ended here.
fn member_pos(member: &Member) -> Pos {
	match member {
		Member::Field(field) => field.pos,
		Member::Padding { pos, .. } => *pos,
		Member::If { pos, .. } => *pos,
		Member::Match { pos, .. } => *pos,
		Member::Try { pos, .. } => *pos,
		Member::Call { pos, .. } => *pos,
		Member::Statement(statement) => statement.pos,
	}
}

fn kind_name(kind: StatementKind) -> &'static str {
	match kind {
		StatementKind::Assign => "an assignment",
		StatementKind::While => "a `while` statement",
		StatementKind::For => "a `for` loop",
		StatementKind::Return => "a `return`",
		StatementKind::Break => "a `break`",
		StatementKind::Continue => "a `continue`",
		StatementKind::Call => "a call whose value is thrown away",
		StatementKind::Local => "a local variable of a function",
		StatementKind::If => "an `if` statement outside a structure",
		StatementKind::Match => "a `match` statement outside a structure",
		StatementKind::Try => "a `try`",
	}
}

/// `[[no_unique_address]]`: the field is read where it stands and takes no room
/// of its own, so the field after it starts in the same place.
fn takes_no_address(field: &AField) -> bool {
	field.attrs.iter().any(|a| a.path == "no_unique_address")
}

/// Whether a field is the structure's own machinery: a count, a length, a
/// reserved word, a field the pattern hides.
fn is_machinery(field: &AField) -> bool {
	field.attrs.iter().any(|a| a.path == "hidden")
}

/// Whether anything in these members assigns to `name`.
fn assigned_later(members: &[Member], name: &str) -> bool {
	members.iter().any(|member| assigns_to(member, name))
}

/// Whether this member, or anything inside it, assigns to `name`.
fn assigns_to(member: &Member, name: &str) -> bool {
	match member {
		Member::Statement(statement) => is_assignment_to(statement, name),
		Member::If { then, otherwise, .. } => assigned_later(then, name) || assigned_later(otherwise, name),
		Member::Match { arms, .. } => arms.iter().any(|arm| assigned_later(&arm.body, name)),
		Member::Try { body, catch, .. } => assigned_later(body, name) || assigned_later(catch, name),
		_ => false,
	}
}

fn is_assignment_to(statement: &super::ast::Statement, name: &str) -> bool {
	match &statement.assign {
		Some(assign) => assign.target == AssignTarget::Name(name.to_string()),
		None => false,
	}
}

/* ------------------------------------------------------------------ */
/* Locals one `if` settles                                             */
/* ------------------------------------------------------------------ */

/// Which members of one level the walk passes over, and what it emits instead.
#[derive(Default)]
struct Folded {
	/// Local declarations that wait for their `if`, by member index.
	decls: HashMap<usize, Fold>,
	/// Those declarations again, by the index of the `if` that settles each.
	at_if: HashMap<usize, Vec<usize>>,
}

/// Whether any of these statements reads the file: a declaration with an
/// address, or an `if` holding one.
fn places_anything(statements: &[super::ast::Statement]) -> bool {
	statements.iter().any(|statement| match statement.kind {
		StatementKind::Local => statement.decl.as_ref().is_some_and(|field| field.placement.is_some()),
		StatementKind::If => statement
			.branches
			.as_ref()
			.is_some_and(|branches| places_anything(&branches.then) || places_anything(&branches.otherwise)),
		_ => false,
	})
}

/// Whether every one of these statements is something the converter has a
/// reading for: a declaration, a call, or an `if` of the same.
///
/// A block with a `return`, an assignment or a loop in it is taken as a whole
/// instead, and reported once. Walking into it would put a gap on every
/// statement in place of the one gap on the `if`, which says less and counts
/// more.
fn every_statement_says_something(statements: &[super::ast::Statement]) -> bool {
	statements.iter().all(|statement| match statement.kind {
		StatementKind::Local => statement.decl.is_some(),
		StatementKind::Call => statement.call.is_some(),
		StatementKind::If => statement.branches.as_ref().is_some_and(|branches| {
			every_statement_says_something(&branches.then) && every_statement_says_something(&branches.otherwise)
		}),
		_ => false,
	})
}

/// The statement one half of a folded `if` holds.
fn branch_statement(member: &Member, then_half: bool, at: usize) -> Option<&super::ast::Statement> {
	let Member::If { then, otherwise, .. } = member else { return None };
	let half = if then_half { then } else { otherwise };
	match half.get(at) {
		Some(Member::Statement(statement)) => Some(statement),
		_ => None,
	}
}

/// Where the two halves of the value of one folded local are written.
#[derive(Clone)]
struct Fold {
	/// The index of the `if` that settles the value.
	at: usize,
	/// The assignment in the `then` half, as an index into its members.
	then: Option<usize>,
	/// The same for the `else` half.
	otherwise: Option<usize>,
}

/// The `if` that settles `name`, if exactly one does and nothing else touches
/// it.
fn fold_of(members: &[Member], decl: usize, name: &str) -> Option<Fold> {
	let mut found: Option<Fold> = None;
	for (index, member) in members.iter().enumerate() {
		if index == decl {
			continue;
		}
		if !assigns_to(member, name) {
			continue;
		}
		// Only an `if`, and only one of them, and only assignments written
		// straight into one of its halves.
		let Member::If { then, otherwise, .. } = member else { return None };
		if found.is_some() || index < decl {
			return None;
		}
		let this = branch_assignment(then, name)?;
		let other = branch_assignment(otherwise, name)?;
		if this.is_none() && other.is_none() {
			return None;
		}
		found = Some(Fold { at: index, then: this, otherwise: other });
	}
	found
}

/// The one assignment to `name` written directly in a block, if the block has
/// at most one and holds no other mention of it.
fn branch_assignment(members: &[Member], name: &str) -> Option<Option<usize>> {
	let mut at = None;
	for (index, member) in members.iter().enumerate() {
		if !assigns_to(member, name) {
			continue;
		}
		let Member::Statement(statement) = member else { return None };
		// A compound assignment reads the value it is changing, and the value
		// before the `if` is the one the IR would have to read.
		if statement.assign.as_ref().is_none_or(|assign| assign.op.is_some()) {
			return None;
		}
		if at.is_some() {
			return None;
		}
		at = Some(index);
	}
	Some(at)
}

/// The value one half of a folded `if` writes.
fn assigned_value(member: &Member, then_half: bool, at: usize) -> Option<&super::expr::Expr> {
	let Member::If { then, otherwise, .. } = member else { return None };
	let half = if then_half { then } else { otherwise };
	match half.get(at) {
		Some(Member::Statement(statement)) => statement.assign.as_ref().map(|assign| &assign.value),
		_ => None,
	}
}

impl Fold {
	/// Where in the `if`'s two halves this fold's assignments are written.
	fn assignments(&self) -> Vec<(bool, usize)> {
		let mut out = Vec::new();
		if let Some(at) = self.then {
			out.push((true, at));
		}
		if let Some(at) = self.otherwise {
			out.push((false, at));
		}
		out
	}
}
