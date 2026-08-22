//! Hand-written sfnt/TTF glyph-metrics parser for the Ferric layout engine.
//!
//! References: the classic Apple sfnt "Font File" layout (Apple TN/legacy
//! `Inside Macintosh: Text` font records), ISO/IEC 14496-22 ("Open Font
//! Format") and the Microsoft OpenType specification chapters `head`,
//! `hhea`, `maxp`, `hmtx` and `cmap`. All multi-byte integers in an sfnt
//! container are big-endian; every accessor below decodes them explicitly.
//!
//! The parser is deliberately narrow: it extracts only what text layout
//! needs (vertical metrics, per-character horizontal advances) and nothing
//! about outlines, hinting, kerning or shaping.
//!
//! # Implemented
//!
//! - sfnt table directory: accepts sfnt version `0x00010000`, `'true'`
//!   (0x74727565) and `'OTTO'` (0x4F54544F, CFF outlines present but their
//!   metrics tables remain readable). `'ttcf'` collections are rejected.
//! - `head`: `unitsPerEm` (u16 at offset 18 of the table).
//! - `hhea`: ascender (i16 @4), descender (i16 @6), lineGap (i16 @8),
//!   numberOfHMetrics (u16 @34).
//! - `maxp`: versions 0.5 (`0x00005000`) and 1.0 (`0x00010000`),
//!   `numGlyphs` (u16 @4).
//! - `hmtx`: array of `[advanceWidth u16, lsb i16]` for the first
//!   numberOfHMetrics glyphs; the last advance repeats for the remaining
//!   glyphs (longVector tail, bare u16s).
//! - `cmap`: subtable selection prefers platform 3 / encoding 10 (UCS-4),
//!   then platform 0 (Unicode "default"), then platform 3 / encoding 1
//!   (BMP). Format 4 is required and fully implemented, including the
//!   spec's idRangeOffset arithmetic where the glyph-id read can run past
//!   the current segment into the next segment's idRangeOffset word (this
//!   is defined behaviour, not an out-of-bounds bug — see
//!   `glyph_index_fmt4`). Format 12 is implemented as an optional bonus
//!   and is the only path that resolves non-BMP characters.
//! - [`FallbackFont`]: average-width heuristic so layout works before a
//!   real font is supplied. This is an *approximation*, not metrics.
//! - [`ScaledMetrics`] with a line-height floor of 1.2 x pixel size.
//!   Sign convention: `descent` is NEGATIVE (hhea descender sign is kept),
//!   and `line_height == ascent - descent + line_gap`, floored.
//! - Panic-freedom on arbitrary input: every table access goes through
//!   bounds-checked readers ([`be_u16`]/[`be_u32`]); `parse` returns `None`
//!   instead of panicking, so hostile/truncated font files are safe.
//!
//! # Deferred / deviations
//!
//! - Checksums are never verified (neither the directory nor head.adjustment).
//! - No `glyf`/`loca`/`CFF ` parsing; no rasterization; no kern/GPOS, so
//!   advances are unshaped and unkerned.
//! - cmap formats 0, 2, 6, 13 and 14 are unsupported; if the best-ranked
//!   subtable is neither format 4 nor 12 the next candidate is tried and,
//!   failing all, `parse` returns `None`.
//! - `unitsPerEm` is only required to be non-zero; the spec restricts it to
//!   16..=16384 but we stay lenient to keep parsing odd-but-real fonts.
//! - `numberOfHMetrics > numGlyphs` is clamped down to `numGlyphs` instead
//!   of rejected; a `numberOfHMetrics` of 0 is rejected (spec requires >= 1).
//! - The cmap subtable's own `length` field is ignored: the whole remainder
//!   of the `cmap` table (bounds-clamped to the file) is kept as the
//!   subtable region, and structural validation bounds all array reads.
//! - Format 4 cannot encode code points above U+FFFF; such characters map
//!   to glyph 0 (`.notdef`) unless a format 12 subtable was selected.
//! - `measure` accumulates integer units before scaling, which is exact for
//!   the tested sizes; very long strings (> 2^24 total units) will round
//!   through f32 like any float pipeline.

/// Vertical and scaling fundamentals of a font, in font units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FontMetrics {
    pub units_per_em: u16,
    /// hhea.ascender, font units, positive up.
    pub ascent: i16,
    /// hhea.descender, usually negative (below the baseline).
    pub descent: i16,
    /// hhea.lineGap.
    pub line_gap: i16,
}

/// Which cmap subtable format was selected at parse time.
#[derive(Debug, Clone)]
enum CmapTable {
    Format4(Vec<u8>),
    Format12(Vec<u8>),
}

