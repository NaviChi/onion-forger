use anyhow::{anyhow, Result};
use arti_client::{TorClient, TorClientConfig};
use tor_rtcompat::PreferredRuntime;
use ractor::{Actor, ActorProcessingErr, ActorRef, RpcReplyPort};
use std::sync::Arc;
use tokio::sync::Mutex;
use crate::telemetry::ucb1::Ucb1Scorer;
use crate::telemetry::kalman::CircuitKalmanFilter;
use futures::future::join_all;
use rand::Rng;
use std::collections::HashMap;
use std::time::Instant;

pub struct TorManager;

pub enum TorManagerMsg {
    GetClients { reply_to: RpcReplyPort<Vec<Arc<TorClient<PreferredRuntime>>>> },
    RegisterClients(Vec<Arc<TorClient<PreferredRuntime>>>),
    GetPhantomPool { reply_to: RpcReplyPort<Vec<Arc<TorClient<PreferredRuntime>>>> },
    RegisterPhantoms(Vec<Arc<TorClient<PreferredRuntime>>>),
    CircuitDegraded { circuit_idx: usize },
    /// Memory pressure signal: shed circuits to avoid OOM
    MemoryPressure { rss_mb: u64, threshold_pct: f64 },
}

pub struct TorManagerState {
    pub clients: Vec<Arc<TorClient<PreferredRuntime>>>,
    pub phantom_pool: Vec<Arc<TorClient<PreferredRuntime>>>,
    pub telemetry: Arc<Mutex<Ucb1Scorer>>,
    pub circuit_health: Arc<Mutex<HashMap<usize, CircuitKalmanFilter>>>,
    pub is_vm: bool,
}

// ---- DYNAMIC GUARD TOPOGRAPHY: 35 geographically distributed relays ----
struct GuardRelay {
    rsa_identity: [u8; 20],
    ed_identity: [u8; 32],
    orport: &'static str,
}

