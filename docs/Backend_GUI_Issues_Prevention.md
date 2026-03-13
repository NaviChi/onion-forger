# Backend & GUI Issues & Prevention Rules

## Backend Encounters & Prevention

### Issue 1: Thread Contention on FileEntry Dispatch
**Encounter**: The Phase 32 autoindex parser floods the `ui_tx` channel. `clone()` on structs in the loop spiked memory bandwidth.
**Fix**: Phase 33 — Batching + `append(&mut vec)` move-semantics replaces `extend(clone)`.
**Prevention Rule 1**: NEVER send cloned structs across channels if a single producer/consumer relationship exists. Use `Arc` wrappers or zero-copy inter-thread buffers (Ringbuffers/Disruptor pattern) for UI dispatch. Use `append()` (move) over `extend()` (clone) when transferring `Vec` ownership.

### Issue 2: Zombie Tor Sockets hanging Workers
**Encounter**: 45s timeout on `reqwest` allowed 24 workers to infinitely block if the exit node blackholed the TCP handshakes without returning RST.
**Fix Phase 27**: Exponential backoff.
**Fix Phase 33**: Connect timeout tightened to 10s (distinct from 120s read timeout). Retry count reduced from 7→5. Backoff capped at 16s (was 128s, wasting 4.8 minutes per failing node).
**Prevention Rule 2**: ALWAYS implement absolute bounds on TCP Handshake (Connect timeout) distinct from Read/Write timeouts. Never exceed 10s for Tor Connect. Cap exponential backoff to prevent blocking worker threads >= 30s cumulative.

### Issue 3: Regex Recompilation on Every can_handle() Call
**Encounter**: `regex::Regex::new()` was called inside `can_handle()` on every site fingerprint check, triggering full NFA compilation each time.
**Fix Phase 33**: Promoted to `static LazyLock<regex::Regex>` for single compilation.
**Fix Phase 34**: Also applied to Stage B regex in `qilin_nodes.rs`.
**Prevention Rule 4**: NEVER compile regex patterns inside hot paths. All patterns MUST be `static LazyLock` or `OnceCell` initialized at first use.

### Issue 4: Static Concurrency Window
**Encounter**: The 24-worker ceiling was hardcoded. Under 429 rate-limiting or Tor circuit brownouts, all 24 workers would simultaneously retry, creating a thundering herd.
**Fix Phase 33**: Activated real AIMD governor: 429 responses trigger multiplicative decrease (halve), sustained success triggers additive increase.
**Prevention Rule 5**: NEVER use static concurrency ceilings for network crawlers. ALWAYS implement dynamic backpressure (AIMD, BBR, or Vegas) that responds to server signals (429, 503, timeout).

### Issue 5: Wasteful 50ms Poll-Sleep on Empty Queue
**Encounter**: When the work queue is empty, 24 workers each burn a 50ms tokio timer (20 wakeups/sec/worker = 480 wakeups/sec total for zero useful work).
**Fix Phase 34**: Replaced with `tokio::sync::Notify`. Workers sleep until notified by a queue push, with 200ms timeout backstop.
**Prevention Rule 6**: NEVER use fixed-interval polling for work queues. ALWAYS use event-driven wakeup (`Notify`, `Condvar`, `eventfd`) with a safety timeout backstop.

### Issue 6: Sequential Node Probing in Discovery
**Encounter**: `qilin_nodes.rs` Stage D probed N storage nodes sequentially (N×30s worst case for 3 nodes = 90s).
**Fix Phase 34**: Converted to `JoinSet` concurrent probing (max(20s) regardless of node count).
**Prevention Rule 7**: NEVER probe independent network endpoints sequentially. ALWAYS fan out with `JoinSet`/`FuturesUnordered` and collect results concurrently.

### Issue 7: Unnecessary String Allocation from resp.text()
**Encounter**: `resp.text().await` performs UTF-8 validation and allocates a new `String`. The memchr parser then converts back to `&[u8]`.
**Fix Phase 34**: Replaced with `resp.bytes().await` + `String::from_utf8_lossy().into_owned()`.
**Prevention Rule 8**: Prefer `resp.bytes()` over `resp.text()` when the body will be parsed as byte slices. Only convert to String when absolutely needed for string-based APIs.

## GUI Encounters & Prevention

### Issue 1: Tauri IPC JSON Serialization Overhead
**Encounter**: Emitting 500-element arrays of `FileEntry` via Tauri IPC caused massive UI thread stalls in React.
**Fix**: Debounced emissions and batched payload rendering.
**Prevention Rule 3**: NEVER serialize > 100 DOM-bound objects at once over IPC. If the result set exceeds 1,000 items, pagination or virtualized rendering via ArrayBuffer (binary IPC) MUST be used instead of JSON.

*Last Updated: 2026-03-12 Phase 34*
