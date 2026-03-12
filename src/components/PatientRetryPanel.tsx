import { Timer, RefreshCw, CheckCircle, XCircle, AlertTriangle, Zap, Clock } from "lucide-react";
import "./PatientRetryPanel.css";

export interface PatientRetryRound {
  round: number;
  status: "waiting" | "probing" | "success" | "failed" | "timeout" | "cancelled";
  resetNodes: number;
  timestamp: number;
  host?: string;
  latencyMs?: number;
  nextRetryMins?: number;
}

export interface PatientRetryState {
  active: boolean;
  intervalMins: number;
  maxRetries: number;
  totalNodes: number;
  rounds: PatientRetryRound[];
  startedAt: number | null;
  countdownSeconds: number;
  currentRound: number;
}

interface PatientRetryPanelProps {
  retryState: PatientRetryState;
}

function statusIcon(status: PatientRetryRound["status"]) {
  switch (status) {
    case "waiting":
      return <Clock size={14} className="retry-icon retry-icon--waiting" />;
    case "probing":
      return <RefreshCw size={14} className="retry-icon retry-icon--probing" />;
    case "success":
      return <CheckCircle size={14} className="retry-icon retry-icon--success" />;
    case "failed":
      return <XCircle size={14} className="retry-icon retry-icon--failed" />;
    case "timeout":
      return <AlertTriangle size={14} className="retry-icon retry-icon--timeout" />;
    case "cancelled":
      return <XCircle size={14} className="retry-icon retry-icon--cancelled" />;
    default:
      return null;
  }
}

function statusLabel(status: PatientRetryRound["status"]) {
  switch (status) {
    case "waiting": return "WAITING";
    case "probing": return "PROBING";
    case "success": return "ALIVE";
    case "failed": return "DEAD";
    case "timeout": return "TIMEOUT";
    case "cancelled": return "CANCELLED";
  }
}

function formatDuration(seconds: number): string {
  if (seconds <= 0) return "0s";
  const h = Math.floor(seconds / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  const s = seconds % 60;
  if (h > 0) return `${h}h ${m}m ${s}s`;
  if (m > 0) return `${m}m ${s}s`;
  return `${s}s`;
}

function CountdownBar({ seconds, totalSeconds }: { seconds: number; totalSeconds: number }) {
  const pct = totalSeconds > 0 ? Math.max(0, Math.min(100, ((totalSeconds - seconds) / totalSeconds) * 100)) : 0;
  return (
    <div className="countdown-track" data-testid="patient-retry-countdown-bar">
      <div className="countdown-fill" style={{ width: `${pct}%` }} />
      <span className="countdown-label">{formatDuration(seconds)}</span>
    </div>
  );
}

export function PatientRetryPanel({ retryState }: PatientRetryPanelProps) {
  if (!retryState.active && retryState.rounds.length === 0) {
    return null;
  }

  const isActive = retryState.active;
  const succeeded = retryState.rounds.find(r => r.status === "success");
  const failed = retryState.rounds.filter(r => r.status === "failed" || r.status === "timeout");
  const cancelled = retryState.rounds.find(r => r.status === "cancelled");
  const totalElapsed = retryState.startedAt
    ? Math.floor((Date.now() - retryState.startedAt) / 1000)
    : 0;

  return (
    <div
      className={`patient-retry-panel ${isActive ? "patient-retry-panel--active" : ""} ${succeeded ? "patient-retry-panel--success" : ""}`}
      data-testid="patient-retry-panel"
    >
      <div className="retry-header">
        <div className="retry-header-left">
          <Timer size={18} className="retry-header-icon" />
          <span className="retry-header-title">PATIENT RETRY MODE</span>
          {isActive && (
            <span className="retry-badge retry-badge--active" data-testid="patient-retry-active-badge">
              <Zap size={10} /> ACTIVE
            </span>
          )}
          {succeeded && (
            <span className="retry-badge retry-badge--success" data-testid="patient-retry-success-badge">
              <CheckCircle size={10} /> NODE ALIVE
            </span>
          )}
          {cancelled && !succeeded && (
            <span className="retry-badge retry-badge--cancelled">
              <XCircle size={10} /> CANCELLED
            </span>
          )}
          {!isActive && !succeeded && !cancelled && failed.length > 0 && (
            <span className="retry-badge retry-badge--exhausted">
              EXHAUSTED
            </span>
          )}
        </div>
        <div className="retry-header-right" style={{ fontFamily: "JetBrains Mono" }}>
          Round {retryState.currentRound}/{retryState.maxRetries} · {retryState.totalNodes} nodes · {retryState.intervalMins}m interval
        </div>
      </div>

      {/* Countdown bar */}
      {isActive && retryState.countdownSeconds > 0 && (
        <CountdownBar
          seconds={retryState.countdownSeconds}
          totalSeconds={retryState.intervalMins * 60}
        />
      )}

      {/* High-level stats row */}
      <div className="retry-stats-row" data-testid="patient-retry-stats">
        <div className="retry-stat">
          <span className="retry-stat-label">Elapsed</span>
          <span className="retry-stat-value">{formatDuration(totalElapsed)}</span>
        </div>
        <div className="retry-stat">
          <span className="retry-stat-label">Rounds</span>
          <span className="retry-stat-value">{retryState.rounds.length}</span>
        </div>
        <div className="retry-stat">
          <span className="retry-stat-label">Failed</span>
          <span className="retry-stat-value retry-stat-value--danger">{failed.length}</span>
        </div>
        <div className="retry-stat">
          <span className="retry-stat-label">Outcome</span>
          <span className={`retry-stat-value ${succeeded ? "retry-stat-value--success" : ""}`}>
            {succeeded ? `✅ ${succeeded.host}` : (cancelled ? "Cancelled" : (isActive ? "Pending..." : "All Dead"))}
          </span>
        </div>
        {succeeded && (
          <div className="retry-stat">
            <span className="retry-stat-label">Latency</span>
            <span className="retry-stat-value retry-stat-value--success">{succeeded.latencyMs || 0}ms</span>
          </div>
        )}
      </div>

      {/* Round audit log */}
      {retryState.rounds.length > 0 && (
        <div className="retry-rounds-log" data-testid="patient-retry-rounds-log">
          <div className="retry-rounds-header">
            <span>ROUND</span>
            <span>STATUS</span>
            <span>NODES RESET</span>
            <span>RESULT</span>
            <span>TIME</span>
          </div>
          {retryState.rounds.map((round) => (
            <div
              key={round.round}
              className={`retry-round-row retry-round-row--${round.status}`}
              data-testid={`patient-retry-round-${round.round}`}
            >
              <span className="retry-round-num">#{round.round}</span>
              <span className="retry-round-status">
                {statusIcon(round.status)} {statusLabel(round.status)}
              </span>
              <span className="retry-round-nodes">{round.resetNodes}</span>
              <span className="retry-round-result">
                {round.status === "success"
                  ? `${round.host} (${round.latencyMs}ms)`
                  : round.status === "failed"
                    ? "All nodes offline"
                    : round.status === "timeout"
                      ? "Discovery timeout"
                      : round.status === "cancelled"
                        ? "User abort"
                        : round.status === "probing"
                          ? "In progress..."
                          : `Next in ${round.nextRetryMins || retryState.intervalMins}m`}
              </span>
              <span className="retry-round-time">
                {new Date(round.timestamp).toLocaleTimeString()}
              </span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
