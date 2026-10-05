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
        let realm_binding = {
            let mut realms = self
                .receiving_realms
                .lock()
                .map_err(|_| invalid("receiving realm association lock"))?;
            if let Some(id) = realms.get(&client.identity.realm) {
                *id
            } else {
                if realms.len() >= 64 {
                    return Err(invalid("receiving realm association quota"));
                }
                let id = u64::try_from(realms.len())
                    .map_err(|_| invalid("receiving realm association limit"))?
                    + 1;
                realms.insert(client.identity.realm, id);
                id
            }
        };
        let capture = ReceivingClientCapture::from_native(client, realm_binding);
        let identity = capture.identity.clone();
        let receipt = self.receiving_proxy().register(RegisterReceivingGlobal {
            session: self.receiving_session,
            client_id: self.client_id,
            epoch: self.epoch,
            capture,
            coep: coep.into(),
            secure_context,
            admission,
        })?;
        if (receipt.client_id, receipt.session) != (self.client_id, self.receiving_session)
            || receipt.identity != identity
        {
            return Err(invalid("receiving server registration identity"));
        }
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
}
