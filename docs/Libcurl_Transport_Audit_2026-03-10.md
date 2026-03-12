# Libcurl Transport Audit - 2026-03-10

## Purpose

This audit reverse-engineers the parts of libcurl that matter most for Crawli's crawl and download paths, then maps them onto the current Rust/Tauri implementation.

Primary sources reviewed:

- [libcurl API overview](https://curl.se/libcurl/c/libcurl.html)
- [libcurl multi interface overview](https://curl.se/libcurl/c/libcurl-multi.html)
- [curl_multi_socket_action](https://curl.se/libcurl/c/curl_multi_socket_action.html)
- [CURLSHOPT_SHARE](https://curl.se/libcurl/c/CURLSHOPT_SHARE.html)
- [CURLMOPT_MAX_HOST_CONNECTIONS](https://curl.se/libcurl/c/CURLMOPT_MAX_HOST_CONNECTIONS.html)
- [CURLMOPT_MAX_TOTAL_CONNECTIONS](https://curl.se/libcurl/c/CURLMOPT_MAX_TOTAL_CONNECTIONS.html)
- [CURLOPT_MAXAGE_CONN](https://curl.se/libcurl/c/CURLOPT_MAXAGE_CONN.html)
- [CURLOPT_LOW_SPEED_LIMIT](https://curl.se/libcurl/c/CURLOPT_LOW_SPEED_LIMIT.html)
- [CURLOPT_LOW_SPEED_TIME](https://curl.se/libcurl/c/CURLOPT_LOW_SPEED_TIME.html)
- [CURLMOPT_PIPELINING / multiplex](https://curl.se/libcurl/c/CURLMOPT_PIPELINING.html)
- [curl source: lib/conncache.c](https://github.com/curl/curl/blob/master/lib/conncache.c)
- [curl source: lib/multi.c](https://github.com/curl/curl/blob/master/lib/multi.c)
- [aria2 manual](https://aria2.github.io/manual/en/html/aria2c.html)
- [wget2 manual](https://gitlab.com/gnuwget/wget2/-/raw/master/docs/wget2.md)
- [reqwest ClientBuilder docs](https://docs.rs/reqwest/latest/reqwest/struct.ClientBuilder.html)

## What libcurl actually does well

### 1. One transfer state, one scheduler, one shared cache

libcurl splits concerns cleanly:

- easy handle: per-transfer state
- multi handle: non-blocking scheduler for many transfers
- share handle: optional shared DNS, SSL session, cookie, and connection cache

The important practical point is not the API surface. The important point is that libcurl avoids relearning the same host properties over and over. When multiple easy handles run inside the same multi handle, they already share the connection cache by default.

This keeps lookup cost effectively O(1) average for host capability reuse and avoids a large amount of repeated network setup.

### 2. Connection reuse is treated as a first-class optimization

libcurl aggressively reuses viable connections, but also fences them:

- it keeps a connection cache
- it can cap total open connections
- it can cap simultaneous connections per host
- it can refuse stale idle connections with max-age policies
- it can multiplex new transfers onto existing HTTP/2 connections when allowed

That combination matters more than "more sockets". It lowers handshake cost, reduces request start latency, and prevents one hot host from being overdriven.

### 3. It prefers event-driven progress over blind polling

The multi socket API is built around readiness notifications, not repeated polling. That is why libcurl scales well with many simultaneous transfers.

Rust async already gives Crawli a similar event-driven runtime, so the gap is not "we need a select loop". The gap is that our host memory and transfer admission logic are still much more stateless than libcurl's.

### 4. It distinguishes "slow" from "dead"

libcurl documents hard timeouts as blunt instruments and recommends low-speed policies for dynamic workloads. The low-speed limit/time pair is specifically designed to kill transfers that are making too little progress for too long, without forcing a fixed wall-clock timeout on every transfer.

This is critical for onion workloads, where "slow but productive" and "stalled forever" are not the same thing.

## Comparable downloader stacks

### aria2

aria2 is the closest mainstream downloader analogue to Crawli's batch direct-download path:

- segmented range fetching
- `max-connection-per-server` active host pressure control
- `lowest-speed-limit` low-speed abort semantics
- piece selectors that bias either head-first preview or throughput

The important lesson is not that aria2 is written in C++. The important lesson is that it exposes first-class knobs for admission, host pressure, and piece scheduling on top of standard HTTP range support.

### Wget2

Wget2 is stronger on robust recursive fetch, retries, mirror semantics, and long-running download correctness than on raw "fastest possible" transfer speed. It is useful as a comparison point for recursion and persistence, but less directly relevant than libcurl or aria2 for Crawli's hot direct-download path.

### Internet Download Manager (IDM)

IDM is proprietary, so the useful comparison comes from official documentation and observed behavior rather than source review. The strongest patterns it advertises are dynamic segmentation, aggressive site-aware connection usage, and a fallback mode that starts downloading immediately when a site will not tolerate the extra request needed for the normal dialog flow.

For Crawli, the important lesson is IDM's exception model: keep the fast multi-request path for healthy hosts, but very quickly downgrade weak hosts into special-case behavior instead of forcing every host through the same aggressive strategy. That maps directly onto Qilin probe-stage degraded-host quarantine and alternate-host rotation.

### reqwest / hyper

reqwest and hyper are not inherently slower because they are Rust. The real gap is that their default surface is more general-purpose and exposes fewer downloader-specific policy knobs out of the box:

- idle pool sizing is easy
- event-driven scheduling is already strong
- downloader-style host admission, low-speed policy, and piece heuristics must be built by the application

That means Rust is still a viable performance foundation here. The missing wins were mostly policy and transport memory, not language choice.

## Language and implementation conclusion

The C and C++ stacks studied here outperform many naïve Rust downloaders for three concrete reasons:

- mature connection cache behavior
- explicit active per-host limits
- low-speed / progress-sensitive abort policies

They do not win simply because the language is C or C++. For Crawli, the evidence points to keeping the Rust transport stack and copying the missing scheduler and host-memory ideas instead of replacing the entire downloader core.

## Current Crawli alignment

### Already aligned

- Tor transport already uses pooled Hyper clients with HTTP/2 enabled in [arti_client.rs](/Users/navi/Documents/Projects/LOKI%20TOOLS/Onion%20Forger/crawli/src-tauri/src/arti_client.rs#L36).
- Clearnet transport already uses a pooled reqwest client in [arti_client.rs](/Users/navi/Documents/Projects/LOKI%20TOOLS/Onion%20Forger/crawli/src-tauri/src/arti_client.rs#L57).
- Downloader probing already uses `GET Range: bytes=0-0` instead of a separate `HEAD`, which is closer to libcurl's "learn from the real transfer path" model in [aria_downloader.rs](/Users/navi/Documents/Projects/LOKI%20TOOLS/Onion%20Forger/crawli/src-tauri/src/aria_downloader.rs#L1126).
- The direct benchmark now consumes production probe metadata instead of a parallel benchmark-only request path.

### Not aligned enough

#### P1. Probe-to-transfer fusion is still missing

`probe_target_with_timeout()` performs a real ranged GET, extracts `content_length` and validators, and then drops the response body. The later transfer starts a new request anyway.

Relevant code:

- [aria_downloader.rs](/Users/navi/Documents/Projects/LOKI%20TOOLS/Onion%20Forger/crawli/src-tauri/src/aria_downloader.rs#L1126)
- [aria_downloader.rs](/Users/navi/Documents/Projects/LOKI%20TOOLS/Onion%20Forger/crawli/src-tauri/src/aria_downloader.rs#L2741)

For micro and small files, that means Crawli often pays two request setups where libcurl-style transfer promotion would pay one.

Impact:

- extra request count
- extra RTT / onion circuit tax
- extra admission failure opportunities before bytes flow

#### P1. Batch lanes actively disable reuse

The small-file swarm and range tournament probe explicitly send `Connection: close`.

Relevant code:

- [aria_downloader.rs](/Users/navi/Documents/Projects/LOKI%20TOOLS/Onion%20Forger/crawli/src-tauri/src/aria_downloader.rs#L1591)
- [aria_downloader.rs](/Users/navi/Documents/Projects/LOKI%20TOOLS/Onion%20Forger/crawli/src-tauri/src/aria_downloader.rs#L3711)

That is the opposite of libcurl's reuse model. It can be defensible for a hostile onion edge only if the host is already proven to punish reuse. Right now it is a blanket behavior inside critical hot paths.

Impact:

- destroys keep-alive gains
- forces extra connect / TLS / circuit setup
- wastes productive winners after they are already known

#### P1. No generic host capability cache exists

Crawli learns range support, validators, probe success, and effective host quality per transfer attempt, but there is no generic downloader-side host capability table analogous to libcurl's shared connection/DNS/session memory.

The current state is mostly per-entry or per-run, not per-host.

Relevant code:

- [aria_downloader.rs](/Users/navi/Documents/Projects/LOKI%20TOOLS/Onion%20Forger/crawli/src-tauri/src/aria_downloader.rs#L1214)
- [lib.rs](/Users/navi/Documents/Projects/LOKI%20TOOLS/Onion%20Forger/crawli/src-tauri/src/lib.rs#L1933)

Existing Qilin repin memory helps, but it is not a general transport capability cache.

Needed per-host memory:

- `supports_ranges`
- `resume_validator_kind`
- median connect latency
- median first-byte latency
- low-speed abort history
- safe parallelism cap
- degraded/quarantine state
- last productive timestamp

Average complexity target: O(1) hash lookup and O(log h) ranking if host selection uses a heap over `h` candidate hosts.

#### P1. Active per-host limits were missing before Phase 100

`pool_max_idle_per_host(32)` only limits idle pooled connections. It does not act like libcurl's `CURLMOPT_MAX_HOST_CONNECTIONS`, which caps simultaneous open connections to one host and queues excess work.

Relevant code:

- [arti_client.rs](/Users/navi/Documents/Projects/LOKI%20TOOLS/Onion%20Forger/crawli/src-tauri/src/arti_client.rs#L44)
- [arti_client.rs](/Users/navi/Documents/Projects/LOKI%20TOOLS/Onion%20Forger/crawli/src-tauri/src/arti_client.rs#L57)

This was the historical gap before the March 10, 2026 evening transport update. The downloader now enforces a live host permit cap inside `aria_downloader.rs`, but the section remains here because it explains why that control became mandatory.

Impact:

- hot-host overcommit
- avoidable throttles
- worse tail latency

#### P2. Hard timeouts dominate where low-speed policy should dominate

The downloader still leans heavily on absolute send / first-byte / body timeouts.

Relevant code:

- [aria_downloader.rs](/Users/navi/Documents/Projects/LOKI%20TOOLS/Onion%20Forger/crawli/src-tauri/src/aria_downloader.rs#L1520)
- [aria_downloader.rs](/Users/navi/Documents/Projects/LOKI%20TOOLS/Onion%20Forger/crawli/src-tauri/src/aria_downloader.rs#L1718)

This is exactly where libcurl recommends low-speed rules instead of fixed total timeouts. A transfer that is slow but progressing should not be classified the same as one that is effectively dead.

#### P2. Re-isolation currently fights transport memory

`new_isolated()` is used aggressively after repeated send/connect/body failures.

Relevant code:

- [arti_client.rs](/Users/navi/Documents/Projects/LOKI%20TOOLS/Onion%20Forger/crawli/src-tauri/src/arti_client.rs#L68)
- [aria_downloader.rs](/Users/navi/Documents/Projects/LOKI%20TOOLS/Onion%20Forger/crawli/src-tauri/src/aria_downloader.rs#L1613)
- [aria_downloader.rs](/Users/navi/Documents/Projects/LOKI%20TOOLS/Onion%20Forger/crawli/src-tauri/src/aria_downloader.rs#L1741)

For onion traffic this is sometimes necessary. For clearnet, and for already-proven productive hosts, it throws away exactly the reused transport state libcurl is designed to preserve.

## Highest-value implementations to steal from libcurl

### 1. Probe-to-transfer promotion

Implement a transport state object that can promote a successful range probe directly into the first transfer segment.

Best initial target:

- micro files
- small files
- clearnet direct files

Why first:

- it directly removes requests
- it reduces Qilin failure surface before bytes flow
- it is measurable immediately as `requests/file`

Expected effect:

- up to one request saved per file in the happy path
- strongest gains on many-small-file targets

### 2. Shared host capability cache

Add a downloader-side host ledger keyed by `scheme + host + port + adapter_class`.

Persist only transport facts, not payload assumptions.

Suggested fields:

- `supports_ranges: bool`
- `validator_kind: none|etag|last_modified`
- `connect_rtt_ms_p50`
- `first_byte_ms_p50`
- `low_speed_abort_count`
- `recent_success_count`
- `recent_fail_count`
- `safe_parallelism_cap`
- `quarantine_until`
- `reuse_allowed`

This is the closest Crawli analogue to libcurl's shared cache behavior.

### 3. Low-speed abort instead of only wall-clock abort

Add downloader-side policy equivalent to:

- low speed limit
- low speed window
- host-specific thresholds

Direct examples:

- clearnet large file: abort if under X KiB/s for Y seconds after first byte
- onion Qilin small file: abort if zero or near-zero progress over a shorter rolling window, not only after an absolute wall timeout

This should replace a meaningful fraction of the current blunt `timeout(...)` usage, not all of it.

### 4. Conditional keep-alive, not blanket `Connection: close`

Switch from unconditional `Connection: close` to host-quality-aware reuse:

- clearnet: default keep-alive
- Qilin/onion unknown host: conservative
- Qilin/onion proven productive winner: allow bounded reuse inside the same host-quality window
- throttled/degraded host: forbid reuse temporarily

This is the most direct "copy libcurl's instincts" change currently visible in the code.

### 5. Generic active per-host cap

Add a true downloader-side active-host limiter, separate from idle pool size.

This should queue excess work instead of starting all host-local transfers immediately.

Target behavior should look like libcurl:

- total global limit
- per-host active limit
- queue pending work until a slot is free

### 6. Reuse security partitions, not reuse globally

Do not copy libcurl's reuse model naively onto onion isolation.

Correct adaptation:

- reuse within the same isolation/security partition
- do not share state across partitions that should remain isolated
- let the host capability cache store only non-sensitive transport facts

## Transport recommendations by traffic class

### Clearnet

Most libcurl patterns transfer directly:

- keep-alive on
- pooled client reuse
- active per-host caps
- probe-to-transfer promotion
- low-speed abort
- host capability cache

This is where Crawli should look most like libcurl.

### Onion / Qilin

The same concepts still apply, but must be security-aware:

- reuse only within a controlled isolation boundary
- quarantine degraded hosts earlier
- promote probe to transfer only when the probe is productive
- avoid blanket connection closing after the winner is proven
- let host memory drive safer admission before widening concurrency

## Implementation status update (March 10, 2026 evening)

The first four libcurl-style improvements are now implemented in production code:

- conditional clearnet/onion keep-alive policy
- probe-to-transfer promotion for bounded micro/small paths
- shared downloader host-capability cache
- low-speed abort telemetry

The active per-host cap is now also implemented in `aria_downloader.rs` as a live host permit ledger, with traffic-class-aware defaults (`32` clearnet, `4` onion) and queueing instead of immediate overcommit.

Live validation from the same evening:

- direct benchmark: `host_cap=32`, `bytes=758923264`, `elapsed_secs=15.39`, `throughput_mbps=394.59`
- exact-target Qilin replay: `host_cap_ceiling=4` in micro/small lanes, `dl_transport` rose to `18/0/0`, but payload bytes remained `0`

So the per-host cap helped align Crawli with libcurl/aria2 host-pressure control, but the onion bottleneck is still earlier probe/connect collapse rather than post-admission oversubscription.

## Priority order

1. Remove blanket `Connection: close` from clearnet and make onion keep-alive conditional.
2. Implement probe-to-transfer promotion for micro/small files.
3. Add a generic downloader host capability cache.
4. Add low-speed abort policy and telemetry.
5. Add degraded-host probe quarantine and stronger alternate-host cursor rotation before transfer scheduling widens again.

## Bottom line

The main libcurl lesson is not "open more connections". It is:

- remember what the host already proved
- reuse productive transport state
- queue instead of overdriving
- kill truly slow transfers by measured progress, not only elapsed time
- avoid spending two requests where one can do both learning and transfer

Those are exactly the places where Crawli can still cut requests and latency without blindly increasing concurrency.
