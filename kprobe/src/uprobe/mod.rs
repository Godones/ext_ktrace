use alloc::sync::Arc;

use lock_api::RawMutex;

use crate::{
    KprobeAuxiliaryOps, KprobeOps, ProbeBuilder, ProbeInstallError, ProbeManager, ProbePointList,
    PtRegs, UniProbe, arch::Probe, clear_single_step, setup_single_step,
};

/// The uprobe structure for the current architecture.
pub type Uprobe<L, F> = Probe<L, F>;

/// Register a uprobe.
///
/// # Parameters
/// - `manager`: The uprobe manager.
/// - `uprobe_point_list`: The list of uprobe points.
/// - `uprobe_builder`: The uprobe builder.
///
/// # Returns
/// - An registered uprobe.
///
pub fn register_uprobe<L: RawMutex + 'static, F: KprobeAuxiliaryOps>(
    manager: &ProbeManager<L, F>,
    uprobe_point_list: &mut ProbePointList<F>,
    uprobe_builder: ProbeBuilder<F>,
) -> Result<Arc<Uprobe<L, F>>, ProbeInstallError> {
    assert!(
        uprobe_builder.user_pid.is_some(),
        "The builder must be for uprobe"
    );
    let uprobe = crate::__register_kprobe(uprobe_point_list, uprobe_builder)?;
    let uprobe = Arc::new(uprobe);
    manager.insert_probe(UniProbe::Uprobe(uprobe.clone()));
    Ok(uprobe)
}

/// Unregister a uprobe.
///
/// # Parameters
/// - `manager`: The uprobe manager.
/// - `uprobe_point_list`: The list of uprobe points.
/// - `uprobe`: The uprobe to unregister.
///
pub fn unregister_uprobe<L: RawMutex + 'static, F: KprobeAuxiliaryOps>(
    manager: &ProbeManager<L, F>,
    uprobe_point_list: &mut ProbePointList<F>,
    uprobe: Arc<Uprobe<L, F>>,
) {
    let uprobe_addr = uprobe.probe_point().break_address();
    manager.remove_probe(UniProbe::Uprobe(uprobe));

    if manager.kprobe_num(uprobe_addr) == 0 {
        uprobe_point_list.remove(&uprobe_addr);
    }
}

/// Run uprobe which has been registered on the address
///
/// # Parameters
/// - `uprobe_manager`: The uprobe manager.
/// - `pt_regs`: The trap pt_regs.
///
/// # Returns
/// - An `Option` containing the result of the uprobe handler. If no uprobe is found, it returns `None`.
///
pub fn uprobe_handler_from_break<L: RawMutex + 'static, F: KprobeAuxiliaryOps>(
    uprobe_manager: &ProbeManager<L, F>,
    pt_regs: &mut PtRegs,
) -> Option<()> {
    let break_addr = pt_regs.break_address();
    // log::debug!("uprobe_handler_from_break: break_addr: {:#x}", break_addr);
    let uprobe_list = uprobe_manager.get_break_list(break_addr);
    if let Some(uprobe_list) = uprobe_list {
        for uprobe in uprobe_list.iter() {
            if uprobe.is_enabled() {
                uprobe.call_pre_handler(pt_regs);
            }
        }

        let point = uprobe_list[0].probe_point();
        let dynamic_user_ptr = point.dynamic_user_ptr();
        if dynamic_user_ptr != 0 {
            setup_single_step(pt_regs, dynamic_user_ptr);
        } else {
            let instruction_slot_len = point.instruction_slot_len();
            let single_step_address = point.single_step_address() as *mut u8;
            let pid = point.pid();
            let user_ptr = F::alloc_user_exec_memory(pid, |ptr| {
                unsafe {
                    // Copy the prepared instruction slot back to user space.
                    core::ptr::copy_nonoverlapping(single_step_address, ptr, instruction_slot_len);
                }
            });

            // the original debug address
            let old_debug_address = point.debug_address();

            // after this, point.debug_address() will return the new debug address(user_ptr+ debug instruction len)
            let new_debug_address = point.set_dynamic_user_ptr(user_ptr as usize);

            // beacause the debug address of uprobe is dynamically created, so the key value of the debug_list maintained in the original uprobe_manager needs to be updated
            uprobe_manager
                .replace_debug_list_with_new_address(old_debug_address, new_debug_address);

            // update the single step address to the copied instruction
            setup_single_step(pt_regs, user_ptr as usize);
        }

        Some(())
    } else {
        // For some architectures, they do not support single step execution,
        // and we need to use breakpoint exceptions to simulate
        uprobe_handler_from_debug(uprobe_manager, pt_regs)
    }
}

/// Run uprobe which has been registered on the address
///
/// # Parameters
/// - `uprobe_manager`: The uprobe manager.
/// - `pt_regs`: The trap pt_regs.
///
/// # Returns
/// - An `Option` containing the result of the uprobe handler. If no uprobe is found, it returns `None`.
///
pub fn uprobe_handler_from_debug<L: RawMutex + 'static, F: KprobeAuxiliaryOps>(
    uprobe_manager: &ProbeManager<L, F>,
    pt_regs: &mut PtRegs,
) -> Option<()> {
    let pc = pt_regs.debug_address();
    if let Some(uprobe_list) = uprobe_manager.get_debug_list(pc) {
        for uprobe in uprobe_list.iter() {
            if uprobe.is_enabled() {
                uprobe.call_post_handler(pt_regs);
                uprobe.call_event_callback(pt_regs);
            }
        }
        let return_address = uprobe_list[0].probe_point().return_address();
        clear_single_step(pt_regs, return_address);
        Some(())
    } else {
        log::debug!("There is no uprobe on pc {pc:#x}");
        None
    }
}
