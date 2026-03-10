import { test, expect } from '@playwright/test';

test.describe('GUI Telemetry & Protocol Mode-Switcher', () => {
    test('verify default onion state and toggle across all four network protocol adapters', async ({ page }) => {
        // Navigate to local DevServer target
        await page.goto('/');

        // Target Input Placeholder
        const targetInput = page.getByTestId('input-target-url');

        // Default Load State should be Onion
        await expect(page.getByTestId('btn-onion')).toHaveClass(/active/);
        await expect(page.getByTestId('btn-direct')).not.toHaveClass(/active/);
        await expect(page.getByTestId('btn-mega')).not.toHaveClass(/active/);
        await expect(page.getByTestId('btn-torrent')).not.toHaveClass(/active/);
        await expect(targetInput).toHaveAttribute('placeholder', /http:\/\/\.\.\. \(⌘\+Enter/);

        // Switch to Direct / clearnet
        await page.getByTestId('btn-direct').click();
        await expect(page.getByTestId('btn-direct')).toHaveClass(/active/);
        await expect(page.getByTestId('btn-onion')).not.toHaveClass(/active/);
        await expect(targetInput).toHaveAttribute('placeholder', /https:\/\/example\.com\/archive\.7z/);

        // Switch to Mega.nz
        await page.getByTestId('btn-mega').click();
        await expect(page.getByTestId('btn-mega')).toHaveClass(/active/);
        await expect(page.getByTestId('btn-direct')).not.toHaveClass(/active/);
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

    test('auto-detects direct vs onion URLs from the input host', async ({ page }) => {
        await page.goto('/');

        const targetInput = page.getByTestId('input-target-url');

        await targetInput.fill('https://proof.ovh.net/files/10Gb.dat');
        await expect(page.getByTestId('btn-direct')).toHaveClass(/active/);
        await expect(page.getByTestId('btn-onion')).not.toHaveClass(/active/);

        await targetInput.fill('https://cdn.breachforums.as/pay_or_leak/shouldve_paid_the_ransom_pathstone.com_shinyhunters.7z');
        await expect(page.getByTestId('btn-direct')).toHaveClass(/active/);
        await expect(page.getByTestId('btn-onion')).not.toHaveClass(/active/);

        await targetInput.fill('http://example.onion/files/');
        await expect(page.getByTestId('btn-onion')).toHaveClass(/active/);
        await expect(page.getByTestId('btn-direct')).not.toHaveClass(/active/);
    });
});
