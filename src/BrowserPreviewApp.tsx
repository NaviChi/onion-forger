import { useEffect, useMemo, useState } from "react";
import {
  Activity,
  AlertCircle,
  CheckCircle,
  CircleHelp,
  Clock,
  Cloud,
  Cpu,
  Database,
  FileJson,
  FolderSearch,
  Globe,
  HardDrive,
  ListTree,
  Magnet,
  Play,
  Save,
  ShieldAlert,
  Terminal,
  XCircle,
  Zap,
} from "lucide-react";
import "./App.css";
import "./components/Dashboard.css";
import {
  FIXTURE_RESOURCE_METRICS,
  VFS_FIXTURE_ENTRIES,
  VFS_FIXTURE_STATS,
} from "./fixtures/vfsFixture";

type InputMode = "onion" | "direct" | "mega" | "torrent";

function isOnionTarget(input: string): boolean {
  const value = input.trim().toLowerCase();
  if (!value) return false;
  try {
    const url = new URL(value);
    return url.hostname.endsWith(".onion");
  } catch {
    return value.includes(".onion");
  }
}

function classifyTargetInputMode(input: string): InputMode {
  const val = input.trim();
  if (val.includes("mega.nz/") || val.includes("mega.co.nz/")) {
    return "mega";
  }
  if (val.toLowerCase().startsWith("magnet:?")) {
    return "torrent";
  }
  if (isOnionTarget(val)) {
    return "onion";
  }
  return "direct";
}

function isEmptyFixture(): boolean {
  if (typeof window === "undefined") return false;
  const params = new URLSearchParams(window.location.search);
  return params.get("fixture") === "empty";
}

function isVfsFixture(): boolean {
  if (typeof window === "undefined") return false;
  const params = new URLSearchParams(window.location.search);
  const fixture = params.get("fixture");
  return fixture === "vfs" || fixture === "download";
}

