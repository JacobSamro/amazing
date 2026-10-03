//! Provider-neutral paint command construction.
//!
//! The builder is the only stage that walks [`LayoutBox`]. Rasterization
//! consumes the flattened command stream, so stacking/effect policy stays
//! separate from pixels and nested stacking contexts remain atomic.

use amazing_layout::{Flow, LayoutBox, Rect};

const MAX_DISPLAY_COMMANDS: usize = 1_000_000;
const MAX_LAYOUT_DEPTH: usize = 256;
const MAX_EFFECT_DEPTH: usize = 16;
const MAX_CHILDREN_PER_BOX: usize = 100_000;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Affine {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub e: f32,
    pub f: f32,
}

impl Affine {
    pub const IDENTITY: Self = Self {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 0.0,
        f: 0.0,
    };

    fn then(self, next: Self) -> Self {
        Self {
            a: next.a * self.a + next.c * self.b,
            b: next.b * self.a + next.d * self.b,
            c: next.a * self.c + next.c * self.d,
            d: next.b * self.c + next.d * self.d,
            e: next.a * self.e + next.c * self.f + next.e,
            f: next.b * self.e + next.d * self.f + next.f,
        }
    }

    pub fn inverse(self) -> Option<Self> {
        let determinant = self.a * self.d - self.b * self.c;
        if !determinant.is_finite() || determinant.abs() < 1e-8 {
            return None;
        }
        let inverse = 1.0 / determinant;
        Some(Self {
            a: self.d * inverse,
            b: -self.b * inverse,
            c: -self.c * inverse,
            d: self.a * inverse,
            e: (self.c * self.f - self.d * self.e) * inverse,
            f: (self.b * self.e - self.a * self.f) * inverse,
        })
    }

    pub fn map(self, x: f32, y: f32) -> (f32, f32) {
        (
            self.a * x + self.c * y + self.e,
            self.b * x + self.d * y + self.f,
        )
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct GroupEffect {
    pub owner: Option<amazing_dom::NodeIdx>,
    pub opacity: f32,
    pub transform: Affine,
    pub origin_x: f32,
    pub origin_y: f32,
    pub blend: amazing_css::effects::BlendMode,
    pub isolate: bool,
    pub filter: crate::filter::FilterPlan,
    /// CSS `backdrop-filter`'s resolved function list. Empty means none.
    pub backdrop_filter: crate::filter::FilterPlan,
    /// The border box `backdrop_filter` samples, in the same untransformed
    /// surface pixel space as this box's own decorations (valid only when
    /// `backdrop_filter` is non-empty).
    pub backdrop_bounds: Rect,
    /// Resolved CSS `clip-path` (`inset()`/`circle()`/`ellipse()`/
    /// `polygon()`/a straight-edged `path()`).
    pub clip: Option<crate::clip::ClipMask>,
    /// Resolved single-layer `mask-image: url(...)` (see [`crate::mask`]).
    pub mask_image: Option<crate::mask::MaskLayer>,
    /// The border box `mask_image` is stretched to fill, in the same
    /// untransformed surface pixel space as `backdrop_bounds` (valid only
    /// when `mask_image` is `Some`).
    pub mask_bounds: Rect,
}

impl GroupEffect {
    pub fn matrix_about_origin(self) -> Affine {
        Affine {
            e: -self.origin_x,
            f: -self.origin_y,
            ..Affine::IDENTITY
        }
        .then(self.transform)
        .then(Affine {
            e: self.origin_x,
            f: self.origin_y,
            ..Affine::IDENTITY
        })
    }
}

pub(crate) enum PaintCommand<'a> {
    BeginGroup(GroupEffect),
    PaintDecorations(&'a LayoutBox, crate::background::BackgroundPlan),
    PaintContent(&'a LayoutBox),
    PaintControl(Box<crate::controls::ControlPaint>),
    PushOverflowClip(&'a LayoutBox),
    PopClip,
    EndGroup,
}

pub(crate) fn build_display_list(root: &LayoutBox) -> Vec<PaintCommand<'_>> {
    let mut commands = Vec::new();
    if emit_context(root, &mut commands, 0, 0) {
        commands
    } else {
        Vec::new()
    }
}

struct ContextEntry<'a> {
    z_index: i32,
    order: usize,
    box_: &'a LayoutBox,
    clips: Vec<&'a LayoutBox>,
}

struct ContextBuckets<'a> {
    negative: Vec<ContextEntry<'a>>,
    middle_phase: Vec<MiddlePhaseItem<'a>>,
    positive: Vec<ContextEntry<'a>>,
    next_order: usize,
}

impl ContextBuckets<'_> {
    fn new() -> Self {
        Self {
            negative: Vec::new(),
            middle_phase: Vec::new(),
            positive: Vec::new(),
            next_order: 0,
        }
    }
}

