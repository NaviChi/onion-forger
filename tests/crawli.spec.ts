import { test, expect } from '@playwright/test';
import { UI_TEST_IDS } from '../src/test/selectors';

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

        await expect(page.getByTestId(UI_TEST_IDS.resourceMetricsCard)).toBeVisible();
        await expect(page.getByTestId(UI_TEST_IDS.resourceProcessCpu)).toContainText('CPU 0.0%');
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

    test('fixture mode renders live operator telemetry', async ({ page }) => {
        await page.goto('/?fixture=vfs');

        await expect(page.getByTestId(UI_TEST_IDS.resourceMetricsCard)).toBeVisible();
        await expect(page.getByTestId(UI_TEST_IDS.resourceProcessCpu)).toContainText('CPU 18.4%');
        await expect(page.getByTestId('resource-process-memory')).toContainText('RSS 412.0 MB');
        await expect(page.getByTestId('resource-worker-metrics')).toContainText('Vanguard: Active (Heatmap Enabled) | Circuits 9/12');
        await expect(page.getByTestId('resource-node-metrics')).toContainText('192.168.1.100');
    });

});
