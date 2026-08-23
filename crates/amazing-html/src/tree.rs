//! WHATWG HTML tree construction (§13.2.6 "Tree construction").
//!
//! Consumes [`Token`]s from [`crate::tokenizer`] and builds an
//! `amazing_dom::Dom` arena tree (indices only — never `Rc<RefCell<..>>`).
//! Insertion modes, the stack of open elements, the list of active
//! formatting elements (including reconstruction and the adoption agency
//! algorithm), foster parenting and implied-element synthesis follow the
//! spec closely enough that the classic misnesting fixtures
//! (`<b>1<p>2</b>3</p>`, `<b>1<i>2</b>3</i>`) come out browser-shaped.
//!
//! Deviations from full spec (deliberate, mirroring lib.rs):
//! - `template` is ignored everywhere; no "in template" insertion modes.
//! - Frameset parsing is skipped entirely; frameset documents degrade to
//!   ordinary body content.
//! - `select`/`optgroup`/`option` have no dedicated insertion modes; a
//!   `select` start tag simply stays in "in body", and the mode-reset table
//!   folds `select` back to "in body".
//! - `noscript` content is parsed as raw text regardless of scripting flag.
//! - Foreign content (`svg`/`math`) becomes ordinary unknown HTML elements;
//!   no MathML/SVG integration points.
//! - `plaintext` loses its tokenizer-level effect (no PLAINTEXT state); it
//!   is inserted as an ordinary unknown element.
//! - The adoption agency algorithm runs at most one full reparenting pass
//!   per invocation, matching shipping engines, though the spec's outer
//!   loop nominally allows eight.
//! - Scope terminator sets omit `template`.
//! - Quirks-mode differences are not modelled; the no-quirks rules always
//!   apply (e.g. `<table>` always closes an open `<p>`).
//! - The "in table text" pending-character buffer is collapsed: non-
//!   whitespace characters in table context are foster-parented token by
//!   token instead of being collected and flushed together.
//!
//! The tokenizer owns RAWTEXT/RCDATA switching (script/style/textarea/
//! title), so this module only implements the "text" insertion-mode
//! bookkeeping: insert character tokens into the current node, pop on the
//! appropriate end tag.

use amazing_dom::{Dom, NodeIdx, NodeKind};

use crate::tokenizer::{Token, Tokenizer};

/// Elements whose start tags never take children (void + obsolete empties).
const VOID_ELEMENTS: &[&str] = &[
    "area", "base", "basefont", "bgsound", "br", "col", "embed", "hr", "img",
    "input", "link", "meta", "param", "source", "track", "wbr",
];

/// Default-scope terminator element names (§13.2.4.3). `template` omitted.
const SCOPE_TERMINATORS: &[&str] =
    &["applet", "caption", "html", "table", "td", "th", "marquee", "object"];

/// Extra terminators for "in button scope".
const BUTTON_SCOPE_EXTRA: &[&str] = &["button"];

/// Extra terminators for "in list item scope".
const LIST_ITEM_SCOPE_EXTRA: &[&str] = &["ol", "ul"];

/// Names popped by "generate implied end tags" (§13.2.6.3). `td`/`th` are
/// included per the project plan so cell-closing paths converge.
const IMPLIED_END_TAGS: &[&str] = &[
    "dd", "dt", "li", "optgroup", "option", "p", "rb", "rp", "rt", "td", "th",
];

/// Active formatting elements (§13.2.4.3).
const FORMATTING_ELEMENTS: &[&str] = &[
    "a", "b", "big", "code", "em", "font", "i", "nobr", "s", "small",
    "strike", "strong", "tt", "u",
];

/// Special category (§13.2.4.3, HTML part). Formatting elements and unknown
/// elements are deliberately absent — this drives furthest-block selection
/// in the adoption agency and the li/dd/dt stack walks.
const SPECIAL_ELEMENTS: &[&str] = &[
    "address", "applet", "area", "article", "aside", "base", "basefont",
    "bgsound", "blockquote", "body", "br", "button", "caption", "center",
    "col", "colgroup", "dd", "details", "dir", "div", "dl", "dt", "embed",
    "fieldset", "figcaption", "figure", "footer", "form", "frame", "frameset",
    "h1", "h2", "h3", "h4", "h5", "h6", "head", "header", "hgroup", "hr",
    "html", "iframe", "img", "input", "keygen", "li", "link", "listing",
    "main", "marquee", "menu", "meta", "nav", "noembed", "noframes",
    "noscript", "object", "ol", "p", "param", "plaintext", "pre", "script",
    "search", "section", "select", "source", "style", "summary", "table",
    "tbody", "td", "template", "textarea", "tfoot", "th", "thead", "title",
    "tr", "track", "ul", "wbr", "xmp",
];

fn is_ws_char(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\u{000C}' | ' ')
}

/// Drops one leading U+000A — the pre/listing/textarea newline rule.
fn strip_leading_lf(mut s: String) -> String {
    if s.starts_with('\n') {
        s.remove(0);
    }
    s
}

/// Insertion modes (§13.2.4.1). `InSelect*`, `InTemplate*` and frameset
/// modes are folded away (see module deviation list).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    Initial,
    BeforeHtml,
    BeforeHead,
    InHead,
    AfterHead,
    InBody,
    Text,
    InTable,
    InTableBody,
    InRow,
    InCell,
    InCaption,
    InColumnGroup,
    AfterBody,
    AfterAfterBody,
}

/// One entry in the list of active formatting elements.
#[derive(Debug)]
enum FmtEntry {
    Marker,
    Element {
        idx: NodeIdx,
        name: String,
        attrs: Vec<(String, String)>,
    },
}

/// Tree-construction state machine producing a [`Dom`].
pub struct TreeBuilder {
    dom: Dom,
    mode: Mode,
    /// Return mode for "text" (the "original insertion mode").
    orig_mode: Mode,
    /// Stack of open elements; `open[0]` is the `html` element once
    /// synthesized. The document node lives outside this stack.
    open: Vec<NodeIdx>,
    head: Option<NodeIdx>,
    form: Option<NodeIdx>,
    #[allow(dead_code)]
    frameset_ok: bool,
    /// pre/listing/textarea: strip one leading U+000A from the next token.
    skip_next_lf: bool,
    /// Foster-parenting flag; set while processing table-fallback content
    /// and cleared after every token.
    foster: bool,
    fmt: Vec<FmtEntry>,
}

impl Default for TreeBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl TreeBuilder {
    pub fn new() -> Self {
        Self {
            dom: Dom::new(),
            mode: Mode::Initial,
            orig_mode: Mode::Initial,
            open: Vec::new(),
            head: None,
            form: None,
            frameset_ok: true,
            skip_next_lf: false,
            foster: false,
            fmt: Vec::new(),
        }
    }

    /// Processes ONE token.
    pub fn feed(&mut self, tok: Token) {
        let tok = if self.skip_next_lf {
            self.skip_next_lf = false;
            match tok {
                Token::Characters(s) => Token::Characters(strip_leading_lf(s)),
                other => other,
            }
        } else {
            tok
        };
        self.process_token(tok);
        self.foster = false;
    }

    /// Returns the finished tree, consuming the builder.
    pub fn into_dom(self) -> Dom {
        self.dom
    }

    /// Borrows the tree under construction.
    pub fn dom(&self) -> &Dom {
        &self.dom
    }

    // ---- token routing ----

