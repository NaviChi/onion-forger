# Windows Architectural Performance & Infrastructure Limits

## Executive Summary
You observed that the crawler successfully ran on Windows using 4 native Arti client slots but was much slower than macOS. This document outlines the architectural boundaries of the Windows OS network stack, how the crawler's resources interact with it, and precise "Prevention Rules" to safely scale performance without triggering catastrophic kernel-level connection drops.

## 1. Technological Blockers & Framework Boundaries

### 1.1 The Ephemeral Port Exhaustion (MaxUserPort)
**The Issue:** On Windows, the TCP/IP stack natively limits outbound, short-lived connections (ephemeral ports) to around 16,384 concurrent sockets (ports 49152 to 65535 by default). 
When the crawler aggressively rips a target via Tor, hidden-service retries and any remaining compatibility SOCKS traffic can still churn through outbound sockets. The direct `ArtiClient` hot path removed the main Rust loopback proxy overhead, but Windows can still hit the `TIME_WAIT` threshold if client fanout and retry pressure are pushed too far.
**The Blocker:** When a socket closes, Windows holds it in a `TIME_WAIT` state for 120 seconds (the default `TcpTimedWaitDelay`). If the crawler burns through 16,000 requests in 2 minutes, Windows will completely block all new network traffic (affecting the entire PC) until those ports clear.

### 1.2 Native Arti Client Pool Overhead
**The Issue:** Native Arti removed the old `tor.exe` process-per-daemon tax, and the Rust hot path now bypasses loopback SOCKS. However, every additional client slot still consumes memory, bootstrap bandwidth, and circuit-management overhead on Windows. 
**The Blocker:** If we set the client count to an extreme number (e.g., 50), the machine stops benefiting from additional isolation and starts burning CPU and local ports on connection management, paradoxically *slowing down* the download speed.

### 1.3 Mechanical HDD IOPS Constraint
**The Issue:** The `aria_downloader.rs` engine utilizes Zero-Copy Memory Mapping (`mmap`) to bypass the OS file lock and write TCP packets directly to RAM, allowing the SSD to sync at maximum speed.
**The Blocker:** On systems with only 4GB of RAM or Mechanical Hard Drives (HDD), allocating massive sparse files into memory causes OS-level Page Faults. The mechanical slider inside the HDD tries to physically write scattered Tor packet chunks out of order, flatlining disk activity to 100% and completely stalling the crawler.

## 2. Infrastructure Limits & Proof of Concept Scaling

Currently, your crawler is configured to do the following:
*   **Mac Default:** 12 managed clients
*   **Windows Default:** 8 managed clients (as defined in `src/lib.rs`)
*   **Your Manual Run:** 4 managed clients (very safe, but heavily bottlenecks the 120 downloader-circuit limit, slicing it to roughly 30 circuits per client).

### How to Make it Faster on Windows (Safely)

To maximize speed on Windows without crashing the TCP stack, we must optimize the **Ratio of Workers to Daemons**:

1.  **Increase Managed Clients to 12 or 16:** Right now, with 4 client slots and a large download swarm, each client can end up handling too much rendezvous and circuit rebuild pressure. By bumping the GUI slider to **12 or 16**, you distribute the load so each live Arti slot handles materially fewer streams.
2.  **Do Not Hand-Edit Adapter Worker Constants:** Qilin and the other metadata adapters are now governed by runtime sizing logic. If Windows tuning is needed, prefer `CRAWLI_LISTING_WORKERS_MAX` / `CRAWLI_LISTING_WORKERS_DOWNLOAD_MAX` and validate the result, rather than patching arbitrary `max_concurrent` literals in adapter files.

## 3. Prevention Rules (Architectural Mandates)

**BEFORE modifying the crawler to push past these limits, adhere to the following:**

*   **Prevention Rule #1 (The 16-Client Ceiling):** Do NOT exceed 16-24 native Arti client slots on Windows by default. Beyond that point, the likely bottleneck becomes local port churn and scheduler overhead, not target throughput.
*   **Prevention Rule #2 (Kernel Bypass for TIME_WAIT):** If the crawler exceeds 10,000 files/minute, the standard OS networking stack will fail. Before increasing the downloader circuit limit past 150, we must implement a script to dynamically patch the Windows Registry (`HKLM\SYSTEM\CurrentControlSet\Services\Tcpip\Parameters`), lowering `TcpTimedWaitDelay` from 120 seconds to 30 seconds, and raising `MaxUserPort` to 65534.
*   **Prevention Rule #3 (Socket Keep-Alive):** Ensure that future Web/API scraping logic reuses persistent client state and avoids needless short-lived compatibility-proxy connections. Repeated connection teardown/rebuild remains a leading cause of memory/port exhaustion on Windows.
*   **Prevention Rule #4 (Sequential Write Fallbacks):** When writing download engines, you MUST explicitly catch `mmap` out-of-bounds or allocation failures. If the target OS is running on an HDD, ensure the `WriteMsg` loop falls back to `file.seek` and `file.write_all` to respect mechanical physical track movements instead of thrashing the disk head. 
