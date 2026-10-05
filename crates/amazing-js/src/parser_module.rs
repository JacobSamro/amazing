//! Module grammar goal: top-level import/export declarations and early errors.
use super::*;
use crate::tokens::Keyword as K;

impl Parser {
    pub(super) fn parse_module(mut self) -> Result<Module, SyntaxError> {
        let start = self.cur_loc();
        let mut items = Vec::new();
        let mut declarations = Vec::new();
        let mut imports = HashSet::new();
        let mut exports = HashSet::new();
        while !self.at_eof() {
            let item = if self.at_kw(K::Import)
                && !self.nth_punct(1, Punct::LParen)
                && !self.nth_punct(1, Punct::Dot)
            {
                ModuleItem::Import(self.parse_import_declaration(&mut imports)?)
            } else if self.at_punct(Punct::At) {
                self.parse_decorated_module_item(&mut exports)?
            } else if self.at_kw(K::Export) {
                self.parse_export_declaration(&mut exports)?
            } else {
                let statement = self.parse_stmt()?;
                ModuleItem::Statement(statement)
            };
            if let ModuleItem::Statement(stmt) | ModuleItem::ExportNamed {
                declaration: Some(stmt),
                ..
            }
            | ModuleItem::ExportDefaultDeclaration(stmt) = &item
            {
                declarations.push(stmt.clone());
            }
            items.push(item);
        }
        self.validate_scope_names(&declarations, true)?;
        self.validate_strict_statements(&declarations)?;
        if let Some(at) = self.tokens.borrow().legacy_escape_in(self.span_from(start)) {
            return Err(self.err_at(at, "legacy string escape is forbidden in modules"));
        }
        if let Some(at) = self.tokens.borrow().legacy_numeric_in(self.span_from(start)) {
            return Err(self.err_at(at, "legacy leading-zero literal in strict mode"));
        }
        cover::validate_program(&declarations)?;
        for name in imports {
            if declarations
                .iter()
                .any(|stmt| statement_declares(stmt, &name))
            {
                return Err(self.err(format!("duplicate module binding {name}")));
            }
        }
        // A local export must refer to a binding declared in this module.
        for item in &items {
            if let ModuleItem::ExportNamed {
                names, from: None, ..
            } = item
            {
                for name in names {
                    if !declarations.iter().any(|stmt| statement_declares(stmt, &name.local))
                        && !items.iter().any(|item| matches!(item, ModuleItem::Import(import) if import.bindings.iter().any(|binding| binding.local.name == name.local)))
                    {
                        return Err(self.err(format!("unresolved export {}", name.local)));
                    }
                }
            }
        }
        Ok(Module {
            items,
            span: self.span_from(start),
            has_top_level_await: self.has_top_level_await,
        })
    }

    fn parse_import_declaration(
        &mut self,
        imports: &mut HashSet<String>,
    ) -> Result<ImportDeclaration, SyntaxError> {
        self.bump(); // import
        let phase = if (self.word("source")
            && ((matches!(self.nth(1).kind, TokenKind::Ident(_))
                && matches!(self.nth(2).kind, TokenKind::Ident(ref name) if name == "from"))
                || self.nth_punct(1, Punct::Star)
                || self.nth_punct(1, Punct::Lbrace)))
            || (self.word("defer") && self.nth_punct(1, Punct::Star))
        {
            match self.bump().kind {
                TokenKind::Ident(name) => Some(name),
                _ => unreachable!(),
            }
        } else {
            None
        };
        let mut bindings = Vec::new();
        let from = if matches!(self.cur().kind, TokenKind::Str(_)) {
            self.module_string()?
        } else {
            if !self.at_punct(Punct::Lbrace) && !self.at_punct(Punct::Star) {
                let local = self.expect_ident()?;
                bindings.push(ImportBinding {
                    imported: "default".into(),
                    local,
                });
                self.eat_punct(Punct::Comma);
            }
            if self.eat_punct(Punct::Star) {
                if !self.word("as") {
                    return Err(self.err("expected `as` after import *"));
                }
                self.bump();
                bindings.push(ImportBinding {
                    imported: "*".into(),
                    local: self.expect_ident()?,
                });
            } else if self.eat_punct(Punct::Lbrace) {
                while !self.at_punct(Punct::Rbrace) {
                    let imported_was_string = matches!(self.cur().kind, TokenKind::Str(_));
                    let imported = self.module_name()?;
                    let local = if self.word("as") {
                        self.bump();
                        self.expect_ident()?
                    } else if imported_was_string {
                        return Err(self.err("string import name requires `as` binding"));
                    } else {
                        Ident::new(imported.clone(), self.last_end_span())
                    };
                    bindings.push(ImportBinding { imported, local });
                    if !self.eat_punct(Punct::Comma) {
                        break;
                    }
                }
                self.expect_punct(Punct::Rbrace)?;
            }
            if !self.word("from") {
                return Err(self.err("expected `from` in import"));
            }
            self.bump();
            self.module_string()?
        };
        let attributes = self.import_attributes()?;
        self.semicolon()?;
        for binding in &bindings {
            if matches!(binding.local.name.as_str(), "eval" | "arguments") {
                return Err(self.err_at(binding.local.span.start, "invalid module import binding"));
            }
            if !imports.insert(binding.local.name.clone()) {
                return Err(self.err_at(binding.local.span.start, "duplicate import binding"));
            }
        }
        Ok(ImportDeclaration {
            from,
            phase,
            bindings,
            attributes,
        })
    }

