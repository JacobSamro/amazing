//! `$262.agent` for the Test262 host (INTERPRETING.md "agent" API).
//!
//! Every agent is an OS thread with its own `Interpreter`; SharedArrayBuffer
//! memory is shared through `amazing_js::shared_memory::SharedBlock`.
//! Agent-side callbacks run from the thread's receive loop, followed by a
//! microtask checkpoint. Main-side `broadcast` returns once every live agent
//! has received the message.
use amazing_js::shared_memory::{self, SharedBlock};
use amazing_js::value::{JsValue, Throw};
use amazing_js::{Interpreter, parse_script};
use std::{
    cell::RefCell,
    collections::VecDeque,
    sync::{mpsc, Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

enum Message {
    Broadcast { block: Arc<SharedBlock>, id: f64, ack: mpsc::Sender<()> },
}

static AGENTS: Mutex<Vec<mpsc::Sender<Message>>> = Mutex::new(Vec::new());
static REPORTS: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());
static START: OnceLock<Instant> = OnceLock::new();

thread_local! {
    static RECEIVER: RefCell<Option<JsValue>> = const { RefCell::new(None) };
}

fn arg(args: &[JsValue], index: usize) -> JsValue {
    args.get(index).cloned().unwrap_or(JsValue::Undefined)
}

pub fn start(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let source = crate::coerce(i, arg(&args, 0))?;
    if parse_script(&source).is_err() {
        return Err(Throw::type_error("agent source does not parse"));
    }
    let (tx, rx) = mpsc::channel();
    shared_memory::note_agent_started();
    AGENTS.lock().unwrap_or_else(|p| p.into_inner()).push(tx);
    std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(move || agent_thread(source, rx))
        .map_err(|e| Throw::type_error(format!("cannot start agent: {e}")))?;
    Ok(JsValue::Undefined)
}

fn agent_thread(source: String, rx: mpsc::Receiver<Message>) {
    let mut i = Interpreter::new();
    crate::install(&mut i);
    if i.run_script(&source).is_err() {
        return;
    }
    let _ = i.drain_microtasks();
    // Event loop: broadcasts plus Atomics.waitAsync settlement, polled at 1 ms
    // while a timed waiter is pending and at 20 ms otherwise.
    let mut pending = i.binding_pump_atomics();
    loop {
        let wait = Duration::from_millis(if pending { 1 } else { 20 });
        match rx.recv_timeout(wait) {
            Ok(Message::Broadcast { block, id, ack }) => {
                let _ = ack.send(());
                let callback = RECEIVER.with(|r| r.borrow().clone());
                if let Some(callback) = callback {
                    let buffer = i.binding_shared_buffer_from_block(block);
                    let _ = i.call_value(&callback, JsValue::Undefined, vec![buffer, JsValue::num(id)]);
                    let _ = i.drain_microtasks();
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        pending = i.binding_pump_atomics();
        if i.binding_has_pending_timers() {
            let _ = i.pump_timers();
            pending = true;
        }
    }
}

pub fn broadcast(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let block = i
        .binding_shared_buffer_block(&arg(&args, 0))
        .ok_or_else(|| Throw::type_error("broadcast requires a SharedArrayBuffer"))?;
    let id = match args.get(1) {
        Some(v) => i.binding_to_number(v)?,
        None => f64::NAN,
    };
    let senders: Vec<_> = AGENTS.lock().unwrap_or_else(|p| p.into_inner()).clone();
    let mut acks = Vec::new();
    for sender in senders {
        let (ack, wait) = mpsc::channel();
        if sender.send(Message::Broadcast { block: block.clone(), id, ack }).is_ok() {
            acks.push(wait);
        }
    }
    for wait in acks {
        let _ = wait.recv();
    }
    Ok(JsValue::Undefined)
}

pub fn get_report(_: &mut Interpreter, _: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    Ok(match REPORTS.lock().unwrap_or_else(|p| p.into_inner()).pop_front() {
        Some(report) => JsValue::str(&report),
        None => JsValue::Null,
    })
}

pub fn sleep(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let ms = i.binding_to_number(&arg(&args, 0))?;
    if ms.is_finite() && ms > 0.0 {
        std::thread::sleep(Duration::from_secs_f64(ms.min(60_000.0) / 1000.0));
    }
    Ok(JsValue::Undefined)
}

pub fn monotonic_now(_: &mut Interpreter, _: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let start = START.get_or_init(Instant::now);
    Ok(JsValue::num(start.elapsed().as_secs_f64() * 1000.0))
}

pub fn receive_broadcast(_: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    RECEIVER.with(|r| *r.borrow_mut() = Some(arg(&args, 0)));
    Ok(JsValue::Undefined)
}

pub fn report(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let text = crate::coerce(i, arg(&args, 0))?;
    REPORTS.lock().unwrap_or_else(|p| p.into_inner()).push_back(text);
    Ok(JsValue::Undefined)
}

pub fn leaving(_: &mut Interpreter, _: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    Ok(JsValue::Undefined)
}
