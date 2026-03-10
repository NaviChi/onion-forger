# OnionForge: AI Engineering & Context Reference
> **Last Updated:** 2026-03-10T11:55 CDT
> **Version:** 2.3.0
> **Authors:** Navi (User), Antigravity (AI)

This document serves as the master blueprint for any AI agent tasked with maintaining, extending, or recreating the OnionForge intelligence gathering application. It contains all critical architectural decisions, environment constraints, GUI styling instructions, and API behavioral knowledge required to build this system from scratch without guessing.

Current repo state note:
- Phase 95 completed the clearnet direct-file audit and direct-mode fix. The app/CLI now classify non-onion HTTP(S) archive URLs as `direct` instead of `onion`, the clearnet downloader keeps connection pooling enabled, and the March 10, 2026 `10Gb.dat` benchmark reached about `3.8 GiB` in `60s` (`~63 MiB/s`, `~530 Mbps`) with the repaired `32`-circuit clearnet lane.
- Piece-mode resume accounting now tracks partial progress across every active piece, not only the first-wave circuit count. Interrupted large direct downloads therefore restore and report progress correctly even after the transfer has moved well past the initial `32`-piece wave.
- Phase 91 completed the downloader throughput audit and macOS storage reclassification pass. Onion-heavy batch downloads now promote mid-size files into the large-file lane, large-file batch summaries keep reporting real aggregate throughput, and macOS storage classification now uses `diskutil` mount-point fallback so Arti bootstrap sees Apple Fabric / NVMe hosts correctly.
- The March 10, 2026 live Qilin download audit also corrected the authoritative full-download baseline: the exact target's `best` snapshot currently holds `5078` entries (`4240` files / `838` folders) while `current` holds only `2926` (`2394` / `532`). Full download validation must use the `best` snapshot explicitly.
- The retained hidden-service downloader default is now mixed and benchmark-driven: keep the improved `12`-client Arti bootstrap, but keep hidden-service multi-file transfer lanes at `16/8/10/24`. Wider first-wave fan-out (`24/12/16/36`) improved bootstrap but did not improve useful-work throughput on the live Qilin target.
- Phase 90 completed the winner-quality and tail-latency biasing pass. Qilin now persists productive-winner quality, emits compact final tail summaries (`winner_host`, `slowest_circuit`, `late_throttles`, `outlier_isolations`), and adapts worker repin cadence from winner quality instead of a fixed interval.
- The March 10, 2026 exact-target live audit also exposed and fixed a real late-tail reconciliation bug. A degraded run kept reopening missing folders near `99.4%`, while the rebuilt rerun on `4xl2hta3...` finished cleanly in `213.52s` with the full `3180`-entry tree and zero failovers/timeouts.
- The remaining bottleneck is no longer hidden deadlock, generic throttle blindness, or missing tail visibility. It is warm-rerun winner stickiness: a fresh Stage A host can still pull the crawl away from a historically productive cached winner and create subtree-reroute churn later in the run.
- Phase 89 completed the deep-crawl stall audit and the first late-layer throttle/outlier repair pass. Qilin now bounds cached fast-path probing to two candidates, first-attempt `503/429/403/400` failures now hit shared throttle telemetry and circuit isolation, and final adapter logs now report effective entries when success comes through direct VFS streaming.
- The March 10, 2026 exact-target full crawls proved the deep-layer problem is route quality variance, not a hidden deadlock. The same `afa2a0ea-20ba-3ddf-8c5c-2aeea9e5dc43` target finished with the same `3180` effective entries in `139.86s` on winner `3pe26tqc...` and `487.71s` on winner `aay7nawy...`.
- The rebuilt-binary replay also confirmed that late throttles are now honest in the shared summary: the crawl surfaced `429/503=2 failovers=2`, logged both `503 Service Unavailable` child failures as `kind=throttle`, and healed them immediately with phantom swaps.
- The new governor-side stall guard did not fire on the live full crawls because progress never flatlined. That is the correct behavior; the next optimization target is winner-quality memory and tail-latency biasing, not a generic deadlock fix.
- Phase 88 completed the operator-surface parity pass. `binary_telemetry.rs`, `telemetry.proto`, and regenerated frontend protobuf bindings now carry the same resource-metrics payload, including `throttle_rate_per_sec`, `phantom_pool_depth`, `subtree_reroutes`, `subtree_quarantine_hits`, and `off_winner_child_requests`.
- The main binary now emits a compact `[summary:final]` line at crawl shutdown with `req=total/success/fail` and `subtree=reroutes/quarantine/offwinner`, so CLI logs expose route quality without raw frame inspection.
- A clean March 10, 2026 same-output exact-target validation proved subtree host-memory restore in practice. Run 1 on `afa2a0ea-20ba-3ddf-8c5c-2aeea9e5dc43` finished in `165.19s`, persisted `647` subtree host preferences, and produced `3180` discovered entries; run 2 reused the same output tree, logged `Restored 647 persisted subtree host preferences`, finished in `157.88s`, and matched the same `3180` discovered entries / `2533` files / `647` folders even though the durable winner rotated from `rbuio2ug...` to `ytbhximf...`.
- The next engineering target is no longer telemetry parity or restore proof. It is controlled degraded-route validation for the new subtree counters plus a small persistent redirect-freshness memory layer.
- Phase 87 made subtree route waste measurable and persistent in the safe direction: shared runtime metrics now expose `subtree_reroutes`, `subtree_quarantine_hits`, and `off_winner_child_requests`, and Qilin now persists subtree preferred hosts by host identity when the same host survives into a future run.
- Repeated March 10, 2026 exact-target reruns proved cross-run winner churn again (`chygwjfx...` -> `2wyohlh5...` -> `lqcxwo4c...`), which is why host-based subtree preferred-route persistence is now enabled.
- Main CLI behavior now keeps `--no-stealth-ramp` benchmark-only unless `CRAWLI_ALLOW_BENCHMARK_FLAGS=1`. The benchmark binary still owns the comparison path.
- Phase 86E fixed the next exact-target Qilin waste point in practice: subtree-aware host affinity and subtree-local standby quarantine now keep child-path failures separate from the global winner route.
- The March 10, 2026 subtree-affinity replay on `afa2a0ea-20ba-3ddf-8c5c-2aeea9e5dc43` held child traffic on the confirmed winner, eliminated off-winner child fetch/failure churn (`9/10` -> `0/0`), and reached `seen=544 processed=282 queue=262` with adapter-local `entries=2336`.
- `--no-stealth-ramp` remains benchmark-only. After root durability and subtree routing repairs, worker induction is not the primary default-path bottleneck.
- Phase 86D fixed the exact-target Qilin fail scenario in practice: winner leases are now root-parse-gated, phantom depletion reuses the live Arti swarm before cold bootstrap, root retries stay on the active seed, and first child retries no longer spill directly onto standby hosts.
- The repaired March 10, 2026 exact-target rerun now reaches a durable winner, parses the real QData root, expands `kent/` as `133 files / 133 folders`, and ends the timed `180.02s` window at `seen=288 processed=59 queue=229` with adapter-local progress reaching `entries=670`.
- Phase 86C reduced the live Arti/Qilin hot path further: strong URL-hint Qilin ingress now skips the blocking onion warmup, `MultiClientPool` seeds itself from already-hot swarm clients, and `CrawlerFrontier` refreshes live Arti clients before hinted onion execution.
- The March 10, 2026 exact-target reruns showed the global handoff to Qilin fall from `138.83s` to `71.08s` once the hinted-path warmup was removed. The same tranche also removed the old `~55s` `storage resolved -> first circuit hot` gap by avoiding a second cold Arti pool bootstrap.
- Residual live bottlenecks have moved beyond subtree standby churn itself. The next engineering work is controlled degraded-route validation for the new route counters and tighter redirect-freshness reuse when the cached winner rotates away.
- `CrawlerFrontier` now has an adapter-progress overlay (`pending`, `active_workers`, `worker_target`) so fast-path adapters can keep the shared crawl status plane truthful without changing the external GUI/CLI contract.
- Qilin now syncs its real request lifecycle into that overlay and drives runtime worker metrics from live request guards. The March 10, 2026 live CLI rerun showed the compact summary moving from `workers=0/0` to `workers=1/8` on the canonical Qilin target instead of remaining pinned at zero.
- The main binary now supports a compact operator summary mode: `--progress-summary --progress-summary-interval-ms <ms>`. It renders condensed `phase/progress/queue/workers/node/failovers` state from `telemetry_bridge_update` on the real application entrypoint.
- Final crawl shutdown now publishes a zeroed worker-metrics snapshot so CLI/GUI surfaces do not retain stale live worker counts after completion.
- Live GUI parity was revalidated on the actual Tauri window for the same Qilin target on March 10, 2026. The GUI path reached the same bootstrap, fingerprint, and Stage A rotated storage-discovery flow as the CLI path.
- The primary `crawli` binary now supports both GUI and headless CLI execution from the same entrypoint. No args launches the Tauri window; CLI subcommands run through `src-tauri/src/cli.rs` using the same managed `AppState`, backend commands, and Tauri event surface.
- The CLI is now the canonical headless path for main-program validation. Default stderr streaming mirrors operator-relevant app-native events, while high-frequency `telemetry_bridge_update` frames are opt-in via `--include-telemetry-events`.
- Single-file download and onion pre-resolve now expose reusable blocking helpers in `src-tauri/src/lib.rs` so CLI runs do not inherit GUI-only detached semantics.
- Live validation of the main binary against `http://ijzn3sicrcy7guixkzjkib4ukbiilwc3xhnmby4mcbccnsd7j2rekvqd.onion/site/view?uuid=f0668431-ee3f-3570-99cb-ea7d9c0691c6` successfully reached rotated storage-node discovery (`bw2eqn5sp5yhe64g...onion/4de7c659-b065-4207-b9ce-16013bac9054/`) and recursive child parsing under the actual storage tree.
- Phase 74E hardened frontend telemetry mapping: binary protobuf frames are now decoded with defaults and merged through normalization adapters before touching React state, preventing `Start Queue` renderer crashes from sparse proto3 payloads.
- The synthetic Qilin benchmark in `src-tauri/examples/qilin_benchmark.rs` now completes fully in both clean and hostile profiles across the `12/24/36` circuit matrix.
- Benchmark correctness depends on the sled VFS summary, and Qilin now waits for its UI/VFS batching path to drain before returning.
- Native Arti healing defaults are intentionally tighter than the earlier Phase 43/46 baseline: shorter probe cadence, lower anomaly threshold outside VM mode, and faster phantom-pool replenishment.
- Piece-mode resume now supports bounded contiguous-span coalescing in `src-tauri/src/aria_downloader.rs`, while preserving the persisted per-piece truth model used for safe restarts.
- A comprehensive CLI adapter test harness (`examples/adapter_test.rs`) is available for per-adapter live crawl verification. Run `cargo run --example adapter_test -- --help` for usage. It supports `--adapter`, `--all`, `--url` override, `--circuits`, `--timeout-seconds`, `--daemons`, and `--json` modes. The harness automatically classifies zero-entry results into failure categories with suggested remediation actions.
- Background tasks in Tauri `setup()` MUST use `tauri::async_runtime::spawn`, not `tokio::spawn`. The tokio reactor is not registered on the macOS main thread during `didFinishLaunching`.
- **Phase 52 (Mega.nz + Torrent):** The system now supports Mega.nz public links and BitTorrent magnet/`.torrent` inputs as first-class features. Two new modules: `mega_handler.rs` (AES-128-CTR via `mega` crate, recursive `Nodes::get_node_by_handle()` tree walking) and `torrent_handler.rs` (bencode parsing via `lava_torrent`, magnet parsing via `magnet_url` v3.0). Both produce canonical `FileEntry` structs. Auto-detection runs in `start_crawl` and via `detect_input_mode` Tauri command. Frontend has permanent Mega.nz / Torrent buttons with auto-detect.
- **Prevention Rule (PR-MEGA-001):** Never persist Mega.nz encryption keys to disk.
- **Prevention Rule (PR-TORRENT-001):** Never route BitTorrent traffic through Tor.
- **Prevention Rule (PR-MEGA-003):** When a dependency crate requires a conflicting major version of a shared crate, use Cargo's `package` rename feature.


