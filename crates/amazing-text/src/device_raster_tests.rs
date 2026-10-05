use super::*;
fn key() -> GlyphCacheKey {
    GlyphCacheKey::new(0, FontKey(0), GlyphId('A' as u32), 20.0, 0, 0).unwrap()
}
fn draw(scale: f64, x: u8, y: u8) -> Result<Option<GlyphImage>, TextError> {
    FallbackTextSystem.rasterize_at_scale(
        GlyphRasterRequest {
            key: key(),
            variations: &[],
            synthesis: FontSynthesis::default(),
        },
        GlyphRasterScale::new(scale, x, y).unwrap(),
    )
}
#[test]
fn device_raster_identity_validates_scale_phase_and_resource_limits() {
    for value in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert!(GlyphRasterScale::new(value, 0, 0).is_none());
    }
    assert!(GlyphRasterScale::new(1.0, 64, 0).is_none());
    assert_ne!(
        GlyphRasterScale::new(1.25, 0, 0),
        GlyphRasterScale::new(1.5, 0, 0)
    );
    assert_ne!(
        GlyphRasterScale::new(2.0, 0, 0),
        GlyphRasterScale::new(2.0, 1, 0)
    );
    assert_eq!(draw(1000.0, 0, 0), Err(TextError::ResourceLimit));
}
#[test]
fn fallback_raster_is_native_device_coverage_and_unit_compatible() {
    let legacy = FallbackTextSystem
        .rasterize(GlyphRasterRequest {
            key: key(),
            variations: &[],
            synthesis: FontSynthesis::default(),
        })
        .unwrap();
    assert_eq!(legacy, draw(1.0, 0, 0).unwrap());
    let unit = legacy.unwrap();
    for scale in [1.25, 1.5, 2.0, 3.0] {
        let image = draw(scale, 0, 0).unwrap().unwrap();
        assert!(image.width > unit.width && image.height > unit.height);
        assert_eq!(
            image.data.len(),
            image.width as usize * image.height as usize
        );
        assert_eq!(key().size_64, 20 * 64);
    }
    let shifted = draw(2.0, 16, 32).unwrap().unwrap();
    assert!(shifted.data.iter().any(|a| *a > 0 && *a < 255));
    assert_ne!(shifted, draw(2.0, 0, 0).unwrap().unwrap());
}
