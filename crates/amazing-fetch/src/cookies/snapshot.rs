//! Bounded versioned snapshot of persistent cookies. Session cookies stay in memory.
use super::{valid_host, Cookie, CookieError, CookieStore, SameSite};
use crate::{RegistrableDomain, Site};

const MAGIC: &[u8; 8] = b"AMCK0001";
const MAX_BYTES: usize = 16 * 1024 * 1024;

fn write_bytes(out: &mut Vec<u8>, bytes: &[u8]) -> Result<(), CookieError> {
    let len = u16::try_from(bytes.len()).map_err(|_| CookieError::Snapshot)?;
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(bytes);
    Ok(())
}
struct Reader<'a> {
    input: &'a [u8],
    at: usize,
}
impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], CookieError> {
        let end = self
            .at
            .checked_add(n)
            .filter(|end| *end <= self.input.len())
            .ok_or(CookieError::Snapshot)?;
        let bytes = &self.input[self.at..end];
        self.at = end;
        Ok(bytes)
    }
    fn u8(&mut self) -> Result<u8, CookieError> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<usize, CookieError> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()) as usize)
    }
    fn i64(&mut self) -> Result<i64, CookieError> {
        Ok(i64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn bytes(&mut self) -> Result<Vec<u8>, CookieError> {
        let len = self.u16()?;
        Ok(self.take(len)?.to_vec())
    }
    fn text(&mut self) -> Result<String, CookieError> {
        String::from_utf8(self.bytes()?).map_err(|_| CookieError::Snapshot)
    }
}

impl<P: RegistrableDomain> CookieStore<P> {
    pub fn snapshot(&mut self, now: i64) -> Result<Vec<u8>, CookieError> {
        self.expire(now);
        let cookies: Vec<_> = self
            .cookies
            .iter()
            .filter(|cookie| {
                cookie.persistent() && !matches!(cookie.partition_key, Some(Site::Opaque(_)))
            })
            .collect();
        let mut out = MAGIC.to_vec();
        out.extend_from_slice(&(cookies.len() as u32).to_le_bytes());
        for cookie in cookies {
            write_bytes(&mut out, &cookie.name)?;
            write_bytes(&mut out, &cookie.value)?;
            write_bytes(&mut out, cookie.domain.as_bytes())?;
            write_bytes(&mut out, &cookie.path)?;
            out.extend_from_slice(&cookie.expires.unwrap().to_le_bytes());
            out.extend_from_slice(&cookie.created.to_le_bytes());
            out.extend_from_slice(&cookie.accessed.to_le_bytes());
            out.push(
                u8::from(cookie.host_only)
                    | (u8::from(cookie.secure) << 1)
                    | (u8::from(cookie.http_only) << 2),
            );
            out.push(match cookie.same_site {
                SameSite::Default => 0,
                SameSite::Strict => 1,
                SameSite::Lax => 2,
                SameSite::None => 3,
            });
            match &cookie.partition_key {
                None => out.push(0),
                Some(Site::Tuple { scheme, domain }) => {
                    out.push(1);
                    write_bytes(&mut out, scheme.as_bytes())?;
                    write_bytes(&mut out, domain.as_bytes())?;
                }
                Some(Site::Opaque(_)) => unreachable!(),
            }
            if out.len() > MAX_BYTES {
                return Err(CookieError::Snapshot);
            }
        }
        Ok(out)
    }

    pub fn restore_snapshot(&mut self, input: &[u8], now: i64) -> Result<(), CookieError> {
        if input.len() > MAX_BYTES {
            return Err(CookieError::Snapshot);
        }
        let mut reader = Reader { input, at: 0 };
        if reader.take(8)? != MAGIC {
            return Err(CookieError::Snapshot);
        }
        let count = u32::from_le_bytes(reader.take(4)?.try_into().unwrap()) as usize;
        if count > self.limits.total {
            return Err(CookieError::Snapshot);
        }
        let mut cookies = Vec::with_capacity(count);
        for _ in 0..count {
            let name = reader.bytes()?;
            let value = reader.bytes()?;
            let domain = reader.text()?;
            let path = reader.bytes()?;
            let expires = reader.i64()?;
            let created = reader.i64()?;
            let accessed = reader.i64()?;
            let flags = reader.u8()?;
            let same_site = match reader.u8()? {
                0 => SameSite::Default,
                1 => SameSite::Strict,
                2 => SameSite::Lax,
                3 => SameSite::None,
                _ => return Err(CookieError::Snapshot),
            };
            let partition_key = match reader.u8()? {
                0 => None,
                1 => Some(Site::Tuple {
                    scheme: reader.text()?,
                    domain: reader.text()?,
                }),
                _ => return Err(CookieError::Snapshot),
            };
            if expires <= now {
                continue;
            }
            let host_only = flags & 1 != 0;
            let secure = flags & 2 != 0;
            let http_only = flags & 4 != 0;
            if flags & !7 != 0
                || !valid_host(&domain, &self.domains)
                || (!host_only && self.domains.is_public_suffix(&domain) != Some(false))
                || !path.starts_with(b"/")
                || path.len() > 1024
                || name.len() + value.len() > 4096
                || (same_site == SameSite::None && !secure)
                || (partition_key.is_some() && !secure)
            {
                return Err(CookieError::Snapshot);
            }
            cookies.push(Cookie {
                name,
                value,
                domain,
                path,
                expires: Some(expires),
                created,
                accessed,
                host_only,
                secure,
                http_only,
                same_site,
                partition_key,
            });
        }
        if reader.at != input.len() {
            return Err(CookieError::Snapshot);
        }
        self.cookies = cookies;
        self.evict();
        Ok(())
    }
}
