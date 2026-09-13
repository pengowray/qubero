//! The `.ksy` spec model lowered to the template IR.
//!
//! The rule this file is built on: a construct the IR cannot express is named,
//! never approximated. Every field ends up in the report saying what it became;
//! anything that could not be said ends up there as a gap, with the path, the
//! text it came from and the reason, and the field is left as bytes of whatever
//! length is still known. Where the mapping is exact but indirect, that is a
//! note rather than a gap: a string compared against a fixed-size field is the
//! same comparison read as a big-endian number, and a reader looking only at
//! the IR would not guess where the number came from.
//!
//! Nothing here executes a `.ksy`. Once converted, a Kaitai format is a
//! `Template` like any other and every view works on it unchanged.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::codec::Codec;
use crate::template::{
	Anchor, Encoding, Expr, Part, StrLen, StructDef, Template, Ty, Until,
};

use super::expr::{BinOp, BoolOp, CmpOp, Expr as KExpr, TypeId, UnaryOp};
use super::imports::Imports;
use super::report::Report;
use super::spec::{
	AttrSpec, ByteSource, ClassSpec, InstanceSpec, ParamSpec, ProcessSpec, RepeatSpec,
	TypeRef, ValidSpec,
};
use super::yaml::KsyError;
use crate::template::Endian;

/// A `.ksy` read into the IR, with everything the conversion had to say about
/// it.
pub struct Converted {
	pub template: Template,
	pub report: Report,
}

/// One parsed `.ksy` and the prefix its types get in the IR's type table.
struct FileSpec {
	/// The import name this was loaded under, and the prefix its type names
	/// take. Empty for the file being converted.
	prefix: String,
	spec: ClassSpec,
}

/// Lower a `.ksy`, and the files it imports, to a template and a report.
pub fn convert(text: &str, imports: &dyn Imports) -> Result<Converted, KsyError> {
	let main = super::parse(text)?;
	let mut files = vec![FileSpec { prefix: String::new(), spec: main }];
	let mut report = Report::new();
	load_imports(&mut files, imports, &mut report)?;

	let mut lower = Lower {
		files: &files,
		report,
		types: Vec::new(),
		emitted: HashSet::new(),
		mono: HashMap::new(),
		parents: HashMap::new(),
		bit_offset: 0,
		pending: Vec::new(),
	};
	lower.collect_parents();

	let root_ctx = Ctx { stack: vec![&files[0].spec], names: vec![String::new()], elem: None };
	let root = lower.lower_class(&root_ctx, &file_name(&files[0].spec), &[]);

	// Every type the files declare, whether or not anything refers to it, so
	// that the panel and the diagram show the whole format. A type with
	// parameters is the exception: it has no meaning without its arguments and
	// exists only as the copies its call sites make.
	for (i, file) in files.iter().enumerate() {
		let ctx = Ctx { stack: vec![&file.spec], names: vec![file.prefix.clone()], elem: None };
		if i > 0 {
			let name = file.prefix.clone();
			lower.ensure(&name, &ctx);
		}
		lower.ensure_declared(&ctx);
	}

	let mut template = Template::new(&file_name(&files[0].spec), root);
	let mut part = Part::new(Ty::Bytes(Expr::Lit(0)));
	for (name, ty) in lower.types.drain(..) {
		part = part.with_type(&name, ty);
	}
	for (name, ty) in part.types {
		template = template.with_type(&name, ty);
	}
	Ok(Converted { template, report: lower.report })
}

fn file_name(spec: &ClassSpec) -> String {
	spec.meta.id.clone().unwrap_or_else(|| "ksy".to_string())
}

/// Load everything `meta/imports` names, and everything those import in turn.
fn load_imports(
	files: &mut Vec<FileSpec>,
	imports: &dyn Imports,
	report: &mut Report,
) -> Result<(), KsyError> {
	let mut seen: HashSet<String> = HashSet::new();
	let mut queue: Vec<(String, String)> = Vec::new();
	for name in &files[0].spec.meta.imports {
		queue.push((files[0].spec.meta.path.clone(), name.clone()));
	}
	while let Some((path, name)) = queue.pop() {
		if !seen.insert(name.clone()) {
			continue;
		}
		let Some(text) = imports.load(&name) else {
			report.gap(
				format!("{path}/imports"),
				name.clone(),
				"the imported .ksy was not available, so the types in it are missing",
			);
			continue;
		};
		let spec = super::parse(&text)?;
		let prefix = file_name(&spec);
		for next in &spec.meta.imports {
			queue.push((spec.meta.path.clone(), next.clone()));
		}
		files.push(FileSpec { prefix, spec });
	}
	Ok(())
}

/// Where a name is looked up from: the chain of types it is written inside,
/// outermost first, and the element type while a `repeat-until` is being read.
struct Ctx<'a> {
	stack: Vec<&'a ClassSpec>,
	/// The IR name of each level. `names[0]` is the file's prefix, empty for
	/// the file being converted.
	names: Vec<String>,
	elem: Option<&'a ClassSpec>,
}

impl<'a> Ctx<'a> {
	fn here(&self) -> &'a ClassSpec {
		self.stack.last().expect("a context always has a type")
	}

	fn inner(&self, name: &str, cls: &'a ClassSpec) -> Ctx<'a> {
		let mut stack = self.stack.clone();
		let mut names = self.names.clone();
		let outer = names.last().cloned().unwrap_or_default();
		stack.push(cls);
		names.push(if outer.is_empty() { name.to_string() } else { format!("{outer}.{name}") });
		Ctx { stack, names, elem: self.elem }
	}

	/// The IR name of the innermost type.
	fn ir_name(&self) -> String {
		self.names.last().cloned().unwrap_or_default()
	}
}

/// Everything a type can be asked for by name: its parameters, its fields and
/// its instances.
fn visible(cls: &ClassSpec) -> Vec<String> {
	let mut out: Vec<String> = cls.params.iter().map(ParamSpec::name).collect();
	out.extend(cls.seq.iter().map(AttrSpec::name));
	out.extend(cls.instances.iter().map(|(n, _)| n.clone()));
	out
}

struct Lower<'a> {
	files: &'a [FileSpec],
	report: Report,
	types: Vec<(String, Ty)>,
	emitted: HashSet<String>,
	/// Monomorphised copies, keyed by the type and the arguments it was called
	/// with, so two call sites passing the same thing share one copy.
	mono: HashMap<String, String>,
	/// Which types instantiate each type, which is what decides whether a
	/// `_parent` reaches a field that is always there.
	parents: HashMap<String, Vec<String>>,
	/// How many bits into the current byte the field being lowered starts,
	/// which is what decides whether a low-bit-first field runs into the next
	/// byte. Zero everywhere but in the middle of a run of `bN` fields.
	bit_offset: u32,
	/// Instances of the type being lowered that have not been written out yet.
	/// A field naming one of these is reading forwards, which the IR cannot do.
	pending: Vec<String>,
}

impl<'a> Lower<'a> {
	// -- the type table ---------------------------------------------------

	/// Lower a declared type into the table, once.
	fn ensure(&mut self, ir_name: &str, ctx: &Ctx<'a>) {
		if !self.emitted.insert(ir_name.to_string()) {
			return;
		}
		let ty = self.lower_class(ctx, ir_name, &[]);
		self.types.push((ir_name.to_string(), ty));
	}

	/// Every type declared under this one, recursively, skipping the ones that
	/// take parameters.
	fn ensure_declared(&mut self, ctx: &Ctx<'a>) {
		let here = ctx.here();
		for (name, inner) in &here.types {
			let inner_ctx = ctx.inner(name, inner);
			if inner.params.is_empty() {
				let ir = inner_ctx.ir_name();
				self.ensure(&ir, &inner_ctx);
			}
			self.ensure_declared(&inner_ctx);
		}
	}

	/// For every `type:` reference anywhere, note which type wrote it. What a
	/// `_parent` reaches depends on this.
	fn collect_parents(&mut self) {
		let mut found: Vec<(String, String)> = Vec::new();
		for file in self.files {
			let ctx = Ctx { stack: vec![&file.spec], names: vec![file.prefix.clone()], elem: None };
			self.collect_parents_in(&ctx, &mut found);
		}
		for (child, parent) in found {
			self.parents.entry(child).or_default().push(parent);
		}
	}

	fn collect_parents_in(&self, ctx: &Ctx<'a>, out: &mut Vec<(String, String)>) {
		let here = ctx.here();
		let me = ctx.ir_name();
		let note = |ty: &TypeRef, out: &mut Vec<(String, String)>| {
			for name in referenced_types(ty) {
				if let Some((ir, _)) = self.resolve_type(ctx, &name) {
					out.push((ir, me.clone()));
				}
			}
		};
		for attr in &here.seq {
			note(&attr.ty, out);
		}
		for (_, instance) in &here.instances {
			if let InstanceSpec::Parse(p) = instance {
				note(&p.attr.ty, out);
			}
		}
		for (name, inner) in &here.types {
			self.collect_parents_in(&ctx.inner(name, inner), out);
		}
	}

	/// Find the type a `type:` string names, the way the compiler finds it:
	/// from the type it is written in, then outwards, then across the imports.
	fn resolve_type(&self, ctx: &Ctx<'a>, name: &TypeId) -> Option<(String, &'a ClassSpec)> {
		let first = name.names.first()?;
		if !name.absolute {
			for level in (0..ctx.stack.len()).rev() {
				if let Some(found) = descend(ctx.stack[level], &name.names) {
					let base = ctx.names[level].clone();
					return Some((join_name(&base, &name.names), found));
				}
				// A type may also name itself or one of its own siblings by
				// the name the level above it filed it under.
				let _ = first;
			}
		}
		// An imported file's own id is the name of its top-level type, so
		// `riff::chunk` is the `chunk` declared inside the file called `riff`.
		for file in self.files {
			if file.prefix.is_empty() || file.prefix != *first {
				continue;
			}
			if name.names.len() == 1 {
				return Some((file.prefix.clone(), &file.spec));
			}
			if let Some(found) = descend(&file.spec, &name.names[1..]) {
				return Some((join_name(&file.prefix, &name.names[1..]), found));
			}
		}
		for file in self.files {
			if let Some(found) = descend(&file.spec, &name.names) {
				return Some((join_name(&file.prefix, &name.names), found));
			}
		}
		None
	}

