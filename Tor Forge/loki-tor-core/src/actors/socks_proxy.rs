use anyhow::Result;
use arti_client::TorClient;
use tor_rtcompat::PreferredRuntime;
use ractor::{Actor, ActorProcessingErr, ActorRef};
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::RwLock;
use bytes::BytesMut;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;
use std::collections::HashMap;
use crate::telemetry::kalman::CircuitKalmanFilter;

// The message enum for SocksProxy
pub enum SocksProxyMsg {
    Shutdown,
    UpdateClients(Vec<Arc<TorClient<PreferredRuntime>>>),
}

pub struct SocksProxy;

/// Per-circuit telemetry for the Bandwidth-Weighted HFT Load Balancer
pub struct CircuitTelemetry {
    pub latency_kalman: CircuitKalmanFilter,
    pub throughput_kalman: CircuitKalmanFilter,
    pub total_bytes: u64,
    pub total_connections: u64,
    pub last_rtt_ms: f64,
    pub last_throughput_kbps: f64, // KB/s
}

impl Default for CircuitTelemetry {
    fn default() -> Self {
        Self {
            latency_kalman: CircuitKalmanFilter::new(200.0, 1.0, 100.0),
            throughput_kalman: CircuitKalmanFilter::new(100.0, 5.0, 200.0), // Start at 100 KB/s estimate
            total_bytes: 0,
            total_connections: 0,
            last_rtt_ms: 0.0,
            last_throughput_kbps: 0.0,
        }
    }
}

pub struct SocksProxyState {
    pub tor_clients: Arc<RwLock<Vec<Arc<TorClient<PreferredRuntime>>>>>,
    pub is_vm: bool,
    pub circuit_telemetry: Arc<RwLock<HashMap<usize, CircuitTelemetry>>>,
}

#[async_trait::async_trait]
impl Actor for SocksProxy {
    type Msg = SocksProxyMsg;
    type State = SocksProxyState;
    type Arguments = (Vec<Arc<TorClient<PreferredRuntime>>>, u16, bool); // (clients, port, is_vm)

    async fn pre_start(
        &self,
        _myself: ActorRef<Self::Msg>,
        args: Self::Arguments,
    ) -> Result<Self::State, ActorProcessingErr> {
        let (tor_clients, port, is_vm) = args;
        tracing::info!("SocksProxy (HFT Load Balancer) Actor starting on 127.0.0.1:{} with {} active backend proxies (vm_mode: {}).", port, tor_clients.len(), is_vm);
        
        let tor_clients_arc = Arc::new(RwLock::new(tor_clients));
        let circuit_telemetry = Arc::new(RwLock::new(HashMap::new()));
        let clients_clone = tor_clients_arc.clone();
        let telemetry_clone = circuit_telemetry.clone();
        let counter = Arc::new(AtomicUsize::new(0));
        let vm_mode = is_vm;
        
        // Bind listener with SO_REUSEADDR for TIME_WAIT recycling
        tokio::spawn(async move {
            if let Err(e) = run_listener(clients_clone, port, counter, telemetry_clone, vm_mode).await {
                tracing::error!("SOCKS listener crashed: {}", e);
            }
        });

        Ok(SocksProxyState {
            tor_clients: tor_clients_arc,
            is_vm,
            circuit_telemetry,
        })
    }

    async fn handle(
        &self,
        _myself: ActorRef<Self::Msg>,
        msg: Self::Msg,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        match msg {
            SocksProxyMsg::Shutdown => {
                tracing::info!("SocksProxy shutting down...");
            }
            SocksProxyMsg::UpdateClients(new_clients) => {
                let mut lock = state.tor_clients.write().await;
                *lock = new_clients;
                tracing::info!("SocksProxy HFT LB updated with {} active Tor client proxies.", lock.len());
            }
        }
        Ok(())
    }
}

/// BANDWIDTH-WEIGHTED HFT LOAD BALANCER: Select the mathematically optimal circuit
/// Uses UCB1-like scoring weighted by actual throughput (KB/s) rather than just RTT.
/// Formula: Score = Throughput_Kalman + C * sqrt(ln(Total) / Circuit_Ops)
fn select_best_circuit(
    num_clients: usize,
    telemetry: &HashMap<usize, CircuitTelemetry>,
    total_ops: u64,
    fallback_counter: &AtomicUsize,
) -> usize {
    if telemetry.is_empty() || total_ops < 10 {
        // Not enough data yet — fall back to round-robin
        return fallback_counter.fetch_add(1, Ordering::Relaxed) % num_clients;
    }

    let exploration_factor = 2.0_f64;
    let ln_total = (total_ops as f64).ln();
    let mut best_idx = 0;
    let mut best_score = f64::NEG_INFINITY;

    for idx in 0..num_clients {
        let score = if let Some(t) = telemetry.get(&idx) {
            if t.total_connections == 0 {
                f64::INFINITY // Unexplored — always prioritize
            } else {
                // BANDWIDTH-WEIGHTED SCORING:
                // Primary signal: Kalman-smoothed throughput (KB/s) — higher is better
                // Secondary signal: Inverse Kalman-smoothed latency — lower RTT is better
                // Combined with UCB1 exploration bonus for underused circuits
                let throughput_score = t.last_throughput_kbps;
                let latency_bonus = 1000.0 / t.last_rtt_ms.max(1.0); // Inverse latency, capped
                let exploration = exploration_factor * (ln_total / t.total_connections as f64).sqrt();
                
                // 70% throughput weight, 30% latency weight (bulk transfer optimization)
                (0.7 * throughput_score) + (0.3 * latency_bonus) + exploration
            }
        } else {
            f64::INFINITY // Never used — must explore
        };

        if score > best_score {
            best_score = score;
            best_idx = idx;
        }
    }

    best_idx
}

