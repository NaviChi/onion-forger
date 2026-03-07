//! Phase 43B: Native Rust Tor Engine (replaces tor.exe dependency)
//!
//! Uses `arti-client` for in-process Tor circuits with:
//! - VM-aware bootstrap jitter to avoid thundering-herd directory fetches
//! - Hot-swappable SOCKS5 bridges for `reqwest` compatibility
//! - Auth-backed stream isolation via `IsolationToken`
//! - Warm standby circuit replacement pool
//! - Lightweight live connectivity probes and memory pressure telemetry

use anyhow::{anyhow, Result};
use arti_client::{IsolationToken, TorClient, TorClientConfig};
use std::collections::{hash_map::Entry, HashMap};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::sync::{OnceLock, RwLock as StdRwLock};
use std::time::Instant;
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tor_rtcompat::PreferredRuntime;

use crate::kalman::KalmanFilter;
use crate::tor_runtime::{jitter_window_ms, runtime_label, state_root};

// ============================================================================
// TOR CLIENT CONFIGURATION (arti-native)
// ============================================================================

const DEFAULT_HEALTH_PROBE_HOST: &str = "check.torproject.org";
const DEFAULT_HEALTH_PROBE_PORT: u16 = 443;
const HEALTH_PROBE_INTERVAL_SECS: u64 = 15;
const HEALTH_PROBE_TIMEOUT_SECS: u64 = 20;
const MAX_CONSECUTIVE_PROBE_ANOMALIES: u8 = 3;
const PHANTOM_POOL_BOOTSTRAP_DELAY_SECS: u64 = 10;
const PHANTOM_POOL_REPLENISH_INTERVAL_SECS: u64 = 20;
const MEMORY_MONITOR_WARMUP_DELAY_SECS: u64 = 30;
const MEMORY_MONITOR_INTERVAL_SECS: u64 = 20;
const DEFAULT_ARTI_CONNECT_TIMEOUT_SECS: u64 = 15;
const DEFAULT_ARTI_REQUEST_TIMEOUT_SECS: u64 = 35;
const DEFAULT_ARTI_REQUEST_MAX_RETRIES: u32 = 8;
const DEFAULT_ARTI_HS_DESC_FETCH_ATTEMPTS: u32 = 5;
const DEFAULT_ARTI_HS_INTRO_REND_ATTEMPTS: u32 = 5;
const DEFAULT_ARTI_PREEMPTIVE_THRESHOLD: usize = 6;
const DEFAULT_ARTI_PREEMPTIVE_MIN_EXIT_CIRCS: usize = 1;
const DEFAULT_ARTI_PREEMPTIVE_PREDICTION_LIFETIME_SECS: u64 = 20 * 60;
const DEFAULT_ARTI_PREEMPTIVE_PORTS: &[u16] = &[80, 443];
const DEFAULT_ARTI_ACTIVE_TARGET_MAX: usize = 24;

fn env_u64(name: &str) -> Option<u64> {
    std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
}

fn env_u32(name: &str) -> Option<u32> {
    std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<u32>().ok())
}

fn env_usize(name: &str) -> Option<usize> {
    std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
}

fn health_probe_interval_secs() -> u64 {
    env_u64("CRAWLI_HEALTH_PROBE_INTERVAL_SECS")
        .unwrap_or(HEALTH_PROBE_INTERVAL_SECS)
        .clamp(5, 120)
}

fn max_consecutive_probe_anomalies(is_vm: bool) -> u8 {
    let default = if is_vm {
        MAX_CONSECUTIVE_PROBE_ANOMALIES.saturating_add(1)
    } else {
        MAX_CONSECUTIVE_PROBE_ANOMALIES
    };
    env_u64("CRAWLI_MAX_CONSECUTIVE_PROBE_ANOMALIES")
        .map(|value| value.clamp(2, 8) as u8)
        .unwrap_or(default)
}

fn phantom_pool_bootstrap_delay_secs() -> u64 {
    env_u64("CRAWLI_PHANTOM_BOOTSTRAP_DELAY_SECS")
        .unwrap_or(PHANTOM_POOL_BOOTSTRAP_DELAY_SECS)
        .clamp(5, 120)
}

fn phantom_pool_replenish_interval_secs() -> u64 {
    env_u64("CRAWLI_PHANTOM_REPLENISH_INTERVAL_SECS")
        .unwrap_or(PHANTOM_POOL_REPLENISH_INTERVAL_SECS)
        .clamp(5, 180)
}

fn memory_monitor_warmup_delay_secs() -> u64 {
    env_u64("CRAWLI_MEMORY_MONITOR_WARMUP_DELAY_SECS")
        .unwrap_or(MEMORY_MONITOR_WARMUP_DELAY_SECS)
        .clamp(10, 180)
}

fn memory_monitor_interval_secs() -> u64 {
    env_u64("CRAWLI_MEMORY_MONITOR_INTERVAL_SECS")
        .unwrap_or(MEMORY_MONITOR_INTERVAL_SECS)
        .clamp(5, 120)
}

fn adaptive_bootstrap_plan(requested: usize) -> (usize, usize) {
    let budget = crate::resource_governor::recommend_bootstrap_budget(requested, None, None);
    let hard_cap = env_usize("CRAWLI_ARTI_ACTIVE_TARGET_MAX")
        .unwrap_or(DEFAULT_ARTI_ACTIVE_TARGET_MAX)
        .clamp(1, 24);
    let target = budget.target_clients.min(hard_cap).max(1);
    let quorum = budget.minimum_ready.clamp(1, target);
    (target, quorum)
}

fn preemptive_ports() -> Vec<u16> {
    std::env::var("CRAWLI_ARTI_PREEMPTIVE_PORTS")
        .ok()
        .map(|value| {
            value
                .split(',')
                .filter_map(|part| part.trim().parse::<u16>().ok())
                .collect::<Vec<_>>()
        })
        .filter(|ports| !ports.is_empty())
        .unwrap_or_else(|| DEFAULT_ARTI_PREEMPTIVE_PORTS.to_vec())
}

