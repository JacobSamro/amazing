//! State and property computation fixtures.

use crate::aria::{InvalidState, Orientation, Tristate, aria_mixin_attribute, aria_mixin_get, aria_mixin_set};
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

/// A11Y-15: `native_checked` must read the *live* checkedness
/// (`Dom::input_checked`, which a real click's activation toggles) rather
/// than the `checked` content attribute, which per spec only ever reflects
/// *default* checkedness from parse time. Simulates a real click's effect
/// with `Dom::set_input_checked` (the same call
/// `input_activation::prepare_input_activation` makes) directly, in both
/// directions, so a regression back to reading the attribute — which would
/// keep reporting whichever state the fixture parsed with, regardless of
/// this call — is caught either way.
#[test]
fn native_checked_reads_live_state_not_the_content_attribute() {
    use crate::test_support::{dom_of_tag, rebuild};

    let mut doc = document("<input type=\"checkbox\">");
    let node = dom_of_tag(&doc, "input").unwrap();
    assert_eq!(ax_of_tag(&doc, "input").unwrap().states.checked, Tristate::False);
    doc.dom.set_input_checked(node, true).unwrap();
    rebuild(&mut doc);
    assert_eq!(
        ax_of_tag(&doc, "input").unwrap().states.checked,
        Tristate::True,
        "a live check must be reported even though the `checked` attribute was never set"
    );

    let mut doc = document("<input type=\"checkbox\" checked>");
    let node = dom_of_tag(&doc, "input").unwrap();
    assert_eq!(ax_of_tag(&doc, "input").unwrap().states.checked, Tristate::True);
    doc.dom.set_input_checked(node, false).unwrap();
    rebuild(&mut doc);
    assert_eq!(
        ax_of_tag(&doc, "input").unwrap().states.checked,
        Tristate::False,
        "a live uncheck must be reported even though the `checked` attribute is still present"
    );
}

/// A11Y-15: a checkbox's live `.indeterminate` (IDL-only, no content
/// attribute at all) maps to `aria-checked="mixed"` when nothing explicit
/// overrides it, and an explicit `aria-checked` still wins over it.
#[test]
fn native_indeterminate_maps_to_mixed_checked() {
    use crate::test_support::{dom_of_tag, rebuild};

    let mut doc = document("<input type=\"checkbox\" checked>");
    let node = dom_of_tag(&doc, "input").unwrap();
    doc.dom.set_input_indeterminate(node, true).unwrap();
    rebuild(&mut doc);
    assert_eq!(ax_of_tag(&doc, "input").unwrap().states.checked, Tristate::Mixed);

    let mut doc = document("<input type=\"checkbox\" checked aria-checked=\"false\">");
    let node = dom_of_tag(&doc, "input").unwrap();
    doc.dom.set_input_indeterminate(node, true).unwrap();
    rebuild(&mut doc);
    assert_eq!(
        ax_of_tag(&doc, "input").unwrap().states.checked,
        Tristate::False,
        "an explicit aria-checked still overrides native indeterminate"
    );
}

/// A11Y-15: `native_selected` must read the *live* selectedness
/// (`Dom::option_selected`) rather than the `selected` content attribute —
/// picking a different option (`Dom::set_option_selected`, what a real
/// selection change calls) never touches the attribute.
#[test]
fn native_selected_reads_live_state_not_the_content_attribute() {
    use crate::test_support::rebuild;

    let mut doc = document("<select><option id=\"a\">A</option><option id=\"b\">B</option></select>");
    let a = crate::test_support::ax_of_id(&doc, "a").unwrap();
    let b = crate::test_support::ax_of_id(&doc, "b").unwrap();
    assert!(a.states.selected, "HTML default: first option is selected");
    assert!(!b.states.selected);

    let b_node = doc
        .dom
        .tree_order(amazing_dom::NodeIdx(0))
        .into_iter()
        .find(|n| doc.dom.get_attribute_ns(*n, None, "id") == Some("b"))
        .unwrap();
    doc.dom.set_option_selected(b_node, true).unwrap();
    rebuild(&mut doc);
    let a = crate::test_support::ax_of_id(&doc, "a").unwrap();
    let b = crate::test_support::ax_of_id(&doc, "b").unwrap();
    assert!(!a.states.selected, "no `selected` attribute changed, but live selectedness moved off a");
    assert!(b.states.selected, "live selection change must be reported even with no `selected` attribute");
}

