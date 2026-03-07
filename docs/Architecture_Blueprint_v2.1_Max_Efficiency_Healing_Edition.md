> **Last Updated:** 2026-03-06T22:05 CST

# Architecture Blueprint v2.1: Max Efficiency Healing Edition

## 1. Executive Summary
- Goal: push `crawli` another `2.3x-3.4x` on hostile onion workloads while treating CPU/RAM ceilings as optimization targets, not hard fail-lines.
- Current repo strengths that remain unchanged:
  - direct Arti hot path over `DataStream`
  - TorForge-only default bootstrap with slot-based healing
  - failure-first resume planning
  - deterministic local piece-mode resume proof
  - bounded degraded retry lanes
- Highest-value v2.1 path:
  1. Fix benchmark truth and cold-start oversubscription first.
  2. Land predictive healing and dual-loop control.
  3. Lower telemetry and writer overhead before raising circuit budgets.
  4. Only then widen tournament/prefetch behavior.

## 2. Implemented in This Pass
- Benchmarks now initialize and query the sled VFS directly, so native-app Qilin runs report real discovered entry counts instead of `0`.
- Frontier BBR cold-start no longer initializes at the configured ceiling by default.
- Qilin retry enqueue paths no longer burn fixed `1500ms` worker sleeps after handing work back to shared retry queues.
- Qilin governor rebalance cadence is now configurable and defaults to `2000ms` instead of a coarse `5s`.
- Qilin crawl completion now waits for the UI/VFS batching path to drain fully, and the batch consumer exits correctly once all senders are dropped.
- Binary telemetry no longer flushes the sink on every frame; it now batches writes to reduce HDD IOPS.
- The downloader writer no longer busy-spins indefinitely when the lock-free queue is empty.
- Resume-mode downloads now coalesce contiguous missing pieces into bounded spans before issuing range requests.
- Native Arti healing defaults are now tighter and configurable: probe cadence `15s`, anomaly threshold `3` (`4` in VM mode), phantom bootstrap delay `10s`, and phantom replenish interval `20s`.
- Measured state after this pass:
  - full Rust test suite passed
  - synthetic Qilin benchmark now completes `4432 / 4432` entries for every clean and hostile profile in the `12/24/36` circuit matrix
  - deterministic piece-mode resume finished with `hash_match=true` and `9` resume-phase ranged GETs after a `2/26` checkpoint under capped tournament noise

## 3. Detailed Implementation Sequence
### Phase 51A: Truthful Measurement + Safe Cold Start
- Scope:
  - benchmark correctness
  - BBR cold-start clamp
  - remove artificial worker idling after retry requeue
- Acceptance:
  - synthetic benchmark reports VFS-backed discovered counts
  - hostile Qilin runs show no `entries=0` false negatives
  - controller startup does not begin at full circuit ceiling
- Validation:
  - `cargo test --manifest-path src-tauri/Cargo.toml --quiet`
  - `cargo run --manifest-path src-tauri/Cargo.toml --example qilin_benchmark --quiet`

### Phase 51B: Fast/Slow Healing Loops
- Scope:
  - fast request-classifier loop for throttle/circuit/timeouts
  - slow sanity probe loop for persistent slot degradation
  - per-target node/circuit feature ledger
- Acceptance:
  - no single degraded slot waits minutes before swap
  - hostile p99 tail falls below `30s` in the analytical harness
- Implemented in this pass:
  - shortened default probe loop
  - lowered anomaly threshold outside VM mode
  - reduced phantom standby bootstrap and replenish delays
- Dependencies:
  - current slot-based healing
  - target-state support root

### Phase 51C: Sparse Merkle Resume and Sequential-First Writer
- Scope:
  - sparse Merkle resume plan
  - coalesced piece spans
  - HDD-mode ordered flushes and queue parking
- Acceptance:
  - partial-file resume bandwidth reduced by `>=60%`
  - HDD random IOPS stays below `50`
- Implemented in this pass:
  - bounded contiguous-piece span planner for resume mode
  - span-aware checkpoint completion writes
  - deterministic local probe counter for resume-phase range requests

### Phase 51D: Shared-Memory Telemetry Plane
- Scope:
  - ring-buffer transport for hot telemetry
  - one bridge task to Tauri/UI
  - histogram packing for high-frequency signals
- Acceptance:
  - `50k`-entry sessions update under `50ms` p99
  - extra RSS stays under `2MB`
- Implemented in this pass:
  - added `src-tauri/src/telemetry_bridge.rs` as the first-stage telemetry plane with one 250ms bridge emitter
  - aggregated `crawl_status`, `resource_metrics`, `batch_progress`, and per-file `download_progress` into a single `telemetry_bridge_update`
  - removed dead hot-path `progress` / `speed` UI events from `aria_downloader.rs`
  - migrated the React dashboard and telemetry-consuming soak/live harnesses to the bridge
  - validated with `cargo test --quiet`, `cargo check --examples --quiet`, `npm run build`, `local_piece_resume_probe`, and `qilin_benchmark`

### Phase 51E: MPC Resource Governor
- Scope:
  - replace coarse tiered governor with constrained controller
  - integrate CPU, RSS, IOPS, and circuit churn constraints
