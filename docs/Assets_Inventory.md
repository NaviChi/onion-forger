Version: 1.0.0
Updated: 2026-03-03
Authors: Navi (User), Codex (GPT-5)
Related Rules: [MANDATORY-L1] Docs Management, [MANDATORY-L1] Living Documents

# Summary
Inventory of key assets used by the `crawli` desktop program and UI.

# Context
Asset tracking is required for reproducibility and licensing hygiene.

# Analysis
Current project assets fall into 4 groups:
- UI code assets (React/TS/CSS).
- Native assets (Tauri icons/config/capabilities).
- Test assets/fixtures (Playwright and local fixture data).
- External package assets (NPM/Cargo dependencies).

# Details
Primary inventory (current):
- `src-tauri/icons/*`
  - Usage: native app icon set across platforms.
  - License: project-owned/generated app branding assets (verify origin before redistribution).
- `src/components/*.tsx`, `src/*.css`
  - Usage: runtime UI rendering and styling.
  - License: project source.
- `src/fixtures/*`
  - Usage: local preview/testing fixture content.
  - License: internal test data.
- `tests/overlay_integrity_runner.cjs`, Playwright reports
  - Usage: UI integrity workflow automation.
  - License: project test scripts.
- Dependencies in `package.json` and `src-tauri/Cargo.toml`
  - Usage: runtime + build stack.
  - License: per-package OSS license; verify via lockfiles before release packaging.

Reuse policy:
- Reuse existing icons/styles/scripts when possible.
- Avoid adding duplicate media assets without inventory entry.

# Prevention Rules
**1. Every new binary/media/test fixture asset must be added here in the same PR.**
**2. Third-party assets must include source and license note before release.**
**3. Delete stale generated artifacts from repo unless they are required fixtures.**
**4. Keep runtime assets and test-only assets clearly separated.**
**5. Confirm cross-platform icon completeness when updating native branding assets.**

# Risk
- Missing third-party license attribution can block release distribution.
- Untracked fixtures can increase repo size and obscure reproducibility.

# History
- 2026-03-03: Initial inventory file created.

# Appendices
- Recommended periodic command: `rg --files | rg 'icons|fixtures|assets|report|playwright'`
