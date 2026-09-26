//! Process boundary messages. All externally supplied fields decode through
//! amazing-ipc's version and allocation limits.
pub use amazing_ipc::codec::ByteBuf;
use amazing_ipc::{interface, wire_enum, wire_struct};

wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct Viewport {width:u32, height:u32}}
wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct Navigate {url:String, html_bytes:ByteBuf, viewport:Viewport}}
wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct Frame {sequence:u64, width:u32, height:u32, rgba:ByteBuf}}
wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct FrameReady {sequence:u64, shared:Option<SharedFrameReady>}}
wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct SharedFrameReady {generation:u64, width:u32, height:u32, stride:u32, format:u32, byte_len:u64, handle_id:u64}}
wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct ReleaseFrame {generation:u64}}
wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct AcquireFrame {generation:u64}}
wire_enum! {#[derive(Debug, Clone, PartialEq, Eq)] pub enum AcquireResult {Transferred(())=1, Superseded(u64)=2, Busy(())=3}}
wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct SadTab {pid:u32, reason:String}}
wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct Point {x:i32, y:i32}}
wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct ScrollDelta {x:i32, y:i32}}
wire_enum! {#[derive(Debug, Clone, PartialEq, Eq)] pub enum InputEvent {Click(Point)=1, Scroll(ScrollDelta)=2, Key(String)=3}}
wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct InputAck {sequence:u64}}
wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct StartRequest {url:String, base_url:String}}
wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct Started {id:u64}}
wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct RequestId {id:u64}}
wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct ResponseHead {status:u16, url:String}}
wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct BodyChunk {bytes:ByteBuf}}
wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct Completion {error:Option<String>}}
wire_enum! {#[derive(Debug, Clone, PartialEq, Eq)] pub enum NetworkEvent {Pending(())=0, Head(ResponseHead)=1, Body(BodyChunk)=2, Complete(Completion)=3}}

interface! {pub interface RendererControls(7) => RendererProxy, RendererReceiver {
    fn navigate(request:Navigate) -> Frame = 1;
    fn input(event:InputEvent) -> InputAck = 2;
    fn navigate_shared(request:Navigate) -> SharedFrameReady = 3;
    fn release_frame(request:ReleaseFrame) -> bool = 4;
    fn acquire_frame(request:AcquireFrame) -> AcquireResult = 5;
    fn frame_received(request:AcquireFrame) -> bool = 6;
}}
interface! {pub interface NetworkRequests(8) => NetworkProxy, NetworkReceiver {
    fn start_request(request:StartRequest) -> Started = 1;
    fn poll_request(request:RequestId) -> NetworkEvent = 2;
    fn cancel_request(request:RequestId) -> bool = 3;
}}

/// Version 3 adds redirect metadata and browser-only subresource starts. It
/// remains isolated from the IPC-2 demo interface (8). The browser
/// constructs NetworkContext; page input must never supply its trust fields.
pub const NETWORK_VERSION: u16 = 5;
pub const NETWORK_MAX_REQUEST_BODY: usize = 1024 * 1024;
pub const NETWORK_MAX_RESPONSE_BODY: u32 = 32 * 1024 * 1024;
pub const NETWORK_CHUNK: u32 = 32 * 1024;
wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct HeaderField {name:String, value:String}}
wire_enum! {#[derive(Debug, Clone, PartialEq, Eq)] pub enum WireOrigin {Tuple(String)=1, Opaque(u128)=2}}
wire_enum! {#[derive(Debug, Clone, PartialEq, Eq)] pub enum WireCookieSite {NoClient(())=1, Origin(WireOrigin)=2, CrossSite(())=3}}
wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct NetworkContext {
    origin:WireOrigin, cookie_site:WireCookieSite, top_level_origin:Option<WireOrigin>,
    ancestor_origins:Vec<WireOrigin>, sandbox_flags:u32, now_unix:i64,
    restricts_mixed_content:bool, image_set_initiator:bool,
    top_level_navigation:bool, embedder_policy:String,
    no_cors_media_state:String, csp_headers:Vec<HeaderField>
}}
wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct NetworkStart {
    version:u16, client_id:u64, epoch:u64, url:String, base_url:String, method:String,
    headers:Vec<HeaderField>, body_present:bool, body:ByteBuf, destination:String,
    mode:String, credentials:String, redirect:String, cache:String,
    referrer:String, referrer_policy:String, response_limit:u32,
    navigation_reload_or_history:bool, navigation_client_reserved_id:Option<u64>,
    context:NetworkContext
}}
wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct NetworkStarted {id:u64, client_id:u64, epoch:u64}}
wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct NetworkRead {id:u64, client_id:u64, epoch:u64, credit:u32}}
wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct NetworkCancel {id:u64, client_id:u64, epoch:u64}}
wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct NetworkEpoch {client_id:u64, epoch:u64}}
wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct NetworkHead {
    id:u64, client_id:u64, epoch:u64, status:u16, url:String, reason:String,
    headers:Vec<HeaderField>, response_type:String, redirected:bool,
    url_list:Vec<String>, redirect_tainted:bool
}}
wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct NetworkBody {id:u64, epoch:u64, bytes:ByteBuf}}
wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct NetworkComplete {id:u64, epoch:u64, error:Option<String>}}
wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct DocumentCookie {
    client_id:u64, epoch:u64, document_url:String,
    top_level_origin:WireOrigin, now_unix:i64, assignment:Option<String>
}}
wire_enum! {#[derive(Debug, Clone, PartialEq, Eq)] pub enum NetworkReply {
    Pending(())=0, Head(NetworkHead)=1, Body(NetworkBody)=2, Complete(NetworkComplete)=3
}}
interface! {pub interface NetworkRequestsV2(10) => NetworkProxyV2, NetworkReceiverV2 {
    fn start(request:NetworkStart) -> NetworkStarted = 1;
    fn read(request:NetworkRead) -> NetworkReply = 2;
    fn cancel(request:NetworkCancel) -> NetworkComplete = 3;
    fn cancel_epoch(request:NetworkEpoch) -> Vec<NetworkComplete> = 4;
    fn subresource(request:NetworkStart) -> NetworkStarted = 5;
    fn document_cookie(request:DocumentCookie) -> String = 6;
}}
interface! {pub interface BrowserEvents(9) => BrowserEventsProxy, BrowserEventsReceiver {
    fn frame_ready(event:FrameReady) -> () = 1;
}}

