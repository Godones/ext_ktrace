use alloc::{boxed::Box, collections::BTreeMap, format, string::String};
use core::{
    any::Any,
    ops::Deref,
    sync::atomic::{AtomicBool, AtomicU32, Ordering},
};

use static_keys::RawStaticFalseKey;
use tp_lexer::{Compiled, Schema};

use crate::{KernelCodeManipulator, KernelTraceOps};

/// A trace entry structure that holds metadata about a trace event.
#[derive(Debug)]
#[repr(C)]
pub struct TraceEntry {
    /// The type of the trace event, typically the tracepoint ID.
    pub common_type: u16,
    /// Flags associated with the trace event.
    pub common_flags: u8,
    /// The preemption count at the time of the event.
    pub common_preempt_count: u8,
    /// The PID of the process that generated the event.
    pub common_pid: i32,
}

impl TraceEntry {
    /// Returns a formatted string representing the latency and preemption state.
    pub fn trace_print_lat_fmt(&self) -> String {
        // todo!("Implement IRQs off logic");
        let irqs_off = '.';
        let resched = '.';
        let hardsoft_irq = '.';
        let mut preempt_low = '.';
        if self.common_preempt_count & 0xf != 0 {
            preempt_low = ((b'0') + (self.common_preempt_count & 0xf)) as char;
        }
        let mut preempt_high = '.';
        if self.common_preempt_count >> 4 != 0 {
            preempt_high = ((b'0') + (self.common_preempt_count >> 4)) as char;
        }
        format!("{irqs_off}{resched}{hardsoft_irq}{preempt_low}{preempt_high}")
    }
}

/// CommonTracePointMeta holds metadata for a common tracepoint.
#[derive(Debug)]
#[repr(C)]
pub struct CommonTracePointMeta<K: KernelTraceOps + 'static> {
    /// A reference to the tracepoint.
    pub trace_point: &'static TracePoint<K>,
    /// The print function for the tracepoint.
    pub print_func: fn(),
}

/// A trait for callback functions that can be registered with a tracepoint.
pub trait TracePointCallBackFunc: Send + Sync {
    /// Call the callback function with the given trace entry data.
    fn call(&self, entry: &[u8]);
}

/// A trait for raw callback functions that can be registered with a tracepoint.
pub trait RawTracePointCallBackFunc: Send + Sync {
    /// Call the callback function with the given raw trace entry data.
    fn call(&self, args: &[u64]);
}

/// A structure representing a registered tracepoint callback function.
#[derive(Debug)]
pub struct TracePointFunc {
    /// The callback function to be executed.
    pub func: fn(),
    /// The data associated with the callback function.
    pub data: Box<dyn Any + Send + Sync>,
}

/// The TracePoint structure represents a tracepoint in the system.
pub struct TracePoint<K: KernelTraceOps + 'static> {
    name: &'static str,
    system: &'static str,
    key: &'static RawStaticFalseKey<KernelCodeManipulator<K>>,
    event_status: AtomicBool,
    id: AtomicU32,
    trace_entry_fmt_func: fn(&[u8]) -> String,
    trace_print_func: fn() -> String,
    schema: Schema,
    flags: u8,
}

impl<K: KernelTraceOps + 'static> core::fmt::Debug for TracePoint<K> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TracePoint")
            .field("name", &self.name)
            .field("system", &self.system)
            .field("id", &self.id())
            .field("flags", &self.flags)
            .finish()
    }
}

/// An extended tracepoint structure that includes additional callback management and compiled expression handling.
pub struct ExtTracePoint<K: KernelTraceOps + 'static> {
    tracepoint: &'static TracePoint<K>,
    default_callbacks: BTreeMap<usize, TracePointFunc>,
    event_callbacks: BTreeMap<usize, Box<dyn TracePointCallBackFunc>>,
    raw_event_callbacks: BTreeMap<usize, Box<dyn RawTracePointCallBackFunc>>,
    compiled_expr: Option<Compiled>,
}

impl<K: KernelTraceOps + 'static> core::fmt::Debug for ExtTracePoint<K> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ExtTracePoint")
            .field("tracepoint", &self.tracepoint)
            .finish()
    }
}

impl<K: KernelTraceOps + 'static> Deref for ExtTracePoint<K> {
    type Target = TracePoint<K>;

    fn deref(&self) -> &Self::Target {
        self.tracepoint
    }
}

