//! Driver DOM adapter shared by the CLI, WPT runner, and fallback shell.
use amazing_js::DomHost;
use crate::cmd::js::resolve_script_url;

pub(crate) struct BrowserDomHost {
    pub(crate) dom: amazing_dom::Dom,
    pub(crate) native_forwarded: bool,
}

impl BrowserDomHost {
    fn serialize_node(&self, node: amazing_dom::NodeIdx, out: &mut String) {
        match &self.dom.get(node).kind {
            amazing_dom::NodeKind::DocumentFragment => {
                for child in self.dom.children(node) { self.serialize_node(child, out); }
            }
            amazing_dom::NodeKind::DocumentType { name, .. } => {
                out.push_str("<!DOCTYPE "); out.push_str(name); out.push('>');
            }
            amazing_dom::NodeKind::Document => {
                for child in self.dom.children(node) {
                    self.serialize_node(child, out);
                }
            }
            amazing_dom::NodeKind::Element { name, attrs, .. } => {
                out.push('<');
                out.push_str(name);
                for (key, value) in attrs {
                    out.push(' ');
                    out.push_str(key);
                    out.push_str("=\"");
                    out.push_str(value);
                    out.push('"');
                }
                out.push('>');
                for child in self.dom.children(node) {
                    self.serialize_node(child, out);
                }
                out.push_str("</");
                out.push_str(name);
                out.push('>');
            }
            amazing_dom::NodeKind::Text { data } => out.push_str(data),
            amazing_dom::NodeKind::CDATASection { data } => {
                out.push_str("<![CDATA["); out.push_str(data); out.push_str("]]>");
            }
            amazing_dom::NodeKind::Attr { .. } => {},
            amazing_dom::NodeKind::Comment { data } => {
                out.push_str("<!--");
                out.push_str(data);
                out.push_str("-->");
            }
            amazing_dom::NodeKind::ProcessingInstruction { target, data } => {
                out.push_str("<?");
                out.push_str(target);
                out.push(' ');
                out.push_str(data);
                out.push_str("?>");
            }
        }
    }

    fn find_named_in(
        dom: &amazing_dom::Dom,
        node: amazing_dom::NodeIdx,
        name: &str,
    ) -> Option<amazing_dom::NodeIdx> {
        if dom.namespace(node) == Some(amazing_dom::Namespace::Html) && dom.name(node) == Some(name)
        {
            return Some(node);
        }
        for child in dom.children(node) {
            if let Some(found) = Self::find_named_in(dom, child, name) {
                return Some(found);
            }
        }
        None
    }

    fn clone_from(
        &mut self,
        source: &amazing_dom::Dom,
        source_node: amazing_dom::NodeIdx,
        parent: amazing_dom::NodeIdx,
    ) {
        let kind = source.get(source_node).kind.clone();
        let created = match kind {
            amazing_dom::NodeKind::DocumentFragment => {
                for child in source.children(source_node) { self.clone_from(source, child, parent); }
                return;
            }
            amazing_dom::NodeKind::DocumentType { name, public_id, system_id } => self.dom.append_child(
                parent, amazing_dom::NodeKind::DocumentType { name, public_id, system_id },
            ),
            amazing_dom::NodeKind::Element {
                name,
                attrs,
                namespace,
            } => self.dom.append_child(
                parent,
                amazing_dom::NodeKind::Element {
                    name,
                    attrs,
                    namespace,
                },
            ),
            amazing_dom::NodeKind::Text { data } => self.dom.create_text(parent, &data),
            amazing_dom::NodeKind::CDATASection { data } => self.dom.append_child(
                parent, amazing_dom::NodeKind::CDATASection { data },
            ),
            amazing_dom::NodeKind::Attr { .. } => return,
            amazing_dom::NodeKind::Comment { data } => self
                .dom
                .append_child(parent, amazing_dom::NodeKind::Comment { data }),
            amazing_dom::NodeKind::ProcessingInstruction { target, data } => self.dom.append_child(
                parent,
                amazing_dom::NodeKind::ProcessingInstruction { target, data },
            ),
            amazing_dom::NodeKind::Document => return,
        };
        let children: Vec<_> = source.children(source_node).collect();
        for child in children {
            self.clone_from(source, child, created);
        }
    }

    fn text_content(&self, node: amazing_dom::NodeIdx, out: &mut String) {
        if let amazing_dom::NodeKind::Text { data } = &self.dom.get(node).kind {
            out.push_str(data);
        }
        for child in self.dom.children(node) {
            self.text_content(child, out);
        }
    }

    fn find(&self, node: amazing_dom::NodeIdx, selector: &str) -> Option<u32> {
        let selectors = Self::parse_selector_list(selector)?;
        self.find_any(node, &selectors)
    }

