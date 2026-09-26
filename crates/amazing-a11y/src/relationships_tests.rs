//! Relationship and collection fixtures: `aria-owns` reparenting,
//! `aria-activedescendant`, set position, table addressing, `aria-relevant`
//! and the focus chain.

use crate::Role;
use crate::test_support::{ax_of_id, document, document_with_focus};

#[test]
fn aria_owns_reparents_in_listed_order() {
    let doc = document(
        "<div role=\"group\" id=\"owner\" aria-owns=\"a b\"></div>\
         <button id=\"a\">A</button><button id=\"b\">B</button>",
    );
    let owner = ax_of_id(&doc, "owner").unwrap();
    assert_eq!(owner.children.len(), 2, "owned children not attached");
    let a = ax_of_id(&doc, "a").unwrap();
    let b = ax_of_id(&doc, "b").unwrap();
    assert_eq!(a.parent, Some(owner.id));
    assert_eq!(b.parent, Some(owner.id));
    assert_eq!(owner.children, vec![a.id, b.id]);
    // The moved nodes are emitted exactly once (the button and its text node
    // are separate nodes, so count by role).
    let buttons = doc
        .tree
        .nodes()
        .iter()
        .filter(|n| n.role == Role::Button)
        .count();
    assert_eq!(buttons, 2, "owned buttons duplicated");
    assert_eq!(
        doc.tree
            .nodes()
            .iter()
            .filter(|n| n.role == Role::StaticText && n.name == "A")
            .count(),
        1,
        "owned text duplicated"
    );
}

#[test]
fn aria_owns_first_owner_wins() {
    let doc = document(
        "<div role=\"group\" id=\"o1\" aria-owns=\"x\"></div>\
         <div role=\"group\" id=\"o2\" aria-owns=\"x\"></div>\
         <button id=\"x\">X</button>",
    );
    let o1 = ax_of_id(&doc, "o1").unwrap();
    let o2 = ax_of_id(&doc, "o2").unwrap();
    let x = ax_of_id(&doc, "x").unwrap();
    assert_eq!(x.parent, Some(o1.id));
    assert!(!o2.children.contains(&x.id));
}

#[test]
fn aria_owns_cycles_are_ignored() {
    let doc = document(
        "<div role=\"group\" id=\"target\">\
           <div role=\"group\" id=\"owner\" aria-owns=\"target\"></div>\
         </div>",
    );
    let target = ax_of_id(&doc, "target").unwrap();
    let owner = ax_of_id(&doc, "owner").unwrap();
    assert_eq!(owner.parent, Some(target.id));
    assert!(target.children.contains(&owner.id));
}

#[test]
fn aria_activedescendant_resolves_to_a_tree_node() {
    let doc = document(
        "<ul id=\"list\" role=\"listbox\" aria-activedescendant=\"opt2\">\
           <li id=\"opt1\">A</li><li id=\"opt2\">B</li>\
         </ul>",
    );
    let list = ax_of_id(&doc, "list").unwrap();
    let opt2 = ax_of_id(&doc, "opt2").unwrap();
    assert_eq!(list.active_descendant, Some(opt2.id));

    let snapshot = doc.tree.snapshot();
    let snapshot_list = snapshot
        .nodes
        .iter()
        .find(|node| node.name.is_empty() && node.role == Role::Listbox)
        .unwrap();
    assert_eq!(snapshot_list.active_descendant, Some(opt2.id));
}

#[test]
fn native_set_position_for_list_and_option() {
    let doc = document("<ul><li>a</li><li>b</li><li>c</li></ul>");
    let items: Vec<_> = doc
        .tree
        .nodes()
        .iter()
        .filter(|node| node.role == Role::Listitem)
        .collect();
    assert_eq!(items.len(), 3);
    assert_eq!(items[0].properties.pos_in_set, Some(1));
    assert_eq!(items[0].properties.set_size, Some(3));
    assert_eq!(items[2].properties.pos_in_set, Some(3));

    let doc = document("<select><option>a</option><option>b</option></select>");
    let options: Vec<_> = doc
        .tree
        .nodes()
        .iter()
        .filter(|node| node.role == Role::Option)
        .collect();
    assert_eq!(options[1].properties.pos_in_set, Some(2));
    assert_eq!(options[1].properties.set_size, Some(2));
}

#[test]
fn aria_set_position_overrides_native() {
    let doc = document(
        "<ul role=\"list\"><li aria-posinset=\"5\" aria-setsize=\"10\">x</li></ul>",
    );
    let item = doc
        .tree
        .nodes()
        .iter()
        .find(|node| node.role == Role::Listitem)
        .unwrap();
    assert_eq!(item.properties.pos_in_set, Some(5));
    assert_eq!(item.properties.set_size, Some(10));
}

#[test]
fn table_addressing_properties() {
    let doc = document(
        "<div role=\"grid\" aria-rowcount=\"10\" aria-colcount=\"4\">\
           <div role=\"row\" aria-rowindex=\"2\">\
             <div role=\"gridcell\" aria-colindex=\"3\" aria-colspan=\"2\" \
                  aria-rowspan=\"1\" aria-rowindextext=\"two\" aria-colindextext=\"three\">c</div>\
           </div>\
         </div>",
    );
    let grid = doc
        .tree
        .nodes()
        .iter()
        .find(|node| node.role == Role::Grid)
        .unwrap();
    assert_eq!(grid.properties.row_count, Some(10));
    assert_eq!(grid.properties.col_count, Some(4));

    let row = doc
        .tree
        .nodes()
        .iter()
        .find(|node| node.role == Role::Row)
        .unwrap();
    assert_eq!(row.properties.row_index, Some(2));

    let cell = doc
        .tree
        .nodes()
        .iter()
        .find(|node| node.role == Role::Gridcell)
        .unwrap();
    assert_eq!(cell.properties.col_index, Some(3));
    assert_eq!(cell.properties.col_span, Some(2));
    assert_eq!(cell.properties.row_span, Some(1));
    assert_eq!(cell.properties.row_index_text.as_deref(), Some("two"));
    assert_eq!(cell.properties.col_index_text.as_deref(), Some("three"));
}

