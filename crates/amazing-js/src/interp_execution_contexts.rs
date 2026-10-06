// Actual executing context identities; suspended coroutine state keeps scalar IDs only.
impl Interpreter {
    /// Whether this agent currently has no executing JavaScript context.
    /// Parked generator/async contexts are excluded until they are resumed.
    pub fn binding_execution_context_stack_is_empty(&self) -> bool {
        self.execution_contexts.is_empty()
    }

    pub(crate) fn with_execution_context<T>(&mut self, identity: Option<u64>,
        operation: impl FnOnce(&mut Interpreter) -> T) -> T {
        let identity = identity.unwrap_or_else(CallFrame::next_context_id);
        // An initial VM body executes the callable context already installed by
        // [[Call]]; a resumed body installs that same identity afresh.
        if self.execution_contexts.last() == Some(&identity) { return operation(self); }
        let depth = self.execution_contexts.len();
        self.execution_contexts.push(identity);
        let result = operation(self);
        self.execution_contexts.truncate(depth);
        result
    }
}
