//! Original receiving-global settings for an ordinary native request.
//! These owned scalar settings survive only that request, never mint a global.
mod embedding_owner;
pub use embedding_owner::{NavigationEmbeddingElement,NavigationEmbeddingOwnerId,PreparedNavigationEmbeddingOwner};
pub(crate) use embedding_owner::NavigationEmbeddingOwnerSnapshot;
use super::*;
use crate::{EmbedderPolicy, EmbedderPolicyContainer, ReceivingDocumentClient, Destination};

#[derive(Debug)]
pub enum CoepRequestStartError {
    Reporting(ReportError),
    Fetch(crate::api::ApiError),
}
/// Move-only original receiving-global authority captured before request IO.
/// No public constructor, Clone, URL registry or fresh-child transfer exists.
/// ```compile_fail
/// use amazing_fetch::reporting::CoepRequestReportSettings;
/// fn cannot_duplicate(settings: CoepRequestReportSettings) { let _ = settings.clone(); }
/// ```
#[must_use]
pub struct CoepRequestReportSettings {
    service: u64,
    document: ReportingDocument,
    saved: std::sync::Arc<Document>,
    container: std::sync::Arc<EmbedderPolicyContainer>,
    _credits: std::sync::Arc<prepared::PreparedCredits>,
    graph_receiver: Option<std::sync::Arc<GraphReceiver>>,
    _loan_credits: Option<prepared::PreparedCredits>,
    navigation_owner_id: Option<NavigationEmbeddingOwnerId>,
}
impl NetworkService {
    pub fn prepare_coep_request_reporting<Identity>(
        &mut self, document: ReportingDocument,
        receiving: &ReceivingDocumentClient<Identity>,
        container: &EmbedderPolicyContainer,
    ) -> Result<CoepRequestReportSettings, ReportError> {
        let saved = self.reporting.document(document)?;
        if saved.creation_url != receiving.creation_url || saved.origin != receiving.origin
            || receiving.context.security.embedder_policy != container.value {
            return Err(ReportError::InvalidReport);
        }
        // Bound scalar settings custody before cloning. This shares the existing
        // application prepared-settings budget; it is not provider memory usage.
        let mut bytes = std::mem::size_of::<Document>() + std::mem::size_of::<EmbedderPolicyContainer>();
        let mut add = |n: usize| -> Result<(), ReportError> {
            bytes = bytes.checked_add(n).filter(|n| *n <= MAX_REPORT_BYTES).ok_or(ReportError::Limit)?; Ok(())
        };
        add(saved.creation_url.serialize().len())?; add(saved.origin.serialize().len())?;
        for (name,url) in &saved.endpoints { add(std::mem::size_of::<(String,HttpUrl)>())?; add(name.len())?; add(url.serialize().len())?; }
        for name in saved.failures.keys() { add(std::mem::size_of::<(String,u64)>())?; add(name.len())?; }
        if let crate::cookies::CookieSite::Origin(origin) = &saved.context.cookie_site {add(origin.serialize().len())?;}
        if let Some(crate::Site::Tuple {scheme,domain}) = &saved.context.partition_key {add(scheme.len())?;add(domain.len())?;}
        for origin in &saved.context.frame_ancestors {add(std::mem::size_of::<Origin>())?;add(origin.serialize().len())?;}
        for policy in &saved.context.csp.policies {
            add(std::mem::size_of::<crate::CspPolicy>())?;add(policy.original_policy.len())?;
            add(policy.original_policy_units.len().checked_mul(2).ok_or(ReportError::Limit)?)?;
            for directive in &policy.directives {
                add(std::mem::size_of::<crate::CspDirective>())?;add(directive.name.len())?;
                for source in &directive.sources {add(std::mem::size_of::<String>())?;add(source.len())?;}
            }
        }
        for name in [&container.reporting_endpoint,&container.report_only_reporting_endpoint].into_iter().flatten() {add(name.len())?;}
        let quota=&self.reporting.prepared_bytes;
        let mut used=quota.load(Ordering::Acquire);
        loop { let next=used.checked_add(bytes).filter(|n|*n<=MAX_REPORT_BYTES).ok_or(ReportError::Limit)?;
            match quota.compare_exchange_weak(used,next,Ordering::AcqRel,Ordering::Acquire) {Ok(_)=>break,Err(n)=>used=n}
        }
        Ok(CoepRequestReportSettings {service:self.reporting.service,document,saved:std::sync::Arc::new(saved.clone()),container:std::sync::Arc::new(container.clone()),
            _credits:std::sync::Arc::new(prepared::PreparedCredits {quota:quota.clone(),bytes}),graph_receiver:None,_loan_credits:None,navigation_owner_id:None})
    }
    pub(crate) fn check_request_coep_reporting(
        &mut self, settings: &CoepRequestReportSettings, request: &Request,
        headers: &HeaderList, includes_credentials: bool, for_navigation: bool, now: Instant,
    ) -> Result<Vec<NativeReportSnapshot>, ReportError> {
        settings.validate(self,request)?;
        if (!for_navigation && request.mode != RequestMode::NoCors)
            || !matches!(request.url().as_parsed().scheme(),"http"|"https") {return Ok(Vec::new());}
        let check=|value| {
            let checking_origin=if for_navigation {request.embedding_owner.as_ref().map(|owner|owner.origin()).unwrap_or(&request.origin)} else {&request.origin};
            crate::security::corp_internal(checking_origin,value,request.url(),request.mode,
                headers,includes_credentials,for_navigation,&crate::psl::PublicSuffixList)
        };
        // Fetch CORP step3: explicit CORP denial is unrelated to COEP reporting.
        if check(EmbedderPolicy::UnsafeNone).is_err() {return Ok(Vec::new());}
        let mut blocked=Vec::new();
        for (value,endpoint,disposition) in [
            (settings.container.report_only_value,&settings.container.report_only_reporting_endpoint,CoepReportDisposition::Reporting),
            (settings.container.value,&settings.container.reporting_endpoint,CoepReportDisposition::Enforce),
        ] { if check(value).is_err() {blocked.push((endpoint,disposition));} }
        if blocked.is_empty() {return Ok(Vec::new());}
        let destination=match &request.embedding_owner {Some(owner)=>owner.destination(),None=>destination(request.security.destination)?};
        let urls:Vec<_>=request.url_list().iter().map(|url|url.as_parsed().clone()).collect();
        let mut reports=Vec::new();
        for (endpoint,disposition) in blocked {
                // A missing reporting endpoint still creates an observer report.
                let group=endpoint.as_deref().unwrap_or("").encode_utf16().collect::<Vec<_>>();
                let report=self.generate_saved_coep_report(settings.document,&settings.saved,
                    CoepReportBody::Corp{response_url_list:&urls,disposition,destination},&group,now,now,endpoint.is_some())?;
                reports.push(report);
        }
        Ok(reports)
    }
}
impl CoepRequestReportSettings {
    pub(crate) fn validate(&self, service:&NetworkService,request:&Request)->Result<(),ReportError> {
        if self.service!=service.reporting.service {return Err(ReportError::StaleDocument);}
        if request.embedding_owner.as_ref().map(|owner|owner.id())!=self.navigation_owner_id {return Err(ReportError::InvalidReport);}
        let (origin,value)=request.embedding_owner.as_ref().map(|owner|(owner.origin(),owner.embedder_policy()))
            .unwrap_or((&request.origin,request.security.embedder_policy));
        if &self.saved.origin!=origin || self.container.value!=value
            || self.graph_receiver.as_ref().is_some_and(|receiver|receiver.client.csp_self_origin!=request.csp_self_origin) {return Err(ReportError::InvalidReport);}
        Ok(())
    }
    pub(crate) fn document(&self)->ReportingDocument {self.document}
}
fn destination(value:Destination)->Result<&'static str,ReportError> {
    Ok(match value {Destination::Empty=>"",Destination::Document=>"document",Destination::Image=>"image",Destination::Font=>"font",
        Destination::Audio=>"audio",Destination::Video=>"video",Destination::Script=>"script",Destination::Worker=>"worker",
        Destination::SharedWorker=>"sharedworker",Destination::ServiceWorker=>"serviceworker",Destination::Style=>"style",
        // Other is not an authentic standard destination name.
        Destination::Other=>return Err(ReportError::InvalidReport)})
}

