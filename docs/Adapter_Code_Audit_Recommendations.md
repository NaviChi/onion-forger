# Adapter Code Audit & Recommendations Guide

> **Date**: 2026-03-03  
> **Scope**: All crawler adapters after `crossbeam_queue::SegQueue` migration  
> **Status**: Post-compilation fix for `inc_ransom.rs`

---

## Executive Summary

After migrating all adapters from sequential `mpsc::unbounded_channel` to lock-free `crossbeam_queue::SegQueue` worker-stealer pools, a comprehensive audit revealed **3 critical**, **4 high**, and **5 medium** severity issues across the codebase.

---

## 🔴 Critical Issues

### C1: Pending Counter Race Condition — `dragonforce.rs`, `pear.rs`

**Lines**: `dragonforce.rs:300`, `pear.rs:163`

Both adapters use a bare `pending_clone.fetch_sub(1, ...)` at the bottom of the loop body. If **any** code path above it panics, errors, or `continue`s early (e.g., a Tor timeout on `client.get()`, a JSON parse failure, or a `tokio::time::timeout` expiry), the pending counter is **never decremented**, causing all 120 workers to spin-wait indefinitely on the `pending > 0` check.

**Impact**: Crawl hangs forever on Tor network failures instead of terminating gracefully.

**Fix**: Apply the same RAII `TaskGuard` pattern from the fixed `inc_ransom.rs`:

```rust
struct TaskGuard { counter: Arc<AtomicUsize> }
impl Drop for TaskGuard {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::SeqCst);
    }
}
let _guard = TaskGuard { counter: pending_clone.clone() };
// ... rest of loop body (remove manual fetch_sub at end)
```

### C2: Silent Data Loss on HTTP Failure — `dragonforce.rs`, `pear.rs`, `autoindex.rs`

When a Tor circuit fails mid-request, these adapters silently skip the entire folder subtree. There is **no retry mechanism** — a single `502 Bad Gateway` or socket timeout permanently discards potentially thousands of nested files.

**Evidence**: INC Ransom returned 84,850 files instead of 106,048 (-20%) before the 5-pass retry was added.

**Fix**: Implement exponential backoff retry loops (3-5 attempts) with jitter, similar to the new `inc_ransom.rs` pattern.

### C3: Infinite Loop in `autoindex.rs` — Line 177

```rust
loop {
    match tokio::time::timeout(..., client.get(&next_url).send()).await {
        Ok(Ok(resp)) => { ... break; }
        _ => {}
    };
    break; // Break if timeout or reqwest error
}
```

This `loop` always executes exactly **once** — the `break` on line 196 fires unconditionally after the first attempt. This is effectively dead code pretending to be a retry loop.

**Fix**: Replace with a proper `for _attempt in 0..5` bounded retry with backoff.

---

## 🟠 High Severity Issues

### H1: Inconsistent Indentation Across All Adapters

The code inside `workers.spawn(async move { loop { ... }})` uses mixed indentation levels (8-space, 24-space, 28-space) across different adapters. This was the **root cause** of the `inc_ransom.rs` compilation failure — a `}` looked correct visually but closed the wrong scope.

**Fix**: Run `rustfmt` on all adapter files and enforce it via CI.

### H2: `worldleaks.rs` is a Mock Emulator — Not a Real Crawler

`worldleaks.rs` uses `rand::random()` to generate fake discovery entries and `tokio::time::sleep(150ms)` to simulate network latency. It never actually contacts the WorldLeaks `.onion` server.

**Impact**: Passes all regression tests but provides zero real-world value. It contaminated our regression report showing it as a "PASS" when it has never actually crawled.

**Fix**: Either implement real crawling logic or clearly mark it as `[MOCK]` in the regression output.

### H3: No Circuit Rotation on Failure — `pear.rs`, `dragonforce.rs`

Both adapters call `f.get_client()` once at the start of each loop iteration but re-use the same `client` for all retry attempts inside the inner `for` loop. If a Tor circuit is blacklisted or rate-limited by the target, all retries hit the same dead circuit.

`dragonforce.rs:254-256` correctly calls `f.get_client()` inside each retry — but `pear.rs:98` does too, so this is actually only an issue for `autoindex.rs:164` and `inc_ransom.rs` (now fixed).

**Fix**: Move `f.get_client()` inside the retry loop for `autoindex.rs`.

### H4: Missing `_attempt` Usage — Logging Gap

In all retry loops, the `_attempt` variable is prefixed with `_` to suppress unused-variable warnings, but it should be logged for diagnostic visibility when a folder fails after 5 retries.

**Fix**: Log the failed path and attempt count:
```rust
eprintln!("[WARN] Failed fetching {} after {} attempts", safe_path, _attempt);
```

---

## 🟡 Medium Severity Issues

### M1: `tokio::sync::Mutex` Contention on `all_discovered_entries`

