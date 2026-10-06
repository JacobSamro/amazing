// Portions ported from boa-dev/boa (core/engine/src/builtins/atomics/mod.rs)
// @ ef40c63283e1bb73f371d93372e9e95217a36945, MIT.
// Adapted to Amazing's native single-agent byte store: all author conversions
// finish before the exclusive backing-store transaction. No GC/agent ABI change.

#[derive(Clone, Copy, PartialEq, Eq)]
enum BinaryAtomicOperation {
    Load,
    Store,
    Exchange,
    CompareExchange,
    Add,
    Sub,
    And,
    Or,
    Xor,
}

impl BinaryAtomicOperation {
    fn named(name: &str) -> Option<Self> {
        Some(match name {
            "load" => Self::Load,
            "store" => Self::Store,
            "exchange" => Self::Exchange,
            "compareExchange" => Self::CompareExchange,
            "add" => Self::Add,
            "sub" => Self::Sub,
            "and" => Self::And,
            "or" => Self::Or,
            "xor" => Self::Xor,
            _ => return None,
        })
    }
}

fn binary_atomic_operand(
    i: &mut Interpreter,
    kind: BinaryKind,
    value: &JsValue,
) -> Result<(JsValue, u64), Throw> {
    if kind.big() {
        let converted = i.to_bigint(value)?;
        let bits = converted.low_u64();
        Ok((Interpreter::bigint_value(converted), bits))
    } else {
        let integer = to_integer_or_infinity(i, value)?;
        // Mathematical zero converts to +0, including truncated negative
        // fractions. Keep the unwrapped result for Atomics.store's return.
        let integer = if integer == 0.0 { 0.0 } else { integer };
        let bytes = binary_encode_number(integer, kind, true);
        let mut raw = [0u8; 8];
        raw[..kind.size()].copy_from_slice(&bytes);
        Ok((JsValue::num(integer), u64::from_le_bytes(raw)))
    }
}

fn binary_atomic_decoded(kind: BinaryKind, bits: u64) -> JsValue {
    if kind.big() {
        Interpreter::bigint_value(if kind == BinaryKind::BigInt64 {
            BinaryBigInt::from_i64(bits as i64)
        } else {
            BinaryBigInt::from_u64(bits)
        })
    } else {
        JsValue::num(binary_number(
            &bits.to_le_bytes()[..kind.size()],
            kind,
            true,
        ))
    }
}

fn binary_atomic_integer(
    i: &mut Interpreter,
    operation: BinaryAtomicOperation,
    args: &[JsValue],
) -> Result<JsValue, Throw> {
    let target = args.first().unwrap_or(&JsValue::Undefined);
    let access = if operation == BinaryAtomicOperation::Load {
        BinaryTypedAccess::Read
    } else {
        BinaryTypedAccess::Write
    };
    let (view, kind) = binary_validate_typed_result(target, access, None, None)?;
    if !kind.integer() || kind == BinaryKind::Uint8Clamped {
        return Err(Throw::type_error("invalid Atomics TypedArray"));
    }
    // This length belongs to the original witness, before ToIndex can run
    // author code. Growth cannot admit a formerly out-of-range request.
    let length = binary_length(&view);
    let index = binary_index(i, args.get(1).unwrap_or(&JsValue::Undefined))?;
    if index >= length {
        return Err(Throw::range_error("Atomics index outside TypedArray"));
    }
    let byte_index = view
        .offset
        .checked_add(
            index
                .checked_mul(kind.size())
                .ok_or_else(|| Throw::range_error("atomic offset"))?,
        )
        .ok_or_else(|| Throw::range_error("atomic offset"))?;
    let (converted, input) = if operation == BinaryAtomicOperation::Load {
        (JsValue::Undefined, 0)
    } else {
        binary_atomic_operand(i, kind, args.get(2).unwrap_or(&JsValue::Undefined))?
    };
    // Replacement conversion is required even when comparison will fail, or
    // the expected conversion detached/shrank the source. Revalidate afterward.
    let replacement = if operation == BinaryAtomicOperation::CompareExchange {
        binary_atomic_operand(i, kind, args.get(3).unwrap_or(&JsValue::Undefined))?.1
    } else {
        0
    };
    if binary_view_out_of_bounds(&view) {
        return Err(Throw::type_error("out-of-bounds atomic view"));
    }
    let size = kind.size();
    let end = byte_index
        .checked_add(size)
        .ok_or_else(|| Throw::range_error("atomic offset"))?;
    // Revalidate the actual complete word too: a tracking RAB may shrink to
    // a partial element. Never resurrect bytes outside its current data block.
    let mut buffer = view.buffer.borrow_mut();
    if byte_index >= buffer.bytes.len() || end > buffer.bytes.len() {
        return Err(Throw::range_error("atomic index after conversion"));
    }
    // No author code runs while this native borrow is held. Every genuine
    // same-agent SharedArrayBuffer alias refers to this same block; the read,
    // comparison and write are one indivisible native transaction. Cross-agent
    // sharing remains the separate agent/backing-store producer's responsibility.
    let mut raw = [0u8; 8];
    raw[..size].copy_from_slice(&buffer.bytes[byte_index..end]);
    let old = u64::from_le_bytes(raw);
    let mask = if size == 8 {
        u64::MAX
    } else {
        (1u64 << (size * 8)) - 1
    };
    let next = match operation {
        BinaryAtomicOperation::Load => None,
        BinaryAtomicOperation::Store | BinaryAtomicOperation::Exchange => Some(input),
        BinaryAtomicOperation::CompareExchange => (old == input & mask).then_some(replacement),
        BinaryAtomicOperation::Add => Some(old.wrapping_add(input)),
        BinaryAtomicOperation::Sub => Some(old.wrapping_sub(input)),
        BinaryAtomicOperation::And => Some(old & input),
        BinaryAtomicOperation::Or => Some(old | input),
        BinaryAtomicOperation::Xor => Some(old ^ input),
    };
    if let Some(next) = next {
        let bytes = (next & mask).to_le_bytes();
        buffer.bytes[byte_index..end].copy_from_slice(&bytes[..size]);
    }
    drop(buffer);
    Ok(if operation == BinaryAtomicOperation::Store {
        converted
    } else {
        binary_atomic_decoded(kind, old)
    })
}

include!("runtime_atomics_integer_tests.rs");
