//! Hand-written sfnt/TTF glyph-metrics parser for the Ferric layout engine.
//!
//! References: the classic Apple sfnt "Font File" layout (Apple TN/legacy
//! `Inside Macintosh: Text` font records), ISO/IEC 14496-22 ("Open Font
//! Format") and the Microsoft OpenType specification chapters `head`,
//! `hhea`, `maxp`, `hmtx` and `cmap`. All multi-byte integers in an sfnt
//! container are big-endian; every accessor below decodes them explicitly.
//!
//! The parser extracts what text layout needs (vertical metrics,
//! per-character horizontal advances) plus TrueType glyph outlines for the
//! paint stage's rasterizer ([`GlyphOutline`]). Nothing about hinting,
//! kerning or shaping.
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
//! - `loca` + `glyf`: TrueType quadratic outlines (optional tables; a font
//!   without them still parses for metrics). Short (offset*2) and long
//!   loca formats both supported via `head.indexToLocFormat`. Simple
//!   glyphs decode on/off-curve flags, x/y short deltas, same-value and
//!   repeat flag runs. Composite glyphs are resolved recursively with a
//!   depth cap of 8 and a total point budget: `ARG_1_AND_2_ARE_WORDS`,
//!   `MORE_COMPONENTS`, `WE_HAVE_A_SCALE` / `WE_HAVE_AN_X_AND_Y_SCALE` /
//!   `WE_HAVE_A_TWO_BY_TWO` (F2Dot14), and xy offsets
//!   (`ARGS_ARE_XY_VALUES`). Quadratic runs get the spec's implied
//!   on-curve midpoints inserted between consecutive off-curve points, so
//!   every contour is a closed cyclic chain of on-curve anchors and
//!   quadratic control points ([`OutlinePoint`]) ready for flattening.
//! - [`ParsedFont::glyph_outline`] / [`FontSource::glyph_outline`] hand
//!   the decomposed outline to the rasterizer in font units (y up).
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
//! - CFF outlines (`OTTO` fonts): metrics tables parse but there is no
//!   `glyf`, so `glyph_outline` returns `None`; no rasterization happens
//!   in this crate (see `amazing-paint`); glyph instructions (hinting) are
//!   skipped, not interpreted; no kern/GPOS, so advances are unshaped and
//!   unkerned.
//! - `OutlinePoint` carries an `on_curve` bit rather than being a bare
//!   `(f32, f32)`: the rasterizer must tell quadratic control points from
//!   segment joints, and a points-only view loses that. Dropping the bit
//!   (`(p.x, p.y)`) recovers the plain-coordinate view.
//! - Composite components using point-matching attachment (args are point
//!   indices, i.e. `ARGS_ARE_XY_VALUES` unset) are attached at offset
//!   (0, 0) instead of doing point matching; `ROUND_XY_TO_GRID`,
//!   `USE_MY_METRICS` and composite instructions are ignored.
//! - A composite nesting deeper than 8 or a glyph decomposing to more
//!   than `POINT_BUDGET` (2^16) points is treated as malformed and yields
//!   `None` (guards recursive/exponential blow-up from hostile fonts).
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

/// Retained `loca` + `glyf` bytes for lazy outline decoding. Built only
/// when BOTH tables exist and `head.indexToLocFormat` is 0 (short) or
/// 1 (long); anything else leaves the font metrics-only.
#[derive(Debug, Clone)]
struct GlyphTables {
    loca: Vec<u8>,
    glyf: Vec<u8>,
    long_format: bool,
}

/// A parsed font: owns copies of the `hmtx` bytes, the selected `cmap`
/// subtable and (when present) the `loca`/`glyf` outline tables, so
/// lookups never borrow the caller's input buffer.
#[derive(Debug, Clone)]
pub struct ParsedFont {
    metrics: FontMetrics,
    num_glyphs: u16,
    num_h_metrics: u16,
    hmtx: Vec<u8>,
    cmap: CmapTable,
    outlines: Option<GlyphTables>,
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

