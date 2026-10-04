# Native tasks and admission ordering

Recorded base: fd9183a86c85f1dbcb3d6b2a62676354e9357052.

Trusted coalesced native Selection work enters the interpreter scheduling FIFO after the actual lower mutation borrow ends and before script resumes. Its token retains the actual binding arena and a checked, never-reused slot. Payloads remain driver-owned through failed admission; successful admission is acknowledged exactly, and dispatch releases custody before author callbacks. Scheduler destruction retires its own pending lower flags even when wrappers retain the binding state. Earlier abrupt completion identity takes precedence over scheduling errors. Disabled external scheduling preserves the lower legacy path.

The lower event loop chooses the oldest admitted first-runnable task across task sources, preserving FIFO within each source and inactive-global skipping. Checked ordinal renumbering preserves all queue order. This is the project scheduling policy, not a normative timer priority. Existing root-global timer/Selection routing remains a documented limitation; direct HTML parser mutations require their owner’s emission checkpoints.

Source is the previously qualified 02011389245c6fe926f6d2ecfed6a693bb336049 to 9a739428a92283c5211ae6708d585b8d0894a186 engine delta, squashed without dependency history. A fresh fixed-base Linux gate is required for this package.

The original foreign-callee poisoned-TypeError expectation found a missing prerequisite on the recorded base: native-error normalization consulted mutable globals. The qualified saved per-realm intrinsic prototype branch is restored with its existing Error/prototype/name eligibility guards. The original failing 7de source and raw remain preserved; no assertion is weakened. Authored null-prototype Error provenance is a separate existing boundary.

Fresh fixed-base gate: workspace4325→4345,17 skipped each,4 doctests each; core88892820→92820,shadow2775676→5676,syntax722706→2706. Exact names/multiplicities/passing files retained; no extra partial gains. The baseline engine and all corpus profiles match the completed flat-tree baseline; only current base tooling differs. Candidate ran on default pool-dom; raw remains Boat.
