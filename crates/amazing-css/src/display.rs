//! CSS Display 3 §2.7–2.8 contextual computed-value transformations.
use crate::ComputedStyle;

/// Block equivalent of a supported display value. Unknown values are preserved.
/// Layout may use this for synthetic styles without an element cascade context.
pub fn blockified_display(display: &str) -> String {
    let lower = display.trim().to_ascii_lowercase();
    let css = crate::values::display_canonical(&lower).unwrap_or(&lower);
    match css {
        "run-in" | "run-in flow-root" | "inline" | "inline-block" | "table-row" | "table-cell" | "table-row-group"
        | "table-header-group" | "table-footer-group" | "table-column"
        | "table-column-group" | "table-caption" | "ruby-base" | "ruby-text"
        | "ruby-base-container" | "ruby-text-container" => "block".into(),
        "inline list-item" | "run-in list-item" | "inline flow-root list-item"
        | "run-in flow-root list-item" => "list-item".into(),
        "inline-flex" | "run-in flex" => "flex".into(),
        "inline-grid" | "run-in grid" => "grid".into(),
        "inline-grid-lanes" | "run-in grid-lanes" => "grid-lanes".into(),
        "inline-table" | "run-in table" => "table".into(),
        "ruby" | "run-in ruby" => "block ruby".into(),
        _ => css.into(),
    }
}

/// Whether a valid display value carries the list-item marker flag.
pub fn is_list_item_display(display: &str) -> bool {
    crate::values::display_canonical(display).is_some_and(|canonical|
        canonical.split_ascii_whitespace().any(|word| word == "list-item"))
}

pub(crate) fn normalize(style: &mut ComputedStyle, parent: Option<&ComputedStyle>, root: bool) {
    let original = style.get("display").unwrap_or("inline").to_owned();
    style.display_before_blockification = Some(original.clone());
    let parent_display = parent.and_then(|parent| {
        if parent.get("display") == Some("contents") { parent.contents_parent_display.as_deref() }
        else { parent.get("display") }
    });
    let positioned = matches!(style.get("position"), Some("absolute" | "fixed"));
    let floating = !matches!(style.get("float"), None | Some("none"));
    let item = matches!(parent_display, Some("flex" | "inline-flex" | "grid" | "inline-grid" | "grid-lanes" | "inline-grid-lanes"));
    if positioned { style.set_contextual_value("float", "none"); }
    if root && original == "contents" { style.set_contextual_value("display", "block"); }
    else if root || positioned || floating || item {
        let block = blockified_display(&original);
        if block != original { style.set_contextual_value("display", &block); }
    }
    style.contents_parent_display = if style.get("display") == Some("contents") {
        parent_display.map(str::to_owned)
    } else { None };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{compute, parse_stylesheet};
    use amazing_dom::{Dom, NodeIdx};

    #[test]
    fn blockification_preserves_inner_modes_and_box_suppression() {
        for (source, expected) in [("inline", "block"), ("inline flow-root", "block"),
            ("inline flex", "flex"), ("inline-grid", "grid"), ("inline-table", "table"),
            ("table-cell", "block"), ("ruby-text", "block"), ("ruby", "block ruby"),
            ("flow-root", "flow-root"), ("none", "none"), ("contents", "contents")] {
            assert_eq!(blockified_display(source), expected);
        }
    }

    #[test]
    fn root_position_float_and_generated_parent_contexts() {
        for parent_display in ["block", "flex", "inline-flex", "grid", "inline-grid"] {
            for declaration in ["display:inline", "display:inline-flex", "display:none", "display:contents",
                "display:inline;float:left", "display:inline-table;position:absolute;float:right",
                "display:inline-block;position:fixed;float:inline-start"] {
                let mut dom = Dom::new();
                let html = dom.create_element(NodeIdx(0), "html");
                let parent = dom.create_element(html, "section");
                let contents1 = dom.create_element(parent, "nav");
                let contents2 = dom.create_element(contents1, "nav");
                let child = dom.create_element(contents2, "span");
                let sheet = parse_stylesheet(&format!("html{{display:contents}}section{{display:{parent_display}}}nav{{display:contents}}span{{{declaration}}}"));
                let styles = compute(&dom, &[sheet]);
                assert_eq!(styles.get(html).unwrap().get("display"), Some("block"));
                let style = styles.get(child).unwrap();
                let original = declaration.split(';').next().unwrap().strip_prefix("display:").unwrap();
                let block = parent_display != "block" || declaration.contains("float:");
                let expected = if block { blockified_display(original) } else { original.into() };
                assert_eq!(style.get("display"), Some(expected.as_str()), "{parent_display} {declaration}");
                assert_eq!(style.display_before_blockification(), Some(original));
                if declaration.contains("position:") { assert_eq!(style.get("float"), Some("none")); }
            }
        }
    }

    #[test]
    fn incremental_parent_changes_refresh_flattened_children() {
        use crate::{StyleInvalidator, AttributeStyleUpdate};
        let mut dom = Dom::new();
        let html = dom.create_element(NodeIdx(0), "html");
        let parent = dom.create_element(html, "section");
        let contents = dom.create_element(parent, "nav");
        let child = dom.create_element(contents, "span");
        let sheets = [parse_stylesheet("section{display:block}section.flex{display:flex}nav{display:contents}span{display:inline-flex}")];
        let invalidator = StyleInvalidator::new(&sheets, 1);
        let mut previous = compute(&dom, &sheets);
        for (old, new, expected) in [(None, Some("flex"), "flex"), (Some("flex"), None, "inline-flex")] {
            dom.set_attrs(parent, new.map(|value| vec![("class".into(), value.into())]).unwrap_or_default());
            let AttributeStyleUpdate::Recomputed(delta) = invalidator.update_attribute(&dom, &previous, parent, "class", old, new, 1)
                else { panic!("display requires subtree computation") };
            assert_eq!(delta.styles.get(child).unwrap().get("display"), Some(expected));
            assert_eq!(delta.styles.get(child).unwrap().display_before_blockification(), Some("inline-flex"));
            let full = compute(&dom, &sheets);
            for node in [html, parent, contents, child] {
                assert_eq!(delta.styles.get(node), full.get(node));
            }
            previous = delta.styles;
        }
    }


    #[test]
    fn generated_children_blockify_without_treating_them_as_roots() {
        use crate::selectors::PseudoElement;
        let mut dom = Dom::new();
        let root = dom.create_element(NodeIdx(0), "div");
        let styles = compute(&dom, &[parse_stylesheet("div{display:flex}div::before{content:'x';display:inline-table}div::after{content:'y';display:inline;position:absolute;float:left}")]);
        let before = styles.get_pseudo(root, PseudoElement::Before).unwrap();
        assert_eq!(before.get("display"), Some("table"));
        assert_eq!(before.display_before_blockification(), Some("inline-table"));
        let after = styles.get_pseudo(root, PseudoElement::After).unwrap();
        assert_eq!(after.get("display"), Some("block"));
        assert_eq!(after.get("float"), Some("none"));
        assert_eq!(after.display_before_blockification(), Some("inline"));
    }

}