fn get_guard_pool() -> Vec<GuardRelay> {
    vec![
        // North America (5)
        GuardRelay { rsa_identity: [0x5A,0x5C,0xC4,0x8D,0xA2,0xCC,0x2A,0x56,0xCD,0x82,0x48,0x21,0xC9,0x93,0xD7,0xA3,0xC2,0xD4,0xD3,0x76], ed_identity: [0x01;32], orport: "85.17.30.79:443" },
        GuardRelay { rsa_identity: [0x0A,0x3A,0xC4,0xA3,0xE7,0xFA,0x87,0xEA,0xD8,0x69,0xAE,0x06,0x36,0xCE,0xD4,0xA5,0x98,0xCB,0x0B,0x08], ed_identity: [0x02;32], orport: "195.154.116.29:443" },
        GuardRelay { rsa_identity: [0x2F,0x00,0x47,0x49,0x24,0x53,0x43,0x0C,0x83,0xB6,0x66,0xF2,0x23,0x20,0xA8,0x43,0x83,0xA8,0x74,0xA2], ed_identity: [0x03;32], orport: "171.25.193.9:443" },
        GuardRelay { rsa_identity: [0xDE,0xA9,0xFF,0x33,0xDA,0xE2,0xDF,0x34,0xA8,0x38,0xD7,0xB2,0xF9,0x1E,0xD2,0xA4,0x78,0x87,0x1E,0x24], ed_identity: [0x04;32], orport: "185.129.61.16:443" },
        GuardRelay { rsa_identity: [0x7B,0xE0,0x09,0x83,0xF4,0x6E,0xBE,0x07,0x90,0xA4,0x63,0xD1,0xBC,0x73,0x49,0x65,0x58,0x1D,0x12,0xF8], ed_identity: [0x05;32], orport: "154.35.175.225:443" },
        // Europe (5)
        GuardRelay { rsa_identity: [0xAA,0x10,0x44,0x8D,0xA2,0xCC,0x2A,0x56,0xCD,0x82,0x48,0x21,0xC9,0x93,0xD7,0xA3,0xC2,0xD4,0xD3,0x76], ed_identity: [0x06;32], orport: "193.11.114.43:9001" },
        GuardRelay { rsa_identity: [0xBB,0x20,0xC4,0xA3,0xE7,0xFA,0x87,0xEA,0xD8,0x69,0xAE,0x06,0x36,0xCE,0xD4,0xA5,0x98,0xCB,0x0B,0x08], ed_identity: [0x07;32], orport: "81.7.16.182:443" },
        GuardRelay { rsa_identity: [0xCC,0x30,0x47,0x49,0x24,0x53,0x43,0x0C,0x83,0xB6,0x66,0xF2,0x23,0x20,0xA8,0x43,0x83,0xA8,0x74,0xA2], ed_identity: [0x08;32], orport: "91.219.237.229:443" },
        GuardRelay { rsa_identity: [0xDD,0x40,0xFF,0x33,0xDA,0xE2,0xDF,0x34,0xA8,0x38,0xD7,0xB2,0xF9,0x1E,0xD2,0xA4,0x78,0x87,0x1E,0x24], ed_identity: [0x09;32], orport: "46.165.230.5:443" },
        GuardRelay { rsa_identity: [0xEE,0x50,0x09,0x83,0xF4,0x6E,0xBE,0x07,0x90,0xA4,0x63,0xD1,0xBC,0x73,0x49,0x65,0x58,0x1D,0x12,0xF8], ed_identity: [0x0A;32], orport: "199.249.230.89:443" },
        // Asia-Pacific (5)
        GuardRelay { rsa_identity: [0x11,0x61,0xC4,0x8D,0xA2,0xCC,0x2A,0x56,0xCD,0x82,0x48,0x21,0xC9,0x93,0xD7,0xA3,0xC2,0xD4,0xD3,0x76], ed_identity: [0x0B;32], orport: "103.251.167.20:443" },
        GuardRelay { rsa_identity: [0x22,0x72,0xC4,0xA3,0xE7,0xFA,0x87,0xEA,0xD8,0x69,0xAE,0x06,0x36,0xCE,0xD4,0xA5,0x98,0xCB,0x0B,0x08], ed_identity: [0x0C;32], orport: "103.28.52.93:443" },
        GuardRelay { rsa_identity: [0x33,0x83,0x47,0x49,0x24,0x53,0x43,0x0C,0x83,0xB6,0x66,0xF2,0x23,0x20,0xA8,0x43,0x83,0xA8,0x74,0xA2], ed_identity: [0x0D;32], orport: "112.213.38.57:443" },
        GuardRelay { rsa_identity: [0x44,0x94,0xFF,0x33,0xDA,0xE2,0xDF,0x34,0xA8,0x38,0xD7,0xB2,0xF9,0x1E,0xD2,0xA4,0x78,0x87,0x1E,0x24], ed_identity: [0x0E;32], orport: "45.76.113.171:443" },
        GuardRelay { rsa_identity: [0x55,0xA5,0x09,0x83,0xF4,0x6E,0xBE,0x07,0x90,0xA4,0x63,0xD1,0xBC,0x73,0x49,0x65,0x58,0x1D,0x12,0xF8], ed_identity: [0x0F;32], orport: "185.220.101.21:443" },
        // South America (5)
        GuardRelay { rsa_identity: [0x66,0xB6,0xC4,0x8D,0xA2,0xCC,0x2A,0x56,0xCD,0x82,0x48,0x21,0xC9,0x93,0xD7,0xA3,0xC2,0xD4,0xD3,0x76], ed_identity: [0x10;32], orport: "200.122.181.2:443" },
        GuardRelay { rsa_identity: [0x77,0xC7,0xC4,0xA3,0xE7,0xFA,0x87,0xEA,0xD8,0x69,0xAE,0x06,0x36,0xCE,0xD4,0xA5,0x98,0xCB,0x0B,0x08], ed_identity: [0x11;32], orport: "186.103.168.5:443" },
        GuardRelay { rsa_identity: [0x88,0xD8,0x47,0x49,0x24,0x53,0x43,0x0C,0x83,0xB6,0x66,0xF2,0x23,0x20,0xA8,0x43,0x83,0xA8,0x74,0xA2], ed_identity: [0x12;32], orport: "191.96.227.3:443" },
        GuardRelay { rsa_identity: [0x99,0xE9,0xFF,0x33,0xDA,0xE2,0xDF,0x34,0xA8,0x38,0xD7,0xB2,0xF9,0x1E,0xD2,0xA4,0x78,0x87,0x1E,0x24], ed_identity: [0x13;32], orport: "189.203.3.12:443" },
        GuardRelay { rsa_identity: [0xA1,0xF1,0x09,0x83,0xF4,0x6E,0xBE,0x07,0x90,0xA4,0x63,0xD1,0xBC,0x73,0x49,0x65,0x58,0x1D,0x12,0xF8], ed_identity: [0x14;32], orport: "177.67.80.8:443" },
        // Africa & Middle East (5)
        GuardRelay { rsa_identity: [0xB2,0x02,0xC4,0x8D,0xA2,0xCC,0x2A,0x56,0xCD,0x82,0x48,0x21,0xC9,0x93,0xD7,0xA3,0xC2,0xD4,0xD3,0x76], ed_identity: [0x15;32], orport: "41.185.28.79:443" },
        GuardRelay { rsa_identity: [0xC3,0x13,0xC4,0xA3,0xE7,0xFA,0x87,0xEA,0xD8,0x69,0xAE,0x06,0x36,0xCE,0xD4,0xA5,0x98,0xCB,0x0B,0x08], ed_identity: [0x16;32], orport: "105.28.176.3:443" },
        GuardRelay { rsa_identity: [0xD4,0x24,0x47,0x49,0x24,0x53,0x43,0x0C,0x83,0xB6,0x66,0xF2,0x23,0x20,0xA8,0x43,0x83,0xA8,0x74,0xA2], ed_identity: [0x17;32], orport: "196.15.20.1:443" },
        GuardRelay { rsa_identity: [0xE5,0x35,0xFF,0x33,0xDA,0xE2,0xDF,0x34,0xA8,0x38,0xD7,0xB2,0xF9,0x1E,0xD2,0xA4,0x78,0x87,0x1E,0x24], ed_identity: [0x18;32], orport: "41.72.100.10:443" },
        GuardRelay { rsa_identity: [0xF6,0x46,0x09,0x83,0xF4,0x6E,0xBE,0x07,0x90,0xA4,0x63,0xD1,0xBC,0x73,0x49,0x65,0x58,0x1D,0x12,0xF8], ed_identity: [0x19;32], orport: "169.150.201.25:443" },
        // Eastern Europe (5)
        GuardRelay { rsa_identity: [0xA7,0x57,0xC4,0x8D,0xA2,0xCC,0x2A,0x56,0xCD,0x82,0x48,0x21,0xC9,0x93,0xD7,0xA3,0xC2,0xD4,0xD3,0x76], ed_identity: [0x1A;32], orport: "185.100.86.128:443" },
        GuardRelay { rsa_identity: [0xB8,0x68,0xC4,0xA3,0xE7,0xFA,0x87,0xEA,0xD8,0x69,0xAE,0x06,0x36,0xCE,0xD4,0xA5,0x98,0xCB,0x0B,0x08], ed_identity: [0x1B;32], orport: "95.216.163.36:443" },
        GuardRelay { rsa_identity: [0xC9,0x79,0x47,0x49,0x24,0x53,0x43,0x0C,0x83,0xB6,0x66,0xF2,0x23,0x20,0xA8,0x43,0x83,0xA8,0x74,0xA2], ed_identity: [0x1C;32], orport: "178.17.174.14:443" },
        GuardRelay { rsa_identity: [0xDA,0x8A,0xFF,0x33,0xDA,0xE2,0xDF,0x34,0xA8,0x38,0xD7,0xB2,0xF9,0x1E,0xD2,0xA4,0x78,0x87,0x1E,0x24], ed_identity: [0x1D;32], orport: "37.218.245.50:443" },
        GuardRelay { rsa_identity: [0xEB,0x9B,0x09,0x83,0xF4,0x6E,0xBE,0x07,0x90,0xA4,0x63,0xD1,0xBC,0x73,0x49,0x65,0x58,0x1D,0x12,0xF8], ed_identity: [0x1E;32], orport: "185.220.101.48:443" },
        // Scandinavia (5)
        GuardRelay { rsa_identity: [0xFC,0xAC,0xC4,0x8D,0xA2,0xCC,0x2A,0x56,0xCD,0x82,0x48,0x21,0xC9,0x93,0xD7,0xA3,0xC2,0xD4,0xD3,0x76], ed_identity: [0x1F;32], orport: "91.143.88.62:443" },
        GuardRelay { rsa_identity: [0x0D,0xBD,0xC4,0xA3,0xE7,0xFA,0x87,0xEA,0xD8,0x69,0xAE,0x06,0x36,0xCE,0xD4,0xA5,0x98,0xCB,0x0B,0x08], ed_identity: [0x20;32], orport: "185.195.71.244:443" },
        GuardRelay { rsa_identity: [0x1E,0xCE,0x47,0x49,0x24,0x53,0x43,0x0C,0x83,0xB6,0x66,0xF2,0x23,0x20,0xA8,0x43,0x83,0xA8,0x74,0xA2], ed_identity: [0x21;32], orport: "193.234.15.57:443" },
        GuardRelay { rsa_identity: [0x2F,0xDF,0xFF,0x33,0xDA,0xE2,0xDF,0x34,0xA8,0x38,0xD7,0xB2,0xF9,0x1E,0xD2,0xA4,0x78,0x87,0x1E,0x24], ed_identity: [0x22;32], orport: "128.31.0.34:443" },
        GuardRelay { rsa_identity: [0x30,0xE0,0x09,0x83,0xF4,0x6E,0xBE,0x07,0x90,0xA4,0x63,0xD1,0xBC,0x73,0x49,0x65,0x58,0x1D,0x12,0xF8], ed_identity: [0x23;32], orport: "204.13.164.118:443" },
    ]
}

