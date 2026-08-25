//! Pure flexible-track resolution from CSS Grid §11.7.
//!
//! Track parsing, intrinsic contribution collection, and box placement stay
//! in `grid`; this module owns the restart loop for finding the used `fr`.

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
