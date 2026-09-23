//! HTML rendering hints. Attribute grammars are HTML microsyntaxes, not CSS.
use amazing_dom::{Dom, Namespace, NodeIdx};

fn attr<'a>(dom: &'a Dom, el: NodeIdx, name: &str) -> Option<&'a str> {
    dom.attrs_of(el)?
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, v)| v.as_str())
}
fn space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{c}')
}

/// HTML dimension parsing consumes a decimal prefix and only an immediately
/// following percent sign. Signs, exponents and CSS units are not parsed.
fn dimension(input: &str) -> Option<(f64, bool)> {
    let input = input.trim_start_matches(space);
    let bytes = input.as_bytes();
    if !bytes.first()?.is_ascii_digit() {
        return None;
    }
    let mut end = 0;
    while bytes.get(end).is_some_and(u8::is_ascii_digit) {
        end += 1;
    }
    if bytes.get(end) == Some(&b'.') {
        end += 1;
        while bytes.get(end).is_some_and(u8::is_ascii_digit) {
            end += 1;
        }
    }
    let number: f64 = input[..end].parse().ok()?;
    number
        .is_finite()
        .then_some((number, bytes.get(end) == Some(&b'%')))
}
fn integer(input: &str) -> Option<i64> {
    let input = input.trim_start_matches(space);
    let (negative, rest) = if let Some(s) = input.strip_prefix('-') {
        (true, s)
    } else {
        (false, input.strip_prefix('+').unwrap_or(input))
    };
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return None;
    }
    let n = digits.bytes().fold(0i64, |n, d| {
        n.saturating_mul(10).saturating_add((d - b'0') as i64)
    });
    Some(if negative { -n } else { n })
}
fn nonnegative(input: &str) -> Option<i64> {
    integer(input).filter(|n| *n >= 0)
}
fn length(input: &str, nonzero: bool) -> Option<String> {
    let (n, percent) = dimension(input)?;
    if nonzero && n == 0.0 {
        return None;
    }
    Some(format!("{n}{}", if percent { "%" } else { "px" }))
}

