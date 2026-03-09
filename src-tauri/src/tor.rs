//! Phase 44: Tor management module — pure arti-client, zero tor.exe dependency.
//! All Tor networking is handled in-process via `tor_native.rs`.

use anyhow::Result;
use tauri::AppHandle;

use crate::tor_native;

// ============================================================================
// PUBLIC API
// ============================================================================

/// Event emitted to React UI during Tor bootstrap
#[derive(Clone, serde::Serialize)]
pub struct TorStatusEvent {
    pub state: String,
    pub message: String,
    pub daemon_count: usize,
    pub ports: Vec<u16>,
}

/// A Guard that manages Tor lifecycle. Dropping it shuts down all circuits.
pub struct TorProcessGuard {
    pub native_swarm: Option<tor_native::ArtiSwarm>,
}

impl Default for TorProcessGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl TorProcessGuard {
    pub fn new() -> Self {
        Self { native_swarm: None }
    }

    pub fn shutdown_all(&mut self) {
        if let Some(swarm) = self.native_swarm.take() {
            swarm.shutdown();
        }
    }

    pub fn runtime_label(&self) -> &'static str {
        if self.native_swarm.is_some() {
            crate::tor_runtime::runtime_label()
        } else {
            "none"
        }
    }
}

impl Drop for TorProcessGuard {
    fn drop(&mut self) {
        self.shutdown_all();
    }
}

/// Bootstraps `daemon_count` native arti Tor circuits.
/// Returns (TorProcessGuard, Vec<u16>) where Vec<u16> are SOCKS proxy ports.
pub async fn bootstrap_tor_cluster(
    app: AppHandle,
    daemon_count: usize,
    node_offset: usize,
) -> Result<(TorProcessGuard, Vec<u16>)> {
    // Explicitly install the rustls crypto provider globally before building the Arti client
    rustls::crypto::ring::default_provider().install_default().ok();
    let (swarm, ports) =
        tor_native::bootstrap_arti_cluster(app.clone(), daemon_count, node_offset).await?;
    let guard = TorProcessGuard { native_swarm: Some(swarm) };
    Ok((guard, ports))
}

impl TorProcessGuard {
    pub fn get_arti_clients(&self) -> Vec<crate::tor_native::SharedTorClient> {
        if let Some(swarm) = &self.native_swarm {
            swarm.clients.read().unwrap().clone()
        } else {
            Vec::new()
        }
    }
}

/// Request a new identity (NEWNYM) for a Tor circuit.
/// In native arti mode this rotates the live managed client behind the SOCKS port.
pub async fn request_newnym(socks_port: u16) -> Result<()> {
    tor_native::request_newnym_arti(socks_port).await
}





/// Cleanup stale Tor daemons — no-op with native arti (no child processes to clean).
pub fn cleanup_stale_tor_daemons() {
    // No tor.exe processes exist with native arti — nothing to clean.
}

/// Detect active managed Tor SOCKS ports. Returns ports where proxies are listening.
pub fn detect_active_managed_tor_ports() -> Vec<u16> {
    let active = tor_native::active_socks_ports();
    if !active.is_empty() {
        return active;
    }

    // Fallback for legacy callers/tests started outside the managed registry.
    let mut discovered = Vec::new();
    for port in 9051..=9070 {
        if std::net::TcpStream::connect(format!("127.0.0.1:{}", port)).is_ok() {
            discovered.push(port);
        }
    }
    discovered
}

// ============================================================================
// TOURNAMENT TELEMETRY (used by tor_native for adaptive circuit sizing)
// ============================================================================

use std::sync::Mutex;

#[derive(Debug, Clone, Copy)]
struct TournamentTelemetry {
    samples: usize,
    p50_ms: f64,
    p95_ms: f64,
    winner_ratio: f64,
}

impl Default for TournamentTelemetry {
    fn default() -> Self {
        Self {
            samples: 0,
            p50_ms: 0.0,
            p95_ms: 0.0,
            winner_ratio: 1.0,
        }
    }
}

