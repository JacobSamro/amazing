//! A small, panic-free tree-walking ECMAScript runtime.
//!
//! This is the Phase 6 execution milestone. It implements ordinary values,
//! objects, arrays, functions, closures, control flow, coercion and a small
//! set of ES built-ins using only `std`. Modules, generators, symbols and a
//! real asynchronous job queue remain explicit later milestones.

use std::cell::RefCell;
use std::rc::Rc;

use crate::ast::{self, Binding, ExprKind, ForInit, ForLeft, MemberProp, PropKey, StmtKind};
use crate::env::{EnvHandle, EnvKind};
use crate::value::{
    number_to_string, Exotic, FunctionBody, JsFunction, JsObject, JsValue, MapKey, MapState, Obj,
    SetState, Throw,
};

pub const MAX_CALL_DEPTH: u32 = 512;

enum Flow {
    Normal,
    Break(Option<String>),
    Continue(Option<String>),
    Return(JsValue),
}

pub struct Interpreter {
    pub global_env: EnvHandle,
    pub global_obj: Obj,
    pub object_proto: Obj,
    pub function_proto: Obj,
    pub array_proto: Obj,
    pub map_proto: Obj,
    pub set_proto: Obj,
    pub output: RefCell<Vec<String>>,
    depth: u32,
}

impl Interpreter {
    pub fn new() -> Self {
        let object_proto = Obj(Rc::new(RefCell::new(JsObject::ordinary())));
        let function_proto = Obj(Rc::new(RefCell::new({
            let mut o = JsObject::ordinary();
            o.proto = Some(object_proto.clone());
            o
        })));
        let array_proto = Obj(Rc::new(RefCell::new({
            let mut o = JsObject::ordinary();
            o.proto = Some(object_proto.clone());
            o
        })));
        let map_proto = Obj(Rc::new(RefCell::new({
            let mut o = JsObject::ordinary();
            o.proto = Some(object_proto.clone());
            o
        })));
        let set_proto = Obj(Rc::new(RefCell::new({
            let mut o = JsObject::ordinary();
            o.proto = Some(object_proto.clone());
            o
        })));
        let global_obj = Obj(Rc::new(RefCell::new({
            let mut o = JsObject::ordinary();
            o.proto = Some(object_proto.clone());
            o
        })));
        let global_env = EnvHandle::new(EnvKind::Global, None);
        let mut i = Self {
            global_env,
            global_obj,
            object_proto,
            function_proto,
            array_proto,
            map_proto,
            set_proto,
            output: RefCell::new(Vec::new()),
            depth: 0,
        };
        i.global_obj
            .borrow_mut()
            .set_own("globalThis", JsValue::Object(i.global_obj.clone()));
        i.install_builtins();
        i
    }

    pub fn run_script(&mut self, source: &str) -> Result<JsValue, Throw> {
        let p = crate::parse_script(source).map_err(|e| Throw::syntax_error(e.to_string()))?;
        self.run_program(&p)
    }

    pub fn run_program(&mut self, p: &ast::Program) -> Result<JsValue, Throw> {
        let env = self.global_env.clone();
        self.hoist(&p.body, &env)?;
        let mut last = JsValue::Undefined;
        for s in &p.body {
            if let StmtKind::Expression(e) = &s.kind {
                last = self.eval_expr(e, &env)?;
                continue;
            }
            match self.exec_stmt(s, &env)? {
                Flow::Normal => {}
                Flow::Return(v) => last = v,
                Flow::Break(_) | Flow::Continue(_) => {
                    return Err(Throw::syntax_error("illegal break or continue"))
                }
            }
        }
        Ok(last)
    }

    fn hoist(&mut self, body: &[ast::Stmt], env: &EnvHandle) -> Result<(), Throw> {
        for s in body {
            match &s.kind {
                StmtKind::Var(d) if d.kind == ast::VarKind::Var => {
                    for x in &d.decls {
                        let Binding::Ident(id) = &x.binding;
                        env.declare_var(&id.name, JsValue::Undefined)?;
                    }
                }
                StmtKind::Function(f) => {
                    if let Some(n) = &f.name {
                        let v = self.make_function(f, env);
                        env.declare_var(&n.name, v)?;
                    }
                }
                StmtKind::Block(b) => self.hoist(&b.body, env)?,
                StmtKind::If { then, els, .. } => {
                    self.hoist(std::slice::from_ref(then), env)?;
                    if let Some(e) = els {
                        self.hoist(std::slice::from_ref(e), env)?;
                    }
                }
                StmtKind::While { body, .. } | StmtKind::DoWhile { body, .. } => {
                    self.hoist(std::slice::from_ref(body), env)?
                }
                StmtKind::For { init, body, .. } => {
                    if let Some(ForInit::Decl(d)) = init {
                        self.hoist_var(d, env)?;
                    }
                    self.hoist(std::slice::from_ref(body), env)?;
                }
                StmtKind::ForIn { left, body, .. } | StmtKind::ForOf { left, body, .. } => {
                    if let ForLeft::Decl(d) = left {
                        self.hoist_var(d, env)?;
                    }
                    self.hoist(std::slice::from_ref(body), env)?;
                }
                StmtKind::Labeled { body, .. } => self.hoist(std::slice::from_ref(body), env)?,
                _ => {}
            }
        }
        Ok(())
    }

    fn hoist_var(&self, d: &ast::VarDecl, env: &EnvHandle) -> Result<(), Throw> {
        if d.kind == ast::VarKind::Var {
            for x in &d.decls {
                let Binding::Ident(id) = &x.binding;
                env.declare_var(&id.name, JsValue::Undefined)?;
            }
        }
        Ok(())
    }

