# Linux dynamic debug 

Dynamic debug 本质上是把内核里的 pr_debug() / dev_dbg() 这类调试点，做成“可枚举、可查询、可运行时开关”的调试系统。它最适合排查驱动、模块初始化、设备枚举这类问题，因为不用重新编译就能精确打开某一批调试日志。

**功能**

- 运行时启停调试日志。不是全局打开 DEBUG，而是按调用点精确控制。
- 提供完整“调试点目录”。可通过 /proc/dynamic_debug/control 查看所有已注册的 debug callsite；若启用了 debugfs，通常也能在 /sys/kernel/debug/dynamic_debug/control 看到。
- 支持按多种条件筛选：file、func、line、module、format，新文档里还支持 class。
- 支持通配符 *、?，并且一次写入可包含多条命令，用 ; 或换行分隔。
- 支持标志位控制：+p 打开打印，-p 关闭；m/f/l/s/t/d 分别给输出附加模块名、函数名、行号、源文件名、线程号/中断标识、调用栈。print_hex_dump_debug() 基本只认 p。
- 支持启动期调试：可用 dyndbg="QUERY" 或 foo.dyndbg="QUERY"，让内建代码或模块初始化阶段一开始就输出调试信息。
- 只影响走 dynamic debug 体系的 debug 宏，不会接管普通 printk() / pr_info()。核心对象主要是 pr_debug()、dev_dbg()、print_hex_dump_debug()、print_hex_dump_bytes()。

**实现原理**

- 编译期改写入口。启用 CONFIG_DYNAMIC_DEBUG 后，pr_debug() 会展开到 dynamic_pr_debug()，dev_dbg() 会展开到 dynamic_dev_dbg()。
- 每个调试语句都会生成一个静态的 struct _ddebug 描述符，记录模块名、函数名、文件名、格式串、行号、class、flags 等元数据，并放进专门的 ELF section。
- 这里有个版本差异：你给的博客用的是较老内核，section 名还是 __verbose；v6.6 头文件里已经是 __dyndbg。但机制没变，都是“每个 callsite 一个描述符”。
- 内核启动时，dynamic_debug_init() 通过链接器符号 __start___dyndbg 到 __stop___dyndbg 扫描整张表，把这些 callsite 按模块组织起来；同时注册模块 notifier，这样后续模块加载/卸载时也能把自己的调试点加入/移除。
- 控制面走 control 文件。用户向 dynamic_debug/control 写入查询语句后，内核会解析关键词和 flags，再遍历所有 _ddebug 项，匹配成功的就修改其 flags 或对应的 static key。
- 数据面走“快速分支”。callsite 执行时会先判断 DYNAMIC_DEBUG_BRANCH(descriptor)：
  - 有 CONFIG_JUMP_LABEL 时，用 static_branch_likely/unlikely，关闭状态开销很低。
  - 没有 jump label 时，就退化成检查 descriptor.flags & _DPRINTK_FLAGS_PRINT。
- 分支命中后，才真正调用 __dynamic_pr_debug() / __dynamic_dev_dbg()，最终落到 printk(KERN_DEBUG, ...) 或 dev_printk_emit(...)。附加前缀如模块名、函数名、行号，也是这里根据 flags 动态拼出来的。
- class 机制是更细粒度的分类控制。模块可以声明自己的 class map，用户就能按逻辑类别而不是按文件/函数去开关调试。

一句话概括：Dynamic debug 是“编译期埋点 + 链接期收集 + 启动期建表 + 运行时匹配改标志 + 执行时快速分支”的一套内核日志动态开关机制。


Flags 说明：
- p    enables the pr_debug() callsite.
- _    enables no flags.

Decorator flags add to the message-prefix, in order:
- t    Include thread ID, or \<intr\> if in interrupt context
- m    Include module name
- f    Include the function name
- s    Include the source file name
- l    Include line number
- d    Include call trace

参考：

- Linux 官方文档: https://docs.kernel.org/admin-guide/dynamic-debug-howto.html
- 博客园文章: https://www.cnblogs.com/JiMoKuangXiangQu/articles/18812427
- v6.6 同位置头文件镜像: https://codebrowser.dev/linux/linux/include/linux/dynamic_debug.h.html
- 实现文件补充: https://codebrowser.dev/linux/linux/lib/dynamic_debug.c.html