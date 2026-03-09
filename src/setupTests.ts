import '@testing-library/jest-dom';
import { vi } from 'vitest';

// Mock matchMedia
Object.defineProperty(window, 'matchMedia', {
    writable: true,
    value: vi.fn().mockImplementation(query => ({
        matches: false,
        media: query,
        onchange: null,
        addListener: vi.fn(), // Deprecated
        removeListener: vi.fn(), // Deprecated
        addEventListener: vi.fn(),
        removeEventListener: vi.fn(),
        dispatchEvent: vi.fn(),
    })),
});

// Mock Tauri API events to prevent Uncaught Errors
vi.mock('@tauri-apps/api/event', () => ({
    listen: vi.fn().mockResolvedValue(() => { }), // Unlisten function
    emit: vi.fn().mockResolvedValue(undefined),
}));

// Mock Tauri Core invoke
vi.mock('@tauri-apps/api/core', () => ({
    invoke: vi.fn().mockResolvedValue({}),
}));
