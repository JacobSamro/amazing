//! Nonvalidating XML 1.0 internal declaration grammar and entity value phases.
//! External resources remain explicit unsupported dependencies.
use super::{XmlError, XmlErrorKind};
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

#[derive(Clone, Debug)]
pub(super) enum EntityDefinition {
    Internal(Rc<str>),
    External { public_id: Option<String>, system_id: String, notation: Option<String> },
}
#[derive(Clone, Debug)]
pub(super) struct DeclaredEntity {
    pub definition: EntityDefinition,
    // Standalone WFC Entity Declared distinguishes direct declarations from
    // ones made inside parameter entities. A later duplicate direct declaration
    // still counts for that WFC, without replacing the first binding's value.
    pub declared_directly: bool,
}
#[derive(Clone, Debug)]
pub(super) struct AttributeDeclaration {
    pub name: String,
    pub tokenized: bool,
    pub default: Option<String>,
}
#[derive(Debug, Default)]
pub(super) struct Dtd {
    pub general: BTreeMap<String, DeclaredEntity>,
    pub parameter: BTreeMap<String, EntityDefinition>,
    pub attributes: BTreeMap<String, Vec<AttributeDeclaration>>,
    pub has_parameter_references: bool,
    declaration_references: Vec<(String, bool)>,
}

