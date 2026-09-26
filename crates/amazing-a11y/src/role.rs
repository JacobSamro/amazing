//! ARIA role taxonomy and HTML-AAM implicit role mapping.
//!
//! `Role` is the normalized role string the snapshot exposes. It contains the
//! ARIA 1.2 roles plus a small number of engine-only roles (`StaticText`,
//! `InlineTextBox`, `LineBreak`, `ListMarker`, `Iframe`, `Mark`) that HTML-AAM
//! assigns to elements which have no ARIA equivalent. Engine-only roles carry
//! a stable lowercase name like the ARIA ones.
//!
//! The implicit mapping follows HTML-AAM: an element's role depends on its tag
//! name, its attributes and its ancestry (`li` in a list, `footer` outside
//! sectioning content, `th` scope, `td` in a grid table, and so on). An
//! explicit, valid, non-abstract `role` attribute wins over the implicit role.

use std::collections::HashMap;

use amazing_dom::{Dom, NodeIdx, NodeKind};

use crate::host;

/// Normalized accessibility role.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub enum Role {
    // --- abstract roles (never exposed; kept for classification) ---
    Command,
    Composite,
    Input,
    Landmark,
    Range,
    Roletype,
    Section,
    Sectionhead,
    Select,
    Structure,
    Widget,
    Window,

    // --- widget roles ---
    Button,
    Checkbox,
    Gridcell,
    Link,
    Menuitem,
    Menuitemcheckbox,
    Menuitemradio,
    Option,
    Progressbar,
    Radio,
    Scrollbar,
    Searchbox,
    Slider,
    Spinbutton,
    Switch,
    Tab,
    Tabpanel,
    Textbox,
    Treeitem,
    Tooltip,

    // --- composite widget roles ---
    Combobox,
    Grid,
    Listbox,
    Menu,
    Menubar,
    Radiogroup,
    Tablist,
    Tree,
    Treegrid,

    // --- document structure roles ---
    Application,
    Article,
    Blockquote,
    Caption,
    Cell,
    Code,
    Columnheader,
    Definition,
    Deletion,
    Directory,
    Document,
    Emphasis,
    Feed,
    Figure,
    Generic,
    Group,
    Heading,
    Img,
    Insertion,
    List,
    Listitem,
    Log,
    Main,
    Marquee,
    Math,
    Meter,
    None,
    Note,
    Paragraph,
    Presentation,
    Row,
    Rowgroup,
    Rowheader,
    /// Tentative ARIA role (https://github.com/w3c/aria/pull/1931): a
    /// section's heading content, structurally like `banner`/`contentinfo`
    /// but scoped to a sectioning element rather than the whole page.
    Sectionheader,
    /// Tentative counterpart to `Sectionheader`.
    Sectionfooter,
    Separator,
    Strong,
    Subscript,
    Superscript,
    Table,
    Term,
    Time,
    Toolbar,

    // --- landmark roles ---
    Banner,
    Complementary,
    Contentinfo,
    Form,
    Navigation,
    Region,
    Search,

    // --- live region roles ---
    Alert,
    Status,
    Timer,

    // --- window roles ---
    Alertdialog,
    Dialog,

    // --- graphics roles ---
    GraphicsDocument,
    GraphicsObject,
    GraphicsSymbol,

    // --- engine-only roles ---
    StaticText,
    InlineTextBox,
    LineBreak,
    ListMarker,
    Iframe,
    Mark,
    Ruby,
    /// `<cite>`: HTML-AAM gives it a distinct "implicit platform computedrole"
    /// with no ARIA equivalent (html-aam's own `roles-minimum.tentative.html`
    /// names it `html-cite`), so it is excluded from the "minimum role"
    /// promotion the same way any other non-generic implicit role is.
    HtmlCite,
}

