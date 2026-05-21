use alloc::{boxed::Box, collections::btree_map::BTreeMap, string::String, sync::Arc};
use core::{
    any::Any,
    fmt::Debug,
    sync::atomic::{AtomicBool, Ordering},
};

use lock_api::{Mutex, RawMutex};

pub mod retprobe;

cfg_if::cfg_if! {
    if #[cfg(target_arch = "x86_64")] {
        mod x86;
        pub use x86::*;
        /// The probe point structure for the current architecture.
        pub type ProbePoint<F> = X86ProbePoint<F>;
    } else if #[cfg(target_arch = "riscv64")] {
        mod rv64;
        pub use rv64::*;
        /// The probe point structure for the current architecture.
        pub type ProbePoint<F> = Rv64ProbePoint<F>;
    } else if #[cfg(target_arch = "loongarch64")] {
        mod loongarch64;
        pub use loongarch64::*;
        /// The probe point structure for the current architecture.
        pub type ProbePoint<F> = LA64ProbePoint<F>;
    } else if #[cfg(target_arch = "aarch64")] {
        mod aarch64;
        pub use aarch64::*;
        /// The probe point structure for the current architecture.
        pub type ProbePoint<F> = AArch64ProbePoint<F>;
    }
    else {
        compile_error!("Unsupported architecture");
    }
}

// mod rv64;
// pub use rv64::*;
// /// The probe point structure for the current architecture.
// pub type ProbePoint<F> = Rv64ProbePoint<F>;

/// The operations available for kprobes.
pub(crate) trait KprobeOps: Send {
    /// The address of the instruction that program should return to.
    fn return_address(&self) -> usize;
    /// The address of the instruction that saved the original instruction.
    ///
    /// Usually, the original instruction at the probe point is saved in an array.
    /// Depending on the architecture, necessary instructions may be filled in after
    /// the saved instruction. For example, x86 architecture supports single-step execution,
    /// while other architectures usually do not. Therefore, we use the break exception to
    /// simulate it, so a breakpoint instruction will be filled in.
    fn single_step_address(&self) -> usize;
    /// The address of the instruction that caused the single step exception.
    fn debug_address(&self) -> usize;
    /// The address of the instruction that caused the break exception.
    ///
    /// It is usually equal to the address of the instruction that used to set the probe point.
    fn break_address(&self) -> usize;

    /// Get the dynamic user pointer.
    fn dynamic_user_ptr(&self) -> usize;
    /// Set the dynamic user pointer and return the new debug address.
    fn set_dynamic_user_ptr(&self, ptr: usize) -> usize;
    /// Get the length of the original instruction.
    fn original_instruction_len(&self) -> usize;
    /// Get the length of the executable instruction slot.
    fn instruction_slot_len(&self) -> usize;
    /// Get the pid of the user process, if applicable.
    fn pid(&self) -> Option<i32>;
}

/// Errors that can occur while installing a probe.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProbeInstallError {
    /// The target address is invalid for the current architecture.
    InvalidAddress,
    /// The instruction at the target address could not be decoded.
    DecodeFailed,
    /// The decoded instruction is not supported by the current implementation.
    UnsupportedInstruction,
    /// The decoded instruction is unsafe to execute out of line.
    UnsafeInstruction,
    /// The instruction could not be relocated into the executable slot.
    RelocationFailed,
    /// The breakpoint patch could not be applied.
    PatchFailed,
}

/// The auxiliary operations required for kprobes.
pub trait KprobeAuxiliaryOps: Send + Debug {
    /// Copy memory from source to destination. If `user_pid` is `Some(pid)`, it indicates that the `src` is in user space.
    /// The `dst` is always in kernel space.
    fn copy_memory(src: *const u8, dst: *mut u8, len: usize, user_pid: Option<i32>);
    /// Enable or disable write permission for the specified address.
    fn set_writeable_for_address<F: FnOnce(*mut u8)>(
        address: usize,
        len: usize,
        user_pid: Option<i32>,
        action: F,
    );
    /// Allocate executable memory(one page)
    fn alloc_kernel_exec_memory() -> *mut u8;
    /// Deallocate executable memory(one page)
    fn free_kernel_exec_memory(ptr: *mut u8);
    /// Allocate user executable memory(one page)
    fn alloc_user_exec_memory<F: FnOnce(*mut u8)>(pid: Option<i32>, action: F) -> *mut u8;
    /// Deallocate user executable memory(one page)
    fn free_user_exec_memory(pid: Option<i32>, ptr: *mut u8);
    /// Insert a kretprobe instance to the current task
    fn insert_kretprobe_instance_to_task(instance: retprobe::RetprobeInstance);
    /// Pop a kretprobe instance from the current task
    fn pop_kretprobe_instance_from_task() -> retprobe::RetprobeInstance;
}

