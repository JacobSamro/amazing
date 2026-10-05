//! Exact immutable source units. Metadata does not affect AST semantic equality.
use std::hash::{Hash, Hasher};
use std::sync::Arc;

#[derive(Clone)]
pub struct SourceText(Arc<[u16]>);
impl SourceText {
    pub fn from_utf16(units: &[u16]) -> Self { Self(Arc::from(units)) }
    pub fn units(&self) -> &[u16] { &self.0 }
    pub fn range(&self, start: usize, end: usize) -> Option<SourceRange> {
        (start <= end && end <= self.0.len()).then(|| SourceRange { text: self.clone(), start, end })
    }
    pub fn same_source(&self, other: &Self) -> bool { Arc::ptr_eq(&self.0, &other.0) }
}
#[derive(Clone)]
pub struct SourceRange { text: SourceText, start: usize, end: usize }
impl SourceRange {
    pub fn units(&self) -> &[u16] { &self.text.units()[self.start..self.end] }
    pub fn start(&self) -> usize { self.start }
    pub fn end(&self) -> usize { self.end }
    pub fn text(&self) -> &SourceText { &self.text }
}
#[derive(Clone, Default)]
pub struct SourceMetadata(Option<SourceRange>);
impl SourceMetadata {
    pub fn new(range: SourceRange) -> Self { Self(Some(range)) }
    pub fn range(&self) -> Option<&SourceRange> { self.0.as_ref() }
    pub fn units(&self) -> Option<&[u16]> { self.range().map(SourceRange::units) }
}
impl PartialEq for SourceMetadata { fn eq(&self, _: &Self) -> bool { true } }
impl Eq for SourceMetadata {}
impl Hash for SourceMetadata { fn hash<H: Hasher>(&self, state: &mut H) { 0_u8.hash(state); } }
impl std::fmt::Debug for SourceMetadata {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("SourceMetadata").field(&self.0.as_ref().map(|r| (r.start, r.end))).finish()
    }
}