impl Role {
    /// The role token as it appears in snapshots: ARIA role names are already
    /// lowercase; engine-only roles use a stable lowercase identifier.
    pub fn as_str(self) -> &'static str {
        use Role::*;
        match self {
            Command => "command",
            Composite => "composite",
            Input => "input",
            Landmark => "landmark",
            Range => "range",
            Roletype => "roletype",
            Section => "section",
            Sectionhead => "sectionhead",
            Select => "select",
            Structure => "structure",
            Widget => "widget",
            Window => "window",
            Button => "button",
            Checkbox => "checkbox",
            Gridcell => "gridcell",
            Link => "link",
            Menuitem => "menuitem",
            Menuitemcheckbox => "menuitemcheckbox",
            Menuitemradio => "menuitemradio",
            Option => "option",
            Progressbar => "progressbar",
            Radio => "radio",
            Scrollbar => "scrollbar",
            Searchbox => "searchbox",
            Slider => "slider",
            Spinbutton => "spinbutton",
            Switch => "switch",
            Tab => "tab",
            Tabpanel => "tabpanel",
            Textbox => "textbox",
            Treeitem => "treeitem",
            Tooltip => "tooltip",
            Combobox => "combobox",
            Grid => "grid",
            Listbox => "listbox",
            Menu => "menu",
            Menubar => "menubar",
            Radiogroup => "radiogroup",
            Tablist => "tablist",
            Tree => "tree",
            Treegrid => "treegrid",
            Application => "application",
            Article => "article",
            Blockquote => "blockquote",
            Caption => "caption",
            Cell => "cell",
            Code => "code",
            Columnheader => "columnheader",
            Definition => "definition",
            Deletion => "deletion",
            // `directory` is deprecated in ARIA 1.2 and computes as `list`, so
            // `from_aria` never produces this variant anymore; kept only so
            // `as_str`/`Role` stay total if something else ever constructs it.
            Directory => "directory",
            Document => "document",
            Emphasis => "emphasis",
            Feed => "feed",
            Figure => "figure",
            Generic => "generic",
            Group => "group",
            Heading => "heading",
            // ARIA 1.2 treats `img`/`image` as synonyms; both compute the
            // canonical string `image` (see `from_aria`, which maps both
            // tokens to this variant).
            Img => "image",
            Insertion => "insertion",
            List => "list",
            Listitem => "listitem",
            Log => "log",
            Main => "main",
            Marquee => "marquee",
            Math => "math",
            Meter => "meter",
            None => "none",
            Note => "note",
            Paragraph => "paragraph",
            Presentation => "presentation",
            Row => "row",
            Rowgroup => "rowgroup",
            Rowheader => "rowheader",
            Sectionheader => "sectionheader",
            Sectionfooter => "sectionfooter",
            Separator => "separator",
            Strong => "strong",
            Subscript => "subscript",
            Superscript => "superscript",
            Table => "table",
            Term => "term",
            Time => "time",
            Toolbar => "toolbar",
            Banner => "banner",
            Complementary => "complementary",
            Contentinfo => "contentinfo",
            Form => "form",
            Navigation => "navigation",
            Region => "region",
            Search => "search",
            Alert => "alert",
            Status => "status",
            Timer => "timer",
            Alertdialog => "alertdialog",
            Dialog => "dialog",
            GraphicsDocument => "graphics-document",
            GraphicsObject => "graphics-object",
            GraphicsSymbol => "graphics-symbol",
            StaticText => "statictext",
            InlineTextBox => "inlinetextbox",
            LineBreak => "linebreak",
            ListMarker => "listmarker",
            Iframe => "iframe",
            Mark => "mark",
            Ruby => "ruby",
            HtmlCite => "html-cite",
        }
    }

    /// Parses an explicit `role` token. Unknown tokens and abstract roles are
    /// rejected so the caller can fall through to the implicit role.
    pub fn from_aria(token: &str) -> Option<Role> {
        use Role::*;
        let role = match token {
            "button" => Button,
            "checkbox" => Checkbox,
            "gridcell" => Gridcell,
            "link" => Link,
            "menuitem" => Menuitem,
            "menuitemcheckbox" => Menuitemcheckbox,
            "menuitemradio" => Menuitemradio,
            "option" => Option,
            "progressbar" => Progressbar,
            "radio" => Radio,
            "scrollbar" => Scrollbar,
            "searchbox" => Searchbox,
            "separator" => Separator,
            "slider" => Slider,
            "spinbutton" => Spinbutton,
            "switch" => Switch,
            "tab" => Tab,
            "tabpanel" => Tabpanel,
            "textbox" => Textbox,
            "treeitem" => Treeitem,
            "tooltip" => Tooltip,
            "combobox" => Combobox,
            "grid" => Grid,
            "listbox" => Listbox,
            "menu" => Menu,
            "menubar" => Menubar,
            "radiogroup" => Radiogroup,
            "tablist" => Tablist,
            "tree" => Tree,
            "treegrid" => Treegrid,
            "application" => Application,
            "article" => Article,
            "blockquote" => Blockquote,
            "caption" => Caption,
            "cell" => Cell,
            "code" => Code,
            "columnheader" => Columnheader,
            "definition" => Definition,
            "deletion" => Deletion,
            // Deprecated in ARIA 1.2; computes as `list`.
            "directory" => List,
            "document" => Document,
            "emphasis" => Emphasis,
            "feed" => Feed,
            "figure" => Figure,
            "generic" => Generic,
            "group" => Group,
            "heading" => Heading,
            "img" => Img,
            "image" => Img,
            "insertion" => Insertion,
            "list" => List,
            "listitem" => Listitem,
            "log" => Log,
            "main" => Main,
            "marquee" => Marquee,
            "math" => Math,
            "meter" => Meter,
            "none" => None,
            "note" => Note,
            "paragraph" => Paragraph,
            "presentation" => Presentation,
            "row" => Row,
            "rowgroup" => Rowgroup,
            "rowheader" => Rowheader,
            "sectionheader" => Sectionheader,
            "sectionfooter" => Sectionfooter,
            "strong" => Strong,
            "subscript" => Subscript,
            "superscript" => Superscript,
            "table" => Table,
            "term" => Term,
            "time" => Time,
            "toolbar" => Toolbar,
            "banner" => Banner,
            "complementary" => Complementary,
            "contentinfo" => Contentinfo,
            "form" => Form,
            "navigation" => Navigation,
            "region" => Region,
            "search" => Search,
            "alert" => Alert,
            "status" => Status,
            "timer" => Timer,
            "alertdialog" => Alertdialog,
            "dialog" => Dialog,
            "graphics-document" => GraphicsDocument,
            "graphics-object" => GraphicsObject,
            "graphics-symbol" => GraphicsSymbol,
            _ => return std::option::Option::None,
        };
        if role.is_abstract() { std::option::Option::None } else { Some(role) }
    }

    /// Abstract roles never appear in the accessibility tree.
    pub fn is_abstract(self) -> bool {
        use Role::*;
        matches!(
            self,
            Command
                | Composite
                | Input
                | Landmark
                | Range
                | Roletype
                | Section
                | Sectionhead
                | Select
                | Structure
                | Widget
                | Window
        )
    }

    /// `none` and `presentation` remove the node unless it carries a global
    /// ARIA attribute or focusability (the "presentational role conflict"
    /// rule in ARIA).
    pub fn is_presentational(self) -> bool {
        matches!(self, Role::None | Role::Presentation)
    }

    /// Roles that support `name from content` (accname-1.2 §2F). `Blockquote`
    /// and `Figure` are deliberately absent (A11Y-10): per HTML-AAM, neither
    /// derives its accessible name from its own rendered text — a
    /// `<blockquote>`'s content is a quotation, not a label, and a
    /// `<figure>`'s name comes only from `aria-label`/`aria-labelledby`/
    /// `title` (its `<figcaption>` is a separate, `img`-only fallback — see
    /// `figure_figcaption_fallback` in `accname.rs`, not this content walk).
    pub fn name_from_content(self) -> bool {
        use Role::*;
        matches!(
            self,
            Button
                | Cell
                | Checkbox
                | Columnheader
                | Gridcell
                | Heading
                | Link
                | Menuitem
                | Menuitemcheckbox
                | Menuitemradio
                | Option
                | Radio
                | Row
                | Rowheader
                | Switch
                | Tab
                | Tooltip
                | Treeitem
                | StaticText
                | InlineTextBox
                | Caption
                | Definition
                | Term
                | Listitem
                | Paragraph
                | Code
                | Emphasis
                | Strong
                | Subscript
                | Superscript
                | Deletion
                | Insertion
                | Mark
                | Time
                | Ruby
        )
    }

    /// Landmark roles participate in landmark navigation and naming.
    pub fn is_landmark(self) -> bool {
        use Role::*;
        matches!(
            self,
            Banner | Complementary | Contentinfo | Form | Main | Navigation | Region | Search
        )
    }

    /// Whether the role is a form/interactive control that can be named by a
    /// `<label>` element.
    pub fn is_labelable_widget(self) -> bool {
        use Role::*;
        matches!(
            self,
            Button
                | Checkbox
                | Combobox
                | Gridcell
                | Link
                | Listbox
                | Menuitem
                | Menuitemcheckbox
                | Menuitemradio
                | Meter
                | Option
                | Progressbar
                | Radio
                | Scrollbar
                | Searchbox
                | Slider
                | Spinbutton
                | Switch
                | Tab
                | Textbox
                | Treeitem
        )
    }
}