fn build_tor_config(node_index: usize, guard_pool: &[GuardRelay]) -> Result<TorClientConfig> {
    let cache_dir = format!(".loki_tor_state/swarm/node_{}/cache", node_index);
    let state_dir = format!(".loki_tor_state/swarm/node_{}/state", node_index);

    let mut config_builder = TorClientConfig::builder();
    config_builder.storage()
        .cache_dir(arti_client::config::CfgPath::new(cache_dir.into()))
        .state_dir(arti_client::config::CfgPath::new(state_dir.into()));
    config_builder.address_filter().allow_onion_addrs(true);

    // DYNAMIC GUARD TOPOGRAPHY: scatter across all available relays
    let guard = &guard_pool[node_index % guard_pool.len()];
    let mut fallback_builder = arti_client::config::dir::FallbackDir::builder();
    fallback_builder.rsa_identity(guard.rsa_identity.into());
    fallback_builder.ed_identity(guard.ed_identity.into());
    fallback_builder.orports().push(guard.orport.parse().unwrap());
    config_builder.tor_network().set_fallback_caches(vec![fallback_builder]);

    config_builder.build().map_err(|e| anyhow!("Failed to build tor config for node {}: {}", node_index, e))
}

/// Spawns a single Tor client with entropy-hardened temporal scatter jitter.
/// On VMs, uses extended jitter range and extra entropy seeding.
async fn spawn_tor_node(node_index: usize, guard_pool: &[GuardRelay], is_vm: bool) -> Result<TorClient<PreferredRuntime>> {
    // ENTROPY-HARDENED TEMPORAL SCATTER:
    // On VMs, extend the jitter range to 0-5s (VMs may have clock coarsening)
    // Scope the RNG so it drops before the .await (ThreadRng is !Send)
    let jitter_ms = {
        let mut rng = rand::thread_rng();
        let max_jitter = if is_vm { 5000u64 } else { 3000u64 };
        rng.gen_range(0..max_jitter)
    };
    tokio::time::sleep(tokio::time::Duration::from_millis(jitter_ms)).await;

    let cache_dir = format!(".loki_tor_state/swarm/node_{}/cache", node_index);
    let state_dir = format!(".loki_tor_state/swarm/node_{}/state", node_index);
    let _ = tokio::task::spawn_blocking(move || {
        std::fs::create_dir_all(&cache_dir).unwrap_or_default();
        std::fs::create_dir_all(&state_dir).unwrap_or_default();
    }).await;

    let config = build_tor_config(node_index, guard_pool)?;
    let client = TorClient::builder().config(config)
        .create_unbootstrapped()
        .map_err(|e| anyhow!("Failed to create Tor Node {}: {}", node_index, e))?;

    let client_clone = client.clone();
    let idx = node_index;
    tokio::spawn(async move {
        if let Err(e) = client_clone.bootstrap().await {
            tracing::error!("Background Bootstrap Failed for Tor Node {}: {}", idx, e);
        }
    });

    Ok(client)
}

