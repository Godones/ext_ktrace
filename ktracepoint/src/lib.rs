//! A Rust library for defining and managing tracepoints in a no_std environment.
//! It provides macros and structures to create tracepoints, manage their state,
//! and handle trace events efficiently.
//! The library is designed to be lightweight and suitable for embedded systems or
//! kernel-level programming where the standard library is not available.
//! It leverages Rust's powerful macro system to simplify the creation and management of tracepoints.
//! The macros provided by this library allow for easy insertion of tracepoints into code with minimal overhead.
//!
#![deny(missing_docs)]
#![no_std]
#![allow(clippy::new_without_default)]
extern crate alloc;

mod basic_macro;
mod point;
pub mod ptr;
mod trace_pipe;

use alloc::{
    boxed::Box,
    collections::BTreeMap,
    format,
    string::{String, ToString},
    sync::Arc,
    vec::Vec,
};
use core::{
    ops::{Deref, DerefMut},
    sync::atomic::AtomicUsize,
};

use lock_api::{Mutex, MutexGuard, RawMutex};
pub use paste;
pub use point::{
    CommonTracePointMeta, RawTracePointCallBackFunc, TraceEntry, TracePoint,
    TracePointCallBackFunc, TracePointFunc,
};
use static_keys::code_manipulate::CodeManipulator;
pub use tp_lexer;
use tp_lexer::compile_with_schema;
pub use trace_pipe::{
    TraceCmdLineCache, TraceCmdLineCacheSnapshot, TraceEntryParser, TracePipeOps, TracePipeRaw,
    TracePipeSnapshot,
};

/// KernelTraceOps trait provides kernel-level operations for tracing.
pub trait KernelTraceOps: Send + Sync {
    /// Get the current time in nanoseconds.
    fn time_now() -> u64;
    /// Get the current CPU ID.
    fn cpu_id() -> u32;
    /// Get the current process ID.
    fn current_pid() -> u32;
    /// Push a raw record to the trace pipe.
    fn trace_pipe_push_raw_record(buf: &[u8]);
    /// Cache the process name for a given PID.
    fn trace_cmdline_push(pid: u32);
    /// Write data to kernel text memory.
    fn write_kernel_text(addr: *mut core::ffi::c_void, data: &[u8]);
}

/// A utility struct to manipulate kernel code, primarily used for ensuring
/// that we can modify kernel code safely.
pub struct KernelCodeManipulator<T> {
    _marker: core::marker::PhantomData<T>,
}

impl<T: KernelTraceOps> CodeManipulator for KernelCodeManipulator<T> {
    unsafe fn write_code<const L: usize>(addr: *mut core::ffi::c_void, data: &[u8; L]) {
        log::debug!("Modifying kernel code at address: {:p}", addr);
        T::write_kernel_text(addr, data);
    }
}

/// TracePointMap is a mapping from tracepoint IDs to TracePoint references.
#[derive(Debug)]
pub struct TracePointMap<L: RawMutex + 'static, K: KernelTraceOps + 'static>(
    BTreeMap<u32, &'static TracePoint<L, K>>,
);

impl<L: RawMutex + 'static, K: KernelTraceOps + 'static> TracePointMap<L, K> {
    /// Create a new TracePointMap
    fn new() -> Self {
        Self(BTreeMap::new())
    }
}

impl<L: RawMutex + 'static, K: KernelTraceOps + 'static> Deref for TracePointMap<L, K> {
    type Target = BTreeMap<u32, &'static TracePoint<L, K>>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<L: RawMutex + 'static, K: KernelTraceOps + 'static> DerefMut for TracePointMap<L, K> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

/// TracingEventsManager manages tracing events, subsystems, and tracepoints.
#[derive(Debug)]
pub struct TracingEventsManager<L: RawMutex + 'static, K: KernelTraceOps + 'static> {
    subsystems: Mutex<L, BTreeMap<String, Arc<EventsSubsystem<L, K>>>>,
    map: Mutex<L, TracePointMap<L, K>>,
}

impl<L: RawMutex + 'static, K: KernelTraceOps + 'static> TracingEventsManager<L, K> {
    fn new(map: TracePointMap<L, K>) -> Self {
        Self {
            subsystems: Mutex::new(BTreeMap::new()),
            map: Mutex::new(map),
        }
    }

