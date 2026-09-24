//! Canvas runtime objects and the driver's retained compositor adapter.
use super::*;
use amazing_bindings::canvas_interfaces::{self, MemberKind as CanvasMemberKind};
use amazing_canvas::{self as canvas, FillRule, ImageData as CanvasImageData, PaintStyle, Path2D, Transform};
use amazing_compositor::{BoundProperty, BoundPropertyChange, BoundPropertyNode, BoundPropertyValue,
    Compositor, CompositorFrame, DrawQuad, Matrix4, NodePropertyKey, PaintViewRasterMode,
    PropertyBindingError, Rect, SceneError,
    SolidColorQuad, BlendMode};
use amazing_css::{AttributeStyleUpdate, StyledDom};

/// Driver-owned scene held across rendering updates. CSS supplies already
/// classified property values; a miss sends the update through full rendering.
pub(crate) struct RetainedPropertyScene {
    compositor: Compositor,
    width: u32,
    height: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PropertySceneError {
    StaleNode,
    UnsupportedPaintMode,
    NeedsFullRendering,
    Binding(PropertyBindingError),
    NeedsPaint(SceneError),
}

impl RetainedPropertyScene {
    pub(crate) fn new(
        dom: &Dom,
        mut compositor: Compositor,
        bindings: &[(NodePropertyKey, BoundPropertyNode)],
        mode: PaintViewRasterMode,
        width: u32,
        height: u32,
    ) -> Result<Self, PropertySceneError> {
        if mode != PaintViewRasterMode::RawLocal {
            return Err(PropertySceneError::UnsupportedPaintMode);
        }
        for &(key, _) in bindings {
            if dom.node_generation(NodeIdx(key.node)) != Some(key.generation) {
                return Err(PropertySceneError::StaleNode);
            }
        }
        compositor.bind_property_nodes(bindings).map_err(PropertySceneError::Binding)?;
        Ok(Self { compositor, width, height })
    }

    /// Returns a frame without style computation, layout, paint-view creation,
    /// or a paint source. The caller rebuilds the scene on any error.
    pub(crate) fn apply_property_only(
        &mut self,
        dom: &Dom,
        changes: &[BoundPropertyChange],
    ) -> Result<CompositorFrame, PropertySceneError> {
        for change in changes {
            if dom.node_generation(NodeIdx(change.key.node)) != Some(change.key.generation) {
                return Err(PropertySceneError::StaleNode);
            }
        }
        self.compositor.apply_bound_property_changes(changes)
            .map_err(PropertySceneError::Binding)?;
        let mut frame = self.compositor.build_cached_frame(self.width, self.height)
            .map_err(PropertySceneError::NeedsPaint)?;
        let viewport = Rect::new(0., 0., self.width as f32, self.height as f32);
        frame.passes[0].quads.insert(0, DrawQuad::SolidColor(SolidColorQuad {
            rect: viewport, color: [255; 4], clip_rect: viewport,
            opacity: 1., blend: BlendMode::Normal,
        }));
        Ok(frame)
    }

    /// Consume CSS-13's resolved inline-style delta. Recomputed changes stay
    /// on the normal style/layout/paint path; a successful property frame also
    /// patches the retained computed-style cache for the next delta.
    pub(crate) fn apply_attribute_style_update(
        &mut self,
        dom: &Dom,
        styles: &mut StyledDom,
        update: &AttributeStyleUpdate,
    ) -> Result<CompositorFrame, PropertySceneError> {
        let AttributeStyleUpdate::CompositorOnly(delta) = update else {
            return Err(PropertySceneError::NeedsFullRendering);
        };
        if styles.get(delta.node).is_none() {
            return Err(PropertySceneError::NeedsFullRendering);
        }
        let generation = dom.node_generation(delta.node).ok_or(PropertySceneError::StaleNode)?;
        let key = |property| NodePropertyKey { node: delta.node.0, generation, property };
        let mut changes = Vec::with_capacity(2);
        if let Some(opacity) = delta.opacity {
            changes.push(BoundPropertyChange {
                key: key(BoundProperty::Opacity), value: BoundPropertyValue::Opacity(opacity),
            });
        }
        if let Some(matrix) = delta.transform_matrix {
            changes.push(BoundPropertyChange {
                key: key(BoundProperty::Transform),
                value: BoundPropertyValue::Transform(Matrix4(matrix)),
            });
        }
        let frame = self.apply_property_only(dom, &changes)?;
        if !delta.apply_to_styles(styles) {
            return Err(PropertySceneError::NeedsFullRendering);
        }
        Ok(frame)
    }

    pub(crate) fn compositor(&self) -> &Compositor { &self.compositor }
    pub(crate) fn compositor_mut(&mut self) -> &mut Compositor { &mut self.compositor }
}

#[cfg(test)]
mod property_scene_tests {
    use super::*;
    use amazing_compositor::{BoundProperty, BoundPropertyValue, CompositingReasons,
        EffectId, PaintChunk, Presenter, PropertyTreeState, PropertyTrees, ReferenceCpuPresenter,
        TransformId};
    use amazing_paint::Surface;
    use std::cell::Cell;

    #[test]
    fn preclassified_opacity_class_mutation_skips_upstream_fallback() {
        let mut dom = Dom::new();
        let element = dom.create_element(NodeIdx(0), "div");
        dom.set_attribute(element, "class", "opaque").unwrap();
        let generation = dom.node_generation(element).unwrap();
        let mut trees = PropertyTrees::new();
        let effect = trees.add_effect(EffectId(0), 1., BlendMode::Normal, None).unwrap();
        let mut compositor = Compositor::new(trees, vec![PaintChunk {
            items: 0..1,
            state: PropertyTreeState { effect, ..Default::default() },
            bounds: Rect::new(0., 0., 32., 32.),
            reasons: CompositingReasons::WILL_CHANGE,
        }], 1).unwrap();
        let mut source = Surface::new(32, 32);
        source.clear([0, 128, 0, 255]);
        let first = compositor.build_frame(&source, 32, 32).unwrap();
        let mut presenter = ReferenceCpuPresenter::default();
        presenter.present(&first).unwrap();
        let key = NodePropertyKey {
            node: element.0, generation, property: BoundProperty::Opacity,
        };
        let mut scene = RetainedPropertyScene::new(&dom, compositor,
            &[(key, BoundPropertyNode::Opacity(effect))], PaintViewRasterMode::RawLocal,
            32, 32).unwrap();

        // This is the driver's rendering-update boundary after CSS has
        // classified the class diff as a value-only opacity change.
        dom.set_attribute(element, "class", "faded").unwrap();
        let style_calls = Cell::new(0);
        let layout_calls = Cell::new(0);
        let paint_view_calls = Cell::new(0);
        let frame = scene.apply_property_only(&dom, &[BoundPropertyChange {
            key, value: BoundPropertyValue::Opacity(0.5),
        }]).unwrap_or_else(|_| {
            style_calls.set(style_calls.get() + 1);
            layout_calls.set(layout_calls.get() + 1);
            paint_view_calls.set(paint_view_calls.get() + 1);
            panic!("property-only class mutation unexpectedly needed full rendering")
        });
        let pixels = presenter.present(&frame).unwrap();
        assert_eq!(pixels.get_pixel(0, 0), Some([127, 191, 127, 255]));
        assert!(frame.resources.uploads.is_empty());
        assert_eq!(scene.compositor().dirty_tiles(), 0);
        assert_eq!((style_calls.get(), layout_calls.get(), paint_view_calls.get()), (0, 0, 0));
    }

