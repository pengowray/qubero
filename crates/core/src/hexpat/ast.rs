//! The declarative half of the pattern language as a tree.
//!
//! Every node carries the [`Pos`] of the token it started at, so that a gap in
//! the conversion report can name a line and column in the pattern the user
//! pasted.
//!
//! The imperative half is not modelled. A `fn` body, a statement `while` or
//! `for`, an assignment, a `return`, a `break` or a `try` is parsed with the
//! same grammar as everything else and then kept as a [`Statement`]: a coarse
//! [`StatementKind`], the source range it covered and that range's text. That
//! is enough for lowering to report it as a gap and show what it was, and it
//! means a construct we do not implement is still *checked*, rather than
//! skipped by counting braces.

use std::sync::Arc;

use super::expr::Expr;
use super::lexer::{Pos, Pragma, ValueType};

/// One parsed file: a pattern, or an include it pulled in.
#[derive(Debug, Clone, PartialEq)]
pub struct Program {
	/// The file this came from, as the resolver named it.
	pub file: String,
	pub decls: Vec<Decl>,
	pub pragmas: Vec<Pragma>,
	/// Every `/*! */` comment in the file, which documents the file itself.
	pub docs: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Endian {
	Little,
	Big,
}

/// A type as it is written at a use site.
#[derive(Debug, Clone, PartialEq)]
pub struct TypeRef {
	/// A `le` or `be` prefix, which overrides the file's `#pragma endian`.
	pub endian: Option<Endian>,
	/// Written `ref T`, which only a function parameter may be.
	pub reference: bool,
	pub kind: TypeKind,
	pub pos: Pos,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TypeKind {
	Builtin(ValueType),
	/// A declared type, `::`-joined, with the arguments of its template list.
	Named { path: String, args: Vec<TemplateArg> },
}

#[derive(Debug, Clone, PartialEq)]
pub enum TemplateArg {
	Type(TypeRef),
	Value(Expr),
}

/// One parameter of a `struct S<T, auto N>` head.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateParam {
	pub name: String,
	/// `T` is a type parameter; `auto N` is a value parameter.
	pub is_type: bool,
	pub pos: Pos,
}

/// One `[[name(args)]]` entry, with `hex::visualize` kept as written.
#[derive(Debug, Clone, PartialEq)]
pub struct Attribute {
	pub path: String,
	pub args: Vec<Expr>,
	pub pos: Pos,
}

/// How many elements an array has.
#[derive(Debug, Clone, PartialEq)]
pub enum ArraySize {
	/// `T x[]`, which reads until the enclosing window ends.
	Unbounded,
	/// `T x[n]`.
	Count(Expr),
	/// `T x[while(c)]`. The condition is checked before each element.
	While(Expr),
}

/// A statement the converter does not model, kept whole so it can be reported.
#[derive(Debug, Clone, PartialEq)]
pub struct Statement {
	pub kind: StatementKind,
	pub pos: Pos,
	/// Byte range of the statement in its own file, `[start, end)`.
	pub span: (u32, u32),
	/// The source text of that range.
	pub text: String,
	/// The parts of an assignment, for the few shapes the lowering can say
	/// exactly: `$ += e` is bytes skipped, and a global assigned once at the
	/// top level is a value worked out before anything is read. Everything
	/// else is still only reported, but reading the pieces beats matching the
	/// source text, which is what the lowering used to do.
	pub assign: Option<Assign>,
	/// The two halves of an `if` statement, kept so that a top-level `if`
	/// whose blocks only place fields can become one `When` per block.
	pub branches: Option<Box<Branches>>,
	/// The call a `Call` statement makes, so a call written inside a
	/// top-level `if` is read the same way as one written outside it.
	pub call: Option<Box<(String, Vec<Expr>)>>,
	/// The variable a `Local` statement declares. A top-level `if` may place a
	/// field inside it, and a placement arrives here rather than as a
	/// [`Decl::Placement`] because the block is parsed as statements.
	pub decl: Option<Box<Field>>,
}

/// The parts of an `if` statement.
#[derive(Debug, Clone, PartialEq)]
pub struct Branches {
	pub cond: Expr,
	pub then: Vec<Statement>,
	pub otherwise: Vec<Statement>,
}

/// The pieces of `x = e`, `$ += e`, `a.b = e`.
#[derive(Debug, Clone, PartialEq)]
pub struct Assign {
	pub target: AssignTarget,
	/// `None` for `=`; the operator of a compound assignment otherwise.
	pub op: Option<crate::hexpat::expr::BinOp>,
	pub value: Expr,
}

/// What an assignment writes to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssignTarget {
	/// `$ = e`, which moves the cursor.
	Dollar,
	/// A bare name: a local, or a global the pattern fills in.
	Name(String),
	/// A path, an element, anything else on the left of the `=`.
	Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatementKind {
	/// `x = e`, `$ = e`, `a.b = e`, `x += e`.
	Assign,
	/// A `while (c) { .. }` statement, not a `[while(c)]` array.
	While,
	For,
	Return,
	Break,
	Continue,
	/// A call written as a statement whose value is discarded.
	Call,
	/// A local variable declared inside a function body.
	Local,
	If,
	Match,
	Try,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Decl {
	/// `#include <path>`, resolved and parsed.
	Include { path: String, program: Option<Arc<Program>>, pos: Pos },
	Import(Import),
	Using(UsingDecl),
	Struct(StructDef),
	Enum(EnumDef),
	Bitfield(BitfieldDef),
	Function(FunctionDef),
	Namespace(NamespaceDecl),
	/// A top-level variable: `T x @ addr;`, `T x in;`, `T x out;`, `T x;`.
	Placement(Placement),
	/// A call written at the top level, `std::assert(..)` and friends.
	Call { path: String, args: Vec<Expr>, pos: Pos },
	Statement(Statement),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Import {
	/// The path with `.` turned into `/`, the way the resolver wants it.
	pub path: String,
	/// `import x as y`, or the required name of an `import * from x as y`.
	pub alias: Option<String>,
	/// Written `import * from`, which names the whole file as one type.
	pub all: bool,
	/// The parsed file, unless this was an `import *`, which is resolved late.
	pub program: Option<Arc<Program>>,
	pub pos: Pos,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UsingDecl {
	pub name: String,
	pub template: Vec<TemplateParam>,
	/// `None` is a forward declaration, `using X;`.
	pub ty: Option<TypeRef>,
	pub attrs: Vec<Attribute>,
	pub doc: Option<String>,
	pub pos: Pos,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StructDef {
	pub name: String,
	/// A `union`, whose members all start at the same offset.
	pub union: bool,
	pub template: Vec<TemplateParam>,
	/// `struct S : A, B`, whose members are read before this one's.
	pub inherits: Vec<TypeRef>,
	pub members: Vec<Member>,
	pub attrs: Vec<Attribute>,
	pub doc: Option<String>,
	pub pos: Pos,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EnumDef {
	pub name: String,
	/// The underlying type, which may not carry an endian prefix.
	pub ty: TypeRef,
	pub entries: Vec<EnumEntry>,
	pub attrs: Vec<Attribute>,
	pub doc: Option<String>,
	pub pos: Pos,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EnumEntry {
	pub name: String,
	/// `None` continues from the entry before, as C does.
	pub value: Option<Expr>,
	/// The end of an `A = 0x10 ... 0x1F` range.
	pub upto: Option<Expr>,
	pub pos: Pos,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BitfieldDef {
	pub name: String,
	pub template: Vec<TemplateParam>,
	pub entries: Vec<BitEntry>,
	pub attrs: Vec<Attribute>,
	pub doc: Option<String>,
	pub pos: Pos,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BitSign {
	/// Written plain, or with the `unsigned` keyword.
	Unsigned,
	Signed,
	/// `padding : n`, which is read and not shown.
	Padding,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BitEntry {
	/// `a : 3`, `unsigned a : 3`, `signed a : 3`, `padding : 3`.
	Bits { name: String, sign: BitSign, width: Expr, attrs: Vec<Attribute>, doc: Option<String>, pos: Pos },
	/// `bool f : 1`, `E e : 2`, a nested `B b;`, or an array `B b[n]`.
	Typed(Field),
	If { cond: Expr, then: Vec<BitEntry>, otherwise: Vec<BitEntry>, pos: Pos },
	Match { scrutinee: Vec<Expr>, arms: Vec<MatchArm<BitEntry>>, pos: Pos },
	Try { body: Vec<BitEntry>, catch: Vec<BitEntry>, pos: Pos },
	Call { path: String, args: Vec<Expr>, pos: Pos },
	Statement(Statement),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Member {
	Field(Field),
	/// `padding[n]`, whose bytes are read and not shown.
	Padding { size: ArraySize, attrs: Vec<Attribute>, pos: Pos },
	If { cond: Expr, then: Vec<Member>, otherwise: Vec<Member>, pos: Pos },
	Match { scrutinee: Vec<Expr>, arms: Vec<MatchArm<Member>>, pos: Pos },
	Try { body: Vec<Member>, catch: Vec<Member>, pos: Pos },
	Call { path: String, args: Vec<Expr>, pos: Pos },
	Statement(Statement),
}

/// One arm of a `match`.
///
/// `patterns[i]` matches `scrutinee[i]`; there is one entry per scrutinee and
/// the parser rejects an arm whose arity does not match.
#[derive(Debug, Clone, PartialEq)]
pub struct MatchArm<T> {
	pub patterns: Vec<CasePattern>,
	pub body: Vec<T>,
	pub pos: Pos,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CasePattern {
	/// `_`, which matches anything.
	Wildcard,
	/// One or more alternatives joined by `|`.
	Values(Vec<CaseValue>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum CaseValue {
	One(Expr),
	/// `a ... b`, inclusive at both ends.
	Range(Expr, Expr),
}

/// What a field is, beyond its type and name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKind {
	/// An ordinary field, read from the file where it stands.
	Normal,
	/// `T x in`, a parameter the host supplies.
	In,
	/// `T x out`, a value the host reads back.
	Out,
	/// `T x = expr;`, which reads nothing and computes a value.
	Local,
}

/// A field, wherever one can be written: in a struct, a union, a bitfield or
/// at the top level.
#[derive(Debug, Clone, PartialEq)]
pub struct Field {
	pub ty: TypeRef,
	/// More than one name only for `u32 x, y, z;`.
	pub names: Vec<String>,
	pub array: Option<ArraySize>,
	/// `T *p : u32`, whose value is an offset read as the given type.
	pub pointer: Option<TypeRef>,
	/// `@ addr`, an absolute address in the enclosing space.
	pub placement: Option<Expr>,
	/// `in section`, which places the field in a created section.
	pub section: Option<Expr>,
	/// A bitfield entry's `: n` width, on a field written `bool f : 1`.
	pub width: Option<Expr>,
	/// The right side of `T x = expr;`, or an `in` parameter's default.
	pub init: Option<Expr>,
	/// The right side of `T x[n] = { a, b, c };`, which only a local has.
	pub init_list: Vec<Expr>,
	pub kind: FieldKind,
	/// Written `const T x = e;`.
	pub constant: bool,
	pub attrs: Vec<Attribute>,
	pub doc: Option<String>,
	pub pos: Pos,
}

impl Field {
	/// The one name a field has, or the first of several.
	pub fn name(&self) -> &str {
		self.names.first().map_or("", |name| name.as_str())
	}
}

/// A top-level variable, which is the only thing that reads the file at all.
pub type Placement = Field;

#[derive(Debug, Clone, PartialEq)]
pub struct NamespaceDecl {
	/// The parts of `namespace a::b`, in order.
	pub path: Vec<String>,
	/// Written `namespace auto a`, which the alias of an `import as` replaces.
	pub auto: bool,
	pub decls: Vec<Decl>,
	pub pos: Pos,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FunctionDef {
	pub name: String,
	pub params: Vec<Param>,
	/// The name of an `auto ... args` pack, which ends the parameter list.
	pub param_pack: Option<String>,
	pub body: Vec<Statement>,
	pub attrs: Vec<Attribute>,
	pub doc: Option<String>,
	pub pos: Pos,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Param {
	pub ty: TypeRef,
	/// A parameter may be written without a name, and then has none.
	pub name: Option<String>,
	pub default: Option<Expr>,
	pub pos: Pos,
}
