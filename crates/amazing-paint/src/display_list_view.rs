//! Read-only paint items and property state for tiled compositors.
//!
//! Indices and property IDs are stable for one built view. Rebuild the view
//! after layout or style changes; no layout ownership escapes through items.

use std::ops::Range;

use amazing_dom::NodeIdx;
use amazing_layout::LayoutBox;
use amazing_text::TextSystem;

use crate::controls::ControlProvider;
use crate::display_list::{build_display_list, Affine, PaintCommand};
use crate::paint::{self, ImageProvider};
use crate::surface::Surface;

/// An RGBA8 destination for rasterized paint items.
pub type RgbaSurface = Surface;

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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaintItemKind {
    Decorations,
    Content,
    Control,
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
    pub kind: PaintItemKind,
    /// Conservative bounds before the item's property transform. Items whose
    /// ink may escape their box use the viewport as a safe bound.
    pub local_bounds: PaintBounds,
    pub properties: PropertyState,
    pub hints: CompositingHints,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TransformNode {
    pub parent: u32,
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
    pub opacity: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScrollNode {
    pub parent: u32,
    pub bounds: PaintBounds,
}

/// A borrowed view of the painter's ordered commands. All layout references
/// remain private; callers receive only value records and rasterized pixels.
pub struct DisplayListView<'a> {
    commands: Vec<PaintCommand<'a>>,
    item_commands: Vec<usize>,
    items: Vec<PaintItem>,
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
        let commands = build_display_list(root);
        let mut view = Self {
            commands,
            item_commands: Vec::new(),
            items: Vec::new(),
            transforms: vec![TransformNode {
                parent: 0,
                matrix: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            }],
            clips: vec![ClipNode {
                parent: 0,
                bounds: PaintBounds::viewport(width, height),
            }],
            effects: vec![EffectNode {
                parent: 0,
                opacity: 1.0,
            }],
            scrolls: vec![ScrollNode {
                parent: 0,
                bounds: PaintBounds::viewport(width, height),
            }],
            width,
            height,
            text,
            images,
            controls,
        };
        let mut state = PropertyState::default();
        let mut group_stack = Vec::new();
        let mut clip_stack = Vec::new();
        for command_index in 0..view.commands.len() {
            let mut item = None;
            match &view.commands[command_index] {
                PaintCommand::BeginGroup(group) => {
                    group_stack.push(state);
                    let next = view.effects.len() as u32;
                    view.effects.push(EffectNode {
                        parent: state.effect,
                        opacity: group.opacity,
                    });
                    state.effect = next;
                    if group.transform != Affine::IDENTITY {
                        let matrix = group.matrix_about_origin();
                        let next = view.transforms.len() as u32;
                        view.transforms.push(TransformNode {
                            parent: state.transform,
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
                    let bounds = clip_bounds(box_, width, height);
                    let next = view.clips.len() as u32;
                    view.clips.push(ClipNode {
                        parent: state.clip,
                        bounds,
                    });
                    state.clip = next;
                    if is_scroller(box_) {
                        let next = view.scrolls.len() as u32;
                        view.scrolls.push(ScrollNode {
                            parent: state.scroll,
                            bounds,
                        });
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
                        PaintItemKind::Decorations,
                        if box_.style.get("box-shadow").is_some() {
                            PaintBounds::viewport(width, height)
                        } else {
                            PaintBounds::from_layout(box_.border_box)
                        },
                        state,
                        hints(box_, is_video),
                    ))
                }
                PaintCommand::PaintContent(box_) => {
                    item = Some((
                        command_index,
                        PaintItemKind::Content,
                        PaintBounds::viewport(width, height),
                        state,
                        hints(box_, is_video),
                    ))
                }
                PaintCommand::PaintControl(control) => {
                    item = Some((
                        command_index,
                        PaintItemKind::Control,
                        PaintBounds::from_layout(control.geometry.border_box),
                        state,
                        hints(&control.geometry, is_video),
                    ))
                }
            }
            if let Some((command_index, kind, bounds, properties, hints)) = item {
                view.push_item(command_index, kind, bounds, properties, hints);
            }
        }
        view
    }

    fn push_item(
        &mut self,
        command_index: usize,
        kind: PaintItemKind,
        local_bounds: PaintBounds,
        properties: PropertyState,
        hints: CompositingHints,
    ) {
        self.item_commands.push(command_index);
        self.items.push(PaintItem {
            index: self.items.len(),
            kind,
            local_bounds,
            properties,
            hints,
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