    fn exec_stmt(&mut self, s: &ast::Stmt, env: &EnvHandle) -> Result<Flow, Throw> {
        match &s.kind {
            StmtKind::Empty | StmtKind::Debugger => Ok(Flow::Normal),
            StmtKind::Expression(e) => {
                self.eval_expr(e, env)?;
                Ok(Flow::Normal)
            }
            StmtKind::Var(d) => {
                for x in &d.decls {
                    let v = x
                        .init
                        .as_ref()
                        .map(|e| self.eval_expr(e, env))
                        .transpose()?
                        .unwrap_or(JsValue::Undefined);
                    let Binding::Ident(id) = &x.binding;
                    match d.kind {
                        ast::VarKind::Var => {
                            if !env.assign(&id.name, v.clone())? {
                                self.global_env.declare_var(&id.name, v)?;
                            }
                        }
                        ast::VarKind::Let => env.declare_lexical(&id.name, v, true)?,
                        ast::VarKind::Const => {
                            if x.init.is_none() {
                                return Err(Throw::syntax_error("const requires an initializer"));
                            }
                            env.declare_lexical(&id.name, v, false)?;
                        }
                    }
                }
                Ok(Flow::Normal)
            }
            StmtKind::Function(_) => Ok(Flow::Normal),
            StmtKind::Class(c) => {
                if let Some(n) = &c.name {
                    let v = self.eval_class(c, env)?;
                    env.declare_lexical(&n.name, v, true)?;
                }
                Ok(Flow::Normal)
            }
            StmtKind::Block(b) => {
                let e = EnvHandle::new(EnvKind::Block, Some(env.clone()));
                self.hoist(&b.body, &e)?;
                self.exec_stmts(&b.body, &e)
            }
            StmtKind::If { cond, then, els } => {
                if self.eval_expr(cond, env)?.to_boolean() {
                    self.exec_stmt(then, env)
                } else if let Some(e) = els {
                    self.exec_stmt(e, env)
                } else {
                    Ok(Flow::Normal)
                }
            }
            StmtKind::While { test, body } => self.loop_while(test, body, env),
            StmtKind::DoWhile { body, test } => {
                loop {
                    match self.exec_stmt(body, env)? {
                        Flow::Break(None) => break,
                        Flow::Break(x) => return Ok(Flow::Break(x)),
                        Flow::Continue(None) | Flow::Normal => {}
                        Flow::Continue(x) => return Ok(Flow::Continue(x)),
                        Flow::Return(v) => return Ok(Flow::Return(v)),
                    }
                    if !self.eval_expr(test, env)?.to_boolean() {
                        break;
                    }
                }
                Ok(Flow::Normal)
            }
            StmtKind::For {
                init,
                test,
                update,
                body,
            } => {
                let e = EnvHandle::new(EnvKind::Block, Some(env.clone()));
                if let Some(i) = init {
                    match i {
                        ForInit::Decl(d) => {
                            let fake = ast::Stmt::new(StmtKind::Var((**d).clone()), s.span.clone());
                            self.exec_stmt(&fake, &e)?;
                        }
                        ForInit::Expr(x) => {
                            self.eval_expr(x, &e)?;
                        }
                    }
                }
                loop {
                    if let Some(t) = test {
                        if !self.eval_expr(t, &e)?.to_boolean() {
                            break;
                        }
                    }
                    match self.exec_stmt(body, &e)? {
                        Flow::Break(None) => break,
                        Flow::Break(x) => return Ok(Flow::Break(x)),
                        Flow::Continue(None) | Flow::Normal => {}
                        Flow::Continue(x) => return Ok(Flow::Continue(x)),
                        Flow::Return(v) => return Ok(Flow::Return(v)),
                    }
                    if let Some(u) = update {
                        self.eval_expr(u, &e)?;
                    }
                }
                Ok(Flow::Normal)
            }
            StmtKind::ForIn { left, right, body } => {
                let v = self.eval_expr(right, env)?;
                for k in self.enumerable_keys(&v) {
                    self.bind_for(left, JsValue::str(k), env)?;
                    match self.exec_stmt(body, env)? {
                        Flow::Normal | Flow::Continue(None) => {}
                        Flow::Break(None) => break,
                        x => return Ok(x),
                    }
                }
                Ok(Flow::Normal)
            }
            StmtKind::ForOf { left, right, body } => {
                let v = self.eval_expr(right, env)?;
                for x in self.iterate(&v)? {
                    self.bind_for(left, x, env)?;
                    match self.exec_stmt(body, env)? {
                        Flow::Normal | Flow::Continue(None) => {}
                        Flow::Break(None) => break,
                        x => return Ok(x),
                    }
                }
                Ok(Flow::Normal)
            }
            StmtKind::Switch { disc, cases } => {
                let d = self.eval_expr(disc, env)?;
                let mut start = None;
                let mut def = None;
                for (n, c) in cases.iter().enumerate() {
                    if let Some(t) = &c.test {
                        let tv = self.eval_expr(t, env)?;
                        if self.strict_eq(&d, &tv) {
                            start = Some(n);
                            break;
                        }
                    } else {
                        def = Some(n);
                    }
                }
                let mut i = start.or(def);
                while let Some(n) = i {
                    match self.exec_stmts(&cases[n].body, env)? {
                        Flow::Normal => {
                            i = (n + 1 < cases.len()).then_some(n + 1);
                        }
                        Flow::Break(_) => break,
                        x => return Ok(x),
                    }
                }
                Ok(Flow::Normal)
            }
            StmtKind::Break { label: l } => Ok(Flow::Break(l.as_ref().map(|x| x.name.clone()))),
            StmtKind::Continue { label: l } => {
                Ok(Flow::Continue(l.as_ref().map(|x| x.name.clone())))
            }
            StmtKind::Labeled { label, body } => match self.exec_stmt(body, env)? {
                Flow::Break(Some(x)) if x == label.name => Ok(Flow::Normal),
                x => Ok(x),
            },
            StmtKind::Return(e) => Ok(Flow::Return(
                e.as_ref()
                    .map(|x| self.eval_expr(x, env))
                    .transpose()?
                    .unwrap_or(JsValue::Undefined),
            )),
            StmtKind::Throw(e) => Err(Throw(self.eval_expr(e, env)?)),
            StmtKind::Try(t) => self.exec_try(t, env),
        }
    }

    fn exec_stmts(&mut self, body: &[ast::Stmt], env: &EnvHandle) -> Result<Flow, Throw> {
        for s in body {
            match self.exec_stmt(s, env)? {
                Flow::Normal => {}
                x => return Ok(x),
            }
        }
        Ok(Flow::Normal)
    }
    fn loop_while(
        &mut self,
        test: &ast::Expr,
        body: &ast::Stmt,
        env: &EnvHandle,
    ) -> Result<Flow, Throw> {
        while self.eval_expr(test, env)?.to_boolean() {
            match self.exec_stmt(body, env)? {
                Flow::Normal | Flow::Continue(None) => {}
                Flow::Break(None) => break,
                x => return Ok(x),
            }
        }
        Ok(Flow::Normal)
    }
    fn bind_for(&mut self, left: &ForLeft, value: JsValue, env: &EnvHandle) -> Result<(), Throw> {
        match left {
            ForLeft::Target(t) => self.assign_target(t, value, env),
            ForLeft::Decl(d) => {
                let id = match d.decls.first().map(|x| &x.binding) {
                    Some(Binding::Ident(i)) => &i.name,
                    _ => return Err(Throw::syntax_error("destructuring is deferred")),
                };
                if d.kind == ast::VarKind::Var {
                    if !env.assign(id, value.clone())? {
                        env.declare_var(id, value)?;
                    }
                } else {
                    env.declare_lexical(id, value, true)?;
                }
                Ok(())
            }
        }
    }
    fn exec_try(&mut self, t: &ast::TryStmt, env: &EnvHandle) -> Result<Flow, Throw> {
        let e = EnvHandle::new(EnvKind::Block, Some(env.clone()));
        self.hoist(&t.block.body, &e)?;
        let mut out = match self.exec_stmts(&t.block.body, &e) {
            Ok(x) => Ok(x),
            Err(th) => {
                if let Some(c) = &t.catch {
                    let ce = EnvHandle::new(EnvKind::Block, Some(env.clone()));
                    if let Some(p) = &c.param {
                        ce.declare_lexical(&p.name, th.0, true)?;
                    }
                    self.hoist(&c.body.body, &ce)?;
                    self.exec_stmts(&c.body.body, &ce)
                } else {
                    Err(th)
                }
            }
        };
        if let Some(f) = &t.finally {
            let fe = EnvHandle::new(EnvKind::Block, Some(env.clone()));
            self.hoist(&f.body, &fe)?;
            match self.exec_stmts(&f.body, &fe) {
                Ok(Flow::Normal) => {}
                Ok(x) => out = Ok(x),
                Err(e) => out = Err(e),
            }
        }
        out
    }

