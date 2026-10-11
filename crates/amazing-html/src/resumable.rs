//! HTML-31: a script-interleaved, pausable HTML parser.
//!
//! [`parse`](crate::parse)/[`build`](crate::tree::build) remain full,
//! non-interactive parses for non-script consumers (fragments, offline
//! rendering, tests): they tokenize a whole document to completion before
//! returning, exactly as before. [`ResumableParser`] is the *other* entry
//! point, for a driver that actually runs a document's scripts: it exposes
//! the WHATWG "pause and run a parser-blocking script, then resume" cycle
//! (§13.2.6.2, the `<script>` end-tag steps) as an explicit `run`/`write`
//! loop instead of hiding it inside one opaque call.
//!
//! The whole document source is fed once (`push_source`); pausing is purely
//! about **script execution order**, not network chunking -- this engine
//! already has the complete decoded bytes before parsing starts. A future
//! streaming-fetch integration can still extend `push_source` to accept
//! successive chunks the same way [`crate::IncrementalParser::push_chunk`]
//! does, since the underlying tokenizer already tracks a stable cursor
//! either way.
//!
//! Node indices are stable across a pause: [`ResumableParser::swap_dom`]
//! physically swaps the same [`Dom`] value in and out (never copies it), so
//! a script that runs while "borrowing" the tree via `swap_dom` sees and can
//! mutate the exact arena the parser keeps building into once it gets the
//! tree back.

use amazing_dom::{Dom, NodeIdx};

use crate::tokenizer::Tokenizer;
use crate::tree::{self, ScriptMode, TreeBuilder};

/// What `ResumableParser::run` stopped for.
#[derive(Debug, PartialEq, Eq)]
pub enum ParseSignal {
    /// Tokenization is paused right after this classic, parser-blocking
    /// script's closing tag (`ScriptMode::Blocking`). The DOM built so far is
    /// reachable via `ResumableParser::swap_dom`; run the script against it
    /// (its own `document.write`/`writeln` calls should go through
    /// `ResumableParser::write`, not a direct DOM mutation, while paused),
    /// then call `run` again to resume.
    Script(NodeIdx),
    /// The document is fully parsed. `take_deferred_or_async` returns every
    /// non-blocking script discovered along the way, in document order.
    Eof,
    /// The embedding retired/replaced this exact active parser.
    Aborted,
}

/// A script-interleaved HTML parser: see the module docs.
pub struct ResumableParser {
    builder: Option<TreeBuilder<'static>>,
    creation_failure: Option<amazing_dom::DomError>,
    aborted: bool,
    tokenizer: Tokenizer,
    deferred_or_async: Vec<(NodeIdx, ScriptMode)>,
    write_inputs: Vec<(Vec<char>, bool)>,
    write_insertion_point: Option<usize>,
}

impl ResumableParser {
    /// `dom` is the (otherwise fresh, as from `Dom::new()`) arena to build
    /// into -- pass an existing arena a JS host already holds so the parser
    /// builds directly into it rather than a private tree the driver would
    /// have to graft in afterward.
    pub fn new(dom: Dom) -> Self {
        Self {
            builder: Some(TreeBuilder::from_dom(dom)),
            creation_failure: None,
            aborted: false,
            tokenizer: Tokenizer::new(Vec::new()),
            deferred_or_async: Vec::new(),
            write_inputs: Vec::new(),
            write_insertion_point: None,
        }
    }

