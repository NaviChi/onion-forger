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
```

## Local macOS Package Build

```bash
npm run tauri build
```

Build outputs are written under:

`src-tauri/target/release/bundle/`

## GitHub Multi-OS Release Build

This repository includes a GitHub Actions workflow at:

`.github/workflows/release.yml`

It builds and uploads release installers for:

- Linux (`.deb` / `.AppImage`)
- Windows (`.msi` / `.exe`)
- macOS (`.dmg` / `.app`) for both Intel and Apple Silicon

### Trigger by Tag (recommended)

```bash
git tag v0.1.0
git push origin v0.1.0
```

### Manual Trigger

Run `Release` in GitHub Actions with input tag like `v0.1.0`.