---

## 1. System Identity and Objectives
OnionForge (codenamed `crawli`) is a cross-platform Tauri (Rust + React/TypeScript) desktop application. Its primary function is to bootstrap a native Arti circuit swarm in-process, route the Rust crawl/download hot path through a direct `ArtiClient` connector with explicit stream isolation, keep managed local SOCKS5 bridges only for compatibility surfaces that still need them (for example Ghost Browser / Chromium and some legacy examples), and systematically export massive deep-web `.onion` directories (specifically targeting complex single-page apps like Qilin and Play Ransomware) through the Rust downloader pipeline.

*   **Primary Constraint:** The system must run flawlessly on Mac, Linux, and Windows 10/11.
*   **Secondary Constraint:** The system must gracefully degrade and protect OS resources (e.g., Ephemeral Port Exhaustion, Mechanical HDD IOPS lockouts, RAM limitations).
*   **Visual Identity:** The React UI uses a hyper-modern "Cyber/Military" dark-mode aesthetic. 

---

## 2. Core Architecture Stack

### Backend Container (Tauri/Rust)
*   **Framework:** Tauri v2 configured with `tauri.conf.json`. 
*   **Asynchronous Engine:** `tokio` multi-threaded runtime (`tokio::spawn`, `Arc<Mutex>`).
*   **Networking:** Rust crawl/download traffic now uses the direct `ArtiClient` + `ArtiConnector` path. Normal TorForge bootstrap no longer starts managed SOCKS listeners; SOCKS remains only as an explicit compatibility layer for consumers that truly need it.
*   **Data Scaffolding Engine:** 
    *   For HTML/API Parsing: High-concurrency `crossbeam-queue` URL frontiers.
    *   For Downloading: In-process Rust downloader/orchestrator in `aria_downloader.rs` with managed Tor-port reuse and batch telemetry.
