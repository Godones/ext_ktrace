# kprobe

[English Version](./README.en.md)

一个面向内核/运行时环境的 `no_std` Rust 探针库，用来在指令地址处动态插入断点，并在断点命中与单步恢复时执行自定义处理逻辑。

这个仓库的定位更接近“探针基础设施”而不是开箱即用的工具：库本身负责安装/卸载 probe、管理断点与单步地址、封装不同架构下的寄存器与指令处理；具体的内存访问、页属性修改、可执行内存分配、任务级 retprobe 栈管理，需要由接入方实现。

## 支持能力

- `kprobe`：函数/指令入口探针
- `kretprobe`：函数返回探针
- `uprobe`：用户态地址探针
- 事件回调与自定义 `ProbeData`
- 同一地址复用同一个 `ProbePoint`
- 多架构 `PtRegs` 与断点/单步处理

## 当前支持架构

- `x86_64`
- `riscv64`
- `loongarch64`
- `aarch64`

## 核心抽象

### `ProbeBuilder<F>`

用于描述一个普通 probe：

- 探针地址：`with_symbol_addr()` + `with_offset()`
- 处理函数：`with_pre_handler()` / `with_post_handler()` / `with_fault_handler()`
- 上下文数据：`with_data()`
- 开关：`with_enable(true)`
- 用户态探针：`with_user_mode(pid)`

### `KretprobeBuilder<L>`

用于描述返回探针：

- `with_entry_handler()`：函数进入时执行
- `with_ret_handler()`：函数返回时执行
- `with_data()`：为每个实例准备用户数据
- `maxactive`：并发实例池大小

### `ProbeManager<L, F>`

统一维护两张表：

- `break_list`：断点命中地址
- `debug_list`：单步恢复后的调试地址

异常/陷阱处理代码需要在命中断点或调试异常时，把对应的 `PtRegs` 交给 manager 驱动 handler。

### `ProbePointList<F>`

`ProbePoint` 的全局索引。多个 probe 挂在同一地址时，会共享同一个已安装的断点指令，只有最后一个 probe 卸载时才真正恢复原始指令。

### `KprobeAuxiliaryOps`

这是接入方必须实现的宿主能力接口，负责：

- 内核/用户内存复制
- 修改目标地址写权限
- 分配/释放可执行内存
- 保存/弹出当前任务的 retprobe 实例

没有这层实现，库本身无法独立工作。

## 最小接入流程

1. 为你的内核/运行时实现 `KprobeAuxiliaryOps`。
2. 选择一个 `RawMutex` 实现，作为 `ProbeManager` 和回调表的锁类型。
3. 在探针异常处理路径里调用：
   - `kprobe_handler_from_break()` / `kprobe_handler_from_debug()`
   - `uprobe_handler_from_break()` / `uprobe_handler_from_debug()`
4. 通过 `register_kprobe()`、`register_kretprobe()`、`register_uprobe()` 注册探针。
5. 在不再需要时调用对应的 `unregister_*()`。

## 使用示例

下面是一个精简示意，重点展示接入方式。`MyAuxOps` 和 `MyRawMutex` 需要由你的环境提供。

```rust,ignore
use alloc::collections::BTreeMap;
use kprobe::{
    ProbeBuilder, ProbeManager, ProbePointList, PtRegs, ProbeData, KprobeAuxiliaryOps,
    register_kprobe, unregister_kprobe,
};

#[derive(Debug)]
struct MyAuxOps;

impl KprobeAuxiliaryOps for MyAuxOps {
    /* 由宿主环境实现 */
    # fn copy_memory(_: *const u8, _: *mut u8, _: usize, _: Option<i32>) {}
    # fn set_writeable_for_address<T: FnOnce(*mut u8)>(_: usize, _: usize, _: Option<i32>, _: T) {}
    # fn alloc_kernel_exec_memory() -> *mut u8 { core::ptr::null_mut() }
    # fn free_kernel_exec_memory(_: *mut u8) {}
    # fn alloc_user_exec_memory<T: FnOnce(*mut u8)>(_: Option<i32>, _: T) -> *mut u8 { core::ptr::null_mut() }
    # fn free_user_exec_memory(_: Option<i32>, _: *mut u8) {}
    # fn insert_kretprobe_instance_to_task(_: kprobe::RetprobeInstance) {}
    # fn pop_kretprobe_instance_from_task() -> kprobe::RetprobeInstance { unimplemented!() }
}

fn on_enter(_data: &dyn ProbeData, regs: &mut PtRegs) {
    let _ = regs.first_ret_value();
}

type MyRawMutex = YourRawMutex;

fn demo(target_addr: usize) {
    let mut manager = ProbeManager::<MyRawMutex, MyAuxOps>::new();
    let mut points: ProbePointList<MyAuxOps> = BTreeMap::new();

    let probe = register_kprobe(
        &mut manager,
        &mut points,
        ProbeBuilder::<MyAuxOps>::new()
            .with_symbol_addr(target_addr)
            .with_pre_handler(on_enter)
            .with_enable(true),
    );

    unregister_kprobe(&mut manager, &mut points, probe);
}
```

## 异常处理接入

这个库不会主动接管异常流程，你需要在自己的 trap/breakpoint handler 中接入它。例如：

```rust,ignore
use kprobe::{PtRegs, kprobe_handler_from_break, kprobe_handler_from_debug};

fn handle_break(regs: &mut PtRegs, manager: &mut ProbeManager<MyRawMutex, MyAuxOps>) -> bool {
    kprobe_handler_from_break(manager, regs).is_some()
}

fn handle_debug(regs: &mut PtRegs, manager: &mut ProbeManager<MyRawMutex, MyAuxOps>) -> bool {
    kprobe_handler_from_debug(manager, regs).is_some()
}
```

`uprobe` 的接入方式相同，只是注册时需要 `with_user_mode(pid)`，并在异常路径调用 `uprobe_handler_from_break/debug()`。

## 返回值获取

返回探针通常通过 `PtRegs` 读取返回寄存器：

- 第一返回值：`PtRegs::first_ret_value()`
- 第二返回值：`PtRegs::second_ret_value()`

对于 Rust 中的 `Option<T>`、`Result<T, E>` 等类型，返回值可能分布在多个寄存器中。详细背景可参考 [docs/kretprobe.md](./docs/kretprobe.md)。

## 项目结构

- `src/lib.rs`：统一导出与 `register_kretprobe()`
- `src/kprobe/`：普通 probe 注册与异常分发
- `src/uprobe/`：用户态 probe 注册与异常分发
- `src/arch/`：各架构 `ProbePoint`、`PtRegs`、指令替换与 retprobe trampoline
- `src/manager.rs`：`ProbeManager` 与 `ProbePointList`
- `docs/`：实现说明与背景资料

## 注意事项

- 这是 `#![no_std]` 库，默认运行场景是内核或低层运行时。
- 文档中的示例是接入示意，不是可直接运行的完整程序。
- 当前代码导出了 `Uretprobe` 类型别名，但没有独立的 `register_uretprobe()` / `unregister_uretprobe()` 公开接口。
- 某些架构依赖裸函数与内联汇编，建议按仓库当前工具链配置进行构建。

## 参考文档

- [docs/kretprobe.md](./docs/kretprobe.md)
- [docs/How uprobe ebpf work.md](./docs/How%20uprobe%20ebpf%20work.md)
- [docs/arm64.md](./docs/arm64.md)
