# OnionForge Hardware & OS Support Matrix

This document defines the official support targets for the OnionForge crawler, spanning Operating Systems, Storage Mediums, Memory configurations, and Network constraints. It also provides definitive **Implementation Guidelines** that any AI or human developer must follow to ensure the application remains perfectly stable across *all* of these environments simultaneously.

---

## 1. Supported Operating Systems

OnionForge is a cross-platform Tauri/Rust application. Code must not assume UNIX-only or Windows-only behaviors without explicit feature gates (`#[cfg(target_os = "...")]`).

| OS | Status | Known Architectural Blockers |
| :--- | :--- | :--- |
| **macOS (Darwin)** | Fully Supported | UNIX Sockets handle Tor multiplexing flawlessly. Apple Silicon (M1/M2/M3) memory bandwidth effectively masks poor IO logic. |
| **Windows 10/11** | Fully Supported | TCP/IP Ephemeral Port Exhaustion (Max 16k ports). Native Arti plus the direct `ArtiClient` hot path removes most old loopback SOCKS churn, but compatibility SOCKS consumers and aggressive hidden-service retry storms can still saturate `TIME_WAIT` and local ports if fanout is uncapped. |
| **Linux** | Fully Supported | Extremely stable, but requires AppImage/Debian GUI fallback states if running headlessly or on X11 vs Wayland. |

---

## 2. Storage & Disk IO Support

To support everything from 15-year-old laptops to enterprise servers, the crawler's filesystem logic must be completely agnostic to disk write speeds.

| Storage Medium | Classification | Required Implementation Strategy |
| :--- | :--- | :--- |
| **NVMe Gen 4/5** | Extreme Performance | Can handle instant 120-circuit Zero-Copy Memory Mapping (`mmap`). |
| **SATA SSD** | High Performance | Can leverage `mmap`, but may exhibit slight wear-leveling latency on massive sparse file allocations. |
| **Mechanical HDD (7200/5400 RPM)** | High Risk / Supported | **CRITICAL CONSTRAINT:** HDDs have a physical spinning platter and a read/write needle. They *cannot* do random access writes efficiently. If an HDD is blasted with 100 out-of-order 5MB Tor chunks, the needle thrashes, disk IO hits 100%, and the OS stalls. |
| **External USB 2.0 / 3.0 Drives** | High Risk / Supported | Prone to USB bus disconnects on heavy load. |

---

## 3. Memory (RAM) Profiles

Rust is memory safe, but Memory-Mapped files (`mmap`) bypass the heap and map directly to virtual address spaces.

| RAM | Expected Behavior | Required Implementation Strategy |
| :--- | :--- | :--- |
| **32GB - 64GB+** | Optimal | Massive files (50GB+) can be safely mapped to memory. |
| **8GB - 16GB** | Standard | The OS Page File will actively swap if too many active downloads are held in memory. |
| **4GB (Low End)** | Constrained / Supported | Out-of-Memory (OOM) killer will trigger if `mmap` allocations fail. The crawler *must* gracefully catch allocation errors and fall back to sequential byte-streaming. |

---

## 4. Network & Bandwidth Constraints

Crawling `.onion` domains introduces extreme latency variations. The engine must actively scale its aggression based on the connection.

| Network State | Constraint | Required Implementation Strategy |
| :--- | :--- | :--- |
| **High Bandwidth (Clearnet / Fiber)** | Fast | Capable of 300+ downloader workers. Bottleneck moves from Network to Disk IO. |
| **Tor Network (Standard)** | Medium / Erratic | Expect random circuit deaths. Thompson Sampling / Multi-Armed Bandit algorithms must be used to dynamically score and drop slow Tor nodes. |
| **Slow Bandwidth (Target Throttling)** | High Risk | **CRITICAL CONSTRAINT:** Targets like Qilin will actively rate-limit or tarpit connections. If the network drops below 50kbps, brute-force workers will stack up and exhaust local ports. |

---

## 5. Global Architectural Directives (How to Prevent Regressions)

To ensure we never repeat the "Mechanical HDD Stall" or the "Windows CPU Port Exhaustion" issues, all future code commits must adhere to these directives:

### Directive A: Adaptive I/O is Mandatory
Never assume the target has an SSD. Downloader loops must attempt `mmap` for speed, but *must* provide a standard `file.seek` and `file.write_all` fallback block. If `mmap` fails, or if out-of-bounds writes occur, the engine must queue bytes internally and flush them sequentially to respect HDD mechanical track constraints.

### Directive B: The 20-to-1 Circuit Ratio Concept
Concurrent downloading is only as fast as the multiplexer allows. 
- You should *never* assign 300 circuits to 4 or 6 managed Arti client slots (a 50:1 or 75:1 ratio). That forces too many isolated streams and circuit rebuilds onto a small client pool, increasing latency variance and slowing recovery on Windows.
- **The Golden Ratio** is between **10:1 and 20:1**. 
    - e.g., 6 managed clients = ~120 circuits.
    - e.g., 12 managed clients = ~240 circuits.
By defaulting to 120 circuits and 6 managed clients, each live Arti slot comfortably routes about 20 files, ensuring high bandwidth without burning down the OS kernel.

### Directive C: Stream, Don't Load (String Parsing)
When parsing JSON or HTML from a target, never load massive strings into RAM (`std::fs::read_to_string`). Use asynchronous streaming readers (`reqwest::Stream`) whenever possible. This ensures computers running on 4GB of RAM won't segfault if QData outputs a 500MB `autoindex` XML document.

### Directive D: Embrace the "Ghost Browser" for Unstable Networks
Instead of firing 300 asynchronous HTML `GET` requests on high-latency Tor connections (which causes timeout overlaps), prefer spawning 1 headless Chromium instance. Let the browser handle the networking, and extract the rendered DOM natively once the Javascript completes. This completely sidesteps slow bandwidth timeouts.
