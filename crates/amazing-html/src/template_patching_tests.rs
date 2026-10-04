//! HTML content patching: actual parser entrypoints and insertion checkpoints.
use amazing_dom::{Dom, NodeIdx};
use crate::{parse, parse_fragment, IncrementalParser};

fn children(dom: &Dom, node: NodeIdx) -> String {
    String::from_utf16(&dom.serialize_children(node).unwrap()).unwrap()
}
fn body(dom: &Dom) -> String {
    children(dom, dom.document_body(NodeIdx(0)).unwrap().unwrap())
}
fn compare_entries(source: &str, expected: &str) {
    let input = format!("<!doctype html><body>{source}");
    let dom = parse(&input);
    assert_eq!(body(&dom), expected, "document: {source}");
    let fragment = parse_fragment("div", source).unwrap();
    assert_eq!(children(&fragment, NodeIdx(0)), expected, "fragment: {source}");
    for split in input.char_indices().map(|(n, _)| n).chain(std::iter::once(input.len())) {
        let mut parser = IncrementalParser::new();
        parser.push_chunk(&input[..split]);
        parser.push_chunk(&input[split..]);
        assert_eq!(body(&parser.finish()), expected, "split {split}: {source}");
    }
}

#[test]
fn patch_markers_replace_sibling_ranges_and_insert_before_live_end() {
    for (source, expected) in [
        ("<section><?start name='x'>old<b>more</b><?end>tail</section><template for=x>new<i>rich</i></template>",
         "<section>new<i>rich</i>tail</section>"),
        ("<section><?start name='x'>old<b>more</b></section><template for=x>new</template>",
         "<section>new</section>"),
        ("<section>a<?marker name='x'>z</section><template for=x><b>b</b></template>",
         "<section>a<b>b</b>z</section>"),
        ("<section><?start name='x'>old<?end>tail</section><template for=x></template>",
         "<section>tail</section>"),
        ("<section>a<?marker name='x'>z</section><template for=x></template>",
         "<section>az</section>"),
    ] { compare_entries(source, expected); }
}

#[test]
fn patch_marker_search_uses_first_exact_name_and_actual_target() {
    for (source, expected) in [
        ("<div><?other name='x'>a<?end name='x'>b<?marker name='x'>c</div><template for=x>p</template>",
         "<div><?other name='x'?>a<?end name='x'?>bpc</div>"),
        ("<div><?marker name='x'>a<?marker name='x'>b</div><template for=x>p</template>",
         "<div>pa<?marker name='x'?>b</div>"),
        ("<div><?marker name='X'>a<?marker name='x'>b</div><template for=x>p</template>",
         "<div><?marker name='X'?>apb</div>"),
        ("<div><?marker name='a&amp;b'>z</div><template for='a&amp;b'>p</template>",
         "<div>pz</div>"),
        ("<div><?marker name='名字'>z</div><template for='名字'>p</template>",
         "<div>pz</div>"),
    ] { compare_entries(source, expected); }
}

#[test]
fn patch_start_nesting_counts_sibling_targets_independently_of_names() {
    for (source, expected) in [
        ("<div><?start name='x'>a<?start name='y'>b<?end name='ignored'>c<?end>d</div><template for=x>p</template>",
         "<div>pd</div>"),
        ("<div><?start name='x'>a<?start name='y'>b<?end name='ignored'>c<?end>d</div><template for=y>p</template>",
         "<div><?start name='x'?>apc<?end ?>d</div>"),
        ("<div><?start name='x'>a<span><?start name='y'>b<?end></span>c<?end>d</div><template for=x>p</template>",
         "<div>pd</div>"),
    ] { compare_entries(source, expected); }
}

