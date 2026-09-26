//! CSS Values 4 viewport lengths at the computed-value boundary.
use crate::media_queries::MediaEnvironment;
use crate::syntax::{Token, tokenize};

pub(crate) fn is_viewport_unit(unit: &str) -> bool {
    matches!(unit.to_ascii_lowercase().as_str(),
        "vw" | "vh" | "vmin" | "vmax" |
        "svw" | "svh" | "svmin" | "svmax" |
        "lvw" | "lvh" | "lvmin" | "lvmax" |
        "dvw" | "dvh" | "dvmin" | "dvmax")
}

pub(crate) fn uses_viewport(value: &str) -> bool {
    tokenize(value).iter().any(|token| {
        matches!(token, Token::Dimension(_, unit) if is_viewport_unit(unit))
    })
}

pub(crate) fn resolve(tokens: &[Token], environment: &MediaEnvironment) -> Option<Vec<Token>> {
    tokens.iter().map(|token| match token {
        Token::Dimension(value, unit) if is_viewport_unit(unit) => {
            let px = value * environment.viewport_unit_px(unit)?;
            px.is_finite().then(|| Token::Dimension(px, "px".to_owned()))
        }
        _ => Some(token.clone()),
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax::serialize;

    #[test]
    fn distinct_viewport_variants_and_math_are_resolved() {
        let env = MediaEnvironment { viewport_width_px: 800.0, viewport_height_px: 600.0,
            small_viewport_px: Some((700.0, 500.0)), large_viewport_px: Some((900.0, 700.0)),
            dynamic_viewport_px: Some((750.0, 550.0)), ..Default::default() };
        for (specified, expected) in [
            ("100vw", "800px"), ("100vh", "600px"),
            ("100vmin", "600px"), ("100vmax", "800px"),
            ("100svw", "700px"), ("100svh", "500px"),
            ("100lvmin", "700px"), ("100lvmax", "900px"),
            ("100dvw", "750px"), ("100dvh", "550px"),
            ("calc(50vw + 10svh)", "calc(400px + 50px)"),
        ] {
            assert_eq!(serialize(&resolve(&tokenize(specified), &env).unwrap()), expected);
        }
    }
}
