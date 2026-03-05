> **Last Updated:** 2026-03-04T15:08 CST

# Nu Server Adapter Flight Manual
*Adapter ID: `nu_server`*
*Matching Strategy: M.A.C Tier 1 Cache | Nested Autoindex Delegation*

## 1. Topography & Ingress
The Nu Server is a lightweight storage architecture frequently observed routing massive file trees. It operates fundamentally similarly to Nginx Autoindex but strips out default header identifiers.

**Known Roots:** 
- `nu-server.onion`

**M.A.C. Regex Marker:** `None`

## 2. DOM Interception Rules
*   **Engine:** `CrawlerAdapter` Delegation (Zero-DOM Code)
*   **Strategy:** Identical to LockBit arrays. It inherits heavily from `<crate::adapters::autoindex::AutoindexAdapter as CrawlerAdapter>::crawl`. Native tables extract cleanly.

## 3. Deep-Crawl 2/2/2 Yield Metric
*   **Target:** Verified >2 yield across all metrics locally. Due to the high stability of raw autoindexes, Nu natively handles deep structural mappings without triggering bot protection.

## 4. Known Bugs & Historical Evolutions
*   *Issue:* Originally attempted custom `<td class="index">` parsing. 
*   *Fix:* Fully decoupled custom parsing into the underlying Autoindex AST engine, reducing memory overhead and isolating `nu.rs` strict routing logic.
