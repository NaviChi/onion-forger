use std::fs::OpenOptions;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectIoPolicy {
    Auto,
    Always,
    Off,
}

static DIRECT_IO_POLICY: OnceLock<DirectIoPolicy> = OnceLock::new();
static DIRECT_IO_DEGRADED: AtomicBool = AtomicBool::new(false);

pub fn direct_io_policy() -> DirectIoPolicy {
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
    // 0x20000000 = FILE_FLAG_NO_BUFFERING
    // 0x80000000 = FILE_FLAG_WRITE_THROUGH
    opts.custom_flags(0x20000000 | 0x80000000)
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