    pub fn eval_expr(&mut self, e: &ast::Expr, env: &EnvHandle) -> Result<JsValue, Throw> {
        match &e.kind {
            ExprKind::Num(n) => self.numeric(n),
            ExprKind::Str(s) => Ok(JsValue::str(s.as_str())),
            ExprKind::Bool(b) => Ok(JsValue::Bool(*b)),
            ExprKind::Null => Ok(JsValue::Null),
            ExprKind::Ident(i) => env
                .resolve(&i.name)
                .ok_or_else(|| Throw::reference_error(format!("{} is not defined", i.name))),
            ExprKind::This => Ok(env.resolve_this()),
            ExprKind::Super => Err(Throw::syntax_error("unexpected super")),
            ExprKind::Template(t) => {
                let mut s = String::new();
                for (i, q) in t.quasis.iter().enumerate() {
                    s.push_str(q);
                    if let Some(x) = t.exprs.get(i) {
                        let v = self.eval_expr(x, env)?;
                        s.push_str(&self.to_string(&v)?);
                    }
                }
                Ok(JsValue::str(s))
            }
            ExprKind::Array(xs) => {
                let a = JsValue::object({
                    let mut o = JsObject::array();
                    o.proto = Some(self.array_proto.clone());
                    o
                });
                let mut n = 0usize;
                for x in xs {
                    match x {
                        None => n += 1,
                        Some(x) if matches!(x.kind, ExprKind::Spread(_)) => {
                            if let ExprKind::Spread(y) = &x.kind {
                                let src = self.eval_expr(y, env)?;
                                let vals = self.iterate(&src)?;
                                for v in vals {
                                    self.set_prop(&a, &n.to_string(), v)?;
                                    n += 1;
                                }
                            }
                        }
                        Some(x) => {
                            let v = self.eval_expr(x, env)?;
                            self.set_prop(&a, &n.to_string(), v)?;
                            n += 1;
                        }
                    }
                }
                self.set_prop(&a, "length", JsValue::num(n as f64))?;
                Ok(a)
            }
            ExprKind::Object(xs) => {
                let o = self.object_value();
                for x in xs {
                    match x {
                        ast::ObjElem::Spread(y) => {
                            let src = self.eval_expr(y, env)?;
                            for k in self.enumerable_keys(&src) {
                                let v = self.get_prop(&src, &k)?;
                                self.set_prop(&o, &k, v)?;
                            }
                        }
                        ast::ObjElem::Prop(p) => {
                            let k = self.prop_key(&p.key, env)?;
                            let v = match &p.value {
                                ast::PropValue::Shorthand(i) => env
                                    .resolve(&i.name)
                                    .ok_or_else(|| Throw::reference_error(i.name.clone()))?,
                                ast::PropValue::Init(x) => self.eval_expr(x, env)?,
                                ast::PropValue::Method(m) => self.method_value(m, env),
                            };
                            self.set_prop(&o, &k, v)?;
                        }
                    }
                }
                Ok(o)
            }
            ExprKind::Spread(x) => self.eval_expr(x, env),
            ExprKind::Function(f) => Ok(self.make_function(f, env)),
            ExprKind::Arrow(a) => Ok(self.make_arrow(a, env)),
            ExprKind::Class(c) => self.eval_class(c, env),
            ExprKind::Paren(x) => self.eval_expr(x, env),
            ExprKind::Sequence(xs) => {
                let mut v = JsValue::Undefined;
                for x in xs {
                    v = self.eval_expr(x, env)?;
                }
                Ok(v)
            }
            ExprKind::Unary { op, operand } => self.unary(*op, operand, env),
            ExprKind::Update { op, prefix, target } => self.update(*op, *prefix, target, env),
            ExprKind::Binary { op, left, right } => {
                let a = self.eval_expr(left, env)?;
                let b = self.eval_expr(right, env)?;
                self.binary(*op, a, b)
            }
            ExprKind::Logical { op, left, right } => {
                let a = self.eval_expr(left, env)?;
                let take = match op {
                    ast::LogicalOp::And => a.to_boolean(),
                    ast::LogicalOp::Or => !a.to_boolean(),
                    ast::LogicalOp::Coalesce => a.is_null() || a.is_undefined(),
                };
                if take {
                    self.eval_expr(right, env)
                } else {
                    Ok(a)
                }
            }
            ExprKind::Assign { op, target, value } => self.assign(*op, target, value, env),
            ExprKind::Conditional {
                test,
                consequent,
                alternate,
            } => {
                if self.eval_expr(test, env)?.to_boolean() {
                    self.eval_expr(consequent, env)
                } else {
                    self.eval_expr(alternate, env)
                }
            }
            ExprKind::Call {
                callee,
                args,
                optional,
            } => self.call_expr(callee, args, *optional, env),
            ExprKind::New { callee, args } => {
                let f = self.eval_expr(callee, env)?;
                let a = self.eval_args(args, env)?;
                self.construct(&f, a)
            }
            ExprKind::Member {
                obj,
                prop,
                optional,
            } => {
                let o = self.eval_expr(obj, env)?;
                if *optional && (o.is_null() || o.is_undefined()) {
                    Ok(JsValue::Undefined)
                } else {
                    let k = self.member_key(prop, env)?;
                    self.get_prop(&o, &k)
                }
            }
            ExprKind::Yield { .. } => Err(Throw::type_error("generators are not implemented")),
        }
    }

