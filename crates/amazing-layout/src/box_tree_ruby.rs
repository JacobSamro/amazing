//! Ruby box generation and base/annotation pairing.
use super::*;

fn collect_ruby_text(
    dom: &Dom,
    styled: &StyledDom,
    node: NodeIdx,
    output: &mut Vec<(NodeIdx, String)>,
    budget: &mut usize,
    segment_budget: &mut usize,
) {
    let mut pending = vec![node];
    while let Some(node) = pending.pop() {
        if *budget == 0 || *segment_budget == 0 {
            break;
        }
        *segment_budget -= 1;
        if styled
            .get(node)
            .is_some_and(|style| is_display_none(style) || is_out_of_flow(style))
        {
            continue;
        }
        match &dom.get(node).kind {
            NodeKind::Text { data } | NodeKind::CDATASection { data } => {
                let mut end = data.len().min(*budget);
                while !data.is_char_boundary(end) {
                    end -= 1;
                }
                output.push((node, data[..end].to_owned()));
                *budget -= end;
            }
            NodeKind::Element { .. } => {
                let children: Vec<_> = dom.children(node).take(*segment_budget).collect();
                pending.extend(children.into_iter().rev());
            }
            _ => {}
        }
    }
}

fn ruby_text(
    dom: &Dom,
    styled: &StyledDom,
    owner: NodeIdx,
    source_node: NodeIdx,
    parent_font_size: f32,
    text_budget: &mut usize,
    mapping_budget: &mut usize,
    segment_budget: &mut usize,
) -> RubyText {
    let style = styled.get(owner).cloned().unwrap_or_default();
    let font_size = values::resolve_used_font_size(&style, parent_font_size);
    let mut segments = Vec::new();
    collect_ruby_text(
        dom,
        styled,
        source_node,
        &mut segments,
        text_budget,
        segment_budget,
    );
    let collapses_spaces = !matches!(
        style.get("white-space").unwrap_or("normal"),
        value if value.eq_ignore_ascii_case("pre")
            || value.eq_ignore_ascii_case("pre-wrap")
            || value.eq_ignore_ascii_case("break-spaces")
    );
    let raw_text = segments.iter().map(|(_, source)| source.as_str()).collect();
    let mut text = String::new();
    let mut source_mappings = Vec::new();
    for (source_node, source) in segments {
        let suppressed = source.replace(['\n', '\r', '\u{c}'], " ");
        let (part, mappings) = text_for_white_space(
            &suppressed,
            &style,
            source_node,
            mapping_budget.saturating_sub(source_mappings.len()),
        );
        let skip = usize::from(collapses_spaces && text.ends_with(' ') && part.starts_with(' '));
        let start = text.len();
        if skip != 0 {
            source_mappings.extend(
                mappings
                    .iter()
                    .filter(|mapping| mapping.text_range.start < skip)
                    .map(|mapping| TextSourceMapping {
                        text_range: start - 1..start,
                        source: mapping.source,
                        source_range: mapping.source_range.clone(),
                    }),
            );
        }
        text.push_str(&part[skip..]);
        source_mappings.extend(
            mappings
                .into_iter()
                .filter(|mapping| mapping.text_range.end > skip)
                .map(|mapping| TextSourceMapping {
                    text_range: start + mapping.text_range.start.saturating_sub(skip)
                        ..start + mapping.text_range.end - skip,
                    source: mapping.source,
                    source_range: mapping.source_range,
                }),
        );
        if source_mappings.len() >= *mapping_budget {
            break;
        }
    }
    *mapping_budget = mapping_budget.saturating_sub(source_mappings.len());
    RubyText {
        node: owner,
        raw_text,
        format: ruby_format(&style, font_size),
        nested: Vec::new(),
        text,
        source_mappings,
        font_size,
        style: Box::new(text_style(dom, owner, &style, font_size, true)),
        color: style.get("color").unwrap_or("black").to_owned(),
    }
}