/// Resolved role for an element: its explicit ARIA role when valid, otherwise
/// the HTML-AAM implicit role, with the HTML-AAM "minimum role" promotion
/// (A11Y-10) applied last. `ids` is a document-wide id -> node index map (the
/// same one `tree::Builder`/`accname::NameContext` already build), used to
/// resolve `aria-labelledby` references for the landmark-name-required checks
/// below.
pub fn role_for(dom: &Dom, ids: &HashMap<String, NodeIdx>, node: NodeIdx) -> Role {
    let role = if let Some(explicit) = explicit_role(dom, ids, node) {
        explicit
    } else {
        implicit_role(dom, ids, node)
    };
    apply_minimum_role(dom, node, role)
}

/// First valid, non-abstract token of the `role` attribute. `none`/`presentation`
/// are valid explicit roles and are returned as-is. Role tokens are matched
/// case-insensitively (ARIA 1.2 role-token matching is ASCII case-insensitive).
/// `form` and `region` additionally require an author-supplied accessible
/// name (`aria-label`/`aria-labelledby`/`title`) to be a valid match; without
/// one the token is skipped and the fallback list continues, per ARIA 1.2's
/// name-required landmark roles (section 5.2.8).
pub fn explicit_role(dom: &Dom, ids: &HashMap<String, NodeIdx>, node: NodeIdx) -> Option<Role> {
    let value = dom.get_attribute_ns(node, None, "role")?;
    value.split_ascii_whitespace().find_map(|token| {
        let lower = token.to_ascii_lowercase();
        let role = Role::from_aria(&lower)?;
        if matches!(role, Role::Form | Role::Region) && !has_author_name(dom, ids, node) {
            return None;
        }
        Some(role)
    })
}