    /// Get the tracepoint map
    pub fn tracepoint_map(&self) -> MutexGuard<'_, L, TracePointMap<L, K>> {
        self.map.lock()
    }

    /// Create a subsystem by name
    ///
    /// If the subsystem already exists, return the existing subsystem.
    fn create_subsystem(&self, subsystem_name: &str) -> Arc<EventsSubsystem<L, K>> {
        if self.subsystems.lock().contains_key(subsystem_name) {
            return self
                .get_subsystem(subsystem_name)
                .expect("Subsystem should exist");
        }
        let subsystem = Arc::new(EventsSubsystem::new());
        self.subsystems
            .lock()
            .insert(subsystem_name.to_string(), subsystem.clone());
        subsystem
    }

    /// Get the subsystem by name
    pub fn get_subsystem(&self, subsystem_name: &str) -> Option<Arc<EventsSubsystem<L, K>>> {
        self.subsystems.lock().get(subsystem_name).cloned()
    }

    /// Remove the subsystem by name
    pub fn remove_subsystem(&self, subsystem_name: &str) -> Option<Arc<EventsSubsystem<L, K>>> {
        self.subsystems.lock().remove(subsystem_name)
    }

    /// Get all subsystems
    pub fn subsystem_names(&self) -> Vec<String> {
        self.subsystems
            .lock()
            .keys()
            .cloned()
            .collect::<Vec<String>>()
    }
}

/// EventsSubsystem represents a collection of events under a specific subsystem.
#[derive(Debug)]
pub struct EventsSubsystem<L: RawMutex + 'static, K: KernelTraceOps + 'static> {
    events: Mutex<L, BTreeMap<String, Arc<EventInfo<L, K>>>>,
}

impl<L: RawMutex + 'static, K: KernelTraceOps + 'static> EventsSubsystem<L, K> {
    fn new() -> Self {
        Self {
            events: Mutex::new(BTreeMap::new()),
        }
    }

    /// Create an event by name
    fn create_event(&self, event_name: &str, event_info: EventInfo<L, K>) {
        self.events
            .lock()
            .insert(event_name.to_string(), Arc::new(event_info));
    }

    /// Get the event by name
    pub fn get_event(&self, event_name: &str) -> Option<Arc<EventInfo<L, K>>> {
        self.events.lock().get(event_name).cloned()
    }

    /// Get all events in the subsystem
    pub fn event_names(&self) -> Vec<String> {
        self.events.lock().keys().cloned().collect::<Vec<String>>()
    }
}

/// EventInfo holds information about a specific trace event.
#[derive(Debug)]
pub struct EventInfo<L: RawMutex + 'static, K: KernelTraceOps + 'static> {
    enable: TracePointEnableFile<L, K>,
    tracepoint: &'static TracePoint<L, K>,
    format: TracePointFormatFile<L, K>,
    id: TracePointIdFile<L, K>,
    filter: TraceFilterFile<L, K>,
    // trigger:,
}

impl<L: RawMutex + 'static, K: KernelTraceOps + 'static> EventInfo<L, K> {
    fn new(tracepoint: &'static TracePoint<L, K>) -> Self {
        let enable = TracePointEnableFile::new(tracepoint);
        let format = TracePointFormatFile::new(tracepoint);
        let id = TracePointIdFile::new(tracepoint);
        let filter = TraceFilterFile::new(tracepoint);
        Self {
            enable,
            tracepoint,
            format,
            id,
            filter,
        }
    }