    fn process_token(&mut self, tok: Token) {
        match self.mode {
            Mode::Initial => self.initial(tok),
            Mode::BeforeHtml => self.before_html(tok),
            Mode::BeforeHead => self.before_head(tok),
            Mode::InHead => self.in_head(tok),
            Mode::AfterHead => self.after_head(tok),
            Mode::InBody => self.in_body(tok),
            Mode::Text => self.text_mode(tok),
            Mode::InTable => self.in_table(tok),
            Mode::InTableBody => self.in_table_body(tok),
            Mode::InRow => self.in_row(tok),
            Mode::InCell => self.in_cell(tok),
            Mode::InCaption => self.in_caption(tok),
            Mode::InColumnGroup => self.in_column_group(tok),
            Mode::AfterBody => self.after_body(tok),
            Mode::AfterAfterBody => self.after_after_body(tok),
        }
    }

    // ---- small queries ----

    fn current(&self) -> Option<NodeIdx> {
        self.open.last().copied()
    }

    fn stack_has(&self, name: &str) -> bool {
        self.open.iter().any(|&i| self.dom.name(i) == Some(name))
    }

    fn stack_contains(&self, idx: NodeIdx) -> bool {
        self.open.iter().any(|&i| i == idx)
    }

    fn is_special(&self, idx: NodeIdx) -> bool {
        matches!(self.dom.name(idx), Some(n) if SPECIAL_ELEMENTS.contains(&n))
    }

    fn elem_name_attrs(&self, idx: NodeIdx) -> (String, Vec<(String, String)>) {
        match self.dom.get(idx).kind {
            NodeKind::Element { ref name, ref attrs } => {
                (name.clone(), attrs.clone())
            },
            _ => (String::new(), Vec::new()),
        }
    }

    /// Generic scope walk (§13.2.4.3): scan top-down; terminate on a match
    /// or on any terminator.
    fn has_in_specific_scope(&self, name: &str, extra: &[&str]) -> bool {
        for &node in self.open.iter().rev() {
            let n = self.dom.name(node).unwrap_or("");
            if n == name {
                return true;
            }
            if SCOPE_TERMINATORS.contains(&n) || extra.contains(&n) {
                return false;
            }
        }
        false
    }

    fn has_in_scope(&self, name: &str) -> bool {
        self.has_in_specific_scope(name, &[])
    }

    fn has_in_button_scope(&self, name: &str) -> bool {
        self.has_in_specific_scope(name, BUTTON_SCOPE_EXTRA)
    }

    #[allow(dead_code)]
    fn has_in_list_item_scope(&self, name: &str) -> bool {
        self.has_in_specific_scope(name, LIST_ITEM_SCOPE_EXTRA)
    }

    // ---- insertion primitives (§13.2.6.1) ----

    /// Appropriate insertion location `(parent, before-sibling)`. Honours
    /// the foster flag when the current node is table family: the node lands
    /// immediately before the last `table` on the open-element stack.
    fn insertion_place(&self) -> (NodeIdx, Option<NodeIdx>) {
        let Some(&target) = self.open.last() else {
            return (NodeIdx(0), None);
        };
        if self.foster {
            let family = {
                let n = self.dom.name(target).unwrap_or("");
                matches!(n, "table" | "tbody" | "tfoot" | "thead" | "tr")
            };
            if family {
                for &t in self.open.iter().rev() {
                    if self.dom.name(t) == Some("table") {
                        if let Some(parent) = self.dom.get(t).parent {
                            return (parent, Some(t));
                        }
                    }
                }
            }
        }
        (target, None)
    }

    fn alloc_element(
        &mut self,
        name: &str,
        attrs: Vec<(String, String)>,
    ) -> NodeIdx {
        self.dom
            .alloc_unattached(NodeKind::Element { name: name.to_owned(), attrs })
    }

    /// Creates an element at the appropriate insertion place WITHOUT
    /// pushing it onto the stack of open elements.
    fn insert_element(
        &mut self,
        name: &str,
        attrs: Vec<(String, String)>,
    ) -> NodeIdx {
        let (parent, before) = self.insertion_place();
        let idx = self.alloc_element(name, attrs);
        self.dom.attach_before(parent, before, idx);
        idx
    }

    fn insert_and_push(
        &mut self,
        name: &str,
        attrs: Vec<(String, String)>,
    ) -> NodeIdx {
        let idx = self.insert_element(name, attrs);
        self.open.push(idx);
        idx
    }

    fn insert_void(
        &mut self,
        name: &str,
        attrs: Vec<(String, String)>,
    ) -> NodeIdx {
        debug_assert!(VOID_ELEMENTS.contains(&name));
        self.insert_element(name, attrs)
    }

    /// Inserts character data, merging into an adjacent text node so markup-
    /// free character runs become a single text node.
    fn insert_text(&mut self, data: &str) {
        if data.is_empty() {
            return;
        }
        let (parent, before) = self.insertion_place();
        let merge_target = match before {
            Some(b) => self.dom.get(b).prev_sibling,
            None => self.dom.last_child(parent),
        };
        if let Some(prev) = merge_target {
            if self.dom.is_text(prev) {
                self.dom.append_text_data(prev, data);
                return;
            }
        }
        let t = self
            .dom
            .alloc_unattached(NodeKind::Text { data: data.to_owned() });
        self.dom.attach_before(parent, before, t);
    }

    fn insert_comment_here(&mut self, data: &str) {
        let (parent, before) = self.insertion_place();
        let c = self
            .dom
            .alloc_unattached(NodeKind::Comment { data: data.to_owned() });
        self.dom.attach_before(parent, before, c);
    }

    fn comment_into(&mut self, parent: NodeIdx, data: &str) {
        let c = self
            .dom
            .alloc_unattached(NodeKind::Comment { data: data.to_owned() });
        self.dom.attach_last(parent, c);
    }

    /// Merges `new_attrs` into an existing element; first occurrence wins.
    fn merge_attrs(&mut self, idx: NodeIdx, new_attrs: Vec<(String, String)>) {
        let mut merged: Vec<(String, String)> = self
            .dom
            .attrs_of(idx)
            .map(|a| a.to_vec())
            .unwrap_or_default();
        for (n, v) in new_attrs {
            if !merged.iter().any(|(en, _)| *en == n) {
                merged.push((n, v));
            }
        }
        self.dom.set_attrs(idx, merged);
    }

    /// Generic RAWTEXT/RCDATA element entry: the tokenizer has already
    /// switched states, so this records tree-builder bookkeeping only.
    fn start_generic_text_element(
        &mut self,
        name: &str,
        attrs: Vec<(String, String)>,
    ) {
        self.insert_and_push(name, attrs);
        self.orig_mode = self.mode;
        self.mode = Mode::Text;
    }

    // ---- stack surgery ----

    fn generate_implied_end_tags(&mut self, except: Option<&str>) {
        loop {
            let Some(&cur) = self.open.last() else { break };
            let pops = {
                let n = self.dom.name(cur).unwrap_or("");
                IMPLIED_END_TAGS.contains(&n) && Some(n) != except
            };
            if pops {
                self.open.pop();
            } else {
                break;
            }
        }
    }

    fn pop_until(&mut self, name: &str) {
        while let Some(idx) = self.open.pop() {
            if self.dom.name(idx) == Some(name) {
                break;
            }
        }
    }

    fn pop_until_any(&mut self, names: &[&str]) {
        while let Some(idx) = self.open.pop() {
            if matches!(self.dom.name(idx), Some(n) if names.contains(&n)) {
                break;
            }
        }
    }