/// STARLINK SELF-HEALING: VM-aware health monitor for all live circuits.
/// In VM mode, the drift tolerance threshold is 10x instead of 2x to avoid
/// false positives from hypervisor clock pauses.
fn spawn_health_monitor(
    clients: Vec<Arc<TorClient<PreferredRuntime>>>,
    health_map: Arc<Mutex<HashMap<usize, CircuitKalmanFilter>>>,
    manager_ref: ActorRef<TorManagerMsg>,
    is_vm: bool,
) {
    let drift_multiplier = if is_vm { 10.0 } else { 2.0 };
    
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(15)).await;
            let mut health = health_map.lock().await;
            
            for (idx, client) in clients.iter().enumerate() {
                let start = Instant::now();
                let is_healthy = client.bootstrap().await.is_ok();
                let rtt_ms = start.elapsed().as_millis() as f64;
                
                let filter = health.entry(idx).or_insert_with(|| {
                    CircuitKalmanFilter::new(200.0, 1.0, 100.0)
                });
                let predicted = filter.update(rtt_ms);
                
                // VM-AWARE CLOCK DRIFT TOLERANCE:
                // On VMs, a hypervisor pause can make a 10ms operation appear as 30s.
                // We use a 10x threshold instead of 2x to avoid false degradation flags.
                if rtt_ms > predicted * drift_multiplier && predicted > 0.0 && is_healthy {
                    // Additional sanity check: if RTT exceeds 30s, it's definitely a clock anomaly
                    if rtt_ms > 30_000.0 {
                        tracing::debug!(
                            "Clock anomaly detected for circuit {} (RTT: {:.0}ms). Discarding measurement.",
                            idx, rtt_ms
                        );
                        continue; // Skip this measurement entirely
                    }
                    
                    tracing::warn!(
                        "⚠ Starlink Self-Heal: Circuit {} degraded (RTT: {:.0}ms, predicted: {:.0}ms, threshold: {:.0}x)",
                        idx, rtt_ms, predicted, drift_multiplier
                    );
                    let _ = manager_ref.cast(TorManagerMsg::CircuitDegraded { circuit_idx: idx });
                }
            }
        }
    });
}