    pub(super) fn parse_export_declaration(
        &mut self,
        exports: &mut HashSet<String>,
    ) -> Result<ModuleItem, SyntaxError> {
        let start = self.bump().span.start; // export
        if self.eat_punct(Punct::Star) {
            let as_name = if self.word("as") {
                self.bump();
                Some(self.module_name()?)
            } else {
                None
            };
            if !self.word("from") {
                return Err(self.err("expected `from` after export *"));
            }
            self.bump();
            let from = self.module_string()?;
            let attributes = self.import_attributes()?;
            self.semicolon()?;
            if let Some(name) = &as_name {
                self.register_export(exports, name, start)?;
            }
            return Ok(ModuleItem::ExportAll {
                as_name,
                from,
                attributes,
            });
        }
        if self.eat_punct(Punct::Lbrace) {
            let mut names = Vec::new();
            let mut string_locals = false;
            while !self.at_punct(Punct::Rbrace) {
                string_locals |= matches!(self.cur().kind, TokenKind::Str(_));
                let local = self.module_name()?;
                let exported = if self.word("as") {
                    self.bump();
                    self.module_name()?
                } else {
                    local.clone()
                };
                self.register_export(exports, &exported, start)?;
                names.push(ExportName { local, exported });
                if !self.eat_punct(Punct::Comma) {
                    break;
                }
            }
            self.expect_punct(Punct::Rbrace)?;
            let from = if self.word("from") {
                self.bump();
                Some(self.module_string()?)
            } else {
                None
            };
            if from.is_none() && string_locals {
                return Err(self.err_at(start, "string export name requires a from clause"));
            }
            let attributes = if from.is_some() {
                self.import_attributes()?
            } else {
                Vec::new()
            };
            self.semicolon()?;
            return Ok(ModuleItem::ExportNamed {
                declaration: None,
                names,
                from,
                attributes,
            });
        }
        if self.eat_kw(K::Default) {
            self.register_export(exports, "default", start)?;
            if self.at_kw(K::Function)
                || self.at_kw(K::Class)
                || self.at_punct(Punct::At)
                || self.word("async") && self.nth_is_function(1)
            {
                let kind = if self.at_kw(K::Class) || self.at_punct(Punct::At) {
                    StmtKind::Class(Box::new(self.parse_class_rest()?))
                } else {
                    let async_ = self.word("async");
                    if async_ {
                        self.bump();
                    }
                    StmtKind::Function(Box::new(self.parse_function_rest(async_, true)?))
                };
                return Ok(ModuleItem::ExportDefaultDeclaration(Stmt::new(
                    kind,
                    self.span_from(start),
                )));
            }
            let expression = self.parse_assignment_expr()?;
            self.semicolon()?;
            return Ok(ModuleItem::ExportDefaultExpression(expression));
        }
        let statement = self.parse_stmt()?;
        let names = declared_names(&statement);
        if names.is_empty() {
            return Err(self.err_at(start, "export requires a declaration"));
        }
        let names: Vec<_> = names
            .into_iter()
            .map(|name| ExportName {
                local: name.clone(),
                exported: name,
            })
            .collect();
        for name in &names {
            self.register_export(exports, &name.exported, start)?;
        }
        Ok(ModuleItem::ExportNamed {
            declaration: Some(statement),
            names,
            from: None,
            attributes: Vec::new(),
        })
    }

    fn register_export(
        &self,
        exports: &mut HashSet<String>,
        name: &str,
        at: SourceLocation,
    ) -> Result<(), SyntaxError> {
        if exports.insert(name.to_string()) {
            Ok(())
        } else {
            Err(self.err_at(at, format!("duplicate export {name}")))
        }
    }

    fn module_name(&mut self) -> Result<String, SyntaxError> {
        match self.cur().kind.clone() {
            TokenKind::Ident(name) => {
                self.bump();
                Ok(name)
            }
            TokenKind::Keyword(keyword) => {
                self.bump();
                Ok(keyword.spelling().into())
            }
            TokenKind::Str(value) => {
                if self
                    .cur()
                    .cooked_utf16
                    .as_ref()
                    .is_some_and(|units| !well_formed_utf16(units))
                {
                    return Err(self.err("module export name contains an unpaired surrogate"));
                }
                self.bump();
                Ok(value)
            }
            _ => Err(self.err("expected module export/import name")),
        }
    }