    /// Reset the insertion mode by walking the open-element stack top-down
    /// (§13.2.4.1). `select` folds to "in body" per the deviations.
    fn reset_insertion_mode(&mut self) {
        for &node in self.open.iter().rev() {
            let n = self.dom.name(node).unwrap_or("");
            let next = match n {
                "select" => Some(Mode::InBody),
                "td" | "th" => Some(Mode::InCell),
                "tr" => Some(Mode::InRow),
                "tbody" | "tfoot" | "thead" => Some(Mode::InTableBody),
                "caption" => Some(Mode::InCaption),
                "colgroup" => Some(Mode::InColumnGroup),
                "table" => Some(Mode::InTable),
                "body" => Some(Mode::InBody),
                "html" => Some(if self.head.is_none() {
                    Mode::BeforeHead
                } else {
                    Mode::AfterHead
                }),
                _ => None,
            };
            if let Some(m) = next {
                self.mode = m;
                return;
            }
        }
        self.mode = Mode::InBody;
    }

    // ---- active formatting elements ----

    fn fmt_pos_by_idx(&self, idx: NodeIdx) -> Option<usize> {
        self.fmt
            .iter()
            .position(|e| matches!(e, FmtEntry::Element { idx: i, .. } if *i == idx))
    }

    /// Last entry after the final marker whose element name matches.
    fn fmt_pos_last_named(&self, name: &str) -> Option<usize> {
        for (i, e) in self.fmt.iter().enumerate().rev() {
            match e {
                FmtEntry::Marker => return None,
                FmtEntry::Element { name: n, .. } if n == name => return Some(i),
                _ => {},
            }
        }
        None
    }

    fn has_active_formatting(&self, name: &str) -> bool {
        self.fmt_pos_last_named(name).is_some()
    }

    fn fmt_idx_at(&self, pos: usize) -> NodeIdx {
        match &self.fmt[pos] {
            FmtEntry::Element { idx, .. } => *idx,
            FmtEntry::Marker => unreachable!("fmt_idx_at on marker"),
        }
    }

    /// Pushes a formatting entry applying the Noah's Ark clause: at most
    /// three identical entries (same tag name, identical attribute list)
    /// after the last marker; the oldest beyond that drops off.
    fn push_fmt(
        &mut self,
        idx: NodeIdx,
        name: String,
        attrs: Vec<(String, String)>,
    ) {
        let mut dups: Vec<usize> = Vec::new();
        for i in (0..self.fmt.len()).rev() {
            match &self.fmt[i] {
                FmtEntry::Marker => break,
                FmtEntry::Element { name: n, attrs: a, .. }
                    if *n == name && *a == attrs =>
                {
                    dups.push(i);
                },
                _ => break,
            }
        }
        if dups.len() >= 3 {
            let oldest = dups[dups.len() - 1];
            self.fmt.remove(oldest);
        }
        self.fmt.push(FmtEntry::Element { idx, name, attrs });
    }

    fn clear_fmt_to_marker(&mut self) {
        while let Some(e) = self.fmt.pop() {
            if matches!(e, FmtEntry::Marker) {
                break;
            }
        }
    }

    /// Reconstruct the active formatting elements (§13.2.4.3). Markers and
    /// entries whose element is on the open-element stack act as barriers;
    /// everything after the deepest barrier is cloned afresh at the current
    /// insertion point and its entry re-pointed at the clone.
    fn reconstruct(&mut self) {
        if self.fmt.is_empty() {
            return;
        }
        let barrier_at_top = match &self.fmt[self.fmt.len() - 1] {
            FmtEntry::Marker => true,
            FmtEntry::Element { idx, .. } => self.stack_contains(*idx),
        };
        if barrier_at_top {
            return;
        }
        // Rewind: walk back while the previous entry is neither a marker nor
        // on the stack; `start` ends at the first entry needing recreation.
        let mut start = self.fmt.len() - 1;
        while start > 0 {
            let blocked = match &self.fmt[start - 1] {
                FmtEntry::Marker => true,
                FmtEntry::Element { idx, .. } => self.stack_contains(*idx),
            };
            if blocked {
                break;
            }
            start -= 1;
        }
        let mut i = start;
        while i < self.fmt.len() {
            let (name, attrs) = match &self.fmt[i] {
                FmtEntry::Marker => unreachable!("marker inside rewind window"),
                FmtEntry::Element { name, attrs, .. } => {
                    (name.clone(), attrs.clone())
                },
            };
            // Recreated elements join the stack of open elements, so the
            // next token nests inside the youngest of them.
            let idx = self.insert_and_push(&name, attrs);
            if let FmtEntry::Element { idx: slot, .. } = &mut self.fmt[i] {
                *slot = idx;
            }
            i += 1;
        }
    }

    // ---- adoption agency (§13.2.6.5) ----

    /// Runs at most one full reparenting pass (module deviation: shipping
    /// engines stop after one despite the spec's eight-iteration loop).
    fn adoption_agency(&mut self, subject: &str) {
        // Current node is the subject yet absent from the fmt list: pop it.
        if let Some(cur) = self.current() {
            let is_subject = self.dom.name(cur) == Some(subject);
            if is_subject && self.fmt_pos_by_idx(cur).is_none() {
                self.open.pop();
                return;
            }
        }
        for _outer in 0..8 {
            let Some(fe_fmt_pos) = self.fmt_pos_last_named(subject) else {
                self.any_other_end_tag(subject);
                return;
            };
            let fe_idx = self.fmt_idx_at(fe_fmt_pos);

            let Some(fe_stack_pos) = self.open.iter().position(|&i| i == fe_idx)
            else {
                self.fmt.remove(fe_fmt_pos);
                return;
            };

            if !self.has_in_scope(subject) {
                return;
            }

            // Furthest block: topmost special-category element below fe.
            let mut fb_stack_pos = None;
            for sp in (fe_stack_pos + 1)..self.open.len() {
                if self.is_special(self.open[sp]) {
                    fb_stack_pos = Some(sp);
                    break;
                }
            }
            let Some(fb_sp) = fb_stack_pos else {
                while let Some(n) = self.open.pop() {
                    if n == fe_idx {
                        break;
                    }
                }
                self.fmt.remove(fe_fmt_pos);
                return;
            };

            // ---- one reparenting pass ----
            let fb_idx = self.open[fb_sp];
            let common = if fe_stack_pos > 0 {
                self.open[fe_stack_pos - 1]
            } else {
                NodeIdx(0)
            };
            let mut bookmark = fe_fmt_pos;
            let mut last_node = fb_idx;
            let mut last_node_pos = fb_sp;
            let mut inner: u32 = 0;

            loop {
                inner += 1;
                if inner > 64 || last_node_pos <= fe_stack_pos {
                    break; // defensive bound; spec caps at ~8 passes
                }
                let cand_pos = last_node_pos - 1;
                let cand = self.open[cand_pos];
                if cand == fe_idx {
                    break;
                }
                let in_fmt = self.fmt_pos_by_idx(cand);
                if inner > 3 && in_fmt.is_some() {
                    let p = in_fmt.expect("checked above");
                    self.fmt.remove(p);
                    if bookmark > p {
                        bookmark -= 1;
                    }
                    continue;
                }
                if in_fmt.is_none() {
                    self.open.remove(cand_pos);
                    last_node_pos -= 1;
                    continue;
                }
                // Clone the candidate; re-point fmt and stack entries.
                let (cname, cattrs) = self.elem_name_attrs(cand);
                let clone = self.dom.alloc_unattached(NodeKind::Element {
                    name: cname.clone(),
                    attrs: cattrs.clone(),
                });
                let p = in_fmt.expect("checked above");
                self.fmt[p] =
                    FmtEntry::Element { idx: clone, name: cname, attrs: cattrs };
                self.open[cand_pos] = clone;
                if last_node == fb_idx {
                    bookmark = p + 1;
                }
                self.dom.detach(last_node);
                self.dom.attach_last(clone, last_node);
                last_node = clone;
                last_node_pos = cand_pos;
            }

            // Move lastNode under the common ancestor.
            self.dom.detach(last_node);
            self.dom.attach_last(common, last_node);

            // Clone the formatting element; adopt furthest block's children.
            let (fname, fattrs) = self.elem_name_attrs(fe_idx);
            let fe_clone = self.dom.alloc_unattached(NodeKind::Element {
                name: fname.clone(),
                attrs: fattrs.clone(),
            });
            while let Some(child) = self.dom.last_child(fb_idx) {
                self.dom.detach(child);
                self.dom.attach_last(fe_clone, child);
            }
            self.dom.attach_last(fb_idx, fe_clone);

            // Patch the fmt list at the bookmark.
            if let Some(p) = self.fmt_pos_by_idx(fe_idx) {
                self.fmt.remove(p);
            }
            let bm = bookmark.min(self.fmt.len());
            self.fmt.insert(
                bm,
                FmtEntry::Element { idx: fe_clone, name: fname, attrs: fattrs },
            );

            // Patch the stack: fe out, clone immediately below furthest block.
            self.open.remove(fe_stack_pos);
            let fb_now = self
                .open
                .iter()
                .position(|&i| i == fb_idx)
                .unwrap_or(self.open.len());
            self.open.insert(fb_now, fe_clone);

            return;
        }
    }

