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
//! - `noscript` content is parsed as raw text regardless of scripting flag.
//! - Foreign content implements namespace-aware SVG/MathML tree construction
//!   and integration points, but not every foreign-attribute adjustment.
//! - Doctype mode classification controls quirks-specific tree recovery.
//!
//! The tokenizer owns RAWTEXT/RCDATA switching (script/style/textarea/
//! title), so this module only implements the "text" insertion-mode
//! bookkeeping: insert character tokens into the current node, pop on the
//! appropriate end tag.

use amazing_dom::{Dom, Namespace, NodeIdx, NodeKind};
use amazing_dom::shadow::{ShadowMode, ShadowOptions, SlotAssignment};
use amazing_dom::events::CallbackId;

use crate::tokenizer::{Token, Tokenizer};

// Pure move, no behavior change (see each module's own doc comment and
// `docs/lanes/html_TODO.md`): these hold insertion-mode groups that used to
// live directly in this file, split out to keep it under the workspace's
// 4,000-line cap. Each is a child module so its `impl TreeBuilder` blocks
// keep the same field/const/helper access they had before the split.
mod foreign;
mod in_select;
mod doctype;
mod in_table;

/// Elements whose start tags never take children (void + obsolete empties).
// `in_body_start_tag`'s "area" | "br" | "embed" | "img" | "keygen" | "wbr"
// arm (§13.2.6.4.7, legacy `<keygen>` handling retained by html5lib's own
// corpus) calls `insert_void("keygen", ...)`, whose `debug_assert!` checks
// this list -- `keygen` was missing, so any debug build crashed outright on
// `<keygen>` (html5lib tests19.dat case 66: `<keygen><frameset>` panicked
// instead of producing a void `<keygen>` element).
const VOID_ELEMENTS: &[&str] = &[
    "area", "base", "basefont", "bgsound", "br", "col", "embed", "hr", "img",
    "input", "keygen", "link", "meta", "param", "source", "track", "wbr", "frame",
];

/// Default-scope terminator element names (§13.2.4.3).
const SCOPE_TERMINATORS: &[&str] =
    &["applet", "caption", "html", "table", "td", "th", "marquee", "object", "template"];

/// Extra terminators for "in button scope".
const BUTTON_SCOPE_EXTRA: &[&str] = &["button"];

/// Extra terminators for "in list item scope".
const LIST_ITEM_SCOPE_EXTRA: &[&str] = &["ol", "ul"];

/// Names popped by "generate implied end tags" (§13.2.6.3). `td`/`th` are
/// included per the project plan so cell-closing paths converge.
const IMPLIED_END_TAGS: &[&str] = &[
    "dd", "dt", "li", "optgroup", "option", "p", "rb", "rp", "rt", "rtc", "td", "th",
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

/// Insertion modes (§13.2.4.1). Current WHATWG select recovery is part of the
/// in-body rules.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    Initial,
    BeforeHtml,
    BeforeHead,
    InHead,
    InTemplate,
    AfterHead,
    InBody,
    InSelect,
    Text,
    InTable,
    InTableText,
    InTableBody,
    InRow,
    InCell,
    InCaption,
    InColumnGroup,
    InFrameset,
    AfterFrameset,
    AfterAfterFrameset,
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

/// A parser-created element whose definition existed when its start tag was
/// processed. The element already has the token's attributes but is detached.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParserConstruction {
    pub document: NodeIdx,
    pub element: NodeIdx,
    pub namespace: Namespace,
    pub local_name: String,
    pub definition: usize,
    pub constructor: CallbackId,
}

/// Embedding callback for synchronous custom-element construction. The host
/// calls `Dom::custom_element_super` when its constructor executes `super()`
/// and returns true only if construction returned this same element. It may
/// mutate the supplied DOM; the parser retains no borrow into the DOM across
/// this call. Constructor exceptions are reported by the host and return false.
pub trait ParserCustomElementHost {
    fn construct(&mut self, dom: &mut Dom, request: ParserConstruction) -> bool;
}

/// Tree-construction state machine producing a [`Dom`].
pub struct TreeBuilder {
    dom: Dom,
    custom_element_host: Option<Box<dyn ParserCustomElementHost>>,
    mode: Mode,
    /// Return mode for "text" (the "original insertion mode").
    orig_mode: Mode,
    /// Return mode and pending characters for the "in table text" rules.
    table_text_return_mode: Mode,
    pending_table_text: String,
    /// Stack of open elements; `open[0]` is the `html` element once
    /// synthesized. The document node lives outside this stack.
    open: Vec<NodeIdx>,
    head: Option<NodeIdx>,
    form: Option<NodeIdx>,
    #[allow(dead_code)]
    frameset_ok: bool,
    document_mode: doctype::DocumentMode,
    mode_locked: bool,
    /// pre/listing/textarea: strip one leading U+000A from the next token.
    skip_next_lf: bool,
    /// Foster-parenting flag; set while processing table-fallback content
    /// and cleared after every token.
    foster: bool,
    fmt: Vec<FmtEntry>,
    template_modes: Vec<Mode>,
    shadow_templates: Vec<(NodeIdx, NodeIdx)>,
    allow_declarative_shadow_roots: bool,
    fragment: bool,
    fragment_context: Option<String>,
    /// HTML-40 (2): the fragment context element's own namespace. `build_
    /// fragment` never pushes the context element itself onto `open` (see
    /// its own doc comment) -- the synthetic root at `open[0]` is always an
    /// HTML `html` element -- so this is the only record of it, needed for
    /// `adjusted_current_namespace`'s spec-mandated "adjusted current node"
    /// special case: while `open` still holds only that synthetic root, the
    /// adjusted current node is the (untracked) context element, not it.
    /// Unused (stays `Html`) outside fragment parsing.
    fragment_context_namespace: Namespace,
    /// HTML-31: set by `text_mode`'s `</script>` handling when the just-closed
    /// script is a classic, parser-blocking script during full-document (not
    /// fragment) parsing. `ResumableParser::run` stops tokenizing the instant
    /// this is set so the driver can run the script against the DOM as
    /// constructed so far, exactly per §13.2.6.2's "insertion point" rules.
    pending_script: Option<NodeIdx>,
    /// HTML-31: every `defer`/`async`/module script closed since the last
    /// drain, in document order. These never pause tokenization; the driver
    /// runs them after `Eof` (see `ScriptMode`'s own doc comment).
    script_events: Vec<(NodeIdx, ScriptMode)>,
}

/// HTML-31: how a just-closed classic (or module) `<script>` element should
/// run, mirroring the WHATWG "prepare the script element" classification
/// closely enough for this engine's synchronous, single-threaded driver.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScriptMode {
    /// No `async`, and no (`defer` with a `src`)/`type=module`: the real
    /// parser-blocking case. `ResumableParser::run` pauses tokenization right
    /// after this script's closing tag so the driver can run it synchronously
    /// against the DOM built so far, then resume.
    Blocking,
    /// `defer` with a `src`, or `type=module` without `async`: runs after the
    /// parser reaches `Eof`, in document order, before `DOMContentLoaded`.
    /// Never pauses tokenization.
    Deferred,
    /// `async` present (with or without `src`): runs "as soon as possible",
    /// not guaranteed to preserve document order. This driver has no true
    /// concurrent fetch/parse to race against, so it runs async scripts
    /// immediately after `Eof` too, ahead of `DOMContentLoaded`. Never pauses
    /// tokenization.
    Async,
}

/// A `<script>` is classic JavaScript or a module only when its `type` is
/// absent, empty, a JavaScript MIME type, or exactly `module`; data blocks
/// (`application/ld+json`, `importmap`, ...) never execute and never pause
/// the parser. Returns `Some(is_module)`.
fn classic_or_module_script_type(attrs: &[(String, String)]) -> Option<bool> {
    let raw = attrs.iter().find(|(name, _)| name.eq_ignore_ascii_case("type")).map(|(_, v)| v.as_str());
    // Per spec, "the script block's type string" is the `type` attribute's
    // value stripped of leading/trailing ASCII whitespace ONLY (tab, LF, FF,
    // CR, space -- notably not U+000B VERTICAL TAB, and not any other
    // Unicode whitespace), matched verbatim (no MIME `;parameter` stripping)
    // against the legacy JavaScript type list. `str::trim()` trims every
    // Unicode-whitespace codepoint and `.split(';')` discards parameters,
    // both of which wrongly accepted types like "text/javascript;charset=x"
    // or "text/javascript\u{00A0}" that WPT's script-type-and-language-with-
    // params.html/script-type-whitespace.html expect to be rejected.
    let value = strip_ascii_whitespace(raw.unwrap_or("")).to_ascii_lowercase();
    if value.is_empty() || is_javascript_mime_type(&value) {
        Some(false)
    } else if value == "module" {
        Some(true)
    } else {
        None
    }
}

/// Classifies a just-closed `<script>` element's run mode from its attributes,
/// or `None` if it is not an executable classic/module script at all (data
/// block, unrecognized `type`).
fn classify_script(attrs: &[(String, String)]) -> Option<ScriptMode> {
    let is_module = classic_or_module_script_type(attrs)?;
    let has = |target: &str| attrs.iter().any(|(name, _)| name.eq_ignore_ascii_case(target));
    if has("async") {
        Some(ScriptMode::Async)
    } else if is_module || (has("defer") && has("src")) {
        Some(ScriptMode::Deferred)
    } else {
        Some(ScriptMode::Blocking)
    }
}

/// HTML's "ASCII whitespace": tab, LF, FF, CR, space -- deliberately not
/// `char::is_whitespace()` (which also strips U+00A0, U+1680, U+3000, ...)
/// and deliberately not U+000B VERTICAL TAB, which HTML's own definition
/// excludes even though `char::is_ascii_whitespace()` would also work here
/// (kept spelled out so the exact spec set is visible at the call site).
pub fn strip_ascii_whitespace(s: &str) -> &str {
    s.trim_matches(|c: char| matches!(c, '\t' | '\n' | '\x0C' | '\r' | ' '))
}

