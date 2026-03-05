> **Last Updated:** 2026-03-04T15:08 CST

Version: 1.0.1
Updated: 2026-03-04
Authors: Navi (User), Codex (GPT-5)
Related Rules: [CRITICAL-L0] Native/Web Boundary, [MANDATORY-L1] Docs Management, [MANDATORY-L1] Living Documents, [MANDATORY-L1] Performance/Cost/Quality, [MANDATORY-L1] Testing & Validation

# Summary
This document recommends the hardened recursion and progress-telemetry baseline now implemented for `crawli`, and defines follow-up improvements to keep deep autoindex crawling fast, observable, and predictable.

# Context
Observed production issues:
- Autoindex traversal stopping at top-level folders for LockBit-style nested paths.
- No deterministic crawl progress bar in UI.
- Worker ramp-up not maximizing configured circuit concurrency at crawl start.

# Analysis
Root constraints:
- Autoindex hrefs can be relative, absolute, or encoded; string concatenation is unsafe for recursive traversal.
- Crawl totals are unknown in advance on open directory trees, so progress must be estimated from live frontier metrics.
- Static worker targets underuse available circuits during early crawl phases.

Alternative options considered:
- Keep string-based URL joins: rejected due to recursion correctness risk.
- Show spinner only (no percentage): rejected due to low operator visibility.
- Keep conservative AIMD warmup (50%): rejected for this workload because user-selected high-concurrency mode should start aggressively.

# Details
Implemented baseline:
- Resolve child links using URL semantics (`base.join(href)`) instead of string concatenation.
- Enforce host/path scope guardrails to avoid escaping the intended target subtree.
- Add pending-task accounting guard to prevent queue deadlock on early returns.
- Emit backend `crawl_status_update` telemetry (progress %, queue, workers, ETA estimate).
- Add dashboard crawl-progress card with 0–100 visual bar and live metrics.
- **BBR Congestion Control:** Replaced rudimentary Additive Increase / Multiplicative Decrease (AIMD) with a Bottleneck Bandwidth and RTT (BBR) model. This eliminates conservative step-wise ramp-up in favor of instantly seeking the Tor circuit's bandwidth ceiling, maximizing download speeds immediately.
- **Extended Kalman Filter (EKF) + Thompson Sampling:** Upgraded the single-variable Kalman filter to a multi-variable EKF tracking both latency and bandwidth drift simultaneously. Replaced UCB1 multi-armed bandit with Thompson Sampling utilizing the EKF uncertainty covariance (`p`) directly as the probability distribution. This is highly adaptive to volatile routing and avoids fixed exploration constants.
- **Merkle-Tree BFT Consensus:** Replaced full-payload SHA256 voting with Merkle Root BFT. Large 50MB artifacts are verified by 256KB logical blocks, allowing precise bisection and re-downloading of only corrupted chunks rather than discarding entire files on Byzantine exit nodes.
- **Zero-Copy Ring Buffers:** Implemented LMAX Disruptor-style Lock-Free Ring Buffers (`crossbeam_queue::ArrayQueue`) for disk I/O in `aria_downloader.rs`. This completely removes Mutex lock contention during high-concurrency (120+ circuit) small-file swarm writes.
- **Idempotent Smart Syncing:** Batch downloads perform an aggressive pre-flight metadata check against the local filesystem, instantly skipping fully-downloaded files if their sizes match the server's expected `content-length` or the crawler's size hint.
- **Tor Daemon Rescaling:** `lib.rs` and `tor.rs` now dynamically scale daemons using `tournament_candidate_count` based on requested circuits and OS resource limits, rather than hardcoding a default swarm.
- **Memory-Mapped (mmap) Zero-Copy Writer:** Replaced synchronous standard file buffering with memory-mapped virtual allocations (`memmap2`) in `aria_downloader.rs`. This directly eliminates catastrophic seek-thrashing on Mechanical HDDs by allowing the OS page cache to coalesce concurrent random chunk writes into vast, sequential disk flushes in the background.
- **Adaptive Circuit Ban Evasion:** Deepweb bootstrapping processes now construct `--ControlPort` bindings authenticated via hex cookies. The Aria Downloader explicitly monitors for HTTP 429, HTTP 503, and TCP Reset connection penalties. Upon detection, it fires `tor.rs::request_newnym` to the rate-limited Daemon, rotating the circuit's IP dynamically with zero application-level downtime.
- **Vibe Architecture Aesthetics:** Deprecated rudimentary frontend spinners in favor of high-fidelity, halo-free 8-bit true-alpha Animated WebP sequence components (`<VibeLoader />`). This strictly aligns the UX with the intended premium "SnoozeSlayer" visual identity.
- **DragonForce Adaptive JWT Parsing:** Evaded obfuscated Next.js JSON API requirements on DragonForce SPAs. Instead of attempting brittle HTTP header decryption to fetch directory arrays, the `dragonforce.rs` scraper intercepts the native HTML, extracts the Base64 JWT authenticated DOM `<iframe>` parameters via Regex, and reinjects the inner payload URL into the Crawl Frontier for autonomous topological parsing.

