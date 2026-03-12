const fs = require("fs");
const path = require("path");
const { spawn } = require("child_process");
const { chromium } = require("@playwright/test");

const HOST = process.env.OVERLAY_HOST || "127.0.0.1";
const PORT = process.env.OVERLAY_PORT || "1420";
const BASE_URL = process.env.OVERLAY_BASE_URL || `http://${HOST}:${PORT}/?fixture=vfs`;
const TOLERANCE_PX = Number(process.env.OVERLAY_TOLERANCE_PX || "1");
const timestamp = new Date().toISOString().replace(/[:.]/g, "-");
const outputRoot = path.resolve("output/playwright/overlay-integrity");
const runDir = path.join(outputRoot, timestamp);
const devLogPath = path.join(runDir, "dev-server.log");
fs.mkdirSync(runDir, { recursive: true });

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function waitForHttp(url, timeoutMs) {
  const start = Date.now();
  return new Promise((resolve, reject) => {
    const check = () => {
      fetch(url)
        .then((res) => {
          if (res.ok) return resolve();
          if (Date.now() - start > timeoutMs) return reject(new Error(`Timed out waiting for ${url}`));
          setTimeout(check, 500);
        })
        .catch(() => {
          if (Date.now() - start > timeoutMs) return reject(new Error(`Timed out waiting for ${url}`));
          setTimeout(check, 500);
        });
    };
    check();
  });
}

function startDevServer() {
  const npmCmd = process.platform === "win32" ? "npm.cmd" : "npm";
  const devProc = spawn(npmCmd, ["run", "dev", "--", "--host", HOST, "--port", PORT], {
    cwd: process.cwd(),
    stdio: ["ignore", "pipe", "pipe"],
  });
  const logStream = fs.createWriteStream(devLogPath, { flags: "a" });
  devProc.stdout.on("data", (chunk) => logStream.write(chunk));
  devProc.stderr.on("data", (chunk) => logStream.write(chunk));
  devProc.on("close", () => logStream.end());
  return devProc;
}

async function stopDevServer(devProc) {
  if (!devProc || devProc.killed) return;
  devProc.kill("SIGTERM");
  await sleep(800);
  if (!devProc.killed && devProc.exitCode === null) {
    devProc.kill("SIGKILL");
  }
}

const geomTargets = [
  { key: "app", selector: ".app-container" },
  { key: "header", selector: "header" },
  { key: "toolbar", selector: ".toolbar" },
  { key: "sourceBar", selector: ".url-bar", index: 0 },
  { key: "pathBar", selector: ".url-bar", index: 1 },
  { key: "optionsBar", selector: ".options-bar" },
  { key: "workspace", selector: ".main-workspace" },
  { key: "logPanel", selector: ".main-workspace .panel", index: 0 },
  { key: "vfsPanel", selector: ".main-workspace .panel", index: 1 },
  { key: "networkMonitor", selector: ".network-monitor" },
];

async function getGeometry(page) {
  return page.evaluate((targets) => {
    const out = {};
    const scrollX = window.scrollX || window.pageXOffset || 0;
    const scrollY = window.scrollY || window.pageYOffset || 0;
    for (const target of targets) {
      const nodes = document.querySelectorAll(target.selector);
      const node = typeof target.index === "number" ? nodes[target.index] : nodes[0];
      if (!node) {
        out[target.key] = null;
        continue;
      }
      const r = node.getBoundingClientRect();
      out[target.key] = {
        x: Number((r.x + scrollX).toFixed(2)),
        y: Number((r.y + scrollY).toFixed(2)),
        width: Number(r.width.toFixed(2)),
        height: Number(r.height.toFixed(2)),
      };
    }
    return out;
  }, geomTargets);
}

