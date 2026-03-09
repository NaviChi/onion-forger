import { defineConfig } from 'vitest/config';
import react from '@vitejs/plugin-react';

export default defineConfig({
    plugins: [react()],
    test: {
        environment: 'jsdom',
        setupFiles: ['./src/setupTests.ts'],
        globals: true,
        include: ['src/**/*.test.{ts,tsx}'],
        coverage: {
            provider: 'v8',
            reporter: ['text', 'json', 'html'],
            include: ['src/components/**/*.{ts,tsx}', 'src/utils/**/*.{ts,tsx}'],
            exclude: ['src/**/*.test.{ts,tsx}', 'src/fixtures/**']
        }
    }
});
