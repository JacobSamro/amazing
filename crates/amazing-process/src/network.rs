//! Network process adapter around Fetch's public NetworkService.
use crate::protocol::*;
use amazing_fetch::api::{ApiRequest, ApiRequestInit, ApiRequestInput};
use amazing_fetch::service::{ClientContext, NetworkService, ServiceEvent};
use amazing_fetch::HttpUrl;
use amazing_ipc::codec::ByteBuf;
use amazing_ipc::{Connection, Error};
use amazing_net::dispatch::DispatchLimits;
use std::collections::{BTreeMap, VecDeque};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

const CHUNK: usize = 32 * 1024;
struct State {
    service: NetworkService,
    events: BTreeMap<u64, VecDeque<NetworkEvent>>,
}
impl State {
    fn new() -> Self {
        Self {
            service: NetworkService::new(DispatchLimits::default()),
            events: BTreeMap::new(),
        }
    }
    fn collect(&mut self) {
        for event in self.service.step(64, Instant::now()) {
            let ServiceEvent::Finished { id, result } = event else {
                continue;
            };
            let Some(queue) = self.events.get_mut(&id.0) else {
                continue;
            };
            match result {
                Ok(mut response) => {
                    queue.push_back(NetworkEvent::Head(ResponseHead {
                        status: response.status,
                        url: response.url,
                    }));
                    match response.body.bytes() {
                        Ok(body) => {
                            for chunk in body.chunks(CHUNK) {
                                queue.push_back(NetworkEvent::Body(BodyChunk {
                                    bytes: ByteBuf(chunk.to_vec()),
                                }));
                            }
                            queue.push_back(NetworkEvent::Complete(Completion { error: None }));
                        }
                        Err(error) => queue.push_back(NetworkEvent::Complete(Completion {
                            error: Some(format!("{error:?}")),
                        })),
                    }
                }
                Err(error) => queue.push_back(NetworkEvent::Complete(Completion {
                    error: Some(format!("{error:?}")),
                })),
            }
        }
    }
    fn start(&mut self, request: StartRequest) -> Result<Started, Error> {
        let base = HttpUrl::parse(&request.base_url)
            .map_err(|e| Error::Remote(format!("base URL: {e:?}")))?;
        let input: Vec<u16> = request.url.encode_utf16().collect();
        let api = ApiRequest::new(
            ApiRequestInput::Url(&input),
            ApiRequestInit::default(),
            &base,
            base.origin().clone(),
        )
        .map_err(|e| Error::Remote(format!("request: {e:?}")))?;
        let id = self
            .service
            .start(api, ClientContext::default())
            .map_err(|e| Error::Remote(format!("start: {e:?}")))?;
        self.events.insert(id.0, VecDeque::new());
        Ok(Started { id: id.0 })
    }
    fn poll(&mut self, request: RequestId) -> Result<NetworkEvent, Error> {
        if !self.events.contains_key(&request.id) {
            return Err(Error::Remote("unknown request".into()));
        }
        if self.events.get(&request.id).is_some_and(VecDeque::is_empty) {
            self.collect();
        }
        let queue = self.events.get_mut(&request.id).unwrap();
        let event = queue.pop_front().unwrap_or(NetworkEvent::Pending(()));
        if matches!(event, NetworkEvent::Complete(_)) {
            self.events.remove(&request.id);
        }
        Ok(event)
    }
    fn cancel(&mut self, request: RequestId) -> bool {
        if self.events.remove(&request.id).is_none() {
            return false;
        }
        self.service
            .abort(amazing_fetch::service::ServiceId(request.id));
        true
    }
}
enum Command {
    Start(StartRequest, mpsc::Sender<Result<Started, Error>>),
    Poll(RequestId, mpsc::Sender<Result<NetworkEvent, Error>>),
    Cancel(RequestId, mpsc::Sender<bool>),
}
/// The non-Send Fetch state is created and consumed on one network actor thread.
pub struct NetworkEndpoint {
    tx: mpsc::SyncSender<Command>,
}
impl Default for NetworkEndpoint {
    fn default() -> Self {
        Self::new()
    }
}
impl NetworkEndpoint {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::sync_channel(256);
        std::thread::spawn(move || {
            let mut state = State::new();
            while let Ok(command) = rx.recv() {
                match command {
                    Command::Start(request, reply) => {
                        let _ = reply.send(state.start(request));
                    }
                    Command::Poll(request, reply) => {
                        let _ = reply.send(state.poll(request));
                    }
                    Command::Cancel(request, reply) => {
                        let _ = reply.send(state.cancel(request));
                    }
                }
            }
        });
        Self { tx }
    }
}
impl NetworkRequests for NetworkEndpoint {
    fn start_request(&self, request: StartRequest) -> Result<Started, Error> {
        let (tx, rx) = mpsc::channel();
        self.tx
            .try_send(Command::Start(request, tx))
            .map_err(|_| Error::Disconnected)?;
        rx.recv().map_err(|_| Error::Disconnected)?
    }
    fn poll_request(&self, request: RequestId) -> Result<NetworkEvent, Error> {
        let (tx, rx) = mpsc::channel();
        self.tx
            .try_send(Command::Poll(request, tx))
            .map_err(|_| Error::Disconnected)?;
        rx.recv().map_err(|_| Error::Disconnected)?
    }
    fn cancel_request(&self, request: RequestId) -> Result<bool, Error> {
        let (tx, rx) = mpsc::channel();
        self.tx
            .try_send(Command::Cancel(request, tx))
            .map_err(|_| Error::Disconnected)?;
        rx.recv().map_err(|_| Error::Disconnected)
    }
}
/// Bind the network role's service and liveness responder.
pub fn serve_network(connection: &Arc<Connection>) {
    connection.bind(0, crate::Ping);
    connection.bind(8, NetworkReceiver(NetworkEndpoint::new()));
}
/// Browser-side broker: the renderer never owns a direct process handle.
pub struct NetworkBroker {
    pub remote: NetworkProxy,
}
impl NetworkRequests for NetworkBroker {
    fn start_request(&self, request: StartRequest) -> Result<Started, Error> {
        self.remote.start_request(request)
    }
    fn poll_request(&self, request: RequestId) -> Result<NetworkEvent, Error> {
        self.remote.poll_request(request)
    }
    fn cancel_request(&self, request: RequestId) -> Result<bool, Error> {
        self.remote.cancel_request(request)
    }
}
/// Resource loader used by the renderer and by the in-process parity oracle.
pub fn fetch_resource(remote: &NetworkProxy, url: &str, base_url: &str) -> Result<Vec<u8>, Error> {
    let started = remote.start_request(StartRequest {
        url: url.into(),
        base_url: base_url.into(),
    })?;
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut body = Vec::new();
    let mut status = None;
    loop {
        if Instant::now() >= deadline {
            let _ = remote.cancel_request(RequestId { id: started.id });
            return Err(Error::Timeout);
        }
        match remote.poll_request(RequestId { id: started.id })? {
            NetworkEvent::Pending(()) => std::thread::sleep(Duration::from_millis(1)),
            NetworkEvent::Head(head) => status = Some(head.status),
            NetworkEvent::Body(chunk) => {
                if body
                    .len()
                    .checked_add(chunk.bytes.0.len())
                    .filter(|&n| n <= amazing_ipc::codec::MAX_FRAME)
                    .is_none()
                {
                    let _ = remote.cancel_request(RequestId { id: started.id });
                    return Err(Error::Codec(amazing_ipc::codec::Error::Limit));
                }
                body.extend_from_slice(&chunk.bytes.0);
            }
            NetworkEvent::Complete(done) => {
                if let Some(error) = done.error {
                    return Err(Error::Remote(error));
                }
                if !matches!(status, Some(200..=299)) {
                    return Err(Error::Remote(format!("HTTP {status:?}")));
                }
                return Ok(body);
            }
        }
    }
}
