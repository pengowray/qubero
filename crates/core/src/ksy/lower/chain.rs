//! Chains of names, and the `.ksy` types they run through.
//!
//! A chain is what a `.ksy` expression writes to reach a value: a name, then
//! any number of steps after it. A step is a field (`a.b`), an element of a
//! list (`a.b[2]`) or a cast (`.as<t>`), and the chain can start at one of the
//! names the language reserves: `_root.x` starts at the top-level type,
//! `_parent.y` climbs out to whatever placed this one, `_` is the element a
//! `repeat-until` is testing, `_io` is the stream and `_index` the position in
//! a repeat.
//!
//! The awkward part is that some names are also methods. `size`, `length`,
//! `first`, `value` and the rest read something *about* the value rather than
//! a field inside it, but a type is free to have a field of that name, and
//! then the field wins. Telling the two apart needs the `.ksy` type the chain
//! has reached, which is what [`Land`] carries: it follows the spec along the
//! chain, a field of a user type landing in that type and a cast landing in
//! the type it names, and it goes to `None` wherever the spec stops saying, as
//! it does for a parameter, a number, or whatever a method answered with. A
//! name that is also a method is read as the field only where [`Land`] says
//! the type has one.
//!
//! [`Acc`] is the other half: what the chain has reached so far, kept in a
//! form more names can still be added to until [`finish`] turns it into an IR
//! [`Expr`].

use crate::template::Expr;

use crate::ksy::expr::{Expr as KExpr, TypeId};
use crate::ksy::spec::{AttrSpec, ClassSpec, InstanceSpec, RepeatSpec, TypeRef, ValueInstanceSpec};
use super::{Ctx, Lower, descend, visible};