	/// The enum a name refers to, as its value table.
	fn resolve_enum(&self, ctx: &Ctx<'a>, in_type: &TypeId, name: &str) -> Option<Vec<(i128, String)>> {
		let holder: &ClassSpec = if in_type.names.is_empty() {
			// The type it is written in, then outwards.
			let mut found = None;
			for level in (0..ctx.stack.len()).rev() {
				if ctx.stack[level].enums.iter().any(|(n, _)| n == name) {
					found = Some(ctx.stack[level]);
					break;
				}
			}
			match found {
				Some(c) => c,
				None => {
					let mut across = None;
					for file in self.files {
						if file.spec.enums.iter().any(|(n, _)| n == name) {
							across = Some(&file.spec);
							break;
						}
					}
					across?
				}
			}
		} else {
			self.resolve_type(ctx, in_type)?.1
		};
		let spec = holder.enums.iter().find(|(n, _)| n == name)?;
		Some(spec.1.values.iter().map(|v| (v.value, v.name.clone())).collect())
	}

	// -- one type ---------------------------------------------------------

	/// One `.ksy` type as a structure.
	///
	/// `params` are the arguments a call site passed, already lowered. They
	/// become zero-width fields at the front, named after the parameters, so
	/// that an expression inside the type reading a parameter reads a field
	/// like any other. They are machinery: they are how the type was called,
	/// not anything the file holds.
	fn lower_class(&mut self, ctx: &Ctx<'a>, ir_name: &str, params: &[(String, Expr)]) -> Ty {
		let here = ctx.here();
		let mut names: Vec<String> = Vec::new();
		let mut tys: Vec<Ty> = Vec::new();
		let mut docs: Vec<(String, String)> = Vec::new();

		for (name, arg) in params {
			names.push(name.clone());
			tys.push(Ty::Computed(arg.clone()));
		}

		// A Kaitai instance is worked out when it is asked for, so a `seq`
		// field may read one declared below it. The IR reads backwards only, so
		// the ones that depend on nothing the file holds are written out first,
		// where a field before them can see them, and the rest stay after the
		// `seq` where they can see it.
		let early = early_instances(here);
		let outer_pending = std::mem::take(&mut self.pending);
		for (name, instance) in &here.instances {
			if !early.contains(name) {
				continue;
			}
			match self.lower_instance(ctx, instance) {
				Ok(ty) => {
					self.report.became(
						instance.path().to_string(),
						instance_source(instance),
						ty.display_name(),
					);
					if let Some(doc) = &instance.doc().summary {
						docs.push((name.clone(), doc.to_string()));
					}
					names.push(name.clone());
					tys.push(ty);
				}
				Err(gap) => {
					self.report.gap(
						instance.path().to_string(),
						instance_source(instance),
						format!("{} (the instance is dropped)", gap.reason),
					);
				}
			}
		}
		self.pending = here
			.instances
			.iter()
			.map(|(n, _)| n.clone())
			.filter(|n| !early.contains(n))
			.collect();

		let mut placeable = true;
		// Kaitai reads bits with a bit position of its own and throws away
		// what is left of the current byte the moment a field that is not
		// bits comes along. Nothing in the IR does that by itself, so the
		// bits thrown away are written out as a field: without them every
		// offset after a `b3` would be wrong by five bits.
		let mut loose_bits: u32 = 0;
		let mut pads = 0;
		for attr in &here.seq {
			let name = attr.name();
			let width = bit_width(attr);
			if width.is_none() && loose_bits % 8 != 0 {
				pads += 1;
				let pad_name =
					if pads == 1 { "padding".to_string() } else { format!("padding{pads}") };
				self.report.note(
					attr.path.clone(),
					source_of(attr),
					format!(
						"Kaitai steps to the next byte here, because the fields before this one were bits; the {} bits it steps over are the `{pad_name}` field",
						8 - loose_bits % 8
					),
				);
				names.push(pad_name);
				tys.push(Ty::UInt { bits: 8 - loose_bits % 8, endian: Endian::Big });
			}
			self.bit_offset = loose_bits;
			loose_bits = match width {
				Some(w) => (loose_bits + w) % 8,
				None => 0,
			};
			if !placeable {
				self.report.gap(
					attr.path.clone(),
					source_of(attr),
					"not placed: the size of an earlier field in this type is unknown",
				);
				continue;
			}
			match self.lower_attr(ctx, attr) {
				Ok(ty) => {
					self.report.became(attr.path.clone(), source_of(attr), ty.display_name());
					if let Some(doc) = attr_doc(attr) {
						docs.push((name.clone(), doc));
					}
					names.push(name);
					tys.push(ty);
				}
				Err(gap) => {
					self.report.gap(attr.path.clone(), source_of(attr), gap.reason);
					if let Some(ty) = gap.fallback {
						names.push(name);
						tys.push(ty);
					}
					placeable = gap.placeable;
				}
			}
		}

		self.bit_offset = 0;
		for (name, instance) in &here.instances {
			if early.contains(name) {
				continue;
			}
			self.pending.retain(|n| n != name);
			match self.lower_instance(ctx, instance) {
				Ok(ty) => {
					self.report.became(
						instance.path().to_string(),
						instance_source(instance),
						ty.display_name(),
					);
					if let Some(doc) = &instance.doc().summary {
						docs.push((name.clone(), doc.to_string()));
					}
					names.push(name.clone());
					tys.push(ty);
				}
				Err(gap) => {
					self.report.gap(
						instance.path().to_string(),
						instance_source(instance),
						format!("{} (the instance is dropped)", gap.reason),
					);
				}
			}
		}

		self.pending = outer_pending;
		let fields: Vec<(&str, Ty)> =
			names.iter().map(String::as_str).zip(tys.into_iter()).collect();
		let mut ty = Ty::structure(&struct_name(ir_name, here), fields);
		let mut machinery: Vec<&str> = params.iter().map(|(n, _)| n.as_str()).collect();
		machinery.extend(names.iter().filter(|n| n.starts_with("padding")).map(String::as_str));
		if !machinery.is_empty() {
			ty = ty.machinery(&machinery);
		}
		for (field, doc) in &docs {
			ty = ty.field_doc(field, doc);
		}
		if let Some(doc) = class_doc(here) {
			ty = ty.doc(&doc);
		}
		self.apply_representation(ctx, here, ty)
	}

	/// `-webide-representation` says what one of these reads as on a single
	/// line, which is what [`StructDef::line`] holds. A representation that is
	/// one field and nothing else names the record instead.
	fn apply_representation(&mut self, _ctx: &Ctx<'a>, cls: &ClassSpec, ty: Ty) -> Ty {
		let Some((_, node)) = cls.vendor.iter().find(|(k, _)| k == "-webide-representation") else {
			return ty;
		};
		let Ok(text) = node.as_str() else { return ty };
		let path = node.path.to_string();
		let parts = match parse_representation(&text) {
			Ok(parts) => parts,
			Err(reason) => {
				// A representation with no field in it is a fixed label, and a
				// fixed label says no more than the type's own name does.
				self.report.note(path, text, reason);
				return ty;
			}
		};
		let known: Vec<String> = visible(cls);
		let mut line: Vec<(String, String, String)> = Vec::new();
		for (word, field, suffix) in parts {
			if field.contains('.') {
				self.report.note(
					path.clone(),
					text.clone(),
					format!("`{field}` reads a field inside another one, which a one-line reading cannot name; left out of the line"),
				);
				continue;
			}
			if !known.contains(&field) {
				self.report.note(
					path.clone(),
					text.clone(),
					format!("`{field}` is not a field of this type; left out of the line"),
				);
				continue;
			}
			if !suffix.is_empty() {
				self.report.note(
					path.clone(),
					text.clone(),
					format!("`{field}:{suffix}` asks for a number base the line has no way to set; the field's own reading is used"),
				);
			}
			line.push((field, word, String::new()));
		}
		if line.is_empty() {
			return ty;
		}
		if line.len() == 1 && line[0].1.is_empty() {
			// One field and no words around it: that field is the record's name.
			return match ty {
				Ty::Struct(s) => Ty::Struct(Arc::new(StructDef {
					named_by: Some(line[0].0.clone()),
					..(*s).clone()
				})),
				other => other,
			};
		}
		let borrowed: Vec<(&str, &str, &str)> =
			line.iter().map(|(f, w, q)| (f.as_str(), w.as_str(), q.as_str())).collect();
		ty.reads_as(&borrowed)
	}

	// -- one field --------------------------------------------------------

	fn lower_attr(&mut self, ctx: &Ctx<'a>, attr: &AttrSpec) -> Result<Ty, Gap> {
		self.note_validation(attr);
		let base = match self.lower_attr_base(ctx, attr) {
			Ok(ty) => ty,
			Err(gap) => {
				// The type could not be said. What stands in its place still
				// has to take up the room the field takes, or nothing after it
				// in the structure can be placed: a repeat multiplies it, and
				// a condition may leave it out.
				let Some(fallback) = gap.fallback else { return Err(gap) };
				if !gap.placeable {
					return Err(Gap { reason: gap.reason, fallback: Some(fallback), placeable: false });
				}
				let ty = match self.apply_repeat(ctx, attr, fallback) {
					Ok(ty) => ty,
					Err(inner) => {
						return Err(Gap {
							reason: gap.reason,
							fallback: inner.fallback,
							placeable: false,
						});
					}
				};
				let ty = match (&attr.if_expr, self.condition(ctx, attr)) {
					(None, _) => ty,
					(Some(_), Some(cond)) => Ty::when(cond, ty),
					(Some(_), None) => {
						return Err(Gap { reason: gap.reason, fallback: Some(ty), placeable: false });
					}
				};
				return Err(Gap { reason: gap.reason, fallback: Some(ty), placeable: true });
			}
		};
		let mut ty = self.apply_enum(ctx, attr, base)?;
		ty = self.apply_repeat(ctx, attr, ty)?;
		if let Some(cond) = &attr.if_expr {
			let cond = self
				.lower_expr(ctx, &attr.path, cond)
				.map_err(|r| Gap::keeping(format!("`if`: {r}"), ty.clone()))?;
			ty = Ty::when(cond, ty);
		}
		Ok(ty)
	}

