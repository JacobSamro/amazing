use crate::*;
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RedirectAction {
    NotRedirect,
    Follow,
    Manual(Response),
    NavigationPending,
}
/// `location` must be resolved against the current URL by the URL layer.
/// None means no Location field, not a failed parse (which is a network error).
/// Validates the source hop before changing the request. Navigation/manual
/// returns a controller handoff; it must not expose an opaque JS response.
pub fn redirect(
    request: &mut Request,
    response: &InternalResponse,
    location: Option<HttpUrl>,
) -> Result<RedirectAction, FetchError> {
    if !matches!(response.status, 301 | 302 | 303 | 307 | 308) {
        return Ok(RedirectAction::NotRedirect);
    }
    let plan = request.prepare()?;
    let csp = request.csp.post_request(request, request.url());
    request
        .csp_violations
        .extend(csp.violations.iter().cloned());
    if csp.blocked {
        return Err(FetchError::Csp(csp.violations));
    }
    // Network redirect responses receive CORP checks before the next URL is
    // adopted, including same-origin redirects carrying their own policy.
    check_redirect_corp(request, response, &crate::psl::PublicSuffixList)?;
    if plan.taint == ResponseType::Cors {
        cors_check(request, &response.headers)?;
    }
    match request.redirect {
        RedirectMode::Error => return Err(FetchError::Redirect),
        RedirectMode::Manual => {
            if request.mode == RequestMode::Navigate {
                return Ok(RedirectAction::NavigationPending);
            }
            if !crate::sri::Metadata::parse(&request.integrity).is_empty() {
                return Err(FetchError::Integrity);
            }
            let mut filtered = Response::hidden(ResponseType::OpaqueRedirect);
            filtered.url_list = request.url_list().to_vec();
            return Ok(RedirectAction::Manual(filtered));
        }
        RedirectMode::Follow => (),
    }
    let Some(mut target) = location else {
        return Ok(RedirectAction::NotRedirect);
    };
    if request.url_list.len() > 20 {
        return Err(FetchError::RedirectLimit);
    }
    if target.includes_credentials()
        && ((request.mode == RequestMode::Cors && request.origin != *target.origin())
            || plan.taint == ResponseType::Cors)
    {
        return Err(FetchError::Cors);
    }
    // Fetch checks replayability before method rewriting, except for 303.
    if response.status != 303 && matches!(request.body(), Some(Body::Unreplayable)) {
        return Err(FetchError::BodyNotReplayable);
    }
    let rewrite = (matches!(response.status, 301 | 302) && request.method() == "POST")
        || (response.status == 303 && !matches!(request.method(), "GET" | "HEAD"));
    if rewrite {
        request.set_method_body("GET", None)?;
        request.headers.retain(|n| !crate::headers::body_header(n));
    }
    if request.url().origin() != target.origin() {
        request.headers.remove("authorization");
    }
    // Referrer is reduced at each hop; a later policy cannot recover information
    // removed by a preceding hop's policy.
    request.referrer = determine_referrer(
        request.referrer.as_ref(),
        request.url(),
        request.referrer_policy,
    )
    .and_then(|s| HttpUrl::parse(&s).ok());
    if let Some(policy) = response
        .headers
        .get("referrer-policy")
        .and_then(|v| ReferrerPolicy::parse_header(&v))
    {
        request.referrer_policy = policy;
    }
    target.inherit_fragment(request.url());
    request.url_list.push(target);
    Ok(RedirectAction::Follow)
}

/// Resolve a raw Location with the shared URL parser before following it.
/// Absent Location remains distinct from a malformed/unsupported target. Manual
/// and error modes do not resolve or expose Location, matching HTTP fetch.
pub fn redirect_location(
    request: &mut Request,
    response: &InternalResponse,
    location: Option<&str>,
) -> Result<RedirectAction, FetchError> {
    let target = if matches!(response.status, 301 | 302 | 303 | 307 | 308)
        && request.redirect == RedirectMode::Follow
    {
        location
            .map(|value| request.url().resolve(value))
            .transpose()?
    } else {
        None
    };
    redirect(request, response, target)
}
