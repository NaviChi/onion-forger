# OnionForge: AI Engineering & Context Reference
> **Last Updated:** 2026-03-05
> **Version:** 2.0.0
> **Authors:** Navi (User), Antigravity (AI)

This document serves as the master blueprint for any AI agent tasked with maintaining, extending, or recreating the OnionForge intelligence gathering application. It contains all critical architectural decisions, environment constraints, GUI styling instructions, and API behavioral knowledge required to build this system from scratch without guessing.

---

## 1. System Identity and Objectives
OnionForge (codenamed `crawli`) is a cross-platform Tauri (Rust + React/TypeScript) desktop application. Its primary function is to securely bootstrap an embedded Tor daemon swarm, route high-concurrency HTTP/S requests through isolated SOCKS5 proxies using TLS fingerprint spoofing, and systematically export massive deep-web `.onion` directories (specifically targeting complex single-page apps like Qilin and Play Ransomware) via `aria2` protocol scaffolding.

*   **Primary Constraint:** The system must run flawlessly on Mac, Linux, and Windows 10/11.
*   **Secondary Constraint:** The system must gracefully degrade and protect OS resources (e.g., Ephemeral Port Exhaustion, Mechanical HDD IOPS lockouts, RAM limitations).
*   **Visual Identity:** The React UI uses a hyper-modern "Cyber/Military" dark-mode aesthetic. 

---

## 2. Core Architecture Stack

### Backend Container (Tauri/Rust)
*   **Framework:** Tauri v2 configured with `tauri.conf.json`. 
*   **Asynchronous Engine:** `tokio` multi-threaded runtime (`tokio::spawn`, `Arc<Mutex>`).
*   **Networking:** `reqwest` initialized inside Tor environments using `Socks5Stream` bindings via `tokio-socks`.
*   **Data Scaffolding Engine:** 
    *   For HTML/API Parsing: High-concurrency `crossbeam-queue` URL frontiers.
    *   For Downloading: Out-of-process RPC to a dynamically spawned `aria2c` multi-connection daemon.
*   **In-Memory DB:** `sled` embedded KV store for recording Visited URLs and VFS arrays securely.

### Frontend Container (React/TypeScript)
*   **Build Tool:** Vite.
*   **Styling:** Pure CSS (`App.css`, `Dashboard.css`). NO Tailwind. Explicit emphasis on glassmorphism, glowing accents (`box-shadow`), dark grays (`#0a0a0a`), neon cyan/purple combinations (`#00e5ff`/`#8b5cf6`), and monospace system fonts (`JetBrains Mono`).
*   **State Management:** Standard React hooks (`useState`, `useEffect`) layered with Tauri IPC event listeners (`listen<T>`).
*   **Components:** Modularized structure (e.g., `VFSExplorer.tsx`, `Dashboard.tsx`). 

---

## 3. The 4-Pillar Pipeline Strategy
To recreate or modify this app, you must understand how data traverses the 4 pillars of the crawler:

#### Pillar 1: Bootstrapping & The Swarm (`tor.rs`)
The host OS spawns exactly `N` lightweight, headless `tor` proxy daemons (e.g., ports 9051 to 9056). Each daemon runs from a dynamically generated, isolated `torrc` config in the `temp_onionforge_forger` output directory. 
*   **Prevention Rule:** Never assign more than 20 simultaneous `aria2` download circuits to a single Tor daemon to prevent Windows CPU context-switching crashes. 

#### Pillar 2: The Frontier Scanner (`frontier.rs` & `adapters/`)
The user inputs a `.onion` URL. The `AdapterRegistry` hits the endpoint to read the HTTP Header and HTML Body (the `SiteFingerprint`). It matches this fingerprint to a specialized adapter (e.g., `qilin.rs`, `play.rs`, `autoindex.rs`).
The Adapter utilizes `tokio` workers to crawl the site, extracting `FileEntry` objects.
*   **Prevention Rule:** Some nodes capitalize protocols (`HTTP://`). ALWAYS use `.to_lowercase()` when parsing URLs so the router doesn't accidentally discard safe links.

