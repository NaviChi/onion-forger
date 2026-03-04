Version: 1.0.4
Updated: 2026-03-03
Authors: Navi (User), Codex (GPT-5)
Related Rules: [CRITICAL-L0] Native/Web Boundary, [MANDATORY-L1] Prevention Discipline, [MANDATORY-L1] Testing & Validation

# Summary
GUI issue ledger for crawl observability and adapter/operator visibility.

# Context
Reported UX gap: no explicit crawl-completion percentage while recursive crawling was active.

# Analysis
Observed GUI issues:
- Dashboard had phase/network telemetry but no dedicated crawl progress bar.
- No direct display of backend queue/worker estimate metrics in the progress area.
- Crawl progress did not transition into structured download progress during mass mirror.
- Download telemetry lacked operator-level file counters, ETA, and timing visibility.
- `NETWORK I/O` could display `0.00 MB/s` and `0.00 MB` during active batch phases because it only read per-file stream events.
- Support popover could show stale adapter capability labels when backend adapter behavior changed.
- `NETWORK I/O` could still drop to `0.00 MB/s` between sparse batch events when payload speed was missing/zero.

# Details
Issue-to-fix mapping:
- Issue: Missing 0–100 crawl progress surface.
  - Fix: Added backend-driven `crawl_status_update` listener in `App.tsx`.
  - Fix: Added progress card in `Dashboard.tsx` with percent, phase, ETA, and worker/queue counts.
  - Fix: Added visual progress bar styles in `Dashboard.css`.
- Issue: Progress state lifecycle ambiguity.
  - Fix: Reset progress state at crawl start and transition on final backend phase (`complete`, `cancelled`, `error`).
- Issue: No dedicated batch download progress mode.
  - Fix: Added `download_batch_started` and `batch_progress` listeners in `App.tsx` with aggregate state.
  - Fix: Dashboard progress card now automatically swaps to download mode and shows totals, downloaded, failed, remaining, elapsed, ETA, throughput, and current file.
- Issue: `NETWORK I/O` card showed zero throughput/total during active batch routing.
  - Fix: Merge telemetry sources in `Dashboard.tsx` (per-file stream stats + batch aggregate + hint-based fallback).
  - Fix: Add payload-key compatibility in `App.tsx` for both camelCase and snake_case batch event fields.
- Issue: Throughput could transiently flatline between sparse batch telemetry frames.
  - Fix: Add frontend delta-based batch speed fallback in `App.tsx` (`downloadedBytes` sample window) when backend speed is unavailable.
  - Fix: Keep fallback reset aligned with `download_batch_started` and crawl restart state reset.
- Issue: Support panel labels for LockBit/Nu were stale (`Detection Only`) after backend crawl delegation was enabled.
  - Fix: Align fallback support catalog entries in `App.tsx` with backend support catalog (`Full Crawl` + updated sample/test metadata).
- Issue: The frontend visual aesthetic felt disjointed during operations due to monolithic React `lucide` spinners.
  - Root Cause: Default CSS rotation algorithms on standard SVG paths lack the premium, zero-latency "SnoozeSlayer" visual weight.
  - Fix: Implemented `<VibeLoader />` wrapping 8-bit true-alpha Animated WebP cinematic sequences. Designed strict CSS fallback states preserving `-webkit-optimize-contrast` halo-free rendering.
- Issue (Theoretical/HFT): Rapid circuit routing and Thompson Sampling algorithm updates causes UI throughput charts to jitter too aggressively.
  - Root Cause: high-frequency bandwidth sampling feeds directly into UI telemetry.
  - Fix (Proposed): Ensure UI telemetry integrates robust EMA (Exponential Moving Average) smoothing at the React layer, decoupling raw sampling rate from rendering rate.

# Prevention Rules
**1. Progress visuals must bind to backend telemetry events, not inferred log strings.**
**2. New dashboard metrics must degrade cleanly to zero-state when backend events are absent.**
**3. Keep native runtime controls (window/process/IPC) in Tauri backend, not DOM hacks.**
**4. Any new UI control must include deterministic `data-testid` when relevant to overlay/integrity tests.**
**5. Event schema changes require frontend type updates in the same change set.**
**6. Progress cards must switch mode based on backend phase events, not log-order heuristics alone.**
**7. Throughput/byte counters must combine stream and batch channels, with hint-based fallback when only aggregate progress is available.**
**8. Support-popover fallback metadata must stay in lockstep with backend support catalog semantics.**
**9. Batch speed rendering must include a delta-based fallback for sparse or partial backend payloads.**
**10. (HFT Standard) Rapidly oscillating telemetry must be smoothed (EMA) in the UI state layer to prevent visual jitter without throttling the backend.**

# Risk
- Estimated progress may briefly plateau in highly dynamic directory trees.
- Additional dashboard card increases visual density; acceptable for operator mode.

# History
- 2026-03-03: Initial GUI issue/fix baseline.
- 2026-03-03: Added merged network telemetry fallback for batch-heavy download phases.
- 2026-03-03: Synced support-popover adapter capabilities with backend adapter behavior.
- 2026-03-03: Added delta-based frontend throughput fallback for sparse batch telemetry updates.

# Appendices
- Validation:
  - `npm run build` (TypeScript + Vite build passing)
