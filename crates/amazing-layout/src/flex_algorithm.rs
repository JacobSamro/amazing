//! Pure CSS Flexbox main-size resolution.
//!
//! Box measurement and physical placement stay in `flex`; this module owns
//! the iterative freeze/distribute/clamp algorithm from Flexbox §9.7.

#[derive(Clone, Copy, Debug)]
pub(crate) struct FlexConstraint {
    pub base: f32,
    pub hypothetical: f32,
    pub min: f32,
    pub max: f32,
    pub grow: f32,
    pub shrink: f32,
    pub scaled_shrink: f32,
    /// Padding, border, and non-auto margins on the main axis.
    pub outer: f32,
}

pub(crate) fn resolve_flexible_lengths(
    available: f32,
    constraints: &[FlexConstraint],
) -> Vec<f32> {
    let growing = constraints
        .iter()
        .map(|item| item.hypothetical + item.outer)
        .sum::<f32>()
        < available;
    let mut target = constraints.iter().map(|item| item.base).collect::<Vec<_>>();
    let mut frozen = vec![false; constraints.len()];

    for (index, item) in constraints.iter().enumerate() {
        let factor = if growing { item.grow } else { item.shrink };
        let wrong_side = (growing && item.base > item.hypothetical)
            || (!growing && item.base < item.hypothetical);
        if factor == 0.0 || wrong_side {
            target[index] = item.hypothetical;
            frozen[index] = true;
        }
    }

    let initial_free = remaining_free_space(available, constraints, &target, &frozen);
    for _ in 0..=constraints.len() {
        if frozen.iter().all(|value| *value) {
            break;
        }
        let mut free = remaining_free_space(available, constraints, &target, &frozen);
        let flex_factor_sum = constraints
            .iter()
            .enumerate()
            .filter(|(index, _)| !frozen[*index])
            .map(|(_, item)| if growing { item.grow } else { item.shrink })
            .sum::<f32>();
        let distribution_sum = constraints
            .iter()
            .enumerate()
            .filter(|(index, _)| !frozen[*index])
            .map(|(_, item)| if growing { item.grow } else { item.scaled_shrink })
            .sum::<f32>();
        if flex_factor_sum < 1.0 {
            let scaled = initial_free * flex_factor_sum;
            if scaled.abs() < free.abs() {
                free = scaled;
            }
        }
        if distribution_sum <= 0.0 {
            break;
        }

        let mut violations = vec![0.0; constraints.len()];
        let mut total_violation = 0.0;
        let infinite_factors = constraints
            .iter()
            .enumerate()
            .filter(|(index, item)| {
                !frozen[*index]
                    && if growing {
                        item.grow.is_infinite()
                    } else {
                        item.scaled_shrink.is_infinite()
                    }
            })
            .count();
        for (index, item) in constraints.iter().enumerate() {
            if frozen[index] {
                continue;
            }
            let factor = if growing { item.grow } else { item.scaled_shrink };
            let ratio = if distribution_sum.is_infinite() {
                if factor.is_infinite() {
                    1.0 / infinite_factors.max(1) as f32
                } else {
                    0.0
                }
            } else {
                factor / distribution_sum
            };
            let unclamped = item.base + free * ratio;
            target[index] = unclamped.max(0.0).clamp(item.min, item.max);
            violations[index] = target[index] - unclamped;
            total_violation += violations[index];
        }

        let mut froze = false;
        for index in 0..constraints.len() {
            let matching = total_violation == 0.0
                || (total_violation > 0.0 && violations[index] > 0.0)
                || (total_violation < 0.0 && violations[index] < 0.0);
            if !frozen[index] && matching {
                frozen[index] = true;
                froze = true;
            }
        }
        if !froze {
            break;
        }
    }
    target
}

fn remaining_free_space(
    available: f32,
    constraints: &[FlexConstraint],
    target: &[f32],
    frozen: &[bool],
) -> f32 {
    available
        - constraints
            .iter()
            .enumerate()
            .map(|(index, item)| {
                item.outer
                    + if frozen[index] {
                        target[index]
                    } else {
                        item.base
                    }
            })
            .sum::<f32>()
}
