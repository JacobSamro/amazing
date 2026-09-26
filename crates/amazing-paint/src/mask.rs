//! Bounded painter reference for CSS `mask-image`/`mask-mode` (the `mask-*`
//! longhands COMP-15 still needed after `clip-path`).
//!
//! Only a single `mask-image: url(...)` layer is supported: the first
//! `url()` reference in the (possibly comma-separated) value. Gradients,
//! `image()`/`image-set()`/`element()`/`cross-fade()`/`paint()` references,
//! more than one layer, and `mask-composite` are skipped rather than
//! guessed — the element paints unmasked, exactly as an unresolved
//! `mask-image` reference does per the CSS Masking specification. The
//! referenced image is stretched to exactly fill the masked box's own
//! border box (equivalent to `mask-size: 100% 100%`, `mask-position: 0 0`,
//! `mask-repeat: no-repeat`); `mask-size`/`mask-position`/`mask-repeat`/
//! `mask-clip`/`mask-origin` are not read, a documented simplification like
//! this module's `clip-path` resolver using the border box regardless of an
//! explicit `<geometry-box>`.
//!
//! The layer is `Copy` with a fixed URL capacity so it can live inside the
//! copyable paint command and effect-node records, matching
//! [`crate::filter::FilterPlan`] and [`crate::clip::ClipMask`].

use amazing_css::effects::MaskMode;
use amazing_css::syntax::{Token, tokenize};

/// The URL byte cap for a `mask-image` reference (see the module doc).
pub const MAX_MASK_URL_BYTES: usize = 256;

/// A resolved single-layer `mask-image: url(...)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MaskLayer {
    url: [u8; MAX_MASK_URL_BYTES],
    url_len: u16,
    /// `mask-mode: alpha`; otherwise (`luminance`, or `match-source`'s
    /// default for a raster image) the pixel's color contributes too.
    pub alpha_mode: bool,
}

impl MaskLayer {
    pub fn url(&self) -> &str {
        std::str::from_utf8(&self.url[..self.url_len as usize]).unwrap_or("")
    }

    /// `mask_image`/`mask_mode` are the element's computed `mask-image`/
    /// `mask-mode` longhand values (`"none"`/`"match-source"` when unset).
    pub fn from_css(mask_image: &str, mask_mode: &str) -> Option<Self> {
        if amazing_css::effects::parse_mask_image(mask_image)?.is_empty() {
            return None; // `none`.
        }
        let url = first_url(mask_image)?;
        if url.len() > MAX_MASK_URL_BYTES {
            return None;
        }
        let alpha_mode = matches!(amazing_css::effects::parse_mask_mode(mask_mode), Some(MaskMode::Alpha));
        let mut bytes = [0u8; MAX_MASK_URL_BYTES];
        bytes[..url.len()].copy_from_slice(url.as_bytes());
        Some(Self { url: bytes, url_len: url.len() as u16, alpha_mode })
    }

    /// The mask value (`0.0` fully masked out .. `1.0` fully shown) at
    /// `(u, v)` in the mask image's own normalized `0..1` space,
    /// nearest-neighbor sampled. Outside `0..1` on either axis (past the
    /// stretched extent) is fully masked out, matching `no-repeat`.
    pub fn sample(&self, image: &amazing_media::Image, u: f32, v: f32) -> f32 {
        if image.width == 0
            || image.height == 0
            || !(0.0..1.0).contains(&u)
            || !(0.0..1.0).contains(&v)
        {
            return 0.0;
        }
        let ix = ((u * image.width as f32) as u32).min(image.width - 1);
        let iy = ((v * image.height as f32) as u32).min(image.height - 1);
        let at = (iy as usize * image.width as usize + ix as usize) * 4;
        let Some(pixel) = image.pixels.get(at..at + 4) else { return 0.0 };
        let alpha = pixel[3] as f32 / 255.0;
        if self.alpha_mode {
            alpha
        } else {
            // The same Filter Effects luma coefficients this painter already
            // uses for `grayscale()` (crate::filter), weighted by the
            // pixel's own alpha so a transparent "white" pixel masks out.
            let luma = 0.2126 * pixel[0] as f32 + 0.7152 * pixel[1] as f32 + 0.0722 * pixel[2] as f32;
            (luma / 255.0) * alpha
        }
    }
}