#[test]
fn aria_relevant_parses_into_flags() {
    let doc = document(
        "<div role=\"log\" aria-live=\"polite\" aria-relevant=\"additions text\" \
         aria-atomic=\"true\" aria-busy=\"true\"></div>",
    );
    let log = doc
        .tree
        .nodes()
        .iter()
        .find(|node| node.role == Role::Log)
        .unwrap();
    assert_eq!(log.properties.live.as_deref(), Some("polite"));
    assert!(log.properties.relevant_flags.additions);
    assert!(log.properties.relevant_flags.text);
    assert!(!log.properties.relevant_flags.removals);
    assert!(log.properties.atomic);
    assert!(log.states.busy);

    let doc = document("<div role=\"log\" aria-relevant=\"all\"></div>");
    let log = doc
        .tree
        .nodes()
        .iter()
        .find(|node| node.role == Role::Log)
        .unwrap();
    assert!(log.properties.relevant_flags.all);
}

#[test]
fn focus_chain_reaches_the_focused_node() {
    let doc = document_with_focus("<div><button>Go</button></div>", "button");
    let chain = doc.tree.focus_chain();
    assert!(!chain.is_empty(), "no focus chain");
    assert_eq!(chain.first().copied(), doc.tree.root());
    let focused = *chain.last().unwrap();
    let node = doc.tree.node(focused).unwrap();
    assert_eq!(node.role, Role::Button);
    assert!(node.states.focused);

    // Every link is a real parent/child edge.
    for pair in chain.windows(2) {
        assert!(doc.tree.children(pair[0]).contains(&pair[1]));
    }
}

#[test]
fn focus_chain_is_empty_without_focus() {
    let doc = document("<button>Go</button>");
    assert!(doc.tree.focused().is_none());
    assert!(doc.tree.focus_chain().is_empty());
}

#[test]
fn resolved_relationships_map_id_lists_to_tree_nodes() {
    // Targets use `<p>` (role Paragraph) rather than a bare `<span>`/`<div>`:
    // an unnamed, non-focusable, non-ARIA generic is flattened out of the tree
    // by presentational pruning, so it could never be a resolvable target.
    let doc = document(
        "<button id=\"open\" aria-labelledby=\"l1 l2\" aria-describedby=\"d1\" \
         aria-details=\"more\" aria-controls=\"panel\" aria-flowto=\"next\">Open</button>\
         <p id=\"l1\">Left</p><p id=\"l2\">Right</p>\
         <p id=\"d1\">Desc</p><p id=\"more\">More</p>\
         <p id=\"panel\">Panel</p><p id=\"next\">Next</p>",
    );
    let open = ax_of_id(&doc, "open").unwrap();
    let l1 = ax_of_id(&doc, "l1").unwrap().id;
    let l2 = ax_of_id(&doc, "l2").unwrap().id;
    let d1 = ax_of_id(&doc, "d1").unwrap().id;
    let more = ax_of_id(&doc, "more").unwrap().id;
    let panel = ax_of_id(&doc, "panel").unwrap().id;
    let next = ax_of_id(&doc, "next").unwrap().id;
    assert_eq!(open.relations.labelledby, vec![l1, l2]);
    assert_eq!(open.relations.describedby, vec![d1]);
    assert_eq!(open.relations.details, vec![more]);
    assert_eq!(open.relations.controls, vec![panel]);
    assert_eq!(open.relations.flowto, vec![next]);

    let via_tree = doc.tree.relations(open.id).unwrap();
    assert_eq!(via_tree, &open.relations);
}

#[test]
fn resolved_errormessage_is_a_single_optional_target() {
    let doc = document(
        "<input id=\"field\" aria-errormessage=\"err\">\
         <p id=\"err\">Required</p>",
    );
    let field = ax_of_id(&doc, "field").unwrap();
    let err = ax_of_id(&doc, "err").unwrap().id;
    assert_eq!(field.relations.errormessage, Some(err));
}

#[test]
fn unresolved_relationship_ids_are_dropped() {
    let doc = document(
        "<button id=\"open\" aria-labelledby=\"missing\" \
         aria-errormessage=\"missing-too\">Open</button>",
    );
    let open = ax_of_id(&doc, "open").unwrap();
    assert!(open.relations.labelledby.is_empty());
    assert_eq!(open.relations.errormessage, None);
}

#[test]
fn resolved_owns_reflects_the_reparented_targets() {
    let doc = document(
        "<div role=\"group\" id=\"owner\" aria-owns=\"a b\"></div>\
         <button id=\"a\">A</button><button id=\"b\">B</button>",
    );
    let owner = ax_of_id(&doc, "owner").unwrap();
    let a = ax_of_id(&doc, "a").unwrap().id;
    let b = ax_of_id(&doc, "b").unwrap().id;
    assert_eq!(owner.relations.owns, vec![a, b]);
}
