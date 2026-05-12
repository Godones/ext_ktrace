# ktracepoint

面向内核场景的 Rust tracepoint 库，设计目标类似 Linux tracepoint：

- 用宏定义事件与字段
- 运行时按唯一事件 ID 管理
- 支持开关、过滤表达式、回调
- 支持原始事件缓冲区与可读文本输出
- no_std 可用

## 核心能力

- 事件定义：通过 define_event_trace! 一次性生成事件元数据、调用函数、注册函数
- 事件管理：TracePointMap 按 tracepoint ID 索引
- 事件控制：enable/disable、format/id/filter
- 过滤表达式：基于 tp-lexer 按 schema 编译并匹配
- 输出链路：TracePipeRaw + TraceEntryParser

## 快速接入

### 1. 添加依赖

```toml
[dependencies]
ktracepoint = "*"
static-keys = "0.8"
```

### 2. 链接脚本中保留 .tracepoint 段

该库通过 __start_tracepoint / __stop_tracepoint 扫描所有事件元数据。
请将 my_section.ld 的内容并入你的链接脚本，确保 .tracepoint 段被 KEEP。

### 3. 实现 KernelTraceOps

你需要提供：

- time_now
- cpu_id
- current_pid
- trace_pipe_push_raw_record
- trace_cmdline_push
- write_kernel_text
- tracepoint state registry 钩子：read_tracepoint_state、write_tracepoint_state

其中 write_kernel_text 用于 static key 指令补丁。
state registry 钩子用于让你的 OS 自行选择 callbacks 和 filters 的同步策略。

### Callback 限制

`read_tracepoint_state` 可以在 tracing 快路径执行 callbacks 时持有读侧锁。如果你的实现使用 `RwLock` 这类不可重入锁，callback 中不得：

- 注册或注销 tracepoint callback
- 更新 tracepoint filter
- 调用其它需要 `write_tracepoint_state` 的 API
- 递归触发由同一个 state registry 支撑的 tracepoint

违反这些规则可能导致死锁。如果宿主使用 RCU、snapshot 或其它非阻塞读侧机制实现 `read_tracepoint_state`，可以自行放宽这些限制。

### 4. 定义并调用事件

```rust
use ktracepoint::{define_event_trace, KernelTraceOps};

define_event_trace!(
    TEST,
    TP_kops(Kops),
    TP_system(tracepoint_test),
    TP_PROTO(a: u32, b: u32),
    TP_STRUCT__entry { a: u32, b: u32 },
    TP_fast_assign { a: a, b: b },
    TP_ident(__entry),
    TP_printk(format_args!("a={}, b={}", __entry.a, __entry.b))
);

// 生成函数: trace_TEST / register_trace_TEST / unregister_trace_TEST
trace_TEST(1, 2);
```

提示：TP_STRUCT__entry 会参与字节布局，请确保字段布局符合预期（建议 C 风格布局思路）。

### 5. 初始化管理器

```rust
use ktracepoint::global_init_events;

static_keys::global_init();
let mut tracepoints = global_init_events::<Kops>()?;
```

### 6. 启用、过滤、消费输出

```rust
use ktracepoint::{TraceFilterFile, TracePointEnableFile, TracePointFormatFile, TracePointIdFile};

let event_id = 0;
let event = tracepoints.get_mut(&event_id).unwrap();
TracePointEnableFile::new(event.trace_point()).write('1');
event.trace_point().enable_event();

let mut filter = TraceFilterFile::new();
filter.write(event, "a > 8 && b > 5").unwrap();

// 读取格式描述
let fmt = TracePointFormatFile::new(event.trace_point()).read();
let id = TracePointIdFile::new(event.trace_point()).read();
```

## 运行示例

```bash
cd examples
cargo run --example usage
```

示例代码位于 examples/usage.rs，覆盖了：

- 事件定义与触发
- 事件启用与过滤
- 注册 event/raw 回调
- TracePipeRaw 快照读取与文本解析

## 主要公开类型

- KernelTraceOps
- TracePoint / ExtTracePoint / TracePointMap
- TracePipeRaw / TracePipeSnapshot / TracePipeOps
- TraceCmdLineCache / TraceEntryParser

## 参考项目

- DragonOS: <https://github.com/DragonOS-Community/DragonOS/blob/master/kernel/src/debug/tracing/mod.rs>
- Hermit: <https://github.com/os-module/hermit-kernel/blob/dev/src/tracepoint/mod.rs>
