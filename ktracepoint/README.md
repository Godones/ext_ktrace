# ktracepoint

A Rust tracepoint library for kernel scenarios, designed with goals similar to Linux tracepoints:

- Define events and fields with macros
- Manage tracepoints by unique event ID at runtime
- Support enable/disable, filter expressions, and callbacks
- Provide both raw event buffering and human-readable output
- no_std compatible

Repository: <https://github.com/Starry-OS/tracepoint>

## Core Capabilities

- Event definition: use `define_event_trace!` to generate event metadata, call functions, and register functions in one place
- Event management: `TracePointMap` indexed by tracepoint ID
- Event control: enable/disable, format/id/filter
- Filter expressions: compiled and evaluated against schema via `tp-lexer`
- Output pipeline: `TracePipeRaw` + `TraceEntryParser`

## Quick Start

### 1. Add dependencies

```toml
[dependencies]
ktracepoint = "*"
static-keys = "0.8"
```

### 2. Keep the `.tracepoint` section in your linker script

This library scans event metadata through `__start_tracepoint / __stop_tracepoint`.
Merge the content of `my_section.ld` into your linker script and ensure the `.tracepoint` section is kept with `KEEP`.

### 3. Implement `KernelTraceOps`

You need to provide:

- `time_now`
- `cpu_id`
- `current_pid`
- `trace_pipe_push_raw_record`
- `trace_cmdline_push`
- `write_kernel_text`
- tracepoint state registry hooks: `read_tracepoint_state`, `write_tracepoint_state`

`write_kernel_text` is used for static key instruction patching.
The state registry hooks let your OS choose its own synchronization strategy for callbacks and filters.

### Callback restrictions

`read_tracepoint_state` may hold a read-side lock while the tracing fast path executes callbacks. If your implementation uses a non-reentrant lock such as `RwLock`, callbacks must not:

- register or unregister tracepoint callbacks
- update tracepoint filters
- call other APIs that require `write_tracepoint_state`
- recursively trigger tracepoints backed by the same state registry

Violating these rules may deadlock. Hosts that implement `read_tracepoint_state` with RCU, snapshots, or another non-blocking read-side mechanism may provide weaker restrictions.

### 4. Define and invoke events

```rust
use ktracepoint::{define_event_trace, KernelTraceOps};

define_event_trace!(
    TEST,
    TP_kops(Kops),
    TP_system(tracepoint_test),
    TP_PROTO(a: u32, b: u32),
    TP_STRUCT__entry { a: u32, b: u32 },
    TP_fast_assign { a: a, b: b },
    TP_ident(__entry),
    TP_printk(format_args!("a={}, b={}", __entry.a, __entry.b))
);

// Generated functions: trace_TEST / register_trace_TEST / unregister_trace_TEST
trace_TEST(1, 2);
```

Note: `TP_STRUCT__entry` participates in byte layout. Ensure your field layout matches expectations (think in C layout terms).

### 5. Initialize the manager

```rust
use ktracepoint::global_init_events;

static_keys::global_init();
let mut tracepoints = global_init_events::<Kops>()?;
```

### 6. Enable, filter, and consume output

```rust
use ktracepoint::{TraceFilterFile, TracePointEnableFile, TracePointFormatFile, TracePointIdFile};

let event_id = 0;
let event = tracepoints.get_mut(&event_id).unwrap();
TracePointEnableFile::new(event.trace_point()).write('1');
event.trace_point().enable_event();

let mut filter = TraceFilterFile::new();
filter.write(event, "a > 8 && b > 5").unwrap();

// Read format and ID
let fmt = TracePointFormatFile::new(event.trace_point()).read();
let id = TracePointIdFile::new(event.trace_point()).read();
```

## Run the example

```bash
cd examples
cargo run --example usage
```

Example code is in `examples/usage.rs`, covering:

- Event definition and triggering
- Event enabling and filtering
- Registering event/raw callbacks
- Reading `TracePipeRaw` snapshots and parsing them into text

## Main Public Types

- `KernelTraceOps`
- `TracePoint` / `ExtTracePoint` / `TracePointMap`
- `TracePipeRaw` / `TracePipeSnapshot` / `TracePipeOps`
- `TraceCmdLineCache` / `TraceEntryParser`

## Reference Projects

- DragonOS: <https://github.com/DragonOS-Community/DragonOS/blob/master/kernel/src/debug/tracing/mod.rs>
- StarryOS: <https://github.com/Starry-OS/StarryOS>