pub(super) fn collect(dom: &Dom, el: NodeIdx, body_link: Option<&str>) -> Vec<(String, String)> {
    let mut out = Vec::new();
    if dom.namespace(el) != Some(Namespace::Html) {
        return out;
    }
    let name = dom.name(el).unwrap_or("");
    let get = |key| attr(dom, el, key);
    let image_input =
        name == "input" && get("type").is_some_and(|s| s.eq_ignore_ascii_case("image"));
    let replaced = matches!(name, "img" | "embed" | "iframe" | "object" | "video") || image_input;
    let cell = matches!(name, "td" | "th");
    let table_group = matches!(name, "thead" | "tbody" | "tfoot" | "tr");
    let mut put = |property: &str, value: String| out.push((property.into(), value));
    if let Some(dir) =
        get("dir").filter(|s| s.eq_ignore_ascii_case("ltr") || s.eq_ignore_ascii_case("rtl"))
    {
        put("direction", dir.to_ascii_lowercase());
    }
    for property in ["width", "height"] {
        let applicable = replaced
            || cell
            || name == "table"
            || name == "marquee"
            || (property == "width" && matches!(name, "col" | "hr"))
            || (property == "height" && table_group);
        if applicable {
            if let Some(value) = get(property)
                .and_then(|s| length(s, cell || (name == "table" && property == "width")))
            {
                put(property, value);
            }
        }
    }
    if matches!(name, "img" | "video" | "canvas") || image_input {
        let parse = |s| {
            if name == "canvas" {
                nonnegative(s).map(|n| (n as f64, false))
            } else {
                dimension(s)
            }
        };
        if let (Some((w, false)), Some((h, false))) =
            (get("width").and_then(parse), get("height").and_then(parse))
        {
            put("aspect-ratio", format!("auto {w} / {h}"));
        }
    }
    if matches!(name, "embed" | "img" | "object" | "marquee") || image_input {
        for (attribute, sides) in [("hspace", ["left", "right"]), ("vspace", ["top", "bottom"])] {
            if let Some(value) = get(attribute).and_then(|s| length(s, false)) {
                for side in sides {
                    put(&format!("margin-{side}"), value.clone());
                }
            }
        }
    }
    if name == "iframe" && get("frameborder").is_some_and(|s| integer(s).unwrap_or(0) == 0) {
        put("border-width", "0px".into());
    }
    if matches!(name, "img" | "object" | "table") || image_input {
        if let Some(raw) = get("border") {
            let value = nonnegative(raw).or(if name == "table" { Some(1) } else { None });
            if let Some(n) = value.filter(|n| name == "table" || *n > 0) {
                put("border-width", format!("{n}px"));
                if name != "table" {
                    put("border-style", "solid".into());
                }
            }
        }
    }
    if name == "table" {
        if let Some(n) = get("cellspacing").and_then(nonnegative) {
            put("border-spacing", format!("{n}px"));
        }
    }
    if cell {
        let mut parent = dom.get(el).parent;
        while let Some(node) = parent {
            if dom.namespace(node) == Some(Namespace::Html) && dom.name(node) == Some("table") {
                if let Some(n) = attr(dom, node, "cellpadding").and_then(nonnegative) {
                    put("padding", format!("{n}px"));
                }
                break;
            }
            parent = dom.get(node).parent;
        }
    }
    if matches!(name, "div" | "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6") || cell || table_group
    {
        if let Some(value) = get("align") {
            let value = value.to_ascii_lowercase();
            let value = if (value == "middle" && (name == "div" || cell || table_group))
                || (value == "absmiddle" && (cell || table_group))
            {
                "center"
            } else {
                &value
            };
            if matches!(value, "left" | "right" | "center" | "justify") {
                put("text-align", value.into());
            }
        }
    }
    if name == "center" {
        put("text-align", "center".into());
    }
    if cell || table_group {
        if let Some(value) = get("valign") {
            let value = value.to_ascii_lowercase();
            if matches!(value.as_str(), "top" | "middle" | "bottom" | "baseline") {
                put("vertical-align", value);
            }
        }
    }
    if name == "pre" && get("wrap").is_some() {
        put("white-space", "pre-wrap".into());
    }
    if cell && get("nowrap").is_some() {
        put("white-space", "nowrap".into());
    }
    if name == "br" {
        if let Some(value) = get("clear") {
            let value = value.to_ascii_lowercase();
            let value = if value == "all" { "both" } else { &value };
            if matches!(value, "left" | "right" | "both") {
                put("clear", value.into());
            }
        }
    }
    if matches!(name, "ol" | "ul" | "li") {
        if let Some(value) = get("type") {
            let style = match value {
                "1" => Some("decimal"),
                "a" => Some("lower-alpha"),
                "A" => Some("upper-alpha"),
                "i" => Some("lower-roman"),
                "I" => Some("upper-roman"),
                _ => None,
            };
            if name != "ul" {
                if let Some(style) = style {
                    put("list-style-type", style.into());
                }
            }
            if name != "ol" {
                let value = value.to_ascii_lowercase();
                if matches!(value.as_str(), "none" | "disc" | "circle" | "square") {
                    put("list-style-type", value);
                }
            }
        }
    }
    if name == "li" {
        if let Some(n) = get("value").and_then(integer) {
            put("counter-set", format!("list-item {n}"));
        }
    }
    if name == "ol" {
        let reversed = get("reversed").is_some();
        if let Some(n) = get("start").and_then(integer) {
            put(
                "counter-reset",
                if reversed {
                    format!("reversed(list-item) {}", n.saturating_add(1))
                } else {
                    format!("list-item {}", n.saturating_sub(1))
                },
            );
        } else if reversed {
            put("counter-reset", "reversed(list-item)".into());
        }
    }
    if name == "font" {
        if let Some(face) = get("face") {
            put("font-family", face.into());
        }
        if let Some(size) = get("size") {
            if let Some(n) = integer(size) {
                let n = if size.trim_start_matches(space).starts_with(['+', '-']) {
                    n.saturating_add(3)
                } else {
                    n
                };
                put(
                    "font-size",
                    [
                        "x-small",
                        "small",
                        "medium",
                        "large",
                        "x-large",
                        "xx-large",
                        "xxx-large",
                    ][(n.clamp(1, 7) - 1) as usize]
                        .into(),
                );
            }
        }
    }
    if name == "body" {
        for (primary, secondary, sides) in [
            ("marginheight", "topmargin", ["top", "bottom"]),
            ("marginwidth", "leftmargin", ["left", "right"]),
        ] {
            if let Some(raw) = get(primary).or_else(|| get(secondary)) {
                let n = nonnegative(raw).unwrap_or(8);
                for side in sides {
                    put(&format!("margin-{side}"), format!("{n}px"));
                }
            }
        }
    }
    if name == "hr" {
        if let Some(n) = get("size").and_then(nonnegative) {
            if get("color").is_some() || get("noshade").is_some() {
                put("border-width", format!("{}px", n as f64 / 2.0));
            } else if n == 1 {
                put("border-bottom-width", "0px".into());
            } else if n > 1 {
                put("height", format!("{}px", n - 2));
            }
        }
    }
    if matches!(
        name,
        "body" | "table" | "thead" | "tbody" | "tfoot" | "tr" | "td" | "th" | "marquee"
    ) {
        if let Some(value) = get("bgcolor").and_then(legacy_color) {
            put("background-color", value);
        }
    }
    if matches!(name, "font" | "hr") {
        if let Some(value) = get("color").and_then(legacy_color) {
            put("color", value);
        }
    }
    if name == "body" {
        if let Some(value) = get("text").and_then(legacy_color) {
            put("color", value);
        }
    }
    if name == "table" {
        if let Some(value) = get("bordercolor").and_then(legacy_color) {
            put("border-color", value);
        }
    }
    if name == "textarea" && get("wrap").is_some_and(|s| s.eq_ignore_ascii_case("off")) {
        put("white-space", "pre".into());
    }
    if name == "table" || name == "hr" {
        if let Some(align) = get("align") {
            match align.to_ascii_lowercase().as_str() {
                "left" | "right" if name == "table" => put("float", align.to_ascii_lowercase()),
                "center" if name == "table" => {
                    put("margin-inline-start", "auto".into());
                    put("margin-inline-end", "auto".into());
                }
                "left" => {
                    put("margin-left", "0px".into());
                    put("margin-right", "auto".into());
                }
                "right" => {
                    put("margin-left", "auto".into());
                    put("margin-right", "0px".into());
                }
                "center" => {
                    put("margin-left", "auto".into());
                    put("margin-right", "auto".into());
                }
                _ => {}
            }
        }
    }
    if name == "caption" && get("align").is_some_and(|s| s.eq_ignore_ascii_case("bottom")) {
        put("caption-side", "bottom".into());
    }
    if matches!(name, "embed" | "iframe" | "img" | "object") || image_input {
        if let Some(align) = get("align") {
            match align.to_ascii_lowercase().as_str() {
                "left" | "right" => put("float", align.to_ascii_lowercase()),
                "top" | "baseline" | "bottom" => put("vertical-align", align.to_ascii_lowercase()),
                "texttop" => put("vertical-align", "text-top".into()),
                "absmiddle" | "abscenter" => put("vertical-align", "middle".into()),
                _ => {}
            }
        }
    }
    if name == "hr" && (get("color").is_some() || get("noshade").is_some()) {
        put("border-style", "solid".into());
    }
    if matches!(name, "a" | "area") && get("href").is_some() {
        if let Some(color) = body_link {
            put("color", color.into());
        }
    }
    if matches!(
        name,
        "body" | "table" | "thead" | "tbody" | "tfoot" | "tr" | "td" | "th"
    ) {
        if let Some(url) = get("background").filter(|s| !s.is_empty()) {
            // Remains document-relative, never relative to an author sheet.
            // The embedding resource loader owns document URL resolution.
            let mut escaped = String::new();
            for c in url.chars() {
                if c == '\0' || c.is_control() {
                    escaped.push_str(&format!("\\{:x} ", c as u32));
                } else {
                    if matches!(c, '\\' | '"') {
                        escaped.push('\\');
                    }
                    escaped.push(c);
                }
            }
            put("background-image", format!("url(\"{escaped}\")"));
        }
    }
    table_borders(dom, el, &mut out);
    out
}

