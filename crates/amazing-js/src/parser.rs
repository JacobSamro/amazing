//! The ECMA-262 syntactic parser (spec §14–§15): tokens → [`Program`].
//!
//! Recursive descent for statements, a precedence-climbing loop for binary
//! operators, and cover-grammar *speculation* for arrow functions: when a
//! `(` could start either parenthesized expressions or arrow parameters the
//! parser attempts a strict parameter parse and rewinds on failure —
//! token-stream position is the only mutable state that speculation needs
//! to restore, so failures are cheap and exact.
//!
//! **ASI correctness** is this module's headline feature (spec §14.13.1):
//! - [`Parser::semicolon`] accepts `;`, or inserts one before `}`, EOF, or
//!   any token with a preceding line terminator.
//! - Restricted productions: `return`/`throw`/`break`/`continue`/`yield`
//!   suppress their operand when a line terminator follows the keyword
//!   (`throw` is a hard error there, as the spec demands).
//! - Postfix `++`/`--` are only postfix on the *same line*; a newline makes
//!   the previous statement end (ASI) and the `++` begin a new one.
//!
//! Implemented:
//! - Script statements and a separate strict Module goal for static imports,
//!   exports, attributes, and top-level await syntax.
//! - Expression grammar with correct precedence: assignment (right-assoc,
//!   all 16 operators), conditional, `??`, `||`, `&&`, bitwise, equality
//!   (incl. `===`/`!==`), relational (`< > <= >= instanceof in`, with the
//!   `no-in` restriction inside for-heads), shifts, additive, multiplicative,
//!   `**` (right-assoc), unary, prefix/postfix update.
//! - The `??`-vs-`||`/`&&` mixing restriction: `a ?? b || c` and
//!   `a || b ?? c` are SyntaxErrors unless parenthesized.
//! - Call/member/`new` chains with `?.` optional-chaining flags; arrow
//!   functions (single-ident fast path plus speculative paren path, async
//!   variants); object/array literals (shorthand, spread, holes, methods,
//!   getters/setters, computed keys); template literals (nested); classes
//!   (constructor/get/set/static/computed keys, field storage); labeled
//!   statements with break/continue target validation; `for-in`/`for-of`
//!   left-target validation; `yield`/`yield*` in generator-flagged
//!   functions; `await` in async-flagged functions.
//!
//! Deferred (documented, not implemented):
//! - Asynchronous module evaluation and dynamic import execution.
//! - Destructuring assignment targets, computed pattern keys, and full
//!   iterator-closing/error semantics (basic declaration/parameter/for-head
//!   array/object patterns are implemented).
//! - Runtime execution of tagged templates, `with`, `new.target`,
//!   `import.meta`, and `for await`; their syntax is represented in the AST.
//!   RegExp literal support uses parser-selected lexical goals.
//! - Sloppy-mode `yield`/`await` as identifiers: `yield` is always the
//!   keyword here; `await` stays an identifier outside async functions.

use crate::ast::*;
use crate::errors::SyntaxError;
use crate::tokens::{Punct, SourceLocation, Span, Token, TokenKind};
use std::collections::{HashMap, HashSet};

#[path = "parser_statements.rs"]
mod statements;
#[path = "parser_functions.rs"]
mod functions;
#[path = "parser_expressions.rs"]
mod expressions;
#[path = "parser_cover.rs"]
mod cover;
#[path = "parser_module.rs"]
mod module;

/// Parses a full script; static imports and exports require [`parse_module`].
pub fn parse_script(src: &str) -> Result<Program, SyntaxError> {
    Parser::new(src).parse_program()
}

pub(crate) fn parse_script_strict(src: &str) -> Result<Program, SyntaxError> {
    let mut parser = Parser::new(src);
    parser.strict_script = true;
    parser.parse_program()
}

/// Parses source under the Module grammar goal (always strict).
pub fn parse_module(src: &str) -> Result<Module, SyntaxError> {
    let mut parser = Parser::new(src);
    parser.strict_script = true;
    parser.module_goal = true;
    parser.tokens.borrow_mut().set_module_goal();
    parser.in_async = 1; // top-level await is valid in a module
    parser.parse_module()
}

#[cfg(test)]
mod js10_grammar_tests {
    use super::*;

    #[test]
    fn using_declarations_keep_resource_kind() {
        let script = parse_script("{ using resource = value; }").unwrap();
        let StmtKind::Block(block) = &script.body[0].kind else { panic!("block") };
        assert!(matches!(&block.body[0].kind, StmtKind::Var(decl) if decl.resource_kind == Some(ResourceKind::Using)));
        assert!(parse_script("using resource = value;").is_err());
        assert!(parse_script("{ using resource; }").is_err());
        assert!(parse_script("{ using [resource] = value; }").is_ok());
        let module = parse_module("await using resource = value;").unwrap();
        assert!(module.has_top_level_await);
        for source in [
            "async function f() { if (true) await using x = null; }",
            "async function f() { while (true) await using x = null; }",
            "async function f() { switch (x) { case 1: await using y = null; } }",
            "for (using let of []) {}",
            "for (using x of []) { var x; }",
        ] {
            assert!(parse_script(source).is_err(), "{source}");
        }
        assert!(parse_script("for (using of of [0]) {}").is_ok());
    }

    #[test]
    fn phase_import_and_for_await_are_distinct_ast_nodes() {
        let script = parse_script("import.source('./x.js');").unwrap();
        assert!(matches!(&script.body[0].kind, StmtKind::Expression(Expr { kind: ExprKind::DynamicImportPhase { phase, .. }, .. }) if phase == "source"));
        assert!(parse_script("new import.source('./x.js')").is_err());
        let module = parse_module("for await (item of items) {}").unwrap();
        assert!(module.has_top_level_await);
        assert!(matches!(&module.items[0], ModuleItem::Statement(Stmt { kind: StmtKind::ForAwaitOf { .. }, .. })));
    }

    #[test]
    fn numeric_separator_and_legacy_zero_early_errors() {
        for source in ["0_0", "00_1", "10._1", "10._", "'use strict'; 08", "'use strict'; 012"] {
            assert!(parse_script(source).is_err(), "{source}");
        }
        assert!(parse_script("10..toString(); 1.0_1; 012;").is_ok());
    }

    #[test]
    fn assignment_pattern_rest_and_optional_chain_targets() {
        assert!(parse_script("({a, ...rest} = source);").is_ok());
        assert!(parse_script("([first, ...rest] = source);").is_ok());
        for source in [
            "({ ...rest, later } = source)",
            "([ ...rest, later ] = source)",
            "([ ...rest, ] = source)",
            "([ ...rest = 1 ] = source)",
            "obj?.property = 1",
            "({key: obj?.property} = source)",
            "'use strict'; ({eval} = source)",
            "'use strict'; arguments += 1",
            "[x] += value",
            "++[x]",
        ] {
            assert!(parse_script(source).is_err(), "{source}");
        }
        assert!(parse_script("call() = 1; call()++; ").is_ok());
        assert!(parse_script("'use strict'; call() = 1").is_err());
    }

    #[test]
    fn class_element_early_errors_and_auto_accessors() {
        for source in [
            "class C { constructor() {} constructor() {} }",
            "class C { 'constructor'; }",
            "class C { static 'prototype'; }",
            "class C { static prototype() {} }",
            "class C { get constructor() {} }",
            "class C { constructor() { super(); } }",
            "class C extends () => {} {}",
        ] {
            assert!(parse_script(source).is_err(), "{source}");
        }
        assert!(parse_script("class C { accessor field; accessor \u{2118}; }").is_ok());
        assert!(parse_script("class C extends Base { constructor() { super(); } }").is_ok());
    }

    #[test]
    fn arrow_formals_are_unique_and_strict_rules_apply() {
        assert!(parse_script("const f = yield => yield;").is_ok());
        for source in [
            "const f = (x, x) => x;",
            "const f = ([x], x) => x;",
            "const f = ({x}, x) => x;",
            "'use strict'; const f = eval => eval;",
            "const f = ({x}) => { 'use strict'; };",
        ] {
            assert!(parse_script(source).is_err(), "{source}");
        }
    }

    #[test]
    fn nested_var_conflicts_with_outer_block_lexical_name() {
        assert!(parse_script("{ let x; { var x; } }").is_err());
        assert!(parse_script("{ { var x; } const x = 1; }").is_err());
        assert!(parse_script("{ let x; { let x; } }").is_ok());
        assert!(parse_script("{ var x; { let x; } }").is_ok());
    }

    #[test]
    fn function_own_strict_directive_validates_preparsed_body() {
        assert!(parse_script("function f() { 'use strict'; var static; }").is_err());
        assert!(parse_script("function f() { 'use strict'; eval = 1; }").is_err());
        assert!(parse_script("function f() { 'use strict'; with (obj) {} }").is_err());
        assert!(parse_script("function f() { var static; }").is_ok());
    }

    #[test]
    fn lexical_declarations_require_statement_list_positions() {
        for source in [
            "if (true) let x = 1;",
            "while (true) const x = 1;",
            "label: let x;",
            "for (;;) class C {}",
            "while (true) function f() {}",
            "do function f() {} while (false);",
            "for (;;) function f() {}",
            "if (false) label: function f() {}",
            "for (;;) label: function f() {}",
            "if (false) async function f() {}",
            "if (false) function* f() {}",
            "'use strict'; if (true) function f() {}",
            "for (let let of []) {}",
            "const let = 1;",
            "function* f() { let\n yield 0; }",
        ] {
            assert!(parse_script(source).is_err(), "{source}");
        }
        assert!(parse_script("if (true) function f() {}").is_ok());
        assert!(parse_script("{ let x = 1; }").is_ok());
        for source in [
            "if (false) let\n{}",
            "if (false) let\nx = 1;",
            "while (false) let\n{}",
            "for (;;) let\nx = 1;",
            "label: let\n{}",
        ] {
            assert!(parse_script(source).is_ok(), "{source}");
        }
    }

