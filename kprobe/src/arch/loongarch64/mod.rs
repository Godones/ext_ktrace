use alloc::sync::Arc;
use core::{
    fmt::Debug,
    ops::{Deref, DerefMut},
    sync::atomic::AtomicUsize,
};

use lock_api::RawMutex;

use super::{
    ExecMemType, KprobeAuxiliaryOps,
    retprobe::{RetprobeInstance, rethook_trampoline_handler},
};
use crate::{KprobeOps, ProbeBasic, ProbeBuilder, ProbeInstallError};
// const BRK_KPROBE_BP: u64 = 10;
// const BRK_KPROBE_SSTEPBP: u64 = 11;
const EBREAK_INST: u32 = 0x002a0000;
const EBREAK_INST_LEN: usize = 4;

/// The kprobe structure.
pub struct Probe<L: RawMutex + 'static, F: KprobeAuxiliaryOps> {
    basic: ProbeBasic<L>,
    point: Arc<LA64ProbePoint<F>>,
}

/// The kprobe point structure for LoongArch64 architecture.
#[derive(Debug)]
pub struct LA64ProbePoint<F: KprobeAuxiliaryOps> {
    addr: usize,
    old_instruction_ptr: ExecMemType<F>,
    user_pid: Option<i32>,
    // Dynamic user pointer for handling user space instruction pointer adjustments
    dynamic_user_ptr: AtomicUsize,
    _marker: core::marker::PhantomData<F>,
}

impl<L: RawMutex + 'static, F: KprobeAuxiliaryOps> Deref for Probe<L, F> {
    type Target = ProbeBasic<L>;

    fn deref(&self) -> &Self::Target {
        &self.basic
    }
}

impl<L: RawMutex + 'static, F: KprobeAuxiliaryOps> DerefMut for Probe<L, F> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.basic
    }
}

impl<L: RawMutex + 'static, F: KprobeAuxiliaryOps> Probe<L, F> {
    /// Get the probe point of the kprobe.
    pub fn probe_point(&self) -> &Arc<LA64ProbePoint<F>> {
        &self.point
    }
}

impl<L: RawMutex + 'static, F: KprobeAuxiliaryOps> Debug for Probe<L, F> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Kprobe")
            .field("basic", &self.basic)
            .field("point", &self.point)
            .finish()
    }
}

impl<F: KprobeAuxiliaryOps> Drop for LA64ProbePoint<F> {
    fn drop(&mut self) {
        let address = self.addr;
        let inst_tmp_ptr = self.old_instruction_ptr.as_ptr();
        F::set_writeable_for_address(address, EBREAK_INST_LEN, self.user_pid, |ptr| {
            unsafe {
                // Restore the original instruction at the probe address
                core::ptr::copy_nonoverlapping(inst_tmp_ptr as *const u8, ptr, EBREAK_INST_LEN);
            }
        });

        // Free the dynamic user pointer if it was allocated
        let dyn_ptr = self.dynamic_user_ptr();
        if dyn_ptr != 0 {
            F::free_user_exec_memory(self.user_pid, dyn_ptr as *mut u8);
        }

        log::trace!("Kprobe::uninstall: address: {address:#x}");
    }
}

impl<F: KprobeAuxiliaryOps> ProbeBuilder<F> {
    /// Install the kprobe by replacing the instruction at the specified address with a breakpoint instruction.
    pub fn install<L: RawMutex + 'static>(
        self,
    ) -> Result<(Probe<L, F>, Arc<LA64ProbePoint<F>>), ProbeInstallError> {
        let probe_point = match &self.probe_point {
            Some(point) => point.clone(),
            None => self.replace_inst()?,
        };
        let probe = Probe {
            basic: ProbeBasic::from(self),
            point: probe_point.clone(),
        };
        Ok((probe, probe_point))
    }

    /// Replace the instruction at the specified address with a breakpoint instruction.
    fn replace_inst(&self) -> Result<Arc<LA64ProbePoint<F>>, ProbeInstallError> {
        let address = self.symbol_addr + self.offset;
        if address & (EBREAK_INST_LEN - 1) != 0 {
            return Err(ProbeInstallError::InvalidAddress);
        }

        let inst_tmp_ptr = super::alloc_exec_memory::<F>(self.user_pid);
        let mut inst_32 = 0u32;
        F::copy_memory(
            address as *const u8,
            &mut inst_32 as *mut u32 as *mut u8,
            4,
            self.user_pid,
        );
        validate_instruction(inst_32)?;

        unsafe {
            F::set_writeable_for_address(address, EBREAK_INST_LEN, self.user_pid, |ptr| {
                // Replace the original instruction with the breakpoint instruction
                core::ptr::write(ptr as *mut u32, EBREAK_INST);
            });
            // Save the original instruction to the temporary memory
            // inst_32 :0-32
            // ebreak  :32-64
            core::ptr::write(inst_tmp_ptr.as_ptr() as *mut u32, inst_32);
            core::ptr::write(
                (inst_tmp_ptr.as_ptr() as usize + 4) as *mut u32,
                EBREAK_INST,
            );
        }

        let point = LA64ProbePoint {
            addr: address,
            old_instruction_ptr: inst_tmp_ptr,
            user_pid: self.user_pid,
            dynamic_user_ptr: AtomicUsize::new(0),
            _marker: core::marker::PhantomData,
        };

        log::trace!(
            "Kprobe::install: address: {:#x}, func_name: {:?}, opcode: {:x?}",
            address,
            self.symbol,
            inst_32
        );
        Ok(Arc::new(point))
    }
}