    /// Get the tracepoint
    pub fn tracepoint(&self) -> &'static TracePoint<L, K> {
        self.tracepoint
    }

    /// Get the enable file
    pub fn enable_file(&self) -> &TracePointEnableFile<L, K> {
        &self.enable
    }

    /// Get the format file
    pub fn format_file(&self) -> &TracePointFormatFile<L, K> {
        &self.format
    }

    /// Get the ID file
    pub fn id_file(&self) -> &TracePointIdFile<L, K> {
        &self.id
    }

    /// Get the filter file
    pub fn filter_file(&self) -> &TraceFilterFile<L, K> {
        &self.filter
    }
}

/// TracePointFormatFile provides a way to get the format of the tracepoint.
#[derive(Debug, Clone)]
pub struct TracePointFormatFile<L: RawMutex + 'static, K: KernelTraceOps + 'static> {
    tracepoint: &'static TracePoint<L, K>,
}

impl<L: RawMutex + 'static, K: KernelTraceOps + 'static> TracePointFormatFile<L, K> {
    fn new(tracepoint: &'static TracePoint<L, K>) -> Self {
        Self { tracepoint }
    }

    /// Read the tracepoint format
    ///
    /// Returns the format string of the tracepoint.
    pub fn read(&self) -> String {
        self.tracepoint.print_fmt()
    }
}

/// TracePointEnableFile provides a way to enable or disable the tracepoint.
#[derive(Debug, Clone)]
pub struct TracePointEnableFile<L: RawMutex + 'static, K: KernelTraceOps + 'static> {
    tracepoint: &'static TracePoint<L, K>,
}

impl<L: RawMutex + 'static, K: KernelTraceOps + 'static> TracePointEnableFile<L, K> {
    fn new(tracepoint: &'static TracePoint<L, K>) -> Self {
        Self { tracepoint }
    }

    /// Read the tracepoint status
    ///
    /// Returns true if the tracepoint is enabled, false otherwise.
    pub fn read(&self) -> &'static str {
        if self.tracepoint.default_is_enabled() {
            "1\n"
        } else {
            "0\n"
        }
    }
    /// Enable or disable the tracepoint
    pub fn write(&self, enable: char) {
        match enable {
            '1' => self.tracepoint.enable_default(),
            '0' => self.tracepoint.disable_default(),
            _ => {
                log::warn!("Invalid value for tracepoint enable: {enable}");
            }
        }
    }
}

/// TracePointEnableFile provides a way to enable or disable the tracepoint.
#[derive(Debug, Clone)]
pub struct TracePointIdFile<L: RawMutex + 'static, K: KernelTraceOps + 'static> {
    tracepoint: &'static TracePoint<L, K>,
}

impl<L: RawMutex + 'static, K: KernelTraceOps + 'static> TracePointIdFile<L, K> {
    fn new(tracepoint: &'static TracePoint<L, K>) -> Self {
        Self { tracepoint }
    }

    /// Read the tracepoint ID
    ///
    /// Returns the ID of the tracepoint.
    pub fn read(&self) -> String {
        format!("{}\n", self.tracepoint.id())
    }
}

/// TraceFilterFile provides a way to set filters on the tracepoint.
#[derive(Debug)]
pub struct TraceFilterFile<L: RawMutex + 'static, K: KernelTraceOps + 'static> {
    tracepoint: &'static TracePoint<L, K>,
    inner: Mutex<L, TraceFilterFileInner>,
}

#[derive(Debug, Clone)]
struct TraceFilterFileInner {
    filter_expr: Option<String>,
    pre_error: Option<String>,
}

impl<L: RawMutex + 'static, K: KernelTraceOps + 'static> TraceFilterFile<L, K> {
    fn new(tracepoint: &'static TracePoint<L, K>) -> Self {
        Self {
            tracepoint,
            inner: Mutex::new(TraceFilterFileInner {
                filter_expr: None,
                pre_error: None,
            }),
        }
    }

