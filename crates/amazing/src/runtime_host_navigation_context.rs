//! Native receiving Window environment, separate from navigation initiator custody.
use super::*;
use amazing_fetch::{cookies::CookieSite, service::ClientContext,
    CspDestination, CspPolicyList, Destination, EmbedderPolicy, NoCorsMediaState,
    Origin, ReceivingDocumentClient, ReceivingDocumentClientFactory, SecurityContext};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ReceivingDocumentIdentity {
    pub(crate) realm: RealmHandle,
    pub(crate) window: u64,
    pub(crate) document: NodeIdx,
    pub(crate) generation: u64,
}
#[derive(Clone)]
pub(super) struct ReceivingWindowEnvironment {
    pub(super) csp_self_origin: Origin,
    /// HTML environment setting captured at genuine Window creation. Child
    /// globals copy their container's setting; History does not retarget it.
    pub(super) top_level_creation_url: ParsedUrl,
    pub(super) restricts_mixed_content: bool,
    pub(super) embedder_policy: amazing_fetch::EmbedderPolicyContainer,
}

impl FrameRuntime {
    /// Creation association, not a claim that an uncommitted child is fully active.
    fn receiving_creation_context(&self, dom: &Dom, realm: RealmHandle)
        -> Result<(WindowContext, Vec<WindowContext>), DomError> {
        let context = self.contexts.get(&realm).ok_or(DomError::InvalidStateError)?.clone();
        let validate = |context: &WindowContext| {
            dom.node_generation(context.document) == Some(context.document_generation)
                && matches!(&dom.get(context.document).kind, NodeKind::Document)
                && dom.document_window_event_target(context.document) == Some(context.event_target_slot)
                && match (context.frame, context.frame_generation) {
                    (None, None) => true,
                    (Some(frame), Some(generation)) => dom.node_generation(frame) == Some(generation),
                    _ => false,
                }
        };
        if !validate(&context) { return Err(DomError::InvalidStateError); }
        let mut ancestors = Vec::new();
        let mut current = context.clone();
        let mut seen = HashSet::from([realm]);
        while current.parent != current.window {
            let (parent_realm, parent) = self.contexts.iter().find(|(realm, candidate)|
                candidate.window == current.parent && self.live_context(candidate, **realm))
                .ok_or(DomError::InvalidStateError)?;
            if !seen.insert(*parent_realm) || !validate(parent) { return Err(DomError::InvalidStateError); }
            ancestors.push(parent.clone());
            current = parent.clone();
        }
        if current.window != context.top || (context.frame.is_some() && ancestors.is_empty()) {
            return Err(DomError::InvalidStateError);
        }
        Ok((context, ancestors))
    }
}

// HTML §8.1.3.5 and Secure Contexts §3.2. Trust is evaluated from the saved
// environment URL, never its sandboxed opaque origin or the response endpoint.
// No additional authenticated schemes/configured trustworthy origins exist.
fn trustworthy_creation_url(url: &ParsedUrl) -> bool {
    if matches_about_url(url, "blank") || matches_about_url(url, "srcdoc")
        || url.scheme() == "data" { return true; }
    let amazing_net::url::UrlOrigin::Tuple(origin) = url.origin() else { return false; };
    if matches!(origin.scheme(), "https" | "wss") { return true; }
    match origin.host() {
        amazing_net::url::Host::Ipv4(address) => *address >> 24 == 127,
        amazing_net::url::Host::Ipv6(address) => *address == [0, 0, 0, 0, 0, 0, 0, 1],
        // The native resolver has no RFC6761 localhost guarantee yet. Secure
        // Contexts §3.1 makes this trust conditional on that actual guarantee;
        // do not derive it from a hostname resolved by an arbitrary transport.
        _ => false,
    }
}

fn client_context(context: &WindowContext, ancestors: &[WindowContext],
    environment: &ReceivingWindowEnvironment, policies: CspPolicyList) -> Result<ClientContext, DomError> {
    let top = ancestors.last().unwrap_or(context);
    let site = top.fetch_origin.site(&amazing_fetch::psl::PublicSuffixList);
    let same_site = std::iter::once(context).chain(ancestors).all(|document|
        document.fetch_origin.site(&amazing_fetch::psl::PublicSuffixList) == site);
    let now_unix = SystemTime::now().duration_since(UNIX_EPOCH)
        .map_err(|_| DomError::InvalidStateError)?.as_secs().try_into()
        .map_err(|_| DomError::InvalidStateError)?;
    Ok(ClientContext {
        cookie_site: if same_site { CookieSite::Origin(top.fetch_origin.clone()) } else { CookieSite::CrossSite },
        partition_key: Some(site),
        frame_ancestors: ancestors.iter().map(|ancestor| ancestor.fetch_origin.clone()).collect(),
        security: SecurityContext {
            client_restricts_mixed_content: environment.restricts_mixed_content,
            destination: Destination::Empty, image_set_initiator: false,
            // This is a receiving client environment, not a navigation request.
            top_level_navigation: false, embedder_policy: environment.embedder_policy.value,
            no_cors_media_state: NoCorsMediaState::NotMedia,
        },
        csp: policies, csp_destination: CspDestination::None, now_unix,
    })
}