/// HTML legacy colors deliberately accept inputs rejected by the CSS grammar.
fn legacy_color(input: &str) -> Option<String> {
    if input.is_empty() {
        return None;
    }
    let input = input.trim_matches(space).to_ascii_lowercase();
    if input == "transparent" {
        return None;
    }
    if crate::values::named_color_keyword(&input) {
        return Some(input);
    }
    if input.len() == 4
        && input.starts_with('#')
        && input[1..].bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Some(format!(
            "#{}{}{}{}{}{}",
            &input[1..2],
            &input[1..2],
            &input[2..3],
            &input[2..3],
            &input[3..4],
            &input[3..4]
        ));
    }
    let mut chars: Vec<char> = input
        .chars()
        .flat_map(|c| {
            if c as u32 > 0xffff {
                vec!['0', '0']
            } else {
                vec![c]
            }
        })
        .take(128)
        .collect();
    if chars.first() == Some(&'#') {
        chars.remove(0);
    }
    let mut hex: Vec<u8> = chars
        .into_iter()
        .map(|c| if c.is_ascii_hexdigit() { c as u8 } else { b'0' })
        .collect();
    while hex.is_empty() || hex.len() % 3 != 0 {
        hex.push(b'0');
    }
    let length = hex.len() / 3;
    let mut start = length.saturating_sub(8);
    while length - start > 2 && (0..3).all(|i| hex[i * length + start] == b'0') {
        start += 1;
    }
    let digits = (length - start).min(2);
    let rgb: Vec<u8> = (0..3)
        .map(|i| {
            u8::from_str_radix(
                std::str::from_utf8(&hex[i * length + start..i * length + start + digits]).unwrap(),
                16,
            )
            .unwrap()
        })
        .collect();
    Some(format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2]))
}

