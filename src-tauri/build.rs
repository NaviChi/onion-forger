use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let expiration = now + 604800; // 7 days in seconds
    println!("cargo:rustc-env=EXPIRATION_TIME={}", expiration);
    tauri_build::build()
}