	fn condition(&mut self, ctx: &Ctx<'a>, attr: &AttrSpec) -> Option<Expr> {
		let cond = attr.if_expr.as_ref()?;
		self.lower_expr(ctx, &attr.path, cond).ok()
	}

	/// The field's own type, before a repeat or a condition is put round it.
	/// A type that could not be said falls back to bytes of whatever length is
	/// still known, so that the fields after it keep their places.
	fn lower_attr_base(&mut self, ctx: &Ctx<'a>, attr: &AttrSpec) -> Result<Ty, Gap> {
		match self.lower_attr_base_inner(ctx, attr) {
			Ok(ty) => Ok(ty),
			Err(gap) if !gap.placeable => match self.known_length(ctx, attr) {
				Some(len) => Err(Gap::bytes(gap.reason, len)),
				None => Err(gap),
			},
			Err(gap) => Err(gap),
		}
	}

	/// How long the field is, where the `.ksy` says so outright: a `size`, a
	/// `size-eos`, or the width the type has by being that type.
	fn known_length(&mut self, ctx: &Ctx<'a>, attr: &AttrSpec) -> Option<Expr> {
		if let Some(bytes) = &attr.contents {
			return Some(Expr::Lit(bytes.len() as i128));
		}
		match &attr.ty {
			TypeRef::Bytes { source }
			| TypeRef::Str { source, .. }
			| TypeRef::UserFromBytes { source, .. } => self.byte_length(ctx, attr, source),
			TypeRef::Int { width, .. } | TypeRef::Float { width, .. } => {
				Some(Expr::Lit(i128::from(*width)))
			}
			TypeRef::Switch(_) => attr
				.size
				.as_ref()
				.and_then(|e| self.lower_expr(ctx, &attr.path, e).ok())
				.or(if attr.size_eos { Some(Expr::Remaining) } else { None }),
			TypeRef::Bits { .. } | TypeRef::User { .. } => None,
		}
	}

	fn lower_attr_base_inner(&mut self, ctx: &Ctx<'a>, attr: &AttrSpec) -> Result<Ty, Gap> {
		if let Some(bytes) = &attr.contents {
			return Ok(Ty::magic(bytes));
		}
		if let Some(ValidSpec::Eq(KExpr::List(items))) = &attr.valid {
			if let Some(bytes) = byte_list(items) {
				if matches!(attr.ty, TypeRef::Bytes { .. }) {
					return Ok(Ty::magic(&bytes));
				}
			}
		}
		match &attr.ty {
			TypeRef::Bytes { source } => {
				// Padding and a terminator take bytes off the *value* while
				// leaving the field the same length. The IR can say that of
				// text and not of raw bytes, so the field covers the right
				// bytes and its value holds what Kaitai would have dropped.
				if attr.pad_right.is_some() || (attr.terminator.is_some() && !matches!(source, ByteSource::Terminated)) {
					let what = if attr.pad_right.is_some() { "`pad-right`" } else { "`terminator`" };
					let len = self.byte_length(ctx, attr, source);
					return Err(Gap {
						reason: format!(
							"{what} takes bytes off the value of a byte field, which the IR can say of text and not of bytes; the value here holds them"
						),
						fallback: len.map(Ty::Bytes),
						placeable: true,
					});
				}
				let inner = |len: Expr| Ty::Bytes(len);
				self.from_bytes(ctx, attr, source, inner, "bytes")
			}
			TypeRef::Int { signed, width, endian } => {
				let endian = self.fixed_endian(*endian, *width == 1)?;
				let bits = u32::from(*width) * 8;
				Ok(if *signed { Ty::Int { bits, endian } } else { Ty::UInt { bits, endian } })
			}
			TypeRef::Float { width, endian } => {
				let endian = self.fixed_endian(*endian, false)?;
				Ok(if *width == 4 { Ty::F32(endian) } else { Ty::F64(endian) })
			}
			// A `bN` field is bits packed into a byte, and `Endian` at
			// sub-byte widths says which end of the byte they are taken from:
			// `Big` is MSB-first and `Little` LSB-first (see `template::Endian`).
			// Kaitai's `bit-endian: be` packs from the most significant bit and
			// `le` from the least, so the two words mean the same thing and the
			// mapping is the identity. `meta/endian` has no say in it.
			TypeRef::Bits { width, endian } => {
				// The IR reads low-bit-first fields out of one byte: a field
				// packed from the bottom that runs into the next byte is not a
				// single range of bits and it refuses to place one. Kaitai's
				// `bit-endian: le` does read such a field, so this is a gap
				// rather than something to guess at.
				if *endian == Endian::Little && self.bit_offset % 8 + *width > 8 {
					return Err(Gap {
						reason: format!(
							"a {width}-bit field packed low-bit-first starting {} bits into a byte runs into the next byte, which the IR cannot read as one number",
							self.bit_offset % 8
						),
						fallback: Some(Ty::UInt { bits: *width, endian: Endian::Big }),
						placeable: true,
					});
				}
				Ok(Ty::UInt { bits: *width, endian: *endian })
			}
			TypeRef::Str { zero_terminated, encoding, source } => {
				self.lower_str(ctx, attr, *zero_terminated, encoding, source)
			}
			TypeRef::User { name, args } => {
				let named = self.user_type(ctx, attr, name, args)?;
				Ok(named)
			}
			TypeRef::UserFromBytes { name, args, source } => {
				let named = self.user_type(ctx, attr, name, args)?;
				self.from_bytes(ctx, attr, source, move |len| Ty::sized(len, named.clone()), "a type")
			}
			TypeRef::Switch(switch) => self.lower_switch(ctx, attr, switch),
		}
	}

	/// Put a length round something read out of a run of bytes, and unpack the
	/// run first where `process` says to.
	fn from_bytes(
		&mut self,
		ctx: &Ctx<'a>,
		attr: &AttrSpec,
		source: &ByteSource,
		build: impl Fn(Expr) -> Ty,
		what: &str,
	) -> Result<Ty, Gap> {
		let len = match source {
			ByteSource::Limit(e) => self
				.lower_expr(ctx, &attr.path, e)
				.map_err(|r| Gap::unplaceable(format!("`size`: {r}")))?,
			ByteSource::Eos => Expr::Remaining,
			ByteSource::Terminated => {
				return Err(Gap::unplaceable(format!(
					"{what} ended by a terminator rather than a length, which the IR has no form for"
				)));
			}
		};
		match &attr.process {
			None => Ok(build(len)),
			Some(ProcessSpec::Zlib) => {
				// The length is the packed run's; what is read inside it is
				// read over the bytes that come out.
				let inner = build(Expr::Remaining);
				Ok(Ty::decoded(len, Codec::Zlib, inner))
			}
			Some(other) => Err(Gap::bytes(
				format!("`process: {}` is not a codec the IR has", process_name(other)),
				len,
			)),
		}
	}

	fn lower_str(
		&mut self,
		ctx: &Ctx<'a>,
		attr: &AttrSpec,
		zero_terminated: bool,
		encoding: &str,
		source: &ByteSource,
	) -> Result<Ty, Gap> {
		let Some(enc) = map_encoding(encoding) else {
			let len = self.byte_length(ctx, attr, source);
			return Err(Gap {
				reason: format!("`encoding: {encoding}` is not one the IR can read"),
				fallback: len.map(Ty::Bytes),
				placeable: true,
			});
		};
		if attr.include {
			return Err(Gap::unplaceable(
				"`include: true` keeps the terminator in the value, which the IR has no form for"
					.to_string(),
			));
		}
		let terminator = attr.terminator.clone();
		if let Some(term) = &terminator {
			if term.len() > 1 {
				return Err(Gap::unplaceable(format!(
					"a {}-byte terminator, where the IR's text fields end at one byte",
					term.len()
				)));
			}
		}
		let end = terminator.as_ref().and_then(|t| t.first().copied());
		let or_end = !attr.eos_error;
		let len = match (source, end) {
			(ByteSource::Terminated, Some(end)) => StrLen::Terminated { end, or_end },
			(ByteSource::Limit(e), _) => {
				let size = self
					.lower_expr(ctx, &attr.path, e)
					.map_err(|r| Gap::unplaceable(format!("`size`: {r}")))?;
				match (attr.pad_right, end) {
					(Some(pad), _) => StrLen::Padded { size, pad },
					// A sized `strz` is a string inside a fixed run: what is
					// past the terminator is the format's padding.
					(None, Some(0)) if zero_terminated => StrLen::Padded { size, pad: 0 },
					_ => StrLen::Fixed(size),
				}
			}
			(ByteSource::Eos, _) => match attr.pad_right {
				Some(pad) => StrLen::Padded { size: Expr::Remaining, pad },
				None => StrLen::Fixed(Expr::Remaining),
			},
			(ByteSource::Terminated, None) => {
				return Err(Gap::unplaceable(
					"a text field with no size and no terminator".to_string(),
				));
			}
		};
		Ok(Ty::text(len, enc))
	}