fn empty_ruby_text(
    dom: &Dom,
    styled: &StyledDom,
    owner: NodeIdx,
    parent_font_size: f32,
) -> RubyText {
    let style = styled.get(owner).cloned().unwrap_or_default();
    let font_size = values::resolve_used_font_size(&style, parent_font_size);
    RubyText {
        node: owner,
        raw_text: String::new(),
        format: ruby_format(&style, font_size),
        nested: Vec::new(),
        text: String::new(),
        source_mappings: Vec::new(),
        font_size,
        style: Box::new(text_style(dom, owner, &style, font_size, true)),
        color: style.get("color").unwrap_or("black").to_owned(),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RubyRole {
    Base,
    Annotation,
    BaseContainer,
    AnnotationContainer,
}

fn ruby_role(_dom: &Dom, styled: &StyledDom, node: NodeIdx) -> Option<RubyRole> {
    role_from_display(styled.get(node).and_then(|style| style.get("display")))
}
fn role_from_display(display: Option<&str>) -> Option<RubyRole> {
    match display {
        Some(value) if value.eq_ignore_ascii_case("ruby-base") => Some(RubyRole::Base),
        Some(value) if value.eq_ignore_ascii_case("ruby-text") => Some(RubyRole::Annotation),
        Some(value) if value.eq_ignore_ascii_case("ruby-base-container") => {
            Some(RubyRole::BaseContainer)
        }
        Some(value) if value.eq_ignore_ascii_case("ruby-text-container") => {
            Some(RubyRole::AnnotationContainer)
        }
        _ => None,
    }
}

pub(super) fn is_ruby_container(_dom: &Dom, styled: &StyledDom, node: NodeIdx) -> bool {
    styled
        .get(node)
        .and_then(|style| style.get("display"))
        .is_some_and(|value| value.eq_ignore_ascii_case("ruby"))
}

pub(super) fn ruby_inline(
    dom: &Dom,
    styled: &StyledDom,
    node: NodeIdx,
    parent_font_size: f32,
) -> InlineNode {
    let mut builder = RubyBuilder {
        dom,
        styled,
        text_budget: MAX_RUBY_TEXT_BYTES,
        mapping_budget: MAX_RUBY_SOURCE_MAPPINGS,
        segment_budget: MAX_RUBY_SOURCE_MAPPINGS,
    };
    let columns = builder.columns(node, parent_font_size, 0);
    InlineNode::Ruby {
        node,
        columns,
        overhang_spaces: ruby_overhang(styled, node),
    }
}

fn ruby_overhang(styled: &StyledDom, node: NodeIdx) -> bool {
    // Auto is UA-defined; retain a conservative no-overhang policy. Explicit
    // spaces (and its legacy alias) consume only measured adjacent blank areas.
    styled
        .get(node)
        .and_then(|s| s.get("ruby-overhang"))
        .is_some_and(|s| matches!(s, "spaces" | "none"))
}

struct RubyBuilder<'a> {
    dom: &'a Dom,
    styled: &'a StyledDom,
    text_budget: usize,
    mapping_budget: usize,
    segment_budget: usize,
}

struct RubyEntry {
    pseudo: Option<PseudoElement>,
    role: Option<RubyRole>,
    owner: NodeIdx,
    nodes: Vec<NodeIdx>,
}
struct RubyUnit {
    text: RubyText,
    whitespace: bool,
}
struct RubyLevel {
    units: Vec<RubyUnit>,
    spanning: bool,
}
#[derive(Default)]
struct RubySegment {
    bases: Vec<RubyUnit>,
    annotations: Vec<RubyLevel>,
    direct_annotations: bool,
}

impl RubyBuilder<'_> {
    fn entries(&self, node: NodeIdx) -> Vec<RubyEntry> {
        let mut entries: Vec<RubyEntry> = Vec::new();
        let children: Vec<_> = self
            .dom
            .children(node)
            .take(MAX_RUBY_COLUMNS)
            .filter(|&child| {
                !matches!(
                    self.dom.get(child).kind,
                    NodeKind::Comment { .. } | NodeKind::ProcessingInstruction { .. }
                ) && !self
                    .styled
                    .get(child)
                    .is_some_and(|s| is_display_none(s) || is_out_of_flow(s))
            })
            .collect();
        let whitespace = |node| matches!(&self.dom.get(node).kind, NodeKind::Text { data } | NodeKind::CDATASection { data } if css_whitespace(data));
        let before = self
            .styled
            .generated_content(node, PseudoElement::Before)
            .is_some();
        let after = self
            .styled
            .generated_content(node, PseudoElement::After)
            .is_some();
        for (index, &child) in children.iter().enumerate() {
            if children.len() > 1
                && whitespace(child)
                && ((!before && index == 0) || (!after && index + 1 == children.len()))
            {
                continue;
            }
            if matches!(
                self.dom.get(child).kind,
                NodeKind::Comment { .. } | NodeKind::ProcessingInstruction { .. }
            ) || self
                .styled
                .get(child)
                .is_some_and(|s| is_display_none(s) || is_out_of_flow(s))
            {
                continue;
            }
            let role = ruby_role(self.dom, self.styled, child);
            if role.is_none() && entries.last().is_some_and(|entry| entry.role.is_none()) {
                entries.last_mut().unwrap().nodes.push(child);
            } else {
                entries.push(RubyEntry {
                    pseudo: None,
                    role,
                    owner: if role.is_some() { child } else { node },
                    nodes: vec![child],
                });
            }
        }
        for pseudo in [PseudoElement::Before, PseudoElement::After] {
            let Some(style) = self.styled.get_pseudo(node, pseudo) else {
                continue;
            };
            if is_display_none(style) || is_out_of_flow(style) {
                continue;
            }
            if self
                .styled
                .generated_content(node, pseudo)
                .is_none_or(|c| c.runs.is_empty())
            {
                continue;
            }
            let entry = RubyEntry {
                role: role_from_display(style.get("display")),
                owner: node,
                nodes: Vec::new(),
                pseudo: Some(pseudo),
            };
            if pseudo == PseudoElement::Before {
                entries.insert(0, entry);
            } else {
                entries.push(entry);
            }
        }
        entries
    }

    fn entry_text(&mut self, entry: &RubyEntry, font: f32, depth: usize) -> RubyText {
        let Some(pseudo) = entry.pseudo else {
            return self.text(entry.owner, &entry.nodes, font, depth);
        };
        let style = self
            .styled
            .get_pseudo(entry.owner, pseudo)
            .cloned()
            .unwrap_or_default();
        let size = values::resolve_used_font_size(&style, font);
        let mut result = empty_ruby_text(self.dom, self.styled, entry.owner, font);
        let raw: String = self
            .styled
            .generated_content(entry.owner, pseudo)
            .into_iter()
            .flat_map(|content| &content.runs)
            .filter_map(|run| {
                if let ContentRun::Text(text) = run {
                    Some(text.as_str())
                } else {
                    None
                }
            })
            .collect();
        let mut end = raw.len().min(self.text_budget);
        while !raw.is_char_boundary(end) {
            end -= 1;
        }
        result.raw_text = raw[..end].to_owned();
        self.text_budget -= end;
        result.text = text_for_white_space(
            &result.raw_text.replace(['\n', '\r', '\u{c}'], " "),
            &style,
            entry.owner,
            self.mapping_budget,
        )
        .0;
        result.font_size = size;
        result.style = Box::new(text_style(self.dom, entry.owner, &style, size, true));
        result.format = ruby_format(&style, size);
        result.color = style.get("color").unwrap_or("black").to_owned();
        result
    }

    fn text(&mut self, owner: NodeIdx, nodes: &[NodeIdx], font: f32, depth: usize) -> RubyText {
        let mut result = empty_ruby_text(self.dom, self.styled, owner, font);
        if depth >= 64 || self.text_budget == 0 || self.segment_budget == 0 {
            return result;
        }
        for &node in nodes {
            if is_ruby_container(self.dom, self.styled, node) && node != owner {
                let columns = self.columns(node, result.font_size, depth + 1);
                let mut part = empty_ruby_text(self.dom, self.styled, owner, font);
                part.text.push('\u{fffc}');
                part.raw_text = columns.iter().map(|c| c.base.raw_text.as_str()).collect();
                part.nested.push(crate::NestedRuby {
                    range: 0..3,
                    columns,
                    overhang_spaces: ruby_overhang(self.styled, node),
                });
                append_ruby_text(&mut result, part);
            } else if self.contains_nested(node) {
                let children: Vec<_> = self.dom.children(node).collect();
                let part = self.text(owner, &children, font, depth + 1);
                append_ruby_text(&mut result, part);
            } else {
                let part = ruby_text(
                    self.dom,
                    self.styled,
                    owner,
                    node,
                    font,
                    &mut self.text_budget,
                    &mut self.mapping_budget,
                    &mut self.segment_budget,
                );
                append_ruby_text(&mut result, part);
            }
        }
        // Inlinified empty blocks can still contribute a specified inline size.
        for &node in nodes {
            result.format.minimum_advance = result
                .format
                .minimum_advance
                .max(self.minimum_advance(node, font, 0));
        }
        result
    }

    fn contains_nested(&self, node: NodeIdx) -> bool {
        let mut pending: Vec<_> = self.dom.children(node).collect();
        let mut visited = 0;
        while let Some(child) = pending.pop() {
            if visited >= MAX_RUBY_COLUMNS {
                break;
            }
            visited += 1;
            if is_ruby_container(self.dom, self.styled, child) {
                return true;
            }
            pending.extend(self.dom.children(child));
        }
        false
    }

    fn minimum_advance(&self, node: NodeIdx, parent_font: f32, depth: usize) -> f32 {
        if depth >= 64 {
            return 0.0;
        }
        let style = self.styled.get(node).cloned().unwrap_or_default();
        if is_display_none(&style) || is_out_of_flow(&style) {
            return 0.0;
        }
        let font = values::resolve_used_font_size(&style, parent_font);
        let own = ruby_format(&style, font).minimum_advance;
        self.dom
            .children(node)
            .map(|c| self.minimum_advance(c, font, depth + 1))
            .fold(own, f32::max)
    }

    fn level(&mut self, node: NodeIdx, annotation: bool, font: f32, depth: usize) -> RubyLevel {
        let entries = self.entries(node);
        let spanning = annotation && entries.len() == 1 && entries[0].role.is_none();
        let mut units = Vec::new();
        for (i, entry) in entries.iter().enumerate() {
            if !annotation
                && matches!(
                    entry.role,
                    Some(RubyRole::Annotation | RubyRole::AnnotationContainer)
                )
            {
                // A text box inside a base container needs its own anonymous
                // ruby container, rather than becoming base text at half size.
                let mut base = empty_ruby_text(self.dom, self.styled, node, font);
                let mut nested = new_column(base.clone(), false);
                let note = if entry.role == Some(RubyRole::AnnotationContainer) {
                    self.level(entry.owner, true, font, depth + 1)
                        .units
                        .into_iter()
                        .next()
                        .map(|u| u.text)
                } else {
                    Some(self.entry_text(entry, font, depth + 1))
                };
                nested.annotation = note;
                base.text = "\u{fffc}".into();
                base.nested.push(crate::NestedRuby {
                    range: 0..3,
                    columns: vec![nested],
                    overhang_spaces: true,
                });
                units.push(RubyUnit {
                    text: base,
                    whitespace: false,
                });
                continue;
            }

            let parent_font = if entry.role.is_some() {
                self.styled
                    .get(node)
                    .map_or(font, |s| values::resolve_used_font_size(s, font))
            } else {
                font
            };
            let text = self.entry_text(entry, parent_font, depth);
            let whitespace =
                entry.role.is_none() && entry.pseudo.is_none() && css_whitespace(&text.raw_text);
            if whitespace && (i == 0 || i + 1 == entries.len()) {
                continue;
            }
            units.push(RubyUnit { text, whitespace });
        }
        if annotation && units.is_empty() {
            units.push(RubyUnit {
                text: empty_ruby_text(self.dom, self.styled, node, font),
                whitespace: false,
            });
        }
        RubyLevel { units, spanning }
    }

    fn columns(&mut self, node: NodeIdx, parent_font: f32, depth: usize) -> Vec<crate::RubyColumn> {
        if depth >= 64 || self.segment_budget == 0 {
            return Vec::new();
        }
        self.segment_budget -= 1;
        let style = self.styled.get(node).cloned().unwrap_or_default();
        let font = values::resolve_used_font_size(&style, parent_font);
        let entries = self.entries(node);
        self.pair_entries(node, parent_font, font, entries, depth)
    }

    fn pair_entries(
        &mut self,
        node: NodeIdx,
        parent_font: f32,
        font: f32,
        entries: Vec<RubyEntry>,
        depth: usize,
    ) -> Vec<crate::RubyColumn> {
        let mut segment = RubySegment::default();
        let mut columns = Vec::new();
        let template = empty_ruby_text(self.dom, self.styled, node, parent_font);
        for (index, entry) in entries.iter().enumerate() {
            if columns.len() >= MAX_RUBY_COLUMNS {
                break;
            }
            match entry.role {
                Some(RubyRole::BaseContainer) => {
                    finish_segment(&mut segment, &mut columns, &template);
                    segment.bases = self.level(entry.owner, false, font, depth).units;
                }
                Some(RubyRole::AnnotationContainer) => {
                    segment
                        .annotations
                        .push(self.level(entry.owner, true, font, depth));
                    segment.direct_annotations = false;
                }
                Some(RubyRole::Annotation) => {
                    if !segment.direct_annotations {
                        segment.annotations.push(RubyLevel {
                            units: Vec::new(),
                            spanning: false,
                        });
                    }
                    let text = self.entry_text(entry, font, depth);
                    segment
                        .annotations
                        .last_mut()
                        .unwrap()
                        .units
                        .push(RubyUnit {
                            text,
                            whitespace: false,
                        });
                    segment.direct_annotations = true;
                }
                Some(RubyRole::Base) | None => {
                    let mut text = self.entry_text(entry, font, depth);
                    let whitespace = entry.role.is_none()
                        && entry.pseudo.is_none()
                        && css_whitespace(&text.raw_text);
                    if whitespace {
                        if index == 0 || index + 1 == entries.len() {
                            continue;
                        }
                        let previous = entries[index - 1].role;
                        let next = entries[index + 1].role;
                        if next == Some(RubyRole::AnnotationContainer)
                            || (next == Some(RubyRole::Annotation)
                                && previous != Some(RubyRole::Annotation))
                        {
                            continue;
                        }
                        if previous == Some(RubyRole::Annotation)
                            && next == Some(RubyRole::Annotation)
                        {
                            if let Some(level) = segment.annotations.last_mut() {
                                level.units.push(RubyUnit {
                                    text,
                                    whitespace: true,
                                });
                            }
                            continue;
                        }
                        if !segment.annotations.is_empty()
                            || previous == Some(RubyRole::BaseContainer)
                            || next == Some(RubyRole::BaseContainer)
                        {
                            finish_segment(&mut segment, &mut columns, &template);
                            columns.push(new_column(text, true));
                            continue;
                        }
                    }
                    if !segment.annotations.is_empty() {
                        finish_segment(&mut segment, &mut columns, &template);
                    }
                    if entry.role.is_none() && segment.bases.is_empty() {
                        trim_ruby_start(&mut text);
                    }
                    segment.bases.push(RubyUnit { text, whitespace });
                }
            }
        }
        finish_segment(&mut segment, &mut columns, &template);
        columns.truncate(MAX_RUBY_COLUMNS);
        columns
    }
}

