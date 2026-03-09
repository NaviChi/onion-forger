use std::time::Instant;
use tauri::{AppHandle, Emitter};

/// A utility for emitting globally-timed verbose forensic logs.
#[derive(Clone)]
pub struct CrawlTimer {
    start: Instant,
    app: AppHandle,
}

impl CrawlTimer {
    pub fn new(app: AppHandle) -> Self {
        Self {
            start: Instant::now(),
            app,
        }
    }

    /// Emits a log to the GUI with a precise `[+X.XXs]` prefix.
    pub fn emit_log(&self, message: &str) {
        let elapsed = self.start.elapsed().as_secs_f64();
        let formatted = format!("[+{:05.2}s] {}", elapsed, message);
        let _ = self.app.emit("crawl_log", formatted);
    }
}
