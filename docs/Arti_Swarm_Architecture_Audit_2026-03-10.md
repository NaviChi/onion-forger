# Arti Swarm Architecture Audit

Date: 2026-03-10
Scope: Arti swarm bootstrap, healing loop, Qilin storage-node discovery, request efficiency, and dashboard/chart telemetry for the current `crawli` Tauri/Rust stack.

## Executive Summary

The current repo already contains many of the right ideas: direct Arti transport, adaptive Qilin concurrency, sticky storage-node winners, bounded failover, and downloader/crawler separation concepts. The next material gains are no longer "more circuits"; they come from removing places where the runtime still waits on the slowest path, heals on the wrong signal, or discards learned node history.

The three highest-value code issues found in the current implementation are:

1. Warmup still waits for the slowest prewarm task before fingerprinting starts.
2. The Arti health monitor still judges onion workloads using a clearnet probe target.
3. Qilin mirror seeding overwrites learned per-node cooldown and success history every run.

Each of those issues can waste tens of seconds or dozens of unnecessary probes in a hostile session. Fixing them should produce better first-byte latency, better automatic healing, lower background bandwidth burn, and fewer wasted requests per discovered entry.

## Phase 89 Update (2026-03-10, deep-crawl stall audit + throttle/outlier repair follow-up)

The next Arti/Qilin tranche has now also been implemented and revalidated:

- `src-tauri/src/adapters/qilin_nodes.rs` now bounds the cached fast path to two deduplicated probes before Stage A, so reruns do not waste long serial probe chains when the cached winner is stale.
- `src-tauri/src/adapters/qilin.rs` now classifies first-attempt `503/429/403/400` failures as throttle-class failures, sends them through circuit isolation and telemetry, and includes a governor-side latency-outlier stall guard for true no-progress windows.
- `src-tauri/src/lib.rs` now reports effective entries at crawl completion when the adapter succeeded via direct VFS streaming rather than returning raw in-memory entries.
- Two full exact-target Qilin crawls were used to distinguish deadlock from route-quality variance on `afa2a0ea-20ba-3ddf-8c5c-2aeea9e5dc43`.

Validation completed after the deep-crawl follow-up:

- `cargo check --manifest-path 'crawli/src-tauri/Cargo.toml'`
- `cargo test --manifest-path 'crawli/src-tauri/Cargo.toml' qilin --lib`
- `cargo build --manifest-path 'crawli/src-tauri/Cargo.toml' --bin crawli`
- two full exact-target reruns against `http://ijzn3sicrcy7guixkzjkib4ukbiilwc3xhnmby4mcbccnsd7j2rekvqd.onion/site/view?uuid=afa2a0ea-20ba-3ddf-8c5c-2aeea9e5dc43`

Observed runtime outcomes:

- The first full crawl finished in `139.86s` on storage winner `3pe26tqc...` and produced the full `3180`-entry tree, but it exposed the old blind spot: `64` deep-layer `503` failures were present while the shared summary still reported `429/503=0`.
- The rebuilt-binary replay finished in `487.71s` on storage winner `aay7nawy...`, produced the same `3180` effective entries / `2533` files / `647` folders, and surfaced the late throttle burst honestly: `429/503=2`, `failovers=2`, and both failing child requests were classified as `kind=throttle`.
- The new final accounting line now reflects real useful work for the Qilin/VFS path: `raw entries=0 effective entries=3180 (files=2533 folders=647)`.
- The new stall guard did not fire on either live full crawl. That is informative: the queue never flatlined, so the remaining bottleneck is winner-host and circuit quality variance, not a hidden crawl deadlock.
- The remaining performance ceiling is now winner selection and tail-latency biasing. The same exact target can vary from `139.86s` to `487.71s` end-to-end purely from the chosen storage winner and late circuit quality.

## Phase 90 Update (2026-03-10, winner-quality memory + tail-latency biasing follow-up)

The next Arti/Qilin tranche has now also been implemented and revalidated:

- `src-tauri/src/adapters/qilin_nodes.rs` now persists productive winner quality and uses it when ranking cached fast-path probes, redirect-ring candidates, and Stage D fallbacks.
- `src-tauri/src/runtime_metrics.rs`, `src-tauri/src/cli.rs`, `src-tauri/src/binary_telemetry.rs`, `src-tauri/src/telemetry_bridge.rs`, and `src/telemetry.proto` now publish compact tail-latency facts (`winner_host`, `slowest_circuit`, `late_throttles`, `outlier_isolations`) across the same operator surfaces as the existing request metrics.
- `src-tauri/src/adapters/qilin.rs` now adapts worker repin cadence from winner quality and no longer resets late-tail retry history during reconciliation; reconciliation also has a hard wall-clock budget now.

