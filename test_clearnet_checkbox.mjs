import { chromium } from "playwright-core";
import { spawn } from "child_process";

const devProc = spawn("npm", ["run", "dev", "--", "--host", "127.0.0.1", "--port", "1421"], {
    stdio: "ignore",
    detached: true,
});

setTimeout(async () => {
    try {
        const browser = await chromium.launch({ headless: true });
        const context = await browser.newContext({ viewport: { width: 1280, height: 800 } });
        const page = await context.newPage();
        await page.goto("http://127.0.0.1:1421/?fixture=vfs", { waitUntil: "networkidle" });
        
        const chk = page.locator('[data-testid="chk-force-clearnet"]');
        const count = await chk.count();
        console.log(`Found force_clearnet checkbox count: ${count}`);
        
        if (count > 0) {
            const isVisible = await chk.isVisible();
            const box = await chk.boundingBox();
            console.log(`Is Visible: ${isVisible}, BoundingBox:`, box);
            
            // Try to click it
            await chk.click({ timeout: 5000 });
            const isChecked = await chk.isChecked();
            console.log(`Is Checked after click: ${isChecked}`);
        } else {
            // Print all test ids
            const testIds = await page.evaluate(() => {
                return Array.from(document.querySelectorAll('[data-testid]')).map(el => el.getAttribute('data-testid'));
            });
            console.log("All test-ids on page:", testIds);
        }
        
        await browser.close();
    } catch (e) {
        console.error("Test failed:", e);
    } finally {
        process.kill(-devProc.pid);
        process.exit(0);
    }
}, 3000);
