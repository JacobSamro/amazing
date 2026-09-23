//! Pull decoders. Each read advances the compressed source only as needed;
//! DEFLATE retains a 32 KiB history ring, never the complete encoded body.
use super::{crc32_update, inflate, invalid};
use crate::NetError;
use inflate::BitRead;
use std::{cell::Cell, io::Read, rc::Rc};

pub(crate) trait Source {
    fn read(&mut self, output: &mut [u8]) -> Result<usize, NetError>;
}

pub(crate) struct IoSource<R: Read>(pub R);
impl<R: Read> Source for IoSource<R> {
    fn read(&mut self, output: &mut [u8]) -> Result<usize, NetError> {
        Ok(self.0.read(output)?)
    }
}

pub(crate) fn read_exact(source: &mut dyn Source, mut bytes: &mut [u8]) -> Result<(), NetError> {
    while !bytes.is_empty() {
        let n = source.read(bytes)?;
        if n == 0 {
            return Err(NetError::IncompleteBody);
        }
        bytes = &mut bytes[n..];
    }
    Ok(())
}

pub(crate) struct CountSource {
    pub inner: Box<dyn Source>,
    pub count: Rc<Cell<usize>>,
}
impl Source for CountSource {
    fn read(&mut self, output: &mut [u8]) -> Result<usize, NetError> {
        let n = self.inner.read(output)?;
        self.count.set(self.count.get().saturating_add(n));
        Ok(n)
    }
}

/// A one-byte probe detects a size/ratio violation in fixed caller storage,
/// before any allocation or delivery of the violating byte.
pub(crate) struct CapSource {
    pub inner: Box<dyn Source>,
    pub total: usize,
    pub limit: usize,
    pub ratio: Option<(Rc<Cell<usize>>, usize)>,
}
impl Source for CapSource {
    fn read(&mut self, output: &mut [u8]) -> Result<usize, NetError> {
        if output.is_empty() {
            return Ok(0);
        }
        let allowed = self.limit.saturating_sub(self.total);
        let count = output.len().min(allowed.saturating_add(1));
        let n = self.inner.read(&mut output[..count])?;
        if n > allowed {
            return Err(NetError::ResponseTooLarge);
        }
        let next = self
            .total
            .checked_add(n)
            .ok_or(NetError::ResponseTooLarge)?;
        if let Some((encoded, ratio)) = &self.ratio {
            // Permit small headers and short streams without spuriously
            // rejecting them. The check is cumulative, never per read.
            let ratio_limit = encoded.get().saturating_mul(*ratio).saturating_add(65536);
            if next > ratio_limit {
                return Err(NetError::ResponseTooLarge);
            }
        }
        self.total = next;
        Ok(n)
    }
}

struct StreamBits {
    input: Box<dyn Source>,
    byte: u8,
    left: u8,
}
impl StreamBits {
    fn into_inner(self) -> Box<dyn Source> {
        self.input
    }
}
impl inflate::BitRead for StreamBits {
    fn read(&mut self, count: usize) -> Result<usize, NetError> {
        let mut result = 0;
        for shift in 0..count {
            if self.left == 0 {
                read_exact(self.input.as_mut(), std::slice::from_mut(&mut self.byte))?;
                self.left = 8;
            }
            result |= ((self.byte & 1) as usize) << shift;
            self.byte >>= 1;
            self.left -= 1;
        }
        Ok(result)
    }
    fn align(&mut self) {
        self.left = 0;
    }
}

enum Block {
    New,
    Stored {
        left: usize,
        final_block: bool,
    },
    Codes {
        literal: inflate::Huffman,
        distance: inflate::Huffman,
        final_block: bool,
        copy: Option<(usize, usize)>,
    },
    Done,
}

