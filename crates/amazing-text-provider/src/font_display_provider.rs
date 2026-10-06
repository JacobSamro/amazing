//! Native CSS Fonts display selection, keyed by actual descriptor/loader state.
use super::*;
use amazing_text::GlyphVisibility;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
// Timed providers can be rebuilt while old scenes remain retained. Give each
// semantic revision its own native generation rather than reset local counters.
static DISPLAY_GENERATION: AtomicU64 = AtomicU64::new(1u64 << 63);
impl TextProvider {
    pub fn register_font_face_with_timeline<L: FontFaceLoader>(
        &mut self,
        descriptor: FontFaceDescriptor,
        loader: &mut L,
        timeline: &FontDisplayTimeline,
        now: Duration,
    ) -> Result<Option<FontKey>, TextError> {
        self.register_font_face_with_clock(descriptor, loader, timeline, || now)
    }
    /// Real consumers supply the same monotonic clock before and AFTER decode;
    /// a response arriving just before expiry is not a decoded font at that time.
    pub fn register_font_face_with_clock<L: FontFaceLoader>(
        &mut self,
        descriptor: FontFaceDescriptor,
        loader: &mut L,
        timeline: &FontDisplayTimeline,
        clock: impl Fn() -> Duration,
    ) -> Result<Option<FontKey>, TextError> {
        if timeline.descriptor() != &descriptor {
            return Err(TextError::InvalidStyle);
        }
        let now = clock();
        self.set_font_display_time(now)?;
        let existing = self
            .display_faces
            .iter()
            .position(|face| face.timeline.same(timeline));
        if let Some(key) = existing.and_then(|i| self.display_faces[i].key) {
            return Ok(Some(key));
        }
        if existing.is_none() && self.display_faces.len() >= MAX_FONT_FACE_RULES {
            return Err(TextError::ResourceLimit);
        }
        let order = existing.map_or(self.face_rules.len(), |i| self.display_faces[i].order);
        let mut pending = false;
        let key = self.register_font_face_impl(descriptor.clone(), loader, true, &mut pending)?;
        let completed = clock();
        let clock_result = self.set_font_display_time(completed);
        if clock_result.is_err() {
            timeline.failed(now)?;
            pending = false;
        } else if key.is_some() {
            timeline.loaded(completed)?;
        } else if !pending {
            timeline.failed(completed)?;
        }
        if let Some(key) = key {
            if let Some(metadata) = self
                .fonts
                .iter_mut()
                .find(|font| font.key == key)
                .and_then(|font| font.author_face.as_mut())
            {
                metadata.declaration_order = order;
            }
        }
        let face = font_display::DisplayFace {
            descriptor,
            timeline: timeline.clone(),
            key,
            pending,
            order,
        };
        if let Some(i) = existing {
            self.display_faces[i] = face;
        } else {
            self.display_faces.push(face);
        }
        self.invalidate_display()?;
        clock_result?;
        Ok(key)
    }
    /// Supply the original monotonic page clock before layout/shape. Never reset
    /// timelines when rebuilding a provider. Returned change requires reflow.
    pub fn set_font_display_time(&mut self, now: Duration) -> Result<bool, TextError> {
        if now < self.display_time {
            return Err(TextError::InvalidStyle);
        }
        let mut changed = false;
        for face in &self.display_faces {
            if face.timeline.snapshot(self.display_time)? != face.timeline.snapshot(now)? {
                changed = true;
            }
        }
        if changed {
            self.invalidate_display()?;
        }
        self.display_time = now;
        Ok(changed)
    }
    pub fn font_display_face_state(
        &self,
        timeline: &FontDisplayTimeline,
    ) -> Result<FontDisplayFaceState, TextError> {
        let face = self
            .display_faces
            .iter()
            .find(|face| face.timeline.same(timeline))
            .ok_or(TextError::MissingFont)?;
        let display = timeline.snapshot(self.display_time)?;
        let metrics = if display.phase == FontDisplayPhase::Loaded {
            face.key
                .map_or(FontDisplayMetrics::Fallback, FontDisplayMetrics::Downloaded)
        } else {
            FontDisplayMetrics::Fallback
        };
        Ok(FontDisplayFaceState { display, metrics })
    }
    pub fn next_font_display_deadline(&self) -> Result<Option<Duration>, TextError> {
        let mut earliest = None;
        for face in &self.display_faces {
            if let Some(at) = face.timeline.snapshot(self.display_time)?.next_change {
                earliest = Some(earliest.map_or(at, |old: Duration| old.min(at)));
            }
        }
        Ok(earliest)
    }
    pub(super) fn refresh_display_generation(&mut self) -> Result<(), TextError> {
        let generation = DISPLAY_GENERATION
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                value.checked_add(1)
            })
            .map_err(|_| TextError::ResourceLimit)?;
        self.display_generation = Some(generation);
        self.hidden_fonts.get_mut().clear();
        Ok(())
    }
    fn invalidate_display(&mut self) -> Result<(), TextError> {
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or(TextError::ResourceLimit)?;
        self.refresh_display_generation()?;
        self.raster_cache.get_mut().clear();
        self.raster_cache_bytes.set(0);
        Ok(())
    }
    /// A genuine rendering-font variant, minted from the same provider key
    /// allocator as loaded fonts. It preserves shaping/metrics and rasterizes
    /// with no coverage. It is not an author-set visibility boolean.
    pub(super) fn invisible_font(&self, base: FontKey) -> Result<FontKey, TextError> {
        if let Some(key) = self.hidden_fonts.borrow().get(&base) {
            return Ok(*key);
        }
        if base != FontKey(0) && !self.fonts.iter().any(|font| font.key == base) {
            return Err(TextError::MissingFont);
        }
        let key = FontKey(self.next_font_key.get());
        let next = key.0.checked_add(1).ok_or(TextError::ResourceLimit)?;
        self.hidden_fonts
            .borrow_mut()
            .try_reserve(1)
            .map_err(|_| TextError::ResourceLimit)?;
        self.next_font_key.set(next);
        self.hidden_fonts.borrow_mut().insert(base, key);
        Ok(key)
    }
    pub(super) fn font_display_available(&self, key: FontKey) -> bool {
        self.display_faces
            .iter()
            .find(|face| face.key == Some(key))
            .is_none_or(|face| {
                face.timeline
                    .snapshot(self.display_time)
                    .is_ok_and(|state| state.phase == FontDisplayPhase::Loaded)
            })
    }
    pub(super) fn font_display_visibility(
        &self,
        style: &TextStyle,
        text: &str,
        selected: Option<usize>,
    ) -> Result<GlyphVisibility, TextError> {
        let selected_family = selected.and_then(|i| {
            style.families.iter().position(|family| {
                self.fonts[i]
                    .family
                    .eq_ignore_ascii_case(font_selection::unquote_family(family))
            })
        });
        for (position, family) in style.families.iter().enumerate() {
            if selected_family.is_some_and(|chosen| position > chosen) {
                break;
            }
            let family = font_selection::unquote_family(family);
            let mut chosen = None;
            for (index, face) in self.display_faces.iter().enumerate() {
                if !face.descriptor.family.trim().eq_ignore_ascii_case(family)
                    || (!face.descriptor.unicode_ranges.is_empty()
                        && !text.chars().all(|ch| {
                            face.descriptor
                                .unicode_ranges
                                .iter()
                                .any(|range| range.contains(ch as u32))
                        }))
                {
                    continue;
                }
                let before = face.timeline.snapshot(self.display_time)?;
                if matches!(
                    before.phase,
                    FontDisplayPhase::Failed | FontDisplayPhase::Expired
                ) {
                    continue;
                }
                if let Some(key) = face.key {
                    let Some(font) = self.fonts.iter().find(|font| font.key == key) else {
                        return Err(TextError::MissingFont);
                    };
                    if !font_selection::font_covers(font, text) {
                        continue;
                    }
                }
                let rank = font_matching::rank_descriptors(
                    style,
                    face.descriptor.stretch,
                    face.descriptor.style,
                    face.descriptor.weight,
                );
                if selected_family == Some(position)
                    && selected.is_some_and(|i| {
                        let selected_rank = font_matching::rank(style, &self.fonts[i]);
                        selected_rank < rank
                            || (selected_rank == rank
                                && self.fonts[i].author_face.as_ref().is_some_and(|metadata| {
                                    metadata.declaration_order > face.order
                                }))
                    })
                {
                    continue;
                }
                if chosen.is_none_or(|(_, old)| rank <= old) {
                    chosen = Some((index, rank));
                }
            }
            if let Some((index, _)) = chosen {
                let face = &self.display_faces[index];
                face.timeline.first_use(self.display_time)?;
                let state = face.timeline.snapshot(self.display_time)?;
                if matches!(
                    state.phase,
                    FontDisplayPhase::Failed | FontDisplayPhase::Expired
                ) {
                    continue;
                }
                return Ok(state.visibility);
            }
        }
        Ok(GlyphVisibility::Visible)
    }
}
