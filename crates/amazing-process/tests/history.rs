use amazing_ipc::codec::ByteBuf;
use amazing_process::history::{
    CommitKind, DocumentStatus, Eligibility, HistoryError, HistorySnapshot, SessionHistory,
    FROZEN_TTL_MS, MAX_FROZEN,
};
use amazing_process::protocol::{
    BrowserEvents, BrowserEventsReceiver, FrameReady, HistoryActivate, HistoryLifecycleEvent,
    HistoryProxy, InputEvent, Navigate, RendererProxy, ScrollDelta, Viewport,
};
use amazing_process::{Capabilities, ProcessHost, RestartPolicy, Role};
use std::collections::BTreeMap;
use std::time::Duration;
struct Events;
impl BrowserEvents for Events {
    fn frame_ready(&self, _: FrameReady) -> Result<(), amazing_ipc::Error> {
        Ok(())
    }
}

fn commit(
    history: &mut SessionHistory,
    renderer: u64,
    navigable: u64,
    source: Option<u64>,
    kind: CommitKind,
    url: &str,
) -> amazing_process::history::SessionHistoryEntry {
    let ticket = history.begin(renderer, navigable, source, kind).unwrap();
    history
        .commit(ticket, url.into(), HistorySnapshot::default())
        .unwrap()
}

#[test]
fn traversable_steps_child_entries_same_document_and_forward_pruning() {
    let mut h = SessionHistory::new(7, 1).unwrap();
    let first = commit(
        &mut h,
        41,
        1,
        None,
        CommitKind::Navigate,
        "https://example.test/a",
    );
    h.add_child(1, 2).unwrap();
    let child = commit(
        &mut h,
        42,
        2,
        None,
        CommitKind::Navigate,
        "https://child.test/a",
    );
    let child_step = h.current_step().unwrap();
    let pushed = commit(
        &mut h,
        41,
        1,
        Some(first.document),
        CommitKind::Push,
        "https://example.test/a#one",
    );
    assert_eq!(pushed.document, first.document);
    assert_eq!(h.active_entry(2).unwrap().id, child.id);
    let push_step = h.current_step().unwrap();
    let replaced = commit(
        &mut h,
        41,
        1,
        Some(first.document),
        CommitKind::Replace,
        "https://example.test/a#two",
    );
    assert_eq!(h.current_step(), Some(push_step));
    assert_eq!(replaced.document, first.document);
    assert_eq!(h.traverse(child_step).unwrap().entries[&1].id, first.id);
    assert_eq!(h.traverse(push_step).unwrap().entries[&1].id, replaced.id);
    h.traverse(child_step).unwrap();
    let next = commit(
        &mut h,
        41,
        1,
        Some(first.document),
        CommitKind::Navigate,
        "https://other.test/new",
    );
    assert_eq!(h.steps().len(), 3);
    assert!(h.traverse(push_step).is_err());
    assert!(h.active_entry(2).is_none());
    assert_ne!(next.document, first.document);
    assert_eq!(h.document(next.document).unwrap().renderer, 41);
    let reload = commit(
        &mut h,
        41,
        1,
        Some(next.document),
        CommitKind::Reload,
        "https://other.test/new",
    );
    assert_eq!(reload.step, next.step);
    assert_ne!(reload.document, next.document);
    assert!(h
        .take_orphaned_discards()
        .iter()
        .any(|discard| discard.document == next.document));
}

