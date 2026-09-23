use crate::{
    composite, Bitmap, CompositeOperation, ImageData, ImageError, PaintStyle, Path2D, Point,
    TextAlign, TextBaseline, Transform,
};
use amazing_paint::Surface;
use amazing_text::TextStyle;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FillRule {
    #[default]
    NonZero,
    EvenOdd,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LineCap {
    #[default]
    Butt,
    Round,
    Square,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LineJoin {
    #[default]
    Miter,
    Round,
    Bevel,
}

/// Canvas filters are retained as opaque style input until a filter backend is
/// installed. Callers can distinguish the supported `none` value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterStatus {
    None,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CanvasState {
    pub transform: Transform,
    pub fill_style: PaintStyle,
    pub stroke_style: PaintStyle,
    pub global_alpha: f64,
    pub composite_operation: CompositeOperation,
    pub line_width: f64,
    pub line_cap: LineCap,
    pub line_join: LineJoin,
    pub miter_limit: f64,
    pub line_dash: Vec<f64>,
    pub line_dash_offset: f64,
    pub image_smoothing_enabled: bool,
    pub shadow_color: [u8; 4],
    pub shadow_blur: f64,
    pub shadow_offset_x: f64,
    pub shadow_offset_y: f64,
    pub filter: String,
    pub text_style: TextStyle,
    pub text_align: TextAlign,
    pub text_baseline: TextBaseline,
    pub(crate) clip: Vec<f32>,
}

impl CanvasState {
    fn new(width: u32, height: u32) -> Self {
        Self {
            transform: Transform::IDENTITY,
            fill_style: PaintStyle::default(),
            stroke_style: PaintStyle::default(),
            global_alpha: 1.,
            composite_operation: CompositeOperation::SourceOver,
            line_width: 1.,
            line_cap: LineCap::Butt,
            line_join: LineJoin::Miter,
            miter_limit: 10.,
            line_dash: Vec::new(),
            line_dash_offset: 0.,
            image_smoothing_enabled: true,
            shadow_color: [0; 4],
            shadow_blur: 0.,
            shadow_offset_x: 0.,
            shadow_offset_y: 0.,
            filter: "none".into(),
            text_style: TextStyle::default(),
            text_align: TextAlign::Start,
            text_baseline: TextBaseline::Alphabetic,
            clip: vec![1.; width as usize * height as usize],
        }
    }
}

/// Script-independent Canvas 2D drawing context. The surface is straight-alpha
/// RGBA; compositing converts channels to premultiplied values internally.
pub struct Canvas2D {
    pub surface: Surface,
    pub state: CanvasState,
    stack: Vec<CanvasState>,
    path: Path2D,
}

impl Canvas2D {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            surface: Surface::new(width, height),
            state: CanvasState::new(width, height),
            stack: Vec::new(),
            path: Path2D::new(),
        }
    }
    pub fn into_surface(self) -> Surface {
        self.surface
    }
    pub fn set_filter(&mut self, value: &str) -> FilterStatus {
        self.state.filter = value.to_owned();
        if value.trim().eq_ignore_ascii_case("none") {
            FilterStatus::None
        } else {
            FilterStatus::Unsupported
        }
    }
    pub fn save(&mut self) {
        self.stack.push(self.state.clone());
    }
    pub fn restore(&mut self) {
        if let Some(state) = self.stack.pop() {
            self.state = state;
        }
    }
    pub fn reset_transform(&mut self) {
        self.state.transform = Transform::IDENTITY;
    }
    pub fn set_transform(&mut self, t: Transform) {
        if t.finite() {
            self.state.transform = t;
        }
    }
    pub fn transform(&mut self, t: Transform) {
        if t.finite() {
            self.state.transform = self.state.transform.multiply(t);
        }
    }
    pub fn translate(&mut self, x: f64, y: f64) {
        self.transform(Transform::translation(x, y));
    }
    pub fn scale(&mut self, x: f64, y: f64) {
        self.transform(Transform::scale(x, y));
    }
    pub fn rotate(&mut self, angle: f64) {
        self.transform(Transform::rotation(angle));
    }
    pub fn set_fill_color(&mut self, value: &str) -> bool {
        if let Some(style) = PaintStyle::from_css_color(value) {
            self.state.fill_style = style;
            true
        } else {
            false
        }
    }
    pub fn set_stroke_color(&mut self, value: &str) -> bool {
        if let Some(style) = PaintStyle::from_css_color(value) {
            self.state.stroke_style = style;
            true
        } else {
            false
        }
    }
    pub fn set_global_alpha(&mut self, alpha: f64) {
        if alpha.is_finite() && (0. ..=1.).contains(&alpha) {
            self.state.global_alpha = alpha;
        }
    }
    pub fn set_line_width(&mut self, width: f64) {
        if width.is_finite() && width > 0. {
            self.state.line_width = width;
        }
    }
    pub fn set_line_dash(&mut self, dash: &[f64]) {
        if dash.iter().all(|v| v.is_finite() && *v >= 0.) {
            self.state.line_dash = if dash.len() % 2 == 1 {
                dash.iter().chain(dash.iter()).copied().collect()
            } else {
                dash.to_vec()
            };
        }
    }
    pub fn begin_path(&mut self) {
        self.path = Path2D::new();
    }
    pub fn current_path(&self) -> &Path2D {
        &self.path
    }
    pub fn move_to(&mut self, x: f64, y: f64) {
        let p = self.state.transform.map(Point::new(x, y));
        self.path.move_to(p.x, p.y);
    }
    pub fn line_to(&mut self, x: f64, y: f64) {
        let p = self.state.transform.map(Point::new(x, y));
        self.path.line_to(p.x, p.y);
    }
    pub fn quadratic_curve_to(&mut self, cx: f64, cy: f64, x: f64, y: f64) {
        let c = self.state.transform.map(Point::new(cx, cy));
        let p = self.state.transform.map(Point::new(x, y));
        self.path.quadratic_curve_to(c.x, c.y, p.x, p.y);
    }
    pub fn bezier_curve_to(&mut self, c1x: f64, c1y: f64, c2x: f64, c2y: f64, x: f64, y: f64) {
        let a = self.state.transform.map(Point::new(c1x, c1y));
        let b = self.state.transform.map(Point::new(c2x, c2y));
        let p = self.state.transform.map(Point::new(x, y));
        self.path.bezier_curve_to(a.x, a.y, b.x, b.y, p.x, p.y);
    }
    pub fn close_path(&mut self) {
        self.path.close_path();
    }
    fn append_local(&mut self, mut p: Path2D) {
        for contour in p.contours.drain(..) {
            for (i, point) in contour.points.into_iter().enumerate() {
                let mapped = self.state.transform.map(point);
                if i == 0 && self.path.current_point().is_none() {
                    self.path.move_to(mapped.x, mapped.y)
                } else {
                    self.path.line_to(mapped.x, mapped.y)
                }
            }
            if contour.closed {
                self.path.close_path();
            }
        }
    }
    pub fn rect(&mut self, x: f64, y: f64, w: f64, h: f64) {
        let mut p = Path2D::new();
        p.rect(x, y, w, h);
        self.path.add_path(&p, self.state.transform);
    }
    pub fn round_rect(&mut self, x: f64, y: f64, w: f64, h: f64, radii: [f64; 4]) {
        let mut p = Path2D::new();
        p.round_rect(x, y, w, h, radii);
        self.path.add_path(&p, self.state.transform);
    }
    pub fn arc(&mut self, x: f64, y: f64, r: f64, start: f64, end: f64, ccw: bool) {
        let mut p = Path2D::new();
        p.arc(x, y, r, start, end, ccw);
        self.append_local(p);
    }
    pub fn arc_to(&mut self, x1: f64, y1: f64, x2: f64, y2: f64, r: f64) {
        let mut p = Path2D::new();
        if let Some(now) = self
            .path
            .current_point()
            .and_then(|v| self.state.transform.inverse().map(|t| t.map(v)))
        {
            p.move_to(now.x, now.y);
        }
        p.arc_to(x1, y1, x2, y2, r);
        self.append_local(p);
    }
    pub fn ellipse(
        &mut self,
        x: f64,
        y: f64,
        rx: f64,
        ry: f64,
        rotation: f64,
        start: f64,
        end: f64,
        ccw: bool,
    ) {
        let mut p = Path2D::new();
        p.ellipse(x, y, rx, ry, rotation, start, end, ccw);
        self.append_local(p);
    }
    pub fn fill(&mut self, rule: FillRule) {
        let p = self.path.clone();
        self.draw_path(&p, rule, false, false);
    }
    pub fn stroke(&mut self) {
        let p = self.path.clone();
        self.draw_path(&p, FillRule::NonZero, true, false);
    }
    pub fn fill_path(&mut self, path: &Path2D, rule: FillRule) {
        let mut p = Path2D::new();
        p.add_path(path, self.state.transform);
        self.draw_path(&p, rule, false, false);
    }
    pub fn stroke_path(&mut self, path: &Path2D) {
        let mut p = Path2D::new();
        p.add_path(path, self.state.transform);
        self.draw_path(&p, FillRule::NonZero, true, false);
    }
    pub fn clip(&mut self, rule: FillRule) {
        let p = self.path.clone();
        self.clip_path_device(&p, rule);
    }
    pub fn clip_path(&mut self, path: &Path2D, rule: FillRule) {
        let mut p = Path2D::new();
        p.add_path(path, self.state.transform);
        self.clip_path_device(&p, rule);
    }
    fn clip_path_device(&mut self, path: &Path2D, rule: FillRule) {
        let mask = self.path_mask(path, rule, false);
        for (old, new) in self.state.clip.iter_mut().zip(mask) {
            *old *= new;
        }
    }
    pub fn fill_rect(&mut self, x: f64, y: f64, w: f64, h: f64) {
        let mut p = Path2D::new();
        p.rect(x, y, w, h);
        self.fill_path(&p, FillRule::NonZero);
    }
    pub fn stroke_rect(&mut self, x: f64, y: f64, w: f64, h: f64) {
        let mut p = Path2D::new();
        p.rect(x, y, w, h);
        self.stroke_path(&p);
    }
    pub fn clear_rect(&mut self, x: f64, y: f64, w: f64, h: f64) {
        let mut p = Path2D::new();
        p.rect(x, y, w, h);
        let mut q = Path2D::new();
        q.add_path(&p, self.state.transform);
        let mask = self.path_mask(&q, FillRule::NonZero, false);
        for (i, coverage) in mask.into_iter().enumerate() {
            let a = (1. - coverage as f64 * self.state.clip[i] as f64).clamp(0., 1.);
            self.surface.pixels[i * 4 + 3] =
                (self.surface.pixels[i * 4 + 3] as f64 * a).round() as u8;
            if self.surface.pixels[i * 4 + 3] == 0 {
                self.surface.pixels[i * 4..i * 4 + 3].fill(0);
            }
        }
    }
    pub fn is_point_in_path(&self, x: f64, y: f64, rule: FillRule) -> bool {
        point_in_fill(&self.path, Point::new(x, y), rule)
    }
    pub fn is_point_in_stroke(&self, x: f64, y: f64) -> bool {
        point_on_stroke(&self.path, Point::new(x, y), &self.state)
    }
    pub fn is_point_in_path2d(&self, path: &Path2D, x: f64, y: f64, rule: FillRule) -> bool {
        let mut device = Path2D::new();
        device.add_path(path, self.state.transform);
        point_in_fill(&device, Point::new(x, y), rule)
    }
    pub fn is_point_in_stroke2d(&self, path: &Path2D, x: f64, y: f64) -> bool {
        let mut device = Path2D::new();
        device.add_path(path, self.state.transform);
        point_on_stroke(&device, Point::new(x, y), &self.state)
    }
    fn path_mask(&self, path: &Path2D, rule: FillRule, stroke: bool) -> Vec<f32> {
        let width = self.surface.width as usize;
        let height = self.surface.height as usize;
        let mut mask = vec![0.; width * height];
        for y in 0..height {
            for x in 0..width {
                let mut covered = 0;
                for sy in 0..4 {
                    for sx in 0..4 {
                        let p = Point::new(
                            x as f64 + (sx as f64 + 0.5) / 4.,
                            y as f64 + (sy as f64 + 0.5) / 4.,
                        );
                        if if stroke {
                            point_on_stroke(path, p, &self.state)
                        } else {
                            point_in_fill(path, p, rule)
                        } {
                            covered += 1;
                        }
                    }
                }
                mask[y * width + x] = covered as f32 / 16.;
            }
        }
        mask
    }
    fn draw_path(&mut self, path: &Path2D, rule: FillRule, stroke: bool, clear: bool) {
        let mask = self.path_mask(path, rule, stroke);
        if clear {
            return;
        }
        self.draw_shadow(&mask);
        let style = if stroke {
            self.state.stroke_style.clone()
        } else {
            self.state.fill_style.clone()
        };
        let inverse = self
            .state
            .transform
            .inverse()
            .unwrap_or(Transform::IDENTITY);
        for (i, coverage) in mask.into_iter().enumerate() {
            let coverage = coverage as f64 * self.state.clip[i] as f64;
            if self.state.clip[i] <= 0. {
                continue;
            }
            if coverage <= 0. {
                if clears_empty_source(self.state.composite_operation) {
                    self.blend_at(i, [0; 4]);
                }
                continue;
            }
            let x = i % self.surface.width as usize;
            let y = i / self.surface.width as usize;
            let p = inverse.map(Point::new(x as f64 + 0.5, y as f64 + 0.5));
            let mut color = style.sample(p);
            color[3] = (color[3] as f64 * coverage * self.state.global_alpha)
                .round()
                .clamp(0., 255.) as u8;
            self.blend_at(i, color);
        }
    }
    fn draw_shadow(&mut self, mask: &[f32]) {
        if self.state.shadow_color[3] == 0 {
            return;
        }
        let w = self.surface.width as i32;
        let h = self.surface.height as i32;
        let radius = self.state.shadow_blur.round().clamp(0., 32.) as i32;
        let dx = self.state.shadow_offset_x.round() as i32;
        let dy = self.state.shadow_offset_y.round() as i32;
        let mut blurred = vec![0.; mask.len()];
        for y in 0..h {
            for x in 0..w {
                let mut sum = 0.;
                let mut count = 0;
                for yy in -radius..=radius {
                    for xx in -radius..=radius {
                        let sx = x - dx + xx;
                        let sy = y - dy + yy;
                        if sx >= 0 && sy >= 0 && sx < w && sy < h {
                            sum += mask[(sy * w + sx) as usize];
                            count += 1;
                        }
                    }
                }
                if count > 0 {
                    blurred[(y * w + x) as usize] = sum / count as f32;
                }
            }
        }
        for (i, coverage) in blurred.into_iter().enumerate() {
            let alpha = coverage as f64 * self.state.clip[i] as f64 * self.state.global_alpha;
            if alpha <= 0. {
                continue;
            }
            let mut color = self.state.shadow_color;
            color[3] = (color[3] as f64 * alpha).round().clamp(0., 255.) as u8;
            self.blend_at(i, color);
        }
    }
    pub(crate) fn blend_at(&mut self, index: usize, color: [u8; 4]) {
        let at = index * 4;
        let dst = self.surface.pixels[at..at + 4].try_into().unwrap();
        self.surface.pixels[at..at + 4].copy_from_slice(&composite(
            dst,
            color,
            self.state.composite_operation,
        ));
    }
    pub fn draw_image(&mut self, image: &Bitmap, dx: f64, dy: f64) {
        self.draw_image_full(
            image,
            0.,
            0.,
            image.width as f64,
            image.height as f64,
            dx,
            dy,
            image.width as f64,
            image.height as f64,
        );
    }
    pub fn draw_image_scaled(&mut self, image: &Bitmap, dx: f64, dy: f64, dw: f64, dh: f64) {
        self.draw_image_full(
            image,
            0.,
            0.,
            image.width as f64,
            image.height as f64,
            dx,
            dy,
            dw,
            dh,
        );
    }
    #[allow(clippy::too_many_arguments)]
    pub fn draw_image_full(
        &mut self,
        image: &Bitmap,
        sx: f64,
        sy: f64,
        sw: f64,
        sh: f64,
        dx: f64,
        dy: f64,
        dw: f64,
        dh: f64,
    ) {
        if ![sx, sy, sw, sh, dx, dy, dw, dh]
            .iter()
            .all(|v| v.is_finite())
            || sw == 0.
            || sh == 0.
            || dw == 0.
            || dh == 0.
        {
            return;
        }
        let Some(inverse) = self.state.transform.inverse() else {
            return;
        };
        let (sx, sw) = if sw < 0. { (sx + sw, -sw) } else { (sx, sw) };
        let (sy, sh) = if sh < 0. { (sy + sh, -sh) } else { (sy, sh) };
        let (dx, dw) = if dw < 0. { (dx + dw, -dw) } else { (dx, dw) };
        let (dy, dh) = if dh < 0. { (dy + dh, -dh) } else { (dy, dh) };
        if self.state.shadow_color[3] > 0 {
            let mut mask = vec![0.; self.surface.width as usize * self.surface.height as usize];
            for y in 0..self.surface.height {
                for x in 0..self.surface.width {
                    let p = inverse.map(Point::new(x as f64 + 0.5, y as f64 + 0.5));
                    let u = (p.x - dx) / dw;
                    let v = (p.y - dy) / dh;
                    if (0. ..1.).contains(&u) && (0. ..1.).contains(&v) {
                        mask[(y * self.surface.width + x) as usize] = image.sample(
                            sx + u * sw,
                            sy + v * sh,
                            self.state.image_smoothing_enabled,
                        )[3]
                            as f32
                            / 255.;
                    }
                }
            }
            self.draw_shadow(&mask);
        }
        for y in 0..self.surface.height {
            for x in 0..self.surface.width {
                let index = (y * self.surface.width + x) as usize;
                if self.state.clip[index] <= 0. {
                    continue;
                }
                let p = inverse.map(Point::new(x as f64 + 0.5, y as f64 + 0.5));
                let u = (p.x - dx) / dw;
                let v = (p.y - dy) / dh;
                if !(0. ..1.).contains(&u) || !(0. ..1.).contains(&v) {
                    if clears_empty_source(self.state.composite_operation) {
                        self.blend_at(index, [0; 4]);
                    }
                    continue;
                }
                let ix = sx + u * sw;
                let iy = sy + v * sh;
                let mut color = image.sample(ix, iy, self.state.image_smoothing_enabled);
                color[3] =
                    (color[3] as f64 * self.state.global_alpha * self.state.clip[index] as f64)
                        .round()
                        .clamp(0., 255.) as u8;
                if color[3] > 0 || clears_empty_source(self.state.composite_operation) {
                    self.blend_at(index, color);
                }
            }
        }
    }
    pub fn create_image_data(width: u32, height: u32) -> Result<ImageData, ImageError> {
        ImageData::new(width, height)
    }
    pub fn get_image_data_signed(
        &self,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    ) -> Result<ImageData, ImageError> {
        if width == 0 || height == 0 {
            return Err(ImageError::InvalidDimensions);
        }
        let left = if width < 0 {
            x as i64 + width as i64
        } else {
            x as i64
        };
        let top = if height < 0 {
            y as i64 + height as i64
        } else {
            y as i64
        };
        if left < i32::MIN as i64
            || left > i32::MAX as i64
            || top < i32::MIN as i64
            || top > i32::MAX as i64
        {
            return Err(ImageError::InvalidDimensions);
        }
        self.get_image_data(
            left as i32,
            top as i32,
            width.unsigned_abs(),
            height.unsigned_abs(),
        )
    }
    pub fn get_image_data(
        &self,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
    ) -> Result<ImageData, ImageError> {
        let mut data = ImageData::new(width, height)?;
        for row in 0..height {
            for col in 0..width {
                let px = x as i64 + col as i64;
                let py = y as i64 + row as i64;
                if px >= 0
                    && py >= 0
                    && px < self.surface.width as i64
                    && py < self.surface.height as i64
                {
                    let at = ((row * width + col) * 4) as usize;
                    data.data[at..at + 4].copy_from_slice(
                        &self.surface.pixels
                            [((py as u32 * self.surface.width + px as u32) * 4) as usize..][..4],
                    );
                }
            }
        }
        Ok(data)
    }
    pub fn put_image_data(&mut self, data: &ImageData, x: i32, y: i32) {
        self.put_image_data_dirty(data, x, y, 0, 0, data.width, data.height);
    }
    pub fn put_image_data_dirty(
        &mut self,
        data: &ImageData,
        x: i32,
        y: i32,
        dirty_x: i32,
        dirty_y: i32,
        dirty_w: u32,
        dirty_h: u32,
    ) {
        for row in 0..data.height {
            for col in 0..data.width {
                if (col as i64) < dirty_x as i64
                    || (row as i64) < dirty_y as i64
                    || col as i64 >= dirty_x as i64 + dirty_w as i64
                    || row as i64 >= dirty_y as i64 + dirty_h as i64
                {
                    continue;
                }
                let px = x as i64 + col as i64;
                let py = y as i64 + row as i64;
                if px >= 0
                    && py >= 0
                    && px < self.surface.width as i64
                    && py < self.surface.height as i64
                {
                    let source = ((row * data.width + col) * 4) as usize;
                    let target = ((py as u32 * self.surface.width + px as u32) * 4) as usize;
                    if data.data[source + 3] == 0 {
                        self.surface.pixels[target..target + 4].fill(0);
                    } else {
                        self.surface.pixels[target..target + 4]
                            .copy_from_slice(&data.data[source..source + 4]);
                    }
                }
            }
        }
    }
    pub fn put_image_data_dirty_signed(
        &mut self,
        data: &ImageData,
        x: i32,
        y: i32,
        dirty_x: i32,
        dirty_y: i32,
        dirty_w: i32,
        dirty_h: i32,
    ) {
        if dirty_w == 0 || dirty_h == 0 {
            return;
        }
        let left = if dirty_w < 0 {
            dirty_x as i64 + dirty_w as i64
        } else {
            dirty_x as i64
        };
        let top = if dirty_h < 0 {
            dirty_y as i64 + dirty_h as i64
        } else {
            dirty_y as i64
        };
        let right = left + (dirty_w as i64).abs();
        let bottom = top + (dirty_h as i64).abs();
        for row in 0..data.height {
            for col in 0..data.width {
                if (col as i64) < left
                    || (col as i64) >= right
                    || (row as i64) < top
                    || (row as i64) >= bottom
                {
                    continue;
                }
                let px = x as i64 + col as i64;
                let py = y as i64 + row as i64;
                if px < 0
                    || py < 0
                    || px >= self.surface.width as i64
                    || py >= self.surface.height as i64
                {
                    continue;
                }
                let source = ((row * data.width + col) * 4) as usize;
                let target = ((py as u32 * self.surface.width + px as u32) * 4) as usize;
                if data.data[source + 3] == 0 {
                    self.surface.pixels[target..target + 4].fill(0)
                } else {
                    self.surface.pixels[target..target + 4]
                        .copy_from_slice(&data.data[source..source + 4]);
                }
            }
        }
    }
}