export default function BrowserPreviewApp() {
  const emptyFixture = isEmptyFixture();
  const vfsFixture = isVfsFixture();
  const [url, setUrl] = useState("");
  const [inputMode, setInputMode] = useState<InputMode>("onion");
  const [outputDir] = useState("OnionForger_Downloads");
  const [crawlOptions, setCrawlOptions] = useState({
    listing: true,
    sizes: true,
    download: false,
    agnosticState: false,
    stealthRamp: true,
  });

  useEffect(() => {
    document.title = "Crawli Engine";
  }, []);

  const logs = useMemo(
    () =>
      emptyFixture
        ? ["[SYSTEM] Browser preview mode ready.", "[SYSTEM] Empty fixture requested."]
        : vfsFixture
          ? [
              "Initializing Kernel Modules...",
              "[SYSTEM] Browser preview mode detected: native backend event streams are disabled.",
              "[SYSTEM] Fixture VFS mode enabled for browser integrity testing.",
            ]
        : [
            "[SYSTEM] Browser preview mode detected: native backend event streams are disabled.",
            "[SYSTEM] Idle browser shell loaded.",
          ],
    [emptyFixture, vfsFixture],
  );

  const stats = emptyFixture || !vfsFixture
    ? { files: 0, folders: 0, size: 0, totalNodes: 0 }
    : VFS_FIXTURE_STATS;

  const resourceMetrics = emptyFixture || !vfsFixture
    ? {
        ...FIXTURE_RESOURCE_METRICS,
        processCpuPercent: 0,
        processMemoryBytes: 0,
        systemMemoryUsedBytes: 0,
        systemMemoryPercent: 0,
        activeWorkers: 0,
        workerTarget: 0,
        activeCircuits: 0,
        peakActiveCircuits: 0,
        currentNodeHost: "unresolved",
      }
    : FIXTURE_RESOURCE_METRICS;

  return (
    <div className="app-container">
      <div className="toast-container">
        <div className="toast toast-slide-in success" style={{ opacity: 0.001, pointerEvents: "none" }}>
          <div className="toast-icon">
            <CheckCircle size={18} />
          </div>
          <div className="toast-content">
            <span className="toast-title">Preview</span>
            <span className="toast-message">Browser fixture shell active</span>
          </div>
        </div>
      </div>

      <header>
        <div className="title">
          <div className="title-icon pulse-line">
            <Globe size={18} />
          </div>
          <div className="title-text">
            <span>Crawli Engine</span>
            <span className="title-sub">Deepweb Content Extractor</span>
          </div>
        </div>
        <div style={{ display: "flex", alignItems: "center", gap: "12px" }}>
          <div className="status-badge" style={{ gap: "6px" }}>
            <Clock size={12} />
            00:00:00
          </div>
          <div className="status-badge ready">
            <Activity size={14} /> SYS: PREVIEW
          </div>
        </div>
      </header>

      <div className="toolbar" data-testid="toolbar">
        <button className="tool-btn" data-testid="btn-load-target" onClick={() => setUrl("http://worldleaks.onion/api/")}>
          <FolderSearch size={22} /> Load Target
        </button>
        <button className="tool-btn" data-testid="btn-resume" disabled={stats.totalNodes === 0}>
          <Play size={22} /> Retry / Resume
        </button>
        <button className="tool-btn danger" data-testid="btn-cancel">
          <XCircle size={22} /> Cancel
        </button>
        <button className="tool-btn" data-testid="btn-export" disabled={stats.totalNodes === 0}>
          <FileJson size={22} /> Export
        </button>
        <button className="tool-btn" data-testid="btn-hex-view" disabled={!url}>
          <HardDrive size={22} /> Native Hex
        </button>
        <button className="tool-btn" data-testid="btn-support">
          <CircleHelp size={22} /> Support
        </button>
        <button className={`tool-btn ${inputMode === "onion" ? "active" : ""}`} data-testid="btn-onion" onClick={() => setInputMode("onion")}>
          <ShieldAlert size={22} /> Tor Node
        </button>
        <button className={`tool-btn ${inputMode === "direct" ? "active" : ""}`} data-testid="btn-direct" onClick={() => setInputMode("direct")}>
          <Globe size={22} /> Direct
        </button>
        <button className={`tool-btn ${inputMode === "mega" ? "active" : ""}`} data-testid="btn-mega" onClick={() => setInputMode("mega")}>
          <Cloud size={22} /> Mega.nz
        </button>
        <button className={`tool-btn ${inputMode === "torrent" ? "active" : ""}`} data-testid="btn-torrent" onClick={() => setInputMode("torrent")}>
          <Magnet size={22} /> Torrent
        </button>
      </div>

      <div className="url-bar">
        <div className="input-group">
          <span className="input-label">
            {inputMode === "mega" ? "MEGA.NZ" : inputMode === "torrent" ? "TORRENT" : inputMode === "direct" ? "DIRECT URL" : "Target Source"}
          </span>
          <input
            data-testid="input-target-url"
            type="text"
            className="url-input"
            placeholder={inputMode === "direct" ? "https://example.com/archive.7z" : "http://... (preview mode)"}
            value={url}
            onChange={(e) => {
              const val = e.target.value;
              setUrl(val);
              setInputMode(classifyTargetInputMode(val));
            }}
          />
        </div>
        <button className="action-btn popup-hover" data-testid="btn-start-queue">
          <span style={{ display: "flex", alignItems: "center", gap: "8px" }}>Start Queue</span>
        </button>
      </div>

      <div className="url-bar" style={{ marginTop: "0", borderTop: "none", paddingTop: "0" }}>
        <div className="input-group">
          <span className="input-label" style={{ minWidth: "100px" }}>Extraction Path</span>
          <input
            data-testid="input-output-path"
            type="text"
            className="url-input"
            style={{ fontFamily: "JetBrains Mono", fontSize: "0.85rem" }}
            readOnly
            value={outputDir}
          />
        </div>
        <button className="action-btn popup-hover" data-testid="btn-change-output" style={{ width: "auto", padding: "0 24px", background: "transparent", border: "1px solid rgba(162, 0, 255, 0.4)" }}>
          <Save size={18} style={{ color: "var(--accent-primary)" }} /> Change
        </button>
      </div>

      <div className="options-bar" style={{ display: "flex", gap: "32px", padding: "0 24px 16px", borderBottom: "var(--panel-border)" }}>
        <label style={{ display: "flex", alignItems: "center", gap: "8px", cursor: "pointer" }}>
          <input data-testid="chk-listing" type="checkbox" checked={crawlOptions.listing} onChange={(e) => setCrawlOptions((prev) => ({ ...prev, listing: e.target.checked }))} />
          <span style={{ fontSize: "0.85rem" }}>Index Framework (Files)</span>
        </label>
        <label style={{ display: "flex", alignItems: "center", gap: "8px", cursor: "pointer" }}>
          <input data-testid="chk-sizes" type="checkbox" checked={crawlOptions.sizes} onChange={(e) => setCrawlOptions((prev) => ({ ...prev, sizes: e.target.checked }))} />
          <span style={{ fontSize: "0.85rem" }}>Map File Sizes</span>
        </label>
        <label style={{ display: "flex", alignItems: "center", gap: "8px", cursor: "pointer" }}>
          <input data-testid="chk-auto-download" type="checkbox" checked={crawlOptions.download} onChange={(e) => setCrawlOptions((prev) => ({ ...prev, download: e.target.checked }))} />
          <span style={{ fontSize: "0.85rem" }}>Auto-Download During Crawl</span>
        </label>
        <label style={{ display: "flex", alignItems: "center", gap: "8px", cursor: "pointer" }}>
          <input data-testid="chk-agnostic-state" type="checkbox" checked={crawlOptions.agnosticState} onChange={(e) => setCrawlOptions((prev) => ({ ...prev, agnosticState: e.target.checked }))} />
          <span style={{ fontSize: "0.85rem" }}>URI-Agnostic State</span>
        </label>
        <label style={{ display: "flex", alignItems: "center", gap: "8px", cursor: "pointer" }}>
          <input data-testid="chk-stealth-ramp" type="checkbox" checked={crawlOptions.stealthRamp} onChange={(e) => setCrawlOptions((prev) => ({ ...prev, stealthRamp: e.target.checked }))} />
          <span style={{ fontSize: "0.85rem" }}>Vanguard Stealth Ramp</span>
        </label>
      </div>

      <div className="ops-dashboard">
        <div className="dash-card" style={{ flex: "0 0 240px" }}>
          <div className="dash-icon"><Database size={24} /></div>
          <div className="dash-info">
            <div className="dash-title">NODES INDEXED</div>
            <div className="dash-value" style={{ fontFamily: "JetBrains Mono" }}>{stats.totalNodes.toLocaleString()}</div>
            <div className="dash-sub">{stats.folders} dirs | {stats.files} files</div>
          </div>
        </div>

        <div
          className="dash-card resource-card"
          data-testid="resource-metrics-card"
          style={{ flex: "0 0 387px", width: "387px", maxWidth: "387px" }}
        >
          <div className="dash-icon"><Cpu size={24} /></div>
          <div className="dash-info">
            <div className="dash-title">PROCESS + SYSTEM</div>
            <div className="dash-value" data-testid="resource-process-cpu">CPU {resourceMetrics.processCpuPercent.toFixed(1)}%</div>
            <div className="dash-sub" data-testid="resource-process-memory" style={{ fontFamily: "JetBrains Mono" }}>
              RSS {(resourceMetrics.processMemoryBytes / 1048576).toFixed(1)} MB | Threads {resourceMetrics.processThreads}
            </div>
            <div className="dash-sub" data-testid="resource-worker-metrics" style={{ fontFamily: "JetBrains Mono" }}>
              <span style={{ color: "var(--accent-primary)" }}>Vanguard: Active (Heatmap Enabled) | Circuits {resourceMetrics.activeCircuits}/{resourceMetrics.peakActiveCircuits}</span>
            </div>
            <div className="dash-sub" data-testid="resource-node-metrics" style={{ fontFamily: "JetBrains Mono" }}>
              Node {resourceMetrics.currentNodeHost || "unresolved"} | Multi-Client Rotations 0 (Pool: 0) | 429/503 {resourceMetrics.throttleCount} | Timeouts {resourceMetrics.timeoutCount}
            </div>
            <div className="dash-sub" style={{ fontFamily: "JetBrains Mono" }}>
              RAM {resourceMetrics.systemMemoryPercent.toFixed(1)}%
            </div>
          </div>
        </div>
      </div>

      <div className="main-workspace">
        <div className="panel" style={{ flex: 1 }}>
          <div className="panel-header">
            <span style={{ display: "flex", alignItems: "center", gap: "6px" }}>
              <Terminal size={14} /> Forensic Log
            </span>
          </div>
          <div className="panel-content">
            <div className="forensic-log">
              {logs.map((log, i) => (
                <div key={i} className="term-line">
                  <span className="term-prefix">{String(i).padStart(4, "0")}</span>
                  {log}
                </div>
              ))}
            </div>
          </div>
        </div>

        <div className="panel" style={{ flex: 1.5, position: "relative" }}>
          <div className="panel-header">
            <span style={{ display: "flex", alignItems: "center", gap: "6px" }}>
              <ListTree size={14} /> Virtual File System
            </span>
            <span style={{ fontSize: "0.8rem", color: "var(--accent-secondary)", background: "rgba(0, 229, 255, 0.1)", padding: "2px 8px", borderRadius: "12px", border: "1px solid rgba(0, 229, 255, 0.3)" }}>
              {stats.totalNodes.toLocaleString()} Nodes
            </span>
          </div>
          <div className="panel-content" style={{ padding: 0 }}>
            <div className="vfs-container" style={{ height: "100%", overflow: "auto" }}>
              {!vfsFixture || emptyFixture ? (
                <div style={{ height: "100%", display: "flex", alignItems: "center", justifyContent: "center", color: "var(--text-muted)" }}>
                  No files detected in virtual file system.
                </div>
              ) : (
                VFS_FIXTURE_ENTRIES.map((entry) => (
                  <div key={entry.path} className="vfs-row" data-testid={`vfs-row-${encodeURIComponent(entry.path)}`} style={{ minHeight: "36px", paddingLeft: `${entry.path.split("/").length * 18}px` }}>
                    <div className="vfs-icon" style={{ marginLeft: "12px" }}>
                      <ListTree size={14} color={entry.entry_type === "Folder" ? "var(--accent-primary)" : "var(--text-muted)"} />
                    </div>
                    <span className="vfs-name">{entry.path.split("/").pop() || entry.path}</span>
                    <div style={{ flex: 1, display: "flex", justifyContent: "flex-end", paddingRight: "12px" }}>
                      <span className="vfs-size">
                        {entry.size_bytes == null ? "--" : entry.size_bytes.toLocaleString()}
                      </span>
                    </div>
                  </div>
                ))
              )}
            </div>
          </div>
        </div>
      </div>

      <div className="network-monitor">
        {Array.from({ length: 4 }).map((_, idx) => (
          <div key={idx} className="daemon-box">
            <div className="daemon-icon">
              <Zap size={18} />
            </div>
            <div className="daemon-info">
              <div className="daemon-header">ARTI NODE {idx + 1}</div>
              <div className="daemon-body">
                <span style={{ color: "var(--text-muted)" }}>STANDBY</span>
                <span style={{ fontSize: "0.8rem", color: "var(--text-muted)", fontFamily: "JetBrains Mono" }}>---</span>
              </div>
            </div>
          </div>
        ))}
      </div>

      <div className="toast-container" style={{ opacity: 0.001, pointerEvents: "none" }}>
        <div className="toast error">
          <div className="toast-icon">
            <AlertCircle size={18} />
          </div>
          <div className="toast-content">
            <span className="toast-title">Preview</span>
            <span className="toast-message">Native actions are disabled in browser mode.</span>
          </div>
        </div>
      </div>
    </div>
  );
}
