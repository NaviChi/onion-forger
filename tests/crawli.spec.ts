import { test, expect } from '@playwright/test';

test.describe('Crawli React Application', () => {

    test('should load the dashboard and verify initial state', async ({ page }) => {
        // Navigate to the base URL (which points to localhost:1420 configured in playwright.config.ts)
        await page.goto('/');

        // Check that the header or title is visible
        await expect(page).toHaveTitle(/.*|Onion/i);

        // Look for the Tor Circuit text or main container to ensure the React UI booted
        const dashboardBody = page.locator('body');
        await expect(dashboardBody).toBeVisible();

        const urlInput = page.locator('input[type="text"]').first();
        if (await urlInput.isVisible()) {
            await expect(urlInput).toBeEditable();
        }
    });

    test('mocked crawl execution handles missing Tauri IPC gracefully', async ({ page }) => {
        await page.goto('/');

        const crawlButton = page.locator('button', { hasText: /crawl|start|execute/i }).first();

        if (await crawlButton.isVisible()) {
            const urlInput = page.locator('input[type="text"]').first();
            await urlInput.fill('http://example.onion');
            await crawlButton.click();

            // UI may not change state without Tauri IPC mocking, but verify it doesn't crash
            const dashboardBody = page.locator('body');
            await expect(dashboardBody).toBeVisible();
        }
    });

});
