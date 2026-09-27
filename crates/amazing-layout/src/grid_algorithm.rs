//! Pure flexible-track resolution from CSS Grid §11.7.
//!
//! Track parsing, intrinsic contribution collection, and box placement stay
//! in `grid`; this module owns the restart loop for finding the used `fr`.

// Portions ported from DioxusLabs/taffy (src/compute/grid/track_sizing.rs,
// `distribute_space_up_to_limits`) @ e661b35980d9644ad8f1df64e183d27e5884533b, MIT.
/// CSS Grid 2 s11.5.1 "Distributing Extra Space Across Spanned Tracks":
/// distribute `space` across the tracks flagged in `affected`, growing each
/// track's `bases` entry up to its `limits` entry before any of them
/// exceeds it, then redistributing the freed-up remainder among the tracks
/// that still have room -- rather than a single equal share hard-clamped
/// per track independently, which loses the excess a saturated track
/// couldn't absorb instead of handing it to its still-growable neighbors.
///
/// Simplified from taffy's version: always equal-proportion (taffy weights
/// by flex factor when every affected track is itself flexible, to match a
/// Chrome quirk for items that span flexible tracks; amazing-layout does
/// not distinguish that case here, matching this function's own prior
/// behavior and the spec's plain-language default of equal shares). Returns
/// whatever space could not be placed within any track's limit -- callers
/// that need the "distribute the remainder beyond limits too" follow-up
/// step (taffy's own next step) do not yet exist in this crate; documented
/// as a follow-up rather than ported now, since it is exercised only when
/// a whole span is already saturated at its limits.
pub(crate) fn distribute_up_to_limits(
    space: f32,
    bases: &mut [f32],
    affected: &[bool],
    limits: &[f32],
) -> f32 {
    const THRESHOLD: f32 = 0.01;
    let mut remaining = space;
    let mut increase = vec![0.0f32; bases.len()];
    let max_iterations = bases.len() + 1;
    for _ in 0..max_iterations {
        if remaining <= THRESHOLD {
            break;
        }
        let growable: Vec<usize> = (0..bases.len())
            .filter(|&i| affected[i] && bases[i] + increase[i] < limits[i])
            .collect();
        if growable.is_empty() {
            break;
        }
        let min_room = growable
            .iter()
            .map(|&i| limits[i] - bases[i] - increase[i])
            .fold(f32::INFINITY, f32::min);
        let share = (remaining / growable.len() as f32).min(min_room);
        if !(share > 0.0) {
            break;
        }
        for &i in &growable {
            increase[i] += share;
            remaining -= share;
        }
    }
    for (i, value) in increase.into_iter().enumerate() {
        bases[i] += value;
    }
    remaining
}

pub(crate) fn maximize_tracks(
    available: f32,
    gaps: f32,
    bases: &[f32],
    limits: &[Option<f32>],
) -> Vec<f32> {
    if bases.len() != limits.len() {
        return bases.to_vec();
    }
    let mut sizes = bases.to_vec();
    let mut free = (available - gaps - sizes.iter().sum::<f32>()).max(0.0);
    let mut growable = limits
        .iter()
        .enumerate()
        .filter(|(index, limit)| limit.is_some_and(|limit| limit > sizes[*index]))
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    while free > 0.0 && !growable.is_empty() {
        let share = free / growable.len() as f32;
        let mut distributed = 0.0;
        growable.retain(|index| {
            let room = limits[*index].unwrap_or(sizes[*index]) - sizes[*index];
            let growth = room.min(share).max(0.0);
            sizes[*index] += growth;
            distributed += growth;
            growth + f32::EPSILON < room
        });
        if distributed <= 0.0 {
            break;
        }
        free -= distributed;
    }
    sizes
}

pub(crate) fn resolve_flexible_tracks(
    available: f32,
    gaps: f32,
    bases: &[f32],
    factors: &[f32],
) -> Vec<f32> {
    if bases.len() != factors.len() {
        return bases.to_vec();
    }
    let mut sizes = bases.to_vec();
    let mut flexible = factors
        .iter()
        .map(|factor| factor.is_finite() && *factor > 0.0)
        .collect::<Vec<_>>();
    for _ in 0..=bases.len() {
        let inflexible = sizes
            .iter()
            .enumerate()
            .filter(|(index, _)| !flexible[*index])
            .map(|(_, size)| *size)
            .sum::<f32>();
        let factor_sum = factors
            .iter()
            .enumerate()
            .filter(|(index, _)| flexible[*index])
            .map(|(_, factor)| *factor)
            .sum::<f32>()
            .max(1.0);
        let fraction = ((available - gaps - inflexible) / factor_sum).max(0.0);
        let mut restarted = false;
        for index in 0..bases.len() {
            if flexible[index] && bases[index] > fraction * factors[index] {
                flexible[index] = false;
                sizes[index] = bases[index];
                restarted = true;
            }
        }
        if restarted {
            continue;
        }
        for index in 0..sizes.len() {
            if flexible[index] {
                sizes[index] = fraction * factors[index];
            }
        }
        break;
    }
    sizes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distribute_up_to_limits_caps_a_saturated_track_and_hands_the_rest_onward() {
        // Two affected tracks share 100px of extra space; the first can only
        // take 20 more before its limit, so the second should get the
        // remaining 80, not an equal 50/50 split that would overshoot the
        // first track's limit.
        let mut bases = [10.0, 10.0];
        let remaining = distribute_up_to_limits(100.0, &mut bases, &[true, true], &[30.0, f32::INFINITY]);
        assert_eq!(remaining, 0.0);
        assert!((bases[0] - 30.0).abs() < 0.001, "first track should freeze at its limit: {bases:?}");
        assert!((bases[1] - 90.0).abs() < 0.001, "second track should absorb the rest: {bases:?}");
    }

    #[test]
    fn distribute_up_to_limits_ignores_unaffected_tracks() {
        let mut bases = [10.0, 10.0, 10.0];
        let remaining =
            distribute_up_to_limits(30.0, &mut bases, &[true, false, true], &[f32::INFINITY, f32::INFINITY, f32::INFINITY]);
        assert_eq!(remaining, 0.0);
        assert!((bases[0] - 25.0).abs() < 0.001);
        assert_eq!(bases[1], 10.0, "unaffected track must not grow");
        assert!((bases[2] - 25.0).abs() < 0.001);
    }

    #[test]
    fn distribute_up_to_limits_returns_the_unplaceable_remainder_when_every_affected_track_is_saturated() {
        let mut bases = [20.0, 20.0];
        let remaining = distribute_up_to_limits(50.0, &mut bases, &[true, true], &[20.0, 20.0]);
        assert_eq!(bases, [20.0, 20.0], "already-saturated tracks must not move");
        assert!((remaining - 50.0).abs() < 0.001, "nothing could be placed: {remaining}");
    }

    #[test]
    fn maximize_tracks_grows_an_infinite_limit_track_to_fill_leftover_space() {
        // grid-template-columns: 40px auto -- bases start at [40, 20], and
        // with 200px available there is 140px of leftover space (200 -
        // (40+20)). The `auto` column (no explicit numeric max, so
        // `TrackSize::maximize_limit()` reports it as unlimited here, not
        // excluded from this step) should absorb all of it, matching every
        // browser's rendering of that pattern.
        let sizes = maximize_tracks(200.0, 0.0, &[40.0, 20.0], &[Some(40.0), Some(f32::INFINITY)]);
        assert_eq!(sizes[0], 40.0, "the fixed track must not grow past its own size");
        assert!((sizes[1] - 160.0).abs() < 0.001, "the unlimited track should take all leftover space: {sizes:?}");
    }
}