    #[test]
    fn css13_attribute_delta_updates_bound_nodes_without_recompute() {
        let mut dom = Dom::new();
        let html = dom.create_element(NodeIdx(0), "html");
        let element = dom.create_element(html, "div");
        let before = "opacity:.4; transform:translate(1px,0px)";
        let after = "opacity:.6; transform:translate(2px,0px)";
        dom.set_attribute(element, "style", before).unwrap();
        let sheet = amazing_css::parse_stylesheet("");
        let mut styles = amazing_css::compute(&dom, &[sheet.clone()]);
        let invalidator = amazing_css::StyleInvalidator::new(&[sheet], 1);

        let mut trees = PropertyTrees::new();
        let transform = trees.add_transform(TransformId(0), Matrix4::translation(1., 0., 0.),
            (0., 0., 0.), false).unwrap();
        let effect = trees.add_effect(EffectId(0), 0.4, BlendMode::Normal, None).unwrap();
        let mut compositor = Compositor::new(trees, vec![PaintChunk {
            items: 0..1,
            state: PropertyTreeState { transform, effect, ..Default::default() },
            bounds: Rect::new(0., 0., 16., 16.),
            reasons: CompositingReasons::WILL_CHANGE,
        }], 1).unwrap();
        let mut source = Surface::new(16, 16);
        source.clear([80, 120, 160, 255]);
        let first = compositor.build_frame(&source, 32, 32).unwrap();
        let mut presenter = ReferenceCpuPresenter::default();
        presenter.present(&first).unwrap();
        let generation = dom.node_generation(element).unwrap();
        let key = |property| NodePropertyKey { node: element.0, generation, property };
        let mut scene = RetainedPropertyScene::new(&dom, compositor, &[
            (key(BoundProperty::Transform), BoundPropertyNode::Transform(transform)),
            (key(BoundProperty::Opacity), BoundPropertyNode::Opacity(effect)),
        ], PaintViewRasterMode::RawLocal, 32, 32).unwrap();

        dom.set_attribute(element, "style", after).unwrap();
        let update = invalidator.update_attribute(&dom, &styles, element, "style",
            Some(before), Some(after), 1);
        assert!(matches!(update, AttributeStyleUpdate::CompositorOnly(_)));
        let frame = scene.apply_attribute_style_update(&dom, &mut styles, &update).unwrap();
        assert!(frame.resources.uploads.is_empty());
        assert_eq!(scene.compositor().trees().effect(effect).unwrap().opacity, 0.6);
        assert_eq!(scene.compositor().trees().transform(transform).unwrap().matrix,
            Matrix4::translation(2., 0., 0.));
        assert_eq!(styles.get(element).unwrap().get("opacity"), Some("0.6"));
        assert_eq!(styles.get(element).unwrap().get("transform"), Some("translate(2px,0px)"));
        let pixels = presenter.present(&frame).unwrap();
        assert_ne!(pixels.get_pixel(2, 2), Some([255; 4]));
    }

}

#[derive(Clone, Copy)]
pub(super) enum CanvasTarget { Element, Context, Path, Gradient, Pattern, ImageData, TextMetrics }

#[derive(Clone, Copy)]
enum CanvasGradientKind { Linear, Radial, Conic }

pub(super) enum CanvasObject {
    Context(NodeIdx, JsValue),
    Path(Path2D),
    Gradient(canvas::Gradient, CanvasGradientKind),
    Pattern(canvas::Pattern, bool),
    ImageData(u32, u32, JsValue),
    TextMetrics(canvas::TextMetrics),
}

impl RuntimeBindings {
    pub(crate) fn set_canvas_text_system(&self, system: Rc<dyn amazing_text::TextSystem>) {
        self.state.borrow_mut().canvas_text_system = Some(system);
    }
    /// Rendering-update seam: drain changed bitmaps after script jobs and
    /// before building the compositor frame.
    pub(crate) fn take_canvas_damage(&self) -> Vec<(NodeIdx, amazing_paint::Surface)> {
        let mut state = self.state.borrow_mut();
        let changed = std::mem::take(&mut state.canvas_dirty);
        changed.into_iter().filter_map(|node| state.canvas_slots.get(&node)
            .map(|slot| (node, slot.context.surface.clone()))).collect()
    }
}

// Canvas registration is deliberately local to this adapter: the shared
// interface tables remain owned by dom-bindings.
impl RuntimeBindingHost<'_> {
    pub(super) fn invoke_operation_with_canvas_reset(&mut self, operation: Operation,
        this: JsValue, args: &[JsValue]) -> Result<JsValue, Throw> {
        if !matches!(operation, Operation::ElementSetAttribute | Operation::ElementRemoveAttribute) {
            return invoke(self, operation, this, args);
        }
        let canvas_node = self.canvas_element(&this).ok();
        let dimension = args.first().and_then(|value| self.js.binding_string_units(value))
            .map(|units| String::from_utf16_lossy(&units).to_ascii_lowercase())
            .is_some_and(|name| name == "width" || name == "height");
        let result = invoke(self, operation, this, args);
        if result.is_ok() && dimension {
            if let Some(node) = canvas_node { self.canvas_reset(node); }
        }
        result
    }

    pub(super) fn canvas_reset(&mut self, node: NodeIdx) {
        let (width, height) = (self.canvas_dimension(node, "width"), self.canvas_dimension(node, "height"));
        let mut state = self.state.borrow_mut();
        if let Some(slot) = state.canvas_slots.get_mut(&node) {
            if width <= 8192 && height <= 8192 && u64::from(width) * u64::from(height) <= 16_777_216 {
                slot.reset(width, height);
            } else { slot.reset(0, 0); }
        }
        state.canvas_style_values.retain(|(key, _), _| *key != node);
        state.canvas_dirty.insert(node);
    }
    pub(super) fn install_canvas(&mut self) -> Result<(), Throw> {
        let element = self.prototype(Interface::HTMLCanvasElement)
            .ok_or_else(|| Throw::type_error("HTMLCanvasElement prototype missing"))?;
        self.install_canvas_members(&element, CanvasTarget::Element, canvas_interfaces::ELEMENT)?;
        for (name, target, members, length) in [
            ("CanvasRenderingContext2D", CanvasTarget::Context, canvas_interfaces::CONTEXT, 0),
            ("Path2D", CanvasTarget::Path, canvas_interfaces::PATH, 0),
            ("CanvasGradient", CanvasTarget::Gradient, canvas_interfaces::GRADIENT, 0),
            ("CanvasPattern", CanvasTarget::Pattern, canvas_interfaces::PATTERN, 0),
            ("ImageData", CanvasTarget::ImageData, canvas_interfaces::IMAGE_DATA, 2),
            ("TextMetrics", CanvasTarget::TextMetrics, canvas_interfaces::TEXT_METRICS, 0),
        ] {
            let prototype = self.js.binding_create_object(Some(&self.js.binding_object_prototype()))?;
            self.install_canvas_members(&prototype, target, members)?;
            let constructor = self.native(name, length, NativeEntry::Canvas(target,
                Member { name, kind: CanvasMemberKind::Constructor(length) }), HostCallPolicy::ConstructOnly);
            self.define_data_property(&constructor, "prototype", prototype.clone(), false, false, false)?;
            self.define_data_property(&prototype, "constructor", constructor.clone(), true, false, true)?;
            let global = self.js.binding_global_object();
            self.define_data_property(&global, name, constructor, true, false, true)?;
        }
        Ok(())
    }

    fn install_canvas_members(&mut self, prototype: &JsValue, target: CanvasTarget, members: &[Member]) -> Result<(), Throw> {
        let mut getters = HashMap::<&'static str, JsValue>::new();
        for member in members {
            let kind = member.kind;
            let name = member.name;
            let length = match kind { CanvasMemberKind::Method(n) | CanvasMemberKind::Constructor(n) => n, _ => 0 };
            let function = self.native(name, length, NativeEntry::Canvas(target, *member), HostCallPolicy::CallOnly);
            match kind {
                CanvasMemberKind::Method(_) => self.define_data_property(prototype, name, function, true, false, true)?,
                CanvasMemberKind::Getter => {
                    self.define_accessor_property(prototype, name, Some(function.clone()), None, true, true)?;
                    getters.insert(name, function);
                }
                CanvasMemberKind::Setter => {
                    let getter = getters.get(name).cloned();
                    self.define_accessor_property(prototype, name, getter, Some(function), true, true)?;
                }
                CanvasMemberKind::Constructor(_) => unreachable!(),
            }
        }
        Ok(())
    }

