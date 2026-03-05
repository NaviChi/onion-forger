# Windows Architectural Performance & Infrastructure Limits

## Executive Summary
You observed that the crawler successfully ran on Windows using 4 Tor daemons but was much slower than macOS. This document outlines the architectural boundaries of the Windows OS network stack, how the crawler's resources interact with it, and precise "Prevention Rules" to safely scale performance without triggering catastrophic kernel-level connection drops.

## 1. Technological Blockers & Framework Boundaries

### 1.1 The Ephemeral Port Exhaustion (MaxUserPort)
**The Issue:** On Windows, the TCP/IP stack natively limits outbound, short-lived connections (ephemeral ports) to around 16,384 concurrent sockets (ports 49152 to 65535 by default). 
When the crawler aggressively rips a target via Tor, `aria2` multiplexes up to 120 streams. If we scale up to 16-24 Tor daemons on Windows, we risk hitting the `TIME_WAIT` threshold. 
**The Blocker:** When a socket closes, Windows holds it in a `TIME_WAIT` state for 120 seconds (the default `TcpTimedWaitDelay`). If the crawler burns through 16,000 requests in 2 minutes, Windows will completely block all new network traffic (affecting the entire PC) until those ports clear.

### 1.2 Tor Bundle RAM & CPU Overhead
**The Issue:** The macOS Tor binary is highly optimized for UNIX socket handling. On Windows, each Tor daemon (running as a heavy `tor.exe` process) requires around 50MB of RAM and incurs higher Context Switching penalties on the CPU when multiplexing thousands of SOCKS5 proxies. 
**The Blocker:** If we set the daemon count to an extreme number (e.g., 50), the Windows CPU will spend more time context-switching between the Tor processes than actually decrypting incoming packets, paradoxically *slowing down* the download speed.

### 1.3 Mechanical HDD IOPS Constraint
**The Issue:** The `aria_downloader.rs` engine utilizes Zero-Copy Memory Mapping (`mmap`) to bypass the OS file lock and write TCP packets directly to RAM, allowing the SSD to sync at maximum speed.
**The Blocker:** On systems with only 4GB of RAM or Mechanical Hard Drives (HDD), allocating massive sparse files into memory causes OS-level Page Faults. The mechanical slider inside the HDD tries to physically write scattered Tor packet chunks out of order, flatlining disk activity to 100% and completely stalling the crawler.

## 2. Infrastructure Limits & Proof of Concept Scaling

Currently, your crawler is configured to do the following:
*   **Mac Default:** 12 Daemons
*   **Windows Default:** 8 Daemons (as defined in `src/lib.rs`)
*   **Your Manual Run:** 4 Daemons (very safe, but heavily bottlenecks the 120 `aria2` circuit limit, slicing it to just 30 circuits per daemon).

### How to Make it Faster on Windows (Safely)

To maximize speed on Windows without crashing the TCP stack, we must optimize the **Ratio of Workers to Daemons**:

1.  **Increase Daemons to 12 or 16:** Right now, with 4 daemons and 120 Aria2 circuits, each daemon is handling 30 simultaneous high-speed file streams. This is overloading the Tor node's internal state machine. By bumping your GUI slider to **12 or 16 daemons**, you distribute the load so each executable only handles ~7-10 streams. 
2.  **Increase Qilin Web Workers to 48:** In `adapters/qilin.rs`, the directory extraction `max_concurrent` is set to 24. For a desktop PC, you can safely bump this value to 48 or 64 to crawl the HTML map 200% faster.

## 3. Prevention Rules (Architectural Mandates)

**BEFORE modifying the crawler to push past these limits, adhere to the following:**

*   **Prevention Rule #1 (The 16-Daemon Ceiling):** Do NOT exceed 16-24 Tor daemons on Windows natively. If extreme scale (100+ daemons) is required, we MUST abandon native `tor.exe` and re-architect the backend to use a custom Rust-embedded Tor library (`arti-client`) to bypass OS process context switching, or run the crawler inside a WSL2 Linux subsystem for native UNIX socket performance.
*   **Prevention Rule #2 (Kernel Bypass for TIME_WAIT):** If the crawler exceeds 10,000 files/minute, the standard OS networking stack will fail. Before increasing the `aria2` circuit limit past 150, we must implement a script to dynamically patch the Windows Registry (`HKLM\SYSTEM\CurrentControlSet\Services\Tcpip\Parameters`), lowering `TcpTimedWaitDelay` from 120 seconds to 30 seconds, and raising `MaxUserPort` to 65534.
*   **Prevention Rule #3 (Socket Keep-Alive):** Ensure that any future Web/API scraping logic strictly uses HTTP `Keep-Alive`. Tearing down and rebuilding SOCKS5 TCP sockets for every requested file is the leading cause of memory/port exhaustion on Windows architectures. 
*   **Prevention Rule #4 (Sequential Write Fallbacks):** When writing download engines, you MUST explicitly catch `mmap` out-of-bounds or allocation failures. If the target OS is running on an HDD, ensure the `WriteMsg` loop falls back to `file.seek` and `file.write_all` to respect mechanical physical track movements instead of thrashing the disk head. 
