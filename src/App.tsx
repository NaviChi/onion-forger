import { useState, useEffect, useCallback, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { readText } from "@tauri-apps/plugin-clipboard-manager";
import { VFSExplorer, FileEntry } from "./components/VFSExplorer";
import { Dashboard } from "./components/Dashboard";
import { downloadDir, join } from "@tauri-apps/api/path";
import { open, save } from "@tauri-apps/plugin-dialog";
import { Zap, Play, Activity, FolderSearch, Globe, ListTree, Terminal, CheckCircle, AlertCircle, Save, Download, FileJson, Clock, XCircle } from "lucide-react";

import "./App.css";


interface DownloadProgressEvent {
  path: string;
  bytes_downloaded: number;
  total_bytes: number | null;
  speed_bps: number;
  active_circuits?: number;
}

interface TorStatus {
  state: string;
  message: string;
  daemon_count: number;
  ports?: number[];
}

interface ToastInfo {
  id: number;
  type: "success" | "error";
  title: string;
  message: string;
}



// Kept dummy function so lines don't shift too much

function formatDuration(ms: number): string {
  const totalSec = Math.floor(ms / 1000);
  const min = Math.floor(totalSec / 60);
  const sec = totalSec % 60;
  if (min > 0) return `${min}m ${sec}s`;
  return `${sec}s`;
}

function App() {
  const isTauriRuntime = typeof (window as any).__TAURI_INTERNALS__ !== "undefined";
  const [url, setUrl] = useState("");
  const [isCrawling, setIsCrawling] = useState(false);
  const [vfsStats, setVfsStats] = useState({ files: 0, folders: 0, size: 0, totalNodes: 0 });
  const [vfsRefreshTrigger, setVfsRefreshTrigger] = useState(0);
  const [logs, setLogs] = useState<string[]>([
    "Initializing Kernel Modules...",
    "[SYSTEM] Local Tor Daemon initialized on 127.0.0.1:9051",
    "[SYSTEM] Adapter Registry loaded (WorldLeaks, DragonForce, LockBit, INC Ransom, Pear, Play, Autoindex)",
  ]);
  const [torStatus, setTorStatus] = useState<TorStatus | null>(null);

  const [downloadProgress, setDownloadProgress] = useState<Record<string, DownloadProgressEvent>>({});
  const [selectedFiles, setSelectedFiles] = useState<FileEntry[]>([]);
  const [toasts, setToasts] = useState<ToastInfo[]>([]);
  const [lastClipboard, setLastClipboard] = useState("");
  const [outputDir, setOutputDir] = useState("");
  const [daemonPorts, setDaemonPorts] = useState<number[]>([9051, 9052, 9053, 9054]);
  const [crawlStartTime, setCrawlStartTime] = useState<number | null>(null);
  const [crawlElapsed, setCrawlElapsed] = useState(0);

  const urlInputRef = useRef<HTMLInputElement>(null);
  const previewNoticeShownRef = useRef(false);

  const [crawlOptions, setCrawlOptions] = useState({
    listing: true,
    sizes: true,
    download: false,
    circuits: 120
  });

  const showToast = (type: "success" | "error", title: string, message: string) => {
    const id = Date.now();
    setToasts((prev) => [...prev, { id, type, title, message }]);
    setTimeout(() => {
      setToasts((prev) => prev.filter((t) => t.id !== id));
    }, 6000);
  };

  // Crawl duration timer
  useEffect(() => {
    if (!isCrawling || !crawlStartTime) return;
    const interval = setInterval(() => {
      setCrawlElapsed(Date.now() - crawlStartTime);
    }, 1000);
    return () => clearInterval(interval);
  }, [isCrawling, crawlStartTime]);

  // Keyboard shortcuts: ⌘+Enter to start, Esc to stop, ⌘+E to export
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      const isMeta = e.metaKey || e.ctrlKey;

      if (e.key === "Enter" && isMeta && !isCrawling && url) {
        e.preventDefault();
        handleCrawl();
      }
      if (e.key === "Escape" && isCrawling) {
        e.preventDefault();
        handleCancelCrawl();
      }
      if (e.key === "e" && isMeta && vfsStats.totalNodes > 0) {
        e.preventDefault();
        handleExportJSON();
      }
      // ⌘+V focus URL input
      if (e.key === "v" && isMeta) {
        urlInputRef.current?.focus();
      }
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [isCrawling, url, vfsStats]);

  useEffect(() => {
    async function initPaths() {
      if (!isTauriRuntime) {
        setOutputDir((prev) => prev || "OnionForger_Downloads");
        return;
      }
      try {
        const dl = await downloadDir();
        const defaultPath = await join(dl, "OnionForger_Downloads");
        setOutputDir(defaultPath);
      } catch (e) {
        console.warn("Could not retrieve download directory", e);
      }
    }
    initPaths();

    const checkClipboard = async () => {
      if (!isTauriRuntime) return;
      if (!isCrawling && !url) {
        try {
          const text = await readText();
          if (text && text.includes(".onion") && text !== lastClipboard) {
            setLastClipboard(text);
            setUrl(text);
            showToast("success", "Clipboard Auto-Detect", "Pasted new .onion link securely.");
            setLogs((l) => [...l.slice(-399), `[SYSTEM] Auto-pasted Tor link from clipboard.`]);
          }
        } catch (e) {
          // Ignore failures gracefully
        }
      }
    };

    const clipboardInterval = setInterval(checkClipboard, 2000);
    const unlistenPromises: Array<Promise<() => void>> = [];

    if (isTauriRuntime) {
      unlistenPromises.push(
        listen<string>("crawl_log", (event) => {
          setLogs((l) => [...l.slice(-399), `> ${event.payload}`]);
        })
      );

      unlistenPromises.push(
        listen<FileEntry[]>("crawl_progress", (event) => {
          // Stream directly to backend DB
          invoke("ingest_vfs_entries", { entries: event.payload }).catch(console.error);

          let newFiles = 0;
          let newFolders = 0;
          let newSize = 0;

          event.payload.forEach((e) => {
            if (e.entry_type === "File") newFiles++;
            else newFolders++;
            if (e.size_bytes) newSize += e.size_bytes;
          });

          setVfsStats((prev) => {
            const next = {
              files: prev.files + newFiles,
              folders: prev.folders + newFolders,
              size: prev.size + newSize,
              totalNodes: prev.totalNodes + event.payload.length,
            };

            return next;
          });

          // Throttle UI refreshes for root nodes to prevent flickering
          setVfsRefreshTrigger(Date.now());
        })
      );

      unlistenPromises.push(
        listen<TorStatus>("tor_status", (event) => {
          setTorStatus(event.payload);
          if (event.payload.ports && event.payload.ports.length > 0) {
            setDaemonPorts(event.payload.ports);
          }
          setLogs((l) => [...l.slice(-399), `[TOR] ${event.payload.state.toUpperCase()}: ${event.payload.message}`]);
        })
      );

      unlistenPromises.push(
        listen<DownloadProgressEvent>("download_progress_update", (event) => {
          let relativePath = event.payload.path;
          // Convert absolute `targetPath` from aria_downloader back to relative `node.id`
          if (typeof outputDir === "string" && relativePath.startsWith(outputDir)) {
            relativePath = relativePath.substring(outputDir.length);
          }
          // Ensure leading slashes are stripped since `node.id` doesn't have them
          relativePath = relativePath.replace(/^[/\\]+/, "");

          setDownloadProgress((prev) => ({
            ...prev,
            [relativePath]: event.payload,
            [event.payload.path]: event.payload, // Fallback
          }));
        })
      );

      unlistenPromises.push(
        listen<{ url: string; path: string; hash: string; time_taken_secs: number }>("complete", (event) => {
          let relativePath = event.payload.path;
          if (typeof outputDir === "string" && relativePath.startsWith(outputDir)) {
            relativePath = relativePath.substring(outputDir.length);
          }
          relativePath = relativePath.replace(/^[/\\]+/, "");

          setLogs((l) => [...l.slice(-399), `[✓] Download finished: ${relativePath} (SHA256: ${event.payload.hash})`]);
          showToast("success", "Download Finished", `File saved and verified (${event.payload.hash})`);
          setDownloadProgress((prev) => {
            const p = prev[relativePath] || prev[event.payload.path];
            if (!p) return prev;
            return {
              ...prev,
              [relativePath]: { ...p, bytes_downloaded: p.total_bytes || p.bytes_downloaded, speed_bps: 0 },
              [event.payload.path]: { ...p, bytes_downloaded: p.total_bytes || p.bytes_downloaded, speed_bps: 0 },
            };
          });
        })
      );

      unlistenPromises.push(
        listen<{ url: string; path: string; error: string }>("download_failed", (event) => {
          setLogs((l) => [...l.slice(-399), `[ERROR] Download failed for ${event.payload.path}: ${event.payload.error}`]);
          showToast("error", "Download Failed", event.payload.error);
        })
      );
    } else if (!previewNoticeShownRef.current) {
      previewNoticeShownRef.current = true;
      setLogs((l) => [
        ...l.slice(-399),
        "[SYSTEM] Browser preview mode detected: native backend event streams are disabled.",
      ]);
    }

    return () => {
      clearInterval(clipboardInterval);
      unlistenPromises.forEach((p) => {
        p.then((f) => f()).catch(() => undefined);
      });
    };
  }, [isCrawling, url, lastClipboard, outputDir, isTauriRuntime]);

  useEffect(() => {
    const logContainer = document.querySelector('.forensic-log');
    if (logContainer) logContainer.scrollTop = logContainer.scrollHeight;
  }, [logs]);

  const handleCrawl = useCallback(async () => {
    if (!url) return;
    setIsCrawling(true);

    setCrawlStartTime(Date.now());
    setCrawlElapsed(0);
    setLogs((l) => [...l, `--- Initiating Crawl ---`]);
    setLogs((l) => [...l, `> Probing Target: ${url}`]);
    setVfsStats({ files: 0, folders: 0, size: 0, totalNodes: 0 });
    setVfsRefreshTrigger(Date.now());

    try {
      if (typeof (window as any).__TAURI_INTERNALS__ === 'undefined') {
        throw new Error("Execution Environment Mismatch: Not running in native Tauri container.");
      }

      const files = await invoke<FileEntry[]>("start_crawl", { url, options: crawlOptions, outputDir });
      setLogs((l) => [...l, `[SYSTEM] Finish signaled. Found ${files.length} unique nodes.`]);
      showToast("success", "Crawl Finished", `Operations complete. Extracted ${files.length} nodes from source.`);

      if (crawlOptions.download) {
        setLogs((l) => [...l, `[OPSEC] Auto-Mirror complete. Files scaffolded to ${outputDir}`]);
      }
    } catch (err: any) {
      if (err instanceof TypeError && err.message.includes('invoke')) {
        setLogs((l) => [...l, `[ERROR] Execution Environment Mismatch: Cannot execute backend tasks in standard browser.`]);
        showToast("error", "Environment Error", "Cannot run crawler from standard browser. Please test in the native app window.");
      } else if (String(err).includes("OFFLINE_SYNC_ERROR")) {
        showToast("error", "Target Offline", "Please manually check the website to verify if it is actually functional and active.");
        setLogs((l) => [...l, `[ERROR] Target site is unreachable or offline. Manual verification required.`]);
      } else {
        setLogs((l) => [...l, `[ERROR] ${err.message || err}`]);
        showToast("error", "Task Failed", String(err.message || err));
      }
    } finally {
      setIsCrawling(false);
      setCrawlStartTime(null);
    }
  }, [url, crawlOptions, outputDir]);

  const handleCancelCrawl = async () => {
    try {
      if (typeof (window as any).__TAURI_INTERNALS__ === 'undefined') {
        throw new Error("Execution Environment Mismatch: Not running in native Tauri container.");
      }
      await invoke<string>("cancel_crawl");
      setLogs((l) => [...l, `[SYSTEM] ⚠ Cancellation requested — workers will finish current task and stop.`]);
      showToast("error", "Crawl Cancelled", "Workers are stopping. Current tasks will complete.");
    } catch (err: any) {
      setLogs((l) => [...l, `[ERROR] Cancel failed: ${err.message || err}`]);
    }
  };

  const handleDownload = async (rawUrl: string, filePath: string) => {
    setLogs((l) => [...l, `> Requesting Download: ${filePath}`]);
    try {
      if (filePath.endsWith('/')) {
        const entry: FileEntry = {
          path: filePath,
          size_bytes: null,
          entry_type: 'Folder',
          raw_url: rawUrl
        };
        const count = await invoke<number>("download_files", { entries: [entry], outputDir });
        showToast("success", "Download Complete", `${count} item(s) saved to ${outputDir}`);
        setLogs((l) => [...l, `[MIRROR] Saved ${filePath} to disk`]);
      } else {
        // High concurrency chunked download for single files
        let targetPath = outputDir.endsWith('/') || outputDir.endsWith('\\') ? `${outputDir}${filePath}` : `${outputDir}/${filePath}`;
        await invoke("initiate_download", {
          args: {
            url: rawUrl,
            path: targetPath,
            connections: 120, // 120 isolated circuits
            force_tor: true
          }
        });
        showToast("success", "Download Engine Started", `Allocating 120 Tor circuits to target...`);
      }
    } catch (err: any) {
      setLogs((l) => [...l, `[ERROR] Download failed: ${err}`]);
      showToast("error", "Download Error", String(err));
    }
  };

  const handleDownloadSelected = async () => {
    if (selectedFiles.length === 0) return;
    setLogs((l) => [...l, `[OPSEC] Manual Mirror: Scaffolding ${selectedFiles.length} selected nodes to ${outputDir}...`]);
    try {
      const count = await invoke<number>("download_files", { entries: selectedFiles, outputDir });
      showToast("success", "Mirror Complete", `${count} items written to ${outputDir}`);
      setLogs((l) => [...l, `[MIRROR] Complete. ${count}/${selectedFiles.length} selected items on disk.`]);
    } catch (err: any) {
      setLogs((l) => [...l, `[ERROR] Mirror failed: ${err}`]);
      showToast("error", "Mirror Error", String(err));
    }
  };

  const handleExportJSON = async () => {
    if (vfsStats.totalNodes === 0) return;
    try {
      const savePath = await save({
        defaultPath: "crawl_results.json",
        filters: [{ name: "JSON", extensions: ["json"] }],
        title: "Export Crawl Results",
      });
      if (!savePath) return;

      const result = await invoke<string>("export_json", { outputPath: savePath });
      showToast("success", "Export Complete", result);
      setLogs((l) => [...l, `[EXPORT] Successfully saved map to ${savePath}`]);
    } catch (err: any) {
      setLogs((l) => [...l, `[ERROR] Export failed: ${err}`]);
      showToast("error", "Export Error", String(err));
    }
  };

  const handleDownloadAll = async () => {
    if (vfsStats.totalNodes === 0) return;
    setLogs((l) => [...l, `[OPSEC] Mass Mirror: Querying VFS and scaffolding full dataset to ${outputDir}...`]);
    try {
      showToast("success", "Scaffolding Started", `Extracting entire VFS structure to primary disk...`);
      const count = await invoke<number>("download_all", { outputDir });
      showToast("success", "Mirror Complete", `${count} total items structured on disk.`);
      setLogs((l) => [...l, `[MIRROR] Complete. ${count} total items on disk.`]);
    } catch (err: any) {
      setLogs((l) => [...l, `[ERROR] Mass Mirror failed: ${err}`]);
      showToast("error", "Mirror Error", String(err));
    }
  };



  const handleSelectOutput = async () => {
    try {
      const selected = await open({
        directory: true,
        multiple: false,
        title: "Select Download Location",
      });
      if (selected && typeof selected === "string") {
        setOutputDir(selected);
        showToast("success", "Storage Linked", "Updated target extraction path.");
      }
    } catch (e) {
      console.warn("Failed to open dialog", e);
    }
  };

  const totalSizeStr = (() => {
    if (vfsStats.size === 0) return "0 B";
    if (vfsStats.size >= 1_073_741_824) return (vfsStats.size / 1_073_741_824).toFixed(2) + " GB";
    if (vfsStats.size >= 1_048_576) return (vfsStats.size / 1_048_576).toFixed(2) + " MB";
    if (vfsStats.size >= 1024) return (vfsStats.size / 1024).toFixed(2) + " KB";
    return vfsStats.size + " B";
  })();

  return (
    <div className="app-container">
      {/* Toast Manager Overlay */}
      <div className="toast-container">
        {toasts.map(t => (
          <div key={t.id} className={`toast toast-slide-in ${t.type}`}>
            <div className="toast-icon">
              {t.type === "success" ? <CheckCircle size={18} /> : <AlertCircle size={18} />}
            </div>
            <div className="toast-content">
              <span className="toast-title">{t.title}</span>
              <span className="toast-message">{t.message}</span>
            </div>
          </div>
        ))}
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
        <div style={{ display: 'flex', alignItems: 'center', gap: '12px' }}>
          {isCrawling && crawlStartTime && (
            <div className="status-badge" style={{ gap: '6px' }}>
              <Clock size={12} />
              {formatDuration(crawlElapsed)}
            </div>
          )}
          <div className={`status-badge ${torStatus?.state === 'ready' || torStatus?.state === 'active' ? 'ready' : torStatus ? 'warn' : ''}`}>
            <Activity size={14} className={isCrawling ? "aura-spin" : ""} /> SYS: {torStatus ? torStatus.state.toUpperCase() : "IDLE"}
          </div>
        </div>
      </header>

      <div className="toolbar">
        <button className="tool-btn" onClick={() => setUrl("http://worldleaks.onion/api/")}>
          <FolderSearch size={22} className={url.includes("worldleaks") ? "pulse-line text-accent-primary" : ""} /> Load Target
        </button>
        <button className="tool-btn" onClick={handleCrawl} disabled={isCrawling}>
          <Play size={22} /> Resume
        </button>
        <button
          className="tool-btn danger"
          onClick={handleCancelCrawl}
          disabled={!isCrawling}
          title="Stop crawl (Esc)"
        >
          <XCircle size={22} /> Cancel
        </button>
        <button
          className="tool-btn"
          onClick={handleExportJSON}
          disabled={vfsStats.totalNodes === 0}
          title="Export JSON (⌘+E)"
        >
          <FileJson size={22} /> Export
        </button>
      </div>

      <div className="url-bar">
        <div className="input-group">
          <span className="input-label">Target Source</span>
          <input
            ref={urlInputRef}
            type="text"
            className="url-input"
            placeholder="http://... (⌘+Enter to start)"
            value={url}
            onChange={(e) => setUrl(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') handleCrawl();
            }}
            disabled={isCrawling}
          />
        </div>

        <button
          className="action-btn popup-hover"
          onClick={handleCrawl}
          disabled={isCrawling}
        >
          {isCrawling ? (
            <span style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
              <Activity size={18} className="aura-spin" /> Scanning
            </span>
          ) : (
            <span style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>Start Queue</span>
          )}
        </button>
      </div>

      <div className="url-bar" style={{ marginTop: '0', borderTop: 'none', paddingTop: '0' }}>
        <div className="input-group">
          <span className="input-label" style={{ minWidth: '100px' }}>Extraction Path</span>
          <input
            type="text"
            className="url-input"
            style={{ fontFamily: 'JetBrains Mono', fontSize: '0.85rem' }}
            readOnly
            value={outputDir}
          />
        </div>
        <button
          className="action-btn popup-hover"
          onClick={handleSelectOutput}
          style={{ width: 'auto', padding: '0 24px', background: 'transparent', border: '1px solid rgba(162, 0, 255, 0.4)' }}
          disabled={isCrawling}
        >
          <Save size={18} style={{ color: "var(--accent-primary)" }} /> Change
        </button>
      </div>

      <div className="options-bar" style={{ display: 'flex', gap: '32px', padding: '0 24px 16px', borderBottom: 'var(--panel-border)' }}>
        <label style={{ display: 'flex', alignItems: 'center', gap: '8px', cursor: 'pointer' }}>
          <input
            type="checkbox"
            checked={crawlOptions.listing}
            onChange={(e) => setCrawlOptions({ ...crawlOptions, listing: e.target.checked })}
            style={{ accentColor: 'var(--accent-primary)', width: '16px', height: '16px' }}
            disabled={isCrawling}
          />
          <span style={{ fontSize: '0.85rem', color: crawlOptions.listing ? 'var(--text-main)' : 'var(--text-muted)' }}>Index Framework (Files)</span>
        </label>

        <label style={{ display: 'flex', alignItems: 'center', gap: '8px', cursor: 'pointer' }}>
          <input
            type="checkbox"
            checked={crawlOptions.sizes}
            onChange={(e) => setCrawlOptions({ ...crawlOptions, sizes: e.target.checked })}
            style={{ accentColor: 'var(--accent-primary)', width: '16px', height: '16px' }}
            disabled={isCrawling}
          />
          <span style={{ fontSize: '0.85rem', color: crawlOptions.sizes ? 'var(--text-main)' : 'var(--text-muted)' }}>Map File Sizes</span>
        </label>

        <label style={{ display: 'flex', alignItems: 'center', gap: '8px', cursor: 'pointer' }} title="Automatically download files to disk as soon as they are found during the crawl.">
          <input
            type="checkbox"
            checked={crawlOptions.download}
            onChange={(e) => setCrawlOptions({ ...crawlOptions, download: e.target.checked })}
            style={{ accentColor: 'var(--accent-primary)', width: '16px', height: '16px' }}
            disabled={isCrawling}
          />
          <span style={{ fontSize: '0.85rem', color: crawlOptions.download ? 'var(--text-main)' : 'var(--text-muted)' }}>Auto-Download During Crawl</span>
        </label>

        <div style={{ display: 'flex', alignItems: 'center', gap: '8px', marginLeft: 'auto' }}>
          <span style={{ fontSize: '0.85rem', color: 'var(--text-muted)' }}>Concurrency:</span>
          <select
            value={crawlOptions.circuits}
            onChange={(e) => setCrawlOptions({ ...crawlOptions, circuits: parseInt(e.target.value) })}
            disabled={isCrawling}
            style={{
              background: 'var(--bg-dark)',
              color: 'var(--text-main)',
              border: '1px solid var(--border-color)',
              borderRadius: '4px',
              padding: '4px 8px',
              fontSize: '0.85rem',
              outline: 'none',
              cursor: isCrawling ? 'not-allowed' : 'pointer'
            }}
          >
            <option value={40}>40 Circuits</option>
            <option value={120}>120 Circuits (Default)</option>
            <option value={150}>150 Circuits</option>
            <option value={200}>200 Circuits</option>
          </select>
        </div>
      </div>

      <Dashboard
        isCrawling={isCrawling}
        torStatus={torStatus}
        logs={logs}
        vfsCount={vfsStats.totalNodes}
        downloadProgress={downloadProgress}
        elapsed={crawlElapsed}
      />

      <div className="main-workspace">
        <div className="panel" style={{ flex: 1 }}>
          <div className="panel-header">
            <span style={{ display: 'flex', alignItems: 'center', gap: '6px' }}>
              <Terminal size={14} /> Forensic Log
            </span>
          </div>
          <div className="panel-content">
            <div className="forensic-log">
              {logs.map((log, i) => (
                <div key={i} className="term-line" style={{
                  color: log.includes("ERROR") ? "#EF4444" :
                    log.includes("FOUND") ? "var(--accent-primary)" :
                      log.includes("Match found") ? "var(--accent-primary)" :
                        log.includes("Target") ? "var(--accent-secondary)" :
                          log.includes("TOR") ? "#a78bfa" :
                            log.includes("MIRROR") ? "#10B981" :
                              log.includes("EXPORT") ? "#60A5FA" :
                                log.includes("⚠") ? "#fbbf24" :
                                  "var(--text-main)"
                }}>
                  <span className="term-prefix">{String(i).padStart(4, '0')}</span>
                  {log}
                </div>
              ))}
            </div>
          </div>
        </div>

        <div className="panel" style={{ flex: 1.5, position: 'relative' }}>
          <div className="panel-header">
            <span style={{ display: 'flex', alignItems: 'center', gap: '6px' }}>
              <ListTree size={14} /> Virtual File System
            </span>

            <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
              {vfsStats.totalNodes > 0 && (
                <span style={{
                  fontSize: "0.75rem",
                  color: "var(--text-muted)",
                  fontFamily: "JetBrains Mono",
                }}>
                  {vfsStats.folders} dirs · {vfsStats.files} files · {totalSizeStr}
                </span>
              )}
              <span style={{ fontSize: "0.8rem", color: "var(--accent-secondary)", background: "rgba(0, 229, 255, 0.1)", padding: "2px 8px", borderRadius: "12px", border: "1px solid rgba(0, 229, 255, 0.3)" }}>
                {vfsStats.totalNodes.toLocaleString()} Nodes
              </span>
              {vfsStats.totalNodes > 0 && !isCrawling && (
                <>
                  <button
                    className="action-btn popup-hover"
                    onClick={handleDownloadAll}
                    style={{ padding: '2px 12px', fontSize: '0.75rem', height: '28px', minWidth: 'auto', background: 'transparent', border: '1px solid var(--border-hud)', color: 'var(--accent-secondary)', display: 'flex', gap: '6px', alignItems: 'center' }}
                    title="Safely Scaffold All Indexed Entries via Multi-Threading"
                  >
                    <Download size={12} /> Mass Extract All
                  </button>
                  {selectedFiles.length > 0 && (
                    <button
                      className="action-btn popup-hover"
                      onClick={handleDownloadSelected}
                      style={{ padding: '2px 12px', fontSize: '0.75rem', height: '28px', minWidth: 'auto', background: 'rgba(0, 229, 255, 0.1)', border: '1px solid var(--border-hud)', color: 'var(--accent-secondary)', display: 'flex', gap: '6px', alignItems: 'center' }}
                      title="Download selected items."
                    >
                      <Download size={12} /> Download Selected ({selectedFiles.length})
                    </button>
                  )}
                </>
              )}
            </div>
          </div>
          <div className="panel-content" style={{ padding: 0 }}>
            <VFSExplorer
              triggerRefresh={vfsRefreshTrigger}
              onDownload={handleDownload}
              onSelectionChange={setSelectedFiles}
              downloadProgress={downloadProgress}
            />
          </div>
        </div>
      </div>

      <div className="network-monitor">
        {daemonPorts.map((port, idx) => (
          <div key={port} className="daemon-box">
            <div className={`daemon-icon ${isCrawling ? 'aura-spin' : ''}`}>
              <Zap size={18} />
            </div>
            <div className="daemon-info">
              <div className="daemon-header">NODE {idx}: PORT {port}</div>
              <div className="daemon-body">
                <span style={{ color: isCrawling ? 'var(--accent-primary)' : 'var(--text-muted)' }}>
                  {isCrawling ? 'ACTIVE' : 'STANDBY'}
                </span>
                <span style={{ fontSize: '0.8rem', color: 'var(--text-muted)', fontFamily: 'JetBrains Mono' }}>
                  {isCrawling ? Math.floor(Math.random() * 50 + 150) + 'ms' : '---'}
                </span>
              </div>
            </div>
          </div>
        ))}
      </div>
    </div >
  );
}

export default App;
