import { Network, Cpu, Database, CloudDownload, TerminalSquare, Cloud } from "lucide-react";
import "./Dashboard.css";
import { VibeLoader } from "./VibeLoader";
import { VfsTreeView } from "./VfsTreeView";

interface DashboardProps {
  isCrawling: boolean;
  torStatus: any;
  activeAdapter: string;
  crawlStatus: {
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
  };
  downloadBatchStatus: {
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
  };
  logs: string[];
  vfsCount: number;
  vfsRefreshTrigger: number;
  downloadProgress: Record<string, any>;
  elapsed: number;
  downloadElapsed: number;
  resourceMetrics: {
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
    nodeFailovers: number;
    throttleCount: number;
    timeoutCount: number;
    uptimeSeconds: number;
    consensusWeight: number;
    multiClientRotations?: number;
    multiClientCount?: number;
  };
  crawlRunStatus: {
    targetKey: string;
    bestPriorCount: number;
    rawThisRunCount: number;
    mergedEffectiveCount: number;
    crawlOutcome: string;
    retryCountUsed: number;
    stableCurrentListingPath: string;
    stableBestListingPath: string;
  } | null;
  downloadResumePlan: {
    failedFirstCount: number;
    missingOrMismatchCount: number;
    skippedExactMatchesCount: number;
    allItemsSkipped: boolean;
    plannedFileCount: number;
    failureManifestPath: string;
  } | null;
  // Phase 74B: Adaptive ceiling status
  ceilingStatus: {
    value: number;
    direction: 'DECAY' | 'RECOVERY' | null;
    lastChange: number | null;
  };
  onAzureClick?: () => void;
}

