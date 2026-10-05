//! HTML sandbox restriction parsing; no browsing-context authority is inferred.
//! https://html.spec.whatwg.org/multipage/browsers.html#sandboxing

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum SandboxRestriction {
    Navigation, AuxiliaryNavigation, TopNavigationWithoutActivation,
    TopNavigationWithActivation, Origin, Forms, PointerLock, Scripts,
    AutomaticFeatures, DocumentDomain, PropagateToAuxiliary, Modals,
    OrientationLock, Presentation, Downloads, CustomProtocolsNavigation,
}

/// Restriction bits, not the legacy driver's three permission bits or wire schema.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SandboxingFlags(u16);
impl SandboxingFlags {
    pub const EMPTY: Self = Self(0);
    pub fn contains(self, flag: SandboxRestriction) -> bool { self.0 & (1 << flag as u8) != 0 }
    pub fn union(self, other: Self) -> Self { Self(self.0 | other.0) }
    pub fn is_empty(self) -> bool { self == Self::EMPTY }
    /// None is an absent iframe attribute; Some(empty) is a full sandbox.
    pub fn from_attribute(input: Option<&[u16]>) -> Self {
        input.map_or(Self::EMPTY, Self::parse_directive)
    }
    /// The complete HTML parsing algorithm, operating on original UTF-16 units.
    pub fn parse_directive(input: &[u16]) -> Self {
        use SandboxRestriction::*;
        let mut flags = Self(u16::MAX);
        let has = |keyword: &str| input.split(|u| matches!(*u, 9 | 10 | 12 | 13 | 32))
            .any(|token| ascii_equals(token, keyword));
        let mut clear = |restriction: SandboxRestriction| flags.0 &= !(1 << restriction as u8);
        for (token, restriction) in [
            ("allow-popups", AuxiliaryNavigation), ("allow-same-origin", Origin),
            ("allow-forms", Forms), ("allow-pointer-lock", PointerLock),
            ("allow-popups-to-escape-sandbox", PropagateToAuxiliary),
            ("allow-modals", Modals), ("allow-orientation-lock", OrientationLock),
            ("allow-presentation", Presentation), ("allow-downloads", Downloads),
        ] { if has(token) { clear(restriction); } }
        if has("allow-scripts") { clear(Scripts); clear(AutomaticFeatures); }
        if has("allow-top-navigation") {
            clear(TopNavigationWithoutActivation); clear(TopNavigationWithActivation);
        } else if has("allow-top-navigation-by-user-activation") { clear(TopNavigationWithActivation); }
        if has("allow-top-navigation-to-custom-protocols") || has("allow-popups") || has("allow-top-navigation") {
            clear(CustomProtocolsNavigation);
        }
        flags
    }
    /// Caller supplies actual popup state when there is no embedder, or the
    /// actual iframe and parent active restrictions when there is one.
    pub fn creation(popup: Self, embedder: Option<(Self, Self)>) -> Self {
        embedder.map_or(popup, |(iframe, parent_active)| iframe.union(parent_active))
    }
}
pub(crate) fn ascii_equals(units: &[u16], keyword: &str) -> bool {
    units.len() == keyword.len() && units.iter().copied().zip(keyword.bytes()).all(|(unit, byte)| {
        let folded = if (65..=90).contains(&unit) { unit + 32 } else { unit };
        folded == u16::from(byte)
    })
}
