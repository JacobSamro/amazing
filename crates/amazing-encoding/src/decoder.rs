use crate::{generated::tables, Encoding};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DecodeError { pub byte_offset: usize }

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum IsoMode { Ascii, Roman, Katakana, Jis0208 }
#[derive(Clone, Copy, Debug)]
enum Step { Need, Skip(usize), Emit(u32, usize), Pair(u32, u32, usize), Error(usize) }

/// Streaming decoder. At most four undecoded bytes are retained between calls;
/// decoded output is written into the caller's `String`.
pub struct Decoder {
    encoding: Encoding,
    fatal: bool,
    pending: Vec<u8>,
    offset: usize,
    replacement_seen: bool,
    iso_mode: IsoMode,
    iso_escape_output: bool,
}
impl Decoder {
    pub fn new(encoding: Encoding, fatal: bool) -> Self {
        Self { encoding, fatal, pending: Vec::with_capacity(4), offset: 0,
            replacement_seen: false, iso_mode: IsoMode::Ascii, iso_escape_output: false }
    }
    pub fn encoding(&self) -> Encoding { self.encoding }
    pub fn buffered_input_bytes(&self) -> usize { self.pending.len() }
    pub fn push(&mut self, input: &[u8], output: &mut String) -> Result<(), DecodeError> {
        for &byte in input {
            self.pending.push(byte);
            self.drain(output, false)?;
        }
        Ok(())
    }
    pub fn finish(&mut self, output: &mut String) -> Result<(), DecodeError> {
        self.drain(output, true)
    }
    fn drain(&mut self, output: &mut String, final_input: bool) -> Result<(), DecodeError> {
        while !self.pending.is_empty() {
            let mut step = self.step();
            if matches!(step, Step::Need) && final_input {
                let n = if matches!(self.encoding, Encoding::Utf16Le | Encoding::Utf16Be)
                    && self.pending.len() == 3 { 2 }
                    else if self.encoding == Encoding::Iso2022Jp { 1 }
                    else { self.pending.len() };
                step = Step::Error(n);
            }
            let consumed = match step {
                Step::Need => break,
                Step::Skip(n) => n,
                Step::Emit(cp, n) => { output.push(char::from_u32(cp).expect("index scalar")); n },
                Step::Pair(a, b, n) => {
                    output.push(char::from_u32(a).expect("index scalar"));
                    output.push(char::from_u32(b).expect("index scalar")); n
                }
                Step::Error(n) => {
                    if self.fatal { return Err(DecodeError { byte_offset: self.offset }); }
                    output.push('\u{FFFD}'); n
                }
            };
            self.pending.drain(..consumed);
            self.offset += consumed;
        }
        Ok(())
    }
    fn step(&mut self) -> Step {
        let p = &self.pending;
        let first = p[0];
        match self.encoding {
            Encoding::Utf8 => match std::str::from_utf8(p) {
                Ok(valid) => Step::Emit(valid.chars().next().expect("nonempty") as u32, p.len()),
                Err(error) => error.error_len().map_or(Step::Need, Step::Error),
            },
            Encoding::Utf16Le | Encoding::Utf16Be => {
                if p.len() < 2 { return Step::Need; }
                let unit = if self.encoding == Encoding::Utf16Le {
                    u16::from_le_bytes([p[0], p[1]])
                } else { u16::from_be_bytes([p[0], p[1]]) };
                if (0xD800..=0xDBFF).contains(&unit) {
                    if p.len() < 4 { return Step::Need; }
                    let next = if self.encoding == Encoding::Utf16Le {
                        u16::from_le_bytes([p[2], p[3]])
                    } else { u16::from_be_bytes([p[2], p[3]]) };
                    if (0xDC00..=0xDFFF).contains(&next) {
                        Step::Emit(0x10000 + (((unit as u32)-0xD800)<<10) + (next as u32)-0xDC00, 4)
                    } else { Step::Error(2) }
                } else if (0xDC00..=0xDFFF).contains(&unit) { Step::Error(2) }
                else { Step::Emit(unit as u32, 2) }
            }
            Encoding::Replacement => {
                if self.replacement_seen { Step::Skip(1) }
                else { self.replacement_seen = true; Step::Error(1) }
            }
            Encoding::XUserDefined => {
                Step::Emit(if first < 0x80 { first as u32 } else { 0xF780 + first as u32 - 0x80 }, 1)
            }
            Encoding::Gbk | Encoding::Gb18030 => self.gb18030_step(),
            Encoding::Big5 => self.big5_step(),
            Encoding::EucJp => self.euc_jp_step(),
            Encoding::ShiftJis => self.shift_jis_step(),
            Encoding::EucKr => self.euc_kr_step(),
            Encoding::Iso2022Jp => self.iso2022_step(),
            single => {
                if first < 0x80 { Step::Emit(first as u32, 1) }
                else { tables::lookup(single.single_byte_index().expect("single-byte"),
                    (first - 0x80) as usize).map_or(Step::Error(1), |cp| Step::Emit(cp, 1)) }
            }
        }
    }
    fn gb18030_step(&self) -> Step {
        let p = &self.pending; let first = p[0];
        if first < 0x80 { return Step::Emit(first as u32, 1); }
        if first == 0x80 { return Step::Emit(0x20AC, 1); }
        if !(0x81..=0xFE).contains(&first) { return Step::Error(1); }
        if p.len() < 2 { return Step::Need; }
        let second = p[1];
        if (0x30..=0x39).contains(&second) {
            if p.len() < 3 { return Step::Need; }
            if !(0x81..=0xFE).contains(&p[2]) { return Step::Error(1); }
            if p.len() < 4 { return Step::Need; }
            if !(0x30..=0x39).contains(&p[3]) { return Step::Error(1); }
            let pointer = ((first as u32-0x81)*12600) + ((second as u32-0x30)*1260)
                + ((p[2] as u32-0x81)*10) + (p[3] as u32-0x30);
            return gb18030_range_code_point(pointer).map_or(Step::Error(4), |cp| Step::Emit(cp,4));
        }
        if !((0x40..=0x7E).contains(&second) || (0x80..=0xFE).contains(&second)) {
            return Step::Error(if second < 0x80 { 1 } else { 2 });
        }
        let offset = if second < 0x7F { 0x40 } else { 0x41 };
        let pointer = ((first as usize - 0x81)*190) + (second as usize - offset);
        tables::lookup("gb18030", pointer).map_or(Step::Error(2), |cp| Step::Emit(cp, 2))
    }
    fn big5_step(&self) -> Step {
        let p=&self.pending; let first=p[0];
        if first < 0x80 { return Step::Emit(first as u32, 1); }
        if !(0x81..=0xFE).contains(&first) { return Step::Error(1); }
        if p.len()<2 { return Step::Need; }
        let second=p[1];
        if !((0x40..=0x7E).contains(&second) || (0xA1..=0xFE).contains(&second)) {
            return Step::Error(if second < 0x80 { 1 } else { 2 });
        }
        let offset=if second < 0x7F {0x40} else {0x62};
        let pointer=(first as usize-0x81)*157+(second as usize-offset);
        match pointer {
            1133 => Step::Pair(0xCA,0x304,2), 1135 => Step::Pair(0xCA,0x30C,2),
            1164 => Step::Pair(0xEA,0x304,2), 1166 => Step::Pair(0xEA,0x30C,2),
            _ => tables::lookup("big5",pointer).map_or(Step::Error(2), |cp| Step::Emit(cp,2)),
        }
    }
    fn euc_jp_step(&self) -> Step {
        let p=&self.pending; let first=p[0];
        if first < 0x80 { return Step::Emit(first as u32,1); }
        if first==0x8E {
            if p.len()<2 { return Step::Need; }
            return if (0xA1..=0xDF).contains(&p[1]) {
                Step::Emit(0xFF61+p[1] as u32-0xA1,2)
            } else { Step::Error(if p[1]<0x80 {1} else {2}) };
        }
        if first==0x8F {
            if p.len()<2 { return Step::Need; }
            if !(0xA1..=0xFE).contains(&p[1]) { return Step::Error(if p[1]<0x80 {1}else{2}); }
            if p.len()<3 { return Step::Need; }
            if !(0xA1..=0xFE).contains(&p[2]) { return Step::Error(if p[2]<0x80 {2}else{3}); }
            let pointer=(p[1] as usize-0xA1)*94+(p[2] as usize-0xA1);
            return tables::lookup("jis0212",pointer).map_or(Step::Error(3), |cp| Step::Emit(cp,3));
        }
        if !(0xA1..=0xFE).contains(&first) { return Step::Error(1); }
        if p.len()<2 { return Step::Need; }
        if !(0xA1..=0xFE).contains(&p[1]) { return Step::Error(if p[1]<0x80 {1}else{2}); }
        let pointer=(first as usize-0xA1)*94+(p[1] as usize-0xA1);
        tables::lookup("jis0208",pointer).map_or(Step::Error(2), |cp| Step::Emit(cp,2))
    }
    fn shift_jis_step(&self) -> Step {
        let p=&self.pending; let first=p[0];
        if first<0x80 || first==0x80 { return Step::Emit(first as u32,1); }
        if (0xA1..=0xDF).contains(&first) { return Step::Emit(0xFF61+first as u32-0xA1,1); }
        if !((0x81..=0x9F).contains(&first)||(0xE0..=0xFC).contains(&first)) { return Step::Error(1); }
        if p.len()<2 { return Step::Need; }
        let second=p[1];
        if !((0x40..=0x7E).contains(&second)||(0x80..=0xFC).contains(&second)) {
            return Step::Error(if second<0x80 {1}else{2});
        }
        let lead_offset=if first<0xA0 {0x81}else{0xC1};
        let offset=if second<0x7F {0x40}else{0x41};
        let pointer=(first as usize-lead_offset)*188+(second as usize-offset);
        if (8836..=10715).contains(&pointer) { return Step::Emit(0xE000+pointer as u32-8836,2); }
        tables::lookup("jis0208",pointer).map_or(Step::Error(2), |cp| Step::Emit(cp,2))
    }
    fn euc_kr_step(&self) -> Step {
        let p=&self.pending; let first=p[0];
        if first<0x80 { return Step::Emit(first as u32,1); }
        if !(0x81..=0xFE).contains(&first) { return Step::Error(1); }
        if p.len()<2 { return Step::Need; }
        let second=p[1];
        if !(0x41..=0xFE).contains(&second) {
            return Step::Error(if second<0x80 {1}else{2});
        }
        let pointer=(first as usize-0x81)*190+(second as usize-0x41);
        tables::lookup("euc-kr",pointer).map_or(Step::Error(2), |cp| Step::Emit(cp,2))
    }
    fn iso2022_step(&mut self) -> Step {
        let p=&self.pending; let first=p[0];
        if first==0x1B {
            if p.len()<2 { return Step::Need; }
            if p[1]!=0x24 && p[1]!=0x28 { self.iso_escape_output=false; return Step::Error(1); }
            if p.len()<3 { return Step::Need; }
            let next=match (p[1],p[2]) {
                (0x28,0x42)=>Some(IsoMode::Ascii), (0x28,0x4A)=>Some(IsoMode::Roman),
                (0x28,0x49)=>Some(IsoMode::Katakana),
                (0x24,0x40)|(0x24,0x42)=>Some(IsoMode::Jis0208), _=>None,
            };
            if let Some(mode)=next {
                self.iso_mode=mode;
                let repeated=self.iso_escape_output;
                self.iso_escape_output=true;
                return if repeated { Step::Error(3) } else { Step::Skip(3) };
            }
            self.iso_escape_output=false;
            return Step::Error(1);
        }
        self.iso_escape_output=false;
        match self.iso_mode {
            IsoMode::Ascii | IsoMode::Roman => {
                if first==0x0E || first==0x0F || first>=0x80 { return Step::Error(1); }
                if self.iso_mode==IsoMode::Roman {
                    if first==0x5C { return Step::Emit(0xA5,1); }
                    if first==0x7E { return Step::Emit(0x203E,1); }
                }
                Step::Emit(first as u32,1)
            }
            IsoMode::Katakana => {
                if (0x21..=0x5F).contains(&first) { Step::Emit(0xFF61+first as u32-0x21,1) }
                else { Step::Error(1) }
            }
            IsoMode::Jis0208 => {
                if !(0x21..=0x7E).contains(&first) { return Step::Error(1); }
                if p.len()<2 { return Step::Need; }
                if p[1]==0x1B { return Step::Error(1); }
                if !(0x21..=0x7E).contains(&p[1]) { return Step::Error(2); }
                let pointer=(first as usize-0x21)*94+(p[1] as usize-0x21);
                tables::lookup("jis0208",pointer).map_or(Step::Error(2), |cp| Step::Emit(cp,2))
            }
        }
    }
}

pub(crate) fn gb18030_range_code_point(pointer: u32) -> Option<u32> {
    if (39420..189000).contains(&pointer) || pointer > 1237575 { return None; }
    if pointer == 7457 { return Some(0xE7C7); }
    let ranges=&tables::GB18030_RANGES;
    let i=ranges.partition_point(|&(start,_)| start<=pointer).checked_sub(1)?;
    let cp=ranges[i].1+(pointer-ranges[i].0);
    (cp<=0x10FFFF && !(0xD800..=0xDFFF).contains(&cp)).then_some(cp)
}
