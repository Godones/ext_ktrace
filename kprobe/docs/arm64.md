# aarch64架构的一些说明

``` as
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
// save lr and pt_regs pointer
"stp x30, x0, [sp, 30*8]",
// Construct a useful saved PSTATE
"mrs x0, nzcv",
"msr x1, daif",
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
```

对于这段代码中的部分：
``` as
// 读取条件标志寄存器（NZCV flags
mrs x0, nzcv
// 读取中断屏蔽位（DAIF）
mrs x1, daif
// 这些位控制是否屏蔽各种中断，属于 PSTATE 的中高位部分
orr x0, x0, x1
// 读取当前异常级别（CurrentEL）
mrs x1, CurrentEL
orr x0, x0, x1
// 读取 Stack Pointer Select 位，表示当前是使用 SP_EL0 还是 SP_ELx
mrs x1, SPSel
orr x0, x0, x1
// 保存伪造的 PC 和 PSTATE 到栈帧（pt_regs）对应偏移位置
// xzr（恒为 0） → 存放到 [sp + 32*8]：代表 PC。因为此时没有真正的指令地址要保存。
"stp xzr, x0, [sp, 32*8]",
```


对于这段代码中的部分：
``` as
// Setup a frame pointer.
"add x29, sp, {S_FP}",
```

把 x29（在 AArch64 ABI 中约定为帧指针 FP）设为 sp + S_FP。也就是说 x29 指向当前栈（sp）之上某个固定偏移位置 S_FP。

1. 刚刚把通用寄存器保存到了栈上（save_all_base_regs），栈上现在有一个 struct pt_regs 风格的保存区，里面包含了保存的寄存器值（包括上一个 x29、返回地址等）。S_FP 是该保存区中“帧指针字段”相对于 sp 的偏移（也就是 pt_regs 中保存旧 x29 的位置或者帧基地址的位置）。

2. 通过 add x29, sp, #S_FP，trampoline 把 x29 指到这个保存区，使得：

   - 当任意工具（调试器、栈回溯/展开器、或 trampoline 内部函数 trampoline_probe_handler）需要查找调用链或访问调用者的帧信息时，可以通过 x29 找到上一帧的 x29/lr 等。

   - trampoline_probe_handler 在被调用时能看到一个“合理的”帧指针，便于异常处理/堆栈回溯/符号解析等


## Reference
https://armv8-doc.readthedocs.io/en/latest/index.html