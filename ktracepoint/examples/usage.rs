use ktracepoint::{
    RawTracePointCallBackFunc, TraceCmdLineCache, TraceEntryParser, TracePipeOps,
    TracePointCallBackFunc, TracePointMap, global_init_events,
};
use spin::Mutex;
extern crate alloc;

mod tracepoint_test {
    use std::{ops::Deref, sync::Arc, time};

    use ktracepoint::{KernelTraceOps, TraceCmdLineCache, define_event_trace};
    use spin::Mutex;

    pub static TRACE_RAW_PIPE: Mutex<ktracepoint::TracePipeRaw> =
        Mutex::new(ktracepoint::TracePipeRaw::new(1024));

    pub static TRACE_CMDLINE_CACHE: Mutex<TraceCmdLineCache> =
        Mutex::new(ktracepoint::TraceCmdLineCache::new(128));
    pub struct Kops;

    impl KernelTraceOps for Kops {
        fn cpu_id() -> u32 {
            0
        }

        fn current_pid() -> u32 {
            1
        }
        fn time_now() -> u64 {
            time::SystemTime::now()
                .duration_since(time::UNIX_EPOCH)
                .unwrap()
                .as_nanos() as u64
        }

        fn trace_pipe_push_raw_record(buf: &[u8]) {
            let mut pipe = TRACE_RAW_PIPE.lock();
            pipe.push_event(buf.to_vec());
        }

        fn trace_cmdline_push(pid: u32) {
            let mut cache = TRACE_CMDLINE_CACHE.lock();
            cache.insert(pid, "test_process".to_string());
        }

        // copy from static-keys
        fn write_kernel_text(addr: *mut core::ffi::c_void, data: &[u8]) {
            let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) as usize };
            let aligned_addr_val = (addr as usize) / page_size * page_size;
            let aligned_addr = aligned_addr_val as *mut core::ffi::c_void;
            let aligned_length = if (addr as usize) + data.len() - aligned_addr_val > page_size {
                page_size * 2
            } else {
                page_size
            };

            // Create a temp mmap, which will store updated content of corresponding pages
            let mmaped_addr = unsafe {
                libc::mmap(
                    core::ptr::null_mut(),
                    aligned_length,
                    libc::PROT_READ | libc::PROT_WRITE,
                    libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
                    -1,
                    0,
                )
            };
            if mmaped_addr == libc::MAP_FAILED {
                panic!("Failed to create temp mappings");
            }
            unsafe {
                let addr_in_mmap = mmaped_addr.offset(addr.offset_from(aligned_addr));
                core::ptr::copy_nonoverlapping(aligned_addr, mmaped_addr, aligned_length);
                core::ptr::copy_nonoverlapping(data.as_ptr(), addr_in_mmap.cast(), data.len());
            }
            let res = unsafe {
                libc::mprotect(
                    mmaped_addr,
                    aligned_length,
                    libc::PROT_READ | libc::PROT_EXEC,
                )
            };
            if res != 0 {
                panic!("Unable to make mmaped mapping executable.");
            }
            // Remap the created temp mmaping to replace old mapping
            let res = unsafe {
                libc::mremap(
                    mmaped_addr,
                    aligned_length,
                    aligned_length,
                    libc::MREMAP_MAYMOVE | libc::MREMAP_FIXED,
                    // Any previous mapping at the address range specified by new_address and new_size is unmapped.
                    // So, no memory leak
                    aligned_addr,
                )
            };
            if res == libc::MAP_FAILED {
                panic!("Failed to mremap.");
            }
            let res = unsafe { clear_cache::clear_cache(addr, addr.add(data.len())) };
            if !res {
                panic!("Failed to clear cache.");
            }
        }
    }

    #[repr(C)]
    #[derive(Debug)]
    #[allow(clippy::redundant_allocation)]
    struct TestS {
        a: u32,
        b: Box<Arc<u32>>,
    }

    define_event_trace!(
        TEST,
        TP_lock(spin::Mutex<()>),
        TP_kops(Kops),
        TP_system(tracepoint_test),
        TP_PROTO(a: u32, b: &TestS),
        TP_STRUCT__entry{
            a: u32,
            pad:[u8;4],
            b: u32
        },
        TP_fast_assign{
            a: a,
            pad: [0; 4],
            b: *b.b.deref().deref()
        },
        TP_ident(__entry),
        TP_printk(
            {
                let arg1 = __entry.a;
                let arg2 = __entry.b;
                format!("Hello from tracepoint! a={:?}, b={}", arg1, arg2)
            }
        )
    );

    define_event_trace!(
        TEST2,
        TP_lock(Mutex<()>),
        TP_kops(Kops),
        TP_system(tracepoint_test),
        TP_PROTO(a: u32, b: u32),
        TP_STRUCT__entry{
            a: u32,
            b: u32
        },
        TP_fast_assign{
            a:a,
            b:b
        },
        TP_ident(__entry),
        TP_printk(format_args!("Hello from tracepoint! a={}, b={}", __entry.a, __entry.b))
    );

    pub fn test_trace(a: u32, b: u32) {
        let x = TestS {
            a,
            b: Box::new(Arc::new(b)),
        };
        trace_TEST(a, &x);
        trace_TEST2(a, b);
        println!(
            "Tracepoint TEST called with a={}, b={}, x ptr={:p}",
            a, b, &x
        );
    }
}