    fn canvas_object_index(&self, value: &JsValue) -> Option<usize> {
        self.state.borrow().canvas_objects.iter().position(|(candidate, _)|
            self.js.binding_same_value(candidate, value))
    }
    fn canvas_new_object(&mut self, name: &str, payload: CanvasObject) -> Result<JsValue, Throw> {
        let global = self.js.binding_global_object();
        let constructor = self.js.binding_get_property(&global, name)?;
        let prototype = self.js.binding_get_property(&constructor, "prototype")?;
        let object = self.js.binding_create_object(Some(&prototype))?;
        self.materialize_accessors(&object);
        self.state.borrow_mut().canvas_objects.push((object.clone(), payload));
        Ok(object)
    }
    fn canvas_numbers(&mut self, args: &[JsValue], count: usize) -> Result<Option<Vec<f64>>, Throw> {
        if args.len() < count { return Err(Throw::type_error("Not enough canvas arguments")); }
        let mut values = Vec::with_capacity(count);
        for arg in &args[..count] { values.push(self.js.binding_to_number(arg)?); }
        Ok(values.iter().all(|v| v.is_finite()).then_some(values))
    }
    fn canvas_string(&mut self, arg: Option<&JsValue>) -> Result<String, Throw> {
        let value = arg.cloned().unwrap_or(JsValue::Undefined);
        let units = self.js.binding_to_dom_string(&value)?;
        Ok(String::from_utf16_lossy(&units))
    }
    fn canvas_dimension(&self, node: NodeIdx, name: &str) -> u32 {
        let host = self.state.borrow().host.clone();
        let host = host.borrow();
        let default = if name == "width" { 300 } else { 150 };
        host.dom.get_attribute_ns(node, None, name)
            .map(|s| canvas_interfaces::parse_dimension(s, default)).unwrap_or(default)
    }
    pub(super) fn canvas_element(&self, value: &JsValue) -> Result<NodeIdx, Throw> {
        let node = self.node_brand(value).ok_or_else(|| Throw::type_error("Expected HTMLCanvasElement"))?;
        let host = self.state.borrow().host.clone();
        let host = host.borrow();
        if host.dom.namespace(node) != Some(Namespace::Html) || host.dom.name(node) != Some("canvas") {
            return Err(Throw::type_error("Expected HTMLCanvasElement"));
        }
        Ok(node)
    }
    fn canvas_context_node(&self, value: &JsValue) -> Result<NodeIdx, Throw> {
        let index = self.canvas_object_index(value).ok_or_else(|| Throw::type_error("Expected CanvasRenderingContext2D"))?;
        match &self.state.borrow().canvas_objects[index].1 {
            CanvasObject::Context(node, _) => Ok(*node),
            _ => Err(Throw::type_error("Expected CanvasRenderingContext2D")),
        }
    }
    fn canvas_exception(&mut self, name: &str) -> Throw {
        let global = self.js.binding_global_object();
        let constructor = self.js.binding_get_property(&global, "DOMException");
        if let Ok(constructor) = constructor {
            if let Ok(value) = self.js.binding_construct(&constructor,
                vec![JsValue::str(name), JsValue::str(name)]) { return self.js.binding_throw(value); }
        }
        Throw::type_error(name)
    }
    fn canvas_image_data_object(&mut self, image: CanvasImageData) -> Result<JsValue, Throw> {
        let global = self.js.binding_global_object();
        let ctor = self.js.binding_get_property(&global, "Uint8ClampedArray")?;
        let data = self.js.binding_construct(&ctor, vec![JsValue::num(image.data.len() as f64)])?;
        for (i, byte) in image.data.iter().enumerate() {
            self.js.binding_set_property(&data, &i.to_string(), JsValue::num(f64::from(*byte)))?;
        }
        self.canvas_new_object("ImageData", CanvasObject::ImageData(image.width, image.height, data))
    }
    fn canvas_read_image_data(&mut self, value: &JsValue) -> Result<CanvasImageData, Throw> {
        let index = self.canvas_object_index(value).ok_or_else(|| Throw::type_error("Expected ImageData"))?;
        let (width, height, data) = match &self.state.borrow().canvas_objects[index].1 {
            CanvasObject::ImageData(w, h, data) => (*w, *h, data.clone()),
            _ => return Err(Throw::type_error("Expected ImageData")),
        };
        let length = width as usize * height as usize * 4;
        let mut pixels = Vec::with_capacity(length);
        for i in 0..length {
            let byte = self.js.binding_get_property(&data, &i.to_string())?;
            pixels.push(self.js.binding_to_number(&byte)? as u8);
        }
        CanvasImageData::from_rgba(width, height, pixels).map_err(|_| self.canvas_exception("InvalidStateError"))
    }
    pub(super) fn invoke_canvas(&mut self, target: CanvasTarget, member: Member, receiver: JsValue, args: &[JsValue]) -> Result<JsValue, Throw> {
        if matches!(member.kind, CanvasMemberKind::Constructor(_)) {
            return self.canvas_construct(target, args);
        }
        match target {
            CanvasTarget::Element => self.canvas_element_call(member, receiver, args),
            CanvasTarget::Context => self.canvas_context_call(member, receiver, args),
            CanvasTarget::Path => self.canvas_path_call(member, receiver, args),
            CanvasTarget::Gradient => self.canvas_gradient_call(member, receiver, args),
            CanvasTarget::Pattern => self.canvas_pattern_call(member, receiver, args),
            CanvasTarget::ImageData => self.canvas_image_data_call(member, receiver),
            CanvasTarget::TextMetrics => self.canvas_metrics_call(member, receiver),
        }
    }
}

impl RuntimeBindingHost<'_> {
    fn canvas_construct(&mut self, target: CanvasTarget, args: &[JsValue]) -> Result<JsValue, Throw> {
        match target {
            CanvasTarget::Path => {
                let path = if let Some(first) = args.first() {
                    if let Some(index) = self.canvas_object_index(first) {
                        if let CanvasObject::Path(path) = &self.state.borrow().canvas_objects[index].1 {
                            path.clone()
                        } else { Path2D::new() }
                    } else {
                        let data = self.canvas_string(Some(first))?;
                        Path2D::from_svg(&data).map_err(|_| self.canvas_exception("SyntaxError"))?
                    }
                } else { Path2D::new() };
                self.canvas_new_object("Path2D", CanvasObject::Path(path))
            }
            CanvasTarget::ImageData => {
                if let Some(JsValue::Object(object)) = args.first() {
                    if !matches!(object.borrow().exotic, Exotic::TypedArray) {
                        return Err(Throw::type_error("ImageData needs Uint8ClampedArray"));
                    }
                    let global = self.js.binding_global_object();
                    let expected = self.js.binding_get_property(&global, "Uint8ClampedArray")?;
                    let actual = self.js.binding_get_property(&args[0], "constructor")?;
                    if !self.js.binding_same_value(&expected, &actual) {
                        return Err(Throw::type_error("ImageData needs Uint8ClampedArray"));
                    }
                    let width_arg = args.get(1).ok_or_else(|| Throw::type_error("Missing ImageData width"))?;
                    let width = self.js.binding_to_number(width_arg)? as u32;
                    let data = args[0].clone();
                    let length_value = self.js.binding_get_property(&data, "length")?;
                    let length = self.js.binding_to_number(&length_value)? as usize;
                    let height = if let Some(value) = args.get(2) {
                        self.js.binding_to_number(value)? as u32
                    } else if width > 0 { (length / 4 / width as usize) as u32 } else { 0 };
                    if width == 0 || height == 0 || width as u64 * height as u64 > 16_777_216 ||
                        length != width as usize * height as usize * 4 {
                        return Err(self.canvas_exception("IndexSizeError"));
                    }
                    return self.canvas_new_object("ImageData", CanvasObject::ImageData(width, height, data));
                }
                let Some(values) = self.canvas_numbers(args, 2)? else { return Err(Throw::type_error("Non-finite ImageData dimensions")); };
                let (width, height) = (values[0] as u32, values[1] as u32);
                if width == 0 || height == 0 || width > 8192 || height > 8192 || width as u64 * height as u64 > 16_777_216 {
                    return Err(self.canvas_exception("IndexSizeError"));
                }
                let image = CanvasImageData::new(width, height).map_err(|_| self.canvas_exception("IndexSizeError"))?;
                self.canvas_image_data_object(image)
            }
            _ => Err(Throw::type_error("Illegal constructor")),
        }
    }
    fn canvas_element_call(&mut self, member: Member, receiver: JsValue, args: &[JsValue]) -> Result<JsValue, Throw> {
        let node = self.canvas_element(&receiver)?;
        let name = member.name;
        match member.kind {
            CanvasMemberKind::Getter if name == "width" || name == "height" =>
                Ok(JsValue::num(self.canvas_dimension(node, name) as f64)),
            CanvasMemberKind::Setter if name == "width" || name == "height" => {
                let value = self.js.binding_to_number(args.first().unwrap_or(&JsValue::Undefined))?;
                let width = if value.is_finite() { value.rem_euclid(4_294_967_296.) as u32 } else { 0 };
                let host = self.state.borrow().host.clone();
                host.borrow_mut().dom.set_attribute(node, name, &width.to_string())
                    .map_err(|_| self.canvas_exception("InvalidStateError"))?;
                let dimensions = (self.canvas_dimension(node, "width"), self.canvas_dimension(node, "height"));
                if let Some(slot) = self.state.borrow_mut().canvas_slots.get_mut(&node) {
                    if dimensions.0 <= 8192 && dimensions.1 <= 8192 &&
                        u64::from(dimensions.0) * u64::from(dimensions.1) <= 16_777_216 {
                        slot.reset(dimensions.0, dimensions.1);
                    } else { slot.reset(0, 0); }
                }
                self.state.borrow_mut().canvas_style_values.retain(|(key, _), _| *key != node);
                self.state.borrow_mut().canvas_dirty.insert(node);
                Ok(JsValue::Undefined)
            }
            CanvasMemberKind::Method(_) if name == "getContext" => {
                if args.is_empty() {
                    let global = self.js.binding_global_object();
                    let constructor = self.js.binding_get_property(&global, "TypeError")?;
                    let error = self.js.binding_construct(&constructor,
                        vec![JsValue::str("getContext requires a context id")])?;
                    return Err(self.js.binding_throw(error));
                }
                let id = self.canvas_string(args.first())?;
                if id != "2d" { return Ok(JsValue::Null); }
                if let Some((object, _)) = self.state.borrow().canvas_objects.iter().find(|(_, payload)|
                    matches!(payload, CanvasObject::Context(key, _) if *key == node)) {
                    return Ok(object.clone());
                }
                let width = self.canvas_dimension(node, "width");
                let height = self.canvas_dimension(node, "height");
                if width > 8192 || height > 8192 || width as u64 * height as u64 > 16_777_216 { return Ok(JsValue::Null); }
                self.state.borrow_mut().canvas_slots.insert(node, CanvasSlot::new(node, width, height));
                self.state.borrow_mut().canvas_dirty.insert(node);
                self.canvas_new_object("CanvasRenderingContext2D", CanvasObject::Context(node, receiver))
            }
            CanvasMemberKind::Method(_) if name == "toDataURL" => {
                let media_type = if args.is_empty() { "image/png".to_string() } else { self.canvas_string(args.first())?.to_ascii_lowercase() };
                if !self.state.borrow().canvas_slots.get(&node).is_none_or(|slot| slot.origin_clean) {
                    return Err(self.canvas_exception("SecurityError"));
                }
                let dimensions = (self.canvas_dimension(node, "width"), self.canvas_dimension(node, "height"));
                if dimensions.0 == 0 || dimensions.1 == 0 || dimensions.0 > 8192 || dimensions.1 > 8192 ||
                    u64::from(dimensions.0) * u64::from(dimensions.1) > 16_777_216 {
                    return Ok(JsValue::str("data:,"));
                }
                let pixels = self.state.borrow().canvas_slots.get(&node)
                    .map(|slot| slot.context.surface.pixels.clone());
                let image = if let Some(pixels) = pixels {
                    CanvasImageData::from_rgba(dimensions.0, dimensions.1, pixels)
                } else { CanvasImageData::new(dimensions.0, dimensions.1) }
                    .map_err(|_| self.canvas_exception("InvalidStateError"))?;
                let _ = media_type; // Unrecognized encodings fall back to PNG per HTML.
                Ok(JsValue::str(format!("data:image/png;base64,{}", canvas_base64(&canvas::encode_png(&image)))))
            }
            CanvasMemberKind::Method(_) if name == "toBlob" => {
                let callback = args.first().ok_or_else(|| Throw::type_error("toBlob needs a callback"))?;
                if !self.js.binding_is_callable(callback) { return Err(Throw::type_error("toBlob callback is not callable")); }
                if !self.state.borrow().canvas_slots.get(&node).is_none_or(|slot| slot.origin_clean) {
                    return Err(self.canvas_exception("SecurityError"));
                }
                let (width, height) = (self.canvas_dimension(node, "width"), self.canvas_dimension(node, "height"));
                let argument = if width == 0 || height == 0 || width > 8192 || height > 8192 ||
                    u64::from(width) * u64::from(height) > 16_777_216 { JsValue::Null } else {
                    let pixels = self.state.borrow().canvas_slots.get(&node)
                        .map(|slot| slot.context.surface.pixels.clone())
                        .unwrap_or_else(|| vec![0; width as usize * height as usize * 4]);
                    let image = CanvasImageData::from_rgba(width, height, pixels)
                        .map_err(|_| self.canvas_exception("InvalidStateError"))?;
                    self.blob_new(Blob::from_bytes(canvas::encode_png(&image), "image/png"))?
                };
                self.js.binding_call(callback, JsValue::Undefined, vec![argument])?;
                Ok(JsValue::Undefined)
            }
            _ => Err(Throw::type_error("Unknown canvas element member")),
        }
    }
}