Validation completed after the winner-quality follow-up:

- `cargo check --manifest-path 'crawli/src-tauri/Cargo.toml'`
- `cargo test --manifest-path 'crawli/src-tauri/Cargo.toml' --lib`
- `cargo build --manifest-path 'crawli/src-tauri/Cargo.toml' --bin crawli`
- `npm run build`
- repeated exact-target reruns against `http://ijzn3sicrcy7guixkzjkib4ukbiilwc3xhnmby4mcbccnsd7j2rekvqd.onion/site/view?uuid=afa2a0ea-20ba-3ddf-8c5c-2aeea9e5dc43`

Observed runtime outcomes:

- The first degraded exact-target run reproduced the late-tail bug the operator had been describing: the crawl looked healthy early, then near `99.4%` it reopened `27` missing folders as fresh work and started tail-churning across degraded hosts.
- The rebuilt rerun confirmed the fix. On durable winner `4xl2hta3ohg474n3onnbtmjnopsrwzorosgqt33sxbavthua3q5bn7qd.onion`, the crawl completed in `213.52s` with `3180` effective entries / `2533` files / `647` folders, `failovers=0`, `timeouts=0`, and final tail telemetry `winner_host=4xl2hta3... slowest_circuit=c0:2436ms late_throttles=0 outlier_isolations=0`.
- The next warm rerun showed the remaining limitation precisely. Productive winner memory is active because the crawl probed cached winner `4xl2hta3...` first, but fresh Stage A discovery later accepted `sc2qyv6...` and degraded into heavy subtree reroute churn by `84.0%` progress (`failovers=547`, `timeouts=11` in the captured tail).
- The performance ceiling is therefore narrower now: productive-winner bias exists, but fresh-host admission is still too permissive on warm reruns. The next step is stronger winner stickiness, not more default concurrency.

## Phase 88 Update (2026-03-10, telemetry parity + clean restore validation follow-up)

The next Arti/Qilin tranche has now also been implemented and revalidated:

- `src-tauri/src/binary_telemetry.rs`, `src-tauri/src/telemetry_bridge.rs`, and `src/telemetry.proto` now agree on the full resource-metrics payload, including throttle-rate, phantom-pool, and subtree-route counters.
- `src/telemetry.js` and `src/telemetry.d.ts` were regenerated from `src/telemetry.proto`, so the frontend ring-buffer decoder now sees the same binary telemetry fields as the Rust emitter.
- `src-tauri/src/cli.rs` now emits a compact `[summary:final]` line with request totals and subtree-route counters at crawl shutdown.
- The exact target was re-run twice against the same output root without an alarm wrapper to validate live restore of the new subtree host-memory layer.

Validation completed after the telemetry-parity follow-up:

- `cargo check --manifest-path 'crawli/src-tauri/Cargo.toml'`
- `cargo test --manifest-path 'crawli/src-tauri/Cargo.toml' qilin --lib`
- `cargo test --manifest-path 'crawli/src-tauri/Cargo.toml' cli::tests --lib`
- `cargo test --manifest-path 'crawli/src-tauri/Cargo.toml' binary_telemetry::tests --lib`
- `npm run build`
- two clean exact-target reruns against the same output root for `http://ijzn3sicrcy7guixkzjkib4ukbiilwc3xhnmby4mcbccnsd7j2rekvqd.onion/site/view?uuid=afa2a0ea-20ba-3ddf-8c5c-2aeea9e5dc43`

Observed runtime outcomes:

- Run 1 completed cleanly in `165.19s`, confirmed a durable winner on `rbuio2ug7hnu5534smyt7wk6wsap7rpqylcjgpuewbyekfacpenug6qd.onion`, and persisted `647` subtree host preferences.
- Run 2 reused that exact output tree, logged `Restored 647 persisted subtree host preferences`, rotated to a different durable winner on `ytbhximfzof7vaaryjjenu3ow5gufxkzrx7vdjpbhyfbl745n4tt5aid.onion`, and still completed cleanly in `157.88s`.
- Same-output useful work remained stable across the winner rotation: both runs ended with `discoveredCount=3180`, `fileCount=2533`, and `folderCount=647`.
- The new compact final summaries exposed request efficiency directly: run 1 finished with `req=685/650/35 subtree=0/0/0`, and run 2 finished with `req=717/648/69 subtree=0/0/0`.
- The subtree-route counters stayed at zero on these successful reruns, which is expected for the stable-path case and means the next validation target is a controlled degraded-route scenario rather than another happy-path replay.