async fn run_listener(
    tor_clients: Arc<RwLock<Vec<Arc<TorClient<PreferredRuntime>>>>>,
    port: u16,
    counter: Arc<AtomicUsize>,
    circuit_telemetry: Arc<RwLock<HashMap<usize, CircuitTelemetry>>>,
    is_vm: bool,
) -> Result<()> {
    // TCP TIME_WAIT RECYCLING: Bind with SO_REUSEADDR
    let socket = socket2::Socket::new(
        socket2::Domain::IPV4,
        socket2::Type::STREAM,
        Some(socket2::Protocol::TCP),
    )?;
    socket.set_reuse_address(true)?;
    #[cfg(all(unix, not(target_os = "macos")))]
    socket.set_reuse_port(true)?;
    socket.set_nonblocking(true)?;
    socket.bind(&format!("127.0.0.1:{}", port).parse::<std::net::SocketAddr>()?.into())?;
    socket.listen(1024)?;
    
    let std_listener: std::net::TcpListener = socket.into();
    let listener = TcpListener::from_std(std_listener)?;
    tracing::info!("SOCKS5 HFT Load Balancer bound to 127.0.0.1:{} with SO_REUSEADDR + SO_REUSEPORT", port);
    
    loop {
        let (stream, peer_addr) = listener.accept().await?;
        tracing::debug!("Accepted SOCKS connection from {}", peer_addr);
        
        let clients_read = tor_clients.read().await;
        if clients_read.is_empty() {
             tracing::warn!("Rejecting SOCKS connection from {} - Swarm not fully bootstrapped yet", peer_addr);
             continue;
        }

        // PREDICTIVE HFT DISPATCH: Use Kalman-scored circuit selection
        let telemetry_read = circuit_telemetry.read().await;
        let total_ops: u64 = telemetry_read.values().map(|t| t.total_connections).sum();
        let idx = select_best_circuit(clients_read.len(), &telemetry_read, total_ops, &counter);
        drop(telemetry_read);
        
        let client = Arc::clone(&clients_read[idx]);
        drop(clients_read);
        
        let telemetry_clone = circuit_telemetry.clone();
        let vm_mode = is_vm;
        tokio::spawn(async move {
            let start = Instant::now();
            let result = handle_socks_connection(stream, client, idx).await;
            let elapsed_ms = start.elapsed().as_millis() as f64;
            
            // VM-AWARE CLOCK ANOMALY FILTER
            let should_update = if vm_mode && elapsed_ms > 30_000.0 {
                tracing::debug!("VM clock anomaly detected for circuit {} (elapsed: {:.0}ms). Discarding.", idx, elapsed_ms);
                false
            } else {
                true
            };
            
            if should_update {
                // Extract bytes transferred from the result
                let bytes_transferred = match &result {
                    Ok(bytes) => *bytes,
                    Err(_) => 0,
                };
                
                let mut telemetry = telemetry_clone.write().await;
                let entry = telemetry.entry(idx).or_insert_with(CircuitTelemetry::default);
                entry.total_connections += 1;
                entry.total_bytes += bytes_transferred;
                entry.last_rtt_ms = entry.latency_kalman.update(elapsed_ms);
                
                // BANDWIDTH-WEIGHTED: Calculate actual throughput in KB/s
                if elapsed_ms > 0.0 && bytes_transferred > 0 {
                    let throughput_kbps = (bytes_transferred as f64 / 1024.0) / (elapsed_ms / 1000.0);
                    entry.last_throughput_kbps = entry.throughput_kalman.update(throughput_kbps);
                }
            }
            
            if let Err(e) = result {
                tracing::warn!("SOCKS connection failed on backend proxy {}: {}", idx, e);
            }
        });
    }
}

