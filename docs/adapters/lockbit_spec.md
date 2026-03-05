> **Last Updated:** 2026-03-04T15:08 CST

# LockBit Adapter Flight Manual
*Adapter ID: `lockbit`*
*Matching Strategy: M.A.C Tier 1 Cache | Nested Autoindex Delegation*

## 1. Topography & Ingress
LockBit 3.0 acts uniquely as a highly fortified hybrid. While its front-facing `.onion` is a completely independent SPA utilizing unique grid logic, the actual *artifact storage servers* (the locations we care about crawling) are highly normalized headless Nginx `autoindex` endpoints protected by intense HTTP anti-DDoS checks.

**Known Roots:** 
- `lockbit.onion`
- `lockbit6vhrjaqzsdj6pqalyideigxv4xycfeyunpx35znogiwmojnid.onion`

**M.A.C. Regex Marker:** `None`

## 2. DOM Interception Rules
*   **Engine:** `CrawlerAdapter` Delegation (Zero-DOM Code)
*   **Strategy:** Natively inherits from the generalized `autoindex.rs` adapter. It explicitly maps unique HTTP request signatures (specifically avoiding aggressive browser User-Agents) and simply delegates `crawl()` to `<AutoindexAdapter as CrawlerAdapter>::crawl`.

## 3. Deep-Crawl 2/2/2 Yield Metric
*   **Target:** Highly successful. The LockBit artifact endpoint historically yields 379 Files and 6 directories exactly on its prime node. Reaching Folder depth > 2 explicitly proves the Tor pipeline has evaded IP Bans.

## 4. Known Bugs & Historical Evolutions
*   *Issue & Prevention:* Hardcoded verification arrays (`files == 379`) previously broke CI pipelines during organic cluster scaling. 
*   *Fix:* Merged logic into the `matrix_signatures.json` Dynamic Registry enabling Autonomous Learning bounds limits.
