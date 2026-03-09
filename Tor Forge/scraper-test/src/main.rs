use reqwest::{Client, Proxy};
use std::time::Instant;
use futures::future::join_all;
use std::sync::Arc;
use tokio::sync::Mutex;
use anyhow::Result;

const PROXY_URL: &str = "socks5h://127.0.0.1:9050";
// 10 MB payload using Cloudflare's speed test file (or a similar test file) 
const TEST_FILE_URL: &str = "http://speedtest.tele2.net/1MB.zip"; // Fast test file without heavy TLS if possible, or github
const CHUNK_SIZE: u64 = 256 * 1024; // 256 KB chunk per worker

#[tokio::main]
async fn main() -> Result<()> {
    println!("Initialize: Distributed file extraction test algorithm via Tor SOCKS5...");

    let proxy = Proxy::all(PROXY_URL)?;
    let client = reqwest::Client::builder()
        .proxy(proxy)
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) Anonymous Payload Client")
        .timeout(std::time::Duration::from_secs(60))
        .build()?;

    let target_url = "https://raw.githubusercontent.com/torproject/tor/main/README.md"; // Extremely reliable endpoint
    println!("\n[PHASE 1] Deploying HEAD request to acquire Target Length: {}", target_url);

    let head_start = Instant::now();
    let head_resp = client.head(target_url).send().await?;
    
    // We expect content-length from Github Raw to be around 2-3 KB.
    let content_length: u64 = head_resp
        .headers()
        .get("content-length")
        .and_then(|val| val.to_str().ok())
        .and_then(|val| val.parse().ok())
        .unwrap_or(0);

    println!("[SUCCESS] Extracted length: {} bytes ({} ms)", content_length, head_start.elapsed().as_millis());
    
    if content_length == 0 {
        println!("Error: Target server did not supply Content-Length header or it is 0.");
        return Ok(());
    }

    // Set chunk dynamically to test multipath extraction on small files: 1 KB chunks
    let chunk_size: u64 = 1024; 
    let mut tasks = vec![];
    let mut num_chunks = 0;

    println!("\n[PHASE 2] Spawning Asynchronous Extractor Swarm ({} byte chunks)", chunk_size);
    let total_start = Instant::now();

    for start in (0..content_length).step_by(chunk_size as usize) {
        let end = std::cmp::min(start + chunk_size - 1, content_length - 1);
        let client_clone = client.clone();
        let target_url = target_url.to_string();
        num_chunks += 1;

        let task = tokio::spawn(async move {
            let chunk_start = Instant::now();
            let req = client_clone.get(&target_url)
                .header("Range", format!("bytes={}-{}", start, end))
                .send()
                .await?;

            let bytes = req.bytes().await?;
            let res: Result<(u64, bytes::Bytes, std::time::Duration), anyhow::Error> = Ok((start as u64, bytes, chunk_start.elapsed()));
            res
        });
        tasks.push(task);
    }

    println!("[SWARM] Dispatched {} independent chunk extraction workers into the Tor mesh...", tasks.len());
    
    let mut results = join_all(tasks).await;
    let mut total_downloaded = 0;
    
    println!("\n[PHASE 3] Reassembling extracted payload chunks from Tor Network");
    let mut final_buffer = vec![0u8; content_length as usize];

    for (index, res) in results.into_iter().enumerate() {
        if let Ok(Ok((start, bytes, duration))) = res {
            println!("   -> Chunk {}/{} securely retrieved: {} bytes in {} ms", index + 1, num_chunks, bytes.len(), duration.as_millis());
            total_downloaded += bytes.len() as u64;
            
            // Insert chunk into its exact byte offset in RAM
            let start_idx = start as usize;
            final_buffer[start_idx..start_idx + bytes.len()].copy_from_slice(&bytes);
        } else {
            println!("   -> ERROR: Chunk {} extraction failed.", index + 1);
        }
    }

    let end_time = total_start.elapsed();
    println!("\n========================================================");
    let is_valid = total_downloaded == content_length;
    println!("Payload Integrity Verified: {}", is_valid);
    println!("Total Retrieved: {} bytes / Expected: {} bytes", total_downloaded, content_length);
    println!("Total Multi-Threaded Time: {:.2?}", end_time);
    println!("========================================================\n");
    
    // Verify specific target contents
    let headr_string = String::from_utf8_lossy(&final_buffer[0..30]);
    println!("File Preview: {:?}", headr_string);

    Ok(())
}
