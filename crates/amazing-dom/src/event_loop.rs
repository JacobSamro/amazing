//! Driver-stepped HTML event loop. No clock, sleeping, JS values or callbacks
//! borrowed from a host. Drivers retain callback payloads behind opaque tokens.
//! Complete every yielded step even when its callback throws (report the error
//! separately). Time and rendering eligibility are supplied by the embedding.
use std::collections::{BTreeMap, VecDeque};
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct GlobalId(pub u64);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CallbackId(pub u64);
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct TimerId(u64);
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct AnimationFrameId(u64);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StepId(u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum TaskSource {
    DomManipulation,
    UserInteraction,
    Networking,
    HistoryTraversal,
    PostedMessage,
    Timer,
    Rendering,
    Other(u64),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoopError {
    Busy,
    UnknownGlobal,
    DuplicateGlobal,
    ClockWentBackwards,
    WrongStep,
    Exhausted,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderPhase {
    BeforeAnimationFrames,
    AfterAnimationFrames,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Task {
        global: Option<GlobalId>,
        source: TaskSource,
        callback: CallbackId,
        timer: Option<TimerId>,
    },
    Microtask {
        global: GlobalId,
        callback: CallbackId,
    },
    /// Host hooks: rejected promises, transaction cleanup, ClearKeptObjects.
    /// The checkpoint guard remains set until this action is completed.
    CheckpointCleanup,
    Render {
        global: GlobalId,
        timestamp: Duration,
        phase: RenderPhase,
    },
    AnimationFrame {
        global: GlobalId,
        callback: CallbackId,
        handle: AnimationFrameId,
        timestamp: Duration,
    },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    pub id: StepId,
    pub action: Action,
}

struct Global {
    active: bool,
    renderable: bool,
    active_time: Duration,
    frames: BTreeMap<AnimationFrameId, CallbackId>,
    render_queued: bool,
    frame_timestamp: Duration,
}
struct Timer {
    global: GlobalId,
    callback: CallbackId,
    delay: Duration,
    due: Duration,
    repeat: bool,
    nesting: u32,
    queued: bool,
}
enum Body {
    Callback(CallbackId),
    Timer(TimerId),
    Render,
}
struct Task {
    global: Option<GlobalId>,
    body: Body,
}
struct Frame {
    global: GlobalId,
    timestamp: Duration,
    handles: VecDeque<AnimationFrameId>,
}
enum Resume {
    RestoreCallback,
    Idle,
    FinishTask(Option<TimerId>),
    StartFrames(GlobalId, Duration),
    Frames(Frame),
}
enum Phase {
    Idle,
    Checkpoint(Resume),
    Frames(Frame),
    AfterRender,
}
enum Completion {
    Task(Option<TimerId>),
    Microtask,
    Cleanup,
    BeforeRender(GlobalId, Duration),
    Frame,
    AfterRender,
}

pub struct EventLoop {
    now: Duration,
    sequence: u64,
    globals: BTreeMap<GlobalId, Global>,
    queues: BTreeMap<TaskSource, VecDeque<Task>>,
    last_source: Option<TaskSource>,
    microtasks: VecDeque<(GlobalId, CallbackId)>,
    timers: BTreeMap<TimerId, Timer>,
    phase: Phase,
    in_flight: Option<(StepId, Completion)>,
    suspended: Option<(StepId, Completion, Phase, u32)>,
    timer_nesting: u32,
    checkpoint: bool,
    render_global: Option<(GlobalId, Duration)>,
}
impl Default for EventLoop {
    fn default() -> Self {
        Self::new()
    }
}
impl EventLoop {
    pub fn new() -> Self {
        Self {
            now: Duration::ZERO,
            sequence: 0,
            globals: BTreeMap::new(),
            queues: BTreeMap::new(),
            last_source: None,
            microtasks: VecDeque::new(),
            timers: BTreeMap::new(),
            phase: Phase::Idle,
            in_flight: None,
            suspended: None,
            timer_nesting: 0,
            checkpoint: false,
            render_global: None,
        }
    }
    fn id(&mut self) -> Result<u64, LoopError> {
        self.sequence = self.sequence.checked_add(1).ok_or(LoopError::Exhausted)?;
        Ok(self.sequence)
    }
    pub fn now(&self) -> Duration {
        self.now
    }
    pub fn register_global(&mut self, id: GlobalId) -> Result<(), LoopError> {
        if self.globals.contains_key(&id) {
            return Err(LoopError::DuplicateGlobal);
        }
        self.globals.insert(
            id,
            Global {
                active: true,
                renderable: true,
                active_time: Duration::ZERO,
                frames: BTreeMap::new(),
                render_queued: false,
                frame_timestamp: Duration::ZERO,
            },
        );
        Ok(())
    }
    /// Call advance_to at the state transition's timestamp before changing this.
    /// Inactive documents/suspended workers do not accrue timer-active time.
    pub fn set_active(&mut self, id: GlobalId, active: bool) -> Result<(), LoopError> {
        self.globals
            .get_mut(&id)
            .ok_or(LoopError::UnknownGlobal)?
            .active = active;
        Ok(())
    }
    /// Aggregate visibility/render-blocking/UA eligibility; independent of activity.
    pub fn set_renderable(&mut self, id: GlobalId, renderable: bool) -> Result<(), LoopError> {
        self.globals
            .get_mut(&id)
            .ok_or(LoopError::UnknownGlobal)?
            .renderable = renderable;
        Ok(())
    }
    /// Driver clock, monotonic and relative to one event-loop origin. May advance
    /// while a callback is in flight; intervals rearm from callback completion.
    pub fn advance_to(&mut self, now: Duration) -> Result<(), LoopError> {
        let elapsed = now
            .checked_sub(self.now)
            .ok_or(LoopError::ClockWentBackwards)?;
        for global in self.globals.values_mut() {
            if global.active {
                global.active_time = global.active_time.saturating_add(elapsed);
            }
        }
        self.now = now;
        self.promote_timers();
        Ok(())
    }
    fn check_global(&self, global: GlobalId) -> Result<(), LoopError> {
        if self.globals.contains_key(&global) {
            Ok(())
        } else {
            Err(LoopError::UnknownGlobal)
        }
    }
    pub fn queue_task(
        &mut self,
        source: TaskSource,
        global: Option<GlobalId>,
        callback: CallbackId,
    ) -> Result<(), LoopError> {
        if let Some(global) = global {
            self.check_global(global)?;
        }
        self.push_task(source, global, Body::Callback(callback));
        Ok(())
    }
    fn push_task(&mut self, source: TaskSource, global: Option<GlobalId>, body: Body) {
        self.queues
            .entry(source)
            .or_default()
            .push_back(Task { global, body });
    }
    pub fn queue_microtask(
        &mut self,
        global: GlobalId,
        callback: CallbackId,
    ) -> Result<(), LoopError> {
        self.check_global(global)?;
        self.microtasks.push_back((global, callback));
        Ok(())
    }
    pub fn performing_microtask_checkpoint(&self) -> bool {
        self.checkpoint
    }
    /// Host script-cleanup checkpoint outside an event-loop callback. Reentrant
    /// checkpoints are no-ops. During a yielded callback, complete it instead.
    pub fn request_microtask_checkpoint(&mut self) -> Result<bool, LoopError> {
        if self.checkpoint {
            return Ok(false);
        }
        if self.in_flight.is_some() || !matches!(self.phase, Phase::Idle) {
            return Err(LoopError::Busy);
        }
        self.start_checkpoint(Resume::Idle);
        Ok(true)
    }
    /// Checkpoint at a host callback boundary within an unfinished task (e.g.
    /// between event listeners or rendering hooks), only when the JS stack is
    /// empty. The host retains its continuation for `id`. Drain yielded actions
    /// until `pending_step() == Some(id)`, then resume that continuation and
    /// eventually complete `id`. No other task runs during this suspension.
    /// Recursive requests while performing a checkpoint are no-ops.
    pub fn checkpoint_after_callback(&mut self, id: StepId) -> Result<bool, LoopError> {
        if self.checkpoint {
            return Ok(false);
        }
        if self.in_flight.as_ref().map(|(current, _)| *current) != Some(id) {
            return Err(LoopError::WrongStep);
        }
        let (id, completion) = self.in_flight.take().unwrap();
        let phase = std::mem::replace(&mut self.phase, Phase::Idle);
        self.suspended = Some((id, completion, phase, self.timer_nesting));
        self.start_checkpoint(Resume::RestoreCallback);
        Ok(true)
    }
    /// In-flight action, including an enclosing callback restored after a nested
    /// cleanup checkpoint. The host owns the corresponding callback continuation.
    pub fn pending_step(&self) -> Option<StepId> {
        self.in_flight.as_ref().map(|(id, _)| *id)
    }
    fn start_checkpoint(&mut self, resume: Resume) {
        self.checkpoint = true;
        self.timer_nesting = 0;
        self.phase = Phase::Checkpoint(resume);
    }
    /// WebIDL numeric conversion belongs to bindings; negative delays become zero.
    /// Timer nesting counts both interval repetitions and nested timeouts.
    pub fn set_timer(
        &mut self,
        global: GlobalId,
        callback: CallbackId,
        delay_ms: i32,
        repeat: bool,
    ) -> Result<TimerId, LoopError> {
        self.check_global(global)?;
        let id = TimerId(self.id()?);
        let mut delay = Duration::from_millis(delay_ms.max(0) as u64);
        if self.timer_nesting > 5 {
            delay = delay.max(Duration::from_millis(4));
        }
        let due = self.globals[&global].active_time.saturating_add(delay);
        self.timers.insert(
            id,
            Timer {
                global,
                callback,
                delay,
                due,
                repeat,
                nesting: self.timer_nesting.saturating_add(1),
                queued: false,
            },
        );
        self.promote_timers();
        Ok(id)
    }
    /// IDs are never reused. Removing a queued timer also invalidates its task.
    pub fn clear_timer(&mut self, id: TimerId) {
        self.timers.remove(&id);
    }
    pub fn timer_pending(&self, id: TimerId) -> bool {
        self.timers.contains_key(&id)
    }
    fn promote_timers(&mut self) {
        // Oldest expiry first, ties by initialization order. Paused globals do
        // not promote timers. The elapsed-active-time ordering rule is retained.
        let mut ready: Vec<_> = self
            .timers
            .iter()
            .filter_map(|(&id, timer)| {
                let global = &self.globals[&timer.global];
                (global.active && !timer.queued && timer.due <= global.active_time).then_some((
                    global.active_time.saturating_sub(timer.due),
                    id,
                    timer.global,
                ))
            })
            .collect();
        ready.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        for (_, id, global) in ready {
            self.timers.get_mut(&id).unwrap().queued = true;
            self.push_task(TaskSource::Timer, Some(global), Body::Timer(id));
        }
    }
    /// Earliest possible timer deadline if current activity states persist.
    /// Runnable tasks, microtasks and pending frames must be considered separately.
    pub fn next_timer_deadline(&self) -> Option<Duration> {
        self.timers
            .values()
            .filter_map(|timer| {
                let global = &self.globals[&timer.global];
                (global.active && !timer.queued).then(|| {
                    self.now
                        .saturating_add(timer.due.saturating_sub(global.active_time))
                })
            })
            .min()
    }
    pub fn request_animation_frame(
        &mut self,
        global: GlobalId,
        callback: CallbackId,
    ) -> Result<AnimationFrameId, LoopError> {
        self.check_global(global)?;
        let id = AnimationFrameId(self.id()?);
        self.globals
            .get_mut(&global)
            .unwrap()
            .frames
            .insert(id, callback);
        Ok(id)
    }
    pub fn cancel_animation_frame(
        &mut self,
        global: GlobalId,
        id: AnimationFrameId,
    ) -> Result<(), LoopError> {
        self.globals
            .get_mut(&global)
            .ok_or(LoopError::UnknownGlobal)?
            .frames
            .remove(&id);
        Ok(())
    }
    /// Called by the driver at a rendering opportunity, in document/container order.
    /// timestamp is already relative to this global's time origin. Repeated
    /// opportunities coalesce until the queued rendering task starts.
    pub fn rendering_opportunity(
        &mut self,
        global: GlobalId,
        timestamp: Duration,
    ) -> Result<(), LoopError> {
        let state = self
            .globals
            .get_mut(&global)
            .ok_or(LoopError::UnknownGlobal)?;
        if !state.active || !state.renderable {
            return Ok(());
        }
        state.frame_timestamp = timestamp;
        if !state.render_queued {
            state.render_queued = true;
            self.push_task(TaskSource::Rendering, Some(global), Body::Render);
        }
        Ok(())
    }
    fn next_task(&mut self) -> Option<(TaskSource, Task)> {
        let sources: Vec<_> = self.queues.keys().copied().collect();
        let start = self
            .last_source
            .and_then(|last| sources.iter().position(|s| *s > last))
            .unwrap_or(0);
        for offset in 0..sources.len() {
            let source = sources[(start + offset) % sources.len()];
            let queue = self.queues.get_mut(&source).unwrap();
            if let Some(index) = queue
                .iter()
                .position(|task| task.global.is_none_or(|id| self.globals[&id].active))
            {
                self.last_source = Some(source);
                return Some((source, queue.remove(index).unwrap()));
            }
        }
        None
    }
    fn yield_step(
        &mut self,
        action: Action,
        completion: Completion,
    ) -> Result<Option<Step>, LoopError> {
        let id = StepId(self.id()?);
        self.in_flight = Some((id, completion));
        Ok(Some(Step { id, action }))
    }
    /// Returns at most one owned action. Complete it before requesting the next.
    /// Source queues use deterministic round-robin selection; within a source,
    /// select the oldest runnable task, skipping inactive documents.
    pub fn step(&mut self) -> Result<Option<Step>, LoopError> {
        if self.in_flight.is_some() {
            return Err(LoopError::Busy);
        }
        self.promote_timers();
        loop {
            match std::mem::replace(&mut self.phase, Phase::Idle) {
                Phase::Checkpoint(resume) => {
                    self.phase = Phase::Checkpoint(resume);
                    if let Some((global, callback)) = self.microtasks.pop_front() {
                        return self.yield_step(
                            Action::Microtask { global, callback },
                            Completion::Microtask,
                        );
                    }
                    return self.yield_step(Action::CheckpointCleanup, Completion::Cleanup);
                }
                Phase::Frames(mut frame) => {
                    while let Some(handle) = frame.handles.pop_front() {
                        if let Some(callback) = self
                            .globals
                            .get_mut(&frame.global)
                            .unwrap()
                            .frames
                            .remove(&handle)
                        {
                            let action = Action::AnimationFrame {
                                global: frame.global,
                                callback,
                                handle,
                                timestamp: frame.timestamp,
                            };
                            self.phase = Phase::Frames(frame);
                            return self.yield_step(action, Completion::Frame);
                        }
                    }
                    self.phase = Phase::AfterRender;
                }
                Phase::AfterRender => {
                    let (global, timestamp) = self.render_global.take().unwrap();
                    return self.yield_step(
                        Action::Render {
                            global,
                            timestamp,
                            phase: RenderPhase::AfterAnimationFrames,
                        },
                        Completion::AfterRender,
                    );
                }
                Phase::Idle => {
                    // A queued microtask outside a task represents script cleanup.
                    if !self.microtasks.is_empty() {
                        self.start_checkpoint(Resume::Idle);
                        continue;
                    }
                    let Some((source, task)) = self.next_task() else {
                        return Ok(None);
                    };
                    match task.body {
                        Body::Callback(callback) => {
                            return self.yield_step(
                                Action::Task {
                                    global: task.global,
                                    source,
                                    callback,
                                    timer: None,
                                },
                                Completion::Task(None),
                            );
                        }
                        Body::Timer(id) => {
                            let Some(timer) = self.timers.get(&id) else {
                                continue;
                            };
                            let callback = timer.callback;
                            self.timer_nesting = timer.nesting;
                            return self.yield_step(
                                Action::Task {
                                    global: task.global,
                                    source,
                                    callback,
                                    timer: Some(id),
                                },
                                Completion::Task(Some(id)),
                            );
                        }
                        Body::Render => {
                            let global = task.global.unwrap();
                            let state = self.globals.get_mut(&global).unwrap();
                            state.render_queued = false;
                            if !state.renderable {
                                continue;
                            }
                            let timestamp = state.frame_timestamp;
                            self.render_global = Some((global, timestamp));
                            return self.yield_step(
                                Action::Render {
                                    global,
                                    timestamp,
                                    phase: RenderPhase::BeforeAnimationFrames,
                                },
                                Completion::BeforeRender(global, timestamp),
                            );
                        }
                    }
                }
            }
        }
    }
    fn finish_timer(&mut self, timer: Option<TimerId>) {
        if let Some(id) = timer {
            if let Some(mut timer) = self.timers.remove(&id) {
                if timer.repeat {
                    if timer.nesting > 5 {
                        timer.delay = timer.delay.max(Duration::from_millis(4));
                    }
                    timer.nesting = timer.nesting.saturating_add(1);
                    timer.due = self.globals[&timer.global]
                        .active_time
                        .saturating_add(timer.delay);
                    timer.queued = false;
                    self.timers.insert(id, timer);
                }
            }
        }
    }
    /// Report host callback exceptions separately, then complete normally so one
    /// exception cannot starve later tasks, frames or microtasks.
    pub fn complete(&mut self, id: StepId) -> Result<(), LoopError> {
        if self.in_flight.as_ref().map(|(expected, _)| *expected) != Some(id) {
            return Err(LoopError::WrongStep);
        }
        let (_, completion) = self.in_flight.take().unwrap();
        match completion {
            Completion::Task(timer) => self.start_checkpoint(Resume::FinishTask(timer)),
            Completion::Microtask => {}
            Completion::Cleanup => {
                self.checkpoint = false;
                let Phase::Checkpoint(resume) = std::mem::replace(&mut self.phase, Phase::Idle)
                else {
                    unreachable!()
                };
                self.phase = match resume {
                    Resume::RestoreCallback => {
                        let (id, completion, phase, nesting) = self.suspended.take().unwrap();
                        self.in_flight = Some((id, completion));
                        self.timer_nesting = nesting;
                        phase
                    }
                    Resume::Idle => Phase::Idle,
                    Resume::Frames(frame) => Phase::Frames(frame),
                    Resume::StartFrames(global, timestamp) => {
                        let handles = self.globals[&global].frames.keys().copied().collect();
                        Phase::Frames(Frame {
                            global,
                            timestamp,
                            handles,
                        })
                    }
                    Resume::FinishTask(timer) => {
                        self.finish_timer(timer);
                        Phase::Idle
                    }
                };
            }
            Completion::BeforeRender(global, timestamp) => {
                self.start_checkpoint(Resume::StartFrames(global, timestamp));
            }
            Completion::Frame => {
                let Phase::Frames(frame) = std::mem::replace(&mut self.phase, Phase::Idle) else {
                    unreachable!()
                };
                self.start_checkpoint(Resume::Frames(frame));
            }
            Completion::AfterRender => self.start_checkpoint(Resume::Idle),
        }
        Ok(())
    }
}
#[cfg(test)]
#[path = "event_loop_tests.rs"]
mod tests;
