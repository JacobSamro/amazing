use amazing_dom::{Dom, Namespace, NodeIdx, NodeKind};
use amazing_js::{DomHost, HostResponse};

#[derive(Debug)]
pub struct ShellDomHost {
    pub dom: Dom,
}

impl ShellDomHost {
    pub fn new(dom: Dom) -> Self {
        Self { dom }
    }

    fn selector_list(selector: &str) -> Option<Vec<amazing_css::selectors::Selector>> {
        selector
            .split(',')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .map(amazing_css::selectors::parse_selector)
            .collect()
    }

    fn find_all(&self, node: NodeIdx, selectors: &[amazing_css::selectors::Selector], out: &mut Vec<u32>) {
        if selectors
            .iter()
            .any(|selector| amazing_css::selectors::matches(&self.dom, node, selector))
        {
            out.push(node.0);
        }
        for child in self.dom.children(node) {
            self.find_all(child, selectors, out);
        }
    }

    fn text_content(&self, node: NodeIdx, out: &mut String) {
        if let NodeKind::Text { data } = &self.dom.get(node).kind {
            out.push_str(data);
        }
        for child in self.dom.children(node) {
            self.text_content(child, out);
        }
    }

    fn named(&self, node: NodeIdx, name: &str) -> Option<NodeIdx> {
        if self.dom.namespace(node) == Some(Namespace::Html) && self.dom.name(node) == Some(name) {
            return Some(node);
        }
        self.dom.children(node).find_map(|child| self.named(child, name))
    }

    fn check_element(&self, node: u32) -> Result<NodeIdx, String> {
        let node = NodeIdx(node);
        if node.0 as usize >= self.dom.node_count() || self.dom.name(node).is_none() {
            Err("DOM operation requires an element".into())
        } else {
            Ok(node)
        }
    }

    fn style_declarations(&self, node: NodeIdx) -> Vec<(String, String)> {
        self.dom
            .attrs_of(node)
            .and_then(|attrs| attrs.iter().find(|(name, _)| name == "style"))
            .map(|(_, value)| value.as_str())
            .unwrap_or("")
            .split(';')
            .filter_map(|declaration| {
                let (name, value) = declaration.split_once(':')?;
                let name = name.trim().to_ascii_lowercase();
                (!name.is_empty()).then(|| (name, value.trim().to_string()))
            })
            .collect()
    }
}

impl DomHost for ShellDomHost {
    fn query_selector(&mut self, selector: &str) -> Option<u32> {
        self.query_selector_all(selector).into_iter().next()
    }

    fn query_selector_all(&mut self, selector: &str) -> Vec<u32> {
        let Some(selectors) = Self::selector_list(selector) else {
            return Vec::new();
        };
        let mut nodes = Vec::new();
        self.find_all(NodeIdx(0), &selectors, &mut nodes);
        nodes
    }

    fn matches_selector(&self, node: u32, selector: &str) -> bool {
        let Some(selectors) = Self::selector_list(selector) else {
            return false;
        };
        selectors.iter().any(|selector| {
            amazing_css::selectors::matches(&self.dom, NodeIdx(node), selector)
        })
    }

    fn parent_node(&self, node: u32) -> Option<u32> {
        self.dom.get(NodeIdx(node)).parent.map(|parent| parent.0)
    }

    fn get_text_content(&self, node: u32) -> Option<String> {
        if node as usize >= self.dom.node_count() {
            return None;
        }
        let mut text = String::new();
        self.text_content(NodeIdx(node), &mut text);
        Some(text)
    }

    fn set_text_content(&mut self, node: u32, text: &str) -> Result<(), String> {
        let node = self.check_element(node)?;
        let children: Vec<_> = self.dom.children(node).collect();
        for child in children {
            self.dom.detach(child);
        }
        self.dom.create_text(node, text);
        Ok(())
    }

    fn get_document_title(&self) -> String {
        let Some(title) = self.named(NodeIdx(0), "title") else {
            return String::new();
        };
        let mut text = String::new();
        self.text_content(title, &mut text);
        text
    }

    fn set_document_title(&mut self, title: &str) -> Result<(), String> {
        if let Some(node) = self.named(NodeIdx(0), "title") {
            return self.set_text_content(node.0, title);
        }
        let head = self
            .named(NodeIdx(0), "head")
            .ok_or_else(|| "document has no head".to_string())?;
        let node = self.dom.create_element(head, "title");
        self.dom.create_text(node, title);
        Ok(())
    }

