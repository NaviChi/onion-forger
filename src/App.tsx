import { useState, useEffect, useCallback, useRef } from "react";
import { VFSExplorer, FileEntry } from "./components/VFSExplorer";
import { Dashboard } from "./components/Dashboard";
import { PatientRetryPanel, PatientRetryState } from "./components/PatientRetryPanel";
import { AzureConnectivityModal } from "./components/AzureConnectivityModal";
import { VibeLoader } from "./components/VibeLoader";
import { HexViewer } from "./components/HexViewer";
import { Zap, Play, Activity, FolderSearch, Globe, ListTree, Terminal, CheckCircle, AlertCircle, Save, Download, FileJson, Clock, XCircle, CircleHelp, Cloud, Magnet, ShieldAlert, HardDrive, Database, Cpu } from "lucide-react";
import { FIXTURE_RESOURCE_METRICS, VFS_FIXTURE_ENTRIES, VFS_FIXTURE_STATS, isVfsFixtureMode } from "./fixtures/vfsFixture";
import { getDownloadDir, invokeCommand, isTauriRuntime as getIsTauriRuntime, joinPath, listenEvent, openDialog, saveDialog } from "./platform/tauriClient";
import { NATIVE_WEBVIEW_SMOKE_TEST_IDS } from "./test/selectors";

import "./App.css";


interface DownloadProgressEvent {
  path: string;
  bytes_downloaded: number;
  total_bytes: number | null;
  speed_bps: number;
  bytesDownloaded?: number;
  totalBytes?: number | null;
  speedBps?: number;
  active_circuits?: number;
  activeCircuits?: number;
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
  deltaNewFiles?: number;
  vanguard?: {
    current: number;
    target: number;
    status: string;
  };
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
  activeCircuits?: number;
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
  smoothedSpeedMbps: number;
  downloadedBytes: number;
  activeCircuits: number;
  peakActiveCircuits: number;
  peakBandwidthMbps: number;
  diskWriteMbps: number;
  peakDiskWriteMbps: number;
  etaConfidence: number;
  outputDir: string;
  bbrBottleneckMbps: number;
  ekfCovariance: number;
  startedAt: number | null;
  etaSeconds: number | null;
}

interface CrawlSessionResult {
  targetKey: string;
  discoveredCount: number;
  fileCount: number;
  folderCount: number;
  bestPriorCount: number;
  rawThisRunCount: number;
  mergedEffectiveCount: number;
  crawlOutcome: string;
  retryCountUsed: number;
  stableCurrentListingPath: string;
  stableCurrentDirsListingPath: string;
  stableBestListingPath: string;
  stableBestDirsListingPath: string;
  autoDownloadStarted: boolean;
  outputDir: string;
}

interface DownloadResumePlan {
  targetKey: string;
  failedFirstCount: number;
  missingOrMismatchCount: number;
  skippedExactMatchesCount: number;
  allItemsSkipped: boolean;
  plannedFileCount: number;
  failureManifestPath: string;
}

interface ResourceMetricsSnapshot {
  processCpuPercent: number;
  processMemoryBytes: number;
  processThreads: number;
  systemMemoryUsedBytes: number;
  systemMemoryTotalBytes: number;
  systemMemoryPercent: number;
  activeWorkers: number;
  workerTarget: number;
  activeCircuits: number;
  peakActiveCircuits: number;
  currentNodeHost?: string | null;
  multiClientRotations?: number;
  multiClientCount?: number;
  nodeFailovers: number;
  throttleCount: number;
  timeoutCount: number;
  uptimeSeconds: number;
  consensusWeight: number;
  swarmRuntimeLabel?: string | null;
  swarmTrafficClass?: string | null;
  swarmClientCount?: number;
  managedPortCount?: number;
  healthProbeTarget?: string | null;
  totalRequests?: number;
  successfulRequests?: number;
  failedRequests?: number;
  fingerprintLatencyMs?: number;
  cachedRouteHits?: number;
  qilinFreshRedirectCandidates?: number;
  qilinStaleHostOnlyCandidates?: number;
  qilinDegradedStageDActivations?: number;
  subtreeReroutes?: number;
  subtreeQuarantineHits?: number;
  offWinnerChildRequests?: number;
  winnerHost?: string | null;
  slowestCircuit?: string | null;
  lateThrottles?: number;
  outlierIsolations?: number;
  downloadHostCacheHits?: number;
  downloadProbePromotionHits?: number;
  downloadLowSpeedAborts?: number;
  downloadProbeQuarantineHits?: number;
  downloadProbeCandidateExhaustions?: number;
}

interface EfficiencyHistory {
  requestsPerEntry: number[];
  requestSuccessRate: number[];
  activeCircuits: number[];
  fingerprintLatencyMs: number[];
}

interface TorStatus {
  state: string;
  message: string;
  daemon_count: number;
  ports?: number[];
  runtime?: string;
  traffic_class?: string;
  ready_clients?: number;
  managed_port_count?: number;
  health_probe_target?: string;
}

interface TelemetryBridgeUpdate {
  crawlStatus?: Partial<CrawlStatusEvent>;
  resourceMetrics?: Partial<ResourceMetricsSnapshot>;
}

interface ToastInfo {
  id: number;
  type: "success" | "error";
  title: string;
  message: string;
}

