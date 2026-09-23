//! Process boundary messages. All externally supplied fields decode through
//! amazing-ipc's version and allocation limits.
use amazing_ipc::codec::ByteBuf;
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

/// Version 2 is isolated from the IPC-2 demo interface (8). The browser
/// constructs NetworkContext; page input must never supply its trust fields.
pub const NETWORK_VERSION: u16 = 2;
pub const NETWORK_MAX_REQUEST_BODY: usize = 1024 * 1024;
pub const NETWORK_MAX_RESPONSE_BODY: u32 = 32 * 1024 * 1024;
pub const NETWORK_CHUNK: u32 = 32 * 1024;
wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct HeaderField {name:String, value:String}}
wire_enum! {#[derive(Debug, Clone, PartialEq, Eq)] pub enum WireOrigin {Tuple(String)=1, Opaque(u128)=2}}
wire_enum! {#[derive(Debug, Clone, PartialEq, Eq)] pub enum WireCookieSite {NoClient(())=1, Origin(WireOrigin)=2, CrossSite(())=3}}
wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct NetworkContext {
    origin:WireOrigin, cookie_site:WireCookieSite, now_unix:i64,
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
    headers:Vec<HeaderField>, response_type:String, redirected:bool
}}
wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct NetworkBody {id:u64, epoch:u64, bytes:ByteBuf}}
wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct NetworkComplete {id:u64, epoch:u64, error:Option<String>}}
wire_enum! {#[derive(Debug, Clone, PartialEq, Eq)] pub enum NetworkReply {
    Pending(())=0, Head(NetworkHead)=1, Body(NetworkBody)=2, Complete(NetworkComplete)=3
}}
interface! {pub interface NetworkRequestsV2(10) => NetworkProxyV2, NetworkReceiverV2 {
    fn start(request:NetworkStart) -> NetworkStarted = 1;
    fn read(request:NetworkRead) -> NetworkReply = 2;
    fn cancel(request:NetworkCancel) -> NetworkComplete = 3;
    fn cancel_epoch(request:NetworkEpoch) -> Vec<NetworkComplete> = 4;
}}
interface! {pub interface BrowserEvents(9) => BrowserEventsProxy, BrowserEventsReceiver {
    fn frame_ready(event:FrameReady) -> () = 1;
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
