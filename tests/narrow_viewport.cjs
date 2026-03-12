const { chromium } = require("@playwright/test");
const path = require("path");

(async () => {
  const browser = await chromium.launch({ headless: true });
  const page = await browser.newPage({ viewport: { width: 1000, height: 750 } });
  
  await page.goto("http://127.0.0.1:4444/?fixture=vfs");
  await page.waitForTimeout(2000);
  
  await page.screenshot({ path: "output/playwright/narrow.png", fullPage: true });
  
  await browser.close();
})();
