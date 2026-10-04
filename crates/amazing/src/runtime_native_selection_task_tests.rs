//! Native task FIFO tests use genuine installed bindings and the actual scheduler.
use super::*;

fn driver(capacity: usize) -> DriverScheduler {
    let host = Rc::new(RefCell::new(BrowserDomHost {
        dom: amazing_html::parse("<body></body>"), native_forwarded: false,
    }));
    let mut driver = DriverScheduler::new(host, capacity).unwrap();
    driver.set_location_url("https://example.test/native-selection");
    driver.run_script("var log=[], text=document.createTextNode('abc'); document.body.appendChild(text); document.onselectionchange=()=>log.push('selection');").unwrap();
    driver
}
fn pump(driver: &mut DriverScheduler) { driver.pump_at(Duration::from_millis(1), false).unwrap(); }
fn check(driver: &mut DriverScheduler, source: &str) {
    assert_eq!(driver.run_script(source).unwrap(), JsValue::Bool(true), "{source}");
}

#[test]
fn native_selection_preserves_both_author_orders_and_microtask_checkpoint() {
    for mutation_first in [true, false] {
        let mut d = driver(64);
        let mutation = "getSelection().setPosition(text,1);getSelection().setPosition(text,2);";
        let timer = "setTimeout(()=>log.push('timer'),0);";
        let script = if mutation_first { format!("{mutation}{timer}") } else { format!("{timer}{mutation}") };
        d.js.run_script(&(script + "Promise.resolve().then(()=>log.push('promise'));" )).unwrap();
        let requests = d.js.take_scheduling_requests();
        let order: Vec<_> = requests.iter().filter_map(|request| match request {
            SchedulingRequest::NativeTask { .. } => Some("selection"),
            SchedulingRequest::Timer { .. } => Some("timer"),
            _ => None,
        }).collect();
        assert_eq!(order, if mutation_first { vec!["selection", "timer"] } else { vec!["timer", "selection"] });
        d.pending.extend(requests);
        d.drain_outbox().unwrap();
        pump(&mut d);
        // Cross-source choice belongs to EventLoop's documented policy. The
        // bridge promises actual emission order, coalescing and checkpoints.
        check(&mut d, "log.length===3&&log[0]==='promise'&&log.filter(x=>x==='selection').length===1&&log.filter(x=>x==='timer').length===1");
    }
}

