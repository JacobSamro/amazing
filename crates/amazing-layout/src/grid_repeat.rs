//! Grid 2 §7.2.3.2 repetition and post-placement empty-track collapse.
//! Keep explicit line indices stable while sizing only non-collapsed tracks.
use super::*;

pub(super) fn repeat_counts(tokens: &[String], ctx: &RepeatCtx, allow_auto: bool) -> Vec<usize> {
    let mut fixed_sum = 0.0;
    let mut fixed_count = 0;
    for token in tokens {
        if token.starts_with('[') { continue; }
        if let Some((count, list)) = repeat_parts(token) {
            if let Ok(count) = count.trim().parse::<usize>() {
                let tracks = parse_track_list(list, ctx, false);
                let count = count.min(MAX_GRID_TRACKS);
                fixed_count += count * tracks.len();
                fixed_sum += count as f32 * tracks.iter()
                    .map(|t| t.repeat_size().unwrap_or(0.0)).sum::<f32>();
            }
        } else {
            fixed_count += 1;
            fixed_sum += parse_track_size(token, ctx.font_px, ctx.content_w, ctx.zoom)
                .repeat_size().unwrap_or(0.0);
        }
    }
    tokens.iter().map(|token| {
        let Some((count, list)) = repeat_parts(token) else { return 1; };
        let count = count.trim();
        if let Ok(count) = count.trim().parse::<usize>() {
            return count.clamp(1, MAX_GRID_TRACKS);
        }
        if !allow_auto || !(count.eq_ignore_ascii_case("auto-fill") || count.eq_ignore_ascii_case("auto-fit")) {
            return 1;
        }
        let Some(available) = ctx.repeat_available else { return 1; };
        let tracks = parse_track_list(list, ctx, false);
        let Some(sizes) = tracks.iter().map(|t| t.repeat_size()).collect::<Option<Vec<_>>>() else {
            return 1;
        };
        // A 1px floor is used only for counting, never for actual track sizing.
        let unit = sizes.iter().map(|s| s.max(1.0)).sum::<f32>() + ctx.gap * sizes.len() as f32;
        if unit <= 0.0 { return 1; }
        let available = available + ctx.gap - fixed_sum - ctx.gap * fixed_count as f32;
        ((available.max(0.0) / unit).floor() as usize).clamp(1, MAX_GRID_TRACKS)
    }).collect()
}

pub(super) fn auto_fit_tracks(value: &str, ctx: &RepeatCtx) -> Vec<bool> {
    if value.eq_ignore_ascii_case("none") || contains_nested_repeat(value) { return Vec::new(); }
    let tokens = top_tokens(value);
    let counts = repeat_counts(&tokens, ctx, true);
    let mut mask = Vec::new();
    for (token, count) in tokens.iter().zip(counts) {
        if token.starts_with('[') { continue; }
        let (count, fit) = if let Some((kind, list)) = repeat_parts(token) {
            (count * parse_track_list(list, ctx, false).len(), kind.trim().eq_ignore_ascii_case("auto-fit"))
        } else { (1, false) };
        mask.extend(std::iter::repeat_n(fit, count.min(MAX_GRID_TRACKS - mask.len())));
        if mask.len() == MAX_GRID_TRACKS { break; }
    }
    mask
}

pub(super) struct ActiveAxis {
    collapsed: Vec<bool>,
    // prefix[i] maps an original grid line to its compact sizing line.
    prefix: Vec<usize>,
}

impl ActiveAxis {
    pub(super) fn new(count: usize, auto_fit: &[bool], occupied: impl Iterator<Item = (usize, usize)>) -> Self {
        let mut collapsed = (0..count).map(|i| auto_fit.get(i).copied().unwrap_or(false)).collect::<Vec<_>>();
        for (start, span) in occupied {
            for entry in collapsed.iter_mut().skip(start).take(span) { *entry = false; }
        }
        let mut prefix = vec![0];
        for value in &collapsed { prefix.push(prefix.last().unwrap() + usize::from(!value)); }
        Self { collapsed, prefix }
    }

    /// Grid Lanes sizes before placement: reserve explicit spans, then the
    /// first N remaining tracks, where N is the sum of automatic spans.
    pub(super) fn for_lanes(count: usize, auto_fit: &[bool], placements: impl Iterator<Item = (Option<u32>, u32)>) -> Self {
        let placements = placements.collect::<Vec<_>>();
        let mut occupied = vec![false; count];
        let mut automatic = 0usize;
        for (line, span) in placements {
            if let Some(line) = line {
                for entry in occupied.iter_mut().skip(line.saturating_sub(1) as usize).take(span as usize) {
                    *entry = true;
                }
            } else {
                automatic = automatic.saturating_add(span as usize);
            }
        }
        for entry in &mut occupied {
            if !*entry && automatic > 0 { *entry = true; automatic -= 1; }
        }
        Self::new(count, auto_fit, occupied.into_iter().enumerate()
            .filter_map(|(index, used)| used.then_some((index, 1))))
    }

    pub(super) fn lane_positions(&self) -> Vec<f32> {
        self.collapsed.iter().map(|collapsed| if *collapsed { f32::INFINITY } else { 0.0 }).collect()
    }

    pub(super) fn gaps(&self, gap: f32) -> f32 {
        gap * self.prefix.last().unwrap().saturating_sub(1) as f32
    }

    pub(super) fn size(&self, kinds: &[TrackSize], gap: f32, available: Option<f32>, contribs: &[Contrib]) -> Vec<f32> {
        let compact = kinds.iter().zip(&self.collapsed).filter_map(|(kind, collapsed)| (!collapsed).then_some(*kind)).collect::<Vec<_>>();
        let contribs = contribs.iter().map(|c| {
            let start = c.start.min(kinds.len());
            let end = (c.start + c.span).min(kinds.len());
            Contrib { start: self.prefix[start], span: self.prefix[end] - self.prefix[start], outer: c.outer }
        }).collect::<Vec<_>>();
        let sizes = size_axis(&compact, gap, available, &contribs);
        self.collapsed.iter().enumerate().map(|(i, collapsed)| {
            if *collapsed { 0.0 } else { sizes[self.prefix[i]] }
        }).collect()
    }

    pub(super) fn stretch(&self, sizes: &mut [f32], kinds: &[TrackSize], available: f32, gap: f32) {
        let autos = kinds.iter().enumerate().filter_map(|(i, kind)|
            (!self.collapsed[i] && kind.has_auto_max()).then_some(i)).collect::<Vec<_>>();
        let spare = available - sizes.iter().sum::<f32>() - self.gaps(gap);
        if spare > 0.0 && !autos.is_empty() {
            let each = spare / autos.len() as f32;
            for i in autos { sizes[i] += each; }
        }
    }

    pub(super) fn positions(&self, sizes: &[f32], gap: f32, pack: Pack, available: f32, origin: f32, reverse: bool) -> Vec<f32> {
        let active = *self.prefix.last().unwrap();
        let free = available - sizes.iter().sum::<f32>() - self.gaps(gap);
        let (lead, between) = pack_pair(pack, free, active);
        let mut cursor = lead;
        let mut positions = Vec::with_capacity(sizes.len());
        for (i, size) in sizes.iter().enumerate() {
            positions.push(if reverse { origin + available - cursor - size } else { origin + cursor });
            if !self.collapsed[i] {
                cursor += size;
                if self.prefix[i + 1] < active { cursor += gap + between; }
            }
        }
        positions
    }
}
