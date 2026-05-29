//! Minimal verbosity-aware reporting to stderr.
//!
//! Kept dependency-free on purpose: a process-global verbosity flag plus a few
//! macros. Normal command output goes to stdout via `println!`; diagnostics and
//! progress go through here to stderr so machine-readable stdout stays clean.

use std::sync::atomic::{AtomicBool, Ordering};

static VERBOSE: AtomicBool = AtomicBool::new(false);

/// Enable or disable verbose diagnostics for the whole process.
pub fn set_verbose(on: bool) {
    VERBOSE.store(on, Ordering::Relaxed);
}

/// Whether verbose diagnostics are enabled.
pub fn is_verbose() -> bool {
    VERBOSE.load(Ordering::Relaxed)
}

/// Emit a verbose diagnostic line to stderr (only when `--verbose`).
#[macro_export]
macro_rules! vlog {
    ($($arg:tt)*) => {{
        if $crate::report::is_verbose() {
            eprintln!("[rosita] {}", format!($($arg)*));
        }
    }};
}

/// Emit a warning to stderr (always shown).
#[macro_export]
macro_rules! warn_user {
    ($($arg:tt)*) => {{
        eprintln!("warning: {}", format!($($arg)*));
    }};
}
