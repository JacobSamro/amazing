//! Driver-owned computed CSSOM objects over the live parsed arena.
use super::*;
use std::collections::BTreeSet;
use amazing_bindings::cssom::{self, ComputedPseudo, ComputedStyleTarget, CssDeclaration, CssomHost, CssomOp};
use amazing_css::{ComputedStyle, selectors::{PseudoElement, Simple}};

#[derive(Clone, Copy)]
pub(super) enum CssomNative {
    GetComputedStyle,
    PropertyGet(u32),
    PropertySet,
}
#[derive(Default)]
pub(super) struct CssomRuntime {
    entries: Vec<CssomEntry>,
    property_names: Vec<String>,
    snapshots: HashMap<ComputedStyleTarget, (SnapshotKey, Rc<CssSnapshot>)>,
}
struct CssomEntry {
    target: ComputedStyleTarget,
}
#[derive(Clone, Copy, PartialEq)]
struct SnapshotKey { epoch: u64, width: f32, height: f32 }
struct CssSnapshot {
    declarations: Vec<CssDeclaration>,
    style: Option<ComputedStyle>,
    values: BTreeMap<String, String>,
}
fn empty_snapshot() -> CssSnapshot {
    CssSnapshot { declarations: Vec::new(), style: None, values: BTreeMap::new() }
}
fn pseudo(value: ComputedPseudo) -> Option<PseudoElement> {
    Some(match value {
        ComputedPseudo::Before => PseudoElement::Before,
        ComputedPseudo::After => PseudoElement::After,
        ComputedPseudo::Marker => PseudoElement::Marker,
        ComputedPseudo::FirstLine => PseudoElement::FirstLine,
        ComputedPseudo::FirstLetter => PseudoElement::FirstLetter,
        ComputedPseudo::Placeholder => PseudoElement::Placeholder,
        ComputedPseudo::Selection => PseudoElement::Selection,
        ComputedPseudo::Backdrop => PseudoElement::Backdrop,
        ComputedPseudo::FileSelectorButton => PseudoElement::FileSelectorButton,
        ComputedPseudo::Element | ComputedPseudo::Empty => return None,
    })
}
fn number(value: f64) -> String {
    if value == 0.0 { return "0".into(); }
    format!("{value:.6}").trim_end_matches('0').trim_end_matches('.').into()
}
fn px(value: f64) -> String { format!("{}px", number(value)) }
fn color(rgba: [u8;4]) -> String {
    let [red, green, blue, alpha] = rgba;
    if alpha == 255 { format!("rgb({red}, {green}, {blue})") }
    else {
        // CSS Color 4 legacy-alpha serialization for the engine's actual
        // eight-bit representation: preserve an exact hundredth if possible.
        let alpha = u32::from(alpha);
        let rounded = (0..=100u32).find(|percent| (*percent*255 + 50)/100 == alpha)
            .map_or_else(|| f64::from((alpha*1000 + 127)/255)/1000.0, |percent| f64::from(percent)/100.0);
        format!("rgba({red}, {green}, {blue}, {})", number(rounded))
    }
}
fn resolved_color(style: &ComputedStyle, name: &str, value: &str) -> Option<String> {
    use amazing_css::values::{Color, ComputedValue};
    let rgba = match style.typed(name) {
        Some(ComputedValue::Color(Color::Rgba(rgba))) => *rgba,
        Some(ComputedValue::Color(Color::CurrentColor)) => {
            match style.typed("color") {
                Some(ComputedValue::Color(Color::Rgba(rgba))) => *rgba,
                _ => return None,
            }
        }
        _ if name.ends_with("-color") || name == "color" => amazing_css::color::parse_css_color(value)?,
        _ => return None,
    };
    Some(color(rgba))
}
fn computed_numeric(style: &ComputedStyle, name: &str) -> Option<String> {
    use amazing_css::values::{ComputedValue, LengthPercentage};
    match style.typed(name)? {
        ComputedValue::LengthPercentage(LengthPercentage::Px(value)) => Some(px(*value)),
        ComputedValue::LengthPercentage(LengthPercentage::Percentage(value)) => Some(format!("{}%", number(*value))),
        ComputedValue::Number(value) => Some(number(*value)),
        ComputedValue::Integer(value) => Some(value.to_string()),
        _ => None,
    }
}
fn absolute_urls(style: &ComputedStyle, name: &str, value: &str) -> String {
    use amazing_css::syntax::Token;
    let Some(base) = style.url_base(name) else { return value.into(); };
    let tokens = amazing_css::syntax::tokenize(value);
    let mut result = Vec::new();
    let mut index = 0;
    let mut changed = false;
    while index < tokens.len() {
        let mut consumed = 1;
        let url = match &tokens[index] {
            Token::Url { value, .. } => Some(value.clone()),
            Token::Function(function) if function.eq_ignore_ascii_case("url") => {
                let mut at = index + 1;
                if matches!(tokens.get(at), Some(Token::OpenParen)) { at += 1; }
                while matches!(tokens.get(at), Some(Token::Whitespace)) { at += 1; }
                if let Some(Token::Str(value)) = tokens.get(at) {
                    let value = value.clone(); at += 1;
                    while matches!(tokens.get(at), Some(Token::Whitespace)) { at += 1; }
                    if matches!(tokens.get(at), Some(Token::CloseParen)) {
                        consumed = at + 1 - index; Some(value)
                    } else { None }
                } else { None }
            }
            _ => None,
        };
        if let Some(url) = url {
            let resolved = crate::cmd::js::resolve_script_url(base, &url);
            result.extend([Token::Function("url".into()), Token::OpenParen, Token::Str(resolved), Token::CloseParen]);
            changed = true;
        } else { result.push(tokens[index].clone()); }
        index += consumed;
    }
    if changed { amazing_css::syntax::serialize(&result) } else { value.into() }
}
fn box_for(root: &amazing_layout::LayoutBox, node: NodeIdx) -> Option<&amazing_layout::LayoutBox> {
    if root.node == Some(node) { return Some(root); }
    for child in &root.positioned {
        if let Some(found) = box_for(child, node) { return Some(found); }
    }
    match &root.flow {
        amazing_layout::Flow::Blocks(children) | amazing_layout::Flow::Flex(children) | amazing_layout::Flow::Grid(children) => {
            children.iter().find_map(|child| box_for(child, node))
        }
        _ => None,
    }
}
fn resolve_origin_value(style: &ComputedStyle, box_: &amazing_layout::LayoutBox, name: &str) -> Option<String> {
    if !matches!(name, "transform-origin" | "perspective-origin") { return None; }
    let zoom=style.effective_zoom();
    if !zoom.is_finite() || zoom <= 0.0 { return None; }
    // These are associated CSS layout boxes. Under Transforms1 section5,
    // fill-box uses the actual content box; stroke/view-box use the border box.
    // Non-CSS SVG geometry never enters this branch without a LayoutBox.
    let reference=match style.get("transform-box").unwrap_or("view-box") {
        "content-box" | "fill-box" => box_.content_box,
        "border-box" | "stroke-box" | "view-box" => box_.border_box,
        _ => return None,
    };
    // CSSOM reports offsets from the reference box's top-left. The matrix
    // handoff can separately supply its local offset when composing transforms.
    let origin=amazing_css::used_origin(style,name,amazing_css::OriginReferenceBox {
        offset:[0.0;2],size:[f64::from(reference.w)/zoom,f64::from(reference.h)/zoom],
    })?;
    let mut value=format!("{} {}",px(origin[0]),px(origin[1]));
    if name=="transform-origin" && origin[2]!=0.0 { value.push_str(&format!(" {}",px(origin[2]))); }
    Some(value)
}
fn resolve_box_value(style: &ComputedStyle, box_: &amazing_layout::LayoutBox, name: &str) -> Option<String> {
    if let Some(origin)=resolve_origin_value(style,box_,name) { return Some(origin); }
    let zoom = style.effective_zoom();
    let length = match name {
        "width" if style.get("box-sizing") == Some("border-box") => box_.border_box.w,
        "height" if style.get("box-sizing") == Some("border-box") => box_.border_box.h,
        "width" => box_.content_box.w,
        "height" => box_.content_box.h,
        "padding-top" => box_.padding.top,
        "padding-right" => box_.padding.right,
        "padding-bottom" => box_.padding.bottom,
        "padding-left" => box_.padding.left,
        "margin-top" => box_.margin.top?,
        "margin-right" => box_.margin.right?,
        "margin-bottom" => box_.margin.bottom?,
        "margin-left" => box_.margin.left?,
        "border-top-width" => box_.border.top,
        "border-right-width" => box_.border.right,
        "border-bottom-width" => box_.border.bottom,
        "border-left-width" => box_.border.left,
        _ => return None,
    };
    Some(px(f64::from(length)/zoom))
}
fn flat_tree_contains(dom: &Dom, node: NodeIdx) -> bool {
    let mut cursor = node;
    while let Some(parent) = dom.get(cursor).parent {
        if dom.shadow_root_of_host(parent).is_some() && dom.assigned_slot(cursor).is_none() { return false; }
        if dom.name(parent) == Some("slot") && dom.assigned_nodes(parent, false, false).ok().is_some_and(|nodes| !nodes.is_empty()) {
            return false;
        }
        cursor = parent;
    }
    true
}
fn document_styles(state: &BindingState, dom: &Dom, document: NodeIdx,
    environment: &amazing_css::MediaEnvironment) -> (amazing_css::StyledDom, crate::page::PageText) {
    let page = if document == NodeIdx(0) { state.document_url.clone().unwrap_or_else(|| "about:blank".into()) }
        else { dom.document_metadata(document).ok().flatten().map(|metadata| metadata.url.clone()).unwrap_or_else(|| "about:blank".into()) };
    let mut sources = Vec::new();
    let _ = crate::page::collect_author_css(dom, document, &page, &mut sources);
    let font_sources = sources.iter().map(|(source, url, _)| (source.clone(), url.clone())).collect::<Vec<_>>();
    let sheets = sources.into_iter().map(|(_,_,sheet)| sheet).collect::<Vec<_>>();
    let text = crate::page::load_page_text_from_sheets(&font_sources, &page);
    let context = crate::font_metrics::FontMetrics { dom, provider: &text.provider };
    let styles = amazing_css::compute_document_with_font_metrics(dom, document, &sheets, environment, None, &context);
    (styles, text)
}
fn active_document(state: &BindingState, dom: &Dom, mut document: NodeIdx,
    environment: &amazing_css::MediaEnvironment) -> bool {
    let mut visited = BTreeSet::new();
    loop {
        if !state.frames.has_document(document) || !visited.insert(document) { return false; }
        let Some(frame) = state.frames.frame_for_document(document) else { return document == NodeIdx(0); };
        if !dom.is_connected(frame) || !flat_tree_contains(dom, frame) { return false; }
        document = dom.owner_document(frame).unwrap_or(NodeIdx(0));
        let (styles, _) = document_styles(state, dom, document, environment);
        let mut cursor = Some(frame);
        while let Some(node) = cursor {
            if styles.get(node).is_some_and(|style| style.get("display") == Some("none")) { return false; }
            cursor = dom.get(node).parent;
        }
    }
}
fn build_snapshot(state: &BindingState, target: ComputedStyleTarget, environment: &amazing_css::MediaEnvironment) -> CssSnapshot {
    if target.element.arena != state.arena || target.pseudo == ComputedPseudo::Empty { return empty_snapshot(); }
    let host = state.host.borrow();
    let dom = &host.dom;
    let node = NodeIdx(target.element.slot as u32);
    if target.element.slot > u32::MAX as u64 || dom.node_generation(node) != Some(target.element.generation)
        || !dom.is_connected(node) || !flat_tree_contains(dom, node) { return empty_snapshot(); }
    let document = dom.owner_document(node).unwrap_or(NodeIdx(0));
    if !active_document(state, dom, document, environment) { return empty_snapshot(); }
    let (styles, text) = document_styles(state, dom, document, environment);
    let style = match pseudo(target.pseudo) {
        Some(pseudo) => styles.get_pseudo(node, pseudo),
        None => styles.get(node),
    };
    let Some(style) = style else { return empty_snapshot(); };
    // A root document layout supplies real used box dimensions. Other document
    // layout roots and generated/inline boxes are not public in Layout yet;
    // retain their computed values rather than substituting guessed geometry.
    let layout = (document == NodeIdx(0) && target.pseudo == ComputedPseudo::Element).then(||
        amazing_layout::layout_styled_document(dom, &styles, &text.provider, &amazing_layout::NoReplacedElements,
            environment.viewport_width_px, Some(environment.viewport_height_px)));
    let box_ = layout.as_ref().filter(|_| style.get("display") != Some("contents"))
        .and_then(|root| box_for(root, node));
    let mut resolved = style.clone();
    let mut declarations = Vec::new();
    let mut values = BTreeMap::new();
    for (name, raw) in style.cssom_declarations() {
        let value = if name.starts_with("--") { raw.clone() } else {
            resolved_color(style, &name, &raw)
                .or_else(|| if name == "line-height" {
                    match (style.typed("line-height"), style.typed("font-size")) {
                        (Some(amazing_css::values::ComputedValue::Number(ratio)),
                         Some(amazing_css::values::ComputedValue::LengthPercentage(amazing_css::values::LengthPercentage::Px(size))))
                            => Some(px(ratio*size)),
                        _ => None,
                    }
                } else { None })
                .or_else(|| box_.and_then(|box_| resolve_box_value(style, box_, &name)))
                .or_else(|| computed_numeric(style, &name))
                .unwrap_or_else(|| absolute_urls(style, &name, &raw))
        };
        if !name.starts_with("--") && value != raw { resolved.set(&name, &value); }
        declarations.push(CssDeclaration { name: name.encode_utf16().collect(), value: value.encode_utf16().collect() });
        values.insert(name, value);
    }
    CssSnapshot { declarations, style: Some(resolved), values }
}

