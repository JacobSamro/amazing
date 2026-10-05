//! Lossless browser-native receiving capture and bounded network-role adoption.
//! URL fields carry actual captured values; they never produce origin/ancestry.
use super::*;
use amazing_fetch::csp::{CspDirective, CspDisposition, CspPolicy, CspPolicySource};
use amazing_fetch::{EmbedderPolicyContainer, ReceivingDocumentClient, ReferrerPolicy, Site};
use amazing_net::ParsedUrl;

fn policy(value: &str) -> Result<EmbedderPolicy, Error> {
    match value {
        "unsafe-none" => Ok(EmbedderPolicy::UnsafeNone),
        "require-corp" => Ok(EmbedderPolicy::RequireCorp),
        "credentialless" => Ok(EmbedderPolicy::Credentialless),
        _ => Err(invalid("receiving COEP value")),
    }
}
fn fetch_destination(value: &str) -> Result<Destination, Error> {
    Ok(match value {
        "" => Destination::Empty,
        "document" => Destination::Document,
        "image" => Destination::Image,
        "font" => Destination::Font,
        "audio" => Destination::Audio,
        "video" => Destination::Video,
        "script" => Destination::Script,
        "worker" => Destination::Worker,
        "sharedworker" => Destination::SharedWorker,
        "serviceworker" => Destination::ServiceWorker,
        "style" => Destination::Style,
        "other" => Destination::Other,
        _ => return Err(invalid("receiving destination")),
    })
}
fn csp_destination(value: &str) -> Result<CspDestination, Error> {
    Ok(match value {
        "" => CspDestination::None,
        "script" => CspDestination::Script,
        "script-element" => CspDestination::ScriptElement,
        "style" => CspDestination::Style,
        "style-element" => CspDestination::StyleElement,
        "image" => CspDestination::Image,
        "connect" => CspDestination::Connect,
        "font" => CspDestination::Font,
        "media" => CspDestination::Media,
        "object" => CspDestination::Object,
        "frame" => CspDestination::Frame,
        "child" => CspDestination::Child,
        "worker" => CspDestination::Worker,
        "manifest" => CspDestination::Manifest,
        _ => return Err(invalid("receiving CSP destination")),
    })
}
fn media(value: &str) -> Result<NoCorsMediaState, Error> {
    match value {
        "not-media" => Ok(NoCorsMediaState::NotMedia),
        "initial" => Ok(NoCorsMediaState::Initial),
        "subsequent" => Ok(NoCorsMediaState::Subsequent),
        _ => Err(invalid("receiving media state")),
    }
}
fn exact_referrer(value: &str) -> Result<ReferrerPolicy, Error> {
    if !matches!(
        value,
        "no-referrer"
            | "no-referrer-when-downgrade"
            | "same-origin"
            | "origin"
            | "strict-origin"
            | "origin-when-cross-origin"
            | "strict-origin-when-cross-origin"
            | "unsafe-url"
    ) {
        return Err(invalid("receiving referrer policy"));
    }
    ReferrerPolicy::parse_header(value).ok_or_else(|| invalid("receiving referrer policy"))
}
fn limited_url(value: &str) -> Result<ParsedUrl, Error> {
    if value.len() > MAX_URL_BYTES {
        return Err(invalid("receiving URL limit"));
    }
    ParsedUrl::parse(value).map_err(|_| invalid("receiving URL"))
}
pub(super) fn decode_csp(values: Vec<ReceivingCspPolicy>) -> Result<CspPolicyList, Error> {
    if values.len() > 32 {
        return Err(invalid("receiving CSP count"));
    }
    let mut retained = 0usize;
    let mut policies = Vec::with_capacity(values.len());
    for value in values {
        retained = retained
            .checked_add(value.original_policy.len())
            .and_then(|n| n.checked_add(value.original_policy_units.len().checked_mul(2)?))
            .ok_or_else(|| invalid("receiving CSP size"))?;
        if value.directives.len() > 128 {
            return Err(invalid("receiving CSP directives"));
        }
        let mut directives = Vec::with_capacity(value.directives.len());
        for directive in value.directives {
            if directive.name.is_empty()
                || directive.name.len() > 128
                || !directive
                    .name
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
                || directive.sources.len() > 256
            {
                return Err(invalid("receiving CSP directive"));
            }
            retained = retained
                .checked_add(directive.name.len())
                .ok_or_else(|| invalid("receiving CSP size"))?;
            for source in &directive.sources {
                retained = retained
                    .checked_add(source.len())
                    .ok_or_else(|| invalid("receiving CSP size"))?;
            }
            directives.push(CspDirective {
                name: directive.name,
                sources: directive.sources,
            });
        }
        if retained > MAX_HEADER_BYTES {
            return Err(invalid("receiving CSP size"));
        }
        policies.push(CspPolicy {
            source: match value.source.as_str() {
                "header" => CspPolicySource::Header,
                "meta" => CspPolicySource::Meta,
                _ => return Err(invalid("receiving CSP source")),
            },
            disposition: match value.disposition.as_str() {
                "enforce" => CspDisposition::Enforce,
                "report-only" => CspDisposition::ReportOnly,
                _ => return Err(invalid("receiving CSP disposition")),
            },
            original_policy: value.original_policy,
            original_policy_units: value.original_policy_units,
            directives,
        });
    }
    Ok(CspPolicyList { policies })
}
pub(super) fn decode_context(value: ReceivingClientContext) -> Result<ClientContext, Error> {
    if value.frame_ancestors.len() > 32 {
        return Err(invalid("receiving ancestors"));
    }
    let cookie_site = match value.cookie_site {
        WireCookieSite::NoClient(()) => CookieSite::NoClient,
        WireCookieSite::CrossSite(()) => CookieSite::CrossSite,
        WireCookieSite::Origin(v) => CookieSite::Origin(origin(&v)?),
    };
    let partition_key = match value.partition_key {
        None => None,
        Some(ReceivingSite::Opaque(id)) if id != 0 => Some(Site::Opaque(id)),
        Some(ReceivingSite::Tuple(site))
            if matches!(site.scheme.as_str(), "http" | "https")
                && !site.domain.is_empty()
                && site.domain.len() <= 253
                && site.domain.bytes().all(|b| {
                    b.is_ascii_lowercase()
                        || b.is_ascii_digit()
                        || matches!(b, b'.' | b'-' | b':' | b'[' | b']')
                }) =>
        {
            Some(Site::Tuple {
                scheme: site.scheme,
                domain: site.domain,
            })
        }
        _ => return Err(invalid("receiving partition key")),
    };
    Ok(ClientContext {
        cookie_site,
        partition_key,
        frame_ancestors: value
            .frame_ancestors
            .iter()
            .map(origin)
            .collect::<Result<_, _>>()?,
        security: SecurityContext {
            client_restricts_mixed_content: value.restricts_mixed_content,
            destination: fetch_destination(&value.destination)?,
            image_set_initiator: value.image_set_initiator,
            top_level_navigation: value.top_level_navigation,
            embedder_policy: policy(&value.embedder_policy)?,
            no_cors_media_state: media(&value.no_cors_media_state)?,
        },
        csp: decode_csp(value.csp)?,
        csp_destination: csp_destination(&value.csp_destination)?,
        now_unix: value.now_unix,
    })
}
pub(super) fn decode_client(
    value: ReceivingClientCapture,
) -> Result<ReceivingDocumentClient<ReceivingGlobalIdentity>, Error> {
    if value.identity.realm_binding == 0
        || value.identity.generation == 0
        || value.identity.document > u64::from(u32::MAX)
    {
        return Err(invalid("receiving global identity"));
    }
    Ok(ReceivingDocumentClient {
        identity: value.identity,
        creation_url: limited_url(&value.creation_url)?,
        document_url: limited_url(&value.document_url)?,
        base_url: limited_url(&value.base_url)?,
        origin: origin(&value.origin)?,
        csp_self_origin: origin(&value.csp_self_origin)?,
        context: decode_context(value.context)?,
        referrer_policy: exact_referrer(&value.referrer_policy)?,
    })
}
pub(super) fn decode_coep(value: ReceivingCoep) -> Result<EmbedderPolicyContainer, Error> {
    for endpoint in [
        &value.reporting_endpoint,
        &value.report_only_reporting_endpoint,
    ]
    .into_iter()
    .flatten()
    {
        if endpoint.is_empty() || endpoint.len() > 1024 {
            return Err(invalid("receiving COEP endpoint"));
        }
    }
    Ok(EmbedderPolicyContainer {
        value: policy(&value.value)?,
        reporting_endpoint: value.reporting_endpoint,
        report_only_value: policy(&value.report_only_value)?,
        report_only_reporting_endpoint: value.report_only_reporting_endpoint,
    })
}
pub(super) fn decode_parent(value: ReceivingNavigable) -> Result<NavigableContext, Error> {
    if value.ancestor_origins.is_empty()
        || value.ancestor_origins.len() > 32
        || value.sandbox_flags & !0xffff != 0
        || value.referrer.len() > MAX_URL_BYTES
    {
        return Err(invalid("receiving navigable bounds"));
    }
    exact_referrer(&value.referrer_policy)?;
    Ok(NavigableContext {
        top_level_origin: origin(&value.top_level_origin)?,
        parent_origin: origin(&value.parent_origin)?,
        ancestor_origins: value
            .ancestor_origins
            .iter()
            .map(origin)
            .collect::<Result<_, _>>()?,
        sandbox_flags: value.sandbox_flags,
        frame_csp: decode_csp(value.frame_csp)?,
        referrer: value.referrer,
        referrer_policy: value.referrer_policy,
        now_unix: value.now_unix,
    })
}

