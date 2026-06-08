//! A Rust library for dynamic kernel probing (kprobes and kretprobes).
//!! This library provides a safe and ergonomic interface for registering and managing kprobes
//! and kretprobes in a kernel environment.
//!! It supports multiple architectures, including x86_64, riscv64, and loongarch64.
//!! # Features
//! - Register and unregister kprobes and kretprobes.
//! - Support for pre-handler and post-handler functions.
//! - Safe management of probe points and handlers.
//! - Architecture-specific implementations for handling breakpoints and single-stepping.
//!
#![no_std]
#![deny(missing_docs)]
extern crate alloc;

mod arch;
mod kprobe;
mod kretprobe;
mod manager;
mod uprobe;
mod uretprobe;

use alloc::sync::Arc;
use core::ops::Deref;

pub use arch::*;
pub use kprobe::*;
pub use kretprobe::*;
use lock_api::RawMutex;
pub use manager::*;
pub use uprobe::*;
pub use uretprobe::*;

use crate::arch::retprobe::RetprobeData;

/// An enum representing either a kprobe or a kretprobe.
#[derive(Debug)]
#[allow(missing_docs)]
pub enum UniProbe<L: RawMutex + 'static, F: KprobeAuxiliaryOps> {
    Kprobe(Arc<Kprobe<L, F>>),
    Kretprobe(Arc<Kretprobe<L, F>>),
    Uprobe(Arc<Uprobe<L, F>>),
    Uretprobe(Arc<Uretprobe<L, F>>),
}

impl<L: RawMutex + 'static, F: KprobeAuxiliaryOps> PartialEq for UniProbe<L, F> {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (UniProbe::Kprobe(kp1), UniProbe::Kprobe(kp2)) => Arc::ptr_eq(kp1, kp2),
            (UniProbe::Kretprobe(kp1), UniProbe::Kretprobe(kp2)) => Arc::ptr_eq(kp1, kp2),
            (UniProbe::Uprobe(up1), UniProbe::Uprobe(up2)) => Arc::ptr_eq(up1, up2),
            (UniProbe::Uretprobe(up1), UniProbe::Uretprobe(up2)) => Arc::ptr_eq(up1, up2),
            _ => false,
        }
    }
}

impl<L: RawMutex + 'static, F: KprobeAuxiliaryOps> Deref for UniProbe<L, F> {
    type Target = Kprobe<L, F>;
    fn deref(&self) -> &Self::Target {
        match self {
            UniProbe::Kprobe(kprobe) => kprobe,
            UniProbe::Kretprobe(kretprobe) => kretprobe.kprobe(),
            UniProbe::Uprobe(uprobe) => uprobe,
            UniProbe::Uretprobe(uretprobe) => uretprobe.kprobe(),
        }
    }
}

impl<L: RawMutex + 'static, F: KprobeAuxiliaryOps> UniProbe<L, F> {
    /// Get the probe point of the probe.
    pub fn probe_point(&self) -> &Arc<ProbePoint<F>> {
        match self {
            UniProbe::Kprobe(kprobe) => kprobe.probe_point(),
            UniProbe::Kretprobe(kretprobe) => kretprobe.kprobe().probe_point(),
            UniProbe::Uprobe(uprobe) => uprobe.probe_point(),
            UniProbe::Uretprobe(uretprobe) => uretprobe.kprobe().probe_point(),
        }
    }
    /// Check if the probe is a kretprobe.
    pub fn is_kretprobe(&self) -> bool {
        matches!(self, UniProbe::Kretprobe(_))
    }
}

impl<L: RawMutex + 'static, F: KprobeAuxiliaryOps> Clone for UniProbe<L, F> {
    fn clone(&self) -> Self {
        match self {
            UniProbe::Kprobe(kprobe) => UniProbe::Kprobe(kprobe.clone()),
            UniProbe::Kretprobe(kretprobe) => UniProbe::Kretprobe(kretprobe.clone()),
            UniProbe::Uprobe(uprobe) => UniProbe::Uprobe(uprobe.clone()),
            UniProbe::Uretprobe(uretprobe) => UniProbe::Uretprobe(uretprobe.clone()),
        }
    }
}

/// Register a kretprobe.
///
/// See [`register_kprobe`] for more details.
pub fn register_kretprobe<L: RawMutex + 'static, F: KprobeAuxiliaryOps + 'static>(
    manager: &ProbeManager<L, F>,
    kprobe_point_list: &mut ProbePointList<F>,
    kretprobe_builder: KretprobeBuilder<L>,
) -> Result<Arc<Kretprobe<L, F>>, ProbeInstallError> {
    let (entry_handler, ret_handler) = kretprobe_builder.handler();

    let kprobe_builder = ProbeBuilder::from(kretprobe_builder);
    let kprobe = kprobe::__register_kprobe(kprobe_point_list, kprobe_builder)?;

    let kretprobe = Kretprobe::new(kprobe, entry_handler, ret_handler);
    let kretprobe = Arc::new(kretprobe);

    let data = kretprobe.kprobe().get_data();
    let data = data.as_any().downcast_ref::<RetprobeData<L, F>>().unwrap();
    *data.retprobe.lock() = Arc::downgrade(&kretprobe);

    manager.insert_probe(UniProbe::Kretprobe(kretprobe.clone()));
    Ok(kretprobe)
}

/// Unregister a kretprobe.
///
/// See [`unregister_kprobe`] for more details.
pub fn unregister_kretprobe<L: RawMutex + 'static, F: KprobeAuxiliaryOps>(
    manager: &ProbeManager<L, F>,
    kprobe_point_list: &mut ProbePointList<F>,
    kretprobe: Arc<Kretprobe<L, F>>,
) {
    let kprobe = kretprobe.kprobe();
    let kprobe_addr = kprobe.probe_point().break_address();
    manager.remove_probe(UniProbe::Kretprobe(kretprobe));

    if manager.kprobe_num(kprobe_addr) == 0 {
        kprobe_point_list.remove(&kprobe_addr);
    }
}
