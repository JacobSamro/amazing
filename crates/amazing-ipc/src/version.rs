//! Per-interface compatibility negotiated during process bootstrap.
use crate::codec::{self, Decoder, Encoder, Wire};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VersionRange {
    pub interface: u32,
    pub min: u16,
    pub max: u16,
}
impl Wire for VersionRange {
    fn encode(&self, e: &mut Encoder) -> Result<(), codec::Error> {
        self.interface.encode(e)?;
        self.min.encode(e)?;
        self.max.encode(e)
    }
    fn decode(d: &mut Decoder<'_>) -> Result<Self, codec::Error> {
        Ok(Self {
            interface: u32::decode(d)?,
            min: u16::decode(d)?,
            max: u16::decode(d)?,
        })
    }
}

/// Select the highest mutually supported version for each interface.
/// Unknown interfaces are optional; malformed or duplicate declarations fail closed.
pub fn negotiate(
    local: &[VersionRange],
    remote: &[VersionRange],
) -> Result<BTreeMap<u32, u16>, codec::Error> {
    fn validate(ranges: &[VersionRange]) -> Result<BTreeMap<u32, VersionRange>, codec::Error> {
        if ranges.len() > codec::MAX_ITEMS {
            return Err(codec::Error::Limit);
        }
        let mut seen = BTreeSet::new();
        let mut map = BTreeMap::new();
        for range in ranges {
            if range.min == 0 || range.min > range.max || !seen.insert(range.interface) {
                return Err(codec::Error::Invalid);
            }
            map.insert(range.interface, *range);
        }
        Ok(map)
    }
    let local = validate(local)?;
    let remote = validate(remote)?;
    let mut result = BTreeMap::new();
    for (id, ours) in local {
        if let Some(theirs) = remote.get(&id) {
            let min = ours.min.max(theirs.min);
            let max = ours.max.min(theirs.max);
            if min <= max {
                result.insert(id, max);
            }
        }
    }
    Ok(result)
}
