//! Driver adapter regression tests.
use crate::{dom_host::BrowserDomHost, page::load_page_text};
use amazing_text::TextSystem;

#[test]
fn document_title_ignores_svg_title_elements() {
    let mut host = BrowserDomHost {
        dom: amazing_html::parse("<svg><title>foreign</title></svg>"),
    };
    assert_eq!(
        <BrowserDomHost as amazing_js::DomHost>::get_document_title(&host),
        ""
    );
    <BrowserDomHost as amazing_js::DomHost>::set_document_title(&mut host, "html title")
        .expect("set title");
    assert_eq!(
        <BrowserDomHost as amazing_js::DomHost>::get_document_title(&host),
        "html title"
    );
}

#[test]
fn malformed_page_font_warns_without_advancing_font_generation() {
    let directory = std::env::temp_dir().join(format!(
        "amazing-text-provider-test-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let font_path = directory.join("broken.woff2");
    std::fs::write(&font_path, [0u8; 64]).unwrap();
    let page_path = directory.join("page.html");
    let page = page_path.to_string_lossy();
    let baseline_generation = load_page_text("", &page).provider.generation();
    let text = load_page_text(
        "@font-face { font-family: Broken; src: url('broken.woff2') format('woff2') }",
        &page,
    );

    assert_eq!(text.provider.generation(), baseline_generation);
    assert_eq!(text.warnings.len(), 1);
    assert!(text.warnings[0].contains("broken.woff2"));
    let _ = std::fs::remove_dir_all(directory);
}