fn build_tor_config(node_index: usize) -> Result<TorClientConfig> {
    let state_root = state_root();

    let cache_dir = state_root.join(format!("arti/node_{}/cache", node_index));
    let state_dir = state_root.join(format!("arti/node_{}/state", node_index));

    let mut config_builder = TorClientConfig::builder();
    config_builder
        .storage()
        .cache_dir(arti_client::config::CfgPath::new(
            cache_dir.to_string_lossy().into_owned(),
        ))
        .state_dir(arti_client::config::CfgPath::new(
            state_dir.to_string_lossy().into_owned(),
        ));
    config_builder.address_filter().allow_onion_addrs(true);
    config_builder
        .stream_timeouts()
        .connect_timeout(std::time::Duration::from_secs(
            env_u64("CRAWLI_ARTI_CONNECT_TIMEOUT_SECS")
                .unwrap_or(DEFAULT_ARTI_CONNECT_TIMEOUT_SECS),
        ));
    let preemptive_ports = preemptive_ports();
    let preemptive_builder = config_builder.preemptive_circuits();
    preemptive_builder
        .disable_at_threshold(
            env_usize("CRAWLI_ARTI_PREEMPTIVE_THRESHOLD")
                .unwrap_or(DEFAULT_ARTI_PREEMPTIVE_THRESHOLD),
        )
        .prediction_lifetime(std::time::Duration::from_secs(
            env_u64("CRAWLI_ARTI_PREEMPTIVE_PREDICTION_LIFETIME_SECS")
                .unwrap_or(DEFAULT_ARTI_PREEMPTIVE_PREDICTION_LIFETIME_SECS),
        ))
        .min_exit_circs_for_port(
            env_usize("CRAWLI_ARTI_PREEMPTIVE_MIN_EXIT_CIRCS")
                .unwrap_or(DEFAULT_ARTI_PREEMPTIVE_MIN_EXIT_CIRCS),
        );
    for port in preemptive_ports {
        preemptive_builder.initial_predicted_ports().push(port);
    }
    config_builder
        .circuit_timing()
        .request_timeout(std::time::Duration::from_secs(
            env_u64("CRAWLI_ARTI_REQUEST_TIMEOUT_SECS")
                .unwrap_or(DEFAULT_ARTI_REQUEST_TIMEOUT_SECS),
        ))
        .request_max_retries(
            env_u32("CRAWLI_ARTI_REQUEST_MAX_RETRIES").unwrap_or(DEFAULT_ARTI_REQUEST_MAX_RETRIES),
        )
        .hs_desc_fetch_attempts(
            env_u32("CRAWLI_ARTI_HS_DESC_FETCH_ATTEMPTS")
                .unwrap_or(DEFAULT_ARTI_HS_DESC_FETCH_ATTEMPTS),
        )
        .hs_intro_rend_attempts(
            env_u32("CRAWLI_ARTI_HS_INTRO_REND_ATTEMPTS")
                .unwrap_or(DEFAULT_ARTI_HS_INTRO_REND_ATTEMPTS),
        );

    // NOTE: Keep Arti's built-in directory authority and guard selection policy.
    // Hardcoding relay pools here would drift from network reality and has no effect
    // unless wired into real Arti policy primitives.

    config_builder
        .build()
        .map_err(|e| anyhow!("Failed to build arti config for node {}: {}", node_index, e))
}

pub type SharedTorClient = Arc<RwLock<Arc<TorClient<PreferredRuntime>>>>;
type SharedIsolationCache = Arc<RwLock<HashMap<SocksIsolationKey, IsolationToken>>>;
type SharedCircuitHealth = Arc<RwLock<HashMap<usize, CircuitHealth>>>;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct SocksIsolationKey {
    username: Vec<u8>,
    password: Vec<u8>,
}

#[allow(dead_code)]
#[derive(Clone)]
struct ManagedSocksPort {
    owner_id: u64,
    node_idx: usize,
    client_slot: SharedTorClient,
    isolation_cache: SharedIsolationCache,
}

#[derive(Clone, Debug)]
struct CircuitHealth {
    latency_kalman: KalmanFilter,
    anomaly_streak: u8,
}

impl CircuitHealth {
    fn new() -> Self {
        Self {
            latency_kalman: KalmanFilter::new(0.1, 250.0, 1500.0),
            anomaly_streak: 0,
        }
    }
}

static ACTIVE_SOCKS_PORTS: OnceLock<StdRwLock<HashMap<u16, ManagedSocksPort>>> = OnceLock::new();
static ACTIVE_TOR_CLIENTS: OnceLock<StdRwLock<Vec<SharedTorClient>>> = OnceLock::new();
static ACTIVE_TOR_ISOLATION_CACHES: OnceLock<StdRwLock<Vec<SharedIsolationCache>>> =
    OnceLock::new();
static NEXT_SWARM_ID: AtomicU64 = AtomicU64::new(1);

pub fn active_tor_clients() -> Vec<SharedTorClient> {
    ACTIVE_TOR_CLIENTS
        .get_or_init(|| StdRwLock::new(Vec::new()))
        .read()
        .unwrap()
        .clone()
}

fn socks_registry() -> &'static StdRwLock<HashMap<u16, ManagedSocksPort>> {
    ACTIVE_SOCKS_PORTS.get_or_init(|| StdRwLock::new(HashMap::new()))
}

#[allow(dead_code)]
fn register_socks_port(port: u16, registration: ManagedSocksPort) {
    if let Ok(mut ports) = socks_registry().write() {
        ports.insert(port, registration);
    }
}

#[allow(dead_code)]
fn register_live_client(
    owner_id: u64,
    node_idx: usize,
    client_slot: SharedTorClient,
    isolation_cache: SharedIsolationCache,
    port: u16,
    clients: &Arc<StdRwLock<Vec<SharedTorClient>>>,
    isolation_caches: &Arc<StdRwLock<Vec<SharedIsolationCache>>>,
    socks_ports: &Arc<StdRwLock<Vec<u16>>>,
    clients_count: &Arc<AtomicUsize>,
) -> ManagedSocksPort {
    {
        let mut global_clients = ACTIVE_TOR_CLIENTS
            .get_or_init(|| StdRwLock::new(Vec::new()))
            .write()
            .unwrap();
        global_clients.push(client_slot.clone());
    }

    {
        let mut clients_guard = clients.write().unwrap();
        clients_guard.push(client_slot.clone());
    }
    {
        let mut caches_guard = isolation_caches.write().unwrap();
        caches_guard.push(isolation_cache.clone());
    }
    {
        let mut ports_guard = socks_ports.write().unwrap();
        ports_guard.push(port);
    }
    clients_count.fetch_add(1, Ordering::Relaxed);

    let registration = ManagedSocksPort {
        owner_id,
        node_idx,
        client_slot,
        isolation_cache,
    };
    register_socks_port(port, registration.clone());
    registration
}

