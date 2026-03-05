> **Last Updated:** 2026-03-04T15:08 CST

Version: 1.0.9
Updated: 2026-03-04
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
**21. Windows child processes in GUI runtime must set no-window creation flags unless user-visible console output is explicitly required.**
**22. Temp-directory cleanup logic must use platform APIs (`std::env::temp_dir`) instead of OS-specific literals.**
**23. Batch progress telemetry must use one global denominator and include smart-skip/large/small phases uniformly.**
**24. Any adapter present in support catalog must also be explicitly registered in `AdapterRegistry`, with a matching engine test assertion.**
**25. Direct I/O acceleration must be policy-driven and degradable to buffered writes on unsupported targets.**
**26. Tournament sizing should be data-informed from prior bootstrap telemetry, not fixed constants only.**
**27. Mixed-size batch schedulers must include starvation prevention when shortest-job strategies are enabled.**
**28. Repository-level quality gates must be enforced in CI, not left to local developer defaults.**

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
- 2026-03-03: Suppressed Windows scan-time command prompt popups with no-window process flags; normalized temp-dir cleanup for Windows.
- 2026-03-03: Unified batch progress counters/bytes across phases and moved frontier WAL path to platform temp directory.
- 2026-03-03: Re-registered LockBit adapter in runtime registry and fixed `engine_test` `CrawlOptions` fixtures for `daemons` field parity.
- 2026-03-04: Added adaptive Direct I/O fallback policy, adaptive tournament telemetry/sizing, SRPT+aging batch scheduling controls, and strict quality workflow/toolchain pinning.

# Appendices
- Validation:
  - `cargo check`
  - `cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check`
  - `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings`
  - `cargo test --lib`
  - `cargo test --test engine_test`
  - `npm run build`
  - `npm run overlay:integrity`
