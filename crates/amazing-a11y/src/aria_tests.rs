//! State and property computation fixtures.

use crate::aria::{InvalidState, Orientation, Tristate};
use crate::test_support::{ax_of_tag, document};

#[test]
fn native_and_aria_checked() {
    let doc = document("<input type=\"checkbox\" checked>");
    let input = ax_of_tag(&doc, "input").unwrap();
    assert_eq!(input.states.checked, Tristate::True);

    let doc = document("<input type=\"checkbox\">");
    assert_eq!(ax_of_tag(&doc, "input").unwrap().states.checked, Tristate::False);

    let doc = document("<div role=\"checkbox\" aria-checked=\"mixed\">x</div>");
    assert_eq!(ax_of_tag(&doc, "div").unwrap().states.checked, Tristate::Mixed);
}

#[test]
fn aria_overrides_native_disabled() {
    let doc = document("<button disabled>x</button>");
    assert!(ax_of_tag(&doc, "button").unwrap().states.disabled);

    let doc = document("<button aria-disabled=\"false\">x</button>");
    assert!(!ax_of_tag(&doc, "button").unwrap().states.disabled);

    // A disabled ancestor fieldset disables controls, except the legend.
    let doc = document("<fieldset disabled><legend>L</legend><input id=\"i\"></fieldset>");
    assert!(ax_of_tag(&doc, "input").unwrap().states.disabled);
}

#[test]
fn required_readonly_and_invalid() {
    let doc = document("<input required readonly aria-invalid=\"true\">");
    let input = ax_of_tag(&doc, "input").unwrap();
    assert!(input.states.required);
    assert!(input.states.readonly);
    assert_eq!(input.states.invalid, InvalidState::True);

    let doc = document("<input aria-invalid=\"spelling\">");
    assert_eq!(ax_of_tag(&doc, "input").unwrap().states.invalid, InvalidState::Spelling);
}

#[test]
fn selected_and_expanded() {
    let doc = document("<select><option selected>a</option><option>b</option></select>");
    let options: Vec<_> = doc
        .tree
        .nodes()
        .iter()
        .filter(|node| node.role == crate::Role::Option)
        .collect();
    assert!(options[0].states.selected);
    assert!(!options[1].states.selected);

    let doc = document("<details open><summary>More</summary><p>Body</p></details>");
    let summary = ax_of_tag(&doc, "summary").unwrap();
    assert_eq!(summary.states.expanded, Some(true));
    let details = ax_of_tag(&doc, "details").unwrap();
    assert_eq!(details.states.expanded, Some(true));
}

#[test]
fn focusable_and_editable() {
    assert!(ax_of_tag(&document("<button>x</button>"), "button").unwrap().states.focusable);
    assert!(ax_of_tag(&document("<a href=\"#\">x</a>"), "a").unwrap().states.focusable);
    assert!(
        !ax_of_tag(&document("<a role=\"link\">x</a>"), "a")
            .unwrap()
            .states
            .focusable
    );
    assert!(ax_of_tag(&document("<div tabindex=\"0\">x</div>"), "div").unwrap().states.focusable);
    assert!(
        ax_of_tag(&document("<textarea></textarea>"), "textarea")
            .unwrap()
            .states
            .editable
    );
    assert!(!ax_of_tag(&document("<input type=\"checkbox\">"), "input").unwrap().states.editable);
}

#[test]
fn hidden_and_inert_states() {
    // aria-hidden="true" removes the node, so test the state through a
    // descendant that is re-exposed.
    let doc = document("<div aria-hidden=\"true\"><span id=\"s\">x</span></div>");
    assert!(doc.tree.nodes().iter().all(|node| node.name != "x"));

    let doc = document("<div inert><button>x</button></div>");
    let button = ax_of_tag(&doc, "button").unwrap();
    assert!(button.states.inert);
}

#[test]
fn properties_are_typed_and_raw_is_preserved() {
    let doc = document(
        "<div role=\"slider\" aria-valuenow=\"5\" aria-valuemin=\"0\" aria-valuemax=\"10\" \
         aria-valuetext=\"five\" aria-orientation=\"vertical\" aria-roledescription=\"dial\" \
         aria-controls=\"a b\" aria-live=\"polite\"></div>",
    );
    let slider = ax_of_tag(&doc, "div").unwrap();
    assert_eq!(slider.properties.value_now, Some(5.0));
    assert_eq!(slider.properties.value_min, Some(0.0));
    assert_eq!(slider.properties.value_max, Some(10.0));
    assert_eq!(slider.properties.value_text.as_deref(), Some("five"));
    assert_eq!(slider.properties.orientation, Orientation::Vertical);
    assert_eq!(slider.properties.roledescription.as_deref(), Some("dial"));
    assert_eq!(slider.properties.controls, vec!["a", "b"]);
    assert_eq!(slider.properties.live.as_deref(), Some("polite"));
    assert_eq!(slider.properties.raw.get("aria-valuenow").map(String::as_str), Some("5"));
}

#[test]
fn placeholder_is_a_property_and_a_name() {
    let doc = document("<input placeholder=\"Search terms\">");
    let input = ax_of_tag(&doc, "input").unwrap();
    assert_eq!(input.name, "Search terms");
    assert_eq!(input.properties.placeholder.as_deref(), Some("Search terms"));
}

#[test]
fn boolean_and_tristate_states() {
    let doc = document(
        "<div role=\"button\" aria-pressed=\"true\" aria-expanded=\"false\" \
         aria-haspopup=\"menu\" aria-busy=\"true\"></div>",
    );
    let button = ax_of_tag(&doc, "div").unwrap();
    assert_eq!(button.states.pressed, Tristate::True);
    assert_eq!(button.states.expanded, Some(false));
    assert!(button.states.haspopup);
    assert!(button.states.busy);

    let doc = document("<ul role=\"listbox\" aria-multiselectable=\"true\"></ul>");
    let listbox = ax_of_tag(&doc, "ul").unwrap();
    assert!(listbox.states.multiselectable);
    // An ARIA listbox is a composite, not an editable field.
    assert!(!listbox.states.editable);
}

#[test]
fn invalid_variants_and_current() {
    let doc = document("<input aria-invalid=\"grammar\">");
    assert_eq!(ax_of_tag(&doc, "input").unwrap().states.invalid, InvalidState::Grammar);

    let doc = document("<div role=\"group\" aria-current=\"page\"></div>");
    assert_eq!(
        ax_of_tag(&doc, "div").unwrap().properties.current.as_deref(),
        Some("page")
    );
}

#[test]
fn aria_level_overrides_heading_tag() {
    let doc = document("<h2 aria-level=\"4\">Title</h2>");
    let heading = ax_of_tag(&doc, "h2").unwrap();
    assert_eq!(heading.properties.level, Some(4));
}

#[test]
fn dialog_modal_state() {
    let doc = document("<dialog aria-modal=\"true\">x</dialog>");
    assert!(ax_of_tag(&doc, "dialog").unwrap().states.modal);
}