fn register_live_client_slot(
    client_slot: SharedTorClient,
    isolation_cache: SharedIsolationCache,
    clients: &Arc<StdRwLock<Vec<SharedTorClient>>>,
    isolation_caches: &Arc<StdRwLock<Vec<SharedIsolationCache>>>,
    clients_count: &Arc<AtomicUsize>,
) {
    {
        let mut global_clients = ACTIVE_TOR_CLIENTS
            .get_or_init(|| StdRwLock::new(Vec::new()))
            .write()
            .unwrap();
        global_clients.push(client_slot.clone());
    }
    {
        let mut global_caches = ACTIVE_TOR_ISOLATION_CACHES
            .get_or_init(|| StdRwLock::new(Vec::new()))
            .write()
            .unwrap();
        global_caches.push(isolation_cache.clone());
    }
    {
        let mut clients_guard = clients.write().unwrap();
        clients_guard.push(client_slot);
    }
    {
        let mut caches_guard = isolation_caches.write().unwrap();
        caches_guard.push(isolation_cache);
    }
    clients_count.fetch_add(1, Ordering::Relaxed);
}

fn unregister_socks_ports(owner_id: u64, ports_to_remove: &[u16]) {
    if let Ok(mut ports) = socks_registry().write() {
        for port in ports_to_remove {
            let should_remove = ports
                .get(port)
                .map(|registration| registration.owner_id == owner_id)
                .unwrap_or(false);
            if should_remove {
                ports.remove(port);
            }
        }
    }
}

fn lookup_socks_port(port: u16) -> Option<ManagedSocksPort> {
    socks_registry()
        .read()
        .ok()
        .and_then(|ports| ports.get(&port).cloned())
}

pub fn active_socks_ports() -> Vec<u16> {
    let mut ports = socks_registry()
        .read()
        .ok()
        .map(|registrations| registrations.keys().copied().collect::<Vec<_>>())
        .unwrap_or_default();
    ports.sort_unstable();
    ports
}

async fn isolation_token_for(
    isolation_cache: &SharedIsolationCache,
    key: SocksIsolationKey,
) -> IsolationToken {
    let mut cache = isolation_cache.write().await;
    match cache.entry(key) {
        Entry::Occupied(entry) => *entry.get(),
        Entry::Vacant(entry) => {
            let token = IsolationToken::new();
            entry.insert(token);
            token
        }
    }
}

async fn install_client(
    client_slot: &SharedTorClient,
    isolation_cache: &SharedIsolationCache,
    replacement: Arc<TorClient<PreferredRuntime>>,
) {
    {
        let mut slot = client_slot.write().await;
        *slot = replacement;
    }
    isolation_cache.write().await.clear();
}

fn health_probe_target() -> (String, u16) {
    let host = std::env::var("CRAWLI_TOR_HEALTH_PROBE_HOST")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_HEALTH_PROBE_HOST.to_string());
    let port = std::env::var("CRAWLI_TOR_HEALTH_PROBE_PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(DEFAULT_HEALTH_PROBE_PORT);
    (host, port)
}

async fn probe_client_connectivity(
    client_slot: &SharedTorClient,
    probe_host: &str,
    probe_port: u16,
) -> Result<f64> {
    let tor_client = { client_slot.read().await.clone() };
    let mut prefs = arti_client::StreamPrefs::new();
    prefs.connect_to_onion_services(arti_client::config::BoolOrAuto::Explicit(true));

    let started = Instant::now();
    match tokio::time::timeout(
        std::time::Duration::from_secs(HEALTH_PROBE_TIMEOUT_SECS),
        tor_client.connect_with_prefs((probe_host.to_string(), probe_port), &prefs),
    )
    .await
    {
        Ok(Ok(_stream)) => Ok(started.elapsed().as_millis() as f64),
        Ok(Err(err)) => Err(anyhow!("probe connect failed: {}", err)),
        Err(_) => Err(anyhow!(
            "probe timed out after {}s",
            HEALTH_PROBE_TIMEOUT_SECS
        )),
    }
}

fn should_rotate_on_onion_connect_error(error_text: &str) -> bool {
    error_text.contains("failed to obtain hidden service circuit")
        || error_text.contains("hidden service circuit")
        || error_text.contains("un-retried transient failure")
        || error_text.contains("tor operation timed out")
        || error_text.contains("operation timed out")
}

async fn take_phantom_or_bootstrap(
    node_idx: usize,
    phantom_pool: &Arc<RwLock<Vec<Arc<TorClient<PreferredRuntime>>>>>,
    is_vm: bool,
) -> Result<Arc<TorClient<PreferredRuntime>>> {
    let mut pool = phantom_pool.write().await;
    if let Some(phantom) = pool.pop() {
        eprintln!(
            "[Aerospace Healing] Circuit {} swapped with phantom standby (pool: {} remaining).",
            node_idx,
            pool.len()
        );
        return Ok(phantom);
    }
    drop(pool);

    eprintln!(
        "[Aerospace Healing Warning] Phantom pool empty for circuit {}. Re-bootstrapping.",
        node_idx
    );
    let replacement = spawn_tor_node(node_idx, is_vm).await?;
    Ok(Arc::new(replacement))
}

/// Spawns a single arti TorClient with entropy-hardened temporal scatter jitter.
/// Waits for bootstrap to complete before returning — ensures the circuit is ready.
pub async fn spawn_tor_node(node_index: usize, is_vm: bool) -> Result<TorClient<PreferredRuntime>> {
    // ENTROPY-HARDENED TEMPORAL SCATTER:
    // Stagger client creation to avoid thundering herd on directory authorities
    let jitter_ms = {
        let mut rng = rand::thread_rng();
        use rand::Rng;
        let max_jitter = jitter_window_ms(is_vm);
        rng.gen_range(0..max_jitter)
    };
    tokio::time::sleep(tokio::time::Duration::from_millis(jitter_ms)).await;

    let config = build_tor_config(node_index)?;

    // Use create_bootstrapped to ensure the client is fully ready with circuits
    // before returning. This blocks until the Tor consensus is downloaded and
    // at least one circuit is built.
    let client = TorClient::create_bootstrapped(config)
        .await
        .map_err(|e| anyhow!("Failed to bootstrap Arti Node {}: {}", node_index, e))?;

    Ok(client)
}

