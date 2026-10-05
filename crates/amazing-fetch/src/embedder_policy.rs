//! HTML cross-origin embedder policy values from actual response headers.
//! The environment owner supplies its real secure-context state; URL shape is
//! not substituted for that state. Policy containers carry this value intact.
use crate::{
    EmbedderPolicy, HeaderList,
    structured_fields::{Parser, Value},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmbedderPolicyContainer {
    pub value: EmbedderPolicy,
    pub reporting_endpoint: Option<String>,
    pub report_only_value: EmbedderPolicy,
    pub report_only_reporting_endpoint: Option<String>,
}
impl Default for EmbedderPolicyContainer {
    fn default() -> Self {
        Self {
            value: EmbedderPolicy::UnsafeNone,
            reporting_endpoint: None,
            report_only_value: EmbedderPolicy::UnsafeNone,
            report_only_reporting_endpoint: None,
        }
    }
}
impl EmbedderPolicy {
    pub fn compatible_with_cross_origin_isolation(self) -> bool {
        matches!(self, Self::RequireCorp | Self::Credentialless)
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UnsafeNone => "unsafe-none",
            Self::RequireCorp => "require-corp",
            Self::Credentialless => "credentialless",
        }
    }
}
impl EmbedderPolicyContainer {
    /// HTML obtain-an-embedder-policy: invalid/absent/unknown fields and real
    /// non-secure environments have the standard initial unsafe-none value.
    pub fn from_response(headers: &HeaderList, secure_context: bool) -> Self {
        if !secure_context {
            return Self::default();
        }
        let (value, reporting_endpoint) = parse_header(headers, "cross-origin-embedder-policy");
        let (report_only_value, report_only_reporting_endpoint) =
            parse_header(headers, "cross-origin-embedder-policy-report-only");
        Self {
            value,
            reporting_endpoint,
            report_only_value,
            report_only_reporting_endpoint,
        }
    }
    /// A fresh nested navigation or dedicated worker may use either isolation
    /// compatible value when its owner requires isolation; values need not match.
    /// Top-level navigation and non-dedicated workers do not use this check.
    pub fn check_inheritance(
        &self,
        child: &Self,
        owner: EmbedderPolicyOwner,
    ) -> EmbedderInheritanceCheck {
        let incompatible =
            owner.requires_owner_check() && !child.value.compatible_with_cross_origin_isolation();
        EmbedderInheritanceCheck {
            allowed: !incompatible || !self.value.compatible_with_cross_origin_isolation(),
            report_only_violation: incompatible
                && self
                    .report_only_value
                    .compatible_with_cross_origin_isolation(),
            enforced_violation: incompatible && self.value.compatible_with_cross_origin_isolation(),
        }
    }
}
/// Actual environment owner selects the native creation boundary. This is not
/// inferred from response URL, destination strings, or an arbitrary caller flag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmbedderPolicyOwner {
    NestedNavigation,
    DedicatedWorker,
    TopLevelNavigation,
    SharedWorker,
    ServiceWorker,
    Worklet,
}
impl EmbedderPolicyOwner {
    pub fn requires_owner_check(self) -> bool {
        matches!(self, Self::NestedNavigation | Self::DedicatedWorker)
    }
}

/// The owner reports each indicated disposition against its authentic global;
/// checking/reporting-only never silently upgrades or changes the child policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EmbedderInheritanceCheck {
    pub allowed: bool,
    pub report_only_violation: bool,
    pub enforced_violation: bool,
}
fn parse_header(headers: &HeaderList, name: &str) -> (EmbedderPolicy, Option<String>) {
    let Some(input) = headers.get(name) else {
        return (EmbedderPolicy::UnsafeNone, None);
    };
    let mut parser = Parser::new(input.as_bytes());
    let value = match parser.parse_item() {
        Ok(Some(Value::Token(range))) => match &input[range] {
            "require-corp" => EmbedderPolicy::RequireCorp,
            "credentialless" => EmbedderPolicy::Credentialless,
            _ => EmbedderPolicy::UnsafeNone,
        },
        _ => return (EmbedderPolicy::UnsafeNone, None),
    };
    let mut endpoint = None;
    loop {
        match parser.parse_param() {
            Ok(Some((name, field))) if name == "report-to" => {
                // Only a string supplies a named endpoint. Later duplicate
                // parameters replace earlier values, including a non-string.
                endpoint = match field {
                    Value::String { range, .. } => {
                        let mut text = String::new();
                        let mut units = input[range].chars();
                        while let Some(unit) = units.next() {
                            text.push(if unit == '\\' {
                                units.next().expect("parsed string escape")
                            } else {
                                unit
                            });
                        }
                        Some(text)
                    }
                    _ => None,
                };
            }
            Ok(Some(_)) => {}
            Ok(None) => break,
            Err(_) => return (EmbedderPolicy::UnsafeNone, None),
        }
    }
    if parser.parse_item() != Ok(None) {
        return (EmbedderPolicy::UnsafeNone, None);
    }
    if !value.compatible_with_cross_origin_isolation() {
        endpoint = None;
    }
    (value, endpoint)
}