fn print_trace_records(
    tracepoint_map: &TracePointMap<Mutex<()>, tracepoint_test::Kops>,
    trace_cmdline_cache: &TraceCmdLineCache,
) {
    let mut snapshot = tracepoint_test::TRACE_RAW_PIPE.lock().snapshot();
    print!("{}", snapshot.default_fmt_str());
    loop {
        let mut flag = false;
        if let Some(event) = snapshot.peek() {
            let trace_str = TraceEntryParser::parse::<tracepoint_test::Kops, _>(
                tracepoint_map,
                trace_cmdline_cache,
                event,
            );
            print!("{}", trace_str);
            flag = true;
        }
        if flag {
            snapshot.pop();
        } else {
            break;
        }
    }
}

struct FakeEventCallback;

impl TracePointCallBackFunc for FakeEventCallback {
    fn call(&self, entry: &[u8]) {
        println!("FakeEventCallback called with entry: {}", entry.len());
    }
}

impl RawTracePointCallBackFunc for FakeEventCallback {
    fn call(&self, args: &[u64]) {
        println!("FakeEventCallback (raw) called with args: {:x?}", args);
    }
}

fn main() {
    env_logger::try_init_from_env(env_logger::Env::default().default_filter_or("debug"))
        .expect("Failed to initialize logger");

    // First, we need to initialize the static keys.
    static_keys::global_init();
    // Then, we need to initialize the tracepoint and events.
    // This will create a new events manager and register the tracepoint.
    // The events manager will be used to manage the tracepoints and events.
    let manager = global_init_events::<Mutex<()>, tracepoint_test::Kops>().unwrap();
    let tracepoint_map = manager.tracepoint_map();

    println!("---Before enabling tracepoints---");
    tracepoint_test::test_trace(1, 2);
    tracepoint_test::test_trace(3, 4);
    print_trace_records(
        &tracepoint_map,
        &tracepoint_test::TRACE_CMDLINE_CACHE.lock(),
    );

    println!();
    for sbs in manager.subsystem_names() {
        let subsystem = manager.get_subsystem(&sbs).unwrap();
        let events = subsystem.event_names();
        for event in events {
            let trace_point_info = subsystem.get_event(&event).unwrap();
            // enable the tracepoint
            trace_point_info.enable_file().write('1');

            // Register fake callbacks
            trace_point_info
                .tracepoint()
                .register_event_callback(1, Box::new(FakeEventCallback));

            // Register raw fake callbacks
            trace_point_info
                .tracepoint()
                .register_raw_event_callback(1, Box::new(FakeEventCallback));

            // Enable the event
            trace_point_info.tracepoint().enable_event();

            trace_point_info
                .filter_file()
                .write("(a > 8 && a<=10) || b >5")
                .unwrap();

            let schema = trace_point_info.tracepoint().schema();
            println!("Schema for {}.{}: {:#?}", sbs, event, schema);
            println!("Enabled tracepoint: {}.{}", sbs, event);
        }
    }

    println!("---After enabling tracepoints---");
    tracepoint_test::test_trace(1, 2);
    tracepoint_test::test_trace(9, 2); // should match
    tracepoint_test::test_trace(3, 4);
    tracepoint_test::test_trace(10, 4); // should match
    tracepoint_test::test_trace(11, 6); // should match

    print_trace_records(
        &tracepoint_map,
        &tracepoint_test::TRACE_CMDLINE_CACHE.lock(),
    );

    for tracepoint in tracepoint_map.values() {
        println!("{}", tracepoint.print_fmt());
    }
}