enum MiddlePhaseItem<'a> {
    Command(PaintCommand<'a>),
    Context(ContextEntry<'a>),
}

fn emit_context<'a>(
    root: &'a LayoutBox,
    commands: &mut Vec<PaintCommand<'a>>,
    depth: usize,
    effect_depth: usize,
) -> bool {
    if depth > MAX_LAYOUT_DEPTH || commands.len() >= MAX_DISPLAY_COMMANDS {
        return false;
    }
    let effect = effect_for(root);
    if effect.is_some_and(|effect| effect.opacity <= 0.0) {
        return true;
    }
    let next_effect_depth = effect_depth + usize::from(effect.is_some());
    if next_effect_depth > MAX_EFFECT_DEPTH {
        return false;
    }
    if let Some(effect) = effect {
        commands.push(PaintCommand::BeginGroup(effect));
    }

    commands.push(PaintCommand::PaintDecorations(
        root,
        crate::background::BackgroundPlan::from_style(&root.style),
    ));
    let root_clips = clips_overflow(root);
    if root_clips {
        commands.push(PaintCommand::PushOverflowClip(root));
    }

    let mut middle = Vec::new();
    let mut buckets = ContextBuckets::new();
    let Some(children) = source_children(root) else {
        return false;
    };
    let items_accept_z = matches!(root.flow, Flow::Flex(_) | Flow::Grid(_));
    for child in children {
        if !walk_middle(
            child,
            items_accept_z,
            &mut middle,
            &mut buckets,
            &mut Vec::new(),
            depth + 1,
        ) {
            return false;
        }
    }

    buckets
        .negative
        .sort_by_key(|entry| (entry.z_index, entry.order));
    buckets
        .positive
        .sort_by_key(|entry| (entry.z_index, entry.order));
    for entry in buckets.negative {
        if !emit_entry(entry, commands, depth + 1, next_effect_depth) {
            return false;
        }
    }
    let Some(controls) = control_commands(root) else {
        return false;
    };
    commands.extend(controls);
    let table_borders = !root.table_decoration().borders.is_empty();
    if !table_borders {
        commands.push(PaintCommand::PaintContent(root));
    }
    commands.append(&mut middle);
    for item in buckets.middle_phase {
        match item {
            MiddlePhaseItem::Command(command) => commands.push(command),
            MiddlePhaseItem::Context(entry) => {
                if !emit_entry(entry, commands, depth + 1, next_effect_depth) {
                    return false;
                }
            }
        }
    }
    for entry in buckets.positive {
        if !emit_entry(entry, commands, depth + 1, next_effect_depth) {
            return false;
        }
    }

    // Collapsed winners cover row/cell backgrounds and are painted once by
    // their table after its descendants, inside the table effect/clip scope.
    if table_borders {
        commands.push(PaintCommand::PaintContent(root));
    }

    if root_clips {
        commands.push(PaintCommand::PopClip);
    }
    if effect.is_some() {
        commands.push(PaintCommand::EndGroup);
    }
    commands.len() <= MAX_DISPLAY_COMMANDS
}

fn walk_middle<'a>(
    box_: &'a LayoutBox,
    z_applies_to_item: bool,
    middle: &mut Vec<PaintCommand<'a>>,
    buckets: &mut ContextBuckets<'a>,
    inherited_clips: &mut Vec<&'a LayoutBox>,
    depth: usize,
) -> bool {
    if depth > MAX_LAYOUT_DEPTH
        || middle.len() >= MAX_DISPLAY_COMMANDS
        || buckets.middle_phase.len() >= MAX_DISPLAY_COMMANDS
        || buckets.next_order >= MAX_DISPLAY_COMMANDS
    {
        return false;
    }
    let order = buckets.next_order;
    buckets.next_order += 1;
    let z_index = box_.style.get("z-index").and_then(parse_integer);
    let z_applies = is_positioned(box_) || z_applies_to_item;
    let creates_context = establishes_effect_context(box_)
        || has_fixed_or_sticky_position(box_)
        || (z_applies && z_index.is_some());
    if creates_context {
        let entry = ContextEntry {
            z_index: z_index.unwrap_or(0),
            order,
            box_,
            clips: inherited_clips.clone(),
        };
        match entry.z_index {
            z if z < 0 => buckets.negative.push(entry),
            0 => buckets.middle_phase.push(MiddlePhaseItem::Context(entry)),
            _ => buckets.positive.push(entry),
        }
        return true;
    }

    if is_positioned(box_) {
        // An auto-z positioned box remains in its containing context, but its
        // in-flow descendants still paint before its positioned descendants.
        // Collect that subtree's flow first, then append its positioned phase
        // as one source-ordered unit in the containing context.
        let mut local_flow = Vec::new();
        let mut local_buckets = ContextBuckets::new();
        local_buckets.next_order = buckets.next_order;
        if !walk_middle_contents(
            box_,
            &mut local_flow,
            &mut local_buckets,
            inherited_clips,
            depth,
        ) {
            return false;
        }
        if buckets
            .middle_phase
            .len()
            .checked_add(local_flow.len())
            .and_then(|n| n.checked_add(local_buckets.middle_phase.len()))
            .and_then(|n| n.checked_add(inherited_clips.len() * 2))
            .is_none_or(|n| n > MAX_DISPLAY_COMMANDS)
        {
            return false;
        }
        buckets.next_order = local_buckets.next_order;
        buckets.negative.extend(local_buckets.negative);
        buckets.positive.extend(local_buckets.positive);
        for clip in inherited_clips.iter().copied() {
            buckets
                .middle_phase
                .push(MiddlePhaseItem::Command(PaintCommand::PushOverflowClip(
                    clip,
                )));
        }
        buckets
            .middle_phase
            .extend(local_flow.into_iter().map(MiddlePhaseItem::Command));
        buckets.middle_phase.extend(local_buckets.middle_phase);
        for _ in inherited_clips.iter() {
            buckets
                .middle_phase
                .push(MiddlePhaseItem::Command(PaintCommand::PopClip));
        }
        true
    } else {
        walk_middle_contents(box_, middle, buckets, inherited_clips, depth)
    }
}