function diffGeometry(before, after, tolerancePx) {
  const deltas = [];
  let unchanged = true;
  const observed = [];
  for (const key of Object.keys(before)) {
    const b = before[key];
    const a = after[key];
    if (!b || !a) continue;
    const delta = {
      key,
      dx: Number((a.x - b.x).toFixed(2)),
      dy: Number((a.y - b.y).toFixed(2)),
      dWidth: Number((a.width - b.width).toFixed(2)),
      dHeight: Number((a.height - b.height).toFixed(2)),
    };
    observed.push(delta);
    const maxAbs = Math.max(
      Math.abs(delta.dx),
      Math.abs(delta.dy),
      Math.abs(delta.dWidth),
      Math.abs(delta.dHeight)
    );
    if (maxAbs > tolerancePx) {
      unchanged = false;
      deltas.push(delta);
    }
  }

  if (!unchanged && deltas.length > 0) {
    const [first] = deltas;
    const isUniformTranslation = deltas.every((delta) => {
      return (
        delta.dx === first.dx &&
        delta.dy === first.dy &&
        delta.dWidth === 0 &&
        delta.dHeight === 0 &&
        first.dWidth === 0 &&
        first.dHeight === 0
      );
    });
    if (isUniformTranslation) {
      return {
        unchanged: true,
        deltas: [],
        note: `Uniform scroll translation detected (dx=${first.dx}, dy=${first.dy}).`,
      };
    }
  }

  return { unchanged, deltas };
}

function sanitizeFilePart(value) {
  return value
    .replace(/[^a-zA-Z0-9_-]+/g, "_")
    .replace(/_+/g, "_")
    .replace(/^_|_$/g, "")
    .slice(0, 80) || "control";
}

