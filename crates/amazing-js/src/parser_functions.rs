//! Function, class, parameter, and property-key parsing.
use super::*;

impl Parser {
    // ----- functions -------------------------------------------------------

    /// Parses everything after `async`?/`function` — the keyword itself is
    /// consumed here. Callers fix up `is_async`.
    pub(super) fn parse_function_rest(&mut self, is_async: bool) -> Result<FunctionItem, SyntaxError> {
        self.bump(); // function
        let is_generator = self.eat_punct(Punct::Star);
        let name = if matches!(self.cur().kind, TokenKind::Ident(_)) {
            Some(self.expect_ident()?)
        } else {
            None
        };
        let saved_async = self.in_async;
        let saved_generator = self.in_generator;
        self.in_async = usize::from(is_async);
        self.in_generator = usize::from(is_generator);
        let params = self.parse_params();
        if params.is_err() {
            self.in_async = saved_async;
            self.in_generator = saved_generator;
        }
        let params = params?;
        self.in_function += 1;
        let saved_field = self.in_class_field_initializer;
        let saved_super = self.super_call_allowed;
        let saved_formals = self.in_formal_params;
        self.in_class_field_initializer = 0;
        self.super_call_allowed = false;
        self.in_formal_params = 0;
        let body = self.parse_block();
        self.in_formal_params = saved_formals;
        self.super_call_allowed = saved_super;
        self.in_class_field_initializer = saved_field;
        self.in_async = saved_async;
        self.in_generator = saved_generator;
        self.in_function -= 1;
        Ok(FunctionItem {
            name,
            params,
            body: body?,
            is_async,
            is_generator,
        })
    }

    pub(super) fn parse_params(&mut self) -> Result<Vec<Param>, SyntaxError> {
        self.in_formal_params += 1;
        let result = self.parse_params_inner();
        self.in_formal_params -= 1;
        result
    }

