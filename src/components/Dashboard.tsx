import { Network, Cpu, Database, CloudDownload, TerminalSquare, RefreshCw } from "lucide-react";
import "./Dashboard.css";

interface DashboardProps {
  isCrawling: boolean;
  torStatus: any;
  logs: string[];
  vfsCount: number;
  downloadProgress: Record<string, any>;
  elapsed: number;
}

export function Dashboard({ isCrawling, torStatus, logs, vfsCount, downloadProgress, elapsed }: DashboardProps) {
  let phase = "IDLE";
  let activeModule = "Unidentified";
  let networkStatus = "Standby";

  if (isCrawling) {
    phase = "PROBING TARGET";
    if (torStatus?.state === "starting" || torStatus?.state === "bootstrapping" || torStatus?.state === "consensus") {
      phase = "BOOTSTRAPPING TOR NODE";
      networkStatus = "Handshake in progress...";
    } else if (torStatus?.state === "ready") {
      networkStatus = "Encrypted Swarm (Active)";
    }

    const adapterLog = [...logs].reverse().find(l => l.includes("Match found:"));
    if (adapterLog) {
      activeModule = adapterLog.split("Match found:")[1]?.trim() || activeModule;
      phase = "SCANNING / FILE LISTING";
    }

    const dlLog = [...logs].reverse().find(l => l.includes("Auto-Mirror engaged") || l.includes("Manual Mirror"));
    const finishedLog = [...logs].reverse().find(l => l.includes("Finish signaled") || l.includes("All nodes processed"));

    if (dlLog && (!finishedLog || logs.indexOf(dlLog) > logs.indexOf(finishedLog))) {
      phase = "SCAFFOLDING (DOWNLOADING)";
    } else if (finishedLog && !dlLog) {
      phase = "COMPLETE";
      networkStatus = "Cooldown";
    }
  }

  // Calculate generic speeds
  const totalDownloaded = Object.values(downloadProgress).reduce((acc: number, p: any) => acc + (p.bytes_downloaded || 0), 0);
  const speedsArray = Object.values(downloadProgress).map((p: any) => p.speed_bps || 0);
  const currentSpeed = speedsArray.length > 0 ? speedsArray.reduce((acc, v) => acc + v, 0) : 0;

  const speedMb = (currentSpeed / 1048576).toFixed(2);
  const downloadedMb = (totalDownloaded / 1048576).toFixed(2);

  return (
    <div className="ops-dashboard">
      <div className="dash-card">
        <div className="dash-icon"><Cpu size={24} /></div>
        <div className="dash-info">
          <div className="dash-title">OPERATION PHASE</div>
          <div className="dash-value" style={{ color: isCrawling ? 'var(--accent-primary)' : 'var(--text-muted)' }}>
            {phase}
            {isCrawling && <RefreshCw size={12} className="aura-spin" style={{ marginLeft: 8 }} />}
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
          <div className="dash-value">{activeModule}</div>
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
          <div className="dash-title">NETWORK I/O</div>
          <div className="dash-value">{speedMb} MB/s</div>
          <div className="dash-sub" style={{ fontFamily: 'JetBrains Mono' }}>Total: {downloadedMb} MB</div>
        </div>
      </div>
    </div>
  );
}