interface LogAggregate {
  key: string;
  sample: string;
  count: number;
  firstSeenAt: number;
  lastSeenAt: number;
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

interface NativeWebviewSmokeConfig {
  enabled: boolean;
  reportPath: string | null;
  autoExit: boolean;
  waitMs: number;
  expectedTestIds: string[];
}

interface NativeWebviewSmokeResult {
  mounted: boolean;
  title: string;
  href: string;
  isTauriRuntime: boolean;
  expectedTestIds: string[];
  foundTestIds: string[];
  missingTestIds: string[];
  reportedAtEpochMs: number;
}

const FALLBACK_SUPPORT_CATALOG: AdapterSupportInfo[] = [
  {
    id: "qilin",
    name: "Qilin Nginx Autoindex",
    supportLevel: "Full Crawl",
    matchingStrategy: "Known-domain + QData marker signature matching",
    sampleUrls: ["http://ijzn3sicrcy7guixkzjkib4ukbiilwc3xhnmby4mcbccnsd7j2rekvqd.onion/site/view?uuid=c9d2ba19-6aa1-3087-8773-f63d023179ed"],
    testedFor: [
      "Adapter fingerprint match (engine_test)",
      "Autoindex traversal delegation (qilin adapter)",
    ],
    notes: "Uses adaptive QData storage-node routing with bounded failover and streamed VFS ingestion.",
  },
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
    id: "abyss",
    name: "Abyss Ransomware",
    supportLevel: "Full Crawl",
    matchingStrategy: "Known-domain + direct archive URL detection (.rar, .zip, .7z)",
    sampleUrls: ["http://vmmefm7ktazj2bwtmy46o3wxhk42tctasyyqv6ymuzlivszteyhkkyad.onion/iamdesign.rar"],
    testedFor: ["Adapter fingerprint match (engine_test)"],
    notes: "Dual mode: direct file HEAD probe for archives, recursive directory traversal for listings.",
  },
  {
    id: "alphalocker",
    name: "AlphaLocker Ransomware",
    supportLevel: "Full Crawl",
    matchingStrategy: "Known-domain + URL-path signature matching",
    sampleUrls: ["http://3v4zoso2ghne47usnhyoe4dsezmfqhfv5v5iuep4saic5nnfpc6phrad.onion/gazomet.pl%20&%20cgas.pl/Files/"],
    testedFor: ["Adapter fingerprint match (engine_test)"],
    notes: "Supports URL-encoded paths and parses both autoindex and custom table layouts.",
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

function stripWindowsVerbatimPrefix(path: string): string {
  if (!path) return path;
  if (path.startsWith("\\\\?\\UNC\\")) {
    return `\\\\${path.slice(8)}`;
  }
  if (path.startsWith("\\\\?\\")) {
    return path.slice(4);
  }
  return path;
}

function normalizePathForCompare(path: string): string {
  return stripWindowsVerbatimPrefix(path)
    .replace(/\\/g, "/")
    .replace(/\/+/g, "/")
    .replace(/\/$/, "")
    .toLowerCase();
}

function deriveRelativePath(rawPath: string, roots: string[]): string {
  const cleaned = stripWindowsVerbatimPrefix(rawPath).replace(/\\/g, "/");
  const cleanedCmp = normalizePathForCompare(cleaned);
  for (const root of roots) {
    if (!root) continue;
    const normalizedRoot = normalizePathForCompare(root);
    if (!normalizedRoot) continue;
    if (cleanedCmp === normalizedRoot) return "";
    if (cleanedCmp.startsWith(`${normalizedRoot}/`)) {
      return cleaned.slice(normalizedRoot.length + 1).replace(/^[/\\]+/, "");
    }
  }
  return cleaned.replace(/^[/\\]+/, "");
}

function toDisplayPath(rawPath: string, roots: string[]): string {
  const relative = deriveRelativePath(rawPath, roots);
  return relative || stripWindowsVerbatimPrefix(rawPath).replace(/\\/g, "/");
}

function formatDuration(ms: number): string {
  const totalSec = Math.floor(ms / 1000);
  const min = Math.floor(totalSec / 60);
  const sec = totalSec % 60;
  if (min > 0) return `${min}m ${sec}s`;
  return `${sec}s`;
}

function summarizeLogKey(raw: string): string {
  return raw
    .replace(/^>\s*/, "")
    .replace(/[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}/gi, "<uuid>")
    .replace(/[a-z2-7]{56}\.onion/gi, "<onion>")
    .replace(/\b[Cc]ircuit\s+\d+\b/g, "Circuit <n>")
    .replace(/\b[Dd]aemon\s+\d+\b/g, "Daemon <n>")
    .replace(/\b[Pp]ort\s+\d+\b/g, "Port <n>")
    .replace(/\b[Aa]ttempt\s+\d+\b/g, "Attempt <n>");
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
  smoothedSpeedMbps: 0,
  downloadedBytes: 0,
  activeCircuits: 0,
  peakActiveCircuits: 0,
  peakBandwidthMbps: 0,
  diskWriteMbps: 0,
  peakDiskWriteMbps: 0,
  etaConfidence: 0,
  outputDir: "",
  bbrBottleneckMbps: 0,
  ekfCovariance: 0,
  startedAt: null,
  etaSeconds: null,
};

const INITIAL_RESOURCE_METRICS: ResourceMetricsSnapshot = {
  processCpuPercent: 0,
  processMemoryBytes: 0,
  processThreads: 0,
  systemMemoryUsedBytes: 0,
  systemMemoryTotalBytes: 0,
  systemMemoryPercent: 0,
  activeWorkers: 0,
  workerTarget: 0,
  activeCircuits: 0,
  peakActiveCircuits: 0,
  currentNodeHost: null,
  nodeFailovers: 0,
  throttleCount: 0,
  timeoutCount: 0,
  uptimeSeconds: 0,
  consensusWeight: 0,
  swarmRuntimeLabel: null,
  swarmTrafficClass: null,
  swarmClientCount: 0,
  managedPortCount: 0,
  healthProbeTarget: null,
  totalRequests: 0,
  successfulRequests: 0,
  failedRequests: 0,
  fingerprintLatencyMs: 0,
  cachedRouteHits: 0,
  qilinFreshRedirectCandidates: 0,
  qilinStaleHostOnlyCandidates: 0,
  qilinDegradedStageDActivations: 0,
  subtreeReroutes: 0,
  subtreeQuarantineHits: 0,
  offWinnerChildRequests: 0,
  winnerHost: null,
  slowestCircuit: null,
  lateThrottles: 0,
  outlierIsolations: 0,
  downloadHostCacheHits: 0,
  downloadProbePromotionHits: 0,
  downloadLowSpeedAborts: 0,
  downloadProbeQuarantineHits: 0,
  downloadProbeCandidateExhaustions: 0,
};

const INITIAL_EFFICIENCY_HISTORY: EfficiencyHistory = {
  requestsPerEntry: [],
  requestSuccessRate: [],
  activeCircuits: [],
  fingerprintLatencyMs: [],
};

function toFiniteNumber(value: unknown, fallback = 0): number {
  if (typeof value === "number" && Number.isFinite(value)) {
    return value;
  }
  if (typeof value === "string") {
    const parsed = Number(value);
    if (Number.isFinite(parsed)) {
      return parsed;
    }
  }
  return fallback;
}

function toNonNegativeInteger(value: unknown, fallback = 0): number {
  const normalized = Math.floor(toFiniteNumber(value, fallback));
  return normalized >= 0 ? normalized : fallback;
}

function normalizeCrawlStatusFrame(
  frame: Partial<CrawlStatusEvent> | null | undefined,
  previous: CrawlStatusEvent,
): CrawlStatusEvent {
  if (!frame) return previous;

  const progressPercent = Math.max(
    0,
    Math.min(100, toFiniteNumber(frame.progressPercent, previous.progressPercent)),
  );
  const phase =
    typeof frame.phase === "string" && frame.phase.length > 0
      ? frame.phase
      : previous.phase;
  const etaSeconds =
    frame.etaSeconds === null
      ? null
      : frame.etaSeconds === undefined
        ? previous.etaSeconds
        : toNonNegativeInteger(frame.etaSeconds, previous.etaSeconds ?? 0);

  return {
    ...previous,
    phase,
    progressPercent,
    visitedNodes: toNonNegativeInteger(frame.visitedNodes, previous.visitedNodes),
    processedNodes: toNonNegativeInteger(frame.processedNodes, previous.processedNodes),
    queuedNodes: toNonNegativeInteger(frame.queuedNodes, previous.queuedNodes),
    activeWorkers: toNonNegativeInteger(frame.activeWorkers, previous.activeWorkers),
    workerTarget: toNonNegativeInteger(frame.workerTarget, previous.workerTarget),
    etaSeconds,
    estimation:
      typeof frame.estimation === "string" && frame.estimation.length > 0
        ? frame.estimation
        : previous.estimation,
    deltaNewFiles:
      frame.deltaNewFiles === undefined
        ? previous.deltaNewFiles
        : toNonNegativeInteger(frame.deltaNewFiles, previous.deltaNewFiles ?? 0),
    vanguard:
      frame.vanguard === undefined
        ? previous.vanguard
        : frame.vanguard,
  };
}

function normalizeResourceMetricsFrame(
  frame: Partial<ResourceMetricsSnapshot> | null | undefined,
  previous: ResourceMetricsSnapshot,
): ResourceMetricsSnapshot {
  if (!frame) return previous;

  const systemMemoryUsedBytes = toNonNegativeInteger(
    frame.systemMemoryUsedBytes,
    previous.systemMemoryUsedBytes,
  );
  const systemMemoryTotalBytes = toNonNegativeInteger(
    frame.systemMemoryTotalBytes,
    previous.systemMemoryTotalBytes,
  );
  const derivedMemoryPercent =
    systemMemoryTotalBytes > 0
      ? (systemMemoryUsedBytes / systemMemoryTotalBytes) * 100
      : 0;
  const systemMemoryPercent = Math.max(
    0,
    Math.min(
      100,
      toFiniteNumber(
        frame.systemMemoryPercent,
        Number.isFinite(derivedMemoryPercent)
          ? derivedMemoryPercent
          : previous.systemMemoryPercent,
      ),
    ),
  );

  return {
    ...previous,
    processCpuPercent: Math.max(
      0,
      toFiniteNumber(frame.processCpuPercent, previous.processCpuPercent),
    ),
    processMemoryBytes: toNonNegativeInteger(
      frame.processMemoryBytes,
      previous.processMemoryBytes,
    ),
    processThreads: toNonNegativeInteger(frame.processThreads, previous.processThreads),
    systemMemoryUsedBytes,
    systemMemoryTotalBytes,
    systemMemoryPercent,
    activeWorkers: toNonNegativeInteger(frame.activeWorkers, previous.activeWorkers),
    workerTarget: toNonNegativeInteger(frame.workerTarget, previous.workerTarget),
    activeCircuits: toNonNegativeInteger(frame.activeCircuits, previous.activeCircuits),
    peakActiveCircuits: toNonNegativeInteger(
      frame.peakActiveCircuits,
      previous.peakActiveCircuits,
    ),
    currentNodeHost:
      frame.currentNodeHost === undefined
        ? previous.currentNodeHost
        : frame.currentNodeHost,
    nodeFailovers: toNonNegativeInteger(frame.nodeFailovers, previous.nodeFailovers),
    throttleCount: toNonNegativeInteger(frame.throttleCount, previous.throttleCount),
    timeoutCount: toNonNegativeInteger(frame.timeoutCount, previous.timeoutCount),
    uptimeSeconds: toNonNegativeInteger(frame.uptimeSeconds, previous.uptimeSeconds),
    consensusWeight: toNonNegativeInteger(frame.consensusWeight, previous.consensusWeight),
    swarmRuntimeLabel:
      frame.swarmRuntimeLabel === undefined
        ? previous.swarmRuntimeLabel
        : frame.swarmRuntimeLabel,
    swarmTrafficClass:
      frame.swarmTrafficClass === undefined
        ? previous.swarmTrafficClass
        : frame.swarmTrafficClass,
    swarmClientCount: toNonNegativeInteger(
      frame.swarmClientCount,
      previous.swarmClientCount ?? 0,
    ),
    managedPortCount: toNonNegativeInteger(
      frame.managedPortCount,
      previous.managedPortCount ?? 0,
    ),
    healthProbeTarget:
      frame.healthProbeTarget === undefined
        ? previous.healthProbeTarget
        : frame.healthProbeTarget,
    totalRequests: toNonNegativeInteger(frame.totalRequests, previous.totalRequests ?? 0),
    successfulRequests: toNonNegativeInteger(
      frame.successfulRequests,
      previous.successfulRequests ?? 0,
    ),
    failedRequests: toNonNegativeInteger(frame.failedRequests, previous.failedRequests ?? 0),
    fingerprintLatencyMs: toNonNegativeInteger(
      frame.fingerprintLatencyMs,
      previous.fingerprintLatencyMs ?? 0,
    ),
    cachedRouteHits: toNonNegativeInteger(
      frame.cachedRouteHits,
      previous.cachedRouteHits ?? 0,
    ),
    qilinFreshRedirectCandidates: toNonNegativeInteger(
      frame.qilinFreshRedirectCandidates,
      previous.qilinFreshRedirectCandidates ?? 0,
    ),
    qilinStaleHostOnlyCandidates: toNonNegativeInteger(
      frame.qilinStaleHostOnlyCandidates,
      previous.qilinStaleHostOnlyCandidates ?? 0,
    ),
    qilinDegradedStageDActivations: toNonNegativeInteger(
      frame.qilinDegradedStageDActivations,
      previous.qilinDegradedStageDActivations ?? 0,
    ),
    subtreeReroutes: toNonNegativeInteger(
      frame.subtreeReroutes,
      previous.subtreeReroutes ?? 0,
    ),
    subtreeQuarantineHits: toNonNegativeInteger(
      frame.subtreeQuarantineHits,
      previous.subtreeQuarantineHits ?? 0,
    ),
    offWinnerChildRequests: toNonNegativeInteger(
      frame.offWinnerChildRequests,
      previous.offWinnerChildRequests ?? 0,
    ),
    winnerHost:
      frame.winnerHost === undefined ? previous.winnerHost : frame.winnerHost,
    slowestCircuit:
      frame.slowestCircuit === undefined
        ? previous.slowestCircuit
        : frame.slowestCircuit,
    lateThrottles: toNonNegativeInteger(
      frame.lateThrottles,
      previous.lateThrottles ?? 0,
    ),
    outlierIsolations: toNonNegativeInteger(
      frame.outlierIsolations,
      previous.outlierIsolations ?? 0,
    ),
    downloadHostCacheHits: toNonNegativeInteger(
      frame.downloadHostCacheHits,
      previous.downloadHostCacheHits ?? 0,
    ),
    downloadProbePromotionHits: toNonNegativeInteger(
      frame.downloadProbePromotionHits,
      previous.downloadProbePromotionHits ?? 0,
    ),
    downloadLowSpeedAborts: toNonNegativeInteger(
      frame.downloadLowSpeedAborts,
      previous.downloadLowSpeedAborts ?? 0,
    ),
    downloadProbeQuarantineHits: toNonNegativeInteger(
      frame.downloadProbeQuarantineHits,
      previous.downloadProbeQuarantineHits ?? 0,
    ),
    downloadProbeCandidateExhaustions: toNonNegativeInteger(
      frame.downloadProbeCandidateExhaustions,
      previous.downloadProbeCandidateExhaustions ?? 0,
    ),
  };
}

export function isOnionTarget(input: string): boolean {
  const trimmed = input.trim();
  if (!trimmed) {
    return false;
  }

  try {
    return new URL(trimmed).hostname.toLowerCase().endsWith(".onion");
  } catch {
    const authority = trimmed
      .toLowerCase()
      .split("://")
      .pop()
      ?.split("/")
      .shift()
      ?.split("@")
      .pop()
      ?.split(":")
      .shift();
    return authority?.endsWith(".onion") ?? false;
  }
}

export function classifyTargetInputMode(input: string): "onion" | "direct" | "mega" | "torrent" {
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

function wait(ms: number): Promise<void> {
  return new Promise((resolve) => window.setTimeout(resolve, ms));
}

function App() {
  const isTauriRuntime = getIsTauriRuntime();
  const isFixtureMode = !isTauriRuntime && isVfsFixtureMode();
  const [url, setUrl] = useState("");
  const [inputMode, setInputMode] = useState<"onion" | "direct" | "mega" | "torrent">("onion");
  const [megaPassword, setMegaPassword] = useState("");
  const [megaProgress, setMegaProgress] = useState<{ index: number; total: number; file: string; status: string; completed: number; failed: number; skipped: number } | null>(null);
  const [torrentProgress, setTorrentProgress] = useState<{ downloaded_bytes: number; total_bytes: number; progress_pct: string; download_speed: string; status: string } | null>(null);
  const [isDragOver, setIsDragOver] = useState(false);
  const [showAzureModal, setShowAzureModal] = useState(false);
  const [isOpenHexViewer, setIsOpenHexViewer] = useState(false);
  const [isCrawling, setIsCrawling] = useState(false);
  const [isCancelling, setIsCancelling] = useState(false);
  const [vfsStats, setVfsStats] = useState({ files: 0, folders: 0, size: 0, totalNodes: 0 });
  const [vfsRefreshTrigger, setVfsRefreshTrigger] = useState(0);
  const [logs, setLogs] = useState<string[]>([
    "Initializing Kernel Modules...",
    "[SYSTEM] Local Tor Daemon initialized on 127.0.0.1:9051",
    "[SYSTEM] Adapter Registry loaded (Qilin, WorldLeaks, DragonForce, LockBit, INC Ransom, Pear, Play, Abyss, AlphaLocker, Autoindex)",
  ]);
  const [activeAdapter, setActiveAdapter] = useState("Unidentified");
  const [torStatus, setTorStatus] = useState<TorStatus | null>(null);

  const [downloadProgress, setDownloadProgress] = useState<Record<string, DownloadProgressEvent>>({});
  // Phase 128B: Throttled download progress to prevent GUI crash during large batch downloads.
  // Per-file progress is stored in a mutable ref and flushed to React state at most every 500ms.
  const downloadProgressBufferRef = useRef<Record<string, DownloadProgressEvent>>({});
  const downloadProgressFlushPending = useRef(false);
  const downloadProgressLastFlush = useRef(0);
  const [crawlStatus, setCrawlStatus] = useState<CrawlStatusEvent>(INITIAL_CRAWL_STATUS);
  const [downloadBatchStatus, setDownloadBatchStatus] = useState<DownloadBatchStatus>(INITIAL_DOWNLOAD_BATCH_STATUS);
  const [resourceMetrics, setResourceMetrics] = useState<ResourceMetricsSnapshot>(INITIAL_RESOURCE_METRICS);
  const [lastCrawlResult, setLastCrawlResult] = useState<CrawlSessionResult | null>(null);
  const [downloadResumePlan, setDownloadResumePlan] = useState<DownloadResumePlan | null>(null);
  const [logAggregates, setLogAggregates] = useState<Record<string, LogAggregate>>({});
  const [selectedFiles, setSelectedFiles] = useState<FileEntry[]>([]);
  const [toasts, setToasts] = useState<ToastInfo[]>([]);
  const [outputDir, setOutputDir] = useState("");
  const [activeDaemons, setActiveDaemons] = useState<number>(4);
  const [crawlStartTime, setCrawlStartTime] = useState<number | null>(null);
  const [crawlElapsed, setCrawlElapsed] = useState(0);
  const [downloadElapsed, setDownloadElapsed] = useState(0);
  const [showSupportPopover, setShowSupportPopover] = useState(false);
  const [supportCatalog, setSupportCatalog] = useState<AdapterSupportInfo[]>([]);
  const [supportCatalogError, setSupportCatalogError] = useState<string | null>(null);
  const [efficiencyHistory, setEfficiencyHistory] = useState<EfficiencyHistory>(INITIAL_EFFICIENCY_HISTORY);

  const urlInputRef = useRef<HTMLInputElement>(null);
  const previewNoticeShownRef = useRef(false);
  const fixtureNoticeShownRef = useRef(false);
  const supportButtonRef = useRef<HTMLButtonElement>(null);
  const supportPopoverRef = useRef<HTMLDivElement>(null);
  const batchSpeedSampleRef = useRef<{ ts: number; bytes: number } | null>(null);
  const aggregateDownloadBytesRef = useRef(0);
  const aggregateDiskSampleRef = useRef<{ ts: number; bytes: number } | null>(null);
  const perFileDownloadedBytesRef = useRef<Record<string, number>>({});
  const activeDownloadOutputDirRef = useRef("");
  const processedLogCountRef = useRef(0);
  const lastEfficiencySampleRef = useRef("");
  const telemetryRuntimeRef = useRef<Promise<{
    pb: typeof import("./telemetry.js");
    Reader: typeof import("protobufjs/minimal.js").Reader;
  }> | null>(null);
  // Phase 74B: Adaptive ceiling tracking for Dashboard
  const [ceilingStatus, setCeilingStatus] = useState<{ value: number; direction: 'DECAY' | 'RECOVERY' | null; lastChange: number | null }>({
    value: 0, direction: null, lastChange: null
  });

  // Phase 116: Patient Retry Mode state
  const [patientRetryState, setPatientRetryState] = useState<PatientRetryState>({
    active: false,
    intervalMins: 15,
    maxRetries: 96,
    totalNodes: 0,
    rounds: [],
    startedAt: null,
    countdownSeconds: 0,
    currentRound: 0,
  });
  const patientRetryCountdownRef = useRef<ReturnType<typeof setInterval> | null>(null);

  const [crawlOptions, setCrawlOptions] = useState({
    listing: true,
    sizes: true,
    download: false,
    circuits: 24,
    agnosticState: false,
    resume: false,
    resumeIndex: undefined as string | undefined,
    stealthRamp: true,
    forceClearnet: false,
    parallelDownload: false,
    downloadMode: 'default' as 'default' | 'high' | 'aggressive',
  });

  useEffect(() => {
    if (!isTauriRuntime) {
      return;
    }

    let cancelled = false;

    const runNativeSmokeProbe = async () => {
      try {
        const config = await invokeCommand<NativeWebviewSmokeConfig>("get_native_smoke_config");
        if (!config.enabled) {
          return;
        }

        const expectedTestIds = config.expectedTestIds.length > 0
          ? config.expectedTestIds
          : [...NATIVE_WEBVIEW_SMOKE_TEST_IDS];
        const deadline = Date.now() + Math.max(config.waitMs, 1_000);
        let foundTestIds: string[] = [];
        let missingTestIds = [...expectedTestIds];

        while (!cancelled && Date.now() <= deadline) {
          foundTestIds = expectedTestIds.filter((testId) => document.querySelector(`[data-testid="${testId}"]`));
          missingTestIds = expectedTestIds.filter((testId) => !foundTestIds.includes(testId));
          if (missingTestIds.length === 0) {
            break;
          }
          await wait(120);
        }

        if (cancelled) {
          return;
        }

        const result: NativeWebviewSmokeResult = {
          mounted: missingTestIds.length === 0,
          title: document.title,
          href: window.location.href,
          isTauriRuntime,
          expectedTestIds,
          foundTestIds,
          missingTestIds,
          reportedAtEpochMs: Date.now(),
        };

        await invokeCommand("report_native_smoke_result", { result });
      } catch (error) {
        console.error("[native-smoke] failed to report Tauri smoke state", error);
      }
    };

    void runNativeSmokeProbe();
    return () => {
      cancelled = true;
    };
  }, [isTauriRuntime]);
  const [systemProfile, setSystemProfile] = useState<{
    preset: string; circuits: number; workers: number;
    cpuCores: number; totalRamGb: number; availableRamGb: number;
    storageClass: string; os: string;
  } | null>(null);

  const showToast = (type: "success" | "error", title: string, message: string) => {
    const id = Date.now();
    setToasts((prev) => [...prev, { id, type, title, message }]);
    setTimeout(() => {
      setToasts((prev) => prev.filter((t) => t.id !== id));
    }, 6000);
  };

  useEffect(() => {
    document.title = "Crawli Engine";
  }, []);

  // Phase 67H: Auto-detect system profile and set recommended concurrency
  useEffect(() => {
    if (!isTauriRuntime) return;
    (async () => {
      try {
        const profile = await invokeCommand<{
          preset: string; circuits: number; workers: number;
          cpuCores: number; totalRamGb: number; availableRamGb: number;
          storageClass: string; os: string;
        }>("get_system_profile");
        setSystemProfile(profile);
        // Phase 140D: Auto-detect download mode based on RAM
        const autoMode = profile.totalRamGb > 20 ? 'aggressive' : profile.totalRamGb > 8.5 ? 'high' : 'default';
        setCrawlOptions(prev => ({ ...prev, downloadMode: autoMode as 'default' | 'high' | 'aggressive' }));
        setLogs(l => [...l, `[SYSTEM] Auto-detected: ${profile.os} ${profile.cpuCores}c/${profile.totalRamGb}GB/${profile.storageClass.toUpperCase()} → ${autoMode} mode`]);
      } catch {
        // Fallback: keep default 8 circuits
      }
    })();
  }, []);

  // Crawl duration timer
  useEffect(() => {
    if (!isCrawling || !crawlStartTime) return;
    const interval = setInterval(() => {
      setCrawlElapsed(Date.now() - crawlStartTime);
    }, 1000);
    return () => clearInterval(interval);
  }, [isCrawling, crawlStartTime]);

  // Pre-resolve onion descriptors 500ms after user finishes typing
  useEffect(() => {
    if (isOnionTarget(url)) {
      const timer = setTimeout(() => {
        if (isTauriRuntime) {
          invokeCommand('pre_resolve_onion', { url }).catch(console.error);
        }
      }, 500);
      return () => clearTimeout(timer);
    }
  }, [url, isTauriRuntime]);

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
    setResourceMetrics(FIXTURE_RESOURCE_METRICS);
    setCrawlStatus((prev) => ({
      ...prev,
      vanguard: { current: 6, target: 12, status: "Active (Heatmap Enabled)" }
    }));
    setLogs((l) => [
      ...l.slice(-399),
      "[SYSTEM] Fixture VFS mode enabled for browser integrity testing.",
    ]);
  }, [isFixtureMode]);

  useEffect(() => {
    const totalRequests = resourceMetrics.totalRequests || 0;
    const successfulRequests = resourceMetrics.successfulRequests || 0;
    const failedRequests = resourceMetrics.failedRequests || 0;
    const fingerprintLatencyMs = resourceMetrics.fingerprintLatencyMs || 0;
    const cachedRouteHits = resourceMetrics.cachedRouteHits || 0;
    const activeCircuits = resourceMetrics.activeCircuits || 0;
    const shouldTrack =
      isCrawling ||
      totalRequests > 0 ||
      fingerprintLatencyMs > 0 ||
      cachedRouteHits > 0;

    if (!shouldTrack) {
      return;
    }

    const requestsPerEntry =
      totalRequests > 0 ? totalRequests / Math.max(vfsStats.totalNodes, 1) : 0;
    const settledRequests = successfulRequests + failedRequests;
    const requestSuccessRate =
      settledRequests > 0 ? (successfulRequests / settledRequests) * 100 : 0;
    const sampleKey = [
      requestsPerEntry.toFixed(4),
      requestSuccessRate.toFixed(2),
      activeCircuits,
      fingerprintLatencyMs,
      cachedRouteHits,
    ].join("|");

    if (sampleKey === lastEfficiencySampleRef.current) {
      return;
    }
    lastEfficiencySampleRef.current = sampleKey;

    setEfficiencyHistory((previous) => ({
      requestsPerEntry: [...previous.requestsPerEntry.slice(-39), requestsPerEntry],
      requestSuccessRate: [...previous.requestSuccessRate.slice(-39), requestSuccessRate],
      activeCircuits: [...previous.activeCircuits.slice(-39), activeCircuits],
      fingerprintLatencyMs: [...previous.fingerprintLatencyMs.slice(-39), fingerprintLatencyMs],
    }));
  }, [
    isCrawling,
    vfsStats.totalNodes,
    resourceMetrics.totalRequests,
    resourceMetrics.successfulRequests,
    resourceMetrics.failedRequests,
    resourceMetrics.activeCircuits,
    resourceMetrics.fingerprintLatencyMs,
    resourceMetrics.cachedRouteHits,
  ]);

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
        const dl = await getDownloadDir();
        const defaultPath = await joinPath(dl, "OnionForger_Downloads");
        const normalizedDefaultPath = stripWindowsVerbatimPrefix(defaultPath);
        setOutputDir((prev) => prev || normalizedDefaultPath);
      } catch (e) {
        console.warn("Could not retrieve download directory", e);
      }
    }
    initPaths();
    const unlistenPromises: Array<Promise<() => void>> = [];

    if (isTauriRuntime) {
      unlistenPromises.push(
        listenEvent<string>("crawl_log", (event) => {
          const payload = event.payload;
          const adapterMatch = payload.match(/Match found:\s*(.+)$/);
          if (adapterMatch && adapterMatch[1]) {
            setActiveAdapter(adapterMatch[1].trim());
          }
          // Phase 74B: Extract ceiling change events
          const ceilingMatch = payload.match(/\[PHASE 74\] Adaptive ceiling (DECAY|RECOVERY): (\d+) → (\d+)/);
          if (ceilingMatch) {
            setCeilingStatus({
              value: parseInt(ceilingMatch[3]),
              direction: ceilingMatch[1] as 'DECAY' | 'RECOVERY',
              lastChange: Date.now()
            });
          }
          // Also extract ceiling= from governor logs
          const govCeilingMatch = payload.match(/ceiling=(\d+)/);
          if (govCeilingMatch && !ceilingMatch) {
            setCeilingStatus(prev => ({ ...prev, value: parseInt(govCeilingMatch[1]) }));
          }
          setLogs((l) => [...l.slice(-399), `> ${payload}`]);
        })
      );
      unlistenPromises.push(
        listenEvent<string>("log", (event) => {
          setLogs((l) => [...l.slice(-399), `> ${event.payload}`]);
        })
      );

      // Phase 116: Patient Retry Mode event listeners
      unlistenPromises.push(
        listenEvent<{ interval_mins: number; max_retries: number; total_nodes: number }>("patient_retry_started", (event) => {
          const p = event.payload;
          setPatientRetryState({
            active: true,
            intervalMins: p.interval_mins,
            maxRetries: p.max_retries,
            totalNodes: p.total_nodes,
            rounds: [],
            startedAt: Date.now(),
            countdownSeconds: p.interval_mins * 60,
            currentRound: 0,
          });
          // Start countdown timer
          if (patientRetryCountdownRef.current) clearInterval(patientRetryCountdownRef.current);
          patientRetryCountdownRef.current = setInterval(() => {
            setPatientRetryState(prev => ({
              ...prev,
              countdownSeconds: Math.max(0, prev.countdownSeconds - 1),
            }));
          }, 1000);
        })
      );
      unlistenPromises.push(
        listenEvent<{ round: number; max_retries: number; wait_mins: number }>("patient_retry_waiting", (event) => {
          const p = event.payload;
          setPatientRetryState(prev => ({
            ...prev,
            currentRound: p.round,
            countdownSeconds: p.wait_mins * 60,
            rounds: prev.rounds.some(r => r.round === p.round)
              ? prev.rounds.map(r => r.round === p.round ? { ...r, status: "waiting" as const } : r)
              : [...prev.rounds, { round: p.round, status: "waiting" as const, resetNodes: 0, timestamp: Date.now(), nextRetryMins: p.wait_mins }],
          }));
        })
      );
      unlistenPromises.push(
        listenEvent<{ round: number; reset_nodes: number }>("patient_retry_probing", (event) => {
          const p = event.payload;
          setPatientRetryState(prev => ({
            ...prev,
            countdownSeconds: 0,
            rounds: prev.rounds.map(r =>
              r.round === p.round
                ? { ...r, status: "probing" as const, resetNodes: p.reset_nodes, timestamp: Date.now() }
                : r
            ),
          }));
        })
      );
      unlistenPromises.push(
        listenEvent<{ round: number; host: string; latency_ms: number }>("patient_retry_success", (event) => {
          const p = event.payload;
          if (patientRetryCountdownRef.current) {
            clearInterval(patientRetryCountdownRef.current);
            patientRetryCountdownRef.current = null;
          }
          setPatientRetryState(prev => ({
            ...prev,
            active: false,
            countdownSeconds: 0,
            rounds: prev.rounds.map(r =>
              r.round === p.round
                ? { ...r, status: "success" as const, host: p.host, latencyMs: p.latency_ms, timestamp: Date.now() }
                : r
            ),
          }));
        })
      );
      unlistenPromises.push(
        listenEvent<{ round: number; max_retries: number; next_retry_mins: number }>("patient_retry_failed", (event) => {
          const p = event.payload;
          const exhausted = p.round >= p.max_retries;
          if (exhausted && patientRetryCountdownRef.current) {
            clearInterval(patientRetryCountdownRef.current);
            patientRetryCountdownRef.current = null;
          }
          setPatientRetryState(prev => ({
            ...prev,
            active: !exhausted,
            rounds: prev.rounds.map(r =>
              r.round === p.round
                ? { ...r, status: "failed" as const, nextRetryMins: p.next_retry_mins, timestamp: Date.now() }
                : r
            ),
          }));
        })
      );
      unlistenPromises.push(
        listenEvent<{ round: number }>("patient_retry_cancelled", (event) => {
          const p = event.payload;
          if (patientRetryCountdownRef.current) {
            clearInterval(patientRetryCountdownRef.current);
            patientRetryCountdownRef.current = null;
          }
          setPatientRetryState(prev => ({
            ...prev,
            active: false,
            countdownSeconds: 0,
            rounds: prev.rounds.map(r =>
              r.round === p.round
                ? { ...r, status: "cancelled" as const, timestamp: Date.now() }
                : r
            ),
          }));
        })
      );

      unlistenPromises.push(
        listenEvent<FileEntry[]>("crawl_progress", (event) => {
          // Stream directly to backend DB
          invokeCommand("ingest_vfs_entries", { entries: event.payload }).catch(console.error);

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

      const applyBatchProgress = (rawPayload: BatchProgressEvent) => {
        const payload = rawPayload as BatchProgressEvent & {
          speed_mbps?: number;
          downloaded_bytes?: number;
          current_file?: string;
          active_circuits?: number;
          bbr_bottleneck_mbps?: number;
          ekf_covariance?: number;
        };
        const speedMbpsRaw = payload.speedMbps ?? payload.speed_mbps;
        const downloadedBytesRaw = payload.downloadedBytes ?? payload.downloaded_bytes;
        const currentFileRaw = payload.currentFile ?? payload.current_file;
        const activeCircuitsRaw = payload.activeCircuits ?? payload.active_circuits;
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
          const mergedDownloadedBytes = Math.max(
            prev.downloadedBytes,
            downloadedBytesRaw || 0,
            aggregateDownloadBytesRef.current,
          );
          aggregateDownloadBytesRef.current = mergedDownloadedBytes;

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

          let diskWriteMbps = prev.diskWriteMbps;
          if (aggregateDiskSampleRef.current) {
            const sample = aggregateDiskSampleRef.current;
            const deltaBytes = Math.max(0, mergedDownloadedBytes - sample.bytes);
            const deltaSeconds = Math.max((now - sample.ts) / 1000, 0);
            if (deltaBytes > 0 && deltaSeconds > 0) {
              diskWriteMbps = (deltaBytes / deltaSeconds) / 1048576;
            }
          }
          aggregateDiskSampleRef.current = { ts: now, bytes: mergedDownloadedBytes };

          if (prev.startedAt === null) {
            setDownloadElapsed(0);
          }

          const roots = [activeDownloadOutputDirRef.current, prev.outputDir, outputDir];
          const currentFile = currentFileRaw ? toDisplayPath(currentFileRaw, roots) : prev.currentFile;
          const activeCircuits = activeCircuitsRaw ?? prev.activeCircuits;
          const peakBandwidthMbps = Math.max(prev.peakBandwidthMbps, resolvedSpeedMbps || 0);
          const peakDiskWriteMbps = Math.max(prev.peakDiskWriteMbps, diskWriteMbps || 0);
          const instant = Math.max(0, resolvedSpeedMbps || 0);
          const smoothedSpeedMbps =
            prev.smoothedSpeedMbps > 0
              ? prev.smoothedSpeedMbps * 0.72 + instant * 0.28
              : instant;
          const progressFactor = totalFiles > 0 ? done / totalFiles : 0;
          const unknownFactor =
            totalFiles > 0 ? 1 - (prev.unknownSizeFiles / totalFiles) : 0.5;
          const speedStability =
            smoothedSpeedMbps > 0
              ? 1 - Math.min(Math.abs(instant - smoothedSpeedMbps) / smoothedSpeedMbps, 1)
              : 0;
          const etaConfidence = Math.max(
            0.05,
            Math.min(0.99, progressFactor * 0.45 + unknownFactor * 0.20 + speedStability * 0.35),
          );

          return {
            ...prev,
            totalFiles,
            completedFiles,
            failedFiles,
            currentFile,
            speedMbps: resolvedSpeedMbps,
            smoothedSpeedMbps,
            downloadedBytes: mergedDownloadedBytes,
            activeCircuits,
            peakActiveCircuits: Math.max(prev.peakActiveCircuits, activeCircuits || 0),
            peakBandwidthMbps,
            diskWriteMbps,
            peakDiskWriteMbps,
            etaConfidence,
            outputDir: prev.outputDir || activeDownloadOutputDirRef.current || outputDir,
            bbrBottleneckMbps: bbrRaw,
            ekfCovariance: ekfRaw,
            startedAt,
            etaSeconds,
          };
        });
      };

      const applyDownloadProgress = (rawPayload: DownloadProgressEvent) => {
        const payload = rawPayload as DownloadProgressEvent & {
          bytesDownloaded?: number;
          totalBytes?: number | null;
          speedBps?: number;
          activeCircuits?: number;
        };
        const bytesDownloaded = payload.bytes_downloaded ?? payload.bytesDownloaded ?? 0;
        const totalBytes = payload.total_bytes ?? payload.totalBytes ?? null;
        const speedBps = payload.speed_bps ?? payload.speedBps ?? 0;
        const activeCircuitsValue = payload.active_circuits ?? payload.activeCircuits ?? 0;
        const roots = [activeDownloadOutputDirRef.current, outputDir];
        const displayPath = toDisplayPath(payload.path, roots);
        const normalizedPayload: DownloadProgressEvent = {
          ...payload,
          path: displayPath,
          bytes_downloaded: bytesDownloaded,
          total_bytes: totalBytes,
          speed_bps: speedBps,
          active_circuits: activeCircuitsValue,
        };

        // Phase 128B: Buffer per-file progress in ref (zero re-renders), flush to state every 500ms
        downloadProgressBufferRef.current[displayPath] = normalizedPayload;
        const progressNow = Date.now();
        if (!downloadProgressFlushPending.current && progressNow - downloadProgressLastFlush.current > 500) {
          downloadProgressFlushPending.current = true;
          requestAnimationFrame(() => {
            setDownloadProgress({ ...downloadProgressBufferRef.current });
            downloadProgressLastFlush.current = Date.now();
            downloadProgressFlushPending.current = false;
          });
        }
        const previousBytes = perFileDownloadedBytesRef.current[displayPath] || 0;
        const nextBytes = Math.max(previousBytes, bytesDownloaded);
        if (nextBytes > previousBytes) {
          perFileDownloadedBytesRef.current[displayPath] = nextBytes;
          aggregateDownloadBytesRef.current += nextBytes - previousBytes;
        }
        const aggregateBytes = aggregateDownloadBytesRef.current;
        const now = Date.now();
        let diskWriteMbps = 0;
        if (aggregateDiskSampleRef.current) {
          const sample = aggregateDiskSampleRef.current;
          const deltaBytes = Math.max(0, aggregateBytes - sample.bytes);
          const deltaSeconds = Math.max((now - sample.ts) / 1000, 0);
          if (deltaBytes > 0 && deltaSeconds > 0) {
            diskWriteMbps = (deltaBytes / deltaSeconds) / 1048576;
          }
        }
        aggregateDiskSampleRef.current = { ts: now, bytes: aggregateBytes };

        const speedMbps = Math.max(0, speedBps / 1048576);
        const activeCircuits = Math.max(0, activeCircuitsValue);
        setDownloadBatchStatus((prev) => {
          const done = prev.completedFiles + prev.failedFiles;
          const total = Math.max(prev.totalFiles, 1);
          const progressFactor = done / total;
          const unknownFactor = 1 - (prev.unknownSizeFiles / total);
          const instant = speedMbps > 0 ? speedMbps : prev.speedMbps;
          const smoothedSpeedMbps =
            instant > 0
              ? (prev.smoothedSpeedMbps > 0
                ? prev.smoothedSpeedMbps * 0.72 + instant * 0.28
                : instant)
              : prev.smoothedSpeedMbps;
          const speedStability =
            smoothedSpeedMbps > 0
              ? 1 - Math.min(Math.abs(instant - smoothedSpeedMbps) / smoothedSpeedMbps, 1)
              : 0;
          return {
            ...prev,
            currentFile: displayPath || prev.currentFile,
            downloadedBytes: Math.max(prev.downloadedBytes, aggregateBytes),
            speedMbps: instant,
            smoothedSpeedMbps,
            activeCircuits,
            peakActiveCircuits: Math.max(prev.peakActiveCircuits, activeCircuits),
            peakBandwidthMbps: Math.max(prev.peakBandwidthMbps, speedMbps),
            diskWriteMbps: diskWriteMbps > 0 ? diskWriteMbps : prev.diskWriteMbps,
            peakDiskWriteMbps: Math.max(prev.peakDiskWriteMbps, diskWriteMbps),
            etaConfidence: Math.max(
              0.05,
              Math.min(0.99, progressFactor * 0.45 + unknownFactor * 0.20 + speedStability * 0.35),
            ),
          };
        });
      };

      unlistenPromises.push(
        listenEvent<DownloadBatchStartedEvent>("download_batch_started", (event) => {
          const startedAt = Date.now();
          const normalizedOutput = stripWindowsVerbatimPrefix(event.payload.outputDir || "");
          activeDownloadOutputDirRef.current = normalizedOutput;
          aggregateDownloadBytesRef.current = 0;
          aggregateDiskSampleRef.current = { ts: startedAt, bytes: 0 };
          perFileDownloadedBytesRef.current = {};
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
            smoothedSpeedMbps: 0,
            downloadedBytes: 0,
            activeCircuits: 0,
            peakActiveCircuits: 0,
            peakBandwidthMbps: 0,
            diskWriteMbps: 0,
            peakDiskWriteMbps: 0,
            etaConfidence: 0,
            outputDir: normalizedOutput,
            bbrBottleneckMbps: 0,
            ekfCovariance: 0,
            startedAt,
            etaSeconds: null,
          });
        })
      );

      // Phase 143: Progressive download total update.
      // As crawling discovers new files during parallel download, the backend
      // emits download_total_update to grow the total WITHOUT resetting progress.
      // This keeps completedFiles, downloadedBytes, speed, etc. intact.
      unlistenPromises.push(
        listenEvent<{ totalFiles: number; totalBytesHint: number; unknownSizeFiles: number }>("download_total_update", (event) => {
          setDownloadBatchStatus((prev) => ({
            ...prev,
            totalFiles: Math.max(prev.totalFiles, event.payload.totalFiles || 0),
            totalBytesHint: Math.max(prev.totalBytesHint, event.payload.totalBytesHint || 0),
            unknownSizeFiles: Math.max(prev.unknownSizeFiles, event.payload.unknownSizeFiles || 0),
          }));
        })
      );

      const pollId = setInterval(async () => {
        try {
          if (!telemetryRuntimeRef.current) {
            telemetryRuntimeRef.current = Promise.all([
              import("./telemetry.js"),
              import("protobufjs/minimal.js"),
            ]).then(([pb, protobuf]) => ({
              pb,
              Reader: protobuf.Reader,
            }));
          }
          const { pb, Reader } = await telemetryRuntimeRef.current;
          const buffer = await invokeCommand<Uint8Array>("drain_telemetry_ring");
          if (!buffer || buffer.length === 0) return;

          const reader = Reader.create(buffer);
          let crawlStatusUpdated = false;
          let resourceMetricsUpdated = false;
          let batchUpdated = false;

          let lastCrawlStatus: any = null;
          let lastResourceMetrics: any = null;
          let lastBatchProgress: any = null;
          const downloadProgresses: any[] = [];

          while (reader.pos < reader.len) {
            const frame = pb.TelemetryFrame.decodeDelimited(reader);
            const payloadReader = Reader.create(frame.payload);

            switch (frame.kind) {
              case 1:
                lastResourceMetrics = pb.ResourceMetricsFrame.toObject(
                  pb.ResourceMetricsFrame.decode(payloadReader),
                  { longs: Number, defaults: true },
                );
                resourceMetricsUpdated = true;
                break;
              case 2:
                lastCrawlStatus = pb.CrawlStatusFrame.toObject(
                  pb.CrawlStatusFrame.decode(payloadReader),
                  { longs: Number, defaults: true },
                );
                crawlStatusUpdated = true;
                break;
              case 3:
                lastBatchProgress = pb.BatchProgressFrame.toObject(
                  pb.BatchProgressFrame.decode(payloadReader),
                  { longs: Number, defaults: true },
                );
                batchUpdated = true;
                break;
              case 4:
                downloadProgresses.push(pb.DownloadStatusFrame.toObject(pb.DownloadStatusFrame.decode(payloadReader), { longs: Number }));
                break;
            }
          }

          if (crawlStatusUpdated) {
            setCrawlStatus((previous) =>
              normalizeCrawlStatusFrame(
                lastCrawlStatus as Partial<CrawlStatusEvent>,
                previous,
              ),
            );
          }
          if (resourceMetricsUpdated) {
            setResourceMetrics((previous) =>
              normalizeResourceMetricsFrame(
                lastResourceMetrics as Partial<ResourceMetricsSnapshot>,
                previous,
              ),
            );
          }
          if (batchUpdated) {
            applyBatchProgress({
              ...lastBatchProgress,
              downloadedBytes: lastBatchProgress.downloadedBytes || 0,
            } as any);
          }
          downloadProgresses.forEach((dp) => applyDownloadProgress({
            path: dp.message,
            bytesDownloaded: 0,
            totalBytes: null,
            speedBps: 0,
            activeCircuits: 0,
            ...dp
          } as any));

        } catch (err) {
          console.error("Telemetry ring poll failed:", err);
        }
      }, 250);

      unlistenPromises.push(Promise.resolve(() => clearInterval(pollId)));

      unlistenPromises.push(
        listenEvent<TelemetryBridgeUpdate>("telemetry_bridge_update", (event) => {
          if (event.payload.resourceMetrics) {
            setResourceMetrics((previous) =>
              normalizeResourceMetricsFrame(event.payload.resourceMetrics, previous),
            );
          }
          if (event.payload.crawlStatus) {
            setCrawlStatus((previous) =>
              normalizeCrawlStatusFrame(event.payload.crawlStatus, previous),
            );
          }
        })
      );

      unlistenPromises.push(
        listenEvent<TorStatus>("tor_status", (event) => {
          setTorStatus(event.payload);
          const readyClients = event.payload.ready_clients ?? event.payload.daemon_count;
          if (readyClients) {
            setActiveDaemons(readyClients);
          }
          if (event.payload.state === "completed_local" || event.payload.state === "completed_managed") {
            setTorStatus(null);
          }
          setLogs((l) => [...l.slice(-399), `[TOR] ${event.payload.state.toUpperCase()}: ${event.payload.message}`]);
        })
      );

      unlistenPromises.push(
        listenEvent<DownloadResumePlan>("download_resume_plan", (event) => {
          setDownloadResumePlan(event.payload);
        })
      );

      unlistenPromises.push(
        listenEvent<{ url: string; path: string; hash: string; time_taken_secs: number }>("complete", (event) => {
          const roots = [activeDownloadOutputDirRef.current, outputDir];
          const displayPath = toDisplayPath(event.payload.path, roots);
          setLogs((l) => [...l.slice(-399), `[✓] File verified: ${displayPath} (SHA256: ${event.payload.hash})`]);
          // Phase 134: During batch downloads, per-file completion must NOT say "Download Finished"
          // because it misleads the user into thinking the entire batch is done.
          setDownloadBatchStatus((prev) => {
            const done = prev.completedFiles + prev.failedFiles + 1;
            if (prev.totalFiles > 1) {
              showToast("success", "File Verified", `${displayPath} (${done}/${prev.totalFiles})`);
            } else {
              showToast("success", "Download Finished", `File saved and verified (${event.payload.hash})`);
            }
            return { ...prev, completedFiles: Math.max(prev.completedFiles, done) };
          });

          // Phase 128B: Mark completed in buffer + aggregate tracking
          const existingProgress = downloadProgressBufferRef.current[displayPath];
          const completedBytes = existingProgress?.total_bytes || existingProgress?.bytes_downloaded || 0;
          downloadProgressBufferRef.current[displayPath] = {
            path: displayPath,
            bytes_downloaded: completedBytes,
            total_bytes: completedBytes > 0 ? completedBytes : null,
            speed_bps: 0,
          };

          const previousBytes = perFileDownloadedBytesRef.current[displayPath] || 0;
          if (completedBytes > previousBytes) {
            perFileDownloadedBytesRef.current[displayPath] = completedBytes;
            aggregateDownloadBytesRef.current += completedBytes - previousBytes;
          }

          // Throttled flush to React state
          const now2 = Date.now();
          if (!downloadProgressFlushPending.current && now2 - downloadProgressLastFlush.current > 500) {
            downloadProgressFlushPending.current = true;
            requestAnimationFrame(() => {
              setDownloadProgress({ ...downloadProgressBufferRef.current });
              downloadProgressLastFlush.current = Date.now();
              downloadProgressFlushPending.current = false;
            });
          }

          setDownloadBatchStatus((prev) => ({
            ...prev,
            downloadedBytes: Math.max(prev.downloadedBytes, aggregateDownloadBytesRef.current),
          }));
        })
      );

      unlistenPromises.push(
        listenEvent<{ url: string; path: string; error: string }>("download_failed", (event) => {
          const displayPath = toDisplayPath(event.payload.path, [activeDownloadOutputDirRef.current, outputDir]);
          setLogs((l) => [...l.slice(-399), `[ERROR] Download failed for ${displayPath}: ${event.payload.error}`]);
          showToast("error", "Download Failed", event.payload.error);
        })
      );

      unlistenPromises.push(
        listenEvent<{ url: string; path: string; reason: string }>("download_interrupted", (event) => {
          const displayPath = toDisplayPath(event.payload.path, [activeDownloadOutputDirRef.current, outputDir]);
          setLogs((l) => [...l.slice(-399), `[SYSTEM] Download interrupted for ${displayPath}: ${event.payload.reason}`]);
          showToast("success", "Download Interrupted", `${event.payload.reason} for ${displayPath}`);
        })
      );

      // Phase 52E: Mega/Torrent progress listeners
      unlistenPromises.push(
        listenEvent<any>("mega_download_progress", (event) => {
          setMegaProgress(event.payload);
          if (event.payload.status === "done" || event.payload.status === "error") {
            // Clear progress card after last file completes
            if (event.payload.index >= event.payload.total) {
              setTimeout(() => setMegaProgress(null), 3000);
            }
          }
        })
      );
      unlistenPromises.push(
        listenEvent<any>("torrent_download_progress", (event) => {
          setTorrentProgress(event.payload);
          if (event.payload.status === "complete") {
            setTimeout(() => setTorrentProgress(null), 3000);
          }
        })
      );
    } else if (!isFixtureMode && !previewNoticeShownRef.current) {
      previewNoticeShownRef.current = true;
      setResourceMetrics(INITIAL_RESOURCE_METRICS);
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
    if (logs.length <= processedLogCountRef.current) return;
    const newLogs = logs.slice(processedLogCountRef.current);
    processedLogCountRef.current = logs.length;

    setLogAggregates((prev) => {
      const next = { ...prev };
      const baseTs = Date.now();
      newLogs.forEach((message, idx) => {
        const key = summarizeLogKey(message);
        const ts = baseTs + idx;
        const existing = next[key];
        next[key] = existing
          ? {
            ...existing,
            count: existing.count + 1,
            lastSeenAt: ts,
            sample: message,
          }
          : {
            key,
            sample: message,
            count: 1,
            firstSeenAt: ts,
            lastSeenAt: ts,
          };
      });
      return next;
    });
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

  const handleCrawl = useCallback(async (resumeMode: boolean = false) => {
    if (!url) return;
    const preserveFixtureState = isFixtureMode;
    processedLogCountRef.current = logs.length;
    setLogAggregates({});
    setIsCrawling(true);
    setActiveAdapter("Unidentified");

    setCrawlStartTime(Date.now());
    setCrawlElapsed(0);
    setDownloadProgress({});
    downloadProgressBufferRef.current = {};
    setDownloadBatchStatus(INITIAL_DOWNLOAD_BATCH_STATUS);
    setLastCrawlResult(null);
    setDownloadResumePlan(null);
    aggregateDownloadBytesRef.current = 0;
    aggregateDiskSampleRef.current = null;
    perFileDownloadedBytesRef.current = {};
    activeDownloadOutputDirRef.current = "";
    batchSpeedSampleRef.current = null;
    setDownloadElapsed(0);
    if (!preserveFixtureState) {
      setResourceMetrics(INITIAL_RESOURCE_METRICS);
    }
    setEfficiencyHistory(INITIAL_EFFICIENCY_HISTORY);
    lastEfficiencySampleRef.current = "";
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
        circuits: crawlOptions.circuits > 0 ? crawlOptions.circuits : null,
        resume: resumeMode,
        mega_password: megaPassword || null,
      };

      const result = await invokeCommand<CrawlSessionResult>("start_crawl", { url, options: payloadOptions, outputDir });
      setLastCrawlResult(result);
      setLogs((l) => [...l, `[SYSTEM] Finish signaled. Found ${result.discoveredCount} unique nodes.`]);
      setLogs((l) => [...l, `[SYSTEM] Crawl baseline status: ${result.crawlOutcome} | raw=${result.rawThisRunCount} | best=${result.bestPriorCount} | merged=${result.mergedEffectiveCount} | retries=${result.retryCountUsed}`]);
      setLogs((l) => [...l, `[SYSTEM] Stable current listing: ${result.stableCurrentListingPath}`]);
      setLogs((l) => [...l, `[SYSTEM] Stable best listing: ${result.stableBestListingPath}`]);
      showToast("success", "Crawl Finished", `Operations complete. Extracted ${result.discoveredCount} nodes from source.`);

      if (crawlOptions.download) {
        setLogs((l) => [...l, `[OPSEC] Auto-Mirror complete. Files scaffolded to ${result.outputDir || outputDir}`]);
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
          speedMbps: 0,
          activeCircuits: 0,
          etaSeconds: null,
        }));
        setResourceMetrics((prev) => ({
          ...prev,
          activeWorkers: 0,
          workerTarget: 0,
          activeCircuits: 0,
        }));
        setLogs((l) => [...l, `[SYSTEM] Cancel acknowledged in preview mode (no native crawl workers active).`]);
        showToast("success", "Cancel Acknowledged", "Preview mode has no active native crawl workers.");
        return;
      }
      const result = await invokeCommand<string>("cancel_crawl");
      setIsCrawling(false);
      setCrawlStartTime(null);
      setCrawlStatus((prev) => ({ ...prev, phase: "cancelled" }));
      setDownloadBatchStatus((prev) => ({
        ...prev,
        currentFile: "cancelled",
        speedMbps: 0,
        activeCircuits: 0,
        etaSeconds: null,
      }));
      setResourceMetrics((prev) => ({
        ...prev,
        activeWorkers: 0,
        workerTarget: 0,
        activeCircuits: 0,
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
        const count = await invokeCommand<number>("download_files", {
          entries: [entry],
          outputDir,
          connections: crawlOptions.circuits,
        });
        showToast("success", "Download Complete", `${count} item(s) saved to ${outputDir}`);
        setLogs((l) => [...l, `[MIRROR] Saved ${filePath} to disk`]);
      } else {
        // High concurrency chunked download for single files
        await invokeCommand("initiate_download", {
          args: {
            url: rawUrl,
            path: filePath,
            output_root: outputDir,
            connections: crawlOptions.circuits || 8,
            force_tor: isOnionTarget(rawUrl),
          }
        });
        showToast("success", "Download Engine Started", `Allocating ${crawlOptions.circuits || 8} Multi-Clients to target...`);
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
      const count = await invokeCommand<number>("download_files", {
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
      const savePath = await saveDialog({
        defaultPath: "crawl_results.json",
        filters: [{ name: "JSON", extensions: ["json"] }],
        title: "Export Crawl Results",
      });
      if (!savePath) return;

      const result = await invokeCommand<string>("export_json", { outputPath: savePath });
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
      const count = await invokeCommand<number>("download_all", {
        outputDir,
        connections: crawlOptions.circuits,
        targetUrl: url || undefined,
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
      const selected = await openDialog({
        directory: true,
        multiple: false,
        title: "Select Download Location",
      });
      if (selected && typeof selected === "string") {
        const normalizedSelected = stripWindowsVerbatimPrefix(selected);
        setOutputDir(normalizedSelected);
        setLogs((l) => [...l.slice(-399), `[PATH] Output location set to: ${normalizedSelected}`]);
        showToast("success", "Storage Linked", `Updated target extraction path to ${normalizedSelected}`);
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
        const catalog = await invokeCommand<AdapterSupportInfo[]>("get_adapter_support_catalog");
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
  const aggregateRows = Object.values(logAggregates).sort((a, b) => {
    if (b.count !== a.count) return b.count - a.count;
    return b.lastSeenAt - a.lastSeenAt;
  });
  const previewLogs = logs.length > 0 ? logs : ["[SYSTEM] Browser preview ready."];

  return (
    <div
      className={`app-container ${isDragOver ? 'global-drag-active' : ''}`}
      onDragOver={(e) => { e.preventDefault(); setIsDragOver(true); }}
      onDragLeave={(e) => {
        if (!e.currentTarget.contains(e.relatedTarget as Node)) {
          setIsDragOver(false);
        }
      }}
      onDrop={(e) => {
        e.preventDefault();
        setIsDragOver(false);
        const file = e.dataTransfer.files?.[0];
        const textStr = e.dataTransfer.getData("text");
        if (file && file.name.endsWith('.torrent')) {
          setInputMode('torrent');
          const path = (file as any).path || file.name;
          setUrl(path);
          setLogs((l) => [...l.slice(-399), `[SYSTEM] .torrent file loaded: ${path}`]);
        } else if (textStr && textStr.trim().startsWith("magnet:?")) {
          setInputMode('torrent');
          setUrl(textStr.trim());
          setLogs((l) => [...l.slice(-399), `[SYSTEM] Magnet link loaded: ${textStr.substring(0, 40)}...`]);
        }
      }}
    >
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
            {isCrawling ? <VibeLoader size={14} variant="accent" style={{ marginRight: "4px" }} /> : <Activity size={14} />} SYS: {torStatus ? torStatus.state.toUpperCase() : "IDLE"}
          </div>
        </div>
      </header>

      <div className="toolbar" data-testid="toolbar">
        <button className="tool-btn" data-testid="btn-load-target" onClick={() => setUrl("http://worldleaks.onion/api/")}>
          <FolderSearch size={22} className={url.includes("worldleaks") ? "pulse-line text-accent-primary" : ""} /> Load Target
        </button>
        <button className="tool-btn" data-testid="btn-resume" onClick={() => handleCrawl(true)} disabled={isCrawling || vfsStats.totalNodes === 0}>
          <Play size={22} /> Retry / Resume
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
          className="tool-btn"
          data-testid="btn-hex-view"
          onClick={() => setIsOpenHexViewer(true)}
          disabled={!url}
          title="Inspect Raw Extents (Zero-Copy View)"
        >
          <HardDrive size={22} /> Native Hex
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
        <button
          className={`tool-btn ${inputMode === 'onion' ? 'active' : ''}`}
          data-testid="btn-onion"
          onClick={() => setInputMode('onion')}
          title="Switch to Tor / Onion routing mode"
        >
          <ShieldAlert size={22} /> Tor Node
        </button>
        <button
          className={`tool-btn ${inputMode === 'direct' ? 'active' : ''}`}
          data-testid="btn-direct"
          onClick={() => setInputMode('direct')}
          title="Switch to clearnet / direct HTTP(S) mode"
        >
          <Globe size={22} /> Direct
        </button>
        <button
          className={`tool-btn ${inputMode === 'mega' ? 'active' : ''}`}
          data-testid="btn-mega"
          onClick={() => setInputMode('mega')}
          title="Switch to Mega.nz download mode"
        >
          <Cloud size={22} /> Mega.nz
        </button>
        <button
          className={`tool-btn ${inputMode === 'torrent' ? 'active' : ''}`}
          data-testid="btn-torrent"
          onClick={() => setInputMode('torrent')}
          title="Switch to BitTorrent download mode"
        >
          <Magnet size={22} /> Torrent
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
          <span className="input-label">
            {inputMode === "mega" ? "MEGA.NZ" : inputMode === "torrent" ? "TORRENT" : inputMode === "direct" ? "DIRECT URL" : "Target Source"}
          </span>
          <input
            ref={urlInputRef}
            data-testid="input-target-url"
            type="text"
            className="url-input"
            placeholder={inputMode === "mega" ? "https://mega.nz/folder/..." : inputMode === "torrent" ? "magnet:?xt=... or drop .torrent file" : inputMode === "direct" ? "https://example.com/archive.7z" : "http://... (⌘+Enter to start)"}
            value={url}
            onChange={(e) => {
              const val = e.target.value;
              setUrl(val);
              // Phase 52: Auto-detect input mode
              setInputMode(classifyTargetInputMode(val));
            }}
            onKeyDown={(e) => {
              if (e.key === 'Enter') handleCrawl();
            }}
            disabled={isCrawling}
          />
        </div>

        {/* Phase 52E: Mega password input for #P! protected links */}
        {inputMode === "mega" && url.includes("#P!") && (
          <div className="input-group" style={{ marginTop: '6px' }}>
            <span className="input-label" style={{ background: 'var(--accent-warning, #ff9800)', color: '#000' }}>PASSWORD</span>
            <input
              data-testid="input-mega-password"
              type="password"
              className="url-input"
              placeholder="Enter Mega.nz folder password"
              value={megaPassword}
              onChange={(e) => setMegaPassword(e.target.value)}
              disabled={isCrawling}
            />
          </div>
        )}

        <button
          className="action-btn popup-hover"
          data-testid="btn-start-queue"
          onClick={() => handleCrawl(false)}
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

      {/* Phase 52E: Mega/Torrent download progress cards */}
      {megaProgress && (
        <div className="mega-torrent-progress-card" data-testid="mega-progress-card">
          <div className="progress-header">MEGA Download — {megaProgress.index}/{megaProgress.total}</div>
          <div className="progress-file">{megaProgress.file}</div>
          <div className="progress-bar-container">
            <div className="progress-bar" style={{ width: `${(megaProgress.index / Math.max(megaProgress.total, 1)) * 100}%` }} />
          </div>
          <div className="progress-counters">
            ✓ {megaProgress.completed} &nbsp; ✗ {megaProgress.failed} &nbsp; ⤳ {megaProgress.skipped}
          </div>
        </div>
      )}
      {torrentProgress && (
        <div className="mega-torrent-progress-card" data-testid="torrent-progress-card">
          <div className="progress-header">Torrent — {torrentProgress.progress_pct}%</div>
          <div className="progress-file">{torrentProgress.download_speed}</div>
          <div className="progress-bar-container">
            <div className="progress-bar" style={{ width: `${parseFloat(torrentProgress.progress_pct || '0')}%` }} />
          </div>
          <div className="progress-counters">
            {(torrentProgress.downloaded_bytes / 1048576).toFixed(1)} MB / {(torrentProgress.total_bytes / 1048576).toFixed(1)} MB
          </div>
        </div>
      )}

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

      <div className="options-bar" style={{ display: 'flex', flexWrap: 'wrap', gap: '32px', padding: '0 24px 16px', borderBottom: 'var(--panel-border)' }}>
        <label style={{ display: 'flex', alignItems: 'center', gap: '8px', cursor: 'pointer' }}>
          <input
            data-testid="chk-listing"
            type="checkbox"
            checked={crawlOptions.listing}
            onChange={(e) => setCrawlOptions((prev) => ({ ...prev, listing: e.target.checked }))}
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
            onChange={(e) => setCrawlOptions((prev) => ({ ...prev, sizes: e.target.checked }))}
            style={{ accentColor: 'var(--accent-primary)', width: '16px', height: '16px' }}
            disabled={isCrawling}
          />
          <span style={{ fontSize: '0.85rem', color: crawlOptions.sizes ? 'var(--text-main)' : 'var(--text-muted)' }}>Map File Sizes</span>
        </label>

        <label style={{ display: 'flex', alignItems: 'center', gap: '8px', cursor: 'pointer' }} title="Download all discovered files after the crawl finishes.">
          <input
            data-testid="chk-auto-download"
            type="checkbox"
            checked={crawlOptions.download && !crawlOptions.parallelDownload}
            onChange={(e) => setCrawlOptions((prev) => ({
              ...prev,
              download: e.target.checked,
              parallelDownload: false,
            }))}
            style={{ accentColor: 'var(--accent-primary)', width: '16px', height: '16px' }}
            disabled={isCrawling}
          />
          <span style={{ fontSize: '0.85rem', color: (crawlOptions.download && !crawlOptions.parallelDownload) ? 'var(--text-main)' : 'var(--text-muted)' }}>Auto Download</span>
        </label>

        <label style={{ display: 'flex', alignItems: 'center', gap: '8px', cursor: 'pointer' }} title="Download files in real-time while the crawl is still running. Faster than Auto Download.">
          <input
            data-testid="chk-parallel-download"
            type="checkbox"
            checked={crawlOptions.parallelDownload}
            onChange={(e) => setCrawlOptions((prev) => ({
              ...prev,
              download: e.target.checked,
              parallelDownload: e.target.checked,
            }))}
            style={{ accentColor: 'var(--accent-primary)', width: '16px', height: '16px' }}
            disabled={isCrawling}
          />
          <span style={{ fontSize: '0.85rem', color: crawlOptions.parallelDownload ? 'var(--text-main)' : 'var(--text-muted)' }}>⚡ Parallel Download</span>
        </label>



        <label style={{ display: 'flex', alignItems: 'center', gap: '8px', cursor: 'pointer' }} title="Ignore the server domain when caching to dynamically resume aborted downloads even if the host changes.">
          <input
            data-testid="chk-agnostic-state"
            type="checkbox"
            checked={crawlOptions.agnosticState}
            onChange={(e) => setCrawlOptions((prev) => ({ ...prev, agnosticState: e.target.checked }))}
            style={{ accentColor: 'var(--accent-primary)', width: '16px', height: '16px' }}
            disabled={isCrawling}
          />
          <span style={{ fontSize: '0.85rem', color: crawlOptions.agnosticState ? 'var(--text-main)' : 'var(--text-muted)' }}>URI-Agnostic State</span>
        </label>

        <label style={{ display: 'flex', alignItems: 'center', gap: '8px', cursor: 'pointer' }} title="Vanguard Ignition: Dynamically stagger Tor circuit ramp-up to prevent server-side 503 HTTP drops on deepweb domains.">
          <input
            data-testid="chk-stealth-ramp"
            type="checkbox"
            checked={crawlOptions.stealthRamp}
            onChange={(e) => setCrawlOptions((prev) => ({ ...prev, stealthRamp: e.target.checked }))}
            style={{ accentColor: 'var(--accent-primary)', width: '16px', height: '16px' }}
            disabled={isCrawling}
          />
          <span style={{ fontSize: '0.85rem', color: crawlOptions.stealthRamp ? 'var(--text-main)' : 'var(--text-muted)' }}>Vanguard Stealth Ramp</span>
        </label>

        <label style={{ display: 'flex', alignItems: 'center', gap: '8px', cursor: 'pointer' }} title="Force routing through standard Clearnet HTTP interface instead of Tor (Requires Direct URL or compatible domain)">
          <input
            data-testid="chk-force-clearnet"
            type="checkbox"
            checked={crawlOptions.forceClearnet}
            onChange={(e) => setCrawlOptions((prev) => ({ ...prev, forceClearnet: e.target.checked }))}
            style={{ accentColor: 'var(--accent-primary)', width: '16px', height: '16px' }}
            disabled={isCrawling}
          />
          <span style={{ fontSize: '0.85rem', color: crawlOptions.forceClearnet ? 'var(--text-main)' : 'var(--text-muted)' }}>Force Clearnet Route</span>
        </label>

        <button
          className="action-btn popup-hover"
          data-testid="btn-resume-index"
          onClick={async () => {
            if (crawlOptions.resumeIndex) {
              setCrawlOptions(prev => ({ ...prev, resumeIndex: undefined, resume: false }));
            } else {
              const selected = await openDialog({
                multiple: false,
                filters: [{ name: 'Onion Forge Index', extensions: ['txt'] }]
              });
              if (selected && typeof selected === 'string') {
                setCrawlOptions(prev => ({ ...prev, resumeIndex: selected, resume: true }));
              }
            }
          }}
          disabled={isCrawling}
          style={{
            width: 'auto',
            padding: '2px 10px',
            background: 'transparent',
            border: `1px solid ${crawlOptions.resumeIndex ? 'var(--accent-primary)' : 'var(--border-color)'}`,
            fontSize: '0.85rem',
            cursor: isCrawling ? 'not-allowed' : 'pointer',
            color: crawlOptions.resumeIndex ? 'var(--accent-primary)' : 'var(--text-main)',
            borderRadius: '4px'
          }}
        >
          {crawlOptions.resumeIndex ? `Advanced Override: ${crawlOptions.resumeIndex.split(/[\\/]/).pop()}` : "Advanced Baseline Override"}
        </button>

        <div style={{ display: 'flex', alignItems: 'center', gap: '6px', marginLeft: 'auto' }} data-testid="preset-selector">
          <span style={{ fontSize: '0.78rem', color: 'var(--text-muted)', textTransform: 'uppercase', letterSpacing: '0.04em', marginRight: '2px' }}>MODE:</span>
          {([
            { key: 'default', label: 'Default', emoji: '⚡', downloadMode: 'default' as const, desc: '2 Guard Nodes', tip: 'Proven 1.83 MB/s peak — recommended for most systems' },
            { key: 'high', label: 'High', emoji: '🔥', downloadMode: 'high' as const, desc: '3 Guard Nodes', tip: '+50% bandwidth — 3 independent Tor guard relays (needs ≥8GB RAM)' },
            { key: 'aggressive', label: 'Aggressive', emoji: '🚀', downloadMode: 'aggressive' as const, desc: '4 Guard Nodes', tip: '+100% bandwidth — 4 independent Tor guard relays (needs ≥16GB RAM)' },
          ] as const).map((preset) => {
            const isSelected = crawlOptions.downloadMode === preset.downloadMode;
            const isAutoDetected = systemProfile && (
              (preset.key === 'default' && (!systemProfile || systemProfile.totalRamGb <= 8.5)) ||
              (preset.key === 'high' && (systemProfile.totalRamGb > 8.5 && systemProfile.totalRamGb <= 20)) ||
              (preset.key === 'aggressive' && systemProfile.totalRamGb > 20)
            );
            return (
              <button
                key={preset.key}
                data-testid={`preset-${preset.key}`}
                title={preset.tip}
                disabled={isCrawling}
                onClick={() => setCrawlOptions(prev => ({ ...prev, downloadMode: preset.downloadMode }))}
                style={{
                  display: 'flex',
                  flexDirection: 'column',
                  alignItems: 'center',
                  gap: '1px',
                  padding: '5px 14px',
                  background: isSelected
                    ? 'linear-gradient(135deg, rgba(139, 92, 246, 0.2), rgba(99, 102, 241, 0.15))'
                    : 'rgba(255, 255, 255, 0.03)',
                  border: isSelected
                    ? '1px solid rgba(139, 92, 246, 0.5)'
                    : '1px solid var(--border-color)',
                  borderRadius: '8px',
                  cursor: isCrawling ? 'not-allowed' : 'pointer',
                  transition: 'all 0.2s ease',
                  minWidth: '105px',
                  opacity: isCrawling ? 0.5 : 1,
                }}
              >
                <span style={{ fontSize: '0.9rem' }}>{preset.emoji}</span>
                <span style={{
                  fontSize: '0.72rem',
                  fontWeight: 700,
                  color: isSelected ? 'var(--accent-primary)' : 'var(--text-main)',
                  letterSpacing: '0.06em',
                }}>
                  {preset.label}{isAutoDetected ? ' ★' : ''}
                </span>
                <span style={{
                  fontSize: '0.6rem',
                  color: 'var(--text-muted)',
                  fontFamily: 'JetBrains Mono',
                  whiteSpace: 'nowrap',
                }}>
                  {preset.desc}
                </span>
              </button>
            );
          })}
          {systemProfile && (
            <span
              style={{ fontSize: '0.68rem', color: 'var(--accent-secondary)', opacity: 0.6, marginLeft: '4px' }}
              title={`Detected: ${systemProfile.cpuCores} cores, ${systemProfile.totalRamGb}GB RAM, ${systemProfile.storageClass.toUpperCase()}, ${systemProfile.os}`}
            >
              {systemProfile.os} · {systemProfile.cpuCores}c · {systemProfile.totalRamGb}GB
            </span>
          )}
        </div>

      </div>

      {!isTauriRuntime ? (
        <>
          <div className="ops-dashboard">
            <div className="dash-card">
              <div className="dash-icon"><Database size={24} /></div>
              <div className="dash-info">
                <div className="dash-title">NODES INDEXED</div>
                <div className="dash-value" style={{ fontFamily: 'JetBrains Mono' }}>{vfsStats.totalNodes.toLocaleString()}</div>
                <div className="dash-sub">Preview fixture state</div>
              </div>
            </div>
            <div className="dash-card resource-card" data-testid="resource-metrics-card">
              <div className="dash-icon"><Cpu size={24} /></div>
              <div className="dash-info">
                <div className="dash-title">PROCESS + SYSTEM</div>
                <div className="dash-value" data-testid="resource-process-cpu">
                  CPU {resourceMetrics.processCpuPercent.toFixed(1)}%
                </div>
                <div className="dash-sub" data-testid="resource-process-memory" style={{ fontFamily: 'JetBrains Mono' }}>
                  RSS {(resourceMetrics.processMemoryBytes / 1048576).toFixed(1)} MB | Threads {resourceMetrics.processThreads}
                </div>
                <div className="dash-sub" data-testid="resource-worker-metrics" style={{ fontFamily: 'JetBrains Mono' }}>
                  {crawlStatus.vanguard ? (
                    <span style={{ color: 'var(--accent-primary)' }}>Vanguard: {crawlStatus.vanguard.status} | Circuits {resourceMetrics.activeCircuits}/{resourceMetrics.peakActiveCircuits}</span>
                  ) : (
                    <span>Workers {resourceMetrics.activeWorkers}/{resourceMetrics.workerTarget} | Circuits {resourceMetrics.activeCircuits}/{resourceMetrics.peakActiveCircuits}</span>
                  )}
                </div>
                <div className="dash-sub" data-testid="resource-node-metrics" style={{ fontFamily: 'JetBrains Mono' }}>
                  Node {resourceMetrics.currentNodeHost || "unresolved"} | Multi-Client Rotations {resourceMetrics.multiClientRotations || 0} (Pool: {resourceMetrics.multiClientCount || 0}) | 429/503 {resourceMetrics.throttleCount} | Timeouts {resourceMetrics.timeoutCount}
                </div>
              </div>
            </div>
          </div>

          <div className="main-workspace">
            <div className="panel" style={{ flex: 1 }}>
              <div className="panel-header">
                <span style={{ display: 'flex', alignItems: 'center', gap: '6px' }}>
                  <Terminal size={14} /> Forensic Log
                </span>
              </div>
              <div className="panel-content">
                <div className="forensic-log">
                  {previewLogs.map((log, i) => (
                    <div key={i} className="term-line">
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
                <span style={{ fontSize: "0.8rem", color: "var(--accent-secondary)", background: "rgba(0, 229, 255, 0.1)", padding: "2px 8px", borderRadius: "12px", border: "1px solid rgba(0, 229, 255, 0.3)" }}>
                  {vfsStats.totalNodes.toLocaleString()} Nodes
                </span>
              </div>
              <div className="panel-content" style={{ padding: 0 }}>
                <div className="vfs-container" style={{ height: '100%', overflow: 'auto' }}>
                  {VFS_FIXTURE_STATS.totalNodes === 0 ? (
                    <div style={{ height: '100%', display: 'flex', alignItems: 'center', justifyContent: 'center', color: 'var(--text-muted)' }}>
                      No files detected in virtual file system.
                    </div>
                  ) : (
                    VFS_FIXTURE_ENTRIES.map((entry) => (
                      <div
                        key={entry.path}
                        className="vfs-row"
                        data-testid={`vfs-row-${encodeURIComponent(entry.path)}`}
                        style={{ minHeight: '36px', paddingLeft: `${entry.path.split('/').length * 18}px` }}
                      >
                        <div className="vfs-icon" style={{ marginLeft: '12px' }}>
                          <ListTree size={14} color={entry.entry_type === "Folder" ? "var(--accent-primary)" : "var(--text-muted)"} />
                        </div>
                        <span className="vfs-name">{entry.path.split('/').pop() || entry.path}</span>
                        <div style={{ flex: 1, display: 'flex', justifyContent: 'flex-end', paddingRight: '12px' }}>
                          <span className="vfs-size">
                            {entry.size_bytes == null ? '--' : entry.size_bytes.toLocaleString()}
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
            {Array.from({ length: Math.max(activeDaemons, 1) }).map((_, idx) => (
              <div key={idx} className="daemon-box">
                <div className="daemon-icon">
                  <Zap size={18} />
                </div>
                <div className="daemon-info">
                  <div className="daemon-header">ARTI NODE {idx + 1}</div>
                  <div className="daemon-body">
                    <span style={{ color: 'var(--text-muted)' }}>STANDBY</span>
                    <span style={{ fontSize: '0.8rem', color: 'var(--text-muted)', fontFamily: 'JetBrains Mono' }}>---</span>
                  </div>
                </div>
              </div>
            ))}
          </div>
        </>
      ) : (
        <>
          <Dashboard
            isCrawling={isCrawling}
            torStatus={torStatus}
            activeAdapter={activeAdapter}
            crawlStatus={crawlStatus}
            downloadBatchStatus={downloadBatchStatus}
            logs={logs}
            vfsCount={vfsStats.totalNodes}
            vfsRefreshTrigger={vfsRefreshTrigger}
            downloadProgress={downloadProgress}
            elapsed={crawlElapsed}
            downloadElapsed={downloadElapsed}
            resourceMetrics={resourceMetrics}
            efficiencyHistory={efficiencyHistory}
            crawlRunStatus={lastCrawlResult}
            downloadResumePlan={downloadResumePlan}
            ceilingStatus={ceilingStatus}
            onAzureClick={() => setShowAzureModal(true)}
          />

          <PatientRetryPanel retryState={patientRetryState} />

          <div className="main-workspace">
            <div className="panel" style={{ flex: 1 }}>
              <div className="panel-header">
                <span style={{ display: 'flex', alignItems: 'center', gap: '6px' }}>
                  <Terminal size={14} /> Forensic Log
                </span>
              </div>
              <div className="panel-content">
                <div style={{
                  display: 'grid',
                  gap: '6px',
                  maxHeight: '180px',
                  overflow: 'auto',
                  padding: '8px 10px',
                  borderBottom: '1px solid rgba(255,255,255,0.06)',
                  background: 'rgba(255,255,255,0.02)'
                }}>
                  <div style={{ fontSize: '0.72rem', color: 'var(--text-muted)', textTransform: 'uppercase', letterSpacing: '0.05em' }}>
                    Unique Message Summary
                  </div>
                  {aggregateRows.length === 0 ? (
                    <div style={{ fontSize: '0.78rem', color: 'var(--text-muted)' }}>
                      Waiting for crawl and downloader logs...
                    </div>
                  ) : aggregateRows.map((aggregate) => (
                    <div
                      key={aggregate.key}
                      style={{
                        display: 'grid',
                        gridTemplateColumns: '56px 1fr',
                        gap: '8px',
                        padding: '6px 8px',
                        border: '1px solid rgba(255,255,255,0.06)',
                        borderRadius: '6px',
                        background: 'rgba(10, 14, 20, 0.55)'
                      }}
                    >
                      <div style={{ fontFamily: 'JetBrains Mono', color: 'var(--accent-primary)', fontSize: '0.78rem' }}>
                        ×{aggregate.count}
                      </div>
                      <div style={{ minWidth: 0 }}>
                        <div style={{ fontSize: '0.8rem', color: 'var(--text-main)', wordBreak: 'break-word' }}>
                          {aggregate.sample}
                        </div>
                        <div style={{ fontSize: '0.72rem', color: 'var(--text-muted)', fontFamily: 'JetBrains Mono' }}>
                          First {new Date(aggregate.firstSeenAt).toLocaleTimeString()} | Last {new Date(aggregate.lastSeenAt).toLocaleTimeString()}
                        </div>
                      </div>
                    </div>
                  ))}
                </div>
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
            {activeDaemons > 6 ? (
              <div className="daemon-box" style={{ flex: 1, justifyContent: "center" }}>
                <div className="daemon-icon">
                  {isCrawling ? <VibeLoader size={18} variant="secondary" /> : <Zap size={18} />}
                </div>
                <div className="daemon-info" style={{ flex: "none" }}>
                  <div className="daemon-header">ARTI SWARM ({activeDaemons} NODES)</div>
                  <div className="daemon-body">
                    <span style={{ fontSize: '0.85rem', color: 'var(--accent-secondary)', fontFamily: 'JetBrains Mono', wordBreak: 'break-all' }}>
                      MODE: NATIVE MEMORY
                    </span>
                  </div>
                </div>
              </div>
            ) : (
              Array.from({ length: activeDaemons }).map((_, idx) => (
                <div key={idx} className="daemon-box">
                  <div className="daemon-icon">
                    {isCrawling ? <VibeLoader size={18} variant="secondary" /> : <Zap size={18} />}
                  </div>
                  <div className="daemon-info">
                    <div className="daemon-header">ARTI NODE {idx + 1}</div>
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
              ))
            )}
          </div >
        </>
      )}

      {/* Phase 53: Azure Connectivity Modal */}
      <AzureConnectivityModal isOpen={showAzureModal} onClose={() => setShowAzureModal(false)} />

      {/* Phase 82: Native Hex Viewer */}
      <HexViewer url={url} isOpen={isOpenHexViewer} onClose={() => setIsOpenHexViewer(false)} />
    </div >
  );
}

export default App;