fn walk_middle_contents<'a>(
    box_: &'a LayoutBox,
    middle: &mut Vec<PaintCommand<'a>>,
    buckets: &mut ContextBuckets<'a>,
    inherited_clips: &mut Vec<&'a LayoutBox>,
    depth: usize,
) -> bool {
    middle.push(PaintCommand::PaintDecorations(
        box_,
        crate::background::BackgroundPlan::from_style(&box_.style),
    ));
    let clips = clips_overflow(box_);
    if clips {
        middle.push(PaintCommand::PushOverflowClip(box_));
        inherited_clips.push(box_);
    }
    let Some(controls) = control_commands(box_) else {
        return false;
    };
    for control in controls {
        middle.push(control);
    }
    let table_borders = !box_.table_decoration().borders.is_empty();
    if !table_borders {
        middle.push(PaintCommand::PaintContent(box_));
    }
    let Some(children) = source_children(box_) else {
        return false;
    };
    let child_z_applies = matches!(box_.flow, Flow::Flex(_) | Flow::Grid(_));
    for child in children {
        if !walk_middle(
            child,
            child_z_applies,
            middle,
            buckets,
            inherited_clips,
            depth + 1,
        ) {
            return false;
        }
    }
    if table_borders {
        middle.push(PaintCommand::PaintContent(box_));
    }
    if clips {
        inherited_clips.pop();
        middle.push(PaintCommand::PopClip);
    }
    middle.len() <= MAX_DISPLAY_COMMANDS
}

fn emit_entry<'a>(
    entry: ContextEntry<'a>,
    commands: &mut Vec<PaintCommand<'a>>,
    depth: usize,
    effect_depth: usize,
) -> bool {
    for clip in &entry.clips {
        commands.push(PaintCommand::PushOverflowClip(clip));
    }
    if !emit_context(entry.box_, commands, depth, effect_depth) {
        return false;
    }
    for _ in entry.clips {
        commands.push(PaintCommand::PopClip);
    }
    commands.len() <= MAX_DISPLAY_COMMANDS
}

fn clips_overflow(box_: &LayoutBox) -> bool {
    ["overflow", "overflow-x", "overflow-y"]
        .into_iter()
        .filter_map(|property| box_.style.get(property))
        .any(|value| !value.eq_ignore_ascii_case("visible"))
}

fn source_children(box_: &LayoutBox) -> Option<Vec<&LayoutBox>> {
    let flow_count = match &box_.flow {
        Flow::Blocks(children) | Flow::Flex(children) | Flow::Grid(children) => children.len(),
        Flow::Inlines(_) | Flow::Empty => 0,
    };
    let total = flow_count.checked_add(box_.positioned.len())?;
    if total > MAX_CHILDREN_PER_BOX {
        return None;
    }
    let mut children = Vec::with_capacity(total);
    match &box_.flow {
        Flow::Blocks(flow) | Flow::Flex(flow) | Flow::Grid(flow) => children.extend(flow),
        Flow::Inlines(_) | Flow::Empty => {}
    }
    children.extend(box_.positioned.iter());
    // LAYOUT-13 assigns every emitted box its pre-order tree position. Older
    // layout producers without that key retain the arena-order fallback.
    children.sort_by_key(|child| {
        child
            .source_order
            .unwrap_or_else(|| child.node.map_or(0, |node| u64::from(node.0)))
    });
    Some(children)
}