/// HTML-AAM implicit role. `Role::Generic` is the fallback for elements that
/// expose a generic container role; elements with no role at all return
/// `None` and are pruned by the tree builder.
pub fn implicit_role(dom: &Dom, ids: &HashMap<String, NodeIdx>, node: NodeIdx) -> Role {
    let NodeKind::Element { name, .. } = &dom.get(node).kind else {
        return Role::None;
    };
    let tag = name.as_str();
    // Only HTML elements have the HTML-AAM mapping; foreign content is generic.
    match dom.namespace(node) {
        Some(amazing_dom::Namespace::Html) => {}
        Some(amazing_dom::Namespace::Svg) => {
            return if tag == "svg" { Role::GraphicsDocument } else { Role::GraphicsObject };
        }
        Some(amazing_dom::Namespace::MathMl) => {
            return if tag == "math" { Role::Math } else { Role::Generic };
        }
        _ => return Role::Generic,
    }

    let attr = |name: &str| dom.get_attribute_ns(node, None, name);
    match tag {
        "a" | "area" => {
            if attr("href").is_some() {
                Role::Link
            } else {
                Role::Generic
            }
        }
        "article" => Role::Article,
        "address" => Role::Group,
        // An `article`/`aside`/`nav`/`section` ancestor demotes `aside` to
        // generic unless it still has an accessible name of its own
        // (HTML-AAM #el-aside), the same name-required pattern as
        // `form`/`section`/`search` — but note `main` does NOT trigger this
        // demotion (unlike `footer`/`header`'s sectioning-ancestor check
        // below, which does include `main`; confirmed against the pinned
        // `roles-contextual.html`'s `el-aside-in-main` case, which stays
        // `complementary` with no name at all).
        "aside" => {
            if has_aside_demoting_ancestor(dom, node) && !has_author_name(dom, ids, node) {
                Role::Generic
            } else {
                Role::Complementary
            }
        }
        "body" => Role::Generic,
        // `Blockquote` intentionally does not appear in `Role::name_from_content`:
        // per HTML-AAM/accname-1.2 a blockquote's own text is not its name (see
        // `dir`-style pinned selection A11Y-10).
        "blockquote" => Role::Blockquote,
        "br" => Role::LineBreak,
        "button" => Role::Button,
        "caption" => Role::Caption,
        // No ARIA role covers `<cite>`; HTML-AAM's own tests key it as
        // `html-cite`, the same engine-only-role pattern as `Code`.
        "cite" => Role::HtmlCite,
        "code" => Role::Code,
        "datalist" => Role::Listbox,
        "dd" => Role::Definition,
        "del" => Role::Deletion,
        // Obsolete alias for `ul` (HTML-AAM still maps it to `list`).
        "dir" => Role::List,
        "dl" => Role::List,
        "details" => Role::Group,
        "dfn" => Role::Term,
        "dialog" => Role::Dialog,
        "dt" => Role::Term,
        "em" => Role::Emphasis,
        "fieldset" => Role::Group,
        "figcaption" => Role::Caption,
        "figure" => Role::Figure,
        // Nested in sectioning content (or `main`), a footer/header is a
        // "section footer/header" (ARIA PR 1931) rather than losing its
        // landmark-ness outright — and, unlike aside/form/section, this
        // demotion is unconditional (no accessible name required).
        "footer" => {
            if has_sectioning_ancestor(dom, node) {
                Role::Sectionfooter
            } else {
                Role::Contentinfo
            }
        }
        "form" => {
            // A form is a landmark only when it has an accessible name.
            if has_author_name(dom, ids, node) {
                Role::Form
            } else {
                Role::Generic
            }
        }
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => Role::Heading,
        "header" => {
            if has_sectioning_ancestor(dom, node) {
                Role::Sectionheader
            } else {
                Role::Banner
            }
        }
        "hgroup" => Role::Group,
        "hr" => Role::Separator,
        "html" => Role::Document,
        "i" => Role::Generic,
        "iframe" => Role::Iframe,
        "img" => implicit_img_role(dom, ids, node),
        "input" => implicit_input_role(dom, node),
        "ins" => Role::Insertion,
        "label" => Role::None,
        "legend" => Role::None,
        "li" => {
            if has_list_ancestor(dom, ids, node) {
                Role::Listitem
            } else {
                Role::Generic
            }
        }
        "main" => Role::Main,
        "mark" => Role::Mark,
        "math" => Role::Math,
        "menu" => Role::List,
        "meter" => Role::Meter,
        "nav" => Role::Navigation,
        "ol" | "ul" => Role::List,
        "optgroup" => Role::Group,
        "option" => Role::Option,
        "output" => Role::Status,
        "p" => Role::Paragraph,
        "pre" => Role::Generic,
        "progress" => Role::Progressbar,
        "ruby" => Role::Ruby,
        // Unlike `form`/`section`, HTML-AAM's `<search>` mapping is
        // unconditional (confirmed by the pinned `roles.html`'s bare,
        // nameless `el-search` case, which still expects `search` — it is
        // not one of ARIA 1.2 5.2.8's name-required landmark roles).
        "search" => Role::Search,
        "section" => {
            if has_author_name(dom, ids, node) {
                Role::Region
            } else {
                Role::Generic
            }
        }
        "select" => {
            let multiple = attr("multiple").is_some();
            let size = attr("size")
                .and_then(|s| s.trim().parse::<i64>().ok())
                .unwrap_or(0);
            if multiple || size > 1 {
                Role::Listbox
            } else {
                Role::Combobox
            }
        }
        "strong" => Role::Strong,
        "s" => Role::Deletion,
        "sub" => Role::Subscript,
        "summary" => Role::Button,
        "sup" => Role::Superscript,
        "table" => Role::Table,
        "tbody" | "thead" | "tfoot" => Role::Rowgroup,
        "td" => {
            if crate::host::has_grid_ancestor(dom, node) {
                Role::Gridcell
            } else {
                Role::Cell
            }
        }
        "textarea" => Role::Textbox,
        "th" => {
            // Explicit scope always wins. With no scope, a `<th>` inside
            // `<thead>` is a columnheader; inside `<tbody>`/`<tfoot>` (or a
            // headerless table body) it is a rowheader (HTML-AAM #el-th's
            // default table header algorithm, simplified to the thead/not-
            // thead split the pinned `table-roles.html` selection exercises).
            match attr("scope").map(str::trim) {
                Some("row") | Some("rowgroup") => Role::Rowheader,
                Some("col") | Some("colgroup") => Role::Columnheader,
                _ => {
                    if th_in_thead(dom, node) {
                        Role::Columnheader
                    } else {
                        Role::Rowheader
                    }
                }
            }
        }
        "time" => Role::Time,
        "tr" => Role::Row,
        // Elements that never produce an accessibility node.
        "base" | "head" | "link" | "meta" | "noscript" | "param" | "script"
        | "slot" | "source" | "style" | "template" | "title" | "track" => Role::None,
        // Elements with no ARIA mapping are generic containers.
        _ => Role::Generic,
    }
}

