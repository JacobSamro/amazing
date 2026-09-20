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
//! - Every statement kind listed in [`crate::ast`]; `import`/`export`
//!   rejected as "modules deferred".
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
//! - Modules (`import`/`export`) and top-level `await`.
//! - Destructuring assignment targets, computed pattern keys, and full
//!   iterator-closing/error semantics (basic declaration/parameter/for-head
//!   array/object patterns are implemented).
//! - Tagged templates. RegExp literal support uses parser-selected lexical goals.
//! - `with`, `new.target`, `import.meta`, labels-on-non-loop `continue`
//!   subtleties beyond loop/switch depth checks, strict-mode early errors,
//!   duplicate-binding checks, getter/setter arity validation, `for await`.
//! - Sloppy-mode `yield`/`await` as identifiers: `yield` is always the
//!   keyword here; `await` stays an identifier outside async functions.

use crate::ast::*;
use crate::errors::SyntaxError;
use crate::tokens::{Punct, SourceLocation, Span, Token, TokenKind};

/// Parses a full script (modules are deferred; see module docs).
pub fn parse_script(src: &str) -> Result<Program, SyntaxError> {
    Parser::new(src).parse_program()
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
    in_generator: usize,
    in_async: usize,
    /// Depth of `${ … }` substitutions currently being parsed. A
    /// `TemplatePart` token seen at depth > 0 is the enclosing template's
    /// continuation (the lexer absorbed the closing `}` into it), not the
    /// start of a tagged template.
    template_depth: usize,
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
            in_generator: 0,
            in_async: 0,
            template_depth: 0,
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
        matches!(self.cur().kind, TokenKind::Keyword(q) if q == k)
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
        matches!(&self.cur().kind, TokenKind::Ident(name) if name == s)
    }

    /// Consumes an identifier token (contextual words included, reserved
    /// words excluded — they are different token kinds).
    fn expect_ident(&mut self) -> Result<Ident, SyntaxError> {
        let t = self.cur().clone();
        match t.kind {
            TokenKind::Ident(name) => {
                self.bump();
                Ok(Ident::new(name, t.span))
            }
            _ => Err(self.err(format!("expected identifier, found {}", t.kind.describe()))),
        }
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

    // ----- program & statements -------------------------------------------

    fn parse_program(mut self) -> Result<Program, SyntaxError> {
        let start = self.cur_loc();
        let mut body = Vec::new();
        while !self.at_eof() {
            body.push(self.parse_stmt()?);
        }
        Ok(Program {
            span: self.span_from(start),
            body,
        })
    }

    fn parse_stmt(&mut self) -> Result<Stmt, SyntaxError> {
        let start = self.cur_loc();
        let kind = match &self.cur().kind {
            TokenKind::Punct(Punct::Lbrace) => return self.parse_block_stmt(),
            TokenKind::Punct(Punct::Semi) => {
                self.bump();
                StmtKind::Empty
            }
            TokenKind::Keyword(crate::tokens::Keyword::Var) => {
                let decl = self.parse_var_decl(VarKind::Var, false)?;
                self.semicolon()?;
                StmtKind::Var(decl)
            }
            TokenKind::Keyword(crate::tokens::Keyword::Const) => {
                let decl = self.parse_var_decl(VarKind::Const, false)?;
                self.semicolon()?;
                StmtKind::Var(decl)
            }
            TokenKind::Ident(name) if name == "let" && self.let_starts_decl() => {
                let decl = self.parse_var_decl(VarKind::Let, false)?;
                self.semicolon()?;
                StmtKind::Var(decl)
            }
            TokenKind::Ident(name) if name == "async" && self.nth_is_function(1) => {
                self.bump(); // `async`
                let mut f = self.parse_function_rest(true)?;
                f.is_async = true;
                if f.name.is_none() {
                    return Err(self.err_at(start, "function declarations must have a name"));
                }
                StmtKind::Function(Box::new(f))
            }
            TokenKind::Keyword(crate::tokens::Keyword::Function) => {
                let f = self.parse_function_rest(false)?;
                if f.name.is_none() {
                    return Err(self.err_at(start, "function declarations must have a name"));
                }
                StmtKind::Function(Box::new(f))
            }
            TokenKind::Keyword(crate::tokens::Keyword::Class) => {
                let c = self.parse_class_rest()?;
                let name = c
                    .name
                    .clone()
                    .ok_or_else(|| self.err_at(start, "class declarations must have a name"))?;
                StmtKind::Class(Box::new(ClassItem {
                    name: Some(name),
                    ..c
                }))
            }
            TokenKind::Keyword(crate::tokens::Keyword::If) => self.parse_if_stmt()?,
            TokenKind::Keyword(crate::tokens::Keyword::For) => self.parse_for_stmt()?,
            TokenKind::Keyword(crate::tokens::Keyword::While) => self.parse_while_stmt()?,
            TokenKind::Keyword(crate::tokens::Keyword::Do) => self.parse_do_while_stmt()?,
            TokenKind::Keyword(crate::tokens::Keyword::Switch) => self.parse_switch_stmt()?,
            TokenKind::Keyword(crate::tokens::Keyword::Try) => self.parse_try_stmt()?,
            TokenKind::Keyword(crate::tokens::Keyword::Throw) => self.parse_throw_stmt()?,
            TokenKind::Keyword(crate::tokens::Keyword::Return) => self.parse_return_stmt()?,
            TokenKind::Keyword(crate::tokens::Keyword::Break) => self.parse_break_stmt()?,
            TokenKind::Keyword(crate::tokens::Keyword::Continue) => self.parse_continue_stmt()?,
            TokenKind::Keyword(crate::tokens::Keyword::Debugger) => {
                self.bump();
                self.semicolon()?;
                StmtKind::Debugger
            }
            TokenKind::Keyword(crate::tokens::Keyword::Import)
            | TokenKind::Keyword(crate::tokens::Keyword::Export) => {
                return Err(self.err("modules (import/export) are deferred"));
            }
            TokenKind::Ident(name) if name == "import" || name == "export" => {
                return Err(self.err("modules (import/export) are deferred"));
            }
            _ => {
                // Labeled statement: `name :` (colon on any line? Spec
                // requires the colon to follow; keep it simple and accept).
                if matches!(self.cur().kind, TokenKind::Ident(_)) && self.nth_punct(1, Punct::Colon)
                {
                    return self.parse_labeled_stmt();
                }
                let expr = self.parse_expression_no_in(false)?;
                self.semicolon()?;
                StmtKind::Expression(expr)
            }
        };
        Ok(Stmt::new(kind, self.span_from(start)))
    }

    /// `let` begins a declaration only when followed by a binding start.
    /// `let[0]` and `let.x` stay expressions (destructuring deferred → the
    /// bracket/brace forms get a dedicated diagnostic).
    fn let_starts_decl(&self) -> bool {
        matches!(self.nth(1).kind, TokenKind::Ident(_))
            || matches!(self.nth(1).kind, TokenKind::Punct(Punct::Lbrace))
            || matches!(self.nth(1).kind, TokenKind::Punct(Punct::LBracket))
    }

    fn nth_is_function(&self, n: usize) -> bool {
        matches!(
            self.nth(n).kind,
            TokenKind::Keyword(crate::tokens::Keyword::Function)
        ) && !self.nth(n).newline_before
    }

    /// Parses one `var`/`let`/`const` declarator list. `allow_missing_init`
    /// relaxes the const-requires-initializer rule for for-heads, where the
    /// deciding context (`in`/`of` vs `;`) is only known afterwards; the
    /// C-style path re-checks explicitly.
    fn parse_var_decl(
        &mut self,
        kind: VarKind,
        allow_missing_init: bool,
    ) -> Result<VarDecl, SyntaxError> {
        self.bump(); // var/let/const keyword
        let mut decls = Vec::new();
        loop {
            let dstart = self.cur_loc();
            let binding = self.parse_binding()?;
            let init = if self.eat_punct(Punct::Eq) {
                Some(self.parse_assignment_expr()?)
            } else {
                None
            };
            if init.is_none() && kind == VarKind::Const && !allow_missing_init {
                return Err(self.err_at(dstart, "const declarations require an initializer"));
            }
            decls.push(Declarator {
                binding,
                init,
                span: self.span_from(dstart),
            });
            if !self.eat_punct(Punct::Comma) {
                break;
            }
        }
        Ok(VarDecl { kind, decls })
    }

    fn parse_binding(&mut self) -> Result<Binding, SyntaxError> {
        self.parse_binding_inner(false)
    }

    fn parse_binding_inner(&mut self, allow_default: bool) -> Result<Binding, SyntaxError> {
        let mut binding = match self.cur().kind {
            TokenKind::Punct(Punct::LBracket) => {
                self.bump();
                let mut elements = Vec::new();
                while !self.at_punct(Punct::RBracket) {
                    if self.eat_punct(Punct::Comma) {
                        elements.push(None);
                        continue;
                    }
                    if self.eat_punct(Punct::Ellipsis) {
                        elements.push(Some(Box::new(Binding::Rest(Box::new(
                            self.parse_binding_inner(false)?,
                        )))));
                        if self.eat_punct(Punct::Comma) && !self.at_punct(Punct::RBracket) {
                            return Err(self.err("rest element must be last in an array pattern"));
                        }
                        break;
                    }
                    elements.push(Some(Box::new(self.parse_binding_inner(true)?)));
                    if !self.eat_punct(Punct::Comma) {
                        break;
                    }
                }
                self.expect_punct(Punct::RBracket)?;
                Binding::Array(elements)
            }
            TokenKind::Punct(Punct::Lbrace) => {
                self.bump();
                let mut properties = Vec::new();
                while !self.at_punct(Punct::Rbrace) {
                    if self.eat_punct(Punct::Ellipsis) {
                        let rest = self.parse_binding_inner(false)?;
                        properties.push((
                            BindingKey::Static("...".to_string()),
                            Box::new(Binding::Rest(Box::new(rest))),
                        ));
                        if self.eat_punct(Punct::Comma) && !self.at_punct(Punct::Rbrace) {
                            return Err(self.err("rest property must be last in an object pattern"));
                        }
                        break;
                    }
                    let (key, shorthand_ident) = if self.eat_punct(Punct::LBracket) {
                        let expression = self.parse_assignment_expr()?;
                        self.expect_punct(Punct::RBracket)?;
                        (BindingKey::Computed(Box::new(expression)), None)
                    } else {
                        let identifier = self.prop_name()?;
                        (
                            BindingKey::Static(identifier.name.clone()),
                            Some(identifier),
                        )
                    };
                    let value = if self.eat_punct(Punct::Colon) {
                        self.parse_binding_inner(true)?
                    } else if let Some(identifier) = shorthand_ident {
                        let mut shorthand = Binding::Ident(identifier);
                        if self.eat_punct(Punct::Eq) {
                            let saved = self.no_in;
                            self.no_in = true;
                            let default = self.parse_assignment_expr()?;
                            self.no_in = saved;
                            shorthand = Binding::Default(Box::new(shorthand), Box::new(default));
                        }
                        shorthand
                    } else {
                        return Err(self.err("computed binding keys require a target"));
                    };
                    properties.push((key, Box::new(value)));
                    if !self.eat_punct(Punct::Comma) {
                        break;
                    }
                }
                self.expect_punct(Punct::Rbrace)?;
                Binding::Object(properties)
            }
            _ => Binding::Ident(self.expect_ident()?),
        };
        if allow_default && self.eat_punct(Punct::Eq) {
            let saved = self.no_in;
            self.no_in = true;
            let default = self.parse_assignment_expr()?;
            self.no_in = saved;
            binding = Binding::Default(Box::new(binding), Box::new(default));
        }
        Ok(binding)
    }

    fn parse_block_stmt(&mut self) -> Result<Stmt, SyntaxError> {
        let block = self.parse_block()?;
        let span = block.span;
        Ok(Stmt::new(StmtKind::Block(block), span))
    }

    fn parse_block(&mut self) -> Result<Block, SyntaxError> {
        let start = self.expect_punct(Punct::Lbrace)?;
        let mut body = Vec::new();
        while !self.at_punct(Punct::Rbrace) {
            if self.at_eof() {
                return Err(self.err("unterminated block: expected `}`"));
            }
            body.push(self.parse_stmt()?);
        }
        self.bump(); // }
        Ok(Block {
            body,
            span: self.span_from(start.start),
        })
    }

    fn parse_if_stmt(&mut self) -> Result<StmtKind, SyntaxError> {
        self.bump(); // if
        self.expect_punct(Punct::LParen)?;
        let cond = self.parse_expression_no_in(false)?;
        self.expect_punct(Punct::RParen)?;
        let then = Box::new(self.parse_stmt()?);
        let els = if self.eat_kw(crate::tokens::Keyword::Else) {
            Some(Box::new(self.parse_stmt()?))
        } else {
            None
        };
        Ok(StmtKind::If { cond, then, els })
    }

    fn parse_while_stmt(&mut self) -> Result<StmtKind, SyntaxError> {
        self.bump(); // while
        self.expect_punct(Punct::LParen)?;
        let test = self.parse_expression_no_in(false)?;
        self.expect_punct(Punct::RParen)?;
        self.loop_depth += 1;
        let body = Box::new(self.parse_stmt()?);
        self.loop_depth -= 1;
        Ok(StmtKind::While { test, body })
    }

    fn parse_do_while_stmt(&mut self) -> Result<StmtKind, SyntaxError> {
        self.bump(); // do
        self.loop_depth += 1;
        let body = Box::new(self.parse_stmt()?);
        self.loop_depth -= 1;
        if !self.eat_kw(crate::tokens::Keyword::While) {
            return Err(self.err("expected `while` after do-while body"));
        }
        self.expect_punct(Punct::LParen)?;
        let test = self.parse_expression_no_in(false)?;
        self.expect_punct(Punct::RParen)?;
        // `do … while (…) ` is followed by optional `;` (ASI rule 1).
        self.semicolon()?;
        Ok(StmtKind::DoWhile { body, test })
    }

    fn parse_for_stmt(&mut self) -> Result<StmtKind, SyntaxError> {
        self.bump(); // for
        self.expect_punct(Punct::LParen)?;
        // Empty init: C-style only.
        if self.at_punct(Punct::Semi) {
            self.bump(); // the `;` terminating the (absent) init clause
            return self.parse_for_c_style(None);
        }
        // Lexical declaration head.
        let decl_head = match &self.cur().kind {
            TokenKind::Keyword(crate::tokens::Keyword::Var) => Some(VarKind::Var),
            TokenKind::Keyword(crate::tokens::Keyword::Const) => Some(VarKind::Const),
            TokenKind::Ident(name) if name == "let" && self.let_starts_decl() => Some(VarKind::Let),
            _ => None,
        };
        if let Some(kind) = decl_head {
            let kw_start = self.cur_loc();
            // Missing initializers are allowed here because a for-in/of head
            // must not have one; the C-style tail below re-imposes the rule.
            let decl = self.parse_var_decl_no_in(kind)?;
            // for-in / for-of?
            if self.at_kw(crate::tokens::Keyword::In) || self.word("of") {
                let decl = validate_for_decl(&decl, kw_start)?;
                let stmt = if self.word("of") {
                    self.bump();
                    let right = self.parse_assignment_expr()?;
                    self.expect_punct(Punct::RParen)?;
                    self.loop_depth += 1;
                    let body = Box::new(self.parse_stmt()?);
                    self.loop_depth -= 1;
                    StmtKind::ForOf {
                        left: ForLeft::Decl(Box::new(decl)),
                        right,
                        body,
                    }
                } else {
                    self.bump(); // in
                    let right = self.parse_expression_no_in(true)?;
                    self.expect_punct(Punct::RParen)?;
                    self.loop_depth += 1;
                    let body = Box::new(self.parse_stmt()?);
                    self.loop_depth -= 1;
                    StmtKind::ForIn {
                        left: ForLeft::Decl(Box::new(decl)),
                        right,
                        body,
                    }
                };
                return Ok(stmt);
            }
            self.expect_punct(Punct::Semi)?;
            if kind == VarKind::Const && decl.decls.iter().any(|d| d.init.is_none()) {
                return Err(self.err_at(kw_start, "const declarations require an initializer"));
            }
            return self.parse_for_c_style(Some(ForInit::Decl(Box::new(decl))));
        }
        // Expression head: `for (x in y)` / `for (x of y)` / C-style.
        let expr = self.parse_expression_no_in(true)?;
        if self.at_kw(crate::tokens::Keyword::In) || self.word("of") {
            ensure_assignable(&expr).map_err(|e| self.err_at(self.cur_loc(), e.message))?;
            let stmt = if self.word("of") {
                self.bump();
                let right = self.parse_assignment_expr()?;
                self.expect_punct(Punct::RParen)?;
                self.loop_depth += 1;
                let body = Box::new(self.parse_stmt()?);
                self.loop_depth -= 1;
                StmtKind::ForOf {
                    left: ForLeft::Target(expr),
                    right,
                    body,
                }
            } else {
                self.bump(); // in
                let right = self.parse_expression_no_in(true)?;
                self.expect_punct(Punct::RParen)?;
                self.loop_depth += 1;
                let body = Box::new(self.parse_stmt()?);
                self.loop_depth -= 1;
                StmtKind::ForIn {
                    left: ForLeft::Target(expr),
                    right,
                    body,
                }
            };
            return Ok(stmt);
        }
        self.expect_punct(Punct::Semi)?;
        self.parse_for_c_style(Some(ForInit::Expr(expr)))
    }

    /// C-style tail after `init;`: `cond; update) body`.
    fn parse_for_c_style(&mut self, init: Option<ForInit>) -> Result<StmtKind, SyntaxError> {
        // Caller consumed the `;` after init when init exists.
        let test = if self.at_punct(Punct::Semi) {
            None
        } else {
            Some(self.parse_expression_no_in(true)?)
        };
        self.expect_punct(Punct::Semi)?;
        let update = if self.at_punct(Punct::RParen) {
            None
        } else {
            Some(self.parse_expression_no_in(true)?)
        };
        self.expect_punct(Punct::RParen)?;
        self.loop_depth += 1;
        let body = Box::new(self.parse_stmt()?);
        self.loop_depth -= 1;
        Ok(StmtKind::For {
            init,
            test,
            update,
            body,
        })
    }

    /// Like [`Parser::parse_var_decl`] but parses initializers with the
    /// `no-in` restriction active (for-heads) and missing initializers
    /// permitted (the for-in/of head re-checks).
    fn parse_var_decl_no_in(&mut self, kind: VarKind) -> Result<VarDecl, SyntaxError> {
        let saved = self.no_in;
        self.no_in = true;
        let r = self.parse_var_decl(kind, true);
        self.no_in = saved;
        r
    }

    fn parse_switch_stmt(&mut self) -> Result<StmtKind, SyntaxError> {
        self.bump(); // switch
        self.expect_punct(Punct::LParen)?;
        let disc = self.parse_expression_no_in(false)?;
        self.expect_punct(Punct::RParen)?;
        self.expect_punct(Punct::Lbrace)?;
        let mut cases = Vec::new();
        let mut seen_default = false;
        self.switch_depth += 1;
        while !self.at_punct(Punct::Rbrace) {
            if self.at_eof() {
                return Err(self.err("unterminated switch: expected `}`"));
            }
            let cstart = self.cur_loc();
            let test = if self.eat_kw(crate::tokens::Keyword::Case) {
                let e = self.parse_expression_no_in(false)?;
                Some(e)
            } else if self.eat_kw(crate::tokens::Keyword::Default) {
                if seen_default {
                    return Err(self.err("a switch may have only one `default` clause"));
                }
                seen_default = true;
                None
            } else {
                return Err(self.err("expected `case` or `default` in switch body"));
            };
            self.expect_punct(Punct::Colon)?;
            let mut body = Vec::new();
            while !self.at_punct(Punct::Rbrace)
                && !self.at_kw(crate::tokens::Keyword::Case)
                && !self.at_kw(crate::tokens::Keyword::Default)
            {
                body.push(self.parse_stmt()?);
            }
            cases.push(SwitchCase {
                test,
                body,
                span: self.span_from(cstart),
            });
        }
        self.switch_depth -= 1;
        self.bump(); // }
        Ok(StmtKind::Switch { disc, cases })
    }

    fn parse_try_stmt(&mut self) -> Result<StmtKind, SyntaxError> {
        self.bump(); // try
        let block = self.parse_block()?;
        let mut catch = None;
        let mut finally = None;
        if self.eat_kw(crate::tokens::Keyword::Catch) {
            let param = if self.eat_punct(Punct::LParen) {
                let p = self.expect_ident()?;
                self.expect_punct(Punct::RParen)?;
                Some(p)
            } else {
                None
            };
            let body = self.parse_block()?;
            catch = Some(CatchClause { param, body });
        }
        if self.eat_kw(crate::tokens::Keyword::Finally) {
            finally = Some(self.parse_block()?);
        }
        if catch.is_none() && finally.is_none() {
            return Err(self.err("try statement requires `catch` or `finally`"));
        }
        Ok(StmtKind::Try(TryStmt {
            block,
            catch,
            finally,
        }))
    }

    fn parse_throw_stmt(&mut self) -> Result<StmtKind, SyntaxError> {
        self.bump(); // throw
        if self.cur().newline_before {
            return Err(self.err("newline not allowed after `throw`"));
        }
        let expr = self.parse_expression_no_in(false)?;
        self.semicolon()?;
        Ok(StmtKind::Throw(expr))
    }

    fn parse_return_stmt(&mut self) -> Result<StmtKind, SyntaxError> {
        self.bump(); // return
        if self.in_function == 0 {
            return Err(self.err("`return` outside of a function"));
        }
        let arg = if self.starts_expression() && !self.cur().newline_before {
            Some(self.parse_expression_no_in(false)?)
        } else {
            None
        };
        self.semicolon()?;
        Ok(StmtKind::Return(arg))
    }

    fn parse_break_stmt(&mut self) -> Result<StmtKind, SyntaxError> {
        self.bump(); // break
        let label = self.opt_label_ref()?;
        match &label {
            Some(id) => {
                if !self.labels.iter().any(|l| l.name == id.name) {
                    return Err(self.err(format!("undefined label `{}`", id.name)));
                }
            }
            None => {
                if self.loop_depth == 0 && self.switch_depth == 0 {
                    return Err(self.err("`break` not inside a loop or switch"));
                }
            }
        }
        self.semicolon()?;
        Ok(StmtKind::Break { label })
    }

    fn parse_continue_stmt(&mut self) -> Result<StmtKind, SyntaxError> {
        self.bump(); // continue
        let label = self.opt_label_ref()?;
        match &label {
            Some(id) => {
                if !self.labels.iter().any(|l| l.name == id.name) {
                    return Err(self.err(format!("undefined label `{}`", id.name)));
                }
                if !self.labels.iter().any(|l| l.name == id.name && l.is_loop) {
                    return Err(self.err(format!(
                        "`continue` target `{}` is not a loop label",
                        id.name
                    )));
                }
            }
            None => {
                if self.loop_depth == 0 {
                    return Err(self.err("`continue` not inside a loop"));
                }
            }
        }
        self.semicolon()?;
        Ok(StmtKind::Continue { label })
    }

    /// Label reference after break/continue: an identifier on the same line.
    fn opt_label_ref(&mut self) -> Result<Option<Ident>, SyntaxError> {
        if matches!(self.cur().kind, TokenKind::Ident(_)) && !self.cur().newline_before {
            Ok(Some(self.expect_ident()?))
        } else {
            Ok(None)
        }
    }

    fn parse_labeled_stmt(&mut self) -> Result<Stmt, SyntaxError> {
        let label = self.expect_ident()?;
        self.expect_punct(Punct::Colon)?;
        // Loop-ness must be known *before* the body parses (so an innermost
        // `continue outer:` resolves), and it holds through chains of labels
        // (`outer: inner: for …`). Decide it by probing tokens: skip any
        // further `ident :` pairs and see whether a loop keyword follows.
        let mut probe = 0usize;
        let is_loop = loop {
            if matches!(self.nth(probe).kind, TokenKind::Ident(_))
                && self.nth_punct(probe + 1, Punct::Colon)
            {
                probe += 2;
            } else {
                break matches!(
                    self.nth(probe).kind,
                    TokenKind::Keyword(
                        crate::tokens::Keyword::For
                            | crate::tokens::Keyword::While
                            | crate::tokens::Keyword::Do
                    )
                );
            }
        };
        self.labels.push(LabelCtx {
            name: label.name.clone(),
            is_loop,
        });
        let body = self.parse_stmt()?;
        let stmt = Stmt::new(
            StmtKind::Labeled {
                label: label.clone(),
                body: Box::new(body),
            },
            label.span,
        );
        self.labels.pop();
        Ok(stmt)
    }

    /// Does the current token plausibly start an expression? Used by the
    /// restricted-production checks (`return`/`yield` operand suppression).
    fn starts_expression(&self) -> bool {
        match &self.cur().kind {
            TokenKind::Punct(Punct::Slash | Punct::SlashEq)
            | TokenKind::RegExp { .. }
            | TokenKind::Numeric(_)
            | TokenKind::Str(_)
            | TokenKind::TemplatePart { .. }
            | TokenKind::Ident(_)
            | TokenKind::Punct(
                Punct::LParen
                | Punct::LBracket
                | Punct::Lbrace
                | Punct::Bang
                | Punct::Tilde
                | Punct::Plus
                | Punct::Minus
                | Punct::Inc
                | Punct::Dec,
            ) => true,
            TokenKind::Keyword(k) => matches!(
                k,
                crate::tokens::Keyword::This
                    | crate::tokens::Keyword::Null
                    | crate::tokens::Keyword::True
                    | crate::tokens::Keyword::False
                    | crate::tokens::Keyword::New
                    | crate::tokens::Keyword::Function
                    | crate::tokens::Keyword::Class
                    | crate::tokens::Keyword::Super
                    | crate::tokens::Keyword::Typeof
                    | crate::tokens::Keyword::Void
                    | crate::tokens::Keyword::Delete
                    | crate::tokens::Keyword::Yield
            ),
            _ => false,
        }
    }

    // ----- functions -------------------------------------------------------

    /// Parses everything after `async`?/`function` — the keyword itself is
    /// consumed here. Callers fix up `is_async`.
    fn parse_function_rest(&mut self, is_async: bool) -> Result<FunctionItem, SyntaxError> {
        self.bump(); // function
        let is_generator = self.eat_punct(Punct::Star);
        let name = if matches!(self.cur().kind, TokenKind::Ident(_)) {
            Some(self.expect_ident()?)
        } else {
            None
        };
        let params = self.parse_params()?;
        self.in_function += 1;
        self.in_generator += usize::from(is_generator);
        self.in_async += usize::from(is_async);
        let body = self.parse_block();
        self.in_async -= usize::from(is_async);
        self.in_generator -= usize::from(is_generator);
        self.in_function -= 1;
        Ok(FunctionItem {
            name,
            params,
            body: body?,
            is_async,
            is_generator,
        })
    }

    fn parse_params(&mut self) -> Result<Vec<Param>, SyntaxError> {
        self.expect_punct(Punct::LParen)?;
        let mut params = Vec::new();
        if !self.at_punct(Punct::RParen) {
            loop {
                let pstart = self.cur_loc();
                let rest = self.eat_punct(Punct::Ellipsis);
                let binding = self.parse_binding()?;
                let default = if !rest && self.eat_punct(Punct::Eq) {
                    let saved = self.no_in;
                    self.no_in = true;
                    let d = self.parse_assignment_expr();
                    self.no_in = saved;
                    Some(d?)
                } else {
                    None
                };
                params.push(Param {
                    binding,
                    default,
                    rest,
                    span: self.span_from(pstart),
                });
                if !self.eat_punct(Punct::Comma) {
                    break;
                }
            }
        }
        self.expect_punct(Punct::RParen)?;
        Ok(params)
    }

    /// Runs `f` with the function-body context flags applied; used by every
    /// callable body so nested `return`/`yield`/`await` validity works.
    fn with_fn_context<T>(
        &mut self,
        is_async: bool,
        is_generator: bool,
        f: impl FnOnce(&mut Self) -> Result<T, SyntaxError>,
    ) -> Result<T, SyntaxError> {
        self.in_function += 1;
        self.in_async += usize::from(is_async);
        self.in_generator += usize::from(is_generator);
        let r = f(self);
        self.in_generator -= usize::from(is_generator);
        self.in_async -= usize::from(is_async);
        self.in_function -= 1;
        r
    }

    // ----- classes -----------------------------------------------------------

    /// Parses everything after the `class` keyword (consumed here).
    fn parse_class_rest(&mut self) -> Result<ClassItem, SyntaxError> {
        self.bump(); // class
        let name = if matches!(self.cur().kind, TokenKind::Ident(_)) {
            Some(self.expect_ident()?)
        } else {
            None
        };
        let super_class = if self.eat_kw(crate::tokens::Keyword::Extends) {
            Some(self.parse_assignment_expr()?)
        } else {
            None
        };
        self.expect_punct(Punct::Lbrace)?;
        let mut members = Vec::new();
        while !self.at_punct(Punct::Rbrace) {
            if self.at_eof() {
                return Err(self.err("unterminated class body: expected `}`"));
            }
            members.push(self.parse_class_member()?);
        }
        self.bump(); // }
        Ok(ClassItem {
            name,
            super_class,
            members,
        })
    }

    /// Can token `n` begin a class-member key? Guards contextual modifiers
    /// (`static`, `get`, `set`, `async`) from being eaten when they are
    /// themselves keys (`static() {}`, `get: 1`, `set = 3`, `static;`).
    fn nth_starts_member_key(&self, n: usize) -> bool {
        match self.nth(n).kind {
            TokenKind::Ident(_)
            | TokenKind::Keyword(_)
            | TokenKind::Str(_)
            | TokenKind::Numeric(_) => true,
            TokenKind::Punct(Punct::LBracket) | TokenKind::Punct(Punct::Star) => true,
            _ => false,
        }
    }

    fn parse_class_member(&mut self) -> Result<ClassMember, SyntaxError> {
        let start = self.cur_loc();
        let is_static = self.word("static") && self.nth_starts_member_key(1);
        if is_static {
            self.bump();
        }
        // Modifier order is fixed by the grammar: `async` precedes `*`
        // (`async *ag() {}`; bare `*g()` and `async m()` are the other combos).
        let is_async =
            self.word("async") && !self.cur().newline_before && self.nth_starts_member_key(1);
        if is_async {
            self.bump();
        }
        let is_generator = self.eat_punct(Punct::Star);
        let mut kind = MethodKind::Method;
        if !is_generator && !is_async {
            if self.word("get") && self.nth_starts_member_key(1) {
                self.bump();
                kind = MethodKind::Get;
            } else if self.word("set") && self.nth_starts_member_key(1) {
                self.bump();
                kind = MethodKind::Set;
            }
        }
        let key = self.parse_prop_key()?;
        if self.at_punct(Punct::LParen) {
            let params = self.parse_params()?;
            let actual_kind = if kind == MethodKind::Method
                && !is_static
                && prop_key_name(&key) == Some("constructor")
            {
                MethodKind::Constructor
            } else {
                kind
            };
            let body = self.with_fn_context(is_async, is_generator, |p| p.parse_block())?;
            return Ok(ClassMember::Method(ClassMethod {
                key,
                kind: actual_kind,
                is_static,
                is_generator,
                is_async,
                params,
                body,
                span: self.span_from(start),
            }));
        }
        // Field definition (storage only).
        if is_generator || is_async || kind != MethodKind::Method {
            return Err(self.err(
                "only methods may use `*`, `async`, `get`, or `set` without a parameter list",
            ));
        }
        let init = if self.eat_punct(Punct::Eq) {
            let saved = self.no_in;
            self.no_in = true;
            let e = self.parse_assignment_expr();
            self.no_in = saved;
            Some(e?)
        } else {
            None
        };
        // Field separator: `;`, `}`, or ASI (newline). Anything else is an
        // error (`{ a = 1 b = 2 }` is invalid).
        if !self.at_punct(Punct::Rbrace) && !self.cur().newline_before {
            self.expect_punct(Punct::Semi)?;
        } else {
            self.eat_punct(Punct::Semi);
        }
        Ok(ClassMember::Field(ClassField {
            key,
            is_static,
            init,
            span: self.span_from(start),
        }))
    }

    /// Property key: identifier (reserved words included), string, numeric
    /// literal, or `[expr]`.
    fn parse_prop_key(&mut self) -> Result<PropKey, SyntaxError> {
        match self.cur().kind.clone() {
            TokenKind::Ident(_) => Ok(PropKey::Ident(self.expect_ident()?)),
            TokenKind::Keyword(k) => {
                let t = self.bump();
                Ok(PropKey::Ident(Ident::new(k.spelling(), t.span)))
            }
            TokenKind::Str(s) => {
                self.bump();
                Ok(PropKey::Str(s))
            }
            TokenKind::Numeric(n) => {
                self.bump();
                Ok(PropKey::Num(n))
            }
            TokenKind::Punct(Punct::LBracket) => {
                self.bump();
                let e = self.parse_assignment_expr()?;
                self.expect_punct(Punct::RBracket)?;
                Ok(PropKey::Computed(Box::new(e)))
            }
            other => Err(self.err(format!(
                "expected property name, found {}",
                other.describe()
            ))),
        }
    }
}

