//! Keep a halo outside the viewport while filtering; otherwise an offscreen
//! source that blurs into view would be culled before the filter executes.
use super::*;
fn shift_rect(r: &mut Rect, p: f32) {
    r.x += p;
    r.y += p;
}
fn shift_point(v: &mut (f32, f32), p: f32) {
    v.0 += p;
    v.1 += p;
}
fn shift_clip(c: &mut EffectClip, p: f32) {
    match c {
        EffectClip::Inset { rect, .. } => shift_rect(rect, p),
        EffectClip::Ellipse { center, .. } => shift_point(center, p),
        EffectClip::Polygon { points, .. } => {
            for v in points {
                shift_point(v, p);
            }
        }
        EffectClip::Path { contours, .. } => {
            for v in contours.iter_mut().flatten() {
                shift_point(v, p);
            }
        }
    }
}
pub(crate) fn padded(
    frame: &CompositorFrame,
) -> Result<Option<(CompositorFrame, u32)>, PresenterError> {
    let radius = frame.trees.effect_halo();
    let pad = radius.ceil() as u32;
    if pad == 0 {
        return Ok(None);
    }
    let width = frame
        .width
        .checked_add(pad.checked_mul(2).ok_or(PresenterError::InvalidResource)?)
        .ok_or(PresenterError::InvalidResource)?;
    let height = frame
        .height
        .checked_add(pad * 2)
        .ok_or(PresenterError::InvalidResource)?;
    if u64::from(width) * u64::from(height) > MAX_EFFECT_PIXELS {
        return Err(PresenterError::InvalidResource);
    }
    let mut out = frame.clone();
    out.width = width;
    out.height = height;
    let p = pad as f32;
    let shift = Matrix4::translation(p, p, 0.);
    for transform in &mut out.trees.transforms {
        transform.accumulated = shift.mul(transform.accumulated);
    }
    for op in out.trees.effect_operations.values_mut() {
        if let Some(r) = &mut op.backdrop_bounds {
            shift_rect(r, p);
        }
        if let Some(c) = &mut op.clip_path {
            shift_clip(c, p);
        }
        if let Some(m) = &mut op.mask {
            shift_rect(&mut m.rect, p);
        }
    }
    for pass in &mut out.passes {
        for quad in &mut pass.quads {
            match quad {
                DrawQuad::Tile(q) => {
                    q.transform = shift.mul(q.transform);
                    shift_rect(&mut q.clip_rect, p);
                }
                DrawQuad::SolidColor(q) => {
                    shift_rect(&mut q.rect, p);
                    shift_rect(&mut q.clip_rect, p);
                }
                DrawQuad::Pass(q) => {
                    q.rect = Rect::new(0., 0., width as f32, height as f32);
                    q.clip_rect = q.rect;
                }
            }
        }
    }
    Ok(Some((out, pad)))
}