    fn parse_selector_list(selector: &str) -> Option<Vec<amazing_css::selectors::Selector>> {
        let mut parts = Vec::new();
        let mut start = 0usize;
        let mut parens = 0usize;
        let mut brackets = 0usize;
        let mut quote = None;
        let mut escaped = false;
        for (index, ch) in selector.char_indices() {
            if escaped {
                escaped = false;
                continue;
            }
            if ch == '\\' && quote.is_some() {
                escaped = true;
                continue;
            }
            if let Some(active) = quote {
                if ch == active {
                    quote = None;
                }
                continue;
            }
            match ch {
                '\'' | '"' => quote = Some(ch),
                '(' => parens += 1,
                ')' if parens > 0 => parens -= 1,
                '[' => brackets += 1,
                ']' if brackets > 0 => brackets -= 1,
                ',' if parens == 0 && brackets == 0 => {
                    parts.push(selector[start..index].trim());
                    start = index + ch.len_utf8();
                }
                _ => {}
            }
        }
        parts.push(selector[start..].trim());
        if parts.iter().any(|part| part.is_empty()) {
            return None;
        }
        parts
            .into_iter()
            .map(amazing_css::selectors::parse_selector)
            .collect()
    }

    fn find_any(
        &self,
        node: amazing_dom::NodeIdx,
        selectors: &[amazing_css::selectors::Selector],
    ) -> Option<u32> {
        if selectors
            .iter()
            .any(|selector| amazing_css::selectors::matches(&self.dom, node, selector))
        {
            return Some(node.0);
        }
        for child in self.dom.children(node) {
            if let Some(found) = self.find_any(child, selectors) {
                return Some(found);
            }
        }
        None
    }

    fn find_all(&self, node: amazing_dom::NodeIdx, selector: &str, out: &mut Vec<u32>) {
        let Some(selectors) = Self::parse_selector_list(selector) else {
            return;
        };
        self.find_all_parsed(node, &selectors, out);
    }

    fn find_all_parsed(
        &self,
        node: amazing_dom::NodeIdx,
        selectors: &[amazing_css::selectors::Selector],
        out: &mut Vec<u32>,
    ) {
        if selectors
            .iter()
            .any(|selector| amazing_css::selectors::matches(&self.dom, node, selector))
        {
            out.push(node.0);
        }
        for child in self.dom.children(node) {
            self.find_all_parsed(child, selectors, out);
        }
    }

    fn css_property_name(name: &str) -> String {
        let mut out = String::new();
        for ch in name.chars() {
            if ch.is_ascii_uppercase() {
                out.push('-');
                out.push(ch.to_ascii_lowercase());
            } else {
                out.push(ch);
            }
        }
        out
    }

    fn style_declarations(&self, node: amazing_dom::NodeIdx) -> Vec<(String, String)> {
        let raw = self
            .dom
            .attrs_of(node)
            .and_then(|attrs| attrs.iter().find(|(key, _)| key == "style"))
            .map(|(_, value)| value.as_str())
            .unwrap_or("");
        raw.split(';')
            .filter_map(|declaration| {
                let (name, value) = declaration.split_once(':')?;
                let name = name.trim().to_ascii_lowercase();
                if name.is_empty() {
                    return None;
                }
                Some((name, value.trim().to_string()))
            })
            .collect()
    }

    fn write_style_declarations(
        &mut self,
        node: amazing_dom::NodeIdx,
        declarations: &[(String, String)],
    ) -> Result<(), String> {
        let serialized = declarations
            .iter()
            .filter(|(_, value)| !value.is_empty())
            .map(|(name, value)| format!("{name}: {value};"))
            .collect::<Vec<_>>()
            .join(" ");
        self.set_attribute(node.0, "style", &serialized)
    }
}

impl amazing_js::DomHost for BrowserDomHost {
    fn query_selector(&mut self, selector: &str) -> Option<u32> {
        self.find(amazing_dom::NodeIdx(0), selector)
    }

    fn query_selector_all(&mut self, selector: &str) -> Vec<u32> {
        let mut nodes = Vec::new();
        self.find_all(amazing_dom::NodeIdx(0), selector, &mut nodes);
        nodes
    }

    fn matches_selector(&self, node: u32, selector: &str) -> bool {
        let Some(selectors) = Self::parse_selector_list(selector) else {
            return false;
        };
        selectors.iter().any(|selector| {
            amazing_css::selectors::matches(&self.dom, amazing_dom::NodeIdx(node), selector)
        })
    }

    fn parent_node(&self, node: u32) -> Option<u32> {
        if node as usize >= self.dom.node_count() {
            return None;
        }
        self.dom
            .get(amazing_dom::NodeIdx(node))
            .parent
            .map(|parent| parent.0)
    }

