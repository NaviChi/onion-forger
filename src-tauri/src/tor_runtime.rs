use std::path::PathBuf;

pub fn runtime_label() -> &'static str {
    "torforge"
}

pub fn jitter_window_ms(is_vm: bool) -> u64 {
    if is_vm {
        5_000
    } else {
        3_000
    }
}

pub fn state_root() -> PathBuf {
    std::env::temp_dir().join("crawli_torforge_state")
}
