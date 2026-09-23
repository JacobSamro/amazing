//! Bounded, versioned wire encoding. All collection lengths and nesting are checked.
use std::collections::BTreeMap;
use std::fmt;

pub const VERSION: u16 = 1;
pub const MAX_FRAME: usize = 8 * 1024 * 1024;
pub const MAX_ITEMS: usize = 65_536;
pub const MAX_DEPTH: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Eof,
    Version,
    Limit,
    Invalid,
    Utf8,
    Trailing,
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for Error {}

pub struct Encoder {
    data: Vec<u8>,
    depth: usize,
}
impl Encoder {
    pub fn new() -> Self {
        Self {
            data: VERSION.to_le_bytes().to_vec(),
            depth: 0,
        }
    }
    pub fn raw() -> Self {
        Self {
            data: Vec::new(),
            depth: 0,
        }
    }
    pub fn bytes(&mut self, bytes: &[u8]) -> Result<(), Error> {
        if self
            .data
            .len()
            .checked_add(bytes.len())
            .filter(|&n| n <= MAX_FRAME)
            .is_none()
        {
            return Err(Error::Limit);
        }
        self.data.extend_from_slice(bytes);
        Ok(())
    }
    pub fn nested<T>(&mut self, f: impl FnOnce(&mut Self) -> Result<T, Error>) -> Result<T, Error> {
        if self.depth >= MAX_DEPTH {
            return Err(Error::Limit);
        }
        self.depth += 1;
        let r = f(self);
        self.depth -= 1;
        r
    }
    pub fn finish(self) -> Vec<u8> {
        self.data
    }
}
impl Default for Encoder {
    fn default() -> Self {
        Self::new()
    }
}
pub struct Decoder<'a> {
    data: &'a [u8],
    offset: usize,
    depth: usize,
}
impl<'a> Decoder<'a> {
    pub fn new(data: &'a [u8]) -> Result<Self, Error> {
        if data.len() > MAX_FRAME {
            return Err(Error::Limit);
        }
        let mut d = Self::raw(data);
        if u16::decode(&mut d)? != VERSION {
            return Err(Error::Version);
        }
        Ok(d)
    }
    pub fn raw(data: &'a [u8]) -> Self {
        Self {
            data,
            offset: 0,
            depth: 0,
        }
    }
    pub fn take(&mut self, n: usize) -> Result<&'a [u8], Error> {
        let end = self.offset.checked_add(n).ok_or(Error::Limit)?;
        let bytes = self.data.get(self.offset..end).ok_or(Error::Eof)?;
        self.offset = end;
        Ok(bytes)
    }
    pub fn remaining(&self) -> usize {
        self.data.len() - self.offset
    }
    pub fn finish(self) -> Result<(), Error> {
        if self.remaining() == 0 {
            Ok(())
        } else {
            Err(Error::Trailing)
        }
    }
    pub fn nested<T>(&mut self, f: impl FnOnce(&mut Self) -> Result<T, Error>) -> Result<T, Error> {
        if self.depth >= MAX_DEPTH {
            return Err(Error::Limit);
        }
        self.depth += 1;
        let r = f(self);
        self.depth -= 1;
        r
    }
}
pub trait Wire: Sized {
    fn encode(&self, e: &mut Encoder) -> Result<(), Error>;
    fn decode(d: &mut Decoder<'_>) -> Result<Self, Error>;
}
macro_rules! number { ($($t:ty),*) => {$ (impl Wire for $t {
    fn encode(&self,e:&mut Encoder)->Result<(),Error>{e.bytes(&self.to_le_bytes())}
    fn decode(d:&mut Decoder<'_>)->Result<Self,Error>{Ok(Self::from_le_bytes(d.take(std::mem::size_of::<Self>())?.try_into().map_err(|_|Error::Invalid)?))}
})*} }
number!(u16, u32, u64, u128, i32, i64);
impl Wire for u8 {
    fn encode(&self, e: &mut Encoder) -> Result<(), Error> {
        e.bytes(&[*self])
    }
    fn decode(d: &mut Decoder<'_>) -> Result<Self, Error> {
        Ok(d.take(1)?[0])
    }
}
impl Wire for bool {
    fn encode(&self, e: &mut Encoder) -> Result<(), Error> {
        (*self as u8).encode(e)
    }
    fn decode(d: &mut Decoder<'_>) -> Result<Self, Error> {
        match u8::decode(d)? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(Error::Invalid),
        }
    }
}
impl Wire for String {
    fn encode(&self, e: &mut Encoder) -> Result<(), Error> {
        self.as_bytes().to_vec().encode(e)
    }
    fn decode(d: &mut Decoder<'_>) -> Result<Self, Error> {
        String::from_utf8(Vec::<u8>::decode(d)?).map_err(|_| Error::Utf8)
    }
}
impl<T: Wire> Wire for Vec<T> {
    fn encode(&self, e: &mut Encoder) -> Result<(), Error> {
        if self.len() > MAX_ITEMS {
            return Err(Error::Limit);
        };
        (self.len() as u32).encode(e)?;
        e.nested(|e| {
            for item in self {
                item.encode(e)?
            }
            Ok(())
        })
    }
    fn decode(d: &mut Decoder<'_>) -> Result<Self, Error> {
        let n = u32::decode(d)? as usize;
        if n > MAX_ITEMS || n > d.remaining() {
            return Err(Error::Limit);
        }
        d.nested(|d| {
            let mut v = Vec::with_capacity(n);
            for _ in 0..n {
                v.push(T::decode(d)?)
            }
            Ok(v)
        })
    }
}
impl<T: Wire> Wire for Option<T> {
    fn encode(&self, e: &mut Encoder) -> Result<(), Error> {
        match self {
            Some(x) => {
                true.encode(e)?;
                e.nested(|e| x.encode(e))
            }
            None => false.encode(e),
        }
    }
    fn decode(d: &mut Decoder<'_>) -> Result<Self, Error> {
        if bool::decode(d)? {
            d.nested(|d| T::decode(d)).map(Some)
        } else {
            Ok(None)
        }
    }
}
impl<K: Wire + Ord, V: Wire> Wire for BTreeMap<K, V> {
    fn encode(&self, e: &mut Encoder) -> Result<(), Error> {
        if self.len() > MAX_ITEMS {
            return Err(Error::Limit);
        };
        (self.len() as u32).encode(e)?;
        e.nested(|e| {
            for (k, v) in self {
                k.encode(e)?;
                v.encode(e)?
            }
            Ok(())
        })
    }
    fn decode(d: &mut Decoder<'_>) -> Result<Self, Error> {
        let n = u32::decode(d)? as usize;
        if n > MAX_ITEMS || n > d.remaining() / 2 {
            return Err(Error::Limit);
        }
        d.nested(|d| {
            let mut m = BTreeMap::new();
            for _ in 0..n {
                if m.insert(K::decode(d)?, V::decode(d)?).is_some() {
                    return Err(Error::Invalid);
                }
            }
            Ok(m)
        })
    }
}
pub fn encode<T: Wire>(value: &T) -> Result<Vec<u8>, Error> {
    let mut e = Encoder::new();
    value.encode(&mut e)?;
    Ok(e.finish())
}
pub fn decode<T: Wire>(bytes: &[u8]) -> Result<T, Error> {
    let mut d = Decoder::new(bytes)?;
    let v = T::decode(&mut d)?;
    d.finish()?;
    Ok(v)
}
/// Declares a struct and its bounded wire implementation.
#[macro_export]
macro_rules! wire_struct { ($(#[$meta:meta])* $vis:vis struct $name:ident {$($field:ident : $ty:ty),* $(,)?}) => {
    $(#[$meta])* $vis struct $name {$(pub $field:$ty),*}
    impl $crate::codec::Wire for $name {
        fn encode(&self,e:&mut $crate::codec::Encoder)->Result<(),$crate::codec::Error>{e.nested(|e|{$($crate::codec::Wire::encode(&self.$field,e)?;)*Ok(())})}
        fn decode(d:&mut $crate::codec::Decoder<'_>)->Result<Self,$crate::codec::Error>{d.nested(|d|Ok(Self{$($field:<$ty as $crate::codec::Wire>::decode(d)?),*}))}
    }
}; }
impl Wire for () {
    fn encode(&self, _: &mut Encoder) -> Result<(), Error> {
        Ok(())
    }
    fn decode(_: &mut Decoder<'_>) -> Result<Self, Error> {
        Ok(())
    }
}
/// Declares an enum with single-field tuple variants; use `()` for a unit case.
#[macro_export]
macro_rules! wire_enum { ($(#[$meta:meta])* $vis:vis enum $name:ident {$($variant:ident ($ty:ty) = $tag:literal),* $(,)?}) => {
    $(#[$meta])* $vis enum $name {$($variant($ty)),*}
    impl $crate::codec::Wire for $name {
        fn encode(&self,e:&mut $crate::codec::Encoder)->Result<(),$crate::codec::Error>{e.nested(|e|{match self {$(Self::$variant(v) => {$crate::codec::Wire::encode(&($tag as u16),e)?;$crate::codec::Wire::encode(v,e)?;}),*}Ok(())})}
        fn decode(d:&mut $crate::codec::Decoder<'_>)->Result<Self,$crate::codec::Error>{d.nested(|d|{use $crate::codec::Wire;match u16::decode(d)? {$($tag=>Ok(Self::$variant(<$ty as Wire>::decode(d)?)),)*_=>Err($crate::codec::Error::Invalid)}})}
    }
}; }

/// A bounded byte blob for images and frames; unlike a generic sequence it may
/// use the whole frame budget. Decoding checks the length before allocating.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ByteBuf(pub Vec<u8>);
impl Wire for ByteBuf {
    fn encode(&self, e: &mut Encoder) -> Result<(), Error> {
        if self.0.len() > MAX_FRAME - 8 {
            return Err(Error::Limit);
        }
        (self.0.len() as u32).encode(e)?;
        e.bytes(&self.0)
    }
    fn decode(d: &mut Decoder<'_>) -> Result<Self, Error> {
        let n = u32::decode(d)? as usize;
        if n > MAX_FRAME - 8 {
            return Err(Error::Limit);
        }
        Ok(Self(d.take(n)?.to_vec()))
    }
}
