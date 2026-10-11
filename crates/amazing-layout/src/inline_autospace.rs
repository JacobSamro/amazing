//! Actual formatting-owner context for the native CSS Text autospace planner.
//! Classification, bidi cluster coverage and spacing mutation remain in Text.
use amazing_css::{ComputedStyle, StyledDom, selectors::PseudoElement};
use amazing_dom::{Dom, NodeIdx};
use amazing_text::{AutospaceContext, AutospaceMode, TextAutospace, TextStyle, TextSystem,
    Paragraph, TextSpan, ShapingBoundary, ShapedLine, AutospaceAdjustment, AutospaceError};
use std::ops::Range;
use std::sync::Arc;

#[derive(Clone)]
pub struct AutospaceScope {
    pub(crate) node: NodeIdx,
    pub(crate) pseudo: Option<PseudoElement>,
    pub(crate) style: ComputedStyle,
    pub(crate) font: TextStyle,
    pub(crate) first_line: Option<(ComputedStyle, TextStyle)>,
}

/// The retained-layout parity oracle compares Debug output. Box styles are
/// updated in place by retained opacity/containment updates, so the cloned
/// snapshot is deliberately outside the identity printed here; the policy
/// and shaping identity that affect layout are printed instead.
impl std::fmt::Debug for AutospaceScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AutospaceScope").field("node", &self.node).field("pseudo", &self.pseudo)
            .field("policy", &policy(&self.style)).field("font", &self.font)
            .field("first_line_policy", &self.first_line.as_ref().map(|(style, font)| (policy(style), font))).finish()
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Binding {
    pub range: Range<usize>,
    pub scopes: Arc<[Arc<AutospaceScope>]>,
}

pub(crate) fn policy(style: &ComputedStyle) -> TextAutospace {
    let value = match style.typed("text-autospace") {
        Some(amazing_css::values::ComputedValue::TextAutospace(value)) => *value,
        _ => style.get("text-autospace").and_then(amazing_css::TextAutospace::parse).unwrap_or_default(),
    }.policy();
    TextAutospace { ideograph_alpha: value.ideograph_alpha,
        ideograph_numeric: value.ideograph_numeric, punctuation: value.punctuation,
        mode: match value.mode { amazing_css::AutospaceMode::Insert => AutospaceMode::Insert,
            amazing_css::AutospaceMode::Replace => AutospaceMode::Replace } }
}

thread_local! {
    /// Per-build memo of ancestor scopes. A scope clones a whole ComputedStyle
    /// and resolves font metrics, so rebuilding the ancestor chain for every
    /// text node costs O(nodes * depth) clones. The memo is cleared at every
    /// box-tree build entry, so retained updates never see a stale snapshot.
    static MEMO: std::cell::RefCell<ScopeMemo> = std::cell::RefCell::new(ScopeMemo::default());
}

#[derive(Default)]
struct ScopeMemo {
    styled: usize,
    scopes: std::collections::HashMap<NodeIdx, Arc<AutospaceScope>>,
    chains: std::collections::HashMap<NodeIdx, Arc<[Arc<AutospaceScope>]>>,
}

pub(crate) fn reset_memo() {
    MEMO.with(|memo| *memo.borrow_mut() = ScopeMemo::default());
}

fn make_scope(dom: &Dom, styled: &StyledDom, node: NodeIdx, style: &ComputedStyle,
    pseudo: Option<PseudoElement>) -> AutospaceScope {
    AutospaceScope {
        node, pseudo, style: style.clone(),
        font: crate::font_metric_text_style(dom, node, style,
            crate::values::resolve_used_font_size(style, crate::values::ROOT_FONT_SIZE)),
        first_line: styled.get_pseudo(node, PseudoElement::FirstLine).map(|first| (first.clone(),
            crate::font_metric_text_style(dom, node, first,
                crate::values::resolve_used_font_size(first, crate::values::ROOT_FONT_SIZE)))),
    }
}

