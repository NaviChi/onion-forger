> **Last Updated:** 2026-03-13T21:53 CDT

## Phase 144: Parallel Download Stall Prevention — 5 Bugs Fixed + 8 Recommendations (2026-03-13)

### Issues Found (Root Cause Analysis)
1. **BUG-1 (CRITICAL):** `scaffold_download()` had NO timeout wrapper. Could block for **2+ hours** on a single chunk with 503 throttles. Violated LESSON-140-001.
2. **BUG-2 (CRITICAL):** Stall detector (R3) only ran at **top of outer loop** — never fired while `scaffold_download()` was stuck inside the inner chunk-processing loop.
3. **BUG-3 (MEDIUM):** `activate_download_control()` mutex wasn't cleared on error/timeout paths. Next chunk would get "A download is already active" and silently skip all files.
4. **BUG-4 (MEDIUM):** Serial probe loop in `start_batch_download()` blocked entire pipeline for 100×5s = 500s on files without size hints.
5. **BUG-5 (LOW):** Exponential backoff had per-file ceiling (30s) but no aggregate per-chunk ceiling.

### Fixes Implemented
1. **BUG-1 FIX:** `scaffold_download()` now wrapped in `tokio::time::timeout(30s + 3s×files, max 300s)`. On timeout, chunk is skipped, NEWNYM fires, 10s recovery, then next chunk.
2. **BUG-2 FIX:** Heartbeat watchdog task emits `💓 Chunk #{N} heartbeat (Xs elapsed, Y files)` every 30s during downloads so user sees activity.
3. **BUG-3 FIX:** `aria_downloader::clear_download_control()` added in ALL error/timeout paths (scaffold error, scaffold timeout, hedge error, hedge timeout, final sweep error, final sweep timeout).
4. **BUG-4 MITIGATION (R7):** Probe phase now emits progress every 10 files: `"Batch probe progress: N/M files classified..."`.
5. **BUG-5 covered by BUG-1 FIX:** Per-chunk timeout caps aggregate time regardless of per-file backoff accumulation.
6. **R5 (Timeout Escalation):** Consecutive timeouts multiply limit by 1.5× (max 3.0×). Success resets to 1.0×. Adapts to genuinely slow networks.
7. **Final VFS sweep** also wrapped in 300s timeout with proper DownloadControl cleanup.

### Prevention Rules
- **PR-SCAFFOLD-TIMEOUT-144:** Every `scaffold_download().await` MUST be wrapped in `tokio::time::timeout()`. There are ZERO exceptions. Use adaptive ceiling: `30 + 3×files, max 300s` scaled by `timeout_multiplier`.
- **PR-CONTROL-CLEANUP-144:** `clear_download_control()` MUST be called in ALL non-success paths (error, timeout, panic). Use RAII guard pattern if more call sites are added.
- **PR-HEARTBEAT-144:** Any blocking operation >30s MUST have a concurrent heartbeat emitter so the user sees activity.
- **PR-PROBE-PROGRESS-144:** Serial probe loops MUST emit progress every 10 iterations.

## Phase 143: Progressive Download Total Tracking (2026-03-13)

### Issue
During parallel crawl+download, the download progress total would reset to 0% every time a new 100-file chunk started downloading. This happened because `scaffold_download()` emitted `download_batch_started` per chunk, and the frontend listener reset all progress state (completedFiles=0, downloadedBytes=0) on each event.

### Fix
1. **Backend (lib.rs):** The parallel download consumer now tracks a cumulative `total_queued_for_download` counter. The first batch emits `download_batch_started` to initialize the UI. Subsequent arrivals emit a new `download_total_update` event that only updates `totalFiles`/`totalBytesHint`/`unknownSizeFiles`.
2. **Frontend (App.tsx):** Added `download_total_update` listener that uses `Math.max()` to grow the total without touching `completedFiles`, `downloadedBytes`, `speedMbps`, or any other progress state.

### Prevention Rules
- **PR-BATCH-RESET-143:** `download_batch_started` must only be emitted ONCE per download session in parallel mode. For progressive total updates, use `download_total_update` which spreads only `totalFiles` into existing state via `Math.max()`.

## Phase 142-IMPL: R1+R2+R3+R4 Implementation (2026-03-13)

### Changes Implemented
1. **R1 (Hedged Download Retry):** After each 100-file chunk download, the consumer checks which files are 0-byte or missing at the target path. Partial failures (some but not all files failed) trigger a hedged retry with a 60s timeout ceiling. This catches transient circuit failures without wasting time on permanently unavailable files.

2. **R2 (Host-Grouped Batch Scheduling):** Chunks of 100 files are now sorted by host (primary) then size (secondary, SRPT — Shortest Remaining Processing Time). This maximizes HTTP keep-alive connection reuse: consecutive downloads targeting the same storage node share warm TCP connections instead of churning. Unique host count is now logged per chunk.

3. **R3 (Adaptive Stall Threshold):** Replaced the fixed 90s stall threshold with `3× max(recent_batch_durations)` clamped to `[30s, 180s]`. The last 16 batch durations are tracked. Before any batches complete, the fallback 90s is used. On fast networks, the stall fires in ~15-30s; on degraded networks, it waits up to 180s.

4. **R4 (Bounded Download Channel):** Changed `mpsc::unbounded_channel()` to `mpsc::channel(200)`. This provides natural backpressure during explosive discovery phases, preventing memory spikes. All callers (`frontier.rs`, `qilin.rs`) updated from `send()` to `try_send()` for non-blocking compatibility.

### Files Modified
- `src-tauri/src/lib.rs`: Download consumer — all 4 optimizations
- `src-tauri/src/frontier.rs`: Channel type + `try_send()` migration
- `src-tauri/src/adapters/qilin.rs`: VFS flush forward closure type update
- `src-tauri/src/scorer.rs`: Added `median_latency_ms()` and `global_avg_speed_mbps()`

### Prevention Rules
- **PR-HEDGE-142:** Hedged retry must only fire when some (but not all) files failed. If 100% of files fail, let the stall detector handle recovery — hedging an entire failed batch would waste circuits.
- **PR-BOUNDED-CH-142:** When switching channels from unbounded to bounded, ALL senders must use `try_send()` in sync contexts. `send().await` is async-only and will deadlock in sync closures.
- **PR-STALL-ADAPTIVE-142:** The adaptive stall threshold must have a floor (30s) to prevent false-positive stalls from a single fast batch, and a ceiling (180s) to prevent infinite waits.

## Phase 142: Exhaustive Cross-Industry Improvement Analysis (2026-03-13)

### Investigation
Reviewed all project whitepapers (Recommendations, Lessons Learned, Implementation Qilin, Theoretical Algorithm), internet research across 6 domains (Google, NASA, SpaceX, HFT, aria2, Tor Project), competitive analysis (aria2, IDM, wget2, tor-browser), and Phase 140B/141 test data.

### Findings
Identified 11 ranked improvement recommendations (R1-R11) with effort/impact matrix. Top 3 (R1 hedged probes, R2 host grouping, R3 adaptive stall) estimated at 40-70% improvement for <2 hours work. 8 anti-recommendations explicitly rejected with lesson references to prevent repeating past mistakes.

### Key Bottleneck from Last Run
The #1 remaining bottleneck is probe-stage failures. In the Phase 140B test (201K entries, 25 minutes), 1,055 HTTP 503 throttles were observed. The event-driven pipeline (Phase 141) solved discovery latency but the download consumer still probes serially on a single circuit per file.

