# OnionForge: AI Engineering & Context Reference
> **Last Updated:** 2026-03-06T22:15 CST
> **Version:** 2.1.7
> **Authors:** Navi (User), Antigravity (AI)

This document serves as the master blueprint for any AI agent tasked with maintaining, extending, or recreating the OnionForge intelligence gathering application. It contains all critical architectural decisions, environment constraints, GUI styling instructions, and API behavioral knowledge required to build this system from scratch without guessing.

Current repo state note:
- The synthetic Qilin benchmark in `src-tauri/examples/qilin_benchmark.rs` now completes fully in both clean and hostile profiles across the `12/24/36` circuit matrix.
- Benchmark correctness depends on the sled VFS summary, and Qilin now waits for its UI/VFS batching path to drain before returning.
- Native Arti healing defaults are intentionally tighter than the earlier Phase 43/46 baseline: shorter probe cadence, lower anomaly threshold outside VM mode, and faster phantom-pool replenishment.
- Piece-mode resume now supports bounded contiguous-span coalescing in `src-tauri/src/aria_downloader.rs`, while preserving the persisted per-piece truth model used for safe restarts.
- A comprehensive CLI adapter test harness (`examples/adapter_test.rs`) is available for per-adapter live crawl verification. Run `cargo run --example adapter_test -- --help` for usage. It supports `--adapter`, `--all`, `--url` override, `--circuits`, `--timeout-seconds`, `--daemons`, and `--json` modes. The harness automatically classifies zero-entry results into failure categories with suggested remediation actions.
- Background tasks in Tauri `setup()` MUST use `tauri::async_runtime::spawn`, not `tokio::spawn`. The tokio reactor is not registered on the macOS main thread during `didFinishLaunching`.

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
*   **Implementation Note:** The repository includes a synthetic local benchmark harness in `src-tauri/examples/qilin_benchmark.rs`. Use it to measure parser/frontier/governor changes safely before considering any live target validation.
*   **Implementation Note:** Native runtime telemetry now lives in `src-tauri/src/runtime_metrics.rs`, while the hot operator plane is aggregated in `src-tauri/src/telemetry_bridge.rs`. The bridge publishes a single `telemetry_bridge_update` frame every 250ms (or slower when unchanged) that carries crawl status, resource metrics, aggregate batch progress, and per-file download deltas.
*   **Implementation Note:** Machine-aware scaling now lives in `src-tauri/src/resource_governor.rs`. TorForge bootstrap caps, frontier permit caps, Qilin page-worker ceilings, downloader Direct I/O policy, small-file swarm width, tournament width, and initial active range budgets are all shaped from one shared CPU/RAM/storage-pressure model before operator overrides are applied.
*   **Implementation Note:** Optional binary telemetry now lives in `src-tauri/src/binary_telemetry.rs`. When `CRAWLI_PROTOBUF_TELEMETRY_PATH` is set, hot operator signals are mirrored into a length-delimited protobuf file sink; the bridge remains the canonical Tauri/UI surface.
*   **Implementation Note:** `start_crawl` no longer returns a full in-memory crawl result to the frontend. The canonical completion contract is `CrawlSessionResult`, and the sled VFS is the source of truth for summary/index/export/download follow-up work.
*   **Implementation Note:** Qilin now uses a bounded storage route plan: one primary storage seed plus a small standby set. Failover is sequential and classified; do not implement wide parallel destination fan-out as the default path.
*   **Implementation Note:** The authorized long-run validation entrypoint is `src-tauri/examples/qilin_authorized_soak.rs`. It is intended for explicit operator use only and writes a JSON report under `tmp/`.
*   **Implementation Note:** Simpler adapters such as Play, DragonForce, Pear, and LockBit inherit the governed crawl width through `frontier.recommended_listing_workers()`. Do not write tests or adapter code that assume `configured circuits == live page workers`.
*   **Implementation Note:** Runtime-loaded adapters now live behind `src-tauri/src/adapters/plugin_host.rs`. They are manifest-driven and host-owned: matching rules come from JSON, but crawl execution still delegates back into hardened Rust pipelines. The shipped skeleton manifest is `adapter_plugins/example_autoindex_plugin.json`.
*   **Implementation Note:** The system now maintains persistent per-target state in `src-tauri/src/target_state.rs`. Each normalized target URL maps to a deterministic `target_key`, stable current/best listing filenames in the selected output root, JSON snapshots under `temp_onionforge_forger/targets/<target_key>/`, and a durable download failure manifest.
*   **Implementation Note:** Repeat crawls are baseline-aware. The backend compares `raw_this_run_count`, `best_prior_count`, and `merged_effective_count`, then classifies the run as `first_run`, `matched_best`, `exceeded_best`, or `degraded`.
*   **Implementation Note:** Download resume planning is now failure-first. Known failed files are queued before the general missing/mismatch set from the authoritative best crawl snapshot, while exact-size matches are skipped.
*   **Implementation Note:** `src-tauri/src/tor_runtime.rs` is now TorForge-only. `Crawli` no longer exposes a runtime selector in the shipped code path.
*   **Implementation Note:** This does **not** mean `Crawli` directly links the `Tor Forge/loki-tor-core` crate. The current integration uses a TorForge-only bootstrap policy inside `Crawli`’s own Arti 0.39 runtime layer.
*   **Implementation Note:** Some older examples still exist to exercise compatibility SOCKS behavior deliberately. Treat them as legacy diagnostics, not as the canonical architecture.

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
