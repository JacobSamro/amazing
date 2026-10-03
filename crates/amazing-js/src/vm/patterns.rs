//! Elisions perform IteratorStep without IteratorValue, sharing completion state.
use super::*;

pub(super) fn skip(i: &mut Interpreter, state: &mut super::super::PatternIter) -> Result<(), Throw> {
    state.skip(i)
}
