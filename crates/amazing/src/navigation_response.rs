//! Accepted root loader custody. No URL/body-only default head is available.
use amazing_process::protocol::{HeaderField,NavigationResponseHead,NetworkHead};
#[derive(Clone)]
pub(crate) struct RootResponseHead {
    metadata:NavigationResponseHead,
    network:Option<NetworkHead>,
    local_source:Option<amazing_net::ReceivedSource>,
}
impl RootResponseHead {
    pub(crate) fn network(head:&NetworkHead)->Result<Self,String> {
        let metadata=head.navigation.as_ref().ok_or("Missing accepted navigation response metadata")?;
        if head.id==0 || head.client_id==0 || head.epoch==0 || amazing_fetch::HttpUrl::parse(&head.url).map(|url|url.without_fragment()).ok().as_deref()!=Some(metadata.url.as_str()) || metadata.status!=head.status {
            return Err("Accepted navigation response identity mismatch".into());
        }
        Ok(Self{metadata:metadata.clone(),network:Some(head.clone()),local_source:None})
    }
    pub(crate) fn received(received:&amazing_net::ReceivedResponse,referrer:String)->Self {
        let response=received.response();
        Self{metadata:NavigationResponseHead{status:response.status,url:amazing_fetch::HttpUrl::parse(&response.url).map(|url|url.without_fragment()).unwrap_or_else(|_|response.url.clone()),reason:response.reason.clone(),
            headers:response.headers.iter().map(|(name,value)|HeaderField{name:name.into(),value:value.into()}).collect(),referrer},
            network:None,local_source:Some(received.source())}
    }
    pub(crate) fn streaming(response:&amazing_net::StreamingResponse,referrer:String)->Self {
        Self{metadata:NavigationResponseHead{status:response.status,url:amazing_fetch::HttpUrl::parse(&response.url).map(|url|url.without_fragment()).unwrap_or_else(|_|response.url.clone()),reason:response.reason.clone(),
            headers:response.headers.iter().map(|(name,value)|HeaderField{name:name.into(),value:value.into()}).collect(),referrer},
            network:None,local_source:Some(response.source())}
    }
    pub(crate) fn metadata(&self)->&NavigationResponseHead {&self.metadata}
    pub(crate) fn is_local_file(&self)->bool {self.local_source.is_some_and(|source|source.is_local_file())}
    pub(crate) fn is_initial_literal(&self)->bool {
        self.local_source.is_some() && (self.is_local_file()
            || amazing_net::ParsedUrl::parse(&self.metadata.url).is_ok_and(|url|matches!(url.scheme(),"data"|"about")))
    }
    pub(crate) fn accepted_network_head(&self)->Option<&NetworkHead> {self.network.as_ref()}
}
pub(crate) struct RootNavigationResponse {
    pub(crate) body:Vec<u8>,
    pub(crate) head:RootResponseHead,
}
impl RootNavigationResponse {
    pub(crate) fn network(body:&[u8],head:&NetworkHead)->Result<Self,String> {
        Ok(Self{body:body.to_vec(),head:RootResponseHead::network(head)?})
    }
    pub(crate) fn direct(received:amazing_net::ReceivedResponse,referrer:String)->Self {
        let head=RootResponseHead::received(&received,referrer);
        Self{body:received.into_response().body,head}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepted_root_head_preserves_identity_raw_headers_and_fragment_lineage() {
        let metadata=NavigationResponseHead{status:200,url:"https://root.test/final".into(),reason:"OK".into(),
            headers:vec![HeaderField{name:"Content-Security-Policy".into(),value:"connect-src 'none'".into()},
                HeaderField{name:"Content-Security-Policy".into(),value:"default-src 'self'".into()}],referrer:"https://source.test/".into()};
        let head=NetworkHead{id:7,client_id:9,epoch:11,status:200,url:"https://root.test/final#kept".into(),reason:"OK".into(),
            headers:vec![],response_type:"basic".into(),redirected:true,url_list:vec!["https://root.test/start".into(),"https://root.test/final#kept".into()],
            redirect_tainted:false,request_includes_credentials:true,navigation:Some(metadata.clone())};
        let accepted=RootResponseHead::network(&head).unwrap();
        assert_eq!(accepted.metadata(),&metadata);assert_eq!(accepted.accepted_network_head(),Some(&head));
        assert!(!accepted.is_initial_literal());assert!(!accepted.is_local_file());
        for field in 0..5 {
            let mut invalid=head.clone();match field {0=>invalid.id=0,1=>invalid.client_id=0,2=>invalid.epoch=0,
                3=>invalid.status=403,_=>invalid.navigation.as_mut().unwrap().url="https://other.test/".into()};
            assert!(RootResponseHead::network(&invalid).is_err());
        }
        let mut missing=head;missing.navigation=None;assert!(RootResponseHead::network(&missing).is_err());
    }
    #[test]
    fn real_direct_http_redirect_stream_keeps_head_and_creation_fragment_distinct() {
        use std::io::{Read,Write};
        let listener=std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let origin=format!("http://{}",listener.local_addr().unwrap());
        let server=std::thread::spawn(move || {
            for path in ["/start","/final"] {
                let (mut socket,_)=listener.accept().unwrap();let mut request=Vec::new();
                while !request.ends_with(b"\r\n\r\n") {let mut byte=[0];socket.read_exact(&mut byte).unwrap();request.push(byte[0]);}
                assert!(String::from_utf8(request).unwrap().starts_with(&format!("GET {path} ")));
                if path=="/start" {socket.write_all(b"HTTP/1.1 302 Found\r\nLocation: /final#kept\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();}
                else {socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Security-Policy: connect-src 'none'\r\nContent-Length: 1\r\nConnection: close\r\n\r\nx").unwrap();}
            }
        });
        let mut response=amazing_net::fetch_stream(&format!("{origin}/start"),&amazing_net::FetchOptions::default(),amazing_net::StreamOptions::default()).unwrap();
        let head=RootResponseHead::streaming(&response,String::new());
        assert_eq!(response.url,format!("{origin}/final#kept"));
        assert_eq!(head.metadata().url,format!("{origin}/final"));
        assert!(!head.is_local_file());assert!(!head.is_initial_literal());
        assert!(head.metadata().headers.iter().any(|field|field.name.eq_ignore_ascii_case("content-security-policy") && field.value=="connect-src 'none'"));
        assert_eq!(response.next_chunk().unwrap(),Some(b"x".to_vec()));server.join().unwrap();
    }
    #[test]
    fn successful_stream_head_retains_real_file_route_before_body_drainage() {
        let path=std::env::temp_dir().join(format!("amazing-receiving-stream-{}.html",std::process::id()));
        std::fs::write(&path,b"<p>native</p>").unwrap();
        let response=amazing_net::fetch_stream(path.to_str().unwrap(),&amazing_net::FetchOptions::default(),amazing_net::StreamOptions::default()).unwrap();
        let head=RootResponseHead::streaming(&response,String::new());
        assert!(head.is_local_file());assert!(head.is_initial_literal());assert!(head.accepted_network_head().is_none());
        drop(response);std::fs::remove_file(path).unwrap();
    }
}