    /// "Any other end tag": walk the stack top-down; on the first name match
    /// generate implied end tags (except the subject) and pop inclusively.
    fn any_other_end_tag(&mut self, name: &str) {
        for i in (0..self.open.len()).rev() {
            if self.dom.name(self.open[i]) == Some(name) {
                self.generate_implied_end_tags(Some(name));
                self.pop_until(name);
                return;
            }
        }
    }

    // ---- implied structure synthesis ----

    fn implied_html(&mut self) {
        self.insert_and_push("html", Vec::new());
        self.mode = Mode::BeforeHead;
    }

    fn implied_head(&mut self) {
        let idx = self.insert_and_push("head", Vec::new());
        self.head = Some(idx);
        self.mode = Mode::InHead;
    }

    fn implied_body(&mut self) {
        self.insert_and_push("body", Vec::new());
        self.frameset_ok = true;
        self.mode = Mode::InBody;
    }

    /// Close an open `p` in button scope, if any.
    fn close_p_element(&mut self) {
        if self.has_in_button_scope("p") {
            self.generate_implied_end_tags(Some("p"));
            self.pop_until("p");
        }
    }

    /// li/dd/dt predecessor-closing walk (§13.2.6.4.7).
    fn close_list_item_like(&mut self, kinds: &[&str]) {
        for i in (0..self.open.len()).rev() {
            let n = self.dom.name(self.open[i]).unwrap_or("").to_owned();
            if kinds.contains(&n.as_str()) {
                self.generate_implied_end_tags(Some(n.as_str()));
                self.pop_until(&n);
                break;
            }
            if self.is_special(self.open[i])
                && !matches!(n.as_str(), "address" | "div" | "p")
            {
                break;
            }
        }
        self.close_p_element();
    }

    fn clear_back_to_table(&mut self) {
        while let Some(&cur) = self.open.last() {
            if self.dom.name(cur) == Some("table") {
                break;
            }
            self.open.pop();
        }
    }

    fn clear_back_to_section(&mut self) {
        while let Some(&cur) = self.open.last() {
            if matches!(
                self.dom.name(cur),
                Some("tbody") | Some("tfoot") | Some("thead") | Some("table")
            ) {
                break;
            }
            self.open.pop();
        }
    }

    fn clear_back_to_row(&mut self) {
        while let Some(&cur) = self.open.last() {
            if matches!(self.dom.name(cur), Some("tr") | Some("table")) {
                break;
            }
            self.open.pop();
        }
    }

    /// Closes the current `tr`; returns whether one existed.
    fn close_tr(&mut self) -> bool {
        if self.stack_has("tr") {
            self.generate_implied_end_tags(Some("tr"));
            self.pop_until("tr");
            true
        } else {
            false
        }
    }

    /// Closes the enclosing row-group element (tbody/tfoot/thead); returns
    /// whether one existed. Leaves the mode unchanged.
    fn close_section(&mut self) -> bool {
        let present = self.stack_has("tbody")
            || self.stack_has("tfoot")
            || self.stack_has("thead");
        if present {
            self.generate_implied_end_tags(None);
            self.pop_until_any(&["tbody", "tfoot", "thead"]);
        }
        present
    }

    /// Closes the current cell. Cells sit in the implied-end-tag set, so a
    /// bare generate-implied pops exactly down through the cell.
    fn close_cell(&mut self) {
        self.generate_implied_end_tags(None);
        self.mode = Mode::InRow;
    }

    // ---- insertion modes ----

    fn initial(&mut self, tok: Token) {
        match tok {
            Token::Doctype { name, force_quirks: _ } => {
                self.dom.doctype = name;
                self.mode = Mode::BeforeHtml;
            },
            Token::Characters(data) => {
                if !data.chars().all(is_ws_char) {
                    self.mode = Mode::BeforeHtml;
                    self.before_html(Token::Characters(data));
                }
            },
            Token::Comment(data) => self.comment_into(NodeIdx(0), &data),
            other => {
                self.mode = Mode::BeforeHtml;
                self.before_html(other);
            },
        }
    }

    fn before_html(&mut self, tok: Token) {
        match tok {
            Token::Characters(data) => {
                if !data.chars().all(is_ws_char) {
                    self.implied_html();
                    self.before_head(Token::Characters(data));
                }
            },
            Token::Comment(data) => self.comment_into(NodeIdx(0), &data),
            Token::Doctype { .. } => {},
            Token::StartTag { ref name, attrs, self_closing: _ }
                if name == "html" =>
            {
                self.insert_and_push(name, attrs);
                self.mode = Mode::BeforeHead;
            },
            Token::EndTag { ref name }
                if matches!(name.as_str(), "head" | "body" | "html" | "br") =>
            {
                self.implied_html();
                self.before_head(Token::EndTag { name: name.clone() });
            },
            other => {
                self.implied_html();
                self.before_head(other);
            },
        }
    }

    fn before_head(&mut self, tok: Token) {
        match tok {
            Token::Characters(data) => {
                if !data.chars().all(is_ws_char) {
                    self.implied_head();
                    self.in_head(Token::Characters(data));
                }
            },
            Token::Comment(data) => self.insert_comment_here(&data),
            Token::Doctype { .. } => {},
            Token::StartTag { ref name, attrs, self_closing: _ }
                if name == "head" =>
            {
                let idx = self.insert_and_push(name, attrs);
                self.head = Some(idx);
                self.mode = Mode::InHead;
            },
            Token::EndTag { ref name }
                if matches!(name.as_str(), "head" | "body" | "html" | "br") =>
            {
                self.implied_head();
                self.in_head(Token::EndTag { name: name.clone() });
            },
            other => {
                self.implied_head();
                self.in_head(other);
            },
        }
    }

