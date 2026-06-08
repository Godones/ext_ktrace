//! The retprobe implementation for the current architecture.
use alloc::{
    boxed::Box,
    collections::btree_map::BTreeMap,
    string::String,
    sync::{Arc, Weak},
    vec::Vec,
};
use core::{any::Any, fmt::Debug, sync::atomic::AtomicU64};

use lock_api::{Mutex, RawMutex};

use crate::{
    CallBackFunc, KprobeAuxiliaryOps, ProbeBuilder, ProbeData, ProbeHandler, ProbeHandlerFunc,
    PtRegs, arch_rethook_fixup_return, arch_rethook_prepare, kprobe::Kprobe,
};

/// The retprobe structure.
pub struct Retprobe<L: RawMutex + 'static, F: KprobeAuxiliaryOps> {
    kprobe: Kprobe<L, F>,
    nmissed: AtomicU64,
    entry_handler: Option<ProbeHandler>,
    ret_handler: Option<ProbeHandler>,
    event_callbacks: Mutex<L, BTreeMap<u32, Arc<dyn CallBackFunc>>>,
}

unsafe impl<L: RawMutex + 'static, F: KprobeAuxiliaryOps> Send for Retprobe<L, F> {}
unsafe impl<L: RawMutex + 'static, F: KprobeAuxiliaryOps> Sync for Retprobe<L, F> {}

impl<L: RawMutex + 'static, F: KprobeAuxiliaryOps> Debug for Retprobe<L, F> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Kretprobe")
            .field("kprobe", &self.kprobe)
            .field("nmissed", &self.nmissed)
            .finish()
    }
}

impl<L: RawMutex + 'static, F: KprobeAuxiliaryOps> Retprobe<L, F> {
    pub(crate) fn new(
        kprobe: Kprobe<L, F>,
        entry_handler: Option<ProbeHandler>,
        ret_handler: Option<ProbeHandler>,
    ) -> Self {
        Retprobe {
            kprobe,
            nmissed: AtomicU64::new(0),
            event_callbacks: Mutex::new(BTreeMap::new()),
            entry_handler,
            ret_handler,
        }
    }
    /// Get the underlying kprobe of the kretprobe.
    pub fn kprobe(&self) -> &Kprobe<L, F> {
        &self.kprobe
    }

    /// Register the event callback function.
    pub fn register_event_callback(&self, callback_id: u32, callback: Arc<dyn CallBackFunc>) {
        self.event_callbacks.lock().insert(callback_id, callback);
    }

    /// Unregister the event callback function.
    pub fn unregister_event_callback(&self, callback_id: u32) {
        self.event_callbacks.lock().remove(&callback_id);
    }
}

pub(crate) struct RetprobeData<L: RawMutex + 'static, F: KprobeAuxiliaryOps> {
    free_instances: Mutex<L, Vec<RetprobeInstance>>,
    pub(crate) retprobe: Mutex<L, Weak<Retprobe<L, F>>>,
}

impl<L: RawMutex + 'static, F: KprobeAuxiliaryOps> Debug for RetprobeData<L, F> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RetprobeData")
            .field("free_instances", &self.free_instances)
            .field("kretprobe", &self.retprobe)
            .finish()
    }
}

unsafe impl<L: RawMutex + 'static, F: KprobeAuxiliaryOps> Send for RetprobeData<L, F> {}
unsafe impl<L: RawMutex + 'static, F: KprobeAuxiliaryOps> Sync for RetprobeData<L, F> {}

/// The instance of a retprobe.
#[derive(Debug)]
pub struct RetprobeInstance {
    pub(crate) ret_addr: usize,
    pub(crate) frame: usize,
    user_data: Option<Box<dyn ProbeData>>,
    retprobe: Weak<dyn Any + Send + Sync>,
}

/// The builder for creating a retprobe.
pub struct RetprobeBuilder<L: RawMutex + 'static> {
    symbol: Option<String>,
    enable: bool,
    symbol_addr: usize,
    maxactive: u32,
    data: Vec<Box<dyn ProbeData>>,
    ret_handler: Option<ProbeHandler>,
    entry_handler: Option<ProbeHandler>,
    event_callbacks: BTreeMap<u32, ProbeHandler>,

    _marker: core::marker::PhantomData<L>,
}