// ============================================================================
// ARTISWARM: The Native Tor Engine (replaces TorProcessGuard)
// ============================================================================

/// Native Rust Tor swarm — replaces tor.exe process management.
/// Each client has an embedded SOCKS5 proxy for reqwest backward compatibility.
pub struct ArtiSwarm {
    pub clients: Arc<StdRwLock<Vec<SharedTorClient>>>,
    isolation_caches: Arc<StdRwLock<Vec<SharedIsolationCache>>>,
    pub phantom_pool: Arc<RwLock<Vec<Arc<TorClient<PreferredRuntime>>>>>,
    pub socks_ports: Arc<StdRwLock<Vec<u16>>>,
    #[allow(dead_code)]
    circuit_health: SharedCircuitHealth,
    shutdown: Arc<AtomicBool>,
    owner_id: u64,
    #[allow(dead_code)]
    is_vm: bool,
}

impl ArtiSwarm {
    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Relaxed);
    }

    /// Phase 46: Aerospace Grade Intelligent Healing
    /// Explicitly isolate a poisoned circuit and hot-swap it from the Phantom Pool
    pub async fn isolate_circuit(&self, target_idx: usize) -> Result<()> {
        let (client_slot, isolation_cache) = {
            let clients = self.clients.read().unwrap();
            let caches = self.isolation_caches.read().unwrap();
            if target_idx >= clients.len() || target_idx >= caches.len() {
                return Err(anyhow!(
                    "Invalid circuit index for dynamic isolation: {}",
                    target_idx
                ));
            }
            (clients[target_idx].clone(), caches[target_idx].clone())
        };
        let replacement =
            take_phantom_or_bootstrap(target_idx, &self.phantom_pool, self.is_vm).await?;
        install_client(&client_slot, &isolation_cache, replacement).await;
        Ok(())
    }
}

impl Drop for ArtiSwarm {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        let socks_ports = self.socks_ports.read().unwrap().clone();
        unregister_socks_ports(self.owner_id, &socks_ports);

        // Remove our clients from the global pool (using pointer equality for simplicity, or just clear if we assume 1 swarm)
        let current_clients = self.clients.read().unwrap().clone();
        let mut global_clients = ACTIVE_TOR_CLIENTS
            .get_or_init(|| StdRwLock::new(Vec::new()))
            .write()
            .unwrap();
        global_clients.retain(|c| !current_clients.iter().any(|my_c| Arc::ptr_eq(my_c, c)));
        drop(global_clients);

        // TorClient Drop handles cleanup automatically — no PID files, no process kills
        eprintln!(
            "ArtiSwarm dropped: {} native Tor clients released",
            current_clients.len()
        );
    }
}

// ============================================================================
// SOCKS5 PROXY (per-client, for reqwest backward compatibility)
// ============================================================================

/// Lightweight SOCKS5 proxy fronting a single TorClient.
/// Allows reqwest to connect via `socks5h://127.0.0.1:<port>` just like tor.exe.
///
/// This public wrapper is for standalone tests/examples that do not need live hot-swapping.
pub async fn run_socks_proxy(
    tor_client: Arc<TorClient<PreferredRuntime>>,
    port: u16,
    shutdown: Arc<AtomicBool>,
    node_idx: usize,
) -> Result<()> {
    let client_slot = Arc::new(RwLock::new(tor_client));
    let isolation_cache = Arc::new(RwLock::new(HashMap::new()));
    run_managed_socks_proxy(client_slot, isolation_cache, port, shutdown, node_idx).await
}

async fn run_managed_socks_proxy(
    client_slot: SharedTorClient,
    isolation_cache: SharedIsolationCache,
    port: u16,
    shutdown: Arc<AtomicBool>,
    node_idx: usize,
) -> Result<()> {
    // TCP TIME_WAIT RECYCLING: Bind with SO_REUSEADDR
    let socket = socket2::Socket::new(
        socket2::Domain::IPV4,
        socket2::Type::STREAM,
        Some(socket2::Protocol::TCP),
    )?;
    socket.set_reuse_address(true)?;
    socket.set_nonblocking(true)?;
    socket.bind(
        &format!("127.0.0.1:{}", port)
            .parse::<std::net::SocketAddr>()?
            .into(),
    )?;
    socket.listen(512)?;

    let std_listener: std::net::TcpListener = socket.into();
    let listener = TcpListener::from_std(std_listener)?;
    eprintln!(
        "SOCKS5 proxy for Arti Node {} bound to 127.0.0.1:{}",
        node_idx, port
    );

    loop {
        if shutdown.load(Ordering::Relaxed) {
            break;
        }

        let accept_result =
            tokio::time::timeout(std::time::Duration::from_secs(2), listener.accept()).await;

        let (stream, _) = match accept_result {
            Ok(Ok((s, a))) => (s, a),
            Ok(Err(e)) => {
                eprintln!("SOCKS accept error on port {}: {}", port, e);
                continue;
            }
            Err(_) => continue, // Timeout — check shutdown flag
        };

        let client_slot = Arc::clone(&client_slot);
        let isolation_cache = Arc::clone(&isolation_cache);
        let idx = node_idx;
        tokio::spawn(async move {
            if let Err(e) = handle_socks_connection(stream, client_slot, isolation_cache, idx).await
            {
                eprintln!("SOCKS conn error on node {}: {}", idx, e);
            }
        });
    }

    Ok(())
}

