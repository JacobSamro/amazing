//! Native policy expectations from the complete pinned tentative/autoupgrades directory.
use amazing_fetch::{Destination, FetchError, HttpUrl, Request, RequestMode};

#[test]
fn all_autoupgrades_media_and_cors_variants() {
    // 2 origin variants each for audio/image/video, plus 3 CORS variants.
    let source = HttpUrl::parse("https://www.web-platform.test/").unwrap();
    let cases = [
        (
            "audio-upgrade.https.sub.html",
            Destination::Audio,
            RequestMode::NoCors,
            2,
        ),
        (
            "image-upgrade.https.sub.html",
            Destination::Image,
            RequestMode::NoCors,
            2,
        ),
        (
            "video-upgrade.https.sub.html",
            Destination::Video,
            RequestMode::NoCors,
            2,
        ),
        (
            "mixed-content-cors.https.sub.html",
            Destination::Audio,
            RequestMode::Cors,
            1,
        ),
        (
            "mixed-content-cors.https.sub.html",
            Destination::Image,
            RequestMode::Cors,
            1,
        ),
        (
            "mixed-content-cors.https.sub.html",
            Destination::Video,
            RequestMode::Cors,
            1,
        ),
    ];
    let mut count = 0;
    for (file, destination, mode, origins) in cases {
        for host in ["www.web-platform.test", "www.not-web-platform.test"]
            .into_iter()
            .take(origins)
        {
            let target =
                HttpUrl::parse(&format!("http://{host}:8443/mixed-content/resource")).unwrap();
            let mut request = Request::new(target, source.origin().clone());
            request.mode = mode;
            request.security.client_restricts_mixed_content = true;
            request.security.destination = destination;
            request
                .prepare()
                .unwrap_or_else(|error| panic!("{file}: {error}"));
            assert_eq!(request.url().as_parsed().scheme(), "https", "{file}");
            assert_eq!(request.url().as_parsed().port(), Some(8443), "{file}");
            count += 1;
        }
    }
    assert_eq!(count, 9);
}

#[test]
fn public_ip_images_block_instead_of_upgrade() {
    let source = HttpUrl::parse("https://www.web-platform.test/").unwrap();
    let target = HttpUrl::parse("http://192.0.2.1/img.png").unwrap();
    let mut request = Request::new(target, source.origin().clone());
    request.mode = RequestMode::NoCors;
    request.security.client_restricts_mixed_content = true;
    request.security.destination = Destination::Image;
    assert_eq!(request.prepare().unwrap_err(), FetchError::MixedContent);
}