impl<L: RawMutex + 'static> RetprobeBuilder<L> {
    /// Create a new kretprobe builder.
    pub fn new(maxactive: u32) -> Self {
        RetprobeBuilder {
            symbol: None,
            symbol_addr: 0,
            maxactive,
            data: Vec::new(),
            ret_handler: None,
            entry_handler: None,
            enable: false,
            event_callbacks: BTreeMap::new(),
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

    /// Build the kprobe with a symbol.
    pub fn with_symbol(mut self, symbol: String) -> Self {
        self.symbol = Some(symbol);
        self
    }

    pub(crate) fn handler(&self) -> (Option<ProbeHandler>, Option<ProbeHandler>) {
        (self.entry_handler, self.ret_handler)
    }

    /// Set the user data for the kretprobe.
    ///
    /// Remember that the data will be cloned for each instance of the kretprobe.
    /// This is useful for sharing data across multiple kretprobe instances.
    pub fn with_data<T: ProbeData + Clone>(mut self, data: T) -> Self {
        for _ in 0..self.maxactive {
            self.data.push(Box::new(data.clone()));
        }
        self
    }

    /// Build the kprobe with an event callback function.
    pub fn with_event_callback(
        mut self,
        callback_id: u32,
        event_callback: ProbeHandlerFunc,
    ) -> Self {
        self.event_callbacks
            .insert(callback_id, ProbeHandler::new(event_callback));
        self
    }

    /// Set the return handler for the kretprobe.
    pub fn with_ret_handler(mut self, func: ProbeHandlerFunc) -> Self {
        self.ret_handler = Some(ProbeHandler::new(func));
        self
    }

    /// Set the entry handler for the kretprobe.
    pub fn with_entry_handler(mut self, func: ProbeHandlerFunc) -> Self {
        self.entry_handler = Some(ProbeHandler::new(func));
        self
    }
}

impl<F: KprobeAuxiliaryOps + 'static, L: RawMutex + 'static> From<RetprobeBuilder<L>>
    for ProbeBuilder<F>
{
    fn from(mut value: RetprobeBuilder<L>) -> Self {
        let retprobe_data = RetprobeData::<L, F> {
            free_instances: {
                let mut instances = Vec::with_capacity(value.maxactive as usize);
                for _ in 0..value.maxactive {
                    let data = value.data.pop();
                    instances.push(RetprobeInstance {
                        ret_addr: 0,
                        user_data: data,
                        frame: 0,
                        retprobe: Weak::<Retprobe<L, F>>::new(),
                    });
                }
                Mutex::new(instances)
            },
            retprobe: Mutex::new(Weak::new()),
        };

        Self {
            symbol: value.symbol.clone(),
            symbol_addr: value.symbol_addr,
            offset: 0,
            pre_handler: Some(ProbeHandler::new(pre_handler_kretprobe::<L, F>)),
            post_handler: None,
            fault_handler: None,
            event_callbacks: BTreeMap::new(),
            probe_point: None,
            enable: value.enable,
            data: Some(Box::new(retprobe_data)),
            user_pid: None,
            _marker: core::marker::PhantomData,
        }
    }
}

fn pre_handler_kretprobe<L: RawMutex + 'static, F: KprobeAuxiliaryOps + 'static>(
    data: &dyn ProbeData,
    pt_regs: &mut PtRegs,
) {
    let retprobe_data = data.as_any().downcast_ref::<RetprobeData<L, F>>().unwrap();
    let free_instance = retprobe_data.free_instances.lock().pop();
    if let Some(mut instance) = free_instance {
        arch_rethook_prepare::<L, F>(&mut instance, pt_regs);
        let user_data = instance.user_data.as_deref().unwrap_or(&());

        let kretprobe = retprobe_data.retprobe.lock();
        let kretprobe = kretprobe.upgrade().unwrap();
        if let Some(entry_handler) = kretprobe.entry_handler {
            entry_handler.call(user_data, pt_regs);
        }
        instance.retprobe = Arc::downgrade(&(kretprobe as Arc<dyn Any + Send + Sync>));

        // insert the instance into the task
        F::insert_kretprobe_instance_to_task(instance);
    } else {
        log::warn!("No free KretprobeInstance available in pre_handler_kretprobe");
        let retprobe = retprobe_data.retprobe.lock();
        let retprobe = retprobe.upgrade();
        let Some(retprobe) = retprobe else {
            panic!("Retprobe is not available in pre_handler_kretprobe");
        };
        retprobe
            .nmissed
            .fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    }
}

pub(crate) fn rethook_trampoline_handler<L: RawMutex + 'static, F: KprobeAuxiliaryOps + 'static>(
    pt_regs: &mut PtRegs,
    frame: usize,
) -> usize {
    let retprobe_instance = F::pop_kretprobe_instance_from_task();
    let correct_ret_addr = retprobe_instance.ret_addr;
    assert_eq!(retprobe_instance.frame, frame);
    let user_data = retprobe_instance.user_data.as_deref().unwrap_or(&());

    let retprobe = retprobe_instance.retprobe.upgrade().unwrap();
    let retprobe = retprobe.as_ref().downcast_ref::<Retprobe<L, F>>().unwrap();

    // call the return handler if it exists
    if let Some(ret_handler) = retprobe.ret_handler {
        ret_handler.call(user_data, pt_regs);
    }

    // call the event callbacks if they exist
    let event_callbacks = retprobe
        .event_callbacks
        .lock()
        .iter()
        .map(|(_, callback)| callback.clone())
        .collect::<Vec<_>>();
    for callback in event_callbacks {
        callback.call(pt_regs);
    }

    // recycle the retprobe instance
    let retprobe_data = retprobe.kprobe.get_data();
    let retprobe_data = retprobe_data
        .as_any()
        .downcast_ref::<RetprobeData<L, F>>()
        .unwrap();
    retprobe_data.free_instances.lock().push(retprobe_instance);

    arch_rethook_fixup_return(pt_regs, correct_ret_addr);
    correct_ret_addr
}
