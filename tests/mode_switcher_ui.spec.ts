import { test, expect } from '@playwright/test';

test.describe('GUI Telemetry & Protocol Mode-Switcher', () => {
    test('verify default onion state and toggle across all three network protocol adapters', async ({ page }) => {
        // Navigate to local DevServer target
        await page.goto('/');

        // Target Input Placeholder
        const targetInput = page.getByTestId('input-target-url');

        // Default Load State should be Onion
        await expect(page.getByTestId('btn-onion')).toHaveClass(/active/);
        await expect(page.getByTestId('btn-mega')).not.toHaveClass(/active/);
        await expect(page.getByTestId('btn-torrent')).not.toHaveClass(/active/);
        await expect(targetInput).toHaveAttribute('placeholder', /http:\/\/\.\.\. \(⌘\+Enter/);

        // Switch to Mega.nz
        await page.getByTestId('btn-mega').click();
        await expect(page.getByTestId('btn-mega')).toHaveClass(/active/);
        await expect(page.getByTestId('btn-onion')).not.toHaveClass(/active/);
        await expect(targetInput).toHaveAttribute('placeholder', /https:\/\/mega\.nz\/folder\/\.\.\./);

        // Switch to BitTorrent
        await page.getByTestId('btn-torrent').click();
        await expect(page.getByTestId('btn-torrent')).toHaveClass(/active/);
        await expect(page.getByTestId('btn-mega')).not.toHaveClass(/active/);
        await expect(targetInput).toHaveAttribute('placeholder', /magnet:\?xt=\.\.\. or drop/);

        // Revert back to Tor / Onion
        await page.getByTestId('btn-onion').click();
        await expect(page.getByTestId('btn-onion')).toHaveClass(/active/);
        await expect(page.getByTestId('btn-torrent')).not.toHaveClass(/active/);
        await expect(targetInput).toHaveAttribute('placeholder', /http:\/\/\.\.\. \(⌘\+Enter/);
    });
});
