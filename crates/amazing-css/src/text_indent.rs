//! CSS Text indentation producer and line-eligibility policy.
//! Existing length, math and used-position evaluators own dimensional arithmetic.
use crate::{ComputedStyle, syntax::{Token, serialize, tokenize}, values::{ComputedValue, LengthPercentage}};

#[derive(Clone, Debug, PartialEq)]
pub struct TextIndent {
    pub length: ComputedValue,
    pub hanging: bool,
    pub each_line: bool,
    pub(crate) expression: String,
}

impl TextIndent {
    /// All three grammar components may occur in any order. Flags are unique;
    /// the length is mandatory and remains signed, including percentages.
    pub fn parse(css: &str) -> Option<Self> {
        // Comments disappear during tokenization without joining the tokens
        // on either side. Flags are complete top-level component values even
        // when no whitespace token separates them from a length/function.
        // Leave nested math untouched and let the existing block/length
        // validators reject malformed component values.
        let mut tokens = Vec::new();
        let mut depth = 0usize;
        for token in tokenize(css) {
            let flag = depth == 0 && matches!(&token, Token::Ident(name)
                if name.eq_ignore_ascii_case("hanging") || name.eq_ignore_ascii_case("each-line"));
            if flag { tokens.push(Token::Whitespace); }
            match &token {
                Token::OpenParen | Token::OpenSquare | Token::OpenCurly => depth += 1,
                Token::CloseParen | Token::CloseSquare | Token::CloseCurly => depth = depth.saturating_sub(1),
                _ => {},
            }
            tokens.push(token);
            if flag { tokens.push(Token::Whitespace); }
        }
        let parts = crate::computed::top_level_components(&tokens)?;
        let (mut hanging, mut each_line, mut expression) = (false, false, None);
        for part in parts {
            let text = serialize(&part);
            match text.to_ascii_lowercase().as_str() {
                "hanging" if !hanging => hanging = true,
                "each-line" if !each_line => each_line = true,
                "hanging" | "each-line" => return None,
                _ if expression.is_none() => expression = Some(text),
                _ => return None,
            }
        }
        let expression = expression?;
        let length = if let Some(length) = LengthPercentage::parse(&expression) {
            ComputedValue::LengthPercentage(length)
        } else if crate::math::length_percentage(&expression, true) {
            ComputedValue::LengthPercentageExpression(expression.clone())
        } else { return None };
        Some(Self { length, hanging, each_line, expression })
    }

    pub fn length_css(&self) -> &str { &self.expression }

    /// A zero primitive has no paragraph geometry effect, independent of basis.
    pub fn is_zero(&self) -> bool {
        matches!(self.length, ComputedValue::LengthPercentage(LengthPercentage::Px(n)
            | LengthPercentage::Percentage(n) | LengthPercentage::Relative(n, _)) if n == 0.0)
    }

    /// Font and root bases come from the installed computed style. Relative
    /// font metrics, viewport units and mixed box expressions stay symbolic.
    pub(crate) fn computed(css: &str, font: f64, root: f64) -> Option<String> {
        let mut value = Self::parse(css)?;
        crate::values::resolve_font_units(&mut value.length, font, root, false);
        value.expression = crate::computed::individual_transforms::length(&value.expression, font, root);
        Some(value.serialize())
    }

    pub fn serialize(&self) -> String {
        let mut result = self.expression.clone();
        if self.hanging { result.push_str(" hanging"); }
        if self.each_line { result.push_str(" each-line"); }
        result
    }

    /// Layout supplies authentic first-formatted-line and forced-break facts.
    /// A soft wrap is never an each-line break. Hanging reverses eligibility.
    pub fn applies_to_line(&self, first_formatted_line: bool, follows_forced_break: bool) -> bool {
        (first_formatted_line || self.each_line && follows_forced_break) != self.hanging
    }
}

/// Percentage basis is the block's own content inline-size, before indentation;
/// intrinsic sizing gives the percentage component a zero basis. The existing
/// used-position evaluator owns actual arithmetic; unknown bases decline.
pub fn used_text_indent(style: &ComputedStyle, content_inline_size: f64, intrinsic: bool) -> Option<f64> {
    if !content_inline_size.is_finite() || content_inline_size < 0.0 { return None; }
    let indent = TextIndent::parse(style.get("text-indent")?)?;
    let basis = if intrinsic { 0.0 } else { content_inline_size };
    crate::used_position_axis(&indent.expression, basis)
}