    fn module_string(&mut self) -> Result<crate::value::JsString, SyntaxError> {
        if let TokenKind::Str(value) = self.cur().kind.clone() {
            let value = self.cur().cooked_utf16.clone()
                .map(crate::value::JsString::from_units)
                .unwrap_or_else(|| value.into());
            self.bump();
            Ok(value)
        } else {
            Err(self.err("expected module specifier string"))
        }
    }

    // AttributeKey is PropName, not ModuleExportName: quoted keys keep lone
    // surrogates, while quoted imported/exported binding names remain validated.
    fn import_attribute_key(&mut self) -> Result<crate::value::JsString, SyntaxError> {
        match self.cur().kind.clone() {
            TokenKind::Str(_) => self.module_string(),
            TokenKind::Ident(name) => { self.bump(); Ok(name.into()) }
            TokenKind::Keyword(keyword) => { self.bump(); Ok(keyword.spelling().into()) }
            _ => Err(self.err("expected import attribute key")),
        }
    }

    fn import_attributes(&mut self) -> Result<Vec<ImportAttribute>, SyntaxError> {
        if !self.at_kw(K::With) {
            return Ok(Vec::new());
        }
        self.bump();
        self.expect_punct(Punct::Lbrace)?;
        let mut attributes = Vec::new();
        let mut seen = HashSet::new();
        while !self.at_punct(Punct::Rbrace) {
            let key = self.import_attribute_key()?;
            if !seen.insert(key.clone()) {
                return Err(self.err("duplicate import attribute"));
            }
            self.expect_punct(Punct::Colon)?;
            let value = self.module_string()?;
            attributes.push(ImportAttribute { key, value });
            if !self.eat_punct(Punct::Comma) {
                break;
            }
        }
        self.expect_punct(Punct::Rbrace)?;
        Ok(attributes)
    }

    fn last_end_span(&self) -> Span {
        Span::new(self.last_end, self.last_end)
    }
}

fn declared_names(statement: &Stmt) -> Vec<String> {
    match &statement.kind {
        StmtKind::Var(decl) => decl
            .decls
            .iter()
            .flat_map(|d| binding_declared_names(&d.binding))
            .collect(),
        StmtKind::Function(function) => function
            .name
            .as_ref()
            .map(|n| vec![n.name.clone()])
            .unwrap_or_default(),
        StmtKind::Class(class) => class
            .name
            .as_ref()
            .map(|n| vec![n.name.clone()])
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

fn binding_declared_names(binding: &Binding) -> Vec<String> {
    match binding {
        Binding::Ident(name) => vec![name.name.clone()],
        Binding::Array(items) => items
            .iter()
            .flatten()
            .flat_map(|item| binding_declared_names(item))
            .collect(),
        Binding::Object(items) => items
            .iter()
            .flat_map(|(_, value)| binding_declared_names(value))
            .collect(),
        Binding::Rest(inner) | Binding::Default(inner, _) => binding_declared_names(inner),
    }
}

fn statement_declares(statement: &Stmt, name: &str) -> bool {
    declared_names(statement)
        .iter()
        .any(|declared| declared == name)
}

fn well_formed_utf16(units: &[u16]) -> bool {
    let mut i = 0;
    while i < units.len() {
        match units[i] {
            0xD800..=0xDBFF => {
                if i + 1 >= units.len() || !(0xDC00..=0xDFFF).contains(&units[i + 1]) {
                    return false;
                }
                i += 2;
            }
            0xDC00..=0xDFFF => return false,
            _ => i += 1,
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_import_export_forms_and_attributes() {
        let source = "import './side.js'; import defaultValue, {name as alias, default as other} from './dep.js' with { type: 'json' }; export {alias as result}; export * as ns from './dep.js'; export const value = 1; export default value;";
        let module = crate::parse_module(source).unwrap();
        assert_eq!(module.items.len(), 6);
        assert!(
            matches!(&module.items[1], ModuleItem::Import(import) if import.bindings.len() == 3 && import.attributes[0].key == "type")
        );
        assert!(
            matches!(&module.items[3], ModuleItem::ExportAll { as_name: Some(name), .. } if name == "ns")
        );
    }

    #[test]
    fn module_early_errors() {
        for source in [
            "export const x = 1; export { x };",
            "export { missing };",
            "import { x } from './x.js'; let x = 1;",
            "import { x, y as x } from './x.js';",
            "import x from './x.js' with { type: 'json', type: 'css' };",
            "with (x) {}",
        ] {
            assert!(crate::parse_module(source).is_err(), "{source}");
        }
    }

    #[test]
    fn module_top_level_await_and_import_meta() {
        let module = crate::parse_module("await work(); import.meta.url;").unwrap();
        assert!(module.has_top_level_await);
        assert!(crate::parse_script("import.meta.url;").is_err());
    }
}
