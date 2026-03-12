import { spawn } from "node:child_process";
import fs from "node:fs";
import fsp from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const repoRoot = path.resolve(__dirname, "..");
const artifactRoot = path.join(repoRoot, "output", "playwright", "native-webview-smoke");
const reportPath = path.join(artifactRoot, "native-webview-report.json");
const stdoutPath = path.join(artifactRoot, "native-webview.stdout.log");
const stderrPath = path.join(artifactRoot, "native-webview.stderr.log");
const timeoutMs = Number.parseInt(process.env.CRAWLI_NATIVE_SMOKE_TIMEOUT_MS ?? "120000", 10);
const manifestPath = path.join(repoRoot, "src-tauri", "Cargo.toml");
const binaryPath = path.join(repoRoot, "src-tauri", "target", "debug", process.platform === "win32" ? "crawli.exe" : "crawli");

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function removeIfExists(targetPath) {
  await fsp.rm(targetPath, { force: true, recursive: true }).catch(() => {});
}

async function runCommand(command, args, options = {}) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      cwd: repoRoot,
      env: process.env,
      stdio: "pipe",
      ...options,
    });
    let stderr = "";
    let stdout = "";
    child.stdout?.on("data", (chunk) => {
      stdout += chunk.toString();
    });
    child.stderr?.on("data", (chunk) => {
      stderr += chunk.toString();
    });
    child.on("exit", (code) => {
      if (code === 0) {
        resolve({ stdout, stderr });
      } else {
        reject(new Error(`${command} ${args.join(" ")} failed with exit ${code}\n${stderr || stdout}`));
      }
    });
    child.on("error", reject);
  });
}

async function killChild(child) {
  if (!child || child.killed) {
    return;
  }

  if (process.platform === "win32") {
    await runCommand("taskkill", ["/pid", String(child.pid), "/T", "/F"]).catch(() => {});
    return;
  }

  try {
    process.kill(child.pid, "SIGTERM");
  } catch {
    // Ignore termination races.
  }
}

async function waitForNativeSmokeReport(child) {
  const deadline = Date.now() + timeoutMs;
  let exitCode = null;
  child.on("exit", (code) => {
    exitCode = code;
  });

  while (Date.now() < deadline) {
    if (fs.existsSync(reportPath)) {
      return;
    }
    if (exitCode !== null) {
      break;
    }
    await sleep(500);
  }

  const stderrTail = fs.existsSync(stderrPath)
    ? (await fsp.readFile(stderrPath, "utf8")).split("\n").slice(-40).join("\n")
    : "(no stderr captured)";
  throw new Error(
    `Native webview smoke report was not produced before timeout/exit. Exit=${exitCode}\n${stderrTail}`,
  );
}

async function main() {
  await fsp.mkdir(artifactRoot, { recursive: true });
  await removeIfExists(reportPath);
  await removeIfExists(stdoutPath);
  await removeIfExists(stderrPath);

  const stdout = fs.createWriteStream(stdoutPath, { flags: "a" });
  const stderr = fs.createWriteStream(stderrPath, { flags: "a" });

  await runCommand("npm", ["run", "build"]);
  await runCommand("cargo", ["build", "--manifest-path", manifestPath, "--bin", "crawli"]);

  const child = spawn(binaryPath, [], {
    cwd: repoRoot,
    env: {
      ...process.env,
      CRAWLI_NATIVE_SMOKE_REPORT_PATH: reportPath,
      CRAWLI_NATIVE_SMOKE_AUTO_EXIT: "1",
      CRAWLI_NATIVE_SMOKE_WAIT_MS: process.env.CRAWLI_NATIVE_SMOKE_WAIT_MS ?? "8000",
    },
    stdio: ["ignore", "pipe", "pipe"],
  });

  child.stdout.pipe(stdout);
  child.stderr.pipe(stderr);

  try {
    await waitForNativeSmokeReport(child);
    const report = JSON.parse(await fsp.readFile(reportPath, "utf8"));
    if (!report.mounted || !report.isTauriRuntime || report.missingTestIds.length > 0) {
      throw new Error(`Native webview smoke failed: ${JSON.stringify(report, null, 2)}`);
    }

    console.log(
      `[native-smoke] mounted real Tauri shell with ${report.foundTestIds.length}/${report.expectedTestIds.length} critical controls`,
    );
  } finally {
    await killChild(child);
    stdout.end();
    stderr.end();
  }
}

main().catch((error) => {
  console.error(`[native-smoke] ${error instanceof Error ? error.message : String(error)}`);
  process.exitCode = 1;
});
