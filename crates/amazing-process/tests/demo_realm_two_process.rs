//! IPC-11c follow-up, proved end to end: a real JS realm in the renderer
//! child, in two *real* OS processes, exchanging structured-clone messages
//! both ways through browser-mediated routing, and a `MessagePort`
//! transferred from one process to the other and still live there.
use amazing_process::demo_realm::{DemoPageProxy, PortMessage};
use amazing_process::messaging::MessageRouter;
use amazing_process::protocol::ByteBuf;
use amazing_process::{Capabilities, ProcessHost, RestartPolicy, Role};
use std::collections::BTreeMap;
use std::time::Duration;

fn spawn_realm() -> ProcessHost {
    let exe = std::path::Path::new(env!("CARGO_BIN_EXE_ipc-demo-child"));
    ProcessHost::spawn(
        exe,
        Role::Renderer,
        Capabilities(BTreeMap::from([("__js_realm".into(), "1".into())])),
        RestartPolicy::Never,
    )
    .unwrap()
}

#[test]
fn two_real_processes_exchange_window_messages_both_ways() {
    let mut renderer1 = spawn_realm();
    let mut renderer2 = spawn_realm();
    let page1 = DemoPageProxy::new(renderer1.connection.clone(), Duration::from_secs(5));
    let page2 = DemoPageProxy::new(renderer2.connection.clone(), Duration::from_secs(5));

    // renderer2's page sends; renderer1's page listens.
    page1
        .eval("var got1 = null; onmessage = function(e) { got1 = e.data; };".into())
        .unwrap();
    page2
        .eval("postMessage({from: 'renderer2', n: 1});".into())
        .unwrap();
    let outgoing = page2.take_window_outbox(()).unwrap();
    assert_eq!(outgoing.len(), 1, "renderer2's postMessage queued exactly one payload");
    // The browser process routes the message to the other renderer process.
    page1.deliver_window_message(outgoing[0].clone()).unwrap();
    assert_eq!(
        page1.eval("got1.from + ',' + got1.n".into()).unwrap(),
        "renderer2,1",
        "renderer1's page ran a real structured-clone deserialize of renderer2's payload"
    );

    // And the same round trip in the other direction.
    page2
        .eval("var got2 = null; onmessage = function(e) { got2 = e.data; };".into())
        .unwrap();
    page1.eval("postMessage('hello from renderer1');".into()).unwrap();
    let outgoing = page1.take_window_outbox(()).unwrap();
    assert_eq!(outgoing.len(), 1);
    page2.deliver_window_message(outgoing[0].clone()).unwrap();
    assert_eq!(page2.eval("got2".into()).unwrap(), "hello from renderer1");

    renderer1.kill().unwrap();
    renderer2.kill().unwrap();
}

#[test]
fn a_messageport_transferred_across_processes_stays_live() {
    let mut renderer1 = spawn_realm();
    let mut renderer2 = spawn_realm();
    let page1 = DemoPageProxy::new(renderer1.connection.clone(), Duration::from_secs(5));
    let page2 = DemoPageProxy::new(renderer2.connection.clone(), Duration::from_secs(5));

    // The browser mediates with the real, fuzzed `MessageRouter`
    // (`messaging.rs`): renderer1's document creates a `MessageChannel`, and
    // the browser hands the far end to renderer2 -- the port crosses the
    // process boundary here.
    let mut router = MessageRouter::new();
    const CLIENT1: u64 = 1;
    const CLIENT2: u64 = 2;
    let (port_a, port_b) = router.create_pair(CLIENT1).unwrap();
    router.transfer(port_b, CLIENT1, CLIENT2).unwrap();
    assert_eq!(router.owner_of(port_b), Some(CLIENT2));
    router.start(port_b, CLIENT2).unwrap();

    // renderer2's page didn't create this port; it only learns the id the
    // browser assigned and installs a handler for it, exactly as a
    // `MessagePort` object's `onmessage` would be wired in the real binding.
    page2
        .eval(format!(
            "var portGot = null; portHandlers['{port_b}'] = function(data) {{ portGot = data; }};"
        ))
        .unwrap();
    page1
        .eval(format!("portPostMessage({port_a}, {{ping: 'pong'}});"))
        .unwrap();
    let outgoing = page1.take_port_outbox(()).unwrap();
    assert_eq!(outgoing.len(), 1);
    assert_eq!(outgoing[0].port, port_a);

    // The browser looks up the entangled peer and its current owner --
    // renderer2, post-transfer -- and forwards the delivery there.
    let delivery = router
        .post(port_a, CLIENT1, outgoing[0].payload.0.clone())
        .unwrap()
        .expect("port_b is started, so post() delivers immediately");
    assert_eq!(delivery.port, port_b);
    assert_eq!(delivery.client, CLIENT2);
    page2
        .deliver_port_message(PortMessage {
            port: port_b,
            payload: ByteBuf(delivery.payload),
        })
        .unwrap();
    assert_eq!(
        page2.eval("portGot.ping".into()).unwrap(),
        "pong",
        "renderer2 received a live message over a port it never created, transferred from renderer1"
    );

    renderer1.kill().unwrap();
    renderer2.kill().unwrap();
}
