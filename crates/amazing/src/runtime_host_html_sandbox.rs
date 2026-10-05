//! Actual iframe sandbox attribute snapshot for existing HTML driver consumers.
//! Full restrictions are separate from the unchanged legacy Net permission bits.
use amazing_html::sandbox::{SandboxingFlags, SandboxRestriction};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct SandboxFlags {
    restrictions: SandboxingFlags,
    attribute_present: bool,
}
pub(super) fn parse_sandbox(attribute: Option<&[u16]>) -> SandboxFlags {
    SandboxFlags { restrictions: SandboxingFlags::from_attribute(attribute), attribute_present: attribute.is_some() }
}
impl SandboxFlags {
    pub(super) fn scripts_restricted(self) -> bool { self.restrictions.contains(SandboxRestriction::Scripts) }
    pub(super) fn origin_restricted(self) -> bool { self.restrictions.contains(SandboxRestriction::Origin) }
    /// Keep the existing permission-bit ABI; do NOT serialize restriction bits.
    pub(super) fn to_bits(self) -> u32 {
        if !self.attribute_present { return 0; }
        1 | ((!self.scripts_restricted()) as u32) << 1 | ((!self.origin_restricted()) as u32) << 2
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn typed_attribute_snapshot_preserves_existing_permission_bit_contract() {
        for (value, bits) in [(None, 0), (Some(""), 1), (Some("allow-scripts"), 3),
            (Some("allow-same-origin"), 5), (Some("allow-scripts allow-same-origin"), 7), (Some("allow-forms"), 1)] {
            let units = value.map(|v| v.encode_utf16().collect::<Vec<_>>());
            assert_eq!(parse_sandbox(units.as_deref()).to_bits(), bits);
        }
        assert_eq!(SandboxFlags::default().to_bits(), 0);
        assert!(!SandboxFlags::default().scripts_restricted());
        assert!(!SandboxFlags::default().origin_restricted());
    }
    #[test]
    fn actual_attribute_units_and_old_navigation_snapshots_remain_separate() {
        let mut value = "allow-scripts".encode_utf16().collect::<Vec<_>>();
        let active = parse_sandbox(Some(&value));
        value.extend([0xd800]);
        let next = parse_sandbox(Some(&value));
        assert!(!active.scripts_restricted()); assert!(next.scripts_restricted());
        assert!(parse_sandbox(Some(&"allow-scripts\u{b}allow-same-origin".encode_utf16().collect::<Vec<_>>())).scripts_restricted());
        assert!(parse_sandbox(Some(&"allow-forms".encode_utf16().collect::<Vec<_>>())).restrictions.contains(SandboxRestriction::Scripts));
        assert!(!parse_sandbox(Some(&"allow-forms".encode_utf16().collect::<Vec<_>>())).restrictions.contains(SandboxRestriction::Forms));
    }
}