*   **In-Memory DB:** `sled` embedded KV store for recording Visited URLs and VFS arrays securely.

### Frontend Container (React/TypeScript)
*   **Build Tool:** Vite.
*   **Styling:** Pure CSS (`App.css`, `Dashboard.css`). NO Tailwind. Explicit emphasis on glassmorphism, glowing accents (`box-shadow`), dark grays (`#0a0a0a`), neon cyan/purple combinations (`#00e5ff`/`#8b5cf6`), and monospace system fonts (`JetBrains Mono`).
*   **State Management:** Standard React hooks (`useState`, `useEffect`) layered with Tauri IPC event listeners (`listen<T>`).
*   **Components:** Modularized structure (e.g., `VFSExplorer.tsx`, `Dashboard.tsx`). 

---

## 3. The 4-Pillar Pipeline Strategy
To recreate or modify this app, you must understand how data traverses the 4 pillars of the crawler:

#### Pillar 1: Bootstrapping & The Swarm (`tor.rs` / `tor_native.rs`)
The app creates exactly `N` native Arti `TorClient`s in-process. The Rust hot path consumes those clients directly through `ArtiClient`. Managed SOCKS5 bridges are no longer part of the default bootstrap path; they remain only as an explicit compatibility surface for Chromium/Ghost Browser and select legacy tooling.
*   **Prevention Rule:** Never hardcode ownership assumptions from fixed port ranges alone. Runtime callers must prefer the managed port registry, because the bootstrap can allocate ephemeral ports.
*   **Prevention Rule:** If a compatibility SOCKS bridge accepts username/password auth, it must convert that auth into explicit Arti isolation state rather than discarding it.
*   **Prevention Rule:** Circuit health must be measured with a real network probe through the live Arti client slot, not by timing `bootstrap()` on an already-initialized client.
*   **Implementation Note:** The current health probe target defaults to `check.torproject.org:443` and can be overridden with `CRAWLI_TOR_HEALTH_PROBE_HOST` / `CRAWLI_TOR_HEALTH_PROBE_PORT`.
*   **Implementation Note:** Native Arti timing/preemptive policy is now explicit rather than default-only. Operator overrides live behind `CRAWLI_ARTI_CONNECT_TIMEOUT_SECS`, `CRAWLI_ARTI_REQUEST_TIMEOUT_SECS`, `CRAWLI_ARTI_REQUEST_MAX_RETRIES`, `CRAWLI_ARTI_HS_DESC_FETCH_ATTEMPTS`, `CRAWLI_ARTI_HS_INTRO_REND_ATTEMPTS`, `CRAWLI_ARTI_PREEMPTIVE_THRESHOLD`, `CRAWLI_ARTI_PREEMPTIVE_MIN_EXIT_CIRCS`, `CRAWLI_ARTI_PREEMPTIVE_PREDICTION_LIFETIME_SECS`, and `CRAWLI_ARTI_PREEMPTIVE_PORTS`.

