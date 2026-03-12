
## [Phase 116] Patient Retry Mode — Automatic Recovery for Dead Infrastructure
- **Issue**: Phase 115 identified that all 47 storage nodes for IJZN c9d2ba19 are dead. The system exhausted all fallback stages (A→B→C→D→42→77) in 130.6s and gave up — user gets zero files with no automatic recovery path.
- **Exact Fix**: Added `Phase 116 Patient Retry Mode` in `qilin.rs`: after all storage discovery paths fail, the system enters a configurable retry loop (default: every 15m, up to 96 retries = 24h). Each retry: (1) waits the interval checking cancel flag every 10s, (2) NEWNYM refreshes all Tor circuits, (3) resets all node cooldowns via new `QilinNodeCache::reset_all_cooldowns()` in `qilin_nodes.rs`, (4) tries fresh CMS /site/data redirect to discover newly rotated hosts, (5) re-runs full `discover_and_resolve_prioritized()`. If any node comes alive, immediately resumes crawl. Emits 6 Tauri events for GUI integration.
- **Prevention Rule**: Long-lived Tor hidden services frequently rotate storage infrastructure. Never permanently give up on a target when the CMS is still alive — the storage nodes WILL come back on new `.onion` addresses. Patient retry with cooldown reset + fresh circuit rotation is mandatory for infrastructure-unstable targets.

## [Phase 115] All Storage Nodes Offline — Zero Files Resolved
- **Issue**: CLI full test against IJZN c9d2ba19 (TBC Consoles) completed in 130.6s but resolved zero files. All 40+ cached storage nodes returned connect failures or TTFB timeouts. CMS is alive but no QData storage mirrors are reachable. Previous successful crawl (March 5) had 35,069 entries.
- **Exact Fix**: Infrastructure-level failure. Storage nodes rotated/decommissioned since March 5. Existing fallback cascade (Stage A→B→C→D→Phase 42→Phase 77) functioned correctly. Resolved by Phase 116 Patient Retry Mode.
- **Prevention Rule**: When all nodes are dead: (1) enter patient retry mode (Phase 116), (2) emit early user notification, (3) check for fresh CMS redirects on each retry.

## [Phase 103] Qilin Active Probe Deadlock & CLI Startup Overheads
- **Issue**: The Qilin adapter exhausted its 3 probe candidates and immediately entered a 3/3 quarantine state, remaining at 0 bytes. Furthermore, running validation tests spent ~11.8s wasting time in native Window Wry startups despite being the CLI variant.
- **Exact Fix**: In `src-tauri/src/cli.rs`, forcefully executed `ctx.config_mut().app.windows.clear()` before the Tauri Builder compiles to bypass the native bridge delay. In `aria_downloader.rs`, corrected `ordered_probe_candidates` to explicitly bias exhausted fully-quarantined sets to surface the active `current-snapshot` host (`entry.url`), ignoring arbitrary rotation ranks that kept re-arming the same degraded failure node.
- **Prevention Rule**: Headless CI and CLI validation MUST explicitly evict OS window definitions dynamically prior to `.build()` or adopt purely generic emitter channels. Probe architectures MUST revert to the most reliable known-state configuration when all rotational fallbacks degrade.

## [Phase 104] IDM-Tier Stream Bisection Bottlenecks
- **Issue**: Achieved 1.03+ MB/s (8.24 Mbps) over Tor swarm by packing 32 connections per proxy, but we are repeatedly hitting 'send timeout' reqeueues and RSS memory bloat under extreme multiplexing load.
- **Exact Fix**: Engineered dynamic Stream Packing inside aria_downloader via target_pieces=32 and host_connections=32, coupled with resource_governor cap extensions to 64 limiters.
- **Prevention Rule**: High-concurrent asynchronous Tor stream engines require Kernel-level zero offset buffering (Mmap), or standard OS MPSC queues will throttle the async runtime causing premature TCP send timeouts.

## [Phase 105] Lock-Free Mmap Constraints and CPU Hash Blocking
- **Issue**: Disk IO bound constraints over 64 concurrent Tor streams surfaced as the synchronous `hasher.update()` locked the OS physical flush thread while the crossbeam `ArrayQueue` choked the Tokio async receive buffer with allocated `bytes::Bytes` blocks.
- **Exact Fix**: Adopted `memmap2::MmapMut` inside an `Arc<LockFreeMmap>` generic struct to allow async streams to cast incoming memory chunks immediately onto the memory map via lockless pointers. Offloaded the synchronous `hasher.update()` onto a specifically mapped `tokio::task::spawn_blocking` utilizing dedicated SIMD threads by exchanging lightweight slice offsets alongside `HashPayload` messages rather than bulk arrays.
- **Prevention Rule**: All high-performance async payload processors operating across 20+ circuits must explicitly bypass MPSC memory queues for payload data. Direct pointer casting into active OS-Paging subsystems (Mmap) handles multi-gigabyte files (e.g. 50+ GB) flawlessly without expanding resident RSS limits, entirely avoiding user-space duplication or OOM conditions.

## [Phase 106] Legacy Adapter Async Starvation & Queue Bloat
- **Issue**: Under recent fast-path optimizations, standard adapters like `Lockbit` experienced massive queue expansions (e.g., Queue=350+, Processed=20) because they utilized headless `max_concurrent=1` worker caps. This caused them to completely ignore the heavily provisioned `MultiClientPool` (8-16 workers) instantiated by the `CrawlerFrontier`, blocking 95% of Tor bandwidth capacity.
- **Exact Fix**: Forcefully overrode single-worker bottlenecks inside the adapter logic (e.g. `src-tauri/src/adapters/lockbit.rs`) to evaluate `std::cmp::max(frontier.recommended_listing_workers(), 16)`, injecting deep parallel loops immediately.
- **Prevention Rule**: Never assume the default headless `max_concurrent` scale is optimal. Explicit minimum worker caps must be coded into `tokio::task::JoinSet` to prevent multi-circuit Tor swarms from sitting idle while sequential queues balloon exponentially.
