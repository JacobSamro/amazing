//! Actual CSS output lattice, independent of layout zoom and image density.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RasterViewport {
    css_viewport: (f64, f64),
    logical_output: (f64, f64),
    scale: f64,
    physical: (u32, u32),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RasterError {
    InvalidViewport,
    ResourceLimit,
    UnsupportedText,
    RasterFailure,
}
impl RasterViewport {
    pub fn new(
        css_viewport: (f64, f64),
        logical_output: (f64, f64),
        scale: f64,
    ) -> Result<Self, RasterError> {
        if ![
            css_viewport.0,
            css_viewport.1,
            logical_output.0,
            logical_output.1,
            scale,
        ]
        .iter()
        .all(|v| v.is_finite() && *v > 0.0)
        {
            return Err(RasterError::InvalidViewport);
        }
        if [
            css_viewport.0,
            css_viewport.1,
            logical_output.0,
            logical_output.1,
            scale,
        ]
        .iter()
        .any(|v| !(*v as f32).is_finite() || *v as f32 == 0.0)
        {
            return Err(RasterError::InvalidViewport);
        }
        let w = (logical_output.0 * scale).ceil();
        let h = (logical_output.1 * scale).ceil();
        if !w.is_finite() || !h.is_finite() || w > u32::MAX as f64 || h > u32::MAX as f64 {
            return Err(RasterError::ResourceLimit);
        }
        let physical = (w as u32, h as u32);
        if u64::from(physical.0) * u64::from(physical.1) > 16 * 1024 * 1024 {
            return Err(RasterError::ResourceLimit);
        }
        Ok(Self {
            css_viewport,
            logical_output,
            scale,
            physical,
        })
    }
    pub fn css_viewport_size(self) -> (f64, f64) {
        self.css_viewport
    }
    pub fn logical_output_size(self) -> (f64, f64) {
        self.logical_output
    }
    pub fn device_scale(self) -> f64 {
        self.scale
    }
    pub fn css_scroll_to_device(self, offset: (f32, f32)) -> Option<(f32, f32)> {
        let value = (
            f64::from(offset.0) * self.scale,
            f64::from(offset.1) * self.scale,
        );
        (value.0.is_finite()
            && value.1.is_finite()
            && value.0.abs() <= f32::MAX as f64
            && value.1.abs() <= f32::MAX as f64)
            .then_some((value.0 as f32, value.1 as f32))
    }
    /// Conjugate an authored CSS affine matrix by the device lattice.
    pub fn css_affine_to_device(self, mut matrix: [f32; 6]) -> Option<[f32; 6]> {
        if !matrix.iter().all(|v| v.is_finite()) {
            return None;
        }
        let offset = self.css_scroll_to_device((matrix[4], matrix[5]))?;
        matrix[4] = offset.0;
        matrix[5] = offset.1;
        Some(matrix)
    }
    pub fn physical_size(self) -> (u32, u32) {
        self.physical
    }
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct RasterLattice {
    pub scale: f64,
    pub offset: (f64, f64),
}
impl RasterLattice {
    pub const UNIT: Self = Self {
        scale: 1.0,
        offset: (0.0, 0.0),
    };
    pub fn css_point(self, x: f64, y: f64) -> (f64, f64) {
        (
            (x + self.offset.0) / self.scale,
            (y + self.offset.1) / self.scale,
        )
    }
    pub fn device_point(self, x: f64, y: f64) -> (f64, f64) {
        (
            x * self.scale - self.offset.0,
            y * self.scale - self.offset.1,
        )
    }
    pub fn bounds(self, rect: [f32; 4]) -> [i64; 4] {
        let a = self.device_point(f64::from(rect[0]), f64::from(rect[1]));
        let b = self.device_point(f64::from(rect[2]), f64::from(rect[3]));
        if self == Self::UNIT {
            return [
                a.0.round() as i64,
                a.1.round() as i64,
                b.0.round() as i64,
                b.1.round() as i64,
            ];
        }
        [
            (a.0 - 0.5).ceil() as i64,
            (a.1 - 0.5).ceil() as i64,
            (b.0 - 0.5).ceil() as i64,
            (b.1 - 0.5).ceil() as i64,
        ]
    }
}
