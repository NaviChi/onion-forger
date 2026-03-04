> **Last Updated:** 2026-03-04T13:30 CST

# Aerospace-Grade Adapter Strategy & Deep-Crawl Architecture
*Version: 1.0.0 | Date: 2026-03-03*

The user has correctly identified a critical flaw in the current testing paradigm: **Shallow Parsing vs. Deep Crawling**. Our current matrix only tests the *root* of a payload. For targets like DragonForce (Next.js SPAs), the root naturally yields `0 Files, 7 Directories`, which looks like a failure under shallow metrics, even though the files are buried inside those directories. Furthermore, blindly "skipping" offline targets (like Qilin) prevents us from mathematically proving the `qilin.rs` logic remains sound.

To meet the standards of Big Tech / CDN architecture (e.g., decentralized routing, maximum mathematical certainty, multi-layer fallbacks), we must implement the **Deep Autonomous Telemetry Mesh**.

---

## 1. The Strategy: Deep Autonomous Telemetry Mesh

### A. Decentralized "Flight Manuals" (Adapter Whitepapers)
Major tech infrastructure relies on highly localized documentation. Instead of keeping a monolithic crawler brain, **every adapter** will receive its own specific `docs/adapters/<name>_spec.md` whitepaper. 
These documents will mathematically define:
*   **Ingress Topology:** The known base URLs and URL routing rules (e.g., SPA query params).
*   **DOM Intercept Rules:** The exact `scraper` Selectors, JSON extraction logic, or JWT parameters expected.
*   **Expected Yield:** The known behavioral topology natively. 
*   **History & Implementations:** If a target alters its DOM, the whitepaper acts as an immutable ledger proving how it worked previously.

### B. True Deep-Crawl CI Validation (2-2-2 Thresholds)
We will deprecate the single-page parser test. A new testing engine (`adapter_deep_matrix_pipeline.rs`) will physically lock into the `CrawlerFrontier` and stream actual multi-threaded Tor traffic autonomously into the subfolders.
It will utilize the user-defined **Success Matrix (2-2-2 Protocol)**:
1.  **Iterate Folders Depth `> 2`**
2.  **Extract Total Folders `> 2`**
3.  **Extract Total Files `> 2`**

If the frontier halts, disconnects, or hits 0 before crossing all three thresholds, it is mathematically flagged as `NOT A SUCCESS`.

### C. Offline Health-Mocking (The CDN Fallback)
Like a CDN detecting a dead edge server, we cannot test Qilin if its `.onion` is completely down. However, we *can* test the parser. We will implement static HTML/JSON network mocks inside the CI test. If Qilin is unreachable, the test autonomously switches to "Mock Mode" feeding the `qilin.rs` adapter a cached payload to prove its DOM interceptors haven't decayed.

---

## 2. Inventory: Current Adapter Roster
We currently possess exactly 9 backend parsing adapters in `src-tauri/src/adapters/`:
1.  `autoindex.rs` (Generic Nginx/Apache fallback)
2.  `dragonforce.rs` (Next.js JSON SPA payload extractor)
3.  `inc_ransom.rs` (Heavy JSON pagination crawler)
4.  `lockbit.rs` (Custom Nginx nested table scraper)
5.  `nu.rs` (Nu Server specific UI scraper)
6.  `pear.rs` (Pear Ransomware Regex parser)
7.  `play.rs` (Play Autoindex variant parser)
8.  `qilin.rs` (QData UI layer delegator)
9.  `worldleaks.rs` (Custom UI grid scraper)

---

## 3. Detailed Planning Guide (Execution Workflow)

We will execute this transformation sequentially side-by-side:

### Phase 1: Whitepaper Generation
*   **Action:** I will autonomously create `docs/adapters/` and generate the 9 localized `<adapter>_whitepaper.md` files pulling exact context from our Rust implementation logic.
*   **Review:** This secures the history of all existing algorithms so they are never lost.

### Phase 2: Building the Aerospace Deep-Crawl Pipeline
*   **Action:** I will write the new `adapter_deep_matrix_pipeline.rs`. Instead of evaluating a single HTML file, it will autonomously invoke `run_crawl()`, traversing into subdirectories utilizing real Tor circuits.
*   **Action:** I will inject an `Atomic` telemetry trap that halts the process immediately when a success (`>2 depth, >2 folders, >2 files`) is achieved to save time.

### Phase 3: The Master Re-Crawl (Execution & Repair)
*   **Action:** We will run all 9 adapters against the true Deep-Crawl Matrix.
*   **Action:** We will compile the list of successful vs. failing targets.
*   **Action:** For the failing targets (specifically DragonForce and Qilin), we will isolate them one-by-one, fix their exact edge cases utilizing their new Whitepapers, patch the Rust code, and re-run until they successfully yield `>2` on all metrics!

Does this MIT/Aerospace-grade blueprint align exactly with your vision? Let me know and we will immediately spin up the Whitepapers and test engine!