/// HTML-AAM's `img` role algorithm (html-aam #el-img plus the tentative
/// `img-src-srcset-roles` extension pinned by A11Y-10). In precedence order:
///
/// 1. An `alt` attribute present with empty/whitespace-only content is a
///    decorative signal (`Role::Presentation`), overridden only by a real
///    ARIA name (`aria-label`/`aria-labelledby` resolving to non-empty
///    content) — `title` does NOT override it.
/// 2. Otherwise (`alt` absent, or non-whitespace), the element is `Role::Img`
///    when it has *any* accessible name (alt text, `aria-label`,
///    `aria-labelledby`, or — only when `alt` is entirely absent — `title`),
///    per the HTML spec's img-accname precedence.
/// 3. With no name at all, it is still `Role::Img` if it has a "valid" `src`
///    or `srcset` (present and non-empty; parse-validity of the URL/
///    descriptor is not checked, matching every "broken value" fixture in
///    the pinned selection), since the element still represents an image
///    request. With neither a name nor a valid source, HTML says the element
///    "represents nothing", so it is `Role::Presentation`.
fn implicit_img_role(dom: &Dom, ids: &HashMap<String, NodeIdx>, node: NodeIdx) -> Role {
    let attr = |name: &str| dom.get_attribute_ns(node, None, name);
    let non_empty_trim = |v: Option<&str>| v.is_some_and(|s| !s.trim().is_empty());

    let has_aria_name = non_empty_trim(attr("aria-label"))
        || attr("aria-labelledby").is_some_and(|id_list| {
            id_list.split_ascii_whitespace().any(|id| {
                ids.get(id).is_some_and(|&target| !host::child_text(dom, target).trim().is_empty())
            })
        });

    if let Some(alt) = attr("alt")
        && alt.trim().is_empty()
    {
        return if has_aria_name { Role::Img } else { Role::Presentation };
    }

    let has_name = non_empty_trim(attr("alt"))
        || has_aria_name
        || (attr("alt").is_none() && non_empty_trim(attr("title")));
    if has_name {
        return Role::Img;
    }

    let has_valid_source =
        non_empty_trim(attr("src")) || non_empty_trim(attr("srcset"));
    if has_valid_source { Role::Img } else { Role::Presentation }
}

