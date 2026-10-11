//! Authentic installed-face metadata; no filename-derived style or metrics.
//! Original implementation of OpenType name/OS2/head/post metadata contracts.
use crate::{FontFormat, FontDescriptorRange, FontStyleDescriptor, TextProvider, WebFont, decode_web_font_to_sfnt};
use amazing_text::{FontKey, FontSlant, TextError, TextStyle};
use harfrust::FontRef;
use read_fonts::{Offset, TableProvider};
use std::sync::Arc;
use std::panic::{AssertUnwindSafe, catch_unwind};

/// Names and selection properties from the actual first face in a font file.
/// Collections follow the existing provider's first-face policy.
#[derive(Clone, Debug, PartialEq)]
pub struct InstalledFontProfile {
    pub families: Vec<String>,
    pub full_names: Vec<String>,
    pub postscript_names: Vec<String>,
    pub weight: u16,
    pub stretch: f32,
    pub slant: FontSlant,
    pub italic_angle: Option<f32>,
    pub fixed_pitch: bool,
    /// Actual OS/2 family class: no inferred serif classification when absent.
    pub family_class: Option<i16>,
    pub serif: Option<bool>,
    pub color: bool,
    /// Exact native OS/2 range bits, never inferred from family/file names.
    pub unicode_ranges: [u32; 4],
    pub code_page_ranges: Option<[u32; 2]>,
    pub registered_axes: Vec<[u8; 4]>,
    pub weight_range: FontDescriptorRange,
    pub stretch_range: FontDescriptorRange,
    pub italic_range: Option<FontDescriptorRange>,
    pub oblique_range: Option<FontDescriptorRange>,
}

/// An immutable validated face snapshot. Cloning this handle shares the exact
/// original bytes, decoded SFNT and native metadata; it never reopens a file.
/// Only this constructor can mint a handle. Embeddings own its lifetime.
#[derive(Clone, Debug)]
pub struct InstalledFontFace {
    pub(super) sfnt: Arc<[u8]>,
    pub(super) metadata: Arc<InstalledFontProfile>,
    original: Arc<[u8]>,
}
impl InstalledFontFace {
    pub fn new(format: FontFormat, bytes: Vec<u8>) -> Result<Self, TextError> {
        let decoded = decode_web_font_to_sfnt(format, &bytes)?;
        let metadata = catch_unwind(AssertUnwindSafe(|| profile(&decoded)))
            .map_err(|_| TextError::MalformedFont)??;
        let same = decoded == bytes;
        let sfnt: Arc<[u8]> = decoded.into();
        let original = if same { sfnt.clone() } else { bytes.into() };
        Ok(Self { sfnt, original, metadata: Arc::new(metadata) })
    }
    pub fn profile(&self) -> &InstalledFontProfile { &self.metadata }
    pub fn original_bytes(&self) -> &[u8] { &self.original }
}

/// Validate bytes before reading native face metadata. Names do not establish
/// CSS generic-family authority; the embedding selects its authentic aliases.
pub fn inspect_installed_font(format: FontFormat, bytes: &[u8]) -> Result<InstalledFontProfile, TextError> {
    let sfnt = decode_web_font_to_sfnt(format, bytes)?;
    catch_unwind(AssertUnwindSafe(|| profile(&sfnt))).map_err(|_| TextError::MalformedFont)?
}

