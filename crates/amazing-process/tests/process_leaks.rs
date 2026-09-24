#![cfg(any(target_os = "linux", windows))]
use amazing_ipc::codec::ByteBuf;
use amazing_process::protocol::{
    BrowserEvents, BrowserEventsReceiver, FrameReady, Navigate, RendererProxy, Viewport,
};
use amazing_process::{Capabilities, ProcessHost, RestartPolicy, Role};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

struct Events;
impl BrowserEvents for Events {
    fn frame_ready(&self, _: FrameReady) -> Result<(), amazing_ipc::Error> {
        Ok(())
    }
}
#[cfg(target_os = "linux")]
fn handle_count() -> usize {
    std::fs::read_dir("/proc/self/fd").unwrap().count()
}
#[cfg(windows)]
fn handle_count() -> usize {
    amazing_ipc_platform::current_process_handle_count().unwrap() as usize
}

#[test]
fn thousand_spawn_navigate_map_kill_cycles_bound_handles() {
    let exe = std::path::Path::new(env!("CARGO_BIN_EXE_ipc-demo-child"));
    let baseline = handle_count();
    for iteration in 0..1000 {
        let mut renderer = ProcessHost::spawn(
            exe,
            Role::Renderer,
            Capabilities(BTreeMap::new()),
            RestartPolicy::Never,
        )
        .unwrap();
        renderer.connection.bind(9, BrowserEventsReceiver(Events));
        let proxy = RendererProxy::new(renderer.connection.clone(), Duration::from_secs(5));
        let ready = proxy
            .navigate_shared(Navigate {
                url: "http://example.test/leak".into(),
                html_bytes: ByteBuf(b"<p>frame</p>".to_vec()),
                viewport: Viewport {
                    width: 16,
                    height: 16,
                },
            })
            .unwrap();
        let view = renderer.receive_frame(&ready).unwrap();
        assert_eq!(view.bytes().len(), 16 * 16 * 4);
        renderer.kill().unwrap();
        drop(view);
        drop(renderer);
        if iteration % 100 == 99 {
            assert!(
                handle_count() <= baseline + 64,
                "handle count grew by iteration {iteration}"
            );
        }
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while handle_count() > baseline + 16 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        handle_count() <= baseline + 16,
        "handle count did not return to baseline"
    );
}
