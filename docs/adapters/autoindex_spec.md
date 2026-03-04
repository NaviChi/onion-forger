# Autoindex Adapter Flight Manual
*Adapter ID: `autoindex`*
*Matching Strategy: Universal Fallback (`<a href="...">`)*

## 1. Topography & Ingress
The Autoindex adapter acts as the absolute baseline parser for unmasked Nginx, Apache, and standard HTTP directory listings. It is natively assigned as the fallback in the M.A.C Tier 3 system.

**Known Roots:** None (Universal Catch-All)
**M.A.C. Regex Marker:** `None`

## 2. DOM Interception Rules
*   **Engine:** `scraper::Html`
*   **Selector:** `<a href="...">`
*   **Constraints:** Expects heavily generic `<tr>` or `<li>` nested structures. Since standard `Index of /` pages format sizes identically, the parser scrapes text succeeding the `<a>` tag to locate strings matching `\d+\s*(B|KB|MB|GB)`.

## 3. Deep-Crawl 2/2/2 Yield Metric
As the baseline parser, this adapter *must* successfully recurse multiple folders instantly if pointed at a generic directory structure. 
*   **Depth Target:** > 2
*   **Folders Target:** > 2
*   **Files Target:** > 2 (Native capability proven against standard HTTP payloads).

## 4. Known Bugs & Historical Evolutions
*   *Issue:* Nginx frequently lists `../` as an anchor link pointing backwards.
*   *Fix:* The `autoindex` adapter natively strips any `href` exactly matching `../` or `/` to prevent infinite recursive loop caching within the `CrawlerFrontier`.