#[test]
fn renderer_cannot_forge_ids_origin_or_unbounded_snapshot() {
    let mut h = SessionHistory::new(9, 1).unwrap();
    let first = commit(
        &mut h,
        5,
        1,
        None,
        CommitKind::Navigate,
        "https://example.test/",
    );
    assert_eq!(
        h.begin(6, 1, Some(first.document), CommitKind::Push),
        Err(HistoryError::Stale)
    );
    let ticket = h
        .begin(5, 1, Some(first.document), CommitKind::Push)
        .unwrap();
    let mut forged = ticket.clone();
    forged.id += 1;
    assert_eq!(
        h.commit(
            forged,
            "https://example.test/x".into(),
            HistorySnapshot::default()
        ),
        Err(HistoryError::Stale)
    );
    assert_eq!(
        h.commit(
            ticket.clone(),
            "https://evil.test/x".into(),
            HistorySnapshot::default()
        ),
        Err(HistoryError::Invalid)
    );
    let oversized = HistorySnapshot {
        history_state: Some(vec![0; 1024 * 1024 + 1]),
        ..Default::default()
    };
    assert_eq!(
        h.commit(ticket.clone(), "https://example.test/x".into(), oversized),
        Err(HistoryError::Limit)
    );
    h.commit(
        ticket,
        "https://example.test/x".into(),
        HistorySnapshot::default(),
    )
    .unwrap();
    let saved = HistorySnapshot {
        history_state: Some(b"{value:1}".to_vec()),
        navigation_state: b"nav".to_vec(),
        persisted_user_state: b"form".to_vec(),
        scroll: vec![(1, 0, 90)],
    };
    assert_eq!(
        h.save_snapshot(6, first.document, saved.clone()),
        Err(HistoryError::Stale)
    );
    h.save_snapshot(5, first.document, saved.clone()).unwrap();
    assert_eq!(h.active_entry(1).unwrap().snapshot, saved);
}

#[test]
fn discarded_document_can_reload_but_cannot_push_state() {
    let mut h = SessionHistory::new(90, 1).unwrap();
    let first = commit(
        &mut h,
        5,
        1,
        None,
        CommitKind::Navigate,
        "https://example.test/",
    );
    h.discard(first.document).unwrap();
    assert_eq!(
        h.begin(5, 1, Some(first.document), CommitKind::Push),
        Err(HistoryError::Stale)
    );
    let ticket = h
        .begin(5, 1, Some(first.document), CommitKind::Reload)
        .unwrap();
    let reloaded = h
        .commit(ticket, first.url.clone(), HistorySnapshot::default())
        .unwrap();
    assert_eq!(reloaded.step, first.step);
    assert_ne!(reloaded.document, first.document);
}

#[test]
fn top_level_navigation_can_freeze_child_before_parent() {
    let mut h = SessionHistory::new(91, 1).unwrap();
    let parent = commit(
        &mut h,
        5,
        1,
        None,
        CommitKind::Navigate,
        "https://example.test/",
    );
    h.add_child(1, 2).unwrap();
    let child = commit(
        &mut h,
        5,
        2,
        None,
        CommitKind::Navigate,
        "https://example.test/frame",
    );
    let ticket = h
        .begin(5, 1, Some(parent.document), CommitKind::Navigate)
        .unwrap();
    for document in [child.document, parent.document] {
        let epoch = h.document(document).unwrap().epoch;
        let freeze = h
            .freeze_request(document, 5, epoch, Eligibility::default(), 4096, 100)
            .unwrap();
        h.freeze_ack(
            amazing_process::protocol::HistoryAck {
                document,
                epoch: freeze.epoch,
                quiescent: true,
                events: vec![
                    HistoryLifecycleEvent::PageHidePersisted(()),
                    HistoryLifecycleEvent::VisibilityHidden(()),
                ],
            },
            101,
        )
        .unwrap();
    }
    assert_eq!(
        h.document(child.document).unwrap().status,
        DocumentStatus::Frozen
    );
    assert_eq!(
        h.document(parent.document).unwrap().status,
        DocumentStatus::Frozen
    );
    h.cancel(&ticket).unwrap();
}

#[test]
fn optional_history_state_round_trips_without_interpreting_bytes() {
    let absent = HistoryActivate {
        document: 1,
        epoch: 2,
        history_state: None,
    };
    let present = HistoryActivate {
        history_state: Some(ByteBuf(vec![0, 255, 1, 0])),
        ..absent.clone()
    };
    for value in [absent, present] {
        let encoded = amazing_ipc::codec::encode(&value).unwrap();
        assert_eq!(
            amazing_ipc::codec::decode::<HistoryActivate>(&encoded).unwrap(),
            value
        );
    }
}

