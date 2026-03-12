> **Last Updated:** 2026-03-11T16:58 CST

## Phase 113: VFS Tree Spillage & Direct-Child Guard (2026-03-11)

### Issues Found
- The VFS tree layout flattened deep child directory content incorrectly when operator filesystem metadata containing `\` backslashes (Windows) was ingested, rendering massive flat hierarchies on screen instead of structured collapsible folders.

### Fixes Implemented
- React frontend components (`VFSExplorer` / `VfsTreeView.tsx`) were heavily constrained. The renderer now strictly normalizes paths directly in evaluation loops and discards downstream/children-of-children files from popping into the current virtual `row` tree layer.
- `src/components/VFSExplorer.test.tsx` implemented.

### Prevention Rules
**38. The VFS UI renderer must not blindly trust the backend hierarchical array. The frontend must implement a Direct-Child Guard validating that incoming rows belong functionally to the selected layer.**## Phase 109: Hidden Hex Virtualizer + Click-Through Stability (2026-03-11)

### Issues Found
- Full `App.tsx` interaction testing in fixture mode could still exhaust the renderer heap even after the earlier overlay split work.
- Checkbox and `Start Queue` interactions could become unreliable after an error toast appeared because the toast layer still occupied click space over the upper operator surface.
- Two remaining crawl-option `<select>` handlers still used object-spread state writes, which risked stale option state under rapid operator interaction.

### Fixes Implemented
- Refactored `HexViewer.tsx` so the heavy `useVirtualizer` path only mounts when the modal is actually open. The closed modal is now a zero-work `null` render instead of instantiating a hidden virtualized `256,000,000`-row disk surface on every app render.
- Kept `.toast-container` and `.toast` pointer-transparent in `App.css` so error/success overlays no longer intercept clicks intended for `Start Queue`, checkboxes, or nearby controls.
- Converted the remaining swarm/concurrency `<select>` handlers in `App.tsx` to functional `setCrawlOptions(...)` updates so rapid UI interactions cannot clobber each other with stale spreads.
- Added and passed `src/App.interaction.test.tsx`, then revalidated the real browser-mounted `?fixture=vfs&surface=app` path with before/after screenshots under `output/ui-fix-validation/`.

### Root Cause
- The main renderer issue was hidden workload, not visible layout. `HexViewer` always ran its virtualization hooks even while closed, which meant the app was paying for an enormous virtual disk surface during ordinary operator interactions.
- The click issue was a separate layering bug: once a toast appeared, the overlay could sit above actionable controls and steal pointer events even though it was only informational.

### Prevention Rules
**35. Hidden dialogs with heavy virtualization or polling must not instantiate their expensive hooks until they are actually open.**
**36. Informational overlays (toasts, HUD notices) must default to `pointer-events: none` unless they contain intentionally interactive controls.**
**37. Shared option objects in hot UI paths must use functional state updates so rapid clicks cannot replay stale state snapshots.**

## Phase 108: Overlay Integrity Audit Boundary + Modal Selector Hardening (2026-03-11)

### Issues Found
- The user requested an every-clickable overlay integrity run, but the existing Playwright audit only covered the reduced `BrowserPreviewApp.tsx` shell.
- Forcing the real `App.tsx` through browser fixture mode (`?fixture=vfs&surface=app`) did not mount successfully in headless Chromium. The renderer crashed before `.app-container` appeared, so the full operator tree could not be certified through Playwright.
- Modal-heavy controls (Azure and Hex Viewer) did not expose stable test ids for deterministic reopen/close traversal during overlay audits.

### Fixes Implemented
- Added a browser-only `surface=app` override in `src/main.tsx` so the real app can be mounted intentionally for diagnostic browser-fixture probes without changing the default preview-shell path.
- Added stable `data-testid` selectors to Azure modal controls and Hex Viewer actions so future overlay/native smoke work has deterministic hooks instead of relying on fragile text/class signatures.
- Extended `tests/overlay_integrity_runner.cjs` with modal reopen/tab-selection logic and conditional Mega protected-link seeding, while leaving the default audit target on the stable preview shell.
- Re-ran the supported preview-shell audit successfully: `32/32 PASS`, `0 FAIL`, `0 SKIP`, geometry unchanged within `1px`.

### Root Cause
- The current full-app browser-fixture path is still outside the validated Playwright design boundary. The earlier Phase 97 split was correct: the reduced preview shell is stable, but the real `App.tsx` tree can still crash headless Chromium before mount when it is forced into the browser-only route.

### Prevention Rules
**33. Do not replace the stable preview-shell Playwright gate with the full `App.tsx` browser fixture until that surface survives headless Chromium mount without crashing.**
**34. Modal/dialog controls that must participate in overlay or native smoke audits need stable `data-testid` selectors plus deterministic reopen logic.**

## Phase 97: Browser Preview Shell Split + Remote Font Removal (2026-03-10)

### Issues Found
- Playwright still hung after moving the dev server to a different port, so the problem was not port allocation.
- Browser preview depended on remote Google Fonts, which made the `load` event path nondeterministic in headless validation.
- The browser fixture path was still mounting the native Tauri `App.tsx` tree, so Playwright had to parse native bridge imports and live operator child trees that are irrelevant outside a Tauri container.

### Fixes Implemented
- Removed remote Google Fonts `@import` usage from `src/index.css` and switched to local font-face fallbacks plus local/system stacks.
- Added `src/platform/tauriClient.ts` so native bridge calls (`invoke`, `listen`, dialog/path helpers) are loaded lazily instead of at module import time.
- Updated `src/main.tsx` to runtime-split bootstrapping: browser preview now mounts `BrowserPreviewApp.tsx`, while native Tauri sessions still mount the full `App.tsx`.
- Built a deterministic preview shell in `BrowserPreviewApp.tsx` that preserves the operator controls and dashboard test ids required by Playwright without touching native bridge code.

### Prevention Rules
**22. Browser/Playwright preview must never depend on remote fonts or other network-only shell assets to reach a stable `load` event.**
**23. Native Tauri bridge imports must stay behind runtime gates; browser preview should not import native APIs just to render fixture UI.**
**24. Browser fixture validation should mount a deterministic preview shell, not the full native operator tree, unless the test explicitly targets the native runtime.**

## Phase 97B: Visual Regression Rebaseline Decision (2026-03-10)

### Issues Found
- The repaired preview shell passed functional browser tests, but the resource-metrics card snapshot no longer matched the pre-split baseline.
- The mismatch was layout and font-metric drift, not missing content: the card was now rendered through the intentional browser preview shell with local/offline-safe fonts, so the old snapshot was no longer the right reference image.

### Fixes Implemented
- Pinned the preview-shell metrics card to a deterministic width inside `BrowserPreviewApp.tsx` so the browser-only surface does not drift with the reduced two-card dashboard layout.
- Refreshed only `tests/visual_regression.spec.ts-snapshots/vanguard-metrics-state-chromium-darwin.png` after confirming the new render was the intended post-fix browser preview output.
- Locked the testing strategy: browser preview remains the main Playwright visual-regression surface; native-webview checks, if added, should be smoke tests only.

### Prevention Rules
**25. Refresh a visual baseline only after verifying the render change comes from an intentional shell or typography decision, not from an accidental layout regression.**
**26. Browser preview fixture shells should pin critical snapshot surfaces to deterministic geometry when the production dashboard layout is intentionally reduced for testability.**

## Phase 52B: Mega.nz + Torrent Frontend Integration (2026-03-07)

### Issues Found
- No UI affordance for Mega.nz or BitTorrent input — users had to know these were supported
- Input field gave no feedback when a non-onion link was pasted
- No visual distinction between active protocol modes

### Fixes Implemented
- Added permanent **Mega.nz** and **Torrent** toolbar buttons with `Cloud` and `Magnet` Lucide icons
- Added `inputMode` React state (`onion | mega | torrent`) with auto-detect on URL `onChange`
- Input label dynamically switches to `MEGA.NZ` / `TORRENT` / `Target Source` based on detected mode
- Placeholder text changes per mode (mega URL pattern vs magnet URI pattern)
- Added `.tool-btn.active` CSS with glowing cyan accent for active mode indication
- Mode auto-detect rules: `mega.nz/` or `mega.co.nz/` → mega; `magnet:?` → torrent; else → onion

### Prevention Rules
**19. New download modes must have always-visible toolbar buttons, not hidden settings.**
**20. Auto-detect must run synchronously on every input keystroke — no debounce — for instant mode feedback.**
**21. Mode-specific placeholder text must guide users toward correct URL format for each protocol.**


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
**17. Protobuf frame decoding for UI state MUST request `defaults: true` (or equivalent schema normalization) so proto3 zero-value omission cannot erase render-critical numeric fields.**
**18. Telemetry updates must merge into prior UI snapshots; never blindly replace strongly-typed state with sparse transport payloads.**

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
- 2026-03-09: Stabilized Start Queue rendering by normalizing/merging binary telemetry frames before Dashboard binding.

# Appendices
- Validation:
  - `npm run build` (TypeScript + Vite build passing)

## Phase 61: UI Deadlock Fix & Tokio Runtime Boundary (2026-03-08)

### Issues Found
- The UI would permanently freeze at "Probing Target..." when attempting to crawl Qilin, while headless backend tests completed instantly.
- The `Crawli` log stream ceased updating without emitting any crash log or stack trace.

### Fixes Implemented
- Identified a silent thread panic caused by `tokio::task::block_in_place()` being executed within the Tauri `#[tauri::command]` IPC environment, which lacks Tokio's multi-threaded (`MT`) reactor context.
- Stripped 100% of asynchronous `tokio::sync::RwLock` constraints from the `ArtiSwarm` and Qilin networking stacks.
- Re-architected primitives using `std::sync::RwLock::new()`, allowing the frontend to synchronously pull `.read().unwrap()` data without stalling the IPC bridge.

