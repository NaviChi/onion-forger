# WorldLeaks Adapter Flight Manual
*Adapter ID: `worldleaks`*
*Matching Strategy: M.A.C Tier 1 Cache | SPA Graph AST*

## 1. Topography & Ingress
WorldLeaks maintains a dedicated Single Page Application (SPA) tracking corporate storage URLs across complex subpaths (e.g. `/companies/[uuid]/storage`). The DOM heavily leverages component-based architecture causing standard anchor logic to drop contexts.

**Known Roots:** 
- `worldleaksartrjm3c6vasllvgacbi5u3mgzkluehrzhk2jz4taufuid.onion`

**M.A.C. Regex Marker:** `None`

## 2. DOM Interception Rules
*   **Engine:** `scraper::Html`
*   **State Extraction:** Production adapter tracking heavily nested component IDs to parse the difference between valid leak sub-directories versus generalized corporate splash pages.

## 3. Deep-Crawl 2/2/2 Yield Metric
*   **Target:** Verified routing path. Due to Tor ingress drops (timeout), achieving a physical 2-layer-deep map is solely constrained by `.onion` availability.

## 4. Known Bugs & Historical Evolutions
*   *Issue:* Constant Tor proxy routing timeouts fetching TLS properties.
*   *Prevention Strategy:* All connections are piped through an Aerospace-Grade Extended Kalman Filter (EKF) evaluating Tor Daemon path-loss dynamically rotating down-circuits.