#### Pillar 2: The Frontier Scanner (`frontier.rs` & `adapters/`)
The user inputs a `.onion` URL. The `AdapterRegistry` hits the endpoint to read the HTTP Header and HTML Body (the `SiteFingerprint`). It matches this fingerprint to a specialized adapter (e.g., `qilin.rs`, `play.rs`, `autoindex.rs`).
The Adapter utilizes `tokio` workers to crawl the site, extracting `FileEntry` objects.
*   **Prevention Rule:** Some nodes capitalize protocols (`HTTP://`). ALWAYS use `.to_lowercase()` when parsing URLs so the router doesn't accidentally discard safe links.
*   **Prevention Rule (PR-PARSER-003):** If a target site transitions from a generic autoindex to a custom SPA structure (e.g., LockBit 5.0 transitioning to `<table id="list">`), do NOT modify parsing logic inside shared utility files. You must forcefully detach the site from the generic `AutoindexAdapter` and build a bespoke, deterministic DOM scraper specifically inside its adapter file (e.g., `adapters/lockbit.rs`).
*   **Prevention Rule (PR-EXPLORER-001):** The `AdaptiveUniversalExplorer` (Tier-4 intelligent fallback) performs speculative prefetch using `JoinSet`. The prefetch MUST BE capped at `max_prefetch = 6` links to avoid overwhelming the concurrent Tor circuits.
*   **Prevention Rule (PR-EXPLORER-002):** The Universal Explorer uses `TargetLedger` to store learned prefixes. Any previously successful URL prefixes MUST receive a massive heuristic score boost (e.g., `+1000`) for priority discovery to quickly re-traverse known structure on subsequent runs.
*   **Implementation Note:** Non-Qilin adapters no longer choose their own listing worker count. They must call `frontier.recommended_listing_workers()` so the swarm budget stays aligned across adapters.
*   **Implementation Note:** `qilin.rs` now runs a local adaptive page governor around directory enumeration. It classifies `429`/`503`, hidden-service circuit failures, generic HTTP failures, and timeouts separately and rebalances active HTML workers every few seconds instead of holding a fixed page-worker ceiling.
*   **Implementation Note:** `qilin_nodes.rs` is a persistent tournament cache, not a flat mirror list. It stores success/failure/cooldown state per storage host in sled, revalidates sticky winners first, then probes the tournament head before opening fallback candidates.