fn is_positioned(box_: &LayoutBox) -> bool {
    box_.style
        .get("position")
        .is_some_and(|value| !value.eq_ignore_ascii_case("static"))
}

fn has_fixed_or_sticky_position(box_: &LayoutBox) -> bool {
    box_.style.get("position").is_some_and(|value| {
        value.eq_ignore_ascii_case("fixed") || value.eq_ignore_ascii_case("sticky")
    })
}

fn establishes_effect_context(box_: &LayoutBox) -> bool {
    effect_for(box_).is_some()
        || box_
            .style
            .get("transform-style")
            .is_some_and(|value| value.eq_ignore_ascii_case("preserve-3d"))
}

fn parse_integer(value: &str) -> Option<i32> {
    let value = value.trim();
    if value.eq_ignore_ascii_case("auto") {
        None
    } else {
        value.parse().ok()
    }
}

fn effect_for(box_: &LayoutBox) -> Option<GroupEffect> {
    let opacity = box_
        .style
        .get("opacity")
        .and_then(|value| value.trim().parse::<f32>().ok())
        .filter(|value| value.is_finite())
        .unwrap_or(1.0)
        .clamp(0.0, 1.0);
    // CSS Transforms 1 §6: these boxes have associated CSS layout geometry.
    // fill-box uses content-box; stroke-box and the default view-box use
    // border-box. Internal SVG object/viewBox geometry is a separate path.
    let reference = match box_.style.get("transform-box") {
        Some("content-box" | "fill-box") => box_.content_box,
        _ => box_.border_box,
    };
    let transform = box_.style.get("transform")
        .and_then(|value| parse_transform(value, reference.w, reference.h));
    let blend = box_
        .style
        .get("mix-blend-mode")
        .and_then(amazing_css::effects::parse_blend_mode)
        .unwrap_or(amazing_css::effects::BlendMode::Normal);
    // `isolation: isolate` opens a group even when nothing else changes: the
    // group's own layer isolates descendant blend modes from the page backdrop.
    let isolate = box_
        .style
        .get("isolation")
        .and_then(amazing_css::effects::parse_isolation)
        .unwrap_or(false);
    let current_color = crate::paint::resolve_text_color(&box_.style);
    let filter = crate::filter::FilterPlan::from_css(
        box_.style.get("filter").unwrap_or("none"),
        current_color,
    );
    let backdrop_filter = crate::filter::FilterPlan::from_css(
        box_.style.get("backdrop-filter").unwrap_or("none"),
        current_color,
    );
    let border = box_.border_box;
    // RC-31: `clip-path`'s shape/bare-box keyword can name any of the four
    // `<geometry-box>` boxes, not just the border box `backdrop_bounds`/
    // `mask_bounds` already use; derive the other three from data this
    // layout box already carries (`content_box` directly, margin/border
    // widths expanding or insetting `border_box`). An unresolved auto
    // margin defaults to 0, matching a fully resolved layout's usual value.
    let margin = box_.margin.unwrap_or(0.0);
    let margin_box = Rect {
        x: border.x - margin.left,
        y: border.y - margin.top,
        w: border.w + margin.left + margin.right,
        h: border.h + margin.top + margin.bottom,
    };
    let content = box_.content_box;
    let padding_box = Rect {
        x: border.x + box_.border.left,
        y: border.y + box_.border.top,
        w: (border.w - box_.border.left - box_.border.right).max(0.0),
        h: (border.h - box_.border.top - box_.border.bottom).max(0.0),
    };
    let geometry_boxes = crate::clip::GeometryBoxes {
        margin: (margin_box.x, margin_box.y, margin_box.w, margin_box.h),
        border: (border.x, border.y, border.w, border.h),
        padding: (padding_box.x, padding_box.y, padding_box.w, padding_box.h),
        content: (content.x, content.y, content.w, content.h),
    };
    let clip = box_
        .style
        .get("clip-path")
        .and_then(|value| crate::clip::ClipMask::from_css(value, geometry_boxes));
    let mask_image = box_.style.get("mask-image").and_then(|value| {
        crate::mask::MaskLayer::from_css(value, box_.style.get("mask-mode").unwrap_or("match-source"))
    });
    if opacity >= 1.0
        && transform.is_none()
        && blend == amazing_css::effects::BlendMode::Normal
        && !isolate
        && filter.is_empty()
        && backdrop_filter.is_empty()
        && clip.is_none()
        && mask_image.is_none()
    {
        return None;
    }
    let origin = box_.style.get("transform-origin")
        .and_then(|value| transform_origin_xy(value, reference, box_.style.effective_zoom()))
        .unwrap_or((reference.x + reference.w * 0.5, reference.y + reference.h * 0.5));
    Some(GroupEffect {
        owner: box_.node,
        opacity,
        transform: transform.unwrap_or(Affine::IDENTITY),
        origin_x: origin.0,
        origin_y: origin.1,
        blend,
        isolate,
        filter,
        backdrop_filter,
        backdrop_bounds: border,
        clip,
        mask_image,
        mask_bounds: border,
    })
}