    /// The embedding publishes a detached genuine tree frame while its
    /// insertion callbacks execute. A second publisher is rejected.
    pub fn publish_execution_frame(&mut self, frame: TreeBuilder<'static>) -> Result<(), amazing_dom::DomError> {
        if self.builder.is_some() { return Err(amazing_dom::DomError::InvalidStateError); }
        self.builder = Some(frame);
        Ok(())
    }
    pub fn reclaim_execution_frame(&mut self) -> Result<TreeBuilder<'static>, amazing_dom::DomError> {
        self.builder.take().ok_or(amazing_dom::DomError::InvalidStateError)
    }

    /// True only while the genuine tree frame is available for another entry.
    pub fn execution_frame_available(&self) -> bool { self.builder.is_some() }
    /// Called only when the embedding retires this actual active-parser entry.
    pub fn abort(&mut self) { self.aborted = true; }
    pub fn is_aborted(&self) -> bool { self.aborted }

    /// Configure a srcdoc navigation before feeding its source. Its document
    /// stays in no-quirks mode, including when the doctype is missing.
    pub fn set_srcdoc(&mut self) {
        self.builder.as_mut().expect("tree execution frame is borrowed").set_srcdoc();
    }

    /// Sets scripting before feeding source. Disabled mode never pauses for
    /// scripts or queues deferred/async script execution events.
    pub fn set_scripting_mode(&mut self, mode: crate::ScriptingMode) {
        self.builder.as_mut().expect("tree execution frame is borrowed").set_scripting_mode(mode);
    }

    /// Sets declarative shadow parsing before feeding input. An embedding
    /// reopening a Document passes that actual Document's preserved policy;
    /// scripting permission is independent. The normal parser default stays on.
    pub fn set_allow_declarative_shadow_roots(&mut self, allowed: bool) {
        self.builder.as_mut().expect("tree execution frame is borrowed").set_allow_declarative_shadow_roots(allowed);
    }

    /// Feeds the whole, already-decoded document source (newline-normalized
    /// exactly like `parse`). Call once, before the first `run`.
    pub fn push_source(&mut self, src: &str) {
        let normalized;
        let src = if src.contains('\r') {
            normalized = src.replace("\r\n", "\n").replace('\r', "\n");
            normalized.as_str()
        } else {
            src
        };
        self.tokenizer = Tokenizer::new(src.chars().collect());
    }

    /// Drives tokenizing/tree construction until a parser-blocking script is
    /// hit or the document reaches `Eof`. Every `defer`/`async`/module script
    /// crossed along the way (which never pauses) is queued; drain it with
    /// `take_deferred_or_async`.
    pub fn run(&mut self) -> ParseSignal {
        self.write_insertion_point = None;
        loop {
            if self.aborted { return ParseSignal::Aborted; }
            let tok = self.tokenizer.next_token();
            let eof = tree::feed_with_tokenizer_feedback(self.builder.as_mut().expect("tree execution frame is borrowed"), &mut self.tokenizer, tok).expect("unhosted parser construction");
            self.deferred_or_async.extend(self.builder.as_mut().expect("tree execution frame is borrowed").take_script_events());
            if let Some(node) = self.builder.as_mut().expect("tree execution frame is borrowed").take_pending_script() {
                self.write_insertion_point = Some(self.tokenizer.cursor());
                return ParseSignal::Script(node);
            }
            if eof {
                return ParseSignal::Eof;
            }
        }
    }

    /// Execute an actual tree frame outside the shared parser borrow. Authored
    /// constructors can reach the tokenizer's real insertion point, while the
    /// driver enforces its genuine dynamic-markup insertion counter.
    pub fn run_with_creation_host(parser: &std::rc::Rc<std::cell::RefCell<Self>>,
        host: &mut dyn crate::ParserElementCreationHost) -> Result<ParseSignal, amazing_dom::DomError> {
        if let Some(error) = parser.borrow().creation_failure { return Err(error); }
        if parser.borrow().aborted { return Ok(ParseSignal::Aborted); }
        parser.borrow_mut().write_insertion_point = None;
        let tree = parser.borrow_mut().builder.take().expect("parser tree frame already executing");
        let mut tree = tree.borrow_creation_host(host);
        let signal = loop {
            if parser.borrow().aborted { break Ok(ParseSignal::Aborted); }
            let token = parser.borrow_mut().tokenizer.next_token();
            let eof = token == crate::tokenizer::Token::Eof;
            let start_name = match &token { crate::tokenizer::Token::StartTag { name, .. } => Some(name.clone()), _ => None };
            if let Err(error) = tree.feed(token) { break Err(error); }
            if parser.borrow().aborted { break Ok(ParseSignal::Aborted); }
            {
                let mut shared = parser.borrow_mut();
                tree::finish_tokenizer_feedback(&tree, &mut shared.tokenizer, start_name);
                shared.deferred_or_async.extend(tree.take_script_events());
                if let Some(node) = tree.take_pending_script() {
                    shared.write_insertion_point = Some(shared.tokenizer.cursor());
                    break Ok(ParseSignal::Script(node));
                }
            }
            if eof { break Ok(ParseSignal::Eof); }
        };
        let mut shared = parser.borrow_mut();
        shared.builder = Some(tree.release_creation_host());
        if let Err(error) = signal { shared.creation_failure = Some(error); }
        signal
    }

    /// `document.write`/`writeln` while paused at a blocking script: per
    /// spec this inserts the string into the input stream just before the
    /// parser's current position, not a DOM mutation -- the written markup
    /// gets tokenized and built into the tree exactly as if it had been in
    /// the original source at this point, the next time `run` is called.
    pub fn write(&mut self, text: &str) {
        let normalized;
        let text = if text.contains('\r') {
            normalized = text.replace("\r\n", "\n").replace('\r', "\n");
            normalized.as_str()
        } else {
            text
        };
        self.tokenizer.insert_chars_at_cursor(text.chars());
    }

    /// Begins synchronous document.write tokenization, bounded by its input
    /// insertion point. Incomplete tokens remain pending for subsequent writes
    /// or the original source; exhausting this input does not signal EOF.
    pub fn begin_write(&mut self, text: &str) {
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        // A stored point belongs to the script that paused the parser. Once the
        // tokenizer has moved past it (a reaction or callback writing while the
        // parser runs), the point is the tokenizer's current position.
        let cursor = self.tokenizer.cursor();
        let point = self.write_insertion_point
            .filter(|point| *point >= cursor && *point <= self.tokenizer.buffered_input_scalars())
            .unwrap_or(cursor);
        self.write_inputs.push(self.tokenizer.begin_write_input(point, &text));
    }

    /// Returns a parser-blocking script, or None after all currently written
    /// input is consumed. The driver can execute a nested script and call this
    /// again; nested begin_write/run_write pairs preserve the outer boundary.
    pub fn run_write(&mut self) -> Option<NodeIdx> {
        assert!(!self.write_inputs.is_empty(), "no active document.write input");
        while let Some(token) = self.tokenizer.next_available() {
            tree::feed_with_tokenizer_feedback(self.builder.as_mut().expect("tree execution frame is borrowed"), &mut self.tokenizer, token).expect("unhosted written parser construction");
            self.deferred_or_async.extend(self.builder.as_mut().expect("tree execution frame is borrowed").take_script_events());
            if let Some(node) = self.builder.as_mut().expect("tree execution frame is borrowed").take_pending_script() {
                self.write_insertion_point = Some(self.tokenizer.cursor());
                return Some(node);
            }
        }
        self.end_write();
        None
    }

    /// The document.write counterpart of the scoped construction frame.
    /// A constructor cannot write into this Document while its real HTML
    /// dynamic-markup counter is positive, but other Documents remain usable.
    pub fn run_write_with_creation_host(parser: &std::rc::Rc<std::cell::RefCell<Self>>,
        host: &mut dyn crate::ParserElementCreationHost) -> Result<Option<NodeIdx>, amazing_dom::DomError> {
        if let Some(error) = parser.borrow().creation_failure { return Err(error); }
        let tree = parser.borrow_mut().builder.take().expect("parser tree frame already executing");
        let mut tree = tree.borrow_creation_host(host);
        let result = loop {
            if parser.borrow().aborted { break Ok(None); }
            let Some(token) = parser.borrow_mut().tokenizer.next_available() else { break Ok(None); };
            let start_name = match &token { crate::tokenizer::Token::StartTag { name, .. } => Some(name.clone()), _ => None };
            if let Err(error) = tree.feed(token) { break Err(error); }
            if parser.borrow().aborted { break Ok(None); }
            let mut shared = parser.borrow_mut();
            tree::finish_tokenizer_feedback(&tree, &mut shared.tokenizer, start_name);
            shared.deferred_or_async.extend(tree.take_script_events());
            if let Some(node) = tree.take_pending_script() {
                shared.write_insertion_point = Some(shared.tokenizer.cursor());
                break Ok(Some(node));
            }
        };
        let mut shared = parser.borrow_mut();
        shared.builder = Some(tree.release_creation_host());
        if let Err(error) = result { shared.creation_failure = Some(error); }
        if !matches!(result, Ok(Some(_))) { shared.end_write(); }
        result
    }

    /// Restores suspended input if the embedding abandons a written script.
    pub fn end_write(&mut self) {
        let (tail, finished) = self.write_inputs.pop().expect("active document.write input");
        self.write_insertion_point = Some(self.tokenizer.end_write_input(tail, finished));
    }

    /// Swaps the DOM under construction with `other`. A driver calls this
    /// twice around running a paused script: once to hand the live tree to
    /// the JS host (so the script sees/mutates the real document), and once
    /// after the script returns to take it back and keep building. This is a
    /// swap, not a copy -- node indices from either side stay valid the
    /// whole time, since it is the same arena moving, never a second one.
    pub fn swap_dom(&mut self, other: &mut Dom) {
        std::mem::swap(self.builder.as_mut().expect("tree execution frame is borrowed").dom_mut(), other);
    }

    /// The DOM under construction, immutably.
    pub fn dom(&self) -> &Dom {
        self.builder.as_ref().expect("tree execution frame is borrowed").dom()
    }

    /// The DOM under construction, mutably. A driver that keeps this arena
    /// separately grafted into a shared document (rather than physically
    /// exchanging it via `swap_dom`, which only works when this parser's
    /// arena has nothing else in it -- see `amazing_dom::markup::
    /// IncrementalDocumentImport`'s own doc comment) needs this to reflect a
    /// script's mutation of the already-grafted subtree back into this
    /// arena before resuming: `IncrementalDocumentImport::reverse_sync`
    /// writes through exactly this accessor.
    pub fn dom_mut(&mut self) -> &mut Dom {
        self.builder.as_mut().expect("tree execution frame is borrowed").dom_mut()
    }

    /// Drains every `defer`/`async`/module script discovered since the last
    /// call, in document order. Meaningful to call after `Eof` (to get the
    /// full, final list) or between pauses (a driver that wants to start
    /// prefetching a deferred script's `src` as soon as it is discovered,
    /// without waiting for the whole document, can drain early).
    pub fn take_deferred_or_async(&mut self) -> Vec<(NodeIdx, ScriptMode)> {
        std::mem::take(&mut self.deferred_or_async)
    }

    /// Consumes the parser and returns the finished DOM. Only meaningful once
    /// `run` has returned `ParseSignal::Eof`; calling it earlier returns
    /// whatever has been built so far (a caller that wants to abandon a
    /// parse mid-document can still use it, matching `document.close()`
    /// abandoning the rest of a real parse).
    pub fn into_dom(mut self) -> Dom {
        self.builder.take().expect("tree execution frame is borrowed").into_dom()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use amazing_dom::NodeIdx;

    #[test]
    fn synchronous_writes_preserve_partial_tokens_order_and_original_tail() {
        for chunks in [vec!["<div id=x>&amp;</div>"],
            vec!["<di", "v id=x>", "&am", "p;", "</div>"]] {
            let mut parser = ResumableParser::new(Dom::new());
            parser.push_source("<body><script>first</script><p id=tail>tail</p>");
            assert!(matches!(parser.run(), ParseSignal::Script(_)));
            for chunk in chunks {
                parser.begin_write(chunk);
                assert_eq!(parser.run_write(), None);
                assert!(!parser.dom().dump(NodeIdx(0)).contains("tail"));
            }
            let written = parser.dom().dump(NodeIdx(0));
            assert!(written.contains("<div id=\"x\">"), "{written}");
            assert!(written.contains("\"&\""), "{written}");
            assert_eq!(parser.run(), ParseSignal::Eof);
            assert!(parser.dom().dump(NodeIdx(0)).contains("<p id=\"tail\">"));
        }
    }

    #[test]
    fn nested_written_scripts_restore_each_input_boundary_without_eof() {
        let mut parser = ResumableParser::new(Dom::new());
        parser.push_source("<body><script>first</script><p id=tail>tail</p>");
        assert!(matches!(parser.run(), ParseSignal::Script(_)));
        parser.begin_write("<script>nested</script><b id=outer>outer</b>");
        assert!(parser.run_write().is_some());
        parser.begin_write("<i id=inner>inner</i>");
        assert_eq!(parser.run_write(), None);
        assert!(!parser.dom().dump(NodeIdx(0)).contains("outer"));
        assert_eq!(parser.run_write(), None);
        let written = parser.dom().dump(NodeIdx(0));
        assert!(written.contains("<i id=\"inner\">"));
        assert!(written.contains("<b id=\"outer\">"));
        assert!(!written.contains("tail"));
        assert_eq!(parser.run(), ParseSignal::Eof);
    }

    /// The whole point: a script observes the DOM as constructed so far, not
    /// the finished document -- the node after it does not exist yet.
    #[test]
    fn run_pauses_right_after_a_blocking_script_with_only_prior_siblings_built() {
        let mut parser = ResumableParser::new(Dom::new());
        parser.push_source("<body><div id=a></div><script>x</script><div id=b></div></body>");
        let ParseSignal::Script(script) = parser.run() else { panic!("expected a script pause") };
        assert_eq!(parser.dom().name(script), Some("script"));
        let dump = parser.dom().dump(NodeIdx(0));
        assert!(dump.contains("id=\"a\""), "prior sibling missing:\n{dump}");
        assert!(!dump.contains("id=\"b\""), "later sibling built before its script paused:\n{dump}");
        assert_eq!(parser.run(), ParseSignal::Eof);
        let dump = parser.dom().dump(NodeIdx(0));
        assert!(dump.contains("id=\"b\""), "later sibling missing after resuming:\n{dump}");
    }

    #[test]
    fn write_inserts_at_the_paused_cursor_not_at_the_end_of_the_buffered_source() {
        let mut parser = ResumableParser::new(Dom::new());
        parser.push_source("<body>a<script>x</script>b</body>");
        let ParseSignal::Script(_) = parser.run() else { panic!("expected a script pause") };
        parser.write("WRITTEN");
        assert_eq!(parser.run(), ParseSignal::Eof);
        let dump = parser.dom().dump(NodeIdx(0));
        assert!(dump.contains("\"aWRITTENb\"") || dump.contains("\"a\"") && dump.contains("\"WRITTENb\""),
            "written text not spliced right after the script, before the remaining source:\n{dump}");
    }

    #[test]
    fn defer_and_async_scripts_never_pause_and_are_collected_in_document_order() {
        let mut parser = ResumableParser::new(Dom::new());
        parser.push_source(
            "<body><script src=a.js defer></script><script src=b.js async></script>\
             <script>blocking</script></body>",
        );
        let ParseSignal::Script(blocking) = parser.run() else { panic!("expected the blocking script") };
        assert_eq!(parser.dom().name(blocking), Some("script"));
        let deferred = parser.take_deferred_or_async();
        assert_eq!(deferred.len(), 2, "{deferred:?}");
        assert_eq!(deferred[0].1, ScriptMode::Deferred);
        assert_eq!(deferred[1].1, ScriptMode::Async);
        assert_eq!(parser.run(), ParseSignal::Eof);
    }

    /// HTML-32: a self-closing SVG `<script/>` (only meaningful with `src`,
    /// since self-closing leaves no room for inline text) used to be popped
    /// by `in_foreign_content`'s `StartTag` branch with no classify/pause
    /// check at all -- the same gap HTML-31 fixed for its non-self-closing
    /// sibling, just reached through the self-closing path instead of the
    /// `EndTag` match arm.
    #[test]
    fn self_closing_svg_script_pauses_the_same_as_a_normal_one() {
        let mut parser = ResumableParser::new(Dom::new());
        // An unquoted attribute value's terminator set has no `/` in it (only
        // whitespace/`>`), so `src=a.js/` would swallow the slash into the
        // value instead of ending the tag self-closing -- the self-closing
        // flag needs a quoted value (or a space before the slash) to leave
        // "after attribute value (quoted)"/"before attribute name" state.
        parser.push_source("<body><svg><script src=\"a.js\"/></svg><div id=after></div></body>");
        let ParseSignal::Script(script) = parser.run() else { panic!("expected a script pause") };
        assert_eq!(parser.dom().name(script), Some("script"));
        let dump = parser.dom().dump(NodeIdx(0));
        assert!(!dump.contains("id=\"after\""), "later sibling built before its script paused:\n{dump}");
        assert_eq!(parser.run(), ParseSignal::Eof);
    }

    /// HTML-34: the iframe counterpart of `run_pauses_right_after_a_blocking_
    /// script_with_only_prior_siblings_built` above, but grafted into a
    /// *different*, pre-existing arena (`amazing_dom::markup::
    /// IncrementalDocumentImport`) rather than swapped in place -- proves the
    /// graft sees exactly the document-so-far at each pause, that an
    /// already-grafted node's destination identity survives the next
    /// `sync`, and that the destination document's own pre-existing content
    /// (standing in for a parent page's own nodes elsewhere in the same
    /// arena) is completely undisturbed throughout.
    #[test]
    fn incremental_import_grafts_document_so_far_at_each_pause_into_a_shared_arena() {
        use amazing_dom::markup::IncrementalDocumentImport;

        let mut host = Dom::new();
        let parent_marker = host.create_element(NodeIdx(0), "parent-page-marker");
        let iframe_document = host.create_document();
        let mut import = IncrementalDocumentImport::new(iframe_document);

        let mut parser = ResumableParser::new(Dom::new());
        parser.push_source("<body><div id=a></div><script>x</script><div id=b></div></body>");
        let ParseSignal::Script(_) = parser.run() else { panic!("expected a script pause") };
        import.sync(&mut host, parser.dom()).unwrap();
        let dump = host.dump(iframe_document);
        assert!(dump.contains("id=\"a\""), "prior sibling missing:\n{dump}");
        assert!(!dump.contains("id=\"b\""), "later sibling grafted before its script paused:\n{dump}");
        // The script pause runs against `host`, exactly like a real script
        // would; grab the already-grafted `<div id=a>`'s destination NodeIdx
        // now, to confirm below that resuming does not recreate it.
        let source_a = parser.dom().tree_order(NodeIdx(0)).into_iter()
            .find(|&n| parser.dom().get_attribute_ns(n, None, "id") == Some("a")).unwrap();
        let dest_a_before = import.imported(source_a).unwrap();
        assert!(host.dump(NodeIdx(0)).contains("parent-page-marker"),
            "unrelated parent-page content must survive the graft untouched");

        assert_eq!(parser.run(), ParseSignal::Eof);
        import.sync(&mut host, parser.dom()).unwrap();
        let dump = host.dump(iframe_document);
        assert!(dump.contains("id=\"b\""), "later sibling missing after resuming:\n{dump}");
        assert_eq!(import.imported(source_a), Some(dest_a_before),
            "already-grafted node must keep its destination identity across the resume");
        assert!(host.dump(NodeIdx(0)).contains("parent-page-marker"), "parent-page content survives to Eof");
        let _ = parent_marker;
    }

    #[test]
    fn fragment_parsing_never_pauses_for_scripts() {
        let dom = crate::parse_fragment("div", "<script>x</script>").expect("fragment");
        // `parse_fragment` always runs to completion; reaching here at all
        // (rather than the fragment builder trying to hand back a pause with
        // nowhere for a fragment-parsing caller to send it) is the assertion.
        assert!(dom.dump(NodeIdx(0)).contains("<script>"));
    }

    #[test]
    fn srcdoc_keeps_standards_table_recovery_with_missing_or_quirky_doctype() {
        for source in ["<p><table>", "<!DOCTYPE html PUBLIC 'html'><p><table>"] {
            for srcdoc in [false, true] {
                let mut parser = ResumableParser::new(Dom::new());
                if srcdoc { parser.set_srcdoc(); }
                parser.push_source(source);
                assert_eq!(parser.run(), ParseSignal::Eof);
                let dom = parser.dom();
                let nodes = dom.tree_order(NodeIdx(0));
                let paragraph = nodes.iter().copied().find(|&n| dom.name(n) == Some("p")).unwrap();
                let table = nodes.iter().copied().find(|&n| dom.name(n) == Some("table")).unwrap();
                assert_eq!(dom.get(table).parent == Some(paragraph), !srcdoc, "{source}, srcdoc={srcdoc}");
            }
        }
    }
    #[test]
    fn declarative_shadow_permission_is_independent_of_scripting_and_srcdoc() {
        for allowed in [false, true] {
            for scripting in [crate::ScriptingMode::Disabled, crate::ScriptingMode::Enabled] {
                for srcdoc in [false, true] {
                    let mut parser = ResumableParser::new(Dom::new());
                    parser.set_allow_declarative_shadow_roots(allowed);
                    parser.set_scripting_mode(scripting);
                    if srcdoc { parser.set_srcdoc(); }
                    parser.push_source("<div><template shadowrootmode=open><span>shadow</span></template></div>");
                    assert_eq!(parser.run(), ParseSignal::Eof);
                    let dom = parser.dom();
                    let host = dom.tree_order(NodeIdx(0)).into_iter().find(|&n| dom.name(n) == Some("div")).unwrap();
                    assert_eq!(dom.shadow_root_of_host(host).is_some(), allowed);
                    assert_eq!(dom.children(host).any(|n| dom.name(n) == Some("template")), !allowed);
                }
            }
        }
    }

}
