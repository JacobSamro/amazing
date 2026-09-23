//! The ECMA-262 syntactic grammar as data: AST node types (spec §14–§15).
//!
//! Every [`Stmt`] and [`Expr`] carries its source [`Span`]; inner structs
//! (`Block`, `Declarator`, `ClassMethod`, ...) carry spans where they help
//! later phases report positions.
//!
//! Implemented (storage for everything the Phase 6 front half accepts):
//! - Statements: `var`/`let`/`const` declarations (single-identifier
//!   bindings), function declarations/expressions (incl. `async`,
//!   generators), classes (constructor/method/getter/setter/computed keys/
//!   static members plus *field definitions kept as storage only*),
//!   blocks, empty, expression statements, `if`/`else`, the three `for`
//!   forms (C-style, `for-in`, `for-of`), `while`, `do-while`, `switch`,
//!   labeled/break/continue, `return`, `throw`, `try/catch/finally`,
//!   `debugger`.
//! - Expressions: all literals, identifiers, `this`/`super` (super stored
//!   bare and validated at runtime), template literals, array/object
//!   literals (holes, spread, shorthand, computed keys, methods), arrow
//!   functions (expression or block bodies, async flag), full unary /
//!   binary / logical / conditional / assignment operator set including
//!   `**`, `??`, and the logical-assignment trio `&&= ||= ??=`, prefix and
//!   postfix `++`/`--`, call/member/`new` chains with `?.` optional
//!   chaining flags, sequences, explicit parenthesization ([`ExprKind::
//!   Paren`] keeps `(a, b)` distinct from the sequence expression `a, b`),
//!   and `yield`/`yield*`/`await`.
//!
//! Deferred (documented, not implemented):
//! - Destructuring patterns everywhere (declarators, parameters, assignment
//!   targets, for-in/of heads) with recursive array/object binding patterns;
//!   assignment-pattern targets remain deferred.
//! - `import`/`export` (the parser rejects them; modules are a later
//!   milestone) and therefore top-level `await` context.
//! - `for await (... of ...)` (async iteration).
//! - Optional-chain short-circuit grouping: `optional` flags are recorded
//!   per link but the runtime grouping semantics are the interpreter's job.
//! - Tagged templates, decorators,
//!   `new.target`/`import.meta` meta properties, `with` statements
//!   (Annex B), getters/setters via `Object` literal `__proto__` special
//!   case, default-export-only module items.

use crate::tokens::{Numeric, Span};

/// A whole parsed script.
#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub body: Vec<Stmt>,
    pub span: Span,
}

/// An identifier reference or binding name.
#[derive(Debug, Clone, PartialEq)]
pub struct Ident {
    pub name: String,
    pub span: Span,
}

impl Ident {
    pub fn new(name: impl Into<String>, span: Span) -> Self {
        Ident {
            name: name.into(),
            span,
        }
    }
}

// ---------------------------------------------------------------------------
// Statements
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct Stmt {
    pub kind: StmtKind,
    pub span: Span,
}

