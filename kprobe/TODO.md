# kprobe TODO

This list tracks the highest-value improvements to discuss and solve first.

## Priority Items

1. Instruction validation and relocation
   - Reject or relocate instructions that cannot safely execute out of line.
   - Cover PC-relative/RIP-relative instructions, branches, calls, and architecture-specific corner cases.

2. Concurrency and lifetime safety
   - Define locking rules for `ProbeManager` and `ProbePointList`.
   - Handle register/unregister racing with trap handlers and probes currently executing on other CPUs.
   - Clarify text patching synchronization and instruction cache requirements.

3. Fault and handler control-flow semantics
   - Wire `fault_handler` into the trap path.
   - Give handlers a return value so pre handlers can request normal single-step, skip single-step, or abort.

4. Uprobe identity and per-process state
   - Avoid keying user probes only by virtual address.
   - Include pid/mm or file mapping identity in probe-point lookup.
   - Revisit dynamic user executable memory ownership for multi-thread and multi-process cases.

5. Kretprobe builder callback and observability
   - Preserve callbacks configured through `RetprobeBuilder::with_event_callback`.
   - Expose `nmissed` and instance-pool status.
   - Document task-local retprobe stack ordering and unregister behavior.