    pub(super) fn parse_params_inner(&mut self) -> Result<Vec<Param>, SyntaxError> {
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

    /// Method definitions use UniqueFormalParameters. Their body directive
    /// also rules out non-simple parameters, even when the containing script
    /// was already strict.
    pub(super) fn validate_method_params(
        &self,
        params: &[Param],
        strict: bool,
        own_strict_directive: bool,
    ) -> Result<(), SyntaxError> {
        let mut names = HashSet::new();
        for param in params {
            if own_strict_directive
                && (param.rest
                    || param.default.is_some()
                    || !matches!(param.binding, Binding::Ident(_)))
            {
                return Err(self.err_at(
                    param.span.start,
                    "a use strict directive cannot accompany non-simple parameters",
                ));
            }
            for ident in binding_names(&param.binding) {
                if !names.insert(&ident.name) {
                    return Err(self.err_at(ident.span.start, "duplicate method parameter"));
                }
                if strict && (ident.name == "eval" || ident.name == "arguments") {
                    return Err(self.err_at(
                        ident.span.start,
                        "eval and arguments cannot be strict parameters",
                    ));
                }
            }
        }
        Ok(())
    }

    /// Runs `f` with the function-body context flags applied; used by every
    /// callable body so nested `return`/`yield`/`await` validity works.
    pub(super) fn with_fn_context<T>(
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
    pub(super) fn parse_class_rest(&mut self) -> Result<ClassItem, SyntaxError> {
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
        self.private_scopes.push(PrivateScope::default());
        let mut members = Vec::new();
        while !self.at_punct(Punct::Rbrace) {
            if self.at_eof() {
                return Err(self.err("unterminated class body: expected `}`"));
            }
            members.push(self.parse_class_member()?);
        }
        self.bump(); // }
        let scope = self.private_scopes.pop().expect("class scope");
        for use_name in scope.uses {
            if !scope.declared.contains_key(&use_name.name) {
                if let Some(outer) = self.private_scopes.last_mut() {
                    outer.uses.push(use_name);
                } else {
                    return Err(self.err_at(
                        use_name.span.start,
                        format!("private name {} is not declared", use_name.name),
                    ));
                }
            }
        }
        Ok(ClassItem {
            name,
            super_class,
            members,
        })
    }

    /// Can token `n` begin a class-member key? Guards contextual modifiers
    /// (`static`, `get`, `set`, `async`) from being eaten when they are
    /// themselves keys (`static() {}`, `get: 1`, `set = 3`, `static;`).
    pub(super) fn nth_starts_member_key(&self, n: usize) -> bool {
        match self.nth(n).kind {
            TokenKind::Ident(_)
            | TokenKind::Keyword(_)
            | TokenKind::Str(_)
            | TokenKind::Numeric(_) => true,
            TokenKind::PrivateIdent(_) => true,
            TokenKind::Punct(Punct::LBracket) | TokenKind::Punct(Punct::Star) => true,
            _ => false,
        }
    }

    pub(super) fn parse_class_member(&mut self) -> Result<ClassMember, SyntaxError> {
        let start = self.cur_loc();
        let is_static = self.word("static") && self.nth_starts_member_key(1);
        if is_static {
            self.bump();
        }
        // Modifier order is fixed by the grammar: `async` precedes `*`
        // (`async *ag() {}`; bare `*g()` and `async m()` are the other combos).
        let is_async =
            self.word("async") && !self.nth(1).newline_before && self.nth_starts_member_key(1);
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
        let (key, key_utf16) = if let TokenKind::PrivateIdent(name) = self.cur().kind.clone() {
            let t = self.bump();
            (PropKey::Private(Ident::new(name, t.span)), None)
        } else {
            self.parse_prop_key()?
        };
        if self.at_punct(Punct::LParen) {
            if !is_static
                && (is_async || is_generator)
                && prop_key_name(&key) == Some("constructor")
            {
                return Err(self.err_at(start, "class constructors cannot be async or generators"));
            }
            self.in_async += usize::from(is_async);
            self.in_generator += usize::from(is_generator);
            let params = self.parse_params();
            self.in_generator -= usize::from(is_generator);
            self.in_async -= usize::from(is_async);
            let params = params?;
            let actual_kind = if kind == MethodKind::Method
                && !is_static
                && prop_key_name(&key) == Some("constructor")
            {
                MethodKind::Constructor
            } else {
                kind
            };
            self.declare_private(&key, actual_kind, is_static)?;
            let saved_field = self.in_class_field_initializer;
            let saved_super = self.super_call_allowed;
            self.in_class_field_initializer = 0;
            self.super_call_allowed = actual_kind == MethodKind::Constructor;
            let body = self.with_fn_context(is_async, is_generator, |p| p.parse_block());
            self.super_call_allowed = saved_super;
            self.in_class_field_initializer = saved_field;
            let body = body?;
            self.validate_method_params(&params, true, has_use_strict(&body))?;
            return Ok(ClassMember::Method(ClassMethod {
                key,
                key_utf16,
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
        self.declare_private(&key, MethodKind::Method, is_static)?;
        let init = if self.eat_punct(Punct::Eq) {
            self.in_class_field_initializer += 1;
            let init = self.parse_assignment_expr();
            self.in_class_field_initializer -= 1;
            Some(init?)
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
            key_utf16,
            is_static,
            init,
            span: self.span_from(start),
        }))
    }

    pub(super) fn declare_private(
        &mut self,
        key: &PropKey,
        kind: MethodKind,
        is_static: bool,
    ) -> Result<(), SyntaxError> {
        let PropKey::Private(id) = key else {
            return Ok(());
        };
        if id.name == "#constructor" {
            return Err(self.err_at(id.span.start, "#constructor is not a valid private name"));
        }
        let bit = match kind {
            MethodKind::Get => 1,
            MethodKind::Set => 2,
            _ => 4,
        };
        let scope = self.private_scopes.last_mut().expect("class scope");
        let previous = scope
            .declared
            .entry(id.name.clone())
            .or_insert((0, is_static));
        if previous.1 != is_static
            || previous.0 & bit != 0
            || (previous.0 | bit) & 4 != 0 && previous.0 != 0
        {
            return Err(self.err_at(id.span.start, format!("duplicate private name {}", id.name)));
        }
        previous.0 |= bit;
        Ok(())
    }

    pub(super) fn use_private(&mut self, id: Ident) -> Result<(), SyntaxError> {
        let Some(scope) = self.private_scopes.last_mut() else {
            return Err(self.err_at(id.span.start, "private name outside class"));
        };
        scope.uses.push(id);
        Ok(())
    }

    /// Property key: identifier (reserved words included), string, numeric
    /// literal, or `[expr]`.
    pub(super) fn parse_prop_key(&mut self) -> Result<(PropKey, Option<Vec<u16>>), SyntaxError> {
        match self.cur().kind.clone() {
            TokenKind::Ident(_) => Ok((PropKey::Ident(self.expect_ident()?), None)),
            TokenKind::Keyword(k) => {
                let t = self.bump();
                Ok((PropKey::Ident(Ident::new(k.spelling(), t.span)), None))
            }
            TokenKind::Str(s) => {
                let token = self.bump();
                Ok((PropKey::Str(s), token.cooked_utf16))
            }
            TokenKind::Numeric(n) => {
                self.bump();
                Ok((PropKey::Num(n), None))
            }
            TokenKind::Punct(Punct::LBracket) => {
                self.bump();
                let e = self.parse_assignment_expr()?;
                self.expect_punct(Punct::RBracket)?;
                Ok((PropKey::Computed(Box::new(e)), None))
            }
            other => Err(self.err(format!(
                "expected property name, found {}",
                other.describe()
            ))),
        }
    }
}