fn profile(sfnt: &[u8]) -> Result<InstalledFontProfile, TextError> {
    let face = FontRef::from_index(sfnt, 0).map_err(|_| TextError::MalformedFont)?;
    let head = face.head().map_err(|_| TextError::MalformedFont)?;
    let os2 = face.os2().ok();
    let post = face.post().ok();
    let flags = os2.as_ref().map(|t| t.fs_selection().bits()).unwrap_or(0);
    let head_flags = head.mac_style().bits();
    let weight = os2.as_ref().map(|t| t.us_weight_class()).filter(|v| (1..=1000).contains(v))
        .unwrap_or(if head_flags & 1 != 0 { 700 } else { 400 });
    let stretch = os2.as_ref().and_then(|t| width_percentage(t.us_width_class())).unwrap_or(100.0);
    let slant = if flags & 1 != 0 { FontSlant::Italic }
        else if os2.as_ref().is_some_and(|t| t.version() >= 4) && flags & 0x200 != 0 { FontSlant::Oblique }
        else if head_flags & 2 != 0 { FontSlant::Italic } else { FontSlant::Normal };
    let mut families = Vec::new();
    let mut full_names = Vec::new();
    let mut postscript_names = Vec::new();
    if let Ok(name) = face.name() {
        if name.name_record().len() > 4096 { return Err(TextError::ResourceLimit); }
        // WWS and typographic names retain real families across bold/italic
        // style-linked legacy family records. All genuine family aliases survive.
        for id in [21,16,1,4,6] {
            let mut records=name.name_record().iter().filter(|row| row.name_id().to_u16() == id).collect::<Vec<_>>();
            records.sort_by_key(|row| (match (row.platform_id(),row.language_id()) { (3,0x409) | (1,0) => 0, (0,_) => 1, _ => 2 },u8::from(!row.is_unicode())));
            for row in records {
                if row.length() > 4096 { return Err(TextError::ResourceLimit); }
                let value: String = if row.is_unicode() {
                    let start=row.string_offset().non_null().unwrap_or(0);
                    let Some(raw)=name.string_data().as_bytes().get(start..start+row.length() as usize) else { continue; };
                    if raw.len()%2 != 0 { continue; }
                    let units=raw.chunks_exact(2).map(|pair|u16::from_be_bytes([pair[0],pair[1]]));
                    let Ok(value)=char::decode_utf16(units).collect::<Result<String,_>>() else { continue; };
                    value
                } else {
                    let Ok(value) = row.string(name.string_data()) else { continue; };
                    value.chars().collect()
                };
                let value = value.trim();
                if value.is_empty() || value.contains('\0') { continue; }
                let output = match id { 4 => &mut full_names, 6 => &mut postscript_names, _ => &mut families };
                if !output.iter().any(|old: &String| old.eq_ignore_ascii_case(value)) { output.push(value.to_owned()); }
            }
        }
    }
    let mut registered_axes=Vec::new();
    let mut weight_range = FontDescriptorRange::single(weight as f32);
    let mut stretch_range = FontDescriptorRange::single(stretch);
    let mut italic_range = None;
    let mut oblique_range = None;
    if let Ok(fvar) = face.fvar() {
        for axis in fvar.axes().map_err(|_| TextError::MalformedFont)? {
            let min = axis.min_value().to_f64() as f32;
            let max = axis.max_value().to_f64() as f32;
            if !min.is_finite() || !max.is_finite() || min > max { return Err(TextError::MalformedFont); }
            let tag=axis.axis_tag().to_be_bytes();
            registered_axes.push(tag);
            match tag {
                [b'w',b'g',b'h',b't'] => {
                    if max >= 1.0 && min <= 1000.0 { weight_range = FontDescriptorRange { min: min.max(1.0), max: max.min(1000.0) }; }
                },
                [b'w',b'd',b't',b'h'] => {
                    if max > 0.0 && min <= 1000.0 { stretch_range = FontDescriptorRange { min: min.max(f32::MIN_POSITIVE), max: max.min(1000.0) }; }
                },
                [b'i',b't',b'a',b'l'] => italic_range = Some(FontDescriptorRange { min, max }),
                [b's',b'l',b'n',b't'] => oblique_range = Some(FontDescriptorRange { min: -max, max: -min }),
                _ => {}
            }
        }
    }
    Ok(InstalledFontProfile {
        families, full_names, postscript_names, weight, stretch, slant,
        registered_axes, weight_range, stretch_range, italic_range, oblique_range,
        italic_angle: post.as_ref().map(|t| t.italic_angle().to_f64() as f32),
        fixed_pitch: post.as_ref().is_some_and(|t| t.is_fixed_pitch() != 0)
            || os2.as_ref().is_some_and(|t| t.panose_10().first() == Some(&2) && t.panose_10().get(3) == Some(&9)),
        family_class: os2.as_ref().map(|t| t.s_family_class()),
        serif: os2.as_ref().and_then(|t| serif_class(t.s_family_class(), t.panose_10())),
        color: crate::emoji_presentation::has_color_glyph_tables(sfnt),
        unicode_ranges: os2.as_ref().map_or([0; 4], |t| [t.ul_unicode_range_1(), t.ul_unicode_range_2(), t.ul_unicode_range_3(), t.ul_unicode_range_4()]),
        code_page_ranges: os2.as_ref().and_then(|t| Some([t.ul_code_page_range_1()?, t.ul_code_page_range_2()?])),
    })
}

