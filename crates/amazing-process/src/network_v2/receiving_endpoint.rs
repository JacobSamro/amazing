//! Authenticated network-role receiving registration and actual stream admission.
use super::receiving_registry::RetainedNavigation;
use super::*;

pub const CLIENT_CAPABILITY: &str = "__receiving_network_client";
pub const SESSION_CAPABILITY: &str = "__receiving_network_session";
pub(super) enum ReceivingCommand {
    Capture(
        CaptureReceivingNavigation,
        mpsc::Sender<Result<NetworkStarted, Error>>,
    ),
    Register(
        RegisterReceivingGlobal,
        mpsc::Sender<Result<RegisteredReceivingGlobal, Error>>,
    ),
    Prepare(
        PrepareReceivingOwner,
        mpsc::Sender<Result<PreparedReceivingOwner, Error>>,
    ),
    Start(
        StartReceivingNavigation,
        mpsc::Sender<Result<ReceivingNavigationOutcome, Error>>,
    ),
    Continue(
        ContinueReceivingNavigation,
        mpsc::Sender<Result<ReceivingNavigationOutcome, Error>>,
    ),
    FinishReport(
        FinishReceivingCspReport,
        mpsc::Sender<Result<ReceivingCspDelivery, Error>>,
    ),
    Retire(RegisteredReceivingGlobal, mpsc::Sender<Result<bool, Error>>),
    Release(PreparedReceivingOwner, mpsc::Sender<Result<bool, Error>>),
    ReleaseNavigation(NetworkStarted, mpsc::Sender<Result<bool, Error>>),
    Reports(
        RegisteredReceivingGlobal,
        mpsc::Sender<Result<Vec<ReceivingReportEvent>, Error>>,
    ),
}
impl State {
    fn receiving_authorized(&self, client: u64, session: u128) -> Result<(), Error> {
        if self.receiving_authority != Some((client, session)) {
            return Err(invalid("receiving authenticated client/session"));
        }
        Ok(())
    }
    fn receiving_register(
        &mut self,
        wire: RegisterReceivingGlobal,
    ) -> Result<RegisteredReceivingGlobal, Error> {
        self.receiving_authorized(wire.client_id, wire.session)?;
        self.receiving.register(
            &mut self.service,
            &self.retained_navigation,
            &self.epochs,
            wire,
        )
    }
    fn receiving_prepare(
        &mut self,
        wire: PrepareReceivingOwner,
    ) -> Result<PreparedReceivingOwner, Error> {
        self.receiving_authorized(wire.owner.client_id, wire.owner.session)?;
        self.receiving.prepare(&mut self.service, wire)
    }
    fn receiving_start(
        &mut self,
        wire: StartReceivingNavigation,
    ) -> Result<ReceivingNavigationOutcome, Error> {
        self.receiving_authorized(wire.source.client_id, wire.source.session)?;
        self.receiving_authorized(wire.owner.owner.client_id, wire.owner.owner.session)?;
        self.receiving_stream_bounds(wire.client_id, wire.epoch, wire.response_limit)?;
        let (request, protected) = self.receiving.take_navigation(&wire)?;
        self.receiving_dispatch(
            request,
            protected,
            wire.client_id,
            wire.epoch,
            wire.response_limit,
            wire.source,
            wire.owner.frame,
        )
    }
    fn receiving_continue(
        &mut self,
        wire: ContinueReceivingNavigation,
    ) -> Result<ReceivingNavigationOutcome, Error> {
        self.receiving_authorized(wire.source.client_id, wire.source.session)?;
        self.receiving_stream_bounds(
            wire.previous.client_id,
            wire.previous.epoch,
            wire.response_limit,
        )?;
        let saved = self
            .retained_navigation
            .get_mut(&wire.previous.id)
            .ok_or_else(|| invalid("unknown continuation navigation"))?;
        if saved.continuation_used
            || (saved.client_id, saved.epoch) != (wire.previous.client_id, wire.previous.epoch)
            || saved.source.as_ref() != Some(&wire.source)
            || saved.frame.as_ref() != Some(&wire.frame)
        {
            return Err(invalid("navigation continuation ownership"));
        }
        let head = saved
            .head
            .as_ref()
            .ok_or_else(|| invalid("continuation head not accepted"))?;
        // Uses the actual sealed request/head. Public NetworkHead fields are
        // deliberately not inputs to this continuation operation.
        let mut request = saved
            .request
            .continue_navigation_head(head)
            .map_err(|e| invalid(&format!("native continuation: {e:?}")))?;
        request
            .set_response_body_limit(u64::from(wire.response_limit))
            .map_err(|_| invalid("continuation response limit"))?;
        let protected = saved
            .protected
            .take()
            .ok_or_else(|| invalid("continuation protected source"))?;
        saved.continuation_used = true;
        self.receiving_dispatch(
            request,
            protected,
            wire.previous.client_id,
            wire.previous.epoch,
            wire.response_limit,
            wire.source,
            wire.frame,
        )
    }
    fn receiving_stream_bounds(&self, client: u64, epoch: u64, limit: u32) -> Result<(), Error> {
        if epoch == 0
            || limit > NETWORK_MAX_RESPONSE_BODY
            || self
                .epochs
                .get(&client)
                .is_some_and(|current| epoch < *current)
            || self.active.len() >= MAX_ACTIVE
            || self.retained_navigation.len() >= receiving_registry::MAX_ACCEPTED_NAVIGATIONS
        {
            return Err(invalid("receiving navigation epoch or quota"));
        }
        Ok(())
    }
    fn receiving_dispatch(
        &mut self,
        mut request: ApiRequest,
        mut protected: super::receiving_csp::ProtectedSource,
        client_id: u64,
        epoch: u64,
        response_limit: u32,
        source: RegisteredReceivingGlobal,
        frame: ReceivingFrameIdentity,
    ) -> Result<ReceivingNavigationOutcome, Error> {
        protected.prepare(&mut self.service, &mut request)?;
        let retained = request
            .clone_request()
            .map_err(|e| invalid(&format!("receiving request retention: {e:?}")))?;
        let retained_context = protected.client.context.clone();
        if let Some(current) = self
            .epochs
            .get(&client_id)
            .copied()
            .filter(|current| epoch > *current)
        {
            self.cancel_epoch(NetworkEpoch {
                client_id,
                epoch: current,
            });
        }
        let id = match self.service.start_navigation_received_stream(
            request,
            &protected.client,
            0,
            Instant::now(),
        ) {
            Ok(id) => id,
            Err(error) => {
                let (diagnostic, progress) = match error {
                    amazing_fetch::service::NavigationReceivedStartError::Reporting(error) => {
                        (format!("Reporting({error:?})"), None)
                    }
                    amazing_fetch::service::NavigationReceivedStartError::Fetch(failure) => {
                        (format!("{:?}", failure.error), failure.csp_provenance)
                    }
                };
                if let Some(progress) = progress.as_ref() {
                    for event in
                        self.receiving_csp
                            .record(&mut self.service, &mut protected, None, progress)
                    {
                        self.receiving.queue_report(protected.reporting, event);
                    }
                }
                let failure = ReceivingNavigationFailure {
                    source: source.clone(),
                    request: None,
                    error: diagnostic,
                    checked: progress.is_some(),
                    url_list: progress.as_ref().map_or_else(Vec::new, |v| {
                        v.url_list().iter().map(HttpUrl::serialize).collect()
                    }),
                };
                protected.release(&mut self.service);
                return Ok(ReceivingNavigationOutcome::Failed(failure));
            }
        };
        self.epochs.insert(client_id, epoch);
        self.retained_navigation.insert(
            id.0,
            RetainedNavigation {
                request: retained,
                context: retained_context,
                protected: Some(protected),
                client_id,
                epoch,
                head: None,
                frame: Some(frame),
                source: Some(source),
                continuation_used: false,
            },
        );
        self.active.insert(
            id.0,
            Active {
                client_id,
                epoch,
                response_limit,
                received: 0,
                credit_remaining: 0,
                seen_head: false,
                terminal: false,
                subresource: false,
                navigation: true,
                events: VecDeque::new(),
            },
        );
        Ok(ReceivingNavigationOutcome::Started(NetworkStarted {
            id: id.0,
            client_id,
            epoch,
        }))
    }
    fn receiving_release_navigation(&mut self, wire: NetworkStarted) -> Result<bool, Error> {
        let (client, session) = self
            .receiving_authority
            .ok_or_else(|| invalid("missing receiving client"))?;
        self.receiving_authorized(wire.client_id, session)?;
        if wire.client_id != client {
            return Err(invalid("receiving navigation client"));
        }
        let Some(saved) = self.retained_navigation.get(&wire.id) else {
            return Ok(false);
        };
        if (saved.client_id, saved.epoch) != (wire.client_id, wire.epoch)
            || self.active.contains_key(&wire.id)
        {
            return Err(invalid(
                "receiving navigation release ownership or live stream",
            ));
        }
        if let Some(mut saved) = self.retained_navigation.remove(&wire.id) {
            if let Some(protected) = saved.protected.as_mut() {
                protected.release(&mut self.service);
            }
        }
        Ok(true)
    }
    pub(super) fn receiving_command(&mut self, command: ReceivingCommand) {
        match command {
            ReceivingCommand::Capture(v, out) => {
                let result = self
                    .receiving_authorized(v.request.client_id, v.session)
                    .and_then(|()| self.start_kind(v.request, false, true));
                let _ = out.send(result);
            }
            ReceivingCommand::Register(v, out) => {
                let _ = out.send(self.receiving_register(v));
            }
            ReceivingCommand::Prepare(v, out) => {
                let _ = out.send(self.receiving_prepare(v));
            }
            ReceivingCommand::Start(v, out) => {
                let _ = out.send(self.receiving_start(v));
            }
            ReceivingCommand::Continue(v, out) => {
                let _ = out.send(self.receiving_continue(v));
            }
            ReceivingCommand::FinishReport(v, out) => {
                let result = self
                    .receiving_authorized(v.source.client_id, v.source.session)
                    .and_then(|()| self.receiving_csp.finish(&mut self.service, v));
                let _ = out.send(result);
            }
            ReceivingCommand::Retire(v, out) => {
                let result = self
                    .receiving_authorized(v.client_id, v.session)
                    .and_then(|()| self.receiving.retire(&mut self.service, v));
                let _ = out.send(result);
            }
            ReceivingCommand::Release(v, out) => {
                let result = self
                    .receiving_authorized(v.owner.client_id, v.owner.session)
                    .and_then(|()| self.receiving.release(v));
                let _ = out.send(result);
            }
            ReceivingCommand::ReleaseNavigation(v, out) => {
                let _ = out.send(self.receiving_release_navigation(v));
            }
            ReceivingCommand::Reports(v, out) => {
                let result = self
                    .receiving_authorized(v.client_id, v.session)
                    .and_then(|()| {
                        self.collect();
                        self.receiving.poll_reports(v)
                    });
                let _ = out.send(result);
            }
        }
    }
}
impl ReceivingGlobals for NetworkEndpointV2 {
    fn capture_navigation(&self, v: CaptureReceivingNavigation) -> Result<NetworkStarted, Error> {
        self.ask(|out| Command::Receiving(ReceivingCommand::Capture(v, out)))?
    }
    fn register(&self, v: RegisterReceivingGlobal) -> Result<RegisteredReceivingGlobal, Error> {
        self.ask(|out| Command::Receiving(ReceivingCommand::Register(v, out)))?
    }
    fn prepare_owner(&self, v: PrepareReceivingOwner) -> Result<PreparedReceivingOwner, Error> {
        self.ask(|out| Command::Receiving(ReceivingCommand::Prepare(v, out)))?
    }
    fn start_navigation(&self, v: StartReceivingNavigation) -> Result<NetworkStarted, Error> {
        match self.start_navigation_outcome(v)? {
            ReceivingNavigationOutcome::Started(started) => Ok(started),
            ReceivingNavigationOutcome::Failed(failure) => Err(invalid(&failure.error)),
        }
    }
    fn continue_navigation(&self, v: ContinueReceivingNavigation) -> Result<NetworkStarted, Error> {
        match self.continue_navigation_outcome(v)? {
            ReceivingNavigationOutcome::Started(started) => Ok(started),
            ReceivingNavigationOutcome::Failed(failure) => Err(invalid(&failure.error)),
        }
    }
    fn start_navigation_outcome(
        &self,
        v: StartReceivingNavigation,
    ) -> Result<ReceivingNavigationOutcome, Error> {
        self.ask(|out| Command::Receiving(ReceivingCommand::Start(v, out)))?
    }
    fn continue_navigation_outcome(
        &self,
        v: ContinueReceivingNavigation,
    ) -> Result<ReceivingNavigationOutcome, Error> {
        self.ask(|out| Command::Receiving(ReceivingCommand::Continue(v, out)))?
    }
    fn finish_csp_report(
        &self,
        v: FinishReceivingCspReport,
    ) -> Result<ReceivingCspDelivery, Error> {
        self.ask(|out| Command::Receiving(ReceivingCommand::FinishReport(v, out)))?
    }
    fn retire(&self, v: RegisteredReceivingGlobal) -> Result<bool, Error> {
        self.ask(|out| Command::Receiving(ReceivingCommand::Retire(v, out)))?
    }
    fn release_owner(&self, v: PreparedReceivingOwner) -> Result<bool, Error> {
        self.ask(|out| Command::Receiving(ReceivingCommand::Release(v, out)))?
    }
    fn release_navigation(&self, v: NetworkStarted) -> Result<bool, Error> {
        self.ask(|out| Command::Receiving(ReceivingCommand::ReleaseNavigation(v, out)))?
    }
    fn poll_reports(
        &self,
        v: RegisteredReceivingGlobal,
    ) -> Result<Vec<ReceivingReportEvent>, Error> {
        self.ask(|out| Command::Receiving(ReceivingCommand::Reports(v, out)))?
    }
}
/// Only the accepted browser bootstrap can install the receiving interface.
/// Legacy network serving remains interface10-only when these captures are absent.
pub fn serve_with_receiving_client(
    connection: &Arc<Connection>,
    capabilities: &crate::Capabilities,
) -> Result<(), crate::Error> {
    let client = capabilities
        .0
        .get(CLIENT_CAPABILITY)
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|v| *v != 0)
        .ok_or(crate::Error::Limit)?;
    let session = capabilities
        .0
        .get(SESSION_CAPABILITY)
        .and_then(|v| v.parse::<u128>().ok())
        .filter(|v| *v != 0)
        .ok_or(crate::Error::Limit)?;
    let endpoint = NetworkEndpointV2::with_receiving_authority(Some((client, session)));
    connection.bind(10, NetworkReceiverV2(endpoint.clone()));
    connection.bind(16, ReceivingGlobalsReceiver(endpoint));
    Ok(())
}