/// A parsed font: owns copies of the `hmtx` bytes and the selected `cmap`
/// subtable so lookups never borrow the caller's input buffer.
#[derive(Debug, Clone)]
pub struct ParsedFont {
    metrics: FontMetrics,
    num_glyphs: u16,
    num_h_metrics: u16,
    hmtx: Vec<u8>,
    cmap: CmapTable,
}

impl ParsedFont {
    /// Parses raw sfnt/TTF bytes. Returns `None` on malformed input or when
    /// a required table (head/hhea/maxp/hmtx/cmap) is missing or unusable.
    /// Never panics on arbitrary bytes.
    pub fn parse(data: &[u8]) -> Option<ParsedFont> {
        if data.len() < 12 {
            return None;
        }
        // sfntVersion: 0x00010000 (TrueType outlines), 'true', or 'OTTO'.
        // 'ttcf' (collections) is intentionally not supported.
        match be_u32(data, 0)? {
            0x0001_0000 | 0x7472_7565 /* 'true' */ | 0x4F54_544F /* 'OTTO' */ => {}
            _ => return None,
        }
        let num_tables = be_u16(data, 4)? as usize;
        let dir_end = 12usize.checked_add(num_tables.checked_mul(16)?)?;
        if dir_end > data.len() {
            return None;
        }

        // Look up a table by 4-byte tag. The directory's declared length is
        // clamped to the bytes actually present so a lying directory cannot
        // push us out of bounds (reads inside are bounds-checked anyway).
        let find = |tag: [u8; 4]| -> Option<(usize, usize)> {
            for i in 0..num_tables {
                let e = 12 + i * 16; // fits: dir_end <= data.len()
                if data[e..e + 4] != tag {
                    continue;
                }
                let off = be_u32(data, e + 8)? as usize;
                let len = be_u32(data, e + 12)? as usize;
                if off > data.len() {
                    return Some((off, 0));
                }
                let avail = data.len() - off;
                return Some((off, len.min(avail)));
            }
            None
        };

        // --- head: unitsPerEm at offset 18 ---
        let (ho, hl) = find(*b"head")?;
        let head = data.get(ho..ho + hl)?;
        if head.len() < 20 {
            return None;
        }
        let units_per_em = be_u16(head, 18)?;
        if units_per_em == 0 {
            return None; // would divide by zero when scaling
        }

        // --- hhea: vertical metrics + numberOfHMetrics ---
        let (vo, vl) = find(*b"hhea")?;
        let hhea = data.get(vo..vo + vl)?;
        if hhea.len() < 36 {
            return None;
        }
        let ascent = be_i16(hhea, 4)?;
        let descent = be_i16(hhea, 6)?;
        let line_gap = be_i16(hhea, 8)?;
        let mut num_h_metrics = be_u16(hhea, 34)?;
        if num_h_metrics == 0 {
            return None; // spec: 1 <= numberOfHMetrics
        }

        // --- maxp: numGlyphs (version 0.5 or 1.0) ---
        let (mo, ml) = find(*b"maxp")?;
        let maxp = data.get(mo..mo + ml)?;
        if maxp.len() < 6 {
            return None;
        }
        match be_u32(maxp, 0)? {
            0x0000_5000 | 0x0001_0000 => {}
            _ => return None,
        }
        let num_glyphs = be_u16(maxp, 4)?;
        if num_glyphs == 0 {
            return None; // a font with no glyphs has no usable hmtx either
        }
        if num_h_metrics as usize > num_glyphs as usize {
            num_h_metrics = num_glyphs; // clamp, see module docs
        }

        // --- hmtx: advances (+lsb) for the first N glyphs ---
        let (to, tl) = find(*b"hmtx")?;
        let hmtx_src = data.get(to..to + tl)?;
        let need = (num_h_metrics as usize).checked_mul(4)?;
        if hmtx_src.len() < need {
            return None;
        }
        let hmtx = hmtx_src.to_vec();

        // --- cmap: rank encoding records, take the best parseable one ---
        let (co, cl) = find(*b"cmap")?;
        let cmap_tbl = data.get(co..co + cl)?;
        if cmap_tbl.len() < 4 {
            return None;
        }
        let n_sub = be_u16(cmap_tbl, 2)? as usize;
        let mut cands: Vec<(u8, usize)> = Vec::new();
        for i in 0..n_sub {
            let r = 4usize.checked_add(i.checked_mul(8)?)?;
            if r + 8 > cmap_tbl.len() {
                return None;
            }
            let pid = be_u16(cmap_tbl, r)?;
            let eid = be_u16(cmap_tbl, r + 2)?;
            let soff = be_u32(cmap_tbl, r + 4)? as usize;
            // Preference per module docs: 3/10 > 0/* > 3/1; other records
            // (symbol 3/0, Microsoft other, etc.) are skipped.
            let score = match (pid, eid) {
                (3, 10) => 3u8,
                (0, _) => 2,
                (3, 1) => 1,
                _ => 0,
            };
            if score > 0 {
                cands.push((score, soff));
            }
        }
        // Stable sort keeps file order among equal-score candidates.
        cands.sort_by(|a, b| b.0.cmp(&a.0));

        let mut selected: Option<CmapTable> = None;
        for (_, soff) in &cands {
            let sub = match cmap_tbl.get(*soff..) {
                Some(s) => s,
                None => continue,
            };
            match be_u16(sub, 0) {
                Some(4) => {
                    if validate_cmap4(sub).is_some() {
                        selected = Some(CmapTable::Format4(sub.to_vec()));
                    }
                }
                Some(12) => {
                    if validate_cmap12(sub).is_some() {
                        selected = Some(CmapTable::Format12(sub.to_vec()));
                    }
                }
                _ => {}
            }
            if selected.is_some() {
                break;
            }
        }
        let cmap = selected?;

        Some(ParsedFont {
            metrics: FontMetrics {
                units_per_em,
                ascent,
                descent,
                line_gap,
            },
            num_glyphs,
            num_h_metrics,
            hmtx,
            cmap,
        })
    }