## Phase 87 Update (2026-03-10, route telemetry + cross-run memory follow-up)

The next Arti/Qilin tranche has now also been implemented and revalidated:

- `src-tauri/src/runtime_metrics.rs` now exports subtree reroutes, subtree quarantine hits, and off-winner child requests through the shared runtime metrics snapshot.
- `src-tauri/src/bin/adapter_benchmark.rs` and `src-tauri/src/bin/crawl_stats.rs` now report those counters directly in benchmark output and CSV results.
- `src-tauri/src/adapters/qilin.rs` now persists subtree preferred hosts per target by host identity and restores them only when the same host is still present in the current winner/standby candidate set.
- `src-tauri/src/cli.rs` now keeps `--no-stealth-ramp` benchmark-only in behavior unless `CRAWLI_ALLOW_BENCHMARK_FLAGS=1`.

Validation completed after the route-telemetry follow-up:

- `cargo check --manifest-path 'crawli/src-tauri/Cargo.toml'`
- `cargo test --manifest-path 'crawli/src-tauri/Cargo.toml' qilin --lib`
- `cargo test --manifest-path 'crawli/src-tauri/Cargo.toml' runtime_metrics::tests --lib`
- `cargo test --manifest-path 'crawli/src-tauri/Cargo.toml' cli::tests --lib`
- `npm run build`
- repeated exact-target reruns against `http://ijzn3sicrcy7guixkzjkib4ukbiilwc3xhnmby4mcbccnsd7j2rekvqd.onion/site/view?uuid=afa2a0ea-20ba-3ddf-8c5c-2aeea9e5dc43`
- `BENCHMARK_ADAPTER=qilin BENCHMARK_DURATION=30 cargo run --manifest-path 'crawli/src-tauri/Cargo.toml' --bin adapter-benchmark`

Observed runtime outcomes:

- Repeated exact-target reruns proved cross-run winner churn again: the earlier subtree-affinity run stabilized on `chygwjfx...`, route-metrics run 1 on `2wyohlh5...`, and route-metrics run 2 on `lqcxwo4c...`.
- Because that churn repeated, host-based subtree preferred-route persistence is now justified. The new persistence layer only restores a subtree preference when the same host survives into the current candidate set.
- The short benchmark now prints route-efficiency counters directly: `subtree_reroutes`, `quarantine_hits`, and `off_winner_child_requests` are visible both in the `[EFF]` line and the final summary row.
- Default operator behavior still keeps `stealth_ramp` enabled. The override remains benchmark-only unless a deliberate benchmark env override is present.

## Phase 86E Update (2026-03-10, subtree affinity follow-up)

The next Arti/Qilin tranche has now also been implemented and revalidated on the same exact CMS target:

- `src-tauri/src/adapters/qilin.rs` now keeps subtree-local preferred seeds, subtree-local host-health, and subtree-local standby quarantine separate from the global winner/root health.
- Child requests now remap to a subtree-preferred seed only when that mapping is still healthy for the subtree itself; quarantined subtree standbys are skipped instead of soaking repeat attempts.
- Global failover rules are now reserved for root/global failures, so ordinary subtree retries do not rewrite the winner lease.

Validation completed after the subtree-affinity follow-up:

- `cargo check --manifest-path 'crawli/src-tauri/Cargo.toml'`
- `cargo test --manifest-path 'crawli/src-tauri/Cargo.toml' qilin --lib`
- controlled exact-target replay against `http://ijzn3sicrcy7guixkzjkib4ukbiilwc3xhnmby4mcbccnsd7j2rekvqd.onion/site/view?uuid=afa2a0ea-20ba-3ddf-8c5c-2aeea9e5dc43`

Observed runtime outcomes:

- The exact-target `120.01s` replay confirmed a durable winner on `chygwjfxnehjkisuex7crh6mqlfbjs2cbr6drskdrf4gy4yyxbpcbsyd.onion`, logged `Qilin Root Parse: files=1 folders=1`, and parsed `kent/` as `133 files / 133 folders`.
- The same replay ended at `seen=544 processed=282 queue=262 workers=16/16 failovers=0 timeouts=0`, with adapter-local progress reaching `entries=2336 pending=216`.
- Compared with the prior repaired `rootfix` run, off-winner child fetches/failures fell from `9/10` to `0/0`; subtree standby churn is no longer the dominant live waste on this target.
- The March 10, 2026 `--no-stealth-ramp` comparison remains benchmark-only. Once route churn is fixed, the default ramp no longer looks like the primary limiter.

