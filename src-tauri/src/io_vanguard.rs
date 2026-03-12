use std::fs::OpenOptions;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectIoPolicy {
    Auto,
    Always,
    Off,
}

static DIRECT_IO_POLICY: OnceLock<DirectIoPolicy> = OnceLock::new();
static DIRECT_IO_DEGRADED: AtomicBool = AtomicBool::new(false);
static RUNTIME_DIRECT_IO_OVERRIDE: OnceLock<Mutex<Option<DirectIoPolicy>>> = OnceLock::new();

fn runtime_override_slot() -> &'static Mutex<Option<DirectIoPolicy>> {
    RUNTIME_DIRECT_IO_OVERRIDE.get_or_init(|| Mutex::new(None))
}

pub fn direct_io_policy() -> DirectIoPolicy {
    if let Ok(guard) = runtime_override_slot().lock() {
        if let Some(policy) = *guard {
            return policy;
        }
    }
    *DIRECT_IO_POLICY.get_or_init(|| {
        match std::env::var("CRAWLI_DIRECT_IO")
            .unwrap_or_else(|_| "auto".to_string())
            .to_ascii_lowercase()
            .as_str()
        {
            "always" | "on" | "force" | "1" => DirectIoPolicy::Always,
            "off" | "disabled" | "0" => DirectIoPolicy::Off,
            _ => DirectIoPolicy::Auto,
        }
    })
}

pub fn direct_io_policy_label() -> &'static str {
    match direct_io_policy() {
        DirectIoPolicy::Auto => "auto",
        DirectIoPolicy::Always => "always",
        DirectIoPolicy::Off => "off",
    }
}

pub fn set_runtime_direct_io_override(policy: Option<DirectIoPolicy>) {
    if let Ok(mut guard) = runtime_override_slot().lock() {
        *guard = policy;
    }
}

pub struct RuntimeDirectIoOverrideGuard;

impl RuntimeDirectIoOverrideGuard {
    pub fn new(policy: Option<DirectIoPolicy>) -> Self {
        set_runtime_direct_io_override(policy);
        Self
    }
}

impl Drop for RuntimeDirectIoOverrideGuard {
    fn drop(&mut self) {
        set_runtime_direct_io_override(None);
    }
}

pub fn should_try_direct_io() -> bool {
    match direct_io_policy() {
        DirectIoPolicy::Off => false,
        DirectIoPolicy::Always => true,
        DirectIoPolicy::Auto => !DIRECT_IO_DEGRADED.load(Ordering::Relaxed),
    }
}

pub fn mark_direct_io_degraded() {
    if direct_io_policy() == DirectIoPolicy::Auto {
        DIRECT_IO_DEGRADED.store(true, Ordering::Relaxed);
    }
}

pub fn is_direct_io_degraded() -> bool {
    DIRECT_IO_DEGRADED.load(Ordering::Relaxed)
}

pub fn apply_direct_io_if_enabled(opts: &mut OpenOptions) -> bool {
    if should_try_direct_io() {
        apply_internal(opts);
        return true;
    }
    false
}

/// Applies OS-specific bypasses for the page cache (Direct I/O) to an OpenOptions builder.
/// This provides High-Frequency Trading tier disk speeds by writing data
/// straight from the network socket to the NVMe disk wrapper.
pub fn apply_direct_io(opts: &mut OpenOptions) -> &mut OpenOptions {
    apply_internal(opts)
}

#[cfg(target_os = "linux")]
fn apply_internal(opts: &mut OpenOptions) -> &mut OpenOptions {
    use std::os::unix::fs::OpenOptionsExt;
    opts.custom_flags(libc::O_DIRECT)
}

#[cfg(target_os = "macos")]
fn apply_internal(opts: &mut OpenOptions) -> &mut OpenOptions {
    // macOS doesn't support O_DIRECT in open().
    // We must open normally and then use fcntl to set F_NOCACHE.
    // So we don't modify the OpenOptions here, we use a separate function after opening.
    opts
}

#[cfg(target_os = "windows")]
fn apply_internal(opts: &mut OpenOptions) -> &mut OpenOptions {
    use std::os::windows::fs::OpenOptionsExt;
    // Phase 129: REMOVED FILE_FLAG_NO_BUFFERING (0x20000000).
    // FILE_FLAG_NO_BUFFERING requires ALL reads and writes to be aligned to the
    // device sector boundary (typically 512 or 4096 bytes). Tor download chunks
    // are arbitrarily sized (BBR-controlled, ranging from 16KB to 1MB with no
    // alignment guarantee), making writes fail with ERROR_INVALID_PARAMETER (87)
    // on Windows. This was a silent corruption source: the write would fail, the
    // error would be swallowed, and the file would be left with holes.
    //
    // We keep FILE_FLAG_WRITE_THROUGH (0x80000000) which still bypasses the
    // Windows write-back cache (data goes straight to disk) but does NOT require
    // sector alignment. This gives us ~90% of the Direct I/O benefit without
    // the alignment landmine.
    opts.custom_flags(0x80000000) // FILE_FLAG_WRITE_THROUGH only
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn apply_internal(opts: &mut OpenOptions) -> &mut OpenOptions {
    opts
}

/// Applies post-open configurations (required for macOS F_NOCACHE).
pub fn post_open_config(file: &std::fs::File) {
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::io::AsRawFd;
        let fd = file.as_raw_fd();
        unsafe {
            if libc::fcntl(fd, libc::F_NOCACHE, 1) == -1 {
                eprintln!("[Warning] Failed to set F_NOCACHE on macOS");
            }
        }
    }
}
