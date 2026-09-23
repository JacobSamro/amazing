use crate::{generated::{reverse, tables}, Encoding};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EncodeError { pub character_offset: usize, pub code_point: u32 }

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum IsoMode { Ascii, Roman, Jis0208 }

/// Streaming scalar encoder. Only ISO-2022-JP retains state between pushes.
pub struct Encoder {
    encoding: Encoding,
    fatal: bool,
    offset: usize,
    iso_mode: IsoMode,
}

impl Encoder {
    pub fn new(encoding: Encoding, fatal: bool) -> Self {
        Self { encoding, fatal, offset: 0, iso_mode: IsoMode::Ascii }
    }

    pub fn encoding(&self) -> Encoding { self.encoding }
    pub fn buffered_input_bytes(&self) -> usize { 0 }

    pub fn push(&mut self, input: &str, output: &mut Vec<u8>) -> Result<(), EncodeError> {
        for ch in input.chars() {
            let cp = ch as u32;
            if !self.encode_one(cp, output) {
                let error_cp = if self.encoding == Encoding::Iso2022Jp
                    && matches!(cp, 0x0E | 0x0F | 0x1B) { 0xFFFD } else { cp };
                if self.fatal { return Err(EncodeError { character_offset: self.offset, code_point: error_cp }); }
                output.extend_from_slice(format!("&#{error_cp};").as_bytes());
            }
            self.offset += 1;
        }
        Ok(())
    }

    pub fn finish(&mut self, output: &mut Vec<u8>) -> Result<(), EncodeError> {
        if self.encoding == Encoding::Iso2022Jp && self.iso_mode != IsoMode::Ascii {
            output.extend_from_slice(b"\x1b(B");
            self.iso_mode = IsoMode::Ascii;
        }
        Ok(())
    }

    fn encode_one(&mut self, cp: u32, out: &mut Vec<u8>) -> bool {
        match self.encoding {
            Encoding::Utf8 => { out.extend_from_slice(char::from_u32(cp).unwrap().encode_utf8(&mut [0; 4]).as_bytes()); true }
            Encoding::Utf16Le | Encoding::Utf16Be => {
                for unit in char::from_u32(cp).unwrap().encode_utf16(&mut [0; 2]).iter() {
                    out.extend_from_slice(&if self.encoding == Encoding::Utf16Le { unit.to_le_bytes() } else { unit.to_be_bytes() });
                }
                true
            }
            Encoding::Replacement => false,
            Encoding::XUserDefined => {
                if cp < 0x80 { out.push(cp as u8); true }
                else if (0xF780..=0xF7FF).contains(&cp) { out.push((cp-0xF780+0x80) as u8); true }
                else { false }
            }
            Encoding::Gbk | Encoding::Gb18030 => self.encode_gb(cp, out),
            Encoding::Big5 => {
                if cp < 0x80 { out.push(cp as u8); return true; }
                let Some(pointer) = big5_pointer(cp) else { return false };
                let trail = pointer % 157;
                out.extend_from_slice(&[(pointer / 157 + 0x81) as u8,
                    (trail + if trail < 0x3F { 0x40 } else { 0x62 }) as u8]); true
            }
            Encoding::EucJp => {
                if cp < 0x80 { out.push(cp as u8); return true; }
                if cp == 0xA5 { out.push(0x5C); return true; }
                if cp == 0x203E { out.push(0x7E); return true; }
                if (0xFF61..=0xFF9F).contains(&cp) { out.extend_from_slice(&[0x8E, (cp - 0xFF61 + 0xA1) as u8]); return true; }
                let cp = if cp == 0x2212 { 0xFF0D } else { cp };
                let Some(pointer) = reverse::reverse("jis0208", cp) else { return false };
                out.extend_from_slice(&[(pointer / 94 + 0xA1) as u8, (pointer % 94 + 0xA1) as u8]); true
            }
            Encoding::Iso2022Jp => self.encode_iso2022(cp, out),
            Encoding::ShiftJis => {
                if cp < 0x80 || cp == 0x80 { out.push(cp as u8); return true; }
                if cp == 0xA5 { out.push(0x5C); return true; }
                if cp == 0x203E { out.push(0x7E); return true; }
                if (0xFF61..=0xFF9F).contains(&cp) { out.push((cp - 0xFF61 + 0xA1) as u8); return true; }
                let cp = if cp == 0x2212 { 0xFF0D } else { cp };
                let Some(pointer) = shift_jis_pointer(cp) else { return false };
                let lead = pointer / 188;
                let trail = pointer % 188;
                out.extend_from_slice(&[(lead + if lead < 0x1F {0x81} else {0xC1}) as u8,
                    (trail + if trail < 0x3F {0x40} else {0x41}) as u8]); true
            }
            Encoding::EucKr => {
                if cp < 0x80 { out.push(cp as u8); return true; }
                let Some(pointer) = reverse::reverse("euc-kr", cp) else { return false };
                out.extend_from_slice(&[(pointer / 190 + 0x81) as u8, (pointer % 190 + 0x41) as u8]); true
            }
            single => {
                if cp < 0x80 { out.push(cp as u8); return true; }
                let Some(pointer) = reverse::reverse(single.single_byte_index().unwrap(), cp) else { return false };
                out.push((pointer + 0x80) as u8); true
            }
        }
    }

