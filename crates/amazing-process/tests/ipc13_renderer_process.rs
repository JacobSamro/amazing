//! IPC-13: a real renderer process, proved end to end.
//!
//! `amazing_process::renderer` already runs the real engine (`amazing_html`
//! parse, `amazing_layout` style+layout, `amazing_paint` paint -- see
//! `render_document`'s doc comment: "the exact same public engine path runs
//! in the renderer and in a browser-side parity oracle") and already ships
//! frames to the browser process over the sealed-frame channel
//! (`navigate_shared` -> `FrameHandle` -> `acquire_frame` -> the mmap'd
//! `FrameView` `receive_frame` returns). `split.rs`'s
//! `renderer_network_frame_parity_and_input_events` already spawns a real
//! renderer + network process pair and checks both the byte-copy and
//! shared-frame paths against the in-process oracle with inline HTML.
//!
//! This test proves the same contract against an actual fixture *file*
//! (`fixtures/ipc13-page.html`, not a Rust string literal) loaded from disk
//! exactly as a browser process would load a page, and additionally proves
//! that input and navigation keep flowing in both directions across
//! *repeated* round trips on the same live renderer process -- not just one
//! navigate.
use amazing_ipc::codec::ByteBuf;
use amazing_ipc::Error;
use amazing_process::network::{fetch_resource, NetworkBroker};
use amazing_process::protocol::*;
use amazing_process::renderer::render_document;
use amazing_process::{Capabilities, ProcessHost, RestartPolicy, Role};
use std::collections::BTreeMap;
use std::sync::mpsc;
use std::time::Duration;

const FIXTURE: &[u8] = include_bytes!("fixtures/ipc13-page.html");

fn exe() -> &'static std::path::Path {
    std::path::Path::new(env!("CARGO_BIN_EXE_ipc-demo-child"))
}
fn spawn(role: Role) -> ProcessHost {
    ProcessHost::spawn(
        exe(),
        role,
        Capabilities(BTreeMap::new()),
        RestartPolicy::Never,
    )
    .unwrap()
}
struct Events(mpsc::Sender<FrameReady>);
impl BrowserEvents for Events {
    fn frame_ready(&self, event: FrameReady) -> Result<(), Error> {
        self.0.send(event).map_err(|_| Error::Disconnected)
    }
}
fn connect(
    renderer: &ProcessHost,
    network: &ProcessHost,
) -> (RendererProxy, mpsc::Receiver<FrameReady>) {
    let (tx, rx) = mpsc::channel();
    renderer.connection.bind(
        8,
        NetworkReceiver(NetworkBroker {
            remote: NetworkProxy::new(network.connection.clone(), Duration::from_secs(5)),
        }),
    );
    renderer
        .connection
        .bind(9, BrowserEventsReceiver(Events(tx)));
    (
        RendererProxy::new(renderer.connection.clone(), Duration::from_secs(5)),
        rx,
    )
}
fn navigate_request(url: &str, width: u32, height: u32) -> Navigate {
    Navigate {
        url: url.into(),
        html_bytes: ByteBuf(FIXTURE.to_vec()),
        viewport: Viewport { width, height },
    }
}

#[test]
fn real_renderer_process_renders_a_fixture_page_over_the_sealed_frame_channel() {
    let mut network = spawn(Role::Network);
    let mut renderer = spawn(Role::Renderer);
    network.ping(Duration::from_secs(2)).unwrap();
    renderer.ping(Duration::from_secs(2)).unwrap();
    let (proxy, notifications) = connect(&renderer, &network);

    // The in-process parity oracle: the exact same `render_document` engine
    // path amazing_process::renderer::RendererEndpoint calls inside the real
    // renderer process, run here directly against the fixture file.
    let oracle_load = |url: &str, base: &str| {
        fetch_resource(
            &NetworkProxy::new(network.connection.clone(), Duration::from_secs(5)),
            url,
            base,
        )
    };
    let request = navigate_request("http://example.test/ipc13", 96, 200);
    let expected = render_document(&request, oracle_load).unwrap();
    // Sanity: the fixture actually exercises parse+style+layout+paint --
    // background, heading color, stylesheet-colored text, and the SVG image
    // must all have landed distinct pixels, or this proves nothing.
    assert!(
        expected
            .rgba
            .0
            .chunks_exact(4)
            .any(|p| p[0] == 0x20 && p[1] == 0x40 && p[2] == 0x60),
        "inline <style> body background missing from the oracle render"
    );
    assert!(
        expected
            .rgba
            .0
            .chunks_exact(4)
            .any(|p| p[0] < 40 && p[1] > 150 && p[2] < 90),
        "SVG <img> pixels missing from the oracle render"
    );

    // Navigation message browser -> renderer, real frame back over the
    // sealed-frame channel (memfd + SCM_RIGHTS on Linux, not the byte-copy
    // fallback RPC): renderer process actually parses, styles, lays out and
    // paints the fixture, then ships the result to this "browser" process.
    let ready = proxy.navigate_shared(request).unwrap();
    assert_eq!(
        notifications
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .sequence,
        ready.generation,
        "FrameReady notification (renderer -> browser) didn't arrive for the navigation"
    );
    let view = renderer.receive_frame(&ready).unwrap();
    assert_eq!(
        view.bytes(),
        expected.rgba.0,
        "cross-process sealed-frame pixels differ from the in-process render of the same fixture"
    );
    drop(view);
    assert!(proxy
        .release_frame(ReleaseFrame {
            generation: ready.generation
        })
        .unwrap());

    // Input messages browser -> renderer, acked and reflected back as a new
    // frame notification, proving the reverse direction isn't a one-shot.
    for event in [
        InputEvent::Scroll(ScrollDelta { x: 0, y: 3 }),
        InputEvent::Click(Point { x: 1, y: 1 }),
    ] {
        let ack = proxy.input(event).unwrap();
        assert_eq!(
            notifications
                .recv_timeout(Duration::from_secs(2))
                .unwrap()
                .sequence,
            ack.sequence,
            "input ack sequence didn't match the renderer's own FrameReady notification"
        );
    }

    // A second, distinct navigation on the same live renderer process,
    // proving navigation keeps working repeatedly rather than only once at
    // process start -- a different viewport forces a real re-layout/repaint.
    let second_request = navigate_request("http://example.test/ipc13-again", 64, 220);
    let second_expected = render_document(&second_request, oracle_load).unwrap();
    let second_ready = proxy.navigate_shared(second_request).unwrap();
    assert_ne!(second_ready.generation, ready.generation);
    assert_eq!(
        notifications
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .sequence,
        second_ready.generation
    );
    let second_view = renderer.receive_frame(&second_ready).unwrap();
    assert_eq!(second_view.bytes(), second_expected.rgba.0);
    drop(second_view);
    assert!(proxy
        .release_frame(ReleaseFrame {
            generation: second_ready.generation
        })
        .unwrap());

    renderer.kill().unwrap();
    network.kill().unwrap();
}
