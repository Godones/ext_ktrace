use alloc::{string::ToString, sync::Arc};
use core::{
    fmt::Debug,
    ops::{Deref, DerefMut},
    sync::atomic::AtomicUsize,
};

use lock_api::RawMutex;
use yaxpeax_arch::LengthedInstruction;
use yaxpeax_x86::amd64::{Instruction, Opcode, Operand, RegSpec};

use crate::{
    KprobeAuxiliaryOps, KprobeOps, ProbeBasic, ProbeBuilder, ProbeInstallError, arch::ExecMemType,
};

const EBREAK_INST: u8 = 0xcc; // x86_64: 0xcc
const MAX_INSTRUCTION_SIZE: usize = 15; // x86_64 max instruction length

/// The x86_64 implementation of Probe.
pub struct Probe<L: RawMutex + 'static, F: KprobeAuxiliaryOps> {
    basic: ProbeBasic<L>,
    point: Arc<X86ProbePoint<F>>,
}

/// The probe point for x86_64 architecture.
#[derive(Debug)]
pub struct X86ProbePoint<F: KprobeAuxiliaryOps> {
    addr: usize,
    old_instruction_ptr: ExecMemType<F>,
    old_instruction_len: usize,
    user_pid: Option<i32>,
    // Dynamic user pointer for handling user space instruction pointer adjustments
    dynamic_user_ptr: AtomicUsize,
    _marker: core::marker::PhantomData<F>,
}

impl<F: KprobeAuxiliaryOps> Drop for X86ProbePoint<F> {
    fn drop(&mut self) {
        let address = self.addr;
        F::set_writeable_for_address(address, self.old_instruction_len, self.user_pid, |ptr| {
            // Restore the original instruction at the probe point
            unsafe {
                core::ptr::copy_nonoverlapping(
                    self.old_instruction_ptr.as_ptr(),
                    ptr,
                    self.old_instruction_len,
                );
            }
        });

        // Free the dynamic user pointer if it was allocated
        let dyn_ptr = self.dynamic_user_ptr();
        if dyn_ptr != 0 {
            F::free_user_exec_memory(self.user_pid, dyn_ptr as *mut u8);
        }
        log::trace!(
            "X86KprobePoint::drop: Restored instruction at address: {:#x}",
            address
        );
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

impl<F: KprobeAuxiliaryOps> ProbeBuilder<F> {
    pub(crate) fn install<L: RawMutex + 'static>(
        self,
    ) -> Result<(Probe<L, F>, Arc<X86ProbePoint<F>>), ProbeInstallError> {
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
    fn replace_inst(&self) -> Result<Arc<X86ProbePoint<F>>, ProbeInstallError> {
        let address = self.symbol_addr + self.offset;
        let inst_tmp = super::alloc_exec_memory::<F>(self.user_pid);

        F::copy_memory(
            address as *const u8,
            inst_tmp.as_ptr(),
            MAX_INSTRUCTION_SIZE,
            self.user_pid,
        );

        let buf = unsafe { core::slice::from_raw_parts(inst_tmp.as_ptr(), MAX_INSTRUCTION_SIZE) };

        let decoder = yaxpeax_x86::amd64::InstDecoder::default();
        let inst = decoder
            .decode_slice(buf)
            .map_err(|_| ProbeInstallError::DecodeFailed)?;
        validate_instruction(&inst)?;
        let len = inst.len().to_const();
        log::trace!("inst: {:?}, len: {:?}", inst.to_string(), len);

        let point = Arc::new(X86ProbePoint {
            addr: address,
            old_instruction_ptr: inst_tmp,
            old_instruction_len: len as usize,
            user_pid: self.user_pid,
            dynamic_user_ptr: AtomicUsize::new(0),
            _marker: core::marker::PhantomData,
        });

        F::set_writeable_for_address(address, len as usize, self.user_pid, |ptr| unsafe {
            core::ptr::write(ptr, EBREAK_INST);
        });
        log::trace!(
            "Kprobe::install: address: {:#x}, func_name: {:?}",
            address,
            self.symbol
        );
        Ok(point)
    }
}

// References:
// - Intel SDM, Vol. 2 instruction reference:
//   https://www.intel.com/content/www/us/en/developer/articles/technical/intel-sdm.html
// - Linux x86 kprobes decode/validation:
//   https://codebrowser.dev/linux/linux/arch/x86/kernel/kprobes/core.c.html
fn validate_instruction(inst: &Instruction) -> Result<(), ProbeInstallError> {
    let opcode = inst.opcode();
    if opcode.is_jcc()
        || matches!(
            opcode,
            Opcode::Invalid
                | Opcode::CALL
                | Opcode::CALLF
                | Opcode::JMP
                | Opcode::JMPF
                | Opcode::JMPE
                | Opcode::RETURN
                | Opcode::RETF
                | Opcode::IRET
                | Opcode::IRETD
                | Opcode::IRETQ
                | Opcode::INT
                | Opcode::INTO
                | Opcode::LOOP
                | Opcode::LOOPZ
                | Opcode::LOOPNZ
                | Opcode::JRCXZ
                | Opcode::SYSCALL
                | Opcode::SYSRET
                | Opcode::SWAPGS
                | Opcode::HLT
                | Opcode::CLI
                | Opcode::STI
                | Opcode::UD0
                | Opcode::UD1
                | Opcode::UD2
                | Opcode::XBEGIN
        )
    {
        return Err(ProbeInstallError::UnsupportedInstruction);
    }

    for operand_index in 0..inst.operand_count() {
        if operand_uses_pc(inst.operand(operand_index)) {
            return Err(ProbeInstallError::UnsafeInstruction);
        }
    }

    Ok(())
}

fn operand_uses_pc(operand: Operand) -> bool {
    fn is_pc(reg: RegSpec) -> bool {
        reg == RegSpec::rip() || reg == RegSpec::eip()
    }

    match operand {
        Operand::MemDeref { base }
        | Operand::Disp { base, .. }
        | Operand::MemDerefMasked { base, .. }
        | Operand::DispMasked { base, .. } => is_pc(base),
        Operand::MemIndexScale { index, .. }
        | Operand::MemIndexScaleDisp { index, .. }
        | Operand::MemIndexScaleMasked { index, .. }
        | Operand::MemIndexScaleDispMasked { index, .. } => is_pc(index),
        Operand::MemBaseIndexScale { base, index, .. }
        | Operand::MemBaseIndexScaleDisp { base, index, .. }
        | Operand::MemBaseIndexScaleMasked { base, index, .. }
        | Operand::MemBaseIndexScaleDispMasked { base, index, .. } => is_pc(base) || is_pc(index),
        _ => false,
    }
}

impl<L: RawMutex + 'static, F: KprobeAuxiliaryOps> Probe<L, F> {
    /// Get the probe point associated with this probe.
    pub fn probe_point(&self) -> &Arc<X86ProbePoint<F>> {
        &self.point
    }
}

impl<F: KprobeAuxiliaryOps> KprobeOps for X86ProbePoint<F> {
    fn return_address(&self) -> usize {
        self.addr + self.original_instruction_len()
    }

