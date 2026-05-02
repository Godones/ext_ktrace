use std::ffi::c_void;

use ddebug::{DebugOps, dynamic_debug_init, pr_debug, pr_debug_fn};

unsafe fn write_kernel_text(addr: *mut c_void, data: &[u8]) {
    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) as usize };
    let aligned_addr_val = (addr as usize) / page_size * page_size;
    let aligned_addr = aligned_addr_val as *mut c_void;
    let aligned_length = if (addr as usize) + data.len() - aligned_addr_val > page_size {
        page_size * 2
    } else {
        page_size
    };

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
    assert_ne!(mmaped_addr, libc::MAP_FAILED);

    unsafe {
        let addr_in_mmap = mmaped_addr.offset(addr.offset_from(aligned_addr));
        core::ptr::copy_nonoverlapping(aligned_addr, mmaped_addr, aligned_length);
        core::ptr::copy_nonoverlapping(data.as_ptr(), addr_in_mmap.cast(), data.len());
    }

    let ret = unsafe {
        libc::mprotect(
            mmaped_addr,
            aligned_length,
            libc::PROT_READ | libc::PROT_EXEC,
        )
    };
    assert_eq!(ret, 0);

    let ret = unsafe {
        libc::mremap(
            mmaped_addr,
            aligned_length,
            aligned_length,
            libc::MREMAP_MAYMOVE | libc::MREMAP_FIXED,
            aligned_addr,
        )
    };
    assert_ne!(ret, libc::MAP_FAILED);
    assert!(unsafe { clear_cache::clear_cache(addr, addr.add(data.len())) });
}
struct MyOps;

impl DebugOps for MyOps {
    fn write_kernel_text(addr: *mut u8, data: &[u8]) {
        unsafe { write_kernel_text(addr as _, data) };
    }

    fn emit(line: &str) {
        print!("{}", line);
    }

    fn thread_id() -> u64 {
        0
    }
}

#[macro_export]
macro_rules! my_pr_debug {
    ($fmt:literal $(, $arg:expr)* $(,)?) => {{
        pr_debug!(MyOps, $fmt $(, $arg)*);
    }};
}

#[macro_export]
macro_rules! my_pr_debug_fn {
    ($fmt:literal $(, $arg:expr)* $(,)?) => {{
        pr_debug_fn!(MyOps, $fmt $(, $arg)*);
    }};
}

#[ddebug::named]
fn fake_run() {
    my_pr_debug!("This is a fake run of the kernel code with dynamic debug.");
    my_pr_debug!("The fake run value of x is: {}", 42);
    my_pr_debug_fn!("The function name is: {}", function_name!());
    println!("This is a normal print statement that should always be printed.");
}

fn main() {
    env_logger::try_init_from_env(env_logger::Env::default().default_filter_or("debug")).unwrap();

    // initialize the static keys and debug system
    static_keys::global_init();
    let mut ctl = dynamic_debug_init::<MyOps>();
    assert_eq!(ctl.procfs_path(), "/proc/dynamic_debug/control");

    {
        println!("Running the fake code with all debug sites disabled (default)...");
        fake_run();
    }

    ctl.write("func fake_run =pmfsl").unwrap(); // only can enable pr_debug_fn sites in fake_run
    ctl.write("format \"fake run\" =pmsl").unwrap(); // enable all sites with "fake run" in their format string
    let listing = ctl.read().unwrap();
    println!("Current debug sites:\n{listing}");

    // enable the 'fake run' site with all flags, then run the code again to see the debug prints
    {
        println!("Running the fake code with all debug sites enabled...");
        fake_run();
    }
}
