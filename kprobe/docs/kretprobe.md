# 如何使用kretprobe获取函数返回值

kretprobe是一种内核探针，允许我们在内核函数返回时执行自定义代码。通过kretprobe，我们可以捕获函数的返回值，并进行相应的处理。但是由于Rust和C语言的常见使用方法不同，可能会导致无法直接得到正确的返回值。

以以下Rust代码示例:
```rust
fn add(x: usize, y: usize) -> usize {
    return x + y;
}
```

这里首先说明一下调用约定：在 x86-64 System V（Linux / macOS 等）下，整数/指针类型的前六个参数分别通过寄存器传递：rdi, rsi, rdx, rcx, r8, r9；第一个函数返回值通过 rax 返回, 第二个返回值通过 rdx 返回。

这段代码产生的汇编代码是:
```asm
0000000000015280 <add>:
   15280:	48 8d 04 37          	lea    (%rdi,%rsi,1),%rax
   15284:	c3                   	ret    
```

也就是说，函数返回值存储在 rax 寄存器中。因此，我们可以直接通过pt_regs结构体中的rax字段来获取返回值。这和C语言的处理是类似的。

但是在内核项目中，大多数的函数的返回值通常用`Result`或者`Option`来表示，这些类型返回值可能会存储在多个寄存器中。例如，`Result<T, E>`类型的返回值通常会使用 rax 和 rdx 两个寄存器来存储。

以以下Rust代码示例:
```rust
pub fn detect_func(x: usize, y: usize, z: Option<usize>) -> Option<usize> {
    if let Some(z) = z {
        Some(x + y + z)
    } else {
        None
    }
}
```
这段代码产生的汇编代码是:
```asm
0000000000015270 <detect_func>:
   15270:	48 89 d0             	mov    %rdx,%rax
   15273:	48 8d 14 37          	lea    (%rdi,%rsi,1),%rdx
   15277:	48 01 ca             	add    %rcx,%rdx
   1527a:	c3                   	ret     
```

- 第1条指令 mov %rdx,%rax：把传入的第三个参数（rdx）复制到 rax，用来作为返回值的准备（因为之后会修改 rdx）。
- 第2条指令 lea (%rdi,%rsi,1),%rdx：计算 rdi + rsi，并把结果写回 rdx（覆盖原来的 rdx）。
- 第3条指令 add %rcx,%rdx：把 rcx 加到上一步的结果上，最终 rdx = rdi + rsi + rcx。
- ret：返回，返回值在 rax（仍然是原来的 rdx）

这里可以看到，对于`Option<usize>`类型的参数，Rust编译器将其存储在rdx和rcx寄存器中,并且rdx通常是一个tag位，用来表示是否为Some（非空）或者None（空），而rcx存储实际的值。在函数返回时，rax寄存器中存储的是tag位，而rdx寄存器中存储的是实际的值。因此，在kretprobe中获取这个函数的返回值时，我们需要同时读取rax和rdx寄存器的值。

对于`Result<T, E>`类型的返回值也是类似的处理方式。我们需要根据具体的实现来确定返回值存储在哪些寄存器中，并在kretprobe中进行相应的读取。

尽管这可以处理大多数情况，但Rust有时会做进一步的优化，比如[niche-optimized](https://www.0xatticus.com/posts/understanding_rust_niche/)。在上面的示例中，`Option<usize>` 在这个目标和编译设置下不是`niche-optimized`成单个字（usize），编译器把它当作带有显式 tag（判别位） + payload（数据）的枚举（scalar pair）来传递和返回。如果 Option 的底层类型有可用的 niche（例如 Option<NonZeroUsize> 或 Option<非空指针>），编译器通常能把 None/Some 合并到单个寄存器，从而避免这个额外的 tag 寄存器。因此，在使用kretprobe获取Rust函数的返回值时，务必了解函数的返回类型及其在寄存器中的存储方式，以确保正确地读取返回值。


## 特殊情况处理

在内核的一些函数中，似乎仍然无法通过获取寄存器的值来正确获取返回值。这还等待进一步的分析。