# Onion Forger (Crawli)

Onion Forger is a Tauri + React desktop application for dark-web crawl orchestration and forensic mirroring workflows.

## Local Development

```bash
npm ci
npm run dev
```

## Validation

```bash
# Frontend compile check
npm run build

# Rust tests
cd src-tauri && cargo test

# Playwright UI test
cd .. && npx playwright test --project=chromium

# Full overlay integrity workflow (geometry + screenshots + matrix)
cd .. && npm run overlay:integrity
```

## Local macOS Package Build

```bash
npm run tauri build
```

Build outputs are written under:

`src-tauri/target/release/bundle/`

## GitHub Release Workflows

Primary multi-OS release workflow:

`.github/workflows/release.yml`

Published assets:
- Linux: `.deb` / `.rpm` bundles + portable tarball
- Windows: portable zip only (`crawli_<tag>_windows_x64_portable.zip`)
- macOS: portable `.app` tarballs for Intel and Apple Silicon

Windows portable zip contents:
- `crawli.exe` (GUI)
- `crawli-cli.exe` (console CLI)
- `crawli-cli.cmd` wrapper
- optional runtime payloads from `src-tauri/bin/win_x64` when present

### Trigger by Tag (recommended)

```bash
git tag v0.1.0
git push origin v0.1.0
```

### Manual Trigger

Run `Release` in GitHub Actions with input tag like `v0.1.0`.

### Windows-Only Portable Publish

Run `.github/workflows/release-windows-portable.yml` with:
- `tag`: release tag (for example `v0.1.0`)
- `ref`: commit/branch to build (defaults to `main`)

This workflow uploads or replaces only the Windows portable zip and removes stale Windows installer assets (`.exe`/`.msi`) if they exist.

## Browser Fixture Mode

Use this URL for deterministic UI fixture data without Tauri backend IPC:

`http://127.0.0.1:1420/?fixture=vfs`