fn canvas_base64(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let a = chunk[0];
        let b = *chunk.get(1).unwrap_or(&0);
        let c = *chunk.get(2).unwrap_or(&0);
        out.push(DIGITS[(a >> 2) as usize] as char);
        out.push(DIGITS[(((a & 3) << 4) | (b >> 4)) as usize] as char);
        out.push(if chunk.len() > 1 { DIGITS[(((b & 15) << 2) | (c >> 6)) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { DIGITS[(c & 63) as usize] as char } else { '=' });
    }
    out
}

impl RuntimeBindingHost<'_> {
    fn canvas_refresh_styles(&mut self, node: NodeIdx) {
        for name in ["fillStyle", "strokeStyle"] {
            let value = self.state.borrow().canvas_style_values.get(&(node, name)).cloned();
            let Some(value) = value else { continue };
            let Some(index) = self.canvas_object_index(&value) else { continue };
            let (style, tainted) = match &self.state.borrow().canvas_objects[index].1 {
                CanvasObject::Gradient(g, kind) => (Some(match kind {
                    CanvasGradientKind::Linear => PaintStyle::LinearGradient(g.clone()),
                    CanvasGradientKind::Radial => PaintStyle::RadialGradient(g.clone()),
                    CanvasGradientKind::Conic => PaintStyle::ConicGradient(g.clone()),
                }), false),
                CanvasObject::Pattern(p, tainted) => (Some(PaintStyle::Pattern(p.clone())), *tainted),
                _ => (None, false),
            };
            let mut state = self.state.borrow_mut();
            if let Some(slot) = state.canvas_slots.get_mut(&node) {
                if let Some(style) = style {
                    if name == "fillStyle" { slot.context.state.fill_style = style; }
                    else { slot.context.state.stroke_style = style; }
                    if tainted { slot.origin_clean = false; }
                }
            }
        }
    }
    fn canvas_context_call(&mut self, member: Member, receiver: JsValue, args: &[JsValue]) -> Result<JsValue, Throw> {
        let node = self.canvas_context_node(&receiver)?;
        let name = member.name;
        if name == "canvas" && member.kind == CanvasMemberKind::Setter { return Ok(JsValue::Undefined); }
        if name == "canvas" { return match &self.state.borrow().canvas_objects[self.canvas_object_index(&receiver).unwrap()].1 {
            CanvasObject::Context(_, element) => Ok(element.clone()), _ => unreachable!()
        }; }
        if matches!(member.kind, CanvasMemberKind::Getter | CanvasMemberKind::Setter) {
            return self.canvas_context_attribute(node, member, args);
        }
        if name == "isContextLost" { return Ok(JsValue::Bool(false)); }
        if name == "drawFocusIfNeeded" {
            if args.is_empty() { return Err(Throw::type_error("Missing focus target")); }
            return Ok(JsValue::Undefined);
        }
        if name == "getContextAttributes" {
            let object = self.js.binding_create_object(Some(&self.js.binding_object_prototype()))?;
            for (key, value) in [("alpha", JsValue::Bool(true)),
                ("desynchronized", JsValue::Bool(false)),
                ("colorSpace", JsValue::str("srgb")),
                ("willReadFrequently", JsValue::Bool(false))] {
                self.js.binding_define_data_property(&object, key, value, BindingPropertyFlags::METHOD)?;
            }
            return Ok(object);
        }
        if name == "createImageData" {
            if let Some(first) = args.first() {
                if let Some(index) = self.canvas_object_index(first) {
                    let dimensions = match &self.state.borrow().canvas_objects[index].1 {
                        CanvasObject::ImageData(w, h, _) => Some((*w, *h)), _ => None,
                    };
                    if let Some((w, h)) = dimensions {
                        let image = CanvasImageData::new(w, h).map_err(|_| self.canvas_exception("IndexSizeError"))?;
                        return self.canvas_image_data_object(image);
                    }
                }
            }
            let Some(v) = self.canvas_numbers(args, 2)? else { return Err(Throw::type_error("Non-finite ImageData dimensions")); };
            let (w, h) = (v[0].abs() as u32, v[1].abs() as u32);
            if w == 0 || h == 0 || w as u64 * h as u64 > 16_777_216 { return Err(self.canvas_exception("IndexSizeError")); }
            let image = CanvasImageData::new(w, h).map_err(|_| self.canvas_exception("IndexSizeError"))?;
            return self.canvas_image_data_object(image);
        }
        if name == "getImageData" {
            let Some(v) = self.canvas_numbers(args, 4)? else { return Err(Throw::type_error("Non-finite image data rectangle")); };
            if !self.state.borrow().canvas_slots.get(&node).is_some_and(|slot| slot.origin_clean) {
                return Err(self.canvas_exception("SecurityError"));
            }
            let image = {
                let state = self.state.borrow();
                state.canvas_slots.get(&node).unwrap().context.get_image_data_signed(v[0] as i32, v[1] as i32, v[2] as i32, v[3] as i32)
            }.map_err(|_| self.canvas_exception("IndexSizeError"))?;
            return self.canvas_image_data_object(image);
        }
        if name == "putImageData" {
            let image = self.canvas_read_image_data(args.first().ok_or_else(|| Throw::type_error("Missing ImageData"))?)?;
            let Some(v) = self.canvas_numbers(&args[1..], 2)? else { return Ok(JsValue::Undefined); };
            if args.len() >= 7 {
                let Some(dirty) = self.canvas_numbers(&args[3..], 4)? else { return Ok(JsValue::Undefined); };
                self.state.borrow_mut().canvas_slots.get_mut(&node).unwrap().context.put_image_data_dirty_signed(
                    &image, v[0] as i32, v[1] as i32,
                    dirty[0] as i32, dirty[1] as i32, dirty[2] as i32, dirty[3] as i32);
            } else {
                self.state.borrow_mut().canvas_slots.get_mut(&node).unwrap().context.put_image_data(&image, v[0] as i32, v[1] as i32);
            }
            self.state.borrow_mut().canvas_dirty.insert(node);
            return Ok(JsValue::Undefined);
        }
        if matches!(name, "createLinearGradient" | "createRadialGradient" | "createConicGradient") {
            let count = match name { "createLinearGradient" => 4, "createRadialGradient" => 6, _ => 3 };
            let Some(v) = self.canvas_numbers(args, count)? else { return Err(Throw::type_error("Non-finite gradient coordinate")); };
            let (gradient, kind) = match name {
                "createLinearGradient" => (canvas::Gradient::linear(v[0], v[1], v[2], v[3]), CanvasGradientKind::Linear),
                "createRadialGradient" => {
                    if v[2] < 0. || v[5] < 0. { return Err(self.canvas_exception("IndexSizeError")); }
                    (canvas::Gradient::radial(v[0], v[1], v[2], v[3], v[4], v[5]), CanvasGradientKind::Radial)
                }
                _ => (canvas::Gradient::conic(v[0], v[1], v[2]), CanvasGradientKind::Conic),
            };
            return self.canvas_new_object("CanvasGradient", CanvasObject::Gradient(gradient, kind));
        }
        if name == "createPattern" {
            let source = args.first().ok_or_else(|| Throw::type_error("Missing pattern source"))?;
            let source_node = self.canvas_element(source)?;
            let repetition = if args.get(1).is_none_or(|value| self.js.binding_is_nullish(value)) {
                String::new()
            } else { self.canvas_string(args.get(1))? };
            let repetition = match repetition.as_str() {
                "" | "repeat" => canvas::Repetition::Repeat,
                "repeat-x" => canvas::Repetition::RepeatX,
                "repeat-y" => canvas::Repetition::RepeatY,
                "no-repeat" => canvas::Repetition::NoRepeat,
                _ => return Err(self.canvas_exception("SyntaxError")),
            };
            let source = self.state.borrow().canvas_slots.get(&source_node).map(|slot|
                (slot.context.surface.width, slot.context.surface.height,
                    slot.context.surface.pixels.clone(), !slot.origin_clean));
            let source = source.or_else(|| {
                let (width, height) = (self.canvas_dimension(source_node, "width"), self.canvas_dimension(source_node, "height"));
                (width <= 8192 && height <= 8192 && u64::from(width) * u64::from(height) <= 16_777_216)
                    .then(|| (width, height, vec![0; width as usize * height as usize * 4], false))
            });
            let Some((width, height, pixels, tainted)) = source else { return Ok(JsValue::Null); };
            if width == 0 || height == 0 { return Ok(JsValue::Null); }
            let bitmap = canvas::Bitmap::new(width, height, pixels)
                .map_err(|_| self.canvas_exception("InvalidStateError"))?;
            return self.canvas_new_object("CanvasPattern", CanvasObject::Pattern(canvas::Pattern {
                bitmap, repetition, offset: canvas::Point::new(0., 0.),
            }, tainted));
        }
        if name == "drawImage" {
            return self.canvas_draw_image(node, args);
        }
        if matches!(name, "measureText" | "fillText" | "strokeText") {
            let text = self.canvas_string(args.first())?;
            let system = CanvasTextSystem(self.state.borrow().canvas_text_system.clone()
                .unwrap_or_else(|| Rc::new(amazing_text::FallbackTextSystem::new())));
            if name == "measureText" {
                let metrics_result = self.state.borrow().canvas_slots.get(&node).unwrap().context
                    .measure_text(&text, &system);
                let metrics = metrics_result.map_err(|_| self.canvas_exception("InvalidStateError"))?;
                return self.canvas_new_object("TextMetrics", CanvasObject::TextMetrics(metrics));
            }
            let Some(v) = self.canvas_numbers(&args[1..], 2)? else { return Ok(JsValue::Undefined); };
            self.canvas_refresh_styles(node);
            let max_width = if let Some(value) = args.get(3) { Some(self.js.binding_to_number(value)?) } else { None };
            let result = {
                let mut state = self.state.borrow_mut();
                let ctx = &mut state.canvas_slots.get_mut(&node).unwrap().context;
                if name == "fillText" { ctx.fill_text(&text, v[0], v[1], max_width, &system) }
                else { ctx.stroke_text(&text, v[0], v[1], max_width, &system) }
            };
            result.map_err(|_| self.canvas_exception("InvalidStateError"))?;
            self.state.borrow_mut().canvas_dirty.insert(node);
            return Ok(JsValue::Undefined);
        }
        if name == "fill" || name == "clip" || name == "stroke" {
            if name != "clip" { self.canvas_refresh_styles(node); }
            let path = args.first().and_then(|arg| self.canvas_object_index(arg)).and_then(|index|
                match &self.state.borrow().canvas_objects[index].1 { CanvasObject::Path(path) => Some(path.clone()), _ => None });
            let rule = if args.iter().any(|arg| self.js.binding_string_units(arg).is_some_and(|s| String::from_utf16_lossy(&s) == "evenodd")) {
                FillRule::EvenOdd
            } else { FillRule::NonZero };
            let mut state = self.state.borrow_mut();
            let context = &mut state.canvas_slots.get_mut(&node).unwrap().context;
            match (name, path) {
                ("fill", Some(path)) => context.fill_path(&path, rule),
                ("fill", None) => context.fill(rule),
                ("stroke", Some(path)) => context.stroke_path(&path),
                ("stroke", None) => context.stroke(),
                ("clip", Some(path)) => context.clip_path(&path, rule),
                ("clip", None) => context.clip(rule),
                _ => unreachable!(),
            }
            if name != "clip" { state.canvas_dirty.insert(node); }
            return Ok(JsValue::Undefined);
        }
        if name == "isPointInPath" || name == "isPointInStroke" {
            let (path, offset) = if let Some(first) = args.first() {
                if let Some(index) = self.canvas_object_index(first) {
                    if let CanvasObject::Path(path) = &self.state.borrow().canvas_objects[index].1 { (Some(path.clone()), 1) }
                    else { (None, 0) }
                } else { (None, 0) }
            } else { (None, 0) };
            let Some(v) = self.canvas_numbers(&args[offset..], 2)? else { return Ok(JsValue::Bool(false)); };
            let state = self.state.borrow();
            let context = &state.canvas_slots.get(&node).unwrap().context;
            let yes = match (name, path) {
                ("isPointInPath", Some(path)) => context.is_point_in_path2d(&path, v[0], v[1], FillRule::NonZero),
                ("isPointInPath", None) => context.is_point_in_path(v[0], v[1], FillRule::NonZero),
                (_, Some(path)) => context.is_point_in_stroke2d(&path, v[0], v[1]),
                (_, None) => context.is_point_in_stroke(v[0], v[1]),
            };
            return Ok(JsValue::Bool(yes));
        }
        if name == "getLineDash" {
            let dash = self.state.borrow().canvas_slots.get(&node).unwrap().context.state.line_dash.clone();
            return self.js.binding_create_array(dash.into_iter().map(JsValue::num).collect());
        }
        if name == "setLineDash" {
            let array = args.first().ok_or_else(|| Throw::type_error("Missing line dash"))?;
            let length_value = self.js.binding_get_property(array, "length")?;
            let length = self.js.binding_to_number(&length_value)? as usize;
            let mut dash = Vec::with_capacity(length.min(4096));
            for i in 0..length.min(4096) {
                let item = self.js.binding_get_property(array, &i.to_string())?;
                dash.push(self.js.binding_to_number(&item)?);
            }
            self.state.borrow_mut().canvas_slots.get_mut(&node).unwrap().context.set_line_dash(&dash);
            return Ok(JsValue::Undefined);
        }
        if name == "getTransform" {
            let matrix = self.state.borrow().canvas_slots.get(&node).unwrap().context.state.transform;
            let object = self.js.binding_create_object(Some(&self.js.binding_object_prototype()))?;
            for (field, value) in [("a", matrix.a), ("b", matrix.b), ("c", matrix.c),
                ("d", matrix.d), ("e", matrix.e), ("f", matrix.f)] {
                self.js.binding_define_data_property(&object, field, JsValue::num(value), BindingPropertyFlags::METHOD)?;
            }
            return Ok(object);
        }
        if name == "setTransform" && args.len() == 1 {
            let object = &args[0];
            let mut values = [1., 0., 0., 1., 0., 0.];
            for (index, field) in ["a", "b", "c", "d", "e", "f"].iter().enumerate() {
                let value = self.js.binding_get_property(object, field)?;
                if !matches!(value, JsValue::Undefined) {
                    values[index] = self.js.binding_to_number(&value)?;
                }
            }
            let transform = Transform { a: values[0], b: values[1], c: values[2],
                d: values[3], e: values[4], f: values[5] };
            if transform.finite() {
                self.state.borrow_mut().canvas_slots.get_mut(&node).unwrap().context.set_transform(transform);
            }
            return Ok(JsValue::Undefined);
        }
        let count = match name {
            "scale" | "translate" | "moveTo" | "lineTo" => 2,
            "rotate" => 1,
            "transform" => 6,
            "setTransform" if args.is_empty() => 0,
            "setTransform" => 6,
            "clearRect" | "fillRect" | "strokeRect" | "rect" | "quadraticCurveTo" => 4,
            "bezierCurveTo" | "ellipse" => if name == "ellipse" { 7 } else { 6 },
            "arcTo" | "arc" => 5,
            "roundRect" => 4,
            _ => 0,
        };
        let Some(v) = self.canvas_numbers(args, count)? else { return Ok(JsValue::Undefined); };
        let radii = if name == "roundRect" { self.canvas_round_radii(args.get(4))? } else { [0.; 4] };
        if matches!(name, "fillRect" | "strokeRect") { self.canvas_refresh_styles(node); }
        if (name == "arc" && v[2] < 0.) || (name == "arcTo" && v[4] < 0.) ||
            (name == "ellipse" && (v[2] < 0. || v[3] < 0.)) {
            return Err(self.canvas_exception("IndexSizeError"));
        }
        let mut state = self.state.borrow_mut();
        let context = &mut state.canvas_slots.get_mut(&node).unwrap().context;
        match name {
            "save" => context.save(), "restore" => context.restore(),
            "reset" => { let (w, h) = (context.surface.width, context.surface.height); state.canvas_slots.get_mut(&node).unwrap().reset(w, h); state.canvas_style_values.retain(|(key, _), _| *key != node); },
            "scale" => context.scale(v[0], v[1]), "rotate" => context.rotate(v[0]),
            "translate" => context.translate(v[0], v[1]),
            "transform" => context.transform(Transform { a:v[0], b:v[1], c:v[2], d:v[3], e:v[4], f:v[5] }),
            "setTransform" => context.set_transform(if v.is_empty() { Transform::IDENTITY } else { Transform { a:v[0], b:v[1], c:v[2], d:v[3], e:v[4], f:v[5] } }),
            "resetTransform" => context.reset_transform(),
            "beginPath" => context.begin_path(), "closePath" => context.close_path(),
            "moveTo" => context.move_to(v[0], v[1]), "lineTo" => context.line_to(v[0], v[1]),
            "quadraticCurveTo" => context.quadratic_curve_to(v[0], v[1], v[2], v[3]),
            "bezierCurveTo" => context.bezier_curve_to(v[0], v[1], v[2], v[3], v[4], v[5]),
            "rect" => context.rect(v[0], v[1], v[2], v[3]),
            "arc" => context.arc(v[0], v[1], v[2], v[3], v[4], args.get(5).is_some_and(|v| self.js.binding_to_boolean(v))),
            "arcTo" => context.arc_to(v[0], v[1], v[2], v[3], v[4]),
            "ellipse" => context.ellipse(v[0], v[1], v[2], v[3], v[4], v[5], v[6], args.get(7).is_some_and(|v| self.js.binding_to_boolean(v))),
            "roundRect" => context.round_rect(v[0], v[1], v[2], v[3], radii),
            "clearRect" => context.clear_rect(v[0], v[1], v[2], v[3]),
            "fillRect" => context.fill_rect(v[0], v[1], v[2], v[3]),
            "strokeRect" => context.stroke_rect(v[0], v[1], v[2], v[3]),
            _ => return Err(Throw::type_error("Canvas operation is unsupported")),
        }
        if matches!(name, "clearRect" | "fillRect" | "strokeRect" | "reset") {
            state.canvas_dirty.insert(node);
        }
        Ok(JsValue::Undefined)
    }
}

