use super::Error;
use std::io::{Read, Write};
pub(super) struct Header {
    pub fin: bool,
    pub opcode: u8,
    pub len: usize,
}
impl Header {
    pub fn read(io: &mut impl Read, max: usize) -> Result<Self, Error> {
        let mut b = [0; 2];
        io.read_exact(&mut b).map_err(|e| {
            if e.kind() == std::io::ErrorKind::UnexpectedEof {
                Error::AbnormalClosure
            } else {
                Error::Io(e)
            }
        })?;
        let fin = b[0] & 128 != 0;
        let opcode = b[0] & 15;
        let short = b[1] & 127;
        if b[0] & 0x70 != 0
            || b[1] & 128 != 0
            || !matches!(opcode, 0 | 1 | 2 | 8 | 9 | 10)
            || opcode >= 8 && (!fin || short > 125)
        {
            return Err(Error::Protocol);
        }
        let len = match short {
            126 => {
                let mut n = [0; 2];
                io.read_exact(&mut n)?;
                let n = u16::from_be_bytes(n) as u64;
                if n < 126 {
                    return Err(Error::Protocol);
                }
                n
            }
            127 => {
                let mut n = [0; 8];
                io.read_exact(&mut n)?;
                let n = u64::from_be_bytes(n);
                if n < 65536 || n >> 63 != 0 {
                    return Err(Error::Protocol);
                }
                n
            }
            n => u64::from(n),
        };
        if len > max as u64 {
            return Err(Error::Limit);
        }
        Ok(Self {
            fin,
            opcode,
            len: len as usize,
        })
    }
}
pub(super) fn write(
    io: &mut impl Write,
    random: fn(&mut [u8]) -> Result<(), Error>,
    opcode: u8,
    fin: bool,
    bytes: &[u8],
) -> Result<(), Error> {
    let mut mask = [0; 4];
    random(&mut mask)?;
    let mut header = [0; 14];
    header[0] = opcode | if fin { 128 } else { 0 };
    let offset = if bytes.len() < 126 {
        header[1] = 128 | bytes.len() as u8;
        2
    } else if bytes.len() <= 65535 {
        header[1] = 128 | 126;
        header[2..4].copy_from_slice(&(bytes.len() as u16).to_be_bytes());
        4
    } else {
        header[1] = 128 | 127;
        header[2..10].copy_from_slice(&(bytes.len() as u64).to_be_bytes());
        10
    };
    header[offset..offset + 4].copy_from_slice(&mask);
    io.write_all(&header[..offset + 4])?;
    let mut scratch = [0; 4096];
    for (i, chunk) in bytes.chunks(scratch.len()).enumerate() {
        for (j, &b) in chunk.iter().enumerate() {
            scratch[j] = b ^ mask[(i * 4096 + j) % 4];
        }
        io.write_all(&scratch[..chunk.len()])?;
    }
    Ok(())
}
// RFC6455 defined codes plus registered 1012-1014; no extension-defined codes.
pub(super) fn valid_code(code: u16, from_server: bool) -> bool {
    matches!(code,1000..=1003|1007..=1014|3000..=4999) && !(from_server && code == 1010)
}
