//! Process boundary messages. All externally supplied fields decode through
//! amazing-ipc's version and allocation limits.
use amazing_ipc::codec::ByteBuf;
use amazing_ipc::{interface, wire_enum, wire_struct};

wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct Viewport {width:u32, height:u32}}
wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct Navigate {url:String, html_bytes:ByteBuf, viewport:Viewport}}
wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct Frame {sequence:u64, width:u32, height:u32, rgba:ByteBuf}}
wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct FrameReady {sequence:u64}}
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
}}
interface! {pub interface NetworkRequests(8) => NetworkProxy, NetworkReceiver {
    fn start_request(request:StartRequest) -> Started = 1;
    fn poll_request(request:RequestId) -> NetworkEvent = 2;
    fn cancel_request(request:RequestId) -> bool = 3;
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
