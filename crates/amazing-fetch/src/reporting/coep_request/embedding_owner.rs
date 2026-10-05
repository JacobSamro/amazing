//! Native embedding settings captured independently of the navigation source.
use super::*;
use crate::api::NavigableContext;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavigationEmbeddingElement {
    Frame,
    Iframe,
}
impl NavigationEmbeddingElement {
    pub fn destination(self) -> &'static str {
        match self {
            Self::Frame => "frame",
            Self::Iframe => "iframe",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NavigationEmbeddingOwnerId {
    service: u64,
    generation: u64,
}
/// Move-only same-service owner settings. No Window roots or fresh-global
/// endpoint authority are retained. Drop releases an unused capture.
#[must_use]
pub struct PreparedNavigationEmbeddingOwner {
    snapshot: Arc<NavigationEmbeddingOwnerSnapshot>,
}
impl PreparedNavigationEmbeddingOwner {
    pub fn id(&self) -> NavigationEmbeddingOwnerId {
        self.snapshot.id
    }
    pub(crate) fn into_snapshot(
        self,
        parent: &NavigableContext,
    ) -> Result<Arc<NavigationEmbeddingOwnerSnapshot>, crate::api::ApiError> {
        if self.snapshot.origin() != &parent.parent_origin
            || self.snapshot.ancestors != parent.ancestor_origins
            || self.snapshot.top_level_origin != parent.top_level_origin
            || self.snapshot.sandbox_flags != parent.sandbox_flags
        {
            return Err(crate::api::ApiError::Network(FetchError::FrameEmbedding));
        }
        Ok(self.snapshot)
    }
}
pub(crate) struct NavigationEmbeddingOwnerSnapshot {
    id: NavigationEmbeddingOwnerId,
    settings: Arc<CoepRequestReportSettings>,
    ancestors: Vec<Origin>,
    top_level_origin: Origin,
    sandbox_flags: u32,
    element: NavigationEmbeddingElement,
    _credits: prepared::PreparedCredits,
}
impl std::fmt::Debug for NavigationEmbeddingOwnerSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NavigationEmbeddingOwnerSnapshot")
            .field("id", &self.id)
            .field("element", &self.element)
            .finish()
    }
}
impl NavigationEmbeddingOwnerSnapshot {
    pub(crate) fn id(&self) -> NavigationEmbeddingOwnerId {
        self.id
    }
    pub(crate) fn origin(&self) -> &Origin {
        &self.settings.saved.origin
    }
    pub(crate) fn embedder_policy(&self) -> EmbedderPolicy {
        self.settings.container.value
    }
    pub(crate) fn ancestors(&self) -> &[Origin] {
        &self.ancestors
    }
    pub(crate) fn destination(&self) -> &'static str {
        self.element.destination()
    }
    pub(crate) fn validate_service(&self, service: u64) -> Result<(), ReportError> {
        if self.id.service != service || self.settings.service != service || self.id.generation == 0
        {
            return Err(ReportError::StaleDocument);
        }
        Ok(())
    }
    pub(crate) fn loan(
        &self,
        service: &NetworkService,
    ) -> Result<CoepRequestReportSettings, ReportError> {
        self.validate_service(service.reporting.service)?;
        let loan = graph_credit(
            &service.reporting.prepared_bytes,
            std::mem::size_of::<CoepRequestReportSettings>(),
        )?;
        let saved = &self.settings;
        Ok(CoepRequestReportSettings {
            service: saved.service,
            document: saved.document,
            saved: saved.saved.clone(),
            container: saved.container.clone(),
            _credits: saved._credits.clone(),
            graph_receiver: None,
            _loan_credits: Some(loan),
            navigation_owner_id: Some(self.id),
        })
    }
}
impl NetworkService {
    /// HTML supplies the actual container Document factory capture and original
    /// registered Reporting capability before author callbacks or retirement.
    pub fn prepare_navigation_embedding_owner<Identity>(
        &mut self,
        document: ReportingDocument,
        receiving: &ReceivingDocumentClient<Identity>,
        container: &EmbedderPolicyContainer,
        parent: &NavigableContext,
        element: NavigationEmbeddingElement,
    ) -> Result<PreparedNavigationEmbeddingOwner, ReportError> {
        if parent.ancestor_origins.is_empty()
            || parent.ancestor_origins.len() > 32
            || parent.ancestor_origins.first() != Some(&parent.top_level_origin)
            || parent.ancestor_origins.last() != Some(&parent.parent_origin)
            || parent.sandbox_flags & !0xffff != 0
            || receiving.origin != parent.parent_origin
            || !receiving
                .context
                .frame_ancestors
                .iter()
                .rev()
                .eq(parent.ancestor_origins[..parent.ancestor_origins.len() - 1].iter())
        {
            return Err(ReportError::InvalidReport);
        }
        let settings = self.prepare_coep_request_reporting(document, receiving, container)?;
        let mut bytes = std::mem::size_of::<NavigationEmbeddingOwnerSnapshot>();
        for origin in parent
            .ancestor_origins
            .iter()
            .chain(std::iter::once(&parent.top_level_origin))
        {
            bytes = bytes
                .checked_add(std::mem::size_of::<Origin>())
                .and_then(|n| n.checked_add(origin.serialize().len()))
                .filter(|n| *n <= MAX_REPORT_BYTES)
                .ok_or(ReportError::Limit)?;
        }
        let credits = graph_credit(&self.reporting.prepared_bytes, bytes)?;
        let generation = self.reporting.next_graph;
        self.reporting.next_graph = generation.checked_add(1).ok_or(ReportError::Limit)?;
        Ok(PreparedNavigationEmbeddingOwner {
            snapshot: Arc::new(NavigationEmbeddingOwnerSnapshot {
                id: NavigationEmbeddingOwnerId {
                    service: self.reporting.service,
                    generation,
                },
                settings: Arc::new(settings),
                ancestors: parent.ancestor_origins.clone(),
                top_level_origin: parent.top_level_origin.clone(),
                sandbox_flags: parent.sandbox_flags,
                element,
                _credits: credits,
            }),
        })
    }
    /// Explicit terminal release of a capture that has not been admitted.
    /// Admitted request clones retain bounded loans until their terminal drop.
    pub fn release_navigation_embedding_owner(
        &mut self,
        plan: PreparedNavigationEmbeddingOwner,
        id: NavigationEmbeddingOwnerId,
    ) -> Result<(), ReportError> {
        if plan.id() != id {
            return Err(ReportError::StaleDocument);
        }
        plan.snapshot.validate_service(self.reporting.service)
    }
}

