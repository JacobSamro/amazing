//! Same-network-service receiving globals and move-only embedding owner plans.
//! ReportingDocument and accepted request/head authority never cross the pipe.
use super::receiving_capture::{decode_client, decode_coep, decode_parent};
use super::*;
use amazing_fetch::reporting::{
    NavigationEmbeddingElement, PreparedNavigationEmbeddingOwner, ReportingDocument,
};
use amazing_fetch::{EmbedderPolicyContainer, ReceivingDocumentClient};

const MAX_GLOBALS: usize = 64;
const MAX_OWNER_PLANS: usize = 32;
pub(super) const MAX_ACCEPTED_NAVIGATIONS: usize = 64;

pub(super) struct RetainedNavigation {
    pub request: ApiRequest,
    pub context: ClientContext,
    pub protected: Option<super::receiving_csp::ProtectedSource>,
    pub client_id: u64,
    pub epoch: u64,
    pub head: Option<ApiResponseHead>,
    pub frame: Option<ReceivingFrameIdentity>,
    pub source: Option<RegisteredReceivingGlobal>,
    pub continuation_used: bool,
}
struct RetainedReportBatch {
    receipt: ReceivingReportBatchReceipt,
    events: Vec<ReceivingReportEvent>,
    bytes: usize,
}
struct ReportConsumer {
    native: Option<amazing_fetch::reporting::ReportingTaskConsumer>,
    receipt: RegisteredReceivingReportConsumer,
    closed: bool,
    batch: Option<RetainedReportBatch>,
    last_ack: Option<u64>,
}
struct RegisteredGlobal {
    receipt: RegisteredReceivingGlobal,
    client: ReceivingDocumentClient<ReceivingGlobalIdentity>,
    coep: EmbedderPolicyContainer,
    reporting: ReportingDocument,
    response: super::receiving_csp::ProtectedResponse,
    retired: bool,
    reports: VecDeque<ReceivingReportEvent>,
    report_overflow: bool,
    report_bytes: usize,
    consumer: Option<ReportConsumer>,
}
struct OwnerPlan {
    receipt: PreparedReceivingOwner,
    parent: NavigableContext,
    native: PreparedNavigationEmbeddingOwner,
}
#[derive(Default)]
pub(super) struct ReceivingRegistry {
    next: u64,
    globals: BTreeMap<u64, RegisteredGlobal>,
    plans: BTreeMap<u64, OwnerPlan>,
}
impl ReceivingRegistry {
    fn next_id(&mut self) -> Result<u64, Error> {
        self.next = self
            .next
            .checked_add(1)
            .ok_or_else(|| invalid("receiving identity exhausted"))?;
        Ok(self.next)
    }
    fn global(&self, receipt: &RegisteredReceivingGlobal) -> Result<&RegisteredGlobal, Error> {
        let saved = self
            .globals
            .get(&receipt.id)
            .ok_or_else(|| invalid("unknown receiving global"))?;
        if &saved.receipt != receipt || saved.retired {
            return Err(invalid("stale receiving global"));
        }
        Ok(saved)
    }
    pub fn register(
        &mut self,
        service: &mut NetworkService,
        heads: &BTreeMap<u64, RetainedNavigation>,
        epochs: &BTreeMap<u64, u64>,
        wire: RegisterReceivingGlobal,
    ) -> Result<RegisteredReceivingGlobal, Error> {
        if wire.client_id == 0
            || wire.epoch == 0
            || epochs
                .get(&wire.client_id)
                .is_some_and(|current| wire.epoch < *current)
        {
            return Err(invalid("receiving registration epoch"));
        }
        let client = decode_client(wire.capture)?;
        let coep = decode_coep(wire.coep)?;
        if client.context.security.embedder_policy != coep.value {
            return Err(invalid("receiving COEP context mismatch"));
        }
        if let Some(saved) = self.globals.values_mut().find(|saved| {
            saved.receipt.client_id == wire.client_id && saved.receipt.identity == client.identity
        }) {
            if saved.retired
                || saved.client.creation_url != client.creation_url
                || saved.client.origin != client.origin
                || saved.client.csp_self_origin != client.csp_self_origin
                || saved.coep != coep
            {
                return Err(invalid(
                    "receiving global resurrection or policy replacement",
                ));
            }
            // History reuse retains its original Reporting settings. Current URL,
            // base URL and delivered Meta policy remain the actual browser capture.
            saved.client = client;
            return Ok(saved.receipt.clone());
        }
        if self.globals.len() >= MAX_GLOBALS {
            return Err(invalid("receiving global quota"));
        }
        let (reporting, response) = match wire.admission {
            ReceivingAdmission::FreshInitial(()) => {
                if !matches!(
                    client.creation_url.serialize().split(['?', '#']).next(),
                    Some("about:blank" | "about:srcdoc")
                ) {
                    return Err(invalid("fresh receiving creation boundary"));
                }
                (
                    service.register_reporting_initial_client(&client),
                    super::receiving_csp::ProtectedResponse::Initial,
                )
            }
            ReceivingAdmission::AcceptedResponse(id) => {
                let saved = heads
                    .get(&id)
                    .ok_or_else(|| invalid("unknown accepted navigation"))?;
                if (saved.client_id, saved.epoch) != (wire.client_id, wire.epoch) {
                    return Err(invalid("accepted navigation ownership"));
                }
                let head = saved
                    .head
                    .as_ref()
                    .ok_or_else(|| invalid("navigation head not admitted"))?;
                let accepted = head
                    .navigation
                    .as_ref()
                    .ok_or_else(|| invalid("missing native navigation admission"))?;
                if !(200..=299).contains(&accepted.status)
                    || client.creation_url.serialize_without_fragment() != accepted.final_url
                {
                    return Err(invalid("receiving response creation mismatch"));
                }
                let fields = accepted.headers.iter().cloned().collect::<Vec<_>>();
                let mut headers = HeaderList::default();
                for (name, value) in fields {
                    headers
                        .append(&name, &value)
                        .map_err(|_| invalid("accepted receiving headers"))?;
                }
                if EmbedderPolicyContainer::from_response(&headers, wire.secure_context) != coep {
                    return Err(invalid("receiving response COEP mismatch"));
                }
                let url = HttpUrl::parse(&accepted.final_url)
                    .map_err(|_| invalid("accepted receiving URL"))?;
                (
                    service.register_reporting_client_response(&client, &url, &headers),
                    super::receiving_csp::ProtectedResponse::Received {
                        status: accepted.status,
                        referrer: accepted.referrer.clone(),
                    },
                )
            }
        };
        let reporting =
            reporting.map_err(|e| invalid(&format!("receiving reporting registration: {e:?}")))?;
        let id = self.next_id()?;
        let receipt = RegisteredReceivingGlobal {
            session: wire.session,
            id,
            client_id: wire.client_id,
            epoch: wire.epoch,
            identity: client.identity.clone(),
        };
        self.globals.insert(
            id,
            RegisteredGlobal {
                receipt: receipt.clone(),
                client,
                coep,
                reporting,
                response,
                retired: false,
                reports: VecDeque::new(),
                report_overflow: false,
                report_bytes: 0,
                consumer: None,
            },
        );
        Ok(receipt)
    }
    pub fn prepare(
        &mut self,
        service: &mut NetworkService,
        wire: PrepareReceivingOwner,
    ) -> Result<PreparedReceivingOwner, Error> {
        if wire.frame.frame == 0
            || wire.frame.generation == 0
            || self.plans.len() >= MAX_OWNER_PLANS
        {
            return Err(invalid("receiving owner frame or quota"));
        }
        let saved = self.global(&wire.owner)?;
        let parent = decode_parent(wire.parent)?;
        let element = match wire.element.as_str() {
            "frame" => NavigationEmbeddingElement::Frame,
            "iframe" => NavigationEmbeddingElement::Iframe,
            _ => return Err(invalid("receiving embedding element")),
        };
        let native = service
            .prepare_navigation_embedding_owner(
                saved.reporting,
                &saved.client,
                &saved.coep,
                &parent,
                element,
            )
            .map_err(|e| invalid(&format!("receiving owner admission: {e:?}")))?;
        let id = self.next_id()?;
        let receipt = PreparedReceivingOwner {
            id,
            owner: wire.owner,
            frame: wire.frame,
        };
        self.plans.insert(
            id,
            OwnerPlan {
                receipt: receipt.clone(),
                parent,
                native,
            },
        );
        Ok(receipt)
    }
    pub fn take_navigation(
        &mut self,
        wire: &StartReceivingNavigation,
    ) -> Result<(ApiRequest, super::receiving_csp::ProtectedSource), Error> {
        if wire.source.client_id != wire.client_id || wire.client_id == 0 || wire.epoch == 0 {
            return Err(invalid("navigation source client"));
        }
        let registered = self.global(&wire.source)?;
        let source = registered.client.clone();
        let protected = super::receiving_csp::ProtectedSource::navigation(
            wire.source.clone(),
            source.clone(),
            registered.response.clone(),
            registered.reporting,
        );
        let saved = self
            .plans
            .get(&wire.owner.id)
            .ok_or_else(|| invalid("unknown receiving owner plan"))?;
        if saved.receipt != wire.owner {
            return Err(invalid("receiving owner plan mismatch"));
        }
        let target =
            HttpUrl::parse(&wire.target).map_err(|_| invalid("receiving navigation target"))?;
        let plan = self.plans.remove(&wire.owner.id).unwrap();
        let mut request = ApiRequest::child_navigation_with_embedding_owner(
            target,
            &plan.parent,
            &source.origin,
            plan.native,
        )
        .map_err(|e| invalid(&format!("receiving child navigation: {e:?}")))?;
        request
            .apply_receiving_csp_client(&source)
            .map_err(|_| invalid("navigation source CSP origin"))?;
        // The incumbent source is independent of the embedding parent's
        // settings. Its real document/referrer policy supplies referrer custody.
        request.referrer = if matches!(source.document_url.scheme(), "http" | "https") {
            source.document_url.serialize()
        } else {
            String::new()
        };
        request.referrer_policy = source.referrer_policy;
        request
            .set_response_body_limit(u64::from(wire.response_limit))
            .map_err(|_| invalid("navigation response limit"))?;
        Ok((request, protected))
    }
    pub fn validate_source(&self, receipt: &RegisteredReceivingGlobal) -> Result<(), Error> {
        self.global(receipt).map(|_| ())
    }
    pub fn retire(
        &mut self,
        service: &mut NetworkService,
        receipt: RegisteredReceivingGlobal,
    ) -> Result<bool, Error> {
        let saved = self
            .globals
            .get_mut(&receipt.id)
            .ok_or_else(|| invalid("unknown receiving global"))?;
        if saved.receipt != receipt {
            return Err(invalid("receiving retirement ownership"));
        }
        if saved.retired {
            return Ok(false);
        }
        saved.retired = true;
        // Existing move-only plans and admitted request loans retain their native
        // saved settings; retiring this registration cannot revive or rewrite it.
        Ok(service.forget_reporting_document(saved.reporting))
    }
    pub fn observer_report(
        &mut self,
        id: ServiceId,
        document: ReportingDocument,
        report: Result<
            amazing_fetch::reporting::NativeReportSnapshot,
            amazing_fetch::reporting::ReportError,
        >,
    ) {
        let event = match report {
            Err(error) => ReceivingReportEvent::CreationError(format!("{error:?}")),
            Ok(report) => {
                let (delivery_id, delivery_error) = match report.delivery {
                    Ok(id) => (Some(id.0), None),
                    Err(error) => (None, Some(format!("{error:?}"))),
                };
                ReceivingReportEvent::Observer(ReceivingObserverReport {
                    request_id: id.0,
                    report_type: report.report_type,
                    report_url: report.report_url,
                    body: report
                        .body
                        .into_iter()
                        .map(|(name, value)| ReceivingReportField { name, value })
                        .collect(),
                    delivery_id,
                    delivery_error,
                })
            }
        };
        self.queue_report(document, event);
    }
    pub fn report_finished(&mut self, report: amazing_fetch::reporting::ReportFinished) {
        use amazing_fetch::reporting::{ReportOutcome, ReportRoute};
        // Read the genuine receipt before moving its independent outcome.
        let native = report.wire();
        let wire = ReceivingReportWire {
            version: 1,
            report_url: native.report_url().to_owned(),
            owner_origin: native.owner_origin().serialize(),
            request_origin: native.request_origin().serialize(),
            registered_endpoint: native.registered_endpoint().serialize(),
            url_list: native.url_list().iter().map(HttpUrl::serialize).collect(),
            terminal_url: native.terminal_url().serialize(),
            terminal_is_preflight: native.terminal_is_preflight(),
            terminal_includes_credentials: native.terminal_includes_credentials(),
        };
        let (status, accepted, error) = match report.outcome {
            ReportOutcome::Response { status, accepted } => (Some(status), accepted, None),
            ReportOutcome::Failed(error) => (None, false, Some(format!("{error:?}"))),
        };
        self.queue_report(
            report.document,
            ReceivingReportEvent::Completion(ReceivingReportCompletion {
                delivery_id: report.id.0,
                route: match report.route {
                    ReportRoute::LegacyCsp => "legacy-csp",
                    ReportRoute::ReportingApi => "reporting-api",
                }
                .into(),
                status,
                accepted,
                error,
                wire,
            }),
        );
    }
    pub(super) fn queue_report(
        &mut self,
        document: ReportingDocument,
        event: ReceivingReportEvent,
    ) {
        if let Some(saved) = self
            .globals
            .values_mut()
            .find(|saved| saved.reporting == document)
        {
            let bytes = match &event {
                ReceivingReportEvent::Observer(report) => {
                    2 * (report.report_type.len()
                        + report.report_url.len()
                        + report
                            .body
                            .iter()
                            .map(|field| field.name.len() + field.value.len())
                            .sum::<usize>())
                        + report.delivery_error.as_ref().map_or(0, String::len)
                        + 128
                }
                ReceivingReportEvent::Completion(report) => {
                    completion_bytes(report)
                }
                ReceivingReportEvent::CreationError(error) => error.len() + 32,
                ReceivingReportEvent::Csp(report) => super::receiving_csp::snapshot_bytes(report),
                ReceivingReportEvent::NavigationFailed(failure) => {
                    failure.error.len()
                        + failure.url_list.iter().map(String::len).sum::<usize>()
                        + 256
                }
            };
            let batch = saved.consumer.as_ref().and_then(|consumer| consumer.batch.as_ref());
            let held_count = batch.map_or(0, |batch| batch.events.len());
            let held_bytes = batch.map_or(0, |batch| batch.bytes);
            if saved.reports.len() + held_count >= 256
                || saved
                    .report_bytes
                    .checked_add(held_bytes)
                    .and_then(|total| total.checked_add(bytes))
                    .is_none_or(|n| n > 512 * 1024)
            {
                saved.report_overflow = true;
            } else {
                saved.report_bytes += bytes;
                saved.reports.push_back(event);
            }
        }
    }
    pub fn poll_reports(
        &mut self,
        receipt: RegisteredReceivingGlobal,
    ) -> Result<Vec<ReceivingReportEvent>, Error> {
        let saved = self
            .globals
            .get_mut(&receipt.id)
            .ok_or_else(|| invalid("unknown report receiving global"))?;
        if saved.receipt != receipt {
            return Err(invalid("receiving report ownership"));
        }
        if saved.consumer.is_some() {
            return Err(invalid("receiving scoped report consumer required"));
        }
        if saved.report_overflow {
            return Err(invalid("receiving report queue quota"));
        }
        // A retired global's existing saved observer and endpoint-delivery facts
        // remain consumable; this never admits a new owner/global capability.
        saved.report_bytes = 0;
        Ok(saved.reports.drain(..).collect())
    }
    /// Capture exactly one task-consumer lease while the real global is live.
    pub fn prepare_report_consumer(&mut self, service: &mut NetworkService, global: RegisteredReceivingGlobal)
        -> Result<RegisteredReceivingReportConsumer, Error>
    {
        if self.global(&global)?.consumer.is_some() {
            return Err(invalid("receiving report consumer already captured"));
        }
        let document = self.global(&global)?.reporting;
        let native = service.prepare_reporting_task_consumer(document)
            .map_err(|error| invalid(&format!("native original task consumer: {error:?}")))?;
        let receipt = RegisteredReceivingReportConsumer { id: self.next_id()?, global };
        self.globals.get_mut(&receipt.global.id).unwrap().consumer = Some(ReportConsumer {
            native: Some(native), receipt: receipt.clone(), closed: false, batch: None, last_ack: None,
        });
        Ok(receipt)
    }
    fn consumer_global(&mut self, receipt: &RegisteredReceivingReportConsumer)
        -> Result<&mut RegisteredGlobal, Error>
    {
        let saved = self.globals.get_mut(&receipt.global.id)
            .ok_or_else(|| invalid("unknown receiving report consumer global"))?;
        if saved.receipt != receipt.global
            || saved.consumer.as_ref().is_none_or(|consumer| consumer.receipt != *receipt)
        { return Err(invalid("receiving report consumer ownership")); }
        Ok(saved)
    }
    pub fn poll_report_batch(&mut self, receipt: RegisteredReceivingReportConsumer)
        -> Result<Option<ReceivingReportBatch>, Error>
    {
        {
            let saved = self.consumer_global(&receipt)?;
            if saved.report_overflow { return Err(invalid("receiving report queue quota")); }
            if let Some(batch) = &saved.consumer.as_ref().unwrap().batch {
                return Ok(Some(ReceivingReportBatch { receipt: batch.receipt.clone(), events: batch.events.clone() }));
            }
            if saved.reports.is_empty() { return Ok(None); }
        }
        let id = self.next_id()?;
        let saved = self.consumer_global(&receipt)?;
        let events = saved.reports.drain(..).collect::<Vec<_>>();
        let bytes = std::mem::replace(&mut saved.report_bytes, 0);
        let receipt = ReceivingReportBatchReceipt { consumer: receipt, batch: id };
        saved.consumer.as_mut().unwrap().batch = Some(RetainedReportBatch {
            receipt: receipt.clone(), events: events.clone(), bytes,
        });
        Ok(Some(ReceivingReportBatch { receipt, events }))
    }
    pub fn acknowledge_report_batch(&mut self, receipt: ReceivingReportBatchReceipt)
        -> Result<bool, Error>
    {
        let consumer = self.consumer_global(&receipt.consumer)?.consumer.as_mut().unwrap();
        // A lost ACK response can retry the exact batch without redispatching
        // its already processed author/terminal tasks or returning new credit.
        if consumer.last_ack == Some(receipt.batch) { return Ok(false); }
        if consumer.batch.as_ref().is_none_or(|batch| batch.receipt != receipt) {
            return Err(invalid("stale or foreign receiving report batch"));
        }
        consumer.batch = None;
        consumer.last_ack = Some(receipt.batch);
        Ok(true)
    }
    pub fn close_report_consumer(&mut self, service: &mut NetworkService, receipt: RegisteredReceivingReportConsumer)
        -> Result<bool, Error>
    {
        let saved = self.consumer_global(&receipt)?;
        if !saved.retired { return Err(invalid("receiving consumer task scope still live")); }
        if saved.report_overflow || !saved.reports.is_empty()
            || saved.consumer.as_ref().unwrap().batch.is_some()
        { return Err(invalid("receiving original report tasks not drained")); }
        let consumer = saved.consumer.as_mut().unwrap();
        if consumer.closed { return Ok(false); }
        service.close_reporting_task_consumer(&mut consumer.native)
            .map_err(|error| invalid(&format!("native task consumer closure: {error:?}")))?;
        consumer.closed = true;
        Ok(true)
    }
    pub fn release_retired_global(&mut self, service: &mut NetworkService,
        receipt: RegisteredReceivingReportConsumer) -> Result<ReceivingGlobalRelease, Error>
    {
        let saved = self.consumer_global(&receipt)?;
        if !saved.retired || !saved.consumer.as_ref().unwrap().closed {
            return Err(invalid("receiving original task consumer not retired and closed"));
        }
        if saved.report_overflow || !saved.reports.is_empty()
            || saved.consumer.as_ref().unwrap().batch.is_some()
        { return Ok(ReceivingGlobalRelease::Pending(())); }
        let released = service.release_retired_reporting_document(saved.reporting)
            .map_err(|error| invalid(&format!("native receiving release: {error:?}")))?;
        if released != amazing_fetch::reporting::ReportingRelease::Released {
            return Ok(ReceivingGlobalRelease::Pending(()));
        }
        self.globals.remove(&receipt.global.id);
        Ok(ReceivingGlobalRelease::Released(()))
    }
    pub fn release(&mut self, receipt: PreparedReceivingOwner) -> Result<bool, Error> {
        let Some(plan) = self.plans.get(&receipt.id) else {
            return Ok(false);
        };
        if plan.receipt != receipt {
            return Err(invalid("receiving release ownership"));
        }
        self.plans.remove(&receipt.id);
        Ok(true)
    }
}

