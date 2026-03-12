## Phase 129D: Batch Progress Speed Telemetry Validation
- **Date**: 2026-03-12
- **Action**: Restored `speed_mbps` to the batch-progress protobuf frame, wired it through the Rust telemetry bridge, and regenerated the tracked frontend protobuf bindings.
- **Validation Commands**:
  - `./node_modules/.bin/pbjs -t static-module -w es6 -o src/telemetry.js src/telemetry.proto`
  - `./node_modules/.bin/pbts -o src/telemetry.d.ts src/telemetry.js`
  - `npm run build`
  - `CARGO_TARGET_DIR=/tmp/crawli-telemetry-check cargo check --manifest-path src-tauri/Cargo.toml`
- **Result**:
  - The tracked frontend bindings `src/telemetry.js` and `src/telemetry.d.ts` were regenerated successfully from the updated schema.
  - Frontend production build passed after the schema/binding update; Vite only emitted the pre-existing `@protobufjs/inquire` eval warning.
  - Clean Rust compile validation passed in `3m29s`, confirming the backend telemetry structs and bridge remain in sync with the regenerated frontend decoder.

## Phase 129C: Windows Portable Reliability + Windows-Only Release Validation
- **Date**: 2026-03-12
- **Action**: Validated the Windows-portable fixes for unbuffered file I/O and privileged NT preallocation, then prepared a Windows-only `0.6.2` release path that pins automatic tag creation to the requested ref.
- **Validation Commands**:
  - `npm run build`
  - `CARGO_TARGET_DIR=/tmp/crawli-release-check-062 cargo check --manifest-path src-tauri/Cargo.toml`
  - `ruby -e 'require "yaml"; YAML.safe_load(File.read(".github/workflows/release-windows-portable.yml"), aliases: true); puts "workflow yaml parse ok"'`
- **Result**:
  - Frontend production build passed on the `0.6.2` metadata set. Vite completed successfully and only emitted the existing `@protobufjs/inquire` eval warning.
  - Clean Rust compile validation passed in `4m26s` against a fresh target directory, confirming the Windows I/O fixes in `src-tauri/src/io_vanguard.rs` and `src-tauri/src/aria_downloader.rs` compile cleanly with the `0.6.2` release metadata.
  - The Windows-only workflow YAML parses cleanly after adding `gh release create --target "${{ github.event.inputs.ref }}"`, which prevents a new Windows-only release from being tagged off the default branch by mistake.

## Phase 129: Parallel Download + IDM-Style Acceleration Release Validation
- **Date**: 2026-03-12
- **Action**: Validated the Phase 128/129 crawl/download pipeline changes and bumped all release metadata to `0.6.1` before the GitHub Windows portable release cut.
- **Validation Commands**:
  - `npm run build`
  - `CARGO_TARGET_DIR=/tmp/crawli-release-check-061 cargo check --manifest-path src-tauri/Cargo.toml`
- **Result**:
  - Frontend production build passed on the `0.6.1` metadata set. Vite completed successfully and only emitted the existing `@protobufjs/inquire` eval warning.
  - Clean Rust compile validation passed in `4m12s` against a fresh target directory, which confirms the `src-tauri/src/lib.rs` parallel-download changes and the `src-tauri/src/aria_downloader.rs` mirror-striping/dynamic-bisection changes compile cleanly together.

## Phase 127: GitHub Release Workflow + Portable Packaging Validation
- **Date**: 2026-03-12
- **Action**: Added a shared Windows portable packaging script and updated both release workflows to call the same script.
- **Validation Commands**:
  - `ruby -e 'require "yaml"; [".github/workflows/release.yml", ".github/workflows/release-windows-portable.yml"].each { |p| YAML.safe_load(File.read(p), aliases: true); }; puts "workflow yaml parse ok"'`
  - `pwsh -NoLogo -NoProfile -File packaging/windows/package-portable.ps1 -Tag v0.6.2`
- **Result**:
  - Workflow YAML files parse successfully after the release edits.
  - `pwsh` is not installed in this macOS environment (`command not found`), so executable-level validation of `package-portable.ps1` must run on `windows-latest` CI or a Windows host.

## Phase 113: VFS Path Canonicalization + Direct-Child Guard
- **Date**: 2026-03-11
- **Action**: Repaired the virtual file system tree so nested files remain under their proper folders even when crawled entries arrive with Windows-style `\` separators or mixed logical path formatting. The backend now canonicalizes logical VFS entry paths during insert/read/query, and the frontend tree loaders normalize and filter returned children so a malformed entry cannot flatten the visible tree layer.
- **Validation Commands**:
  - `node ./node_modules/vitest/vitest.mjs run src/components/VFSExplorer.test.tsx`
  - `node ./node_modules/typescript/bin/tsc --noEmit`
  - `cargo test --manifest-path src-tauri/Cargo.toml db::tests:: -- --nocapture`
  - `cargo test --manifest-path src-tauri/Cargo.toml support_ -- --nocapture`
  - `cargo test --manifest-path src-tauri/Cargo.toml normalize_windows_device_path -- --nocapture`
- **Result**:
  - `VFSExplorer.test.tsx` passed `5/5`, including a new regression that feeds a root-layer response with `\\folder1\\file1.txt` and verifies the file stays hidden until `folder1` is expanded.
  - `db::tests::insert_entries_normalizes_windows_separators` and `db::tests::get_children_keeps_legacy_windows_paths_nested` both passed, proving new inserts are canonicalized and legacy malformed entries still hydrate into the correct nested tree.
  - The Windows support-path regression suite remained green after the VFS patch, which confirms the new logical-path normalization is isolated from real filesystem/output-root handling.

## Phase 112: In-Output Support Root Simplification
- **Date**: 2026-03-11
- **Action**: Simplified support-artifact resolution so the backend always writes hidden support data under the operator-selected output root instead of preferring a sibling hidden root first. This removes the remaining Windows-specific `Start Queue` failure mode where the runtime still tried to reason about a second anchor outside the chosen export folder.
- **Validation Commands**:
  - `cargo test --manifest-path src-tauri/Cargo.toml --lib support_ -- --nocapture`
  - `cargo test --manifest-path src-tauri/Cargo.toml --lib normalize_windows_device_path -- --nocapture`
- **Result**:
  - Targeted Rust validation passed `3/3` support-path tests and `1/1` Windows display-path tests.
  - The support-root invariant is now explicit: for any selected output root, support artifacts resolve under `<output_root>/.onionforge_support/<support_key>/`.
  - A regression test now creates a blocked sibling `.onionforge_support` file next to the selected output folder and verifies that crawl/download startup still succeeds because the resolver no longer touches that sibling location at all.
  - User-facing failures are also cleaner. If support-directory creation now fails, the backend reports only the actual in-output support path instead of a preferred/fallback pair.

## Phase 111: Windows Support-Path Hardening
- **Date**: 2026-03-11
- **Action**: Hardened Windows output/support path handling so GUI download/start-queue flows no longer fail when the selected export directory is directly under a drive/share root and the backend is operating on extended-length paths. The backend now normalizes `\\?\` / `\\?\UNC\` prefixes for display, strips reserved characters from the generated support key, and proactively keeps `.onionforge_support` inside the selected output folder when the sibling layout would otherwise land at `X:\` or `\\server\share\`.
- **Validation Commands**:
  - `cargo test --manifest-path src-tauri/Cargo.toml --lib support_ -- --nocapture`
  - `cargo test --manifest-path src-tauri/Cargo.toml --lib windows_ -- --nocapture`
- **Result**:
  - Targeted Rust validation passed `3/3` support-path tests and `3/3` Windows-path tests.
  - The new tests prove three specific fixes: `support_key_for_path(r"\\\\?\\X:\\Exports\\Case 1")` no longer contains `?`, `:`, or separator characters; operator-facing path strings now display `X:\...` / `\\server\share\...` instead of raw device syntax; and output roots like `X:\Exports` or `\\server\share\Exports` are treated as “root-parent” layouts that must use the in-output hidden support root.
  - Expected behavior changed on Windows only for that root-parent case: selecting `X:\Exports` now resolves support artifacts under `X:\Exports\.onionforge_support\<support_key>\` instead of trying to create `X:\.onionforge_support\<support_key>\`.

## Phase 110: Support Directory Fallback Hardening
- **Date**: 2026-03-11
- **Action**: Hardened support-artifact directory resolution so the backend no longer aborts when the preferred hidden sibling support root cannot be created. The runtime now falls back to `output_root/.onionforge_support/<support_key>/` after a preferred-root creation failure, and `target_state.rs` now uses the same resolver so crawl/download state stays path-consistent.
- **Validation Commands**:
  - `cargo test --manifest-path src-tauri/Cargo.toml --lib support_artifact_dir_ -- --nocapture`
- **Result**:
  - The preferred behavior is unchanged when the parent-side hidden root is writable: support artifacts still live outside the payload tree under the sibling `.onionforge_support` directory.
  - The new regression test simulates a blocked sibling anchor by placing a file at `<output_parent>/.onionforge_support`, which deterministically forces the fallback path.
  - The targeted Rust validation passed with `2/2` support-directory tests, covering both the normal sibling-root case and the blocked-sibling fallback case.

## Phase 109: Renderer Stability + Hidden Hex Virtualizer Fix
- **Date**: 2026-03-11
- **Action**: Repaired the remaining full-app renderer instability by moving the hidden Hex viewer virtualizer behind an open-only mount boundary, preserving click-through behavior on toast overlays, and hardening the remaining crawl-option state writes.
- **Validation Commands**:
  - `node node_modules/vitest/vitest.mjs run src/App.interaction.test.tsx --maxWorkers=1 --reporter=verbose`
  - `node node_modules/typescript/bin/tsc --noEmit`
  - `npm run build`
  - `node - <<'EOF' ... chromium.launch() ... page.goto('http://127.0.0.1:1420/?fixture=vfs&surface=app') ... click checkboxes/start queue ... EOF`
- **Result**:
  - The previous jsdom heap blowup is gone. `src/App.interaction.test.tsx` now passed `2/2` in about one second instead of exhausting the worker heap near `4 GB`.
  - Frontend compilation and production bundling stayed green after the fix. `tsc --noEmit` passed, and `npm run build` completed successfully; Vite only emitted the pre-existing `@protobufjs/inquire` eval warning.
  - The full browser-mounted `App.tsx` fixture is now usable for targeted diagnostics. A raw Chromium validation run mounted `?fixture=vfs&surface=app`, toggled the crawl option checkboxes, clicked `Start Queue`, surfaced the expected browser-only environment error toast, and then clicked the same checkbox again successfully with the toast still visible.
  - Layout integrity held during the interaction path. `.main-workspace` measured `1400x629` before and after the click/toast cycle, and screenshots were captured at `output/ui-fix-validation/full-app-before.png` and `output/ui-fix-validation/full-app-after.png`.

## Phase 108: Overlay Integrity Audit + Full-App Fixture Probe
- **Date**: 2026-03-11
- **Action**: Added a browser-only `surface=app` bootstrap override in `src/main.tsx`, added stable modal test ids in `AzureConnectivityModal.tsx` / `HexViewer.tsx`, extended `tests/overlay_integrity_runner.cjs` for modal re-entry and conditional Mega password coverage, then ran the overlay audit on both the stable preview shell and the new full-app browser fixture probe.
- **Validation Commands**:
  - `node node_modules/typescript/bin/tsc`
  - `npm run overlay:integrity`
  - `node - <<'EOF' ... page.goto('http://127.0.0.1:1420/?fixture=vfs&surface=app') ... EOF`
- **Result**:
  - Frontend compilation stayed green after the new browser-fixture/test-id work.
  - The supported Playwright surface remains healthy. `npm run overlay:integrity` passed `32/32` controls with `0` failures and `0` skips on `http://127.0.0.1:1420/?fixture=vfs`, and all geometry assertions stayed within the `1px` tolerance. Artifacts live under `output/playwright/overlay-integrity/2026-03-11T13-07-38-714Z/`.
  - The full browser-mounted `App.tsx` surface is still not certifiable. When the same app was forced through `?fixture=vfs&surface=app`, Chromium imported the full dependency graph (`App.tsx`, `VFSExplorer`, `Dashboard`, `AzureConnectivityModal`, `HexViewer`) but the page never reached `.app-container` and the renderer crashed before mount. The failed artifact root is `output/playwright/overlay-integrity/2026-03-11T13-03-42-529Z/`.
  - Operationally, preview-shell overlay integrity remains the canonical Playwright gate today. The new `surface=app` path is a diagnostic/debug surface until the headless Chromium crash is removed.

