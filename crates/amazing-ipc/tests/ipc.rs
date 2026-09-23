use amazing_ipc::{codec, interface, memory_pair, wire_enum, wire_struct, Connection, Error};
use std::collections::BTreeMap;
use std::time::Duration;
wire_enum! {#[derive(Debug,PartialEq,Eq)] pub enum Mode {Off(()) = 0, On(String) = 1}}
wire_struct! {#[derive(Debug,PartialEq,Eq)] pub struct Data {title:String, bytes:Vec<u8>, tags:BTreeMap<String,String>, mode:Option<Mode>}}
interface! {pub interface Echo(3) => EchoProxy, EchoReceiver {fn echo(message:String) -> String = 1;}}
struct Server;
impl Echo for Server {
    fn echo(&self, message: String) -> Result<String, Error> {
        Ok(message)
    }
}
#[test]
fn codec_roundtrip_and_truncation() {
    let mut tags = BTreeMap::new();
    tags.insert("a".into(), "b".into());
    let input = Data {
        title: "hello".into(),
        bytes: vec![0, 1, 255],
        tags,
        mode: Some(Mode::On("yes".into())),
    };
    let bytes = codec::encode(&input).unwrap();
    assert_eq!(codec::decode::<Data>(&bytes).unwrap(), input);
    for n in 0..bytes.len() {
        assert!(codec::decode::<Data>(&bytes[..n]).is_err(), "prefix {n}")
    }
}
#[test]
fn random_input_never_panics() {
    let mut state = 0x1234_5678_9abc_def0u64;
    for n in 0..2048 {
        let mut bytes = vec![0; n % 193];
        for b in &mut bytes {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            *b = state as u8;
        }
        let _ = codec::decode::<Data>(&bytes);
    }
}
#[test]
fn interface_roundtrip_disconnect() {
    let (a, b) = memory_pair();
    let client = Connection::new(a);
    let server = Connection::new(b);
    server.bind(3, EchoReceiver(Server));
    let proxy = EchoProxy::new(client.clone(), Duration::from_secs(2));
    assert_eq!(proxy.echo("hello".into()).unwrap(), "hello");
    server.close();
    assert!(proxy.echo("again".into()).is_err());
}

#[test]
fn rejects_versions_limits_and_depth() {
    let mut bytes = codec::encode(&"hello".to_string()).unwrap();
    bytes[0] = 2;
    assert_eq!(codec::decode::<String>(&bytes), Err(codec::Error::Version));
    let mut e = codec::Encoder::new();
    assert_eq!(
        e.bytes(&vec![0; codec::MAX_FRAME]),
        Err(codec::Error::Limit)
    );
    fn descend(e: &mut codec::Encoder, n: usize) -> Result<(), codec::Error> {
        if n == 0 {
            Ok(())
        } else {
            e.nested(|e| descend(e, n - 1))
        }
    }
    let mut e = codec::Encoder::new();
    assert_eq!(
        descend(&mut e, codec::MAX_DEPTH + 1),
        Err(codec::Error::Limit)
    );
    let mut bad = vec![1, 0];
    bad.extend_from_slice(&u32::MAX.to_le_bytes());
    assert_eq!(codec::decode::<Vec<u8>>(&bad), Err(codec::Error::Limit));
}
#[test]
fn timeout_cancels_request_and_interfaces_are_associated() {
    interface! {interface Other(4) => OtherProxy, OtherReceiver {fn wait(value:u32) -> u32 = 1;}}
    struct OtherServer;
    impl Other for OtherServer {
        fn wait(&self, value: u32) -> Result<u32, Error> {
            std::thread::sleep(Duration::from_millis(50));
            Ok(value)
        }
    }
    let (a, b) = memory_pair();
    let client = Connection::new(a);
    let server = Connection::new(b);
    server.bind(3, EchoReceiver(Server));
    server.bind(4, OtherReceiver(OtherServer));
    assert_eq!(
        EchoProxy::new(client.clone(), Duration::from_secs(1))
            .echo("ok".into())
            .unwrap(),
        "ok"
    );
    assert_eq!(
        OtherProxy::new(client.clone(), Duration::from_millis(1)).wait(5),
        Err(Error::Timeout)
    );
    assert_eq!(
        OtherProxy::new(client, Duration::from_secs(1))
            .wait(6)
            .unwrap(),
        6
    );
}
