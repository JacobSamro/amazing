//! Units shared by live content measurement, layout, input and presentation.
//! A device scale changes raster density; it never changes CSS layout sizes.

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewportMetrics {
    pub css_width: f32,
    pub css_height: f32,
    pub device_scale: f32,
    pub raster_width: u32,
    pub raster_height: u32,
}

impl Default for ViewportMetrics {
    fn default() -> Self { Self::new(960., 570., 1.).expect("legacy viewport") }
}

impl ViewportMetrics {
    /// Empty or invalid measurements do not replace the last valid frame.
    /// Checked physical extents avoid Rust's saturating float-to-int casts.
    pub fn new(css_width: f32, css_height: f32, device_scale: f32) -> Option<Self> {
        fn extent(css: f32, scale: f32) -> Option<u32> {
            if !css.is_finite() || css <= 0. { return None; }
            let value = (f64::from(css) * f64::from(scale)).ceil();
            (value.is_finite() && value >= 1. && value <= f64::from(u32::MAX))
                .then_some(value as u32)
        }
        if !device_scale.is_finite() || device_scale <= 0. { return None; }
        let raster_width = extent(css_width, device_scale)?;
        let raster_height = extent(css_height, device_scale)?;
        // The allocator/raster provider still owns its real byte admission.
        // This check only proves that the requested RGBA length is representable.
        (raster_width as usize).checked_mul(raster_height as usize)?.checked_mul(4)?;
        Some(Self { css_width, css_height, device_scale, raster_width, raster_height })
    }

    pub fn raster_viewport(self) -> Result<amazing_paint::RasterViewport, String> {
        let logical = (f64::from(self.css_width), f64::from(self.css_height));
        amazing_paint::RasterViewport::new(logical, logical, f64::from(self.device_scale))
            .map_err(|error| format!("viewport: {error:?}"))
    }

    pub fn matches_logical_bounds(self, width: f32, height: f32) -> bool {
        self.css_width == width && self.css_height == height
    }

    /// GPUI pointer positions and scroll offsets are logical pixels. The two
    /// axes use independent bounds. Live input is admitted only after the
    /// measurement's matching reflow, so an old unstretched frame is not hit
    /// tested as if it had filled newly resized content bounds.
    pub fn document_point(self, local: (f32, f32), bounds: (f32, f32), scroll: (f32, f32))
        -> Option<(f32, f32)>
    {
        if ![local.0, local.1, bounds.0, bounds.1, scroll.0, scroll.1]
            .iter().all(|value| value.is_finite()) || bounds.0 <= 0. || bounds.1 <= 0.
        { return None; }
        let x = local.0 * (self.css_width / bounds.0) + scroll.0;
        let y = local.1 * (self.css_height / bounds.1) + scroll.1;
        (x.is_finite() && y.is_finite()).then_some((x, y))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logical_resize_and_density_have_separate_extents() {
        for (w, h, dpr, rw, rh) in [
            (960., 570., 1., 960, 570), (1231.5, 689.25, 1., 1232, 690),
            (1231.5, 689.25, 2., 2463, 1379), (801., 603., 1.25, 1002, 754),
        ] {
            let metrics = ViewportMetrics::new(w, h, dpr).unwrap();
            assert_eq!((metrics.css_width, metrics.css_height), (w, h));
            assert_eq!((metrics.raster_width, metrics.raster_height), (rw, rh));
        }
    }

    #[test]
    fn invalid_measurements_and_overflow_never_make_a_valid_extent() {
        for bad in [0., -1., f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert!(ViewportMetrics::new(bad, 600., 1.).is_none());
            assert!(ViewportMetrics::new(800., bad, 1.).is_none());
            assert!(ViewportMetrics::new(800., 600., bad).is_none());
        }
        assert!(ViewportMetrics::new(f32::MAX, 600., 2.).is_none());
        assert!(ViewportMetrics::new(1., f32::MAX, 1.).is_none());
    }

    #[test]
    fn input_uses_both_axes_and_css_scroll_at_every_density() {
        for density in [1., 1.25, 2., 3.] {
            let metrics = ViewportMetrics::new(1000., 600., density).unwrap();
            assert_eq!(metrics.document_point((200., 150.), (500., 300.), (7., 90.)),
                Some((407., 390.)));
            assert_eq!(metrics.document_point((200., 150.), (1000., 600.), (7., 90.)),
                Some((207., 240.)));
            assert_eq!(metrics.document_point((0., 0.), (0., 600.), (0., 0.)), None);
        }
    }
}