fn implicit_input_role(dom: &Dom, node: NodeIdx) -> Role {
    let ty = dom
        .get_attribute_ns(node, None, "type")
        .map(|t| t.trim().to_ascii_lowercase())
        .unwrap_or_default();
    match ty.as_str() {
        "checkbox" => {
            // The `switch` content attribute (no value needed) switches a
            // checkbox's role to `switch` (HTML-AAM's `switch` mapping).
            if dom.get_attribute_ns(node, None, "switch").is_some() {
                Role::Switch
            } else {
                Role::Checkbox
            }
        }
        "button" | "submit" | "reset" | "image" => Role::Button,
        "radio" => Role::Radio,
        "range" => Role::Slider,
        "number" => Role::Spinbutton,
        "search" => Role::Searchbox,
        "email" | "tel" | "url" | "text" | "password" | "" => Role::Textbox,
        "color" | "date" | "datetime-local" | "file" | "month" | "time" | "week" => {
            Role::Textbox
        }
        "hidden" => Role::None,
        _ => Role::Textbox,
    }
}

/// HTML-AAM's "minimum role" (html-aam #minimum-role, HTML-AAM issue #454,
/// A11Y-10): a generic/none/presentational element that carries `autofocus`,
/// `draggable` or `popover` is promoted to `Role::Group` instead, since HTML
/// gives those attributes behavior an AT should not silently lose. This is a
/// final step, applied on top of whatever explicit/implicit role resolution
/// produced (including an explicit `role="none"`/`"generic"` or an invalid
/// token, none of which are "valid" enough to opt out per the fixtures this
/// mirrors) — an explicit *valid, non-generic/non-none* role, or an implicit
/// role that isn't generic/none/presentational, is unaffected.
pub(crate) fn apply_minimum_role(dom: &Dom, node: NodeIdx, role: Role) -> Role {
    if matches!(role, Role::Generic | Role::None | Role::Presentation)
        && (dom.get_attribute_ns(node, None, "autofocus").is_some()
            || dom.get_attribute_ns(node, None, "draggable").is_some()
            || dom.get_attribute_ns(node, None, "popover").is_some())
    {
        Role::Group
    } else {
        role
    }
}