    fn numeric(&self, n: &crate::tokens::Numeric) -> Result<JsValue, Throw> {
        if n.bigint {
            return Err(Throw::type_error("BigInt is not implemented"));
        }
        let raw: String = n.raw.chars().filter(|c| *c != '_').collect();
        let v = if n.radix == 10 {
            raw.parse().unwrap_or(f64::NAN)
        } else {
            u128::from_str_radix(&raw, n.radix as u32)
                .map(|x| x as f64)
                .unwrap_or(f64::NAN)
        };
        Ok(JsValue::num(v))
    }
    fn unary(&mut self, op: ast::UnOp, x: &ast::Expr, env: &EnvHandle) -> Result<JsValue, Throw> {
        match op {
            ast::UnOp::Typeof => {
                if let ExprKind::Ident(i) = &x.kind {
                    if let Some(v) = env.resolve(&i.name) {
                        return Ok(JsValue::str(self.typeof_value(&v)));
                    }
                    return Ok(JsValue::str("undefined"));
                }
                let v = self.eval_expr(x, env)?;
                Ok(JsValue::str(self.typeof_value(&v)))
            }
            ast::UnOp::Void => {
                self.eval_expr(x, env)?;
                Ok(JsValue::Undefined)
            }
            ast::UnOp::Delete => match &x.kind {
                ExprKind::Member { obj, prop, .. } => {
                    let o = self.eval_expr(obj, env)?;
                    let k = self.member_key(prop, env)?;
                    if let JsValue::Object(o) = o {
                        Ok(JsValue::Bool(o.borrow_mut().delete_own(&k)))
                    } else {
                        Ok(JsValue::Bool(true))
                    }
                }
                _ => {
                    self.eval_expr(x, env)?;
                    Ok(JsValue::Bool(true))
                }
            },
            ast::UnOp::Await => self.eval_expr(x, env),
            ast::UnOp::Not => Ok(JsValue::Bool(!self.eval_expr(x, env)?.to_boolean())),
            ast::UnOp::BitNot => {
                let v = self.eval_expr(x, env)?;
                Ok(JsValue::num((!self.to_int32(&v)?) as f64))
            }
            ast::UnOp::Neg => {
                let v = self.eval_expr(x, env)?;
                Ok(JsValue::num(-self.to_number(&v)?))
            }
            ast::UnOp::Pos => {
                let v = self.eval_expr(x, env)?;
                Ok(JsValue::num(self.to_number(&v)?))
            }
        }
    }
    fn update(
        &mut self,
        op: ast::UpdateOp,
        prefix: bool,
        x: &ast::Expr,
        env: &EnvHandle,
    ) -> Result<JsValue, Throw> {
        let old = self.eval_expr(x, env)?;
        let n = self.to_number(&old)? + if op == ast::UpdateOp::Inc { 1.0 } else { -1.0 };
        self.assign_target(x, JsValue::num(n), env)?;
        Ok(if prefix { JsValue::num(n) } else { old })
    }
    fn binary(&mut self, op: ast::BinOp, a: JsValue, b: JsValue) -> Result<JsValue, Throw> {
        use ast::BinOp::*;
        match op {
            Add => {
                let x = self.primitive(a)?;
                let y = self.primitive(b)?;
                if x.is_str() || y.is_str() {
                    Ok(JsValue::str(format!(
                        "{}{}",
                        self.to_string(&x)?,
                        self.to_string(&y)?
                    )))
                } else {
                    Ok(JsValue::num(self.to_number(&x)? + self.to_number(&y)?))
                }
            }
            Sub => Ok(JsValue::num(self.to_number(&a)? - self.to_number(&b)?)),
            Mul => Ok(JsValue::num(self.to_number(&a)? * self.to_number(&b)?)),
            Div => Ok(JsValue::num(self.to_number(&a)? / self.to_number(&b)?)),
            Rem => Ok(JsValue::num(self.to_number(&a)? % self.to_number(&b)?)),
            Pow => Ok(JsValue::num(self.to_number(&a)?.powf(self.to_number(&b)?))),
            Lt | Gt | Le | Ge => {
                let x = self.primitive(a)?;
                let y = self.primitive(b)?;
                let c = if x.is_str() && y.is_str() {
                    x.as_str().unwrap().cmp(y.as_str().unwrap())
                } else {
                    self.to_number(&x)?
                        .partial_cmp(&self.to_number(&y)?)
                        .unwrap_or(std::cmp::Ordering::Equal)
                };
                Ok(JsValue::Bool(match op {
                    Lt => c.is_lt(),
                    Gt => c.is_gt(),
                    Le => c.is_le(),
                    Ge => c.is_ge(),
                    _ => false,
                }))
            }
            EqEq | NotEq => {
                let e = self.loose_eq(&a, &b)?;
                Ok(JsValue::Bool(if op == EqEq { e } else { !e }))
            }
            StrictEq | StrictNeq => {
                let e = self.strict_eq(&a, &b);
                Ok(JsValue::Bool(if op == StrictEq { e } else { !e }))
            }
            InstanceOf => self.instanceof(&a, &b),
            In => {
                let k = self.to_property_key(&b)?;
                match a {
                    JsValue::Object(o) => Ok(JsValue::Bool(o.borrow().has_property(&k))),
                    _ => Err(Throw::type_error("right operand of in is not an object")),
                }
            }
            Shl | Shr | UShr | BitAnd | BitOr | BitXor => {
                let x = self.to_int32(&a)?;
                let y = self.to_uint32(&b)? & 31;
                Ok(JsValue::num(match op {
                    Shl => (x << y) as f64,
                    Shr => (x >> y) as f64,
                    UShr => (x.wrapping_shr(y)) as u32 as f64,
                    BitAnd => (x & (y as i32)) as f64,
                    BitOr => (x | (y as i32)) as f64,
                    BitXor => (x ^ (y as i32)) as f64,
                    _ => 0.0,
                }))
            }
        }
    }
    fn assign(
        &mut self,
        op: ast::AssignOp,
        t: &ast::Expr,
        r: &ast::Expr,
        e: &EnvHandle,
    ) -> Result<JsValue, Throw> {
        if op == ast::AssignOp::Assign {
            let v = self.eval_expr(r, e)?;
            self.assign_target(t, v.clone(), e)?;
            return Ok(v);
        }
        let old = self.eval_expr(t, e)?;
        if op == ast::AssignOp::And && !old.to_boolean() {
            return Ok(old);
        }
        if op == ast::AssignOp::Or && old.to_boolean() {
            return Ok(old);
        }
        if op == ast::AssignOp::Nullish && !old.is_null() && !old.is_undefined() {
            return Ok(old);
        }
        let v = self.eval_expr(r, e)?;
        let bop = match op {
            ast::AssignOp::Add => ast::BinOp::Add,
            ast::AssignOp::Sub => ast::BinOp::Sub,
            ast::AssignOp::Mul => ast::BinOp::Mul,
            ast::AssignOp::Div => ast::BinOp::Div,
            ast::AssignOp::Rem => ast::BinOp::Rem,
            ast::AssignOp::Pow => ast::BinOp::Pow,
            ast::AssignOp::Shl => ast::BinOp::Shl,
            ast::AssignOp::Shr => ast::BinOp::Shr,
            ast::AssignOp::UShr => ast::BinOp::UShr,
            ast::AssignOp::BitAnd => ast::BinOp::BitAnd,
            ast::AssignOp::BitOr => ast::BinOp::BitOr,
            ast::AssignOp::BitXor => ast::BinOp::BitXor,
            _ => ast::BinOp::Add,
        };
        let out = self.binary(bop, old, v)?;
        self.assign_target(t, out.clone(), e)?;
        Ok(out)
    }
    fn assign_target(&mut self, t: &ast::Expr, v: JsValue, e: &EnvHandle) -> Result<(), Throw> {
        match &t.kind {
            ExprKind::Ident(i) => {
                if !e.assign(&i.name, v.clone())? {
                    self.global_env.declare_var(&i.name, v)?;
                }
                Ok(())
            }
            ExprKind::Member { obj, prop, .. } => {
                let o = self.eval_expr(obj, e)?;
                let k = self.member_key(prop, e)?;
                self.set_prop(&o, &k, v)
            }
            ExprKind::Paren(x) => self.assign_target(x, v, e),
            _ => Err(Throw::reference_error("invalid assignment target")),
        }
    }
    fn call_expr(
        &mut self,
        c: &ast::Expr,
        args: &[ast::Expr],
        optional: bool,
        e: &EnvHandle,
    ) -> Result<JsValue, Throw> {
        let (f, this) = if let ExprKind::Member {
            obj,
            prop,
            optional: m,
        } = &c.kind
        {
            let o = self.eval_expr(obj, e)?;
            if (optional || *m) && (o.is_null() || o.is_undefined()) {
                return Ok(JsValue::Undefined);
            }
            let k = self.member_key(prop, e)?;
            (self.get_prop(&o, &k)?, o)
        } else {
            (self.eval_expr(c, e)?, JsValue::Undefined)
        };
        if optional && (f.is_null() || f.is_undefined()) {
            return Ok(JsValue::Undefined);
        }
        let argv = self.eval_args(args, e)?;
        self.call_value(&f, this, argv)
    }
    fn eval_args(&mut self, xs: &[ast::Expr], e: &EnvHandle) -> Result<Vec<JsValue>, Throw> {
        let mut out = Vec::new();
        for x in xs {
            if let ExprKind::Spread(y) = &x.kind {
                let v = self.eval_expr(y, e)?;
                out.extend(self.iterate(&v)?)
            } else {
                out.push(self.eval_expr(x, e)?);
            }
        }
        Ok(out)
    }
    fn member_key(&mut self, p: &MemberProp, e: &EnvHandle) -> Result<String, Throw> {
        match p {
            MemberProp::Ident(i) => Ok(i.name.clone()),
            MemberProp::Computed(x) => {
                let v = self.eval_expr(x, e)?;
                self.to_property_key(&v)
            }
        }
    }
    fn prop_key(&mut self, p: &PropKey, e: &EnvHandle) -> Result<String, Throw> {
        match p {
            PropKey::Ident(i) => Ok(i.name.clone()),
            PropKey::Str(s) => Ok(s.clone()),
            PropKey::Num(n) => Ok(number_to_string(self.numeric(n)?.to_number()?)),
            PropKey::Computed(x) => {
                let v = self.eval_expr(x, e)?;
                self.to_property_key(&v)
            }
        }
    }

