//! Opt-in local request-body transmission receipts. These attest that bytes
//! reached the socket, not that the remote application acknowledged them.
use super::RequestId;
use std::collections::{BTreeMap, VecDeque};

#[derive(Debug,Clone,Copy,PartialEq,Eq)]
pub struct RequestUploadProgress {
    pub id: RequestId,
    pub sent: u64,
    pub total: u64,
    pub complete: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    fn frame(kind:u8,flags:u8,stream:u32,payload:&[u8])->Vec<u8> {
        let length=payload.len();
        let mut bytes=vec![(length>>16) as u8,(length>>8) as u8,length as u8,kind,flags];
        bytes.extend_from_slice(&stream.to_be_bytes());bytes.extend_from_slice(payload);bytes
    }
    #[test]
    fn http_body_credit_excludes_headers_and_is_once_after_drain() {
        let mut upload=HttpUpload::new(100,20);
        assert_eq!(upload.drained(RequestId(1),99),None);
        assert_eq!(upload.drained(RequestId(1),107),Some(RequestUploadProgress{id:RequestId(1),sent:7,total:20,complete:false}));
        assert_eq!(upload.drained(RequestId(1),107),None);
        assert_eq!(upload.drained(RequestId(1),120),Some(RequestUploadProgress{id:RequestId(1),sent:20,total:20,complete:true}));
        assert_eq!(upload.drained(RequestId(1),120),None);
        let mut empty=HttpUpload::new(100,0);
        assert_eq!(empty.drained(RequestId(2),99),None);
        assert!(empty.drained(RequestId(2),100).unwrap().complete);
    }
    #[test]
    fn h2_flow_control_frame_boundaries_multiplex_and_retirement() {
        let mut uploads=H2Uploads::default();let mut progress=vec![];
        uploads.track(1,RequestId(1),5);uploads.track(3,RequestId(2),4);
        let headers=frame(1,4,1,b"header");
        uploads.output(&headers,0);
        uploads.drained(headers.len() as u64,&mut progress);assert!(progress.is_empty());
        // Queued provider body without emitted DATA grants nothing. A full
        // frame boundary is needed; plaintext/payload-prefix credit is absent.
        let data=frame(0,0,1,b"abc");let start=headers.len() as u64;
        uploads.output(&data,start);uploads.drained(start+data.len() as u64-1,&mut progress);assert!(progress.is_empty());
        uploads.drained(start+data.len() as u64,&mut progress);
        assert_eq!(progress.pop(),Some(RequestUploadProgress{id:RequestId(1),sent:3,total:5,complete:false}));
        let a=frame(0,1,1,b"de");let b=frame(0,1,3,b"late");let start=start+data.len() as u64;
        uploads.output(&a,start);uploads.output(&b,start+a.len() as u64);
        assert_eq!(uploads.forget(3),Some(RequestId(2)));
        uploads.drained(start+(a.len()+b.len()) as u64,&mut progress);
        assert_eq!(progress,vec![RequestUploadProgress{id:RequestId(1),sent:5,total:5,complete:true}]);
        assert!(uploads.markers.is_empty());assert!(uploads.streams.is_empty());
    }
    #[test]
    fn h2_empty_body_end_requires_complete_header_block() {
        let mut uploads=H2Uploads::default();let mut progress=vec![];
        uploads.track(1,RequestId(1),0);
        let first=frame(1,1,1,b"head");let last=frame(9,4,1,b"tail");
        uploads.output(&first,0);uploads.drained(first.len() as u64,&mut progress);assert!(progress.is_empty());
        uploads.output(&last,first.len() as u64);
        uploads.drained((first.len()+last.len()) as u64-1,&mut progress);assert!(progress.is_empty());
        uploads.drained((first.len()+last.len()) as u64,&mut progress);
        assert_eq!(progress,vec![RequestUploadProgress{id:RequestId(1),sent:0,total:0,complete:true}]);
    }
}
pub(super) struct HttpUpload {
    pub header_bytes: usize,
    pub total: usize,
    sent: usize,
    complete: bool,
}
impl HttpUpload {
    pub fn new(header_bytes:usize,total:usize)->Self {
        Self {header_bytes,total,sent:0,complete:false}
    }
    pub fn drained(&mut self,id:RequestId,written:usize)->Option<RequestUploadProgress> {
        let sent=written.saturating_sub(self.header_bytes).min(self.total);
        let complete=written>=self.header_bytes.saturating_add(self.total);
        if self.complete || (sent==self.sent && !complete) {return None;}
        self.sent=sent;self.complete=complete;
        Some(RequestUploadProgress{id,sent:sent as u64,total:self.total as u64,complete})
    }
}
struct StreamUpload {id:RequestId,total:u64,sent:u64,header_end:bool}
struct Marker {end:u64,stream:u32,bytes:u64,complete:bool}
#[derive(Default)]
pub(super) struct H2Uploads {
    streams:BTreeMap<u32,StreamUpload>,
    markers:VecDeque<Marker>,
}
impl H2Uploads {
    pub fn track(&mut self,stream:u32,id:RequestId,total:usize) {
        self.streams.insert(stream,StreamUpload{id,total:total as u64,sent:0,header_end:false});
    }
    pub fn forget(&mut self,stream:u32)->Option<RequestId> {
        let id=self.streams.remove(&stream).map(|upload|upload.id);
        self.markers.retain(|marker|marker.stream!=stream);
        id
    }
    /// FrameIo emits complete serialized frames. Ledger memory follows the
    /// bounded outbound queue; no frame payload or lifetime history is copied.
    pub fn output(&mut self,mut bytes:&[u8],mut start:u64) {
        const PREFACE:&[u8]=b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";
        if bytes.starts_with(PREFACE) {bytes=&bytes[PREFACE.len()..];start+=PREFACE.len() as u64;}
        while bytes.len()>=9 {
            let length=((bytes[0] as usize)<<16)|((bytes[1] as usize)<<8)|bytes[2] as usize;
            if bytes.len()<9+length {break;}
            let kind=bytes[3];let flags=bytes[4];
            let stream=u32::from_be_bytes(bytes[5..9].try_into().expect("frame stream"))&0x7fffffff;
            let end=start+(9+length) as u64;
            if let Some(upload)=self.streams.get_mut(&stream) {
                if kind==1 && flags&1!=0 {upload.header_end=true;}
                let header_complete=matches!(kind,1|9) && flags&4!=0 && upload.header_end;
                if kind==0 || header_complete {
                    let padding=if kind==0 && flags&8!=0 {1+usize::from(bytes[9])} else {0};
                    self.markers.push_back(Marker{end,stream,
                        bytes:if kind==0 {length.saturating_sub(padding) as u64} else {0},
                        complete:header_complete || kind==0 && flags&1!=0});
                }
            }
            bytes=&bytes[9+length..];start=end;
        }
    }
    pub fn drained(&mut self,position:u64,output:&mut Vec<RequestUploadProgress>) {
        let mut changed=BTreeMap::new();
        while self.markers.front().is_some_and(|marker|marker.end<=position) {
            let marker=self.markers.pop_front().expect("front marker");
            if let Some(upload)=self.streams.get_mut(&marker.stream) {
                upload.sent=upload.sent.saturating_add(marker.bytes).min(upload.total);
                changed.insert(marker.stream,RequestUploadProgress{id:upload.id,sent:upload.sent,
                    total:upload.total,complete:marker.complete});
            }
        }
        for (stream,progress) in changed {
            if progress.complete {self.streams.remove(&stream);}
            output.push(progress);
        }
    }
}
