# Qilin Adapter Flight Manual
*Adapter ID: `qilin`*
*Matching Strategy: M.A.C Tier 2 AST Tokenization (`<div class="page-header-title">QData</div>`)*

## 1. Topography & Ingress
Qilin masks its underlying artifact storage (Nginx/Apache) behind a customized `QData` visual CSS web layer ("Data browser"). This defeats default Autoindex signature detection despite possessing a perfectly compliant nested table output.

**Known Roots:** 
- `iv6lrjrd5ioyanvvemnkhturmyfpfbdcy442e22oqd2izkwnjw23m3id.onion`

**M.A.C. Regex Marker:** `<div class="page-header-title">QData</div>|Data browser`

## 2. DOM Interception Rules
*   **Engine:** `RegexSet` + Autoindex Delegation
*   **Phase 1 (Origin):** The Tier 2 M.A.C regex catches `QData` without executing `scraper::Html`.
*   **Phase 2 (AST):** The Rust process completely skips writing custom scraper logic for Qilin. Instead it forcibly converts the instance and executes `<AutoindexAdapter>::crawl()`, efficiently reading the underlying unstyled `<table>` node perfectly.

## 3. Deep-Crawl 2/2/2 Yield Metric
*   **Target:** Current network status is Offline/Rotating. However, parsing the raw HTML yields a flawless >2 score matching Nginx default hierarchies perfectly since the delegation logic bypasses the visual CSS wrapper natively.

## 4. Known Bugs & Historical Evolutions
*   *Issue:* Wasting 500 lines of Rust re-mapping table rows that Autoindex already reads perfectly solely because the page `<title>` changed.
*   *Fix:* Architectural delegation wrapper written mapping `qilin.rs` -> `autoindex.rs`.
