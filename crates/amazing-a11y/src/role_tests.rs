//! Table-driven HTML-AAM implicit role and ARIA explicit role fixtures.

use crate::Role;
use crate::test_support::{document, first_role, implicit_role_of_tag, role_for_tag, role_of_tag};

#[test]
fn html_aam_implicit_roles() {
    use Role::*;
    let cases: &[(&str, Role)] = &[
        ("<a href=\"#\">x</a>", Link),
        ("<a>x</a>", Generic),
        ("<article>x</article>", Article),
        ("<aside>x</aside>", Complementary),
        ("<blockquote>x</blockquote>", Blockquote),
        ("<button>x</button>", Button),
        ("<code>x</code>", Code),
        ("<dd>x</dd>", Definition),
        ("<del>x</del>", Deletion),
        ("<details>x</details>", Group),
        ("<dfn>x</dfn>", Term),
        ("<dialog>x</dialog>", Dialog),
        ("<dt>x</dt>", Term),
        ("<em>x</em>", Emphasis),
        ("<fieldset><legend>l</legend></fieldset>", Group),
        ("<figure><figcaption>c</figcaption></figure>", Figure),
        ("<h1>x</h1>", Heading),
        ("<h6>x</h6>", Heading),
        ("<hgroup><h1>x</h1></hgroup>", Group),
        ("<hr>", Separator),
        ("<ins>x</ins>", Insertion),
        ("<li>x</li>", Generic), // no list ancestor
        ("<ul><li>x</li></ul>", List),
        ("<main>x</main>", Main),
        ("<mark>x</mark>", Mark),
        ("<math></math>", Math),
        ("<menu><li>x</li></menu>", List),
        ("<meter>1</meter>", Meter),
        ("<nav>x</nav>", Navigation),
        ("<ol><li>x</li></ol>", List),
        ("<option>x</option>", Option),
        ("<output>x</output>", Status),
        ("<p>x</p>", Paragraph),
        ("<progress>1</progress>", Progressbar),
        ("<strong>x</strong>", Strong),
        ("<sub>x</sub>", Subscript),
        ("<summary>x</summary>", Button),
        ("<sup>x</sup>", Superscript),
        ("<table><tr><td>x</td></tr></table>", Table),
        ("<textarea></textarea>", Textbox),
        ("<time>2020</time>", Time),
        ("<p>x</p>", Paragraph),
        // Text-level and structural elements with generic or engine-only roles.
        ("<address>x</address>", Group),
        ("<b>x</b>", Generic),
        ("<bdi>x</bdi>", Generic),
        ("<bdo>x</bdo>", Generic),
        ("<cite>x</cite>", HtmlCite),
        ("<data value=\"1\">x</data>", Generic),
        ("<div>x</div>", Generic),
        ("<i>x</i>", Generic),
        ("<kbd>x</kbd>", Generic),
        ("<map name=\"m\">x</map>", Generic),
        ("<object>x</object>", Generic),
        ("<pre>x</pre>", Generic),
        ("<q>x</q>", Generic),
        ("<s>x</s>", Deletion),
        ("<samp>x</samp>", Generic),
        ("<small>x</small>", Generic),
        ("<span>x</span>", Generic),
        ("<u>x</u>", Generic),
        ("<var>x</var>", Generic),
        ("<wbr>", Generic),
        ("<canvas>x</canvas>", Generic),
        ("<audio>x</audio>", Generic),
        ("<video>x</video>", Generic),
        ("<dl><dt>t</dt><dd>d</dd></dl>", List),
        ("<optgroup><option>a</option></optgroup>", Group),
        ("<datalist><option>a</option></datalist>", Listbox),
        ("<ruby>x</ruby>", Ruby),
        ("<iframe></iframe>", Iframe),
        ("<output>x</output>", Status),
        ("<details>x</details>", Group),
        ("<hgroup><h1>x</h1></hgroup>", Group),
        ("<br>", LineBreak),
        ("<hr>", Separator),
        ("<input>", Textbox),
        ("<textarea></textarea>", Textbox),
        ("<select><option>a</option></select>", Combobox),
        ("<dialog>x</dialog>", Dialog),
        // Elements that expose no accessibility node.
        ("<base>", Role::None),
        ("<head><title>t</title></head>", Role::None),
        ("<label>x</label>", Role::None),
        ("<legend>x</legend>", Role::None),
        ("<link>", Role::None),
        ("<meta>", Role::None),
        ("<noscript>x</noscript>", Role::None),
        ("<param>", Role::None),
        ("<script>x</script>", Role::None),
        ("<slot>x</slot>", Role::None),
        ("<source>", Role::None),
        ("<style>x</style>", Role::None),
        ("<template>x</template>", Role::None),
        ("<track>", Role::None),
    ];
    for (html, expected) in cases {
        // Find the tag we mean: the first non-structural element.
        let tag = first_tag(html);
        assert_eq!(
            implicit_role_of_tag(html, &tag),
            Some(*expected),
            "tag <{tag}> in {html:?}"
        );
    }
}

