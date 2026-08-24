use std::time::Duration;

#[test]
#[ignore = "requires external HTTPS access"]
fn google_https_returns_html() {
    let response = amazing_net::fetch(
        "https://google.com/",
        &amazing_net::FetchOptions {
            timeout: Duration::from_secs(10),
        },
    )
    .expect("Google HTTPS fetch");

    assert_eq!(response.status, 200);
    assert_eq!(response.content_type(), Some("text/html"));
    assert!(response.url.starts_with("https://"));
    assert!(!response.body.is_empty());
}
