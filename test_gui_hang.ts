import { chromium } from "playwright";

async function run() {
    const browser = await chromium.launch();
    const page = await browser.newPage();

    console.log("Navigating to app...");
    await page.goto("http://localhost:1420");
    await page.waitForTimeout(2000);

    console.log("Filling URL");
    await page.fill('input[type="text"]', "http://ijzn3sicrcy7guixkzjkib4ukbiilwc3xhnmby4mcbccnsd7j2rekvqd.onion/site/view?uuid=c9d2ba19-6aa1-3087-8773-f63d023179ed");

    console.log("Clicking Retry/Resume");
    await page.getByTestId("btn-resume").click();

    for (let i = 0; i < 30; i++) {
        await page.waitForTimeout(5000);
        console.log(`Waiting... ${i * 5}s elapsed`);
        const logs = await page.locator('.forensic-line').allTextContents();
        console.log(`Got ${logs.length} logs lines:`);
        console.log(logs.slice(-5).join('\n'));

        const toasts = await page.locator('.toast-message').allTextContents();
        if (toasts.length > 0) {
            console.log("TOAST APPEARED:", toasts);
            break;
        }
    }

    await browser.close();
}

run().catch(console.error);