## Phase 102: Probe Admission Telemetry + Cooldown Escalation
- **Date**: 2026-03-11
- **Action**: Added shared probe-admission counters plus stricter degraded-host productivity decay/cooldown in the downloader, regenerated protobuf bindings, revalidated the Rust library suite, and ran a short exact-target `crawli-cli download-files` replay against the existing Phase 90 reconciliation snapshot.
- **Validation Commands**:
  - `cargo fmt --manifest-path 'src-tauri/Cargo.toml'`
  - `node node_modules/protobufjs-cli/bin/pbjs -t static-module -w es6 -o src/telemetry.js src/telemetry.proto`
  - `node node_modules/protobufjs-cli/bin/pbts -o src/telemetry.d.ts src/telemetry.js`
  - `CARGO_NET_OFFLINE=true cargo test --manifest-path 'src-tauri/Cargo.toml' --lib`
  - `node node_modules/typescript/bin/tsc`
  - `CARGO_NET_OFFLINE=true cargo build --manifest-path 'src-tauri/Cargo.toml' --bin crawli-cli`
  - `perl -e 'alarm shift; exec @ARGV' 90 ./src-tauri/target/debug/crawli-cli --progress-summary --progress-summary-interval-ms 5000 download-files --entries-file '/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli/tmp/live_cli_qilin_afa2a0ea_20260310_phase90_reconciliation_fix/output/targets/ijzn3sicrcy7guixkzjkib4ukbii__afa2a0ea-20ba-3ddf-8c5c-__35770556215d08e/current/listing_canonical.json' --output-dir '/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli/tmp/qilin_cli_quick_probe_20260311_20260311_025128/output' --connections 120`
- **Result**:
  - Backend validation remained green after the telemetry/cooldown tranche: the Rust library suite passed `129/129`, including the new downloader and runtime-metrics regressions.
  - Frontend schema/type drift was checked through protobuf regeneration and `tsc`. A direct `vite build` rerun was started locally but did not complete cleanly before session closure, so no fresh bundle artifact is claimed for this tranche.
  - The quick exact-target CLI replay proved the new counters are live. The first file logged `GET Range` probe timeouts at `8s`, `12s`, and `16s`, then emitted `Probe candidate exhaustion ... exhausted 3 candidates`; the very next file entered with `quarantined_candidates=3/3` and logged `Probe candidate set fully quarantined`.
  - The compact summary now exposes the new state directly: `dl_transport=1/0/0 probe_admission=1/1`. That means the operator can now distinguish “transport path active but no productive probe candidate remained” from generic silence.
  - Useful payload work is still not proven in this short replay. The next blocker is no longer “can we see admission collapse?” but “what does the downloader do after all three known hosts are already degraded?”

## Phase 97: Browser Preview Render Audit + Alternate-Port Playwright Validation
- **Date**: 2026-03-10
- **Action**: Re-tested the GUI on a fresh Vite port after Playwright snapshotting kept hanging, then repaired the browser-only preview path so Playwright would exercise a deterministic preview shell instead of the native Tauri tree.
- **Validation Commands**:
  - `npm --prefix '/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli' run dev -- --host 127.0.0.1 --port 4173`
  - `curl -I --max-time 10 'http://127.0.0.1:4173/'`
  - `npm --prefix '/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli' run build`
  - `npx playwright test tests/crawli.spec.ts --project=chromium`
  - `npx playwright test tests/vanguard_ui.spec.ts --project=chromium`
- **Result**:
  - The alternate port proved the original symptom was not a port collision. Vite served cleanly on `127.0.0.1:4173`, but the old browser path still stalled in Playwright.
  - The first browser-only blocker was external fonts: three Google Fonts `@import` calls in `src/index.css` kept the `load` event path nondeterministic in headless preview mode.
  - The second blocker was architectural: browser preview was still mounting the full native `App.tsx` surface and transitively pulling native bridge code / live operator child trees into Playwright, even though browser preview only needs fixture-safe UI.
  - The final fix was to split bootstrap by runtime: `src/main.tsx` now loads `BrowserPreviewApp.tsx` when `__TAURI_INTERNALS__` is absent and keeps the full native `App.tsx` only for real Tauri sessions. Native bridge calls were also wrapped behind lazy loaders in `src/platform/tauriClient.ts`.
  - Final validation passed. `tests/crawli.spec.ts` (`3/3`) and `tests/vanguard_ui.spec.ts` (`1/1`) both passed, and a waited browser screenshot confirmed the preview shell renders the operator surface instead of a blank dark frame.

## Phase 97B: Visual Regression Rebaseline + Long-Term Playwright Surface Decision
- **Date**: 2026-03-10
- **Action**: Ran the visual regression suite against the repaired browser preview shell, inspected the one failing snapshot, and refreshed the baseline only after confirming the new metrics-card render was caused by the intentional offline-safe preview shell and local-font stack.
- **Validation Commands**:
  - `npx playwright test tests/visual_regression.spec.ts --project=chromium`
  - `npx playwright test tests/visual_regression.spec.ts --project=chromium --update-snapshots`
  - `npx playwright test tests/visual_regression.spec.ts --project=chromium`
- **Result**:
  - The suite first failed `1/3` only on `vanguard-metrics-state.png`; the other two page-level baselines stayed valid.
  - The mismatch was intentional, not accidental. The new preview shell no longer renders through remote Google Fonts or the native Tauri tree, so the resource metrics card now has different but stable font metrics and wrapping behavior.
  - Only `tests/visual_regression.spec.ts-snapshots/vanguard-metrics-state-chromium-darwin.png` was regenerated. The rerun after snapshot refresh passed `3/3`.
  - Testing strategy is now explicit: `BrowserPreviewApp.tsx` remains the primary Playwright/visual-regression surface because it is deterministic and fixture-safe. Any future native-webview validation should be a separate smoke layer that proves real Tauri mounting/bridging, not the canonical visual-baseline source.

## Phase 96: Windows Portable CLI Audit + Dedicated Console Binary
- **Date**: 2026-03-10
- **Action**: Audited the shipped Windows portable artifact after operator reports that CLI commands did not work. Added a dedicated console binary (`crawli-cli`) plus portable packaging changes so Windows terminal usage no longer depends on the GUI-subsystem `crawli.exe`.
- **Validation Commands**:
  - `cargo check --manifest-path 'crawli/src-tauri/Cargo.toml' --bin crawli-cli`
  - `cargo test --manifest-path 'crawli/src-tauri/Cargo.toml' --lib cli::tests`
  - `cargo build --manifest-path 'crawli/src-tauri/Cargo.toml' --bin crawli-cli`
  - `./crawli/src-tauri/target/debug/crawli-cli --help`
  - `./crawli/src-tauri/target/debug/crawli-cli detect-input-mode --input 'https://proof.ovh.net/files/10Gb.dat' --compact-json`
- **Result**:
  - Root cause was confirmed in code and packaging: `src-tauri/src/main.rs` builds `crawli.exe` with `windows_subsystem = "windows"`, and the portable workflow only copied `crawli.exe` into the release zip.
  - The new dedicated console binary now boots the same shared backend CLI surface through `crawli_lib::run_cli()` and prints normal terminal help/output.
  - Local smoke validation succeeded: `crawli-cli --help` printed the full command catalog and `detect-input-mode` returned `{"input":"https://proof.ovh.net/files/10Gb.dat","mode":"direct"}`.
  - Windows portable packaging now builds and copies `crawli-cli.exe`, a `crawli-cli.cmd` wrapper, and `README.txt` in addition to the GUI `crawli.exe`.


