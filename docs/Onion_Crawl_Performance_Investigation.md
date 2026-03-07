> **Last Updated:** 2026-03-06T16:40 CST

Version: 1.0.0
Updated: 2026-03-06
Authors: Navi (User), Codex (GPT-5)
Related Rules: [MANDATORY-L1] Docs Management, [MANDATORY-L1] Living Documents, [MANDATORY-L1] Performance/Cost/Quality, [MANDATORY-L1] Testing & Validation

# Summary
This whitepaper investigates how to speed up onion crawling in `crawli` after the native Arti migration. The short answer is:

- There is still room for meaningful speedup.
- The next gains do **not** come from blindly increasing worker counts, circuits, or "more IPs".
- The real levers are target-aware concurrency, prewarmed circuit supply, better node selection, and splitting metadata crawling from bulk file transfer.

## Implemented Status (2026-03-06)
Completed from this recommendation set:
- Target-aware page concurrency was implemented in `qilin.rs`
- Node tournament scoring, cooldown, and sticky-winner revalidation were implemented in `qilin_nodes.rs`
- Metadata crawling now reserves headroom when the same session is expected to download, instead of blindly consuming the full swarm
- Native Arti preemptive/request timing is now tuned explicitly in `tor_native.rs`
- Non-Qilin adapters now inherit frontier-owned listing-worker caps instead of hardcoded `120` worker pools
- Qilin Stage D now probes the tournament head first and only opens fallback candidates if needed
- `aria_downloader.rs` is now the explicit production download engine; `multipath.rs` remains laboratory-only until it reaches resume/control parity
- Large-file downloader tournament width is now capped and fed by live Tor tournament telemetry instead of a fixed `2x` race width
- The downloader's BBR controller now gates live piece fetchers through an active window instead of collecting metrics without affecting work issuance

Still recommended but not implemented in this pass:
- A fuller two-swarm runtime split for crawl traffic vs download traffic
- Richer per-target historical telemetry for storage-node selection beyond cooldown/reliability/latency

## Synthetic Benchmark Snapshot (2026-03-06)
Method:
- Replaced the old live-onion `qilin_benchmark.rs` with a synthetic local QData benchmark harness.
- The harness serves deterministic QData-like HTML from a local mock server and runs the real `QilinAdapter` plus `CrawlerFrontier` against it.
- Two profiles were measured:
  - `clean`: low latency, no throttling
  - `hostile`: deterministic delay + first-hit `429` injection

Tree shape:
- depth `4`
- `4` directories per level
- `12` files per directory
- expected entries: `4432`

Observed results:
- `clean / 12 circuits`: `4432` entries in `0.33s` (`13274.8 entries/s`)
- `clean / 24 circuits`: `4432` entries in `0.42s` (`10557.7 entries/s`)
- `clean / 36 circuits`: `4432` entries in `0.49s` (`8996.1 entries/s`)
- `hostile / 12 circuits`: `4432` entries in `9.29s` (`477.2 entries/s`)
- `hostile / 24 circuits`: `4432` entries in `10.09s` (`439.2 entries/s`)
- `hostile / 36 circuits`: `4432` entries in `10.22s` (`433.9 entries/s`)

Interpretation:
- Even on a safe local benchmark, pushing metadata concurrency higher than `12` reduced throughput.
- That means parser/scheduling overhead alone is enough to make "more workers" lose efficiency.
- Real onion-service conditions will penalize over-concurrency more severely because descriptor, introduction, rendezvous, and target-side throttling get added on top.

Artifacts:
- Benchmark harness: [src-tauri/examples/qilin_benchmark.rs](/Users/navi/Documents/Projects/LOKI%20TOOLS/Onion%20Forger/crawli/src-tauri/examples/qilin_benchmark.rs)
- Latest result bundle: [tmp/qilin_benchmark_latest.json](/Users/navi/Documents/Projects/LOKI%20TOOLS/Onion%20Forger/crawli/tmp/qilin_benchmark_latest.json)

Important scope note:
- No `KillNet` adapter exists in the current repository. The nearest matching workload is the Qilin-style adapter and the shared onion crawling stack.
- Recommendations below therefore apply to the current onion architecture generally, and to Qilin-like hidden-service storage crawls specifically.