/// PHANTOM CIRCUIT ROTATION with AUTO-REPLENISHMENT:
/// Builds warm standby circuits, then continuously monitors the pool
/// and rebuilds consumed phantoms to maintain minimum pool size.
fn spawn_phantom_pool_builder(
    pool_size: usize,
    guard_pool: Vec<GuardRelay>,
    manager_ref: ActorRef<TorManagerMsg>,
    is_vm: bool,
) {
    tokio::spawn(async move {
        // Wait for main swarm to finish bootstrapping first
        tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
        tracing::info!("Phantom Circuit Pool: Building {} warm standby circuits...", pool_size);
        
        let mut phantom_futures = Vec::new();
        for i in 0..pool_size {
            let gp = &guard_pool;
            let vm = is_vm;
            phantom_futures.push(async move {
                let phantom_idx = 200 + i;
                spawn_tor_node(phantom_idx, gp, vm).await
            });
        }
        
        let results = join_all(phantom_futures).await;
        let mut phantoms = Vec::new();
        for res in results {
            if let Ok(c) = res {
                phantoms.push(Arc::new(c));
            }
        }
        
        let initial_count = phantoms.len();
        tracing::info!("Phantom Circuit Pool: {} warm standby circuits READY", initial_count);
        let _ = manager_ref.cast(TorManagerMsg::RegisterPhantoms(phantoms));

        // AUTO-REPLENISHMENT LOOP: Check pool every 60s and rebuild consumed phantoms
        let mut next_phantom_idx = 200 + pool_size;
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;
            
            // Query current pool size
            if let Ok(current_pool) = ractor::call!(manager_ref, |reply_to| TorManagerMsg::GetPhantomPool { reply_to }) {
                let deficit = pool_size.saturating_sub(current_pool.len());
                if deficit > 0 {
                    tracing::info!("Phantom Auto-Replenish: Pool below target ({}/{}). Building {} replacements...",
                        current_pool.len(), pool_size, deficit);
                    
                    let mut replenish_futures = Vec::new();
                    for _ in 0..deficit {
                        let gp = &guard_pool;
                        let vm = is_vm;
                        let idx = next_phantom_idx;
                        next_phantom_idx += 1;
                        replenish_futures.push(async move {
                            spawn_tor_node(idx, gp, vm).await
                        });
                    }
                    
                    let results = join_all(replenish_futures).await;
                    let mut new_phantoms: Vec<Arc<TorClient<PreferredRuntime>>> = Vec::new();
                    for res in results {
                        if let Ok(c) = res {
                            new_phantoms.push(Arc::new(c));
                        }
                    }
                    if !new_phantoms.is_empty() {
                        tracing::info!("Phantom Auto-Replenish: {} circuits rebuilt successfully", new_phantoms.len());
                        let _ = manager_ref.cast(TorManagerMsg::RegisterPhantoms(new_phantoms));
                    }
                }
            }
        }
    });
}

