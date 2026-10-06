//! One explicit-resource parse prepares the real declaration environment.
use super::external_dtd;
use super::*;
use crate::xml_resources::*;
pub(super) struct Prepared {
    pub dtd: Rc<Dtd>,
    pub internal_end: Option<usize>,
    pub budget: Rc<RefCell<ExpansionBudget>>,
}
pub(super) fn prepare(
    src: &str,
    source: &XmlEntitySource,
    resolver: &mut dyn XmlExternalResolver,
) -> ResultXml<Option<Prepared>> {
    if let Some((offset, _)) = src.char_indices().find(|(_, c)| !xml_char(*c)) {
        return Err(XmlError {
            offset,
            kind: XmlErrorKind::Malformed,
            message: "forbidden XML character".into(),
        });
    }
    let mut p = Parser {
        src,
        pos: 0,
        external_subset: false,
        canonical_xhtml_subset: false,
        standalone: false,
        dtd: Rc::default(),
        budget: Rc::default(),
        inherited_depth: 0,
        replacement: false,
        prepared_internal_end: None,
        document_version: "1.0".into(),
    };
    let declaration = if p.rest().starts_with("<?xml") && p.rest()[5..].starts_with(xml_space) {
        Some(p.declaration()?)
    } else {
        None
    };
    p.standalone = declaration
        .as_ref()
        .is_some_and(|d| d.standalone == Some(true));
    let version = p.document_version.clone();
    loop {
        p.space();
        if p.take("<!--") {
            let text = p.until("-->")?;
            if text.contains("--") || text.ends_with('-') {
                return Err(p.error("invalid XML comment"));
            }
        } else if p.take("<?") {
            let target = p.name()?;
            if target.eq_ignore_ascii_case("xml") {
                return Err(p.error("reserved processing instruction target"));
            }
            if !p.take("?>") {
                p.require_space()?;
                p.until("?>")?;
            }
        } else {
            break;
        }
    }
    if !p.take("<!DOCTYPE") {
        return Ok(None);
    }
    p.require_space()?;
    p.qname()?;
    let separated = p.space();
    let mut public = None;
    let mut system = None;
    if separated && p.take("SYSTEM") {
        p.require_space()?;
        system = Some(p.literal()?.to_owned());
        p.space();
        p.external_subset = true;
    } else if separated && p.take("PUBLIC") {
        p.require_space()?;
        let value = p.literal()?;
        if !value.chars().all(pubid_char) {
            return Err(p.error("invalid public identifier"));
        }
        p.canonical_xhtml_subset = canonical_xhtml_public_id(value);
        public = Some(value.to_owned());
        p.require_space()?;
        system = Some(p.literal()?.to_owned());
        p.space();
        p.external_subset = true;
    }
    let policy = p.policy();
    let mut dtd = Dtd::default();
    let mut budget = ExpansionBudget::default();
    let mut resources = XmlResourceSession::new(resolver);
    let mut cutoff = false;
    let internal_end = if p.take("[") {
        let consumed = external_dtd::scan(
            p.rest(),
            source,
            false,
            false,
            true,
            &mut dtd,
            &mut budget,
            &mut resources,
            policy,
            &version,
            &mut cutoff,
        )?
        .ok_or_else(|| p.error("unclosed internal subset"))?;
        p.pos += consumed;
        p.space();
        Some(p.pos)
    } else {
        None
    };
    p.expect(">")?;
    if let Some(system) = system {
        if system.contains('#') {
            return Err(p.error("fragment identifier in XML system identifier"));
        }
        if let Some(resource) = resources.read(XmlExternalRequest {
            declaration_source: source,
            system_id: &system,
            public_id: public.as_deref(),
            kind: XmlExternalKind::Subset,
        })? {
            let (body, actual) = external_dtd::decode_resource(resource, &version)?;
            resources.enter(&actual)?;
            let result = external_dtd::scan(
                &body,
                &actual,
                true,
                false,
                false,
                &mut dtd,
                &mut budget,
                &mut resources,
                policy,
                &version,
                &mut cutoff,
            );
            resources.leave();
            result?;
        }
    }
    let mut done = BTreeSet::new();
    for name in external_dtd::content_references(p.rest(), false) {
        external_dtd::resolve_general(
            &mut dtd,
            &name,
            &mut resources,
            &version,
            &mut BTreeSet::new(),
            &mut done,
        )?;
    }
    dtd::validate(&dtd, policy, 0)?;
    Ok(Some(Prepared {
        dtd: Rc::new(dtd),
        internal_end,
        budget: Rc::new(RefCell::new(budget)),
    }))
}
