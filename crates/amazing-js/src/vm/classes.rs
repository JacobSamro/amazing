use super::*;

// Snapshots and records are strong Rc roots, owned by this invocation. There
// is no independent tracing heap or cycle-reclamation claim here.
#[derive(Clone)]
pub(super) struct ScopeSnapshot {
    pub env: EnvHandle,
    strict: bool,
    strict_depth: usize,
}

pub(super) struct ClassState {
    pub evaluation: super::super::ClassEvaluation,
    pub scope_depth: usize,
}

pub(super) fn snapshot(env: &EnvHandle, strict: bool, stack: &[bool]) -> ScopeSnapshot {
    ScopeSnapshot { env: env.clone(), strict, strict_depth: stack.len() }
}

pub(super) fn restore(saved: ScopeSnapshot, env: &mut EnvHandle, strict: &mut bool,
    stack: &mut Vec<bool>, i: &mut Interpreter) {
    *env = saved.env;
    *strict = saved.strict;
    stack.truncate(saved.strict_depth);
    i.strict_eval_context = *strict;
}

pub(super) fn discard_exited(states: &mut [Option<ClassState>], depth: usize) {
    for state in states {
        if state.as_ref().is_some_and(|state| state.scope_depth > depth) { *state = None; }
    }
}

pub(super) fn state(states: &mut [Option<ClassState>], slot: usize) -> Result<&mut ClassState, Throw> {
    states.get_mut(slot).and_then(Option::as_mut)
        .ok_or_else(|| Throw::type_error("invalid VM class evaluation lifetime"))
}

pub(super) fn heritage(_i: &mut Interpreter, code: &ClassHeritageCode,
    registers: &[JsValue], references: &mut [Option<Reference>]) -> Result<super::super::ClassHeritageOperand, Throw> {
    use super::super::ClassHeritageOperand;
    Ok(match code {
        ClassHeritageCode::Value(source) => ClassHeritageOperand::Value(registers[*source].clone()),
        ClassHeritageCode::Reference(slot, fallback) => match references[*slot].take() {
            Some(reference) => ClassHeritageOperand::Reference(reference),
            None => ClassHeritageOperand::Value(registers[*fallback].clone()),
        },
        ClassHeritageCode::Super(base, key, receiver) => ClassHeritageOperand::Reference(
            Reference::SuperProperty(registers[*base].clone(), key.resolve(registers)?, registers[*receiver].clone())),
    })
}