#[test]
fn native_selection_range_mutation_listener_reentry_and_abrupt_conversion() {
    let mut d = driver(64);
    d.run_script(r#"
        var sentinel={}, caught=false, selectionSeen=0;
        document.onselectionchange=function(){log.push('selection');
            if(++selectionSeen===1)getSelection().getRangeAt(0).setEnd(text,3);};
        try { getSelection().setPosition(text,{valueOf(){getSelection().setPosition(text,1);throw sentinel;}}); }
        catch(e){caught=e===sentinel;}
        setTimeout(()=>log.push('timer'),0);
    "#).unwrap();
    check(&mut d, "caught&&log.length===0");
    pump(&mut d);
    check(&mut d, "log.length===3&&selectionSeen===2&&log.filter(x=>x==='timer').length===1&&getSelection().getRangeAt(0).endOffset===3");
    d.run_script("log=[];document.onselectionchange=()=>log.push('selection');text.remove();setTimeout(()=>log.push('timer'),0);").unwrap();
    d.pump_at(Duration::from_millis(2), false).unwrap();
    check(&mut d, "log.length===2&&log.includes('selection')&&log.includes('timer')");
}

#[test]
fn native_selection_full_js_quota_and_partial_abrupt_dom_mutation() {
    let mut d = driver(1);
    d.run_script("var moCount=0;new MutationObserver(()=>++moCount).observe(text,{characterData:true});setTimeout(()=>log.push('timer'),0);getSelection().setPosition(text,1);text.data='abcd';").unwrap();
    pump(&mut d);
    check(&mut d, "log.length===2&&log.includes('timer')&&log.includes('selection')&&moCount===1");
    // A lower bulk operation can commit a selection mutation before an error.
    // The adapter must emit it after releasing Dom and retain the earlier error.
    let bindings = d.bindings.clone();
    let error = bindings.test_partial_selection_mutation(&mut d.js).unwrap_err();
    assert!(matches!(error.0, JsValue::Object(_)));
    d.run_script("setTimeout(()=>log.push('later'),0)").unwrap();
    d.pump_at(Duration::from_millis(2), false).unwrap();
    check(&mut d, "log.length===4&&log.filter(x=>x==='selection').length===2&&log.includes('timer')&&log.includes('later')");
}

#[test]
fn native_selection_failed_admission_keeps_front_and_payload_then_releases() {
    let mut d = driver(64);
    d.next_callback = u64::MAX;
    assert!(d.run_script("getSelection().setPosition(text,1);setTimeout(()=>log.push('timer'),0)").is_err());
    assert!(matches!(d.pending.front(), Some(SchedulingRequest::NativeTask { .. })));
    let token = match d.pending.front().copied().unwrap() { SchedulingRequest::NativeTask { token } => token, _ => unreachable!() };
    assert!(d.bindings.native_selection_task(token).unwrap().is_some());
    assert!(d.payloads.is_empty());
    // Recover the injected downstream callback-ID exhaustion; no request was lost.
    d.next_callback = 1;
    pump(&mut d);
    check(&mut d, "log.length===2&&log.includes('selection')&&log.includes('timer')");
    assert!(d.bindings.native_selection_task(token).is_err());
    d.run_script("getSelection().setPosition(text,2)").unwrap();
    // The next allocation never reuses the released opaque slot.
    let newer = d.payloads.values().find_map(|p| match p { Payload::NativeSelection(t) => Some(*t), _ => None }).unwrap();
    assert!(newer.slot() > token.slot());
    assert!(d.admit(SchedulingRequest::NativeTask { token: NativeTaskToken::new(newer.owner()+1,newer.slot()) }).is_err());
    assert!(d.admit(SchedulingRequest::NativeTask { token: newer }).is_err());
    d.pump_at(Duration::from_millis(2), false).unwrap();
    check(&mut d, "log.length===3");
}

#[test]
fn native_selection_stale_generation_and_removed_child_drop_without_cross_document_delivery() {
    let mut d = driver(64);
    // A captured-generation mismatch exercises admission's stale branch. The
    // lower native group separately performs real retirement and slot reuse.
    let bindings = d.bindings.clone();
    d.js.run_script("getSelection().setPosition(text,1)").unwrap();
    let requests = d.js.take_scheduling_requests();
    let token = requests.iter().find_map(|request| match request {
        SchedulingRequest::NativeTask { token } => Some(*token), _ => None,
    }).unwrap();
    let original = bindings.test_stale_native_generation(token);
    d.pending.extend(requests);
    d.drain_outbox().unwrap();
    assert!(bindings.test_native_tasks_empty());
    // Discarding a mismatched snapshot must not clear a genuine current flag.
    assert!(d.host.borrow_mut().dom.begin_selection_change(original));
    pump(&mut d);
    check(&mut d, "log.length===0");
    d.run_script(r#"
        log=[];var frame=document.createElement('iframe');document.body.appendChild(frame);
        var child=frame.contentWindow,ct=child.document.createTextNode('x');child.document.body.appendChild(ct);
        child.document.onselectionchange=()=>log.push('child');
        Selection.prototype.setPosition.call(child.getSelection(),ct,0);frame.remove();
        setTimeout(()=>log.push('timer'),0);
    "#).unwrap();
    d.pump_at(Duration::from_millis(2), false).unwrap();
    check(&mut d, "log.join(',')==='timer'");
}

#[test]
fn native_selection_global_teardown_retires_tasks_while_bindings_are_retained() {
    let mut d = driver(64);
    d.run_script("getSelection().setPosition(text,1)").unwrap();
    let bindings = d.bindings.clone();
    let host = d.host.clone();
    assert!(!bindings.test_native_tasks_empty());
    drop(d); // The admitted callback never executes; bindings remain held here.
    assert!(bindings.test_native_tasks_empty());
    let mut host = host.borrow_mut();
    host.dom.selection_remove_all_ranges(amazing_dom::NodeIdx(0)).unwrap();
    // Teardown cleared the exact old document flag, permitting a fresh event.
    assert_eq!(host.dom.pending_selection_change_tasks().len(), 1);
}