## Phase 95: Clearnet Direct-File Audit + Direct Mode Fix
- **Date**: 2026-03-10
- **Action**: Verified the direct-artifact path against a safe public HTTP(S) target, repaired clearnet-vs-onion mode detection, fixed piece-mode resume accounting for large multi-piece transfers, and re-ran both the tool and a single-stream control.
- **Validation Commands**:
  - `curl -I --max-time 20 'https://cdn.breachforums.as/pay_or_leak/shouldve_paid_the_ransom_pathstone.com_shinyhunters.7z'`
  - `./crawli/src-tauri/target/debug/crawli detect-input-mode --input 'https://cdn.breachforums.as/pay_or_leak/shouldve_paid_the_ransom_pathstone.com_shinyhunters.7z' --compact-json`
  - `./crawli/src-tauri/target/debug/crawli detect-input-mode --input 'https://proof.ovh.net/files/10Gb.dat' --compact-json`
  - `./crawli/src-tauri/target/debug/crawli --progress-summary --progress-summary-interval-ms 5000 crawl --url 'https://proof.ovh.net/files/10Gb.dat' --output-dir '/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli/tmp/clearnet_direct_artifact_probe_phase94c/output'`
  - `perl -e 'alarm shift; exec @ARGV' 60 ./crawli/src-tauri/target/debug/crawli --progress-summary --progress-summary-interval-ms 5000 initiate-download --url 'https://proof.ovh.net/files/10Gb.dat' --path '10Gb.dat' --output-root '/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli/tmp/clearnet_download_phase94h_tool_c120_patched2/output' --connections 120`
  - `perl -e 'alarm shift; exec @ARGV' 60 curl -L --fail --silent --show-error 'https://proof.ovh.net/files/10Gb.dat' -o '/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli/tmp/clearnet_download_phase94i_curl/10Gb.dat'`
  - `cargo test --manifest-path 'crawli/src-tauri/Cargo.toml' --lib`
  - `cargo build --manifest-path 'crawli/src-tauri/Cargo.toml' --bin crawli`
  - `npm --prefix '/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli' run build`
- **Result**:
  - The user-provided HTTPS `.7z` endpoint was verified non-destructively only. `HEAD` returned `HTTP/2 200`, `accept-ranges: bytes`, `content-length: 15982638474`, `content-type: application/x-7z-compressed`, and `server: ddos-guard`. The archive itself was not downloaded during this audit.
  - `detect-input-mode` now classifies both the user-provided `.7z` URL and the public benchmark file as `direct` instead of `onion`.
  - The direct-artifact crawl path works for clearnet binaries: the `10Gb.dat` probe was intercepted as a raw direct artifact and persisted as a single discovered file.
  - The rebuilt direct downloader materially improved on the same host/file pair. The earlier `120`-connection build advanced about `717MB` of persisted state in `60s` (`~95.7 Mbps`). The repaired run reached about `3.8 GiB` in the live summary within `60s` (`~63 MiB/s`, `~530 Mbps`) with `32` active circuits after the handshake filter.
  - The same-day single-stream control (`curl`) reached `1,562,718,208` bytes in `60s` (`~24.8 MiB/s`, `~208 Mbps`), so the current clearnet direct path is about `2.4x` faster than the single-stream baseline on this benchmark.
  - The interrupted-run accounting bug is fixed. Piece-mode state now persists `current_offsets` across all `256` pieces instead of only the first `32` circuit slots, so resumed large transfers no longer silently underreport progress beyond the initial wave.

## Phase 94: Qilin Download Host Remap + Hidden Support Root
- **Date**: 2026-03-10
- **Action**: Implemented per-file alternate-host remap for repeated Qilin connection/send stalls, moved downloader support artifacts into a hidden sibling support root, replaced eager per-file `.onionforge.meta` writes with `_onionforge_manifest.txt` plus `download_support_index.json`, and reran the exact same Qilin timed download window.
- **Validation Commands**:
  - `cargo test --manifest-path 'crawli/src-tauri/Cargo.toml' --lib`
  - `cargo build --manifest-path 'crawli/src-tauri/Cargo.toml' --bin crawli`
  - `./crawli/src-tauri/target/debug/crawli --progress-summary --progress-summary-interval-ms 5000 download-files --entries-file '/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli/tmp/live_cli_qilin_afa2a0ea_20260310_phase90_reconciliation_fix/output/targets/ijzn3sicrcy7guixkzjkib4ukbii__afa2a0ea-20ba-3ddf-8c5c-__35770556215d08e/current/listing_canonical.json' --output-dir '/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli/tmp/live_qilin_download_phase94a/full_download' --connections 120`
  - `./crawli/src-tauri/target/debug/crawli --progress-summary --progress-summary-interval-ms 5000 download-files --entries-file '/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli/tmp/live_cli_qilin_afa2a0ea_20260310_phase90_reconciliation_fix/output/targets/ijzn3sicrcy7guixkzjkib4ukbii__afa2a0ea-20ba-3ddf-8c5c-__35770556215d08e/current/listing_canonical.json' --output-dir '/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli/tmp/live_qilin_download_phase94b_support_index/full_download' --connections 120`
- **Result**:
  - Library validation stayed green after the downloader/runtime refactor: `cargo test --lib` passed `113/113`, and the main CLI rebuilt cleanly.
  - The exact-target `150s` run on March 10, 2026 proved that per-file alternate-host remap is now live. The downloader logged `24` remap events: `18` `zqetti36... -> lblnwlid...` and `6` `lblnwlid... -> zqetti36...`.
  - The visible payload tree is now clean of non-payload support artifacts. `temp_onionforge_forger` was absent from the output root, payload bytes outside placeholders stayed at `0`, and the tree contained `522` scaffolded directories with `519` `.gitkeep` placeholders.
  - The new single support index is now emitted during scaffold, not only after a clean batch exit. A separate interrupted `20s` rerun created `_onionforge_manifest.txt` (`722,464` bytes) and `download_support_index.json` (`947,433` bytes) under `.onionforge_support/<support_key>/` while keeping the payload root clean.
  - Useful work is still `0` on the exact target inside the `150s` window. The current snapshot still spans `3` hosts (`lbln...=962`, `4xl2hta3...=745`, `zqetti...=687`), but the measured remap traffic only exercised the `zqetti.../lbln...` pair. That narrows the remaining throughput blocker to first-wave repin/admission bias rather than missing remap support or payload-root artifact pollution.

## Phase 93: First-Byte Escape + Host-Diversified Admission
- **Date**: 2026-03-10
- **Action**: Added queue-backed requeue handling plus explicit first-byte timeout gating in `aria_downloader.rs`, diversified the first micro/small wave across distinct hosts before the SRPT queue settles, and re-ran the exact same Qilin current-snapshot timed download window.
- **Validation Commands**:
  - `cargo test --manifest-path 'crawli/src-tauri/Cargo.toml' aria_downloader::tests --lib`
  - `cargo build --manifest-path 'crawli/src-tauri/Cargo.toml' --bin crawli`
  - `./crawli/src-tauri/target/debug/crawli --progress-summary --progress-summary-interval-ms 5000 download-files --entries-file '/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli/tmp/live_cli_qilin_afa2a0ea_20260310_phase90_reconciliation_fix/output/targets/ijzn3sicrcy7guixkzjkib4ukbii__afa2a0ea-20ba-3ddf-8c5c-__35770556215d08e/current/listing_canonical.json' --output-dir '/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli/tmp/live_qilin_download_phase93a/full_download' --connections 120`
- **Result**:
  - The new downloader behavior is live: the run logged `36` explicit micro/small requeues (`18` per lane) instead of silently pinning those files behind the original first wave.
  - The `.onionforge.meta` files seen in the output root are expected support artifacts under `temp_onionforge_forger`, created during scaffold before transfer, not completed payloads.
  - Useful work is still `0` on this exact-target `150s` window. The payload tree outside `temp_onionforge_forger` contained only `519` zero-byte `.gitkeep` placeholders and no real document bytes, so the remaining bug is now clearly per-file host failure recovery, not confusion about what the sidecars mean.

## Phase 92: Download Admission Audit + Qilin Route Repinning
- **Date**: 2026-03-10
- **Action**: Added a bounded large-file overlap planner plus warmup gate in `aria_downloader.rs`, repinned saved Qilin download URLs through persisted subtree route memory in `lib.rs` / `cli.rs`, and shortened onion micro/small batch timeouts so repeated failures actually rotate isolated clients instead of sitting on a dead first wave.
- **Validation Commands**:
  - `cargo test --manifest-path 'crawli/src-tauri/Cargo.toml' aria_downloader::tests --lib`
  - `cargo test --manifest-path 'crawli/src-tauri/Cargo.toml' split_qilin_download_seed_preserves_relative_tree --lib`
  - `cargo test --manifest-path 'crawli/src-tauri/Cargo.toml' preferred_qilin_host_walks_up_to_parent_subtree --lib`
  - `cargo test --manifest-path 'crawli/src-tauri/Cargo.toml' download_host_bias_prefers_fresh_winner_over_aged_subtree_host --lib`
  - `cargo build --manifest-path 'crawli/src-tauri/Cargo.toml' --bin crawli`
  - `./crawli/src-tauri/target/debug/crawli --progress-summary --progress-summary-interval-ms 5000 download-files --entries-file '/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli/tmp/live_cli_qilin_afa2a0ea_20260310_phase90_reconciliation_fix/output/targets/ijzn3sicrcy7guixkzjkib4ukbii__afa2a0ea-20ba-3ddf-8c5c-__35770556215d08e/current/listing_canonical.json' --output-dir '/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli/tmp/live_qilin_download_phase92d/full_download' --connections 120`
  - `./crawli/src-tauri/target/debug/crawli --progress-summary --progress-summary-interval-ms 5000 download-files --entries-file '/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli/tmp/live_cli_qilin_afa2a0ea_20260310_phase90_reconciliation_fix/output/targets/ijzn3sicrcy7guixkzjkib4ukbii__afa2a0ea-20ba-3ddf-8c5c-__35770556215d08e/current/listing_canonical.json' --output-dir '/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli/tmp/live_qilin_download_phase92e/full_download' --connections 120`
