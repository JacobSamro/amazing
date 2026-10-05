// Browser-only receiving-global registration on the existing network-role pipe.
// These records are native browser captures, never renderer/author input.
// Interface 16 does not change interface 10/version 4 or its wire-7 messages.

wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct ReceivingGlobalIdentity {
    realm_binding:u64, window:u64, document:u64, generation:u64
}}
wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct ReceivingFrameIdentity {
    frame:u64, generation:u64
}}
wire_enum! {#[derive(Debug, Clone, PartialEq, Eq)] pub enum ReceivingSite {
    Tuple(ReceivingTupleSite)=1, Opaque(u128)=2
}}
wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct ReceivingTupleSite {
    scheme:String, domain:String
}}
wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct ReceivingCspDirective {
    name:String, sources:Vec<String>
}}
wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct ReceivingCspPolicy {
    source:String, disposition:String, original_policy:String,
    original_policy_units:Vec<u16>, directives:Vec<ReceivingCspDirective>
}}
/// Exact ClientContext: neither ancestry nor partition identity is derived from URL.
wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct ReceivingClientContext {
    cookie_site:WireCookieSite, partition_key:Option<ReceivingSite>,
    frame_ancestors:Vec<WireOrigin>, restricts_mixed_content:bool,
    destination:String, image_set_initiator:bool, top_level_navigation:bool,
    embedder_policy:String, no_cors_media_state:String,
    csp:Vec<ReceivingCspPolicy>, csp_destination:String, now_unix:i64
}}
wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct ReceivingClientCapture {
    identity:ReceivingGlobalIdentity, creation_url:String, document_url:String,
    base_url:String, origin:WireOrigin, csp_self_origin:WireOrigin,
    context:ReceivingClientContext, referrer_policy:String
}}
wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct ReceivingCoep {
    value:String, reporting_endpoint:Option<String>, report_only_value:String,
    report_only_reporting_endpoint:Option<String>
}}
/// AcceptedResponse uses the server's retained real request/head, not wire headers.
/// FreshInitial is only the native creation boundary for about:blank/srcdoc.
wire_enum! {#[derive(Debug, Clone, PartialEq, Eq)] pub enum ReceivingAdmission {
    AcceptedResponse(u64)=1, FreshInitial(())=2
}}
wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct RegisterReceivingGlobal {
    session:u128, client_id:u64, epoch:u64, capture:ReceivingClientCapture, coep:ReceivingCoep,
    secure_context:bool, admission:ReceivingAdmission
}}
/// Receipt is valid only on the issuing authenticated Connection. Its number is
/// a bounded lookup key, not a portable ReportingDocument or bearer capability.
wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct RegisteredReceivingGlobal {
    session:u128, id:u64, client_id:u64, epoch:u64, identity:ReceivingGlobalIdentity
}}
wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct ReceivingNavigable {
    top_level_origin:WireOrigin, parent_origin:WireOrigin,
    ancestor_origins:Vec<WireOrigin>, sandbox_flags:u32,
    frame_csp:Vec<ReceivingCspPolicy>, referrer:String, referrer_policy:String,
    now_unix:i64
}}
wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct PrepareReceivingOwner {
    owner:RegisteredReceivingGlobal, frame:ReceivingFrameIdentity,
    parent:ReceivingNavigable, element:String
}}
wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct PreparedReceivingOwner {
    id:u64, owner:RegisteredReceivingGlobal, frame:ReceivingFrameIdentity
}}
wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct StartReceivingNavigation {
    source:RegisteredReceivingGlobal, owner:PreparedReceivingOwner,
    client_id:u64, epoch:u64, target:String, response_limit:u32
}}
wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct ContinueReceivingNavigation {
    previous:NetworkStarted, source:RegisteredReceivingGlobal,
    frame:ReceivingFrameIdentity, response_limit:u32
}}
interface! {pub interface ReceivingGlobals(16) => ReceivingGlobalsProxy, ReceivingGlobalsReceiver {
    fn register(request:RegisterReceivingGlobal) -> RegisteredReceivingGlobal = 1;
    fn prepare_owner(request:PrepareReceivingOwner) -> PreparedReceivingOwner = 2;
    fn start_navigation(request:StartReceivingNavigation) -> NetworkStarted = 3;
    fn continue_navigation(request:ContinueReceivingNavigation) -> NetworkStarted = 4;
    fn retire(request:RegisteredReceivingGlobal) -> bool = 5;
    fn release_owner(request:PreparedReceivingOwner) -> bool = 6;
    fn release_navigation(request:NetworkStarted) -> bool = 7;
    fn poll_reports(request:RegisteredReceivingGlobal) -> Vec<ReceivingReportEvent> = 8;
    fn capture_navigation(request:CaptureReceivingNavigation) -> NetworkStarted = 9;
    fn start_navigation_outcome(request:StartReceivingNavigation) -> ReceivingNavigationOutcome = 10;
    fn continue_navigation_outcome(request:ContinueReceivingNavigation) -> ReceivingNavigationOutcome = 11;
    fn finish_csp_report(request:FinishReceivingCspReport) -> ReceivingCspDelivery = 12;
}}
wire_struct! {#[derive(Debug,Clone,PartialEq,Eq)] pub struct ReceivingReportField {name:Vec<u16>,value:Vec<u16>}}
wire_struct! {#[derive(Debug,Clone,PartialEq,Eq)] pub struct ReceivingObserverReport {
    request_id:u64, report_type:Vec<u16>, report_url:Vec<u16>, body:Vec<ReceivingReportField>,
    delivery_id:Option<u64>, delivery_error:Option<String>
}}
wire_struct! {#[derive(Debug,Clone,PartialEq,Eq)] pub struct ReceivingReportCompletion {
    delivery_id:u64, route:String, status:Option<u16>, accepted:bool, error:Option<String>
}}
wire_enum! {#[derive(Debug,Clone,PartialEq,Eq)] pub enum ReceivingReportEvent {
    Observer(ReceivingObserverReport)=1, Completion(ReceivingReportCompletion)=2,
    CreationError(String)=3, Csp(ReceivingCspSnapshot)=4, NavigationFailed(ReceivingNavigationFailure)=5
}}

