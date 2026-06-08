use alloc::sync::Arc;
use core::{
    fmt::Debug,
    ops::{Deref, DerefMut},
    sync::atomic::AtomicUsize,
};

use lock_api::RawMutex;

use super::ExecMemType;
use crate::{KprobeAuxiliaryOps, KprobeOps, ProbeBasic, ProbeBuilder, ProbeInstallError};
const EBREAK_INST: u32 = 0x00100073; // ebreak
const C_EBREAK_INST: u32 = 0x9002; // c.ebreak
const INSN_LENGTH_MASK: u16 = 0x3;
const INSN_LENGTH_32: u16 = 0x3;

/// The Probe structure.
pub struct Probe<L: RawMutex + 'static, F: KprobeAuxiliaryOps> {
    basic: ProbeBasic<L>,
    point: Arc<Rv64ProbePoint<F>>,
}

#[derive(Debug)]
enum OpcodeTy {
    Inst16(u16),
    Inst32(u32),
}

/// The probe point structure for RISC-V architecture.
#[derive(Debug)]
pub struct Rv64ProbePoint<F: KprobeAuxiliaryOps> {
    addr: usize,
    old_instruction: OpcodeTy,
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
    /// Get the probe point of the Probe.
    pub fn probe_point(&self) -> &Arc<Rv64ProbePoint<F>> {
        &self.point
    }
}

impl<L: RawMutex + 'static, F: KprobeAuxiliaryOps> Debug for Probe<L, F> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Probe")
            .field("basic", &self.basic)
            .field("point", &self.point)
            .finish()
    }
}

impl<F: KprobeAuxiliaryOps> Drop for Rv64ProbePoint<F> {
    fn drop(&mut self) {
        let address = self.addr;
        match self.old_instruction {
            OpcodeTy::Inst16(inst_16) => {
                F::set_writeable_for_address(address, 2, self.user_pid, |ptr| unsafe {
                    core::ptr::write(ptr as *mut u16, inst_16);
                });
            }
            OpcodeTy::Inst32(inst_32) => {
                F::set_writeable_for_address(address, 4, self.user_pid, |ptr| unsafe {
                    core::ptr::write(ptr as *mut u32, inst_32);
                });
            }
        }
        // Free the dynamic user pointer if it was allocated
        let dyn_ptr = self.dynamic_user_ptr();
        if dyn_ptr != 0 {
            F::free_user_exec_memory(self.user_pid, dyn_ptr as *mut u8);
        }
        log::trace!("Kprobe::uninstall: address: {:#x}", address,);
    }
}

impl<F: KprobeAuxiliaryOps> ProbeBuilder<F> {
    /// Install the probe and return the probe and its probe point.
    pub fn install<L: RawMutex + 'static>(
        self,
    ) -> Result<(Probe<L, F>, Arc<Rv64ProbePoint<F>>), ProbeInstallError> {
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
    fn replace_inst(&self) -> Result<Arc<Rv64ProbePoint<F>>, ProbeInstallError> {
        let address = self.symbol_addr + self.offset;
        if address & 0x1 != 0 {
            return Err(ProbeInstallError::InvalidAddress);
        }

        let mut inst_16 = 0u16;
        F::copy_memory(
            address as *const u8,
            &mut inst_16 as *mut u16 as *mut u8,
            2,
            self.user_pid,
        );
        // See <https://elixir.bootlin.com/linux/v6.10.2/source/arch/riscv/kernel/probes/kprobes.c#L68>
        let is_inst_16 = (inst_16 & INSN_LENGTH_MASK) != INSN_LENGTH_32;

        let inst_tmp_ptr = super::alloc_exec_memory::<F>(self.user_pid);

        let old_instruction;
        if is_inst_16 {
            validate_inst16(inst_16)?;
            old_instruction = OpcodeTy::Inst16(inst_16);
            unsafe {
                F::set_writeable_for_address(address, 2, self.user_pid, |ptr| {
                    core::ptr::write(ptr as *mut u16, C_EBREAK_INST as u16);
                });
                // inst_16 :0-16
                // c.ebreak:16-32
                core::ptr::write(inst_tmp_ptr.as_ptr() as *mut u16, inst_16);
                core::ptr::write(
                    (inst_tmp_ptr.as_ptr() as usize + 2) as *mut u16,
                    C_EBREAK_INST as u16,
                );
            }
        } else {
            if address & 0x3 != 0 {
                return Err(ProbeInstallError::InvalidAddress);
            }
            let mut inst_32 = 0u32;
            F::copy_memory(
                address as *const u8,
                &mut inst_32 as *mut u32 as *mut u8,
                4,
                self.user_pid,
            );
            validate_inst32(inst_32)?;
            old_instruction = OpcodeTy::Inst32(inst_32);
            unsafe {
                F::set_writeable_for_address(address, 4, self.user_pid, |ptr| {
                    core::ptr::write(ptr as *mut u32, EBREAK_INST);
                });
                // inst_32 :0-32
                // ebreak  :32-64
                core::ptr::write(inst_tmp_ptr.as_ptr() as *mut u32, inst_32);
                core::ptr::write(
                    (inst_tmp_ptr.as_ptr() as usize + 4) as *mut u32,
                    EBREAK_INST,
                );
            }
        }

        let point = Rv64ProbePoint {
            old_instruction,
            old_instruction_ptr: inst_tmp_ptr,
            addr: address,
            user_pid: self.user_pid,
            dynamic_user_ptr: AtomicUsize::new(0),
            _marker: core::marker::PhantomData,
        };

        log::trace!(
            "Kprobe::install: address: {:#x}, func_name: {:?}, opcode: {:x?}",
            address,
            self.symbol,
            point.old_instruction
        );
        Ok(Arc::new(point))
    }
}

