use super::*;
fn point(p: (f32, f32)) -> bool {
    p.0.is_finite() && p.1.is_finite() && p.0.abs() <= 1e7 && p.1.abs() <= 1e7
}
impl EffectClip {
    pub(crate) fn validate(&self) -> Result<(), EffectError> {
        let valid = match self {
            Self::Inset { rect, radii } => {
                !rect.empty() && radii.iter().all(|v| v.is_finite() && *v >= 0.)
            }
            Self::Ellipse { center, radii } => {
                point(*center) && point(*radii) && radii.0 > 0. && radii.1 > 0.
            }
            Self::Polygon { points, .. } => {
                if points.len() > MAX_EFFECT_POINTS {
                    return Err(EffectError::LimitExceeded);
                }
                points.len() >= 3 && points.iter().all(|p| point(*p))
            }
            Self::Path { contours, .. } => {
                if contours.len() > MAX_EFFECT_POINTS
                    || contours.iter().map(Vec::len).sum::<usize>() > MAX_EFFECT_POINTS
                {
                    return Err(EffectError::LimitExceeded);
                }
                contours
                    .iter()
                    .all(|p| p.len() >= 3 && p.iter().all(|p| point(*p)))
            }
        };
        if valid {
            Ok(())
        } else {
            Err(EffectError::InvalidValue)
        }
    }
    fn contains(&self, x: f32, y: f32) -> bool {
        match self {
            Self::Inset { rect, radii } => {
                if !rect.contains(x, y) {
                    return false;
                }
                // CSS scales all corner radii together when their sums exceed
                // an edge, preserving each corner's relative radius.
                let mut scale = 1f32;
                for (length, sum) in [
                    (rect.w, radii[0] + radii[1]),
                    (rect.w, radii[2] + radii[3]),
                    (rect.h, radii[0] + radii[3]),
                    (rect.h, radii[1] + radii[2]),
                ] {
                    if sum > 0. {
                        scale = scale.min(length / sum);
                    }
                }
                let r = radii.map(|r| r * scale);
                for (cx, cy, rad, inside) in [
                    (
                        rect.x + r[0],
                        rect.y + r[0],
                        r[0],
                        x < rect.x + r[0] && y < rect.y + r[0],
                    ),
                    (
                        rect.x + rect.w - r[1],
                        rect.y + r[1],
                        r[1],
                        x > rect.x + rect.w - r[1] && y < rect.y + r[1],
                    ),
                    (
                        rect.x + rect.w - r[2],
                        rect.y + rect.h - r[2],
                        r[2],
                        x > rect.x + rect.w - r[2] && y > rect.y + rect.h - r[2],
                    ),
                    (
                        rect.x + r[3],
                        rect.y + rect.h - r[3],
                        r[3],
                        x < rect.x + r[3] && y > rect.y + rect.h - r[3],
                    ),
                ] {
                    if inside && (x - cx).powi(2) + (y - cy).powi(2) > rad.powi(2) {
                        return false;
                    }
                }
                true
            }
            Self::Ellipse { center, radii } => {
                ((x - center.0) / radii.0).powi(2) + ((y - center.1) / radii.1).powi(2) <= 1.
            }
            Self::Polygon { points, rule } => inside(winding(points, x, y), *rule),
            Self::Path { contours, rule } => {
                inside(contours.iter().map(|p| winding(p, x, y)).sum(), *rule)
            }
        }
    }
    pub fn coverage(&self, x: u32, y: u32) -> f32 {
        let mut count = 0;
        for sy in 0..4 {
            for sx in 0..4 {
                if self.contains(
                    x as f32 + (sx as f32 + 0.5) / 4.,
                    y as f32 + (sy as f32 + 0.5) / 4.,
                ) {
                    count += 1;
                }
            }
        }
        count as f32 / 16.
    }
}
fn inside(winding: i32, rule: FillRule) -> bool {
    match rule {
        FillRule::NonZero => winding != 0,
        FillRule::EvenOdd => winding % 2 != 0,
    }
}
fn winding(points: &[(f32, f32)], x: f32, y: f32) -> i32 {
    let mut result = 0;
    for (&a, &b) in points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .take(points.len())
    {
        let side = (b.0 - a.0) * (y - a.1) - (x - a.0) * (b.1 - a.1);
        if a.1 <= y && b.1 > y && side > 0. {
            result += 1;
        }
        if a.1 > y && b.1 <= y && side < 0. {
            result -= 1;
        }
    }
    result
}
impl EffectMask {
    pub(crate) fn coverage(&self, x: u32, y: u32) -> f32 {
        let (x, y) = (x as f32 + 0.5, y as f32 + 0.5);
        if !self.rect.contains(x, y) {
            return 0.;
        }
        let sx = ((x - self.rect.x) / self.rect.w * self.image.width as f32).floor() as u32;
        let sy = ((y - self.rect.y) / self.rect.h * self.image.height as f32).floor() as u32;
        let c = self.image.get_pixel(sx, sy).unwrap_or([0; 4]);
        let a = f32::from(c[3]) / 255.;
        match self.mode {
            MaskMode::Alpha => a,
            MaskMode::Luminance => {
                a * (0.2125 * f32::from(c[0]) + 0.7154 * f32::from(c[1]) + 0.0721 * f32::from(c[2]))
                    / 255.
            }
        }
    }
}
pub(crate) fn apply_masks(surface: &mut Surface, operations: &EffectOperations) {
    for y in 0..surface.height {
        for x in 0..surface.width {
            // A single center-point sample, not `EffectClip::coverage`'s 4x4
            // supersampling: the reference painter's `RoundedRect::contains`
            // mask test in `composite_group` is a hard per-pixel boolean with
            // no antialiasing, and a real-painter-imported `clip_path` must
            // reproduce that pixel-exact edge rather than a smoother one.
            let coverage = operations
                .clip_path
                .as_ref()
                .map_or(1., |c| if c.contains(x as f32 + 0.5, y as f32 + 0.5) { 1. } else { 0. })
                * operations.mask.as_ref().map_or(1., |m| m.coverage(x, y));
            let i = ((y * surface.width + x) * 4 + 3) as usize;
            surface.pixels[i] = (f32::from(surface.pixels[i]) * coverage).round() as u8;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ClipPathSegment {
    MoveTo(f32, f32),
    LineTo(f32, f32),
    QuadraticTo {
        control: (f32, f32),
        end: (f32, f32),
    },
    CubicTo {
        first: (f32, f32),
        second: (f32, f32),
        end: (f32, f32),
    },
    /// Resolved SVG elliptical arc, angles in radians. The CSS/SVG producer
    /// resolves endpoint flags into center/radii/rotation/start/sweep once.
    Arc {
        center: (f32, f32),
        radii: (f32, f32),
        rotation: f32,
        start: f32,
        sweep: f32,
    },
    Close,
}
impl EffectClip {
    pub fn from_path(segments: &[ClipPathSegment], rule: FillRule) -> Result<Self, EffectError> {
        if segments.len() > MAX_EFFECT_POINTS {
            return Err(EffectError::LimitExceeded);
        }
        let mut contours = Vec::new();
        let mut current = Vec::new();
        let mut position = (0., 0.);
        let mut total = 0usize;
        let push = |p: (f32, f32),
                    out: &mut Vec<(f32, f32)>,
                    total: &mut usize|
         -> Result<(), EffectError> {
            if !point(p) {
                return Err(EffectError::InvalidValue);
            }
            *total += 1;
            if *total > MAX_EFFECT_POINTS {
                return Err(EffectError::LimitExceeded);
            }
            out.push(p);
            Ok(())
        };
        for segment in segments {
            match *segment {
                ClipPathSegment::MoveTo(x, y) => {
                    if current.len() >= 3 {
                        contours.push(std::mem::take(&mut current));
                    } else {
                        current.clear();
                    }
                    push((x, y), &mut current, &mut total)?;
                    position = (x, y);
                }
                ClipPathSegment::Close => {
                    if let Some(&start) = current.first() {
                        position = start;
                    }
                    if current.len() >= 3 {
                        contours.push(std::mem::take(&mut current));
                    } else {
                        current.clear();
                    }
                }
                other => {
                    if current.is_empty() {
                        push(position, &mut current, &mut total)?;
                    }
                    match other {
                        ClipPathSegment::LineTo(x, y) => {
                            position = (x, y);
                            push(position, &mut current, &mut total)?;
                        }
                        ClipPathSegment::QuadraticTo { control, end } => {
                            let a = (
                                position.0 + (control.0 - position.0) * 2. / 3.,
                                position.1 + (control.1 - position.1) * 2. / 3.,
                            );
                            let b = (
                                end.0 + (control.0 - end.0) * 2. / 3.,
                                end.1 + (control.1 - end.1) * 2. / 3.,
                            );
                            flatten(position, a, b, end, 0, &mut current, &mut total)?;
                            position = end;
                        }
                        ClipPathSegment::CubicTo { first, second, end } => {
                            flatten(position, first, second, end, 0, &mut current, &mut total)?;
                            position = end;
                        }
                        ClipPathSegment::Arc {
                            center,
                            radii,
                            rotation,
                            start,
                            sweep,
                        } => {
                            if !point(center)
                                || !point(radii)
                                || radii.0 < 0.
                                || radii.1 < 0.
                                || ![rotation, start, sweep].into_iter().all(f32::is_finite)
                                || sweep.abs() > std::f32::consts::TAU + 1e-5
                            {
                                return Err(EffectError::InvalidValue);
                            }
                            let radius = radii.0.max(radii.1);
                            let step = if radius > 0.125 {
                                2. * (1. - 0.125 / radius).acos()
                            } else {
                                std::f32::consts::PI / 2.
                            };
                            let count = (sweep.abs() / step.max(0.00001)).ceil().max(1.) as usize;
                            if count > MAX_EFFECT_POINTS {
                                return Err(EffectError::LimitExceeded);
                            }
                            let (sr, cr) = rotation.sin_cos();
                            for i in 1..=count {
                                let angle = start + sweep * i as f32 / count as f32;
                                let (s, c) = angle.sin_cos();
                                position = (
                                    center.0 + cr * radii.0 * c - sr * radii.1 * s,
                                    center.1 + sr * radii.0 * c + cr * radii.1 * s,
                                );
                                push(position, &mut current, &mut total)?;
                            }
                        }
                        _ => unreachable!(),
                    }
                }
            }
        }
        if current.len() >= 3 {
            contours.push(current);
        }
        let result = Self::Path { contours, rule };
        result.validate()?;
        Ok(result)
    }
}
fn flatten(
    a: (f32, f32),
    b: (f32, f32),
    c: (f32, f32),
    d: (f32, f32),
    depth: usize,
    out: &mut Vec<(f32, f32)>,
    total: &mut usize,
) -> Result<(), EffectError> {
    if ![a, b, c, d].into_iter().all(point) {
        return Err(EffectError::InvalidValue);
    }
    let dx = d.0 - a.0;
    let dy = d.1 - a.1;
    let length = dx.hypot(dy);
    let distance = |p: (f32, f32)| {
        if length > 1e-5 {
            ((p.0 - a.0) * dy - (p.1 - a.1) * dx).abs() / length
        } else {
            (p.0 - a.0).hypot(p.1 - a.1)
        }
    };
    // Also bound the control polygon: collinear reversing curves can escape
    // the endpoints despite zero distance to the infinite chord.
    let polygon =
        (b.0 - a.0).hypot(b.1 - a.1) + (c.0 - b.0).hypot(c.1 - b.1) + (d.0 - c.0).hypot(d.1 - c.1);
    if distance(b).max(distance(c)) <= 0.125 && polygon - length <= 0.125 {
        *total += 1;
        if *total > MAX_EFFECT_POINTS {
            return Err(EffectError::LimitExceeded);
        }
        out.push(d);
        return Ok(());
    }
    if depth >= 20 {
        return Err(EffectError::LimitExceeded);
    }
    let mid = |p: (f32, f32), q: (f32, f32)| ((p.0 + q.0) * 0.5, (p.1 + q.1) * 0.5);
    let ab = mid(a, b);
    let bc = mid(b, c);
    let cd = mid(c, d);
    let abc = mid(ab, bc);
    let bcd = mid(bc, cd);
    let center = mid(abc, bcd);
    flatten(a, ab, abc, center, depth + 1, out, total)?;
    flatten(center, bcd, cd, d, depth + 1, out, total)
}