fn snapshot(state: &Rc<RefCell<BindingState>>, handle: HostHandle) -> Result<Rc<CssSnapshot>, Throw> {
    let (target, key, environment) = {
        let state = state.borrow();
        if handle.arena != state.arena || handle.generation != 1 || handle.brand != InterfaceBrand::CSS_STYLE_DECLARATION.0 {
            return Err(Throw::type_error("Invalid CSSStyleDeclaration"));
        }
        let entry = state.cssom.entries.get(handle.slot as usize).ok_or_else(|| Throw::type_error("Invalid CSSStyleDeclaration"))?;
        let environment = state.integrator_assist.cssom_environment();
        let epoch = state.host.borrow().dom.render_mutation_epoch();
        let key = SnapshotKey { epoch, width:environment.viewport_width_px, height:environment.viewport_height_px };
        if let Some((cached_key, snapshot)) = state.cssom.snapshots.get(&entry.target)
            && *cached_key == key { return Ok(snapshot.clone()); }
        (entry.target, key, environment)
    };
    let result = Rc::new(build_snapshot(&state.borrow(), target, &environment));
    state.borrow_mut().cssom.snapshots.insert(target, (key, result.clone()));
    Ok(result)
}

impl RuntimeBindingHost<'_> {
    pub(super) fn cssom_install(&mut self) -> Result<OptionalHookInstall, Throw> {
        let getter = self.native("getComputedStyle", 1, NativeEntry::Cssom(CssomNative::GetComputedStyle), HostCallPolicy::CallOnly);
        let global = self.global_object()?;
        self.define_data_property(&global, "getComputedStyle", getter, true, true, true)?;
        let prototype = self.prototype(Interface::CSSStyleProperties).ok_or_else(|| Throw::type_error("CSSStyleProperties prototype missing"))?;
        for name in amazing_css::cssom_property_names() {
            for alias in property_aliases(name) {
                if alias == "cssFloat" { continue; }
                let index = self.state.borrow().cssom.property_names.len() as u32;
                self.state.borrow_mut().cssom.property_names.push(name.into());
                let getter = self.native("get CSS property", 0, NativeEntry::Cssom(CssomNative::PropertyGet(index)), HostCallPolicy::CallOnly);
                let setter = self.native("set CSS property", 1, NativeEntry::Cssom(CssomNative::PropertySet), HostCallPolicy::CallOnly);
                let name_flags = BindingPropertyFlags { writable:false, enumerable:false, configurable:true };
                self.js.binding_define_data_property(&getter, "name", JsValue::str(&format!("get {alias}")), name_flags)?;
                self.js.binding_define_data_property(&setter, "name", JsValue::str(&format!("set {alias}")), name_flags)?;
                self.js.binding_define_accessor_property(&prototype, &alias, Some(&getter), Some(&setter), true, true)?;
            }
        }
        let count_state = self.state.clone();
        let get_state = self.state.clone();
        self.js.binding_define_legacy_platform_object(InterfaceBrand::CSS_STYLE_DECLARATION.0, RuntimeLegacyHooks {
            supported_index_count: Rc::new(move |handle| Ok(snapshot(&count_state, handle)?.declarations.len() as u32)),
            indexed_getter: Rc::new(move |_,handle,index| {
                let values = snapshot(&get_state, handle)?;
                Ok(values.declarations.get(index as usize).map_or(JsValue::Undefined, |declaration| JsValue::str_units(declaration.name.clone())))
            }),
            supported_names: None, named_getter: None, indexed_setter: None,
            named_setter: None, named_deleter: None, unenumerable_names:false, override_builtins:false,
        });
        // An indexed getter interface gets Array.prototype.values as its
        // default iterator; this does not add NodeList's iterable methods.
        let array = self.js.binding_get_property(&global, "Array")?;
        let array_prototype = self.js.binding_get_property(&array, "prototype")?;
        let values = self.js.binding_get_property(&array_prototype, "values")?;
        let declaration_prototype = self.prototype(Interface::CSSStyleDeclaration)
            .ok_or_else(|| Throw::type_error("CSSStyleDeclaration prototype missing"))?;
        self.js.binding_define_symbol_data_property(&declaration_prototype,
            amazing_js::value::WellKnownSymbol::Iterator.symbol(), values, BindingPropertyFlags::METHOD)?;
        Ok(OptionalHookInstall::Installed)
    }
    pub(super) fn cssom_invoke_native(&mut self, operation: CssomNative, receiver: JsValue, args: &[JsValue]) -> Result<JsValue, Throw> {
        match operation {
            CssomNative::GetComputedStyle => {
                if !self.is_nullish(&receiver) && self.dom_window_event_target_slot(&receiver).is_none() {
                    return Err(Throw::type_error("getComputedStyle requires a Window receiver"));
                }
                cssom::get_computed_style(self, args)
            }
            CssomNative::PropertyGet(index) => {
                let target = self.cssom_target(&receiver).ok_or_else(|| Throw::type_error("Expected a CSSStyleDeclaration"))?;
                let name = self.state.borrow().cssom.property_names.get(index as usize).cloned().ok_or_else(|| Throw::type_error("Invalid CSS property"))?;
                let value = self.cssom_property_value(target, &name.encode_utf16().collect::<Vec<_>>())?;
                self.string(&value)
            }
            CssomNative::PropertySet => cssom::invoke(self, CssomOp::SetCssFloat, receiver, args),
        }
    }
}
fn property_aliases(name: &str) -> Vec<String> {
    let mut camel = String::new();
    let mut upper = false;
    for ch in name.chars() {
        if ch == '-' { upper = true; continue; }
        camel.push(if upper { ch.to_ascii_uppercase() } else { ch }); upper=false;
    }
    if name == "float" { return vec!["float".into(), "cssFloat".into()]; }
    let mut aliases = if camel == name { vec![name.into()] } else { vec![name.into(),camel.clone()] };
    if name.starts_with("-webkit-") {
        camel.replace_range(..1, "w");
        aliases.push(camel);
    }
    aliases
}
impl CssomHost for RuntimeBindingHost<'_> {
    fn cssom_parse_pseudo(&mut self, units: &[u16]) -> ComputedPseudo {
        if units.first() != Some(&58) { return ComputedPseudo::Element; }
        let Ok(text) = String::from_utf16(units) else { return ComputedPseudo::Empty };
        if !text.starts_with(':') { return ComputedPseudo::Element; }
        // Parse one pseudo-element selector, not a selector surrounded by
        // whitespace. Hex-escape terminator whitespace is consumed inside
        // the token, so only a distinct trailing token rejects the request.
        if matches!(amazing_css::syntax::tokenize(&text).last(), Some(amazing_css::syntax::Token::Whitespace)) {
            return ComputedPseudo::Empty;
        }
        let Some(selector) = amazing_css::selectors::parse_selector(&text) else { return ComputedPseudo::Empty };
        if selector.compounds.len()!=1 || !selector.combinators.is_empty() { return ComputedPseudo::Empty; }
        match selector.compounds[0].simple.as_slice() {
            [Simple::PseudoElement(pseudo)] => match pseudo {
                PseudoElement::Before => ComputedPseudo::Before, PseudoElement::After => ComputedPseudo::After,
                PseudoElement::Marker => ComputedPseudo::Marker, PseudoElement::FirstLine => ComputedPseudo::FirstLine,
                PseudoElement::FirstLetter => ComputedPseudo::FirstLetter, PseudoElement::Placeholder => ComputedPseudo::Placeholder,
                PseudoElement::Selection => ComputedPseudo::Selection, PseudoElement::Backdrop => ComputedPseudo::Backdrop,
                PseudoElement::FileSelectorButton => ComputedPseudo::FileSelectorButton,
            },
            _ => ComputedPseudo::Empty,
        }
    }
    fn cssom_element_key(&self, value: &JsValue) -> Option<PlatformKey> {
        let node = self.node_brand(value)?;
        let state = self.state.borrow(); let host = state.host.borrow();
        if !matches!(host.dom.get(node).kind, NodeKind::Element { .. }) { return None; }
        Some(PlatformKey { arena:state.arena,slot:u64::from(node.0),generation:host.dom.node_generation(node)? })
    }
    fn cssom_wrap_computed(&mut self, target: ComputedStyleTarget) -> Result<JsValue, Throw> {
        let slot = self.state.borrow().cssom.entries.len() as u64;
        self.state.borrow_mut().cssom.entries.push(CssomEntry { target });
        let prototype = self.prototype(Interface::CSSStyleProperties).ok_or_else(|| Throw::type_error("CSSStyleProperties prototype missing"))?;
        self.js.binding_wrap_host(HostHandle { arena:self.state.borrow().arena,slot,generation:1,brand:InterfaceBrand::CSS_STYLE_DECLARATION.0 }, &prototype)
    }
    fn cssom_target(&self, value: &JsValue) -> Option<ComputedStyleTarget> {
        let handle = self.js.binding_host_handle(value, InterfaceBrand::CSS_STYLE_DECLARATION.0)?;
        let state = self.state.borrow();
        if handle.arena != state.arena || handle.generation != 1 { return None; }
        state.cssom.entries.get(handle.slot as usize).map(|entry| entry.target)
    }
    fn cssom_declarations(&mut self, target: ComputedStyleTarget) -> Result<Vec<CssDeclaration>, Throw> {
        let slot = self.state.borrow().cssom.entries.iter().position(|entry| entry.target == target)
            .ok_or_else(|| Throw::type_error("Invalid computed target"))?;
        let handle = HostHandle { arena:self.state.borrow().arena,slot:slot as u64,generation:1,brand:InterfaceBrand::CSS_STYLE_DECLARATION.0 };
        Ok(snapshot(&self.state, handle)?.declarations.clone())
    }
    fn cssom_property_value(&mut self, target: ComputedStyleTarget, property: &[u16]) -> Result<Vec<u16>, Throw> {
        let Ok(property) = String::from_utf16(property) else { return Ok(Vec::new()); };
        let slot = self.state.borrow().cssom.entries.iter().position(|entry| entry.target == target)
            .ok_or_else(|| Throw::type_error("Invalid computed target"))?;
        let handle = HostHandle { arena:self.state.borrow().arena,slot:slot as u64,generation:1,brand:InterfaceBrand::CSS_STYLE_DECLARATION.0 };
        let values = snapshot(&self.state, handle)?;
        let value = values.values.get(&property).cloned().or_else(|| values.style.as_ref().and_then(|style| style.cssom_property_value(&property)));
        Ok(value.unwrap_or_default().encode_utf16().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn scheduler(html: &str) -> crate::js_scheduler::DriverScheduler {
        let host = Rc::new(RefCell::new(BrowserDomHost { dom:amazing_html::parse(html),native_forwarded:false }));
        crate::js_scheduler::DriverScheduler::new(host, crate::js_scheduler::JS_CAPACITY).unwrap()
    }
    #[test]
    fn cssom_live_brand_enumeration_and_real_resolved_box_values() {
        let mut scheduler = scheduler("<style>div{width:100px;height:20px;padding:2px;border:1px solid red;color:red;--Case: 7} .changed{width:130px;color:blue}</style><div id=target>A</div><span id=resolved style='color:rgba(0,0,255,.5);font-size:20px;line-height:1.25;position:relative;top:2em;opacity:.5'>B</span>");
        let result = scheduler.run_script(r#"
            let el=document.getElementById('target'), style=getComputedStyle(el);
            let first=[style instanceof CSSStyleDeclaration, style instanceof CSSStyleProperties,
              Object.getPrototypeOf(style)===CSSStyleProperties.prototype,
              Object.prototype.toString.call(style),style!==getComputedStyle(el),style.cssText,
              style.parentRule===null,style.width,style.height,style.paddingTop,style.borderTopWidth,
              style.color,style.getPropertyValue('COLOR'),style.getPropertyPriority('color'),
              style.getPropertyValue('--Case'),style.getPropertyValue('--case')].join('|');
            let names=Array.from(style), ordinary=names.filter(n=>!n.startsWith('--'));
            let order=ordinary.join('|')===ordinary.slice().sort().join('|');
            let indexed=style[0]===style.item(0)&&style.item(-1)===''&&style.item(4294967296)===style[0];
            el.setAttribute('class','changed');
            let inline=getComputedStyle(document.getElementById('resolved'));
            [first,order,indexed,style.width,style.color,style.getPropertyValue('padding'),
             style.getPropertyValue('border-top'),style.length===Array.from(style).length,
             inline.color==='rgba(0, 0, 255, 0.5)'&&inline.lineHeight==='25px'&&style.float===style.cssFloat&&inline.top==='40px'&&inline.opacity==='0.5',
             style.padding==='2px'&&typeof style.background==='string'&&typeof style.font==='string',
             CSSStyleDeclaration.prototype[Symbol.iterator]===Array.prototype.values&&
             !Object.prototype.hasOwnProperty.call(CSSStyleProperties.prototype,Symbol.iterator),
             Object.getOwnPropertyDescriptor(CSSStyleProperties.prototype,'paddingTop').get.name==='get paddingTop'&&
             Object.getOwnPropertyDescriptor(CSSStyleProperties.prototype,'padding-top').set.name==='set padding-top'].join(';')
        "#).unwrap();
        assert_eq!(result, JsValue::str("true|true|true|[object CSSStyleProperties]|true||true|100px|20px|2px|1px|rgb(255, 0, 0)|rgb(255, 0, 0)||7|;true;true;130px;rgb(0, 0, 255);2px;1px solid rgb(255, 0, 0);true;true;true;true;true"));
    }
    #[test]
    fn cssom_webidl_conversion_order_readonly_errors_and_reentrancy() {
        let mut scheduler = scheduler("<style>div{color:red}</style><div id=target>A</div>");
        let result = scheduler.run_script(r#"
            let el=document.getElementById('target'), style=getComputedStyle(el), errors=[],order=[];
            for(let run of [()=>style.setProperty('color','blue'),()=>style.removeProperty('color'),
               ()=>{style.cssText='color:blue'},()=>{style.color='blue'},()=>{style.cssFloat=null}]) {
              try{run()}catch(e){errors.push(e instanceof DOMException && e.name==='NoModificationAllowedError' && e.code===7)}
            }
            let property={toString(){order.push('name');return 'color'}},
                value={toString(){order.push('value');return 'blue'}},
                priority={toString(){order.push('priority');return ''}};
            try{style.setProperty(property,value,priority)}catch(e){order.push(e.name)}
            let thrown={marker:1},same=false;
            try{style.setProperty('color',{toString(){throw thrown}})}catch(e){same=e===thrown}
            let pseudo={toString(){el.setAttribute('style','color:blue');return ''}};
            let reentrant=getComputedStyle(el,pseudo).color;
            let typeErrors=0;
            for(let run of [()=>getComputedStyle(),()=>getComputedStyle({}),()=>getComputedStyle(document),
               ()=>CSSStyleDeclaration.prototype.getPropertyValue.call({},'color'),
               ()=>style.getPropertyValue(),()=>style.item(),()=>style.setProperty('color')]) {
              try{run()}catch(e){if(e instanceof TypeError)typeErrors++}
            }
            [errors.every(Boolean),order.join(','),same,reentrant,typeErrors,el.getAttribute('style')].join('|')
        "#).unwrap();
        assert_eq!(result, JsValue::str("true|name,value,priority,NoModificationAllowedError|true|rgb(0, 0, 255)|7|color:blue"));
    }
    #[test]
    fn cssom_pseudos_detached_and_non_flat_tree_nodes_are_live() {
        let mut scheduler = scheduler("<style>div{color:red}div::before{content:'A';color:blue}</style><div id=target>A</div><section id=shadow><span id=light>light</span></section>");
        let result = scheduler.run_script(r#"
            let el=document.getElementById('target'), before=getComputedStyle(el,'::before');
            let original=[getComputedStyle(el,':before').color,before.color,
                getComputedStyle(el,'before').color,getComputedStyle(el,'::slotted(*)').length,
                getComputedStyle(el,'::part(a)').length,getComputedStyle(el,'::unknown').length,
                getComputedStyle(el,'::before ').length,getComputedStyle(el,'::befor\\65 ').color].join('|');
            let light=document.getElementById('light'),root=document.getElementById('shadow').attachShadow({mode:'open'});
            let hidden=getComputedStyle(light).length;
            el.remove();let removed=before.length;
            document.body.appendChild(el);let restored=before.color;
            let doc=document.implementation.createHTMLDocument('inert');
            [original,hidden,removed,restored,getComputedStyle(doc.body).length].join(';')
        "#).unwrap();
        assert_eq!(result, JsValue::str("rgb(0, 0, 255)|rgb(0, 0, 255)|rgb(255, 0, 0)|0|0|0|0|rgb(0, 0, 255);0;0;rgb(0, 0, 255);0"));
    }
    #[test]
    fn cssom_uses_registered_font_before_inheritance_and_keeps_custom_case() {
        let directory=std::env::temp_dir().join(format!("amazing-cssom-font-{}",std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let font=directory.join("face.ttf");
        std::fs::write(&font,include_bytes!("../tests/fixtures/font-metrics-space.ttf")).unwrap();
        let html=format!("<style>@font-face{{font-family:Actual;src:url('{}')}}div{{font-family:Actual;font-size-adjust:cap-height from-font;--Case: 2;--bad:var(--bad)}}span{{font-family:monospace}}</style><div id=parent><span id=child>A</span></div>",font.display());
        let mut scheduler=scheduler(&html);
        let result=scheduler.run_script(r#"
            let parent=getComputedStyle(document.getElementById('parent')),
                child=getComputedStyle(document.getElementById('child'));
            let value=parent.fontSizeAdjust;
            [!value.includes('from-font')&&value!=='none',child.fontSizeAdjust===value,
              parent.getPropertyValue('--Case'),parent.getPropertyValue('--case'),
              !Array.from(parent).includes('--bad')].join('|')
        "#).unwrap();
        assert_eq!(result,JsValue::str("true|true|2||true"));
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn cssom_origin_reference_boxes_use_real_content_and_border_geometry() {
        let mut scheduler=scheduler("<style>div{width:100px;height:80px;padding:10px;border:2px solid;transform-origin:50% 50%;perspective-origin:100% 25%}</style><div id=target></div>");
        let result=scheduler.run_script(r#"
            let el=document.getElementById('target'),s=getComputedStyle(el),out=[];
            for(let box of ['view-box','border-box','stroke-box','content-box','fill-box']) {
                el.setAttribute('style','transform-box:'+box);
                out.push(s.transformOrigin+'|'+s.perspectiveOrigin);
            }
            out.join(';')
        "#).unwrap();
        assert_eq!(result,JsValue::str("62px 52px|124px 26px;62px 52px|124px 26px;62px 52px|124px 26px;50px 40px|100px 20px;50px 40px|100px 20px"));
    }
    #[test]
    fn cssom_origin_calculations_use_document_font_and_nonzero_z() {
        let mut scheduler=scheduler("<style>body{font-size:40px}div{width:200px;height:300px;transform-origin:calc(-100% + 10px - .5em) calc(10px - .5em) calc(10px - .5em);perspective-origin:right calc(10px - .5em) top calc(10px - .5em)}</style><div id=target></div>");
        let result=scheduler.run_script(r#"
            let el=document.getElementById('target'),s=getComputedStyle(el);
            let first=s.transformOrigin+'|'+s.perspectiveOrigin;
            el.setAttribute('style','font-size:20px;transform-origin:10% bottom 0px');
            [first,s.transformOrigin,s.perspectiveOrigin].join(';')
        "#).unwrap();
        assert_eq!(result,JsValue::str("-210px -10px -10px|210px -10px;20px 300px;200px 0px"));
    }
    #[test]
    fn cssom_origins_keep_effective_zoom_in_css_pixels_and_update_live_dimensions() {
        let mut scheduler=scheduler("<style>section{zoom:2}div{zoom:1.5;box-sizing:border-box;width:124px;height:104px;padding:10px;border:2px solid;transform-origin:50% 50% 7px;perspective-origin:50% 50%}</style><section><div id=target></div></section>");
        let result=scheduler.run_script(r#"
            let el=document.getElementById('target'),s=getComputedStyle(el);
            let first=s.transformOrigin+'|'+s.perspectiveOrigin;
            el.setAttribute('style','transform-box:content-box');
            let content=s.transformOrigin+'|'+s.perspectiveOrigin;
            el.setAttribute('style','width:224px;height:204px;transform-box:content-box');
            [first,content,s.transformOrigin,s.perspectiveOrigin].join(';')
        "#).unwrap();
        assert_eq!(result,JsValue::str("62px 52px 7px|62px 52px;50px 40px 7px|50px 40px;100px 90px 7px;100px 90px"));
    }
    #[test]
    fn cssom_origin_fallbacks_preserve_unavailable_box_and_detached_behavior() {
        let mut scheduler=scheduler("<style>div{width:100px;height:80px;transform-origin:25% 75% 5px;perspective-origin:40% 60%}div::before{content:'x';transform-origin:10% 20%;perspective-origin:30% 40%}</style><div id=target></div>");
        let result=scheduler.run_script(r#"
            let el=document.getElementById('target'),s=getComputedStyle(el),pseudo=getComputedStyle(el,'::before');
            let first=s.transformOrigin+'|'+s.perspectiveOrigin;
            let generated=pseudo.transformOrigin+'|'+pseudo.perspectiveOrigin;
            el.setAttribute('style','display:none');
            let hidden=s.transformOrigin+'|'+s.perspectiveOrigin;
            el.setAttribute('style','display:contents');
            let contents=s.transformOrigin+'|'+s.perspectiveOrigin;
            el.remove();
            [first,generated,hidden,contents,s.transformOrigin,s.perspectiveOrigin].join(';')
        "#).unwrap();
        assert_eq!(result,JsValue::str("25px 60px 5px|40px 48px;10% 20%|30% 40%;25% 75% 5px|40% 60%;25% 75% 5px|40% 60%;;"));
    }

    #[test]
    fn cssom_origins_use_loaded_document_font_context_before_inheritance() {
        let font=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/font-metrics-space.ttf");
        let html=format!("<style>@font-face{{font-family:Actual;src:url('{}')}}section{{font-family:Actual;font-size:40px}}div{{width:200px;height:300px;transform-origin:calc(10% + .5em) calc(50% - 1em) 1em;perspective-origin:calc(100% - .5em) 1em}}</style><section><div id=target></div></section>",font.display());
        let mut scheduler=scheduler(&html);
        let result=scheduler.run_script(r#"
            let s=getComputedStyle(document.getElementById('target'));
            [s.fontFamily,s.fontSize,s.transformOrigin,s.perspectiveOrigin].join('|')
        "#).unwrap();
        assert_eq!(result,JsValue::str("Actual|40px|40px 110px 40px|180px 40px"));
    }

}