    pub fn call_value(
        &mut self,
        f: &JsValue,
        this: JsValue,
        args: Vec<JsValue>,
    ) -> Result<JsValue, Throw> {
        let o = match f {
            JsValue::Object(o) => o.clone(),
            _ => return Err(Throw::type_error("value is not callable")),
        };
        let fun = o
            .borrow()
            .func
            .clone()
            .ok_or_else(|| Throw::type_error("value is not callable"))?;
        if self.depth >= MAX_CALL_DEPTH {
            return Err(Throw::range_error("maximum call stack size exceeded"));
        }
        self.depth += 1;
        let r = match fun.body {
            FunctionBody::Native(n) => n(self, this, args),
            FunctionBody::Expr(x) => {
                let e = EnvHandle::new(EnvKind::Function, Some(fun.env.clone()));
                e.bind_this(if fun.is_arrow {
                    fun.env.resolve_this()
                } else {
                    this
                });
                self.bind_params(&e, &fun.params, args)?;
                self.eval_expr(&x, &e)
            }
            FunctionBody::Block(b) => {
                let e = EnvHandle::new(EnvKind::Function, Some(fun.env.clone()));
                e.bind_this(if fun.is_arrow {
                    fun.env.resolve_this()
                } else {
                    this
                });
                self.bind_params(&e, &fun.params, args)?;
                self.hoist(&b.body, &e)?;
                match self.exec_stmts(&b.body, &e)? {
                    Flow::Return(v) => Ok(v),
                    _ => Ok(JsValue::Undefined),
                }
            }
        };
        self.depth -= 1;
        r
    }
    fn bind_params(
        &mut self,
        e: &EnvHandle,
        ps: &[ast::Param],
        args: Vec<JsValue>,
    ) -> Result<(), Throw> {
        for (i, p) in ps.iter().enumerate() {
            let Binding::Ident(id) = &p.binding;
            let v = if let Some(v) = args.get(i) {
                v.clone()
            } else if let Some(d) = &p.default {
                self.eval_expr(d, e)?
            } else {
                JsValue::Undefined
            };
            e.declare_lexical(&id.name, v, true)?;
        }
        let a = JsValue::object(JsObject::array());
        for (i, v) in args.into_iter().enumerate() {
            self.set_prop(&a, &i.to_string(), v)?;
        }
        self.set_prop(&a, "length", JsValue::num(ps.len() as f64))?;
        e.declare_var("arguments", a)?;
        Ok(())
    }
    fn construct(&mut self, f: &JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
        let o = match f {
            JsValue::Object(o) => o.clone(),
            _ => return Err(Throw::type_error("constructor is not an object")),
        };
        let fun = o
            .borrow()
            .func
            .clone()
            .ok_or_else(|| Throw::type_error("not a constructor"))?;
        if fun.is_arrow {
            return Err(Throw::type_error("arrow is not a constructor"));
        }
        let proto = match o.borrow().get_own("prototype") {
            Some(JsValue::Object(p)) => p,
            _ => self.object_proto.clone(),
        };
        let obj = Obj(Rc::new(RefCell::new({
            let mut x = JsObject::ordinary();
            x.proto = Some(proto);
            x
        })));
        let v = self.call_value(f, JsValue::Object(obj.clone()), args)?;
        if v.is_object() {
            Ok(v)
        } else {
            Ok(JsValue::Object(obj))
        }
    }
    fn make_function(&self, f: &ast::FunctionItem, e: &EnvHandle) -> JsValue {
        let mut o = JsObject::function(JsFunction {
            name: f.name.as_ref().map(|x| x.name.clone()).unwrap_or_default(),
            params: f.params.clone(),
            body: FunctionBody::Block(f.body.clone()),
            is_arrow: false,
            is_async: f.is_async,
            is_generator: f.is_generator,
            env: e.clone(),
            home: None,
            proto_for_new: None,
        });
        o.proto = Some(self.function_proto.clone());
        let h = Obj(Rc::new(RefCell::new(o)));
        let p = Obj(Rc::new(RefCell::new({
            let mut x = JsObject::ordinary();
            x.proto = Some(self.object_proto.clone());
            x
        })));
        h.borrow_mut()
            .set_own("prototype", JsValue::Object(p.clone()));
        h.borrow_mut().func.as_mut().unwrap().proto_for_new = Some(p);
        JsValue::Object(h)
    }
    fn make_arrow(&self, a: &ast::ArrowFunc, e: &EnvHandle) -> JsValue {
        let body = match &a.body {
            ast::ArrowBody::Expr(x) => FunctionBody::Expr((**x).clone()),
            ast::ArrowBody::Block(b) => FunctionBody::Block(b.clone()),
        };
        let mut o = JsObject::function(JsFunction {
            name: "".into(),
            params: a.params.clone(),
            body,
            is_arrow: true,
            is_async: a.is_async,
            is_generator: false,
            env: e.clone(),
            home: None,
            proto_for_new: None,
        });
        o.proto = Some(self.function_proto.clone());
        JsValue::Object(Obj(Rc::new(RefCell::new(o))))
    }
    fn method_value(&self, m: &ast::ClassMethod, e: &EnvHandle) -> JsValue {
        let mut o = JsObject::function(JsFunction {
            name: "".into(),
            params: m.params.clone(),
            body: FunctionBody::Block(m.body.clone()),
            is_arrow: false,
            is_async: m.is_async,
            is_generator: m.is_generator,
            env: e.clone(),
            home: None,
            proto_for_new: None,
        });
        o.proto = Some(self.function_proto.clone());
        JsValue::Object(Obj(Rc::new(RefCell::new(o))))
    }
    fn eval_class(&mut self, c: &ast::ClassItem, e: &EnvHandle) -> Result<JsValue, Throw> {
        let ctor = c.members.iter().find_map(|m| {
            if let ast::ClassMember::Method(x) = m {
                (x.kind == ast::MethodKind::Constructor).then_some(x)
            } else {
                None
            }
        });
        let f = if let Some(x) = ctor {
            self.method_value(x, e)
        } else {
            self.make_function(
                &ast::FunctionItem {
                    name: c.name.clone(),
                    params: Vec::new(),
                    body: ast::Block {
                        body: Vec::new(),
                        span: crate::tokens::Span::new(
                            crate::tokens::SourceLocation::new(1, 1),
                            crate::tokens::SourceLocation::new(1, 1),
                        ),
                    },
                    is_async: false,
                    is_generator: false,
                },
                e,
            )
        };
        if let JsValue::Object(o) = &f {
            if let Some(JsValue::Object(p)) = o.borrow().get_own("prototype") {
                for m in &c.members {
                    if let ast::ClassMember::Method(x) = m {
                        if x.kind != ast::MethodKind::Constructor {
                            let k = self.prop_key(&x.key, e)?;
                            self.set_prop(
                                &JsValue::Object(p.clone()),
                                &k,
                                self.method_value(x, e),
                            )?;
                        }
                    }
                }
            }
        }
        Ok(f)
    }

