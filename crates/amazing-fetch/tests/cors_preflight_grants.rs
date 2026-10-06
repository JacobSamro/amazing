//! Native Fetch §4.8–4.9 controls. WPT reference: pinned c2a2fce69e5c2509d9dc28440ec8b42532e5c293
//! fetch/api/cors/cors-preflight-cache.any.js. These are native controls, not installed JS WPT.
use amazing_fetch::*;
use amazing_fetch::api::{ApiRequest, ApiRequestInit, ApiRequestInput, ApiError};
use amazing_net::dispatch::DispatchLimits;
use std::{io::{Read, Write}, net::TcpListener, sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}}, thread, time::{Duration, Instant}};
fn request(method: &str, names: &[&str]) -> Request {
    let mut r=Request::new(HttpUrl::parse("https://api.example/resource").unwrap(),HttpUrl::parse("https://client.example/").unwrap().origin().clone());
    r.credentials=CredentialsMode::Omit;
    r.set_method_body(method,None).unwrap();
    for name in names {r.headers.append(name,"value").unwrap();} r
}
fn grant(r:&Request,methods:Option<&str>,names:Option<&str>,age:Option<&str>)->InternalResponse {
    let mut headers=HeaderList::default();
    headers.append("access-control-allow-origin",&cors_origin(r)).unwrap();
    headers.append("access-control-allow-credentials","true").unwrap();
    for (name,value) in [("access-control-allow-methods",methods),("access-control-allow-headers",names),("access-control-max-age",age)] {
        if let Some(value)=value {headers.append(name,value).unwrap();}
    }
    InternalResponse{status:204,status_text:String::new(),headers,body:None,request_includes_credentials:false}
}
#[test]
fn server_grants_other_methods_and_header_subsets_independently() {
    let now=Instant::now();let r=request("PUT",&["x-one","x-two"]);let mut cache=PreflightCache::default();
    let response=grant(&r,Some("PUT, DELETE, patch"),Some("X-One, x-two, x-extra"),Some("60"));
    check_preflight(&r,&response).unwrap();cache.insert(&r,&response,now);
    for method in ["PUT","DELETE","patch","GET","POST"] {
        for names in [vec![],vec!["x-one"],vec!["x-two"],vec!["x-extra"],vec!["x-one","x-two"]] {
            assert!(cache.contains(&request(method,&names),now),"{method}/{names:?}");
        }
    }
    assert!(!cache.contains(&request("PATCH",&["x-one"]),now));
    assert!(!cache.contains(&request("PUT",&["x-ungranted"]),now));
}
#[test]
fn independent_responses_compose_permissions_without_aggregated_header_identity() {
    let now=Instant::now();let mut cache=PreflightCache::default();
    for name in ["x-one","x-two"] {let r=request("PUT",&[name]);cache.insert(&r,&grant(&r,Some("PUT"),Some(name),Some("60")),now);}
    assert!(cache.contains(&request("PUT",&["x-one","x-two"]),now));
}
#[test]
fn wildcard_grants_cover_future_names_but_never_authorization() {
    let now=Instant::now();let r=request("PUT",&["x-one"]);let mut cache=PreflightCache::default();
    cache.insert(&r,&grant(&r,Some("*"),Some("*"),Some("60")),now);
    assert!(cache.contains(&request("DELETE",&["x-later"]),now));
    assert!(!cache.contains(&request("POST",&["authorization"]),now));
    let mut include=request("DELETE",&["x-later"]);include.credentials=CredentialsMode::Include;
    assert!(!cache.contains(&include,now));
    // Fetch §3.3.5: a literal * in an Include response is not a wildcard.
    let mut literal=request("POST",&[]);literal.credentials=CredentialsMode::Include;
    let mut credentialed=PreflightCache::default();credentialed.insert(&literal,&grant(&literal,Some("*"),Some("*"),Some("60")),now);
    let mut future=request("PUT",&[]);future.credentials=CredentialsMode::Include;
    assert!(!credentialed.contains(&future,now));
    future.set_method_body("POST",None).unwrap();future.headers.append("x-later","v").unwrap();
    assert!(!credentialed.contains(&future,now));
    let broad=request("PUT",&["x-one"]);
    credentialed.insert(&broad,&grant(&broad,Some("*"),Some("*"),Some("60")),now);
    assert!(credentialed.contains(&request("DELETE",&["x-later"]),now));
    assert!(!credentialed.contains(&future,now));
    let auth=request("POST",&["authorization"]);
    cache.insert(&auth,&grant(&auth,Some("POST"),Some("Authorization"),Some("60")),now);
    assert!(cache.contains(&auth,now));
}
#[test]
fn forced_safelisted_method_without_acam_is_cached_but_unforced_is_not_minted() {
    let now=Instant::now();let mut r=request("POST",&[]);r.use_cors_preflight=true;
    let mut cache=PreflightCache::default();cache.insert(&r,&grant(&r,None,None,Some("60")),now);
    assert!(cache.contains(&r,now));
    let mut changed=r.clone();changed.set_method_body("GET",None).unwrap();assert!(!cache.contains(&changed,now));
}
#[test]
fn refresh_and_zero_age_affect_matching_permission_without_erasing_siblings() {
    let now=Instant::now();let r=request("PUT",&["x-one"]);let mut cache=PreflightCache::default();
    cache.insert(&r,&grant(&r,Some("PUT, DELETE"),Some("x-one, x-two"),Some("2")),now);
    cache.insert(&r,&grant(&r,Some("PUT"),Some("x-one"),Some("10")),now+Duration::from_secs(1));
    assert!(cache.contains(&r,now+Duration::from_secs(3)));
    assert!(!cache.contains(&request("DELETE",&["x-two"]),now+Duration::from_secs(3)));
    cache.insert(&r,&grant(&r,Some("PUT"),Some("x-one"),Some("0")),now+Duration::from_secs(4));
    assert!(!cache.contains(&r,now+Duration::from_secs(4)));
}
#[test]
fn expired_default_capped_age_and_invalid_response_never_create_authority() {
    let now=Instant::now();let r=request("PUT",&["x-one"]);
    for (age,seconds) in [(None,5),(Some("bad"),5),(Some("7201"),7200)] {
        let mut cache=PreflightCache::default();cache.insert(&r,&grant(&r,Some("PUT"),Some("x-one"),age),now);
        assert!(cache.contains(&r,now+Duration::from_secs(seconds-1)));
        assert!(!cache.contains(&r,now+Duration::from_secs(seconds)));
    }
    for bad in ["status","origin","methods","headers"] {
        let mut response=grant(&r,Some("PUT"),Some("x-one"),Some("60"));
        match bad {"status"=>response.status=403,"origin"=>{response.headers.remove("access-control-allow-origin");response.headers.append("access-control-allow-origin","https://wrong.example").unwrap();},"methods"=>{response.headers.remove("access-control-allow-methods");response.headers.append("access-control-allow-methods","DELETE").unwrap();},_=>{response.headers.remove("access-control-allow-headers");response.headers.append("access-control-allow-headers","x-other").unwrap();}}
        let mut cache=PreflightCache::default();cache.insert(&r,&response,now);assert!(!cache.contains(&r,now),"{bad}");
    }
}
struct Peer {url:HttpUrl,stop:Arc<AtomicBool>,trace:Arc<Mutex<Vec<String>>>,thread:Option<thread::JoinHandle<()>>}
impl Peer {
    fn new(methods:&str,names:&str,fail_main:Option<usize>)->Self { Self::with_failure(methods,names,fail_main,false) }
    fn with_failure(methods:&str,names:&str,fail_main:Option<usize>,transport:bool)->Self {Self::with_behavior(methods,names,fail_main,transport,false,None)}
    fn with_behavior(methods:&str,names:&str,fail_main:Option<usize>,transport:bool,echo_origin:bool,redirect:Option<String>)->Self {
        let listener=TcpListener::bind("127.0.0.1:0").unwrap();listener.set_nonblocking(true).unwrap();
        let url=HttpUrl::parse(&format!("http://127.0.0.1:{}/resource",listener.local_addr().unwrap().port())).unwrap();
        let stop=Arc::new(AtomicBool::new(false));let trace=Arc::new(Mutex::new(Vec::new()));
        let s=stop.clone();let t=trace.clone();let methods=methods.to_owned();let names=names.to_owned();
        let worker=thread::spawn(move||{let end=Instant::now()+Duration::from_secs(20);let mut main_count=0;
            while !s.load(Ordering::SeqCst) && Instant::now()<end {
                let (mut socket,_)=match listener.accept(){Ok(x)=>x,Err(e) if e.kind()==std::io::ErrorKind::WouldBlock=>{thread::sleep(Duration::from_millis(1));continue;},Err(e)=>panic!("{e}")};
                socket.set_read_timeout(Some(Duration::from_secs(3))).unwrap();let mut bytes=Vec::new();
                loop {let mut b=[0;1];if socket.read_exact(&mut b).is_err(){break;}bytes.push(b[0]);if bytes.ends_with(b"\r\n\r\n"){break;}}
                let text=String::from_utf8(bytes).unwrap();let method=text.split_whitespace().next().unwrap_or("");
                t.lock().unwrap().push(text.clone());
                let allowed_origin=if echo_origin {text.lines().filter_map(|line|line.split_once(':')).find(|(name,_)|name.eq_ignore_ascii_case("origin")).map(|(_,v)|v.trim()).unwrap_or("missing")} else {"http://client.example"};
                let response=if method=="OPTIONS" {format!("HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: {allowed_origin}\r\nAccess-Control-Allow-Credentials: true\r\nAccess-Control-Allow-Methods: {methods}\r\nAccess-Control-Allow-Headers: {names}\r\nAccess-Control-Max-Age: 60\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")}
                else {main_count+=1;if let Some(target)=redirect.as_ref() {format!("HTTP/1.1 307 Temporary Redirect\r\nLocation: {target}\r\nAccess-Control-Allow-Origin: {allowed_origin}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")} else if fail_main==Some(main_count){if transport {"HTTP/1.1 200 OK\r\nAccess-Control-Allow-Origin: http://client.example\r\nContent-Length: 2\r\nConnection: close\r\n\r\n".into()} else {"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into()}}else{format!("HTTP/1.1 200 OK\r\nAccess-Control-Allow-Origin: {allowed_origin}\r\nAccess-Control-Allow-Credentials: true\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")}};
                let _=socket.write_all(response.as_bytes());
            }
        });Self{url,stop,trace,thread:Some(worker)}
    }
    fn count(&self,method:&str)->usize {self.trace.lock().unwrap().iter().filter(|line|line.starts_with(&format!("{method} "))).count()}
}
impl Drop for Peer {fn drop(&mut self){self.stop.store(true,Ordering::SeqCst);self.thread.take().unwrap().join().unwrap();}}
fn fetch(service:&mut NetworkService,peer:&Peer,method:&str,names:&[&str],partition:u128,credentials:CredentialsMode)->Result<api::ApiResponse,ApiError> {
    let units=peer.url.serialize().encode_utf16().collect::<Vec<_>>();
    let origin=HttpUrl::parse("http://client.example/").unwrap().origin().clone();
    let mut init=ApiRequestInit::default();init.method=Some(method.encode_utf16().collect());
    let mut req=ApiRequest::new(ApiRequestInput::Url(&units),init,&peer.url,origin).unwrap();
    req.referrer="http://client.example/original-page".into();req.referrer_policy=ReferrerPolicy::UnsafeUrl;
    req.credentials=credentials;for name in names {req.headers.append(&name.encode_utf16().collect::<Vec<_>>(),&[b'v' as u16]).unwrap();}
    let mut context=ClientContext::default();context.partition_key=Some(Site::Opaque(partition));
    let now=Instant::now();let id=service.start_at(req,context,now)?;
    for tick in 0..4000 {for event in service.step(8192,now+Duration::from_millis(tick)){if let ServiceEvent::Finished{id:found,result}=event {if found==id{return result;}}}thread::sleep(Duration::from_millis(1));}panic!("native HTTP deadline")
}
#[test]
fn real_http_server_grants_reuse_methods_header_subsets_and_extra_names() {
    let peer=Peer::new("PUT, DELETE","x-one, x-two, x-extra",None);let mut service=NetworkService::new(DispatchLimits::default());
    for (method,names) in [("PUT",vec!["x-one","x-two"]),("DELETE",vec!["x-one"]),("PUT",vec!["x-extra"]),("GET",vec!["x-two"])] {assert!(fetch(&mut service,&peer,method,&names,1,CredentialsMode::Omit).is_ok());}
    assert_eq!(peer.count("OPTIONS"),1);assert_eq!(peer.count("DELETE"),1);
    let trace=peer.trace.lock().unwrap();let probe=trace.iter().find(|s|s.starts_with("OPTIONS ")).unwrap().to_ascii_lowercase();
    assert!(probe.contains("access-control-request-headers: x-one,x-two\r\n"));assert!(!probe.contains("cookie:"));assert!(probe.contains("referer: http://client.example/original-page\r\n"));assert_eq!(probe.lines().filter(|line|line.starts_with("origin:")).count(),1);
}
#[test]
fn real_http_client_partitions_cannot_borrow_each_others_grants() {
    let peer=Peer::new("PUT","x-one",None);let mut service=NetworkService::new(DispatchLimits::default());
    for partition in [1,2,1,2] {assert!(fetch(&mut service,&peer,"PUT",&["x-one"],partition,CredentialsMode::Omit).is_ok());}
    assert_eq!(peer.count("OPTIONS"),2);assert_eq!(peer.count("PUT"),4);
}
#[test]
fn real_http_cors_main_failure_clears_only_its_original_partition() {
    let peer=Peer::new("PUT","x-one",Some(3));let mut service=NetworkService::new(DispatchLimits::default());
    assert!(fetch(&mut service,&peer,"PUT",&["x-one"],1,CredentialsMode::Omit).is_ok());
    assert!(fetch(&mut service,&peer,"PUT",&["x-one"],2,CredentialsMode::Omit).is_ok());
    assert_eq!(fetch(&mut service,&peer,"PUT",&["x-one"],1,CredentialsMode::Omit).unwrap_err(),ApiError::Network(FetchError::Cors));
    assert!(fetch(&mut service,&peer,"PUT",&["x-one"],2,CredentialsMode::Omit).is_ok());
    assert!(fetch(&mut service,&peer,"PUT",&["x-one"],1,CredentialsMode::Omit).is_ok());
    assert_eq!(peer.count("OPTIONS"),3);assert_eq!(peer.count("PUT"),5);
}
#[test]
fn real_http_wildcard_and_authorization_require_distinct_admission() {
    let peer=Peer::new("*","*",None);let mut service=NetworkService::new(DispatchLimits::default());
    assert!(fetch(&mut service,&peer,"PUT",&["x-one"],1,CredentialsMode::Omit).is_ok());
    assert!(fetch(&mut service,&peer,"DELETE",&["x-later"],1,CredentialsMode::Omit).is_ok());
    assert_eq!(fetch(&mut service,&peer,"POST",&["authorization"],1,CredentialsMode::Omit).unwrap_err(),ApiError::Network(FetchError::Cors));
    assert_eq!(peer.count("OPTIONS"),2);assert_eq!(peer.count("POST"),0);
}
#[test]
fn origin_url_credentials_and_bounded_eviction_remain_separate() {
    let now=Instant::now();let mut r=request("PUT",&["x-one"]);r.credentials=CredentialsMode::Include;
    let mut cache=PreflightCache::default();cache.insert(&r,&grant(&r,Some("PUT"),Some("x-one"),Some("60")),now);
    let mut omit=r.clone();omit.credentials=CredentialsMode::Omit;assert!(cache.contains(&omit,now));
    let mut origin=r.clone();origin.origin=HttpUrl::parse("https://other.example/").unwrap().origin().clone();assert!(!cache.contains(&origin,now));
    let mut different=Request::new(HttpUrl::parse("https://api.example/other").unwrap(),r.origin.clone());different.set_method_body("PUT",None).unwrap();different.headers=r.headers.clone();assert!(!cache.contains(&different,now));
    // Actual scalar entries remain bounded: adding 257 distinct method grants
    // may evict the oldest, never grants a nonexistent method.
    let mut cache=PreflightCache::default();
    for index in 0..257 {let mut next=request(&format!("CUSTOM{index}"),&[]);next.credentials=CredentialsMode::Omit;cache.insert(&next,&grant(&next,Some(next.method()),None,Some("60")),now);}
    assert!(!cache.contains(&request("CUSTOM0",&[]),now));assert!(cache.contains(&request("CUSTOM256",&[]),now));assert!(!cache.contains(&request("CUSTOM999",&[]),now));
}
#[test]
fn real_http_include_and_omit_permissions_are_asymmetric() {
    let peer=Peer::new("PUT","x-one",None);let mut service=NetworkService::new(DispatchLimits::default());
    for mode in [CredentialsMode::Omit,CredentialsMode::Include,CredentialsMode::Omit,CredentialsMode::Include] {assert!(fetch(&mut service,&peer,"PUT",&["x-one"],1,mode).is_ok());}
    assert_eq!(peer.count("OPTIONS"),2);assert_eq!(peer.count("PUT"),4);
}

#[test]
fn real_http_framing_failure_clears_grants_before_retry() {
    let peer=Peer::with_failure("PUT","x-one",Some(2),true);let mut service=NetworkService::new(DispatchLimits::default());
    assert!(fetch(&mut service,&peer,"PUT",&["x-one"],1,CredentialsMode::Omit).is_ok());
    assert_eq!(fetch(&mut service,&peer,"PUT",&["x-one"],1,CredentialsMode::Omit).unwrap_err(),ApiError::Network(FetchError::Transport));
    assert!(fetch(&mut service,&peer,"PUT",&["x-one"],1,CredentialsMode::Omit).is_ok());
    assert_eq!(peer.count("OPTIONS"),2);assert_eq!(peer.count("PUT"),3);
}
#[test]
fn actual_abort_retires_pending_request_and_clears_only_its_grants() {
    let peer=Peer::new("PUT","x-one",None);let mut service=NetworkService::new(DispatchLimits::default());
    assert!(fetch(&mut service,&peer,"PUT",&["x-one"],1,CredentialsMode::Omit).is_ok());
    let units=peer.url.serialize().encode_utf16().collect::<Vec<_>>();
    let origin=HttpUrl::parse("http://client.example/").unwrap().origin().clone();
    let mut init=ApiRequestInit::default();init.method=Some("PUT".encode_utf16().collect());
    let mut req=ApiRequest::new(ApiRequestInput::Url(&units),init,&peer.url,origin).unwrap();
    req.credentials=CredentialsMode::Omit;req.headers.append(&"x-one".encode_utf16().collect::<Vec<_>>(),&[b'v' as u16]).unwrap();
    let mut context=ClientContext::default();context.partition_key=Some(Site::Opaque(1));
    let id=service.start_at(req,context,Instant::now()).unwrap();assert!(service.abort(id));
    assert!(fetch(&mut service,&peer,"PUT",&["x-one"],1,CredentialsMode::Omit).is_ok());
    assert_eq!(peer.count("OPTIONS"),2);assert_eq!(peer.count("PUT"),2);
}

#[test]
fn real_http_redirect_preflight_and_main_use_actual_origin_lineage_once() {
    let target=Peer::with_behavior("PUT","x-one",None,false,true,None);
    let source=Peer::with_behavior("PUT","x-one",None,false,true,Some(target.url.serialize()));
    let mut service=NetworkService::new(DispatchLimits::default());
    assert!(fetch(&mut service,&source,"PUT",&["x-one"],1,CredentialsMode::Omit).is_ok());
    assert_eq!(source.count("OPTIONS"),1);assert_eq!(target.count("OPTIONS"),1);assert_eq!(target.count("PUT"),1);
    for text in target.trace.lock().unwrap().iter() {
        let text=text.to_ascii_lowercase();assert_eq!(text.lines().filter(|line|line.starts_with("origin:")).count(),1);
        assert!(text.contains("origin: null\r\n"));assert!(text.contains("referer: http://client.example/original-page\r\n"));
    }
}