#### Pillar 3: The Virtual File System (`vfs.rs` & `VFSExplorer.tsx`)
Extracted paths are blasted over Tauri IPC to the React frontend, where they are mapped onto a TanStack generic virtualizer (`useVirtualizer`) to ensure the DOM doesn't lock up when 50,000 files are rendered.
*   **Prevention Rule:** Do not trust size headers perfectly. UI progress bars must rely on definitive `0 bps` backend completion signals instead of assuming a file is 100% finished just because bytes stream in without a `Content-Length`.

#### Pillar 4: The Storage Scaffolder (`aria_downloader.rs`)
When the user clicks "Download", the Rust backend intercepts the file array. It generates 0-byte structural placeholders for folders and then routes physical files through the in-process downloader pipeline, reusing managed Tor ports when the target is onion-backed.
*   **Prevention Rule:** The backend MUST fallback to sequential byte writes (instead of zero-copy `mmap`) if memory mapping fails. This protects users with 5400 RPM Mechanical HDDs from 100% disk usage lockouts.
*   **Prevention Rule:** Batch/onion download code must bootstrap a fresh managed Tor cluster when no active ports are registered, rather than assuming `9051` exists.
*   **Prevention Rule:** Portable release packaging must treat `src-tauri/bin/*` as optional legacy payloads; native-Arti builds do not require bundled Tor binaries.
*   **Prevention Rule:** Experimental download engines must not bypass production control semantics. If a path does not preserve `.ariaforge_state`, `DownloadControl`, and standardized telemetry, it stays lab-only.
*   **Implementation Note:** The current integrated-Arti architecture deliberately distinguishes metadata crawling from download pressure. When a session includes download work, the Qilin page governor reserves part of the swarm so HTML discovery does not starve transfer stages.
*   **Implementation Note:** `aria_downloader.rs` is the production downloader. It now caps handshake tournament width with live telemetry and gates active range fetchers through a BBR-managed startup window; `multipath.rs` is only for isolated experimentation.
*   **Implementation Note:** The repository includes a synthetic local benchmark harness in `src-tauri/examples/qilin_benchmark.rs`, but the main binary itself now has a first-class CLI mode for headless operator validation. Prefer `cargo run --manifest-path src-tauri/Cargo.toml -- <subcommand>` when validating the shipped program surface.
*   **Implementation Note:** Native runtime telemetry now lives in `src-tauri/src/runtime_metrics.rs`, while the hot operator plane is aggregated in `src-tauri/src/telemetry_bridge.rs`. The bridge publishes a single `telemetry_bridge_update` frame every 250ms (or slower when unchanged) that carries crawl status, resource metrics, aggregate batch progress, and per-file download deltas.
*   **Implementation Note:** If an adapter does meaningful work outside the generic frontier semaphore, it must push `pending / active / target` state into the frontier overlay so shared crawl status, CLI summaries, and GUI dashboards stay aligned.
*   **Implementation Note:** Machine-aware scaling now lives in `src-tauri/src/resource_governor.rs`. TorForge bootstrap caps, frontier permit caps, Qilin page-worker ceilings, downloader Direct I/O policy, small-file swarm width, tournament width, and initial active range budgets are all shaped from one shared CPU/RAM/storage-pressure model before operator overrides are applied.
*   **Implementation Note:** Optional binary telemetry now lives in `src-tauri/src/binary_telemetry.rs`. When `CRAWLI_PROTOBUF_TELEMETRY_PATH` is set, hot operator signals are mirrored into a length-delimited protobuf file sink; the bridge remains the canonical Tauri/UI surface.
*   **Implementation Note:** `start_crawl` no longer returns a full in-memory crawl result to the frontend. The canonical completion contract is `CrawlSessionResult`, and the sled VFS is the source of truth for summary/index/export/download follow-up work.
*   **Implementation Note:** Qilin now uses a bounded storage route plan: one primary storage seed plus a small standby set. Failover is sequential and classified; do not implement wide parallel destination fan-out as the default path.
*   **Implementation Note:** The authorized long-run validation entrypoint is `src-tauri/examples/qilin_authorized_soak.rs`. It is intended for explicit operator use only and writes a JSON report under `tmp/`.
*   **Implementation Note:** Simpler adapters such as Play, DragonForce, Pear, and LockBit inherit the governed crawl width through `frontier.recommended_listing_workers()`. Do not write tests or adapter code that assume `configured circuits == live page workers`.
*   **Implementation Note:** Runtime-loaded adapters now live behind `src-tauri/src/adapters/plugin_host.rs`. They are manifest-driven and host-owned: matching rules come from JSON, but crawl execution still delegates back into hardened Rust pipelines. The shipped skeleton manifest is `adapter_plugins/example_autoindex_plugin.json`.
*   **Implementation Note:** The system now maintains persistent per-target state in `src-tauri/src/target_state.rs`. Each normalized target URL maps to a deterministic `target_key`, stable current/best listing filenames under `<selected_output>/targets/<target_key>/`, a hidden support root under `<selected_output_parent>/.onionforge_support/<support_key>/`, and a durable download failure manifest.
*   **Implementation Note:** Repeat crawls are baseline-aware. The backend compares `raw_this_run_count`, `best_prior_count`, and `merged_effective_count`, then classifies the run as `first_run`, `matched_best`, `exceeded_best`, or `degraded`.
*   **Implementation Note:** Download resume planning is now failure-first. Known failed files are queued before the general missing/mismatch set from the authoritative best crawl snapshot, while exact-size matches are skipped.
*   **Implementation Note:** `src-tauri/src/tor_runtime.rs` is now TorForge-only. `Crawli` no longer exposes a runtime selector in the shipped code path.
*   **Implementation Note:** This does **not** mean `Crawli` directly links the `Tor Forge/loki-tor-core` crate. The current integration uses a TorForge-only bootstrap policy inside `Crawli`’s own Arti 0.39 runtime layer.
*   **Implementation Note:** Some older examples still exist to exercise compatibility SOCKS behavior deliberately. Treat them as legacy diagnostics, not as the canonical architecture.
*   **Implementation Note:** If you add new main-program backend functionality, mirror it into `src-tauri/src/cli.rs` unless there is a concrete reason it must remain GUI-only. The CLI is part of the primary product surface now.
*   **Implementation Note:** For long live crawls, prefer the main-binary CLI summary mode (`--progress-summary`) over raw telemetry-frame inspection unless you are diagnosing the transport layer itself.