fn clears_empty_source(mode: CompositeOperation) -> bool {
    matches!(
        mode,
        CompositeOperation::SourceIn
            | CompositeOperation::SourceOut
            | CompositeOperation::DestinationIn
            | CompositeOperation::DestinationAtop
            | CompositeOperation::Copy
    )
}

fn point_in_fill(path: &Path2D, p: Point, rule: FillRule) -> bool {
    let mut winding = 0i32;
    let mut crossings = 0i32;
    for contour in &path.contours {
        let n = contour.points.len();
        if n < 2 {
            continue;
        }
        for i in 0..n {
            let a = contour.points[i];
            let b = contour.points[(i + 1) % n];
            if (a.y <= p.y && b.y > p.y) || (b.y <= p.y && a.y > p.y) {
                let x = a.x + (p.y - a.y) * (b.x - a.x) / (b.y - a.y);
                if x > p.x {
                    crossings += 1;
                    winding += if b.y > a.y { 1 } else { -1 };
                }
            }
        }
    }
    match rule {
        FillRule::EvenOdd => crossings % 2 != 0,
        FillRule::NonZero => winding != 0,
    }
}

fn point_on_stroke(path: &Path2D, p: Point, state: &CanvasState) -> bool {
    let scale = (state.transform.a * state.transform.d - state.transform.b * state.transform.c)
        .abs()
        .sqrt();
    let half = state.line_width * scale / 2.;
    if half <= 0. {
        return false;
    }
    for contour in &path.contours {
        let points = &contour.points;
        let n = points.len();
        if n < 2 {
            continue;
        }
        let count = if contour.closed { n } else { n - 1 };
        let mut cumulative = 0.;
        for i in 0..count {
            let a = points[i];
            let b = points[(i + 1) % n];
            let dx = b.x - a.x;
            let dy = b.y - a.y;
            let length = dx.hypot(dy);
            if length < 1e-9 {
                continue;
            }
            let t = ((p.x - a.x) * dx + (p.y - a.y) * dy) / (length * length);
            let projection = t.clamp(0., 1.);
            let perpendicular = ((p.x - a.x) * dy - (p.y - a.y) * dx).abs() / length;
            if t >= 0.
                && t <= 1.
                && perpendicular <= half
                && dash_on(cumulative + projection * length, state)
            {
                return true;
            }
            if !contour.closed && state.line_cap != LineCap::Butt {
                let start_cap = i == 0 && t < 0. && t >= -half / length;
                let end_cap = i == count - 1 && t > 1. && t <= 1. + half / length;
                let shape = if state.line_cap == LineCap::Square {
                    perpendicular <= half
                } else if start_cap {
                    (p.x - a.x).hypot(p.y - a.y) <= half
                } else {
                    (p.x - b.x).hypot(p.y - b.y) <= half
                };
                if (start_cap || end_cap)
                    && shape
                    && dash_on(cumulative + projection * length, state)
                {
                    return true;
                }
            }
            cumulative += length;
        }
        for i in if contour.closed { 0..n } else { 1..n - 1 } {
            let previous = points[(i + n - 1) % n];
            let vertex = points[i];
            let next = points[(i + 1) % n];
            let u = Point::new(vertex.x - previous.x, vertex.y - previous.y);
            let v = Point::new(next.x - vertex.x, next.y - vertex.y);
            let lu = u.x.hypot(u.y);
            let lv = v.x.hypot(v.y);
            if lu < 1e-9 || lv < 1e-9 {
                continue;
            }
            let u = Point::new(u.x / lu, u.y / lu);
            let v = Point::new(v.x / lv, v.y / lv);
            let cross = u.x * v.y - u.y * v.x;
            if cross.abs() < 1e-9 {
                continue;
            }
            if state.line_join == LineJoin::Round {
                if (p.x - vertex.x).hypot(p.y - vertex.y) <= half {
                    return true;
                }
                continue;
            }
            let sign = if cross > 0. { 1. } else { -1. };
            let a = Point::new(vertex.x + sign * u.y * half, vertex.y - sign * u.x * half);
            let b = Point::new(vertex.x + sign * v.y * half, vertex.y - sign * v.x * half);
            if in_triangle(p, vertex, a, b) {
                return true;
            }
            if state.line_join == LineJoin::Miter {
                let t = ((b.x - a.x) * v.y - (b.y - a.y) * v.x) / cross;
                let m = Point::new(a.x + u.x * t, a.y + u.y * t);
                if (m.x - vertex.x).hypot(m.y - vertex.y) <= state.miter_limit * half
                    && in_triangle(p, a, m, b)
                {
                    return true;
                }
            }
        }
    }
    false
}

fn in_triangle(p: Point, a: Point, b: Point, c: Point) -> bool {
    let cross =
        |u: Point, v: Point, w: Point| (v.x - u.x) * (w.y - u.y) - (v.y - u.y) * (w.x - u.x);
    let x = cross(a, b, p);
    let y = cross(b, c, p);
    let z = cross(c, a, p);
    (x >= -1e-9 && y >= -1e-9 && z >= -1e-9) || (x <= 1e-9 && y <= 1e-9 && z <= 1e-9)
}

fn dash_on(distance: f64, state: &CanvasState) -> bool {
    let total = state.line_dash.iter().sum::<f64>();
    if total <= 0. {
        return true;
    }
    let mut d = (distance + state.line_dash_offset).rem_euclid(total);
    for (i, length) in state.line_dash.iter().enumerate() {
        if d < *length {
            return i % 2 == 0;
        }
        d -= length;
    }
    true
}