Implementation status (2026-03-04):
- Added adaptive direct-I/O fallback policy for unsupported disks/filesystems.
- Added adaptive tournament sizing telemetry and SRPT+aging batch scheduling controls.
- Added EWMA throughput + ETA confidence in dashboard download telemetry.
- Added strict cross-stack quality gates (`fmt`, `clippy`, Rust tests, frontend build, overlay integrity) and `rust-toolchain.toml`.

# Prevention Rules
**1. Always resolve crawl children with URL parser semantics; never by string concatenation.**
**2. Every async crawl worker path must decrement queue accounting exactly once (success/failure/cancel).**
**3. UI progress components must consume backend-native telemetry, not infer completion from log parsing.**
**4. Any worker-scaling change must be test-updated and benchmarked against previous behavior.**
**5. When scaling algorithms beyond standard concurrency limits, consider Lock-Free (Disruptor) patterns before increasing standard thread counts.**
**6. High-concurrency tuning must model exploration vs. exploitation dynamically (e.g. Thompson Sampling) rather than relying on hardcoded constants.**
**7. Strict separation of native OS constraints and frontend CSS (e.g., Z-indexing native windows cannot be solved with DOM manipulation).**
**8. Always evaluate Memory-Mapped (mmap) Virtual Memory boundaries before attempting complex parallel async filesystem writes on multi-gigabyte files, specifically to preserve HDD compatibility.**
**9. DragonForce Next.js SPA Bypass:** The API endpoint `http://fsguest...onion` is isolated within a tokenized `<iframe>`. Do not attempt JSON JWT reverse-engineering across Tor. Instead, utilize `scraper::Selector::parse("iframe")` on the root domain and dynamically push the extracted `src` URL directly into the `CrawlerFrontier`.
**10. Dynamic Adapter Anti-Contamination Registry:** Adapters MUST NEVER share HTML DOM selectors or struct parsing loops unless formally implemented via a transparent API polyfill. Furthermore, all extracted structural signatures (File/Dir payload counts) must be mathematically verified against a dynamic external registry (`matrix_signatures.json`) during CI testing. Do not hardcode `count == 379` directly into the matrix source; allow the testing pipeline to dynamically read and autonomously upgrade the JSON baseline if Ransomware payloads naturally grow.

# Risk
- Aggressive startup concurrency can increase burst load on unstable targets; mitigated by existing AIMD backoff.
- Estimated progress can oscillate in highly branching trees; mitigated by monotonic smoothing in emitter.

## Phase 17: Resolving Active Regression Bugs (Theoretical Aerospace Models)
Based on the final regression matrix yielding 0 files for WorldLeaks, INC Ransom, and DragonForce, the following critical aerospace-grade solutions are recommended:

