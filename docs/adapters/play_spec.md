> **Last Updated:** 2026-03-04T13:30 CST

# Play Ransomware Adapter Flight Manual
*Adapter ID: `play`*
*Matching Strategy: M.A.C Tier 1 Cache | Body Struct Hash*

## 1. Topography & Ingress
Play utilizes an intensely specific base64-layer trailing string configuration for its leak sites, natively modifying how recursive folders are indexed on the frontend.

**Known Roots:** 
- `b3pzp6qwelgeygmzn6awkduym6s4gxh6htwxuxeydrziwzlx63zergyd.onion/FALOp`

**M.A.C. Regex Marker:** `None`

## 2. DOM Interception Rules
*   **Engine:** `scraper::Html`
*   **Selector:** Specific Nginx variant tracking URL paths ending cleanly in the Play `FALOp` encoding format.
*   **Extraction:** Heavily tested explicitly via `play_features_test` feature resilience matrices.

## 3. Deep-Crawl 2/2/2 Yield Metric
*   **Target:** Maps natively to 11 Files, 1 Directory on core roots. Safe against 2/2/2 logic.

## 4. Known Bugs & Historical Evolutions
*   *Prevention Strategy:* Due to Play's extreme testing coverage (most heavily unit-tested adapter in the framework), the M.A.C engine handles Play instantly upon detection.
