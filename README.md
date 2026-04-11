# ext_ktrace

This project aims to provide a set of components for adding trace support to the Rust kernel.


## Components

- [ktracepoint](ktracepoint/): A Rust crate for implementing tracepoints in the kernel. This crate provides a flexible and efficient way to add tracing capabilities to your kernel, similar to Linux kernel's tracepoint mechanism.
- [kprobe](kprobe/): A Rust crate for implementing kprobes/uprobes in the kernel. This crate provides a way to dynamically instrument kernel functions and collect data at runtime.
- [bpf-basic](bpf-basic/): A Rust library providing basic abstractions and utilities for eBPF (Extended Berkeley Packet Filter) programming.
- [tp-lexer](tp-lexer/): A Rust library for parsing and evaluating filter expressions for tracepoints.
- [ksym](ksym/): A Rust library for generating symbol tables for operating systems, similar to Linux's kallsyms.

## Roadmap
- [x] Implement basic kprobe support
- [x] Implement basic tracepoint support
- [x] Implement basic eBPF map and helper functions
- [x] Implement basic eBPF support in some kernels 
    - Monolithic kernels
        - [x] [DragonOS](https://github.com/DragonOS-Community/DragonOS)
        - [x] [Starry](https://github.com/Starry-OS/StarryOS)
        - [x] [Alien](https://github.com/Godones/Alien)
    - Unikernel
        - [x] [Hermit](https://github.com/os-module/hermit-rs/tree/dev) 
- [ ] Implement more eBPF maps and helper functions
  - [x] ringbuf
- [ ] Implement other kernel hooks
  - [x] rawtracepoint
  - [x] uprobe
  - [ ] uretprobe
  - [ ] perf events


### License

This project is licensed under the MIT License. See the [LICENSE](LICENSE) file for the full license text.

## Reference
- https://docs.cilium.io/en/stable/reference-guides/bpf/architecture/
- https://blog.spoock.com/2024/01/11/bpf-tail-call-intro/