    /// Scale-independent vertical metrics in font units.
    pub fn metrics(&self) -> &FontMetrics {
        &self.metrics
    }

    /// Horizontal advance for one character in FONT UNITS (not pixels).
    /// Unmapped characters (glyph 0 / `.notdef`, out-of-BMP under format 4,
    /// or glyph ids beyond `numGlyphs`) yield 0.
    pub fn advance(&self, c: char) -> u16 {
        let gid = self.glyph_index(c) as usize;
        if gid == 0 || gid >= self.num_glyphs as usize {
            return 0;
        }
        let n = self.num_h_metrics as usize;
        // First N glyphs: 4-byte [advance, lsb] pairs. Tail glyphs repeat
        // the last advance as bare u16s (hmtx longVector semantics).
        let off = if gid < n {
            Some(gid * 4) // gid <= 65534, product fits usize comfortably
        } else {
            n.checked_mul(4)
                .and_then(|base| (gid - n).checked_mul(2).and_then(|t| base.checked_add(t)))
        };
        match off {
            Some(o) => be_u16(&self.hmtx, o).unwrap_or(0),
            None => 0,
        }
    }

    /// Advance width of `text`, scaled to pixel size `px_size`
    /// (px = units * size / units_per_em), summed over characters.
    /// Non-positive or non-finite sizes yield 0. Units are accumulated as
    /// integers and scaled once so ordinary sizes compare exactly.
    pub fn measure(&self, text: &str, px_size: f32) -> f32 {
        let px = sanitize_px(px_size);
        let mut units: u64 = 0;
        for ch in text.chars() {
            units += self.advance(ch) as u64;
        }
        (units as f32) * px / self.metrics.units_per_em as f32
    }

    /// Character -> glyph id; 0 means unmapped/.notdef.
    fn glyph_index(&self, c: char) -> u16 {
        let code = c as u32;
        match &self.cmap {
            CmapTable::Format4(d) => {
                if code > 0xFFFF {
                    0 // format 4 is BMP-only by construction
                } else {
                    glyph_index_fmt4(d, code).unwrap_or(0)
                }
            }
            CmapTable::Format12(d) => glyph_index_fmt12(d, code).unwrap_or(0),
        }
    }
}

// ---------------------------------------------------------------------------
// cmap format 4 (segment mapping to delta values)
// ---------------------------------------------------------------------------

/// Structural validation of a format 4 subtable. Requires the trailing
/// segment to be 0xFFFF..0xFFFF as the spec mandates; glyph-level mapping
/// of that segment is left to the normal lookup (well-formed fonts arrange
/// delta/glyph data so 0xFFFF maps to glyph 0).
fn validate_cmap4(sub: &[u8]) -> Option<()> {
    if sub.len() < 14 {
        return None;
    }
    let seg_x2 = be_u16(sub, 6)?;
    if seg_x2 == 0 || seg_x2 % 2 != 0 {
        return None;
    }
    let sc = seg_x2 as usize / 2; // segCountX2 stores twice the segment count
    // endCode[sc] + reservedPad(2) + startCode[sc] + idDelta[sc] + idRangeOffset[sc]
    let need = 14usize.checked_add(sc.checked_mul(8)?)?.checked_add(2)?;
    if sub.len() < need {
        return None;
    }
    let end_base = 14usize;
    if be_u16(sub, end_base + (sc - 1) * 2)? != 0xFFFF {
        return None; // last endCode must be 0xFFFF
    }
    let start_base = end_base + sc * 2 + 2;
    if be_u16(sub, start_base + (sc - 1) * 2)? != 0xFFFF {
        return None; // last startCode must be 0xFFFF
    }
    Some(())
}