    fn encode_gb(&self, cp: u32, out: &mut Vec<u8>) -> bool {
        if cp < 0x80 { out.push(cp as u8); return true; }
        if cp == 0xE5E5 { return false; }
        if self.encoding == Encoding::Gbk && cp == 0x20AC { out.push(0x80); return true; }
        const SPECIAL: &[(u32, u8, u8)] = &[
            (0xE78D,0xA6,0xD9),(0xE78E,0xA6,0xDA),(0xE78F,0xA6,0xDB),
            (0xE790,0xA6,0xDC),(0xE791,0xA6,0xDD),(0xE792,0xA6,0xDE),
            (0xE793,0xA6,0xDF),(0xE794,0xA6,0xEC),(0xE795,0xA6,0xED),
            (0xE796,0xA6,0xF3),(0xE81E,0xFE,0x59),(0xE826,0xFE,0x61),
            (0xE82B,0xFE,0x66),(0xE82C,0xFE,0x67),(0xE832,0xFE,0x6D),
            (0xE843,0xFE,0x7E),(0xE854,0xFE,0x90),(0xE864,0xFE,0xA0),
        ];
        if let Some(&(_, a, b)) = SPECIAL.iter().find(|&&(key, _, _)| key == cp) {
            out.extend_from_slice(&[a,b]); return true;
        }
        if let Some(pointer) = reverse::reverse("gb18030", cp) {
            let trail = pointer % 190;
            out.extend_from_slice(&[(pointer / 190 + 0x81) as u8,
                (trail + if trail < 0x3F {0x40} else {0x41}) as u8]);
            return true;
        }
        if self.encoding == Encoding::Gbk { return false; }
        let Some(pointer) = gb18030_range_pointer(cp) else { return false };
        out.extend_from_slice(&[(pointer / 12600 + 0x81) as u8,
            ((pointer % 12600) / 1260 + 0x30) as u8,
            ((pointer % 1260) / 10 + 0x81) as u8,
            (pointer % 10 + 0x30) as u8]); true
    }

    fn encode_iso2022(&mut self, cp: u32, out: &mut Vec<u8>) -> bool {
        if matches!(cp, 0x0E | 0x0F | 0x1B) {
            if self.iso_mode == IsoMode::Jis0208 {
                out.extend_from_slice(b"\x1b(B"); self.iso_mode = IsoMode::Ascii;
            }
            return false;
        }
        if cp < 0x80 {
            if self.iso_mode == IsoMode::Roman && cp != 0x5C && cp != 0x7E {
                out.push(cp as u8); return true;
            }
            if self.iso_mode != IsoMode::Ascii { out.extend_from_slice(b"\x1b(B"); self.iso_mode = IsoMode::Ascii; }
            out.push(cp as u8); return true;
        }
        if cp == 0xA5 || cp == 0x203E {
            if self.iso_mode != IsoMode::Roman { out.extend_from_slice(b"\x1b(J"); self.iso_mode = IsoMode::Roman; }
            out.push(if cp == 0xA5 { 0x5C } else { 0x7E }); return true;
        }
        let cp = if cp == 0x2212 { 0xFF0D } else if (0xFF61..=0xFF9F).contains(&cp) {
            tables::lookup("iso-2022-jp-katakana", (cp - 0xFF61) as usize).unwrap()
        } else { cp };
        let Some(pointer) = reverse::reverse("jis0208", cp) else {
            if self.iso_mode == IsoMode::Jis0208 { out.extend_from_slice(b"\x1b(B"); self.iso_mode = IsoMode::Ascii; }
            return false;
        };
        if self.iso_mode != IsoMode::Jis0208 { out.extend_from_slice(b"\x1b$B"); self.iso_mode = IsoMode::Jis0208; }
        out.extend_from_slice(&[(pointer / 94 + 0x21) as u8, (pointer % 94 + 0x21) as u8]); true
    }
}

fn gb18030_range_pointer(cp: u32) -> Option<u32> {
    if cp == 0xE7C7 { return Some(7457); }
    let ranges = &tables::GB18030_RANGES;
    let i = ranges.partition_point(|&(_, code_point)| code_point <= cp).checked_sub(1)?;
    Some(ranges[i].0 + cp - ranges[i].1)
}

fn shift_jis_pointer(cp: u32) -> Option<u32> {
    // The index excludes pointers 8272..=8835 for this encoder.
    let first = reverse::reverse("jis0208", cp)?;
    if !(8272..=8835).contains(&first) { return Some(first); }
    (8836..tables::INDEX_JIS0208.len()).find(|&pointer| tables::INDEX_JIS0208[pointer] == cp).map(|p| p as u32)
}

fn big5_pointer(cp: u32) -> Option<u32> {
    let start = (0xA1 - 0x81) * 157;
    let table = &tables::INDEX_BIG5;
    if matches!(cp, 0x2550 | 0x255E | 0x2561 | 0x256A | 0x5341 | 0x5345) {
        return (start..table.len()).rev().find(|&p| table[p] == cp).map(|p| p as u32);
    }
    let first = reverse::reverse("big5", cp)?;
    if first >= start as u32 { Some(first) }
    else { (start..table.len()).find(|&p| table[p] == cp).map(|p| p as u32) }
}
