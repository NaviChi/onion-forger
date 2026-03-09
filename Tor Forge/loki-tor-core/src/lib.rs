pub mod actors;
pub mod telemetry;
pub mod quorum;

use anyhow::Result;
use ractor::Actor;
use tokio::time::Duration;

use crate::actors::tor_manager::{TorManager, TorManagerMsg};
use crate::actors::socks_proxy::SocksProxy;

/// Starts the Tor Daemon and SOCKS5 Proxy detached in the background.
/// `swarm_size`: Dynamic number of Tor circuits (auto-calculated from RAM/CPU or user override).
/// `is_vm`: Whether we're running inside a VM/container (enables clock drift tolerance).
pub async fn bootstrap_tor_daemon(port: u16, swarm_size: usize, is_vm: bool) -> Result<()> {
    tracing::info!("LOKI Tor Core initializing (swarm: {}, vm_mode: {})...", swarm_size, is_vm);

    // 1. Start TorManager Actor with dynamic swarm size and VM awareness
    let (tor_manager_ref, _tor_manager_handle) = Actor::spawn(None, TorManager, (swarm_size, is_vm))
        .await
        .map_err(|e| anyhow::anyhow!("Failed to spawn TorManager: {:?}", e))?;

    tracing::info!("Waiting for Tor to bootstrap...");
    
    // Request the initialized TorClients from the TorManager Actor.
    let tor_clients = ractor::call!(tor_manager_ref, |reply_to| TorManagerMsg::GetClients { reply_to })
        .map_err(|e| anyhow::anyhow!("TorManager failed to provide clients: {:?}", e))?;

    // 2. Start SocksProxy Actor, passing the TorClients and Dynamic Port
    let (socks_proxy_ref, _socks_proxy_handle) = Actor::spawn(None, SocksProxy, (tor_clients, port, is_vm))
        .await
        .map_err(|e| anyhow::anyhow!("Failed to spawn SocksProxy: {:?}", e))?;

    tracing::info!("LOKI Tor Core backend daemon successfully bootstrapped.");

    let tor_manager_clone = tor_manager_ref.clone();
    let socks_proxy_clone = socks_proxy_ref.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;
            if let Ok(clients) = ractor::call!(tor_manager_clone, |reply| TorManagerMsg::GetClients { reply_to: reply }) {
                if !clients.is_empty() {
                    let _ = socks_proxy_clone.cast(crate::actors::socks_proxy::SocksProxyMsg::UpdateClients(clients));
                    break;
                }
            }
        }
    });

    Ok(())
}