/// Format 4 lookup, following the spec's pseudo-code literally:
///
/// - Segments are sorted by endCode; binary-search for the first segment
///   whose `endCode >= code`, then reject if `code < startCode`.
/// - `idRangeOffset == 0`: glyph = (code + idDelta) mod 65536.
/// - Otherwise the glyph id word sits at byte offset
///   `&idRangeOffset[i] + idRangeOffset[i] + 2*(code - startCode)`.
///   Per the spec this word MAY land past this segment's idRangeOffset
///   entry — inside the next segment's idRangeOffset word or the
///   glyphIdArray proper — and that is the intended behaviour, not an
///   out-of-bounds condition. Because we address raw bytes inside the
///   retained subtable, the cross-segment case falls out naturally; a read
///   past the retained region maps to glyph 0 defensively.
/// - If the glyph read is 0, glyph 0 is returned WITHOUT adding idDelta.
fn glyph_index_fmt4(sub: &[u8], code: u32) -> Option<u16> {
    debug_assert!(code <= 0xFFFF);
    let seg_x2 = be_u16(sub, 6)?;
    if seg_x2 == 0 || seg_x2 % 2 != 0 {
        return None;
    }
    let sc = seg_x2 as usize / 2; // true segment count (segCountX2 is doubled)
    let end_base = 14usize;
    let start_base = end_base + sc * 2 + 2;
    let delta_base = start_base + sc * 2;
    let roff_base = delta_base + sc * 2;

    // First segment index with endCode >= code (ends ascending).
    let mut lo = 0usize;
    let mut hi = sc;
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        let e = be_u16(sub, end_base + mid * 2).unwrap_or(0);
        if code <= e as u32 {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }
    if lo == sc {
        return Some(0); // beyond every endCode: unmapped
    }
    let start = be_u16(sub, start_base + lo * 2)?;
    if code < start as u32 {
        return Some(0); // inside an endCode hole: unmapped
    }
    let delta = be_u16(sub, delta_base + lo * 2)?;
    let roff = be_u16(sub, roff_base + lo * 2)?;
    if roff == 0 {
        Some((code as u16).wrapping_add(delta))
    } else {
        // Word measured in bytes from &idRangeOffset[lo]; may reach into
        // the next segment's words or the glyphIdArray (documented above).
        let step = (code - start as u32) as usize; // code >= start checked
        let addr = roff_base + lo * 2 + roff as usize + step * 2;
        match be_u16(sub, addr) {
            Some(g) if g != 0 => Some(g.wrapping_add(delta)),
            _ => Some(0),
        }
    }
}

// ---------------------------------------------------------------------------
// cmap format 12 (segmented coverage, optional bonus; handles non-BMP)
// ---------------------------------------------------------------------------

fn validate_cmap12(sub: &[u8]) -> Option<()> {
    if sub.len() < 16 {
        return None;
    }
    let ng = be_u32(sub, 12)? as usize;
    let need = 16usize.checked_add(ng.checked_mul(12)?)?;
    if sub.len() < need {
        return None;
    }
    Some(())
}

/// Groups are required by the spec to be sorted by startCharCode, so a
/// binary search applies: find the last group starting at or before
/// `code`, then confirm `code <= endCharCode`.
fn glyph_index_fmt12(sub: &[u8], code: u32) -> Option<u16> {
    let ng = be_u32(sub, 12)? as usize;
    let base = 16usize;
    let mut lo = 0usize;
    let mut hi = ng;
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        let s = be_u32(sub, base + mid * 12)?;
        if code < s {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }
    if lo == 0 {
        return Some(0);
    }
    let i = lo - 1;
    let s = be_u32(sub, base + i * 12)?;
    let e = be_u32(sub, base + i * 12 + 4)?;
    if code > e {
        return Some(0);
    }
    let g = be_u32(sub, base + i * 12 + 8)?.checked_add(code - s)?;
    if g == 0 || g > u16::MAX as u32 {
        return Some(0);
    }
    Some(g as u16)
}

// ---------------------------------------------------------------------------
// Fallback heuristic font
// ---------------------------------------------------------------------------

/// Engine fallback used when no font file is supplied. This produces an
/// AVERAGE-WIDTH APPROXIMATION, not real metrics: widths are guessed per
/// character class (~0.55em for ASCII letters/digits, ~0.30em for ASCII
/// punctuation and whitespace, 1.0em for anything else, e.g. CJK or emoji).
/// Good enough for layout to function; never claim typographic accuracy.
#[derive(Debug, Clone, Copy, Default)]
pub struct FallbackFont;