    fn object_value(&self) -> JsValue {
        let mut o = JsObject::ordinary();
        o.proto = Some(self.object_proto.clone());
        JsValue::object(o)
    }
    fn get_prop(&mut self, v: &JsValue, k: &str) -> Result<JsValue, Throw> {
        match v {
            JsValue::Object(o) => {
                if k == "size" {
                    let size = match &o.borrow().exotic {
                        Exotic::Map(state) => Some(state.entries.len()),
                        Exotic::Set(state) => Some(state.entries.len()),
                        _ => None,
                    };
                    if let Some(size) = size {
                        return Ok(JsValue::num(size as f64));
                    }
                }
                if let Some(x) = o.borrow().get_own(k) {
                    Ok(x)
                } else if let Some(p) = o.borrow().proto.clone() {
                    self.get_prop(&JsValue::Object(p), k)
                } else {
                    Ok(JsValue::Undefined)
                }
            }
            JsValue::Str(s) => {
                if k == "length" {
                    Ok(JsValue::num(s.chars().count() as f64))
                } else if let Ok(n) = k.parse::<usize>() {
                    Ok(s.chars()
                        .nth(n)
                        .map(|c| JsValue::str(c.to_string()))
                        .unwrap_or(JsValue::Undefined))
                } else {
                    Ok(JsValue::Undefined)
                }
            }
            _ => Ok(JsValue::Undefined),
        }
    }
    fn set_prop(&mut self, v: &JsValue, k: &str, x: JsValue) -> Result<(), Throw> {
        let o = match v {
            JsValue::Object(o) => o.clone(),
            _ => return Err(Throw::type_error("cannot set property on primitive")),
        };
        if k == "length" && matches!(o.borrow().exotic, Exotic::Array) {
            let n = self.to_number(&x)?;
            if !n.is_finite() || n < 0.0 || n.fract() != 0.0 {
                return Err(Throw::range_error("invalid array length"));
            }
        }
        o.borrow_mut().set_own(k, x);
        if let Ok(n) = k.parse::<usize>() {
            if matches!(o.borrow().exotic, Exotic::Array) {
                let len = match o.borrow().get_own("length") {
                    Some(JsValue::Num(n)) => n,
                    _ => 0.0,
                };
                if n as f64 + 1.0 > len {
                    o.borrow_mut()
                        .set_own("length", JsValue::num(n as f64 + 1.0));
                }
            }
        }
        Ok(())
    }
    fn enumerable_keys(&self, v: &JsValue) -> Vec<String> {
        match v {
            JsValue::Object(o) => o.borrow().own_keys_enumerable(),
            JsValue::Str(s) => (0..s.chars().count()).map(|i| i.to_string()).collect(),
            _ => Vec::new(),
        }
    }
    fn array_length(&self, v: &JsValue) -> usize {
        match v {
            JsValue::Object(o) => match o.borrow().get_own("length") {
                Some(JsValue::Num(n)) if n >= 0.0 => n as usize,
                _ => 0,
            },
            _ => 0,
        }
    }
    fn iterate(&mut self, v: &JsValue) -> Result<Vec<JsValue>, Throw> {
        match v {
            JsValue::Object(o) if matches!(o.borrow().exotic, Exotic::Array) => {
                let n = match o.borrow().get_own("length") {
                    Some(JsValue::Num(n)) => n as usize,
                    _ => 0,
                };
                Ok((0..n)
                    .map(|n| self.get_prop(v, &n.to_string()))
                    .collect::<Result<_, _>>()?)
            }
            JsValue::Str(s) => Ok(s.chars().map(|c| JsValue::str(c.to_string())).collect()),
            _ => Err(Throw::type_error("value is not iterable")),
        }
    }
    fn typeof_value(&self, v: &JsValue) -> &'static str {
        match v {
            JsValue::Undefined => "undefined",
            JsValue::Null => "object",
            JsValue::Bool(_) => "boolean",
            JsValue::Num(_) => "number",
            JsValue::Str(_) => "string",
            JsValue::Object(o) => {
                if o.borrow().func.is_some() {
                    "function"
                } else {
                    "object"
                }
            }
        }
    }
    fn primitive(&mut self, v: JsValue) -> Result<JsValue, Throw> {
        if !v.is_object() {
            return Ok(v);
        };
        for n in ["valueOf", "toString"] {
            let f = self.get_prop(&v, n)?;
            if matches!(f,JsValue::Object(ref o)if o.borrow().func.is_some()) {
                let x = self.call_value(&f, v.clone(), Vec::new())?;
                if !x.is_object() {
                    return Ok(x);
                }
            }
        }
        Err(Throw::type_error("cannot convert object to primitive"))
    }
    fn to_number(&mut self, v: &JsValue) -> Result<f64, Throw> {
        if v.is_object() {
            let p = self.primitive(v.clone())?;
            self.to_number(&p)
        } else {
            v.to_number()
        }
    }
    fn to_string(&mut self, v: &JsValue) -> Result<String, Throw> {
        if v.is_object() {
            let p = self.primitive(v.clone())?;
            self.to_string(&p)
        } else {
            Ok(crate::value::to_display_string(v))
        }
    }
    fn to_property_key(&mut self, v: &JsValue) -> Result<String, Throw> {
        self.to_string(v)
    }
    fn to_int32(&mut self, v: &JsValue) -> Result<i32, Throw> {
        let n = self.to_number(v)?;
        if !n.is_finite() || n == 0.0 {
            Ok(0)
        } else {
            Ok(n.trunc().rem_euclid(4294967296.0) as u32 as i32)
        }
    }
    fn to_uint32(&mut self, v: &JsValue) -> Result<u32, Throw> {
        Ok(self.to_int32(v)? as u32)
    }
    fn strict_eq(&self, a: &JsValue, b: &JsValue) -> bool {
        a == b
    }
    fn loose_eq(&mut self, a: &JsValue, b: &JsValue) -> Result<bool, Throw> {
        use JsValue::*;
        Ok(match (a, b) {
            (Undefined, Null) | (Null, Undefined) | (Undefined, Undefined) | (Null, Null) => true,
            (Num(x), Num(y)) => x == y,
            (Str(x), Str(y)) => x == y,
            (Bool(_), _) => {
                let n = a.to_number()?;
                self.loose_eq(&Num(n), b)?
            }
            (_, Bool(_)) => {
                let n = b.to_number()?;
                self.loose_eq(a, &Num(n))?
            }
            (Num(_) | Str(_), Object(_)) => {
                let p = self.primitive(b.clone())?;
                self.loose_eq(a, &p)?
            }
            (Object(_), Num(_) | Str(_)) => {
                let p = self.primitive(a.clone())?;
                self.loose_eq(&p, b)?
            }
            (Object(x), Object(y)) => Rc::ptr_eq(&x.0, &y.0),
            _ => false,
        })
    }
    fn instanceof(&mut self, a: &JsValue, b: &JsValue) -> Result<JsValue, Throw> {
        let r = match b {
            JsValue::Object(o) => o.clone(),
            _ => return Err(Throw::type_error("right operand is not callable")),
        };
        let p = match r.borrow().get_own("prototype") {
            Some(JsValue::Object(p)) => p,
            _ => return Err(Throw::type_error("prototype is not an object")),
        };
        let mut c = match a {
            JsValue::Object(o) => o.borrow().proto.clone(),
            _ => None,
        };
        while let Some(o) = c {
            if Rc::ptr_eq(&o.0, &p.0) {
                return Ok(JsValue::Bool(true));
            }
            c = o.borrow().proto.clone()
        }
        Ok(JsValue::Bool(false))
    }

    fn native(name: &str, f: crate::value::NativeFn, proto: &Obj) -> JsValue {
        let mut o = JsObject::function(JsFunction {
            name: name.into(),
            params: Vec::new(),
            body: FunctionBody::Native(f),
            is_arrow: false,
            is_async: false,
            is_generator: false,
            env: EnvHandle::new(EnvKind::Global, None),
            home: None,
            proto_for_new: None,
        });
        o.proto = Some(proto.clone());
        JsValue::Object(Obj(Rc::new(RefCell::new(o))))
    }
    fn install_builtins(&mut self) {
        self.define_global("undefined", JsValue::Undefined);
        self.define_global("NaN", JsValue::Num(f64::NAN));
        self.define_global("Infinity", JsValue::Num(f64::INFINITY));
        let object_ctor = Self::native("Object", native_object, &self.function_proto);
        if let JsValue::Object(o) = &object_ctor {
            let _ = self.set_prop(
                &JsValue::Object(o.clone()),
                "keys",
                Self::native("keys", native_object_keys, &self.function_proto),
            );
        }
        self.define_global("Object", object_ctor);
        self.define_global(
            "Array",
            Self::native("Array", native_array, &self.function_proto),
        );
        self.define_global("Map", Self::native("Map", native_map, &self.function_proto));
        self.define_global("Set", Self::native("Set", native_set, &self.function_proto));
        let _ = self.set_prop(
            &JsValue::Object(self.object_proto.clone()),
            "toString",
            Self::native("toString", native_object_to_string, &self.function_proto),
        );
        let _ = self.set_prop(
            &JsValue::Object(self.object_proto.clone()),
            "valueOf",
            Self::native("valueOf", native_object_value_of, &self.function_proto),
        );
        for (n, f) in [
            ("push", native_array_push as crate::value::NativeFn),
            ("pop", native_array_pop as crate::value::NativeFn),
            ("join", native_array_join as crate::value::NativeFn),
        ] {
            let _ = self.set_prop(
                &JsValue::Object(self.array_proto.clone()),
                n,
                Self::native(n, f, &self.function_proto),
            );
        }
        for (n, f) in [
            ("get", native_map_get as crate::value::NativeFn),
            ("set", native_map_set as crate::value::NativeFn),
            ("has", native_map_has as crate::value::NativeFn),
            ("delete", native_map_delete as crate::value::NativeFn),
            ("clear", native_map_clear as crate::value::NativeFn),
            ("keys", native_map_keys as crate::value::NativeFn),
            ("values", native_map_values as crate::value::NativeFn),
            ("entries", native_map_entries as crate::value::NativeFn),
        ] {
            let _ = self.set_prop(
                &JsValue::Object(self.map_proto.clone()),
                n,
                Self::native(n, f, &self.function_proto),
            );
        }
        for (n, f) in [
            ("add", native_set_add as crate::value::NativeFn),
            ("has", native_set_has as crate::value::NativeFn),
            ("delete", native_set_delete as crate::value::NativeFn),
            ("clear", native_set_clear as crate::value::NativeFn),
            ("keys", native_set_values as crate::value::NativeFn),
            ("values", native_set_values as crate::value::NativeFn),
            ("entries", native_set_entries as crate::value::NativeFn),
        ] {
            let _ = self.set_prop(
                &JsValue::Object(self.set_proto.clone()),
                n,
                Self::native(n, f, &self.function_proto),
            );
        }
        let math = self.object_value();
        for (n, f) in [
            ("abs", native_abs as crate::value::NativeFn),
            ("max", native_max as crate::value::NativeFn),
            ("min", native_min as crate::value::NativeFn),
        ] {
            let _ = self.set_prop(&math, n, Self::native(n, f, &self.function_proto));
        }
        self.define_global("Math", math);
        let console = self.object_value();
        let _ = self.set_prop(
            &console,
            "log",
            Self::native("log", native_log, &self.function_proto),
        );
        self.define_global("console", console);
        self.define_global(
            "Number",
            Self::native("Number", native_number, &self.function_proto),
        );
        self.define_global(
            "String",
            Self::native("String", native_string, &self.function_proto),
        );
        self.define_global(
            "Boolean",
            Self::native("Boolean", native_boolean, &self.function_proto),
        );
    }
    fn define_global(&self, n: &str, v: JsValue) {
        self.global_obj.borrow_mut().set_own(n, v.clone());
        let _ = self.global_env.declare_var(n, v);
    }
}

