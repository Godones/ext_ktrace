# 基于uprobe的eBPF工作原理

uprobe（用户空间探针）是一种用于在用户空间应用程序中动态插入探针的机制。它允许开发者在不修改应用程序源代码的情况下，监控和分析用户空间程序的行为。eBPF（扩展的伯克利包过滤器）是一种强大的内核技术，允许在内核中运行用户定义的程序，以实现高效的数据包处理和系统监控。eBPF与uprobe结合使用，可以实现对用户空间应用程序的深入监控和分析。

## uprobe的工作原理
uprobe通过在用户空间应用程序的特定函数入口处插入探针来工作。当应用程序执行到该函数时，uprobe会触发一个事件，允许eBPF程序捕获该事件并执行预定义的操作。以下是uprobe的工作流程：
1. **探针插入**：开发者使用工具（如`bpftrace`或`BCC`）在用户空间应用程序的特定函数入口处插入uprobe探针。探针可以插入在函数的开始处或结束处。
2. **事件触发**：当应用程序执行到插入探针的函数时，uprobe会触发一个事件(通常是break异常)。这个事件会通知内核，eBPF程序需要执行。
3. **eBPF程序执行**：内核加载并执行与uprobe事件关联的eBPF程序。eBPF程序可以访问函数的参数、返回值以及其他上下文信息。
4. **数据收集和处理**：eBPF程序可以收集数据（如函数调用次数、参数值等），并将这些数据存储在内核空间的映射中，供后续分析使用。
5. **用户空间读取数据**：用户空间应用程序可以通过系统调用从内核空间读取eBPF程序收集的数据，以进行进一步的分析和处理。

## eBPF与uprobe的结合
以Aya框架中的uprobe为例，其挂载ebpf的接口是 `program.attach("getaddrinfo", "libc", pid, None /* cookie */)?;
`，其中第一个参数是函数名，第二个参数是库名，第三个参数是进程ID，第四个参数是可选的cookie。这个接口会在指定的函数入口处插入uprobe探针，并将eBPF程序与该探针关联起来。

uprobe的运行过程主要分为几个步骤：
1. 在指定具体进程的情况下，搜索进程的内存映射，找到指定库（如libc）的路径。
2. 如果未指定具体进程，则首先检查该库是否是绝对路径
   1. 如果是绝对路径，则直接使用该路径
   2. 如果不是绝对路径，则在`/etc/ld.so.cache`中搜索该库的路径
3. 计算指定跟踪点在库中的偏移地址
   1. 用户程序可以指定三种偏移类型
```rust
   pub enum UProbeAttachLocation<'a> {
    /// The location of the target function in the target object file.
    Symbol(&'a str),
    /// The location of the target function in the target object file, offset by
    /// the given number of bytes.
    SymbolOffset(&'a str, u64),
    /// The offset in the target object file, in bytes.
    AbsoluteOffset(u64),
}
```
4. 计算跟踪点在进程内存中的实际地址（函数符号地址 + 偏移地址）
5. 在库中插入uprobe探针


对于插入探针的过程，内核的处理过程：
- 如果是不指定pid的情况，则在所有加载该库的进程中插入uprobe探针
- 如果是指定pid的情况，则只在该进程中插入uprobe探针


**由于共享库正常情况下被多个进程共享使用，因此全局插入uprobe探针时，理论上只需要在共享的物理页上修改指令，而对于指定pid插入uprobe探针时，则需要为该进程单独创建一份私有的内存页，并在该页上修改指令。**

## Linux内核的做法

Linux 利用了inode中的反向映射来实现在用户地址空间插入探针。

在指定pid的情况下, 内核首先获取该库的inode，并且使用inode:offset建立一个红黑树（rbtree），用来存储uprobe和该inode的映射关系 [alloc_uprobe](https://elixir.bootlin.com/linux/v6.6/source/kernel/events/uprobes.c#L721)。并且为该进程创建一个私有的内存页（通过copy-on-write机制），然后在该页上插入uprobe探针。

在不指定pid的情况下，内核会在全局范围内插入uprobe探针。它会遍历所有使用该库的进程，并为每个进程创建一个私有的内存页（通过copy-on-write机制），然后在该页上插入uprobe探针。对于稍后加载该库的进程，内核会在加载时检查该库的inode，并且在该进程的内存页上插入uprobe探针。

[linux:uprobe_register](https://elixir.bootlin.com/linux/v6.6/source/kernel/events/uprobes.c#L1189)
