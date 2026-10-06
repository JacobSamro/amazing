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
    Svw,
    Svh,
    Svmin,
    Svmax,
    Lvw,
    Lvh,
    Lvmin,
    Lvmax,
    Dvw,
    Dvh,
    Dvmin,
    Dvmax,
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
                    "svw" => return Some(Self::Relative(*n, RelativeUnit::Svw)),
                    "svh" => return Some(Self::Relative(*n, RelativeUnit::Svh)),
                    "svmin" => return Some(Self::Relative(*n, RelativeUnit::Svmin)),
                    "svmax" => return Some(Self::Relative(*n, RelativeUnit::Svmax)),
                    "lvw" => return Some(Self::Relative(*n, RelativeUnit::Lvw)),
                    "lvh" => return Some(Self::Relative(*n, RelativeUnit::Lvh)),
                    "lvmin" => return Some(Self::Relative(*n, RelativeUnit::Lvmin)),
                    "lvmax" => return Some(Self::Relative(*n, RelativeUnit::Lvmax)),
                    "dvw" => return Some(Self::Relative(*n, RelativeUnit::Dvw)),
                    "dvh" => return Some(Self::Relative(*n, RelativeUnit::Dvh)),
                    "dvmin" => return Some(Self::Relative(*n, RelativeUnit::Dvmin)),
                    "dvmax" => return Some(Self::Relative(*n, RelativeUnit::Dvmax)),
                    _ => return None,
                };
                let px = n * scale;
                px.is_finite().then_some(Self::Px(px))
            }
            _ => None,
        }
    }

    pub fn is_nonnegative(self) -> bool {
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
            pub(crate) fn parse(css: &str) -> Option<Self> {
                match css.trim().to_ascii_lowercase().as_str() {
                    $($css => Some(Self::$variant),)+ _ => None,
                }
            }
        }
    )+ };
}
keywords! {
    Display { None => "none", Contents => "contents", Block => "block", Inline => "inline",
        Ruby => "ruby", BlockRuby => "block ruby", RubyBase => "ruby-base", RubyText => "ruby-text",
        RubyBaseContainer => "ruby-base-container", RubyTextContainer => "ruby-text-container",
        InlineBlock => "inline-block", Flex => "flex", InlineFlex => "inline-flex", Grid => "grid",
        InlineGrid => "inline-grid", GridLanes => "grid-lanes",
        InlineGridLanes => "inline-grid-lanes", Table => "table", InlineTable => "inline-table",
        TableRow => "table-row", TableCell => "table-cell", TableRowGroup => "table-row-group",
        TableHeaderGroup => "table-header-group", TableFooterGroup => "table-footer-group",
        TableColumn => "table-column", TableColumnGroup => "table-column-group",
        TableCaption => "table-caption", ListItem => "list-item", FlowRoot => "flow-root",
        RunIn => "run-in", RunInFlowRoot => "run-in flow-root", RunInFlex => "run-in flex",
        RunInGrid => "run-in grid", RunInGridLanes => "run-in grid-lanes",
        RunInTable => "run-in table", RunInRuby => "run-in ruby",
        InlineListItem => "inline list-item", RunInListItem => "run-in list-item",
        FlowRootListItem => "flow-root list-item", InlineFlowRootListItem => "inline flow-root list-item",
        RunInFlowRootListItem => "run-in flow-root list-item" }
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
    Stretch,
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
    Translate3D(LengthPercentage, LengthPercentage, LengthPercentage),
    TranslateExpression([String; 3]),
    Scale3D([f64; 3]),
    Rotate3D([f64; 3], Angle),
    Skew(Angle, Angle),
    Matrix3D([f64; 16]),
    Perspective(Option<String>),
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
    FontSizeAdjust(crate::font_size_adjust::FontSizeAdjust),
    FilterList(crate::effects::FilterList),
    BlendMode(crate::effects::BlendMode),
    /// `isolation: isolate` is true; `auto` is false.
    Isolation(bool),
    MaskImage(Vec<String>),
    MaskMode(crate::effects::MaskMode),
    MaskType(crate::effects::MaskType),
    MaskComposite(crate::effects::MaskComposite),
    MaskBox(crate::effects::MaskBox),
    ClipPath(crate::effects::ClipPath),
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

/// `<alpha-value> = <number> | <percentage>` (CSS Color 4 SS4.5), with its
/// `calc()`/`min()`/`max()`/`clamp()` math (CSS Values 4 SS10). The
/// percentage basis is always exactly 1 (100% = 1), a fixed basis unlike a
/// length's containing-block-dependent one, so -- unlike `crate::math`,
/// which only validates grammar and leaves resolution to a later consumer --
/// this evaluates to a concrete `f64` immediately. Used by `opacity` and
/// `fill-opacity`; the caller still clamps the final result to `[0, 1]`.
pub(crate) fn alpha_value(css: &str) -> Option<f64> {
    let tokens = significant(&tokenize(css));
    let mut pos = 0;
    let value = alpha_sum(&tokens, &mut pos)?;
    (pos == tokens.len()).then_some(value)
}
/// This tokenizer lexes a standalone `-` or `+` as a one-character
/// `Token::Ident`, not a `Token::Delim` (a bare `-` is a valid identifier
/// under CSS Syntax 3's "would start an identifier" rule) -- so `calc(1 -
/// 1)`'s middle token is `Ident("-")`. Match both forms.
fn is_op(token: Option<&Token>, op: char) -> bool {
    matches!(token, Some(Token::Delim(c)) if *c == op)
        || matches!(token, Some(Token::Ident(s)) if s.len() == 1 && s.starts_with(op))
}
fn alpha_sum(tokens: &[Token], pos: &mut usize) -> Option<f64> {
    let mut value = alpha_product(tokens, pos)?;
    loop {
        if is_op(tokens.get(*pos), '+') { *pos += 1; value += alpha_product(tokens, pos)?; }
        else if is_op(tokens.get(*pos), '-') { *pos += 1; value -= alpha_product(tokens, pos)?; }
        else { return Some(value); }
    }
}
fn alpha_product(tokens: &[Token], pos: &mut usize) -> Option<f64> {
    let mut value = alpha_atom(tokens, pos)?;
    loop {
        match tokens.get(*pos) {
            Some(Token::Delim('*')) => { *pos += 1; value *= alpha_atom(tokens, pos)?; }
            Some(Token::Delim('/')) => {
                *pos += 1;
                let divisor = alpha_atom(tokens, pos)?;
                if divisor == 0.0 { return None; }
                value /= divisor;
            }
            _ => return Some(value),
        }
    }
}
fn alpha_comma_args(tokens: &[Token], pos: &mut usize) -> Option<Vec<f64>> {
    let mut values = vec![alpha_sum(tokens, pos)?];
    while matches!(tokens.get(*pos), Some(Token::Comma)) {
        *pos += 1;
        values.push(alpha_sum(tokens, pos)?);
    }
    matches!(tokens.get(*pos), Some(Token::CloseParen)).then(|| { *pos += 1; values })
}
fn alpha_atom(tokens: &[Token], pos: &mut usize) -> Option<f64> {
    match tokens.get(*pos)?.clone() {
        Token::Number(n) => { *pos += 1; Some(n) }
        Token::Percentage(n) => { *pos += 1; Some(n / 100.0) }
        Token::Delim('-') => { *pos += 1; Some(-alpha_atom(tokens, pos)?) }
        Token::Delim('+') => { *pos += 1; alpha_atom(tokens, pos) }
        Token::OpenParen => {
            *pos += 1;
            let value = alpha_sum(tokens, pos)?;
            matches!(tokens.get(*pos), Some(Token::CloseParen)).then(|| { *pos += 1; value })
        }
        Token::Function(name) => {
            let lower = name.to_ascii_lowercase();
            *pos += 1;
            if !matches!(tokens.get(*pos), Some(Token::OpenParen)) { return None; }
            *pos += 1;
            match lower.as_str() {
                "calc" => {
                    let value = alpha_sum(tokens, pos)?;
                    matches!(tokens.get(*pos), Some(Token::CloseParen)).then(|| { *pos += 1; value })
                }
                "min" => alpha_comma_args(tokens, pos)?.into_iter().reduce(f64::min),
                "max" => alpha_comma_args(tokens, pos)?.into_iter().reduce(f64::max),
                "clamp" => match alpha_comma_args(tokens, pos)?.as_slice() {
                    // CSS Values 4 SS10.6: `max(MIN, min(VAL, MAX))`, not
                    // `VAL` clamped naively -- when MIN > MAX the result is
                    // MIN, and Rust's own `f64::clamp` panics on that input.
                    [min_v, val, max_v] => Some(min_v.max(val.min(*max_v))),
                    _ => None,
                },
                _ => None,
            }
        }
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
    if let [Token::Ident(word)] = words.as_slice() {
        let word = word.to_ascii_lowercase();
        let single = match word.as_str() {
            "none" => Some("none"), "contents" => Some("contents"),
            "inline-block" => Some("inline-block"), "inline-table" => Some("inline-table"),
            "inline-flex" => Some("inline-flex"), "inline-grid" => Some("inline-grid"),
            "inline-grid-lanes" => Some("inline-grid-lanes"),
            "table-row" => Some("table-row"), "table-cell" => Some("table-cell"),
            "table-row-group" => Some("table-row-group"), "table-header-group" => Some("table-header-group"),
            "table-footer-group" => Some("table-footer-group"), "table-column" => Some("table-column"),
            "table-column-group" => Some("table-column-group"), "table-caption" => Some("table-caption"),
            "ruby-base" => Some("ruby-base"), "ruby-text" => Some("ruby-text"),
            "ruby-base-container" => Some("ruby-base-container"), "ruby-text-container" => Some("ruby-text-container"),
            _ => None,
        };
        if single.is_some() { return single; }
    }
    let mut outer = None;
    let mut inner = None;
    let mut list = false;
    if words.is_empty() || words.len() > 3 { return None; }
    for token in &words {
        let Token::Ident(word) = token else { return None };
        let word = word.to_ascii_lowercase();
        match word.as_str() {
            "block" | "inline" | "run-in" => {
                if outer.replace(word).is_some() { return None; }
            }
            "flow" | "flow-root" | "table" | "flex" | "grid" | "ruby" | "grid-lanes" => {
                if inner.replace(word).is_some() { return None; }
            }
            "list-item" if !list => list = true,
            _ => return None,
        }
    }
    let inner = inner.as_deref().unwrap_or("flow");
    let outer = outer.as_deref().unwrap_or(if inner == "ruby" && !list { "inline" } else { "block" });
    if list {
        return match (outer, inner) {
            ("block", "flow") => Some("list-item"),
            ("inline", "flow") => Some("inline list-item"),
            ("run-in", "flow") => Some("run-in list-item"),
            ("block", "flow-root") => Some("flow-root list-item"),
            ("inline", "flow-root") => Some("inline flow-root list-item"),
            ("run-in", "flow-root") => Some("run-in flow-root list-item"),
            _ => None,
        };
    }
    match (outer, inner) {
        ("inline", "flow") => Some("inline"),
        ("block", "flow") => Some("block"),
        ("run-in", "flow") => Some("run-in"),
        ("inline", "flow-root") => Some("inline-block"),
        ("block", "flow-root") => Some("flow-root"),
        ("run-in", "flow-root") => Some("run-in flow-root"),
        ("inline", "flex") => Some("inline-flex"),
        ("block", "flex") => Some("flex"),
        ("run-in", "flex") => Some("run-in flex"),
        ("inline", "grid") => Some("inline-grid"),
        ("block", "grid") => Some("grid"),
        ("run-in", "grid") => Some("run-in grid"),
        ("inline", "ruby") => Some("ruby"),
        ("block", "ruby") => Some("block ruby"),
        ("run-in", "ruby") => Some("run-in ruby"),
        ("inline", "table") => Some("inline-table"),
        ("block", "table") => Some("table"),
        ("run-in", "table") => Some("run-in table"),
        ("inline", "grid-lanes") => Some("inline-grid-lanes"),
        ("block", "grid-lanes") => Some("grid-lanes"),
        ("run-in", "grid-lanes") => Some("run-in grid-lanes"),
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

/// Whether `value` is a valid value for `property`, per this crate's
/// per-property grammars. Exposed read-only for the WPT parsing-conformance
/// runner (CSS-25, `examples/wpt_conformance.rs`); cascade code keeps using
/// the crate-internal `parse` directly. Properties this crate does not model
/// a grammar for fall through to `ComputedValue::Unparsed` (see `parse`'s own
/// doc comment) and so always report valid here -- an accurate reflection of
/// what this crate can and cannot reject, not a false pass.
pub fn parses(property: &str, value: &str) -> bool {
    parse(property, value).is_some()
}

/// None means invalid in a grammar owned here; Unparsed means outside our scope.
pub(crate) fn parse(property: &str, css: &str) -> Option<ComputedValue> {
    use ComputedValue as V;
    // Tree-dependent numbers have a specified type before their actual value
    // exists. Do not assign a fake count merely to validate numeric grammar.
    if crate::computed::tree_context::contains(css) {
        match property {
            "opacity"|"fill-opacity"|"zoom" => return crate::math::number_percentage(css,true).then_some(V::Unparsed),
            "order"|"z-index"|"column-count"|"orphans"|"widows" => return crate::math::number_percentage(css,false).then_some(V::Unparsed),
            _=>{},
        }
    }
    let lower = css.trim().to_ascii_lowercase();
    let s = lower.as_str();
    macro_rules! keyword {
        ($ty:ident) => {
            return $ty::parse(css).map(V::$ty)
        };
    }
    if crate::text_properties::supported(property) {
        let canonical = crate::text_properties::canonical(property, css)?;
        return Some(if property == "white-space" { WhiteSpace::parse(&canonical).map(V::WhiteSpace).unwrap_or(V::Unparsed) } else { V::Unparsed });
    }
    if crate::computed::border_radius::supported(property) {
        return crate::computed::border_radius::valid(property, css).then_some(V::Unparsed);
    }
    if property == "clip-path" {
        return crate::effects::parse_clip_path(css).map(V::ClipPath);
    }
    if crate::svg_presentation::is_svg_property(property) {
        return crate::svg_presentation::parse_property(property, css);
    }
    match property {
        "font-width" => return crate::font_selection::canonical("font-stretch",css).map(|_|V::Unparsed),
        p if crate::font_selection::supported(p) => return crate::font_selection::canonical(p,css).map(|_|V::Unparsed),
        "font" => return crate::font_shorthand::expand(css).map(|_| V::Unparsed),
        "font-variant" => return crate::font_variant::expand(css).map(|_| V::Unparsed),
        "font-synthesis" => return crate::font_properties::expand_synthesis(css).map(|_| V::Unparsed),
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
        "object-fit" | "image-rendering" => {
            return crate::image_policy::canonical(property,css).map(|_| V::Unparsed);
        }
        "object-position" | "transform-origin" | "perspective-origin" | "transform-box" => {
            return crate::computed::position_values::canonical(property,css).map(|_|V::Unparsed);
        }
        "grid-template-columns" | "grid-template-rows" => {
            if crate::computed::css_wide_keyword(css).is_some() {
                return Some(V::Unparsed);
            }
            if !crate::computed::valid_grid_track_list(css) {
                return None;
            }
            return Some(
                crate::computed::parse_subgrid(css)
                    .map(V::Subgrid)
                    .unwrap_or(V::Unparsed),
            );
        }
        "grid-row-start" | "grid-row-end" | "grid-column-start" | "grid-column-end" => {
            return crate::computed::valid_grid_line(css).then_some(V::Unparsed);
        }
        "grid-row" | "grid-column" => {
            return crate::computed::valid_grid_line_pair(css).then_some(V::Unparsed);
        }
        "grid-area" => {
            return crate::computed::valid_grid_area(css).then_some(V::Unparsed);
        }
        "grid-auto-columns" | "grid-auto-rows" => {
            return crate::computed::valid_grid_auto_track_list(css).then_some(V::Unparsed);
        }
        "grid-auto-flow" => {
            return crate::computed::valid_grid_auto_flow(css).then_some(V::Unparsed);
        }
        "grid-template-areas" => {
            return crate::computed::valid_grid_template_areas(css).then_some(V::Unparsed);
        }
        "grid-template" => {
            return crate::computed::valid_grid_template(css).then_some(V::Unparsed);
        }
        "grid" => {
            return crate::computed::valid_grid_shorthand(css).then_some(V::Unparsed);
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
                    [Token::Function(name), ..] if name.eq_ignore_ascii_case("light-dark") => {
                        crate::computed::valid_light_dark(css).then_some(V::Unparsed)
                    }
                    // CSS-27: `alpha()` gets its own dedicated grammar check
                    // rather than the generic "unrecognized function ->
                    // preserved as Unparsed" fallback just below, which
                    // otherwise makes a genuinely malformed `alpha(...)`
                    // (e.g. missing its mandatory `/ <alpha>` clause) parse
                    // as valid-but-unresolved instead of being rejected.
                    [Token::Function(name), ..] if name.eq_ignore_ascii_case("alpha") => {
                        match crate::color::classify_alpha_function(css) {
                            crate::color::AlphaClassification::Invalid => None,
                            crate::color::AlphaClassification::Deferred => Some(V::Unparsed),
                            crate::color::AlphaClassification::Resolved(rgba) => {
                                Some(V::Color(Color::Rgba(rgba)))
                            }
                        }
                    }
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
                    _ => crate::color::parse_css_color(css)
                        .map(|rgba| V::Color(Color::Rgba(rgba)))
                        .or_else(|| {
                            // A relative color whose source is `currentcolor`
                            // is resolved against the element's `color` by the
                            // computed-value seam; keep it as a preserved
                            // grammar rather than an invalid declaration.
                            let tokens = significant(&tokenize(css));
                            matches!(tokens.get(2),
                                Some(Token::Ident(name)) if name.eq_ignore_ascii_case("from"))
                            .then_some(V::Unparsed)
                        }),
                }
            };
        }
        "color-scheme" => {
            let tokens: Vec<&str> = s.split_ascii_whitespace().collect();
            let valid = match tokens.as_slice() {
                ["normal"] => true,
                [] => false,
                _ => {
                    let (mut only, mut light, mut dark, mut ok) = (0u8, 0u8, 0u8, true);
                    for token in &tokens {
                        match *token {
                            "only" => only += 1,
                            "light" => light += 1,
                            "dark" => dark += 1,
                            _ => { ok = false; break; }
                        }
                    }
                    ok && only <= 1 && light <= 1 && dark <= 1 && light + dark >= 1
                }
            };
            return valid.then_some(V::Unparsed);
        }
        // <alpha-value> = <number> | <percentage>, with the usual calc()
        // math (CSS Color 4 SS4.5, CSS Values 4 SS10) -- not just a bare
        // number as before, which silently dropped every calc()/percentage/
        // min()/max()/clamp() opacity to the initial value 1.
        "translate" | "rotate" | "scale" => return crate::computed::individual_transforms::valid(property, css).then_some(V::Unparsed),
        "opacity" | "fill-opacity" => return alpha_value(css).map(|n| V::Number(n.clamp(0.0, 1.0))),
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
        "flow-tolerance" => return crate::computed::flow_tolerance::parse(css),
        "transform" => return transforms(css).map(V::Transform),
        // CSS-29: box-shadow keeps its cascade-side literal representation
        // (`Unparsed`, same as before), but now actually goes through the
        // real grammar check -- previously this property had no arm here at
        // all, so `values::parses`/the WPT parsing-conformance runner
        // reported every string as valid regardless of content, even though
        // `expand_declaration`'s cascade path already rejects malformed
        // shadows via `valid_box_shadow`.
        "box-shadow" => {
            return (s == "none" || crate::computed::valid_box_shadow(css)).then_some(V::Unparsed);
        }
        "filter" | "backdrop-filter" => {
            return crate::effects::parse_filter_list(css).map(V::FilterList);
        }
        "mix-blend-mode" => {
            return crate::effects::parse_blend_mode(css).map(V::BlendMode);
        }
        "isolation" => {
            return crate::effects::parse_isolation(css).map(V::Isolation);
        }
        "mask-image" => {
            return crate::effects::parse_mask_image(css).map(V::MaskImage);
        }
        "mask-mode" => {
            return crate::effects::parse_mask_mode(css).map(V::MaskMode);
        }
        "mask-type" => {
            return crate::effects::parse_mask_type(css).map(V::MaskType);
        }
        "mask-composite" => {
            return crate::effects::parse_mask_composite(css).map(V::MaskComposite);
        }
        "mask-clip" => {
            return crate::effects::parse_mask_box(css, true).map(V::MaskBox);
        }
        "mask-origin" => {
            return crate::effects::parse_mask_box(css, false).map(V::MaskBox);
        }
        "mask-repeat" => {
            return crate::effects::valid_repeat_list(css).then_some(V::Unparsed);
        }
        "mask-position" => {
            return crate::effects::valid_position_list(css).then_some(V::Unparsed);
        }
        "mask-size" => {
            return crate::effects::valid_size_list(css).then_some(V::Unparsed);
        }
        "line-height" if number(css).is_some() => {
            return number(css).filter(|n| *n >= 0.0).map(V::Number);
        }
        // CSS Cascade 3 SS7.4: `all`'s grammar is the CSS-wide keywords only
        // (it resets every longhand to that keyword); it is not itself a
        // longhand with a real value space, so it needs its own arm rather
        // than falling into the generic accept-anything `Unparsed` case
        // below for any property this crate does not otherwise model.
        "all" => {
            return matches!(s, "initial" | "inherit" | "unset" | "revert" | "revert-layer")
                .then_some(V::Unparsed);
        }
        // CSS Contain 3 SS container-name/-type/shorthand. Not evaluated
        // (no layout size dependency here), but validated and canonically
        // serialized so cascade/computed-value plumbing is honest about
        // acceptance instead of the generic accept-anything fallback.
        "container-name" => {
            return (s == "none" || custom_idents(css).is_some_and(|idents| !idents.is_empty()))
                .then_some(V::Unparsed);
        }
        "container-type" => {
            return valid_container_type(s).then_some(V::Unparsed);
        }
        "container" => {
            return parse_container_shorthand(css).is_some().then_some(V::Unparsed);
        }
        // CSS-30: these five had no dedicated arm at all, so every one of
        // them fell into the generic accept-anything `Unparsed` case below
        // (it does not match `margin`/`inset`/`size`/`border`/`spacing`/
        // `gap`/`padding`) -- `values::parses`, which the WPT
        // parsing-conformance runner's "valid"/"invalid" dimension calls
        // directly, reported every invalid value as valid regardless of
        // content. `crate::computed`'s per-property validators (CSS-30,
        // `background_values.rs`) are the same ones `expand_declaration`
        // uses for the computed-value dimension, so both dimensions now
        // agree on what is accepted.
        p if crate::computed::contain_intrinsic::is_longhand(p) || p == "contain-intrinsic-size" => {
            return crate::computed::contain_intrinsic::valid(p, css).then_some(V::Unparsed);
        }
        p if crate::border_image::supported(p) => return crate::border_image::canonical(p, css).map(|_| V::Unparsed),
        "background-image" => return crate::image_values::valid_background_image(css).then_some(V::Unparsed),
        "list-style-image" => return crate::image_values::valid_image_source(css).then_some(V::Unparsed),
        "background-clip" => {
            return crate::computed::valid_background_clip(css).then_some(V::Unparsed);
        }
        "background-origin" => {
            return crate::computed::valid_background_origin(css).then_some(V::Unparsed);
        }
        "background-attachment" => {
            return crate::computed::valid_background_attachment(css).then_some(V::Unparsed);
        }
        "background-repeat" => {
            return crate::computed::valid_background_repeat(css).then_some(V::Unparsed);
        }
        "background-size" => {
            return crate::computed::valid_background_size(css).then_some(V::Unparsed);
        }
        "background-position" => {
            return crate::computed::valid_background_position(css).then_some(V::Unparsed);
        }
        "background-position-x" => {
            return crate::computed::valid_background_position_axis(css, true)
                .then_some(V::Unparsed);
        }
        "background-position-y" => {
            return crate::computed::valid_background_position_axis(css, false)
                .then_some(V::Unparsed);
        }
        _ => {}
    }
    if (property.starts_with("border-inline-") || property.starts_with("border-block-"))
        && (property.contains("-start-") || property.contains("-end-"))
        && let Some(component) = property.rsplit('-').next()
        && matches!(component, "width" | "style" | "color")
    {
        return parse(&format!("border-top-{component}"), css);
    }
    if let Some(valid) = crate::computed::logical_border::valid(property, css) {
        return valid.then_some(V::Unparsed);
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
        // CSS Sizing 4 `stretch` (formerly `-webkit-fill-available`): accept
        // the bare keyword for the same sizing properties as the three
        // keywords above. `math.rs`'s `calc_size_basis` already accepted
        // `stretch` as a `calc-size()` basis; this is the separate bare-
        // keyword grammar path every `width: stretch` (no wrapper) fixture
        // uses -- amazing-layout already resolves it once it survives the
        // cascade (LAYOUT-32/33).
        "stretch" | "-webkit-fill-available" if size => Some(Keyword::Stretch),
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
    // CSS Values 5 calc-size() is valid in sizing contexts. The intrinsic
    // `size` keyword stays unresolved for layout's used-value stage; the
    // canonical text is retained through `get()` and `typed()`.
    if size && crate::math::calc_size(css) {
        return Some(V::LengthPercentageExpression(crate::syntax::serialize(
            &tokenize(css),
        )));
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
    if !(margin || inset || spacing || property == "text-indent") && !length.is_nonnegative() {
        return None;
    }
    Some(V::LengthPercentage(length))
}

/// CSS Contain 3 `<custom-ident>+`: one or more identifiers, none of them a
/// CSS-wide keyword or `none`/`default` (reserved everywhere a custom-ident
/// grammar appears). Returns `None` for anything that isn't a plain
/// whitespace-separated identifier list (so a `var()` or other substitution
/// doesn't get rejected here -- callers gate on that separately).
fn custom_idents(css: &str) -> Option<Vec<String>> {
    let all_tokens = tokenize(css);
    let tokens: Vec<&Token> = all_tokens.iter()
        .filter(|t| !matches!(t, Token::Whitespace)).collect();
    if tokens.is_empty() { return None; }
    tokens.into_iter().map(|token| match token {
        Token::Ident(name) if !matches!(
            name.to_ascii_lowercase().as_str(),
            "none" | "default" | "initial" | "inherit" | "unset" | "revert" | "revert-layer"
        ) => Some(name.clone()),
        _ => None,
    }).collect()
}

/// CSS Contain 3 `container-type`: `normal | [ [ size | inline-size ] ||
/// scroll-state ]`.
fn valid_container_type(lower: &str) -> bool {
    if lower == "normal" { return true; }
    let mut words: Vec<&str> = lower.split_ascii_whitespace().collect();
    words.sort_unstable();
    matches!(words.as_slice(),
        ["scroll-state"] | ["size"] | ["inline-size"]
            | ["scroll-state", "size"] | ["inline-size", "scroll-state"])
}

/// CSS Contain 3 `container`: `<'container-name'> [ / <'container-type'> ]?`.
fn parse_container_shorthand(css: &str) -> Option<()> {
    let (name_part, type_part) = match css.split_once('/') {
        Some((name, container_type)) => (name.trim(), Some(container_type.trim())),
        None => (css.trim(), None),
    };
    if !(name_part.eq_ignore_ascii_case("none") || custom_idents(name_part).is_some_and(|i| !i.is_empty())) {
        return None;
    }
    if let Some(container_type) = type_part {
        if !valid_container_type(&container_type.to_ascii_lowercase()) {
            return None;
        }
    }
    Some(())
}

fn transforms(css: &str) -> Option<Vec<Transform>> { crate::transform_values::parse(css) }

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
    fn calc_size_is_preserved_for_layout() {
        let canonical = |css: &str| crate::syntax::serialize(&tokenize(css));
        for css in [
            "calc-size(auto, size + 20px)",
            "calc-size(min-content, size / 2)",
            "calc-size(max-content, size)",
            "calc-size(fit-content, size)",
            "calc-size(fit-content(10px), size + 1px)",
            "calc-size(any, 20px)",
            "calc-size(50%, size + 10px)",
            "calc-size(calc-size(auto, size), size + 10px)",
        ] {
            let expected = canonical(css);
            for property in ["width", "min-width", "max-height", "flex-basis", "inline-size"] {
                assert!(
                    matches!(parse(property, css),
                        Some(ComputedValue::LengthPercentageExpression(value)) if value == expected),
                    "{property}: {css}"
                );
            }
        }
        // The computed declaration survives the validity gate so layout can
        // evaluate the intrinsic basis at the used-value stage.
        let style = style(None, &[("min-width", "calc-size(auto, size + 20px)")]);
        assert_eq!(style.get("min-width"), Some("calc-size(auto, size + 20px)"));
        assert!(matches!(style.typed("min-width"),
            Some(ComputedValue::LengthPercentageExpression(_))));
        // `any` forbids the `size` keyword; malformed arithmetic and trailing
        // tokens are rejected; non-sizing properties do not accept the function.
        for invalid in [
            "calc-size(any, size)",
            "calc-size(auto)",
            "calc-size(auto size)",
            "calc-size(auto, size + 20px) 5px",
            "calc-size(bogus, 20px)",
            "calc-size(auto, size +)",
            "calc-size(auto, size + 1em +)",
        ] {
            assert!(parse("min-width", invalid).is_none(), "{invalid}");
        }
        assert!(parse("margin-top", "calc-size(auto, size + 20px)").is_none());
    }
    #[test]
    fn calc_size_nests_inside_min_max_and_clamp() {
        // CSS Values 5 SS11: `calc-size()` is itself a `<calc-sum>`, so it may
        // appear as an argument to `min()`/`max()`/`clamp()`, not just at the
        // top level or as its own basis.
        for css in [
            "min(calc-size(auto, size + 20px), 300px)",
            "max(calc-size(min-content, size), 10px)",
            "clamp(10px, calc-size(auto, size), 500px)",
            "clamp(calc-size(auto, size - 5px), 200px, calc-size(max-content, size + 5px))",
            "min(10px, max(calc-size(fit-content, size), 20px))",
        ] {
            assert!(
                matches!(parse("width", css), Some(ComputedValue::LengthPercentageExpression(_))),
                "{css}"
            );
        }
        // The nested basis keyword still has to be a valid one, and `any`
        // still forbids `size` inside the nested calc-size.
        for invalid in [
            "min(calc-size(bogus, size), 10px)",
            "max(calc-size(any, size), 10px)",
        ] {
            assert!(parse("width", invalid).is_none(), "{invalid}");
        }
    }
    #[test]
    fn relative_currentcolor_rebinds_on_inheritance() {
        let parent = style(
            None,
            &[
                ("color", "red"),
                ("background-color", "rgb(from currentcolor r g b)"),
            ],
        );
        assert_eq!(parent.get("background-color"), Some("#ff0000ff"));
        // The child's explicit `inherit` re-binds `currentcolor` to the child's
        // own color, so the retained relative source is re-resolved.
        let child = style(
            Some(&parent),
            &[("color", "blue"), ("background-color", "inherit")],
        );
        assert_eq!(child.get("background-color"), Some("#0000ffff"));
        assert!(matches!(
            parse("color", "rgb(from currentcolor r g b)"),
            Some(ComputedValue::Unparsed)
        ));
    }
    #[test]
    fn relative_currentcolor_color_space_rebinds_on_inheritance() {
        // CSS-22: general `color(from currentcolor <space> ...)` forms (not
        // just the identity `r g b` copy) resolve through the same
        // computed-value seam as the other relative-color functions.
        let parent = style(
            None,
            &[
                ("color", "red"),
                ("background-color", "color(from currentcolor srgb calc(r / 2) g b)"),
            ],
        );
        assert_eq!(parent.get("background-color"), Some("#800000ff"));
        let child = style(
            Some(&parent),
            &[("color", "blue"), ("background-color", "inherit")],
        );
        assert_eq!(child.get("background-color"), Some("#0000ffff"));
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
                match op {
                    Transform::Translate(x,y) => {resolve(x,font,root,false);resolve(y,font,root,false);}
                    Transform::Translate3D(x,y,z) => {resolve(x,font,root,false);resolve(y,font,root,false);resolve(z,font,root,false);}
                    Transform::TranslateExpression(parts) => {for part in parts {*part=crate::computed::individual_transforms::length(part,font,root);}}
                    Transform::Perspective(Some(value)) => {*value=crate::computed::individual_transforms::length(value,font,root);}
                    _=>{}
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