/// Resumable RFC1951 decoder. The caller may stop after any output chunk.
pub(crate) struct DeflateSource {
    bits: StreamBits,
    state: Block,
    history: Box<[u8; 32768]>,
    write: usize,
    produced: usize,
    window: usize,
}
impl DeflateSource {
    pub fn new(input: Box<dyn Source>, window: usize) -> Self {
        Self {
            bits: StreamBits {
                input,
                byte: 0,
                left: 0,
            },
            state: Block::New,
            history: Box::new([0; 32768]),
            write: 0,
            produced: 0,
            window,
        }
    }
    pub fn into_inner(self) -> Box<dyn Source> {
        self.bits.into_inner()
    }
    fn emit(&mut self, byte: u8, output: &mut [u8], used: &mut usize) {
        output[*used] = byte;
        *used += 1;
        self.history[self.write] = byte;
        self.write = (self.write + 1) % 32768;
        self.produced += 1;
    }
    fn fixed() -> Result<(inflate::Huffman, inflate::Huffman), NetError> {
        let mut lengths = [8; 288];
        lengths[144..256].fill(9);
        lengths[256..280].fill(7);
        Ok((
            inflate::Huffman::new(&lengths, false)?,
            inflate::Huffman::new(&[5; 32], false)?,
        ))
    }
}
impl Source for DeflateSource {
    fn read(&mut self, output: &mut [u8]) -> Result<usize, NetError> {
        if output.is_empty() {
            return Ok(0);
        }
        let mut used = 0;
        while used < output.len() {
            let state = std::mem::replace(&mut self.state, Block::Done);
            match state {
                Block::New => {
                    let final_block = self.bits.read(1)? != 0;
                    self.state = match self.bits.read(2)? {
                        0 => {
                            self.bits.align();
                            let len = self.bits.read(16)?;
                            if self.bits.read(16)? != len ^ 0xffff {
                                return Err(invalid("stored LEN/NLEN mismatch"));
                            }
                            Block::Stored {
                                left: len,
                                final_block,
                            }
                        }
                        kind @ (1 | 2) => {
                            let (literal, distance) = if kind == 1 {
                                Self::fixed()?
                            } else {
                                inflate::trees(&mut self.bits)?
                            };
                            Block::Codes {
                                literal,
                                distance,
                                final_block,
                                copy: None,
                            }
                        }
                        _ => return Err(invalid("reserved block type")),
                    };
                }
                Block::Stored {
                    mut left,
                    final_block,
                } => {
                    if left == 0 {
                        self.state = if final_block { Block::Done } else { Block::New };
                        continue;
                    }
                    let byte = self.bits.read(8)? as u8;
                    self.emit(byte, output, &mut used);
                    left -= 1;
                    self.state = Block::Stored { left, final_block };
                }
                Block::Codes {
                    literal,
                    distance,
                    final_block,
                    mut copy,
                } => {
                    if let Some((mut left, dist)) = copy.take() {
                        if dist == 0 || dist > self.window || dist > self.produced {
                            return Err(invalid("distance exceeds DEFLATE history"));
                        }
                        let byte = self.history[(self.write + 32768 - dist) % 32768];
                        self.emit(byte, output, &mut used);
                        left -= 1;
                        copy = (left > 0).then_some((left, dist));
                    } else {
                        match literal.symbol(&mut self.bits)? {
                            byte @ 0..=255 => self.emit(byte as u8, output, &mut used),
                            256 => {
                                self.state = if final_block { Block::Done } else { Block::New };
                                continue;
                            }
                            symbol @ 257..=285 => {
                                let index = symbol - 257;
                                let length = inflate::LENGTH[index]
                                    + self.bits.read(inflate::LENGTH_BITS[index])?;
                                let index = distance.symbol(&mut self.bits)?;
                                if index >= 30 {
                                    return Err(invalid("reserved distance code"));
                                }
                                let dist = inflate::DISTANCE[index]
                                    + self.bits.read(inflate::DISTANCE_BITS[index])?;
                                copy = Some((length, dist));
                            }
                            _ => return Err(invalid("reserved literal/length code")),
                        }
                    }
                    self.state = Block::Codes {
                        literal,
                        distance,
                        final_block,
                        copy,
                    };
                }
                Block::Done => {
                    self.state = Block::Done;
                    break;
                }
            }
        }
        Ok(used)
    }
}

fn byte(input: &mut dyn Source) -> Result<Option<u8>, NetError> {
    let mut b = [0];
    match input.read(&mut b)? {
        0 => Ok(None),
        _ => Ok(Some(b[0])),
    }
}
fn required(input: &mut dyn Source) -> Result<u8, NetError> {
    byte(input)?.ok_or(NetError::IncompleteBody)
}