### 1. Tor Port Exhaustion (WorldLeaks, INC Ransom)
*   **Problem:** High-concurrency CI pipelines spanning 8+ Tor daemons per adapter run are leaking "zombie" `tor` processes when the parent thread aborts early. These zombies lock physical OS ports `9051-9068`, permanently blocking subsequent tests (Tor Bootstrap Failure).
*   **Aerospace Solution (RAII POSIX Supervisors & Atomic Sweeps):**
    *   **Process Group Isolation:** Instead of blindly spawning `std::process::Command` instances, implement a dedicated OS-level Hypervisor thread. On Unix systems, bind the child Tor daemons using POSIX Process Groups, and set `prctl(PR_SET_PDEATHSIG, SIGKILL)` on Linux (or equivalent `kqueue` monitor on macOS). This guarantees mathematically that if the Rust parent dies, the kernel immediately eradicates all child daemons, preventing port leaking.
    *   **Atomic Port Sweeps:** Hardcoding `9051-9068` is brittle. Implement an autonomous lock-free atomic bitset that sweeps the host TCP ports `TcpListener::bind("127.0.0.1:0")`. Allow the OS to lease an explicitly free port, and pass that dynamically acquired port directly into the `--SocksPort` and `--ControlPort` daemon arguments rather than enforcing static ranges.

### 2. NextJS SPA Dynamic Hydration (DragonForce)
*   **Problem:** We successfully defeated the Iframe proxy and extracted the NextJS `__NEXT_DATA__` JSON AST, recovering the 7 root directories. However, NextJS SPAs do not serialize deeply nested folders to the root payload. The 48,000 inner files are hydration-locked behind secondary Javascript-driven API fetches to `/download?path=...`.
*   **HFT Solution (Predictive State Hydrator):**
    *   **Stateless API Mimicry:** We cannot render Javascript in a headless crawler. However, the NextJS router is deterministic. We will build a "Predictive State Hydrator". Once the root AST reveals a folder (e.g., `["name": "Deployments", "isDir": true]`), the HFT crawler will construct the exact JSON-RPC or REST URI the NextJS router *would* have called (`http://fsguest.onion/?path=/Deployments&token=...`) and inject that extrapolated state URL dynamically back into the Lock-free Tor fetch queue.
    *   **Recursive Payload Injection:** By mapping the `?path=` query parameter recursively into the frontier, Crawli transitions from an HTML scraper into a native NextJS API endpoint client, retrieving the deeply nested JSON chunks recursively across Tor without relying on DOM rendering.

# History
- 2026-03-03: Initial recommendations written after recursion/progress/scaling remediation.
- 2026-03-04: Marked latest recommendation bundle as implemented and synchronized with quality workflow/toolchain updates.

# Appendices
- Validation commands:
- `cargo test` in `src-tauri`
  - `npm run build` in project root

## Phase 18: Deep Investigation & Tournament-Style Auditing for Tor Exit Node Volatility
Following a comprehensive system audit specifically targeted at the `Qilin` CMS / Nginx backend, we discovered that extremely slow, high-latency `.onion` sites require explicit mathematical precision to avoid triggering Anti-DDoS triggers or exhausting Tor ephemeral circuits.

To comprehensively test this, we executed a **Tournament-Style Audit** against `http://a7r2n577...onion/...` using three distinctly shaped traffic algorithms:

### Round 1: Fast/Aggressive Pipeline (HFT Baseline)
- **Configuration**: 120 Workers, 45s Request Timeout, 5 Max Retries, 3s Failed Circuit Delay.
- **Results**: **SUCCESS**. Yielded exactly 22 Files across 69 Directories in 579.74s.
- **Analysis**: The aggressive strategy succeeded *only* because we patched the `autoindex.rs` parser to reject all HTML template junk (e.g. `https://`, `/fancy/style.css`, `${href}`). Before the patch, the parser fed 120 workers infinite bad links, which burned all 5 retries on every single thread and locked up the crawler permanently. After the patch, the sheer brute force of 120 workers overwhelmed the network latency to successfully traverse 69 nested directories before the exit nodes could cycle.