## Phase 86 Update (2026-03-10, continued)

The next tranche of this audit has now also been partially implemented:

- Known Qilin/CMS ingress URLs now bypass the network fingerprint request entirely through strong URL-hint adapter matching in `src-tauri/src/lib.rs`, `src-tauri/src/bin/adapter_benchmark.rs`, and `src-tauri/src/bin/crawl_stats.rs`.
- Arti swarm telemetry now exposes runtime label, traffic class, ready client count, managed port count, health probe target, request totals, fingerprint latency, and cached-route hits through the runtime metrics plane and the React dashboard.
- Qilin discovery now records Stage A/B/D request outcomes into shared telemetry and defers broad mirror seeding until the cached node pool is actually sparse.

Validation completed after the Phase 86 follow-up:

- `cargo check --manifest-path 'crawli/src-tauri/Cargo.toml'`
- `npm run build`
- `cargo test --manifest-path 'crawli/src-tauri/Cargo.toml' qilin --lib`
- `BENCHMARK_ADAPTER=qilin BENCHMARK_DURATION=45 cargo run --manifest-path 'crawli/src-tauri/Cargo.toml' --bin adapter-benchmark`

Observed runtime outcomes:

- March 10, 2026 live Qilin reruns now report `FP_SECS=0.00` for the canonical CMS URL because the fingerprint GET is skipped completely.
- The latest warm rerun went from `Fast Path — Probing cached redirect hint` directly into Stage A; it did not broad-seed 27 fallback hosts first.
- The benchmark now reports non-zero discovery traffic (`requests=3`, `success=1`, `failure=2`) instead of the misleading `0` request plane seen before telemetry was wired into Qilin discovery.
- The remaining live bottleneck is rotated storage-host reachability: Stage A still discovers fresh storage hosts, but the resolved storage node often becomes unreachable before listing validation completes.

## Phase 86C Update (2026-03-10, Arti hot-start follow-up)

The next Arti/Qilin tranche has now also been implemented and revalidated on the same live CMS target:

- `src-tauri/src/multi_client_pool.rs` now seeds the Qilin/DragonForce `MultiClientPool` from already-hot swarm clients and expands the rest of the slots with cheap `TorClient::isolated_client()` handles instead of cold-bootstrapping a second vanguard. Per the `arti-client` rustdocs, these handles share internal state while keeping streams on distinct circuits.
- `src-tauri/src/frontier.rs` / `src-tauri/src/lib.rs` now refresh the frontier's live Arti clients before hinted onion execution so the crawl is not capped by the ready-at-return snapshot from the bootstrap quorum.
- `src-tauri/src/lib.rs` now skips the blocking onion warmup on strong URL-hint paths, because there is no reason to pay a 45s warmup right before a skipped fingerprint and immediate adapter execution.
- `src-tauri/src/adapters/qilin_nodes.rs` now reserves first-wave Stage D capacity for a stable cached winner instead of allowing two fresh redirect candidates to crowd out the proven node entirely.

Validation completed after the hot-start follow-up:

- `cargo build --manifest-path 'crawli/src-tauri/Cargo.toml' --bin crawli`
- `cargo test --manifest-path 'crawli/src-tauri/Cargo.toml' qilin --lib`
- repeated controlled five-minute runs against `http://ijzn3sicrcy7guixkzjkib4ukbiilwc3xhnmby4mcbccnsd7j2rekvqd.onion/site/view?uuid=afa2a0ea-20ba-3ddf-8c5c-2aeea9e5dc43`

Observed runtime outcomes:

- The seeded-pool path removed the old `~55s` `storage resolved -> first circuit hot` gap that was previously spent cold-bootstrapping a second Arti pool after storage resolution.
- The Stage D stable-slot reservation produced one live rerun where storage resolution happened inside Stage D at `+48.68s` instead of falling through to the `+97.72s` direct-mirror retry path.
- The strong-hint warmup bypass moved global adapter handoff from `138.83s` to `71.08s` on the same target, saving `67.75s`.
- Residual bottlenecks are now root durability and phantom-pool depletion, not fingerprint cost or second-pool bootstrap cost.

## Phase 86D Update (2026-03-10, root durability follow-up)

The next Arti/Qilin tranche has now also been implemented and revalidated on the same exact CMS target:

- `src-tauri/src/adapters/qilin_nodes.rs` now promotes winner leases only after a real root listing fetch/parse confirms durability.
- `src-tauri/src/tor_native.rs` now replenishes phantom depletion from live Arti client slots before it cold-bootstraps a new replacement.
- `src-tauri/src/adapters/qilin.rs` now protects root retries from standby remap and keeps the first ordinary child retry on the confirmed active host.

