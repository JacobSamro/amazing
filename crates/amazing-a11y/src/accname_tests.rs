//! accname-1.2 accessible name and description fixtures.
//!
//! These are table-driven selections from the WPT `accname/` suite and the
//! HTML-AAM name tables, expressed directly against the crate's seam.

use crate::test_support::{ax_of_id, ax_of_tag, document, document_with_css, name_of_tag};

fn name(html: &str, tag: &str) -> String {
    name_of_tag(html, tag).unwrap_or_default()
}

#[test]
fn aria_label_beats_content() {
    assert_eq!(name("<button aria-label=\"Close\">X</button>", "button"), "Close");
    assert_eq!(name("<a href=\"#\" aria-label=\"Home\">x</a>", "a"), "Home");
}

#[test]
fn labelledby_beats_label() {
    assert_eq!(
        name(
            "<span id=\"l\">Save</span>\
             <button aria-labelledby=\"l\" aria-label=\"Ignore\">x</button>",
            "button"
        ),
        "Save"
    );
    assert_eq!(
        name(
            "<span id=\"a\">Hello</span><span id=\"b\">World</span>\
             <button aria-labelledby=\"a b\">x</button>",
            "button"
        ),
        "Hello World"
    );
    // Whitespace in the id list is collapsed and empty pieces ignored.
    assert_eq!(
        name("<span id=\"a\">One</span><button aria-labelledby=\"  a   \">x</button>", "button"),
        "One"
    );
}

#[test]
fn associated_label_element() {
    assert_eq!(
        name("<label for=\"i\">Name</label><input id=\"i\">", "input"),
        "Name"
    );
    assert_eq!(name("<label>Name <input></label>", "input"), "Name");
    assert_eq!(
        name("<label for=\"t\">Notes</label><textarea id=\"t\"></textarea>", "textarea"),
        "Notes"
    );
    assert_eq!(
        name(
            "<label for=\"s\">Pick</label><select id=\"s\"><option>a</option></select>",
            "select"
        ),
        "Pick"
    );
    // Two labels concatenate.
    assert_eq!(
        name(
            "<label for=\"i\">First</label><label for=\"i\">Last</label><input id=\"i\">",
            "input"
        ),
        "First Last"
    );
}

#[test]
fn host_language_alternatives() {
    assert_eq!(name("<img alt=\"Logo\">", "img"), "Logo");
    // No `alt` attribute: `title` is the name.
    assert_eq!(name("<img src=\"a.png\" title=\"Logo\">", "img"), "Logo");
    assert_eq!(
        name("<abbr title=\"HyperText Markup Language\">HTML</abbr>", "abbr"),
        "HyperText Markup Language"
    );
    assert_eq!(name("<input type=\"image\" alt=\"Search\">", "input"), "Search");
    assert_eq!(name("<input type=\"button\" value=\"Go\">", "input"), "Go");
    assert_eq!(name("<input type=\"submit\" value=\"Send\">", "input"), "Send");
    assert_eq!(name("<input type=\"submit\">", "input"), "Submit");
    assert_eq!(name("<input type=\"reset\">", "input"), "Reset");
    assert_eq!(name("<input placeholder=\"Search terms\">", "input"), "Search terms");
    assert_eq!(name("<textarea placeholder=\"Bio\"></textarea>", "textarea"), "Bio");
}

#[test]
fn explicit_empty_alt_removes_the_image() {
    let doc = document("<img src=\"a.png\" alt=\"\">");
    assert!(ax_of_tag(&doc, "img").is_none());
}