/// Helper: name of an identifier property key (constructor detection).
fn prop_key_name(key: &PropKey) -> Option<&str> {
    match key {
        PropKey::Ident(id) => Some(id.name.as_str()),
        _ => None,
    }
}

/// Validates a for-in/of lexical head: exactly one declarator, no
/// initializer (`for (const x of y)` ok, `for (let i = 0 of y)` rejected).
fn validate_for_decl(decl: &VarDecl, at: SourceLocation) -> Result<VarDecl, SyntaxError> {
    if decl.decls.len() != 1 {
        return Err(SyntaxError::new(
            "for-in/of head must declare exactly one binding",
            at.line,
            at.column,
        ));
    }
    if decl.decls[0].init.is_some() {
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
        ExprKind::Ident(_) | ExprKind::Member { .. } => Ok(()),
        ExprKind::Paren(inner) => ensure_assignable(inner),
        ExprKind::Assign {
            op: AssignOp::Assign,
            target,
            ..
        } => ensure_assignable(target),
        ExprKind::Array(elements) => {
            for element in elements.iter().flatten() {
                if let ExprKind::Spread(inner) = &element.kind {
                    ensure_assignable(inner)?;
                } else {
                    ensure_assignable(element)?;
                }
            }
            Ok(())
        }
        ExprKind::Object(elements) => {
            for element in elements {
                match element {
                    ObjElem::Spread(inner) => ensure_assignable(inner)?,
                    ObjElem::Prop(property) => match &property.value {
                        PropValue::Shorthand(_) => {}
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

impl Parser {
    // ----- expressions: entry points ----------------------------------------

    /// `Expression` (comma sequences allowed) with a configurable no-in
    /// restriction (spec §14.10 "it is a Syntax Error if … Contains in").
    fn parse_expression_no_in(&mut self, no_in: bool) -> Result<Expr, SyntaxError> {
        let saved = self.no_in;
        self.no_in = no_in;
        let r = self.parse_sequence();
        self.no_in = saved;
        r
    }

    /// Full expression, sequences allowed, `in` allowed.
    fn parse_expression(&mut self) -> Result<Expr, SyntaxError> {
        self.parse_expression_no_in(false)
    }

    fn parse_sequence(&mut self) -> Result<Expr, SyntaxError> {
        let start = self.cur_loc();
        let first = self.parse_assignment_expr()?;
        if !self.at_punct(Punct::Comma) {
            return Ok(first);
        }
        let mut items = vec![first];
        while self.eat_punct(Punct::Comma) {
            // A trailing comma (`(a,)`) makes the next assignment parse fail
            // with a positioned "unexpected …" diagnostic.
            items.push(self.parse_assignment_expr()?);
        }
        Ok(Expr::new(ExprKind::Sequence(items), self.span_from(start)))
    }

    /// `AssignmentExpression`: conditional, or an assignment whose left side
    /// is validated as assignable (right-associative).
    fn parse_assignment_expr(&mut self) -> Result<Expr, SyntaxError> {
        let start = self.cur_loc();
        let left = self.parse_conditional_expr()?;
        let op = match self.cur().kind {
            TokenKind::Punct(p) => assign_op_of(p),
            _ => None,
        };
        if let Some(op) = op {
            ensure_assignable(&left).map_err(|e| self.err(e.message))?;
            self.bump(); // operator
            let value = self.parse_assignment_expr()?;
            Ok(Expr::new(
                ExprKind::Assign {
                    op,
                    target: Box::new(left),
                    value: Box::new(value),
                },
                self.span_from(start),
            ))
        } else {
            Ok(left)
        }
    }

    fn parse_conditional_expr(&mut self) -> Result<Expr, SyntaxError> {
        let start = self.cur_loc();
        let test = self.parse_binary(MIN_PREC)?;
        if !self.at_punct(Punct::Question) {
            return Ok(test);
        }
        self.bump(); // ?
                     // The middle operand allows `in` even inside for-heads.
        let saved = self.no_in;
        self.no_in = false;
        let consequent = self.parse_assignment_expr();
        self.no_in = saved;
        let consequent = consequent?;
        self.expect_punct(Punct::Colon)?;
        let alternate = self.parse_assignment_expr()?;
        Ok(Expr::new(
            ExprKind::Conditional {
                test: Box::new(test),
                consequent: Box::new(consequent),
                alternate: Box::new(alternate),
            },
            self.span_from(start),
        ))
    }

    // ----- precedence climbing -----------------------------------------------

    /// Precedence-climbing loop over all binary and short-circuit operators.
    /// `saw_coalesce` / `saw_logical` enforce the spec rule that `??`
    /// cannot be mixed with `||`/`&&` without parentheses; the flags stay
    /// local to one unparenthesized chain because each *operand* recursion
    /// starts fresh.
    fn parse_binary(&mut self, min_prec: u8) -> Result<Expr, SyntaxError> {
        let start = self.cur_loc();
        let mut left = self.parse_unary()?;
        let mut saw_coalesce = false;
        let mut saw_logical = false;
        loop {
            let (prec, kind) = match self.peek_binop() {
                Some(pair) => pair,
                None => break,
            };
            if prec < min_prec {
                break;
            }
            if kind == BinKind::Arithmetic(BinOp::In) && self.no_in {
                break;
            }
            // Unary operators cannot directly feed `**` (the grammar gives
            // ExponentiationExpression no unary left operand), so
            // `-2 ** 2` is a SyntaxError while `(-2) ** 2` parses.
            if kind == BinKind::Arithmetic(BinOp::Pow)
                && matches!(left.kind, ExprKind::Unary { .. } | ExprKind::Update { .. })
            {
                return Err(self.err("unary operand needs parentheses before `**`"));
            }
            match kind {
                BinKind::Logical(LogicalOp::Coalesce) => {
                    if saw_logical {
                        return Err(
                            self.err("`??` cannot be mixed with `||`/`&&` without parentheses")
                        );
                    }
                    saw_coalesce = true;
                }
                BinKind::Logical(_) => {
                    if saw_coalesce {
                        return Err(
                            self.err("`||`/`&&` cannot be mixed with `??` without parentheses")
                        );
                    }
                    saw_logical = true;
                }
                BinKind::Arithmetic(_) => {}
            }
            self.bump(); // operator
                         // `**` is right-associative; everything else left-assoc.
            let next_min = if kind == BinKind::Arithmetic(BinOp::Pow) {
                prec
            } else {
                prec + 1
            };
            let right = self.parse_binary(next_min)?;
            // The flags above only see operators at the *current* precedence
            // level; `a ?? b || c` slips past them because the right-operand
            // recursion (min_prec 4) absorbs `||` silently. A logical op
            // absorbed there is always the root of the right subtree, so one
            // root check closes the hole. Parenthesized operands surface as
            // [`ExprKind::Paren`] and are correctly allowed.
            if kind == BinKind::Logical(LogicalOp::Coalesce)
                && matches!(
                    right.kind,
                    ExprKind::Logical {
                        op: LogicalOp::And | LogicalOp::Or,
                        ..
                    }
                )
            {
                return Err(self.err("`??` cannot be mixed with `||`/`&&` without parentheses"));
            }
            let span = Span::new(start, right.span.end);
            left = match kind {
                BinKind::Arithmetic(op) => Expr::new(
                    ExprKind::Binary {
                        op,
                        left: Box::new(left),
                        right: Box::new(right),
                    },
                    span,
                ),
                BinKind::Logical(op) => Expr::new(
                    ExprKind::Logical {
                        op,
                        left: Box::new(left),
                        right: Box::new(right),
                    },
                    span,
                ),
            };
        }
        Ok(left)
    }

    fn peek_binop(&self) -> Option<(u8, BinKind)> {
        use crate::tokens::Keyword as K;
        use BinKind::{Arithmetic as A, Logical as L};
        use BinOp::*;
        use LogicalOp as Lo;
        let pair = match self.cur().kind {
            TokenKind::Punct(Punct::Nullish) => (3, L(Lo::Coalesce)),
            TokenKind::Punct(Punct::Or) => (4, L(Lo::Or)),
            TokenKind::Punct(Punct::And) => (5, L(Lo::And)),
            TokenKind::Punct(Punct::BitOr) => (6, A(BitOr)),
            TokenKind::Punct(Punct::BitXor) => (7, A(BitXor)),
            TokenKind::Punct(Punct::BitAnd) => (8, A(BitAnd)),
            TokenKind::Punct(Punct::EqEq) => (9, A(EqEq)),
            TokenKind::Punct(Punct::NotEq) => (9, A(NotEq)),
            TokenKind::Punct(Punct::EqEqEq) => (9, A(StrictEq)),
            TokenKind::Punct(Punct::NotEqEq) => (9, A(StrictNeq)),
            TokenKind::Punct(Punct::Lt) => (10, A(Lt)),
            TokenKind::Punct(Punct::Gt) => (10, A(Gt)),
            TokenKind::Punct(Punct::Le) => (10, A(Le)),
            TokenKind::Punct(Punct::Ge) => (10, A(Ge)),
            TokenKind::Keyword(K::InstanceOf) => (10, A(InstanceOf)),
            TokenKind::Keyword(K::In) => (10, A(In)),
            TokenKind::Punct(Punct::Shl) => (11, A(Shl)),
            TokenKind::Punct(Punct::Shr) => (11, A(Shr)),
            TokenKind::Punct(Punct::UShr) => (11, A(UShr)),
            TokenKind::Punct(Punct::Plus) => (12, A(Add)),
            TokenKind::Punct(Punct::Minus) => (12, A(Sub)),
            TokenKind::Punct(Punct::Star) => (13, A(Mul)),
            TokenKind::Punct(Punct::Slash) => (13, A(Div)),
            TokenKind::Punct(Punct::Percent) => (13, A(Rem)),
            TokenKind::Punct(Punct::Pow) => (14, A(Pow)),
            _ => return None,
        };
        Some(pair)
    }

    // ----- unary & postfix -----------------------------------------------------

    fn parse_unary(&mut self) -> Result<Expr, SyntaxError> {
        let start = self.cur_loc();
        use crate::tokens::Keyword as K;
        // Prefix update.
        if self.at_punct(Punct::Inc) || self.at_punct(Punct::Dec) {
            let op = if self.eat_punct(Punct::Inc) {
                UpdateOp::Inc
            } else {
                self.bump();
                UpdateOp::Dec
            };
            let operand = self.parse_unary()?;
            ensure_assignable(&operand).map_err(|e| self.err_at(start, e.message))?;
            return Ok(Expr::new(
                ExprKind::Update {
                    op,
                    prefix: true,
                    target: Box::new(operand),
                },
                self.span_from(start),
            ));
        }
        let simple_unop = match self.cur().kind {
            TokenKind::Punct(Punct::Bang) => Some(UnOp::Not),
            TokenKind::Punct(Punct::Tilde) => Some(UnOp::BitNot),
            TokenKind::Punct(Punct::Plus) => Some(UnOp::Pos),
            TokenKind::Punct(Punct::Minus) => Some(UnOp::Neg),
            TokenKind::Keyword(K::Typeof) => Some(UnOp::Typeof),
            TokenKind::Keyword(K::Void) => Some(UnOp::Void),
            TokenKind::Keyword(K::Delete) => Some(UnOp::Delete),
            _ => None,
        };
        if let Some(op) = simple_unop {
            self.bump();
            let operand = self.parse_unary()?;
            return Ok(Expr::new(
                ExprKind::Unary {
                    op,
                    operand: Box::new(operand),
                },
                self.span_from(start),
            ));
        }
        // `await` (contextual: only inside async functions).
        if self.word("await") && self.in_async > 0 && !self.cur().newline_before {
            self.bump();
            let operand = self.parse_unary()?;
            return Ok(Expr::new(
                ExprKind::Unary {
                    op: UnOp::Await,
                    operand: Box::new(operand),
                },
                self.span_from(start),
            ));
        }
        self.parse_postfix()
    }

    /// Postfix `++`/`--` — postfix only when on the same line (ASI's
    /// restricted production; a newline ends the statement instead).
    fn parse_postfix(&mut self) -> Result<Expr, SyntaxError> {
        let start = self.cur_loc();
        let target = self.parse_lhs()?;
        let is_update = matches!(self.cur().kind, TokenKind::Punct(Punct::Inc | Punct::Dec))
            && !self.cur().newline_before;
        if !is_update {
            return Ok(target);
        }
        ensure_assignable(&target).map_err(|e| self.err(e.message))?;
        let op = if self.eat_punct(Punct::Inc) {
            UpdateOp::Inc
        } else {
            self.bump();
            UpdateOp::Dec
        };
        Ok(Expr::new(
            ExprKind::Update {
                op,
                prefix: false,
                target: Box::new(target),
            },
            self.span_from(start),
        ))
    }

    // ----- call / member / new chains --------------------------------------------

    fn parse_lhs(&mut self) -> Result<Expr, SyntaxError> {
        let start = self.cur_loc();
        if self.at_kw(crate::tokens::Keyword::New) {
            self.bump(); // new
            let callee = self.parse_member_chain(false)?;
            let args = if self.at_punct(Punct::LParen) {
                self.parse_args()?
            } else {
                Vec::new()
            };
            let node = Expr::new(
                ExprKind::New {
                    callee: Box::new(callee),
                    args,
                },
                self.span_from(start),
            );
            return self.parse_member_suffix(node, true);
        }
        self.parse_member_chain(true)
    }

    fn parse_member_chain(&mut self, allow_calls: bool) -> Result<Expr, SyntaxError> {
        let prim = self.parse_primary()?;
        self.parse_member_suffix(prim, allow_calls)
    }

    fn parse_member_suffix(&mut self, mut e: Expr, allow_calls: bool) -> Result<Expr, SyntaxError> {
        loop {
            if self.at_punct(Punct::Dot) {
                self.bump();
                let prop = self.prop_name()?;
                let span = Span::new(e.span.start, self.last_end);
                e = Expr::new(
                    ExprKind::Member {
                        obj: Box::new(e),
                        prop: MemberProp::Ident(prop),
                        optional: false,
                    },
                    span,
                );
            } else if self.at_punct(Punct::LBracket) {
                self.bump();
                let idx = self.parse_expression()?;
                self.expect_punct(Punct::RBracket)?;
                let span = Span::new(e.span.start, self.last_end);
                e = Expr::new(
                    ExprKind::Member {
                        obj: Box::new(e),
                        prop: MemberProp::Computed(Box::new(idx)),
                        optional: false,
                    },
                    span,
                );
            } else if self.at_punct(Punct::QuestionDot) {
                self.bump();
                if self.at_punct(Punct::LParen) {
                    if !allow_calls {
                        break;
                    }
                    let args = self.parse_args()?;
                    let span = Span::new(e.span.start, self.last_end);
                    e = Expr::new(
                        ExprKind::Call {
                            callee: Box::new(e),
                            args,
                            optional: true,
                        },
                        span,
                    );
                } else if self.at_punct(Punct::LBracket) {
                    self.bump();
                    let idx = self.parse_expression()?;
                    self.expect_punct(Punct::RBracket)?;
                    let span = Span::new(e.span.start, self.last_end);
                    e = Expr::new(
                        ExprKind::Member {
                            obj: Box::new(e),
                            prop: MemberProp::Computed(Box::new(idx)),
                            optional: true,
                        },
                        span,
                    );
                } else {
                    let prop = self.prop_name()?;
                    let span = Span::new(e.span.start, self.last_end);
                    e = Expr::new(
                        ExprKind::Member {
                            obj: Box::new(e),
                            prop: MemberProp::Ident(prop),
                            optional: true,
                        },
                        span,
                    );
                }
            } else if self.at_punct(Punct::LParen) && allow_calls {
                let args = self.parse_args()?;
                let span = Span::new(e.span.start, self.last_end);
                e = Expr::new(
                    ExprKind::Call {
                        callee: Box::new(e),
                        args,
                        optional: false,
                    },
                    span,
                );
            } else if matches!(self.cur().kind, TokenKind::TemplatePart { .. }) {
                if self.template_depth == 0 {
                    return Err(self.err("tagged templates are deferred"));
                }
                // Continuation chunk of the enclosing `${ … }` (the lexer
                // absorbed its closing `}`); parse_template_lit consumes it.
                break;
            } else {
                break;
            }
        }
        Ok(e)
    }

    fn parse_args(&mut self) -> Result<Vec<Expr>, SyntaxError> {
        self.expect_punct(Punct::LParen)?;
        let mut args = Vec::new();
        if !self.at_punct(Punct::RParen) {
            loop {
                let arg_start = self.cur_loc();
                if self.eat_punct(Punct::Ellipsis) {
                    let inner = self.parse_assignment_expr()?;
                    args.push(Expr::new(
                        ExprKind::Spread(Box::new(inner)),
                        self.span_from(arg_start),
                    ));
                } else {
                    args.push(self.parse_assignment_expr()?);
                }
                if !self.eat_punct(Punct::Comma) {
                    break;
                }
            }
        }
        self.expect_punct(Punct::RParen)?;
        Ok(args)
    }

    // ----- primary ------------------------------------------------------------

    fn parse_primary(&mut self) -> Result<Expr, SyntaxError> {
        let start = self.cur_loc();
        use crate::tokens::Keyword as K;
        if self.at_punct(Punct::Slash) || self.at_punct(Punct::SlashEq) {
            self.tokens.borrow_mut().regex(self.pos)?;
        }
        match self.cur().kind.clone() {
            TokenKind::RegExp { pattern, flags } => {
                let compiled = amazing_regex::Regex::new(&pattern, &flags)
                    .map_err(|e| self.err(e.to_string()))?;
                self.bump();
                Ok(Expr::new(
                    ExprKind::RegExp(std::rc::Rc::new(compiled)),
                    self.span_from(start),
                ))
            }
            TokenKind::Numeric(n) => {
                self.bump();
                Ok(Expr::new(ExprKind::Num(n), self.span_from(start)))
            }
            TokenKind::Str(s) => {
                self.bump();
                Ok(Expr::new(ExprKind::Str(s), self.span_from(start)))
            }
            TokenKind::TemplatePart { .. } => {
                let lit = self.parse_template_lit()?;
                Ok(Expr::new(ExprKind::Template(lit), self.span_from(start)))
            }
            TokenKind::Keyword(K::This) => {
                self.bump();
                Ok(Expr::new(ExprKind::This, self.span_from(start)))
            }
            TokenKind::Keyword(K::Super) => {
                self.bump();
                Ok(Expr::new(ExprKind::Super, self.span_from(start)))
            }
            TokenKind::Keyword(K::Null) => {
                self.bump();
                Ok(Expr::new(ExprKind::Null, self.span_from(start)))
            }
            TokenKind::Keyword(K::True) => {
                self.bump();
                Ok(Expr::new(ExprKind::Bool(true), self.span_from(start)))
            }
            TokenKind::Keyword(K::False) => {
                self.bump();
                Ok(Expr::new(ExprKind::Bool(false), self.span_from(start)))
            }
            TokenKind::Keyword(K::Function) => {
                let f = self.parse_function_rest(false)?;
                Ok(Expr::new(
                    ExprKind::Function(Box::new(f)),
                    self.span_from(start),
                ))
            }
            TokenKind::Keyword(K::Class) => {
                let c = self.parse_class_rest()?;
                Ok(Expr::new(
                    ExprKind::Class(Box::new(c)),
                    self.span_from(start),
                ))
            }
            TokenKind::Keyword(K::Yield) => self.parse_yield_expr(),
            TokenKind::Punct(Punct::LParen) => self.parse_paren_or_arrow(),
            TokenKind::Punct(Punct::LBracket) => self.parse_array_literal(),
            TokenKind::Punct(Punct::Lbrace) => self.parse_object_literal(),
            TokenKind::Ident(_) => self.parse_ident_primary(),
            other => Err(self.err(format!("unexpected {}", other.describe()))),
        }
    }

    /// Identifier-position primaries: plain references plus every arrow
    /// shape that starts with an identifier (`x =>`, `async x =>`,
    /// `async (…) =>`, `async function`).
    fn parse_ident_primary(&mut self) -> Result<Expr, SyntaxError> {
        let start = self.cur_loc();
        // async function expression
        if self.word("async") && self.nth_is_function(1) {
            self.bump(); // async
            let f = self.parse_function_rest(true)?;
            return Ok(Expr::new(
                ExprKind::Function(Box::new(f)),
                self.span_from(start),
            ));
        }
        // async single-param arrow: `async x => …`
        if self.word("async")
            && matches!(self.nth(1).kind, TokenKind::Ident(_))
            && !self.nth(1).newline_before
            && self.nth_punct(2, Punct::Arrow)
            && !self.nth(2).newline_before
        {
            self.bump(); // async
            let param = self.expect_ident()?;
            self.bump(); // =>
            return self.finish_arrow(start, vec![plain_param(param)], true);
        }
        // async parenthesized arrow: speculate `(params) =>`.
        if self.word("async") && self.nth_punct(1, Punct::LParen) && !self.nth(1).newline_before {
            let save_pos = self.pos;
            let save_end = self.last_end;
            let save_noin = self.no_in;
            self.bump(); // async
            match self.try_arrow_params() {
                Ok(params) => return self.finish_arrow(start, params, true),
                Err(_) => {
                    self.pos = save_pos;
                    self.last_end = save_end;
                    self.no_in = save_noin;
                }
            }
        }
        // Single-param arrow fast path: `x => …` (covers `async => …` too,
        // where `async` is just the parameter name).
        if matches!(self.cur().kind, TokenKind::Ident(_))
            && self.nth_punct(1, Punct::Arrow)
            && !self.nth(1).newline_before
        {
            let param = self.expect_ident()?;
            self.bump(); // =>
            return self.finish_arrow(start, vec![plain_param(param)], false);
        }
        // Plain identifier reference.
        let id = self.expect_ident()?;
        Ok(Expr::new(ExprKind::Ident(id), self.span_from(start)))
    }

    /// The cover grammar for `(…)` : either parenthesized expressions or
    /// arrow parameters. Attempts arrow parameters strictly; rewinds to the
    /// saved position on any mismatch and parses parentheses instead.
    fn parse_paren_or_arrow(&mut self) -> Result<Expr, SyntaxError> {
        let start = self.cur_loc();
        let save_pos = self.pos;
        let save_end = self.last_end;
        let save_noin = self.no_in;
        match self.try_arrow_params() {
            Ok(params) => return self.finish_arrow(start, params, false),
            Err(_) => {
                self.pos = save_pos;
                self.last_end = save_end;
                self.no_in = save_noin;
            }
        }
        self.expect_punct(Punct::LParen)?;
        if self.at_punct(Punct::RParen) {
            // Empty parens are only valid after a successful arrow attempt.
            return Err(self.err("empty parenthesized expression"));
        }
        let inner = self.parse_expression()?;
        self.expect_punct(Punct::RParen)?;
        Ok(Expr::new(
            ExprKind::Paren(Box::new(inner)),
            self.span_from(start),
        ))
    }

    /// Consumes `( binding (= default)? , … ) =>` or errors (caller
    /// rewinds). The Arrow token is consumed on success.
    fn try_arrow_params(&mut self) -> Result<Vec<Param>, SyntaxError> {
        let params = self.parse_params()?;
        if !matches!(self.cur().kind, TokenKind::Punct(Punct::Arrow)) || self.cur().newline_before {
            return Err(self.err("not an arrow function"));
        }
        self.bump(); // =>
        Ok(params)
    }

    fn finish_arrow(
        &mut self,
        start: SourceLocation,
        params: Vec<Param>,
        is_async: bool,
    ) -> Result<Expr, SyntaxError> {
        let body = self.with_fn_context(is_async, false, |p| p.parse_arrow_body())?;
        Ok(Expr::new(
            ExprKind::Arrow(ArrowFunc {
                params,
                body,
                is_async,
                is_generator: false,
            }),
            self.span_from(start),
        ))
    }

    fn parse_arrow_body(&mut self) -> Result<ArrowBody, SyntaxError> {
        if self.at_punct(Punct::Lbrace) {
            Ok(ArrowBody::Block(self.parse_block()?))
        } else {
            Ok(ArrowBody::Expr(Box::new(self.parse_assignment_expr()?)))
        }
    }

    fn parse_yield_expr(&mut self) -> Result<Expr, SyntaxError> {
        let start = self.cur_loc();
        if self.in_generator == 0 {
            return Err(self.err("`yield` is only valid inside generator functions"));
        }
        self.bump(); // yield
        let delegate = self.eat_punct(Punct::Star);
        let argument = if self.starts_expression() && !self.cur().newline_before {
            Some(Box::new(self.parse_assignment_expr()?))
        } else {
            None
        };
        Ok(Expr::new(
            ExprKind::Yield { argument, delegate },
            self.span_from(start),
        ))
    }

    // ----- literals -----------------------------------------------------------

    fn parse_template_lit(&mut self) -> Result<TemplateLit, SyntaxError> {
        let t0 = self.bump();
        let (first, tail0) = match t0.kind {
            TokenKind::TemplatePart { cooked, tail } => (cooked, tail),
            _ => return Err(self.err("expected template literal")),
        };
        let mut quasis = vec![first];
        let mut exprs = Vec::new();
        let mut tail = tail0;
        while !tail {
            self.expect_punct(Punct::DollarBrace)?;
            self.template_depth += 1;
            let sub = self.parse_expression();
            self.template_depth -= 1;
            exprs.push(sub?);
            // The closing `}` was absorbed by the lexer together with the
            // next chunk, so a TemplatePart must follow directly.
            let t = self.bump();
            match t.kind {
                TokenKind::TemplatePart { cooked, tail: t2 } => {
                    quasis.push(cooked);
                    tail = t2;
                }
                other => {
                    return Err(self.err(format!(
                        "expected template continuation, found {}",
                        other.describe()
                    )));
                }
            }
        }
        Ok(TemplateLit { quasis, exprs })
    }

    fn parse_array_literal(&mut self) -> Result<Expr, SyntaxError> {
        let start = self.cur_loc();
        self.expect_punct(Punct::LBracket)?;
        let mut elems: Vec<Option<Expr>> = Vec::new();
        loop {
            if self.eat_punct(Punct::RBracket) {
                break;
            }
            if self.at_punct(Punct::Comma) {
                // Hole (or separator before another hole).
                self.bump();
                elems.push(None);
                continue;
            }
            let elem = if self.eat_punct(Punct::Ellipsis) {
                let inner = self.parse_assignment_expr()?;
                Expr::new(ExprKind::Spread(Box::new(inner)), self.span_from(start))
            } else {
                self.parse_assignment_expr()?
            };
            elems.push(Some(elem));
            if self.at_punct(Punct::Comma) {
                self.bump();
                if self.at_punct(Punct::RBracket) {
                    self.bump(); // trailing comma, no hole
                    break;
                }
            } else {
                self.expect_punct(Punct::RBracket)?;
                break;
            }
        }
        Ok(Expr::new(ExprKind::Array(elems), self.span_from(start)))
    }

    fn parse_object_literal(&mut self) -> Result<Expr, SyntaxError> {
        let start = self.cur_loc();
        self.expect_punct(Punct::Lbrace)?;
        let mut elems = Vec::new();
        while !self.at_punct(Punct::Rbrace) {
            if self.at_eof() {
                return Err(self.err("unterminated object literal: expected `}`"));
            }
            elems.push(self.parse_object_element()?);
            if !self.eat_punct(Punct::Comma) {
                break;
            }
        }
        self.expect_punct(Punct::Rbrace)?;
        Ok(Expr::new(ExprKind::Object(elems), self.span_from(start)))
    }

    fn parse_object_element(&mut self) -> Result<ObjElem, SyntaxError> {
        let start = self.cur_loc();
        if self.eat_punct(Punct::Ellipsis) {
            let inner = self.parse_assignment_expr()?;
            return Ok(ObjElem::Spread(Box::new(Expr::new(
                ExprKind::Spread(Box::new(inner)),
                self.span_from(start),
            ))));
        }
        // Modifiers (no `static` in object literals).
        // Modifier order is fixed by the grammar: `async` precedes `*`
        // (`async *ag() {}`; bare `*g()` and `async m()` are the other combos).
        let is_async =
            self.word("async") && !self.cur().newline_before && self.nth_starts_member_key(1);
        if is_async {
            self.bump();
        }
        let is_generator = self.eat_punct(Punct::Star);
        let mut kind = MethodKind::Method;
        if !is_generator && !is_async {
            if self.word("get") && self.nth_starts_member_key(1) {
                self.bump();
                kind = MethodKind::Get;
            } else if self.word("set") && self.nth_starts_member_key(1) {
                self.bump();
                kind = MethodKind::Set;
            }
        }
        let key_start = self.cur_loc();
        let key = self.parse_prop_key()?;
        if self.at_punct(Punct::Colon) {
            self.bump();
            let value = self.parse_assignment_expr()?;
            return Ok(ObjElem::Prop(Property {
                key,
                value: PropValue::Init(value),
                span: self.span_from(key_start),
            }));
        }
        if self.at_punct(Punct::LParen) {
            let params = self.parse_params()?;
            let body = self.with_fn_context(is_async, is_generator, |p| p.parse_block())?;
            return Ok(ObjElem::Prop(Property {
                key: key.clone(),
                value: PropValue::Method(ClassMethod {
                    key,
                    kind,
                    is_static: false,
                    is_generator,
                    is_async,
                    params,
                    body,
                    span: self.span_from(start),
                }),
                span: self.span_from(start),
            }));
        }
        // Shorthand `{ a }` — only for unmodified identifier keys.
        if is_generator || is_async || kind != MethodKind::Method {
            return Err(self.err("method modifiers require a parameter list"));
        }
        match key {
            PropKey::Ident(ref id) => {
                let value = PropValue::Shorthand(Ident::new(id.name.clone(), id.span));
                Ok(ObjElem::Prop(Property {
                    key,
                    value,
                    span: self.span_from(start),
                }))
            }
            _ => Err(self.err("expected `:` after property key")),
        }
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
    fn tagged_templates_are_rejected_as_deferred() {
        let e = err("tag`x${1}y`");
        assert!(e.message.contains("tagged"));
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
    fn class_decl_requires_a_name_and_private_names_are_deferred() {
        assert!(err("class {}").message.contains("name"));
        assert!(err("class A { #p = 1; }").message.contains("#"));
        // Static blocks are not a member shape this phase accepts.
        assert!(
            matches!(parse_script("class A { static {} }"), Err(_)),
            "static blocks must be rejected"
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
        assert!(err("yield 1;").message.contains("generator"));
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
}