impl InstalledFontProfile {
    pub(crate) fn style_descriptor(&self, style: &TextStyle) -> FontStyleDescriptor {
        if let Some(range) = self.italic_range {
            let requested = if style.slant == FontSlant::Italic || (style.slant == FontSlant::Oblique && self.oblique_range.is_none()) { 1.0 } else { 0.0 };
            if (style.slant != FontSlant::Oblique || self.oblique_range.is_none()) && range.min <= requested && requested <= range.max {
                return if requested == 1.0 { FontStyleDescriptor::Italic } else { FontStyleDescriptor::Normal };
            }
        }
        if let Some(range) = self.oblique_range { return FontStyleDescriptor::Oblique { min_degrees: range.min, max_degrees: range.max }; }
        match self.slant {
            FontSlant::Normal => FontStyleDescriptor::Normal,
            FontSlant::Italic => FontStyleDescriptor::Italic,
            FontSlant::Oblique => { let angle = self.italic_angle.map(|angle| -angle).unwrap_or(14.0);
                FontStyleDescriptor::Oblique { min_degrees: angle, max_degrees: angle } }
        }
    }
}

fn width_percentage(width: u16) -> Option<f32> {
    // OpenType usWidthClass maps to the nine CSS named widths. Integer
    // selection preserves the half-percent classes without rounding.
    match width { 1=>Some(50.0),2=>Some(62.5),3=>Some(75.0),4=>Some(87.5),5=>Some(100.0),
        6=>Some(112.5),7=>Some(125.0),8=>Some(150.0),9=>Some(200.0),_=>None }
}

fn serif_class(class: i16, panose: &[u8]) -> Option<bool> {
    match (class as u16 >> 8) & 255 { 1..=7 => Some(true),8 => Some(false),_ => {
        if panose.first() != Some(&2) { return None; }
        match panose.get(1).copied() { Some(2..=10) => Some(true),Some(11..=15) => Some(false),_=>None }
    }}
}

impl TextProvider {
    /// Register a real installed face with its native selection metadata.
    /// Author @font-face declarations keep their separate descriptor authority.
    pub fn register_installed_font(&mut self, family: &str, format: FontFormat, bytes: Vec<u8>) -> Result<FontKey, TextError> {
        self.register_installed_font_aliases(&[family], format, bytes)
    }

    /// One native record owns the actual face bytes; its genuine family names
    /// and supplied platform aliases select that same record and font key.
    pub fn register_installed_font_aliases(&mut self, families: &[&str], format: FontFormat, bytes: Vec<u8>) -> Result<FontKey, TextError> {
        let face = InstalledFontFace::new(format, bytes)?;
        self.register_installed_face_aliases(families, &face)
    }

    /// Register an already validated immutable face without decoding or parsing
    /// it again. Native metadata remains authoritative for installed selection.
    pub fn register_installed_face_aliases(&mut self, families: &[&str], face: &InstalledFontFace) -> Result<FontKey, TextError> {
        let metadata = face.profile();
        let family = match families.first() {
            Some(family) if !family.trim().is_empty() => family.trim(),
            Some(_) => return Err(TextError::InvalidStyle),
            None => metadata.families.first().map(String::as_str).ok_or(TextError::InvalidStyle)?,
        };
        let mut aliases=std::collections::BTreeSet::new();
        for alias in families.iter().skip(1).copied().chain(metadata.families.iter().map(String::as_str)) {
            let alias=alias.trim();
            if alias.is_empty() { return Err(TextError::InvalidStyle); }
            aliases.insert(alias.to_ascii_lowercase());
        }
        let font = WebFont { family: family.to_owned(), weight: metadata.weight,
            stretch: metadata.stretch.round() as u16, slant: metadata.slant, format: FontFormat::OpenType, bytes: Vec::new() };
        let key = self.store_font_bytes(font, face.sfnt.clone(), None)?;
        let font=self.fonts.iter_mut().find(|font| font.key==key).expect("newly allocated font");
        font.installed_aliases=aliases; font.installed_face=Some(face.metadata.clone());
        Ok(key)
    }
}