/// True when the element is inside sectioning content (`article`, `aside`,
/// `main`, `nav`, `section`), which demotes `header`/`footer` landmarks.
fn has_sectioning_ancestor(dom: &Dom, node: NodeIdx) -> bool {
    let mut cursor = dom.get(node).parent;
    while let Some(parent) = cursor {
        if matches!(dom.name(parent), Some("article" | "aside" | "main" | "nav" | "section")) {
            return true;
        }
        cursor = dom.get(parent).parent;
    }
    false
}

/// `aside`'s own, narrower demoting-ancestor set: `article`, `aside`, `nav`,
/// `section` — NOT `main` (see the call site's comment).
fn has_aside_demoting_ancestor(dom: &Dom, node: NodeIdx) -> bool {
    let mut cursor = dom.get(node).parent;
    while let Some(parent) = cursor {
        if matches!(dom.name(parent), Some("article" | "aside" | "nav" | "section")) {
            return true;
        }
        cursor = dom.get(parent).parent;
    }
    false
}

/// True when `node` (a `<th>` with no explicit `scope`) is in a row inside a
/// `<thead>` ancestor, rather than `<tbody>`/`<tfoot>`.
fn th_in_thead(dom: &Dom, node: NodeIdx) -> bool {
    let mut cursor = dom.get(node).parent;
    while let Some(parent) = cursor {
        match dom.name(parent) {
            Some("thead") => return true,
            Some("tbody" | "tfoot" | "table") => return false,
            _ => {}
        }
        cursor = dom.get(parent).parent;
    }
    false
}

