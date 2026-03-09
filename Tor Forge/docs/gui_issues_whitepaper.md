# GitHub: GUI Issues & Prevention Rules Whitepaper

## Overview
This document catalogs issues specific to the `loki-tor-gui` Tauri integration and provides strict prevention rules to maintain the high-end aesthetic ("Vibe Architecture") and memory safety.

## Issue 1: DOM Bloat from Circuit Telemetry Tracking
**Predicted Issue:** As `loki-tor-core` creates and destroys hundreds of circuits based on Kalman filter predictions, pushing raw telemetry logs directly to the React DOM will cause massive Memory Leaks and JavaScript thread blocking.
**Root Cause:** Rendering 10,000+ un-virtualized DOM elements in a Chromium WebView will crash the UI.

### Prevention Rule 1: Windowed Telemetry
* **Action:** Any React component displaying Tor circuit health or logs must use virtualization (e.g., `react-window` or `react-virtuoso`). Only render the exact items currently visible in the viewport.

## Issue 2: Framework Integration Conflicts
**Predicted Issue:** Tauri V2 beta methods differ from V1; relying on outdated web queries for IPC (Inter-Process Communication) leads to silent frontend failures.
**Prevention Rule 2: Enforce V2 Strong Typing:**
* **Action:** Always use `@tauri-apps/api/core` for invoking commands. Do not use deprecated `window.__TAURI__` injections.

## Issue 3: Aesthetic Degradation
**Prevention Rule 3: Military-Grade Theming Standards:**
* **Action:** Do not use Tailwind CSS unless explicitly required in a sub-module. Enforce Vanilla CSS with CSS Variables for themes. Use HSL definitions for dynamic dark modes (e.g., deep space blacks, aggressive neon accents for active Tor circuits). Ensure the GUI matches Cloud Imperium Games / Star Citizen UI standards (glassmorphism, clean fonts, sharp edges).