    fn leave_head(&mut self) {
        if let Some(cur) = self.current() {
            if self.dom.name(cur) == Some("head") {
                self.open.pop();
            }
        }
        self.mode = Mode::AfterHead;
    }

    fn in_head(&mut self, tok: Token) {
        match tok {
            Token::Characters(data) => {
                if data.chars().all(is_ws_char) {
                    self.insert_text(&data);
                } else {
                    self.leave_head();
                    self.after_head(Token::Characters(data));
                }
            },
            Token::Comment(data) => self.insert_comment_here(&data),
            Token::Doctype { .. } => {},
            Token::StartTag { ref name, attrs, self_closing: _ } => {
                match name.as_str() {
                    "base" | "basefont" | "bgsound" | "link" | "meta" => {
                        self.insert_void(name, attrs);
                    },
                    "title" | "noframes" | "style" | "script" | "noscript" => {
                        self.start_generic_text_element(name, attrs);
                    },
                    "template" => {}, // deviation: ignored
                    "head" => {},
                    _ => {
                        self.leave_head();
                        self.after_head(Token::StartTag {
                            name: name.clone(),
                            attrs,
                            self_closing: false,
                        });
                    },
                }
            },
            Token::EndTag { ref name } => match name.as_str() {
                "head" => self.leave_head(),
                "body" | "html" | "br" => {
                    self.leave_head();
                    self.after_head(Token::EndTag { name: name.clone() });
                },
                _ => {}, // template and strays ignored
            },
            other => {
                self.leave_head();
                self.after_head(other);
            },
        }
    }

    fn after_head(&mut self, tok: Token) {
        match tok {
            Token::Characters(data) => {
                if data.chars().all(is_ws_char) {
                    self.insert_text(&data);
                } else {
                    self.implied_body();
                    self.in_body(Token::Characters(data));
                }
            },
            Token::Comment(data) => self.insert_comment_here(&data),
            Token::Doctype { .. } => {},
            Token::StartTag { ref name, attrs, self_closing: _ } => {
                match name.as_str() {
                    "body" => {
                        self.insert_and_push(name, attrs);
                        self.frameset_ok = true;
                        self.mode = Mode::InBody;
                    },
                    "frameset" => {}, // deviation: frameset parsing skipped
                    _ => {
                        self.implied_body();
                        self.in_body(Token::StartTag {
                            name: name.clone(),
                            attrs,
                            self_closing: false,
                        });
                    },
                }
            },
            Token::EndTag { ref name }
                if matches!(name.as_str(), "body" | "html" | "br") =>
            {
                self.implied_body();
                self.in_body(Token::EndTag { name: name.clone() });
            },
            Token::EndTag { .. } => {},
            other => {
                self.implied_body();
                self.in_body(other);
            },
        }
    }

    fn in_body(&mut self, tok: Token) {
        match tok {
            Token::Characters(data) => self.in_body_characters(&data),
            Token::Comment(data) => self.insert_comment_here(&data),
            Token::Doctype { .. } => {},
            Token::StartTag { name, attrs, self_closing: _ } => {
                self.in_body_start_tag(&name, attrs)
            },
            Token::EndTag { name } => self.in_body_end_tag(&name),
            Token::Eof => {},
        }
    }

    fn in_body_characters(&mut self, data: &str) {
        if data.is_empty() {
            return;
        }
        self.reconstruct();
        self.frameset_ok = false;
        self.insert_text(data);
    }

