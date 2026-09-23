//! CLI integration checks. Run on win-001; these are not upstream WPT credit.
use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicU32, Ordering},
    thread,
    time::{Duration, Instant},
};

static NEXT_DIRECTORY: AtomicU32 = AtomicU32::new(0);

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "amazing-xml-{}-{}",
            std::process::id(),
            NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn write(&self, name: &str, source: &str) -> PathBuf {
        let path = self.0.join(name);
        fs::write(&path, source).unwrap();
        path
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_amazing")).args(args).output().unwrap()
}
fn path(path: &Path) -> &str {
    path.to_str().unwrap()
}
fn check_exit(output: &Output, expected: i32) {
    assert_eq!(
        output.status.code(), Some(expected),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}
fn render(input: &Path, output: &Path) -> Output {
    run(&["render-wpt", path(input), path(output), "16", "16"])
}

const STATIC_XHTML: &str = r#"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head>
<style>html, body { margin: 0; } div { width: 16px; height: 16px; background: lime; }</style>
</head><body><div></div></body></html>"#;
const EXTERNAL_ENTITY: &str = r#"<!DOCTYPE root SYSTEM "https://example.invalid/unloaded.dtd">
<root>&external;</root>"#;

#[test]
fn xml_files_preserve_case_and_do_not_gain_an_html_document() {
    let fixture = Fixture::new();
    for extension in ["xml", "xht", "xhtml", "XHTML", "svg"] {
        let file = fixture.write(&format!("document.{extension}"), "<Root><Item/></Root>");
        let output = run(&["parse", path(&file)]);
        check_exit(&output, 0);
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(text.contains("<Root>"), "{text}");
        assert!(text.contains("<Item>"), "{text}");
        assert!(!text.contains("<html>"), "{text}");
    }
}

#[test]
fn valid_static_xhtml_renders_the_same_nonwhite_pixels_as_html() {
    let fixture = Fixture::new();
    let xml = fixture.write("page.xhtml", STATIC_XHTML);
    let html = fixture.write("page.html", STATIC_XHTML);
    let xml_output = fixture.path("xml.ppm");
    let html_output = fixture.path("html.ppm");
    check_exit(&render(&xml, &xml_output), 0);
    check_exit(&render(&html, &html_output), 0);
    let pixels = fs::read(xml_output).unwrap();
    assert_eq!(pixels, fs::read(html_output).unwrap());
    assert!(pixels.ends_with(&[0, 255, 0]), "fixture must paint lime, not an inert white pass");
}

#[test]
fn malformed_xml_is_fatal_in_every_file_document_command_without_html_recovery() {
    let fixture = Fixture::new();
    let source = "<Root><Item></Root>";
    let xml = fixture.write("malformed.xml", source);
    for command in ["parse", "style", "layout"] {
        let output = run(&[command, path(&xml)]);
        check_exit(&output, 2);
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8_lossy(&output.stderr).contains("XML Malformed"));
    }
    let bmp = fixture.path("bad.bmp");
    check_exit(&run(&["render", path(&xml), path(&bmp), "16"]), 2);
    assert!(!bmp.exists());
    let ppm = fixture.path("bad.ppm");
    check_exit(&render(&xml, &ppm), 2);
    assert!(!ppm.exists());
    let html = fixture.write("recovery.html", source);
    check_exit(&run(&["parse", path(&html)]), 0);
    check_exit(&render(&html, &fixture.path("html.ppm")), 0);
}

#[test]
fn unloaded_external_dtd_entities_have_a_distinct_unsupported_exit() {
    let fixture = Fixture::new();
    let xml = fixture.write("external.xml", EXTERNAL_ENTITY);
    let parsed = run(&["parse", path(&xml)]);
    check_exit(&parsed, 3);
    assert!(String::from_utf8_lossy(&parsed.stderr).contains("XML Unsupported"));
    let output = fixture.path("unsupported.ppm");
    check_exit(&render(&xml, &output), 3);
    assert!(!output.exists());
}

#[test]
fn xml_scripts_handlers_and_domrun_are_not_executed_while_html_scripts_still_run() {
    let fixture = Fixture::new();
    let source = r##"<html xmlns="http://www.w3.org/1999/xhtml"><head>
<style>html, body { margin: 0; }</style></head><body>
<div id="target" style="width:16px;height:16px;background:red"></div>
<script>document.querySelector("#target").setAttribute("style", "width:16px;height:16px;background:lime");</script>
</body></html>"##;
    let xml = fixture.write("script.xhtml", source);
    let xml_output = fixture.path("xml-script.ppm");
    let result = render(&xml, &xml_output);
    check_exit(&result, 3);
    assert!(String::from_utf8_lossy(&result.stderr).contains("XML script"));
    assert!(!xml_output.exists());
    let handler = fixture.write("handler.xhtml", r#"<html xmlns="http://www.w3.org/1999/xhtml"><body onload="document.write('executed')"></body></html>"#);
    check_exit(&render(&handler, &fixture.path("handler.ppm")), 3);
    let script = fixture.write("must-not-run.js", "throw 'XML_SCRIPT_EXECUTED';");
    let result = run(&["domrun", path(&xml), path(&script)]);
    check_exit(&result, 3);
    assert!(!String::from_utf8_lossy(&result.stderr).contains("XML_SCRIPT_EXECUTED"));
    let html = fixture.write("script.html", source);
    let html_output = fixture.path("html-script.ppm");
    check_exit(&render(&html, &html_output), 0);
    assert!(fs::read(html_output).unwrap().ends_with(&[0, 255, 0]));
}

#[test]
fn html_script_failures_remain_renderer_failures_not_xml_errors() {
    let fixture = Fixture::new();
    let html = fixture.write("throw.html", "<script>throw 7;</script>");
    let output = render(&html, &fixture.path("throw.ppm"));
    check_exit(&output, 1);
    assert!(String::from_utf8_lossy(&output.stderr).contains("page script failed"));
}

fn fetch_parse(mime: &str, source: &str, request_path: &str) -> Output {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{source}",
        source.len(),
    );
    let server = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                    let mut request = [0; 4096];
                    let _ = stream.read(&mut request).unwrap();
                    stream.write_all(response.as_bytes()).unwrap();
                    return;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < deadline, "driver did not connect");
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("{error}"),
            }
        }
    });
    let output = run(&["fetch-parse", &format!("http://{address}{request_path}")]);
    server.join().unwrap();
    output
}

#[test]
fn http_xml_mime_types_dispatch_without_using_url_extensions() {
    for mime in ["application/xml", "text/xml", "APPLICATION/XHTML+XML; charset=UTF-8", "image/svg+xml"] {
        let output = fetch_parse(mime, "<Root><Item/></Root>", "/no-extension");
        check_exit(&output, 0);
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(text.contains("<Root>"), "{mime}: {text}");
        assert!(!text.contains("<html>"), "{mime}: {text}");
    }
    let html = fetch_parse("text/html", "<Root><Item/></Root>", "/document.xhtml");
    check_exit(&html, 0);
    assert!(String::from_utf8_lossy(&html.stdout).contains("<html>"));
    let malformed = fetch_parse("application/xml", "<Root><Item></Root>", "/document.html");
    check_exit(&malformed, 2);
    assert!(malformed.stdout.is_empty());
    check_exit(&fetch_parse("application/xml", EXTERNAL_ENTITY, "/document.html"), 3);
}