/// Full SOCKS5 handshake → arti TorClient::connect_with_prefs → bidirectional relay
async fn handle_socks_connection(
    mut stream: tokio::net::TcpStream,
    client_slot: SharedTorClient,
    isolation_cache: SharedIsolationCache,
    proxy_idx: usize,
) -> Result<()> {
    use bytes::BytesMut;
    let mut buf = BytesMut::with_capacity(1024);
    let mut username = Vec::new();
    let mut password = Vec::new();

    // 1. SOCKS5 Handshake
    buf.resize(2, 0);
    stream.read_exact(&mut buf[0..2]).await?;
    if buf[0] != 0x05 {
        return Err(anyhow!("Only SOCKS5 is supported"));
    }
    let n_methods = buf[1] as usize;
    buf.resize(n_methods, 0);
    stream.read_exact(&mut buf[0..n_methods]).await?;

    let mut use_auth = false;
    if buf[0..n_methods].contains(&0x02) {
        use_auth = true;
        stream.write_all(&[0x05, 0x02]).await?; // USER/PASS AUTH
    } else {
        stream.write_all(&[0x05, 0x00]).await?; // NO AUTH
    }

    if use_auth {
        // Read Auth Version and ULEN
        buf.resize(2, 0);
        stream.read_exact(&mut buf[..2]).await?;
        if buf[0] != 0x01 {
            return Err(anyhow!("Invalid SOCKS5 Auth Version"));
        }
        let ulen = buf[1] as usize;
        // Read Username
        if ulen > 0 {
            buf.resize(ulen, 0);
            stream.read_exact(&mut buf[..ulen]).await?;
            username = buf[..ulen].to_vec();
        }
        // Read PLEN
        buf.resize(1, 0);
        stream.read_exact(&mut buf[..1]).await?;
        let plen = buf[0] as usize;
        // Read Password
        if plen > 0 {
            buf.resize(plen, 0);
            stream.read_exact(&mut buf[..plen]).await?;
            password = buf[..plen].to_vec();
        }
        // Respond Auth Success
        stream.write_all(&[0x01, 0x00]).await?;
    }

    // 2. Request Phase
    buf.resize(4, 0);
    stream.read_exact(&mut buf[0..4]).await?;
    if buf[0] != 0x05 || buf[1] != 0x01 {
        return Err(anyhow!("Only CONNECT commands supported"));
    }

    let atyp = buf[3];
    let target_addr = match atyp {
        0x01 => {
            // IPv4
            buf.resize(4, 0);
            stream.read_exact(&mut buf[0..4]).await?;
            format!("{}.{}.{}.{}", buf[0], buf[1], buf[2], buf[3])
        }
        0x03 => {
            // Domain
            buf.resize(1, 0);
            stream.read_exact(&mut buf[0..1]).await?;
            let len = buf[0] as usize;
            buf.resize(len, 0);
            stream.read_exact(&mut buf[0..len]).await?;
            String::from_utf8_lossy(&buf[0..len]).into_owned()
        }
        0x04 => {
            // IPv6
            buf.resize(16, 0);
            stream.read_exact(&mut buf[0..16]).await?;
            let a = u16::from_be_bytes([buf[0], buf[1]]);
            let b = u16::from_be_bytes([buf[2], buf[3]]);
            let c = u16::from_be_bytes([buf[4], buf[5]]);
            let d = u16::from_be_bytes([buf[6], buf[7]]);
            let e = u16::from_be_bytes([buf[8], buf[9]]);
            let f = u16::from_be_bytes([buf[10], buf[11]]);
            let g = u16::from_be_bytes([buf[12], buf[13]]);
            let h = u16::from_be_bytes([buf[14], buf[15]]);
            format!(
                "{:x}:{:x}:{:x}:{:x}:{:x}:{:x}:{:x}:{:x}",
                a, b, c, d, e, f, g, h
            )
        }
        _ => return Err(anyhow!("Unknown SOCKS5 address type")),
    };

    buf.resize(2, 0);
    stream.read_exact(&mut buf[0..2]).await?;
    let port = u16::from_be_bytes([buf[0], buf[1]]);

    let target_addr = target_addr
        .trim_matches(char::from(0))
        .trim()
        .to_lowercase();
    let is_onion = target_addr.ends_with(".onion");

    // OPSEC FIREWALL: Block port 80 clearnet (allows for .onion E2E encrypted)
    if port == 80 && !is_onion {
        eprintln!(
            "OPSEC FIREWALL: Blocked clearnet port 80 request to {}",
            target_addr
        );
        return Err(anyhow!(
            "Port 80 blocked to prevent exit node SSL-stripping."
        ));
    }

    // 3. Connect via arti with onion service support + retry
    let isolation_key = use_auth.then_some(SocksIsolationKey { username, password });
    let isolation_token = match isolation_key {
        Some(key) => Some(isolation_token_for(&isolation_cache, key).await),
        None => None,
    };

    let max_retries = if is_onion { 3u32 } else { 1 };
    let mut tor_stream = None;
    for attempt in 1..=max_retries {
        let mut prefs = arti_client::StreamPrefs::new();
        prefs.connect_to_onion_services(arti_client::config::BoolOrAuto::Explicit(true));
        if let Some(token) = isolation_token {
            prefs.set_isolation(token);
        }

        let tor_client = client_slot.read().await.clone();
        match tor_client
            .connect_with_prefs((target_addr.clone(), port), &prefs)
            .await
        {
            Ok(s) => {
                tor_stream = Some(s);
                break;
            }
            Err(e) => {
                let error_text = e.to_string().to_lowercase();
                eprintln!(
                    "[Node {}] Arti connect attempt {}/{} to {}:{} failed: {}",
                    proxy_idx, attempt, max_retries, target_addr, port, e
                );
                if is_onion
                    && attempt < max_retries
                    && should_rotate_on_onion_connect_error(&error_text)
                {
                    let replacement = {
                        let current = client_slot.read().await.clone();
                        Arc::new(current.isolated_client())
                    };
                    install_client(&client_slot, &isolation_cache, replacement).await;
                    eprintln!(
                        "[Node {}] Rotated managed client slot after hidden-service circuit failure.",
                        proxy_idx
                    );
                }
                if attempt < max_retries {
                    tokio::time::sleep(tokio::time::Duration::from_millis(1500 * attempt as u64))
                        .await;
                } else {
                    return Err(anyhow!(
                        "Arti connect failed after {} retries: {}",
                        max_retries,
                        e
                    ));
                }
            }
        }
    }
    let mut tor_stream = tor_stream.unwrap();

    // 4. Success reply
    stream
        .write_all(&[0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
        .await?;

    // 5. Bidirectional TCP pump
    let _ = tokio::io::copy_bidirectional(&mut stream, &mut tor_stream).await;
    Ok(())
}

// ============================================================================
// LIVE CIRCUIT PROBING (Kalman-smoothed anomaly detection)
// ============================================================================

fn spawn_health_monitor(
    clients: Arc<StdRwLock<Vec<SharedTorClient>>>,
    isolation_caches: Arc<StdRwLock<Vec<SharedIsolationCache>>>,
    health_map: SharedCircuitHealth,
    phantom_pool: Arc<RwLock<Vec<Arc<TorClient<PreferredRuntime>>>>>,
    probe_host: String,
    probe_port: u16,
    is_vm: bool,
    shutdown: Arc<AtomicBool>,
) {
    let drift_multiplier = if is_vm { 4.0 } else { 2.5 };
    let probe_interval_secs = health_probe_interval_secs();
    let anomaly_threshold = max_consecutive_probe_anomalies(is_vm);

    tokio::spawn(async move {
        loop {
            if shutdown.load(Ordering::Relaxed) {
                break;
            }
            tokio::time::sleep(tokio::time::Duration::from_secs(probe_interval_secs)).await;
            if shutdown.load(Ordering::Relaxed) {
                break;
            }

            let current_clients = clients.read().unwrap().clone();
            let current_caches = isolation_caches.read().unwrap().clone();
            let registrations: Vec<(usize, SharedTorClient, SharedIsolationCache)> =
                current_clients
                    .into_iter()
                    .zip(current_caches.into_iter())
                    .enumerate()
                    .map(|(idx, (client_slot, isolation_cache))| {
                        (idx, client_slot, isolation_cache)
                    })
                    .collect();

            let samples = futures::future::join_all(registrations.iter().map(|registration| {
                let (idx, client_slot, isolation_cache) = registration.clone();
                let probe_host = probe_host.clone();
                async move {
                    let sample =
                        probe_client_connectivity(&client_slot, &probe_host, probe_port).await;
                    ((idx, client_slot, isolation_cache), sample)
                }
            }))
            .await;

            let mut degraded = Vec::new();
            let mut health = health_map.write().await;

            for ((idx, client_slot, isolation_cache), sample) in samples {
                let circuit = health.entry(idx).or_insert_with(CircuitHealth::new);
                match sample {
                    Ok(rtt_ms) => {
                        let baseline_ms = circuit.latency_kalman.predict();
                        let predicted_ms = circuit.latency_kalman.update(rtt_ms);
                        let is_slow = baseline_ms > 0.0 && rtt_ms > baseline_ms * drift_multiplier;

                        if is_slow {
                            circuit.anomaly_streak = circuit.anomaly_streak.saturating_add(1);
                            eprintln!(
                                "[Tor Probe] Circuit {} slow: {:.0} ms (baseline: {:.0} ms, predicted: {:.0} ms, streak: {})",
                                idx,
                                rtt_ms,
                                baseline_ms,
                                predicted_ms,
                                circuit.anomaly_streak
                            );
                        } else {
                            circuit.anomaly_streak = 0;
                        }
                    }
                    Err(err) => {
                        circuit.anomaly_streak = circuit.anomaly_streak.saturating_add(1);
                        eprintln!(
                            "[Tor Probe] Circuit {} probe failed (streak: {}): {}",
                            idx, circuit.anomaly_streak, err
                        );
                    }
                }

                if circuit.anomaly_streak >= anomaly_threshold {
                    eprintln!(
                        "[Tor Probe] Circuit {} marked degraded after {} consecutive probe anomalies.",
                        idx,
                        circuit.anomaly_streak
                    );
                    circuit.anomaly_streak = 0;
                    degraded.push((idx, client_slot, isolation_cache));
                }
            }

            drop(health);

            for (idx, client_slot, isolation_cache) in degraded {
                match take_phantom_or_bootstrap(idx, &phantom_pool, is_vm).await {
                    Ok(replacement) => {
                        install_client(&client_slot, &isolation_cache, replacement).await;
                    }
                    Err(err) => {
                        eprintln!(
                            "[Aerospace Healing Warning] Failed to replenish circuit {}: {}",
                            idx, err
                        );
                    }
                }
            }
        }
    });
}

// ============================================================================
// PHANTOM CIRCUIT POOL (warm standby + auto-replenishment)
// ============================================================================

fn spawn_phantom_pool_builder(
    pool_size: usize,
    phantom_pool: Arc<RwLock<Vec<Arc<TorClient<PreferredRuntime>>>>>,
    is_vm: bool,
    shutdown: Arc<AtomicBool>,
) {
    tokio::spawn(async move {
        // Wait for main swarm to bootstrap first
        tokio::time::sleep(tokio::time::Duration::from_secs(
            phantom_pool_bootstrap_delay_secs(),
        ))
        .await;
        if shutdown.load(Ordering::Relaxed) {
            return;
        }

        eprintln!(
            "Phantom Pool: Building {} warm standby circuits...",
            pool_size
        );
        let mut phantom_futures = Vec::new();
        for i in 0..pool_size {
            let vm = is_vm;
            phantom_futures.push(async move { spawn_tor_node(200 + i, vm).await });
        }

        let results = futures::future::join_all(phantom_futures).await;
        let mut phantoms = Vec::new();
        for c in results.into_iter().flatten() {
            phantoms.push(Arc::new(c));
        }

        let count = phantoms.len();
        {
            let mut pool = phantom_pool.write().await;
            pool.extend(phantoms);
        }
        eprintln!("Phantom Pool: {} warm standby circuits READY", count);

        // AUTO-REPLENISHMENT LOOP
        let mut next_phantom_idx = 200 + pool_size;
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(
                phantom_pool_replenish_interval_secs(),
            ))
            .await;
            if shutdown.load(Ordering::Relaxed) {
                break;
            }

            let current_len = phantom_pool.read().await.len();
            let deficit = pool_size.saturating_sub(current_len);
            if deficit > 0 {
                eprintln!(
                    "Phantom Auto-Replenish: Building {} replacements...",
                    deficit
                );
                let mut futures_vec = Vec::new();
                for _ in 0..deficit {
                    let vm = is_vm;
                    let idx = next_phantom_idx;
                    next_phantom_idx += 1;
                    futures_vec.push(async move { spawn_tor_node(idx, vm).await });
                }
                let results = futures::future::join_all(futures_vec).await;
                let mut new_phantoms = Vec::new();
                for c in results.into_iter().flatten() {
                    new_phantoms.push(Arc::new(c));
                }
                if !new_phantoms.is_empty() {
                    let mut pool = phantom_pool.write().await;
                    let count = new_phantoms.len();
                    pool.extend(new_phantoms);
                    eprintln!(
                        "Phantom Auto-Replenish: {} circuits rebuilt (pool: {})",
                        count,
                        pool.len()
                    );
                }
            }
        }
    });
}

