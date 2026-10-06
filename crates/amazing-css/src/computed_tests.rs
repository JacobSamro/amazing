use super::*;

#[test]
fn typed_attributes_substitute_after_variables_and_validate_casts() {
    let attrs = vec![("data-WIDTH".to_owned(), "200px".to_owned()),
        ("data-bad".to_owned(), "red".to_owned())];
    let style = resolve_with_root_and_attrs(None, cascaded(&[
        ("--size", "attr(DATA-width type(<length>))"),
        ("width", "var(--size)"),
        ("height", "attr(data-bad type(<length>), 40px)"),
        ("min-width", "attr(data-bad type(<length>))"),
    ]), 16.0, Some(&attrs));
    assert_eq!(style.get("width"), Some("200px"));
    assert_eq!(style.get("height"), Some("40px"));
    assert_eq!(style.get("min-width"), Some("auto"));
}

#[test]
fn untyped_attribute_fallback_requires_a_string() {
    let empty = resolve_with_root_and_attrs(None,
        cascaded(&[("content", "attr(missing, invalid)")]), 16.0, Some(&[]));
    assert_eq!(empty.get("content"), Some("\"\""));
    let fallback = resolve_with_root_and_attrs(None,
        cascaded(&[("content", "attr(missing, 'fallback')")]), 16.0, Some(&[]));
    assert_eq!(fallback.get("content"), Some("\"fallback\""));
}

#[test]
fn deprecated_system_colors_resolve_before_paint_handoff() {
    let style = resolve(None, cascaded(&[
        ("background-color", "ButtonShadow"),
        ("border-top-color", "ThreeDShadow"),
        ("color", "silver"),
    ]));
    assert_eq!(style.get("background-color"), Some("#f0f0f0ff"));
    assert_eq!(style.get("border-top-color"), Some("#808080ff"));
    assert_eq!(style.get("color"), Some("silver"));
}