    fn single_step_address(&self) -> usize {
        self.old_instruction_ptr.as_ptr() as usize
    }

    fn debug_address(&self) -> usize {
        let dynamic_user_ptr = self.dynamic_user_ptr();
        if dynamic_user_ptr != 0 {
            dynamic_user_ptr + self.original_instruction_len()
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
        self.old_instruction_len
    }

    fn instruction_slot_len(&self) -> usize {
        self.old_instruction_len
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
    pt_regs.set_single_step(true);
}

/// Clear the single step for the given address.
///
/// This function updates the program counter (PC) to the specified address.
pub(crate) fn clear_single_step(pt_regs: &mut PtRegs, single_step_address: usize) {
    pt_regs.update_pc(single_step_address);
    pt_regs.set_single_step(false);
}

/// The CPU register state for x86_64 architecture.
#[repr(C)]
#[derive(Debug, Copy, Clone)]
#[allow(missing_docs)]
pub struct PtRegs {
    pub r15: usize,
    pub r14: usize,
    pub r13: usize,
    pub r12: usize,
    pub rbp: usize,
    pub rbx: usize,
    pub r11: usize,
    pub r10: usize,
    pub r9: usize,
    pub r8: usize,
    pub rax: usize,
    pub rcx: usize,
    pub rdx: usize,
    pub rsi: usize,
    pub rdi: usize,
    // On syscall entry, this is syscall#. On CPU exception, this is error code.
    // On hw interrupt, it's IRQ number
    pub orig_rax: usize,
    pub rip: usize,
    pub cs: usize,
    pub rflags: usize,
    pub rsp: usize,
    pub ss: usize,
}

impl PtRegs {
    pub(crate) fn break_address(&self) -> usize {
        self.rip - 1 // The breakpoint instruction is at the address of rip - 1
    }

    pub(crate) fn debug_address(&self) -> usize {
        self.rip // The debug address is the current instruction pointer
    }

    pub(crate) fn update_pc(&mut self, pc: usize) {
        self.rip = pc as _;
    }

    pub(crate) fn set_single_step(&mut self, enable: bool) {
        if enable {
            self.rflags |= 0x100;
        } else {
            self.rflags &= !0x100;
        }
    }

    pub(crate) fn sp(&self) -> usize {
        self.rsp
    }

    /// Get the return value from the rax register.
    pub fn first_ret_value(&self) -> usize {
        self.rax
    }

    /// Get the return value from the rdx register.
    pub fn second_ret_value(&self) -> usize {
        self.rdx
    }
}

const KERNEL_DS: usize = 24; // Kernel data segment selector

/// See <https://elixir.bootlin.com/linux/v6.6/source/arch/x86/kernel/rethook.c#L62>
#[unsafe(naked)]
pub(crate) unsafe extern "C" fn arch_rethook_trampoline<
    L: RawMutex + 'static,
    F: KprobeAuxiliaryOps + 'static,
>() {
    core::arch::naked_asm!(
        // Push a fake return address to tell the unwinder it's a kretprobe.
        // TODO: Use the real return address later.
        "pushq $0",
        "pushq ${kernel_data_segment}", // fake ss
        // Save the 'sp - 16', this will be fixed later.
        "pushq %rsp",
        "pushfq", // rflags

        // SAVE_REGS_STRING
        "subq $24, %rsp", // skip cs, ip, orig_ax
        "pushq %rdi",
        "pushq %rsi",
        "pushq %rdx",
        "pushq %rcx",
        "pushq %rax",
        "pushq %r8",
        "pushq %r9",
        "pushq %r10",
        "pushq %r11",
        "pushq %rbx",
        "pushq %rbp",
        "pushq %r12",
        "pushq %r13",
        "pushq %r14",
        "pushq %r15",
        // ENCODE_FRAME_POINTER
        // "lea 1(%rsp), %rbp",
        "movq %rsp, %rdi",
        "call {rethook_trampoline_callback}",
        // RESTORE_REGS_STRING
        "popq %r15",
        "popq %r14",
        "popq %r13",
        "popq %r12",
        "popq %rbp",
        "popq %rbx",
        "popq %r11",
        "popq %r10",
        "popq %r9",
        "popq %r8",
        "popq %rax",
        "popq %rcx",
        "popq %rdx",
        "popq %rsi",
        "popq %rdi",
        // Skip orig_ax, ip, cs
        "addq $24, %rsp",

        // In the callback function, 'regs->flags' is copied to 'regs->ss'.
        "addq $16, %rsp",

        "popfq",
        "ret",
        kernel_data_segment = const KERNEL_DS, // Kernel data segment
        // arch_rethook_trampoline = sym arch_rethook_trampoline::<L,F>,
        rethook_trampoline_callback = sym arch_rethook_trampoline_callback::<L, F>,
        options(att_syntax)
    )
}

pub(crate) fn arch_rethook_trampoline_callback<
    L: RawMutex + 'static,
    F: KprobeAuxiliaryOps + 'static,
>(
    pt_regs: &mut PtRegs,
) -> usize {
    pt_regs.rip = arch_rethook_trampoline::<L, F> as *const () as usize; // Set return address to trampoline
    pt_regs.orig_rax = usize::MAX;
    pt_regs.rsp += 16; // Adjust rsp to remove the fake return address

    let pt_regs_pointer = unsafe { (pt_regs as *mut PtRegs).add(1) as *mut usize };

    let correct_ret_addr =
        super::retprobe::rethook_trampoline_handler::<L, F>(pt_regs, pt_regs_pointer as _);
    pt_regs.ss = pt_regs.rflags; // Copy eflags to ss
    correct_ret_addr
}

/// Fix up the return address in the pt_regs after a rethook.
pub(crate) fn arch_rethook_fixup_return(pt_regs: &mut PtRegs, correct_ret_addr: usize) {
    let pt_regs_pointer = unsafe { (pt_regs as *mut PtRegs).add(1) as *mut usize };
    unsafe {
        // Replace fake return address with real one.
        *pt_regs_pointer = correct_ret_addr;
    }
}

/// Prepare the kretprobe instance for the rethook.
pub(crate) fn arch_rethook_prepare<L: RawMutex + 'static, F: KprobeAuxiliaryOps + 'static>(
    kretprobe_instance: &mut super::retprobe::RetprobeInstance,
    pt_regs: &mut PtRegs,
) {
    let sp = pt_regs.sp();
    let stack = unsafe { &mut *(sp as *mut usize) };
    // Prepare the kretprobe instance for the rethook
    // Get the return address from the stack
    kretprobe_instance.ret_addr = *stack;
    kretprobe_instance.frame = sp;
    // Set the return address to the trampoline
    *stack = arch_rethook_trampoline::<L, F> as *const () as usize;
}