	/// A reference to a type the file declares, monomorphised where the call
	/// passes arguments.
	fn user_type(
		&mut self,
		ctx: &Ctx<'a>,
		attr: &AttrSpec,
		name: &TypeId,
		args: &[KExpr],
	) -> Result<Ty, Gap> {
		let Some((ir, cls)) = self.resolve_type(ctx, name) else {
			return Err(Gap::sized(format!("no type named `{name}` is in scope")));
		};
		if args.is_empty() && cls.params.is_empty() {
			let target = ctx.inner("", cls);
			let _ = target;
			self.ensure_type_named(&ir, cls);
			return Ok(Ty::Named(ir.into()));
		}
		if args.len() != cls.params.len() {
			return Err(Gap::sized(format!(
				"`{name}` takes {} arguments and the call passes {}",
				cls.params.len(),
				args.len()
			)));
		}
		// A parameter that shadows something the type could otherwise read
		// would change what every expression inside it means, so the copy is
		// refused rather than made wrong.
		let inside = visible(cls);
		for param in &cls.params {
			let pname = param.name();
			if inside.iter().filter(|n| **n == pname).count() > 1 {
				return Err(Gap::sized(format!(
					"the parameter `{pname}` of `{name}` has the same name as a field of it"
				)));
			}
		}
		let mut lowered = Vec::new();
		for (param, arg) in cls.params.iter().zip(args) {
			let e = self
				.lower_expr(ctx, &attr.path, arg)
				.map_err(|r| Gap::sized(format!("argument `{}` of `{name}`: {r}", param.name())))?;
			lowered.push((param.name(), e));
		}
		let key = format!("{ir}({:?})", lowered.iter().map(|(_, e)| e).collect::<Vec<_>>());
		if let Some(existing) = self.mono.get(&key) {
			return Ok(Ty::Named(existing.as_str().into()));
		}
		let written: Vec<String> = args.iter().map(ToString::to_string).collect();
		let mut copy = format!("{ir}({})", written.join(", "));
		let mut n = 2;
		while self.emitted.contains(&copy) {
			copy = format!("{ir}({})~{n}", written.join(", "));
			n += 1;
		}
		self.mono.insert(key, copy.clone());
		self.emitted.insert(copy.clone());
		let Some(target) = self.ctx_for(&ir, cls) else {
			return Err(Gap::sized(format!("`{name}` could not be placed in its own file")));
		};
		let ty = self.lower_class(&target, &copy, &lowered);
		self.types.push((copy.clone(), ty));
		Ok(Ty::Named(copy.into()))
	}

	/// Lower a type by its IR name, if nothing has yet.
	fn ensure_type_named(&mut self, ir: &str, cls: &'a ClassSpec) {
		if self.emitted.contains(ir) {
			return;
		}
		if let Some(ctx) = self.ctx_for(ir, cls) {
			self.ensure(ir, &ctx);
		}
	}

