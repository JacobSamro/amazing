//! Driver byte-boundary checks. Run on win-001; these are not upstream WPT.
use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::PathBuf,
    process::{Command, Output},
    sync::atomic::{AtomicU32, Ordering},
    thread,
    time::{Duration, Instant},
};

static NEXT_FIXTURE: AtomicU32 = AtomicU32::new(0);

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "amazing-html2-{}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn parse(&self, name: &str, bytes: &[u8]) -> Output {
        let path = self.0.join(name);
        fs::write(&path, bytes).unwrap();
        Command::new(env!("CARGO_BIN_EXE_amazing"))
            .args(["parse", path.to_str().unwrap()])
            .output()
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn success_text(output: Output) -> String {
    assert_eq!(output.status.code(), Some(0), "{}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn local_html_bytes_obey_bom_meta_fallback_and_replacement() {
    let fixture = Fixture::new();
    let mut bom = vec![0xef, 0xbb, 0xbf];
    bom.extend_from_slice("<meta charset=windows-1252><p>é</p>".as_bytes());
    let tree = success_text(fixture.parse("bom.html", &bom));
    assert!(tree.contains("\"é\""), "{tree}");
    assert!(!tree.contains('\u{feff}'), "{tree}");

    let mut meta = b"<meta charset=windows-1252><p>".to_vec();
    meta.extend_from_slice(&[0x93, b'x', 0x94]);
    meta.extend_from_slice(b"</p>");
    assert!(success_text(fixture.parse("meta.html", &meta)).contains("\"“x”\""));
    assert!(success_text(fixture.parse("fallback.html", b"<p>\x80</p>")).contains("\"€\""));
    assert!(success_text(fixture.parse("replacement.html", b"<meta charset=utf-8><p>A\xffB</p>"))
        .contains("\"A�B\""));
}

#[test]
fn local_utf16_html_and_xml_mode_remain_distinct() {
    let fixture = Fixture::new();
    for (name, bom, little_endian) in [
        ("le.html", [0xff, 0xfe], true),
        ("be.html", [0xfe, 0xff], false),
    ] {
        let mut bytes = bom.to_vec();
        for code_unit in "<p>雪</p>".encode_utf16() {
            bytes.extend_from_slice(&if little_endian {
                code_unit.to_le_bytes()
            } else {
                code_unit.to_be_bytes()
            });
        }
        assert!(success_text(fixture.parse(name, &bytes)).contains("\"雪\""));
    }
    let xml = success_text(fixture.parse("page.xhtml", b"<Root><Item/></Root>"));
    assert!(xml.contains("<Root>"));
    assert!(!xml.contains("<html>"));
    let malformed = fixture.parse("bad.xhtml", b"<Root><Item></Root>");
    assert_eq!(malformed.status.code(), Some(2));
    let undecoded = success_text(fixture.parse("undecoded.xml", b"<Root>\x93</Root>"));
    assert!(undecoded.contains("\"�\""));
}

fn fetch_parse(content_type: &str, body: &[u8]) -> Output {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let response = [
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .into_bytes(),
        body.to_vec(),
    ]
    .concat();
    let server = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < deadline, "driver did not connect");
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("{error}"),
            }
        };
        let mut request = [0; 4096];
        let _ = stream.read(&mut request).unwrap();
        stream.write_all(&response).unwrap();
    });
    let output = Command::new(env!("CARGO_BIN_EXE_amazing"))
        .args(["fetch-parse", &format!("http://{address}/document.xhtml")])
        .output()
        .unwrap();
    server.join().unwrap();
    output
}

#[test]
fn http_charset_uses_raw_header_while_xml_media_type_still_selects_xml() {
    let mut body = b"<meta charset=utf-8><p>".to_vec();
    body.push(0x93);
    body.extend_from_slice(b"</p>");
    let html = success_text(fetch_parse("text/html; charset=\"windows-1252\"", &body));
    assert!(html.contains("\"“\""), "{html}");
    let xml = fetch_parse("application/xhtml+xml; charset=windows-1252", b"<Root><Item></Root>");
    assert_eq!(xml.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&xml.stderr).contains("XML Malformed"));
}
