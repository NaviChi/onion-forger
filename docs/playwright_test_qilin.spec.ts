import { test, expect } from '@playwright/test';

test('Run Qilin target and log output', async ({ page }) => {
  await page.goto('http://localhost:1420'); // Tauri dev server port
  console.log("Page loaded");

  // Wait for React to mount
  await page.waitForSelector('text=SYS: READY', { state: 'visible', timeout: 30000 });

  // Paste the URL
  const inputSelector = 'input[placeholder*="Target Address"]';
  await page.fill(inputSelector, 'http://ijzn3sicrcy7guixkzjkib4ukbiilwc3xhnmby4mcbccnsd7j2rekvqd.onion/site/view?uuid=c9d2ba19-6aa1-3087-8773-f63d023179ed');
  
  // Click Scan
  // Using generic button selector based on text as fallback if missing data-testid
  await page.click('button:has-text("SCANNING"), button:has-text("START SCAN")');
  console.log("Scan initiated");

  // Wait up to 120s and dump terminal logs
  let lastAdapter = "";
  for (let i = 0; i < 20; i++) {
    await page.waitForTimeout(5000);
    // Try to read adapter state from DOM
    // Look for a div that contains "ACTIVE ADAPTER" and its sibling/child text
    try {
        const adapterBlock = await page.locator('text=ACTIVE ADAPTER').locator('xpath=..').textContent();
        console.log(`[Status ${i*5}s] Adapter Block: ${adapterBlock}`);
    } catch(e) { }
    
    // Check phase
    try {
        const phaseBlock = await page.locator('text=OPERATION PHASE').locator('xpath=..').textContent();
        console.log(`[Status ${i*5}s] Phase Block: ${phaseBlock}`);
    } catch(e) {}
  }
});