#[test]
fn bfcache_demo_freeze_restore_eviction_and_crash() {
    let exe = std::path::Path::new(env!("CARGO_BIN_EXE_ipc-demo-child"));
    let mut renderer = ProcessHost::spawn(
        exe,
        Role::Renderer,
        Capabilities(BTreeMap::new()),
        RestartPolicy::Never,
    )
    .unwrap();
    renderer.connection.bind(9, BrowserEventsReceiver(Events));
    let render = RendererProxy::new(renderer.connection.clone(), Duration::from_secs(5));
    render
        .navigate(Navigate {
            url: "https://example.test/".into(),
            html_bytes: ByteBuf(b"<p>live</p>".to_vec()),
            viewport: Viewport {
                width: 32,
                height: 32,
            },
        })
        .unwrap();
    let proxy = HistoryProxy::new(renderer.connection.clone(), Duration::from_secs(5));
    let mut h = SessionHistory::new(10, 1).unwrap();
    let entry = commit(
        &mut h,
        77,
        1,
        None,
        CommitKind::Navigate,
        "https://example.test/",
    );
    let epoch = h.document(entry.document).unwrap().epoch;
    assert!(proxy
        .activate(HistoryActivate {
            document: entry.document,
            epoch,
            history_state: None,
        })
        .unwrap());
    let request = h
        .freeze_request(entry.document, 77, epoch, Eligibility::default(), 4096, 100)
        .unwrap();
    let ack = proxy.freeze(request).unwrap();
    assert_eq!(
        ack.events,
        vec![
            HistoryLifecycleEvent::PageHidePersisted(()),
            HistoryLifecycleEvent::VisibilityHidden(())
        ]
    );
    assert!(h.freeze_ack(ack, 101).unwrap().is_empty());
    assert_eq!(
        h.document(entry.document).unwrap().status,
        DocumentStatus::Frozen
    );
    assert!(render
        .input(InputEvent::Scroll(ScrollDelta { x: 0, y: 1 }))
        .is_err());
    let request = h.restore_request(entry.document, 77, 102).unwrap();
    let ack = proxy.restore(request).unwrap();
    assert_eq!(
        ack.events,
        vec![
            HistoryLifecycleEvent::VisibilityVisible(()),
            HistoryLifecycleEvent::PageShowPersisted(())
        ]
    );
    h.restore_ack(ack).unwrap();
    assert_eq!(
        h.document(entry.document).unwrap().status,
        DocumentStatus::Active
    );
    assert!(render
        .input(InputEvent::Scroll(ScrollDelta { x: 0, y: 1 }))
        .is_ok());
    assert!(h
        .freeze_request(
            entry.document,
            77,
            epoch,
            Eligibility {
                active_socket: true,
                ..Default::default()
            },
            4096,
            103
        )
        .is_err());
    let request = h
        .freeze_request(entry.document, 77, epoch, Eligibility::default(), 4096, 104)
        .unwrap();
    h.freeze_ack(proxy.freeze(request).unwrap(), 105).unwrap();
    let expired = h.evict(105 + FROZEN_TTL_MS);
    assert_eq!(expired, vec![entry.document]);
    assert!(h
        .restore_request(entry.document, 77, 105 + FROZEN_TTL_MS)
        .is_err());
    assert!(proxy.discard(h.discard(entry.document).unwrap()).unwrap());
    assert!(render
        .input(InputEvent::Scroll(ScrollDelta { x: 0, y: 1 }))
        .is_err());
    assert_eq!(h.renderer_crashed(77), vec![entry.document]);
    renderer.kill().unwrap();
}

