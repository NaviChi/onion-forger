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
