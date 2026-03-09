
## Vitest Frontend Component Testing
- **Date**: 2026-03-08
- **Action**: Installed `@testing-library/react`, `@testing-library/jest-dom`, and `@testing-library/user-event` to provide isolated component smoke tests via Vitest. Added `src/setupTests.ts` to mock Tauri OS-level APIs in a Node `jsdom` context.
- **Coverage**: Addressed 0% coverage gaps by wiring `VibeLoader.test.tsx`, `Dashboard.test.tsx`, `VfsTreeView.test.tsx`, `VFSExplorer.test.tsx`, and `AzureConnectivityModal.test.tsx`.
- **Purpose**: Fast feedback loop without launching Headless Chrome E2E via Playwright. Playwright tests (`tests/*.spec.ts`) continue to reign for integration tests (Port 0 mapping).
