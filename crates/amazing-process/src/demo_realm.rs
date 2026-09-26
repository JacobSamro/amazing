//! IPC-11c follow-up: a real JS realm in the renderer child, so a page in one
//! renderer process can `postMessage` to a page in another, with the browser
//! process routing the message, and so a `MessagePort` can be handed from one
//! process to the other and stay live.
//!
//! This is deliberately scoped to `ipc-demo-child` (opt-in via the
//! `__js_realm` capability), not the production `renderer::render_document`
//! byte-frame pipeline: running real page `<script>` through layout/paint is
//! a much larger, separate ticket (see `docs/lanes/ipc_TODO.md`, 11c
//! follow-ups). What's proved here is the IPC/process-model half: a real
//! `amazing_js::Interpreter` living in its own OS process, reachable only
//! through this crate's own wire protocol, with cross-process delivery
//! mediated by a browser-held router.
//!
//! Design: each renderer's `Interpreter` lives on one dedicated worker
//! thread (it is `Rc`-based internally, so it cannot cross threads, and
//! `amazing_ipc::Connection` dispatches every request on its own spawned
//! thread). The `DemoPage` receiver is a thin, `Send + Sync` handle that
//! posts jobs to that worker over an `mpsc` channel and waits for the reply.
//! `postMessage`/`portPostMessage` are installed as host-native globals
//! (`Interpreter::set_host_native_dispatcher`) that queue structured-clone
//! bytes into an outbox the browser drains and forwards; `deliver_*` feed a
//! delivery back in as a real `MessageEvent`-shaped call into `onmessage` /
//! `portHandlers[id]`.
use crate::protocol::ByteBuf;
use crate::structured_clone_wire as wire;
use amazing_ipc::wire_struct;
use amazing_ipc::{interface, Connection};
use amazing_js::interp::BindingPropertyFlags;
use amazing_js::value::{HostCallPolicy, HostFunctionId, JsValue, Throw};
use amazing_js::Interpreter;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};