    fn in_body_start_tag(&mut self, name: &str, attrs: Vec<(String, String)>) {
        match name {
            "html" => {
                if let Some(&root) = self.open.first() {
                    self.merge_attrs(root, attrs);
                }
            },
            "base" | "basefont" | "bgsound" | "link" | "meta" | "noframes"
            | "script" | "style" | "template" | "title" => {
                self.in_head(Token::StartTag {
                    name: name.to_owned(),
                    attrs,
                    self_closing: false,
                });
            },
            "body" => {
                if let Some(&b) = self
                    .open
                    .iter()
                    .rev()
                    .find(|&&i| self.dom.name(i) == Some("body"))
                {
                    self.merge_attrs(b, attrs);
                }
                self.frameset_ok = false;
            },
            "frameset" => {}, // deviation: ignored
            "address" | "article" | "aside" | "blockquote" | "center"
            | "details" | "dialog" | "dir" | "div" | "dl" | "fieldset"
            | "figcaption" | "figure" | "footer" | "header" | "hgroup"
            | "main" | "menu" | "nav" | "ol" | "p" | "section" | "summary"
            | "ul" => {
                self.close_p_element();
                self.insert_and_push(name, attrs);
            },
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                self.close_p_element();
                if let Some(cur) = self.current() {
                    if matches!(
                        self.dom.name(cur),
                        Some("h1" | "h2" | "h3" | "h4" | "h5" | "h6")
                    ) {
                        self.open.pop();
                    }
                }
                self.insert_and_push(name, attrs);
            },
            "pre" | "listing" => {
                self.close_p_element();
                self.insert_and_push(name, attrs);
                self.skip_next_lf = true;
                self.frameset_ok = false;
            },
            "form" => {
                if self.form.is_none() {
                    self.close_p_element();
                    let idx = self.insert_element("form", attrs);
                    self.open.push(idx);
                    self.form = Some(idx);
                }
            },
            "li" => {
                self.frameset_ok = false;
                self.close_list_item_like(&["li"]);
                self.insert_and_push("li", attrs);
            },
            "dd" | "dt" => {
                self.frameset_ok = false;
                self.close_list_item_like(&["dd", "dt"]);
                self.insert_and_push(name, attrs);
            },
            "plaintext" => {
                // deviation: no PLAINTEXT tokenizer state; ordinary element.
                self.close_p_element();
                self.insert_and_push(name, attrs);
            },
            "button" => {
                if self.has_in_scope("button") {
                    self.generate_implied_end_tags(None);
                    self.pop_until("button");
                }
                self.reconstruct();
                self.insert_and_push("button", attrs);
                self.frameset_ok = false;
            },
            "a" => {
                if self.has_active_formatting("a") {
                    self.adoption_agency("a");
                }
                self.reconstruct();
                let idx = self.insert_and_push("a", attrs.clone());
                self.push_fmt(idx, "a".to_owned(), attrs);
            },
            "b" | "big" | "code" | "em" | "font" | "i" | "s" | "small"
            | "strike" | "strong" | "tt" | "u" => {
                self.reconstruct();
                let owned = name.to_owned();
                let idx = self.insert_and_push(name, attrs.clone());
                self.push_fmt(idx, owned, attrs);
            },
            "nobr" => {
                self.reconstruct();
                if self.has_in_button_scope("nobr") {
                    self.adoption_agency("nobr");
                    self.reconstruct();
                }
                let idx = self.insert_and_push("nobr", attrs.clone());
                self.push_fmt(idx, "nobr".to_owned(), attrs);
            },
            "applet" | "marquee" | "object" => {
                self.reconstruct();
                self.insert_and_push(name, attrs);
                self.fmt.push(FmtEntry::Marker);
                self.frameset_ok = false;
            },
            "table" => {
                self.close_p_element();
                self.insert_and_push("table", attrs);
                self.frameset_ok = false;
                self.mode = Mode::InTable;
            },
            "area" | "br" | "embed" | "img" | "input" | "keygen" | "wbr" => {
                self.reconstruct();
                self.insert_void(name, attrs);
                if name == "br" {
                    self.frameset_ok = false;
                }
            },
            "hr" => {
                self.close_p_element();
                self.insert_void("hr", attrs);
                self.frameset_ok = false;
            },
            "image" => {
                // Parse-error alias rewritten to img (§13.2.6.4.7).
                self.in_body_start_tag("img", attrs);
            },
            "param" | "source" | "track" => {
                self.insert_void(name, attrs);
            },
            "textarea" => {
                self.close_p_element();
                self.insert_and_push("textarea", attrs);
                self.skip_next_lf = true;
                self.orig_mode = Mode::InBody;
                self.mode = Mode::Text;
            },
            "xmp" => {
                self.close_p_element();
                self.insert_and_push("xmp", attrs);
                self.skip_next_lf = true;
                self.orig_mode = Mode::InBody;
                self.mode = Mode::Text;
            },
            "iframe" => self.start_generic_text_element("iframe", attrs),
            "noscript" => self.start_generic_text_element("noscript", attrs),
            "select" => {
                self.reconstruct();
                self.insert_and_push("select", attrs);
                self.frameset_ok = false;
                // deviation: no InSelect mode; stay in body.
            },
            "optgroup" | "option" => {
                loop {
                    let Some(cur) = self.current() else { break };
                    let n = self.dom.name(cur).unwrap_or("");
                    if n == "option" || (name == "optgroup" && n == "optgroup") {
                        self.open.pop();
                    } else {
                        break;
                    }
                }
                self.insert_and_push(name, attrs);
            },
            "rb" | "rtc" => {
                self.generate_implied_end_tags(None);
                self.insert_and_push(name, attrs);
            },
            "rp" | "rt" => {
                self.generate_implied_end_tags(Some("rtc"));
                self.insert_and_push(name, attrs);
            },
            "math" | "svg" => {
                // deviation: foreign content as ordinary unknown elements.
                self.reconstruct();
                self.insert_and_push(name, attrs);
            },
            _ => {
                self.reconstruct();
                self.insert_and_push(name, attrs);
            },
        }
    }

    fn end_body(&mut self) {
        if !self.has_in_scope("body") {
            return;
        }
        self.generate_implied_end_tags(None);
        self.mode = Mode::AfterBody;
    }

    fn in_body_end_tag(&mut self, name: &str) {
        match name {
            "body" => self.end_body(),
            "html" => {
                self.end_body();
                if self.mode == Mode::AfterBody {
                    let n = name.to_owned();
                    self.after_body(Token::EndTag { name: n });
                }
            },
            n if FORMATTING_ELEMENTS.contains(&n) => self.adoption_agency(n),
            "br" => {
                // Parse error: </br> behaves like a <br> start tag.
                self.in_body_start_tag("br", Vec::new());
            },
            "p" => {
                if !self.has_in_button_scope("p") {
                    // Synthesize an empty p (not pushed), then close it.
                    let _ = self.insert_element("p", Vec::new());
                }
                self.close_p_element();
            },
            "applet" | "marquee" | "object" => {
                if self.has_in_scope(name) {
                    self.generate_implied_end_tags(None);
                    self.pop_until(name);
                    self.clear_fmt_to_marker();
                }
            },
            "dd" | "dt" | "li" | "optgroup" | "option" | "rb" | "rp" | "rt" => {
                if self.stack_has(name) {
                    self.generate_implied_end_tags(Some(name));
                    self.pop_until(name);
                }
            },
            "form" => {
                if let Some(f) = self.form {
                    if self.stack_contains(f) {
                        self.generate_implied_end_tags(None);
                        if self.current() == Some(f) {
                            self.open.pop();
                        }
                    }
                    self.form = None;
                }
            },
            "caption" | "table" | "td" | "th" | "tr" | "colgroup" | "tbody"
            | "tfoot" | "thead" => self.any_other_end_tag(name),
            _ => self.any_other_end_tag(name),
        }
    }

    fn text_mode(&mut self, tok: Token) {
        match tok {
            Token::Characters(data) => self.insert_text(&data),
            Token::EndTag { name } => {
                let closes = self
                    .current()
                    .map(|c| self.dom.name(c) == Some(name.as_str()))
                    .unwrap_or(false);
                if closes {
                    self.open.pop();
                    self.mode = self.orig_mode;
                }
            },
            Token::Eof => {
                self.open.pop();
                self.mode = self.orig_mode;
            },
            _ => {},
        }
    }

    fn in_table(&mut self, tok: Token) {
        match tok {
            Token::Characters(data) => {
                if data.chars().all(is_ws_char) {
                    self.insert_text(&data);
                } else {
                    self.foster = true;
                    self.in_body(Token::Characters(data));
                }
            },
            Token::Comment(data) => self.insert_comment_here(&data),
            Token::Doctype { .. } => {},
            Token::StartTag { name, attrs, self_closing: _ } => {
                self.in_table_start_tag(&name, attrs)
            },
            Token::EndTag { name } => self.in_table_end_tag(&name),
            Token::Eof => {},
        }
    }

    fn in_table_start_tag(&mut self, name: &str, attrs: Vec<(String, String)>) {
        match name {
            "caption" => {
                self.clear_back_to_table();
                self.fmt.push(FmtEntry::Marker);
                self.insert_and_push("caption", attrs);
                self.mode = Mode::InCaption;
            },
            "colgroup" => {
                self.clear_back_to_table();
                self.insert_and_push("colgroup", attrs);
                self.mode = Mode::InColumnGroup;
            },
            "col" => {
                self.clear_back_to_table();
                self.insert_and_push("colgroup", Vec::new());
                self.mode = Mode::InColumnGroup;
                self.in_column_group(Token::StartTag {
                    name: "col".to_owned(),
                    attrs,
                    self_closing: false,
                });
            },
            "tbody" | "tfoot" | "thead" => {
                self.clear_back_to_table();
                self.insert_and_push(name, attrs);
                self.mode = Mode::InTableBody;
            },
            "td" | "th" | "tr" => {
                // Implicit tbody (§13.2.6.4.9).
                self.clear_back_to_table();
                self.insert_and_push("tbody", Vec::new());
                self.mode = Mode::InTableBody;
                self.in_table_body_start_tag(name, attrs);
            },
            "table" => {
                if self.has_in_scope("table") {
                    self.pop_until("table");
                    self.reset_insertion_mode();
                    self.process_token(Token::StartTag {
                        name: name.to_owned(),
                        attrs,
                        self_closing: false,
                    });
                }
            },
            "style" | "script" | "title" | "noframes" => {
                self.start_generic_text_element(name, attrs);
            },
            "input" => {
                let hidden = attrs.iter().any(|(n, v)| {
                    n == "type" && v.eq_ignore_ascii_case("hidden")
                });
                if hidden {
                    self.insert_void("input", attrs);
                } else {
                    self.foster = true;
                    self.in_body_start_tag(name, attrs);
                }
            },
            "form" => {
                if self.form.is_none() {
                    let idx = self.insert_element("form", attrs);
                    self.form = Some(idx);
                }
            },
            _ => {
                self.foster = true;
                self.in_body_start_tag(name, attrs);
            },
        }
    }

    fn in_table_end_tag(&mut self, name: &str) {
        match name {
            "table" => {
                if self.has_in_scope("table") {
                    self.pop_until("table");
                    self.reset_insertion_mode();
                }
            },
            "body" | "caption" | "col" | "colgroup" | "html" | "tbody" | "td"
            | "tfoot" | "th" | "thead" | "tr" => {},
            _ => {
                self.foster = true;
                self.in_body_end_tag(name);
            },
        }
    }

    fn in_table_body(&mut self, tok: Token) {
        match tok {
            Token::StartTag { name, attrs, self_closing: _ } => {
                self.in_table_body_start_tag(&name, attrs)
            },
            Token::EndTag { name } => self.in_table_body_end_tag(&name),
            other => {
                self.foster = true;
                self.in_table(other);
            },
        }
    }

    fn in_table_body_start_tag(
        &mut self,
        name: &str,
        attrs: Vec<(String, String)>,
    ) {
        match name {
            "tr" => {
                self.clear_back_to_section();
                self.insert_and_push("tr", attrs);
                self.mode = Mode::InRow;
            },
            "td" | "th" => {
                // Parse error: synthesize the missing row, then reprocess
                // the cell token using the in-row rules.
                self.clear_back_to_section();
                self.insert_and_push("tr", Vec::new());
                self.mode = Mode::InRow;
                self.in_row(Token::StartTag {
                    name: name.to_owned(),
                    attrs,
                    self_closing: false,
                });
            },
            "caption" | "colgroup" | "col" | "tbody" | "tfoot" | "thead" => {
                if self.close_section() {
                    self.reset_insertion_mode();
                    self.in_table_start_tag(name, attrs);
                }
            },
            _ => {
                self.foster = true;
                self.in_table_start_tag(name, attrs);
            },
        }
    }

    fn in_table_body_end_tag(&mut self, name: &str) {
        match name {
            "tbody" | "tfoot" | "thead" => {
                if self.stack_has(name) {
                    self.generate_implied_end_tags(Some(name));
                    self.pop_until(name);
                    self.mode = Mode::InTable;
                }
            },
            "td" | "th" | "tr" => {
                // Stray cell/row end: imply a tbody, then handle the tag.
                let _ = self.insert_element("tbody", Vec::new());
                self.any_other_end_tag(name);
            },
            _ => {
                self.foster = true;
                self.in_table_end_tag(name);
            },
        }
    }

    fn in_row(&mut self, tok: Token) {
        match tok {
            Token::StartTag { name, attrs, self_closing: _ } => {
                match name.as_str() {
                    "td" | "th" => {
                        self.clear_back_to_row();
                        self.fmt.push(FmtEntry::Marker);
                        self.insert_and_push(&name, attrs);
                        self.mode = Mode::InCell;
                    },
                    "caption" | "col" | "colgroup" | "tbody" | "tfoot"
                    | "thead" | "tr" => {
                        if self.close_tr() {
                            self.mode = Mode::InTableBody;
                            self.in_table_body_start_tag(&name, attrs);
                        }
                    },
                    _ => {
                        self.foster = true;
                        self.in_table_start_tag(&name, attrs);
                    },
                }
            },
            Token::EndTag { name } => match name.as_str() {
                "tr" => {
                    if self.close_tr() {
                        self.mode = Mode::InTableBody;
                    }
                },
                "td" | "th" => {
                    if self.stack_has(&name) {
                        self.generate_implied_end_tags(Some(name.as_str()));
                        self.pop_until(&name);
                        self.mode = Mode::InRow;
                    }
                },
                "tbody" | "tfoot" | "thead" => {
                    if self.close_tr() {
                        self.mode = Mode::InTableBody;
                        self.in_table_body_end_tag(&name);
                    }
                },
                _ => {
                    self.foster = true;
                    self.in_table_end_tag(&name);
                },
            },
            other => {
                self.foster = true;
                self.in_table(other);
            },
        }
    }

    fn in_cell(&mut self, tok: Token) {
        match tok {
            Token::StartTag { name, attrs, self_closing: _ } => {
                let cell_open = self.stack_has("td") || self.stack_has("th");
                let is_boundary = matches!(
                    name.as_str(),
                    "caption" | "col" | "colgroup" | "tbody" | "td" | "tfoot"
                        | "th" | "thead" | "tr"
                );
                if is_boundary && cell_open {
                    self.close_cell();
                    self.in_row(Token::StartTag {
                        name,
                        attrs,
                        self_closing: false,
                    });
                } else {
                    self.in_body_start_tag(&name, attrs);
                }
            },
            Token::EndTag { name } => match name.as_str() {
                "td" | "th" => {
                    if self.stack_has(&name) {
                        self.generate_implied_end_tags(Some(name.as_str()));
                        self.pop_until(&name);
                        self.mode = Mode::InRow;
                    }
                },
                "body" | "caption" | "col" | "colgroup" | "html" => {},
                "tbody" | "tfoot" | "thead" | "tr" => {
                    if self.stack_has(&name) {
                        self.pop_until(&name);
                        self.mode = Mode::InRow;
                    }
                },
                _ => self.in_body_end_tag(&name),
            },
            other => self.in_body(other),
        }
    }

    fn in_caption(&mut self, tok: Token) {
        match tok {
            Token::EndTag { ref name } => match name.as_str() {
                "caption" => {
                    if self.stack_has("caption") {
                        self.generate_implied_end_tags(None);
                        self.pop_until("caption");
                        self.clear_fmt_to_marker();
                        self.mode = Mode::InTable;
                    }
                },
                "table" => {
                    if self.has_in_scope("table") {
                        self.generate_implied_end_tags(None);
                        self.pop_until("caption");
                        self.clear_fmt_to_marker();
                        self.mode = Mode::InTable;
                        self.in_table(Token::EndTag { name: name.clone() });
                    }
                },
                "body" | "col" | "colgroup" | "html" | "tbody" | "td"
                | "tfoot" | "th" | "thead" | "tr" => {},
                _ => self.in_body(tok.clone()),
            },
            other => self.in_body(other),
        }
    }

    fn in_column_group(&mut self, tok: Token) {
        let in_colgroup = self
            .current()
            .map(|c| self.dom.name(c))
            == Some(Some("colgroup"));
        match tok {
            Token::Characters(data) => {
                if data.chars().all(is_ws_char) {
                    self.insert_text(&data);
                } else if in_colgroup {
                    self.open.pop();
                    self.mode = Mode::InTable;
                    self.in_table(Token::Characters(data));
                }
            },
            Token::Comment(data) => self.insert_comment_here(&data),
            Token::Doctype { .. } => {},
            Token::StartTag { ref name, attrs, self_closing: _ } => {
                match name.as_str() {
                    "html" => self.in_body_start_tag("html", attrs),
                    "col" => {
                        self.insert_void("col", attrs);
                    },
                    _ => {
                        if in_colgroup {
                            self.open.pop();
                            self.mode = Mode::InTable;
                            self.in_table_start_tag(name, attrs);
                        }
                    },
                }
            },
            Token::EndTag { ref name } => match name.as_str() {
                "colgroup" => {
                    if in_colgroup {
                        self.open.pop();
                        self.mode = Mode::InTable;
                    }
                },
                "col" => {},
                _ => {
                    if in_colgroup {
                        self.open.pop();
                        self.mode = Mode::InTable;
                        self.in_table_end_tag(name);
                    }
                },
            },
            _ => {},
        }
    }

    fn after_body(&mut self, tok: Token) {
        match tok {
            Token::Comment(data) => {
                if let Some(&html) = self.open.first() {
                    self.comment_into(html, &data);
                }
            },
            Token::EndTag { ref name } if name == "html" => {
                self.mode = Mode::AfterAfterBody;
            },
            other => self.in_body(other), // deviation: non-ws reprocessed too
        }
    }

    fn after_after_body(&mut self, tok: Token) {
        match tok {
            Token::Comment(data) => self.comment_into(NodeIdx(0), &data),
            Token::Doctype { .. } => {}
            other => self.in_body(other),
        }
    }
}