impl RuntimeBindingHost<'_> {
    fn canvas_context_attribute(&mut self, node: NodeIdx, member: Member, args: &[JsValue]) -> Result<JsValue, Throw> {
        let name = member.name;
        if member.kind == CanvasMemberKind::Setter {
            let value = args.first().cloned().unwrap_or(JsValue::Undefined);
            if name == "fillStyle" || name == "strokeStyle" {
                let style = if let Some(index) = self.canvas_object_index(&value) {
                    match &self.state.borrow().canvas_objects[index].1 {
                        CanvasObject::Gradient(g, kind) => Some(match kind {
                            CanvasGradientKind::Linear => PaintStyle::LinearGradient(g.clone()),
                            CanvasGradientKind::Radial => PaintStyle::RadialGradient(g.clone()),
                            CanvasGradientKind::Conic => PaintStyle::ConicGradient(g.clone()),
                        }),
                        CanvasObject::Pattern(p, _) => Some(PaintStyle::Pattern(p.clone())),
                        _ => None,
                    }
                } else { PaintStyle::from_css_color(&self.canvas_string(Some(&value))?) };
                if let Some(style) = style {
                    let reflected = match &style {
                        PaintStyle::Color(color) => JsValue::str(canvas_color_text(*color)),
                        _ => value,
                    };
                    let mut state = self.state.borrow_mut();
                    let ctx = &mut state.canvas_slots.get_mut(&node).unwrap().context;
                    if name == "fillStyle" { ctx.state.fill_style = style; } else { ctx.state.stroke_style = style; }
                    state.canvas_style_values.insert((node, name), reflected);
                }
                return Ok(JsValue::Undefined);
            }
            if matches!(name, "lineCap" | "lineJoin" | "globalCompositeOperation" | "textAlign" | "textBaseline" | "filter" | "imageSmoothingQuality" | "direction" | "font" | "shadowColor" | "lang" | "letterSpacing" | "fontKerning" | "fontStretch" | "fontVariantCaps" | "textRendering" | "wordSpacing") {
                let value = self.canvas_string(Some(&value))?;
                let mut state = self.state.borrow_mut();
                let ctx = &mut state.canvas_slots.get_mut(&node).unwrap().context;
                let mut store = false;
                match name {
                    "lineCap" => if let Some(cap) = match value.as_str() { "butt" => Some(canvas::LineCap::Butt), "round" => Some(canvas::LineCap::Round), "square" => Some(canvas::LineCap::Square), _ => None } { ctx.state.line_cap = cap; },
                    "lineJoin" => if let Some(join) = match value.as_str() { "miter" => Some(canvas::LineJoin::Miter), "round" => Some(canvas::LineJoin::Round), "bevel" => Some(canvas::LineJoin::Bevel), _ => None } { ctx.state.line_join = join; },
                    "globalCompositeOperation" => if let Some(op) = canvas::CompositeOperation::parse(&value) { ctx.state.composite_operation = op; },
                    "textAlign" => if let Some(align) = match value.as_str() { "start" => Some(canvas::TextAlign::Start), "end" => Some(canvas::TextAlign::End), "left" => Some(canvas::TextAlign::Left), "right" => Some(canvas::TextAlign::Right), "center" => Some(canvas::TextAlign::Center), _ => None } { ctx.state.text_align = align; },
                    "textBaseline" => if let Some(base) = match value.as_str() { "top" => Some(canvas::TextBaseline::Top), "hanging" => Some(canvas::TextBaseline::Hanging), "middle" => Some(canvas::TextBaseline::Middle), "alphabetic" => Some(canvas::TextBaseline::Alphabetic), "ideographic" => Some(canvas::TextBaseline::Ideographic), "bottom" => Some(canvas::TextBaseline::Bottom), _ => None } { ctx.state.text_baseline = base; },
                    "filter" => { ctx.set_filter(&value); },
                    "shadowColor" => if let Some(PaintStyle::Color(color)) = PaintStyle::from_css_color(&value) {
                        ctx.state.shadow_color = color;
                        store = true;
                    },
                    "font" => {
                        if let Some(size) = value.split_whitespace().find_map(|part| part.strip_suffix("px").and_then(|s| s.parse::<f32>().ok())) {
                            if size.is_finite() && size > 0. {
                                ctx.state.text_style.size = size;
                                store = true;
                            }
                        }
                    },
                    "direction" => if matches!(value.as_str(), "ltr" | "rtl" | "inherit") {
                        ctx.state.text_style.direction = if value == "rtl" { amazing_text::Direction::RightToLeft }
                            else { amazing_text::Direction::LeftToRight };
                        store = true;
                    },
                    "imageSmoothingQuality" => store = matches!(value.as_str(), "low" | "medium" | "high"),
                    "fontKerning" => store = matches!(value.as_str(), "auto" | "normal" | "none"),
                    "fontStretch" => store = matches!(value.as_str(), "ultra-condensed" | "extra-condensed" | "condensed" | "semi-condensed" | "normal" | "semi-expanded" | "expanded" | "extra-expanded" | "ultra-expanded"),
                    "fontVariantCaps" => store = matches!(value.as_str(), "normal" | "small-caps" | "all-small-caps" | "petite-caps" | "all-petite-caps" | "unicase" | "titling-caps"),
                    "textRendering" => store = matches!(value.as_str(), "auto" | "optimizeSpeed" | "optimizeLegibility" | "geometricPrecision"),
                    "lang" | "letterSpacing" | "wordSpacing" => store = true,
                    _ => unreachable!(),
                }
                if store { state.canvas_style_values.insert((node, name), JsValue::str(value)); }
                return Ok(JsValue::Undefined);
            }
            if name == "imageSmoothingEnabled" {
                self.state.borrow_mut().canvas_slots.get_mut(&node).unwrap().context.state.image_smoothing_enabled = self.js.binding_to_boolean(&value);
                return Ok(JsValue::Undefined);
            }
            let n = self.js.binding_to_number(&value)?;
            if !n.is_finite() { return Ok(JsValue::Undefined); }
            let mut state = self.state.borrow_mut();
            let ctx = &mut state.canvas_slots.get_mut(&node).unwrap().context;
            match name {
                "globalAlpha" => ctx.set_global_alpha(n),
                "lineWidth" => ctx.set_line_width(n),
                "miterLimit" if n > 0. => ctx.state.miter_limit = n,
                "lineDashOffset" => ctx.state.line_dash_offset = n,
                "shadowBlur" if n >= 0. => ctx.state.shadow_blur = n,
                "shadowOffsetX" => ctx.state.shadow_offset_x = n,
                "shadowOffsetY" => ctx.state.shadow_offset_y = n,
                _ => {},
            }
            return Ok(JsValue::Undefined);
        }
        let state = self.state.borrow();
        let ctx = &state.canvas_slots.get(&node).unwrap().context;
        let style = if name == "fillStyle" { Some(&ctx.state.fill_style) } else if name == "strokeStyle" { Some(&ctx.state.stroke_style) } else { None };
        if let Some(style) = style {
            if let Some(value) = state.canvas_style_values.get(&(node, name)) { return Ok(value.clone()); }
            return Ok(JsValue::str(match style { PaintStyle::Color(color) => canvas_color_text(*color), _ => String::new() }));
        }
        if let Some(value) = state.canvas_style_values.get(&(node, name)) {
            return Ok(value.clone());
        }
        Ok(match name {
            "globalAlpha" => JsValue::num(ctx.state.global_alpha),
            "globalCompositeOperation" => JsValue::str(canvas_composite_name(ctx.state.composite_operation)),
            "lineWidth" => JsValue::num(ctx.state.line_width),
            "lineCap" => JsValue::str(match ctx.state.line_cap { canvas::LineCap::Butt => "butt", canvas::LineCap::Round => "round", canvas::LineCap::Square => "square" }),
            "lineJoin" => JsValue::str(match ctx.state.line_join { canvas::LineJoin::Miter => "miter", canvas::LineJoin::Round => "round", canvas::LineJoin::Bevel => "bevel" }),
            "miterLimit" => JsValue::num(ctx.state.miter_limit),
            "lineDashOffset" => JsValue::num(ctx.state.line_dash_offset),
            "shadowBlur" => JsValue::num(ctx.state.shadow_blur),
            "shadowOffsetX" => JsValue::num(ctx.state.shadow_offset_x),
            "shadowOffsetY" => JsValue::num(ctx.state.shadow_offset_y),
            "imageSmoothingEnabled" => JsValue::Bool(ctx.state.image_smoothing_enabled),
            "imageSmoothingQuality" => JsValue::str("low"),
            "filter" => JsValue::str(ctx.state.filter.clone()),
            "font" => JsValue::str("10px sans-serif"),
            "lang" => JsValue::str("inherit"),
            "letterSpacing" | "wordSpacing" => JsValue::str("0px"),
            "fontKerning" | "textRendering" => JsValue::str("auto"),
            "fontStretch" | "fontVariantCaps" => JsValue::str("normal"),
            "direction" => JsValue::str("inherit"),
            "textAlign" => JsValue::str(match ctx.state.text_align { canvas::TextAlign::Start => "start", canvas::TextAlign::End => "end", canvas::TextAlign::Left => "left", canvas::TextAlign::Right => "right", canvas::TextAlign::Center => "center" }),
            "textBaseline" => JsValue::str(match ctx.state.text_baseline { canvas::TextBaseline::Top => "top", canvas::TextBaseline::Hanging => "hanging", canvas::TextBaseline::Middle => "middle", canvas::TextBaseline::Alphabetic => "alphabetic", canvas::TextBaseline::Ideographic => "ideographic", canvas::TextBaseline::Bottom => "bottom" }),
            "shadowColor" => JsValue::str("rgba(0, 0, 0, 0)"),
            _ => JsValue::Undefined,
        })
    }
}