// ============================================================================
// MEMORY PRESSURE MONITOR (OOM prevention)
// ============================================================================

fn spawn_memory_pressure_monitor(
    clients_count: Arc<AtomicUsize>,
    phantom_pool: Arc<RwLock<Vec<Arc<TorClient<PreferredRuntime>>>>>,
    shutdown: Arc<AtomicBool>,
) {
    tokio::spawn(async move {
        tokio::time::sleep(tokio::time::Duration::from_secs(
            memory_monitor_warmup_delay_secs(),
        ))
        .await;

        let total_memory = {
            use sysinfo::System;
            let sys = System::new_all();
            sys.total_memory()
        };
        let threshold_pct = 0.80;

        loop {
            if shutdown.load(Ordering::Relaxed) {
                break;
            }
            tokio::time::sleep(tokio::time::Duration::from_secs(
                memory_monitor_interval_secs(),
            ))
            .await;
            if shutdown.load(Ordering::Relaxed) {
                break;
            }

            let current_rss = {
                use sysinfo::{Pid, System};
                let mut sys = System::new();
                let pid = Pid::from(std::process::id() as usize);
                sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
                sys.process(pid).map(|p| p.memory()).unwrap_or(0)
            };

            let usage_pct = current_rss as f64 / total_memory as f64;
            let rss_mb = current_rss / (1024 * 1024);

            if usage_pct > threshold_pct {
                let reclaimed = {
                    let mut pool = phantom_pool.write().await;
                    let count = pool.len();
                    pool.clear();
                    count
                };
                eprintln!(
                    "⚠ MEMORY PRESSURE: RSS {} MB ({:.1}%). Active circuits: {}. Phantom pool shed: {}",
                    rss_mb,
                    usage_pct * 100.0,
                    clients_count.load(Ordering::Relaxed),
                    reclaimed,
                );
            } else {
                eprintln!("Memory OK: RSS {} MB ({:.1}%)", rss_mb, usage_pct * 100.0);
            }
        }
    });
}