        // --- loca + glyf (optional): retained raw, decoded per lookup ---
        // A missing pair, a head too short for indexToLocFormat or an
        // indexToLocFormat outside {0, 1} degrades to metrics-only instead
        // of failing the whole font.
        let outlines = (|| {
            if head.len() < 52 {
                return None; // no room for indexToLocFormat @50
            }
            let long_format = match be_i16(head, 50)? {
                0 => false,
                1 => true,
                _ => return None, // spec allows only short/long loca
            };
            let (lo_off, lo_len) = find(*b"loca")?;
            let loca = data.get(lo_off..lo_off + lo_len)?.to_vec();
            let (gl_off, gl_len) = find(*b"glyf")?;
            let glyf = data.get(gl_off..gl_off + gl_len)?.to_vec();
            Some(GlyphTables {
                loca,
                glyf,
                long_format,
            })
        })();

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
            outlines,
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
    ///
    /// Ids the cmap produces beyond `numGlyphs` (filler segments in
    /// hand-built or damaged subtables) are treated as unmapped, matching
    /// [`Self::advance`].
    pub fn glyph_index(&self, c: char) -> u16 {
        let code = c as u32;
        let gid = match &self.cmap {
            CmapTable::Format4(d) => {
                if code > 0xFFFF {
                    0 // format 4 is BMP-only by construction
                } else {
                    glyph_index_fmt4(d, code).unwrap_or(0)
                }
            }
            CmapTable::Format12(d) => glyph_index_fmt12(d, code).unwrap_or(0),
        };
        if gid >= self.num_glyphs { 0 } else { gid }
    }

    /// Decomposed TrueType outline of one glyph, in font units (y up).
    ///
    /// `None` when the font has no usable `loca`/`glyf`, the id is out of
    /// range, the glyph is blank (`loca[i] == loca[i+1]`, e.g. space), or
    /// its record is malformed (including composite recursion past the
    /// depth cap or past [`POINT_BUDGET`]). Never panics on hostile bytes.
    pub fn glyph_outline(&self, glyph_id: u16) -> Option<GlyphOutline> {
        let tables = self.outlines.as_ref()?;
        if glyph_id >= self.num_glyphs {
            return None;
        }
        let rec = glyph_record(tables, glyph_id)?;
        // Header: numberOfContours + 4 x bbox i16 = 10 bytes minimum.
        if rec.len() < 10 {
            return None;
        }
        let n_contours = be_i16(rec, 0)?;
        let mut budget = POINT_BUDGET;
        if n_contours >= 0 {
            if n_contours == 0 {
                return None; // declared empty: no outline
            }
            decode_simple(rec, n_contours as usize, &mut budget)
        } else {
            decode_composite(tables, rec, 0, &mut budget)
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
// TrueType glyph outlines (`loca` + `glyf`)
// ---------------------------------------------------------------------------

/// Total decoded-point cap across one glyph's whole composite tree. A
/// hostile font nesting composites could otherwise explode memory
/// exponentially; exceeding the budget yields `None`.
const POINT_BUDGET: usize = 1 << 16;

/// Composite component recursion depth cap (spec fonts nest 1-2 deep).
const COMPOSITE_DEPTH_CAP: u8 = 8;

// simple-glyph flag bits (glyf spec)
const F_ON_CURVE: u8 = 0x01;
const F_X_SHORT: u8 = 0x02;
const F_Y_SHORT: u8 = 0x04;
const F_REPEAT: u8 = 0x08;
const F_X_SAME_OR_POS: u8 = 0x10;
const F_Y_SAME_OR_POS: u8 = 0x20;

// composite component flag bits
const C_ARG_1_AND_2_ARE_WORDS: u16 = 0x0001;
const C_ARGS_ARE_XY_VALUES: u16 = 0x0002;
const C_WE_HAVE_A_SCALE: u16 = 0x0008;
const C_MORE_COMPONENTS: u16 = 0x0020;
const C_WE_HAVE_AN_X_AND_Y_SCALE: u16 = 0x0040;
const C_WE_HAVE_A_TWO_BY_TWO: u16 = 0x0080;

/// One vertex of a decomposed outline, in font units, y up.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OutlinePoint {
    pub x: f32,
    pub y: f32,
    /// `true`: on-curve anchor. `false`: quadratic B-spline control
    /// point; per the TrueType convention the curve continues through the
    /// implied midpoint of two consecutive control points.
    pub on_curve: bool,
}

/// One closed contour: a cyclic chain of [`OutlinePoint`]s. Walking the
/// chain cyclically, an off-curve point followed by an on-curve point is
/// a quadratic segment (anchor, control, anchor); two consecutive
/// off-curve points carry an implied midpoint between them; two
/// consecutive on-curve points form a straight line.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Contour {
    pub points: Vec<OutlinePoint>,
}

/// Decomposed outline of one glyph: its contours, in font units.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GlyphOutline {
    pub contours: Vec<Contour>,
}

/// Slices the raw glyf record for `gid` out of the retained tables.
/// Bounds-checks the loca pair against the glyf length; a pair with
/// `end < start` is malformed and yields `None`.
fn glyph_record<'a>(t: &'a GlyphTables, gid: u16) -> Option<&'a [u8]> {
    let i = gid as usize;
    let (start, end) = if t.long_format {
        (
            be_u32(&t.loca, i.checked_mul(4)?)? as usize,
            be_u32(&t.loca, i.checked_mul(4)?.checked_add(4)?)? as usize,
        )
    } else {
        // Short loca stores offset/2; odd stored values are undefined by
        // the spec — we shift and let the range checks below catch junk.
        let s = be_u16(&t.loca, i.checked_mul(2)?)? as usize;
        let e = be_u16(&t.loca, i.checked_mul(2)?.checked_add(2)?)? as usize;
        (s * 2, e * 2)
    };
    if end < start || end > t.glyf.len() {
        return None;
    }
    t.glyf.get(start..end)
}