impl<K: KernelTraceOps + 'static> ExtTracePoint<K> {
    /// Creates a new ExtTracePoint instance.
    pub const fn new(tracepoint: &'static TracePoint<K>) -> Self {
        Self {
            tracepoint,
            default_callbacks: BTreeMap::new(),
            event_callbacks: BTreeMap::new(),
            raw_event_callbacks: BTreeMap::new(),
            compiled_expr: None,
        }
    }

    /// Returns a reference to the underlying TracePoint.
    pub const fn trace_point(&self) -> &'static TracePoint<K> {
        self.tracepoint
    }

    /// Sets the compiled expression for the tracepoint.
    pub fn set_compiled_expr(&mut self, compiled: Option<Compiled>) {
        self.compiled_expr = compiled;
    }

    /// Returns the compiled expression for the tracepoint.
    pub fn get_compiled_expr(&self) -> Option<Compiled> {
        self.compiled_expr.clone()
    }

    /// Register a callback function to the tracepoint
    pub fn register(&mut self, func: fn(), data: Box<dyn Any + Sync + Send>) {
        let trace_point_func = TracePointFunc { func, data };
        let ptr = func as usize;
        self.default_callbacks
            .entry(ptr)
            .or_insert(trace_point_func);
    }

    /// Unregister a callback function from the tracepoint
    pub fn unregister(&mut self, func: fn()) {
        let func_ptr = func as usize;
        self.default_callbacks.remove(&func_ptr);
    }

    /// Iterate over all registered callback functions
    pub fn callback_list(&self, f: &dyn Fn(&TracePointFunc)) {
        for trace_func in self.default_callbacks.values() {
            f(trace_func);
        }
    }

    /// Register a event callback function to the tracepoint
    ///
    /// This function will be called when default tracepoint fmt function is called.
    pub fn register_event_callback(
        &mut self,
        callback_id: usize,
        callback: Box<dyn TracePointCallBackFunc>,
    ) {
        self.event_callbacks.entry(callback_id).or_insert(callback);
    }

    /// Unregister a event callback function from the tracepoint
    pub fn unregister_event_callback(&mut self, callback_id: usize) {
        self.event_callbacks.remove(&callback_id);
    }

    /// Iterate over all registered event callback functions
    pub fn event_callback_list(&self, f: &dyn Fn(&Box<dyn TracePointCallBackFunc>)) {
        for callback in self.event_callbacks.values() {
            f(callback);
        }
    }

    /// Register a raw event callback function to the tracepoint
    pub fn register_raw_event_callback(
        &mut self,
        callback_id: usize,
        callback: Box<dyn RawTracePointCallBackFunc>,
    ) {
        self.raw_event_callbacks
            .entry(callback_id)
            .or_insert(callback);
    }

    /// Unregister a raw event callback function from the tracepoint
    pub fn unregister_raw_event_callback(&mut self, callback_id: usize) {
        self.raw_event_callbacks.remove(&callback_id);
    }

    /// Iterate over all registered raw event callback functions
    pub fn raw_event_callback_list(&self, f: &dyn Fn(&Box<dyn RawTracePointCallBackFunc>)) {
        for callback in self.raw_event_callbacks.values() {
            f(callback);
        }
    }
}

impl<K: KernelTraceOps + 'static> TracePoint<K> {
    /// Creates a new TracePoint instance.
    pub const fn new(
        key: &'static RawStaticFalseKey<KernelCodeManipulator<K>>,
        name: &'static str,
        system: &'static str,
        fmt_func: fn(&[u8]) -> String,
        trace_print_func: fn() -> String,
        schema: Schema,
    ) -> Self {
        Self {
            name,
            system,
            key,
            event_status: AtomicBool::new(false),
            id: AtomicU32::new(0),
            flags: 0,
            trace_entry_fmt_func: fmt_func,
            trace_print_func,
            schema,
        }
    }

    /// Returns the schema of the tracepoint.
    pub fn schema(&self) -> &Schema {
        &self.schema
    }

    /// Returns the name of the tracepoint.
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// Returns the system of the tracepoint.
    pub fn system(&self) -> &'static str {
        self.system
    }

    /// Sets the ID of the tracepoint.
    pub(crate) fn set_id(&self, id: u32) {
        self.id.store(id, Ordering::Relaxed);
    }

    /// Returns the ID of the tracepoint.
    pub fn id(&self) -> u32 {
        self.id.load(Ordering::Relaxed)
    }

    /// Returns the flags of the tracepoint.
    pub fn flags(&self) -> u8 {
        self.flags
    }

    /// Returns the format function for the tracepoint.
    pub(crate) fn fmt_func(&self) -> fn(&[u8]) -> String {
        self.trace_entry_fmt_func
    }

    /// Returns a string representation of the format function for the tracepoint.
    ///
    /// You can use `cat /sys/kernel/debug/tracing/events/syscalls/sys_enter_openat/format` in linux
    /// to see the format of the tracepoint.
    pub fn print_fmt(&self) -> String {
        let post_str = (self.trace_print_func)();
        format!("name: {}\nID: {}\n{}\n", self.name(), self.id(), post_str)
    }

    /// Enable the tracepoint for the default print
    pub fn enable_default(&self) {
        unsafe {
            self.key.enable();
        }
    }

    /// Disable the tracepoint for the default print
    pub fn disable_default(&self) {
        unsafe {
            self.key.disable();
        }
    }

    /// Check if the tracepoint is enabled for the default print
    pub fn default_is_enabled(&self) -> bool {
        self.key.is_enabled()
    }

    /// Enable the tracepoint event for custom event handling
    pub fn enable_event(&self) {
        self.event_status.store(true, Ordering::Relaxed);
    }

    /// Disable the tracepoint event for custom event handling
    pub fn disable_event(&self) {
        self.event_status.store(false, Ordering::Relaxed);
    }

    /// Check if the tracepoint event is enabled for custom event handling
    pub fn event_is_enabled(&self) -> bool {
        self.event_status.load(Ordering::Relaxed)
    }
}
