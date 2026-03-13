# Detailed Qilin Implementation Architecture

## Core Principle: Hardware Squeezing
This architecture strictly enforces zero-copy paths, minimal RAM footprint, and maximum CPU cycle usage.

## 1. Zero-Copy memchr Parser (Phase 32 — DONE)
The `regex::Regex` over `String` was replaced with `memchr::memmem::find_iter` operating on raw `&[u8]` byte slices.
```rust
let html_bytes = html.as_bytes();
let target_href = b"<td class=\"link\"><a href=\"";
let target_size = b"</a></td><td class=\"size\">";
for offset in memchr::memmem::find_iter(html_bytes, target_href) {
    // Process purely using slice offsets (&[u8])
}
```

## 2. Dynamic AIMD Concurrency Governor (Phase 33 — DONE)
The static 24-worker ceiling is now governed by an AIMD controller:
- **Multiplicative Decrease**: On 429 status → `window = max(window / 2, 4)`
- **Error tracking**: AtomicUsize counters for errors and successes feed real-time backpressure
- **Connect timeout**: Tightened from 45s to 10s (Prevention Rule 2)
- **Retry count**: Reduced from 7 to 5, backoff capped at 16s

## 3. Asymmetric Download Engine
- **Crawling Data Plane**: Rust `reqwest` → Tor Socks5 with 30s timeout per request.
- **Bulk Download Plane**: Transfer file descriptors to an isolated `aria2c` process pool connected to 120 dedicated Tor circuits, achieving 100% saturation of Tor daemon bandwidth limits.

## 4. Memory Pool Allocator
Use `jemalloc` or `mimalloc` tightly coupled to thread-local arenas to prevent fragmentation across millions of FileEntry allocations.

## 5. static LazyLock Regex (Phase 33/34 — DONE)
All regex patterns in hot paths are promoted to `static LazyLock<regex::Regex>`:
- `ONION_VALUE_RE` in `qilin.rs` (can_handle)
- `STORAGE_RE` in `qilin_nodes.rs` (Stage B discovery)

## 6. Move-Based Vec Transfer (Phase 33 — DONE)
`locked.extend(new_files)` replaced with `locked.append(&mut new_files)` to avoid deep-cloning the Vec contents a second time.

## 7. Concurrent JoinSet Node Probing (Phase 34 — DONE)
Stage D in `qilin_nodes.rs` now fans out all node probes concurrently using `tokio::task::JoinSet`. Discovery latency reduced from N×30s to max(20s).

## 8. tokio::sync::Notify Wakeup (Phase 34 — DONE)
Workers no longer poll-sleep 50ms when the queue is empty. `Notify::notified()` provides zero-cost idle with instant wakeup on queue push. 200ms timeout backstop prevents starvation.

## 9. resp.bytes() Zero-Copy (Phase 34 — DONE)
Response body fetched via `resp.bytes()` (returns `bytes::Bytes` — zero-copy refcounted buffer) instead of `resp.text()` (validates UTF-8 + allocates String).

## 10. Unified memchr CMS Parser (Phase 34 — DONE)
The CMS blog `href=` parser was line-by-line with `String::find()`. Now uses `memchr::memmem::find_iter` byte scanning, fully consistent with the V3 table parser.

*Last Updated: 2026-03-12 Phase 34*