#[derive(Debug)]
enum ExecMemType<F: KprobeAuxiliaryOps> {
    User(usize),
    Kernel(usize),
    _Marker(core::marker::PhantomData<F>),
}

impl<F: KprobeAuxiliaryOps> ExecMemType<F> {
    /// Get the pointer of the executable memory.
    pub fn as_ptr(&self) -> *mut u8 {
        match self {
            ExecMemType::User(ptr) => *ptr as *mut u8,
            ExecMemType::Kernel(ptr) => *ptr as *mut u8,
            ExecMemType::_Marker(_) => core::ptr::null_mut(),
        }
    }
}

impl<F: KprobeAuxiliaryOps> Drop for ExecMemType<F> {
    fn drop(&mut self) {
        match self {
            ExecMemType::User(ptr) => unsafe {
                let _ = Box::from_raw(*ptr as *mut [u8; 4096]);
            },
            ExecMemType::Kernel(ptr) => {
                F::free_kernel_exec_memory(*ptr as *mut u8);
            }
            ExecMemType::_Marker(_) => {}
        }
    }
}

fn alloc_exec_memory<F: KprobeAuxiliaryOps>(user_pid: Option<i32>) -> ExecMemType<F> {
    if user_pid.is_some() {
        ExecMemType::User(Box::into_raw(Box::new([0u8; 4096])) as usize)
    } else {
        ExecMemType::Kernel(F::alloc_kernel_exec_memory() as usize)
    }
}

/// The user data associated with a probe point.
pub trait ProbeData: Any + Send + Sync + Debug {
    /// Get a reference to the data as a `dyn Any`.
    fn as_any(&self) -> &dyn Any;
}

/// The type of the probe handler function.
pub type ProbeHandlerFunc = fn(&dyn ProbeData, &mut PtRegs);

#[derive(Clone, Copy, Debug)]
pub(crate) struct ProbeHandler {
    pub(crate) func: ProbeHandlerFunc,
}

impl ProbeHandler {
    pub fn new(func: ProbeHandlerFunc) -> Self {
        ProbeHandler { func }
    }

    pub fn call(&self, data: &dyn ProbeData, pt_regs: &mut PtRegs) {
        (self.func)(data, pt_regs);
    }
}

/// The callback function type for events.
pub trait CallBackFunc: Send + Sync {
    /// Call the callback function.
    fn call(&self, trap_frame: &mut PtRegs);
}

/// The builder for creating a kprobe.
pub struct ProbeBuilder<F: KprobeAuxiliaryOps> {
    pub(crate) symbol: Option<String>,
    pub(crate) symbol_addr: usize,
    pub(crate) offset: usize,
    pub(crate) pre_handler: Option<ProbeHandler>,
    pub(crate) post_handler: Option<ProbeHandler>,
    pub(crate) fault_handler: Option<ProbeHandler>,
    pub(crate) event_callbacks: BTreeMap<u32, Box<dyn CallBackFunc>>,
    pub(crate) probe_point: Option<Arc<ProbePoint<F>>>,
    pub(crate) enable: bool,
    pub(crate) data: Option<Box<dyn ProbeData>>,
    pub(crate) user_pid: Option<i32>,
    pub(crate) _marker: core::marker::PhantomData<F>,
}

impl<F: KprobeAuxiliaryOps> Default for ProbeBuilder<F> {
    fn default() -> Self {
        Self::new()
    }
}

impl<F: KprobeAuxiliaryOps> ProbeBuilder<F> {
    /// Create a new kprobe builder.
    pub fn new() -> Self {
        ProbeBuilder {
            symbol: None,
            symbol_addr: 0,
            offset: 0,
            pre_handler: None,
            post_handler: None,
            event_callbacks: BTreeMap::new(),
            fault_handler: None,
            probe_point: None,
            enable: false,
            data: None,
            user_pid: None,
            _marker: core::marker::PhantomData,
        }
    }

    /// Build the kprobe with enable or disable.
    pub fn with_enable(mut self, enable: bool) -> Self {
        self.enable = enable;
        self
    }

    /// Build the kprobe with a symbol address.
    pub fn with_symbol_addr(mut self, symbol_addr: usize) -> Self {
        self.symbol_addr = symbol_addr;
        self
    }

    /// Build the kprobe with an offset.
    pub fn with_offset(mut self, offset: usize) -> Self {
        self.offset = offset;
        self
    }

    /// Build the kprobe with a symbol.
    pub fn with_symbol(mut self, symbol: String) -> Self {
        self.symbol = Some(symbol);
        self
    }

    /// Build the kprobe with user mode flag.
    pub fn with_user_mode(mut self, user_pid: i32) -> Self {
        self.user_pid = Some(user_pid);
        self
    }

    /// Build the kprobe with a specific user data.
    pub fn with_data<T: ProbeData>(mut self, data: T) -> Self {
        self.data = Some(Box::new(data));
        self
    }

