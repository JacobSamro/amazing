//! Bounded background acquisition. HTTP requests share the engine's cooperative
//! Dispatcher, so one tab's stalled resources cannot occupy every transport worker.
use super::{WorkerJob, ResourceRequest, acquisition, Acquired, AcquisitionResult};
use amazing_net::{FetchOptions, ParsedUrl, Response, dispatch::{Dispatcher, DispatchLimits, NetEvent, RequestId}};
use std::{collections::{HashMap,VecDeque}, sync::{Arc,Mutex,atomic::Ordering,mpsc::{self,SyncSender,TrySendError}},time::{Duration,Instant}};

struct HttpWork {
    job: WorkerJob,
    current: ParsedUrl,
    options: FetchOptions,
    response: Option<Response>,
    redirects: usize,
}
impl HttpWork {
    fn new(job: WorkerJob) -> Result<Self,(WorkerJob,String)> {
        let (address,mut options)=match &job.request {
            ResourceRequest::Session(address,options)|ResourceRequest::Raw(address,options)=>(address,options.clone()),
            _=>unreachable!("font uses existing font service"),
        };
        options.timeout=options.timeout.max(Duration::from_secs(30));
        let current=match ParsedUrl::parse(address) {Ok(url)=>url,Err(error)=>return Err((job,format!("{error:?}")))};
        Ok(Self {job,current,options,response:None,redirects:0})
    }
    fn submit(&self,dispatcher:&mut Dispatcher)->Result<RequestId,String> {
        dispatcher.submit(&self.current.to_string(),self.options.clone(),Instant::now()).map_err(|error|error.to_string())
    }
    fn complete(self,result:AcquisitionResult) {let _=self.job.result.send(result);}
    fn done(mut self,dispatcher:&mut Dispatcher,active:&mut HashMap<RequestId,HttpWork>) {
        let Some(response)=self.response.take() else {self.complete(Err("response head missing".into()));return};
        if !matches!(response.status,301|302|303|307|308) {self.complete(Ok(Acquired::Response(response)));return}
        if self.redirects==amazing_net::fetch::MAX_REDIRECTS {self.complete(Err("too many redirects".into()));return}
        let Some(location)=response.headers.get("location") else {self.complete(Err("redirect location missing".into()));return};
        let target=match self.current.resolve(location) {Ok(url)=>url.with_inherited_fragment(&self.current),Err(error)=>{self.complete(Err(format!("{error:?}")));return}};
        if !matches!(target.scheme(),"http"|"https") {self.complete(Err("unsupported redirect scheme".into()));return}
        if matches!(response.status,301..=303)&&self.options.method.eq_ignore_ascii_case("POST") {
            self.options.method="GET".into();self.options.body.clear();
            self.options.request_headers.retain(|(name,_)|!name.eq_ignore_ascii_case("content-type")&&!name.eq_ignore_ascii_case("content-length"));
        }
        self.current=target;self.redirects+=1;
        match self.submit(dispatcher) {Ok(id)=>{active.insert(id,self);},Err(error)=>self.complete(Err(error))}
    }
}
fn blocking(job:WorkerJob) {
    let result=match &job.request {
        ResourceRequest::Session(address,options)|ResourceRequest::Raw(address,options)=>acquisition::transport(address,options,&job.cancel).map(Acquired::Response),
        ResourceRequest::Font {source,stylesheet,document,max_bytes}=>{
            if job.cancel.load(Ordering::Acquire) {Err("navigation cancelled".into())}
            else {
                let resolved = crate::engine_viewport::resolve_font_url(stylesheet, source);
                if resolved.starts_with("http://") || resolved.starts_with("https://") {
                    crate::engine_viewport::page_font_loader::fetch_font(source,stylesheet,document,*max_bytes,&job.cancel).map(Acquired::Bytes)
                } else {
                    let options = FetchOptions { max_response_bytes: *max_bytes, ..FetchOptions::default() };
                    acquisition::transport(&resolved, &options, &job.cancel).map(|response| Acquired::Bytes(response.body))
                }
            }
        },
    };
    let _=job.result.send(result);
}
fn route(job:WorkerJob,http:&mut VecDeque<WorkerJob>,other:&mut VecDeque<WorkerJob>) {
    let is_http=match &job.request {
        ResourceRequest::Session(address,_)|ResourceRequest::Raw(address,_)=>address.starts_with("http://")||address.starts_with("https://"),
        _=>false,
    };
    if is_http {http.push_back(job)} else {other.push_back(job)}
}
pub(super) fn start()->SyncSender<WorkerJob> {
    let (sender,receiver)=mpsc::sync_channel::<WorkerJob>(64);
    let (other_sender,other_receiver)=mpsc::sync_channel::<WorkerJob>(32);
    let other_receiver=Arc::new(Mutex::new(other_receiver));
    // Font CORS service and local-file legacy transport retain their existing semantics.
    // They cannot prevent the HTTP dispatcher from advancing other tabs' documents.
    for _ in 0..2 {
        let receiver=other_receiver.clone();
        std::thread::spawn(move||loop {
            let job=match receiver.lock().expect("navigation fallback queue").recv() {Ok(job)=>job,Err(_)=>break};
            blocking(job);
        });
    }
    std::thread::spawn(move|| {
        let mut dispatcher=match Dispatcher::new_pending_io(DispatchLimits {max_in_flight:16,..DispatchLimits::default()}) {
            Ok(dispatcher) => dispatcher,
            Err(error) => {
                while let Ok(job) = receiver.recv() { let _ = job.result.send(Err(format!("pending navigation transport: {error}"))); }
                return;
            },
        };
        let mut active:HashMap<RequestId,HttpWork>=HashMap::new();
        let (mut http,mut other)=(VecDeque::new(),VecDeque::new());
        loop {
            if active.is_empty()&&http.is_empty()&&other.is_empty() {
                match receiver.recv() {Ok(job)=>route(job,&mut http,&mut other),Err(_)=>break}
            }
            while http.len()+other.len()<64 {
                match receiver.try_recv() {Ok(job)=>route(job,&mut http,&mut other),Err(_)=>break}
            }
            while let Some(job)=other.pop_front() {
                match other_sender.try_send(job) {Ok(())=>{},Err(TrySendError::Full(job))=>{other.push_front(job);break},Err(TrySendError::Disconnected(job))=>{let _=job.result.send(Err("navigation fallback unavailable".into()));}}
            }
            let cancelled:Vec<_>=active.iter().filter(|(_,work)|work.job.cancel.load(Ordering::Acquire)).map(|(id,_)|*id).collect();
            for id in cancelled {dispatcher.cancel(id);if let Some(work)=active.remove(&id) {work.complete(Err("navigation cancelled".into()));}}
            while dispatcher.in_flight()<16 {
                let Some(job)=http.pop_front() else {break};
                if job.cancel.load(Ordering::Acquire) {let _=job.result.send(Err("navigation cancelled".into()));continue}
                match HttpWork::new(job) {
                    Ok(work)=>match work.submit(&mut dispatcher) {Ok(id)=>{active.insert(id,work);},Err(error)=>work.complete(Err(error))},
                    Err((job,error))=>{let _=job.result.send(Err(error));},
                }
            }
            for event in dispatcher.step(64 * 1024,Instant::now()) {
                let id=match &event {NetEvent::Head{id,..}|NetEvent::Body{id,..}|NetEvent::Done{id}|NetEvent::Error{id,..}|NetEvent::Aborted{id}=>*id};
                let Some(mut work)=active.remove(&id) else {continue};
                match event {
                    NetEvent::Head{url,status,reason,headers,..}=>work.response=Some(Response {url,status,reason,headers,body:Vec::new()}),
                    NetEvent::Body{bytes,..}=>{
                        let Some(response)=work.response.as_mut() else {dispatcher.cancel(id);work.complete(Err("body before response head".into()));continue};
                        if response.body.len().saturating_add(bytes.len())>work.options.max_response_bytes {dispatcher.cancel(id);work.complete(Err("response size limit".into()));continue}
                        response.body.extend(bytes);
                    },
                    NetEvent::Done{..}=>{work.done(&mut dispatcher,&mut active);continue},
                    NetEvent::Error{error,..}=>{work.complete(Err(error.to_string()));continue},
                    NetEvent::Aborted{..}=>{work.complete(Err("navigation cancelled".into()));continue},
                }
                active.insert(id,work);
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    });
    sender
}