fn canvas_composite_name(op: canvas::CompositeOperation) -> &'static str {
    use canvas::CompositeOperation as C;
    match op {
        C::SourceOver => "source-over", C::SourceIn => "source-in", C::SourceOut => "source-out", C::SourceAtop => "source-atop",
        C::DestinationOver => "destination-over", C::DestinationIn => "destination-in", C::DestinationOut => "destination-out", C::DestinationAtop => "destination-atop",
        C::Lighter => "lighter", C::Copy => "copy", C::Xor => "xor", C::Multiply => "multiply", C::Screen => "screen", C::Overlay => "overlay",
        C::Darken => "darken", C::Lighten => "lighten", C::ColorDodge => "color-dodge", C::ColorBurn => "color-burn", C::HardLight => "hard-light",
        C::SoftLight => "soft-light", C::Difference => "difference", C::Exclusion => "exclusion", C::Hue => "hue", C::Saturation => "saturation",
        C::Color => "color", C::Luminosity => "luminosity",
    }
}

impl RuntimeBindingHost<'_> {
    fn canvas_path_call(&mut self, member: Member, receiver: JsValue, args: &[JsValue]) -> Result<JsValue, Throw> {
        let index = self.canvas_object_index(&receiver).ok_or_else(|| Throw::type_error("Expected Path2D"))?;
        if !matches!(self.state.borrow().canvas_objects[index].1, CanvasObject::Path(_)) {
            return Err(Throw::type_error("Expected Path2D"));
        }
        if member.name == "addPath" {
            let source = args.first().and_then(|value| self.canvas_object_index(value))
                .and_then(|index| match &self.state.borrow().canvas_objects[index].1 { CanvasObject::Path(path) => Some(path.clone()), _ => None })
                .ok_or_else(|| Throw::type_error("Expected Path2D"))?;
            let mut matrix = Transform::IDENTITY;
            if let Some(object) = args.get(1) {
                for (name, slot) in [("a", &mut matrix.a), ("b", &mut matrix.b),
                    ("c", &mut matrix.c), ("d", &mut matrix.d),
                    ("e", &mut matrix.e), ("f", &mut matrix.f)] {
                    let value = self.js.binding_get_property(object, name)?;
                    if !matches!(value, JsValue::Undefined) { *slot = self.js.binding_to_number(&value)?; }
                }
            }
            if let CanvasObject::Path(path) = &mut self.state.borrow_mut().canvas_objects[index].1 {
                path.add_path(&source, matrix);
            }
            return Ok(JsValue::Undefined);
        }
        let count = match member.name {
            "closePath" => 0, "moveTo" | "lineTo" => 2,
            "quadraticCurveTo" | "rect" => 4, "bezierCurveTo" => 6,
            "arcTo" | "arc" => 5, "roundRect" => 4, "ellipse" => 7,
            _ => return Err(Throw::type_error("Unknown Path2D member")),
        };
        let Some(v) = self.canvas_numbers(args, count)? else { return Ok(JsValue::Undefined); };
        let radii = if member.name == "roundRect" { self.canvas_round_radii(args.get(4))? } else { [0.; 4] };
        if (member.name == "arc" && v[2] < 0.) || (member.name == "arcTo" && v[4] < 0.) ||
            (member.name == "ellipse" && (v[2] < 0. || v[3] < 0.)) {
            return Err(self.canvas_exception("IndexSizeError"));
        }
        let ccw = args.get(count).is_some_and(|value| self.js.binding_to_boolean(value));
        let mut state = self.state.borrow_mut();
        let CanvasObject::Path(path) = &mut state.canvas_objects[index].1 else { unreachable!() };
        match member.name {
            "closePath" => path.close_path(), "moveTo" => path.move_to(v[0], v[1]),
            "lineTo" => path.line_to(v[0], v[1]),
            "quadraticCurveTo" => path.quadratic_curve_to(v[0], v[1], v[2], v[3]),
            "bezierCurveTo" => path.bezier_curve_to(v[0], v[1], v[2], v[3], v[4], v[5]),
            "arcTo" => path.arc_to(v[0], v[1], v[2], v[3], v[4]),
            "rect" => path.rect(v[0], v[1], v[2], v[3]),
            "roundRect" => path.round_rect(v[0], v[1], v[2], v[3], radii),
            "arc" => path.arc(v[0], v[1], v[2], v[3], v[4], ccw),
            "ellipse" => path.ellipse(v[0], v[1], v[2], v[3], v[4], v[5], v[6], ccw),
            _ => unreachable!(),
        }
        Ok(JsValue::Undefined)
    }
    fn canvas_gradient_call(&mut self, member: Member, receiver: JsValue, args: &[JsValue]) -> Result<JsValue, Throw> {
        let index = self.canvas_object_index(&receiver).ok_or_else(|| Throw::type_error("Expected CanvasGradient"))?;
        if member.name != "addColorStop" { return Err(Throw::type_error("Unknown gradient member")); }
        let Some(v) = self.canvas_numbers(args, 1)? else { return Err(self.canvas_exception("IndexSizeError")); };
        let color = self.canvas_string(args.get(1))?;
        let result = {
            let mut state = self.state.borrow_mut();
            let CanvasObject::Gradient(gradient, _) = &mut state.canvas_objects[index].1 else { return Err(Throw::type_error("Expected CanvasGradient")); };
            gradient.add_color_stop(v[0], &color)
        };
        match result {
            Err(canvas::GradientError::InvalidOffset) => return Err(self.canvas_exception("IndexSizeError")),
            Err(canvas::GradientError::InvalidColor) => return Err(self.canvas_exception("SyntaxError")),
            Ok(()) => {},
        }
        Ok(JsValue::Undefined)
    }
    fn canvas_image_data_call(&mut self, member: Member, receiver: JsValue) -> Result<JsValue, Throw> {
        let index = self.canvas_object_index(&receiver).ok_or_else(|| Throw::type_error("Expected ImageData"))?;
        match &self.state.borrow().canvas_objects[index].1 {
            CanvasObject::ImageData(w, h, data) => Ok(match member.name {
                "width" => JsValue::num(f64::from(*w)), "height" => JsValue::num(f64::from(*h)),
                "data" => data.clone(), "colorSpace" => JsValue::str("srgb"),
                _ => JsValue::Undefined,
            }),
            _ => Err(Throw::type_error("Expected ImageData")),
        }
    }
    fn canvas_metrics_call(&mut self, member: Member, receiver: JsValue) -> Result<JsValue, Throw> {
        let index = self.canvas_object_index(&receiver).ok_or_else(|| Throw::type_error("Expected TextMetrics"))?;
        let state = self.state.borrow();
        let CanvasObject::TextMetrics(metrics) = &state.canvas_objects[index].1 else { return Err(Throw::type_error("Expected TextMetrics")); };
        let value = match member.name {
            "width" => metrics.width,
            "actualBoundingBoxLeft" => metrics.actual_bounding_box_left,
            "actualBoundingBoxRight" => metrics.actual_bounding_box_right,
            "actualBoundingBoxAscent" => metrics.actual_bounding_box_ascent,
            "actualBoundingBoxDescent" => metrics.actual_bounding_box_descent,
            "fontBoundingBoxAscent" => metrics.font_bounding_box_ascent,
            "fontBoundingBoxDescent" => metrics.font_bounding_box_descent,
            _ => 0.,
        };
        Ok(JsValue::num(value))
    }
    fn canvas_draw_image(&mut self, node: NodeIdx, args: &[JsValue]) -> Result<JsValue, Throw> {
        let source = args.first().ok_or_else(|| Throw::type_error("drawImage needs a source"))?;
        let source_node = self.node_brand(source).ok_or_else(|| Throw::type_error("Unsupported image source"))?;
        let source_name = {
            let host = self.state.borrow().host.clone();
            let name = host.borrow().dom.name(source_node).unwrap_or("").to_owned();
            name
        };
        let (bitmap, tainted) = if source_name == "canvas" {
            let source = self.state.borrow().canvas_slots.get(&source_node).map(|slot|
                (slot.context.surface.width, slot.context.surface.height,
                    slot.context.surface.pixels.clone(), !slot.origin_clean));
            let source = source.or_else(|| {
                let (width, height) = (self.canvas_dimension(source_node, "width"), self.canvas_dimension(source_node, "height"));
                (width <= 8192 && height <= 8192 && u64::from(width) * u64::from(height) <= 16_777_216)
                    .then(|| (width, height, vec![0; width as usize * height as usize * 4], false))
            });
            let (width, height, pixels, tainted) = source.ok_or_else(|| self.canvas_exception("InvalidStateError"))?;
            (canvas::Bitmap::new(width, height, pixels)
                .map_err(|_| self.canvas_exception("InvalidStateError"))?, tainted)
        } else if source_name == "img" {
            let source_url = {
                let host = self.state.borrow().host.clone();
                let host = host.borrow();
                host.dom.get_attribute_ns(source_node, None, "src").unwrap_or("").to_owned()
            };
            if source_url.is_empty() { return Err(self.canvas_exception("InvalidStateError")); }
            let page = self.state.borrow().document_url.clone().unwrap_or_default();
            let resolved = crate::cmd::js::resolve_script_url(&page, &source_url);
            let bytes = if resolved.get(..5).is_some_and(|prefix| prefix.eq_ignore_ascii_case("data:")) {
                let url = amazing_net::ParsedUrl::parse(&resolved).map_err(|_| self.canvas_exception("InvalidStateError"))?;
                amazing_fetch::process_data_url(&url).ok_or_else(|| self.canvas_exception("InvalidStateError"))?.body
            } else {
                amazing_net::fetch(&resolved, &amazing_net::FetchOptions { max_response_bytes: 16 * 1024 * 1024, ..Default::default() })
                    .map_err(|_| self.canvas_exception("InvalidStateError"))?.body
            };
            let image = amazing_media::decode_image(&bytes).map_err(|_| self.canvas_exception("InvalidStateError"))?;
            let tainted = if resolved.starts_with("data:") { false } else {
                match (amazing_net::ParsedUrl::parse(&page), amazing_net::ParsedUrl::parse(&resolved)) {
                    (Ok(page), Ok(source)) => page.origin().serialize() != source.origin().serialize(),
                    _ => false,
                }
            };
            (canvas::Bitmap::new(image.width, image.height, image.pixels)
                .map_err(|_| self.canvas_exception("InvalidStateError"))?, tainted)
        } else { return Err(Throw::type_error("Unsupported image source")); };
        let count = match args.len() { 3..=4 => 2, 5..=8 => 4, 9.. => 8, _ => return Err(Throw::type_error("Invalid drawImage overload")) };
        let Some(v) = self.canvas_numbers(&args[1..], count)? else { return Ok(JsValue::Undefined); };
        let mut state = self.state.borrow_mut();
        if tainted {
            state.canvas_slots.get_mut(&node).unwrap().origin_clean = false;
        }
        let context = &mut state.canvas_slots.get_mut(&node).unwrap().context;
        match count {
            2 => context.draw_image(&bitmap, v[0], v[1]),
            4 => context.draw_image_scaled(&bitmap, v[0], v[1], v[2], v[3]),
            _ => context.draw_image_full(&bitmap, v[0], v[1], v[2], v[3], v[4], v[5], v[6], v[7]),
        }
        state.canvas_dirty.insert(node);
        Ok(JsValue::Undefined)
    }
}

