import { test, expect } from '@playwright/test';

test.describe('Vanguard UI', () => {

    test('should display Vanguard Stealth Ramp status when present in telemetry', async ({ page }) => {
        // Navigate to the base URL configured with fixture=vfs
        await page.goto('/?fixture=vfs');

        // Check that the resource metrics card is visible
        await expect(page.getByTestId('resource-metrics-card')).toBeVisible();

        // Check that the Vanguard text appears inside the metrics card or output area
        // Since the previous implementation replaced worker counts or appended Vanguard state inside resource-worker-metrics
        const vanguardTextLocator = page.locator('text=Vanguard: Active (Heatmap Enabled)');
        await expect(vanguardTextLocator).toBeVisible();

        // Also check that circuit counts are merged into the display properly
        const circuitsTextLocator = page.locator('text=Circuits 9/12');
        await expect(circuitsTextLocator).toBeVisible();
    });

});