// References:
// - LoongArch Reference Manual, Vol. 1:
//   https://loongson.github.io/LoongArch-Documentation/LoongArch-Vol1-EN.html
// - Linux LoongArch kprobes implementation:
//   https://codebrowser.dev/linux/linux/arch/loongarch/kernel/kprobes.c.html
fn validate_instruction(inst: u32) -> Result<(), ProbeInstallError> {
    if inst == EBREAK_INST {
        return Err(ProbeInstallError::UnsupportedInstruction);
    }

    let opcode = inst >> 26;
    // LoongArch branch and PC-relative forms require relocation support before out-of-line use.
    if matches!(opcode, 0x13..=0x17 | 0x19..=0x1b) {
        return Err(ProbeInstallError::UnsafeInstruction);
    }

    Ok(())
}

impl<F: KprobeAuxiliaryOps> KprobeOps for LA64ProbePoint<F> {
    fn return_address(&self) -> usize {
        self.addr + self.original_instruction_len()
    }

    fn single_step_address(&self) -> usize {
        self.old_instruction_ptr.as_ptr() as usize
    }

    fn debug_address(&self) -> usize {
        let dyn_ptr = self.dynamic_user_ptr();
        if dyn_ptr != 0 {
            dyn_ptr + self.original_instruction_len()
        } else {
            self.old_instruction_ptr.as_ptr() as usize + self.original_instruction_len()
        }
    }

    fn break_address(&self) -> usize {
        self.addr
    }

    fn dynamic_user_ptr(&self) -> usize {
        self.dynamic_user_ptr
            .load(core::sync::atomic::Ordering::SeqCst)
    }

    fn set_dynamic_user_ptr(&self, ptr: usize) -> usize {
        self.dynamic_user_ptr
            .store(ptr, core::sync::atomic::Ordering::SeqCst);

        ptr + self.original_instruction_len()
    }

    fn original_instruction_len(&self) -> usize {
        EBREAK_INST_LEN
    }

    fn instruction_slot_len(&self) -> usize {
        EBREAK_INST_LEN * 2
    }

    fn pid(&self) -> Option<i32> {
        self.user_pid
    }
}
/// The register state at the time of the probe.
#[repr(C)]
#[derive(Debug, Copy, Clone)]
#[repr(align(8))]
#[allow(missing_docs)]
pub struct PtRegs {
    pub regs: [usize; 32],
    pub orig_a0: usize,
    pub csr_era: usize,
    pub csr_badvaddr: usize,
    pub csr_crmd: usize,
    pub csr_prmd: usize,
    pub csr_euen: usize,
    pub csr_ecfg: usize,
    pub csr_estat: usize,
}

impl PtRegs {
    pub(crate) fn break_address(&self) -> usize {
        self.csr_era
    }

    pub(crate) fn debug_address(&self) -> usize {
        self.csr_era
    }

    pub(crate) fn update_pc(&mut self, pc: usize) {
        self.csr_era = pc;
    }

    /// Get the return value from the r4(a0) registers.
    pub fn first_ret_value(&self) -> usize {
        self.regs[4]
    }

    /// Get the return value from the r5(a1) registers.
    pub fn second_ret_value(&self) -> usize {
        self.regs[5]
    }

    /// Get the arguments from the registers according to the LoongArch64 calling convention.
    pub fn args(&self) -> [usize; 8] {
        [
            self.regs[4],
            self.regs[5],
            self.regs[6],
            self.regs[7],
            self.regs[8],
            self.regs[9],
            self.regs[10],
            self.regs[11],
        ]
    }
}

/// Set up a single step for the given address.
/// This function updates the program counter (PC) to the specified address.
pub(crate) fn setup_single_step(pt_regs: &mut PtRegs, single_step_address: usize) {
    pt_regs.update_pc(single_step_address);
}

/// Clear the single step for the given address.
///
/// This function updates the program counter (PC) to the specified address.
pub(crate) fn clear_single_step(pt_regs: &mut PtRegs, single_step_address: usize) {
    pt_regs.update_pc(single_step_address);
}

