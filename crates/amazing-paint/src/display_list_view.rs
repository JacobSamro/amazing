//! Read-only paint items and property state for tiled compositors.
//!
//! Indices and property IDs are stable for one built view. Rebuild the view
//! after layout or style changes; no layout ownership escapes through items.

use std::ops::Range;
use std::collections::HashMap;

use amazing_dom::NodeIdx;
use amazing_layout::{Flow, LayoutBox};
use amazing_text::TextSystem;

use crate::controls::ControlProvider;
use crate::display_list::{build_display_list, Affine, PaintCommand};
use crate::paint::{self, ImageProvider};
use crate::surface::Surface;

#[path = "display_list_delta.rs"]
mod delta;
pub use delta::{PaintFallbackReason, PaintItemChange, PaintResourceKey, PaintViewDelta, PaintViewUpdate};
#[path = "retained_paint.rs"]
mod retained;
pub use retained::{PaintDamage, RetainedPaintDelta, RetainedPaintView, RetainedRaster};

/// An RGBA8 destination for rasterized paint items.
pub type RgbaSurface = Surface;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawRasterError {
    InvalidRange,
    InvalidTile,
    TileTooLarge,
    /// This item requires the existing baked-pixel path until it can be
    /// separated from its command context without changing its pixels.
    UnseparableItem(usize),
    RasterFailed(usize),
}

/// Bounds in untransformed document paint coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PaintBounds {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl PaintBounds {
    fn from_layout(rect: amazing_layout::Rect) -> Self {
        Self {
            x: rect.x,
            y: rect.y,
            width: rect.w,
            height: rect.h,
        }
    }

    fn viewport(width: u32, height: u32) -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            width: width as f32,
            height: height as f32,
        }
    }

    fn union(self, other: Self) -> Self {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        Self {
            x,
            y,
            width: (self.x + self.width).max(other.x + other.width) - x,
            height: (self.y + self.height).max(other.y + other.height) - y,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PaintItemKind {
    Decorations,
    Content,
    Control,
}

/// Identity supplied by the DOM journal. The generation must change when an
/// arena slot is reused; a document-wide mutation counter is not sufficient.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PaintNodeKey {
    pub node: NodeIdx,
    pub generation: u64,
}

/// A paint item keeps its identity when unrelated siblings are rebuilt.
/// `ordinal` distinguishes multiple boxes of the same phase for one node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StablePaintItemKey {
    pub node: PaintNodeKey,
    pub phase: PaintItemKind,
    pub ordinal: u32,
}

impl StablePaintItemKey {
    /// Deterministic 64-bit adapter for the compositor's `PaintItemKey(u64)`.
    /// The update seam checks for collisions before publishing a delta.
    pub fn compositor_id(self) -> u64 {
        let mut hash = 0xcbf29ce484222325u64;
        for byte in self.node.node.0.to_le_bytes().into_iter()
            .chain(self.node.generation.to_le_bytes())
            .chain([self.phase as u8])
            .chain(self.ordinal.to_le_bytes())
        {
            hash = (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3);
        }
        hash
    }
}

