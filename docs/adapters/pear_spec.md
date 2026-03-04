> **Last Updated:** 2026-03-04T13:30 CST

# Pear Ransomware Adapter Flight Manual
*Adapter ID: `pear`*
*Matching Strategy: M.A.C Tier 1 Cache | AST Payload Filtering*

## 1. Topography & Ingress
Pear acts as a lightweight leak blog primarily routing payloads through an intermediate `.org` extension path mounted locally to the onion instance. 

**Known Roots:** 
- `m3wwhkus4dxbnxbtihexlyd2cv63qrvex6jiebc4vqe22kg2z3udebid.onion/sdeb.org/`

**M.A.C. Regex Marker:** `None`

## 2. DOM Interception Rules
*   **Engine:** `scraper::Html`
*   **Selector:** Iterates target `<a>` strings searching for internal file links.
*   **Extraction:** Requires absolute resolution of `href` properties against the parent domain given its nested proxy configuration.

## 3. Deep-Crawl 2/2/2 Yield Metric
*   **Target:** Historically bounded to exactly `Files: 2, Dirs: 1` locally. Depending on how the `.org` ingress routes, a true 2/2/2 pass requires deeper execution.

## 4. Known Bugs & Historical Evolutions
*   *Issue:* Pear targets frequently go fully offline triggering `reqwest` timeout bugs spanning 5+ minutes on default configs.
*   *Fix:* Hard-limited Tor `CRAWL_TIMEOUT_SECS` universally across the `adapter_matrix_live_pipeline.rs` CI to 600s, preventing silent zombie threading.