pub(crate) struct ZlibSource {
    deflate: Option<DeflateSource>,
    a: u32,
    b: u32,
    done: bool,
}
impl ZlibSource {
    pub fn new(mut input: Box<dyn Source>) -> Result<Self, NetError> {
        let cmf = required(input.as_mut())?;
        let flg = required(input.as_mut())?;
        if cmf & 15 != 8 || cmf >> 4 > 7 || u16::from_be_bytes([cmf, flg]) % 31 != 0 {
            return Err(invalid("invalid zlib header"));
        }
        if flg & 32 != 0 {
            return Err(invalid("zlib preset dictionary unsupported"));
        }
        let window = 1usize << ((cmf >> 4) + 8);
        Ok(Self {
            deflate: Some(DeflateSource::new(input, window)),
            a: 1,
            b: 0,
            done: false,
        })
    }
}
impl Source for ZlibSource {
    fn read(&mut self, output: &mut [u8]) -> Result<usize, NetError> {
        if output.is_empty() || self.done {
            return Ok(0);
        }
        let n = self.deflate.as_mut().unwrap().read(output)?;
        if n > 0 {
            for &value in &output[..n] {
                self.a = (self.a + u32::from(value)) % 65521;
                self.b = (self.b + self.a) % 65521;
            }
            return Ok(n);
        }
        let mut input = self.deflate.take().unwrap().into_inner();
        let mut trailer = [0; 4];
        read_exact(input.as_mut(), &mut trailer)?;
        if u32::from_be_bytes(trailer) != (self.b << 16) | self.a {
            return Err(invalid("zlib Adler32 mismatch"));
        }
        if byte(input.as_mut())?.is_some() {
            return Err(invalid("trailing zlib data"));
        }
        self.done = true;
        Ok(0)
    }
}

pub(crate) struct GzipSource {
    input: Option<Box<dyn Source>>,
    deflate: Option<DeflateSource>,
    crc: u32,
    size: u32,
    members: usize,
    done: bool,
}
impl GzipSource {
    pub fn new(input: Box<dyn Source>) -> Self {
        Self {
            input: Some(input),
            deflate: None,
            crc: !0,
            size: 0,
            members: 0,
            done: false,
        }
    }
    fn header(&mut self) -> Result<bool, NetError> {
        let input = self.input.as_mut().unwrap();
        let Some(first) = byte(input.as_mut())? else {
            if self.members == 0 {
                return Err(NetError::IncompleteBody);
            }
            return Ok(false);
        };
        let mut crc = crc32_update(!0, &[first]);
        let mut head = [0; 9];
        read_exact(input.as_mut(), &mut head)?;
        crc = crc32_update(crc, &head);
        if first != 0x1f || head[0] != 0x8b || head[1] != 8 || head[2] & 0xe0 != 0 {
            return Err(invalid("invalid gzip header"));
        }
        let flags = head[2];
        let mut header_bytes = 10usize;
        let mut next = |input: &mut dyn Source, crc: &mut u32| -> Result<u8, NetError> {
            if header_bytes == 65536 {
                return Err(NetError::ResponseTooLarge);
            }
            let value = required(input)?;
            *crc = crc32_update(*crc, &[value]);
            header_bytes += 1;
            Ok(value)
        };
        if flags & 4 != 0 {
            let len = u16::from_le_bytes([
                next(input.as_mut(), &mut crc)?,
                next(input.as_mut(), &mut crc)?,
            ]) as usize;
            for _ in 0..len {
                next(input.as_mut(), &mut crc)?;
            }
        }
        for flag in [8, 16] {
            if flags & flag != 0 {
                while next(input.as_mut(), &mut crc)? != 0 {}
            }
        }
        if flags & 2 != 0 {
            let expected =
                u16::from_le_bytes([required(input.as_mut())?, required(input.as_mut())?]);
            if expected != (!crc as u16) {
                return Err(invalid("gzip header CRC mismatch"));
            }
        }
        let input = self.input.take().unwrap();
        self.deflate = Some(DeflateSource::new(input, 32768));
        self.crc = !0;
        self.size = 0;
        self.members += 1;
        Ok(true)
    }
}
impl Source for GzipSource {
    fn read(&mut self, output: &mut [u8]) -> Result<usize, NetError> {
        if output.is_empty() || self.done {
            return Ok(0);
        }
        loop {
            if self.deflate.is_none() && !self.header()? {
                self.done = true;
                return Ok(0);
            }
            let n = self.deflate.as_mut().unwrap().read(output)?;
            if n > 0 {
                self.crc = crc32_update(self.crc, &output[..n]);
                self.size = self.size.wrapping_add(n as u32);
                return Ok(n);
            }
            let mut input = self.deflate.take().unwrap().into_inner();
            let mut trailer = [0; 8];
            read_exact(input.as_mut(), &mut trailer)?;
            if u32::from_le_bytes(trailer[..4].try_into().unwrap()) != !self.crc {
                return Err(invalid("gzip CRC32 mismatch"));
            }
            if u32::from_le_bytes(trailer[4..].try_into().unwrap()) != self.size {
                return Err(invalid("gzip ISIZE mismatch"));
            }
            self.input = Some(input);
        }
    }
}