// Charge retained observation strings, not provider allocation totals. The
// queue enforces this estimate alongside its original 256-event bound.
fn completion_bytes(report: &ReceivingReportCompletion) -> usize {
    report.error.as_ref().map_or(0, String::len)
        + report.route.len()
        + report.wire.report_url.len()
        + report.wire.owner_origin.len()
        + report.wire.request_origin.len()
        + report.wire.registered_endpoint.len()
        + report.wire.terminal_url.len()
        + report.wire.url_list.iter().map(String::len).sum::<usize>()
        + 256
}
#[cfg(test)]
mod completion_accounting_tests {
    use super::*;
    #[test]
    fn every_wire_observation_is_charged_including_rejected_hop_lineage() {
        // A codec observation fixture cannot mint a native ReportingDocument,
        // receiving client, or ReportFinished. This tests queue accounting.
        let mut report = ReceivingReportCompletion {
            delivery_id: 1, route: "reporting-api".into(), status: None,
            accepted: false, error: Some("Transport".into()),
            wire: ReceivingReportWire {
                version: 1, report_url: "original".into(), owner_origin: "null".into(),
                request_origin: "origin".into(), registered_endpoint: "endpoint".into(),
                url_list: vec!["admitted".into(), "rejected".into()], terminal_url: "admitted".into(),
                terminal_is_preflight: false, terminal_includes_credentials: false,
            },
        };
        let original = completion_bytes(&report);
        let expected = 256 + 13 + 9 + 8 + 4 + 6 + 8 + 8 + 8 + 8;
        assert_eq!(original, expected);
        // Added actual URL observations must consume queue credit; the old
        // error-only accounting would accept this single oversized event.
        report.wire.url_list.push("x".repeat(512*1024));
        assert!(completion_bytes(&report) > 512*1024);
        report.wire.url_list.pop();
        assert_eq!(completion_bytes(&report), original);
    }
}
