// Portions ported from lucid-softworks/lumen (crates/lumen/src/builtins/typedarray.rs)
// @ f6c9860d44fa52249cfd647122edea184d32c863, MIT.
// Copyright (c) 2026 Lucid Softworks limited. See THIRD_PARTY_NOTICES.md.
// Whole stable TypedArray sorting algorithm adapted to Amazing's values/calls.

fn binary_typed_stable_sort(
    i: &mut Interpreter,
    values: &mut [JsValue],
    compare: &JsValue,
) -> Result<(), Throw> {
    binary_stable_merge_sort(values, &mut |a, b| {
        if !matches!(compare, JsValue::Undefined) {
            let result = i.call_value(compare, JsValue::Undefined, vec![a.clone(), b.clone()])?;
            let number = i.to_number(&result)?;
            return Ok(number.partial_cmp(&0.0).unwrap_or(std::cmp::Ordering::Equal));
        }
        if let (JsValue::BigInt(a), JsValue::BigInt(b)) = (a, b) {
            return Ok(a.cmp(b));
        }
        let a = a.to_number()?;
        let b = b.to_number()?;
        Ok(if a.is_nan() {
            if b.is_nan() { std::cmp::Ordering::Equal } else { std::cmp::Ordering::Greater }
        } else if b.is_nan() {
            std::cmp::Ordering::Less
        } else {
            a.total_cmp(&b)
        })
    })
}

// The upstream fallible merge algorithm accepts even inconsistent comparators:
// it always terminates, preserves a permutation, and stops JS calls on a throw.
// Only the captured list is sorted; the receiver is written after success.
fn binary_stable_merge_sort<T: Clone, E>(
    values: &mut [T],
    compare: &mut impl FnMut(&T, &T) -> Result<std::cmp::Ordering, E>,
) -> Result<(), E> {
    if values.len() < 2 { return Ok(()); }
    let middle = values.len() / 2;
    let mut left = values[..middle].to_vec();
    let mut right = values[middle..].to_vec();
    binary_stable_merge_sort(&mut left, compare)?;
    binary_stable_merge_sort(&mut right, compare)?;
    let (mut li, mut ri, mut out) = (0, 0, 0);
    while li < left.len() && ri < right.len() {
        if compare(&left[li], &right[ri])? != std::cmp::Ordering::Greater {
            values[out] = left[li].clone();
            li += 1;
        } else {
            values[out] = right[ri].clone();
            ri += 1;
        }
        out += 1;
    }
    while li < left.len() {
        values[out] = left[li].clone(); li += 1; out += 1;
    }
    while ri < right.len() {
        values[out] = right[ri].clone(); ri += 1; out += 1;
    }
    Ok(())
}

#[cfg(test)]
#[path = "builtins_binary_sort_tests.rs"]
mod binary_sort_tests;
