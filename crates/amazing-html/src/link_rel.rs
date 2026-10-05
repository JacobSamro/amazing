//! Link relationship classification and native fetch-trigger rules.
//! Recognition is NOT an assertion that an embedding implements a processing model.
//! https://html.spec.whatwg.org/multipage/links.html#linkTypes
use crate::sandbox::ascii_equals;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LinkRelation { Alternate, DnsPrefetch, Icon, Manifest, ModulePreload, Preconnect, Prefetch, Preload, Stylesheet }
impl LinkRelation {
    pub const ADVERTISED_EXISTING: [Self; 9] = [Self::Alternate, Self::DnsPrefetch, Self::Icon,
        Self::Manifest, Self::ModulePreload, Self::Preconnect, Self::Prefetch, Self::Preload, Self::Stylesheet];
    pub fn keyword(self) -> &'static str {
        match self { Self::Alternate => "alternate", Self::DnsPrefetch => "dns-prefetch", Self::Icon => "icon",
            Self::Manifest => "manifest", Self::ModulePreload => "modulepreload", Self::Preconnect => "preconnect",
            Self::Prefetch => "prefetch", Self::Preload => "preload", Self::Stylesheet => "stylesheet" }
    }
    pub fn is_body_ok(self) -> bool {
        matches!(self, Self::DnsPrefetch | Self::ModulePreload | Self::Preconnect | Self::Prefetch | Self::Preload | Self::Stylesheet)
    }
    pub fn may_delay_document_load(self) -> bool { self == Self::Stylesheet }
}
/// Known relationships in first occurrence order. Non-ASCII units never alias a keyword.
pub fn parse_link_rel(input: &[u16]) -> Vec<LinkRelation> {
    let mut result = vec![];
    for token in input.split(|u| matches!(*u, 9 | 10 | 12 | 13 | 32)) {
        if let Some(relation) = LinkRelation::ADVERTISED_EXISTING.into_iter().find(|r| ascii_equals(token, r.keyword())) {
            if !result.contains(&relation) { result.push(relation); }
        }
    }
    result
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LinkAttribute { Href, As, Type, Media, CrossOrigin, ReferrerPolicy, Other }
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PriorFetch { NotAttempted, Obtained, UnsupportedType, MediaMismatch }
/// Attribute trigger rules for the hint models. Relationship creation and real
/// browsing-context connection are separate native transitions, not attribute edits.
pub fn hint_attribute_triggers(relation: LinkRelation, attribute: LinkAttribute, prior: PriorFetch) -> Option<bool> {
    use LinkAttribute::*;
    match relation {
        LinkRelation::DnsPrefetch | LinkRelation::ModulePreload | LinkRelation::Manifest => Some(attribute == Href),
        LinkRelation::Preconnect | LinkRelation::Prefetch => Some(matches!(attribute, Href | CrossOrigin)),
        LinkRelation::Preload => Some(matches!(attribute, Href | As)
            || attribute == Type && prior == PriorFetch::UnsupportedType
            || attribute == Media && prior == PriorFetch::MediaMismatch),
        // Their full own algorithms include selection/style state and cannot be
        // reduced to hint rules. Embeddings must invoke those concrete models.
        LinkRelation::Alternate | LinkRelation::Icon | LinkRelation::Stylesheet => None,
    }
}