/// Original-owner settings captured separately before native child/worker
/// admission. This cannot become the fresh child's Reporting endpoint authority.
/// ```compile_fail
/// use amazing_fetch::reporting::PreparedCoepInheritanceReport;
/// fn cannot_duplicate(plan:PreparedCoepInheritanceReport) {let _=plan.clone();}
/// ```
#[must_use]
pub struct PreparedCoepInheritanceReport {settings:CoepRequestReportSettings}
#[derive(Debug)]
pub struct CoepInheritanceReports {
    pub document:ReportingDocument,
    pub check:crate::EmbedderInheritanceCheck,
    pub reports:Vec<Result<NativeReportSnapshot,ReportError>>,
}

/// Same-service graph generation, minted only by live graph admission.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CoepGraphId { service:u64, generation:u64 }
struct GraphReceiver {
    // No native Window/Document roots or caller identity are retained here.
    client:ReceivingDocumentClient<()>,
    _credits:prepared::PreparedCredits,
}
/// Original graph settings, released explicitly at graph termination. HTML
/// owns actual graph/node membership; a URL does not mint this capability.
/// ```compile_fail
/// use amazing_fetch::reporting::CoepGraphReportSettings;
/// fn cannot_duplicate(graph:CoepGraphReportSettings) {let _=graph.clone();}
/// ```
#[must_use]
pub struct CoepGraphReportSettings { id:CoepGraphId, settings:CoepRequestReportSettings }
impl CoepGraphReportSettings { pub fn id(&self)->CoepGraphId {self.id} }

