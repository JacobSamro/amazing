//! Typed values for the CSS-1 prototype. Absolute lengths use CSS px (96/in).
//! Percentages and context-dependent units retain their basis; this module
//! never guesses a containing block or font metric. Unsupported compound
//! grammars remain explicitly `Unparsed`, behind the existing CSS seam.

use crate::syntax::{Token, tokenize};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LengthPercentage {
    Px(f64),
    /// 100 means 100%, not 1%.
    Percentage(f64),
    Relative(f64, RelativeUnit),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelativeUnit {
    Em,
    Rem,
    Ex,
    Ch,
    Ic,
    Cap,
    Lh,
    Rch,
    Ric,
    Rex,
    Rcap,
    Rlh,
    Vw,
    Vh,
    Vmin,
    Vmax,
}

impl LengthPercentage {
    pub fn parse(css: &str) -> Option<Self> {
        let tokens = tokenize(css);
        match significant(&tokens).as_slice() {
            [Token::Number(n)] if *n == 0.0 => Some(Self::Px(0.0)),
            [Token::Percentage(n)] if n.is_finite() => Some(Self::Percentage(*n)),
            [Token::Dimension(n, unit)] if n.is_finite() => {
                let scale = match unit.to_ascii_lowercase().as_str() {
                    "px" => 1.0,
                    "in" => 96.0,
                    "cm" => 96.0 / 2.54,
                    "mm" => 96.0 / 25.4,
                    "q" => 96.0 / 101.6,
                    "pt" => 96.0 / 72.0,
                    "pc" => 16.0,
                    "em" => return Some(Self::Relative(*n, RelativeUnit::Em)),
                    "rem" => return Some(Self::Relative(*n, RelativeUnit::Rem)),
                    "ex" => return Some(Self::Relative(*n, RelativeUnit::Ex)),
                    "ch" => return Some(Self::Relative(*n, RelativeUnit::Ch)),
                    "ic" => return Some(Self::Relative(*n, RelativeUnit::Ic)),
                    "cap" => return Some(Self::Relative(*n, RelativeUnit::Cap)),
                    "lh" => return Some(Self::Relative(*n, RelativeUnit::Lh)),
                    "rch" => return Some(Self::Relative(*n, RelativeUnit::Rch)),
                    "ric" => return Some(Self::Relative(*n, RelativeUnit::Ric)),
                    "rex" => return Some(Self::Relative(*n, RelativeUnit::Rex)),
                    "rcap" => return Some(Self::Relative(*n, RelativeUnit::Rcap)),
                    "rlh" => return Some(Self::Relative(*n, RelativeUnit::Rlh)),
                    "vw" => return Some(Self::Relative(*n, RelativeUnit::Vw)),
                    "vh" => return Some(Self::Relative(*n, RelativeUnit::Vh)),
                    "vmin" => return Some(Self::Relative(*n, RelativeUnit::Vmin)),
                    "vmax" => return Some(Self::Relative(*n, RelativeUnit::Vmax)),
                    _ => return None,
                };
                let px = n * scale;
                px.is_finite().then_some(Self::Px(px))
            }
            _ => None,
        }
    }

    fn nonnegative(self) -> bool {
        match self {
            Self::Px(n) | Self::Percentage(n) | Self::Relative(n, _) => n >= 0.0,
        }
    }
}

/// Angles use radians without reducing modulo a turn: rotations may interpolate
/// through multiple revolutions. Unitless zero is the existing transform policy.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Angle {
    pub radians: f64,
}
impl Angle {
    pub fn parse(css: &str) -> Option<Self> {
        let tokens = tokenize(css);
        let radians = match significant(&tokens).as_slice() {
            [Token::Number(n)] if *n == 0.0 => 0.0,
            [Token::Dimension(n, unit)] if n.is_finite() => {
                n * match unit.to_ascii_lowercase().as_str() {
                    "rad" => 1.0,
                    "deg" => std::f64::consts::PI / 180.0,
                    "grad" => std::f64::consts::PI / 200.0,
                    "turn" => std::f64::consts::TAU,
                    _ => return None,
                }
            }
            _ => return None,
        };
        radians.is_finite().then_some(Self { radians })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Color {
    Rgba([u8; 4]),
    CurrentColor,
}

macro_rules! keywords {
    ($($name:ident { $($variant:ident => $css:literal),+ $(,)? })+) => { $(
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum $name { $($variant),+ }
        impl $name {
            fn parse(css: &str) -> Option<Self> {
                match css.trim().to_ascii_lowercase().as_str() {
                    $($css => Some(Self::$variant),)+ _ => None,
                }
            }
        }
    )+ };
}
keywords! {
    Display { None => "none", Contents => "contents", Block => "block", Inline => "inline",
        Ruby => "ruby", RubyBase => "ruby-base", RubyText => "ruby-text",
        RubyBaseContainer => "ruby-base-container", RubyTextContainer => "ruby-text-container",
        InlineBlock => "inline-block", Flex => "flex", InlineFlex => "inline-flex", Grid => "grid",
        InlineGrid => "inline-grid", GridLanes => "grid-lanes",
        InlineGridLanes => "inline-grid-lanes", Table => "table", InlineTable => "inline-table",
        TableRow => "table-row", TableCell => "table-cell", TableRowGroup => "table-row-group",
        TableHeaderGroup => "table-header-group", TableFooterGroup => "table-footer-group",
        TableColumn => "table-column", TableColumnGroup => "table-column-group",
        TableCaption => "table-caption", ListItem => "list-item", FlowRoot => "flow-root" }
    Float { None => "none", Left => "left", Right => "right", InlineStart => "inline-start", InlineEnd => "inline-end" }
    Clear { None => "none", Left => "left", Right => "right", Both => "both", InlineStart => "inline-start", InlineEnd => "inline-end" }
    Alignment { Auto => "auto", Normal => "normal", Start => "start", End => "end", FlexStart => "flex-start", FlexEnd => "flex-end",
        Center => "center", Stretch => "stretch", Baseline => "baseline", FirstBaseline => "first baseline", LastBaseline => "last baseline",
        SpaceBetween => "space-between", SpaceAround => "space-around", SpaceEvenly => "space-evenly", SelfStart => "self-start", SelfEnd => "self-end",
        Left => "left", Right => "right", SafeCenter => "safe center", UnsafeCenter => "unsafe center" }
    Position { Static => "static", Relative => "relative", Absolute => "absolute", Fixed => "fixed", Sticky => "sticky" }
    Overflow { Visible => "visible", Hidden => "hidden", Clip => "clip", Scroll => "scroll", Auto => "auto" }
    Direction { Ltr => "ltr", Rtl => "rtl" }
    BoxSizing { ContentBox => "content-box", BorderBox => "border-box" }
    FlexDirection { Row => "row", RowReverse => "row-reverse", Column => "column", ColumnReverse => "column-reverse" }
    FlexWrap { Nowrap => "nowrap", Wrap => "wrap", WrapReverse => "wrap-reverse" }
    BorderStyle { None => "none", Hidden => "hidden", Dotted => "dotted", Dashed => "dashed", Solid => "solid",
        Double => "double", Groove => "groove", Ridge => "ridge", Inset => "inset", Outset => "outset" }
    WhiteSpace { Normal => "normal", Nowrap => "nowrap", Pre => "pre", PreWrap => "pre-wrap", PreLine => "pre-line", BreakSpaces => "break-spaces" }
    Visibility { Visible => "visible", Hidden => "hidden", Collapse => "collapse" }
    TextAlign { Start => "start", End => "end", Left => "left", Right => "right", Center => "center", Justify => "justify", MatchParent => "match-parent" }
    WritingMode { HorizontalTb => "horizontal-tb", VerticalRl => "vertical-rl", VerticalLr => "vertical-lr", SidewaysRl => "sideways-rl", SidewaysLr => "sideways-lr" }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keyword {
    Auto,
    None,
    Normal,
    MinContent,
    MaxContent,
    FitContent,
    Content,
}

/// A subgridded axis adopts track sizes from its parent. Names are kept in
/// source order; layout resolves repeat(auto-fill) against the used span.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subgrid {
    pub line_names: Vec<SubgridLineNames>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubgridLineNames {
    Names(Vec<String>),
    Repeat {
        count: SubgridRepeatCount,
        names: Vec<Vec<String>>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubgridRepeatCount {
    Integer(u32),
    AutoFill,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Transform {
    Translate(LengthPercentage, LengthPercentage),
    Scale(f64, f64),
    Rotate(Angle),
    SkewX(Angle),
    SkewY(Angle),
    Matrix([f64; 6]),
}

#[derive(Debug, Clone, PartialEq)]
pub enum SvgPaint {
    None,
    CurrentColor,
    Color([u8; 4]),
    Url(String),
}
#[derive(Debug, Clone, PartialEq)]
pub enum SvgResource {
    None,
    Url(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum ComputedValue {
    LengthPercentage(LengthPercentage),
    /// Valid CSS math retained until the consumer supplies a percentage basis.
    LengthPercentageExpression(String),
    SvgPaint(SvgPaint),
    SvgResource(SvgResource),
    SvgDasharray(Vec<LengthPercentage>),
    SvgKeyword(String),
    Color(Color),
    Number(f64),
    Integer(i32),
    Keyword(Keyword),
    Display(Display),
    Subgrid(Subgrid),
    Position(Position),
    Overflow(Overflow),
    Float(Float),
    Clear(Clear),
    Alignment(Alignment),
    Direction(Direction),
    BoxSizing(BoxSizing),
    FlexDirection(FlexDirection),
    FlexWrap(FlexWrap),
    BorderStyle(BorderStyle),
    WhiteSpace(WhiteSpace),
    Visibility(Visibility),
    TextAlign(TextAlign),
    WritingMode(WritingMode),
    Transform(Vec<Transform>),
    /// Grammar outside this prototype. The original text is available via get().
    Unparsed,
}

fn significant(tokens: &[Token]) -> Vec<Token> {
    tokens
        .iter()
        .filter(|t| !matches!(t, Token::Whitespace))
        .cloned()
        .collect()
}
fn number(css: &str) -> Option<f64> {
    match significant(&tokenize(css)).as_slice() {
        [Token::Number(n)] if n.is_finite() => Some(*n),
        _ => None,
    }
}
fn integer(css: &str) -> Option<i32> {
    // Token::Number does not retain the integer flag; check lexical syntax too.
    let css = css.trim();
    let digits = css.strip_prefix(['+', '-']).unwrap_or(css);
    if digits.is_empty() || !digits.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    css.parse().ok()
}

/// Canonical single-keyword serialization for supported CSS Display 3 pairs.
pub(crate) fn display_canonical(css: &str) -> Option<&'static str> {
    let tokens = tokenize(css);
    let words = significant(&tokens);
    let [Token::Ident(outer), Token::Ident(inner)] = words.as_slice() else {
        return None;
    };
    let outer = outer.to_ascii_lowercase();
    let inner = inner.to_ascii_lowercase();
    match (outer.as_str(), inner.as_str()) {
        ("inline", "flow") => Some("inline"),
        ("block", "flow") => Some("block"),
        ("inline", "flow-root") => Some("inline-block"),
        ("block", "flow-root") => Some("flow-root"),
        ("inline", "flex") => Some("inline-flex"),
        ("block", "flex") => Some("flex"),
        ("inline", "grid") => Some("inline-grid"),
        ("block", "grid") => Some("grid"),
        ("inline", "table") => Some("inline-table"),
        ("block", "table") => Some("table"),
        ("inline", "grid-lanes") => Some("inline-grid-lanes"),
        ("block", "grid-lanes") => Some("grid-lanes"),
        _ => None,
    }
}

/// CSS Ruby 1 keyword grammars and the legacy `ruby-overhang: none` alias.
pub(crate) fn ruby_canonical(property: &str, css: &str) -> Option<&'static str> {
    let tokens = tokenize(css);
    let words = significant(&tokens);
    let words: Option<Vec<_>> = words.iter().map(|token| match token {
        Token::Ident(word) => Some(word.to_ascii_lowercase()),
        _ => None,
    }).collect();
    let words = words?;
    let words: Vec<_> = words.iter().map(String::as_str).collect();
    match (property, words.as_slice()) {
        ("ruby-align", ["start"]) => Some("start"),
        ("ruby-align", ["center"]) => Some("center"),
        ("ruby-align", ["space-between"]) => Some("space-between"),
        ("ruby-align", ["space-around"]) => Some("space-around"),
        ("ruby-position", ["alternate"]) => Some("alternate"),
        ("ruby-position", ["over"]) => Some("over"),
        ("ruby-position", ["under"]) => Some("under"),
        ("ruby-position", ["inter-character"]) => Some("inter-character"),
        ("ruby-position", ["alternate", "over"] | ["over", "alternate"]) => Some("alternate over"),
        ("ruby-position", ["alternate", "under"] | ["under", "alternate"]) => Some("alternate under"),
        ("ruby-overhang", ["auto"]) => Some("auto"),
        ("ruby-overhang", ["spaces"] | ["none"]) => Some("spaces"),
        ("ruby-merge", ["separate"]) => Some("separate"),
        ("ruby-merge", ["merge"]) => Some("merge"),
        ("ruby-merge", ["auto"]) => Some("auto"),
        _ => None,
    }
}

/// None means invalid in a grammar owned here; Unparsed means outside our scope.
pub(crate) fn parse(property: &str, css: &str) -> Option<ComputedValue> {
    use ComputedValue as V;
    let lower = css.trim().to_ascii_lowercase();
    let s = lower.as_str();
    macro_rules! keyword {
        ($ty:ident) => {
            return $ty::parse(css).map(V::$ty)
        };
    }
    if crate::svg_presentation::is_svg_property(property) {
        return crate::svg_presentation::parse_property(property, css);
    }
    match property {
        p if crate::font_properties::is_audited_longhand(p) => {
            return crate::font_properties::canonical(p, css).map(|_| V::Unparsed);
        }
        p if p.starts_with("animation-") || p.starts_with("transition-") => {
            return crate::animation::canonical(p, css).map(|_| V::Unparsed);
        }
        "content" | "counter-reset" | "counter-increment" | "counter-set" | "quotes" => {
            return crate::generated::valid_property(property, css).then_some(V::Unparsed);
        }
        "font-feature-settings" | "font-variant-ligatures" => {
            return crate::computed::canonical_font_feature_value(property, css)
                .map(|_| V::Unparsed);
        }
        "zoom" => {
            return match significant(&tokenize(css)).as_slice() {
                [Token::Number(n)] if n.is_finite() && *n >= 0.0 => Some(V::Number(*n)),
                [Token::Percentage(n)] if n.is_finite() && *n >= 0.0 => Some(V::Number(*n / 100.0)),
                _ => None,
            };
        }
        "display" => {
            return Display::parse(display_canonical(css).unwrap_or(css)).map(V::Display);
        }
        "ruby-align" | "ruby-position" | "ruby-overhang" | "ruby-merge" => {
            return ruby_canonical(property, css).map(|_| V::Unparsed);
        }
        "image-orientation" => {
            if matches!(s, "from-image" | "none") { return Some(V::Unparsed); }
            let parts: Vec<_> = s.split_ascii_whitespace().collect();
            if parts == ["flip"]
                || matches!(parts.as_slice(), [angle] if Angle::parse(angle).is_some())
                || matches!(parts.as_slice(), ["flip", angle] | [angle, "flip"] if Angle::parse(angle).is_some()) {
                return Some(V::Unparsed);
            }
            return None;
        }
        "object-fit" => {
            return matches!(s, "fill" | "contain" | "cover" | "none" | "scale-down").then_some(V::Unparsed);
        }
        "object-position" => {
            let parts: Vec<_> = s.split_ascii_whitespace().collect();
            let valid = |part: &str| matches!(part, "left" | "right" | "top" | "bottom" | "center")
                || LengthPercentage::parse(part).is_some();
            return (!parts.is_empty() && parts.len() <= 4 && parts.iter().all(|part| valid(part))).then_some(V::Unparsed);
        }
        "grid-template-columns" | "grid-template-rows" => {
            return Some(
                crate::computed::parse_subgrid(css)
                    .map(V::Subgrid)
                    .unwrap_or(V::Unparsed),
            );
        }
        "position" => keyword!(Position),
        "float" => keyword!(Float),
        "clear" => keyword!(Clear),
        // Alignment's compound grammar is broader than the consumer subset.
        // Preserve unfamiliar combinations rather than declaring them invalid.
        "align-content" | "align-items" | "align-self" | "justify-content" | "justify-items"
        | "justify-self" => {
            return Some(
                Alignment::parse(css)
                    .map(V::Alignment)
                    .unwrap_or(V::Unparsed),
            );
        }
        "overflow" | "overflow-x" | "overflow-y" => keyword!(Overflow),
        "direction" => keyword!(Direction),
        "box-sizing" => keyword!(BoxSizing),
        "flex-direction" => keyword!(FlexDirection),
        "flex-wrap" => keyword!(FlexWrap),
        "white-space" => keyword!(WhiteSpace),
        "visibility" => keyword!(Visibility),
        "text-align" => keyword!(TextAlign),
        "writing-mode" => keyword!(WritingMode),
        "border-top-style" | "border-right-style" | "border-bottom-style" | "border-left-style" => {
            keyword!(BorderStyle)
        }
        "color"
        | "background-color"
        | "border-top-color"
        | "border-right-color"
        | "border-bottom-color"
        | "border-left-color" => {
            return if s == "currentcolor" {
                Some(V::Color(Color::CurrentColor))
            } else {
                // Color spaces outside the existing parser remain a preserved
                // unsupported grammar, not an invalid declaration that revives
                // a lower-ranked fallback. This keeps the staged seam honest.
                match significant(&tokenize(css)).as_slice() {
                    [Token::Function(name), ..]
                        if !matches!(name.to_ascii_lowercase().as_str(), "rgb" | "rgba") =>
                    {
                        Some(
                            crate::color::parse_css_color(css)
                                .map(|rgba| V::Color(Color::Rgba(rgba)))
                                .unwrap_or(V::Unparsed),
                        )
                    }
                    [Token::Ident(name)] if known_color_keyword(name) => Some(
                        crate::color::parse_css_color(css)
                            .map(|rgba| V::Color(Color::Rgba(rgba)))
                            .unwrap_or(V::Unparsed),
                    ),
                    _ => crate::color::parse_css_color(css).map(|rgba| V::Color(Color::Rgba(rgba))),
                }
            };
        }
        "opacity" => return number(css).map(|n| V::Number(n.clamp(0.0, 1.0))),
        "flex-grow" | "flex-shrink" => {
            // The existing layout solver supports these math constants. Keep
            // their policy there until calc() has a typed grammar here.
            if matches!(s, "calc(infinity)" | "calc(-infinity)" | "calc(nan)") {
                return Some(V::Unparsed);
            }
            return number(css).filter(|n| *n >= 0.0).map(V::Number);
        }
        "order" => return integer(css).map(V::Integer),
        "orphans" | "widows" => return integer(css).filter(|n| *n > 0).map(V::Integer),
        "z-index" => {
            return if s == "auto" {
                Some(V::Keyword(Keyword::Auto))
            } else {
                integer(css).map(V::Integer)
            };
        }
        "transform" => return transforms(css).map(V::Transform),
        "line-height" if number(css).is_some() => {
            return number(css).filter(|n| *n >= 0.0).map(V::Number);
        }
        _ => {}
    }
    let physical_property = match property {
        "inline-size" | "block-size" => "width",
        "min-inline-size" | "min-block-size" => "min-width",
        "max-inline-size" | "max-block-size" => "max-width",
        p if p.starts_with("margin-inline-") || p.starts_with("margin-block-") => "margin-top",
        p if p.starts_with("padding-inline-") || p.starts_with("padding-block-") => "padding-top",
        p if p.starts_with("inset-inline-") || p.starts_with("inset-block-") => "top",
        _ => property,
    };
    let property = physical_property;
    let margin = matches!(
        property,
        "margin-top" | "margin-right" | "margin-bottom" | "margin-left"
    );
    let inset = matches!(property, "top" | "right" | "bottom" | "left");
    let size = matches!(
        property,
        "width" | "height" | "min-width" | "min-height" | "max-width" | "max-height" | "flex-basis"
    );
    let border = matches!(
        property,
        "border-top-width" | "border-right-width" | "border-bottom-width" | "border-left-width"
    );
    let spacing = matches!(property, "letter-spacing" | "word-spacing");
    let gap = matches!(property, "row-gap" | "column-gap");
    let padding = matches!(
        property,
        "padding-top" | "padding-right" | "padding-bottom" | "padding-left"
    );
    if !(margin
        || inset
        || size
        || border
        || spacing
        || gap
        || padding
        || matches!(property, "font-size" | "line-height" | "text-indent"))
    {
        return Some(V::Unparsed);
    }
    let keyword = match s {
        "auto" if margin || inset || (size && !property.starts_with("max-")) => Some(Keyword::Auto),
        "none" if property.starts_with("max-") => Some(Keyword::None),
        "normal" if spacing || gap || property == "line-height" => Some(Keyword::Normal),
        "min-content" if size => Some(Keyword::MinContent),
        "max-content" if size => Some(Keyword::MaxContent),
        "fit-content" if size => Some(Keyword::FitContent),
        "content" if property == "flex-basis" => Some(Keyword::Content),
        _ => None,
    };
    if let Some(k) = keyword {
        return Some(V::Keyword(k));
    }
    if border {
        let px = match s {
            "thin" => Some(1.0),
            "medium" => Some(3.0),
            "thick" => Some(5.0),
            _ => None,
        };
        if let Some(px) = px {
            return Some(V::LengthPercentage(LengthPercentage::Px(px)));
        }
    }
    // Font-size keyword resolution already belongs to computed.rs; constructors
    // can still carry its unresolved default until compute supplies context.
    if property == "font-size"
        && matches!(
            s,
            "xx-small"
                | "x-small"
                | "small"
                | "medium"
                | "large"
                | "x-large"
                | "xx-large"
                | "xxx-large"
                | "larger"
                | "smaller"
        )
    {
        return Some(V::Unparsed);
    }
    if crate::math::length_percentage(css, !(border || property == "letter-spacing")) {
        return Some(V::LengthPercentageExpression(crate::syntax::serialize(
            &tokenize(css),
        )));
    }
    let length = LengthPercentage::parse(css)?;
    if (border || property == "letter-spacing") && matches!(length, LengthPercentage::Percentage(_))
    {
        return None;
    }
    if !(margin || inset || spacing || property == "text-indent") && !length.nonnegative() {
        return None;
    }
    Some(V::LengthPercentage(length))
}

fn transforms(css: &str) -> Option<Vec<Transform>> {
    if css.trim().eq_ignore_ascii_case("none") {
        return Some(Vec::new());
    }
    let tokens = tokenize(css);
    let components = crate::computed::top_level_components(&tokens)?;
    let mut out = Vec::new();
    for component in components {
        let Token::Function(name) = component.first()? else {
            return None;
        };
        let inner = crate::computed::function_inner(component, name)?;
        let args = crate::computed::transform_arguments(inner)?;
        let args: Vec<String> = args
            .iter()
            .map(|arg| crate::syntax::serialize(arg))
            .collect();
        let arg = |i: usize| args.get(i).map(String::as_str);
        let zero = LengthPercentage::Px(0.0);
        out.push(match name.to_ascii_lowercase().as_str() {
            "rotate" if args.len() == 1 => Transform::Rotate(Angle::parse(arg(0)?)?),
            "skewx" if args.len() == 1 => Transform::SkewX(Angle::parse(arg(0)?)?),
            "skewy" if args.len() == 1 => Transform::SkewY(Angle::parse(arg(0)?)?),
            "translate" if (1..=2).contains(&args.len()) => Transform::Translate(
                LengthPercentage::parse(arg(0)?)?,
                LengthPercentage::parse(arg(1).unwrap_or("0"))?,
            ),
            "translatex" if args.len() == 1 => {
                Transform::Translate(LengthPercentage::parse(arg(0)?)?, zero)
            }
            "translatey" if args.len() == 1 => {
                Transform::Translate(zero, LengthPercentage::parse(arg(0)?)?)
            }
            "scale" if (1..=2).contains(&args.len()) => {
                Transform::Scale(number(arg(0)?)?, number(arg(1).unwrap_or(arg(0)?))?)
            }
            "scalex" if args.len() == 1 => Transform::Scale(number(arg(0)?)?, 1.0),
            "scaley" if args.len() == 1 => Transform::Scale(1.0, number(arg(0)?)?),
            "matrix" if args.len() == 6 => Transform::Matrix([
                number(arg(0)?)?,
                number(arg(1)?)?,
                number(arg(2)?)?,
                number(arg(3)?)?,
                number(arg(4)?)?,
                number(arg(5)?)?,
            ]),
            _ => return None,
        });
    }
    (!out.is_empty()).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::computed::resolve;
    use std::collections::BTreeMap;
    fn style(
        parent: Option<&crate::ComputedStyle>,
        pairs: &[(&str, &str)],
    ) -> crate::ComputedStyle {
        resolve(
            parent,
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect::<BTreeMap<_, _>>(),
        )
    }
    #[test]
    fn absolute_units_and_percentages_have_distinct_bases() {
        for css in ["1in", "72pt", "6pc", "2.54cm", "25.4mm", "101.6Q", "96PX"] {
            let Some(LengthPercentage::Px(px)) = LengthPercentage::parse(css) else {
                panic!("{css}");
            };
            assert!((px - 96.0).abs() < 1e-10, "{css}");
        }
        assert_eq!(
            LengthPercentage::parse("25%"),
            Some(LengthPercentage::Percentage(25.0))
        );
        assert_eq!(
            LengthPercentage::parse("2em"),
            Some(LengthPercentage::Relative(2.0, RelativeUnit::Em))
        );
        for invalid in ["1 px", "1/**/px", "12", "1e999px", "2px 3px", "NaNpx"] {
            assert_eq!(LengthPercentage::parse(invalid), None, "{invalid}");
        }
    }
    #[test]
    fn values_four_font_relative_units_keep_their_metric_basis() {
        for (css, unit) in [
            ("2ic", RelativeUnit::Ic), ("2cap", RelativeUnit::Cap),
            ("2lh", RelativeUnit::Lh), ("2rch", RelativeUnit::Rch),
            ("2ric", RelativeUnit::Ric), ("2rex", RelativeUnit::Rex),
            ("2rcap", RelativeUnit::Rcap), ("2rlh", RelativeUnit::Rlh),
        ] {
            assert_eq!(LengthPercentage::parse(css),
                Some(LengthPercentage::Relative(2.0, unit)), "{css}");
            assert!(matches!(parse("width", css),
                Some(ComputedValue::LengthPercentage(LengthPercentage::Relative(2.0, found)))
                    if found == unit));
            let calc = format!("calc({css} + 1px)");
            assert!(matches!(parse("width", &calc),
                Some(ComputedValue::LengthPercentageExpression(value)) if value == calc));
        }
    }
    #[test]
    fn ruby_one_grammars_and_display_values() {
        for display in ["ruby", "ruby-base", "ruby-text", "ruby-base-container", "ruby-text-container"] {
            assert!(matches!(parse("display", display), Some(ComputedValue::Display(_))), "{display}");
        }
        for (property, valid, invalid) in [
            ("ruby-align", &["start", "center", "space-between", "space-around"][..], &["end", "justify", "space-evenly"][..]),
            ("ruby-position", &["over", "under", "alternate", "alternate over", "over alternate", "under alternate", "inter-character"][..], &["over under", "alternate alternate", "inter-character over"][..]),
            ("ruby-overhang", &["auto", "spaces", "none"][..], &["all", "never", "auto spaces"][..]),
            ("ruby-merge", &["separate", "merge", "auto"][..], &["collapse", "none", "merge separate"][..]),
        ] {
            for css in valid { assert!(parse(property, css).is_some(), "{property}: {css}"); }
            for css in invalid { assert!(parse(property, css).is_none(), "{property}: {css}"); }
        }
        assert_eq!(ruby_canonical("ruby-position", "UNDER alternate"), Some("alternate under"));
        assert_eq!(ruby_canonical("ruby-overhang", "NONE"), Some("spaces"));
    }
    #[test]
    fn angles_preserve_turn_count_and_convert_all_units() {
        for css in ["180deg", "200grad", "0.5turn"] {
            assert!((Angle::parse(css).unwrap().radians - std::f64::consts::PI).abs() < 1e-10);
        }
        assert_eq!(Angle::parse("2rad"), Some(Angle { radians: 2.0 }));
        assert!(Angle::parse("1").is_none());
        assert_eq!(
            Angle::parse("2turn").unwrap().radians,
            2.0 * std::f64::consts::TAU
        );
    }
    #[test]
    fn typed_values_follow_substitution_wide_keywords_and_invalid_winners() {
        let parent = style(None, &[("color", "blue"), ("width", "1in"), ("order", "4")]);
        let child = style(
            Some(&parent),
            &[
                ("--w", "2.54cm"),
                ("width", "var(--w)"),
                ("--bad", "red"),
                ("color", "var(--missing)"),
                ("order", "var(--bad)"),
                ("height", "inherit"),
                ("display", "initial"),
                ("opacity", "50%"),
            ],
        );
        assert_eq!(
            child.typed("width"),
            Some(&ComputedValue::LengthPercentage(LengthPercentage::Px(96.0)))
        );
        assert_eq!(
            child.typed("color"),
            Some(&ComputedValue::Color(Color::Rgba([0, 0, 255, 255])))
        );
        assert_eq!(child.typed("order"), Some(&ComputedValue::Integer(0)));
        assert_eq!(
            child.typed("display"),
            Some(&ComputedValue::Display(Display::Inline))
        );
        assert_eq!(child.typed("opacity"), Some(&ComputedValue::Number(0.5)));
        assert!(child.typed("--w").is_none());
        assert_eq!(
            style(Some(&parent), &[("width", "inherit")]).typed("width"),
            parent.typed("width")
        );
    }
    #[test]
    fn numeric_ranges_and_integer_lexical_grammar_are_enforced() {
        for (property, css) in [
            ("padding-left", "-1px"),
            ("width", "-2%"),
            ("flex-grow", "-1"),
            ("order", "1.5"),
            ("order", "1.0"),
            ("order", "1e2"),
            ("z-index", "2147483648"),
            ("orphans", "0"),
            ("border-left-width", "1%"),
            ("letter-spacing", "1%"),
        ] {
            assert!(parse(property, css).is_none(), "{property}: {css}");
        }
        assert_eq!(parse("order", "+42"), Some(ComputedValue::Integer(42)));
        assert!(parse("margin-top", "-1in").is_some());
        assert_eq!(parse("opacity", "2"), Some(ComputedValue::Number(1.0)));
    }
    #[test]
    fn mutable_and_collected_styles_cannot_leave_stale_typed_values() {
        let mut s: crate::ComputedStyle = [("width".into(), "1in".into())].into_iter().collect();
        assert_eq!(
            s.typed("width"),
            Some(&ComputedValue::LengthPercentage(LengthPercentage::Px(96.0)))
        );
        s.set("width", "50%");
        s.set("margin", "1px auto");
        assert_eq!(
            s.typed("width"),
            Some(&ComputedValue::LengthPercentage(
                LengthPercentage::Percentage(50.0)
            ))
        );
        assert_eq!(
            s.typed("margin-right"),
            Some(&ComputedValue::Keyword(Keyword::Auto))
        );
        s.set("--hidden", "red");
        assert!(s.typed("--hidden").is_none());
    }
    #[test]
    fn transform_list_retains_operations_and_typed_angles() {
        let s = style(
            None,
            &[(
                "transform",
                "translate(1px, 2px) rotate(200grad) scale(2, 3)",
            )],
        );
        let Some(ComputedValue::Transform(ops)) = s.typed("transform") else {
            panic!("missing transform")
        };
        assert_eq!(ops.len(), 3);
        assert!(
            matches!(ops[1], Transform::Rotate(Angle { radians }) if (radians - std::f64::consts::PI).abs() < 1e-10)
        );
    }
    #[test]
    fn color_and_layout_keywords_are_typed_without_changing_legacy_access() {
        let s = style(
            None,
            &[
                ("background-color", "rgba(255,0,0,0.5)"),
                ("position", "fixed"),
                ("flex-direction", "column-reverse"),
                ("border-top-style", "solid"),
            ],
        );
        assert_eq!(
            s.typed("background-color"),
            Some(&ComputedValue::Color(Color::Rgba([255, 0, 0, 128])))
        );
        assert_eq!(
            s.typed("position"),
            Some(&ComputedValue::Position(Position::Fixed))
        );
        assert_eq!(
            s.typed("flex-direction"),
            Some(&ComputedValue::FlexDirection(FlexDirection::ColumnReverse))
        );
        assert_eq!(s.get("position"), Some("fixed"));
        assert!(
            s.iter()
                .any(|(name, css)| name == "position" && css == "fixed")
        );
    }
}

/// Resolve units whose bases CSS already owns. Font metrics and viewport units
/// stay explicitly relative until the consumer supplies those used-value bases.
pub(crate) fn resolve_font_units(
    value: &mut ComputedValue,
    font: f64,
    root: f64,
    line_height: bool,
) {
    fn resolve(length: &mut LengthPercentage, font: f64, root: f64, percent: bool) {
        let px = match *length {
            LengthPercentage::Relative(n, RelativeUnit::Em) => Some(n * font),
            LengthPercentage::Relative(n, RelativeUnit::Rem) => Some(n * root),
            LengthPercentage::Percentage(n) if percent => Some(n * font / 100.0),
            _ => None,
        };
        if let Some(px) = px.filter(|px| px.is_finite()) {
            *length = LengthPercentage::Px(px);
        }
    }
    match value {
        ComputedValue::LengthPercentage(length) => resolve(length, font, root, line_height),
        ComputedValue::Transform(ops) => {
            for op in ops {
                if let Transform::Translate(x, y) = op {
                    resolve(x, font, root, false);
                    resolve(y, font, root, false);
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod inheritance_tests {
    use super::*;
    use crate::computed::resolve;
    fn style(
        parent: Option<&crate::ComputedStyle>,
        pairs: &[(&str, &str)],
    ) -> crate::ComputedStyle {
        resolve(
            parent,
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        )
    }
    #[test]
    fn inherited_computed_lengths_do_not_resolve_again_against_child_font() {
        let parent = style(
            None,
            &[
                ("font-size", "20px"),
                ("text-indent", "2em"),
                ("line-height", "150%"),
            ],
        );
        let mut child = style(Some(&parent), &[("font-size", "10px"), ("width", "2em")]);
        assert_eq!(
            child.typed("text-indent"),
            Some(&ComputedValue::LengthPercentage(LengthPercentage::Px(40.0)))
        );
        assert_eq!(
            child.typed("line-height"),
            Some(&ComputedValue::LengthPercentage(LengthPercentage::Px(30.0)))
        );
        assert_eq!(
            child.typed("width"),
            Some(&ComputedValue::LengthPercentage(LengthPercentage::Px(20.0)))
        );
        child.set("width", "3em");
        assert_eq!(
            child.typed("width"),
            Some(&ComputedValue::LengthPercentage(LengthPercentage::Px(30.0)))
        );
        assert_eq!(child.typed("text-indent"), parent.typed("text-indent"));
        let child = style(
            Some(&parent),
            &[("font-size", "10px"), ("text-indent", "var(--missing)")],
        );
        assert_eq!(child.typed("text-indent"), parent.typed("text-indent"));
    }
    #[test]
    fn currentcolor_on_color_inherits_and_absolute_font_size_computes() {
        let parent = style(None, &[("color", "red"), ("font-size", "12pt")]);
        let child = style(
            Some(&parent),
            &[("color", "currentcolor"), ("font-size", "2em")],
        );
        assert_eq!(
            child.typed("color"),
            Some(&ComputedValue::Color(Color::Rgba([255, 0, 0, 255])))
        );
        assert_eq!(
            child.typed("font-size"),
            Some(&ComputedValue::LengthPercentage(LengthPercentage::Px(32.0)))
        );
        assert_eq!(
            child.typed("border-top-color"),
            Some(&ComputedValue::Color(Color::CurrentColor))
        );
    }
}

// CSS Color 4 named/system keywords; recognizes validity without extending
// the transitional RGBA8 parser. https://www.w3.org/TR/css-color-4/#named-colors
pub(crate) fn named_color_keyword(name: &str) -> bool {
    const NAMES: &[&str] = &[
        "aliceblue",
        "antiquewhite",
        "aqua",
        "aquamarine",
        "azure",
        "beige",
        "bisque",
        "black",
        "blanchedalmond",
        "blue",
        "blueviolet",
        "brown",
        "burlywood",
        "cadetblue",
        "chartreuse",
        "chocolate",
        "coral",
        "cornflowerblue",
        "cornsilk",
        "crimson",
        "cyan",
        "darkblue",
        "darkcyan",
        "darkgoldenrod",
        "darkgray",
        "darkgreen",
        "darkgrey",
        "darkkhaki",
        "darkmagenta",
        "darkolivegreen",
        "darkorange",
        "darkorchid",
        "darkred",
        "darksalmon",
        "darkseagreen",
        "darkslateblue",
        "darkslategray",
        "darkslategrey",
        "darkturquoise",
        "darkviolet",
        "deeppink",
        "deepskyblue",
        "dimgray",
        "dimgrey",
        "dodgerblue",
        "firebrick",
        "floralwhite",
        "forestgreen",
        "fuchsia",
        "gainsboro",
        "ghostwhite",
        "gold",
        "goldenrod",
        "gray",
        "green",
        "greenyellow",
        "grey",
        "honeydew",
        "hotpink",
        "indianred",
        "indigo",
        "ivory",
        "khaki",
        "lavender",
        "lavenderblush",
        "lawngreen",
        "lemonchiffon",
        "lightblue",
        "lightcoral",
        "lightcyan",
        "lightgoldenrodyellow",
        "lightgray",
        "lightgreen",
        "lightgrey",
        "lightpink",
        "lightsalmon",
        "lightseagreen",
        "lightskyblue",
        "lightslategray",
        "lightslategrey",
        "lightsteelblue",
        "lightyellow",
        "lime",
        "limegreen",
        "linen",
        "magenta",
        "maroon",
        "mediumaquamarine",
        "mediumblue",
        "mediumorchid",
        "mediumpurple",
        "mediumseagreen",
        "mediumslateblue",
        "mediumspringgreen",
        "mediumturquoise",
        "mediumvioletred",
        "midnightblue",
        "mintcream",
        "mistyrose",
        "moccasin",
        "navajowhite",
        "navy",
        "oldlace",
        "olive",
        "olivedrab",
        "orange",
        "orangered",
        "orchid",
        "palegoldenrod",
        "palegreen",
        "paleturquoise",
        "palevioletred",
        "papayawhip",
        "peachpuff",
        "peru",
        "pink",
        "plum",
        "powderblue",
        "purple",
        "rebeccapurple",
        "red",
        "rosybrown",
        "royalblue",
        "saddlebrown",
        "salmon",
        "sandybrown",
        "seagreen",
        "seashell",
        "sienna",
        "silver",
        "skyblue",
        "slateblue",
        "slategray",
        "slategrey",
        "snow",
        "springgreen",
        "steelblue",
        "tan",
        "teal",
        "thistle",
        "tomato",
        "turquoise",
        "violet",
        "wheat",
        "white",
        "whitesmoke",
        "yellow",
        "yellowgreen",
    ];
    NAMES.contains(&name.to_ascii_lowercase().as_str())
}

fn known_color_keyword(name: &str) -> bool {
    const SYSTEM_NAMES: &[&str] = &[
        "accentcolor",
        "accentcolortext",
        "activetext",
        "buttonborder",
        "buttonface",
        "buttontext",
        "canvas",
        "canvastext",
        "field",
        "fieldtext",
        "graytext",
        "highlight",
        "highlighttext",
        "linktext",
        "mark",
        "marktext",
        "selecteditem",
        "selecteditemtext",
        "visitedtext",
        "transparent",
        "currentcolor",
        "activeborder",
        "activecaption",
        "appworkspace",
        "background",
        "buttonhighlight",
        "buttonshadow",
        "captiontext",
        "inactiveborder",
        "inactivecaption",
        "inactivecaptiontext",
        "infobackground",
        "infotext",
        "menu",
        "menutext",
        "scrollbar",
        "threeddarkshadow",
        "threedface",
        "threedhighlight",
        "threedlightshadow",
        "threedshadow",
        "window",
        "windowframe",
        "windowtext",
    ];
    named_color_keyword(name) || SYSTEM_NAMES.contains(&name.to_ascii_lowercase().as_str())
}