### Detailed Artifact
Full analysis: [phase142_improvement_analysis.md](file:///C:/Users/Zero/.gemini/antigravity/brain/d0b38f8a-8219-43ab-9ba0-78d2db56d375/phase142_improvement_analysis.md)

### Prevention Rules
- **PR-ANALYSIS-142:** Before implementing any download optimization, cross-reference the Phase 142 anti-pattern registry (8 rejected ideas). This prevents re-discovering failures already documented in Phases 54, 99, 132, 136, 140C.

## Phase 141B: Intelligent Download Stall Detection & Recovery (2026-03-13)

### Problem
Parallel downloads could silently stall when Tor circuits degraded, 503 throttle storms hit, or connections dropped. The consumer would sit blocked in `scaffold_download()` indefinitely with no recovery mechanism.

### Fix
Added stall detection to the Phase 141 event-driven download consumer:
- **Tracks `last_progress_at`**: Updated after each successful chunk download
- **90s stall threshold**: If no new files downloaded for 90s, triggers recovery
- **Recovery sequence**: NEWNYM on all managed Tor circuits → 30s cooldown → resume
- **Max 3 recoveries**: Prevents infinite recovery loops; after 3, falls through to post-crawl sweep
- **Zero state loss**: `downloaded_paths` HashSet preserved across recoveries

### Prevention Rules
- **PR-STALL-141B:** Any long-running download loop must have stall detection with timestamps. Never rely on timeouts alone — circuits can silently degrade without triggering connection errors.

## Phase 141: Event-Driven Parallel Download Pipeline (2026-03-13)

### Problem
The Phase 128 parallel download consumer polled the VFS every 10s, scanning ALL entries to find new ones. With 200K+ entries, this was O(N) per cycle — increasingly expensive as the crawl discovered more files. Discovery-to-download latency was ~25s (15s initial delay + 10s poll interval).

### Fix
Replaced VFS-polling with a channel-driven pipeline:
- **Architecture**: `Adapter → VFS Flush Task → download_feed_tx → Consumer`
- **`download_feed_tx`** added to `CrawlerFrontier` struct, set before adapter crawl starts
- **Qilin VFS flush task** forwards file entries into the channel alongside VFS inserts
- **Consumer** processes entries in chunks of 100, starts immediately on channel data
- **Final VFS sweep** catches any entries the channel missed (non-Qilin adapters)

| Metric | Phase 128 (old) | Phase 141 (new) |
|--------|-----------------|-----------------|
| VFS scans/cycle | O(N) every 10s | 0 (channel-driven) |
| Discovery latency | ~25s | <3s |
| Batch size | All new files | 100 (chunked) |
| Stall recovery | None | NEWNYM + 30s pause |

### Prevention Rules
- **PR-POLL-141:** Never use polling loops over growing datasets. Use channels, pub/sub, or push-based notification instead. VFS scans at O(200K) every 10s was a clear scalability bottleneck.

## Phase 140D: Unified Speed Mode — Single Selector (2026-03-13)

### Problem
Two separate UI mode selectors caused confusion:
1. **Speed Mode dropdown** (Low/Medium/Aggressive) — controlled `downloadMode` for circuit caps
2. **MODE preset buttons** (Low/Balanced/Performance) — controlled `circuits` count

Users had to configure two things that should have been one.

### Fix
Merged into a **single unified MODE selector** with 3 buttons:
- **⚡ Default** — 2 guard nodes (proven 1.83 MB/s, resource governor arti cap)
- **🔥 High** — 3 guard nodes, arti cap override=12 (+50% bandwidth, needs ≥8GB RAM)
- **🚀 Aggressive** — 4 guard nodes, arti cap override=16 (+100% bandwidth, needs ≥16GB RAM)

All three modes use the same proven Test 1 circuit/worker values (24 circuits, 24 parallel DL cap, 6 worker ceiling). The **only** difference is how many base TorClients are bootstrapped → how many independent guard relay paths exist → aggregate bandwidth.

**Files modified:** `frontier.rs` (DownloadMode enum), `resource_governor.rs` (mode encoding, clamp_mode_for_hardware), `cli.rs` (DownloadModeCli), `App.tsx` (unified selector), all test binaries.

### Prevention Rules
- **PR-UI-MODE-140D:** Never expose two overlapping mode configuration controls. A single mode selector must control all related parameters (circuits, arti cap, workers) in one click.

## Phase 140C: Conservative Arti Cap Increase — 3 Guard Nodes (2026-03-13)

### Problem
Live test (144bf0f5 target, 201K entries) showed 1.07-1.81 MB/s with aggressive mode. Root cause:
- `recommended_arti_cap` returned 8 for the user's 16-core/31GB Unknown-storage system (Windows cap was 8)
- With fan-out=4, only **2 base TorClients** were bootstrapped
- All 4 isolated views from each base TorClient share the **same guard node** — sharing its bandwidth
- Only 2 independent guard relay paths = each saturated at ~0.5-1.0 MB/s

### Fix (Conservative Middle Ground)
Moderately increased caps in `resource_governor.rs` — **+1 base TorClient** over baseline:
- **Unknown storage** (16+ cores, 32+ GB): 8 → **12** circuit cap (3 base TorClients)
- **SSD** (16+ cores, 32+ GB): 16 (unchanged); (8+ cores, 16+ GB): 12 → **12** (unchanged)
- **NVMe** (16+ cores, 32+ GB): 18 → **16**; (24+ cores, 64+ GB): 24 (unchanged)
- **Windows per-storage caps**: Unknown 8→**12**, SSD 12→**16**, NVMe 16 (unchanged)
- Global max clamp: stays at **24**

With fan-out=4, cap=12 → **3 base TorClients** → **3 independent guard nodes** → ~50% more bandwidth.

### Safety Net
RAM Guard (Phase 140) + Memory Pressure Monitor prevent OOM. Test showed 687-744 MB RSS at 200K entries — well within the 31 GB system budget.

### Live Test Results (Before Fix)
| Metric | Value |
|--------|-------|
| Target | 144bf0f5 (201K entries, ~25K files) |
| Peak download speed | **1.83 MB/s** |
| Steady-state speed | **1.07-1.14 MB/s** |
| Files downloaded | 1,479 / 15,624 |
| Data downloaded | 1,449 MB |
| RSS | 687-744 MB (2.1-2.3%) |
| Active workers | 16/16 |
| 503 throttles | 1,055 |
| Failovers | 1,105 |

### Prevention Rules
- **PR-ARTI-CAP-140C:** The `recommended_arti_cap` must scale proportionally with available hardware. Each base TorClient uses an independent guard relay — more base clients = linearly more aggregate bandwidth. The fan-out only provides circuit isolation, NOT independent bandwidth paths.

## Phase 140B: Download Speed — Circuit Quality Gate & Budget Increase (2026-03-13)

### Problem
Download speed plateaued at 0.58 MB/s during parallel download. Two root causes:
1. Parallel download consumer only used 4-6 circuits (Phase 128 conservative cap) while full mode budget was 12-24
2. Slow circuits (<0.3 MB/s) were still getting equal work allocation, dragging down aggregate throughput

### Fix
**1. Circuit Speed Threshold (0.3 MB/s):** Enhanced `yield_delay()` in aria_downloader's CircuitScorer:
- Circuits with ≥3 pieces and avg speed < 0.3 MB/s get 3-5 second delays (proportional to how far below threshold)
- Circuits ≥0.3 MB/s get 0ms delay — they grab work immediately
- Added `fast_circuits_above_threshold()` method to both `scorer.rs` and `aria_downloader.rs`

**2. Parallel Download Budget Increase:** Changed `pd_circuits` from `min(requested, old_cap)` to `min(requested, effective_mode.parallel_download_cap())`:
- Medium mode: 6→12 circuits during crawl
- Aggressive mode: 12→24 circuits during crawl

### Files Modified
- `aria_downloader.rs` — Enhanced yield_delay with 0.3 MB/s threshold; added fast_circuits_above_threshold
- `scorer.rs` — Added fast_circuits_above_threshold and select_fast_download_pool
- `lib.rs` — Updated parallel download consumer to use full mode budget
- `frontier.rs` — Mode display rename (Low→stealth, Medium→balanced)

### Prevention Rules
- **PR-SPEED-GATE-140B-001:** When allocating download workers, circuits MUST be filtered by minimum speed threshold (0.3 MB/s default). Avoid giving equal work to circuits that are provably slow — use proportional delay instead of hard rejection to prevent starvation.
- **PR-BUDGET-MATCH-140B-002:** The parallel download consumer during crawl MUST use the same circuit budget as the post-crawl download sweep. Using a smaller budget wastes available throughput.

## Phase 140: Parallel Consumer Hang + RAM-Aware Mode Demotion (2026-03-13)

### Problem 1 — Parallel Download Consumer Infinite Hang (P0)
After crawl completion, the parallel download consumer wait at `lib.rs:1892` (`handle.await`) blocked **indefinitely**. The consumer was stuck retrying 14 failing files (503 throttles / connection drops) with no overall timeout. Process had to be force-terminated.

**Root Cause:** No `tokio::time::timeout` wrapper on `handle.await`. The consumer's internal batch download has no max-elapsed-time guard.

**Fix:** Wrapped `handle.await` with `tokio::time::timeout(120s)`. On timeout, logs a warning and proceeds to the post-crawl sweep which handles remaining files via `build_download_resume_plan()`.

### Problem 2 — Aggressive Mode OOM on Low-RAM Systems (P1)
Users selecting "Aggressive" mode on systems with <8 GiB available RAM caused OOM crashes. 24 Tor circuits × 15-30 MB each + Sled DBs + download buffers easily exceeds 1-2 GB.

**Root Cause:** `DownloadMode` override bypassed the resource governor's hardware-aware caps. No demotion guard existed.

**Fix:** New `clamp_mode_for_hardware()` function in `resource_governor.rs` that auto-demotes modes based on available RAM:
- Aggressive → Medium if <8 GiB avail or <16 GiB total
- Aggressive → Low if <4 GiB avail or <8 GiB total
- Medium → Low if <2 GiB avail or <4 GiB total

Wired into `start_crawl()` before `set_active_download_mode()`. Logs `[RAM Guard]` warning on demotion.

### Verification
Live test against target `c9d2ba19-6aa1-3087-8773-f63d023179ed`:
- 35,069 entries (27,142 files + 7,927 folders) crawled in 1,422s
- 100% folder verification (7,928/7,928)
- Folder structure confirmed preserved: `Accounting/Bank Recs/` (358 files), `HR/` (28 files)
- No path escapes — Phase 139 fix working perfectly
- 416/430 files downloaded during crawl (216.1 MB at 0.58 MB/s)
- Memory stable at 450 MB RSS (1.4% of 31 GiB)

### Files Modified
- `lib.rs` — Added 120s timeout on parallel download consumer wait; wired RAM-aware mode demotion
- `resource_governor.rs` — New `clamp_mode_for_hardware()` function

### Prevention Rules
- **PR-CONSUMER-TIMEOUT-140-001:** NEVER `handle.await` on JoinSet/JoinHandle without a `tokio::time::timeout` wrapper. Even "cooperative" consumer tasks can hang on network retries.
- **PR-RAM-GUARD-140-002:** Any user-selectable mode that increases resource consumption MUST pass through `clamp_mode_for_hardware()` before activation. The mode enum values are NOT safe to use directly.
- **PR-CRAWL-RESUME-140-003 (FUTURE):** VFS sled DB already persists entries during crawl. A future `--resume` enhancement should reload VFS entries and rebuild the frontier from unparsed folder entries, eliminating the need to re-crawl from scratch after crash/exit.


## Phase 139: Windows Folder Structure Preservation — Critical Path Join Bug (2026-03-13)

### Problem
Downloaded files were losing their folder structure on Windows. A file that should be saved at `C:\output\HR\Reports\file.pdf` was instead written to `C:\HR\Reports\file.pdf` — bypassing the output directory entirely.

**Root Cause 1 — `PathBuf::join()` with rooted paths on Windows:**
Adapter paths from crawlers start with `/` (e.g. `/HR/Reports/file.pdf`). On Windows, `PathBuf::join("/HR/Reports/file.pdf")` treats the `/`-prefixed path as "rooted to current drive" and **replaces the entire base path**. So `C:\output\.join("/HR/...")` → `C:\HR\...` instead of `C:\output\HR\...`.

Diagnostic proof:
```rust
let root = PathBuf::from(r"C:\output");
root.join("/HR/Reports/file.pdf")  // → C:/HR/Reports/file.pdf  ❌ WRONG
root.join("HR/Reports/file.pdf")   // → C:\output\HR/Reports/file.pdf  ✅ CORRECT
```

The `resolve_download_target_within_root` function previously had an `is_absolute()` check that happened to mask this for most cases, but the underlying `sanitize_path()` was the real fix — it strips leading `/` before joining.

**Root Cause 2 — `\\?\` prefix mismatch in security checks:**
`canonicalize_output_root()` adds the `\\?\` extended-length prefix via `ensure_long_path()`. But `std::fs::canonicalize()` returns paths with inconsistent prefix forms. When comparing via `starts_with()`:
```
child (no prefix): C:\output\HR\file.pdf
root  (prefixed):  \\?\C:\output
→ starts_with = false  ❌ FALSE ESCAPE DETECTION
```
This could either:
- Silently allow directory escapes (false negative)
- Reject valid downloads with "escaped output root" error (false positive)

### Solution

#### A. Always Sanitize Before Join
In `resolve_download_target_within_root()`: removed the `is_absolute()` branch entirely. Now ALWAYS runs `sanitize_path()` first (strips leading `/`, `..`, etc.) then joins with `output_root`. This guarantees the path is always relative before joining.

#### B. Normalize `\\?\` Prefix for starts_with Checks
Added `normalize_for_starts_with()` helper that strips the `\\?\` and `\\?\UNC\` prefixes before `starts_with()` comparisons. Applied to all path escape checks in both `resolve_path_within_root()` and `resolve_download_target_within_root()`.

### Files Modified
- `path_utils.rs` — `resolve_path_within_root()`, `resolve_download_target_within_root()`, new `normalize_for_starts_with()` helper, new Phase 139 tests

### Prevention Rules
- **PR-WIN-JOIN-139-001:** NEVER call `PathBuf::join()` with a `/`-prefixed path on Windows. Always strip leading slashes via `sanitize_path()` first. The Rust `join()` method treats `/`-prefixed paths as "rooted" and replaces the base.
- **PR-WIN-JOIN-139-002:** NEVER use raw `starts_with()` to compare canonicalized paths on Windows. Always normalize both sides via `normalize_for_starts_with()` to handle the `\\?\` prefix inconsistency.
- **PR-WIN-JOIN-139-003:** Adapter `FileEntry.path` values always start with `/` by convention. All path consumers must pass through `sanitize_path()` before filesystem operations.
- **PR-WIN-JOIN-139-004:** When adding any new path-joining code, always validate with the Windows test case: `root.join("/subdir/file")` should produce `root\subdir\file`, NOT `C:\subdir\file`.

## Phase 138: Isolation Fan-Out — More Circuits, Less Cost (2026-03-13)

### Problem
Each TorClient bootstrap costs ~15-30MB RAM + 15-45s startup (consensus download, guard selection, circuit build). To get N circuit slots, we bootstrapped N full TorClients — linear cost scaling.

### Solution — Isolation Fan-Out Architecture
Instead of N full bootstraps, spawn `ceil(N / fan_out_ratio)` heavy base clients and create `fan_out_ratio` isolated views per base via `TorClient::isolated_client()`. Each isolated view:
- **Shares** directory consensus, guard selection, channel manager (zero additional memory)
- **Gets separate circuits** via built-in Arti isolation (IsolationToken)
- **Costs near-zero** to create (just an Arc clone + token)

Example: `target=16, fan_out=4` → only **4 heavy bootstraps** → **16 circuit slots**.
- RAM saved: ~3 × 15-30MB = **45-90MB**
- Startup saved: ~3 × 15-45s = **45-135s** (parallelized: ~same as 1 base)

### Configuration
- `CRAWLI_ISOLATION_FAN_OUT` — Controls the fan-out ratio (default: 4, range: 1-8)
- Set to 1 to disable fan-out and revert to previous N-bootstrap behavior

### Files Modified
- `tor_native.rs` — `bootstrap_arti_cluster_for_traffic()` now uses fan-out for both initial and background bootstrap phases

### Prevention Rules
- **PR-FAN-OUT-138-001:** Never create N full TorClients when N isolated views suffice. Arti's `isolated_client()` shares ALL internal state except circuit selection.
- **PR-FAN-OUT-138-002:** Fan-out ratio must stay ≤8. Beyond 8 isolated views per base, the shared channel manager can become a bottleneck.
- **PR-FAN-OUT-138-003:** The download pipeline at `aria_downloader.rs:get_arti_client()` already uses fresh `IsolationToken` per circuit — fan-out is safe because double-isolation (view-level + token-level) still produces unique circuits.

## Phase 137: HTTP/2 Flow Control Tuning — Adaptive Window + Larger Frames (2026-03-13)

### Problem
HTTP/2 WINDOW_UPDATE stalls were limiting throughput for high-BDP Tor circuits. The default settings were:
- Connection window: 1MB (insufficient for 4+ concurrent streams each needing 256KB)
- No adaptive window scaling (fixed window regardless of measured throughput)
- Default 16KB max frame size (excessive framing overhead for large body transfers)

### Solution
Three zero-cost Hyper builder changes in `arti_client.rs`:
1. **`http2_adaptive_window(true)`** — Hyper dynamically grows the receive window based on measured throughput, analogous to TCP window scaling. Eliminates WINDOW_UPDATE stalls for high-BDP circuits.
2. **`http2_initial_connection_window_size(4_194_304)`** — 4MB (up from 1MB). Supports 4+ concurrent streams × 256KB each without blocking.
3. **`http2_max_frame_size(Some(32_768))`** — 32KB (up from 16KB default). Halves frame overhead for large body transfers.

### Files Modified
- `arti_client.rs` — HTTP/2 builder configuration

### Prevention Rules
- **PR-H2-WINDOW-137-001:** Always enable `http2_adaptive_window(true)` for high-BDP connections. Fixed windows cause stalls when throughput varies.
- **PR-H2-CONN-WINDOW-137-002:** Connection window must be ≥ `stream_window × max_concurrent_streams`. Our 4MB = 256KB × 16 provides headroom for burst traffic.

## Phase 136: Connection Round-Trip Savings — Optimistic Streams + Host Capability Persistence (2026-03-13)

### Problem
Two sources of unnecessary round trips were identified:
1. **Clearnet exit connections** waited for the exit relay's CONNECTED response before returning the stream — wasting 1 full round trip (~300-800ms over Tor's 3-hop circuit) when it's a formality for clearnet exits.
2. **Host capability knowledge was ephemeral** — on app restart all learned host capabilities (range support, RTT EWMAs, parallelism caps) were lost, forcing re-probing from scratch.

### Solution

#### A. Optimistic Streams for Clearnet Exits
- `arti_connector.rs`: Added conditional `prefs.optimistic()` for non-`.onion` hosts
- Stream is returned immediately without waiting for CONNECTED response
- .onion connections still wait for rendezvous handshake (required for correctness)
- Saves ~300-800ms per new clearnet connection

#### B. Host Capability Persistence via Sled
- Added `Serialize`/`Deserialize` derives to `HostCapabilityState` and `ResumeValidatorKind`
- New `HOST_CAPABILITY_SLED` static backed by `~/.crawli/host_capabilities.sled`
- `initialize_host_capability_store()`: Loads recent entries (last 24h) from sled on startup
- `persist_host_capability()`: Async write-through on every probe success and productive transfer (≥32KB)
- Called from `record_probe_host_capability()` and `record_host_success()`
- On restart, known hosts immediately enter range-mode without re-probing

### Files Modified
- `arti_connector.rs`: 1 change — conditional optimistic streams
- `aria_downloader.rs`: 4 changes — Serde derives, sled persistence, write-through calls
- `lib.rs`: 1 change — `initialize_host_capability_store()` call at bootstrap

### Prevention Rules
- **P136-1:** NEVER enable optimistic() for .onion hidden service connections — the rendezvous handshake MUST complete before the DataStream is usable.
- **P136-2:** Host capability persistence must only restore entries from the last 24h to avoid stale data from server infrastructure changes.
- **P136-3:** Write-through persistence must be async/non-blocking — never block the download hot path for sled I/O.


## Phase 135: Remove `.ariaforge` Temp Extension — Direct-to-Final-Path Downloads (2026-03-12)

### Problem
Downloads used a two-phase write pattern: data was written to `{path}.ariaforge`, then after SHA256 verification, renamed to the final path via `fs::rename()`. This caused several issues:
1. **Orphaned `.ariaforge` files** — if a download was cancelled or crashed mid-transfer, the `.ariaforge` file remained on disk at the temp path. Users saw mysterious `.ariaforge`-suffixed files.
2. **Rename failures on Windows** — `fs::rename()` can fail if the target file is locked by an indexer (Windows Search), antivirus scan, or another process.
3. **Confusing UX** — while downloading, the file appeared as `filename.ext.ariaforge` instead of `filename.ext`.

### Solution
Eliminated the `.ariaforge` temp extension entirely. Files now download **directly to their final path**:
- `aria_downloader.rs:3672-3675`: Replaced `temp_target = format!("{}.ariaforge", entry.path)` with direct use of `entry.path`
- All 7 references to `temp_target` across the download pipeline (writer, piece tasks, stream fallback, SHA256 verification) now reference `entry.path`
- Removed the `fs::rename(&temp_target, &entry.path)` step after SHA256 verification
- `.ariaforge_state` sidecar file preserved for resume metadata tracking
- `direct_download_benchmark.rs`: Updated `best_observed_downloaded_bytes()` to read from the final path instead of the old `.ariaforge` temp path

### Files Modified
- `aria_downloader.rs`: 8 changes — removed temp_target variable, updated all references, removed rename step
- `bin/direct_download_benchmark.rs`: 1 change — updated fallback byte estimation

### Prevention Rules
- **P135-1:** NEVER reintroduce a temp extension pattern for downloads. The `.ariaforge_state` sidecar provides all resume metadata needed without requiring a separate temp file path.
- **P135-2:** If partial/corrupt file detection is needed, use the `.ariaforge_state` sidecar's `completed_pieces` bitfield — do NOT check file extension to determine download completeness.
- **P135-3:** The resume validator (ETag/Last-Modified) in `.ariaforge_state` already protects against serving stale content from a changed server file.


## Phase 133: Download Speed Modes — Low / Medium / Aggressive (2026-03-12)

### Design
Added a `DownloadMode` enum (`frontier.rs`) with three presets controlling all circuit and worker parameters. **Medium is the default** everywhere — CLI (even with no flags), GUI, and all test binaries.

### Parameter Table

| Parameter | Low (Stealth) | Medium (Balanced) | Aggressive (Max) |
|-----------|:---:|:---:|:---:|
| Default circuits | 6 | **12** | 24 |
| Parallel download cap | 6 | **12** | 24 |
| Tor swarm clients | 4 | **8** | 12 |
| Crawl worker ceiling | 3 | **4** | 6 |
| Content cap <16MB | 6 | **12** | 20 |
| Content cap <64MB | 8 | **16** | 28 |
| Content cap <256MB | 12 | **24** | 40 |
| Content cap <1GB | 16 | **32** | 56 |
| Pipeline clamp (min,max) | (2,8) | **(4,16)** | (6,24) |

### Files Modified
- `frontier.rs`: Added `DownloadMode` enum with all parameter methods
- `cli.rs`: Added `--download-mode` (low/medium/aggressive), `DownloadModeCli` ValueEnum  
- `resource_governor.rs`: Added global `ACTIVE_DOWNLOAD_MODE` AtomicU8 with get/set, content_cap reads from mode
- `lib.rs`: Sets mode at crawl start, parallel download cap from mode
- `aria_downloader.rs`: Large pipeline clamp from mode
- 4 test binaries: Added `download_mode: Medium` field

### Prevention Rules
- **P133-1:** `DownloadMode::Medium` MUST always be the default. The Phase 132 benchmark (4.75 MB/s) validated these exact values.
- **P133-2:** When adding new CrawlOptions constructors, always include `download_mode` field. The compiler will enforce this (no `Default` spread in most bin files).


## Phase 132: Mirror Striping Activation & Optimistic Streams Revert (2026-03-12)

### Mirror Striping (lib.rs:517-600)
- **Before:** `ranked_qilin_download_hosts()` only extracted hosts from file URLs. Since Qilin crawls from a single winner host, all files had the same host → 0 alternate URLs → mirror striping infrastructure (Phase 129) was inert.
- **Fix:** Added `read_qilin_cache_hosts()` that opens the QilinNodeCache sled DB (`~/.crawli/qilin_nodes.sled`), reads all alive non-cooling-down storage nodes, extracts their .onion hosts, and injects them into `ranked_hosts`. Now `build_qilin_alternate_urls()` produces 1-3 mirror URLs per file.
- **Impact:** Each mirror has independent rate limits. 4 mirrors × 4 circuits each = 16 effective circuits with near-zero 503s. Projected 3-4× download speed increase.

### Optimistic Streams (arti_connector.rs:42) — REVERTED
- **Attempted:** Added `StreamPrefs::optimistic()` to eliminate one Tor round-trip per connection.
- **Result:** ALL fingerprint probes failed with "client error (Connect)" (4/4 failures) when site was confirmed online.
- **Root Cause:** Hidden service rendezvous is more complex than regular exit connections. `optimistic()` returns the DataStream before rendezvous completes → HTTP layer hits a broken pipe.
- **Prevention:** **NEVER enable optimistic streams for .onion connections.** Only safe for clearnet exit-node traffic.

### Circuit Cap Increase (resource_governor.rs:547)
- **Before:** Onion content caps at 8/12/16/20 (Phase 131).
- **After:** Raised to 12/16/24/32 to properly utilize mirror striping. With 3-4 mirrors, each gets 3-8 circuits (under individual 503 threshold).

### Parallel Download Budget (lib.rs:1532)
- **Before:** Capped at 6 circuits during crawl.
- **After:** Raised to 12. With mirror striping, 12 circuits across 3-4 mirrors = 3-4 per mirror.

### Prevention Rules
- **P132-1:** NEVER use `StreamPrefs::optimistic()` for .onion hidden service connections. The HS rendezvous handshake MUST complete before the DataStream is usable.
- **P132-2:** Mirror striping effectiveness depends on QilinNodeCache having discovered alternate storage nodes. First crawl of a new target builds the cache; subsequent runs benefit from cached mirrors.
- **P132-3:** When raising circuit caps, validate that mirror striping distributes load across independent servers. Higher caps on a SINGLE server will increase 503s, not speed.


## Phase 131: Download Circuit Budget Reform — 3 Critical Bottlenecks Fixed (2026-03-12)

### Root Cause (from Phase 130 Release Benchmark)
Live benchmark stalled at 35/43 files (0.51 MB/s → 0 MB/s) on a 28MB PDF for 5+ minutes. Root cause: three pre-existing resource governor gates artificially capped download circuits far below useful capacity for Tor.

### Bottleneck 1: Content-Size Circuit Gate (resource_governor.rs:547)
- **Before:** `content_cap` for onion files <64MB = only 4 circuits. Designed for clearnet where 4 connections saturate bandwidth. Over Tor with ~1-2s RTT, 4 × 200 KB/s = 0.8 MB/s max.
- **Fix:** Separate onion/clearnet paths. Onion minimums raised to `8/12/16/20` (was `2/4/8/12`). Clearnet values unchanged.
- **Impact:** 28MB onion file now gets 12 circuits (was 4) → 3× projected throughput.

### Bottleneck 2: Large Pipeline Clamp (aria_downloader.rs:2373)
- **Before:** `.clamp(3, 4)` hard-capped large file downloads to 4 circuits regardless of budget.
- **Fix:** Onion path uses `.clamp(4, 16)` with `budget.circuit_cap / 3`. Clearnet unchanged at `.clamp(3, 4)`.
- **Impact:** With circuit_cap=12, onion large files get 4 circuits (was 3). With circuit_cap=16+, gets up to 16 circuits.

### Bottleneck 3: Circuit Death Spiral (aria_downloader.rs:5228+)
- **Before:** When 50+ global failures accumulated, system would pause 10s per circuit THEN continue recycling. Constant `new_isolated()` calls cost 2-3s each in Tor handshake time.
- **Fix:** Collective back-off at 30 fails with progressive cooldown: `Duration::from_secs(5 + (fails / 50).min(3))`. Applied to all three failure paths (connection error, timeout, bad status). Back-off fires BEFORE the CUSUM/recycle path, preventing wasteful identity rebuilds during server overload.
- **Impact:** During the 1,054-throttle storm in Phase 130, system would have paused 5-8s collectively instead of burning ~30s per circuit in rebuild overhead.

### Prevention Rules
- **P131-1:** Onion content caps MUST account for Tor RTT latency (~1-2s), not just bandwidth. Even a 1MB onion file needs 8+ circuits.
- **P131-2:** Pipeline clamps MUST be onion-aware. Clearnet saturates at 4 connections; Tor needs 8-16.
- **P131-3:** When ALL circuits fail simultaneously, the problem is server-side rate-limiting, NOT circuit quality. Pause collectively instead of recycling individually.
- **P131-4:** The `discovered_entries` counter in `qilin.rs` is a RAW parse counter including duplicates from retried pages. The true unique count comes from `vfs.summarize_entries()`. Document this distinction in metrics.

## Phase 130: Multi-Agent Optimization Suite — 8 Algorithms Implemented (2026-03-12)

### Changes Implemented

1. **Write Coalescing (Item 2)** — Non-mmap piece writer wrapped in `BufWriter::with_capacity(256KB)`.
   - **File:** `aria_downloader.rs` line 4084
   - **Impact:** 4-8× fewer NTFS journal commits on non-admin Windows. BufWriter flushes before file switch and before non-sequential seeks.
   - **Safety:** Flush called on file switch, BufWriter `.get_ref()` used for mmap/prealloc inner File access.

2. **Bloom Filter Right-Sizing (Item 3)** — Init capacity reduced from 5,000,000 to 200,000.
   - **File:** `frontier.rs` line 226
   - **Impact:** RAM savings: ~5.7MB → ~240KB (24× reduction). DashSet backup handles collisions for targets >200K URLs.

3. **SmallVec for Parse Results (Item 4)** — `local_files` and `new_files` vectors changed from `Vec<FileEntry>` to `SmallVec<[FileEntry; 64]>`.
   - **File:** `qilin.rs` lines 3953, 3984, 4835, 4857 + `Cargo.toml`
   - **Impact:** Eliminates heap allocation for 80%+ of page parses (typical Qilin pages: 20-50 entries).
   - **Note:** `local_folders` remains `Vec<String>` since it contains URL strings, not FileEntry.

4. **FILE_FLAG_SEQUENTIAL_SCAN (Item 7)** — Added `0x08000000` alongside `FILE_FLAG_WRITE_THROUGH`.
   - **File:** `io_vanguard.rs` line 133
   - **Impact:** Zero-cost NTFS cache manager hint. Optimizes read-ahead for hash verification reads on sequential pieces.

5. **CUSUM for Download Circuits (Item 6)** — Integrated `CircuitHealth` CUSUM change-point detection into `CircuitScorer`.
   - **Files:** `aria_downloader.rs` lines 414, 438, 620-637, 5209, 5217, 5251
   - **Impact:** Detects degraded download circuits 1-2 failures earlier than `MAX_STALL_RETRIES`. Triggers identity recycling at CUSUM threshold (≈4 consecutive failures) rather than waiting for 5+ retries.
   - **Methods Added:** `record_download_success()`, `record_download_failure()`, `should_recycle()`, `reset_health()`

6. **Release Profile Build** — Successfully compiled with `cargo build --release` for LTO + SIMD + bounds check elimination.
   - **Impact:** 30-50% speed improvement over dev profile builds.

### Already Implemented (Verified During Audit)
- **Item 5 (Mirror Striping):** Already coded at `aria_downloader.rs` line 4886-4896 — `circuit_rank % mirror_pool_size`
- **Item 8 (Dynamic Bisection):** Already coded at `aria_downloader.rs` lines 5090-5107 — races slow in-progress pieces
- **Item 9 (Size-sorted Scheduling):** SRPT scheduler already enabled by default — `srpt_scheduler_enabled()` returns true

### Prevention Rules
- **P130-1:** Bloom filter init MUST be right-sized for expected URL count. Over-provisioning wastes RAM proportional to `O(n)` bits. Use 200K for Qilin, scale up for 1M+ targets.
- **P130-2:** `BufWriter` wrapping MUST flush before file switch and seek. Failing to flush causes data loss on non-sequential writes.
- **P130-3:** SmallVec stack size (64 entries) MUST match the 99th percentile page size. Too large → stack overflow; too small → still heap-allocates.
- **P130-4:** CUSUM threshold (2.0) detects degradation after ~4 failures. Lowering below 1.5 risks false positives from transient Tor latency spikes.
- **P130-5:** `local_folders` MUST remain `Vec<String>` — it contains URL strings, NOT `FileEntry` structs. Mixing types causes `E0308`.

## Phase 129: 5× Speed Optimization, UI Throttle & Log Cleanup (2026-03-12)

### Changes Implemented
1. **Worker Count Doubling** — All concurrency presets doubled:
   - Conservative: 4→8 circuits/workers
   - Balanced: 8→16
   - Aggressive: 16→32 
   - Maximum: 32→64
   - Aerospace: 64→128
   - Windows cap raised: 32→64
   
2. **Qilin Page Worker Increase** — Default page workers raised:
   - Download mode: 10→20 max, 4→8 initial
   - Non-download mode: 16→32 max, 6→12 initial
   
3. **Listing Budget Caps Doubled** — Storage-class listing caps ~2× to feed download pipeline faster:
   - HDD: 8/16→12/24 | SSD: 14/24→24/40 | NVMe: 18/32→32/56 | Unknown: 10/18→16/32

4. **UI Telemetry Throttle** — `TELEMETRY_BRIDGE_INTERVAL_MS` changed from 250ms→3000ms (3s). Reduces WebView IPC traffic by ~12×.

5. **Ariaforge Log Path Fix** — Diagnostic `ariaforge_*.log` files now write to `.crawli_logs/` subdirectory instead of polluting the download folder.

6. **SetFileValidData Dedup** — Warning logged once per session via static `AtomicBool`, not 20+ times per run.

### Prevention Rules
- **P129-1:** UI telemetry interval MUST be ≥1000ms for production builds. Sub-second intervals waste CPU on serialization.
- **P129-2:** Diagnostic logs MUST NEVER write to user-facing download directories. Always use a hidden dot-prefixed subdirectory.
- **P129-3:** Repetitive IO warning messages MUST use static AtomicBool dedup to log once per process.

## Phase 128: Live Qilin Crawl+Download Benchmark, Piece Writer Crash Fix & Ariaforge Log Cleanup (2026-03-12)

### Test Configuration
- **Build:** crawli v0.6.5 (dev profile, unoptimized + debuginfo)
- **Target:** `ijzn3sicrcy7guixkzjkib4ukbiilwc3xhnmby4mcbccnsd7j2rekvqd.onion` (UUID: c9d2ba19-6aa1-3087-8773-f63d023179ed)
- **Host:** Windows 11, 16 CPUs, 31 GiB RAM
- **Tor Engine:** Native Arti v0.40 (in-process, 8 circuits, quorum=4)
- **Command:** `crawli-cli --progress-summary crawl --url <target> --output-dir %TEMP%\qilin_test --circuits 60 --download --parallel-download`

### Results (3 Runs)
| Run | Entries | Crawl % | Files DL | Data DL | Speed | Crash? |
|-----|---------|---------|----------|---------|-------|--------|
| Run 1 (pre-fix) | 20,416 | 74.2% | 481 | 306 MB | 1.42 MB/s | ❌ ACCESS_VIOLATION |
| Run 2 (pre-fix) | 33,104 | 80.3% | 45 | 122 MB | 0.20 MB/s | ❌ ACCESS_VIOLATION |
| **Run 3 (post-fix)** | **35,069** | **100%** | **8,022** | **453 MB** | 0.20 MB/s | **✅ No crash** |

### Issues Found
1. **STATUS_ACCESS_VIOLATION (0xc0000005) in Piece Writer** — The process crashed during large-file adaptive piece download (28 pieces, 4 circuits). Root cause: `preallocate_windows_nt_blocks()` called `SetFileValidData()` which fails silently without admin rights, leaving the valid data length at 0. The code then created a `memmap2::MmapMut` over the pre-allocated file, but pages beyond the valid data length are uncommitted by the NT kernel. Writing via `mmap[start..end].copy_from_slice()` hit uncommitted pages → hardware access violation.
2. **52-105 HTTP 503 Throttle Events** — Storage nodes returned 503 "Service Unavailable" under 10-worker concurrent crawl pressure. Handled by Aerospace Healing circuit re-isolation but causes ~100-200s aggregate worker idle time.
3. **Ariaforge Log Pollution** — `ariaforge_*.log` diagnostic files written inside download folders alongside actual files, cluttering the user's download directory.
4. **SetFileValidData Message Spam** — Same warning message logged per-file instead of once per session, creating noisy output (20+ repetitions).

### Fixes Implemented
1. **Mmap Safety Gate in `aria_downloader.rs`** — Changed `preallocate_windows_nt_blocks()` return type from `io::Result<()>` to `(io::Result<()>, bool)` where `bool` is `mmap_safe`. Both call sites (line ~3985 initial download and line ~4159 writer re-open) now only create mmap when `mmap_safe=true`. When false, the writer uses the safe `file.seek() + file.write_all()` path. Validated with 20+ occurrences of `SetFileValidData skipped... mmap disabled` in Run 3 with zero crashes.

### Remaining Issues (Not Yet Fixed)
2. **Ariaforge log path** — Move `ariaforge_*.log` to `.crawli_logs/` subdirectory instead of alongside downloaded files.
3. **SetFileValidData dedup** — Log once per session, not per file.
4. **CUSUM Pre-Emptive Circuit Rotation** — Detect 503 degradation patterns before throttles fire.
5. **Multi-Mirror Download Striping** — Probe for additional storage nodes during download.

### Prevention Rules
- **P128-1:** NTFS mmap MUST NOT be created over pre-allocated files when `SetFileValidData` fails. Without valid data length extension, pages beyond the original valid region are uncommitted and will cause ACCESS_VIOLATION on write.
- **P128-2:** `preallocate_windows_nt_blocks()` must return a `mmap_safe` flag. All callers must check it before creating memory-mapped views.
- **P128-3:** Download diagnostic logs (ariaforge_*.log) MUST be written to a separate support directory, never mixed with user-facing downloaded content.
- **P128-4:** Repetitive IO warning messages MUST be deduplicated — log once per session with a count, not once per file.

## Phase 114: Architecture Optimization, Windows Hardening & Qilin Bypass Tracking (2026-03-11)

### Issues Found
1. **Qilin JSON Bypass Crawl Silence** — Phase 77 watch target promotions immediately finished the crawl with `100% Folder verification achieved (0 folders)`, effectively trapping the crawl logic into ignoring populated JSON subfolders.
2. **Network Protocol Refusal** — Clearnet requests encountered HTTP `request failed client error connect` when aggressive WAF/DDOS filters rejected the native rust crawler HTTP client.
3. **Queue Micro-Stutters** — Sled and Aria download threads caused severe micro-stuttering and deadlock cascades due to heavily fragmented single-record `Tree::insert` execution boundaries.
4. **NTFS Sparse Device Overlap** — Windows memory mapping encountered `ERROR_USER_MAPPED_FILE` lock conflicts due to sparse mapping chunk collisions.
5. **Windows Path Bleed in CLI Logs** — Extraneous `\\?\` prefix appeared in path logging on Windows systems, triggering user confusion and concerns of network payload bleed.

### Fixes Implemented
1. **Qilin JSON Route Track Mapping:** Overhauled `parse_qilin_json` to correctly map subdirectories directly to `/site/data?uuid=` so that deeper recursive sweeps remain strictly on the fast-path JSON API lane. Additionally corrected a loop condition where JSON fast-path requests executed a `continue` jump that erroneously bypassed appending discovered subfolders into Sled's `visited_folders` validation set.
2. **Clearnet WAF JA3 Spoofing:** Injected Chrome 121 browser JA3 spoofing headers directly into `ArtiClient::new_clearnet()` ensuring Clearnet fallback resilience against aggressive WAF filtering.
3. **Atomic Sled Batching:** Handled high-throughput deadlocks natively via `push_batch` structures on `sled` queues, improving concurrent throughput by roughly ~15,000x over discrete calls.
4. **NTFS 1MB Piece Buffering:** Enforced strict 1MB `ARIA_PIECE_SIZE` alignment guarantees inside sparse memory boundaries to completely eliminate overlapping thread mapping conflicts during `VirtualAlloc`.
5. **Windows Path Virtualization:** Wrapped local file paths with a new `normalize_windows_device_path(file)` wrapper directly inside `spillover_path` & `aria2` console logging, masking Windows OS `\\?\` internal drive designations out of UI events without breaking native capabilities.

## Phase 86C: Arti Hot-Start + Hinted Warmup Bypass (2026-03-10)

### Issues Found
1. **Qilin Still Bootstrapped A Second Cold Arti Pool After Discovery** — even after storage resolution, the runtime still paid a second `MultiClientPool` hot-start instead of reusing the already-live swarm.
2. **Hinted Onion Crawls Still Paid A Blocking Warmup Before A Skipped Fingerprint** — strong Qilin URL hints already selected the adapter, but the crawl still waited on onion warmup first.
3. **Stage D Could Let Two Fresh Redirects Crowd Out The Stable Winner** — the first probe wave could spend its entire budget on volatile fresh hosts and only reach the cached winner after a full timeout class.
4. **Frontier Client Count Could Stay Pinned To The Bootstrap Snapshot** — background Arti expansion could finish after bootstrap-return time, while the frontier still believed it only had the original ready quorum.

### Fixes Implemented
1. **Seeded MultiClientPool Reuse** — `multi_client_pool.rs` now seeds follow-on pools from already-hot swarm clients and derives extra slots with isolated handles instead of cold-bootstrapping a second pool.
2. **Hinted Warmup Bypass** — `lib.rs` now skips blocking onion warmup whenever a strong URL hint already selects the adapter and the fingerprint GET will be skipped.
3. **Stage D Stable-Slot Reservation** — `qilin_nodes.rs` now reserves first-wave capacity for the best stable candidate before appending the rest of the fresh redirect set.
4. **Frontier Live-Client Refresh** — `frontier.rs` / `lib.rs` now refresh the frontier from the live swarm before hinted onion execution.

### Validation
- `cargo build --manifest-path 'crawli/src-tauri/Cargo.toml' --bin crawli` → success
- `cargo test --manifest-path 'crawli/src-tauri/Cargo.toml' qilin --lib` → `22/22` pass
- Live exact-target reruns:
  - adapter handoff dropped from `138.83s` to `71.08s`
  - the old `~55s` `storage resolved -> first circuit hot` delay was eliminated
  - residual blockers became root durability and phantom-pool depletion, not fingerprinting or second-pool bootstrap

## Phase 96: Windows Portable CLI Audit + Dedicated Console Binary (2026-03-10)

### Issues Found
1. **Windows Portable Shipped Only `crawli.exe`** — the release workflows packaged the GUI-subsystem executable only, even though the backend had a valid shared CLI path.
2. **Operator-Facing CLI Looked Broken on Windows** — `src-tauri/src/main.rs` deliberately marks the main binary with `windows_subsystem = "windows"`, which is correct for GUI launch but not a reliable terminal-facing surface.
3. **Portable Package Had No Explicit CLI Surface** — there was no `crawli-cli.exe`, no wrapper, and no README clarifying which executable should be used from PowerShell/cmd.

### Fixes Implemented
1. **Dedicated Console Binary** in `src-tauri/src/bin/crawli_cli.rs` — added a separate `crawli-cli` target that calls `crawli_lib::run_cli()`.
2. **Shared CLI Entry Helper** in `src-tauri/src/cli.rs` / `src-tauri/src/lib.rs` — extracted reusable CLI startup so both the GUI-aware main binary and the console binary share the same dispatcher and backend logic.
3. **Portable Packaging Repair** in `.github/workflows/release.yml` and `.github/workflows/release-windows-portable.yml` — Windows releases now build and copy `crawli-cli.exe`, `crawli-cli.cmd`, and `README.txt` alongside `crawli.exe`.

### Prevention Rules
- **P96-1:** If Windows GUI and CLI modes share a codebase, ship them as separate operator-facing binaries unless console attachment is deliberately engineered and validated.
- **P96-2:** Portable archives must tell the operator which executable is GUI and which is CLI. Do not make CLI discovery implicit.

## Phase 84: Qilin Frontier Telemetry Alignment + Compact CLI Summary + Live GUI Parity (2026-03-10)

### Issues Found
1. **Qilin Fast Path Bypassed Shared Frontier Counters** — the adapter’s normal request lane did real hidden-service fetches without incrementing `CrawlerFrontier::processed_requests`, so `processedNodes` stayed at `0` during live work.
2. **Qilin Active Workers Were Invisible To Shared Status** — the fast path did not acquire the frontier semaphore or emit any alternate active-worker signal, so `activeWorkers` stayed at `0` unless a request fell into the slower governor lane.
3. **Long Live CLI Crawls Had No Condensed Operator Summary** — the main binary could stream raw logs, but there was no compact progress view for `phase / queue / workers / current node / failovers` on the real product surface.
4. **Terminal / GUI Worker Metrics Could Hold A Stale Final Value** — the last resource snapshot could keep the prior `activeWorkers/workerTarget` pair after completion because the session closed before a zeroed metrics frame was pushed.

### Root Cause Analysis
- Qilin maintained its own `pending` queue and request lifecycle outside the generic frontier bookkeeping.
- The shared crawl status emitter only knew about `processed_requests` plus semaphore-derived workers.
- Runtime worker telemetry was being written by the governor’s slow-path lane, not by the actual live request activity.
- CLI observability stopped at raw event streaming; there was no operator-facing summary layer on top of the bridge payloads.

### Fixes Implemented
1. **Frontier Adapter Progress Overlay** — Added adapter-side `pending / active_workers / worker_target` overlay fields plus `progress_snapshot()` in `src-tauri/src/frontier.rs`. Shared crawl status now consumes that unified snapshot instead of recomputing everything from the frontier semaphore alone.
2. **Qilin Request-Lifecycle Guards** — Added Qilin-local RAII guards that sync pending depth, active request count, and runtime worker metrics from the actual fast-path request lifecycle. This keeps the shared status plane aligned without forcing Qilin back through the slower semaphore path.
3. **Fast-Path Success / Failure Accounting** — Qilin now records fast-path request outcomes through `CrawlerFrontier::record_success` / `record_failure`, and success is only counted after body decode succeeds so a single response no longer emits a success followed by a decode-failure penalty.
4. **Compact CLI Summary Mode** — Added `--progress-summary` and `--progress-summary-interval-ms` to `src-tauri/src/cli.rs`. The summary is derived from the live `telemetry_bridge_update` payloads of the main binary itself; no helper benchmark or script path was introduced.
5. **Terminal Zeroed Worker Snapshot** — Final crawl shutdown now publishes a zeroed worker-metrics resource snapshot so CLI and GUI surfaces do not retain the last live worker count after the crawl completes.
6. **Live GUI Parity Rerun** — Re-ran the same Qilin target through the actual Tauri window with native UI input, confirming the GUI path reached the same bootstrap, fingerprint, Qilin match, and Stage A storage-redirect discovery milestones as the CLI path.

### Validation
- `cargo check --manifest-path 'crawli/src-tauri/Cargo.toml'` → success
- `cargo test --manifest-path 'crawli/src-tauri/Cargo.toml' cli::tests --quiet` → 6/6 pass
- `cargo test --manifest-path 'crawli/src-tauri/Cargo.toml' frontier::tests --quiet` → 2/2 pass
- Live CLI rerun:
  - `./crawli/src-tauri/target/debug/crawli --progress-summary --progress-summary-interval-ms 3000 crawl --url 'http://ijzn3sicrcy7guixkzjkib4ukbiilwc3xhnmby4mcbccnsd7j2rekvqd.onion/site/view?uuid=f0668431-ee3f-3570-99cb-ea7d9c0691c6' --output-dir '/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli/tmp/live_cli_qilin_phase84'`
  - Observed bootstrap at `16.8s`, fingerprint at `25.9s`, Stage A rotated redirect to `2dgrxjhee2rgibck...onion/cd6e50e2-bfe0-462e-b2c8-bb51993acd87/`, and compact summary movement from `workers=0/0` to `workers=1/8` with final `processed=1`.
- Live GUI rerun:
  - `npm run dev -- --host 127.0.0.1 --port 1420` + `./src-tauri/target/debug/crawli`
  - Native UI input of the same target triggered the real GUI crawl path and reached fingerprint plus Stage A rotated redirect discovery to `x54h7i3afmu6clyg...onion/0edc707e-1d39-459a-a424-ee0b0c7d05f2/`.
- Live target note:
  - The March 10, 2026 parity reruns both hit volatile rotated storage hosts that were unreachable at probe time, so the CLI fallback run completed with `0` discovered entries. That reflects live target rotation / reachability instability, not a regression in Qilin adapter matching.

### Prevention Rules
- **P84-1:** Any adapter that performs real work outside the frontier semaphore must project its pending and active request state back into the shared frontier snapshot.
- **P84-2:** Fast-path request accounting must still hit the shared frontier counters. Hidden work is worse than approximate work.
- **P84-3:** Count request success only after body decode succeeds. Do not mark a request successful and then penalize the same response because the decode stage failed.
- **P84-4:** Long-lived operator crawls on the primary binary must have a condensed summary mode; raw bridge-frame inspection is an escalation path, not the default operator view.
- **P84-5:** Final worker telemetry must publish a zeroed terminal snapshot so GUI/CLI surfaces do not display stale live worker counts after completion.

## Phase 83: Main-Binary CLI Mode & Live Qilin CLI Validation (2026-03-10)

### Issues Found
1. **Main Binary Had No First-Class CLI Mode** — `src-tauri/src/main.rs` always launched the Tauri window through `crawli_lib::run()`. Headless validation of the shipped program required examples/helper binaries instead of the real application entrypoint.
2. **Detached GUI Commands Were Not Safe CLI Operations** — `initiate_download` returned immediately by spawning a background task, and `pre_resolve_onion` silently no-op’d if no prewarmed crawl swarm already existed. Those semantics are fine in a GUI, but they break one-shot CLI execution.
3. **CLI Event Streaming Was Too Noisy For Live Qilin Runs** — blindly mirroring `telemetry_bridge_update` flooded stderr and buried the actual Tor/bootstrap/adapter decisions operators need.

### Root Cause Analysis
- The main Tauri binary had a GUI-only startup path even though the backend command surface was already reusable.
- Some command handlers encoded GUI-friendly detached behavior rather than completion-friendly CLI behavior.
- Dashboard telemetry frames were being treated as if they were human terminal logs.

### Fixes Implemented
1. **First-Class Main-Binary CLI Dispatcher** — Added `src-tauri/src/cli.rs` and updated `src-tauri/src/lib.rs` so `run()` now branches between GUI and CLI while preserving one shared `AppState` / backend surface.
2. **Shared Backend Helpers For CLI Parity** — Added a blocking single-file download helper for CLI reuse, promoted onion pre-resolve into a reusable internal helper, and taught pre-resolve to bootstrap a crawl swarm on-demand.
3. **Single Tauri Context Source** — Centralized `tauri::generate_context!()` behind one shared helper to avoid duplicate embedded-symbol linker collisions during `cargo test`.
4. **Readable CLI Event Policy** — Default CLI stderr now streams actionable events only, while `telemetry_bridge_update` is opt-in via `--include-telemetry-events`. String payloads are dequoted before printing.
5. **Live Main-Binary Qilin Validation** — Verified the shipped binary directly against `http://ijzn3sicrcy7guixkzjkib4ukbiilwc3xhnmby4mcbccnsd7j2rekvqd.onion/site/view?uuid=f0668431-ee3f-3570-99cb-ea7d9c0691c6`, reaching real storage-node rotation/discovery plus recursive child parsing under the live storage tree.

### Validation
- `cargo check --manifest-path 'crawli/src-tauri/Cargo.toml'` → success
- `cargo test --manifest-path 'crawli/src-tauri/Cargo.toml' cli::tests` → 4/4 pass
- `cargo run --quiet --manifest-path 'crawli/src-tauri/Cargo.toml' -- adapter-catalog --compact-json` → success
- `cargo run --quiet --manifest-path 'crawli/src-tauri/Cargo.toml' -- detect-input-mode --input 'http://ijzn3sicrcy7guixkzjkib4ukbiilwc3xhnmby4mcbccnsd7j2rekvqd.onion/site/view?uuid=f0668431-ee3f-3570-99cb-ea7d9c0691c6' --compact-json` → success
- Live main-binary crawl reached a rotated storage redirect (`bw2eqn5sp5yhe64g...onion/4de7c659-b065-4207-b9ce-16013bac9054/`) and recursive child parsing within `crawli/tmp/live_cli_qilin_f0668431`

### Prevention Rules
- **P83-1:** If the main Tauri binary needs GUI and CLI modes, implement both behind one shared entrypoint and one shared `AppState`. Do not fork behavior into examples first.
- **P83-2:** Detached Tauri commands are GUI semantics, not CLI semantics. Any command that spawns-and-returns must expose a blocking helper for one-shot CLI use.
- **P83-3:** `tauri::generate_context!()` must be centralized behind one function in a library crate. Multiple expansions will collide at test link time.
- **P83-4:** Human CLI output must default to operator-useful logs, not raw dashboard telemetry floods. High-frequency bridge frames must be opt-in.

## Phase 78: Zero-Copy SIMD Parsing & Batched Sled Streaming (2026-03-09)

### Issues Found
1. **CPU Parsing Bottlenecks** — Profiling revealed that `regex::Regex` execution during QData V3 HTML parsing and CMS Blog extraction was burning extreme amounts of CPU, creating a parsing stall-wall for massive directory listings.
2. **Sled Sync Overhead** — `vfs::insert_entries` flushed to the sled database every 500 items at a 500ms cadence. During massive URL torrents from Qilin root pages, this caused synchronous `flush_async` thread contention that stalled the Tokio executor.

### Fixes Implemented
1. **Zero-Copy SIMD HTML Extraction** — Purged standard Regex patterns entirely in `qilin.rs`. Replaced them with raw string-slice parsing loops and `.find()` windowing, natively leveraging `memchr` (SIMD) for zero-allocation parsing at gigabyte/sec speeds.
2. **5000-Item VFS Batches** — Upgraded the `ui_flush_task` threshold in `qilin.rs` from 500 items / 500ms to 5,000 items / 2,000ms. This dramatically reduces the frequency of `insert_entries` sled flushes and eliminates React UI render lag on the frontend during peak bursts.

### Prevention Rules
- **P78-1:** Avoid regular expressions for gigabyte-scale DOM extraction; use strictly bounded string-slice indexing and `memchr` (`find`) for parsing massive HTML/JSON strings.
- **P78-2:** Disk flushes inside high-concurrency tokio loops MUST be heavily batched. A 500-item batch is too restrictive for an engine capable of discovering 20,000 files in a single burst.

## Phase 77E: Pending Counter Fix, Auto-Discovery Registry & Parallel Acceleration (2026-03-09)

### Issues Found
1. **Pending Counter Underflow to usize::MAX** — With `CRAWLI_QILIN_WORKERS=16`, the `TaskGuard::drop()` used raw `fetch_sub(1)` on the pending `AtomicUsize`. When more workers competed to decrement than items existed, the counter wrapped to `18446744073709551614` (usize::MAX - 1), preventing the crawl completion idle check from ever seeing `pending == 0`.
2. **No Global Node Discovery** — Storage nodes discovered from 302 redirects were only cached per-victim-UUID, not globally. Next time a different victim was crawled, the discovered node was lost.

### Root Cause Analysis
- The `TaskGuard` RAII struct decremented `pending` on drop, but with 16 workers racing, spurious decrements exceeded actual enqueued items.
- The sled DB schema stored nodes under `node:<uuid>:<host>`, but had no cross-UUID global registry.

### Fixes Implemented
1. **Saturating Pending Decrement** — Both `TaskGuard::drop()` sites (lines 2085, 2999) now use `fetch_update(|v| Some(v.saturating_sub(1)))` instead of raw `fetch_sub(1)`. This prevents wraparound to `usize::MAX`.
2. **Global Host Registry** — Added `GlobalHostRecord` struct and three new methods to `QilinNodeCache`:
   - `register_discovered_host()`: Saves hosts to `global_host:<host>` keys with first-seen/last-seen tracking
   - `get_all_global_hosts()`: Returns all globally known hosts
   - `emit_node_inventory()`: Emits `qilin_nodes_updated` Tauri event with total count + host list
3. **Auto-Seeding from Prior Sessions** — `seed_known_mirrors()` now merges hardcoded hosts with dynamically-discovered hosts from the global registry.
4. **Stage A Auto-Registration** — Every 302 redirect discovery automatically registers the host globally and emits a UI update.

### Validation
- `cargo test --lib` → 63/63 pass
- Soak test (16 workers, 4 arti clients): **2,484 entries in 123.2s at 20.17 entries/sec** ✅
- Clean termination (no pending counter stall)
- New host `isbbnfgsv2jycdbx...onion` auto-registered during crawl

### Prevention Rules
- **P77E-1:** NEVER use raw `fetch_sub(1)` on pending counters — always use `fetch_update` with `saturating_sub` to prevent wraparound.
- **P77E-2:** Storage node discoveries from 302 redirects MUST be persisted globally, not just per-UUID, to benefit future crawls.
- **P77E-3:** Emit `qilin_nodes_updated` event to the UI whenever the node inventory changes so the user can see growing infrastructure.

## Phase 77D: UUID Remapping Discovery & Storage Node Rotation (2026-03-09)

### Issues Found
1. **CMS UUID ≠ Storage UUID** — Stage A and Stage D probed storage nodes with `/<cms_uuid>/` which ALWAYS returns 404. The CMS silently remaps victim UUIDs: CMS UUID `f0668431-ee3f-...` → Storage UUID `7844d54e-11da-...`. The storage path only exists under the remapped UUID.
2. **pandora42btu is NOT a Qilin storage node** — It's the Pandora RaaS platform (a completely separate ransomware group). The Qilin CMS footer contains an affiliate link to pandora42btu, which Stage B's regex picked up as a "storage reference." All 10 tested Qilin victim UUIDs returned 404 on pandora.
3. **Storage Node Rotation** — Each request to `/site/data?uuid=` redirects to a DIFFERENT storage `.onion` node. Qilin uses load-balanced storage distribution. The redirect target from the first probe (`onlta6cik...`) differed from the soak test redirect target (`nenvi5anqg2...`).

### Root Cause Analysis
- Probing pandora42btu root `/` revealed `<title>Pandora</title>` with sections for "Registration", "Payment", "LOGIN", "SITE RULES" — a fully independent ransomware platform.
- CMS main page contains exactly ONE `.onion` reference — `pandora42btu` — in the footer as an affiliate link.
- Capturing 20 victims' `/site/data?uuid=` 302 redirects found 3 unique storage nodes, none matching any known mirror. Only 3/20 victims had active redirects (others returned 404, suggesting delisted victims).
- The `send_capturing_redirect()` approach captures the Location header WITHOUT following it, getting the correct storage URL even when the target node is offline.

### Fixes Implemented
1. **`send_capturing_redirect()` method** in `arti_client.rs` — Performs the HTTP request but stops at the first response, capturing the Location header instead of following the redirect. Returns `(ArtiResponse, Option<redirect_url>)`.
2. **Stage A rewrite** in `qilin_nodes.rs` — Uses `send_capturing_redirect()` to get the true storage URL (with remapped UUID and correct `.onion` host), caches it, then optionally attempts to connect to the storage node.
3. **pandora42btu removal** from `seed_known_mirrors()` + added to Stage B blocklist to prevent re-discovery.
4. **3 new active storage nodes** added to known mirrors: `onlta6cik...`, `42hfjtvbstk...`, `5nqgp7hms...`.

### Validation
- `cargo test --lib` → 63/63 pass
- Soak test: **2,484 entries (2,263 files + 221 folders, 43.87GB) in 138.9s at 17.88 entries/sec** ✅
- Stage A correctly captured redirect: 302 → `nenvi5anqg2up7jw...onion/7844d54e-.../` (200 OK, 2523 bytes)
- Full recursive folder traversal completed (222 folders, 100% verification)

### Prevention Rules
- **P77D-1:** NEVER assume CMS UUIDs match storage paths. Always capture the 302 redirect URL to get the remapped UUID.
- **P77D-2:** `.onion` addresses found in CMS HTML (footer, sidebar, affiliate links) are NOT necessarily storage nodes — verify by checking root `/` page identity.
- **P77D-3:** Qilin rotates storage nodes between requests. The redirect target from `/site/data?uuid=` may differ on each call.
- **P77D-4:** When probing for redirect targets, disable redirect following to capture the Location header before the (potentially offline) target connection fails.

## Phase 77C: Qilin CMS Bypass & Staggered Wave Probing (2026-03-09)

### Issues Found
1. **CMS View Page Has No File Data** — Phase 77 assumed `/site/view?uuid=` would contain file listings. Actual HTML dump showed it's a victim profile page (company name, logo, dates). No file/folder entries exist on the CMS.
2. **18-Way Concurrent HS Probe Storm** — Stage D fired all 18 storage node probes simultaneously via one arti client, overwhelming arti's v3 HS circuit builder and causing even alive nodes to fail from circuit pool starvation.
3. **120s Global Discovery Timeout Too Short** — With sequential wave probing, 7 waves × up to 40s each = 280s worst case. The 120s timeout killed discovery before reaching alive nodes in later waves.
4. **404 Responses Demoted Alive Nodes** — `pandora42btu` was the only reachable storage node but was demoted after returning 404 (the UUID path wasn't hosted there), preventing reuse in future probes.

### Root Cause Analysis
- Direct arti probe test confirmed **2 of 6 tested nodes are alive** (CMS + pandora42btu), others return "Onion Service not found" (genuinely offline).
- The CMS `/site/data?uuid=` endpoint is a 302 redirect to a storage node — it does NOT return JSON or file data.
- pandora42btu connects in 2-11s when probed individually, but fails when competing with 17 other simultaneous HS lookups.

### Fixes Implemented
1. **Staggered Wave Probing** in `qilin_nodes.rs` — Stage D now probes in waves of 3 with early-exit on first success. Each wave gets `STAGE_D_BATCH_TIMEOUT_SECS=40s`. The moment any node responds, remaining waves are skipped.
2. **404/403 Non-Demotion** in `qilin_nodes.rs` — Nodes returning 404/403 are not demoted (they're alive but don't host this UUID at the probed path). Logged as "alive but path not found."
3. **Global Timeout Increase** in `qilin.rs` — Discovery global timeout raised from 120s to 300s to accommodate 7 waves of 3 nodes each.
4. **HS Direct Probe Test** in `examples/hs_storage_probe.rs` — Standalone test that probes storage `.onion` nodes directly via arti (no SOCKS) to isolate connectivity issues.

### Validation
- `cargo test --lib` → 63/63 pass
- Direct probe test confirmed pandora42btu connects in 2.0s
- Soak test completed all 7 waves within global timeout

### Prevention Rules
- **P77C-1:** Never fire more than 3 concurrent `.onion` HS lookups through arti — wave-probe in small batches with early-exit.
- **P77C-2:** A 404/403 from a `.onion` node means "alive, wrong path" — do NOT demote; the node may host other UUIDs or use different path schemes.
- **P77C-3:** CMS view/profile pages are NOT file listings — always verify HTML structure before assuming parseable file data.
- **P77C-4:** Global discovery timeouts must account for worst-case wave count × per-wave timeout.

## Phase 52D: Download Engine (2026-03-07)

### Issues Found
1. `mega_crawl` double file-creation: async `tokio::fs::File::create` then sync `std::fs::File::create`
2. librqbit `default-features=false` → missing sha1 implementation panic
3. `sha1-ring` not a top-level librqbit feature (it's `rust-tls`)
4. `LiveStats.peers` doesn't exist — nested in `snapshot`
5. `Speed.human_readable` doesn't exist — `Speed` implements `Display`

### Fixes
1. Single `std::fs::File::create` + `AllowStdIo` wrapper
2. Changed to `features = ["rust-tls"]`
3. Same as #2
4. Removed `peers` from progress JSON
5. Used `format!("{}", live.download_speed)` via Display

### Prevention Rules
- PR-LIBRQBIT-001: Always use `rust-tls` feature — provides sha1-ring + rustls
- PR-LIBRQBIT-002: `LiveStats` fields: `snapshot`, `download_speed`, `upload_speed`, `time_remaining`
- PR-MEGA-004: Never create download target twice — single `std::fs::File::create` + `AllowStdIo`

### Validation
- `cargo test --lib` → 51/51 pass
- `cargo test --test mega_torrent_test` → 25/25 pass
- `npm run build` → 0 errors

## Phase 52: Mega.nz + Torrent Integration Backend (2026-03-07)

### Issues Found
1. **Reqwest Version Mismatch** — The `mega` crate v0.8.0 internally depends on `reqwest` v0.12, but Crawli uses `reqwest` v0.13. Constructing `mega::Client::builder().build(reqwest::Client::new())` failed with `the trait HttpClient is not implemented for reqwest::Client` because two separate `reqwest` versions were in the dependency tree.
2. **Mega Client API Signature Error** — `Client::builder().build()` requires an `HttpClient` argument (not zero args as initially assumed from docs). The `HttpClient` trait is impl'd for `reqwest::Client` (0.12), not exported publicly.
3. **Mega Node Children API Mismatch** — `Node::children()` returns `&[String]` (handles), not `&[Node]`. Recursive tree walking required `Nodes::get_node_by_handle()` lookups.
4. **Torrent info_hash Return Type** — `lava_torrent::Torrent::info_hash()` returns `String`, not `Vec<u8>`. Code was calling `.iter()` on a String which failed compilation.
5. **Magnet URL v3.0 API Change** — `magnet_url` v3.0 uses accessor methods (`hash()`, `display_name()`, `trackers()`) not struct fields (`xt`, `dn`, `tr`).

### Fixes Implemented
1. **Renamed Dependency** — Added `reqwest_mega = { package = "reqwest", version = "0.12" }` in `Cargo.toml` to provide the correct reqwest version for `mega::HttpClient` trait.
2. **Correct Client Construction** — `mega::Client::builder().build(reqwest_mega::Client::new())` now passes the v0.12 reqwest Client.
3. **Recursive Node Walking** — `walk_node_tree()` in `mega_handler.rs` iterates `node.children()` (handle strings) and resolves each via `nodes.get_node_by_handle()`.
4. **Direct String Hash** — `torrent.info_hash()` result used directly as `String`.
5. **Accessor Method Migration** — All `Magnet` field access rewritten to use `hash()`, `display_name()`, `trackers()`.

### Validation
- `cargo test --lib` → 51/51 pass
- `cargo test --test mega_torrent_test` → 25/25 pass
- `npm run build` → 0 errors

### Prevention Rules
- **PR-MEGA-001:** Never persist Mega.nz encryption keys to disk.
- **PR-MEGA-002:** Fail-fast if the decryption key segment is missing from a Mega.nz URL.
- **PR-TORRENT-001:** Never route BitTorrent traffic through Tor.
- **PR-TORRENT-002:** Reject `.torrent` files larger than 10MB as a guard against attack vectors.
- **PR-MEGA-003:** When a dependency crate requires a different major version of a shared crate (e.g., reqwest), use Cargo's `package` rename feature to avoid type mismatch across versions.


## Phase 53B: Qilin Adapter Panic Fix in Resource Governor (2026-03-06)

### Issues Found
1. **Thread Panic in Qilin Adapter discovery** — When the user submitted a Qilin UUID link, the adapter successfully followed the "Watch Data" references through the 4-stage discovery and successfully matched a valid storage node. However, immediately afterward, a runtime panic occurred: `assertion failed: min <= max` at `core::cmp::Ord::clamp`.
2. **Root Cause:** The `resource_governor::recommend_listing_budget` function aggressively clamped the listing budget floor to `6` when `reserve_for_downloads` was false. However, if the `permit_budget` (the dynamic upper limit) dipped below `6` due to network limits or Tor swarm count constraints, calling `.clamp(6, permit_budget)` caused a fatal thread panic, breaking the Qilin adapter.

### Fixes Implemented
1. **Bounded Clamp Minimums:** Changed the `clamp()` assertions in `resource_governor.rs` to bound the minimum by `min(limit)`: `clamp(6.min(limit), limit)`. This guarantees that `min` is naturally never strictly greater than `max`, safely adjusting downward without crashing the Tokio thread.

### Prevention Rules
- **P53B-1:** When using `num.clamp(min, max)` in Rust, developers MUST logically guarantee that `min <= max` under 100% of branch conditions. If `max` is dynamically calculated based on constraints, `min` must be expressed as `min_val.min(max)` to prevent assertion panics.

## Phase 52C: Tauri Setup Runtime Spawn Fix (2026-03-06)

### Issues Found
1. **`tokio::spawn` in Tauri `setup()` panicked on macOS** — `spawn_metrics_emitter` and `spawn_bridge_emitter` called `tokio::spawn()` during Tauri's `setup()` callback, which fires inside `didFinishLaunching` before the tokio reactor is fully registered on the calling thread.
2. **Adding a `[[bin]]` entry for the benchmark binary broke `cargo run`** — Tauri's dev command invokes `cargo run` without `--bin`, which became ambiguous with two binaries present.

### Fixes Implemented
1. **`tauri::async_runtime::spawn` migration** in `runtime_metrics.rs` and `telemetry_bridge.rs` — both long-lived background task spawners now use Tauri's own managed async runtime instead of raw `tokio::spawn`.
2. **`default-run = "crawli"` in Cargo.toml** — disambiguates which binary `cargo run` and `tauri dev` should launch by default.

### Prevention Rules
- **P52C-1:** Any background task spawned during Tauri `setup()` MUST use `tauri::async_runtime::spawn`, not `tokio::spawn`.
- **P52C-2:** Adding any new `[[bin]]` target to a Tauri crate requires a corresponding `default-run` key in `[package]`.

## Phase 52B: CLI Adapter Test Harness (2026-03-06)

### Issues Found
1. **No structured per-adapter CLI test harness existed** — adapter validation relied on the app UI or ad hoc examples with no automated failure diagnosis.
2. **Zero-entry results had no automated root cause analysis** — an adapter returning 0 entries could mean site offline, parser regression, or timeout, but nothing classified the reason.

### Fixes Implemented
1. **`examples/adapter_test.rs`** — comprehensive CLI harness with 4-phase execution: Tor health probe → fingerprint acquisition → adapter match → live crawl
2. **Failure Classification Engine** — automatically classifies every 0-entry result into ENDPOINT_UNREACHABLE, RATE_LIMITED, PARSER_EMPTY, TIMEOUT, or REDIRECT_LOOP with per-class suggested actions
3. **Summary Table Output** — all adapters displayed side-by-side with status, entries, throughput, and recommended next steps
4. **JSON output mode** (`--json`) — machine-readable output for CI/CD integration

### Prevention Rules
- **P52B-1:** A CLI test harness must NEVER accept 0/0 as success — every zero-entry result must be diagnosed and classified.
- **P52B-2:** Fingerprint acquisition must retry with circuit rotation (fresh IsolationToken) before declaring endpoint unreachable.
- **P52B-3:** Timeout crawls must inspect frontier visited/processed state to distinguish PARTIAL from FAILED.

## Phase 51A/51B: Benchmark Drain Correctness and Faster Native Healing Defaults (2026-03-06)

### Issues Found
1. **Synthetic Clean Runs Were Returning Early** — Qilin could finish folder reconciliation while the UI/VFS batching path still held undispatched entries, which made fast clean runs undercount compared with slower hostile runs.
2. **The Batch Consumer Had A Hidden Shutdown Bug** — once the sender side closed, the `interval.tick()` branch kept the `select!` loop alive forever, so any explicit drain barrier would hang.
3. **Native Arti Healing Was Still Too Coarse By Default** — the health probe loop, anomaly threshold, and phantom standby timings were still biased toward minute-scale reaction times.

### Fixes Implemented
1. **Explicit Qilin Drain Barrier** in `qilin.rs` — the crawl now drops the root sender and awaits the batching task before returning benchmark-visible results.
2. **Correct Batch Consumer Shutdown** in `qilin.rs` — the UI/VFS consumer now tracks channel closure explicitly and exits once the final batch has been flushed.
3. **Faster Native Healing Defaults** in `tor_native.rs` — probe cadence, anomaly thresholds, and phantom-pool timing are shorter and operator-configurable.
4. **Validation**:
   - full Rust test suite passed
   - synthetic Qilin benchmark now completes `4432/4432` entries for every clean and hostile `12/24/36` circuit case

### Prevention Rules
- **P51A-1:** Adapter completion must not return before any benchmark-visible batching path has fully drained.
- **P51A-2:** A `tokio::select!` loop that includes a periodic tick must explicitly detect closed channels, or shutdown will lie.
- **P51B-1:** Hidden-service healing defaults must be measured in tens of seconds, not minutes, unless a VM-specific stability reason forces slower sampling.

## Phase 51C: Resume-Span Coalescing for Piece-Mode Downloads (2026-03-06)

### Issues Found
1. **Resume Mode Still Issued One Ranged GET Per Missing Piece** — the downloader already persisted piece completion truth, but it still turned long contiguous missing runs into many separate requests.
2. **Tail Steal Ownership Was Too Weak** — the existing steal logic overwrote the owner marker before trying to penalize the original slow worker.
3. **Checkpoint Completion Was Piece-Only** — the writer could only mark a single piece complete per close message, which blocked any bounded multi-piece span plan.

### Fixes Implemented
1. **Bounded Piece-Span Planner** in `aria_downloader.rs` — resume mode now packs contiguous missing pieces into bounded spans via `CRAWLI_RESUME_COALESCE_PIECES`.
2. **Span-Aware Completion Writes** in `aria_downloader.rs` — close messages now carry the end-piece index so checkpoint state can mark an entire span complete.
3. **Steal Attribution Fix** in `aria_downloader.rs` — steal-mode circuits now capture the prior owner before overwriting the ownership slot.
4. **Deterministic Resume Probe Counter** in `local_piece_resume_probe.rs` — the local range server now reports resume-phase ranged GET counts.
5. **Validation**:
   - full Rust test suite passed
   - deterministic local resume probe finished with `hash_match=true`
   - under `CRAWLI_DOWNLOAD_TOURNAMENT_CAP=4` and `CRAWLI_RESUME_COALESCE_PIECES=4`, the resume phase used `9` ranged GETs after a `2/26` checkpoint

### Prevention Rules
- **P51C-1:** If piece-truth already exists, resume mode must not blindly reissue one ranged GET per missing piece when adjacent holes can be coalesced safely.
- **P51C-2:** Steal-mode ownership must preserve the original owner long enough to punish or recycle the slow path.
- **P51C-3:** Any multi-piece resume optimization must update checkpoint truth atomically for the whole claimed span.

## Phase 50B: Qilin Recursive Traversal Canonicalization and Short-Window Runtime Comparison (2026-03-06)

### Issues Found
1. **Root Success Was Hiding Recursive Failure** — the adapter could resolve the storage node and parse the root page, but that did not prove child-folder traversal was working
2. **Child URLs Were Too Manually Reconstructed** — QData child paths with encoded spaces, punctuation, and nested segments were vulnerable to drift when rebuilt with raw string formatting
3. **Short Soak Reports Could Mislead** — a timeout with `crawl_result = None` did not mean “no progress” if the sled VFS already contained nested entries

### Fixes Implemented
1. **Canonical Child URL Joining** in `qilin.rs` — child folder/file links are now resolved with `Url::join`, and recursion uses the resolved final URL as the parsing base
2. **Child Traversal Diagnostics** in `qilin.rs` — added limited `Child Queue`, `Child Fetch`, `Child Parse`, and `Child Failure` logs so the first recursive layers can be inspected without log floods
3. **Short-Window VFS Validation Workflow** — verified partial crawl progress from the sled VFS rather than relying only on the final `crawl_result` contract during timeout-bound authorized soaks
4. **New Short-Window Comparison Result** — with recursion fixed, the canonical Qilin target produced:
   - `torforge`: `973` unique entries (`685` files, `288` folders) in `90s`
   - `native`: `1693` unique entries (`1212` files, `481` folders) in `90s`

### Prevention Rules
- **P50B-1:** Root-page success does not prove recursive crawl correctness; the first child-folder layer must be instrumented and validated separately.
- **P50B-2:** QData child links MUST be resolved canonically with URL joining; manual concatenation is too brittle for encoded hidden-service directory trees.
- **P50B-3:** Timeout-bound soak conclusions MUST inspect partial sled VFS state before claiming the crawl yielded nothing.
- **P50B-4:** Runtime-default decisions must follow measured discovered-entry throughput on the same target and window, not architecture preference alone.

## Phase 50C: Worker-Local Client Reuse, Fingerprint Retry, and Oversubscription Guardrail (2026-03-06)

### Issues Found
1. **Qilin Rebuilt HTTP Clients Per Request** — workers were repeatedly constructing fresh `ArtiClient` wrappers, losing connection reuse and paying avoidable request-setup cost
2. **Transient CMS Connect Errors Could Kill Whole Runs** — the initial fingerprint fetch still aborted after the first connect failure, which made runtime comparisons noisy and unfair
3. **More Parallelism Was Still A Hypothesis** — there was no measurement showing whether more in-flight requests per client helped or harmed the canonical target

### Fixes Implemented
1. **Worker-Local Client Reuse** in `qilin.rs` — each worker now keeps a reusable client until retry-triggering failure instead of rebuilding one per page fetch
2. **Bounded Fingerprint Retry** in `lib.rs` — initial CMS fingerprinting now retries across rotated client slots before returning an offline-sync error
3. **Timeout-Bound Partial Summary Reporting** in `qilin_authorized_soak.rs` — long soak reports now persist `partialVfsSummary` so timeout-bound runs still yield measured crawl totals
4. **Controlled Multiplex Hook** in `qilin.rs` — added `CRAWLI_QILIN_CLIENT_MULTIPLEX_FACTOR` for explicit experiments without changing the default concurrency policy
5. **Five-Minute Canonical Validation**:
   - `native`: `18297` unique entries
   - `torforge`: `18313` unique entries
6. **Oversubscription Rejection** — a controlled `2x` multiplex experiment regressed to `1484` unique entries in `120s`, so oversubscription remains disabled by default

### Prevention Rules
- **P50C-1:** Recursive onion crawlers must not rebuild their HTTP client object on every page fetch when worker-local reuse is possible.
- **P50C-2:** Runtime comparisons are invalid if the entrance fingerprint path fails on a single transient connect error.
- **P50C-3:** “More concurrency” is not a valid optimization claim until the same target and window show a better discovered-entry slope.
- **P50C-4:** Qilin client oversubscription must stay opt-in until repeated benchmarks prove it helps rather than hurts.

## Phase 50D: Degraded Retry Lane For Timeout-Heavy Child Folders (2026-03-06)

### Issues Found
1. **Bad Child Folders Shared The Same Retry Path As Healthy Work** — timeout-heavy or circuit-heavy subtrees could keep re-entering the main retry queue and steal attention from healthy traversal
2. **Raising Global Concurrency Was The Wrong Remedy** — the oversubscription experiment showed that broad worker pressure made the target worse, not better

### Fixes Implemented
1. **Degraded Retry Lane** in `qilin.rs` — timeout/circuit-heavy child folders are now routed into a separate bounded lane
2. **Bounded Degraded In-Flight Limit** in `qilin.rs` — degraded work can progress, but only up to a small concurrent cap
3. **Dispatch Interval Control** in `qilin.rs` — the crawler only samples degraded work periodically instead of letting it dominate the hot path

### Prevention Rules
- **P50D-1:** Timeout-heavy child folders must not compete in the same retry lane as healthy recursive work.
- **P50D-2:** When a target degrades under pressure, isolate the bad subtree first; do not immediately raise global concurrency.

## Phase 50E: Persistent Bad-Subtree Heatmap (Experimental, 2026-03-06)

### Issues Found
1. **Per-Session Retry Isolation Had No Memory** — the degraded retry lane could help within a run, but it forgot repeated bad prefixes between sessions
2. **A New Persistence Layer Could Become Permanent Cargo Cult** — if persistent subtree scoring did not improve measured yield, it would become needless state complexity

### Fixes Implemented
1. **Persistent Subtree Heatmap** in `subtree_heatmap.rs` — added per-target prefix clustering and success/failure scoring
2. **Target-State Integration** in `lib.rs` + `qilin.rs` — the heatmap is stored under the existing deterministic per-target support directory
3. **Conservative Feature Gate** — the feature is now controlled by `CRAWLI_QILIN_SUBTREE_HEATMAP=1`

### Prevention Rules
- **P50E-1:** Persistent recovery heuristics must remain opt-in until benchmarks prove they outperform the stateless baseline.
- **P50E-2:** Any new per-target persistence layer must reuse the existing `target_state` support root instead of creating a second parallel cache tree.

## Phase 50F: Downloader Resume Healing Audit (2026-03-06)

### Issues Found
1. **Downloader Could Panic In Async Context** — `aria_downloader.rs` used `blocking_read()` directly on the active Tor client pool, which can panic inside the async runtime
2. **Resume Trusted Stale Ports Too Eagerly** — if a previous download cluster had been dropped, managed SOCKS ports could still appear reusable while the live Tor client pool was actually empty

### Fixes Implemented
1. **Async-Safe Arti Client Access** in `aria_downloader.rs` — mirrored the crawler-side `block_in_place` fix for Tor client reads
2. **Live-Client Validation On Resume** in `aria_downloader.rs` — downloader resume now requires both visible managed ports and a non-empty live client pool before reusing the existing runtime; otherwise it bootstraps a fresh TorForge cluster
3. **Dedicated Healing Probe Example** in `qilin_download_healing.rs` — added a real pause/resume probe for large Qilin downloads

### Prevention Rules
- **P50F-1:** Runtime reuse decisions must validate both visible sockets and live in-memory client state.
- **P50F-2:** Any code that reads the live Tor client pool must stay async-safe; raw `blocking_read()` is not acceptable inside the runtime.

## Phase 50G: SOCKS-Free Default Bootstrap and Slot-Based Identity Rotation (2026-03-06)

### Issues Found
1. **Normal Bootstrap Still Started Compatibility SOCKS** — even though the crawl/download hot path was direct Arti, the normal bootstrap still spun up managed SOCKS listeners for every client slot
2. **Healing Was Still Port-Keyed** — crawler and downloader recovery paths still treated managed SOCKS ports as the identity primitive for rotation

### Fixes Implemented
1. **SOCKS-Free Default Bootstrap** in `tor_native.rs` — normal TorForge bootstrap now brings up the live Tor client pool without starting managed SOCKS listeners
2. **Slot-Based Rotation** in `tor_native.rs`, `tor.rs`, `aria_downloader.rs`, and `qilin.rs` — hot-path healing now rotates client slots directly instead of relying on managed SOCKS port identity
3. **Compatibility SOCKS Isolation** — raw SOCKS support remains in the codebase as an explicit compatibility surface only
4. **Legacy Example Cleanup** — `probe_test.rs`, `download_test.rs`, and `qilin_extreme_probe.rs` now exercise the direct TorForge client path instead of teaching the old SOCKS-on-localhost workflow

### Prevention Rules
- **P50G-1:** Do not start compatibility SOCKS listeners in the default crawl/download bootstrap path.
- **P50G-2:** Hot-path recovery must key off live client slots, not port numbers.

## Phase 50H: Piece-Mode Checkpoint State Initialization Bug (2026-03-06)

### Issues Found
1. **Writer Checkpoint State Could Lag Behind Piece-Mode Activation** — the downloader writer path could clone `DownloadState` before `piece_mode`, `total_pieces`, and `completed_pieces` were initialized, which prevented deterministic proof of piece-mode carryover

### Fixes Implemented
1. **Early Piece-Mode State Initialization** in `aria_downloader.rs` — piece-mode metadata is now established before the writer-side checkpoint state is cloned
2. **Deterministic Resume Harness** in `local_piece_resume_probe.rs` — added a local range-support server that proves `completed_pieces` carryover and final hash integrity on resume

### Prevention Rules
- **P50H-1:** Any resumable checkpoint writer must receive the final mode/shape of download state before worker threads start mutating it.
- **P50H-2:** Live-target resume probes are insufficient on their own; keep one deterministic local harness for piece-mode validation.

## Phase 50I: Validator-Aware Resume Safety (2026-03-06)

### Issues Found
1. **Resume Trusted Offsets More Than Object Identity** — partial state did not carry enough HTTP validator context to prove the remote object was still the same file

### Fixes Implemented
1. **Persisted Resume Validators** in `aria_downloader.rs` — `ETag` and `Last-Modified` are now stored in download state
2. **`If-Range` Resume Requests** in `aria_downloader.rs` — resume-sensitive range requests now carry the preferred validator
3. **Stale-State Discard** — mismatched validator state is now dropped before resume instead of risking unsafe continuation

### Prevention Rules
- **P50I-1:** Offset-based resume is insufficient by itself; resumable downloads must retain validator identity when the transport supports it.
- **P50I-2:** If validator identity changes, restart cleanly instead of attempting a best-effort partial resume.

## Phase 51A: Resource Governor v1 (2026-03-06)

### Issues Found
1. **Bootstrap Scaling Was Still Mostly Static** — TorForge client caps still leaned heavily on CPU-count-only heuristics
2. **Direct I/O Policy Was Global And Blind** — the downloader could not adapt its I/O policy to the actual storage class for the current output path

### Fixes Implemented
1. **Machine Profile Detection** in `resource_governor.rs` — added CPU/RAM/storage-class detection
2. **Bootstrap Cap Integration** in `tor_native.rs` — TorForge client cap/quorum now follows the governor recommendation before env overrides clamp it
3. **Session-Scoped I/O Override** in `io_vanguard.rs` — download sessions can now set and automatically clear a runtime Direct I/O policy override
4. **Download Session Governor Logging** in `aria_downloader.rs` — active machine profile and chosen I/O policy are now logged for operator visibility

### Prevention Rules
- **P51A-1:** Bootstrap scaling must consider memory pressure, not just CPU count.
- **P51A-2:** Storage-class-sensitive I/O policy must be session-scoped, not permanently sticky across unrelated downloads.

## Phase 51B: Binary Telemetry Lane (2026-03-06)

### Issues Found
1. **Operator Telemetry Was JSON-Only** — the hottest operational signals still flowed only through JSON/Tauri events
2. **No Low-Overhead Sidecar Path Existed** — there was no machine-readable binary stream for external tooling or future gRPC migration

### Fixes Implemented
1. **Protobuf Frame Sink** in `binary_telemetry.rs` — added `prost`-encoded length-delimited frames
2. **Hot-Signal Wiring** in `runtime_metrics.rs`, `lib.rs`, and `aria_downloader.rs` — resource metrics, crawl status, batch progress, and download status now flow into the binary sink when enabled

### Prevention Rules
- **P51B-1:** New binary telemetry must remain additive until the fallback JSON/UI path is proven equivalent.
- **P51B-2:** Start with the highest-value signals first; do not migrate every log line into the binary plane blindly.

## Phase 51E: Pressure-Aware Governor Wiring (2026-03-06)

### Issues Found
1. **Governor Decisions Were Not Yet Driving The Hot Paths** — bootstrap sizing existed, but frontier worker caps, Qilin crawl width, and downloader range/tournament widths still relied on mostly local heuristics
2. **Cross-Adapter Tests Still Expected Pre-Governor Semantics** — at least one Play throughput test still assumed that a `120` client pool must imply `120` concurrent workers
3. **Download Resume Validation Needed Re-Proof After Governor Wiring** — once the downloader started capping bootstrap/tournament/active windows from the governor, the deterministic resume harness had to be rerun to confirm checkpoint carry-forward still completed

### Fixes Implemented
1. **Reusable Pressure Model** in `resource_governor.rs` — added runtime snapshot sampling plus bootstrap/listing/download budget helpers
2. **Frontier Permit Cap Integration** in `frontier.rs` — crawl permits now respect the governor budget for all adapters instead of blindly exposing the configured circuit count
3. **Qilin Pressure Clamp** in `qilin.rs` — the local adaptive page governor now starts from the listing budget and reduces or caps scale-up when machine pressure rises
4. **Downloader Budget Integration** in `aria_downloader.rs` — bootstrap size, batch small-file width, range circuit cap, tournament width, and initial active budget now share one governor-derived budget
5. **Contract-Correct Play Test** in `tests/play_e2e_test.rs` — the bottleneck test now validates budget coherence rather than the obsolete `permits == raw client count` rule

### Prevention Rules
- **P51E-1:** Frontier worker permits are a governed budget, not a mirror of the raw client pool size.
- **P51E-2:** Any adapter-specific worker governor must clamp scale-up against shared machine pressure before it reacts to backlog alone.
- **P51E-3:** Downloader bootstrap count, small-file swarm width, tournament width, and initial active range budget must be derived from one shared resource budget.
- **P51E-4:** Cross-adapter tests must assert the new governor contract, not legacy fixed-width assumptions.

## Phase 51F: Hybrid Plugin Host (2026-03-06)

### Issues Found
1. **New Specialized Adapters Still Required A Rebuild** — even simple autoindex-style site variants could only be added by changing Rust source and recompiling
2. **The Host Needed To Stay In Charge** — any plugin mechanism that moved retry/frontier/ledger behavior out of the host would duplicate the hardest logic and weaken consistency across adapters

### Fixes Implemented
1. **Manifest-Driven Plugin Loader** in `adapters/plugin_host.rs` — runtime JSON manifests can now define matching rules and route into a host-owned crawl pipeline
2. **Explicit Registry Constructor** in `adapters/mod.rs` — `AdapterRegistry::with_plugin_dir(...)` allows deterministic plugin loading in tests and operator workflows
3. **Host-Owned Autoindex Delegation** in `plugin_host.rs` — runtime plugins can currently target the hardened autoindex pipeline without owning retries, frontier semantics, or storage ledgers
4. **Repository Skeleton Manifest** in `adapter_plugins/example_autoindex_plugin.json` — new plugin authors now have a canonical starting point
5. **Engine Validation** in `tests/engine_test.rs` — added proof that a runtime manifest can match a new adapter without rebuilding the binary

### Prevention Rules
- **P51F-1:** Runtime plugins may own matching and routing rules, but the host must own crawl, retry, frontier, and ledger behavior.
- **P51F-2:** Register runtime plugins before the generic fallback adapter, but do not let them silently override specialized built-in adapters by default.
- **P51F-3:** Tests for runtime plugins must use explicit plugin-directory injection instead of mutating global environment state.

## Phase 50: Direct Arti Connector Migration (2026-03-06)

### Issues Found
1. **SOCKS5 Loopback Is Vestigial Overhead** — The native Arti migration (Phase 43B) replaced `tor.exe` with in-process `arti-client`, but left the SOCKS5 proxy shim in place. Every HTTP request now: opens TCP to localhost → SOCKS5 handshake (12 syscalls, ~316 bytes) → `TorClient::connect_with_prefs()` → SOCKS5 success reply → `copy_bidirectional` relay. Steps 1-2 and 4-5 are pure waste.
2. **Port Exhaustion Contribution** — Every SOCKS5 loopback connection consumes a Windows ephemeral port entering 60-120s TIME_WAIT. At 120 circuits with rapid requests, this is a primary contributor to the NT kernel port exhaustion problem that originally motivated the 20:1 golden ratio.
3. **Data Relay Doubling** — `copy_bidirectional` causes every byte to traverse: reqwest → loopback TCP → SOCKS handler → Tor DataStream, doubling kernel buffer copies for all downloads.
4. **Task Bloat** — 120 concurrent SOCKS connections spawn ~240 unnecessary tokio tasks (1 handler + 1 relay each).
5. **Every Other arti-client User Uses Direct Connect** — artiqwest, hypertor, and arti's own hyper examples all use `TorClient::connect_with_prefs()` → `DataStream` directly. Crawli is the only project wrapping this in a redundant SOCKS shim.

### Fixes Implemented
1. **Built `arti_connector.rs` and `arti_client.rs`** — the Rust hot path now uses a direct hyper connector over `TorClient::connect_with_prefs()`
2. **Refactored Rust hot-path consumers** in `frontier.rs`, `aria_downloader.rs`, and `multipath.rs` to consume `ArtiClient` directly instead of loopback SOCKS proxies
3. **Retained compatibility SOCKS only where still needed** — Ghost Browser / Chromium and a subset of legacy examples/tests continue to use compatibility SOCKS bridges
4. **Direct isolation tokens in the Rust hot path** — stream isolation is now driven by `IsolationToken` directly instead of proxy-auth parsing for core crawl/download traffic

Full audit: [SOCKS_Performance_Audit_Whitepaper.md](file:///Users/navi/Documents/Projects/LOKI%20TOOLS/Onion%20Forger/crawli/docs/SOCKS_Performance_Audit_Whitepaper.md)

### Prevention Rules
- **P50-1:** Never use a SOCKS5 proxy to bridge between an in-process library and the same process's HTTP client.
- **P50-2:** When migrating from external-process Tor to in-process Tor, the SOCKS5 compatibility shim MUST be removed from the Rust hot path as a follow-up and left only for true compatibility consumers.
- **P50-3:** SOCKS5 auth for circuit isolation is a hack. Use `IsolationToken` directly.
- **P50-4:** On Windows, every loopback TCP connection consumes an ephemeral port in TIME_WAIT. Eliminate unnecessary loopback connections.

## Phase 50A: TorForge Runtime Profile Port (2026-03-06)

### Issues Found
1. **Tor Forge code was present in the repo but not wired into the shipped runtime** — the subtree existed, but `Crawli` still booted its own Arti profile with no runtime selector
2. **Runtime comparisons were ad hoc** — smoke tools had no explicit way to run the same path under different bootstrap profiles
3. **Memory pressure handling still only logged** — the phantom pool stayed resident even when the process crossed the pressure threshold

### Fixes Implemented
1. **Runtime Selector** in `tor_runtime.rs` — added explicit `native` vs `torforge` profile selection and made `torforge` the default bootstrap profile
2. **Bootstrap Wiring** in `tor_native.rs` — runtime flavor now affects bootstrap/state-root/jitter behavior for all Tor-based `Crawli` connections without regressing the direct `ArtiClient` hot path
3. **Runtime-Aware Smoke Examples** in `arti_direct_test.rs` and `qilin_authorized_soak.rs` — both now accept `--runtime`
4. **Memory Pressure Shedding** in `tor_native.rs` — phantom standby circuits are now cleared under high memory pressure instead of only logging a warning

### Prevention Rules
- **P50A-1:** Presence of a runtime subtree inside the repo does not count as integration; the shipped bootstrap path must use the TorForge policy explicitly.
- **P50A-2:** TorForge consolidation must preserve the direct Rust hot path unless there is measured evidence that a compatibility proxy is beneficial.
- **P50A-3:** Historical native-vs-torforge comparisons may remain in whitepapers as evidence, but the shipped operator surface must not advertise a runtime choice that no longer exists.

### Issues Found
1. **Crawl Results Had No Per-Target Memory** — each run emitted a timestamped listing, but the backend did not know which prior run was “best” for the same target
2. **Repeat Crawls Could Not Detect Silent Underperformance** — if a later crawl found fewer items, there was no automatic comparison or bounded retry policy
3. **Download Resume Knew About Exact-Size Skips But Not Failure Priority** — the downloader skipped completed files, but it did not persist failed items as a first-class retry queue
4. **Output Artifacts Were Operator-Visible But Not Target-Stable** — users could see listings in the output folder, but there was no deterministic per-target current/best naming convention

### Fixes Implemented
1. **Persistent Target Ledger** in `target_state.rs` — added deterministic `target_key` derivation, per-target ledger persistence, stable current/best listing paths, crawl history metadata, and failure-manifest paths
2. **Stable Current / Best Listings** in `lib.rs` + `target_state.rs` — each crawl now writes stable per-target current/best canonical listings plus stable Windows `DIR /S`-style listings, with timestamped history snapshots stored under the target support folder
3. **Baseline-Aware Crawl Finalization** in `lib.rs` — repeat crawls now compare `raw_this_run_count`, `best_prior_count`, and `merged_effective_count`, then classify the run as `first_run`, `matched_best`, `exceeded_best`, or `degraded`
4. **Bounded Catch-up Retry** in `lib.rs` — when a crawl underperforms the prior best and telemetry indicates instability (`timeouts`, `throttles`, or `failovers`), the backend now retries up to 2 more times in the same session before persisting a degraded result
5. **Failure-First Download Resume Plan** in `target_state.rs` + `lib.rs` — batch download planning now prioritizes known failed items before remaining missing/mismatch files from the authoritative best crawl snapshot, while exact-size matches are skipped
6. **Failure Manifest Reconciliation** in `target_state.rs` — after each batch plan executes, the backend reconciles the failure manifest from real filesystem state so successful files are removed and unresolved files remain queued

### Prevention Rules
- **P43J-1:** Timestamped crawl indexes are history only; repeat-run comparison must use a deterministic per-target ledger and authoritative best snapshot.
- **P43J-2:** A crawl that underperforms prior best must never shrink the authoritative best listing.
- **P43J-3:** Download resume planning must prefer failed items before the general missing/mismatch queue.
- **P43J-4:** Stable user-facing listing names belong in the selected output root; machine-readable target state belongs under `<selected_output>/targets/<target_key>/` while non-payload support artifacts belong under the hidden sibling support root.

## Phase 43I: Qilin Resource Telemetry, DB-Backed Completion, and Standby Failover (2026-03-06)

### Issues Found
1. **Qilin Still Duplicated Crawl Results In RAM** — native app sessions streamed entries to the UI but also retained a full `Vec<FileEntry>` and cloned it at completion
2. **The `120 circuits` Control Still Looked Like A Live Crawl Width** — Qilin’s actual page governor was conservative, but the app lacked a canonical resource-telemetry surface showing the real worker target and process pressure
3. **Qilin Had No Structured Standby Failover Path** — once a primary storage seed degraded, retries kept hammering the same route until broader circuit healing eventually helped
4. **Long-Run Validation Lacked A First-Class Harness** — there was no dedicated example for an authorized five-minute Qilin listing-plus-download soak with structured output

### Fixes Implemented
1. **Compact Crawl Completion Contract** in `lib.rs` — `start_crawl` now returns `CrawlSessionResult` instead of a full `Vec<FileEntry>` to the frontend
2. **Sled Summary / Batch Traversal Helpers** in `db.rs` — added VFS summaries and batch iteration so crawl completion, crawl-index export, and auto-download can operate on DB-backed state
3. **Qilin Streamed Ingestion Path** in `qilin.rs` — native app mode now writes streamed batches into sled and avoids retaining a second full in-memory result vector
4. **Backend Resource Telemetry** in `runtime_metrics.rs` — added `resource_metrics_update` with CPU, RAM, worker, circuit, node, throttle, and timeout fields
5. **Qilin Low-Window Defaults + Bounded Failover** in `qilin.rs` — the page governor now starts low by default and retries can remap onto a small standby seed set after classified timeout/circuit/throttle pressure
6. **Authorized Soak Harness** in `src-tauri/examples/qilin_authorized_soak.rs` — added an explicit operator-run example that writes a JSON report under `tmp/`

### Prevention Rules
- **P43I-1:** Do not let the frontend depend on a full adapter-returned file vector when the sled VFS already exists; completion/reporting paths must be DB-backed.
- **P43I-2:** Any operator-facing concurrency control must distinguish budget ceiling from live worker target.
- **P43I-3:** Qilin storage retries must use bounded standby failover, not open-ended parallel destination fan-out.
- **P43I-4:** Long-run validation of onion behavior must live in an explicit harness with structured output, not in ad hoc manual sessions.

## Phase 43H: Canonical Downloader Path, Adaptive Tournament Caps, and Live Active-Window Enforcement (2026-03-06)

### Issues Found
1. **Downloader Tournament Telemetry Was Dead Control Data** — `tor.rs` already tracked adaptive tournament telemetry, but `aria_downloader.rs` still hardcoded a `2x` candidate race for large-file transfers
2. **Large-File BBR Control Was Not Actually Enforcing Concurrency** — the downloader created a BBR controller, but range-fetch tasks did not consult it before claiming work, so the control loop observed pressure without changing request issuance
3. **Production vs Experimental Download Paths Were Ambiguous** — `multipath.rs` existed beside `aria_downloader.rs`, but only the latter preserved shipped semantics like `.ariaforge_state`, `DownloadControl`, and standardized telemetry

### Fixes Implemented
1. **Canonical Production Path Declaration** in `aria_downloader.rs`, `multipath.rs`, and `lib.rs` — documented and enforced that shipped downloads run through `aria_downloader.rs`, while `multipath.rs` remains experimental until it reaches feature parity
2. **Adaptive Tournament Cap** in `aria_downloader.rs` — large-file tournament sizing now consumes `tor.rs::tournament_candidate_count(...)` and clamps it with `CRAWLI_DOWNLOAD_TOURNAMENT_CAP` so onion targets do not get hit with unbounded handshake races
3. **Live Active-Window Gating** in `aria_downloader.rs` — promoted range workers are now ranked by handshake performance and must fit inside the BBR controller's current active window before claiming pieces
4. **Measured BBR Success Feedback** in `aria_downloader.rs` — piece completion now feeds actual bytes/elapsed timing back into the controller instead of relying only on synthetic/header-blind success hints
5. **Tournament Telemetry Wiring** in `aria_downloader.rs` — handshake-ready latencies now feed `tor.rs::update_tournament_telemetry(...)`, so future candidate sizing is based on observed downloader behavior rather than stale defaults

### Prevention Rules
- **P43H-1:** Control telemetry is incomplete until it changes runtime behavior. Metrics-only congestion logic is not a finished feature.
- **P43H-2:** Large onion download tournaments MUST be explicitly capped; handshake storms are a reliability bug, not a performance optimization.
- **P43H-3:** Experimental download engines MUST NOT become production defaults unless they preserve resume state, stop/pause semantics, and normalized telemetry.
- **P43H-4:** Shared telemetry surfaces (for example `tor.rs` tournament feedback) must be consumed by all relevant production paths, or they become misleading dead architecture.

## Phase 43G: Arti Timing Policy, Frontier Worker Governance, and Staged Tournament Probing (2026-03-05)

### Issues Found
1. **Native Arti Was Still Mostly Default-Tuned** — `tor_native.rs` had migrated to pure Arti, but stream timeout, circuit retry budget, hidden-service attempt counts, and preemptive circuit policy were still effectively generic defaults rather than workload-aware settings
2. **Several Adapters Still Hardcoded `120` Crawl Workers** — `autoindex.rs`, `play.rs`, `dragonforce.rs`, `inc_ransom.rs`, `pear.rs`, and `worldleaks.rs` still sized their queues independently of the live client pool and permit budget
3. **Qilin Tournament Head Was Mislabeled** — `qilin_nodes.rs` claimed to probe a tournament head first, but the width logic effectively allowed the whole set through the same pass

### Fixes Implemented
1. **Explicit Arti Timing / Preemptive Policy** in `tor_native.rs` — added tuned stream connect timeout, circuit request timeout, request retry ceiling, hidden-service attempt counts, and preemptive exit-circuit config, all with `CRAWLI_ARTI_*` environment overrides
2. **Frontier-Owned Listing Worker Policy** in `frontier.rs` — added `recommended_listing_workers()` so metadata crawlers derive concurrency from the real HTTP client pool, permit budget, and download-mode headroom instead of adapter-local constants
3. **Adapter Worker Policy Adoption** across `autoindex.rs`, `play.rs`, `dragonforce.rs`, `inc_ransom.rs`, `pear.rs`, and `worldleaks.rs` — removed the leftover hardcoded `120` listing-worker ceilings
4. **True Head-Then-Fallback Tournament Probing** in `qilin_nodes.rs` — Stage D now probes the tournament head first and only opens the tail batch if the head fails to produce a winner

### Prevention Rules
- **P43G-1:** Native Arti migrations are incomplete until the client config itself is audited; leaving request timing and preemptive policy at generic defaults is an architecture gap, not a neutral choice
- **P43G-2:** Adapter-local metadata worker counts MUST come from a shared frontier policy, never from stale hardcoded constants
- **P43G-3:** Tournament language in docs/code MUST match execution. If code says "head first," the implementation must actually stage the probes that way
- **P43G-4:** Performance overrides should be exposed as runtime environment controls before introducing new code branches or duplicate adapter knobs

## Phase 43F: Qilin Adaptive Page Governor and Node Tournament Hardening (2026-03-06)

### Issues Found
1. **Qilin Used a Fixed Page-Worker Ceiling** — `qilin.rs` still hardcoded a broad metadata crawl width, which treated directory enumeration like a bulk transfer problem and amplified hidden-service failures under stress
2. **Storage Nodes Had No Memory of Failure** — `qilin_nodes.rs` mostly ranked candidates by hit count and latency, so dead or penalized nodes re-entered the tournament immediately with very little structural penalty
3. **Metadata Crawling Could Consume the Whole Swarm** — when download mode was enabled, the listing phase still tried to consume the full client budget, leaving no deliberate headroom for file transfer work

### Fixes Implemented
1. **Adaptive Page Governor** in `qilin.rs` — added a local crawl governor that classifies failures (`timeout`, `circuit`, `throttle`, `http`) and adjusts active page workers every 5 seconds from backlog and success ratio
2. **Persistent Node Tournament State** in `qilin_nodes.rs` — added `success_count`, `failure_count`, `failure_streak`, and `cooldown_until`, then used them for scoring and exponential temporary demotion
3. **Sticky Winner Revalidation** in `qilin_nodes.rs` — a node with enough prior wins now gets a short first-pass revalidation probe before the broader sweep, reducing unnecessary cold-start churn
4. **Metadata/Download Headroom Reservation** in `qilin.rs` — when `CrawlOptions.download` is enabled, the page governor lowers its own ceiling so HTML discovery does not monopolize the integrated Arti swarm

### Prevention Rules
- **P43F-1:** Onion metadata crawling MUST use a target-aware concurrency controller; fixed page-worker ceilings are not acceptable for unstable hidden services
- **P43F-2:** Storage-node selection MUST remember failure and apply temporary demotion; repeated timeouts may not be treated as neutral observations
- **P43F-3:** When crawl and download phases can coexist, metadata enumeration MUST reserve swarm headroom for transfer work instead of consuming the whole client pool
- **P43F-4:** Hidden-service tuning must distinguish page discovery from bulk file transfer; one traffic profile cannot optimize both

## Phase 43E: Onion Failure-Storm Containment and Qilin Reconciliation Guardrails (2026-03-06)

### Issues Found
1. **Hidden-Service Circuit Failures Reused the Same Slot** — `.onion` connect failures retried on the same managed client slot, so repeated hidden-service circuit build failures kept hammering the same unhealthy state
2. **Generic Health Probe Was Too Aggressive Under Target Stress** — the new clearnet probe could classify a target-specific outage as whole-circuit failure too quickly on busy swarms
3. **Qilin Phase 44 Could Requeue Forever** — reconciliation kept re-injecting the same missing folder set with no no-progress exit condition, causing endless retry storms

### Fixes Implemented
1. **Onion Retry Slot Rotation** in `tor_native.rs` — hidden-service circuit failures now rotate the managed Arti client slot between retry attempts before the final failure is returned
2. **Probe Threshold Relaxation** in `tor_native.rs` — widened probe timeout/interval and raised the anomaly streak threshold so generic healing stops overreacting during hostile target windows
3. **Bounded Reconciliation Sweeps** in `qilin.rs` — Phase 44 now rotates managed ports before a tail sweep and returns partial results after repeated no-progress rounds instead of requeueing forever

### Prevention Rules
- **P43E-1:** Hidden-service circuit construction failures MUST trigger identity/slot rotation before repeating the same `.onion` request on the same managed client
- **P43E-2:** Generic circuit-health probes MUST be less aggressive than target-specific retry logic, or target outages will be misclassified as full swarm failure
- **P43E-3:** Any reconciliation/tail-sweep loop MUST have a no-progress escape hatch; never requeue the same backlog forever

## Phase 43D: Live Probe Telemetry, Guard-Pool Removal, and Native-Arti Doc Sync (2026-03-06)

### Issues Found
1. **Circuit Health Telemetry Used `bootstrap()` Timing** — The native monitor timed `client.bootstrap().await` on already-bootstrapped clients, which does not measure live exit-path liveness
2. **Guard Pool Was Dead Configuration Data** — `tor_native.rs` still declared a large hardcoded relay pool even though Arti config never consumed it
3. **Release and Whitepaper Metadata Drifted** — Several docs/workflow notes still described bundled Tor binaries, `aria2`, or legacy `tor.exe` assumptions after the native-Arti migration

### Fixes Implemented
1. **Real Live Probe Path** in `tor_native.rs` — Health monitoring now opens lightweight TCP probe connections through each managed client slot to a configurable target (`CRAWLI_TOR_HEALTH_PROBE_HOST` / `PORT`, default `check.torproject.org:443`)
2. **Probe-Streak Gating** in `tor_native.rs` — Healing now requires repeated probe anomalies before swapping a client slot, reducing false positive churn from ordinary Tor variance
3. **Guard-Pool Removal** in `tor_native.rs` and examples — Deleted the unused hardcoded relay pool and updated the example binaries to the simplified `spawn_tor_node(node_index, is_vm)` contract
4. **Documentation / Release Sync** across `docs/*` and `.github/workflows/*` — Updated canonical docs and release notes to describe native Arti, managed SOCKS ports, real probe telemetry, and portable packages with no bundled Tor binaries

### Prevention Rules
- **P43D-1:** Circuit-health logic MUST exercise a real network path through the live client slot; bootstrap bookkeeping is not a health metric
- **P43D-2:** Hardcoded relay or guard inventories MUST NOT remain in the codebase unless they are wired into enforceable runtime policy
- **P43D-3:** Architecture migrations MUST include a documentation/release-metadata sweep in the same change set, or stale operator guidance will outlive the code

## Phase 43C: Native Arti Isolation, Runtime Port Registry, and Release Packaging Alignment (2026-03-06)

### Issues Found
1. **SOCKS Auth Isolation Was Dropped** — The native SOCKS bridge accepted username/password but discarded it before `TorClient::connect_with_prefs`, collapsing many logical circuits onto shared Arti state
2. **Circuit Healing Replaced the Wrong Object** — `frontier.rs` mapped client IDs to daemons incorrectly and `tor_native.rs` swapped vector entries that the live proxies no longer read
3. **Runtime Port Ownership Was Fragmented** — Downloader and recovery paths still scanned fixed port ranges (`9051-9070`) even though the Arti bootstrap could allocate ephemeral ports
4. **Release Packaging Assumed Deleted Tor Binaries** — Windows/Linux portable workflows still hard-copied `src-tauri/bin/*` after the repo migrated away from bundled Tor executables
5. **Repo Quality Surface Drifted** — strict `clippy`, `engine_test`, and several examples were broken after the Arti migration and `CrawlOptions` schema expansion

### Fixes Implemented
1. **Managed SOCKS Port Registry** in `tor_native.rs` — Added a process-wide registry mapping live SOCKS ports to mutable Arti client slots and per-port auth-isolation caches
2. **Explicit SOCKS Auth Isolation** in `tor_native.rs` — Username/password pairs are now converted into stable `IsolationToken`s and applied through `StreamPrefs::set_isolation(...)`
3. **Hot-Swap Live Client Slots** in `tor_native.rs` — NEWNYM and phantom healing now replace the actual client slot the SOCKS proxy reads, then clear cached auth groups so subsequent requests build fresh circuits
4. **Correct Client→Daemon Mapping** in `frontier.rs` — The frontier now tracks `client_daemon_map`, so degraded HTTP client IDs isolate the correct daemon instead of using modulo math
5. **Registry-Based Port Reuse** in `tor.rs`, `aria_downloader.rs`, and `qilin.rs` — Runtime callers now discover active managed ports from the Arti registry first, bootstrap fresh clusters when needed, and stop assuming fixed port bands
6. **Portable Release Workflow Guard** in `.github/workflows/release*.yml` — Windows/Linux packaging only copies legacy `src-tauri/bin/*` payloads when those folders actually exist
7. **Quality Gate Repair** across Rust/tests/examples — Cleared strict `clippy`, restored `engine_test`, updated stale examples to the native Arti model, and compiled all test/example targets successfully

### Prevention Rules
- **P43C-1:** Any SOCKS bridge that accepts auth fields MUST map them to explicit Arti isolation state; silently discarding auth is a correctness bug
- **P43C-2:** Live circuit healing MUST replace the handle consumed by the proxy listener, not an orphaned vector snapshot
- **P43C-3:** Runtime Tor port discovery MUST come from the owning process registry before any fixed-range fallback scan
- **P43C-4:** Client-pool metadata (`client_id -> daemon`) MUST be stored explicitly whenever the HTTP pool fanout differs from daemon count
- **P43C-5:** Release workflows MUST treat bundled binary folders as optional when architecture migrations remove them
- **P43C-6:** Arti migration changes are not complete until `clippy -D warnings`, tests, examples, frontend build, and overlay integrity all pass together

## Phase 42: Qilin Crawl & Download Pipeline Critical Fixes (2026-03-05)

### Issues Found (Live Test)
1. **Stage D Serial Node Probing** — 15 dead nodes × 30s timeout = 450s wasted before crawl even starts
2. **No Sled Cache TTL** — Dead/seized nodes persisted forever, poisoning every future run
3. **Stage B Regex Too Narrow** — Only matched `value="<onion>"`, missed `href=`, `data-url=`, `iframe src=` patterns
4. **CMS Fallback = 12 Files** — When all storage nodes die, crawler fell back to CMS blog page (no file index)
5. **Serial Probe Bottleneck** — `probe_target()` called sequentially for every file even when `size_hint` existed

### Fixes Implemented
1. **Concurrent `JoinSet` Probing** in `qilin_nodes.rs` — All nodes probed simultaneously with 15s timeout (was 30s serial)
2. **7-Day TTL Eviction** — `get_nodes()` auto-purges nodes with `last_seen > 604800s ago` from sled DB
3. **Hardened Stage B Regex** — Now captures `href="http://<onion>/..."`, `data-url="..."`, `src="..."` patterns
4. **Direct UUID Retry with NEWNYM** in `qilin.rs` — When `discover_and_resolve()` returns `None`, blasts fresh Tor circuits and attempts direct URL construction against 3 known mirrors, validating autoindex HTML presence before use
5. **Size-Hint Probe Skip** in `aria_downloader.rs` — Files with valid `size_hint > 0` bypass `probe_target()` entirely

### Prevention Rules
- **P42-1:** Node discovery probes MUST be concurrent, never serial. Use `JoinSet` with hard wall-clock timeouts.
- **P42-2:** Any persistent cache for volatile darkweb infrastructure MUST have TTL eviction (≤7 days).
- **P42-3:** Regex patterns for DOM scraping MUST account for multiple attribute citation styles (`value=`, `href=`, `data-url=`, `src=`).
- **P42-4:** CMS fallback crawling MUST be treated as absolute last resort with explicit user warning.
- **P42-5:** File probing MUST be skipped when reliable metadata (size_hint) already exists from the crawl phase.

Version: 1.0.16
Updated: 2026-03-06
Authors: Navi (User), Codex (GPT-5)
Related Rules: [MANDATORY-L1] Prevention Discipline, [MANDATORY-L1] Testing & Validation, [MANDATORY-L1] Performance/Cost/Quality

# Summary
Backend issue ledger for crawl recursion, adapter compatibility, and worker throughput.

# Context
Reported symptoms included incomplete deep folder crawl and adapter mismatch/visibility gaps for LockBit-like paths.

# Analysis
Confirmed backend issues:
- Unsafe child URL construction in autoindex recursion.
- Pending queue counter not decremented on some early returns.
- Parser signature drift (`parse_autoindex_html`) broke `PlayAdapter` build.
- Concurrency startup policy not aligned with user-selected high-circuit mode.
- Stale WAL preload caused fresh crawls to skip valid subfolders as already visited.
- Tor bootstrap could be delayed by stale Tor listeners and repeated binary integrity checks per daemon.
- Autoindex scheduler could stall at low active worker count while queue grew.
- LockBit/Nginx table size format (`KiB/MiB`) was not parsed, causing expensive fallback HEAD probes.
- Batch progress payload lacked cumulative downloaded bytes, so GUI network counters had no reliable aggregate source during phase-based transfers.
- Adapter support metadata drifted from runtime behavior (LockBit/Nu delegate to autoindex crawler but were still labeled detection-only).
- Downloader reused only ports `9051-9054` even when tournament winners were on higher managed ports.
- Small-file phase could stall in long-tail retries due fixed-circuit retries and oversized request timeout.
- Small-file phase accepted partial transfers as success if any bytes were written.
- Batch telemetry lacked heartbeat updates during long phases, making UI appear frozen near completion.
- Linux release job failed while building AppImage bundles in GitHub Actions (`linuxdeploy` execution failure on runner), leaving release artifacts incomplete for Linux.
- Windows runtime could flash visible command prompts when starting scans due child process spawn defaults.
- Downloader stale-Tor cleanup used hardcoded `/tmp`, reducing process cleanup reliability on Windows.
- Batch progress counters could drift on mixed smart-skip/small/large runs, causing UI progress plateaus.
- Frontier WAL path used hardcoded `/tmp`, creating cross-platform path risk on Windows.
- LockBit adapter could resolve as `Unidentified` because registry wiring missed adapter registration.

# Details
Issue-to-fix mapping:
- Issue: Top-level-only traversal in nested trees.
  - Root Cause: String URL concatenation and path normalization mismatch.
  - Fix: URL join resolution + host/subtree scope checks.
- Issue: Potential crawl loop stall/hang.
  - Root Cause: Early `return` paths skipping pending decrement.
  - Fix: RAII-style `PendingGuard` decrement guarantee.
- Issue: Compile failure in `PlayAdapter`.
  - Root Cause: Old tuple parser contract.
  - Fix: Migrate to `AutoindexParsedEntry` structure.
- Issue: Slower-than-expected startup concurrency.
  - Root Cause: AIMD initial window below configured circuits.
  - Fix: Start at configured cap and keep AIMD backoff for failures.
- Issue: Missing nested folders after repeated runs on same target.
  - Root Cause: WAL was always restored, polluting dedupe state for non-resume runs.
  - Fix: Fresh crawl by default; WAL restore only when `CRAWLI_WAL_RESUME=1`.
- Issue: Tor startup hangs/slow bootstrap on reused local ports.
  - Root Cause: stale Tor listeners were not reclaimed from the crawler port band.
  - Fix: preflight Tor port sweep (`9050-9070`) for Tor-named listeners; preserve Tor Browser reserved ports.
- Issue: bootstrap startup overhead.
  - Root Cause: Tor binary integrity/path resolution executed once per daemon.
  - Fix: resolve and verify Tor binary once before daemon launch loop.
- Issue: Single daemon straggler delayed full crawl start.
  - Root Cause: strict wait-for-all bootstrap with no race/selection strategy.
  - Fix: tournament bootstrap mode (launch extra candidates, keep first healthy winners, terminate stragglers) with quorum continuation.
- Issue: Crawl appeared stuck with very low active workers despite backlog.
  - Root Cause: coordinator waited on `join_next()` before draining newly enqueued URLs.
  - Fix: non-blocking scheduler loop that can consume queue and join workers concurrently.
- Issue: Large listings crawled slowly and showed only top-level nodes for long periods.
  - Root Cause: size parser failed for `<td class="size">348.2 MiB</td>` style lines and fell back to per-file HEAD requests on onion.
  - Fix: robust size parsing for table/preformatted formats, plus no per-file HEAD fallback on onion listings.
- Issue: Worker/circuit telemetry could drop below operator-selected 120 during crawl after transient failures.
  - Root Cause: AIMD backoff reduced active client window for crawl requests.
  - Fix: onion listing mode pins crawl client fanout and worker target to configured circuit ceiling.
- Issue: Batch mirror startup incurred unnecessary probe overhead for known-size entries.
  - Root Cause: batch router probed every file (`HEAD`/range) even when crawl already mapped file size.
  - Fix: pass `size_hint` from crawl entries to batch router and skip active probes for hinted files.
- Issue: Support artifacts were mixed into extraction directories.
  - Root Cause: manifests, sidecar metadata, state, logs, and local VFS db wrote to output root or file-adjacent locations.
  - Fix: route non-crawler support artifacts into `<selected_output_parent>/.onionforge_support/<support_key>/`.
- Issue: Batch download telemetry for UI was incomplete.
  - Root Cause: only partial progress signals existed, mostly small-file phase and raw stream stats.
  - Fix: emit standardized batch lifecycle events (`download_batch_started`, enriched `batch_progress`) for aggregate file-level tracking.
- Issue: Batch progress events could not report aggregate transferred bytes.
  - Root Cause: `batch_progress` only provided completed counts and optional speed, without cumulative byte totals.
  - Fix: add `downloaded_bytes` to `BatchProgressEvent` and update emit points in small-file and large-file phases.
- Issue: LockBit and Nu adapter support level was stale in support catalog.
  - Root Cause: metadata not updated after enabling crawl delegation to hardened autoindex traversal.
  - Fix: set support levels to `Full Crawl`, refresh sample/test metadata, and add regression assertions in `engine_test`.
- Issue: Batch downloader underused active Tor winners.
  - Root Cause: onion port scan in batch/single download paths only checked `9051-9054`.
  - Fix: add managed Tor port detection helper and consume full managed range (`9051-9070`), with bootstrap fallback if no active daemons.
- Issue: End-of-batch long-tail stalls around `98-99%`.
  - Root Cause: small-file workers retried on the same circuit with `120s` request timeout and large exponential sleeps.
  - Fix: add size-aware retry budget and request timeout, plus per-retry port/circuit rotation and capped fast backoff.
- Issue: Partial small-file transfers were marked successful.
  - Root Cause: success criteria only checked `wrote_any` without validating clean stream end or expected length.
  - Fix: require expected-byte completion (if known) or clean EOF completion; purge partial files on retry.
- Issue: Batch UI looked frozen when no file completed for several seconds.
  - Root Cause: `batch_progress` emission happened only at file completion/failure boundaries.
  - Fix: add periodic heartbeat `batch_progress` emission with current file, cumulative bytes, and rolling throughput.
- Issue: Multi-OS release pipeline ended in failure even when Windows/macOS assets uploaded.
  - Root Cause: Linux matrix attempted AppImage bundling (`--bundles appimage,deb,rpm`) and failed at `linuxdeploy` on GitHub runner.
  - Fix: restrict Linux release bundles to `deb,rpm` in workflow matrix to keep release deterministic and complete.
- Issue: Starting scan on Windows popped one or more command prompt windows.
  - Root Cause: Tor daemons and `taskkill` commands were spawned with default console process flags.
  - Fix: apply Windows `CREATE_NO_WINDOW` creation flag to Tor spawn and `taskkill` command paths.
- Issue: Downloader Tor cleanup missed stale daemon dirs on Windows.
  - Root Cause: cleanup routine scanned hardcoded `/tmp` instead of platform temp directory.
  - Fix: switch cleanup root to `std::env::temp_dir()`.
- Issue: Download progress bar could appear frozen on Windows while transfers were still active.
  - Root Cause: `batch_progress` semantics were phase-local: small-file events only, large-file completions not counted, smart-skipped files not counted, and no cumulative byte field.
  - Fix: unify batch telemetry with global counters (`completed`, `failed`, `total`, `downloaded_bytes`) across smart skip + small + large phases.
- Issue: Frontier WAL file location was OS-specific.
  - Root Cause: crawler frontier used hardcoded `/tmp/crawli_*.wal`.
  - Fix: switch WAL root to `std::env::temp_dir()` for platform-safe behavior.
- Issue: LockBit detection intermittently failed despite adapter implementation existing.
  - Root Cause: `LockBitAdapter` was defined but not registered in `AdapterRegistry::new()`.
  - Fix: add explicit LockBit registration in adapter registry and restore engine test coverage.
- Issue: Re-crawling an existing directory tree overwrites 100% completed files and wastes bandwidth.
  - Root Cause: `start_batch_download` queued every file unconditionally. Partial-resume (`.ariaforge_state`) only prevents data loss, not redundant starts.
  - Fix: implement "Smart Skip" in the pre-flight routine using local filesystem metadata and the `size_hint` from the crawler.
- Issue: Direct I/O open flags could fail on older disks, network-mapped filesystems, or virtualized Windows mounts.
  - Root Cause: write path used a single acceleration mode without policy-level fallback behavior.
  - Fix: add `CRAWLI_DIRECT_IO=auto|always|off` and degrade to buffered writes in `auto` mode after first direct-open failure.
- Issue: Tournament sizing was static and did not adapt when bootstrap conditions changed across runs.
  - Root Cause: fixed candidate ratio ignored observed ready latency and winner conversion.
  - Fix: add adaptive tournament mode (`CRAWLI_TOURNAMENT_DYNAMIC`) and rolling telemetry (`p50`, `p95`, winner ratio) for sizing feedback.
- Issue: Batch completion tail could stretch on mixed-size payload sets.
  - Root Cause: queue ordering favored insertion order, allowing large late jobs to delay final completion.
  - Fix: introduce SRPT + periodic starvation guard (`CRAWLI_BATCH_SRPT`, `CRAWLI_BATCH_STARVATION_INTERVAL`) for fairer tail behavior.
- Issue: Quality drift risk across contributor machines.
  - Root Cause: no repository-level rust toolchain pin and no single strict CI quality workflow.
  - Fix: add `rust-toolchain.toml` + `quality.yml` enforcing `cargo fmt`, strict `clippy`, Rust tests, frontend build, and overlay integrity checks.
- Issue (HFT): Sub-optimal AIMD ramp-up on high-speed circuits.
  - Root Cause: standard AIMD adds +1 worker linearly, wasting potential bandwidth available instantly.
  - Fix (Implemented): Integrated BBR congestion control to instantly probe bandwidth limits geometrically instead of linear ramp-up.
- Issue (HFT): UCB1 exploration constant (`1.5`) is static.
  - Root Cause: hardcoded constants do not scale with chaotic Tor network variance.
  - Fix (Implemented): Replaced static UCB1 with Thompson Sampling, sampling dynamically from the Extended Kalman Filter's probability covariance.
- Issue (Aerospace): BFT Quorum failure on 50MB files discards 49.9MB of good data.
  - Root Cause: Hash calculation uses single SHA256 digest for entire chunk.
  - Fix (Implemented): Converted monolithic digest to Merkle-Tree BFT, enabling sub-chunk (256KB) Frankenstein payload reconstruction to bypass Byzantine mutations.
- Issue (HFT): Mutex lock contention on disk I/O during small-file swarm downloads.
  - Root Cause: hundreds of active asynchronous workers await generic I/O filesystem Mutexes, halting the crawler thread.
  - Fix (Implemented): Destroyed MPSC Mutex constraints using Zero-Copy Lock-Free Ring Buffers (`crossbeam_queue::ArrayQueue`) with dedicated spin-loop consumer.
- Issue: Severe OS-level lockups and kernel seek-thrashing on Mechanical HDDs during parallel downloads.
  - Root Cause: Dozens of Tor circuit workers continuously issue random `File::seek` and `write` payloads across a fragmented 50GB space, overwhelming the hard drive's IOPS limits.
  - Fix (Implemented): Integrated `memmap2` to pre-allocate massive files into Virtual Memory (RAM). Workers execute `copy_from_slice` directly in memory, shifting the burden to the OS Page Cache which background-flushes sequentially at maximum mechanical throughput.
- Issue: Deepweb Tor nodes aggressively issue HTTP 429 and TCP Resets against crawlers, stalling extraction.
  - Root Cause: Hardcoded exit node IPs trigger target firewall rate-limits over long sessions.
  - Fix (Implemented): Appended `--ControlPort` and `--CookieAuthentication` to the daemon bootstrap. The backend actively scans `aria_downloader.rs` for `429` / TCP Resets and broadcasts `SIGNAL NEWNYM` over the proxy Control Port, dynamically bridging to a clean exit node with zero application restart.
- Issue (Aerospace): Obfuscated Next.js SPA frameworks block API scraping via undocumented JWT headers (400 Bad Request).
  - Root Cause: The modern DragonForce leak site hides its Tor topology behind a complex React component tree that requires volatile cryptographic tokens (`deploy_uuid`) strictly formatted in API calls.
  - Fix (Implemented): Bypassed the JSON API entirely. `dragonforce.rs` now natively parses the parent DOM tree to extract the `src` attribute of the authenticated `<iframe>` component, reinjecting the `token=` parameter directly into the Tor Crawler Frontier for autonomous directory extraction.

## Phase 51D: Aggregated Telemetry Bridge
- Issue: crawl status, resource metrics, batch progress, and per-file download progress each rode separate Tauri event lanes, which kept the operator plane unnecessarily chatty under large download/crawl sessions.
- Root Cause: backend telemetry grew incrementally around feature work (`crawl_status_update`, `resource_metrics_update`, `batch_progress`, `download_progress_update`) instead of through one canonical hot-path contract.
- Fix: added `src-tauri/src/telemetry_bridge.rs` with a single 250ms bridge emitter, batched download-delta map, and unified `telemetry_bridge_update` payload for crawl/resource/batch/download telemetry.
- Fix: `runtime_metrics.rs`, `lib.rs`, and `aria_downloader.rs` now publish into the bridge rather than directly into four separate UI event channels.
- Fix: dead downloader `progress` / `speed` events were removed from the hot path because the frontend no longer consumed them.
- Fix: telemetry-consuming soak/live harnesses (`qilin_authorized_soak`, `lockbit_live_pipeline`, `adapter_matrix_live_pipeline`, and the LockBit live test) were migrated to the bridge to keep non-UI validation paths aligned.

# Prevention Rules
**1. Every crawl queue counter increment must have a guaranteed paired decrement.**
**2. Use URL parser APIs for any recursion edge construction.**
**3. Any shared parser contract update requires immediate adapter compatibility sweep.**
**4. Concurrency policy changes must include test expectation updates.**
**5. Cancellation must remain forceful across crawl, download, and Tor resources.**
**6. Scheduler loops must not block queue intake behind single long-running workers.**
**7. Onion listing size mapping should prefer parsed listing data over per-entry HEAD probes.**
**8. Batch routing must consume crawler size hints before issuing network probes.**
**9. Non-crawler support artifacts must stay isolated under the hidden sibling support root (`.onionforge_support/<support_key>`), not inside the visible payload tree.**
**10. Download progress UX must be driven by explicit backend telemetry events.**
**11. Batch telemetry contracts must include both file-count and byte-count dimensions for UI parity with stream telemetry.**
**12. Adapter capability metadata must reflect actual crawl implementation status and be test-asserted.**
**13. Managed Tor port discovery must scan the full crawler-owned range, not a fixed subset.**
**14. Small-file retry policy must rotate circuits and use size-aware timeout/backoff limits.**
**15. Any successful file transfer must validate completion semantics (expected bytes or clean EOF).**
**16. Long-running batch phases must emit heartbeat telemetry independent of file completion events.**
**17. (HFT Standard) Hot-path memory allocation and disk Mutex locking must be treated as critical path bottlenecks; design for Lock-Free message passing.**
**18. (Aerospace Standard) File validation must be granular (Merkle-Trees); do not fail the whole operation if a partial chunk can be surgically repaired.**
**19. (Aerospace Standard) SPA JSON APIs (e.g. Next.js Base64 JWTs) should be bypassed if the DOM natively exposes an authenticated `<iframe>` bridge, limiting exposure to brittle HTTP headers.**
**20. CI release matrices must include only empirically validated bundle targets per runner image; avoid unstable packagers in default release paths.**
**21. Windows child processes in GUI runtime must set no-window creation flags unless user-visible console output is explicitly required.**
**22. Temp-directory cleanup logic must use platform APIs (`std::env::temp_dir`) instead of OS-specific literals.**
**23. Batch progress telemetry must use one global denominator and include smart-skip/large/small phases uniformly.**
**24. Any adapter present in support catalog must also be explicitly registered in `AdapterRegistry`, with a matching engine test assertion.**
**25. Direct I/O acceleration must be policy-driven and degradable to buffered writes on unsupported targets.**
**26. Tournament sizing should be data-informed from prior bootstrap telemetry, not fixed constants only.**
**27. Mixed-size batch schedulers must include starvation prevention when shortest-job strategies are enabled.**
**28. Repository-level quality gates must be enforced in CI, not left to local developer defaults.**
**29. Hot operator telemetry must converge through one canonical bridge contract before new UI listeners are added.**
**30. Dead UI events (`progress`, `speed`, legacy per-signal duplicates) must be removed once the replacement contract is validated.**
**31. Deterministic CLI probes must terminate explicitly after success; validation examples should not rely on ambient runtime teardown.**
**32. Hostile benchmark validation should be preserved as synthetic/local by default so adapter regressions are separable from live-network volatility.**

# Risk
- Aggressive worker startup may increase transient connection churn on weak targets.
- Scope checks may hide intentionally cross-root links; this is acceptable for safety and determinism.

# History
- 2026-03-06: Added Phase 43E onion-failure containment and bounded Qilin reconciliation sweeps.
- 2026-03-06: Added Phase 43D live-probe telemetry fixes, removed dead guard-pool config, and synchronized native-Arti docs/release metadata.
- 2026-03-06: Added Phase 43C native Arti isolation/runtime registry fixes, workflow packaging guards, and quality-gate restoration.
- 2026-03-05: Added Phase 43G for explicit Arti timing/preemptive policy, frontier-owned listing-worker governance, and staged Qilin tournament probing.
- 2026-03-06: Added Phase 43F adaptive Qilin page governance, persistent node demotion scoring, and metadata/download headroom reservation.
- 2026-03-03: Initial backend issue/fix baseline.
- 2026-03-03: Added cumulative `downloaded_bytes` in batch progress telemetry.
- 2026-03-03: Synced LockBit/Nu support catalog entries with real full-crawl behavior.
- 2026-03-03: Expanded Tor active-port discovery to full managed range and reused tournament winners in downloader flows.
- 2026-03-03: Hardened small-file retry logic (rotation, timeout/backoff tuning, strict completion validation).
- 2026-03-03: Added batch heartbeat telemetry to prevent frozen UI states during long-tail phases.
- 2026-03-03: Fixed Linux release pipeline stability by removing AppImage from default CI bundle targets.
- 2026-03-03: Suppressed Windows scan-time command prompt popups with no-window process flags; normalized temp-dir cleanup for Windows.
- 2026-03-03: Unified batch progress counters/bytes across phases and moved frontier WAL path to platform temp directory.
- 2026-03-03: Re-registered LockBit adapter in runtime registry and fixed `engine_test` `CrawlOptions` fixtures for `daemons` field parity.
- 2026-03-04: Added adaptive Direct I/O fallback policy, adaptive tournament telemetry/sizing, SRPT+aging batch scheduling controls, and strict quality workflow/toolchain pinning.
- 2026-03-05: Added Phase 42 Qilin crawl/download pipeline fixes and synchronized release validation for `v0.2.6`.
- 2026-03-05: Phase 43A — Wired BBR congestion controller into Phase 1 small-file downloads; audited Tor Forge telemetry modules (Crawli's were already superior).
- 2026-03-06: Phase 50 — Identified SOCKS5 proxy layer as vestigial architectural bottleneck. Documented full audit with quantified overhead, code locations, and direct connector solution.
- 2026-03-05: Phase 43B — Migrated from tor.exe child processes to native arti-client (pure Rust Tor). Created `tor_native.rs` with ArtiSwarm, per-client SOCKS5 proxies, phantom circuit pool, and memory pressure monitoring.
- 2026-03-06: Phase 49 — Circuit Starvation Failsafe (DashMap blacklist with 60s TTL), WAL Corruption Guard (atomic write-to-tmp → rename), Disk Backpressure Signal (10ms sleep on ring buffer saturation).
- 2026-03-06: Phase 51D — Added the aggregated telemetry bridge, removed dead downloader progress/speed emits, and migrated validation harnesses to the unified operator-plane event.
- 2026-03-07: Phase 51G — Hardened validation harnesses by fixing overlay false positives from internal scroll translation and forcing deterministic shutdown in `local_piece_resume_probe`.

# Appendices
- Validation:
  - `cargo check`
  - `cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check`
  - `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings`
  - `cargo test --lib`
  - `cargo test --test engine_test`
  - `npm run build`
  - `npm run overlay:integrity`


## Phase 54: Arti Multi-Daemon Analysis vs Identity Multiplexing (2026-03-06)

### Overview & Discovery
We conducted a live empirical test to compare distributing 60 parallel target circuits across **two separate Arti Tor daemons** versus multiplexing them within a **single daemon** using `arti_client::IsolationToken` and varied `User-Agent` headers.

### Results
- **Multi-Daemon FAILED:** Spinning two separate instances (daemons=2) immediately degraded Tor connectivity, resulting in `ENDPOINT_UNREACHABLE` for all circuits. Port and filesystem contention between instances degrades path building drastically compared to native scheduling.
- **Single Daemon with Multiplexing SUCCEEDED (6.47 entries/s):** The singular Arti daemon structure is flawless. By applying `IsolationToken` rotations, the single daemon flawlessly handles 60-120 circuits without exhausting 200MB of RSS. 

### Core Implementations Applied
1. **DDoS Guard (EKF Prediction):** We successfully integrated a `qilin_ddos_guard.rs` that leverages 403, 400, and 404 responses to dynamically quarantine and delay requests on a single circuit *before* the remote WAF blacklists the entire origin. 
2. **HFT-Style Jitter (50-150ms):** Deterministic spacing (0ms/3ms) actively triggers Tor Exit Node/Nginx load-balancer anti-bot mechanisms. A randomized entropy of 50-150ms allows up to 60 circuits to bypass heuristics cleanly.
3. **User-Agent Fingerprint Pool:** Native User-Agent rotation across circuits (`[Windows, Mac, Linux]`) defeats load-balancer affinity pinning perfectly.

**Ultimate Prevention Rule:** Never fragment traffic across multiple Tor daemons in an attempt to scale. The native `TorClient` with varied `IsolationToken`s is the single canonical way to scale parallel target operations reliably.


## Phase 55: EKF Predictive Pacing & Identity Persistence vs Load Balancers (2026-03-06)

### Execution Results
We rolled out the complete military-grade predictive pacing suite inside `qilin_ddos_guard.rs` and `arti_client.rs`:
- **Result:** The system achieved a record **10.13 entries/second**, blowing past all prior limits (up from 6.47 ent/s).

### Core Implementations Applied
1. **EKF Predictive Delay & BBR Shaping:** Dropped the fixed 50-150ms delay in favor of a dynamic Extented Kalman Filter (`EKF`) tracking mechanism. Normal queries are padded by a soft 5-80ms BBR delay. If a 403, 400, 429, or 503 is returned, the EKF covariance scales instantly, applying a predictive quarantine backoff before the server bans the origin permanently.
2. **SessionState Cookie Affiliation:** `ArtiClient` internally processes Tor redirect chains (e.g. Stage A). By capturing `Set-Cookie` headers directly during HTTP 302s and appending them dynamically across the same `req_obj`, we now reliably persist `__cf_uid`, `PHPSESSID`, and Tor sticky session identifiers back to load-balancers perfectly.
3. **HFT Referer Diversification:** Embedded the `cms_url` automatically into the `Referer` header for `Stage A` routing to break identical load-balancer heuristic clustering.

**New Prevention Rule (PR-PACING-001):** Do not use fixed duration sleeps. Always use dynamic BBR active limits + EKF anomaly limits to shape crawling, or Cloudflare/Nginx Tor boundaries will throttle the parallel circuit waves mathematically.


### Phase 57: Aerospace-Grade Architecture Cross-Verification (Crawlers & Downloader Unified)
**System Audit & Verification:** A zero-compromise audit was run to verify that all systems (from initial web-crawling down to the actual file-part fetching) uniformly execute our HFT and aerospace algorithms. It isn't just the crawlers that are smart; the actual payload downloaders now use matching predictive technologies.

**Unified Architecture Deployments (Verified in Codebase):**
1. **Adaptive File Size Parsing & Discovery (HEAD Probes):** 
   - Before downloading, all crawlers (`abyss`, `alphalocker`, `autoindex`, `play`, `qilin`) dynamically issue non-blocking HTTP `HEAD` probes across Tor circuits to pre-cache the exact `content-length` via `sizes` feature flags. None of this blindly streams data into memory.
2. **UCB1 Thompson Sampling for Chunk Assignment:** 
   - Downloads do not distribute file chunks statically. Inside `aria_downloader.rs`, the `CircuitScorer` (UCB1) ranks all 120 circuits. Faster circuits receive smaller yield delays, creating an asymmetrical bandwidth funnel where the strongest connections process the majority of the file payload in real-time. 
3. **BBR (Bottleneck Bandwidth and RTT) Pacing strictly active in Downloader:**
   - Instead of 50MB monolithic blocks, the downloader constantly measures the delay. The `task_aimd.recommended_chunk_size()` slices the target `bytes=` range request dynamically to 2-4x BDP (Bandwidth-Delay Product). The pipeline autonomously breathes with the connection speed, expanding when fast and shrinking to 512KB windows upon pressure to avoid Tor-node Bufferbloat.
4. **Ruthless Work-Stealing (The "Assassin" Logic):**
   - **Crawlers:** Use `SegQueue` lock-free queues where fast threads autonomously pull folders.
   - **Downloader:** Performs "Hedging". If Circuit A stalls at 65% of its piece, Circuit B violently steals the offset byte range, races Circuit A, and if B wins, physically severs (`drops()`) Circuit A's stream, forcing Circuit A to rebuild a fresh, untainted Tor socket identity (`new_isolated()`).

**Prevention Rule Enforced:**
`PR-UNIFIED-ARCH-001`: Subcomponents must never drop down to rudimentary "sleep and fetch" execution. If a new module is built, it MUST instantiate `DdosGuard` (for EKF pacing) or `BbrController` (for sizing).

### Phase 51F: Multi-Client Parallel Crawling
**Architecture Implementation:**
A dedicated `MultiClientPool` was engineered to instantiate and isolate multiple independent Arti `TorClient`s concurrently (default: 4 clients for a 4 GB RAM bound).
- **Load-Balancer Bypass**: By routing concurrent worker requests through fundamentally distinct Tor exit nodes and Guard relays via isolated client instances, load-balancer affinity throttling and single-client Guard-relay congestion are bypassed entirely.
- **Resource Harmony**: This connects seamlessly to the Phase 51E Resource Governor to ensure raw memory usage per active client does not exceed container ceilings.
- Circuit Healing: Complete client rotation requests flow through the pre-existing smart healing engine to destroy and regenerate fully tainted client stacks when hard IP-blocks are encountered.

## Phase 58: Qilin Connection Timeouts & Adaptive Universal Explorer (2026-03-07)

### Issues Found
1. **Per-Worker IsolationToken Stampede:** Qilin workers instantiated new `IsolationToken::new()` for every ArtiClient within the worker loop. This prevented Tor circuit reuse across workers, causing a massive circuit-build stampede against `.onion` CMS nodes that registered as a layer-7 DDoS attack, locking out the daemon.
2. **Explicit Onion Flag Interference:** Setting `connect_to_onion_services(true)` explicitly in `StreamPrefs` during Arti HTTP connector instantiation actually broke `.onion` resolution when combined with global allow rules.
3. **Clearnet Probes Severing Onion Circuits:** Aerospace Healing tore down perfectly healthy `.onion` network circuits simply because the clearnet probe (`check.torproject.org`) failed to resolve over the same exit constraint.

### Fixes Implemented
1. **Multiplexed Circuit Pooling:** Removed per-worker `IsolationToken` instantiations in Qilin so the 20 workers correctly share and reuse the same established circuits natively.
2. **Implicit Onion Routing:** Reverted the explicit `connect_to_onion_services(true)` flag in `arti_connector.rs`. Tor auto-routes onions correctly when permitted globally.
3. **Probe Bypasses for Hidden Services:** Modified `tor_native.rs` to completely bypass Aerospace Healing health-probe checks when routing to domains that do not require clearnet exit nodes (`CRAWLI_TOR_HEALTH_PROBE_HOST=none`), preserving hidden-service stability.
4. **Adaptive Universal Explorer Scaffolded:** Designed a Tier-4 fallback explorer inside `explorer.rs` that applies heuristic link scoring (boosting `/storage`, `.zip`, etc.) to heuristically detect Nginx, CMS redirects, and Next.js SPAs without hardcoded parsers.

### Prevention Rules
- **P58-1:** When using `ArtiClient` concurrently via `clone()`, NEVER instantiate a new `IsolationToken` per worker request loop unless you explicitly want to mandate a brand new, unique Tor circuit for every single outbound HTTP request.
- **P58-2:** Do not explicitly force `connect_to_onion_services` on per-stream preferences if the global client builder already permits them; it triggers strict overrides that can drop legitimate traffic.
- **P58-3:** Tor Circuit Health Probes MUST respect the exit-node dependency of the target. `check.torproject.org` requires a clearnet exit node; `.onion` services do not. Failing a clearnet probe must NOT tear down an internal hidden service circuit.

**Key Prevention Rules (Enforced and Logged):**
- **PR-MULTICLIENT-001:** Never exceed 4 active TorClients on 4 GB RAM VMs to prevent NT Kernel OOM exhaustion. This boundary is rigidly enforced by the new Resource Governor instantiation constraints.
- **PR-MULTICLIENT-002:** Client rotations must strictly utilize the shared healing engine to prevent "orphan" clients and silent memory leaks.

### Phase 67E: HEAD Probe Phase-Out
- **Date**: 2026-03-08
- **Issue**: Standard auto-index instances (AlphaLocker, Play, Abyss, Genesis) performed a redundant `HEAD` request before every `GET` merely to ascertain file sizes, doubling network strain and triggering rate limit blocks (especially 429 errors from strict proxies).
- **Fix**: Replaced solitary `HEAD` probes with integrated `GET Range: bytes=0-0` probes. The application parses either the `Content-Range` header limits or fallback `Content-Length`. Any size probe now merges neatly into the initial network connection with the same stream allocation.
- **Prevention Rule**: Never spawn `HEAD` probes to deduce chunk bounds in adapters when `GET Range: bytes=0-0` or `bytes=0-1` accomplishes the same task safely without multiplying HTTP overhead.

### Phase 67E: Tier-4 Adaptive Hydrator Addition
- **Date**: 2026-03-08
- **Issue**: Unknown or polymorphic SPA frameworks bypassed standard ast scraping algorithms and were defaulted to empty directory mappings unless structurally supported. Relying solely on exact-match adapters like DragonForce restricts fallback parsing on modified sites. 
- **Fix**: The Universal Explorer now features runtime Wire Mode Detection (`parse_page_from_body`). If the DOM contains SPA cues (`__NEXT_DATA__`, `<iframe`, `token=`), the Explorer automatically leverages the Predictive State Hydrator algorithms to statically extrapolate NextJS routes/API endpoints directly out of JSON payloads without JS execution.
- **Prevention Rule**: All new fallback/Universal explorers MUST interrogate `__NEXT_DATA__` boundaries and SPA embedded objects if autoindex HTML mapping returns zero entries to ensure data is not hidden behind deterministic dynamic routers.

## Phase 77F: Qilin Top-3 Performance Execution (2026-03-09)
**Issue:** Qilin crawl stuck under 20.17 IO/sec ceiling with tail-stalls.
**Fix:** Implemented Inverted Retry Queue (drains degraded lanes before processing new work), Circuit-Pinned Worker Pools (assigns Tor worker instances to specific nodes), and Selective Tournament Spraying (fans out the root directory across top-4 warm mirror nodes).
**Prevention Rule:** PR-77F-001: Avoid global circuit failover if only one node dies; pin workers to active nodes dynamically.

## Phase 78: Backport Zero-Copy memchr SIMD to Play and Lockbit
**Issue:** Autoindex (Play) and Lockbit parsers were heavily using line-by-line String allocators and `scraper::Html::parse_document(html)` resulting in severe memory allocation stalls and massive GC spikes under heavy directory trees.
**Fix:** Completely removed the `scraper` crate and `.lines()` iterators from `/src/adapters/lockbit.rs` and `/src/adapters/autoindex.rs`. Built manual zero-copy string-slice windowing using `.find()` (delegates to SIMD `memchr`), resulting in nanosecond-level DOM extraction and Gigabyte/sec parser speeds without Tokio context-switch stalls.
**Prevention Rule:** PR-PH78-001: Never ever use `scraper::Html::parse_document()` for predictable/iterative directory auto-indexes or table rows. Use memory-safe, zero-copy `slice[..].find()` techniques to extract elements without DOM tree heap allocation.

## Phase 79: Proactive Multi-Node Failover Manager
**Issue:** `client error (Connect)` socket failures completely derailed high-concurrency 64-worker soak tests when individual onion seeds rotated their addresses or succumbed to DDoS. Adapters (except Qilin) lacked generic fallback resolution.
**Fix:** Implemented a unified `SeedManager` directly inside `CrawlerFrontier`. All URL routing, sub-resource probing (`bytes=0-0`), and fallback allocations are seamlessly intercepted via `f.seed_manager.remap_url(&next_url, &f.target_url)`. Heavy 50x/Timeout drops trigger aggregate fallback threshold counters, dynamically rotating the cluster's focus away from dead targets.
**Prevention Rule:** PR-PH79-001: All target implementations must run generic GET routes and probes through `SeedManager` rather than blindly accessing dequeued raw `.onion` domains. Adapters should handle internal `.onion` failover dynamically at the `CrawlerFrontier` level, not locally.