wire_struct! {#[derive(Debug,Clone,PartialEq,Eq)] pub struct CaptureReceivingNavigation {session:u128, request:NetworkStart}}

// These are enriched native snapshots, not reconstructable CSP check receipts.
wire_enum! {#[derive(Debug,Clone,PartialEq,Eq)] pub enum ReceivingCspValue {
    Null(())=1, NumberBits(u64)=2, String(Vec<u16>)=3
}}
wire_struct! {#[derive(Debug,Clone,PartialEq,Eq)] pub struct ReceivingCspField {
    name:Vec<u16>, value:ReceivingCspValue
}}
wire_struct! {#[derive(Debug,Clone,PartialEq,Eq)] pub struct ReceivingSourceLocation {
    source_file:String, line:u32, column:u32
}}
wire_struct! {#[derive(Debug,Clone,PartialEq,Eq)] pub struct ReceivingSourceElement {
    node:u64, generation:u64, owner_document:u64, owner_generation:u64
}}
wire_struct! {#[derive(Debug,Clone,PartialEq,Eq)] pub struct ReceivingCspSnapshot {
    source:RegisteredReceivingGlobal, request:Option<NetworkStarted>,
    element:Option<ReceivingSourceElement>, location:Option<ReceivingSourceLocation>,
    received_response:bool, phase:String, url_list:Vec<String>, effective_directive:String,
    violated_directive:String, source_directive:String, policy_index:u64,
    report_type:Vec<u16>, report_url:Vec<u16>, body:Vec<ReceivingCspField>,
    prepared_report:Option<u64>, delivery_error:Option<String>
}}
wire_struct! {#[derive(Debug,Clone,PartialEq,Eq)] pub struct ReceivingNavigationFailure {
    source:RegisteredReceivingGlobal, request:Option<NetworkStarted>, error:String,
    checked:bool, url_list:Vec<String>
}}
wire_enum! {#[derive(Debug,Clone,PartialEq,Eq)] pub enum ReceivingNavigationOutcome {
    Started(NetworkStarted)=1, Failed(ReceivingNavigationFailure)=2
}}
wire_struct! {#[derive(Debug,Clone,PartialEq,Eq)] pub struct FinishReceivingCspReport {
    source:RegisteredReceivingGlobal, report:u64
}}
wire_struct! {#[derive(Debug,Clone,PartialEq,Eq)] pub struct ReceivingCspDelivery {
    ids:Vec<u64>, error:Option<String>
}}