wire_struct! {#[derive(Debug, Clone, PartialEq, Eq)] pub struct PortMessage {port: u64, payload: ByteBuf}}

interface! {pub interface DemoPage(20) => DemoPageProxy, DemoPageReceiver {
    // Run a script in the realm; returns `String(result)`.
    fn eval(source: String) -> String = 1;
    // A window-style message the browser is delivering into this realm.
    fn deliver_window_message(payload: ByteBuf) -> () = 2;
    // A message arriving on a `MessagePort` this realm now owns.
    fn deliver_port_message(message: PortMessage) -> () = 3;
    // Drain messages the page sent via the global `postMessage(data)`.
    fn take_window_outbox(request: ()) -> Vec<ByteBuf> = 4;
    // Drain messages the page sent via `portPostMessage(id, data)`.
    fn take_port_outbox(request: ()) -> Vec<PortMessage> = 5;
}}

#[derive(Default)]
struct Outbox {
    window: Vec<Vec<u8>>,
    ports: Vec<(u64, Vec<u8>)>,
}

enum Job {
    Eval(String, Sender<Result<String, String>>),
    DeliverWindow(Vec<u8>, Sender<Result<(), String>>),
    DeliverPort(u64, Vec<u8>, Sender<Result<(), String>>),
    TakeWindowOutbox(Sender<Vec<Vec<u8>>>),
    TakePortOutbox(Sender<Vec<(u64, Vec<u8>)>>),
}

fn eval_script(interp: &mut Interpreter, source: &str) -> Result<String, String> {
    let value = interp.run_script(source).map_err(|t| t.describe())?;
    let units = interp.binding_to_dom_string(&value).map_err(|t| t.describe())?;
    Ok(String::from_utf16_lossy(&units))
}

fn deliver_window(interp: &mut Interpreter, bytes: &[u8]) -> Result<(), String> {
    let serialized = wire::decode(bytes)?;
    let value = interp
        .structured_deserialize(&serialized)
        .map_err(|t| t.describe())?;
    let global = interp.binding_global_object();
    let handler = interp
        .binding_get_property(&global, "onmessage")
        .map_err(|t| t.describe())?;
    if interp.binding_is_callable(&handler) {
        let event = interp
            .binding_create_object(None)
            .map_err(|t| t.describe())?;
        interp
            .binding_define_data_property(&event, "data", value, BindingPropertyFlags::METHOD)
            .map_err(|t| t.describe())?;
        interp
            .binding_call(&handler, JsValue::Undefined, vec![event])
            .map_err(|t| t.describe())?;
    }
    Ok(())
}

fn deliver_port(interp: &mut Interpreter, port_id: u64, bytes: &[u8]) -> Result<(), String> {
    let serialized = wire::decode(bytes)?;
    let value = interp
        .structured_deserialize(&serialized)
        .map_err(|t| t.describe())?;
    let global = interp.binding_global_object();
    let handlers = interp
        .binding_get_property(&global, "portHandlers")
        .map_err(|t| t.describe())?;
    let handler = interp
        .binding_get_property(&handlers, &port_id.to_string())
        .map_err(|t| t.describe())?;
    if interp.binding_is_callable(&handler) {
        interp
            .binding_call(&handler, JsValue::Undefined, vec![value])
            .map_err(|t| t.describe())?;
    }
    Ok(())
}

/// `postMessage(data)`: the page's window-message send.
const POST_MESSAGE: HostFunctionId = HostFunctionId(1);
/// `portPostMessage(id, data)`: the page's send over a locally owned port.
const PORT_POST_MESSAGE: HostFunctionId = HostFunctionId(2);

fn install_natives(interp: &mut Interpreter, outbox: Rc<RefCell<Outbox>>) {
    let dispatch: Rc<amazing_js::interp::HostNativeDispatch> = Rc::new(
        move |interp: &mut Interpreter, id, _this, args, _new_target| -> Result<JsValue, Throw> {
            match id {
                POST_MESSAGE => {
                    let value = args.first().cloned().unwrap_or(JsValue::Undefined);
                    let serialized = interp.structured_serialize(&value)?;
                    let bytes = wire::encode(&serialized).map_err(Throw::type_error)?;
                    outbox.borrow_mut().window.push(bytes);
                    Ok(JsValue::Undefined)
                }
                PORT_POST_MESSAGE => {
                    let port_id = interp.binding_to_number(
                        args.first().unwrap_or(&JsValue::Undefined),
                    )? as u64;
                    let value = args.get(1).cloned().unwrap_or(JsValue::Undefined);
                    let serialized = interp.structured_serialize(&value)?;
                    let bytes = wire::encode(&serialized).map_err(Throw::type_error)?;
                    outbox.borrow_mut().ports.push((port_id, bytes));
                    Ok(JsValue::Undefined)
                }
                _ => Err(Throw::type_error("unknown demo_realm host function")),
            }
        },
    );
    interp.set_host_native_dispatcher(dispatch);
    let global = interp.binding_global_object();
    let post = interp.binding_create_native_function(
        POST_MESSAGE,
        "postMessage",
        1,
        HostCallPolicy::CallOnly,
    );
    let port_post = interp.binding_create_native_function(
        PORT_POST_MESSAGE,
        "portPostMessage",
        2,
        HostCallPolicy::CallOnly,
    );
    interp
        .binding_define_data_property(&global, "postMessage", post, BindingPropertyFlags::METHOD)
        .expect("install postMessage");
    interp
        .binding_define_data_property(
            &global,
            "portPostMessage",
            port_post,
            BindingPropertyFlags::METHOD,
        )
        .expect("install portPostMessage");
    let handlers = interp
        .binding_create_object(None)
        .expect("create portHandlers");
    interp
        .binding_define_data_property(
            &global,
            "portHandlers",
            handlers,
            BindingPropertyFlags::METHOD,
        )
        .expect("install portHandlers");
}

fn run_worker(rx: Receiver<Job>) {
    let mut interp = Interpreter::new();
    let outbox = Rc::new(RefCell::new(Outbox::default()));
    install_natives(&mut interp, outbox.clone());
    while let Ok(job) = rx.recv() {
        match job {
            Job::Eval(source, reply) => {
                let _ = reply.send(eval_script(&mut interp, &source));
            }
            Job::DeliverWindow(bytes, reply) => {
                let _ = reply.send(deliver_window(&mut interp, &bytes));
            }
            Job::DeliverPort(port, bytes, reply) => {
                let _ = reply.send(deliver_port(&mut interp, port, &bytes));
            }
            Job::TakeWindowOutbox(reply) => {
                let _ = reply.send(std::mem::take(&mut outbox.borrow_mut().window));
            }
            Job::TakePortOutbox(reply) => {
                let _ = reply.send(std::mem::take(&mut outbox.borrow_mut().ports));
            }
        }
    }
}

/// One JS realm, one dedicated worker thread, reachable from any dispatch
/// thread through a job channel (the interpreter itself cannot cross
/// threads: it is `Rc`-based, like every embedding of it in this codebase).
pub struct DemoPageHandler {
    tx: Mutex<Sender<Job>>,
}
impl Default for DemoPageHandler {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || run_worker(rx));
        Self { tx: Mutex::new(tx) }
    }
}
impl DemoPageHandler {
    fn send<T>(&self, make: impl FnOnce(Sender<T>) -> Job) -> Result<T, amazing_ipc::Error> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.tx
            .lock()
            .unwrap()
            .send(make(reply_tx))
            .map_err(|_| amazing_ipc::Error::Disconnected)?;
        reply_rx.recv().map_err(|_| amazing_ipc::Error::Disconnected)
    }
}
impl DemoPage for DemoPageHandler {
    fn eval(&self, source: String) -> Result<String, amazing_ipc::Error> {
        self.send(|reply: Sender<Result<String, String>>| Job::Eval(source, reply))?
            .map_err(amazing_ipc::Error::Remote)
    }
    fn deliver_window_message(&self, payload: ByteBuf) -> Result<(), amazing_ipc::Error> {
        self.send(|reply: Sender<Result<(), String>>| Job::DeliverWindow(payload.0, reply))?
            .map_err(amazing_ipc::Error::Remote)
    }
    fn deliver_port_message(&self, message: PortMessage) -> Result<(), amazing_ipc::Error> {
        self.send(|reply: Sender<Result<(), String>>| {
            Job::DeliverPort(message.port, message.payload.0, reply)
        })?
        .map_err(amazing_ipc::Error::Remote)
    }
    fn take_window_outbox(&self, _request: ()) -> Result<Vec<ByteBuf>, amazing_ipc::Error> {
        Ok(self
            .send(Job::TakeWindowOutbox)?
            .into_iter()
            .map(ByteBuf)
            .collect())
    }
    fn take_port_outbox(&self, _request: ()) -> Result<Vec<PortMessage>, amazing_ipc::Error> {
        Ok(self
            .send(Job::TakePortOutbox)?
            .into_iter()
            .map(|(port, payload)| PortMessage {
                port,
                payload: ByteBuf(payload),
            })
            .collect())
    }
}

