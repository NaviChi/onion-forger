# Lessons Learned: Onion Forger / Crawli

## Backend / Rust Enhancements

### Lesson 1: HTTP/2 over Tor Circuits
**Issue:** Making hundreds of quick `reqwest` requests over SOCKS5 isolated proxies causes massive port exhaustion and TCP teardown overhead in Windows due to the ephemeral port limit (max 16,384 by default).
**Fix:** Bypassed `reqwest` for complex interactions by migrating to `arti_client` native bindings combined with `hyper-rustls`. Configured the TLS builder to force `enable_http2()` and tuned the hyper `Client` pooling to share streams over a single Tor circuit.
**Prevention Rule:**
* DO NOT use vanilla `reqwest::Client` over SOCKS5 for concurrent spidering. Always use a configured HTTP/2 hyper client wrapper directly integrating `ArtiConnector` to multiplex streams onto single circuits.

### Lesson 2: Speculative Pre-Fetching Limits
**Issue:** Fetching too many speculative futures simultaneously causes memory bloat and Tor circuit collapse due to hidden service `RELAY_DROP` thresholds.
**Fix:** Limited the `SpeculativePrefetcher` to an exact pipeline queue depth of `3` child URLs using an mpsc channel to regulate back-pressure. 
**Prevention Rule:** 
* DO NOT spawn unbounded `tokio::task` pre-fetches for every anchor tag encountered unless constrained by a bounded channel semaphore.

### Lesson 3: HDD vs SSD File Writing
**Issue:** Writing thousands of 2KB target manifests in a flat directory thrashes HDD IOPS and ruins download speeds.
**Fix:** Separated `targets/<target_key>/` (metrics/logs) from physical `downloads/<target_key>/` and implemented human-readable `listing_windows.txt` exports so the user doesn't need to perform OS-level searches across thousands of JSON shards.
**Prevention Rule:**
* ALWAYS write target manifests sequentially to a deterministic folder. Use `target_key` hashing for safe routing.

### Lesson 4: blocking_read Panics Inside Async Runtimes (Phase 74B)
**Issue:** `tokio::sync::RwLock::blocking_read()` panics when called from within the tokio async runtime. This occurred in `QilinRoutePlan::current_seed_url_sync()` across 6 call sites, causing worker panics during 20-min soak tests.
**Fix:** Replaced `blocking_read()` with `try_read()` + empty-string fallback. Since `current_seed_url_sync` is used for best-effort URL reads (the caller retries), an empty fallback on lock contention is safe.
**Prevention Rule:**
* NEVER use `blocking_read()` or `blocking_write()` from `tokio::sync::RwLock` inside async contexts. Always use `try_read()` / `try_write()` or the async `.read().await` / `.write().await` variants.
* If the call site cannot be made async, use `try_read()` with a sensible fallback value.