/// mimesniff's "JavaScript MIME type" list
/// (<https://mimesniff.spec.whatwg.org/#javascript-mime-type>), the exact
/// authority "prepare the script element" (HTML §13.2.7.4) cites for
/// recognizing a classic script's `type` string. `value` must already be
/// ASCII-lowercased and ASCII-whitespace-trimmed (`strip_ascii_whitespace`)
/// by the caller -- this only does the fixed-set membership check.
/// HTML-32: this list previously had only 8 of the 16 entries (missing
/// `application/x-ecmascript`, `text/x-ecmascript`, and `text/javascript1.0`
/// through `text/javascript1.5`), caught while investigating WPT's
/// `script-type-and-language-js.html`.
pub fn is_javascript_mime_type(value: &str) -> bool {
    matches!(
        value,
        "application/ecmascript"
            | "application/javascript"
            | "application/x-ecmascript"
            | "application/x-javascript"
            | "text/ecmascript"
            | "text/javascript"
            | "text/javascript1.0"
            | "text/javascript1.1"
            | "text/javascript1.2"
            | "text/javascript1.3"
            | "text/javascript1.4"
            | "text/javascript1.5"
            | "text/jscript"
            | "text/livescript"
            | "text/x-ecmascript"
            | "text/x-javascript"
    )
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
            custom_element_host: None,
            mode: Mode::Initial,
            orig_mode: Mode::Initial,
            table_text_return_mode: Mode::Initial,
            pending_table_text: String::new(),
            open: Vec::new(),
            head: None,
            form: None,
            frameset_ok: true,
            document_mode: doctype::DocumentMode::NoQuirks,
            mode_locked: false,
            skip_next_lf: false,
            foster: false,
            fmt: Vec::new(),
            template_modes: Vec::new(),
            shadow_templates: Vec::new(),
            allow_declarative_shadow_roots: true,
            fragment: false,
            fragment_context: None,
            fragment_context_namespace: Namespace::Html,
            pending_script: None,
            script_events: Vec::new(),
        }
    }

    /// Srcdoc documents keep no-quirks mode regardless of their doctype.
    pub(crate) fn set_srcdoc(&mut self) {
        self.mode_locked = true;
    }

    /// Starts a document parse with a preconfigured DOM registry and host.
    pub fn with_custom_element_host(dom: Dom, host: Box<dyn ParserCustomElementHost>) -> Self {
        let mut builder = Self::new();
        builder.dom = dom;
        builder.custom_element_host = Some(host);
        builder
    }

    /// HTML-31: starts a document parse building directly into an existing,
    /// otherwise-fresh `Dom` (as produced by `Dom::new()`), so a caller that
    /// owns the DOM other code already runs against (a JS host's arena) can
    /// hand it in and later reclaim the exact same value via `into_dom`/
    /// `dom_mut` -- see `ResumableParser::swap_dom`.
    pub(crate) fn from_dom(dom: Dom) -> Self {
        let mut builder = Self::new();
        builder.dom = dom;
        builder
    }

    /// HTML-31: the DOM under construction, mutably -- used only by
    /// `ResumableParser::swap_dom` to hand the live tree to a script and take
    /// it back, never to reach in and mutate tree-construction state
    /// directly.
    pub(crate) fn dom_mut(&mut self) -> &mut Dom {
        &mut self.dom
    }

    /// HTML-31: takes the parser-blocking script (if any) that `text_mode`
    /// just recorded when processing the last token. `ResumableParser::run`
    /// checks this after every single token fed and stops immediately when
    /// it is `Some`, before tokenizing anything past that script's closing
    /// tag.
    pub(crate) fn take_pending_script(&mut self) -> Option<NodeIdx> {
        self.pending_script.take()
    }

    /// HTML-31: drains every `defer`/`async`/module script recorded since the
    /// last drain, in document order.
    pub(crate) fn take_script_events(&mut self) -> Vec<(NodeIdx, ScriptMode)> {
        std::mem::take(&mut self.script_events)
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
    pub fn into_dom(mut self) -> Dom {
        self.apply_selectedcontent_mirroring();
        self.dom
    }

    /// Borrows the tree under construction.
    pub fn dom(&self) -> &Dom {
        &self.dom
    }

    // ---- token routing ----

    fn process_token(&mut self, tok: Token) {
        // Pending table text must flush before every non-character token,
        // including the non-HTML processing-instruction extension below.
        if self.mode == Mode::InTableText {
            self.in_table_text(tok);
            return;
        }
        if let Token::ProcessingInstruction { target, data } = &tok {
            let parent = match self.mode {
                Mode::Initial
                | Mode::BeforeHtml
                | Mode::AfterAfterBody
                | Mode::AfterAfterFrameset => NodeIdx(0),
                Mode::AfterBody | Mode::AfterFrameset => {
                    self.open.first().copied().unwrap_or(NodeIdx(0))
                },
                _ => self.insertion_place().0,
            };
            self.processing_instruction_into(parent, target, data);
            return;
        }
        if self.should_process_in_foreign_content(&tok) {
            self.in_foreign_content(tok);
            return;
        }
        self.process_token_in_html(tok);
    }

    fn process_token_in_html(&mut self, tok: Token) {
        if self.mode != Mode::Text && self.mode != Mode::InTableText {
            if tok == Token::Eof && self.stack_has("template") {
                self.close_template();
                self.process_token_in_html(tok);
                return;
            }
            // §13.2.6: only the modes that actually redirect a `template`
            // tag to "in head" rules (directly, or by cascading through "in
            // table"/"in caption"/etc., which this shortcut short-circuits)
            // belong here. `InSelect`, `InFrameset`, `AfterFrameset`,
            // `AfterBody` and the `AfterAfter*` modes have no such rule --
            // per spec `template` there is just an ordinary, unhandled
            // token, ignored like any other. Reusing this shortcut for them
            // wrongly opened/closed a template while, e.g., inside a
            // `<frameset>` (html5lib template.dat case 40:
            // `<frameset><template><frame></template></frameset>` swallowed
            // the `<frame>` into an unwanted template instead of leaving it
            // a plain child of `<frameset>`).
            if matches!(
                self.mode,
                Mode::InBody
                    | Mode::InTable
                    | Mode::InCaption
                    | Mode::InColumnGroup
                    | Mode::InTableBody
                    | Mode::InRow
                    | Mode::InCell
                    | Mode::InTemplate
            ) {
                match &tok {
                    Token::StartTag { name, attrs, .. } if name == "template" => {
                        self.start_template(attrs.clone());
                        return;
                    },
                    Token::EndTag { name } if name == "template" => {
                        self.close_template();
                        return;
                    },
                    _ => {},
                }
            }
        }
        let self_closing_foreign_root = match &tok {
            Token::StartTag { name, self_closing: true, .. } if name == "svg" => {
                Some((Namespace::Svg, "svg"))
            },
            Token::StartTag { name, self_closing: true, .. } if name == "math" => {
                Some((Namespace::MathMl, "math"))
            },
            _ => None,
        };
        match self.mode {
            Mode::Initial => self.initial(tok),
            Mode::BeforeHtml => self.before_html(tok),
            Mode::BeforeHead => self.before_head(tok),
            Mode::InHead => self.in_head(tok),
            Mode::InTemplate => self.in_template(tok),
            Mode::AfterHead => self.after_head(tok),
            Mode::InBody => self.in_body(tok),
            Mode::InSelect => self.in_select(tok),
            Mode::Text => self.text_mode(tok),
            Mode::InTable => self.in_table(tok),
            Mode::InTableText => unreachable!("handled before token routing"),
            Mode::InTableBody => self.in_table_body(tok),
            Mode::InRow => self.in_row(tok),
            Mode::InCell => self.in_cell(tok),
            Mode::InCaption => self.in_caption(tok),
            Mode::InColumnGroup => self.in_column_group(tok),
            Mode::InFrameset => self.in_frameset(tok),
            Mode::AfterFrameset => self.after_frameset(tok),
            Mode::AfterAfterFrameset => self.after_after_frameset(tok),
            Mode::AfterBody => self.after_body(tok),
            Mode::AfterAfterBody => self.after_after_body(tok),
        }
        if let Some((namespace, name)) = self_closing_foreign_root {
            if self.current().is_some_and(|current| {
                self.dom.namespace(current) == Some(namespace)
                    && self.dom.name(current) == Some(name)
            }) {
                self.open.pop();
            }
        }
    }

    // ---- small queries ----

    fn current(&self) -> Option<NodeIdx> {
        self.open.last().copied()
    }

    /// The namespace of the spec's "adjusted current node"
    /// (<https://html.spec.whatwg.org/multipage/parsing.html#adjusted-current-node>):
    /// the fragment context element's own namespace, if this is a fragment
    /// parse and `open` still holds only its synthetic (always-HTML) root
    /// -- that root is not a real stack member, see `build_fragment`'s own
    /// doc comment -- otherwise the current node's own namespace. HTML-40
    /// (2): only `feed_with_tokenizer_feedback`'s CDATA-allowed check uses
    /// this so far; the foreign-content dispatch helpers below
    /// (`is_mathml_text_integration_point` etc.) still key off `current()`
    /// directly, unaffected in practice since a fragment's synthetic root
    /// is never a real integration point/foreign element by construction.
    fn adjusted_current_namespace(&self) -> Option<Namespace> {
        if self.fragment && self.open.len() == 1 {
            Some(self.fragment_context_namespace)
        } else {
            self.current().and_then(|idx| self.dom.namespace(idx))
        }
    }

    /// The spec's "adjusted current node" as a (namespace, local name) pair
    /// rather than a `NodeIdx`: for a fragment parse where `open` still
    /// holds only the synthetic (always-HTML) root -- never a real stack
    /// member, see `build_fragment`'s own doc comment -- this is the
    /// fragment context element itself, which exists only as this name/
    /// namespace pair and never gets a real `NodeIdx` of its own.
    /// Otherwise it's simply the current node's own identity. Used by
    /// `foreign.rs`'s foreign-content dispatch, which otherwise keyed off
    /// `current()` directly and so treated a foreign (svg/math) fragment
    /// context, with nothing else yet on the stack, as HTML (html5lib/WPT
    /// `foreign-fragment.dat`).
    pub(super) fn adjusted_current(&self) -> Option<(Namespace, &str)> {
        if self.fragment && self.open.len() == 1 {
            self.fragment_context.as_deref().map(|name| {
                // The context string (e.g. from `parse-fragment`'s CLI
                // argument) is lowercased on the way in
                // (`parse_fragment_inner`), unlike a real inserted SVG
                // element's name, which goes through `adjusted_svg_tag_name`
                // to restore camelCase spellings such as "foreignObject".
                // Without the same adjustment here, an SVG `foreignObject`
                // fragment context would fail the case-sensitive HTML-
                // integration-point name match below.
                let name = if self.fragment_context_namespace == Namespace::Svg {
                    Self::adjusted_svg_tag_name(name)
                } else {
                    name
                };
                (self.fragment_context_namespace, name)
            })
        } else {
            let idx = self.current()?;
            Some((self.dom.namespace(idx)?, self.dom.name(idx)?))
        }
    }

    /// The adjusted current node's attributes -- `None` for the synthetic
    /// fragment-context case (see `adjusted_current`), since the fragment
    /// context has no real `NodeIdx` to look attributes up on.
    pub(super) fn adjusted_current_attrs(&self) -> Option<&[(String, String)]> {
        if self.fragment && self.open.len() == 1 {
            None
        } else {
            self.current().and_then(|idx| self.dom.attrs_of(idx))
        }
    }

    fn stack_has(&self, name: &str) -> bool {
        self.open.iter().any(|&i| {
            self.dom.namespace(i) == Some(Namespace::Html)
                && self.dom.name(i) == Some(name)
        })
    }

    fn stack_contains(&self, idx: NodeIdx) -> bool {
        self.open.contains(&idx)
    }

    fn is_special(&self, idx: NodeIdx) -> bool {
        match (self.dom.namespace(idx), self.dom.name(idx)) {
            (Some(Namespace::Html), Some(name)) => SPECIAL_ELEMENTS.contains(&name),
            (Some(Namespace::MathMl), Some(name)) => matches!(
                name,
                "mi" | "mo" | "mn" | "ms" | "mtext" | "annotation-xml"
            ),
            (Some(Namespace::Svg), Some(name)) => {
                matches!(name, "foreignObject" | "desc" | "title")
            },
            _ => false,
        }
    }

    fn elem_name_attrs(&self, idx: NodeIdx) -> (String, Vec<(String, String)>) {
        match self.dom.get(idx).kind {
            NodeKind::Element { ref name, ref attrs, .. } => {
                (name.clone(), attrs.clone())
            },
            _ => (String::new(), Vec::new()),
        }
    }

    fn adjusted_svg_tag_name(name: &str) -> &str {
        match name {
            "altglyph" => "altGlyph",
            "altglyphdef" => "altGlyphDef",
            "altglyphitem" => "altGlyphItem",
            "animatecolor" => "animateColor",
            "animatemotion" => "animateMotion",
            "animatetransform" => "animateTransform",
            "clippath" => "clipPath",
            "feblend" => "feBlend",
            "fecolormatrix" => "feColorMatrix",
            "fecomponenttransfer" => "feComponentTransfer",
            "fecomposite" => "feComposite",
            "feconvolvematrix" => "feConvolveMatrix",
            "fediffuselighting" => "feDiffuseLighting",
            "fedisplacementmap" => "feDisplacementMap",
            "fedistantlight" => "feDistantLight",
            "fedropshadow" => "feDropShadow",
            "feflood" => "feFlood",
            "fefunca" => "feFuncA",
            "fefuncb" => "feFuncB",
            "fefuncg" => "feFuncG",
            "fefuncr" => "feFuncR",
            "fegaussianblur" => "feGaussianBlur",
            "feimage" => "feImage",
            "femerge" => "feMerge",
            "femergenode" => "feMergeNode",
            "femorphology" => "feMorphology",
            "feoffset" => "feOffset",
            "fepointlight" => "fePointLight",
            "fespecularlighting" => "feSpecularLighting",
            "fespotlight" => "feSpotLight",
            "fetile" => "feTile",
            "feturbulence" => "feTurbulence",
            "foreignobject" => "foreignObject",
            "glyphref" => "glyphRef",
            "lineargradient" => "linearGradient",
            "radialgradient" => "radialGradient",
            "textpath" => "textPath",
            _ => name,
        }
    }

    fn adjusted_svg_attribute_name(name: &str) -> &str {
        match name {
            "attributename" => "attributeName",
            "attributetype" => "attributeType",
            "basefrequency" => "baseFrequency",
            "baseprofile" => "baseProfile",
            "calcmode" => "calcMode",
            "clippathunits" => "clipPathUnits",
            "diffuseconstant" => "diffuseConstant",
            "edgemode" => "edgeMode",
            "filterunits" => "filterUnits",
            "glyphref" => "glyphRef",
            "gradienttransform" => "gradientTransform",
            "gradientunits" => "gradientUnits",
            "kernelmatrix" => "kernelMatrix",
            "kernelunitlength" => "kernelUnitLength",
            "keypoints" => "keyPoints",
            "keysplines" => "keySplines",
            "keytimes" => "keyTimes",
            "lengthadjust" => "lengthAdjust",
            "limitingconeangle" => "limitingConeAngle",
            "markerheight" => "markerHeight",
            "markerunits" => "markerUnits",
            "markerwidth" => "markerWidth",
            "maskcontentunits" => "maskContentUnits",
            "maskunits" => "maskUnits",
            "numoctaves" => "numOctaves",
            "pathlength" => "pathLength",
            "patterncontentunits" => "patternContentUnits",
            "patterntransform" => "patternTransform",
            "patternunits" => "patternUnits",
            "pointsatx" => "pointsAtX",
            "pointsaty" => "pointsAtY",
            "pointsatz" => "pointsAtZ",
            "preservealpha" => "preserveAlpha",
            "preserveaspectratio" => "preserveAspectRatio",
            "primitiveunits" => "primitiveUnits",
            "refx" => "refX",
            "refy" => "refY",
            "repeatcount" => "repeatCount",
            "repeatdur" => "repeatDur",
            "requiredextensions" => "requiredExtensions",
            "requiredfeatures" => "requiredFeatures",
            "specularconstant" => "specularConstant",
            "specularexponent" => "specularExponent",
            "spreadmethod" => "spreadMethod",
            "startoffset" => "startOffset",
            "stddeviation" => "stdDeviation",
            "stitchtiles" => "stitchTiles",
            "surfacescale" => "surfaceScale",
            "systemlanguage" => "systemLanguage",
            "tablevalues" => "tableValues",
            "targetx" => "targetX",
            "targety" => "targetY",
            "textlength" => "textLength",
            "viewbox" => "viewBox",
            "viewtarget" => "viewTarget",
            "xchannelselector" => "xChannelSelector",
            "ychannelselector" => "yChannelSelector",
            "zoomandpan" => "zoomAndPan",
            _ => name,
        }
    }

    /// WHATWG "adjust foreign attributes" table. Names outside this table,
    /// even names containing a colon, retain a null namespace.
    fn foreign_attribute_identity(name: &str) -> Option<(&'static str, Option<&'static str>, &'static str)> {
        const XLINK: &str = "http://www.w3.org/1999/xlink";
        const XML: &str = "http://www.w3.org/XML/1998/namespace";
        const XMLNS: &str = "http://www.w3.org/2000/xmlns/";
        match name {
            "xlink:actuate" => Some((XLINK, Some("xlink"), "actuate")),
            "xlink:arcrole" => Some((XLINK, Some("xlink"), "arcrole")),
            "xlink:href" => Some((XLINK, Some("xlink"), "href")),
            "xlink:role" => Some((XLINK, Some("xlink"), "role")),
            "xlink:show" => Some((XLINK, Some("xlink"), "show")),
            "xlink:title" => Some((XLINK, Some("xlink"), "title")),
            "xlink:type" => Some((XLINK, Some("xlink"), "type")),
            "xml:lang" => Some((XML, Some("xml"), "lang")),
            "xml:space" => Some((XML, Some("xml"), "space")),
            "xmlns" => Some((XMLNS, None, "xmlns")),
            "xmlns:xlink" => Some((XMLNS, Some("xmlns"), "xlink")),
            _ => None,
        }
    }

    fn insert_foreign_element(
        &mut self,
        name: &str,
        mut attrs: Vec<(String, String)>,
        namespace: Namespace,
    ) -> NodeIdx {
        let local_name = if namespace == Namespace::Svg {
            Self::adjusted_svg_tag_name(name)
        } else {
            name
        };
        for (attribute_name, _) in &mut attrs {
            let adjusted = match namespace {
                Namespace::Svg => Self::adjusted_svg_attribute_name(attribute_name),
                Namespace::MathMl if attribute_name == "definitionurl" => {
                    "definitionURL"
                },
                _ => attribute_name,
            };
            if adjusted != attribute_name {
                *attribute_name = adjusted.to_owned();
            }
        }
        let mut ordinary_attrs = Vec::new();
        let mut expanded_attrs = Vec::new();
        for (name, value) in attrs {
            if let Some((uri, prefix, local_name)) = Self::foreign_attribute_identity(&name) {
                expanded_attrs.push((uri, prefix, local_name, value));
            } else {
                ordinary_attrs.push((name, value));
            }
        }
        let (parent, before) = self.insertion_place();
        let idx = self.dom.alloc_unattached(NodeKind::Element {
            name: local_name.to_owned(),
            attrs: ordinary_attrs,
            namespace,
        });
        for (uri, prefix, local_name, value) in expanded_attrs {
            self.dom.set_attribute_expanded(idx, Some(uri), prefix, local_name, &value)
                .expect("foreign attribute table has valid expanded identities");
        }
        self.dom.attach_before(parent, before, idx);
        self.open.push(idx);
        idx
    }

    /// Generic scope walk (§13.2.4.3): scan top-down; terminate on a match
    /// or on any terminator.
    fn has_in_specific_scope(&self, name: &str, extra: &[&str]) -> bool {
        for &node in self.open.iter().rev() {
            let n = self.dom.name(node).unwrap_or("");
            let namespace = self.dom.namespace(node);
            if namespace == Some(Namespace::Html) && n == name {
                return true;
            }
            let is_terminator = match namespace {
                Some(Namespace::Html) => {
                    SCOPE_TERMINATORS.contains(&n) || extra.contains(&n)
                },
                Some(Namespace::MathMl) => matches!(
                    n,
                    "mi" | "mo" | "mn" | "ms" | "mtext" | "annotation-xml"
                ),
                Some(Namespace::Svg) => {
                    matches!(n, "foreignObject" | "desc" | "title")
                },
                Some(Namespace::Null | Namespace::Other(_)) => false,
                None => false,
            };
            if is_terminator {
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

    fn has_in_table_scope(&self, name: &str) -> bool {
        for &node in self.open.iter().rev() {
            if self.dom.namespace(node) != Some(Namespace::Html) {
                continue;
            }
            let local = self.dom.name(node).unwrap_or("");
            if local == name {
                return true;
            }
            if matches!(local, "html" | "table" | "template") {
                return false;
            }
        }
        false
    }

    #[allow(dead_code)]
    fn has_in_list_item_scope(&self, name: &str) -> bool {
        self.has_in_specific_scope(name, LIST_ITEM_SCOPE_EXTRA)
    }

    // ---- insertion primitives (§13.2.6.1) ----

    /// Appropriate insertion location `(parent, before-sibling)`. Honours
    /// the foster flag when the current node is table family: the node lands
    /// immediately before the last `table` on the open-element stack.
    fn adjusted_insertion_place_for(
        &self,
        target: NodeIdx,
    ) -> (NodeIdx, Option<NodeIdx>) {
        if self.foster {
            let family = {
                let n = self.dom.name(target).unwrap_or("");
                matches!(n, "table" | "tbody" | "tfoot" | "thead" | "tr")
            };
            if family {
                let last_template_pos = self.open.iter()
                    .rposition(|&node| self.dom.name(node) == Some("template"));
                let last_table_pos = self.open.iter()
                    .rposition(|&node| self.dom.name(node) == Some("table"));
                // §13.2.6.1 foster-parenting step 3: a `template` more
                // recent (higher on the stack) than any `table` -- or with
                // no `table` on the stack at all -- redirects insertion
                // into THAT template's own content fragment, not the
                // nearest `table`. This is the normal case for a `<tr>`/
                // `<td>`/etc. reached only through "in template"'s synthetic
                // table-context modes (`in_table`/`in_row`/`in_cell`, ...),
                // which never push a real `<table>` element at all (html5lib
                // template.dat: `<body><template><tr><div></div></tr>
                // </template>` foster-parented `<div>` as a *child of `<tr>`*
                // instead of the template's content, since the old code
                // fell straight through to `(target, None)` whenever no
                // `<table>` was found).
                let template_more_recent = match (last_template_pos, last_table_pos) {
                    (Some(t), Some(tb)) => t > tb,
                    (Some(_), None) => true,
                    (None, _) => false,
                };
                if template_more_recent {
                    let template = self.open[last_template_pos.expect("checked above")];
                    if let Some(contents) = self.dom.template_contents(template) {
                        return (contents, None);
                    }
                }
                if let Some(table_pos) = last_table_pos {
                    let table = self.open[table_pos];
                    if let Some(parent) = self.dom.get(table).parent {
                        return (parent, Some(table));
                    }
                    if table_pos > 0 {
                        return (self.open[table_pos - 1], None);
                    }
                } else if let Some(&html) = self.open.first() {
                    // Step 4: no `table` on the stack at all (only possible
                    // during fragment parsing, per spec) -- foster-parent
                    // into the synthetic `html` root.
                    return (html, None);
                }
            }
        }
        let target = self.shadow_templates.iter().find(|(template, _)| *template == target)
            .map(|(_, root)| *root).or_else(|| self.dom.template_contents(target)).unwrap_or(target);
        (target, None)
    }

    fn insertion_place(&self) -> (NodeIdx, Option<NodeIdx>) {
        let Some(&target) = self.open.last() else {
            return (NodeIdx(0), None);
        };
        let (parent, before) = self.adjusted_insertion_place_for(target);
        let parent = self.shadow_templates.iter().find(|(template, _)| *template == parent)
            .map(|(_, root)| *root).or_else(|| self.dom.template_contents(parent)).unwrap_or(parent);
        (parent, before)
    }

    fn alloc_element(&mut self, name: &str, attrs: Vec<(String, String)>, document: NodeIdx) -> NodeIdx {
        self.dom.create_node_in(document, NodeKind::Element {
                name: name.to_owned(),
                attrs,
                namespace: Namespace::Html,
            }).expect("parser document")
    }

    /// Creates an element at the appropriate insertion place WITHOUT
    /// pushing it onto the stack of open elements.
    fn insert_element(
        &mut self,
        name: &str,
        attrs: Vec<(String, String)>,
    ) -> NodeIdx {
        let (parent, before) = self.insertion_place();
        let document = self.dom.owner_document(parent).unwrap_or(NodeIdx(0));
        let is_name = attrs.iter().find(|(name, _)| name == "is").map(|(_, value)| value.clone());
        let idx = self.alloc_element(name, Vec::new(), document);
        // The registry is owned by DOM. A missing host or definition keeps the
        // ordinary parser path, leaving an undefined element upgradeable later.
        let definition = if self.custom_element_host.is_some() {
            self.dom.custom_element_definitions(document).into_iter()
                .find(|(_, definition)| definition.local_name == name
                    && (definition.name == name || is_name.as_deref() == Some(definition.name.as_str())))
                .map(|(id, _)| id)
        } else {
            None
        };
        let construction = definition.and_then(|definition| {
            self.dom.begin_custom_element_upgrade(idx, definition).ok().map(|constructor| {
                let request = ParserConstruction {
                    document, element: idx, namespace: Namespace::Html,
                    local_name: name.to_owned(), definition, constructor,
                };
                let constructed = self.custom_element_host.as_mut().expect("host checked")
                    .construct(&mut self.dom, request);
                (definition, constructed)
            })
        });
        for (name, value) in attrs {
            self.dom.set_attribute(idx, &name, &value).expect("parser attribute");
        }
        if let Some((definition, constructed)) = construction {
            // Initial attribute reactions precede connected reactions, and
            // both precede insertion of the token's children.
            let _ = self.dom.finish_custom_element_upgrade(idx, definition, constructed);
        }
        self.dom.attach_before(parent, before, idx);
        // With no host, a preexisting definition remains eligible for a
        // later DOM-14 upgrade. With no definition this is a no-op.
        if construction.is_none() { self.dom.queue_custom_element_upgrade(idx); }
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

    fn processing_instruction_into(
        &mut self,
        parent: NodeIdx,
        target: &str,
        data: &str,
    ) {
        let instruction = self.dom.alloc_unattached(
            NodeKind::ProcessingInstruction {
                target: target.to_owned(),
                data: data.to_owned(),
            },
        );
        self.dom.attach_last(parent, instruction);
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
        while let Some(&cur) = self.open.last() {
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
            if self.dom.namespace(idx) == Some(Namespace::Html)
                && self.dom.name(idx) == Some(name)
            {
                break;
            }
        }
    }

    fn pop_until_any(&mut self, names: &[&str]) {
        while let Some(idx) = self.open.pop() {
            if self.dom.namespace(idx) == Some(Namespace::Html)
                && matches!(self.dom.name(idx), Some(n) if names.contains(&n))
            {
                break;
            }
        }
    }

    /// Reset the insertion mode by walking the open-element stack top-down
    /// (§13.2.4.1).
    fn reset_insertion_mode(&mut self) {
        for &node in self.open.iter().rev() {
            let n = self.dom.name(node).unwrap_or("");
            let next = match n {
                "select" => Some(Mode::InBody), // current spec: in-body recovery
                "td" | "th" => Some(Mode::InCell),
                "tr" => Some(Mode::InRow),
                "tbody" | "tfoot" | "thead" => Some(Mode::InTableBody),
                "caption" => Some(Mode::InCaption),
                "colgroup" => Some(Mode::InColumnGroup),
                "table" => Some(Mode::InTable),
                "template" => Some(*self.template_modes.last().unwrap_or(&Mode::InTemplate)),
                "frameset" => Some(Mode::InFrameset),
                "body" => Some(Mode::InBody),
                // Spec step 12 (§13.2.4.1): a `head` element still open on the
                // stack resets to "in head", not "after head". This case was
                // missing entirely, so `head` fell through to `_ => None` and
                // the loop continued to `html`, whose own check only looks at
                // whether the head element *pointer* (`self.head`) was ever
                // set -- true as soon as the first head element was created,
                // even while that same head is still open above it on the
                // stack (e.g. right after `close_template` pops a `<template>`
                // that was a direct child of `<head>`). That produced "after
                // head" while `<head>` was still the current node, so the
                // next implied `<body>` was inserted as `<head>`'s own child
                // instead of a sibling of `<head>` under `<html>` (html5lib
                // template.dat: ~30 of this file's cases had a `<body>` one
                // level too deep for exactly this reason).
                "head" => Some(Mode::InHead),
                "html" if self.fragment => Some(fragment_insertion_mode(
                    self.fragment_context.as_deref().unwrap_or("html")
                )),
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
                _ => {},
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

    /// Runs the spec-bounded outer loop (at most eight reparenting passes).
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
                if last_node_pos <= fe_stack_pos {
                    break;
                }
                let cand_pos = last_node_pos - 1;
                let cand = self.open[cand_pos];
                if cand == fe_idx {
                    break;
                }
                let mut in_fmt = self.fmt_pos_by_idx(cand);
                if inner > 3 {
                    if let Some(p) = in_fmt {
                        self.fmt.remove(p);
                        if bookmark > p {
                            bookmark -= 1;
                        }
                        in_fmt = None;
                    }
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
                    namespace: Namespace::Html,
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

            // Move lastNode to the adjusted insertion location for the
            // common ancestor. This is foster-parented when the agency was
            // entered through the table fallback rules.
            //
            // Two ways this can go wrong once a script has mutated the tree
            // from underneath this algorithm (the open-elements stack still
            // points at nodes script may have relocated before the parser
            // resumed):
            //
            // - The insertion location can end up inside `last_node`'s own
            //   subtree (e.g. a formatting element's furthest block got
            //   re-parented into one of its own descendants via a shadow
            //   root or a `<template>`'s inert contents). Performing the
            //   move as written would create a cycle -- attach a node under
            //   itself.
            // - Foster-parenting resolves `parent` from the last `<table>`
            //   on the stack's *current* DOM parent, which script can have
            //   replaced with a `Document` that already has a document
            //   element (e.g. moving that `<table>` into a second, freshly
            //   created `Document`). A `Document` may only ever have one
            //   element child.
            //
            // Both are a no-op per the general "insert a node" step the
            // spec's tree construction relies on throughout, which silently
            // drops a node whose target has become unreachable or already
            // full rather than erroring; `last_node` is left detached.
            self.dom.detach(last_node);
            let (parent, before) = self.adjusted_insertion_place_for(common);
            let cyclic = self.dom.is_host_including_inclusive_ancestor(last_node, parent);
            let document_full = matches!(self.dom.get(parent).kind, NodeKind::Document)
                && self.dom.document_element(parent).ok().flatten().is_some();
            if !cyclic && !document_full {
                self.dom.attach_before(parent, before, last_node);
            }

            // Clone the formatting element; adopt furthest block's children.
            let (fname, fattrs) = self.elem_name_attrs(fe_idx);
            let fe_clone = self.dom.alloc_unattached(NodeKind::Element {
                name: fname.clone(),
                attrs: fattrs.clone(),
                namespace: Namespace::Html,
            });
            let children: Vec<NodeIdx> = self.dom.children(fb_idx).collect();
            for child in children {
                self.dom.detach(child);
                self.dom.attach_last(fe_clone, child);
            }
            self.dom.attach_last(fb_idx, fe_clone);

            // Patch the fmt list at the bookmark.
            if let Some(p) = self.fmt_pos_by_idx(fe_idx) {
                self.fmt.remove(p);
                if bookmark > p {
                    bookmark -= 1;
                }
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
            self.open.insert(fb_now.saturating_add(1), fe_clone);

        }
    }

    /// "Any other end tag": walk the stack top-down; on the first name match
    /// generate implied end tags (except the subject) and pop inclusively.
    fn any_other_end_tag(&mut self, name: &str) {
        for i in (0..self.open.len()).rev() {
            let node = self.open[i];
            if self.dom.namespace(node) == Some(Namespace::Html)
                && self.dom.name(node) == Some(name)
            {
                self.generate_implied_end_tags(Some(name));
                self.pop_until(name);
                return;
            }
            if self.is_special(node) {
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

    // Per spec, each of these three "clear the stack back to a ... context"
    // algorithms stops at a `template` element as well as its own named
    // targets and `html` -- not just when a `table` is present. Without a
    // `<table>` on the open-elements stack (e.g. a `<tbody>`/`<tr>`/`<td>`
    // start tag reached via the "in template" insertion mode's own table-
    // context redirect, with no enclosing `<table>`), omitting `template`
    // here popped the template element itself (and anything above it) off
    // the stack, so the element then inserted outside `template.content`
    // entirely instead of inside it.
    fn clear_back_to_table(&mut self) {
        while let Some(&cur) = self.open.last() {
            if matches!(self.dom.name(cur), Some("table" | "template" | "html")) {
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
                    | Some("template") | Some("html")
            ) {
                break;
            }
            self.open.pop();
        }
    }

    fn clear_back_to_row(&mut self) {
        while let Some(&cur) = self.open.last() {
            if matches!(self.dom.name(cur), Some("tr") | Some("table") | Some("template") | Some("html")) {
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
        if self.stack_has("td") || self.stack_has("th") {
            self.pop_until_any(&["td", "th"]);
        }
        self.clear_fmt_to_marker();
        self.mode = Mode::InRow;
    }

    // ---- insertion modes ----

    fn initial(&mut self, tok: Token) {
        match tok {
            Token::Doctype { name, public_id, system_id, force_quirks } => {
                if !self.mode_locked {
                    self.document_mode = doctype::classify(name.as_deref(), public_id.as_deref(),
                        system_id.as_deref(), force_quirks);
                }
                self.dom.doctype = name.clone();
                self.dom.append_child(NodeIdx(0), NodeKind::DocumentType {
                    name: name.unwrap_or_default(),
                    public_id: public_id.unwrap_or_default(),
                    system_id: system_id.unwrap_or_default(),
                });
                self.mode = Mode::BeforeHtml;
            },
            Token::Characters(data) => {
                if !data.chars().all(is_ws_char) {
                    if !self.mode_locked { self.document_mode = doctype::DocumentMode::Quirks; }
                    self.mode = Mode::BeforeHtml;
                    self.before_html(Token::Characters(data));
                }
            },
            Token::Comment(data) => self.comment_into(NodeIdx(0), &data),
            other => {
                if !self.mode_locked { self.document_mode = doctype::DocumentMode::Quirks; }
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
                let first_non_whitespace = data
                    .char_indices()
                    .find_map(|(index, c)| (!is_ws_char(c)).then_some(index));
                if let Some(index) = first_non_whitespace {
                    self.implied_head();
                    self.in_head(Token::Characters(data[index..].to_owned()));
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
            // §13.2.6.4.3 "before head", "any other end tag": parse error,
            // ignore the token -- this must NOT fall into the catch-all
            // below, which implicitly creates `<head>` and reprocesses.
            // Doing that for a stray end tag (e.g. a bare `</p>` before any
            // real content) created `<head>` a token early, so a comment
            // immediately after landed *inside* it instead of staying a
            // sibling of the not-yet-created `<head>` under `<html>`
            // (html5lib tests19.dat case 2: `<html></p><!--foo-->`).
            Token::EndTag { .. } => {},
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

    fn start_template(&mut self, attrs: Vec<(String, String)>) {
        let mode = attrs.iter().find(|(name, _)| name == "shadowrootmode")
            .and_then(|(_, value)| match value.to_ascii_lowercase().as_str() {
                "open" => Some(ShadowMode::Open), "closed" => Some(ShadowMode::Closed), _ => None,
            });
        let shadow = if self.allow_declarative_shadow_roots && self.open.len() > 1 {
            mode.and_then(|mode| {
                let host = self.current()?;
                let has = |name| attrs.iter().any(|(attribute, _)| attribute == name);
                let manual = attrs.iter().any(|(name, value)| name == "shadowrootslotassignment"
                    && value.eq_ignore_ascii_case("manual"));
                let options = ShadowOptions {
                    mode, delegates_focus: has("shadowrootdelegatesfocus"),
                    slot_assignment: if manual { SlotAssignment::Manual } else { SlotAssignment::Named },
                    clonable: has("shadowrootclonable"), serializable: has("shadowrootserializable"),
                };
                self.dom.attach_declarative_shadow(
                    host, options, has("shadowrootcustomelementregistry")
                ).ok()
            })
        } else { None };
        let template = if let Some(root) = shadow {
            let document = self.dom.owner_document(root).unwrap_or(NodeIdx(0));
            let template = self.alloc_element("template", attrs, document);
            self.open.push(template);
            self.shadow_templates.push((template, root));
            template
        } else {
            self.insert_and_push("template", attrs)
        };
        self.dom.ensure_template_contents(template).expect("HTML template contents");
        self.fmt.push(FmtEntry::Marker);
        self.frameset_ok = false;
        self.mode = Mode::InTemplate;
        self.template_modes.push(Mode::InTemplate);
    }

    fn close_template(&mut self) {
        if !self.stack_has("template") { return; }
        self.generate_implied_end_tags(None);
        self.pop_until("template");
        self.clear_fmt_to_marker();
        self.template_modes.pop();
        self.reset_insertion_mode();
    }

    fn in_template(&mut self, tok: Token) {
        match &tok {
            Token::Characters(_) | Token::Comment(_) | Token::Doctype { .. } => self.in_body(tok),
            Token::StartTag { name, .. } if matches!(name.as_str(),
                "base" | "basefont" | "bgsound" | "link" | "meta" | "noframes" | "script" | "style" | "title") => self.in_head(tok),
            Token::StartTag { name, .. } => {
                let next = match name.as_str() {
                    "caption" | "colgroup" | "tbody" | "tfoot" | "thead" => Mode::InTable,
                    "col" => Mode::InColumnGroup,
                    "tr" => Mode::InTableBody,
                    "td" | "th" => Mode::InRow,
                    _ => Mode::InBody,
                };
                *self.template_modes.last_mut().expect("template mode") = next;
                self.mode = next;
                self.process_token(tok);
            },
            Token::EndTag { .. } => {},
            Token::Eof => {},
            Token::ProcessingInstruction { .. } => unreachable!("handled by dispatcher"),
        }
    }

    fn in_head(&mut self, tok: Token) {
        match tok {
            Token::Characters(data) => {
                let first_non_whitespace = data
                    .char_indices()
                    .find_map(|(index, c)| (!is_ws_char(c)).then_some(index));
                if let Some(index) = first_non_whitespace {
                    if index > 0 {
                        self.insert_text(&data[..index]);
                    }
                    self.leave_head();
                    self.after_head(Token::Characters(data[index..].to_owned()));
                } else {
                    self.insert_text(&data);
                }
            },
            Token::Comment(data) => self.insert_comment_here(&data),
            Token::Doctype { .. } => {},
            Token::StartTag { ref name, attrs, self_closing } => {
                match name.as_str() {
                    "base" | "basefont" | "bgsound" | "link" | "meta" => {
                        self.insert_void(name, attrs);
                    },
                    "title" | "noframes" | "style" | "script" | "noscript" => {
                        self.start_generic_text_element(name, attrs);
                    },
                    "template" => self.start_template(attrs),
                    "head" => {},
                    _ => {
                        self.leave_head();
                        self.after_head(Token::StartTag {
                            name: name.clone(),
                            attrs,
                            self_closing,
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
                "template" => self.close_template(),
                _ => {},
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
            Token::StartTag { ref name, attrs, self_closing } => {
                match name.as_str() {
                    "body" => {
                        self.insert_and_push(name, attrs);
                        self.frameset_ok = false;
                        self.mode = Mode::InBody;
                    },
                    "frameset" => {
                        self.insert_and_push("frameset", attrs);
                        self.mode = Mode::InFrameset;
                    },
                    "base" | "basefont" | "bgsound" | "link" | "meta"
                    | "noframes" | "script" | "style" | "template" | "title" => {
                        if let Some(head) = self.head {
                            self.open.push(head);
                            self.in_head(Token::StartTag {
                                name: name.clone(),
                                attrs,
                                self_closing,
                            });
                            if let Some(position) =
                                self.open.iter().position(|&node| node == head)
                            {
                                self.open.remove(position);
                            }
                        }
                    },
                    _ => {
                        self.implied_body();
                        self.in_body(Token::StartTag {
                            name: name.clone(),
                            attrs,
                            self_closing,
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
            Token::ProcessingInstruction { .. } => unreachable!("handled by dispatcher"),
        }
    }

    /// "In body" ignores a character token that is U+0000 NULL outright (§13.2.6.4.7):
    /// no reconstruction, no `frameset_ok` effect, nothing inserted for it. The
    /// tokenizer emits NUL literally in the data state (only RCDATA/RAWTEXT/script
    /// data/plaintext replace it with U+FFFD there), so this mode must drop any
    /// embedded NUL from a batched character run before deciding what to do with
    /// the rest of it.
    fn in_body_characters(&mut self, data: &str) {
        if data.is_empty() {
            return;
        }
        let owned = data.contains('\0').then(|| data.chars().filter(|&c| c != '\0').collect::<String>());
        let text = owned.as_deref().unwrap_or(data);
        if text.is_empty() {
            return;
        }
        self.reconstruct();
        if text.chars().any(|c| !is_ws_char(c)) {
            self.frameset_ok = false;
        }
        self.insert_text(text);
    }

    fn in_body_start_tag(&mut self, name: &str, attrs: Vec<(String, String)>) {
        match name {
            // §13.2.6.4.7 "in body", start tag `html`: if a `template` is on
            // the stack of open elements, ignore the token outright instead
            // of merging attributes onto the stack's bottom element -- a
            // stray `<html b=c>` inside template content was leaking `b=c`
            // onto the real `<html>` element (html5lib template.dat cases
            // 65-67: `<html a=b><template><div><html b=c>...`).
            "html" => {
                if !self.stack_has("template") {
                    if let Some(&root) = self.open.first() {
                        self.merge_attrs(root, attrs);
                    }
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
            // §13.2.6.4.7 "in body", start tag `body`: ignore the token
            // entirely (including leaving `frameset_ok` untouched) unless a
            // `body` element is actually in scope -- a bare stack search
            // found a real `<body>` *below* an intervening `<template>` and
            // merged the stray tag's attributes onto it regardless (html5lib
            // template.dat case 64: `<body a=b><template>...<body c=d>...`
            // leaked `c=d` onto the outer body, since `template` terminates
            // scope per §13.2.4.3 and should have hidden it).
            "body" => {
                if self.has_in_scope("body") {
                    if let Some(&b) = self
                        .open
                        .iter()
                        .rev()
                        .find(|&&i| self.dom.name(i) == Some("body"))
                    {
                        self.merge_attrs(b, attrs);
                    }
                    self.frameset_ok = false;
                }
            },
            "frameset" => {
                if self.open.len() < 2
                    || self.dom.name(self.open[1]) != Some("body")
                    || !self.frameset_ok
                {
                    return;
                }
                self.dom.detach(self.open[1]);
                self.open.truncate(1);
                self.insert_and_push("frameset", attrs);
                self.mode = Mode::InFrameset;
            },
            "address" | "article" | "aside" | "blockquote" | "center"
            | "details" | "dialog" | "dir" | "div" | "dl" | "fieldset"
            | "figcaption" | "figure" | "footer" | "header" | "hgroup"
            | "main" | "menu" | "nav" | "ol" | "p" | "search" | "section" | "summary"
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
                self.close_p_element();
                self.insert_and_push(name, attrs);
                self.frameset_ok = false;
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
                if let Some(old_pos) = self.fmt_pos_last_named("a") {
                    let old_anchor = self.fmt_idx_at(old_pos);
                    self.adoption_agency("a");
                    if let Some(position) = self.fmt_pos_by_idx(old_anchor) {
                        self.fmt.remove(position);
                    }
                    if let Some(position) = self
                        .open
                        .iter()
                        .position(|&node| node == old_anchor)
                    {
                        self.open.remove(position);
                    }
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
                if self.document_mode != doctype::DocumentMode::Quirks { self.close_p_element(); }
                self.insert_and_push("table", attrs);
                self.frameset_ok = false;
                self.mode = Mode::InTable;
            },
            "area" | "br" | "embed" | "img" | "keygen" | "wbr" => {
                self.reconstruct();
                self.insert_void(name, attrs);
                self.frameset_ok = false;
            },
            "input" => {
                if self.has_in_scope("select") {
                    self.pop_until("select");
                    self.reset_insertion_mode();
                }
                let hidden = attrs.iter().any(|(attr, value)| {
                    attr == "type" && value.eq_ignore_ascii_case("hidden")
                });
                self.reconstruct();
                self.insert_void("input", attrs);
                if !hidden {
                    self.frameset_ok = false;
                }
            },
            "hr" => {
                self.close_p_element();
                if self.has_in_scope("select") {
                    self.generate_implied_end_tags(None);
                }
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
                self.frameset_ok = false;
                self.orig_mode = Mode::InBody;
                self.mode = Mode::Text;
            },
            "xmp" => {
                self.close_p_element();
                self.insert_and_push("xmp", attrs);
                self.skip_next_lf = true;
                self.frameset_ok = false;
                self.orig_mode = Mode::InBody;
                self.mode = Mode::Text;
            },
            "iframe" => {
                self.frameset_ok = false;
                self.start_generic_text_element("iframe", attrs);
            },
            "noembed" => self.start_generic_text_element("noembed", attrs),
            "noscript" => self.start_generic_text_element("noscript", attrs),
            "select" => {
                if self.has_in_scope("select") {
                    self.pop_until("select");
                    self.reset_insertion_mode();
                } else {
                    self.reconstruct();
                    self.insert_and_push("select", attrs);
                    self.frameset_ok = false;
                }
            },
            "option" => {
                if self.has_in_scope("select") {
                    self.generate_implied_end_tags(Some("optgroup"));
                } else if self
                    .current()
                    .is_some_and(|cur| self.dom.name(cur) == Some("option"))
                {
                    self.open.pop();
                }
                self.reconstruct();
                self.insert_and_push("option", attrs);
            },
            "optgroup" => {
                if self.has_in_scope("select") {
                    self.generate_implied_end_tags(None);
                } else if self
                    .current()
                    .is_some_and(|cur| self.dom.name(cur) == Some("option"))
                {
                    self.open.pop();
                }
                self.reconstruct();
                self.insert_and_push("optgroup", attrs);
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
                self.reconstruct();
                let namespace = if name == "svg" {
                    Namespace::Svg
                } else {
                    Namespace::MathMl
                };
                self.insert_foreign_element(name, attrs, namespace);
            },
            "caption" | "col" | "colgroup" | "frame" | "head" | "tbody"
            | "td" | "tfoot" | "th" | "thead" | "tr" => {},
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
            "address" | "article" | "aside" | "blockquote" | "button"
            | "center" | "details" | "dialog" | "dir" | "div" | "dl"
            | "fieldset" | "figcaption" | "figure" | "footer" | "header"
            | "hgroup" | "listing" | "main" | "menu" | "nav" | "ol"
            | "pre" | "search" | "section" | "summary" | "ul" => {
                if self.has_in_scope(name) {
                    self.generate_implied_end_tags(None);
                    self.pop_until(name);
                }
            },
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                let has_heading = ["h1", "h2", "h3", "h4", "h5", "h6"]
                    .iter()
                    .any(|heading| self.has_in_scope(heading));
                if has_heading {
                    self.generate_implied_end_tags(None);
                    self.pop_until_any(&["h1", "h2", "h3", "h4", "h5", "h6"]);
                }
            },
            "applet" | "marquee" | "object" => {
                if self.has_in_scope(name) {
                    self.generate_implied_end_tags(None);
                    self.pop_until(name);
                    self.clear_fmt_to_marker();
                }
            },
            "li" => {
                if self.has_in_list_item_scope("li") {
                    self.generate_implied_end_tags(Some("li"));
                    self.pop_until("li");
                }
            },
            "dd" | "dt" | "optgroup" | "option" | "rb" | "rp" | "rt" => {
                if name == "option" {
                    if self
                        .current()
                        .is_some_and(|cur| self.dom.name(cur) == Some("option"))
                    {
                        self.open.pop();
                    }
                } else if name == "optgroup" {
                    if self.current().is_some_and(|cur| {
                        self.dom.name(cur) == Some("option")
                            && self.open.len() >= 2
                            && self.dom.name(self.open[self.open.len() - 2])
                                == Some("optgroup")
                    }) {
                        self.open.pop();
                    }
                    if self
                        .current()
                        .is_some_and(|cur| self.dom.name(cur) == Some("optgroup"))
                    {
                        self.open.pop();
                    }
                } else if self.stack_has(name) {
                    self.generate_implied_end_tags(Some(name));
                    self.pop_until(name);
                }
            },
            "select" => {
                if self.has_in_scope("select") {
                    self.pop_until("select");
                    self.reset_insertion_mode();
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
                    let node = self.current().unwrap();
                    self.open.pop();
                    self.mode = self.orig_mode;
                    // HTML-31: only a full-document parse (never a fragment
                    // parse -- scripts never execute there, per spec), never
                    // while any number of `<template>` contents are
                    // currently open (a script parsed into template content
                    // is never "inserted into a document" the way the spec's
                    // script-execution algorithm requires -- it sits inert
                    // until the template is later cloned/used, same as
                    // real browsers; `template_modes` is non-empty exactly
                    // while building into template content, including
                    // nested templates), and only for a `<script>` closing
                    // tag in the HTML or SVG namespace (both have real
                    // script elements that execute per spec; MathML does
                    // not).
                    if !self.fragment
                        && self.template_modes.is_empty()
                        && name == "script"
                        && matches!(self.dom.namespace(node), Some(Namespace::Html) | Some(Namespace::Svg))
                    {
                        let attrs = self.dom.attrs_of(node).map(<[_]>::to_vec).unwrap_or_default();
                        match classify_script(&attrs) {
                            Some(ScriptMode::Blocking) => self.pending_script = Some(node),
                            Some(mode) => self.script_events.push((node, mode)),
                            None => {},
                        }
                    }
                }
            },
            Token::Eof => {
                self.open.pop();
                self.mode = self.orig_mode;
                self.process_token(Token::Eof);
            },
            _ => {},
        }
    }

    fn in_frameset(&mut self, tok: Token) {
        match tok {
            Token::Characters(data) => {
                let whitespace: String = data.chars().filter(|&c| is_ws_char(c)).collect();
                self.insert_text(&whitespace)
            },
            Token::Comment(data) => self.insert_comment_here(&data),
            Token::Doctype { .. } => {},
            Token::StartTag { name, attrs, self_closing: _ } => {
                match name.as_str() {
                    "html" => self.in_body_start_tag("html", attrs),
                    "frameset" => {
                        self.insert_and_push("frameset", attrs);
                    },
                    "frame" => {
                        self.insert_void("frame", attrs);
                    },
                    "noframes" => self.in_head(Token::StartTag {
                        name,
                        attrs,
                        self_closing: false,
                    }),
                    _ => {},
                }
            },
            Token::EndTag { name } if name == "frameset" => {
                if self.current().is_some_and(|node| {
                    self.dom.namespace(node) == Some(Namespace::Html)
                        && self.dom.name(node) == Some("frameset")
                }) {
                    self.open.pop();
                    if self
                        .current()
                        .is_some_and(|node| self.dom.name(node) == Some("html"))
                    {
                        self.mode = Mode::AfterFrameset;
                    }
                }
            },
            Token::Eof => {},
            _ => {},
        }
    }

    fn after_frameset(&mut self, tok: Token) {
        match tok {
            Token::Characters(data) => {
                let whitespace: String = data.chars().filter(|&c| is_ws_char(c)).collect();
                self.insert_text(&whitespace)
            },
            Token::Comment(data) => self.insert_comment_here(&data),
            Token::Doctype { .. } => {},
            Token::StartTag { name, attrs, self_closing: _ }
                if name == "html" =>
            {
                self.in_body_start_tag("html", attrs)
            },
            Token::StartTag { name, attrs, self_closing: _ }
                if name == "noframes" =>
            {
                self.in_head(Token::StartTag {
                    name,
                    attrs,
                    self_closing: false,
                })
            },
            Token::EndTag { name } if name == "html" => {
                self.mode = Mode::AfterAfterFrameset;
            },
            Token::Eof => {},
            _ => {},
        }
    }

    fn after_after_frameset(&mut self, tok: Token) {
        match tok {
            Token::Characters(data) if data.chars().all(is_ws_char) => {
                self.in_body(Token::Characters(data));
            },
            Token::Comment(data) => self.comment_into(NodeIdx(0), &data),
            Token::Doctype { .. } => {},
            Token::StartTag { name, attrs, self_closing: _ }
                if name == "html" =>
            {
                self.in_body_start_tag("html", attrs)
            },
            Token::StartTag { name, attrs, self_closing: _ }
                if name == "noframes" =>
            {
                self.in_head(Token::StartTag {
                    name,
                    attrs,
                    self_closing: false,
                })
            },
            Token::Eof => {},
            _ => {},
        }
    }

    fn after_body(&mut self, tok: Token) {
        match tok {
            Token::Characters(data) if data.chars().all(is_ws_char) => {
                self.in_body(Token::Characters(data));
            },
            Token::Comment(data) => {
                if let Some(&html) = self.open.first() {
                    self.comment_into(html, &data);
                }
            },
            Token::Doctype { .. } | Token::Eof => {},
            Token::StartTag { name, attrs, self_closing } if name == "html" => {
                self.in_body(Token::StartTag { name, attrs, self_closing });
            },
            Token::EndTag { ref name } if name == "html" => {
                self.mode = Mode::AfterAfterBody;
            },
            other => {
                self.mode = Mode::InBody;
                self.in_body(other);
            },
        }
    }

    fn after_after_body(&mut self, tok: Token) {
        match tok {
            Token::Characters(data) if data.chars().all(is_ws_char) => {
                self.in_body(Token::Characters(data));
            },
            Token::Comment(data) => self.comment_into(NodeIdx(0), &data),
            Token::Doctype { .. } | Token::Eof => {},
            Token::StartTag { name, attrs, self_closing } if name == "html" => {
                self.in_body(Token::StartTag { name, attrs, self_closing });
            },
            other => {
                self.mode = Mode::InBody;
                self.in_body(other);
            },
        }
    }
}

/// Convenience: tokenize `src` and run tree construction to completion.
/// `pub(crate)`: also used by `resumable::ResumableParser`'s own token-at-a-
/// time driving loop.
pub(crate) fn feed_with_tokenizer_feedback(
    builder: &mut TreeBuilder,
    tokenizer: &mut Tokenizer,
    tok: Token,
) -> bool {
    let eof = tok == Token::Eof;
    let start_name = match &tok {
        Token::StartTag { name, .. } => Some(name.clone()),
        _ => None,
    };
    builder.feed(tok);
    tokenizer.set_allow_cdata(
        builder.adjusted_current_namespace().is_some_and(|namespace| namespace != Namespace::Html)
    );
    if let Some(name) = start_name {
        let inserted_html_text_element = (builder.mode == Mode::Text || name == "plaintext")
            && builder.current().is_some_and(|current| {
                builder.dom.namespace(current) == Some(Namespace::Html)
                    && builder.dom.name(current) == Some(name.as_str())
            });
        if inserted_html_text_element {
            tokenizer.enter_text_element(&name);
        }
    }
    eof
}

/// Incremental HTML parser for already-decoded text chunks.
///
/// Chunks may end anywhere between Unicode scalar values. Tokenizer input
/// already consumed is released after each call; an unfinished tag, comment,
/// doctype, or character reference can remain buffered until it is resolved.
pub struct IncrementalParser {
    builder: TreeBuilder,
    tokenizer: Tokenizer,
    pending_cr: bool,
}

impl Default for IncrementalParser {
    fn default() -> Self { Self::new() }
}

impl IncrementalParser {
    pub fn new() -> Self {
        Self {
            builder: TreeBuilder::new(),
            tokenizer: Tokenizer::streaming(),
            pending_cr: false,
        }
    }

    /// Starts a streamed document parse with a synchronous custom-element host.
    /// Fragment parsing remains inert and uses a separate entry point.
    pub fn with_custom_element_host(dom: Dom, host: Box<dyn ParserCustomElementHost>) -> Self {
        Self {
            builder: TreeBuilder::with_custom_element_host(dom, host),
            tokenizer: Tokenizer::streaming(),
            pending_cr: false,
        }
    }

    /// Feeds one decoded chunk without treating its end as document EOF.
    pub fn push_chunk(&mut self, chunk: &str) {
        let mut normalized = Vec::with_capacity(chunk.chars().count() + usize::from(self.pending_cr));
        for c in chunk.chars() {
            if self.pending_cr {
                normalized.push('\n');
                self.pending_cr = false;
                if c == '\n' { continue; }
            }
            if c == '\r' { self.pending_cr = true; }
            else { normalized.push(c); }
        }
        self.tokenizer.push_chars(normalized);
        self.drive();
    }

    /// Number of scalar values retained as tokenizer input. This excludes
    /// the DOM and the current token's decoded fields.
    pub fn buffered_input_scalars(&self) -> usize {
        self.tokenizer.buffered_input_scalars()
    }

    /// Signals the one final EOF and returns the completed DOM.
    pub fn finish(mut self) -> Dom {
        if self.pending_cr { self.tokenizer.push_chars(['\n']); }
        self.tokenizer.finish_input();
        self.drive();
        self.builder.into_dom()
    }

    fn drive(&mut self) {
        while let Some(tok) = self.tokenizer.next_available() {
            if feed_with_tokenizer_feedback(&mut self.builder, &mut self.tokenizer, tok) {
                break;
            }
        }
        self.tokenizer.compact_input();
    }
}

/// Convenience: tokenize `src` and run tree construction to completion.
pub fn build(src: &str) -> Dom {
    let mut builder = TreeBuilder::new();
    let mut tokenizer = Tokenizer::new(src.chars().collect());
    loop {
        let tok = tokenizer.next_token();
        let eof = feed_with_tokenizer_feedback(&mut builder, &mut tokenizer, tok);
        if eof {
            break;
        }
    }
    builder.into_dom()
}

/// Parses with definitions already installed on `dom`. The host receives each
/// matching HTML element synchronously during tree construction. Reactions
/// remain queued in the returned DOM for the embedding's parser-task boundary.
pub fn build_with_custom_element_host(
    src: &str,
    dom: Dom,
    host: Box<dyn ParserCustomElementHost>,
) -> Dom {
    let mut builder = TreeBuilder::with_custom_element_host(dom, host);
    let mut tokenizer = Tokenizer::new(src.chars().collect());
    loop {
        let tok = tokenizer.next_token();
        if feed_with_tokenizer_feedback(&mut builder, &mut tokenizer, tok) {
            break;
        }
    }
    builder.into_dom()
}

/// The fragment context is the last node used by the reset insertion mode
/// algorithm, even though it is not on the synthetic open-element stack.
fn fragment_insertion_mode(context: &str) -> Mode {
    match context {
        "html" => Mode::BeforeHead,
        "frameset" => Mode::InFrameset,
        "select" => Mode::InBody,
        "template" => Mode::InTemplate,
        "table" => Mode::InTable,
        "tbody" | "thead" | "tfoot" => Mode::InTableBody,
        "tr" => Mode::InRow,
        "td" | "th" => Mode::InCell,
        "caption" => Mode::InCaption,
        "colgroup" => Mode::InColumnGroup,
        _ => Mode::InBody,
    }
}

/// Fragment tree construction with only the synthetic `html` root on the open
/// stack. The context element sets the initial mode but is not a tree child.
/// `namespace` is the *context element's own* namespace (HTML fragment
/// contexts pass `Namespace::Html`; a foreign context, e.g. `Element.
/// innerHTML` on a MathML/SVG element, passes that element's own namespace)
/// -- HTML-40 (2): needed for `adjusted_current_namespace`'s spec-mandated
/// fragment-case special-casing (`cdata-in-integration-point-fragment.html`),
/// since the synthetic root pushed below is always HTML regardless.
///
/// `allow_declarative_shadow_roots` gates `<template shadowrootmode>`
/// handling exactly as it does for a full document parse (`start_template`,
/// unchanged): a template whose *host* -- the real, already-open ancestor
/// element it would otherwise be inserted under, e.g. a `<div>` earlier in
/// this same fragment's own markup -- is on the open-elements stack gets its
/// content redirected straight into a newly attached shadow root instead of
/// becoming an ordinary child. `setHTMLUnsafe`/`Document.parseHTMLUnsafe`
/// are the spec's two fragment-parsing entry points that turn this on;
/// ordinary `innerHTML`/`insertAdjacentHTML` must keep it off, which is why
/// this is a parameter here rather than always-on. The synthetic `html` root
/// itself is never a valid shadow host (`Dom::attach_shadow` doesn't allow
/// the name "html"), so a *top-level* `<template shadowrootmode>` -- with no
/// real element ancestor in the fragment at all, as `Element.setHTMLUnsafe`'s
/// own most common usage looks like -- still comes back as an ordinary
/// leftover template rather than attaching to the caller's real host
/// element; that caller-side host-identity gap is intentionally left to the
/// binding layer, which has an actual host `NodeIdx` available to attach to
/// (see `parse_fragment_with_shadow_roots`'s doc comment in `lib.rs`).
pub(crate) fn build_fragment(
    src: &str,
    context: &str,
    namespace: Namespace,
    allow_declarative_shadow_roots: bool,
) -> Dom {
    let mut builder = TreeBuilder::new();
    builder.allow_declarative_shadow_roots = allow_declarative_shadow_roots;
    builder.fragment = true;
    builder.fragment_context = Some(context.to_owned());
    builder.fragment_context_namespace = namespace;
    builder.insert_and_push("html", Vec::new());
    builder.mode = fragment_insertion_mode(context);
    if context == "template" { builder.template_modes.push(Mode::InTemplate); }
    let mut tokenizer = Tokenizer::new(src.chars().collect());
    // These are HTML-only RAWTEXT/RCDATA-ish tag names; a foreign context
    // (e.g. SVG's own `<title>`, which is an ordinary child-bearing element,
    // not RAWTEXT) must not enter the tokenizer's text-context state just
    // because its local name happens to collide with one of these.
    if namespace == Namespace::Html
        && matches!(context, "textarea" | "title" | "style" | "xmp" | "iframe" | "noembed" | "noframes" | "script" | "noscript" | "plaintext")
    {
        tokenizer.enter_fragment_text_context(context);
    }
    loop {
        let tok = tokenizer.next_token();
        let eof = feed_with_tokenizer_feedback(&mut builder, &mut tokenizer, tok);
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
            NodeKind::DocumentFragment => {
                out.push_str("#document-fragment[");
                for child in dom.children(idx) { shape_node(dom, child, out); }
                out.push(']');
            },
            NodeKind::DocumentType { name, .. } => {
                out.push_str("<!DOCTYPE "); out.push_str(name); out.push('>');
            },
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
            NodeKind::CDATASection { data } => {
                out.push_str("<![CDATA[");
                out.push_str(data);
                out.push_str("]]>");
            },
            NodeKind::Attr { .. } => {},
            NodeKind::Comment { data } => {
                out.push_str("<!");
                out.push_str(data);
                out.push('>');
            },
            NodeKind::ProcessingInstruction { target, data } => {
                out.push_str("<?");
                out.push_str(target);
                out.push(' ');
                out.push_str(data);
                out.push_str("?>");
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
            "(doc <!DOCTYPE html>html[head[title[\"t\"]]body[p[\"x\"]]])"
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
    fn adoption_agency_repeats_across_nested_blocks() {
        assert_eq!(
            build_shape("<a>1<div>2<div>3</a>4</div>5</div>"),
            "(doc html[head[]body[a[\"1\"]div[a[\"2\"]div[a[\"3\"]\"4\"]\"5\"]]])"
        );
    }

    #[test]
    fn adoption_agency_uses_adjusted_table_insertion_location() {
        assert_eq!(
            build_shape("<table><a>1<p>2</a>3</p>"),
            "(doc html[head[]body[a[\"1\"]p[a[\"2\"]\"3\"]table[]]])"
        );
    }

    #[test]
    fn formatting_reconstructs_after_a_table_cell() {
        assert_eq!(
            build_shape("<table><a>1<td>2</td>3</table>"),
            "(doc html[head[]body[a[\"1\"]a[\"3\"]table[tbody[tr[td[\"2\"]]]]]])"
        );
    }

    #[test]
    fn explicit_cell_close_pops_non_implied_descendants() {
        assert_eq!(
            build_shape("<table><tr><td><div>x</td>z</tr></table>"),
            "(doc html[head[]body[\"z\"table[tbody[tr[td[div[\"x\"]]]]]]])"
        );
    }

    #[test]
    fn self_closing_foreign_root_does_not_capture_following_html() {
        let dom = build("<svg/><circle>");
        let html = dom.get(NodeIdx(0)).first_child.expect("html");
        let body = dom.get(html).last_child.expect("body");
        let children: Vec<NodeIdx> = dom.children(body).collect();
        assert_eq!(children.len(), 2);
        assert_eq!(dom.namespace(children[0]), Some(Namespace::Svg));
        assert_eq!(dom.name(children[0]), Some("svg"));
        assert_eq!(dom.namespace(children[1]), Some(Namespace::Html));
        assert_eq!(dom.name(children[1]), Some("circle"));
    }

    #[test]
    fn table_fallback_preserves_foreign_root_self_closing() {
        let dom = build("<table><svg/><circle>");
        let html = dom.get(NodeIdx(0)).first_child.expect("html");
        let body = dom.get(html).last_child.expect("body");
        let children: Vec<NodeIdx> = dom.children(body).collect();
        assert_eq!(children.len(), 3);
        assert_eq!(dom.namespace(children[0]), Some(Namespace::Svg));
        assert_eq!(dom.name(children[0]), Some("svg"));
        assert_eq!(dom.namespace(children[1]), Some(Namespace::Html));
        assert_eq!(dom.name(children[1]), Some("circle"));
        assert_eq!(dom.name(children[2]), Some("table"));
    }

    #[test]
    fn foreign_integration_points_are_special_scope_boundaries() {
        let mut builder = TreeBuilder::new();
        let anchor = builder.dom.alloc_unattached(NodeKind::Element {
            name: "a".into(),
            attrs: Vec::new(),
            namespace: Namespace::Html,
        });
        let desc = builder.dom.alloc_unattached(NodeKind::Element {
            name: "desc".into(),
            attrs: Vec::new(),
            namespace: Namespace::Svg,
        });
        let mtext = builder.dom.alloc_unattached(NodeKind::Element {
            name: "mtext".into(),
            attrs: Vec::new(),
            namespace: Namespace::MathMl,
        });
        assert!(builder.is_special(desc));
        assert!(builder.is_special(mtext));
        builder.open = vec![anchor, desc];
        assert!(!builder.has_in_scope("a"));
    }

    #[test]
    fn select_options_and_groups_close_predecessors() {
        assert_eq!(
            build_shape("<select><optgroup><option>1<option>2<optgroup><option>3</select>"),
            "(doc html[head[]body[select[optgroup[option[\"1\"]option[\"2\"]]optgroup[option[\"3\"]]]]])"
        );
    }

    #[test]
    fn nested_select_uses_current_in_body_recovery() {
        assert_eq!(
            build_shape("<select><b><option><select><option></b></select>X"),
            "(doc html[head[]body[select[b[option[]]]b[option[]]\"X\"]])"
        );
    }

    #[test]
    fn select_scope_adjusts_hr_and_input() {
        assert_eq!(
            build_shape("<select><option>x<hr>y</select>"),
            "(doc html[head[]body[select[option[\"x\"]hr[]\"y\"]]])"
        );
        assert_eq!(
            build_shape("<select><option>x<input>y"),
            "(doc html[head[]body[select[option[\"x\"]]input[]\"y\"]])"
        );
    }

    #[test]
    fn svg_desc_is_an_html_integration_point() {
        let dom = build("<table><tr><td><svg><desc><td>");
        assert_eq!(
            dom.dump(NodeIdx(0)),
            "#document\n  <html>\n    <head>\n    <body>\n      <table>\n        <tbody>\n          <tr>\n            <td>\n              <svg svg>\n                <svg desc>\n            <td>\n"
        );
    }

    #[test]
    fn svg_adjusts_foreign_object_and_reenters_html() {
        let dom = build("<div><table><svg><foreignObject><select><table><s>");
        assert_eq!(
            dom.dump(NodeIdx(0)),
            "#document\n  <html>\n    <head>\n    <body>\n      <div>\n        <svg svg>\n          <svg foreignObject>\n            <select>\n        <table>\n        <s>\n        <table>\n"
        );
    }

    #[test]
    fn svg_children_stay_in_svg_namespace() {
        let dom = build("<a><svg><tr><input></a>");
        assert_eq!(
            dom.dump(NodeIdx(0)),
            "#document\n  <html>\n    <head>\n    <body>\n      <a>\n        <svg svg>\n          <svg tr>\n            <svg input>\n"
        );
    }

    #[test]
    fn unmatched_p_and_br_end_tags_leave_foreign_content_before_html_recovery() {
        for (foreign, ending, html_child) in [
            ("svg", "p", "p"), ("svg", "br", "br"),
            ("math", "p", "p"), ("math", "br", "br"),
        ] {
            let dom = build(&format!("<{foreign}></{ending}><foo>"));
            assert_eq!(dom.dump(NodeIdx(0)), format!(
                "#document\n  <html>\n    <head>\n    <body>\n      <{foreign} {foreign}>\n      <{html_child}>\n      <foo>\n"
            ), "{foreign} / {ending}");
        }
    }

    #[test]
    fn other_unmatched_end_tags_keep_the_foreign_stack_open() {
        assert_eq!(build("<math><annotation-xml></svg>x").dump(NodeIdx(0)),
            "#document\n  <html>\n    <head>\n    <body>\n      <math math>\n        <math annotation-xml>\n          \"x\"\n");
        assert_eq!(build("<svg><tfoot></mi><td>").dump(NodeIdx(0)),
            "#document\n  <html>\n    <head>\n    <body>\n      <svg svg>\n        <svg tfoot>\n          <svg td>\n");
    }

    #[test]
    fn foreign_attributes_receive_canonical_local_names() {
        let dom = build("<svg viewbox='' preserveaspectratio=''><g xlink:href='x'></g></svg><math><mn definitionurl='u'>");
        let dump = dom.dump(NodeIdx(0));
        assert!(dump.contains("<svg svg viewBox=\"\" preserveAspectRatio=\"\">"));
        assert!(dump.contains("<svg g xlink href=\"x\">"));
        assert!(dump.contains("<math mn definitionURL=\"u\">"));
        let svg = dom.children(NodeIdx(0)).flat_map(|n| dom.children(n))
            .flat_map(|n| dom.children(n)).find(|n| dom.name(*n) == Some("svg")).unwrap();
        let g = dom.children(svg).next().unwrap();
        let href = dom.attribute_ns(g, Some("http://www.w3.org/1999/xlink"), "href").unwrap();
        assert_eq!((href.prefix, href.local_name, href.qualified_name(), href.value),
            (Some("xlink"), "href", "xlink:href".into(), "x"));
    }

    fn find_element(dom: &Dom, root: NodeIdx, namespace: Namespace, name: &str) -> Option<NodeIdx> {
        if dom.namespace(root) == Some(namespace) && dom.name(root) == Some(name) {
            return Some(root);
        }
        dom.children(root).find_map(|child| find_element(dom, child, namespace, name))
    }

    #[test]
    fn foreign_attribute_table_has_expanded_identity_in_svg_and_mathml() {
        const XLINK: &str = "http://www.w3.org/1999/xlink";
        const XML: &str = "http://www.w3.org/XML/1998/namespace";
        const XMLNS: &str = "http://www.w3.org/2000/xmlns/";
        let table = [
            ("xlink:actuate", XLINK, Some("xlink"), "actuate"),
            ("xlink:arcrole", XLINK, Some("xlink"), "arcrole"),
            ("xlink:href", XLINK, Some("xlink"), "href"),
            ("xlink:role", XLINK, Some("xlink"), "role"),
            ("xlink:show", XLINK, Some("xlink"), "show"),
            ("xlink:title", XLINK, Some("xlink"), "title"),
            ("xlink:type", XLINK, Some("xlink"), "type"),
            ("xml:lang", XML, Some("xml"), "lang"),
            ("xml:space", XML, Some("xml"), "space"),
            ("xmlns", XMLNS, None, "xmlns"),
            ("xmlns:xlink", XMLNS, Some("xmlns"), "xlink"),
        ];
        let attrs = table.iter().enumerate().map(|(i, (name, _, _, _))| {
            format!(" {name}='{i}'")
        }).collect::<String>();
        let dom = build(&format!("<svg{attrs} viewbox='0' data:x='s'></svg><math{attrs} definitionurl='u' odd:piece='m'></math>"));
        let svg = find_element(&dom, NodeIdx(0), Namespace::Svg, "svg").unwrap();
        let math = find_element(&dom, NodeIdx(0), Namespace::MathMl, "math").unwrap();
        for node in [svg, math] {
            for (i, (qualified, uri, prefix, local)) in table.iter().enumerate() {
                let attr = dom.attribute_ns(node, Some(uri), local).unwrap();
                assert_eq!(attr.namespace_uri, Some(*uri), "{qualified}");
                assert_eq!(attr.prefix, *prefix, "{qualified}");
                assert_eq!(attr.local_name, *local, "{qualified}");
                assert_eq!(attr.qualified_name(), *qualified, "{qualified}");
                assert_eq!(attr.value, i.to_string(), "{qualified}");
                assert_eq!(dom.attribute_ns(node, None, qualified), None, "{qualified}");
            }
        }
        let view_box = dom.attribute_ns(svg, None, "viewBox").unwrap();
        assert_eq!((view_box.prefix, view_box.local_name, view_box.value), (None, "viewBox", "0"));
        let definition_url = dom.attribute_ns(math, None, "definitionURL").unwrap();
        assert_eq!((definition_url.prefix, definition_url.local_name, definition_url.value), (None, "definitionURL", "u"));
        for (node, name, value) in [(svg, "data:x", "s"), (math, "odd:piece", "m")] {
            let attr = dom.attribute_ns(node, None, name).unwrap();
            assert_eq!((attr.namespace_uri, attr.prefix, attr.local_name, attr.qualified_name(), attr.value),
                (None, None, name, name.into(), value));
        }
    }

    #[test]
    fn nested_foreign_elements_adjust_attributes_but_html_children_do_not() {
        const XLINK: &str = "http://www.w3.org/1999/xlink";
        const XML: &str = "http://www.w3.org/XML/1998/namespace";
        const XMLNS: &str = "http://www.w3.org/2000/xmlns/";
        let dom = build("<svg><g xlink:href='g'><foreignObject><div xlink:href='h' xml:lang='fr'></div></foreignObject></g></svg><math><mn xml:space='preserve' xmlns='urn:m' strange:token='z'></mn></math>");
        let g = find_element(&dom, NodeIdx(0), Namespace::Svg, "g").unwrap();
        let div = find_element(&dom, NodeIdx(0), Namespace::Html, "div").unwrap();
        let mn = find_element(&dom, NodeIdx(0), Namespace::MathMl, "mn").unwrap();
        assert_eq!(dom.attribute_ns(g, Some(XLINK), "href").unwrap().qualified_name(), "xlink:href");
        assert_eq!(dom.attribute_ns(mn, Some(XML), "space").unwrap().value, "preserve");
        assert_eq!(dom.attribute_ns(mn, Some(XMLNS), "xmlns").unwrap().prefix, None);
        assert_eq!(dom.attribute_ns(mn, None, "strange:token").unwrap().local_name, "strange:token");
        assert_eq!(dom.attribute_ns(div, Some(XLINK), "href"), None);
        assert_eq!(dom.attribute_ns(div, Some(XML), "lang"), None);
        assert_eq!(dom.attribute_ns(div, None, "xlink:href").unwrap().local_name, "xlink:href");
        assert_eq!(dom.attribute_ns(div, None, "xml:lang").unwrap().prefix, None);
    }

    #[test]
    fn stray_mismatched_end_tag_ignored() {
        assert_eq!(
            build_shape("<div>x</div></div>y"),
            "(doc html[head[]body[div[\"x\"]\"y\"]])"
        );
    }

    #[test]
    fn ordinary_end_tag_does_not_cross_special_element() {
        assert_eq!(
            build_shape("<!DOCTYPE html><span><button>foo</span>bar"),
            "(doc <!DOCTYPE html>html[head[]body[span[button[\"foobar\"]]]])"
        );
        assert_eq!(
            build_shape("<b>A<cite>B<div>C</cite>D"),
            "(doc html[head[]body[b[\"A\"cite[\"B\"div[\"CD\"]]]]])"
        );
    }

    #[test]
    fn adoption_preserves_furthest_block_child_order() {
        assert_eq!(
            build_shape("<font><p>hello<b>cruel</font>world"),
            "(doc html[head[]body[font[]p[font[\"hello\"b[\"cruel\"]]b[\"world\"]]]])"
        );
    }

    #[test]
    fn eof_in_head_text_mode_still_implies_body() {
        assert_eq!(
            build_shape("<!DOCTYPE html><style> EOF"),
            "(doc <!DOCTYPE html>html[head[style[\" EOF\"]]body[]])"
        );
    }

    #[test]
    fn late_head_metadata_uses_the_head_pointer() {
        assert_eq!(
            build_shape("<head><meta></head><link>"),
            "(doc html[head[meta[]link[]]body[]])"
        );
        assert_eq!(
            build_shape("<body><meta><link>"),
            "(doc html[head[]body[meta[]link[]]])"
        );
    }

    #[test]
    fn list_item_end_respects_nested_list_scope() {
        assert_eq!(
            build_shape("<ul><li><ul></li><li>a</li></ul></li></ul>"),
            "(doc html[head[]body[ul[li[ul[li[\"a\"]]]]]])"
        );
    }

    #[test]
    fn table_only_start_tags_are_ignored_in_body() {
        assert_eq!(
            build_shape("<table></table><col><colgroup>"),
            "(doc html[head[]body[table[]]])"
        );
    }

    #[test]
    fn second_anchor_removes_stale_table_scoped_anchor() {
        assert_eq!(
            build_shape("<a><table><a></table><p><a><div><a>"),
            "(doc html[head[]body[a[a[]table[]]p[a[]]div[a[]]]])"
        );
    }

    #[test]
    fn frameset_tree_uses_frameset_insertion_modes() {
        assert_eq!(
            build_shape("<frameset><frame><frameset><frame></frameset><noframes></noframes></frameset>"),
            "(doc html[head[]frameset[frame[]frameset[frame[]]noframes[]]])"
        );
        assert_eq!(
            build_shape("<frameset>"),
            "(doc html[head[]frameset[]])"
        );
    }

    #[test]
    fn processing_instruction_precedes_document_element() {
        assert_eq!(
            build_shape("<?COMMENT?>"),
            "(doc <?COMMENT ?>html[head[]body[]])"
        );
        assert_eq!(
            build_shape("<?COM--MENT?>"),
            "(doc <?COM--MENT ?>html[head[]body[]])"
        );
    }

    #[test]
    fn in_head_splits_leading_whitespace_before_body_reprocessing() {
        assert_eq!(
            build_shape("<!DOCTYPE html><script> <!-- </script> --> </script> EOF"),
            "(doc <!DOCTYPE html>html[head[script[\" <!-- \"]]body[\"-->  EOF\"]])"
        );
    }

    #[test]
    fn self_closing_syntax_does_not_disable_html_text_state() {
        assert_eq!(
            build_shape("<textarea/>x<b>y</textarea><p>z"),
            "(doc html[head[]body[textarea[\"x<b>y\"]p[\"z\"]]])"
        );
    }

    #[test]
    fn noembed_uses_raw_text_in_body() {
        assert_eq!(
            build_shape("<noembed><b>x</b></noembed><p>y"),
            "(doc html[head[]body[noembed[\"<b>x</b>\"]p[\"y\"]]])"
        );
    }

    #[test]
    fn processing_instruction_after_html_is_a_document_child() {
        assert_eq!(
            build_shape("<body></body></html><?foo?>"),
            "(doc html[head[]body[]]<?foo ?>)"
        );
        assert_eq!(
            build_shape("<frameset></frameset></html><?foo?>"),
            "(doc html[head[]frameset[]]<?foo ?>)"
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
        assert_eq!(shape(&dom), "(doc <!DOCTYPE html>html[head[]body[p[\"x\"]]])");
    }

    #[test]
    fn doctype_identifiers_are_document_children() {
        let cases = [
            ("<!DOCTYPE html PUBLIC \"pub\">", "pub", ""),
            ("<!DOCTYPE html SYSTEM 'sys'>", "", "sys"),
            ("<!DOCTYPE Html PUBLIC 'p' \"s\"><p>x", "p", "s"),
            ("<!DOCTYPE html PUBLIC '' \"\">", "", ""),
            ("<!DOCTYPE html PUBLIC \"p\">", "p", ""),
            ("<!DOCTYPE html PUBLIC\"p\">", "p", ""),
            ("<!DOCTYPE html SYSTEM's'>", "", "s"),
            ("<!DOCTYPE html PUBLIC nope>", "", ""),
            ("<!DOCTYPE html SYSTEM nope>", "", ""),
            ("<!DOCTYPE html PUBLIC \"partial", "partial", ""),
            ("<!DOCTYPE html SYSTEM 'partial", "", "partial"),
            ("<!DOCTYPE html PUBLIC \"p>tail", "p", ""),
            ("<!DOCTYPE html SYSTEM \"s\"junk>", "", "s"),
            ("<!DOCTYPE html PUBLIC 'a\0b'>", "a\u{FFFD}b", ""),
        ];
        for (input, expected_public, expected_system) in cases {
            let dom = build(input);
            let children: Vec<_> = dom.children(NodeIdx(0)).collect();
            assert!(children.len() >= 2, "{input:?}");
            assert_eq!(dom.document_doctype(NodeIdx(0)).unwrap(), Some(children[0]), "{input:?}");
            assert!(matches!(dom.get(children[1]).kind, NodeKind::Element { ref name, .. } if name == "html"));
            assert_eq!(dom.get(children[0]).kind, NodeKind::DocumentType {
                name: "html".into(), public_id: expected_public.into(), system_id: expected_system.into(),
            }, "{input:?}");
            assert_eq!(dom.doctype.as_deref(), Some("html"));
        }
        assert_eq!(build_shape("<!DOCTYPE Html PUBLIC 'p' \"s\"><p>x"),
            "(doc <!DOCTYPE html>html[head[]body[p[\"x\"]]])");
    }

    /// HTML-32: `is_javascript_mime_type` previously (as an inline `matches!`
    /// in `classic_or_module_script_type`, duplicated in `amazing`'s
    /// `cmd/js.rs::is_classic_javascript_type`) had only 8 of mimesniff's 16
    /// "JavaScript MIME type" entries -- caught while triaging WPT's
    /// `script-type-and-language-js.html`. `value` here is already
    /// lowercased/trimmed, matching every call site's contract.
    #[test]
    fn javascript_mime_type_matches_the_full_mimesniff_list() {
        for value in [
            "application/ecmascript", "application/javascript", "application/x-ecmascript",
            "application/x-javascript", "text/ecmascript", "text/javascript",
            "text/javascript1.0", "text/javascript1.1", "text/javascript1.2",
            "text/javascript1.3", "text/javascript1.4", "text/javascript1.5",
            "text/jscript", "text/livescript", "text/x-ecmascript", "text/x-javascript",
        ] {
            assert!(is_javascript_mime_type(value), "{value} should be a recognized JS MIME type");
        }
        for value in ["", "javascript", "text/javascript1.6", "text/plain", "module"] {
            assert!(!is_javascript_mime_type(value), "{value} should not be recognized");
        }
    }
}