fn native_object(i: &mut Interpreter, _: JsValue, a: Vec<JsValue>) -> Result<JsValue, Throw> {
    Ok(a.into_iter()
        .next()
        .filter(|v| v.is_object())
        .unwrap_or_else(|| i.object_value()))
}

fn map_key(value: &JsValue) -> MapKey {
    match value {
        JsValue::Undefined => MapKey::Undefined,
        JsValue::Null => MapKey::Null,
        JsValue::Bool(v) => MapKey::Bool(*v),
        JsValue::Num(v) => MapKey::Num(*v),
        JsValue::Str(v) => MapKey::Str(v.clone()),
        JsValue::Object(v) => MapKey::Object(v.clone()),
    }
}

fn require_map(value: &JsValue) -> Result<Obj, Throw> {
    match value {
        JsValue::Object(o) if matches!(o.borrow().exotic, Exotic::Map(_)) => Ok(o.clone()),
        _ => Err(Throw::type_error("Map method called on a non-Map object")),
    }
}

fn require_set(value: &JsValue) -> Result<Obj, Throw> {
    match value {
        JsValue::Object(o) if matches!(o.borrow().exotic, Exotic::Set(_)) => Ok(o.clone()),
        _ => Err(Throw::type_error("Set method called on a non-Set object")),
    }
}

fn array_values(i: &mut Interpreter, values: Vec<JsValue>) -> Result<JsValue, Throw> {
    let out = JsValue::object({
        let mut o = JsObject::array();
        o.proto = Some(i.array_proto.clone());
        o
    });
    for (n, value) in values.into_iter().enumerate() {
        i.set_prop(&out, &n.to_string(), value)?;
    }
    Ok(out)
}

fn map_set_raw(map: &Obj, key: MapKey, value: JsValue) {
    let mut object = map.borrow_mut();
    if let Exotic::Map(state) = &mut object.exotic {
        if let Some((_, slot)) = state.entries.iter_mut().find(|(k, _)| k.same(&key)) {
            *slot = value;
        } else {
            state.entries.push((key, value));
        }
    }
}

fn native_map(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let map = Obj(Rc::new(RefCell::new({
        let mut o = JsObject::ordinary();
        o.exotic = Exotic::Map(MapState::default());
        o.proto = Some(i.map_proto.clone());
        o
    })));
    if let Some(source) = args.first() {
        if !source.is_null() && !source.is_undefined() {
            for entry in i.iterate(source)? {
                let key = i.get_prop(&entry, "0")?;
                let value = i.get_prop(&entry, "1")?;
                map_set_raw(&map, map_key(&key), value);
            }
        }
    }
    Ok(JsValue::Object(map))
}

fn native_map_get(
    _: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let map = require_map(&this)?;
    let key = map_key(args.first().unwrap_or(&JsValue::Undefined));
    let object = map.borrow();
    if let Exotic::Map(state) = &object.exotic {
        if let Some((_, value)) = state.entries.iter().find(|(k, _)| k.same(&key)) {
            return Ok(value.clone());
        }
    }
    Ok(JsValue::Undefined)
}

fn native_map_set(
    _: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let map = require_map(&this)?;
    let key = map_key(args.first().unwrap_or(&JsValue::Undefined));
    let value = args.get(1).cloned().unwrap_or(JsValue::Undefined);
    map_set_raw(&map, key, value);
    Ok(this)
}

fn native_map_has(
    _: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let map = require_map(&this)?;
    let key = map_key(args.first().unwrap_or(&JsValue::Undefined));
    let object = map.borrow();
    let found = matches!(&object.exotic, Exotic::Map(state) if state.entries.iter().any(|(k, _)| k.same(&key)));
    Ok(JsValue::Bool(found))
}

fn native_map_delete(
    _: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let map = require_map(&this)?;
    let key = map_key(args.first().unwrap_or(&JsValue::Undefined));
    let mut object = map.borrow_mut();
    let deleted = if let Exotic::Map(state) = &mut object.exotic {
        if let Some(pos) = state.entries.iter().position(|(k, _)| k.same(&key)) {
            state.entries.remove(pos);
            true
        } else {
            false
        }
    } else {
        false
    };
    Ok(JsValue::Bool(deleted))
}

fn native_map_clear(_: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let map = require_map(&this)?;
    if let Exotic::Map(state) = &mut map.borrow_mut().exotic {
        state.entries.clear();
    }
    Ok(JsValue::Undefined)
}

fn native_map_keys(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let map = require_map(&this)?;
    let values = if let Exotic::Map(state) = &map.borrow().exotic {
        state
            .entries
            .iter()
            .map(|(key, _)| key_value(key))
            .collect()
    } else {
        Vec::new()
    };
    array_values(i, values)
}

fn native_map_values(
    i: &mut Interpreter,
    this: JsValue,
    _: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let map = require_map(&this)?;
    let values = if let Exotic::Map(state) = &map.borrow().exotic {
        state
            .entries
            .iter()
            .map(|(_, value)| value.clone())
            .collect()
    } else {
        Vec::new()
    };
    array_values(i, values)
}

fn native_map_entries(
    i: &mut Interpreter,
    this: JsValue,
    _: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let map = require_map(&this)?;
    let entries = if let Exotic::Map(state) = &map.borrow().exotic {
        state
            .entries
            .iter()
            .map(|(key, value)| {
                array_values(i, vec![key_value(key), value.clone()]).unwrap_or(JsValue::Undefined)
            })
            .collect()
    } else {
        Vec::new()
    };
    array_values(i, entries)
}

fn key_value(key: &MapKey) -> JsValue {
    match key {
        MapKey::Undefined => JsValue::Undefined,
        MapKey::Null => JsValue::Null,
        MapKey::Bool(v) => JsValue::Bool(*v),
        MapKey::Num(v) => JsValue::Num(*v),
        MapKey::Str(v) => JsValue::Str(v.clone()),
        MapKey::Object(v) => JsValue::Object(v.clone()),
    }
}

fn native_set(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let set = Obj(Rc::new(RefCell::new({
        let mut o = JsObject::ordinary();
        o.exotic = Exotic::Set(SetState::default());
        o.proto = Some(i.set_proto.clone());
        o
    })));
    if let Some(source) = args.first() {
        if !source.is_null() && !source.is_undefined() {
            for value in i.iterate(source)? {
                let key = map_key(&value);
                if let Exotic::Set(state) = &mut set.borrow_mut().exotic {
                    if !state.entries.iter().any(|existing| existing.same(&key)) {
                        state.entries.push(key);
                    }
                }
            }
        }
    }
    Ok(JsValue::Object(set))
}

fn native_set_add(
    _: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let set = require_set(&this)?;
    let key = map_key(args.first().unwrap_or(&JsValue::Undefined));
    if let Exotic::Set(state) = &mut set.borrow_mut().exotic {
        if !state.entries.iter().any(|existing| existing.same(&key)) {
            state.entries.push(key);
        }
    }
    Ok(this)
}

fn native_set_has(
    _: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let set = require_set(&this)?;
    let key = map_key(args.first().unwrap_or(&JsValue::Undefined));
    let found = matches!(&set.borrow().exotic, Exotic::Set(state) if state.entries.iter().any(|existing| existing.same(&key)));
    Ok(JsValue::Bool(found))
}

fn native_set_delete(
    _: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let set = require_set(&this)?;
    let key = map_key(args.first().unwrap_or(&JsValue::Undefined));
    let mut object = set.borrow_mut();
    let deleted = if let Exotic::Set(state) = &mut object.exotic {
        if let Some(pos) = state
            .entries
            .iter()
            .position(|existing| existing.same(&key))
        {
            state.entries.remove(pos);
            true
        } else {
            false
        }
    } else {
        false
    };
    Ok(JsValue::Bool(deleted))
}

fn native_set_clear(_: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let set = require_set(&this)?;
    if let Exotic::Set(state) = &mut set.borrow_mut().exotic {
        state.entries.clear();
    }
    Ok(JsValue::Undefined)
}

fn native_set_values(
    i: &mut Interpreter,
    this: JsValue,
    _: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let set = require_set(&this)?;
    let values = if let Exotic::Set(state) = &set.borrow().exotic {
        state.entries.iter().map(key_value).collect()
    } else {
        Vec::new()
    };
    array_values(i, values)
}