pub(crate) struct BrotliSource {
    input: Box<dyn Source>,
    decoder: amazing_net_brotli::StreamDecoder,
    buffer: [u8; 8192],
    pos: usize,
    end: usize,
    done: bool,
}
impl BrotliSource {
    pub fn new(input: Box<dyn Source>, limit: usize) -> Self {
        Self {
            input,
            decoder: amazing_net_brotli::StreamDecoder::new(limit),
            buffer: [0; 8192],
            pos: 0,
            end: 0,
            done: false,
        }
    }
}
impl Source for BrotliSource {
    fn read(&mut self, output: &mut [u8]) -> Result<usize, NetError> {
        if output.is_empty() || self.done {
            return Ok(0);
        }
        loop {
            if self.pos == self.end {
                self.pos = 0;
                self.end = self.input.read(&mut self.buffer)?;
            }
            let step = self
                .decoder
                .step(&self.buffer[self.pos..self.end], output)
                .map_err(|error| match error {
                    amazing_net_brotli::DecodeError::OutputLimit => NetError::ResponseTooLarge,
                    other => invalid(&other.to_string()),
                })?;
            self.pos += step.consumed;
            if step.finished {
                if self.pos != self.end || byte(self.input.as_mut())?.is_some() {
                    return Err(invalid("brotli trailing data"));
                }
                self.done = true;
            }
            if step.written > 0 {
                return Ok(step.written);
            }
            if self.done {
                return Ok(0);
            }
            if self.end == 0 {
                return Err(NetError::IncompleteBody);
            }
            if step.consumed == 0 && self.pos != self.end {
                return Err(invalid("brotli decoder made no progress"));
            }
        }
    }
}

