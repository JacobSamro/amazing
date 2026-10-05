// Portions ported from boa-dev/boa (core/parser/src/source/utf16.rs and
// core/parser/src/lexer/cursor.rs) @ 39cd11214ecb366200157dab96b3919e61e9a391, MIT.
// UTF-16 SourceCharacter decoding adapted to shared, clone-preserved storage.
use std::rc::Rc;

/// Original source units and their real Unicode code points. Lone surrogates
/// remain surrogate code points; no scalar replacement or sentinel is used.
#[derive(Clone)]
pub(crate) struct LexerSource {
    units: crate::source_text::SourceText,
    points: Rc<[u32]>,
    unit_starts: Rc<[usize]>,
}

impl LexerSource {
    pub(crate) fn utf16(input: &[u16]) -> Self {
        let mut points = Vec::new();
        let mut starts = Vec::new();
        let mut index = 0;
        while let Some(&first) = input.get(index) {
            starts.push(index);
            index += 1;
            let point = if (0xd800..=0xdbff).contains(&first) {
                match input.get(index).copied() {
                    Some(second @ 0xdc00..=0xdfff) => {
                        index += 1;
                        (((u32::from(first & 0x3ff)) << 10) | u32::from(second & 0x3ff)) + 0x10000
                    }
                    _ => u32::from(first),
                }
            } else { u32::from(first) };
            points.push(point);
        }
        starts.push(input.len());
        Self { units: crate::source_text::SourceText::from_utf16(input), points: points.into(), unit_starts: starts.into() }
    }

    pub(crate) fn text(&self) -> crate::source_text::SourceText { self.units.clone() }
    pub(crate) fn unit_offset(&self, index: usize) -> usize { self.unit_starts[index] }

    pub(crate) fn len(&self) -> usize { self.points.len() }
    pub(crate) fn point(&self, index: usize) -> Option<u32> { self.points.get(index).copied() }
    pub(crate) fn scalar(&self, index: usize) -> Option<char> { self.point(index).and_then(char::from_u32) }
    pub(crate) fn units(&self, start: usize, end: usize) -> &[u16] {
        &self.units.units()[self.unit_starts[start]..self.unit_starts[end]]
    }
    pub(crate) fn contains(&self, start: usize, end: usize, point: u32) -> bool {
        self.points[start..end].contains(&point)
    }
}
