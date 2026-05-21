use alloc::sync::Arc;
use core::{
    arch::naked_asm,
    fmt::Debug,
    mem::offset_of,
    ops::{Deref, DerefMut},
    sync::atomic::AtomicUsize,
};

use lock_api::RawMutex;

use super::{
    ExecMemType, KprobeAuxiliaryOps,
    retprobe::{RetprobeInstance, rethook_trampoline_handler},
};
use crate::{KprobeOps, ProbeBasic, ProbeBuilder, ProbeInstallError};

/// See <https://elixir.bootlin.com/linux/v6.6/source/arch/arm64/include/uapi/asm/ptrace.h#L58>
const PSR_V_BIT_POS: usize = 0x10000000;
const PSR_C_BIT_POS: usize = 0x20000000;
const PSR_Z_BIT_POS: usize = 0x40000000;
const PSR_N_BIT_POS: usize = 0x80000000;

const KPROBES_BRK_IMM: u32 = 0x004;
// const UPROBES_BRK_IMM: u32 = 0x005;
const KPROBES_BRK_SS_IMM: u32 = 0x006;

/// See <https://elixir.bootlin.com/linux/v6.6/source/arch/arm64/include/asm/insn-def.h#L15>
const AARCH64_BREAK_MON: u32 = 0xd4200000;

/// kprobes BRK opcodes with ESR encoding
/// See <https://elixir.bootlin.com/linux/v6.6/source/arch/arm64/include/asm/debug-monitors.h#L43>
const BRK64_OPCODE_KPROBES: u32 = AARCH64_BREAK_MON | (KPROBES_BRK_IMM << 5);
/// single step BRK opcode with ESR encoding
const BRK64_OPCODE_KPROBES_SS: u32 = AARCH64_BREAK_MON | (KPROBES_BRK_SS_IMM << 5);

const BRK64_INST_LEN: usize = 4;

/// The kprobe structure.
pub struct Probe<L: RawMutex + 'static, F: KprobeAuxiliaryOps> {
    basic: ProbeBasic<L>,
    point: Arc<AArch64ProbePoint<F>>,
}

