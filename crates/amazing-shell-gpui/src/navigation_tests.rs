//! End-to-end shell lifecycle controls using actual loopback transport and engine pages.
use super::*;
use std::{io::{Read, Write}, net::{TcpListener, TcpStream}, sync::{Mutex, atomic::{AtomicBool, AtomicUsize, Ordering}}};
pub(super) struct Fixture {
    address: String,
    released: Arc<AtomicBool>,
    stopped: Arc<AtomicBool>,
    mutated_released: Arc<AtomicBool>,
    requested: Arc<Mutex<Vec<String>>>,
    disconnected: Arc<AtomicUsize>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl Fixture {
    pub(super) fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let released = Arc::new(AtomicBool::new(false));
        let stopped = Arc::new(AtomicBool::new(false));
        let mutated_released = Arc::new(AtomicBool::new(false));
        let requested = Arc::new(Mutex::new(Vec::new()));
        let disconnected = Arc::new(AtomicUsize::new(0));
        let (release, stop, requests, closed, mutated) = (released.clone(), stopped.clone(), requested.clone(), disconnected.clone(), mutated_released.clone());
        let worker = std::thread::spawn(move || {
            let mut handlers = vec![];
            while !stop.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let (release, stop, requests, closed, mutated) = (release.clone(), stop.clone(), requests.clone(), closed.clone(), mutated.clone());
                        handlers.push(std::thread::spawn(move || serve(stream, &release, &stop, &requests, &closed, &mutated)));
                    },
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(2)),
                    Err(error) => panic!("fixture accept: {error}"),
                }
            }
            for handler in handlers { handler.join().unwrap(); }
        });
        Self { address, released, stopped, mutated_released, requested, disconnected, worker: Some(worker) }
    }
    pub(super) fn url(&self, path: &str) -> String { format!("{}{path}", self.address) }
    fn seen(&self, path: &str) -> usize { self.requested.lock().unwrap().iter().filter(|value| value.as_str() == path).count() }
    fn release(&self) { self.released.store(true, Ordering::Release); }
    fn release_mutated(&self) { self.mutated_released.store(true, Ordering::Release); }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.release(); self.release_mutated(); self.stopped.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() { worker.join().unwrap(); }
    }
}
fn serve(mut stream: TcpStream, released: &AtomicBool, stopped: &AtomicBool,
    requested: &Mutex<Vec<String>>, disconnected: &AtomicUsize, mutated: &AtomicBool) {
    stream.set_read_timeout(Some(Duration::from_millis(25))).unwrap();
    let mut bytes = Vec::new(); let mut buffer = [0; 4096];
    while !bytes.windows(4).any(|value| value == b"\r\n\r\n") {
        match stream.read(&mut buffer) { Ok(0) => return, Ok(count) => bytes.extend_from_slice(&buffer[..count]),
            Err(error) if matches!(error.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {
                if stopped.load(Ordering::Acquire) { return; }
            }, Err(_) => return }
    }
    let header_end = bytes.windows(4).position(|value| value == b"\r\n\r\n").unwrap() + 4;
    let length = String::from_utf8_lossy(&bytes[..header_end]).lines()
        .filter_map(|line| line.split_once(':'))
        .find_map(|(name, value)| name.eq_ignore_ascii_case("content-length").then(|| value.trim().parse::<usize>().unwrap())).unwrap_or(0);
    while bytes.len() < header_end + length {
        match stream.read(&mut buffer) { Ok(0) => return, Ok(count) => bytes.extend_from_slice(&buffer[..count]), Err(_) => return }
    }
    let request = String::from_utf8_lossy(&bytes);
    let path = request.split_whitespace().nth(1).unwrap_or("/").to_owned();
    requested.lock().unwrap().push(path.clone());
    if path.starts_with("/slow-") {
        let released = if path == "/slow-mutated.ppm" { mutated } else { released };
        while !released.load(Ordering::Acquire) && !stopped.load(Ordering::Acquire) {
            match stream.read(&mut buffer[..1]) { Ok(0) => { disconnected.fetch_add(1, Ordering::AcqRel); return; },
                Err(error) if matches!(error.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {},
                Err(_) => { disconnected.fetch_add(1, Ordering::AcqRel); return; }, _ => {} }
        }
    }
    if path == "/slow-font.ttf" {
        let font = std::fs::read("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf").expect("native fixture font");
        let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: font/ttf\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", font.len());
        let _ = stream.write_all(&font); return;
    }
    if path == "/failure" { return; }
    if path == "/redirect" {
        let _ = stream.write_all(b"HTTP/1.1 302 Found\r\nLocation: /normal\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"); return;
    }
    let (kind, body, extra) = match path.as_str() {
        "/post" => {
            assert!(request.starts_with("POST /post HTTP/1.1"));
            assert!(request.contains("application/x-www-form-urlencoded"));
            assert_eq!(&bytes[header_end..header_end + length], b"q=responsive+browser");
            ("text/html", "<title>Posted</title><h1>Posted form</h1>", "")
        },
        "/font-script" => ("text/html", "<title>Before font</title><style>@font-face{font-family:NavProof;src:url('/slow-font.ttf');font-display:swap}h1{font-family:NavProof}</style><h1>Fonts ready</h1><script>var n=Number(localStorage.getItem('fontRuns')||'0')+1;localStorage.setItem('fontRuns',String(n));document.title='Font once '+n;</script>", ""),
        "/font-continuation" => ("text/html", "<title>Continuation initial</title><style>@font-face{font-family:NavProof;src:url('/slow-font.ttf');font-display:swap}h1{font-family:NavProof}img{width:20px;height:20px}</style><h1>Continuation ready</h1><img id=dynamic><script>var n=Number(localStorage.getItem('continuationRuns')||'0')+1;localStorage.setItem('continuationRuns',String(n));setTimeout(function(){var m=Number(localStorage.getItem('continuationCallbacks')||'0')+1;localStorage.setItem('continuationCallbacks',String(m));document.querySelector('#dynamic').setAttribute('src','/slow-mutated.ppm');document.title='Continuation callback '+m;},20);</script>", ""),
        "/font-static" => ("text/html", "<title>Static font</title><style>@font-face{font-family:NavProof;src:url('/slow-font.ttf');font-display:swap}h1{font-family:NavProof}</style><h1>Fonts ready</h1>", ""),
        "/resource" => ("text/html", "<title>Resources</title><link rel=stylesheet href=/slow-style.css><link rel=stylesheet href=/slow-style-two.css><h1>Resource page</h1><img src=/slow-image.ppm><img src=/slow-image-two.ppm>", ""),
        "/removed" => ("text/html", "<title>Removed resource</title><img id=remove src=/slow-removed.ppm><script>document.querySelector('#remove').removeAttribute('src'); document.title = 'Removed before acquisition';</script>", ""),
        "/script" => ("text/html", "<title>Before</title><p id=message>Before</p><img id=dynamic><script src=/slow-script.js></script>", ""),
        "/slow-script.js" => ("text/javascript", "var n = Number(localStorage.getItem('runs') || '0') + 1; localStorage.setItem('runs', String(n)); document.title = 'Once ' + n; document.querySelector('#message').textContent = 'Executed once'; document.querySelector('#dynamic').setAttribute('src', '/slow-mutated.ppm');", ""),
        "/slow-style.css" | "/slow-style-two.css" => ("text/css", "h1 {color:#123456} body {background:#fafafa}", ""),
        "/slow-image.ppm" | "/slow-image-two.ppm" | "/slow-mutated.ppm" => ("image/x-portable-pixmap", "P3\n1 1\n255\n20 40 60\n", ""),
        "/cookie" => ("text/html", "<title>Cookie</title><h1>Cookie page</h1>", "Set-Cookie: nav=kept; Path=/\r\nCache-Control: max-age=3600\r\n"),
        "/check-cookie" => { assert!(request.contains("nav=kept")); ("text/html", "<title>Cookie kept</title><h1>Cookie kept</h1>", "") },
        "/slow-document" => ("text/html", "<title>Slow document</title><h1>Slow document</h1>", ""),
        _ => ("text/html", "<title>Normal</title><h1>Normal page</h1><p>Ready</p>", ""),
    };
    let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nContent-Length: {}\r\n{extra}Connection: close\r\n\r\n{body}", body.len());
}
fn pump_until(shell: &mut BrowserShell, predicate: impl Fn(&BrowserShell) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !predicate(shell) {
        shell.poll_navigation();
        assert!(Instant::now() < deadline, "navigation condition deadline");
        std::thread::sleep(Duration::from_millis(3));
    }
}
fn navigate(shell: &mut BrowserShell, address: String) { shell.address = address; shell.commit_navigation(); }
fn settle(shell: &mut BrowserShell) { pump_until(shell, |shell| shell.pending_navigation.iter().all(Option::is_none)); }

#[test]
fn delayed_document_keeps_visible_page_and_stop_closes_actual_socket() {
    let fixture = Fixture::new(); let mut shell = BrowserShell::new(None, None);
    let visible = shell.tabs[0].viewport.as_ref().unwrap().image.clone();
    navigate(&mut shell, fixture.url("/slow-document"));
    pump_until(&mut shell, |_| fixture.seen("/slow-document") == 1);
    assert!(Arc::ptr_eq(&visible, &shell.tabs[0].viewport.as_ref().unwrap().image));
    assert_eq!(shell.committed_address, "amazing://status");
    shell.stop_navigation();
    let deadline = Instant::now() + Duration::from_secs(2);
    while fixture.disconnected.load(Ordering::Acquire) == 0 {
        assert!(Instant::now() < deadline, "stop must retire actual request"); std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(shell.address, "amazing://status"); assert_eq!(shell.tabs[0].history.len(), 1);
}
#[test]
fn supersession_and_closed_tab_results_never_publish() {
    let fixture = Fixture::new(); let mut shell = BrowserShell::new(None, None);
    navigate(&mut shell, fixture.url("/slow-document"));
    pump_until(&mut shell, |_| fixture.seen("/slow-document") == 1);
    navigate(&mut shell, fixture.url("/normal")); settle(&mut shell);
    assert_eq!(shell.committed_address, fixture.url("/normal")); assert_eq!(shell.tabs[0].history.len(), 2);
    shell.add_tab(); let closed = shell.active;
    navigate(&mut shell, fixture.url("/slow-document"));
    pump_until(&mut shell, |_| fixture.seen("/slow-document") == 2);
    shell.close_tab(closed); fixture.release();
    for _ in 0..50 { shell.poll_navigation(); std::thread::sleep(Duration::from_millis(2)); }
    assert!(!shell.tabs[closed].open); assert_eq!(shell.tabs[closed].url, "amazing://newtab");
    assert_eq!(shell.tabs[0].url, fixture.url("/normal"));
}
#[test]
fn delayed_resources_keep_other_tabs_responsive_and_preserve_density() {
    for scale in [1.0, 2.0] {
        let fixture = Fixture::new(); let mut shell = BrowserShell::new(None, None);
        shell.viewport_metrics = ViewportMetrics::new(320.0, 240.0, scale).unwrap();
        navigate(&mut shell, fixture.url("/resource"));
        pump_until(&mut shell, |_| fixture.seen("/slow-style.css") == 1 && fixture.seen("/slow-image.ppm") == 1
            && fixture.seen("/slow-style-two.css") == 1 && fixture.seen("/slow-image-two.ppm") == 1);
        shell.add_tab(); let other = shell.active;
        navigate(&mut shell, fixture.url("/normal"));
        pump_until(&mut shell, |shell| shell.tabs[other].title == "Normal");
        assert!(shell.pending_navigation[0].is_some());
        fixture.release(); settle(&mut shell);
        assert_eq!(shell.tabs[0].title, "Resources"); assert_eq!(shell.tabs[other].title, "Normal");
        assert_eq!(shell.tabs[0].viewport.as_ref().unwrap().metrics, shell.viewport_metrics);
    }
}
#[test]
fn external_script_and_mutated_resource_execute_author_once() {
    let fixture = Fixture::new(); let mut shell = BrowserShell::new(None, None);
    navigate(&mut shell, fixture.url("/script"));
    pump_until(&mut shell, |_| fixture.seen("/slow-script.js") == 1);
    assert_eq!(shell.session.borrow().storage_get(&fixture.address, "runs"), None);
    fixture.release();
    pump_until(&mut shell, |_| fixture.seen("/slow-mutated.ppm") == 1);
    assert_eq!(shell.session.borrow().storage_get(&fixture.address, "runs"), Some("1".into()));
    assert!(shell.pending_navigation[0].is_some());
    fixture.release_mutated(); settle(&mut shell);
    assert_eq!(shell.tabs[0].title, "Once 1");
    assert_eq!(shell.session.borrow().storage_get(&fixture.address, "runs"), Some("1".into()));
    assert_eq!(fixture.seen("/slow-script.js"), 1); assert_eq!(fixture.seen("/slow-mutated.ppm"), 1);
}
#[test]
fn redirect_error_history_and_session_cache_recover() {
    let fixture = Fixture::new(); let mut shell = BrowserShell::new(None, None);
    navigate(&mut shell, fixture.url("/redirect")); settle(&mut shell);
    assert_eq!(shell.committed_address, fixture.url("/normal"));
    navigate(&mut shell, fixture.url("/failure")); settle(&mut shell);
    assert_eq!(shell.committed_address, fixture.url("/failure")); assert!(shell.tabs[0].viewport.is_some());
    shell.go_back(); settle(&mut shell); assert_eq!(shell.committed_address, fixture.url("/normal"));
    shell.go_forward(); settle(&mut shell); assert_eq!(shell.committed_address, fixture.url("/failure"));
    navigate(&mut shell, fixture.url("/cookie")); settle(&mut shell);
    navigate(&mut shell, fixture.url("/check-cookie")); settle(&mut shell);
    shell.go_back(); settle(&mut shell);
    // The first response sets a cookie, so its anonymous cache partition cannot
    // satisfy the first authenticated traversal. Subsequent traversal must hit it.
    assert_eq!(fixture.seen("/cookie"), 2); assert_eq!(shell.tabs[0].title, "Cookie");
    navigate(&mut shell, fixture.url("/check-cookie")); settle(&mut shell);
    shell.go_back(); settle(&mut shell);
    assert_eq!(fixture.seen("/cookie"), 2, "same-cookie history traversal must use fresh cache");
    assert_eq!(shell.tabs[0].title, "Cookie");
}

#[test]
fn author_removed_resource_is_never_acquired_from_pre_script_dom() {
    let fixture = Fixture::new(); let mut shell = BrowserShell::new(None, None);
    navigate(&mut shell, fixture.url("/removed")); settle(&mut shell);
    assert_eq!(shell.tabs[0].title, "Removed before acquisition");
    assert_eq!(fixture.seen("/slow-removed.ppm"), 0);
}

#[test]
fn retained_author_effect_is_once_across_stop_supersession_and_tab_close() {
    for action in ["stop", "supersede", "close"] {
        let fixture = Fixture::new(); let mut shell = BrowserShell::new(None, None);
        if action == "close" { shell.add_tab(); }
        let original = shell.active;
        navigate(&mut shell, fixture.url("/script")); fixture.release();
        pump_until(&mut shell, |_| fixture.seen("/slow-mutated.ppm") == 1);
        assert_eq!(shell.session.borrow().storage_get(&fixture.address, "runs"), Some("1".into()));
        match action {
            "stop" => shell.stop_navigation(),
            "supersede" => navigate(&mut shell, fixture.url("/normal")),
            "close" => shell.close_tab(original),
            _ => unreachable!(),
        }
        fixture.release_mutated(); settle(&mut shell);
        assert_eq!(shell.session.borrow().storage_get(&fixture.address, "runs"), Some("1".into()));
        assert_eq!(fixture.seen("/slow-script.js"), 1);
        assert_eq!(fixture.seen("/slow-mutated.ppm"), 1);
        assert_ne!(shell.tabs[original].title, "Once 1", "cancelled document must not publish");
        if action == "close" { assert!(!shell.tabs[original].open); }
    }
}

#[test]
fn reload_pending_request_keeps_target_and_deferred_history() {
    let fixture = Fixture::new(); let mut shell = BrowserShell::new(None, None);
    let target = fixture.url("/slow-document");
    navigate(&mut shell, target.clone());
    pump_until(&mut shell, |_| fixture.seen("/slow-document") == 1);
    let first = shell.pending_navigation[0].as_ref().unwrap().id;
    shell.reload();
    assert_ne!(shell.pending_navigation[0].as_ref().unwrap().id, first);
    assert_eq!(shell.pending_navigation[0].as_ref().unwrap().address, target);
    assert_eq!(shell.tabs[0].history.len(), 1);
    pump_until(&mut shell, |_| fixture.seen("/slow-document") == 2);
    fixture.release(); settle(&mut shell);
    assert_eq!(shell.committed_address, target);
    assert_eq!(shell.tabs[0].history, ["amazing://status".to_owned(), target]);
}

#[test]
fn background_navigation_preserves_form_method_body_and_committed_history() {
    let fixture = Fixture::new(); let mut shell = BrowserShell::new(None, None);
    let target = fixture.url("/post");
    shell.commit_request(target.clone(), "POST", b"q=responsive+browser".to_vec());
    settle(&mut shell);
    assert_eq!(shell.tabs[0].title, "Posted");
    assert_eq!(shell.committed_address, target);
    assert_eq!(fixture.seen("/post"), 1);
    assert_eq!(shell.tabs[0].history.len(), 2);
}

#[test]
fn pending_real_font_publishes_page_then_reflows_same_author_once_runtime() {
    for scale in [1.0, 2.0] {
        let fixture = Fixture::new(); let mut shell = BrowserShell::new(None, None);
        shell.viewport_metrics = ViewportMetrics::new(320.0, 240.0, scale).unwrap();
        let url = fixture.url("/font-script"); navigate(&mut shell, url.clone());
        pump_until(&mut shell, |shell| fixture.seen("/slow-font.ttf") == 1 && shell.tabs[0].title == "Font once 1");
        assert_eq!(shell.committed_address, url);
        assert!(shell.pending_navigation[0].as_ref().unwrap().committed);
        assert_eq!(shell.tabs[0].history.len(), 2);
        let arena = shell.page_runtimes[0].as_ref().unwrap().navigation_runtime_identity();
        let reflow = shell.page_runtimes[0].as_mut().unwrap().reflow(shell.viewport_metrics).unwrap();
        assert_eq!(reflow.metrics, shell.viewport_metrics);
        assert_eq!(fixture.seen("/slow-font.ttf"), 1, "pending font reflow must not open a UI-thread duplicate");
        shell.add_tab(); let other = shell.active; navigate(&mut shell, fixture.url("/normal"));
        pump_until(&mut shell, |shell| shell.tabs[other].title == "Normal");
        assert!(shell.pending_navigation[0].is_some());
        fixture.release(); settle(&mut shell);
        assert_eq!(shell.page_runtimes[0].as_ref().unwrap().navigation_runtime_identity(), arena);
        assert_eq!(shell.tabs[0].title, "Font once 1"); assert_eq!(shell.tabs[0].history.len(), 2);
        assert_eq!(shell.tabs[0].viewport.as_ref().unwrap().metrics, shell.viewport_metrics);
        shell.select_tab(0); navigate(&mut shell, url); settle(&mut shell);
        assert_eq!(shell.tabs[0].title, "Font once 2");
    }
}
#[test]
fn stopping_font_after_fallback_commit_keeps_page_and_rejects_late_delivery() {
    let fixture = Fixture::new(); let mut shell = BrowserShell::new(None, None);
    let url = fixture.url("/font-static"); navigate(&mut shell, url.clone());
    pump_until(&mut shell, |shell| fixture.seen("/slow-font.ttf") == 1 && shell.tabs[0].title == "Static font");
    let owner = shell.tabs[0].viewport.as_ref().unwrap().identity();
    shell.stop_navigation();
    assert!(shell.tabs[0].viewport.as_ref().unwrap().accepts_identity(&owner), "stop keeps original static document owner");
    let visible = shell.tabs[0].viewport.as_ref().unwrap().image.clone();
    shell.tabs[0].viewport.as_ref().unwrap().try_reflow_static(shell.viewport_metrics).unwrap();
    assert_eq!(fixture.seen("/slow-font.ttf"), 1, "stop then reflow must retain the cancelled font result");
    assert_eq!(shell.committed_address, url); assert_eq!(shell.tabs[0].history.len(), 2);
    fixture.release();
    for _ in 0..30 { shell.poll_navigation(); std::thread::sleep(Duration::from_millis(3)); }
    assert!(shell.pending_navigation[0].is_none());
    assert!(Arc::ptr_eq(&visible, &shell.tabs[0].viewport.as_ref().unwrap().image));
}

#[test]
fn authored_timer_resources_progress_and_cancel_while_swap_font_stays_pending() {
    for scale in [1.0, 2.0] { for action in ["complete", "stop", "supersede", "close"] {
        let fixture = Fixture::new(); let mut shell = BrowserShell::new(None, None);
        shell.viewport_metrics = ViewportMetrics::new(320.0, 240.0, scale).unwrap();
        if action == "close" { shell.add_tab(); }
        let index = shell.active;
        navigate(&mut shell, fixture.url("/font-continuation"));
        pump_until(&mut shell, |shell| shell.pending_navigation[index].as_ref().is_some_and(|pending| pending.committed));
        let arena = shell.page_runtimes[index].as_ref().unwrap().navigation_runtime_identity();
        std::thread::sleep(Duration::from_millis(30)); shell.pump_active_runtime();
        assert_eq!(shell.tabs[index].title, "Continuation callback 1");
        pump_until(&mut shell, |_| fixture.seen("/slow-mutated.ppm") == 1);
        assert!(shell.pending_navigation[index].is_some());
        assert_eq!(shell.tabs[index].viewport.as_ref().unwrap().decoded_image_count(), 0);
        match action {
            "complete" => {
                fixture.release_mutated();
                pump_until(&mut shell, |shell| shell.tabs[index].viewport.as_ref().unwrap().decoded_image_count() == 1);
                assert!(shell.pending_navigation[index].is_some(), "image must render before the held font completes");
                assert_eq!(shell.page_runtimes[index].as_ref().unwrap().navigation_runtime_identity(), arena);
            },
            "stop" => { shell.stop_navigation(); assert!(shell.pending_navigation[index].is_none());
                assert_eq!(shell.tabs[index].viewport.as_ref().unwrap().decoded_image_count(), 0);
                assert_eq!(fixture.seen("/slow-mutated.ppm"), 1, "stop reflow cannot reopen the cancelled image"); },
            "supersede" => { navigate(&mut shell, fixture.url("/normal")); },
            "close" => shell.close_tab(index),
            _ => unreachable!(),
        }
        fixture.release_mutated(); fixture.release(); settle(&mut shell);
        for _ in 0..30 { shell.poll_navigation(); std::thread::sleep(Duration::from_millis(3)); }
        assert_eq!(fixture.seen("/slow-font.ttf"), 1); assert_eq!(fixture.seen("/slow-mutated.ppm"), 1);
        assert_eq!(shell.session.borrow().storage_get(&fixture.address, "continuationRuns"), Some("1".into()));
        assert_eq!(shell.session.borrow().storage_get(&fixture.address, "continuationCallbacks"), Some("1".into()));
        if action == "stop" { assert_eq!(shell.tabs[index].title, "Continuation callback 1");
            assert_eq!(shell.tabs[index].viewport.as_ref().unwrap().decoded_image_count(), 0); }
        if action == "supersede" { assert_eq!(shell.tabs[index].title, "Normal"); }
        if action == "close" { assert!(!shell.tabs[index].open); assert!(shell.page_runtimes[index].is_none()); }
    } }
}
