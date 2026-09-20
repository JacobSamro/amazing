use super::*;
const G: GlobalId = GlobalId(1);
fn scheduler() -> EventLoop {
    let mut e = EventLoop::new();
    e.register_global(G).unwrap();
    e
}
fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}
fn next(e: &mut EventLoop) -> Step {
    e.step().unwrap().unwrap()
}
fn done(e: &mut EventLoop) -> Action {
    let s = next(e);
    e.complete(s.id).unwrap();
    s.action
}
fn cleanup(e: &mut EventLoop) {
    assert_eq!(done(e), Action::CheckpointCleanup);
}
fn callback(a: Action) -> u64 {
    match a {
        Action::Task { callback, .. }
        | Action::Microtask { callback, .. }
        | Action::AnimationFrame { callback, .. } => callback.0,
        _ => panic!("not callback: {a:?}"),
    }
}

#[test]
fn source_queues_are_fair_and_fifo_within_each_source() {
    let mut e = scheduler();
    for (source, id) in [
        (TaskSource::Networking, 1),
        (TaskSource::Networking, 2),
        (TaskSource::UserInteraction, 3),
    ] {
        e.queue_task(source, Some(G), CallbackId(id)).unwrap();
    }
    assert_eq!(callback(done(&mut e)), 3);
    cleanup(&mut e);
    assert_eq!(callback(done(&mut e)), 1);
    cleanup(&mut e);
    assert_eq!(callback(done(&mut e)), 2);
    cleanup(&mut e);
    assert!(e.step().unwrap().is_none());
}
#[test]
fn tasks_skip_inactive_documents_but_microtasks_do_not() {
    let mut e = scheduler();
    let b = GlobalId(2);
    e.register_global(b).unwrap();
    e.set_active(G, false).unwrap();
    e.queue_task(TaskSource::Networking, Some(G), CallbackId(1))
        .unwrap();
    e.queue_task(TaskSource::Networking, Some(b), CallbackId(2))
        .unwrap();
    e.queue_microtask(G, CallbackId(3)).unwrap();
    assert_eq!(callback(done(&mut e)), 3);
    cleanup(&mut e);
    assert_eq!(callback(done(&mut e)), 2);
    cleanup(&mut e);
    assert!(e.step().unwrap().is_none());
    e.set_active(G, true).unwrap();
    assert_eq!(callback(done(&mut e)), 1);
    cleanup(&mut e);
}
#[test]
fn microtasks_drain_newly_queued_jobs_before_next_task() {
    let mut e = scheduler();
    e.queue_task(TaskSource::DomManipulation, Some(G), CallbackId(1))
        .unwrap();
    e.queue_task(TaskSource::DomManipulation, Some(G), CallbackId(2))
        .unwrap();
    let task = next(&mut e);
    e.queue_microtask(G, CallbackId(3)).unwrap();
    e.complete(task.id).unwrap();
    let micro = next(&mut e);
    assert_eq!(callback(micro.action), 3);
    e.queue_microtask(G, CallbackId(4)).unwrap();
    e.complete(micro.id).unwrap();
    assert_eq!(callback(done(&mut e)), 4);
    cleanup(&mut e);
    assert_eq!(callback(done(&mut e)), 2);
}
#[test]
fn checkpoint_reentrancy_is_suppressed_through_cleanup() {
    let mut e = scheduler();
    assert!(e.request_microtask_checkpoint().unwrap());
    assert!(!e.request_microtask_checkpoint().unwrap());
    let s = next(&mut e);
    assert_eq!(s.action, Action::CheckpointCleanup);
    assert!(e.performing_microtask_checkpoint());
    assert!(!e.request_microtask_checkpoint().unwrap());
    e.complete(s.id).unwrap();
    assert!(!e.performing_microtask_checkpoint());
    assert!(e.step().unwrap().is_none());
}
#[test]
fn cleanup_enqueued_jobs_wait_for_a_new_checkpoint() {
    let mut e = scheduler();
    e.request_microtask_checkpoint().unwrap();
    let s = next(&mut e);
    e.queue_microtask(G, CallbackId(1)).unwrap();
    e.complete(s.id).unwrap();
    assert_eq!(callback(done(&mut e)), 1);
    cleanup(&mut e);
}
#[test]
fn completion_tokens_guard_reentrancy_and_duplicate_completion() {
    let mut e = scheduler();
    e.queue_task(TaskSource::Other(3), None, CallbackId(1))
        .unwrap();
    let s = next(&mut e);
    assert_eq!(e.step(), Err(LoopError::Busy));
    assert_eq!(e.request_microtask_checkpoint(), Err(LoopError::Busy));
    assert_eq!(e.complete(StepId(999)), Err(LoopError::WrongStep));
    e.complete(s.id).unwrap();
    assert_eq!(e.complete(s.id), Err(LoopError::WrongStep));
    cleanup(&mut e);
}
#[test]
fn thrown_host_callback_still_allows_later_work() {
    let mut e = scheduler();
    e.queue_microtask(G, CallbackId(1)).unwrap();
    e.queue_microtask(G, CallbackId(2)).unwrap();
    // Host catches/reports throw for first callback, then completes its token.
    assert_eq!(callback(done(&mut e)), 1);
    assert_eq!(callback(done(&mut e)), 2);
    cleanup(&mut e);
    assert!(e.step().unwrap().is_none());
}
#[test]
fn timer_deadlines_and_expiry_order_use_driver_time() {
    let mut e = scheduler();
    e.set_timer(G, CallbackId(1), 20, false).unwrap();
    e.set_timer(G, CallbackId(2), 10, false).unwrap();
    e.set_timer(G, CallbackId(3), 10, false).unwrap();
    assert_eq!(e.next_timer_deadline(), Some(ms(10)));
    e.advance_to(ms(9)).unwrap();
    assert!(e.step().unwrap().is_none());
    e.advance_to(ms(30)).unwrap();
    for id in [2, 3, 1] {
        assert_eq!(callback(done(&mut e)), id);
        cleanup(&mut e);
    }
    assert_eq!(e.next_timer_deadline(), None);
    assert_eq!(e.advance_to(ms(1)), Err(LoopError::ClockWentBackwards));
    assert_eq!(e.now(), ms(30));
}
#[test]
fn clear_timer_invalidates_already_queued_task_without_reusing_id() {
    let mut e = scheduler();
    let old = e.set_timer(G, CallbackId(1), 0, false).unwrap();
    e.clear_timer(old);
    let new = e.set_timer(G, CallbackId(2), 0, false).unwrap();
    assert_ne!(old, new);
    assert_eq!(callback(done(&mut e)), 2);
    cleanup(&mut e);
    assert!(!e.timer_pending(new));
    assert!(e.step().unwrap().is_none());
}
#[test]
fn timer_active_time_excludes_document_inactivity() {
    let mut e = scheduler();
    e.set_timer(G, CallbackId(1), 10, false).unwrap();
    e.advance_to(ms(4)).unwrap();
    e.set_active(G, false).unwrap();
    e.advance_to(ms(100)).unwrap();
    assert_eq!(e.next_timer_deadline(), None);
    assert!(e.step().unwrap().is_none());
    e.set_active(G, true).unwrap();
    assert_eq!(e.next_timer_deadline(), Some(ms(106)));
    e.advance_to(ms(105)).unwrap();
    assert!(e.step().unwrap().is_none());
    e.advance_to(ms(106)).unwrap();
    assert_eq!(callback(done(&mut e)), 1);
}
#[test]
fn interval_rearms_after_callback_checkpoint_and_can_be_canceled_there() {
    let mut e = scheduler();
    let timer = e.set_timer(G, CallbackId(1), 5, true).unwrap();
    e.advance_to(ms(5)).unwrap();
    let s = next(&mut e);
    e.advance_to(ms(20)).unwrap();
    e.queue_microtask(G, CallbackId(2)).unwrap();
    e.complete(s.id).unwrap();
    assert_eq!(callback(done(&mut e)), 2);
    e.advance_to(ms(30)).unwrap();
    cleanup(&mut e);
    assert_eq!(e.next_timer_deadline(), Some(ms(35)));
    e.advance_to(ms(35)).unwrap();
    let s = next(&mut e);
    e.queue_microtask(G, CallbackId(3)).unwrap();
    e.complete(s.id).unwrap();
    let micro = next(&mut e);
    e.clear_timer(timer);
    e.complete(micro.id).unwrap();
    cleanup(&mut e);
    assert!(!e.timer_pending(timer));
    e.advance_to(ms(100)).unwrap();
    assert!(e.step().unwrap().is_none());
}
#[test]
fn nested_timers_and_intervals_clamp_after_six_levels() {
    let mut e = scheduler();
    e.set_timer(G, CallbackId(1), -4, false).unwrap();
    for level in 1..=6 {
        let s = next(&mut e);
        assert_eq!(callback(s.action), 1);
        e.set_timer(G, CallbackId(1), 0, false).unwrap();
        e.complete(s.id).unwrap();
        cleanup(&mut e);
        if level == 6 {
            assert_eq!(e.next_timer_deadline(), Some(ms(4)));
        }
    }
    assert!(e.step().unwrap().is_none());
    e.advance_to(ms(4)).unwrap();
    assert_eq!(callback(done(&mut e)), 1);
    cleanup(&mut e);
    let mut e = scheduler();
    e.set_timer(G, CallbackId(2), 0, true).unwrap();
    for _ in 0..6 {
        assert_eq!(callback(done(&mut e)), 2);
        cleanup(&mut e);
    }
    assert_eq!(e.next_timer_deadline(), Some(ms(4)));
}
#[test]
fn microtask_created_timer_does_not_inherit_timer_nesting() {
    let mut e = scheduler();
    e.timer_nesting = 9;
    e.queue_microtask(G, CallbackId(1)).unwrap();
    let micro = next(&mut e);
    e.set_timer(G, CallbackId(2), 0, false).unwrap();
    e.complete(micro.id).unwrap();
    cleanup(&mut e);
    assert_eq!(callback(done(&mut e)), 2);
}
#[test]
fn animation_frames_are_not_timers_and_use_supplied_frame_timestamp() {
    let mut e = scheduler();
    e.request_animation_frame(G, CallbackId(1)).unwrap();
    e.advance_to(ms(1000)).unwrap();
    assert!(e.step().unwrap().is_none());
    e.rendering_opportunity(G, ms(17)).unwrap();
    assert_eq!(
        done(&mut e),
        Action::Render {
            global: G,
            timestamp: ms(17),
            phase: RenderPhase::BeforeAnimationFrames
        }
    );
    cleanup(&mut e);
    let s = next(&mut e);
    assert!(
        matches!(s.action,Action::AnimationFrame{callback:CallbackId(1),timestamp,..} if timestamp==ms(17))
    );
    e.complete(s.id).unwrap();
    cleanup(&mut e);
    assert_eq!(
        done(&mut e),
        Action::Render {
            global: G,
            timestamp: ms(17),
            phase: RenderPhase::AfterAnimationFrames
        }
    );
    cleanup(&mut e);
    assert!(e.step().unwrap().is_none());
}
#[test]
fn animation_snapshot_honors_cancellation_and_defers_new_callbacks() {
    let mut e = scheduler();
    e.request_animation_frame(G, CallbackId(1)).unwrap();
    let second = e.request_animation_frame(G, CallbackId(2)).unwrap();
    e.rendering_opportunity(G, ms(16)).unwrap();
    done(&mut e);
    cleanup(&mut e);
    let first = next(&mut e);
    e.queue_microtask(G, CallbackId(3)).unwrap();
    e.complete(first.id).unwrap();
    let micro = next(&mut e);
    e.cancel_animation_frame(G, second).unwrap();
    e.request_animation_frame(G, CallbackId(4)).unwrap();
    e.complete(micro.id).unwrap();
    cleanup(&mut e);
    assert!(matches!(
        done(&mut e),
        Action::Render {
            phase: RenderPhase::AfterAnimationFrames,
            ..
        }
    ));
    cleanup(&mut e);
    assert!(e.step().unwrap().is_none());
    e.rendering_opportunity(G, ms(32)).unwrap();
    done(&mut e);
    cleanup(&mut e);
    assert_eq!(callback(done(&mut e)), 4);
}
#[test]
fn frame_requested_by_pre_render_microtask_is_in_current_snapshot() {
    let mut e = scheduler();
    e.rendering_opportunity(G, ms(16)).unwrap();
    let render = next(&mut e);
    e.queue_microtask(G, CallbackId(1)).unwrap();
    e.complete(render.id).unwrap();
    let micro = next(&mut e);
    e.request_animation_frame(G, CallbackId(2)).unwrap();
    e.complete(micro.id).unwrap();
    cleanup(&mut e);
    assert_eq!(callback(done(&mut e)), 2);
}
#[test]
fn hidden_rendering_keeps_callbacks_and_opportunities_coalesce() {
    let mut e = scheduler();
    e.request_animation_frame(G, CallbackId(1)).unwrap();
    e.set_renderable(G, false).unwrap();
    e.rendering_opportunity(G, ms(10)).unwrap();
    assert!(e.step().unwrap().is_none());
    e.set_renderable(G, true).unwrap();
    e.rendering_opportunity(G, ms(20)).unwrap();
    e.rendering_opportunity(G, ms(30)).unwrap();
    assert!(matches!(done(&mut e),Action::Render{timestamp,..} if timestamp==ms(30)));
    cleanup(&mut e);
    assert_eq!(callback(done(&mut e)), 1);
    cleanup(&mut e);
    done(&mut e);
    cleanup(&mut e);
    assert!(e.step().unwrap().is_none());
}
#[test]
fn visibility_is_rechecked_when_render_task_runs() {
    let mut e = scheduler();
    e.request_animation_frame(G, CallbackId(1)).unwrap();
    e.rendering_opportunity(G, ms(10)).unwrap();
    e.set_renderable(G, false).unwrap();
    assert!(e.step().unwrap().is_none());
    e.set_renderable(G, true).unwrap();
    e.rendering_opportunity(G, ms(20)).unwrap();
    done(&mut e);
    cleanup(&mut e);
    assert_eq!(callback(done(&mut e)), 1);
}
#[test]
fn invalid_global_calls_fail_and_keep_existing_state() {
    let mut e = scheduler();
    let bad = GlobalId(99);
    assert_eq!(e.register_global(G), Err(LoopError::DuplicateGlobal));
    assert_eq!(
        e.queue_task(TaskSource::Timer, Some(bad), CallbackId(1)),
        Err(LoopError::UnknownGlobal)
    );
    assert_eq!(
        e.set_timer(bad, CallbackId(1), 0, false),
        Err(LoopError::UnknownGlobal)
    );
    assert_eq!(
        e.request_animation_frame(bad, CallbackId(1)),
        Err(LoopError::UnknownGlobal)
    );
    assert!(e.step().unwrap().is_none());
}