// Limits describe resource boundaries, not fictitious wellformedness failures.
// Recursion uses actual active entity identities; acyclic depth is a Limit.
#[derive(Debug, Default)]
pub(super) struct ExpansionBudget {
    active: BTreeSet<(bool, String)>,
    stack: Vec<(bool, String)>,
    references: usize,
    bytes: usize,
    declarations: usize,
}
impl ExpansionBudget {
    fn error(offset: usize, kind: XmlErrorKind, message: &str) -> XmlError {
        XmlError { offset, kind, message: message.into() }
    }
    pub fn enter(&mut self, parameter: bool, name: &str, offset: usize) -> Result<(), XmlError> {
        let identity = (parameter, name.to_owned());
        if self.active.contains(&identity) {
            return Err(Self::error(offset, XmlErrorKind::Malformed, "recursive entity reference"));
        }
        if self.stack.len() >= 32 {
            return Err(Self::error(offset, XmlErrorKind::Limit, "entity nesting exceeds 32"));
        }
        self.references = self.references.checked_add(1)
            .ok_or_else(|| Self::error(offset, XmlErrorKind::Limit, "entity reference budget overflow"))?;
        if self.references > 1_000_000 {
            return Err(Self::error(offset, XmlErrorKind::Limit, "entity reference budget exceeded"));
        }
        self.active.insert(identity.clone());
        self.stack.push(identity);
        Ok(())
    }
    pub fn leave(&mut self) {
        if let Some(identity) = self.stack.pop() { self.active.remove(&identity); }
    }
    pub fn charge_bytes(&mut self, bytes: usize, offset: usize) -> Result<(), XmlError> {
        self.bytes = self.bytes.checked_add(bytes)
            .ok_or_else(|| Self::error(offset, XmlErrorKind::Limit, "entity expansion budget overflow"))?;
        if self.bytes > 16 * 1024 * 1024 {
            return Err(Self::error(offset, XmlErrorKind::Limit, "entity expansion exceeds 16 MiB"));
        }
        Ok(())
    }
    pub fn declaration(&mut self, offset: usize) -> Result<(), XmlError> {
        self.declarations += 1;
        if self.declarations > 16_384 {
            return Err(Self::error(offset, XmlErrorKind::Limit, "DTD declaration budget exceeded"));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Default)]
pub(super) struct EntityPolicy {
    pub external_subset: bool,
    pub canonical: bool,
    pub standalone: bool,
}
type ResultXml<T> = Result<T, XmlError>;
struct Reader<'a> { src: &'a str, pos: usize, base: usize, parameter: bool }
impl<'a> Reader<'a> {
    fn rest(&self) -> &'a str { &self.src[self.pos..] }
    fn error(&self, message: &str) -> XmlError {
        XmlError { offset: self.base + self.pos, kind: XmlErrorKind::Malformed, message: message.into() }
    }
    fn take(&mut self, text: &str) -> bool {
        if self.rest().starts_with(text) { self.pos += text.len(); true } else { false }
    }
    fn expect(&mut self, text: &str) -> ResultXml<()> {
        if self.take(text) { Ok(()) } else { Err(self.error(&format!("expected {text} in DTD"))) }
    }
    fn space(&mut self) -> bool {
        let before = self.pos;
        while let Some(c) = self.rest().chars().next().filter(|c| super::xml_space(*c)) { self.pos += c.len_utf8(); }
        before != self.pos
    }
    fn separated(&mut self) -> ResultXml<()> {
        if self.space() { Ok(()) } else { Err(self.error("DTD declaration requires whitespace")) }
    }
    fn name(&mut self, token: bool, qualified: bool) -> ResultXml<String> {
        let begin = self.pos;
        let c = self.rest().chars().next().ok_or_else(|| self.error("missing DTD name"))?;
        if !(if token { super::name_char(c) } else { super::name_start(c) }) {
            return Err(self.error("invalid DTD name"));
        }
        self.pos += c.len_utf8();
        while let Some(c) = self.rest().chars().next().filter(|c| super::name_char(*c)) { self.pos += c.len_utf8(); }
        let name = &self.src[begin..self.pos];
        if !token {
            if qualified {
                let parts: Vec<_> = name.split(':').collect();
                if parts.len() > 2 || parts.iter().any(|part| !part.chars().next().is_some_and(|c| c != ':' && super::name_start(c))) {
                    return Err(self.error("invalid DTD qualified name"));
                }
            } else if name.contains(':') { return Err(self.error("colon in entity or notation name")); }
        }
        Ok(name.into())
    }
    fn literal(&mut self) -> ResultXml<&'a str> {
        let quote = self.rest().chars().next().filter(|c| matches!(c, '\'' | '"'))
            .ok_or_else(|| self.error("missing DTD literal"))?;
        self.pos += 1;
        let end = self.rest().find(quote).ok_or_else(|| self.error("unclosed DTD literal"))?;
        let value = &self.rest()[..end];
        self.pos += end + 1;
        Ok(value)
    }
    fn external_id(&mut self, notation: bool) -> ResultXml<(Option<String>, String)> {
        if self.take("SYSTEM") {
            self.separated()?;
            Ok((None, self.literal()?.into()))
        } else {
            self.expect("PUBLIC")?;
            self.separated()?;
            let public = self.literal()?;
            if !public.chars().all(super::pubid_char) { return Err(self.error("invalid DTD public identifier")); }
            let spaced = self.space();
            let system = if notation && !self.rest().starts_with(['\'', '"']) { String::new() }
                else { if !spaced { return Err(self.error("missing public identifier system literal")); } self.literal()?.into() };
            Ok((Some(public.into()), system))
        }
    }
    fn sequence(&mut self, dtd: &mut Dtd, budget: &mut ExpansionBudget, policy: EntityPolicy, main: bool) -> ResultXml<()> {
        loop {
            self.space();
            if main && self.take("]") { return Ok(()); }
            if self.rest().is_empty() {
                return if main { Err(self.error("unclosed internal subset")) } else { Ok(()) };
            }
            if self.take("<!--") {
                let end = self.rest().find("-->").ok_or_else(|| self.error("unclosed DTD comment"))?;
                let data = &self.rest()[..end];
                if data.contains("--") || data.ends_with('-') { return Err(self.error("invalid DTD comment")); }
                self.pos += end + 3;
            } else if self.take("<?") {
                let target = self.name(false, false)?;
                if target.eq_ignore_ascii_case("xml") { return Err(self.error("reserved DTD PI target")); }
                if !self.take("?>") {
                    self.separated()?;
                    let end = self.rest().find("?>").ok_or_else(|| self.error("unclosed DTD PI"))?;
                    self.pos += end + 2;
                }
            } else if self.take("%") {
                dtd.has_parameter_references = true;
                let name = self.name(false, false)?;
                self.expect(";")?;
                let body = parameter_body(dtd, &name, self.base + self.pos, policy.external_subset)?;
                budget.enter(true, &name, self.base + self.pos)?;
                let result = (|| {
                    budget.charge_bytes(body.len(), self.base + self.pos)?;
                    Reader { src: &body, pos: 0, base: self.base + self.pos, parameter: true }
                        .sequence(dtd, budget, policy, false)
                })();
                budget.leave();
                result?;
            } else if self.take("<!ENTITY") {
                budget.declaration(self.base + self.pos)?;
                self.separated()?;
                let parameter = self.take("%");
                if parameter { self.separated()?; }
                let name = self.name(false, false)?;
                self.separated()?;
                let definition = if self.rest().starts_with(['\'', '"']) {
                    let raw = self.literal()?;
                    // WFC PEs in Internal Subset applies to entity literals too.
                    // Numeric percent references remain legal and are decoded
                    // only after this lexical constraint has been checked.
                    if raw.contains('%') { return Err(self.error("parameter reference inside internal markup declaration")); }
                    let body = entity_value(dtd, budget, raw, self.base + self.pos)?;
                    // Bypassed references become markup only when the
                    // replacement is included. CDATA/comment/PI contents
                    // remain literal then, including undeclared names.
                    for referenced in replacement_references(&body) {
                        if !referenced.starts_with('#') { dtd.declaration_references.push((referenced.into(), !self.parameter)); }
                    }
                    if !parameter { validate_predefined(&name, &body, self.base + self.pos)?; }
                    EntityDefinition::Internal(body.into())
                } else {
                    let (public_id, system_id) = self.external_id(false)?;
                    let spaced = self.space();
                    let notation = if !parameter && spaced && self.take("NDATA") {
                        self.separated()?; Some(self.name(false, false)?)
                    } else { None };
                    EntityDefinition::External { public_id, system_id, notation }
                };
                self.space(); self.expect(">")?;
                if parameter { dtd.parameter.entry(name).or_insert(definition); }
                else {
                    let declaration = dtd.general.entry(name).or_insert(DeclaredEntity { definition, declared_directly: false });
                    declaration.declared_directly |= !self.parameter;
                }
            } else if self.take("<!ELEMENT") {
                budget.declaration(self.base + self.pos)?;
                self.separated()?; self.name(false, true)?; self.separated()?;
                if !self.take("EMPTY") && !self.take("ANY") { self.model(0, true)?; }
                self.space(); self.expect(">")?;
            } else if self.take("<!ATTLIST") {
                budget.declaration(self.base + self.pos)?;
                self.separated()?; let element = self.name(false, true)?;
                loop {
                    let spaced = self.space();
                    if self.take(">") { break; }
                    if !spaced { return Err(self.error("missing attribute declaration whitespace")); }
                    budget.declaration(self.base + self.pos)?;
                    let name = self.name(false, true)?;
                    self.separated()?;
                    let tokenized = if self.take("CDATA") { false }
                    else if self.take("NOTATION") { self.separated()?; self.enumeration(false)?; true }
                    else if self.rest().starts_with('(') { self.enumeration(true)?; true }
                    else {
                        let kind = self.name(false, false)?;
                        if !matches!(kind.as_str(), "ID" | "IDREF" | "IDREFS" | "ENTITY" | "ENTITIES" | "NMTOKEN" | "NMTOKENS") {
                            return Err(self.error("unknown attribute declaration type"));
                        }
                        true
                    };
                    self.separated()?;
                    let default = if self.take("#REQUIRED") || self.take("#IMPLIED") { None }
                    else {
                        if self.take("#FIXED") { self.separated()?; }
                        let raw = self.literal()?;
                        let value = attribute_value(dtd, budget, policy, raw, self.base + self.pos, !self.parameter)?;
                        Some(if tokenized { collapse_spaces(&value) } else { value })
                    };
                    let attributes = dtd.attributes.entry(element.clone()).or_default();
                    if !attributes.iter().any(|attr| attr.name == name) { attributes.push(AttributeDeclaration { name, tokenized, default }); }
                }
            } else if self.take("<!NOTATION") {
                budget.declaration(self.base + self.pos)?;
                self.separated()?; self.name(false, false)?; self.separated()?;
                self.external_id(true)?; self.space(); self.expect(">")?;
            } else { return Err(self.error("invalid internal DTD declaration")); }
        }
    }
    fn enumeration(&mut self, token: bool) -> ResultXml<()> {
        self.expect("(")?; self.space(); self.name(token, false)?;
        loop {
            self.space();
            if self.take(")") { return Ok(()); }
            self.expect("|")?; self.space(); self.name(token, false)?;
        }
    }
    fn model(&mut self, depth: usize, mixed: bool) -> ResultXml<()> {
        if depth >= 512 { return Err(ExpansionBudget::error(self.base + self.pos, XmlErrorKind::Limit, "content model depth exceeds 512")); }
        self.expect("(")?; self.space();
        if mixed && self.take("#PCDATA") {
            let mut names = false;
            loop {
                self.space();
                if self.take(")") { break; }
                self.expect("|")?; self.space(); self.name(false, true)?; names = true;
            }
            if names { self.expect("*")?; } else { self.take("*"); }
            return Ok(());
        }
        self.particle(depth + 1)?;
        let mut separator = None;
        loop {
            self.space();
            if self.take(")") { break; }
            let current = if self.take(",") { ',' } else if self.take("|") { '|' }
                else { return Err(self.error("invalid content model separator")); };
            if separator.is_some_and(|previous| previous != current) { return Err(self.error("mixed content model separators")); }
            separator = Some(current);
            self.space(); self.particle(depth + 1)?;
        }
        self.occurrence();
        Ok(())
    }
    fn particle(&mut self, depth: usize) -> ResultXml<()> {
        if self.rest().starts_with('(') { self.model(depth, false) }
        else { self.name(false, true)?; self.occurrence(); Ok(()) }
    }
    fn occurrence(&mut self) { if !self.take("?") && !self.take("*") { self.take("+"); } }
}