Validation completed after the root-durability follow-up:

- `cargo check --manifest-path 'crawli/src-tauri/Cargo.toml'`
- `cargo test --manifest-path 'crawli/src-tauri/Cargo.toml' qilin --lib`
- controlled exact-target reruns against `http://ijzn3sicrcy7guixkzjkib4ukbiilwc3xhnmby4mcbccnsd7j2rekvqd.onion/site/view?uuid=afa2a0ea-20ba-3ddf-8c5c-2aeea9e5dc43`
- a follow-up `--no-stealth-ramp` comparison run on the same target

Observed runtime outcomes:

- The repaired exact-target rerun now reaches a real QData root again, confirms a durable storage winner, parses `kent/` as `133 files / 133 folders`, and ends the timed `180.02s` window at `seen=288 processed=59 queue=229 workers=15/16` while adapter-local progress reaches `entries=670`.
- Persisted subtree state now records successful production work (`kent/2012`, `kent/2016`, `kent/2017`, `kent/AARP medical filing`, `kent/Chase Bank`, `kent/Credit Protection`) instead of mostly failed retry metadata.
- Live phantom replenishment is cheaper: the runtime now reuses live client slots before cold bootstrap, which removes one more mid-crawl stall mode from the hot path.
- The `--no-stealth-ramp` comparison also restored durable root parsing and reached `14/16` workers quickly, but it did not show a clear useful-work improvement over the repaired default path. Worker induction is not the primary remaining bottleneck.
- The remaining waste is now subtree-level standby churn after the winner host is already known, not root acquisition, fingerprint cost, or second-pool bootstrap.

## Implementation Update (2026-03-10, post-audit)

The Phase 85 recommendations in this audit have now been implemented in the current tree:

- Onion warmup in `src-tauri/src/lib.rs` now blocks only until a first-ready quorum exists.
- Onion-first swarm bootstraps now use traffic-class-aware entrypoints in `src-tauri/src/tor.rs` / `src-tauri/src/tor_native.rs`.
- Qilin node seeding in `src-tauri/src/adapters/qilin_nodes.rs` now merges into existing state instead of wiping learned latency/cooldown/failure history.
- Redirect hints and winner leases are now persisted and reused before broad discovery.
- Stage D and direct-mirror fallback now probe in tiny isolated waves instead of broad sequential/shared-client retries.

Validation completed after implementation:

- `cargo check --manifest-path 'crawli/src-tauri/Cargo.toml'`
- `cargo test --manifest-path 'crawli/src-tauri/Cargo.toml' qilin --lib`
- `BENCHMARK_ADAPTER=qilin BENCHMARK_DURATION=20 cargo run --manifest-path 'crawli/src-tauri/Cargo.toml' --bin adapter-benchmark`
- `BENCHMARK_ADAPTER=qilin BENCHMARK_DURATION=5 cargo run --manifest-path 'crawli/src-tauri/Cargo.toml' --bin adapter-benchmark`

Observed runtime outcomes:

- Cold Qilin benchmark run: Stage A captured and resolved a fresh rotated storage redirect on `6eoxnxd2y5xryvgyh22k3wknwrmw7w5i7l4yi2tu57w52v5ttjcif4ad.onion`.
- Warm-cache rerun: discovery hit the cached winner lease immediately before broad mirror seeding, proving the new fast path is active in real traffic.
- Remaining dominant cost: fingerprint/first-page acquisition still consumed roughly 13-20s on the March 10, 2026 live Qilin reruns.

## Residual Findings After Implementation

### [P2] Subtree route metrics are still weaker than the runtime behavior

Status update:
- This finding is now fixed in the shared runtime metrics plane and benchmark output.

Evidence:
- March 10, 2026 exact-target subtree-affinity replay

What remains:
- The route planner now avoids off-winner child churn in practice, but proving that still requires log-forensics instead of first-class metrics.
- There is not yet a direct counter for subtree reroutes, quarantine hits, or off-winner child attempts in the benchmark/CLI surfaces.

Recommended next fix:
- Expose subtree reroute count, subtree quarantine-hit rate, and off-winner child request count through runtime metrics and benchmark output.
- Keep the metrics keyed separately from global winner-health so audits can distinguish root instability from subtree-local degradation.

### [P2] Persistent subtree memory is unit-tested, but not yet clean-exit live-validated

Evidence:
- March 10, 2026 exact-target route-memory follow-up