    fn get_text_content(&self, node: u32) -> Option<String> {
        let idx = amazing_dom::NodeIdx(node);
        if idx.0 as usize >= self.dom.node_count() {
            return None;
        }
        let mut text = String::new();
        self.text_content(idx, &mut text);
        Some(text)
    }

    fn set_text_content(&mut self, node: u32, text: &str) -> Result<(), String> {
        let idx = amazing_dom::NodeIdx(node);
        if idx.0 as usize >= self.dom.node_count() || self.dom.name(idx).is_none() {
            return Err("textContent requires an element".into());
        }
        let children: Vec<_> = self.dom.children(idx).collect();
        for child in children {
            self.dom.detach(child);
        }
        self.dom.create_text(idx, text);
        Ok(())
    }

    fn get_document_title(&self) -> String {
        let Some(title) = Self::find_named_in(&self.dom, amazing_dom::NodeIdx(0), "title") else {
            return String::new();
        };
        let mut text = String::new();
        self.text_content(title, &mut text);
        text
    }

    fn set_document_title(&mut self, title: &str) -> Result<(), String> {
        if let Some(node) = Self::find_named_in(&self.dom, amazing_dom::NodeIdx(0), "title") {
            return self.set_text_content(node.0, title);
        }
        let head = Self::find_named_in(&self.dom, amazing_dom::NodeIdx(0), "head")
            .ok_or_else(|| "document has no head element".to_string())?;
        let title_node = self.dom.create_element(head, "title");
        self.dom.create_text(title_node, title);
        Ok(())
    }

    fn fetch(&mut self, base_url: &str, input: &str) -> Result<amazing_js::HostResponse, String> {
        let target = if base_url.is_empty() {
            input.to_owned()
        } else {
            resolve_script_url(base_url, input)
        };
        let response = amazing_net::fetch(&target, &amazing_net::FetchOptions::default())
            .map_err(|error| error.to_string())?;
        let body = response
            .text()
            .ok_or_else(|| "fetch response was not text".to_string())?;
        Ok(amazing_js::HostResponse {
            url: response.url,
            status: response.status,
            body,
            headers: response
                .headers
                .iter()
                .map(|(name, value)| (name.to_string(), value.to_string()))
                .collect(),
        })
    }

    fn get_inner_html(&self, node: u32) -> Option<String> {
        if node as usize >= self.dom.node_count()
            || self.dom.name(amazing_dom::NodeIdx(node)).is_none()
        {
            return None;
        }
        let mut output = String::new();
        for child in self.dom.children(amazing_dom::NodeIdx(node)) {
            self.serialize_node(child, &mut output);
        }
        Some(output)
    }

    fn set_inner_html(&mut self, node: u32, html: &str) -> Result<(), String> {
        let target = amazing_dom::NodeIdx(node);
        if node as usize >= self.dom.node_count() || self.dom.name(target).is_none() {
            return Err("innerHTML requires an element".into());
        }
        let parsed = amazing_html::parse(html);
        let fragment_root = Self::find_named_in(&parsed, amazing_dom::NodeIdx(0), "body")
            .unwrap_or(amazing_dom::NodeIdx(0));
        let children: Vec<_> = self.dom.children(target).collect();
        for child in children {
            self.dom.detach(child);
        }
        let source_children: Vec<_> = parsed.children(fragment_root).collect();
        for child in source_children {
            self.clone_from(&parsed, child, target);
        }
        Ok(())
    }

