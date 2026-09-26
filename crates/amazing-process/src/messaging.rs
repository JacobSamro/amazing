//! Browser-owned cross-document message routing (BroadcastChannel + entangled
//! MessagePort pairs) with bounded FIFO delivery. The browser mints every
//! client, channel and port identity; renderers never choose their own.
use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub const MAX_CLIENTS: usize = 4096;
pub const MAX_CHANNELS: usize = 8192;
pub const MAX_CHANNELS_PER_CLIENT: usize = 1024;
pub const MAX_PORTS: usize = 4096;
pub const MAX_PORTS_PER_CLIENT: usize = 1024;
pub const MAX_QUEUE: usize = 1024;
pub const MAX_NAME_BYTES: usize = 1024;
pub const MAX_ORIGIN_BYTES: usize = 4096;
/// One structured-clone graph fits in a single IPC frame with headroom.
pub const MAX_MESSAGE_BYTES: usize = 16 * 1024 * 1024;

pub type ClientId = u64;
pub type PortId = u64;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ChannelKey {
    pub origin: String,
    pub top_level_site: String,
    pub name: String,
}

impl ChannelKey {
    pub fn new(origin: impl Into<String>, top_level_site: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            origin: origin.into(),
            top_level_site: top_level_site.into(),
            name: name.into(),
        }
    }
    fn valid(&self) -> bool {
        !self.origin.is_empty()
            && !self.top_level_site.is_empty()
            && self.origin.len() <= MAX_ORIGIN_BYTES
            && self.top_level_site.len() <= MAX_ORIGIN_BYTES
            && self.name.len() <= MAX_NAME_BYTES
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessagingError {
    Invalid,
    Limit,
    NotFound,
    Stale,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BroadcastDelivery {
    pub client: ClientId,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortDelivery {
    pub client: ClientId,
    pub port: PortId,
    pub payload: Vec<u8>,
}

#[derive(Default)]
struct ClientState {
    channels: BTreeSet<ChannelKey>,
    ports: BTreeSet<PortId>,
}

#[derive(Default)]
struct PortState {
    peer: PortId,
    owner: ClientId,
    queue: VecDeque<Vec<u8>>,
    started: bool,
    closed: bool,
}

/// One router per browser. The browser adapter mints client ids (one per
/// document/agent) and translates deliveries into `message` events.
#[derive(Default)]
pub struct MessageRouter {
    next_port: PortId,
    clients: BTreeMap<ClientId, ClientState>,
    channels: BTreeMap<ChannelKey, BTreeSet<ClientId>>,
    ports: BTreeMap<PortId, PortState>,
}

impl MessageRouter {
    pub fn new() -> Self {
        Self::default()
    }

    fn client(&mut self, client: ClientId) -> Result<&mut ClientState, MessagingError> {
        if client == 0 {
            return Err(MessagingError::Invalid);
        }
        if !self.clients.contains_key(&client) {
            if self.clients.len() >= MAX_CLIENTS {
                return Err(MessagingError::Limit);
            }
            self.clients.insert(client, ClientState::default());
        }
        Ok(self.clients.get_mut(&client).unwrap())
    }

    /// Subscribe one client to a named channel. Subscribing twice is a no-op.
    pub fn subscribe(&mut self, key: ChannelKey, client: ClientId) -> Result<(), MessagingError> {
        if !key.valid() {
            return Err(MessagingError::Invalid);
        }
        self.client(client)?;
        if self
            .channels
            .get(&key)
            .is_some_and(|set| set.contains(&client))
        {
            return Ok(());
        }
        if (!self.channels.contains_key(&key) && self.channels.len() >= MAX_CHANNELS)
            || self.clients[&client].channels.len() >= MAX_CHANNELS_PER_CLIENT
        {
            return Err(MessagingError::Limit);
        }
        self.channels.entry(key.clone()).or_default().insert(client);
        self.clients.get_mut(&client).unwrap().channels.insert(key);
        Ok(())
    }

    pub fn unsubscribe(&mut self, key: &ChannelKey, client: ClientId) -> bool {
        let removed = self
            .channels
            .get_mut(key)
            .map(|set| set.remove(&client))
            .unwrap_or(false);
        if removed {
            if let Some(state) = self.clients.get_mut(&client) {
                state.channels.remove(key);
            }
            if self.channels.get(key).is_some_and(|set| set.is_empty()) {
                self.channels.remove(key);
            }
        }
        removed
    }

    /// Deliver to every other subscriber of the channel. The sender must be a
    /// subscriber; per Web Messaging the sender is never delivered to itself.
    pub fn broadcast(
        &self,
        key: &ChannelKey,
        sender: ClientId,
        payload: Vec<u8>,
    ) -> Result<Vec<BroadcastDelivery>, MessagingError> {
        if !key.valid() || sender == 0 || payload.len() > MAX_MESSAGE_BYTES {
            return Err(MessagingError::Invalid);
        }
        let subscribers = self.channels.get(key).ok_or(MessagingError::NotFound)?;
        if !subscribers.contains(&sender) {
            return Err(MessagingError::Stale);
        }
        Ok(subscribers
            .iter()
            .copied()
            .filter(|client| *client != sender)
            .map(|client| BroadcastDelivery {
                client,
                payload: payload.clone(),
            })
            .collect())
    }

    /// Mint an entangled pair of ports owned by `owner`. Both start paused.
    pub fn create_pair(&mut self, owner: ClientId) -> Result<(PortId, PortId), MessagingError> {
        if self.ports.len() + 2 > MAX_PORTS {
            return Err(MessagingError::Limit);
        }
        self.client(owner)?;
        if self.clients[&owner].ports.len() + 2 > MAX_PORTS_PER_CLIENT {
            return Err(MessagingError::Limit);
        }
        let a = self.next_port.checked_add(1).ok_or(MessagingError::Limit)?;
        let b = a.checked_add(1).ok_or(MessagingError::Limit)?;
        self.next_port = b;
        self.ports.insert(
            a,
            PortState {
                peer: b,
                owner,
                ..PortState::default()
            },
        );
        self.ports.insert(
            b,
            PortState {
                peer: a,
                owner,
                ..PortState::default()
            },
        );
        let state = self.clients.get_mut(&owner).unwrap();
        state.ports.insert(a);
        state.ports.insert(b);
        Ok((a, b))
    }

    /// Begin delivery on a port, flushing any queued messages to its owner.
    pub fn start(&mut self, port: PortId, client: ClientId) -> Result<Vec<PortDelivery>, MessagingError> {
        let state = self.ports.get_mut(&port).ok_or(MessagingError::NotFound)?;
        if state.owner != client || state.closed {
            return Err(MessagingError::Stale);
        }
        state.started = true;
        let queued: Vec<Vec<u8>> = state.queue.drain(..).collect();
        Ok(queued
            .into_iter()
            .map(|payload| PortDelivery {
                client,
                port,
                payload,
            })
            .collect())
    }

    /// Post from `client` to the port's entangled peer. A started peer receives
    /// the message immediately; a paused peer queues it (bounded FIFO).
    pub fn post(
        &mut self,
        port: PortId,
        client: ClientId,
        payload: Vec<u8>,
    ) -> Result<Option<PortDelivery>, MessagingError> {
        if payload.len() > MAX_MESSAGE_BYTES {
            return Err(MessagingError::Invalid);
        }
        let peer = {
            let state = self.ports.get(&port).ok_or(MessagingError::NotFound)?;
            if state.owner != client || state.closed {
                return Err(MessagingError::Stale);
            }
            state.peer
        };
        let peer_state = self.ports.get_mut(&peer).ok_or(MessagingError::NotFound)?;
        if peer_state.closed {
            return Ok(None);
        }
        if peer_state.started {
            return Ok(Some(PortDelivery {
                client: peer_state.owner,
                port: peer,
                payload,
            }));
        }
        if peer_state.queue.len() >= MAX_QUEUE {
            return Err(MessagingError::Limit);
        }
        peer_state.queue.push_back(payload);
        Ok(None)
    }

    /// Move a port to another owner, preserving queued messages.
    pub fn transfer(
        &mut self,
        port: PortId,
        from: ClientId,
        to: ClientId,
    ) -> Result<(), MessagingError> {
        self.client(to)?;
        if self.clients[&to].ports.len() >= MAX_PORTS_PER_CLIENT {
            return Err(MessagingError::Limit);
        }
        let state = self.ports.get_mut(&port).ok_or(MessagingError::NotFound)?;
        if state.owner != from || state.closed {
            return Err(MessagingError::Stale);
        }
        state.owner = to;
        if let Some(previous) = self.clients.get_mut(&from) {
            previous.ports.remove(&port);
        }
        self.clients.get_mut(&to).unwrap().ports.insert(port);
        Ok(())
    }

    /// Close one end. The peer is marked closed and returned so the browser can
    /// send it a close notification.
    pub fn close(&mut self, port: PortId, client: ClientId) -> Result<PortId, MessagingError> {
        let peer = {
            let state = self.ports.get_mut(&port).ok_or(MessagingError::NotFound)?;
            if state.owner != client || state.closed {
                return Err(MessagingError::Stale);
            }
            state.closed = true;
            state.queue.clear();
            state.peer
        };
        if let Some(peer_state) = self.ports.get_mut(&peer) {
            peer_state.closed = true;
            peer_state.queue.clear();
        }
        Ok(peer)
    }

    /// Drop every channel and port owned by a disconnecting or crashed client.
    /// Returns the peer ports that must also be closed.
    pub fn disconnect(&mut self, client: ClientId) -> Vec<PortId> {
        let Some(state) = self.clients.remove(&client) else {
            return Vec::new();
        };
        for key in &state.channels {
            if let Some(set) = self.channels.get_mut(key) {
                set.remove(&client);
                if set.is_empty() {
                    self.channels.remove(key);
                }
            }
        }
        let mut peers = Vec::new();
        for port in state.ports {
            if let Some(port_state) = self.ports.remove(&port) {
                if !port_state.closed {
                    if let Some(peer_state) = self.ports.get_mut(&port_state.peer) {
                        peer_state.closed = true;
                        peer_state.queue.clear();
                        peers.push(port_state.peer);
                    }
                }
            }
        }
        peers
    }

    pub fn channel_count(&self) -> usize {
        self.channels.len()
    }
    pub fn port_count(&self) -> usize {
        self.ports.len()
    }
    pub fn client_count(&self) -> usize {
        self.clients.len()
    }
    pub fn queued(&self, port: PortId) -> usize {
        self.ports.get(&port).map_or(0, |state| state.queue.len())
    }
    pub fn is_started(&self, port: PortId) -> bool {
        self.ports.get(&port).is_some_and(|state| state.started)
    }
    pub fn is_closed(&self, port: PortId) -> bool {
        self.ports.get(&port).is_none_or(|state| state.closed)
    }
    pub fn owner_of(&self, port: PortId) -> Option<ClientId> {
        self.ports.get(&port).map(|state| state.owner)
    }
}

// ---------------------------------------------------------------------------
// Process endpoint: the browser owns one router and serves renderer requests.
use crate::protocol::{
    BroadcastPost, ChannelRegistration, MessageDelivery, MessagingClient, MessagingReceiver,
    MessagingService, PortClose, PortPair, PortPost, PortStart, StoragePartition,
};
use amazing_ipc::{Connection, Error as IpcError};
use std::sync::{Arc, Mutex};

fn channel_key(partition: StoragePartition, name: String) -> ChannelKey {
    ChannelKey::new(partition.origin, partition.top_level_site, name)
}

fn ipc(error: MessagingError) -> IpcError {
    IpcError::Remote(format!("messaging: {error:?}"))
}

/// A browser-bound endpoint. It holds the single router; the browser feeds it
/// the renderers' requests and drains deliveries back to their realms.
pub struct MessagingEndpoint {
    router: Mutex<MessageRouter>,
}

impl MessagingEndpoint {
    pub fn new(router: MessageRouter) -> Self {
        Self {
            router: Mutex::new(router),
        }
    }
    pub fn into_router(self) -> MessageRouter {
        self.router.into_inner().unwrap()
    }
}

impl Default for MessagingEndpoint {
    fn default() -> Self {
        Self::new(MessageRouter::new())
    }
}

impl MessagingService for MessagingEndpoint {
    fn subscribe(&self, request: ChannelRegistration) -> Result<bool, IpcError> {
        let key = channel_key(request.partition, request.name);
        self.router
            .lock()
            .unwrap()
            .subscribe(key, request.client)
            .map(|()| true)
            .map_err(ipc)
    }
    fn unsubscribe(&self, request: ChannelRegistration) -> Result<bool, IpcError> {
        let key = channel_key(request.partition, request.name);
        Ok(self.router.lock().unwrap().unsubscribe(&key, request.client))
    }
    fn broadcast(&self, request: BroadcastPost) -> Result<Vec<MessageDelivery>, IpcError> {
        let key = channel_key(request.partition, request.name);
        let deliveries = self
            .router
            .lock()
            .unwrap()
            .broadcast(&key, request.client, request.payload.0)
            .map_err(ipc)?;
        Ok(deliveries
            .into_iter()
            .map(|delivery| MessageDelivery {
                client: delivery.client,
                port: 0,
                payload: ByteBuf(delivery.payload),
            })
            .collect())
    }
    fn create_ports(&self, request: MessagingClient) -> Result<PortPair, IpcError> {
        let (a, b) = self
            .router
            .lock()
            .unwrap()
            .create_pair(request.client)
            .map_err(ipc)?;
        Ok(PortPair { a, b })
    }
    fn start_port(&self, request: PortStart) -> Result<Vec<MessageDelivery>, IpcError> {
        let deliveries = self
            .router
            .lock()
            .unwrap()
            .start(request.port, request.client)
            .map_err(ipc)?;
        Ok(deliveries
            .into_iter()
            .map(|delivery| MessageDelivery {
                client: delivery.client,
                port: delivery.port,
                payload: ByteBuf(delivery.payload),
            })
            .collect())
    }
    fn post_port(&self, request: PortPost) -> Result<Option<MessageDelivery>, IpcError> {
        let mut router = self.router.lock().unwrap();
        let peer_owner = router.owner_of(request.port);
        let delivery = router
            .post(request.port, request.client, request.payload.0)
            .map_err(ipc)?;
        if let Some(target) = peer_owner {
            for port in request.transfer {
                router.transfer(port, request.client, target).map_err(ipc)?;
            }
        }
        Ok(delivery.map(|delivery| MessageDelivery {
            client: delivery.client,
            port: delivery.port,
            payload: ByteBuf(delivery.payload),
        }))
    }
    fn close_port(&self, request: PortClose) -> Result<u64, IpcError> {
        self.router
            .lock()
            .unwrap()
            .close(request.port, request.client)
            .map_err(ipc)
    }
    fn disconnect(&self, request: MessagingClient) -> Result<Vec<u64>, IpcError> {
        Ok(self.router.lock().unwrap().disconnect(request.client))
    }
}

use crate::protocol::ByteBuf;

/// Bind the browser's messaging endpoint on interface 15.
pub fn serve_messaging(connection: &Arc<Connection>, router: MessageRouter) {
    connection.bind(15, MessagingReceiver(MessagingEndpoint::new(router)));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> ChannelKey {
        ChannelKey::new("https://a.test", "https://a.test", "updates")
    }

    #[test]
    fn broadcast_excludes_sender_and_keeps_fifo_per_client() {
        let mut router = MessageRouter::new();
        router.subscribe(key(), 1).unwrap();
        router.subscribe(key(), 2).unwrap();
        router.subscribe(key(), 3).unwrap();
        let first = router.broadcast(&key(), 1, b"one".to_vec()).unwrap();
        assert_eq!(first.len(), 2);
        assert!(first.iter().all(|d| d.client != 1 && d.payload == b"one"));
        router.unsubscribe(&key(), 2);
        let second = router.broadcast(&key(), 1, b"two".to_vec()).unwrap();
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].client, 3);
        // A non-subscriber cannot send.
        assert_eq!(
            router.broadcast(&key(), 9, vec![]).unwrap_err(),
            MessagingError::Stale
        );
    }

    #[test]
    fn ports_queue_until_start_then_deliver_in_order() {
        let mut router = MessageRouter::new();
        let (a, b) = router.create_pair(1).unwrap();
        assert!(router.post(a, 1, b"first".to_vec()).unwrap().is_none());
        assert!(router.post(a, 1, b"second".to_vec()).unwrap().is_none());
        assert_eq!(router.queued(b), 2);
        let flushed = router.start(b, 1).unwrap();
        assert_eq!(
            flushed.iter().map(|d| d.payload.as_slice()).collect::<Vec<_>>(),
            vec![b"first".as_slice(), b"second".as_slice()]
        );
        let live = router.post(a, 1, b"third".to_vec()).unwrap().unwrap();
        assert_eq!(live.port, b);
        assert_eq!(live.payload, b"third");
    }

    #[test]
    fn transfer_close_and_disconnect_are_bounded() {
        let mut router = MessageRouter::new();
        let (a, b) = router.create_pair(1).unwrap();
        router.transfer(b, 1, 2).unwrap();
        assert!(router
            .post(b, 1, b"x".to_vec())
            .is_err());
        let peer = router.close(a, 1).unwrap();
        assert_eq!(peer, b);
        assert!(router.is_closed(b));

        let (c, d) = router.create_pair(3).unwrap();
        let peers = router.disconnect(3);
        assert_eq!(peers, vec![d]);
        assert!(router.is_closed(c) && router.is_closed(d));
        assert_eq!(router.client_count(), 2);
    }

    #[test]
    fn messaging_service_round_trips_over_ipc() {
        use crate::protocol::{
            BroadcastPost, ChannelRegistration, MessageDelivery, MessagingClient, MessagingProxy,
            PortClose, PortPost, PortStart,
        };
        let (a, b) = amazing_ipc::memory_pair();
        let browser = Connection::new(a);
        let renderer = Connection::new(b);
        serve_messaging(&browser, MessageRouter::new());
        let proxy = MessagingProxy::new(renderer, std::time::Duration::from_secs(2));
        let partition = StoragePartition {
            origin: "https://a.test".into(),
            top_level_site: "https://a.test".into(),
            bucket: "default".into(),
        };
        let registration = |client| ChannelRegistration {
            client,
            partition: partition.clone(),
            name: "updates".into(),
        };
        assert!(proxy.subscribe(registration(1)).unwrap());
        assert!(proxy.subscribe(registration(2)).unwrap());
        let delivered: Vec<MessageDelivery> = proxy
            .broadcast(BroadcastPost {
                client: 1,
                partition: partition.clone(),
                name: "updates".into(),
                payload: ByteBuf(b"hello".to_vec()),
            })
            .unwrap();
        assert_eq!(delivered.len(), 1);
        assert_eq!(delivered[0].client, 2);
        assert_eq!(delivered[0].port, 0);
        assert_eq!(delivered[0].payload.0, b"hello");

        let pair = proxy
            .create_ports(MessagingClient { client: 1 })
            .unwrap();
        assert!(proxy
            .post_port(PortPost {
                client: 1,
                port: pair.a,
                payload: ByteBuf(b"queued".to_vec()),
                transfer: Vec::new(),
            })
            .unwrap()
            .is_none());
        let flushed = proxy
            .start_port(PortStart {
                client: 1,
                port: pair.b,
            })
            .unwrap();
        assert_eq!(flushed.len(), 1);
        assert_eq!(flushed[0].port, pair.b);
        assert_eq!(flushed[0].payload.0, b"queued");
        assert_eq!(
            proxy
                .close_port(PortClose {
                    client: 1,
                    port: pair.a,
                })
                .unwrap(),
            pair.b
        );
    }
}
