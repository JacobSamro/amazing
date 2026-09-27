//! `Element.prototype.animate()` (Animatable mixin) and a minimal `Animation`
//! object -- finished/ready promises, cancel/finish/play/pause, onfinish.
//! DOM-B-REFLECT's scope is explicitly *not* the full Web Animations timing
//! model: there is no timeline, no keyframe/effect application, no real
//! pending-then-later-resolve scheduling. An animation this crate creates is
//! immediately in the "finished" play state -- see
//! `amazing-dom/src/animation.rs`'s module doc for why that is enough for
//! real pages that call `.animate(...)` and then use the returned object's
//! promises or methods, without pretending to model timing this engine does
//! not implement yet.
use crate::host::{BindingHost, InterfaceBrand, Operation as O, PlatformKey};
use amazing_dom::{animation::{AnimationId, PlayState}, DomError, NodeKind};

pub(crate) fn handles(op: O) -> bool {
    matches!(op, O::ElementAnimate | O::AnimationFinished | O::AnimationReady | O::AnimationCancel
        | O::AnimationFinish | O::AnimationPlay | O::AnimationPause
        | O::AnimationOnFinish | O::AnimationSetOnFinish)
}
fn key<H: BindingHost>(host: &H, id: AnimationId) -> PlatformKey {
    PlatformKey { arena: host.node_platform_key(host.current_document()).arena,
        slot: id.0 as u64, generation: 1 }
}
fn animation_id<H: BindingHost>(host: &mut H, value: &H::Value) -> Result<AnimationId, H::Error> {
    let platform_key = host.platform_object(value, InterfaceBrand::ANIMATION)
        .ok_or_else(|| host.type_error("Expected an Animation"))?;
    let id = AnimationId(platform_key.slot as usize);
    host.with_dom(|dom| dom.animation_checked(id))?;
    Ok(id)
}
pub(crate) fn invoke<H: BindingHost>(host: &mut H, op: O, receiver: H::Value, args: &[H::Value])
    -> Result<H::Value, H::Error> {
    if op == O::ElementAnimate {
        let node = host.node_brand(&receiver).ok_or_else(|| host.type_error("Expected an Element"))?;
        host.with_dom(|dom| if matches!(dom.get(node).kind, NodeKind::Element { .. }) { Ok(()) }
            else { Err(DomError::TypeError) })?;
        let finished_value = host.new_promise()?;
        let ready_value = host.new_promise()?;
        let finished_root = host.retain_value(finished_value.clone());
        let ready_root = host.retain_value(ready_value.clone());
        let id = host.with_dom(|dom| Ok(dom.create_animation(finished_root, ready_root)))?;
        let animation = host.wrap_platform_object(key(host, id), InterfaceBrand::ANIMATION)?;
        host.resolve_promise(&finished_value, animation.clone())?;
        host.resolve_promise(&ready_value, animation.clone())?;
        return Ok(animation);
    }
    let id = animation_id(host, &receiver)?;
    match op {
        O::AnimationFinished => {
            let root = host.with_dom(|dom| dom.animation_finished(id))?;
            Ok(host.root_value(root).unwrap_or_else(|| host.undefined()))
        }
        O::AnimationReady => {
            let root = host.with_dom(|dom| dom.animation_ready(id))?;
            Ok(host.root_value(root).unwrap_or_else(|| host.undefined()))
        }
        O::AnimationOnFinish => {
            let handler = host.with_dom(|dom| dom.animation_on_finish(id))?;
            Ok(handler.and_then(|root| host.root_value(root)).unwrap_or_else(|| host.null()))
        }
        O::AnimationSetOnFinish => {
            let value = args.first().ok_or_else(|| host.type_error("Not enough arguments"))?;
            let handler = if host.is_callable(value) { Some(host.retain_value(value.clone())) } else { None };
            let old = host.with_dom(|dom| dom.set_animation_on_finish(id, handler))?;
            if let Some(old) = old { host.release_value(old); }
            Ok(host.undefined())
        }
        O::AnimationPlay => {
            host.with_dom(|dom| dom.set_animation_play_state(id, PlayState::Running))?;
            Ok(host.undefined())
        }
        O::AnimationPause => {
            host.with_dom(|dom| dom.set_animation_play_state(id, PlayState::Paused))?;
            Ok(host.undefined())
        }
        O::AnimationFinish => {
            let onfinish = host.with_dom(|dom| dom.animation_finish(id))?;
            if let Some(handler) = onfinish.and_then(|root| host.root_value(root)) {
                let animation = host.wrap_platform_object(key(host, id), InterfaceBrand::ANIMATION)?;
                host.call(&handler, &animation, &[])?;
            }
            Ok(host.undefined())
        }
        O::AnimationCancel => {
            let reason = crate::abort_bindings::default_reason(host, "AbortError")?;
            let rejected = host.new_promise()?;
            host.reject_promise(&rejected, reason)?;
            let new_root = host.retain_value(rejected);
            let old = host.with_dom(|dom| dom.animation_cancel(id, new_root))?;
            match old {
                Some(old) => host.release_value(old),
                // Already idle: the freshly created (but unused) rejected
                // promise's root is released rather than kept live forever.
                None => host.release_value(new_root),
            }
            Ok(host.undefined())
        }
        _ => unreachable!("routed by handles()"),
    }
}
