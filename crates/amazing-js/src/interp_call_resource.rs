/// Physical synchronous call-entry policy. This complements the unchanged
/// logical MAX_CALL_DEPTH; Proxy forwarding need not create JS call frames.
///
/// Supported embeddings supply an ordinary OS-thread stack of at least 2 MiB.
/// Native/HostNative callbacks must stay within the configured headroom before
/// reentering the engine, and must not switch fibers or run on signal stacks.
/// Those callbacks cannot be made stack-safe merely by checking their caller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SynchronousCallPolicy { reserve_bytes: usize }
impl SynchronousCallPolicy {
    // Candidate headroom, not a qualification claim: the minimum-stack debug
    // and release matrix must validate controlled entry/error/unwind frames.
    pub const MINIMUM_RESERVE_BYTES: usize = 256 * 1024;
    pub fn new(reserve_bytes: usize) -> Option<Self> {
        (reserve_bytes >= Self::MINIMUM_RESERVE_BYTES).then_some(Self { reserve_bytes })
    }
    pub fn reserve_bytes(self) -> usize { self.reserve_bytes }
}
impl Default for SynchronousCallPolicy {
    fn default() -> Self { Self { reserve_bytes: Self::MINIMUM_RESERVE_BYTES } }
}

impl Interpreter {
    /// Increase physical admission headroom for an embedding with larger host
    /// callback frames. Changing this does not alter JS execution-context depth.
    pub fn set_synchronous_call_policy(&mut self, policy: SynchronousCallPolicy) {
        self.synchronous_call_policy = policy;
    }
    pub fn synchronous_call_policy(&self) -> SynchronousCallPolicy { self.synchronous_call_policy }

    #[inline(never)]
    fn admit_synchronous_call(&self) -> Result<(), Throw> {
        match crate::call_stack::remaining() {
            Some(bytes) if bytes >= self.synchronous_call_policy.reserve_bytes => Ok(()),
            _ => {
                // Do not ask authored globals or accessors to construct an
                // exhaustion error. Use this active realm's saved intrinsic.
                let error = Throw::range_error("synchronous call stack resource exhausted");
                if let JsValue::Object(object) = &error.0 {
                    object.borrow_mut().proto = self.intrinsic_constructors
                        .get("RangeError").map(|(_, prototype)| prototype.clone());
                }
                Err(error)
            },
        }
    }
}
