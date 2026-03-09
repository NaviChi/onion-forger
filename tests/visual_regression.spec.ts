import { test, expect } from '@playwright/test';

test.describe('Phase 70: UI Visual Regression Pipelines', () => {

    test('Dashboard empty state matches baseline perfectly', async ({ page }) => {
        await page.goto('/?fixture=empty');
        await expect(page).toHaveScreenshot('dashboard-empty-state.png', {
            maxDiffPixelRatio: 0.05,
            animations: 'disabled'
        });
    });

    test('Dashboard crawling VFS fixture matches baseline perfectly', async ({ page }) => {
        await page.goto('/?fixture=vfs');
        await expect(page).toHaveScreenshot('dashboard-vfs-state.png', {
            maxDiffPixelRatio: 0.05,
            animations: 'disabled'
        });
    });

    test('Vanguard UI metrics scaling matches baseline perfectly', async ({ page }) => {
        await page.goto('/?fixture=vfs');
        const metricsCard = page.getByTestId('resource-metrics-card');
        await expect(metricsCard).toBeVisible();
        await expect(metricsCard).toHaveScreenshot('vanguard-metrics-state.png', {
            maxDiffPixelRatio: 0.02,
            animations: 'disabled' // Stop animations for deterministic renders
        });
    });
});
