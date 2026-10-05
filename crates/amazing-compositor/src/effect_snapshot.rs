//! Versioned effect extension. V1 transports must reject effect-bearing trees
//! rather than silently dropping their operations. IPC adoption is separate.
use super::*;
pub const EFFECT_SNAPSHOT_VERSION: u16 = 2;
#[derive(Debug, Clone, PartialEq)]
pub struct EffectsFrameSnapshot {
    pub version: u16,
    pub frame: CompositorFrameSnapshot,
    pub operations: Vec<(EffectId, EffectOperations)>,
}
impl CompositorFrame {
    pub fn to_effects_snapshot(&self) -> EffectsFrameSnapshot {
        let mut operations = self
            .trees
            .effect_operations
            .iter()
            .map(|(&id, op)| (id, op.clone()))
            .collect::<Vec<_>>();
        operations.sort_by_key(|(id, _)| id.0);
        let mut frame = self.to_snapshot();
        frame.trees.version = PROPERTY_TREES_SNAPSHOT_VERSION;
        EffectsFrameSnapshot {
            version: EFFECT_SNAPSHOT_VERSION,
            frame,
            operations,
        }
    }
    pub fn from_effects_snapshot(snapshot: EffectsFrameSnapshot) -> Result<Self, SnapshotError> {
        if snapshot.version != EFFECT_SNAPSHOT_VERSION {
            return Err(SnapshotError::UnsupportedVersion);
        }
        if snapshot.operations.len() > 1024 {
            return Err(SnapshotError::LimitExceeded);
        }
        let mut result = Self::from_snapshot(snapshot.frame)?;
        let mut ids = HashSet::new();
        let mut mask_bytes = 0usize;
        for (id, op) in snapshot.operations {
            if !ids.insert(id) {
                return Err(SnapshotError::DuplicateId);
            }
            if let Some(mask) = &op.mask {
                mask_bytes = mask_bytes
                    .checked_add(mask.image.pixels.len())
                    .ok_or(SnapshotError::LimitExceeded)?;
                if mask_bytes > 64 * 1024 * 1024 {
                    return Err(SnapshotError::LimitExceeded);
                }
            }
            result
                .trees
                .set_effect_operations(id, op)
                .map_err(|_| SnapshotError::InvalidTree)?;
        }
        Ok(result)
    }
}
