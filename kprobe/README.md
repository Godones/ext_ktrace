# kprobe

[中文版](./README.md)

A `no_std` Rust probing library for kernels and low-level runtimes. It dynamically patches instructions at target addresses, triggers handlers on breakpoint hits, and resumes execution through single-step or equivalent architecture-specific flows.

This repository is better understood as probe infrastructure rather than a ready-to-run tool. The crate handles probe installation/removal, breakpoint and debug address management, and architecture-specific register/instruction logic. The embedding environment must still provide memory access, page permission changes, executable memory allocation, and task-local retprobe instance management.

## Features

- `kprobe` for function or instruction entry probing
- `kretprobe` for function return probing
- `uprobe` for user-space address probing
- Event callbacks and custom `ProbeData`
- Shared `ProbePoint` reuse for multiple probes on the same address
- Multi-architecture `PtRegs` and breakpoint/single-step handling

## Supported Architectures

- `x86_64`
- `riscv64`
- `loongarch64`
- `aarch64`

## Core Abstractions

### `ProbeBuilder<F>`

Used to describe a regular probe:

- Probe location: `with_symbol_addr()` + `with_offset()`
- Handlers: `with_pre_handler()` / `with_post_handler()` / `with_fault_handler()`
- Custom data: `with_data()`
- Enabled state: `with_enable(true)`
- User-space mode: `with_user_mode(pid)`

### `KretprobeBuilder<L>`

Used to describe a return probe:

- `with_entry_handler()` runs on function entry
- `with_ret_handler()` runs on function return
- `with_data()` prepares per-instance user data
- `maxactive` controls the instance pool size

### `ProbeManager<L, F>`

Maintains two internal tables:

- `break_list` for breakpoint-hit addresses
- `debug_list` for post-single-step debug addresses

Your trap or exception path is expected to pass the current `PtRegs` into the manager-facing handlers when a breakpoint or debug exception occurs.

### `ProbePointList<F>`

The global index of installed `ProbePoint`s. Multiple probes attached to the same address share one patched instruction, and the original instruction is only restored when the last probe is removed.

### `KprobeAuxiliaryOps`

This trait is the required host integration layer. It provides:

- Kernel/user memory copying
- Writable permission updates for target addresses
- Executable memory allocation and release
- Push/pop of task-local retprobe instances

Without this trait implementation, the crate cannot run on its own.

## Minimal Integration Flow

1. Implement `KprobeAuxiliaryOps` for your kernel or runtime.
2. Choose a `RawMutex` implementation for `ProbeManager` and callback storage.
3. Call these handlers from your trap/exception path:
   - `kprobe_handler_from_break()` / `kprobe_handler_from_debug()`
   - `uprobe_handler_from_break()` / `uprobe_handler_from_debug()`
4. Register probes with `register_kprobe()`, `register_kretprobe()`, and `register_uprobe()`.
5. Remove them with the matching `unregister_*()` APIs when no longer needed.

## Example

This is a minimal integration sketch. `MyAuxOps` and `MyRawMutex` must be provided by your environment.

```rust,ignore
use alloc::collections::BTreeMap;
use kprobe::{
    ProbeBuilder, ProbeManager, ProbePointList, PtRegs, ProbeData, KprobeAuxiliaryOps,
    register_kprobe, unregister_kprobe,
};

#[derive(Debug)]
struct MyAuxOps;

impl KprobeAuxiliaryOps for MyAuxOps {
    /* implemented by the embedding environment */
    # fn copy_memory(_: *const u8, _: *mut u8, _: usize, _: Option<i32>) {}
    # fn set_writeable_for_address<T: FnOnce(*mut u8)>(_: usize, _: usize, _: Option<i32>, _: T) {}
    # fn alloc_kernel_exec_memory() -> *mut u8 { core::ptr::null_mut() }
    # fn free_kernel_exec_memory(_: *mut u8) {}
    # fn alloc_user_exec_memory<T: FnOnce(*mut u8)>(_: Option<i32>, _: T) -> *mut u8 { core::ptr::null_mut() }
    # fn free_user_exec_memory(_: Option<i32>, _: *mut u8) {}
    # fn insert_kretprobe_instance_to_task(_: kprobe::RetprobeInstance) {}
    # fn pop_kretprobe_instance_from_task() -> kprobe::RetprobeInstance { unimplemented!() }
}

fn on_enter(_data: &dyn ProbeData, regs: &mut PtRegs) {
    let _ = regs.first_ret_value();
}

type MyRawMutex = YourRawMutex;

fn demo(target_addr: usize) -> Result<(), kprobe::ProbeInstallError> {
    let mut manager = ProbeManager::<MyRawMutex, MyAuxOps>::new();
    let mut points: ProbePointList<MyAuxOps> = BTreeMap::new();

    let probe = register_kprobe(
        &mut manager,
        &mut points,
        ProbeBuilder::<MyAuxOps>::new()
            .with_symbol_addr(target_addr)
            .with_pre_handler(on_enter)
            .with_enable(true),
    )?;

    unregister_kprobe(&mut manager, &mut points, probe);
    Ok(())
}
```

## Trap Integration

The crate does not take over your exception flow. You are expected to call it from your own trap or breakpoint handlers, for example:

```rust,ignore
use kprobe::{PtRegs, kprobe_handler_from_break, kprobe_handler_from_debug};

fn handle_break(regs: &mut PtRegs, manager: &mut ProbeManager<MyRawMutex, MyAuxOps>) -> bool {
    kprobe_handler_from_break(manager, regs).is_some()
}

fn handle_debug(regs: &mut PtRegs, manager: &mut ProbeManager<MyRawMutex, MyAuxOps>) -> bool {
    kprobe_handler_from_debug(manager, regs).is_some()
}
```

`uprobe` is integrated the same way, except registration must include `with_user_mode(pid)`, and your exception path should call `uprobe_handler_from_break()` / `uprobe_handler_from_debug()`.

## Reading Return Values

Return probes usually read return registers through `PtRegs`:

- First return value: `PtRegs::first_ret_value()`
- Second return value: `PtRegs::second_ret_value()`

For Rust return types such as `Option<T>` and `Result<T, E>`, values may span multiple registers depending on ABI and compiler layout. See [docs/kretprobe.md](./docs/kretprobe.md) for more background.

## Project Layout

- `src/lib.rs`: shared exports and `register_kretprobe()`
- `src/kprobe/`: regular probe registration and dispatch
- `src/uprobe/`: user-space probe registration and dispatch
- `src/arch/`: per-architecture `ProbePoint`, `PtRegs`, instruction patching, and retprobe trampoline code
- `src/manager.rs`: `ProbeManager` and `ProbePointList`
- `docs/`: implementation notes and background material

## Notes

- This is a `#![no_std]` crate intended for kernels or low-level runtimes.
- The examples are integration sketches, not copy-paste complete programs.
- The crate currently exports the `Uretprobe` type alias, but does not expose standalone `register_uretprobe()` / `unregister_uretprobe()` APIs.
- Some architectures rely on naked functions and inline assembly, so it is best to build with the toolchain expected by the repository.

## References

- [docs/kretprobe.md](./docs/kretprobe.md)
- [docs/How uprobe ebpf work.md](./docs/How%20uprobe%20ebpf%20work.md)
- [docs/arm64.md](./docs/arm64.md)
