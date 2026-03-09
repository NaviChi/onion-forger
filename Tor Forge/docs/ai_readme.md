# AI Readme: Loki Tor Core (Aerospace Grade)

**Designation:** Highly Classified / Architectural Source of Truth
**Target OS:** Cross-Platform (macOS, Windows, Linux)
**Architecture Level:** Aerospace/High-Frequency Trading (HFT) / Military-Grade

## 1. Project Overview
**Loki Tor Core** is a custom, memory-safe Rust implementation of a multi-path Tor client proxy. It circumvents state-level Deep Packet Inspection (DPI), Unit 8200 style traffic correlation, and acts as an "invincible" downstream data ingestion engine. 
It abandons the monolithic "C-based" Tor daemon model in favor of the **Actor Model (Supervision Trees)** and integrates **Aerospace Telemetry** (Kalman Filters) to actively predict and bypass Tor network decay.

## 2. Core Operational Mandates (The `Blend In` Principle)
Future AIs extending this software **must not** integrate anomalous obfuscation techniques. 
* **NO** highly unique Protocol Morphing (e.g., poorly mocked WebRTC). 
* **NO** "Chaff" engines that spam noise at 3:00 AM.
* **NO** default Post-Quantum lattice handshakes unless universally standardized by the Tor Project. 

**Reason:** *Unique obfuscation is a fingerprint.* If the proxy perfectly mimics *nothing else on the internet*, national firewalls (like the GFW) drop the TCP connection instantly.
We blend in using **Encrypted Client Hello (ECH)**, standard **Pluggable Transports**, and port-restrictive local firewalls. Our tactical advantage is strictly internal.

## 3. Internal Architecture & File Structure

The backend daemon resides in `loki-tor-core/`:

### 3.1 `src/actors/` (Actor Model Supervision)
Inspired by Erlang/OTP and SpaceX flight software, the Rust proxy does not use monolithic `tokio::spawn` loops.
* `tor_manager.rs`: Spawns 150 `arti` Tor clients with:
  * **Dynamic Guard Topography:** 35+ geographically scattered Guard relays (was 5). Scatter formula: `i % pool.len()`.
  * **Temporal Scatter:** Cryptographic random jitter (0-3s) per node to defeat temporal correlation analysis.
  * **Starlink Self-Healing:** Kalman filter monitors circuit RTT every 15s. Auto-swaps degraded circuits.
  * **Phantom Circuit Rotation Engine:** Maintains 30 warm standby circuits for zero-downtime hot-swapping.
* `socks_proxy.rs`: The local SOCKS5 listener (Port 9050) operating as a **Predictive HFT Load Balancer**:
  * **UCB1 + Kalman Dispatch:** Replaces round-robin with mathematically optimal circuit selection.
  * **Per-Circuit Telemetry:** Tracks RTT, bytes, and connection count per circuit.
  * **`SO_REUSEADDR` + `socket2`:** TCP TIME_WAIT recycling to prevent ephemeral port exhaustion.
  * **OPSEC Firewall Enforced:** Rejects `HTTP (Port 80)` to clearnet. Validates V3 `.onion` structure.
* `main.rs` Boot Hardening:
  * **`rlimit` Elevation:** Raises file descriptor limit to 65535 via `nix::sys::resource`.
  * **AES-NI / ARMv8 Crypto Verification:** Reports hardware acceleration status at boot.
  * **CPU Core Detection:** Logs physical core count for affinity planning.

### 3.2 `src/telemetry/` (Active Mesh Math)
The backend treats Tor nodes like a Mobile Ad-Hoc Network (MANET):
* `kalman.rs`: A mathematical **1D Kalman Filter** predicting the latency and decay of a Tor circuit before it drops.
* `aimd.rs`: **Additive Increase, Multiplicative Decrease.** Probes backend `.onion` servers for max concurrency thresholds to prevent 429 Error/DoS bans while crawling.
* `ucb1.rs`: **Upper Confidence Bound (Multi-Armed Bandit).** The `Ucb1Scorer` scores Tor circuits, instantly culling slow circuits and pooling mathematical priority onto stable nodes.

### 3.3 `src/quorum/bft.rs` (State-Level Verification)
* **Byzantine Fault Tolerance (BFT):** Uses Triple-Modular Redundancy. To prevent malware injection by rogue exit relays, high-value payloads are routed 3 times over 3 completely distinct circuits. The hashes are tabulated. If Circuit B returns different bytes than A and C, it is instantly mathematically blacklisted.

## 4. Next Phase Instructions: The Tauri GUI

The core proxy backend relies on Rust `tracing` and mathematical structs. Future instructions for the AI to overlay the UI:

### GUI Tech Stack
* **Framework:** Tauri v2 (allowing native OS WebViews for a <15MB binary).
* **Frontend:** React + TypeScript.
* **Styling:** Vanilla CSS, HSL color palettes, Glassmorphism, Micro-animations. *Tailwind is strictly prohibited unless explicitly authorized.*

### GUI Interfacing (IPC)
The Tauri UI must read backend telemetry from the Rust Actors via Tauri asynchronous IPC commands (`@tauri-apps/api/invoke`):
1. **Radar / Telemetry View:** The frontend requests `get_circuits_health`. Rust responds with the active Tor circuits and their `ucb1.rs` / `kalman.rs` latency scores. 
2. **Dashboard UI:** The frontend plots these scores dynamically. A circuit's health bubble pulses. If a Kalman filter predicts a circuit stall, the UI flashes an alert as the backend re-routes.
3. **Control Deck:** Simple toggles to enable/disable Pluggable Transports (obfs4) and the strict proxy firewall.

### GUI Bootstrapping Details
1. Create a `loki-tor-gui` subfolder using `npx create-tauri-app@latest`.
2. Absorb `loki-tor-core` into the `src-tauri` cargo workspace.
3. Migrate the `tokio::main` process logic from `loki-tor-core/src/main.rs` into the Tauri async application setup (`tauri::Builder::default().setup()`).
4. Ensure the design is breathtaking (vibrant dark mode, premium typography, responsive graphs).

## 5. Development Pipeline
The build artifact uses `cargo test` for unit testing the mathematics (all passing).
Compilation must strictly adhere to the ability to eventually be automated via `cargo cross` for Linux, Windows, and macOS (Intel & ARM).