/// CSS Transforms 1 §§5–6: origin offsets use the actual reference rectangle,
/// including its top-left displacement. Computed canonical px/percentage axes
/// are resolved by CSS's shared used-axis parser, including piecewise math;
/// no font/container metrics are invented here. CSS Viewport §4 scales absolute
/// lengths once: evaluate against unscaled dimensions, then apply effective
/// zoom to the result, retaining the already-scaled percentage basis.
/// This painter is 2D: the optional computed Z length stays in style and has no
/// effect on an affine transform. Perspective requires a projective paint path.
fn transform_origin_xy(value: &str, reference: Rect, zoom: f64) -> Option<(f32, f32)> {
    if !zoom.is_finite() || zoom <= 0.0 { return None; }
    let parts = origin_components(value)?;
    let xy = match parts.as_slice() {
        [_, _, _] => format!("{} {}", parts[0], parts[1]),
        [_] | [_, _] => value.to_owned(),
        _ => return None,
    };
    let (x, y) = if let Some((x, y)) = amazing_css::affine_position_pair(&xy) {
        (x.resolve(reference.w as f64 / zoom)?, y.resolve(reference.h as f64 / zoom)?)
    } else {
        let components = origin_components(&xy)?;
        let [x, y] = components.as_slice() else { return None; };
        (amazing_css::used_position_axis(x, reference.w as f64 / zoom)?,
         amazing_css::used_position_axis(y, reference.h as f64 / zoom)?)
    };
    let x = (reference.x as f64 + x * zoom) as f32;
    let y = (reference.y as f64 + y * zoom) as f32;
    (x.is_finite() && y.is_finite()).then_some((x, y))
}

fn origin_components(value: &str) -> Option<Vec<&str>> {
    let mut depth = 0usize;
    let mut start = None;
    let mut parts = Vec::new();
    for (i, ch) in value.char_indices() {
        if ch.is_ascii_whitespace() && depth == 0 {
            if let Some(begin) = start.take() { parts.push(&value[begin..i]); }
            continue;
        }
        start.get_or_insert(i);
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.checked_sub(1)?,
            _ => {},
        }
    }
    if depth != 0 { return None; }
    if let Some(begin) = start { parts.push(&value[begin..]); }
    Some(parts)
}

fn parse_transform(value: &str, reference_width: f32, reference_height: f32) -> Option<Affine> {
    if value.trim().eq_ignore_ascii_case("none") {
        return None;
    }
    let mut remaining = value.trim();
    let mut matrix = Affine::IDENTITY;
    let mut parsed = false;
    while !remaining.is_empty() {
        let open = remaining.find('(')?;
        let name = remaining[..open].trim().to_ascii_lowercase();
        let close = find_closing_paren(remaining, open)?;
        let arguments = &remaining[open + 1..close];
        let values = split_arguments(arguments)?;
        let next = match name.as_str() {
            "translate" if (1..=2).contains(&values.len()) => Affine {
                e: parse_translation_length(values.first()?, reference_width)?,
                f: match values.get(1) {
                    Some(value) => parse_translation_length(value, reference_height)?,
                    None => 0.0,
                },
                ..Affine::IDENTITY
            },
            "translatex" if values.len() == 1 => Affine {
                e: parse_translation_length(values.first()?, reference_width)?,
                ..Affine::IDENTITY
            },
            "translatey" if values.len() == 1 => Affine {
                f: parse_translation_length(values.first()?, reference_height)?,
                ..Affine::IDENTITY
            },
            "scale" if (1..=2).contains(&values.len()) => {
                let x = parse_number(values.first()?)?;
                let y = values
                    .get(1)
                    .and_then(|value| parse_number(value))
                    .unwrap_or(x);
                Affine {
                    a: x,
                    d: y,
                    ..Affine::IDENTITY
                }
            }
            "scalex" if values.len() == 1 => Affine {
                a: parse_number(values.first()?)?,
                ..Affine::IDENTITY
            },
            "scaley" if values.len() == 1 => Affine {
                d: parse_number(values.first()?)?,
                ..Affine::IDENTITY
            },
            "rotate" if values.len() == 1 => {
                let radians = parse_angle(values.first()?)?;
                let (sin, cos) = radians.sin_cos();
                Affine {
                    a: cos,
                    b: sin,
                    c: -sin,
                    d: cos,
                    e: 0.0,
                    f: 0.0,
                }
            }
            "matrix" if values.len() == 6 => Affine {
                a: parse_number(values[0])?,
                b: parse_number(values[1])?,
                c: parse_number(values[2])?,
                d: parse_number(values[3])?,
                e: parse_number(values[4])?,
                f: parse_number(values[5])?,
            },
            _ => return None,
        };
        matrix = matrix.then(next);
        parsed = true;
        remaining = remaining[close + 1..].trim_start();
    }
    parsed.then_some(matrix)
}

