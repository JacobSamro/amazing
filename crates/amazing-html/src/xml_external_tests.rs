use super::*;
use crate::xml_resources::*;
use std::collections::BTreeMap;
struct Inputs {
    bodies: BTreeMap<String, Vec<u8>>,
    reads: Vec<(String, String, XmlExternalKind)>,
}
impl Inputs {
    fn new(rows: &[(&str, &str)]) -> Self {
        Self {
            bodies: rows
                .iter()
                .map(|(name, text)| (name.to_string(), text.as_bytes().to_vec()))
                .collect(),
            reads: Vec::new(),
        }
    }
}
impl XmlExternalResolver for Inputs {
    fn resolve(
        &mut self,
        request: XmlExternalRequest<'_>,
    ) -> Result<Option<XmlExternalResource>, XmlError> {
        self.reads.push((
            request.declaration_source.base().into(),
            request.system_id.into(),
            request.kind,
        ));
        Ok(self
            .bodies
            .get(request.system_id)
            .map(|bytes| XmlExternalResource {
                source: XmlEntitySource::new(
                    format!("memory:{}", request.system_id),
                    format!("memory:{}", request.system_id),
                )
                .unwrap(),
                bytes: bytes.clone(),
                encoding_authority: XmlExternalEncoding::Entity,
            }))
    }
}
fn parse(src: &str, inputs: &mut Inputs) -> Result<Dom, XmlError> {
    parse_xml_decoded_with_resolver(
        src,
        &XmlEntitySource::new("memory:doc".into(), "memory:doc".into()).unwrap(),
        inputs,
    )
}
fn text(dom: &Dom) -> String {
    (0..dom.node_count())
        .filter_map(|i| match &dom.get(NodeIdx(i as u32)).kind {
            NodeKind::Text { data } | NodeKind::CDATASection { data } => Some(data.as_str()),
            _ => None,
        })
        .collect()
}
fn root(dom: &Dom) -> NodeIdx {
    (0..dom.node_count())
        .map(|i| NodeIdx(i as u32))
        .find(|i| dom.name(*i).is_some())
        .unwrap()
}
#[test]
fn actual_external_subset_declarations_defaults_and_tokenized_normalization() {
    let mut inputs = Inputs::new(&[(
        "dtd",
        "<!ENTITY e 'value'><!ATTLIST r a NMTOKENS '  a  b  '> ",
    )]);
    let dom = parse("<!DOCTYPE r SYSTEM 'dtd'><r>&e;</r>", &mut inputs).unwrap();
    assert_eq!(text(&dom), "value");
    assert_eq!(dom.get_attribute_ns(root(&dom), None, "a"), Some("a b"));
    assert_eq!(
        inputs.reads,
        [("memory:doc".into(), "dtd".into(), XmlExternalKind::Subset)]
    );
}
#[test]
fn internal_first_binding_and_all_duplicate_declarations_are_parsed() {
    let mut inputs = Inputs::new(&[(
        "dtd",
        "<!ENTITY e 'external'><!ATTLIST r a CDATA 'external'>",
    )]);
    let dom = parse("<!DOCTYPE r SYSTEM 'dtd' [<!ENTITY e 'internal'><!ATTLIST r a CDATA 'internal'>]><r>&e;</r>", &mut inputs).unwrap();
    assert_eq!(text(&dom), "internal");
    assert_eq!(
        dom.get_attribute_ns(root(&dom), None, "a"),
        Some("internal")
    );
    let mut bad = Inputs::new(&[("dtd", "<!ENTITY e 'external'><!ENTITY e >")]);
    assert_eq!(
        parse(
            "<!DOCTYPE r SYSTEM 'dtd' [<!ENTITY e 'internal'>]><r/>",
            &mut bad
        )
        .unwrap_err()
        .kind,
        XmlErrorKind::Malformed
    );
}
#[test]
fn complete_include_ignore_nesting_and_parameter_keyword_matrix() {
    for body in [
        "<![INCLUDE[<!ENTITY e 'yes'>]]>",
        "<!ENTITY % flag 'INCLUDE'><![%flag;[<!ENTITY e 'yes'>]]>",
        "<![IGNORE[invalid %missing; <![INCLUDE[ nonsense ]]> ]]><![INCLUDE[<!ENTITY e 'yes'>]]>",
        "<!ENTITY % inner '<![INCLUDE[<!ENTITY e \"yes\">]]>'>%inner;",
    ] {
        let mut inputs = Inputs::new(&[("dtd", body)]);
        assert_eq!(
            text(&parse("<!DOCTYPE r SYSTEM 'dtd'><r>&e;</r>", &mut inputs).unwrap()),
            "yes",
            "{body}"
        );
    }
    for body in [
        "<![include[]]>",
        "<![IGNORE[",
        "<![INCLUDE[<!ENTITY e 'x'>]>",
        "<![IGNORE[ <![]]> ",
        "<![INCLUDE[<r/>]]>",
    ] {
        assert!(
            parse(
                "<!DOCTYPE r SYSTEM 'dtd'><r/>",
                &mut Inputs::new(&[("dtd", body)])
            )
            .is_err(),
            "{body}"
        );
    }
    assert!(parse("<!DOCTYPE r [<![IGNORE[]]>]><r/>", &mut Inputs::new(&[])).is_err());
}
#[test]
fn external_parameter_entity_in_internal_subset_uses_external_grammar() {
    let mut inputs = Inputs::new(&[("parameter", "<![INCLUDE[<!ENTITY e 'good'>]]>")]);
    let dom = parse(
        "<!DOCTYPE r [<!ENTITY % p SYSTEM 'parameter'>%p;]><r>&e;</r>",
        &mut inputs,
    )
    .unwrap();
    assert_eq!(text(&dom), "good");
    assert_eq!(inputs.reads[0].2, XmlExternalKind::ParameterEntity);
}
#[test]
fn parameter_in_external_markup_models_and_token_space_boundaries() {
    let mut inputs = Inputs::new(&[(
        "dtd",
        "<!ENTITY % model '(a,b?)'><!ELEMENT r %model;><!ELEMENT a EMPTY><!ELEMENT b EMPTY>",
    )]);
    parse("<!DOCTYPE r SYSTEM 'dtd'><r><a/><b/></r>", &mut inputs).unwrap();
    let mut bad = Inputs::new(&[("dtd", "<!ENTITY % word 'MP'><!ELEMENT r E%word;TY>")]);
    assert!(parse("<!DOCTYPE r SYSTEM 'dtd'><r/>", &mut bad).is_err());
}
#[test]
fn literal_parameter_quotes_do_not_close_entity_values_and_general_refs_bypass() {
    let mut inputs = Inputs::new(&[(
        "dtd",
        "<!ENTITY % quoted '\"Yes\"'><!ENTITY other 'tail'><!ENTITY e \"He said %quoted;. &other;\">",
    )]);
    assert_eq!(
        text(&parse("<!DOCTYPE r SYSTEM 'dtd'><r>&e;</r>", &mut inputs).unwrap()),
        "He said \"Yes\". tail"
    );
    assert!(
        parse(
            "<!DOCTYPE r [<!ENTITY % p 'x'><!ENTITY e '%p;'>]><r/>",
            &mut Inputs::new(&[])
        )
        .is_err()
    );
}
#[test]
fn percent_character_reference_activates_only_in_the_correct_inclusion_phase() {
    let mut inputs = Inputs::new(&[(
        "dtd",
        "<!ENTITY % decl '<!ENTITY e \"ok\">'><!ENTITY % ref '&#37;decl;'>%ref;",
    )]);
    assert_eq!(
        text(&parse("<!DOCTYPE r SYSTEM 'dtd'><r>&e;</r>", &mut inputs).unwrap()),
        "ok"
    );
}
#[test]
fn declaration_separator_parameter_must_be_a_complete_external_subset_declaration() {
    for body in [
        "<!ENTITY",
        "<!ENTITY e 'x'",
        "<![INCLUDE[<!ENTITY e 'x'>",
        "text",
        "<!ENTITY e 'x'> junk",
    ] {
        let mut inputs = Inputs::new(&[("p", body)]);
        assert!(
            parse(
                "<!DOCTYPE r [<!ENTITY % p SYSTEM 'p'>%p;]><r/>",
                &mut inputs
            )
            .is_err(),
            "{body}"
        );
    }
}
#[test]
fn unread_parameter_cutoff_preserves_first_bindings_and_standalone_distinction() {
    let source = "<!DOCTYPE r [<!ENTITY first 'yes'><!ENTITY % p SYSTEM 'missing'>%p;<!ENTITY later 'no'><!ATTLIST r a CDATA 'no'>]><r>&first;</r>";
    let dom = parse(source, &mut Inputs::new(&[])).unwrap();
    assert_eq!(text(&dom), "yes");
    assert_eq!(dom.get_attribute_ns(root(&dom), None, "a"), None);
    let mut inputs = Inputs::new(&[]);
    let dom = parse(
        &format!("<?xml version='1.0' standalone='yes'?>{source}"),
        &mut inputs,
    )
    .unwrap();
    assert_eq!(dom.get_attribute_ns(root(&dom), None, "a"), Some("no"));
}
#[test]
fn real_external_general_content_imports_under_the_actual_namespace_context() {
    let mut inputs = Inputs::new(&[(
        "piece",
        "<?xml encoding='UTF-8'?><p:child xmlns:p='urn:x'>ok</p:child>",
    )]);
    let dom = parse(
        "<!DOCTYPE r [<!ENTITY e SYSTEM 'piece'>]><r>&e;</r>",
        &mut inputs,
    )
    .unwrap();
    assert_eq!(text(&dom), "ok");
    assert!(
        (0..dom.node_count())
            .map(|i| NodeIdx(i as u32))
            .any(|n| dom.namespace_uri(n) == Some("urn:x"))
    );
    assert_eq!(inputs.reads.len(), 1);
    assert_eq!(inputs.reads[0].2, XmlExternalKind::GeneralEntity);
}
#[test]
fn forbidden_attributes_unparsed_entities_and_literal_regions_never_fetch_general_bytes() {
    for source in [
        "<!DOCTYPE r [<!ENTITY e SYSTEM 'piece'>]><r a='&e;'/>",
        "<!DOCTYPE r [<!ENTITY e SYSTEM 'piece'>]><r><![CDATA[&e;]]><!--&e;--><?pi &e;?></r>",
        "<!DOCTYPE r [<!ENTITY e SYSTEM 'piece' NDATA gif><!NOTATION gif SYSTEM 'x'>]><r>&e;</r>",
    ] {
        let mut inputs = Inputs::new(&[("piece", "<x/>")]);
        let result = parse(source, &mut inputs);
        assert!(inputs.reads.is_empty());
        if source.contains("a='") || source.contains("NDATA") {
            assert!(result.is_err());
        } else {
            assert!(result.is_ok());
        }
    }
}
#[test]
fn recursion_and_graph_depth_are_actual_entity_errors_not_empty_replacement() {
    let mut inputs = Inputs::new(&[("dtd", "<!ENTITY % p '%p;'>%p;")]);
    assert!(parse("<!DOCTYPE r SYSTEM 'dtd'><r/>", &mut inputs).is_err());
    let mut inputs = Inputs::new(&[("a", "&b;"), ("b", "&a;")]);
    assert!(
        parse(
            "<!DOCTYPE r [<!ENTITY a SYSTEM 'a'><!ENTITY b SYSTEM 'b'>]><r>&a;</r>",
            &mut inputs
        )
        .is_err()
    );
    let mut inputs = Inputs::new(&[("p", "<!ENTITY % nested SYSTEM 'p'>%nested;")]);
    assert!(
        parse(
            "<!DOCTYPE r [<!ENTITY % p SYSTEM 'p'>%p;]><r/>",
            &mut inputs
        )
        .is_err()
    );
}
#[test]
fn independent_declaration_bases_survive_parameter_and_general_requests() {
    let mut inputs = Inputs::new(&[
        ("dir/dtd", "<!ENTITY % p SYSTEM 'child'>%p;"),
        ("child", "<!ENTITY e SYSTEM 'text'>"),
        ("text", "data"),
    ]);
    assert_eq!(
        text(&parse("<!DOCTYPE r SYSTEM 'dir/dtd'><r>&e;</r>", &mut inputs).unwrap()),
        "data"
    );
    assert_eq!(inputs.reads[1].0, "memory:dir/dtd");
    assert_eq!(inputs.reads[2].0, "memory:child");
}
#[test]
fn complete_external_encoding_and_mime_authority_matrix() {
    use amazing_encoding::Encoding;
    for encoding in [Encoding::Utf8, Encoding::Utf16Le, Encoding::Utf16Be] {
        let label = encoding.name();
        let body = format!("<?xml encoding='{label}'?><x>é</x>");
        let marker: &[u8] = match encoding {
            Encoding::Utf8 => &[0xef, 0xbb, 0xbf],
            Encoding::Utf16Le => &[0xff, 0xfe],
            _ => &[0xfe, 0xff],
        };
        let mut bytes = marker.to_vec();
        bytes.extend(amazing_encoding::encode(&body, encoding, true).unwrap());
        let decoded =
            crate::xml_encoding::decode_external_entity(&bytes, &XmlExternalEncoding::Entity)
                .unwrap();
        assert_eq!(decoded.text, "<x>é</x>");
        let wrong = "<?xml encoding='windows-1252'?><x/>";
        let mut bytes = marker.to_vec();
        bytes.extend(amazing_encoding::encode(wrong, encoding, true).unwrap());
        assert!(
            crate::xml_encoding::decode_external_entity(&bytes, &XmlExternalEncoding::Entity)
                .is_err()
        );
        assert!(
            crate::xml_encoding::decode_external_entity(
                &bytes,
                &XmlExternalEncoding::Mime { charset: None }
            )
            .is_ok()
        );
    }
    for bytes in [
        b"<?xml version='1.0'?><x/>".as_slice(),
        b"<?xml encoding='unknown'?><x/>",
        b"\xff\xfe\0<\0",
        b"\0\0\0<",
    ] {
        assert!(
            crate::xml_encoding::decode_external_entity(bytes, &XmlExternalEncoding::Entity)
                .is_err()
        );
    }
}
#[test]
fn malformed_resource_input_and_text_version_do_not_yield_partial_documents() {
    for body in [
        "<?xml encoding='utf-8' standalone='no'?><!ENTITY e 'x'>",
        "<?xml version='1.1' encoding='utf-8'?><!ENTITY e 'x'>",
        "<!ENTITY e 'x'>\0",
        "<!ENTITY e 'x'",
        "<?xml encoding='utf-8'?><r/>",
    ] {
        assert!(
            parse(
                "<!DOCTYPE r SYSTEM 'dtd'><r/>",
                &mut Inputs::new(&[("dtd", body)])
            )
            .is_err(),
            "{body}"
        );
    }
    let mut inputs = Inputs::new(&[("piece", "<open>")]);
    assert!(
        parse(
            "<!DOCTYPE r [<!ENTITY e SYSTEM 'piece'>]><r>&e;</r>",
            &mut inputs
        )
        .is_err()
    );
}
#[test]
fn original_no_resolver_api_keeps_external_reads_explicitly_unavailable() {
    assert_eq!(
        parse_xml("<!DOCTYPE r [<!ENTITY e SYSTEM 'never'>]><r>&e;</r>")
            .unwrap_err()
            .kind,
        XmlErrorKind::Unsupported
    );
    parse_xml("<!DOCTYPE r SYSTEM 'never'><r/>").unwrap();
}
struct Files {
    root: std::path::PathBuf,
}
impl Files {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("amazing-xml-resources-{}-{n}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        Self { root }
    }
    fn write(&self, path: &str, bytes: &[u8]) -> std::path::PathBuf {
        let path = self.root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, bytes).unwrap();
        path
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).unwrap();
    }
}
#[test]
fn actual_file_provider_resolves_nested_bases_and_entity_text_without_fixture_handlers() {
    let files = Files::new();
    let doc = files.write("doc.xml", b"<!DOCTYPE r SYSTEM 'dir/main.dtd'><r>&e;</r>");
    files.write("dir/main.dtd", b"<!ENTITY % p SYSTEM 'sub/decl.dtd'>%p;");
    files.write("dir/sub/decl.dtd", b"<!ENTITY e SYSTEM '../text.xml'>");
    files.write("dir/text.xml", b"<?xml encoding='utf-8'?>real");
    let mut resolver = XmlFileResolver::new(&files.root).unwrap();
    let accepted = resolver.read_document(&doc).unwrap();
    let decoded = crate::xml_encoding::decode_entity(&accepted.bytes).unwrap();
    assert_eq!(
        text(
            &parse_xml_decoded_with_resolver(&decoded.text, &accepted.source, &mut resolver)
                .unwrap()
        ),
        "real"
    );
}
#[test]
fn actual_file_policy_handles_escapes_local_uris_and_rejects_other_authority() {
    let files = Files::new();
    let doc = files.write("doc.xml", b"<r/>");
    let target = files.write("with space.txt", b"payload");
    let mut resolver = XmlFileResolver::new(&files.root).unwrap();
    let source = resolver.read_document(&doc).unwrap().source;
    for name in [
        "with%20space.txt".into(),
        format!("FILE://localhost{}", target.display()),
        "with%20space.txt?query".into(),
    ] {
        let accepted = resolver
            .resolve(XmlExternalRequest {
                declaration_source: &source,
                system_id: &name,
                public_id: None,
                kind: XmlExternalKind::GeneralEntity,
            })
            .unwrap()
            .unwrap();
        assert_eq!(accepted.bytes, b"payload");
    }
    for name in [
        "http://example.invalid/never",
        "file://other/never",
        "missing",
        "../outside",
    ] {
        assert!(
            resolver
                .resolve(XmlExternalRequest {
                    declaration_source: &source,
                    system_id: name,
                    public_id: None,
                    kind: XmlExternalKind::Subset
                })
                .unwrap()
                .is_none()
        );
    }
    for name in ["bad%escape", "doc.xml#fragment"] {
        assert!(
            resolver
                .resolve(XmlExternalRequest {
                    declaration_source: &source,
                    system_id: name,
                    public_id: None,
                    kind: XmlExternalKind::Subset
                })
                .is_err()
        );
    }
}