/// Sized adapter for the driver-selected shaping/raster provider. Canvas2D's
/// text methods accept a concrete `impl TextSystem`.
struct CanvasTextSystem(Rc<dyn amazing_text::TextSystem>);
impl amazing_text::TextSystem for CanvasTextSystem {
    fn generation(&self) -> u64 { self.0.generation() }
    fn metrics(&self, style: &amazing_text::TextStyle) -> Result<amazing_text::FontMetrics, amazing_text::TextError> {
        self.0.metrics(style)
    }
    fn font_relative_metrics(&self, style: &amazing_text::TextStyle) -> amazing_text::FontRelativeMetrics {
        self.0.font_relative_metrics(style)
    }
    fn shape(&self, paragraph: amazing_text::Paragraph<'_>) -> Result<amazing_text::ShapedParagraph, amazing_text::TextError> {
        self.0.shape(paragraph)
    }
    fn shape_line(&self, request: amazing_text::LineRequest<'_>) -> Result<amazing_text::ShapedLine, amazing_text::TextError> {
        self.0.shape_line(request)
    }
    fn rasterize(&self, request: amazing_text::GlyphRasterRequest<'_>) -> Result<Option<amazing_text::GlyphImage>, amazing_text::TextError> {
        self.0.rasterize(request)
    }
}