fn encode_csp(value: &CspPolicyList) -> Vec<ReceivingCspPolicy> {
    value
        .policies
        .iter()
        .map(|p| ReceivingCspPolicy {
            source: match p.source {
                CspPolicySource::Header => "header",
                CspPolicySource::Meta => "meta",
            }
            .into(),
            disposition: match p.disposition {
                CspDisposition::Enforce => "enforce",
                CspDisposition::ReportOnly => "report-only",
            }
            .into(),
            original_policy: p.original_policy.clone(),
            original_policy_units: p.original_policy_units.clone(),
            directives: p
                .directives
                .iter()
                .map(|d| ReceivingCspDirective {
                    name: d.name.clone(),
                    sources: d.sources.clone(),
                })
                .collect(),
        })
        .collect()
}
pub(super) fn referrer_name(value: ReferrerPolicy) -> &'static str {
    match value {
        ReferrerPolicy::NoReferrer => "no-referrer",
        ReferrerPolicy::NoReferrerWhenDowngrade => "no-referrer-when-downgrade",
        ReferrerPolicy::SameOrigin => "same-origin",
        ReferrerPolicy::Origin => "origin",
        ReferrerPolicy::StrictOrigin => "strict-origin",
        ReferrerPolicy::OriginWhenCrossOrigin => "origin-when-cross-origin",
        ReferrerPolicy::StrictOriginWhenCrossOrigin => "strict-origin-when-cross-origin",
        ReferrerPolicy::UnsafeUrl => "unsafe-url",
    }
}
impl From<&ClientContext> for ReceivingClientContext {
    fn from(value: &ClientContext) -> Self {
        let security = value.security;
        Self {
            cookie_site: match &value.cookie_site {
                CookieSite::NoClient => WireCookieSite::NoClient(()),
                CookieSite::CrossSite => WireCookieSite::CrossSite(()),
                CookieSite::Origin(v) => WireCookieSite::Origin(origin_to_wire(v)),
            },
            partition_key: value.partition_key.as_ref().map(|s| match s {
                Site::Opaque(id) => ReceivingSite::Opaque(*id),
                Site::Tuple { scheme, domain } => ReceivingSite::Tuple(ReceivingTupleSite {
                    scheme: scheme.clone(),
                    domain: domain.clone(),
                }),
            }),
            frame_ancestors: value.frame_ancestors.iter().map(origin_to_wire).collect(),
            restricts_mixed_content: security.client_restricts_mixed_content,
            destination: match security.destination {
                Destination::Empty => "",
                Destination::Document => "document",
                Destination::Image => "image",
                Destination::Font => "font",
                Destination::Audio => "audio",
                Destination::Video => "video",
                Destination::Script => "script",
                Destination::Worker => "worker",
                Destination::SharedWorker => "sharedworker",
                Destination::ServiceWorker => "serviceworker",
                Destination::Style => "style",
                Destination::Other => "other",
            }
            .into(),
            image_set_initiator: security.image_set_initiator,
            top_level_navigation: security.top_level_navigation,
            embedder_policy: security.embedder_policy.as_str().into(),
            no_cors_media_state: match security.no_cors_media_state {
                NoCorsMediaState::NotMedia => "not-media",
                NoCorsMediaState::Initial => "initial",
                NoCorsMediaState::Subsequent => "subsequent",
            }
            .into(),
            csp: encode_csp(&value.csp),
            csp_destination: match value.csp_destination {
                CspDestination::None => "",
                CspDestination::Script => "script",
                CspDestination::ScriptElement => "script-element",
                CspDestination::Style => "style",
                CspDestination::StyleElement => "style-element",
                CspDestination::Image => "image",
                CspDestination::Connect => "connect",
                CspDestination::Font => "font",
                CspDestination::Media => "media",
                CspDestination::Object => "object",
                CspDestination::Frame => "frame",
                CspDestination::Child => "child",
                CspDestination::Worker => "worker",
                CspDestination::Manifest => "manifest",
            }
            .into(),
            now_unix: value.now_unix,
        }
    }
}
impl From<&ReceivingDocumentClient<ReceivingGlobalIdentity>> for ReceivingClientCapture {
    fn from(value: &ReceivingDocumentClient<ReceivingGlobalIdentity>) -> Self {
        Self {
            identity: value.identity.clone(),
            creation_url: value.creation_url.serialize(),
            document_url: value.document_url.serialize(),
            base_url: value.base_url.serialize(),
            origin: origin_to_wire(&value.origin),
            csp_self_origin: origin_to_wire(&value.csp_self_origin),
            context: (&value.context).into(),
            referrer_policy: referrer_name(value.referrer_policy).into(),
        }
    }
}
impl From<&EmbedderPolicyContainer> for ReceivingCoep {
    fn from(value: &EmbedderPolicyContainer) -> Self {
        Self {
            value: value.value.as_str().into(),
            reporting_endpoint: value.reporting_endpoint.clone(),
            report_only_value: value.report_only_value.as_str().into(),
            report_only_reporting_endpoint: value.report_only_reporting_endpoint.clone(),
        }
    }
}
impl From<&NavigableContext> for ReceivingNavigable {
    fn from(value: &NavigableContext) -> Self {
        Self {
            top_level_origin: origin_to_wire(&value.top_level_origin),
            parent_origin: origin_to_wire(&value.parent_origin),
            ancestor_origins: value.ancestor_origins.iter().map(origin_to_wire).collect(),
            sandbox_flags: value.sandbox_flags,
            frame_csp: encode_csp(&value.frame_csp),
            referrer: value.referrer.clone(),
            referrer_policy: value.referrer_policy.clone(),
            now_unix: value.now_unix,
        }
    }
}
