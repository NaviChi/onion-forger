import { test, expect, chromium } from '@playwright/test';
const run = async () => {
    const browser = await chromium.launch();
    const page = await browser.newPage();
    await page.goto("http://localhost:1420");
    await page.waitForTimeout(3000);
    
    await page.getByTestId("btn-load-target").click();
    await page.waitForTimeout(500);
    
    await page.fill('input.search-bar', 'http://ijzn3sicrcy7guixkzjkib4ukbiilwc3xhnmby4mcbccnsd7j2rekvqd.onion/site/view?uuid=c9d2ba19-6aa1-3087-8773-f63d023179ed');
    await page.waitForTimeout(500);
    
    await page.getByTestId("btn-resume").click();
    
    console.log("Crawl started. Waiting up to 135 seconds for outcome...");
    
    // Check elements
    for (let i = 0; i < 30; i++) {
        const text = await page.locator(".toast-message").allTextContents();
        if (text.length > 0) {
            console.log("Toast detected:", text);
            break;
        }
        await page.waitForTimeout(5000);
        const logs = await page.locator(".forensic-line").allTextContents();
        console.log(`Log lines: ${logs.length}`);
    }
    
    await browser.close();
};
run();