export function Dashboard({
  isCrawling,
  torStatus,
  activeAdapter,
  crawlStatus,
  downloadBatchStatus,
  logs,
  vfsCount,
  vfsRefreshTrigger,
  downloadProgress,
  elapsed,
  downloadElapsed,
  resourceMetrics,
  crawlRunStatus,
  downloadResumePlan,
  ceilingStatus = { value: 0, direction: null, lastChange: null },
  onAzureClick,
}: DashboardProps) {
  let phase = "IDLE";
  let networkStatus = "Standby";

  const dedupedProgress = new Map<string, any>();
  Object.entries(downloadProgress).forEach(([key, value]) => {
    const dedupeKey = value?.path || key;
    dedupedProgress.set(dedupeKey, value);
  });
  const progressValues = Array.from(dedupedProgress.values());

  const progressDownloadedBytes = progressValues.reduce((acc: number, p: any) => acc + (p.bytes_downloaded || 0), 0);
  const progressSpeedBps = progressValues.reduce((acc: number, p: any) => acc + (p.speed_bps || 0), 0);

  const batchTotal = Math.max(downloadBatchStatus.totalFiles || 0, 0);
  const batchDone = Math.max(downloadBatchStatus.completedFiles || 0, 0);
  const batchFailed = Math.max(downloadBatchStatus.failedFiles || 0, 0);
  const batchProcessed = batchDone + batchFailed;
  const batchRemaining = Math.max(batchTotal - batchProcessed, 0);
  const hasBatch = batchTotal > 0;
  const showDownloadProgress = hasBatch && (isCrawling || batchProcessed > 0 || !!downloadBatchStatus.currentFile);
  const filePercent = hasBatch ? Math.min(100, (batchProcessed / Math.max(batchTotal, 1)) * 100) : 0;
  const downloadEtaLabel =
    batchRemaining > 0 && downloadBatchStatus.etaSeconds && downloadBatchStatus.etaSeconds > 0
      ? `ETA ${downloadBatchStatus.etaSeconds}s`
      : (batchRemaining === 0 && hasBatch ? "Complete" : "Estimating");
  const hintedGb = (downloadBatchStatus.totalBytesHint / 1073741824).toFixed(2);
  const downloadElapsedSec = Math.max(0, Math.floor(downloadElapsed / 1000));
  const batchHintedDownloadedBytes = hasBatch && downloadBatchStatus.totalBytesHint > 0
    ? Math.floor(downloadBatchStatus.totalBytesHint * (batchProcessed / Math.max(batchTotal, 1)))
    : 0;
  const resolvedDownloadedBytes = Math.max(
    progressDownloadedBytes,
    downloadBatchStatus.downloadedBytes || 0,
    batchHintedDownloadedBytes,
  );
  const throughputFromProgress = progressSpeedBps / 1048576;
  const throughputFromBatch = downloadBatchStatus.speedMbps || 0;
  const throughputFromAverage =
    downloadElapsedSec > 0 ? resolvedDownloadedBytes / downloadElapsedSec / 1048576 : 0;
  const resolvedSpeedMbps =
    throughputFromProgress > 0
      ? throughputFromProgress
      : (throughputFromBatch > 0 ? throughputFromBatch : throughputFromAverage);
  const bytePercent =
    hasBatch && downloadBatchStatus.totalBytesHint > 0
      ? Math.min(100, (resolvedDownloadedBytes / downloadBatchStatus.totalBytesHint) * 100)
      : 0;
  const downloadPercent = hasBatch ? Math.max(filePercent, bytePercent) : 0;
  const speedMb = resolvedSpeedMbps.toFixed(2);
  const smoothedSpeedMb = Math.max(
    0,
    downloadBatchStatus.smoothedSpeedMbps || resolvedSpeedMbps || 0,
  ).toFixed(2);
  const downloadedMb = (resolvedDownloadedBytes / 1048576).toFixed(2);
  const diskWriteMbps = Math.max(0, downloadBatchStatus.diskWriteMbps || 0);
  const activeCircuits = Math.max(0, downloadBatchStatus.activeCircuits || 0);
  const etaConfidencePct = Math.round(Math.max(0, Math.min(1, downloadBatchStatus.etaConfidence || 0)) * 100);
  const processMemoryMb = (resourceMetrics.processMemoryBytes / 1048576).toFixed(1);
  const systemMemoryGbUsed = (resourceMetrics.systemMemoryUsedBytes / 1073741824).toFixed(1);
  const systemMemoryGbTotal = (resourceMetrics.systemMemoryTotalBytes / 1073741824).toFixed(1);
  const effectiveActiveWorkers =
    resourceMetrics.workerTarget > 0 ? resourceMetrics.activeWorkers : crawlStatus.activeWorkers;
  const effectiveWorkerTarget =
    resourceMetrics.workerTarget > 0 ? resourceMetrics.workerTarget : Math.max(crawlStatus.workerTarget, 1);
  const currentListingName = crawlRunStatus?.stableCurrentListingPath?.split(/[\\/]/).pop() || "";
  const bestListingName = crawlRunStatus?.stableBestListingPath?.split(/[\\/]/).pop() || "";

  if (isCrawling) {
    phase = "PROBING TARGET";
    if (torStatus?.state === "starting" || torStatus?.state === "bootstrapping" || torStatus?.state === "consensus") {
      phase = "BOOTSTRAPPING TOR NODE";
      networkStatus = "Handshake in progress...";
    } else if (torStatus?.state === "ready") {
      networkStatus = "Encrypted Swarm (Active)";
    }

    if (activeAdapter && activeAdapter !== "Unidentified") {
      phase = "SCANNING / FILE LISTING";
    }

    const dlLog = [...logs].reverse().find(l => l.includes("Auto-Mirror engaged") || l.includes("Manual Mirror"));
    const finishedLog = [...logs].reverse().find(l => l.includes("Finish signaled") || l.includes("All nodes processed"));

    if (showDownloadProgress || (dlLog && (!finishedLog || logs.indexOf(dlLog) > logs.indexOf(finishedLog)))) {
      phase = "SCAFFOLDING (DOWNLOADING)";
    } else if (finishedLog && !dlLog) {
      phase = "COMPLETE";
      networkStatus = "Cooldown";
    }
  }
  const crawlPercent = Math.max(0, Math.min(100, crawlStatus.progressPercent || 0));
  const crawlPhase = (crawlStatus.phase || "idle").replace(/_/g, " ").toUpperCase();
  const etaLabel = crawlStatus.etaSeconds && crawlStatus.etaSeconds > 0
    ? `ETA ${crawlStatus.etaSeconds}s`
    : "Estimating";

  return (
    <div className="ops-dashboard">
      <div className="dash-card">
        <div className="dash-icon"><Cpu size={24} /></div>
        <div className="dash-info">
          <div className="dash-title">OPERATION PHASE</div>
          <div className="dash-value" style={{ color: isCrawling ? 'var(--accent-primary)' : 'var(--text-muted)' }}>
            <span style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
              {phase}
              {isCrawling && <VibeLoader size={14} variant="accent" />}
            </span>
          </div>
        </div>
      </div>

      <div className="dash-card">
        <div className="dash-icon"><Network size={24} /></div>
        <div className="dash-info">
          <div className="dash-title">TOR SWARM</div>
          <div className="dash-value">{networkStatus}</div>
          {elapsed > 0 && <div className="dash-sub">Elapsed: {Math.floor(elapsed / 1000)}s</div>}
        </div>
      </div>

      <div className="dash-card">
        <div className="dash-icon"><TerminalSquare size={24} /></div>
        <div className="dash-info">
          <div className="dash-title">ACTIVE ADAPTER</div>
          <div className="dash-value">{activeAdapter || "Unidentified"}</div>
        </div>
      </div>

      <div className="dash-card">
        <div className="dash-icon"><Database size={24} /></div>
        <div className="dash-info">
          <div className="dash-title">NODES INDEXED</div>
          <div className="dash-value" style={{ fontFamily: 'JetBrains Mono' }}>{vfsCount.toLocaleString()}</div>
        </div>
      </div>

      <div className="dash-card highlight-card">
        <div className="dash-icon"><CloudDownload size={24} /></div>
        <div className="dash-info">
          <div className="dash-title">NETWORK I/O (BBR)</div>
          <div className="dash-value">{speedMb} MB/s</div>
          <div className="dash-sub" style={{ fontFamily: 'JetBrains Mono' }}>Peak BW: {downloadBatchStatus.peakBandwidthMbps?.toFixed(2) || "0.00"} MB/s</div>
          <div className="dash-sub" style={{ fontFamily: 'JetBrains Mono' }}>Disk I/O: {diskWriteMbps.toFixed(2)} MB/s (Peak {downloadBatchStatus.peakDiskWriteMbps?.toFixed(2) || "0.00"})</div>
          <div className="dash-sub" style={{ fontFamily: 'JetBrains Mono' }}>Active Circuits: {activeCircuits} (Peak {downloadBatchStatus.peakActiveCircuits || 0})</div>
          <div className="dash-sub" style={{ fontFamily: 'JetBrains Mono' }}>BBR Bottleneck: {downloadBatchStatus.bbrBottleneckMbps?.toFixed(2) || "0.00"} MB/s</div>
          <div className="dash-sub" style={{ fontFamily: 'JetBrains Mono' }}>EKF Var/Cov: {downloadBatchStatus.ekfCovariance?.toFixed(3) || "0.000"} P</div>
          <div className="dash-sub" style={{ fontFamily: 'JetBrains Mono' }}>Total Payload: {downloadedMb} MB</div>
          <div className="dash-sub" style={{ fontFamily: 'JetBrains Mono' }}>Tor Uptime: {Math.floor((resourceMetrics.uptimeSeconds || 0) / 3600)}h {Math.floor(((resourceMetrics.uptimeSeconds || 0) % 3600) / 60)}m {(resourceMetrics.uptimeSeconds || 0) % 60}s</div>
          <div className="dash-sub" style={{ fontFamily: 'JetBrains Mono' }}>Consensus Wgt: {resourceMetrics.consensusWeight || 0} CW</div>
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
            RSS {processMemoryMb} MB | Threads {resourceMetrics.processThreads}
          </div>
          <div className="dash-sub" data-testid="resource-system-memory" style={{ fontFamily: 'JetBrains Mono' }}>
            RAM {resourceMetrics.systemMemoryPercent.toFixed(1)}% ({systemMemoryGbUsed}/{systemMemoryGbTotal} GB)
          </div>
          <div className="dash-sub" data-testid="resource-worker-metrics" style={{ fontFamily: 'JetBrains Mono' }}>
            {crawlStatus.vanguard ? (
              <span style={{ color: 'var(--accent-primary)' }}>Vanguard: {crawlStatus.vanguard.status} | Circuits {resourceMetrics.activeCircuits}/{resourceMetrics.peakActiveCircuits}</span>
            ) : (
              <span>Workers {effectiveActiveWorkers}/{effectiveWorkerTarget} | Circuits {resourceMetrics.activeCircuits}/{resourceMetrics.peakActiveCircuits}</span>
            )}
          </div>
          <div className="dash-sub" data-testid="resource-node-metrics" style={{ fontFamily: 'JetBrains Mono' }}>
            Node {resourceMetrics.currentNodeHost || "unresolved"} | Multi-Client Rotations {resourceMetrics.multiClientRotations || 0} (Pool: {resourceMetrics.multiClientCount || 0}) | 429/503 {resourceMetrics.throttleCount} | Timeouts {resourceMetrics.timeoutCount}
          </div>
          {ceilingStatus?.value > 0 && (
            <div className="dash-sub" data-testid="resource-ceiling-status" style={{ fontFamily: 'JetBrains Mono' }}>
              <span style={{
                color: ceilingStatus.direction === 'DECAY' ? '#ef4444'
                  : ceilingStatus.direction === 'RECOVERY' ? '#10b981'
                    : 'var(--text-muted)'
              }}>
                {ceilingStatus.direction === 'DECAY' ? '▼' : ceilingStatus.direction === 'RECOVERY' ? '▲' : '●'}
              </span>
              {' '}Adaptive Ceiling: {ceilingStatus.value}
              {ceilingStatus.direction && ` (${ceilingStatus.direction})`}
              {ceilingStatus.lastChange && ` [${Math.floor((Date.now() - ceilingStatus.lastChange) / 1000)}s ago]`}
            </div>
          )}
        </div>
      </div>

      {/* Phase 53: Azure Connectivity */}
      {onAzureClick && (
        <div className="dash-card" style={{ cursor: 'pointer' }} onClick={onAzureClick} data-testid="azure-connectivity-btn">
          <div className="dash-icon"><Cloud size={24} /></div>
          <div className="dash-info">
            <div className="dash-title">AZURE CONNECTIVITY</div>
            <div className="dash-value" style={{ fontSize: '0.85rem' }}>Enterprise Cloud + Intranet</div>
            <div className="dash-sub">Click to configure Azure Storage or Intranet access</div>
          </div>
        </div>
      )}

      <div className="dash-card resource-card" data-testid="crawl-baseline-card">
        <div className="dash-icon"><Database size={24} /></div>
        <div className="dash-info">
          <div className="dash-title">TARGET BASELINE</div>
          <div className="dash-value" data-testid="crawl-baseline-outcome">
            {crawlRunStatus ? crawlRunStatus.crawlOutcome.replace(/_/g, ' ').toUpperCase() : "NO BASELINE YET"}
          </div>
          <div className="dash-sub" style={{ fontFamily: 'JetBrains Mono' }}>
            {crawlRunStatus
              ? `${crawlRunStatus.targetKey} | raw ${crawlRunStatus.rawThisRunCount} | best ${crawlRunStatus.bestPriorCount} | merged ${crawlRunStatus.mergedEffectiveCount}`
              : "Run a crawl to initialize per-target best/current listings."}
          </div>
          <div className="dash-sub" style={{ fontFamily: 'JetBrains Mono' }}>
            {crawlRunStatus
              ? `Retries used ${crawlRunStatus.retryCountUsed}/2 | current ${currentListingName} | best ${bestListingName}`
              : "Stable listing files will be shown here after the first run."}
          </div>
        </div>
      </div>

      <div className="dash-card progress-card">
        <div className="dash-info" style={{ width: "100%" }}>
          <div className="dash-title">{showDownloadProgress ? "DOWNLOAD PROGRESS" : "CRAWL PROGRESS"}</div>
          <div className="progress-header">
            <div className="dash-value">{(showDownloadProgress ? downloadPercent : crawlPercent).toFixed(1)}%</div>
            <div className="dash-sub">
              {showDownloadProgress
                ? `DOWNLOADING • ${downloadEtaLabel}`
                : `${crawlPhase} • ${etaLabel}`}
            </div>
          </div>
          <div className="crawl-progress-track">
            <div
              className="crawl-progress-fill"
              style={{ width: `${showDownloadProgress ? downloadPercent : crawlPercent}%` }}
            />
          </div>
          {showDownloadProgress ? (
            <>
              <div className="dash-sub" style={{ fontFamily: "JetBrains Mono" }}>
                Total: {batchTotal.toLocaleString()} | Downloaded: {batchDone.toLocaleString()} | Failed: {batchFailed.toLocaleString()} | Remaining: {batchRemaining.toLocaleString()}
              </div>
              <div className="dash-sub" style={{ fontFamily: "JetBrains Mono" }}>
                Elapsed: {downloadElapsedSec}s | Throughput: {speedMb} MB/s (EWMA {smoothedSpeedMb}) | ETA Confidence: {etaConfidencePct}% | Disk I/O: {diskWriteMbps.toFixed(2)} MB/s | Hint Size: {hintedGb} GB | Unknown Sizes: {downloadBatchStatus.unknownSizeFiles.toLocaleString()}
              </div>
              {downloadResumePlan ? (
                <div className="dash-sub" style={{ fontFamily: "JetBrains Mono" }}>
                  Failures first: {downloadResumePlan.failedFirstCount} | Missing/Mismatch: {downloadResumePlan.missingOrMismatchCount} | Skipped exact: {downloadResumePlan.skippedExactMatchesCount} | {downloadResumePlan.allItemsSkipped ? "All items skipped" : `Planned files: ${downloadResumePlan.plannedFileCount}`}
                </div>
              ) : null}
              {downloadBatchStatus.currentFile ? (
                <div className="dash-sub" style={{ fontFamily: "JetBrains Mono" }}>
                  Current: {downloadBatchStatus.currentFile}
                </div>
              ) : null}
            </>
          ) : (
            <div className="dash-sub" style={{ fontFamily: "JetBrains Mono" }}>
              Seen: {crawlStatus.visitedNodes.toLocaleString()} | Processed: {crawlStatus.processedNodes.toLocaleString()} | Queue: {crawlStatus.queuedNodes.toLocaleString()} | Workers: {effectiveActiveWorkers}/{effectiveWorkerTarget}
              {crawlStatus.deltaNewFiles !== undefined ? ` | Delta New: ${crawlStatus.deltaNewFiles.toLocaleString()}` : ""}
            </div>
          )}
        </div>
      </div>

      <div className="dash-card vfs-tree-card" style={{ gridColumn: "1 / -1", height: "400px", padding: 0, overflow: "hidden", display: "flex", flexDirection: "column" }}>
        <div className="dash-info" style={{ width: "100%", padding: "16px", borderBottom: "1px solid var(--border-color)", flexShrink: 0 }}>
          <div className="dash-title">VFS TARGET TREE</div>
        </div>
        <div style={{ flex: 1, overflow: "hidden" }}>
          <VfsTreeView
            triggerRefresh={vfsRefreshTrigger}
            targetKey={crawlRunStatus?.targetKey || null}
            stableCurrentListingPath={crawlRunStatus?.stableCurrentListingPath || null}
            outputDir={downloadBatchStatus.outputDir}
          />
        </div>
      </div>
    </div>
  );
}