#[test]
fn patch_close_uses_last_template_even_with_unclosed_children() {
    for (source, expected) in [
        ("<div><?start name='x'>old<?end>tail</div><template for=x><span>new</template>",
         "<div><span>new</span>tail</div>"),
        ("<div><?marker name='x'>tail</div><template for=x><b>new</template>",
         "<div><b>new</b>tail</div>"),
        ("<div><?start name='x'>old<?end>tail</div><template for=x>new",
         "<div>newtail</div>"),
        ("<div><?marker name='x'>tail</div><template for=x><b>new",
         "<div><b>new</b>tail</div>"),
    ] { compare_entries(source, expected); }
}

#[test]
fn invalid_patch_names_preserve_the_same_ordinary_template() {
    for (source, expected) in [
        ("<div><?marker name='x'>a</div><template for=''>b</template>",
         "<div><?marker name='x'?>a</div><template for=\"\">b</template>"),
        ("<div><?marker name='x'>a</div><template for=y>b</template>",
         "<div><?marker name='x'?>a</div><template for=\"y\">b</template>"),
        ("<div><?marker name='x'>a</div><template>b</template>",
         "<div><?marker name='x'?>a</div><template>b</template>"),
        ("<div><?marker name=x>a</div><template for=x>b</template>",
         "<div><?marker name=x?>a</div><template for=\"x\">b</template>"),
    ] { compare_entries(source, expected); }
}

#[test]
fn patching_inside_ordinary_templates_preserves_the_actual_inert_contents() {
    let dom = parse("<body><template id=outer><div><?marker name='x'>a</div><template for=x>b</template></template>");
    let outer = dom.tree_order(NodeIdx(0)).into_iter()
        .find(|node| dom.get_attribute_ns(*node, None, "id") == Some("outer")).unwrap();
    let contents = dom.template_contents(outer).unwrap();
    assert_eq!(children(&dom, contents), "<div>ba</div>");
    assert_eq!(dom.children(outer).count(), 0);
    for child in dom.tree_order(contents) {
        assert_eq!(dom.owner_document(child), dom.owner_document(contents));
    }
}

#[test]
fn patch_insertions_share_element_text_comment_and_instruction_positions() {
    compare_entries(
        "<div><?start name='x'>old<?end>tail</div><template for=x>text<!--c--><?other name='z'><b>bold</b></template>",
        "<div>text<!--c--><?other name='z'?><b>bold</b>tail</div>",
    );
}

#[test]
fn patch_scope_excludes_nested_template_contents() {
    compare_entries(
        "<template><div><?marker name='x'>inner</div></template><div><?marker name='x'>outer</div><template for=x>patch</template>",
        "<template><div><?marker name='x'?>inner</div></template><div>patchouter</div>",
    );
}

#[test]
fn head_patching_uses_head_scope_and_body_patching_can_find_head_markers() {
    let dom = parse("<!doctype html><head><?marker name='x'><template for=x><meta name=patched></template></head><body>body");
    let head = dom.tree_order(NodeIdx(0)).into_iter().find(|node| dom.name(*node) == Some("head")).unwrap();
    assert_eq!(children(&dom, head), "<meta name=\"patched\">");
    let dom = parse("<!doctype html><head><?marker name='x'></head><body><template for=x><!--patched--></template>body");
    let head = dom.tree_order(NodeIdx(0)).into_iter().find(|node| dom.name(*node) == Some("head")).unwrap();
    assert_eq!(children(&dom, head), "<!--patched-->");
    assert_eq!(body(&dom), "body");
}

#[test]
fn shadow_template_precedence_and_shadow_scope_remain_genuine() {
    let dom = parse("<body><div><template shadowrootmode=open><?marker name='x'><template for=x><b>patched</b></template></template></div><template for=x>outside</template>");
    let host = dom.tree_order(NodeIdx(0)).into_iter().find(|node| dom.name(*node) == Some("div")).unwrap();
    let root = dom.shadow_root_of_host(host).unwrap();
    assert_eq!(children(&dom, root), "<b>patched</b>");
    assert_eq!(body(&dom), "<div></div><template for=\"x\">outside</template>");
}