# Current State In Repo
Relevant runtime surfaces already present:
- Native Arti client swarm in [tor_native.rs](/Users/navi/Documents/Projects/LOKI%20TOOLS/Onion%20Forger/crawli/src-tauri/src/tor_native.rs)
- Shared crawl frontier and client pool in [frontier.rs](/Users/navi/Documents/Projects/LOKI%20TOOLS/Onion%20Forger/crawli/src-tauri/src/frontier.rs)
- Qilin adapter worker/retry model in [qilin.rs](/Users/navi/Documents/Projects/LOKI%20TOOLS/Onion%20Forger/crawli/src-tauri/src/adapters/qilin.rs)
- Qilin node discovery cache in [qilin_nodes.rs](/Users/navi/Documents/Projects/LOKI%20TOOLS/Onion%20Forger/crawli/src-tauri/src/adapters/qilin_nodes.rs)
- Production range/tournament downloader in [aria_downloader.rs](/Users/navi/Documents/Projects/LOKI%20TOOLS/Onion%20Forger/crawli/src-tauri/src/aria_downloader.rs)
- Experimental download laboratory in [multipath.rs](/Users/navi/Documents/Projects/LOKI%20TOOLS/Onion%20Forger/crawli/src-tauri/src/multipath.rs)

Observed constraints in the current implementation:
- `qilin.rs` no longer relies on a fixed page-worker ceiling; it uses a local adaptive governor capped below the raw client pool.
- Non-Qilin adapters now derive metadata worker count from `frontier.recommended_listing_workers()`, but crawl/download traffic still share the same underlying swarm.
- `aria_downloader.rs` already owns resume state, stop/pause semantics, batch telemetry, managed-port reuse, and range-request orchestration. Any new production download strategy must preserve those semantics.
- `multipath.rs` is useful as a benchmarking sandbox, but auto-promoting production downloads into it today would throw away control-plane behavior that the shipped app depends on.
- `qilin_nodes.rs` now stages probing correctly, but the node tournament still uses simple probe success/latency snapshots rather than richer per-target historical telemetry.