// References:
// - RISC-V unprivileged ISA manual:
//   https://docs.riscv.org/reference/isa/_attachments/riscv-unprivileged.pdf
// - Linux RISC-V kprobes instruction classification:
//   https://codebrowser.dev/linux/linux/arch/riscv/kernel/probes/decode-insn.c.html
fn validate_inst16(inst: u16) -> Result<(), ProbeInstallError> {
    if inst == C_EBREAK_INST as u16 {
        return Err(ProbeInstallError::UnsupportedInstruction);
    }

    let quadrant = inst & 0x3;
    let funct3 = (inst >> 13) & 0x7;
    match (quadrant, funct3) {
        // c.jal/c.j/c.beqz/c.bnez use PC-relative control flow.
        (0x1, 0x1 | 0x5 | 0x6 | 0x7) => Err(ProbeInstallError::UnsafeInstruction),
        // c.jr/c.jalr/c.ebreak share quadrant 2, funct3 4 and rs2 == 0.
        (0x2, 0x4) if ((inst >> 2) & 0x1f) == 0 => Err(ProbeInstallError::UnsupportedInstruction),
        _ => Ok(()),
    }
}

fn validate_inst32(inst: u32) -> Result<(), ProbeInstallError> {
    match inst & 0x7f {
        // auipc, jalr, branch, jal, fence, and system are not safe without relocation support.
        0x17 | 0x67 | 0x63 | 0x6f | 0x0f | 0x73 => Err(ProbeInstallError::UnsupportedInstruction),
        _ => Ok(()),
    }
}

impl<F: KprobeAuxiliaryOps> KprobeOps for Rv64ProbePoint<F> {
    fn return_address(&self) -> usize {
        self.addr + self.original_instruction_len()
    }

    fn single_step_address(&self) -> usize {
        self.old_instruction_ptr.as_ptr() as usize
    }

    fn debug_address(&self) -> usize {
        let dynamic_user_ptr = self.dynamic_user_ptr();
        if dynamic_user_ptr == 0 {
            self.old_instruction_ptr.as_ptr() as usize + self.original_instruction_len()
        } else {
            dynamic_user_ptr + self.original_instruction_len()
        }
    }

    fn break_address(&self) -> usize {
        self.addr
    }

    fn dynamic_user_ptr(&self) -> usize {
        self.dynamic_user_ptr
            .load(core::sync::atomic::Ordering::SeqCst)
    }

    /// Set the dynamic user pointer and return the new debug address.
    fn set_dynamic_user_ptr(&self, ptr: usize) -> usize {
        self.dynamic_user_ptr
            .store(ptr, core::sync::atomic::Ordering::SeqCst);
        ptr + self.original_instruction_len()
    }

    fn original_instruction_len(&self) -> usize {
        match self.old_instruction {
            OpcodeTy::Inst16(_) => 2,
            OpcodeTy::Inst32(_) => 4,
        }
    }

