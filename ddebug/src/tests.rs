extern crate std;

use alloc::{
    format,
    string::{String, ToString},
    vec::Vec,
};
use core::ffi::c_void;
use std::sync::{Mutex as StdMutex, OnceLock};

use crate::{
    ControlFile, DebugOps,
    control::{
        expected_print_mask, flag_mask_for_tests, flag_op_is_add, flag_op_is_replace,
        parse_commands_for_tests,
    },
    dynamic_debug_init, pr_debug, pr_debug_fn,
};

static TEST_LOCK: StdMutex<()> = StdMutex::new(());
static EMITTED: OnceLock<StdMutex<Vec<String>>> = OnceLock::new();
static STATIC_KEYS_INIT: OnceLock<()> = OnceLock::new();

fn emitted() -> &'static StdMutex<Vec<String>> {
    EMITTED.get_or_init(|| StdMutex::new(Vec::new()))
}

fn test_emit(line: &str) {
    emitted().lock().unwrap().push(line.to_string());
}

fn clear_output() {
    emitted().lock().unwrap().clear();
}

fn take_output() -> Vec<String> {
    core::mem::take(&mut *emitted().lock().unwrap())
}

fn ensure_static_keys_init() {
    STATIC_KEYS_INIT.get_or_init(static_keys::global_init);
}

fn init_for_tests() -> ControlFile<TestOps> {
    ensure_static_keys_init();
    let mut ctl = dynamic_debug_init::<TestOps>();
    ctl.write("=_").unwrap();
    clear_output();
    ctl
}

struct TestOps;

impl DebugOps for TestOps {
    fn write_kernel_text(addr: *mut u8, data: &[u8]) {
        unsafe { write_kernel_text(addr as _, data) };
    }

    fn emit(line: &str) {
        test_emit(line);
    }

    fn thread_id() -> u64 {
        4242
    }
}

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

fn demo_debug(value: u32) {
    pr_debug!(TestOps, "demo value={}", value);
}

#[crate::named]
fn prefix_debug(value: u32) {
    pr_debug_fn!(TestOps, "prefix value={}", value);
}

fn line_debug() -> u32 {
    let line = line!() + 1;
    pr_debug!(TestOps, "line filtered");
    line
}

#[crate::named]
fn function_debug(value: u32) {
    pr_debug_fn!(TestOps, "func filtered value={}", value);
}

#[test]
fn parser_handles_quotes_and_flags() {
    let _guard = TEST_LOCK.lock().unwrap();
    let commands = parse_commands_for_tests(
        "file src/tests.rs func function_debug line 10-12 format \"alloc page\" +p; module ddebug::tests =ptmfsl",
    )
    .unwrap();
    assert_eq!(commands.len(), 2);
    assert!(flag_op_is_add(&commands[0]));
    assert_eq!(flag_mask_for_tests(&commands[0]), expected_print_mask());
    assert!(flag_op_is_replace(&commands[1]));
}

#[test]
fn control_file_exposes_linux_paths() {
    let _guard = TEST_LOCK.lock().unwrap();
    let ctl = init_for_tests();
    assert_eq!(ctl.procfs_path(), ControlFile::<TestOps>::PROC_PATH);
    assert_eq!(ctl.debugfs_path(), ControlFile::<TestOps>::DEBUGFS_PATH);
}

#[test]
fn format_query_enables_and_disables_printing() {
    let _guard = TEST_LOCK.lock().unwrap();
    let mut ctl = init_for_tests();

    let count = ctl.write("format \"demo value=\" +p").unwrap();
    assert!(count >= 1);

    demo_debug(7);
    let lines = take_output();
    assert_eq!(lines.len(), 1);
    assert!(lines[0].contains("demo value=7"));

    ctl.write("format \"demo value=\" -p").unwrap();
    demo_debug(9);
    assert!(take_output().is_empty());
}

#[test]
fn selectors_and_prefix_flags_work() {
    let _guard = TEST_LOCK.lock().unwrap();
    let mut ctl = init_for_tests();

    let line = line_debug();
    assert!(take_output().is_empty());

    let query = format!(
        "module ddebug::tests file src/tests.rs line {} format \"line filtered\" =ptmsl",
        line
    );
    let matches = ctl.write(&query).unwrap();
    assert!(matches >= 1);

    let listing = ctl.read().unwrap();
    assert!(listing.contains("line filtered"));
    assert!(listing.contains("=ptmsl"));

    ctl.write("format \"prefix value=\" =ptmfsl").unwrap();
    prefix_debug(11);
    let lines = take_output();
    assert_eq!(lines.len(), 1);
    assert!(lines[0].contains("tid=4242"));
    assert!(lines[0].contains("[ddebug::tests]"));
    assert!(lines[0].contains("prefix_debug"));
    assert!(lines[0].contains("src/tests.rs"));
    assert!(lines[0].contains("prefix value=11"));
}

#[test]
fn func_selector_matches_named_callsites() {
    let _guard = TEST_LOCK.lock().unwrap();
    let mut ctl = init_for_tests();

    ctl.write("func function_debug +p").unwrap();
    function_debug(5);
    let lines = take_output();
    assert_eq!(lines.len(), 1);
    assert!(lines[0].contains("func filtered value=5"));

    let listing = ctl.read().unwrap();
    assert!(listing.contains("[ddebug::tests]function_debug =p"));
}

#[test]
fn wildcard_func_selector_matches_only_named_sites() {
    let _guard = TEST_LOCK.lock().unwrap();
    let mut ctl = init_for_tests();

    let matches = ctl.write("func *debug +p").unwrap();
    assert!(matches >= 2);

    demo_debug(1);
    prefix_debug(2);
    function_debug(3);

    let lines = take_output();
    assert_eq!(lines.len(), 2);
    assert!(lines.iter().any(|line| line.contains("prefix value=2")));
    assert!(
        lines
            .iter()
            .any(|line| line.contains("func filtered value=3"))
    );
    assert!(!lines.iter().any(|line| line.contains("demo value=1")));
}

#[test]
fn wildcard_file_and_format_selectors_work_together() {
    let _guard = TEST_LOCK.lock().unwrap();
    let mut ctl = init_for_tests();

    let matches = ctl
        .write("file *src/tests.rs format \"demo value=*\" +p")
        .unwrap();
    assert!(matches >= 1);

    demo_debug(7);
    prefix_debug(8);

    let lines = take_output();
    assert_eq!(lines.len(), 1);
    assert!(lines[0].contains("demo value=7"));
}

#[test]
fn module_suffix_question_mark_and_multi_command_queries_work() {
    let _guard = TEST_LOCK.lock().unwrap();
    let mut ctl = init_for_tests();

    let matches = ctl
        .write("module tests func ?refix_debug +p; format \"func filtered*\" +p")
        .unwrap();
    assert!(matches >= 2);

    demo_debug(10);
    prefix_debug(11);
    function_debug(12);

    let lines = take_output();
    assert_eq!(lines.len(), 2);
    assert!(lines.iter().any(|line| line.contains("prefix value=11")));
    assert!(
        lines
            .iter()
            .any(|line| line.contains("func filtered value=12"))
    );
    assert!(!lines.iter().any(|line| line.contains("demo value=10")));
}