    fn fetch(&mut self, base_url: &str, input: &str) -> Result<HostResponse, String> {
        let target = if base_url.is_empty() {
            input.to_string()
        } else {
            let base = amazing_net::Url::parse(base_url).map_err(|error| error.to_string())?;
            amazing_net::resolve(input, &base)
                .map_err(|error| error.to_string())?
                .to_string()
        };
        let response = amazing_net::fetch(&target, &amazing_net::FetchOptions::default())
            .map_err(|error| error.to_string())?;
        let body = response
            .text()
            .ok_or_else(|| "fetch response was not text".to_string())?;
        let headers = response
            .headers
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect();
        Ok(HostResponse {
            url: response.url,
            status: response.status,
            body,
            headers,
        })
    }

    fn get_inner_html(&self, _node: u32) -> Option<String> {
        None
    }

    fn set_inner_html(&mut self, _node: u32, _html: &str) -> Result<(), String> {
        Err("innerHTML mutation is not available in the GPUI runtime yet".into())
    }

    fn get_attribute(&self, node: u32, name: &str) -> Option<Option<String>> {
        Some(
            self.dom
                .attrs_of(NodeIdx(node))?
                .iter()
                .find(|(attribute, _)| attribute == name)
                .map(|(_, value)| value.clone()),
        )
    }

    fn set_attribute(&mut self, node: u32, name: &str, value: &str) -> Result<(), String> {
        let node = self.check_element(node)?;
        let mut attrs = self.dom.attrs_of(node).unwrap_or(&[]).to_vec();
        if let Some((_, current)) = attrs.iter_mut().find(|(attribute, _)| attribute == name) {
            *current = value.to_string();
        } else {
            attrs.push((name.to_string(), value.to_string()));
        }
        self.dom.set_attrs(node, attrs);
        Ok(())
    }

    fn remove_attribute(&mut self, node: u32, name: &str) -> Result<(), String> {
        let node = self.check_element(node)?;
        let mut attrs = self.dom.attrs_of(node).unwrap_or(&[]).to_vec();
        attrs.retain(|(attribute, _)| attribute != name);
        self.dom.set_attrs(node, attrs);
        Ok(())
    }

    fn create_element(&mut self, name: &str) -> Result<u32, String> {
        Ok(self
            .dom
            .alloc_unattached(NodeKind::Element {
                name: name.to_ascii_lowercase(),
                attrs: Vec::new(),
                namespace: Namespace::Html,
            })
            .0)
    }

    fn create_text_node(&mut self, text: &str) -> Result<u32, String> {
        Ok(self
            .dom
            .alloc_unattached(NodeKind::Text {
                data: text.to_string(),
            })
            .0)
    }

    fn append_child(&mut self, parent: u32, child: u32) -> Result<(), String> {
        let parent = self.check_element(parent)?;
        let child = NodeIdx(child);
        if child.0 as usize >= self.dom.node_count() {
            return Err("unknown child node".into());
        }
        if self.dom.get(child).parent.is_some() {
            self.dom.detach(child);
        }
        self.dom.attach_last(parent, child);
        Ok(())
    }

    fn append_text(&mut self, parent: u32, text: &str) -> Result<(), String> {
        let parent = self.check_element(parent)?;
        self.dom.create_text(parent, text);
        Ok(())
    }

    fn remove_node(&mut self, node: u32) -> Result<(), String> {
        if node == 0 || node as usize >= self.dom.node_count() {
            return Err("cannot remove DOM node".into());
        }
        self.dom.detach(NodeIdx(node));
        Ok(())
    }

    fn get_style(&self, node: u32, property: &str) -> Option<String> {
        self.style_declarations(NodeIdx(node))
            .into_iter()
            .find(|(name, _)| name == property)
            .map(|(_, value)| value)
    }

    fn set_style(&mut self, node: u32, property: &str, value: &str) -> Result<(), String> {
        let node = self.check_element(node)?;
        let mut declarations = self.style_declarations(node);
        if let Some((_, current)) = declarations.iter_mut().find(|(name, _)| name == property) {
            *current = value.to_string();
        } else {
            declarations.push((property.to_string(), value.to_string()));
        }
        let serialized = declarations
            .iter()
            .map(|(name, value)| format!("{name}: {value};"))
            .collect::<Vec<_>>()
            .join(" ");
        self.set_attribute(node.0, "style", &serialized)
    }
}