    fn instruction_slot_len(&self) -> usize {
        match self.old_instruction {
            OpcodeTy::Inst16(_) => 2 + 2,
            OpcodeTy::Inst32(_) => 4 + 4,
        }
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

/// The register state for RISC-V architecture.
#[repr(C)]
#[derive(Debug, Copy, Clone)]
#[allow(missing_docs)]
pub struct PtRegs {
    pub epc: usize,
    pub ra: usize,
    pub sp: usize,
    pub gp: usize,
    pub tp: usize,
    pub t0: usize,
    pub t1: usize,
    pub t2: usize,
    pub s0: usize,
    pub s1: usize,
    pub a0: usize,
    pub a1: usize,
    pub a2: usize,
    pub a3: usize,
    pub a4: usize,
    pub a5: usize,
    pub a6: usize,
    pub a7: usize,
    pub s2: usize,
    pub s3: usize,
    pub s4: usize,
    pub s5: usize,
    pub s6: usize,
    pub s7: usize,
    pub s8: usize,
    pub s9: usize,
    pub s10: usize,
    pub s11: usize,
    pub t3: usize,
    pub t4: usize,
    pub t5: usize,
    pub t6: usize,
    // Supervisor/Machine CSRs
    pub status: usize,
    pub badaddr: usize,
    pub cause: usize,
    // a0 value before the syscall
    pub orig_a0: usize,
}

impl PtRegs {
    pub(crate) fn break_address(&self) -> usize {
        // for riscv64
        self.epc as _
    }
    pub(crate) fn debug_address(&self) -> usize {
        self.epc as _
    }

    pub(crate) fn update_pc(&mut self, pc: usize) {
        self.epc = pc as _;
    }

    /// Get the return value from the a0 register.
    pub fn first_ret_value(&self) -> usize {
        self.a0
    }

    /// Get the return value from the a1 register.
    pub fn second_ret_value(&self) -> usize {
        self.a1
    }

    /// Get the arguments from the registers according to the RISC-V calling convention.
    pub fn args(&self) -> [usize; 6] {
        [self.a0, self.a1, self.a2, self.a3, self.a4, self.a5]
    }
}

#[unsafe(naked)]
pub(crate) unsafe extern "C" fn arch_rethook_trampoline<
    L: RawMutex + 'static,
    F: KprobeAuxiliaryOps + 'static,
>() {
    core::arch::naked_asm!(
        "addi sp, sp, -{pt_size}",
        //  Save all general-purpose registers
        "sd ra, 8(sp)",
        "sd gp, 24(sp)",
        "sd tp, 32(sp)",
        "sd t0, 40(sp)",
        "sd t1, 48(sp)",
        "sd t2, 56(sp)",
        "sd s0, 64(sp)",
        "sd s1, 72(sp)",
        "sd a0, 80(sp)",
        "sd a1, 88(sp)",
        "sd a2, 96(sp)",
        "sd a3, 104(sp)",
        "sd a4, 112(sp)",
        "sd a5, 120(sp)",
        "sd a6, 128(sp)",
        "sd a7, 136(sp)",
        "sd s2, 144(sp)",
        "sd s3, 152(sp)",
        "sd s4, 160(sp)",
        "sd s5, 168(sp)",
        "sd s6, 176(sp)",
        "sd s7, 184(sp)",
        "sd s8, 192(sp)",
        "sd s9, 200(sp)",
        "sd s10, 208(sp)",
        "sd s11, 216(sp)",
        "sd t3, 224(sp)",
        "sd t4, 232(sp)",
        "sd t5, 240(sp)",
        "sd t6, 248(sp)",
        "mv a0, sp",
        "call {callback}",
        "mv ra, a0",
        // Restore all general-purpose registers
        "ld gp, 24(sp)",
        "ld tp, 32(sp)",
        "ld t0, 40(sp)",
        "ld t1, 48(sp)",
        "ld t2, 56(sp)",
        "ld s0, 64(sp)",
        "ld s1, 72(sp)",
        "ld a0, 80(sp)",
        "ld a1, 88(sp)",
        "ld a2, 96(sp)",
        "ld a3, 104(sp)",
        "ld a4, 112(sp)",
        "ld a5, 120(sp)",
        "ld a6, 128(sp)",
        "ld a7, 136(sp)",
        "ld s2, 144(sp)",
        "ld s3, 152(sp)",
        "ld s4, 160(sp)",
        "ld s5, 168(sp)",
        "ld s6, 176(sp)",
        "ld s7, 184(sp)",
        "ld s8, 192(sp)",
        "ld s9, 200(sp)",
        "ld s10, 208(sp)",
        "ld s11, 216(sp)",
        "ld t3, 224(sp)",
        "ld t4, 232(sp)",
        "ld t5, 240(sp)",
        "ld t6, 248(sp)",
        "addi sp, sp, {pt_size}",
        "ret",
        pt_size = const core::mem::size_of::<PtRegs>(),
        callback = sym arch_rethook_trampoline_callback::<L,F>,
    )
}

pub(crate) fn arch_rethook_trampoline_callback<
    L: RawMutex + 'static,
    F: KprobeAuxiliaryOps + 'static,
>(
    pt_regs: &mut PtRegs,
) -> usize {
    super::retprobe::rethook_trampoline_handler::<L, F>(pt_regs, pt_regs.s0)
}

pub(crate) fn arch_rethook_fixup_return(_pt_regs: &mut PtRegs, _correct_ret_addr: usize) {
    // Set the return address to the correct one
    // pt_regs.ra = correct_ret_addr; // we don't need to set ra,
}

/// Prepare the kretprobe instance for the rethook.
pub(crate) fn arch_rethook_prepare<L: RawMutex + 'static, F: KprobeAuxiliaryOps + 'static>(
    kretprobe_instance: &mut super::retprobe::RetprobeInstance,
    pt_regs: &mut PtRegs,
) {
    // Prepare the kretprobe instance for the rethook
    kretprobe_instance.ret_addr = pt_regs.ra;
    kretprobe_instance.frame = pt_regs.s0; // fp
    pt_regs.ra = arch_rethook_trampoline::<L, F> as *const () as _; // Set the return address to the trampoline
}