#[test]
fn host_callback_checkpoint_resumes_same_task_before_later_tasks() {
    let mut e = scheduler();
    e.queue_task(TaskSource::UserInteraction, Some(G), CallbackId(1))
        .unwrap();
    e.queue_task(TaskSource::Networking, Some(G), CallbackId(2))
        .unwrap();
    let event = next(&mut e);
    e.queue_microtask(G, CallbackId(3)).unwrap();
    assert!(e.checkpoint_after_callback(event.id).unwrap());
    assert_eq!(e.complete(event.id), Err(LoopError::WrongStep));
    let job = next(&mut e);
    assert_eq!(callback(job.action), 3);
    assert!(!e.checkpoint_after_callback(job.id).unwrap());
    e.complete(job.id).unwrap();
    cleanup(&mut e);
    assert_eq!(e.pending_step(), Some(event.id));
    assert_eq!(e.step(), Err(LoopError::Busy));
    // A second listener can itself queue a job before the event task completes.
    e.queue_microtask(G, CallbackId(4)).unwrap();
    e.checkpoint_after_callback(event.id).unwrap();
    assert_eq!(callback(done(&mut e)), 4);
    cleanup(&mut e);
    e.complete(event.id).unwrap();
    cleanup(&mut e);
    assert_eq!(callback(done(&mut e)), 2);
}

#[test]
fn nested_callback_checkpoint_restores_render_frame_continuation() {
    let mut e = scheduler();
    e.request_animation_frame(G, CallbackId(1)).unwrap();
    e.request_animation_frame(G, CallbackId(2)).unwrap();
    e.rendering_opportunity(G, ms(16)).unwrap();
    done(&mut e);
    cleanup(&mut e);
    let first = next(&mut e);
    e.checkpoint_after_callback(first.id).unwrap();
    cleanup(&mut e);
    assert_eq!(e.pending_step(), Some(first.id));
    e.complete(first.id).unwrap();
    cleanup(&mut e);
    assert_eq!(callback(done(&mut e)), 2);
}