#[test]
fn bfcache_evicts_oldest_when_count_limit_is_reached() {
    let mut h = SessionHistory::new(11, 1).unwrap();
    commit(
        &mut h,
        7,
        1,
        None,
        CommitKind::Navigate,
        "https://example.test/root",
    );
    let mut first = 0;
    for n in 0..=MAX_FROZEN {
        let child = n as u64 + 2;
        h.add_child(1, child).unwrap();
        let entry = commit(
            &mut h,
            7,
            child,
            None,
            CommitKind::Navigate,
            &format!("https://example.test/{n}"),
        );
        let epoch = h.document(entry.document).unwrap().epoch;
        let request = h
            .freeze_request(
                entry.document,
                7,
                epoch,
                Eligibility::default(),
                1024,
                n as u64,
            )
            .unwrap();
        let ack = amazing_process::protocol::HistoryAck {
            document: request.document,
            epoch: request.epoch,
            quiescent: true,
            events: vec![
                HistoryLifecycleEvent::PageHidePersisted(()),
                HistoryLifecycleEvent::VisibilityHidden(()),
            ],
        };
        let evicted = h.freeze_ack(ack, n as u64).unwrap();
        if n == 0 {
            first = entry.document;
        }
        if n == MAX_FROZEN {
            assert_eq!(evicted, vec![first]);
        }
    }
    assert_eq!(h.document(first).unwrap().status, DocumentStatus::Discarded);
}

#[test]
fn navigation_freezes_outgoing_document_before_process_swap_and_restores_after_traverse() {
    let mut h = SessionHistory::new(12, 1).unwrap();
    let first = commit(&mut h, 7, 1, None, CommitKind::Navigate, "https://a.test/");
    let first_step = h.current_step().unwrap();
    let ticket = h
        .begin(7, 1, Some(first.document), CommitKind::Navigate)
        .unwrap();
    let epoch = h.document(first.document).unwrap().epoch;
    let request = h
        .freeze_request(first.document, 7, epoch, Eligibility::default(), 1024, 100)
        .unwrap();
    h.freeze_ack(
        amazing_process::protocol::HistoryAck {
            document: request.document,
            epoch: request.epoch,
            quiescent: true,
            events: vec![
                HistoryLifecycleEvent::PageHidePersisted(()),
                HistoryLifecycleEvent::VisibilityHidden(()),
            ],
        },
        101,
    )
    .unwrap();
    let second = h
        .commit_to_renderer(
            ticket,
            8,
            "https://b.test/".into(),
            HistorySnapshot::default(),
        )
        .unwrap();
    assert_eq!(h.document(second.document).unwrap().renderer, 8);
    assert!(h.restore_request(first.document, 7, 102).is_err());
    assert_eq!(h.traverse(first_step), Err(HistoryError::Order));
    let second_epoch = h.document(second.document).unwrap().epoch;
    let request = h
        .freeze_request(
            second.document,
            8,
            second_epoch,
            Eligibility::default(),
            1024,
            102,
        )
        .unwrap();
    h.freeze_ack(
        amazing_process::protocol::HistoryAck {
            document: request.document,
            epoch: request.epoch,
            quiescent: true,
            events: vec![
                HistoryLifecycleEvent::PageHidePersisted(()),
                HistoryLifecycleEvent::VisibilityHidden(()),
            ],
        },
        103,
    )
    .unwrap();
    h.traverse(first_step).unwrap();
    assert!(h.restore_request(first.document, 8, 102).is_err());
    assert_eq!(
        h.restore_request(first.document, 7, 102).unwrap().document,
        first.document
    );
}

#[test]
fn freeze_ack_timeout_discards_document() {
    let mut h = SessionHistory::new(13, 1).unwrap();
    let entry = commit(&mut h, 7, 1, None, CommitKind::Navigate, "https://a.test/");
    let epoch = h.document(entry.document).unwrap().epoch;
    let request = h
        .freeze_request(entry.document, 7, epoch, Eligibility::default(), 1024, 100)
        .unwrap();
    assert_eq!(
        h.freeze_ack(
            amazing_process::protocol::HistoryAck {
                document: request.document,
                epoch: request.epoch,
                quiescent: true,
                events: vec![
                    HistoryLifecycleEvent::PageHidePersisted(()),
                    HistoryLifecycleEvent::VisibilityHidden(())
                ]
            },
            request.deadline_ms + 1
        ),
        Err(HistoryError::Stale)
    );
    assert_eq!(
        h.document(entry.document).unwrap().status,
        DocumentStatus::Discarded
    );
}