- **Result**:
  - Rust validation remained green after the new downloader admission logic and Qilin repin helpers. The lane-planner tests, Qilin route rewrite tests, and the rebuilt main binary all passed.
  - The downloader now proves its new routing logic in the live CLI path: one exact-target rerun logged `[Qilin Download] Repinned 1769 saved URLs using subtree route memory (winner host zqetti36k3enp7ww53tyifmgdwckmzdnppraqho6tic5lj4q5qtim2ad.onion)`.
  - The overlap gate also behaved correctly. On repeated bad-first-wave runs the large-file lane stayed parked instead of burning time immediately on a weak first large file: `Phase 2 overlap parked; no early useful completions yet. Large files stay in serial fallback.`
  - The new micro/small timeout tuning is active in the live path (`send_timeout=45s body_timeout=90s`), but repeated `150s` exact-target timed windows still produced `0` useful completions. That means the remaining bottleneck is no longer missing host-memory reuse or missing large-lane overlap. It is the first-wave micro/small hidden-service request path itself: too many lanes can still get trapped behind no-byte or very-low-byte stalls before the queue finds a productive route.

## Phase 91: Downloader Throughput Audit + macOS Storage Reclassification
- **Date**: 2026-03-10
- **Action**: Audited the exact Qilin download path with Arti in focus, repaired onion-heavy batch classification so mid-size files enter the large-file lane, fixed large-file batch progress so aggregate `speed_mbps` no longer falls back to `0.0`, and added macOS `diskutil`-based storage fallback in `resource_governor.rs` so root bootstrap no longer under-classifies Apple Fabric / NVMe storage as `Unknown`.
- **Validation Commands**:
  - `cargo check --manifest-path 'crawli/src-tauri/Cargo.toml'`
  - `cargo test --manifest-path 'crawli/src-tauri/Cargo.toml' resource_governor::tests::macos_diskutil_parser_detects_nvme_ssd --lib`
  - `cargo test --manifest-path 'crawli/src-tauri/Cargo.toml' resource_governor::tests::onion_batch_budget_keeps_first_wave_capped --lib`
  - `cargo test --manifest-path 'crawli/src-tauri/Cargo.toml' aria_downloader::tests::onion_batch_promotes_mid_size_files_to_large_pipeline --lib`
  - `cargo test --manifest-path 'crawli/src-tauri/Cargo.toml' cli::tests::final_progress_summary_includes_route_counters --lib`
  - `cargo build --manifest-path 'crawli/src-tauri/Cargo.toml' --bin crawli`
  - `./crawli/src-tauri/target/debug/crawli --progress-summary --progress-summary-interval-ms 5000 download-files --entries-file '/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli/tmp/live_cli_qilin_afa2a0ea_20260310_phase90_reconciliation_fix/output/targets/ijzn3sicrcy7guixkzjkib4ukbii__afa2a0ea-20ba-3ddf-8c5c-__35770556215d08e/current/listing_canonical.json' --output-dir '/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli/tmp/live_qilin_download_phase91d/full_download' --connections 120`
  - `./crawli/src-tauri/target/debug/crawli --progress-summary --progress-summary-interval-ms 5000 download-files --entries-file '/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli/tmp/live_cli_qilin_afa2a0ea_20260310_phase90_reconciliation_fix/output/targets/ijzn3sicrcy7guixkzjkib4ukbii__afa2a0ea-20ba-3ddf-8c5c-__35770556215d08e/best/listing_canonical.json' --output-dir '/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli/tmp/live_qilin_download_phase91f/full_download' --connections 120`
  - `./crawli/src-tauri/target/debug/crawli --progress-summary --progress-summary-interval-ms 5000 download-files --entries-file '/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli/tmp/live_cli_qilin_afa2a0ea_20260310_phase90_reconciliation_fix/output/targets/ijzn3sicrcy7guixkzjkib4ukbii__afa2a0ea-20ba-3ddf-8c5c-__35770556215d08e/best/listing_canonical.json' --output-dir '/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli/tmp/live_qilin_download_phase91g/full_download' --connections 120`
  - `./crawli/src-tauri/target/debug/crawli --progress-summary --progress-summary-interval-ms 5000 download-files --entries-file '/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli/tmp/live_cli_qilin_afa2a0ea_20260310_phase90_reconciliation_fix/output/targets/ijzn3sicrcy7guixkzjkib4ukbii__afa2a0ea-20ba-3ddf-8c5c-__35770556215d08e/best/listing_canonical.json' --output-dir '/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli/tmp/live_qilin_download_phase91i/full_download' --connections 120`
- **Result**:
  - Targeted Rust validation remained green after the downloader and governor changes.
  - The earlier “full download” assumption was wrong for this target root. `current/listing_canonical.json` contains `2926` entries (`2394` files / `532` folders, `2.92GB` hint), while `best/listing_canonical.json` contains `5078` entries (`4240` files / `838` folders, `4.72GB` hint). All authoritative full-download benchmarking now uses the `best` snapshot explicitly.
  - The onion batch routing fix is real. On the partial-snapshot run, the downloader moved from an all-small path to `2283 micro + 98 small + 13 large`, and the live run climbed into a stable `~1.1-1.2MB/s` band with roughly `562MB` transferred by the time the benchmark window was stopped.
  - The macOS storage fallback is also real. Root bootstrap moved from `8` to `12` native Arti clients and the full best-snapshot ready quorum fell from about `35.9s` to `15.2s`.
  - More initial download circuits did **not** translate into more useful work on the hidden-service workload. The aggressive full best-snapshot posture (`24/12/16/36`) requested `24` circuits and hit ready quorum quickly, but early throughput stayed around `0.15-0.25MB/s`. The widened `16/12/16/24` posture was worse and produced early failures without useful completions.
  - The retained default posture is therefore the mixed profile validated in the final startup run: root/bootstrap sees NVMe and uses `12` Arti clients, but hidden-service multi-file batch lanes stay at `circuit_cap=16 small_parallel=8 active_start=10 tournament_cap=24`. This preserves the bootstrap win without promoting the over-aggressive transfer fan-out that degraded early useful-work throughput.

## Phase 90: Winner-Quality Memory + Tail-Latency Biasing
- **Date**: 2026-03-10
- **Action**: Persisted winner-host quality memory into the Qilin node cache, added compact final tail-latency summaries to the shared telemetry / CLI / protobuf planes, made worker repin cadence adaptive to winner quality, and then ran exact-target live crawls until the late-tail behavior was explained. The live audit also exposed a real bug in Phase 44 reconciliation, so the pass additionally stopped late missing-folder retries from resetting their history and added a wall-clock budget for reconciliation.
- **Validation Commands**:
  - `cargo check --manifest-path 'crawli/src-tauri/Cargo.toml'`
  - `cargo test --manifest-path 'crawli/src-tauri/Cargo.toml' --lib`
  - `cargo build --manifest-path 'crawli/src-tauri/Cargo.toml' --bin crawli`
  - `npm run build`
  - `./crawli/src-tauri/target/debug/crawli --progress-summary --progress-summary-interval-ms 5000 crawl --url 'http://ijzn3sicrcy7guixkzjkib4ukbiilwc3xhnmby4mcbccnsd7j2rekvqd.onion/site/view?uuid=afa2a0ea-20ba-3ddf-8c5c-2aeea9e5dc43' --output-dir '/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli/tmp/live_cli_qilin_afa2a0ea_20260310_phase90_winner_quality/output' --daemons 4 --circuits 120`
  - `./crawli/src-tauri/target/debug/crawli --progress-summary --progress-summary-interval-ms 5000 crawl --url 'http://ijzn3sicrcy7guixkzjkib4ukbiilwc3xhnmby4mcbccnsd7j2rekvqd.onion/site/view?uuid=afa2a0ea-20ba-3ddf-8c5c-2aeea9e5dc43' --output-dir '/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli/tmp/live_cli_qilin_afa2a0ea_20260310_phase90_reconciliation_fix/output' --daemons 4 --circuits 120`
  - `./crawli/src-tauri/target/debug/crawli --progress-summary --progress-summary-interval-ms 5000 crawl --url 'http://ijzn3sicrcy7guixkzjkib4ukbiilwc3xhnmby4mcbccnsd7j2rekvqd.onion/site/view?uuid=afa2a0ea-20ba-3ddf-8c5c-2aeea9e5dc43' --output-dir '/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli/tmp/live_cli_qilin_afa2a0ea_20260310_phase90_reconciliation_fix/output' --daemons 4 --circuits 120`
- **Result**:
  - Rust and frontend validation remained green after the winner-quality/tail-summary tranche: `cargo check`, `cargo build --bin crawli`, `npm run build`, and the full library test suite passed (`97` tests).
  - The first live run exposed the actual deep-tail bug. On a degraded route set that drifted across `lblnwlid...`, `4xl2hta3...`, and `sc2qyv6...`, the crawl reached roughly `99.4%` and then reopened `27` missing folders with fresh retry history, producing the user-visible "looks good early, then stalls near the end" behavior.
  - The rebuilt exact-target rerun fixed that tail. It completed in `213.52s`, confirmed durable winner `4xl2hta3ohg474n3onnbtmjnopsrwzorosgqt33sxbavthua3q5bn7qd.onion`, and ended with `3180` effective entries / `2533` files / `647` folders, `failovers=0`, `timeouts=0`, `winner_host=4xl2hta3...`, and `slowest_circuit=c0:2436ms`.
  - The next warm rerun proved winner-quality bias is active but not yet dominant enough. It probed cached winner `4xl2hta3...` first, but later accepted a fresh Stage A host `sc2qyv6...` and degraded into heavy subtree reroutes by the captured tail (`84.0%` progress, `failovers=547`, `timeouts=11`). That is the remaining bottleneck: warm reruns can still abandon a historically productive winner too easily.
  - The new final summary now carries compact tail forensics directly: `tail=winner_host/slowest_circuit/late_throttles/outlier_isolations`.