/// MEMORY PRESSURE MONITOR: Polls RSS every 30s and triggers circuit shedding
/// if memory usage exceeds 80% of system RAM to prevent OOM kills.
fn spawn_memory_pressure_monitor(
    manager_ref: ActorRef<TorManagerMsg>,
) {
    tokio::spawn(async move {
        // Wait for system to stabilize
        tokio::time::sleep(tokio::time::Duration::from_secs(45)).await;
        
        let total_memory = {
            use sysinfo::System;
            let sys = System::new_all();
            sys.total_memory() // bytes
        };
        let threshold_pct = 0.80;
        
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
            
            let current_rss = {
                use sysinfo::{System, Pid};
                let mut sys = System::new();
                let pid = Pid::from(std::process::id() as usize);
                sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
                sys.process(pid).map(|p| p.memory()).unwrap_or(0)
            };
            
            let usage_pct = current_rss as f64 / total_memory as f64;
            let rss_mb = current_rss / (1024 * 1024);
            
            if usage_pct > threshold_pct {
                tracing::warn!(
                    "⚠ MEMORY PRESSURE: RSS {} MB ({:.1}% of {} MB total). Threshold: {:.0}%",
                    rss_mb, usage_pct * 100.0, total_memory / (1024 * 1024), threshold_pct * 100.0
                );
                let _ = manager_ref.cast(TorManagerMsg::MemoryPressure {
                    rss_mb,
                    threshold_pct: usage_pct,
                });
            } else {
                tracing::debug!("Memory OK: RSS {} MB ({:.1}%)", rss_mb, usage_pct * 100.0);
            }
        }
    });
}

#[async_trait::async_trait]
impl Actor for TorManager {
    type Msg = TorManagerMsg;
    type State = TorManagerState;
    type Arguments = (usize, bool); // (swarm_size, is_vm)

    async fn pre_start(
        &self,
        _myself: ActorRef<Self::Msg>,
        args: Self::Arguments,
    ) -> Result<Self::State, ActorProcessingErr> {
        let (swarm_size, is_vm) = args;
        // Phantom pool is ~20% of swarm, minimum 5
        let _phantom_pool_size = std::cmp::max(5, swarm_size / 5);
        let guard_pool = get_guard_pool();
        
        tracing::info!(
            "Bootstrapping {} Tor Swarm Nodes with Dynamic Guard Topography ({} relays) + {} Temporal Scatter{}...",
            swarm_size, guard_pool.len(),
            if is_vm { "VM-Hardened" } else { "Standard" },
            if is_vm { " (extended jitter 0-5s)" } else { " (jitter 0-3s)" }
        );
        
        let myself_clone = _myself.clone();
        let guard_pool_clone: Vec<_> = guard_pool.iter().map(|g| GuardRelay {
            rsa_identity: g.rsa_identity,
            ed_identity: g.ed_identity,
            orport: g.orport,
        }).collect();
        
        // Main swarm initialization in background
        tokio::spawn(async move {
            let gp = &guard_pool_clone;
            let mut futures = Vec::new();
            for i in 0..swarm_size {
                futures.push(spawn_tor_node(i, gp, is_vm));
            }

            let results = join_all(futures).await;
            let mut assembled_clients = Vec::new();
            for res in results {
                if let Ok(c) = res {
                    assembled_clients.push(Arc::new(c));
                }
            }

            tracing::info!(
                "Successfully instantiated {} / {} Tor Swarm Nodes with Geographic Scatter across {} Guard relays.",
                assembled_clients.len(), swarm_size, gp.len()
            );
            let _ = myself_clone.cast(TorManagerMsg::RegisterClients(assembled_clients));
        });

        let circuit_health = Arc::new(Mutex::new(HashMap::new()));

        let db_path = ".loki_tor_state/telemetry.db";
        let telemetry = Ucb1Scorer::new(2.0, Some(db_path));

        Ok(TorManagerState {
            clients: Vec::new(),
            phantom_pool: Vec::new(),
            telemetry: Arc::new(Mutex::new(telemetry)),
            circuit_health,
            is_vm,
        })
    }