pub(super) fn read_subset(src: &str, base: usize, policy: EntityPolicy, budget: &mut ExpansionBudget) -> ResultXml<(Dtd, usize)> {
    let mut dtd = Dtd::default();
    let mut reader = Reader { src, pos: 0, base, parameter: false };
    reader.sequence(&mut dtd, budget, policy, true)?;
    for (name, direct) in &dtd.declaration_references {
        valid_reference_name(name, base)?;
        if predefined(name).is_some() { continue; }
        if let Some(entity) = dtd.general.get(name) {
            if policy.standalone && *direct && !entity.declared_directly {
                return Err(ExpansionBudget::error(base, XmlErrorKind::Malformed, "standalone entity value requires a direct declaration"));
            }
        } else if !(policy.canonical && !policy.standalone && crate::entities::named_lookup(&format!("{name};")).is_some()) {
            return Err(ExpansionBudget::error(base, if !policy.standalone && (policy.external_subset || dtd.has_parameter_references) { XmlErrorKind::Unsupported } else { XmlErrorKind::Malformed }, "undeclared entity in declaration"));
        }
    }
    let mut visited = BTreeSet::new();
    let mut active = BTreeSet::new();
    for name in dtd.general.keys() { check_entity_graph(&dtd, name, &mut visited, &mut active, base)?; }
    Ok((dtd, reader.pos))
}
fn check_entity_graph(dtd: &Dtd, name: &str, visited: &mut BTreeSet<String>, active: &mut BTreeSet<String>, offset: usize) -> ResultXml<()> {
    if visited.contains(name) { return Ok(()); }
    if active.contains(name) { return Err(ExpansionBudget::error(offset, XmlErrorKind::Malformed, "recursive entity declaration")); }
    if active.len() >= 32 { return Err(ExpansionBudget::error(offset, XmlErrorKind::Limit, "entity dependency depth exceeds 32")); }
    active.insert(name.into());
    if let Some(DeclaredEntity { definition: EntityDefinition::Internal(body), .. }) = dtd.general.get(name) {
        for next in replacement_references(body) {
            if dtd.general.contains_key(next) && predefined(next).is_none() {
                check_entity_graph(dtd, next, visited, active, offset)?;
            }
        }
    }
    active.remove(name); visited.insert(name.into());
    Ok(())
}
fn replacement_references(mut rest: &str) -> Vec<&str> {
    let mut references = Vec::new();
    while !rest.is_empty() {
        let literal_end = if rest.starts_with("<![CDATA[") { rest.find("]]>").map(|end| end + 3) }
            else if rest.starts_with("<!--") { rest.find("-->").map(|end| end + 3) }
            else if rest.starts_with("<?") { rest.find("?>").map(|end| end + 2) } else { None };
        if let Some(end) = literal_end { rest = &rest[end..]; continue; }
        if let Some(tail) = rest.strip_prefix('&') {
            if let Some(end) = tail.find(';') {
                references.push(&tail[..end]); rest = &tail[end + 1..]; continue;
            }
        }
        let c = rest.chars().next().unwrap(); rest = &rest[c.len_utf8()..];
    }
    references
}
fn parameter_body(dtd: &Dtd, name: &str, offset: usize, external_subset: bool) -> ResultXml<Rc<str>> {
    match dtd.parameter.get(name) {
        Some(EntityDefinition::Internal(body)) => Ok(body.clone()),
        Some(EntityDefinition::External { .. }) => Err(ExpansionBudget::error(offset, XmlErrorKind::Unsupported, "external parameter entity resources are not loaded")),
        None => Err(ExpansionBudget::error(offset, if external_subset { XmlErrorKind::Unsupported } else { XmlErrorKind::Malformed }, "undeclared parameter entity")),
    }
}
fn entity_value(_dtd: &Dtd, budget: &mut ExpansionBudget, raw: &str, offset: usize) -> ResultXml<String> {
    let mut result = String::new();
    let mut rest = raw;
    while !rest.is_empty() {
        if let Some(tail) = rest.strip_prefix('&') {
            let end = tail.find(';').ok_or_else(|| ExpansionBudget::error(offset, XmlErrorKind::Malformed, "unterminated entity value reference"))?;
            let name = &tail[..end];
            if name.starts_with('#') { result.push(character_reference(name, offset)?); }
            else { valid_reference_name(name, offset)?; result.push_str(&rest[..end + 2]); }
            rest = &tail[end + 1..];
        } else {
            let c = rest.chars().next().unwrap(); result.push(c); rest = &rest[c.len_utf8()..];
        }
        if result.len() > 16 * 1024 * 1024 { return Err(ExpansionBudget::error(offset, XmlErrorKind::Limit, "entity declaration exceeds 16 MiB")); }
    }
    budget.charge_bytes(result.len(), offset)?;
    Ok(result)
}
pub(super) fn valid_reference_name(name: &str, offset: usize) -> ResultXml<()> {
    if name.contains(':') || !name.chars().next().is_some_and(super::name_start) || !name.chars().all(super::name_char) {
        Err(ExpansionBudget::error(offset, XmlErrorKind::Malformed, "invalid entity reference name"))
    } else { Ok(()) }
}
pub(super) fn character_reference(name: &str, offset: usize) -> ResultXml<char> {
    let (digits, radix) = if let Some(digits) = name.strip_prefix("#x") { (digits, 16) }
        else if let Some(digits) = name.strip_prefix('#') { (digits, 10) }
        else { return Err(ExpansionBudget::error(offset, XmlErrorKind::Malformed, "invalid character reference")); };
    if digits.is_empty() || !digits.bytes().all(|c| if radix == 16 { c.is_ascii_hexdigit() } else { c.is_ascii_digit() }) {
        return Err(ExpansionBudget::error(offset, XmlErrorKind::Malformed, "invalid character reference"));
    }
    u32::from_str_radix(digits, radix).ok().and_then(char::from_u32).filter(|c| super::xml_char(*c))
        .ok_or_else(|| ExpansionBudget::error(offset, XmlErrorKind::Malformed, "forbidden character reference"))
}
fn validate_predefined(name: &str, body: &str, offset: usize) -> ResultXml<()> {
    let expected = match name { "lt" => '<', "amp" => '&', "gt" => '>', "apos" => '\'', "quot" => '"', _ => return Ok(()) };
    let value = if body.starts_with("&#") && body.ends_with(';') { character_reference(&body[1..body.len()-1], offset)? }
        else if body.chars().count() == 1 && !matches!(name, "lt" | "amp") { body.chars().next().unwrap() }
        else { return Err(ExpansionBudget::error(offset, XmlErrorKind::Malformed, "invalid predefined entity declaration")); };
    if value == expected { Ok(()) } else { Err(ExpansionBudget::error(offset, XmlErrorKind::Malformed, "invalid predefined entity replacement")) }
}
pub(super) fn collapse_spaces(value: &str) -> String {
    value.split(' ').filter(|part| !part.is_empty()).collect::<Vec<_>>().join(" ")
}
pub(super) fn attribute_value(dtd: &Dtd, budget: &mut ExpansionBudget, policy: EntityPolicy, raw: &str, offset: usize, direct: bool) -> ResultXml<String> {
    let mut result = String::new(); let mut rest = raw;
    while !rest.is_empty() {
        if let Some(tail) = rest.strip_prefix('&') {
            let end = tail.find(';').ok_or_else(|| ExpansionBudget::error(offset, XmlErrorKind::Malformed, "unterminated attribute reference"))?;
            let name = &tail[..end];
            if name.starts_with('#') { result.push(character_reference(name, offset)?); }
            else if let Some(c) = predefined(name) { result.push(c); }
            else if !dtd.general.contains_key(name) && policy.canonical && !policy.standalone {
                valid_reference_name(name, offset)?;
                let value = crate::entities::named_lookup(&tail[..=end]).ok_or_else(|| ExpansionBudget::error(offset, XmlErrorKind::Malformed, "undeclared entity in canonical XHTML subset"))?;
                for c in value.chars() { result.push(if super::xml_space(c) { ' ' } else { c }); }
            } else {
                if dtd.general.get(name).is_some_and(|entity| matches!(entity.definition, EntityDefinition::External { .. })) {
                    return Err(ExpansionBudget::error(offset, XmlErrorKind::Malformed, "external entity in attribute value"));
                }
                let body = general_body(dtd, policy, name, offset, direct)?;
                budget.enter(false, name, offset)?;
                let expansion = attribute_value(dtd, budget, policy, &body, offset, false);
                budget.leave(); result.push_str(&expansion?);
            }
            rest = &tail[end + 1..];
        } else {
            let c = rest.chars().next().unwrap();
            if c == '<' { return Err(ExpansionBudget::error(offset, XmlErrorKind::Malformed, "literal < in attribute replacement")); }
            result.push(if super::xml_space(c) { ' ' } else { c }); rest = &rest[c.len_utf8()..];
        }
        if result.len() > 16 * 1024 * 1024 { return Err(ExpansionBudget::error(offset, XmlErrorKind::Limit, "attribute expansion exceeds 16 MiB")); }
    }
    budget.charge_bytes(result.len(), offset)?;
    Ok(result)
}
pub(super) fn predefined(name: &str) -> Option<char> {
    match name { "lt" => Some('<'), "gt" => Some('>'), "amp" => Some('&'), "apos" => Some('\''), "quot" => Some('"'), _ => None }
}
pub(super) fn general_body(dtd: &Dtd, policy: EntityPolicy, name: &str, offset: usize, direct: bool) -> ResultXml<Rc<str>> {
    valid_reference_name(name, offset)?;
    if let Some(declaration) = dtd.general.get(name) {
        if policy.standalone && direct && !declaration.declared_directly {
            return Err(ExpansionBudget::error(offset, XmlErrorKind::Malformed, "standalone entity requires a direct declaration"));
        }
        return match &declaration.definition {
            EntityDefinition::Internal(body) => Ok(body.clone()),
            EntityDefinition::External { notation: Some(_), .. } => Err(ExpansionBudget::error(offset, XmlErrorKind::Malformed, "unparsed entity reference")),
            EntityDefinition::External { .. } => Err(ExpansionBudget::error(offset, XmlErrorKind::Unsupported, "external entity resources are not loaded")),
        };
    }
    Err(ExpansionBudget::error(offset, if !policy.standalone && (policy.external_subset || dtd.has_parameter_references) { XmlErrorKind::Unsupported } else { XmlErrorKind::Malformed }, "undeclared entity"))
}