/// The kprobe point structure for aarch64 architecture.
#[derive(Debug)]
pub struct AArch64ProbePoint<F: KprobeAuxiliaryOps> {
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
    pub fn probe_point(&self) -> &Arc<AArch64ProbePoint<F>> {
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

impl<F: KprobeAuxiliaryOps> Drop for AArch64ProbePoint<F> {
    fn drop(&mut self) {
        let address = self.addr;
        let inst_tmp_ptr = self.old_instruction_ptr.as_ptr();
        F::set_writeable_for_address(address, BRK64_INST_LEN, self.user_pid, |ptr| {
            // Restore the original instruction at the probe address
            unsafe {
                core::ptr::copy_nonoverlapping(inst_tmp_ptr as *const u8, ptr, BRK64_INST_LEN)
            };
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
    ) -> Result<(Probe<L, F>, Arc<AArch64ProbePoint<F>>), ProbeInstallError> {
        let probe_point = match &self.probe_point {
            Some(point) => point.clone(),
            None => self.replace_inst()?,
        };
        let kprobe = Probe {
            basic: ProbeBasic::from(self),
            point: probe_point.clone(),
        };
        Ok((kprobe, probe_point))
    }

    /// Replace the instruction at the specified address with a breakpoint instruction.
    fn replace_inst(&self) -> Result<Arc<AArch64ProbePoint<F>>, ProbeInstallError> {
        let address = self.symbol_addr + self.offset;
        if address & (BRK64_INST_LEN - 1) != 0 {
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
            F::set_writeable_for_address(address, BRK64_INST_LEN, self.user_pid, |ptr| {
                // Replace the original instruction with the breakpoint instruction
                core::ptr::write(ptr as *mut u32, BRK64_OPCODE_KPROBES);
            });
            // inst_32 :0-32
            // break  :32-64
            core::ptr::write(inst_tmp_ptr.as_ptr() as *mut u32, inst_32);
            core::ptr::write(
                (inst_tmp_ptr.as_ptr() as usize + 4) as *mut u32,
                BRK64_OPCODE_KPROBES_SS,
            );
        }

        let point = AArch64ProbePoint {
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
// - Arm A64 base instruction reference:
//   https://developer.arm.com/documentation/ddi0487/latest
// - Armv8-A ISA overview, PC-relative and branch behavior:
//   https://developer.arm.com/-/media/Arm%20Developer%20Community/PDF/Learn%20the%20Architecture/Armv8-A%20Instruction%20Set%20Architecture.pdf
// - Linux arm64 kprobes decode path:
//   https://codebrowser.dev/linux/linux/arch/arm64/kernel/probes/kprobes.c.html
fn validate_instruction(inst: u32) -> Result<(), ProbeInstallError> {
    if inst == BRK64_OPCODE_KPROBES || inst == BRK64_OPCODE_KPROBES_SS {
        return Err(ProbeInstallError::UnsupportedInstruction);
    }

    let is_unconditional_branch = (inst & 0x7c00_0000) == 0x1400_0000;
    let is_compare_branch = (inst & 0x7e00_0000) == 0x3400_0000;
    let is_test_branch = (inst & 0x7e00_0000) == 0x3600_0000;
    let is_conditional_branch = (inst & 0xff00_0010) == 0x5400_0000;
    let is_register_branch = matches!(inst & 0xffff_fc1f, 0xd61f_0000 | 0xd63f_0000 | 0xd65f_0000);
    let is_pc_relative_address = (inst & 0x9f00_0000) == 0x1000_0000;
    let is_literal_load = (inst & 0x3b00_0000) == 0x1800_0000;
    let is_exception_or_system = matches!(inst & 0xff00_0000, 0xd400_0000 | 0xd500_0000);

    if is_unconditional_branch
        || is_compare_branch
        || is_test_branch
        || is_conditional_branch
        || is_register_branch
        || is_pc_relative_address
        || is_literal_load
        || is_exception_or_system
    {
        return Err(ProbeInstallError::UnsafeInstruction);
    }

    Ok(())
}

impl<F: KprobeAuxiliaryOps> KprobeOps for AArch64ProbePoint<F> {
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
        BRK64_INST_LEN
    }

    fn instruction_slot_len(&self) -> usize {
        BRK64_INST_LEN * 2
    }

    fn pid(&self) -> Option<i32> {
        self.user_pid
    }
}

/// Set up a single step for the given address.
///
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

/// The register state at the time of the probe.
///
/// See <https://elixir.bootlin.com/linux/v6.6/source/arch/arm64/include/asm/ptrace.h#L178>
#[repr(C)]
#[derive(Debug, Copy, Clone)]
#[repr(align(8))]
#[allow(missing_docs)]
pub struct PtRegs {
    pub regs: [u64; 31],
    pub sp: u64,
    pub pc: u64,
    pub pstate: u64,
    pub orig_x0: u64,
    // for little endian
    pub syscallno: i32,
    pub unused2: u32,
}

const FP_OFFSET: usize = offset_of!(PtRegs, regs) + 29 * core::mem::size_of::<u64>();

impl PtRegs {
    pub(crate) fn break_address(&self) -> usize {
        self.pc as usize
    }

    pub(crate) fn debug_address(&self) -> usize {
        self.pc as usize
    }

    pub(crate) fn update_pc(&mut self, pc: usize) {
        self.pc = pc as u64;
    }

    /// Get the return value from the r4(a0) registers.
    pub fn first_ret_value(&self) -> usize {
        self.regs[0] as usize
    }

    /// Get the return value from the r5(a1) registers.
    pub fn second_ret_value(&self) -> usize {
        self.regs[1] as usize
    }
}

/// See <https://elixir.bootlin.com/linux/v6.6/source/arch/arm64/kernel/probes/kprobes_trampoline.S#L10>
#[unsafe(naked)]
pub(crate) unsafe extern "C" fn arch_rethook_trampoline<
    L: RawMutex + 'static,
    F: KprobeAuxiliaryOps + 'static,
>() {
    naked_asm!(
        "sub sp, sp, {pt_regs_size}",
        // save all base regs
        "stp x0, x1, [sp]",
        "stp x2, x3, [sp, 2*8]",
        "stp x4, x5, [sp, 4*8]",
        "stp x6, x7, [sp, 6*8]",
        "stp x8, x9, [sp, 8*8]",
        "stp x10, x11, [sp, 10*8]",
        "stp x12, x13, [sp, 12*8]",
        "stp x14, x15, [sp, 14*8]",
        "stp x16, x17, [sp, 16*8]",
        "stp x18, x19, [sp, 18*8]",
        "stp x20, x21, [sp, 20*8]",
        "stp x22, x23, [sp, 22*8]",
        "stp x24, x25, [sp, 24*8]",
        "stp x26, x27, [sp, 26*8]",
        "stp x28, x29, [sp, 28*8]",
        // calculate PtRegs pointer
        "add x0, sp, {pt_regs_size}",
        // save lr(x30) and pt_regs pointer
        "stp lr, x0, [sp, 30*8]",
        // Construct a useful saved PSTATE
        "mrs x0, nzcv",
        "mrs x1, daif",
        "orr x0, x0, x1",
        "mrs x1, CurrentEL",
        "orr x0, x0, x1",
        "mrs x1, SPSel",
        "orr x0, x0, x1",
        // save pc and pstate
        "stp xzr, x0, [sp, 32*8]",
        // Setup a frame pointer.
        "add x29, sp, {S_FP}",
        // call the handler
        "mov x0, sp",
        "bl {rethook_trampoline_handler}",

        // Replace trampoline address in lr(x30) with actual orig_ret_addr return address
        "mov lr, x0",
        // restore all base regs
        // The frame pointer (x29) is restored with other registers.
        "ldr x0, [sp, {S_PSTATE}]",
        "and x0, x0, {M_FLAGS}",
        "msr nzcv, x0",
        "ldp x0, x1, [sp]",
        "ldp x2, x3, [sp, 2*8]",
        "ldp x4, x5, [sp, 4*8]",
        "ldp x6, x7, [sp, 6*8]",
        "ldp x8, x9, [sp, 8*8]",
        "ldp x10, x11, [sp, 10*8]",
        "ldp x12, x13, [sp, 12*8]",
        "ldp x14, x15, [sp, 14*8]",
        "ldp x16, x17, [sp, 16*8]",
        "ldp x18, x19, [sp, 18*8]",
        "ldp x20, x21, [sp, 20*8]",
        "ldp x22, x23, [sp, 22*8]",
        "ldp x24, x25, [sp, 24*8]",
        "ldp x26, x27, [sp, 26*8]",
        "ldp x28, x29, [sp, 28*8]",

        "add sp, sp, {pt_regs_size}",
        "ret",
        S_FP = const FP_OFFSET,
        S_PSTATE = const offset_of!(PtRegs, pstate),
        M_FLAGS = const PSR_V_BIT_POS
                | PSR_C_BIT_POS
                | PSR_Z_BIT_POS
                | PSR_N_BIT_POS,
        pt_regs_size = const core::mem::size_of::<PtRegs>(),
        rethook_trampoline_handler = sym arch_rethook_trampoline_callback::<L, F>,
    )
}

pub(crate) fn arch_rethook_trampoline_callback<
    L: RawMutex + 'static,
    F: KprobeAuxiliaryOps + 'static,
>(
    pt_regs: &mut PtRegs,
) -> usize {
    rethook_trampoline_handler::<L, F>(pt_regs, pt_regs.regs[29] as usize)
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
    // lr(x30) is the return address
    kretprobe_instance.ret_addr = pt_regs.regs[30] as usize;
    kretprobe_instance.frame = pt_regs.regs[29] as usize;
    // Set the return address to the trampoline
    pt_regs.regs[30] = arch_rethook_trampoline::<L, F> as *const () as usize as u64;
}