impl NetworkService {
    /// Admit an already-authentic child request with precise settings/quota
    /// errors before socket admission. Source client metadata stays unchanged.
    pub fn start_navigation_with_embedding_owner(
        &mut self,
        request: crate::api::ApiRequest,
        context: ClientContext,
        now: Instant,
    ) -> Result<crate::ServiceId, CoepRequestStartError> {
        let owner = request
            .embedding_owner
            .as_ref()
            .ok_or(CoepRequestStartError::Reporting(ReportError::InvalidReport))?;
        let settings = owner.loan(self).map_err(CoepRequestStartError::Reporting)?;
        let mut policy = request
            .policy_request()
            .map_err(CoepRequestStartError::Fetch)?;
        policy.security = context.security;
        settings
            .validate(self, &policy)
            .map_err(CoepRequestStartError::Reporting)?;
        self.start_at_inner(request, context, now, Some(settings), &mut None)
            .map_err(CoepRequestStartError::Fetch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ReferrerPolicy,
        api::{ApiRequest, PendingFetchApi},
    };
    use amazing_net::dispatch::{DispatchLimits, Dispatcher};
    fn capture(
        service: &mut NetworkService,
        value: EmbedderPolicy,
    ) -> (
        crate::reporting::ReportingDocument,
        crate::ReceivingDocumentClient<(u64, u64)>,
        EmbedderPolicyContainer,
        NavigableContext,
    ) {
        let url = HttpUrl::parse("https://container.test/document").unwrap();
        let mut context = ClientContext::default();
        context.security.embedder_policy = value;
        context.partition_key = Some(url.origin().site(&crate::psl::PublicSuffixList));
        let receiver = ReceivingDocumentClient {
            identity: (31, 7),
            creation_url: url.as_parsed().clone(),
            document_url: url.as_parsed().clone(),
            base_url: url.as_parsed().clone(),
            origin: url.origin().clone(),
            csp_self_origin: url.origin().clone(),
            context,
            referrer_policy: ReferrerPolicy::NoReferrer,
        };
        // Explicit lower fixture: a fresh native global, no accepted HTTP head
        // and no endpoint registry. This is not an installed Document claim.
        let handle = service
            .register_reporting_initial_client(&receiver)
            .unwrap();
        let container = EmbedderPolicyContainer {
            value,
            reporting_endpoint: None,
            report_only_value: EmbedderPolicy::Credentialless,
            report_only_reporting_endpoint: None,
        };
        let parent = NavigableContext {
            top_level_origin: receiver.origin.clone(),
            parent_origin: receiver.origin.clone(),
            ancestor_origins: vec![receiver.origin.clone()],
            sandbox_flags: 1,
            frame_csp: receiver.context.csp.clone(),
            referrer: String::new(),
            referrer_policy: "no-referrer".into(),
            now_unix: 10,
        };
        (handle, receiver, container, parent)
    }
    #[test]
    fn owner_origin_policy_and_request_source_remain_distinct_across_clone_and_retirement() {
        for value in [
            EmbedderPolicy::UnsafeNone,
            EmbedderPolicy::RequireCorp,
            EmbedderPolicy::Credentialless,
        ] {
            for element in [
                NavigationEmbeddingElement::Frame,
                NavigationEmbeddingElement::Iframe,
            ] {
                let mut service = NetworkService::new(DispatchLimits::default());
                let (handle, receiver, container, parent) = capture(&mut service, value);
                let plan = service
                    .prepare_navigation_embedding_owner(
                        handle, &receiver, &container, &parent, element,
                    )
                    .unwrap();
                assert!(service.forget_reporting_document(handle));
                let target = HttpUrl::parse("https://resource.test/child").unwrap();
                let initiator = target.origin().clone();
                let request = ApiRequest::child_navigation_with_embedding_owner(
                    target, &parent, &initiator, plan,
                )
                .unwrap();
                let cloned = request.clone_request().unwrap();
                let mut policy = cloned.policy_request().unwrap();
                policy.security.embedder_policy = EmbedderPolicy::UnsafeNone;
                assert_eq!(policy.origin, initiator);
                assert_eq!(
                    policy.embedding_owner.as_ref().unwrap().origin(),
                    &receiver.origin
                );
                assert_eq!(
                    policy.embedding_owner.as_ref().unwrap().destination(),
                    element.destination()
                );
                assert_eq!(
                    crate::check_corp_with_credentials(
                        &policy,
                        &HeaderList::default(),
                        true,
                        true,
                        &crate::psl::PublicSuffixList
                    )
                    .is_ok(),
                    value == EmbedderPolicy::UnsafeNone
                );
                let loan = policy
                    .embedding_owner
                    .as_ref()
                    .unwrap()
                    .loan(&service)
                    .unwrap();
                let reports = service
                    .check_request_coep_reporting(
                        &loan,
                        &policy,
                        &HeaderList::default(),
                        true,
                        true,
                        Instant::now(),
                    )
                    .unwrap();
                assert_eq!(
                    reports.len(),
                    if value == EmbedderPolicy::UnsafeNone {
                        1
                    } else {
                        2
                    }
                );
                for report in reports {
                    assert_eq!(report.delivery, Err(ReportError::NoEndpoint));
                }
                drop(loan);
                drop(policy);
                drop(cloned);
                drop(request);
                assert_eq!(service.reporting.prepared_bytes.load(Ordering::Acquire), 0);
            }
        }
    }
    #[test]
    fn owner_service_parent_generation_quota_and_standalone_transport_reject_before_io() {
        let mut service = NetworkService::new(DispatchLimits::default());
        let (handle, receiver, container, parent) =
            capture(&mut service, EmbedderPolicy::RequireCorp);
        let first = service
            .prepare_navigation_embedding_owner(
                handle,
                &receiver,
                &container,
                &parent,
                NavigationEmbeddingElement::Iframe,
            )
            .unwrap();
        let second = service
            .prepare_navigation_embedding_owner(
                handle,
                &receiver,
                &container,
                &parent,
                NavigationEmbeddingElement::Iframe,
            )
            .unwrap();
        let first_id = first.id();
        assert_ne!(first_id, second.id());
        assert_eq!(
            service.release_navigation_embedding_owner(second, first_id),
            Err(ReportError::StaleDocument)
        );
        let target = HttpUrl::parse("https://container.test/child").unwrap();
        let mut request = ApiRequest::child_navigation_with_embedding_owner(
            target.clone(),
            &parent,
            &receiver.origin,
            first,
        )
        .unwrap();
        let mut other = NetworkService::new(DispatchLimits::default());
        assert!(matches!(
            other.start_navigation_with_embedding_owner(
                request.clone_request().unwrap(),
                receiver.context.clone(),
                Instant::now()
            ),
            Err(CoepRequestStartError::Reporting(ReportError::StaleDocument))
        ));
        assert_eq!(other.dispatcher.in_flight(), 0);
        let mut dispatcher = Dispatcher::new(DispatchLimits::default());
        assert!(PendingFetchApi::start(&mut request, &mut dispatcher, Instant::now()).is_err());
        assert_eq!(dispatcher.in_flight(), 0);
        let owner = request.embedding_owner.as_ref().unwrap().clone();
        let mut loans = Vec::new();
        loop {
            match owner.loan(&service) {
                Ok(loan) => loans.push(loan),
                Err(ReportError::Limit) => break,
                Err(error) => panic!("{error:?}"),
            }
        }
        assert!(!loans.is_empty());
        assert!(service.reporting.prepared_bytes.load(Ordering::Acquire) <= MAX_REPORT_BYTES);
        drop(loans);
        drop(owner);
        drop(request);
        assert_eq!(service.reporting.prepared_bytes.load(Ordering::Acquire), 0);
        let plan = service
            .prepare_navigation_embedding_owner(
                handle,
                &receiver,
                &container,
                &parent,
                NavigationEmbeddingElement::Iframe,
            )
            .unwrap();
        let mut wrong = parent.clone();
        wrong.parent_origin = Origin::opaque_with_id(72);
        assert!(
            ApiRequest::child_navigation_with_embedding_owner(
                target,
                &wrong,
                &receiver.origin,
                plan
            )
            .is_err()
        );
        assert_eq!(service.reporting.prepared_bytes.load(Ordering::Acquire), 0);
        assert_eq!(service.dispatcher.in_flight(), 0);
        assert!(service.forget_reporting_document(handle));
        assert!(matches!(
            service.prepare_navigation_embedding_owner(
                handle,
                &receiver,
                &container,
                &parent,
                NavigationEmbeddingElement::Iframe
            ),
            Err(ReportError::StaleDocument)
        ));
    }
    #[test]
    fn actual_receiving_parent_to_top_ancestry_validates_against_top_to_parent_navigation() {
        for depth in [1usize, 2, 3, 8, 32] {
            let mut service = NetworkService::new(DispatchLimits::default());
            let (old, mut receiver, container, mut parent) =
                capture(&mut service, EmbedderPolicy::Credentialless);
            assert!(service.forget_reporting_document(old));
            let mut ancestors = (0..depth - 1)
                .map(|i| Origin::opaque_with_id(1000 + i as u128))
                .collect::<Vec<_>>();
            receiver.context.frame_ancestors = ancestors.iter().rev().cloned().collect();
            ancestors.push(receiver.origin.clone());
            parent.top_level_origin = ancestors[0].clone();
            parent.ancestor_origins = ancestors;
            let original = receiver.context.frame_ancestors.clone();
            let handle = service
                .register_reporting_initial_client(&receiver)
                .unwrap();
            let plan = service
                .prepare_navigation_embedding_owner(
                    handle,
                    &receiver,
                    &container,
                    &parent,
                    NavigationEmbeddingElement::Iframe,
                )
                .unwrap();
            assert_eq!(
                receiver.context.frame_ancestors, original,
                "factory custody remains unchanged"
            );
            let id = plan.id();
            service
                .release_navigation_embedding_owner(plan, id)
                .unwrap();
            assert_eq!(service.reporting.prepared_bytes.load(Ordering::Acquire), 0);
            if depth >= 3 {
                receiver.context.frame_ancestors.reverse();
                assert!(matches!(
                    service.prepare_navigation_embedding_owner(
                        handle,
                        &receiver,
                        &container,
                        &parent,
                        NavigationEmbeddingElement::Iframe
                    ),
                    Err(ReportError::InvalidReport)
                ));
                assert_eq!(service.dispatcher.in_flight(), 0);
                assert_eq!(service.reporting.prepared_bytes.load(Ordering::Acquire), 0);
            }
        }
    }

    #[test]
    fn actual_navigation_peer_checks_embedding_owner_on_final_and_location_heads() {
        use std::{
            io::{Read, Write},
            net::TcpListener,
            thread,
            time::Duration,
        };
        for value in [
            EmbedderPolicy::UnsafeNone,
            EmbedderPolicy::RequireCorp,
            EmbedderPolicy::Credentialless,
        ] {
            for status in [200, 302] {
                for streamed in [false, true] {
                    for corp in [None, Some("cross-origin"), Some("same-origin")] {
                        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
                        let port = listener.local_addr().unwrap().port();
                        let target =
                            HttpUrl::parse(&format!("http://127.0.0.1:{port}/child")).unwrap();
                        let peer = thread::spawn(move || {
                            let (mut socket, _) = listener.accept().unwrap();
                            socket
                                .set_read_timeout(Some(Duration::from_secs(15)))
                                .unwrap();
                            let mut head = Vec::new();
                            while !head.ends_with(b"\r\n\r\n") {
                                let mut b = [0];
                                socket.read_exact(&mut b).unwrap();
                                head.push(b[0]);
                                assert!(head.len() < 65536);
                            }
                            let head = String::from_utf8(head).unwrap();
                            assert!(head.starts_with("GET /child HTTP/1.1\r\n"));
                            let mut response = format!(
                                "HTTP/1.1 {status} Test\r\nContent-Length: 0\r\nConnection: close\r\n"
                            );
                            if status == 302 {
                                response.push_str("Location: /next\r\n");
                            }
                            if let Some(corp) = corp {
                                response
                                    .push_str(&format!("Cross-Origin-Resource-Policy: {corp}\r\n"));
                            }
                            response.push_str("\r\n");
                            socket.write_all(response.as_bytes()).unwrap();
                            head
                        });
                        let mut service = NetworkService::new(DispatchLimits::default());
                        let (handle, mut receiver, container, mut parent) =
                            capture(&mut service, value);
                        // An explicit fresh receiving-global fixture with a separate
                        // native HTTP origin. No accepted response is fabricated.
                        assert!(service.forget_reporting_document(handle));
                        let owner_url = HttpUrl::parse("http://container.test/document").unwrap();
                        receiver.creation_url = owner_url.as_parsed().clone();
                        receiver.document_url = owner_url.as_parsed().clone();
                        receiver.base_url = owner_url.as_parsed().clone();
                        receiver.origin = owner_url.origin().clone();
                        receiver.csp_self_origin = receiver.origin.clone();
                        parent.parent_origin = receiver.origin.clone();
                        parent.top_level_origin = receiver.origin.clone();
                        parent.ancestor_origins = vec![receiver.origin.clone()];
                        let handle = service
                            .register_reporting_initial_client(&receiver)
                            .unwrap();
                        let plan = service
                            .prepare_navigation_embedding_owner(
                                handle,
                                &receiver,
                                &container,
                                &parent,
                                NavigationEmbeddingElement::Iframe,
                            )
                            .unwrap();
                        assert!(service.forget_reporting_document(handle));
                        let request = ApiRequest::child_navigation_with_embedding_owner(
                            target.clone(),
                            &parent,
                            target.origin(),
                            plan,
                        )
                        .unwrap();
                        let mut source = receiver.context.clone();
                        source.security.embedder_policy = EmbedderPolicy::UnsafeNone;
                        let id = if streamed {
                            service
                                .start_navigation_stream_with_embedding_owner(
                                    request,
                                    source,
                                    0,
                                    Instant::now(),
                                )
                                .unwrap()
                        } else {
                            service
                                .start_navigation_with_embedding_owner(
                                    request,
                                    source,
                                    Instant::now(),
                                )
                                .unwrap()
                        };
                        // Fetch CORP internal step1 exempts unsafe-none nested
                        // navigations even from explicit CORP denial. RO still runs
                        // with its own Credentialless value at wrapper step4.
                        let blocked =
                            value != EmbedderPolicy::UnsafeNone && corp != Some("cross-origin");
                        let expected_reports = if corp != Some("cross-origin") {
                            if value == EmbedderPolicy::UnsafeNone {
                                1
                            } else {
                                2
                            }
                        } else {
                            0
                        };
                        let mut reports = 0;
                        let mut heads = 0;
                        let mut finished = false;
                        let deadline = Instant::now() + Duration::from_secs(15);
                        while !finished && Instant::now() < deadline {
                            for event in service.step(97, Instant::now()) {
                                match event {
                                    crate::ServiceEvent::CoepReport {
                                        id: found,
                                        document,
                                        report,
                                    } => {
                                        assert_eq!(found, id);
                                        assert_eq!(document, handle);
                                        assert_eq!(
                                            report.unwrap().delivery,
                                            Err(ReportError::NoEndpoint)
                                        );
                                        reports += 1;
                                    }
                                    crate::ServiceEvent::ResponseHead { id: found, .. } => {
                                        assert!(streamed && !blocked);
                                        assert_eq!(found, id);
                                        heads += 1;
                                    }
                                    crate::ServiceEvent::StreamFinished { id: found } => {
                                        assert!(streamed && !blocked);
                                        assert_eq!(found, id);
                                        finished = true;
                                    }
                                    crate::ServiceEvent::StreamFailed { id: found, error } => {
                                        assert!(streamed && blocked);
                                        assert_eq!(found, id);
                                        assert_eq!(
                                            error,
                                            crate::api::ApiError::Network(
                                                FetchError::CrossOriginResourcePolicy
                                            )
                                        );
                                        finished = true;
                                    }
                                    crate::ServiceEvent::Finished { id: found, result } => {
                                        assert!(!streamed);
                                        assert_eq!(found, id);
                                        if blocked {
                                            assert_eq!(
                                                result.unwrap_err(),
                                                crate::api::ApiError::Network(
                                                    FetchError::CrossOriginResourcePolicy
                                                )
                                            );
                                        } else {
                                            assert!(result.is_ok());
                                        }
                                        finished = true;
                                    }
                                    _ => {}
                                }
                            }
                            thread::yield_now();
                        }
                        assert!(finished);
                        assert_eq!(reports, expected_reports);
                        assert_eq!(heads, usize::from(streamed && !blocked));
                        assert!(peer.join().unwrap().starts_with("GET /child"));
                        assert_eq!(service.dispatcher.in_flight(), 0);
                        assert_eq!(service.reporting.prepared_bytes.load(Ordering::Acquire), 0);
                    }
                }
            }
        }
    }

    #[test]
    fn actual_owner_response_registration_retirement_and_coep_post_completion() {
        use std::{
            io::{Read, Write},
            net::TcpListener,
            thread,
            time::Duration,
        };
        for status in [200, 302] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            let owner_url = HttpUrl::parse(&format!("http://localhost:{port}/document")).unwrap();
            let child_url = HttpUrl::parse(&format!("http://127.0.0.1:{port}/child")).unwrap();
            let peer = thread::spawn(move || {
                let mut posts = Vec::new();
                for index in 0..4 {
                    let (mut socket, _) = listener.accept().unwrap();
                    socket
                        .set_read_timeout(Some(Duration::from_secs(15)))
                        .unwrap();
                    let mut head = Vec::new();
                    while !head.ends_with(b"\r\n\r\n") {
                        let mut b = [0];
                        socket.read_exact(&mut b).unwrap();
                        head.push(b[0]);
                        assert!(head.len() < 65536);
                    }
                    let head = String::from_utf8(head).unwrap();
                    if index == 0 {
                        assert!(head.starts_with("GET /document HTTP/1.1\r\n"));
                        socket.write_all(b"HTTP/1.1 200 OK\r\nReporting-Endpoints: coep=\"/reports\"\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
                    } else if index == 1 {
                        assert!(head.starts_with("GET /child HTTP/1.1\r\n"));
                        let location = if status == 302 {
                            "Location: /next\r\n"
                        } else {
                            ""
                        };
                        socket.write_all(format!("HTTP/1.1 {status} Test\r\n{location}Content-Length: 0\r\nConnection: close\r\n\r\n").as_bytes()).unwrap();
                    } else {
                        assert!(head.starts_with("POST /reports HTTP/1.1\r\n"));
                        assert!(
                            head.to_ascii_lowercase()
                                .contains("content-type: application/reports+json")
                        );
                        let n: usize = head
                            .lines()
                            .find_map(|line| {
                                line.split_once(':')
                                    .filter(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                                    .map(|(_, v)| v.trim().parse().unwrap())
                            })
                            .unwrap();
                        let mut body = vec![0; n];
                        socket.read_exact(&mut body).unwrap();
                        posts.push(String::from_utf8(body).unwrap());
                        socket.write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
                    }
                }
                posts
            });
            let mut service = NetworkService::new(DispatchLimits::default());
            let (old, mut receiver, mut container, mut parent) =
                capture(&mut service, EmbedderPolicy::RequireCorp);
            assert!(service.forget_reporting_document(old));
            receiver.creation_url = owner_url.as_parsed().clone();
            receiver.document_url = owner_url.as_parsed().clone();
            receiver.base_url = owner_url.as_parsed().clone();
            receiver.origin = owner_url.origin().clone();
            receiver.csp_self_origin = receiver.origin.clone();
            parent.parent_origin = receiver.origin.clone();
            parent.top_level_origin = receiver.origin.clone();
            parent.ancestor_origins = vec![receiver.origin.clone()];
            let document_request = ApiRequest::navigation(
                owner_url.clone(),
                crate::api::NavigationRequestInit {
                    method: "GET".into(),
                    body: Vec::new(),
                    referrer: String::new(),
                    referrer_policy: "no-referrer".into(),
                    initiator_origin: receiver.origin.clone(),
                    reload_or_history: false,
                    client_reserved_id: None,
                },
            )
            .unwrap();
            let id = service
                .start_at(document_request, receiver.context.clone(), Instant::now())
                .unwrap();
            let mut accepted = None;
            let deadline = Instant::now() + Duration::from_secs(15);
            while accepted.is_none() && Instant::now() < deadline {
                for event in service.step(97, Instant::now()) {
                    if let crate::ServiceEvent::Finished { id: found, result } = event {
                        assert_eq!(found, id);
                        accepted = Some(result.unwrap());
                    }
                }
                thread::yield_now();
            }
            let accepted = accepted.expect("actual owner response");
            assert_eq!(accepted.status, 200);
            let mut fields = HeaderList::default();
            fields
                .append(
                    "Reporting-Endpoints",
                    &accepted
                        .headers
                        .get_string("reporting-endpoints")
                        .unwrap()
                        .expect("actual accepted response header"),
                )
                .unwrap();
            let handle = service
                .register_reporting_client_response(&receiver, &owner_url, &fields)
                .unwrap();
            container.reporting_endpoint = Some("coep".into());
            container.report_only_reporting_endpoint = Some("coep".into());
            let plan = service
                .prepare_navigation_embedding_owner(
                    handle,
                    &receiver,
                    &container,
                    &parent,
                    NavigationEmbeddingElement::Iframe,
                )
                .unwrap();
            assert!(service.forget_reporting_document(handle));
            let request = ApiRequest::child_navigation_with_embedding_owner(
                child_url.clone(),
                &parent,
                child_url.origin(),
                plan,
            )
            .unwrap();
            let mut source = receiver.context.clone();
            source.security.embedder_policy = EmbedderPolicy::UnsafeNone;
            let id = service
                .start_navigation_with_embedding_owner(request, source, Instant::now())
                .unwrap();
            let mut finished = false;
            let mut snapshots = 0;
            let mut completions = 0;
            let deadline = Instant::now() + Duration::from_secs(15);
            while (!finished || completions < 2) && Instant::now() < deadline {
                for event in service.step(97, Instant::now()) {
                    match event {
                        crate::ServiceEvent::Finished { id: found, result } => {
                            assert_eq!(found, id);
                            assert_eq!(
                                result.unwrap_err(),
                                crate::api::ApiError::Network(
                                    FetchError::CrossOriginResourcePolicy
                                )
                            );
                            finished = true;
                        }
                        crate::ServiceEvent::CoepReport {
                            id: found,
                            document,
                            report,
                        } => {
                            assert_eq!(found, id);
                            assert_eq!(document, handle);
                            let snapshot = report.unwrap();
                            assert_eq!(
                                snapshot.report_url,
                                owner_url.serialize().encode_utf16().collect::<Vec<_>>()
                            );
                            assert!(snapshot.delivery.is_ok());
                            snapshots += 1;
                        }
                        crate::ServiceEvent::ReportFinished(report) => {
                            assert_eq!(report.document, handle);
                            assert!(matches!(
                                report.outcome,
                                ReportOutcome::Response {
                                    status: 204,
                                    accepted: true
                                }
                            ));
                            completions += 1;
                        }
                        _ => {}
                    }
                }
                thread::yield_now();
            }
            assert!(finished);
            assert_eq!(snapshots, 2);
            assert_eq!(completions, 2);
            let posts = peer.join().unwrap();
            assert_eq!(posts.len(), 2);
            for body in posts {
                assert!(body.contains(&owner_url.serialize()));
                assert!(body.contains("iframe"));
                assert!(body.contains("coep"));
            }
            assert_eq!(service.reporting.prepared_bytes.load(Ordering::Acquire), 0);
            assert!(
                !service.forget_reporting_document(handle),
                "no original global was revived"
            );
        }
    }
}