## Phase 89: Deep-Crawl Stall Audit + Throttle/Outlier Repair
- **Date**: 2026-03-10
- **Action**: Audited the exact Qilin target with full live crawls, bounded the cached fast-path probe budget, repaired first-attempt throttle classification so deep-layer `503` failures hit telemetry/healing, added a governor-side latency-outlier stall guard, and fixed final crawl accounting to report effective entries when Qilin streams directly into the VFS.
- **Validation Commands**:
  - `cargo check --manifest-path 'crawli/src-tauri/Cargo.toml'`
  - `cargo test --manifest-path 'crawli/src-tauri/Cargo.toml' qilin --lib`
  - `cargo build --manifest-path 'crawli/src-tauri/Cargo.toml' --bin crawli`
  - `./crawli/src-tauri/target/debug/crawli --progress-summary --progress-summary-interval-ms 5000 crawl --url 'http://ijzn3sicrcy7guixkzjkib4ukbiilwc3xhnmby4mcbccnsd7j2rekvqd.onion/site/view?uuid=afa2a0ea-20ba-3ddf-8c5c-2aeea9e5dc43' --output-dir '/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli/tmp/live_cli_qilin_afa2a0ea_20260310_phase89_fullcrawl_20260310_101630/output' --daemons 4 --circuits 120`
  - `./crawli/src-tauri/target/debug/crawli --progress-summary --progress-summary-interval-ms 5000 crawl --url 'http://ijzn3sicrcy7guixkzjkib4ukbiilwc3xhnmby4mcbccnsd7j2rekvqd.onion/site/view?uuid=afa2a0ea-20ba-3ddf-8c5c-2aeea9e5dc43' --output-dir '/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli/tmp/live_cli_qilin_afa2a0ea_20260310_phase89_stallguard_v2_20260310_103834/output' --daemons 4 --circuits 120`
- **Result**:
  - Rust validation remained green after the Phase 89 changes: `cargo check` passed and `qilin --lib` passed `36`.
  - The first full exact-target crawl finished in `139.86s` on storage winner `3pe26tqc...`, produced the full stable tree (`3180` effective entries), and proved the crawl was not deadlocking in deep layers. It also exposed the old blind spot: `64` deep-layer `503 Service Unavailable` failures were present in the log while the shared summary still reported `429/503=0`.
  - The rebuilt-binary exact-target replay finished in `487.71s` on storage winner `aay7nawy...`, again produced `3180` effective entries / `2533` files / `647` folders, and now surfaced the late throttle burst honestly: `429/503=2`, `failovers=2`, and both `503` child failures were classified as `kind=throttle` with phantom-swap healing triggered immediately.
  - The new final accounting line is now correct for the Qilin/VFS path: `raw entries=0 effective entries=3180 (files=2533 folders=647)`.
  - The new stall guard did not fire on either live full crawl. That is the correct outcome for these runs: the queue kept making progress, so the issue was deep-layer host/circuit slowness rather than a true no-progress stall.

## Phase 88: Binary Telemetry Parity + Clean Same-Output Restore Validation
- **Date**: 2026-03-10
- **Action**: Synced the binary telemetry/protobuf plane with the runtime resource-metrics snapshot, added compact final CLI summaries, and live-validated subtree host-memory restore by running the exact Qilin target twice against the same output root without an alarm wrapper.
- **Validation Commands**:
  - `cargo check --manifest-path 'crawli/src-tauri/Cargo.toml'`
  - `cargo test --manifest-path 'crawli/src-tauri/Cargo.toml' qilin --lib`
  - `cargo test --manifest-path 'crawli/src-tauri/Cargo.toml' cli::tests --lib`
  - `cargo test --manifest-path 'crawli/src-tauri/Cargo.toml' binary_telemetry::tests --lib`
  - `npm run build`
  - `./crawli/src-tauri/target/debug/crawli --progress-summary --progress-summary-interval-ms 5000 crawl --url 'http://ijzn3sicrcy7guixkzjkib4ukbiilwc3xhnmby4mcbccnsd7j2rekvqd.onion/site/view?uuid=afa2a0ea-20ba-3ddf-8c5c-2aeea9e5dc43' --output-dir '/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli/tmp/live_cli_qilin_afa2a0ea_20260310_restore_validation/output' --daemons 4 --circuits 120`
  - `./crawli/src-tauri/target/debug/crawli --progress-summary --progress-summary-interval-ms 5000 crawl --url 'http://ijzn3sicrcy7guixkzjkib4ukbiilwc3xhnmby4mcbccnsd7j2rekvqd.onion/site/view?uuid=afa2a0ea-20ba-3ddf-8c5c-2aeea9e5dc43' --output-dir '/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli/tmp/live_cli_qilin_afa2a0ea_20260310_restore_validation/output' --daemons 4 --circuits 120`
- **Result**:
  - Rust and frontend validation remained green after the telemetry parity pass: `qilin --lib` passed `31`, `cli::tests` passed `8`, `binary_telemetry::tests` passed `1`, and `cargo check` / `npm run build` completed cleanly.
  - The binary telemetry path is now in lockstep with the runtime snapshot. `ResourceMetricsFrame` carries throttle-rate, phantom-pool depth, `subtree_reroutes`, `subtree_quarantine_hits`, and `off_winner_child_requests` through Rust, `telemetry.proto`, and regenerated JS/TS bindings.
  - The CLI now emits a one-shot `[summary:final]` line on completion. The exact-target run 1 ended with `req=685/650/35 subtree=0/0/0`; run 2 ended with `req=717/648/69 subtree=0/0/0`.
  - Run 1 completed cleanly in `165.19s`, confirmed a durable winner on `rbuio2ug7hnu5534smyt7wk6wsap7rpqylcjgpuewbyekfacpenug6qd.onion`, and persisted `647` subtree host preferences to `/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli/tmp/live_cli_qilin_afa2a0ea_20260310_restore_validation/output/targets/ijzn3sicrcy7guixkzjkib4ukbii__afa2a0ea-20ba-3ddf-8c5c-__35770556215d08e/qilin_subtree_route_summary.json`.
  - Run 2 reused that same output tree, loaded the `3180`-entry best-known baseline, logged `Restored 647 persisted subtree host preferences`, rotated to a different durable winner on `ytbhximfzof7vaaryjjenu3ow5gufxkzrx7vdjpbhyfbl745n4tt5aid.onion`, and still finished cleanly in `157.88s` with `crawlOutcome=matched_best`.
  - Same-output useful work was preserved despite the winner rotation. Both runs ended with `discoveredCount=3180`, `fileCount=2533`, and `folderCount=647`.

## Phase 87: Subtree Route Telemetry + Host-Based Memory Validation
- **Date**: 2026-03-10
- **Action**: Added shared subtree-route counters to `RuntimeTelemetry`, benchmark summaries, and the dashboard metrics plane; kept `--no-stealth-ramp` benchmark-only in main CLI behavior; and added host-based subtree preferred-route persistence after repeated live reruns proved cross-run winner churn again.
- **Validation Commands**:
  - `cargo check --manifest-path 'crawli/src-tauri/Cargo.toml'`
  - `cargo test --manifest-path 'crawli/src-tauri/Cargo.toml' qilin --lib`
  - `cargo test --manifest-path 'crawli/src-tauri/Cargo.toml' runtime_metrics::tests --lib`
  - `cargo test --manifest-path 'crawli/src-tauri/Cargo.toml' cli::tests --lib`
  - `npm run build`
  - `./crawli/src-tauri/target/debug/crawli --progress-summary --progress-summary-interval-ms 5000 crawl --url 'http://ijzn3sicrcy7guixkzjkib4ukbiilwc3xhnmby4mcbccnsd7j2rekvqd.onion/site/view?uuid=afa2a0ea-20ba-3ddf-8c5c-2aeea9e5dc43' --output-dir '/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli/tmp/live_cli_qilin_afa2a0ea_20260310_route_metrics_run1/output' --daemons 4 --circuits 120`
  - `./crawli/src-tauri/target/debug/crawli --progress-summary --progress-summary-interval-ms 5000 crawl --url 'http://ijzn3sicrcy7guixkzjkib4ukbiilwc3xhnmby4mcbccnsd7j2rekvqd.onion/site/view?uuid=afa2a0ea-20ba-3ddf-8c5c-2aeea9e5dc43' --output-dir '/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli/tmp/live_cli_qilin_afa2a0ea_20260310_route_metrics_run2/output' --daemons 4 --circuits 120`
  - `BENCHMARK_ADAPTER=qilin BENCHMARK_DURATION=30 cargo run --manifest-path 'crawli/src-tauri/Cargo.toml' --bin adapter-benchmark`
- **Result**:
  - Rust and frontend validation remained green after the telemetry/persistence tranche: `qilin --lib` passed `31`, `runtime_metrics::tests` passed `3`, `cli::tests` passed `7`, and `cargo check`/`npm run build` completed cleanly.
  - The new subtree-route counters are now first-class shared metrics. The short benchmark printed `[EFF] final requests=2 success=2 failure=0 req/entry=2.00 fingerprint=0ms cache_hits=0 subtree_reroutes=0 quarantine_hits=0 off_winner_child_requests=0`, and the final benchmark row now includes `SUB_RER`, `Q_HITS`, and `OFFWIN`.
  - Repeated exact-target live reruns proved cross-run winner churn again: the earlier subtree-affinity replay stabilized on `chygwjfx...`, route-metrics run 1 stabilized on `2wyohlh5...`, and route-metrics run 2 stabilized on `lqcxwo4c...`. That is why subtree preferred-host persistence is now enabled.
  - The persistence path is deliberately host-based, not full-URL-based, so stored subtree preferences only restore when the same host appears in the current winner/standby set. The new `persisted_subtree_preference_remaps_to_current_seed_host` unit test verifies that old UUID paths do not bleed into new runs.
  - Main CLI behavior now keeps `--no-stealth-ramp` benchmark-only unless `CRAWLI_ALLOW_BENCHMARK_FLAGS=1`; the benchmark binary still forces `stealth_ramp=false` for comparison work.

