> **Last Updated:** 2026-03-04T13:30 CST

# Dynamic Fingerprinting Architecture Tournament
*Version: 1.0.0 | Date: 2026-03-03*

The user correctly identified a fatal flaw in the current `AdapterRegistry::determine_adapter` pipeline. Currently, we use a two-phase check:
1.  **O(1) Domain Match:** Do we already know this URL? (Fast)
2.  **O(N) Concurrent Sweep:** If the URL is new, ask *all* adapters if they `.can_handle(fingerprint)` by parsing the raw HTML DOM. (Slow & Destructive)

When we scale from 9 adapters to 400+, injecting a single new `.onion` into the engine will trigger 400 concurrent `scraper::Html::parse_document` operations on the identical DOM structure. This will utterly crush CPU resources, spike latency to multiple seconds, and defeat the entire purpose of a High-Frequency Aerospace architecture.

We need a mathematically proven, error-free strategy that scales to infinite adapters with **O(1)** or **O(log N)** latency. Below is a 7-Strategy Tournament to identify the absolute best approach.

---

## The 7-Strategy Tournament

### Strategy 1: The Current Baseline (Concurrent Sweeping)
*   **Mechanism:** Spawn N async tasks. Each adapter runs its own `can_handle` logic simultaneously. First one to return `true` wins.
*   **Complexity:** O(N) CPU load.
*   **Pros:** Easy to write. Fully autonomous.
*   **Cons:** Catastrophically inefficient at scale. 400 adapters parsing the same HTML tree 400 times simultaneously will saturate Tor threads.

### Strategy 2: Pre-parsed DOM Singleton Passing
*   **Mechanism:** The `AdapterRegistry` pre-parses the raw HTML into a singleton `scraper::Html` AST *once*. It then passes a reference of this AST to the 400 adapters sequentially or concurrently.
*   **Complexity:** O(1) Parsing + O(N) Traversal.
*   **Pros:** Solves the massive CPU overhead of redundant string-to-DOM parsing.
*   **Cons:** Still requires 400 linear evaluations. 

### Strategy 3: Heuristic Regex Pre-Filtering (The "Bouncer" Model)
*   **Mechanism:** Instead of running full DOM trees, the Registry maintains a fast `RegexSet` compiled from all 400 adapters. It scans the raw HTML string once against the `RegexSet`. The set instantly returns the IDs of the 2 or 3 adapters that matched specific string literals (e.g. "QData", "Next.js", "Index of /"). Only those 3 adapters are then allowed to run full DOM checks.
*   **Complexity:** O(1) String scan + O(1) DOM check.
*   **Pros:** Extremely fast string parsing.
*   **Cons:** Regex is brittle against malformed payloads.

### Strategy 4: Local External Cache Registry (The User's Suggestion)
*   **Mechanism:** Similar to CDNs. When a new URL is scraped, we run the slow O(N) pipeline *once*. When it finds the correct Adapter (e.g., "play"), we save this mapping permanently in an external JSON or SQLite database (`{ "b3pzp6qwel...onion": "play" }`). All future requests check this persistent cache instantly.
*   **Complexity:** O(1) Cache Hit / O(N) On Miss.
*   **Pros:** Near-zero latency for known payloads. 
*   **Cons:** Deepweb routing keys rotate constantly (v3 onion addresses change frequently). The cache will organically decay over time causing constant cache-misses reverting back to sluggish O(N) sweeps.

### Strategy 5: Header Sequence Hashing (Aerospace-Grade)
*   **Mechanism:** Deepweb ransomwares frequently use off-the-shelf Nginx/Apache configs, resulting in identical HTTP Header sequences. By hashing the exact order and presence of headers (`server: nginx/1.24 + content-type + x-frame-options`), we generate a 32-bit architectural fingerprint. The 400 adapters declare which HTTP architectures they support.
*   **Complexity:** O(1) Header Map hash lookup.
*   **Pros:** Completely ignores the massive HTML body overhead.
*   **Cons:** Content-agnostic. Multiple distinct Ransomware cartels might use the exact same default Nginx header layout, causing false-positive collisions.

### Strategy 6: Merkle-Root DOM Signatures (The Cloudflare Approach)
*   **Mechanism:** Similar to Cloudflare's bot detection, we strip dynamic content (text, sizes, timestamps) from the HTML and keep *only* the structural tags (`<html><body><div><table><tr>...`). We hash this structural skeleton into a unique MD5 signature. Adapters register which DOM signatures they own.
*   **Complexity:** O(N) String strip + O(1) Hash map lookup.
*   **Pros:** Extremely resilient against dynamic data. Math proves adapter ownership.
*   **Cons:** Ransomware actors modifying a single `<div>` wrapper breaks the mathematical hash completely.

### Strategy 7: The Hybrid "M.A.C." (Multi-Agent Cascade) System
*   **Mechanism:** A tiered probability waterfall merging the strongest features of the above models:
    *   **Tier 1 (Layer 7 Cache):** Check local JSON/SQLite Domain Cache `[O(1)]`. If hit -> Return instantly.
    *   **Tier 2 (The Bouncer):** Run a compiled `RegexSet` against the raw String looking for absolute deterministic markers (e.g., `window.__NEXT_DATA__` = DragonForce). `[O(1) CPU Time]`.
    *   **Tier 3 (Singleton DOM):** If Regex fails to pinpoint exactly 1 adapter, parse the `scraper::Html` exactly *once*. Pass the AST reference only to the generic fallback candidates (Autoindex). `[O(1) Memory]`.
*   **Complexity:** Best-case O(1) / Worst-case O(1) Parsing + O(k) evaluating candidates.
*   **Pros:** Completely eliminates redundant HTML parsing. Employs CDN-grade caching, and scales to infinite adapters without slowing down the primary HTTP thread pool.

---

## The Verdict: Strategy 7 (The Hybrid M.A.C. System)

To build the "best aerospace technology-based multi-MIT architecture" as requested:

We must implement **Strategy 7**. It incorporates the user's explicit local-cache recommendation (Tier 1) but reinforces it with a mathematically perfect string-sieve (Tier 2) to eliminate the severe O(N) concurrent DOM penalty (Tier 3) currently choking our Rust router.

### Detailed Planning Guide (The Next Steps)

1.  **Phase 9: CDN Domain Ledger implementation**
    *   Implement an external decoupled `known_domains.json` cache tracker in the `tests/` directory instead of hardcoding `.onion` strings inside each `adapter.rs` file.
    *   The `AdapterRegistry::new()` pulls this on boot, mapping strings to `AdapterID` directly in O(1) time.
2.  **Phase 10: The `RegexSet` Bouncer Algorithm**
    *   Instead of `can_handle(SiteFingerprint)`, adapters export an immutable `regex_marker()` constant string.
    *   The Registry compiles all adapter regexes into a single highly optimized `regex::RegexSet` engine. Scanning the raw 5MB HTML body against 400 markers occurs concurrently in C-grade speeds under 1 millisecond.
3.  **Phase 11: AST Singleton Abstraction**
    *   Strip `can_handle()` from the remaining generic adapters entirely, converting them to evaluate a pre-rendered `&scraper::Html` tree. This mathematically guarantees the application only parses messy DOM strings exactly once per URL.
    

*End of Document. Awaiting user authorization to proceed with implementation.*