static TOURNAMENT_TELEMETRY: std::sync::OnceLock<Mutex<TournamentTelemetry>> =
    std::sync::OnceLock::new();

pub fn tournament_candidate_count(target: usize) -> usize {
    let target = target.max(1);
    let baseline = if target == 1 {
        2
    } else {
        target + (target / 2).max(1)
    };

    if !dynamic_tournament_enabled() {
        return baseline;
    }

    let telemetry = TOURNAMENT_TELEMETRY
        .get_or_init(|| Mutex::new(TournamentTelemetry::default()))
        .lock()
        .ok()
        .map(|guard| *guard)
        .unwrap_or_default();

    if telemetry.samples < 2 {
        return baseline;
    }

    let latency_spread = if telemetry.p50_ms > 0.0 {
        (telemetry.p95_ms / telemetry.p50_ms).clamp(1.0, 3.0)
    } else {
        1.0
    };
    let reliability_penalty = (1.0 - telemetry.winner_ratio).clamp(0.0, 1.0);
    let dynamic_bonus = ((latency_spread - 1.0) * target as f64 * 0.5
        + reliability_penalty * target as f64)
        .ceil() as usize;
    let adaptive = baseline.saturating_add(dynamic_bonus);

    adaptive.clamp(target + 1, target.saturating_mul(2).max(2))
}

fn dynamic_tournament_enabled() -> bool {
    match std::env::var("CRAWLI_TOURNAMENT_DYNAMIC") {
        Ok(value) => {
            let normalized = value.to_ascii_lowercase();
            normalized == "1" || normalized == "true" || normalized == "on" || normalized == "yes"
        }
        Err(_) => true,
    }
}

fn percentile(mut data: Vec<u64>, p: f64) -> f64 {
    if data.is_empty() {
        return 0.0;
    }
    data.sort_unstable();
    let idx = ((data.len() - 1) as f64 * p.clamp(0.0, 1.0)).round() as usize;
    data[idx] as f64
}

pub fn update_tournament_telemetry(
    ready_durations_ms: &[u64],
    winner_count: usize,
    candidate_count: usize,
) {
    let p50 = percentile(ready_durations_ms.to_vec(), 0.50);
    let p95 = percentile(ready_durations_ms.to_vec(), 0.95);
    let winner_ratio = if candidate_count > 0 {
        (winner_count as f64 / candidate_count as f64).clamp(0.0, 1.0)
    } else {
        0.0
    };

    if let Ok(mut telemetry) = TOURNAMENT_TELEMETRY
        .get_or_init(|| Mutex::new(TournamentTelemetry::default()))
        .lock()
    {
        let alpha = 0.35;
        if telemetry.samples == 0 {
            telemetry.p50_ms = p50;
            telemetry.p95_ms = p95;
            telemetry.winner_ratio = winner_ratio;
        } else {
            telemetry.p50_ms = telemetry.p50_ms * (1.0 - alpha) + p50 * alpha;
            telemetry.p95_ms = telemetry.p95_ms * (1.0 - alpha) + p95 * alpha;
            telemetry.winner_ratio = telemetry.winner_ratio * (1.0 - alpha) + winner_ratio * alpha;
        }
        telemetry.samples = telemetry.samples.saturating_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::tournament_candidate_count;

    #[test]
    fn test_tournament_candidate_count_defaults() {
        assert_eq!(tournament_candidate_count(0), 2);
        assert_eq!(tournament_candidate_count(1), 2);
        assert_eq!(tournament_candidate_count(2), 3);
        assert_eq!(tournament_candidate_count(4), 6);
    }

    #[test]
    fn test_tournament_candidate_count_cap() {
        assert_eq!(tournament_candidate_count(5), 7);
        assert_eq!(tournament_candidate_count(8), 12);
        assert_eq!(tournament_candidate_count(12), 18);
        assert_eq!(tournament_candidate_count(100), 150);
    }
}
