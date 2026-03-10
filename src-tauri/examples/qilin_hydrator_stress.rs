use anyhow::{anyhow, Result};
use crawli_lib::adapters::universal_explorer::AdaptiveUniversalExplorer;
use crawli_lib::AppState;
use std::time::Duration;
use tauri::{Event, Listener, Manager};

#[tokio::main]
async fn main() -> Result<()> {
    // Setup Headless Tauri App specifically for observing Tier-4 Hydrator Telemetry bounds
    let app = tauri::Builder::default()
        .manage(AppState::default())
        .build(tauri::generate_context!())
        .map_err(|e| anyhow!("build tauri app: {}", e))?;

    let logs = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let logs_sink = logs.clone();

    app.listen_any("crawl_log", move |evt: Event| {
        if let Ok(line) = serde_json::from_str::<String>(evt.payload()) {
            println!(">> TELEMETRY: {}", line);
            logs_sink.lock().unwrap().push(line);
        }
    });

    let paths = crawli_lib::target_state::target_paths(
        &std::path::PathBuf::from("/tmp/onionforge_qilin_stress"),
        "http://qilin-onion.onion",
    )?;
    let ledger = crawli_lib::target_state::load_or_default_ledger(&paths)?;
    let explorer = AdaptiveUniversalExplorer::new(std::sync::Arc::new(ledger));

    println!("=== TEST 1: QILIN AUTOINDEX (MODE 3 HYDRATOR) ===");
    let html_autoindex = r#"
        <html><body>
        <table id="list">
          <tr><td class="link"><a href="accounting.zip">accounting.zip</a></td><td class="size">1.2 GB</td></tr>
        </table>
        QData browser
        </body></html>
    "#;
    let entries = explorer.parse_page_from_body(
        html_autoindex,
        "http://qilin-onion.onion",
        Some(&app.handle()),
    );
    assert!(entries.is_some(), "Mode 3 Autoindex failed to hydrate");
    println!("Extracted Entries: {}", entries.unwrap().len());
    tokio::time::sleep(Duration::from_millis(100)).await;

    println!("\n=== TEST 2: NEXTJS SPA MIMICRY (MODE 1 HYDRATOR) ===");
    let html_nextjs = r#"
        <html><body>
        <script id="__NEXT_DATA__" type="application/json">{"props":{"pageProps":{"files":[{"fsguest":"token123","path":"/api/dl/1"}]}}}</script>
        </body></html>
    "#;
    let entries = explorer.parse_page_from_body(
        html_nextjs,
        "http://advanced-qilin.onion",
        Some(&app.handle()),
    );
    println!("Extracted SPA Entries: {:?}", entries.is_some());
    tokio::time::sleep(Duration::from_millis(100)).await;

    println!("\n=== TEST 3: EMBEDDED IFRAME (MODE 2 HYDRATOR) ===");
    let html_iframe = r#"
        <html><body>
        <iframe src="http://advanced-qilin.onion/api/files?token=secure"></iframe>
        </body></html>
    "#;
    let entries = explorer.parse_page_from_body(
        html_iframe,
        "http://advanced-qilin.onion",
        Some(&app.handle()),
    );
    println!("Extracted Iframe Entries: {:?}", entries.is_some());
    tokio::time::sleep(Duration::from_millis(100)).await;

    let captured_logs = logs.lock().unwrap();
    let modes_detected = captured_logs
        .iter()
        .filter(|l| l.contains("Tier-4 Hydrator"))
        .count();
    println!(
        "\nTier-4 Hydrator Observability Validated. Detected {} telemetry pulses.",
        modes_detected
    );

    assert!(
        modes_detected >= 3,
        "Missing Tier-4 Hydrator Telemetry pulses"
    );

    Ok(())
}