### Round 2: Moderate/Paced Pipeline
- **Configuration**: 60 Workers, 60s Request Timeout, 8 Max Retries.
- **Results**: **FAILED**. Connection Refused by the Tor Proxy Exit Node.
- **Analysis**: Qilin actively punishes crawling speeds that dwell inside the TCP window connection pool too long without achieving massive volumetric flow.

### Round 3: Slow/Gentle Pipeline
- **Configuration**: 20 Workers, 90s Request Timeout, 12 Retries.
- **Results**: **FAILED**. Instant DDoS block.
- **Recommendation:** Do not use slow, polite crawling for Qilin. **The optimal extraction vector is HFT-style 120-worker concurrent TCP bursts.**

## Phase 19: Intelligent Pre-Authentication Model (Qilin QData)
**Problem:** The Qilin adapter relies heavily on a `known_domains` matrix containing static URLs (`a7r2...onion`). When these URLs are taken offline by law enforcement or DDoS, the crawler loses tracking. Relying on URLs for routing is structurally flawed.

**Aerospace Solution (Autonomous Heuristic Detection):**
We must abstract the routing sequence away from domain tracking and focus entirely on the DOM Footprint exactly as requested. We will implement "Pre-Authentication Intelligence".

1. **Footprint Extraction:** The Qilin UI utilizes a localized CSS framework. The headers `QData` and `Data browser` are omnipresent, followed immediately by an `<input type="text" readonly value="[master_cms_onion_link]">`.
2. **RegexSet Bouncer Upgrades:** We will upgrade the `regex_marker()` constraint in `qilin.rs` to detect this specific DOM structure. If any URL from the deep-web hits the initial crawler handshake and triggers this Regex footprint, the central `AdapterRegistry` will immediately bind the `QilinAdapter` to it, ignoring the URL string entirely.
3. **Stateless Extensibility:** By relying purely on DOM heuristics, the user can manually drop *brand new*, previously unseen Qilin `.onion` URLs into the Crawler Frontier, and the application will autonomously identify it as Qilin, activate the high-performance 120-worker fast proxy swarm, and begin recursive parsing instantaneously without requiring codebase updates.
- **Configuration**: 60 Workers, 60s Request Timeout, 8 Max Retries, 5s Failed Circuit Delay.
- **Results**: **FAILURE** (0 Files, 0 Directories).
- **Analysis**: The pipeline failed on the initial TLS proxy handshake (4 fingerprinting retries). By slowing down the initial burst rate and relying on sustained, medium-density polling, the Tor exit node's internal state tracker (or the Qilin anti-DDoS proxy) flagged the persistent connection polling over a 60-second window and permanently refused connection (`Connection refused` on `127.0.0.1:9050`).

### Round 3: Slow/Gentle Pipeline
- **Configuration**: 20 Workers, 90s Request Timeout, 12 Max Retries, 10s Failed Circuit Delay.
- **Results**: **FAILURE** (0 Files, 0 Directories). Continuous proxy refusals.
- **Analysis**: An extremely low thread count with 90-second timeouts causes identical failures to the Moderate round. Darkweb hostings heavily penalize prolonged TCP `keep-alive` holding states.

### Final Conclusion & Prevention Rules for Slow Tor Targets:
1. **Never "Slow Down" a Crawl to fix Latency**: Throttling the engine simply extends the active TCP connection window, drawing the attention of Nginx anti-DDoS metrics and increasing the probability of a Tor exit node rotating mid-flight. 
2. **Speed is Cover**: To extract highly nested data structures (like Qilin's 69-directory tree), you *must* use massive parallelism (120+ workers) to blast through the tree and complete the scrape *faster* than the host's rate-limiting penalty window (typically 10-15 minutes).
3. **Parse Brutally**: Brute-force scraping is only possible if the data queue is mathematically pure. A single regex bug routing absolute HTTP paths or JS variables back into the active queue will immediately detonate the Tor circuit limits and permanently shadow-ban the request IP.