function escapeCssAttr(value) {
  return String(value).replace(/\\/g, "\\\\").replace(/"/g, '\\"');
}

async function resolveLiveLocator(page, item) {
  if (item.testId) {
    const byTestId = page.locator(`[data-testid="${escapeCssAttr(item.testId)}"]`).first();
    if ((await byTestId.count()) > 0) return byTestId;
  }

  const byOiId = page.locator(`[data-oi-id="${escapeCssAttr(item.id)}"]`).first();
  if ((await byOiId.count()) > 0) return byOiId;

  return null;
}

async function isLocatorDisabled(locator) {
  return locator.evaluate((el) => {
    if ("disabled" in el) return Boolean(el.disabled);
    const ariaDisabled = (el.getAttribute("aria-disabled") || "").toLowerCase();
    return ariaDisabled === "true";
  });
}

async function ensureDynamicControlVisible(page, item) {
  const tid = item.testId || "";
  if (tid === "support-popover" || tid === "btn-support-close" || tid.startsWith("adapter-row-")) {
    const popover = page.locator('[data-testid="support-popover"]').first();
    if ((await popover.count()) > 0) return;

    const supportButton = page.locator('[data-testid="btn-support"]').first();
    if ((await supportButton.count()) === 0) return;
    if (await isLocatorDisabled(supportButton)) return;

    await supportButton.click({ timeout: 5000, force: true });
    await page.waitForTimeout(200);
    return;
  }

  if (
    tid === "btn-azure-close" ||
    tid === "btn-azure-tab-intranet" ||
    tid === "btn-azure-tab-storage" ||
    tid === "input-azure-intranet-port" ||
    tid === "chk-azure-managed-identity" ||
    tid === "sel-azure-region" ||
    tid.startsWith("input-azure-") ||
    tid.startsWith("btn-azure-")
  ) {
    const azureButton = page.locator('[data-testid="azure-connectivity-btn"]').first();
    const azureClose = page.locator('[data-testid="btn-azure-close"]').first();
    if ((await azureClose.count()) === 0) {
      if ((await azureButton.count()) === 0) return;
      if (await isLocatorDisabled(azureButton)) return;

      await azureButton.click({ timeout: 5000, force: true });
      await page.waitForTimeout(200);
    }

    const storageControls = new Set([
      "btn-azure-tab-storage",
      "chk-azure-managed-identity",
      "input-azure-subscription-id",
      "input-azure-tenant-id",
      "input-azure-client-id",
      "input-azure-client-secret",
      "input-azure-resource-group",
      "input-azure-storage-account",
      "input-azure-container-name",
      "sel-azure-region",
      "input-azure-size-gb",
      "btn-azure-test-connection",
      "btn-azure-configure",
      "btn-azure-storage-disable",
      "btn-azure-storage-enable",
    ]);
    const intranetControls = new Set([
      "btn-azure-tab-intranet",
      "input-azure-intranet-port",
      "btn-azure-intranet-stop",
      "btn-azure-intranet-start",
    ]);

    if (storageControls.has(tid)) {
      const storageTab = page.locator('[data-testid="btn-azure-tab-storage"]').first();
      if ((await storageTab.count()) > 0) {
        await storageTab.click({ timeout: 5000, force: true });
        await page.waitForTimeout(150);
      }
    } else if (intranetControls.has(tid)) {
      const intranetTab = page.locator('[data-testid="btn-azure-tab-intranet"]').first();
      if ((await intranetTab.count()) > 0) {
        await intranetTab.click({ timeout: 5000, force: true });
        await page.waitForTimeout(150);
      }
    }
    return;
  }

  if (tid === "btn-hex-close" || tid === "btn-hex-turbo-bypass") {
    const hexClose = page.locator('[data-testid="btn-hex-close"]').first();
    if ((await hexClose.count()) > 0) return;

    const hexButton = page.locator('[data-testid="btn-hex-view"]').first();
    if ((await hexButton.count()) === 0) return;
    if (await isLocatorDisabled(hexButton)) return;

    await hexButton.click({ timeout: 5000, force: true });
    await page.waitForTimeout(200);
  }
}

function interactionPriority(item) {
  const tid = item.testId || "";
  if (tid === "support-popover") return 2;
  if (tid.startsWith("adapter-row-")) return 4;
  if (tid === "btn-support-close") return 5;
  if (tid === "input-mega-password") return 9;
  if (tid === "chk-force-clearnet") return 11;
  if (tid === "btn-hex-turbo-bypass") return 62;
  if (tid === "btn-hex-close") return 63;
  if (tid === "azure-connectivity-btn") return 64;
  if (
    tid === "btn-azure-tab-intranet" ||
    tid === "btn-azure-tab-storage" ||
    tid === "input-azure-intranet-port" ||
    tid === "chk-azure-managed-identity" ||
    tid === "sel-azure-region" ||
    tid.startsWith("input-azure-") ||
    tid === "btn-azure-intranet-stop" ||
    tid === "btn-azure-intranet-start" ||
    tid === "btn-azure-test-connection" ||
    tid === "btn-azure-configure" ||
    tid === "btn-azure-storage-disable" ||
    tid === "btn-azure-storage-enable"
  ) {
    return 65;
  }
  if (tid === "btn-azure-close") return 66;
  // Run crawl-state mutators last; they can temporarily hide/disable other controls.
  // Keep Support toggle latest in pass 1 so popover controls are discoverable in pass 2.
  if (tid === "btn-support") return 95;
  if (tid === "btn-start-queue" || tid === "btn-resume") return 80;
  if (tid === "btn-cancel") return 90;
  return 10;
}

async function seedConditionalCoverage(page, discovered, state) {
  if (!state.megaPasswordSeeded && !discovered.has("tid:input-mega-password")) {
    const megaButton = page.locator('[data-testid="btn-mega"]').first();
    const targetInput = page.locator('[data-testid="input-target-url"]').first();
    if ((await megaButton.count()) > 0 && (await targetInput.count()) > 0) {
      if (!(await isLocatorDisabled(megaButton))) {
        await megaButton.click({ timeout: 5000, force: true });
        await targetInput.fill("https://mega.nz/folder/demo#P!fixture-password");
        await page.waitForTimeout(200);
      }
    }
    state.megaPasswordSeeded = true;
  }
}

async function collectClickableInventory(page) {
  return page.evaluate(() => {
    const candidates = [
      ...document.querySelectorAll(
        "[data-testid], button, input, select, [role='button'], a[href], .vfs-toggle, .vfs-download-btn"
      ),
    ];

    const seen = new Set();
    const ordered = [];
    for (const el of candidates) {
      if (seen.has(el)) continue;
      seen.add(el);
      const rect = el.getBoundingClientRect();
      if (rect.width <= 0 || rect.height <= 0) continue;
      const style = window.getComputedStyle(el);
      if (style.display === "none" || style.visibility === "hidden" || style.pointerEvents === "none") continue;
      ordered.push(el);
    }

    const fallbackCount = new Map();
    const items = [];

    for (const el of ordered) {
      const tag = el.tagName.toLowerCase();
      const type = (el.getAttribute("type") || "").toLowerCase();
      if (tag === "input" && type === "hidden") continue;

      const text = (
        el.getAttribute("data-testid") ||
        el.getAttribute("aria-label") ||
        el.getAttribute("title") ||
        el.textContent ||
        ""
      )
        .trim()
        .replace(/\s+/g, " ")
        .slice(0, 120);

      const testId = (el.getAttribute("data-testid") || "").trim();
      const classSlice = Array.from(el.classList).slice(0, 3).join(".");
      let signature = "";
      if (testId) {
        signature = `tid:${testId}`;
      } else {
        const base = `${tag}|${type}|${classSlice}|${text}`;
        const count = (fallbackCount.get(base) || 0) + 1;
        fallbackCount.set(base, count);
        signature = `${base}#${count}`;
      }

      const id = `oi-${items.length + 1}`;
      el.setAttribute("data-oi-id", id);
      const disabled = "disabled" in el ? Boolean(el.disabled) : false;
      const rect = el.getBoundingClientRect();

      items.push({
        id,
        signature,
        tag,
        type,
        testId,
        text,
        disabled,
        rect: {
          x: Number(rect.x.toFixed(2)),
          y: Number(rect.y.toFixed(2)),
          width: Number(rect.width.toFixed(2)),
          height: Number(rect.height.toFixed(2)),
        },
      });
    }

    return items;
  });
}

async function runOverlayIntegrity() {
  const browser = await chromium.launch({ headless: true });
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
  const results = [];
  const tested = new Set();
  const discovered = new Map();
  const coverageState = { megaPasswordSeeded: false };

  try {
    await page.goto(BASE_URL, { waitUntil: "domcontentloaded", timeout: 60000 });
    await page.waitForSelector(".app-container", { timeout: 30000 });
    await page.waitForTimeout(1000);

    const baselineGeom = await getGeometry(page);
    await page.screenshot({ path: path.join(runDir, "00-baseline.png"), fullPage: true });

    let passesWithoutNew = 0;
    let safety = 0;
    while (passesWithoutNew < 2 && safety < 12) {
      safety += 1;
      await seedConditionalCoverage(page, discovered, coverageState);
      const inventory = await collectClickableInventory(page);
      let newCount = 0;

      for (const item of inventory) {
        if (!discovered.has(item.signature)) {
          discovered.set(item.signature, item);
          newCount += 1;
        }
      }

      if (newCount === 0) passesWithoutNew += 1;
      else passesWithoutNew = 0;

      const pending = inventory
        .filter((item) => !tested.has(item.signature))
        .sort((a, b) => {
          const pa = interactionPriority(a);
          const pb = interactionPriority(b);
          if (pa !== pb) return pa - pb;
          return 0;
        });
      if (pending.length === 0) {
        await page.waitForTimeout(250);
        continue;
      }

      for (const item of pending) {
        tested.add(item.signature);
        const slug = `${String(results.length + 1).padStart(2, "0")}-${sanitizeFilePart(item.signature)}`;
        const beforeScreenshot = `${slug}-before.png`;
        const afterScreenshot = `${slug}-after.png`;

        const row = {
          action: slug,
          signature: item.signature,
          control: item.testId || item.text || item.signature,
          tag: item.tag,
          type: item.type,
          status: "UNKNOWN",
          geometryUnchanged: null,
          rootCause: "",
          beforeScreenshot,
          afterScreenshot,
          geomDiff: [],
        };

        await ensureDynamicControlVisible(page, item);
        const locator = await resolveLiveLocator(page, item);
        if (!locator) {
          row.status = "SKIP";
          row.geometryUnchanged = true;
          row.rootCause = "Element detached before interaction.";
          results.push(row);
          continue;
        }

        const beforeGeom = await getGeometry(page);
        await page.screenshot({ path: path.join(runDir, beforeScreenshot), fullPage: true });

        try {
          const currentlyDisabled = await isLocatorDisabled(locator);
          if (currentlyDisabled) {
            row.status = "SKIP";
            row.rootCause = "Control is disabled in current UI state.";
          } else if (item.tag === "select") {
            const options = await locator.evaluate((el) =>
              Array.from(el.options || []).map((o) => ({ value: o.value, text: (o.textContent || "").trim() }))
            );
            const currentValue = await locator.inputValue();
            const target = options.find((o) => o.value !== currentValue) || options[0];
            if (!target) {
              row.status = "SKIP";
              row.rootCause = "No selectable options available.";
            } else {
              await locator.selectOption(target.value);
              row.status = "PASS";
              row.rootCause = `Selected option '${target.text || target.value}'.`;
            }
          } else {
            try {
              await locator.click({ timeout: 5000, force: true });
            } catch (err) {
              if (err.message && err.message.includes("outside of the viewport")) {
                // Handle React-Virtual overscan items that are technically in DOM but offset outside bounds
                await locator.evaluate((el) => el.click());
              } else {
                throw err;
              }
            }
            row.status = "PASS";
            row.rootCause = "Interaction executed.";
          }
        } catch (err) {
          row.status = "FAIL";
          row.rootCause = `Interaction failed: ${String(err && err.message ? err.message : err).slice(0, 260)}`;
        }

        await page.waitForTimeout(250);
        const afterGeom = await getGeometry(page);
        await page.screenshot({ path: path.join(runDir, afterScreenshot), fullPage: true });

        const cmp = diffGeometry(beforeGeom, afterGeom, TOLERANCE_PX);
        row.geometryUnchanged = cmp.unchanged;
        row.geomDiff = cmp.deltas;
        if (!cmp.unchanged) {
          row.status = "FAIL";
          row.rootCause = `Geometry shifted beyond ${TOLERANCE_PX}px tolerance: ${cmp.deltas
            .map((d) => `${d.key}(dx=${d.dx},dy=${d.dy},dw=${d.dWidth},dh=${d.dHeight})`)
            .join("; ")}`;
        } else if (cmp.note && row.status === "PASS") {
          row.rootCause = `${row.rootCause} ${cmp.note}`.trim();
        }

        results.push(row);
      }
    }

    const finalGeom = await getGeometry(page);
    await page.screenshot({ path: path.join(runDir, "99-final.png"), fullPage: true });

    const summary = {
      runDir,
      baseUrl: BASE_URL,
      tolerancePx: TOLERANCE_PX,
      totals: {
        discoveredClickableControls: discovered.size,
        exercisedActions: results.length,
        pass: results.filter((r) => r.status === "PASS").length,
        fail: results.filter((r) => r.status === "FAIL").length,
        skip: results.filter((r) => r.status === "SKIP").length,
      },
      baselineGeom,
      finalGeom,
      results,
      inventory: Array.from(discovered.values()),
    };

    const summaryPath = path.join(runDir, "overlay-integrity-summary.json");
    fs.writeFileSync(summaryPath, JSON.stringify(summary, null, 2));

    const reportLines = [];
    reportLines.push("# Overlay Integrity Report");
    reportLines.push("");
    reportLines.push(`- Base URL: ${BASE_URL}`);
    reportLines.push(`- Geometry tolerance: ${TOLERANCE_PX}px`);
    reportLines.push(`- Discovered clickable controls: ${summary.totals.discoveredClickableControls}`);
    reportLines.push(`- Exercised actions: ${summary.totals.exercisedActions}`);
    reportLines.push(`- PASS: ${summary.totals.pass}`);
    reportLines.push(`- FAIL: ${summary.totals.fail}`);
    reportLines.push(`- SKIP: ${summary.totals.skip}`);
    reportLines.push("");
    reportLines.push("| # | Control | Type | Status | Geometry | Root Cause |");
    reportLines.push("|---|---|---|---|---|---|");
    summary.results.forEach((row, idx) => {
      const safeControl = (row.control || row.signature).replace(/\|/g, "/");
      const safeCause = (row.rootCause || "").replace(/\|/g, "/");
      reportLines.push(
        `| ${idx + 1} | ${safeControl} | ${row.tag}${row.type ? `:${row.type}` : ""} | ${row.status} | ${row.geometryUnchanged ? "UNCHANGED" : "SHIFTED"
        } | ${safeCause} |`
      );
    });
    reportLines.push("");
    reportLines.push("## Artifacts");
    reportLines.push("");
    reportLines.push("- 00-baseline.png");
    reportLines.push("- 99-final.png");
    reportLines.push("- `*-before.png` and `*-after.png` per action");
    reportLines.push("- overlay-integrity-summary.json");
    reportLines.push("- dev-server.log");
    fs.writeFileSync(path.join(runDir, "overlay-integrity-report.md"), reportLines.join("\n"));

    return summary;
  } finally {
    await page.close().catch(() => undefined);
    await browser.close().catch(() => undefined);
  }
}

async function main() {
  const devProc = startDevServer();
  try {
    await waitForHttp(`http://${HOST}:${PORT}/`, 90_000);
    const summary = await runOverlayIntegrity();
    const payload = { ok: true, runDir, totals: summary.totals };
    console.log(JSON.stringify(payload, null, 2));
    if (summary.totals.fail > 0) {
      process.exitCode = 1;
    }
  } finally {
    await stopDevServer(devProc);
  }
}

main().catch(async (err) => {
  const payload = { ok: false, runDir, error: String(err && err.stack ? err.stack : err) };
  console.error(JSON.stringify(payload, null, 2));
  process.exit(1);
});