fn css_whitespace(text: &str) -> bool {
    text.chars()
        .all(|c| matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{c}'))
}
fn new_column(base: RubyText, whitespace: bool) -> crate::RubyColumn {
    crate::RubyColumn {
        base,
        annotation: None,
        annotation_span: 1,
        annotation_hidden: false,
        additional_annotations: Vec::new(),
        intra_level_whitespace: whitespace,
    }
}
fn append_ruby_text(result: &mut RubyText, mut part: RubyText) {
    let skip = usize::from(result.text.ends_with(' ') && part.text.starts_with(' '));
    let start = result.text.len();
    result.raw_text.push_str(&part.raw_text);
    result.text.push_str(&part.text[skip..]);
    result.format.minimum_advance = result
        .format
        .minimum_advance
        .max(part.format.minimum_advance);
    for mapping in &mut part.source_mappings {
        mapping.text_range = start + mapping.text_range.start.saturating_sub(skip)
            ..start + mapping.text_range.end.saturating_sub(skip);
    }
    result.source_mappings.extend(part.source_mappings);
    for nested in &mut part.nested {
        nested.range = start + nested.range.start.saturating_sub(skip)
            ..start + nested.range.end.saturating_sub(skip);
    }
    result.nested.extend(part.nested);
}
fn trim_ruby_start(text: &mut RubyText) {
    if matches!(
        text.style.white_space,
        WhiteSpace::Pre | WhiteSpace::PreWrap | WhiteSpace::BreakSpaces
    ) {
        return;
    }
    let count = text.text.len() - text.text.trim_start_matches(' ').len();
    text.text.drain(..count);
    for mapping in &mut text.source_mappings {
        mapping.text_range.start = mapping.text_range.start.saturating_sub(count);
        mapping.text_range.end = mapping.text_range.end.saturating_sub(count);
    }
    text.source_mappings.retain(|m| !m.text_range.is_empty());
    for nested in &mut text.nested {
        nested.range.start = nested.range.start.saturating_sub(count);
        nested.range.end = nested.range.end.saturating_sub(count);
    }
}

fn finish_segment(
    segment: &mut RubySegment,
    output: &mut Vec<crate::RubyColumn>,
    template: &RubyText,
) {
    if segment.bases.is_empty() && segment.annotations.is_empty() {
        return;
    }
    let mut segment = std::mem::take(segment);
    let regular_count = segment
        .bases
        .iter()
        .filter(|u| !u.whitespace)
        .count()
        .max(
            segment
                .annotations
                .iter()
                .filter(|l| !l.spanning)
                .map(|l| l.units.iter().filter(|u| !u.whitespace).count())
                .max()
                .unwrap_or(0),
        )
        .max(1);
    let mut gaps = vec![false; regular_count + 1];
    for units in std::iter::once(&segment.bases).chain(segment.annotations.iter().map(|l| &l.units))
    {
        let mut regular = 0;
        for unit in units {
            if unit.whitespace {
                gaps[regular.min(regular_count)] = true;
            } else {
                regular += 1;
            }
        }
    }
    let mut columns = Vec::new();
    let mut indices = Vec::new();
    let mut gap_indices = vec![None; regular_count + 1];
    for i in 0..=regular_count {
        if gaps[i] {
            gap_indices[i] = Some(columns.len());
            columns.push(new_column(template.clone(), true));
        }
        if i < regular_count {
            indices.push(columns.len());
            columns.push(new_column(template.clone(), false));
        }
    }
    let mut regular = 0;
    for unit in segment.bases.drain(..) {
        let index = if unit.whitespace {
            gap_indices[regular]
        } else {
            let i = indices.get(regular).copied();
            regular += 1;
            i
        };
        if let Some(index) = index {
            columns[index].base = unit.text;
        }
    }
    let mut previous_space = false;
    for column in &mut columns {
        if previous_space {
            trim_ruby_start(&mut column.base);
        }
        if !column.base.text.is_empty() {
            previous_space = column.base.text.ends_with(' ')
                && !matches!(
                    column.base.style.white_space,
                    WhiteSpace::Pre | WhiteSpace::PreWrap | WhiteSpace::BreakSpaces
                );
        }
    }
    for (level_index, mut level) in segment.annotations.drain(..).enumerate() {
        if level.units.first().is_some_and(|u| u.text.format.merge) {
            let mut combined = level.units.first().unwrap().text.clone();
            for unit in level.units.drain(1..) {
                append_ruby_text(&mut combined, unit.text);
            }
            level.units = vec![RubyUnit {
                text: combined,
                whitespace: false,
            }];
            level.spanning = true;
        }
        let mut regular = 0;
        for unit in level.units {
            let index = if level.spanning {
                Some(0)
            } else if unit.whitespace {
                gap_indices[regular.min(regular_count)]
            } else {
                let i = indices.get(regular).copied();
                regular += 1;
                i
            };
            let Some(index) = index else {
                continue;
            };
            let span = if level.spanning { columns.len() } else { 1 };
            let hidden = unit.text.format.visibility == "collapse"
                || (!unit.text.format.merge
                    && unit.text.raw_text
                        == columns[index..index + span]
                            .iter()
                            .map(|c| c.base.raw_text.as_str())
                            .collect::<String>());
            if level_index == 0 {
                columns[index].annotation = Some(unit.text);
                columns[index].annotation_span = span;
                columns[index].annotation_hidden = hidden;
            } else {
                columns[index]
                    .additional_annotations
                    .push(crate::RubyAnnotation {
                        text: unit.text,
                        span,
                        level: level_index,
                        hidden,
                    });
            }
        }
    }
    output.extend(columns);
}

fn ruby_format(style: &ComputedStyle, font_size: f32) -> crate::RubyFormat {
    crate::RubyFormat {
        tab_size: tab_size(style, font_size),
        align: style.get("ruby-align").unwrap_or("space-around").to_owned(),
        position: style.get("ruby-position").unwrap_or("alternate").to_owned(),
        visibility: style.get("visibility").unwrap_or("visible").to_owned(),
        merge: style.get("ruby-merge") == Some("merge"),
        minimum_advance: values::style_length_percentage(
            style,
            "width",
            font_size,
            font_size * 0.6,
        )
        .filter(|v| !v.has_percentage)
        .map_or(0.0, |v| v.px.max(0.0)),
    }
}

pub(super) fn orphan_run(
    dom: &Dom,
    styled: &StyledDom,
    entries: &[FormattingChild],
    start: usize,
    owner: NodeIdx,
    font: f32,
) -> Option<(usize, InlineNode)> {
    let internal = |node| {
        ruby_role(dom, styled, node).is_some()
            && !styled
                .get(node)
                .is_some_and(|s| is_out_of_flow(s) || crate::floats::side(s).is_some())
    };
    if !internal(entries[start].node) {
        return None;
    }
    let mut end = start;
    for (offset, entry) in entries[start..].iter().enumerate() {
        if internal(entry.node) {
            end = start + offset + 1;
        } else if matches!(&dom.get(entry.node).kind, NodeKind::Text { data } | NodeKind::CDATASection { data } if css_whitespace(data))
        {
        } else {
            break;
        }
    }
    let nodes = entries[start..end]
        .iter()
        .map(|entry| RubyEntry {
            pseudo: None,
            role: ruby_role(dom, styled, entry.node),
            owner: if internal(entry.node) {
                entry.node
            } else {
                owner
            },
            nodes: vec![entry.node],
        })
        .collect();
    let mut builder = RubyBuilder {
        dom,
        styled,
        text_budget: MAX_RUBY_TEXT_BYTES,
        mapping_budget: MAX_RUBY_SOURCE_MAPPINGS,
        segment_budget: MAX_RUBY_SOURCE_MAPPINGS,
    };
    let columns = builder.pair_entries(owner, font, font, nodes, 0);
    Some((
        end,
        InlineNode::Ruby {
            node: entries[start].node,
            columns,
            overhang_spaces: true,
        },
    ))
}