What remains:
- The new subtree preferred-host persistence is implemented and unit-tested, but the exact-target validation runs in this audit were alarm-capped replays, so they do not yet prove a clean same-output rerun restored and then reused persisted subtree host summaries end-to-end.

Recommended next fix:
- Re-run the same target to normal completion on the same output root and confirm that the next run logs restored subtree host preferences and uses them without reviving stale UUID paths.

### [P3] Worker induction remains benchmark-only, not a default tuning lever

Evidence:
- March 10, 2026 pre-skip and final hinted-path reruns (`[Qilin Vanguard] Inducting worker ...`)

What remains:
- The default ramp is no longer the main limiter on the exact target once root durability and subtree routing are repaired.
- Faster induction may still help on unusually stable targets, but the evidence does not justify making it the default.

Recommended next fix:
- Keep `stealth_ramp=false` and smaller ramp intervals as benchmark-only knobs until a stable target proves a useful-work gain over the repaired default path.

### [P2] Fingerprint still dominates end-to-end onion startup

Status update:
- This finding is now fixed for strong Qilin CMS ingress URLs.
- It may still apply to unknown/weakly identified sites where a network fingerprint remains necessary.

Evidence:
- March 10, 2026 live Qilin benchmark reruns after the Phase 85 fixes

What remains:
- Even with warm-cache discovery, the canonical Qilin benchmark still spends roughly 13-20s in the fingerprint/initial GET phase before the adapter can exploit the faster storage-node routing path.

Recommended next fix:
- Cache strong adapter hints for known CMS ingress patterns and allow a bounded direct adapter fast path when the URL shape is already highly diagnostic.

### [P2] Arti-native readiness is still reported through empty `ports=[]` surfaces

Evidence:
- March 10, 2026 benchmark output after the Phase 85 implementation

What remains:
- The benchmark now uses the correct onion-service traffic class, but it still reports readiness as `ports=[]`, which is not a useful Arti-native operator signal.
- The continued reruns also showed that extra local SOCKS ports are not the lever for this app's hot path. The backend is already using direct in-process Arti clients; client-slot availability and redirect-host quality matter more than managed proxy port count.

Recommended next fix:
- Expose runtime label, active client count, traffic class, and health-monitor mode directly in benchmark/CLI output.

### [P2] Warm Qilin reruns still waste time on a single stale cached redirect before fresh redirect capture

Evidence:
- March 10, 2026 live Qilin rerun after the Phase 86 telemetry/lazy-seeding changes

What remains:
- The warm run now avoids broad seed churn, but it still spends one bounded probe on a single cached redirect hint before Stage A captures the freshest redirect.
- In the latest rerun, the cached hint missed and Stage A then revealed a completely new storage host (`tyxoxeljccxxxm55vlntefoftstbelml6txbqtclhahb63iz34peqiid.onion`).

Recommended next fix:
- Replace the single cached redirect hint with a tiny redirect ring keyed by recency and success ratio.
- Probe at most the freshest 2 cached redirect hosts before falling back to Stage A.
- Add per-stage latency counters so the benchmark can prove whether the ring is worth its extra request budget.

## Primary-Source Anchors

