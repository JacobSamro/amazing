//! Provider-neutral intrinsic image sizing. CSS constraint resolution belongs
//! to layout; this helper resolves definite dimensions and natural aspect ratio.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImageSize {
    pub width: f32,
    pub height: f32,
}
impl ImageSize {
    /// Resolve definite CSS/HTML dimensions; one auto axis keeps the decoded
    /// natural ratio. Explicit zero is preserved. Reject non-finite geometry.
    pub fn resolve(
        natural_width: u32,
        natural_height: u32,
        width: Option<f32>,
        height: Option<f32>,
    ) -> Option<Self> {
        if natural_width == 0
            || natural_height == 0
            || width
                .into_iter()
                .chain(height)
                .any(|n| !n.is_finite() || n < 0.0)
        {
            return None;
        }
        let ratio = natural_width as f64 / natural_height as f64;
        let (w, h) = match (width, height) {
            (Some(w), Some(h)) => (w as f64, h as f64),
            (Some(w), None) => (w as f64, w as f64 / ratio),
            (None, Some(h)) => (h as f64 * ratio, h as f64),
            (None, None) => (natural_width as f64, natural_height as f64),
        };
        let result = Self {
            width: w as f32,
            height: h as f32,
        };
        (result.width.is_finite() && result.height.is_finite()).then_some(result)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn natural_ratio_and_definite_overrides() {
        for (w, h, expected) in [
            (None, None, (200.0, 100.0)),
            (Some(80.0), None, (80.0, 40.0)),
            (None, Some(20.0), (40.0, 20.0)),
            (Some(10.0), Some(30.0), (10.0, 30.0)),
            (Some(0.0), None, (0.0, 0.0)),
        ] {
            let s = ImageSize::resolve(200, 100, w, h).unwrap();
            assert_eq!((s.width, s.height), expected);
        }
    }
    #[test]
    fn invalid_and_overflowing_dimensions_are_rejected() {
        assert!(ImageSize::resolve(0, 1, None, None).is_none());
        for n in [f32::NAN, f32::INFINITY, -1.0] {
            assert!(ImageSize::resolve(2, 1, Some(n), None).is_none());
        }
        assert!(ImageSize::resolve(u32::MAX, 1, None, Some(f32::MAX)).is_none());
    }
}