/// Capture real ancestor styles once when the arena and style epoch are live.
/// The generated pseudo has its own identity before its originating element.
/// Scopes built from the document's own styles are shared between text nodes.
pub(crate) fn capture(dom: &Dom, styled: &StyledDom, owner: NodeIdx,
    leaf: &ComputedStyle, pseudo: Option<PseudoElement>) -> Arc<[Arc<AutospaceScope>]> {
    let key = styled as *const StyledDom as usize;
    let shared = pseudo.is_none() && styled.get(owner).is_some_and(|style| std::ptr::eq(style, leaf));
    let sync = |memo: &mut ScopeMemo| if memo.styled != key { *memo = ScopeMemo { styled: key, ..ScopeMemo::default() }; };
    if shared {
        let hit = MEMO.with(|memo| { let mut memo = memo.borrow_mut(); sync(&mut memo); memo.chains.get(&owner).cloned() });
        if let Some(chain) = hit { return chain; }
    }
    let scope_for = |node: NodeIdx, style: &ComputedStyle| -> Arc<AutospaceScope> {
        let hit = MEMO.with(|memo| { let mut memo = memo.borrow_mut(); sync(&mut memo); memo.scopes.get(&node).cloned() });
        if let Some(scope) = hit { return scope; }
        let scope = Arc::new(make_scope(dom, styled, node, style, None));
        MEMO.with(|memo| memo.borrow_mut().scopes.insert(node, Arc::clone(&scope)));
        scope
    };
    let mut scopes = Vec::new();
    if pseudo.is_some() { scopes.push(Arc::new(make_scope(dom, styled, owner, leaf, pseudo))); }
    let mut cursor = Some(owner);
    while let Some(node) = cursor {
        if let Some(style) = styled.get(node) {
            if node == owner && pseudo.is_none() && !shared {
                scopes.push(Arc::new(make_scope(dom, styled, node, leaf, None)));
            } else {
                scopes.push(scope_for(node, style));
            }
        }
        cursor = dom.get(node).parent;
    }
    let chain: Arc<[Arc<AutospaceScope>]> = scopes.into();
    if shared { MEMO.with(|memo| memo.borrow_mut().chains.insert(owner, Arc::clone(&chain))); }
    chain
}

fn probe_advance(text: &dyn TextSystem, style: &TextStyle, character: &str, owner: u32) -> Result<f32, AutospaceError> {
    let mut style = style.clone();
    style.autospace = TextAutospace::NONE;
    let spans = [TextSpan { range: 0..character.len(), source: owner,
        style, boundary_before: ShapingBoundary::Hard }];
    let shaped = text.shape(Paragraph { text: character, spans: &spans,
        sources: &[], bidi_scopes: &[], base_direction: spans[0].style.direction }).map_err(|_| AutospaceError::InvalidMetric)?;
    let width = shaped.runs.iter().flat_map(|run| &run.glyphs)
        .map(|glyph| crate::inline::inline_advance(spans[0].style.writing_mode, glyph.advance_x, glyph.advance_y)).sum::<f32>();
    if width.is_finite() && width >= 0.0 { Ok(width) } else { Err(AutospaceError::InvalidMetric) }
}

fn binding<'a>(bindings: &'a [Binding], range: &Range<usize>) -> Option<&'a Binding> {
    bindings.iter().find(|binding| binding.range.contains(&range.start))
}

fn owner<'a>(bindings: &'a [Binding], left: &Range<usize>, right: &Range<usize>) -> Option<&'a AutospaceScope> {
    let left = binding(bindings, left)?;
    let right = binding(bindings, right)?;
    left.scopes.iter().find(|scope| right.scopes.iter().any(|other|
        other.node == scope.node && other.pseudo == scope.pseudo)).map(|scope| &**scope)
}

pub(crate) fn paint_scope<'a>(bindings: &'a [Binding], receipt: &AutospaceAdjustment) -> Option<&'a AutospaceScope> {
    let scope = owner(bindings, &receipt.after_cluster, &receipt.before_cluster)?;
    (scope.node.0 == receipt.owner).then_some(scope)
}

pub(crate) fn context(bindings: &[Binding], left: &Range<usize>, right: &Range<usize>,
    text: &dyn TextSystem, adjoining: bool, first_line: bool) -> Result<Option<AutospaceContext>, AutospaceError> {
    let Some(scope) = owner(bindings, left, right) else { return Ok(None); };
    let (style, font) = if first_line {
        scope.first_line.as_ref().map(|(style, font)| (style, font)).unwrap_or((&scope.style, &scope.font))
    } else { (&scope.style, &scope.font) };
    let resolved_policy = policy(style);
    if !resolved_policy.ideograph_alpha && !resolved_policy.ideograph_numeric && !resolved_policy.punctuation {
        return Ok(None);
    }
    let adjusted = crate::font_policy::adjusted_style(font, crate::font_policy::size_adjust_request(style), text);
    let used = adjusted.as_ref().unwrap_or(font);
    let ic = text.font_relative_metrics(used).ic_advance;
    if !ic.is_finite() || ic < 0.0 { return Err(AutospaceError::InvalidMetric); }
    let french = font.language.as_deref().is_some_and(|lang|
        lang.split('-').next().is_some_and(|part| part.eq_ignore_ascii_case("fr")));
    Ok(Some(AutospaceContext { owner: scope.node.0, policy: resolved_policy, ic_advance: ic,
        nbsp_advance: if french { probe_advance(text, used, "\u{00a0}", scope.node.0)? } else { 0.0 },
        narrow_nbsp_advance: if french { probe_advance(text, used, "\u{202f}", scope.node.0)? } else { 0.0 },
        french, adjoining }))
}

