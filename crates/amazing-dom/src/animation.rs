//! Minimal Animation state for `Element.prototype.animate()` (the
//! `Animatable` mixin). DOM-B-REFLECT's scope is explicitly a *minimal*
//! Animation object -- finished/ready promises, cancel/finish/play/pause and
//! onfinish -- not the full Web Animations timing model (no timeline, no
//! effect/keyframe application, no real pending-then-later-resolve
//! scheduling). An animation is immediately in the "finished" play state as
//! soon as it is created: this is enough for real pages that call
//! `element.animate(...)` and then use the returned object's promises or
//! methods to stop throwing "value is not callable", without pretending to
//! model timing this crate does not implement.
use crate::{events::CallbackId, Dom, DomError};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AnimationId(pub usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayState { Idle, Running, Paused, Finished }

#[derive(Debug)]
struct AnimationData {
    play_state: PlayState,
    /// The Promise value root currently returned by `animation.finished`.
    /// `cancel()` replaces this with a freshly rejected promise so a getter
    /// read after `cancel()` observes the rejection, matching the one
    /// observable state transition this minimal model needs to support.
    finished: CallbackId,
    ready: CallbackId,
    onfinish: Option<CallbackId>,
}

#[derive(Debug, Default)]
pub struct Animations { entries: Vec<AnimationData> }

impl Dom {
    /// `finished` and `ready` are host-owned Promise roots, already resolved
    /// by the caller before this is called (this model has no pending state
    /// at creation: an animation with no timeline is immediately finished).
    pub fn create_animation(&mut self, finished: CallbackId, ready: CallbackId) -> AnimationId {
        let id = AnimationId(self.animations.entries.len());
        self.animations.entries.push(AnimationData {
            play_state: PlayState::Finished, finished, ready, onfinish: None,
        });
        id
    }
    fn entry(&self, id: AnimationId) -> Result<&AnimationData, DomError> {
        self.animations.entries.get(id.0).ok_or(DomError::InvalidStateError)
    }
    fn entry_mut(&mut self, id: AnimationId) -> Result<&mut AnimationData, DomError> {
        self.animations.entries.get_mut(id.0).ok_or(DomError::InvalidStateError)
    }
    pub fn animation_checked(&self, id: AnimationId) -> Result<(), DomError> { self.entry(id).map(|_| ()) }
    pub fn animation_play_state(&self, id: AnimationId) -> Result<PlayState, DomError> {
        Ok(self.entry(id)?.play_state)
    }
    pub fn animation_finished(&self, id: AnimationId) -> Result<CallbackId, DomError> {
        Ok(self.entry(id)?.finished)
    }
    pub fn animation_ready(&self, id: AnimationId) -> Result<CallbackId, DomError> {
        Ok(self.entry(id)?.ready)
    }
    pub fn animation_on_finish(&self, id: AnimationId) -> Result<Option<CallbackId>, DomError> {
        Ok(self.entry(id)?.onfinish)
    }
    pub fn set_animation_on_finish(&mut self, id: AnimationId, handler: Option<CallbackId>)
        -> Result<Option<CallbackId>, DomError> {
        Ok(std::mem::replace(&mut self.entry_mut(id)?.onfinish, handler))
    }
    /// `play()`/`pause()` play-state transitions. Neither has a timeline to
    /// drive here, so they never touch the promises or fire `onfinish`.
    pub fn set_animation_play_state(&mut self, id: AnimationId, state: PlayState) -> Result<(), DomError> {
        self.entry_mut(id)?.play_state = state;
        Ok(())
    }
    /// `cancel()`: idle is a terminal, repeatable no-op (matching the spec's
    /// "cancel an animation" early return once already idle). Replaces
    /// `finished` with a fresh (host-rejected) promise root, returned to the
    /// caller so it can retire the old root.
    pub fn animation_cancel(&mut self, id: AnimationId, new_finished: CallbackId)
        -> Result<Option<CallbackId>, DomError> {
        let entry = self.entry_mut(id)?;
        if entry.play_state == PlayState::Idle { return Ok(None); }
        entry.play_state = PlayState::Idle;
        Ok(Some(std::mem::replace(&mut entry.finished, new_finished)))
    }
    /// `finish()`: sets the play state and reports whether `onfinish` should
    /// run (every call, mirroring a real re-dispatched `finish` event; a
    /// fresh `finished` promise is not needed since this model settles it
    /// once at creation and never invalidates it except via `cancel()`).
    pub fn animation_finish(&mut self, id: AnimationId) -> Result<Option<CallbackId>, DomError> {
        self.entry_mut(id)?.play_state = PlayState::Finished;
        Ok(self.entry(id)?.onfinish)
    }
}