    #[test]
    fn assignment_cover_name_and_return_import() {
        assert!(parse_script("'use strict'; var x; ({x = 1} = {});").is_ok());
        assert!(parse_script("'use strict'; ({x = 1});").is_err());
        assert!(parse_script("function f() { return import('./dep.js'); }").is_ok());
    }

    #[test]
    fn annex_b_var_for_in_initializer_only_in_sloppy_code() {
        assert!(parse_script("for (var a = 0 in {});").is_ok());
        assert!(parse_script("'use strict'; for (var a = 0 in {});").is_err());
        assert!(parse_script("for (a = 0 in {});").is_err());
        assert!(parse_script("for (var a = 0 of []);").is_err());
    }

    #[test]
    fn strict_string_legacy_escapes_are_early_errors() {
        assert!(parse_script("'\\8';").is_ok());
        assert!(parse_script("'use strict'; '\\8';").is_err());
        assert!(parse_script("'use strict'; '\\1';").is_err());
        assert!(parse_script("function f() { 'use strict'; '\\9'; }").is_err());
        assert!(parse_module("'\\1';").is_err());
    }

    #[test]
    fn assignment_target_rejects_parenthesized_patterns_and_nested_assignments() {
        assert!(parse_script("(x = y) = 1;").is_err());
        assert!(parse_script("({x}) = source;").is_err());
        assert!(parse_script("([x]) = source;").is_err());
        assert!(parse_script("(x) = 1;").is_ok());
        assert!(parse_script("[x = 1] = source;").is_ok());
    }

    #[test]
    fn nested_pattern_defaults_allow_in_inside_for_of_head() {
        assert!(parse_script("for ([x = 'x' in {}] of [[]]) {}").is_ok());
        assert!(parse_script("for ({x = 'x' in {}} of [{}]) {}").is_ok());
        assert!(parse_script("for (x in object) {}").is_ok());
    }
}

/// Which kind of binary operator the Pratt loop is currently holding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BinKind {
    Arithmetic(BinOp),
    Logical(LogicalOp),
}

#[derive(Debug, Clone)]
struct LabelCtx {
    name: String,
    /// `true` when the label directly wraps a loop (allows `continue`).
    is_loop: bool,
}

#[derive(Default)]
struct PrivateScope {
    // Getter and setter may share one name; all other duplicates are errors.
    declared: HashMap<String, (u8, bool)>,
    uses: Vec<Ident>,
}

struct Parser {
    tokens: std::cell::RefCell<crate::token_stream::TokenStream>,
    pos: usize,
    /// End of the most recently consumed token (for node spans).
    last_end: SourceLocation,
    /// `in`-operator suppression inside `for(...)` heads (spec: no-in).
    no_in: bool,
    labels: Vec<LabelCtx>,
    loop_depth: usize,
    switch_depth: usize,
    /// Nesting counters (stacks flattened to depth because only depth
    /// matters for validity checks).
    in_function: usize,
    /// Non-arrow function or method context; arrows inherit this lexically.
    in_new_target: usize,
    in_generator: usize,
    in_async: usize,
    in_formal_params: usize,
    /// A script directive applies to methods parsed after it.
    strict_script: bool,
    module_goal: bool,
    has_top_level_await: bool,
    /// Class field initializers inherit this restriction through arrows.
    in_class_field_initializer: usize,
    in_static_block: usize,
    block_depth: usize,
    /// Only a class constructor may contain a direct `super()` call.
    super_call_allowed: bool,
    /// Depth of `${ … }` substitutions currently being parsed. A
    /// `TemplatePart` token seen at depth > 0 is the enclosing template's
    /// continuation (the lexer absorbed the closing `}` into it), not the
    /// start of a tagged template.
    template_depth: usize,
    private_scopes: Vec<PrivateScope>,
    class_heritage: Vec<bool>,
}

impl Parser {
    fn new(src: &str) -> Self {
        let last_end = SourceLocation::new(1, 1);
        Parser {
            tokens: std::cell::RefCell::new(crate::token_stream::TokenStream::new(src)),
            pos: 0,
            last_end,
            no_in: false,
            labels: Vec::new(),
            loop_depth: 0,
            switch_depth: 0,
            in_function: 0,
            in_new_target: 0,
            in_generator: 0,
            in_async: 0,
            in_formal_params: 0,
            strict_script: false,
            module_goal: false,
            has_top_level_await: false,
            in_class_field_initializer: 0,
            in_static_block: 0,
            block_depth: 0,
            super_call_allowed: false,
            template_depth: 0,
            private_scopes: Vec::new(),
            class_heritage: Vec::new(),
        }
    }

    // ----- token cursor ---------------------------------------------------

    fn cur(&self) -> Token {
        self.nth(0)
    }

    fn nth(&self, n: usize) -> Token {
        self.tokens.borrow_mut().get(self.pos + n)
    }

    fn bump(&mut self) -> Token {
        let t = self.cur();
        if !matches!(t.kind, TokenKind::Eof) {
            self.pos += 1;
        }
        self.last_end = t.span.end;
        t
    }

    fn at_eof(&self) -> bool {
        matches!(self.cur().kind, TokenKind::Eof)
    }

    fn cur_loc(&self) -> SourceLocation {
        self.cur().span.start
    }

    fn span_from(&self, start: SourceLocation) -> Span {
        Span::new(start, self.last_end)
    }

    fn parse_assignment_with_in(&mut self) -> Result<Expr, SyntaxError> {
        let previous = self.no_in;
        self.no_in = false;
        let result = self.parse_assignment_expr();
        self.no_in = previous;
        result
    }

    fn err(&self, msg: impl Into<String>) -> SyntaxError {
        if let TokenKind::Invalid(error) = self.cur().kind {
            return error;
        }
        SyntaxError::new(msg, self.cur_loc().line, self.cur_loc().column)
    }

    fn err_at(&self, at: SourceLocation, msg: impl Into<String>) -> SyntaxError {
        SyntaxError::new(msg, at.line, at.column)
    }

    fn at_punct(&self, p: Punct) -> bool {
        matches!(self.cur().kind, TokenKind::Punct(q) if q == p)
    }

    fn nth_punct(&self, n: usize, p: Punct) -> bool {
        matches!(self.nth(n).kind, TokenKind::Punct(q) if q == p)
    }