---

## 4. UI / UX Design Guidelines

If generating new frontend React code, strictly adhere to these rules:

1.  **Colors:** 
    *   Primary Background: `#0f1014`
    *   Panel Background: `rgba(20, 22, 28, 0.7)` with `backdrop-filter: blur(12px)`
    *   Accent Primary: `#a200ff` (Deep Purple)
    *   Accent Secondary: `#00e5ff` (Neon Cyan)
    *   Text: `#e2e8f0` (Main), `#94a3b8` (Muted)
2.  **Typography:** Use sans-serif for UI elements (`Inter`, `system-ui`) and heavily utilize `JetBrains Mono` for ALL numbers, logs, paths, and statuses.
3.  **Components:** Use `lucide-react` for iconography. Build custom animated loaders (e.g., `VibeLoader.tsx`), relying on `@keyframes` rather than static SVGs for scanning indiciators.
4.  **No Placeholders:** If you must simulate data in the UI without a backend, construct a static fixture file (like `vfsFixture.ts`) and inject it gracefully. Do not write generic "Hello World" placeholder blocks. Everything must look premium and dense.

---

## 5. Development Rituals & Edge Cases

When editing Rust logic, remember these historical constraints:
*   **Windows Process Limits:** Windows has a strict TCP `MaxUserPort` limit (~16,000). Uncapped HTTP requests will blue-screen the network adapter. Keep async workers clamped (e.g., max 60 workers per adapter).
*   **Tor TLS Fingerprinting:** Cloudflare and Nginx will drop connections if the `reqwest` TLS client acts like a bot. You must bind `rustls` instead of `native-tls` inside the `cargo` build and mimic standard browser headers.
*   **Deadlocks:** You must release `Mutex` guards *before* triggering `await` in `tokio`, otherwise the async runtime thread will permanently freeze waiting for data chunks.
*   **Adaptive JWT Iframe Parsing (DragonForce):** Do not attempt bare-metal API calls against Deepweb architectures protected by tokenized Next.js wrappers. Use standard `scraper::Selector` tools to capture the `<iframe>` bridging URLs and inject them back into `CrawlerFrontier`. This offloads authentication logic back to Tor.
*   **Adapter Polyfill Delegation (Qilin):** When encountering ransomware sites utilizing custom CSS template frameworks ("QData") masking standard HTML tables, create an adapter isolated purely to the fingerprint detection step (e.g. `body.contains("QData")`). Do not build a custom scraper. In `crawl()`, delegate execution immediately back to the master `<AutoindexAdapter as CrawlerAdapter>::crawl` generic framework. Every custom scraper logic tree requires rigorous unit testing boundaries, avoid code sprawl.

