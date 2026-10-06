//! Browser-side receiving handles retain the actual issuing IPC connection.
//! Wire lookup IDs never stand alone as Reporting or owner authority.
use super::*;
use amazing_fetch::{EmbedderPolicyContainer, ReceivingDocumentClient};

/// The browser supplies the genuine opaque RealmHandle and arena Document.
/// Neither is reconstructed from a network URL or a numeric realm guess.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeReceivingIdentity {
    pub realm: amazing_js::interp::RealmHandle,
    pub window: u64,
    pub document: amazing_dom::NodeIdx,
    pub generation: u64,
}
impl ReceivingClientCapture {
    pub fn from_native(
        client: &ReceivingDocumentClient<NativeReceivingIdentity>,
        realm_binding: u64,
    ) -> Self {
        Self {
            identity: ReceivingGlobalIdentity {
                realm_binding,
                window: client.identity.window,
                document: u64::from(client.identity.document.0),
                generation: client.identity.generation,
            },
            creation_url: client.creation_url.serialize(),
            document_url: client.document_url.serialize(),
            base_url: client.base_url.serialize(),
            origin: origin_to_wire(&client.origin),
            csp_self_origin: origin_to_wire(&client.csp_self_origin),
            context: (&client.context).into(),
            referrer_policy: super::receiving_capture::referrer_name(client.referrer_policy).into(),
        }
    }
}
#[derive(Clone)]
pub struct ReceivingGlobalHandle {
    connection: Arc<Connection>,
    receipt: RegisteredReceivingGlobal,
    identity: NativeReceivingIdentity,
}
impl ReceivingGlobalHandle {
    pub fn identity(&self) -> &NativeReceivingIdentity {
        &self.identity
    }
}
/// An unused owner plan has one browser-side owner. Explicit release returns
/// its native quota; successful navigation consumes the plan once on the server.
#[must_use]
pub struct ReceivingOwnerHandle {
    connection: Arc<Connection>,
    receipt: PreparedReceivingOwner,
}
/// Implemented by the real original Window/task owner, never a worker stub.
/// Listener errors must be recorded along with terminal facts; success means
/// the whole batch was processed or explicitly disposed as stale in that saved
/// scope. Closure verifies actual scope retirement AND queued-task drain.
/// No default implementation exists, and socket receipt is not an ACK.
pub trait ReceivingReportTaskOwner {
    fn consume_original_report_batch(&mut self, identity: &NativeReceivingIdentity,
        batch_id: u64, events: &[ReceivingReportEvent]) -> Result<(), Error>;
    fn retire_original_report_tasks(&mut self, identity: &NativeReceivingIdentity)
        -> Result<(), Error>;
}
/// One actual original-scope consumer. It cannot be cloned or reconstructed
/// from a global URL or decoded observations. Capture before original tasks.
/// ```compile_fail
/// use amazing_process::network_v2::ReceivingReportConsumerHandle;
/// fn duplicate(value: ReceivingReportConsumerHandle) { let _ = value.clone(); }
/// ```
#[must_use]
pub struct ReceivingReportConsumerHandle {
    connection: Arc<Connection>,
    receipt: RegisteredReceivingReportConsumer,
    identity: NativeReceivingIdentity,
    pending_batch: Option<u64>,
    closed: bool,
    released: bool,
}
/// Server-retained original outcomes. ACK follows real original-scope task
/// processing (or explicit stale-task disposition), never network receipt.
#[must_use]
pub struct ReceivingReportBatchHandle {
    connection: Arc<Connection>,
    wire: ReceivingReportBatch,
    acknowledged: bool,
    processed: bool,
}
impl ReceivingReportBatchHandle {
    pub fn events(&self) -> &[ReceivingReportEvent] { &self.wire.events }
}
#[derive(Default)]
pub(super) struct ReceivingRealmAssociations {
    next_binding: u64,
    entries: std::collections::HashMap<amazing_js::interp::RealmHandle, ReceivingRealmAssociation>,
}
struct ReceivingRealmAssociation {
    binding: u64,
    globals: std::collections::BTreeSet<u64>,
}
impl NetworkSession {
    fn receiving_proxy(&self) -> ReceivingGlobalsProxy {
        ReceivingGlobalsProxy::new(self.host.connection.clone(), self.timeout)
    }
    fn check_receiving(&self, handle: &ReceivingGlobalHandle) -> Result<(), Error> {
        if !Arc::ptr_eq(&self.host.connection, &handle.connection)
            || (handle.receipt.client_id, handle.receipt.session)
                != (self.client_id, self.receiving_session)
        {
            return Err(invalid("foreign receiving connection"));
        }
        Ok(())
    }
    fn check_owner(&self, handle: &ReceivingOwnerHandle) -> Result<(), Error> {
        if !Arc::ptr_eq(&self.host.connection, &handle.connection)
            || (handle.receipt.owner.client_id, handle.receipt.owner.session)
                != (self.client_id, self.receiving_session)
        {
            return Err(invalid("foreign owner connection"));
        }
        Ok(())
    }
    /// Retain the authentic navigation request/head past normal body completion.
    /// Legacy start/fetch paths keep their original lifetime and quotas.
    pub fn capture_receiving_navigation(
        &self,
        mut request: NetworkStart,
    ) -> Result<NetworkStarted, Error> {
        request.client_id = self.client_id;
        request.epoch = self.epoch;
        request.version = NETWORK_VERSION;
        self.receiving_proxy()
            .capture_navigation(CaptureReceivingNavigation {
                session: self.receiving_session,
                request,
            })
    }
    /// Actual HTML capture + server accepted request ID, or explicit fresh
    /// about creation. Never accepts a driver-service ReportingDocument.
    pub fn register_receiving_global(
        &self,
        client: &ReceivingDocumentClient<NativeReceivingIdentity>,
        coep: &EmbedderPolicyContainer,
        secure_context: bool,
        accepted: Option<&NetworkStarted>,
    ) -> Result<ReceivingGlobalHandle, Error> {
        let admission = match accepted {
            Some(accepted) => {
                if (accepted.client_id, accepted.epoch) != (self.client_id, self.epoch) {
                    return Err(invalid("foreign accepted receiving navigation"));
                }
                ReceivingAdmission::AcceptedResponse(accepted.id)
            }
            None => ReceivingAdmission::FreshInitial(()),
        };
        // Hold the association through registration so failures cannot leave
        // unused realms occupying the existing 64-entry quota. Binding IDs are
        // monotonic; releasing a real last global never reuses a retired ID.
        let mut realms = self.receiving_realms.lock()
            .map_err(|_| invalid("receiving realm association lock"))?;
        let realm_binding = if let Some(saved) = realms.entries.get(&client.identity.realm) {
            saved.binding
        } else {
            if realms.entries.len() >= 64 { return Err(invalid("receiving realm association quota")); }
            realms.next_binding = realms.next_binding.checked_add(1)
                .ok_or_else(|| invalid("receiving realm association limit"))?;
            let binding = realms.next_binding;
            realms.entries.insert(client.identity.realm, ReceivingRealmAssociation {
                binding, globals: Default::default(),
            });
            binding
        };
        let capture = ReceivingClientCapture::from_native(client, realm_binding);
        let identity = capture.identity.clone();
        let registration = self.receiving_proxy().register(RegisterReceivingGlobal {
            session: self.receiving_session,
            client_id: self.client_id,
            epoch: self.epoch,
            capture,
            coep: coep.into(),
            secure_context,
            admission,
        });
        let receipt = match registration {
            Ok(receipt) => receipt,
            Err(error) => {
                if realms.entries.get(&client.identity.realm).is_some_and(|entry| entry.globals.is_empty()) {
                    realms.entries.remove(&client.identity.realm);
                }
                return Err(error);
            }
        };
        if (receipt.client_id, receipt.session) != (self.client_id, self.receiving_session)
            || receipt.identity != identity
        {
            return Err(invalid("receiving server registration identity"));
        }
        realms.entries.get_mut(&client.identity.realm).unwrap().globals.insert(receipt.id);
        Ok(ReceivingGlobalHandle {
            connection: self.host.connection.clone(),
            receipt,
            identity: client.identity,
        })
    }
    pub fn prepare_receiving_owner(
        &self,
        owner: &ReceivingGlobalHandle,
        frame: ReceivingFrameIdentity,
        parent: &NavigableContext,
        element: amazing_fetch::reporting::NavigationEmbeddingElement,
    ) -> Result<ReceivingOwnerHandle, Error> {
        self.check_receiving(owner)?;
        let receipt = self
            .receiving_proxy()
            .prepare_owner(PrepareReceivingOwner {
                owner: owner.receipt.clone(),
                frame: frame.clone(),
                parent: parent.into(),
                element: match element {
                    amazing_fetch::reporting::NavigationEmbeddingElement::Frame => "frame",
                    amazing_fetch::reporting::NavigationEmbeddingElement::Iframe => "iframe",
                }
                .into(),
            })?;
        if receipt.owner != owner.receipt || receipt.frame != frame {
            return Err(invalid("receiving server owner identity"));
        }
        Ok(ReceivingOwnerHandle {
            connection: self.host.connection.clone(),
            receipt,
        })
    }
    pub fn start_receiving_navigation(
        &self,
        source: &ReceivingGlobalHandle,
        owner: ReceivingOwnerHandle,
        target: &HttpUrl,
        response_limit: u32,
    ) -> Result<NetworkStarted, Error> {
        self.check_receiving(source)?;
        self.check_owner(&owner)?;
        self.receiving_proxy()
            .start_navigation(StartReceivingNavigation {
                source: source.receipt.clone(),
                owner: owner.receipt,
                client_id: self.client_id,
                epoch: self.epoch,
                target: target.serialize(),
                response_limit,
            })
    }
    pub fn continue_receiving_navigation(
        &self,
        previous: &NetworkStarted,
        source: &ReceivingGlobalHandle,
        frame: ReceivingFrameIdentity,
        response_limit: u32,
    ) -> Result<NetworkStarted, Error> {
        self.check_receiving(source)?;
        if (previous.client_id, previous.epoch) != (self.client_id, self.epoch) {
            return Err(invalid("foreign navigation continuation"));
        }
        self.receiving_proxy()
            .continue_navigation(ContinueReceivingNavigation {
                previous: previous.clone(),
                source: source.receipt.clone(),
                frame,
                response_limit,
            })
    }
    /// A native admission failure has no invented ServiceId. Its actual CSP
    /// snapshots remain available through the protecting global's report queue.
    pub fn start_receiving_navigation_outcome(
        &self,
        source: &ReceivingGlobalHandle,
        owner: ReceivingOwnerHandle,
        target: &HttpUrl,
        response_limit: u32,
    ) -> Result<ReceivingNavigationOutcome, Error> {
        self.check_receiving(source)?;
        self.check_owner(&owner)?;
        self.receiving_proxy()
            .start_navigation_outcome(StartReceivingNavigation {
                source: source.receipt.clone(),
                owner: owner.receipt,
                client_id: self.client_id,
                epoch: self.epoch,
                target: target.serialize(),
                response_limit,
            })
    }
    pub fn continue_receiving_navigation_outcome(
        &self,
        previous: &NetworkStarted,
        source: &ReceivingGlobalHandle,
        frame: ReceivingFrameIdentity,
        response_limit: u32,
    ) -> Result<ReceivingNavigationOutcome, Error> {
        self.check_receiving(source)?;
        if (previous.client_id, previous.epoch) != (self.client_id, self.epoch) {
            return Err(invalid("foreign navigation continuation"));
        }
        self.receiving_proxy()
            .continue_navigation_outcome(ContinueReceivingNavigation {
                previous: previous.clone(),
                source: source.receipt.clone(),
                frame,
                response_limit,
            })
    }
    /// Consume after actual browser SPVE dispatch, including author exceptions.
    /// The server's move-only native plan may outlive global retirement.
    pub fn finish_receiving_csp_report(
        &self,
        source: &ReceivingGlobalHandle,
        report: u64,
    ) -> Result<ReceivingCspDelivery, Error> {
        self.check_receiving(source)?;
        self.receiving_proxy()
            .finish_csp_report(FinishReceivingCspReport {
                source: source.receipt.clone(),
                report,
            })
    }
    pub fn retire_receiving_global(&self, handle: &ReceivingGlobalHandle) -> Result<bool, Error> {
        self.check_receiving(handle)?;
        self.receiving_proxy().retire(handle.receipt.clone())
    }
    pub fn release_receiving_owner(&self, handle: ReceivingOwnerHandle) -> Result<bool, Error> {
        self.check_owner(&handle)?;
        self.receiving_proxy().release_owner(handle.receipt)
    }
    pub fn release_receiving_navigation(&self, started: &NetworkStarted) -> Result<bool, Error> {
        if (started.client_id, started.epoch) != (self.client_id, self.epoch) {
            return Err(invalid("foreign retained navigation"));
        }
        self.receiving_proxy().release_navigation(started.clone())
    }
    pub fn poll_receiving_reports(
        &self,
        handle: &ReceivingGlobalHandle,
    ) -> Result<Vec<ReceivingReportEvent>, Error> {
        self.check_receiving(handle)?;
        self.receiving_proxy().poll_reports(handle.receipt.clone())
    }
    fn check_report_consumer(&self, handle: &ReceivingReportConsumerHandle) -> Result<(), Error> {
        if handle.released || !Arc::ptr_eq(&self.host.connection, &handle.connection)
            || (handle.receipt.global.client_id, handle.receipt.global.session)
                != (self.client_id, self.receiving_session)
        { return Err(invalid("foreign or released receiving report consumer")); }
        Ok(())
    }
    /// The actual task owner captures this before original global retirement.
    /// This is not a new reporting/global authority or a worker implementation.
    pub fn prepare_receiving_report_consumer(&self, global: &ReceivingGlobalHandle)
        -> Result<ReceivingReportConsumerHandle, Error>
    {
        self.check_receiving(global)?;
        let receipt = self.receiving_proxy().prepare_report_consumer(global.receipt.clone())?;
        if receipt.global != global.receipt { return Err(invalid("receiving report consumer issuance")); }
        Ok(ReceivingReportConsumerHandle { connection: self.host.connection.clone(),
            receipt, identity: global.identity, pending_batch: None, closed: false, released: false })
    }
    pub fn poll_receiving_report_batch(&self, consumer: &mut ReceivingReportConsumerHandle)
        -> Result<Option<ReceivingReportBatchHandle>, Error>
    {
        self.check_report_consumer(consumer)?;
        if consumer.pending_batch.is_some() { return Err(invalid("receiving batch needs original consumer ACK")); }
        let Some(wire) = self.receiving_proxy().poll_report_batch(consumer.receipt.clone())? else { return Ok(None); };
        if wire.receipt.consumer != consumer.receipt || wire.events.is_empty() {
            return Err(invalid("receiving report batch issuance"));
        }
        consumer.pending_batch = Some(wire.receipt.batch);
        Ok(Some(ReceivingReportBatchHandle { connection: self.host.connection.clone(), wire, acknowledged: false, processed: false }))
    }
    /// Call only after genuine processing under the saved original task scope.
    /// Errors leave the batch pending so the exact ACK can be retried; no
    /// callbacks should run twice merely because its acknowledgement was lost.
    fn acknowledge_receiving_report_batch(&self, consumer: &mut ReceivingReportConsumerHandle,
        batch: &mut ReceivingReportBatchHandle) -> Result<bool, Error>
    {
        self.check_report_consumer(consumer)?;
        if !Arc::ptr_eq(&self.host.connection, &batch.connection)
            || batch.wire.receipt.consumer != consumer.receipt
        { return Err(invalid("foreign receiving report batch")); }
        if batch.acknowledged { return Ok(false); }
        if consumer.pending_batch != Some(batch.wire.receipt.batch) {
            return Err(invalid("receiving original batch mismatch"));
        }
        let first = self.receiving_proxy().acknowledge_report_batch(batch.wire.receipt.clone())?;
        batch.acknowledged = true;
        consumer.pending_batch = None;
        Ok(first)
    }
    /// The real original Window/task owner calls this only after retirement AND
    /// task drain/disposition. Navigation or a socket reply is not that proof.
    /// Existing native transport/completion custody still prevents release.
    fn close_receiving_report_consumer(&self, consumer: &mut ReceivingReportConsumerHandle)
        -> Result<bool, Error>
    {
        self.check_report_consumer(consumer)?;
        if consumer.closed { return Ok(false); }
        let first = self.receiving_proxy().close_report_consumer(consumer.receipt.clone())?;
        consumer.closed = true;
        Ok(first)
    }
    /// Processing errors retain the server batch and its quota. An actual task
    /// owner records author errors/dispositions before returning success.
    pub fn consume_receiving_report_batch(&self, consumer: &mut ReceivingReportConsumerHandle,
        batch: &mut ReceivingReportBatchHandle, owner: &mut impl ReceivingReportTaskOwner)
        -> Result<bool, Error>
    {
        self.check_report_consumer(consumer)?;
        if batch.acknowledged { return Ok(false); }
        if batch.wire.receipt.consumer != consumer.receipt
            || !Arc::ptr_eq(&self.host.connection, &batch.connection)
        { return Err(invalid("foreign original-scope batch")); }
        if !batch.processed {
            // The owner uses this original batch ID for a per-event cursor if
            // task infrastructure fails partway through. Author errors are
            // handled/recorded dispositions, not an implicit retry from zero.
            owner.consume_original_report_batch(&consumer.identity, batch.wire.receipt.batch, batch.events())?;
            batch.processed = true;
        }
        self.acknowledge_receiving_report_batch(consumer, batch)
    }
    pub fn close_receiving_report_consumer_with_owner(&self, consumer: &mut ReceivingReportConsumerHandle,
        owner: &mut impl ReceivingReportTaskOwner) -> Result<bool, Error>
    {
        self.check_report_consumer(consumer)?;
        if consumer.closed { return Ok(false); }
        owner.retire_original_report_tasks(&consumer.identity)?;
        self.close_receiving_report_consumer(consumer)
    }
    pub fn release_retired_receiving_global(&self, consumer: &mut ReceivingReportConsumerHandle)
        -> Result<ReceivingGlobalRelease, Error>
    {
        self.check_report_consumer(consumer)?;
        if !consumer.closed { return Err(invalid("receiving original task scope not closed")); }
        let result = self.receiving_proxy().release_retired_global(consumer.receipt.clone())?;
        if result == ReceivingGlobalRelease::Released(()) {
            consumer.released = true;
            let mut realms = self.receiving_realms.lock()
                .map_err(|_| invalid("receiving realm association lock"))?;
            if let Some(entry) = realms.entries.get_mut(&consumer.identity.realm) {
                entry.globals.remove(&consumer.receipt.global.id);
                if entry.globals.is_empty() { realms.entries.remove(&consumer.identity.realm); }
            }
        }
        Ok(result)
    }

}