/// Convenience: tokenize `src` and run tree construction to completion.
pub fn build(src: &str) -> Dom {
    let mut builder = TreeBuilder::new();
    let mut tokenizer = Tokenizer::new(src.chars().collect());
    loop {
        let tok = tokenizer.next_token();
        let eof = tok == Token::Eof;
        builder.feed(tok);
        if eof {
            break;
        }
    }
    builder.into_dom()
}

#[cfg(test)]
mod tests {
    use super::*;
    use amazing_dom::NodeKind;

    /// Compact s-expression view of a finished tree:
    /// document -> `(doc ...)`, element -> `name[...]`,
    /// text -> `"data"`, comment -> `<!data>`.
    ///
    /// Deviation: pure-whitespace-only text nodes are dropped so expected
    /// strings stay stable against insignificant inter-tag whitespace.
    fn shape(dom: &Dom) -> String {
        let mut out = String::new();
        out.push('(');
        shape_node(dom, NodeIdx(0), &mut out);
        out.push(')');
        out
    }

    fn shape_node(dom: &Dom, idx: NodeIdx, out: &mut String) {
        match &dom.get(idx).kind {
            NodeKind::Document => {
                out.push_str("doc");
                let kids: Vec<NodeIdx> = dom.children(idx).collect();
                if !kids.is_empty() {
                    out.push(' ');
                }
                for k in kids {
                    shape_node(dom, k, out);
                }
            },
            NodeKind::Element { name, .. } => {
                out.push_str(name);
                out.push('[');
                for k in dom.children(idx) {
                    shape_node(dom, k, out);
                }
                out.push(']');
            },
            NodeKind::Text { data } => {
                if data.chars().all(|c| matches!(c, '\t' | '\n' | '\r' | ' ')) {
                    return;
                }
                out.push('"');
                out.push_str(data);
                out.push('"');
            },
            NodeKind::Comment { data } => {
                out.push_str("<!");
                out.push_str(data);
                out.push('>');
            },
        }
    }