#### Pillar 3: The Virtual File System (`vfs.rs` & `VFSExplorer.tsx`)
Extracted paths are blasted over Tauri IPC to the React frontend, where they are mapped onto a TanStack generic virtualizer (`useVirtualizer`) to ensure the DOM doesn't lock up when 50,000 files are rendered.
*   **Prevention Rule:** Do not trust size headers perfectly. UI progress bars must rely on definitive `0 bps` backend completion signals instead of assuming a file is 100% finished just because bytes stream in without a `Content-Length`.

#### Pillar 4: The Storage Scaffolder (`aria_downloader.rs`)
When the user clicks "Download", the Rust backend intercepts the file array. It generates 0-byte structural placeholders for folders. For physical files, it passes the URLs using an RPC XML payload to a background `aria2c` process.
*   **Prevention Rule:** The backend MUST fallback to sequential byte writes (instead of zero-copy `mmap`) if memory mapping fails. This protects users with 5400 RPM Mechanical HDDs from 100% disk usage lockouts.

---

## 4. UI / UX Design Guidelines

If generating new frontend React code, strictly adhere to these rules:

1.  **Colors:** 
    *   Primary Background: `#0f1014`
    *   Panel Background: `rgba(20, 22, 28, 0.7)` with `backdrop-filter: blur(12px)`
    *   Accent Primary: `#a200ff` (Deep Purple)
    *   Accent Secondary: `#00e5ff` (Neon Cyan)
    *   Text: `#e2e8f0` (Main), `#94a3b8` (Muted)
2.  **Typography:** Use sans-serif for UI elements (`Inter`, `system-ui`) and heavily utilize `JetBrains Mono` for ALL numbers, logs, paths, and statuses.
3.  **Components:** Use `lucide-react` for iconography. Build custom animated loaders (e.g., `VibeLoader.tsx`), relying on `@keyframes` rather than static SVGs for scanning indiciators.
4.  **No Placeholders:** If you must simulate data in the UI without a backend, construct a static fixture file (like `vfsFixture.ts`) and inject it gracefully. Do not write generic "Hello World" placeholder blocks. Everything must look premium and dense.

---

## 5. Development Rituals & Edge Cases

When editing Rust logic, remember these historical constraints:
*   **Windows Process Limits:** Windows has a strict TCP `MaxUserPort` limit (~16,000). Uncapped HTTP requests will blue-screen the network adapter. Keep async workers clamped (e.g., max 60 workers per adapter).
*   **Tor TLS Fingerprinting:** Cloudflare and Nginx will drop connections if the `reqwest` TLS client acts like a bot. You must bind `rustls` instead of `native-tls` inside the `cargo` build and mimic standard browser headers.
*   **Deadlocks:** You must release `Mutex` guards *before* triggering `await` in `tokio`, otherwise the async runtime thread will permanently freeze waiting for data chunks.
*   **Adaptive JWT Iframe Parsing (DragonForce):** Do not attempt bare-metal API calls against Deepweb architectures protected by tokenized Next.js wrappers. Use standard `scraper::Selector` tools to capture the `<iframe>` bridging URLs and inject them back into `CrawlerFrontier`. This offloads authentication logic back to Tor.
*   **Adapter Polyfill Delegation (Qilin):** When encountering ransomware sites utilizing custom CSS template frameworks ("QData") masking standard HTML tables, create an adapter isolated purely to the fingerprint detection step (e.g. `body.contains("QData")`). Do not build a custom scraper. In `crawl()`, delegate execution immediately back to the master `<AutoindexAdapter as CrawlerAdapter>::crawl` generic framework. Every custom scraper logic tree requires rigorous unit testing boundaries, avoid code sprawl.

By internalizing this document, you possess the context necessary to forge new adapters and structural improvements without compromising the system's foundational stability.