All 120 workers contend on a single `tokio::sync::Mutex<Vec<FileEntry>>` every time they flush results. Under peak throughput (e.g., Pear at 380K files), this creates a significant serialization bottleneck.

**Fix**: Replace with `crossbeam_queue::SegQueue<FileEntry>` for the results collector too, and drain it at the end. This eliminates all lock contention.

### M2: UI Channel Capacity — 500,000 vs 50,000 Inconsistency

| Adapter | Channel Capacity |
|---|---|
| `inc_ransom.rs` | 500,000 |
| `dragonforce.rs` | 50,000 |
| `pear.rs` | 500,000 |
| `autoindex.rs` | 500,000 |
| `play.rs` | 500,000 |
| `worldleaks.rs` | 500,000 |

`dragonforce.rs` uses 10x smaller capacity. Under peak load (49K files), this could theoretically apply backpressure to workers.

**Fix**: Standardize all adapters to 500,000.

### M3: Missing Cancellation Check Inside Retry Loops

The `is_cancelled()` check only happens at the top of the outer `loop`. If a user clicks "Cancel" during a 5-pass retry with exponential backoff (up to 16s between attempts), the worker won't respond for up to 31 seconds.

**Fix**: Add `if f.is_cancelled() { break; }` at the start of each retry iteration.

### M4: `record_success()` Called on Failed Requests — `inc_ransom.rs`

In the new retry loop, `f.record_success(cid, 4096, ...)` is called **before** checking whether the response was successful. This pollutes the CircuitScorer with false positive latency data.

**Fix**: Move `record_success` inside the `if resp.status().is_success()` block and call `record_failure` in the `else` branch.

### M5: HEAD Requests Block Workers — `autoindex.rs`, `play.rs`

When `f.active_options.sizes` is true, workers perform synchronous `HEAD` requests for every file to get `Content-Length`. For Pear's 380K files, this would generate 380K additional HTTP requests, effectively doubling the crawl time.

**Fix**: Batch HEAD requests or skip size resolution during crawl phase and defer it to the download phase where sizes are inherently available from `Content-Length` headers.

---

## Architecture Recommendations

### R1: Extract Common Worker Pool Pattern

All 6 adapters copy-paste the same 40-line worker pool boilerplate (queue setup, pending counter, UI batcher, JoinSet spawn loop). This should be extracted into a shared utility:

```rust
// In frontier.rs or a new worker_pool.rs
pub async fn run_worker_pool<F, Fut>(
    frontier: Arc<CrawlerFrontier>,
    app: AppHandle,
    initial_seeds: Vec<String>,
    max_concurrent: usize,
    process_item: F,
) -> Vec<FileEntry>
where
    F: Fn(Arc<CrawlerFrontier>, String) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Vec<(FileEntry, Vec<String>)>> + Send,
```

### R2: Implement Adaptive Concurrency

The hardcoded `120` worker cap is aggressive for smaller targets (Play has 11 files). Consider adaptive scaling:
- Start with 8 workers
- Double every 10 seconds if queue depth > workers × 2
- Cap at 120

### R3: Add Structured Telemetry

Replace ad-hoc `println!` logging with structured telemetry:
- Per-worker success/fail/retry counters
- Queue depth over time  
- Pending counter watermark
- Circuit rotation frequency

---

## Priority Matrix

| ID | Severity | Effort | Impact | Recommendation |
|---|---|---|---|---|
| C1 | 🔴 Critical | Low | High | Apply RAII TaskGuard to dragonforce + pear |
| C2 | 🔴 Critical | Medium | High | Add retry loops to dragonforce + pear + autoindex |
| C3 | 🔴 Critical | Low | Medium | Fix autoindex infinite loop → bounded retry |
| H1 | 🟠 High | Low | Medium | Run rustfmt, add to CI |
| H2 | 🟠 High | High | Medium | Implement real WorldLeaks crawler |
| M1 | 🟡 Medium | Low | Medium | Replace Mutex results collector with SegQueue |
| M4 | 🟡 Medium | Low | Low | Move record_success after status check |
| R1 | 💡 Arch | High | High | Extract shared worker pool utility |

---

## Prevention Rules

> [!CAUTION]
> **PR-1**: Never use bare `pending.fetch_sub()` at the end of a loop body. Always use RAII drop guards.

> [!CAUTION]  
> **PR-2**: Never skip a folder on HTTP failure without retrying at least 3 times with exponential backoff.

> [!WARNING]
> **PR-3**: Always run `rustfmt` after modifying deeply nested async closures. Visual indentation can mask scope errors.

> [!WARNING]
> **PR-4**: Always run the Python brace checker (`/tmp/brace_checker.py`) before committing changes to adapter worker pools.

> [!IMPORTANT]
> **PR-5**: When adding retry loops, always include `if f.is_cancelled() { break; }` inside the retry to maintain cancellation responsiveness.
