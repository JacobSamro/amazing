//! External DTD inclusion uses physical source frames and explicit literal phase.
use super::{XmlError, XmlErrorKind, dtd};
use crate::xml_resources::*;
use dtd::{Dtd, EntityDefinition, EntityPolicy, ExpansionBudget};
use std::rc::Rc;
type ResultXml<T> = Result<T, XmlError>;
fn error(message: &str) -> XmlError {
    XmlError {
        offset: 0,
        kind: XmlErrorKind::Malformed,
        message: message.into(),
    }
}
fn normalize(text: &str) -> ResultXml<String> {
    if let Some((offset, _)) = text.char_indices().find(|(_, c)| !super::xml_char(*c)) {
        return Err(XmlError {
            offset,
            kind: XmlErrorKind::Malformed,
            message: "forbidden external XML character".into(),
        });
    }
    Ok(text.replace("\r\n", "\n").replace('\r', "\n"))
}
struct Frame {
    text: String,
    pos: usize,
    source: XmlEntitySource,
    external: bool,
    parameter: bool,
    loan: bool,
    resource: bool,
    id: usize,
}
struct Scanner<'a, 'r> {
    frames: Vec<Frame>,
    next_id: usize,
    dtd: &'a mut Dtd,
    budget: &'a mut ExpansionBudget,
    resources: &'a mut XmlResourceSession<'r>,
    policy: EntityPolicy,
    version: &'a str,
    cutoff: &'a mut bool,
}
impl Scanner<'_, '_> {
    fn ensure(&mut self) -> bool {
        while self.frames.last().is_some_and(|f| f.pos == f.text.len()) {
            let f = self.frames.pop().unwrap();
            if f.loan {
                self.budget.leave();
            }
            if f.resource {
                self.resources.leave();
            }
        }
        !self.frames.is_empty()
    }
    fn rest(&mut self) -> &str {
        if !self.ensure() {
            return "";
        }
        let f = self.frames.last().unwrap();
        &f.text[f.pos..]
    }
    fn take(&mut self, text: &str) -> bool {
        if self.rest().starts_with(text) {
            self.frames.last_mut().unwrap().pos += text.len();
            true
        } else {
            false
        }
    }
    fn char(&mut self) -> Option<char> {
        if !self.ensure() {
            return None;
        }
        let f = self.frames.last_mut().unwrap();
        let c = f.text[f.pos..].chars().next()?;
        f.pos += c.len_utf8();
        Some(c)
    }
    fn space(&mut self) {
        while self.rest().chars().next().is_some_and(super::xml_space) {
            self.char();
        }
    }
    fn source(&mut self) -> ResultXml<(XmlEntitySource, bool, bool, usize)> {
        if !self.ensure() {
            return Err(error("unexpected external DTD end"));
        }
        let f = self.frames.last().unwrap();
        Ok((f.source.clone(), f.external, f.parameter, f.id))
    }
    fn reference(&mut self) -> ResultXml<String> {
        let mut name = String::new();
        loop {
            let c = self
                .char()
                .ok_or_else(|| error("unclosed parameter reference"))?;
            if c == ';' {
                break;
            }
            name.push(c);
            if name.len() > 16 * 1024 * 1024 {
                return Err(error("oversized parameter reference"));
            }
        }
        dtd::valid_reference_name(&name, 0)?;
        Ok(name)
    }
    fn parameter(&mut self, name: &str) -> ResultXml<Option<(String, XmlEntitySource, bool)>> {
        self.dtd.has_parameter_references = true;
        let Some(definition) = self.dtd.parameter.get(name).cloned() else {
            if *self.cutoff && !self.policy.standalone {
                return Ok(None);
            }
            return Err(error("undeclared parameter entity"));
        };
        match definition {
            EntityDefinition::Internal(body) | EntityDefinition::ResolvedExternal(body) => {
                let source = self
                    .dtd
                    .sources
                    .get(&(true, name.into()))
                    .cloned()
                    .ok_or_else(|| error("parameter declaration has no source"))?;
                Ok(Some((body.to_string(), source, false)))
            }
            EntityDefinition::External {
                public_id,
                system_id,
                source,
                notation,
            } => {
                if notation.is_some() {
                    return Err(error("unparsed parameter entity"));
                }
                let source = source.ok_or_else(|| error("external declaration has no source"))?;
                let resource = self.resources.read(XmlExternalRequest {
                    declaration_source: &source,
                    system_id: &system_id,
                    public_id: public_id.as_deref(),
                    kind: XmlExternalKind::ParameterEntity,
                })?;
                let Some(resource) = resource else {
                    *self.cutoff = true;
                    return Ok(None);
                };
                let decoded = crate::xml_encoding::decode_external_entity(
                    &resource.bytes,
                    &resource.encoding_authority,
                )
                .map_err(|e| error(&e.to_string()))?;
                check_version(decoded.version.as_deref(), self.version)?;
                Ok(Some((normalize(&decoded.text)?, resource.source, true)))
            }
        }
    }
    fn expand_literal(&mut self, raw: &str, depth: usize) -> ResultXml<String> {
        if depth >= 32 {
            return Err(XmlError {
                offset: 0,
                kind: XmlErrorKind::Limit,
                message: "entity literal depth exceeds 32".into(),
            });
        }
        let mut text = String::new();
        let mut rest = raw;
        while !rest.is_empty() {
            if let Some(tail) = rest.strip_prefix('%') {
                let end = tail
                    .find(';')
                    .ok_or_else(|| error("unclosed literal parameter reference"))?;
                let name = &tail[..end];
                dtd::valid_reference_name(name, 0)?;
                self.budget.enter(true, name, 0)?;
                let result = (|| {
                    let (body, source, external) =
                        self.parameter(name)?.ok_or_else(|| XmlError {
                            offset: 0,
                            kind: XmlErrorKind::Unsupported,
                            message: "unread parameter entity in literal".into(),
                        })?;
                    if external {
                        self.resources.enter(&source)?;
                    }
                    let result = self.expand_literal(&body, depth + 1);
                    if external {
                        self.resources.leave();
                    }
                    result
                })();
                self.budget.leave();
                text.push_str(&result?);
                rest = &tail[end + 1..];
            } else if let Some(tail) = rest.strip_prefix('&') {
                let end = tail
                    .find(';')
                    .ok_or_else(|| error("unclosed entity value reference"))?;
                let name = &tail[..end];
                if name.starts_with('#') {
                    text.push(dtd::character_reference(name, 0)?);
                } else {
                    dtd::valid_reference_name(name, 0)?;
                    text.push_str(&rest[..end + 2]);
                }
                rest = &tail[end + 1..];
            } else {
                let c = rest.chars().next().unwrap();
                text.push(c);
                rest = &rest[c.len_utf8()..];
            }
            if text.len() > 16 * 1024 * 1024 {
                return Err(XmlError {
                    offset: 0,
                    kind: XmlErrorKind::Limit,
                    message: "entity literal exceeds 16 MiB".into(),
                });
            }
        }
        Ok(text)
    }
    fn inline_parameter(&mut self) -> ResultXml<()> {
        let (_, external_context, _, _) = self.source()?;
        if !external_context {
            return Err(error(
                "parameter reference inside internal markup declaration",
            ));
        }
        self.char();
        let name = self.reference()?;
        self.budget.enter(true, &name, 0)?;
        let result = self.parameter(&name);
        let (text, source, external_resource) = match result {
            Ok(Some(value)) => value,
            Ok(None) => {
                self.budget.leave();
                return Err(XmlError {
                    offset: 0,
                    kind: XmlErrorKind::Unsupported,
                    message: "unread parameter in markup declaration".into(),
                });
            }
            Err(e) => {
                self.budget.leave();
                return Err(e);
            }
        };
        if let Err(e) = self.budget.charge_bytes(text.len(), 0) {
            self.budget.leave();
            return Err(e);
        }
        if external_resource {
            if let Err(e) = self.resources.enter(&source) {
                self.budget.leave();
                return Err(e);
            }
        }
        let id = self.next_id;
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or_else(|| error("source frame identity overflow"))?;
        self.frames.push(Frame {
            text: format!(" {text} "),
            pos: 0,
            source,
            external: external_context || external_resource,
            parameter: true,
            loan: true,
            resource: external_resource,
            id,
        });
        Ok(())
    }
    fn declaration(&mut self) -> ResultXml<()> {
        let (source, _, parameter, _) = self.source()?;
        let direct = !parameter && !self.frames.last().unwrap().external;
        let mut text = String::new();
        loop {
            if self.rest().starts_with('%') && !self.rest()[1..].starts_with(super::xml_space) {
                self.inline_parameter()?;
                continue;
            }
            let token_source = self.source()?;
            let c = self
                .char()
                .ok_or_else(|| error("unclosed DTD markup declaration"))?;
            text.push(c);
            if c == '>' {
                break;
            }
            if matches!(c, '\'' | '"') {
                let (_, external, _, frame) = token_source;
                let header = text[..text.len() - 1].trim_end();
                let entity_value = header.strip_prefix("<!ENTITY").is_some_and(|tail| {
                    let mut words = tail.split_whitespace();
                    let name = words.next();
                    if name == Some("%") {
                        words.next();
                    }
                    words.next().is_none()
                });
                let mut raw = String::new();
                loop {
                    let (_, _, parameter, current) = self.source()?;
                    let next = self.char().ok_or_else(|| error("unclosed DTD literal"))?;
                    if next == c && (current == frame || !parameter) {
                        break;
                    }
                    raw.push(next);
                }
                if entity_value {
                    if !external && raw.contains('%') {
                        return Err(error("parameter reference inside internal entity literal"));
                    }
                    let value = if external {
                        self.expand_literal(&raw, 0)?
                    } else {
                        raw.clone()
                    };
                    if external {
                        for c in value.chars() {
                            if matches!(c, '&' | '%' | '\'' | '"') {
                                text.push_str(&format!("&#{};", c as u32));
                            } else {
                                text.push(c);
                            }
                        }
                    } else {
                        text.push_str(&value);
                    }
                } else {
                    text.push_str(&raw);
                }
                text.push(c);
            }
            if text.len() > 16 * 1024 * 1024 {
                return Err(XmlError {
                    offset: 0,
                    kind: XmlErrorKind::Limit,
                    message: "DTD declaration exceeds 16 MiB".into(),
                });
            }
        }
        let apply = !*self.cutoff || self.policy.standalone;
        dtd::apply_declaration(
            &text,
            self.dtd,
            self.budget,
            self.policy,
            &source,
            direct,
            apply,
        )
    }
    fn ignored(&mut self) -> ResultXml<()> {
        let mut depth = 1usize;
        while depth != 0 {
            if self.take("<![") {
                depth += 1;
                if depth > 512 {
                    return Err(XmlError {
                        offset: 0,
                        kind: XmlErrorKind::Limit,
                        message: "conditional section depth exceeds 512".into(),
                    });
                }
            } else if self.take("]]>") {
                depth -= 1;
            } else if self.char().is_none() {
                return Err(error("unclosed ignored conditional section"));
            }
        }
        Ok(())
    }
    fn sequence(
        &mut self,
        internal: bool,
        included: bool,
        depth: usize,
    ) -> ResultXml<Option<usize>> {
        if depth > 512 {
            return Err(XmlError {
                offset: 0,
                kind: XmlErrorKind::Limit,
                message: "conditional section depth exceeds 512".into(),
            });
        }
        loop {
            self.space();
            if internal && self.frames.len() == 1 && self.rest().starts_with(']') {
                let offset = self.frames[0].pos + 1;
                self.char();
                return Ok(Some(offset));
            }
            if included && self.take("]]>") {
                return Ok(None);
            }
            if self.rest().is_empty() {
                return if internal || included {
                    Err(error("unclosed DTD subset or conditional section"))
                } else {
                    Ok(None)
                };
            }
            if self.take("<!--") {
                let mut data = String::new();
                while !self.take("-->") {
                    data.push(self.char().ok_or_else(|| error("unclosed DTD comment"))?);
                }
                if data.contains("--") || data.ends_with('-') {
                    return Err(error("invalid DTD comment"));
                }
            } else if self.take("<?") {
                let mut data = String::new();
                while !self.take("?>") {
                    data.push(self.char().ok_or_else(|| error("unclosed DTD PI"))?);
                }
                let declaration = format!("<?{data}?>");
                let source = self.frames.last().map(|f| f.source.clone());
                if let Some(source) = source {
                    dtd::apply_declaration(
                        &declaration,
                        self.dtd,
                        self.budget,
                        self.policy,
                        &source,
                        false,
                        true,
                    )?;
                } else {
                    return Err(error("DTD PI crossed entity boundary"));
                }
            } else if self.take("<![") {
                let (_, external, _, _) = self.source()?;
                if !external {
                    return Err(error("conditional section in internal subset"));
                }
                let mut keyword = String::new();
                loop {
                    if self.rest().starts_with('%') {
                        self.inline_parameter()?;
                        continue;
                    }
                    let c = self
                        .char()
                        .ok_or_else(|| error("unclosed conditional header"))?;
                    if c == '[' {
                        break;
                    }
                    keyword.push(c);
                }
                match keyword.trim_matches(super::xml_space) {
                    "INCLUDE" => {
                        self.sequence(false, true, depth + 1)?;
                    }
                    "IGNORE" => self.ignored()?,
                    _ => return Err(error("invalid conditional section keyword")),
                }
            } else if self.rest().starts_with('%') {
                let (_, parent_external, _, _) = self.source()?;
                self.char();
                let name = self.reference()?;
                self.budget.enter(true, &name, 0)?;
                let result = (|| {
                    let Some((body, source, external)) = self.parameter(&name)? else {
                        return Ok(());
                    };
                    self.budget.charge_bytes(body.len(), 0)?;
                    if external {
                        self.resources.enter(&source)?;
                    }
                    let result = scan(
                        &body,
                        &source,
                        external || parent_external,
                        true,
                        false,
                        self.dtd,
                        self.budget,
                        self.resources,
                        self.policy,
                        self.version,
                        self.cutoff,
                    );
                    if external {
                        self.resources.leave();
                    }
                    result.map(|_| ())
                })();
                self.budget.leave();
                result?;
            } else if self.rest().starts_with("<!") {
                self.declaration()?;
            } else {
                return Err(error("invalid DTD subset declaration"));
            }
        }
    }
}
impl Drop for Scanner<'_, '_> {
    fn drop(&mut self) {
        while let Some(f) = self.frames.pop() {
            if f.loan {
                self.budget.leave();
            }
            if f.resource {
                self.resources.leave();
            }
        }
    }
}
#[allow(clippy::too_many_arguments)]
pub(super) fn scan(
    text: &str,
    source: &XmlEntitySource,
    external: bool,
    parameter: bool,
    internal: bool,
    dtd: &mut Dtd,
    budget: &mut ExpansionBudget,
    resources: &mut XmlResourceSession<'_>,
    policy: EntityPolicy,
    version: &str,
    cutoff: &mut bool,
) -> ResultXml<Option<usize>> {
    let mut scanner = Scanner {
        frames: vec![Frame {
            text: text.into(),
            pos: 0,
            source: source.clone(),
            external,
            parameter,
            loan: false,
            resource: false,
            id: 0,
        }],
        next_id: 1,
        dtd,
        budget,
        resources,
        policy,
        version,
        cutoff,
    };
    scanner.sequence(internal, false, 0)
}
pub(super) fn decode_resource(
    resource: XmlExternalResource,
    version: &str,
) -> ResultXml<(String, XmlEntitySource)> {
    let decoded =
        crate::xml_encoding::decode_external_entity(&resource.bytes, &resource.encoding_authority)
            .map_err(|e| error(&e.to_string()))?;
    check_version(decoded.version.as_deref(), version)?;
    Ok((normalize(&decoded.text)?, resource.source))
}
fn check_version(child: Option<&str>, parent: &str) -> ResultXml<()> {
    if let Some(child) = child {
        let c = child
            .strip_prefix("1.")
            .unwrap_or("")
            .trim_start_matches('0');
        let p = parent
            .strip_prefix("1.")
            .unwrap_or("")
            .trim_start_matches('0');
        if c.len() > p.len() || (c.len() == p.len() && c > p) {
            return Err(error("external entity version exceeds document version"));
        }
    }
    Ok(())
}
/// Only references in character content are admitted for external loading.
/// Attributes/comments/PIs/CDATA are left to the actual XML parser, not fetched.
pub(super) fn content_references(text: &str, fragment: bool) -> Vec<String> {
    let mut refs = Vec::new();
    let mut rest = text;
    let mut depth = usize::from(fragment);
    while !rest.is_empty() {
        if rest.starts_with("<!--") || rest.starts_with("<![CDATA[") || rest.starts_with("<?") {
            let marker = if rest.starts_with("<!--") {
                "-->"
            } else if rest.starts_with("<![CDATA[") {
                "]]>"
            } else {
                "?>"
            };
            let Some(end) = rest.find(marker) else {
                break;
            };
            rest = &rest[end + marker.len()..];
            continue;
        }
        if rest.starts_with('<') {
            let mut quote = None;
            let mut end = None;
            for (i, c) in rest.char_indices().skip(1) {
                if let Some(q) = quote {
                    if c == q {
                        quote = None;
                    }
                } else if matches!(c, '\'' | '"') {
                    quote = Some(c);
                } else if c == '>' {
                    end = Some(i);
                    break;
                }
            }
            let Some(end) = end else {
                break;
            };
            let tag = &rest[..end + 1];
            if tag.starts_with("</") {
                depth = depth.saturating_sub(1);
            } else if !tag.starts_with("<!") && !tag.ends_with("/>") {
                depth += 1;
            }
            rest = &rest[end + 1..];
            continue;
        }
        if depth != 0 && rest.starts_with('&') {
            if let Some(end) = rest.find(';') {
                let name = &rest[1..end];
                if !name.starts_with('#') {
                    refs.push(name.into());
                }
                rest = &rest[end + 1..];
                continue;
            }
        }
        let c = rest.chars().next().unwrap();
        rest = &rest[c.len_utf8()..];
    }
    refs
}
pub(super) fn resolve_general(
    dtd: &mut Dtd,
    name: &str,
    resources: &mut XmlResourceSession<'_>,
    version: &str,
    active: &mut std::collections::BTreeSet<String>,
    done: &mut std::collections::BTreeSet<String>,
) -> ResultXml<()> {
    if dtd::predefined(name).is_some() || done.contains(name) {
        return Ok(());
    }
    dtd::valid_reference_name(name, 0)?;
    let Some(declaration) = dtd.general.get(name).cloned() else {
        return Ok(());
    };
    if !active.insert(name.into()) {
        return Err(error("recursive general entity"));
    }
    if active.len() > 32 {
        active.remove(name);
        return Err(XmlError {
            offset: 0,
            kind: XmlErrorKind::Limit,
            message: "general entity dependency depth exceeds 32".into(),
        });
    }
    let result = (|| {
        let (body, source) = match declaration.definition {
            EntityDefinition::Internal(body) | EntityDefinition::ResolvedExternal(body) => {
                (body.to_string(), None)
            }
            EntityDefinition::External {
                public_id,
                system_id,
                source,
                notation,
            } => {
                if notation.is_some() {
                    return Err(error("unparsed entity in content"));
                }
                let source =
                    source.ok_or_else(|| error("external entity has no declaration source"))?;
                let Some(resource) = resources.read(XmlExternalRequest {
                    declaration_source: &source,
                    system_id: &system_id,
                    public_id: public_id.as_deref(),
                    kind: XmlExternalKind::GeneralEntity,
                })?
                else {
                    return Ok(());
                };
                let (body, source) = decode_resource(resource, version)?;
                resources.enter(&source)?;
                dtd.general.get_mut(name).unwrap().definition =
                    EntityDefinition::ResolvedExternal(Rc::from(body.clone()));
                (body, Some(source))
            }
        };
        let result = (|| {
            for next in content_references(&body, true) {
                resolve_general(dtd, &next, resources, version, active, done)?;
            }
            Ok(())
        })();
        if source.is_some() {
            resources.leave();
        }
        result
    })();
    active.remove(name);
    if result.is_ok() {
        done.insert(name.into());
    }
    result
}
