//! A no_std dynamic debug facility inspired by Linux `dynamic_debug`.
//!
//! The crate focuses on `pr_debug!` style callsites:
//! each macro expansion creates a per-callsite descriptor in the `__dyndbg`
//! linker section and guards the slow path with a static key.
//!
//! Typical flow:
//!
//! 1. Build a lock with your OS-specific `lock_api::RawMutex`.
//! 2. Initialize `static_keys` in your kernel/runtime.
//! 3. Call [`dynamic_debug_init`] to get a file-like [`ControlFile`].
//! 4. Use `read()` / `write()` on that control file to configure callsites.
//! 5. Sprinkle [`pr_debug!`] or [`pr_debug_fn!`] in kernel code.
#![no_std]
#![deny(missing_docs)]

extern crate alloc;

mod control;
mod runtime;

#[cfg(test)]
mod tests;

pub use control::{ControlFile, Error};
pub use function_name::named;
#[doc(hidden)]
pub use runtime::__dynamic_pr_debug;
pub use runtime::DebugOps;
#[doc(hidden)]
pub use runtime::{DebugCodeManipulator, DebugSite};

/// Emit a dynamically controllable debug print.
///
/// The first argument is the ops type, and the second argument must be a
/// string literal so the callsite can store the format string.
#[macro_export]
macro_rules! pr_debug {
    ($ops:path, $fmt:literal $(, $arg:expr)* $(,)?) => {{
        static_keys::define_static_key_false_generic!(KEY, $crate::DebugCodeManipulator<$ops>);
        #[used]
        #[unsafe(link_section = "__dyndbg")]
        static SITE: $crate::DebugSite<$ops> = $crate::DebugSite::new(
            &KEY,
            module_path!(),
            file!(),
            line!(),
            "",
            $fmt,
        );

        if static_keys::static_branch_unlikely!(KEY) {
            $crate::__dynamic_pr_debug::<$ops>(&SITE, format_args!($fmt $(, $arg)*));
        }
    }};
}

/// Emit a dynamically controllable debug print and capture the current function name.
///
/// Callers should annotate the containing function with [`named`] or
/// `#[ddebug::named]`, then use this macro instead of [`pr_debug!`]
/// if they want the callsite to match `func` selectors.
#[macro_export]
macro_rules! pr_debug_fn {
    ($ops:path, $fmt:literal $(, $arg:expr)* $(,)?) => {{
        static_keys::define_static_key_false_generic!(KEY, $crate::DebugCodeManipulator<$ops>);
        #[used]
        #[unsafe(link_section = "__dyndbg")]
        static SITE: $crate::DebugSite<$ops> = $crate::DebugSite::new(
            &KEY,
            module_path!(),
            file!(),
            line!(),
            function_name!(),
            $fmt,
        );

        if static_keys::static_branch_unlikely!(KEY) {
            $crate::__dynamic_pr_debug::<$ops>(&SITE, format_args!($fmt $(, $arg)*));
        }
    }};
}

/// Scan registered `pr_debug!`  or `pr_debug_fn!` callsites and return a Linux-like control file handle.
///
/// The caller is responsible for calling `static_keys::global_init()` before
/// enabling any dynamic debug sites.
pub fn dynamic_debug_init<K>() -> ControlFile<K>
where
    K: DebugOps + 'static,
{
    ControlFile::new(runtime::scan_debug_sites::<K>())
}
