//! Basic eBPF library providing essential functionalities for eBPF programs.
//!! This library includes support for BPF maps, helper functions, and program
//! loading mechanisms, making it easier to develop and run eBPF programs in a
//! kernel-like environment.
//!

#![deny(missing_docs)]
#![no_std]
#![feature(c_variadic)]
#![allow(unused)]
extern crate alloc;
use alloc::string::String;

use map::UnifiedMap;
pub mod helper;
pub mod linux_bpf;
pub mod map;
pub mod perf;
mod preprocessor;
pub mod prog;
pub mod raw_tracepoint;

pub use preprocessor::EBPFPreProcessor;

type Result<T> = axerrno::LinuxResult<T>;
type BpfError = axerrno::LinuxError;

/// PollWaiter trait for maps that support polling.
pub trait PollWaker: Send + Sync {
    /// Wake up any waiters on the map.
    fn wake_up(&self);
}

/// The KernelAuxiliaryOps trait provides auxiliary operations which should
/// be implemented by the kernel or a kernel-like environment.
pub trait KernelAuxiliaryOps: Send + Sync + 'static {
    /// Get a unified map from a pointer.
    fn get_unified_map_from_ptr<F, R>(ptr: *const u8, func: F) -> Result<R>
    where
        F: FnOnce(&mut UnifiedMap) -> Result<R>;
    /// Get a unified map from a file descriptor.
    fn get_unified_map_from_fd<F, R>(map_fd: u32, func: F) -> Result<R>
    where
        F: FnOnce(&mut UnifiedMap) -> Result<R>;
    /// Get a unified map pointer from a file descriptor.
    fn get_unified_map_ptr_from_fd(map_fd: u32) -> Result<*const u8>;
    /// Copy data from a user space pointer to a kernel space buffer.
    fn copy_from_user(src: *const u8, size: usize, dst: &mut [u8]) -> Result<()>;
    /// Copy data from a kernel space buffer to a user space pointer.
    fn copy_to_user(dest: *mut u8, size: usize, src: &[u8]) -> Result<()>;
    /// Get the current CPU ID.
    fn current_cpu_id() -> u32;
    /// Output some data to a perf buf
    fn perf_event_output(
        ctx: *mut core::ffi::c_void,
        fd: u32,
        flags: u32,
        data: &[u8],
    ) -> Result<()>;
    /// Read a string from a user space pointer.
    fn string_from_user_cstr(ptr: *const u8) -> Result<String>;
    /// For ebpf print helper functions
    fn ebpf_write_str(str: &str) -> Result<()>;
    /// For ebpf ktime helper functions
    fn ebpf_time_ns() -> Result<u64>;

    /// Allocate pages in kernel space. Return the physical address of the allocated page.
    fn alloc_page() -> Result<usize>;
    /// Free the allocated page in kernel space.
    fn free_page(phys_addr: usize);
    /// Create a virtual mapping for the given physical addresses. Return the virtual address.
    fn vmap(phys_addrs: &[usize]) -> Result<usize>;
    /// Unmap the given virtual address.
    fn unmap(vaddr: usize);
}

struct DummyAuxImpl;
impl KernelAuxiliaryOps for DummyAuxImpl {
    fn get_unified_map_from_ptr<F, R>(_ptr: *const u8, _func: F) -> Result<R>
    where
        F: FnOnce(&mut UnifiedMap) -> Result<R>,
    {
        Err(BpfError::EPERM)
    }

    fn get_unified_map_from_fd<F, R>(_map_fd: u32, _func: F) -> Result<R>
    where
        F: FnOnce(&mut UnifiedMap) -> Result<R>,
    {
        Err(BpfError::EPERM)
    }

    fn get_unified_map_ptr_from_fd(_map_fd: u32) -> Result<*const u8> {
        Err(BpfError::EPERM)
    }

    fn copy_from_user(_src: *const u8, _size: usize, _dst: &mut [u8]) -> Result<()> {
        Err(BpfError::EPERM)
    }

    fn copy_to_user(_dest: *mut u8, _size: usize, _src: &[u8]) -> Result<()> {
        Err(BpfError::EPERM)
    }

    fn current_cpu_id() -> u32 {
        0
    }

    fn perf_event_output(
        _ctx: *mut core::ffi::c_void,
        _fd: u32,
        _flags: u32,
        _data: &[u8],
    ) -> Result<()> {
        Err(BpfError::EPERM)
    }

    fn string_from_user_cstr(_ptr: *const u8) -> Result<String> {
        Err(BpfError::EPERM)
    }

    fn ebpf_write_str(_str: &str) -> Result<()> {
        Err(BpfError::EPERM)
    }

    fn ebpf_time_ns() -> Result<u64> {
        Err(BpfError::EPERM)
    }

    fn alloc_page() -> Result<usize> {
        Err(BpfError::EPERM)
    }

    fn free_page(_phys_addr: usize) {}

    fn vmap(_phys_addrs: &[usize]) -> Result<usize> {
        Err(BpfError::EPERM)
    }

    fn unmap(_vaddr: usize) {}
}
