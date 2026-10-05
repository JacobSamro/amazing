//! CSP inline behavior and violation production. Implements CSP3 sections
//! 4.2.3, 6.7.3 and 6.8.2 from the 16 September 2026 primary editor's draft.
//! The embedder supplies native element metadata and queues returned violations.
use super::{fallback, CspDisposition, CspPolicyList};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CspInlineKind {
    Script,
    ScriptAttribute,
    Navigation,
    Style,
    StyleAttribute,
}
impl CspInlineKind {
    pub fn effective_directive(self) -> &'static str {
        match self {
            Self::Script | Self::Navigation => "script-src-elem",
            Self::ScriptAttribute => "script-src-attr",
            Self::Style => "style-src-elem",
            Self::StyleAttribute => "style-src-attr",
        }
    }
    fn is_element(self) -> bool {
        matches!(self, Self::Script | Self::Style)
    }
    fn is_script(self) -> bool {
        matches!(
            self,
            Self::Script | Self::ScriptAttribute | Self::Navigation
        )
    }
}
/// Attribute strings before any lossy UTF-16 conversion. Include all available
/// tokenizer attributes; duplicate rejection additionally uses the parser flag.
#[derive(Debug, Clone, Copy)]
pub struct CspInlineAttribute<'a> {
    pub name: &'a [u16],
    pub value: &'a [u16],
}
/// Current element/document context, captured at the inline behavior check.
/// `nonce` is the internal cryptographic nonce, not getAttribute("nonce").
/// `nonce_attribute_present` distinguishes a removed nonce from a hidden value.
/// Source is exactly the HTML script text/event-handler/navigation source; this
/// producer neither trims it nor normalizes newlines. Hashing converts isolated
/// surrogates to U+FFFD per Infra's JavaScript string conversion, then UTF-8.
#[derive(Debug, Clone)]
pub struct CspInlineContext<'a> {
    pub kind: CspInlineKind,
    pub source: &'a [u16],
    pub nonce: Option<&'a [u16]>,
    pub nonce_attribute_present: bool,
    pub attributes: &'a [CspInlineAttribute<'a>],
    pub duplicate_attribute: bool,
    pub parser_inserted: bool,
    /// The opaque native element identity is returned unchanged to the reporter.
    pub element: Option<u64>,
    pub document_url: &'a str,
    pub referrer: &'a str,
    pub status_code: u16,
    pub source_file: &'a str,
    pub line_number: u32,
    pub column_number: u32,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CspInlineViolation {
    pub effective_directive: String,
    /// Legacy event field equals effectiveDirective (CSP3 section 5.5).
    pub violated_directive: String,
    /// Native diagnostic: the actual directive selected by fallback.
    pub source_directive: String,
    pub original_policy: String,
    pub policy_index: usize,
    pub disposition: CspDisposition,
    pub blocked_uri: String,
    pub element: Option<u64>,
    pub document_uri: String,
    pub referrer: String,
    pub status_code: u16,
    pub source_file: String,
    pub line_number: u32,
    pub column_number: u32,
    /// Exact first 40 UTF-16 code units, only when report-sample is present.
    pub sample: Vec<u16>,
    /// Declared report-to value. A present empty value still supersedes
    /// report-uri; endpoint validation/resolution belongs to the reporter.
    pub report_to: Option<String>,
    pub report_uris: Vec<String>,
}
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CspInlineCheck {
    pub blocked: bool,
    pub violations: Vec<CspInlineViolation>,
}
impl CspPolicyList {
    /// Evaluate every policy independently. Report-only violations never block,
    /// and an enforcing failure never prevents later policies being reported.
    pub fn check_inline(&self, context: &CspInlineContext<'_>) -> CspInlineCheck {
        let mut check = CspInlineCheck::default();
        let effective = context.kind.effective_directive();
        let chain = match effective {
            "script-src-attr" => &["script-src-attr", "script-src", "default-src"][..],
            "style-src-attr" => &["style-src-attr", "style-src", "default-src"][..],
            _ => fallback(effective),
        };
        // Hash at most once per algorithm across every policy, after nonce/
        // unsafe-inline matching. UTF-16 source identity is preserved separately.
        let mut hashes: [Option<String>; 3] = [None, None, None];
        for (policy_index, policy) in self.policies.iter().enumerate() {
            let Some(directive) = chain
                .iter()
                .find_map(|name| policy.directives.iter().find(|d| d.name == *name))
            else {
                continue;
            };
            if matches_inline(&directive.sources, context, &mut hashes) {
                continue;
            }
            check.blocked |= policy.disposition == CspDisposition::Enforce;
            let reporting = |name: &str| policy.directives.iter().find(|d| d.name == name);
            check.violations.push(CspInlineViolation {
                effective_directive: effective.into(),
                violated_directive: effective.into(),
                source_directive: directive.name.clone(),
                original_policy: policy.original_policy.clone(),
                policy_index,
                disposition: policy.disposition,
                blocked_uri: "inline".into(),
                element: if context.kind == CspInlineKind::Navigation {
                    None
                } else {
                    context.element
                },
                document_uri: strip_report_url(context.document_url),
                referrer: strip_report_url(context.referrer),
                status_code: context.status_code,
                source_file: strip_report_url(context.source_file),
                line_number: context.line_number,
                column_number: context.column_number,
                sample: if context.kind != CspInlineKind::Navigation
                    && keyword(&directive.sources, "'report-sample'")
                {
                    context.source.iter().take(40).copied().collect()
                } else {
                    Vec::new()
                },
                report_to: reporting("report-to").map(|d| d.sources.join(" ")),
                report_uris: reporting("report-uri")
                    .map(|d| d.sources.clone())
                    .unwrap_or_default(),
            });
        }
        check
    }
}
/// CSP's nonceability check is deliberately separate from matching a nonce.
/// Other native producers can use it before starting an external fetch.
pub fn csp_inline_nonceable(context: &CspInlineContext<'_>) -> bool {
    if !context.nonce_attribute_present || context.duplicate_attribute {
        return false;
    }
    if context.kind == CspInlineKind::Script {
        for attribute in context.attributes {
            for text in [attribute.name, attribute.value] {
                if ["<script", "<style", "<link"]
                    .iter()
                    .any(|needle| contains_ascii(text, needle))
                {
                    return false;
                }
            }
        }
    }
    true
}
fn contains_ascii(text: &[u16], needle: &str) -> bool {
    text.windows(needle.len()).any(|part| {
        part.iter()
            .zip(needle.bytes())
            .all(|(&a, b)| a <= 127 && (a as u8).eq_ignore_ascii_case(&b))
    })
}
fn keyword(sources: &[String], token: &str) -> bool {
    sources.iter().any(|s| s.eq_ignore_ascii_case(token))
}
/// ABNF base64-value is syntactic, not a successfully decoded digest. Even a
/// grammatically valid nonce/hash of the wrong length suppresses unsafe-inline.
fn base64_value(value: &str) -> bool {
    let body = value.trim_end_matches('=');
    !body.is_empty()
        && value.len() - body.len() <= 2
        && body
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'-' | b'_'))
}
fn nonce_source(source: &str) -> Option<&str> {
    let inner = source.strip_prefix('\'')?.strip_suffix('\'')?;
    let (prefix, value) = inner.split_once('-')?;
    (prefix.eq_ignore_ascii_case("nonce") && base64_value(value)).then_some(value)
}
fn hash_source(source: &str) -> Option<(usize, &str)> {
    let inner = source.strip_prefix('\'')?.strip_suffix('\'')?;
    let (algorithm, value) = inner.split_once('-')?;
    if !base64_value(value) {
        return None;
    }
    let rank = if algorithm.eq_ignore_ascii_case("sha256") {
        0
    } else if algorithm.eq_ignore_ascii_case("sha384") {
        1
    } else if algorithm.eq_ignore_ascii_case("sha512") {
        2
    } else {
        return None;
    };
    Some((rank, value))
}
fn matches_inline(
    sources: &[String],
    c: &CspInlineContext<'_>,
    hashes: &mut [Option<String>; 3],
) -> bool {
    let suppress = sources
        .iter()
        .any(|s| nonce_source(s).is_some() || hash_source(s).is_some())
        || (c.kind.is_script() && keyword(sources, "'strict-dynamic'"));
    if !suppress && keyword(sources, "'unsafe-inline'") {
        return true;
    }
    if c.kind.is_element() && csp_inline_nonceable(c) {
        if let Some(nonce) = c.nonce {
            if sources
                .iter()
                .filter_map(|s| nonce_source(s))
                .any(|value| value.encode_utf16().eq(nonce.iter().copied()))
            {
                return true;
            }
        }
    }
    if !c.kind.is_element() && !keyword(sources, "'unsafe-hashes'") {
        return false;
    }
    for source in sources {
        // Current primary CSP3 permits non-parser-inserted inline scripts.
        if c.kind == CspInlineKind::Script
            && !c.parser_inserted
            && source.eq_ignore_ascii_case("'strict-dynamic'")
        {
            return true;
        }
        if let Some((rank, expected)) = hash_source(source) {
            let actual = hashes[rank].get_or_insert_with(|| {
                let utf8 = String::from_utf16_lossy(c.source);
                encode_base64(&crate::sri::sha2::digest(rank, utf8.as_bytes()))
            });
            // CSP compares base64 text (padding included), not decoded bytes.
            if *actual == expected.replace('-', "+").replace('_', "/") {
                return true;
            }
        }
    }
    false
}
fn encode_base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut encoded = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let a = chunk[0];
        let b = chunk.get(1).copied().unwrap_or(0);
        let c = chunk.get(2).copied().unwrap_or(0);
        encoded.push(ALPHABET[(a >> 2) as usize] as char);
        encoded.push(ALPHABET[((a & 3) << 4 | b >> 4) as usize] as char);
        encoded.push(if chunk.len() > 1 {
            ALPHABET[((b & 15) << 2 | c >> 6) as usize] as char
        } else {
            '='
        });
        encoded.push(if chunk.len() > 2 {
            ALPHABET[(c & 63) as usize] as char
        } else {
            '='
        });
    }
    encoded
}

/// CSP section 5.4: HTTP(S) report URLs omit credentials and fragments;
/// non-HTTP(S) URLs reveal only their scheme. Empty nullable URLs stay empty.
pub fn csp_strip_report_url(input: &str) -> String {
    strip_report_url(input)
}
fn strip_report_url(input: &str) -> String {
    if input.is_empty() {
        return String::new();
    }
    let Ok(url) = amazing_net::ParsedUrl::parse(input) else {
        return String::new();
    };
    if matches!(url.scheme(), "http" | "https") {
        url.serialize_referrer()
    } else {
        url.scheme().to_owned()
    }
}