// The byte blob is the IPC-owned encoding of CompositorFrameSnapshot, including
// its versioned ResourceDeltaSnapshot and retained property trees.
wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct GpuFrame {sequence:u64, snapshot:ByteBuf}}
interface! {pub interface GpuPresentation(11) => GpuProxy, GpuReceiver {
    fn submit(frame:GpuFrame) -> Frame = 1;
}}

// Utility output is an immutable ADR 0007 mapping. `payload_len` excludes
// padding in the final RGBA row; image dimensions or audio metadata describe
// the decoded content, not the shape of the transport mapping.
wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct DecodeRequest {kind:u8, bytes:ByteBuf}}
wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct DecodedReady {kind:u8, width:u32, height:u32, sample_rate:u32, channels:u16, payload_len:u32, shared:SharedFrameReady}}
interface! {pub interface UtilityDecoder(12) => UtilityProxy, UtilityReceiver {
    fn decode(request:DecodeRequest) -> DecodedReady = 1;
    fn acquire(request:AcquireFrame) -> AcquireResult = 2;
    fn received(request:AcquireFrame) -> bool = 3;
    fn release(request:ReleaseFrame) -> bool = 4;
}}

// Only the browser owns the Storage connection and chooses partition keys.
wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct StoragePartition {origin:String, top_level_site:String, bucket:String}}
wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct StorageEntry {partition:StoragePartition, namespace:String, key:ByteBuf}}
wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct StoragePut {entry:StorageEntry, value:ByteBuf}}
wire_enum! {#[derive(Debug, Clone, PartialEq, Eq)] pub enum LockMode {Shared(())=1, Exclusive(())=2}}
wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct LockOptions {if_available:bool, steal:bool}}
wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct LockRequest {
    partition:StoragePartition, client:u64, name:String, mode:LockMode, options:LockOptions
}}
wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct LockOutcome {
    id:Option<u64>, granted:Vec<u64>, stolen:Vec<u64>
}}
wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct LockRelease {client:u64, id:u64}}
wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct LockClient {client:u64}}
wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct LockQuery {partition:StoragePartition}}
wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct LockInfo {
    id:u64, client:u64, name:String, mode:LockMode
}}
wire_struct! {#[derive(Debug, Clone, Default, PartialEq, Eq)] pub struct LockSnapshot {
    held:Vec<LockInfo>, pending:Vec<LockInfo>
}}
interface! {pub interface StorageService(13) => StorageProxy, StorageReceiver {
    fn put(request:StoragePut) -> bool = 1;
    fn get(request:StorageEntry) -> Option<ByteBuf> = 2;
    fn delete(request:StorageEntry) -> bool = 3;
    fn checkpoint(request:()) -> bool = 4;
    fn lock_request(request:LockRequest) -> LockOutcome = 5;
    fn lock_release(request:LockRelease) -> Vec<u64> = 6;
    fn lock_release_client(request:LockClient) -> Vec<u64> = 7;
    fn lock_query(request:LockQuery) -> LockSnapshot = 8;
}}