async fn handle_socks_connection(
    mut stream: tokio::net::TcpStream,
    tor_client: Arc<TorClient<PreferredRuntime>>,
    proxy_idx: usize
) -> Result<u64> {
    let mut buf = BytesMut::with_capacity(1024);

    // 1. Handshake Phase
    buf.resize(2, 0);
    stream.read_exact(&mut buf[0..2]).await?;
    if buf[0] != 0x05 { 
        return Err(anyhow::anyhow!("Only SOCKS5 is supported")); 
    }
    let n_methods = buf[1] as usize;
    buf.resize(n_methods, 0);
    stream.read_exact(&mut buf[0..n_methods]).await?;

    // Reply NO AUTH required
    stream.write_all(&[0x05, 0x00]).await?;

    // 2. Request Phase
    buf.resize(4, 0);
    stream.read_exact(&mut buf[0..4]).await?;
    if buf[0] != 0x05 || buf[1] != 0x01 { 
        return Err(anyhow::anyhow!("Only CONNECT commands are supported")); 
    }

    let atyp = buf[3];
    let target_addr = match atyp {
        0x01 => { // IPv4
            buf.resize(4, 0);
            stream.read_exact(&mut buf[0..4]).await?;
            format!("{}.{}.{}.{}", buf[0], buf[1], buf[2], buf[3])
        },
        0x03 => { // Domain
            buf.resize(1, 0);
            stream.read_exact(&mut buf[0..1]).await?;
            let len = buf[0] as usize;
            buf.resize(len, 0);
            stream.read_exact(&mut buf[0..len]).await?;
            String::from_utf8_lossy(&buf[0..len]).into_owned()
        },
        0x04 => { // IPv6
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
            format!("{:x}:{:x}:{:x}:{:x}:{:x}:{:x}:{:x}:{:x}", a, b, c, d, e, f, g, h)
        },
        _ => return Err(anyhow::anyhow!("Unknown SOCKS5 address type")),
    };

    buf.resize(2, 0);
    stream.read_exact(&mut buf[0..2]).await?;
    let port = u16::from_be_bytes([buf[0], buf[1]]);

    // 3. OPSEC WARNING & FIREWALL
    let target_addr = target_addr.trim_matches(char::from(0)).trim().to_lowercase();
    let is_onion = target_addr.ends_with(".onion");
    
    if port == 80 && !is_onion {
        tracing::error!("OPSEC FIREWALL BLOCK: Dropped insecure plain-text Port 80 request to clearnet route: {}", target_addr);
        return Err(anyhow::anyhow!("Port 80 blocked to prevent malicious exit node SSL-stripping."));
    } else if port == 80 {
        tracing::info!("Allowed Port 80 for verified .onion E2E encrypted connection: {}", target_addr);
    } else if port != 443 {
        tracing::info!("Routing non-standard port request to {}:{}", target_addr, port);
    }

    // V3 Onion Validation
    if is_onion {
        let onion_regex = regex::Regex::new(r"^[a-z0-9-]{16,56}\.onion$").unwrap();
        if !onion_regex.is_match(&target_addr) {
            tracing::error!("Invalid Onion Address Structure: {}", target_addr);
            return Err(anyhow::anyhow!("Invalid Onion Address format."));
        }
    }

    tracing::info!("[Node {}] HFT-LB routing via Tor to: {}:{}", proxy_idx, target_addr, port);

    // 4. Circuit Build via Arti with Retry for Onion Volatility
    let mut prefs = arti_client::StreamPrefs::new();
    prefs.connect_to_onion_services(arti_client::config::BoolOrAuto::Explicit(true));
    
    let mut tor_stream = None;
    let mut retries = 0;
    let max_retries = if is_onion { 3 } else { 1 };
    
    while retries < max_retries {
        match tor_client.connect_with_prefs((target_addr.clone(), port), &prefs).await {
            Ok(stream) => {
                tor_stream = Some(stream);
                break;
            }
            Err(e) => {
                retries += 1;
                tracing::warn!("Tor connection attempt {}/{} failed for {}: {}", retries, max_retries, target_addr, e);
                if retries < max_retries {
                    tokio::time::sleep(tokio::time::Duration::from_millis(1500 * retries as u64)).await;
                } else {
                    return Err(anyhow::anyhow!("Failed to connect via Tor after {} retries: {}", max_retries, e));
                }
            }
        }
    }
    
    let mut tor_stream = tor_stream.unwrap();

    // 5. Success Reply
    stream.write_all(&[0x05, 0x00, 0x00, 0x01, 0,0,0,0, 0,0]).await?;

    // 6. Transparent bidirectional TCP pump — track bytes for bandwidth scoring
    let (client_to_tor, tor_to_client) = tokio::io::copy_bidirectional(&mut stream, &mut tor_stream)
        .await
        .unwrap_or((0, 0));
    let total_bytes = client_to_tor + tor_to_client;

    Ok(total_bytes)
}

