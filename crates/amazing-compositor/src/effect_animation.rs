//! Sampling resolved effects needs no CSS, layout, paint view or raster source.
use super::*;
use FilterOperation as F;
impl FilterOperation {
    fn identity(self) -> Self {
        match self {
            F::Blur(_) => F::Blur(0.),
            F::Brightness(_) => F::Brightness(1.),
            F::Contrast(_) => F::Contrast(1.),
            F::Grayscale(_) => F::Grayscale(0.),
            F::HueRotate(_) => F::HueRotate(0.),
            F::Invert(_) => F::Invert(0.),
            F::Opacity(_) => F::Opacity(1.),
            F::Saturate(_) => F::Saturate(1.),
            F::Sepia(_) => F::Sepia(0.),
            F::DropShadow { .. } => F::DropShadow {
                offset: (0., 0.),
                sigma: 0.,
                color: [0; 4],
            },
        }
    }
    fn interpolate(self, to: Self, t: f32) -> Option<Self> {
        let lerp = |a: f32, b: f32| a + (b - a) * t;
        Some(match (self, to) {
            (F::Blur(a), F::Blur(b)) => F::Blur(lerp(a, b)),
            (F::Brightness(a), F::Brightness(b)) => F::Brightness(lerp(a, b)),
            (F::Contrast(a), F::Contrast(b)) => F::Contrast(lerp(a, b)),
            (F::Grayscale(a), F::Grayscale(b)) => F::Grayscale(lerp(a, b)),
            (F::HueRotate(a), F::HueRotate(b)) => F::HueRotate(lerp(a, b)),
            (F::Invert(a), F::Invert(b)) => F::Invert(lerp(a, b)),
            (F::Opacity(a), F::Opacity(b)) => F::Opacity(lerp(a, b)),
            (F::Saturate(a), F::Saturate(b)) => F::Saturate(lerp(a, b)),
            (F::Sepia(a), F::Sepia(b)) => F::Sepia(lerp(a, b)),
            (
                F::DropShadow {
                    offset: a,
                    sigma: sa,
                    color: ca,
                },
                F::DropShadow {
                    offset: b,
                    sigma: sb,
                    color: cb,
                },
            ) => {
                let alpha = lerp(f32::from(ca[3]), f32::from(cb[3]));
                let mut color = [0; 4];
                color[3] = alpha.round() as u8;
                if alpha > 0. {
                    for i in 0..3 {
                        color[i] = (lerp(
                            f32::from(ca[i]) * f32::from(ca[3]),
                            f32::from(cb[i]) * f32::from(cb[3]),
                        ) / alpha)
                            .round() as u8;
                    }
                }
                F::DropShadow {
                    offset: (lerp(a.0, b.0), lerp(a.1, b.1)),
                    sigma: lerp(sa, sb),
                    color,
                }
            }
            _ => return None,
        })
    }
}
fn sample_list(from: &[F], to: &[F], t: f32) -> Vec<F> {
    let mut out = Vec::new();
    for i in 0..from.len().max(to.len()) {
        let a = from.get(i).copied().unwrap_or_else(|| to[i].identity());
        let b = to.get(i).copied().unwrap_or_else(|| from[i].identity());
        let Some(value) = a.interpolate(b, t) else {
            return if t < 0.5 { from.to_vec() } else { to.to_vec() };
        };
        out.push(value);
    }
    out
}
/// `progress` is resolved/eased by the caller's timeline. Blend, isolation,
/// geometry and mask resources use discrete interpolation; numeric compatible
/// filter lists interpolate, with identity padding as specified by Filter Effects.
pub fn sample_effect_operations(
    from: &EffectOperations,
    to: &EffectOperations,
    progress: f32,
) -> Result<EffectOperations, EffectError> {
    from.validate()?;
    to.validate()?;
    if !progress.is_finite() {
        return Err(EffectError::InvalidValue);
    }
    let t = progress.clamp(0., 1.);
    let mut out = if t < 0.5 { from.clone() } else { to.clone() };
    out.filters = sample_list(&from.filters, &to.filters, t);
    out.backdrop_filters = sample_list(&from.backdrop_filters, &to.backdrop_filters, t);
    if !out.backdrop_filters.is_empty() && out.backdrop_bounds.is_none() {
        out.backdrop_bounds = from.backdrop_bounds.or(to.backdrop_bounds);
    }
    out.validate()?;
    Ok(out)
}

impl Compositor {
    /// Reserve both endpoint halos before the initial raster. Compatible
    /// filter interpolation stays within this envelope, so later blur/shadow
    /// samples can use build_cached_frame even at viewport/tile boundaries.
    pub fn reserve_effect_animation(
        &mut self,
        id: EffectId,
        from: &EffectOperations,
        to: &EffectOperations,
    ) -> Result<(), EffectError> {
        let mut trees = self.trees.clone();
        trees.set_effect_operations(id, from.clone())?;
        trees.set_effect_operations(id, to.clone())?;
        let halo = |op: &EffectOperations| {
            let mut backdrop = op.clone();
            backdrop.filters = op.backdrop_filters.clone();
            op.outset() + backdrop.outset()
        };
        let reserved = self.reserved_effect_halos.entry(id).or_default();
        *reserved = reserved.max(halo(from)).max(halo(to));
        Ok(())
    }
    pub(crate) fn effect_raster_halo(&self) -> f32 {
        self.trees
            .effect_halo_with_reservations(&self.reserved_effect_halos)
    }
}
