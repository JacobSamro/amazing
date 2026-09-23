//! Provider-neutral paint command construction.
//!
//! The builder is the only stage that walks [`LayoutBox`]. Rasterization
//! consumes the flattened command stream, so stacking/effect policy stays
//! separate from pixels and nested stacking contexts remain atomic.

use amazing_layout::{Flow, LayoutBox};

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
    pub opacity: f32,
    pub transform: Affine,
    pub origin_x: f32,
    pub origin_y: f32,
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
            false,
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
    commands.push(PaintCommand::PaintContent(root));
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
    in_positioned_phase: bool,
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

    let starts_positioned_phase = !in_positioned_phase && is_positioned(box_);
    let positioned_phase = in_positioned_phase || starts_positioned_phase;
    if starts_positioned_phase {
        for clip in inherited_clips.iter().copied() {
            buckets
                .middle_phase
                .push(MiddlePhaseItem::Command(PaintCommand::PushOverflowClip(
                    clip,
                )));
        }
    }
    push_middle_command(
        positioned_phase,
        middle,
        buckets,
        PaintCommand::PaintDecorations(
            box_,
            crate::background::BackgroundPlan::from_style(&box_.style),
        ),
    );
    let clips = clips_overflow(box_);
    if clips {
        push_middle_command(
            positioned_phase,
            middle,
            buckets,
            PaintCommand::PushOverflowClip(box_),
        );
        inherited_clips.push(box_);
    }
    let Some(controls) = control_commands(box_) else {
        return false;
    };
    for control in controls {
        push_middle_command(positioned_phase, middle, buckets, control);
    }
    push_middle_command(
        positioned_phase,
        middle,
        buckets,
        PaintCommand::PaintContent(box_),
    );
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
            positioned_phase,
        ) {
            return false;
        }
    }
    if clips {
        inherited_clips.pop();
        push_middle_command(positioned_phase, middle, buckets, PaintCommand::PopClip);
    }
    if starts_positioned_phase {
        for _ in inherited_clips.iter() {
            buckets
                .middle_phase
                .push(MiddlePhaseItem::Command(PaintCommand::PopClip));
        }
    }
    true
}

fn push_middle_command<'a>(
    positioned_phase: bool,
    flow: &mut Vec<PaintCommand<'a>>,
    buckets: &mut ContextBuckets<'a>,
    command: PaintCommand<'a>,
) {
    if positioned_phase {
        buckets.middle_phase.push(MiddlePhaseItem::Command(command));
    } else {
        flow.push(command);
    }
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
    children.sort_by_key(|child| child.node.map(|node| node.0).unwrap_or(0));
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
    let transform = box_.style.get("transform").and_then(parse_transform);
    if opacity >= 1.0 && transform.is_none() {
        return None;
    }
    let border = box_.border_box;
    Some(GroupEffect {
        opacity,
        transform: transform.unwrap_or(Affine::IDENTITY),
        origin_x: border.x + border.w * 0.5,
        origin_y: border.y + border.h * 0.5,
    })
}

fn parse_transform(value: &str) -> Option<Affine> {
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
                e: parse_length(values.first()?)?,
                f: values
                    .get(1)
                    .and_then(|value| parse_length(value))
                    .unwrap_or(0.0),
                ..Affine::IDENTITY
            },
            "translatex" if values.len() == 1 => Affine {
                e: parse_length(values.first()?)?,
                ..Affine::IDENTITY
            },
            "translatey" if values.len() == 1 => Affine {
                f: parse_length(values.first()?)?,
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
    use amazing_layout::{replaced::ReplacedKind, Display};
    let mut commands = Vec::new();
    for fragment in box_.styled_replaced_fragments() {
        if matches!(fragment.kind, ReplacedKind::Image | ReplacedKind::Svg)
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