#[test]
fn structural_alternatives() {
    // A11Y-10: unlike `table`/`caption` and `fieldset`/`legend` below, a
    // `<figure>`'s own accessible name is NOT derived from its
    // `<figcaption>` — per HTML-AAM (confirmed by the pinned
    // `figure-name-no-figcaption.tentative.html`), only
    // `aria-label`/`aria-labelledby`/`title` name a figure; a bare figure
    // with only a figcaption and no such attribute has no name at all. (The
    // figcaption text is a separate, `img`-only fallback — see
    // `figure_figcaption_fallback` — not a mapping for the figure itself.)
    assert_eq!(name("<figure><figcaption>Fig 1</figcaption></figure>", "figure"), "");
    assert_eq!(
        name(
            "<figure aria-label=\"Fig 1\"><figcaption>caption text</figcaption></figure>",
            "figure"
        ),
        "Fig 1"
    );
    assert_eq!(
        name("<table><caption>Data</caption><tr><td>x</td></tr></table>", "table"),
        "Data"
    );
    assert_eq!(
        name("<fieldset><legend>Address</legend><input></fieldset>", "fieldset"),
        "Address"
    );
    assert_eq!(
        name("<svg><title>Icon</title><rect/></svg>", "svg"),
        "Icon"
    );
}

#[test]
fn name_from_content() {
    assert_eq!(name("<button>Save now</button>", "button"), "Save now");
    assert_eq!(name("<button>Save <b>now</b></button>", "button"), "Save now");
    assert_eq!(name("<a href=\"#\">Home</a>", "a"), "Home");
    assert_eq!(name("<h2>Section</h2>", "h2"), "Section");
    assert_eq!(name("<button><img alt=\"Save\"></button>", "button"), "Save");
    // Content wins over `aria-label` only when the label is absent/empty.
    assert_eq!(name("<button aria-label=\" \">Fallback</button>", "button"), "Fallback");
}

#[test]
fn title_is_a_last_resort_for_non_content_roles() {
    assert_eq!(name("<div role=\"group\" title=\"Go\"></div>", "div"), "Go");
    assert_eq!(
        name("<div role=\"img\" title=\"Chart\"></div>", "div"),
        "Chart"
    );
}

#[test]
fn hidden_content_is_excluded_but_referenced_hidden_is_included() {
    assert_eq!(
        name("<button>Visible<span hidden>Hidden</span></button>", "button"),
        "Visible"
    );
    assert_eq!(
        name(
            "<span id=\"l\" hidden>Save</span><button aria-labelledby=\"l\">x</button>",
            "button"
        ),
        "Save"
    );
}

#[test]
fn cycles_are_broken() {
    let doc = document(
        "<button id=\"a\" aria-labelledby=\"b\">A</button>\
         <button id=\"b\" aria-labelledby=\"a\">B</button>",
    );
    // `a` names from `b`, whose own labelledby back to `a` is cut by the
    // visited set, leaving `b`'s content as its name.
    assert_eq!(ax_of_id(&doc, "a").unwrap().name, "B");
}

#[test]
fn description_precedence() {
    let doc = document(
        "<button aria-describedby=\"d\" title=\"tip\">Go</button>\
         <span id=\"d\">More info</span>",
    );
    let button = ax_of_tag(&doc, "button").unwrap();
    assert_eq!(button.name, "Go");
    assert_eq!(button.description, "More info");

    let doc = document("<button aria-description=\"Explain\" title=\"tip\">Go</button>");
    assert_eq!(ax_of_tag(&doc, "button").unwrap().description, "Explain");

    // `title` becomes the description when it differs from the name.
    let doc = document("<button title=\"tip\">Go</button>");
    assert_eq!(ax_of_tag(&doc, "button").unwrap().description, "tip");

    // ...but not when the title *is* the name.
    let doc = document("<div role=\"group\" title=\"tip\"></div>");
    assert!(ax_of_tag(&doc, "div").unwrap().description.is_empty());
}

#[test]
fn css_generated_content_contributes() {
    let doc = document_with_css(
        "<button class=\"badge\">Save</button>",
        ".badge::before{content:\"New \"}",
    );
    assert_eq!(ax_of_tag(&doc, "button").unwrap().name, "New Save");
}

#[test]
fn describedby_can_reference_hidden_content() {
    let doc = document(
        "<button aria-describedby=\"d\">Go</button>\
         <span id=\"d\" hidden>Hidden help</span>",
    );
    let button = ax_of_tag(&doc, "button").unwrap();
    assert_eq!(button.name, "Go");
    assert_eq!(button.description, "Hidden help");
}

