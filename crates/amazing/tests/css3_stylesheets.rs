//! Exercise stylesheet collection through the actual driver binary.
use std::fs;
use std::path::PathBuf;
use std::process::Command;

struct Fixture(PathBuf);

impl Fixture {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("amazing-css3-{}-{name}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn write(&self, name: &str, source: &str) -> String {
        let path = self.0.join(name);
        fs::write(&path, source).unwrap();
        path.to_str().unwrap().to_owned()
    }

    fn run(&self, args: &[&str]) -> String {
        let output = Command::new(env!("CARGO_BIN_EXE_amazing"))
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }

    fn render(&self, name: &str, html: &str) -> Vec<u8> {
        let page = self.write(&format!("{name}.html"), html);
        let ppm = self.0.join(format!("{name}.ppm"));
        self.run(&["render-wpt", &page, ppm.to_str().unwrap(), "80", "60"]);
        let bytes = fs::read(ppm).unwrap();
        let header = b"P6\n80 60\n255\n";
        assert!(bytes.starts_with(header));
        assert_eq!(bytes.len(), header.len() + 80 * 60 * 3);
        bytes[header.len()..].to_vec()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn splitting_sheets_preserves_cascade_and_extra_file_order() {
    let f = Fixture::new("order");
    let first = "@layer early, late; @layer early {p {color:red!important}} \
        @layer late {p {color:blue!important}} p {--chosen:11px; width:var(--chosen)}";
    let second = "@layer early {p {color:green!important}} \
        p {--chosen:23px; height:17px; background-color:red} \
        p {background-color:blue}";
    let body = "<body><p>target</p>";
    let joined = f.write(
        "joined.html",
        &format!("<!doctype html><style>{first}\n{second}</style>{body}"),
    );
    let split = f.write(
        "split.html",
        &format!("<!doctype html><style>{first}</style><style>{second}</style>{body}"),
    );
    let extra1 = f.write("extra1.css", "p {height:31px}");
    let extra2 = f.write("extra2.css", "p {height:43px}");
    let before = f.run(&["style", &joined, &extra1, &extra2]);
    let after = f.run(&["style", &split, &extra1, &extra2]);
    let target = |text: &str| {
        text.lines()
            .find(|line| line.trim_start().starts_with("<p "))
            .unwrap()
            .trim()
            .to_owned()
    };
    assert_eq!(target(&before), target(&after));
    for expected in [
        "color:green",
        "width:23px",
        "height:43px",
        "background-color:blue",
    ] {
        assert!(target(&after).contains(expected), "{after}");
    }
}

#[test]
fn interleaved_external_and_inline_sheets_preserve_rendered_cascade() {
    let f = Fixture::new("linked-order");
    let first = "@layer a,b; body{margin:0} div{width:80px;height:60px} \
        @layer a {div{background-color:red!important}}";
    let external = "@layer b {div{background-color:blue!important}} \
        div{--size:70px;width:var(--size)}";
    let last = "@layer a {div{background-color:green!important}} div{--size:80px}";
    f.write("linked.css", external);
    let split = f.render(
        "split",
        &format!(
            "<!doctype html><style>{first}</style>\
        <link rel=stylesheet href=linked.css><style>{last}</style><body><div></div>"
        ),
    );
    let joined = f.render(
        "joined",
        &format!("<!doctype html><style>{first}\n{external}\n{last}</style><body><div></div>"),
    );
    assert_eq!(split, joined);
    assert!(split.chunks_exact(3).all(|pixel| pixel == [0, 128, 0]));
}

#[test]
fn implicit_scope_uses_each_style_or_link_owner_parent() {
    let f = Fixture::new("scope");
    f.write("scoped.css", "@scope { :scope {background-color:blue} }");
    let actual = f.render(
        "scoped",
        "<!doctype html><style>\
        body{margin:0} section{height:20px;background-color:red}\
        </style><body><section><style>@scope { :scope {background-color:green} }</style></section>\
        <section><link rel=stylesheet href=scoped.css></section><section></section>",
    );
    let expected = f.render(
        "reference",
        "<!doctype html><style>body{margin:0}\
        section{height:20px;background-color:red} #a{background-color:green}\
        #b{background-color:blue}</style><body><section id=a></section>\
        <section id=b></section><section></section>",
    );
    assert_eq!(actual, expected);
    for (y, color) in [(10, [0, 128, 0]), (30, [0, 0, 255]), (50, [255, 0, 0])] {
        assert_eq!(&actual[(y * 80 + 40) * 3..(y * 80 + 40) * 3 + 3], &color);
    }
}

#[test]
fn nested_implicit_scopes_do_not_escape_their_owners() {
    let f = Fixture::new("nested");
    let page = f.write(
        "page.html",
        "<!doctype html><body><section>\
        <style>@scope {p{color:green}}</style><p>outer</p>\
        <article><style>@scope {p{color:blue}}</style><p>inner</p></article>\
        </section><p style='color:red'>outside</p>",
    );
    let output = f.run(&["style", &page]);
    let paragraphs: Vec<_> = output
        .lines()
        .filter(|line| line.trim_start().starts_with("<p "))
        .collect();
    assert_eq!(paragraphs.len(), 3, "{output}");
    for (line, color) in paragraphs
        .iter()
        .zip(["color:green", "color:blue", "color:red"])
    {
        assert!(line.contains(color), "{output}");
    }
}

#[test]
fn layout_command_preserves_implicit_scope() {
    let f = Fixture::new("layout");
    let scoped = f.write(
        "scoped.html",
        "<!doctype html><body><div>\
        <style>@scope {:scope{width:37px;height:19px}}</style></div>",
    );
    let reference = f.write(
        "reference.html",
        "<!doctype html><body>\
        <div style='width:37px;height:19px'><style></style></div>",
    );
    assert_eq!(
        f.run(&["layout", &scoped, "80"]),
        f.run(&["layout", &reference, "80"])
    );
}

#[test]
fn unclosed_sheet_does_not_consume_the_next_sheet() {
    let f = Fixture::new("boundary");
    let page = f.write(
        "page.html",
        "<!doctype html><style>/* unclosed</style>\
        <style>p{color:green}</style><body><p>target</p>",
    );
    let output = f.run(&["style", &page]);
    assert!(
        output
            .lines()
            .any(|line| line.trim_start().starts_with("<p ") && line.contains("color:green")),
        "{output}"
    );
}