fn cascaded(values: &[(&str, &str)]) -> BTreeMap<String, String> {
    values
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[test]
fn case_sensitive_custom_properties_inherit_and_substitute() {
    let parent = resolve(None, cascaded(&[("--A", "green"), ("--a", "crimson")]));
    let child = resolve(
        Some(&parent),
        cascaded(&[("color", "var(--A)"), ("background-color", "var(--a)")]),
    );
    assert_eq!(child.get("color"), Some("green"));
    assert_eq!(child.get("background-color"), Some("crimson"));
    assert_eq!(child.custom_serialized("--A"), Some(Some("green".into())));
    assert_eq!(child.custom_serialized("--a"), Some(Some("crimson".into())));
}

#[test]
fn nested_fallback_and_fallback_commas_survive_substitution() {
    let style = resolve(
        None,
        cascaded(&[
            ("color", "var(--missing,var(--also-missing,green))"),
            ("font-family", "var(--family,serif,sans-serif)"),
        ]),
    );
    assert_eq!(style.get("color"), Some("green"));
    assert_eq!(style.get("font-family"), Some("serif, sans-serif"));
}

#[test]
fn cycle_members_are_invalid_before_consumer_fallback_runs() {
    let style = resolve(
        None,
        cascaded(&[
            ("--a", "var(--b)"),
            ("--b", "var(--a,red)"),
            ("color", "var(--a,green)"),
        ]),
    );
    assert_eq!(style.custom_serialized("--a"), Some(None));
    assert_eq!(style.custom_serialized("--b"), Some(None));
    assert_eq!(style.get("color"), Some("green"));
}

#[test]
fn missing_var_is_invalid_at_computed_value_time_not_previous_cascade_value() {
    let style = resolve(None, cascaded(&[("color", "var(--missing)")]));
    assert_eq!(style.get("color"), Some("black"));
}

#[test]
fn css_wide_keywords_use_property_metadata() {
    let parent = resolve(None, cascaded(&[("color", "blue"), ("display", "block")]));
    let child = resolve(
        Some(&parent),
        cascaded(&[
            ("color", "unset"),
            ("display", "inherit"),
            ("background-color", "initial"),
        ]),
    );
    assert_eq!(child.get("color"), Some("blue"));
    assert_eq!(child.get("display"), Some("block"));
    assert_eq!(child.get("background-color"), Some("transparent"));
}

#[test]
fn all_expands_supported_properties_but_excludes_direction_and_unicode_bidi() {
    let expanded = expand_declaration("all", "initial");
    assert!(expanded.iter().any(|(name, _)| name == "color"));
    assert!(expanded.iter().any(|(name, _)| name == "margin-left"));
    assert!(!expanded.iter().any(|(name, _)| name == "direction"));
    assert!(!expanded.iter().any(|(name, _)| name == "unicode-bidi"));
}

#[test]
fn variable_grammar_distinguishes_nested_from_top_level_remnants() {
    assert!(valid_custom_property_value(&tokenize("var(--a,(;))")));
    assert!(valid_custom_property_value(&tokenize("var(--a,)")));
    assert!(!valid_custom_property_value(&tokenize("var(--a,;)")));
    assert!(!valid_custom_property_value(&tokenize("var(--a,!)")));
}

#[test]
fn substitution_preserves_token_boundaries() {
    let style = resolve(None, cascaded(&[("--n", "1"), ("width", "var(--n)px")]));
    assert_eq!(style.get("width"), Some("auto"));
    assert_eq!(
        style.typed("width"),
        Some(&ComputedValue::Keyword(values::Keyword::Auto))
    );
    assert_eq!(
        serialize_computed(&[Token::Number(1.0), Token::Ident("px".into())]),
        "1/**/px"
    );
}

#[test]
fn public_set_uses_the_same_shorthand_expansion_seam() {
    let mut style = ComputedStyle::default();
    style.set("gap", "10px 20px");
    style.set("grid-column", "1 / -1");
    assert_eq!(style.get("row-gap"), Some("10px"));
    assert_eq!(style.get("column-gap"), Some("20px"));
    assert_eq!(style.get("grid-column-start"), Some("1"));
    assert_eq!(style.get("grid-column-end"), Some("-1"));
    assert_eq!(style.get("gap"), None);
    assert_eq!(style.get("grid-column"), None);
}

#[test]
fn grid_lines_validate_both_longhands_and_shorthand_halves() {
    for property in ["grid-column-start", "grid-column-end", "grid-row-start", "grid-row-end"] {
        for invalid in ["default", "0", "span 0", "span -1", "span", "auto foo", "1 2"] {
            assert!(expand_declaration(property, invalid).is_empty(), "{property}: {invalid}");
        }
        for valid in ["auto", "foo", "-1 foo", "span foo 2", "foo span", "span 2"] {
            assert_eq!(expand_declaration(property, valid), vec![(property.to_owned(), valid.to_owned())]);
        }
    }
    for property in ["grid-column", "grid-row"] {
        for invalid in ["default", "1 / default", "default / 1", "span 0", "0 / 1", "1 / 0", "1 / 2 / 3"] {
            assert!(expand_declaration(property, invalid).is_empty(), "{property}: {invalid}");
        }
        assert_eq!(expand_declaration(property, "-1 foo / span foo 2"), vec![
            (format!("{property}-start"), "-1 foo".into()),
            (format!("{property}-end"), "span foo 2".into()),
        ]);
        assert_eq!(expand_declaration(property, "foo"), vec![
            (format!("{property}-start"), "foo".into()),
            (format!("{property}-end"), "foo".into()),
        ]);
        assert_eq!(expand_declaration(property, "span foo 2"), vec![
            (format!("{property}-start"), "span foo 2".into()),
            (format!("{property}-end"), "auto".into()),
        ]);
        assert_eq!(expand_declaration(property, "var(--line)").len(), 2);
    }
    let declarations = [("grid-column", "default"), ("grid-row-start", "default")];
    let style = resolve(None, declarations.into_iter()
        .flat_map(|(property, value)| expand_declaration(property, value))
        .collect());
    assert_eq!(style.get("grid-column-start"), Some("auto"));
    assert_eq!(style.get("grid-column-end"), Some("auto"));
    assert_eq!(style.get("grid-row-start"), Some("auto"));
}

#[test]
fn flex_flow_rejects_unknown_and_duplicate_components() {
    assert_eq!(
        expand_declaration("flex-flow", "wrap column"),
        vec![
            ("flex-direction".into(), "column".into()),
            ("flex-wrap".into(), "wrap".into()),
        ]
    );
    assert_eq!(
        expand_declaration("flex-flow", "row column"),
        Vec::<(String, String)>::new()
    );
    assert_eq!(
        expand_declaration("flex-flow", "row banana"),
        Vec::<(String, String)>::new()
    );
}

#[test]
fn grid_template_only_splits_on_a_top_level_solidus() {
    // CSS-28: `calc()` is now a valid grid track breadth (a `/` division
    // inside it must not be mistaken for `grid-template`'s own top-level
    // rows/columns separator), so the rows side below is valid on its
    // own; the columns side's unrecognized `banana` keyword is what
    // keeps the whole declaration invalid, still exercising the
    // nested-solidus case the test name promises.
    assert!(
        expand_declaration(
            "grid-template",
            "minmax(10px,calc(20px / 2)) / repeat(2,banana)"
        )
        .is_empty()
    );
    assert_eq!(
        expand_declaration(
            "grid-template",
            "minmax(10px,calc(20px / 2)) / repeat(2,1fr)"
        ),
        vec![
            ("grid-template-rows".to_owned(), "minmax(10px, calc(20px / 2))".to_owned()),
            ("grid-template-columns".to_owned(), "repeat(2, 1fr)".to_owned()),
            ("grid-template-areas".to_owned(), "none".to_owned()),
        ]
    );
    assert_eq!(
        expand_declaration("grid-template", "10px / 20px / 30px"),
        Vec::<(String, String)>::new()
    );
    assert!(expand_declaration("grid-template", "banana / banana").is_empty());
    assert!(expand_declaration("grid-template", "fit-content(auto) / 1fr").is_empty());
    assert!(expand_declaration("grid-template", "[span] 10px / 1fr").is_empty());
    assert!(expand_declaration("grid-template", "repeat(auto-fill,1fr) / 30px").is_empty());
    assert!(
        expand_declaration(
            "grid-template",
            "repeat(auto-fill,10px) repeat(auto-fit,20px) / 30px"
        )
        .is_empty()
    );
}

// CSS-28: `place-content`/`place-items`/`place-self` had no shorthand
// expansion at all before this ticket -- not even `PROPERTIES` metadata
// -- so a declaration like `place-items: center` was silently inert.
#[test]
fn place_shorthands_expand_to_their_align_and_justify_longhands() {
    assert_eq!(
        expand_declaration("place-items", "center"),
        vec![("align-items".to_owned(), "center".to_owned()),
             ("justify-items".to_owned(), "center".to_owned())]
    );
    assert_eq!(
        expand_declaration("place-content", "center start"),
        vec![("align-content".to_owned(), "center".to_owned()),
             ("justify-content".to_owned(), "start".to_owned())]
    );
    assert_eq!(
        expand_declaration("place-self", "stretch"),
        vec![("align-self".to_owned(), "stretch".to_owned()),
             ("justify-self".to_owned(), "stretch".to_owned())]
    );
    // Compound two-word units (`first baseline`/`last baseline`/`safe
    // <position>`/`unsafe <position>`) are kept as one unit, not split.
    assert_eq!(
        expand_declaration("place-items", "first baseline"),
        vec![("align-items".to_owned(), "first baseline".to_owned()),
             ("justify-items".to_owned(), "first baseline".to_owned())]
    );
    assert_eq!(
        expand_declaration("place-content", "safe start end"),
        vec![("align-content".to_owned(), "safe start".to_owned()),
             ("justify-content".to_owned(), "end".to_owned())]
    );
    assert_eq!(
        expand_declaration("place-self", "last baseline unsafe self-end"),
        vec![("align-self".to_owned(), "last baseline".to_owned()),
             ("justify-self".to_owned(), "unsafe self-end".to_owned())]
    );
    // CSS-wide keywords propagate to both longhands unresolved.
    for kw in ["inherit", "initial", "unset", "revert"] {
        assert_eq!(
            expand_declaration("place-items", kw),
            vec![("align-items".to_owned(), kw.to_owned()),
                 ("justify-items".to_owned(), kw.to_owned())]
        );
    }
    // var() defers both longhands rather than expanding eagerly, and
    // once substitution has run, `resolve_deferred_shorthand` re-splits
    // the substituted text for each specific longhand.
    let deferred = expand_declaration("place-content", "var(--x)");
    assert_eq!(deferred.len(), 2);
    assert_eq!(deferred[0].0, "align-content");
    assert_eq!(deferred[1].0, "justify-content");
    assert_eq!(
        resolve_deferred_shorthand("align-content", "amazing-internal-place-content-align(center start)"),
        Ok("center".to_owned())
    );
    assert_eq!(
        resolve_deferred_shorthand("justify-content", "amazing-internal-place-content-justify(center start)"),
        Ok("start".to_owned())
    );
    // Invalid/ambiguous forms drop the whole declaration.
    for (property, value) in [
        ("place-items", "center center center"),
        ("place-items", "banana"),
        ("place-content", "safe"),
        ("place-self", "safe banana"),
    ] {
        assert!(expand_declaration(property, value).is_empty(), "{property}: {value}");
    }
}

#[test]
fn paint_properties_use_validated_computed_value_grammars() {
    for property in [
        "background-clip",
        "background-origin",
        "background-attachment",
        "background-repeat",
        "background-size",
        "background-position",
        "background-position-x",
        "background-position-y",
        "box-shadow",
        "transform",
        "transform-style",
        "border-top-left-radius",
    ] {
        assert!(
            is_supported_property(property),
            "missing metadata for {property}"
        );
    }
    // CSS-30: background-clip's box+text combination and comma list
    // (previously rejected by a single-keyword-only validator, which
    // silently dropped the declaration back to its initial value).
    assert_eq!(
        expand_declaration("background-clip", "text border-area"),
        vec![("background-clip".to_owned(), "border-area text".to_owned())]
    );
    assert_eq!(
        expand_declaration("background-clip", "border-box, padding-box"),
        vec![(
            "background-clip".to_owned(),
            "border-box, padding-box".to_owned()
        )]
    );
    assert!(expand_declaration("background-clip", "fill-box").is_empty());
    assert!(expand_declaration("background-attachment", "auto").is_empty());
    assert!(expand_declaration("background-repeat", "repeat repeat-x").is_empty());
    assert_eq!(
        expand_declaration("background-repeat", "repeat repeat"),
        vec![("background-repeat".to_owned(), "repeat".to_owned())]
    );
    assert_eq!(
        expand_declaration("background-size", "1px"),
        vec![("background-size".to_owned(), "1px auto".to_owned())]
    );
    assert_eq!(
        expand_declaration("background-size", "auto auto"),
        vec![("background-size".to_owned(), "auto".to_owned())]
    );
    assert!(expand_declaration("background-size", "-1px").is_empty());
    assert!(expand_declaration("background-size", "1px 2px 3px").is_empty());
    // CSS-30: background-position's edge+offset resolution (CSS
    // Backgrounds 3 SS3.9), including the right/bottom-anchored
    // `calc(100% - <length>)` complement form.
    assert_eq!(
        expand_declaration("background-position", "top 17px right -18px"),
        vec![(
            "background-position".to_owned(),
            "calc(100% + 18px) 17px".to_owned()
        )]
    );
    assert_eq!(
        expand_declaration("background-position", "top left"),
        vec![("background-position".to_owned(), "0% 0%".to_owned())]
    );
    assert!(expand_declaration("background-position", "left right").is_empty());
    assert_eq!(
        expand_declaration("background-position-x", "right -10px"),
        vec![(
            "background-position-x".to_owned(),
            "calc(100% + 10px)".to_owned()
        )]
    );
    assert!(expand_declaration("background-position-x", "top").is_empty());
    assert!(expand_declaration("background-position-y", "left").is_empty());
    assert!(expand_declaration("transform", "translate(10px,20px)").len() == 1);
    assert!(expand_declaration("transform", "translate(10px,)").is_empty());
    assert!(expand_declaration("transform", "scale(1,2,3)").is_empty());
    assert!(expand_declaration("box-shadow", "5px 5px black").len() == 1);
    assert!(expand_declaration("box-shadow", "5px 5px banana").is_empty());
    assert!(expand_declaration("box-shadow", "5px 5px rgb(banana)").is_empty());
    assert!(expand_declaration("box-shadow", "inset inset 5px 5px black").is_empty());
    assert!(expand_declaration("box-shadow", "5px 5px red blue").is_empty());
    let radius = expand_declaration("border-radius", "10px 20% / 5px");
    assert_eq!(radius.len(), 4);
    assert_eq!(
        radius[0],
        ("border-top-left-radius".into(), "10px 5px".into())
    );
    assert_eq!(
        radius[1],
        ("border-top-right-radius".into(), "20% 5px".into())
    );
    assert!(expand_declaration("opacity", "banana").is_empty());
    assert_eq!(
        expand_declaration("z-index", "-2"),
        vec![("z-index".to_owned(), "-2".to_owned())]
    );
    assert!(expand_declaration("z-index", "2.5").is_empty());
    assert!(expand_declaration("z-index", "banana").is_empty());
    assert!(expand_declaration("overflow-x", "banana").is_empty());
}

#[test]
fn font_shorthand_expands_size_line_height_family_and_resets_optionals() {
    let expanded =
        expand_declaration("font", "italic 700 120px / 2 \"Arabic Test\", sans-serif");
    let values: BTreeMap<_, _> = expanded.into_iter().collect();

    assert_eq!(values.get("font-size").map(String::as_str), Some("120px"));
    assert_eq!(values.get("line-height").map(String::as_str), Some("2"));
    assert_eq!(
        values.get("font-family").map(String::as_str),
        Some("\"Arabic Test\", sans-serif")
    );
    assert_eq!(values.get("font-style").map(String::as_str), Some("italic"));
    assert_eq!(
        values.get("font-variant").map(String::as_str),
        Some("normal")
    );
    assert_eq!(values.get("font-weight").map(String::as_str), Some("700"));

    for invalid in ["italic oblique 16px serif", "16px \"Quoted\" adjacent", "normal normal normal normal italic 16px serif"] {
        assert!(expand_declaration("font", invalid).is_empty());
    }
}
