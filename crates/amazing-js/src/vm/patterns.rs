//! Elisions perform IteratorStep, without the IteratorValue read required by
//! actual elements. Reuse the shared pattern iterator's completion state.
use super::*;

pub(super) fn skip(i: &mut Interpreter, state: &mut super::super::PatternIter) -> Result<(), Throw> {
    match state {
        super::super::PatternIter::Fast { index, len, .. } => { if *index < *len { *index += 1; } Ok(()) }
        super::super::PatternIter::Protocol { iterator, next, done } => {
            if *done { return Ok(()); }
            let result = (|| {
                let result = i.call_value(next, iterator.clone(), Vec::new())?;
                if !result.is_object() { return Err(Throw::type_error("iterator result is not an object")); }
                *done = i.get_prop(&result, "done")?.to_boolean();
                Ok(())
            })();
            if result.is_err() { *done = true; }
            result
        }
    }
}
