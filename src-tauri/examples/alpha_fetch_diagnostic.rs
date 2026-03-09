use crawli_lib::frontier::{CrawlOptions, CrawlerFrontier};
use crawli_lib::AppState;
use std::time::Duration;
use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let app = tauri::Builder::default()
        .manage(AppState::default())
        .build(tauri::generate_context!())?;
        
    println!("Bootstrapping frontier...");
    crawli_lib::tor::cleanup_stale_tor_daemons();
    let (swarm_guard, ports) = crawli_lib::tor::bootstrap_tor_cluster(app.handle().clone(), 1).await?;
    let arti_clients = swarm_guard.get_arti_clients();
    
    let opts = CrawlOptions {
        listing: true,
        circuits: Some(5),
        daemons: Some(1),
        resume: false,
        ..Default::default()
    };
    
    let url = "http://3v4zoso2ghne47usnhyoe4dsezmfqhfv5v5iuep4saic5nnfpc6phrad.onion/gazomet.pl%20&%20cgas.pl/Files/";
    
    let frontier = std::sync::Arc::new(CrawlerFrontier::new(
        Some(app.handle().clone()),
        url.to_string(),
        5,
        true,
        ports.clone(),
        arti_clients.clone(),
        opts.clone(),
        None,
    ));
    
    let (_, client) = frontier.get_client();
    
    println!("Fetching {}...", url);
    match tokio::time::timeout(Duration::from_secs(45), client.get(url).send()).await {
        Ok(Ok(resp)) => {
            println!("Status: {}", resp.status());
            if let Ok(body) = resp.text().await {
                std::fs::write("/tmp/alphalocker.html", body)?;
                println!("Wrote alpha_locker dump to /tmp/alphalocker.html");
            }
        }
        Ok(Err(e)) => println!("Req err: {}", e),
        Err(_) => println!("Timeout"),
    }
    
    drop(swarm_guard);
    Ok(())
}