- Acceptance:
  - HDD path remains sequential-first
  - NVMe path can scale beyond current conservative caps without churn spikes
- Implemented in this pass:
  - expanded `src-tauri/src/resource_governor.rs` into bootstrap/listing/download budget helpers with pressure scoring
  - wired `frontier.rs` to derive permit caps from the governor instead of treating configured circuits as a literal crawl width
  - wired `qilin.rs` to start from a pressure-aware listing budget and clamp scale-up against live pressure
  - wired `aria_downloader.rs` so bootstrap count, small-file swarm width, tournament width, and initial active range windows all follow the same budget model
  - updated the Play bottleneck test to enforce budget coherence instead of the pre-governor `120 workers always` assumption
- Validation:
  - `cargo test --manifest-path src-tauri/Cargo.toml --quiet`
  - `cargo check --manifest-path src-tauri/Cargo.toml --examples --quiet`
  - `CRAWLI_DOWNLOAD_TOURNAMENT_CAP=4 CRAWLI_RESUME_COALESCE_PIECES=4 cargo run --manifest-path src-tauri/Cargo.toml --example local_piece_resume_probe --quiet`
  - `cargo run --manifest-path src-tauri/Cargo.toml --example qilin_benchmark --quiet`
  - hostile synthetic result remains complete at all tested widths, with `24` circuits currently outperforming `36` under throttling pressure

### Phase 51F: Hybrid Plugin Host
- Scope:
  - keep URL resolution/retry/ledger in the host
  - allow runtime-loaded site-specific stages
- Acceptance:
  - new directory-listing adapter can load without core rebuild
  - host semantics stay shared across Play, DragonForce, Pear, LockBit, and Qilin
- Implemented in this pass:
  - added `src-tauri/src/adapters/plugin_host.rs` as a manifest-driven runtime adapter host
  - added `AdapterRegistry::with_plugin_dir(...)` for explicit plugin-directory loading in tests and operator workflows
  - runtime plugins now register before the generic autoindex fallback and delegate crawl execution back into the hardened host autoindex pipeline
  - added `adapter_plugins/example_autoindex_plugin.json` as the shipped skeleton manifest
  - added engine coverage proving a runtime manifest matches without rebuilding the binary
- Validation:
  - `cargo test --manifest-path src-tauri/Cargo.toml --quiet`
  - `cargo check --manifest-path src-tauri/Cargo.toml --examples --quiet`
  - `npm --prefix /Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli run build`
  - result: runtime plugin host landed without regressing the existing adapter matrix

## 4. Must-Track Metrics
- entries/sec
- p95 and p99 crawl latency
- timeout, circuit, throttle, and HTTP failure buckets
- client-slot rotations / minute
- RSS peak
- random vs sequential write mix
- bytes retried / bytes completed
- benchmark truth source: return vector vs sled VFS

## 5. Prevention Rules v2.1
- Do not trust adapter return vectors as the sole benchmark truth source in native-app mode.
- Do not initialize congestion control at its maximum window.
- Do not sleep a worker after retry work has already been returned to the shared queue.
- Do not flush binary telemetry on every frame.
- Do not busy-spin a downloader writer thread when the queue is empty.
- Do not widen circuit budgets until measurement overhead is under control.
- Do not assert that raw client pool size must equal crawl worker ceiling after the resource governor is enabled.
- Do not let runtime plugins own retry, ledger, or frontier semantics; plugins may match and route, but the host owns crawl behavior.

## 6. Validation Matrix
- Unit + integration:
  - `cargo test --manifest-path src-tauri/Cargo.toml --quiet`
  - result: passed
- Synthetic hostile tree:
  - `cargo run --manifest-path src-tauri/Cargo.toml --example qilin_benchmark --quiet`
  - result:
    - clean `12`: `4432/4432` in `1.57s`
    - clean `24`: `4432/4432` in `1.07s`
    - clean `36`: `4432/4432` in `1.06s`
    - hostile `12`: `4432/4432` in `6.70s`
    - hostile `24`: `4432/4432` in `6.60s`
    - hostile `36`: `4432/4432` in `8.05s`
- Deterministic downloader resume:
  - `CRAWLI_DOWNLOAD_TOURNAMENT_CAP=4 CRAWLI_RESUME_COALESCE_PIECES=4 cargo run --manifest-path src-tauri/Cargo.toml --example local_piece_resume_probe --quiet`
  - result: resumed piece-mode checkpoint to completion with `hash_match=true` and `9` resume-phase ranged GETs after a `2/26` checkpoint
- Optional telemetry capture:
  - `CRAWLI_PROTOBUF_TELEMETRY_PATH=tmp/telemetry.bin cargo run --manifest-path src-tauri/Cargo.toml --example qilin_benchmark --quiet`

## 7. Open Questions
- Whether Arti `0.40` produces enough hidden-service stability gain to justify immediate migration before the predictive-healing work lands.
- Whether the final operator plane should be shared-memory first with protobuf framing, or shared-memory plus Cap'n Proto payloads.
- Whether session-only subtree fairness is sufficient without a persistent default heatmap.