    fn build_shape(src: &str) -> String {
        shape(&build(src))
    }

    #[test]
    fn nesting_dump_shape() {
        assert_eq!(
            build_shape("<!DOCTYPE html><title>t</title><p>x"),
            "(doc html[head[title[\"t\"]]body[p[\"x\"]]])"
        );
    }

    #[test]
    fn unclosed_p_yields_siblings() {
        assert_eq!(
            build_shape("<p>one<p>two"),
            "(doc html[head[]body[p[\"one\"]p[\"two\"]]])"
        );
    }

    #[test]
    fn implicit_tbody_row_and_cell() {
        assert_eq!(
            build_shape("<table><tr><td>y"),
            "(doc html[head[]body[table[tbody[tr[td[\"y\"]]]]]])"
        );
    }

    #[test]
    fn bare_table_cell_synthesizes_row_without_recursing() {
        assert_eq!(
            build_shape("<table><th>"),
            "(doc html[head[]body[table[tbody[tr[th[]]]]]])"
        );
    }

    #[test]
    fn li_sequence_closes_previous_li() {
        assert_eq!(
            build_shape("<ul><li>a<li>b</ul>"),
            "(doc html[head[]body[ul[li[\"a\"]li[\"b\"]]]])"
        );
    }

    #[test]
    fn adoption_agency_block_fixture() {
        // <b>1<p>2</b>3</p> -> body > b>"1", p>[ b>"2", "3" ]
        assert_eq!(
            build_shape("<b>1<p>2</b>3</p>"),
            "(doc html[head[]body[b[\"1\"]p[b[\"2\"]\"3\"]]])"
        );
    }

    #[test]
    fn adoption_agency_inline_fixture() {
        // <b>1<i>2</b>3</i> -> body > b>["1", i>"2"], i>"3"
        let dom = build("<b>1<i>2</b>3</i>");
        assert_eq!(shape(&dom), "(doc html[head[]body[b[\"1\"i[\"2\"]]i[\"3\"]]])");
    }

    #[test]
    fn stray_mismatched_end_tag_ignored() {
        assert_eq!(
            build_shape("<div>x</div></div>y"),
            "(doc html[head[]body[div[\"x\"]\"y\"]])"
        );
    }

    #[test]
    fn void_element_takes_no_children() {
        assert_eq!(build_shape("<br>a"), "(doc html[head[]body[br[]\"a\"]])");
    }

    #[test]
    fn bare_text_implies_html_head_body() {
        assert_eq!(build_shape("hello"), "(doc html[head[]body[\"hello\"]])");
    }

    #[test]
    fn comment_lands_under_body() {
        assert_eq!(
            build_shape("<body><!-- hi -->"),
            "(doc html[head[]body[<! hi >]])"
        );
    }

    #[test]
    fn foster_parented_text_lands_before_table() {
        assert_eq!(
            build_shape("<table>x<tr><td>y"),
            "(doc html[head[]body[\"x\"table[tbody[tr[td[\"y\"]]]]]])"
        );
    }

    #[test]
    fn second_anchor_frees_first_via_agency() {
        let dom = build("<a href=\"x\">1<a href=\"y\">2");
        assert_eq!(
            shape(&dom),
            "(doc html[head[]body[a[\"1\"]a[\"2\"]]])"
        );
        let doc = NodeIdx(0);
        let html = dom.get(doc).first_child.expect("html");
        let body = dom.get(html).last_child.expect("body");
        // The two anchor siblings keep their distinct href attributes.
        let hrefs: Vec<String> = dom
            .children(body)
            .filter_map(|k| {
                dom.attrs_of(k).and_then(|a| {
                    a.iter()
                        .find(|(n, _)| n == "href")
                        .map(|(_, v)| v.clone())
                })
            })
            .collect();
        assert_eq!(hrefs, ["x", "y"]);
    }

    #[test]
    fn br_end_tag_becomes_void_br() {
        assert_eq!(build_shape("</br>x"), "(doc html[head[]body[br[]\"x\"]])");
    }

    #[test]
    fn adjacent_characters_merge_into_one_text_node() {
        let dom = build("<p>ab");
        let html = dom.get(NodeIdx(0)).first_child.expect("html");
        let body = dom.get(html).last_child.expect("body");
        let p = dom.get(body).last_child.expect("p");
        let kids: Vec<NodeIdx> = dom.children(p).collect();
        assert_eq!(kids.len(), 1, "expected one merged text node");
        match dom.get(kids[0]).kind {
            NodeKind::Text { ref data } => assert_eq!(data, "ab"),
            ref other => panic!("expected text node, got {:?}", other),
        }
    }

    #[test]
    fn doctype_name_captured_into_dom() {
        let dom = build("<!DOCTYPE html><p>x");
        assert_eq!(dom.doctype.as_deref(), Some("html"));
        assert_eq!(shape(&dom), "(doc html[head[]body[p[\"x\"]]])");
    }
}
