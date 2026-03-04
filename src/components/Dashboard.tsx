import { Network, Cpu, Database, CloudDownload, TerminalSquare } from "lucide-react";
import "./Dashboard.css";
import { VibeLoader } from "./VibeLoader";

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
  };
  downloadBatchStatus: {
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
  };
  logs: string[];
  vfsCount: number;
  downloadProgress: Record<string, any>;
  elapsed: number;
  downloadElapsed: number;
}

export function Dashboard({
  isCrawling,
  torStatus,
  activeAdapter,
  crawlStatus,
  downloadBatchStatus,
  logs,
  vfsCount,
  downloadProgress,
  elapsed,
  downloadElapsed,
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
  const downloadPercent = hasBatch ? Math.min(100, (batchProcessed / Math.max(batchTotal, 1)) * 100) : 0;
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
  const speedMb = resolvedSpeedMbps.toFixed(2);
  const downloadedMb = (resolvedDownloadedBytes / 1048576).toFixed(2);

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
          <div className="dash-sub" style={{ fontFamily: 'JetBrains Mono' }}>BBR Bottleneck: {downloadBatchStatus.bbrBottleneckMbps?.toFixed(2) || "0.00"} MB/s</div>
          <div className="dash-sub" style={{ fontFamily: 'JetBrains Mono' }}>EKF Var/Cov: {downloadBatchStatus.ekfCovariance?.toFixed(3) || "0.000"} P</div>
          <div className="dash-sub" style={{ fontFamily: 'JetBrains Mono' }}>Total Payload: {downloadedMb} MB</div>
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
                Elapsed: {downloadElapsedSec}s | Throughput: {speedMb} MB/s | Hint Size: {hintedGb} GB | Unknown Sizes: {downloadBatchStatus.unknownSizeFiles.toLocaleString()}
              </div>
              {downloadBatchStatus.currentFile ? (
                <div className="dash-sub" style={{ fontFamily: "JetBrains Mono" }}>
                  Current: {downloadBatchStatus.currentFile}
                </div>
              ) : null}
            </>
          ) : (
            <div className="dash-sub" style={{ fontFamily: "JetBrains Mono" }}>
              Seen: {crawlStatus.visitedNodes.toLocaleString()} | Processed: {crawlStatus.processedNodes.toLocaleString()} | Queue: {crawlStatus.queuedNodes.toLocaleString()} | Workers: {crawlStatus.activeWorkers}/{Math.max(crawlStatus.workerTarget, 1)}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