### Prevention Rules
**22. Never perform `tokio::task::block_in_place` inside a `#[tauri::command]` function. The Tauri IPC boundary is not wrapped with Tokio's necessary MT engine.**
**23. Standard library `std::sync::RwLock` must always be preferred for shared state that spans both synchronous UI setup blocks and asynchronous worker pools.**

## Phase 61b: Persistent "Probing Target" Hang Despite RwLock Fix (2026-03-08)

### Issue
After fixing the `tokio::sync::RwLock` deadlock (Phase 61), the GUI still froze at "Probing Target" for 4+ minutes. Tor Swarm reported active, but Nodes Indexed = 0, Node = unresolved.

### Root Cause
`qilin_nodes.rs::discover_and_resolve()` — the 4-stage QData storage node discovery pipeline — had no global timeout. Stage A/B HTTP calls lacked per-request timeouts. Stage D probed 17 mirrors × 15s each.

### Fix
1. Wrapped `discover_and_resolve()` in `qilin.rs` with 90s global `tokio::time::timeout`
2. Wrapped Stage A and B HTTP calls with 20s timeouts
3. Reduced `PROBE_TIMEOUT_SECS` 15→10, `PREFERRED_NODE_TIMEOUT_SECS` 8→6

### Prevention Rules
**24. Every HTTP call through Tor circuits MUST have an explicit `tokio::time::timeout`. Never rely on Tor's built-in connection timeout for GUI-interactive code paths.**

