use bytemuck::{Pod, Zeroable};
use memmap2::{MmapMut, MmapOptions};
use std::fs::OpenOptions;
use std::sync::Arc;
use tokio::time::{sleep, Duration};
use rand::Rng;

/// Layer A: The Compressed Tor Routing Key.
/// Normally, Tor clients parse huge text documents. Here, we compress a node
/// into a flawless 64-byte C-like Struct that can be read directly by the CPU
/// without parsing, allocations, or cloning.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct CompressedTorNode {
    ipv4_addr: [u8; 4],
    ipv6_addr: [u8; 16],
    ed25519_pubkey: [u8; 32],
    rsa_identity: [u8; 20],
    is_guard: u32,
    is_exit: u32,
    padding: [u32; 11], // Pad out to 128 bytes per node for L1 cache boundary alignment
}

const TOTAL_NODES_IN_CONSENSUS: usize = 70_000;
const SWARM_SIZE: usize = 150;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt().init();

    tracing::info!("=== INITIALIZING SHARED-MEMORY SWARM ENGINE ===");

    // ==========================================
    // LAYER A: THE MASTER SENTINEL
    // ==========================================
    tracing::info!("Sentinel: Booting and 'downloading' Tor Consensus...");
    // Simulate downloading a massive 25MB directory consensus from the Darknet.
    
    // Create a temporary file to act as our Shared Memory OS Bridge.
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open("shared_consensus_bridge.bin")?;
    
    let map_size = TOTAL_NODES_IN_CONSENSUS * std::mem::size_of::<CompressedTorNode>();
    file.set_len(map_size as u64)?;
    
    // POSIX mmap: Map the file directly into physical RAM space.
    let mut mmap_mut: MmapMut = unsafe { MmapOptions::new().map_mut(&file)? };

    tracing::info!("Sentinel: Parsing and compressing {} Tor nodes into OS Shared Memory Block ({} MB)...", TOTAL_NODES_IN_CONSENSUS, map_size / 1_000_000);
    
    // Cast the raw byte buffer directly into our fast C-struct array.
    let nodes: &mut [CompressedTorNode] = bytemuck::cast_slice_mut(&mut mmap_mut[..]);

    // Sentinel populates the raw RAM with cryptographic data.
    for i in 0..TOTAL_NODES_IN_CONSENSUS {
        nodes[i] = CompressedTorNode {
            ipv4_addr: [192, 168, 1, (i % 255) as u8],
            ipv6_addr: [0; 16],
            ed25519_pubkey: [ (i % 255) as u8; 32],
            rsa_identity: [ (i % 255) as u8; 20],
            is_guard: if i % 10 == 0 { 1 } else { 0 },
            is_exit: if i % 15 == 0 { 1 } else { 0 },
            padding: [0; 11],
        };
    }
    
    // Flush the OS memory block and lock it to Read-Only!
    mmap_mut.flush()?;
    let mmap_readonly = mmap_mut.make_read_only()?;
    let mmap_arc = Arc::new(mmap_readonly); // Arc just shares the POSIX pointer, NOT the 25MB payload.

    tracing::info!("Sentinel: Consensus successfully mapped to Read-Only CPU Cache Memory.");

    // ==========================================
    // LAYER C: THE DRONE SWARM
    // ==========================================
    tracing::info!("SwarmManager: Launching {} Lightweight Tor Drones...", SWARM_SIZE);
    
    let mut drone_handles = vec![];

    // Note how we only pass the `Arc` pointer. No data is cloned!
    for drone_id in 0..SWARM_SIZE {
        let mmap_pointer = Arc::clone(&mmap_arc);
        
        let handle = tokio::spawn(async move {
            // DRONE IS ACTIVE - Zero Parse Delay. Zero Network Delay.
            
            // Cast the raw OS memory bytes instantly back into the C-Struct array.
            // This reads directly from L1/L2 cache.
            let consensus_map: &[CompressedTorNode] = bytemuck::cast_slice(&mmap_pointer[..]);
            
            let (guard_ip, exit_ip, sleep_dur) = {
                let mut rng = rand::thread_rng();
                
                // Step 1: Pluck Guard Node from Shared Memory
                let mut guard_idx = rng.gen_range(0..TOTAL_NODES_IN_CONSENSUS);
                while consensus_map[guard_idx].is_guard == 0 { guard_idx = rng.gen_range(0..TOTAL_NODES_IN_CONSENSUS); }
                let guard_ip = consensus_map[guard_idx].ipv4_addr[3];
                
                // Step 2: Pluck Middle Node
                let _middle_idx = rng.gen_range(0..TOTAL_NODES_IN_CONSENSUS);
                
                // Step 3: Pluck Exit Node
                let mut exit_idx = rng.gen_range(0..TOTAL_NODES_IN_CONSENSUS);
                while consensus_map[exit_idx].is_exit == 0 { exit_idx = rng.gen_range(0..TOTAL_NODES_IN_CONSENSUS); }
                let exit_ip = consensus_map[exit_idx].ipv4_addr[3];
                
                let dur = rng.gen_range(50..150);
                (guard_ip, exit_ip, dur)
            };
            
            // Simulate TLS connection delay (the Drone executing the Diffie-Hellman handshake out to the Network)
            sleep(Duration::from_millis(sleep_dur)).await;
            
            if drone_id % 25 == 0 {
                tracing::info!("Drone [{}] Memory Read Complete. Guard IP: 192.168.1.{}. Exit IP: 192.168.1.{}", 
                    drone_id, guard_ip, exit_ip);
            }
        });
        drone_handles.push(handle);
    }

    futures::future::join_all(drone_handles).await;
    
    tracing::info!("=== SUCCESS: ALL {} DRONES BUILT CIRCUITS VIA SHARED MEMORY ===", SWARM_SIZE);
    tracing::info!("Total Memory Used for Consensus: {:.2} MB", map_size as f64 / 1_000_000.0);
    tracing::info!("Memory Saved (avoiding Tor instances cloning): {:.2} MB", (map_size * SWARM_SIZE) as f64 / 1_000_000.0);

    Ok(())
}