	/// Rebuild the scope a type sits in from its IR name, which is the chain of
	/// names it is nested under.
	fn ctx_for(&self, ir: &str, cls: &'a ClassSpec) -> Option<Ctx<'a>> {
		for file in self.files {
			let rest = if file.prefix.is_empty() {
				Some(ir)
			} else if ir == file.prefix {
				Some("")
			} else {
				ir.strip_prefix(&format!("{}.", file.prefix))
			};
			let Some(rest) = rest else { continue };
			let mut stack: Vec<&'a ClassSpec> = vec![&file.spec];
			let mut names = vec![file.prefix.clone()];
			let mut here = &file.spec;
			let mut ok = true;
			if !rest.is_empty() {
				for part in rest.split('.') {
					match here.types.iter().find(|(n, _)| n == part) {
						Some((_, inner)) => {
							here = inner;
							stack.push(inner);
							let outer = names.last().cloned().unwrap_or_default();
							names.push(if outer.is_empty() {
								part.to_string()
							} else {
								format!("{outer}.{part}")
							});
						}
						None => {
							ok = false;
							break;
						}
					}
				}
			}
			if ok && std::ptr::eq(here, cls) {
				return Some(Ctx { stack, names, elem: None });
			}
		}
		None
	}

	fn lower_switch(
		&mut self,
		ctx: &Ctx<'a>,
		attr: &AttrSpec,
		switch: &super::spec::SwitchSpec,
	) -> Result<Ty, Gap> {
		let on = self
			.lower_expr(ctx, &attr.path, &switch.on)
			.map_err(|r| Gap::sized(format!("`switch-on`: {r}")))?;
		let mut ints: Vec<(i128, Ty)> = Vec::new();
		let mut texts: Vec<(String, Ty)> = Vec::new();
		let mut default: Option<Ty> = None;
		for case in &switch.cases {
			let case_attr = AttrSpec { ty: case.ty.clone(), ..attr.clone() };
			let ty = match self.lower_attr_base(ctx, &case_attr) {
				Ok(ty) => ty,
				Err(gap) => {
					return Err(Gap::sized(format!("case `{}`: {}", case.key_text, gap.reason)));
				}
			};
			if case.is_else() {
				default = Some(ty);
				continue;
			}
			match &case.key {
				KExpr::IntNum(n) => ints.push((*n, ty)),
				KExpr::Bool(b) => ints.push((i128::from(*b), ty)),
				KExpr::EnumByLabel { enum_name, label, in_type } => {
					match self.enum_value(ctx, in_type, enum_name, label) {
						Some(v) => ints.push((v, ty)),
						None => {
							return Err(Gap::sized(format!(
								"case `{}` names an enum member that is not in scope",
								case.key_text
							)));
						}
					}
				}
				KExpr::Str(s) => texts.push((s.clone(), ty)),
				KExpr::List(items) => match byte_list(items) {
					Some(bytes) if bytes.len() <= 16 => {
						self.report.note(
							case.path.clone(),
							case.key_text.clone(),
							"the case is those bytes read as one big-endian number, which is what the field they are compared with reads as".to_string(),
						);
						ints.push((be_int(&bytes), ty));
					}
					_ => {
						return Err(Gap::sized(format!(
							"case `{}` is a byte array the IR cannot compare",
							case.key_text
						)));
					}
				},
				other => {
					return Err(Gap::sized(format!(
						"case `{}` is not a value the IR can switch on: {other}",
						case.key_text
					)));
				}
			}
		}
		if !ints.is_empty() && !texts.is_empty() {
			return Err(Gap::sized(
				"the cases mix numbers and text, and a switch reads one or the other".to_string(),
			));
		}
		let default = match default {
			Some(ty) => ty,
			None => {
				self.report.note(
					attr.path.clone(),
					source_of(attr),
					"no default case, so a value none of the cases name reads as nothing"
						.to_string(),
				);
				Ty::Bytes(Expr::Lit(0))
			}
		};
		if !texts.is_empty() {
			let cases: Vec<(&str, Ty)> =
				texts.iter().map(|(k, t)| (k.as_str(), t.clone())).collect();
			return Ok(Ty::matches(on, cases, default));
		}
		Ok(Ty::switch(on, ints, default))
	}

	fn apply_enum(&mut self, ctx: &Ctx<'a>, attr: &AttrSpec, ty: Ty) -> Result<Ty, Gap> {
		let Some(name) = &attr.enum_ref else { return Ok(ty) };
		let id = TypeId::plain(name.split("::").map(str::to_string).collect());
		let bare = id.names.last().cloned().unwrap_or_default();
		let holder = TypeId::plain(id.names[..id.names.len() - 1].to_vec());
		let Some(cases) = self.resolve_enum(ctx, &holder, &bare) else {
			return Err(Gap::keeping(format!("no enum named `{name}` is in scope"), ty));
		};
		let borrowed: Vec<(i128, &str)> = cases.iter().map(|(v, n)| (*v, n.as_str())).collect();
		let mut out = Ty::enumeration(&bare, ty, &borrowed);
		if let Some(spec) = self.enum_spec(ctx, &holder, &bare) {
			for value in &spec {
				if let Some(doc) = &value.1 {
					out = out.enum_doc(value.0, doc);
				}
			}
		}
		Ok(out)
	}

	/// The docs written beside an enum's members, if any.
	fn enum_spec(
		&self,
		ctx: &Ctx<'a>,
		in_type: &TypeId,
		name: &str,
	) -> Option<Vec<(i128, Option<String>)>> {
		let holder: &ClassSpec = if in_type.names.is_empty() {
			let mut found = None;
			for level in (0..ctx.stack.len()).rev() {
				if ctx.stack[level].enums.iter().any(|(n, _)| n == name) {
					found = Some(ctx.stack[level]);
					break;
				}
			}
			match found {
				Some(c) => c,
				None => self.files.iter().find(|f| f.spec.enums.iter().any(|(n, _)| n == name)).map(|f| &f.spec)?,
			}
		} else {
			self.resolve_type(ctx, in_type)?.1
		};
		let spec = holder.enums.iter().find(|(n, _)| n == name)?;
		Some(spec.1.values.iter().map(|v| (v.value, v.doc.summary.clone())).collect())
	}

	fn apply_repeat(&mut self, ctx: &Ctx<'a>, attr: &AttrSpec, ty: Ty) -> Result<Ty, Gap> {
		match &attr.repeat {
			RepeatSpec::No => Ok(ty),
			RepeatSpec::Eos => Ok(Ty::repeat(ty, Until::End)),
			RepeatSpec::Expr(e) => {
				let count = self
					.lower_expr(ctx, &attr.path, e)
					.map_err(|r| Gap::keeping(format!("`repeat-expr`: {r}"), ty.clone()))?;
				Ok(Ty::array(ty, count))
			}
			RepeatSpec::Until(e) => {
				let until = self
					.lower_until(ctx, attr, e)
					.map_err(|r| Gap::keeping(format!("`repeat-until`: {r}"), ty.clone()))?;
				Ok(Ty::repeat(ty, until))
			}
		}
	}

	/// What ends a `repeat: until` run.
	fn lower_until(
		&mut self,
		ctx: &Ctx<'a>,
		attr: &AttrSpec,
		e: &KExpr,
	) -> Result<Until, String> {
		// `_io.eof` is the run filling its container, which is what
		// `Until::End` says and says better.
		if let KExpr::Attribute { value, attr: name } = e {
			if matches!(&**value, KExpr::Name(n) if n == "_io") && name == "eof" {
				return Ok(Until::End);
			}
		}
		// `_.field == literal` is one field against one fixed thing, which the
		// IR has a shape for and shows as the fact it is.
		if let KExpr::Compare { left, op: CmpOp::Eq, right } = e {
			if let KExpr::Attribute { value, attr: field } = &**left {
				if matches!(&**value, KExpr::Name(n) if n == "_") {
					let value = match &**right {
						KExpr::IntNum(n) => Some(*n),
						KExpr::Bool(b) => Some(i128::from(*b)),
						KExpr::EnumByLabel { enum_name, label, in_type } => {
							self.enum_value(ctx, in_type, enum_name, label)
						}
						_ => None,
					};
					if let Some(value) = value {
						return Ok(Until::FieldValue { field: field.clone(), value });
					}
				}
			}
		}
		let elem = self.element_class(ctx, attr);
		let inner = Ctx { stack: ctx.stack.clone(), names: ctx.names.clone(), elem };
		Ok(Until::Cond(self.lower_expr(&inner, &attr.path, e)?))
	}

	/// The type one element of a repeating field has, so that `_.field` inside
	/// a `repeat-until` can be checked.
	fn element_class(&self, ctx: &Ctx<'a>, attr: &AttrSpec) -> Option<&'a ClassSpec> {
		let name = match &attr.ty {
			TypeRef::User { name, .. } | TypeRef::UserFromBytes { name, .. } => name,
			_ => return None,
		};
		self.resolve_type(ctx, name).map(|(_, cls)| cls)
	}

	fn note_validation(&mut self, attr: &AttrSpec) {
		let Some(valid) = &attr.valid else { return };
		if attr.contents.is_some() {
			// Already a `Magic`, which is the check.
			return;
		}
		if matches!(valid, ValidSpec::Eq(KExpr::List(items)) if byte_list(items).is_some())
			&& matches!(attr.ty, TypeRef::Bytes { .. })
		{
			return;
		}
		self.report.note(
			format!("{}/valid", attr.path),
			source_of(attr),
			"the value constraint is not carried over: the IR's checks are checksums, and it has no form for a constraint on one field's value"
				.to_string(),
		);
	}

	// -- instances --------------------------------------------------------

	fn lower_instance(&mut self, ctx: &Ctx<'a>, instance: &'a InstanceSpec) -> Result<Ty, Gap> {
		match instance {
			InstanceSpec::Value(v) => {
				if let Some(reason) = not_an_integer(&v.value) {
					return Err(Gap::dropped(reason));
				}
				let e = self
					.lower_expr(ctx, &v.path, &v.value)
					.map_err(|r| Gap::dropped(format!("`value`: {r}")))?;
				let mut ty = Ty::Computed(e);
				if let Some(name) = &v.enum_ref {
					let id = TypeId::plain(name.split("::").map(str::to_string).collect());
					let bare = id.names.last().cloned().unwrap_or_default();
					let holder = TypeId::plain(id.names[..id.names.len() - 1].to_vec());
					let Some(cases) = self.resolve_enum(ctx, &holder, &bare) else {
						return Err(Gap::dropped(format!("no enum named `{name}` is in scope")));
					};
					let borrowed: Vec<(i128, &str)> =
						cases.iter().map(|(v, n)| (*v, n.as_str())).collect();
					ty = Ty::enumeration(&bare, ty, &borrowed);
				}
				if let Some(cond) = &v.if_expr {
					let cond = self
						.lower_expr(ctx, &v.path, cond)
						.map_err(|r| Gap::dropped(format!("`if`: {r}")))?;
					ty = Ty::when(cond, ty);
				}
				Ok(ty)
			}
			InstanceSpec::Parse(p) => {
				let inner = self.lower_attr(ctx, &p.attr).map_err(|g| Gap::dropped(g.reason))?;
				let Some(pos) = &p.pos else {
					// No `pos`: the field is read where it stands, which for
					// something outside `seq` is nowhere the IR can put it.
					return Err(Gap::dropped(
						"an instance with no `pos` has no place in the file the IR can name"
							.to_string(),
					));
				};
				let at = self
					.lower_expr(ctx, &p.path, pos)
					.map_err(|r| Gap::dropped(format!("`pos`: {r}")))?;
				let anchor = self.anchor_for(p.io.as_ref())?;
				let inner = match &p.attr.if_expr {
					// The condition already wrapped the inner type; keep the
					// `At` inside it so an absent field is placed nowhere.
					Some(_) => inner,
					None => inner,
				};
				Ok(Ty::At { anchor, at, inner: Box::new(inner) })
			}
		}
	}

	/// Which stream an instance's `pos` counts from.
	fn anchor_for(&self, io: Option<&KExpr>) -> Result<Anchor, Gap> {
		let Some(io) = io else { return Ok(Anchor::Window) };
		match io {
			KExpr::Name(n) if n == "_io" => Ok(Anchor::Window),
			KExpr::Attribute { value, attr } if attr == "_io" => match &**value {
				KExpr::Name(n) if n == "_root" => Ok(Anchor::File),
				KExpr::Name(n) if n == "_parent" => Ok(Anchor::Window),
				other => Err(Gap::dropped(format!(
					"`io: {other}._io` reads inside another field's stream, which the IR has no anchor for"
				))),
			},
			other => {
				Err(Gap::dropped(format!("`io: {other}` is not a stream the IR has an anchor for")))
			}
		}
	}

	// -- helpers ----------------------------------------------------------

	fn fixed_endian(&self, endian: Option<Endian>, one_byte: bool) -> Result<Endian, Gap> {
		match endian {
			Some(e) => Ok(e),
			None if one_byte => Ok(Endian::Big),
			None => Err(Gap::unplaceable(
				"the endianness is chosen while the file is read, and the IR has no form for that"
					.to_string(),
			)),
		}
	}

	/// The field's length in bytes, where it has one that lowers.
	fn byte_length(
		&mut self,
		ctx: &Ctx<'a>,
		attr: &AttrSpec,
		source: &ByteSource,
	) -> Option<Expr> {
		match source {
			ByteSource::Limit(e) => self.lower_expr(ctx, &attr.path, e).ok(),
			ByteSource::Eos => Some(Expr::Remaining),
			ByteSource::Terminated => None,
		}
	}

	fn enum_value(
		&self,
		ctx: &Ctx<'a>,
		in_type: &TypeId,
		enum_name: &str,
		label: &str,
	) -> Option<i128> {
		let cases = self.resolve_enum(ctx, in_type, enum_name)?;
		cases.iter().find(|(_, n)| n == label).map(|(v, _)| *v)
	}
}

// ---------------------------------------------------------------------------
// Expressions
// ---------------------------------------------------------------------------

/// One step of a chain like `a.b[2].c`.
enum Seg<'a> {
	Name(&'a str),
	Index(&'a KExpr),
}

/// What a chain has read so far.
enum Acc {
	/// A field, and names down into it.
	Path(Vec<String>),
	/// One element of a list, and names down into that.
	Elem { array: Vec<String>, index: Expr, field: Vec<String> },
	/// A value nothing more can be read out of but a method.
	Done(Expr),
}

/// Split `a.b[2].c` into its base and the steps after it, dropping any cast:
/// `.as<t>` says what a value is, and the IR reads the value either way.
fn flatten(e: &KExpr) -> (&KExpr, Vec<Seg<'_>>) {
	match e {
		KExpr::Attribute { value, attr } => {
			let (base, mut segs) = flatten(value);
			segs.push(Seg::Name(attr));
			(base, segs)
		}
		KExpr::Subscript { value, index } => {
			let (base, mut segs) = flatten(value);
			segs.push(Seg::Index(index));
			(base, segs)
		}
		KExpr::CastToType { value, .. } => flatten(value),
		other => (other, Vec::new()),
	}
}

/// The names that read something *about* a value rather than inside it.
fn is_method(name: &str) -> bool {
	matches!(
		name,
		"size" | "length" | "to_i" | "to_s" | "first" | "last" | "reverse" | "substring" | "value"
	)
}

impl<'a> Lower<'a> {
	/// One `.ksy` expression as an IR expression, or why it cannot be one.
	fn lower_expr(&mut self, ctx: &Ctx<'a>, path: &str, e: &KExpr) -> Result<Expr, String> {
		match e {
			KExpr::IntNum(n) => Ok(Expr::Lit(*n)),
			KExpr::Bool(b) => Ok(Expr::Lit(i128::from(*b))),
			KExpr::FloatNum(n) => Err(format!("`{n}` is a floating point number, and the IR's expressions are integers")),
			KExpr::Str(s) => Err(format!("`{s}` is text, and the IR's expressions are integers")),
			KExpr::InterpolatedStr(_) => {
				Err("an interpolated string, and the IR's expressions are integers".to_string())
			}
			KExpr::List(_) => Err("a list, and the IR's expressions are integers".to_string()),
			KExpr::EnumByLabel { enum_name, label, in_type } => {
				match self.enum_value(ctx, in_type, enum_name, label) {
					Some(v) => Ok(Expr::Lit(v)),
					None => Err(format!("`{enum_name}::{label}` names no enum member in scope")),
				}
			}
			KExpr::UnaryOp { op, operand } => {
				let inner = self.lower_expr(ctx, path, operand)?;
				match op {
					UnaryOp::Minus => Ok(Expr::Lit(0).sub(inner)),
					UnaryOp::Not => Ok(inner.negate()),
					UnaryOp::Invert => {
						Err("`~` is a bitwise complement, which the IR has no operator for".to_string())
					}
				}
			}
			KExpr::BinOp { left, op, right } => {
				let l = self.lower_expr(ctx, path, left)?;
				let r = self.lower_expr(ctx, path, right)?;
				Ok(match op {
					BinOp::Add => l.add(r),
					BinOp::Sub => l.sub(r),
					BinOp::Mult => l.mul(r),
					BinOp::Div => l.div(r),
					// Kaitai's `%` takes the sign of the divisor, which is
					// what `Expr::Mod` is.
					BinOp::Mod => l.modulo(r),
					BinOp::LShift => l.shl(r),
					BinOp::RShift => l.shr(r),
					BinOp::BitAnd => l.and(r),
					BinOp::BitOr => {
						return Err("`|` is a bitwise or, and the IR's `Or` is a value-or (the right side only when the left is zero), which is a different answer".to_string());
					}
					BinOp::BitXor => {
						return Err("`^` is a bitwise exclusive or, which the IR has no operator for".to_string());
					}
				})
			}
			KExpr::Compare { left, op, right } => self.lower_compare(ctx, path, left, *op, right),
			KExpr::BoolOp { op, values } => {
				let mut out: Option<Expr> = None;
				for value in values {
					let e = self.lower_expr(ctx, path, value)?;
					out = Some(match out {
						None => e,
						Some(acc) => match op {
							BoolOp::And => acc.both(e),
							BoolOp::Or => acc.either(e),
						},
					});
				}
				out.ok_or_else(|| "an empty boolean expression".to_string())
			}
			KExpr::IfExp { condition, if_true, if_false } => {
				let when = self.lower_expr(ctx, path, condition)?;
				let then = self.lower_expr(ctx, path, if_true)?;
				let otherwise = self.lower_expr(ctx, path, if_false)?;
				Ok(Expr::cond(when, then, otherwise))
			}
			KExpr::ByteSizeOfType(t) => match self.static_ksy_bits(ctx, t) {
				Some(bits) => Ok(Expr::Lit(i128::from(bits / 8))),
				None => Err(format!("`sizeof<{t}>` asks the size of a type whose size the file decides")),
			},
			KExpr::BitSizeOfType(t) => match self.static_ksy_bits(ctx, t) {
				Some(bits) => Ok(Expr::Lit(i128::from(bits))),
				None => Err(format!("`bitsizeof<{t}>` asks the size of a type whose size the file decides")),
			},
			KExpr::Call { func, args } => self.lower_call(ctx, path, func, args),
			KExpr::Name(_) | KExpr::Attribute { .. } | KExpr::Subscript { .. } | KExpr::CastToType { .. } => {
				self.lower_chain(ctx, path, e)
			}
		}
	}

	/// A comparison, with the one place a string is allowed: against a field
	/// whose bytes are what is really being compared.
	fn lower_compare(
		&mut self,
		ctx: &Ctx<'a>,
		path: &str,
		left: &KExpr,
		op: CmpOp,
		right: &KExpr,
	) -> Result<Expr, String> {
		let literal_bytes = |e: &KExpr| -> Option<Vec<u8>> {
			match e {
				KExpr::Str(s) => Some(s.as_bytes().to_vec()),
				KExpr::List(items) => byte_list(items),
				_ => None,
			}
		};
		let note_bytes = |this: &mut Self, bytes: &[u8], written: String| -> Result<Expr, String> {
			if bytes.len() > 16 {
				return Err(format!("`{written}` is {} bytes, more than a number holds", bytes.len()));
			}
			this.report.note(
				path.to_string(),
				written,
				"compared as those bytes read as one big-endian number, which is what the field it is compared with reads as".to_string(),
			);
			Ok(Expr::Lit(be_int(bytes)))
		};
		let l = match literal_bytes(left) {
			Some(bytes) => note_bytes(self, &bytes, left.to_string())?,
			None => self.lower_expr(ctx, path, left)?,
		};
		let r = match literal_bytes(right) {
			Some(bytes) => note_bytes(self, &bytes, right.to_string())?,
			None => self.lower_expr(ctx, path, right)?,
		};
		Ok(match op {
			CmpOp::Eq => l.equal_to(r),
			CmpOp::NotEq => l.not_equal(r),
			CmpOp::Lt => l.less_than(r),
			CmpOp::LtE => l.less_or_equal(r),
			CmpOp::Gt => l.greater_than(r),
			CmpOp::GtE => l.greater_or_equal(r),
		})
	}

	/// A method call. Kaitai's expression language has no functions of its own,
	/// so this is always a method on something.
	fn lower_call(&mut self, _ctx: &Ctx<'a>, _path: &str, func: &KExpr, args: &[KExpr]) -> Result<Expr, String> {
		match func {
			KExpr::Attribute { value, attr } if attr == "to_s" => Err(format!(
				"`{value}.to_s(...)` reads bytes as text, and the IR's expressions are integers"
			)),
			KExpr::Attribute { value, attr } => Err(format!(
				"`{value}.{attr}(...)` takes {} arguments and is not a method the IR has",
				args.len()
			)),
			other => Err(format!("`{other}(...)` is not a call the IR has")),
		}
	}

	/// A chain of names, indexes and methods, read left to right.
	fn lower_chain(&mut self, ctx: &Ctx<'a>, path: &str, e: &KExpr) -> Result<Expr, String> {
		let (base, segs) = flatten(e);
		let KExpr::Name(name) = base else {
			// Something read out of a value rather than out of a field: only a
			// cast could have come before it, and the cast is already dropped.
			let inner = self.lower_expr(ctx, path, base)?;
			return self.apply_segs(ctx, path, Acc::Done(inner), &segs);
		};
		match name.as_str() {
			"_io" => {
				let [Seg::Name(what)] = segs.as_slice() else {
					return Err("`_io` on its own is a stream, which is not a value".to_string());
				};
				match *what {
					"pos" => Ok(Expr::Pos),
					"size" => Ok(Expr::WindowSize),
					"eof" => Ok(Expr::Remaining.equal_to(Expr::Lit(0))),
					other => Err(format!("`_io.{other}` is not something the IR can ask a stream")),
				}
			}
			"_index" => {
				if segs.is_empty() {
					Ok(Expr::Idx)
				} else {
					Err("`_index` is a number, and nothing is read out of it".to_string())
				}
			}
			"_" => {
				let Some(Seg::Name(field)) = segs.first() else {
					return Err("`_` on its own is the element itself, which the IR has no name for".to_string());
				};
				let Some(elem) = ctx.elem else {
					return Err("`_` is used where there is no element to read".to_string());
				};
				if !visible(elem).iter().any(|n| n == field) {
					return Err(format!("`_.{field}` names no field of the element"));
				}
				self.apply_segs(ctx, path, Acc::Path(vec![(*field).to_string()]), &segs[1..])
			}
			"_root" => {
				let root = ctx.stack[0];
				let Some(Seg::Name(field)) = segs.first() else {
					return Err("`_root` on its own is a type, not a value".to_string());
				};
				if *field == "_io" {
					return Err(
						"`_root._io` is the whole file as a stream, and the IR has no expression for the file's own size or position"
							.to_string(),
					);
				}
				if !visible(root).iter().any(|n| n == field) {
					return Err(format!("`_root.{field}` names no field of the top-level type"));
				}
				for level in &ctx.stack[1..] {
					if visible(level).iter().any(|n| n == field) {
						return Err(format!(
							"`_root.{field}` is hidden by a field of the same name in an enclosing type"
						));
					}
				}
				self.apply_segs(ctx, path, Acc::Path(vec![(*field).to_string()]), &segs[1..])
			}
			"_parent" => self.lower_parent(ctx, path, &segs),
			_ => {
				if !self.in_scope(ctx, name) {
					return Err(format!("no field named `{name}` is in scope"));
				}
				if self.pending.iter().any(|n| n == name) {
					return Err(format!(
						"`{name}` is worked out after this field, and the IR reads only what comes before"
					));
				}
				self.apply_segs(ctx, path, Acc::Path(vec![name.clone()]), &segs)
			}
		}
	}

	/// `_parent.x`, and `_parent._parent.x` above that.
	///
	/// The IR's `Ref` climbs out of the structure it is written in, which lands
	/// where Kaitai's `_parent` lands as long as two things hold: nothing
	/// between hides the name, and every type that places this one has a field
	/// of that name. Both are checked, because a `_parent` that reaches
	/// somewhere else on some files and not on others is the kind of silent
	/// wrongness this converter exists to refuse.
	fn lower_parent(&mut self, ctx: &Ctx<'a>, path: &str, segs: &[Seg<'_>]) -> Result<Expr, String> {
		let mut depth = 1;
		let mut rest = segs;
		while let Some(Seg::Name("_parent")) = rest.first() {
			depth += 1;
			rest = &rest[1..];
		}
		let Some(Seg::Name(field)) = rest.first() else {
			return Err("`_parent` on its own is a type, not a value".to_string());
		};
		let here = ctx.ir_name();
		let Some(ancestors) = self.ancestors(&here, depth) else {
			return Err(format!(
				"`{}{field}` climbs past the types that place this one",
				"_parent.".repeat(depth)
			));
		};
		if ancestors.is_empty() {
			return Err(format!(
				"`{}{field}` climbs out of a type nothing places",
				"_parent.".repeat(depth)
			));
		}
		for name in &ancestors {
			let Some(cls) = self.class_named(name) else {
				return Err(format!("`{}{field}` climbs to a type that is not in the file", "_parent.".repeat(depth)));
			};
			if !visible(cls).iter().any(|n| n == field) {
				return Err(format!(
					"`{}{field}` reaches `{name}`, which has no field of that name",
					"_parent.".repeat(depth)
				));
			}
		}
		if visible(ctx.here()).iter().any(|n| n == field) {
			return Err(format!(
				"`{}{field}` is hidden by a field of the same name in this type",
				"_parent.".repeat(depth)
			));
		}
		self.apply_segs(ctx, path, Acc::Path(vec![(*field).to_string()]), &rest[1..])
	}

	/// The types that place this one, `depth` levels up.
	fn ancestors(&self, ir_name: &str, depth: usize) -> Option<Vec<String>> {
		let mut level: Vec<String> = vec![ir_name.to_string()];
		for _ in 0..depth {
			let mut next: Vec<String> = Vec::new();
			for name in &level {
				for parent in self.parents.get(name).into_iter().flatten() {
					if !next.contains(parent) {
						next.push(parent.clone());
					}
				}
			}
			if next.is_empty() {
				return Some(Vec::new());
			}
			level = next;
		}
		Some(level)
	}

	fn class_named(&self, ir_name: &str) -> Option<&'a ClassSpec> {
		for file in self.files {
			let rest = if file.prefix.is_empty() {
				Some(ir_name)
			} else if ir_name == file.prefix {
				Some("")
			} else {
				ir_name.strip_prefix(&format!("{}.", file.prefix))
			};
			let Some(rest) = rest else { continue };
			let names: Vec<String> =
				if rest.is_empty() { Vec::new() } else { rest.split('.').map(str::to_string).collect() };
			if let Some(found) = descend(&file.spec, &names) {
				return Some(found);
			}
		}
		None
	}

	fn apply_segs(
		&mut self,
		ctx: &Ctx<'a>,
		path: &str,
		mut acc: Acc,
		segs: &[Seg<'_>],
	) -> Result<Expr, String> {
		for seg in segs {
			acc = match seg {
				Seg::Index(index) => {
					let index = self.lower_expr(ctx, path, index)?;
					match acc {
						Acc::Path(array) => Acc::Elem { array, index, field: Vec::new() },
						_ => {
							return Err("an index into something that is not a list of the file's own".to_string());
						}
					}
				}
				Seg::Name(name) if is_method(name) => self.apply_method(ctx, acc, name)?,
				Seg::Name(name) => match acc {
					Acc::Path(mut p) => {
						p.push((*name).to_string());
						Acc::Path(p)
					}
					Acc::Elem { array, index, mut field } => {
						field.push((*name).to_string());
						Acc::Elem { array, index, field }
					}
					Acc::Done(_) => {
						return Err(format!("`.{name}` reads inside a value that has no fields"));
					}
				},
			};
		}
		Ok(finish(acc))
	}

	/// What a method on a field means in the IR.
	fn apply_method(&mut self, ctx: &Ctx<'a>, acc: Acc, name: &str) -> Result<Acc, String> {
		let single = match &acc {
			Acc::Path(p) if p.len() == 1 => Some(p[0].clone()),
			_ => None,
		};
		Ok(match name {
			// `.to_i` on a number or a boolean is the number it already is.
			"to_i" => {
				if let Some(field) = &single {
					if self.field_is_text(ctx, field) {
						return Err(format!(
							"`{field}.to_i` reads text as a number, which the IR does only where the field itself is declared as digits"
						));
					}
				}
				acc
			}
			"value" => acc,
			"size" | "length" => {
				let Some(field) = single else {
					return Err(format!("`.{name}` measures a field, and this is not one"));
				};
				Acc::Done(if self.field_is_list(ctx, &field) {
					Expr::len_of(&field)
				} else {
					Expr::size_of(&field)
				})
			}
			"first" | "last" => {
				let Acc::Path(array) = acc else {
					return Err(format!("`.{name}` takes an element of a list, and this is not one"));
				};
				if array.len() == 1 && !self.field_is_list(ctx, &array[0]) {
					return Err(format!(
						"`{}.{name}` reads an element of something that is not a list",
						array[0]
					));
				}
				let index = if name == "first" {
					Expr::Lit(0)
				} else if array.len() == 1 {
					Expr::len_of(&array[0]).sub(Expr::Lit(1))
				} else {
					return Err(format!(
						"`.{name}` takes the last element of a list one level in, which the IR cannot count"
					));
				};
				Acc::Elem { array, index, field: Vec::new() }
			}
			"to_s" => {
				return Err("`.to_s` reads a number as text, and the IR's expressions are integers".to_string());
			}
			other => return Err(format!("`.{other}` is not a method the IR has")),
		})
	}

	/// Whether a name is one of the things the type it is written in can read.
	fn in_scope(&self, ctx: &Ctx<'a>, name: &str) -> bool {
		if let Some(elem) = ctx.elem {
			if visible(elem).iter().any(|n| n == name) {
				return true;
			}
		}
		ctx.stack.iter().any(|cls| visible(cls).iter().any(|n| n == name))
	}

	fn find_attr(&self, ctx: &Ctx<'a>, name: &str) -> Option<&'a AttrSpec> {
		let mut classes: Vec<&'a ClassSpec> = ctx.stack.clone();
		if let Some(elem) = ctx.elem {
			classes.push(elem);
		}
		for cls in classes.iter().rev() {
			if let Some(attr) = cls.seq.iter().find(|a| a.name() == name) {
				return Some(attr);
			}
			for (n, instance) in &cls.instances {
				if n == name {
					if let InstanceSpec::Parse(p) = instance {
						return Some(&p.attr);
					}
					return None;
				}
			}
		}
		None
	}

	fn field_is_list(&self, ctx: &Ctx<'a>, name: &str) -> bool {
		self.find_attr(ctx, name).is_some_and(|a| !matches!(a.repeat, RepeatSpec::No))
	}

	fn field_is_text(&self, ctx: &Ctx<'a>, name: &str) -> bool {
		self.find_attr(ctx, name).is_some_and(|a| matches!(a.ty, TypeRef::Str { .. }))
	}

	/// How wide a `.ksy` type always is, in bits, read from the spec rather
	/// than from the IR so that asking does not lower anything.
	fn static_ksy_bits(&self, ctx: &Ctx<'a>, name: &TypeId) -> Option<u64> {
		let (_, cls) = self.resolve_type(ctx, name)?;
		let inner = self.ctx_for(&self.resolve_type(ctx, name)?.0, cls)?;
		let mut total = 0u64;
		for attr in &cls.seq {
			if !matches!(attr.repeat, RepeatSpec::No) || attr.if_expr.is_some() {
				return None;
			}
			total += self.static_attr_bits(&inner, attr)?;
		}
		Some(total)
	}

	fn static_attr_bits(&self, ctx: &Ctx<'a>, attr: &AttrSpec) -> Option<u64> {
		if let Some(bytes) = &attr.contents {
			return Some(bytes.len() as u64 * 8);
		}
		let fixed = |source: &ByteSource| match source {
			ByteSource::Limit(KExpr::IntNum(n)) => Some((*n).max(0) as u64 * 8),
			_ => None,
		};
		match &attr.ty {
			TypeRef::Bytes { source } | TypeRef::Str { source, .. } => fixed(source),
			TypeRef::Int { width, .. } | TypeRef::Float { width, .. } => Some(u64::from(*width) * 8),
			TypeRef::Bits { width, .. } => Some(u64::from(*width)),
			TypeRef::User { name, .. } => self.static_ksy_bits(ctx, name),
			TypeRef::UserFromBytes { source, .. } => fixed(source),
			TypeRef::Switch(_) => None,
		}
	}
}

fn finish(acc: Acc) -> Expr {
	match acc {
		Acc::Path(p) => {
			if p.len() == 1 {
				Expr::Ref(p[0].as_str().into())
			} else {
				Expr::Within(p.into())
			}
		}
		Acc::Elem { array, index, field } => {
			if array.len() == 1 {
				Expr::Elem { array: array[0].as_str().into(), index: Box::new(index), field: field.into() }
			} else {
				Expr::ElemWithin { path: array.into(), index: Box::new(index), field: field.into() }
			}
		}
		Acc::Done(e) => e,
	}
}

/// A field the IR could not say, and what to put there instead.
struct Gap {
	reason: String,
	fallback: Option<Ty>,
	/// Whether the fields after this one in the same type can still be placed.
	placeable: bool,
}

impl Gap {
	/// The type was expressed and something round it was not, so the field
	/// keeps what did lower.
	fn keeping(reason: String, ty: Ty) -> Gap {
		Gap { reason, fallback: Some(ty), placeable: true }
	}
	/// The field is bytes of a known length.
	fn bytes(reason: String, len: Expr) -> Gap {
		Gap { reason, fallback: Some(Ty::Bytes(len)), placeable: true }
	}
	/// The field takes whatever room is left, so nothing after it is placed.
	fn unplaceable(reason: String) -> Gap {
		Gap { reason, fallback: Some(Ty::Bytes(Expr::Remaining)), placeable: false }
	}
	/// A reference the IR cannot make; the field is bytes only if the caller
	/// knows a length, which for a bare type reference it does not.
	fn sized(reason: String) -> Gap {
		Gap { reason, fallback: Some(Ty::Bytes(Expr::Remaining)), placeable: false }
	}
	/// Nothing is emitted at all, which only an instance can afford.
	fn dropped(reason: String) -> Gap {
		Gap { reason, fallback: None, placeable: true }
	}
}

/// Walk down `names` from `cls`, following the `types` map.
fn descend<'a>(cls: &'a ClassSpec, names: &[String]) -> Option<&'a ClassSpec> {
	let mut here = cls;
	for name in names {
		here = &here.types.iter().find(|(n, _)| n == name)?.1;
	}
	Some(here)
}

fn join_name(prefix: &str, names: &[String]) -> String {
	let tail = names.join(".");
	if prefix.is_empty() { tail } else { format!("{prefix}.{tail}") }
}

/// What the structure is called in the type column: its own name, not the
/// whole path, since the path is already the key it is filed under.
fn struct_name(ir_name: &str, cls: &ClassSpec) -> String {
	if ir_name.is_empty() {
		return cls.meta.id.clone().unwrap_or_else(|| "ksy".to_string());
	}
	ir_name.rsplit('.').next().unwrap_or(ir_name).to_string()
}

fn class_doc(cls: &ClassSpec) -> Option<String> {
	doc_text(&cls.doc)
}

fn attr_doc(attr: &AttrSpec) -> Option<String> {
	doc_text(&attr.doc)
}

/// The prose a `doc` holds, with any `doc-ref` links after it.
fn doc_text(doc: &super::spec::DocSpec) -> Option<String> {
	let mut parts: Vec<String> = Vec::new();
	if let Some(summary) = &doc.summary {
		parts.push(summary.trim().to_string());
	}
	for reference in &doc.refs {
		match reference {
			super::spec::DocRef::Url { url, text } => {
				parts.push(if text == "Source" { url.clone() } else { format!("{text}: {url}") });
			}
			super::spec::DocRef::Text(text) => parts.push(text.clone()),
		}
	}
	if parts.is_empty() { None } else { Some(parts.join("\n")) }
}

/// What the report quotes for a field: the `type:` it was written with, or the
/// key that stood in for one.
fn source_of(attr: &AttrSpec) -> String {
	match &attr.ty {
		TypeRef::Bytes { source } => match (&attr.contents, source) {
			(Some(bytes), _) => format!("contents: {} bytes", bytes.len()),
			(None, ByteSource::Limit(e)) => format!("size: {e}"),
			(None, ByteSource::Eos) => "size-eos: true".to_string(),
			(None, ByteSource::Terminated) => "terminator".to_string(),
		},
		TypeRef::Switch(switch) => format!("type: switch-on {}", switch.on),
		other => format!("type: {other}"),
	}
}

fn instance_source(instance: &InstanceSpec) -> String {
	match instance {
		InstanceSpec::Value(v) => format!("value: {}", v.value),
		InstanceSpec::Parse(p) => match &p.pos {
			Some(pos) => format!("pos: {pos}"),
			None => source_of(&p.attr),
		},
	}
}

fn process_name(process: &ProcessSpec) -> String {
	match process {
		ProcessSpec::Zlib => "zlib".to_string(),
		ProcessSpec::Xor(_) => "xor".to_string(),
		ProcessSpec::Rotate { left: true, .. } => "rol".to_string(),
		ProcessSpec::Rotate { left: false, .. } => "ror".to_string(),
		ProcessSpec::Custom { name, .. } => name.join("."),
	}
}

/// Which of a type's value instances can be written out before its `seq`: the
/// ones whose value depends on nothing the file holds, only on parameters,
/// literals and each other. A `Computed` field takes no bytes, so moving one
/// earlier changes only which rows can read it.
fn early_instances(cls: &ClassSpec) -> Vec<String> {
	let params: Vec<String> = cls.params.iter().map(ParamSpec::name).collect();
	let mut early: Vec<String> = Vec::new();
	loop {
		let mut added = false;
		for (name, instance) in &cls.instances {
			if early.contains(name) {
				continue;
			}
			let InstanceSpec::Value(value) = instance else { continue };
			let mut ok = true;
			value.value.walk(&mut |node| {
				if let KExpr::Name(n) = node {
					if !params.contains(n) && !early.contains(n) && !n.starts_with('_') {
						ok = false;
					}
				}
				if matches!(node, KExpr::Attribute { .. } | KExpr::Subscript { .. }) {
					ok = false;
				}
			});
			if let Some(cond) = &value.if_expr {
				let _ = cond;
				ok = false;
			}
			if ok {
				early.push(name.clone());
				added = true;
			}
		}
		if !added {
			return early;
		}
	}
}

/// How many bits a field takes out of the current byte, where it is one of the
/// bit-wide fields at all. A repeat or a condition means the answer depends on
/// the file, and then nothing can be said about where the next field starts.
fn bit_width(attr: &AttrSpec) -> Option<u32> {
	if !matches!(attr.repeat, RepeatSpec::No) || attr.if_expr.is_some() {
		return None;
	}
	match &attr.ty {
		TypeRef::Bits { width, .. } => Some(*width),
		_ => None,
	}
}

/// A list of expressions that is really a list of bytes.
fn byte_list(items: &[KExpr]) -> Option<Vec<u8>> {
	let mut out = Vec::new();
	for item in items {
		match item {
			KExpr::IntNum(n) if (0..256).contains(n) => out.push(*n as u8),
			_ => return None,
		}
	}
	Some(out)
}

fn be_int(bytes: &[u8]) -> i128 {
	bytes.iter().fold(0i128, |acc, b| (acc << 8) | i128::from(*b))
}

/// Whether a value instance holds something that is not a number, which is
/// what the IR's computed fields are.
fn not_an_integer(e: &KExpr) -> Option<String> {
	let mut found = None;
	e.walk(&mut |node| {
		if found.is_some() {
			return;
		}
		found = match node {
			KExpr::FloatNum(_) => Some("the value is a floating point number".to_string()),
			KExpr::InterpolatedStr(_) => Some("the value is an interpolated string".to_string()),
			_ => None,
		};
	});
	found
}

/// The encodings the IR can read, by every spelling the corpus uses.
fn map_encoding(name: &str) -> Option<Encoding> {
	let flat: String =
		name.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_lowercase();
	Some(match flat.as_str() {
		"utf8" => Encoding::Utf8,
		"ascii" | "usascii" => Encoding::Ascii,
		"iso88591" | "latin1" | "8859" => Encoding::Latin1,
		"ibm437" | "cp437" | "oem437" | "dos437" => Encoding::Cp437,
		"utf16le" => Encoding::Utf16(Endian::Little),
		"utf16be" => Encoding::Utf16(Endian::Big),
		// Kaitai's plain `UTF-16` is the one a byte-order mark decides.
		"utf16" => Encoding::Bom { fallback: Box::new(Encoding::Utf16(Endian::Big)) },
		_ => return None,
	})
}

/// The types a `type:` key refers to, including every case of a switch.
fn referenced_types(ty: &TypeRef) -> Vec<TypeId> {
	match ty {
		TypeRef::User { name, .. } | TypeRef::UserFromBytes { name, .. } => vec![name.clone()],
		TypeRef::Switch(switch) => {
			switch.cases.iter().flat_map(|c| referenced_types(&c.ty)).collect()
		}
		_ => Vec::new(),
	}
}

/// Split a `-webide-representation` into `(text before, field, suffix)` parts.
fn parse_representation(text: &str) -> Result<Vec<(String, String, String)>, String> {
	let mut parts = Vec::new();
	let mut word = String::new();
	let mut rest = text;
	while let Some(open) = rest.find('{') {
		word.push_str(&rest[..open]);
		let after = &rest[open + 1..];
		let Some(close) = after.find('}') else {
			return Err("the representation has a `{` with no `}` after it".to_string());
		};
		let inside = &after[..close];
		let (field, suffix) = match inside.split_once(':') {
			Some((f, s)) => (f.to_string(), s.to_string()),
			None => (inside.to_string(), String::new()),
		};
		parts.push((word.trim().to_string(), field, suffix));
		word = String::new();
		rest = &after[close + 1..];
	}
	if parts.is_empty() {
		return Err("the representation is a fixed label with no field in it, so the type reads as its own name".to_string());
	}
	Ok(parts)
}

/// How wide a type always is, in bits, where that is a fixed number.
pub fn static_bits(ty: &Ty) -> Option<u64> {
	Some(match ty {
		Ty::UInt { bits, .. } | Ty::Int { bits, .. } | Ty::SignMagnitude { bits, .. } => {
			u64::from(*bits)
		}
		Ty::F16(_) | Ty::BF16(_) => 16,
		Ty::F32(_) => 32,
		Ty::F64(_) => 64,
		Ty::F80(_) => 80,
		Ty::F8 { .. } => 8,
		Ty::Fixed { bits, .. } => u64::from(*bits),
		Ty::Magic(b) => b.len() as u64 * 8,
		Ty::Bytes(Expr::Lit(n)) => (*n).max(0) as u64 * 8,
		Ty::Str { len: StrLen::Fixed(Expr::Lit(n)) | StrLen::Padded { size: Expr::Lit(n), .. }, .. } => {
			(*n).max(0) as u64 * 8
		}
		Ty::Computed(_) | Ty::ComputedText(_) => 0,
		Ty::Enum { inner, .. } | Ty::Flags { inner, .. } | Ty::Nullable { inner, .. } => {
			static_bits(inner)?
		}
		Ty::Array { elem, count: Expr::Lit(n) } => static_bits(elem)? * (*n).max(0) as u64,
		Ty::Struct(s) => {
			let mut total = 0;
			for field in &s.fields {
				total += static_bits(&field.ty)?;
			}
			total
		}
		_ => return None,
	})
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::ksy::imports::NoImports;
	use crate::template_text;

	pub(super) fn convert_text(text: &str) -> Converted {
		convert(text, &NoImports).unwrap_or_else(|e| panic!("{e}"))
	}

	pub(super) fn rendered(text: &str) -> String {
		template_text::render(&convert_text(text).template)
	}

	#[test]
	fn a_representation_splits_into_words_and_fields() {
		assert_eq!(
			parse_representation("{id}").unwrap(),
			vec![(String::new(), "id".to_string(), String::new())]
		);
		assert_eq!(
			parse_representation("seq {seq:dec} of {total}").unwrap(),
			vec![
				("seq".to_string(), "seq".to_string(), "dec".to_string()),
				("of".to_string(), "total".to_string(), String::new()),
			]
		);
		assert!(parse_representation("plain text").is_err());
	}

	#[test]
	fn a_small_format_reads_as_the_ir_writes_it() {
		let text = "\
meta:
  id: demo
  endian: le
  encoding: ASCII
doc: A little format.
seq:
  - id: magic
    contents: DEMO
  - id: version
    type: u2
    doc: Which revision this is.
  - id: len_name
    type: u1
  - id: name
    type: str
    size: len_name
  - id: flags
    type: b3
  - id: rest
    size-eos: true
instances:
  total:
    value: len_name + 4
";
		let out = rendered(text);
		assert!(out.contains("// A little format."), "{out}");
		assert!(out.contains("magic: magic \"DEMO\""), "{out}");
		assert!(out.contains("// Which revision this is."), "{out}");
		assert!(out.contains("version: u16le"), "{out}");
		assert!(out.contains("name: text[len_name] ascii"), "{out}");
		assert!(out.contains("flags: u3be"), "{out}");
		assert!(out.contains("rest: bytes[remaining]"), "{out}");
		assert!(out.contains("total: computed len_name + 4"), "{out}");
	}

	#[test]
	fn a_fixed_width_structure_can_be_measured() {
		let ty = Ty::structure("s", vec![("a", Ty::u8()), ("b", Ty::u32(Endian::Little))]);
		assert_eq!(static_bits(&ty), Some(40));
		assert_eq!(static_bits(&Ty::Bytes(Expr::Remaining)), None);
	}
}