/// Admission is independent of Script queues. Creation initialization alone
/// does not authenticate an HTTP response or permit late policy substitution.
#[derive(Clone)]
pub(super) struct ReceivingNativeAdmission {
    pub(super) creation_url: ParsedUrl,
    pub(super) environment: ReceivingWindowEnvironment,
    pub(super) referrer_policy: amazing_fetch::ReferrerPolicy,
    pub(super) response_head: Option<crate::navigation_response::RootResponseHead>,
}
impl ReceivingDocumentClientFactory<ReceivingDocumentIdentity> for RuntimeBindingHost<'_> {
    type Error=DomError;
    fn receiving_document_client(&self,identity:&ReceivingDocumentIdentity)
        ->Result<ReceivingDocumentClient<ReceivingDocumentIdentity>,DomError> {
        receiving_document_client_from_state(&self.state.borrow(),identity)
    }
}
impl RuntimeBindings {
    pub(crate) fn receiving_document_identity(&self,document:NodeIdx)
        ->Result<ReceivingDocumentIdentity,DomError> {
        let state=self.state.borrow();let host=state.host.borrow();
        let realm=state.frames.document_realm(&host.dom,document).ok_or(DomError::InvalidStateError)?;
        let context=state.frames.contexts.get(&realm).ok_or(DomError::InvalidStateError)?;
        if state.frames.history_context(&host.dom,document).is_none()
            || host.dom.document_window_event_target(document)!=Some(context.event_target_slot) {
            return Err(DomError::InvalidStateError);
        }
        Ok(ReceivingDocumentIdentity{realm,document,generation:context.document_generation,window:context.event_target_slot})
    }
    pub(crate) fn receiving_document_client(&self,js:&mut Interpreter,identity:&ReceivingDocumentIdentity)
        ->Result<ReceivingDocumentClient<ReceivingDocumentIdentity>,DomError> {
        ReceivingDocumentClientFactory::receiving_document_client(
            &RuntimeBindingHost{js,state:self.state.clone()},identity)
    }
}
pub(in crate::runtime_binding_host) fn receiving_document_client_from_state(
    state:&BindingState,identity:&ReceivingDocumentIdentity,
)->Result<ReceivingDocumentClient<ReceivingDocumentIdentity>,DomError> {
    let host=state.host.borrow();
    let context=state.frames.contexts.get(&identity.realm).ok_or(DomError::InvalidStateError)?;
    if context.document!=identity.document || context.document_generation!=identity.generation
        || context.event_target_slot!=identity.window
        || state.frames.history_context(&host.dom,identity.document).is_none()
        || host.dom.document_window_event_target(identity.document)!=Some(identity.window) {
        return Err(DomError::InvalidStateError);
    }
    let admission=state.frames.receiving_admissions.get(&(identity.document,identity.generation))
        .ok_or(DomError::InvalidStateError)?;
    let (creation,ancestors)=state.frames.receiving_creation_context(&host.dom,identity.realm)?;
    let policy=state.net.document_network_policy(&host.dom,identity.document,identity.generation)
        .ok_or(DomError::InvalidStateError)?;
    let client=client_context(&creation,&ancestors,&admission.environment,policy.csp)?;
    let current=state.frames.document_context_for_realm(&host.dom,identity.realm)
        .ok_or(DomError::InvalidStateError)?;
    let base=super::super::dom_bindings::document_base_url_from_state(state,identity.document)?;
    Ok(ReceivingDocumentClient{identity:*identity,creation_url:admission.creation_url.clone(),
        document_url:ParsedUrl::parse(&current.document_url).map_err(|_|DomError::InvalidStateError)?,
        base_url:ParsedUrl::parse(&base).map_err(|_|DomError::InvalidStateError)?,
        origin:context.fetch_origin.clone(),csp_self_origin:admission.environment.csp_self_origin.clone(),
        context:client,referrer_policy:admission.referrer_policy})
}

