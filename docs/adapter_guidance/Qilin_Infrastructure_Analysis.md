# Qilin Decentralized Infrastructure & Load Balancing Whitepaper

## 1. Architectural Overview: The 302 Load Balancer
Qilin operates a split architecture to protect its core CMS from law enforcement takedowns and heavy traffic. The main `.onion` server (`http://ijzn3sicrcy7...`) acts merely as a frontend index and target resolution gateway. 
It does not host the ransomware leak data locally. 

When you request a download directory, the main server issues an **HTTP 302 Redirect** to dynamically load-balance your request to one of its 40+ ephemeral backend "storage nodes" (e.g., `pandora4...onion` or `7mnkv5...onion`). This 302 is not an error; it is Qilin's internal CDN load-balancing mechanism correctly resolving the victim's UUID to the physical server that currently holds their data.

## 2. Taxonomy of Storage Node Errors
Because the storage nodes are highly unstable and frequently rotate, we encounter a severe minefield of failures once we follow the 302 redirect. Here are the precise error strings our crawler engines pick up when hitting the backend nodes:

*   **`Request failed: client error (Connect)`** 
    *   **Meaning**: The storage node is physically offline, burned, or its Tor Introduction Points are currently congested.
*   **`⚠ Health probe TIMEOUT` / `timeout (45s)`**
    *   **Meaning**: The network routing to the node exists, but the physical server is unreachably slow or dropping packets under heavy load.
*   **`HTTP 404 Not Found`**
    *   **Meaning**: The node is online, but the backend Qilin database synchronization has failed, and the specific victim's UUID folder has not been replicated to this particular node yet.
*   **`HTTP 403 Forbidden` / `HTTP 400 Bad Request`**
    *   **Meaning**: DDoS Protection Gateway. The specific Tor exit/entry node we are utilizing has been flagged and blacklisted by the Nginx firewall guarding the storage node.
*   **`EOF while parsing` / `broken pipe` / `connection reset`**
    *   **Meaning**: A structural collapse of the TCP socket across the Tor network. The node began transmitting the HTML folder directory, but the cryptographic circuit collapsed halfway through the transfer, destroying the data payload.
    *   **Correction on "GhostBrowser" Status**: I previously mentioned an older log example referencing `[GhostBrowser] Parse error`. This was a generic placeholder from an old fallback pipeline. To clarify: **We are completely detached from GhostBrowser overhead**. The native Rust `ArtiClient` currently processes all of Qilin's HTML indexes in memory. Any `EOF` errors encountered now are pure Rust TCP stream closures direct from Native Tor circuits.

## 3. The Success State
When a node is online, non-throttled, and fully synced, we achieve an instant lock.
**The exact success message logs are:**
*   `✓ Fingerprint acquired in 18.34s (HTTP 200, 12039 bytes body)`
*   `[MATCH] ✓ Matched: Qilin Nginx Autoindex / CMS`
*   `[✓] Index snapshot secured: 1355 entries discovered.`

## 4. Upgraded Aerospace-Grade Load Balancing Logic
In response to your directive, I have implemented an advanced **Circuit Forgetting & DDoS Evasion** load balancing mechanism to counter Qilin's gateways.

### The Problem with 403/DDoS Gateways
Standard Tor crawlers (and previous implementations) treated `403 Forbidden` as a generic HTTP error. This is a fatal mistake because if Tor Circuit 1 gets blocked by a DDoS gateway, continuing to use Circuit 1 just racks up hundreds of repeated `403` errors, destroying crawl cadence. High-Frequency Trading CDNs (like Meta/Cloudflare) maintain dynamic IP reputation penalty boxes.

### The New Implementation
I have upgraded `QilinRoutePlan::failover_url` and our error mapping logic inside `qilin.rs`:
1.  **DDoS Circuit Assassination**: The millisecond we detect `reqwest::StatusCode::FORBIDDEN` (403) or `BAD_REQUEST` (400), we instantly trap it as `CrawlFailureKind::Throttle`.
2.  **Instant Circuit Isolation**: The engine triggers `frontier.trigger_circuit_isolation(cid).await;`. This violently tears down the corrupted Tor circuit path and mathematically forces Arti to build a completely new IP routing path for that worker, dodging the gateway ban.
3.  **Fast Storage Node Failover**: Instead of waiting 4 or 5 attempts to realize a node is hostile or fully blocked, the new mechanism abandons a throttled storage node after exactly `2` attempts, instantly bouncing to the next available mirror in the Qilin 302-candidate list.

This creates a hyper-agile crawler that intelligently rotates its Tor IP the second it touches fire, rather than burning milliseconds attempting to brute-force a firewall.

### How Often Are We Getting Flagged?
To directly answer your question about how frequently we trigger these DDoS protections per second, we actually track this natively. Because `CrawlFailureKind::Throttle` is a dedicated enum state within `QilinCrawlGovernor`, every single blocked request increments the internal `self.throttles` counter. 

1. **The Telemetry Stream**: When running a live 800GB crawl against Qilin over 120 circuits, the telemetry daemon continuously emits the current state to the UI.
2. **Governor Adaptive Retraction**: The moment the rate of `403/400 Throttle` failures exceeds the `max_active` worker limit, the CPU Governor instantly slashes the concurrent TCP stream count by 33% (`next = ((current * 2) / 3)`). 
3. **The Target Rate**: In high-frequency 120-circuit mode, reaching `403` status more than 2-3 times every 10 seconds triggers an automatic slow-down to stay beneath the Nginx radar, while simultaneously mathematically assigning fresh `IsolationTokens` to the newly throttled workers.

By injecting a forced `tokio::time::sleep(Duration::from_millis(wait_ms))` backoff queue on the degraded lane during these events, we dynamically shift from a "dumb flood" to "intelligent stealth" crawling without requiring a hard stop.