#[unsafe(naked)]
pub(crate) unsafe extern "C" fn arch_rethook_trampoline<
    L: RawMutex + 'static,
    F: KprobeAuxiliaryOps + 'static,
>() {
    core::arch::naked_asm!(
        "addi.d $sp, $sp, -{pt_regs_size}",
        "st.d $ra, $sp, 1*8",
        "st.d $tp, $sp, 2*8",
        "st.d $a0, $sp, 4*8",
        "st.d $a1, $sp, 5*8",
        "st.d $a2, $sp, 6*8",
        "st.d $a3, $sp, 7*8",
        "st.d $a4, $sp, 8*8",
        "st.d $a5, $sp, 9*8",
        "st.d $a6, $sp, 10*8",
        "st.d $a7, $sp, 11*8",
        "st.d $t0, $sp, 12*8",
        "st.d $t1, $sp, 13*8",
        "st.d $t2, $sp, 14*8",
        "st.d $t3, $sp, 15*8",
        "st.d $t4, $sp, 16*8",
        "st.d $t5, $sp, 17*8",
        "st.d $t6, $sp, 18*8",
        "st.d $t7, $sp, 19*8",
        "st.d $t8, $sp, 20*8",
        "st.d $r21, $sp, 21*8",
        "st.d $fp, $sp, 22*8",
        "st.d $s0, $sp, 23*8",
        "st.d $s1, $sp, 24*8",
        "st.d $s2, $sp, 25*8",
        "st.d $s3, $sp, 26*8",
        "st.d $s4, $sp, 27*8",
        "st.d $s5, $sp, 28*8",
        "st.d $s6, $sp, 29*8",
        "st.d $s7, $sp, 30*8",
        "st.d $s8, $sp, 31*8",

        "addi.d $t0, $sp, {pt_regs_size}",
        "st.d $t0, $sp, 3*8", // sp
        "move $a0, $sp", // pt_regs pointer

        "bl {callback}",

        "move $ra, $a0", // Restore return address
        "ld.d $tp, $sp, 2*8",
        "ld.d $a0, $sp, 4*8",
        "ld.d $a1, $sp, 5*8",
        "ld.d $a2, $sp, 6*8",
        "ld.d $a3, $sp, 7*8",
        "ld.d $a4, $sp, 8*8",
        "ld.d $a5, $sp, 9*8",
        "ld.d $a6, $sp, 10*8",
        "ld.d $a7, $sp, 11*8",
        "ld.d $t0, $sp, 12*8",
        "ld.d $t1, $sp, 13*8",
        "ld.d $t2, $sp, 14*8",
        "ld.d $t3, $sp, 15*8",
        "ld.d $t4, $sp, 16*8",
        "ld.d $t5, $sp, 17*8",
        "ld.d $t6, $sp, 18*8",
        "ld.d $t7, $sp, 19*8",
        "ld.d $t8, $sp, 20*8",
        "ld.d $r21, $sp, 21*8",
        "ld.d $fp, $sp, 22*8",
        "ld.d $s0, $sp, 23*8",
        "ld.d $s1, $sp, 24*8",
        "ld.d $s2, $sp, 25*8",
        "ld.d $s3, $sp, 26*8",
        "ld.d $s4, $sp, 27*8",
        "ld.d $s5, $sp, 28*8",
        "ld.d $s6, $sp, 29*8",
        "ld.d $s7, $sp, 30*8",
        "ld.d $s8, $sp, 31*8",
        "addi.d $sp, $sp, {pt_regs_size}",
        "jr $ra",
        pt_regs_size = const core::mem::size_of::<PtRegs>(),
        callback = sym arch_rethook_trampoline_callback::<L, F>,
    )
}

pub(crate) fn arch_rethook_trampoline_callback<
    L: RawMutex + 'static,
    F: KprobeAuxiliaryOps + 'static,
>(
    pt_regs: &mut PtRegs,
) -> usize {
    rethook_trampoline_handler::<L, F>(pt_regs, 0)
}

pub(crate) fn arch_rethook_fixup_return(_pt_regs: &mut PtRegs, _correct_ret_addr: usize) {
    // Set the return address to the correct one
    // pt_regs.ra = correct_ret_addr as usize; // we don't need to set ra,
}

/// Prepare the kretprobe instance for the rethook.
pub(crate) fn arch_rethook_prepare<L: RawMutex + 'static, F: KprobeAuxiliaryOps + 'static>(
    kretprobe_instance: &mut RetprobeInstance,
    pt_regs: &mut PtRegs,
) {
    // Prepare the kretprobe instance for the rethook
    kretprobe_instance.ret_addr = pt_regs.regs[1];
    kretprobe_instance.frame = 0; // fp
    pt_regs.regs[1] = arch_rethook_trampoline::<L, F> as *const () as _; // Set the return address to the trampoline
}