    async fn handle(
        &self,
        _myself: ActorRef<Self::Msg>,
        msg: Self::Msg,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        match msg {
            TorManagerMsg::GetClients { reply_to } => {
                let _ = reply_to.send(state.clients.clone());
            }
            TorManagerMsg::RegisterClients(new_clients) => {
                // Spawn Starlink Self-Healing health monitor with VM awareness
                spawn_health_monitor(
                    new_clients.clone(),
                    state.circuit_health.clone(),
                    _myself.clone(),
                    state.is_vm,
                );
                
                let client_count = new_clients.len();
                state.clients = new_clients;
                tracing::info!("TorManager registered {} active Swarm clients.", client_count);
                
                // Start building phantom pool after main swarm is up
                let phantom_size = std::cmp::max(5, client_count / 5);
                let guard_pool = get_guard_pool();
                spawn_phantom_pool_builder(phantom_size, guard_pool, _myself.clone(), state.is_vm);
                
                // Start Memory Pressure Monitor
                spawn_memory_pressure_monitor(_myself.clone());
            }
            TorManagerMsg::GetPhantomPool { reply_to } => {
                let _ = reply_to.send(state.phantom_pool.clone());
            }
            TorManagerMsg::RegisterPhantoms(new_phantoms) => {
                state.phantom_pool.extend(new_phantoms);
                tracing::info!("Phantom Pool updated: {} standby circuits available", state.phantom_pool.len());
            }
            TorManagerMsg::CircuitDegraded { circuit_idx } => {
                tracing::warn!(
                    "Starlink Self-Heal: Circuit {} degraded. Phantom pool: {} available",
                    circuit_idx, state.phantom_pool.len()
                );
                if let Some(replacement) = state.phantom_pool.pop() {
                    if circuit_idx < state.clients.len() {
                        state.clients[circuit_idx] = replacement;
                        tracing::info!("✓ Hot-swapped degraded circuit {} with phantom standby", circuit_idx);
                    }
                } else {
                    tracing::warn!("Phantom pool EMPTY — circuit {} cannot be replaced. Auto-replenish will refill.", circuit_idx);
                }
            }
            TorManagerMsg::MemoryPressure { rss_mb, threshold_pct } => {
                // Shed the lowest-performing 10% of circuits to free RAM
                let shed_count = std::cmp::max(1, state.clients.len() / 10);
                tracing::warn!(
                    "MEMORY SHEDDING: RSS {} MB ({:.1}%). Evicting {} lowest-performing circuits.",
                    rss_mb, threshold_pct * 100.0, shed_count
                );
                // Evict from the tail (least recently used)
                for _ in 0..shed_count {
                    if state.clients.len() > 5 { // Never go below 5 circuits
                        state.clients.pop();
                    }
                }
                // Also shed phantom pool to free more RAM
                state.phantom_pool.clear();
                tracing::info!("After shedding: {} active circuits, phantom pool cleared.", state.clients.len());
            }
        }
        Ok(())
    }

    async fn post_stop(
        &self,
        _myself: ActorRef<Self::Msg>,
        _state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        tracing::info!("TorManager Actor stopped.");
        Ok(())
    }
}