# Research Anchors
- [Tor Project: How do Onion Services work?](https://community.torproject.org/en/onion-services/overview/)
- [Tor protocol proposal 224: next-generation rendezvous flow](https://spec.torproject.org/proposals/224-rend-spec-ng.html)
- [Google SRE: Addressing Cascading Failures](https://sre.google/sre-book/addressing-cascading-failures/)
- [Google Research: BBR Congestion-Based Congestion Control](https://research.google/pubs/pub45814)

# Protocol Facts That Matter
Official Tor documentation makes several hard boundaries clear:

1. Onion services do not expose the server IP to users, and the server does not learn the client IP in the normal clearnet sense.
2. A client reaches an onion service by learning the descriptor, selecting a rendezvous point, and then connecting through introduction/rendezvous machinery.
3. A single onion connection already traverses more Tor machinery than a plain exit-based web request, so adding concurrency is expensive.

Primary implications:
- "More IP addresses" is not the same performance lever on onion services that it is on clearnet scraping.
- More circuits help only until hidden-service descriptor fetch, intro-point responsiveness, rendezvous success, and server-side rate limits become the bottleneck.
- Once those bottlenecks dominate, adding workers often increases failure rate faster than throughput.

# Feasibility Matrix
## A. High-confidence, implement-now improvements

### 1. Target-aware concurrency governor
Best immediate win.

Instead of a fixed Qilin worker ceiling, maintain a per-target control loop that reacts to:
- hidden-service circuit failures
- connect timeouts
- HTTP 429 / 503
- successful pages per minute
- queue drain rate

Recommended behavior:
- Start lower for directory discovery, around 8-16 active page workers per target.
- Increase only when success ratio and queue drain both improve.
- Decrease quickly when hidden-service circuit failures or 429s rise.
- Keep file download concurrency on a separate controller from HTML crawl concurrency.

Why this is better than "crazier workers":
- Directory traversal is latency-bound and intro/rendezvous-bound.
- File transfer is range-download and throughput-bound.
- One controller cannot optimize both without oscillation.

### 2. Split the swarm into two classes
Current architecture mixes concerns too much.

Recommended split:
- `crawl swarm`: low-to-medium concurrency, short HTML requests, node discovery, path enumeration
- `download swarm`: high-throughput circuits reserved for large file transfer only

This prevents directory enumeration from being starved by large downloads, and prevents large downloads from poisoning the same circuits used for link discovery.

### 3. Promote `qilin_nodes.rs` into a real node tournament
Current node discovery is good but not yet optimal.

Add:
- temporary demotion windows for nodes with repeated timeout/429 behavior
- per-node success ratio
- rolling median RTT instead of only average latency
- "stickiness" to the current winning node until it degrades materially
- cooldown before re-probing clearly bad nodes

This is the highest-value adapter-specific improvement because Qilin-like targets often have multiple storage hosts with materially different behavior.

### 4. Enable Arti circuit prewarming deliberately
Your current stack already uses `arti-client 0.39.0`, and the Arti/Tor config surfaces support:
- preemptive circuit construction
- request retry budgets
- hidden-service attempt counters

Recommendation:
- turn on deliberate preemptive circuit pools for the ports you actually use
- avoid building too many at once
- tune hidden-service attempt counts conservatively to reduce long tail stalls

Goal:
- shave circuit acquisition latency without detonating the network with useless speculative paths

### 5. Add per-target failure buckets
Treat failures differently:
- descriptor fetch failures
- intro/rendezvous failures
- HTML timeout with otherwise healthy node
- HTTP rate limit / overload response
- parser-empty but request-success cases

Each class should trigger different recovery:
- rotate slot
- switch storage node
- reduce concurrency
- short cooldown
- give partial results

Right now too many different failure classes converge on generic retry pressure.

## B. Medium-confidence improvements worth prototyping

### 6. Descriptor and node-path warmup phase
Before launching a full crawl, spend a short warmup window:
- probe candidate storage nodes
- build a small live set of "known-good-enough" circuits
- establish an initial winning node ranking

This is especially useful for Qilin-like adapters where the first 30-90 seconds often determine whether the session stabilizes or collapses into retries.

### 7. Adaptive page batching / frontier ordering
Directory crawls are not all equal.

Improve frontier ordering:
- prioritize shallow high-fanout pages first
- deprioritize long-tail retry nodes until the main queue is drained
- favor same-host/same-subtree locality when a node is currently hot and responsive

This reduces cold-start thrash and improves early path discovery density.

### 8. Partial-response tolerant reconciliation
Phase 44 is already bounded now, but it can still be smarter.

Improve reconciliation with:
- confidence bands on completeness
- "good enough" finish thresholds
- exact reporting of which subtrees are incomplete

That shortens time-to-useful-results on unstable targets.

### 9. Range-download promotion path
When a directory crawl finds large files:
- do not fetch them over the metadata swarm
- keep them on the canonical `aria_downloader.rs` path unless an alternate engine proves parity on resume/control/telemetry semantics
- continue migrating proven scheduling ideas from `multipath.rs` into `aria_downloader.rs` instead of bypassing the production control plane

This should remain file-size-gated and target-health-gated.

## C. Low-value or misleading ideas

### 10. Blindly increasing worker counts to 120+
Not recommended as a universal strategy.

Reason:
- On onion services, more parallel requests amplify descriptor, intro, rendezvous, and server overload pressure.
- In your current logs, higher failure pressure already manifests as repeated hidden-service circuit failures and endless reconciliation churn.

### 11. "Multiple IP addresses"
Misleading in onion-service context.

Reason:
- The server side is interacting with onion rendezvous behavior, not a simple pool of visible client IPs.
- More local egress addresses do not provide the same scaling effect as in clearnet scraping.

### 12. Kernel-bypass, raw sockets, or custom packet tricks
Not appropriate here.

Reason:
- The protocol bottleneck is Tor/onion-service path construction and target behavior, not a raw TCP kernel hot path.
- These ideas add massive complexity without changing the hidden-service architecture that dominates latency.

### 13. Infinite NEWNYM-style churn
Actively harmful past a threshold.

Reason:
- Rotating too often destroys path reuse and increases bootstrap/circuit build overhead.
- Rotation should be classifier-driven, not reflexive.

# What We Can Invent Safely
The highest-value new functionality to add is not lower-level networking. It is smarter control theory around the traffic you already generate.

## Invention 1: Dual-loop controller
Two coupled but separate loops:
- Loop A: metadata crawl concurrency
- Loop B: large-file download concurrency

Inputs:
- success ratio
- hidden-service circuit failure ratio
- mean/median RTT
- queue drain slope
- bytes/sec for download tasks

Outputs:
- active workers
- preferred node count
- retry unlock window
- whether to rotate or hold circuits

This is the right place to apply "advanced math" in a way that actually helps.

## Invention 2: Node tournament with confidence intervals
Treat each storage node as an arm in a bandit/tournament:
- success probability
- median RTT
- rate-limit penalty score
- freshness / recent stability

Prefer nodes with the best joint score, not just lowest average latency.

## Invention 3: Completion-aware crawl mode
Add two explicit modes:
- `fast-inventory`: maximize early coverage, permit partial completion
- `full-reconciliation`: slower tail-sweep mode focused on completeness

This would reduce operator confusion and prevent wasting time on long-tail retries when the user only needs the bulk of the tree quickly.

# Recommended Implementation Order
## P0
1. Add target-aware concurrency control to `qilin.rs`
2. Split crawl swarm vs download swarm
3. Upgrade `qilin_nodes.rs` to a proper node tournament with demotion/cooldown

## P1
4. Tune Arti preemptive circuits and hidden-service timing in `tor_native.rs`
5. Add failure-bucket classification and differentiated recovery
6. Keep `multipath.rs` experimental unless it reaches feature parity with `aria_downloader.rs`

## P2
7. Add warmup phase + winning-node lock-in
8. Add completion-aware crawl modes
9. Add operator telemetry for node ranking, failure buckets, and tail-stall detection

# Professional Recommendation
If the goal is to materially speed up the current onion crawl stack, the best next implementation is:

1. Stop treating throughput as a single "more workers" knob.
2. Build a target-aware concurrency governor around the adapter.
3. Separate discovery traffic from bulk-transfer traffic.
4. Use Arti's preemptive circuit machinery carefully to reduce wait time, not to brute-force the network.

Based on the synthetic benchmark now in-repo, the safe default for metadata discovery should stay in the low-teens, not the high-twenties or thirties, unless target-specific evidence proves otherwise.

That path is much more likely to improve real throughput than spinning more circuits blindly.

## Phase 20A: Implemented Follow-Through (2026-03-06)
Completed from this investigation in the current codebase:
- Qilin metadata discovery now treats the operator-selected circuit count as capacity, not as the live worker target.
- Backend resource telemetry now makes CPU/RAM pressure observable during real crawl/download work.
- Qilin storage-node routing now keeps a small standby list and performs bounded failover on classified degradation rather than relying on a single seed forever.
- The native Qilin crawl path no longer keeps a second full crawl-result vector in memory while also streaming VFS entries to the UI/database.
- An operator-run authorized soak harness now exists for `listing-plus-one-large-file` sessions so long-run behavior can be profiled without inventing a second execution path.
- Repeat crawls now persist a deterministic per-target best snapshot and perform bounded catch-up retries when raw runs underperform that best snapshot under unstable conditions.
- Download resume planning now prefers the failed-file set before the general missing/mismatch set from the authoritative best crawl snapshot.

# Prevention Rules
1. Do not equate onion-service crawling with clearnet scraping. More workers are not linearly additive.
2. Do not use "multiple IP addresses" as a primary performance plan for onion services.
3. Do not merge HTML traversal traffic and bulk file transfer traffic into the same concurrency controller.
4. Do not rotate circuits on every failure; classify the failure first.
5. Do not ship universal worker-count guidance without adapter-specific benchmark evidence.
6. Do not expose a `circuits` selector as though it were the live Qilin HTML worker count; keep it as capacity and let the governor choose the active window.
7. Do not ship new speed claims without CPU/RAM telemetry, because apparent slowdown may be parser/memory pressure rather than network scarcity.

# Sources
- Tor Project, Onion Services overview: https://community.torproject.org/onion-services/overview/
- Tor Browser Manual, Onion services hide server location/IP: https://tb-manual.torproject.org/onion-services/
- Tor Project, Tor network path overview: https://community.torproject.org/relay/types-of-relays/
- Tor Project, Onion service ecosystem and Conflux/Arti status: https://onionservices.torproject.org/apps/base/onionbalance/tutorial/
- Arti/Tor configuration surfaces verified locally in:
  - [Cargo.toml](/Users/navi/Documents/Projects/LOKI%20TOOLS/Onion%20Forger/crawli/src-tauri/Cargo.toml)
  - [tor_native.rs](/Users/navi/Documents/Projects/LOKI%20TOOLS/Onion%20Forger/crawli/src-tauri/src/tor_native.rs)
  - Local cargo registry source for `arti-client` and `tor-circmgr`