impl Stmt {
    pub fn new(kind: StmtKind, span: Span) -> Self {
        Stmt { kind, span }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum StmtKind {
    Var(VarDecl),
    /// Function *declaration* (expressions live in [`ExprKind::Function`]).
    Function(Box<FunctionItem>),
    Class(Box<ClassItem>),
    Block(Block),
    /// The lone `;`.
    Empty,
    /// Expression statement (sequence commas preserved inside [`ExprKind::
    /// Sequence`]).
    Expression(Expr),
    If {
        cond: Expr,
        then: Box<Stmt>,
        els: Option<Box<Stmt>>,
    },
    DoWhile {
        body: Box<Stmt>,
        test: Expr,
    },
    While {
        test: Expr,
        body: Box<Stmt>,
    },
    For {
        init: Option<ForInit>,
        test: Option<Expr>,
        update: Option<Expr>,
        body: Box<Stmt>,
    },
    ForIn {
        left: ForLeft,
        right: Expr,
        body: Box<Stmt>,
    },
    ForOf {
        left: ForLeft,
        right: Expr,
        body: Box<Stmt>,
    },
    Switch {
        disc: Expr,
        cases: Vec<SwitchCase>,
    },
    Continue {
        label: Option<Ident>,
    },
    Break {
        label: Option<Ident>,
    },
    Return(Option<Expr>),
    Labeled {
        label: Ident,
        body: Box<Stmt>,
    },
    Throw(Expr),
    Try(TryStmt),
    Debugger,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VarKind {
    Var,
    Let,
    Const,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VarDecl {
    pub kind: VarKind,
    pub decls: Vec<Declarator>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Declarator {
    pub binding: Binding,
    pub init: Option<Expr>,
    pub span: Span,
}

/// A binding target used by declarations, parameters, and `for` heads.
/// Assignment-pattern targets remain a later parser milestone.
#[derive(Debug, Clone, PartialEq)]
pub enum Binding {
    Ident(Ident),
    Array(Vec<Option<Box<Binding>>>),
    Object(Vec<(BindingKey, Box<Binding>)>),
    Rest(Box<Binding>),
    Default(Box<Binding>, Box<Expr>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum BindingKey {
    Static(String),
    Computed(Box<Expr>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum ForInit {
    Decl(Box<VarDecl>),
    Expr(Expr),
}

/// Left side of `for-in`/`for-of`: either a bare target expression
/// (`for (x in y)`) or a single-declarator lexical declaration
/// (`for (const x of y)`); the parser rejects initializers there.
#[derive(Debug, Clone, PartialEq)]
pub enum ForLeft {
    Target(Expr),
    Decl(Box<VarDecl>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub body: Vec<Stmt>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SwitchCase {
    /// `None` for `default:` (at most one per switch; parser-enforced).
    pub test: Option<Expr>,
    pub body: Vec<Stmt>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TryStmt {
    pub block: Block,
    pub catch: Option<CatchClause>,
    pub finally: Option<Block>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CatchClause {
    /// `None` for optional catch binding (`try {} catch {}`).
    pub param: Option<Ident>,
    pub body: Block,
}

// ---------------------------------------------------------------------------
// Functions & classes
// ---------------------------------------------------------------------------

/// Function declaration or expression (shared shape; `name` is required for
/// declarations — the parser enforces that).
#[derive(Debug, Clone, PartialEq)]
pub struct FunctionItem {
    pub name: Option<Ident>,
    pub params: Vec<Param>,
    pub body: Block,
    pub is_async: bool,
    pub is_generator: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub binding: Binding,
    pub default: Option<Expr>,
    pub rest: bool,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClassItem {
    /// `None` for anonymous class expressions; declarations always have a
    /// name (parser-enforced).
    pub name: Option<Ident>,
    pub super_class: Option<Expr>,
    pub members: Vec<ClassMember>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ClassMember {
    Method(ClassMethod),
    /// Field definition — **storage only** in this phase (initializers are
    /// captured but not scheduled/evaluated semantics-wise yet).
    Field(ClassField),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClassMethod {
    pub key: PropKey,
    pub kind: MethodKind,
    pub is_static: bool,
    pub is_generator: bool,
    pub is_async: bool,
    pub params: Vec<Param>,
    pub body: Block,
    pub span: Span,
}

/// Also reused for object-literal methods (where [`MethodKind::Constructor`]
/// never occurs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MethodKind {
    Constructor,
    Method,
    Get,
    Set,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClassField {
    pub key: PropKey,
    pub is_static: bool,
    pub init: Option<Expr>,
    pub span: Span,
}

/// Property key shared by object literals and classes.
#[derive(Debug, Clone, PartialEq)]
pub enum PropKey {
    Ident(Ident),
    /// Lexically scoped class private name, including its `#`.
    Private(Ident),
    Str(String),
    Num(Numeric),
    Computed(Box<Expr>),
}

// ---------------------------------------------------------------------------
// Expressions
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

impl Expr {
    pub fn new(kind: ExprKind, span: Span) -> Self {
        Expr { kind, span }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExprKind {
    /// Only valid on the left of `in`; tests membership of a private brand.
    PrivateName(Ident),
    /// A validated pattern; evaluation creates a fresh RegExp object.
    RegExp(std::rc::Rc<amazing_regex::Regex>),
    Num(Numeric),
    Str(String),
    Bool(bool),
    Null,
    Ident(Ident),
    This,
    /// Bare `super`; legal only as the object of member/call chains, which
    /// the interpreter validates.
    Super,
    Template(TemplateLit),
    /// Elements are `None` for holes; spread elements appear as
    /// [`ExprKind::Spread`] wrapped in `Some`.
    Array(Vec<Option<Expr>>),
    Object(Vec<ObjElem>),
    /// `...x` in arrays/objects/call arguments.
    Spread(Box<Expr>),
    Function(Box<FunctionItem>),
    Arrow(ArrowFunc),
    Class(Box<ClassItem>),
    /// Includes `await` via [`UnOp::Await`].
    Unary {
        op: UnOp,
        operand: Box<Expr>,
    },
    /// Prefix/postfix `++`/`--`.
    Update {
        op: UpdateOp,
        prefix: bool,
        target: Box<Expr>,
    },
    Binary {
        op: BinOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    /// `&&`, `||`, `??` (short-circuit evaluation differs from [`BinOp`]).
    Logical {
        op: LogicalOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Assign {
        op: AssignOp,
        target: Box<Expr>,
        value: Box<Expr>,
    },
    Conditional {
        test: Box<Expr>,
        consequent: Box<Expr>,
        alternate: Box<Expr>,
    },
    Call {
        callee: Box<Expr>,
        args: Vec<Expr>,
        /// `true` when reached through `?.(...)`; short-circuit grouping
        /// across the whole chain is applied by the interpreter.
        optional: bool,
    },
    New {
        callee: Box<Expr>,
        args: Vec<Expr>,
    },
    Member {
        obj: Box<Expr>,
        prop: MemberProp,
        /// `true` when accessed through `?.`.
        optional: bool,
    },
    /// Only produced at statement level and inside parentheses-free comma
    /// lists; parenthesized `(a, b)` stays wrapped in [`ExprKind::Paren`].
    Sequence(Vec<Expr>),
    /// Explicitly parenthesized expression — preserved so the interpreter
    /// can distinguish `(a, b)` from `a, b` and to future-proof strict-mode
    /// checks around `eval`/`arguments`.
    Paren(Box<Expr>),
    Yield {
        argument: Option<Box<Expr>>,
        delegate: bool,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct TemplateLit {
    /// Cooked quasi strings; always exactly one more quasi than `exprs`
    /// (empty leading/trailing quasis included).
    pub quasis: Vec<String>,
    pub exprs: Vec<Expr>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MemberProp {
    Ident(Ident),
    /// Class private lookup; distinct from a public property named `#x`.
    Private(Ident),
    Computed(Box<Expr>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ArrowFunc {
    pub params: Vec<Param>,
    pub body: ArrowBody,
    pub is_async: bool,
    /// Arrows are never generators; reserved for completeness of queries.
    pub is_generator: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ArrowBody {
    Expr(Box<Expr>),
    Block(Block),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Property {
    pub key: PropKey,
    pub value: PropValue,
    pub span: Span,
}

/// Object-literal elements: normal properties plus keyless spreads.
#[derive(Debug, Clone, PartialEq)]
pub enum ObjElem {
    Prop(Property),
    Spread(Box<Expr>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum PropValue {
    /// `{ a }` — key doubles as the value binding.
    Shorthand(Ident),
    Init(Expr),
    /// Method/getter/setter in an object literal.
    Method(ClassMethod),
}

// ---------------------------------------------------------------------------
// Operators
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssignOp {
    Assign,
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Pow,
    Shl,
    Shr,
    UShr,
    BitAnd,
    BitOr,
    BitXor,
    And,
    Or,
    Nullish,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Pow,
    Lt,
    Gt,
    Le,
    Ge,
    InstanceOf,
    In,
    EqEq,
    NotEq,
    StrictEq,
    StrictNeq,
    Shl,
    Shr,
    UShr,
    BitAnd,
    BitXor,
    BitOr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogicalOp {
    And,
    Or,
    Coalesce,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Pos,
    Not,
    BitNot,
    Typeof,
    Void,
    Delete,
    Await,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateOp {
    Inc,
    Dec,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokens::{SourceLocation, Span};

    fn dummy_span() -> Span {
        let at = SourceLocation::new(1, 1);
        Span::new(at, at)
    }

    #[test]
    fn expression_nodes_round_trip_through_debug_clone_eq() {
        let e = Expr::new(
            ExprKind::Binary {
                op: BinOp::Add,
                left: Box::new(Expr::new(
                    ExprKind::Num(Numeric::new("1", 10, false)),
                    dummy_span(),
                )),
                right: Box::new(Expr::new(
                    ExprKind::Ident(Ident::new("x", dummy_span())),
                    dummy_span(),
                )),
            },
            dummy_span(),
        );
        let clone = e.clone();
        assert!(matches!(
            clone.kind,
            ExprKind::Binary { op: BinOp::Add, .. }
        ));
    }

    #[test]
    fn paren_and_sequence_are_distinct_kinds() {
        let seq = Expr::new(
            ExprKind::Sequence(vec![
                Expr::new(ExprKind::Null, dummy_span()),
                Expr::new(ExprKind::Null, dummy_span()),
            ]),
            dummy_span(),
        );
        let paren = Expr::new(ExprKind::Paren(Box::new(seq.clone())), dummy_span());
        assert!(matches!(paren.kind, ExprKind::Paren(_)));
        assert!(matches!(seq.kind, ExprKind::Sequence(_)));
    }

    #[test]
    fn method_kinds_cover_constructor_get_set_and_plain() {
        let kinds = [
            MethodKind::Constructor,
            MethodKind::Method,
            MethodKind::Get,
            MethodKind::Set,
        ];
        assert_eq!(kinds.len(), 4);
        assert_eq!(kinds.iter().filter(|k| **k == MethodKind::Get).count(), 1);
    }
}