## Phase 86E: Subtree Host Affinity + Standby Quarantine Validation
- **Date**: 2026-03-10
- **Action**: Implemented subtree-aware host affinity, subtree-local standby quarantine, and route planning that keeps subtree health separate from global winner/root health. Revalidated on the exact live `afa2a0ea-20ba-3ddf-8c5c-2aeea9e5dc43` target with a focused replay after the new subtree routing helpers landed.
- **Validation Commands**:
  - `cargo check --manifest-path 'crawli/src-tauri/Cargo.toml'`
  - `cargo test --manifest-path 'crawli/src-tauri/Cargo.toml' qilin --lib`
  - `./crawli/src-tauri/target/debug/crawli --progress-summary --progress-summary-interval-ms 5000 crawl --url 'http://ijzn3sicrcy7guixkzjkib4ukbiilwc3xhnmby4mcbccnsd7j2rekvqd.onion/site/view?uuid=afa2a0ea-20ba-3ddf-8c5c-2aeea9e5dc43' --output-dir '/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli/tmp/live_cli_qilin_afa2a0ea_20260310_subtree_affinity/output' --daemons 4 --circuits 120`
- **Result**:
  - Targeted Rust validation remained green after the subtree-route changes: `28 passed`.
  - The exact-target replay confirmed a durable winner on `chygwjfxnehjkisuex7crh6mqlfbjs2cbr6drskdrf4gy4yyxbpcbsyd.onion`, logged `Qilin Root Parse: files=1 folders=1`, and parsed `kent/` as `133 files / 133 folders` again.
  - The timed `120.01s` replay ended at `seen=544 processed=282 queue=262 workers=16/16 failovers=0 timeouts=0`, while adapter-local progress reached `entries=2336 pending=216`.
  - Child-path routing stayed pinned to the confirmed winner instead of leaking onto standby nodes. Compared to the prior repaired `rootfix` run (`64` child fetches with `9` off-active, `33` child failures with `10` off-active), the subtree-affinity replay logged `64` child fetches with `0` off-active and `0` child failures.
  - No default `--no-stealth-ramp` change was made. It remains a benchmark-only knob until a stable target proves a clear useful-work gain over the repaired default ramp.

## Phase 98: Native-Webview Smoke Fast Path + Direct Benchmark Hardening
- **Date**: 2026-03-10
- **Action**: Implemented a native-webview smoke harness centered on real Tauri IPC instead of browser preview assumptions and added a first-class clearnet direct-download regression binary with believable transferred-byte accounting. Also hardened Qilin download repinning so subtree memory must satisfy stronger proof and a balanced host cap before saved URLs are rewritten.
- **Validation Commands**:
  - `cargo test --manifest-path 'crawli/src-tauri/Cargo.toml' --lib`
  - `npm run build`
  - `npx playwright test tests/crawli.spec.ts --project=chromium`
  - `DIRECT_BENCH_DURATION=15 cargo run --manifest-path 'crawli/src-tauri/Cargo.toml' --bin direct-download-benchmark`
  - `CRAWLI_NATIVE_SMOKE_TIMEOUT_MS=45000 CRAWLI_NATIVE_SMOKE_WAIT_MS=4000 npm run native:smoke`
- **Result**:
  - Core validation passed: Rust unit coverage remained green (`122/122`), the browser preview regression smoke stayed green (`3/3`), and the direct benchmark produced believable throughput after the accounting fix.
  - The direct benchmark now reports transferred bytes from persisted piece state or allocated-block size instead of logical sparse-file length. The latest March 10, 2026 rerun on `https://proof.ovh.net/files/10Gb.dat` produced `bytes=880099328`, `elapsed_secs=22.18`, and `throughput_mbps=317.45`; an earlier post-fix run on the same path produced about `385.63 Mbps`, so the reliable regression band is now `~317-386 Mbps`.
  - The native-webview smoke path itself is implemented but not yet fully validated on this local macOS host. After the new startup bootstrap bypass landed, the real debug `crawli` binary still failed to emit `native-webview-report.json` during the local smoke run, while browser preview remained healthy. Treat that as a platform-specific validation gap, not a browser-shell regression.
  - The Qilin download admission-bias change is currently unit-validated through `strong_subtree_route_is_required_before_download_repin`, `balanced_qilin_repin_cap_preserves_host_diversity`, and rotated alternate-host ordering tests. A live exact-target download rerun is still required before claiming a throughput gain on the onion path.

## Phase 98B: Production-Probe Benchmark Parity + Exact Qilin Download Replay
- **Date**: 2026-03-10
- **Action**: Switched `direct_download_benchmark.rs` to use `aria_downloader::probe_target(...)` instead of a synthetic `HEAD` probe, then re-ran both the direct clearnet benchmark and the exact-target Qilin timed download against the host-capped repin logic.
- **Validation Commands**:
  - `cargo build --manifest-path 'crawli/src-tauri/Cargo.toml' --bin direct-download-benchmark`
  - `DIRECT_BENCH_DURATION=15 ./crawli/src-tauri/target/debug/direct-download-benchmark`
  - `cargo test --manifest-path 'crawli/src-tauri/Cargo.toml' --lib strong_subtree_route_is_required_before_download_repin -- --nocapture`
  - `perl -e 'alarm shift; exec @ARGV' 150 ./crawli/src-tauri/target/debug/crawli --progress-summary --progress-summary-interval-ms 5000 download-files --entries-file '/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli/tmp/live_cli_qilin_afa2a0ea_20260310_phase90_reconciliation_fix/output/targets/ijzn3sicrcy7guixkzjkib4ukbii__afa2a0ea-20ba-3ddf-8c5c-__35770556215d08e/current/listing_canonical.json' --output-dir '/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli/tmp/live_qilin_download_phase98c/full_download' --connections 120 > '/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli/tmp/live_qilin_download_phase98c/stdout.log' 2> '/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli/tmp/live_qilin_download_phase98c/stderr.log'`
- **Result**:
  - The direct benchmark now reflects the exact production probe metadata: `range_mode=true`, `content_length=10737418240`, `resume_validator=etag`, `circuit_cap=32`, `active_start=16`, `tournament_cap=36`.
  - The latest rebuilt direct benchmark transferred `939671552` bytes in `15.20s`, about `58.95 MiB/s` / `494.51 Mbps`, confirming the direct path remains strong and that the benchmark no longer depends on a fake `HEAD` probe.
  - The exact-target Qilin rerun still failed before useful admission. The captured `150s` replay produced `0` payload files and `0` payload bytes in `/tmp/live_qilin_download_phase98c/full_download`, and the log showed repeated `GET Range probe timed out after 8s` messages plus one `client error (Connect)` before the batch ever reached meaningful work.
  - No `[Qilin Download] Repinned ...` line appeared in the captured run, which means the new host-capped repin logic did not materially engage; the bottleneck is still the initial probe stage, not post-probe host concentration.

## Phase 99: Downloader Transport Reuse + Telemetry Hardening
- **Date**: 2026-03-10
- **Action**: Implemented the first libcurl-inspired downloader transport tranche: conditional keep-alive policy, probe-to-transfer promotion for micro/small swarm files, shared host-capability cache, and low-speed abort counters across runtime/CLI/protobuf/UI telemetry.
- **Validation Commands**:
  - `cargo check --manifest-path '/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli/src-tauri/Cargo.toml'`
  - `cargo test --manifest-path '/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli/src-tauri/Cargo.toml' --lib`
  - `cd '/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli' && npx pbjs -t static-module -w es6 -o src/telemetry.js src/telemetry.proto`
  - `cd '/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli' && npx pbts -o src/telemetry.d.ts src/telemetry.js`
  - `cd '/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli' && npm run build`
- **Result**:
  - `cargo check` passed after the transport refactor with no compile errors in `aria_downloader.rs`.
  - Full Rust library validation passed: `124 passed; 0 failed`.
  - Protobuf bindings regenerated successfully and `npm run build` passed, proving the new counters do not break the frontend or binary telemetry schema.
  - No new throughput claim is attached to this phase yet. A follow-up live direct benchmark process was intentionally excluded from the result after it failed to produce a clean datapoint before session closure.

## Phase 101: IDM-Inspired Probe Admission Hardening
- **Date**: 2026-03-11
- **Action**: Investigated Internet Download Manager transport behavior from official IDM documentation and implemented the matching Qilin-side ideas that apply safely to hidden-service hosts: probe-stage degraded-host quarantine, health-aware probe candidate ordering, and pre-transfer alternate reseeding so failed probe hosts are demoted before the batch scheduler starts spending worker slots.
- **Validation Commands**:
  - `rustc -vV`
  - `cargo build --manifest-path '/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli/src-tauri/Cargo.toml' --bin crawli`
  - `cargo build --manifest-path '/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli/src-tauri/Cargo.toml' --bin crawli-cli`
  - `python3 - <<'PY' ... subprocess.run(['.../crawli-cli','detect-input-mode','--input','https://example.com'], timeout=120) ... PY`
  - `perl -e 'alarm shift; exec @ARGV' 150 ./crawli/src-tauri/target/debug/crawli-cli --progress-summary --progress-summary-interval-ms 5000 download-files --entries-file '/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli/tmp/live_cli_qilin_afa2a0ea_20260310_phase90_reconciliation_fix/output/targets/ijzn3sicrcy7guixkzjkib4ukbii__afa2a0ea-20ba-3ddf-8c5c-__35770556215d08e/current/listing_canonical.json' --output-dir '/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli/tmp/live_qilin_download_phase101c/full_download' --connections 120 > '/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli/tmp/live_qilin_download_phase101c/stdout.log' 2> '/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli/tmp/live_qilin_download_phase101c/stderr.log'`
