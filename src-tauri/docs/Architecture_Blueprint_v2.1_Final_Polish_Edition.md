# Crawli v2.1 Final Polish Recommendations
**Last Updated**: 2026-03-07  
**Status**: Ready for implementation (one-file-at-a-time loop)

## 1. Output Directory Structure (Already Recommended)
```
<output-root>/
├── targets/<target_key>/
│   ├── current/          ← latest crawl (listing + metadata)
│   ├── best/             ← highest-yield crawl ever
│   ├── crawl_history/    ← timestamped snapshots
│   └── download_failures.json
├── downloads/<target_key>/   ← real mirrored files
├── temp_onionforge_forger/   ← logs, sled, telemetry
└── index.html                ← beautiful summary
```

## 2. Adapters & Extensibility
- Implement Adapter Pipeline Trait + WASM plugin system (new adapters without recompiling).

## 3. Crawling Engine Upgrades
- Speculative directory pre-fetch (Tesla Dojo pipeline) + HTTP/2 multiplexing per circuit.
- Expected: 60–85 files/s on current VM.

## 4. Downloading Engine Upgrades
- Dedicated small-file swarm after crawl finishes (SRPT + separate governor budget).

## 5. UI / Visuals Upgrades
- Collapsible VFS tree view in Dashboard.
- One-click export buttons (Open folder, Windows DIR /S, JSON).
- Live bad-subtree heatmap overlay.
- Session summary card (Matched/Exceeded/Degraded + total size).

## 6. System Polish
- Auto-generate index.html per target.
- Make binary telemetry opt-in with dashboard toggle.
- “Health Report” button that runs adapter_test CLI.

## Next Implementation Order (One-File Loop)
1. speculative_prefetch.rs + HTTP/2 wiring  
2. VfsTreeView.tsx + export buttons  
3. Dedicated small-file swarm  
4. Adapter Pipeline Trait (long-term)

All changes respect the existing prevention rules, resource governor, and 4 GB HDD VM constraints. No Ghost Browser fallback.

**Prevention Rules Reminder** (add these):
- PR-PREFETCH-001 – Cap prefetch at 3 children; spread across clients.
- PR-UI-001 – Every new dashboard surface must have data-testid for overlay integrity tests.