    /// Build the kprobe with a pre handler function.
    pub fn with_pre_handler(mut self, func: ProbeHandlerFunc) -> Self {
        self.pre_handler = Some(ProbeHandler::new(func));
        self
    }

    /// Build the kprobe with a post handler function.
    pub fn with_post_handler(mut self, func: ProbeHandlerFunc) -> Self {
        self.post_handler = Some(ProbeHandler::new(func));
        self
    }

    /// Build the kprobe with a pre handler function.
    pub fn with_fault_handler(mut self, func: ProbeHandlerFunc) -> Self {
        self.fault_handler = Some(ProbeHandler::new(func));
        self
    }

    pub(crate) fn with_probe_point(mut self, point: Arc<ProbePoint<F>>) -> Self {
        self.probe_point = Some(point);
        self
    }

    /// Build the kprobe with an event callback function.
    pub fn with_event_callback(
        mut self,
        callback_id: u32,
        event_callback: Box<dyn CallBackFunc>,
    ) -> Self {
        self.event_callbacks.insert(callback_id, event_callback);
        self
    }

    /// Get the address of the instruction that should be probed.
    pub fn probe_addr(&self) -> usize {
        self.symbol_addr + self.offset
    }
}

/// The basic information of a probe.
pub struct ProbeBasic<L: RawMutex + 'static> {
    symbol: Option<String>,
    symbol_addr: usize,
    offset: usize,
    pre_handler: Option<ProbeHandler>,
    post_handler: Option<ProbeHandler>,
    fault_handler: Option<ProbeHandler>,
    event_callbacks: Mutex<L, BTreeMap<u32, Box<dyn CallBackFunc>>>,
    enable: AtomicBool,
    data: Box<dyn ProbeData>,
}

impl<L: RawMutex + 'static> Debug for ProbeBasic<L> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Kprobe")
            .field("symbol", &self.symbol)
            .field("symbol_addr", &self.symbol_addr)
            .field("offset", &self.offset)
            .finish()
    }
}

impl<L: RawMutex + 'static> ProbeBasic<L> {
    /// Call the pre handler function.
    pub fn call_pre_handler(&self, pt_regs: &mut PtRegs) {
        if let Some(ref handler) = self.pre_handler {
            handler.call(self.data.as_ref(), pt_regs);
        }
    }

    /// Call the post handler function.
    pub fn call_post_handler(&self, pt_regs: &mut PtRegs) {
        if let Some(ref handler) = self.post_handler {
            handler.call(self.data.as_ref(), pt_regs);
        }
    }

    /// Call the fault handler function.
    pub fn call_fault_handler(&self, pt_regs: &mut PtRegs) {
        if let Some(ref handler) = self.fault_handler {
            handler.call(self.data.as_ref(), pt_regs);
        }
    }

    /// Call the event callback function.
    pub fn call_event_callback(&self, pt_regs: &mut PtRegs) {
        let event_callbacks = self.event_callbacks.lock();
        for callback in event_callbacks.values() {
            callback.call(pt_regs);
        }
    }

    /// Register the event callback function.
    pub fn register_event_callback(&self, callback_id: u32, callback: Box<dyn CallBackFunc>) {
        self.event_callbacks.lock().insert(callback_id, callback);
    }

    /// Unregister the event callback function.
    pub fn unregister_event_callback(&self, callback_id: u32) {
        self.event_callbacks.lock().remove(&callback_id);
    }

    /// Disable the probe point.
    pub fn disable(&self) {
        self.enable.store(false, Ordering::Relaxed);
    }

    /// Enable the probe point.
    pub fn enable(&self) {
        self.enable.store(true, Ordering::Relaxed);
    }

    /// Check if the probe point is enabled.
    pub fn is_enabled(&self) -> bool {
        self.enable.load(Ordering::Relaxed)
    }

    /// Get the function name of the probe point.
    pub fn symbol(&self) -> Option<&str> {
        self.symbol.as_deref()
    }

    pub(crate) fn get_data(&self) -> &dyn ProbeData {
        self.data.as_ref()
    }
}

impl<L: RawMutex + 'static, F: KprobeAuxiliaryOps> From<ProbeBuilder<F>> for ProbeBasic<L> {
    fn from(value: ProbeBuilder<F>) -> Self {
        ProbeBasic {
            symbol: value.symbol,
            symbol_addr: value.symbol_addr,
            offset: value.offset,
            pre_handler: value.pre_handler,
            post_handler: value.post_handler,
            event_callbacks: Mutex::new(value.event_callbacks),
            fault_handler: value.fault_handler,
            enable: AtomicBool::new(value.enable),
            data: value.data.unwrap_or_else(|| Box::new(())),
        }
    }
}

impl<T: Any + Send + Sync + Debug> ProbeData for T {
    fn as_any(&self) -> &dyn Any {
        self
    }
}
