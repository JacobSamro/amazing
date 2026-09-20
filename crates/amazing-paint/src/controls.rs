//! Control payload supplied by the embedding/DOM/layout owners.
//! Paint never reads DOM attributes, chooses labels, shapes, or measures text.
use amazing_dom::NodeIdx;
use amazing_layout::LineBox;

/// State plus absolute, layout-positioned glyph lines for one control.
/// The provider must retain these lines for the duration of a paint call.
#[derive(Debug, Default)]
pub struct ControlContent<'a> {
    pub checked: bool,
    pub disabled: bool,
    pub lines: &'a [LineBox],
}

pub trait ControlProvider {
    fn content(&self, node: NodeIdx) -> ControlContent<'_>;
}

pub struct NoControls;
impl ControlProvider for NoControls {
    fn content(&self, _node: NodeIdx) -> ControlContent<'_> {
        ControlContent::default()
    }
}

pub(crate) struct ControlPaint {
    pub kind: amazing_layout::replaced::ReplacedKind,
    /// Decoration-only snapshot of geometry supplied by layout. No children,
    /// line measurement, or box construction policy is performed by paint.
    pub geometry: amazing_layout::LayoutBox,
    pub background: crate::background::BackgroundPlan,
}