fn table_borders(dom: &Dom, el: NodeIdx, out: &mut Vec<(String, String)>) {
    let name = dom.name(el).unwrap_or("");
    if !matches!(
        name,
        "table" | "td" | "th" | "thead" | "tbody" | "tfoot" | "tr" | "colgroup"
    ) {
        return;
    }
    let mut table = Some(el);
    while let Some(node) = table {
        if dom.namespace(node) == Some(Namespace::Html) && dom.name(node) == Some("table") {
            break;
        }
        table = dom.get(node).parent;
    }
    let Some(table) = table else {
        return;
    };
    let rules = attr(dom, table, "rules").unwrap_or("").to_ascii_lowercase();
    let border = attr(dom, table, "border").is_some_and(|v| nonnegative(v).unwrap_or(1) != 0);
    let mut put = |p: &str, v: &str| out.push((p.into(), v.into()));
    if table == el {
        if matches!(rules.as_str(), "none" | "groups" | "rows" | "cols" | "all") {
            put("border-style", "hidden");
            put("border-collapse", "collapse");
        }
        if border {
            put("border-style", "outset");
        }
        if let Some(frame) = attr(dom, table, "frame") {
            let value = match frame.to_ascii_lowercase().as_str() {
                "void" => Some("hidden"),
                "above" => Some("outset hidden hidden hidden"),
                "below" => Some("hidden hidden outset hidden"),
                "hsides" => Some("outset hidden"),
                "lhs" => Some("hidden hidden hidden outset"),
                "rhs" => Some("hidden outset hidden hidden"),
                "vsides" => Some("hidden outset"),
                "box" | "border" => Some("outset"),
                _ => None,
            };
            if let Some(value) = value {
                put("border-style", value);
            }
        }
    } else if matches!(name, "td" | "th") {
        // Corresponding cells, never cells of a nested table.
        if border {
            put("border-width", "1px");
            put("border-style", "inset");
        }
        match rules.as_str() {
            "none" | "groups" | "rows" => {
                put("border-width", "1px");
                put("border-style", "none");
            }
            "cols" => {
                put("border-width", "1px");
                for edge in ["block-start", "block-end", "inline-start", "inline-end"] {
                    put(
                        &format!("border-{edge}-style"),
                        if edge.starts_with("block") {
                            "none"
                        } else {
                            "solid"
                        },
                    );
                }
            }
            "all" => {
                put("border-width", "1px");
                put("border-style", "solid");
            }
            _ => {}
        }
    } else {
        let axis = match (rules.as_str(), name) {
            ("groups", "colgroup") => Some("inline"),
            ("groups", "thead" | "tbody" | "tfoot") | ("rows", "tr") => Some("block"),
            _ => None,
        };
        if let Some(axis) = axis {
            for edge in ["start", "end"] {
                put(&format!("border-{axis}-{edge}-width"), "1px");
                put(&format!("border-{axis}-{edge}-style"), "solid");
            }
        }
    }
}

pub(super) fn body_link(dom: &Dom) -> Option<String> {
    let mut pending = vec![NodeIdx(0)];
    while let Some(node) = pending.pop() {
        if dom.namespace(node) == Some(Namespace::Html) && dom.name(node) == Some("body") {
            return attr(dom, node, "link").and_then(legacy_color);
        }
        let children: Vec<_> = dom.children(node).collect();
        pending.extend(children.into_iter().rev());
    }
    None
}
