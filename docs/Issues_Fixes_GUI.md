> **Last Updated:** 2026-03-06T16:40 CST

## Phase 1.0.8: Target Baseline Status and Failure-First Queue Visibility (2026-03-06)

### Issues Found
- Operators could not see whether the latest crawl matched, exceeded, or degraded relative to the best known result for the same target
- The UI exposed manual resume-index selection, but it did not make the new automatic per-target baseline behavior visible
- Download telemetry showed batch progress, but not whether the next queue was built from failures first, missing/mismatch files, or all-skipped completion

### Fixes Implemented
- Added persisted crawl baseline state to the frontend result contract and rendered it in a dedicated dashboard surface
- Added download resume-plan state to the frontend and surfaced `failures first`, `missing/mismatch`, `skipped exact`, and `all items skipped` counts in the download-progress area
- Relabeled manual resume-index selection as an advanced baseline override instead of the primary path
- Added stable listing file-path logging so operators can locate the per-target current/best artifacts directly in the selected output folder

### Prevention Rules
**17. Repeat-crawl UX must show baseline outcome (`first`, `matched`, `exceeded`, `degraded`) explicitly; operators should not infer it from raw node counts alone.**
**18. Automatic baseline behavior must remain the default path, with manual resume-index selection presented as an advanced override.**
**19. Download queue telemetry must distinguish failures-first planning from ordinary batch progress.**

## Phase 1.0.7: Resource Telemetry Dashboard and Fixture Coverage (2026-03-06)

### Issues Found
- Operators could see bandwidth/circuit metrics, but not Crawli process CPU usage, process RSS, or whole-system RAM pressure during heavy sessions
- The progress surface still made it easy to confuse circuit budget with live Qilin worker target
- Browser fixture mode had no representation of the new operator telemetry surface, so Playwright could not validate it

### Fixes Implemented
- Added `resourceMetrics` state to `App.tsx` and subscribed it to the backend `resource_metrics_update` event
- Added a dedicated dashboard operator card in `Dashboard.tsx` for process CPU, process RSS, system RAM, active workers/target, active/peak circuits, node host, failovers, throttles, and timeouts
- Added deterministic fixture telemetry in `vfsFixture.ts` and Playwright coverage for zero-state plus active-state operator rendering
- Updated crawl-finish handling in `App.tsx` to consume the compact `CrawlSessionResult` contract instead of expecting a full returned file array

### Prevention Rules
**14. Operator load telemetry must come from backend resource events, not from inferred frontend timing or bandwidth counters.**
**15. Fixture mode must include any new critical dashboard surface that is expected to be covered by Playwright.**
**16. If the backend changes a command return shape, the frontend contract and browser tests must change in the same patch.**

Version: 1.0.7
Updated: 2026-03-06
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
- Windows UI could surface raw canonical path prefixes (`\\?\X:\...`) in progress fields.
- Download progress could appear stuck when only file-count progress moved but byte transfer continued.
- Operators lacked explicit active-circuit and peak throughput ceilings on the dashboard.

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
- Issue: Windows canonical path prefixes leaked into download progress UI.
  - Fix: normalize display paths in `App.tsx` by stripping Windows verbatim prefixes and rendering root-relative paths.
- Issue: Progress bar looked frozen while backend still downloaded data.
  - Fix: switched download progress fill model from file-count only to `max(filePercent, bytePercent)` with cumulative byte telemetry.
- Issue: Operators could not quickly see active circuit load or observed ceilings.
  - Fix: added `active/peak circuits`, `peak bandwidth`, and `current/peak disk I/O` metrics to the dashboard network cards.
- Issue: Throughput and ETA values could oscillate heavily on sparse batch telemetry, reducing operator trust.
  - Root Cause: raw instantaneous speed was rendered directly and ETA confidence was implicit.
  - Fix: added EWMA speed smoothing (`smoothedSpeedMbps`) and explicit `etaConfidence` scoring in `App.tsx`, then surfaced both in `Dashboard.tsx`.
- Issue: Support panel labels for LockBit/Nu were stale (`Detection Only`) after backend crawl delegation was enabled.
  - Fix: Align fallback support catalog entries in `App.tsx` with backend support catalog (`Full Crawl` + updated sample/test metadata).
- Issue: The frontend visual aesthetic felt disjointed during operations due to monolithic React `lucide` spinners.
  - Root Cause: Default CSS rotation algorithms on standard SVG paths lack the premium, zero-latency "SnoozeSlayer" visual weight.
  - Fix: Implemented `<VibeLoader />` wrapping 8-bit true-alpha Animated WebP cinematic sequences. Designed strict CSS fallback states preserving `-webkit-optimize-contrast` halo-free rendering.
- Issue: Rapid backend routing updates caused UI throughput labels to jitter.
  - Root Cause: high-frequency bandwidth sampling fed directly into UI telemetry without smoothing.
  - Fix: implemented EMA/EWMA smoothing in React state and rendered both instant and smoothed throughput for operator context.
- Issue: The dashboard listened to four separate hot telemetry events, which increased renderer wakeups and made schema migration harder.
  - Root Cause: crawl, resource, batch, and per-file download telemetry were added independently over time, so `App.tsx` accumulated multiple event listeners and duplicated normalization logic.
  - Fix: switched the frontend to a single `telemetry_bridge_update` listener and moved the batch/download normalization into shared reducer-style helpers inside `App.tsx`.

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
**11. Display-path rendering must sanitize OS-specific canonical prefixes before binding to UI text or keys.**
**12. Download progress bars must blend file-count and byte-count signals to avoid false plateaus.**
**13. ETA displays must include confidence signaling when totals/speeds are estimate-driven.**
**14. High-frequency operator UI state must prefer one aggregated bridge event over multiple parallel hot listeners.**
**15. Overlay integrity geometry checks must distinguish true layout shifts from internal scroll-container translation.**
**16. Dynamic popovers/menus used in integrity tests must be reopened deterministically before child-control interaction.**

# Risk
- Estimated progress may briefly plateau in highly dynamic directory trees.
- Additional dashboard card increases visual density; acceptable for operator mode.

# History
- 2026-03-03: Initial GUI issue/fix baseline.
- 2026-03-03: Added merged network telemetry fallback for batch-heavy download phases.
- 2026-03-03: Synced support-popover adapter capabilities with backend adapter behavior.
- 2026-03-03: Added delta-based frontend throughput fallback for sparse batch telemetry updates.
- 2026-03-03: Added Windows path normalization, byte-aware progress fill, and active/peak circuit+throughput telemetry.
- 2026-03-04: Added EWMA throughput smoothing and explicit ETA confidence telemetry to stabilize download operator readouts.
- 2026-03-06: Migrated the dashboard to `telemetry_bridge_update`, consolidating crawl/resource/batch/download listeners into one aggregated operator-plane feed.
- 2026-03-07: Hardened the overlay integrity harness so internal `.app-container` scrolling and support-popover re-entry no longer produce false UI regressions.

# Appendices
- Validation:
  - `npm run build` (TypeScript + Vite build passing)
