# AI Readme: Onion Forger / Crawli

This document contains everything an AI needs to understand, recreate, and modify the core logic of the system.

## Project Structure
- **/crawli/src-tauri**: The core backend written in Rust.
- **/crawli/src**: The React/Vite/Tailwind frontend.

## The Qilin Crawler (Tor Native)
Qilin is an advanced indexing and download engine designed specifically for obfuscated onion targets.

### Key AI Context Files:
1. `src-tauri/src/tor_native.rs`: Contains the `ArtiSwarm` native Tor engine. Replaces external `tor.exe` with a smart Rust implementation. Handles health probing, jittered bootstrap delays, and Kalman filter latency checks.
2. `src-tauri/src/arti_client.rs`: Custom Hyper wrapper binding directly to Tor circuits to force HTTP/2 connection pooling.
3. `src-tauri/src/target_state.rs`: The determinism engine. All crawled targets are mapped mathematically via `derive_target_identity(url)` into `targets/<target_key>` isolated structures.
4. `src-tauri/src/speculative_prefetch.rs`: Background `HEAD` pre-warming of child directories to maintain high connection reuse over HTTP/2.
5. `src-tauri/src/index_generator.rs`: HTML aesthetic generator to present `listing_canonical.json` files as a sleek tree view.

## Frontend Vibe & Aesthetics
The frontend operates on a "Tactical Military-Grade / Aerospace" merged with "Dark Mode Ghibli" aesthetic.
- Color schema is deeply grounded in Zinc palettes (Zinc-900 `var(--bg)` and Zinc-800 surfaces).
- Accents use Tactical Green (e.g., `#22c55e`).
- Font stacks lean heavily into `Space Mono` for IDs and `Inter` for interfaces.

*To be continued/updated during frontend implementation cycles...*
