//! Phase 77B: Aggressive Storage Node HS Probe Test
//!
//! Tests whether arti can connect to ANY of the Qilin storage .onion nodes.
//! Uses multiple TorClient instances with staggered attempts and longer timeouts.
//! This directly tests arti's v3 HS rendezvous capability.
//!
//! Usage: cargo run --example hs_storage_probe

use anyhow::Result;
use std::time::Instant;

const STORAGE_NODES: &[&str] = &[
    "szgkpzhcrnshftjb5mtvd6bc5oep5yabmgfmwt7u3tiqzfikoew27hqd.onion",
    "pandora42btuwlldza4uthk4bssbtsv47y4t5at5mo4ke3h4nqveobyd.onion",
    "7mnkv5nvnjyifezlfyba6gek7aeimg5eghej5vp65qxnb2hjbtlttlyd.onion",
    "25mjg55vcbjzwykz2uqsvaw7hcevm4pqxl42o324zr6qf5zgddmghkqd.onion",
    "arrfcpipltlfgxc6hvjylixc6c5hrummwctz4wqysk3h56ntqz5scnad.onion",
];

const CMS_NODE: &str = "ijzn3sicrcy7guixkzjkib4ukbiilwc3xhnmby4mcbccnsd7j2rekvqd.onion";

#[tokio::main]
async fn main() -> Result<()> {
    println!("╔══════════════════════════════════════════════════════╗");
    println!("║  Phase 77B: Qilin Storage HS Direct Probe           ║");
    println!("╚══════════════════════════════════════════════════════╝");
    println!();

    // Bootstrap a fresh arti client
    println!("[BOOT] Bootstrapping arti client...");
    let boot_start = Instant::now();

    let state_dir = std::env::temp_dir().join("arti_hs_probe");
    let _ = std::fs::remove_dir_all(&state_dir);

    let mut builder = arti_client::TorClientConfig::builder();
    builder
        .storage()
        .cache_dir(arti_client::config::CfgPath::new(
            state_dir.join("cache").to_string_lossy().into_owned(),
        ))
        .state_dir(arti_client::config::CfgPath::new(
            state_dir.join("state").to_string_lossy().into_owned(),
        ));
    builder.address_filter().allow_onion_addrs(true);
    let config = builder.build().unwrap();

    let client = arti_client::TorClient::create_bootstrapped(config).await?;
    println!(
        "[BOOT] ✓ Bootstrapped in {:.1}s",
        boot_start.elapsed().as_secs_f64()
    );
    println!();

    // Test 1: CMS node (baseline — should succeed)
    println!("═══ CMS Node (baseline) ═══");
    probe_onion(&client, CMS_NODE, 80).await;
    println!();

    // Wait 5s for warmup
    println!("[WAIT] 5s warmup pause...");
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;

    // Test 2: Storage nodes
    println!("═══ Storage Nodes ═══");
    for (i, node) in STORAGE_NODES.iter().enumerate() {
        println!("[{}/{}] Probing: {}", i + 1, STORAGE_NODES.len(), node);
        probe_onion(&client, node, 80).await;

        // 3s pause between probes to avoid circuit exhaustion
        if i < STORAGE_NODES.len() - 1 {
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        }
    }

    println!();
    println!("✓ Probe test complete.");
    Ok(())
}

async fn probe_onion(
    client: &arti_client::TorClient<tor_rtcompat::PreferredRuntime>,
    host: &str,
    port: u16,
) {
    let start = Instant::now();

    // Try with 90s timeout
    match tokio::time::timeout(
        std::time::Duration::from_secs(90),
        client.connect((host, port)),
    )
    .await
    {
        Ok(Ok(_stream)) => {
            println!("  ✅ CONNECTED in {:.1}s", start.elapsed().as_secs_f64());
        }
        Ok(Err(e)) => {
            let err_str = format!("{}", e);
            let elapsed = start.elapsed().as_secs_f64();
            if elapsed < 1.0 {
                println!(
                    "  ❌ INSTANT FAIL ({:.1}s): {} [HS descriptor likely missing]",
                    elapsed, err_str
                );
            } else {
                println!("  ❌ FAILED ({:.1}s): {}", elapsed, err_str);
            }
        }
        Err(_) => {
            println!("  ⏰ TIMEOUT (90s) [circuit build took too long]");
        }
    }
}