/// Bind interface 20 on a child that opted in with the `__js_realm`
/// capability (see `ipc-demo-child.rs`).
pub fn serve_demo_realm(connection: &Arc<Connection>) {
    connection.bind(20, DemoPageReceiver(DemoPageHandler::default()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eval_postmessage_and_delivery_work_in_one_realm() {
        let handler = DemoPageHandler::default();
        handler
            .eval("var received = null; onmessage = function(e) { received = e.data; };".into())
            .unwrap();
        handler.eval("postMessage({greeting: 'hi', n: 7});".into()).unwrap();
        let outbox = handler.take_window_outbox(()).unwrap();
        assert_eq!(outbox.len(), 1);
        handler.deliver_window_message(outbox[0].clone()).unwrap();
        assert_eq!(
            handler
                .eval("received.greeting + ',' + received.n".into())
                .unwrap(),
            "hi,7"
        );
    }

    #[test]
    fn port_send_and_delivery_round_trip() {
        let handler = DemoPageHandler::default();
        handler
            .eval("var got = null; portHandlers['9'] = function(data) { got = data; };".into())
            .unwrap();
        handler.eval("portPostMessage(9, 'over-the-wire');".into()).unwrap();
        let outbox = handler.take_port_outbox(()).unwrap();
        assert_eq!(outbox.len(), 1);
        assert_eq!(outbox[0].port, 9);
        handler.deliver_port_message(outbox[0].clone()).unwrap();
        assert_eq!(handler.eval("got".into()).unwrap(), "over-the-wire");
    }
}