pub(crate) fn decode_layer(
    input: Box<dyn Source>,
    coding: &str,
    limit: usize,
) -> Result<Box<dyn Source>, NetError> {
    let decoded: Box<dyn Source> = match coding {
        "gzip" | "x-gzip" => Box::new(GzipSource::new(input)),
        "deflate" => Box::new(ZlibSource::new(input)?),
        "br" => Box::new(BrotliSource::new(input, limit)),
        "identity" => input,
        _ => return Err(invalid("unsupported encoding")),
    };
    Ok(Box::new(CapSource {
        inner: decoded,
        total: 0,
        limit,
        ratio: None,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    struct OneByte {
        bytes: Vec<u8>,
        pos: usize,
    }
    impl Source for OneByte {
        fn read(&mut self, output: &mut [u8]) -> Result<usize, NetError> {
            if output.is_empty() || self.pos == self.bytes.len() {
                return Ok(0);
            }
            output[0] = self.bytes[self.pos];
            self.pos += 1;
            Ok(1)
        }
    }
    fn collect_with_chunk(mut source: Box<dyn Source>, chunk: usize) -> Result<Vec<u8>, NetError> {
        let mut result = Vec::new();
        let mut buffer = vec![0; chunk];
        loop {
            let n = source.read(&mut buffer)?;
            if n == 0 {
                return Ok(result);
            }
            result.extend_from_slice(&buffer[..n]);
        }
    }
    fn collect(source: Box<dyn Source>) -> Result<Vec<u8>, NetError> {
        collect_with_chunk(source, 7)
    }
    #[test]
    fn one_byte_deflate_gzip_zlib_and_adversarial_output_splits() {
        for name in [
            "hello-stored-raw",
            "hello-fixed-raw",
            "dynamic-default-raw",
            "hello-stored-zlib",
            "hello-fixed-zlib",
            "dynamic-default-zlib",
            "hello-stored-gzip",
            "hello-fixed-gzip",
            "dynamic-default-gzip",
            "overlap-default-gzip",
        ] {
            let (plain, encoded) = crate::coding::tests::fixture(name);
            let whole = if name.ends_with("raw") {
                plain.clone()
            } else {
                crate::coding::decode(
                    &encoded,
                    if name.ends_with("gzip") { "gzip" } else { "deflate" },
                    plain.len(),
                )
                .unwrap()
            };
            for chunk in [1, 2, 7, 8192] {
                let input: Box<dyn Source> = Box::new(OneByte {
                    bytes: encoded.clone(),
                    pos: 0,
                });
                let source: Box<dyn Source> = if name.ends_with("raw") {
                    Box::new(DeflateSource::new(input, 32768))
                } else if name.ends_with("gzip") {
                    Box::new(GzipSource::new(input))
                } else {
                    Box::new(ZlibSource::new(input).unwrap())
                };
                assert_eq!(collect_with_chunk(source, chunk).unwrap(), whole, "{name}/{chunk}");
            }
        }
    }
    #[test]
    fn brotli_one_byte_input_and_output_below_limit() {
        let plain = include_bytes!("../../../amazing-net-brotli/tests/vectors/hello.raw");
        let encoded = include_bytes!("../../../amazing-net-brotli/tests/vectors/hello-11.br");
        let input: Box<dyn Source> = Box::new(OneByte {
            bytes: encoded.to_vec(),
            pos: 0,
        });
        assert_eq!(
            collect(Box::new(BrotliSource::new(input, plain.len()))).unwrap(),
            plain
        );
        let input: Box<dyn Source> = Box::new(OneByte {
            bytes: encoded.to_vec(),
            pos: 0,
        });
        assert!(matches!(
            collect(Box::new(BrotliSource::new(input, plain.len() - 1))),
            Err(NetError::ResponseTooLarge)
        ));
    }
    #[test]
    fn independent_brotli_vectors_stream_in_single_byte_reads() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../amazing-net-brotli/tests/vectors");
        for name in ["empty", "hello", "unicode", "binary", "random"] {
            let plain = std::fs::read(root.join(format!("{name}.raw"))).unwrap();
            for quality in [0, 5, 11] {
                let encoded = std::fs::read(root.join(format!("{name}-{quality}.br"))).unwrap();
                let whole = amazing_net_brotli::decode(&encoded, plain.len()).unwrap();
                for chunk in [1, 7, 8192] {
                    let input: Box<dyn Source> = Box::new(OneByte {
                        bytes: encoded.clone(),
                        pos: 0,
                    });
                    assert_eq!(
                        collect_with_chunk(Box::new(BrotliSource::new(input, plain.len())), chunk)
                            .unwrap(),
                        whole,
                        "{name}-{quality}/{chunk}"
                    );
                }
            }
        }
    }
    #[test]
    fn cumulative_cap_and_ratio_reject_before_delivery() {
        let bytes = [b'x'; 128];
        let source: Box<dyn Source> = Box::new(OneByte {
            bytes: bytes.to_vec(),
            pos: 0,
        });
        let mut capped = CapSource {
            inner: source,
            total: 0,
            limit: 17,
            ratio: None,
        };
        let mut output = [0; 8];
        assert_eq!(capped.read(&mut output).unwrap(), 1);
        while capped.total < 17 {
            capped.read(&mut output).unwrap();
        }
        assert!(matches!(
            capped.read(&mut output),
            Err(NetError::ResponseTooLarge)
        ));
        assert_eq!(capped.total, 17);
        let encoded = Rc::new(Cell::new(0));
        let source: Box<dyn Source> = Box::new(IoSource(std::io::Cursor::new(vec![0; 65538])));
        let mut capped = CapSource {
            inner: source,
            total: 0,
            limit: 100000,
            ratio: Some((encoded, 1)),
        };
        let mut output = [0; 8192];
        while capped.total <= 65536 {
            if matches!(capped.read(&mut output), Err(NetError::ResponseTooLarge)) {
                return;
            }
        }
        panic!("ratio did not reject");
    }
    #[test]
    fn real_brotli_bomb_ratio_is_cumulative_across_output_chunks() {
        let encoded = include_bytes!("../../../amazing-net-brotli/tests/vectors/bomb-11.br");
        let count = Rc::new(Cell::new(0));
        let input: Box<dyn Source> = Box::new(CountSource {
            inner: Box::new(OneByte {
                bytes: encoded.to_vec(),
                pos: 0,
            }),
            count: count.clone(),
        });
        let decoded: Box<dyn Source> = Box::new(BrotliSource::new(input, 2 * 1024 * 1024));
        let mut capped = CapSource {
            inner: decoded,
            total: 0,
            limit: 2 * 1024 * 1024,
            ratio: Some((count, 1)),
        };
        let mut output = [0; 8192];
        loop {
            match capped.read(&mut output) {
                Ok(n) => assert!(n > 0, "bomb completed before the ratio bound fired"),
                Err(NetError::ResponseTooLarge) => break,
                Err(error) => panic!("unexpected bomb error: {error}"),
            }
        }
        assert!(capped.total <= 65536 + encoded.len());
    }
}