#[test]
fn labelledby_chains_through_references() {
    let doc = document(
        "<span id=\"c\">Deep</span>\
         <span id=\"b\" aria-labelledby=\"c\"></span>\
         <button aria-labelledby=\"b\">x</button>",
    );
    assert_eq!(ax_of_tag(&doc, "button").unwrap().name, "Deep");
}

#[test]
fn whitespace_in_names_is_flattened() {
    assert_eq!(
        name("<button>  Save\n\t  now  </button>", "button"),
        "Save now"
    );
    assert_eq!(
        name("<div role=\"group\" aria-label=\"  Spaced   out  \"></div>", "div"),
        "Spaced out"
    );
}

#[test]
fn form_control_text_is_not_container_content() {
    // An input's own text/value is not folded into a wrapping role=button name.
    assert_eq!(
        name("<button><input value=\"ignored\">Save</button>", "button"),
        "Save"
    );
}

/// A11Y-15: an embedded `<textarea>`'s value substitution must read its live
/// text (`Dom::textarea_value`) — before this ticket, `control_value` had no
/// `<textarea>` branch at all, so a labelled textarea contributed nothing.
/// Uses `aria-labelledby` to a `<label>` from a *third*, unrelated element
/// (rather than implicit `<label>` wrapping) so the textarea's own value
/// isn't subject to the self-reference exclusion a control's *own* name
/// computation applies to its associated label — this exercises
/// `collect_content`'s `embed_control_values: true` path for a `<label>`
/// directly, the same one HTML's real label-association step uses for a
/// *different* labelled control than the one carrying the value.
#[test]
fn embedded_textarea_value_substitution_reads_live_text() {
    use crate::test_support::{ax_of_tag, dom_of_tag, document, rebuild};

    let mut doc = document(
        "<label id=\"lbl\">Bio <textarea>original</textarea></label>\
         <div aria-labelledby=\"lbl\">x</div>",
    );
    let name = ax_of_tag(&doc, "div").unwrap().name.clone();
    assert!(name.contains("original"), "expected the textarea's parsed text in the name, got {name:?}");

    let node = dom_of_tag(&doc, "textarea").unwrap();
    doc.dom.set_textarea_value(node, "updated".encode_utf16().collect()).unwrap();
    rebuild(&mut doc);

    let name = ax_of_tag(&doc, "div").unwrap().name.clone();
    assert!(name.contains("updated"), "expected the live textarea value in the name, got {name:?}");
    assert!(!name.contains("original"), "the stale parsed text must not still show, got {name:?}");
}

/// A11Y-15: an embedded `<select>`'s value substitution must read its live
/// selectedness (`Dom::option_selected`, via `selected_native_option_text`)
/// rather than the `selected` content attribute — picking a different
/// option never touches the attribute. See the textarea test above for why
/// `aria-labelledby` from a third element is used instead of implicit
/// `<label>` wrapping.
#[test]
fn embedded_select_value_substitution_reads_live_selection() {
    use crate::test_support::{ax_of_tag, document, rebuild};
    use amazing_dom::NodeIdx;

    let mut doc = document(
        "<label id=\"lbl\">Choice <select><option>A</option><option>B</option></select></label>\
         <div aria-labelledby=\"lbl\">x</div>",
    );
    let name = ax_of_tag(&doc, "div").unwrap().name.clone();
    assert!(name.contains('A'), "HTML default: first option selected, got {name:?}");

    let option_b = doc
        .dom
        .tree_order(NodeIdx(0))
        .into_iter()
        .find(|n| {
            doc.dom.name(*n) == Some("option")
                && crate::host::child_text(&doc.dom, *n).trim() == "B"
        })
        .unwrap();
    doc.dom.set_option_selected(option_b, true).unwrap();
    rebuild(&mut doc);

    let name = ax_of_tag(&doc, "div").unwrap().name.clone();
    assert!(name.contains('B'), "expected the live selection in the name, got {name:?}");
}