#[test]
fn resource_limits_and_error_cleanup_remain_explicit_application_results() {
    let files = Files::new();
    let doc = files.write("doc.xml", b"<r/>");
    let oversized = files.write("large.xml", &vec![b' '; 16 * 1024 * 1024 + 1]);
    let mut resolver = XmlFileResolver::new(&files.root).unwrap();
    assert_eq!(
        resolver.read_document(&oversized).err().unwrap().kind,
        XmlErrorKind::Limit
    );
    let source = resolver.read_document(&doc).unwrap().source;
    assert!(
        resolver
            .resolve(XmlExternalRequest {
                declaration_source: &source,
                system_id: "unread.dtd",
                public_id: None,
                kind: XmlExternalKind::Subset
            })
            .unwrap()
            .is_none()
    );
    let mut inputs = Inputs::new(&[("p", "<![IGNORE[unterminated")]);
    assert!(
        parse(
            "<!DOCTYPE r [<!ENTITY % p SYSTEM 'p'>%p;]><r/>",
            &mut inputs
        )
        .is_err()
    );
    inputs
        .bodies
        .insert("p".into(), b"<!ENTITY e 'recovered'>".to_vec());
    assert_eq!(
        text(
            &parse(
                "<!DOCTYPE r [<!ENTITY % p SYSTEM 'p'>%p;]><r>&e;</r>",
                &mut inputs
            )
            .unwrap()
        ),
        "recovered"
    );
}

#[test]
fn unread_unused_and_literal_regions_do_not_invent_resource_admission() {
    let mut inputs = Inputs::new(&[(
        "dtd",
        "<!ENTITY unused SYSTEM 'never'><!--%never;--><?pi %never;?><![IGNORE[%never;]]><!ENTITY e 'ok'>",
    )]);
    assert_eq!(
        text(&parse("<!DOCTYPE r SYSTEM 'dtd'><r>&e;</r>", &mut inputs).unwrap()),
        "ok"
    );
    assert_eq!(inputs.reads.len(), 1);
    parse(
        "<!DOCTYPE r SYSTEM 'unavailable'><r/>",
        &mut Inputs::new(&[]),
    )
    .unwrap();
}