impl FrameRuntime {
    pub(crate) fn receiving_fallback_base_url(&self,dom:&Dom,document:NodeIdx)->Option<String> {
        let (generation,base)=self.about_base_urls.get(&document)?;
        (dom.node_generation(document)==Some(*generation)).then(||base.clone())
    }
}
impl RuntimeBindings {
    /// Actual root loader creation checkpoint. Calling this is an explicit
    /// initial/literal admission, never a fallback for a failed HTTP response.
    pub(crate) fn initialize_initial_root_document(&self,js:&mut Interpreter,url:&str)
        ->Result<ReceivingDocumentIdentity,Throw> {
        self.initialize_receiving_root(js,url,None)
    }
    /// Only a successful native loader receipt supplies response admission.
    /// Successful filesystem receipts initialize a no-response container.
    pub(crate) fn initialize_received_root_document(&self,js:&mut Interpreter,url:&str,
        received:&amazing_net::ReceivedResponse)->Result<ReceivingDocumentIdentity,Throw> {
        let head=crate::navigation_response::RootResponseHead::received(received,String::new());
        self.initialize_receiving_root(js,url,Some(&head))
    }
    pub(crate) fn initialize_received_root_head(&self,js:&mut Interpreter,url:&str,
        head:&crate::navigation_response::RootResponseHead)->Result<ReceivingDocumentIdentity,Throw> {
        self.initialize_receiving_root(js,url,Some(head))
    }
    fn initialize_receiving_root(&self,js:&mut Interpreter,url:&str,
        received:Option<&crate::navigation_response::RootResponseHead>)->Result<ReceivingDocumentIdentity,Throw> {
        let realm=js.binding_current_realm();
        let (identity,already)= {
            let state=self.state.borrow();let host=state.host.borrow();
            let context=state.frames.contexts.get(&realm)
                .ok_or_else(||Throw::type_error("Missing actual receiving root Window"))?;
            if context.frame.is_some() || context.parent!=context.window || context.top!=context.window
                || host.dom.node_generation(context.document)!=Some(context.document_generation)
                || host.dom.document_window_event_target(context.document)!=Some(context.event_target_slot) {
                return Err(Throw::type_error("Stale actual receiving root association"));
            }
            (ReceivingDocumentIdentity{realm,window:context.event_target_slot,document:context.document,
                generation:context.document_generation},state.frames.receiving_admissions.contains_key(&(context.document,context.document_generation)))
        };
        if already {return Err(Throw::type_error("Receiving Document already admitted"));}
        let creation_url=ParsedUrl::parse(url).map_err(|_|Throw::type_error("Invalid receiving creation URL"))?;
        let response=received.filter(|received|!received.is_initial_literal()).map(|received|received.metadata());
        if let Some(response)=response {
            if HttpUrl::parse(url).map(|url|url.without_fragment()).ok().as_deref()!=Some(response.url.as_str()) {
                return Err(Throw::type_error("Receiving creation URL differs from accepted response"));
            }
        }
        let mut policies=CspPolicyList{policies:vec![]};
        let mut headers=amazing_fetch::HeaderList::default();
        let protected=response.map(|response| {
            let response_url=HttpUrl::parse(&response.url)
                .ok().filter(|url|matches!(url.as_parsed().scheme(),"http"|"https"))
                .ok_or_else(||Throw::type_error("Unsupported received response policy origin"))?;
            for field in &response.headers {
                headers.append(&field.name,&field.value).map_err(|_|Throw::type_error("Invalid actual response policy header"))?;
            }
            policies=CspPolicyList::from_headers(&headers)
                .map_err(|_|Throw::type_error("Invalid actual response CSP"))?;
            Ok::<_,Throw>(response_url.origin().clone())
        }).transpose()?;
        self.set_module_document_base(js,url);
        self.state.borrow_mut().document_url=Some(url.into());
        self.html_set_document_origin(url);
        let context=self.state.borrow().frames.contexts.get(&realm)
            .ok_or_else(||Throw::type_error("Retired receiving root Window"))?.clone();
        let environment=ReceivingWindowEnvironment{
            csp_self_origin:protected.unwrap_or_else(||context.fetch_origin.clone()),
            top_level_creation_url:creation_url.clone(),restricts_mixed_content:creation_url.scheme()=="https",
            embedder_policy:if response.is_some() {
                amazing_fetch::EmbedderPolicyContainer::from_response(&headers,trustworthy_creation_url(&creation_url))
            } else {amazing_fetch::EmbedderPolicyContainer::default()},
        };
        let policy=client_context(&context,&[],&environment,policies)
            .map_err(|_|Throw::type_error("Invalid receiving root environment"))?;
        let referrer_policy=response.and_then(|_|amazing_fetch::ReferrerPolicy::parse_header(
            &headers.get("referrer-policy").unwrap_or_default()))
            .unwrap_or(amazing_fetch::ReferrerPolicy::StrictOriginWhenCrossOrigin);
        js.binding_set_realm_origin(realm,&context.origin)?;
        let adapter=RuntimeBindingHost{js,state:self.state.clone()};
        adapter.set_document_network_policy(identity.document,policy)?;
        let mut state=self.state.borrow_mut();
        let context=state.frames.contexts.get_mut(&realm).expect("root retained during native admission");
        context.creation_url=url.into();context.receiving_environment=Some(environment.clone());
        state.frames.receiving_admissions.insert((identity.document,identity.generation),
            ReceivingNativeAdmission{creation_url,environment,referrer_policy,response_head:received.cloned()});
        state.frames.document_referrer_policies.insert(identity.document,(identity.generation,referrer_policy));
        state.document_referrer=Some(received.map(|head|head.metadata().referrer.clone()).unwrap_or_default());
        drop(state);
        self.navigate_iframes(js);
        Ok(identity)
    }
}