    fn eat_punct(&mut self, p: Punct) -> bool {
        if self.at_punct(p) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn expect_punct(&mut self, p: Punct) -> Result<Span, SyntaxError> {
        if self.at_punct(p) {
            Ok(self.bump().span)
        } else {
            Err(self.err(format!(
                "expected `{}`, found {}",
                p.spelling(),
                self.cur().kind.describe()
            )))
        }
    }

    fn at_kw(&self, k: crate::tokens::Keyword) -> bool {
        !self.cur().escaped_identifier && matches!(self.cur().kind, TokenKind::Keyword(q) if q == k)
    }

    fn eat_kw(&mut self, k: crate::tokens::Keyword) -> bool {
        if self.at_kw(k) {
            self.bump();
            true
        } else {
            false
        }
    }

    /// True when the current token is an identifier with this exact spelling
    /// (contextual keywords are identifiers by design).
    fn word(&self, s: &str) -> bool {
        let token = self.cur();
        !token.escaped_identifier && matches!(&token.kind, TokenKind::Ident(name) if name == s)
    }

    /// Consumes an identifier token (contextual words included, reserved
    /// words excluded — they are different token kinds).
    fn expect_ident(&mut self) -> Result<Ident, SyntaxError> {
        let t = self.cur().clone();
        match t.kind {
            TokenKind::Ident(name) => {
                if name == "await" && (self.in_async > 0 || self.in_static_block > 0) {
                    return Err(self.err_at(t.span.start, "await is reserved in async functions"));
                }
                if self.forbidden_binding_name(&name) {
                    return Err(self.err_at(t.span.start, "reserved binding name"));
                }
                self.bump();
                Ok(Ident::new(name, t.span))
            }
            TokenKind::Keyword(crate::tokens::Keyword::Yield)
                if !self.strict_script && self.in_generator == 0 => {
                    self.bump();
                    Ok(Ident::new("yield", t.span))
                }
            _ => Err(self.err(format!("expected identifier, found {}", t.kind.describe()))),
        }
    }

    fn forbidden_binding_name(&self, name: &str) -> bool {
        name == "enum"
            || name == "await" && self.in_static_block > 0
            || self.strict_script && matches!(name,
                "implements" | "interface" | "let" | "package" | "private" | "protected"
                | "public" | "static" | "yield")
            || crate::tokens::keyword_of(name).is_some()
                && !(name == "yield" && !self.strict_script && self.in_generator == 0)
    }

    /// A property name position accepts reserved words too (`a.if`,
    /// `{if: 1}`). Returns the spelling.
    fn prop_name(&mut self) -> Result<Ident, SyntaxError> {
        let t = self.cur().clone();
        match &t.kind {
            TokenKind::Ident(name) => {
                let id = Ident::new(name.clone(), t.span);
                self.bump();
                Ok(id)
            }
            TokenKind::Keyword(k) => {
                let id = Ident::new(k.spelling(), t.span);
                self.bump();
                Ok(id)
            }
            _ => Err(self.err(format!(
                "expected property name, found {}",
                t.kind.describe()
            ))),
        }
    }

    fn member_name(&mut self) -> Result<MemberProp, SyntaxError> {
        if let TokenKind::PrivateIdent(name) = self.cur().kind.clone() {
            let t = self.bump();
            let id = Ident::new(name, t.span);
            self.use_private(id.clone())?;
            Ok(MemberProp::Private(id))
        } else {
            Ok(MemberProp::Ident(self.prop_name()?))
        }
    }

    /// Automatic semicolon insertion (spec §14.13.1, rules 1–3).
    fn semicolon(&mut self) -> Result<(), SyntaxError> {
        if self.eat_punct(Punct::Semi) {
            return Ok(());
        }
        let t = self.cur();
        if matches!(t.kind, TokenKind::Eof | TokenKind::Punct(Punct::Rbrace)) || t.newline_before {
            Ok(()) // inserted
        } else {
            Err(self.err(format!("expected `;`, found {}", t.kind.describe())))
        }
    }

}

/// Helper: name of an identifier property key (constructor detection).
fn prop_key_name(key: &PropKey) -> Option<&str> {
    match key {
        PropKey::Ident(id) => Some(id.name.as_str()),
        PropKey::Str(name) => Some(name.as_str()),
        _ => None,
    }
}

fn directive_string(stmt: &Stmt) -> Option<&str> {
    match &stmt.kind {
        StmtKind::Expression(Expr {
            kind: ExprKind::Str(value),
            ..
        }) => Some(value),
        _ => None,
    }
}

fn has_use_strict(block: &Block) -> bool {
    for stmt in &block.body {
        match directive_string(stmt) {
            Some("use strict") => return true,
            Some(_) => {}
            None => break,
        }
    }
    false
}

fn binding_names(binding: &Binding) -> Vec<&Ident> {
    match binding {
        Binding::Ident(ident) => vec![ident],
        Binding::Array(elements) => elements
            .iter()
            .flatten()
            .flat_map(|item| binding_names(item))
            .collect(),
        Binding::Object(properties) => properties
            .iter()
            .flat_map(|(_, item)| binding_names(item))
            .collect(),
        Binding::Rest(inner) | Binding::Default(inner, _) => binding_names(inner),
    }
}

fn is_private_reference(expr: &Expr) -> bool {
    match &expr.kind {
        ExprKind::Member {
            prop: MemberProp::Private(_),
            ..
        } => true,
        ExprKind::Paren(inner) => is_private_reference(inner),
        _ => false,
    }
}

/// Validates a for-in/of lexical head: exactly one declarator, no
/// initializer (`for (const x of y)` ok, `for (let i = 0 of y)` rejected).
fn validate_for_decl(decl: &VarDecl, at: SourceLocation, allow_annex_b_initializer: bool) -> Result<VarDecl, SyntaxError> {
    if decl.decls.len() != 1 {
        return Err(SyntaxError::new(
            "for-in/of head must declare exactly one binding",
            at.line,
            at.column,
        ));
    }
    if decl.decls[0].init.is_some()
        && !(allow_annex_b_initializer && decl.kind == VarKind::Var
            && matches!(decl.decls[0].binding, Binding::Ident(_))) {
        return Err(SyntaxError::new(
            "for-in/of loop variables must not have initializers",
            at.line,
            at.column,
        ));
    }
    Ok(decl.clone())
}

/// Assignment-target validity. Array/object patterns reuse literal AST nodes;
/// their elements must recursively be assignable references.
fn ensure_assignable(e: &Expr) -> Result<(), SyntaxError> {
    match &e.kind {
        ExprKind::Ident(_) => Ok(()),
        ExprKind::Member { .. } if !optional_chain_target(e) => Ok(()),
        ExprKind::Paren(inner) if matches!(inner.kind, ExprKind::Object(_) | ExprKind::Array(_) | ExprKind::Assign { .. }) => {
            Err(SyntaxError::new("parenthesized pattern is not an assignment target", e.span.start.line, e.span.start.column))
        }
        ExprKind::Paren(inner) => ensure_assignable(inner),
        ExprKind::Assign {
            op: AssignOp::Assign,
            target,
            ..
        } => ensure_assignable(target),
        ExprKind::Array(elements) => {
            if e.trailing_comma_after_spread {
                return Err(SyntaxError::new("rest element cannot have trailing comma", e.span.start.line, e.span.start.column));
            }
            for (index, element) in elements.iter().enumerate() {
                let Some(element) = element else { continue };
                if let ExprKind::Spread(inner) = &element.kind {
                    if index + 1 != elements.len() || matches!(inner.kind, ExprKind::Assign { .. }) {
                        return Err(SyntaxError::new("rest element must be last without initializer", e.span.start.line, e.span.start.column));
                    }
                    ensure_assignable(inner)?;
                } else {
                    ensure_assignable(element)?;
                }
            }
            Ok(())
        }
        ExprKind::Object(elements) => {
            if e.trailing_comma_after_spread {
                return Err(SyntaxError::new("rest property cannot have trailing comma", e.span.start.line, e.span.start.column));
            }
            for (index, element) in elements.iter().enumerate() {
                match element {
                    ObjElem::Spread(inner) => {
                        if index + 1 != elements.len() {
                            return Err(SyntaxError::new("rest property must be last", e.span.start.line, e.span.start.column));
                        }
                        if let ExprKind::Spread(target) = &inner.kind { ensure_assignable(target)?; }
                        else { ensure_assignable(inner)?; }
                    }
                    ObjElem::Prop(property) => match &property.value {
                        PropValue::Shorthand(_) => {}
                        PropValue::CoverInitializedName { .. } => {}
                        PropValue::Init(value) => ensure_assignable(value)?,
                        PropValue::Method(_) => {
                            return Err(SyntaxError::new(
                                "methods are not assignment targets",
                                e.span.start.line,
                                e.span.start.column,
                            ));
                        }
                    },
                }
            }
            Ok(())
        }
        _ => Err(SyntaxError::new(
            "invalid assignment target",
            e.span.start.line,
            e.span.start.column,
        )),
    }
}

fn optional_chain_target(expr: &Expr) -> bool {
    match &expr.kind {
        ExprKind::Member { obj, optional, .. } => *optional || optional_chain_target(obj),
        ExprKind::Call { callee, optional, .. } => *optional || optional_chain_target(callee),
        ExprKind::Paren(_) => false,
        _ => false,
    }
}

fn ensure_simple_assignable(expr: &Expr, sloppy: bool) -> Result<(), SyntaxError> {
    match &expr.kind {
        ExprKind::Ident(_) => Ok(()),
        ExprKind::Member { .. } if !optional_chain_target(expr) => Ok(()),
        ExprKind::Paren(inner) => ensure_simple_assignable(inner, sloppy),
        ExprKind::Call { .. } if sloppy && !optional_chain_target(expr) => Ok(()),
        _ => Err(SyntaxError::new("invalid simple assignment target", expr.span.start.line, expr.span.start.column)),
    }
}

fn strict_assignment_name(expr: &Expr) -> Option<Span> {
    match &expr.kind {
        ExprKind::Ident(name) if matches!(name.name.as_str(), "eval" | "arguments") => Some(name.span),
        ExprKind::Paren(inner) | ExprKind::Spread(inner) => strict_assignment_name(inner),
        ExprKind::Assign { target, .. } => strict_assignment_name(target),
        ExprKind::Array(items) => items.iter().flatten().find_map(strict_assignment_name),
        ExprKind::Object(items) => items.iter().find_map(|item| match item {
            ObjElem::Spread(inner) => strict_assignment_name(inner),
            ObjElem::Prop(property) => match &property.value {
                PropValue::Shorthand(name) if matches!(name.name.as_str(), "eval" | "arguments") => Some(name.span),
                PropValue::CoverInitializedName { binding, .. } if matches!(binding.name.as_str(), "eval" | "arguments") => Some(binding.span),
                PropValue::Init(value) => strict_assignment_name(value),
                _ => None,
            },
        }),
        _ => None,
    }
}


/// Minimum precedence for the Pratt loop (everything above primaries).
const MIN_PREC: u8 = 1;

fn plain_param(id: Ident) -> Param {
    let span = id.span;
    Param {
        binding: Binding::Ident(id),
        default: None,
        rest: false,
        span,
    }
}

fn assign_op_of(p: Punct) -> Option<AssignOp> {
    use AssignOp::*;
    Some(match p {
        Punct::Eq => Assign,
        Punct::PlusEq => Add,
        Punct::MinusEq => Sub,
        Punct::StarEq => Mul,
        Punct::SlashEq => Div,
        Punct::PercentEq => Rem,
        Punct::PowEq => Pow,
        Punct::ShlEq => Shl,
        Punct::ShrEq => Shr,
        Punct::UShrEq => UShr,
        Punct::AndEq => BitAnd,
        Punct::OrEq => BitOr,
        Punct::XorEq => BitXor,
        Punct::AndAndEq => And,
        Punct::OrOrEq => Or,
        Punct::NullishEq => Nullish,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::parse_script;
    use crate::ast::*;
    use crate::errors::SyntaxError;

    fn program(src: &str) -> Program {
        parse_script(src).expect("should parse")
    }

    fn stmt_kind(src: &str) -> StmtKind {
        let mut p = program(src);
        assert_eq!(p.body.len(), 1, "expected exactly one statement");
        p.body.remove(0).kind
    }

    fn expr_kind(src: &str) -> ExprKind {
        match stmt_kind(src) {
            StmtKind::Expression(e) => e.kind,
            other => panic!("expected expression statement, got {other:?}"),
        }
    }

    #[test]
    fn bigint_literals_have_lossless_decimal_ast_digits() {
        for (source, expected) in [
            ("123n", "123"), ("0x1Fn", "31"), ("0o7n", "7"),
            ("0b1n", "1"), ("0xFFFFFFFFFFFFFFFFFFFFFFFFn", "79228162514264337593543950335"),
            ("1_000n", "1000"),
        ] {
            assert_eq!(expr_kind(source), ExprKind::BigInt(expected.into()), "{source}");
        }
    }

    #[test]
    fn front_end_forms_preserve_shape_and_early_errors() {
        let p = program("class C { ; static { x(); } *m(a,) {} field = 1;; }");
        let StmtKind::Class(class) = &p.body[0].kind else { panic!("class"); };
        assert!(matches!(class.members[0], ClassMember::StaticBlock(_)));
        assert!(matches!(class.members[1], ClassMember::Method(_)));
        assert!(matches!(class.members[2], ClassMember::Field(_)));
        assert!(matches!(stmt_kind("try {} catch ([x, ...rest]) {}"), StmtKind::Try(_)));
        assert!(matches!(stmt_kind("with (obj) x;"), StmtKind::With { .. }));
        assert!(matches!(expr_kind("tag`a${x}b`"), ExprKind::TaggedTemplate { .. }));
        assert!(matches!(expr_kind("import('./dep.js')"), ExprKind::DynamicImport(args) if args.len() == 1));
        assert!(err("import()").message.contains("dynamic import"));
        assert!(parse_script("({x = 1, nested: {y = 2}} = source);").is_ok());
        assert!(err("({x = 1});").message.contains("cover initialized"));
        assert!(matches!(program("do {} while (false) x;").body[0].kind, StmtKind::DoWhile { .. }));
        assert!(err("class C { constructor = 1; }").message.contains("constructor"));
        assert!(err("\"use strict\"; with (obj) x;").message.contains("strict"));
    }

    fn err(src: &str) -> SyntaxError {
        parse_script(src).expect_err("should be a SyntaxError")
    }

    // =========================================================================
    // ASI battery (spec §14.13.1)
    // =========================================================================

    #[test]
    fn newline_separates_two_expression_statements() {
        let p = program("a\nb");
        assert_eq!(p.body.len(), 2);
        assert!(matches!(p.body[0].kind, StmtKind::Expression(_)));
        assert!(matches!(p.body[1].kind, StmtKind::Expression(_)));
    }

    #[test]
    fn asi_inserts_semicolon_before_closing_brace_and_eof() {
        // `a` and `b` both end by ASI before `}` / EOF.
        let p = program("{ a\nb\n}");
        assert!(matches!(p.body[0].kind, StmtKind::Block(_)));
    }

    #[test]
    fn no_asi_when_next_line_continues_the_expression() {
        // `a` followed by `+ b` on the next line is ONE statement.
        let e = expr_kind("a\n+ b");
        assert!(matches!(e, ExprKind::Binary { op: BinOp::Add, .. }));
        // Member access across a newline is likewise one statement.
        let e = expr_kind("x\n.y");
        assert!(matches!(e, ExprKind::Member { .. }));
    }

    #[test]
    fn return_suppresses_operand_after_newline() {
        let p = program("function f() {\nreturn\nx;\n}");
        let f = match &p.body[0].kind {
            StmtKind::Function(f) => f,
            other => panic!("expected function decl, got {other:?}"),
        };
        assert_eq!(f.body.body.len(), 2, "return; and x; must be separate");
        assert!(matches!(f.body.body[0].kind, StmtKind::Return(None)));
        assert!(matches!(f.body.body[1].kind, StmtKind::Expression(_)));
    }

    #[test]
    fn throw_never_survives_a_newline() {
        let e = err("throw\nnew Error('x')");
        assert!(e.message.contains("throw"));
    }

    #[test]
    fn postfix_update_is_restricted_to_the_same_line() {
        // `a\n++b` is two statements: `a;` then prefix `++b`.
        let p = program("a\n++b");
        assert_eq!(p.body.len(), 2);
        assert!(matches!(
            p.body[1].kind,
            StmtKind::Expression(Expr {
                kind: ExprKind::Update { prefix: true, .. },
                ..
            })
        ));
    }

    #[test]
    fn postfix_update_on_same_line_binds_tightly() {
        let e = expr_kind("i++");
        assert!(matches!(e, ExprKind::Update { prefix: false, .. }));
        let e = expr_kind("i--");
        assert!(matches!(e, ExprKind::Update { prefix: false, .. }));
    }

    #[test]
    fn do_while_takes_optional_semicolon_via_asi() {
        let p = program("do ; while (0)\nx");
        assert_eq!(p.body.len(), 2);
        assert!(matches!(p.body[0].kind, StmtKind::DoWhile { .. }));
    }

    #[test]
    fn break_gets_asi_before_the_next_case_label() {
        let mut p = program("switch (x) { case 1: break\ncase 2: y }");
        let cases = match p.body.remove(0).kind {
            StmtKind::Switch { cases, .. } => cases,
            other => panic!("expected switch, got {other:?}"),
        };
        assert!(matches!(
            cases[0].body[0].kind,
            StmtKind::Break { label: None }
        ));
        // `break\ncase` must NOT have consumed `case` as a label.
        assert_eq!(cases.len(), 2);
    }

    #[test]
    fn yield_suppresses_operand_after_newline() {
        let p = program("function* g() {\nyield\n1;\n}");
        let body = match &p.body[0].kind {
            StmtKind::Function(f) => &f.body.body,
            other => panic!("expected generator decl, got {other:?}"),
        };
        assert!(matches!(
            body[0].kind,
            StmtKind::Expression(Expr {
                kind: ExprKind::Yield { argument: None, .. },
                ..
            })
        ));
        assert_eq!(body.len(), 2);
    }

    #[test]
    fn var_declaration_ends_by_asi() {
        let p = program("var x = 1\nvar y = 2");
        assert_eq!(p.body.len(), 2);
        assert!(matches!(p.body[1].kind, StmtKind::Var(_)));
    }

    #[test]
    fn missing_semicolon_without_newline_is_an_error() {
        assert!(err("a b").message.contains(';'));
    }

    // =========================================================================
    // Precedence & associativity fixtures
    // =========================================================================

    #[test]
    fn multiplication_binds_tighter_than_addition() {
        let e = expr_kind("1 + 2 * 3");
        let ExprKind::Binary {
            op: BinOp::Add,
            left,
            right,
        } = e
        else {
            panic!("expected + at top")
        };
        assert!(matches!(left.kind, ExprKind::Num(_)));
        assert!(matches!(
            right.kind,
            ExprKind::Binary { op: BinOp::Mul, .. }
        ));
    }

    #[test]
    fn parentheses_override_precedence_and_stay_marked() {
        let e = expr_kind("(1 + 2) * 3");
        let ExprKind::Binary {
            op: BinOp::Mul,
            left,
            ..
        } = e
        else {
            panic!("expected * at top")
        };
        assert!(matches!(left.kind, ExprKind::Paren(_)));
        // `(a, b)` stays Paren(Sequence) — distinct from a bare sequence.
        let e = expr_kind("(a, b)");
        assert!(matches!(e, ExprKind::Paren(inner) if matches!(inner.kind, ExprKind::Sequence(_))));
    }

    #[test]
    fn exponentiation_is_right_associative() {
        let e = expr_kind("2 ** 3 ** 2");
        let ExprKind::Binary {
            op: BinOp::Pow,
            right,
            ..
        } = e
        else {
            panic!("expected ** at top")
        };
        assert!(matches!(
            right.kind,
            ExprKind::Binary { op: BinOp::Pow, .. }
        ));
    }

    #[test]
    fn unary_operand_cannot_directly_feed_exponentiation() {
        let e = err("-2 ** 2");
        assert!(e.message.contains("**"));
        // Parenthesized form is fine.
        assert!(matches!(
            expr_kind("(-2) ** 2"),
            ExprKind::Binary { op: BinOp::Pow, .. }
        ));
    }

    #[test]
    fn coalesce_cannot_mix_with_logical_operators_unparenthesized() {
        assert!(err("a ?? b || c").message.contains("??"));
        assert!(err("a || b ?? c").message.contains("??"));
        assert!(err("a ?? b && c").message.contains("??"));
        assert!(err("a && b ?? c").message.contains("??"));
        // Parenthesized mixes are fine in both orders.
        assert!(matches!(
            expr_kind("(a ?? b) || c"),
            ExprKind::Logical {
                op: LogicalOp::Or,
                ..
            }
        ));
        assert!(matches!(
            expr_kind("a ?? (b || c)"),
            ExprKind::Logical {
                op: LogicalOp::Coalesce,
                ..
            }
        ));
        // Chained `??` is legal.
        assert!(matches!(
            expr_kind("a ?? b ?? c"),
            ExprKind::Logical {
                op: LogicalOp::Coalesce,
                ..
            }
        ));
    }

    #[test]
    fn assignment_is_right_associative() {
        let e = expr_kind("a = b = c");
        let ExprKind::Assign { value, .. } = e else {
            panic!("expected assignment")
        };
        assert!(matches!(value.kind, ExprKind::Assign { .. }));
    }

    #[test]
    fn logical_assignment_operators_are_recognized() {
        for src in ["a &&= b", "a ||= b", "a ??= b"] {
            assert!(matches!(expr_kind(src), ExprKind::Assign { .. }), "{src}");
        }
        assert!(matches!(
            expr_kind("a **= b"),
            ExprKind::Assign {
                op: AssignOp::Pow,
                ..
            }
        ));
    }

    #[test]
    fn typeof_binds_tighter_than_equality() {
        let e = expr_kind("typeof a === 'string'");
        let ExprKind::Binary {
            op: BinOp::StrictEq,
            left,
            ..
        } = e
        else {
            panic!("expected === at top")
        };
        assert!(matches!(
            left.kind,
            ExprKind::Unary {
                op: UnOp::Typeof,
                ..
            }
        ));
    }

    #[test]
    fn relational_binds_tighter_than_equality() {
        let e = expr_kind("1 < 2 === 3 > 2");
        let ExprKind::Binary {
            op: BinOp::StrictEq,
            left,
            right,
        } = e
        else {
            panic!("expected === at top")
        };
        assert!(matches!(left.kind, ExprKind::Binary { op: BinOp::Lt, .. }));
        assert!(matches!(right.kind, ExprKind::Binary { op: BinOp::Gt, .. }));
    }

    #[test]
    fn and_binds_tighter_than_or() {
        let e = expr_kind("a || b && c");
        let ExprKind::Logical {
            op: LogicalOp::Or,
            right,
            ..
        } = e
        else {
            panic!("expected || at top")
        };
        assert!(matches!(
            right.kind,
            ExprKind::Logical {
                op: LogicalOp::And,
                ..
            }
        ));
    }

    #[test]
    fn instance_of_and_in_parse_as_relational_operators() {
        assert!(matches!(
            expr_kind("a instanceof B"),
            ExprKind::Binary {
                op: BinOp::InstanceOf,
                ..
            }
        ));
        assert!(matches!(
            expr_kind("'k' in obj"),
            ExprKind::Binary { op: BinOp::In, .. }
        ));
    }

    #[test]
    fn in_operator_is_suppressed_in_for_heads_but_allowed_when_parenthesized() {
        // no-in restriction: unparenthesized `in` in the for-init head makes
        // the whole statement invalid (spec §14.8.2).
        assert!(
            matches!(parse_script("for (q = 'k' in o; ; ) ;"), Err(_)),
            "unparenthesized `in` must be rejected in a for-init head"
        );
        let p = program("for (var i = ('k' in o); i; i++) ;");
        assert!(matches!(p.body[0].kind, StmtKind::For { .. }));
    }

    #[test]
    fn conditional_binds_looser_than_assignment_target_side() {
        let e = expr_kind("a ? b : c");
        assert!(matches!(e, ExprKind::Conditional { .. }));
        // Nested in branches via assignment grammar.
        assert!(matches!(
            expr_kind("a ? b = 1 : c = 2"),
            ExprKind::Conditional { .. }
        ));
    }

    // =========================================================================
    // Arrow functions
    // =========================================================================

    #[test]
    fn single_param_arrow_with_expression_body() {
        let e = expr_kind("x => x * 2");
        let ExprKind::Arrow(a) = e else {
            panic!("expected arrow")
        };
        assert_eq!(a.params.len(), 1);
        assert!(matches!(a.body, ArrowBody::Expr(_)));
        assert!(!a.is_async);
    }

    #[test]
    fn parenthesized_params_with_defaults_and_rest() {
        let e = expr_kind("(a, b = 1, ...rest) => a");
        let ExprKind::Arrow(a) = e else {
            panic!("expected arrow")
        };
        assert_eq!(a.params.len(), 3);
        assert!(a.params[1].default.is_some());
        assert!(a.params[2].rest);
    }

    #[test]
    fn empty_param_arrow_block_body_vs_object_body() {
        // `() => {}` is a BLOCK body (empty), never an object literal.
        let e = expr_kind("() => {}");
        let ExprKind::Arrow(a) = e else {
            panic!("expected arrow")
        };
        assert!(matches!(a.body, ArrowBody::Block(_)));
        // Object literal body needs explicit parens.
        let e = expr_kind("() => ({ x: 1 })");
        let ExprKind::Arrow(a) = e else {
            panic!("expected arrow")
        };
        assert!(matches!(a.body, ArrowBody::Expr(_)));
    }

    #[test]
    fn async_arrows_in_all_shapes() {
        let e = expr_kind("async x => x");
        let ExprKind::Arrow(a) = e else {
            panic!("expected arrow")
        };
        assert!(a.is_async);
        let e = expr_kind("async () => 1");
        let ExprKind::Arrow(a) = e else {
            panic!("expected arrow")
        };
        assert!(a.is_async);
        let e = expr_kind("async (a, b) => a + b");
        let ExprKind::Arrow(a) = e else {
            panic!("expected arrow")
        };
        assert!(a.is_async && a.params.len() == 2);
        // `async => 1` is a NON-async arrow whose parameter is `async`.
        let e = expr_kind("async => 1");
        let ExprKind::Arrow(a) = e else {
            panic!("expected arrow")
        };
        assert!(!a.is_async && a.params.len() == 1);
    }

    #[test]
    fn parenthesized_non_arrow_falls_back_to_sequence() {
        let e = expr_kind("(a, b)");
        assert!(matches!(e, ExprKind::Paren(_)));
        let e = expr_kind("(a = 1)");
        assert!(matches!(
            e,
            ExprKind::Paren(inner) if matches!(inner.kind, ExprKind::Assign { .. })
        ));
    }

    #[test]
    fn arrow_as_call_argument_and_in_ternary_branches() {
        assert!(matches!(
            expr_kind("[1].map(x => x * 2)"),
            ExprKind::Call { .. }
        ));
        assert!(matches!(
            expr_kind("c ? (a) => 1 : (b) => 2"),
            ExprKind::Conditional { .. }
        ));
        // Immediately-invoked arrow.
        assert!(matches!(expr_kind("(() => 1)()"), ExprKind::Call { .. }));
    }

    #[test]
    fn await_parses_as_unary_inside_async_functions_and_as_ident_outside() {
        let p = program("async function f() { await g(); }");
        let body = match &p.body[0].kind {
            StmtKind::Function(f) => &f.body.body,
            other => panic!("expected async function, got {other:?}"),
        };
        assert!(matches!(
            body[0].kind,
            StmtKind::Expression(Expr {
                kind: ExprKind::Unary {
                    op: UnOp::Await,
                    ..
                },
                ..
            })
        ));
        // Sloppy-mode deferral: outside async functions `await` stays a
        // plain identifier (see module docs).
        assert!(parse_script("function f() { await(1); }").is_ok());
    }

    #[test]
    fn async_methods_and_await_accept_a_newline_before_the_token() {
        for source in [
            "var driver = {\n  async subscribe() {\n    await ready();\n  }\n};",
            "class Driver {\n  async subscribe() {\n    await ready();\n  }\n}",
            "async function run() {\n  await ready();\n}",
        ] {
            assert!(parse_script(source).is_ok(), "rejected {source}");
        }
        for source in ["var driver = { async\nsubscribe() {} };"] {
            assert!(parse_script(source).is_err(), "accepted {source}");
        }
        // In a class, the same spelling is a field followed by a method.
        assert!(parse_script("class Driver { async\nsubscribe() {} }").is_ok());
    }

    #[test]
    fn async_method_formals_keep_their_early_errors() {
        for source in [
            "var o = {\n async method(x, x) {} };",
            "var o = {\n async *method(x = 0, x) {} };",
            "var o = {\n async method([x]) { 'use strict'; } };",
            "var o = {\n async method(await) {} };",
            "var o = {\n async method(x = await value) {} };",
            "'use strict'; var o = {\n async method(eval) {} };",
            "'use strict'; var o = {\n async method(arguments) {} };",
        ] {
            assert!(parse_script(source).is_err(), "accepted {source}");
        }
        assert!(parse_script("var o = {\n async method(a, b) { await ready(); } };").is_ok());
    }

    #[test]
    fn class_constructors_reject_async_and_generator_modifiers() {
        for source in [
            "class C {\n async constructor() {} }",
            "class C {\n async *constructor() {} }",
            "var C = class {\n async constructor() {} };",
            "var C = class {\n async *constructor() {} };",
        ] {
            assert!(parse_script(source).is_err(), "accepted {source}");
        }
        assert!(parse_script("class C { static async constructor() {} }").is_ok());
    }

    #[test]
    fn call_arguments_allow_one_trailing_comma() {
        for source in [
            "f(1,);",
            "f(1, ...rest,);",
            "new C(1,);",
            "obj?.method(1,);",
        ] {
            assert!(parse_script(source).is_ok(), "rejected {source}");
        }
        for source in ["f(,);", "f(1,,);", "f(...rest,,);"] {
            assert!(parse_script(source).is_err(), "accepted {source}");
        }
    }

    // =========================================================================
    // Template literals
    // =========================================================================

    #[test]
    fn template_with_substitution_shapes_quasis_and_exprs() {
        let e = expr_kind("`a${1 + 2}c`");
        let ExprKind::Template(t) = e else {
            panic!("expected template")
        };
        assert_eq!(t.quasis, vec!["a".to_string(), "c".to_string()]);
        assert_eq!(t.exprs.len(), 1);
        assert!(matches!(
            t.exprs[0].kind,
            ExprKind::Binary { op: BinOp::Add, .. }
        ));
    }

    #[test]
    fn string_literals_and_property_keys_keep_lossless_utf16() {
        let StmtKind::Expression(string) = stmt_kind("'\\uD800'") else {
            panic!("expected string expression")
        };
        assert_eq!(string.string_utf16, Some(vec![0xd800]));
        assert!(matches!(string.kind, ExprKind::Str(_)));

        let StmtKind::Expression(pair) = stmt_kind("'\\uD83D\\uDE00'") else {
            panic!("expected string expression")
        };
        assert_eq!(pair.string_utf16, Some(vec![0xd83d, 0xde00]));
        let StmtKind::Expression(code_point) = stmt_kind("'\\u{1F600}'") else {
            panic!("expected string expression")
        };
        assert_eq!(code_point.string_utf16, pair.string_utf16);

        let ExprKind::Paren(object) = expr_kind("({ '\\uD800': 1 })") else {
            panic!("expected parenthesized object")
        };
        let ExprKind::Object(elements) = object.kind else {
            panic!("expected object expression")
        };
        let ObjElem::Prop(property) = &elements[0] else {
            panic!("expected property")
        };
        assert_eq!(property.key_utf16, Some(vec![0xd800]));
        let StmtKind::Class(class) = stmt_kind("class C { '\\uD800'() {} }") else {
            panic!("expected class")
        };
        let ClassMember::Method(method) = &class.members[0] else {
            panic!("expected class method")
        };
        assert_eq!(method.key_utf16, Some(vec![0xd800]));
    }

    #[test]
    fn template_cooked_and_raw_quasis_keep_distinct_utf16() {
        let ExprKind::Template(template) = expr_kind("`\\uD800\\n${1}\\uD83D\\uDE00`") else {
            panic!("expected template")
        };
        assert_eq!(
            template.quasis_utf16,
            vec![vec![0xd800, 10], vec![0xd83d, 0xde00]]
        );
        assert_eq!(
            template.raw_quasis_utf16,
            vec![
                "\\uD800\\n".encode_utf16().collect::<Vec<_>>(),
                "\\uD83D\\uDE00".encode_utf16().collect::<Vec<_>>()
            ]
        );
        let ExprKind::Template(code_point) = expr_kind("`\\u{1F600}`") else {
            panic!("expected template")
        };
        assert_eq!(code_point.quasis_utf16, vec![vec![0xd83d, 0xde00]]);
        assert_eq!(
            code_point.raw_quasis_utf16,
            vec!["\\u{1F600}".encode_utf16().collect::<Vec<_>>()]
        );
    }

    #[test]
    fn plain_template_is_one_quasi_no_exprs() {
        let e = expr_kind("`hello`");
        let ExprKind::Template(t) = e else {
            panic!("expected template")
        };
        assert_eq!(t.quasis, vec!["hello".to_string()]);
        assert!(t.exprs.is_empty());
    }

    #[test]
    fn nested_templates_parse_recursively() {
        let e = expr_kind("`a${`b${x}`}c`");
        let ExprKind::Template(outer) = e else {
            panic!("expected template")
        };
        assert_eq!(outer.quasis.len(), 2);
        assert_eq!(outer.exprs.len(), 1);
        let ExprKind::Template(inner) = &outer.exprs[0].kind else {
            panic!("expected nested template")
        };
        assert_eq!(inner.quasis, vec!["b".to_string(), "".to_string()]);
    }

    #[test]
    fn template_substitutions_hold_object_braces_and_calls() {
        let e = expr_kind("`x${ {k: 1}.k }y`");
        let ExprKind::Template(t) = e else {
            panic!("expected template")
        };
        assert!(matches!(t.exprs[0].kind, ExprKind::Member { .. }));
        assert!(parse_script("`v${f(a, b)}w`").is_ok());
    }

    #[test]
    fn tagged_templates_preserve_tag_and_template() {
        assert!(matches!(expr_kind("tag`x${1}y`"), ExprKind::TaggedTemplate { .. }));
    }

    #[test]
    fn unterminated_template_is_a_syntax_error() {
        assert!(err("`abc").message.contains("template"));
    }
}

#[cfg(test)]
mod construct_tests {
    use super::parse_script;
    use crate::ast::*;
    use crate::errors::SyntaxError;

    fn program(src: &str) -> Program {
        parse_script(src).expect("should parse")
    }

    fn stmt_kind(src: &str) -> StmtKind {
        let mut p = program(src);
        assert_eq!(p.body.len(), 1);
        p.body.remove(0).kind
    }

    fn expr_kind(src: &str) -> ExprKind {
        match stmt_kind(src) {
            StmtKind::Expression(e) => e.kind,
            other => panic!("expected expression statement, got {other:?}"),
        }
    }

    fn err(src: &str) -> SyntaxError {
        parse_script(src).expect_err("should be a SyntaxError")
    }

    // =========================================================================
    // Declarations
    // =========================================================================

    #[test]
    fn var_let_const_declarators_with_and_without_initializers() {
        let d = match stmt_kind("var a = 1, b;") {
            StmtKind::Var(v) => v,
            other => panic!("expected var decl, got {other:?}"),
        };
        assert_eq!(d.kind, VarKind::Var);
        assert_eq!(d.decls.len(), 2);
        assert!(d.decls[0].init.is_some());
        assert!(d.decls[1].init.is_none());
        assert!(matches!(
            d.decls[0].binding,
            Binding::Ident(Ident { ref name, .. }) if name == "a"
        ));
        assert!(matches!(stmt_kind("let x = 1;"), StmtKind::Var(v) if v.kind == VarKind::Let));
        assert!(matches!(stmt_kind("const c = 1;"), StmtKind::Var(v) if v.kind == VarKind::Const));
        assert!(err("const c;").message.contains("initializer"));
    }

    #[test]
    fn function_declarations_carry_flags_params_and_defaults() {
        let f = match stmt_kind("function f(a, b = 2, ...rest) { return a; }") {
            StmtKind::Function(f) => f,
            other => panic!("expected function decl, got {other:?}"),
        };
        assert_eq!(f.params.len(), 3);
        assert!(f.params[1].default.is_some());
        assert!(f.params[2].rest);
        assert!(!f.is_async && !f.is_generator);
        let g = match stmt_kind("function* gen() { yield 1; }") {
            StmtKind::Function(f) => f,
            other => panic!("expected generator, got {other:?}"),
        };
        assert!(g.is_generator);
        let a = match stmt_kind("async function af() {}") {
            StmtKind::Function(f) => f,
            other => panic!("expected async fn, got {other:?}"),
        };
        assert!(a.is_async);
        assert!(err("function () {}").message.contains("name"));
    }

    #[test]
    fn function_expressions_may_be_anonymous_and_named() {
        let e = expr_kind("(function named() {})");
        let ExprKind::Paren(inner) = e else {
            panic!("expected paren")
        };
        assert!(matches!(inner.kind, ExprKind::Function(f) if f.name.is_some()));
        let e = expr_kind("(function* () { yield 1; })");
        let ExprKind::Paren(inner) = e else {
            panic!("expected paren")
        };
        assert!(matches!(
            inner.kind,
            ExprKind::Function(f) if f.name.is_none() && f.is_generator
        ));
    }

    #[test]
    fn classes_capture_extends_constructor_static_get_set_computed_and_fields() {
        let c = match stmt_kind(
            "class A extends B {\n\
             constructor(x) { super(x); }\n\
             static make() { return new A(1); }\n\
             get size() { return 1; }\n\
             set size(v) {}\n\
             ['comp' + key]() {}\n\
             count = 0;\n\
             static tag;\n\
             }",
        ) {
            StmtKind::Class(c) => c,
            other => panic!("expected class, got {other:?}"),
        };
        assert_eq!(c.name.as_ref().expect("name").name, "A");
        assert!(c.super_class.is_some());
        let mut saw = (false, false, false, false, false, false); // ctor, static, get, set, computed, fields
        for m in &c.members {
            match m {
                ClassMember::Method(m) => match m.kind {
                    MethodKind::Constructor => {
                        saw.0 = true;
                        assert!(!m.is_static);
                    }
                    MethodKind::Get => saw.2 = true,
                    MethodKind::Set => saw.3 = true,
                    MethodKind::Method => {
                        if m.is_static {
                            saw.1 = true;
                        } else {
                            saw.4 = true;
                        }
                    }
                },
                ClassMember::Field(f) => {
                    saw.5 = true;
                    let is_tag = matches!(&f.key, PropKey::Ident(id) if id.name == "tag");
                    assert_eq!(
                        f.init.is_some(),
                        !is_tag,
                        "`count` initialized, bare `static tag;` not"
                    );
                }
                ClassMember::StaticBlock(_) => panic!("unexpected static block"),
            }
        }
        assert_eq!(saw, (true, true, true, true, true, true));
    }

    #[test]
    fn class_expressions_may_be_anonymous() {
        // (`let C = class …` would be a *declaration*, not an expression
        // statement, so the test assigns through an existing binding.)
        let e = expr_kind("C = class extends Object {};");
        let ExprKind::Assign { value, .. } = e else {
            panic!("expected assignment")
        };
        let ExprKind::Class(c) = value.kind else {
            panic!("expected class expr")
        };
        assert!(c.name.is_none());
        assert!(c.super_class.is_some());
        assert!(c.members.is_empty());
    }

    #[test]
    fn class_decl_requires_a_name_and_private_names_are_scoped() {
        assert!(err("class {}").message.contains("name"));
        assert!(parse_script(
            "class A { #p = 1; get #q() { return this.#p; } set #q(v) { this.#p = v; } }"
        )
        .is_ok());
        assert!(
            parse_script("class A { static get #q() { return 1; } static set #q(v) {} }").is_ok()
        );
        assert!(parse_script("class A { method() { return this.#later; } #later; }").is_ok());
        assert!(parse_script(
            "class A { method() { return class B { method() { return this.#outer; } }; } #outer; }"
        )
        .is_ok());
        assert!(
            parse_script("class A { #p; has(o) { return #p in o; } check = #p in this; }").is_ok()
        );
        assert!(err("class A { #p; #p; }")
            .message
            .contains("duplicate private"));
        assert!(err("class A { get #p() {} get #p() {} }")
            .message
            .contains("duplicate private"));
        assert!(err("class A { static get #p() {} set #p(v) {} }")
            .message
            .contains("duplicate private"));
        assert!(err("class A { #constructor; }")
            .message
            .contains("#constructor"));
        assert!(err("class A { #p; m() { delete this.#p; } }")
            .message
            .contains("cannot delete"));
        assert!(err("class A { #p; m() { delete (this.#p); } }")
            .message
            .contains("cannot delete"));
        assert!(err("class A extends B { #p; m() { return super.#p; } }")
            .message
            .contains("super cannot"));
        assert!(err("class A { m() { return this.#missing; } }")
            .message
            .contains("not declared"));
        assert!(err("this.#p").message.contains("outside class"));
        assert!(err("class A { #p; m() { return #p; } }")
            .message
            .contains("only valid with `in`"));
        assert!(matches!(
            stmt_kind("class A { static {} }"),
            StmtKind::Class(class) if matches!(class.members[0], ClassMember::StaticBlock(_))
        ));
    }

    #[test]
    fn private_names_have_distinct_ast_variants() {
        let program =
            parse_script("class A { #x; m(o) { return o.#x; } has(o) { return #x in o; } }")
                .unwrap();
        let StmtKind::Class(class) = &program.body[0].kind else {
            panic!("class")
        };
        let ClassMember::Field(field) = &class.members[0] else {
            panic!("field")
        };
        assert!(matches!(field.key, PropKey::Private(_)));
        let ClassMember::Method(method) = &class.members[1] else {
            panic!("method")
        };
        let StmtKind::Return(Some(expr)) = &method.body.body[0].kind else {
            panic!("return")
        };
        assert!(matches!(
            expr.kind,
            ExprKind::Member {
                prop: MemberProp::Private(_),
                ..
            }
        ));
        let ClassMember::Method(method) = &class.members[2] else {
            panic!("method")
        };
        let StmtKind::Return(Some(expr)) = &method.body.body[0].kind else {
            panic!("return")
        };
        assert!(
            matches!(expr.kind, ExprKind::Binary { op: BinOp::In, ref left, .. } if matches!(left.kind, ExprKind::PrivateName(_)))
        );
    }

    // =========================================================================
    // Control flow statements
    // =========================================================================

    #[test]
    fn dangling_else_binds_to_the_nearest_if() {
        let s = stmt_kind("if (a) if (b) ; else ;");
        let StmtKind::If { els, .. } = s else {
            panic!("expected if")
        };
        assert!(els.is_none(), "outer if must have no else");
    }

    #[test]
    fn while_and_do_while_shapes() {
        assert!(matches!(stmt_kind("while (x) ;"), StmtKind::While { .. }));
        let s = stmt_kind("do ; while (x)");
        let StmtKind::DoWhile { body, test } = s else {
            panic!("expected do-while")
        };
        assert!(matches!(body.kind, StmtKind::Empty));
        assert!(matches!(test.kind, ExprKind::Ident(_)));
    }

    #[test]
    fn c_style_for_with_all_parts_optional() {
        assert!(matches!(stmt_kind("for (;;) ;"), StmtKind::For { .. }));
        let s = stmt_kind("for (var i = 0, j = 1; i < j; i++, j--) ;");
        let StmtKind::For {
            init, test, update, ..
        } = s
        else {
            panic!("expected for")
        };
        assert!(matches!(
            init,
            Some(ForInit::Decl(d)) if d.decls.len() == 2
        ));
        assert!(test.is_some());
        // Update is a sequence expression `i++, j--`.
        assert!(matches!(
            update,
            Some(Expr { kind: ExprKind::Sequence(items), .. }) if items.len() == 2
        ));
    }

    #[test]
    fn switch_cases_store_fall_through_bodies_and_single_default() {
        let s = stmt_kind("switch (x) { case 1: a; b; case 2: ; default: c; }");
        let StmtKind::Switch { disc, cases } = s else {
            panic!("expected switch")
        };
        assert!(matches!(disc.kind, ExprKind::Ident(_)));
        assert_eq!(cases.len(), 3);
        assert_eq!(cases[0].body.len(), 2, "fall-through body kept in one case");
        assert!(cases[2].test.is_none());
        assert!(err("switch (x) { default: ; default: ; }")
            .message
            .contains("default"));
    }

    #[test]
    fn try_catch_finally_combinations() {
        let s = stmt_kind("try { a(); } catch (e) { log(e); } finally { done(); }");
        let StmtKind::Try(t) = s else {
            panic!("expected try")
        };
        assert!(matches!(t.catch, Some(CatchClause { param: Some(_), .. })));
        assert!(t.finally.is_some());
        let s = stmt_kind("try {} catch {}");
        let StmtKind::Try(t) = s else {
            panic!("expected try")
        };
        assert!(matches!(t.catch, Some(CatchClause { param: None, .. })));
        assert!(t.finally.is_none());
        assert!(err("try {}").message.contains("catch"));
    }

    #[test]
    fn throw_return_debugger_empty_block_statements() {
        assert!(matches!(
            stmt_kind("throw new Error('x');"),
            StmtKind::Throw(Expr {
                kind: ExprKind::New { .. },
                ..
            })
        ));
        assert!(matches!(
            stmt_kind("function f() { return 1; }"),
            StmtKind::Function(_)
        ));
        assert!(matches!(stmt_kind("debugger;"), StmtKind::Debugger));
        assert!(matches!(stmt_kind(";"), StmtKind::Empty));
        assert!(matches!(stmt_kind("{ ; ; }"), StmtKind::Block(_)));
        assert!(err("return 1;").message.contains("return"));
    }

    // =========================================================================
    // for-in / for-of left targets
    // =========================================================================

    #[test]
    fn for_in_of_accept_ident_member_and_single_decl_targets() {
        let s = stmt_kind("for (x in obj) ;");
        let StmtKind::ForIn { left, .. } = s else {
            panic!("expected for-in")
        };
        assert!(matches!(
            left,
            ForLeft::Target(Expr {
                kind: ExprKind::Ident(_),
                ..
            })
        ));

        let s = stmt_kind("for (const k of items) ;");
        let StmtKind::ForOf { left, .. } = s else {
            panic!("expected for-of")
        };
        assert!(matches!(left, ForLeft::Decl(_)));

        let s = stmt_kind("for (a.b of items) ;");
        let StmtKind::ForOf { left, .. } = s else {
            panic!("expected for-of")
        };
        assert!(matches!(
            left,
            ForLeft::Target(Expr {
                kind: ExprKind::Member { .. },
                ..
            })
        ));

        assert!(matches!(
            stmt_kind("for (var i in o) ;"),
            StmtKind::ForIn { .. }
        ));
        assert!(matches!(
            stmt_kind("for (let x of y) ;"),
            StmtKind::ForOf { .. }
        ));
    }

    #[test]
    fn for_in_of_reject_initializers_and_multi_declarators() {
        assert!(err("for (let i = 0 of x) ;")
            .message
            .contains("initializer"));
        assert!(err("for (let i, j of x) ;").message.contains("exactly one"));
        assert!(err("for (a, b of x) ;")
            .message
            .contains("invalid assignment target"));
    }

    // =========================================================================
    // Labeled statements & break/continue matching
    // =========================================================================

    #[test]
    fn labeled_break_matches_registered_loop_label() {
        let s = stmt_kind("outer: for (;;) break outer;");
        let StmtKind::Labeled { label, body } = s else {
            panic!("expected labeled")
        };
        assert_eq!(label.name, "outer");
        let StmtKind::For {
            body: loop_body, ..
        } = body.kind
        else {
            panic!("expected for")
        };
        assert!(matches!(
            loop_body.kind,
            StmtKind::Break { label: Some(Ident { ref name, .. }) } if name == "outer"
        ));
    }

    #[test]
    fn continue_targets_outer_loop_label_through_nested_label() {
        // `outer` is loop-labeled transitively through the inner labeled
        // loop, so `continue outer` from the innermost body resolves.
        assert!(parse_script("outer: while (1) { inner: for (;;) continue outer; }").is_ok());
    }

    #[test]
    fn continue_rejects_non_loop_labels_and_unregistered_labels() {
        // `lbl` goes out of label scope once its statement ends.
        assert!(err("lbl: x = 1;\nfor (;;) { continue lbl; }")
            .message
            .contains("undefined label"));
        // A live label whose statement is a block is not a continue target.
        assert!(err("lbl: { for (;;) { continue lbl; } }")
            .message
            .contains("not a loop"));
        assert!(err("for (;;) continue nope;")
            .message
            .contains("undefined label"));
        assert!(err("continue;").message.contains("loop"));
        assert!(err("break;").message.contains("loop or switch"));
        assert!(parse_script("switch (1) { case 1: break; }").is_ok());
    }

    #[test]
    fn chained_labels_inherit_loop_status() {
        assert!(parse_script("a: b: for (;;) continue a;").is_ok());
    }

    // =========================================================================
    // Expression constructs
    // =========================================================================

    #[test]
    fn this_super_and_member_chains() {
        assert!(matches!(expr_kind("this"), ExprKind::This));
        let e = expr_kind("super.method()");
        let ExprKind::Call { callee, .. } = e else {
            panic!("expected call")
        };
        let ExprKind::Member { obj, .. } = &callee.kind else {
            panic!("expected member callee")
        };
        assert!(
            matches!(obj.kind, ExprKind::Super),
            "`super.m()` callee object"
        );
        assert!(parse_script("super[key] = 1;").is_ok());
    }

    #[test]
    fn new_expressions_with_and_without_arguments() {
        assert!(matches!(expr_kind("new Foo()"), ExprKind::New { .. }));
        let e = expr_kind("new Foo");
        let ExprKind::New { args, .. } = e else {
            panic!("expected new")
        };
        assert!(args.is_empty());
        // `new a.b.c(x)` keeps the whole chain as the callee.
        let e = expr_kind("new a.b.c(x)");
        let ExprKind::New { callee, .. } = e else {
            panic!("expected new")
        };
        assert!(matches!(callee.kind, ExprKind::Member { .. }));
        // `new Foo().bar` — member on the constructed object.
        assert!(matches!(
            expr_kind("new Foo().bar"),
            ExprKind::Member { .. }
        ));
        assert!(matches!(expr_kind("new (Sized)()"), ExprKind::New { .. }));
    }

    #[test]
    fn optional_chaining_flags_each_link() {
        let e = expr_kind("a?.b");
        let ExprKind::Member { optional, obj, .. } = e else {
            panic!("expected member")
        };
        assert!(optional);
        assert!(matches!(obj.kind, ExprKind::Ident(_)));
        // `a?.b.c` — inner link optional, outer link plain.
        let e = expr_kind("a?.b.c");
        let ExprKind::Member { optional, obj, .. } = e else {
            panic!("expected member")
        };
        assert!(!optional);
        let ExprKind::Member {
            optional: inner_opt,
            ..
        } = obj.kind
        else {
            panic!("expected member")
        };
        assert!(inner_opt);
        assert!(matches!(
            expr_kind("f?.()"),
            ExprKind::Call { optional: true, .. }
        ));
        assert!(matches!(
            expr_kind("a?.[k]"),
            ExprKind::Member { optional: true, .. }
        ));
    }

    #[test]
    fn unary_and_update_operator_set() {
        for (src, op) in [
            ("-a", UnOp::Neg),
            ("+a", UnOp::Pos),
            ("!a", UnOp::Not),
            ("~a", UnOp::BitNot),
            ("typeof a", UnOp::Typeof),
            ("void a", UnOp::Void),
        ] {
            assert!(
                matches!(
                    expr_kind(src),
                    ExprKind::Unary { op: got, .. } if got == op
                ),
                "{src}"
            );
        }
        assert!(matches!(
            expr_kind("delete a.b"),
            ExprKind::Unary {
                op: UnOp::Delete,
                ..
            }
        ));
        assert!(matches!(
            expr_kind("++a"),
            ExprKind::Update { prefix: true, .. }
        ));
        assert!(matches!(
            expr_kind("a--"),
            ExprKind::Update { prefix: false, .. }
        ));
        assert!(err("++1").message.contains("target"));
    }

    #[test]
    fn object_literals_cover_shorthand_computed_methods_and_spread() {
        let e = expr_kind("({ a, b: 1, ['k']: 2, m() { return 3; }, get g() { return 4; }, set s(v) {}, async *ag() {}, ...rest })");
        let ExprKind::Paren(inner) = e else {
            panic!("expected paren")
        };
        let ExprKind::Object(elems) = inner.kind else {
            panic!("expected object")
        };
        assert_eq!(elems.len(), 8);
        assert!(
            matches!(&elems[0], ObjElem::Prop(p) if matches!(p.value, PropValue::Shorthand(_)))
        );
        assert!(matches!(&elems[2], ObjElem::Prop(p) if matches!(p.key, PropKey::Computed(_))));
        assert!(
            matches!(&elems[3], ObjElem::Prop(p) if matches!(p.value, PropValue::Method(ClassMethod { kind: MethodKind::Method, .. })))
        );
        assert!(
            matches!(&elems[4], ObjElem::Prop(p) if matches!(p.value, PropValue::Method(ClassMethod { kind: MethodKind::Get, .. })))
        );
        assert!(
            matches!(&elems[5], ObjElem::Prop(p) if matches!(p.value, PropValue::Method(ClassMethod { kind: MethodKind::Set, .. })))
        );
        let m = match &elems[5] {
            ObjElem::Prop(p) => match &p.value {
                PropValue::Method(m) => m,
                other => panic!("expected method, got {other:?}"),
            },
            other => panic!("expected prop, got {other:?}"),
        };
        assert_eq!(m.params.len(), 1, "setters keep their parameter");
        // `async *ag() {}` carries both modifiers.
        assert!(matches!(
            &elems[6],
            ObjElem::Prop(p) if matches!(
                &p.value,
                PropValue::Method(ClassMethod { is_async: true, is_generator: true, .. })
            )
        ));
        assert!(matches!(&elems[7], ObjElem::Spread(_)));
    }

    #[test]
    fn object_keys_accept_reserved_words_and_numbers() {
        assert!(parse_script("({ if: 1, class: 2, 0: 'zero' });").is_ok());
    }

    #[test]
    fn array_literals_cover_holes_spread_and_trailing_comma() {
        let e = expr_kind("[1, , 3]");
        let ExprKind::Array(items) = e else {
            panic!("expected array")
        };
        assert_eq!(items.len(), 3);
        assert!(items[0].is_some() && items[1].is_none() && items[2].is_some());
        let e = expr_kind("[1,]");
        let ExprKind::Array(items) = e else {
            panic!("expected array")
        };
        assert_eq!(items.len(), 1, "trailing comma adds no hole");
        let e = expr_kind("[...a, ...b]");
        let ExprKind::Array(items) = e else {
            panic!("expected array")
        };
        assert_eq!(items.len(), 2);
        assert!(matches!(
            items[0],
            Some(Expr {
                kind: ExprKind::Spread(_),
                ..
            })
        ));
        assert!(matches!(expr_kind("[]"), ExprKind::Array(items) if items.is_empty()));
    }

    #[test]
    fn sequence_statements_and_generator_yield_forms() {
        let e = expr_kind("a, b, c");
        let ExprKind::Sequence(items) = e else {
            panic!("expected sequence")
        };
        assert_eq!(items.len(), 3);

        let p = program("function* g() { yield; yield 1; yield* iter; }");
        let body = match &p.body[0].kind {
            StmtKind::Function(f) => &f.body.body,
            other => panic!("expected generator, got {other:?}"),
        };
        assert!(matches!(
            body[0].kind,
            StmtKind::Expression(Expr {
                kind: ExprKind::Yield {
                    argument: None,
                    delegate: false
                },
                ..
            })
        ));
        assert!(matches!(
            body[1].kind,
            StmtKind::Expression(Expr {
                kind: ExprKind::Yield {
                    argument: Some(_),
                    delegate: false
                },
                ..
            })
        ));
        assert!(matches!(
            body[2].kind,
            StmtKind::Expression(Expr {
                kind: ExprKind::Yield { delegate: true, .. },
                ..
            })
        ));
        assert!(matches!(expr_kind("yield"), ExprKind::Ident(id) if id.name == "yield"));
        assert!(err("yield 1;").message.contains("expected `;`"));
    }

    #[test]
    fn modules_are_deferred_with_a_clear_diagnostic() {
        assert!(err("import x from 'mod';").message.contains("deferred"));
        assert!(err("export default 1;").message.contains("deferred"));
        assert!(err("export { a };").message.contains("deferred"));
    }

    #[test]
    fn binding_patterns_parse_in_declarations_and_heads() {
        assert!(matches!(
            stmt_kind("var [a] = b;"),
            StmtKind::Var(VarDecl { decls, .. }) if matches!(decls[0].binding, Binding::Array(_))
        ));
        assert!(matches!(
            stmt_kind("var {a} = b;"),
            StmtKind::Var(VarDecl { decls, .. }) if matches!(decls[0].binding, Binding::Object(_))
        ));
        assert!(matches!(
            stmt_kind("function f([a]) {}"),
            StmtKind::Function(_)
        ));
        assert!(matches!(expr_kind("[a] = [1];"), ExprKind::Assign { .. }));
        assert!(matches!(
            expr_kind("({a} = obj);"),
            ExprKind::Paren(inner) if matches!(inner.kind, ExprKind::Assign { .. })
        ));
        assert!(matches!(
            stmt_kind("for ([a] of x) ;"),
            StmtKind::ForOf { .. }
        ));
    }

    #[test]
    fn positioned_errors_carry_line_and_column() {
        let e = err("let ok = 1;\nconst bad;");
        assert_eq!(e.line, 2);
        assert!(e.column > 1);
    }

    #[test]
    fn private_class_early_errors_do_not_reach_runtime() {
        for source in [
            "class C { async *#m() { var await; } }",
            "class C { async #m() { await: 1; } }",
            "class C { *#m() { void yield; } }",
            "class C { #x = () => arguments; }",
            "class C { #x = super(); }",
            "class C extends B { #m() { super(); } }",
            "class C { #x; m() { #x in () => {}; } }",
            "function* g(x = yield 1) {}",
            "async function* g(x = await 1) {}",
            "class A { *g() { (function yield() {}); } }",
        ] {
            assert!(parse_script(source).is_err(), "accepted {source}");
        }
        assert!(parse_script("class C extends B { constructor() { super(); } }").is_ok());
        assert!(parse_script("class C { #x = () => 1; m() { return this.#x; } }").is_ok());
        assert!(parse_script("async function outer() { function inner() { await(1); } }").is_ok());
    }
}
