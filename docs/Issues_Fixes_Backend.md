Version: 1.0.5
Updated: 2026-03-03
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
  - Fix: route non-crawler support artifacts into `<selected_output>/temp_onionforge_forger`.
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
- Issue: Re-crawling an existing directory tree overwrites 100% completed files and wastes bandwidth.
  - Root Cause: `start_batch_download` queued every file unconditionally. Partial-resume (`.ariaforge_state`) only prevents data loss, not redundant starts.
  - Fix: implement "Smart Skip" in the pre-flight routine using local filesystem metadata and the `size_hint` from the crawler.
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

# Prevention Rules
**1. Every crawl queue counter increment must have a guaranteed paired decrement.**
**2. Use URL parser APIs for any recursion edge construction.**
**3. Any shared parser contract update requires immediate adapter compatibility sweep.**
**4. Concurrency policy changes must include test expectation updates.**
**5. Cancellation must remain forceful across crawl, download, and Tor resources.**
**6. Scheduler loops must not block queue intake behind single long-running workers.**
**7. Onion listing size mapping should prefer parsed listing data over per-entry HEAD probes.**
**8. Batch routing must consume crawler size hints before issuing network probes.**
**9. Non-crawler support artifacts must stay isolated under `temp_onionforge_forger` in the selected output root.**
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

# Risk
- Aggressive worker startup may increase transient connection churn on weak targets.
- Scope checks may hide intentionally cross-root links; this is acceptable for safety and determinism.

# History
- 2026-03-03: Initial backend issue/fix baseline.
- 2026-03-03: Added cumulative `downloaded_bytes` in batch progress telemetry.
- 2026-03-03: Synced LockBit/Nu support catalog entries with real full-crawl behavior.
- 2026-03-03: Expanded Tor active-port discovery to full managed range and reused tournament winners in downloader flows.
- 2026-03-03: Hardened small-file retry logic (rotation, timeout/backoff tuning, strict completion validation).
- 2026-03-03: Added batch heartbeat telemetry to prevent frozen UI states during long-tail phases.
- 2026-03-03: Fixed Linux release pipeline stability by removing AppImage from default CI bundle targets.

# Appendices
- Validation:
  - `cargo check`
  - `cargo test --lib`
  - `cargo test --test engine_test`
  - `cargo run --example lockbit_live_pipeline` (live onion run, completed 379/379)