#[cfg(test)]
#[path = "runtime_host_navigation_context/tests.rs"]
mod tests;

impl RuntimeBindings {
    /// Actual accepted child commit. Missing creator/ancestor admission keeps
    /// this receiver unavailable; legacy view construction supplies no fallback
    /// network authority. No callback is made by this native publication.
    pub(super) fn admit_receiving_child(&self,js:&mut Interpreter,realm:RealmHandle,
        source:&NavigationSource,response:Option<&(Origin,amazing_fetch::HeaderList)>,
        referrer_policy:amazing_fetch::ReferrerPolicy)->Result<(),Throw> {
        let (context,ancestors)= {
            let state=self.state.borrow();let host=state.host.borrow();
            state.frames.receiving_creation_context(&host.dom,realm)
                .map_err(|_|Throw::type_error("Stale receiving child association"))?
        };
        if context.frame.is_none() {return Err(Throw::type_error("Child admission requires an actual frame"));}
        let creation_url=ParsedUrl::parse(&context.creation_url)
            .map_err(|_|Throw::type_error("Invalid actual child creation URL"))?;
        let parent_environment=ancestors.first().and_then(|parent|parent.receiving_environment.as_ref())
            .ok_or_else(||Throw::type_error("Missing actual parent receiving environment"))?;
        let about=matches_about_url(&creation_url,"blank") || matches_about_url(&creation_url,"srcdoc");
        let creator=if about {Some(source.receiving_environment.as_ref()
            .ok_or_else(||Throw::type_error("Missing saved actual about creator environment"))?)} else {None};
        let policies=if about {source.client_context.as_ref()
            .ok_or_else(||Throw::type_error("Missing saved actual creator policy"))?.csp.clone()}
            else {CspPolicyList::from_headers(&response.ok_or_else(||Throw::type_error("Missing accepted child response policy"))?.1)
                .map_err(|_|Throw::type_error("Invalid actual child response CSP"))?};
        let top_level_creation_url=parent_environment.top_level_creation_url.clone();
        let environment=ReceivingWindowEnvironment{
            csp_self_origin:if let Some(creator)=creator {creator.csp_self_origin.clone()}
                else {response.expect("response validated").0.clone()},
            restricts_mixed_content:creation_url.scheme()=="https" || ancestors.iter().any(|parent|
                parent.receiving_environment.as_ref().is_some_and(|env|env.restricts_mixed_content)),
            embedder_policy:if let Some(creator)=creator {creator.embedder_policy.clone()}
                else {amazing_fetch::EmbedderPolicyContainer::from_response(&response.expect("response validated").1,
                    trustworthy_creation_url(&top_level_creation_url))},
            top_level_creation_url,
        };
        let policy=client_context(&context,&ancestors,&environment,policies)
            .map_err(|_|Throw::type_error("Invalid actual child receiving environment"))?;
        RuntimeBindingHost{js,state:self.state.clone()}.set_document_network_policy(context.document,policy)?;
        let mut state=self.state.borrow_mut();
        state.frames.contexts.get_mut(&realm).expect("child retained during native admission")
            .receiving_environment=Some(environment.clone());
        state.frames.receiving_admissions.insert((context.document,context.document_generation),
            ReceivingNativeAdmission{creation_url,environment,referrer_policy,response_head:None});
        Ok(())
    }
}

impl RuntimeBindings {
    /// Same-Document URL projection cannot remint the VM's receiving origin.
    pub(crate) fn native_receiving_realm_origin(&self,realm:RealmHandle)->Option<Origin> {
        let state=self.state.borrow();let host=state.host.borrow();
        state.frames.document_context_for_realm(&host.dom,realm).map(|context|context.origin)
    }
}
