use crate::{Bitmap, Canvas2D, PaintStyle, Point};
use amazing_text::{
    Direction, GlyphCacheKey, GlyphContent, GlyphRasterRequest, Paragraph, ShapedParagraph,
    ShapingBoundary, TextError, TextSpan, TextSystem,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextAlign {
    #[default]
    Start,
    End,
    Left,
    Right,
    Center,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextBaseline {
    Top,
    Hanging,
    Middle,
    #[default]
    Alphabetic,
    Ideographic,
    Bottom,
}

fn alignment_shift(align: TextAlign, direction: Direction, width: f64) -> f64 {
    match align {
        TextAlign::Left => 0.,
        TextAlign::Right => -width,
        TextAlign::Center => -width / 2.,
        TextAlign::Start if direction == Direction::RightToLeft => -width,
        TextAlign::End if direction == Direction::LeftToRight => -width,
        _ => 0.,
    }
}

fn baseline_shift(baseline: TextBaseline, ascent: f64, descent: f64) -> f64 {
    match baseline {
        TextBaseline::Top => ascent,
        TextBaseline::Hanging => ascent * 0.8,
        TextBaseline::Middle => (ascent - descent) / 2.,
        TextBaseline::Alphabetic => 0.,
        TextBaseline::Ideographic => -descent * 0.5,
        TextBaseline::Bottom => -descent,
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextMetrics {
    pub width: f64,
    pub actual_bounding_box_left: f64,
    pub actual_bounding_box_right: f64,
    pub actual_bounding_box_ascent: f64,
    pub actual_bounding_box_descent: f64,
    pub font_bounding_box_ascent: f64,
    pub font_bounding_box_descent: f64,
}

fn shape(
    text: &str,
    style: &amazing_text::TextStyle,
    system: &impl TextSystem,
) -> Result<ShapedParagraph, TextError> {
    let span = TextSpan {
        range: 0..text.len(),
        source: 0,
        style: style.clone(),
        boundary_before: ShapingBoundary::Continue,
    };
    system.shape(Paragraph {
        text,
        spans: &[span],
        sources: &[],
        bidi_scopes: &[],
        base_direction: style.direction,
    })
}

impl Canvas2D {
    pub fn measure_text(
        &self,
        text: &str,
        system: &impl TextSystem,
    ) -> Result<TextMetrics, TextError> {
        let shaped = shape(text, &self.state.text_style, system)?;
        let width = shaped
            .runs
            .iter()
            .flat_map(|run| &run.glyphs)
            .map(|g| g.advance_x.abs() as f64)
            .sum::<f64>();
        let metrics = system.metrics(&self.state.text_style)?;
        let shift = alignment_shift(
            self.state.text_align,
            self.state.text_style.direction,
            width,
        );
        Ok(TextMetrics {
            width,
            actual_bounding_box_left: -shift,
            actual_bounding_box_right: width + shift,
            actual_bounding_box_ascent: metrics.ascent as f64,
            actual_bounding_box_descent: metrics.descent as f64,
            font_bounding_box_ascent: metrics.ascent as f64,
            font_bounding_box_descent: metrics.descent as f64,
        })
    }
    pub fn fill_text(
        &mut self,
        text: &str,
        x: f64,
        y: f64,
        max_width: Option<f64>,
        system: &impl TextSystem,
    ) -> Result<(), TextError> {
        self.draw_text(text, x, y, max_width, system, false)
    }
    pub fn stroke_text(
        &mut self,
        text: &str,
        x: f64,
        y: f64,
        max_width: Option<f64>,
        system: &impl TextSystem,
    ) -> Result<(), TextError> {
        self.draw_text(text, x, y, max_width, system, true)
    }
    fn draw_text(
        &mut self,
        text: &str,
        x: f64,
        y: f64,
        max_width: Option<f64>,
        system: &impl TextSystem,
        stroke: bool,
    ) -> Result<(), TextError> {
        if !x.is_finite() || !y.is_finite() {
            return Ok(());
        }
        let shaped = shape(text, &self.state.text_style, system)?;
        let natural = shaped
            .runs
            .iter()
            .flat_map(|run| &run.glyphs)
            .map(|g| g.advance_x.abs() as f64)
            .sum::<f64>();
        let factor = max_width
            .filter(|v| v.is_finite() && *v > 0. && *v < natural)
            .map_or(1., |v| v / natural);
        let mut cursor = x + alignment_shift(
            self.state.text_align,
            self.state.text_style.direction,
            natural * factor,
        );
        let metrics = system.metrics(&self.state.text_style)?;
        let baseline_y = y + baseline_shift(
            self.state.text_baseline,
            metrics.ascent as f64,
            metrics.descent as f64,
        );
        let style: PaintStyle = if stroke {
            self.state.stroke_style.clone()
        } else {
            self.state.fill_style.clone()
        };
        for run in &shaped.runs {
            for glyph in &run.glyphs {
                let key = GlyphCacheKey::new(
                    system.generation(),
                    run.font,
                    glyph.id,
                    run.font_size,
                    0,
                    0,
                )
                .ok_or(TextError::InvalidStyle)?;
                if let Some(image) = system.rasterize(GlyphRasterRequest {
                    key,
                    variations: &run.variations,
                    synthesis: run.synthesis,
                })? {
                    let margin = if stroke {
                        (self.state.line_width / 2.).ceil().clamp(1., 64.) as i32
                    } else {
                        0
                    };
                    let width = image.width + (margin as u32) * 2;
                    let height = image.height + (margin as u32) * 2;
                    let mut pixels = vec![0; width as usize * height as usize * 4];
                    let alpha_at = |gx: i32, gy: i32| -> u8 {
                        if gx < 0 || gy < 0 || gx >= image.width as i32 || gy >= image.height as i32
                        {
                            return 0;
                        }
                        let index = gy as usize * image.width as usize + gx as usize;
                        match image.content {
                            GlyphContent::Mask => image.data.get(index).copied().unwrap_or(0),
                            GlyphContent::Color => {
                                image.data.get(index * 4 + 3).copied().unwrap_or(0)
                            }
                        }
                    };
                    for oy in 0..height {
                        for ox in 0..width {
                            let gx = ox as i32 - margin;
                            let gy = oy as i32 - margin;
                            let coverage = if stroke {
                                let mut high = 0u8;
                                let mut low = 255u8;
                                for yy in -margin..=margin {
                                    for xx in -margin..=margin {
                                        if xx * xx + yy * yy > margin * margin {
                                            continue;
                                        }
                                        let alpha = alpha_at(gx + xx, gy + yy);
                                        high = high.max(alpha);
                                        low = low.min(alpha);
                                    }
                                }
                                high.saturating_sub(low)
                            } else {
                                alpha_at(gx, gy)
                            };
                            if coverage == 0 {
                                continue;
                            }
                            let local = Point::new(
                                cursor + (gx as f64 + image.bearing_x as f64) * factor,
                                baseline_y + gy as f64 - image.bearing_y as f64
                                    + glyph.offset_y as f64,
                            );
                            let mut color = if !stroke && image.content == GlyphContent::Color {
                                let index = gy as usize * image.width as usize + gx as usize;
                                image.data[index * 4..index * 4 + 4].try_into().unwrap()
                            } else {
                                style.sample(local)
                            };
                            if image.content != GlyphContent::Color || stroke {
                                color[3] = (color[3] as u16 * coverage as u16 / 255) as u8;
                            }
                            let target = (oy as usize * width as usize + ox as usize) * 4;
                            pixels[target..target + 4].copy_from_slice(&color);
                        }
                    }
                    let bitmap = Bitmap {
                        width,
                        height,
                        pixels,
                    };
                    self.draw_image_scaled(
                        &bitmap,
                        cursor + (image.bearing_x as f64 - margin as f64) * factor,
                        baseline_y - image.bearing_y as f64 - margin as f64 + glyph.offset_y as f64,
                        width as f64 * factor,
                        height as f64,
                    );
                }
                cursor += glyph.advance_x as f64 * factor;
            }
        }
        Ok(())
    }
}
