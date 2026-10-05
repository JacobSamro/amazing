//! Native Meta policy text, kept separate from decoded HTTP header provenance.
use super::{
    CspDirective, CspDisposition, CspParseError, CspPolicy, CspPolicyList, CspPolicySource,
    MAX_DIRECTIVES, MAX_POLICY_BYTES, is_ascii_whitespace, valid_source,
};

impl CspPolicyList {
    /// Parse the actual native Meta content attribute DOMString. HTML owns the
    /// insertion/checkpoint, head membership and absent/empty attribute checks.
    /// This parser does not manufacture a document or an insertion receipt.
    ///
    /// Bounded to 65536 UTF16 units and 256 semicolon-delimited directives.
    /// NonASCII directives, including isolated surrogates, are skipped whole
    /// before any conversion. They remain exact in original_policy_units.
    /// Meta is one enforcing policy; commas never invent a second policy.
    pub fn from_meta_utf16(serialized: &[u16]) -> Result<Self, CspParseError> {
        if serialized.len() > MAX_POLICY_BYTES {
            return Err(CspParseError::TooLong);
        }
        if serialized
            .iter()
            .filter(|unit| **unit == u16::from(b';'))
            .count()
            >= MAX_DIRECTIVES
        {
            return Err(CspParseError::TooManyDirectives);
        }
        let mut policy = CspPolicy {
            source: CspPolicySource::Meta,
            // This legacy String is never parsed or used as DOMString authority.
            original_policy: String::from_utf16_lossy(serialized)
                .trim_matches(is_ascii_whitespace)
                .into(),
            original_policy_units: serialized.to_vec(),
            disposition: CspDisposition::Enforce,
            directives: Vec::new(),
        };
        for part in serialized.split(|unit| *unit == u16::from(b';')) {
            if part.iter().any(|unit| *unit > 0x7f) {
                continue;
            }
            // The complete directive was proven ASCII before this bounded copy.
            let part: String = part.iter().map(|unit| char::from(*unit as u8)).collect();
            let mut tokens = part
                .split(is_ascii_whitespace)
                .filter(|token| !token.is_empty());
            let Some(name) = tokens.next() else {
                continue;
            };
            if !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            {
                continue;
            }
            let name = name.to_ascii_lowercase();
            if matches!(name.as_str(), "report-uri" | "frame-ancestors" | "sandbox")
                || policy
                    .directives
                    .iter()
                    .any(|directive| directive.name == name)
            {
                continue;
            }
            policy.directives.push(CspDirective {
                name,
                sources: tokens
                    .filter(|token| valid_source(token))
                    .map(str::to_owned)
                    .collect(),
            });
        }
        Ok(Self {
            policies: vec![policy],
        })
    }
}

impl CspPolicyList {
    /// Atomic bounded policy-container admission after the HTML owner has
    /// validated a genuine native Meta insertion checkpoint. This method does
    /// not validate/mint such a checkpoint. Limits include inherited policies.
    pub fn append_meta_utf16(&mut self, serialized: &[u16]) -> Result<(), CspParseError> {
        if self.policies.len() >= super::MAX_POLICIES {
            return Err(CspParseError::TooManyPolicies);
        }
        let retained = self
            .policies
            .iter()
            .try_fold(0usize, |total, policy| {
                total.checked_add(policy.original_policy_units.len())
            })
            .ok_or(CspParseError::TooLong)?;
        if serialized.len() > MAX_POLICY_BYTES.saturating_sub(retained) {
            return Err(CspParseError::TooLong);
        }
        let mut meta = Self::from_meta_utf16(serialized)?;
        let directives = self
            .policies
            .iter()
            .try_fold(0usize, |total, policy| {
                total.checked_add(policy.directives.len())
            })
            .ok_or(CspParseError::TooManyDirectives)?;
        if meta.policies[0].directives.len() > MAX_DIRECTIVES.saturating_sub(directives) {
            return Err(CspParseError::TooManyDirectives);
        }
        self.policies.append(&mut meta.policies);
        Ok(())
    }
}
