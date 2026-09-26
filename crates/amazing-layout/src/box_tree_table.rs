//! Preserve misparented table siblings before inline flattening discards their
//! inline parent. CSS2 §17.2.1 requires an inline-table at this boundary.
use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) fn orphan_run(
    dom: &Dom,
    styled: &StyledDom,
    entries: &[FormattingChild],
    start: usize,
    parent_style: &ComputedStyle,
    font_size: f32,
    fonts: &FontSource,
    replaced: &dyn ReplacedElementProvider,
) -> Option<(usize, LayoutBox)> {
    let internal = |entry: &FormattingChild| {
        let Some(style) = styled.get(entry.node) else {
            return false;
        };
        !is_out_of_flow(style)
            && crate::floats::side(style).is_none()
            && !matches!(
                crate::table_tree::Role::of(style),
                crate::table_tree::Role::Other | crate::table_tree::Role::Table
            )
    };
    if !internal(&entries[start]) {
        return None;
    }
    let mut children = Vec::new();
    let mut end = start;
    for (offset, entry) in entries[start..].iter().enumerate() {
        if internal(entry) {
            children.push(build_element(
                dom,
                styled,
                entry.node,
                entry.parent_font,
                fonts,
                replaced,
            ));
            end = start + offset + 1;
        } else if matches!(&dom.get(entry.node).kind,
            NodeKind::Text { data } | NodeKind::CDATASection { data }
                if data.chars().all(|c| matches!(c, ' ' | '\t' | '\r' | '\n' | '\u{000c}')))
            || matches!(
                dom.get(entry.node).kind,
                NodeKind::Comment { .. } | NodeKind::ProcessingInstruction { .. }
            )
        {
            // Only whitespace between table siblings disappears. Keep the
            // trailing run for the surrounding inline formatting context.
        } else {
            break;
        }
    }
    let line_height = values::resolve_used_line_height(
        parent_style,
        font_size,
        fonts.scaled_metrics(font_size).line_height,
    );
    let parent = LayoutBox::new(
        None,
        Display::Inline,
        parent_style.clone(),
        font_size,
        line_height,
    );
    let mut table = crate::table_tree::anonymous(&parent, crate::table_tree::Role::Table, children);
    table.style.set("display", "inline-table");
    // Inline placeholders identify an atomic box by a source node. The first
    // table descendant gives this otherwise anonymous wrapper a stable key.
    table.node = Some(entries[start].node);
    Some((end, table))
}