/// Parent IDs are zero for the root state. IDs index the corresponding node
/// table; they are not DOM IDs and do not survive a view rebuild.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PropertyState {
    pub transform: u32,
    pub clip: u32,
    pub effect: u32,
    pub scroll: u32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CompositingHints {
    pub will_change: bool,
    pub fixed: bool,
    pub sticky: bool,
    pub video: bool,
    pub scroller: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PaintItem {
    pub index: usize,
    /// `None` requires a full rebuild: the adapter did not provide an
    /// unambiguous DOM generation for this item's originating node.
    pub key: Option<StablePaintItemKey>,
    pub kind: PaintItemKind,
    /// Conservative bounds before the item's property transform. Items whose
    /// ink may escape their box use the viewport as a safe bound.
    pub local_bounds: PaintBounds,
    pub properties: PropertyState,
    pub hints: CompositingHints,
    /// Sticky position limits in untransformed document coordinates.
    pub sticky: Option<StickyMetadata>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StickyMetadata {
    pub normal: PaintBounds,
    pub containing_block: PaintBounds,
    pub scroll: u32,
    pub top: Option<f32>,
    pub right: Option<f32>,
    pub bottom: Option<f32>,
    pub left: Option<f32>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TransformNode {
    pub parent: u32,
    /// Stable DOM owner; absent only for the synthetic root in a bindable view.
    pub owner: Option<PaintNodeKey>,
    /// CSS 2D affine matrix [a, b, c, d, e, f], including transform origin.
    pub matrix: [f32; 6],
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClipNode {
    pub parent: u32,
    pub bounds: PaintBounds,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EffectNode {
    pub parent: u32,
    /// Stable DOM owner; absent only for the synthetic root in a bindable view.
    pub owner: Option<PaintNodeKey>,
    pub opacity: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScrollNode {
    pub parent: u32,
    /// Stable DOM owner; absent only for the synthetic root in a bindable view.
    pub owner: Option<PaintNodeKey>,
    pub bounds: PaintBounds,
    /// Untransformed scrollable overflow, including the scrollport itself.
    pub contents: PaintBounds,
    /// Layout does not yet supply a live scroll offset; newly built views start at zero.
    pub initial_offset: (f32, f32),
}

/// A borrowed view of the painter's ordered commands. All layout references
/// remain private; callers receive only value records and rasterized pixels.
pub struct DisplayListView<'a> {
    commands: Vec<PaintCommand<'a>>,
    item_commands: Vec<usize>,
    items: Vec<PaintItem>,
    node_generations: HashMap<NodeIdx, u64>,
    ambiguous_generation: bool,
    transforms: Vec<TransformNode>,
    clips: Vec<ClipNode>,
    effects: Vec<EffectNode>,
    scrolls: Vec<ScrollNode>,
    width: u32,
    height: u32,
    text: &'a dyn TextSystem,
    images: &'a dyn ImageProvider,
    controls: &'a dyn ControlProvider,
}

/// Per-property lookup into one view's property tables. IDs are valid only
/// while this view is retained. `None` from `property_owner_mapping` means the
/// caller must rebuild the scene instead of binding a property update.
#[derive(Debug, Clone)]
pub struct PropertyOwnerMapping {
    pub transforms: HashMap<PaintNodeKey, u32>,
    pub effects: HashMap<PaintNodeKey, u32>,
    pub scrolls: HashMap<PaintNodeKey, u32>,
}

impl<'a> DisplayListView<'a> {
    pub fn new(
        root: &'a LayoutBox,
        width: u32,
        height: u32,
        text: &'a dyn TextSystem,
        images: &'a dyn ImageProvider,
        controls: &'a dyn ControlProvider,
    ) -> Self {
        Self::with_video_hint(root, width, height, text, images, controls, &|_| false)
    }

    /// The adapter knows element names; paint only knows layout geometry.
    pub fn with_video_hint(
        root: &'a LayoutBox,
        width: u32,
        height: u32,
        text: &'a dyn TextSystem,
        images: &'a dyn ImageProvider,
        controls: &'a dyn ControlProvider,
        is_video: &dyn Fn(NodeIdx) -> bool,
    ) -> Self {
        Self::with_scroll_offsets(root, width, height, text, images, controls, is_video, &|_| None)
    }

    /// The embedding adapter can supply live scroll positions without adding
    /// mutable DOM state to the borrowed layout tree.
    pub fn with_scroll_offsets(
        root: &'a LayoutBox,
        width: u32,
        height: u32,
        text: &'a dyn TextSystem,
        images: &'a dyn ImageProvider,
        controls: &'a dyn ControlProvider,
        is_video: &dyn Fn(NodeIdx) -> bool,
        scroll_offset: &dyn Fn(NodeIdx) -> Option<(f32, f32)>,
    ) -> Self {
        Self::with_node_generations_and_scroll_offsets(
            root, width, height, text, images, controls, is_video, scroll_offset, &|_| None,
        )
    }

    /// The embedding adapter supplies per-node arena generations from its
    /// frozen DOM epoch. Missing generations deliberately prevent deltas.
    pub fn with_node_generations(
        root: &'a LayoutBox,
        width: u32,
        height: u32,
        text: &'a dyn TextSystem,
        images: &'a dyn ImageProvider,
        controls: &'a dyn ControlProvider,
        generation: &dyn Fn(NodeIdx) -> Option<u64>,
    ) -> Self {
        Self::with_node_generations_and_scroll_offsets(
            root, width, height, text, images, controls, &|_| false, &|_| None, generation,
        )
    }

    /// Convenience for the retained DOM journal's arena generations. A
    /// retired or unknown node returns `None` and forces the safe full path.
    pub fn with_dom_generations(
        root: &'a LayoutBox,
        width: u32,
        height: u32,
        text: &'a dyn TextSystem,
        images: &'a dyn ImageProvider,
        controls: &'a dyn ControlProvider,
        dom: &amazing_dom::Dom,
    ) -> Self {
        Self::with_node_generations(
            root, width, height, text, images, controls, &|node| dom.node_generation(node),
        )
    }

    /// Combined adapter entry point for retained scroll state and identity.
    pub fn with_node_generations_and_scroll_offsets(
        root: &'a LayoutBox,
        width: u32,
        height: u32,
        text: &'a dyn TextSystem,
        images: &'a dyn ImageProvider,
        controls: &'a dyn ControlProvider,
        is_video: &dyn Fn(NodeIdx) -> bool,
        scroll_offset: &dyn Fn(NodeIdx) -> Option<(f32, f32)>,
        generation: &dyn Fn(NodeIdx) -> Option<u64>,
    ) -> Self {
        let commands = build_display_list(root);
        let mut containing_blocks = HashMap::new();
        collect_containing_blocks(root, PaintBounds::viewport(width, height), &mut containing_blocks);
        let root_owner = root.node.and_then(|node| generation(node)
            .map(|generation| PaintNodeKey { node, generation }));
        let mut view = Self {
            commands,
            item_commands: Vec::new(),
            items: Vec::new(),
            node_generations: HashMap::new(),
            ambiguous_generation: root.node.is_some() && root_owner.is_none(),
            transforms: vec![TransformNode {
                parent: 0,
                owner: None,
                matrix: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            }],
            clips: vec![ClipNode {
                parent: 0,
                bounds: PaintBounds::viewport(width, height),
            }],
            effects: vec![EffectNode {
                parent: 0,
                owner: None,
                opacity: 1.0,
            }],
            scrolls: vec![
                ScrollNode {
                    parent: 0,
                    owner: None,
                    bounds: PaintBounds::viewport(width, height),
                    contents: PaintBounds::viewport(width, height),
                    initial_offset: (0.0, 0.0),
                },
                // The layout viewport scrolls the document even when no element
                // has an explicit CSS overflow declaration. The layout root is
                // the body (or html when body is absent), so its overflow is
                // represented here rather than as a second element scroller.
                ScrollNode {
                    parent: 0,
                    owner: root_owner,
                    bounds: PaintBounds::viewport(width, height),
                    contents: scrollable_extent(root, PaintBounds::viewport(width, height), true),
                    initial_offset: offset_for(root, scroll_offset),
                },
            ],
            width,
            height,
            text,
            images,
            controls,
        };
        let mut state = PropertyState { scroll: 1, ..PropertyState::default() };
        let mut group_stack = Vec::new();
        let mut clip_stack = Vec::new();
        let mut ordinals = HashMap::new();
        // One CSS scroll container can open more than one clip scope in a
        // single display list: a positioned/stacking context re-opens the
        // overflow clips it inherits. That is still one compositor scroll
        // node, so reuse its ID instead of registering a duplicate owner.
        let mut scroll_owners: HashMap<PaintNodeKey, u32> = HashMap::new();
        for command_index in 0..view.commands.len() {
            let mut item = None;
            match &view.commands[command_index] {
                PaintCommand::BeginGroup(group) => {
                    group_stack.push(state);
                    let owner = group.owner.and_then(|node| generation(node).map(|generation| PaintNodeKey { node, generation }));
                    let next = view.effects.len() as u32;
                    view.effects.push(EffectNode {
                        parent: state.effect,
                        owner,
                        opacity: group.opacity,
                    });
                    state.effect = next;
                    if group.transform != Affine::IDENTITY {
                        let matrix = group.matrix_about_origin();
                        let next = view.transforms.len() as u32;
                        view.transforms.push(TransformNode {
                            parent: state.transform,
                            owner,
                            matrix: [matrix.a, matrix.b, matrix.c, matrix.d, matrix.e, matrix.f],
                        });
                        state.transform = next;
                    }
                }
                PaintCommand::EndGroup => {
                    if let Some(previous) = group_stack.pop() {
                        state.transform = previous.transform;
                        state.effect = previous.effect;
                    }
                }
                PaintCommand::PushOverflowClip(box_) => {
                    clip_stack.push(state);
                    let bounds = if std::ptr::eq(*box_, root) {
                        PaintBounds::viewport(width, height)
                    } else {
                        clip_bounds(box_, width, height)
                    };
                    let next = view.clips.len() as u32;
                    view.clips.push(ClipNode {
                        parent: state.clip,
                        bounds,
                    });
                    state.clip = next;
                    if is_scroller(box_) && !std::ptr::eq(*box_, root) {
                        let owner = box_.node.and_then(|node| generation(node).map(|generation| PaintNodeKey { node, generation }));
                        let next = match owner.and_then(|owner| scroll_owners.get(&owner).copied()) {
                            Some(existing) => existing,
                            None => {
                                let next = view.scrolls.len() as u32;
                                view.scrolls.push(ScrollNode {
                                    parent: state.scroll,
                                    owner,
                                    bounds,
                                    contents: scrollable_extent(box_, bounds, false),
                                    initial_offset: offset_for(box_, scroll_offset),
                                });
                                if let Some(owner) = owner {
                                    scroll_owners.insert(owner, next);
                                }
                                next
                            }
                        };
                        state.scroll = next;
                    }
                }
                PaintCommand::PopClip => {
                    if let Some(previous) = clip_stack.pop() {
                        state.clip = previous.clip;
                        state.scroll = previous.scroll;
                    }
                }
                PaintCommand::PaintDecorations(box_, _) => {
                    item = Some((
                        command_index,
                        box_.node,
                        PaintItemKind::Decorations,
                        if box_.style.get("box-shadow").is_some() {
                            PaintBounds::viewport(width, height)
                        } else {
                            PaintBounds::from_layout(box_.border_box)
                        },
                        state,
                        hints(box_, is_video),
                        sticky_metadata(box_, state.scroll, &view.scrolls, &containing_blocks),
                    ))
                }
                PaintCommand::PaintContent(box_) => {
                    item = Some((
                        command_index,
                        box_.node,
                        PaintItemKind::Content,
                        content_ink_bounds(box_, width, height),
                        state,
                        hints(box_, is_video),
                        sticky_metadata(box_, state.scroll, &view.scrolls, &containing_blocks),
                    ))
                }
                PaintCommand::PaintControl(control) => {
                    item = Some((
                        command_index,
                        control.geometry.node,
                        PaintItemKind::Control,
                        PaintBounds::from_layout(control.geometry.border_box),
                        state,
                        hints(&control.geometry, is_video),
                        None,
                    ))
                }
            }
            if let Some((command_index, node, kind, bounds, properties, hints, sticky)) = item {
                let key = node.and_then(|node| generation(node).map(|generation| {
                    if view.node_generations.insert(node, generation).is_some_and(|old| old != generation) {
                        view.ambiguous_generation = true;
                    }
                    let node = PaintNodeKey { node, generation };
                    let ordinal = ordinals.entry((node, kind)).or_insert(0u32);
                    let key = StablePaintItemKey { node, phase: kind, ordinal: *ordinal };
                    *ordinal = ordinal.saturating_add(1);
                    key
                }));
                view.push_item(command_index, key, kind, bounds, properties, hints, sticky);
            }
        }
        for command in &view.commands {
            if let PaintCommand::PaintContent(box_) = command {
                for fragment in box_.styled_replaced_fragments() {
                    if let Some(value) = generation(fragment.node) {
                        if view.node_generations.insert(fragment.node, value).is_some_and(|old| old != value) {
                            view.ambiguous_generation = true;
                        }
                    }
                }
            }
        }
        view
    }

    fn push_item(
        &mut self,
        command_index: usize,
        key: Option<StablePaintItemKey>,
        kind: PaintItemKind,
        local_bounds: PaintBounds,
        properties: PropertyState,
        hints: CompositingHints,
        sticky: Option<StickyMetadata>,
    ) {
        self.item_commands.push(command_index);
        self.items.push(PaintItem {
            index: self.items.len(),
            key,
            kind,
            local_bounds,
            properties,
            hints,
            sticky,
        });
    }

    pub fn items(&self) -> &[PaintItem] {
        &self.items
    }

    pub fn transforms(&self) -> &[TransformNode] {
        &self.transforms
    }

    pub fn clips(&self) -> &[ClipNode] {
        &self.clips
    }

    pub fn effects(&self) -> &[EffectNode] {
        &self.effects
    }

    pub fn scrolls(&self) -> &[ScrollNode] {
        &self.scrolls
    }

    /// Resolve a DOM owner and CSS property to a view-local property ID.
    /// Missing generations and duplicate owners are ambiguous, so the caller
    /// must use its full scene rebuild path. Root entries are synthetic.
    pub fn property_owner_mapping(&self) -> Option<PropertyOwnerMapping> {
        if self.ambiguous_generation {
            return None;
        }
        let mut mapping = PropertyOwnerMapping {
            transforms: HashMap::new(),
            effects: HashMap::new(),
            scrolls: HashMap::new(),
        };
        for (id, node) in self.transforms.iter().enumerate().skip(1) {
            if mapping.transforms.insert(node.owner?, id as u32).is_some() {
                return None;
            }
        }
        for (id, node) in self.effects.iter().enumerate().skip(1) {
            if mapping.effects.insert(node.owner?, id as u32).is_some() {
                return None;
            }
        }
        for (id, node) in self.scrolls.iter().enumerate().skip(1) {
            if mapping.scrolls.insert(node.owner?, id as u32).is_some() {
                return None;
            }
        }
        let mut generations = self.node_generations.clone();
        for owner in mapping.transforms.keys().chain(mapping.effects.keys()).chain(mapping.scrolls.keys()) {
            if generations.insert(owner.node, owner.generation).is_some_and(|old| old != owner.generation) {
                return None;
            }
        }
        Some(mapping)
    }

    /// Paints items in `[start, end)` using the existing command rasterizer.
    /// `offset` maps document pixels into the target; `clip` is in target
    /// coordinates. Returns false for an invalid range or a raster budget hit.
    pub fn raster_range(
        &self,
        items: Range<usize>,
        target: &mut RgbaSurface,
        offset: (i32, i32),
        clip: PaintBounds,
    ) -> bool {
        if items.start > items.end || items.end > self.items.len() {
            return false;
        }
        if items.is_empty() {
            return true;
        }
        let first = self.item_commands[items.start];
        let last = self.item_commands[items.end - 1] + 1;
        paint::raster_command_range(
            &self.commands,
            first..last,
            (self.width, self.height),
            self.text,
            self.images,
            self.controls,
            target,
            offset,
            clip,
        )
    }

    /// Rasterize separable items into a tile whose top-left is `tile_origin`
    /// in untransformed document coordinates. The caller applies the item
    /// property state; this path skips group transforms, opacity and clips.
    /// Unsupported items fail explicitly and leave `target` unchanged.
    pub fn raster_raw_local(
        &self,
        items: Range<usize>,
        target: &mut RgbaSurface,
        tile_origin: (i32, i32),
    ) -> Result<(), RawRasterError> {
        if items.start > items.end || items.end > self.items.len() {
            return Err(RawRasterError::InvalidRange);
        }
        let Some(pixels) = (target.width as usize).checked_mul(target.height as usize) else {
            return Err(RawRasterError::TileTooLarge);
        };
        if pixels > 16 * 1024 * 1024 {
            return Err(RawRasterError::TileTooLarge);
        }
        if pixels.checked_mul(4) != Some(target.pixels.len()) {
            return Err(RawRasterError::InvalidTile);
        }
        for index in items.clone() {
            let command = &self.commands[self.item_commands[index]];
            if !matches!(
                command,
                PaintCommand::PaintDecorations(..) | PaintCommand::PaintContent(..)
            ) {
                return Err(RawRasterError::UnseparableItem(index));
            }
        }
        let mut result = target.clone();
        for index in items {
            let command = &self.commands[self.item_commands[index]];
            if !paint::raster_raw_item(command, &mut result, tile_origin, self.text, self.images) {
                return Err(RawRasterError::RasterFailed(index));
            }
        }
        *target = result;
        Ok(())
    }
}

fn finite_bounds(bounds: PaintBounds, fallback: PaintBounds) -> PaintBounds {
    if [bounds.x, bounds.y, bounds.width, bounds.height]
        .into_iter()
        .all(f32::is_finite)
        && bounds.width >= 0.0 && bounds.height >= 0.0
    { bounds } else { fallback }
}

fn offset_for(box_: &LayoutBox, get: &dyn Fn(NodeIdx) -> Option<(f32, f32)>) -> (f32, f32) {
    box_.node.and_then(get)
        .filter(|(x, y)| x.is_finite() && y.is_finite())
        .unwrap_or((0.0, 0.0))
}

fn content_ink_bounds(box_: &LayoutBox, width: u32, height: u32) -> PaintBounds {
    let viewport = PaintBounds::viewport(width, height);
    let mut result: Option<PaintBounds> = None;
    let mut include = |bounds: PaintBounds| {
        result = Some(result.map_or(bounds, |old| old.union(bounds)));
    };
    for line in &box_.lines {
        for fragment in &line.replaced {
            include(PaintBounds { x: fragment.x, y: fragment.y, width: fragment.width, height: fragment.height });
        }
        for decoration in &line.decorations {
            include(PaintBounds { x: decoration.x, y: decoration.y, width: decoration.width, height: decoration.height });
        }
        for glyph in &line.glyphs {
            // Text raster metrics are provider-owned. Four em around the pen
            // includes normal ascenders, side bearings, synthetic slant and
            // the glyph's explicit scale; unknown nonfinite metrics fall back.
            let radius = glyph.font_size * glyph.scale_x.abs().max(glyph.scale_y.abs()).max(1.0) * 4.0;
            let baseline = if glyph.writing_mode.is_vertical_flow() { glyph.y } else { line.baseline + glyph.y - glyph.offset_y };
            include(PaintBounds {
                x: glyph.x.min(glyph.x + glyph.advance_x) - radius,
                y: baseline.min(baseline + glyph.advance_y) - radius,
                width: glyph.advance_x.abs() + radius * 2.0,
                height: glyph.advance_y.abs() + radius * 2.0,
            });
        }
        for word in &line.words {
            let radius = word.font_size * 2.0;
            include(PaintBounds { x: word.x - radius, y: line.baseline - radius, width: word.width + radius * 2.0, height: radius * 2.0 });
        }
    }
    for edge in &box_.table_decoration().borders {
        include(PaintBounds::from_layout(edge.rect));
    }
    finite_bounds(result.unwrap_or(PaintBounds { x: box_.content_box.x, y: box_.content_box.y, width: 0.0, height: 0.0 }), viewport)
}

fn collect_containing_blocks(
    box_: &LayoutBox,
    parent: PaintBounds,
    out: &mut HashMap<usize, PaintBounds>,
) {
    out.insert(box_ as *const LayoutBox as usize, parent);
    let content = box_.content_box;
    let own = PaintBounds {
        x: content.x - box_.padding.left,
        y: content.y - box_.padding.top,
        width: content.w + box_.padding.left + box_.padding.right,
        height: content.h + box_.padding.top + box_.padding.bottom,
    };
    if let Flow::Blocks(children) | Flow::Flex(children) | Flow::Grid(children) = &box_.flow {
        for child in children {
            collect_containing_blocks(child, own, out);
        }
    }
    for child in &box_.positioned {
        collect_containing_blocks(child, own, out);
    }
}

fn scrollable_extent(box_: &LayoutBox, port: PaintBounds, stop_at_clips: bool) -> PaintBounds {
    fn include(box_: &LayoutBox, bounds: &mut PaintBounds, depth: usize, stop_at_clips: bool) {
        if depth > 256 { return; }
        let own = PaintBounds::from_layout(box_.border_box);
        if [own.x, own.y, own.width, own.height].into_iter().all(f32::is_finite)
            && own.width >= 0.0 && own.height >= 0.0 {
            *bounds = bounds.union(own);
        }
        if stop_at_clips && depth > 0 && ["overflow", "overflow-x", "overflow-y"]
            .into_iter().filter_map(|name| box_.style.get(name))
            .any(|value| !value.eq_ignore_ascii_case("visible")) {
            return;
        }
        for line in &box_.lines {
            let own = PaintBounds { x: box_.content_box.x, y: line.y_top, width: box_.content_box.w, height: line.height };
            if [own.x, own.y, own.width, own.height].into_iter().all(f32::is_finite) {
                *bounds = bounds.union(own);
            }
            for fragment in &line.replaced {
                let own = PaintBounds { x: fragment.x, y: fragment.y, width: fragment.width, height: fragment.height };
                if [own.x, own.y, own.width, own.height].into_iter().all(f32::is_finite) {
                    *bounds = bounds.union(own);
                }
            }
        }
        if let Flow::Blocks(children) | Flow::Flex(children) | Flow::Grid(children) = &box_.flow {
            for child in children { if child.style.get("position") != Some("fixed") { include(child, bounds, depth + 1, stop_at_clips); } }
        }
        for child in &box_.positioned {
            if child.style.get("position") != Some("fixed") { include(child, bounds, depth + 1, stop_at_clips); }
        }
    }
    let mut result = port;
    include(box_, &mut result, 0, stop_at_clips);
    result
}

fn sticky_metadata(
    box_: &LayoutBox,
    scroll: u32,
    scrolls: &[ScrollNode],
    containing_blocks: &HashMap<usize, PaintBounds>,
) -> Option<StickyMetadata> {
    if box_.style.get("position") != Some("sticky") { return None; }
    let cb = *containing_blocks.get(&(box_ as *const LayoutBox as usize))?;
    let port = scrolls.get(scroll as usize)?.bounds;
    let side = |name: &str, basis: f32| -> Option<f32> {
        amazing_layout::values::parse_length_percentage(box_.style.get(name)?, box_.font_size, box_.font_size * 0.5)?
            .resolve(Some(basis))
            .filter(|value| value.is_finite())
    };
    Some(StickyMetadata {
        normal: PaintBounds::from_layout(box_.border_box),
        containing_block: cb,
        scroll,
        top: side("top", port.height),
        right: side("right", port.width),
        bottom: side("bottom", port.height),
        left: side("left", port.width),
    })
}

fn is_scroller(box_: &LayoutBox) -> bool {
    ["overflow", "overflow-x", "overflow-y"]
        .into_iter()
        .filter_map(|property| box_.style.get(property))
        .any(|value| value.eq_ignore_ascii_case("auto") || value.eq_ignore_ascii_case("scroll"))
}

fn clip_bounds(box_: &LayoutBox, width: u32, height: u32) -> PaintBounds {
    let clipping = |property| {
        box_.style
            .get(property)
            .is_some_and(|value| !value.eq_ignore_ascii_case("visible"))
    };
    let horizontal = clipping("overflow") || clipping("overflow-x");
    let vertical = clipping("overflow") || clipping("overflow-y");
    let content = box_.content_box;
    let x = content.x - box_.padding.left;
    let y = content.y - box_.padding.top;
    let w = content.w + box_.padding.left + box_.padding.right;
    let h = content.h + box_.padding.top + box_.padding.bottom;
    PaintBounds {
        x: if horizontal { x } else { 0.0 },
        y: if vertical { y } else { 0.0 },
        width: if horizontal { w } else { width as f32 },
        height: if vertical { h } else { height as f32 },
    }
}

fn hints(box_: &LayoutBox, is_video: &dyn Fn(NodeIdx) -> bool) -> CompositingHints {
    let position = box_.style.get("position").unwrap_or("");
    let will_change = box_
        .style
        .get("will-change")
        .is_some_and(|value| !value.eq_ignore_ascii_case("auto") && !value.trim().is_empty());
    CompositingHints {
        will_change,
        fixed: position.eq_ignore_ascii_case("fixed"),
        sticky: position.eq_ignore_ascii_case("sticky"),
        video: box_.node.is_some_and(is_video),
        scroller: is_scroller(box_),
    }
}

#[cfg(test)]
#[path = "display_list_view_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "display_list_delta_tests.rs"]
mod delta_tests;