impl FallbackFont {
    const LETTER_EM: f32 = 0.55;
    const PUNCT_EM: f32 = 0.30;
    const WIDE_EM: f32 = 1.0;

    /// Approximate advance of `text` at pixel size `px_size`.
    pub fn measure(&self, text: &str, px_size: f32) -> f32 {
        let px = sanitize_px(px_size);
        let mut ems = 0.0f32;
        for ch in text.chars() {
            ems += self.width_em(ch);
        }
        ems * px
    }

    /// Plausible vertical metrics for the heuristic: ascent 0.8em,
    /// descent -0.2em, no extra gap; the 1.2em line-height floor does the
    /// rest (see [`ScaledMetrics`]).
    pub fn scaled_metrics(&self, px_size: f32) -> ScaledMetrics {
        let px = sanitize_px(px_size);
        ScaledMetrics {
            ascent: 0.8 * px,
            descent: -0.2 * px,
            line_height: px * 1.2,
        }
    }

    fn width_em(&self, c: char) -> f32 {
        if c.is_ascii_alphanumeric() {
            Self::LETTER_EM
        } else if c.is_ascii_punctuation() || c.is_whitespace() {
            Self::PUNCT_EM
        } else {
            Self::WIDE_EM
        }
    }
}

/// Where the layout engine gets its numbers from.
#[derive(Debug, Clone)]
pub enum FontSource {
    Ttf(ParsedFont),
    Fallback(FallbackFont),
}

/// Vertical metrics already scaled to a pixel size. Sign convention:
/// `descent` is negative-or-zero (hhea descender sign preserved), so a
/// typical line box spans from `-ascent` down through `+|descent|`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScaledMetrics {
    pub ascent: f32,
    pub descent: f32,
    /// ascent - descent + lineGap, scaled; never below 1.2 * px_size.
    pub line_height: f32,
}

impl FontSource {
    /// Constructor for the engine's no-font-file path: a heuristic
    /// average-width font (see [`FallbackFont`]). This is an approximation,
    /// not real metrics.
    pub fn fallback() -> FontSource {
        FontSource::Fallback(FallbackFont)
    }

    /// See [`ParsedFont::measure`] / [`FallbackFont::measure`].
    pub fn measure(&self, text: &str, px_size: f32) -> f32 {
        match self {
            FontSource::Ttf(f) => f.measure(text, px_size),
            FontSource::Fallback(_) => FALLBACK.measure(text, px_size),
        }
    }

    /// Scaled vertical metrics; `line_height` never drops below
    /// 1.2 * px_size (guards against cramped lines from fonts whose
    /// hhea values are unusually tight).
    pub fn scaled_metrics(&self, px_size: f32) -> ScaledMetrics {
        match self {
            FontSource::Ttf(f) => {
                let m = f.metrics();
                let px = sanitize_px(px_size);
                let upem = m.units_per_em as f32;
                // Multiply-then-divide keeps common sizes exact in f32
                // (e.g. 800 * 100 / 1000 == 80.0 bit-for-bit).
                ScaledMetrics {
                    ascent: m.ascent as f32 * px / upem,
                    descent: m.descent as f32 * px / upem,
                    line_height: ((m.ascent - m.descent + m.line_gap) as f32 * px / upem)
                        .max(px * 1.2),
                }
            }
            FontSource::Fallback(_) => FALLBACK.scaled_metrics(px_size),
        }
    }
}

/// Shared unit-value fallback instance (FallbackFont carries no state).
static FALLBACK: FallbackFont = FallbackFont;

// ---------------------------------------------------------------------------
// panic-free big-endian primitives
// ---------------------------------------------------------------------------

fn sanitize_px(px: f32) -> f32 {
    if px.is_finite() && px > 0.0 {
        px
    } else {
        0.0
    }
}

fn be_u16(d: &[u8], off: usize) -> Option<u16> {
    let b = d.get(off..off.checked_add(2)?)?;
    Some(u16::from_be_bytes([b[0], b[1]]))
}

fn be_i16(d: &[u8], off: usize) -> Option<i16> {
    be_u16(d, off).map(|v| v as i16)
}

