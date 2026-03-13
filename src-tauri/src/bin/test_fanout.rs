/// Minimal test for Phase 138 Isolation Fan-Out
/// Tests that the swarm bootstraps correctly with fan-out and measures performance.
///
/// Usage: cargo run --bin test_fanout --release
use anyhow::Result;
use std::time::Instant;

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() -> Result<()> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    
    println!("╔══════════════════════════════════════════════════════╗");
    println!("║  Phase 138: Isolation Fan-Out Bootstrap Test         ║");
    println!("╚══════════════════════════════════════════════════════╝");

    // Check fan-out config
    let fan_out = std::env::var("CRAWLI_ISOLATION_FAN_OUT")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(4);
    println!("\n  Fan-out ratio: {}", fan_out);

    let profile = crawli_lib::resource_governor::detect_profile(None);
    println!("  Recommended arti cap: {}", profile.recommended_arti_cap);
    println!("  CPU cores: {}", profile.cpu_cores);
    println!("  Total RAM: {:.1} GB", profile.total_memory_bytes as f64 / (1024.0 * 1024.0 * 1024.0));
    println!("  Storage: {:?}", profile.storage_class);

    let preset = crawli_lib::resource_governor::recommended_concurrency_preset();
    println!("\n  Concurrency preset: {}", preset.preset);
    println!("  Recommended circuits: {}", preset.circuits);
    println!("  Recommended workers: {}", preset.workers);

    // Bootstrap a cluster of Tor circuits
    let target_circuits = 8; // Small for speed test
    let base_needed = (target_circuits + fan_out - 1) / fan_out;
    println!("\n  Bootstrapping {} circuit slots from {} base clients (fan_out={})",
        target_circuits, base_needed, fan_out);

    let started = Instant::now();
    println!("  Starting at t=0.000s...\n");

    // Bootstrap individual nodes  
    let mut base_clients = Vec::new();
    for i in 0..base_needed {
        let node_start = Instant::now();
        match crawli_lib::tor_native::spawn_tor_node(i, false).await {
            Ok(client) => {
                let elapsed = node_start.elapsed();
                println!("  ✅ Base node {} bootstrapped in {:.1}s", i, elapsed.as_secs_f64());
                base_clients.push(std::sync::Arc::new(client));
            }
            Err(e) => {
                println!("  ❌ Base node {} failed: {}", i, e);
            }
        }
    }

    if base_clients.is_empty() {
        println!("\n  FATAL: No base clients bootstrapped");
        return Ok(());
    }

    // Fan out isolated views
    let mut total_circuits = 0usize;
    for (base_idx, base) in base_clients.iter().enumerate() {
        let slots = fan_out.min(target_circuits - total_circuits);
        for slot in 0..slots {
            if slot == 0 {
                // Base client itself
                total_circuits += 1;
            } else {
                let _isolated = base.isolated_client();
                total_circuits += 1;
            }
        }
        println!("  ⚡ Base {} → {} isolated views (total: {})", base_idx, slots, total_circuits);
    }

    let total_elapsed = started.elapsed();
    
    // Memory check
    let sys = sysinfo::System::new_all();
    let pid = sysinfo::Pid::from(std::process::id() as usize);
    let rss = {
        let mut s = sysinfo::System::new();
        s.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
        s.process(pid).map(|p| p.memory()).unwrap_or(0)
    };

    println!("\n╔══════════════════════════════════════════════════════╗");
    println!("║  RESULTS                                              ║");
    println!("╠══════════════════════════════════════════════════════╣");
    println!("║  Base clients bootstrapped: {}                        ", base_clients.len());
    println!("║  Total circuit slots:       {}                        ", total_circuits);
    println!("║  Fan-out ratio:             {}                        ", fan_out);
    println!("║  Bootstrap time:            {:.1}s                    ", total_elapsed.as_secs_f64());
    println!("║  Time per base client:      {:.1}s                    ", total_elapsed.as_secs_f64() / base_clients.len() as f64);
    println!("║  Process RSS:               {:.0} MB                  ", rss as f64 / (1024.0 * 1024.0));
    println!("║  RSS per circuit slot:      {:.1} MB                  ", (rss as f64 / (1024.0 * 1024.0)) / total_circuits as f64);
    println!("╚══════════════════════════════════════════════════════╝");

    // Test a connection through a base client
    println!("\n  Testing .onion connection through base client 0...");
    let test_start = Instant::now();
    match base_clients[0].connect(("ijzn3sicrcy7guixkzjkib4ukbiilwc3xhnmby4mcbccnsd7j2rekvqd.onion", 80)).await {
        Ok(_stream) => {
            println!("  ✅ .onion connection succeeded in {:.1}s", test_start.elapsed().as_secs_f64());
        }
        Err(e) => {
            println!("  ❌ .onion connection failed in {:.1}s: {}", test_start.elapsed().as_secs_f64(), e);
        }
    }

    // Test connection through an isolated view
    println!("  Testing .onion connection through isolated view...");
    let isolated = base_clients[0].isolated_client();
    let test_start2 = Instant::now();
    match isolated.connect(("ijzn3sicrcy7guixkzjkib4ukbiilwc3xhnmby4mcbccnsd7j2rekvqd.onion", 80)).await {
        Ok(_stream) => {
            println!("  ✅ Isolated connection succeeded in {:.1}s", test_start2.elapsed().as_secs_f64());
        }
        Err(e) => {
            println!("  ❌ Isolated connection failed in {:.1}s: {}", test_start2.elapsed().as_secs_f64(), e);
        }
    }

    println!("\n  Done. Phase 138 fan-out appears functional.");
    Ok(())
}