- **Result**:
  - The local Rust problem was cleared. `rustc -vV` returned immediately, the main debug binary rebuilt in `3m49s`, and `crawli-cli` rebuilt in `10.70s`.
  - The correct replay surface on macOS is `crawli-cli`, not the GUI `crawli` binary. A small `detect-input-mode` CLI call completed in `11.79s`, which proves the console binary is functional but still pays a noticeable startup tax from the current Tauri-linked dependency graph.
  - The rebuilt exact-target `150s` replay still produced `0` payload files and `0` payload bytes. The visible output tree contained only `519` `.gitkeep` placeholders.
  - The new probe logic is active in the live path. `stderr.log` shows probe timeouts at `8s`, `12s`, and `16s`, then `Probe rotation` lines with `quarantined_candidates=2/3` and `3/3`, followed by `Probe routing` fallbacks that arm alternates on `lbln...` and `4xl2hta3...`.
  - The failure stage is now even narrower: `dl_transport` rose from `0/0/0` to `18/0/0`, but repeated `client error (Connect)` failures still killed first-wave admission before any payload bytes were written. Concurrency remains frozen.

## Phase 100: Active Per-Host Transfer Cap + Clean Direct/Qilin Replay
- **Date**: 2026-03-10
- **Action**: Implemented a live downloader-side active per-host permit ledger in `src-tauri/src/aria_downloader.rs`, then re-ran both the clean direct regression harness and the exact-target Qilin timed batch download with the new host-pressure behavior visible in logs and summaries.
- **Validation Commands**:
  - `cargo check --manifest-path '/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli/src-tauri/Cargo.toml'`
  - `cargo build --manifest-path '/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli/src-tauri/Cargo.toml' --bin direct-download-benchmark`
  - `DIRECT_BENCH_DURATION=15 ./crawli/src-tauri/target/debug/direct-download-benchmark`
  - `cargo build --manifest-path '/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli/src-tauri/Cargo.toml' --bin crawli`
  - `perl -e 'alarm shift; exec @ARGV' 150 ./crawli/src-tauri/target/debug/crawli --progress-summary --progress-summary-interval-ms 5000 download-files --entries-file '/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli/tmp/live_cli_qilin_afa2a0ea_20260310_phase90_reconciliation_fix/output/targets/ijzn3sicrcy7guixkzjkib4ukbii__afa2a0ea-20ba-3ddf-8c5c-__35770556215d08e/current/listing_canonical.json' --output-dir '/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli/tmp/live_qilin_download_phase100a/full_download' --connections 120 > '/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli/tmp/live_qilin_download_phase100a/stdout.log' 2> '/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli/tmp/live_qilin_download_phase100a/stderr.log'`
- **Result**:
  - The clean direct regression path stayed healthy after the new host-pressure control landed. The latest March 10, 2026 run reported `host_cap=32`, `bytes=758923264`, `elapsed_secs=15.39`, `throughput_mib_per_sec=47.04`, `throughput_mbps=394.59`, and `transport=0/0/1`.
  - The exact-target Qilin replay proved the live host cap is active on the onion path: the batch logged `host_cap_ceiling=4` for both `Phase 1 (Small)` and `Phase 0 (Micro)`, while compact summaries moved from `dl_transport=0/0/0` to `18/0/0`.
  - The onion useful-work result is still zero. The same captured replay produced `0` non-placeholder payload files and `0` payload bytes under `tmp/live_qilin_download_phase100a/full_download`.
  - The failure stage is now clearer, not worse: the log shows repeated `GET Range probe timed out after 8s`, `...after 12s`, `...after 16s`, plus `client error (Connect)` before any productive first-wave admission. That means the active host cap is working as intended, but the remaining bottleneck is still degraded-host probe admission rather than oversubscription.

## Phase 98C: Probe-Stage Alternate Remap + Graded Onion Probe Budgets
- **Date**: 2026-03-10
- **Action**: Moved Qilin download hardening deeper into the probe stage. The probe layer now applies graded onion probe budgets by attempt and keeps alternate-host probing inside transfer-mode selection instead of relying on later remap/requeue behavior alone.
- **Validation Commands**:
  - `cargo build --manifest-path 'crawli/src-tauri/Cargo.toml' --bin crawli`
  - `perl -e 'alarm shift; exec @ARGV' 150 ./crawli/src-tauri/target/debug/crawli --progress-summary --progress-summary-interval-ms 5000 download-files --entries-file '/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli/tmp/live_cli_qilin_afa2a0ea_20260310_phase90_reconciliation_fix/output/targets/ijzn3sicrcy7guixkzjkib4ukbii__afa2a0ea-20ba-3ddf-8c5c-__35770556215d08e/current/listing_canonical.json' --output-dir '/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli/tmp/live_qilin_download_phase98d/full_download' --connections 120 > '/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli/tmp/live_qilin_download_phase98d/stdout.log' 2> '/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli/tmp/live_qilin_download_phase98d/stderr.log'`
- **Result**:
  - The captured exact-target replay proves the new graded budgets are active. `stderr.log` now shows `GET Range probe timed out after 8s` followed by `GET Range probe timed out after 12s`, which did not exist in the earlier flat-budget runs.
  - Even with the wider alternate-probe window, the run still produced `0` payload files and `0` payload bytes in `/tmp/live_qilin_download_phase98d/full_download`.
  - The batch still collapsed before productive admission: the same log also shows repeated `client error (Connect)` failures during the probe phase, and no probe-stage success/remap line was emitted. The next fix therefore needs to quarantine degraded probe hosts earlier and rotate the alternate-host cursor more aggressively before the file ever reaches transfer scheduling.

## Phase 86D: Root Durability + Active-Host Affinity Validation
- **Date**: 2026-03-10
- **Action**: Implemented durable winner promotion, live-client phantom replenishment, root-retry remap protection, and first-child-retry active-host affinity. Revalidated on the exact live `afa2a0ea-20ba-3ddf-8c5c-2aeea9e5dc43` target, then ran a `--no-stealth-ramp` comparison pass after root durability was restored.
- **Validation Commands**:
  - `cargo check --manifest-path 'crawli/src-tauri/Cargo.toml'`
  - `cargo test --manifest-path 'crawli/src-tauri/Cargo.toml' qilin --lib`
  - `./crawli/src-tauri/target/debug/crawli --progress-summary --progress-summary-interval-ms 5000 crawl --url 'http://ijzn3sicrcy7guixkzjkib4ukbiilwc3xhnmby4mcbccnsd7j2rekvqd.onion/site/view?uuid=afa2a0ea-20ba-3ddf-8c5c-2aeea9e5dc43' --output-dir '/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli/tmp/live_cli_qilin_afa2a0ea_20260310_rootfix/output' --daemons 4 --circuits 120`
  - `./crawli/src-tauri/target/debug/crawli --progress-summary --progress-summary-interval-ms 5000 crawl --url 'http://ijzn3sicrcy7guixkzjkib4ukbiilwc3xhnmby4mcbccnsd7j2rekvqd.onion/site/view?uuid=afa2a0ea-20ba-3ddf-8c5c-2aeea9e5dc43' --output-dir '/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli/tmp/live_cli_qilin_afa2a0ea_20260310_nostealth/output' --daemons 4 --circuits 120 --no-stealth-ramp`
- **Result**:
  - Targeted Rust validation remained green after the new route-affinity helper landed: `25 passed`.
  - The exact-target repaired run reached the real listing path again instead of stalling before durable enumeration. It logged `Qilin Root Parse: files=1 folders=1`, confirmed a durable storage winner, parsed `kent/` as `133 files / 133 folders`, and by the timed `180.02s` cutoff reached `seen=288 processed=59 queue=229 workers=15/16 failovers=5 timeouts=5` while adapter-local progress climbed to `entries=670`.
  - Persisted subtree state now reflects successful production work instead of only failed retries: `qilin_bad_subtrees.json` captured fresh successes for `kent/2012`, `kent/2016`, `kent/2017`, `kent/AARP medical filing`, `kent/Chase Bank`, and `kent/Credit Protection`.
  - The pre-fix live rerun on the same target never logged a real root parse or durable winner confirmation and did not expand into large-scale file/folder enumeration; the repaired path does.
  - The `--no-stealth-ramp` comparison also reached a durable winner and restored tree parsing (`kent/` again at `133 files / 133 folders`) and hit `14/16` workers quickly, but it did not show a clear useful-work advantage over the repaired default path. Keep `no_stealth_ramp` as a benchmark knob for now, not the default.

## Phase 86C: Arti Hot-Start + Hinted Warmup Bypass Live Validation
- **Date**: 2026-03-10
- **Action**: Implemented and validated four more Qilin/Arti runtime reductions on the exact live CMS target: seeded `MultiClientPool` reuse from the active swarm, frontier live-client refresh before hinted onion execution, a Stage D first-wave stable-slot reservation, and a strong-URL-hint bypass for the blocking onion warmup itself.
- **Validation Commands**:
  - `cargo build --manifest-path 'crawli/src-tauri/Cargo.toml' --bin crawli`
  - `cargo test --manifest-path 'crawli/src-tauri/Cargo.toml' qilin --lib`
  - `./crawli/src-tauri/target/debug/crawli --progress-summary --progress-summary-interval-ms 5000 crawl --url 'http://ijzn3sicrcy7guixkzjkib4ukbiilwc3xhnmby4mcbccnsd7j2rekvqd.onion/site/view?uuid=afa2a0ea-20ba-3ddf-8c5c-2aeea9e5dc43' --output-dir '/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli/tmp/live_cli_qilin_afa2a0ea_20260310_ultimate/output' --daemons 4 --circuits 120`