/// One step of a chain like `a.b[2].c`.
enum Seg<'a> {
	Name(&'a str),
	Index(&'a KExpr),
	/// `.as<t>`: what the value so far is declared to be.
	Cast(&'a TypeId),
}

/// The `.ksy` type a chain of names has reached, by IR name and spec, when
/// the spec says: a field of a user type lands in that type, and a cast lands
/// in the type it names. None where the spec is silent, as it is for a
/// parameter, a number, or what a method answers with.
type Land<'a> = Option<(String, &'a ClassSpec)>;

/// What one name inside a type reaches: a field read from the file, or one
/// worked out from others.
enum Member<'a> {
	Attr(&'a AttrSpec),
	Value(&'a ValueInstanceSpec),
}

/// A value instance that names another that names it back would be followed
/// for ever; this is where the following stops.
const LANDING_DEPTH: usize = 8;

/// What a chain has read so far.
enum Acc {
	/// A field, and names down into it.
	Path(Vec<String>),
	/// One element of a list, and names down into that.
	Elem { array: Vec<String>, index: Expr, field: Vec<String> },
	/// A value nothing more can be read out of but a method.
	Done(Expr),
}

/// Split `a.b[2].c` into its base and the steps after it. A cast is kept as
/// a step of its own: the IR reads the value either way, but `.as<t>` says
/// which type the names after it are fields of.
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
		KExpr::CastToType { value, type_name } => {
			let (base, mut segs) = flatten(value);
			segs.push(Seg::Cast(type_name));
			(base, segs)
		}
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
	/// A chain of names, indexes and methods, read left to right.
	pub(super) fn lower_chain(&mut self, ctx: &Ctx<'a>, path: &str, e: &KExpr) -> Result<Expr, String> {
		let (base, segs) = flatten(e);
		let KExpr::Name(name) = base else {
			// Something read out of a value rather than out of a field: only a
			// cast could have come before it, and the cast is already dropped.
			let inner = self.lower_expr(ctx, path, base)?;
			return self.apply_segs(ctx, path, Acc::Done(inner), None, &segs);
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
					other => Err(format!("`_io.{other}` is not something a template can ask of a stream")),
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
					return Err("`_` on its own is the whole element, which a template expression cannot name".to_string());
				};
				let Some(elem) = ctx.elem else {
					return Err("`_` is used where there is no element to read".to_string());
				};
				if !visible(elem).iter().any(|n| n == field) {
					return Err(format!("`_.{field}` names no field of the element"));
				}
				self.apply_segs(ctx, path, Acc::Path(vec![(*field).to_string()]), None, &segs[1..])
			}
			"_root" => {
				let root = ctx.stack[0];
				let Some(Seg::Name(field)) = segs.first() else {
					return Err("`_root` on its own is a type, not a value".to_string());
				};
				if *field == "_io" {
					return Err(
						"`_root._io` is the whole file as a stream; a template has no expression for the file's own size or position"
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
				let root: Land<'a> = Some((ctx.names[0].clone(), root));
				let land = self.member_at(&root, field, 0).and_then(|(_, next)| next);
				self.apply_segs(ctx, path, Acc::Path(vec![(*field).to_string()]), land, &segs[1..])
			}
			"_parent" => self.lower_parent(ctx, path, &segs),
			_ => {
				if !self.in_scope(ctx, name) {
					return Err(format!("no field named `{name}` is in scope"));
				}
				if self.pending.iter().any(|n| n == name) {
					return Err(format!(
						"`{name}` is worked out after this field; a template reads only what comes before"
					));
				}
				let land = match self.find_member(ctx, name) {
					Some(Member::Attr(a)) => self.class_of_attr(ctx, a),
					Some(Member::Value(v)) => self.landing(ctx, &v.value, 1).1,
					None => None,
				};
				// A name read out of a value instance that stands for a field
				// is read out of that field: `name_as_info.value`, where
				// `name_as_info` is `_root.pool[i].entry`, is the `value`
				// field of that entry. The instance itself is a number in the
				// IR, and nothing can be read out of a number.
				if let (Some(Member::Value(v)), Some(Seg::Name(first))) = (Self::member_of(ctx.here(), name), segs.first()) {
					if self.member_at(&land, first, 0).is_some() {
						let inner = self.lower_expr(ctx, path, &v.value)?;
						if let Some(acc) = unfinish(inner) {
							return self.apply_segs(ctx, path, acc, land, &segs);
						}
					}
				}
				self.apply_segs(ctx, path, Acc::Path(vec![name.clone()]), land, &segs)
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
		self.apply_segs(ctx, path, Acc::Path(vec![(*field).to_string()]), None, &rest[1..])
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

	/// `land` is the type `acc` is so far, when the spec says, and it decides
	/// what a name that is also a method means: `x.value` reads the field
	/// called `value` of a type that has one, and is the value itself of one
	/// that has not.
	fn apply_segs(
		&mut self,
		ctx: &Ctx<'a>,
		path: &str,
		mut acc: Acc,
		mut land: Land<'a>,
		segs: &[Seg<'_>],
	) -> Result<Expr, String> {
		for seg in segs {
			acc = match seg {
				Seg::Cast(ty) => {
					land = self.resolve_type(ctx, ty);
					acc
				}
				Seg::Index(index) => {
					let index = self.lower_expr(ctx, path, index)?;
					match acc {
						Acc::Path(array) => Acc::Elem { array, index, field: Vec::new() },
						_ => {
							return Err("an index into something that is not a list of the file's own".to_string());
						}
					}
				}
				Seg::Name(name) if is_method(name) && self.member_at(&land, name, 0).is_none() => {
					land = None;
					self.apply_method(ctx, acc, name)?
				}
				Seg::Name(name) => match acc {
					Acc::Path(mut p) => {
						land = self.member_at(&land, name, 0).and_then(|(_, next)| next);
						p.push((*name).to_string());
						Acc::Path(p)
					}
					Acc::Elem { array, index, mut field } => {
						land = self.member_at(&land, name, 0).and_then(|(_, next)| next);
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

	/// The type a field is declared as, when it is a user type.
	fn class_of_attr(&self, ctx: &Ctx<'a>, attr: &'a AttrSpec) -> Land<'a> {
		match &attr.ty {
			TypeRef::User { name, .. } | TypeRef::UserFromBytes { name, .. } => self.resolve_type(ctx, name),
			_ => None,
		}
	}

	/// The member of `cls` called `name`.
	fn member_of(cls: &'a ClassSpec, name: &str) -> Option<Member<'a>> {
		if let Some(attr) = cls.seq.iter().find(|a| a.name() == name) {
			return Some(Member::Attr(attr));
		}
		cls.instances.iter().find(|(n, _)| n == name).map(|(_, instance)| match instance {
			InstanceSpec::Parse(p) => Member::Attr(&p.attr),
			InstanceSpec::Value(v) => Member::Value(v),
		})
	}

	/// The member of the type at `land` called `name`, and the field it ends
	/// on and the type it lands in. A value instance is followed to where its
	/// own expression lands, so `size.value` on a varint type is the number
	/// the varint's `value` instance works out.
	fn member_at(&self, land: &Land<'a>, name: &str, depth: usize) -> Option<(Option<&'a AttrSpec>, Land<'a>)> {
		let (ir, cls) = land.as_ref()?;
		let member = Self::member_of(cls, name)?;
		let inner = self.ctx_for(ir, cls);
		Some(match (member, inner) {
			(Member::Attr(attr), Some(inner)) => (Some(attr), self.class_of_attr(&inner, attr)),
			(Member::Attr(attr), None) => (Some(attr), None),
			(Member::Value(v), Some(inner)) => self.landing(&inner, &v.value, depth + 1),
			(Member::Value(_), None) => (None, None),
		})
	}

	/// The field a chain of names ends on, and the type it lands in, as far
	/// as the spec says. What decides whether an instance is text: one whose
	/// value is `_root.pool[i].entry.as<utf8_info>.value` ends on the `value`
	/// field of `utf8_info`, and if that is a `str` the instance is one too.
	pub(super) fn landing(&self, ctx: &Ctx<'a>, e: &KExpr, depth: usize) -> (Option<&'a AttrSpec>, Land<'a>) {
		if depth > LANDING_DEPTH {
			return (None, None);
		}
		let (base, segs) = flatten(e);
		let KExpr::Name(name) = base else { return (None, None) };
		let (mut attr, mut land): (Option<&'a AttrSpec>, Land<'a>) = match name.as_str() {
			"_root" => (None, Some((ctx.names[0].clone(), ctx.stack[0]))),
			"_" | "_parent" | "_io" | "_index" | "_sizeof" => return (None, None),
			plain => match self.find_member(ctx, plain) {
				Some(Member::Attr(a)) => (Some(a), self.class_of_attr(ctx, a)),
				Some(Member::Value(v)) => self.landing(ctx, &v.value, depth + 1),
				None => return (None, None),
			},
		};
		for seg in &segs {
			match seg {
				Seg::Cast(ty) => land = self.resolve_type(ctx, ty),
				// One element of a list is what the list's field is declared as.
				Seg::Index(_) => {}
				Seg::Name(n) => {
					(attr, land) = match self.member_at(&land, n, depth) {
						Some(found) => found,
						None => (None, None),
					};
				}
			}
		}
		(attr, land)
	}

	/// `find_attr`, but finding a value instance as well.
	fn find_member(&self, ctx: &Ctx<'a>, name: &str) -> Option<Member<'a>> {
		let mut classes: Vec<&'a ClassSpec> = ctx.stack.clone();
		if let Some(elem) = ctx.elem {
			classes.push(elem);
		}
		classes.iter().rev().find_map(|cls| Self::member_of(cls, name))
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
							"`{field}.to_i` reads text as a number, which a template does only where the field itself is declared as digits"
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
						"`.{name}` takes the last element of a list one level in, which a template cannot count"
					));
				};
				Acc::Elem { array, index, field: Vec::new() }
			}
			"to_s" => {
				return Err("`.to_s` reads a number as text; template expressions are integers only".to_string());
			}
			other => return Err(format!("`.{other}` is not a method templates have")),
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
}

/// The chain an IR path expression came from, so more names can go on the end
/// of it. None for anything that is not a path.
fn unfinish(e: Expr) -> Option<Acc> {
	let strings = |p: &[String]| p.to_vec();
	Some(match e {
		Expr::Ref(name) => Acc::Path(vec![name.to_string()]),
		Expr::Within(p) => Acc::Path(strings(&p)),
		Expr::Elem { array, index, field } => {
			Acc::Elem { array: vec![array.to_string()], index: *index, field: strings(&field) }
		}
		Expr::ElemWithin { path, index, field } => {
			Acc::Elem { array: strings(&path), index: *index, field: strings(&field) }
		}
		_ => return None,
	})
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