/// True when `node` (an `<li>`) is an accessible child of a list: walking up
/// the DOM, ancestors whose *resolved role* is `Generic` (a plain wrapper
/// `<div>`, or a list-ish element demoted by, say, `role="none"`) are
/// transparent and skipped; the walk stops at the first ancestor with any
/// other role, which counts as a list ancestor only when that role is
/// `Role::List` (covers `<ul>`/`<ol>`/`<menu>`/`<dl>`/`<dir>`, and an
/// explicit `role="list"` on any element, e.g. a plain `<div>`).
///
/// This does not see `aria-owns`-based reparenting (a later, separate
/// `tree::Builder` pass over the already-built `AxTree`, after roles are
/// decided) — an orphaned `<li>` owned only via `aria-owns` is a documented,
/// logged gap (A11Y-10, `html_aam_selection`'s `KNOWN_GAP_ROLE_TESTNAMES`).
fn has_list_ancestor(dom: &Dom, ids: &HashMap<String, NodeIdx>, node: NodeIdx) -> bool {
    let mut cursor = dom.get(node).parent;
    while let Some(parent) = cursor {
        if dom.name(parent).is_some() {
            match role_for(dom, ids, parent) {
                Role::List => return true,
                Role::Generic => {}
                _ => return false,
            }
        }
        cursor = dom.get(parent).parent;
    }
    false
}

/// An author-provided name: non-empty `aria-label`, an `aria-labelledby`
/// reference that resolves to an element with non-empty text content, or a
/// non-empty `title`. This is a lighter-weight approximation of the full
/// accname-1.2 algorithm — good enough for the landmark/aside/img
/// name-required decisions above (whose pinned fixtures only ever reference
/// plain-text targets), but it reads raw `host::child_text` rather than
/// recursing through `accname::accessible_name`, so a target named only via
/// its own `aria-label` (rather than text content) would be under-counted
/// here. `role.rs` is a dependency of `accname.rs`, so it cannot call back
/// into the full algorithm without a cycle; documented rather than routed
/// around.
fn has_author_name(dom: &Dom, ids: &HashMap<String, NodeIdx>, node: NodeIdx) -> bool {
    if dom
        .get_attribute_ns(node, None, "aria-label")
        .is_some_and(|v| !v.trim().is_empty())
    {
        return true;
    }
    if let Some(id_list) = dom.get_attribute_ns(node, None, "aria-labelledby")
        && id_list.split_ascii_whitespace().any(|id| {
            ids.get(id).is_some_and(|&target| !host::child_text(dom, target).trim().is_empty())
        })
    {
        return true;
    }
    if dom
        .get_attribute_ns(node, None, "title")
        .is_some_and(|v| !v.trim().is_empty())
    {
        return true;
    }
    false
}

/// Global ARIA attributes that keep a presentational element in the tree.
pub fn has_global_aria(dom: &Dom, node: NodeIdx) -> bool {
    const GLOBAL: &[&str] = &[
        "aria-atomic", "aria-busy", "aria-controls", "aria-current", "aria-describedby",
        "aria-description", "aria-details", "aria-disabled", "aria-dropeffect", "aria-errormessage",
        "aria-flowto", "aria-grabbed", "aria-haspopup", "aria-hidden", "aria-invalid", "aria-keyshortcuts",
        "aria-label", "aria-labelledby", "aria-live", "aria-owns", "aria-relevant", "aria-roledescription",
    ];
    let Some(attrs) = dom.attrs_of(node) else { return false };
    attrs.iter().any(|(name, _)| GLOBAL.contains(&name.as_str()))
}