fn be_u32(d: &[u8], off: usize) -> Option<u32> {
    let b = d.get(off..off.checked_add(4)?)?;
    Some(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

// ---------------------------------------------------------------------------
// Tests: fonts are synthesized entirely in code (no filesystem access).
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn be16(v: u16) -> Vec<u8> {
        v.to_be_bytes().to_vec()
    }

    fn be32(v: u32) -> Vec<u8> {
        v.to_be_bytes().to_vec()
    }

    // --- table builders (field offsets follow the spec layouts) ---

    fn head(upem: u16) -> Vec<u8> {
        let mut t = vec![0u8; 54];
        t[18..20].copy_from_slice(&be16(upem)); // unitsPerEm @18
        t
    }

    fn hhea(asc: i16, desc: i16, gap: i16, num_h_metrics: u16) -> Vec<u8> {
        let mut t = vec![0u8; 36];
        t[4..6].copy_from_slice(&be16(asc as u16)); // ascender @4
        t[6..8].copy_from_slice(&be16(desc as u16)); // descender @6
        t[8..10].copy_from_slice(&be16(gap as u16)); // lineGap @8
        t[34..36].copy_from_slice(&be16(num_h_metrics)); // numberOfHMetrics @34
        t
    }

    fn maxp_v1(num_glyphs: u16) -> Vec<u8> {
        let mut t = vec![0u8; 32];
        t[0..4].copy_from_slice(&be32(0x0001_0000)); // version 1.0
        t[4..6].copy_from_slice(&be16(num_glyphs)); // numGlyphs @4
        t
    }

    fn hmtx(pairs: &[(u16, i16)]) -> Vec<u8> {
        let mut t = Vec::new();
        for &(adv, lsb) in pairs {
            t.extend_from_slice(&be16(adv));
            t.extend_from_slice(&be16(lsb as u16));
        }
        t
    }

    /// Format 4 subtable, five delta-only segments (no glyphIdArray):
    /// 65..67 -> glyphs 1..3 via idDelta 65472 (= -64 mod 65536);
    /// 68..89 and 91..0xFFFE are fillers whose identity glyph ids fall
    /// outside numGlyphs and so behave as unmapped; 90..90 -> 0 (Z);
    /// 0xFFFF..0xFFFF -> 0 via idDelta 1 (wraps 0xFFFF + 1 = 0).
    /// Byte count: 14 + 4*segX2(10) + 2 = 56.
    fn cmap_fmt4_delta() -> Vec<u8> {
        let mut s = Vec::new();
        s.extend_from_slice(&be16(4)); // format
        s.extend_from_slice(&be16(56)); // length
        s.extend_from_slice(&be16(0)); // language
        s.extend_from_slice(&be16(10)); // segCountX2 = 5 segments
        s.extend_from_slice(&be16(0)); // searchRange (not validated)
        s.extend_from_slice(&be16(0)); // entrySelector
        s.extend_from_slice(&be16(0)); // rangeShift
        for e in [67u16, 89, 90, 0xFFFE, 0xFFFF] {
            s.extend_from_slice(&be16(e));
        }
        s.extend_from_slice(&be16(0)); // reservedPad
        for st in [65u16, 68, 90, 91, 0xFFFF] {
            s.extend_from_slice(&be16(st));
        }
        for d in [65472u16, 0, 65446, 0, 1] {
            s.extend_from_slice(&be16(d));
        }
        for _ in 0..5 {
            s.extend_from_slice(&be16(0)); // idRangeOffset all zero
        }
        s
    }

    /// Format 4 subtable exercising the idRangeOffset path: segments
    /// 65..66 and 0xFFFF..0xFFFF; A's and B's glyphs live in a trailing
    /// glyphIdArray addressed from &idRangeOffset[0] (+4 bytes). Layout:
    /// header 14 | endCodes 4 | pad 2 | startCodes 4 | idDeltas 4 |
    /// idRangeOffsets 4 | glyphIdArray 4 => length 36.
    fn cmap_fmt4_idarray() -> Vec<u8> {
        let mut s = Vec::new();
        s.extend_from_slice(&be16(4));
        s.extend_from_slice(&be16(36));
        s.extend_from_slice(&be16(0));
        s.extend_from_slice(&be16(4)); // segCountX2 = 2 segments
        s.extend_from_slice(&be16(0));
        s.extend_from_slice(&be16(0));
        s.extend_from_slice(&be16(0));
        for e in [66u16, 0xFFFF] {
            s.extend_from_slice(&be16(e));
        }
        s.extend_from_slice(&be16(0)); // reservedPad
        for st in [65u16, 0xFFFF] {
            s.extend_from_slice(&be16(st));
        }
        for d in [0u16, 1] {
            s.extend_from_slice(&be16(d));
        }
        for r in [4u16, 0] {
            s.extend_from_slice(&be16(r));
        }
        s.extend_from_slice(&be16(1)); // glyphIdArray[0] -> A
        s.extend_from_slice(&be16(2)); // glyphIdArray[1] -> B
        s
    }

    /// cmap wrapper: one encoding record (platform 3, encoding 1) pointing
    /// at offset 12 (right after the 4-byte header + 8-byte record).
    fn cmap_wrapper(sub: &[u8]) -> Vec<u8> {
        let mut t = Vec::new();
        t.extend_from_slice(&be16(0)); // version
        t.extend_from_slice(&be16(1)); // numTables
        t.extend_from_slice(&be16(3)); // platformID
        t.extend_from_slice(&be16(1)); // encodingID
        t.extend_from_slice(&be32(12)); // subtable offset
        t.extend_from_slice(sub);
        t
    }

    /// Minimal sfnt container: directory (searchRange etc. left zeroed --
    /// nothing validates them) followed by unpadded table bodies.
    fn assemble(tables: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&be32(0x0001_0000)); // sfntVersion
        buf.extend_from_slice(&be16(tables.len() as u16)); // numTables
        buf.extend_from_slice(&be16(0)); // searchRange
        buf.extend_from_slice(&be16(0)); // entrySelector
        buf.extend_from_slice(&be16(0)); // rangeShift
        let mut off = (12 + tables.len() * 16) as u32;
        for (tag, body) in tables.iter() {
            buf.extend_from_slice(*tag);
            buf.extend_from_slice(&be32(0)); // checksum (never verified)
            buf.extend_from_slice(&be32(off));
            buf.extend_from_slice(&be32(body.len() as u32));
            off += body.len() as u32;
        }
        for (_, body) in tables.iter() {
            buf.extend_from_slice(body);
        }
        buf
    }

    const ADVANCES: &[(u16, i16)] = &[(500, 0), (1000, 50), (1100, 60), (1200, 70)];

    fn font_with_line_gap(gap: i16) -> Vec<u8> {
        assemble(&[
            (b"head", head(1000)),
            (b"hhea", hhea(800, -200, gap, 4)),
            (b"maxp", maxp_v1(4)),
            (b"hmtx", hmtx(ADVANCES)),
            (b"cmap", cmap_wrapper(&cmap_fmt4_delta())),
        ])
    }

    fn fixture() -> Vec<u8> {
        font_with_line_gap(300)
    }

    fn parsed() -> ParsedFont {
        ParsedFont::parse(&fixture()).unwrap()
    }

    #[test]
    fn parses_synthetic_font() {
        assert!(ParsedFont::parse(&fixture()).is_some());
    }

    #[test]
    fn metrics_roundtrip() {
        let m = *parsed().metrics();
        assert_eq!(m.units_per_em, 1000);
        assert_eq!(m.ascent, 800);
        assert_eq!(m.descent, -200);
        assert_eq!(m.line_gap, 300);
    }

    #[test]
    fn advances_per_char() {
        let f = parsed();
        assert_eq!(f.advance('A'), 1000);
        assert_eq!(f.advance('B'), 1100);
        assert_eq!(f.advance('C'), 1200);
        // Mapped to glyph 0 by an explicit segment.
        assert_eq!(f.advance('Z'), 0);
        // Falls into filler segments whose identity glyph ids are >= numGlyphs;
        // out-of-range ids are treated as unmapped.
        assert_eq!(f.advance('Q'), 0);
        assert_eq!(f.advance('a'), 0);
        // Below every startCode.
        assert_eq!(f.advance(' '), 0);
        // Non-BMP: format 4 cannot represent it.
        assert_eq!(f.advance('\u{1F980}'), 0);
        // The mandatory 0xFFFF terminator segment maps to 0 (delta 1 wrap).
        assert_eq!(f.advance('\u{FFFF}'), 0);
    }

    #[test]
    fn idrangeoffset_lookup_path() {
        let font = assemble(&[
            (b"head", head(1000)),
            (b"hhea", hhea(800, -200, 300, 4)),
            (b"maxp", maxp_v1(4)),
            (b"hmtx", hmtx(ADVANCES)),
            (b"cmap", cmap_wrapper(&cmap_fmt4_idarray())),
        ]);
        let f = ParsedFont::parse(&font).unwrap();
        // Glyph ids come from the trailing glyphIdArray via the spec's
        // &idRangeOffset[i] + idRangeOffset[i] + 2*(code - startCode) rule.
        assert_eq!(f.advance('A'), 1000);
        assert_eq!(f.advance('B'), 1100);
        assert_eq!(f.advance('C'), 0); // past segment endCode 66
    }

    #[test]
    fn measure_exact_float_compare() {
        let f = parsed();
        // (1000 + 1100) * 50 / 1000, computed as integer-sum * size / upem.
        assert_eq!(f.measure("AB", 50.0), 105.0);
        assert_eq!(f.measure("ABC", 25.0), 82.5);
        assert_eq!(f.measure("", 50.0), 0.0);
        // Degenerate sizes collapse to zero rather than NaN/infinity.
        assert_eq!(f.measure("AB", 0.0), 0.0);
        assert_eq!(f.measure("AB", -5.0), 0.0);
        assert_eq!(f.measure("AB", f32::NAN), 0.0);
    }

    #[test]
    fn rejects_truncated_and_garbage() {
        let fx = fixture();
        // Truncations at several depths.
        assert!(ParsedFont::parse(&fx[..fx.len() - 5]).is_none());
        assert!(ParsedFont::parse(&fx[..40]).is_none());
        assert!(ParsedFont::parse(&fx[..20]).is_none());
        // Zeroed garbage / empty input.
        assert!(ParsedFont::parse(&vec![0u8; 128]).is_none());
        assert!(ParsedFont::parse(&[]).is_none());
    }

    #[test]
    fn rejects_missing_required_table() {
        let full = fixture();
        let cmap_part = cmap_wrapper(&cmap_fmt4_delta());
        // No hmtx.
        let no_hmtx = assemble(&[
            (b"head", head(1000)),
            (b"hhea", hhea(800, -200, 300, 4)),
            (b"maxp", maxp_v1(4)),
            (b"cmap", cmap_part.clone()),
        ]);
        assert!(ParsedFont::parse(&no_hmtx).is_none());
        // No cmap.
        let no_cmap = assemble(&[
            (b"head", head(1000)),
            (b"hhea", hhea(800, -200, 300, 4)),
            (b"maxp", maxp_v1(4)),
            (b"hmtx", hmtx(ADVANCES)),
        ]);
        assert!(ParsedFont::parse(&no_cmap).is_none());
        // No head.
        let no_head = assemble(&[
            (b"hhea", hhea(800, -200, 300, 4)),
            (b"maxp", maxp_v1(4)),
            (b"hmtx", hmtx(ADVANCES)),
            (b"cmap", cmap_part),
        ]);
        assert!(ParsedFont::parse(&no_head).is_none());
        // Sanity: the full set still parses.
        assert!(ParsedFont::parse(&full).is_some());
    }

    #[test]
    fn sfnt_version_variants() {
        let mut otto = fixture();
        otto[0..4].copy_from_slice(b"OTTO");
        assert!(ParsedFont::parse(&otto).is_some());

        let mut true_version = fixture();
        true_version[0..4].copy_from_slice(b"true");
        assert!(ParsedFont::parse(&true_version).is_some());

        let mut ttcf = fixture();
        ttcf[0..4].copy_from_slice(b"ttcf");
        assert!(ParsedFont::parse(&ttcf).is_none());
    }

    #[test]
    fn fallback_measure_sanity() {
        let fb = FallbackFont;
        assert_eq!(fb.measure("", 10.0), 0.0);
        // Space is nonzero but narrower than a letter.
        assert!(fb.measure(" ", 10.0) > 0.0);
        assert!(fb.measure(" ", 10.0) < fb.measure("W", 10.0));
        // Wide (non-ASCII) characters dominate letters.
        assert!(fb.measure("\u{1F980}", 10.0) > fb.measure("W", 10.0));
        // Scales linearly with pixel size.
        assert!((fb.measure("W", 20.0) - 2.0 * fb.measure("W", 10.0)).abs() < 1e-4);
    }

    #[test]
    fn scaled_metrics_ttf_values() {
        let src = FontSource::Ttf(parsed());
        let sm = src.scaled_metrics(100.0);
        // (800 * 100 / 1000) etc. -- exact in f32 by multiply-then-divide.
        assert_eq!(sm.ascent, 80.0);
        assert_eq!(sm.descent, -20.0);
        // (800 + 200 + 300) * 100 / 1000 = 130 > the 120 floor.
        assert_eq!(sm.line_height, 130.0);
    }

    #[test]
    fn line_height_floor_engages() {
        // Natural line height here: (800 + 200 + 100) = 1100 units = 1.1em,
        // below the 1.2em floor at every size.
        let src = FontSource::Ttf(ParsedFont::parse(&font_with_line_gap(100)).unwrap());
        let sm = src.scaled_metrics(100.0);
        assert_eq!(sm.ascent, 80.0);
        assert_eq!(sm.descent, -20.0);
        assert!(sm.line_height >= 1.2 * 100.0);
        assert!(sm.line_height < 121.0);

        let fb = FontSource::fallback().scaled_metrics(10.0);
        assert!(fb.ascent > 0.0 && fb.descent < 0.0);
        assert!((fb.line_height - 12.0).abs() < 1e-3);
    }
}