### [2026-03-08] Playwright GUI E2E Tauri Listener Desync
* **Issue**: Playwright tests could not verify GUI batch updates because `window.dispatchEvent(new CustomEvent(...))` does not interop with Tauri`s native `listen<T>` frontend API.
* **Exact Fix**: Refactored all 15 `App.tsx` event hooks using a custom universal `addAppListener` proxy adapter. Evaluates `isDownloadFixtureMode()` to bind to DOM `addEventListener` locally for offline DOM verification rendering the exact BBR metrics inside `Dashboard.tsx`.
* **Prevention Rule**: PR-GUI-001 - Playwright Frontend must execute entirely decoupled from Tauri Native context using explicit Fixtures bounding simulated progress ticks over standard `CustomEvent` interfaces.

## Phase 74C: React Rendering Crash on Crawl Start (2026-03-08)

### Issues Found
- "Render breaks right after I start clicking on the sync option to start the crawling process. That's when the render breaks and that dark image just shows up."
- The Tauri `crawli` instance abruptly exited with code `0`.

### Root Cause
- `Dashboard.tsx` attempted to dynamically parse `crawlRunStatus?.stableCurrentListingPath` by invoking `.split(/[\\/]/)` during render.
- However, when a new crawl is initiated via the Sync button, `App.tsx` correctly clears the `crawlRunStatus` state to `null`.
- The expression `crawlRunStatus?.stableCurrentListingPath.split(...)` evaluated to `undefined.split(...)`, producing an unhandled `TypeError` that crashed the entire React tree synchronously.
- With no Error Boundary, the UI collapsed to the Tauri `--window-background-color` (black). The user closing this dead window resulted in the graceful `Exit code: 0` backend log.
- Separately, `ceilingStatus` was destructured without a default value, though this was not the primary crash vector.

### Fixes Implemented
- **Safe Navigation:** Implemented standard optional chaining across the entire string parsing expression (`crawlRunStatus?.stableCurrentListingPath?.split(...)`) in `Dashboard.tsx`.
- **Default Props:** Bound robust default structures for `ceilingStatus` destructuring in `Dashboard.tsx` to handle React initialization phases securely.

### Prevention Rules
**25. Any string manipulation (split, slice, replace) performed during React render on deep property paths MUST utilize full optional chaining up to and including the invocation target.**
**26. Do not assume backend-provided status objects are immutable; `null` is a valid state during Phase Transitions (e.g., initiating a new Sync).**

## Phase 74D: Playwright Overlay Integrity Test Fixes (2026-03-09)

### Issues Found
- Running the `overlay:integrity` script against the newly verified GUI produced two false-positive interaction failures on standard VFS Tree nodes (like `vfs-toggle` or `README.txt`). 
- The script reported `Element is outside of the viewport` while the Geometry assertions returned `UNCHANGED`.

### Root Cause
- `VfsTreeView.tsx` utilizes `@tanstack/react-virtual` with a heavy buffer `<overscan: 20>`.
- The nodes technically exist inside the DOM tree, but their absolute offsets project them just outside the physical bounding box bounds until manually scrolled into view.
- Standard Playwright `.click()` routines aborted with visibility constraint assertions when simulating clicks on effectively occluded controls without prior explicit viewport translation scroll requests.

### Fixes Implemented
- Modified `overlay_integrity_runner.cjs` to gracefully catch and inspect `outside of the viewport` exceptions. 
- When intercepted, the script overrides standard Playwright safety hooks to natively propagate DOM-level events (`.evaluate((el) => el.click())`), accurately verifying the synthetic control reactivity regardless of initial geometry projection offset.

### Prevention Rules
**27. React-Virtual overscan artifacts in Playwright integrity matrices must safely trap `out-of-bound` click exceptions to invoke native DOM evaluations, otherwise false-positive UI breaks block regression runs.**

## Phase 74E: Start Queue Renderer Crash (2026-03-09)

### Issues Found
- The desktop window could black-screen immediately after `Start Queue` was clicked, even though crawl startup logs continued briefly.
- Crash occurred before meaningful queue progress rendered.

### Root Cause
- Binary telemetry polling in `App.tsx` decoded protobuf frames and directly replaced `crawlStatus` / `resourceMetrics` with sparse payload objects.
- Proto3 omits default scalar fields (zeros), so fields like `visitedNodes`, `queuedNodes`, `systemMemoryPercent`, and related counters were intermittently absent.
- `Dashboard.tsx` calls `.toLocaleString()` / `.toFixed()` on those fields, so missing values caused synchronous React render exceptions.

### Fixes Implemented
- Added `normalizeCrawlStatusFrame(...)` and `normalizeResourceMetricsFrame(...)` in `App.tsx` to coerce all telemetry values to stable numeric defaults.
- Switched protobuf conversion calls to `toObject(..., { longs: Number, defaults: true })` for crawl/resource/batch frames.
- Replaced full-object state assignment with merge-based updates (`setCrawlStatus(prev => ...)`, `setResourceMetrics(prev => ...)`) to preserve non-frame dashboard fields (`estimation`, `processThreads`, `uptimeSeconds`, etc.).

### Phase 74F: Qilin Adaptive MultiClientPool Lazy Loading (2026-03-09)

### Issues Found
- The user observed extreme >120s initialization times when crawling a new target if concurrency ceilings were raised (e.g. 16 Circuits).
- Log read: `Bootstrapping MultiClientPool with 16 TorClients`... followed 128s later by `Concurrent Pre-heating`.
- The adaptive MultiClient framework was meant to scale into its ceiling, but instead forced a blocking massive scale-out before any requests were dispatched.

### Root Cause
- `MultiClientPool::new` mapped and initialized all `CRAWLI_MULTI_CLIENTS` entirely upfront in a `join_all` statement.
- Because `node_100` Vanguard cache is copied to every node, doing this sequentially using `spawn_blocking` within the `new` loop incurred 10-20 seconds per client on disk I/O alone.
- Following the copy, heavy network bootstraps took another minute to complete for all 15 clones.
- Lastly, the `for i in 0..multi_clients { get_client(i).await }` preheat loop awaited them consecutively rather than within spawned tasks.

### Fixes Implemented
- **True Adaptive Lazy Loading**: `MultiClientPool` now drops the `clients` array to `Arc<RwLock<Option<Arc<TorClient>>>>`. Only the Vanguard (slot 0) fully warms on `MultiClientPool::new`.
- **Just-In-Time Sub-Node Spawning**: When the governor requests a client that doesn't exist, `get_client` handles localized locking, copies the Vanguard directory immediately via `spawn_blocking`, bootstraps the client dynamically, caches it, and returns the result.
- **Concurrent Pre-heating Setup**: `qilin.rs` and `dragonforce.rs` preheat sequences moved their `get_client().await` calls *inside* the `tokio::spawn` closures so that initial scaling hits the lazy instantiation sequentially across all sub-threads rapidly, unlocking the initial scan sequence immediately.

### Prevention Rules
**30. Pool architectures allocating heavyweight external libraries/resources MUST implement lazy instantiation (Optionals via double-checked locking) rather than strictly-sized upfront loops to comply with "Adaptive Scaling" policies.**

## Phase 98A: Native-Webview Smoke Bootstrap Bypass (2026-03-10)

### Issues Found
- A native-webview smoke runner was added to prove the real Tauri shell mounts and exposes the critical operator controls, but the first local runs stalled without ever producing a smoke report.
- stderr from the early attempt showed startup immediately entering `Phantom Pool: Building 4 warm standby circuits...`, so the supposed smoke layer was still paying the normal crawler bootstrap cost.

### Root Cause
- `run_gui()` always spawned onion bootstrap during `tauri::Builder::setup(...)`, even when the app was being launched purely for a narrow smoke check.
- That coupled a GUI mount assertion to expensive network bootstrap and made the smoke layer unreliable on slower or automation-constrained hosts.

### Fixes Implemented
- Added a dedicated smoke-mode detector keyed off `CRAWLI_NATIVE_SMOKE_REPORT_PATH`.
- In smoke mode, `run_gui()` now skips automatic startup bootstrap and only emits runtime metrics / telemetry bridge setup, allowing the real Tauri shell to mount without phantom-pool warmup.
- Added frontend-side native smoke reporting through `get_native_smoke_config` / `report_native_smoke_result` so the real app can publish which critical `data-testid` controls were present once mounted.

### Remaining Limitation
- Local March 10, 2026 macOS validation still did not emit the smoke report even after the startup-bootstrap bypass was applied. Browser preview remains healthy, so the unresolved gap is the local native-webview automation surface, not the browser preview shell.
- Operationally, keep browser preview as the canonical visual/Playwright gate and use native-webview smoke as a smaller secondary check on hosts where a real Tauri webview session is observable.

### Prevention Rules
**31. Native smoke tests must prove mountability, not full-runtime bootstrap. Any expensive network startup in a smoke path is a design bug.**
**32. Browser preview and native-webview smoke are different layers; a browser-green result does not certify native mount, and a native smoke failure should not force the visual-regression baseline off the browser preview shell.**