fn first_tag(html: &str) -> String {
    let trimmed = html.trim_start_matches('<');
    let end = trimmed
        .find(|c: char| c.is_whitespace() || c == '>' || c == '/')
        .unwrap_or(trimmed.len());
    trimmed[..end].to_string()
}

#[test]
fn heading_levels() {
    let doc = document("<h3>Title</h3>");
    let heading = first_role(&doc, Role::Heading).unwrap();
    assert_eq!(heading.name, "Title");
    assert_eq!(heading.properties.level, Some(3));
}

#[test]
fn context_sensitive_landmarks() {
    // header/footer are landmarks at the top level...
    assert_eq!(
        implicit_role_of_tag("<body><header>x</header></body>", "header"),
        Some(Role::Banner)
    );
    assert_eq!(
        implicit_role_of_tag("<body><footer>x</footer></body>", "footer"),
        Some(Role::Contentinfo)
    );
    // ...but demoted inside sectioning content (A11Y-10: to `sectionheader`/
    // `sectionfooter` per ARIA PR 1931, not plain `generic` — confirmed via
    // the pinned `roles-contextual.tentative.html`'s nameless
    // `el-footer-ancestormain` case, which still expects `sectionfooter`).
    assert_eq!(
        implicit_role_of_tag("<article><header>x</header></article>", "header"),
        Some(Role::Sectionheader)
    );
    assert_eq!(
        implicit_role_of_tag("<section><footer>x</footer></section>", "footer"),
        Some(Role::Sectionfooter)
    );
}

#[test]
fn form_and_section_are_landmarks_only_when_named() {
    assert_eq!(
        implicit_role_of_tag("<form><input></form>", "form"),
        Some(Role::Generic)
    );
    assert_eq!(
        implicit_role_of_tag("<form aria-label=\"Search\"><input></form>", "form"),
        Some(Role::Form)
    );
    assert_eq!(
        implicit_role_of_tag("<section><p>x</p></section>", "section"),
        Some(Role::Generic)
    );
    assert_eq!(
        implicit_role_of_tag("<section aria-label=\"News\"><p>x</p></section>", "section"),
        Some(Role::Region)
    );
}

#[test]
fn select_variants() {
    assert_eq!(
        implicit_role_of_tag("<select><option>a</option></select>", "select"),
        Some(Role::Combobox)
    );
    assert_eq!(
        implicit_role_of_tag("<select multiple><option>a</option></select>", "select"),
        Some(Role::Listbox)
    );
    assert_eq!(
        implicit_role_of_tag("<select size=\"4\"><option>a</option></select>", "select"),
        Some(Role::Listbox)
    );
}

#[test]
fn input_types() {
    use Role::*;
    let cases = [
        ("text", Textbox),
        ("search", Searchbox),
        ("email", Textbox),
        ("password", Textbox),
        ("number", Spinbutton),
        ("range", Slider),
        ("checkbox", Checkbox),
        ("radio", Radio),
        ("button", Button),
        ("submit", Button),
        ("reset", Button),
        ("image", Button),
        ("color", Textbox),
        ("date", Textbox),
    ];
    for (ty, expected) in cases {
        let html = format!("<input type=\"{ty}\">");
        assert_eq!(
            implicit_role_of_tag(&html, "input"),
            Some(expected),
            "type={ty}"
        );
    }
}