    /// Read the tracepoint filter.
    ///
    /// Returns the current filter expression or an error message if there was a pre-error.
    pub fn read(&self) -> String {
        if let Some(err) = self.inner.lock().pre_error.as_ref() {
            return err.clone();
        }
        if let Some(filter) = self.inner.lock().filter_expr.as_ref() {
            filter.clone()
        } else {
            "none\n".to_string()
        }
    }

    /// Write a new filter expression to the tracepoint.
    pub fn write(&self, filter: &str) -> Result<(), &'static str> {
        if filter.as_bytes()[0] == b'0' {
            // clear the filter and pre-error
            let mut inner = self.inner.lock();
            inner.filter_expr = None;
            inner.pre_error = None;
            self.tracepoint.set_compiled_expr(None);
            Ok(())
        } else {
            let schema = self.tracepoint.schema();
            let res = compile_with_schema(filter, *schema);
            match res {
                Ok(compiled_expr) => {
                    let mut inner = self.inner.lock();
                    inner.filter_expr = Some(filter.to_string());
                    inner.pre_error = None;
                    self.tracepoint.set_compiled_expr(Some(compiled_expr));
                    Ok(())
                }
                Err(mut e) => {
                    let mut inner = self.inner.lock();
                    e.message.push('\n');
                    inner.pre_error = Some(e.message);
                    inner.filter_expr = None;
                    self.tracepoint.set_compiled_expr(None);
                    Err("compile error")
                }
            }
        }
    }
}

unsafe extern "C" {
    fn __start_tracepoint();
    fn __stop_tracepoint();
}

/// Initialize the tracing events
///
/// The L type parameter is the lock type used for synchronizing access to the tracepoint map.
/// The K type parameter is the kernel trace operations type used for performing kernel-level operations.
///
/// Returns a Result containing the initialized TracingEventsManager or an error message.
pub fn global_init_events<L: RawMutex + 'static + Send + Sync, K: KernelTraceOps + 'static>()
-> Result<TracingEventsManager<L, K>, &'static str> {
    static TRACE_POINT_ID: AtomicUsize = AtomicUsize::new(0);
    let events_manager = TracingEventsManager::new(TracePointMap::<L, K>::new());
    let tracepoint_data_start = __start_tracepoint as *mut CommonTracePointMeta<L, K>;
    let tracepoint_data_end = __stop_tracepoint as *mut CommonTracePointMeta<L, K>;
    log::info!(
        "tracepoint_data_start: {:#x}, tracepoint_data_end: {:#x}",
        tracepoint_data_start as usize,
        tracepoint_data_end as usize
    );
    let tracepoint_data_len = (tracepoint_data_end as usize - tracepoint_data_start as usize)
        / size_of::<CommonTracePointMeta<L, K>>();
    let tracepoint_data =
        unsafe { core::slice::from_raw_parts_mut(tracepoint_data_start, tracepoint_data_len) };
    tracepoint_data.sort_by(|a, b| {
        a.trace_point
            .name()
            .cmp(b.trace_point.name())
            .then(a.trace_point.system().cmp(b.trace_point.system()))
    });
    log::info!("tracepoint_data_len: {tracepoint_data_len}");

    let mut tracepoint_map = events_manager.tracepoint_map();
    for tracepoint_meta in tracepoint_data {
        let tracepoint = tracepoint_meta.trace_point;
        let id = TRACE_POINT_ID.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        tracepoint.set_id(id as u32);
        tracepoint.register(tracepoint_meta.print_func, Box::new(tracepoint));
        tracepoint_map.insert(id as u32, tracepoint);
        log::info!(
            "tracepoint registered: {}:{}",
            tracepoint.system(),
            tracepoint.name(),
        );
        let subsys_name = tracepoint.system();
        let subsys = events_manager.create_subsystem(subsys_name);
        let event_info = EventInfo::new(tracepoint);
        subsys.create_event(tracepoint.name(), event_info);
    }
    drop(tracepoint_map); // Release the lock on the tracepoint map
    Ok(events_manager)
}
