use amazing_fetch::cookies::{CookieAccess, CookieSite};
use amazing_fetch::service::{CookiePersistence, HstsPersistence};
use amazing_fetch::HttpUrl;
use amazing_process::network_cookies::DiskCookies;
use amazing_process::network_hsts::DiskHsts;

#[test]
fn cookies_and_hsts_survive_network_profile_restart() {
    let dir = std::env::temp_dir().join(format!("amazing-net30-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let url = HttpUrl::parse("https://example.test/").unwrap();
    let access = CookieAccess {
        url: &url,
        site_for_cookies: CookieSite::Origin(url.origin().clone()),
        http_api: true,
        top_level_navigation: false,
        method: "GET",
        partition_key: None,
    };
    {
        let (mut cookies, cookie_disk) = DiskCookies::open(&dir, 100).unwrap();
        cookies
            .set_cookie(b"persistent=yes; Secure; Max-Age=100", &access, 100)
            .unwrap();
        cookies.set_cookie(b"session=no", &access, 100).unwrap();
        cookie_disk.save(&mut cookies, 100).unwrap();
        let (mut hsts, hsts_disk) = DiskHsts::open(&dir, 100).unwrap();
        assert!(hsts.observe(&url, "max-age=100; includeSubDomains", 100));
        hsts_disk.save(&hsts, 100).unwrap();
    }
    {
        let (mut cookies, _) = DiskCookies::open(&dir, 101).unwrap();
        assert_eq!(
            cookies.cookie_header(&access, 101).unwrap(),
            b"persistent=yes"
        );
        let (hsts, _) = DiskHsts::open(&dir, 101).unwrap();
        assert!(hsts.is_strict("sub.example.test", 101));
        assert!(!hsts.is_strict("sub.example.test", 200));
    }
    std::fs::remove_dir_all(dir).unwrap();
}