// Browser-minted document tokens and epochs gate every lifecycle transition.
// The renderer reports the observable HTML event order in its acknowledgment.
wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct HistoryActivate {document:u64, epoch:u64, history_state:Option<ByteBuf>}}
wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct HistoryFreeze {document:u64, epoch:u64, deadline_ms:u64}}
wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct HistoryRestore {document:u64, epoch:u64}}
wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct HistoryDiscard {document:u64, epoch:u64}}
wire_enum! {#[derive(Debug, Clone, PartialEq, Eq)] pub enum HistoryLifecycleEvent {
    PageHidePersisted(())=1, VisibilityHidden(())=2,
    VisibilityVisible(())=3, PageShowPersisted(())=4
}}
wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct HistoryAck {
    document:u64, epoch:u64, quiescent:bool, events:Vec<HistoryLifecycleEvent>
}}
interface! {pub interface HistoryLifecycle(14) => HistoryProxy, HistoryReceiver {
    fn activate(request:HistoryActivate) -> bool = 1;
    fn freeze(request:HistoryFreeze) -> HistoryAck = 2;
    fn restore(request:HistoryRestore) -> HistoryAck = 3;
    fn discard(request:HistoryDiscard) -> bool = 4;
}}

// Cross-document messaging. The browser mints every client and port id; a
// renderer only names its own client. Payloads are bounded structured-clone
// graphs; port `0` in a delivery means a BroadcastChannel message.
pub const MESSAGING_VERSION: u16 = 1;
wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct MessagingClient {client:u64}}
wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct ChannelRegistration {
    client:u64, partition:StoragePartition, name:String
}}
wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct BroadcastPost {
    client:u64, partition:StoragePartition, name:String, payload:ByteBuf
}}
wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct MessageDelivery {
    client:u64, port:u64, payload:ByteBuf
}}
wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct PortPair {a:u64, b:u64}}
wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct PortPost {
    client:u64, port:u64, payload:ByteBuf, transfer:Vec<u64>
}}
wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct PortStart {client:u64, port:u64}}
wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct PortClose {client:u64, port:u64}}
interface! {pub interface MessagingService(15) => MessagingProxy, MessagingReceiver {
    fn subscribe(request:ChannelRegistration) -> bool = 1;
    fn unsubscribe(request:ChannelRegistration) -> bool = 2;
    fn broadcast(request:BroadcastPost) -> Vec<MessageDelivery> = 3;
    fn create_ports(request:MessagingClient) -> PortPair = 4;
    fn start_port(request:PortStart) -> Vec<MessageDelivery> = 5;
    fn post_port(request:PortPost) -> Option<MessageDelivery> = 6;
    fn close_port(request:PortClose) -> u64 = 7;
    fn disconnect(request:MessagingClient) -> Vec<u64> = 8;
}}

impl Frame {
    pub fn validate(&self) -> Result<(), amazing_ipc::Error> {
        let len = usize::try_from(self.width)
            .ok()
            .and_then(|w| {
                usize::try_from(self.height)
                    .ok()
                    .and_then(|h| w.checked_mul(h))
            })
            .and_then(|pixels| pixels.checked_mul(4));
        if len != Some(self.rgba.0.len()) {
            return Err(amazing_ipc::Error::Codec(
                amazing_ipc::codec::Error::Invalid,
            ));
        }
        Ok(())
    }
}

impl SharedFrameReady {
    pub fn metadata(
        &self,
    ) -> Result<amazing_ipc_platform::FrameMetadata, amazing_ipc_platform::Error> {
        let metadata = amazing_ipc_platform::FrameMetadata {
            generation: self.generation,
            width: self.width,
            height: self.height,
            stride: self.stride,
            format: self.format,
            byte_len: self.byte_len,
            handle_id: self.handle_id,
        };
        metadata.validate()?;
        Ok(metadata)
    }
}
impl From<amazing_ipc_platform::FrameMetadata> for SharedFrameReady {
    fn from(value: amazing_ipc_platform::FrameMetadata) -> Self {
        Self {
            generation: value.generation,
            width: value.width,
            height: value.height,
            stride: value.stride,
            format: value.format,
            byte_len: value.byte_len,
            handle_id: value.handle_id,
        }
    }
}