    fn get_attribute(&self, node: u32, name: &str) -> Option<Option<String>> {
        if node as usize >= self.dom.node_count() {
            return None;
        }
        let attrs = self.dom.attrs_of(amazing_dom::NodeIdx(node))?;
        Some(
            attrs
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.clone()),
        )
    }

    fn set_attribute(&mut self, node: u32, name: &str, value: &str) -> Result<(), String> {
        let idx = amazing_dom::NodeIdx(node);
        if node as usize >= self.dom.node_count() {
            return Err("unknown DOM node".into());
        }
        self.dom
            .set_attribute(idx, name, value)
            .map_err(|error| error.to_string())
    }

    fn remove_attribute(&mut self, node: u32, name: &str) -> Result<(), String> {
        let idx = amazing_dom::NodeIdx(node);
        if node as usize >= self.dom.node_count() {
            return Err("unknown DOM node".into());
        }
        let mut attrs = self
            .dom
            .attrs_of(idx)
            .ok_or_else(|| "attribute requires an element".to_string())?
            .to_vec();
        attrs.retain(|(key, _)| key != name);
        self.dom.set_attrs(idx, attrs);
        Ok(())
    }

    fn create_element(&mut self, name: &str) -> Result<u32, String> {
        Ok(self
            .dom
            .alloc_unattached(amazing_dom::NodeKind::Element {
                name: name.to_ascii_lowercase(),
                attrs: Vec::new(),
                namespace: amazing_dom::Namespace::Html,
            })
            .0)
    }

    fn create_text_node(&mut self, text: &str) -> Result<u32, String> {
        Ok(self
            .dom
            .alloc_unattached(amazing_dom::NodeKind::Text {
                data: text.to_owned(),
            })
            .0)
    }

    fn append_child(&mut self, parent: u32, child: u32) -> Result<(), String> {
        let parent = amazing_dom::NodeIdx(parent);
        let child = amazing_dom::NodeIdx(child);
        if parent.0 as usize >= self.dom.node_count() || child.0 as usize >= self.dom.node_count() {
            return Err("unknown DOM node".into());
        }
        if self.dom.name(parent).is_none() {
            return Err("append target is not an element".into());
        }
        let mut ancestor = Some(parent);
        while let Some(current) = ancestor {
            if current == child {
                return Err("cannot append an ancestor into its descendant".into());
            }
            ancestor = self.dom.get(current).parent;
        }
        if self.dom.get(child).parent.is_some() {
            self.dom.detach(child);
        }
        self.dom.attach_last(parent, child);
        Ok(())
    }

    fn append_text(&mut self, parent: u32, text: &str) -> Result<(), String> {
        let parent = amazing_dom::NodeIdx(parent);
        if parent.0 as usize >= self.dom.node_count() {
            return Err("unknown DOM node".into());
        }
        if self.dom.name(parent).is_none() {
            return Err("append target is not an element".into());
        }
        self.dom.create_text(parent, text);
        Ok(())
    }

    fn remove_node(&mut self, node: u32) -> Result<(), String> {
        if node as usize >= self.dom.node_count() {
            return Err("unknown DOM node".into());
        }
        let idx = amazing_dom::NodeIdx(node);
        if idx.0 == 0 {
            return Err("cannot remove the document node".into());
        }
        self.dom.detach(idx);
        Ok(())
    }

    fn get_style(&self, node: u32, property: &str) -> Option<String> {
        if node as usize >= self.dom.node_count()
            || self.dom.name(amazing_dom::NodeIdx(node)).is_none()
        {
            return None;
        }
        if property.eq_ignore_ascii_case("cssText") {
            return Some(self.dom.attrs_of(amazing_dom::NodeIdx(node))
                .and_then(|attrs| attrs.iter().find(|(name, _)| name == "style"))
                .map_or(String::new(), |(_, value)| value.clone()));
        }
        let property = BrowserDomHost::css_property_name(property).to_ascii_lowercase();
        if property == "csstext" {
            return self
                .dom
                .attrs_of(amazing_dom::NodeIdx(node))
                .and_then(|attrs| attrs.iter().find(|(key, _)| key == "style"))
                .map(|(_, value)| value.clone())
                .or_else(|| Some(String::new()));
        }
        self.style_declarations(amazing_dom::NodeIdx(node))
            .into_iter()
            .find(|(name, _)| name == &property)
            .map(|(_, value)| value)
    }

    fn set_style(&mut self, node: u32, property: &str, value: &str) -> Result<(), String> {
        let idx = amazing_dom::NodeIdx(node);
        if node as usize >= self.dom.node_count() || self.dom.name(idx).is_none() {
            return Err("style requires an element".into());
        }
        if property.eq_ignore_ascii_case("cssText") {
            return self.set_attribute(node, "style", value);
        }
        let property = BrowserDomHost::css_property_name(property).to_ascii_lowercase();
        let mut declarations = self.style_declarations(idx);
        if let Some((_, current)) = declarations.iter_mut().find(|(name, _)| name == &property) {
            *current = value.trim().to_string();
        } else {
            declarations.push((property, value.trim().to_string()));
        }
        self.write_style_declarations(idx, &declarations)
    }

    fn pending_native_microtasks(&self) -> Vec<amazing_js::interp::NativeMicrotaskRequest> {
        if self.dom.mutation_microtask_pending() && !self.native_forwarded {
            vec![amazing_js::interp::NativeMicrotaskRequest {
                global: crate::js_scheduler::GLOBAL.0,
                kind: amazing_js::interp::NativeMicrotaskKind::MutationObserver,
            }]
        } else {
            Vec::new()
        }
    }

    fn acknowledge_native_microtask(&mut self, request: amazing_js::interp::NativeMicrotaskRequest) {
        debug_assert_eq!(request.global, crate::js_scheduler::GLOBAL.0);
        debug_assert_eq!(request.kind, amazing_js::interp::NativeMicrotaskKind::MutationObserver);
        debug_assert!(self.dom.mutation_microtask_pending());
        self.native_forwarded = true;
    }
}