fn native_set_entries(
    i: &mut Interpreter,
    this: JsValue,
    _: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let set = require_set(&this)?;
    let entries = if let Exotic::Set(state) = &set.borrow().exotic {
        state
            .entries
            .iter()
            .map(|key| {
                array_values(i, vec![key_value(key), key_value(key)]).unwrap_or(JsValue::Undefined)
            })
            .collect()
    } else {
        Vec::new()
    };
    array_values(i, entries)
}
fn native_object_keys(i: &mut Interpreter, _: JsValue, a: Vec<JsValue>) -> Result<JsValue, Throw> {
    let src = a.first().cloned().unwrap_or(JsValue::Undefined);
    let out = JsValue::object(JsObject::array());
    for (k, n) in i.enumerable_keys(&src).into_iter().enumerate() {
        i.set_prop(&out, &k.to_string(), JsValue::str(n))?;
    }
    Ok(out)
}
fn native_object_to_string(
    i: &mut Interpreter,
    this: JsValue,
    _: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let s = match this {
        JsValue::Object(o) if matches!(o.borrow().exotic, Exotic::Array) => "[object Array]",
        JsValue::Object(_) => "[object Object]",
        JsValue::Null => "[object Null]",
        JsValue::Undefined => "[object Undefined]",
        _ => "[object Object]",
    };
    let _ = i;
    Ok(JsValue::str(s))
}
fn native_object_value_of(
    _: &mut Interpreter,
    this: JsValue,
    _: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    Ok(this)
}
fn native_array_push(
    i: &mut Interpreter,
    this: JsValue,
    a: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let mut n = i.array_length(&this);
    for v in a {
        i.set_prop(&this, &n.to_string(), v)?;
        n += 1;
    }
    Ok(JsValue::num(n as f64))
}
fn native_array_pop(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let n = i.array_length(&this);
    if n == 0 {
        return Ok(JsValue::Undefined);
    }
    let k = (n - 1).to_string();
    let v = i.get_prop(&this, &k)?;
    i.set_prop(&this, "length", JsValue::num((n - 1) as f64))?;
    if let JsValue::Object(o) = this {
        o.borrow_mut().delete_own(&k);
    }
    Ok(v)
}
fn native_array_join(
    i: &mut Interpreter,
    this: JsValue,
    a: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let sep = if let Some(v) = a.first() {
        i.to_string(v)?
    } else {
        " ,".trim().to_string()
    };
    let mut out = String::new();
    for n in 0..i.array_length(&this) {
        if n > 0 {
            out.push_str(&sep)
        }
        let v = i.get_prop(&this, &n.to_string())?;
        if !v.is_null() && !v.is_undefined() {
            out.push_str(&i.to_string(&v)?);
        }
    }
    Ok(JsValue::str(out))
}
fn native_array(i: &mut Interpreter, _: JsValue, a: Vec<JsValue>) -> Result<JsValue, Throw> {
    let v = JsValue::object({
        let mut o = JsObject::array();
        o.proto = Some(i.array_proto.clone());
        o
    });
    for (n, x) in a.into_iter().enumerate() {
        i.set_prop(&v, &n.to_string(), x)?;
    }
    Ok(v)
}
fn native_abs(i: &mut Interpreter, _: JsValue, a: Vec<JsValue>) -> Result<JsValue, Throw> {
    Ok(JsValue::num(
        i.to_number(a.first().unwrap_or(&JsValue::Undefined))?.abs(),
    ))
}
fn native_max(i: &mut Interpreter, _: JsValue, a: Vec<JsValue>) -> Result<JsValue, Throw> {
    let mut n = f64::NEG_INFINITY;
    for x in &a {
        n = n.max(i.to_number(x)?);
    }
    Ok(JsValue::num(n))
}
fn native_min(i: &mut Interpreter, _: JsValue, a: Vec<JsValue>) -> Result<JsValue, Throw> {
    let mut n = f64::INFINITY;
    for x in &a {
        n = n.min(i.to_number(x)?);
    }
    Ok(JsValue::num(n))
}
fn native_number(i: &mut Interpreter, _: JsValue, a: Vec<JsValue>) -> Result<JsValue, Throw> {
    Ok(JsValue::num(
        i.to_number(a.first().unwrap_or(&JsValue::Undefined))?,
    ))
}
fn native_string(i: &mut Interpreter, _: JsValue, a: Vec<JsValue>) -> Result<JsValue, Throw> {
    Ok(JsValue::str(
        i.to_string(a.first().unwrap_or(&JsValue::Undefined))?,
    ))
}
fn native_boolean(_: &mut Interpreter, _: JsValue, a: Vec<JsValue>) -> Result<JsValue, Throw> {
    Ok(JsValue::Bool(
        a.first().map(|x| x.to_boolean()).unwrap_or(false),
    ))
}
fn native_log(i: &mut Interpreter, _: JsValue, a: Vec<JsValue>) -> Result<JsValue, Throw> {
    let mut s = String::new();
    for (n, x) in a.iter().enumerate() {
        if n > 0 {
            s.push(' ')
        }
        s.push_str(&i.to_string(x)?);
    }
    i.output.borrow_mut().push(s);
    Ok(JsValue::Undefined)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn num(v: JsValue) -> f64 {
        match v {
            JsValue::Num(n) => n,
            _ => panic!("expected number: {v:?}"),
        }
    }

    #[test]
    fn evaluates_arithmetic_and_coercion() {
        let mut i = Interpreter::new();
        assert_eq!(num(i.run_script("1 + 2 * 3").unwrap()), 7.0);
        let v = i.run_script("'a' + 2").unwrap();
        assert_eq!(i.to_string(&v).unwrap(), "a2");
    }

    #[test]
    fn closures_capture_and_mutate_lexical_bindings() {
        let mut i = Interpreter::new();
        let v = i
            .run_script("function make(){ let n=0; return ()=>++n; } let f=make(); f()+f();")
            .unwrap();
        assert_eq!(num(v), 3.0);
    }

    #[test]
    fn arrays_objects_and_builtin_methods_work() {
        let mut i = Interpreter::new();
        let v = i
            .run_script("let a=[1,2]; a.push(3); a.join('-');")
            .unwrap();
        assert_eq!(i.to_string(&v).unwrap(), "1-2-3");
        let v = i.run_script("Object.keys({a:1,b:2}).length").unwrap();
        assert_eq!(num(v), 2.0);
    }

    #[test]
    fn map_uses_same_value_zero_and_preserves_insertion_order() {
        let mut i = Interpreter::new();
        let v = i
            .run_script(
                "let m=new Map([['a',1],['b',2]]); m.set('a',9).set('c',3); m.get('a') + m.size;",
            )
            .unwrap();
        assert_eq!(num(v), 12.0);
        let v = i
            .run_script("new Map([['a',1],['b',2]]).keys().join(',');")
            .unwrap();
        assert_eq!(i.to_string(&v).unwrap(), "a,b");
        let v = i
            .run_script("let m2=new Map(); m2.set(NaN,1); m2.has(NaN);")
            .unwrap();
        assert!(matches!(v, JsValue::Bool(true)));
    }

    #[test]
    fn set_deduplicates_same_value_zero_and_supports_mutation() {
        let mut i = Interpreter::new();
        let v = i
            .run_script("let s=new Set([1,1,2,NaN,NaN]); s.add(3); s.delete(2); s.size;")
            .unwrap();
        assert_eq!(num(v), 3.0);
        let v = i
            .run_script("new Set(['x','y']).values().join('-');")
            .unwrap();
        assert_eq!(i.to_string(&v).unwrap(), "x-y");
    }

    #[test]
    fn loops_and_try_catch_are_executable() {
        let mut i = Interpreter::new();
        let v = i
            .run_script(
                "let s=0; for(let n=0;n<5;n++){s+=n;} try { throw 4; } catch(e) { s+=e; } s;",
            )
            .unwrap();
        assert_eq!(num(v), 14.0);
    }

    #[test]
    fn console_output_is_captured_without_io() {
        let mut i = Interpreter::new();
        i.run_script("console.log('hello', 3);").unwrap();
        assert_eq!(&*i.output.borrow(), &["hello 3".to_string()]);
    }
}
