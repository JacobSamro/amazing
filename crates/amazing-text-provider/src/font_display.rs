use super::FontDisplay;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FontDisplayPeriods {
    pub block_ms: u32,
    /// `None` means the swap period does not expire.
    pub swap_ms: Option<u32>,
}

impl FontDisplay {
    pub fn periods(self) -> FontDisplayPeriods {
        match self {
            Self::Auto | Self::Block => FontDisplayPeriods { block_ms: 3000, swap_ms: None },
            Self::Swap => FontDisplayPeriods { block_ms: 100, swap_ms: None },
            Self::Fallback => FontDisplayPeriods { block_ms: 100, swap_ms: Some(3000) },
            Self::Optional => FontDisplayPeriods { block_ms: 0, swap_ms: Some(0) },
        }
    }
}