pub(super) fn graph_credit(quota:&std::sync::Arc<std::sync::atomic::AtomicUsize>,bytes:usize)
    ->Result<prepared::PreparedCredits,ReportError> {
    let mut used=quota.load(Ordering::Acquire);
    loop {
        let next=used.checked_add(bytes).filter(|n|*n<=MAX_REPORT_BYTES).ok_or(ReportError::Limit)?;
        match quota.compare_exchange_weak(used,next,Ordering::AcqRel,Ordering::Acquire) {
            Ok(_)=>return Ok(prepared::PreparedCredits {quota:quota.clone(),bytes}),Err(n)=>used=n,
        }
    }
}
impl NetworkService {
    /// Capture once at actual live root graph admission. Each subsequent loan
    /// shares immutable original settings, while its own metadata consumes the
    /// existing bounded prepared-settings quota. Retirement needs no relookup.
    pub fn prepare_coep_graph_reporting<Identity>(&mut self,document:ReportingDocument,
        receiving:&ReceivingDocumentClient<Identity>,container:&EmbedderPolicyContainer,
    )->Result<CoepGraphReportSettings,ReportError> {
        let mut settings=self.prepare_coep_request_reporting(document,receiving,container)?;
        let generation=self.reporting.next_graph;
        self.reporting.next_graph=generation.checked_add(1).ok_or(ReportError::Limit)?;
        let mut bytes=std::mem::size_of::<GraphReceiver>()+std::mem::size_of::<CoepGraphReportSettings>();
        let mut add=|n:usize|->Result<(),ReportError> {
            bytes=bytes.checked_add(n).filter(|n|*n<=MAX_REPORT_BYTES).ok_or(ReportError::Limit)?;Ok(())
        };
        for url in [&receiving.creation_url,&receiving.document_url,&receiving.base_url] {add(url.serialize().len())?;}
        for origin in [&receiving.origin,&receiving.csp_self_origin] {add(origin.serialize().len())?;}
        let context=&receiving.context;
        if let crate::cookies::CookieSite::Origin(origin)=&context.cookie_site {add(origin.serialize().len())?;}
        if let Some(crate::Site::Tuple{scheme,domain})=&context.partition_key {add(scheme.len())?;add(domain.len())?;}
        for origin in &context.frame_ancestors {add(std::mem::size_of::<Origin>())?;add(origin.serialize().len())?;}
        for policy in &context.csp.policies {
            add(std::mem::size_of::<crate::CspPolicy>())?;add(policy.original_policy.len())?;
            add(policy.original_policy_units.len().checked_mul(2).ok_or(ReportError::Limit)?)?;
            for directive in &policy.directives {add(std::mem::size_of::<crate::CspDirective>())?;add(directive.name.len())?;
                for source in &directive.sources {add(std::mem::size_of::<String>())?;add(source.len())?;}
            }
        }
        let credits=graph_credit(&self.reporting.prepared_bytes,bytes)?;
        settings.graph_receiver=Some(std::sync::Arc::new(GraphReceiver {
            client:ReceivingDocumentClient {identity:(),creation_url:receiving.creation_url.clone(),
                document_url:receiving.document_url.clone(),base_url:receiving.base_url.clone(),
                origin:receiving.origin.clone(),csp_self_origin:receiving.csp_self_origin.clone(),
                context:context.clone(),referrer_policy:receiving.referrer_policy},_credits:credits,
        }));
        Ok(CoepGraphReportSettings {id:CoepGraphId {service:self.reporting.service,generation},settings})
    }
    /// The module owner must first validate the actual dependency belongs to
    /// this graph, and supply its original native client/request metadata to
    /// the ordinary request API. This operation creates no Reporting global.
    pub fn issue_coep_graph_request_settings(&mut self,graph:&mut CoepGraphReportSettings,id:CoepGraphId)
        ->Result<CoepRequestReportSettings,ReportError> {
        if id!=graph.id || id.service!=self.reporting.service {return Err(ReportError::StaleDocument);}
        let loan=graph_credit(&self.reporting.prepared_bytes,std::mem::size_of::<CoepRequestReportSettings>())?;
        let saved=&graph.settings;
        Ok(CoepRequestReportSettings {service:saved.service,document:saved.document,saved:saved.saved.clone(),
            container:saved.container.clone(),_credits:saved._credits.clone(),graph_receiver:saved.graph_receiver.clone(),
            _loan_credits:Some(loan),navigation_owner_id:None})
    }
    /// Consuming the graph prevents future dependency loans. Already admitted
    /// requests finish under their original settings; the final drop releases
    /// shared quota. Error/cancellation paths may also simply drop the graph.
    pub fn release_coep_graph_reporting(&mut self,graph:CoepGraphReportSettings,id:CoepGraphId)
        ->Result<(),ReportError> {
        if id!=graph.id || id.service!=self.reporting.service {return Err(ReportError::StaleDocument);}
        drop(graph);Ok(())
    }
}
impl NetworkService {
    pub fn prepare_coep_inheritance_reporting<Identity>(&mut self,document:ReportingDocument,
        receiving:&ReceivingDocumentClient<Identity>,container:&EmbedderPolicyContainer,
    )->Result<PreparedCoepInheritanceReport,ReportError> {
        Ok(PreparedCoepInheritanceReport{settings:self.prepare_coep_request_reporting(document,receiving,container)?})
    }
    /// The native owner supplies the actual parsed child's full container and
    /// authenticated response lineage at the real owner check. Report admission
    /// failures do not change enforcement, and original retirement does not
    /// erase previously captured scalar settings or revive the original global.
    pub fn queue_coep_inheritance_reports(&mut self,plan:PreparedCoepInheritanceReport,
        child:&EmbedderPolicyContainer,owner:crate::EmbedderPolicyOwner,
        response_urls:&[ParsedUrl],queued_at:Instant,now:Instant,
    )->Result<CoepInheritanceReports,ReportError> {
        let settings=plan.settings;
        if settings.service!=self.reporting.service {return Err(ReportError::StaleDocument);}
        let check=settings.container.check_inheritance(child,owner);
        let mut reports=Vec::new();
        for (violation,endpoint,disposition) in [
            (check.report_only_violation,&settings.container.report_only_reporting_endpoint,CoepReportDisposition::Reporting),
            (check.enforced_violation,&settings.container.reporting_endpoint,CoepReportDisposition::Enforce),
        ] {
            if !violation {continue;}
            let body=match owner {
                crate::EmbedderPolicyOwner::NestedNavigation=>CoepReportBody::Navigation {response_url_list:response_urls,disposition},
                crate::EmbedderPolicyOwner::DedicatedWorker=>CoepReportBody::WorkerInitialization {response_url_list:response_urls,disposition},
                _=>continue,
            };
            let group=endpoint.as_deref().unwrap_or("").encode_utf16().collect::<Vec<_>>();
            reports.push(self.generate_saved_coep_report(settings.document,&settings.saved,body,&group,queued_at,now,endpoint.is_some()));
        }
        Ok(CoepInheritanceReports {document:settings.document,check,reports})
    }
}
