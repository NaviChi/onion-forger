# GitHub: Backend Issues & Prevention Rules Whitepaper

## Overview
This document catalogs encountered issues within the `loki-tor-core` Rust backend implementation and prescribes strict architectural prevention rules. Cross-reference this document before modifying the Tor pipeline or actor models.

## Issue 1: Arti Onion V3 Address Parsing & Service Availability
**Encountered Issue:** During load testing, attempting to route proxy traffic to specific V3 Onion addresses (e.g., `duckduckgogg42xjoc72x3sjiqbvqdz2x5bgcocnmq3jc6pxndbd.onion`) resulted in parsing errors (`tor: .onion address was invalid`) or immediate closure (`SOCKS connection failed`). Furthermore, secondary onion addresses timed out with `Onion Service not found`.
**Root Cause:**
1. The integration of `arti-client` (v0.23.0) enforces extremely strict validation on `.onion` URL strings. Malformed characters or unsupported V3 feature flags can trigger an invalid address panic.
2. Dark web nodes are inherently volatile; assuming an `.onion` service will be highly available leads to connection drops that the proxy currently ungracefully panics on or drops.

### Prevention Rule 1: Graceful Onion Degradation & Validation
* **Action:** Before feeding a SOCKS request into the Arti client's connection pipeline, structurally validate the `.onion` hostname using a dedicated regex/length verify function.
* **Action:** Implement a retry-with-backoff mechanism for the SOCKS proxy listener. If `Onion Service not found` is triggered, the actor should attempt to rebuild the circuit through different relays before failing the user's Python/Playwright request.
* **Advanced Architecture Integration (Conflux & Pooling):** To fully mitigate service drops and latency spikes inherent to V3 onion services, the `TorManager` must be upgraded to natively support **Conflux** (multipath onion routing) and **Pre-emptive Circuit Pooling**. 
    * By splitting traffic dynamically across multiple overlay paths based on measured latency (Conflux), we can bypass failing or congested middle relays. 
    * The daemon must maintain a "warm pool" of spare, pre-built circuits to ensure immediate fallback without waiting for a new TLS handshake when an onion node goes down.

## Issue 2: Cold-Boot Bootstrap Latency
**Encountered Issue:** Running `cargo run` without an existing `.loki_tor_state` forces the daemon to download over 1,300 microdescriptors, causing a 15-25 second delay before the SOCKS proxy actually multiplexes.
**Prevention Rule 2: State Persistence:**
* **Action:** Never launch the Tor client entirely in-memory. Ensure `arti_client::config::CfgPath` always maps to a persistent local cache on disk.

## Issue 3: Missing Security Enhancements for Onion Proxy
**Encountered Issue:** The SOCKS proxy routes plaintext HTTP (Port 80) traffic. While there is a warning, it allows the traffic.
**Prevention Rule 3: Strict HTTPS Filtering:**
* **Action:** If the target is a `.onion` site, Port 80 is acceptable (as the connection is end-to-end encrypted). However, if routing clearnet traffic out of an exit node, Port 80 must be dropped to prevent malicious exit nodes from packet-sniffing or SSL-stripping.

## Issue 4: TLS Layer 7 Corruptibility (Aria2 HTTP Range Interception)
**Encountered Issue:** Attempting to build an `Aria2Engine` directly into the `SOCKS5` bidirectional stream to intercept `Accept-Ranges: bytes` traffic caused fatal `[SSL: UNEXPECTED_EOF_WHILE_READING]` errors on the client.
**Root Cause:** Modern applications exclusively use HTTPS (Port 443). The HTTP headers (including the URL and Range requests) are completely encrypted inside the TLS payload. If the SOCKS proxy attempts to buffer or inject bytes before the TLS handshake completes, it irrevocably corrupts the cipher state.
**Prevention Rule 4: Client-Side Multipath Delegation:**
* **Action:** Do **NOT** attempt to parse Layer 7 HTTP connections inside the Layer 5 SOCKS proxy if the target port is 443 (TLS). 
* **Architecture Shift:** True Multipath Tor speeds (splitting a 10MB file into 10 concurrent Tor circuits) must be initiated *by the end-user client Application* (e.g., the Tauri GUI or Python scraper) which handles the TLS termination natively. The `loki-tor-core` must remain a passive multiplexing pipeline that blindly accepts these 10 concurrent streams and routes them efficiently.

## Issue 5: Tor Directory Authority Rate Limiting ("Thundering Herd")
**Encountered Issue:** When spawning 150 local Tor instances simultaneously, the network continuously returned `Unable to select a guard relay: No usable fallbacks`. 
**Root Cause:** Initiating 150 concurrent identical requests to the default list of 5 hardcoded Fallback Directories from a single IP Address inherently triggers Anti-DDoS restrictions on the Guard nodes, causing them to forcibly reject TCP requests.
**Prevention Rule 5: Drone Scatter Algorithm (Dynamic Geographic Fan-Out):**
* **Action:** Always structurally shard Tor client Fallback Directory and Guard node assignments. Use the `scatter_seed = i % N` methodology to assign unique `rsa_identity` and `ed_identity` IPs to each chunk of nodes. This effectively scatters the connection patterns to physically distinct hardware across the globe, inherently bypassing the geographic volumetric rate limiting.

## Issue 6: Zero-Downtime Daemon Initialization
**Encountered Issue:** Creating 150 distinct Tor configurations and 300 SQLite database caches (for telemetry caching) synchronously forced the SOCKS proxy's `127.0.0.1:9050` listener to stall for 45+ seconds. Client scripts (such as `test_multipath.py`) crashed purely because the SOCKS daemon had not yet bound to the port.
**Root Cause:** The `fs::create_dir_all` and SQLite setup on disk are heavily I/O bound. Executing them within the `Actor::pre_start` Tokio thread completely blocks process initialization.
**Prevention Rule 6: Asynchronous Proxy Instantiation:**
* **Action:** Decouple proxy readiness from Tor readiness. Always bind the `SocksProxy` to `9050` immediately, regardless of background circuit completion. `SocksProxy` should boot with `0` backend circuits.
* **Action:** Send the heavy SQLite parsing task into a detached `tokio::spawn(async move { tokio::task::spawn_blocking(...) })` wrapper within `TorManager`. Have the `SocksProxy` periodically poll the `TorManager` (or receive a push message) when the clients are ready, updating its state behind an `RwLock`. If connection requests hit the SOCKS LB while `circuits.is_empty()`, intelligently defer or `Connection Reset` the traffic until circuits are injected.