fn find_closing_paren(value: &str, open: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (offset, character) in value[open..].char_indices() {
        match character {
            '(' => depth += 1,
            ')' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(open + offset);
                }
            }
            _ => {}
        }
    }
    None
}

fn split_arguments(value: &str) -> Option<Vec<&str>> {
    if value.contains(',') {
        let values = value.split(',').map(str::trim).collect::<Vec<_>>();
        (!values.is_empty() && values.iter().all(|part| !part.is_empty())).then_some(values)
    } else {
        let values = value.split_ascii_whitespace().collect::<Vec<_>>();
        (!values.is_empty()).then_some(values)
    }
}

fn parse_number(value: &str) -> Option<f32> {
    value
        .trim()
        .parse::<f32>()
        .ok()
        .filter(|value| value.is_finite())
}

/// Only translations accept percentages: X is relative to reference width,
/// Y to reference height. Scale/matrix coefficients and angles keep their
/// independent number/angle grammars.
fn parse_translation_length(value: &str, reference_extent: f32) -> Option<f32> {
    let value = value.trim();
    let used = if let Some(percent) = value.strip_suffix('%') {
        parse_number(percent)? / 100.0 * reference_extent
    } else {
        parse_length(value)?
    };
    used.is_finite().then_some(used)
}

fn parse_length(value: &str) -> Option<f32> {
    let value = value.trim().to_ascii_lowercase();
    if let Some(px) = value.strip_suffix("px") {
        parse_number(px)
    } else if value == "0" {
        Some(0.0)
    } else {
        None
    }
}

fn parse_angle(value: &str) -> Option<f32> {
    let value = value.trim().to_ascii_lowercase();
    if let Some(degrees) = value.strip_suffix("deg") {
        Some(parse_number(degrees)?.to_radians())
    } else if let Some(radians) = value.strip_suffix("rad") {
        parse_number(radians)
    } else if let Some(turns) = value.strip_suffix("turn") {
        Some(parse_number(turns)? * std::f32::consts::TAU)
    } else if value == "0" {
        Some(0.0)
    } else {
        None
    }
}

/// Only this stage enumerates layout's directly-owned replaced fragment view.
fn control_commands(box_: &LayoutBox) -> Option<Vec<PaintCommand<'_>>> {
    use amazing_layout::{Display, replaced::ReplacedKind};
    let mut commands = Vec::new();
    for fragment in box_.styled_replaced_fragments() {
        if fragment.kind == ReplacedKind::Image
            || fragment
                .style
                .get("visibility")
                .is_some_and(|v| matches!(v, "hidden" | "collapse"))
        {
            continue;
        }
        if commands.len() >= MAX_CHILDREN_PER_BOX {
            return None;
        }
        let mut geometry = LayoutBox::new(
            Some(fragment.node),
            Display::Inline,
            fragment.style.clone(),
            fragment.font_size,
            0.0,
        );
        geometry.content_box = fragment.content_box;
        geometry.border_box = fragment.border_box;
        geometry.padding = fragment.padding;
        geometry.border = fragment.border;
        let effect = effect_for(&geometry);
        if effect.is_some_and(|e| e.opacity <= 0.0) {
            continue;
        }
        let background = crate::background::BackgroundPlan::from_style(fragment.style);
        if let Some(effect) = effect {
            commands.push(PaintCommand::BeginGroup(effect));
        }
        commands.push(PaintCommand::PaintControl(Box::new(
            crate::controls::ControlPaint {
                kind: fragment.kind,
                geometry,
                background,
            },
        )));
        if effect.is_some() {
            commands.push(PaintCommand::EndGroup);
        }
    }
    Some(commands)
}

#[cfg(test)]
mod transform_translation_tests {
    use super::*;

    fn close(actual: f32, expected: f32) {
        assert!((actual - expected).abs() < 0.001, "{actual} != {expected}");
    }

    #[test]
    fn translations_use_independent_reference_axes_and_keep_absolute_units() {
        for (width, height) in [(200.0, 80.0), (0.0, 0.0), (75.0, 300.0)] {
            for (css, x, y) in [
                ("translate(50%, -25%)", width * 0.5, height * -0.25),
                ("translate(50% -25%)", width * 0.5, height * -0.25),
                ("translateX(-100%)", -width, 0.0),
                ("translateY(150%)", 0.0, height * 1.5),
                ("translate(12px, 50%)", 12.0, height * 0.5),
                ("translate(-25%, -4px)", width * -0.25, -4.0),
                ("translate(50%)", width * 0.5, 0.0),
                ("translateX(0%)", 0.0, 0.0),
                ("translate(0, 0)", 0.0, 0.0),
                ("translateX(25%) translateY(-50%)", width * 0.25, height * -0.5),
            ] {
                let transform = parse_transform(css, width, height).unwrap();
                let mapped = transform.map(3.0, 7.0);
                close(mapped.0, 3.0 + x);
                close(mapped.1, 7.0 + y);
            }
        }
    }

