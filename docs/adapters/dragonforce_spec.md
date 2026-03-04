> **Last Updated:** 2026-03-04T13:30 CST

# DragonForce Adapter Flight Manual
*Adapter ID: `dragonforce`*
*Matching Strategy: M.A.C Tier 2 AST Tokenization (`__NEXT_DATA__` | `iframe src=".*token="`)*

## 1. Topography & Ingress
DragonForce utilizes a fully decoupled Next.js React Single Page Application (SPA). Instead of standard Nginx autoindexes, the HTTP Response body emits a serialized JSON AST object embedded inside a `<script id="__NEXT_DATA__">` window node. Furthermore, its root `.onion` frequently proxies this Next.js app inside an `<iframe>` appending an encrypted `token=` JWT signature.

**Known Roots:** 
- `dragonforce.onion`
- `fsguestuctexqqaoxuahuydfa6ovxuhtng66pgyr5gqcrsi7qgchpkad.onion`

**M.A.C. Regex Marker:** `__NEXT_DATA__|iframe.*token=`

## 2. DOM Interception Rules
*   **Engine:** Native `serde_json` + `scraper::Html`
*   **Phase 1 (Origin):** Intercept the <iframe> src JWT token. Rewrite the URL to push the exact `fsguest` sub-endpoint into the Crawler Frontier.
*   **Phase 2 (AST):** Extract `<script id="__NEXT_DATA__" type="application/json">` text. Deserialize mapping nested recursive dict arrays for `type="dir"` or `size`.

## 3. Deep-Crawl 2/2/2 Yield Metric
*   **Shallow Trap:** The outer root endpoint natively parses `0 Files, 7 Directories`. A 1-depth pass mathematically fails here!
*   **Deep Strategy:** The CI matrix relies on the recursive Frontier drilling into the 7 internal folders where the React AST emits the specific file arrays. The 2/2/2 threshold safely catches true API regression.

## 4. Known Bugs & Historical Evolutions
*   *Issue (Deprecated):* The original adapter scraped `<a>` tags with `.dir` classes. This completely broke when DragonForce migrated to Next.js Client-Side DOM compilation.
*   *Fix:* Stripped DOM reliance. Rewrote adapter to natively bypass the GUI entirely and intercept the window payload AST state natively.