/// Decodes a simple (non-composite) glyph record.
///
/// Layout: i16 nContours, 4 x i16 bbox, u16 endPts[n], u16 instructionLen,
/// u8 instructions[], flag bytes (with REPEAT runs), x deltas, y deltas.
/// All reads are bounds-checked; a truncated stream returns `None`.
fn decode_simple(rec: &[u8], n_contours: usize, budget: &mut usize) -> Option<GlyphOutline> {
    let ends_end = 10usize.checked_add(n_contours.checked_mul(2)?)?;
    if rec.len() < ends_end.checked_add(2)? {
        return None;
    }
    // Point count = last end point + 1 (end points are inclusive indices).
    let n_points = be_u16(rec, ends_end - 2)? as usize + 1;
    let instr_len = be_u16(rec, ends_end)? as usize;
    let flags_off = ends_end.checked_add(2)?.checked_add(instr_len)?;
    if n_points == 0 {
        return None;
    }
    if n_points > *budget {
        return None;
    }

    // Expand the flag stream (REPEAT runs add `count` extra copies).
    let mut flags: Vec<u8> = Vec::with_capacity(n_points);
    let mut pos = flags_off;
    while flags.len() < n_points {
        let f = *rec.get(pos)?;
        pos = pos.checked_add(1)?;
        flags.push(f);
        if f & F_REPEAT != 0 {
            let rep = *rec.get(pos)? as usize;
            pos = pos.checked_add(1)?;
            for _ in 0..rep {
                if flags.len() == n_points {
                    break; // overlong run: clamp (hostile input)
                }
                flags.push(f);
            }
        }
    }

    // X deltas first, then Y deltas, each cumulative over the glyph.
    let mut xs = vec![0i32; n_points];
    let mut ys = vec![0i32; n_points];
    decode_deltas(rec, &mut pos, &flags, true, &mut xs)?;
    decode_deltas(rec, &mut pos, &flags, false, &mut ys)?;
    *budget -= n_points;

    // Split points into contours at the end points, inserting implied
    // quadratic midpoints per contour.
    let mut ends = Vec::with_capacity(n_contours);
    for k in 0..n_contours {
        ends.push(be_u16(rec, 10usize.checked_add(k.checked_mul(2)?)?)?);
    }
    let mut out = GlyphOutline::default();
    let mut start = 0usize;
    for end in ends {
        let end = end as usize;
        if end < start || end >= n_points {
            return None; // end points must be ascending indices
        }
        let raw: Vec<OutlinePoint> = (start..=end)
            .map(|i| OutlinePoint {
                x: xs[i] as f32,
                y: ys[i] as f32,
                on_curve: flags[i] & F_ON_CURVE != 0,
            })
            .collect();
        if let Some(closed) = close_quadratic(&raw) {
            out.contours.push(Contour { points: closed });
        }
        start = end + 1;
    }
    if out.contours.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// Decodes one axis of packed delta coordinates, writing ABSOLUTE
/// coordinates into `out` (deltas accumulate left-to-right).
/// `is_x` selects which flag bits apply. Advances `pos` past the axis.
fn decode_deltas(
    rec: &[u8],
    pos: &mut usize,
    flags: &[u8],
    is_x: bool,
    out: &mut [i32],
) -> Option<()> {
    let (short, same_or_pos) = if is_x {
        (F_X_SHORT, F_X_SAME_OR_POS)
    } else {
        (F_Y_SHORT, F_Y_SAME_OR_POS)
    };
    let mut v: i32 = 0;
    for (i, &f) in flags.iter().enumerate() {
        if f & short != 0 {
            let d = *rec.get(*pos)? as i32;
            *pos = pos.checked_add(1)?;
            v += if f & same_or_pos != 0 { d } else { -d };
        } else if f & same_or_pos == 0 {
            v += be_i16(rec, *pos)? as i32; // 16-bit signed delta
            *pos = pos.checked_add(2)?;
        }
        // else: same-or-positive with no short bit means delta 0
        out[i] = v;
    }
    Some(())
}

/// Normalizes one raw contour into the closed flagged chain documented on
/// [`Contour`]: rotates to start at an on-curve anchor, inserts implied
/// on-curve midpoints between consecutive off-curve points, and
/// synthesizes a midpoint anchor for all-off-curve (fully curved) loops.
/// Returns `None` for degenerate (< 2 point) contours.
fn close_quadratic(pts: &[OutlinePoint]) -> Option<Vec<OutlinePoint>> {
    let n = pts.len();
    if n < 2 {
        return None;
    }
    let mut out: Vec<OutlinePoint> = Vec::with_capacity(n + n / 2 + 2);
    match pts.iter().position(|p| p.on_curve) {
        Some(s) => {
            // Rotate so an on-curve anchor leads; the chain then closes
            // cyclically back onto out[0].
            for k in 0..n {
                let cur = pts[(s + k) % n];
                out.push(cur);
                let next = pts[(s + k + 1) % n];
                if !cur.on_curve && !next.on_curve {
                    out.push(OutlinePoint {
                        x: (cur.x + next.x) / 2.0,
                        y: (cur.y + next.y) / 2.0,
                        on_curve: true, // implied midpoint
                    });
                }
            }
        }
        None => {
            // No on-curve point: anchors are the implied midpoints.
            // Emit mid(c[n-1], c[0]), then each control followed by the
            // midpoint to its successor; the chain closes cyclically.
            for k in 0..n {
                let prev = pts[(k + n - 1) % n];
                let cur = pts[k];
                out.push(OutlinePoint {
                    x: (prev.x + cur.x) / 2.0,
                    y: (prev.y + cur.y) / 2.0,
                    on_curve: true,
                });
                out.push(cur);
            }
        }
    }
    Some(out)
}

/// Decodes a composite glyph: a list of component records, each placing a
/// referenced glyph with an optional affine transform (F2Dot14 scale /
/// x-y scale / full 2x2) plus an xy offset. Recursion is capped at
/// [`COMPOSITE_DEPTH_CAP`] and the shared point budget bounds total size.
/// Point-matching attachment (ARGS_ARE_XY_VALUES unset) is approximated
/// as offset (0, 0) — see module docs.
fn decode_composite(
    tables: &GlyphTables,
    rec: &[u8],
    depth: u8,
    budget: &mut usize,
) -> Option<GlyphOutline> {
    if depth > COMPOSITE_DEPTH_CAP {
        return None;
    }
    let mut out = GlyphOutline::default();
    let mut pos = 10usize; // past the i16 numberOfContours + bbox header
    loop {
        if rec.len() < pos.checked_add(4)? {
            return None;
        }
        let flags = be_u16(rec, pos)?;
        let comp_gid = be_u16(rec, pos + 2)?;
        pos = pos.checked_add(4)?;
        let (arg1, arg2) = if flags & C_ARG_1_AND_2_ARE_WORDS != 0 {
            let a = be_i16(rec, pos)?;
            let b = be_i16(rec, pos + 2)?;
            pos = pos.checked_add(4)?;
            (a as f32, b as f32)
        } else {
            let a = *rec.get(pos)? as i8;
            let b = *rec.get(pos.checked_add(1)?)? as i8;
            pos = pos.checked_add(2)?;
            (a as f32, b as f32)
        };
        // 2x2 in file order: xscale, scale01, scale10, yscale; applied as
        // x' = a*x + c*y, y' = b*x + d*y (spec's composite transform).
        let (mut a, mut b, mut c, mut d) = (1.0f32, 0.0f32, 0.0f32, 1.0f32);
        if flags & C_WE_HAVE_A_SCALE != 0 {
            let s = f2dot14(rec, pos)?;
            pos = pos.checked_add(2)?;
            a = s;
            d = s;
        } else if flags & C_WE_HAVE_AN_X_AND_Y_SCALE != 0 {
            a = f2dot14(rec, pos)?;
            d = f2dot14(rec, pos.checked_add(2)?)?;
            pos = pos.checked_add(4)?;
        } else if flags & C_WE_HAVE_A_TWO_BY_TWO != 0 {
            a = f2dot14(rec, pos)?;
            c = f2dot14(rec, pos.checked_add(2)?)?;
            b = f2dot14(rec, pos.checked_add(4)?)?;
            d = f2dot14(rec, pos.checked_add(6)?)?;
            pos = pos.checked_add(8)?;
        }
        let (dx, dy) = if flags & C_ARGS_ARE_XY_VALUES != 0 {
            (arg1, arg2)
        } else {
            (0.0, 0.0) // point matching unsupported (documented deviation)
        };
        let sub = decode_glyph_rec(tables, comp_gid, depth + 1, budget)?;
        for contour in sub.contours {
            let mapped: Vec<OutlinePoint> = contour
                .points
                .iter()
                .map(|p| OutlinePoint {
                    x: a * p.x + c * p.y + dx,
                    y: b * p.x + d * p.y + dy,
                    on_curve: p.on_curve,
                })
                .collect();
            out.contours.push(Contour { points: mapped });
        }
        if flags & C_MORE_COMPONENTS == 0 {
            break;
        }
    }
    if out.contours.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// Depth-carrying recursive step for composite components (also used for
/// the top-level glyph, which shares the same header layout).
fn decode_glyph_rec(
    tables: &GlyphTables,
    gid: u16,
    depth: u8,
    budget: &mut usize,
) -> Option<GlyphOutline> {
    if depth > COMPOSITE_DEPTH_CAP {
        return None;
    }
    let rec = glyph_record(tables, gid)?;
    if rec.len() < 10 {
        return None;
    }
    let n_contours = be_i16(rec, 0)?;
    if n_contours > 0 {
        decode_simple(rec, n_contours as usize, budget)
    } else if n_contours < 0 {
        decode_composite(tables, rec, depth, budget)
    } else {
        None // blank component
    }
}

/// F2Dot14: 16-bit signed fixed point with 14 fraction bits.
fn f2dot14(d: &[u8], off: usize) -> Option<f32> {
    Some(be_i16(d, off)? as f32 / 16384.0)
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

    /// Character -> glyph id (0 = unmapped/.notdef). Always 0 under the
    /// fallback font, which has no glyph space.
    pub fn glyph_index(&self, c: char) -> u16 {
        match self {
            FontSource::Ttf(f) => f.glyph_index(c),
            FontSource::Fallback(_) => 0,
        }
    }

    /// Decomposed outline of one glyph in font units (see
    /// [`ParsedFont::glyph_outline`]). Always `None` under the fallback
    /// font.
    pub fn glyph_outline(&self, glyph_id: u16) -> Option<GlyphOutline> {
        match self {
            FontSource::Ttf(f) => f.glyph_outline(glyph_id),
            FontSource::Fallback(_) => None,
        }
    }
}

/// Shared unit-value fallback instance (FallbackFont carries no state).
static FALLBACK: FallbackFont = FallbackFont;

// ---------------------------------------------------------------------------
// panic-free big-endian primitives
// ---------------------------------------------------------------------------

fn sanitize_px(px: f32) -> f32 {
    if px.is_finite() && px > 0.0 { px } else { 0.0 }
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

    // --- glyph outline builders -------------------------------------------

    /// Emits one packed coordinate delta, choosing between zero / short /
    /// 16-bit encodings and setting the matching flag bits.
    fn push_delta(stream: &mut Vec<u8>, flag: &mut u8, d: i32, is_x: bool) {
        let (short_bit, pos_bit) = if is_x {
            (0x02u8, 0x10u8)
        } else {
            (0x04u8, 0x20u8)
        };
        if d == 0 {
            *flag |= pos_bit; // not short + same-bit => delta 0
        } else if d.abs() <= 255 {
            *flag |= short_bit;
            if d > 0 {
                *flag |= pos_bit; // positive short
            }
            stream.push(d.abs() as u8);
        } else {
            stream.extend_from_slice(&(d as i16).to_be_bytes()); // 16-bit signed
        }
    }

    /// Simple-glyph record from one or more contours of `(x, y, on_curve)`
    /// points; deltas are auto-encoded (mixed short/16-bit/zero paths).
    fn simple_glyph_record(contours: &[&[(i32, i32, bool)]]) -> Vec<u8> {
        let flat: Vec<(i32, i32, bool)> = contours.iter().flat_map(|c| c.iter().copied()).collect();
        let mut rec = vec![0u8; 10];
        rec[0..2].copy_from_slice(&be16(contours.len() as u16));
        let xmin = flat.iter().map(|p| p.0).min().unwrap_or(0) as i16;
        let xmax = flat.iter().map(|p| p.0).max().unwrap_or(0) as i16;
        let ymin = flat.iter().map(|p| p.1).min().unwrap_or(0) as i16;
        let ymax = flat.iter().map(|p| p.1).max().unwrap_or(0) as i16;
        rec[2..4].copy_from_slice(&(xmin as u16).to_be_bytes());
        rec[4..6].copy_from_slice(&(ymin as u16).to_be_bytes());
        rec[6..8].copy_from_slice(&(xmax as u16).to_be_bytes());
        rec[8..10].copy_from_slice(&(ymax as u16).to_be_bytes());
        let mut acc = 0u16;
        for c in contours {
            acc += c.len() as u16;
            rec.extend_from_slice(&be16(acc.wrapping_sub(1))); // inclusive end pt
        }
        rec.extend_from_slice(&be16(0)); // instructionLength
        let mut flags = Vec::new();
        let (mut xs, mut ys) = (Vec::new(), Vec::new());
        let (mut px, mut py) = (0i32, 0i32);
        for &(x, y, on) in &flat {
            let mut f = if on { 0x01u8 } else { 0u8 };
            push_delta(&mut xs, &mut f, x - px, true);
            push_delta(&mut ys, &mut f, y - py, false);
            flags.push(f);
            px = x;
            py = y;
        }
        rec.extend_from_slice(&flags);
        rec.extend_from_slice(&xs);
        rec.extend_from_slice(&ys);
        rec
    }

    fn f2d(v: f32) -> Vec<u8> {
        ((v * 16384.0) as i16).to_be_bytes().to_vec() // F2Dot14
    }

    /// sfnt font whose glyphs are `records`, mapped by the shared delta
    /// cmap (A->1, B->2, C->3). loca is short or long per `long_loca`.
    /// Record 0 is expected to be blank (.notdef).
    fn outline_font(records: &[Vec<u8>], long_loca: bool) -> Vec<u8> {
        let mut padded: Vec<Vec<u8>> = records.to_vec();
        for g in padded.iter_mut() {
            if g.len() % 2 == 1 {
                g.push(0); // short loca needs even offsets
            }
        }
        let mut offs = vec![0usize];
        for g in &padded {
            offs.push(offs.last().unwrap() + g.len());
        }
        let loca: Vec<u8> = if long_loca {
            offs.iter().flat_map(|&o| be32(o as u32)).collect()
        } else {
            offs.iter().flat_map(|&o| be16((o / 2) as u16)).collect()
        };
        let glyf: Vec<u8> = padded.concat();
        let n = records.len() as u16;
        let advances: Vec<(u16, i16)> = (0..n).map(|i| (500 + 100 * i, 0)).collect();
        // indexToLocFormat lives at head offset 50: 0 = short, 1 = long.
        let mut hd = head(1000);
        hd[50..52].copy_from_slice(&be16(u16::from(long_loca)));
        assemble(&[
            (b"head", hd),
            (b"hhea", hhea(800, -200, 300, n)),
            (b"maxp", maxp_v1(n)),
            (b"hmtx", hmtx(&advances)),
            (b"cmap", cmap_wrapper(&cmap_fmt4_delta())),
            (b"loca", loca),
            (b"glyf", glyf),
        ])
    }

    const SQUARE: &[(i32, i32, bool)] = &[
        (100, 100, true),
        (600, 100, true),
        (600, 800, true),
        (100, 800, true),
    ];

    fn square_glyph() -> Vec<u8> {
        simple_glyph_record(&[SQUARE])
    }

    /// Standard fixture: blank .notdef, square@1 (A), triangle@2 (B).
    fn shape_font(long_loca: bool) -> Vec<u8> {
        outline_font(
            &[
                Vec::new(),
                square_glyph(),
                simple_glyph_record(&[&[(0, 0, true), (500, 700, true), (1000, 0, true)]]),
            ],
            long_loca,
        )
    }

    #[test]
    fn square_outline_decoded() {
        let f = ParsedFont::parse(&shape_font(false)).unwrap();
        let o = f.glyph_outline(1).expect("square");
        assert_eq!(o.contours.len(), 1);
        let pts = &o.contours[0].points;
        assert_eq!(pts.len(), 4);
        assert!(pts.iter().all(|p| p.on_curve));
        for (got, want) in pts.iter().zip(SQUARE) {
            assert_eq!((got.x, got.y), (want.0 as f32, want.1 as f32));
        }
    }

    #[test]
    fn triangle_outline_decoded() {
        let f = ParsedFont::parse(&shape_font(true)).unwrap();
        let o = f.glyph_outline(2).expect("triangle");
        assert_eq!(o.contours.len(), 1);
        assert_eq!(o.contours[0].points.len(), 3);
        assert_eq!(
            o.contours[0].points[1],
            OutlinePoint {
                x: 500.0,
                y: 700.0,
                on_curve: true
            }
        );
    }

    #[test]
    fn composite_of_two_squares() {
        // Two components referencing the square glyph (gid 1):
        //  - byte args (offset 50, 0), uniform scale 0.5
        //  - word args (offset 400, 200), identity transform
        let mut rec = vec![0u8; 10]; // header: nContours = -1 + bbox zeros
        rec[0..2].copy_from_slice(&(-1i16).to_be_bytes());
        let mut comp = Vec::new();
        comp.extend_from_slice(&be16(0x0008 | 0x0020 | 0x0002)); // SCALE | MORE | XY
        comp.extend_from_slice(&be16(1));
        comp.extend_from_slice(&[50, 0]); // int8 args (xy values)
        comp.extend_from_slice(&f2d(0.5));
        comp.extend_from_slice(&be16(0x0001 | 0x0002)); // WORD_ARGS | XY_VALUES
        comp.extend_from_slice(&be16(1));
        comp.extend_from_slice(&be16(400));
        comp.extend_from_slice(&be16(200));
        rec.extend_from_slice(&comp);

        let font = outline_font(&[Vec::new(), square_glyph(), rec], false);
        let f = ParsedFont::parse(&font).unwrap();
        let o = f.glyph_outline(2).expect("composite");
        assert_eq!(o.contours.len(), 2);
        // Component 1: (100,100)*0.5 + (50,0) = (100,50); (600,800)*0.5 + (50,0)
        // = (350,400).
        let p = &o.contours[0].points;
        assert_eq!((p[0].x, p[0].y), (100.0, 50.0));
        assert_eq!((p[2].x, p[2].y), (350.0, 400.0));
        assert!(p.iter().all(|q| q.on_curve));
        // Component 2: pure translation.
        let q = &o.contours[1].points;
        assert_eq!((q[0].x, q[0].y), (500.0, 300.0));
        assert_eq!((q[3].x, q[3].y), (500.0, 1000.0));
    }

    #[test]
    fn composite_x_y_scale_and_2x2() {
        // One component with WE_HAVE_AN_X_AND_Y_SCALE (x1.5, y0.5 — F2Dot14
        // cannot encode 2.0), then one with WE_HAVE_A_TWO_BY_TWO
        // [[1,0],[0,1]] reading scale01/scale10.
        let mut rec = vec![0u8; 10];
        rec[0..2].copy_from_slice(&(-1i16).to_be_bytes());
        let mut comp = Vec::new();
        comp.extend_from_slice(&be16(0x0040 | 0x0020 | 0x0002)); // X_Y_SCALE | MORE | XY
        comp.extend_from_slice(&be16(1));
        comp.extend_from_slice(&[10, 10]);
        comp.extend_from_slice(&f2d(1.5));
        comp.extend_from_slice(&f2d(0.5));
        comp.extend_from_slice(&be16(0x0080 | 0x0001 | 0x0002)); // TWO_BY_TWO | WORD_ARGS | XY_VALUES
        comp.extend_from_slice(&be16(1));
        comp.extend_from_slice(&be16(0));
        comp.extend_from_slice(&be16(0));
        comp.extend_from_slice(&f2d(1.0)); // xscale
        comp.extend_from_slice(&f2d(0.0)); // scale01
        comp.extend_from_slice(&f2d(0.0)); // scale10
        comp.extend_from_slice(&f2d(1.0)); // yscale
        rec.extend_from_slice(&comp);

        let font = outline_font(&[Vec::new(), square_glyph(), rec], true);
        let f = ParsedFont::parse(&font).unwrap();
        let o = f.glyph_outline(2).expect("composite");
        let p = &o.contours[0].points; // x*1.5+10, y*0.5+10
        assert_eq!((p[0].x, p[0].y), (160.0, 60.0));
        assert_eq!((p[1].x, p[1].y), (910.0, 60.0));
        let q = &o.contours[1].points; // identity 2x2 + zero offset
        assert_eq!((q[2].x, q[2].y), (600.0, 800.0));
    }

    #[test]
    fn implied_midpoints_between_off_curve_points() {
        // On(0,0) -> Off(40,80) -> Off(80,0): an implied on-curve midpoint
        // (60,40) sits between the two controls.
        let g = simple_glyph_record(&[&[(0, 0, true), (40, 80, false), (80, 0, false)]]);
        let font = outline_font(&[Vec::new(), g], false);
        let f = ParsedFont::parse(&font).unwrap();
        let o = f.glyph_outline(1).expect("curve");
        let pts = &o.contours[0].points;
        assert_eq!(
            pts,
            &[
                OutlinePoint {
                    x: 0.0,
                    y: 0.0,
                    on_curve: true
                },
                OutlinePoint {
                    x: 40.0,
                    y: 80.0,
                    on_curve: false
                },
                OutlinePoint {
                    x: 60.0,
                    y: 40.0,
                    on_curve: true
                },
                OutlinePoint {
                    x: 80.0,
                    y: 0.0,
                    on_curve: false
                },
            ]
        );
    }

    #[test]
    fn all_off_curve_contour_gets_synthetic_anchor() {
        // Fully-curved loop: anchors are the pairwise midpoints.
        let g = simple_glyph_record(&[&[(0, 10, false), (10, 20, false), (20, 10, false)]]);
        let font = outline_font(&[Vec::new(), g], false);
        let f = ParsedFont::parse(&font).unwrap();
        let pts = &f.glyph_outline(1).unwrap().contours[0].points;
        assert_eq!(pts.len(), 6); // 3 anchors + 3 controls, alternating
        assert!(pts[0].on_curve);
        assert_eq!((pts[0].x, pts[0].y), (10.0, 10.0)); // mid(c2, c0)
        assert!(!pts[1].on_curve);
        assert_eq!((pts[1].x, pts[1].y), (0.0, 10.0));
        assert_eq!((pts[2].x, pts[2].y), (5.0, 15.0)); // mid(c0, c1)
        assert!(pts.iter().skip(1).step_by(2).all(|p| !p.on_curve));
        assert!(pts.iter().step_by(2).all(|p| p.on_curve));
    }

    #[test]
    fn short_and_long_loca_agree() {
        let short = ParsedFont::parse(&shape_font(false)).unwrap();
        let long = ParsedFont::parse(&shape_font(true)).unwrap();
        assert_eq!(short.glyph_outline(1), long.glyph_outline(1));
        assert_eq!(short.glyph_outline(2), long.glyph_outline(2));
    }

    #[test]
    fn blank_and_missing_outlines_are_none() {
        let f = ParsedFont::parse(&shape_font(false)).unwrap();
        assert!(f.glyph_outline(0).is_none()); // blank .notdef record
        assert!(f.glyph_outline(99).is_none()); // beyond numGlyphs
        // The metrics-only fixture has no loca/glyf at all.
        assert!(parsed().glyph_outline(1).is_none());
        assert!(parsed().glyph_outline(0).is_none());
    }

    #[test]
    fn truncated_glyf_rejected_while_font_parses() {
        let mut fx = shape_font(false).to_vec();
        fx.truncate(fx.len() - 4); // bites into glyf only (last table)
        let f = ParsedFont::parse(&fx).expect("font still parses");
        // glyf is the last table: the tail cut lands in the triangle
        // record (gid 2), while the square (gid 1) survives untouched.
        assert!(f.glyph_outline(2).is_none()); // record cut mid-stream
        assert!(f.glyph_outline(1).is_some());
    }

    #[test]
    fn self_referential_composite_hits_depth_cap() {
        // A composite that includes itself must terminate (depth cap) and
        // yield None instead of recursing forever.
        let mut rec = vec![0u8; 10];
        rec[0..2].copy_from_slice(&(-1i16).to_be_bytes());
        let mut comp = Vec::new();
        comp.extend_from_slice(&be16(0x0002)); // XY_VALUES, no MORE
        comp.extend_from_slice(&be16(1)); // points at ITSELF (gid 1)
        comp.extend_from_slice(&be16(0));
        comp.extend_from_slice(&be16(0));
        rec.extend_from_slice(&comp);
        let font = outline_font(&[Vec::new(), rec], false);
        let f = ParsedFont::parse(&font).unwrap();
        assert!(f.glyph_outline(1).is_none());
    }

    #[test]
    fn repeat_flag_run_decodes() {
        // Six points sharing one flag byte via REPEAT: on-curve, +10 x
        // steps, y unchanged (flags 0x33 = ON | X_SHORT | X_POS | Y_SAME).
        let mut rec = vec![0u8; 10];
        rec[0..2].copy_from_slice(&be16(1));
        rec[8..10].copy_from_slice(&be16(60)); // xmax
        rec.extend_from_slice(&be16(5)); // endPtsOfContours[0] = 5
        rec.extend_from_slice(&be16(0)); // instructions
        rec.extend_from_slice(&[0x33 | 0x08, 5]); // flag 0x33, REPEAT count 5
        for _ in 0..6 {
            rec.push(10); // x short positives
        }
        // no y bytes
        let font = outline_font(&[Vec::new(), rec], true);
        let f = ParsedFont::parse(&font).unwrap();
        let pts = &f.glyph_outline(1).unwrap().contours[0].points;
        assert_eq!(pts.len(), 6);
        for (i, p) in pts.iter().enumerate() {
            assert_eq!((p.x, p.y), (((i + 1) * 10) as f32, 0.0));
            assert!(p.on_curve);
        }
    }

    #[test]
    fn char_to_outline_end_to_end() {
        let src = FontSource::Ttf(ParsedFont::parse(&shape_font(false)).unwrap());
        assert_eq!(src.glyph_index('A'), 1);
        assert_eq!(src.glyph_index('B'), 2);
        assert_eq!(src.glyph_index('Q'), 0);
        let o = src.glyph_outline(src.glyph_index('A')).expect("A outline");
        assert_eq!(o.contours.len(), 1);
        assert_eq!(o.contours[0].points.len(), 4);
        // Unmapped char resolves to glyph 0 (blank here) -> no outline.
        assert!(src.glyph_outline(src.glyph_index('Q')).is_none());
        // Fallback source never has outlines.
        assert_eq!(FontSource::fallback().glyph_index('A'), 0);
        assert!(FontSource::fallback().glyph_outline(1).is_none());
    }
}
