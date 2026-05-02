# ddebug

`ddebug` is a `no_std` Rust implementation of a Linux-style dynamic debug facility.

Current scope:

- `pr_debug!` callsites only
- Per-callsite metadata collected in the `__dyndbg` linker section
- Static-key guarded hot path
- `dynamic_debug_init()` returns a Linux-like `ControlFile`
- Supported selectors: `file`, `func`, `module`, `line`, `format`
- Supported flags:
  - `p`: print enable
  - `m`: module prefix
  - `f`: file prefix
  - `l`: line prefix
  - `t`: thread id prefix

## Quick start

```rust
use ddebug::{DebugOps, dynamic_debug_init, pr_debug, pr_debug_fn};

struct MyOps;

impl DebugOps for MyOps {
    fn write_kernel_text(addr: *mut u8, data: &[u8]) {
        // platform specific text patching
    }

    fn emit(line: &str) {
        // send to printk / serial / console
    }

    fn thread_id() -> u64 {
        0
    }
}


fn demo(order: usize) {
    pr_debug!(MyOps, "alloc page order={}", order);
}

#[ddebug::named]
fn named_demo(order: usize) {
    pr_debug_fn!(MyOps, "alloc page order={}", order);
}

fn enable_demo() {
    static_keys::global_init();
    let mut ctl = dynamic_debug_init::<MyOps>();
    assert_eq!(ctl.procfs_path(), "/proc/dynamic_debug/control");
    ctl.write("func named_demo =pmfl").unwrap();
    let listing = ctl.read().unwrap();
}
```

## Notes

- `dynamic_debug_init::<K>()` scans the `__dyndbg` section and returns a `ControlFile<K>`.
- `pr_debug!` now names the ops type explicitly, for example `pr_debug!(MyOps, "x={}", x)`.
- Use `#[ddebug::named]` with `pr_debug_fn!` when you want a callsite to participate in `func` matching.
- `pr_debug!` requires the first argument to be a string literal so the callsite can store the format string.
- `format` selector uses substring matching by default, and supports `*` / `?` wildcards.
- `file` and `module` also support `*` / `?`.
- `func` matches the captured function name exactly by default, and also supports `*` / `?`.
- Call `static_keys::global_init()` from your runtime before enabling any `pr_debug!` sites.
- Toggling `p` patches branch sites through the `write_kernel_text` hook, so apply control changes in a serialized context.
