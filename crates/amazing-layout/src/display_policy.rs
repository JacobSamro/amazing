//! Consume CSS Display outer/inner types independently of the marker flag.
//! CSS owns validation/canonicalization. Run-in sequence reparenting is a
//! separate box-tree step, not supplied by this type decomposition.
use amazing_css::{ComputedStyle, values::{ComputedValue, Display as CssDisplay}};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Outer { Block, Inline, RunIn }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Inner { Flow, FlowRoot, Flex, Grid, GridLanes, Table, Ruby }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BoxDisplay { pub outer: Outer, pub inner: Inner }

pub(crate) fn of(style: &ComputedStyle) -> Option<BoxDisplay> {
    use CssDisplay as D;
    use Inner as I;
    use Outer as O;
    let ComputedValue::Display(display) = style.typed("display")? else { return None };
    let (outer,inner) = match display {
        D::Block | D::ListItem => (O::Block,I::Flow),
        D::FlowRoot | D::FlowRootListItem => (O::Block,I::FlowRoot),
        D::Inline | D::InlineListItem => (O::Inline,I::Flow),
        D::InlineBlock | D::InlineFlowRootListItem => (O::Inline,I::FlowRoot),
        D::RunIn | D::RunInListItem => (O::RunIn,I::Flow),
        D::RunInFlowRoot | D::RunInFlowRootListItem => (O::RunIn,I::FlowRoot),
        D::Flex => (O::Block,I::Flex), D::InlineFlex => (O::Inline,I::Flex), D::RunInFlex => (O::RunIn,I::Flex),
        D::Grid => (O::Block,I::Grid), D::InlineGrid => (O::Inline,I::Grid), D::RunInGrid => (O::RunIn,I::Grid),
        D::GridLanes => (O::Block,I::GridLanes), D::InlineGridLanes => (O::Inline,I::GridLanes), D::RunInGridLanes => (O::RunIn,I::GridLanes),
        D::Table => (O::Block,I::Table), D::InlineTable => (O::Inline,I::Table), D::RunInTable => (O::RunIn,I::Table),
        D::BlockRuby => (O::Block,I::Ruby), D::Ruby => (O::Inline,I::Ruby), D::RunInRuby => (O::RunIn,I::Ruby),
        _ => return None, // absent boxes and layout-internal roles stay explicit
    };
    Some(BoxDisplay {outer,inner})
}

pub(crate) fn layout_display(style: &ComputedStyle) -> Option<crate::Display> {
    let display = of(style)?;
    Some(match display.inner {
        Inner::Flex => crate::Display::Flex,
        Inner::Grid | Inner::GridLanes => crate::Display::Grid,
        Inner::Flow if display.outer != Outer::Block => crate::Display::Inline,
        Inner::Ruby if display.outer != Outer::Block => crate::Display::Inline,
        _ => crate::Display::Block,
    })
}

pub(crate) fn atomic_inline(style: &ComputedStyle) -> bool {
    of(style).is_some_and(|display| display.outer != Outer::Block
        && !matches!(display.inner,Inner::Flow | Inner::Ruby))
}

pub(crate) fn non_atomic_inline(style: &ComputedStyle) -> bool {
    of(style).is_some_and(|display| display.outer != Outer::Block
        && matches!(display.inner,Inner::Flow | Inner::Ruby))
}

pub(crate) fn independent_inner(style: &ComputedStyle) -> bool {
    of(style).is_some_and(|display| matches!(display.inner,
        Inner::FlowRoot | Inner::Flex | Inner::Grid | Inner::GridLanes | Inner::Table))
}

pub(crate) fn inline_flow_root(style: &ComputedStyle) -> bool {
    of(style).is_some_and(|display| display.outer != Outer::Block && display.inner == Inner::FlowRoot)
}

pub(crate) fn inline_ruby(style: &ComputedStyle) -> bool {
    of(style).is_some_and(|display| display.outer != Outer::Block && display.inner == Inner::Ruby)
}

pub(crate) fn block_ruby(style: &ComputedStyle) -> bool {
    of(style).is_some_and(|display| display.outer == Outer::Block && display.inner == Inner::Ruby)
}

pub(crate) fn table(style: &ComputedStyle) -> bool {
    of(style).is_some_and(|display| display.inner == Inner::Table)
}

/// Metadata is the CSS-normalized original value before blockification.
pub(crate) fn originally_inline(css: &str) -> bool {
    css == "ruby" || css.starts_with("inline") || css.starts_with("run-in")
}