    #[test]
    fn effect_translation_uses_own_border_dimensions() {
        let mut style = amazing_css::ComputedStyle::default();
        style.set("transform", "translate(50%, -25%)");
        let mut box_ = LayoutBox::new(None, amazing_layout::Display::Block, style, 16.0, 20.0);
        box_.border_box = Rect { x: 400.0, y: 500.0, w: 120.0, h: 80.0 };
        box_.content_box = Rect { x: 410.0, y: 510.0, w: 100.0, h: 60.0 };
        for (reference, x, y) in [("view-box", 60.0, -20.0), ("border-box", 60.0, -20.0),
            ("stroke-box", 60.0, -20.0), ("content-box", 50.0, -15.0), ("fill-box", 50.0, -15.0)] {
            box_.style.set("transform-box", reference);
            let transform = effect_for(&box_).unwrap().transform;
            close(transform.e, x);
            close(transform.f, y);
        }
    }

    #[test]
    fn parser_keeps_other_operation_grammars_and_rejects_invalid_translations() {
        assert_eq!(parse_transform("none", 100.0, 50.0), None);
        let scale = parse_transform("scale(2, 3)", 100.0, 50.0).unwrap();
        assert_eq!(scale.map(2.0, 4.0), (4.0, 12.0));
        let matrix = parse_transform("matrix(1, 0, 0, 1, 5, 6)", 100.0, 50.0).unwrap();
        assert_eq!(matrix.map(2.0, 4.0), (7.0, 10.0));
        let rotate = parse_transform("rotate(90deg)", 100.0, 50.0).unwrap();
        close(rotate.map(2.0, 4.0).0, -4.0);
        close(rotate.map(2.0, 4.0).1, 2.0);
        for css in ["translate(10px, nope)", "translateX(1)", "translateY(1deg)",
            "translate(1%, 2%, 3%)", "translateX(NaN%)", "translateY(inf%)",
            "scaleX(50%)", "rotate(50%)", "matrix(1,0,0,1,50%,0)", "translateX(10%) junk"] {
            assert!(parse_transform(css, 100.0, 50.0).is_none(), "{css}");
        }
        assert!(parse_transform("translateX(100%)", f32::INFINITY, 50.0).is_none());
    }
}

#[cfg(test)]
mod transform_origin_tests {
    use super::*;

    fn close(actual: f32, expected: f32) {
        assert!((actual - expected).abs() < 0.001, "{actual} != {expected}");
    }
    fn box_with_origin(value: &str, reference: &str) -> LayoutBox {
        let mut style = amazing_css::ComputedStyle::default();
        style.set("transform", "rotate(90deg)");
        style.set("transform-origin", value);
        style.set("transform-box", reference);
        let mut box_ = LayoutBox::new(None, amazing_layout::Display::Block, style, 16.0, 20.0);
        box_.border_box = Rect { x: 400.0, y: 500.0, w: 120.0, h: 80.0 };
        box_.content_box = Rect { x: 410.0, y: 510.0, w: 100.0, h: 60.0 };
        box_
    }

    #[test]
    fn origins_resolve_independent_axes_against_actual_reference_rectangles() {
        for reference in ["view-box", "border-box", "stroke-box", "content-box", "fill-box"] {
            let rect = if matches!(reference, "content-box" | "fill-box") {
                Rect { x: 410.0, y: 510.0, w: 100.0, h: 60.0 }
            } else { Rect { x: 400.0, y: 500.0, w: 120.0, h: 80.0 } };
            for (css, x, y) in [
                ("0 0", 0.0, 0.0), ("50% 50%", rect.w*0.5, rect.h*0.5),
                ("100% 0%", rect.w, 0.0), ("left bottom", 0.0, rect.h),
                ("12px -4px", 12.0, -4.0), ("-25% 150%", -rect.w*0.25, rect.h*1.5),
                ("calc(100% - 10px) calc(50% + 2px)",rect.w-10.0,rect.h*0.5+2.0),
                ("calc((25% + 5px) * 2) calc(100% / 2 - 3px)",rect.w*0.5+10.0,rect.h*0.5-3.0),
                ("0% 100% 25px",0.0,rect.h),
                ("min(10px, 50%) max(10px, 25%)",10.0,(rect.h*0.25).max(10.0)),
                ("clamp(20px, 50%, 10px) min(25%, 100px)",20.0,rect.h*0.25),
                ("calc(min(50%, 10px) + 2px) clamp(5px, 25%, 100px)",12.0,(rect.h*0.25).max(5.0)),
            ] {
                let box_ = box_with_origin(css, reference);
                let effect = effect_for(&box_).unwrap();
                close(effect.origin_x, rect.x+x); close(effect.origin_y, rect.y+y);
                // The chosen pivot is invariant; an offset point rotates about it.
                let matrix = effect.matrix_about_origin();
                let pivot = matrix.map(rect.x+x,rect.y+y);
                close(pivot.0,rect.x+x); close(pivot.1,rect.y+y);
                let rotated = matrix.map(rect.x+x+7.0,rect.y+y+11.0);
                close(rotated.0,rect.x+x-11.0); close(rotated.1,rect.y+y+7.0);
            }
        }
    }

