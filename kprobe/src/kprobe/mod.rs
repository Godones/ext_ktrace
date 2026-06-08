use alloc::sync::Arc;

use lock_api::RawMutex;

use crate::{
    KprobeAuxiliaryOps, KprobeOps, ProbeBuilder, ProbeInstallError, ProbeManager, ProbePointList,
    PtRegs, UniProbe, arch::Probe, clear_single_step, setup_single_step,
};

/// The kprobe structure for the current architecture.
pub type Kprobe<L, F> = Probe<L, F>;

pub(crate) fn __register_kprobe<L: RawMutex + 'static, F: KprobeAuxiliaryOps>(
    kprobe_point_list: &mut ProbePointList<F>,
    kprobe_builder: ProbeBuilder<F>,
) -> Result<Kprobe<L, F>, ProbeInstallError> {
    let address = kprobe_builder.probe_addr();
    let existed_point = kprobe_point_list.get(&address).map(Clone::clone);

    match existed_point {
        Some(existed_point) => Ok(kprobe_builder.with_probe_point(existed_point).install()?.0),
        None => {
            let (kprobe, probe_point) = kprobe_builder.install()?;
            kprobe_point_list.insert(address, probe_point);
            Ok(kprobe)
        }
    }
}

/// Register a kprobe.
///
/// # Parameters
/// - `manager`: The kprobe manager.
/// - `kprobe_point_list`: The list of kprobe points.
/// - `kprobe_builder`: The kprobe builder.
///
/// # Returns
/// - An registered kprobe.
///
pub fn register_kprobe<L: RawMutex + 'static, F: KprobeAuxiliaryOps>(
    manager: &ProbeManager<L, F>,
    kprobe_point_list: &mut ProbePointList<F>,
    kprobe_builder: ProbeBuilder<F>,
) -> Result<Arc<Kprobe<L, F>>, ProbeInstallError> {
    let kprobe = __register_kprobe(kprobe_point_list, kprobe_builder)?;
    let kprobe = Arc::new(kprobe);
    manager.insert_probe(UniProbe::Kprobe(kprobe.clone()));
    Ok(kprobe)
}

/// Unregister a kprobe.
///
/// # Parameters
/// - `manager`: The kprobe manager.
/// - `kprobe_point_list`: The list of kprobe points.
/// - `kprobe`: The kprobe to unregister.
///
pub fn unregister_kprobe<L: RawMutex + 'static, F: KprobeAuxiliaryOps>(
    manager: &ProbeManager<L, F>,
    kprobe_point_list: &mut ProbePointList<F>,
    kprobe: Arc<Kprobe<L, F>>,
) {
    let kprobe_addr = kprobe.probe_point().break_address();
    manager.remove_probe(UniProbe::Kprobe(kprobe));

    if manager.kprobe_num(kprobe_addr) == 0 {
        kprobe_point_list.remove(&kprobe_addr);
    }
}

/// Run kprobe which has been registered on the address
///
/// # Parameters
/// - `manager`: The kprobe manager.
/// - `pt_regs`: The trap pt_regs.
///
/// # Returns
/// - An `Option` containing the result of the kprobe handler. If no kprobe is found, it returns `None`.
///
pub fn kprobe_handler_from_break<L: RawMutex + 'static, F: KprobeAuxiliaryOps>(
    manager: &ProbeManager<L, F>,
    pt_regs: &mut PtRegs,
) -> Option<()> {
    let break_addr = pt_regs.break_address();
    // log::debug!("kprobe_handler_from_break: break_addr: {:#x}", break_addr);
    let kprobe_list = manager.get_break_list(break_addr);
    if let Some(kprobe_list) = kprobe_list {
        for kprobe in kprobe_list.iter() {
            if kprobe.is_enabled() {
                kprobe.call_pre_handler(pt_regs);
            }
        }
        let single_step_address = kprobe_list[0].probe_point().single_step_address();
        setup_single_step(pt_regs, single_step_address);
        Some(())
    } else {
        // For some architectures, they do not support single step execution,
        // and we need to use breakpoint exceptions to simulate
        kprobe_handler_from_debug(manager, pt_regs)
    }
}

/// Run kprobe which has been registered on the address
///
/// # Parameters
/// - `manager`: The kprobe manager.
/// - `pt_regs`: The trap pt_regs.
///
/// # Returns
/// - An `Option` containing the result of the kprobe handler. If no kprobe is found, it returns `None`.
///
pub fn kprobe_handler_from_debug<L: RawMutex + 'static, F: KprobeAuxiliaryOps>(
    manager: &ProbeManager<L, F>,
    pt_regs: &mut PtRegs,
) -> Option<()> {
    let pc = pt_regs.debug_address();
    if let Some(kprobe_list) = manager.get_debug_list(pc) {
        for kprobe in kprobe_list.iter() {
            if kprobe.is_enabled() {
                kprobe.call_post_handler(pt_regs);
                kprobe.call_event_callback(pt_regs);
            }
        }
        let return_address = kprobe_list[0].probe_point().return_address();
        clear_single_step(pt_regs, return_address);
        Some(())
    } else {
        log::info!("There is no kprobe on pc {pc:#x}");
        None
    }
}
