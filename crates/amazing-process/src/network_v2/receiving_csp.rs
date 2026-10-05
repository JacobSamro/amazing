//! Only the network service's genuine CSP check receipts are enriched here.
//! The browser receives complete event/report snapshots, never private receipts.
use super::*;
use amazing_fetch::reporting::{NavigationCspSourceLoan, PreparedCspReportPlan, ReportingDocument};
use amazing_fetch::{
    CspReportValue, CspRequestPhase, CspRequestReportContext, CspResponseProvenance, CspViolation,
    ReceivingDocumentClient,
};

#[derive(Clone)]
pub(super) enum ProtectedResponse {
    Initial,
    Received { status: u16, referrer: String },
}
pub(super) struct ProtectedSource {
    pub receipt: RegisteredReceivingGlobal,
    pub client: ReceivingDocumentClient<ReceivingGlobalIdentity>,
    response: ProtectedResponse,
    pub reporting: ReportingDocument,
    // Navigation has no authoritative source element or parser location.
    // A future element loader must supply its genuine saved producer separately.
    seen: Vec<CspViolation>,
    pub loan: Option<NavigationCspSourceLoan>,
}
impl ProtectedSource {
    pub fn navigation(
        receipt: RegisteredReceivingGlobal,
        client: ReceivingDocumentClient<ReceivingGlobalIdentity>,
        response: ProtectedResponse,
        reporting: ReportingDocument,
    ) -> Self {
        Self {
            receipt,
            client,
            response,
            reporting,
            seen: Vec::new(),
            loan: None,
        }
    }
    pub fn prepare(
        &mut self,
        service: &mut NetworkService,
        request: &mut ApiRequest,
    ) -> Result<(), Error> {
        if self.loan.is_some() {
            return Ok(());
        }
        let (status, referrer) = match &self.response {
            ProtectedResponse::Initial => (0, ""),
            ProtectedResponse::Received { status, referrer } => (*status, referrer.as_str()),
        };
        let context = CspRequestReportContext {
            element: None,
            referrer,
            status_code: status,
            source_file: "",
            line_number: 0,
            column_number: 0,
        };
        self.loan = Some(
            service
                .prepare_navigation_csp_reporting(self.reporting, &self.client, request, &context)
                .map_err(|e| invalid(&format!("native CSP source admission: {e:?}")))?,
        );
        Ok(())
    }
    pub fn release(&mut self, service: &mut NetworkService) {
        if let Some(loan) = self.loan.take() {
            // Same-service ownership was established at admission. Release only
            // scalar custody; already prepared event plans remain independent.
            let _ = service.release_navigation_csp_reporting(loan);
        }
    }
}
struct PendingReport {
    source: RegisteredReceivingGlobal,
    native: PreparedCspReportPlan,
}
#[derive(Default)]
pub(super) struct CspTransport {
    next: u64,
    pending: BTreeMap<u64, PendingReport>,
}
impl CspTransport {
    pub fn record(
        &mut self,
        service: &mut NetworkService,
        protected: &mut ProtectedSource,
        request: Option<NetworkStarted>,
        provenance: &CspResponseProvenance,
    ) -> Vec<ReceivingReportEvent> {
        let mut output = Vec::new();
        for (index, receipt) in provenance.violations().iter().enumerate() {
            if protected.seen.contains(receipt) {
                continue;
            }
            if protected.seen.len() >= 1024 {
                output.push(ReceivingReportEvent::CreationError(
                    "CSP receipt quota".into(),
                ));
                break;
            }
            let Some(loan) = protected.loan.as_mut() else {
                output.push(ReceivingReportEvent::CreationError(
                    "missing native CSP source loan".into(),
                ));
                break;
            };
            let native = match service.prepare_navigation_csp_report(
                loan,
                provenance,
                index,
                Instant::now(),
            ) {
                Ok(plan) => plan,
                Err(error) => {
                    output.push(ReceivingReportEvent::CreationError(format!(
                        "native CSP report admission: {error:?}"
                    )));
                    continue;
                }
            };
            let violation = native.violation();
            let snapshot = native.report_snapshot();
            let mut wire = ReceivingCspSnapshot {
                source: protected.receipt.clone(),
                request: request.clone(),
                element: None,
                location: None,
                received_response: matches!(
                    &protected.response,
                    ProtectedResponse::Received { .. }
                ),
                phase: match receipt.request_phase() {
                    CspRequestPhase::PreRequest => "pre-request",
                    CspRequestPhase::PostRequest => "post-request",
                    CspRequestPhase::WebSocketPreRequest => "websocket-pre-request",
                }
                .into(),
                url_list: receipt
                    .request_url_list()
                    .iter()
                    .map(HttpUrl::serialize)
                    .collect(),
                effective_directive: violation.effective_directive.clone(),
                violated_directive: violation.violated_directive.clone(),
                source_directive: violation.source_directive.clone(),
                policy_index: violation.policy_index as u64,
                report_type: snapshot.report_type,
                report_url: snapshot.url,
                body: snapshot
                    .body
                    .into_iter()
                    .map(|(name, value)| ReceivingCspField {
                        name,
                        value: match value {
                            CspReportValue::Null => ReceivingCspValue::Null(()),
                            CspReportValue::Number(value) => {
                                ReceivingCspValue::NumberBits(value.to_bits())
                            }
                            CspReportValue::String(value) => ReceivingCspValue::String(value),
                        },
                    })
                    .collect(),
                prepared_report: None,
                delivery_error: None,
            };
            if snapshot_bytes(&wire) > 128 * 1024 {
                output.push(ReceivingReportEvent::CreationError(
                    "CSP snapshot quota".into(),
                ));
                continue;
            }
            // The native plan uses the original source settings captured at
            // request admission, including when retirement preceded this receipt.
            if self.pending.len() >= 128 || self.next == u64::MAX {
                wire.delivery_error = Some("CSP prepared report quota".into());
            } else {
                self.next += 1;
                self.pending.insert(
                    self.next,
                    PendingReport {
                        source: protected.receipt.clone(),
                        native,
                    },
                );
                wire.prepared_report = Some(self.next);
            }
            protected.seen.push(receipt.clone());
            output.push(ReceivingReportEvent::Csp(wire));
        }
        output
    }
    pub fn finish(
        &mut self,
        service: &mut NetworkService,
        request: FinishReceivingCspReport,
    ) -> Result<ReceivingCspDelivery, Error> {
        let pending = self
            .pending
            .get(&request.report)
            .ok_or_else(|| invalid("unknown CSP prepared report"))?;
        if pending.source != request.source {
            return Err(invalid("CSP prepared report ownership"));
        }
        let pending = self.pending.remove(&request.report).unwrap();
        Ok(
            match service.queue_prepared_csp_report(pending.native, Instant::now()) {
                Ok(ids) => ReceivingCspDelivery {
                    ids: ids.into_iter().map(|id| id.0).collect(),
                    error: None,
                },
                Err(error) => ReceivingCspDelivery {
                    ids: Vec::new(),
                    error: Some(format!("{error:?}")),
                },
            },
        )
    }
}
pub(super) fn snapshot_bytes(snapshot: &ReceivingCspSnapshot) -> usize {
    let units = snapshot.report_type.len()
        + snapshot.report_url.len()
        + snapshot
            .body
            .iter()
            .map(|field| {
                field.name.len()
                    + match &field.value {
                        ReceivingCspValue::String(value) => value.len(),
                        _ => 4,
                    }
            })
            .sum::<usize>();
    units
        .saturating_mul(2)
        .saturating_add(snapshot.url_list.iter().map(String::len).sum::<usize>())
        .saturating_add(snapshot.delivery_error.as_ref().map_or(0, String::len))
        .saturating_add(512)
}
impl State {
    pub(super) fn receiving_csp_progress(
        &mut self,
        id: ServiceId,
        provenance: Option<&CspResponseProvenance>,
    ) {
        let Some(provenance) = provenance else {
            return;
        };
        let Some(saved) = self.retained_navigation.get_mut(&id.0) else {
            return;
        };
        let Some(protected) = saved.protected.as_mut() else {
            return;
        };
        let request = Some(NetworkStarted {
            id: id.0,
            client_id: saved.client_id,
            epoch: saved.epoch,
        });
        let events = self
            .receiving_csp
            .record(&mut self.service, protected, request, provenance);
        for event in events {
            self.receiving.queue_report(protected.reporting, event);
        }
    }
}