- **Result**:
  - Targeted Rust validation remained green after the new changes: `22 passed`.
  - The seeded-pool rerun removed the old `storage resolved -> first circuit hot` delay entirely. The previous good run needed about `55.05s` between `Storage Node Resolved (+39.12s)` and `First circuit hot (+94.17s)` inside the Qilin phase; the seeded-pool path now logs `First circuit hot` at `+0.00s` relative to pool boot.
  - The intermediate pre-skip rerun proved the Stage D stable-slot reservation can prevent the old mirror-timeout path when the live storage host stays valid: that run resolved `ai55k7agm5ly...onion` inside Stage D at `+48.68s` instead of timing out into the `+97.72s` direct-mirror fallback seen on the prior seeded rerun.
  - The final hinted-path warmup bypass cut the global adapter handoff on the same URL from `138.83s` to `71.08s`, saving `67.75s`, because the crawl no longer waits on a blocking onion warmup immediately before a skipped fingerprint.
  - The final five-minute run still timed out at `300.025s` with only `seen=2 processed=1 queue=1`. The remaining live blockers are no longer fingerprinting or pool boot; they are Stage A volatility, root durability after discovery, and phantom-pool depletion (`[Aerospace Healing Warning] Phantom pool empty for circuit 0. Re-bootstrapping.`).

## Five-Minute Live Crawl Audit
- **Date**: 2026-03-10
- **Target**: `http://ijzn3sicrcy7guixkzjkib4ukbiilwc3xhnmby4mcbccnsd7j2rekvqd.onion/site/view?uuid=afa2a0ea-20ba-3ddf-8c5c-2aeea9e5dc43`
- **Action**: Ran the main `crawli` binary under a hard-timed live controller with `--progress-summary`, `--daemons 4`, and `--circuits 120`, capturing absolute-timestamped stdout/stderr plus a run metadata file.
- **Validation Command**:
  - `./crawli/src-tauri/target/debug/crawli --progress-summary --progress-summary-interval-ms 5000 crawl --url 'http://ijzn3sicrcy7guixkzjkib4ukbiilwc3xhnmby4mcbccnsd7j2rekvqd.onion/site/view?uuid=afa2a0ea-20ba-3ddf-8c5c-2aeea9e5dc43' --output-dir '/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli/tmp/live_cli_qilin_afa2a0ea_20260310_5m/output' --daemons 4 --circuits 120`
- **Result**: The crawl started at `2026-03-10T04:32:10.013130-05:00` and finished cleanly at `2026-03-10T04:36:48.338354-05:00` (`278.323s`). Arti quorum was reached at `58.2s`, circuit prewarm completed at `104.4s`, and the live CLI path still spent `18.2s` on fingerprinting, so the Phase 86 Qilin URL-hint bypass did not fire on this route. Qilin Stage A discovered a fresh rotated storage host at `143.8s`, Phase 30 hit its `90s` discovery timeout before falling back to direct mirrors and Phase 77 parsing, and the final output contained `0` files and `0` folders in the canonical/current listings because the resolved page parsed as a non-QData CMS view (`qdata=false`, `data_browser=false`, `table=false`, `hrefs=10`) rather than a downloadable listing.

## Phase 86: Arti Fingerprint Bypass + Discovery Telemetry
- **Date**: 2026-03-10
- **Action**: Added strong Qilin/CMS URL-hint bypass for fingerprinting, wired discovery-plane request accounting into runtime telemetry, deferred broad mirror seeding until the cached node pool is sparse, and exposed swarm-efficiency charts in the React dashboard.
- **Validation Commands**:
  - `cargo check --manifest-path 'crawli/src-tauri/Cargo.toml'`
  - `npm run build`
  - `cargo test --manifest-path 'crawli/src-tauri/Cargo.toml' qilin --lib`
  - `BENCHMARK_ADAPTER=qilin BENCHMARK_DURATION=45 cargo run --manifest-path 'crawli/src-tauri/Cargo.toml' --bin adapter-benchmark`
- **Result**: Rust and frontend builds passed, the targeted Qilin suite passed (`20 passed`), and the live Qilin/Arti benchmark now reports `FP_SECS=0.00` plus non-zero discovery efficiency (`requests=3`, `success=1`, `failure=2`). The warm rerun no longer broad-seeds the full fallback mirror set before Stage A; the remaining blocker is live storage-host volatility after redirect capture.

## Phase 85: Arti Swarm Efficiency Audit
- **Date**: 2026-03-10
- **Action**: Implemented the full Phase 85 runtime corrections, then revalidated them against the live Qilin benchmark target with Arti swarm behavior in focus.
- **Validation Commands**:
  - `cargo check --manifest-path 'crawli/src-tauri/Cargo.toml'`
  - `cargo test --manifest-path 'crawli/src-tauri/Cargo.toml' qilin --lib`
  - `BENCHMARK_ADAPTER=qilin BENCHMARK_DURATION=20 cargo run --manifest-path 'crawli/src-tauri/Cargo.toml' --bin adapter-benchmark`
  - `BENCHMARK_ADAPTER=qilin BENCHMARK_DURATION=5 cargo run --manifest-path 'crawli/src-tauri/Cargo.toml' --bin adapter-benchmark`
- **External Sources Reviewed**:
  - Tor path selection constraints
  - Tor path weighting
  - Arti config options
  - `arti-client` rustdocs for `StreamPrefs` / `IsolationToken`
  - Google "The Tail at Scale"
- **Result**: `cargo check` passed and the targeted Qilin test set passed (`20 passed`). The live Qilin benchmark cold run captured a fresh Stage A redirect on `6eoxnxd2y5xryvgyh22k3wknwrmw7w5i7l4yi2tu57w52v5ttjcif4ad.onion`, while the warm-cache rerun hit the cached winner lease before mirror seeding. The dominant remaining live bottleneck is fingerprint wall time, not storage rediscovery.

## Phase 84: Qilin Telemetry Alignment & Live Parity Validation
- **Date**: 2026-03-10
- **Action**: Added a frontier-level adapter progress overlay, repaired Qilin fast-path request accounting, introduced compact CLI summaries, and revalidated the same Qilin target on both the main-binary CLI and the live Tauri GUI.
- **Validation Commands**:
  - `cargo check --manifest-path 'crawli/src-tauri/Cargo.toml'`
  - `cargo test --manifest-path 'crawli/src-tauri/Cargo.toml' cli::tests --quiet`
  - `cargo test --manifest-path 'crawli/src-tauri/Cargo.toml' frontier::tests --quiet`
  - `./crawli/src-tauri/target/debug/crawli --progress-summary --progress-summary-interval-ms 3000 crawl --url 'http://ijzn3sicrcy7guixkzjkib4ukbiilwc3xhnmby4mcbccnsd7j2rekvqd.onion/site/view?uuid=f0668431-ee3f-3570-99cb-ea7d9c0691c6' --output-dir '/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli/tmp/live_cli_qilin_phase84'`
  - `npm run dev -- --host 127.0.0.1 --port 1420` followed by `./src-tauri/target/debug/crawli`, then use the native app window to enter the same target URL and press `Enter`
- **Result**: The CLI summary now surfaces non-zero Qilin worker activity during live traversal, and the GUI path reached the same bootstrap / fingerprint / Stage A rotated storage discovery milestones on the actual app window.
- **Operator Policy**: Use `--progress-summary` for long live crawls. Keep `--include-telemetry-events` as an escalation flag for raw bridge-frame inspection only.

## Main-Binary CLI Validation
- **Date**: 2026-03-10
- **Action**: Added a first-class headless CLI path to the primary `crawli` binary (`src-tauri/src/cli.rs`) and validated it through the real Tauri crate instead of examples/helper binaries.
- **Validation Commands**:
  - `cargo check --manifest-path 'crawli/src-tauri/Cargo.toml'`
  - `cargo test --manifest-path 'crawli/src-tauri/Cargo.toml' cli::tests`
  - `cargo run --quiet --manifest-path 'crawli/src-tauri/Cargo.toml' -- adapter-catalog --compact-json`
  - `cargo run --quiet --manifest-path 'crawli/src-tauri/Cargo.toml' -- detect-input-mode --input 'http://ijzn3sicrcy7guixkzjkib4ukbiilwc3xhnmby4mcbccnsd7j2rekvqd.onion/site/view?uuid=f0668431-ee3f-3570-99cb-ea7d9c0691c6' --compact-json`
  - `cargo run --quiet --manifest-path 'crawli/src-tauri/Cargo.toml' -- crawl --url 'http://ijzn3sicrcy7guixkzjkib4ukbiilwc3xhnmby4mcbccnsd7j2rekvqd.onion/site/view?uuid=f0668431-ee3f-3570-99cb-ea7d9c0691c6' --output-dir '/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli/tmp/live_cli_qilin_f0668431'`
- **Result**: The main binary successfully handled CLI smoke commands and the live Qilin crawl path reached real storage discovery plus recursive child parsing on the rotated storage node.
- **Operator Policy**: Default CLI stderr should stream human-useful events only. Enable `--include-telemetry-events` explicitly when raw bridge-frame inspection is needed.

## Vitest Frontend Component Testing
- **Date**: 2026-03-08
- **Action**: Installed `@testing-library/react`, `@testing-library/jest-dom`, and `@testing-library/user-event` to provide isolated component smoke tests via Vitest. Added `src/setupTests.ts` to mock Tauri OS-level APIs in a Node `jsdom` context.
- **Coverage**: Addressed 0% coverage gaps by wiring `VibeLoader.test.tsx`, `Dashboard.test.tsx`, `VfsTreeView.test.tsx`, `VFSExplorer.test.tsx`, and `AzureConnectivityModal.test.tsx`.
- **Purpose**: Fast feedback loop without launching Headless Chrome E2E via Playwright. Playwright tests (`tests/*.spec.ts`) continue to reign for integration tests (Port 0 mapping).


### Persistent Scale Validation (Phase 107.5)
Sustained 3+ hour Qilin crawls verified via live 15-second CLI RSS telemetry updates natively asserting no hidden heap bloat. Any future large-scale node collection MUST reside on sled engines instead of Vec instances.