/// The first top-level `url(...)` reference's decoded value in a (possibly
/// comma-separated) `<image>#` list, or `None` if the first component is
/// not a plain `url()` reference. An unquoted `url(foo.png)` tokenizes as
/// one URL token; a quoted `url("foo.png")` tokenizes as a function call
/// (`Function("url")`, `OpenParen`, `Str`, `CloseParen`) per this
/// tokenizer's documented simplification — both forms are accepted.
fn first_url(value: &str) -> Option<String> {
    let first_component = value.split(',').next()?.trim();
    let tokens: Vec<Token> = tokenize(first_component)
        .into_iter()
        .filter(|token| *token != Token::Whitespace)
        .collect();
    match tokens.as_slice() {
        [Token::Url { value, .. }, ..] if !value.is_empty() => Some(value.clone()),
        [Token::Function(name), Token::OpenParen, Token::Str(value), Token::CloseParen, ..]
            if name.eq_ignore_ascii_case("url") && !value.is_empty() =>
        {
            Some(value.clone())
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::MaskLayer;
    use amazing_media::Image;

    fn solid(color: [u8; 4]) -> Image {
        Image { width: 1, height: 1, pixels: color.to_vec() }
    }

    #[test]
    fn none_and_a_gradient_are_skipped_not_guessed() {
        assert!(MaskLayer::from_css("none", "match-source").is_none());
        assert!(MaskLayer::from_css("linear-gradient(red, blue)", "match-source").is_none());
    }

    #[test]
    fn url_layer_resolves_and_defaults_to_luminance() {
        let layer = MaskLayer::from_css("url(\"mask.png\")", "match-source").expect("valid mask-image");
        assert_eq!(layer.url(), "mask.png");
        assert!(!layer.alpha_mode);
    }

    #[test]
    fn a_second_url_in_a_layer_list_is_ignored_only_the_first_is_used() {
        let layer = MaskLayer::from_css("url(a.png), url(b.png)", "match-source").expect("valid mask-image");
        assert_eq!(layer.url(), "a.png");
    }

    #[test]
    fn mask_mode_alpha_uses_the_pixels_own_alpha_channel() {
        let layer = MaskLayer::from_css("url(a.png)", "alpha").expect("valid mask-image");
        // Opaque black: alpha-mode reads only the alpha channel (1.0), while
        // luminance-mode would read near 0 (black has ~0 luma).
        let image = solid([0, 0, 0, 255]);
        assert_eq!(layer.sample(&image, 0.5, 0.5), 1.0);
    }

    #[test]
    fn mask_mode_luminance_weighs_color_by_its_own_alpha() {
        let layer = MaskLayer::from_css("url(a.png)", "luminance").expect("valid mask-image");
        let white_opaque = solid([255, 255, 255, 255]);
        assert!((layer.sample(&white_opaque, 0.5, 0.5) - 1.0).abs() < 0.01);
        let white_half_transparent = solid([255, 255, 255, 128]);
        assert!((layer.sample(&white_half_transparent, 0.5, 0.5) - 0.5).abs() < 0.01);
        let black_opaque = solid([0, 0, 0, 255]);
        assert!(layer.sample(&black_opaque, 0.5, 0.5) < 0.01);
    }

    #[test]
    fn sampling_outside_the_stretched_extent_is_fully_masked_out() {
        let layer = MaskLayer::from_css("url(a.png)", "alpha").expect("valid mask-image");
        let image = solid([255, 255, 255, 255]);
        assert_eq!(layer.sample(&image, 1.5, 0.5), 0.0);
        assert_eq!(layer.sample(&image, 0.5, -0.1), 0.0);
    }
}
