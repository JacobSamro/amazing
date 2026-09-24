//! Minimal lossless RGBA PNG writer. Rows use filter zero and stored DEFLATE blocks.
use crate::ImageData;

fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for byte in data {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320u32 & (0u32.wrapping_sub(crc & 1)));
        }
    }
    !crc
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let start = out.len() - data.len() - 4;
    out.extend_from_slice(&crc32(&out[start..]).to_be_bytes());
}

/// Encode a bounded, straight-alpha RGBA bitmap as an 8-bit color type 6 PNG.
pub fn encode_png(image: &ImageData) -> Vec<u8> {
    let stride = image.width as usize * 4;
    let mut scanlines = Vec::with_capacity(image.data.len() + image.height as usize);
    for row in image.data.chunks_exact(stride) {
        scanlines.push(0);
        scanlines.extend_from_slice(row);
    }
    let mut zlib = vec![0x78, 0x01];
    for (index, block) in scanlines.chunks(65_535).enumerate() {
        let final_block = index == (scanlines.len().saturating_sub(1)) / 65_535;
        zlib.push(u8::from(final_block));
        let len = block.len() as u16;
        zlib.extend_from_slice(&len.to_le_bytes());
        zlib.extend_from_slice(&(!len).to_le_bytes());
        zlib.extend_from_slice(block);
    }
    if scanlines.is_empty() {
        zlib.extend_from_slice(&[1, 0, 0, 255, 255]);
    }
    const MOD: u32 = 65_521;
    let mut s1 = 1u32;
    let mut s2 = 0u32;
    for byte in &scanlines {
        s1 = (s1 + u32::from(*byte)) % MOD;
        s2 = (s2 + s1) % MOD;
    }
    zlib.extend_from_slice(&((s2 << 16) | s1).to_be_bytes());
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut header = Vec::with_capacity(13);
    header.extend_from_slice(&image.width.to_be_bytes());
    header.extend_from_slice(&image.height.to_be_bytes());
    header.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &header);
    chunk(&mut out, b"IDAT", &zlib);
    chunk(&mut out, b"IEND", &[]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn png_has_valid_chunks_and_uncompressed_pixel() {
        let image = ImageData::from_rgba(1, 1, vec![12, 34, 56, 78]).unwrap();
        let bytes = encode_png(&image);
        assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert!(bytes.windows(5).any(|window| window == [0, 12, 34, 56, 78]));
        assert!(bytes.ends_with(&[0xae, 0x42, 0x60, 0x82]));
    }
}