/// Fonts are selected by the original provider. Only genuine qualifying
/// boundaries interrupt optional ligatures, without changing any text bytes.
pub(crate) fn protect_ligatures(text: &str, spans: &mut Vec<TextSpan>, bindings: &[Binding],
    system: &dyn TextSystem, adjoining: impl Fn(&Range<usize>, &Range<usize>) -> bool,
    first_line_eligible: bool) -> Result<(), AutospaceError> {
    if !amazing_text::autospace_possible(text) { return Ok(()); }
    let mut metric_error = None;
    let mut boundaries = Vec::new();
    for first_line in [false, true].into_iter().filter(|first| !*first || first_line_eligible) {
        boundaries.extend(amazing_text::autospace_shaping_boundaries(text,
        |range| spans.iter().find(|span| span.range.contains(&range.start)).is_some_and(|span|
            span.style.writing_mode.uses_vertical_metrics()
                && (span.style.text_orientation == amazing_text::TextOrientation::Upright
                    || span.style.text_combine == amazing_text::TextCombine::All)),
        |left, right| match context(bindings, left, right, system, adjoining(left, right), first_line) {
            Ok(value) => value, Err(error) => { metric_error = Some(error); None }
        }));
    }
    boundaries.sort_unstable(); boundaries.dedup();
    if let Some(error) = metric_error { return Err(error); }
    if boundaries.is_empty() { return Ok(()); }
    let old = std::mem::take(spans);
    for span in old {
        let mut start = span.range.start;
        for &boundary in boundaries.iter().filter(|&&offset| offset > span.range.start && offset < span.range.end) {
            let mut part = span.clone();
            part.range = start..boundary;
            if start != span.range.start { part.boundary_before = ShapingBoundary::Hard; }
            spans.push(part);
            start = boundary;
        }
        let mut part = span;
        if start != part.range.start { part.boundary_before = ShapingBoundary::Hard; }
        part.range.start = start;
        spans.push(part);
    }
    Ok(())
}

pub(crate) fn apply(line: &mut ShapedLine, text: &str, bindings: &[Binding], system: &dyn TextSystem,
    adjoining: impl Fn(&Range<usize>, &Range<usize>) -> bool,
    transparent: impl Fn(&Range<usize>) -> bool, first_line: bool)
    -> Result<Vec<AutospaceAdjustment>, AutospaceError> {
    if bindings.is_empty() { return Ok(Vec::new()); }
    // No pair anywhere in the paragraph can qualify under any policy.
    if !amazing_text::autospace_possible(text) { return Ok(Vec::new()); }
    // Inter-script candidates require non-ASCII text; layout placeholders are
    // not characters. ASCII-only text without punctuation policy stays on the
    // unchanged provider path, including explicit breaks and manual controls.
    if text.chars().all(|ch| ch.is_ascii() || ch == '\u{fffc}')
        && bindings.iter().flat_map(|binding| binding.scopes.iter())
            .all(|scope| !policy(&scope.style).punctuation && (!first_line
                || scope.first_line.as_ref().is_none_or(|(style, _)| !policy(style).punctuation))) { return Ok(Vec::new()); }
    // Real zero-width inline scope markers are transparent to spacing. They
    // remain in the provider's text and glyph runs; only the visual-unit input
    // omits them. Atomic content and nonzero edges remain genuine barriers.
    // Each forced paragraph is independent even during provisional shaping.
    let mut paragraph_start = 0;
    let mut plans = Vec::new();
    for paragraph_end in text.char_indices().filter_map(|(offset, ch)|
        matches!(ch, '\n' | '\r').then_some(offset)).chain(std::iter::once(text.len())) {
        let visual = line.visual_clusters.iter().filter(|range|
            range.start >= paragraph_start && range.end <= paragraph_end && !transparent(range))
            .cloned().collect::<Vec<_>>();
        let mut metric_error = None;
        let plan = amazing_text::plan_autospace(text, &line.runs, &visual,
            |left, right| match context(bindings, left, right, system,
                adjoining(left, right), first_line && paragraph_start == 0) {
                Ok(value) => value, Err(error) => { metric_error = Some(error); None }
            })?;
        if let Some(error) = metric_error { return Err(error); }
        plans.push(plan);
        paragraph_start = paragraph_end.saturating_add(1);
    }
    let mut receipts = Vec::new();
    for plan in plans { receipts.extend(amazing_text::apply_autospace_plan(text, line, plan)?); }
    Ok(receipts)
}