// ============================================================================
// PUBLIC API: bootstrap_arti_cluster (replaces bootstrap_tor_cluster)
// ============================================================================

/// Bootstraps N native arti TorClient instances, each fronted by a SOCKS5 proxy.
/// Returns (ArtiSwarm, Vec<u16>) where Vec<u16> contains the SOCKS ports —
/// identical API shape to the old tor.exe-based bootstrap for backward compatibility.
pub async fn bootstrap_arti_cluster(
    app: AppHandle,
    daemon_count: usize,
) -> Result<(ArtiSwarm, Vec<u16>)> {
    let requested = daemon_count.max(1);
    let governor_profile = crate::resource_governor::detect_profile(None);
    let target = 1;
    let min_ready = 1;
    let is_vm = detect_vm_environment();
    let (probe_host, probe_port) = health_probe_target();
    let shutdown = Arc::new(AtomicBool::new(false));
    let owner_id = NEXT_SWARM_ID.fetch_add(1, Ordering::Relaxed);
    let started_at = Instant::now();

    let _ = app.emit(
        "tor_status",
        crate::tor::TorStatusEvent {
            state: "starting".to_string(),
            message: format!(
                "Bootstrapping {} native arti Tor circuits via {} runtime (ready quorum {}/{})...",
                target,
                runtime_label(),
                min_ready,
                requested,
            ),
            daemon_count: target,
            ports: vec![],
        },
    );

    let _ = app.emit(
        "crawl_log",
        format!(
            "[TOR] Phase 43B: Native arti engine ({}) . target={} quorum={} requested={}. Health probe: {}:{}. VM mode: {}",
            runtime_label(),
            target,
            min_ready,
            requested,
            probe_host,
            probe_port,
            is_vm
        ),
    );
    let _ = app.emit(
        "crawl_log",
        format!(
            "[TOR] Resource Governor: cpu={} total_gib={} avail_gib={} storage={} recommended_cap={} direct_io={}",
            governor_profile.cpu_cores,
            governor_profile.total_memory_bytes / (1024 * 1024 * 1024),
            governor_profile.available_memory_bytes / (1024 * 1024 * 1024),
            crate::resource_governor::storage_class_label(governor_profile.storage_class),
            governor_profile.recommended_arti_cap,
            crate::io_vanguard::direct_io_policy_label()
        ),
    );

    let clients_arc = Arc::new(StdRwLock::new(Vec::new()));
    let isolation_caches_arc = Arc::new(StdRwLock::new(Vec::new()));
    let socks_ports_arc = Arc::new(StdRwLock::new(Vec::new()));
    let clients_count = Arc::new(AtomicUsize::new(0));

    let mut client_futures = tokio::task::JoinSet::new();
    for i in 0..target {
        let vm = is_vm;
        client_futures.spawn(async move { (i, spawn_tor_node(i, vm).await) });
    }

    while clients_count.load(Ordering::Relaxed) < min_ready {
        match client_futures.join_next().await {
            Some(Ok((idx, Ok(client)))) => {
                let shared_client = Arc::new(RwLock::new(Arc::new(client)));
                let isolation_cache = Arc::new(RwLock::new(HashMap::new()));
                register_live_client_slot(
                    shared_client.clone(),
                    isolation_cache.clone(),
                    &clients_arc,
                    &isolation_caches_arc,
                    &clients_count,
                );
                let _ = idx;
            }
            Some(Ok((_idx, Err(e)))) => eprintln!("Arti node failed to create: {}", e),
            Some(Err(e)) => eprintln!("Arti join task failed: {}", e),
            None => break,
        }
    }

    if clients_count.load(Ordering::Relaxed) == 0 {
        return Err(anyhow!("Failed to create any arti Tor clients"));
    }

    let boot_elapsed = started_at.elapsed();
    let _ = app.emit(
        "crawl_log",
        format!(
            "[TOR] {} / {} arti nodes ready in {:.1}s via {} runtime",
            clients_count.load(Ordering::Relaxed),
            target,
            boot_elapsed.as_secs_f64(),
            runtime_label()
        ),
    );

    let ready_ports = socks_ports_arc.read().unwrap().clone();

    let _ = app.emit(
        "tor_status",
        crate::tor::TorStatusEvent {
            state: "ready".to_string(),
            message: format!(
                "{} native arti client(s) ready on {:?}",
                clients_count.load(Ordering::Relaxed),
                ready_ports
            ),
            daemon_count: clients_count.load(Ordering::Relaxed),
            ports: ready_ports.clone(),
        },
    );

    let _ = app.emit(
        "crawl_log",
        format!(
            "[TOR] ✓ Native Tor ready: {} client(s) on {:?} (runtime={} no tor.exe!)",
            clients_count.load(Ordering::Relaxed),
            ready_ports,
            runtime_label()
        ),
    );

    // Health monitoring, phantom pool, memory monitor
    let circuit_health = Arc::new(RwLock::new(HashMap::new()));
    let phantom_pool = Arc::new(RwLock::new(Vec::new()));

    spawn_health_monitor(
        clients_arc.clone(),
        isolation_caches_arc.clone(),
        circuit_health.clone(),
        phantom_pool.clone(),
        probe_host,
        probe_port,
        is_vm,
        shutdown.clone(),
    );

    let phantom_size = std::cmp::max(2, clients_count.load(Ordering::Relaxed).max(1) / 3);
    spawn_phantom_pool_builder(phantom_size, phantom_pool.clone(), is_vm, shutdown.clone());

    spawn_memory_pressure_monitor(
        clients_count.clone(),
        phantom_pool.clone(),
        shutdown.clone(),
    );

    if target > min_ready {
        let app_bg = app.clone();
        let clients_arc_bg = clients_arc.clone();
        let isolation_caches_bg = isolation_caches_arc.clone();
        let clients_count_bg = clients_count.clone();
        tokio::spawn(async move {
            while let Some(result) = client_futures.join_next().await {
                match result {
                    Ok((_idx, Ok(client))) => {
                        let shared_client = Arc::new(RwLock::new(Arc::new(client)));
                        let isolation_cache = Arc::new(RwLock::new(HashMap::new()));
                        register_live_client_slot(
                            shared_client,
                            isolation_cache,
                            &clients_arc_bg,
                            &isolation_caches_bg,
                            &clients_count_bg,
                        );

                        let ready_now = clients_count_bg.load(Ordering::Relaxed);
                        let _ = app_bg.emit(
                            "crawl_log",
                            format!(
                                "[TOR] Background bootstrap expanded client pool to {} / {}",
                                ready_now, target
                            ),
                        );
                        let _ = app_bg.emit(
                            "tor_status",
                            crate::tor::TorStatusEvent {
                                state: "ready".to_string(),
                                message: format!(
                                    "Background bootstrap expanded client pool to {} / {}",
                                    ready_now, target
                                ),
                                daemon_count: ready_now,
                                ports: vec![],
                            },
                        );
                    }
                    Ok((_idx, Err(err))) => {
                        eprintln!("Background arti node failed to create: {}", err)
                    }
                    Err(err) => eprintln!("Background arti join task failed: {}", err),
                }
            }
        });
    }

    let swarm = ArtiSwarm {
        clients: clients_arc,
        isolation_caches: isolation_caches_arc,
        phantom_pool,
        socks_ports: socks_ports_arc,
        circuit_health,
        shutdown,
        owner_id,
        is_vm,
    };

    Ok((swarm, ready_ports))
}

