import { useState, useEffect, useCallback, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { VFSExplorer, FileEntry } from "./components/VFSExplorer";
import { Dashboard } from "./components/Dashboard";
import { VibeLoader } from "./components/VibeLoader";
import { downloadDir, join } from "@tauri-apps/api/path";
import { open, save } from "@tauri-apps/plugin-dialog";
import { Zap, Play, Activity, FolderSearch, Globe, ListTree, Terminal, CheckCircle, AlertCircle, Save, Download, FileJson, Clock, XCircle, CircleHelp } from "lucide-react";
import { VFS_FIXTURE_STATS, isVfsFixtureMode } from "./fixtures/vfsFixture";

import "./App.css";


interface DownloadProgressEvent {
  path: string;
  bytes_downloaded: number;
  total_bytes: number | null;
  speed_bps: number;
  active_circuits?: number;
}

interface CrawlStatusEvent {
  phase: string;
  progressPercent: number;
  visitedNodes: number;
  processedNodes: number;
  queuedNodes: number;
  activeWorkers: number;
  workerTarget: number;
  etaSeconds: number | null;
  estimation: string;
}

interface DownloadBatchStartedEvent {
  totalFiles: number;
  totalBytesHint: number;
  unknownSizeFiles: number;
  outputDir: string;
}

interface BatchProgressEvent {
  completed: number;
  failed?: number;
  total: number;
  currentFile: string;
  speedMbps?: number;
  downloadedBytes?: number;
  bbrBottleneckMbps?: number;
  ekfCovariance?: number;
}

interface DownloadBatchStatus {
  totalFiles: number;
  completedFiles: number;
  failedFiles: number;
  totalBytesHint: number;
  unknownSizeFiles: number;
  currentFile: string;
  speedMbps: number;
  downloadedBytes: number;
  bbrBottleneckMbps: number;
  ekfCovariance: number;
  startedAt: number | null;
  etaSeconds: number | null;
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

interface AdapterSupportInfo {
  id: string;
  name: string;
  supportLevel: string;
  matchingStrategy: string;
  sampleUrls: string[];
  testedFor: string[];
  notes: string;
}

const FALLBACK_SUPPORT_CATALOG: AdapterSupportInfo[] = [
  {
    id: "worldleaks",
    name: "WorldLeaks SPA",
    supportLevel: "Full Crawl",
    matchingStrategy: "Known-domain and SPA fingerprint matching",
    sampleUrls: ["http://worldleaks.onion"],
    testedFor: ["Adapter fingerprint match (engine_test)"],
    notes: "Production adapter with crawl traversal and progress streaming.",
  },
  {
    id: "dragonforce",
    name: "DragonForce Iframe SPA",
    supportLevel: "Full Crawl",
    matchingStrategy: "Known-domain and body marker matching",
    sampleUrls: [
      "http://dragonforce.onion",
      "fsguestuctexqqaoxuahuydfa6ovxuhtng66pgyr5gqcrsi7qgchpkad.onion",
    ],
    testedFor: [
      "Adapter fingerprint match (engine_test)",
      "Parser extraction flow (dragon_cli_test)",
    ],
    notes: "Production adapter for iframe and tokenized listing layouts.",
  },
  {
    id: "lockbit",
    name: "LockBit Embedded Nginx",
    supportLevel: "Full Crawl",
    matchingStrategy: "Known-domain + Nginx marker and body signature matching",
    sampleUrls: [
      "http://lockbit.onion",
      "http://lockbit6vhrjaqzsdj6pqalyideigxv4xycfeyunpx35znogiwmojnid.onion/secret/212f70e703d758fbccbda3013a21f5de-f033da37-5fa7-31df-b10c-cc04b8538e85/jobberswarehouse.com/",
    ],
    testedFor: [
      "Adapter fingerprint match (engine_test)",
      "Direct artifact URL routing (engine_test)",
      "Autoindex traversal delegation (lockbit adapter)",
    ],
    notes: "Uses hardened autoindex crawler for full recursive traversal and size mapping.",
  },
  {
    id: "nu_server",
    name: "Nu Server",
    supportLevel: "Full Crawl",
    matchingStrategy: "Response preamble signature matching",
    sampleUrls: ["http://nu-server.onion"],
    testedFor: [
      "Adapter fingerprint match (engine_test)",
      "Autoindex traversal delegation (nu adapter)",
    ],
    notes: "Delegates crawl execution to hardened autoindex traversal for directory/file extraction.",
  },
  {
    id: "inc_ransom",
    name: "INC Ransom Crawler",
    supportLevel: "Full Crawl",
    matchingStrategy: "Known-domain and blog signature matching",
    sampleUrls: [
      "http://incblog6qu4y4mm4zvw5nrmue6qbwtgjsxpw6b7ixzssu36tsajldoad.onion/blog/disclosures/698d5c538f1d14b7436dd63b",
    ],
    testedFor: ["Adapter fingerprint match (engine_test)"],
    notes: "Production adapter using disclosure API enrichment and crawl streaming.",
  },
  {
    id: "pear",
    name: "Pear Ransomware Crawler",
    supportLevel: "Full Crawl",
    matchingStrategy: "Known-domain and body signature matching",
    sampleUrls: [
      "http://m3wwhkus4dxbnxbtihexlyd2cv63qrvex6jiebc4vqe22kg2z3udebid.onion/sdeb.org/",
    ],
    testedFor: ["Adapter fingerprint match (engine_test)"],
    notes: "Production adapter with concurrent crawl workers and UI batching.",
  },
  {
    id: "play",
    name: "Play Ransomware (Autoindex)",
    supportLevel: "Full Crawl",
    matchingStrategy: "Known-domain, URL-path, and autoindex fingerprint matching",
    sampleUrls: [
      "http://b3pzp6qwelgeygmzn6awkduym6s4gxh6htwxuxeydrziwzlx63zergyd.onion/FALOp",
    ],
    testedFor: [
      "Adapter fingerprint suite (engine_test + play_e2e_test)",
      "Feature and resilience suite (play_features_test)",
    ],
    notes: "Most heavily tested adapter with full listing/scaffold validation.",
  },
  {
    id: "autoindex",
    name: "Generic Autoindex",
    supportLevel: "Fallback",
    matchingStrategy: "Generic 'Index of /' autoindex detection",
    sampleUrls: ["http://unknown.onion/files/"],
    testedFor: ["Fallback adapter match (engine_test)"],
    notes: "Default catch-all fallback when specialized adapters do not match.",
  },
];



// Kept dummy function so lines don't shift too much

function formatDuration(ms: number): string {
  const totalSec = Math.floor(ms / 1000);
  const min = Math.floor(totalSec / 60);
  const sec = totalSec % 60;
  if (min > 0) return `${min}m ${sec}s`;
  return `${sec}s`;
}

const INITIAL_CRAWL_STATUS: CrawlStatusEvent = {
  phase: "idle",
  progressPercent: 0,
  visitedNodes: 0,
  processedNodes: 0,
  queuedNodes: 0,
  activeWorkers: 0,
  workerTarget: 0,
  etaSeconds: null,
  estimation: "adaptive-frontier",
};

const INITIAL_DOWNLOAD_BATCH_STATUS: DownloadBatchStatus = {
  totalFiles: 0,
  completedFiles: 0,
  failedFiles: 0,
  totalBytesHint: 0,
  unknownSizeFiles: 0,
  currentFile: "",
  speedMbps: 0,
  downloadedBytes: 0,
  bbrBottleneckMbps: 0,
  ekfCovariance: 0,
  startedAt: null,
  etaSeconds: null,
};

function App() {
  const isTauriRuntime = typeof (window as any).__TAURI_INTERNALS__ !== "undefined";
  const isFixtureMode = !isTauriRuntime && isVfsFixtureMode();
  const [url, setUrl] = useState("");
  const [isCrawling, setIsCrawling] = useState(false);
  const [isCancelling, setIsCancelling] = useState(false);
  const [vfsStats, setVfsStats] = useState({ files: 0, folders: 0, size: 0, totalNodes: 0 });
  const [vfsRefreshTrigger, setVfsRefreshTrigger] = useState(0);
  const [logs, setLogs] = useState<string[]>([
    "Initializing Kernel Modules...",
    "[SYSTEM] Local Tor Daemon initialized on 127.0.0.1:9051",
    "[SYSTEM] Adapter Registry loaded (WorldLeaks, DragonForce, LockBit, INC Ransom, Pear, Play, Autoindex)",
  ]);
  const [activeAdapter, setActiveAdapter] = useState("Unidentified");
  const [torStatus, setTorStatus] = useState<TorStatus | null>(null);

  const [downloadProgress, setDownloadProgress] = useState<Record<string, DownloadProgressEvent>>({});
  const [crawlStatus, setCrawlStatus] = useState<CrawlStatusEvent>(INITIAL_CRAWL_STATUS);
  const [downloadBatchStatus, setDownloadBatchStatus] = useState<DownloadBatchStatus>(INITIAL_DOWNLOAD_BATCH_STATUS);
  const [selectedFiles, setSelectedFiles] = useState<FileEntry[]>([]);
  const [toasts, setToasts] = useState<ToastInfo[]>([]);
  const [outputDir, setOutputDir] = useState("");
  const [daemonPorts, setDaemonPorts] = useState<number[]>([9051, 9052, 9053, 9054]);
  const [crawlStartTime, setCrawlStartTime] = useState<number | null>(null);
  const [crawlElapsed, setCrawlElapsed] = useState(0);
  const [downloadElapsed, setDownloadElapsed] = useState(0);
  const [showSupportPopover, setShowSupportPopover] = useState(false);
  const [supportCatalog, setSupportCatalog] = useState<AdapterSupportInfo[]>([]);
  const [supportCatalogError, setSupportCatalogError] = useState<string | null>(null);

  const urlInputRef = useRef<HTMLInputElement>(null);
  const previewNoticeShownRef = useRef(false);
  const fixtureNoticeShownRef = useRef(false);
  const supportButtonRef = useRef<HTMLButtonElement>(null);
  const supportPopoverRef = useRef<HTMLDivElement>(null);
  const batchSpeedSampleRef = useRef<{ ts: number; bytes: number } | null>(null);

  const [crawlOptions, setCrawlOptions] = useState({
    listing: true,
    sizes: true,
    download: false,
    circuits: 120,
    daemons: 0
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

  useEffect(() => {
    const startedAt = downloadBatchStatus.startedAt;
    if (!startedAt) return;
    const done = downloadBatchStatus.completedFiles + downloadBatchStatus.failedFiles;
    if (downloadBatchStatus.totalFiles > 0 && done >= downloadBatchStatus.totalFiles) {
      return;
    }
    const interval = setInterval(() => {
      setDownloadElapsed(Date.now() - startedAt);
    }, 1000);
    return () => clearInterval(interval);
  }, [
    downloadBatchStatus.startedAt,
    downloadBatchStatus.completedFiles,
    downloadBatchStatus.failedFiles,
    downloadBatchStatus.totalFiles,
  ]);

  useEffect(() => {
    if (!isFixtureMode || fixtureNoticeShownRef.current) return;
    fixtureNoticeShownRef.current = true;
    setVfsStats(VFS_FIXTURE_STATS);
    setVfsRefreshTrigger(Date.now());
    setLogs((l) => [
      ...l.slice(-399),
      "[SYSTEM] Fixture VFS mode enabled for browser integrity testing.",
    ]);
  }, [isFixtureMode]);

  // Keyboard shortcuts: ⌘+Enter to start, Esc to stop, ⌘+E to export
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      const isMeta = e.metaKey || e.ctrlKey;

      if (e.key === "Escape" && showSupportPopover) {
        e.preventDefault();
        setShowSupportPopover(false);
        return;
      }

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
  }, [isCrawling, showSupportPopover, url, vfsStats]);

  useEffect(() => {
    async function initPaths() {
      if (!isTauriRuntime) {
        setOutputDir((prev) => prev || "OnionForger_Downloads");
        return;
      }
      try {
        const dl = await downloadDir();
        const defaultPath = await join(dl, "OnionForger_Downloads");
        setOutputDir((prev) => prev || defaultPath);
      } catch (e) {
        console.warn("Could not retrieve download directory", e);
      }
    }
    initPaths();
    const unlistenPromises: Array<Promise<() => void>> = [];

    if (isTauriRuntime) {
      unlistenPromises.push(
        listen<string>("crawl_log", (event) => {
          const payload = event.payload;
          const adapterMatch = payload.match(/Match found:\s*(.+)$/);
          if (adapterMatch && adapterMatch[1]) {
            setActiveAdapter(adapterMatch[1].trim());
          }
          setLogs((l) => [...l.slice(-399), `> ${payload}`]);
        })
      );
      unlistenPromises.push(
        listen<string>("log", (event) => {
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
        listen<CrawlStatusEvent>("crawl_status_update", (event) => {
          setCrawlStatus(event.payload);
        })
      );

      unlistenPromises.push(
        listen<DownloadBatchStartedEvent>("download_batch_started", (event) => {
          const startedAt = Date.now();
          batchSpeedSampleRef.current = { ts: startedAt, bytes: 0 };
          setDownloadElapsed(0);
          setDownloadBatchStatus({
            totalFiles: Math.max(event.payload.totalFiles || 0, 0),
            completedFiles: 0,
            failedFiles: 0,
            totalBytesHint: Math.max(event.payload.totalBytesHint || 0, 0),
            unknownSizeFiles: Math.max(event.payload.unknownSizeFiles || 0, 0),
            currentFile: "Routing download queue...",
            speedMbps: 0,
            downloadedBytes: 0,
            bbrBottleneckMbps: 0,
            ekfCovariance: 0,
            startedAt,
            etaSeconds: null,
          });
        })
      );

      unlistenPromises.push(
        listen<BatchProgressEvent>("batch_progress", (event) => {
          const payload = event.payload as BatchProgressEvent & {
            speed_mbps?: number;
            downloaded_bytes?: number;
            current_file?: string;
            bbr_bottleneck_mbps?: number;
            ekf_covariance?: number;
          };
          const speedMbpsRaw = payload.speedMbps ?? payload.speed_mbps;
          const downloadedBytesRaw = payload.downloadedBytes ?? payload.downloaded_bytes;
          const currentFileRaw = payload.currentFile ?? payload.current_file;
          const bbrRaw = payload.bbrBottleneckMbps ?? payload.bbr_bottleneck_mbps ?? 0;
          const ekfRaw = payload.ekfCovariance ?? payload.ekf_covariance ?? 0;
          const now = Date.now();
          setDownloadBatchStatus((prev) => {
            const startedAt = prev.startedAt ?? now;
            const completedFiles = Math.max(prev.completedFiles, payload.completed || 0);
            const failedFiles = Math.max(prev.failedFiles, payload.failed || 0);
            const totalFiles = Math.max(prev.totalFiles, payload.total || 0);
            const done = completedFiles + failedFiles;
            const remaining = Math.max(totalFiles - done, 0);
            const elapsedSeconds = Math.max(1, Math.floor((now - startedAt) / 1000));
            const etaSeconds =
              done > 0 && remaining > 0 ? Math.ceil((elapsedSeconds / done) * remaining) : null;
            const mergedDownloadedBytes = Math.max(prev.downloadedBytes, downloadedBytesRaw || 0);

            let resolvedSpeedMbps = speedMbpsRaw ?? prev.speedMbps;
            if ((speedMbpsRaw === undefined || speedMbpsRaw <= 0) && batchSpeedSampleRef.current) {
              const sample = batchSpeedSampleRef.current;
              const deltaBytes = Math.max(0, mergedDownloadedBytes - sample.bytes);
              const deltaSeconds = Math.max((now - sample.ts) / 1000, 0);
              if (deltaBytes > 0 && deltaSeconds > 0) {
                resolvedSpeedMbps = (deltaBytes / deltaSeconds) / 1048576;
              }
            }
            batchSpeedSampleRef.current = { ts: now, bytes: mergedDownloadedBytes };

            if (prev.startedAt === null) {
              setDownloadElapsed(0);
            }

            return {
              ...prev,
              totalFiles,
              completedFiles,
              failedFiles,
              currentFile: currentFileRaw || prev.currentFile,
              speedMbps: resolvedSpeedMbps,
              downloadedBytes: mergedDownloadedBytes,
              bbrBottleneckMbps: bbrRaw,
              ekfCovariance: ekfRaw,
              startedAt,
              etaSeconds,
            };
          });
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

      unlistenPromises.push(
        listen<{ url: string; path: string; reason: string }>("download_interrupted", (event) => {
          setLogs((l) => [...l.slice(-399), `[SYSTEM] Download interrupted for ${event.payload.path}: ${event.payload.reason}`]);
          showToast("success", "Download Interrupted", `${event.payload.reason} for ${event.payload.path}`);
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
      unlistenPromises.forEach((p) => {
        p.then((f) => f()).catch(() => undefined);
      });
    };
  }, [outputDir, isTauriRuntime]);

  useEffect(() => {
    const logContainer = document.querySelector('.forensic-log');
    if (logContainer) logContainer.scrollTop = logContainer.scrollHeight;
  }, [logs]);

  useEffect(() => {
    if (!showSupportPopover) return;

    const handleOutsideClick = (event: MouseEvent) => {
      const target = event.target as Node;
      if (supportPopoverRef.current?.contains(target) || supportButtonRef.current?.contains(target)) {
        return;
      }
      setShowSupportPopover(false);
    };

    document.addEventListener("mousedown", handleOutsideClick);
    return () => document.removeEventListener("mousedown", handleOutsideClick);
  }, [showSupportPopover]);

  const handleCrawl = useCallback(async () => {
    if (!url) return;
    const preserveFixtureState = isFixtureMode;
    setIsCrawling(true);
    setActiveAdapter("Unidentified");

    setCrawlStartTime(Date.now());
    setCrawlElapsed(0);
    setDownloadProgress({});
    setDownloadBatchStatus(INITIAL_DOWNLOAD_BATCH_STATUS);
    batchSpeedSampleRef.current = null;
    setDownloadElapsed(0);
    setLogs((l) => [...l, `--- Initiating Crawl ---`]);
    setLogs((l) => [...l, `> Probing Target: ${url}`]);
    setCrawlStatus({
      ...INITIAL_CRAWL_STATUS,
      phase: "probing",
    });
    if (preserveFixtureState) {
      setVfsStats(VFS_FIXTURE_STATS);
    } else {
      setVfsStats({ files: 0, folders: 0, size: 0, totalNodes: 0 });
    }
    setVfsRefreshTrigger(Date.now());

    try {
      if (typeof (window as any).__TAURI_INTERNALS__ === 'undefined') {
        throw new Error("Execution Environment Mismatch: Not running in native Tauri container.");
      }

      const payloadOptions = {
        ...crawlOptions,
        daemons: crawlOptions.daemons > 0 ? crawlOptions.daemons : null,
      };

      const files = await invoke<FileEntry[]>("start_crawl", { url, options: payloadOptions, outputDir });
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
      setCrawlStatus((prev) => {
        if (prev.phase === "complete" || prev.phase === "error" || prev.phase === "cancelled") {
          return prev;
        }
        return {
          ...prev,
          phase: "idle",
        };
      });
      if (preserveFixtureState) {
        setVfsStats(VFS_FIXTURE_STATS);
        setVfsRefreshTrigger(Date.now());
      }
    }
  }, [url, crawlOptions, outputDir, isFixtureMode]);

  const handleCancelCrawl = async () => {
    if (isCancelling) return;
    setIsCancelling(true);
    try {
      if (!isTauriRuntime) {
        setIsCrawling(false);
        setCrawlStartTime(null);
        setCrawlStatus((prev) => ({ ...prev, phase: "cancelled" }));
        setDownloadBatchStatus((prev) => ({
          ...prev,
          currentFile: "cancelled",
          etaSeconds: null,
        }));
        setLogs((l) => [...l, `[SYSTEM] Cancel acknowledged in preview mode (no native crawl workers active).`]);
        showToast("success", "Cancel Acknowledged", "Preview mode has no active native crawl workers.");
        return;
      }
      const result = await invoke<string>("cancel_crawl");
      setIsCrawling(false);
      setCrawlStartTime(null);
      setCrawlStatus((prev) => ({ ...prev, phase: "cancelled" }));
      setDownloadBatchStatus((prev) => ({
        ...prev,
        currentFile: "cancelled",
        etaSeconds: null,
      }));
      setLogs((l) => [...l, `[SYSTEM] ⚠ ${result}`]);
      showToast("success", "Cancellation Requested", result);
    } catch (err: any) {
      setLogs((l) => [...l, `[ERROR] Cancel failed: ${err.message || err}`]);
      showToast("error", "Cancel Failed", String(err.message || err));
    } finally {
      setIsCancelling(false);
    }
  };

  const handleDownload = async (rawUrl: string, filePath: string) => {
    setLogs((l) => [...l, `> Requesting Download: ${filePath}`]);
    if (!isTauriRuntime) {
      setLogs((l) => [...l, `[SIMULATION] Preview mode: download request captured for ${filePath}.`]);
      showToast("success", "Preview Download", "No backend download executed in browser preview mode.");
      return;
    }
    try {
      if (filePath.endsWith('/')) {
        const entry: FileEntry = {
          path: filePath,
          size_bytes: null,
          entry_type: 'Folder',
          raw_url: rawUrl
        };
        const count = await invoke<number>("download_files", {
          entries: [entry],
          outputDir,
          connections: crawlOptions.circuits,
        });
        showToast("success", "Download Complete", `${count} item(s) saved to ${outputDir}`);
        setLogs((l) => [...l, `[MIRROR] Saved ${filePath} to disk`]);
      } else {
        // High concurrency chunked download for single files
        let targetPath = outputDir.endsWith('/') || outputDir.endsWith('\\') ? `${outputDir}${filePath}` : `${outputDir}/${filePath}`;
        await invoke("initiate_download", {
          args: {
            url: rawUrl,
            path: targetPath,
            output_root: outputDir,
            connections: crawlOptions.circuits || 120,
            force_tor: rawUrl.includes(".onion"),
          }
        });
        showToast("success", "Download Engine Started", `Allocating ${crawlOptions.circuits || 120} circuits to target...`);
      }
    } catch (err: any) {
      setLogs((l) => [...l, `[ERROR] Download failed: ${err}`]);
      showToast("error", "Download Error", String(err));
    }
  };

  const handleDownloadSelected = async () => {
    if (selectedFiles.length === 0) return;
    setLogs((l) => [...l, `[OPSEC] Manual Mirror: Scaffolding ${selectedFiles.length} selected nodes to ${outputDir}...`]);
    if (!isTauriRuntime) {
      setLogs((l) => [...l, `[SIMULATION] Preview mode: selected mirror request captured (${selectedFiles.length} items).`]);
      showToast("success", "Preview Mirror", "No backend write executed in browser preview mode.");
      return;
    }
    try {
      const count = await invoke<number>("download_files", {
        entries: selectedFiles,
        outputDir,
        connections: crawlOptions.circuits,
      });
      showToast("success", "Mirror Complete", `${count} items written to ${outputDir}`);
      setLogs((l) => [...l, `[MIRROR] Complete. ${count}/${selectedFiles.length} selected items on disk.`]);
    } catch (err: any) {
      setLogs((l) => [...l, `[ERROR] Mirror failed: ${err}`]);
      showToast("error", "Mirror Error", String(err));
    }
  };

  const handleExportJSON = async () => {
    if (vfsStats.totalNodes === 0) return;
    if (!isTauriRuntime) {
      setLogs((l) => [...l, `[SIMULATION] Preview mode: export request captured.`]);
      showToast("success", "Preview Export", "Export dialog is only available in native Tauri mode.");
      return;
    }
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
    if (!isTauriRuntime) {
      setLogs((l) => [...l, `[SIMULATION] Preview mode: mass mirror request captured.`]);
      showToast("success", "Preview Mirror", "Mass mirror is only available in native Tauri mode.");
      return;
    }
    try {
      showToast("success", "Scaffolding Started", `Extracting entire VFS structure to primary disk...`);
      const count = await invoke<number>("download_all", {
        outputDir,
        connections: crawlOptions.circuits,
      });
      showToast("success", "Mirror Complete", `${count} total items structured on disk.`);
      setLogs((l) => [...l, `[MIRROR] Complete. ${count} total items on disk.`]);
    } catch (err: any) {
      setLogs((l) => [...l, `[ERROR] Mass Mirror failed: ${err}`]);
      showToast("error", "Mirror Error", String(err));
    }
  };



  const handleSelectOutput = async () => {
    if (!isTauriRuntime) {
      showToast("success", "Preview Mode", "Output path picker is only available in native Tauri mode.");
      return;
    }
    try {
      const selected = await open({
        directory: true,
        multiple: false,
        title: "Select Download Location",
      });
      if (selected && typeof selected === "string") {
        setOutputDir(selected);
        setLogs((l) => [...l.slice(-399), `[PATH] Output location set to: ${selected}`]);
        showToast("success", "Storage Linked", `Updated target extraction path to ${selected}`);
      }
    } catch (e) {
      console.warn("Failed to open dialog", e);
    }
  };

  const handleToggleSupportPopover = async () => {
    if (showSupportPopover) {
      setShowSupportPopover(false);
      return;
    }

    setShowSupportPopover(true);
    if (supportCatalog.length > 0) return;

    try {
      if (isTauriRuntime) {
        const catalog = await invoke<AdapterSupportInfo[]>("get_adapter_support_catalog");
        if (Array.isArray(catalog) && catalog.length > 0) {
          setSupportCatalog(catalog);
          setSupportCatalogError(null);
          return;
        }
      }
      setSupportCatalog(FALLBACK_SUPPORT_CATALOG);
      setSupportCatalogError(null);
    } catch (err: any) {
      const message = String(err?.message || err || "Failed to load adapter support catalog.");
      setSupportCatalog(FALLBACK_SUPPORT_CATALOG);
      setSupportCatalogError(message);
      setLogs((l) => [...l.slice(-399), `[SYSTEM] Support catalog fallback active: ${message}`]);
    }
  };

  const totalSizeStr = (() => {
    if (vfsStats.size === 0) return "0 B";
    if (vfsStats.size >= 1_073_741_824) return (vfsStats.size / 1_073_741_824).toFixed(2) + " GB";
    if (vfsStats.size >= 1_048_576) return (vfsStats.size / 1_048_576).toFixed(2) + " MB";
    if (vfsStats.size >= 1024) return (vfsStats.size / 1024).toFixed(2) + " KB";
    return vfsStats.size + " B";
  })();

  const supportRows = supportCatalog.length > 0 ? supportCatalog : FALLBACK_SUPPORT_CATALOG;
  const fullCrawlCount = supportRows.filter((item) => item.supportLevel === "Full Crawl").length;

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
            {isCrawling ? <VibeLoader size={12} variant="accent" /> : <Activity size={14} />} SYS: {torStatus ? torStatus.state.toUpperCase() : "IDLE"}
          </div>
        </div>
      </header>

      <div className="toolbar" data-testid="toolbar">
        <button className="tool-btn" data-testid="btn-load-target" onClick={() => setUrl("http://worldleaks.onion/api/")}>
          <FolderSearch size={22} className={url.includes("worldleaks") ? "pulse-line text-accent-primary" : ""} /> Load Target
        </button>
        <button className="tool-btn" data-testid="btn-resume" onClick={handleCrawl} disabled={isCrawling}>
          <Play size={22} /> Resume
        </button>
        <button
          className="tool-btn danger"
          data-testid="btn-cancel"
          onClick={handleCancelCrawl}
          disabled={isCancelling}
          title="Stop crawl (Esc)"
        >
          <XCircle size={22} /> {isCancelling ? "Cancelling" : "Cancel"}
        </button>
        <button
          className="tool-btn"
          data-testid="btn-export"
          onClick={handleExportJSON}
          disabled={vfsStats.totalNodes === 0}
          title="Export JSON (⌘+E)"
        >
          <FileJson size={22} /> Export
        </button>
        <button
          ref={supportButtonRef}
          className="tool-btn"
          data-testid="btn-support"
          onClick={handleToggleSupportPopover}
          title="Show supported adapters and test coverage"
        >
          <CircleHelp size={22} /> Support
        </button>
      </div>

      {showSupportPopover && (
        <div className="support-popover" ref={supportPopoverRef} data-testid="support-popover" role="dialog" aria-modal="false" aria-label="Supported adapters">
          <div className="support-popover-header">
            <div>
              <div className="support-popover-title">Supported Adapters</div>
              <div className="support-popover-subtitle">
                {fullCrawlCount} full-crawl adapters, {supportRows.length - fullCrawlCount} detection/fallback adapters
              </div>
            </div>
            <button
              className="support-close-btn"
              data-testid="btn-support-close"
              onClick={() => setShowSupportPopover(false)}
            >
              Close
            </button>
          </div>

          {supportCatalogError && (
            <div className="support-warning">
              Live catalog unavailable, displaying local fallback list.
            </div>
          )}

          <div className="support-list">
            {supportRows.map((adapter) => (
              <div className="support-card" key={adapter.id} data-testid={`adapter-row-${adapter.id}`}>
                <div className="support-card-top">
                  <span className="support-card-name">{adapter.name}</span>
                  <span
                    className={`support-level-badge ${adapter.supportLevel === "Full Crawl"
                      ? "full"
                      : adapter.supportLevel === "Fallback"
                        ? "fallback"
                        : "detection"
                      }`}
                  >
                    {adapter.supportLevel}
                  </span>
                </div>
                <div className="support-card-line">
                  <span className="support-label">Matching:</span> {adapter.matchingStrategy}
                </div>
                <div className="support-card-line">
                  <span className="support-label">Sample URL(s):</span>{" "}
                  <span className="support-sample-urls">
                    {adapter.sampleUrls.length > 0 ? adapter.sampleUrls.join(" | ") : "Not listed"}
                  </span>
                </div>
                <div className="support-card-line">
                  <span className="support-label">Tested for:</span>{" "}
                  {adapter.testedFor.length > 0 ? adapter.testedFor.join(" | ") : "No dedicated adapter test yet"}
                </div>
                <div className="support-card-note">{adapter.notes}</div>
              </div>
            ))}
          </div>
        </div>
      )}

      <div className="url-bar">
        <div className="input-group">
          <span className="input-label">Target Source</span>
          <input
            ref={urlInputRef}
            data-testid="input-target-url"
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
          data-testid="btn-start-queue"
          onClick={handleCrawl}
          disabled={isCrawling}
        >
          {isCrawling ? (
            <span style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
              <VibeLoader size={18} variant="primary" /> Scanning
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
            data-testid="input-output-path"
            type="text"
            className="url-input"
            style={{ fontFamily: 'JetBrains Mono', fontSize: '0.85rem' }}
            readOnly
            value={outputDir}
          />
        </div>
        <button
          className="action-btn popup-hover"
          data-testid="btn-change-output"
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
            data-testid="chk-listing"
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
            data-testid="chk-sizes"
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
            data-testid="chk-auto-download"
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
            data-testid="sel-circuits"
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

        <div style={{ display: 'flex', alignItems: 'center', gap: '8px', marginLeft: '24px' }}>
          <span style={{ fontSize: '0.85rem', color: 'var(--text-muted)' }}>Tor Daemons:</span>
          <select
            data-testid="sel-daemons"
            value={crawlOptions.daemons}
            onChange={(e) => setCrawlOptions({ ...crawlOptions, daemons: parseInt(e.target.value) })}
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
            <option value={0}>Auto (Balanced)</option>
            <option value={4}>4 Daemons</option>
            <option value={8}>8 Daemons</option>
            <option value={12}>12 Daemons (HFT)</option>
            <option value={16}>16 Daemons (Max)</option>
          </select>
        </div>
      </div>

      <Dashboard
        isCrawling={isCrawling}
        torStatus={torStatus}
        activeAdapter={activeAdapter}
        crawlStatus={crawlStatus}
        downloadBatchStatus={downloadBatchStatus}
        logs={logs}
        vfsCount={vfsStats.totalNodes}
        downloadProgress={downloadProgress}
        elapsed={crawlElapsed}
        downloadElapsed={downloadElapsed}
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
                    data-testid="btn-mass-extract-all"
                    onClick={handleDownloadAll}
                    style={{ padding: '2px 12px', fontSize: '0.75rem', height: '28px', minWidth: 'auto', background: 'transparent', border: '1px solid var(--border-hud)', color: 'var(--accent-secondary)', display: 'flex', gap: '6px', alignItems: 'center' }}
                    title="Safely Scaffold All Indexed Entries via Multi-Threading"
                  >
                    <Download size={12} /> Mass Extract All
                  </button>
                  {selectedFiles.length > 0 && (
                    <button
                      className="action-btn popup-hover"
                      data-testid="btn-download-selected"
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
            <div className="daemon-icon">
              {isCrawling ? <VibeLoader size={18} variant="secondary" /> : <Zap size={18} />}
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
