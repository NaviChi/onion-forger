> **Last Updated:** 2026-03-04T13:30 CST

# Theoretical Algorithm Battle: High-Frequency Tor Crawling & Downloading

**Objective:** Analyze the current standard algorithms against High-Frequency Trading (HFT) and Aerospace-grade topologies for Tor-based network crawling and downloading. Determine the optimal implementation paths.

---

## 1. Tor Daemon Scaling & Parallelism
**Current State:** Linear/Hardcoded scaling (e.g., spin up N candidates, pick M winners).
**The Battle:** 
- *Pro Massive Scaling:* Spinning up 20 daemons to pick 10 allows us to aggressively filter out the absolute worst Tor routes during the bootstrap phase. It increases overall aggregated bandwidth capacity since Tor relies on strictly single-threaded crypto operations per daemon.
- *Con Massive Scaling:* High memory and CPU overhead. Tor daemons consume 50-100MB RAM each. 20 daemons = 2GB RAM just for proxy idling, plus thousands of ephemeral TCP sockets.
- *Winner Configuration:* **Dynamic Logarithmic Tournament Scaling**. If the user UI requests 180 circuits, we do not need 180 daemons. A single Tor daemon efficiently multiplexes ~30-50 high-speed circuits. 
  - Implementation: `target_daemons = max(4, circuits / 30)`. 
  - Tournament Candidate Pool: `target_daemons + (target_daemons / 2)`. This provides a 50% buffer for stragglers and bad handshakes without quadratic system overhead. 

## 2. Crawl State Resumption vs Fresh Crawls
**Current State:** `CRAWLI_WAL_RESUME=1` enables WAL (Write-Ahead-Log) DB restoration.
**The Battle:**
- *Pro Resume:* Saves traversal time on massive directory architectures (100k+ files).
- *Con Resume:* In volatile web environments, directory structures mutate constantly. Resuming a 2-day-old crawl might miss newly deposited files or attempt to queue deleted ones, causing failure cascades.
- *Winner Configuration:* **Forced Fresh Crawls (by default)**. The crawler must always traverse the tree from scratch utilizing the massive concurrency pool (120+ circuits). The WAL persists for crash-loop analysis but is purged on every UI-initiated crawl. This ensures perfectly accurate, ground-truth target mapping 100% of the time.

## 3. BBR Congestion Control vs AIMD
**Current State:** AIMD (Additive Increase, Multiplicative Decrease).
**The Battle:**
- *Pro AIMD:* Safe, standard TCP-era logic. Prevents overwhelming the local network.
- *Con AIMD:* Incredibly conservative. Halving the concurrency window on a single Tor timeout is devastating, as random jitter is inherent to the Tor network. Linear increase leaves bandwidth unutilized for minutes.
- *Winner Configuration:* **Application-Layer BBR (Bottleneck Bandwidth and RTT)**. We pace our application-layer requests by tracking `max_delivered_rate` (bandwidth) and `min_rtt`. Allowed concurrency dynamically snaps to `max_delivered_rate * min_rtt`. It instantly scales up to the actual bandwidth ceiling and completely ignores transient exit-node jitter.

## 4. Thompson Sampling vs UCB1 for Circuit Routing
**Current State:** UCB1 (Upper Confidence Bound).
**The Battle:**
- *Pro UCB1:* Deterministic exploration guarantees every circuit is tested based on visit counts.
- *Con UCB1:* Relies on a rigid, hardcoded exploration constant. A fast Tor circuit might suddenly collapse. UCB1 takes too long to "unlearn" its confidence bounds when variance fluctuates wildly.
- *Winner Configuration:* **Thompson Sampling with EKF**. We fuse the Extended Kalman Filter (EKF) to maintain the state (mean) and covariance (uncertainty) of each circuit's speed. Thompson Sampling draws a random sample from each circuit's probability distribution `N(mean, covariance)` and picks the highest. Unused circuits grow in uncertainty, dynamically triggering exploration WITHOUT hardcoded constants. This is the mathematical maximum for non-stationary multi-armed bandit problems.

## 5. Merkle-Tree BFT vs Monolithic SHA256 Verification
**Current State:** Full-chunk SHA256. If a 5MB chunk fails Quorum, all 5MB are discarded.
**The Battle:**
- *Pro Monolithic:* Trivial to implement.
- *Con Monolithic:* Byzantine/malicious Tor exit nodes frequently flip bits. Discarding an entire 5MB block because of 1 corrupted byte costs massive amounts of bandwidth and retry time.
- *Winner Configuration:* **Merkle-Tree Quorum Verification**. A 5MB chunk is logically segmented into 256KB blocks. Hash each block to build a Merkle Tree. When cross-referencing with quorum peers, we compare the Root Hash. If they differ, we traverse the tree to find the exact 256KB block that was poisoned. Only that 256KB sub-block is discarded and re-downloaded. Pure Aerospace-grade data integrity.

## 6. Zero-Copy Ring Buffers vs Mutex MPSC for Disk I/O
**Current State:** `mpsc` queue to a dedicated background I/O writer.
**The Battle:**
- *Pro MPSC:* Safe asynchronous actor model, detaches network from disk.
- *Con MPSC:* At ultra-high scale (180+ circuits pushing chunks concurrently), `mpsc` channel locking, message allocation overhead, and `BufWriter` buffer flushes introduce micro-stalls across the async runtime executor.
- *Winner Configuration:* **LMAX Disruptor-Style Ring Buffer**. Pre-allocate a large contiguous block of memory (e.g., 64MB ring). Active downloading circuits claim a sequence number atomically (Lock-Free `AtomicUsize`), write data directly into the array, and publish. A dedicated background thread spins sequentially reading memory and blasting to disk via raw OS writes (`O_DIRECT`). Complete eradication of Mutexes and allocation locks on the hot path.