- Tor path-selection constraints: [Tor Project spec](https://spec.torproject.org/path-spec/path-selection-constraints.html)
- Tor relay/path weighting: [Tor Project spec](https://spec.torproject.org/path-spec/path-weighting.html)
- Arti configuration options: [Tor Project Arti docs](https://tpo.pages.torproject.net/core/arti/contributing/for-developers/config-options)
- `arti-client` stream preferences and isolation: [docs.rs StreamPrefs](https://docs.rs/arti-client/latest/arti_client/struct.StreamPrefs.html), [docs.rs IsolationToken](https://docs.rs/arti-client/latest/arti_client/struct.IsolationToken.html), [docs.rs TorClient](https://docs.rs/arti-client/latest/arti_client/struct.TorClient.html)
- Tail-latency reduction / hedging: [The Tail at Scale](https://research.google/pubs/the-tail-at-scale/)
- Retry/overload discipline: [Google SRE, Addressing Cascading Failures](https://sre.google/sre-book/addressing-cascading-failures/)

## Code Findings

### [P1] Prewarm wall time is still bounded by the slowest circuit

Evidence:
- `src-tauri/src/lib.rs:438-458`

What the code does now:
- For onion targets, the runtime launches one `HEAD` prewarm per client.
- It then waits on `join_all(pre_warms)`.
- Every task has a 45s timeout.

Why this is expensive:
- Wall time is `O(max client latency)` instead of "ready when enough fast circuits exist".
- One bad hidden-service route can hold fingerprinting for up to 45s even when several other clients are already usable.
- The repo's own lessons already say warmup should use first-ready/quorum behavior, not full-fanout completion waits.

Impact:
- Cold-start latency can be inflated by 10-45s.
- Those delays also burn power and bootstrap bandwidth on clients that do not help the critical path.

Recommended fix:
- Replace full `join_all` warmup with quorum warmup: stop the blocking phase after the first ready quorum and continue warming stragglers in the background.
- Suggested quorum: `min(max(2, client_count / 2), 4)`.
- Complexity improvement:
  - Current wall time: `O(max_i warmup_i)`
  - Proposed wall time: `O(kth_ready)` where `k` is the quorum, with the remaining `O(n-k)` work shifted off the critical path.

### [P1] Automatic healing is still driven by a clearnet health probe

Evidence:
- `src-tauri/src/tor_native.rs:29-30`
- `src-tauri/src/tor_native.rs:385-418`
- `src-tauri/src/tor_native.rs:847-910`

What the code does now:
- The default health probe target is `check.torproject.org:443`.
- `spawn_health_monitor()` probes every registered client against that target.
- Probe failures increment anomaly streaks and can trigger phantom replacement.

Why this is structurally wrong for Qilin/onion workloads:
- The active workload is onion-service listing and storage fetch, not clearnet exit traffic.
- A circuit can be perfectly fine for hidden services while failing or slowing on a clearnet probe path.
- This mixes traffic classes and can trigger false healing, unnecessary client replacement, and extra background bootstrap traffic.

Impact:
- False-positive circuit rotations.
- Phantom pool burn and replenishment churn.
- More bootstrap bandwidth and energy use without improving the actual target session.

Recommended fix:
- Make health classification traffic-class aware.
- Default the global probe target to `none` for onion-first sessions unless the operator explicitly opts in.
- Prefer passive health from real request telemetry:
  - rolling success ratio
  - rolling p50/p95 latency by target class
  - timeout/throttle streaks
- Only fall back to active probes when a client is idle or confidence is low.
- Complexity improvement:
  - Current: `O(c)` synthetic probes every interval for `c` clients
  - Proposed: `O(1)` update per real request, plus bounded `O(s)` active probes only for suspicious slots.

### [P1] Qilin mirror seeding wipes learned node quality on every run

Evidence:
- `src-tauri/src/adapters/qilin_nodes.rs:574-642`
- `src-tauri/src/adapters/qilin_nodes.rs:933-960`

What the code does now:
- `seed_known_mirrors()` constructs fresh `StorageNode` entries for every hardcoded and globally known host.
- Each seeded node is written with:
  - `last_seen = now`
  - `avg_latency_ms = 0`
  - `success_count = 0`
  - `failure_count = 0`
  - `failure_streak = 0`
  - `cooldown_until = 0`
- Stage D then ranks from this rewritten state.

Why this is a serious request-efficiency bug:
- Per-UUID node history is overwritten instead of merged.
- Cooldowns and failure streaks disappear.
- Previously bad hosts look fresh and healthy again on the next run.
- Tournament ranking becomes biased toward newly seeded records rather than actually reliable nodes.

Impact:
- Re-probing dead or recently failing nodes every session.
- More Stage D requests than necessary.
- Higher tail latency before a valid listing is found.

Recommended fix:
- Change seeding from destructive overwrite to merge-only:
  - create missing nodes
  - preserve existing `avg_latency_ms`, `success_count`, `failure_count`, `failure_streak`, and `cooldown_until`
  - only update the URL if the redirect gives a better remapped UUID path
- Keep freshness tied to successful contact, not to seed time.
- Complexity improvement:
  - Current: `O(g)` blind writes per run for `g` known/global hosts
  - Proposed: `O(m)` writes for only the truly new/missing hosts, where `m << g` in stable sessions.

### [P2] Direct-mirror fallback does not diversify probes across independent client identities

Evidence:
- `src-tauri/src/adapters/qilin.rs:1542-1677`

What the code does now:
- After discovery failure, direct mirrors are probed concurrently.
- Those probes clone the same `ArtiClient` handle captured from `frontier.get_client()`.

Why this is leaving latency on the table:
- Concurrent probes are issued, but they are not guaranteed to represent independent client identities or independently healed slots.
- This reduces the value of the fallback race, especially when the original client is already on a weak path.

Recommended fix:
- Probe fallback mirrors in waves of 2-3 using distinct frontier-selected clients or isolated client handles.
- Cancel losers immediately after the first listing-valid winner.
- This is the right place for bounded hedging because the target set is tiny and highly valuable.

## Architecture Enhancements Worth Building

### 1. Winner-Lease Node Selection

Current Stage D sorts candidates each run and then probes one by one. Keep that, but add a winner lease:

- Maintain `winner_host`, `winner_url`, `lease_until`, `p50_latency_ms`, `success_ratio`, and `last_remapped_uuid`.
- If the winner is still within lease and not cooling down, try it first without rerunning the full tournament.
- Reopen the tournament only on timeout, throttle burst, or lease expiry.

Expected value:
- Saves 1-4 node probes on steady-state sessions.
- Very good ROI when repeated runs hit the same victim or same rotated storage fleet.

Complexity:
- Update per request: `O(1)`
- Tournament reopen: `O(k log N)` with fixed top-N heap, where `k` is cached node count and `N` is the small probe set.

### 2. Fresh Redirect Cache

The code already captures Stage A redirects. Persist a short-lived cache:

- Key: `cms_uuid`
- Value: `redirect_host`, `redirect_url`, `captured_at`, `success/failure delta since capture`
- TTL: 5-20 minutes

Use it before Stage B and before broad fallback.

Expected value:
- If a storage node rotates but remains valid for a short window, follow-up runs can skip the full rediscovery path.

### 3. Bounded Hedged Probing for Discovery

Do not blast 18+ nodes; the repo's lessons are right about that. But Stage D should not stay fully sequential either.

Recommended policy:
- Wave 1: redirect winner + last known good
- Wave 2: next top 1-2 ranked nodes only if Wave 1 fails
- Cancel all stragglers as soon as the first valid listing is confirmed

Expected value:
- Lower tail latency without causing the probe stampede that earlier experiments already disproved.

### 4. Replace Size-Only HEADs With Request-Coalesced Metadata

The backend docs already identify HEAD inflation as a request multiplier. For onion paths, the default should be:

- Use HTML-extracted sizes when available.
- Else use `GET Range: bytes=0-0` or downloader-time metadata fetches instead of standalone HEADs.

Expected value:
- Reduces request count materially on deep listings.
- Also lowers energy and exposure to server-side anti-bot heuristics.

### 5. Passive Healing Before Phantom Rotation

Add a two-stage healing model:

- Stage 1: passive demotion
  - lower scheduling weight
  - stop assigning new speculative work
  - keep existing in-flight requests alive
- Stage 2: active replacement
  - only if passive demotion keeps failing for a bounded interval

Expected value:
- Fewer unnecessary phantom swaps.
- Better energy efficiency and less bootstrap churn.

### 6. Request-Efficiency Telemetry for the Chart Swarm

The dashboard exposes worker/circuit/resource state, but it still lacks the efficiency metrics needed to tune the system scientifically.

Add these time-series:

- requests per discovered entry
- Stage A / B / C / D latency
- node probe attempts per successful winner
- phantom swaps by reason
- p50/p95 request latency by target phase
- winner-host lease hit rate
- bytes downloaded per successful request

These should become first-class chart surfaces, not only log lines. Without them, the UI can show that the swarm is busy but not whether it is efficient.

## Priority Order

1. Fix destructive Qilin node seeding.
2. Move warmup to first-ready quorum.
3. Decouple onion healing from the default clearnet probe.
4. Add fresh redirect cache + winner lease.
5. Add bounded 2-3 wave hedging for Stage D and direct-mirror fallback.
6. Add request-efficiency telemetry to the dashboard.

## Proposed Validation Plan

1. Synthetic:
- Extend `src-tauri/examples/qilin_benchmark.rs` to emit `requests`, `requests_per_entry`, Stage D winner depth, and warmup-to-first-fingerprint latency.

2. Live:
- Run the canonical Qilin target with:
  - baseline
  - merged node-seed preservation
  - quorum warmup
  - traffic-class-aware healing
- Compare:
  - time to first valid listing
  - total discovery requests
  - phantom swaps
  - node failovers
  - entries per minute

3. GUI:
- Add charts for:
  - request efficiency
  - node winner stability
  - healing causes
  - Stage A/B/C/D timing

## Bottom Line

The fastest path forward is not a bigger swarm. It is a smarter swarm:

- block on fewer warmup clients
- stop healing from the wrong signal
- keep learned node state instead of rewriting it
- spend duplicate requests only where they cut tail latency materially
- expose efficiency metrics directly in the dashboard so the control loops can be tuned from evidence

Those changes are all local to the current architecture. They do not require replacing Arti or rewriting the crawler, and they are realistic places to win 100ms-to-several-seconds at a time.