By internalizing this document, you possess the context necessary to forge new adapters and structural improvements without compromising the system's foundational stability.


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

### Phase 61: Tauri Asynchronous Reactor Deadlock Fix (2026-03-08)
**System Audit & Verification:** The UI experienced permanent deadlocks when fetching `.onion` storage parameters during live execution, whereas raw terminal config tests demonstrated 100% correctness.
**Architectural Upgrade:**
1. **Tokio Panic Eradication:** Tauri executes its Event IPC `#[tauri::command]` functions outside of Tokio's asynchronous MT (multi-thread) reactor context. Attempting to force synchronous locks via `tokio::sync::RwLock` backed by `tokio::task::block_in_place` caused a silent thread panic within the UI connector, instantly freezing the crawl.
2. **Synchronous Locking Transition:** Purged asynchronous locks from the `PhantomPool` and `TorClientSlot`. Native standard library `std::sync::RwLock` primitives are now the canonical standard for shared configuration memory.
3. **Strict Structural Scoping:** Enforced hard `{ }` closure bounds across all background monitoring routines to guarantee that standard `!Send` lock-guards vanish entirely before reaching `.await` pause execution blocks.

**Prevention Rule Enforced:** `PR-TAURI-RUNTIME-001`: Never use `tokio::sync::RwLock` for primitive configurations initialized or handled directly by Tauri IPC hooks. Use `std::sync::RwLock`.

