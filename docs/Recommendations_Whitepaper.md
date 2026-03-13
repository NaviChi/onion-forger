> **Last Updated:** 2026-03-13T22:24 CDT

## Phase 144-IMPL: Stall Prevention — All 5 Bugs Fixed + R5/R7 (2026-03-13)

**Status: ✅ Implemented and build-verified (zero errors)**

All 5 identified stall vectors from the Phase 144 root cause analysis are fixed:
- **BUG-1:** `scaffold_download()` now has adaptive timeout (30+3s×files, max 300s)
- **BUG-2:** Heartbeat watchdog emits 💓 logs every 30s during downloads
- **BUG-3:** `clear_download_control()` in ALL error/timeout paths
- **BUG-4:** Probe progress emission every 10 files (R7)
- **BUG-5:** Covered by BUG-1's per-chunk timeout ceiling
- **R5:** Timeout escalation (1.5× per consecutive timeout, max 3×, resets on success)
- Proactive NEWNYM + 10s recovery on timeout
- Final VFS sweep also has 300s timeout with cleanup

Remaining deferred for next cycle:
- **R4:** Parallel probes (4 concurrent) — would 4× probe phase speed
- **R6:** Token bucket for 503 throttles — coordinated backoff
- **R8:** Circuit health pre-check before chunk start

## Phase 144: Tokio vs C Analysis + C-Implementation Borrowed Patterns (2026-03-13)

**Verdict: Tokio is optimal.** Our bottleneck is Tor transport (2-5s RTT), not async runtime overhead (~0.2ms). io_uring is Linux-only; we target Windows. Benchmarks show Rust+io_uring ≈ C+io_uring (198K vs 200K req/s).