#[test]
fn bfcache_evicts_for_memory_pressure() {
    let mut h = SessionHistory::new(14, 1).unwrap();
    commit(
        &mut h,
        7,
        1,
        None,
        CommitKind::Navigate,
        "https://example.test/root",
    );
    let mut first = 0;
    for n in 0..2 {
        let child = n + 2;
        h.add_child(1, child).unwrap();
        let entry = commit(
            &mut h,
            7,
            child,
            None,
            CommitKind::Navigate,
            &format!("https://example.test/{n}"),
        );
        if n == 0 {
            first = entry.document;
        }
        let epoch = h.document(entry.document).unwrap().epoch;
        let request = h
            .freeze_request(
                entry.document,
                7,
                epoch,
                Eligibility::default(),
                80 * 1024 * 1024,
                n,
            )
            .unwrap();
        let evicted = h
            .freeze_ack(
                amazing_process::protocol::HistoryAck {
                    document: request.document,
                    epoch: request.epoch,
                    quiescent: true,
                    events: vec![
                        HistoryLifecycleEvent::PageHidePersisted(()),
                        HistoryLifecycleEvent::VisibilityHidden(()),
                    ],
                },
                n,
            )
            .unwrap();
        if n == 1 {
            assert_eq!(evicted, vec![first]);
        }
    }
}

// HTML 7.4.1.3's /a, /b, then history.back() racing with /b#foo:
// the synchronous fragment entry appears before the queued traversal applies.
#[test]
fn html_traversal_queue_preserves_synchronous_fragment_race() {
    let mut h = SessionHistory::new(15, 1).unwrap();
    let a = commit(
        &mut h,
        7,
        1,
        None,
        CommitKind::Navigate,
        "https://example.test/a",
    );
    let ticket = h
        .begin(7, 1, Some(a.document), CommitKind::Navigate)
        .unwrap();
    let a_epoch = h.document(a.document).unwrap().epoch;
    let freeze = h
        .freeze_request(a.document, 7, a_epoch, Eligibility::default(), 1024, 0)
        .unwrap();
    h.freeze_ack(
        amazing_process::protocol::HistoryAck {
            document: freeze.document,
            epoch: freeze.epoch,
            quiescent: true,
            events: vec![
                HistoryLifecycleEvent::PageHidePersisted(()),
                HistoryLifecycleEvent::VisibilityHidden(()),
            ],
        },
        1,
    )
    .unwrap();
    let b = h
        .commit(
            ticket,
            "https://example.test/b".into(),
            HistorySnapshot::default(),
        )
        .unwrap();
    h.queue_traversal(a.step).unwrap();
    let fragment = commit(
        &mut h,
        7,
        1,
        Some(b.document),
        CommitKind::Push,
        "https://example.test/b#foo",
    );
    assert_eq!(h.current_step(), Some(fragment.step));
    assert_eq!(h.steps().len(), 3);
    let b_epoch = h.document(b.document).unwrap().epoch;
    let freeze = h
        .freeze_request(b.document, 7, b_epoch, Eligibility::default(), 1024, 2)
        .unwrap();
    h.freeze_ack(
        amazing_process::protocol::HistoryAck {
            document: freeze.document,
            epoch: freeze.epoch,
            quiescent: true,
            events: vec![
                HistoryLifecycleEvent::PageHidePersisted(()),
                HistoryLifecycleEvent::VisibilityHidden(()),
            ],
        },
        3,
    )
    .unwrap();
    assert_eq!(h.apply_queued_traversal().unwrap().entries[&1].id, a.id);
    assert_eq!(h.steps()[2].entries[&1], fragment.id);
}