### Phase 61b: Storage Discovery Timeout Hardening (2026-03-08)
**Root Cause:** Even after fixing the RwLock deadlock, the UI continued to hang at "Probing Target". The real stall was in `qilin_nodes.rs::discover_and_resolve()` — the 4-stage storage node discovery pipeline had **zero global timeout**. Stage A (3 HTTP retries) and Stage B (1 HTTP call) lacked per-request timeouts. Stage D probed up to 17 cached mirrors × 15s each. With degraded Tor circuits, the pipeline blocked for 4+ minutes.

**3-Layer Timeout Fix:**
1. **Global Timeout (90s):** Wrapped the entire `discover_and_resolve()` call in `qilin.rs` with `tokio::time::timeout(90s)`. On expiry, falls through to the Phase 42 direct-mirror retry logic.
2. **Per-Stage HTTP Timeouts (20s):** Stage A and B HTTP calls now use `tokio::time::timeout(20s)`.
3. **Reduced Probe Timeouts:** `PROBE_TIMEOUT_SECS` 15→10, `PREFERRED_NODE_TIMEOUT_SECS` 8→6.

**Prevention Rule Enforced:** `PR-CRAWLER-012`: Every HTTP call through Tor circuits MUST have an explicit `tokio::time::timeout` wrapper. Never rely on Tor's built-in connection timeout alone.

### Phase 61b+: Stage D Batch Timeout & Discovery Progress (2026-03-08)
Added 30-second batch timeouts to Stage D tournament head and tail JoinSet drains. Added `emit_discovery_progress()` emitter for per-stage UI visibility during "Probing Target". The `discover_and_resolve()` function now accepts an optional `AppHandle` parameter for progress emissions. Maximum discovery worst-case reduced from 255s to ~60s for Stage D alone.

### Phase 67B+C: Live Crawl Bottleneck Fixes (2026-03-08)
**Phase 67B — MultiClientPool Size Separation:**
`circuits_ceiling=120` was being passed directly to `MultiClientPool::new(120)`, creating 120 independent TorClients and consuming the entire 5-minute crawl window with zero entries discovered. Fix: Separated pool size (capped at 8, `CRAWLI_MULTI_CLIENTS` env var override) from `circuits_ceiling` (worker budget). Pool now creates 8 TorClients regardless of circuit budget.

**Phase 67C — Governor Worker Scaling:**
`QilinCrawlGovernor` received `available_clients=1` (frontier's single bootstrap client) instead of the MultiClientPool size (8). This capped `max_active=4` and `desired_active=4`, limiting visible workers to 2. Fix: Compute `governor_pool_size` using the same env var logic. Now `available_clients=8 → max_active=12 → desired_active=6`.

**Prevention Rules:**
- `PR-POOL-001`: NEVER use `circuits_ceiling` as TorClient pool size. Pool size must be capped at 8 and controlled separately via `CRAWLI_MULTI_CLIENTS`.
- `PR-POOL-002`: Always run live `.onion` crawl tests after optimization changes. Synthetic benchmarks miss infrastructure-level bottlenecks.
- `PR-GOV-001`: Always initialize governor with the actual pool size, not the frontier's bootstrap client count.

### Phase 67D: Throttle-Adaptive Governor (2026-03-08)
Three-layer adaptive throttle system: (1) Per-worker 2s cool-off after 503/429, (2) Reactive `acquire_slot()` halving within 5s of any throttle via shared `AtomicU64` timestamp, (3) Graduated re-escalation (+1 instead of +4) for 30s post-throttle. Live test: **zero 503s** (vs 4×503 pre-67D). Memory dropped from 318→264MB.

**Prevention Rules:**
- `PR-THROTTLE-001`: Per-worker cool-off required for rate-limited servers, not just queue-level backoff.
- `PR-THROTTLE-002`: Governor re-escalation must be graduated (+1) during cooldown window, not full-speed (+4).
