Version: 1.0.1
Updated: 2026-03-03
Authors: Navi (User), Codex (GPT-5)
Related Rules: [MANDATORY-L1] Living Documents, [MANDATORY-L1] Workflow, [MANDATORY-L1] Testing & Validation

# Summary
Recreation guide for AI and engineers to rebuild the current `crawli` behavior, including backend flow, UI state, and telemetry contracts.

# Context
This file is a technical “rebuild bible” for deterministic reimplementation.

# Analysis
System architecture:
- Frontend: React + TypeScript (`src/*`).
- Native shell/backend: Tauri + Rust (`src-tauri/*`).
- Core crawl loop: adapter-driven recursion over `CrawlerFrontier`.
- Download path: Aria-backed mirror/download pipeline.

# Details
Rebuild checklist:
1. Initialize frontend shell and dashboard.
2. Implement Tauri commands:
   - `start_crawl`, `cancel_crawl`, `download_files`, `download_all`, `initiate_download`, `get_vfs_children`, `get_adapter_support_catalog`.
3. Implement adapter registry and fingerprint matching.
4. Implement `CrawlerFrontier`:
   - URL dedupe, client pool, semaphore, AIMD, cancellation flags, metrics.
5. Implement recursive autoindex crawler:
   - URL join-based recursion, subtree scope checks, queue accounting guard.
6. Emit events:
   - `crawl_log`, `crawl_progress`, `crawl_status_update`, download events.
7. Implement dashboard progress UI:
   - Consume `crawl_status_update` and render 0–100 progress bar plus metrics.
8. Validate with:
   - `cargo test`
   - `npm run build`

Current event contract (`crawl_status_update`):
- `phase: string`
- `progressPercent: number`
- `visitedNodes: number`
- `processedNodes: number`
- `queuedNodes: number`
- `activeWorkers: number`
- `workerTarget: number`
- `etaSeconds: number | null`
- `estimation: string`

## Advanced Rebuild Instructions (HFT / Lock-Free Paradigm)
This architecture actively enforces a high-throughput, lock-free paradigm. Any future engineering or rebuilding MUST strictly implement these patterns:
1. **Concurrency Control:** Prefer BBR (Bottleneck Bandwidth and RTT) models over simplistic AIMD routines.
2. **Predictive Latency:** Use Extended Kalman Filters (EKF) to continuously model network jitter.
3. **Data Integrity:** Implement Merkle-Tree BFT consensus for chunking. Large downloads must never drop entirely due to localized Byzantine failures.
4. **I/O Subsystem:** Treat disk writes as a unified sequential stream fed by an LMAX Disruptor Ring Buffer. A single dedicated I/O worker consumes the buffer, removing Mutex bottlenecks across Tor workers.
5. **Memory-Mapped (mmap) Writes:** To achieve maximum Mechanical HDD compatibility during massive concurrent Tor downloads, the I/O consumer MUST implement `memmap2` zero-copy allocations in `aria_downloader.rs`. Native OS Page Catching prevents hardware IOPS limits from cracking the pipeline.
6. **Adaptive Circuit Evasion:** Hardcoded `--ExitNodes` are obsolete. Rebuilds must utilize `--ControlPort` bindings combined with `CookieAuthentication 1`. Upon detecting HTTP 429 penalties or TCP Resets, the engine MUST fire a Hex-authenticated TCP `SIGNAL NEWNYM` to force an immediate, zero-downtime routing rotate without sacrificing the existing listener sockets.
7. **Adaptive JWT Iframe Parsing:** When extracting endpoints from SPA frameworks like Next.js that utilize volatile JWT-authentic### Adaptive JWT Iframe Parsing (DragonForce)
Do not attempt bare-metal API calls against Deepweb architectures protected by tokenized Next.js wrappers. Use standard `scraper::Selector` tools to capture the `<iframe>` bridging URLs and inject them back into `CrawlerFrontier`. This offloads authentication logic back to Tor.

### Adapter Polyfill Delegation (Qilin)
When encountering ransomware sites utilizing custom CSS template frameworks ("QData") masking standard HTML tables, create an adapter isolated purely to the fingerprint detection step (e.g. `body.contains("QData")`). Do not build a custom scraper. In `crawl()`, delegate execution immediately back to the master `<AutoindexAdapter as CrawlerAdapter>::crawl` generic framework. Every custom scraper logic tree requires rigorous unit testing boundaries, avoid code sprawl.
8. **Testing & Integrity:** Any Lock-Free transition must maintain strictly identical Cancellation and Tor lifecycle teardowns.

# Prevention Rules
**1. Keep event payload schemas versioned and synchronized across Rust + TS.**
**2. Never implement native process/circuit behaviors in frontend-only code.**
**3. Preserve cancellation semantics across crawl workers, download workers, and Tor cleanup.**
**4. Require test pass + frontend build pass before claiming parity.**
**5. Update this file whenever core flow, IPC, or state contracts change.**

# Risk
- Unsynced event contracts cause silent UI regressions.
- Missing cancellation propagation can leak background resources.

# History
- 2026-03-03: Initial recreation baseline authored.

# Appendices
- Core files:
  - `src-tauri/src/lib.rs`
  - `src-tauri/src/frontier.rs`
  - `src-tauri/src/adapters/*`
  - `src/App.tsx`
  - `src/components/Dashboard.tsx`