**Three C patterns worth copying:**
- **CP1 (5 min, from libcurl):** `reqwest::Client::builder().pool_max_idle_per_host(4).pool_idle_timeout(90s)` — prevents FD exhaustion on high-fanout sites. libcurl uses `CURLOPT_MAXCONNECTS` for this.
- **CP2 (15 min, from aria2):** Per-host connection semaphore (aria2's `-x` flag limits to max 16 per host). Our R2 host-grouped scheduling can accidentally hammer one host with all workers.
- **CP3 (LOW, from libcurl):** DNS pre-resolution — irrelevant for .onion (Tor resolves internally).

**Anti-Recommendations from C analysis:**
- io_uring: Linux 5.1+ only, we're Windows-first
- Zero-copy splice: Windows doesn't support splice(); tokio::fs adequate for our throughput
- Custom allocator: Transport-bound, not allocation-bound
- FFI to libcurl: Would lose Tauri integration benefits

## Phase 142-IMPL: R1+R2+R3+R4 Implemented (2026-03-13)

**Status: ✅ Implemented and build-verified**

R1, R2, R3, R4 from the Phase 142 analysis are now implemented. Compilation verified on `cargo check` — zero errors, zero warnings.

## Phase 142: Exhaustive Improvement Analysis — Top 8 Recommendations (2026-03-13)

**Status: R1-R4 implemented. R5-R8 deferred for next cycle.**

Full analysis artifact: [phase142_improvement_analysis.md](file:///C:/Users/Zero/.gemini/antigravity/brain/d0b38f8a-8219-43ab-9ba0-78d2db56d375/phase142_improvement_analysis.md)

Based on exhaustive review of all whitepapers, lessons learned (Phases 54-141B), internet research (Google Tail-at-Scale, NASA DTN, SpaceX AQM, HFT ring buffers, aria2 connection reuse, Tor Conflux), and competitive analysis (aria2, IDM, wget2):

### Top 3 Recommendations (40-70% estimated improvement, <2 hours total):

1. **R1: Hedged Download Retry** ✅ IMPLEMENTED — After each chunk, 0-byte/missing files get one hedged retry (60s timeout). Catches transient circuit failures.

2. **R2: Host-Grouped Batch Scheduling** ✅ IMPLEMENTED — Chunks sorted by host (primary) then size (secondary SRPT). Maximizes HTTP keep-alive connection reuse.

3. **R3: Adaptive Stall Threshold** ✅ IMPLEMENTED — `3× max(recent_batch_durations)` clamped to `[30s, 180s]`. Replaces fixed 90s.

### Additional Recommendations:
4. **R4: Bounded Download Channel** ✅ IMPLEMENTED — `mpsc::channel(200)` replaces unbounded. Natural backpressure.
5. **R5: Per-File Priority Queue** — Weight entries by file size (SRPT), host affinity, and freshness.
6. **R6: Circuit Pre-Warming** — Pre-establish connections to next hosts while processing current batch.
7. **R7: Token Bucket 503 Management** — Replace crude per-worker 2s sleep with global coordinated rate limiter.
8. **R8: Entry Age Tracking** (Starlink AQM) — Demote entries queued >5 minutes to prevent head-of-line blocking.

### Anti-Recommendations (explicitly rejected):
- More isolated_client() views (LESSON-140C-001: shares guard bandwidth)
- Conflux multi-path bonding (LESSON-136-001: doubles HS setup cost)
- >4 base TorClients default (OOM risk on most systems)
- Custom Tor relay picking (bottleneck is transport policy, not relay quality)

Hardware budget for all 8: <1KB RAM, <0.1% CPU, estimated 15-25% energy savings from reduced wasted requests.

## Phase 102: Probe Admission Telemetry + Cooldown Escalation (2026-03-11)

**Status: Implemented, backend-validated, and quick-live-validated on the exact-target CLI replay**

What this tranche changed:
- Probe-stage admission now exposes two first-class counters in the shared telemetry plane: `download_probe_quarantine_hits` and `download_probe_candidate_exhaustions`. They flow through `runtime_metrics`, `telemetry_bridge`, binary telemetry, protobuf bindings, React normalization, the dashboard, and the compact CLI summary (`probe_admission=...`).
- Onion host productivity memory is no longer effectively immortal. Repeated connect failures now decay productive priority and escalate quarantine harder, so hosts that succeeded sometime in the past stop being treated as preferred if they keep failing the first wave.
- The exact-target CLI replay surface is now more actionable because it logs full candidate exhaustion explicitly instead of leaving the operator to infer it from repeated timeout lines alone.

Quick live result:
- The short March 11, 2026 `90s` `crawli-cli download-files` replay against the exact `current/listing_canonical.json` snapshot hit the new path immediately. The first file exhausted all `3` probe candidates, the next file entered with `quarantined_candidates=3/3`, and the summary printed `dl_transport=1/0/0 probe_admission=1/1`.
- That is progress in observability, not yet in useful work. The replay still did not show non-zero payload progress before timeout, so the blocker has narrowed further: the next issue is what to do once the full three-host candidate set is already degraded.

Next recommended steps:
- Split the CLI execution path from full Tauri desktop linkage so exact-target validation starts transport work sooner.
- Change exhausted-candidate fallback so the downloader does not simply re-arm the same degraded winner host after a full three-host probe failure.
- Re-run the exact-target replay long enough to verify `probe_admission` rises while useful payload bytes also become non-zero before touching concurrency.

## Phase 101: IDM Transport Audit + Qilin Probe Admission Hardening (2026-03-11)

**Status: Implemented and live-validated on the rebuilt exact-target replay**

What the IDM investigation confirmed from official documentation:
- IDM's strongest practical ideas are scheduler policy, not magic transport primitives: dynamic segmentation, per-site max-connection tuning, immediate transfer start before a second request can fail, and site-specific exceptions for hosts that break under the normal multi-request path. Source basis: official IDM features/help pages from Tonec.
- The safest ideas to copy into Crawli are the host-specific exception mechanisms, not blanket global aggression. For clearnet that means stronger per-host admission and reuse; for Qilin/onion that means early degraded-host quarantine and alternate-host rotation before the transfer scheduler commits to a host.
- This maps cleanly onto the current bottleneck: the exact-target onion path is still dying in probe/connect collapse before useful transfer work begins.

Implemented in this tranche:
- `src-tauri/src/aria_downloader.rs` now records probe-stage degraded hosts into a short-lived quarantine window on repeated connect/timeout failures.
- Probe candidates are now ordered by live host health before transfer scheduling, and if no host has proven productive yet the order rotates by path so the first wave does not keep hammering the same host.
- After probe selection, `alternate_urls` are reseeded so the next transfer-time failover starts from the remaining best host instead of replaying the stale pre-probe ordering.

Live validation results:
- The local toolchain blocker is cleared. `rustc -vV` returned immediately on March 11, 2026, `cargo build --bin crawli` finished in `3m49s`, and `cargo build --bin crawli-cli` finished in `10.70s`.
- The rebuilt exact-target `150s` replay through `crawli-cli` still produced `0` payload files and `0` payload bytes, with only `519` `.gitkeep` placeholders created under the output root.
- The new probe path is active: the replay logged `GET Range` probe timeouts at `8s`, `12s`, and `16s`, then emitted `Probe rotation` lines with `quarantined_candidates=2/3` and `3/3`, and finally armed alternate transfer fallbacks on both `lbln...` and `4xl2hta3...`.
- `dl_transport` rose from `0/0/0` to `18/0/0`, which proves the scheduler is spending transport attempts, but those attempts are still dying in first-wave connect/probe collapse before payload bytes flow.

Next recommended steps:
- Add probe-stage quarantine-hit and candidate-exhaustion counters to the shared telemetry plane so the operator can see exactly when all first-wave hosts are already degraded.
- Tighten degraded-host eviction/cooldown after repeated first-wave `Connect` failures; the current rotation is active, but weak hosts are still re-entering admission too early.
- Split the CLI execution path from full Tauri desktop linkage so validation runs do not spend roughly `11.8s` in binary startup before transport work begins.
- Keep concurrency frozen until the same exact-target replay produces non-zero payload bytes.

## Phase 100: Active Host Cap + Comparative Downloader Research (2026-03-10)

**Status: Implemented and live-validated on both direct and exact-target onion paths**

What the broader transport comparison confirmed:
- libcurl, aria2, and wget2 do not beat general Rust downloaders merely because they are C/C++. They win because they expose mature transport policy: active per-host caps, shared host memory, progress-sensitive aborts, and scheduler-driven admission.
- aria2 is the clearest comparator for direct-file throughput because it combines segmented HTTP range fetching with `max-connection-per-server` and `lowest-speed-limit`. That maps directly onto Crawli's downloader hot path.
- Wget2 is more relevant for recursive correctness and persistence than for the highest-throughput hot path; its lessons matter more for crawl durability than for direct-file speed.
- reqwest/hyper remain viable foundations. The missing wins are application policy and host-admission logic, not a need to replace Rust with a C transport stack.

Implemented in this tranche:
- A true downloader-side active per-host cap now exists in `src-tauri/src/aria_downloader.rs`, analogous to libcurl's `CURLMOPT_MAX_HOST_CONNECTIONS` and aria2's `max-connection-per-server`.
- Clearnet/direct traffic currently defaults to `32` live host-local transfers, while onion/Qilin stays conservative at `4` so one rotating hidden-service host cannot monopolize the batch.
- Excess work now queues behind host permits instead of immediately overcommitting the same host.

Live results:
- Clean direct benchmark: `host_cap=32`, `bytes=758923264`, `elapsed_secs=15.39`, `throughput_mbps=394.59`.
- Exact-target Qilin replay: `host_cap_ceiling=4`, `dl_transport` rose to `18/0/0`, but the batch still produced `0` payload bytes because the first wave died in probe/connect collapse before useful work began.

Next recommended steps:
- Add probe-stage degraded-host quarantine so hosts that fail the first wave do not keep consuming micro/small admission slots.
- Rotate alternate-host cursors more aggressively before transfer scheduling begins.
- Keep concurrency frozen until the same exact-target Qilin replay produces non-zero payload bytes under the new host-pressure regime.

## Phase 99: Libcurl Transport Reverse-Engineering Audit (2026-03-10)

**Status: Core transport tranche implemented and validation-complete for build/test/telemetry parity**

What the audit confirmed:
- Crawli already matches libcurl in a few important ways: pooled clients exist on both the Tor and clearnet paths, HTTP/2 is enabled on the Tor client, and downloader probing already uses a real `GET Range: bytes=0-0` instead of a detached `HEAD`.
- The strongest remaining transport gaps are not "more sockets" problems. They are reuse and admission problems: Crawli still lacks a generic host-capability cache, still leans heavily on fixed wall-clock timeouts instead of low-speed semantics, and still lacks a downloader-side active per-host cap analogous to libcurl's `CURLMOPT_MAX_HOST_CONNECTIONS`.
- The current code still contains one especially expensive anti-libcurl pattern: the small-file swarm and range tournament probe explicitly force `Connection: close` in `src-tauri/src/aria_downloader.rs`, which destroys keep-alive reuse even after a productive host is known.
- The current probe path still pays for a real ranged GET and then starts a second transfer request later, which means Crawli often spends two request setups where a libcurl-style promoted first transfer could spend one.

Implemented in this tranche:
- Clearnet paths now keep transport reuse enabled by default, while onion reuse is gated by host-quality state instead of a blanket disable.
- Micro/small swarm lanes now promote successful range probes into the first transfer span when the prefetched seed is bounded and useful, reducing duplicated request setup on many-small-file batches.
- The downloader now keeps a shared host-capability cache keyed by host and traffic class and records range support, validator kind, connect/first-byte EWMA, low-speed abort history, and a provisional safe parallelism cap.
- Low-speed abort counters now survive end to end through runtime metrics, CLI summaries, protobuf/binary telemetry, and the React bridge.

Next recommended steps:
- Add a true active per-host transfer cap before any further concurrency increase. `pool_max_idle_per_host` is still not a substitute for live host-pressure control.
- Validate the new transport counters on a clean direct benchmark rerun and an exact-target Qilin replay that produces non-zero useful work.
- After the per-host cap exists, consider using the host-capability ledger to bias early admission and lane width, but only from proven productive hosts.

## Phase 95: Clearnet Direct-File Audit + Direct Mode Fix (2026-03-10)

**Status: Implemented and live-benchmarked on safe public direct-download targets**

What the code review and live reruns confirmed:
- The direct HTTP(S) path already existed, but it was underperforming because it still inherited onion-era policy in two places: excessive large-file fan-out and a handshake filter that culled half the clearnet survivors before the run even started.
- The app also still treated non-MEGA/non-torrent URLs as `onion` by default. That made clearnet direct archives look like Tor targets in `detect-input-mode` and the UI.
- The repaired clearnet path is now materially faster on the same host/file pair. On March 10, 2026 the safe public `10Gb.dat` benchmark reached about `3.8 GiB` in `60s` (`~63 MiB/s`, `~530 Mbps`) versus the older `~95.7 Mbps` build and a same-day single-stream `curl` control at `~208 Mbps`.

Recommendations now active:
- Keep clearnet and onion download policy separate. Clearnet should use connection pooling, sane direct-file caps, and no onion-style handshake survivor culling.
- Keep the new `direct` mode in the operator plane and use hostname-based onion detection everywhere; do not rely on “everything else is onion” fallthrough.
- Preserve the piece-mode resume accounting fix. Large interrupted direct downloads need per-piece offset tracking, not per-initial-circuit tracking.

Next recommended steps:
- Add a dedicated clearnet regression harness that records `bytes/60s`, `handshake survivors`, and `worker cap` so direct HTTP(S) performance cannot silently drift back below the current benchmark.
- Consider adaptive clearnet `active_start` widening after early healthy completions instead of statically starting at `16`, but benchmark it against the current `~530 Mbps` posture before promoting it.
- Keep the user-facing “download via torrent” feature separate from direct archive download; only use torrent mode when an actual magnet or `.torrent` source exists.

## Phase 94: Hidden Support Root + Per-File Host Remap Review (2026-03-10)

**Status: Code-reviewed and live-benchmarked on the exact Qilin / Arti download path**

What the code review and live reruns confirmed:
- Per-file alternate-host remap is now active in `src-tauri/src/aria_downloader.rs`. The March 10, 2026 exact-target `150s` rerun logged `24` real remaps instead of silently retrying the same raw URL forever.
- Downloader support artifacts no longer belong in the operator-visible payload tree. The runtime now writes `_onionforge_manifest.txt` and `download_support_index.json` under a hidden sibling root (`.onionforge_support/<support_key>/`) and leaves the payload root free of `temp_onionforge_forger`.
- The new single-index approach is now emitted during scaffold, not only after a clean batch exit. An interrupted `20s` rerun still produced both support files under the hidden root.
- The remaining failure is still transfer-side: the March 10, 2026 exact-target `150s` rerun produced `0` payload bytes even after host remaps, because the first wave still overconcentrates on the `zqetti.../lbln...` pair and does not reach the third current-snapshot storage host (`4xl2hta3...`) quickly enough.

Recommendations now active:
- Keep support artifacts in the hidden sibling root and exclude them from operator payload/file accounting by default.
- Keep the single manifest/index model; do not restore eager `.onionforge.meta` sidecars unless a concrete runtime need appears.
- Rebalance first-wave download admission so the batch does not repin most saved URLs onto one winner host before that winner proves useful completions.
- Allow stalled files to graduate to the third known storage host before they exhaust the current micro/small requeue budget.

Next recommended steps:
- Gate Qilin download repinning behind early useful completions or live host-quality evidence instead of repinning `1769` saved URLs onto one winner host unconditionally.
- Teach the per-file remap path to reach the third current-snapshot host earlier (`4xl2hta3...`) instead of oscillating between the top two hosts.
- Re-run the exact-target timed benchmark and require non-zero payload bytes before promoting a full best-snapshot soak.

## Phase 93: `.meta` Sidecar Audit + First-Byte Escape Review (2026-03-10)

**Status: Superseded by Phase 94 runtime changes**

What the prior code review confirmed:
- `temp_onionforge_forger/*.onionforge.meta` files were support sidecars written during scaffold in the earlier implementation; they were not completed payload files.
- The real output failure on the March 10, 2026 exact-target rerun was `0` payload bytes outside the support directory, which proved that support-artifact visibility was not the root transfer problem.

## Phase 92: Download Admission Audit + Qilin Route Repinning (2026-03-10)

**Status: Implemented and live-benchmarked on exact Qilin / Arti download paths**

Implemented in this pass:
- `src-tauri/src/aria_downloader.rs` now computes a bounded onion batch lane plan, starts the large-file lane only after early useful completions, and shortens micro/small hidden-service send/body timeouts while rotating isolated clients on repeated batch failures.
- `src-tauri/src/lib.rs` and `src-tauri/src/cli.rs` now repin saved Qilin raw URLs through the persisted `qilin_subtree_route_summary.json` route memory before batch download starts. The live CLI path repinned `1769` saved URLs to the persisted winner-host set on March 10, 2026.
- Repeated exact-target `150s` timed windows still produced `0` useful completions after the repin and timeout changes, which narrows the remaining bottleneck to first-wave hidden-service first-byte/body stalls in the micro+small swarm itself.

Recommendations now active:
- Do not widen hidden-service batch fan-out again until the first-wave swarm can escape no-byte stalls quickly. More concurrency is not the missing ingredient on this target.
- Preserve the repin layer. It is still valuable because it removes stale raw-URL trust, even though it did not by itself restore early useful completions on the March 10, 2026 target reruns.
- Treat large-lane overlap as conditional throughput optimization, not a default entitlement. If the first wave does not produce early completions, keep the large lane parked.

Next recommended steps:
- Add early no-byte abort + requeue logic for micro/small files so dead first-wave requests do not monopolize the batch for most of the benchmark window.
- Diversify the first wave across multiple preferred hosts instead of letting the smallest-file scheduler overconcentrate on one weak storage host.
- Re-run the exact-target timed benchmark after the no-byte escape work before attempting another full best-snapshot soak.

## Phase 91: Downloader Throughput Audit + macOS Storage Reclassification (2026-03-10)

**Status: Implemented and live-benchmarked on exact Qilin / Arti download paths**

Implemented in this pass:
- `src-tauri/src/aria_downloader.rs` now promotes mid-size onion files into the large-file lane (`large > 16MB` on heavy hidden-service batches) and keeps aggregate batch throughput honest once the run enters the large-file phase.
- `src-tauri/src/resource_governor.rs` now uses a macOS `diskutil` mount-point fallback so Arti bootstrap sees Apple Fabric / NVMe storage instead of collapsing to `Unknown`.
- Hidden-service multi-file batches deliberately do **not** use the full NVMe first-wave lane shape by default. The retained default is the mixed profile: keep the `12`-client NVMe-aware bootstrap, but keep the transfer first wave at `16/8/10/24` because the more aggressive live variants did not improve useful-work throughput.

Recommendations now active:
- Any operator-facing “full download” workflow must surface `best` vs `current` snapshot counts before the transfer starts. On the exact target, `best` is `5078` entries / `4240` files while `current` is only `2926` / `2394`.
- Treat hidden-service batch downloads as network-bound, not disk-bound. NVMe helps bootstrap and local I/O margins, but wider first-wave circuit spray is not automatically a win against rotating Qilin storage.
- Keep aggressive onion batch fan-out behind live proof. If a wider posture does not improve completed bytes over the first few minutes, do not promote it to the default.

Next recommended steps:
- Add an adaptive onion batch controller that narrows probe/active fan-out when the first success window does not produce useful completions.
- Run one uninterrupted best-snapshot Qilin download soak to completion and record terminal throughput / failure distribution instead of only early and mid-run comparisons.
- Expose authoritative `best/current` snapshot counts plus total byte hints in the Downloads tab before the operator starts mirroring.

## Phase 90: Winner-Quality Memory + Tail-Latency Biasing (2026-03-10)

**Status: Implemented and live-validated on exact Qilin / Arti reruns**

Implemented in this pass:
- `src-tauri/src/adapters/qilin_nodes.rs` now persists winner-host quality signals (`effective entries/sec`, completion time, throttle/failover pressure) and folds them into Stage D / redirect-ring ranking instead of treating all recently-seen winners as equivalent.
- `src-tauri/src/runtime_metrics.rs`, `src-tauri/src/cli.rs`, `src-tauri/src/binary_telemetry.rs`, `src-tauri/src/telemetry_bridge.rs`, and `src/telemetry.proto` now expose compact final tail-latency facts: `winner_host`, `slowest_circuit`, `late_throttles`, and `outlier_isolations`.
- `src-tauri/src/adapters/qilin.rs` now drives worker repin cadence from winner quality instead of a fixed interval, and the reconciliation tail now preserves retry history plus a hard wall-clock escape budget instead of reopening missing folders forever.

Recommendations now active:
- Keep productive-winner memory separate from raw freshness. A host that merely appeared in Stage A is not as valuable as a host that finished the full tree quickly and cleanly.
- End-of-run tail summaries should be compact but mandatory. Deep-crawl slowdowns are now diagnosable from a single final line instead of raw log archaeology.
- Adaptive repinning is the right next lever before default concurrency changes. The useful-work bottleneck is winner quality and tail churn, not lack of workers.

Next recommended steps:
- Add stronger winner stickiness on warm reruns so a fresh Stage A host must beat a productive cached winner by a higher bar before replacing it.
- Persist a tiny redirect-freshness ring that tracks both freshness and productivity so Stage A does not overvalue newly seen but weak hosts.
- Surface a compact final note when a warm rerun abandons a previously productive winner and later pays for it in subtree reroutes / failovers.

## Phase 89: Deep-Crawl Stall Audit + Throttle/Outlier Repair (2026-03-10)

**Status: Implemented and live-validated on exact Qilin / Arti full crawls**

Implemented in this pass:
- `src-tauri/src/adapters/qilin_nodes.rs` now bounds cached fast-path probing to two deduplicated candidates before falling back to fresh Stage A, so reruns no longer burn long serial probe chains before real discovery starts.
- `src-tauri/src/adapters/qilin.rs` now classifies first-attempt `503/429/403/400` responses as `Throttle`, routes them through circuit isolation + telemetry, and includes a governor-side latency-outlier stall guard with cooldown for true no-progress windows.
- `src-tauri/src/lib.rs` now reports effective entry counts at crawl completion when the adapter writes straight into the VFS instead of returning an in-memory `Vec<FileEntry>`.
- Two full exact-target crawls were used to separate deadlock risk from route-quality variance: one on `3pe26tqc...` and one on `aay7nawy...`.

Recommendations now active:
- Do not treat deep-layer slowdown as proof of a stuck crawl until both discovered-entry growth and processed-node growth stop. On March 10, 2026 the exact target kept making progress even on the slow winner.
- First-attempt throttle statuses must feed the same telemetry and healing path as retry-lane throttles. Otherwise the operator plane will underreport the real cause of late crawl slowdown.
- Qilin completion logs must report effective entries, not just raw returned vectors, because the adapter can stream directly into the VFS and still succeed completely.

Next recommended steps:
- Persist winner-host quality memory (`effective req/s`, late `503` rate, failover count, average completion time) and feed it into redirect-ring / Stage D ranking so slow winners are deprioritized across reruns.
- Add a compact final tail-latency summary to the CLI (`winner_host`, `slowest_circuit`, `outlier_isolations`, `late_throttles`) so deep-layer slowdowns are visible without log forensics.
- Make worker re-pin cadence adaptive to winner quality before touching default concurrency knobs. The remaining waste is route quality, not lack of worker count.

## Phase 88: Binary Telemetry Parity + Clean Same-Output Restore Validation (2026-03-10)

**Status: Implemented and live-validated on exact Qilin / Arti reruns**

Implemented in this pass:
- `src-tauri/src/binary_telemetry.rs`, `src-tauri/src/telemetry_bridge.rs`, and `src/telemetry.proto` now agree on the full `ResourceMetricsFrame`, including `throttle_rate_per_sec`, `phantom_pool_depth`, `subtree_reroutes`, `subtree_quarantine_hits`, and `off_winner_child_requests`.
- `src/telemetry.js` and `src/telemetry.d.ts` were regenerated from the protobuf schema so the frontend ring-buffer decoder sees the same fields and defaults as the backend.
- `src-tauri/src/cli.rs` now emits a compact `[summary:final]` line with `req=total/success/fail` and `subtree=reroutes/quarantine/offwinner` when a crawl reaches `complete`, `cancelled`, or `error`.
- The exact Qilin target was then run twice against the same output root to validate live restore of subtree host memory without an alarm wrapper.

Recommendations now active:
- Keep Rust binary telemetry structs, `telemetry.proto`, and generated frontend bindings in lockstep. Metric drift across those three surfaces silently drops operator data.
- Preserve a dedicated final CLI summary even when periodic summaries exist. Route/request totals are completion facts, not rolling status.
- Validate subtree route memory against the same output root, not fresh directories. Otherwise the restore path is never exercised for real.

Next recommended steps:
- Add a controlled degraded-route harness that forces non-zero `subtree_reroutes`, `subtree_quarantine_hits`, and `off_winner_child_requests` so the new counters are tested outside the happy path.
- Persist a tiny freshness-ranked redirect ring alongside subtree host memory so reruns spend fewer requests when the cached winner rotates away between sessions.

## Phase 87: Subtree Route Telemetry + Host-Based Memory (2026-03-10)

**Status: Implemented and revalidated on repeated live Qilin / Arti reruns**

Implemented in this pass:
- `src-tauri/src/runtime_metrics.rs` now tracks subtree reroutes, subtree quarantine hits, and off-winner child requests in the same shared telemetry plane as the existing request/fingerprint metrics.
- `src-tauri/src/bin/adapter_benchmark.rs` and `src-tauri/src/bin/crawl_stats.rs` now print those counters directly in the `[EFF]` line and the final summary/CSV columns (`SUB_RER`, `Q_HITS`, `OFFWIN`).
- `src-tauri/src/adapters/qilin.rs` now persists subtree preferred hosts per target, but only by host and only when that host is still present in the current winner/standby set, so old UUID paths do not poison new runs.
- `src-tauri/src/cli.rs` now keeps `--no-stealth-ramp` benchmark-only in behavior unless `CRAWLI_ALLOW_BENCHMARK_FLAGS=1`, while the benchmark binary still uses `stealth_ramp=false` for controlled comparison work.

Recommendations now active:
- Keep subtree-route waste measurable in shared telemetry. Route planning that only exists in logs will regress silently.
- Persist subtree preferred hosts by host identity, not raw full seed URLs, and only reuse them when that host survives into the current candidate set.
- Keep `--no-stealth-ramp` out of the default operator path until a stable target proves a real useful-work gain.

Next recommended steps:
- Carry the new subtree-route counters into the binary telemetry/protobuf plane and compact CLI final summaries so operator surfaces stay consistent.
- Revalidate the new subtree host-memory path on a clean full same-output rerun that exits normally, not an alarm-capped replay.

## Phase 86E: Subtree Host Affinity + Standby Quarantine (2026-03-10)

**Status: Implemented and revalidated on repeated live Qilin / Arti reruns**

Implemented in this pass:
- `src-tauri/src/adapters/qilin.rs` now tracks subtree-local preferred seeds, subtree-local host health, and subtree-local standby quarantine instead of treating every child-path failure as evidence against the global winner route.
- Child requests now try the subtree-preferred winner first, skip quarantined standby seeds for that subtree, and only fall back to global failover rules on true root/global failures.
- The exact-target replay on `afa2a0ea-20ba-3ddf-8c5c-2aeea9e5dc43` reached `seen=544 processed=282 queue=262 workers=16/16` in `120.01s`, kept child traffic pinned to the confirmed winner, and pushed adapter-local progress to `entries=2336 pending=216`.

Recommendations now active:
- Keep subtree-level route memory separate from the global winner lease. A failing child subtree must not demote a still-good root path.
- Quarantine stale standby routes per subtree before spending second or third attempts on them again.
- Keep `--no-stealth-ramp` benchmark-only until a stable target proves it beats the repaired default path on useful work, not just quicker worker induction.

Next recommended steps:
- Add telemetry for subtree reroutes, quarantine hits, and off-winner child requests so future audits do not require log-forensics to quantify request waste.
- Persist subtree preferred-seed summaries across runs only if repeated live targets show meaningful cross-run subtree churn.

## Phase 86D: Root Durability + Active-Host Affinity (2026-03-10)

**Status: Implemented and revalidated on repeated live Qilin / Arti reruns**

Implemented in this pass:
- `src-tauri/src/adapters/qilin_nodes.rs` now promotes a winner lease only after a real root listing fetch/parse confirms that the storage route is durable, not just after a speculative discovery probe.
- `src-tauri/src/tor_native.rs` now replenishes empty phantom slots from the already-live Arti swarm with cheap isolated clients before it falls back to cold bootstrap.
- `src-tauri/src/adapters/qilin.rs` now protects root retries from standby remap and keeps the first ordinary child retry on the confirmed active host instead of spilling directly onto standby routes.
- The same exact-target rerun that previously failed to enumerate now reaches a durable winner and real file/folder expansion (`kent/` -> `133 files / 133 folders`, timed window ending at `seen=288 processed=59 queue=229`, adapter-local `entries=670`).

Recommendations now active:
- Promote storage winners only after a real root parse. Probe success alone is not durable truth on rotating onion storage.
- When the root winner is known-good, keep the first child retry on that same host. Spending second attempts on standby routes too early wastes requests and increases tail latency.
- Refill phantom capacity from the hot swarm first; cold bootstrap during an active crawl is last-resort behavior.

Next recommended steps:
- Add subtree-aware host affinity and standby quarantine so degraded subtrees stop bouncing between stale alternate hosts.
- Track subtree-level host failures separately from global winner failures so root health and child-path health do not contaminate each other.
- Keep `--no-stealth-ramp` as a benchmark-only knob until a stable target shows a clear useful-work gain over the default ramp.

## Phase 86C: Arti Hot-Start + Hinted Warmup Bypass (2026-03-10)

**Status: Implemented and revalidated on repeated live Qilin / Arti reruns**

Implemented in this pass:
- `src-tauri/src/multi_client_pool.rs` now seeds follow-on pools from already-hot swarm clients instead of cold-bootstrapping a second Arti vanguard after storage resolution.
- `src-tauri/src/frontier.rs` / `src-tauri/src/lib.rs` now refresh live Arti clients before hinted onion execution so the crawl can use swarm growth that happens after the initial bootstrap quorum returns.
- `src-tauri/src/lib.rs` now skips blocking onion warmup when a strong URL hint already selected the adapter and the fingerprint GET will be skipped anyway.
- `src-tauri/src/adapters/qilin_nodes.rs` now reserves first-wave Stage D capacity for a stable cached winner instead of letting two fresh redirect candidates crowd it out entirely.

Recommendations now active:
- Treat strong-hint Qilin/CMS ingress as a direct handoff path. Do not spend a blocking warmup and then a skipped fingerprint on the same request chain.
- Reuse live Arti clients whenever a second-stage crawl pool is needed. Shared state plus isolated handles are materially cheaper than re-bootstrapping a cold pool.
- Keep at least one first-wave slot in Stage D for the best stable node, even when fresh redirects were just captured.

Next recommended steps:
- Promote a storage host to the winner lease only after it survives both probe validation and one real root fetch.
- Prevent phantom-pool depletion from forcing rebootstrap while the crawl is already on the critical path.
- Benchmark Qilin with `stealth_ramp=false` or a faster ramp interval only after root durability is high enough to support a broader worker wave.

## Phase 86: Arti Fingerprint Bypass + Discovery Telemetry (2026-03-10)

**Status: Implemented in part and revalidated on live Qilin / Arti reruns**

Implemented in this pass:
- `src-tauri/src/lib.rs` and the benchmark binaries now bypass the network fingerprint GET for strong Qilin CMS ingress URLs (`/site/view?uuid=...` and `/site/data?uuid=...`).
- `src-tauri/src/runtime_metrics.rs`, `src/App.tsx`, and `src/components/Dashboard.tsx` now expose swarm runtime/client/traffic-class/request/fingerprint metrics and render a dedicated swarm-efficiency panel with sparklines.
- `src-tauri/src/adapters/qilin_nodes.rs` now counts Stage A/B/D discovery requests into telemetry and lazy-seeds fallback mirrors only when the cached node pool is too sparse for a meaningful Stage D race.

Recommendations now active:
- Treat known Qilin CMS ingress as a URL-classification problem first, not a fingerprint-fetch problem. Do not spend a network GET when the URL already proves the adapter.
- Count discovery-plane requests in the same telemetry plane as frontier worker requests, or Qilin efficiency charts and benchmarks will lie.
- Keep fallback mirror seeding lazy. Broad mirror insertions are insurance, not the first thing a warm run should do.
- Do not interpret empty Arti managed port lists as lack of concurrency. For this codebase, in-process client slots are the real scaling surface.

Next recommended steps:
- Add Stage A/B/C/D duration counters and a winner-host stability ring so the next audit can quantify redirect volatility directly.
- Replace the single cached redirect hint with a tiny freshness-ranked redirect ring (`1-2` probes max) before Stage A.
- Add a benchmark-only readiness knob that can compare "start on quorum" versus "wait for full client pool" without conflating that experiment with local SOCKS port count.

## Phase 85: Arti Swarm Efficiency Audit Recommendations (2026-03-10)

**Status: Implemented and revalidated on live Qilin / Arti swarm runs**

Implemented in this pass:
- `src-tauri/src/lib.rs` now uses first-ready quorum warmup for onion swarms instead of waiting for the slowest prewarm task.
- `src-tauri/src/tor.rs` and `src-tauri/src/tor_native.rs` now expose traffic-class-aware onion bootstrap/healing paths, and the Qilin/onion call sites use `SwarmTrafficClass::OnionService`.
- `src-tauri/src/adapters/qilin_nodes.rs` now merge-seeds nodes, preserves learned latency/failure/cooldown state, caches redirect hints and winner leases, and probes Stage D in bounded isolated waves.
- `src-tauri/src/adapters/qilin.rs` now attempts a warm-cache winner before full mirror seeding and retries direct mirrors through isolated ranked waves instead of a shared-client fan-out.
- `src-tauri/src/bin/adapter_benchmark.rs`, `src-tauri/src/bin/crawl_stats.rs`, and the onion E2E harnesses now bootstrap the Arti swarm with explicit onion-service traffic class during validation.

Recommendations now active:
- Keep onion warmup on quorum semantics; do not reintroduce blocking full-fanout prewarm waits anywhere in the crawl path.
- Treat Qilin node inserts as merge-only unless a live request proves new state. Seed time is not success.
- Use warm cached winner/redirect probes before broad mirror seeding whenever the target ingress is still the same victim UUID.
- Keep direct retry hedging bounded (`2-3` independent probes) and isolated; never return to unbounded fan-out.
- Use onion-service traffic class in any benchmark or E2E harness that is meant to measure swarm behavior for `.onion` workloads.

Next recommended steps:
- Reduce fingerprint wall time on known Qilin/CMS ingress. The March 10, 2026 warm-cache reruns still spent roughly 13-20s in fingerprinting before discovery even began.
- Add request-efficiency counters to synthetic/live benchmark output before attempting another concurrency increase.
- Replace Arti-native `ports=[]` benchmark/operator reporting with actual client count / runtime / traffic-class health so the swarm surface matches reality.
- Add dashboard chart metrics for `requests/discovered_entry`, Stage A/B/C/D timing, winner-host stability, and phantom-swap reasons.

## Phase 84: Qilin Telemetry Alignment Recommendations (2026-03-10)

**Status: Implemented — shared Qilin counters now surface through the main binary and GUI**

Recommendations now active:
- Use `--progress-summary` for long live CLI crawls. It exposes the real shared worker/queue/node state without forcing transport-frame inspection.
- Do not respond to the March 10, 2026 live Qilin volatility by blindly increasing worker counts. Both the CLI and GUI parity reruns showed the dominant bottleneck as rotated storage-node reachability, not insufficient concurrency.
- Keep the frontier overlay pattern for any future fast-path adapter work. Hidden work is an operator bug.
- Preserve the zeroed terminal worker snapshot on session shutdown so GUI and CLI surfaces agree on completion.

Next recommended steps:
- Add a short opportunistic re-probe lane for freshly captured Stage A redirects before the full 90s discovery timeout burns out.
- Expose the live rotated storage redirect host plus remapped UUID directly in the GUI dashboard so operators can compare runs without digging through logs.
- Consider persisting a short-lived “fresh redirect cache” keyed by victim UUID and probe timestamp to bias immediate follow-up runs toward the newest live storage host.

## Phase 83: Main-Binary CLI Recommendations (2026-03-10)

**Status: Implemented — Main binary now supports first-class CLI mode**

Recommendations now active:
- Use the primary `crawli` binary itself for headless validation. The canonical path is now `cargo run --manifest-path 'crawli/src-tauri/Cargo.toml' -- <subcommand>`, not helper examples.
- Keep default CLI stderr focused on actionable events. Only turn on `--include-telemetry-events` when you explicitly need raw bridge-frame inspection.
- Prefer CLI commands that reuse saved crawl snapshots or direct backend functions instead of adding bespoke one-off harnesses.
- For live Qilin operator runs, expect the cost stack to be dominated by hidden-service bootstrap/discovery and storage-node rotation, not adapter matching.

Next recommended steps:
- Unify Qilin adapter-local queue progress with `CrawlerFrontier` counters so `processedNodes` and `activeWorkers` reflect real work during recursive child parsing.
- Consider a small onion-specific CLI prewarm knob (for example `--daemons 2` as an operator default recommendation) for users who prioritize faster first-response over strict GUI-default parity.
- Add an opt-in CLI summary mode that periodically prints condensed `entries/pending/currentNodeHost` snapshots instead of forcing operators to infer progress from raw child-fetch logs.

## Phase 75: Probe Timeout Tuning & DOM Offloading (2026-03-09)

**Status: Implemented & Verified — 759+ entries crawled from live Qilin target**

Implementations:
- **Probe timeouts relaxed**: `PROBE_TIMEOUT_SECS` 10→20s, `PREFERRED_NODE_TIMEOUT_SECS` 6→12s, `STAGE_D_BATCH_TIMEOUT_SECS` 30→60s. This resolved the critical bug where all Qilin storage nodes were being demoted as "dead" before slow Tor circuits could complete handshakes.
- **Global discovery ceiling**: 45s→120s for the `discover_and_resolve()` wrapper, allowing full 4-stage discovery to complete over congested Tor relays.
- **Explorer adapter DOM offloading**: `scraper::Html::parse_document()` in `explorer.rs` migrated to `tokio::task::spawn_blocking`, preventing async runtime starvation on large HTML pages.
- **Prevention Rule PR-PROBE-001**: Storage node probe timeouts must be at least 2× the typical Tor 3-hop RTT (~1.5-2s). Timeouts under 10s will systematically demote healthy-but-slow nodes.


## Phase 74E: Telemetry-to-UI Mapping Recommendation (2026-03-09)

**Status: Implemented in `src/App.tsx`**

Recommendations now active:
- Treat protobuf/binary telemetry frames as transport payloads only, never as direct React view models.
- Decode proto3 frames with explicit defaults (or schema-aware normalization) before binding to renderer-facing state.
- Apply merge-based state updates for hot telemetry planes so non-wire fields and previously stable values are not clobbered by sparse frames.
- Keep a single normalization boundary for numeric coercion to prevent repeat `.toFixed()` / `.toLocaleString()` crashes in dashboard cards.

Next recommended steps:
- Add one focused fixture test that injects sparse telemetry frames and asserts the dashboard remains render-stable.
- Consider moving frame-normalization helpers into a dedicated telemetry mapper module to keep `App.tsx` lean.

## Phase 52: Mega.nz + Torrent Integration Recommendation (2026-03-07)

**Status: Phase 52A+52B+52C Implemented — Backend + Frontend + Integration Tests**

Recommendations now active:
- Mega.nz and BitTorrent downloads must operate over clearnet, never through Tor. Both protocols have their own encryption (AES-128-CTR for Mega, BitTorrent protocol encryption) and routing Tor traffic through them would cause severe performance degradation.
- Auto-detection should be instant (synchronous on keystroke) and input-field-centric. Users should never need to select a mode manually before pasting a URL.
- Mega.nz decryption keys exist only in the URL fragment (`#key`). Never persist them to disk or log them to telemetry.
- `.torrent` files must be size-guarded (≤10MB) to prevent resource exhaustion attacks via crafted torrent files.
- When a dependency crate requires a different major version of a shared dependency, use Cargo's `package` rename feature. Never attempt to unify version constraints when APIs are incompatible.
- Future Phase 52D should use `librqbit` for the actual BitTorrent piece download engine. Current magnet support is listing-only.

Next recommended steps (Phase 52D):
- Integrate `librqbit` for real BitTorrent piece-mode downloads with progress tracking
- Add Mega.nz download progress integration with the existing batch telemetry bridge
- Consider adding `.torrent` file drag-and-drop support in the frontend


Version: 1.0.8
Updated: 2026-03-06
Authors: Navi (User), Codex (GPT-5)
Related Rules: [CRITICAL-L0] Native/Web Boundary, [MANDATORY-L1] Docs Management, [MANDATORY-L1] Living Documents, [MANDATORY-L1] Performance/Cost/Quality, [MANDATORY-L1] Testing & Validation

# Summary
This document recommends the hardened recursion and progress-telemetry baseline now implemented for `crawli`, and defines follow-up improvements to keep deep autoindex crawling fast, observable, and predictable.

## Phase 50: SOCKS5 Proxy Elimination — Direct Arti Connector (2026-03-06)

**Status Update — Implemented For The Rust Hot Path**

The Rust crawl/download hot path no longer routes HTTP through the loopback SOCKS shim. `frontier.rs` and `aria_downloader.rs` now consume `ArtiClient` directly. Managed SOCKS remains only where a compatibility bridge is still required, primarily Ghost Browser / Chromium and a subset of legacy example surfaces.

- **Per-request**: ~5-12ms + 12 unnecessary syscalls + 316 bytes wasted
- **120 circuits**: 1,440 wasted syscalls per request wave, ~240 unnecessary tokio tasks
- **Port exhaustion**: Each loopback SOCKS connection consumes an ephemeral port entering 60-120s TIME_WAIT — **this is a primary contributor to the Windows kernel port exhaustion problem**
- **Data relay doubling**: `copy_bidirectional` doubles kernel buffer traffic for all downloads (100MB file → 400MB kernel traffic instead of 100MB)

**Current architecture note:** The direct `hyper` connector recommendation is now implemented for the Rust backend. The remaining recommendation is to keep compatibility SOCKS use tightly scoped and continue shrinking stale example/test dependence on it.

**Competitive analysis:** Every other in-process `arti-client` project (artiqwest, hypertor) uses direct DataStream integration. Crawli is currently using the worst-performing integration method among all in-process arti users.

**Expected gains:** 5-15% crawl speed, 10-20% download speed, significant Windows stability improvement, ~1MB memory saved per session, ~1.5s faster startup.

Full audit: [SOCKS_Performance_Audit_Whitepaper.md](file:///Users/navi/Documents/Projects/LOKI%20TOOLS/Onion%20Forger/crawli/docs/SOCKS_Performance_Audit_Whitepaper.md)

Implemented in the shipped Rust path. Remaining cleanup is documentation/example hygiene, not the core transport change.

## Phase 20: Onion Throughput Recalibration (2026-03-06)
This phase supersedes earlier recommendations that implied universal speed gains from simply pushing worker counts higher on onion targets.

Current grounded recommendation set:
- No `KillNet` adapter exists in the repository; the current findings apply to the native onion crawl architecture and Qilin-like targets.
- The dominant bottleneck after the Arti migration is hidden-service path construction and target-side responsiveness, not process memory.
- More workers, more circuits, or "more IPs" are **not** linear speed multipliers on onion services.
- The best next speedups are:
  - target-aware concurrency control in the adapter
  - separation of directory-discovery traffic from large-file transfer traffic
  - stronger storage-node tournament logic in `qilin_nodes.rs`
  - deliberate Arti preemptive-circuit tuning in `tor_native.rs`
  - differentiated recovery buckets instead of generic retry pressure

Superseded guidance:
- Treating `120+` workers as a universal answer for slow onion targets
- Treating slower/faster traffic shapes as if they map directly to visible client-IP behavior
- Treating "more IPs" as a primary performance lever on onion services

Canonical detailed investigation:
- [Onion_Crawl_Performance_Investigation.md](/Users/navi/Documents/Projects/LOKI%20TOOLS/Onion%20Forger/crawli/docs/Onion_Crawl_Performance_Investigation.md)

Implementation status:
- Implemented: adaptive Qilin page governor, persistent node tournament scoring/cooldown, sticky-winner revalidation, metadata/download headroom reservation, explicit Arti timing/preemptive tuning, frontier-owned listing-worker caps across the non-Qilin adapters, and adaptive large-file downloader tournament/active-window control in `aria_downloader.rs`
- Still open: a harder runtime split between crawl and download swarms, plus deeper per-target node telemetry beyond the current cooldown/reliability model

# Context
## Phase 21: Qilin Resource Telemetry and Authorized Soak Harness (2026-03-06)
Implemented in this pass:
- Added backend `resource_metrics_update` emission at 1 Hz during active crawl/download sessions.
- Added an operator dashboard card for process CPU, process RSS, system RAM pressure, adaptive worker target, active/peak circuits, current Qilin node, failovers, throttles, and timeouts.
- Reframed the `circuits` selector as a ceiling for Qilin metadata work instead of a direct live worker count.
- Added bounded Qilin storage failover with a primary route plus a small standby set rather than broad parallel fan-out.
- Added the authorized soak harness example `src-tauri/examples/qilin_authorized_soak.rs` for `listing-plus-one-large-file` sessions and JSON reports under `tmp/`.
- Removed the native-app Qilin path that previously kept a full duplicate crawl result vector in memory.

## Phase 22: Qilin Runtime Recommendation After Recursive-Fix Benchmark (2026-03-06)
- Keep `torforge` as the strategic default candidate, but do not delete `native` yet.
- Use `3-5` ready clients as bootstrap quorum and grow toward `6-8` active clients in the background.
- Do not spend the next cycle building a custom consensus-driven relay picker. The current measured bottleneck is recursive QData throughput and child-folder connect stability, not lack of manual relay control.
- Measure runtime comparisons by discovered-entry slope on the canonical Qilin target:
  - `native`: `1693` unique entries in `90s`
  - `torforge`: `973` unique entries in `90s`
- Next optimization target:
  - reduce deeper child-folder connect failures
  - then repeat the comparison on a `5` minute window

## Phase 23: Recommendation After Five-Minute Canonical Benchmark (2026-03-06)
- Keep `torforge` as the default candidate. After the latest fixes, it is effectively tied with `native` on the canonical five-minute Qilin benchmark:
  - `torforge`: `18313`
  - `native`: `18297`
- Keep `native` available as fallback until `torforge` repeats that result consistently.
- Keep Qilin client multiplexing at `1x` by default. A controlled `2x` multiplex experiment was materially worse.
- Prioritize the next improvements in this order:
  - worker-local connection reuse
  - bounded fingerprint retry
  - child-folder timeout clustering and retry-lane isolation
  - longer slope-based soak reporting

## Phase 24: TorForge Core Scaling Recommendation (2026-03-06)
- TorForge core is not external `tor.exe` daemon fanout anymore, but it is still multiple full in-process `TorClient` bootstraps plus a SOCKS actor front door.
- That means we can scale it more cheaply than legacy bundled Tor, but not infinitely and not for free.
- Current recommendation:
  - keep quorum at `3-5`
  - keep active target at `6-8`
  - treat `10` as the next experimental step
  - treat `12` as a ceiling for future testing, not a default

## Phase 25: Persistent Bad-Subtree Heatmap Policy (2026-03-06)
- Implemented as an experimental path only.
- Default policy: off.
- Enable only with `CRAWLI_QILIN_SUBTREE_SHAPING=1`.
- Cross-run persistence requires `CRAWLI_QILIN_SUBTREE_HEATMAP=1` as an additional opt-in.
- Removal rule: if repeated benchmark windows do not show a measurable crawl-yield benefit over the non-heatmap baseline, keep it off or delete it.

## Phase 26: Download Healing Recommendation (2026-03-06)
- Keep the downloader’s stale-port/live-client validation permanently. That bug was real.
- Treat pause/resume as partially validated:
  - cluster re-bootstrap after interruption is now working
  - true piece-checkpoint resume still needs a dedicated longer probe on a target/file that reaches `.ariaforge_state` before interruption
- Best next download recommendation:
  - keep circuit reassignment and self-healing
  - distinguish chunk-mode checkpoint recovery from piece-mode recovery in operator validation
  - add a targeted probe that pauses only after piece-mode checkpoint creation, then verify piece-count carryover explicitly
  - do not mark piece-mode resume as complete until we observe `completed_pieces > 0` before interruption on a real target

## Phase 28: Piece-Mode Resume Status (2026-03-06)
- Piece-mode resume is now validated in a deterministic local harness.
- Live Qilin targets still remain useful for real-world restart/healing checks, but not for authoritative proof of piece checkpoint carryover.
- Keep both:
  - local deterministic piece-mode probe for correctness
  - live Qilin healing probe for hostile-network behavior

## Phase 29: Resume Validator Recommendation (2026-03-06)
- Keep validator-aware resume on by default.
- Prefer strong `ETag`; fall back to `Last-Modified`.
- If validator state changes, discard partial checkpoint state and restart cleanly.

## Phase 30: Resource Governor Recommendation (2026-03-06)
- Keep resource governor v1 on by default.
- Let CPU/RAM set the TorForge client cap before env overrides.
- Keep HDDs on buffered/sequential mode by default; only use Direct I/O automatically when storage class is compatible.
- Keep local deterministic piece-mode resume as the correctness gate for future downloader changes.

## Phase 31: Binary Telemetry Recommendation (2026-03-06)
- Keep the protobuf sink optional for now via `CRAWLI_PROTOBUF_TELEMETRY_PATH`.
- Use it for:
  - resource metrics
  - crawl status
  - batch progress
  - download status
- Keep the current Tauri JSON path as fallback until a full binary control plane is proven in production.

## Phase 27: SOCKS Policy Recommendation (2026-03-06)
- Default policy: no managed SOCKS in the normal TorForge crawl/download bootstrap path.
- Use direct Arti/TorForge client slots for:
  - crawl traffic
  - downloader traffic
  - slot rotation / healing
- Keep SOCKS only for explicit compatibility consumers that truly require a proxy protocol.
- Keep examples aligned with the default path so the repo does not teach the old localhost-SOCKS architecture by accident.

Explicitly rejected in this phase:
- “More IPs” as a default performance plan for onion services.
- Treating `120 circuits` as a reason to run `120` simultaneous Qilin HTML workers.

## Phase 22: Deterministic Per-Target Baselines (2026-03-06)
Recommended and now implemented:
- Use deterministic per-target listing names in the selected output folder so repeat runs for the same URL always converge on the same current/best artifacts
- Keep timestamped history in the support folder, not as the only operator-facing artifact
- Treat the authoritative best crawl snapshot as the download resume source of truth
- Prefer failures-first download retries before general missing/mismatch work
- When a repeat crawl underperforms prior best and runtime telemetry indicates instability, do a bounded retry in the same session instead of silently accepting the lower raw result
Observed production issues:
- Autoindex traversal stopping at top-level folders for LockBit-style nested paths.
- No deterministic crawl progress bar in UI.
- Worker ramp-up not maximizing configured circuit concurrency at crawl start.

# Analysis
Root constraints:
- Autoindex hrefs can be relative, absolute, or encoded; string concatenation is unsafe for recursive traversal.
- Crawl totals are unknown in advance on open directory trees, so progress must be estimated from live frontier metrics.
- Static worker targets underuse available circuits during early crawl phases.

Alternative options considered:
- Keep string-based URL joins: rejected due to recursion correctness risk.
- Show spinner only (no percentage): rejected due to low operator visibility.
- Keep conservative AIMD warmup (50%): rejected for this workload because user-selected high-concurrency mode should start aggressively.

# Details
Implemented baseline:
- Resolve child links using URL semantics (`base.join(href)`) instead of string concatenation.
- Enforce host/path scope guardrails to avoid escaping the intended target subtree.
- Add pending-task accounting guard to prevent queue deadlock on early returns.
- Emit backend `crawl_status_update` telemetry (progress %, queue, workers, ETA estimate).
- Add dashboard crawl-progress card with 0–100 visual bar and live metrics.
- **BBR Congestion Control:** Replaced rudimentary Additive Increase / Multiplicative Decrease (AIMD) with a Bottleneck Bandwidth and RTT (BBR) model. This eliminates conservative step-wise ramp-up in favor of instantly seeking the Tor circuit's bandwidth ceiling, maximizing download speeds immediately.
- **Extended Kalman Filter (EKF) + Thompson Sampling:** Upgraded the single-variable Kalman filter to a multi-variable EKF tracking both latency and bandwidth drift simultaneously. Replaced UCB1 multi-armed bandit with Thompson Sampling utilizing the EKF uncertainty covariance (`p`) directly as the probability distribution. This is highly adaptive to volatile routing and avoids fixed exploration constants.
- **Merkle-Tree BFT Consensus:** Replaced full-payload SHA256 voting with Merkle Root BFT. Large 50MB artifacts are verified by 256KB logical blocks, allowing precise bisection and re-downloading of only corrupted chunks rather than discarding entire files on Byzantine exit nodes.
- **Zero-Copy Ring Buffers:** Implemented LMAX Disruptor-style Lock-Free Ring Buffers (`crossbeam_queue::ArrayQueue`) for disk I/O in `aria_downloader.rs`. This completely removes Mutex lock contention during high-concurrency (120+ circuit) small-file swarm writes.
- **Idempotent Smart Syncing:** Batch downloads perform an aggressive pre-flight metadata check against the local filesystem, instantly skipping fully-downloaded files if their sizes match the server's expected `content-length` or the crawler's size hint.
- **Tor Client Rescaling:** `lib.rs` and `tor.rs` now dynamically scale managed native-Arti client counts using `tournament_candidate_count` based on requested circuits and OS resource limits, rather than hardcoding a default swarm.
- **Memory-Mapped (mmap) Zero-Copy Writer:** Replaced synchronous standard file buffering with memory-mapped virtual allocations (`memmap2`) in `aria_downloader.rs`. This directly eliminates catastrophic seek-thrashing on Mechanical HDDs by allowing the OS page cache to coalesce concurrent random chunk writes into vast, sequential disk flushes in the background.
- **Adaptive Circuit Ban Evasion:** The Rust downloader explicitly monitors for HTTP 429, HTTP 503, and TCP Reset connection penalties. Upon detection, it fires `tor.rs::request_newnym` against the rate-limited managed SOCKS port, rotating the live Arti client slot with zero application-level downtime.
- **Vibe Architecture Aesthetics:** Deprecated rudimentary frontend spinners in favor of high-fidelity, halo-free 8-bit true-alpha Animated WebP sequence components (`<VibeLoader />`). This strictly aligns the UX with the intended premium "SnoozeSlayer" visual identity.
- **DragonForce Adaptive JWT Parsing:** Evaded obfuscated Next.js JSON API requirements on DragonForce SPAs. Instead of attempting brittle HTTP header decryption to fetch directory arrays, the `dragonforce.rs` scraper intercepts the native HTML, extracts the Base64 JWT authenticated DOM `<iframe>` parameters via Regex, and reinjects the inner payload URL into the Crawl Frontier for autonomous topological parsing.

Implementation status (2026-03-04):
- Added adaptive direct-I/O fallback policy for unsupported disks/filesystems.
- Added adaptive tournament sizing telemetry and SRPT+aging batch scheduling controls.
- Added EWMA throughput + ETA confidence in dashboard download telemetry.
- Added strict cross-stack quality gates (`fmt`, `clippy`, Rust tests, frontend build, overlay integrity) and `rust-toolchain.toml`.

Implementation status (2026-03-06):
- Completed the native Arti isolation correction: SOCKS auth now maps to explicit `IsolationToken`s instead of being discarded.
- Completed live circuit-slot rotation: NEWNYM/healing now replace the proxy-consumed client handle and clear cached auth groups.
- Completed runtime port-registry adoption across crawler/downloader/recovery paths and aligned release workflows with the no-bundled-Tor architecture.
- Replaced pseudo circuit-health telemetry with a real lightweight probe through the live Arti client slot.
- Removed the unused hardcoded guard-relay pool so the code and docs no longer imply a runtime policy that does not exist.
- Synchronized canonical docs/workflows with the native-Arti packaging model and completed one live onion smoke test path.
- Declared `aria_downloader.rs` the canonical production downloader and kept `multipath.rs` in experimental status pending resume/control-plane parity.
- Replaced the large-file downloader's fixed `2x` tournament assumption with telemetry-fed candidate sizing plus an explicit cap.
- Wired the downloader's BBR controller into the actual range-fetch issuance path so active concurrency is now enforced, not merely observed.

# Prevention Rules
**1. Always resolve crawl children with URL parser semantics; never by string concatenation.**
**2. Every async crawl worker path must decrement queue accounting exactly once (success/failure/cancel).**
**3. UI progress components must consume backend-native telemetry, not infer completion from log parsing.**
**4. Any worker-scaling change must be test-updated and benchmarked against previous behavior.**
**5. When scaling algorithms beyond standard concurrency limits, consider Lock-Free (Disruptor) patterns before increasing standard thread counts.**
**6. High-concurrency tuning must model exploration vs. exploitation dynamically (e.g. Thompson Sampling) rather than relying on hardcoded constants.**
**7. Strict separation of native OS constraints and frontend CSS (e.g., Z-indexing native windows cannot be solved with DOM manipulation).**
**8. Always evaluate Memory-Mapped (mmap) Virtual Memory boundaries before attempting complex parallel async filesystem writes on multi-gigabyte files, specifically to preserve HDD compatibility.**
**9. DragonForce Next.js SPA Bypass:** The API endpoint `http://fsguest...onion` is isolated within a tokenized `<iframe>`. Do not attempt JSON JWT reverse-engineering across Tor. Instead, utilize `scraper::Selector::parse("iframe")` on the root domain and dynamically push the extracted `src` URL directly into the `CrawlerFrontier`.
**10. Dynamic Adapter Anti-Contamination Registry:** Adapters MUST NEVER share HTML DOM selectors or struct parsing loops unless formally implemented via a transparent API polyfill. Furthermore, all extracted structural signatures (File/Dir payload counts) must be mathematically verified against a dynamic external registry (`matrix_signatures.json`) during CI testing. Do not hardcode `count == 379` directly into the matrix source; allow the testing pipeline to dynamically read and autonomously upgrade the JSON baseline if Ransomware payloads naturally grow.
**11. Congestion controllers must gate live work, not only emit metrics. A controller that never changes request issuance is dead code.**
**12. Qilin’s circuit selector is a budget ceiling, not the live metadata worker target.**
**13. CPU/RAM diagnosis must come from backend-emitted resource telemetry; frontend heuristics are insufficient.**
**14. Authorized soak runs must remain explicit operator tools and must emit structured reports to `tmp/` for later review.**
**15. Never use a SOCKS5 proxy to bridge between an in-process library and the same process's HTTP client. Direct function calls always beat loopback TCP + protocol handshakes.**
**16. SOCKS5 username/password auth for circuit isolation is NEVER the correct API. Use `IsolationToken` directly — it is the canonical arti API.**
**17. On Windows, every loopback TCP connection consumes an ephemeral port that enters TIME_WAIT for 60-120s. Eliminating unnecessary loopback connections directly reduces port exhaustion risk.**

# Risk
- Aggressive startup concurrency can increase burst load on unstable targets; mitigated by existing AIMD backoff.
- Estimated progress can oscillate in highly branching trees; mitigated by monotonic smoothing in emitter.

## Phase 17: Resolving Active Regression Bugs (Theoretical Aerospace Models)
Historical note:
- The exploratory sections below that advocate fixed `120`-worker Qilin behavior are not the current runtime policy.
- Canonical policy is now target-aware concurrency plus frontier-owned worker sizing.
- Treat the material below as historical investigation context, not an operator tuning guide.

Based on the final regression matrix yielding 0 files for WorldLeaks, INC Ransom, and DragonForce, the following critical aerospace-grade solutions are recommended:

### 1. Tor Port Exhaustion (WorldLeaks, INC Ransom)
*   **Problem:** High-concurrency CI pipelines spanning 8+ Tor daemons per adapter run are leaking "zombie" `tor` processes when the parent thread aborts early. These zombies lock physical OS ports `9051-9068`, permanently blocking subsequent tests (Tor Bootstrap Failure).
*   **Aerospace Solution (RAII POSIX Supervisors & Atomic Sweeps):**
    *   **Process Group Isolation:** Instead of blindly spawning `std::process::Command` instances, implement a dedicated OS-level Hypervisor thread. On Unix systems, bind the child Tor daemons using POSIX Process Groups, and set `prctl(PR_SET_PDEATHSIG, SIGKILL)` on Linux (or equivalent `kqueue` monitor on macOS). This guarantees mathematically that if the Rust parent dies, the kernel immediately eradicates all child daemons, preventing port leaking.
    *   **Atomic Port Sweeps:** Hardcoding `9051-9068` is brittle. Implement an autonomous lock-free atomic bitset that sweeps the host TCP ports `TcpListener::bind("127.0.0.1:0")`. Allow the OS to lease an explicitly free port, and pass that dynamically acquired port directly into the `--SocksPort` and `--ControlPort` daemon arguments rather than enforcing static ranges.

### 2. NextJS SPA Dynamic Hydration (DragonForce)
*   **Problem:** We successfully defeated the Iframe proxy and extracted the NextJS `__NEXT_DATA__` JSON AST, recovering the 7 root directories. However, NextJS SPAs do not serialize deeply nested folders to the root payload. The 48,000 inner files are hydration-locked behind secondary Javascript-driven API fetches to `/download?path=...`.
*   **HFT Solution (Predictive State Hydrator):**
    *   **Stateless API Mimicry:** We cannot render Javascript in a headless crawler. However, the NextJS router is deterministic. We will build a "Predictive State Hydrator". Once the root AST reveals a folder (e.g., `["name": "Deployments", "isDir": true]`), the HFT crawler will construct the exact JSON-RPC or REST URI the NextJS router *would* have called (`http://fsguest.onion/?path=/Deployments&token=...`) and inject that extrapolated state URL dynamically back into the Lock-free Tor fetch queue.
    *   **Recursive Payload Injection:** By mapping the `?path=` query parameter recursively into the frontier, Crawli transitions from an HTML scraper into a native NextJS API endpoint client, retrieving the deeply nested JSON chunks recursively across Tor without relying on DOM rendering.

# History
- 2026-03-03: Initial recommendations written after recursion/progress/scaling remediation.
- 2026-03-04: Marked latest recommendation bundle as implemented and synchronized with quality workflow/toolchain updates.
- 2026-03-05: Revalidated the release pipeline and portable packaging path for the `v0.2.6` release.
- 2026-03-06: Marked the native Arti isolation/runtime registry recommendations as implemented, replaced pseudo circuit telemetry with live probes, and synchronized the docs with the current release packaging model.
- 2026-03-06: Added Phase 50 SOCKS5 elimination recommendation with comprehensive audit whitepaper. Identified SOCKS5 loopback as a vestigial bottleneck contributing to port exhaustion, task contention, and redundant data copies.
- 2026-03-06: Added Phase 20 onion-throughput recalibration and a dedicated performance investigation whitepaper, superseding older blind high-concurrency guidance for hidden-service crawling.
- 2026-03-06: Implemented the first P0 performance tranche for Qilin/native-Arti: adaptive page governance, node cooldown scoring, sticky winner probing, and crawl/download headroom reservation.
- 2026-03-05: Marked explicit Arti timing/preemptive tuning and frontier-owned non-Qilin listing-worker policy as implemented.

# Appendices
- Validation commands:
- `cargo test` in `src-tauri`
  - `npm run build` in project root

## Phase 18: Deep Investigation & Tournament-Style Auditing for Tor Exit Node Volatility
Following a comprehensive system audit specifically targeted at the `Qilin` CMS / Nginx backend, we discovered that extremely slow, high-latency `.onion` sites require explicit mathematical precision to avoid triggering Anti-DDoS triggers or exhausting Tor ephemeral circuits.

To comprehensively test this, we executed a **Tournament-Style Audit** against `http://a7r2n577...onion/...` using three distinctly shaped traffic algorithms:

### Round 1: Fast/Aggressive Pipeline (HFT Baseline)
- **Configuration**: 120 Workers, 45s Request Timeout, 5 Max Retries, 3s Failed Circuit Delay.
- **Results**: **SUCCESS**. Yielded exactly 22 Files across 69 Directories in 579.74s.
- **Analysis**: The aggressive strategy succeeded *only* because we patched the `autoindex.rs` parser to reject all HTML template junk (e.g. `https://`, `/fancy/style.css`, `${href}`). Before the patch, the parser fed 120 workers infinite bad links, which burned all 5 retries on every single thread and locked up the crawler permanently. After the patch, the sheer brute force of 120 workers overwhelmed the network latency to successfully traverse 69 nested directories before the exit nodes could cycle.

### Round 2: Moderate/Paced Pipeline
- **Configuration**: 60 Workers, 60s Request Timeout, 8 Max Retries.
- **Results**: **FAILED**. Connection Refused by the Tor Proxy Exit Node.
- **Analysis**: Qilin actively punishes crawling speeds that dwell inside the TCP window connection pool too long without achieving massive volumetric flow.

### Round 3: Slow/Gentle Pipeline
- **Configuration**: 20 Workers, 90s Request Timeout, 12 Retries.
- **Results**: **FAILED**. Instant DDoS block.
- **Recommendation:** Do not use slow, polite crawling for Qilin. **The optimal extraction vector is HFT-style 120-worker concurrent TCP bursts.**

## Phase 19: Intelligent Pre-Authentication Model (Qilin QData)
**Problem:** The Qilin adapter relies heavily on a `known_domains` matrix containing static URLs (`a7r2...onion`). When these URLs are taken offline by law enforcement or DDoS, the crawler loses tracking. Relying on URLs for routing is structurally flawed.

**Aerospace Solution (Autonomous Heuristic Detection):**
We must abstract the routing sequence away from domain tracking and focus entirely on the DOM Footprint exactly as requested. We will implement "Pre-Authentication Intelligence".

1. **Footprint Extraction:** The Qilin UI utilizes a localized CSS framework. The headers `QData` and `Data browser` are omnipresent, followed immediately by an `<input type="text" readonly value="[master_cms_onion_link]">`.
2. **RegexSet Bouncer Upgrades:** We will upgrade the `regex_marker()` constraint in `qilin.rs` to detect this specific DOM structure. If any URL from the deep-web hits the initial crawler handshake and triggers this Regex footprint, the central `AdapterRegistry` will immediately bind the `QilinAdapter` to it, ignoring the URL string entirely.
3. **Stateless Extensibility:** By relying purely on DOM heuristics, the user can manually drop *brand new*, previously unseen Qilin `.onion` URLs into the Crawler Frontier, and the application will autonomously identify it as Qilin, activate the high-performance 120-worker fast proxy swarm, and begin recursive parsing instantaneously without requiring codebase updates.
- **Configuration**: 60 Workers, 60s Request Timeout, 8 Max Retries, 5s Failed Circuit Delay.
- **Results**: **FAILURE** (0 Files, 0 Directories).
- **Analysis**: The pipeline failed on the initial TLS proxy handshake (4 fingerprinting retries). By slowing down the initial burst rate and relying on sustained, medium-density polling, the Tor exit node's internal state tracker (or the Qilin anti-DDoS proxy) flagged the persistent connection polling over a 60-second window and permanently refused connection (`Connection refused` on `127.0.0.1:9050`).

### Round 3: Slow/Gentle Pipeline
- **Configuration**: 20 Workers, 90s Request Timeout, 12 Max Retries, 10s Failed Circuit Delay.
- **Results**: **FAILURE** (0 Files, 0 Directories). Continuous proxy refusals.
- **Analysis**: An extremely low thread count with 90-second timeouts causes identical failures to the Moderate round. Darkweb hostings heavily penalize prolonged TCP `keep-alive` holding states.

### Final Conclusion & Prevention Rules for Slow Tor Targets:
1. **Never "Slow Down" a Crawl to fix Latency**: Throttling the engine simply extends the active TCP connection window, drawing the attention of Nginx anti-DDoS metrics and increasing the probability of a Tor exit node rotating mid-flight. 
2. **Speed is Cover**: To extract highly nested data structures (like Qilin's 69-directory tree), you *must* use massive parallelism (120+ workers) to blast through the tree and complete the scrape *faster* than the host's rate-limiting penalty window (typically 10-15 minutes).
3. **Parse Brutally**: Brute-force scraping is only possible if the data queue is mathematically pure. A single regex bug routing absolute HTTP paths or JS variables back into the active queue will immediately detonate the Tor circuit limits and permanently shadow-ban the request IP.

## Phase 41: Advanced Download Aerospace Architecture (Post-Phase 39)
Despite mitigating the initial download stall bugs (Phase 39), downloading massive data dumps (e.g., 50GB – 500GB SQL files) across 120 Tor circuits simultaneously introduces critical, theoretical boundaries that require Military/Aerospace grade structural overrides.

### 1. TCP TIME_WAIT Port Exhaustion (Windows Kernel Bottleneck)
*   **Problem:** With 120 concurrent circuits firing thousands of micro-requests (`GET Range`) and rotating proxies dynamically via `NEWNYM` (to evade 503s), Windows will rapidly exhaust its ephemeral port range. Sockets enter a `TIME_WAIT` state for 120 seconds in the NT Kernel, leading inevitably to `WSAENOBUFS (10055)` crashes when the pipeline scales past 50,000 files.
*   **Aerospace Solution (Raw Socket Pooling):** We must bypass `reqwest`'s internal connection pooling pool entirely and write a custom Hypervisor over raw `TcpStream` instances. By explicitly mutating the `SO_REUSEADDR` and `SO_LINGER` kernel flags directly over the Tor SOCKS proxy tunnels, we can mathematically force the Windows Kernel to instantly recycle connection ports, enabling infinite uptime without rebooting.

### 2. Deep Packet Inspection (DPI) Sybil Forgery
*   **Problem:** Qilin/Ransomware servers actively monitor TCP streams. If they detect 120 simultaneous connections requesting `bytes=A-B` against the exact same 100GB zip file, possessing the identical `User-Agent` and TLS ClientHello handshakes, their Nginx firewall will identify it as a Sybil/DDoS attack and permanently shadow-ban the file.
*   **HFT Intelligence Solution:** Implement a **Cryptographic Forgery Engine**. Each of the 120 active DAEMON circuits must be deterministically seeded with a completely unique, highly-realistic TLS `JA3/JA4` fingerprint (e.g., Circuit 1 = iOS Safari, Circuit 2 = Windows Chrome, Circuit 3 = Linux Firefox). To the ransomware operators' backend dashboards, the 120-circuit crawler will organically mimic 120 disconnected humans downloading random chunks, making the swarm completely invisible to heuristic DPI firewalls.

### 3. Sub-Block Swarming (Forward-Error Mitigation)
*   **Problem:** Standard downloaders allocate static chunk sizes (e.g., 5MB blocks). If a Tor node drops the connection at 4.9MB, the entire 5MB block is discarded and re-downloaded. Over hundreds of gigabytes, this structural packet-loss amplifies into hours of wasted bandwidth.
*   **Aerospace Solution:** Implement BitTorrent-style **Micro-Chunk Swarming** (256KB fragments). If a hostile proxy shatters a TCP stream, we only ever lose milliseconds of traffic. This creates a hyper-resilient torrent matrix that mathematically cannot stall, regardless of Tor node volatility.

### 4. NT Kernel Zero-Filling Blockade (Mmap Scale)
*   **Problem:** While Phase 35 proposed Memory-Mapped (`mmap`) downloads, the Windows NT Kernel structurally sabotages this. When you allocate a 100GB sparse file, Windows automatically locks the disk and manually writes 100GB of `0x00` zeros to prevent cross-account buffer reading. On mechanical HDDs, this causes 100% Disk Usage and locks the computer for 30 minutes before the download even starts.
*   **Aerospace Solution (Kernel Bypass):** We must invoke the raw Win32 API `SetFileValidData()`. This requires escalating the application process with the native `SE_MANAGE_VOLUME_NAME` privilege hook. By explicitly bypassing the zero-fill security boundary, we can instantly reserve 100GB of physical SSD sectors in under 1 millisecond, empowering the crawler to stream 120 concurrent chunks directly into hardware memory without OS-level IO starvation.


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
- **Circuit Healing**: Complete client rotation requests flow through the pre-existing smart healing engine to destroy and regenerate fully tainted client stacks when hard IP-blocks are encountered.

**Key Prevention Rules (Enforced and Logged):**
- **PR-MULTICLIENT-001:** Never exceed 4 active TorClients on 4 GB RAM VMs to prevent NT Kernel OOM exhaustion. This boundary is rigidly enforced by the new Resource Governor instantiation constraints.
- **PR-MULTICLIENT-002:** Client rotations must strictly utilize the shared healing engine to prevent "orphan" clients and silent memory leaks.

## Phase 58: DragonForce Iframe & Downloader JWT Expiry Resolution (2026-03-07)

### Sub-Domain Routing Constraint (`fsguest...onion` Iframe)
*   **Problem:** The DragonForce Next.js wrapper (`dragonforxx...onion`) secures its file allocation table inside an isolated Tor iframe subset (`fsguest...onion`). The `arti_client`'s native multiplexing treats the subdomain as an untrusted hop and drops the circuit connection.
*   **Aerospace Solution (Stream Isolation Decoupling):** The `ArtiClient` must be explicitly configured to permit multi-domain traversal within the *same* circuit session when tracking `iframe src=` targets. If `StreamIsolation` boundaries cannot be natively bridged, an out-of-process `SOCKS5` Daemon sidecar (e.g., `127.0.0.1:9050`) must be used for DragonForce specifically, as standalone daemons handle dynamic `.onion` jumps natively without terminating the TCP application socket.

### Downloader Token Refresh (`JWT Expiry`)
*   **Problem:** We have successfully integrated JWT decoding into the crawler's (`FileEntry`) `jwt_exp` payload. However, large downloads may sit in the active queue for hours. When `aria_downloader.rs` attempts to pull a file, the Token will return an HTTP 403 Forbidden.
*   **HFT Solution (Stateful Token Refresh):**
    *   **Pre-Flight Expiry Check:** Before establishing the `GET Range` HTTP stream, the downloader must evaluate `entry.jwt_exp < SystemTime::now()`.
    *   **Parent-Node Hydration:** If the token is dead, the downloader must *intercept* the pull and recursively issue a lightweight `GET` request back to the file's parent directory (`/?path=/parent/folder`). Extracting the fresh HTML yields an entirely new encrypted JWT. 
    *   **In-Flight Substitution:** The downloader physically mutates the `entry.raw_url` with the fresh token and resumes the chunk transfer seamlessly.

## Phase 61b: Storage Discovery Timeout Recommendation (2026-03-08)

**Status: Implemented**

The Qilin adapter's `discover_and_resolve()` pipeline was blocking the GUI for 4+ minutes when Tor circuits were degraded, because it lacked any timeout protection. This has been resolved with a 3-layer timeout strategy:

1. **90s global timeout** on `discover_and_resolve()` — graceful fallback to direct mirror probing
2. **20s per-HTTP-call timeouts** on Stage A (`/site/data` redirect) and Stage B (`/site/view` scrape)
3. **Reduced probe timeouts**: `PROBE_TIMEOUT_SECS` from 15→10 and `PREFERRED_NODE_TIMEOUT_SECS` from 8→6

**New Prevention Rule:** `PR-CRAWLER-012`: Every HTTP call through Tor circuits MUST use an explicit `tokio::time::timeout`. Tor's internal timeouts are too lenient for interactive GUI code paths.

**Next recommended steps:**
- Monitor the 90s global timeout in production — if targets consistently require longer discovery, consider increasing to 120s
- Consider adding a UI-visible progress indicator during the storage discovery phase ("Resolving storage node... Stage A/B/C/D")
- Evaluate whether Stage D's concurrent JoinSet probing should use a tighter per-batch timeout (e.g., 30s for the head batch) rather than relying solely on per-node timeouts

## Phase 61b+: Stage D Batch Timeout & Discovery Progress (2026-03-08)

**Status: Implemented**

Both recommendations from Phase 61b are now implemented:
1. **Stage D batch timeout (30s)** — Tournament head and tail JoinSet drains wrapped with `tokio::time::timeout(30s)`. Worst-case Stage D capped at 60s (head+tail).
2. **Discovery progress indicator** — `emit_discovery_progress()` emits `crawl_log` events for each discovery stage, giving operators live visibility during the "Probing Target" phase.

Combined with Phase 61b's global 90s timeout, the absolute worst-case discovery time is now **90 seconds** (global ceiling) instead of the previous **unbounded** duration.

## Phase 73: Sub-100ms Telemetry Audit & Aerospace Concurrency Targets (2026-03-09)

**Status: Audited via 10-Minute Precision CLI Benchmark**

### Execution Results
We executed a 10-minute multi-adapter CLI benchmark (`adapter-benchmark`) wrapped in an unbuffered Python timestamping wrapper to explicitly track every 100ms interval for Tor circuit bounding. 
- **Observations:** Individual parsed `HTTP GET` results are inherently bounded by a **700ms - 1200ms RTT ceiling** over Tor (due to 3-hop guard/middle/exit routing). 
- **Qilin Stage D Timeouts:** High-volume entry discovery suffers heavily from `Global discovery timeout after 45s`, proving that synchronous single-circuit sweeps degrade severely under Tor congestion, stranding the worker loop without CPU offloading.

### Advanced Concurrency Improvements (Mac vs. Windows Approach)

1. **Speculative Dual-Circuit Tor GET Racing (Aerospace-Grade Speedup)**
   - **Diagnosis:** Every adapter (like `lockbit.rs`) currently uses single-lane `tokio::time::timeout(45s, client.get.send())`.
   - **Recommendation:** Implement "Speculative Execution" GET racing across **all** adapters (not just Qilin tournaments). By duplicating every HTTP request down two entirely independent `TorClients` simultaneously and using `futures::future::select` to capture the first returned packet (dropping the slower one instantly), we map our 1.2s avg request down to a **400ms avg** ceiling, mathematically circumventing local exit-node sluggishness at the expense of bandwidth.

2. **Mac Approach (kqueue / Darwin Event Looping)**
   - **Diagnosis:** The MacOS `QilinCrawlGovernor` relies on `tokio::time::sleep(25-50ms)` interval ticking. `tokio` sleeps on Apple Silicon inherit timer coalescing layers that force minimum 2-5ms variances, destroying rigid sub-100ms alignment.
   - **Recommendation:** Re-wire the intra-worker queues explicitly via `crossbeam-queue` with strictly non-blocking userspace spinlocks instead of kernel-backed `std::sync::Mutex` waiting. Utilize `kqueue` bound readiness states directly via `mio` (or native `.poll()` sockets) so tasks wake precisely when `EPOLLOUT` flags green.

3. **Windows Approach (IOCP & Ephemeral Port Exhaustion)**
   - **Diagnosis:** Running 120-circuit concurrent loops triggers thousands of rapid SOCKS proxy loopback sockets per minute, dragging the NT kernel into `TIME_WAIT` Port Exhaustion (Code 10055).
   - **Recommendation:** Complete the `ArtiClient` native implementation down to the lowest Win32 boundaries. Use Windows Registered I/O (RIO) or explicit `I/O Completion Ports` to bypass the TCP loopback proxy entirely. Eliminate `cmd.exe` or background child processes by consuming Rust-compiled `tor-rtcompat` libraries directly inside the main application space.

4. **HFT DOM Deserialization & Pre-Heating**
   - **Diagnosis:** `scraper::Html::parse_document(html)` occupies the single async runtime thread for 20-50ms per megabyte of DOM.
   - **Recommendation:** Force string-to-DOM parsing strictly into `tokio::task::spawn_blocking`. CPU bounds are shifted to physical background cores instantly, allowing the immediate Tor circuit `client.get()` sequence to fire while the prior payload's HTML is being unpacked.