    #[test]
    fn origin_uses_finite_computed_geometry_and_keeps_unsupported_units_explicit() {
        let rect = Rect { x: 20.0, y: 30.0, w: 0.0, h: 0.0 };
        assert_eq!(transform_origin_xy("100% 100%",rect,1.0),Some((20.0,30.0)));
        assert_eq!(transform_origin_xy("min(10px, 50%) max(10px, 25%)",rect,1.0),Some((20.0,40.0)));
        for css in ["1em 2px", "2rem 0", "1cqw 2px", "1ex 2px", "min(1em, 50%) 0", "NaN% 0", "calc(50% - 2px 0", "0 0 0 0"] {
            assert!(transform_origin_xy(css,rect,1.0).is_none(), "{css}");
        }
        for bad in [f32::INFINITY,f32::NAN] {
            assert!(transform_origin_xy("50% 50%",Rect { w:bad,..rect },1.0).is_none());
            assert!(transform_origin_xy("0 0",Rect { x:bad,..rect },1.0).is_none());
        }
        let box_ = box_with_origin("1em 2px", "content-box");
        let effect = effect_for(&box_).unwrap();
        close(effect.origin_x,460.0);close(effect.origin_y,540.0);
        assert_eq!(box_.style.get("transform-origin"),Some("1em 2px"));
    }

    #[test]
    fn zoom_scales_absolute_origin_terms_once_and_preserves_percentage_basis() {
        for zoom in [0.5_f64,1.0,2.0,4.0] {
            for (css,x,y) in [
                ("12px -4px",12.0*zoom,-4.0*zoom),
                ("50% 25%",60.0*zoom,20.0*zoom),
                ("calc(100% - 10px) calc(50% + 2px)",110.0*zoom,42.0*zoom),
                ("min(10px, 50%) clamp(20px, 50%, 10px)",10.0*zoom,20.0*zoom),
            ] {
                let mut box_ = box_with_origin(css,"border-box");
                box_.style.set("zoom",&zoom.to_string());
                box_.border_box.w *= zoom as f32; box_.border_box.h *= zoom as f32;
                let effect = effect_for(&box_).unwrap();
                close(effect.origin_x,400.0+x as f32); close(effect.origin_y,500.0+y as f32);
            }
        }
        let mut dom = amazing_dom::Dom::new();
        let parent = dom.create_element(amazing_dom::NodeIdx(0),"div");
        let child = dom.create_element(parent,"span");
        let styles = amazing_css::compute(&dom,&[amazing_css::parse_stylesheet(
            "div{zoom:2}span{display:block;zoom:2;font-size:10px;transform:rotate(90deg);transform-origin:1em .5em}")]);
        let style = styles.get(child).unwrap().clone();
        assert_eq!(style.effective_zoom(),4.0);
        assert_eq!(style.get("transform-origin"),Some("10px 5px"));
        let mut box_ = LayoutBox::new(Some(child),amazing_layout::Display::Block,style,40.0,40.0);
        box_.border_box = Rect { x:400.0,y:500.0,w:480.0,h:320.0 };
        let effect = effect_for(&box_).unwrap();
        close(effect.origin_x,440.0); close(effect.origin_y,520.0);
    }

    #[test]
    fn z_and_perspective_origins_do_not_invent_two_dimensional_offsets() {
        let base = box_with_origin("25% 75%", "border-box");
        let expected = effect_for(&base).unwrap().matrix_about_origin();
        for z in ["0px", "100px", "-40px", "calc(10px + 2px)"] {
            let mut box_ = box_with_origin(&format!("25% 75% {z}"), "border-box");
            box_.style.set("perspective", "200px");
            box_.style.set("perspective-origin", "0% 100%");
            assert_eq!(effect_for(&box_).unwrap().matrix_about_origin(),expected);
            let commands = build_display_list(&box_);
            assert!(commands.iter().any(|command| matches!(command,PaintCommand::BeginGroup(effect)
                if effect.matrix_about_origin()==expected)));
        }
    }
}
