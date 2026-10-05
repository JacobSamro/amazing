use super::*;
use amazing_text::{FontSynthesis, GlyphId, GlyphRasterScale};
fn request(key: GlyphCacheKey) -> GlyphRasterRequest<'static> {
    GlyphRasterRequest {
        key,
        variations: &[],
        synthesis: FontSynthesis::default(),
    }
}
#[test]
fn device_cache_separates_density_phase_and_generation() {
    let provider = TextProvider::new();
    let key = GlyphCacheKey::new(
        provider.generation(),
        FontKey(0),
        GlyphId('A' as u32),
        20.0,
        0,
        0,
    )
    .unwrap();
    let unit = provider.rasterize(request(key)).unwrap();
    assert_eq!(
        unit,
        provider
            .rasterize_at_scale(request(key), GlyphRasterScale::unscaled(key))
            .unwrap()
    );
    for scale in [1.25, 1.5, 2.0, 3.0] {
        for phase in [0, 16, 32, 63] {
            let raster = GlyphRasterScale::new(scale, phase, phase).unwrap();
            let first = provider.rasterize_at_scale(request(key), raster).unwrap();
            assert_eq!(
                first,
                provider.rasterize_at_scale(request(key), raster).unwrap()
            );
            assert!(provider.raster_cache.borrow().contains_key(&(key, raster)));
        }
    }
    assert_eq!(provider.raster_cache.borrow().len(), 17);
    assert_eq!(key.size_64, 20 * 64);
    let wrong = GlyphCacheKey {
        generation: provider.generation() + 1,
        ..key
    };
    assert_eq!(
        provider.rasterize_at_scale(request(wrong), GlyphRasterScale::new(2.0, 0, 0).unwrap()),
        Err(TextError::MissingFont)
    );
}
#[test]
fn genuine_outline_device_ppem_and_phase_use_original_css_font_identity() {
    let mut provider = TextProvider::new();
    let font = provider
        .register_web_font(WebFont {
            family: "Device Test".into(),
            weight: 400,
            stretch: 100,
            slant: FontSlant::Normal,
            format: FontFormat::OpenType,
            bytes: super::tests::synthetic_font('A'),
        })
        .unwrap();
    let key = GlyphCacheKey::new(provider.generation(), font, GlyphId(1), 20.0, 0, 0).unwrap();
    let unit = provider.rasterize(request(key)).unwrap().unwrap();
    for scale in [1.25, 1.5, 2.0, 3.0] {
        let raster = GlyphRasterScale::new(scale, 16, 32).unwrap();
        let image = provider
            .rasterize_at_scale(request(key), raster)
            .unwrap()
            .unwrap();
        assert!(image.width >= unit.width && image.height >= unit.height);
        assert_eq!(
            image.data.len(),
            image.width as usize * image.height as usize
        );
        assert_eq!(
            provider.rasterize_at_scale(request(key), raster).unwrap(),
            Some(image)
        );
    }
    assert_eq!(key.size(), 20.0);
    assert_eq!(
        provider.rasterize_at_scale(request(key), GlyphRasterScale::new(1000.0, 0, 0).unwrap()),
        Err(TextError::ResourceLimit)
    );
}