/// A11Y-15: `<input type=range>`'s ARIA `valuenow` fallback must read the
/// live value (`Dom::input_value_as_number`, dirty-value-flag-aware) rather
/// than the `value` content attribute, which stays at its parse-time
/// default once a real interaction (a drag, `stepUp()`, `.valueAsNumber =`)
/// changes the live value without touching the attribute.
#[test]
fn native_range_valuenow_reads_live_value_not_the_content_attribute() {
    use crate::test_support::{dom_of_tag, rebuild};

    let mut doc = document("<input type=\"range\" min=\"0\" max=\"10\" value=\"5\">");
    let node = dom_of_tag(&doc, "input").unwrap();
    assert_eq!(ax_of_tag(&doc, "input").unwrap().properties.value_now, Some(5.0));
    doc.dom.set_input_value_as_number(node, 8.0).unwrap();
    rebuild(&mut doc);
    assert_eq!(
        ax_of_tag(&doc, "input").unwrap().properties.value_now,
        Some(8.0),
        "the live value must be reported even though the `value` attribute is still \"5\""
    );
}

/// A11Y-15: `<progress>`/`<meter>` have no separate live/dirty value state
/// (setting `.value` reflects straight back to the attribute per spec), so
/// their native valuenow fallback stays a plain attribute read — confirmed
/// here so a future change doesn't accidentally require a live accessor
/// that doesn't exist for these two.
#[test]
fn native_progress_and_meter_valuenow_use_the_plain_attribute() {
    let doc = document("<progress value=\"3\" max=\"4\"></progress>");
    let progress = ax_of_tag(&doc, "progress").unwrap();
    assert_eq!(progress.properties.value_now, Some(3.0));
    assert_eq!(progress.properties.value_max, Some(4.0));

    let doc = document("<meter value=\"2\" min=\"0\" max=\"10\"></meter>");
    let meter = ax_of_tag(&doc, "meter").unwrap();
    assert_eq!(meter.properties.value_now, Some(2.0));
    assert_eq!(meter.properties.value_max, Some(10.0));
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

/// A11Y-14: `aria_mixin_attribute`'s mechanical IDL-name → content-attribute
/// mapping, spot-checked against `ARIAMixin`'s own members (see its doc
/// comment for why no hand-maintained list is needed instead).
#[test]
fn aria_mixin_attribute_name_mapping() {
    assert_eq!(aria_mixin_attribute("role").as_deref(), Some("role"));
    assert_eq!(aria_mixin_attribute("ariaLabel").as_deref(), Some("aria-label"));
    assert_eq!(aria_mixin_attribute("ariaChecked").as_deref(), Some("aria-checked"));
    assert_eq!(aria_mixin_attribute("ariaValueNow").as_deref(), Some("aria-value-now"));
    assert_eq!(
        aria_mixin_attribute("ariaColIndexText").as_deref(),
        Some("aria-col-index-text")
    );
    assert_eq!(
        aria_mixin_attribute("ariaBrailleRoleDescription").as_deref(),
        Some("aria-braille-role-description")
    );
    assert_eq!(aria_mixin_attribute("aria-label"), None, "kebab input is not an IDL name");
    assert_eq!(aria_mixin_attribute("tabIndex"), None, "not an ARIAMixin member at all");
    assert_eq!(aria_mixin_attribute("aria"), None, "bare prefix with nothing after it");
}

/// A11Y-14: the getter/setter halves are plain content-attribute reflection
/// — no accessibility computation, matching WebIDL's `DOMString?` reflection
/// exactly (present → `Some`, absent → `None`/removed).
#[test]
fn aria_mixin_get_and_set_reflect_the_content_attribute() {
    use crate::test_support::dom_of_tag;

    let mut doc = document("<div id=\"x\" aria-checked=\"true\"></div>");
    let node = dom_of_tag(&doc, "div").unwrap();
    let attr = aria_mixin_attribute("ariaChecked").unwrap();

    assert_eq!(aria_mixin_get(&doc.dom, node, &attr), Some("true"));

    aria_mixin_set(&mut doc.dom, node, &attr, Some("mixed")).unwrap();
    assert_eq!(aria_mixin_get(&doc.dom, node, &attr), Some("mixed"));

    aria_mixin_set(&mut doc.dom, node, &attr, None).unwrap();
    assert_eq!(aria_mixin_get(&doc.dom, node, &attr), None);
}

