use super::{Error, MAX_FRAME};
use std::io::{Read, Write};
#[derive(Debug)]
pub(super) struct Frame {
    pub kind: u8,
    pub flags: u8,
    pub stream: u32,
    pub payload: Vec<u8>,
}
impl Frame {
    pub fn read(io: &mut impl Read) -> Result<Self, Error> {
        let mut head = [0; 9];
        io.read_exact(&mut head).map_err(Error::Io)?;
        let len = ((head[0] as usize) << 16) | ((head[1] as usize) << 8) | head[2] as usize;
        if len > MAX_FRAME {
            return Err(Error::FrameSize);
        }
        let kind = head[3];
        let flags = head[4];
        let stream = u32::from_be_bytes(head[5..9].try_into().unwrap()) & 0x7fffffff;
        // Validate fixed-size/control frames before allocating their payload.
        if matches!(kind, 0 | 1 | 2 | 3 | 5 | 9) && stream == 0
            || matches!(kind, 4 | 6 | 7) && stream != 0
        {
            return Err(Error::Protocol);
        }
        if (kind == 2 && len != 5)
            || (matches!(kind, 3 | 8) && len != 4)
            || (kind == 6 && len != 8)
            || (kind == 7 && len < 8)
            || (kind == 4 && (len % 6 != 0 || flags & 1 != 0 && len != 0))
        {
            return Err(Error::FrameSize);
        }
        if kind == 5 {
            return Err(Error::Protocol);
        } // ENABLE_PUSH=0 from preface.
        let mut payload = Vec::new();
        payload.try_reserve_exact(len).map_err(|_| Error::Limit)?;
        payload.resize(len, 0);
        io.read_exact(&mut payload).map_err(Error::Io)?;
        Ok(Self {
            kind,
            flags,
            stream,
            payload,
        })
    }
}
pub(super) fn write(
    io: &mut impl Write,
    kind: u8,
    flags: u8,
    stream: u32,
    data: &[u8],
) -> Result<(), Error> {
    if data.len() > MAX_FRAME {
        return Err(Error::FrameSize);
    }
    let len = data.len() as u32;
    let mut head = [0; 9];
    head[..3].copy_from_slice(&len.to_be_bytes()[1..]);
    head[3] = kind;
    head[4] = flags;
    head[5..].copy_from_slice(&stream.to_be_bytes());
    io.write_all(&head).map_err(Error::Io)?;
    io.write_all(data).map_err(Error::Io)?;
    Ok(())
}
pub(super) fn number(data: &[u8]) -> u32 {
    u32::from_be_bytes(data[..4].try_into().unwrap())
}
pub(super) fn unpad(data: &[u8], padded: bool) -> Result<&[u8], Error> {
    if !padded {
        return Ok(data);
    }
    let padding = usize::from(*data.first().ok_or(Error::Protocol)?);
    let end = data.len().checked_sub(padding).ok_or(Error::Protocol)?;
    if end < 1 {
        return Err(Error::Protocol);
    }
    Ok(&data[1..end])
}