impl RuntimeBindingHost<'_> {
    fn canvas_pattern_call(&mut self, member: Member, receiver: JsValue, args: &[JsValue]) -> Result<JsValue, Throw> {
        if member.name != "setTransform" { return Err(Throw::type_error("Unknown CanvasPattern member")); }
        let index = self.canvas_object_index(&receiver).ok_or_else(|| Throw::type_error("Expected CanvasPattern"))?;
        let mut offset = [0., 0.];
        if let Some(object) = args.first() {
            for (index, name) in ["e", "f"].iter().enumerate() {
                let value = self.js.binding_get_property(object, name)?;
                if !matches!(value, JsValue::Undefined) {
                    offset[index] = self.js.binding_to_number(&value)?;
                }
            }
        }
        if offset.iter().all(|n| n.is_finite()) {
            let mut state = self.state.borrow_mut();
            let CanvasObject::Pattern(pattern, _) = &mut state.canvas_objects[index].1 else {
                return Err(Throw::type_error("Expected CanvasPattern"));
            };
            pattern.offset = canvas::Point::new(offset[0], offset[1]);
        }
        Ok(JsValue::Undefined)
    }
}

impl RuntimeBindingHost<'_> {
    fn canvas_round_radii(&mut self, value: Option<&JsValue>) -> Result<[f64; 4], Throw> {
        let Some(value) = value else { return Ok([0.; 4]); };
        let values = if matches!(value, JsValue::Object(_)) {
            let length = self.js.binding_get_property(value, "length")?;
            if !matches!(length, JsValue::Undefined) {
                let length = self.js.binding_to_number(&length)? as usize;
                if !(1..=4).contains(&length) { return Err(Throw::range_error("roundRect radii length")); }
                let mut radii = Vec::with_capacity(length);
                for i in 0..length {
                    let item = self.js.binding_get_property(value, &i.to_string())?;
                    radii.push(self.js.binding_to_number(&item)?);
                }
                radii
            } else {
                let x = self.js.binding_get_property(value, "x")?;
                vec![if matches!(x, JsValue::Undefined) { 0. } else { self.js.binding_to_number(&x)? }]
            }
        } else { vec![self.js.binding_to_number(value)?] };
        if values.iter().any(|v| !v.is_finite() || *v < 0.) {
            return Err(Throw::range_error("Invalid roundRect radius"));
        }
        Ok(match values.as_slice() {
            [a] => [*a; 4],
            [a, b] => [*a, *b, *a, *b],
            [a, b, c] => [*a, *b, *c, *b],
            [a, b, c, d] => [*a, *b, *c, *d],
            _ => unreachable!(),
        })
    }
}

fn canvas_color_text(color: [u8; 4]) -> String {
    if color[3] == 255 {
        format!("#{:02x}{:02x}{:02x}", color[0], color[1], color[2])
    } else {
        let alpha = (f64::from(color[3]) / 255.).to_string();
        format!("rgba({}, {}, {}, {})", color[0], color[1], color[2], alpha)
    }
}
