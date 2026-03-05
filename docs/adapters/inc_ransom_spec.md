> **Last Updated:** 2026-03-04T15:08 CST

# INC Ransom Adapter Flight Manual
*Adapter ID: `inc_ransom`*
*Matching Strategy: M.A.C Tier 1 Cache | M.A.C Tier 3 Fallback*

## 1. Topography & Ingress
INC Ransom uses a heavy paginated blog structure. Instead of extracting raw filesystem directories, the pipeline translates blog "disclosures" into logical sub-folders containing the target victim's artifacts. 

**Known Roots:** 
- `incblog6qu4y4mm4zvw5nrmue6qbwtgjsxpw6b7ixzssu36tsajldoad.onion`

**M.A.C. Regex Marker:** `None`

## 2. DOM Interception Rules
*   **Engine:** `scraper::Html`
*   **Selector:** Targets disclosure API grids mapping `/blog/disclosures/[uuid]` URLs. Extracts nested `<a>` elements dynamically rebuilding structural layouts.
*   **Constraints:** Operates under extremely heavy structural bloat. Needs careful stream separation to ensure `FileEntry::raw_url` safely points directly to the leaked payload download instead of the blog post preview UI.

## 3. Deep-Crawl 2/2/2 Yield Metric
*   **Target:** Easily clears the 2/2/2 parameter. Since each disclosure maps to its own folder structure containing 10,000+ files natively, hitting depth 2 immediately confirms full traversal mapping capability.

## 4. Known Bugs & Historical Evolutions
*   *Issue:* Unbounded blog pagination trapping crawler workers inside infinite `next_page` loop vectors.
*   *Fix:* Leveraged Bloom-Filter deduping inside the `CrawlerFrontier` globally preventing duplicate URLs from being queued twice natively, securing the ingress.