/// Allocate a SOCKS port — try preferred, fallback to OS ephemeral
#[allow(dead_code)]
fn allocate_socks_port(preferred: u16) -> Result<u16> {
    // Reserved ports for Tor Browser
    const RESERVED: &[u16] = &[9150, 9151];
    if !RESERVED.contains(&preferred) {
        if let Ok(listener) = std::net::TcpListener::bind(format!("127.0.0.1:{}", preferred)) {
            if let Ok(addr) = listener.local_addr() {
                return Ok(addr.port());
            }
        }
    }
    // Fallback to OS ephemeral
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    Ok(listener.local_addr()?.port())
}

/// Detect if we're running inside a VM (extend jitter for clock coarsening)
fn detect_vm_environment() -> bool {
    // Check common VM indicators
    if let Ok(content) = std::fs::read_to_string("/sys/hypervisor/uuid") {
        return !content.trim().is_empty();
    }
    if let Ok(content) = std::fs::read_to_string("/proc/cpuinfo") {
        let lower = content.to_lowercase();
        return lower.contains("hypervisor") || lower.contains("vmware") || lower.contains("kvm");
    }
    // macOS: assume native unless in Docker
    std::env::var("container").is_ok()
}

/// NEWNYM equivalent for arti — rotate the live managed client behind a SOCKS port.
///
/// This is cheap compared to a full client bootstrap: it creates a fresh isolated
/// TorClient handle, swaps it into the live port slot, and clears cached auth tokens
/// so future requests establish new isolation groups.
pub async fn request_newnym_arti(socks_port: u16) -> Result<()> {
    let registration = lookup_socks_port(socks_port).ok_or_else(|| {
        anyhow!(
            "No managed Arti SOCKS proxy is registered on port {}",
            socks_port
        )
    })?;
    let replacement = {
        let client = registration.client_slot.read().await.clone();
        Arc::new(client.isolated_client())
    };
    install_client(
        &registration.client_slot,
        &registration.isolation_cache,
        replacement,
    )
    .await;
    eprintln!(
        "[Arti NEWNYM] Rotated managed circuit for SOCKS port {}",
        socks_port
    );
    Ok(())
}

pub async fn request_newnym_slot_arti(slot_idx: usize) -> Result<()> {
    let client_slot = ACTIVE_TOR_CLIENTS
        .get_or_init(|| StdRwLock::new(Vec::new()))
        .read()
        .unwrap()
        .get(slot_idx)
        .cloned()
        .ok_or_else(|| anyhow!("No managed Arti client slot at index {}", slot_idx))?;
    let isolation_cache = ACTIVE_TOR_ISOLATION_CACHES
        .get_or_init(|| StdRwLock::new(Vec::new()))
        .read()
        .unwrap()
        .get(slot_idx)
        .cloned()
        .unwrap_or_else(|| Arc::new(RwLock::new(HashMap::new())));
    let replacement = {
        let client = client_slot.read().await.clone();
        Arc::new(client.isolated_client())
    };
    install_client(&client_slot, &isolation_cache, replacement).await;
    eprintln!("[Arti NEWNYM] Rotated managed circuit slot {}", slot_idx);
    Ok(())
}
