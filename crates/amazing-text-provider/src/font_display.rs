use super::{FontDisplay, FontFaceDescriptor};
use amazing_text::{FontKey, GlyphVisibility, TextError};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FontDisplayPeriods {
    pub block_ms: u32,
    /// `None` means the swap period does not expire.
    pub swap_ms: Option<u32>,
}

impl FontDisplay {
    pub fn periods(self) -> FontDisplayPeriods {
        match self {
            Self::Auto | Self::Block => FontDisplayPeriods {
                block_ms: 3000,
                swap_ms: None,
            },
            Self::Swap => FontDisplayPeriods {
                block_ms: 0,
                swap_ms: None,
            },
            Self::Fallback => FontDisplayPeriods {
                block_ms: 100,
                swap_ms: Some(3000),
            },
            Self::Optional => FontDisplayPeriods {
                block_ms: 100,
                swap_ms: Some(0),
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontDisplayPhase {
    NotUsed,
    Block,
    Swap,
    Loaded,
    Failed,
    Expired,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FontDisplaySnapshot {
    pub phase: FontDisplayPhase,
    pub visibility: GlyphVisibility,
    pub first_use: Option<Duration>,
    pub next_change: Option<Duration>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontDisplayMetrics {
    Fallback,
    Downloaded(FontKey),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FontDisplayFaceState {
    pub display: FontDisplaySnapshot,
    pub metrics: FontDisplayMetrics,
}
#[derive(Debug, Default)]
struct TimelineState {
    first_use: Option<Duration>,
    loaded_at: Option<Duration>,
    failed_at: Option<Duration>,
}
/// One authentic face's timeline, retained by the loader across provider rebuilds.
/// The descriptor is immutable; attaching it to a different face is rejected.
#[derive(Debug, Clone)]
pub struct FontDisplayTimeline {
    pub(super) descriptor: Arc<FontFaceDescriptor>,
    state: Arc<Mutex<TimelineState>>,
}
impl FontDisplayTimeline {
    pub fn new(descriptor: FontFaceDescriptor) -> Self {
        Self {
            descriptor: Arc::new(descriptor),
            state: Arc::new(Mutex::new(TimelineState::default())),
        }
    }
    pub fn descriptor(&self) -> &FontFaceDescriptor {
        &self.descriptor
    }
    pub(super) fn same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.state, &other.state)
    }
    pub(super) fn first_use(&self, now: Duration) -> Result<(), TextError> {
        let mut s = self.state.lock().map_err(|_| TextError::InvalidStyle)?;
        s.first_use.get_or_insert(now);
        Ok(())
    }
    pub(super) fn loaded(&self, now: Duration) -> Result<(), TextError> {
        let mut s = self.state.lock().map_err(|_| TextError::InvalidStyle)?;
        if s.failed_at.is_none() {
            s.loaded_at.get_or_insert(now);
        }
        Ok(())
    }
    pub(super) fn failed(&self, now: Duration) -> Result<(), TextError> {
        let mut s = self.state.lock().map_err(|_| TextError::InvalidStyle)?;
        if s.loaded_at.is_none() {
            s.failed_at.get_or_insert(now);
        }
        Ok(())
    }
    pub fn snapshot(&self, now: Duration) -> Result<FontDisplaySnapshot, TextError> {
        let s = self.state.lock().map_err(|_| TextError::InvalidStyle)?;
        let periods = self.descriptor.display.periods();
        let block_end = s
            .first_use
            .map(|start| {
                start
                    .checked_add(Duration::from_millis(periods.block_ms.into()))
                    .ok_or(TextError::ResourceLimit)
            })
            .transpose()?;
        let swap_end = match (block_end, periods.swap_ms) {
            (Some(start), Some(ms)) => Some(
                start
                    .checked_add(Duration::from_millis(ms.into()))
                    .ok_or(TextError::ResourceLimit)?,
            ),
            _ => None,
        };
        let failed = s.failed_at.is_some_and(|t| t <= now);
        let loaded = s.loaded_at.filter(|t| *t <= now);
        let phase = if failed {
            FontDisplayPhase::Failed
        } else if loaded.is_some_and(|loaded| swap_end.is_none_or(|end| loaded < end)) {
            FontDisplayPhase::Loaded
        } else if swap_end.is_some_and(|end| now >= end) {
            FontDisplayPhase::Expired
        } else if block_end.is_none_or(|end| s.first_use.is_some_and(|t| now < t) || now < end) {
            if s.first_use.is_none_or(|t| now < t) {
                FontDisplayPhase::NotUsed
            } else {
                FontDisplayPhase::Block
            }
        } else {
            FontDisplayPhase::Swap
        };
        let next_change = match phase {
            FontDisplayPhase::Block => block_end,
            FontDisplayPhase::Swap => swap_end,
            _ => None,
        };
        Ok(FontDisplaySnapshot {
            phase,
            visibility: if phase == FontDisplayPhase::Block {
                GlyphVisibility::InvisibleFallback
            } else {
                GlyphVisibility::Visible
            },
            first_use: s.first_use,
            next_change,
        })
    }
}
#[derive(Debug, Clone)]
pub(super) struct DisplayFace {
    pub descriptor: FontFaceDescriptor,
    pub timeline: FontDisplayTimeline,
    pub key: Option<FontKey>,
    pub pending: bool,
    pub order: usize,
}