#[test]
fn table_roles() {
    use Role::*;
    let html = "<table><thead><tr><th scope=\"col\">H</th></tr></thead>\
                <tbody><tr><td>c</td></tr></tbody></table>";
    let doc = document(html);
    assert!(first_role(&doc, Table).is_some());
    assert!(first_role(&doc, Rowgroup).is_some());
    assert!(first_role(&doc, Row).is_some());
    assert!(first_role(&doc, Columnheader).is_some());
    assert!(first_role(&doc, Cell).is_some());

    let grid = document("<table role=\"grid\"><tr><td>c</td></tr></table>");
    assert!(first_role(&grid, Gridcell).is_some(), "td in a grid is a gridcell");
}

#[test]
fn th_row_scope_is_rowheader() {
    assert_eq!(
        implicit_role_of_tag(
            "<table role=\"grid\"><tr><th scope=\"row\">r</th></tr></table>",
            "th"
        ),
        Some(Role::Rowheader)
    );
}

#[test]
fn explicit_role_overrides_implicit() {
    assert_eq!(
        role_for_tag("<div role=\"button\">x</div>", "div"),
        Some(Role::Button)
    );
    // First valid token wins; abstract roles are skipped.
    assert_eq!(
        role_for_tag("<div role=\"widget tab\">x</div>", "div"),
        Some(Role::Tab)
    );
    // Unknown tokens fall back to the implicit role.
    assert_eq!(
        role_for_tag("<div role=\"brontosaurus\">x</div>", "div"),
        Some(Role::Generic)
    );
    // `presentation` is a valid explicit role; the conflict rule that keeps a
    // focusable/ARIA-bearing element is exercised below through the tree.
    assert_eq!(
        role_for_tag("<nav role=\"presentation\"><a href=\"#\">x</a></nav>", "nav"),
        Some(Role::Presentation)
    );
    // ...and the pruned nav does not appear in the built tree.
    assert_eq!(
        role_of_tag("<nav role=\"presentation\"><a href=\"#\">x</a></nav>", "nav"),
        None
    );
    assert_eq!(
        role_of_tag("<nav role=\"presentation\"><a href=\"#\">x</a></nav>", "a"),
        Some(Role::Link)
    );
}

#[test]
fn presentational_conflict_resolution() {
    // role=presentation on a focusable element is ignored.
    assert_eq!(
        role_of_tag("<div role=\"presentation\" tabindex=\"0\">x</div>", "div"),
        Some(Role::Generic)
    );
    // role=presentation with a global ARIA attribute is ignored.
    assert_eq!(
        role_of_tag("<img src=\"a.png\" alt=\"pic\" role=\"presentation\" aria-label=\"p\">", "img"),
        Some(Role::Img)
    );
    // Empty alt removes a decorative image.
    assert_eq!(role_of_tag("<img src=\"a.png\" alt=\"\">", "img"), None);
}

#[test]
fn foreign_content_roles() {
    assert_eq!(
        implicit_role_of_tag("<svg></svg>", "svg"),
        Some(Role::GraphicsDocument)
    );
    assert_eq!(
        implicit_role_of_tag("<svg><rect/></svg>", "rect"),
        Some(Role::GraphicsObject)
    );
    assert_eq!(implicit_role_of_tag("<math></math>", "math"), Some(Role::Math));
}

#[test]
fn role_name_round_trip() {
    for role in [
        Role::Button,
        Role::Checkbox,
        Role::Columnheader,
        Role::GraphicsDocument,
        Role::Listitem,
        Role::Tabpanel,
    ] {
        assert_eq!(Role::from_aria(role.as_str()), Some(role), "{role:?}");
    }
    // Abstract roles are not parseable.
    assert_eq!(Role::from_aria("widget"), None);
    assert_eq!(Role::from_aria("landmark"), None);
    assert_eq!(Role::from_aria("image"), Some(Role::Img));
    // Engine-only roles are not ARIA tokens and do not round-trip.
    assert_eq!(Role::from_aria("statictext"), None);
    assert_eq!(Role::from_aria("inlinetextbox"), None);
}
